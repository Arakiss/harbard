//! The Lobby's character pieces (`DESIGN.md` §3.1), ported from the console's
//! `rooms.css`: the presence pill on a card's band, the harness and project
//! chips, and the speech bubble with its tail towards the robot.

use super::{MONO, SANS, leading, space, weight};
use crate::theme::Theme;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    BoxShadow, Div, ParentElement, PathBuilder, SharedString, Styled, canvas, div, point, px,
};

/// `.hb-lobby-presence`: 24 px high, fully round.
const PILL_H: f32 = 24.;
/// `.hb-lobby-presence-dot`.
const PILL_DOT: f32 = 9.;
/// `.hb-lobby-tag`: 20 px high, 5 px corners.
const CHIP_H: f32 = 20.;
const CHIP_RADIUS: f32 = 5.;
/// `.hb-lobby-bubble`: 12 px corners.
pub const BUBBLE_RADIUS: f32 = 12.;
/// The tail: a 14 px square turned 45°, 44 px in from the bubble's left and
/// 6 px above it (`.hb-lobby-bubble::before`).
const TAIL_SIDE: f32 = 14.;
const TAIL_LEFT: f32 = 44.;
const TAIL_TOP: f32 = -6.;

/// How a presence dot is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PillDot {
    /// Filled with this tone (`green`, `amber`, `grey`).
    Filled(&'static str),
    /// A 2 px ring in grey: gone.
    Ring,
}

/// The pill on a card's band: a dot and the presence word, 12/700 on the
/// raised surface.
pub fn presence_pill(word: impl Into<SharedString>, dot: PillDot, theme: &Theme) -> Div {
    let mark = div().size(px(PILL_DOT)).flex_none().rounded_full();
    let mark = match dot {
        PillDot::Filled(tone) => mark.bg(theme.c(tone)),
        PillDot::Ring => mark.shadow(vec![BoxShadow {
            color: theme.c("grey"),
            offset: point(px(0.), px(0.)),
            blur_radius: px(0.),
            spread_radius: px(space::HAIR),
            inset: true,
        }]),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(space::SM))
        .h(px(PILL_H))
        .pl(px(8.))
        .pr(px(9.))
        .rounded_full()
        .bg(theme.c("surface-raised"))
        .font_family(SANS)
        .text_size(px(12.))
        .line_height(px(leading::META))
        .font_weight(weight::STRONG)
        .text_color(theme.c("ink-2"))
        .whitespace_nowrap()
        .child(mark)
        .child(word.into())
}

/// A harness or project chip: 20 px, 12/700 on `chip`, mono for a project.
pub fn tag_chip(label: impl Into<SharedString>, mono: bool, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(CHIP_H))
        .max_w_full()
        .px(px(space::SM))
        .rounded(px(CHIP_RADIUS))
        .bg(theme.c("chip"))
        .text_color(theme.c("chip-ink"))
        .font_family(if mono { MONO } else { SANS })
        .text_size(px(12.))
        .line_height(px(leading::META))
        .font_weight(weight::STRONG)
        .overflow_hidden()
        .whitespace_nowrap()
        .text_ellipsis()
        .child(label.into())
}

/// The speech bubble around what an agent said, its tail pointing up at the
/// robot. `silent` is the outlined box for one that has said nothing: no
/// fill, no tail, `line-strong` edge, `ink-3` text.
pub fn bubble(silent: bool, theme: &Theme) -> Div {
    let fill = theme.c("chip");
    div()
        .relative()
        .px(px(space::LG))
        .py(px(10.))
        .rounded(px(BUBBLE_RADIUS))
        .when(!silent, |d| {
            d.bg(fill).child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        let (top, left) = (bounds.origin.y, bounds.origin.x);
                        let (tip, base) = tail_points();
                        let mut b = PathBuilder::fill();
                        b.move_to(point(left + px(tip.0), top + px(tip.1)));
                        b.line_to(point(left + px(base.0), top + px(base.1)));
                        b.line_to(point(left + px(2. * tip.0 - base.0), top + px(base.1)));
                        b.close();
                        if let Ok(path) = b.build() {
                            window.paint_path(path, fill);
                        }
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size(px(1.)),
            )
        })
        .when(silent, |d| {
            d.border_1()
                .border_color(theme.c("line-strong"))
                .text_color(theme.c("ink-3"))
        })
}

/// The tail as a triangle, relative to the bubble's top-left corner: its
/// tip, and the right end of its base (the left end mirrors it). The turned
/// square's upper half; the lower half sits inside the bubble's own fill.
/// The 3 px corner of the square blunts the tip by `3·(√2 − 1)`.
pub fn tail_points() -> ((f32, f32), (f32, f32)) {
    let half = TAIL_SIDE / 2.;
    let diag = half * std::f32::consts::SQRT_2;
    let (cx, cy) = (TAIL_LEFT + half, TAIL_TOP + half);
    let blunt = 3. * (std::f32::consts::SQRT_2 - 1.);
    ((cx, cy - diag + blunt), (cx + diag, cy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tail_points_up_from_inside_the_bubble() {
        let ((tx, ty), (bx, by)) = tail_points();
        // Centred 51 px in, as the console's square is (44 + 14 / 2).
        assert!((tx - 51.).abs() < 1e-4);
        // The tip stands above the bubble's edge; the base hides inside it.
        assert!(ty < 0. && ty > -9.);
        assert!(by > 0.);
        assert!((bx - tx - 14. / 2. * std::f32::consts::SQRT_2).abs() < 1e-4);
    }
}
