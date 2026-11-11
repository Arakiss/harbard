//! Small shared pieces every screen uses: icons, the design tokens, badges,
//! rows. The tokens are `DESIGN.md` §2: a screen never writes a bare number
//! for a size, a weight, a gap, a radius or a duration; it uses these.

pub mod avatar;
pub mod format;
pub mod kit_theme;

use crate::theme::Theme;
use gpui::{
    AnyElement, App, BoxShadow, Div, FontWeight, Hsla, IntoElement, ParentElement, Pixels,
    SharedString, Styled, Svg, div, point, px, svg,
};

/// The workspace face.
pub const SANS: &str = "Lato";
/// Code, SHAs, paths.
pub const MONO: &str = "IBM Plex Mono";
/// Headings outside the workspace (Setup, Strip).
pub const DISPLAY: &str = "Schibsted Grotesk";

/// The type scale, in pixels (T1): these eight and no others.
pub mod size {
    /// Count badges, key chips.
    pub const MICRO: f32 = 11.0;
    /// Timestamps, section labels, captions, chips.
    pub const META: f32 = 12.0;
    /// Second lines, status lines, disclosures, hints.
    pub const SMALL: f32 = 13.0;
    /// Controls, sidebar rows, tabs, menu items, list-row titles.
    pub const UI: f32 = 14.0;
    /// Message text and anything read as prose.
    pub const BODY: f32 = 15.0;
    /// Sidebar rows.
    pub const NAV: f32 = 14.0;
    /// The one title of a pane or dialog.
    pub const TITLE: f32 = 18.0;
    /// Schibsted Grotesk headings outside the workspace.
    pub const DISPLAY: f32 = 24.0;
}

/// The line height for each size in [`size`].
pub mod leading {
    pub const MICRO: f32 = 14.0;
    pub const META: f32 = 16.0;
    pub const SMALL: f32 = 18.0;
    pub const UI: f32 = 20.0;
    pub const BODY: f32 = 22.0;
    pub const TITLE: f32 = 24.0;
    pub const DISPLAY: f32 = 30.0;

    /// The line height that goes with a text size.
    pub fn for_size(size: f32) -> f32 {
        match size {
            s if s <= super::size::MICRO => MICRO,
            s if s <= super::size::META => META,
            s if s <= super::size::SMALL => SMALL,
            s if s <= super::size::UI => UI,
            s if s <= super::size::BODY => BODY,
            s if s <= super::size::TITLE => TITLE,
            s if s <= super::size::DISPLAY => DISPLAY,
            s => (s * 1.25).round(),
        }
    }
}

/// The three weights Lato ships (T2). `HEAVY` is for the pane title and
/// author and agent names only.
pub mod weight {
    use gpui::FontWeight;
    pub const REGULAR: FontWeight = FontWeight::NORMAL;
    pub const STRONG: FontWeight = FontWeight::BOLD;
    pub const HEAVY: FontWeight = FontWeight::BLACK;
}

/// Spacing on a 4 px unit (S1): these steps and no others.
pub mod space {
    pub const HAIR: f32 = 2.0;
    pub const XS: f32 = 4.0;
    pub const SM: f32 = 6.0;
    pub const MD: f32 = 8.0;
    pub const LG: f32 = 12.0;
    pub const XL: f32 = 16.0;
    /// The left and right inset of every pane (S2).
    pub const GUTTER: f32 = 20.0;
    pub const SECTION: f32 = 24.0;
    pub const BREAK: f32 = 32.0;
}

/// Row heights (§2.3).
pub mod row {
    /// Sidebar destination and channel rows.
    pub const NAV: f32 = 28.0;
    /// Menu, popover and finder rows.
    pub const MENU: f32 = 28.0;
    /// Two-line agent row in the sidebar.
    pub const AGENT: f32 = 44.0;
    /// Pane and dialog header.
    pub const HEADER: f32 = 56.0;
    /// Tab strip under a header.
    pub const TABS: f32 = 40.0;
}

