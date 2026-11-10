//! "While you were away" (`away-card.tsx`, `lib/away.ts`): a modal over the
//! shell when the profile store holds an [`super::profile::Away`]. What
//! waits for the operator by urgency, what finished, and who still works;
//! then back to Available (or Done) or on to Activity.

use super::calm::{hb_btn, monogram_avatar};
use super::profile::{Away, ProfileStore, refusal_text, state_info};
use crate::derive::away::{away_model, away_span, reason_key, tone};
use crate::derive::{agent_name, presence_line};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::nav::{Route, navigate};
use crate::theme::Theme;
use crate::ui::icon;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Context, Div, FocusHandle, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px, relative,
};
use serde_json::json;

pub struct AwayCard {
    focus: FocusHandle,
    /// Which away the local state below belongs to (digest id and start).
    key: Option<(Option<String>, u64)>,
    /// Items put off with "Later", for this card only.
    later: Vec<String>,
    error: Option<SharedString>,
}

impl AwayCard {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&ProfileStore::get(cx), |_, _, cx| cx.notify()).detach();
        cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        AwayCard {
            focus: cx.focus_handle(),
            key: None,
            later: Vec::new(),
            error: None,
        }
    }
}

fn close(cx: &mut App) {
    ProfileStore::get(cx).update(cx, |p, cx| p.close_away(cx));
}

/// The title per state: Away is "While you were away"; the others say which
/// state held it.
fn title_of(away: &Away, cx: &App) -> String {
    let span = away_span(away.since, away.until);
    let head: SharedString = match away.state.as_str() {
        "away" => t(cx, "home.whileAway"),
        "done" => t(cx, "home.sinceEndOfDay"),
        other => {
            let label = state_info(other).label(cx).to_lowercase();
            tv(cx, "home.whileYouWere", &[("state", &label)])
        }
    };
    match span {
        Some(s) => format!("{head} · {s}"),
        None => head.to_string(),
    }
}

/// `.hb-chip` with its tone.
fn chip(text: SharedString, tone: &str, theme: &Theme) -> Div {
    let (bg, ink, ring) = match tone {
        "red" => (theme.c("red-bg"), theme.c("red-ink"), Some(theme.c("red-line"))),
        "amber" => (theme.c("amber-bg"), theme.c("amber-ink"), None),
        "green" => (theme.c("green-bg"), theme.c("green-ink"), Some(theme.c("green-line"))),
        _ => (theme.c("chip"), theme.c("chip-ink"), None),
    };
    div()
        .flex()
        .items_center()
        .h(px(22.))
        .px(px(8.))
        .rounded(px(6.))
        .bg(bg)
        .text_color(ink)
        .text_size(px(13.))
        .font_weight(FontWeight::BOLD)
        .when_some(ring, |d, r| d.border_1().border_color(r))
        .child(text)
}

fn section(icon_name: &'static str, icon_color: gpui::Hsla, title: SharedString, theme: &Theme) -> Div {
    div()
        .flex_none()
        .flex()
        .flex_col()
        .border_1()
        .border_color(theme.c("line"))
        .rounded(px(12.))
        .overflow_hidden()
        .bg(theme.c("surface"))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(10.))
                .px(px(16.))
                .py(px(10.))
                .text_size(px(18.))
                .font_weight(FontWeight::BLACK)
                .bg(theme.c("tint"))
                .border_b_1()
                .border_color(theme.c("line"))
                .child(icon(icon_name, 20., icon_color))
                .child(title),
        )
}

fn empty(text: SharedString, theme: &Theme) -> Div {
    div().px(px(16.)).py(px(10.)).text_size(px(14.)).text_color(theme.c("ink-3")).child(text)
}

impl Render for AwayCard {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(away) = ProfileStore::get(cx).read(cx).away.clone() else {
            self.key = None;
            return div().into_any_element();
        };
        let key = Some((away.digest.clone(), away.since));
        if self.key != key {
            self.key = key;
            self.later.clear();
            self.error = None;
            self.focus.focus(window, cx);
        }
        let theme = cx.global::<Theme>().clone();
        let view = Desk::get(cx).read(cx).view.clone();
        let now = Desk::get(cx).read(cx).now_ms();
        let model = away_model(view.as_deref(), &away.items, away.since, away.until);
        let waiting: Vec<_> = model.waiting.iter().filter(|w| !self.later.contains(&w.id)).cloned().collect();

