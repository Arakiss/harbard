//! A robot's face: the console's `face.tsx` (a 100×64 SVG of light) and its
//! keyframes in `robot.css`, painted natively into the model's screen box.
//! Every state maps to something harbard observes.

use super::activity::Face;
use super::draw::{Affine, EASE, EASE_IN_OUT, LINEAR, Pen, phase, quad, track};
use gpui::{Bounds, Hsla, Pixels, Window};

/// One frame of a face: what to draw and where each loop stands.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FaceFrame {
    pub face: Face,
    pub sleepy: bool,
    /// 40 points and below: bolder eyes, no thin marks, no glow.
    pub tiny: bool,
    pub glow: Hsla,
    pub off: Hsla,
    /// The blink's eye height factor (1 open).
    pub blink: f32,
    /// The loops' clock in ms; `None` draws every loop at rest (Reduce motion).
    pub t: Option<f64>,
    /// The face group's offset and scale (the blocked shake, glance, spark).
    pub dx: f32,
    pub scale: f32,
    /// The glass's opacity (the boot flicker).
    pub power: f32,
    /// The hover tilt in degrees (negative leans left) about `pivot`, in
    /// window pixels: the body's centre, so the face turns with it.
    pub tilt: f32,
    pub pivot: (f32, f32),
}

impl FaceFrame {
    pub fn resting(face: Face, glow: Hsla, off: Hsla, tiny: bool) -> FaceFrame {
        FaceFrame {
            face,
            sleepy: false,
            tiny,
            glow,
            off,
            blink: 1.0,
            t: None,
            dx: 0.0,
            scale: 1.0,
            power: 1.0,
            tilt: 0.0,
            pivot: (0.0, 0.0),
        }
    }
}

/// Whether a face moves between blinks at this size (its loops).
pub fn loops(face: Face, tiny: bool) -> bool {
    match face {
        Face::Thinking | Face::Reading | Face::Delegating | Face::Verified => true,
        Face::Running | Face::Asleep => !tiny,
        _ => false,
    }
}

/// Whether a face has a blinking pair of eyes.
pub fn blinks(face: Face) -> bool {
    matches!(
        face,
        Face::Normal
            | Face::Thinking
            | Face::Running
            | Face::Reading
            | Face::Web
            | Face::Delegating
            | Face::Waiting
    )
}

