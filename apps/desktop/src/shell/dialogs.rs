//! The workspace menu's small dialogs (`SmallDialog`, `ShortcutsList` in
//! `workspace-menu.tsx`; "About harbard" is Preferences'): a 480px raised card 12% down the
//! window, a 56px head with its title and a close button, closed by Escape,
//! the button or a press on the scrim.
//!
//! They are GPUI Kit dialogs dressed as the console's, so focus, Escape and
//! the scrim behave like every other dialog in the app.

use crate::i18n::t;
use crate::theme::Theme;
use crate::ui::{self, icon, kit_theme};
use gpui::{
    AnyElement, App, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_kit::component::WindowExt as _;
use std::rc::Rc;

/// A dialog's body, built again on every render of the dialog layer.
pub type Body = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// Open a small dialog titled `title` holding `body`.
pub fn open_small(title: SharedString, body: Body, window: &mut Window, cx: &mut App) {
    let top = window.viewport_size().height * 0.12;
    window.open_dialog(cx, move |dialog, window, cx| {
        let theme = cx.global::<Theme>().clone();
        kit_theme::hb_dialog(dialog, cx)
            .w(px(480.))
            .margin_top(top)
            .close_button(false)
            .p_0()
            .gap_0()
            .min_h(px(0.))
            .line_height(gpui::relative(1.46))
            .font_family(ui::SANS)
            .text_size(px(ui::size::BODY))
            .text_color(theme.c("ink"))
            .child(pane_head(title.clone(), &theme))
            .child(
                div()
                    .px(px(18.))
                    .pt(px(14.))
                    .pb(px(18.))
                    .child(body(window, cx)),
            )
    });
}

/// `.hb-pane-head` with `.hb-pane-title` and `.hb-close`.
pub fn pane_head(title: SharedString, theme: &Theme) -> impl IntoElement {
    div()
        .h(px(56.))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(8.))
        .pl(px(18.))
        .pr(px(12.))
        .border_b_1()
        .border_color(theme.c("line"))
        .child(
            div()
                .text_size(px(18.))
                .font_weight(FontWeight::BLACK)
                .child(title),
        )
        .child(
            close_button("hb-small-close", theme).on_click(|_, window, cx| window.close_dialog(cx)),
        )
}

/// `.hb-close`: 32px, an X in ink-3, the control wash under the pointer.
pub fn close_button(id: &'static str, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    let hover = theme.c("control");
    div()
        .id(id)
        .ml_auto()
        .size(px(32.))
        .rounded(px(6.))
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(icon("x", 20., theme.c("ink-3")))
}

/// `.hb-shortcuts > div`: a term column 150px wide and its meaning.
fn row(term: AnyElement, what: AnyElement, theme: &Theme) -> impl IntoElement {
    div()
        .flex()
        .gap(px(12.))
        .py(px(6.))
        .border_b_1()
        .border_color(theme.c("line"))
        .child(
            div()
                .w(px(150.))
                .flex_none()
                .font_weight(FontWeight::BOLD)
                .child(term),
        )
        .child(div().flex_1().min_w_0().child(what))
}

/// `.hb-kbd`.
fn kbd(keys: impl Into<SharedString>, theme: &Theme) -> impl IntoElement {
    div()
        .px(px(6.))
        .py(px(1.))
        .rounded(px(4.))
        .border_1()
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface-2"))
        .text_size(px(12.))
        .font_weight(FontWeight::NORMAL)
        .text_color(theme.c("ink-3"))
        .child(keys.into())
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
                .children(shortcuts(cx).into_iter().map(|(keys, what)| {
                    row(
                        div().flex().child(kbd(keys, &theme)).into_any_element(),
                        what.into_any_element(),
                        &theme,
                    )
                }))
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

/// `.hb-btn` at a height.
pub fn button(
    id: &'static str,
    label: SharedString,
    height: f32,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let hover = theme.c("btn-hover");
    div()
        .id(id)
        .h(px(height))
        .px(px(if height < 32. { 10. } else { 16. }))
        .flex()
        .items_center()
        .rounded(px(6.))
        .border_1()
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface"))
        .font_weight(FontWeight::BOLD)
        .text_size(px(if height < 32. { 13. } else { 15. }))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .child(label)
}

/// Keep a dialog's view alive across the builder's re-runs.
pub fn view_body<V: Render>(view: Entity<V>) -> Body {
    Rc::new(move |_, _| view.clone().into_any_element())
}
