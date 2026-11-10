//! Activity (`/activity`, `?tab=needs|todo|threads`, `?sel=`; `activity.tsx`):
//! the list of what needs the operator, everything, or the threads on the
//! left, and the open item in context on the right with what can be done
//! about it (reply, resend, resolve, keep or revoke).

use crate::chat::{self, ComposeTarget, ComposerActions, ComposerState, Composer, MessageOpts};
use crate::derive::activity::{Entry, Shown, activity_selection, entries, nudge_shown, read_mark_of, thread};
use crate::derive::agent_label;
use crate::derive::cards::{
    card_body, card_hidden, card_reason, ellipsize_words, read_all_notice, reason_icon, session_gone, session_gone_notice,
    without_label_prefix,
};
use crate::derive::direct::{composer_block, composer_confirm, composer_notice, dm_hint, note_reader};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{Agent, Card, Message, WorkspaceView};
use crate::nav::{self, Nav, Route};
use crate::screens::inbox as parts;
use crate::theme::Theme;
use crate::ui::agent_card::post;
use crate::ui::format::hhmm;
use crate::ui::{self, icon};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, Div, Entity, FontWeight, InteractiveElement, IntoElement, ListAlignment, ListState,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, div, list, px,
};
use serde_json::json;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// "Resolve all" leaves a one-step undo for a few seconds; the read-all notice as long.
const UNDO: Duration = Duration::from_secs(8);
/// serve's cached Retro count: soon while its first pass runs, then every few minutes.
const RETRO_WAIT: Duration = Duration::from_secs(5);
const RETRO_READY: Duration = Duration::from_secs(300);

/// Where a list row comes from in the view.
#[derive(Clone, Copy, PartialEq)]
enum Src {
    Card(usize),
    Message(usize),
}

pub struct ActivityScreen {
    shown: Option<Shown>,
    shown_at: usize,
    /// What Esc marks read: the open item's agent, up to its time.
    mark: Option<(String, u64)>,
    undo: Option<Vec<String>>,
    notice: Option<SharedString>,
    toast_task: Option<Task<()>>,
    keep_code: Option<(String, String)>,
    error: Option<SharedString>,
    sending: bool,
    retro: u64,
    composer: Entity<Composer>,
    list: ListState,
    list_ids: Vec<String>,
    _retro_task: Task<()>,
    _subs: Vec<Subscription>,
}

fn route_parts(cx: &App) -> (String, Option<String>) {
    match Nav::route(cx) {
        Route::Activity { tab, sel } => (tab.clone(), sel.clone()),
        _ => ("needs".into(), None),
    }
}

fn go(tab: &str, sel: Option<String>, cx: &mut App) {
    nav::replace(Route::Activity { tab: tab.to_string(), sel }, cx);
}

