//! A bust's gesture when you reach for it (`.hb-av-pick[data-motion]` and
//! `.hb-me:hover .hb-op-photo` in the console): each gallery character hops,
//! tilts, sways, turns, nods, pops or floats once, and the operator's own
//! face in the rail nods. Raster images cannot rotate here, so a rotation is
//! drawn as the lean it gives the head: a sideways shift of the bust.
//!
//! A gesture runs once from the moment it starts and asks for frames only
//! while it runs; with reduced motion (M7) it never starts.

use crate::robot::draw::{Ease, track};
use gpui::{AnyElement, Div, IntoElement, ParentElement, Styled, div, px};
use std::time::{Duration, Instant};

/// `cubic-bezier(0.3, 0.7, 0.3, 1)`, applied per keyframe as CSS does.
const EASE: Ease = Ease(0.3, 0.7, 0.3, 1.0);
/// The scale's pivot: `transform-origin: 50% 80%`.
const ORIGIN_X: f32 = 0.5;
const ORIGIN_Y: f32 = 0.8;
/// How far the top of the head travels per unit of `sin(angle)`, as a share
/// of the bust: the pivot sits 80 % down, so the visible lean is about half.
const LEAN: f32 = 0.4;

/// The console's gestures (`AvatarMotion`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gesture {
    Hop,
    Tilt,
    Sway,
    Turn,
    Nod,
    Pop,
    Float,
    /// The operator's face in the rail: `hb-av-nod` over 620 ms.
    RailNod,
    /// Every character once picked: `hb-av-pop` over 520 ms.
    Picked,
}

/// Where the bust is in a frame: its scale, and its shift and lean as a
/// share of its own size (`dx`, `dy`), the lean in degrees.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub scale: f32,
    pub dx: f32,
    pub dy: f32,
    pub deg: f32,
}

impl Pose {
    pub const REST: Pose = Pose {
        scale: 1.,
        dx: 0.,
        dy: 0.,
        deg: 0.,
    };
}

/// One keyframe: offset, scale, translate x and y (share of size), rotation.
type Key = (f32, f32, f32, f32, f32);

const HOP: &[Key] = &[
    (0., 1., 0., 0., 0.),
    (0.30, 1.14, 0., -0.07, 0.),
    (0.55, 1.10, 0., 0., 0.),
    (0.75, 1.10, 0., -0.03, 0.),
    (1., 1., 0., 0., 0.),
];
const TILT: &[Key] = &[
    (0., 1., 0., 0., 0.),
    (0.35, 1.18, 0., 0., -7.),
    (0.70, 1.14, 0., 0., 3.),
    (1., 1., 0., 0., 0.),
];
const SWAY: &[Key] = &[
    (0., 1., 0., 0., 0.),
    (0.25, 1.16, -0.02, 0., -5.),
    (0.55, 1.16, 0.02, 0., 5.),
    (0.80, 1.12, 0., 0., -2.),
    (1., 1., 0., 0., 0.),
];
const TURN: &[Key] = &[
    (0., 1., 0., 0., 0.),
    (0.30, 1.22, 0., 0., -11.),
    (0.65, 1.22, 0., 0., 11.),
    (1., 1., 0., 0., 0.),
];
const NOD: &[Key] = &[
    (0., 1., 0., 0., 0.),
    (0.30, 1.10, 0., 0.04, 0.),
    (0.55, 1.08, 0., 0., 0.),
    (0.78, 1.08, 0., 0.03, 0.),
    (1., 1., 0., 0., 0.),
];
const POP: &[Key] = &[
    (0., 1., 0., 0., 0.),
    (0.40, 1.20, 0., 0., 0.),
    (0.70, 1.05, 0., 0., 0.),
    (1., 1., 0., 0., 0.),
];
const FLOAT: &[Key] = &[
    (0., 1., 0., 0., 0.),
    (0.50, 1.14, 0., -0.05, 2.),
    (1., 1., 0., 0., 0.),
];

impl Gesture {
    /// The console's `data-motion` name.
    pub fn parse(s: &str) -> Option<Gesture> {
        Some(match s {
            "hop" => Gesture::Hop,
            "tilt" => Gesture::Tilt,
            "sway" => Gesture::Sway,
            "turn" => Gesture::Turn,
            "nod" => Gesture::Nod,
            "pop" => Gesture::Pop,
            "float" => Gesture::Float,
            _ => return None,
        })
    }

    pub fn duration(self) -> Duration {
        Duration::from_millis(match self {
            Gesture::Float => 1400,
            Gesture::RailNod => 620,
            Gesture::Picked => 520,
            _ => 720,
        })
    }

    fn keys(self) -> &'static [Key] {
        match self {
            Gesture::Hop => HOP,
            Gesture::Tilt => TILT,
            Gesture::Sway => SWAY,
            Gesture::Turn => TURN,
            Gesture::Nod | Gesture::RailNod => NOD,
            Gesture::Pop | Gesture::Picked => POP,
            Gesture::Float => FLOAT,
        }
    }

    /// The pose at `t` (0 to 1 of the gesture), each keyframe eased.
    pub fn pose(self, t: f32) -> Pose {
        let keys = self.keys();
        let lane = |pick: fn(&Key) -> f32| -> Vec<(f32, f32)> {
            keys.iter().map(|k| (k.0, pick(k))).collect()
        };
        Pose {
            scale: track(t, &lane(|k| k.1), EASE),
            dx: track(t, &lane(|k| k.2), EASE),
            dy: track(t, &lane(|k| k.3), EASE),
            deg: track(t, &lane(|k| k.4), EASE),
        }
    }

    /// The pose `elapsed` after it started, or `None` once it is over.
    pub fn pose_after(self, elapsed: Duration) -> Option<Pose> {
        let total = self.duration();
        (elapsed < total).then(|| self.pose(elapsed.as_secs_f32() / total.as_secs_f32()))
    }
}

