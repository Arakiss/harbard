//! The status line: the console's `StatusLine` / `AgentStatus` — a 14×14
//! icon per state, the words from `statusParts`, and a timer that ticks.
//!
//! The icon is a small nested view on the robot clock (its loops redraw only
//! it). The timer asks the view around it to draw again only when the words
//! change: every second for the first minute of a state, then every minute.

use super::activity::{
    Micro, StatusPart, elapsed, elapsed_next_change, micro_next_change, micro_state, status_parts,
};
use super::clock;
use super::draw::{Affine, EASE_IN_OUT, LINEAR, Pen, phase, track};
use crate::desk::Desk;
use crate::model::{Activity, Agent, Thresholds};
use crate::perf::Source;
use crate::theme::Theme;
use crate::ui::{radius, size, space};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Bounds, Context, ElementId, Hsla, IntoElement, ParentElement, Pixels, Render,
    RenderOnce, SharedString, StyleRefinement, Styled, Window, canvas, div, px,
};
use std::time::Instant;

/// An agent's status line. Build with [`status_line`].
#[derive(IntoElement)]
pub struct StatusLine {
    key: SharedString,
    activity: Option<Activity>,
    reports: usize,
    live: bool,
    show_time: bool,
    show_words: bool,
    color: Option<Hsla>,
    long_color: Option<Hsla>,
    text_size: f32,
    time_gap: Option<f32>,
}

/// The status line of `agent` (`AgentStatus`): its live activity, or the
/// read's when the faces slice has not got it.
pub fn status_line(agent: &Agent) -> StatusLine {
    StatusLine {
        key: agent.key.clone().into(),
        activity: Some(agent.activity.clone()).filter(|a| !a.state.is_empty()),
        reports: agent.reports.len(),
        live: true,
        show_time: true,
        show_words: true,
        color: None,
        long_color: None,
        text_size: size::SMALL,
        time_gap: None,
    }
}

impl StatusLine {
    /// Show or hide the timer (`showTime`).
    pub fn time(mut self, show: bool) -> Self {
        self.show_time = show;
        self
    }

    /// Only the timer (an idle agent row: `[data-idle] .hb-status`).
    pub fn time_only(mut self) -> Self {
        self.show_words = false;
        self
    }

    /// The text colour (default `ink-3`); the sidebar uses `sidebar-ink-2`.
    pub fn color(mut self, color: Hsla) -> Self {
        self.color = Some(color);
        self
    }

    /// The words' colour while a microstate asks for attention (default
    /// `amber-ink`; the sidebar keeps its own ink).
    pub fn long_color(mut self, color: Hsla) -> Self {
        self.long_color = Some(color);
        self
    }

    pub fn text_size(mut self, size: f32) -> Self {
        self.text_size = size;
        self
    }

    /// The timer sits this far after the words instead of at the far end.
    pub fn time_gap(mut self, gap: f32) -> Self {
        self.time_gap = Some(gap);
        self
    }
}

/// `ms` (wall clock) rounded down to its whole second.
fn on_second(ms: u64) -> u64 {
    ms - ms % 1000
}

/// When a timer read at grid second `now` next shows a different value.
fn timer_change(now: u64, since: u64) -> u64 {
    now + elapsed_next_change(now.saturating_sub(since))
}

/// A change at `at` is shown from the first grid second at or after it.
fn shown_at(at: u64) -> u64 {
    on_second(at + 999)
}

fn thresholds(cx: &App) -> Option<Thresholds> {
    let desk = Desk::get(cx).read(cx);
    desk.activity
        .as_ref()
        .map(|a| a.thresholds.clone())
        .or_else(|| desk.view.as_ref().map(|v| v.thresholds.clone()))
        .filter(|t| *t != Thresholds::default())
}

