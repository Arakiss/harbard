//! Calm mode (`calm-screen.tsx`, `lib/calm.ts`): while Swamped the panel
//! shows only the one thing that needs the operator now, and one calm line.
//! The sidebar keeps its headings and nothing else. "Reply" opens a reply
//! box inside the card; "Later" moves on to the next item; "Exit" leaves
//! calm mode for this Swamped without changing the state.

use crate::derive::agent_name;
use crate::derive::away::{CalmTitle, calm_model, reason_key};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::WorkspaceView;
use crate::theme::Theme;
use crate::ui::{self, format::hhmm, icon};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, Div, ElementId, Entity, Focusable, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, div, px, relative,
};
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use serde_json::json;

/// On in Swamped, unless they left it for this Swamped (`exited` is that Swamped's start).
pub fn calm_on(state: &str, since: u64, exited: Option<u64>) -> bool {
    state == "swamped" && exited != Some(since)
}

pub struct CalmScreen {
    /// Items put off with "Later", in this calm mode only.
    later: Vec<String>,
    /// The item whose reply box is open; another top closes it.
    replying: Option<String>,
    reply: Entity<TextareaState>,
    error: Option<SharedString>,
    sending: bool,
    /// "Reply to {name}" as last set on the box.
    placeholder: SharedString,
}

