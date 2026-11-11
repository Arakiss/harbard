//! The Lobby's motion, from the console's `rooms.css` (`DESIGN.md` §3.1):
//! robots and cards arrive staggered, a stand lifts 5 px and a card 3 px
//! under the pointer, the portrait waves hello, a fresh introduction pops.
//! Pure functions of elapsed time; the screen asks for frames while any of
//! them is still moving, and under reduced motion (M7) every one is at its
//! end state at once.

use crate::robot::draw::{EASE, Ease, track};
use std::time::Instant;

/// `hb-lobby-arrive`: 380 ms, 10 px up from transparent.
pub const ARRIVE_MS: f32 = 380.;
const ARRIVE_RISE: f32 = 10.;
const ARRIVE_EASE: Ease = Ease(0.2, 0.9, 0.3, 1.25);
/// The stagger: 45 ms a robot on the shelf, 35 ms a card, at most 12 steps.
pub const STAND_STEP_MS: f32 = 45.;
pub const CARD_STEP_MS: f32 = 35.;
const ORDER_CAP: usize = 12;

/// The hover transitions: 160 ms.
pub const LIFT_MS: f32 = 160.;
/// `.hb-lobby-stand:hover`: 5 px up, with a spring.
pub const STAND_LIFT: f32 = 5.;
const STAND_EASE: Ease = Ease(0.3, 1.5, 0.5, 1.);
/// `.hb-lobby-card:hover`: 3 px up, `ease`.
pub const CARD_LIFT: f32 = 3.;

/// `hb-lobby-hello`: 620 ms of a wave as the pointer arrives.
pub const HELLO_MS: f32 = 620.;
/// `hb-lobby-said`: 420 ms.
pub const SAID_MS: f32 = 420.;
const SAID_EASE: Ease = Ease(0.2, 0.9, 0.3, 1.4);

/// Where a piece is now: its opacity and its offset in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub opacity: f32,
    pub dx: f32,
    pub dy: f32,
}

impl Pose {
    pub const REST: Pose = Pose {
        opacity: 1.,
        dx: 0.,
        dy: 0.,
    };
}

/// The arrival of the `order`-th piece `ms` after the screen opened, each
/// one `step` ms behind the one before (`animation-fill-mode: both`).
pub fn arrival(ms: f32, order: usize, step: f32) -> Pose {
    let start = order.min(ORDER_CAP) as f32 * step;
    let q = ((ms - start) / ARRIVE_MS).clamp(0., 1.);
    if q >= 1. {
        return Pose::REST;
    }
    // Opacity follows the same curve, held inside 0..1.
    let k = if q <= 0. { 0. } else { ARRIVE_EASE.at(q) };
    Pose {
        opacity: k.clamp(0., 1.),
        dx: 0.,
        dy: ARRIVE_RISE * (1. - k),
    }
}

/// When the last of `n` pieces has arrived, in ms after opening.
pub fn arrival_ends(n: usize, step: f32) -> f32 {
    n.saturating_sub(1).min(ORDER_CAP) as f32 * step + ARRIVE_MS
}

/// The pointer entering or leaving a piece: which way, when, and how far
/// up it was at that moment (a transition reverses from where it stands).
#[derive(Clone, Copy, Debug)]
pub struct Hover {
    pub on: bool,
    pub at: Instant,
    pub from: f32,
}

/// How far up (0..=1 of the lift, overshooting for the stand) a hovered
/// piece stands `ms` after the pointer came or went.
pub fn lift(h: &Hover, ms: f32, stand: bool) -> f32 {
    let to = if h.on { 1. } else { 0. };
    let ease = if stand { STAND_EASE } else { EASE };
    let q = (ms / LIFT_MS).clamp(0., 1.);
    if q >= 1. {
        return to;
    }
    if q <= 0. {
        return h.from;
    }
    h.from + (to - h.from) * ease.at(q)
}

/// The portrait's hello `ms` after the pointer arrived, for a robot of
/// `size`: up and back, swaying as the console's −5°/+4° turn moves it.
pub fn hello(ms: f32, size: f32) -> Pose {
    if !(0. ..HELLO_MS).contains(&ms) || ms == 0. {
        return Pose::REST;
    }
    let q = ms / HELLO_MS;
    let deg = track(q, &[(0., 0.), (0.25, -5.), (0.6, 4.), (1., 0.)], EASE);
    let dy = track(q, &[(0., 0.), (0.25, -4.), (0.6, -2.), (1., 0.)], EASE);
    Pose {
        opacity: 1.,
        // A turn about the centre moves the head sideways by about this.
        dx: deg.to_radians() * size * 0.35,
        dy,
    }
}