impl RenderOnce for StatusLine {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let desk = Desk::get(cx).read(cx);
        let wall = desk.now_ms();
        // Every line reads the time on one whole-second grid, so all the
        // timers on screen change in one frame instead of one frame each.
        let now = on_second(wall);
        let activity = if self.live {
            desk.activity
                .as_ref()
                .and_then(|v| v.agents.iter().find(|a| a.key == self.key.as_ref()))
                .map(|a| a.activity.clone())
                .filter(|a| !a.state.is_empty())
                .or(self.activity)
        } else {
            self.activity
        };
        let Some(activity) = activity else {
            return div().into_any_element();
        };
        let th = thresholds(cx);
        let micro: Option<Micro> = th
            .as_ref()
            .map(|t| micro_state(&activity.state, activity.since_ms, now, t));
        let theme = cx.global::<Theme>().clone();
        let color = self.color.unwrap_or_else(|| theme.c("ink-3"));
        let long = micro.is_some_and(|m| m.long);
        let words_color = if long {
            self.long_color.unwrap_or_else(|| theme.c("amber-ink"))
        } else {
            color
        };

        // Draw again when the words or the timer next change.
        let mut next: Option<u64> = th
            .as_ref()
            .and_then(|t| micro_next_change(&activity.state, activity.since_ms, now, t));
        let timer = self.show_time && activity.state != "disconnected";
        if timer {
            let at = timer_change(now, activity.since_ms);
            next = Some(next.map_or(at, |n| n.min(at)));
        }
        if let Some(at) = next {
            let at = shown_at(at);
            let t = clock::clock_ms(Instant::now()) + at.saturating_sub(wall) as f64;
            // On the loop grid, so every line on screen changes in one frame,
            // the one the loops draw anyway.
            let t = (t / clock::LOOP_MS).ceil() * clock::LOOP_MS;
            clock::wake_at(
                window.current_view(),
                clock::instant_at(t),
                Source::Timers,
                window,
                cx,
            );
        }

        let mut row = div()
            .flex()
            .items_center()
            .gap(px(space::SM))
            .min_w_0()
            .font_family(crate::ui::SANS)
            .text_size(px(self.text_size))
            .line_height(px(crate::ui::leading::for_size(self.text_size)))
            .whitespace_nowrap()
            .text_color(color);
        if !self.show_words {
            row = row.flex_none();
        }
        if self.show_words {
            if let Some(icon) = status_icon(&self.key, &activity, window, cx) {
                // Never squeezed: long words truncate instead of running under it.
                row = row.child(div().flex_none().child(icon));
            }
            let tr = |key: &str, vars: &[(&str, &str)]| crate::i18n::tv(cx, key, vars).to_string();
            let parts = status_parts(
                &activity,
                self.reports,
                micro.as_ref(),
                &tr,
                &crate::ui::format::hhmm,
            );
            let words = div()
                .flex()
                .items_center()
                .min_w_0()
                .overflow_hidden()
                .text_color(words_color)
                .children(parts.into_iter().map(|p| {
                    match p {
                        StatusPart::Text(t) => {
                            div().min_w_0().truncate().child(t).into_any_element()
                        }
                        StatusPart::Code(c) => div()
                            .min_w_0()
                            .truncate()
                            .px(px(space::XS))
                            .rounded(px(radius::SM))
                            .bg(theme.c("control"))
                            .border_1()
                            .border_color(theme.c("line"))
                            .text_color(theme.c("ink-2"))
                            .font_family(crate::ui::MONO)
                            .text_size(px(size::META))
                            .line_height(px(crate::ui::leading::META))
                            .child(c)
                            .into_any_element(),
                    }
                }));
            row = row.child(words);
        }
        if timer {
            let time = div()
                .flex_none()
                .text_size(px(size::META))
                .when_some(self.time_gap.filter(|_| self.show_words), |d, gap| {
                    d.ml(px(gap))
                })
                .when(self.time_gap.is_none() && self.show_words, |d| d.ml_auto())
                .child(elapsed(now.saturating_sub(activity.since_ms)));
            row = row.child(time);
        }
        row.into_any_element()
    }
}

// ---------------------------------------------------------------------------
// The icons
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub struct IconProps {
    state: String,
    size: f32,
    since_ms: u64,
    reduce: bool,
    theme: (String, bool),
}

pub struct StatusIconView {
    props: IconProps,
}

