//! Pieces the secondary screens share (Lobby, Deliveries, History, Retro,
//! Setup, a deleted room): the console's `.hb-head`, `.hb-btn`, `.hb-chip`,
//! `.hb-empty` and `.hb-search`, a width probe for container breakpoints,
//! and a background read of serve.

use crate::api::{self, ApiError};
use crate::desk::Desk;
use crate::theme::Theme;
use crate::ui::{self, size};
use gpui::{
    AnyElement, App, AppContext, Bounds, Div, ElementId, EntityId, FontWeight, Hsla,
    InteractiveElement, IntoElement, ParentElement, Pixels, SharedString, Stateful, Styled, Task,
    canvas, div, px,
};
use serde::de::DeserializeOwned;
use std::cell::Cell;
use std::rc::Rc;

/// `.hb-main`: the screen's column on the surface.
pub fn main_column(theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .size_full()
        .bg(theme.c("surface"))
        .font_family(ui::SANS)
        .text_size(px(size::BODY))
        .line_height(px(size::BODY * 1.46))
        .text_color(theme.c("ink"))
}

/// `.hb-head`: icon and title, then the grey topic line.
pub fn head(icon: &str, title: SharedString, topic: Option<SharedString>, theme: &Theme) -> Div {
    head_bar(theme)
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(6.))
                .flex_none()
                .text_size(px(19.))
                .font_weight(FontWeight::BLACK)
                .whitespace_nowrap()
                .child(ui::icon(icon, 20., theme.c("ink")))
                .child(title),
        )
        .children(topic.map(|t| {
            div()
                .min_w_0()
                .text_size(px(size::SMALL))
                .text_color(theme.c("ink-4"))
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .child(t)
        }))
}

/// The 56px bar under every screen title.
pub fn head_bar(theme: &Theme) -> Div {
    div()
        .h(px(56.))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(10.))
        .pl(px(20.))
        .pr(px(16.))
        .border_b_1()
        .border_color(theme.c("line"))
}

/// `.hb-btn` (and `.hb-btn-primary`): 36px, 6px radius, bold.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    primary: bool,
    disabled: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let (bg, edge, ink, hover) = if primary {
        (
            theme.c("accent-deep"),
            theme.c("accent-deep"),
            theme.c("accent-ink"),
            theme.c("accent-hover"),
        )
    } else {
        (
            theme.c("surface"),
            theme.c("line-strong"),
            theme.c("ink"),
            theme.c("btn-hover"),
        )
    };
    let base = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(36.))
        .px(px(16.))
        .rounded(px(6.))
        .border_1()
        .border_color(edge)
        .bg(bg)
        .text_color(ink)
        .font_weight(FontWeight::BOLD)
        .text_size(px(size::BODY))
        .child(label.into());
    if disabled {
        base.opacity(0.5)
    } else {
        base.cursor_pointer().hover(move |s| s.bg(hover))
    }
}

/// `.hb-chip[data-tone]`: neutral, amber, red, green.
pub fn chip(text: impl Into<SharedString>, tone: &str, theme: &Theme) -> Div {
    let (bg, ink, ring): (Hsla, Hsla, Option<Hsla>) = match tone {
        "amber" => (theme.c("amber-bg"), theme.c("amber-ink"), None),
        "red" => (
            theme.c("red-bg"),
            theme.c("red-ink"),
            Some(theme.c("red-line")),
        ),
        "green" => (
            theme.c("green-bg"),
            theme.c("green-ink"),
            Some(theme.c("green-line")),
        ),
        _ => (theme.c("chip"), theme.c("chip-ink"), None),
    };
    let d = div()
        .flex()
        .flex_none()
        .items_center()
        .h(px(22.))
        .px(px(8.))
        .rounded(px(6.))
        .bg(bg)
        .text_color(ink)
        .text_size(px(size::SMALL))
        .font_weight(FontWeight::BOLD)
        .child(text.into());
    match ring {
        Some(c) => d.border_1().border_color(c),
        None => d,
    }
}

