//! The living logo (`components/brand/*`, `mark.css`, `useLogo`): the brand
//! mark at the head of the rail, whose dot says how the fleet is.
//!
//! Lasting states (first match wins, `logoState`): disconnected, connecting,
//! blocked, needs you, working, idle. One-shots over them: a new message pops
//! the bubble, a verified delivery grows the dot and draws a check in it.
//!
//! Motion is cheap on purpose: one timer, only while something moves, at the
//! rate the motion needs (4 fps for the slow breathing, 8 for the wave, both
//! on the robots' clock so they share its frames; 30 for the short
//! one-shots), notifying only this view. With motion reduced, or the window
//! in the background, the mark is still and nothing ticks.

use crate::desk::{Connection, Desk};
use crate::robot::clock;
use crate::theme::Theme;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, Hsla, IntoElement, ParentElement, PathBuilder, Render, Styled, Task, Transformation,
    Window, canvas, div, point, px, size, svg,
};
use std::collections::HashSet;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogoState {
    Idle,
    Working,
    Needs,
    Blocked,
    Connecting,
    Disconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OneShot {
    Message,
    Verified,
}

impl OneShot {
    /// How long it runs (mark.css), plus the console's 100 ms margin.
    fn length(self) -> Duration {
        match self {
            OneShot::Message => Duration::from_millis(800),
            OneShot::Verified => Duration::from_millis(2_760),
        }
    }
}

/// Any agent active within this makes the logo "working".
const WORKING_MS: u64 = 60_000;

/// States in which an agent is at work even while its transcript is quiet.
pub const WORKING_STATES: [&str; 7] = [
    "thinking",
    "running",
    "testing",
    "editing",
    "reading",
    "web",
    "delegating",
];

/// The logo's lasting state, by precedence (`logoState`).
pub fn logo_state(
    connection: Connection,
    blocked: u64,
    needs: u64,
    last_actions: &[Option<u64>],
    working: usize,
    now: u64,
) -> LogoState {
    match connection {
        Connection::Disconnected => return LogoState::Disconnected,
        Connection::Connecting => return LogoState::Connecting,
        Connection::Connected => {}
    }
    if blocked > 0 {
        return LogoState::Blocked;
    }
    if needs > 0 {
        return LogoState::Needs;
    }
    if working > 0
        || last_actions
            .iter()
            .flatten()
            .any(|&ms| now.saturating_sub(ms) <= WORKING_MS)
    {
        return LogoState::Working;
    }
    LogoState::Idle
}

/// The logo's inputs from the live read: the faces slice when there is one.
fn read_state(desk: &Desk) -> (LogoState, Vec<(String, String, u64)>, bool) {
    let now = desk.now_ms();
    let connection = desk.connection();
    let (blocked, needs) = match (&desk.activity, &desk.view) {
        (Some(a), _) => (a.blocked, a.needs),
        (None, Some(v)) => (
            v.cards
                .iter()
                .filter(|c| c.kind == "blocked" && c.needs && c.resolved_ms.is_none())
                .count() as u64,
            v.unread.activity,
        ),
        _ => (0, 0),
    };
    let mut last = Vec::new();
    let mut working = 0;
    let mut reactions = Vec::new();
    let motion = desk
        .activity
        .as_ref()
        .and_then(|a| a.motion.clone())
        .or_else(|| desk.view.as_ref().and_then(|v| v.motion.clone()));
    if let Some(a) = &desk.activity {
        for ag in &a.agents {
            last.push(ag.last_action_ms);
            working += WORKING_STATES.contains(&ag.activity.state.as_str()) as usize;
            reactions.extend(
                ag.reactions
                    .iter()
                    .map(|r| (ag.key.clone(), r.kind.clone(), r.at_ms)),
            );
        }
    } else if let Some(v) = &desk.view {
        for ag in &v.agents {
            last.push(ag.last_action_ms);
            working += WORKING_STATES.contains(&ag.activity.state.as_str()) as usize;
            reactions.extend(
                ag.reactions
                    .iter()
                    .map(|r| (ag.key.clone(), r.kind.clone(), r.at_ms)),
            );
        }
    }
    let state = logo_state(connection, blocked, needs, &last, working, now);
    (state, reactions, motion.as_deref() == Some("reduce"))
}

pub struct LivingMark {
    state: LogoState,
    /// When the lasting state began (its own one-time motion: bounce, shake).
    since: Instant,
    once: Option<(OneShot, Instant)>,
    seen: HashSet<(String, String, u64)>,
    /// Reactions at or before this (serve's clock at the first read) are history.
    baseline: Option<u64>,
    reduce: bool,
    quiet: bool,
    active: bool,
    hover: Option<(bool, Instant)>,
    ticker: Option<Task<()>>,
    /// The mark's side in points (the rail's 32 unless [`LivingMark::set_size`]).
    size: f32,
    /// Moves while its window is visible, focused or not (the Strip, a
    /// window nobody focuses).
    ambient: bool,
    visible: bool,
}

impl LivingMark {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |m, _, cx| m.sync(cx)).detach();
        let profile = super::profile::ProfileStore::get(cx);
        cx.observe(&profile, |m, p, cx| {
            m.quiet = !p.read(cx).one_shots(super::profile::now_ms());
        })
        .detach();
        cx.observe_window_activation(window, |m, window, cx| {
            m.active = window.is_window_active();
            m.pace(cx);
        })
        .detach();
        let mut m = LivingMark {
            state: LogoState::Connecting,
            since: Instant::now(),
            once: None,
            seen: HashSet::new(),
            baseline: None,
            reduce: false,
            quiet: false,
            active: window.is_window_active(),
            hover: None,
            ticker: None,
            size: MARK,
            ambient: false,
            visible: window.is_visible(),
        };
        m.sync(cx);
        m
    }

    /// A mark for a window nobody focuses (the Strip): it moves while the
    /// window is visible rather than while it is in front.
    pub fn ambient(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_window_visibility(window, |m, _, window, cx| {
            m.visible = window.is_visible();
            m.pace(cx);
        })
        .detach();
        LivingMark {
            ambient: true,
            ..LivingMark::new(window, cx)
        }
    }

    /// Draw the mark `size` points wide (the 64-unit drawing scaled).
    pub fn set_size(&mut self, size: f32, cx: &mut Context<Self>) {
        if (size - self.size).abs() > 0.01 {
            self.size = size;
            cx.notify();
        }
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let desk = Desk::get(cx).read(cx);
        let (state, reactions, reduce) = read_state(desk);
        let now = desk.now_ms();
        let has_read = desk.view.is_some() || desk.activity.is_some();
        let mut changed = false;
        if state != self.state {
            self.state = state;
            self.since = Instant::now();
            changed = true;
        }
        if reduce != self.reduce {
            self.reduce = reduce;
            changed = true;
        }
        if has_read {
            let baseline = *self.baseline.get_or_insert(now);
            let mut next = None;
            for r in reactions {
                if r.2 <= baseline || !self.seen.insert(r.clone()) {
                    continue;
                }
                if r.1 == "hop" {
                    next = Some(OneShot::Verified);
                } else if r.1 == "glance" && next.is_none() {
                    next = Some(OneShot::Message);
                }
            }
            if let Some(once) = next
                && !self.reduce
                && !self.quiet
            {
                self.once = Some((once, Instant::now()));
                changed = true;
            }
        }
        if changed {
            self.pace(cx);
            cx.notify();
        }
    }

    /// The frame interval the current motion needs; `None` when still.
    fn interval(&self) -> Option<Duration> {
        let seen = if self.ambient {
            self.visible
        } else {
            self.active
        };
        if self.reduce || !seen {
            return None;
        }
        let short = self.once.is_some_and(|(o, at)| at.elapsed() < o.length())
            || self
                .hover
                .is_some_and(|(_, at)| at.elapsed() < Duration::from_millis(400))
            || match self.state {
                LogoState::Needs => self.since.elapsed() < Duration::from_millis(900),
                LogoState::Blocked => self.since.elapsed() < Duration::from_millis(750),
                _ => false,
            };
        if short {
            return Some(Duration::from_millis(33));
        }
        // The slow pulse and the wave run on the robots' clock (4 and 8 fps),
        // so their frames are the frames the robots draw anyway.
        match self.state {
            LogoState::Idle => Some(Duration::from_millis(clock::LOOP_MS as u64)),
            LogoState::Working => Some(Duration::from_millis(clock::TICK_MS as u64)),
            LogoState::Connecting => Some(Duration::from_millis(50)),
            _ => None,
        }
    }

    /// Start or stop the timer to match the motion.
    fn pace(&mut self, cx: &mut Context<Self>) {
        if self.interval().is_none() {
            self.ticker = None;
            return;
        }
        if self.ticker.is_some() {
            return;
        }
        self.ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                let Ok(Some(every)) = this.update(cx, |m, _| m.interval()) else {
                    break;
                };
                // On the clock's grid, so a tick lands in the robots' frame.
                let step = every.as_secs_f64() * 1000.0;
                let now = clock::clock_ms(Instant::now());
                let at = clock::instant_at(clock::next_grid(now, step));
                cx.background_executor()
                    .timer(at.saturating_duration_since(Instant::now()))
                    .await;
                if this.update(cx, |_, cx| cx.notify()).is_err() {
                    break;
                }
            }
            let _ = this.update(cx, |m, cx| {
                m.ticker = None;
                cx.notify();
            });
        }));
    }

    /// The tile under the mark was entered or left: the greeting tilt.
    pub fn set_hover(&mut self, on: bool, cx: &mut Context<Self>) {
        if self.hover.map(|(h, _)| h) != Some(on) {
            self.hover = Some((on, Instant::now()));
            self.pace(cx);
            cx.notify();
        }
    }
}

