//! Activity (`/activity`, `?tab=needs|todo|threads`, `?sel=`; `activity.tsx`):
//! the list of what needs the operator, everything, or the threads on the
//! left, and the open item in context on the right with what can be done
//! about it (reply, resend, resolve, keep or revoke).

use crate::chat::{self, ComposeTarget, Composer, ComposerActions, ComposerState, MessageOpts};
use crate::derive::activity::{
    Entry, Shown, activity_selection, entries, nudge_shown, read_mark_of, thread,
};
use crate::derive::agent_label;
use crate::derive::cards::{
    card_body, card_hidden, card_reason, ellipsize_words, read_all_notice, reason_icon,
    session_gone, session_gone_notice, without_label_prefix,
};
use crate::derive::direct::{
    Recipient, composer_block, composer_confirm, composer_notice, dm_hint, lead_display_name,
    recipient,
};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{Agent, Card, Message, WorkspaceView};
use crate::nav::{self, Nav, Route};
use crate::screens::inbox::{self as parts, ListKey, RowState};
use crate::theme::Theme;
use crate::ui::agent_card::post;
use crate::ui::format::hhmm;
use crate::ui::pill_people::{self as character, robot_px, type_px};
use crate::ui::{
    self, MONO, avatar_size, control, icon, icon_size, radius, row, size, space, weight,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    KeyDownEvent, ListAlignment, ListState, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, list, px,
};
use serde_json::json;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// "Resolve all" leaves a one-step undo for a few seconds; the read-all notice as long.
const UNDO: Duration = Duration::from_secs(8);
/// `.hb-item`'s padding above and below.
const ROW_PAD: f32 = 14.;
/// `.hb-tab`'s underline.
const TAB_LINE: f32 = 3.;
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
    /// Where the open item sits in the list, when it is in it.
    open_at: Option<usize>,
    /// The list's focus: its keys act while it holds it.
    focus: FocusHandle,
    /// The keyboard's focus row.
    cursor: Option<usize>,
    /// What `U` marks read for each row.
    row_marks: Vec<Option<(String, u64)>>,
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
    nav::replace(
        Route::Activity {
            tab: tab.to_string(),
            sel,
        },
        cx,
    );
}