/// A fresh introduction `ms` after it was first seen: it fades in and
/// settles 4 px, where the console scales it from 0.92.
pub fn said(ms: f32) -> Pose {
    let q = (ms / SAID_MS).clamp(0., 1.);
    if q >= 1. {
        return Pose::REST;
    }
    let k = if q <= 0. { 0. } else { SAID_EASE.at(q) };
    Pose {
        opacity: k.clamp(0., 1.),
        dx: 0.,
        dy: 4. * (1. - k),
    }
}

/// Whether an introduction said at `intro_ts` counts as new: after the
/// moment this app started watching (`baseline`, ms since the epoch).
pub fn is_fresh(intro_ts: Option<u64>, baseline: u64) -> bool {
    intro_ts.is_some_and(|ts| baseline > 0 && ts > baseline)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrival_starts_low_and_clear_and_ends_in_place() {
        let first = arrival(0., 0, CARD_STEP_MS);
        assert_eq!(first.opacity, 0.);
        assert!((first.dy - 10.).abs() < 1e-4);
        assert_eq!(arrival(ARRIVE_MS, 0, CARD_STEP_MS), Pose::REST);
    }

    #[test]
    fn arrival_is_staggered_and_capped_at_twelve() {
        // The third card has not started when the first is under way.
        assert!(arrival(60., 0, CARD_STEP_MS).opacity > 0.);
        assert_eq!(arrival(60., 2, CARD_STEP_MS).opacity, 0.);
        // Order 40 waits as long as order 12.
        assert_eq!(
            arrival(500., 40, CARD_STEP_MS),
            arrival(500., 12, CARD_STEP_MS)
        );
        assert_eq!(arrival_ends(1, STAND_STEP_MS), ARRIVE_MS);
        assert_eq!(arrival_ends(30, CARD_STEP_MS), 12. * 35. + ARRIVE_MS);
    }

    #[test]
    fn arrival_overshoots_but_opacity_never_does() {
        let mid: Vec<Pose> = (0..38).map(|i| arrival(i as f32 * 10., 0, 0.)).collect();
        assert!(mid.iter().any(|p| p.dy < 0.), "the spring passes its mark");
        assert!(mid.iter().all(|p| (0. ..=1.).contains(&p.opacity)));
    }

    #[test]
    fn lift_reverses_from_where_it_stands() {
        let now = Instant::now();
        let up = Hover {
            on: true,
            at: now,
            from: 0.,
        };
        assert_eq!(lift(&up, 0., false), 0.);
        assert!((lift(&up, LIFT_MS, false) - 1.).abs() < 1e-4);
        let down = Hover {
            on: false,
            at: now,
            from: 0.4,
        };
        assert!((lift(&down, 0., false) - 0.4).abs() < 1e-4);
        assert!(lift(&down, LIFT_MS, false).abs() < 1e-4);
        // The stand's spring overshoots on the way up.
        assert!((0..16).any(|i| lift(&up, i as f32 * 10., true) > 1.));
    }

    #[test]
    fn hello_waves_and_comes_to_rest() {
        assert_eq!(hello(0., 104.), Pose::REST);
        let top = hello(HELLO_MS * 0.25, 104.);
        assert!((top.dy + 4.).abs() < 1e-3);
        assert!(top.dx < 0.);
        assert!(hello(HELLO_MS * 0.6, 104.).dx > 0.);
        assert_eq!(hello(HELLO_MS, 104.), Pose::REST);
        assert_eq!(hello(-1., 104.), Pose::REST);
    }

    #[test]
    fn said_pops_in() {
        assert_eq!(said(0.).opacity, 0.);
        assert_eq!(said(SAID_MS), Pose::REST);
    }

    #[test]
    fn only_what_was_said_after_the_baseline_is_fresh() {
        assert!(is_fresh(Some(2_000), 1_000));
        assert!(!is_fresh(Some(1_000), 1_000));
        assert!(!is_fresh(None, 1_000));
        assert!(!is_fresh(Some(2_000), 0));
    }
}
