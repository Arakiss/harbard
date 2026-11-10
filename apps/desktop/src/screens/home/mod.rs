//! Home (`home.tsx`): the sidebar every Home route shares, the channel
//! screen (`/`, `/c/<id>`), the Threads screen and the thread pane.
//!
//! [`HomeShell`] puts the shared [`sidebar::Sidebar`] beside any screen, as
//! the console's `HomeShell` did for Home, Threads, Deliveries and Lobby.

pub mod channel;
pub mod header;
pub mod members;
pub mod sidebar;
pub mod thread_pane;
pub mod threads;
pub mod work_panel;

use crate::desk::Desk;
use crate::theme::Theme;
use crate::ui;
use gpui::{AnyElement, AnyView, App, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div};

/// The sidebar beside a Home screen. Shows the loading state, with no
/// sidebar, until the first read arrives.
pub struct HomeShell {
    sidebar: Entity<sidebar::Sidebar>,
    content: AnyView,
}

impl HomeShell {
    pub fn new(content: AnyView, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()).detach();
        HomeShell {
            sidebar: sidebar::shared(window, cx),
            content,
        }
    }
}

impl Render for HomeShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if Desk::get(cx).read(cx).view.is_none() {
            return div().size_full().flex().child(loading(cx)).into_any_element();
        }
        div()
            .size_full()
            .flex()
            .child(self.sidebar.clone())
            .child(self.content.clone())
            .into_any_element()
    }
}

/// `Loading` (`parts.tsx`): reading, or serve is not answering.
pub fn loading(cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    ui::empty(crate::i18n::t(cx, "messages.loading"), theme).into_any_element()
}