impl clock::Nested for StatusIconView {
    type Props = IconProps;
    fn from_props(props: IconProps) -> Self {
        StatusIconView { props }
    }
    fn props_mut(&mut self) -> &mut IconProps {
        &mut self.props
    }
}

impl Render for StatusIconView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        draw_icon(&self.props, window, cx)
    }
}

/// The 14×14 icon for an activity state (`StatusIcon`); none at rest.
pub fn status_icon(
    key: &str,
    activity: &Activity,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    status_icon_sized(key, activity, crate::ui::icon_size::INLINE, window, cx)
}

/// Whether a state has an icon (every state but rest, "no task right now").
pub fn has_icon(state: &str) -> bool {
    ICON_STATES.contains(&state)
}

/// The animated, coloured icon for an activity state at `size` points (the
/// 14-unit drawing scaled): the Strip's chips, a hover card's state pill.
/// `key` tells two agents' icons apart. None at rest.
pub fn status_icon_sized(
    key: &str,
    activity: &Activity,
    size: f32,
    window: &mut Window,
    cx: &mut App,
) -> Option<AnyElement> {
    if !has_icon(&activity.state) {
        return None;
    }
    let reduce = clock::still(window, cx);
    let theme = cx.global::<Theme>();
    let props = IconProps {
        state: activity.state.clone(),
        size,
        since_ms: activity.since_ms,
        reduce,
        theme: (theme.revision.clone(), theme.dark),
    };
    let id = ElementId::Name(format!("status-icon:{key}:{size}").into());
    let style = StyleRefinement::default().size(px(size)).flex_none();
    Some(
        match clock::nested::<StatusIconView>(id, style, props, window, cx) {
            Ok(el) => el,
            Err(props) => draw_icon(&props, window, cx),
        },
    )
}

const ICON_STATES: [&str; 13] = [
    "thinking",
    "running",
    "testing",
    "editing",
    "reading",
    "web",
    "delegating",
    "waiting",
    "asleep",
    "blocked",
    "finished",
    "verified",
    "disconnected",
];

/// What a looping icon shows at clock time `t`, as far as it can change;
/// `None` when it changes on every tick.
fn icon_look(state: &str, t: f64) -> Option<i32> {
    match state {
        // The prompt's cursor: on for half a second, then off.
        "running" => Some(i32::from(phase(t, 1000.0, 0.0) < 0.5)),
        // The hourglass holds, then turns over in the last fifth.
        "waiting" => Some(hourglass_turn(phase(t, 3000.0, 0.0)).round() as i32),
        _ => None,
    }
}

fn hourglass_turn(p: f32) -> f32 {
    track(
        p,
        &[(0.0, 0.0), (0.8, 0.0), (0.95, 180.0), (1.0, 180.0)],
        EASE_IN_OUT,
    )
}

fn icon_loops(state: &str) -> bool {
    matches!(
        state,
        "thinking"
            | "running"
            | "testing"
            | "editing"
            | "reading"
            | "web"
            | "delegating"
            | "waiting"
            | "asleep"
    )
}

struct IconColors {
    ink2: Hsla,
    ink4: Hsla,
    term: Hsla,
    term_ink: Hsla,
    green: Hsla,
    line: Hsla,
    line_strong: Hsla,
    accent: Hsla,
    lens: Hsla,
    web: Hsla,
    orbit: Hsla,
    amber: Hsla,
    red: Hsla,
    grey: Hsla,
}

