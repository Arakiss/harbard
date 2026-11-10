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
    pub state: ComposerState,
    actions: Option<ComposerActions>,
}

impl Composer {
    pub fn new(target: ComposeTarget, _window: &mut Window, _cx: &mut Context<Self>) -> Self {
        Composer {
            target,
            placeholder: SharedString::default(),
            state: ComposerState::default(),
            actions: None,
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let note = |s: &SharedString| {
            div()
                .mx(px(20.))
                .text_size(px(13.))
                .text_color(theme.c("ink-3"))
                .child(s.clone())
        };
        if let Some(blocked) = &self.state.blocked {
            return div()
                .m(px(20.))
                .px(px(16.))
                .py(px(14.))
                .rounded(px(10.))
                .border_1()
                .border_dashed()
                .border_color(theme.c("line-strong"))
                .text_color(theme.c("ink-3"))
                .child(format!("{blocked}."))
                .into_any_element();
        }
        let actions = self.actions.clone().map(|a| a(window, cx));
        div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .children(self.state.notice.as_ref().map(|n| note(n).mt(px(8.))))
            .child(
                div()
                    .mx(px(20.))
                    .mt(px(4.))
                    .h(px(if self.state.boxed { 96. } else { 52. }))
                    .rounded(px(8.))
                    .border_1()
                    .border_color(theme.c("line-strong"))
                    .child(self.placeholder.clone()),
            )
            .children(self.state.hint.as_ref().map(note))
            .children(actions.map(|a| div().mx(px(20.)).mt(px(8.)).flex().gap(px(16.)).child(a)))
            .child(div().h(px(12.)))
            .into_any_element()
    }
}

// contract: added by inbox lane ------------------------------------------------
// Stubs with the signatures the inbox screens build against; the chat lane
// fills the bodies (the lead reconciles this block at integration).

/// What a composer says around its box: why there is none (`blocked`), what
/// the note waits for (`notice`), the question before sending (`confirm`,
/// answered with "Send anyway" → `anyway: true`), the path a message takes
/// (`hint`), and the box variant (`boxed`: ⌘Enter sends, a "Reply" button).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComposerState {
    pub blocked: Option<SharedString>,
    pub notice: Option<SharedString>,
    pub confirm: Option<SharedString>,
    pub hint: Option<SharedString>,
    pub boxed: bool,
}

/// Controls drawn beside "Reply" in the box variant (`ComposerActionsContext`).
pub type ComposerActions = std::rc::Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// A message left the composer (`onSent`).
#[derive(Clone, Debug, PartialEq)]
pub enum ComposerEvent {
    Sent,
}

impl gpui::EventEmitter<ComposerEvent> for Composer {}

impl Composer {
    pub fn set_state(&mut self, state: ComposerState, cx: &mut Context<Self>) {
        if self.state != state {
            self.state = state;
            cx.notify();
        }
    }

    pub fn set_actions(&mut self, actions: Option<ComposerActions>, cx: &mut Context<Self>) {
        self.actions = actions;
        cx.notify();
    }
}

/// An agent's name with its role line (`AgentName`); double click renames
/// when `renamable` and the agent has a workshop name (chat lane).
pub fn agent_name_el(agent: &crate::model::Agent, renamable: bool, cx: &App) -> AnyElement {
    let _ = renamable;
    let theme = cx.global::<Theme>();
    let role = agent
        .role_line
        .as_deref()
        .map(|line| crate::i18n::serve_text(cx, agent.role_line_code.as_deref(), agent.role_line_params.as_ref(), line));
    div()
        .flex()
        .flex_col()
        .min_w_0()
        .child(div().truncate().child(crate::derive::agent_name(agent, cx)))
        .children(role.map(|r| {
            div()
                .truncate()
                .text_size(px(12.))
                .font_weight(gpui::FontWeight::NORMAL)
                .text_color(theme.c("ink-4"))
                .child(r)
        }))
        .into_any_element()
}

/// The status line (`AgentStatus`): icon, words, ticking timer. Inherits the
/// size and colour of where it sits (chat/robot lane draws the real icons).
pub fn agent_status_line(agent: &crate::model::Agent, now: u64, cx: &App) -> AnyElement {
    let _ = now;
    crate::ui::agent_card::status_line(
        agent,
        crate::ui::agent_card::StatusStyle {
            size: 13.,
            color: cx.global::<Theme>().c("ink-3"),
            time_right: true,
            show_time: true,
        },
        cx,
    )
    .into_any_element()
}

/// What a row menu item does when chosen.
pub type RowMenuAction = std::rc::Rc<dyn Fn(&mut Window, &mut App)>;

/// One item of a row's ⋯ menu.
#[derive(Clone)]
pub struct RowMenuItem {
    pub label: SharedString,
    pub icon: Option<&'static str>,
    pub action: RowMenuAction,
}

/// A row's ⋯ button and its menu (`RowMenu`).
pub fn row_menu(id: SharedString, items: Vec<RowMenuItem>, cx: &App) -> AnyElement {
    use gpui_kit::component::button::{Button, ButtonVariants as _};
    use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
    let _ = cx;
    Button::new(id)
        .ghost()
        .compact()
        .icon(gpui_kit::component::Icon::default().path("icons/ellipsis.svg"))
        .dropdown_menu_with_anchor(gpui::Anchor::TopRight, move |mut menu, _, _| {
            for item in &items {
                let run = item.action.clone();
                menu = menu.item(PopupMenuItem::new(item.label.clone()).on_click(move |_, window, cx| run(window, cx)));
            }
            menu
        })
        .into_any_element()
}

/// A local file, opened on this Mac when clicked (`FileCard`).
pub fn file_card(path: &str, cx: &App) -> AnyElement {
    use gpui::{InteractiveElement as _, StatefulInteractiveElement as _};
    let theme = cx.global::<Theme>();
    let open = path.to_string();
    let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or(path).to_string();
    div()
        .id(SharedString::from(format!("hb-file-{path}")))
        .mt(px(8.))
        .flex()
        .items_center()
        .gap(px(12.))
        .min_w(px(240.))
        .max_w(px(420.))
        .overflow_hidden()
        .px(px(16.))
        .py(px(12.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface-2"))
        .cursor_pointer()
        .hover(|s| s.bg(theme.c("file-hover")).border_color(theme.c("line-hover")))
        .child(ui::icon("file-text", 24., theme.c("ink-2")))
        .child(
            div()
                .flex()
                .flex_col()
                .min_w_0()
                .child(div().truncate().font_weight(gpui::FontWeight::BOLD).text_color(theme.c("ink")).child(name))
                .child(
                    div()
                        .truncate()
                        .text_size(px(12.))
                        .text_color(theme.c("ink-4"))
                        .child(ui::format::short_path(path)),
                ),
        )
        .on_click(move |_, _, cx| {
            // `open` shows the file on this Mac; serve answers, no refresh follows.
            crate::desk::Desk::get(cx)
                .update(cx, |d, cx| d.post("open", serde_json::json!({ "path": open }), cx))
                .detach();
        })
        .into_any_element()
}
