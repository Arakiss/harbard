//! The room dialogs (`CreateRoomDialog`, `DeleteRoomDialog`).

use gpui::{App, Window};

pub fn open_create_room(window: &mut Window, cx: &mut App) {
    let _ = (window, cx);
}

pub fn open_delete_room(name: String, window: &mut Window, cx: &mut App) {
    let _ = (name, window, cx);
}
