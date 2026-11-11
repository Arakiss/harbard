//! The console's popover menus (`.hb-pop`, `.hb-pop-item`, `workspace-menu.tsx`):
//! the pieces every rail menu is built from, and the workspace menu's items.
//!
//! A menu is a raised 300px panel at a fixed spot in the window, drawn over
//! everything ([`float`]) on a layer that takes the pointer, so
//! nothing under it hovers or shows a tooltip. Arrows walk the items
//! (wrapping), Home and End jump, Enter runs, Escape closes, a press outside
//! closes. The keyboard highlight appears only once a key is used, as
//! `:focus-visible` did; the pointer highlights by hovering.
//!
//! Rows are 28 px, inset 4 px from the panel with 6 px corners; an item's
//! keys sit at its trailing edge (`DESIGN.md` K2).

use crate::theme::Theme;
use crate::ui::kit_theme::FocusRing as _;
use crate::ui::{self, icon, icon_size, leading, motion, radius, row, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, Animation, AnimationExt as _, AnyElement, App, Div, InteractiveElement, IntoElement,
    ParentElement, Pixels, Point, SharedString, Stateful, StatefulInteractiveElement, Styled,
    Window, anchored, deferred, div, ease_out_quint, point, px,
};
use std::rc::Rc;

/// A menu's width.
pub const MENU_WIDTH: f32 = 300.;

/// What a menu entry does.
pub type Action = Rc<dyn Fn(&mut Window, &mut App)>;

/// One entry of a menu.
#[derive(Clone)]
pub struct MenuItem {
    pub id: SharedString,
    pub label: SharedString,
    pub icon: &'static str,
    /// A small amber dot after the label: something new behind this item.
    pub dot: bool,
    /// The item's shortcut, one chip per key.
    pub keys: &'static [&'static str],
    pub run: Action,
}

/// Arrow-key focus inside a menu: the next index, wrapping (`menuStep`).
pub fn menu_step(at: Option<usize>, count: usize, key: &str) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let at = at.unwrap_or(match key {
        "up" => 0,
        _ => count - 1,
    });
    Some(match key {
        "down" => (at + 1) % count,
        "up" => (at + count - 1) % count,
        "home" => 0,
        "end" => count - 1,
        _ => at,
    })
}

/// Float a menu over the window with its `anchor` corner at `at`. It sits
/// on a layer the size of the window that takes the pointer, so what is
/// under the menu neither hovers nor shows a tooltip; give the panel
/// `on_mouse_down_out` to close on a press there. It appears in
/// `motion::BASE` (opacity and 4 px of travel) unless motion is reduced.
pub fn float(
    id: &'static str,
    panel: impl IntoElement,
    at: Point<Pixels>,
    anchor: Anchor,
    window: &Window,
    cx: &App,
) -> impl IntoElement {
    let view = window.viewport_size();
    let panel = div().child(panel);
    let panel = if crate::robot::clock::reduced(cx) {
        panel.into_any_element()
    } else {
        panel
            .with_animation(
                id,
                Animation::new(motion::BASE).with_easing(ease_out_quint()),
                |d, t| d.opacity(t).mt(px(space::XS * (t - 1.))),
            )
            .into_any_element()
    };
    deferred(
        anchored().position(point(px(0.), px(0.))).child(
            div().w(view.width).h(view.height).occlude().child(
                anchored()
                    .anchor(anchor)
                    .position(at)
                    .snap_to_window()
                    .child(panel),
            ),
        ),
    )
    .with_priority(1)
}

/// `.hb-pop`: the raised panel, 8px corners, the pop shadow.
pub fn pop_panel(theme: &Theme) -> Div {
    div()
        .w(px(MENU_WIDTH))
        .flex()
        .flex_col()
        .py(px(space::XS))
        .bg(theme.c("surface-raised"))
        .text_color(theme.c("ink"))
        .rounded(px(radius::MD))
        .shadow(theme.pop_shadow())
        .font_family(ui::SANS)
        .text_size(px(size::UI))
        .line_height(px(leading::UI))
        .font_weight(weight::REGULAR)
}

/// `.hb-pop-head`: the name and the status line, ruled below.
pub fn pop_head(
    title: impl Into<SharedString>,
    status: impl Into<SharedString>,
    theme: &Theme,
) -> Div {
    div()
        .flex()
        .flex_col()
        .px(px(space::LG))
        .pt(px(space::SM))
        .pb(px(space::MD))
        .mb(px(space::XS))
        .border_b_1()
        .border_color(theme.c("line"))
        .child(ui::text(title, size::UI, weight::STRONG, theme.c("ink")).truncate())
        .child(ui::text(status, size::META, weight::REGULAR, theme.c("ink-3")).truncate())
}

