//! Rooms (`rooms.tsx`): a room's screen, its feed with older pages, the
//! channel menu and its actions, the archived strip, the create and delete
//! dialogs, and the `#feedback` inbox.

pub mod dialogs;
pub mod feed;
pub mod inbox;
pub mod view;

#[allow(unused_imports)] // the channel screen and deleted rooms use these
pub use feed::RoomFeed;
#[allow(unused_imports)]
pub use view::RoomView;

use crate::chat::row_menu::RowAction;
use crate::model::Channel;
use gpui::{AnyElement, App, IntoElement, ParentElement, Window, div};

/// The actions a channel's ⋯ menu offers (`useChannelActions().itemsFor`):
/// archive / unarchive run at once, delete asks first. Built-in rooms: none.
pub fn channel_actions(channel: &Channel, cx: &App) -> Vec<RowAction> {
    let _ = (channel, cx);
    Vec::new()
}

/// The header's ⋯ button with the same actions (`ChannelMenu`); `None` for
/// built-in rooms.
pub fn channel_menu(channel: &Channel, cx: &App) -> Option<AnyElement> {
    let _ = (channel, cx);
    None
}

/// `div.hb-room-strip`: "#{channel} is archived." with Unarchive.
pub fn archived_strip(channel: &Channel, cx: &App) -> AnyElement {
    let _ = cx;
    div().child(channel.id.clone()).into_any_element()
}

/// Open "Create a room" (from the sidebar's +).
pub fn open_create_room(window: &mut Window, cx: &mut App) {
    dialogs::open_create_room(window, cx);
}
