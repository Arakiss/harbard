//! Pieces for the character surfaces the people lane draws (DESIGN.md §3.1):
//! the DM header and list, Activity, the calm screen, Setup and the away
//! card. On these surfaces the console at `f6a79ed` is the reference, so the
//! sizes here are its sizes (`workspace.css`), not the dense steps of §2.
//!
//! Colour still comes from theme tokens only (C1), and motion still obeys
//! reduced motion (M7): [`entrance`] is the one curve the staged entrances
//! read, and callers skip it when motion is reduced.

use crate::theme::Theme;
use crate::ui::{self, space, weight};
use gpui::{Div, FontWeight, ParentElement, SharedString, Styled, div, px};
use std::time::Duration;

/// Robot sizes per surface (§3.1 "a size tier per surface").
pub mod robot_px {
    /// The DM header.
    pub const HEADER: f32 = 64.0;
    /// Rows of the DM list and Activity; the away card's waiting rows.
    pub const ROW: f32 = 48.0;
    /// The away card's "still running" rows.
    pub const SMALL: f32 = 32.0;
    /// The calm screen's one item.
    pub const CALM: f32 = 120.0;
}

/// Text sizes from the console's stylesheet, by what they set.
pub mod type_px {
    /// `.hb-list-title`: the title of the DM and Activity list panes.
    pub const LIST_TITLE: f32 = 24.0;
    /// `.hb-dm-name`: the agent's name in the DM header.
    pub const DM_NAME: f32 = 22.0;
    /// `.hb-dm-presence`: what the agent is, under its name.
    pub const DM_RELATION: f32 = 16.0;
    /// `.hb-dm-presence .hb-status`: how it is, under that.
    pub const DM_PRESENCE: f32 = 15.0;
    /// `.hb-item-name`: a row's name in the DM list and Activity.
    pub const ITEM_NAME: f32 = 16.0;
    /// `.hb-unread`, `.hb-tab`, a card's reason line.
    pub const PILL: f32 = 15.0;
    /// `.hb-count`.
    pub const COUNT: f32 = 13.0;
    /// `.hb-setup-brand` and `.hb-calm-card h1`.
    pub const HERO: f32 = 34.0;
    /// `.hb-calm-question`.
    pub const QUESTION: f32 = 26.0;
    /// `.hb-calm-actions .hb-btn`, `.hb-away-head h2`, `.hb-setup-title`.
    pub const LARGE: f32 = 24.0;
    /// `.hb-calm-detail`.
    pub const DETAIL: f32 = 19.0;
    /// `.hb-calm-chip`, `.hb-calm-pill`, `.hb-away-section h3`.
    pub const CHIP: f32 = 18.0;
    /// `.hb-setup-lead`, `.hb-away-foot .hb-btn`.
    pub const LEAD: f32 = 17.0;
    /// `.hb-setup-go`, `.hb-away-who`.
    pub const ACTION: f32 = 16.0;
}

/// The staged entrances (`setup-*-in`, `.hb-setup-row`): 200 ms for a
/// step, 180 ms for a row that slides in.
pub const ENTER: Duration = Duration::from_millis(200);
pub const ROW_ENTER: Duration = Duration::from_millis(180);

/// Text at a console size, with the console's line height (`1.25` above
/// 18 px, the scale's own below).
pub fn text_px(
    s: impl Into<SharedString>,
    size: f32,
    weight: FontWeight,
    theme_ink: gpui::Hsla,
) -> Div {
    ui::text(s, size, weight, theme_ink).line_height(px(line_for(size)))
}

/// The line height the console gives a size: tight for headings, the
/// scale's own for text.
pub fn line_for(size: f32) -> f32 {
    if size > ui::size::TITLE {
        (size * 1.25).round()
    } else {
        ui::leading::for_size(size).max((size * 1.4).round())
    }
}

