//! The message layer every conversation screen shares: one message, a
//! stream of them, the composer, attachments. Owned by the chat lane; other
//! screens call only the functions and types below, whose signatures are
//! the contract between lanes.
//!
//! - [`message_item`] and [`stream`] live in `message.rs` and `stream.rs`.
//! - [`Composer`] lives in `composer.rs`.
//! - Attachments, the lightbox and the report pane live in `attachments.rs`.

pub mod attachments;
pub mod composer;
pub mod message;
pub mod stream;

pub use composer::Composer;

use crate::model::{Message, WorkspaceView};
use gpui::{AnyElement, App, SharedString, Window};
use std::rc::Rc;

/// Opens a message's thread (`onOpenThread`): each screen decides where.
#[derive(Clone)]
pub struct ThreadOpener(pub Rc<dyn Fn(&str, &mut Window, &mut App)>);

impl ThreadOpener {
    pub fn new(f: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        ThreadOpener(Rc::new(f))
    }
}

impl std::fmt::Debug for ThreadOpener {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ThreadOpener")
    }
}

/// How a message is shown where it appears. Build it with
/// `MessageOpts { fold: true, ..Default::default() }` so new fields never
/// break a caller.
#[derive(Clone, Debug, Default)]
pub struct MessageOpts {
    /// In a thread pane (`whole`): never folded, technical details open,
    /// no reply count, no "reply" action.
    pub in_thread: bool,
    /// Collapse long text behind "See more" (channels).
    pub fold: bool,
    /// Same author as the message above: no avatar or name line.
    pub continued: bool,
    /// The message drawn as selected (the open thread).
    pub selected: Option<String>,
    /// No hover tools (save, thread, feedback): archives and deleted rooms.
    pub read_only: bool,
    /// The direct-message stream's larger rows (`.hb-stream-dm`).
    pub dm: bool,
    /// Fold runs of internal updates into one line (channels, `foldInternal`).
    pub fold_internal: bool,
    /// Shows the replies link and the "Reply in a thread" tool.
    pub on_thread: Option<ThreadOpener>,
    /// The stream's empty-state line.
    pub empty: Option<SharedString>,
    /// Keeps a stream's scroll position apart from another stream on the
    /// same screen (defaults to one per screen).
    pub key: Option<SharedString>,
}

/// One message row (`MessageItem`).
pub fn message_item(msg: &Message, view: &WorkspaceView, opts: &MessageOpts, cx: &App) -> AnyElement {
    message::item(msg, view, opts, cx)
}

/// A list of messages in order, grouped by day with run and internal
/// grouping (`Stream`).
pub fn stream(messages: &[&Message], view: &WorkspaceView, opts: &MessageOpts, cx: &App) -> AnyElement {
    stream::stream(messages, view, opts, cx)
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
