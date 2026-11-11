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
    self as words, CARD_GAP, CARD_WIDTH, CLOSE_GRACE_MS, Micro, OPEN_DELAY_MS, Rect as CardRect,
    Side, StatusPart, card_model, clock, micro_state, now_icon, now_quiet, now_wash, place_card,
    presence_pill, progress_parts, relative, status_parts, step_icon, step_ink, step_kind,
    step_text, step_wash, test_summary,
};
use crate::derive::live::{LiveState, merge_live, next_delay};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::inbox::LiveDelta;
use crate::model::{Agent, Thresholds};
use crate::nav::{self, Route};
use crate::theme::Theme;
use crate::ui::kit_theme::FocusRing as _;
use crate::ui::{self, MONO, SANS, icon, icon_size, leading, radius, row, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext, Bounds, BoxShadow, Context, Div,
    ElementId, Entity, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    PathBuilder, Pixels, Render, RenderOnce, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, WeakEntity, Window, anchored, canvas, deferred, div, point, px,
};
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

/// The card's own measures, from the console (`direct.css`, `.hb-card*`).
/// The card is a character surface (DESIGN §3.1): here the console's sizes,
/// bands and pills win over the type and spacing scales.
mod look {
    use std::time::Duration;
    pub const RADIUS: f32 = 14.;
    pub const PAD: f32 = 18.;
    pub const PAD_BOTTOM: f32 = 16.;
    pub const GAP: f32 = 12.;
    /// The robot: breathing, with its smile.
    pub const AVATAR: f32 = 104.;
    pub const HEAD_GAP: f32 = 16.;
    pub const NAME: f32 = 21.;
    pub const NAME_LEADING: f32 = 25.;
    pub const PILL_H: f32 = 26.;
    pub const PILL_PX: f32 = 11.;
    pub const PILL_GAP: f32 = 6.;
    pub const PILL_DOT: f32 = 8.;
    /// `.hb-card`: 15/1.4.
    pub const BODY: f32 = 15.;
    pub const BODY_LEADING: f32 = 21.;
    pub const HEADING: f32 = 16.;
    pub const ROW_TEXT: f32 = 14.;
    pub const TIME_TEXT: f32 = 13.;
    pub const SECTION_GAP: f32 = 6.;
    pub const SECTION_TOP: f32 = 10.;
    pub const ROW_RADIUS: f32 = 8.;
    pub const ROW_GAP: f32 = 10.;
    pub const TILE_RADIUS: f32 = 6.;
    pub const NOW_TILE: f32 = 28.;
    pub const NOW_ICON: f32 = 18.;
    pub const STEP_TILE: f32 = 26.;
    pub const STEP_ICON: f32 = 16.;
    pub const STEP_GAP: f32 = 5.;
    pub const BUTTON_H: f32 = 40.;
    pub const BUTTON_ICON: f32 = 18.;
    pub const LIVE_DOT: f32 = 10.;
    /// How far the arrow stands out of the card, and half its base.
    pub const ARROW: f32 = 8.;
    /// Until the card has been measured once it is placed at this height.
    pub const GUESS_HEIGHT: f32 = 520.;
    /// `hb-card-in`: opacity and 2 px of travel.
    pub const FADE: Duration = Duration::from_millis(120);
    pub const RISE: f32 = 2.;
    /// `hb-live-pulse`: a ring of 6 px that fades out, every 1.6 s.
    pub const PULSE: Duration = Duration::from_millis(1600);
    pub const PULSE_RING: f32 = 6.;
}

/// The panel under a screen's header, inset by a margin: the card stays in
/// it, so it never covers the header or the title band.
fn panel_area(viewport: gpui::Size<Pixels>) -> CardRect {
    let margin = space::MD;
    CardRect {
        left: crate::shell::RAIL + margin,
        top: crate::shell::BAND + 1. + row::HEADER + margin,
        right: f32::from(viewport.width) - space::SM - margin,
        bottom: f32::from(viewport.height) - space::SM - margin,
    }
}

/// One trigger's card: open or not, and what places it (the trigger's
/// bounds and the card's height, both from the last frame).
#[derive(Default)]
pub struct CardHost {
    open: bool,
    on_trigger: bool,
    on_card: bool,
    timer: Option<Task<()>>,
    trigger: Option<Bounds<Pixels>>,
    height: Option<f32>,
}