/// Piecewise keyframes, eased in and out per segment as CSS does.
fn frames(t: f32, keys: &[(f32, f32)]) -> f32 {
    let t = t.clamp(0., 1.);
    for w in keys.windows(2) {
        let ((a, va), (b, vb)) = (w[0], w[1]);
        if t <= b {
            let u = if b > a { (t - a) / (b - a) } else { 1. };
            let e = u * u * (3. - 2. * u);
            return va + (vb - va) * e;
        }
    }
    keys.last().map_or(0., |k| k.1)
}

fn ease_out(t: f32) -> f32 {
    1. - (1. - t.clamp(0., 1.)).powi(3)
}

/// The rail's mark is 32px: the 64-unit box at half size; the dot sits at
/// (54,13) r 7.
const MARK: f32 = 32.;

impl Render for LivingMark {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let (mark, k) = (self.size, self.size / 64.);
        let moving = !self.reduce;
        let since = self.since.elapsed().as_secs_f32();
        let clock = Instant::now();
        let once = self
            .once
            .filter(|(o, at)| moving && at.elapsed() < o.length())
            .map(|(o, at)| (o, at.elapsed().as_secs_f32()));
        if once.is_none() {
            self.once = None;
        }

        // Ink: hover tilt, blocked shake, message pop.
        let mut rotate = 0f32;
        if let Some((on, at)) = self.hover
            && moving
        {
            let u = (at.elapsed().as_secs_f32() / 0.35).min(1.);
            let spring = 1. - (1. - u).powi(3) * (1. - 1.6 * u).max(-0.2);
            let v = if on {
                -7. * spring
            } else {
                -7. * (1. - spring)
            };
            rotate += v;
        }
        if self.state == LogoState::Blocked && moving && since < 0.72 {
            rotate += frames(
                since / 0.72,
                &[
                    (0., 0.),
                    (0.22, -3.),
                    (0.44, 2.5),
                    (0.67, -1.5),
                    (0.83, 0.6),
                    (1., 0.),
                ],
            );
        }
        let mut scale = 1f32;
        if let Some((OneShot::Message, t)) = once {
            scale = frames(t / 0.7, &[(0., 1.), (0.27, 0.93), (0.64, 1.04), (1., 1.)]);
        }
        let pop_shift = point(
            px((1. - scale) * -0.36 * mark),
            px((1. - scale) * 0.5 * mark),
        );
        let ink = svg()
            .path("brand/mark-ink.svg")
            .size(px(mark))
            .text_color(theme.c("rail-ink"))
            .with_transformation(
                Transformation::rotate(gpui::radians(rotate.to_radians()))
                    .with_scaling(size(scale, scale))
                    .with_translation(pop_shift),
            )
            .when(self.state == LogoState::Disconnected, |s| s.opacity(0.45));

