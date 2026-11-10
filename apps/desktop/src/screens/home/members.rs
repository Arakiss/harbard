//! Who has worked in a channel (`ChannelMembers`): the avatar stack and its
//! popover of members working now and earlier.

use crate::model::{Channel, WorkspaceView};
use gpui::{AnyElement, App};

/// The members button with its popover; `None` when nobody has worked here.
pub fn members_button(view: &WorkspaceView, channel: &Channel, cx: &App) -> Option<AnyElement> {
    let _ = (view, channel, cx);
    None
}