impl CardHost {
    fn hover_trigger(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.on_trigger = on;
        if on {
            self.after(OPEN_DELAY_MS, true, window, cx);
        } else if !self.on_card {
            self.after(CLOSE_GRACE_MS, false, window, cx);
        }
    }

    fn hover_card(&mut self, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.on_card = on;
        if on {
            self.timer = None;
        } else if !self.on_trigger {
            self.after(CLOSE_GRACE_MS, false, window, cx);
        }
    }

    /// Open or close after `ms`, unless something changes first.
    fn after(&mut self, ms: u64, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.open == open {
            self.timer = None;
            return;
        }
        self.timer = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(ms))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.timer = None;
                if open || !(this.on_trigger || this.on_card) {
                    this.set_open(open, cx);
                }
            });
        }));
    }

    /// Close now (an action on the card was taken).
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.timer = None;
        self.on_card = false;
        self.set_open(false, cx);
    }

    fn set_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.open != open {
            self.open = open;
            // Measured again on the next opening: the agent may have moved on.
            self.height = None;
            cx.notify();
        }
    }
}

/// Wrap `trigger` (an agent's row, name or avatar) so pointing at it opens
/// the agent's card. `id` must be unique on screen; `block` lets the trigger
/// fill its row.
pub fn agent_hover(
    id: impl Into<ElementId>,
    agent_key: &str,
    block: bool,
    trigger: impl IntoElement + 'static,
) -> AnyElement {
    AgentHover {
        id: id.into(),
        key: agent_key.to_string(),
        block,
        trigger: trigger.into_any_element(),
    }
    .into_any_element()
}

#[derive(IntoElement)]
struct AgentHover {
    id: ElementId,
    key: String,
    block: bool,
    trigger: AnyElement,
}

impl RenderOnce for AgentHover {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let host = window.use_keyed_state(self.id.clone(), cx, |_, _| CardHost::default());
        let (open, trigger_bounds, height) = {
            let h = host.read(cx);
            (h.open, h.trigger, h.height)
        };
        let on_hover = {
            let host = host.clone();
            move |on: &bool, window: &mut Window, cx: &mut App| {
                host.update(cx, |h, cx| h.hover_trigger(*on, window, cx))
            }
        };
        // The trigger's bounds, noted every frame for when the card opens.
        let measure = {
            let host = host.clone();
            canvas(
                move |bounds, _, cx| host.update(cx, |h, _| h.trigger = Some(bounds)),
                |_, _, _, _| {},
            )
            .absolute()
            .top_0()
            .left_0()
            .size_full()
        };
        let card = match (open, trigger_bounds) {
            (true, Some(bounds)) => Some(card_layer(host, &self.key, bounds, height, window, cx)),
            _ => None,
        };
        // The outer box takes the trigger's place in its row (a row may
        // stretch it); the inner one is as tall as the trigger, so the card
        // and its arrow find the agent, not the row.
        div().when(self.block, |d| d.w_full()).child(
            div()
                .id(self.id)
                .relative()
                .on_hover(on_hover)
                .child(self.trigger)
                .child(measure)
                .children(card),
        )
    }
}