/// Paint `frame` into `screen` (the model's screen box, in window pixels).
pub fn paint(frame: &FaceFrame, screen: Bounds<Pixels>, window: &mut Window) {
    if frame.power <= 0.0 {
        return;
    }
    let w = f32::from(screen.size.width);
    let h = f32::from(screen.size.height);
    // viewBox 0 0 100 64, `xMidYMid meet`.
    let k = (w / 100.0).min(h / 64.0);
    let ox = f32::from(screen.origin.x) + (w - 100.0 * k) / 2.0;
    let oy = f32::from(screen.origin.y) + (h - 64.0 * k) / 2.0;
    let base = Affine::rotate_at(frame.tilt, frame.pivot.0, frame.pivot.1)
        .then(Affine::translate(ox, oy))
        .then(Affine::scale(k, k));
    let group = base
        .then(Affine::translate(frame.dx, 0.0))
        .then(Affine::scale_at(frame.scale, frame.scale, 50.0, 36.0));
    let g = frame.glow.opacity(frame.power);
    let glow = (!frame.tiny).then_some(2.4);
    let t = frame.t;
    let at = |period: f64, delay: f64| t.map(|t| phase(t, period, delay));

    // The pair of eyes, with the blink and any look applied.
    let eyes = |window: &mut Window, look: Affine, y: f32, h: f32, r: f32, xs: [f32; 2]| {
        let blink = Affine::scale_at(1.0, frame.blink, 50.0, y + h / 2.0);
        let m = group.then(look).then(blink);
        for x in xs {
            let eye = if frame.tiny {
                m.then(Affine::scale_at(1.25, 1.25, x, y + h / 2.0))
            } else {
                m
            };
            Pen { m: eye }.round_rect(window, x - 7.0, y, 14.0, h, r, g, glow);
        }
    };
    let round_eyes = |window: &mut Window, look: Affine| {
        let h = if frame.sleepy { 9.0 } else { 18.0 };
        eyes(window, look, 22.0, h, 7.0, [34.0, 66.0]);
    };
    let pen = Pen { m: group };
    let thin = |window: &mut Window, pts: &[(f32, f32)]| {
        if !frame.tiny {
            pen.glow_stroke(window, pts, 3.4, g, true);
        }
    };
    let bold =
        |window: &mut Window, pts: &[(f32, f32)]| pen.glow_stroke(window, pts, 5.0, g, !frame.tiny);
    let arcs = |window: &mut Window, up: bool| {
        for x in [34.0, 66.0] {
            let pts = if up {
                quad((x - 8.0, 34.0), (x, 24.0), (x + 8.0, 34.0), 12)
            } else {
                quad((x - 8.0, 30.0), (x, 38.0), (x + 8.0, 30.0), 12)
            };
            bold(window, &pts);
        }
    };

    match frame.face {
        Face::Normal => {
            round_eyes(window, Affine::ID);
            thin(window, &quad((45.0, 47.0), (50.0, 51.0), (55.0, 47.0), 10));
        }
        Face::Thinking => {
            // rb-ponder: 3.4 s, (4,-4) → (1,-5) → (4,-4).
            let look = match at(3400.0, 0.0) {
                Some(p) => {
                    let x = track(p, &[(0.0, 4.0), (0.5, 1.0), (1.0, 4.0)], EASE_IN_OUT);
                    let y = track(p, &[(0.0, -4.0), (0.5, -5.0), (1.0, -4.0)], EASE_IN_OUT);
                    Affine::translate(x, y)
                }
                None => Affine::ID,
            };
            round_eyes(window, look);
            if !frame.tiny {
                for (i, x) in [40.0, 50.0, 60.0].into_iter().enumerate() {
                    // rb-dot: 1.3 s, each 0.18 s behind the one before.
                    let (op, dy) = match at(1300.0, 180.0 * i as f64) {
                        Some(p) => (
                            track(
                                p,
                                &[(0.0, 0.25), (0.3, 1.0), (0.6, 0.25), (1.0, 0.25)],
                                EASE_IN_OUT,
                            ),
                            track(
                                p,
                                &[(0.0, 0.0), (0.3, -3.0), (0.6, 0.0), (1.0, 0.0)],
                                EASE_IN_OUT,
                            ),
                        ),
                        None => (1.0, 0.0),
                    };
                    pen.dot(window, x, 52.0 + dy, 2.6, g.opacity(op), glow);
                }
            }
        }
        Face::Running => {
            eyes(window, Affine::ID, 30.0, 11.0, 5.5, [34.0, 66.0]);
            // rb-caret: 1 s, on for the first half.
            let on = at(1000.0, 0.0).is_none_or(|p| p < 0.5);
            if !frame.tiny && on {
                pen.round_rect(window, 46.0, 49.0, 8.0, 3.0, 1.5, g, glow);
            }
        }
        Face::Reading => {
            round_eyes(window, scan(at(2000.0, 0.0)));
            thin(window, &[(44.0, 48.0), (56.0, 48.0)]);
        }
        Face::Web => {
            round_eyes(window, Affine::translate(0.0, -3.0));
            let m = if frame.tiny {
                group.then(Affine::scale_at(1.25, 1.25, 50.0, 50.0))
            } else {
                group
            };
            Pen { m }.dot(window, 50.0, 50.0, 2.2, g, glow);
        }
        Face::Delegating => {
            round_eyes(window, scan(at(3600.0, 0.0)));
            if !frame.tiny {
                // rb-orb: a full turn in 2.4 s about (50, 52).
                let turn = at(2400.0, 0.0).map_or(0.0, |p| 360.0 * LINEAR.at(p));
                let orbit = Pen {
                    m: group.then(Affine::rotate_at(turn, 50.0, 52.0)),
                };
                orbit.dot(window, 50.0, 44.0, 2.2, g, glow);
                orbit.dot(window, 50.0, 60.0, 2.2, g, glow);
            }
        }
        Face::Waiting => {
            round_eyes(window, Affine::translate(0.0, -4.0));
            thin(window, &[(44.0, 49.0), (56.0, 49.0)]);
        }
        Face::Asleep => {
            arcs(window, false);
            if !frame.tiny {
                for (i, (x, y)) in [(76.0, 20.0), (84.0, 12.0)].into_iter().enumerate() {
                    // rb-zz: 2.8 s, the second 1.4 s behind; fades in, drifts up and out.
                    let (op, dx, dy) = match at(2800.0, 1400.0 * i as f64) {
                        Some(p) => (
                            track(p, &[(0.0, 0.0), (0.3, 1.0), (1.0, 0.0)], EASE_IN_OUT),
                            5.0 * EASE_IN_OUT.at(p),
                            -7.0 * EASE_IN_OUT.at(p),
                        ),
                        None => (1.0, 0.0, 0.0),
                    };
                    letter_z(&pen, window, x + dx, y + dy, 12.0, g.opacity(op));
                }
            }
        }
        Face::Blocked => {
            if frame.tiny {
                // The tiny rule's scale replaces the tilt, as in CSS.
                eyes(window, Affine::ID, 24.0, 16.0, 7.0, [34.0, 66.0]);
            } else {
                for (x, deg) in [(34.0, -10.0), (66.0, 10.0)] {
                    Pen {
                        m: group.then(Affine::rotate_at(deg, x, 32.0)),
                    }
                    .round_rect(
                        window,
                        x - 7.0,
                        24.0,
                        14.0,
                        16.0,
                        7.0,
                        g,
                        None,
                    );
                }
            }
            let mut wave = quad((43.0, 50.0), (46.5, 47.0), (50.0, 50.0), 8);
            wave.extend(
                quad((50.0, 50.0), (53.5, 53.0), (57.0, 50.0), 8)
                    .into_iter()
                    .skip(1),
            );
            thin(window, &wave);
            if !frame.tiny {
                question(&pen, window, 80.0, 22.0, g);
            }
        }
        Face::Finished => {
            arcs(window, true);
            bold(window, &quad((42.0, 45.0), (50.0, 53.0), (58.0, 45.0), 12));
        }
        Face::Verified => {
            arcs(window, true);
            // rb-chk: the tick draws itself in, holds, and starts again (2.8 s).
            let check = [(41.0, 47.0), (47.0, 53.0), (59.0, 41.0)];
            let shown = match at(2800.0, 0.0) {
                Some(p) => {
                    26.0 - track(
                        p,
                        &[(0.0, 26.0), (0.1, 26.0), (0.35, 0.0), (1.0, 0.0)],
                        EASE_IN_OUT,
                    )
                }
                None => 26.0,
            };
            if shown > 0.3 {
                bold(window, &super::draw::partial(&check, shown));
            }
        }
        Face::Disconnected => {
            pen.circle(window, 50.0, 32.0, 2.0, frame.off.opacity(frame.power));
        }
    }
}