impl ActivityScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| Composer::new(ComposeTarget::Agent(String::new()), window, cx));
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        cx.set_global(parts::ListFocus(focus.clone()));
        let subs = vec![
            cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()),
            cx.observe_global::<Nav>(|_, cx| cx.notify()),
            cx.observe_global::<Theme>(|_, cx| cx.notify()),
            // Esc marks the open item read, as in Direct.
            cx.observe_keystrokes(|this, ev, _, cx| {
                let k = &ev.keystroke;
                if k.key == "escape"
                    && !k.modifiers.shift
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
                cx.background_executor()
                    .timer(if ready { RETRO_READY } else { RETRO_WAIT })
                    .await;
            }
        });
        ActivityScreen {
            shown: None,
            shown_at: 0,
            open_at: None,
            focus,
            cursor: None,
            row_marks: Vec::new(),
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
                this.notice = Some(if ok {
                    read_all_notice(&before, cx)
                } else {
                    t(cx, "activity.saveFailed")
                });
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
    fn card_post(
        &mut self,
        action: &'static str,
        body: serde_json::Value,
        failed: &'static str,
        cx: &mut Context<Self>,
    ) {
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

impl ActivityScreen {
    /// The keys of the list (K4), while the list itself holds the focus.
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.focus.is_focused(window) {
            return;
        }
        let Some(key) = parts::list_key(ev) else {
            return;
        };
        let (tab, _) = route_parts(cx);
        match key {
            ListKey::Next | ListKey::Prev => {
                let from = self.cursor.or(self.open_at);
                self.cursor = parts::step(from, key, self.list_ids.len());
                if let Some(ix) = self.cursor {
                    self.list.scroll_to_reveal_item(ix);
                }
            }
            ListKey::Open => {
                if let Some(id) = self.cursor.and_then(|ix| self.list_ids.get(ix)).cloned() {
                    go(&tab, Some(id), cx);
                }
            }
            // The keystroke observer marks the open item read on Esc.
            ListKey::Back => self.cursor = None,
            ListKey::Read => {
                let mark = match self.cursor {
                    Some(ix) => self.row_marks.get(ix).cloned().flatten(),
                    None => self.mark.clone(),
                };
                if let Some((agent, ts)) = mark {
                    post("read", json!({ "agent": agent, "ts_ms": ts }), cx).detach();
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }
}

/// For each agent with unread messages, the time of the oldest one: serve
/// counts what an agent said after the operator's read mark, so its newest
/// `unread` messages are the unread ones.
fn unread_since(view: &WorkspaceView) -> HashMap<String, u64> {
    let mut times: HashMap<&str, Vec<u64>> = view
        .agents
        .iter()
        .filter(|a| a.unread > 0)
        .map(|a| (a.key.as_str(), Vec::new()))
        .collect();
    for m in view.messages.iter().filter(|m| m.from == "agent") {
        if let Some(list) = times.get_mut(m.agent.as_str()) {
            list.push(m.ts_ms);
        }
    }
    view.agents
        .iter()
        .filter(|a| a.unread > 0)
        .filter_map(|a| {
            let mut list = times.remove(a.key.as_str())?;
            list.sort_unstable_by(|x, y| y.cmp(x));
            let oldest = list.get(a.unread as usize - 1).or(list.last())?;
            Some((a.key.clone(), *oldest))
        })
        .collect()
}

impl Render for ActivityScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let (tab, picked) = route_parts(cx);
        let desk = Desk::get(cx).read(cx);
        let now = desk.now_ms();
        let Some(view) = desk.view.clone() else {
            return div()
                .size_full()
                .bg(theme.c("surface"))
                .child(parts::loading(cx))
                .into_any_element();
        };
        let listed = entries(&view, &tab);
        let shown = self.shown.take();
        let selected =
            activity_selection(&listed, picked.as_deref(), shown.as_ref(), self.shown_at)
                .map(Shown::of);
        drop(shown);
        self.open_at = selected
            .as_ref()
            .and_then(|s| listed.iter().position(|e| e.id() == s.id()));
        self.shown_at = self.open_at.unwrap_or(0);
        self.mark = selected
            .as_ref()
            .and_then(|s| read_mark_of(&view, s.entry()));
        self.row_marks = listed.iter().map(|e| read_mark_of(&view, *e)).collect();
        let selected_id = selected.as_ref().map(Shown::id);

        // The list, drawn by a virtual list: rows come from the view by index.
        let srcs: Vec<Src> = listed
            .iter()
            .map(|e| match e {
                Entry::Card(c) => Src::Card(
                    view.cards
                        .iter()
                        .position(|x| std::ptr::eq(x, *c))
                        .unwrap_or(0),
                ),
                Entry::Message(m) => Src::Message(
                    view.messages
                        .iter()
                        .position(|x| std::ptr::eq(x, *m))
                        .unwrap_or(0),
                ),
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
            self.cursor = self.cursor.filter(|ix| *ix < ids.len());
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
        let rows_unread = Rc::new(unread_since(&view));
        let cursor = self.cursor;
        let this = cx.entity().downgrade();
        let count = rows.len();
        let list_el = list(
            self.list.clone(),
            move |ix, _: &mut Window, cx: &mut App| {
                let theme = cx.global::<Theme>().clone();
                let id = rows_ids[ix].clone();
                let tab = rows_tab.clone();
                let this = this.clone();
                let (unread, cells) = match rows[ix] {
                    Src::Card(i) => card_row(&rows_view, &rows_view.cards[i], now, &theme, cx),
                    Src::Message(i) => {
                        message_row(&rows_view, &rows_view.messages[i], &rows_unread, &theme, cx)
                    }
                };
                let state = RowState {
                    active: rows_sel.as_deref() == Some(id.as_str()),
                    cursor: cursor == Some(ix),
                    dot: unread,
                };
                // `.hb-item`: as tall as what it says, 14 px above and below.
                parts::item_row(SharedString::from(format!("hb-act-{id}")), state, &theme)
                    .h_auto()
                    .py(px(ROW_PAD))
                    .children(cells)
                    .on_click(move |_, _, cx| {
                        let _ = this.update(cx, |this, _| this.cursor = None);
                        go(&tab, Some(id.clone()), cx)
                    })
                    .into_any_element()
            },
        )
        .flex_1()
        .min_h_0();

        let tabs = [
            ("needs", "activity.tabNeedsYou"),
            ("todo", "activity.tabAll"),
            ("threads", "activity.tabThreads"),
        ];
        let pane = parts::list_pane(&theme)
            // The console's header: the title at 24/900 and "Needs you: N"
            // as an accent pill (§3.1).
            .child(character::list_head(t(cx, "activity.title"), &theme).when(
                view.unread.activity > 0,
                |d| {
                    d.child(
                        character::accent_pill(
                            tv(
                                cx,
                                "activity.needsYouCount",
                                &[("count", &view.unread.activity.to_string())],
                            ),
                            &theme,
                        )
                        .ml_auto(),
                    )
                },
            ))
            // The buttons' labels start at the gutter; their wash reaches past it.
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(space::XS))
                    .px(px(space::LG))
                    .py(px(space::MD))
                    .child(
                        parts::quiet_btn(
                            "hb-read-all",
                            None,
                            t(cx, "activity.markAllRead"),
                            &theme,
                        )
                        .tooltip(parts::tip(
                            t(cx, "activity.doesNotResolve"),
                            &["\u{21e7}", "Esc"],
                        ))
                        .on_click(cx.listener({
                            let view = view.clone();
                            move |this, _, _, cx| this.read_all(&view, cx)
                        })),
                    )
                    .when(tab == "needs" && count > 0, |d| {
                        d.child(
                            parts::quiet_btn(
                                "hb-resolve-all",
                                None,
                                t(cx, "activity.resolveAll"),
                                &theme,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| this.resolve_all(needs_ids.clone(), cx),
                            )),
                        )
                    }),
            )
            .when(self.retro > 0, |d| {
                d.child(retro_card(self.retro, &theme, cx))
            })
            .children(crate::ui::feedback::dock_feedback(now, cx))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .gap(px(space::XL))
                    .h(px(row::TABS))
                    .px(px(space::GUTTER))
                    .border_b_1()
                    .border_color(theme.c("line"))
                    .children(tabs.iter().map(|(id, key)| {
                        let on = tab == *id;
                        let id = *id;
                        parts::control(
                            div()
                                .id(SharedString::from(format!("hb-tab-{id}")))
                                .flex()
                                .items_center()
                                .gap(px(space::SM))
                                .h_full()
                                // `.hb-tab`: 15/700, a 3 px accent underline.
                                .border_b(px(TAB_LINE))
                                .border_color(if on {
                                    theme.c("accent")
                                } else {
                                    gpui::transparent_black()
                                })
                                .cursor_pointer()
                                .child(character::text_px(
                                    t(cx, key),
                                    type_px::PILL,
                                    weight::STRONG,
                                    if on { theme.c("ink") } else { theme.c("ink-3") },
                                ))
                                .when(id == "todo" && view.unread.todo > 0, |d| {
                                    d.child(character::count_pill(view.unread.todo, &theme))
                                }),
                            &theme,
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.cursor = None;
                            go(id, None, cx)
                        }))
                    })),
            )
            .child(if count == 0 {
                let (title, hint) = match tab.as_str() {
                    "threads" => (t(cx, "activity.emptyThreads"), None),
                    "todo" => (t(cx, "activity.emptyAll"), None),
                    _ => (
                        t(cx, "activity.emptyNeeds"),
                        Some(t(cx, "activity.emptyNeedsHint")),
                    ),
                };
                div()
                    .flex_1()
                    .child(parts::empty(Some(title), hint, &theme))
                    .into_any_element()
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
                // An empty header keeps the hairline and the baseline the
                // list beside it has (T4, F3).
                .child(parts::detail_head(&theme))
                .child(parts::empty(
                    Some(t(cx, "activity.pickAlert")),
                    None,
                    &theme,
                ))
                .into_any_element(),
        };
        self.shown = selected;

        let toast = if let Some(ids) = &self.undo {
            let ids = ids.clone();
            let text = if ids.len() == 1 {
                t(cx, "activity.resolvedOne")
            } else {
                tv(
                    cx,
                    "activity.resolvedMany",
                    &[("count", &ids.len().to_string())],
                )
            };
            Some(parts::toast(
                &theme,
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::XL))
                    .child(text)
                    .child(
                        div()
                            .id("hb-undo")
                            .font_weight(weight::STRONG)
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
            .id("hb-activity")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
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
    parts::control(
        div()
            .id("hb-retro-card")
            .flex_none()
            .flex()
            .items_center()
            .gap(px(space::MD))
            .h(px(control::MD))
            .mx(px(space::GUTTER))
            .mb(px(space::MD))
            .px(px(space::LG))
            .border_1()
            .border_color(theme.c("line-strong"))
            .rounded(px(radius::BASE))
            .bg(theme.c("surface"))
            .cursor_pointer()
            .hover(|s| s.bg(theme.c("hover-2")))
            .child(icon("lightbulb", icon_size::ROW, theme.c("ink-3")))
            .child(
                ui::text(
                    t(cx, "retro.weeklyCardTitle"),
                    size::UI,
                    weight::STRONG,
                    theme.c("ink"),
                )
                .flex_none(),
            )
            .child(
                ui::text(
                    format!("· {rules}"),
                    size::UI,
                    weight::REGULAR,
                    theme.c("ink-2"),
                )
                .min_w_0()
                .truncate(),
            ),
        theme,
    )
    .on_click(|_, _, cx| nav::navigate(Route::Retro, cx))
    .into_any_element()
}

/// The face of a row: the agent's robot, the operator, or the robot the
/// name alone gives.
fn face(
    view: &WorkspaceView,
    agent: Option<&Agent>,
    from: &str,
    name: &str,
    cx: &App,
) -> AnyElement {
    match agent {
        Some(a) => ui::avatar::agent_avatar(a, robot_px::ROW, true, cx),
        None if from == "operator" => operator_face(view, cx),
        None => ui::avatar::name_avatar(name, robot_px::ROW, cx),
    }
}

fn operator_face(view: &WorkspaceView, cx: &App) -> AnyElement {
    let initials = if view.operator_initials.is_empty() {
        "OP"
    } else {
        &view.operator_initials
    };
    ui::avatar::operator_avatar(initials, robot_px::ROW, cx)
}

/// `.hb-item-preview`: two lines of prose at most, ended with an ellipsis.
fn row_preview(text: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
    character::text_px(text, size::BODY, weight::REGULAR, theme.c("ink-2"))
        .line_clamp(2)
        .text_ellipsis()
}

/// A title and the sentence after it, as one run of prose.
fn title_then(title: &str, text: &str) -> SharedString {
    if text.is_empty() {
        return title.to_string().into();
    }
    if title.ends_with(['.', '?', '!', ':']) {
        format!("{title} {text}").into()
    } else {
        format!("{title}. {text}").into()
    }
}

/// A card's row (`CardItem`): what it asks and where as the title, who and
/// what under it. Returns whether it is unread and the row's cells.
fn card_row(
    view: &WorkspaceView,
    card: &Card,
    now: u64,
    theme: &Theme,
    cx: &App,
) -> (bool, Vec<AnyElement>) {
    let message = card.message.as_deref().and_then(|id| view.message(id));
    let agent = card.agents.first().and_then(|k| view.agent(k));
    let open = card.needs && card.resolved_ms.is_none();
    let body = card_body(card, view, now, cx);
    let (avatar, preview): (AnyElement, SharedString) = if card.kind == "unanswered" {
        (operator_face(view, cx), title_then(&body.title, &body.text))
    } else if let (true, Some(a)) = (card.kind == "dormant", agent) {
        (
            ui::avatar::agent_avatar(a, robot_px::ROW, true, cx),
            body.text,
        )
    } else if let (Some(m), Some(a)) = (message, agent) {
        let said = format!(
            "{}: {}",
            a.name,
            without_label_prefix(&m.text, m.label.as_deref())
        );
        let preview = if card.kind == "blocked" && session_gone(a) {
            title_then(&session_gone_notice(cx), &said)
        } else {
            said.into()
        };
        (
            ui::avatar::agent_avatar(a, robot_px::ROW, true, cx),
            preview,
        )
    } else {
        (
            parts::icon_slot(reason_icon(&card.kind), theme.c("warn-icon")).into_any_element(),
            title_then(&body.title, &body.text),
        )
    };
    let when = if let Some(ms) = card.resolved_ms {
        tv(cx, "activity.resolvedAt", &[("time", &hhmm(ms))])
    } else if !card.needs {
        t(cx, "activity.cardHistory")
    } else {
        hhmm(card.since_ms).into()
    };
    let cells = vec![
        avatar,
        parts::item_body()
            .child(
                parts::item_line()
                    .h(px(character::line_for(type_px::PILL)))
                    .child(
                        character::text_px(
                            card_reason(card, cx),
                            type_px::PILL,
                            weight::STRONG,
                            theme.c("ink"),
                        )
                        .min_w_0()
                        .truncate(),
                    )
                    .when(card_hidden(view, card), |d| {
                        d.child(ui::chip(t(cx, "activity.hidden"), "neutral", theme))
                    })
                    .child(parts::item_time(when, theme)),
            )
            .child(row_preview(preview, theme))
            .into_any_element(),
    ];
    (open, cells)
}

/// A message's row (`MessageEntry`): who said it as the title, its chip,
/// its replies and its time, and what it says under it.
fn message_row(
    view: &WorkspaceView,
    m: &Message,
    unread_since: &HashMap<String, u64>,
    theme: &Theme,
    cx: &App,
) -> (bool, Vec<AnyElement>) {
    let agent = view.agent(&m.agent);
    let name = agent.map_or(m.author.clone(), |a| a.name.clone());
    let unread = m.from == "agent"
        && unread_since
            .get(&m.agent)
            .is_some_and(|since| m.ts_ms >= *since);
    let replies = m.replies.len();
    let cells = vec![
        face(view, agent, &m.from, &name, cx),
        parts::item_body()
            .child(
                parts::item_line()
                    .h(px(character::line_for(type_px::ITEM_NAME)))
                    .child(
                        character::text_px(name, type_px::ITEM_NAME, weight::HEAVY, theme.c("ink"))
                            .min_w_0()
                            .truncate(),
                    )
                    .children(parts::label_chip(m, view, theme, cx))
                    .child(
                        div()
                            .ml_auto()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap(px(space::MD))
                            .when(replies > 0, |d| {
                                d.child(ui::text(
                                    if replies == 1 {
                                        t(cx, "workspace.repliesOne")
                                    } else {
                                        tv(
                                            cx,
                                            "workspace.repliesMany",
                                            &[("n", &replies.to_string())],
                                        )
                                    },
                                    size::META,
                                    weight::REGULAR,
                                    theme.c("link"),
                                ))
                            })
                            .child(parts::item_time(hhmm(m.ts_ms), theme)),
                    ),
            )
            .child(row_preview(
                without_label_prefix(&m.text, m.label.as_deref()).to_string(),
                theme,
            ))
            .into_any_element(),
    ];
    (unread, cells)
}

impl ActivityScreen {
    #[allow(clippy::too_many_arguments)]
    fn detail(
        &mut self,
        view: &Arc<WorkspaceView>,
        s: &Shown,
        now: u64,
        window: &mut Window,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let card: Option<&Card> = match s {
            Shown::Card(c) => Some(c),
            Shown::Message(_) => None,
        };
        let message: Option<Message> = match s {
            Shown::Message(m) => Some((**m).clone()),
            Shown::Card(c) => c
                .message
                .as_deref()
                .and_then(|id| view.message(id))
                .cloned(),
        };
        let channel = match s {
            Shown::Card(c) => c.channel.clone(),
            Shown::Message(m) => m.channels.first().cloned(),
        };
        let agent_key = message
            .as_ref()
            .map(|m| m.agent.clone())
            .or_else(|| card.and_then(|c| c.agents.first().cloned()));
        let agent = agent_key.as_deref().and_then(|k| view.agent(k)).cloned();

        let head = parts::detail_head(theme)
            // The same title as the channel's own header: the mark recedes,
            // the name is what is read.
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::SM))
                    .min_w_0()
                    .when(channel.is_some(), |d| {
                        d.child(ui::icon("hash", icon_size::ROW, theme.c("ink-3")))
                    })
                    .child(
                        ui::text(
                            match &channel {
                                Some(c) => SharedString::from(c.clone()),
                                None => t(cx, "activity.detailFallback"),
                            },
                            size::TITLE,
                            weight::HEAVY,
                            theme.c("ink"),
                        )
                        .min_w_0()
                        .truncate(),
                    ),
            )
            .when_some(channel.clone(), |d, c| {
                let thread = message
                    .as_ref()
                    .map(|m| m.reply_to.clone().unwrap_or_else(|| m.id.clone()));
                d.child(
                    parts::quiet_btn(
                        "hb-go-channel",
                        Some("arrow-up-right"),
                        t(cx, "activity.goToChannel"),
                        theme,
                    )
                    .ml_auto()
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
        // The message brings its own gutter; everything else is inset to it.
        let mut body = div().flex().flex_col();
        if let Some(m) = &message {
            let replies = thread(view, &m.id);
            body = body.child(chat::message_item(m, view, &whole, cx)).when(
                !replies.is_empty(),
                |d| {
                    d.child(
                        div()
                            .ml(px(space::GUTTER + avatar_size::ROW + space::LG))
                            .border_l_1()
                            .border_color(theme.c("line"))
                            .children(
                                replies
                                    .iter()
                                    .map(|r| chat::message_item(r, view, &whole, cx)),
                            ),
                    )
                },
            );
        } else if let Some(c) = card {
            let b = card_body(c, view, now, cx);
            let names: Vec<String> = c
                .agents
                .iter()
                .map(|k| agent_label(view, k, cx).to_string())
                .collect();
            body = body.child(
                div()
                    .flex()
                    .gap(px(space::LG))
                    .px(px(space::GUTTER))
                    .child(icon(
                        "triangle-alert",
                        icon_size::RAIL,
                        theme.c("warn-icon"),
                    ))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .gap(px(space::XS))
                            .child(ui::text(b.title, size::UI, weight::STRONG, theme.c("ink")))
                            .child(ui::text(
                                b.text,
                                size::BODY,
                                weight::REGULAR,
                                theme.c("ink"),
                            ))
                            .children(c.checkout.clone().map(|co| {
                                ui::text(co, size::META, weight::REGULAR, theme.c("ink-3"))
                                    .font_family(MONO)
                                    .truncate()
                            }))
                            .child(ui::text(
                                names.join(", "),
                                size::SMALL,
                                weight::REGULAR,
                                theme.c("ink-3"),
                            )),
                    ),
            );
        }
        let actions = self.card_actions(
            view,
            card,
            message.as_ref(),
            agent.as_ref(),
            now,
            window,
            theme,
            cx,
        );
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
                    .py(px(space::XL))
                    .child(body)
                    .child(
                        div()
                            .mt(px(space::SECTION))
                            .px(px(space::GUTTER))
                            .child(actions),
                    ),
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
        let actions_row = || {
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(space::MD))
                .mt(px(space::LG))
        };
        // "This didn't need me" teaches Retro the alert was noise; it does not
        // dismiss it, so it stays a quiet control beside the actions.
        let not_needed =
            card.and_then(|c| crate::ui::feedback::feedback_row("needs_you", &c.id, None, cx));
        // What "Dismiss alert" does and does not do, said where the button is.
        let resolve_tip = card
            .is_some_and(|c| c.kind != "blocked")
            .then(|| t(cx, "activity.resolveHint"));
        // "Was it you?": Keep (after a fresh confirmation), or Revoke.
        let check = card
            .filter(|c| c.kind == "check" && c.resolved_ms.is_none())
            .map(|c| {
                let id = c.id.clone();
                match self.keep_code.clone().filter(|(cid, _)| *cid == id) {
                    Some((_, code)) => vec![
                        ui::text(
                            t(cx, "activity.keepConfirm"),
                            size::UI,
                            weight::REGULAR,
                            theme.c("ink-2"),
                        )
                        .flex()
                        .items_center()
                        .into_any_element(),
                        parts::btn("hb-keep-yes", t(cx, "activity.yesKeep"), true, false, theme)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.keep_code = None;
                                this.card_post(
                                    "resolve",
                                    json!({ "card": id, "confirm": code }),
                                    "activity.saveFailed",
                                    cx,
                                );
                            }))
                            .into_any_element(),
                        parts::btn(
                            "hb-keep-cancel",
                            t(cx, "activity.cancel"),
                            false,
                            false,
                            theme,
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.keep_code = None;
                            cx.notify();
                        }))
                        .into_any_element(),
                    ],
                    None => {
                        let keep_id = id.clone();
                        vec![
                            parts::btn("hb-keep", t(cx, "activity.keep"), false, false, theme)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.error = None;
                                    let id = keep_id.clone();
                                    let task = post("resolve", json!({ "card": id }), cx);
                                    cx.spawn(async move |this, cx| {
                                        let out = task.await;
                                        let _ = this.update(cx, |this, cx| {
                                            match out {
                                                Ok(v) => {
                                                    if let Some(code) = v
                                                        .pointer("/confirm/id")
                                                        .and_then(|c| c.as_str())
                                                    {
                                                        this.keep_code =
                                                            Some((id, code.to_string()));
                                                    }
                                                }
                                                Err(_) => {
                                                    this.error = Some(t(cx, "activity.saveFailed"))
                                                }
                                            }
                                            cx.notify();
                                        });
                                    })
                                    .detach();
                                }))
                                .into_any_element(),
                            parts::btn("hb-revoke", t(cx, "activity.revoke"), true, false, theme)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.card_post(
                                        "check-revoke",
                                        json!({ "card": id }),
                                        "activity.revokeFailed",
                                        cx,
                                    );
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
                    parts::btn(
                        "hb-resolve",
                        if reopen {
                            t(cx, "activity.reopen")
                        } else {
                            t(cx, "activity.dismissAlert")
                        },
                        false,
                        false,
                        theme,
                    )
                    .when_some(resolve_tip.clone(), |b, tip| {
                        b.tooltip(parts::tip(tip, &[]))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let action = if reopen { "reopen" } else { "resolve" };
                        this.card_post(action, json!({ "card": id }), "activity.saveFailed", cx);
                    }))
                    .into_any_element(),
                ]
            }
            (None, None) => Vec::new(),
        };
        let nudged_ms = message
            .and_then(|m| m.delivery.as_ref())
            .and_then(|d| d.nudged_ms);
        let nudged = nudge_shown(nudged_ms, now);
        let mut out = div().flex().flex_col();
        if let Some(c) =
            card.filter(|c| c.kind == "unanswered" && c.needs && c.resolved_ms.is_none())
        {
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
                .child(ui::text(
                    tv(cx, "activity.nudgeExplain", &[("title", &title)]),
                    size::BODY,
                    weight::REGULAR,
                    theme.c("ink"),
                ))
                .child(
                    actions_row()
                        .child(parts::btn("hb-nudge", label, true, disabled, theme).when(
                            !disabled,
                            |b| {
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
                            },
                        ))
                        .children(resolve)
                        .children(not_needed),
                )
                .when(nudged, |d| {
                    d.child(
                        ui::text(
                            t(cx, "activity.resentNote"),
                            size::SMALL,
                            weight::REGULAR,
                            theme.c("ink-3"),
                        )
                        .mt(px(space::MD)),
                    )
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
            // Who the reply reaches, as serve routes it: named on the box, its
            // button and a line above it, never corrected by a footnote.
            let about = ellipsize_words(&a.name, 40);
            let (who, reader, placeholder, send) = match recipient(&view.agents, a) {
                Recipient::Direct(me) => (
                    tv(cx, "activity.whoActsDirect", &[("name", &about)]),
                    Some(me),
                    tv(cx, "activity.messageAgent", &[("name", &about)]),
                    tv(cx, "activity.sendTo", &[("name", &about)]),
                ),
                Recipient::Lead(lead) => {
                    let lead_name = ellipsize_words(lead_display_name(&lead.name), 40);
                    (
                        tv(
                            cx,
                            "activity.whoActsLead",
                            &[("lead", &lead_name), ("name", &about)],
                        ),
                        Some(lead),
                        tv(
                            cx,
                            "activity.messageLeadAbout",
                            &[("lead", &lead_name), ("name", &about)],
                        ),
                        tv(cx, "activity.sendTo", &[("name", &lead_name)]),
                    )
                }
                Recipient::UnknownLead => (
                    tv(cx, "activity.whoActsUnknown", &[("name", &about)]),
                    None,
                    tv(cx, "activity.messageLeadOf", &[("name", &about)]),
                    t(cx, "messages.reply"),
                ),
            };
            self.composer
                .update(cx, |c, cx| c.set_target(target, placeholder, cx));
            let state = ComposerState {
                blocked: None,
                notice: composer_notice(&[a], cx),
                confirm: composer_confirm(&[a], cx),
                // Only a session that ended still needs a word under the box.
                hint: session_gone(a).then(|| dm_hint(a, None, cx)),
                boxed: true,
                send_label: Some(send),
            };
            let who_row = div()
                .flex()
                .items_center()
                .gap(px(space::MD))
                .mb(px(space::LG))
                .children(reader.map(|r| {
                    crate::robot::robot(r.name.clone(), crate::robot::tier::DENSE)
                        .agent(r)
                        .into_any_element()
                }))
                .child(ui::text(who, size::UI, weight::STRONG, theme.c("ink")));
            // "Dismiss alert" (or "Reopen") sits beside the box's send button.
            let actions: Option<ComposerActions> = card.filter(|c| c.kind != "check").map(|c| {
                let id = c.id.clone();
                let reopen = c.resolved_ms.is_some();
                let label = if reopen {
                    t(cx, "activity.reopen")
                } else {
                    t(cx, "activity.dismissAlert")
                };
                let this = cx.entity().downgrade();
                let tip = resolve_tip.clone();
                let f: ComposerActions = Rc::new(move |_, cx: &mut App| {
                    let theme = cx.global::<Theme>().clone();
                    // Read while the composer renders, so it redraws when the
                    // feedback store changes.
                    let not_needed = crate::ui::feedback::feedback_row("needs_you", &id, None, cx);
                    let (id, this) = (id.clone(), this.clone());
                    let dismiss = parts::btn("hb-resolve", label.clone(), false, false, &theme)
                        .when_some(tip.clone(), |b, tip| b.tooltip(parts::tip(tip, &[])))
                        .on_click(move |_, _, cx| {
                            let action = if reopen { "reopen" } else { "resolve" };
                            let _ = this.update(cx, |this, cx| {
                                this.card_post(
                                    action,
                                    json!({ "card": id }),
                                    "activity.saveFailed",
                                    cx,
                                )
                            });
                        });
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(space::MD))
                        .child(dismiss)
                        .children(not_needed)
                        .into_any_element()
                });
                f
            });
            let has_actions = actions.is_some();
            self.composer.update(cx, |c, cx| {
                c.set_state(state, cx);
                c.set_actions(actions, cx);
            });
            // With a card, its actions sit in the box's row; without one,
            // there is nothing to dismiss.
            out = out
                .child(who_row)
                .child(self.composer.clone())
                .when(!has_actions && !resolve.is_empty(), |d| {
                    d.child(actions_row().children(resolve).children(not_needed))
                });
        } else {
            let blocked = match agent {
                Some(a) => composer_block(&[a], cx).unwrap_or_default(),
                None => t(cx, "activity.noAgentToAnswer"),
            };
            out = out
                .child(
                    div()
                        .p(px(space::LG))
                        .border_1()
                        .border_dashed()
                        .border_color(theme.c("line-strong"))
                        .rounded(px(radius::MD))
                        .bg(theme.c("surface-2"))
                        .child(ui::text(
                            format!("{blocked}."),
                            size::SMALL,
                            weight::REGULAR,
                            theme.c("ink-3"),
                        )),
                )
                .when(!resolve.is_empty() || not_needed.is_some(), |d| {
                    d.child(actions_row().children(resolve).children(not_needed))
                });
        }
        // A failure, where the action was (F: error).
        out.children(self.error.clone().map(|e| {
            div().flex().mt(px(space::MD)).child(
                div()
                    .px(px(space::MD))
                    .py(px(space::XS))
                    .rounded(px(radius::SM))
                    .bg(theme.c("red-bg"))
                    .child(ui::text(
                        e,
                        size::SMALL,
                        weight::REGULAR,
                        theme.c("red-ink"),
                    )),
            )
        }))
        .into_any_element()
    }
}
