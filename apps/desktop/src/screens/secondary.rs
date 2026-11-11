//! Pieces the secondary screens share (Lobby, Deliveries, History, Retro,
//! Setup, a deleted room): the console's `.hb-head`, `.hb-btn`, `.hb-chip`,
//! `.hb-empty` and `.hb-search`, a width probe for container breakpoints,
//! and a background read of serve.

use crate::api::{self, ApiError};
use crate::desk::Desk;
use crate::theme::Theme;
use crate::ui::{self, avatar_size, control, icon_size, leading, radius, row, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, AnyView, App, AppContext, Bounds, Div, ElementId, EntityId, InteractiveElement,
    IntoElement, ParentElement, Pixels, SharedString, Stateful, StatefulInteractiveElement, Styled,
    Task, Window, canvas, div, px,
};
use serde::de::DeserializeOwned;
use std::cell::Cell;
use std::rc::Rc;

/// The widest an empty state's text runs (F3).
const EMPTY_W: f32 = 320.;

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
        .line_height(px(leading::BODY))
        .text_color(theme.c("ink"))
}

/// `.hb-head`: the screen's one title at the gutter (T4), then the grey
/// topic line when it says something the title does not.
pub fn head(title: SharedString, topic: Option<SharedString>, theme: &Theme) -> Div {
    head_bar(theme)
        .child(ui::text(title, size::TITLE, weight::HEAVY, theme.c("ink")).flex_none())
        .children(topic.map(|t| {
            ui::text(t, size::SMALL, weight::REGULAR, theme.c("ink-3"))
                .min_w_0()
                .truncate()
        }))
}

/// The 56px bar every screen title sits in.
pub fn head_bar(theme: &Theme) -> Div {
    div()
        .h(px(row::HEADER))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(space::LG))
        .px(px(space::GUTTER))
        .border_b_1()
        .border_color(theme.c("line"))
}

/// The keyboard focus ring of a row (F1): inset, so rows do not overlap.
pub fn row_focus(row: Stateful<Div>, theme: &Theme) -> Stateful<Div> {
    let ring = ui::focus_ring(theme, true);
    row.tab_index(0)
        .focus_visible(move |s| s.shadow(vec![ring]))
}

/// The keyboard focus ring of a control (F1): outside its edge.
pub fn control_focus(control: Stateful<Div>, theme: &Theme) -> Stateful<Div> {
    let ring = ui::focus_ring(theme, false);
    control
        .tab_index(0)
        .focus_visible(move |s| s.shadow(vec![ring]))
}

/// A tooltip with one line of text (K3).
pub fn tip(text: SharedString) -> impl Fn(&mut Window, &mut App) -> AnyView + 'static {
    ui::kit_theme::tip(text)
}

/// `.hb-btn` (and `.hb-btn-primary`): `control::MD`, 14/700. `primary` is
/// the one accent action of a view (C2).
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
    let pressed = theme.c("control");
    let base = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(control::MD))
        .px(px(space::LG))
        .rounded(px(radius::BASE))
        .border_1()
        .border_color(edge)
        .bg(bg)
        .child(ui::text(label, size::UI, weight::STRONG, ink).whitespace_nowrap());
    if disabled {
        base.opacity(DISABLED)
    } else {
        control_focus(base, theme)
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .when(!primary, |d| d.active(move |s| s.bg(pressed)))
    }
}

/// A quiet action (`control::SM`, no edge, `ink-2`): what sits in a header
/// or at the end of a row and must not compete with the content.
pub fn quiet_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    disabled: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let hover = theme.c("hover");
    let pressed = theme.c("control");
    let base = div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .h(px(control::SM))
        .px(px(space::MD))
        .rounded(px(radius::BASE))
        .child(ui::text(label, size::UI, weight::STRONG, theme.c("ink-2")).whitespace_nowrap());
    if disabled {
        base.opacity(DISABLED)
    } else {
        control_focus(base, theme)
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .active(move |s| s.bg(pressed))
    }
}

/// What failed, where the action was (§2.7): `red-ink` on `red-bg`.
pub fn error_note(text: impl Into<SharedString>, theme: &Theme) -> Div {
    ui::text(text, size::SMALL, weight::REGULAR, theme.c("red-ink"))
        .px(px(space::MD))
        .py(px(space::HAIR))
        .rounded(px(radius::SM))
        .bg(theme.c("red-bg"))
}

/// Disabled controls (§2.7).
pub const DISABLED: f32 = 0.5;

/// `.hb-chip[data-tone]`: a state (green, amber, red) or neutral (C3).
pub fn chip(text: impl Into<SharedString>, tone: &str, theme: &Theme) -> Div {
    ui::chip(text, tone, theme).whitespace_nowrap()
}

/// `.hb-empty` (F3): a 14/700 line over a 14/400 sentence, `BREAK` below
/// what is above it, at most 320 px wide.
pub fn empty_block(title: Option<SharedString>, body: Vec<AnyElement>, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .items_center()
        .gap(px(space::XS))
        .pt(px(space::BREAK))
        .px(px(space::GUTTER))
        .children(title.map(|t| {
            ui::text(t, size::UI, weight::STRONG, theme.c("ink-2"))
                .max_w(px(EMPTY_W))
                .text_center()
        }))
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .gap(px(space::LG))
                .max_w(px(EMPTY_W))
                .text_size(px(size::UI))
                .line_height(px(leading::UI))
                .text_color(theme.c("ink-3"))
                .text_center()
                .children(body),
        )
}

/// The console's `Avatar`: a monogram on the name's tone, for something
/// that has no robot (a channel, an agent the view no longer knows).
pub fn monogram_avatar(name: &str, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(avatar_size::ROW))
        .rounded(px(radius::MD))
        .bg(theme.tone_for(name))
        .child(ui::text(
            crate::ui::format::monogram(name),
            size::UI,
            weight::STRONG,
            theme.c("av-ink"),
        ))
}

/// Something a person may copy, in the mono face beside `UI` or `SMALL`
/// text (T6: two pixels smaller than its neighbour).
pub fn mono(text: impl Into<SharedString>) -> Div {
    div()
        .font_family(ui::MONO)
        .text_size(px(size::META))
        .line_height(px(leading::SMALL))
        .child(text.into())
}

/// `.hb-search`: a `control::MD` box with the magnifier, holding a text
/// field; `focused` draws the ring the field itself does not.
pub fn search_frame(field: impl IntoElement, focused: bool, theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .gap(px(space::MD))
        .h(px(control::MD))
        .px(px(space::MD))
        .rounded(px(radius::BASE))
        .border_1()
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface"))
        .text_color(theme.c("ink"))
        .when(focused, |d| d.shadow(vec![ui::focus_ring(theme, false)]))
        .child(ui::icon("search", icon_size::ROW, theme.c("ink-3")))
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
