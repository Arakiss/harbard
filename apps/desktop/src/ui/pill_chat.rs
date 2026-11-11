//! The console's pills and chips for the message stream and the screens
//! that list work (Deliveries, History, Retro). These are character
//! surfaces (`DESIGN.md` §3.1): sizes, rings and shadows come from the
//! console's `workspace.css` at `f6a79ed`, not from the dense tokens of §2.

use crate::theme::Theme;
use crate::ui::{self, icon_size, radius, size, space, weight};
use gpui::{
    AnyElement, BoxShadow, Div, Hsla, IntoElement, ParentElement, SharedString, Styled, div, hsla,
    point, px,
};

/// `.hb-chip`: 22 px high, 8 px inline, radius 6, 13/700.
const CHIP_H: f32 = 22.;
/// `.hb-pill`: 24 px high, fully round at 12.
const PILL_H: f32 = 24.;
/// `.hb-day span`: 2 px by 14 px, radius 14.
const DAY_PAD: (f32, f32) = (2., 14.);
/// `.hb-ship-chip`: 1 px by 8 px.
const SHIP_PAD_Y: f32 = 1.;

/// The colours of a message's state chip: the wash, the ink and, for the
/// two states that carry a verdict (red, green), the 1 px inset ring.
pub fn chip_colors(tone: &str, theme: &Theme) -> (Hsla, Hsla, Option<Hsla>) {
    match chip_tone(tone) {
        ChipTone::Green => (
            theme.c("green-bg"),
            theme.c("green-ink"),
            Some(theme.c("green-line")),
        ),
        ChipTone::Red => (
            theme.c("red-bg"),
            theme.c("red-ink"),
            Some(theme.c("red-line")),
        ),
        ChipTone::Amber => (theme.c("amber-bg"), theme.c("amber-ink"), None),
        ChipTone::Neutral => (theme.c("chip"), theme.c("chip-ink"), None),
    }
}

/// The four looks a chip can have.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChipTone {
    Green,
    Red,
    Amber,
    Neutral,
}

/// A tone name (`green`, `red`, `amber`, anything else) as a chip look.
pub fn chip_tone(tone: &str) -> ChipTone {
    match tone {
        "green" => ChipTone::Green,
        "red" => ChipTone::Red,
        "amber" => ChipTone::Amber,
        _ => ChipTone::Neutral,
    }
}

/// A 1 px ring drawn inside the edge (`box-shadow: inset 0 0 0 1px`).
pub fn inset_ring(color: Hsla) -> BoxShadow {
    BoxShadow {
        color,
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(1.),
        inset: true,
    }
}

/// `.hb-chip`: a message's type (Starting, Finding, Delivered, Blocked).
/// Delivered and Blocked wear an inset ring of their state's line.
pub fn state_chip(label: impl Into<SharedString>, tone: &str, theme: &Theme) -> Div {
    let (bg, ink, ring) = chip_colors(tone, theme);
    let chip = div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(CHIP_H))
        .px(px(space::MD))
        .rounded(px(radius::BASE))
        .bg(bg)
        .whitespace_nowrap()
        .child(ui::text(label, size::SMALL, weight::STRONG, ink));
    match ring {
        Some(line) => chip.shadow(vec![inset_ring(line)]),
        None => chip,
    }
}

/// `.hb-pill`: the round "verified" pill, its icon and label in `ok-ink`
/// on `ok-bg` with an `ok-line` ring.
pub fn ok_pill(icon: &str, label: impl Into<SharedString>, theme: &Theme) -> Div {
    let ink = theme.c("ok-ink");
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(space::XS))
        .h(px(PILL_H))
        .px(px(space::MD))
        .rounded_full()
        .bg(theme.c("ok-bg"))
        .shadow(vec![inset_ring(theme.c("ok-line"))])
        .child(ui::icon(icon, icon_size::INLINE, ink))
        .child(ui::text(label, size::META, weight::STRONG, ink))
}

/// `.hb-day`: the day's name in a round pill on a hairline that crosses the
/// pane, `margin: 8px 0 4px`.
pub fn day_pill(label: impl Into<SharedString>, theme: &Theme) -> AnyElement {
    div()
        .w_full()
        .relative()
        .flex()
        .justify_center()
        .mt(px(space::MD))
        .mb(px(space::XS))
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(gpui::relative(0.5))
                .h(px(1.))
                .bg(theme.c("line")),
        )
        .child(
            div()
                .py(px(DAY_PAD.0))
                .px(px(DAY_PAD.1))
                .rounded(px(DAY_PAD.1))
                .border_1()
                .border_color(theme.c("line"))
                .bg(theme.c("surface"))
                .child(ui::text(
                    label,
                    size::SMALL,
                    weight::STRONG,
                    theme.c("ink-2"),
                )),
        )
        .into_any_element()
}

/// `--hb-tools-shadow`: the hover toolbar's small drop, darker in dark mode.
pub fn tools_shadow(theme: &Theme) -> Vec<BoxShadow> {
    let alpha = if theme.dark { 0.4 } else { 0.08 };
    vec![BoxShadow {
        color: hsla(0., 0., 0., alpha),
        offset: point(px(0.), px(1.)),
        blur_radius: px(3.),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// The wash and ink of a delivery's state chip (`.hb-ship-chip`): in review
/// amber, with fixes red, integrated the accent's soft wash, installed green,
/// delivered the neutral chip.
pub fn ship_colors(state: &str, theme: &Theme) -> (Hsla, Hsla) {
    match state {
        "in_review" => (theme.c("amber-bg"), theme.c("amber-ink")),
        "with_fixes" => (theme.c("red-bg"), theme.c("red-ink")),
        "integrated" => (theme.c("accent-soft"), theme.c("accent-text")),
        "installed" => (theme.c("green-bg"), theme.c("green-ink")),
        _ => (theme.c("chip"), theme.c("chip-ink")),
    }
}

/// `.hb-ship-chip`: where a delivery is, coloured by its state.
pub fn ship_chip(label: impl Into<SharedString>, state: &str, theme: &Theme) -> Div {
    let (bg, ink) = ship_colors(state, theme);
    div()
        .flex_none()
        .py(px(SHIP_PAD_Y))
        .px(px(space::MD))
        .rounded(px(radius::SM))
        .bg(bg)
        .whitespace_nowrap()
        .child(ui::text(label, size::META, weight::STRONG, ink))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_verdicts_wear_a_ring() {
        assert_eq!(chip_tone("green"), ChipTone::Green);
        assert_eq!(chip_tone("red"), ChipTone::Red);
        assert_eq!(chip_tone("amber"), ChipTone::Amber);
        assert_eq!(chip_tone("lead"), ChipTone::Neutral);
        assert_eq!(chip_tone("neutral"), ChipTone::Neutral);
    }
}
