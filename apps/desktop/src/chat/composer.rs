//! The message box (`Composer`, `parts.tsx`): text, @-mentions, the + menu,
//! attachments.

use super::ComposeTarget;
use crate::theme::Theme;
use gpui::{Context, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, px};

/// Create it once per screen with `cx.new(|cx| Composer::new(target, window, cx))`
/// and call `set_target` when the conversation changes.
pub struct Composer {
    pub target: ComposeTarget,
    pub placeholder: SharedString,
}

impl Composer {
    pub fn new(target: ComposeTarget, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Composer {
            target,
            placeholder: SharedString::default(),
        }
    }

    pub fn set_target(&mut self, target: ComposeTarget, placeholder: SharedString, cx: &mut Context<Self>) {
        if self.target != target || self.placeholder != placeholder {
            self.target = target;
            self.placeholder = placeholder;
            cx.notify();
        }
    }
}

impl Render for Composer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        div()
            .m(px(20.))
            .h(px(52.))
            .rounded(px(8.))
            .border_1()
            .border_color(theme.c("line-strong"))
            .child(self.placeholder.clone())
    }
}