        // The dot.
        let dot_color: Hsla = match self.state {
            LogoState::Needs => theme.c("mark-dot-amber"),
            LogoState::Blocked => theme.c("mark-dot-red"),
            _ => theme.c("mark-dot"),
        };
        let mut dot_scale = 1f32;
        let mut dot_alpha = 1f32;
        let mut dot_y = 0f32;
        let t_loop = clock.duration_since(self.since).as_secs_f32();
        match self.state {
            LogoState::Idle if moving => {
                let u = (t_loop % 4.8) / 4.8;
                dot_scale = frames(u, &[(0., 1.), (0.5, 1.08), (1., 1.)]);
                dot_alpha = frames(u, &[(0., 0.88), (0.5, 1.), (1., 0.88)]);
            }
            LogoState::Needs if moving && since < 0.88 => {
                dot_y = frames(
                    since / 0.88,
                    &[(0., 0.), (0.27, -2.6), (0.55, 0.), (0.77, -1.3), (1., 0.)],
                ) * k
                    * 2.;
            }
            _ => {}
        }
        if let Some((OneShot::Verified, t)) = once {
            dot_scale = frames(t / 2.66, &[(0., 1.), (0.16, 1.25), (0.84, 1.25), (1., 1.)]);
        }
        let check = once.and_then(|(o, t)| (o == OneShot::Verified).then_some(t / 2.66));
        let wave = (self.state == LogoState::Working && moving).then(|| {
            let u = (t_loop % 2.6) / 2.6;
            let p = ease_out((u / 0.7).min(1.));
            (1. + 0.9 * p, 0.7 * (1. - p))
        });
        let spin = (self.state == LogoState::Connecting)
            .then(|| if moving { (t_loop % 1.1) / 1.1 } else { 0. });
        let state = self.state;
        let grey = theme.c("mark-dot-grey");
        let rail_ink = theme.c("rail-ink");

