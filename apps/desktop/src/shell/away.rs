//! "While you were away" (`away-card.tsx`, `lib/away.ts`): a modal over the
//! shell when the profile store holds an [`super::profile::Away`]. What
//! waits for the operator by urgency, what finished, and who still works;
//! then back to Available (or Done) or on to Activity.

use super::calm::{hb_btn, monogram_avatar};
use super::dialogs::close_button;
use super::profile::{Away, ProfileStore, refusal_text, state_info};
use crate::derive::away::{away_model, away_span, reason_key, tone};
use crate::derive::{agent_name, presence_line};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::nav::{Route, navigate};
use crate::theme::Theme;
use crate::ui::{self, avatar_size, control, icon, icon_size, radius, row, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, Context, Div, FocusHandle, InteractiveElement, IntoElement, MouseButton, ParentElement,
    Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
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
        cx.observe(&ProfileStore::get(cx), |_, _, cx| cx.notify())
            .detach();
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

/// The card's width.
const WIDTH: f32 = 640.;

/// A section of the card: its mark and name, then its rows. Sections are
/// set apart by space, rows by a hairline.
fn section(
    icon_name: &'static str,
    icon_color: gpui::Hsla,
    title: SharedString,
    theme: &Theme,
) -> Div {
    div().flex_none().flex().flex_col().child(
        div()
            .flex()
            .items_center()
            .gap(px(space::SM))
            .h(px(row::NAV))
            .child(icon(icon_name, icon_size::INLINE, icon_color))
            .child(ui::section_label(title, theme)),
    )
}

fn empty(text: SharedString, theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .min_h(px(row::NAV))
        .child(ui::text(text, size::UI, weight::REGULAR, theme.c("ink-3")))
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
        let waiting: Vec<_> = model
            .waiting
            .iter()
            .filter(|w| !self.later.contains(&w.id))
            .cloned()
            .collect();

        // Waiting for you.
        let mut wait_section = section(
            "inbox",
            theme.c("ink-3"),
            tv(
                cx,
                "home.waitingForYou",
                &[("count", &waiting.len().to_string())],
            ),
            &theme,
        );
        if waiting.is_empty() {
            wait_section = wait_section.child(empty(t(cx, "home.nothingWaiting"), &theme));
        }
        for (i, w) in waiting.iter().enumerate() {
            let agent = w
                .agent
                .as_deref()
                .and_then(|k| view.as_ref()?.agent(k).cloned());
            let who: SharedString = agent
                .as_ref()
                .map(|a| agent_name(a, cx))
                .unwrap_or_else(|| w.who.clone().into());
            let avatar = match &agent {
                Some(a) => crate::ui::avatar::agent_avatar(a, avatar_size::ROW, false, cx),
                None => monogram_avatar(&w.who, avatar_size::ROW, &theme),
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
                    .gap(px(space::LG))
                    .py(px(space::LG))
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
                                    .gap(px(space::MD))
                                    .child(
                                        ui::text(who, size::BODY, weight::HEAVY, theme.c("ink"))
                                            .min_w_0()
                                            .truncate(),
                                    )
                                    .child(ui::chip(
                                        t(cx, reason_key(&w.kind)),
                                        tone(&w.kind),
                                        &theme,
                                    )),
                            )
                            .when(!w.text.is_empty(), |d| {
                                d.child(
                                    ui::text(
                                        w.text.clone(),
                                        size::BODY,
                                        weight::REGULAR,
                                        theme.c("ink"),
                                    )
                                    .line_clamp(3)
                                    .text_ellipsis(),
                                )
                            })
                            .child(
                                div()
                                    .flex()
                                    .gap(px(space::MD))
                                    .mt(px(space::MD))
                                    .child(
                                        hb_btn(
                                            gpui::ElementId::Name(
                                                format!("away-open-{}", w.id).into(),
                                            ),
                                            false,
                                            &theme,
                                        )
                                        .h(px(control::SM))
                                        .on_click(move |_, _, cx| {
                                            navigate(route.clone(), cx);
                                            close(cx);
                                        })
                                        .child(t(cx, "home.open")),
                                    )
                                    .child(
                                        hb_btn(
                                            gpui::ElementId::Name(
                                                format!("away-later-{}", w.id).into(),
                                            ),
                                            false,
                                            &theme,
                                        )
                                        .h(px(control::SM))
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
            tv(
                cx,
                "home.finishedCount",
                &[("count", &model.done.len().to_string())],
            ),
            &theme,
        );
        if model.done.is_empty() {
            done_section = done_section.child(empty(t(cx, "home.nothingFinished"), &theme));
        } else {
            done_section = done_section.children(model.done.iter().map(|d| {
                let text: SharedString = match &d.branch {
                    Some(b) => tv(cx, "home.deliveryVerified", &[("branch", b)]),
                    None => d.text.clone().into(),
                };
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::MD))
                    .min_h(px(row::NAV))
                    .child(
                        ui::text(text, size::UI, weight::REGULAR, theme.c("ink"))
                            .min_w_0()
                            .truncate(),
                    )
                    .when(!d.who.is_empty(), |el| {
                        el.child(
                            ui::text(d.who.clone(), size::META, weight::REGULAR, theme.c("ink-3"))
                                .flex_none(),
                        )
                    })
            }));
        }

        // Still running.
        let mut run_section = section(
            "refresh-cw",
            theme.c("ink-3"),
            tv(
                cx,
                "home.stillRunning",
                &[("count", &model.running.len().to_string())],
            ),
            &theme,
        );
        if model.running.is_empty() {
            run_section = run_section.child(empty(t(cx, "home.noAgentWorking"), &theme));
        }
        for key in model.running.iter() {
            let Some(a) = view.as_ref().and_then(|v| v.agent(key)) else {
                continue;
            };
            run_section = run_section.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::MD))
                    .py(px(space::XS))
                    .child(div().flex_none().child(crate::ui::avatar::agent_avatar(
                        a,
                        avatar_size::SIDEBAR,
                        false,
                        cx,
                    )))
                    .child(
                        ui::text(agent_name(a, cx), size::UI, weight::STRONG, theme.c("ink"))
                            .flex_none(),
                    )
                    .child(
                        ui::text(
                            a.description.clone().unwrap_or_default(),
                            size::SMALL,
                            weight::REGULAR,
                            theme.c("ink-2"),
                        )
                        .flex_1()
                        .min_w_0()
                        .truncate(),
                    )
                    .child(
                        ui::text(
                            presence_line(a, now, cx),
                            size::META,
                            weight::REGULAR,
                            theme.c("ink-3"),
                        )
                        .flex_none(),
                    ),
            );
        }

        let primary = if away.still_away {
            hb_btn("away-available", true, &theme)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.error = None;
                    let task = ProfileStore::get(cx).update(cx, |p, cx| {
                        p.update(json!({ "state": { "id": "available" } }), cx)
                    });
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
                .child(t(cx, "home.setAvailable"))
        } else {
            hb_btn("away-done", true, &theme)
                .on_click(|_, _, cx| close(cx))
                .child(t(cx, "home.done"))
        };

        let card = div()
            .id("away-card")
            .occlude()
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(WIDTH))
            .max_w(window.viewport_size().width - px(space::BREAK))
            .max_h(window.viewport_size().height - px(space::BREAK))
            .flex()
            .flex_col()
            .rounded(px(radius::LG))
            .bg(theme.c("surface-raised"))
            .shadow(theme.pop_shadow())
            .overflow_hidden()
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(space::MD))
                    .h(px(row::HEADER))
                    .pl(px(space::GUTTER))
                    .pr(px(space::LG))
                    .border_b_1()
                    .border_color(theme.c("line"))
                    .child(
                        ui::text(
                            title_of(&away, cx),
                            size::TITLE,
                            weight::HEAVY,
                            theme.c("ink"),
                        )
                        .flex_1()
                        .min_w_0()
                        .truncate(),
                    )
                    .child(close_button("away-close", &theme, cx).on_click(|_, _, cx| close(cx))),
            )
            .child(
                div()
                    .id("away-body")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(space::SECTION))
                    .px(px(space::GUTTER))
                    .py(px(space::XL))
                    .child(wait_section)
                    .child(done_section)
                    .child(run_section)
                    .when_some(self.error.clone(), |d, e| {
                        d.child(
                            div()
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
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .gap(px(space::MD))
                    .px(px(space::GUTTER))
                    .py(px(space::XL))
                    .border_t_1()
                    .border_color(theme.c("line"))
                    .child(
                        hb_btn("away-activity", false, &theme)
                            .on_click(|_, _, cx| {
                                navigate(Route::activity(), cx);
                                close(cx);
                            })
                            .child(t(cx, "home.seeAllInActivity")),
                    )
                    .child(primary),
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