/// The open card, floated beside its trigger with an arrow at it.
fn card_layer(
    host: Entity<CardHost>,
    key: &str,
    trigger: Bounds<Pixels>,
    height: Option<f32>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let theme = cx.global::<Theme>().clone();
    let area = panel_area(window.viewport_size());
    let at = CardRect {
        left: f32::from(trigger.left()),
        top: f32::from(trigger.top()),
        right: f32::from(trigger.right()),
        bottom: f32::from(trigger.bottom()),
    };
    let place = place_card(
        at,
        CARD_WIDTH,
        height.unwrap_or(look::GUESS_HEIGHT),
        area,
        CARD_GAP,
    );
    let card = {
        let key = key.to_string();
        window.use_keyed_state(
            SharedString::from(format!("hb-card-{key}")),
            cx,
            |window, cx| AgentCard::new(key.clone(), window, cx),
        )
    };
    card.update(cx, |c, _| c.host = Some(host.downgrade()));

    // The card's height, measured where it is drawn; the arrow, painted
    // over its edge once the card is in place.
    let fill = theme.c("surface-raised");
    let edge = theme.c("line");
    let side = place.side;
    let along = place.arrow;
    let measured = height.is_some();
    let overlay = {
        let host = host.clone();
        canvas(
            move |bounds, _, cx| {
                let h = f32::from(bounds.size.height);
                host.update(cx, |state, cx| {
                    if state.height.is_none_or(|old| (old - h).abs() > 0.5) {
                        state.height = Some(h);
                        cx.notify();
                    }
                });
            },
            move |bounds, _, window, _| {
                if measured {
                    paint_arrow(bounds, side, along, fill, edge, window);
                }
            },
        )
        .absolute()
        .inset_0()
        .size_full()
    };
    let body = div().relative().child(card).child(overlay);

    // The gap between the trigger and the card belongs to the card, so the
    // pointer can cross it without the card closing.
    let (origin, wrap) = match side {
        Side::Right => (
            point(px(place.left - CARD_GAP), px(place.top)),
            div().pl(px(CARD_GAP)),
        ),
        Side::Left => (point(px(place.left), px(place.top)), div().pr(px(CARD_GAP))),
        Side::Below => (
            point(px(place.left), px(place.top - CARD_GAP)),
            div().pt(px(CARD_GAP)),
        ),
        Side::Above => (point(px(place.left), px(place.top)), div().pb(px(CARD_GAP))),
    };
    let on_hover = {
        let host = host.clone();
        move |on: &bool, window: &mut Window, cx: &mut App| {
            host.update(cx, |h, cx| h.hover_card(*on, window, cx))
        }
    };
    let wrap = wrap
        .id("hb-card-wrap")
        .occlude()
        .on_hover(on_hover)
        .child(body);
    // Placed and measured: it fades in (`hb-card-in`), unless motion is
    // reduced. Before that it is laid out unseen, to learn its height.
    let wrap = if !measured {
        wrap.opacity(0.).into_any_element()
    } else if crate::robot::clock::reduced(cx) {
        wrap.into_any_element()
    } else {
        let rise = match side {
            Side::Above => -look::RISE,
            _ => look::RISE,
        };
        wrap.with_animation(
            "hb-card-in",
            Animation::new(look::FADE).with_easing(gpui::ease_out_quint()),
            move |d, t| d.opacity(t).mt(px(rise * (1. - t))),
        )
        .into_any_element()
    };
    deferred(anchored().position(origin).child(wrap))
        .with_priority(1)
        .into_any_element()
}

/// The little arrow on the card's edge that faces the agent (`::before`):
/// the card's fill, with the hairline on its two slopes.
fn paint_arrow(
    card: Bounds<Pixels>,
    side: Side,
    along: f32,
    fill: Hsla,
    edge: Hsla,
    window: &mut Window,
) {
    let a = px(look::ARROW);
    let (base_a, tip, base_b) = match side {
        Side::Right => {
            let x = card.left();
            let y = card.top() + px(along);
            (point(x, y - a), point(x - a, y), point(x, y + a))
        }
        Side::Left => {
            let x = card.right();
            let y = card.top() + px(along);
            (point(x, y - a), point(x + a, y), point(x, y + a))
        }
        Side::Below => {
            let y = card.top();
            let x = card.left() + px(along);
            (point(x - a, y), point(x, y - a), point(x + a, y))
        }
        Side::Above => {
            let y = card.bottom();
            let x = card.left() + px(along);
            (point(x - a, y), point(x, y + a), point(x + a, y))
        }
    };
    let mut shape = PathBuilder::fill();
    shape.move_to(base_a);
    shape.line_to(tip);
    shape.line_to(base_b);
    shape.close();
    if let Ok(path) = shape.build() {
        window.paint_path(path, fill);
    }
    // Hide the card's ring where the arrow joins it.
    let join = match side {
        Side::Right | Side::Left => Bounds::from_corners(
            point(base_a.x - px(1.), base_a.y + px(1.)),
            point(base_b.x + px(1.), base_b.y - px(1.)),
        ),
        Side::Below | Side::Above => Bounds::from_corners(
            point(base_a.x + px(1.), base_a.y - px(1.)),
            point(base_b.x - px(1.), base_b.y + px(1.)),
        ),
    };
    window.paint_quad(gpui::fill(join, fill));
    let mut line = PathBuilder::stroke(px(1.));
    line.move_to(base_a);
    line.line_to(tip);
    line.line_to(base_b);
    if let Ok(path) = line.build() {
        window.paint_path(path, edge);
    }
}

