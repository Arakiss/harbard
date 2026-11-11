//! The agent hover card (`agent-card.tsx`): wrap any agent name or avatar
//! with [`agent_hover`] and pointing at it for 300 ms opens a card with the
//! agent's name, pills, what it is doing now, its latest steps and progress,
//! and the ways to open its conversation, watch it live or hide it.
//!
//! Also here: [`status_line`], the one-line "what it is doing" with its
//! timer, and [`LivePoll`], one agent's live read polled every second while
//! the window is visible (the card asks for its progress only; the live
//! view asks for everything).

use crate::derive::agent_card::{
    self as words, CARD_WIDTH, CLOSE_GRACE_MS, Micro, OPEN_DELAY_MS, StatusPart, card_model, clock,
    micro_state, now_icon, progress_parts, relative, status_parts, step_icon, step_kind, step_text,
    test_summary,
};
use crate::derive::live::{LiveState, merge_live, next_delay};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::inbox::LiveDelta;
use crate::model::{Agent, Thresholds};
use crate::nav::{self, Route};
use crate::theme::Theme;
use crate::ui::kit_theme::FocusRing as _;
use crate::ui::{
    self, MONO, SANS, avatar_size, control, icon, icon_size, leading, radius, row, size, space,
    weight,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, Div, ElementId, Entity, Hsla, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, Window, div, px,
};
use gpui_kit::component::hover_card::HoverCard;
use std::time::Duration;

/// `POST /workspace/<action>` through the desk, then a fresh read.
pub fn post(
    action: &str,
    body: serde_json::Value,
    cx: &mut App,
) -> Task<Result<serde_json::Value, crate::api::ApiError>> {
    Desk::get(cx).update(cx, |desk, cx| desk.post(action, body, cx))
}

// ---------------------------------------------------------------------------
// The status line
// ---------------------------------------------------------------------------

/// The timer's box at a status line's right edge.
const TIMER_BOX: f32 = 36.;

/// How a status line is drawn where it appears.
#[derive(Clone, Copy)]
pub struct StatusStyle {
    pub size: f32,
    pub color: Hsla,
    /// The timer at the right edge (`margin-left: auto`), or beside the words.
    pub time_right: bool,
    pub show_time: bool,
}

/// The icon tint per state, from the console's status icons.
/// The status line's icon per state (the console draws its own; these are the nearest Lucide ones).
fn status_icon(state: &str) -> &'static str {
    match state {
        "thinking" => "ellipsis",
        "normal" => "coffee",
        other => now_icon(other),
    }
}

fn state_tone(theme: &Theme, state: &str) -> Hsla {
    match state {
        "running" | "testing" => theme.c("green"),
        "editing" | "delegating" => theme.c("accent"),
        "reading" | "web" => theme.c("focus"),
        "waiting" | "asleep" => theme.c("amber"),
        "blocked" => theme.c("red"),
        "finished" | "verified" => theme.c("green"),
        _ => theme.c("ink-4"),
    }
}

/// The microstate of an agent now, when serve sent thresholds.
pub fn agent_micro(agent: &Agent, now: u64, thresholds: &Thresholds) -> Option<Micro> {
    (thresholds.amber_ms > 0).then(|| micro_state(&agent.activity, now, thresholds))
}

/// Words with code spans, as one line.
fn parts_line(parts: Vec<StatusPart>, size: f32, color: Hsla, theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .min_w_0()
        .overflow_hidden()
        .whitespace_nowrap()
        .text_color(color)
        .children(parts.into_iter().map(|p| {
            match p {
                StatusPart::Text(s) => div()
                    .flex_none()
                    .text_size(px(size))
                    .child(s)
                    .into_any_element(),
                // Mono sits two pixels under the words beside it (T6).
                StatusPart::Code(s) => div()
                    .flex_none()
                    .font_family(MONO)
                    .text_size(px((size - 2.).max(size::META)))
                    .font_weight(weight::REGULAR)
                    .px(px(space::XS))
                    .rounded(px(radius::SM))
                    .bg(theme.c("control"))
                    .text_color(theme.c("ink-2"))
                    .child(s)
                    .into_any_element(),
            }
        }))
}

