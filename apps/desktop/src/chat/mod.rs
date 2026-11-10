//! The message layer every conversation screen shares: one message, a
//! stream of them, the composer, attachments. Owned by the chat lane; other
//! screens call only the functions and types below, whose signatures are
//! the contract between lanes.
//!
//! - [`message_item`] and [`stream`] live in `message.rs` and `stream.rs`.
//! - [`Composer`] lives in `composer.rs`.
//! - Attachments, the lightbox and the report pane live in `attachments.rs`.

pub mod agent_name;
pub mod attachments;
pub mod composer;
pub mod message;
pub mod preview;
pub mod row_menu;
pub mod stream;

#[allow(unused_imports)] // the other lanes' screens use these
pub use composer::{Composer, ComposerActions, ComposerEvent, ComposerState};

use crate::model::{Message, WorkspaceView};
use gpui::{AnyElement, App, SharedString, Window};
use std::rc::Rc;

/// What opening a thread runs, given the parent message id.
pub type OpenThread = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// Opens a message's thread (`onOpenThread`): each screen decides where.
#[derive(Clone)]
pub struct ThreadOpener(pub OpenThread);

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

/// An agent's name with its role line (`AgentName`); double click renames
/// when `renamable` and the agent has a workshop name.
pub fn agent_name_el(agent: &crate::model::Agent, renamable: bool, cx: &App) -> AnyElement {
    let _ = cx;
    gpui::IntoElement::into_any_element(agent_name::agent_name(agent, renamable))
}

/// The status line (`AgentStatus`): icon, words, ticking timer.
pub fn agent_status_line(agent: &crate::model::Agent, now: u64, cx: &App) -> AnyElement {
    let _ = now;
    let theme = cx.global::<crate::theme::Theme>();
    gpui::IntoElement::into_any_element(crate::robot::status_line(agent).color(theme.c("ink-3")))
}

/// What a row menu item does when chosen.
pub type RowMenuAction = Rc<dyn Fn(&mut Window, &mut App)>;

/// One item of a row's ⋯ menu.
#[derive(Clone)]
pub struct RowMenuItem {
    pub label: SharedString,
    pub icon: Option<&'static str>,
    pub action: RowMenuAction,
}

/// A row's ⋯ button and its menu (`RowMenu`). For the whole row with
/// right-click and "Mark as read", use [`row_menu::row_menu`].
pub fn row_menu(id: SharedString, items: Vec<RowMenuItem>, cx: &App) -> AnyElement {
    let label = id.clone();
    let actions = items
        .into_iter()
        .map(|i| row_menu::RowAction { label: i.label, run: i.action })
        .collect();
    row_menu::more_button(id, label, actions, cx)
}

/// A local file, opened on this Mac when clicked (`FileCard`).
pub fn file_card(path: &str, cx: &App) -> AnyElement {
    message::file_card(path, cx)
}
