//! The channel screen (`ChannelScreen`, `home.tsx`): `/` and `/c/<id>`.

use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div};

pub struct ChannelScreen {}

impl ChannelScreen {
    pub fn new(_window: &mut Window, _cx: &mut Context<Self>) -> Self {
        ChannelScreen {}
    }
}

impl Render for ChannelScreen {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().flex_1().child("channel")
    }
}