/// "Running `cargo test` … 2 min" (`AgentStatus` / `StatusLine`).
pub fn status_line(agent: &Agent, style: StatusStyle, cx: &App) -> Div {
    let theme = cx.global::<Theme>();
    let desk = Desk::get(cx).read(cx);
    let now = desk.now_ms();
    let thresholds = desk
        .view
        .as_ref()
        .map(|v| v.thresholds.clone())
        .unwrap_or_default();
    let activity = &agent.activity;
    let state = if activity.state.is_empty() {
        "normal"
    } else {
        activity.state.as_str()
    };
    let micro = agent_micro(agent, now, &thresholds);
    let long = micro.as_ref().is_some_and(|m| m.long);
    let parts = status_parts(activity, agent.reports.len(), micro.as_ref(), cx);
    let words_color = if long {
        theme.c("amber-ink")
    } else {
        style.color
    };
    div()
        .flex()
        .items_center()
        .gap(px(space::SM))
        .min_w_0()
        .font_family(SANS)
        .text_size(px(style.size))
        .line_height(px(leading::for_size(style.size)))
        .child(icon(
            status_icon(state),
            icon_size::INLINE,
            state_tone(theme, state),
        ))
        .child(parts_line(parts, style.size, words_color, theme))
        .when(
            style.show_time && state != "disconnected" && activity.since_ms > 0,
            |d| {
                d.child(
                    // A fixed box at the right edge, so a ticking timer
                    // does not shake the row (T8).
                    div()
                        .flex()
                        .flex_none()
                        .when(style.time_right, |d| {
                            d.ml_auto().min_w(px(TIMER_BOX)).justify_end()
                        })
                        .when(!style.time_right, |d| d.ml(px(space::HAIR)))
                        .text_size(px(size::META))
                        .text_color(style.color)
                        .child(words::elapsed(now.saturating_sub(activity.since_ms))),
                )
            },
        )
}

// ---------------------------------------------------------------------------
// One agent's live read
// ---------------------------------------------------------------------------

/// One agent's live read, asked every second while the window is visible;
/// hidden, it asks nothing and holds no timer. After failures it asks less
/// often (up to every 30 s). Dropping it stops everything.
pub struct LivePoll {
    key: String,
    prose: bool,
    only_progress: bool,
    pub state: Option<LiveState>,
    /// At least one answer arrived.
    pub answered: bool,
    /// The last ask failed.
    pub error: bool,
    /// The window is hidden: nothing is asked.
    pub paused: bool,
    failures: u32,
    task: Option<Task<()>>,
    _visibility: Subscription,
}

impl LivePoll {
    pub fn new(
        key: String,
        prose: bool,
        only_progress: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let visibility = cx.observe_window_visibility(window, |this, _, window, cx| {
            let visible = window.is_visible();
            if visible && this.task.is_none() {
                this.paused = false;
                this.schedule(Duration::ZERO, window, cx);
            } else if !visible {
                this.task = None;
                this.paused = true;
            }
            cx.notify();
        });
        let mut poll = LivePoll {
            key,
            prose,
            only_progress,
            state: None,
            answered: false,
            error: false,
            paused: !window.is_visible(),
            failures: 0,
            task: None,
            _visibility: visibility,
        };
        if !poll.paused {
            poll.schedule(Duration::ZERO, window, cx);
        }
        poll
    }

    fn schedule(&mut self, delay: Duration, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.key.clone();
        let after = self.state.as_ref().map(|s| s.rev.to_string());
        let prose = (!self.prose).then(|| "0".to_string());
        let only = self.only_progress.then(|| "progress".to_string());
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            if !delay.is_zero() {
                cx.background_executor().timer(delay).await;
            }
            let answer = cx
                .background_executor()
                .spawn(async move {
                    harbard_cli::serve::live_route::live_body(&key, after, prose, only)
                })
                .await;
            let _ = this.update_in(cx, |this, window, cx| {
                match answer {
                    (200, body) => match serde_json::from_value::<LiveDelta>(body) {
                        Ok(d) => {
                            this.state = Some(merge_live(this.state.as_ref(), &d));
                            this.failures = 0;
                            this.error = false;
                        }
                        Err(_) => {
                            this.failures = (this.failures + 1).min(8);
                            this.error = true;
                        }
                    },
                    _ => {
                        this.failures = (this.failures + 1).min(8);
                        this.error = true;
                    }
                }
                this.answered = true;
                cx.notify();
                if window.is_visible() {
                    this.schedule(next_delay(this.failures), window, cx);
                } else {
                    this.task = None;
                    this.paused = true;
                }
            });
        }));
    }
}

