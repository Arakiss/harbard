//! A room's messages with older pages from serve (`RoomFeed`).

use gpui::{Context, IntoElement, ParentElement, Render, SharedString, Styled, Window, div};

pub struct RoomFeed {
    pub room: String,
    pub read_only: bool,
    pub empty: SharedString,
}

impl RoomFeed {
    pub fn new(room: String, read_only: bool, empty: SharedString, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        RoomFeed { room, read_only, empty }
    }
}

impl Render for RoomFeed {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().flex_1().child(self.room.clone())
    }
}
