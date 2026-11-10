//! The main window's frame (`ShellBody`): a 40px title band across the top,
//! the 78px rail on the left and the rounded panel holding the screen.
//!
//! ```text
//! ┌──────────── topbar 40 (drag; traffic lights at 14,18) ────────────┐
//! │ rail 78 │ panel (margin 0 6 6 0, radius 8)                        │
//! │         │   the screen for the current route                      │
//! └─────────┴──────────────────────────────────────────────────────────┘
//! ```

pub mod rail;

use crate::desk::Desk;
use crate::nav::Nav;
use crate::screens::ScreenHost;
use crate::theme::Theme;
use crate::ui;
use gpui::{
    App, AppContext, Context, Entity, InteractiveElement, IntoElement, MouseButton, ParentElement, Render, Styled,
    Window, WindowControlArea, div, px,
};

pub struct Workspace {
    screens: Entity<ScreenHost>,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let desk = Desk::get(cx);
        cx.observe(&desk, |_, _, cx| cx.notify()).detach();
        cx.observe_global::<Nav>(|_, cx| cx.notify()).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        let screens = cx.new(|cx| ScreenHost::new(window, cx));
        Workspace { screens }
    }
}

/// A strip that drags the window; a double click zooms it, as the title bar.
pub fn drag_band() -> gpui::Div {
    div()
        .window_control_area(WindowControlArea::Drag)
        .on_mouse_down(MouseButton::Left, |ev, window, _| {
            if ev.click_count == 2 {
                window.titlebar_double_click();
            }
        })
}

impl Render for Workspace {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let desk = Desk::get(cx).read(cx);
        let stale = desk.view.is_some() && desk.error.is_some();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.c("frame"))
            .font_family(ui::SANS)
            .text_size(px(ui::size::BODY))
            .text_color(theme.c("ink"))
            .child(drag_band().h(px(40.)).w_full().flex_none())
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(rail::Rail::new())
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .mr(px(6.))
                            .mb(px(6.))
                            .rounded(px(8.))
                            .overflow_hidden()
                            .bg(theme.c("list"))
                            .border_1()
                            .border_color(theme.c("panel-ring"))
                            .child(self.screens.clone()),
                    ),
            )
            .when(stale, |d| {
                d.child(
                    div()
                        .absolute()
                        .right(px(20.))
                        .bottom(px(20.))
                        .px(px(14.))
                        .py(px(10.))
                        .rounded(px(8.))
                        .bg(theme.c("red-bg"))
                        .border_1()
                        .border_color(theme.c("red-line"))
                        .text_color(theme.c("banner-ink"))
                        .text_size(px(ui::size::SMALL))
                        .child(crate::i18n::t(cx, "shell.harbardNotResponding")),
                )
            })
    }
}

use gpui::prelude::FluentBuilder as _;

pub fn init(_cx: &mut App) {}
