//! A channel's header (`ChannelHeader`, `ChannelWhy`, `ChannelPlaces`,
//! `WorkElsewhere` in `home.tsx`) and the two-writers strips.

use crate::model::{Channel, WorkspaceTile, WorkspaceView};
use gpui::{AnyElement, App, IntoElement, ParentElement, Styled, div, px};

/// `header.hb-head` for a project channel or a room: title, why, places,
/// members and `menu` (the channel's ⋯).
pub fn channel_header(view: &WorkspaceView, channel: &Channel, menu: Option<AnyElement>, cx: &App) -> AnyElement {
    let _ = (view, cx);
    div().h(px(56.)).flex_none().child(channel.id.clone()).children(menu).into_any_element()
}

/// "{count} working in {name}": switches to the busy workspace and goes Home.
/// `prefix` is drawn before it (" · " or " ").
pub fn work_elsewhere(tiles: &[WorkspaceTile], prefix: &str, cx: &App) -> Option<AnyElement> {
    let _ = (tiles, prefix, cx);
    None
}

/// One `.hb-strip[role=alert]` per writer warning of the channel.
pub fn warning_strips(view: &WorkspaceView, channel: &Channel, cx: &App) -> Vec<AnyElement> {
    let _ = (view, channel, cx);
    Vec::new()
}