impl ActivityScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| Composer::new(ComposeTarget::Agent(String::new()), window, cx));
        let subs = vec![
            cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()),
            cx.observe_global::<Nav>(|_, cx| cx.notify()),
            cx.observe_global::<Theme>(|_, cx| cx.notify()),
            // Esc marks the open item read, as in Direct.
            cx.observe_keystrokes(|this, ev, _, cx| {
                let k = &ev.keystroke;
                if k.key == "escape" && !k.modifiers.shift
                    && let Some((agent, ts)) = this.mark.clone()
                {
                    post("read", json!({ "agent": agent, "ts_ms": ts }), cx).detach();
                }
            }),
        ];
        let retro_task = cx.spawn(async move |this, cx| {
            loop {
                let got = cx
                    .background_spawn(async { crate::api::get("/retro/count") })
                    .await
                    .ok()
                    .and_then(|v| v.get("candidates").map(|c| c.as_u64()));
                let ready = matches!(got, Some(Some(_)));
                let alive = this.update(cx, |this, cx| {
                    let n = got.flatten().unwrap_or(0);
                    if this.retro != n {
                        this.retro = n;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
                cx.background_executor().timer(if ready { RETRO_READY } else { RETRO_WAIT }).await;
            }
        });
        ActivityScreen {
            shown: None,
            shown_at: 0,
            mark: None,
            undo: None,
            notice: None,
            toast_task: None,
            keep_code: None,
            error: None,
            sending: false,
            retro: 0,
            composer,
            list: ListState::new(0, ListAlignment::Top, px(600.)),
            list_ids: Vec::new(),
            _retro_task: retro_task,
            _subs: subs,
        }
    }

    fn toast_for_a_while(&mut self, cx: &mut Context<Self>) {
        self.toast_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(UNDO).await;
            let _ = this.update(cx, |this, cx| {
                this.undo = None;
                this.notice = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn read_all(&mut self, view: &WorkspaceView, cx: &mut Context<Self>) {
        let before = view.unread.clone();
        let task = post("read-all", json!({}), cx);
        cx.spawn(async move |this, cx| {
            let ok = task.await.is_ok();
            let _ = this.update(cx, |this, cx| {
                this.undo = None;
                this.notice = Some(if ok { read_all_notice(&before, cx) } else { t(cx, "activity.saveFailed") });
                this.toast_for_a_while(cx);
            });
        })
        .detach();
    }

    fn resolve_all(&mut self, ids: Vec<String>, cx: &mut Context<Self>) {
        let task = post("resolve-all", json!({ "cards": ids }), cx);
        cx.spawn(async move |this, cx| {
            if task.await.is_ok() {
                let _ = this.update(cx, |this, cx| {
                    this.undo = Some(ids);
                    this.toast_for_a_while(cx);
                });
            }
        })
        .detach();
    }

    /// Post an action about a card; a failure shows `failed` under the actions.
    fn card_post(&mut self, action: &'static str, body: serde_json::Value, failed: &'static str, cx: &mut Context<Self>) {
        self.error = None;
        let task = post(action, body, cx);
        cx.spawn(async move |this, cx| {
            if task.await.is_err() {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(t(cx, failed));
                    cx.notify();
                });
            }
        })
        .detach();
        cx.notify();
    }
}

impl Render for ActivityScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let (tab, picked) = route_parts(cx);
        let desk = Desk::get(cx).read(cx);
        let now = desk.now_ms();
        let Some(view) = desk.view.clone() else {
            return div().size_full().bg(theme.c("surface")).child(parts::loading(cx)).into_any_element();
        };
        let listed = entries(&view, &tab);
        let shown = self.shown.take();
        let selected = activity_selection(&listed, picked.as_deref(), shown.as_ref(), self.shown_at).map(Shown::of);
        drop(shown);
        self.shown_at = selected
            .as_ref()
            .and_then(|s| listed.iter().position(|e| e.id() == s.id()))
            .unwrap_or(0);
        self.mark = selected.as_ref().and_then(|s| read_mark_of(&view, s.entry()));
        let selected_id = selected.as_ref().map(Shown::id);

        // The list, drawn by a virtual list: rows come from the view by index.
        let srcs: Vec<Src> = listed
            .iter()
            .map(|e| match e {
                Entry::Card(c) => Src::Card(view.cards.iter().position(|x| std::ptr::eq(x, *c)).unwrap_or(0)),
                Entry::Message(m) => Src::Message(view.messages.iter().position(|x| std::ptr::eq(x, *m)).unwrap_or(0)),
            })
            .collect();
        let ids: Vec<String> = listed.iter().map(Entry::id).collect();
        if ids != self.list_ids {
            let top = self.list.logical_scroll_top();
            self.list.reset(ids.len());
            if top.item_ix < ids.len() {
                self.list.scroll_to(top);
            }
            self.list_ids = ids.clone();
        }
        let needs_ids: Vec<String> = listed
            .iter()
            .filter_map(|e| match e {
                Entry::Card(c) => Some(c.id.clone()),
                _ => None,
            })
            .collect();
        drop(listed);

        let rows_view = view.clone();
        let rows_tab = tab.clone();
        let rows_sel = selected_id.clone();
        let rows = Rc::new(srcs);
        let rows_ids = Rc::new(ids);
        let count = rows.len();
        let list_el = list(self.list.clone(), move |ix, _: &mut Window, cx: &mut App| {
            let theme = cx.global::<Theme>().clone();
            let id = rows_ids[ix].clone();
            let active = rows_sel.as_deref() == Some(id.as_str());
            let tab = rows_tab.clone();
            let body = match rows[ix] {
                Src::Card(i) => card_item(&rows_view, &rows_view.cards[i], now, &theme, cx),
                Src::Message(i) => message_entry(&rows_view, &rows_view.messages[i], &theme, cx),
            };
            parts::item_row(SharedString::from(format!("hb-act-{id}")), active, &theme)
                .flex_col()
                .gap(px(0.))
                .child(body)
                .on_click(move |_, _, cx| go(&tab, Some(id.clone()), cx))
                .into_any_element()
        })
        .flex_1()
        .min_h_0();

        let tabs = [("needs", "activity.tabNeedsYou"), ("todo", "activity.tabAll"), ("threads", "activity.tabThreads")];
        let pane = parts::list_pane(&theme)
            .child(parts::list_head(t(cx, "activity.title"), &theme).when(view.unread.activity > 0, |d| {
                d.child(parts::unread_pill(
                    tv(cx, "activity.needsYouCount", &[("count", &view.unread.activity.to_string())]),
                    &theme,
                ))
            }))
            .child(
                div()
                    .flex()
                    .gap(px(16.))
                    .px(px(20.))
                    .pb(px(8.))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .items_center()
                            .child(parts::link_btn("hb-read-all", t(cx, "activity.markAllRead"), &theme).on_click(cx.listener({
                                let view = view.clone();
                                move |this, _, _, cx| this.read_all(&view, cx)
                            })))
                            .child(
                                div()
                                    .text_size(px(13.))
                                    .text_color(theme.c("ink-3"))
                                    .child(t(cx, "activity.doesNotResolve")),
                            ),
                    )
                    .when(tab == "needs" && count > 0, |d| {
                        d.child(parts::link_btn("hb-resolve-all", t(cx, "activity.resolveAll"), &theme).on_click(
                            cx.listener(move |this, _, _, cx| this.resolve_all(needs_ids.clone(), cx)),
                        ))
                    }),
            )
            .when(self.retro > 0, |d| d.child(retro_card(self.retro, &theme, cx)))
            .children(crate::ui::feedback::dock_feedback(now, cx))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .gap(px(6.))
                    .px(px(14.))
                    .border_b_1()
                    .border_color(theme.c("line"))
                    .children(tabs.iter().map(|(id, key)| {
                        let on = tab == *id;
                        let id = *id;
                        div()
                            .id(SharedString::from(format!("hb-tab-{id}")))
                            .flex()
                            .items_center()
                            .gap(px(4.))
                            .pt(px(8.))
                            .px(px(12.))
                            .pb(px(10.))
                            .mb(px(-1.))
                            .border_b(px(3.))
                            .border_color(if on { theme.c("accent") } else { gpui::transparent_black() })
                            .font_weight(FontWeight::BOLD)
                            .text_size(px(15.))
                            .text_color(if on { theme.c("ink") } else { theme.c("ink-3") })
                            .cursor_pointer()
                            .child(t(cx, key))
                            .when(id == "todo" && view.unread.todo > 0, |d| d.child(parts::count(view.unread.todo, &theme).ml(px(4.))))
                            .on_click(move |_, _, cx| go(id, None, cx))
                    })),
            )
            .child(if count == 0 {
                let (title, hint) = match tab.as_str() {
                    "threads" => (t(cx, "activity.emptyThreads"), None),
                    "todo" => (t(cx, "activity.emptyAll"), None),
                    _ => (t(cx, "activity.emptyNeeds"), Some(t(cx, "activity.emptyNeedsHint"))),
                };
                div().flex_1().child(parts::empty(Some(title), hint, &theme)).into_any_element()
            } else {
                list_el.into_any_element()
            });

        // A different item opens: what was typed or asked about the old one goes.
        if self.shown.as_ref().map(Shown::id) != selected_id {
            self.keep_code = None;
            self.error = None;
        }
        let detail = match &selected {
            Some(s) => self.detail(&view, s, now, window, &theme, cx),
            None => div()
                .flex_1()
                .min_w_0()
                .bg(theme.c("surface"))
                .child(parts::empty(None, Some(t(cx, "activity.pickAlert")), &theme))
                .into_any_element(),
        };
        self.shown = selected;

        let toast = if let Some(ids) = &self.undo {
            let ids = ids.clone();
            let text = if ids.len() == 1 {
                t(cx, "activity.resolvedOne")
            } else {
                tv(cx, "activity.resolvedMany", &[("count", &ids.len().to_string())])
            };
            Some(parts::toast(
                &theme,
                div().flex().items_center().gap(px(16.)).child(text).child(
                    div()
                        .id("hb-undo")
                        .font_weight(FontWeight::BLACK)
                        .underline()
                        .cursor_pointer()
                        .child(t(cx, "activity.undo"))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.undo = None;
                            post("reopen-all", json!({ "cards": ids }), cx).detach();
                            cx.notify();
                        })),
                ),
            ))
        } else {
            self.notice.clone().map(|n| parts::toast(&theme, n))
        };
        div()
            .relative()
            .size_full()
            .flex()
            .min_h_0()
            .child(pane)
            .child(detail)
            .children(toast)
            .into_any_element()
    }
}