/// A 1-second tick for timers on screen; drop it to stop.
pub fn second_tick<T: 'static>(window: &mut Window, cx: &mut Context<T>) -> Task<()> {
    cx.spawn_in(window, async move |this, cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let alive = this.update_in(cx, |_, window, cx| {
                if window.is_visible() {
                    cx.notify();
                }
            });
            if alive.is_err() {
                break;
            }
        }
    })
}

// ---------------------------------------------------------------------------
// The hover card
// ---------------------------------------------------------------------------

/// Wrap `trigger` (an agent's row, name or avatar) so pointing at it opens
/// the agent's card. `id` must be unique on screen; `block` lets the trigger
/// fill its row.
pub fn agent_hover(
    id: impl Into<ElementId>,
    agent_key: &str,
    block: bool,
    trigger: impl IntoElement + 'static,
) -> AnyElement {
    let key = agent_key.to_string();
    HoverCard::new(id)
        .appearance(false)
        .anchor(gpui::Anchor::LeftCenter)
        .open_delay(Duration::from_millis(OPEN_DELAY_MS))
        .close_delay(Duration::from_millis(CLOSE_GRACE_MS))
        .when(block, |h| {
            h.trigger_style(gpui::StyleRefinement::default().w_full())
        })
        .trigger(trigger)
        .content(move |_, window, cx| {
            let key = key.clone();
            let card = window.use_keyed_state(
                SharedString::from(format!("hb-card-{key}")),
                cx,
                |window, cx| AgentCard::new(key.clone(), window, cx),
            );
            // The gap between the trigger and the card stays part of the card.
            div().pl(px(space::LG)).child(card)
        })
        .into_any_element()
}

/// The card's body (`AgentCardView`).
pub struct AgentCard {
    key: String,
    poll: Entity<LivePoll>,
    _tick: Task<()>,
    _subs: Vec<Subscription>,
}

impl AgentCard {
    pub fn new(key: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let poll = cx.new(|cx| LivePoll::new(key.clone(), false, true, window, cx));
        let subs = vec![
            cx.observe(&poll, |_, _, cx| cx.notify()),
            cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()),
        ];
        AgentCard {
            key,
            poll,
            _tick: second_tick(window, cx),
            _subs: subs,
        }
    }
}

/// A section of the card: a hairline, its label, its content.
fn section(title: SharedString, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(space::SM))
        .pt(px(space::LG))
        .border_t_1()
        .border_color(theme.c("line"))
        .child(ui::section_label(title, theme))
}

/// A button of the card's foot: 32 px, 14/700, a surface with a strong edge.
fn card_button(id: &str, theme: &Theme) -> gpui::Stateful<Div> {
    let hover = theme.c("btn-hover");
    div()
        .id(SharedString::from(id.to_string()))
        .min_w_0()
        .h(px(control::MD))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(space::SM))
        .px(px(space::LG))
        .rounded(px(radius::BASE))
        .border_1()
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface"))
        .text_color(theme.c("ink"))
        .text_size(px(size::UI))
        .line_height(px(leading::UI))
        .font_weight(weight::STRONG)
        .whitespace_nowrap()
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .focus_ring(theme, false)
}

impl Render for AgentCard {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let desk = Desk::get(cx).read(cx);
        let now = desk.now_ms();
        let Some(view) = desk.view.clone() else {
            return div().into_any_element();
        };
        let Some(agent) = view.agent(&self.key).cloned() else {
            return div().into_any_element();
        };
        let thresholds = view.thresholds.clone();
        let poll = self.poll.read(cx);
        let progress = poll
            .state
            .as_ref()
            .map(|s| s.progress.clone())
            .or_else(|| agent.progress.clone());
        let model = card_model(&agent, cx);
        let (presence_text, presence_tone) = model.presence.clone();
        // A bare session's label is "session:" with nothing after it: no role to show.
        let role = model.role.clone().filter(|r| !r.trim_end().ends_with(':'));

