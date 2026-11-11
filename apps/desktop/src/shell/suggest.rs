//! The desktop's away suggestion (`suggest-card.tsx`, `lib/suggest.ts`): a
//! corner card after an absence ("Set your status to Away?") or on the way
//! back ("You're back"). Not modal, never a switch: the state changes on the
//! operator's click and nothing else.

use super::calm::hb_btn;
use super::dialogs::close_button;
use super::profile::{ProfileStore, now_ms, refusal_text, state_info, tone_token};
use crate::derive::away::suggestion_for;
use crate::i18n::t;
use crate::theme::Theme;
use crate::ui::{self, control, icon, icon_size, radius, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Context, Global, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};

/// The card's width.
const WIDTH: f32 = 400.;
use serde_json::json;

/// What the desktop's away watch saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Suggestion {
    /// Ten minutes away from the Mac, or a locked screen.
    Away,
    /// Input again after being away.
    Available,
}

impl Suggestion {
    fn target(self) -> &'static str {
        match self {
            Suggestion::Away => "away",
            Suggestion::Available => "available",
        }
    }
}

/// The suggestion waiting for the operator's answer.
#[derive(Default)]
struct Pending(Option<Suggestion>);

impl Global for Pending {}

/// Offer a state; a newer suggestion replaces the one showing.
pub fn show(s: Suggestion, cx: &mut App) {
    cx.set_global(Pending(Some(s)));
    cx.refresh_windows();
}

fn dismiss(cx: &mut App) {
    cx.set_global(Pending(None));
    cx.refresh_windows();
}

pub struct SuggestCard {
    error: Option<SharedString>,
    shown: Option<Suggestion>,
}

impl SuggestCard {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        if !cx.has_global::<Pending>() {
            cx.set_global(Pending(None));
        }
        cx.observe_global::<Pending>(|_, cx| cx.notify()).detach();
        cx.observe(&ProfileStore::get(cx), |_, _, cx| cx.notify())
            .detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        SuggestCard {
            error: None,
            shown: None,
        }
    }
}

impl Render for SuggestCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let pending = cx.global::<Pending>().0;
        if pending != self.shown {
            self.shown = pending;
            self.error = None;
        }
        let store = ProfileStore::get(cx).read(cx);
        let target = match (pending, &store.view) {
            (Some(s), Some(_)) => {
                suggestion_for(s.target(), store.state_id(now_ms()), store.away_suggest())
            }
            _ => None,
        };
        let Some(target) = target else {
            return div().into_any_element();
        };
        let theme = cx.global::<Theme>().clone();
        let (title, body, accept, decline, tone) = if target == "away" {
            (
                "workspace.suggestAwayTitle",
                "workspace.suggestAwayBody",
                "workspace.suggestAwayAccept",
                "workspace.suggestNotNow",
                "grey",
            )
        } else {
            (
                "workspace.suggestBackTitle",
                "workspace.suggestBackBody",
                "workspace.suggestBackAccept",
                "workspace.suggestStayAway",
                "green",
            )
        };
        let tone_color = theme.c(tone_token(tone));
        div()
            .id("suggest-card")
            .occlude()
            .absolute()
            .right(px(space::GUTTER))
            .bottom(px(space::GUTTER))
            .w(px(WIDTH))
            .flex()
            .gap(px(space::LG))
            .pt(px(space::LG))
            .pr(px(space::MD))
            .pb(px(space::XL))
            .pl(px(space::XL))
            .rounded(px(radius::MD))
            .bg(theme.c("surface-raised"))
            .shadow(theme.pop_shadow())
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(control::SM))
                    .child(icon(state_info(target).icon, icon_size::ROW, tone_color)),
            )
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
                            .min_h(px(control::SM))
                            .child(ui::text(
                                t(cx, title),
                                size::UI,
                                weight::STRONG,
                                theme.c("ink"),
                            )),
                    )
                    .child(ui::text(
                        t(cx, body),
                        size::UI,
                        weight::REGULAR,
                        theme.c("ink-2"),
                    ))
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
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(space::MD))
                            .mt(px(space::LG))
                            .child(
                                hb_btn("suggest-accept", true, &theme)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.error = None;
                                        let task = ProfileStore::get(cx).update(cx, |p, cx| {
                                            p.update(json!({ "state": { "id": target } }), cx)
                                        });
                                        cx.spawn(async move |this, cx| {
                                            let out = task.await;
                                            let _ = this.update(cx, |this, cx| match out {
                                                Ok(_) => dismiss(cx),
                                                Err(e) => {
                                                    this.error = Some(refusal_text(&e, cx));
                                                    cx.notify();
                                                }
                                            });
                                        })
                                        .detach();
                                    }))
                                    .child(t(cx, accept)),
                            )
                            .child(
                                hb_btn("suggest-dismiss", false, &theme)
                                    .on_click(|_, _, cx| dismiss(cx))
                                    .child(t(cx, decline)),
                            ),
                    ),
            )
            .child(close_button("suggest-close", &theme, cx).on_click(|_, _, cx| dismiss(cx)))
            .into_any_element()
    }
}