/// `.hb-unread`: the outlined accent pill in a list header ("2 unread",
/// "Needs you: 2"). 34 px, a 2 px accent edge, 15/900 in `accent-text`.
pub fn accent_pill(label: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .h(px(34.))
        .px(px(14.))
        .rounded_full()
        .border_2()
        .border_color(theme.c("accent"))
        .child(
            text_px(label, type_px::PILL, weight::HEAVY, theme.c("accent-text"))
                .whitespace_nowrap(),
        )
}

/// `.hb-count`: a row's unread count, 22 px, `accent-deep`, 13/900.
pub fn count_pill(n: u64, theme: &Theme) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .justify_center()
        .min_w(px(26.))
        .h(px(22.))
        .px(px(space::MD))
        .rounded_full()
        .bg(theme.c("accent-deep"))
        .child(text_px(
            count_label(n),
            type_px::COUNT,
            weight::HEAVY,
            theme.c("accent-ink"),
        ))
}

/// A tab's count as quiet meta text: a tally, not an alarm, so it stays out
/// of the accent (C2, C3).
pub fn quiet_count(n: u64, theme: &Theme) -> Div {
    div().flex_none().child(text_px(
        count_label(n),
        type_px::COUNT,
        weight::STRONG,
        theme.c("ink-3"),
    ))
}

/// What a count says: the number, or "99+" above 99.
pub fn count_label(n: u64) -> String {
    if n > 99 {
        "99+".to_string()
    } else {
        n.to_string()
    }
}

/// `.hb-list-head`: 64 px, the title at 24/900, no hairline (the list
/// under it draws its own). Put what goes right of it with `.child(…)`.
pub fn list_head(title: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .flex_none()
        .flex()
        .items_center()
        .gap(px(space::MD))
        .h(px(64.))
        .pl(px(space::GUTTER))
        .pr(px(space::XL))
        .child(
            text_px(title, type_px::LIST_TITLE, weight::HEAVY, theme.c("ink"))
                .min_w_0()
                .truncate(),
        )
}

/// A soft pill in a state's wash (`.hb-calm-working`, `.hb-away-pill`):
/// `tone` is `green`, `amber` or `red`, used only when the thing is in it.
pub fn tone_pill(label: impl Into<SharedString>, tone: &str, size: f32, theme: &Theme) -> Div {
    div()
        .flex_none()
        .px(px(10.))
        .py(px(space::HAIR))
        .rounded(px(ui::radius::MD))
        .bg(theme.c(&format!("{tone}-bg")))
        .child(text_px(
            label,
            size,
            weight::STRONG,
            theme.c(&format!("{tone}-ink")),
        ))
}

/// Where a staged element is at `t` (0..1 of the whole entrance) when it
/// starts after `delay` and runs for `run`, both as shares of the whole:
/// 0 before its turn, then eased out to 1.
pub fn entrance(t: f32, delay: f32, run: f32) -> f32 {
    if run <= 0. {
        return 1.;
    }
    let u = ((t - delay) / run).clamp(0., 1.);
    // `cubic-bezier(.32,.72,0,1)` is close to an ease out of the fifth power.
    1. - (1. - u).powi(5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_count_above_ninety_nine_is_said_as_ninety_nine_plus() {
        assert_eq!(count_label(0), "0");
        assert_eq!(count_label(99), "99");
        assert_eq!(count_label(100), "99+");
    }

    #[test]
    fn a_staged_element_waits_for_its_turn_then_lands() {
        assert_eq!(entrance(0.0, 0.25, 0.5), 0.0);
        assert_eq!(entrance(0.25, 0.25, 0.5), 0.0);
        let mid = entrance(0.5, 0.25, 0.5);
        assert!(mid > 0.5 && mid < 1.0, "eases out: {mid}");
        assert_eq!(entrance(0.75, 0.25, 0.5), 1.0);
        assert_eq!(entrance(1.0, 0.25, 0.5), 1.0);
        assert_eq!(entrance(0.0, 0.0, 0.0), 1.0);
    }

    #[test]
    fn headings_set_tight_and_text_keeps_the_scale() {
        assert_eq!(line_for(34.), 43.);
        assert_eq!(line_for(24.), 30.);
        assert_eq!(line_for(14.), 20.);
        assert_eq!(line_for(16.), 24.);
    }
}
