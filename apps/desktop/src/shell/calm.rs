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
use crate::ui::pill_people::{self as character, robot_px, type_px};
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

/// The sidebar's width.
const SIDEBAR: f32 = 300.;

// `.hb-calm-*` in the console's `workspace.css` (§3.1): the calm screen keeps
// its large type, its 120 px robot and its 72 px buttons.
const HEAD_PAD: f32 = 14.;
const PILL_GAP: f32 = 10.;
const PILL_PAD_X: f32 = 18.;
const EXIT_H: f32 = 40.;
const EXIT_RADIUS: f32 = 10.;
const CARD: f32 = 860.;
const CARD_MARGIN: f32 = 48.;
const CARD_PAD_TOP: f32 = 40.;
const CARD_PAD_X: f32 = 48.;
const CARD_RADIUS: f32 = 16.;
const ITEM_GAP: f32 = 28.;
const CHIP_PAD_X: f32 = 14.;
const QUESTION_GAP: f32 = 10.;
const QUOTE_PAD_Y: f32 = 10.;
const QUOTE_TEXT: f32 = 20.;
const BIG_BTN_H: f32 = 72.;
const BIG_BTN_GAP: f32 = 14.;
const BIG_ICON: f32 = 24.;
const LINE_GAP: f32 = 28.;

/// `.hb-calm-chip`'s words: who, and why in lower case.
pub(super) fn calm_chip(who: &str, reason: &str) -> String {
    format!("{who} · {}", reason.to_lowercase())
}

