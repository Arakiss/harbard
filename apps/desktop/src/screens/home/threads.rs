//! The Threads screen (`ThreadsScreen`, `home.tsx`): `/threads`.

use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div};

pub struct ThreadsScreen {}

impl ThreadsScreen {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        ThreadsScreen {}
    }
}

impl Render for ThreadsScreen {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().flex_1().child("threads")
    }
}
