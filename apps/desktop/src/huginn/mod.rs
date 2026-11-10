//! Huginn views: `.huginn` JSON attachments rendered natively. Owned by the
//! huginn lane; the chat lane embeds [`HuginnView`] for each such attachment.
//! The constructor's signature is the contract.

use crate::model::{Attachment, Message};
use crate::theme::Theme;
use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div, px};

/// One rendered view, with its own form state. Create once per attachment:
/// `cx.new(|cx| HuginnView::new(attachment.clone(), message.clone(), window, cx))`.
pub struct HuginnView {
    pub attachment: Attachment,
    pub message: Message,
}

impl HuginnView {
    pub fn new(attachment: Attachment, message: Message, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        HuginnView { attachment, message }
    }
}

impl Render for HuginnView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        div()
            .p(px(12.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme.c("line"))
            .child(self.attachment.name.clone())
    }
}
