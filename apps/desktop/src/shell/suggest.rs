//! The desktop's away suggestion (`suggest-card.tsx`, `lib/suggest.ts`): a
//! corner card after an absence ("Set your status to Away?") or on the way
//! back ("You're back"). Not modal, never a switch: the state changes on the
//! operator's click and nothing else.

use super::calm::hb_btn;
use super::profile::{ProfileStore, now_ms, refusal_text, state_info, tone_token};
use crate::derive::away::suggestion_for;
use crate::i18n::t;
use crate::theme::Theme;
use crate::ui::icon;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Context, FontWeight, Global, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px, relative,
};
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
        cx.observe(&ProfileStore::get(cx), |_, _, cx| cx.notify()).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        SuggestCard { error: None, shown: None }
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
            (Some(s), Some(_)) => suggestion_for(s.target(), store.state_id(now_ms()), store.away_suggest()),
            _ => None,
        };
        let Some(target) = target else {
            return div().into_any_element();
        };
        let theme = cx.global::<Theme>().clone();
        let (title, body, accept, decline, tone) = if target == "away" {
            ("workspace.suggestAwayTitle", "workspace.suggestAwayBody", "workspace.suggestAwayAccept", "workspace.suggestNotNow", "grey")
        } else {
            ("workspace.suggestBackTitle", "workspace.suggestBackBody", "workspace.suggestBackAccept", "workspace.suggestStayAway", "green")
        };
        let tone_color = theme.c(tone_token(tone));
        div()
            .id("suggest-card")
            .occlude()
            .absolute()
            .right(px(20.))
            .bottom(px(20.))
            .w(px(400.))
            .flex()
            .gap(px(12.))
            .pt(px(16.))
            .pr(px(12.))
            .pb(px(16.))
            .pl(px(16.))
            .rounded(px(12.))
            .bg(theme.c("surface-raised"))
            .text_color(theme.c("ink"))
            .shadow(theme.pop_shadow())
            .child(
                div()
                    .size(px(36.))
                    .flex_none()
                    .rounded(px(9.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(tone_color.opacity(0.16))
                    .child(icon(state_info(target).icon, 20., tone_color)),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(div().text_size(px(16.)).font_weight(FontWeight::BLACK).child(t(cx, title)))
                    .child(
                        div()
                            .mt(px(4.))
                            .text_size(px(14.))
                            .line_height(relative(1.4))
                            .text_color(theme.c("ink-2"))
                            .child(t(cx, body)),
                    )
                    .when_some(self.error.clone(), |d, e| {
                        d.child(div().mt(px(4.)).text_size(px(13.)).text_color(theme.c("red-ink")).child(e))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(8.))
                            .mt(px(12.))
                            .child(
                                hb_btn("suggest-accept", true, &theme)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.error = None;
                                        let task = ProfileStore::get(cx)
                                            .update(cx, |p, cx| p.update(json!({ "state": { "id": target } }), cx));
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
                            .child(hb_btn("suggest-dismiss", false, &theme).on_click(|_, _, cx| dismiss(cx)).child(t(cx, decline))),
                    ),
            )
            .child(
                div()
                    .id("suggest-close")
                    .size(px(32.))
                    .flex_none()
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.c("control")))
                    .on_click(|_, _, cx| dismiss(cx))
                    .child(icon("x", 16., theme.c("ink-3"))),
            )
            .into_any_element()
    }
}