/// "This week's Retro · 6 suggested rules", a link to Retro.
fn retro_card(n: u64, theme: &Theme, cx: &App) -> AnyElement {
    let rules = if n == 1 {
        tv(cx, "retro.proposedRulesOne", &[("count", "1")])
    } else {
        tv(cx, "retro.proposedRulesMany", &[("count", &n.to_string())])
    };
    div()
        .id("hb-retro-card")
        .flex()
        .items_center()
        .gap(px(8.))
        .mx(px(16.))
        .mb(px(8.))
        .px(px(12.))
        .py(px(8.))
        .border_1()
        .border_color(theme.c("line-strong"))
        .rounded(px(6.))
        .bg(theme.c("surface"))
        .text_size(px(14.))
        .text_color(theme.c("ink"))
        .cursor_pointer()
        .hover(|s| s.bg(theme.c("control")))
        .child(icon("lightbulb", 16., theme.c("ink")))
        .child(
            div()
                .flex()
                .child(div().font_weight(FontWeight::BLACK).child(t(cx, "retro.weeklyCardTitle")))
                .child(div().whitespace_nowrap().child(format!("\u{a0}· {rules}"))),
        )
        .on_click(|_, _, cx| nav::navigate(Route::Retro, cx))
        .into_any_element()
}

/// An avatar with the agent's card on hover, or the plain monogram.
fn avatar_for(agent: Option<&Agent>, name: &str, size: f32, theme: &Theme, cx: &App) -> AnyElement {
    match agent {
        Some(a) => ui::avatar::agent_avatar(a, size, true, cx),
        None => div()
            .size(px(size))
            .flex_none()
            .rounded(px((size * 0.22).round().max(6.)))
            .bg(theme.tone_for(name))
            .flex()
            .items_center()
            .justify_center()
            .text_color(theme.c("av-ink"))
            .font_weight(FontWeight::BOLD)
            .text_size(px((size * 0.4).round()))
            .child(ui::format::monogram(name))
            .into_any_element(),
    }
}

