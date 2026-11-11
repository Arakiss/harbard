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
use crate::ui::kit_theme::{self, FocusRing as _};
use crate::ui::{
    self, avatar_size, control, format::hhmm, icon, icon_size, leading, radius, row, size, space,
    weight,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, Div, ElementId, Entity, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, Stateful, StatefulInteractiveElement, Styled,
    Window, div, px,
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

/// A `.hb-btn`: 32 px, a surface with a strong edge (or `accent-deep` for
/// the one primary action), 6px corners, 14/700.
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
        .flex_none()
        .items_center()
        .justify_center()
        .gap(px(space::SM))
        .h(px(control::MD))
        .px(px(space::LG))
        .rounded(px(radius::BASE))
        .border_1()
        .border_color(edge)
        .bg(bg)
        .text_color(ink)
        .font_family(ui::SANS)
        .text_size(px(size::UI))
        .line_height(px(leading::UI))
        .font_weight(weight::STRONG)
        .whitespace_nowrap()
        .cursor_pointer()
        .focus_ring(theme, false)
}

/// A name's monogram tile (`Avatar`), when there is no agent to draw.
pub(super) fn monogram_avatar(name: &str, size: f32, theme: &Theme) -> AnyElement {
    div()
        .size(px(size))
        .flex_none()
        .rounded(px(radius::MD))
        .bg(theme.tone_for(name))
        .flex()
        .items_center()
        .justify_center()
        .child(ui::text(
            ui::format::monogram(name),
            if size < avatar_size::ROW {
                size::META
            } else {
                size::UI
            },
            weight::STRONG,
            theme.c("av-ink"),
        ))
        .into_any_element()
}

/// A sidebar heading (T5): sentence case, 12/700 in `sidebar-ink-2`.
fn section(label: SharedString, theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(row::NAV))
        .px(px(space::MD))
        .child(ui::text(
            label,
            size::META,
            weight::STRONG,
            theme.c("sidebar-ink-2"),
        ))
}

