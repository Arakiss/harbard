//! The message layer every conversation screen shares: one message, a
//! stream of them, the composer, attachments. Owned by the chat lane; other
//! screens call only the functions and types below, whose signatures are
//! the contract between lanes. Bodies here are placeholders until the chat
//! lane lands the real ones.

use crate::model::{Message, WorkspaceView};
use crate::theme::Theme;
use crate::ui;
use gpui::{AnyElement, App, Context, IntoElement, ParentElement, Render, SharedString, Styled, Window, div, px};

/// How a message is shown where it appears.
#[derive(Clone, Debug, Default)]
pub struct MessageOpts {
    /// In a thread pane: no reply count, no "reply" action.
    pub in_thread: bool,
    /// Collapse long text behind "See more" (channels).
    pub fold: bool,
    /// Same author as the message above: no avatar or name line.
    pub continued: bool,
}

/// One message row (`MessageItem`).
pub fn message_item(msg: &Message, view: &WorkspaceView, opts: &MessageOpts, cx: &App) -> AnyElement {
    let _ = (view, opts);
    let theme = cx.global::<Theme>();
    div()
        .px(px(20.))
        .py(px(6.))
        .child(ui::text(msg.author.clone(), ui::size::BODY, gpui::FontWeight::BLACK, theme.c("ink")))
        .child(ui::text(msg.text.clone(), ui::size::BODY, gpui::FontWeight::NORMAL, theme.c("ink")))
        .into_any_element()
}

/// A list of messages in order, grouped by day with run and internal
/// grouping (`Stream`).
pub fn stream(messages: &[&Message], view: &WorkspaceView, opts: &MessageOpts, cx: &App) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .children(messages.iter().map(|m| message_item(m, view, opts, cx)))
        .into_any_element()
}

/// Where a composer sends.
#[derive(Clone, Debug, PartialEq)]
pub enum ComposeTarget {
    /// A channel or room by id (`reply {channel, text}`).
    Channel(String),
    /// A direct message to an agent key (`reply {agent, text}`).
    Agent(String),
    /// A reply in a thread (`reply {agent?|channel?, reply_to, text}`).
    Thread { parent: String, channel: Option<String>, agent: Option<String> },
}

/// The message box (`Composer`): text, @-mentions, the + menu, attachments.
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