impl CalmScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        let reply = cx.new(|cx| TextareaState::new(window, cx).auto_grow(3, 8));
        // The box composer's ⌘Enter sends; plain Enter is a new line.
        cx.subscribe_in(&reply, window, |this, _, ev: &InputEvent, window, cx| {
            if let InputEvent::PressEnter {
                secondary: true, ..
            } = ev
            {
                this.send(window, cx);
            }
        })
        .detach();
        CalmScreen {
            later: Vec::new(),
            replying: None,
            reply,
            error: None,
            sending: false,
            placeholder: SharedString::default(),
        }
    }

    /// The top item's card and the agent a reply goes to.
    fn target(&self, cx: &App) -> Option<(String, Option<String>, Option<String>)> {
        let view = Desk::get(cx).read(cx).view.clone()?;
        let m = calm_model(Some(&view), &self.later, crate::shell::profile::now_ms());
        let (w, _) = m.top?;
        let card = view.cards.iter().find(|c| format!("card:{}", c.id) == w.id);
        Some((
            card.map(|c| c.id.clone()).unwrap_or_default(),
            w.agent.clone(),
            card.and_then(|c| c.message.clone()),
        ))
    }

    fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self.reply.read(cx).value().trim().to_string();
        if text.is_empty() || self.sending {
            return;
        }
        let Some((_, Some(agent), reply_to)) = self.target(cx) else {
            return;
        };
        let mut body = json!({ "agent": agent, "text": text });
        if let Some(id) = reply_to {
            body["reply_to"] = json!(id);
        }
        self.sending = true;
        self.error = None;
        let task = Desk::get(cx).update(cx, |d, cx| d.post("reply", body, cx));
        cx.spawn_in(window, async move |this, cx| {
            let out = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.sending = false;
                match out {
                    Ok(_) => this.reply.update(cx, |s, cx| s.set_value("", window, cx)),
                    Err(_) => this.error = Some(t(cx, "workspace.replySaveFailed")),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn resolve(&mut self, cx: &mut Context<Self>) {
        let Some((card, _, _)) = self.target(cx) else {
            return;
        };
        if card.is_empty() {
            return;
        }
        self.error = None;
        let task = Desk::get(cx).update(cx, |d, cx| d.post("resolve", json!({ "card": card }), cx));
        cx.spawn(async move |this, cx| {
            if task.await.is_err() {
                let _ = this.update(cx, |this, cx| {
                    this.error = Some(t(cx, "activity.saveFailed"));
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

/// A `.hb-btn`: surface, strong edge, 6px corners, 15px bold.
pub(super) fn hb_btn(id: impl Into<ElementId>, primary: bool, theme: &Theme) -> Stateful<Div> {
    let hover = if primary {
        theme.c("accent-hover")
    } else {
        theme.c("btn-hover")
    };
    hb_btn_base(id, primary, theme).hover(move |s| s.bg(hover))
}

/// [`hb_btn`] without its hover, for a button that sets its own.
fn hb_btn_base(id: impl Into<ElementId>, primary: bool, theme: &Theme) -> Stateful<Div> {
    let (bg, edge, ink) = if primary {
        (
            theme.c("accent-deep"),
            theme.c("accent-deep"),
            theme.c("accent-ink"),
        )
    } else {
        (theme.c("surface"), theme.c("line-strong"), theme.c("ink"))
    };
    div()
        .id(id)
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .h(px(36.))
        .px(px(16.))
        .rounded(px(6.))
        .border_1()
        .border_color(edge)
        .bg(bg)
        .text_color(ink)
        .text_size(px(15.))
        .font_weight(FontWeight::BOLD)
        .cursor_pointer()
}

/// A name's monogram tile (`Avatar`), when there is no agent to draw.
pub(super) fn monogram_avatar(name: &str, size: f32, theme: &Theme) -> AnyElement {
    div()
        .size(px(size))
        .flex_none()
        .rounded(px((size * 0.22).max(6.)))
        .bg(theme.tone_for(name))
        .flex()
        .items_center()
        .justify_center()
        .text_color(theme.c("av-ink"))
        .font_weight(FontWeight::BLACK)
        .text_size(px(size * 0.4))
        .child(ui::format::monogram(name))
        .into_any_element()
}

fn section(label: SharedString, theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .min_h(px(24.))
        .mt(px(18.))
        .mb(px(2.))
        .px(px(10.))
        .text_size(px(12.))
        .font_weight(FontWeight::EXTRA_BOLD)
        .text_color(theme.c("sidebar-ink-2"))
        .child(label.to_uppercase())
}

impl Render for CalmScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let view: Option<std::sync::Arc<WorkspaceView>> = Desk::get(cx).read(cx).view.clone();
        let model = calm_model(
            view.as_deref(),
            &self.later,
            crate::shell::profile::now_ms(),
        );
        if let Some(r) = &self.replying
            && model.top.as_ref().is_none_or(|(w, _)| &w.id != r)
        {
            self.replying = None;
        }

        let sidebar = div()
            .w(px(300.))
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme.c("sidebar"))
            .border_r_1()
            .border_color(theme.c("sidebar-line"))
            .text_color(theme.c("sidebar-ink"))
            .child(
                crate::shell::drag_band()
                    .h(px(56.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .px(px(18.))
                    .text_size(px(20.))
                    .font_weight(FontWeight::BLACK)
                    .child("harbard")
                    .child(icon("chevron-down", 20., theme.c("sidebar-ink"))),
            )
            .child(
                div()
                    .flex_1()
                    .px(px(10.))
                    .pt(px(4.))
                    .child(section(t(cx, "home.channels"), &theme))
                    .child(section(t(cx, "home.agents"), &theme).mt(px(220.))),
            );

        let head = crate::shell::drag_band()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .px(px(20.))
            .py(px(14.))
            .border_b_1()
            .border_color(theme.c("line"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(18.))
                    .py(px(8.))
                    .rounded(px(12.))
                    .bg(theme.c("green-bg"))
                    .text_color(theme.c("ink"))
                    .text_size(px(18.))
                    .child(icon("cloud", 20., theme.c("green-ink")))
                    .child(t(cx, "home.calmPill")),
            )
            .child(
                hb_btn("calm-exit", false, &theme)
                    .h(px(40.))
                    .rounded(px(10.))
                    .gap(px(8.))
                    .tooltip({
                        let hint = t(cx, "home.calmExitHint");
                        move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(hint.clone())
                                .build(window, cx)
                        }
                    })
                    .on_click(|_, _, cx| crate::shell::Workspace::exit_calm(cx))
                    .child(icon("power", 16., theme.c("ink")))
                    .child(t(cx, "home.calmExit")),
            );

        let mut card = div()
            .w_full()
            .max_w(px(860.))
            .mx_auto()
            .my(px(48.))
            .pt(px(40.))
            .px(px(48.))
            .pb(px(24.))
            .border_1()
            .border_color(theme.c("line"))
            .rounded(px(16.))
            .bg(theme.c("surface-2"))
            .child(
                div()
                    .w_full()
                    .flex()
                    .justify_center()
                    .text_size(px(34.))
                    .font_weight(FontWeight::BLACK)
                    .line_height(relative(1.2))
                    .text_color(theme.c("accent-text"))
                    .child(if model.top.is_some() {
                        t(cx, "home.calmOnlyThing")
                    } else {
                        t(cx, "home.calmNothing")
                    }),
            );

        match &model.top {
            Some((w, title)) => {
                let agent = w
                    .agent
                    .as_deref()
                    .and_then(|k| view.as_ref()?.agent(k).cloned());
                let who: SharedString = match &agent {
                    Some(a) => agent_name(a, cx),
                    None => w.who.clone().into(),
                };
                let reason = t(cx, reason_key(&w.kind));
                let avatar = match &agent {
                    Some(a) => crate::ui::avatar::agent_avatar(a, 120., false, cx),
                    None => monogram_avatar(&w.who, 120., &theme),
                };
                let (question, detail, quote): (
                    SharedString,
                    Option<SharedString>,
                    Option<String>,
                ) = match title {
                    CalmTitle::Own { quote, ts_ms } => (
                        t(cx, "home.nobodyAnswered"),
                        Some(tv(
                            cx,
                            "home.youWroteTo",
                            &[("who", &who), ("time", &hhmm(*ts_ms))],
                        )),
                        Some(quote.clone()),
                    ),
                    CalmTitle::Ask {
                        title,
                        detail,
                        gone,
                    } => (
                        if title.is_empty() {
                            reason.clone()
                        } else {
                            title.clone().into()
                        },
                        if *gone {
                            Some(format!("{}.", t(cx, "workspace.sessionGone")).into())
                        } else {
                            (!detail.is_empty()).then(|| detail.clone().into())
                        },
                        None,
                    ),
                };
                let replying = self.replying.as_deref() == Some(w.id.as_str());
                let top_id = w.id.clone();
                let later_id = w.id.clone();
                card = card
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(28.))
                            .mt(px(32.))
                            .child(div().flex_none().child(avatar))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .min_w_0()
                                    .flex_1()
                                    .child(
                                        div().flex().child(
                                            div()
                                                .px(px(14.))
                                                .py(px(4.))
                                                .rounded(px(8.))
                                                .bg(theme.c("amber-bg"))
                                                .text_color(theme.c("amber-ink"))
                                                .text_size(px(18.))
                                                .font_weight(FontWeight::BOLD)
                                                .child(format!(
                                                    "{who} · {}",
                                                    reason.to_lowercase()
                                                )),
                                        ),
                                    )
                                    .child(
                                        div()
                                            .mt(px(10.))
                                            .text_size(px(26.))
                                            .font_weight(FontWeight::BLACK)
                                            .line_height(relative(1.25))
                                            .child(question),
                                    )
                                    .when_some(quote, |d, q| {
                                        d.child(
                                            div()
                                                .mt(px(12.))
                                                .px(px(16.))
                                                .py(px(10.))
                                                .border_l(px(4.))
                                                .border_color(theme.c("line"))
                                                .rounded(px(4.))
                                                .bg(theme.c("surface"))
                                                .child(
                                                    div()
                                                        .text_size(px(14.))
                                                        .font_weight(FontWeight::BOLD)
                                                        .text_color(theme.c("ink-3"))
                                                        .child(t(cx, "home.yourMessage")),
                                                )
                                                .child(
                                                    div()
                                                        .mt(px(2.))
                                                        .text_size(px(20.))
                                                        .child(format!("«{q}»")),
                                                ),
                                        )
                                    })
                                    .when_some(detail, |d, text| {
                                        d.child(
                                            div()
                                                .mt(px(6.))
                                                .text_size(px(19.))
                                                .text_color(theme.c("ink-3"))
                                                .child(text),
                                        )
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .grid()
                            .grid_cols(2)
                            .gap(px(20.))
                            .mt(px(32.))
                            .child(
                                hb_btn_base("calm-reply", false, &theme)
                                    .h(px(72.))
                                    .rounded(px(16.))
                                    .gap(px(14.))
                                    .text_size(px(24.))
                                    .border_color(theme.c("send"))
                                    .bg(theme.c("send"))
                                    .text_color(theme.c("send-ink"))
                                    .hover(|s| s.opacity(0.92))
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        if this.replying.as_deref() == Some(top_id.as_str()) {
                                            this.replying = None;
                                        } else {
                                            this.replying = Some(top_id.clone());
                                            this.error = None;
                                            this.reply.focus_handle(cx).focus(window, cx);
                                        }
                                        cx.notify();
                                    }))
                                    .child(icon("message-square-reply", 24., theme.c("send-ink")))
                                    .child(t(cx, "home.reply")),
                            )
                            .child(
                                hb_btn_base("calm-later", false, &theme)
                                    .h(px(72.))
                                    .rounded(px(16.))
                                    .gap(px(14.))
                                    .text_size(px(24.))
                                    .border_color(gpui::transparent_black())
                                    .bg(theme.c("accent-soft"))
                                    .text_color(theme.c("accent-text"))
                                    .hover(|s| s.opacity(0.92))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.later.push(later_id.clone());
                                        cx.notify();
                                    }))
                                    .child(icon("clock-3", 24., theme.c("accent-text")))
                                    .child(t(cx, "home.calmLater")),
                            ),
                    )
                    .when(replying, |d| {
                        d.child(self.reply_box(agent.as_ref(), &who, &theme, window, cx))
                    });
            }
            None => {
                card = card.child(
                    div()
                        .w_full()
                        .flex()
                        .justify_center()
                        .mt(px(6.))
                        .text_size(px(19.))
                        .text_color(theme.c("ink-3"))
                        .child(t(cx, "home.calmEmpty")),
                );
            }
        }

        card = card.child(
            div()
                .mt(px(28.))
                .pt(px(16.))
                .border_t_1()
                .border_color(theme.c("line"))
                .flex()
                .justify_center()
                .items_center()
                .gap(px(4.))
                .text_size(px(15.))
                .text_color(theme.c("ink-3"))
                .child(format!("{} ·", t(cx, "home.calmLine")))
                .child(
                    div()
                        .px(px(10.))
                        .py(px(2.))
                        .rounded(px(8.))
                        .bg(theme.c("green-bg"))
                        .text_color(theme.c("green-ink"))
                        .font_weight(FontWeight::BOLD)
                        .child(tv(
                            cx,
                            "home.workingCount",
                            &[("count", &model.working.to_string())],
                        )),
                ),
        );

        div().size_full().flex().child(sidebar).child(
            div()
                .flex_1()
                .min_w_0()
                .flex()
                .flex_col()
                .bg(theme.c("surface"))
                .child(head)
                .child(
                    div()
                        .id("calm-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px(px(24.))
                        .child(card),
                ),
        )
    }
}

impl CalmScreen {
    /// The reply box inside the card (the console's `CardActions`, the
    /// reply and "Mark resolved" parts).
    fn reply_box(
        &mut self,
        agent: Option<&crate::model::Agent>,
        who: &SharedString,
        theme: &Theme,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let resolve = hb_btn("calm-resolve", false, theme)
            .on_click(cx.listener(|this, _, _, cx| this.resolve(cx)))
            .child(t(cx, "activity.markResolved"));
        let body = match agent {
            Some(_) => {
                let placeholder = tv(cx, "activity.replyTo", &[("name", who)]);
                if self.placeholder != placeholder {
                    self.placeholder = placeholder.clone();
                    self.reply
                        .update(cx, |s, cx| s.set_placeholder(placeholder, _window, cx));
                }
                div()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .child(Textarea::new(&self.reply).w_full())
                    .child(
                        div().flex().gap(px(8.)).justify_end().child(resolve).child(
                            hb_btn("calm-send", true, theme)
                                .when(self.sending, |d| d.opacity(0.5))
                                .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))
                                .child(t(cx, "messages.send")),
                        ),
                    )
            }
            None => div()
                .flex()
                .items_center()
                .gap(px(12.))
                .child(
                    div()
                        .text_color(theme.c("ink-3"))
                        .child(t(cx, "activity.noAgentToAnswer")),
                )
                .child(resolve),
        };
        div()
            .mt(px(24.))
            .child(body)
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .mt(px(8.))
                        .text_size(px(13.))
                        .text_color(theme.c("red-ink"))
                        .child(e),
                )
            })
            .into_any_element()
    }
}