fn draw_icon(p: &IconProps, window: &mut Window, cx: &mut App) -> AnyElement {
    let theme = cx.global::<Theme>();
    let c = IconColors {
        ink2: theme.c("ink-2"),
        ink4: theme.c("ink-4"),
        term: theme.c("st-term"),
        term_ink: theme.c("st-term-ink"),
        green: theme.c("green"),
        line: theme.c("line"),
        line_strong: theme.c("line-strong"),
        accent: theme.c("accent"),
        lens: theme.c("st-lens"),
        web: theme.c("st-web"),
        orbit: theme.c("st-orbit"),
        amber: theme.c("amber"),
        red: theme.c("red"),
        grey: theme.c("grey"),
    };
    let t_now = clock::clock_ms(Instant::now());
    let mut t = None;
    let mut shake = 0.0;
    if !p.reduce {
        if icon_loops(&p.state) {
            let loop_t = clock::on_grid(t_now, clock::LOOP_MS);
            t = Some(loop_t);
            let state = p.state.as_str();
            let wake = if icon_look(state, loop_t).is_some() {
                clock::next_change(loop_t, clock::LOOP_MS, 3000.0, |t| icon_look(state, t))
            } else {
                loop_t + clock::LOOP_MS
            };
            clock::wake_at(
                window.current_view(),
                clock::instant_at(wake),
                Source::Icons,
                window,
                cx,
            );
        }
        // ic-stop: the blocked sign shakes once as it arrives.
        if p.state == "blocked" {
            let age = Desk::get(cx).read(cx).now_ms().saturating_sub(p.since_ms);
            if age < 500 {
                shake = track(
                    age as f32 / 500.0,
                    &[(0.0, 0.0), (0.25, -1.5), (0.75, 1.5), (1.0, 0.0)],
                    EASE_IN_OUT,
                );
                crate::perf::count(Source::Reactions, 1);
                window.request_animation_frame();
            }
        }
    }
    let state = p.state.clone();
    let size = p.size;
    canvas(
        |_, _, _| (),
        move |bounds, _, window, _| paint_icon(&state, size, t, shake, &c, bounds, window),
    )
    .size(px(size))
    .flex_none()
    .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn ring(
    pen: &Pen,
    window: &mut Window,
    cx: f32,
    cy: f32,
    rx: f32,
    ry: f32,
    width: f32,
    color: Hsla,
) {
    let pts: Vec<(f32, f32)> = (0..=32)
        .map(|i| {
            let a = (i as f32 / 32.0) * std::f32::consts::TAU;
            (cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect();
    pen.stroke(window, &pts, width, color, false);
}

fn cubic(
    p0: (f32, f32),
    c1: (f32, f32),
    c2: (f32, f32),
    p1: (f32, f32),
    n: usize,
) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|i| {
            let s = i as f32 / n as f32;
            let u = 1.0 - s;
            let f = |a: f32, b: f32, c: f32, d: f32| {
                u * u * u * a + 3.0 * u * u * s * b + 3.0 * u * s * s * c + s * s * s * d
            };
            (f(p0.0, c1.0, c2.0, p1.0), f(p0.1, c1.1, c2.1, p1.1))
        })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn paint_icon(
    state: &str,
    size: f32,
    t: Option<f64>,
    shake: f32,
    c: &IconColors,
    bounds: Bounds<Pixels>,
    window: &mut Window,
) {
    clock::note_painted();
    // The drawings are in a 14-unit box.
    let k = size / 14.0;
    let base = Affine::translate(f32::from(bounds.origin.x), f32::from(bounds.origin.y))
        .then(Affine::scale(k, k));
    let pen = Pen { m: base };
    let at = |period: f64, delay: f64| t.map(|t| phase(t, period, delay));
    match state {
        "thinking" => {
            for (i, x) in [3.0, 7.0, 11.0].into_iter().enumerate() {
                let op = at(1300.0, 200.0 * i as f64).map_or(1.0, |p| {
                    track(p, &[(0.0, 0.3), (0.5, 1.0), (1.0, 0.3)], EASE_IN_OUT)
                });
                pen.circle(window, x, 7.0, 1.5, c.ink4.opacity(op));
            }
        }
        "running" => {
            pen.round_rect(window, 0.5, 1.5, 13.0, 11.0, 2.5, c.term, None);
            pen.stroke(
                window,
                &[(3.5, 5.0), (5.5, 7.0), (3.5, 9.0)],
                1.4,
                c.green,
                true,
            );
            if at(1000.0, 0.0).is_none_or(|p| p < 0.5) {
                pen.round_rect(window, 7.0, 8.2, 3.5, 1.3, 0.0, c.term_ink, None);
            }
        }
        "testing" => {
            pen.round_rect(window, 0.5, 5.0, 13.0, 4.0, 2.0, c.line, None);
            let dx = at(1400.0, 0.0).map_or(0.0, |p| {
                track(p, &[(0.0, -6.0), (0.5, 6.0), (1.0, -6.0)], EASE_IN_OUT)
            });
            Pen {
                m: base.then(Affine::translate(dx, 0.0)),
            }
            .round_rect(window, 4.0, 5.0, 6.0, 4.0, 2.0, c.green, None);
        }
        "editing" => {
            // ic-ink: the line writes itself in (dash offset 10 → 0 by 60%).
            let shown = at(1600.0, 0.0).map_or(9.0, |p| {
                10.0 - track(p, &[(0.0, 10.0), (0.6, 0.0), (1.0, 0.0)], EASE_IN_OUT)
            });
            if shown > 0.2 {
                pen.stroke(
                    window,
                    &[(1.0, 12.5), (1.0 + shown.min(9.0), 12.5)],
                    1.4,
                    c.accent,
                    true,
                );
            }
            let (dx, deg) = match at(1600.0, 0.0) {
                Some(p) => (
                    track(p, &[(0.0, -2.0), (0.5, 2.0), (1.0, -2.0)], EASE_IN_OUT),
                    track(p, &[(0.0, 0.0), (0.5, -6.0), (1.0, 0.0)], EASE_IN_OUT),
                ),
                None => (0.0, 0.0),
            };
            let m = base
                .then(Affine::translate(dx, 0.0))
                .then(Affine::rotate_at(deg, 6.9, 6.1));
            Pen { m }.fill(
                window,
                &[(8.8, 1.8), (11.2, 4.2), (5.6, 9.8), (2.6, 10.4), (3.2, 7.4)],
                c.ink2,
            );
        }
        "reading" => {
            for (x1, y) in [(13.0, 3.0), (10.0, 6.0), (12.0, 9.0)] {
                pen.stroke(window, &[(1.0, y), (x1, y)], 1.3, c.line_strong, true);
            }
            let dx = at(1800.0, 0.0).map_or(0.0, |p| {
                track(p, &[(0.0, -2.0), (0.5, 2.0), (1.0, -2.0)], EASE_IN_OUT)
            });
            let lens = Pen {
                m: base.then(Affine::translate(dx, 0.0)),
            };
            lens.circle(window, 7.0, 7.0, 3.2, c.lens);
            ring(&lens, window, 7.0, 7.0, 3.2, 3.2, 1.4, c.ink2);
            lens.stroke(window, &[(9.3, 9.3), (11.9, 11.9)], 1.6, c.ink2, true);
        }
        "web" => {
            ring(&pen, window, 7.0, 7.0, 5.8, 5.8, 1.3, c.web);
            let sx = at(2200.0, 0.0).map_or(1.0, |p| {
                track(p, &[(0.0, 1.0), (0.5, 0.15), (1.0, 1.0)], LINEAR)
            });
            ring(&pen, window, 7.0, 7.0, 2.6 * sx, 5.8, 1.2, c.web);
            pen.stroke(window, &[(1.5, 7.0), (12.5, 7.0)], 1.1, c.web, false);
        }
        "delegating" => {
            pen.circle(window, 7.0, 7.0, 2.3, c.accent);
            let turn = at(2400.0, 0.0).map_or(0.0, |p| 360.0 * p);
            let orb = Pen {
                m: base.then(Affine::rotate_at(turn, 7.0, 7.0)),
            };
            orb.circle(window, 7.0, 1.6, 1.5, c.orbit);
            orb.circle(window, 7.0, 12.4, 1.5, c.orbit);
        }
        "waiting" => {
            let deg = at(3000.0, 0.0).map_or(0.0, hourglass_turn);
            let glass = Pen {
                m: base.then(Affine::rotate_at(deg, 7.0, 7.0)),
            };
            glass.stroke(window, &[(3.5, 1.5), (10.5, 1.5)], 1.3, c.amber, true);
            glass.stroke(window, &[(3.5, 12.5), (10.5, 12.5)], 1.3, c.amber, true);
            let mut left = cubic((4.5, 1.5), (4.5, 4.7), (9.5, 5.3), (9.5, 7.0), 8);
            left.extend(
                cubic((9.5, 7.0), (9.5, 8.7), (4.5, 9.3), (4.5, 12.5), 8)
                    .into_iter()
                    .skip(1),
            );
            let mut right = cubic((9.5, 1.5), (9.5, 4.7), (4.5, 5.3), (4.5, 7.0), 8);
            right.extend(
                cubic((4.5, 7.0), (4.5, 8.7), (9.5, 9.3), (9.5, 12.5), 8)
                    .into_iter()
                    .skip(1),
            );
            glass.stroke(window, &left, 1.3, c.amber, true);
            glass.stroke(window, &right, 1.3, c.amber, true);
            glass.fill(window, &[(5.6, 11.6), (8.4, 11.6), (7.0, 9.6)], c.amber);
        }
        "asleep" => {
            for (i, (x, y)) in [(2.0, 12.0), (7.0, 8.0)].into_iter().enumerate() {
                let (op, dx, dy) = match at(2600.0, 1300.0 * i as f64) {
                    Some(p) => (
                        track(p, &[(0.0, 0.0), (0.3, 1.0), (1.0, 0.0)], EASE_IN_OUT),
                        3.0 * EASE_IN_OUT.at(p),
                        -6.0 * EASE_IN_OUT.at(p),
                    ),
                    None => (1.0, 0.0, 0.0),
                };
                super::face::letter_z(&pen, window, x + dx, y + dy, 8.0, c.amber.opacity(op));
            }
        }
        "blocked" => {
            let sign = Pen {
                m: base.then(Affine::translate(shake, 0.0)),
            };
            sign.fill(
                window,
                &[
                    (4.6, 1.0),
                    (9.4, 1.0),
                    (13.0, 4.6),
                    (13.0, 9.4),
                    (9.4, 13.0),
                    (4.6, 13.0),
                    (1.0, 9.4),
                    (1.0, 4.6),
                ],
                c.red,
            );
            sign.round_rect(window, 4.0, 6.2, 6.0, 1.6, 0.8, c.term_ink, None);
        }
        "finished" | "verified" => {
            pen.circle(
                window,
                7.0,
                7.0,
                6.0,
                if state == "verified" { c.green } else { c.grey },
            );
            pen.stroke(
                window,
                &[(4.3, 7.2), (6.2, 9.1), (9.8, 5.3)],
                1.5,
                c.term_ink,
                true,
            );
        }
        "disconnected" => ring(&pen, window, 7.0, 7.0, 5.0, 5.0, 1.6, c.grey),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timers_started_apart_change_on_the_same_second() {
        // Two agents started 300 ms apart, read anywhere in one second.
        let wall = 1_000_000_450;
        let now = on_second(wall);
        let a = shown_at(timer_change(now, 999_990_100));
        let b = shown_at(timer_change(now, 999_990_400));
        assert_eq!(a, b);
        assert_eq!(a, now + 1000);
        assert!(a > wall);
        // Past a minute the timer reads minutes and wakes on the minute's
        // first grid second, not every second.
        let old = shown_at(timer_change(now, now - 90_000 - 300));
        assert!(old > now + 1000 && old - now <= 60_000, "{}", old - now);
        assert_eq!(old % 1000, 0);
    }

    #[test]
    fn a_still_icon_wakes_only_when_it_changes() {
        let wakes = |state: &str| {
            let (mut t, mut n) = (0.0, 0);
            while t < 12_000.0 {
                t = clock::next_change(t, clock::LOOP_MS, 3000.0, |t| icon_look(state, t));
                n += 1;
            }
            n
        };
        // The cursor: on and off, twice a second, not four times.
        assert_eq!(wakes("running"), 24);
        // The hourglass turns over in 450 ms of every 3 s.
        assert!(wakes("waiting") <= 12, "{}", wakes("waiting"));
        assert!(icon_look("thinking", 0.0).is_none());
    }

    #[test]
    fn icons_that_move_are_the_working_states() {
        assert!(icon_loops("thinking") && icon_loops("waiting") && icon_loops("asleep"));
        assert!(!icon_loops("finished") && !icon_loops("blocked") && !icon_loops("disconnected"));
    }
}