/// `.hb-empty`: a bold title over a grey hint, centred.
pub fn empty_block(title: Option<SharedString>, body: Vec<AnyElement>, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .px(px(28.))
        .py(px(40.))
        .text_color(theme.c("ink-3"))
        .text_center()
        .children(title.map(|t| {
            div()
                .text_size(px(17.))
                .font_weight(FontWeight::BLACK)
                .text_color(theme.c("ink"))
                .mb(px(6.))
                .child(t)
        }))
        .child(
            div()
                .flex()
                .flex_wrap()
                .justify_center()
                .items_baseline()
                .gap(px(4.))
                .children(body),
        )
}

/// The console's `Avatar`: a monogram on the name's tone, for someone the
/// view does not know.
pub fn monogram_avatar(name: &str, size: f32, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(size))
        .rounded(px((size * 0.22).round().max(6.)))
        .bg(theme.tone_for(name))
        .text_color(theme.c("av-ink"))
        .font_weight(FontWeight::BLACK)
        .text_size(px((size * 0.4).round()))
        .child(crate::ui::format::monogram(name))
}

/// A span in the mono face (`.hb-mono`, 13px).
pub fn mono(text: impl Into<SharedString>) -> Div {
    div()
        .font_family(ui::MONO)
        .text_size(px(size::SMALL))
        .child(text.into())
}

/// `.hb-search`: a 40px box with the magnifier, holding a text field.
pub fn search_frame(field: impl IntoElement, theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(8.))
        .h(px(40.))
        .px(px(12.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface"))
        .text_color(theme.c("ink"))
        .child(ui::icon("search", 20., theme.c("ink-4")))
        .child(div().flex_1().min_w_0().child(field))
}

/// Reports the width the parent lays out at into `cell`, and re-renders
/// `view` when it changes (container queries).
pub fn width_probe(cell: Rc<Cell<f32>>, view: EntityId) -> impl IntoElement {
    canvas(
        move |bounds: Bounds<Pixels>, _, cx: &mut App| {
            let w = f32::from(bounds.size.width);
            if (w - cell.get()).abs() > 0.5 {
                cell.set(w);
                cx.notify(view);
            }
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Bring a list's rows up to `count` after the data changed, keeping the
/// scroll position: rows are added or cut at the end, and every row is
/// measured again (its content may have changed in place).
pub fn sync_list(state: &gpui::ListState, count: usize) {
    let old = state.item_count();
    if old == 0 {
        // A splice into an empty list moves its scroll anchor past the new
        // rows (to the end); a reset starts at the top.
        state.reset(count);
        return;
    }
    if count > old {
        state.splice(old..old, count - old);
    } else if count < old {
        state.splice(count..old, 0);
    }
    if old > 0 {
        state.remeasure_items(0..count.min(old));
    }
}

/// A list row stretched to the list's width (rows are laid out at their
/// content's width otherwise).
pub fn full_row(row: AnyElement) -> AnyElement {
    div().w_full().child(row).into_any_element()
}

/// The current workspace, for `?ws=` on reads.
pub fn workspace(cx: &App) -> Option<String> {
    Desk::get(cx)
        .read(cx)
        .workspace
        .clone()
        .filter(|w| !w.is_empty())
}

/// `GET` off the UI thread, read into `T`.
pub fn fetch<T: DeserializeOwned + Send + 'static>(
    path: String,
    cx: &App,
) -> Task<Result<T, ApiError>> {
    cx.background_spawn(async move { api::get_as::<T>(&path) })
}

/// `POST` off the UI thread (no workspace stamped, no refresh).
pub fn send(
    path: String,
    body: serde_json::Value,
    cx: &App,
) -> Task<Result<serde_json::Value, ApiError>> {
    cx.background_spawn(async move { api::post(&path, &body) })
}

/// Development aid: `HARBARD_APP_ROUTE=/lobby` (any console URL, see
/// `Route::parse`) opens the window there.
pub fn open_route_from_env(cx: &mut App) {
    let Ok(path) = std::env::var("HARBARD_APP_ROUTE") else {
        return;
    };
    if let Some(route) = crate::nav::Route::parse(&path) {
        crate::nav::replace(route, cx);
    }
}