/// `.hb-calm-actions .hb-btn`: half the row, 72 px, 24 px words, 16 px corners.
fn big_btn(id: &'static str, theme: &Theme) -> Stateful<Div> {
    div()
        .id(id)
        .occlude()
        .flex_1()
        .flex()
        .items_center()
        .justify_center()
        .gap(px(BIG_BTN_GAP))
        .h(px(BIG_BTN_H))
        .rounded(px(CARD_RADIUS))
        .font_family(ui::SANS)
        .text_size(px(type_px::LARGE))
        .line_height(px(character::line_for(type_px::LARGE)))
        .font_weight(weight::STRONG)
        .whitespace_nowrap()
        .cursor_pointer()
        .focus_ring(theme, false)
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

        // `.hb-calm-head`: the state as a green pill, and the way out.
        let head = crate::shell::drag_band()
            .flex_none()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(space::MD))
            .px(px(space::GUTTER))
            .py(px(HEAD_PAD))
            .border_b_1()
            .border_color(theme.c("line"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(PILL_GAP))
                    .min_w_0()
                    .px(px(PILL_PAD_X))
                    .py(px(space::MD))
                    .rounded(px(radius::LG))
                    .bg(theme.c("green-bg"))
                    .child(icon("cloud", icon_size::RAIL, theme.c("green-ink")))
                    .child(
                        character::text_px(
                            t(cx, "home.calmPill"),
                            type_px::CHIP,
                            weight::REGULAR,
                            theme.c("ink"),
                        )
                        .truncate(),
                    ),
            )
            .child(
                hb_btn("calm-exit", false, &theme)
                    .h(px(EXIT_H))
                    .px(px(space::LG))
                    .rounded(px(EXIT_RADIUS))
                    .gap(px(space::MD))
                    .tooltip(kit_theme::tip(t(cx, "home.calmExitHint")))
                    .on_click(|_, _, cx| crate::shell::Workspace::exit_calm(cx))
                    .child(icon("power", icon_size::ROW, theme.c("ink-2")))
                    .child(t(cx, "home.calmExit")),
            );

        // `.hb-calm-card`: the one thing, large, on its own card.
        let mut card = div()
            .w_full()
            .max_w(px(CARD))
            .mx_auto()
            .my(px(CARD_MARGIN))
            .pt(px(CARD_PAD_TOP))
            .px(px(CARD_PAD_X))
            .pb(px(space::SECTION))
            .border_1()
            .border_color(theme.c("line"))
            .rounded(px(CARD_RADIUS))
            .bg(theme.c("surface-2"))
            .flex()
            .flex_col()
            .child(
                character::text_px(
                    if model.top.is_some() {
                        t(cx, "home.calmOnlyThing")
                    } else {
                        t(cx, "home.calmNothing")
                    },
                    type_px::HERO,
                    weight::HEAVY,
                    theme.c("accent-text"),
                )
                .text_center(),
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
                    Some(a) => crate::ui::avatar::agent_avatar(a, robot_px::CALM, false, cx),
                    None => monogram_avatar(&w.who, robot_px::CALM, &theme),
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
                let soft = theme.c("accent-soft");
                card = card
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(ITEM_GAP))
                            .mt(px(space::BREAK))
                            .child(div().flex_none().child(avatar))
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .items_start()
                                    .min_w_0()
                                    .flex_1()
                                    // `.hb-calm-chip`: who and why, in the
                                    // waiting state's wash.
                                    .child(
                                        div()
                                            .max_w_full()
                                            .px(px(CHIP_PAD_X))
                                            .py(px(space::XS))
                                            .rounded(px(radius::MD))
                                            .bg(theme.c("amber-bg"))
                                            .child(
                                                character::text_px(
                                                    calm_chip(&who, &reason),
                                                    type_px::CHIP,
                                                    weight::STRONG,
                                                    theme.c("amber-ink"),
                                                )
                                                .truncate(),
                                            ),
                                    )
                                    .child(
                                        character::text_px(
                                            question,
                                            type_px::QUESTION,
                                            weight::HEAVY,
                                            theme.c("ink"),
                                        )
                                        .mt(px(QUESTION_GAP))
                                        .line_clamp(3)
                                        .text_ellipsis(),
                                    )
                                    .when_some(quote, |d, q| {
                                        d.child(
                                            div()
                                                .w_full()
                                                .mt(px(space::LG))
                                                .px(px(space::XL))
                                                .py(px(QUOTE_PAD_Y))
                                                .border_l(px(space::XS))
                                                .border_color(theme.c("line"))
                                                .rounded(px(radius::SM))
                                                .bg(theme.c("surface"))
                                                .child(character::text_px(
                                                    t(cx, "home.yourMessage"),
                                                    size::UI,
                                                    weight::STRONG,
                                                    theme.c("ink-3"),
                                                ))
                                                .child(
                                                    character::text_px(
                                                        format!("«{q}»"),
                                                        QUOTE_TEXT,
                                                        weight::REGULAR,
                                                        theme.c("ink"),
                                                    )
                                                    .mt(px(space::HAIR))
                                                    .line_clamp(4)
                                                    .text_ellipsis(),
                                                ),
                                        )
                                    })
                                    .when_some(detail, |d, text| {
                                        d.child(
                                            character::text_px(
                                                text,
                                                type_px::DETAIL,
                                                weight::REGULAR,
                                                theme.c("ink-3"),
                                            )
                                            .mt(px(space::SM))
                                            .line_clamp(4)
                                            .text_ellipsis(),
                                        )
                                    }),
                            ),
                    )
                    // `.hb-calm-actions`: two large buttons, half each.
                    .child(
                        div()
                            .flex()
                            .gap(px(space::GUTTER))
                            .mt(px(space::BREAK))
                            .child(
                                big_btn("calm-reply", &theme)
                                    .bg(send)
                                    .text_color(theme.c("send-ink"))
                                    .hover(move |s| s.bg(send.opacity(0.9)))
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
                                    .child(icon(
                                        "message-square-reply",
                                        BIG_ICON,
                                        theme.c("send-ink"),
                                    ))
                                    .child(t(cx, "home.reply")),
                            )
                            .child(
                                big_btn("calm-later", &theme)
                                    .bg(soft)
                                    .text_color(theme.c("accent-text"))
                                    .hover(move |s| s.bg(soft.opacity(0.8)))
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.later.push(later_id.clone());
                                        cx.notify();
                                    }))
                                    .child(icon("clock-3", BIG_ICON, theme.c("accent-text")))
                                    .child(t(cx, "home.calmLater")),
                            ),
                    )
                    .when(replying, |d| {
                        d.child(self.reply_box(agent.as_ref(), &who, &theme, window, cx))
                    });
            }
            None => {
                card = card.child(
                    character::text_px(
                        t(cx, "home.calmEmpty"),
                        type_px::DETAIL,
                        weight::REGULAR,
                        theme.c("ink-3"),
                    )
                    .mt(px(space::SM))
                    .text_center(),
                );
            }
        }

        // `.hb-calm-line`: the calm line, and who works, in green.
        card = card.child(
            div()
                .mt(px(LINE_GAP))
                .pt(px(space::XL))
                .border_t_1()
                .border_color(theme.c("line"))
                .flex()
                .flex_wrap()
                .items_center()
                .justify_center()
                .gap(px(space::SM))
                .child(character::text_px(
                    format!("{} ·", t(cx, "home.calmLine")),
                    size::BODY,
                    weight::REGULAR,
                    theme.c("ink-3"),
                ))
                .child({
                    let working = tv(
                        cx,
                        "home.workingCount",
                        &[("count", &model.working.to_string())],
                    );
                    if model.working > 0 {
                        character::tone_pill(working, "green", size::BODY, &theme)
                    } else {
                        character::text_px(working, size::BODY, weight::REGULAR, theme.c("ink-3"))
                    }
                }),
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
            .mt(px(space::SECTION))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_chip_says_who_then_why_in_lower_case() {
        assert_eq!(calm_chip("Roy", "Blocked"), "Roy · blocked");
        assert_eq!(calm_chip("Ana", "Sin respuesta"), "Ana · sin respuesta");
    }

    #[test]
    fn calm_holds_in_swamped_until_left_for_that_swamped() {
        assert!(calm_on("swamped", 10, None));
        assert!(!calm_on("swamped", 10, Some(10)));
        assert!(calm_on("swamped", 11, Some(10)));
        assert!(!calm_on("available", 10, None));
    }
}
