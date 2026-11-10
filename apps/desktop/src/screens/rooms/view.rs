//! A room's screen (`RoomView`): header, archived strip, feed and composer,
//! or the `#feedback` inbox.

use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div};

pub struct RoomView {
    pub room: String,
}

impl RoomView {
    pub fn new(room: String, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        RoomView { room }
    }

    /// Show another room (the channel screen keeps one RoomView).
    pub fn set_room(&mut self, room: String, cx: &mut Context<Self>) {
        if self.room != room {
            self.room = room;
            cx.notify();
        }
    }
}

impl Render for RoomView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().flex_1().child(self.room.clone())
    }
}