/// A gesture under way: which one, on what, since when.
#[derive(Clone, Debug)]
pub struct Running<K> {
    pub key: K,
    pub gesture: Gesture,
    pub since: Instant,
}

impl<K: PartialEq> Running<K> {
    pub fn new(key: K, gesture: Gesture) -> Self {
        Running {
            key,
            gesture,
            since: Instant::now(),
        }
    }

    /// The pose of `key` now, if its gesture is still running.
    pub fn pose_of(&self, key: &K) -> Option<Pose> {
        (self.key == *key)
            .then(|| self.gesture.pose_after(self.since.elapsed()))
            .flatten()
    }
}

/// Where the bust's box sits inside a `size` frame in `pose`: its left and
/// top offsets and its side, in pixels.
pub fn placement(size: f32, pose: Pose) -> (f32, f32, f32) {
    let side = size * pose.scale;
    let lean = pose.deg.to_radians().sin() * LEAN;
    let left = size * ORIGIN_X * (1. - pose.scale) + (pose.dx + lean) * side;
    let top = size * ORIGIN_Y * (1. - pose.scale) + pose.dy * side;
    (left, top, side)
}

/// A `size` frame with rounded corners that clips the bust `draw` paints at
/// the side it is given, posed.
pub fn posed(size: f32, corner: f32, pose: Pose, draw: impl FnOnce(f32) -> AnyElement) -> Div {
    let (left, top, side) = placement(size, pose);
    div()
        .relative()
        .flex_none()
        .size(px(size))
        .rounded(px(corner))
        .overflow_hidden()
        .child(
            div()
                .absolute()
                .left(px(left))
                .top(px(top))
                .size(px(side))
                .child(draw(side)),
        )
}

/// [`posed`] for an element that fills whatever box it is in.
pub fn posed_fill(size: f32, corner: f32, pose: Pose, child: impl IntoElement) -> Div {
    posed(size, corner, pose, move |_| {
        div().size_full().child(child).into_any_element()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_gesture_starts_and_ends_at_rest() {
        for g in [
            Gesture::Hop,
            Gesture::Tilt,
            Gesture::Sway,
            Gesture::Turn,
            Gesture::Nod,
            Gesture::Pop,
            Gesture::Float,
            Gesture::RailNod,
            Gesture::Picked,
        ] {
            for t in [0., 1.] {
                let p = g.pose(t);
                let off = (p.scale - 1.).abs() + p.dx.abs() + p.dy.abs() + p.deg.abs();
                assert!(off < 1e-3, "{g:?} at {t}: {p:?}");
            }
        }
    }

    #[test]
    fn the_keyframes_land_on_the_consoles_values() {
        let p = Gesture::Hop.pose(0.30);
        assert!((p.scale - 1.14).abs() < 1e-4 && (p.dy + 0.07).abs() < 1e-4);
        let p = Gesture::Turn.pose(0.65);
        assert!((p.scale - 1.22).abs() < 1e-4 && (p.deg - 11.).abs() < 1e-4);
        assert!((Gesture::Pop.pose(0.40).scale - 1.2).abs() < 1e-4);
    }

    #[test]
    fn a_gesture_is_over_after_its_duration() {
        let g = Gesture::RailNod;
        assert!(g.pose_after(Duration::from_millis(300)).is_some());
        assert!(g.pose_after(Duration::from_millis(620)).is_none());
        assert_eq!(Gesture::Float.duration(), Duration::from_millis(1400));
    }

    #[test]
    fn the_scale_grows_from_low_in_the_middle() {
        // At rest the bust fills the frame exactly.
        assert_eq!(placement(44., Pose::REST), (0., 0., 44.));
        // Scaled up, it grows mostly upward: the pivot is 80 % down.
        let (left, top, side) = placement(
            100.,
            Pose {
                scale: 1.2,
                ..Pose::REST
            },
        );
        assert!((side - 120.).abs() < 1e-3);
        assert!((left + 10.).abs() < 1e-3);
        assert!((top + 16.).abs() < 1e-3);
    }

    #[test]
    fn a_lean_to_the_left_shifts_the_bust_left() {
        let (left, _, _) = placement(
            100.,
            Pose {
                deg: -7.,
                ..Pose::REST
            },
        );
        assert!(left < 0.);
    }

    #[test]
    fn the_consoles_motion_names_parse() {
        assert_eq!(Gesture::parse("float"), Some(Gesture::Float));
        assert_eq!(Gesture::parse("wiggle"), None);
    }

    #[test]
    fn a_running_gesture_belongs_to_its_key() {
        let r = Running::new("zorro", Gesture::Tilt);
        assert!(r.pose_of(&"zorro").is_some());
        assert!(r.pose_of(&"gato").is_none());
    }
}
