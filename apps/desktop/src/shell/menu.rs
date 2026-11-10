//! The console's popover menus (`.hb-pop`, `.hb-pop-item`, `workspace-menu.tsx`):
//! the pieces every rail menu is built from, and the workspace menu's items.
//!
//! A menu is a raised 300px panel at a fixed spot in the window, drawn over
//! everything (`deferred`). Arrows walk the items (wrapping), Home and End
//! jump, Enter runs, Escape closes; a press outside closes it and still
//! reaches what it landed on. The keyboard highlight appears only once a key
//! is used, as `:focus-visible` did; the pointer highlights by hovering.

use crate::theme::Theme;
use crate::ui::{self, icon};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Div, FontWeight, InteractiveElement, IntoElement, ParentElement, SharedString, Stateful,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use std::rc::Rc;

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

/// `.hb-pop`: the raised panel, 300px, 8px corners, the pop shadow.
pub fn pop_panel(theme: &Theme) -> Div {
    div()
        .w(px(300.))
        .flex()
        .flex_col()
        .py(px(8.))
        .bg(theme.c("surface-raised"))
        .text_color(theme.c("ink"))
        .rounded(px(8.))
        .shadow(theme.pop_shadow())
        .font_family(ui::SANS)
        .text_size(px(ui::size::BODY))
        .line_height(gpui::relative(1.46))
}

/// `.hb-pop-head`: bold "harbard" and the status line, ruled below.
pub fn pop_head(title: impl Into<SharedString>, status: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .flex()
        .flex_col()
        .px(px(16.))
        .pt(px(10.))
        .pb(px(12.))
        .border_b_1()
        .border_color(theme.c("line"))
        .child(div().font_weight(FontWeight::BLACK).child(title.into()))
        .child(div().text_size(px(13.)).text_color(theme.c("ink-3")).child(status.into()))
}

/// `.hb-pop-title`.
pub fn pop_title(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .px(px(16.))
        .pt(px(6.))
        .pb(px(2.))
        .text_size(px(13.))
        .font_weight(FontWeight::BOLD)
        .text_color(theme.c("ink-3"))
        .child(text.into())
}

/// `.hb-pop-sep`.
pub fn pop_sep(theme: &Theme) -> Div {
    div().h(px(1.)).my(px(6.)).bg(theme.c("line"))
}

/// `.hb-pop-item`: a full-width row, blue under the pointer or the keyboard.
pub fn pop_item(id: impl Into<SharedString>, lit: bool, theme: &Theme) -> Stateful<Div> {
    let hover_bg = theme.c("menu-hover");
    let hover_ink = theme.c("menu-hover-ink");
    div()
        .id(id.into())
        .w_full()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(16.))
        .py(px(6.))
        .cursor_pointer()
        .when(lit, |d| d.bg(hover_bg).text_color(hover_ink))
        .hover(move |s| s.bg(hover_bg).text_color(hover_ink))
}

/// The workspace menu's rows: icon (16px), label, and the amber dot.
pub fn menu_rows(items: &[MenuItem], lit: Option<usize>, theme: &Theme, on_run: impl Fn(usize) -> Action) -> Vec<AnyElement> {
    items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let run = on_run(i);
            let lit_here = lit == Some(i);
            let ink = if lit_here { theme.c("menu-hover-ink") } else { theme.c("ink") };
            let group: SharedString = format!("hb-pop-{}", item.id).into();
            let hover_ink = theme.c("menu-hover-ink");
            pop_item(item.id.clone(), lit_here, theme)
                .group(group.clone())
                .on_click(move |_, window, cx| run(window, cx))
                .child(icon(item.icon, 16., ink).group_hover(group, move |s| s.text_color(hover_ink)))
                .child(item.label.clone())
                .when(item.dot, |d| {
                    d.child(div().ml_auto().size(px(8.)).rounded_full().bg(theme.c("amber")))
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