/// `.hb-pop-title`: a section label at the rows' text edge.
pub fn pop_title(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .flex()
        .items_center()
        .h(px(row::MENU))
        .px(px(space::LG))
        .child(ui::section_label(text, theme))
}

/// `.hb-pop-sep`.
pub fn pop_sep(theme: &Theme) -> Div {
    div()
        .flex_none()
        .h(px(1.))
        .my(px(space::XS))
        .bg(theme.c("line"))
}

/// `.hb-pop-item`: a 28 px row, `menu-hover` under the pointer or the keyboard.
pub fn pop_item(id: impl Into<SharedString>, lit: bool, theme: &Theme) -> Stateful<Div> {
    let hover_bg = theme.c("menu-hover");
    let hover_ink = theme.c("menu-hover-ink");
    div()
        .id(id.into())
        .flex_none()
        .mx(px(space::XS))
        .h(px(row::MENU))
        .flex()
        .items_center()
        .gap(px(space::MD))
        .px(px(space::MD))
        .rounded(px(radius::BASE))
        .cursor_pointer()
        .when(lit, |d| d.bg(hover_bg).text_color(hover_ink))
        .hover(move |s| s.bg(hover_bg).text_color(hover_ink))
        .focus_ring(theme, true)
}

/// A row's leading icon: `ink-3`, the row's ink while it is lit or hovered.
pub fn item_icon(
    name: &'static str,
    group: impl Into<SharedString>,
    lit: bool,
    theme: &Theme,
) -> impl IntoElement {
    let hover_ink = theme.c("menu-hover-ink");
    let ink = if lit { hover_ink } else { theme.c("ink-3") };
    icon(name, icon_size::ROW, ink).group_hover(group, move |s| s.text_color(hover_ink))
}

/// A row's keys at its trailing edge (K2), readable on the lit row too.
pub fn item_keys(
    keys: &'static [&'static str],
    group: impl Into<SharedString>,
    lit: bool,
    theme: &Theme,
) -> Div {
    let group: SharedString = group.into();
    let hover_ink = theme.c("menu-hover-ink");
    let edge = hover_ink.opacity(0.4);
    div()
        .ml_auto()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(space::HAIR))
        .children(keys.iter().map(|k| {
            ui::kbd(*k, theme)
                .when(lit, |d| d.text_color(hover_ink).border_color(edge))
                .group_hover(group.clone(), move |s| {
                    s.text_color(hover_ink).border_color(edge)
                })
        }))
}

/// The workspace menu's rows: icon, label, then the amber dot or the keys.
pub fn menu_rows(
    items: &[MenuItem],
    lit: Option<usize>,
    theme: &Theme,
    on_run: impl Fn(usize) -> Action,
) -> Vec<AnyElement> {
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let run = on_run(i);
            let lit_here = lit == Some(i);
            let group: SharedString = format!("hb-pop-{}", item.id).into();
            pop_item(item.id.clone(), lit_here, theme)
                .group(group.clone())
                .on_click(move |_, window, cx| run(window, cx))
                .child(item_icon(item.icon, group.clone(), lit_here, theme))
                .child(div().min_w_0().truncate().child(item.label.clone()))
                .when(item.dot, |d| {
                    d.child(
                        div()
                            .ml_auto()
                            .flex_none()
                            .size(px(space::MD))
                            .rounded_full()
                            .bg(theme.c("amber")),
                    )
                })
                .when(!item.dot && !item.keys.is_empty(), |d| {
                    d.child(item_keys(item.keys, group, lit_here, theme))
                })
                .into_any_element()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_wrap_and_home_end_jump() {
        assert_eq!(menu_step(Some(0), 4, "down"), Some(1));
        assert_eq!(menu_step(Some(3), 4, "down"), Some(0));
        assert_eq!(menu_step(Some(0), 4, "up"), Some(3));
        assert_eq!(menu_step(Some(2), 4, "home"), Some(0));
        assert_eq!(menu_step(Some(0), 4, "end"), Some(3));
        assert_eq!(menu_step(None, 4, "down"), Some(0));
        assert_eq!(menu_step(None, 4, "up"), Some(3));
        assert_eq!(menu_step(Some(0), 0, "down"), None);
    }
}