        // Now
        let state = if agent.activity.state.is_empty() {
            "normal".to_string()
        } else {
            agent.activity.state.clone()
        };
        let micro = agent_micro(&agent, now, &thresholds);
        let long = micro.as_ref().is_some_and(|m| m.long);
        let quiet = matches!(state.as_str(), "normal" | "finished" | "disconnected");
        // A wash only when the state is one: blocked, or waiting on someone.
        let now_bg = match state.as_str() {
            "blocked" => Some("red-bg"),
            "waiting" | "asleep" => Some("warn-bg"),
            _ if long => Some("warn-bg"),
            _ => None,
        };
        let parts = status_parts(&agent.activity, agent.reports.len(), micro.as_ref(), cx);
        let now_line = div()
            .flex()
            .items_center()
            .gap(px(space::MD))
            .min_w_0()
            .min_h(px(row::NAV))
            .when_some(now_bg, |d, bg| {
                d.px(px(space::MD))
                    .rounded(px(radius::BASE))
                    .bg(theme.c(bg))
            })
            .child(icon(
                now_icon(&state),
                icon_size::ROW,
                if quiet {
                    theme.c("ink-3")
                } else {
                    state_tone(&theme, &state)
                },
            ))
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .font_weight(weight::STRONG)
                    .child(parts_line(parts, size::UI, theme.c("ink"), &theme)),
            )
            .when(
                state != "disconnected" && agent.activity.since_ms > 0,
                |d| {
                    d.child(
                        ui::text(
                            clock(now.saturating_sub(agent.activity.since_ms)),
                            size::META,
                            weight::REGULAR,
                            theme.c("ink-3"),
                        )
                        .flex_none(),
                    )
                },
            );

        // Latest steps: one line each, the mark quiet unless the step failed
        // or is still running.
        let steps = progress
            .as_ref()
            .map(|p| p.steps.iter().take(5).cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let steps_el = if steps.is_empty() {
            ui::text(
                t(cx, "agents.noSteps"),
                size::SMALL,
                weight::REGULAR,
                theme.c("ink-3"),
            )
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(space::XS))
                .children(steps.iter().map(|s| {
                    let kind = step_kind(s).to_string();
                    let tint = match s.status.as_str() {
                        "running" => theme.c("green"),
                        "error" => theme.c("red"),
                        _ => theme.c("ink-3"),
                    };
                    let mut text = step_text(s, cx).to_string();
                    if let Some(test) = &s.test {
                        text.push_str(&format!(" · {}", test_summary(test, cx)));
                    }
                    let time: SharedString = if s.status == "running" {
                        t(cx, "agents.statusRunning")
                    } else {
                        relative(s.ended_ms.unwrap_or(s.started_ms), now, cx)
                    };
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::MD))
                        .min_w_0()
                        .child(icon(step_icon(&kind), icon_size::INLINE, tint))
                        .child(
                            ui::text(text, size::SMALL, weight::REGULAR, theme.c("ink-2"))
                                .min_w_0()
                                .truncate(),
                        )
                        .when(s.status == "error", |d| {
                            d.child(
                                ui::text(
                                    t(cx, "agents.stepHadError"),
                                    size::SMALL,
                                    weight::REGULAR,
                                    theme.c("red-ink"),
                                )
                                .flex_none(),
                            )
                        })
                        .when(s.status == "stopped", |d| {
                            d.child(
                                ui::text(
                                    t(cx, "agents.statusStopped"),
                                    size::SMALL,
                                    weight::REGULAR,
                                    theme.c("ink-3"),
                                )
                                .flex_none(),
                            )
                        })
                        .child(
                            ui::text(time, size::META, weight::REGULAR, theme.c("ink-3"))
                                .ml_auto()
                                .flex_none(),
                        )
                }))
        };

        // Progress
        let parts = progress_parts(&agent, progress.as_ref(), now, cx);
        let progress_el = (!parts.is_empty()).then(|| {
            // One run of text, the numbers in bold.
            let mut text = String::new();
            let mut bold: Vec<(std::ops::Range<usize>, gpui::HighlightStyle)> = Vec::new();
            for (i, p) in parts.iter().enumerate() {
                if i > 0 {
                    text.push_str(" · ");
                }
                if let Some(lead) = &p.lead {
                    text.push_str(lead);
                    text.push(' ');
                }
                let strong = if p.tone == Some("fail") {
                    theme.c("red-ink")
                } else {
                    theme.c("ink")
                };
                for (j, (n, w)) in p.values.iter().enumerate() {
                    if j > 0 {
                        text.push_str(", ");
                    }
                    let at = text.len();
                    text.push_str(n);
                    bold.push((
                        at..text.len(),
                        gpui::HighlightStyle {
                            font_weight: Some(weight::STRONG),
                            color: Some(strong),
                            ..Default::default()
                        },
                    ));
                    if !w.is_empty() {
                        text.push(' ');
                        text.push_str(w);
                    }
                }
            }
            let line = div()
                .text_size(px(size::SMALL))
                .line_height(px(leading::SMALL))
                .text_color(theme.c("ink-2"))
                .child(gpui::StyledText::new(text).with_highlights(bold));
            section(t(cx, "agents.progress"), &theme).child(line)
        });

        let key_open = self.key.clone();
        let key_live = self.key.clone();
        let key_hide = self.key.clone();
        let hidden = agent.hidden;
        div()
            .id("hb-card")
            .occlude()
            .w(px(CARD_WIDTH))
            .flex()
            .flex_col()
            .gap(px(space::LG))
            .p(px(space::XL))
            .rounded(px(radius::MD))
            .bg(theme.c("surface-raised"))
            .shadow(theme.pop_shadow())
            .font_family(SANS)
            .text_size(px(size::UI))
            .line_height(px(leading::UI))
            .font_weight(weight::REGULAR)
            .text_color(theme.c("ink"))
            .child(
                div()
                    .flex()
                    .gap(px(space::LG))
                    .child(div().flex_none().child(ui::avatar::agent_avatar(
                        &agent,
                        avatar_size::ROW,
                        false,
                        cx,
                    )))
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(px(space::MD))
                                    .child(
                                        ui::text(
                                            model.name.clone(),
                                            size::BODY,
                                            weight::HEAVY,
                                            theme.c("ink"),
                                        )
                                        .min_w_0()
                                        .truncate(),
                                    )
                                    .child(ui::chip(presence_text, presence_tone, &theme)),
                            )
                            .child(
                                ui::text(
                                    model
                                        .teams
                                        .iter()
                                        .map(|team| format!("#{team}"))
                                        .chain(role)
                                        .collect::<Vec<_>>()
                                        .join(" · "),
                                    size::SMALL,
                                    weight::REGULAR,
                                    theme.c("ink-3"),
                                )
                                .truncate(),
                            )
                            .children(model.brief.clone().map(|b| {
                                ui::text(b, size::SMALL, weight::REGULAR, theme.c("ink-2"))
                                    .mt(px(space::XS))
                                    .line_clamp(2)
                            })),
                    ),
            )
            .child(
                section(t(cx, "agents.now"), &theme)
                    .child(now_line)
                    .children(
                        (!agent.activity.state.is_empty())
                            .then(|| {
                                let sample = serde_json::json!({
                                    "state": agent.activity.state,
                                    "label": agent.activity.detail,
                                });
                                crate::ui::feedback::feedback_row(
                                    "robot_state",
                                    &agent.key,
                                    Some(sample),
                                    cx,
                                )
                            })
                            .flatten(),
                    ),
            )
            .child(section(t(cx, "agents.lastSteps"), &theme).child(steps_el))
            .children(progress_el)
            .child(
                div()
                    .flex()
                    .gap(px(space::MD))
                    .pt(px(space::XS))
                    .child(
                        card_button("hb-card-open", &theme)
                            .child(icon("message-circle", icon_size::ROW, theme.c("ink-2")))
                            .child(t(cx, "agents.openConversation"))
                            .on_click(move |_, _, cx| {
                                nav::navigate(Route::direct(Some(key_open.clone())), cx)
                            }),
                    )
                    .child(
                        card_button("hb-card-live", &theme)
                            .child(icon("radio", icon_size::ROW, theme.c("ink-2")))
                            .child(t(cx, "agents.watchLive"))
                            .on_click(move |_, _, cx| {
                                nav::navigate(
                                    Route::Direct {
                                        agent: Some(key_live.clone()),
                                        profile: false,
                                        live: true,
                                    },
                                    cx,
                                )
                            }),
                    )
                    .child(
                        card_button("hb-card-hide", &theme)
                            .ml_auto()
                            .flex_none()
                            .child(icon(
                                if hidden { "eye" } else { "eye-off" },
                                icon_size::ROW,
                                theme.c("ink-2"),
                            ))
                            .child(if hidden {
                                t(cx, "shell.show")
                            } else {
                                t(cx, "shell.hide")
                            })
                            .on_click(move |_, _, cx| {
                                let action = if hidden { "show-agent" } else { "hide-agent" };
                                post(action, serde_json::json!({ "agent": key_hide }), cx).detach();
                            }),
                    ),
            )
            .into_any_element()
    }
}

/// "Card for {name}" (screen readers).
pub fn card_label(agent: &Agent, cx: &App) -> SharedString {
    tv(
        cx,
        "agents.cardAria",
        &[("name", &crate::derive::agent_name(agent, cx))],
    )
}