/// The card's body (`AgentCardView`).
pub struct AgentCard {
    key: String,
    poll: Entity<LivePoll>,
    /// The trigger it opened from, closed when an action is taken.
    host: Option<WeakEntity<CardHost>>,
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
            host: None,
            _tick: second_tick(window, cx),
            _subs: subs,
        }
    }
}

/// Close the card that `host` opened (`onDone`).
fn done(host: &Option<WeakEntity<CardHost>>, cx: &mut App) {
    if let Some(h) = host.as_ref().and_then(WeakEntity::upgrade) {
        h.update(cx, |h, cx| h.close(cx));
    }
}

/// Text in the card's face at a size and weight.
fn card_text(s: impl Into<SharedString>, size: f32, weight: FontWeight, color: Hsla) -> Div {
    div()
        .font_family(SANS)
        .text_size(px(size))
        .line_height(px(leading::for_size(size)))
        .font_weight(weight)
        .text_color(color)
        .child(s.into())
}

/// A section of the card (`.hb-card-sec`): a hairline, its heading, its content.
fn section(title: SharedString, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap(px(look::SECTION_GAP))
        .pt(px(look::SECTION_TOP))
        .border_t_1()
        .border_color(theme.c("line"))
        .child(card_text(
            title,
            look::HEADING,
            weight::HEAVY,
            theme.c("ink"),
        ))
}

/// A pill under the name (`.hb-card-pill`): 26 px, fully round, 14/700.
fn pill(label: impl Into<SharedString>, bg: Hsla, ink: Hsla) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(look::PILL_GAP))
        .h(px(look::PILL_H))
        .px(px(look::PILL_PX))
        .rounded_full()
        .bg(bg)
        .text_color(ink)
        .font_family(SANS)
        .text_size(px(look::ROW_TEXT))
        .line_height(px(leading::UI))
        .font_weight(weight::STRONG)
        .whitespace_nowrap()
        .child(label.into())
}

/// An icon on its tile (`.hb-card-now-ic`, `.hb-step-ic`).
fn tile(name: &str, tile: f32, glyph: f32, bg: Hsla, ink: Hsla) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(tile))
        .rounded(px(look::TILE_RADIUS))
        .bg(bg)
        .child(icon(name, glyph, ink))
}

/// A button of the card's foot (`.hb-card-btn`): 40 px, 15/700, a surface
/// with a strong edge, `hover` under the pointer; the two main ones share
/// the width.
fn card_button(id: &str, hover: Hsla, theme: &Theme) -> gpui::Stateful<Div> {
    div()
        .id(SharedString::from(id.to_string()))
        .flex_1()
        .min_w_0()
        .h(px(look::BUTTON_H))
        .flex()
        .items_center()
        .justify_center()
        .gap(px(space::MD))
        .px(px(space::LG))
        .rounded(px(look::ROW_RADIUS))
        .border_1()
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface"))
        .text_color(theme.c("ink"))
        .text_size(px(look::BODY))
        .line_height(px(look::BODY_LEADING))
        .font_weight(weight::STRONG)
        .whitespace_nowrap()
        .overflow_hidden()
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .focus_ring(theme, false)
}

