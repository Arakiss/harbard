//! The season's decorations over the frame (`season-decor.tsx`).

use gpui::{Context, IntoElement, Render, Window, div};

pub struct SeasonDecor;

impl SeasonDecor {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        SeasonDecor
    }
}

impl Render for SeasonDecor {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
