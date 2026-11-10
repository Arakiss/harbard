//! One message row (`MessageItem`, `parts.tsx`).

use super::MessageOpts;
use crate::model::{Message, WorkspaceView};
use crate::theme::Theme;
use crate::ui;
use gpui::{AnyElement, App, IntoElement, ParentElement, Styled, div, px};

pub fn item(msg: &Message, view: &WorkspaceView, opts: &MessageOpts, cx: &App) -> AnyElement {
    let _ = (view, opts);
    let theme = cx.global::<Theme>();
    div()
        .px(px(20.))
        .py(px(6.))
        .child(ui::text(msg.author.clone(), ui::size::BODY, gpui::FontWeight::BLACK, theme.c("ink")))
        .child(ui::text(msg.text.clone(), ui::size::BODY, gpui::FontWeight::NORMAL, theme.c("ink")))
        .into_any_element()
}