        // Waiting for you.
        let mut wait_section = section(
            "inbox",
            theme.c("ink"),
            tv(cx, "home.waitingForYou", &[("count", &waiting.len().to_string())]),
            &theme,
        );
        if waiting.is_empty() {
            wait_section = wait_section.child(empty(t(cx, "home.nothingWaiting"), &theme));
        }
        for (i, w) in waiting.iter().enumerate() {
            let agent = w.agent.as_deref().and_then(|k| view.as_ref()?.agent(k).cloned());
            let who: SharedString = agent.as_ref().map(|a| agent_name(a, cx)).unwrap_or_else(|| w.who.clone().into());
            let avatar = match &agent {
                Some(a) => crate::ui::avatar::agent_avatar(a, 48., false, cx),
                None => monogram_avatar(&w.who, 48., &theme),
            };
            let route = if let Some(card) = w.id.strip_prefix("card:") {
                Route::Activity {
                    tab: "needs".into(),
                    sel: Some(format!("card:{card}")),
                }
            } else if let Some(a) = &w.agent {
                Route::direct(Some(a.clone()))
            } else {
                Route::activity()
            };
            let later_id = w.id.clone();
            wait_section = wait_section.child(
                div()
                    .flex()
                    .gap(px(14.))
                    .px(px(16.))
                    .py(px(12.))
                    .when(i > 0, |d| d.border_t_1().border_color(theme.c("line")))
                    .child(div().flex_none().child(avatar))
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
                                    .text_size(px(16.))
                                    .child(div().font_weight(FontWeight::BOLD).child(who))
                                    .child(chip(t(cx, reason_key(&w.kind)), tone(&w.kind), &theme)),
                            )
                            .when(!w.text.is_empty(), |d| d.child(div().mt(px(2.)).text_size(px(15.)).child(w.text.clone())))
                            .child(
                                div()
                                    .flex()
                                    .gap(px(8.))
                                    .mt(px(8.))
                                    .child(
                                        hb_btn(gpui::ElementId::Name(format!("away-open-{}", w.id).into()), true, &theme)
                                            .h(px(32.))
                                            .text_size(px(14.))
                                            .on_click(move |_, _, cx| {
                                                navigate(route.clone(), cx);
                                                close(cx);
                                            })
                                            .child(t(cx, "home.open")),
                                    )
                                    .child(
                                        hb_btn(gpui::ElementId::Name(format!("away-later-{}", w.id).into()), false, &theme)
                                            .h(px(32.))
                                            .text_size(px(14.))
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.later.push(later_id.clone());
                                                cx.notify();
                                            }))
                                            .child(t(cx, "home.later")),
                                    ),
                            ),
                    ),
            );
        }

        // Finished.
        let mut done_section = section(
            "circle-check",
            theme.c("green"),
            tv(cx, "home.finishedCount", &[("count", &model.done.len().to_string())]),
            &theme,
        );
        if model.done.is_empty() {
            done_section = done_section.child(empty(t(cx, "home.nothingFinished"), &theme));
        } else {
            done_section = done_section.child(div().flex().flex_col().gap(px(6.)).pt(px(10.)).pb(px(12.)).pl(px(22.)).pr(px(16.)).children(
                model.done.iter().map(|d| {
                    let text: SharedString = match &d.branch {
                        Some(b) => tv(cx, "home.deliveryVerified", &[("branch", b)]),
                        None => d.text.clone().into(),
                    };
                    div()
                        .flex()
                        .items_center()
                        .gap(px(8.))
                        .child(icon("circle-check", 16., theme.c("green")))
                        .child(
                            div()
                                .flex()
                                .flex_wrap()
                                .px(px(10.))
                                .py(px(2.))
                                .rounded(px(6.))
                                .bg(theme.c("green-bg"))
                                .text_color(theme.c("green-ink"))
                                .text_size(px(14.))
                                .child(text)
                                .when(!d.who.is_empty(), |el| {
                                    el.child(div().text_color(theme.c("ink-3")).child(format!("\u{a0}· {}", d.who)))
                                }),
                        )
                }),
            ));
        }

        // Still running.
        let mut run_section = section(
            "refresh-cw",
            theme.c("ink"),
            tv(cx, "home.stillRunning", &[("count", &model.running.len().to_string())]),
            &theme,
        );
        if model.running.is_empty() {
            run_section = run_section.child(empty(t(cx, "home.noAgentWorking"), &theme));
        }
        for (i, key) in model.running.iter().enumerate() {
            let Some(a) = view.as_ref().and_then(|v| v.agent(key)) else { continue };
            run_section = run_section.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(16.))
                    .py(px(8.))
                    .text_size(px(14.))
                    .when(i > 0, |d| d.border_t_1().border_color(theme.c("line")))
                    .child(div().flex_none().child(crate::ui::avatar::agent_avatar(a, 32., false, cx)))
                    .child(div().flex_none().font_weight(FontWeight::BOLD).child(agent_name(a, cx)))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_color(theme.c("ink-2"))
                            .child(a.description.clone().unwrap_or_default()),
                    )
                    .child(div().flex_none().text_size(px(13.)).text_color(theme.c("ink-3")).child(presence_line(a, now, cx))),
            );
        }

        let primary = if away.still_away {
            hb_btn("away-available", true, &theme)
                .gap(px(10.))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.error = None;
                    let task = ProfileStore::get(cx).update(cx, |p, cx| p.update(json!({ "state": { "id": "available" } }), cx));
                    cx.spawn(async move |this, cx| {
                        let out = task.await;
                        let _ = this.update(cx, |this, cx| match out {
                            Ok(_) => close(cx),
                            Err(e) => {
                                this.error = Some(refusal_text(&e, cx));
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                }))
                .child(
                    div()
                        .size(px(12.))
                        .rounded_full()
                        .bg(theme.c("green"))
                        .border_1()
                        .border_color(theme.c("green-edge")),
                )
                .child(t(cx, "home.setAvailable"))
        } else {
            hb_btn("away-done", true, &theme).on_click(|_, _, cx| close(cx)).child(t(cx, "home.done"))
        };

        let card = div()
            .id("away-card")
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(760.))
            .max_w(window.viewport_size().width - px(48.))
            .max_h(window.viewport_size().height - px(48.))
            .flex()
            .flex_col()
            .rounded(px(12.))
            .bg(theme.c("surface-raised"))
            .text_color(theme.c("ink"))
            .shadow(theme.pop_shadow())
            .overflow_hidden()
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .pt(px(18.))
                    .pr(px(20.))
                    .pb(px(12.))
                    .pl(px(24.))
                    .child(
                        div()
                            .size(px(38.))
                            .flex_none()
                            .rounded(px(9.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .bg(theme.c("accent"))
                            .child(icon("clock-3", 20., theme.c("accent-ink"))),
                    )
                    .child(
                        div()
                            .text_size(px(24.))
                            .font_weight(FontWeight::BLACK)
                            .line_height(relative(1.25))
                            .child(title_of(&away, cx)),
                    )
                    .child(
                        div()
                            .id("away-close")
                            .ml_auto()
                            .size(px(32.))
                            .flex_none()
                            .rounded(px(6.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|s| s.bg(theme.c("control")))
                            .on_click(|_, _, cx| close(cx))
                            .child(icon("x", 20., theme.c("ink-3"))),
                    ),
            )
            .child(
                div()
                    .id("away-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(14.))
                    .pt(px(4.))
                    .px(px(24.))
                    .pb(px(8.))
                    .child(wait_section)
                    .child(done_section)
                    .child(run_section)
                    .when_some(self.error.clone(), |d, e| {
                        d.child(div().text_size(px(13.)).text_color(theme.c("red-ink")).child(e))
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .grid()
                    .grid_cols(2)
                    .gap(px(14.))
                    .pt(px(14.))
                    .px(px(24.))
                    .pb(px(20.))
                    .child(primary.h(px(46.)).text_size(px(17.)))
                    .child(
                        hb_btn("away-activity", false, &theme)
                            .h(px(46.))
                            .text_size(px(17.))
                            .gap(px(10.))
                            .on_click(|_, _, cx| {
                                navigate(Route::activity(), cx);
                                close(cx);
                            })
                            .child(icon("list", 20., theme.c("ink")))
                            .child(t(cx, "home.seeAllInActivity")),
                    ),
            );

        div()
            .id("away-scrim")
            .track_focus(&self.focus)
            .key_context("AwayCard")
            .on_key_down(|ev, _, cx| {
                if ev.keystroke.key == "escape" {
                    cx.stop_propagation();
                    close(cx);
                }
            })
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_center()
            .justify_center()
            .bg(theme.c("scrim"))
            .on_mouse_down(MouseButton::Left, |_, _, cx| close(cx))
            .child(card)
            .into_any_element()
    }
}
