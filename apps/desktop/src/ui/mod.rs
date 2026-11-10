//! Small shared pieces every screen uses: icons, type sizes, badges, rows.
//!
//! The console's type scale (`text-{micro,meta,small,ui,nav,body,title,display}`)
//! and its 15px/1.46 Lato body are the defaults here.

pub mod avatar;
pub mod format;

use crate::theme::Theme;
use gpui::{
    AnyElement, App, Div, FontWeight, Hsla, IntoElement, ParentElement, Pixels, SharedString, Styled, Svg, div, px,
    svg,
};

/// The workspace face.
pub const SANS: &str = "Lato";
/// Code, SHAs, paths.
pub const MONO: &str = "IBM Plex Mono";
/// Headings outside the workspace (Setup, Strip).
pub const DISPLAY: &str = "Schibsted Grotesk";

/// The console's type scale, in pixels.
pub mod size {
    pub const MICRO: f32 = 11.0;
    pub const META: f32 = 12.0;
    pub const SMALL: f32 = 13.0;
    pub const UI: f32 = 14.0;
    pub const BODY: f32 = 15.0;
    pub const NAV: f32 = 15.0;
    pub const TITLE: f32 = 18.0;
    pub const DISPLAY: f32 = 28.0;
}

/// A Lucide icon by its kebab-case name (`message-square`), tinted.
pub fn icon(name: &str, size: f32, color: Hsla) -> Svg {
    svg()
        .path(SharedString::from(format!("icons/{name}.svg")))
        .size(px(size))
        .flex_none()
        .text_color(color)
}

/// Text in the workspace face at a size and weight.
pub fn text(s: impl Into<SharedString>, size: f32, weight: FontWeight, color: Hsla) -> Div {
    div()
        .font_family(SANS)
        .text_size(px(size))
        .font_weight(weight)
        .text_color(color)
        .child(s.into())
}

/// A counter pill (`.hb-row-count`, `.hb-rail-badge`): "99+" above 99.
pub fn count_badge(n: u64, bg: Hsla, fg: Hsla) -> Div {
    let label = if n > 99 { "99+".to_string() } else { n.to_string() };
    div()
        .flex()
        .items_center()
        .justify_center()
        .min_w(px(18.))
        .h(px(18.))
        .px(px(5.))
        .rounded(px(9.))
        .bg(bg)
        .text_color(fg)
        .font_family(SANS)
        .text_size(px(11.))
        .font_weight(FontWeight::BLACK)
        .child(label)
}

/// A presence dot with its 1px edge.
pub fn dot(color: Hsla, edge: Hsla, size: Pixels) -> Div {
    div().size(size).rounded_full().bg(color).border_1().border_color(edge).flex_none()
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

/// An empty-state line centred in a pane.
pub fn empty(message: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .p(px(24.))
        .font_family(SANS)
        .text_size(px(size::UI))
        .text_color(theme.c("ink-3"))
        .child(message.into())
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
