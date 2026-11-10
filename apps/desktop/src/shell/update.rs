//! The update pill in the title band (`update-banner.tsx`, `update-state.tsx`).

use gpui::{App, Context, IntoElement, Render, Window, div};

pub fn init(_cx: &mut App) {}

pub struct UpdatePill;

impl UpdatePill {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        UpdatePill
    }
}

impl Render for UpdatePill {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}