/// The body of a card's row (`CardItem`).
fn card_item(view: &WorkspaceView, card: &Card, now: u64, theme: &Theme, cx: &App) -> AnyElement {
    let message = card.message.as_deref().and_then(|id| view.message(id));
    let agent = card.agents.first().and_then(|k| view.agent(k));
    let right = if let Some(ms) = card.resolved_ms {
        div()
            .ml_auto()
            .text_size(px(13.))
            .font_weight(FontWeight::NORMAL)
            .text_color(theme.c("ink-3"))
            .child(tv(cx, "activity.resolvedAt", &[("time", &hhmm(ms))]))
    } else if !card.needs {
        div()
            .ml_auto()
            .text_size(px(13.))
            .font_weight(FontWeight::NORMAL)
            .text_color(theme.c("ink-3"))
            .child(t(cx, "activity.cardHistory"))
    } else if card.kind == "blocked" {
        parts::count(1, theme).ml_auto()
    } else {
        div().ml_auto().size(px(18.)).rounded_full().bg(theme.c("accent"))
    };
    let reason = div()
        .flex()
        .items_center()
        .gap(px(8.))
        .mb(px(8.))
        .font_weight(FontWeight::BOLD)
        .text_size(px(15.))
        .text_color(theme.c("ink"))
        .child(icon(reason_icon(&card.kind), 20., theme.c("ink")))
        .child(card_reason(card, cx))
        .when(card_hidden(view, card), |d| {
            d.child(
                div()
                    .px(px(6.))
                    .rounded(px(4.))
                    .bg(theme.c("chip"))
                    .text_color(theme.c("chip-ink"))
                    .text_size(px(12.))
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(t(cx, "activity.hidden")),
            )
        })
        .child(right);
    let line = |avatar: AnyElement, name: SharedString, chip: Option<Div>, preview: SharedString, gone: Option<SharedString>| {
        div()
            .flex()
            .gap(px(12.))
            .child(avatar)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().flex().items_center().gap(px(8.)).child(parts::item_name(name, theme)).children(chip))
                    .child(parts::item_preview(preview, theme))
                    .children(gone.map(|g| div().mt(px(2.)).text_size(px(14.)).text_color(theme.c("ink-3")).child(g))),
            )
            .into_any_element()
    };
    let body = card_body(card, view, now, cx);
    let content = if card.kind == "unanswered" {
        let initials = if view.operator_initials.is_empty() { "OP" } else { &view.operator_initials };
        line(ui::avatar::operator_avatar(initials, 48., cx), body.title, None, body.text, None)
    } else if let (true, Some(a)) = (card.kind == "dormant", agent) {
        line(avatar_for(Some(a), &a.name, 48., theme, cx), a.name.clone().into(), None, body.text, None)
    } else if let (Some(m), Some(a)) = (message, agent) {
        let gone = (card.kind == "blocked" && session_gone(a)).then(|| format!("{}.", session_gone_notice(cx)).into());
        line(
            avatar_for(Some(a), &a.name, 48., theme, cx),
            a.name.clone().into(),
            parts::label_chip(m.label.as_deref(), theme, cx),
            without_label_prefix(&m.text, m.label.as_deref()).to_string().into(),
            gone,
        )
    } else {
        div()
            .pl(px(32.))
            .flex()
            .flex_col()
            .child(div().font_weight(FontWeight::BLACK).text_size(px(16.)).child(body.title))
            .child(parts::item_preview(body.text, theme))
            .into_any_element()
    };
    div().flex().flex_col().child(reason).child(content).into_any_element()
}