/// Control heights (Z1: nothing taller than `LG`).
pub mod control {
    /// Icon buttons, header buttons, sidebar filter fields.
    pub const SM: f32 = 28.0;
    /// Buttons, inputs, selects, search fields.
    pub const MD: f32 = 32.0;
    /// The single primary action of a dialog.
    pub const LG: f32 = 36.0;
}

/// Icon sizes (Z3): three, never two in one row.
pub mod icon_size {
    /// Beside `META` and `SMALL` text.
    pub const INLINE: f32 = 14.0;
    /// Rows, controls, headers.
    pub const ROW: f32 = 16.0;
    /// The rail and the composer's send.
    pub const RAIL: f32 = 20.0;
}

/// Avatar sizes (Z4): one per list.
pub mod avatar_size {
    /// Messages and list rows.
    pub const ROW: f32 = 36.0;
    /// Sidebar agent rows.
    pub const SIDEBAR: f32 = 28.0;
    /// Stacks.
    pub const STACK: f32 = 20.0;
}

/// Radii (R1): `rounded_full` is the fifth, for dots, badges and the rail.
pub mod radius {
    /// Chips, tags, key chips, inline code, small swatches.
    pub const SM: f32 = 4.0;
    /// Buttons, inputs, sidebar and menu rows, tabs.
    pub const BASE: f32 = 6.0;
    /// Avatars, banners, cards, popovers, the composer.
    pub const MD: f32 = 8.0;
    /// Dialogs and the main panel.
    pub const LG: f32 = 12.0;
}

/// Interface motion (§2.6), in milliseconds; all ease out (quint) but the
/// arrival wash, which is linear. Reduced motion turns `BASE`, `SLOW` and
/// `ARRIVAL` off.
pub mod motion {
    use std::time::Duration;
    /// Press feedback; a tooltip or hover toolbar appearing.
    pub const FAST: Duration = Duration::from_millis(100);
    /// A menu, popover or the finder appearing (opacity and 4 px of travel).
    pub const BASE: Duration = Duration::from_millis(160);
    /// A dialog and its scrim; a pane opening or closing.
    pub const SLOW: Duration = Duration::from_millis(240);
    /// The wash behind a row that just arrived, fading out.
    pub const ARRIVAL: Duration = Duration::from_millis(150);
}

/// A Lucide icon by its kebab-case name (`message-square`), tinted.
pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// Text in the workspace face at a size and weight; the line height
/// follows the size ([`leading::for_size`]).
pub fn text(s: impl Into<SharedString>, size: f32, weight: FontWeight, color: Hsla) -> Div {
    div()
        .font_family(SANS)
        .text_size(px(size))
        .line_height(px(leading::for_size(size)))
        .font_weight(weight)
        .text_color(color)
        .child(s.into())
}

/// A counter pill (`.hb-row-count`, `.hb-rail-badge`): "99+" above 99.
pub fn count_badge(n: u64, bg: Hsla, fg: Hsla) -> Div {
    let label = if n > 99 {
        "99+".to_string()
    } else {
        n.to_string()
    };
    div()
        .flex()
        .items_center()
        .justify_center()
        .min_w(px(16.))
        .h(px(16.))
        .px(px(space::XS))
        .rounded_full()
        .bg(bg)
        .text_color(fg)
        .font_family(SANS)
        .text_size(px(size::MICRO))
        .line_height(px(leading::MICRO))
        .font_weight(weight::STRONG)
        .child(label)
}

/// A key chip (K1): 20 px high, `MICRO` 11/700 in `ink-3`, 1 px `line`.
pub fn kbd(key: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(20.))
        .min_w(px(20.))
        .px(px(space::XS))
        .rounded(px(radius::SM))
        .border_1()
        .border_color(theme.c("line"))
        .font_family(SANS)
        .text_size(px(size::MICRO))
        .line_height(px(leading::MICRO))
        .font_weight(weight::STRONG)
        .text_color(theme.c("ink-3"))
        .child(key.into())
}

/// A row of key chips, 2 px apart (`⌘`, `K`).
pub fn kbd_keys<'a>(keys: impl IntoIterator<Item = &'a str>, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(space::HAIR))
        .children(keys.into_iter().map(|k| kbd(k.to_string(), theme)))
}