/// rb-scan: side to side, ±6 units.
fn scan(p: Option<f32>) -> Affine {
    match p {
        Some(p) => Affine::translate(
            track(p, &[(0.0, -6.0), (0.5, 6.0), (1.0, -6.0)], EASE_IN_OUT),
            0.0,
        ),
        None => Affine::ID,
    }
}

/// A bold "z" of `em` units with its baseline at `y`.
pub fn letter_z(pen: &Pen, window: &mut Window, x: f32, y: f32, em: f32, color: Hsla) {
    let k = em / 12.0;
    let top = y - 6.2 * k;
    let pts = [
        (x + 0.7 * k, top),
        (x + 5.7 * k, top),
        (x + 0.7 * k, y - 0.4 * k),
        (x + 5.9 * k, y - 0.4 * k),
    ];
    pen.stroke(window, &pts, 1.8 * k, color, false);
}

/// A bold "?" of 16 units with its baseline at `y`.
fn question(pen: &Pen, window: &mut Window, x: f32, y: f32, color: Hsla) {
    let mut hook = quad(
        (x + 0.9, y - 8.6),
        (x + 1.2, y - 11.6),
        (x + 4.0, y - 11.6),
        6,
    );
    hook.extend(
        quad(
            (x + 4.0, y - 11.6),
            (x + 7.0, y - 11.6),
            (x + 7.0, y - 8.8),
            6,
        )
        .into_iter()
        .skip(1),
    );
    hook.extend(
        quad(
            (x + 7.0, y - 8.8),
            (x + 7.0, y - 7.0),
            (x + 4.6, y - 6.0),
            6,
        )
        .into_iter()
        .skip(1),
    );
    hook.push((x + 4.0, y - 4.0));
    pen.stroke(window, &hook, 2.0, color, true);
    pen.circle(window, x + 4.0, y - 1.0, 1.25, color);
}

/// How the blink closes and opens over its 364 ms (`rb-blink`, `ease`).
pub fn blink_at(ms: f64) -> f32 {
    if !(0.0..364.0).contains(&ms) {
        return 1.0;
    }
    track(
        (ms / 364.0) as f32,
        &[(0.0, 1.0), (0.43, 0.1), (1.0, 1.0)],
        EASE,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_faces_drop_the_marks_that_move_and_keep_the_eyes_that_do() {
        assert!(
            loops(Face::Thinking, true)
                && loops(Face::Reading, true)
                && loops(Face::Delegating, true)
        );
        assert!(!loops(Face::Running, true) && loops(Face::Running, false));
        assert!(!loops(Face::Asleep, true) && loops(Face::Asleep, false));
        assert!(
            !loops(Face::Normal, false)
                && !loops(Face::Finished, false)
                && !loops(Face::Disconnected, false)
        );
        assert!(blinks(Face::Normal) && !blinks(Face::Asleep) && !blinks(Face::Blocked));
    }

    #[test]
    fn a_blink_closes_to_a_tenth_and_opens_again() {
        assert_eq!(blink_at(-1.0), 1.0);
        assert!((blink_at(0.43 * 364.0) - 0.1).abs() < 1e-3);
        assert_eq!(blink_at(400.0), 1.0);
        assert!(blink_at(125.0) < 0.5 && blink_at(250.0) > blink_at(125.0));
    }
}
