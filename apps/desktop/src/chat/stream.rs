//! A scrolling, day-grouped list of messages (`Stream`, `parts.tsx`).

use super::MessageOpts;
use crate::model::{Message, WorkspaceView};
use gpui::{AnyElement, App, IntoElement, ParentElement, Styled, div};

pub fn stream(messages: &[&Message], view: &WorkspaceView, opts: &MessageOpts, cx: &App) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .children(messages.iter().map(|m| super::message::item(m, view, opts, cx)))
        .into_any_element()
}