/// The card's width, and the sidebar's.
const CARD: f32 = 640.;
const SIDEBAR: f32 = 300.;

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
            .w(px(SIDEBAR))
            .flex_none()
            .flex()
            .flex_col()
            .bg(theme.c("sidebar"))
            .border_r_1()
            .border_color(theme.c("sidebar-line"))
            .child(
                crate::shell::drag_band()
                    .h(px(row::HEADER))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(space::SM))
                    .px(px(space::GUTTER))
                    .child(ui::text(
                        "harbard",
                        size::TITLE,
                        weight::HEAVY,
                        theme.c("sidebar-ink"),
                    ))
                    .child(icon(
                        "chevron-down",
                        icon_size::ROW,
                        theme.c("sidebar-ink-2"),
                    )),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .flex_col()
                    .gap(px(space::SECTION))
                    .px(px(space::LG))
                    .pt(px(space::MD))
                    .child(section(t(cx, "home.channels"), &theme))
                    .child(section(t(cx, "home.agents"), &theme)),
            );

        let head = crate::shell::drag_band()
            .h(px(row::HEADER))
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(space::MD))
            .px(px(space::GUTTER))
            .border_b_1()
            .border_color(theme.c("line"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::SM))
                    .min_w_0()
                    .child(icon("cloud", icon_size::ROW, theme.c("green-ink")))
                    .child(
                        ui::text(
                            t(cx, "home.calmPill"),
                            size::UI,
                            weight::REGULAR,
                            theme.c("ink-2"),
                        )
                        .truncate(),
                    ),
            )
            .child(
                hb_btn("calm-exit", false, &theme)
                    .h(px(control::SM))
                    .px(px(space::MD))
                    .tooltip(kit_theme::tip(t(cx, "home.calmExitHint")))
                    .on_click(|_, _, cx| crate::shell::Workspace::exit_calm(cx))
                    .child(icon("power", icon_size::ROW, theme.c("ink-2")))
                    .child(t(cx, "home.calmExit")),
            );

        // The one thing: a heading, the item, its two actions, one calm line.
        let mut card = div()
            .w_full()
            .max_w(px(CARD))
            .mx_auto()
            .mt(px(space::BREAK))
            .mb(px(space::SECTION))
            .flex()
            .flex_col()
            .child(
                div()
                    .font_family(ui::DISPLAY)
                    .text_size(px(size::DISPLAY))
                    .line_height(px(leading::DISPLAY))
                    .font_weight(weight::STRONG)
                    .text_color(theme.c("ink"))
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
                    Some(a) => crate::ui::avatar::agent_avatar(a, avatar_size::ROW, false, cx),
                    None => monogram_avatar(&w.who, avatar_size::ROW, &theme),
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
                let send = theme.c("send");
                card = card.child(
                    div()
                        .flex()
                        .gap(px(space::LG))
                        .mt(px(space::SECTION))
                        .child(div().flex_none().child(avatar))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .flex_1()
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(space::MD))
                                        .child(
                                            ui::text(
                                                who.clone(),
                                                size::BODY,
                                                weight::HEAVY,
                                                theme.c("ink"),
                                            )
                                            .min_w_0()
                                            .truncate(),
                                        )
                                        .child(ui::chip(reason.clone(), "amber", &theme)),
                                )
                                .child(
                                    ui::text(question, size::TITLE, weight::HEAVY, theme.c("ink"))
                                        .mt(px(space::XS))
                                        .line_clamp(3),
                                )
                                .when_some(quote, |d, q| {
                                    d.child(
                                        div()
                                            .mt(px(space::MD))
                                            .pl(px(space::LG))
                                            .border_l_1()
                                            .border_color(theme.c("line-strong"))
                                            .child(ui::section_label(
                                                t(cx, "home.yourMessage"),
                                                &theme,
                                            ))
                                            .child(
                                                ui::text(
                                                    format!("«{q}»"),
                                                    size::BODY,
                                                    weight::REGULAR,
                                                    theme.c("ink-2"),
                                                )
                                                .line_clamp(4),
                                            ),
                                    )
                                })
                                .when_some(detail, |d, text| {
                                    d.child(
                                        ui::text(
                                            text,
                                            size::BODY,
                                            weight::REGULAR,
                                            theme.c("ink-2"),
                                        )
                                        .mt(px(space::XS))
                                        .line_clamp(4),
                                    )
                                })
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(px(space::MD))
                                        .mt(px(space::XL))
                                        .child(
                                            hb_btn_base("calm-reply", false, &theme)
                                                .border_color(send)
                                                .bg(send)
                                                .text_color(theme.c("send-ink"))
                                                .hover(move |s| s.bg(send.opacity(0.9)))
                                                .on_click(cx.listener(
                                                    move |this, _, window, cx| {
                                                        if this.replying.as_deref()
                                                            == Some(top_id.as_str())
                                                        {
                                                            this.replying = None;
                                                        } else {
                                                            this.replying = Some(top_id.clone());
                                                            this.error = None;
                                                            this.reply
                                                                .focus_handle(cx)
                                                                .focus(window, cx);
                                                        }
                                                        cx.notify();
                                                    },
                                                ))
                                                .child(icon(
                                                    "message-square-reply",
                                                    icon_size::ROW,
                                                    theme.c("send-ink"),
                                                ))
                                                .child(t(cx, "home.reply")),
                                        )
                                        .child(
                                            hb_btn("calm-later", false, &theme)
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.later.push(later_id.clone());
                                                    cx.notify();
                                                }))
                                                .child(icon(
                                                    "clock-3",
                                                    icon_size::ROW,
                                                    theme.c("ink-2"),
                                                ))
                                                .child(t(cx, "home.calmLater")),
                                        ),
                                )
                                .when(replying, |d| {
                                    d.child(self.reply_box(
                                        agent.as_ref(),
                                        &who,
                                        &theme,
                                        window,
                                        cx,
                                    ))
                                }),
                        ),
                );
            }
            None => {
                card = card.child(
                    ui::text(
                        t(cx, "home.calmEmpty"),
                        size::UI,
                        weight::REGULAR,
                        theme.c("ink-3"),
                    )
                    .mt(px(space::XS)),
                );
            }
        }

        card = card.child(
            div()
                .mt(px(space::SECTION))
                .pt(px(space::LG))
                .border_t_1()
                .border_color(theme.c("line"))
                .flex()
                .items_center()
                .gap(px(space::XS))
                .child(ui::text(
                    format!("{} ·", t(cx, "home.calmLine")),
                    size::SMALL,
                    weight::REGULAR,
                    theme.c("ink-3"),
                ))
                .child(ui::text(
                    tv(
                        cx,
                        "home.workingCount",
                        &[("count", &model.working.to_string())],
                    ),
                    size::SMALL,
                    weight::REGULAR,
                    if model.working > 0 {
                        theme.c("green-ink")
                    } else {
                        theme.c("ink-3")
                    },
                )),
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
                        .px(px(space::GUTTER))
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
                    .gap(px(space::MD))
                    .child(Textarea::new(&self.reply).w_full())
                    .child(
                        div()
                            .flex()
                            .gap(px(space::MD))
                            .justify_end()
                            .child(resolve)
                            .child(
                                hb_btn("calm-send", true, theme)
                                    .when(self.sending, |d| d.opacity(0.5))
                                    .tooltip(kit_theme::tip_keys(
                                        t(cx, "messages.send"),
                                        &["⌘", "↵"],
                                    ))
                                    .on_click(
                                        cx.listener(|this, _, window, cx| this.send(window, cx)),
                                    )
                                    .child(t(cx, "messages.send")),
                            ),
                    )
            }
            None => div()
                .flex()
                .items_center()
                .gap(px(space::LG))
                .child(
                    ui::text(
                        t(cx, "activity.noAgentToAnswer"),
                        size::UI,
                        weight::REGULAR,
                        theme.c("ink-3"),
                    )
                    .flex_1()
                    .min_w_0(),
                )
                .child(resolve),
        };
        div()
            .mt(px(space::XL))
            .child(body)
            .when_some(self.error.clone(), |d, e| {
                d.child(
                    div()
                        .mt(px(space::MD))
                        .px(px(space::MD))
                        .py(px(space::XS))
                        .rounded(px(radius::BASE))
                        .bg(theme.c("red-bg"))
                        .child(ui::text(
                            e,
                            size::SMALL,
                            weight::REGULAR,
                            theme.c("red-ink"),
                        )),
                )
            })
            .into_any_element()
    }
}
