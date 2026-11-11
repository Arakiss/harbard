//! The workspace menu's small dialogs (`SmallDialog`, `ShortcutsList` in
//! `workspace-menu.tsx`; "About harbard" is Preferences'): a 480px raised card 12% down the
//! window, a 56px head with its title and a close button, closed by Escape,
//! the button or a press on the scrim.
//!
//! They are GPUI Kit dialogs dressed as the console's, so focus, Escape and
//! the scrim behave like every other dialog in the app.

use crate::i18n::t;
use crate::theme::Theme;
use crate::ui::kit_theme::{self, FocusRing as _};
use crate::ui::{self, control, icon, icon_size, leading, radius, row, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Entity, InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_kit::component::WindowExt as _;
use std::rc::Rc;

/// A small dialog's width, and the keys' column in the shortcuts list.
const WIDTH: f32 = 480.;
const KEYS_COLUMN: f32 = 144.;

/// A dialog's body, built again on every render of the dialog layer.
pub type Body = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// Open a small dialog titled `title` holding `body`.
pub fn open_small(title: SharedString, body: Body, window: &mut Window, cx: &mut App) {
    let top = window.viewport_size().height * 0.12;
    window.open_dialog(cx, move |dialog, window, cx| {
        let theme = cx.global::<Theme>().clone();
        kit_theme::hb_dialog(dialog, cx)
            .w(px(WIDTH))
            .margin_top(top)
            .close_button(false)
            .p_0()
            .gap_0()
            .min_h(px(0.))
            .line_height(px(leading::UI))
            .font_family(ui::SANS)
            .text_size(px(size::UI))
            .text_color(theme.c("ink"))
            .child(pane_head(title.clone(), &theme, cx))
            .child(
                div()
                    .px(px(space::GUTTER))
                    .py(px(space::XL))
                    .child(body(window, cx)),
            )
    });
}

/// `.hb-pane-head`: 56 px, the one title at the gutter and the close button.
pub fn pane_head(title: SharedString, theme: &Theme, cx: &App) -> impl IntoElement {
    div()
        .h(px(row::HEADER))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(space::MD))
        .pl(px(space::GUTTER))
        .pr(px(space::LG))
        .border_b_1()
        .border_color(theme.c("line"))
        .child(
            ui::text(title, size::TITLE, weight::HEAVY, theme.c("ink"))
                .flex_1()
                .min_w_0()
                .truncate(),
        )
        .child(
            close_button("hb-small-close", theme, cx)
                .on_click(|_, window, cx| window.close_dialog(cx)),
        )
}

/// `.hb-close`: a 28 px icon button, the control wash under the pointer,
/// "Close" and its key in the tooltip.
pub fn close_button(id: &'static str, theme: &Theme, cx: &App) -> gpui::Stateful<gpui::Div> {
    let hover = theme.c("control");
    div()
        .id(id)
        .flex_none()
        .size(px(control::SM))
        .rounded(px(radius::BASE))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .focus_ring(theme, false)
        .tooltip(kit_theme::tip_keys(t(cx, "shell.close"), &["Esc"]))
        .child(icon("x", icon_size::ROW, theme.c("ink-3")))
}

/// `.hb-shortcuts > div`: the keys' column and what they do; a hairline
/// between rows of the list.
fn row(keys: AnyElement, what: SharedString, first: bool, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(space::LG))
        .py(px(space::MD))
        .when(!first, |d| d.border_t_1().border_color(theme.c("line")))
        .child(div().w(px(KEYS_COLUMN)).flex_none().flex().child(keys))
        .child(
            ui::text(what, size::UI, weight::REGULAR, theme.c("ink"))
                .flex_1()
                .min_w_0(),
        )
}

/// "⇧Esc" as its keys: a modifier glyph is its own key, and so are the
/// parts of "Shift+Enter".
pub fn key_parts(keys: &str) -> Vec<String> {
    let mut parts: Vec<String> = Vec::new();
    for word in keys.split('+').map(str::trim).filter(|w| !w.is_empty()) {
        let mut rest = String::new();
        for c in word.chars() {
            if matches!(c, '⌘' | '⇧' | '⌥' | '⌃') {
                parts.push(c.to_string());
            } else {
                rest.push(c);
            }
        }
        if !rest.is_empty() {
            parts.push(rest);
        }
    }
    parts
}

/// The keyboard shortcuts the shell answers (`shortcuts()`).
pub fn shortcuts(cx: &App) -> Vec<(SharedString, SharedString)> {
    vec![
        ("⌘K".into(), t(cx, "workspace.shortcutSearch")),
        ("Esc".into(), t(cx, "workspace.shortcutMarkRead")),
        ("⇧Esc".into(), t(cx, "workspace.shortcutMarkAllRead")),
        (
            t(cx, "workspace.shortcutKeyEnter"),
            t(cx, "workspace.shortcutSend"),
        ),
        (
            t(cx, "workspace.shortcutKeyShiftEnter"),
            t(cx, "workspace.shortcutNewLine"),
        ),
        ("@".into(), t(cx, "workspace.shortcutMention")),
    ]
}

/// "Keyboard shortcuts".
pub fn open_shortcuts(window: &mut Window, cx: &mut App) {
    open_small(
        t(cx, "shell.keyboardShortcuts"),
        Rc::new(|_, cx| {
            let theme = cx.global::<Theme>().clone();
            div()
                .flex()
                .flex_col()
                .children(
                    shortcuts(cx)
                        .into_iter()
                        .enumerate()
                        .map(|(i, (keys, what))| {
                            let parts = key_parts(&keys);
                            row(
                                ui::kbd_keys(parts.iter().map(String::as_str), &theme)
                                    .into_any_element(),
                                what,
                                i == 0,
                                &theme,
                            )
                        }),
                )
                .into_any_element()
        }),
        window,
        cx,
    );
}

/// "About harbard": one dialog for every entry point (Preferences owns it).
pub fn open_about(window: &mut Window, cx: &mut App) {
    crate::prefs::open_about(window, cx);
}

/// `.hb-btn`: a surface button with a strong edge, 14/700. `height` is one
/// of the control heights; the label's inset follows it.
pub fn button(
    id: &'static str,
    label: SharedString,
    height: f32,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let hover = theme.c("btn-hover");
    let height = height.clamp(control::SM, control::LG);
    div()
        .id(id)
        .flex_none()
        .h(px(height))
        .px(px(if height < control::MD {
            space::MD
        } else {
            space::LG
        }))
        .flex()
        .items_center()
        .rounded(px(radius::BASE))
        .border_1()
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface"))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .focus_ring(theme, false)
        .child(ui::text(label, size::UI, weight::STRONG, theme.c("ink")))
}

/// Keep a dialog's view alive across the builder's re-runs.
pub fn view_body<V: Render>(view: Entity<V>) -> Body {
    Rc::new(move |_, _| view.clone().into_any_element())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_split_into_one_chip_each() {
        assert_eq!(key_parts("⌘K"), vec!["⌘", "K"]);
        assert_eq!(key_parts("⇧Esc"), vec!["⇧", "Esc"]);
        assert_eq!(key_parts("Shift+Enter"), vec!["Shift", "Enter"]);
        assert_eq!(key_parts("Esc"), vec!["Esc"]);
        assert_eq!(key_parts("@"), vec!["@"]);
    }
}
