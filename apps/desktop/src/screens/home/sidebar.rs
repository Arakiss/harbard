//! Home's sidebar (`Sidebar`, `home.tsx`): the workspace name, Lobby,
//! Threads, Deliveries, channels, main sessions, agents, temporary
//! sessions, rooms, and "More tools" at the foot.

use crate::theme::Theme;
use gpui::{App, AppContext, Context, Entity, Global, IntoElement, ParentElement, Render, Styled, Window, div, px};

pub struct Sidebar {}

struct Shared(Entity<Sidebar>);

impl Global for Shared {}

/// The one sidebar every Home screen shows, so its folds and filter stay
/// as they were when moving between Home, Threads, Deliveries and Lobby.
pub fn shared(window: &mut Window, cx: &mut App) -> Entity<Sidebar> {
    if let Some(s) = cx.try_global::<Shared>() {
        return s.0.clone();
    }
    let sidebar = cx.new(|cx| Sidebar::new(window, cx));
    cx.set_global(Shared(sidebar.clone()));
    sidebar
}

impl Sidebar {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Sidebar {}
    }
}

impl Render for Sidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        div().w(px(300.)).h_full().flex_none().bg(theme.c("sidebar")).child("sidebar")
    }
}