/// A tag or chip: `META` 12 on its wash, `RADIUS_SM`. `tone` is a state
/// (`green`, `amber`, `red`) only when the thing is in that state (C3);
/// anything else is neutral.
pub fn chip(label: impl Into<SharedString>, tone: &str, theme: &Theme) -> Div {
    let (bg, ink) = match tone {
        "green" | "amber" | "red" => (
            theme.c(&format!("{tone}-bg")),
            theme.c(&format!("{tone}-ink")),
        ),
        _ => (theme.c("control"), theme.c("ink-2")),
    };
    div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(20.))
        .px(px(space::SM))
        .rounded(px(radius::SM))
        .bg(bg)
        .font_family(SANS)
        .text_size(px(size::META))
        .line_height(px(leading::META))
        .font_weight(weight::STRONG)
        .text_color(ink)
        .child(label.into())
}

/// A section label (T5): sentence case, `META` 12/700 in `ink-3`.
pub fn section_label(label: impl Into<SharedString>, theme: &Theme) -> Div {
    text(label, size::META, weight::STRONG, theme.c("ink-3"))
}

/// A pane header (T4): 56 px, the title `TITLE` 18/900 at the gutter,
/// vertically centred; put actions after it with `.child(…)`.
pub fn pane_header(title: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(space::MD))
        .h(px(row::HEADER))
        .px(px(space::GUTTER))
        .border_b_1()
        .border_color(theme.c("line"))
        .child(text(title, size::TITLE, weight::HEAVY, theme.c("ink")).truncate())
}

/// The focus ring (F1): 2 px of `focus`, inset on rows, outside on controls.
pub fn focus_ring(theme: &Theme, inset: bool) -> BoxShadow {
    BoxShadow {
        color: theme.c("focus"),
        offset: point(px(0.), px(0.)),
        blur_radius: px(0.),
        spread_radius: px(2.),
        inset,
    }
}

/// A presence dot with its 1px edge.
pub fn dot(color: Hsla, edge: Hsla, size: Pixels) -> Div {
    div()
        .size(size)
        .rounded_full()
        .bg(color)
        .border_1()
        .border_color(edge)
        .flex_none()
}

/// The colour pair for a presence tone: green, amber, red or grey.
pub fn tone_colors(theme: &Theme, tone: &str) -> (Hsla, Hsla) {
    match tone {
        "green" => (theme.c("green"), theme.c("green-edge")),
        "amber" => (theme.c("amber"), theme.c("amber-edge")),
        "red" => (theme.c("red"), theme.c("red-edge")),
        _ => (theme.c("grey"), theme.c("grey-edge")),
    }
}

/// An empty state with one line (F3); see [`empty_state`] for the title and
/// sentence form.
pub fn empty(message: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .pt(px(space::BREAK))
        .px(px(space::GUTTER))
        .child(
            text(message, size::UI, weight::REGULAR, theme.c("ink-3"))
                .max_w(px(320.))
                .text_center(),
        )
}

/// The empty state (F3): one line of `UI` 14/700 in `ink-2` and one sentence
/// of `UI` 14/400 in `ink-3`, at most 320 px wide, `BREAK` below the header.
pub fn empty_state(
    title: impl Into<SharedString>,
    sentence: impl Into<SharedString>,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .items_center()
        .gap(px(space::XS))
        .pt(px(space::BREAK))
        .px(px(space::GUTTER))
        .child(
            text(title, size::UI, weight::STRONG, theme.c("ink-2"))
                .max_w(px(320.))
                .text_center(),
        )
        .child(
            text(sentence, size::UI, weight::REGULAR, theme.c("ink-3"))
                .max_w(px(320.))
                .text_center(),
        )
}

/// Wrap any element as `AnyElement` (for heterogeneous children lists).
pub fn any(e: impl IntoElement) -> AnyElement {
    e.into_any_element()
}

/// The current theme, the common first line of a render.
pub fn theme(cx: &App) -> &Theme {
    cx.global::<Theme>()
}
pub mod agent_card;
pub mod feedback;
pub mod pill_lobby;
pub mod pill_people;