/// The body of a message's row (`MessageEntry`).
fn message_entry(view: &WorkspaceView, m: &Message, theme: &Theme, cx: &App) -> AnyElement {
    let agent = view.agent(&m.agent);
    let name = agent.map_or(m.author.clone(), |a| a.name.clone());
    let replies = m.replies.len();
    div()
        .flex()
        .gap(px(12.))
        .child(avatar_for(agent, &name, 48., theme, cx))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(parts::item_name(name, theme))
                        .children(parts::label_chip(m.label.as_deref(), theme, cx))
                        .child(parts::item_time(hhmm(m.ts_ms), theme)),
                )
                .child(parts::item_preview(without_label_prefix(&m.text, m.label.as_deref()).to_string(), theme))
                .when(replies > 0, |d| {
                    d.child(
                        div()
                            .mt(px(6.))
                            .flex()
                            .items_center()
                            .gap(px(6.))
                            .text_size(px(14.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.c("link"))
                            .child(icon("message-square", 16., theme.c("link")))
                            .child(if replies == 1 {
                                t(cx, "workspace.repliesOne")
                            } else {
                                tv(cx, "workspace.repliesMany", &[("n", &replies.to_string())])
                            }),
                    )
                }),
        )
        .into_any_element()
}

impl ActivityScreen {
    #[allow(clippy::too_many_arguments)]
    fn detail(&mut self, view: &Arc<WorkspaceView>, s: &Shown, now: u64, window: &mut Window, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let card: Option<&Card> = match s {
            Shown::Card(c) => Some(c),
            Shown::Message(_) => None,
        };
        let message: Option<Message> = match s {
            Shown::Message(m) => Some((**m).clone()),
            Shown::Card(c) => c.message.as_deref().and_then(|id| view.message(id)).cloned(),
        };
        let channel = match s {
            Shown::Card(c) => c.channel.clone(),
            Shown::Message(m) => m.channels.first().cloned(),
        };
        let agent_key = message.as_ref().map(|m| m.agent.clone()).or_else(|| card.and_then(|c| c.agents.first().cloned()));
        let agent = agent_key.as_deref().and_then(|k| view.agent(k)).cloned();

        let head = div()
            .h(px(64.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .mx(px(28.))
            .border_b_1()
            .border_color(theme.c("line"))
            .font_weight(FontWeight::BLACK)
            .text_size(px(18.))
            .child(match &channel {
                Some(c) => SharedString::from(format!("# {c}")),
                None => t(cx, "activity.detailFallback"),
            })
            .when_some(channel.clone(), |d, c| {
                let thread = message.as_ref().map(|m| m.reply_to.clone().unwrap_or_else(|| m.id.clone()));
                d.child(
                    div()
                        .id("hb-go-channel")
                        .ml_auto()
                        .font_weight(FontWeight::BOLD)
                        .text_size(px(16.))
                        .text_color(theme.c("link"))
                        .underline()
                        .cursor_pointer()
                        .child(t(cx, "activity.goToChannel"))
                        .on_click(move |_, _, cx| {
                            nav::navigate(
                                Route::Home {
                                    channel: Some(c.clone()),
                                    thread: thread.clone(),
                                    item: None,
                                },
                                cx,
                            )
                        }),
                )
            });

        let whole = MessageOpts {
            in_thread: true,
            fold: false,
            continued: false,
            ..Default::default()
        };
        let mut body = div().flex().flex_col();
        if let Some(m) = &message {
            let replies = thread(view, &m.id);
            body = body.child(
                div()
                    .mx(px(-20.))
                    .child(chat::message_item(m, view, &whole, cx))
                    .when(!replies.is_empty(), |d| {
                        d.child(
                            div()
                                .ml(px(52.))
                                .border_l_2()
                                .border_color(theme.c("line"))
                                .children(replies.iter().map(|r| chat::message_item(r, view, &whole, cx))),
                        )
                    }),
            );
        } else if let Some(c) = card {
            let b = card_body(c, view, now, cx);
            let names: Vec<String> = c.agents.iter().map(|k| agent_label(view, k, cx).to_string()).collect();
            body = body.child(
                div()
                    .flex()
                    .gap(px(12.))
                    .child(div().mt(px(4.)).child(icon("triangle-alert", 24., theme.c("warn-icon"))))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(div().font_weight(FontWeight::BLACK).text_size(px(16.)).child(b.title))
                            .child(div().mt(px(4.)).child(b.text))
                            .children(c.checkout.clone().map(|co| {
                                div()
                                    .mt(px(8.))
                                    .font_family(ui::MONO)
                                    .text_size(px(14.))
                                    .text_color(theme.c("ink-3"))
                                    .child(co)
                            }))
                            .child(
                                div()
                                    .mt(px(8.))
                                    .text_size(px(14.))
                                    .text_color(theme.c("ink-3"))
                                    .child(names.join(", ")),
                            ),
                    ),
            );
        }
        // "This didn't need me", for any card.
        if let Some(c) = card {
            let pad = if message.is_some() { px(0.) } else { px(36.) };
            body = body.children(crate::ui::feedback::feedback_row("needs_you", &c.id, None, cx).map(|r| div().mt(px(12.)).pl(pad).child(r)));
        }
        let actions = self.card_actions(view, card, message.as_ref(), agent.as_ref(), now, window, theme, cx);
        div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme.c("surface"))
            .child(head)
            .child(
                div()
                    .id("hb-detail-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(28.))
                    .py(px(20.))
                    .child(body)
                    .child(div().mt(px(24.)).child(actions)),
            )
            .into_any_element()
    }

    /// What the operator can do about the item (`CardActions`).
    #[allow(clippy::too_many_arguments)]
    fn card_actions(
        &mut self,
        view: &Arc<WorkspaceView>,
        card: Option<&Card>,
        message: Option<&Message>,
        agent: Option<&Agent>,
        now: u64,
        _window: &mut Window,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let actions_row = || div().flex().gap(px(16.)).mt(px(24.));
        let big = |b: gpui::Stateful<Div>| b.h(px(48.)).px(px(32.)).text_size(px(17.));
        // "Was it you?": Keep (after a fresh confirmation), or Revoke.
        let check = card.filter(|c| c.kind == "check" && c.resolved_ms.is_none()).map(|c| {
            let id = c.id.clone();
            match self.keep_code.clone().filter(|(cid, _)| *cid == id) {
                Some((_, code)) => vec![
                    div().text_color(theme.c("ink-2")).child(t(cx, "activity.keepConfirm")).into_any_element(),
                    big(parts::btn("hb-keep-yes", t(cx, "activity.yesKeep"), true, false, theme))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.keep_code = None;
                            this.card_post("resolve", json!({ "card": id, "confirm": code }), "activity.saveFailed", cx);
                        }))
                        .into_any_element(),
                    big(parts::btn("hb-keep-cancel", t(cx, "activity.cancel"), false, false, theme))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.keep_code = None;
                            cx.notify();
                        }))
                        .into_any_element(),
                ],
                None => {
                    let keep_id = id.clone();
                    vec![
                        big(parts::btn("hb-keep", t(cx, "activity.keep"), false, false, theme))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.error = None;
                                let id = keep_id.clone();
                                let task = post("resolve", json!({ "card": id }), cx);
                                cx.spawn(async move |this, cx| {
                                    let out = task.await;
                                    let _ = this.update(cx, |this, cx| {
                                        match out {
                                            Ok(v) => {
                                                if let Some(code) = v.pointer("/confirm/id").and_then(|c| c.as_str()) {
                                                    this.keep_code = Some((id, code.to_string()));
                                                }
                                            }
                                            Err(_) => this.error = Some(t(cx, "activity.saveFailed")),
                                        }
                                        cx.notify();
                                    });
                                })
                                .detach();
                            }))
                            .into_any_element(),
                        big(parts::btn("hb-revoke", t(cx, "activity.revoke"), true, false, theme))
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.card_post("check-revoke", json!({ "card": id }), "activity.revokeFailed", cx);
                            }))
                            .into_any_element(),
                    ]
                }
            }
        });
        let resolve: Vec<AnyElement> = match (check, card) {
            (Some(check), _) => check,
            (None, Some(c)) => {
                let id = c.id.clone();
                let reopen = c.resolved_ms.is_some();
                vec![
                    big(parts::btn(
                        "hb-resolve",
                        if reopen { t(cx, "activity.reopen") } else { t(cx, "activity.markResolved") },
                        false,
                        false,
                        theme,
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let action = if reopen { "reopen" } else { "resolve" };
                        this.card_post(action, json!({ "card": id }), "activity.saveFailed", cx);
                    }))
                    .into_any_element(),
                ]
            }
            (None, None) => Vec::new(),
        };
        let nudged_ms = message.and_then(|m| m.delivery.as_ref()).and_then(|d| d.nudged_ms);
        let nudged = nudge_shown(nudged_ms, now);
        let mut out = div().flex().flex_col();
        if let Some(c) = card.filter(|c| c.kind == "unanswered" && c.needs && c.resolved_ms.is_none()) {
            let title = card_body(c, view, now, cx).title;
            let label = if self.sending {
                t(cx, "activity.resending")
            } else if let (true, Some(ms)) = (nudged, nudged_ms) {
                tv(cx, "activity.resentAt", &[("time", &hhmm(ms))])
            } else {
                t(cx, "activity.resendToLead")
            };
            let id = c.id.clone();
            let disabled = nudged || self.sending;
            out = out
                .child(div().text_size(px(15.)).child(tv(cx, "activity.nudgeExplain", &[("title", &title)])))
                .child(
                    actions_row()
                        .child(big(parts::btn("hb-nudge", label, true, disabled, theme)).when(!disabled, |b| {
                            b.on_click(cx.listener(move |this, _, _, cx| {
                                if this.sending {
                                    return;
                                }
                                this.sending = true;
                                this.error = None;
                                let task = post("nudge", json!({ "card": id }), cx);
                                cx.spawn(async move |this, cx| {
                                    let out = task.await;
                                    let _ = this.update(cx, |this, cx| {
                                        this.sending = false;
                                        if let Err(e) = out
                                            && e.status != Some(409)
                                        {
                                            this.error = Some(t(cx, "activity.resendFailed"));
                                        }
                                        cx.notify();
                                    });
                                })
                                .detach();
                                cx.notify();
                            }))
                        }))
                        .children(resolve),
                )
                .when(nudged, |d| {
                    d.child(div().mt(px(12.)).text_size(px(14.)).text_color(theme.c("ink-3")).child(t(cx, "activity.resentNote")))
                });
        } else if let Some(a) = agent.filter(|a| composer_block(&[a], cx).is_none()) {
            let target = match message {
                Some(m) => ComposeTarget::Thread {
                    parent: m.id.clone(),
                    channel: None,
                    agent: Some(a.key.clone()),
                },
                None => ComposeTarget::Agent(a.key.clone()),
            };
            let placeholder = tv(cx, "activity.replyTo", &[("name", &ellipsize_words(&a.name, 40))]);
            self.composer.update(cx, |c, cx| c.set_target(target, placeholder, cx));
            let state = ComposerState {
                blocked: None,
                notice: composer_notice(&[a], cx),
                confirm: composer_confirm(&[a], cx),
                hint: Some(dm_hint(a, note_reader(&view.agents, &a.key), cx)),
                boxed: true,
            };
            // "Mark resolved" (or "Reopen") sits beside the box's "Reply".
            let actions: Option<ComposerActions> = card.filter(|c| c.kind != "check").map(|c| {
                let id = c.id.clone();
                let reopen = c.resolved_ms.is_some();
                let label = if reopen { t(cx, "activity.reopen") } else { t(cx, "activity.markResolved") };
                let this = cx.entity().downgrade();
                let f: ComposerActions = Rc::new(move |_, cx: &mut App| {
                    let theme = cx.global::<Theme>().clone();
                    let (id, this) = (id.clone(), this.clone());
                    parts::btn("hb-resolve", label.clone(), false, false, &theme)
                        .h(px(48.))
                        .px(px(32.))
                        .text_size(px(17.))
                        .on_click(move |_, _, cx| {
                            let action = if reopen { "reopen" } else { "resolve" };
                            let _ = this.update(cx, |this, cx| {
                                this.card_post(action, json!({ "card": id }), "activity.saveFailed", cx)
                            });
                        })
                        .into_any_element()
                });
                f
            });
            let has_actions = actions.is_some();
            self.composer.update(cx, |c, cx| {
                c.set_state(state, cx);
                c.set_actions(actions, cx);
            });
            out = out
                .child(div().mx(px(-20.)).child(self.composer.clone()))
                .when(!has_actions && !resolve.is_empty(), |d| d.child(actions_row().children(resolve)));
        } else {
            let blocked = match agent {
                Some(a) => composer_block(&[a], cx).unwrap_or_default(),
                None => t(cx, "activity.noAgentToAnswer"),
            };
            out = out
                .child(
                    div()
                        .px(px(16.))
                        .py(px(14.))
                        .border_1()
                        .border_dashed()
                        .border_color(theme.c("line-strong"))
                        .rounded(px(10.))
                        .bg(theme.c("surface-2"))
                        .text_size(px(14.))
                        .text_color(theme.c("ink-3"))
                        .child(format!("{blocked}.")),
                )
                .when(!resolve.is_empty(), |d| d.child(actions_row().children(resolve)));
        }
        out.children(self.error.clone().map(|e| div().mt(px(8.)).text_size(px(13.)).text_color(theme.c("red-ink")).child(e)))
            .when(card.is_some_and(|c| c.kind != "blocked"), |d| {
                d.child(div().mt(px(16.)).text_size(px(13.)).text_color(theme.c("ink-3")).child(t(cx, "activity.resolveHint")))
            })
            .into_any_element()
    }
}