        let (cx0, cy0, r) = (54. * k, 13. * k, 7. * k);
        let marks = canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                let o = bounds.origin;
                let c = point(o.x + px(cx0), o.y + px(cy0 + dot_y));
                let circle = |b: &mut PathBuilder, radius: f32| {
                    b.move_to(point(c.x, c.y - px(radius)));
                    b.arc_to(
                        point(px(radius), px(radius)),
                        px(0.),
                        false,
                        true,
                        point(c.x, c.y + px(radius)),
                    );
                    b.arc_to(
                        point(px(radius), px(radius)),
                        px(0.),
                        false,
                        true,
                        point(c.x, c.y - px(radius)),
                    );
                    b.close();
                };
                if let Some((s, a)) = wave {
                    let mut b = PathBuilder::stroke(px(1.6 * k));
                    circle(&mut b, r * s);
                    if let Ok(p) = b.build() {
                        window.paint_path(p, dot_color.opacity(a));
                    }
                }
                match state {
                    LogoState::Connecting => {
                        // The dot becomes a spinning arc (dasharray 20 18 on r 6).
                        let ar = r - k;
                        let start = spin.unwrap_or(0.) * std::f32::consts::TAU
                            - std::f32::consts::FRAC_PI_2;
                        let sweep = (20. * k) / ar;
                        let at = |a: f32| point(c.x + px(ar * a.cos()), c.y + px(ar * a.sin()));
                        let mut b = PathBuilder::stroke(px(2.4 * k));
                        b.move_to(at(start));
                        b.arc_to(
                            point(px(ar), px(ar)),
                            px(0.),
                            sweep > std::f32::consts::PI,
                            true,
                            at(start + sweep),
                        );
                        if let Ok(p) = b.build() {
                            window.paint_path(p, grey);
                        }
                    }
                    LogoState::Disconnected => {
                        let mut b = PathBuilder::stroke(px(2.2 * k));
                        circle(&mut b, r * 0.8);
                        if let Ok(p) = b.build() {
                            window.paint_path(p, grey);
                        }
                    }
                    _ => {
                        let mut b = PathBuilder::fill();
                        circle(&mut b, r * dot_scale);
                        if let Ok(p) = b.build() {
                            window.paint_path(p, dot_color.opacity(dot_alpha));
                        }
                    }
                }
                if let Some(u) = check {
                    // Drawn between 19% and 41%, held, gone by 95%.
                    let drawn = ((u - 0.19) / 0.22).clamp(0., 1.);
                    let alpha = if u < 0.78 {
                        1.
                    } else {
                        (1. - (u - 0.78) / 0.17).clamp(0., 1.)
                    };
                    if drawn > 0. && alpha > 0. {
                        let s = r / 7.;
                        let p0 = point(c.x - px(3.4 * s), c.y + px(0.2 * s));
                        let p1 = point(p0.x + px(2.4 * s), p0.y + px(2.4 * s));
                        let p2 = point(p1.x + px(4.6 * s), p1.y - px(4.8 * s));
                        let l1 = 2.4 * s * std::f32::consts::SQRT_2;
                        let l2 = (4.6f32 * 4.6 + 4.8 * 4.8).sqrt() * s;
                        let mut len = drawn * (l1 + l2);
                        let mut b = PathBuilder::stroke(px(2.2 * k));
                        b.move_to(p0);
                        let f = (len / l1).min(1.);
                        b.line_to(point(p0.x + (p1.x - p0.x) * f, p0.y + (p1.y - p0.y) * f));
                        len -= l1;
                        if len > 0. {
                            let f = (len / l2).min(1.);
                            b.line_to(point(p1.x + (p2.x - p1.x) * f, p1.y + (p2.y - p1.y) * f));
                        }
                        if let Ok(p) = b.build() {
                            window.paint_path(p, rail_ink.opacity(alpha));
                        }
                    }
                }
            },
        )
        .absolute()
        .inset_0();

        div().size(px(mark)).relative().child(ink).child(marks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_logo_takes_the_first_state_that_holds() {
        let now = 1_000_000;
        assert_eq!(
            logo_state(Connection::Disconnected, 3, 3, &[], 2, now),
            LogoState::Disconnected
        );
        assert_eq!(
            logo_state(Connection::Connecting, 3, 3, &[], 2, now),
            LogoState::Connecting
        );
        assert_eq!(
            logo_state(Connection::Connected, 1, 3, &[], 2, now),
            LogoState::Blocked
        );
        assert_eq!(
            logo_state(Connection::Connected, 0, 3, &[], 2, now),
            LogoState::Needs
        );
        assert_eq!(
            logo_state(Connection::Connected, 0, 0, &[], 1, now),
            LogoState::Working
        );
        assert_eq!(
            logo_state(Connection::Connected, 0, 0, &[Some(now - 59_000)], 0, now),
            LogoState::Working
        );
        assert_eq!(
            logo_state(
                Connection::Connected,
                0,
                0,
                &[Some(now - 61_000), None],
                0,
                now
            ),
            LogoState::Idle
        );
    }

    #[test]
    fn keyframes_hit_their_stops() {
        let k = [(0., 0.), (0.5, 10.), (1., 0.)];
        assert_eq!(frames(0., &k), 0.);
        assert_eq!(frames(0.5, &k), 10.);
        assert_eq!(frames(1., &k), 0.);
        assert!(frames(0.25, &k) > 0. && frames(0.25, &k) < 10.);
    }
}