/// The red dot that says "live", its ring pulsing out (`.hb-live-dot`);
/// still when motion is reduced.
fn live_dot(theme: &Theme, still: bool) -> AnyElement {
    let ring = theme.c("red-line");
    let dot = div()
        .flex_none()
        .size(px(look::LIVE_DOT))
        .rounded_full()
        .bg(theme.c("red"));
    if still {
        return dot.into_any_element();
    }
    dot.with_animation(
        "hb-live-pulse",
        Animation::new(look::PULSE).repeat(),
        move |d, t| {
            // 0 → 70 %: the ring grows to 6 px and fades; then nothing.
            let k = (t / 0.7).min(1.);
            let mut color = ring;
            color.a *= 1. - k;
            d.shadow(vec![BoxShadow {
                color,
                offset: point(px(0.), px(0.)),
                blur_radius: px(0.),
                spread_radius: px(look::PULSE_RING * k),
                inset: false,
            }])
        },
    )
    .into_any_element()
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
        let still = crate::robot::clock::reduced(cx);

        // The head: the robot, the name, the pills, the brief.
        let mut pills: Vec<AnyElement> = model
            .teams
            .iter()
            .map(|team| {
                pill(team.clone(), theme.c("amber-bg"), theme.c("amber-ink")).into_any_element()
            })
            .collect();
        if let Some(role) = role {
            pills.push(pill(role, theme.c("chip"), theme.c("chip-ink")).into_any_element());
        }
        let (wash, ink) = presence_pill(presence_tone);
        let (dot_color, _) = ui::tone_colors(&theme, presence_tone);
        pills.push(
            pill(presence_text, theme.c(wash), theme.c(ink))
                .child(
                    div()
                        .flex_none()
                        .size(px(look::PILL_DOT))
                        .rounded_full()
                        .bg(dot_color),
                )
                // The dot comes first, as in the console.
                .flex_row_reverse()
                .into_any_element(),
        );
        let head = div()
            .flex()
            .items_center()
            .gap(px(look::HEAD_GAP))
            .child(crate::robot::big(Some(&agent), &agent.name, look::AVATAR))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(
                        card_text(
                            model.name.clone(),
                            look::NAME,
                            weight::HEAVY,
                            theme.c("ink"),
                        )
                        .line_height(px(look::NAME_LEADING)),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(look::PILL_GAP))
                            .mt(px(look::PILL_GAP))
                            .children(pills),
                    )
                    .children(model.brief.clone().map(|b| {
                        card_text(b, look::BODY, weight::REGULAR, theme.c("ink-2"))
                            .line_height(px(look::BODY_LEADING))
                            .mt(px(space::MD))
                            .line_clamp(2)
                            .text_ellipsis()
                    })),
            );

        // Now: one tinted row, the state's icon on a dark tile.
        let state = if agent.activity.state.is_empty() {
            "normal".to_string()
        } else {
            agent.activity.state.clone()
        };
        let micro = agent_micro(&agent, now, &thresholds);
        let long = micro.as_ref().is_some_and(|m| m.long);
        let quiet = now_quiet(&state);
        let words: String = status_parts(&agent.activity, agent.reports.len(), micro.as_ref(), cx)
            .into_iter()
            .map(|p| match p {
                StatusPart::Text(s) | StatusPart::Code(s) => s.to_string(),
            })
            .collect();
        let (tile_bg, tile_ink) = if quiet {
            (theme.c("control"), theme.c("ink-3"))
        } else {
            (theme.c("st-term"), theme.c("st-term-ink"))
        };
        let now_line =
            div()
                .flex()
                .items_center()
                .gap(px(look::ROW_GAP))
                .min_w_0()
                .px(px(look::ROW_GAP))
                .py(px(7.))
                .rounded(px(look::ROW_RADIUS))
                .bg(theme.c(now_wash(&state, long)))
                .child(tile(
                    now_icon(&state),
                    look::NOW_TILE,
                    look::NOW_ICON,
                    tile_bg,
                    tile_ink,
                ))
                .child(
                    div()
                        .flex()
                        .min_w_0()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_size(px(look::BODY))
                        .line_height(px(look::BODY_LEADING))
                        .child(
                            div()
                                .flex_none()
                                .font_weight(weight::STRONG)
                                .text_color(theme.c("ink"))
                                .child(words),
                        )
                        .when(
                            state != "disconnected" && agent.activity.since_ms > 0,
                            |d| {
                                d.child(div().flex_none().text_color(theme.c("ink-3")).child(
                                    format!(
                                        "\u{a0}· {}",
                                        clock(now.saturating_sub(agent.activity.since_ms))
                                    ),
                                ))
                            },
                        ),
                );

        // Latest steps: tinted rows, an icon tile per kind of step.
        let steps = progress
            .as_ref()
            .map(|p| p.steps.iter().take(5).cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        let steps_el = if steps.is_empty() {
            card_text(
                t(cx, "agents.noSteps"),
                look::ROW_TEXT,
                weight::REGULAR,
                theme.c("ink-4"),
            )
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(look::STEP_GAP))
                .children(steps.iter().map(|s| {
                    let kind = step_kind(s).to_string();
                    let mut note = String::new();
                    if let Some(test) = &s.test {
                        note.push_str(&format!(" · {}", test_summary(test, cx)));
                    }
                    if s.status == "stopped" {
                        note.push_str(&format!(" · {}", t(cx, "agents.statusStopped")));
                    }
                    let flag = (s.status == "error")
                        .then(|| format!(" · {}", t(cx, "agents.stepHadError")));
                    let time: SharedString = if s.status == "running" {
                        t(cx, "agents.statusRunning")
                    } else {
                        relative(s.ended_ms.unwrap_or(s.started_ms), now, cx)
                    };
                    div()
                        .flex()
                        .items_center()
                        .gap(px(look::ROW_GAP))
                        .min_w_0()
                        .pl(px(space::SM))
                        .pr(px(look::ROW_GAP))
                        .py(px(space::XS))
                        .rounded(px(look::ROW_RADIUS))
                        .bg(theme.c(step_wash(&s.status)))
                        .text_size(px(look::ROW_TEXT))
                        .line_height(px(leading::UI))
                        .child(tile(
                            step_icon(&kind),
                            look::STEP_TILE,
                            look::STEP_ICON,
                            theme.c("surface"),
                            theme.c(step_ink(&kind)),
                        ))
                        .child(
                            div()
                                .flex()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(theme.c("ink"))
                                        .child(step_text(s, cx)),
                                )
                                .when(!note.is_empty(), |d| {
                                    d.child(
                                        div().flex_none().text_color(theme.c("ink-3")).child(note),
                                    )
                                })
                                .children(flag.map(|f| {
                                    div().flex_none().text_color(theme.c("red-ink")).child(f)
                                })),
                        )
                        .child(
                            card_text(time, look::TIME_TEXT, weight::REGULAR, theme.c("ink-3"))
                                .flex_none(),
                        )
                }))
        };

        // Progress: one line of real numbers, on a band.
        let parts = progress_parts(&agent, progress.as_ref(), now, cx);
        let progress_el = (!parts.is_empty()).then(|| {
            // One run of text, the numbers in black.
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
                            font_weight: Some(weight::HEAVY),
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
                .px(px(look::ROW_GAP))
                .py(px(space::MD))
                .rounded(px(look::ROW_RADIUS))
                .bg(theme.c("tint"))
                .text_size(px(look::ROW_TEXT))
                .line_height(px(leading::UI))
                .text_color(theme.c("ink-2"))
                .child(gpui::StyledText::new(text).with_highlights(bold));
            section(t(cx, "agents.progress"), &theme).child(line)
        });

        let key_open = self.key.clone();
        let key_live = self.key.clone();
        let key_hide = self.key.clone();
        let hidden = agent.hidden;
        let (host_open, host_live, host_hide) =
            (self.host.clone(), self.host.clone(), self.host.clone());
        let live_soft = theme.c("accent-soft");
        // The layer around it takes the pointer; blocking it here as well
        // would hide the pointer from that layer, and the card would close.
        div()
            .id("hb-card")
            .w(px(CARD_WIDTH))
            .flex()
            .flex_col()
            .gap(px(look::GAP))
            .pt(px(look::PAD))
            .px(px(look::PAD))
            .pb(px(look::PAD_BOTTOM))
            .rounded(px(look::RADIUS))
            .bg(theme.c("surface-raised"))
            .shadow(theme.pop_shadow())
            .font_family(SANS)
            .text_size(px(look::BODY))
            .line_height(px(look::BODY_LEADING))
            .font_weight(weight::REGULAR)
            .text_color(theme.c("ink"))
            .child(head)
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
                    .gap(px(look::ROW_GAP))
                    .pt(px(space::HAIR))
                    .child(
                        // Words only, as the console drew it: the width goes to them.
                        card_button("hb-card-open", theme.c("btn-hover"), &theme)
                            .child(t(cx, "agents.openConversation"))
                            .on_click(move |_, _, cx| {
                                done(&host_open, cx);
                                nav::navigate(Route::direct(Some(key_open.clone())), cx)
                            }),
                    )
                    .child(
                        card_button("hb-card-live", theme.c("tint"), &theme)
                            .border_color(live_soft)
                            .bg(live_soft)
                            .text_color(theme.c("accent-text"))
                            .child(live_dot(&theme, still))
                            .child(t(cx, "agents.watchLive"))
                            .on_click(move |_, _, cx| {
                                done(&host_live, cx);
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
                        card_button("hb-card-hide", theme.c("btn-hover"), &theme)
                            .flex_none()
                            .text_color(theme.c("ink-2"))
                            .child(icon(
                                if hidden { "eye" } else { "eye-off" },
                                look::BUTTON_ICON,
                                theme.c("ink-2"),
                            ))
                            .child(if hidden {
                                t(cx, "shell.show")
                            } else {
                                t(cx, "shell.hide")
                            })
                            .on_click(move |_, _, cx| {
                                done(&host_hide, cx);
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
