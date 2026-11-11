//! The message box (`Composer`, `parts.tsx`): text, @-mentions, the + menu,
//! attachments, "send anyway?", and the Activity detail's boxed variant.
//!
//! A reply is stored on the bus: to an agent as an ask its lead relays (or
//! its own session reads), in a channel as a note per reader. Nothing is
//! echoed before serve accepts it; the hub's next read shows it.

use super::ComposeTarget;
use super::{focusable, tip, tip_keys};
use crate::derive::composer::{
    ComposerGate, MentionAt, MentionOption, apply_mention, full_body, insert_at_sign,
    match_mentions, mention_at, mention_options, notice_line, reply_body, reply_error, review_text,
};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::theme::Theme;
use crate::ui::{self, SANS, control, icon_size, leading, radius, row, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable as _,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, ParentElement, PathPromptOptions,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, deferred, div,
    px, relative,
};
use gpui_kit::component::input::{
    Enter, Escape, IndentInline, InputEvent, MoveDown, MoveUp, Textarea, TextareaState,
};
use gpui_kit::component::{Sizable as _, Size};
use std::rc::Rc;

/// What the screen decides about its composer each render.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ComposerState {
    /// Shown instead of the box (`{blocked}.`): nobody to leave it for.
    pub blocked: Option<SharedString>,
    /// Shown above the box: the note is stored and waits.
    pub notice: Option<SharedString>,
    /// Asked before sending ("send anyway?").
    pub confirm: Option<SharedString>,
    /// The quiet line under the box.
    pub hint: Option<SharedString>,
    /// The Activity detail's box: a tall field, ⌘Enter sends, a Reply button.
    pub boxed: bool,
}

impl ComposerState {
    /// A channel's or an agent's gate (`channel_composer`, `composer_*`).
    pub fn from_gate(gate: ComposerGate) -> Self {
        ComposerState {
            blocked: gate.blocked.map(Into::into),
            notice: gate.notice.map(Into::into),
            confirm: gate.confirm.map(Into::into),
            ..Default::default()
        }
    }
}

/// What the composer tells the screen.
#[derive(Clone, Debug, PartialEq)]
pub enum ComposerEvent {
    /// serve accepted a message; the box is empty again.
    Sent,
}

/// Extra buttons beside the boxed variant's Reply (`ComposerActionsContext`).
pub type ComposerActions = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

type SentHandler = Rc<dyn Fn(&mut Window, &mut App)>;
type NewThreadHandler = Rc<dyn Fn(String, &mut Window, &mut App)>;

/// A file "Attach file" uploaded, cited by path when the message is sent.
#[derive(Clone, Debug, PartialEq)]
struct Upload {
    path: String,
    name: String,
}

/// The + menu: closed, its items, or the reviewer picker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Plus {
    Closed,
    Main,
    Picking,
}

#[derive(Clone, Debug)]
enum PlusItem {
    Attach,
    Mention,
    Review,
    NewThread,
    Reviewer(String),
}

/// Create it once per screen with `cx.new(|cx| Composer::new(target, window, cx))`
/// and call `set_target` (and `set_state`) as the conversation changes.
pub struct Composer {
    pub target: ComposeTarget,
    pub placeholder: SharedString,
    state: ComposerState,
    input: Entity<TextareaState>,
    /// The field has the caret (the box draws its focus ring).
    focused: bool,
    uploads: Vec<Upload>,
    busy: bool,
    error: Option<SharedString>,
    /// ConfirmSend is showing.
    asking: bool,
    plus: Plus,
    /// The + menu's keyboard highlight.
    plus_at: Option<usize>,
    menu_focus: FocusHandle,
    /// The mention list's highlight.
    pick: usize,
    /// Esc hid the mention list until the next keystroke.
    mention_off: bool,
    on_sent: Option<SentHandler>,
    on_new_thread: Option<NewThreadHandler>,
    actions: Option<ComposerActions>,
    _subs: Vec<Subscription>,
}

impl EventEmitter<ComposerEvent> for Composer {}

/// Auto-grow bounds in rows: the line box grows to about 160px.
const LINE_ROWS: (usize, usize) = (1, 7);
const BOX_ROWS: (usize, usize) = (4, 14);
const LINE_HEIGHT: f32 = leading::BODY;
/// The + menu's and the mention list's widths, and the boxed field's
/// least height.
const PLUS_MENU_WIDTH: f32 = 260.;
const MENTIONS_WIDTH: f32 = 420.;
const BOX_MIN_HEIGHT: f32 = 120.;

impl Composer {
    pub fn new(target: ComposeTarget, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextareaState::new(window, cx).auto_grow(LINE_ROWS.0, LINE_ROWS.1));
        let sub = cx.subscribe_in(
            &input,
            window,
            |this, _, ev: &InputEvent, _window, cx| match ev {
                InputEvent::Change => {
                    this.mention_off = false;
                    this.pick = 0;
                    cx.notify();
                }
                InputEvent::Focus | InputEvent::Blur => cx.notify(),
                InputEvent::PressEnter { .. } => {}
            },
        );
        Composer {
            target,
            placeholder: SharedString::default(),
            state: ComposerState::default(),
            input,
            focused: false,
            uploads: Vec::new(),
            busy: false,
            error: None,
            asking: false,
            plus: Plus::Closed,
            plus_at: None,
            menu_focus: cx.focus_handle(),
            pick: 0,
            mention_off: false,
            on_sent: None,
            on_new_thread: None,
            actions: None,
            _subs: vec![sub],
        }
    }

    /// Point the composer at another conversation. The draft stays, as it
    /// did in the console when the route changed under a mounted composer;
    /// call `set_draft("")` to start empty.
    pub fn set_target(
        &mut self,
        target: ComposeTarget,
        placeholder: SharedString,
        cx: &mut Context<Self>,
    ) {
        if self.target != target {
            self.target = target;
            self.error = None;
            self.asking = false;
            cx.notify();
        }
        if self.placeholder != placeholder {
            self.placeholder = placeholder;
            cx.notify();
        }
    }

    /// What blocks, waits, asks or hints, and the variant. Cheap when unchanged.
    pub fn set_state(&mut self, state: ComposerState, cx: &mut Context<Self>) {
        if self.state == state {
            return;
        }
        if self.state.boxed != state.boxed {
            let (min, max) = if state.boxed { BOX_ROWS } else { LINE_ROWS };
            self.input.update(cx, |s, cx| s.set_auto_grow(min, max, cx));
        }
        if state.confirm.is_none() {
            self.asking = false;
        }
        self.state = state;
        cx.notify();
    }

    /// Extra buttons beside Reply in the boxed variant. Only a change between
    /// none and some re-renders; pass a fresh closure when their content
    /// changes and notify the composer yourself.
    pub fn set_actions(&mut self, actions: Option<ComposerActions>, cx: &mut Context<Self>) {
        let flip = self.actions.is_some() != actions.is_some();
        self.actions = actions;
        if flip {
            cx.notify();
        }
    }

    /// Called after serve accepted a message (besides `ComposerEvent::Sent`).
    pub fn on_sent(&mut self, f: impl Fn(&mut Window, &mut App) + 'static) {
        self.on_sent = Some(Rc::new(f));
    }

    /// Shows "New thread" in the + menu; it hands over the draft.
    pub fn on_new_thread(&mut self, f: impl Fn(String, &mut Window, &mut App) + 'static) {
        self.on_new_thread = Some(Rc::new(f));
    }

    /// Replace the draft and put the caret at its end.
    pub fn set_draft(
        &mut self,
        text: impl Into<SharedString>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text: SharedString = text.into();
        let end = text.len();
        self.input.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.set_selected_range(end..end, cx);
        });
        cx.notify();
    }

    /// Put the caret in the box.
    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| s.focus(window, cx));
    }

    pub fn text(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn paths(&self) -> Vec<String> {
        self.uploads.iter().map(|u| u.path.clone()).collect()
    }

    fn channel(&self) -> Option<&str> {
        match &self.target {
            ComposeTarget::Channel(c) => Some(c),
            ComposeTarget::Thread {
                channel: Some(c),
                agent: None,
                ..
            } => Some(c),
            _ => None,
        }
    }

    /// Whether Esc belongs to this composer now: its mention list or its +
    /// menu is open. Screens that mark read on Esc check this first.
    pub fn takes_escape(&self, cx: &App) -> bool {
        self.plus != Plus::Closed || !self.mentions(cx).1.is_empty()
    }

    /// The `@word` at the caret and the options it matches, unless Esc hid
    /// them or the + menu is open over them.
    fn mentions(&self, cx: &App) -> (Option<MentionAt>, Vec<MentionOption>) {
        if self.mention_off || self.plus != Plus::Closed {
            return (None, Vec::new());
        }
        let input = self.input.read(cx);
        let text = input.value();
        let Some(at) = mention_at(&text, input.cursor()) else {
            return (None, Vec::new());
        };
        let Some(view) = Desk::get(cx).read(cx).view.clone() else {
            return (None, Vec::new());
        };
        let options = mention_options(&view, self.channel(), cx);
        let shown = match_mentions(&options, &at.query);
        (Some(at), shown)
    }

    fn choose(&mut self, option: &MentionOption, window: &mut Window, cx: &mut Context<Self>) {
        let (text, caret) = {
            let input = self.input.read(cx);
            let text = input.value();
            let caret = input.cursor();
            let Some(at) = mention_at(&text, caret) else {
                return;
            };
            apply_mention(&text, at.start, caret, &option.handle)
        };
        self.input.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.set_selected_range(caret..caret, cx);
        });
        cx.notify();
    }

    // ---------- keys (capture phase, before the field sees them) ----------

    fn key_enter(&mut self, action: &Enter, window: &mut Window, cx: &mut Context<Self>) {
        let (_, shown) = self.mentions(cx);
        if !shown.is_empty() {
            let option = shown[self.pick.min(shown.len() - 1)].clone();
            self.choose(&option, window, cx);
            cx.stop_propagation();
            return;
        }
        let submit = if self.state.boxed {
            action.secondary
        } else {
            !action.shift
        };
        if submit {
            cx.stop_propagation();
            self.send(false, window, cx);
        }
    }

    fn key_tab(&mut self, _: &IndentInline, window: &mut Window, cx: &mut Context<Self>) {
        let (_, shown) = self.mentions(cx);
        if !shown.is_empty() {
            let option = shown[self.pick.min(shown.len() - 1)].clone();
            self.choose(&option, window, cx);
            cx.stop_propagation();
        }
    }

    fn key_move(&mut self, down: bool, cx: &mut Context<Self>) {
        let (_, shown) = self.mentions(cx);
        if shown.is_empty() {
            return;
        }
        let n = shown.len();
        let at = self.pick.min(n - 1);
        self.pick = if down { (at + 1) % n } else { (at + n - 1) % n };
        cx.stop_propagation();
        cx.notify();
    }

    /// Esc with the list open hides it and stops there: the screen behind
    /// uses Esc to mark the conversation read.
    fn key_escape(&mut self, _: &Escape, _window: &mut Window, cx: &mut Context<Self>) {
        let (_, shown) = self.mentions(cx);
        if !shown.is_empty() {
            self.mention_off = true;
            cx.stop_propagation();
            cx.notify();
        }
    }

    // ---------- writes ----------

    fn send(&mut self, anyway: bool, window: &mut Window, cx: &mut Context<Self>) {
        let body = full_body(&self.text(cx), &self.paths());
        if body.is_empty() || self.busy {
            return;
        }
        if self.state.confirm.is_some() && !anyway {
            self.asking = true;
            cx.notify();
            return;
        }
        self.asking = false;
        self.busy = true;
        self.error = None;
        cx.notify();
        let payload = reply_body(&self.target, &body, anyway);
        let task = Desk::get(cx).update(cx, |d, cx| d.post("reply", payload, cx));
        cx.spawn_in(window, async move |this, cx| {
            let out = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match out {
                    Ok(_) => {
                        this.clear(window, cx);
                        cx.emit(ComposerEvent::Sent);
                        if let Some(f) = this.on_sent.clone() {
                            f(window, cx);
                        }
                    }
                    Err(e) => this.error = Some(refusal(&e, cx).into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |s, cx| s.set_value("", window, cx));
        self.uploads.clear();
        self.error = None;
    }

    /// "Ask for a review from…": always to that agent, whatever the target.
    fn review(&mut self, agent: String, window: &mut Window, cx: &mut Context<Self>) {
        let text = review_text(&self.text(cx), &self.paths(), cx);
        let payload = reply_body(&ComposeTarget::Agent(agent), &text, false);
        let task = Desk::get(cx).update(cx, |d, cx| d.post("reply", payload, cx));
        cx.spawn_in(window, async move |this, cx| {
            let out = task.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match out {
                    Ok(_) => this.clear(window, cx),
                    Err(e) => this.error = Some(refusal(&e, cx).into()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// "Attach file": the OS picker, then [`Self::attach_paths`].
    fn attach(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| this.attach_paths(paths, window, cx));
        })
        .detach();
    }

    /// Each file's bytes to serve, which stores them like `harbard attach`;
    /// a chip per stored file, in the order they finish.
    pub fn attach_paths(
        &mut self,
        paths: Vec<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for path in paths {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            cx.spawn_in(window, async move |this, cx| {
                let up_name = name.clone();
                let out = cx
                    .background_spawn(async move { upload(&path, &up_name) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    match out {
                        Ok(u) => this.uploads.push(u),
                        Err(error) => {
                            this.error = Some(tv(
                                cx,
                                "messages.attachFailed",
                                &[("name", &name), ("error", &error)],
                            ))
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn mention_from_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (text, caret) = {
            let input = self.input.read(cx);
            insert_at_sign(&input.value(), input.cursor())
        };
        self.input.update(cx, |s, cx| {
            s.set_value(text, window, cx);
            s.set_selected_range(caret..caret, cx);
            s.focus(window, cx);
        });
        self.mention_off = false;
        cx.notify();
    }

    // ---------- the + menu ----------

    fn reviewers(&self, cx: &App) -> Vec<(String, String)> {
        let Some(view) = Desk::get(cx).read(cx).view.clone() else {
            return Vec::new();
        };
        view.agents
            .iter()
            .filter(|a| !a.temporary && a.presence != "finished")
            .take(12)
            .map(|a| (a.key.clone(), a.name.clone()))
            .collect()
    }

    fn plus_items(&self, cx: &App) -> Vec<(PlusItem, SharedString)> {
        match self.plus {
            Plus::Closed => Vec::new(),
            Plus::Main => {
                let mut items = vec![
                    (PlusItem::Attach, t(cx, "messages.attachFile")),
                    (PlusItem::Mention, t(cx, "messages.mention")),
                ];
                if !self.reviewers(cx).is_empty() {
                    items.push((PlusItem::Review, t(cx, "messages.askReviewFrom")));
                }
                if self.on_new_thread.is_some() {
                    items.push((PlusItem::NewThread, t(cx, "messages.newThread")));
                }
                items
            }
            Plus::Picking => self
                .reviewers(cx)
                .into_iter()
                .map(|(key, name)| (PlusItem::Reviewer(key), name.into()))
                .collect(),
        }
    }

    fn close_plus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.plus = Plus::Closed;
        self.plus_at = None;
        self.focus(window, cx);
        cx.notify();
    }

    fn run_plus(&mut self, item: PlusItem, window: &mut Window, cx: &mut Context<Self>) {
        match item {
            PlusItem::Review => {
                self.plus = Plus::Picking;
                self.plus_at = None;
                window.focus(&self.menu_focus, cx);
                cx.notify();
            }
            PlusItem::Attach => {
                self.close_plus(window, cx);
                self.attach(window, cx);
            }
            PlusItem::Mention => {
                self.close_plus(window, cx);
                self.mention_from_menu(window, cx);
            }
            PlusItem::NewThread => {
                self.close_plus(window, cx);
                if let Some(f) = self.on_new_thread.clone() {
                    f(self.text(cx), window, cx);
                }
            }
            PlusItem::Reviewer(agent) => {
                self.close_plus(window, cx);
                self.review(agent, window, cx);
            }
        }
    }

    fn plus_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let items = self.plus_items(cx);
        let n = items.len();
        match ev.keystroke.key.as_str() {
            "escape" => {
                cx.stop_propagation();
                self.close_plus(window, cx);
            }
            "down" | "up" if n > 0 => {
                let down = ev.keystroke.key == "down";
                self.plus_at = Some(match self.plus_at {
                    None => {
                        if down {
                            0
                        } else {
                            n - 1
                        }
                    }
                    Some(i) if down => (i + 1) % n,
                    Some(i) => (i + n - 1) % n,
                });
                cx.stop_propagation();
                cx.notify();
            }
            "enter" | "space" if n > 0 => {
                cx.stop_propagation();
                let at = self.plus_at.unwrap_or(0).min(n - 1);
                self.run_plus(items[at].0.clone(), window, cx);
            }
            _ => {}
        }
    }

    fn toggle_plus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.plus == Plus::Closed {
            self.plus = Plus::Main;
            self.plus_at = None;
            window.focus(&self.menu_focus, cx);
        } else {
            self.plus = Plus::Closed;
        }
        cx.notify();
    }

    // ---------- drawing ----------

    fn render_plus_menu(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let items = self.plus_items(cx);
        let picking = self.plus == Plus::Picking;
        let mut menu = div()
            .id("composer-plus-menu")
            .track_focus(&self.menu_focus)
            .on_key_down(cx.listener(Self::plus_key))
            .absolute()
            .bottom(px(control::MD + space::MD))
            .left(px(-space::SM))
            .w(px(PLUS_MENU_WIDTH))
            .p(px(space::XS))
            .gap(px(space::HAIR))
            .rounded(px(radius::MD))
            .bg(theme.c("surface-raised"))
            .text_color(theme.c("ink"))
            .shadow(theme.pop_shadow())
            .flex()
            .flex_col();
        if picking {
            menu = menu.child(
                div()
                    .flex()
                    .items_center()
                    .h(px(row::MENU))
                    .px(px(space::MD))
                    .child(ui::section_label(t(cx, "messages.askReviewFrom"), theme)),
            );
            if items.is_empty() {
                menu = menu.child(
                    div()
                        .flex()
                        .items_center()
                        .h(px(row::MENU))
                        .px(px(space::MD))
                        .child(ui::text(
                            t(cx, "messages.noReviewers"),
                            size::UI,
                            weight::REGULAR,
                            theme.c("ink-3"),
                        )),
                );
            }
        }
        for (i, (item, label)) in items.into_iter().enumerate() {
            let lit = self.plus_at == Some(i);
            let hover_bg = theme.c("menu-hover");
            let hover_ink = theme.c("menu-hover-ink");
            menu = menu.child(
                div()
                    .id(("composer-plus-item", i))
                    .flex()
                    .flex_none()
                    .items_center()
                    .w_full()
                    .h(px(row::MENU))
                    .px(px(space::MD))
                    .rounded(px(radius::BASE))
                    .text_size(px(size::UI))
                    .line_height(px(leading::UI))
                    .cursor_pointer()
                    .when(lit, |d| d.bg(hover_bg).text_color(hover_ink))
                    .hover(move |s| s.bg(hover_bg).text_color(hover_ink))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.run_plus(item.clone(), window, cx)
                    }))
                    .child(div().min_w_0().truncate().child(label)),
            );
        }
        deferred(menu).with_priority(1).into_any_element()
    }

    fn render_mentions(
        &self,
        shown: &[MentionOption],
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let at = self.pick.min(shown.len().saturating_sub(1));
        let mut list = div()
            .w(px(MENTIONS_WIDTH))
            .max_w(relative(0.9))
            .p(px(space::XS))
            .gap(px(space::HAIR))
            .rounded(px(radius::MD))
            .bg(theme.c("surface-raised"))
            .text_color(theme.c("ink"))
            .shadow(theme.pop_shadow())
            .flex()
            .flex_col();
        for (i, o) in shown.iter().enumerate() {
            let option = o.clone();
            let selected = i == at;
            list = list.child(
                div()
                    .id(("composer-mention", i))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(space::MD))
                    .h(px(row::MENU))
                    .px(px(space::MD))
                    .rounded(px(radius::BASE))
                    .text_size(px(size::UI))
                    .line_height(px(leading::UI))
                    .cursor_pointer()
                    .when(!selected, |d| {
                        d.hover(|s| {
                            s.bg(theme.c("menu-hover"))
                                .text_color(theme.c("menu-hover-ink"))
                        })
                    })
                    .when(selected, |d| {
                        d.bg(theme.c("menu-hover"))
                            .text_color(theme.c("menu-hover-ink"))
                    })
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| {
                            cx.stop_propagation();
                            this.choose(&option, window, cx);
                        }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_weight(weight::STRONG)
                            .child(o.label.clone()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .text_size(px(size::SMALL))
                            .opacity(0.75)
                            .truncate()
                            .child(o.hint.clone()),
                    ),
            );
        }
        let left = if self.state.boxed {
            px(0.)
        } else {
            px(space::GUTTER)
        };
        deferred(
            div()
                .absolute()
                .bottom_full()
                .left(left)
                .right_0()
                .pb(px(space::XS))
                .child(list),
        )
        .with_priority(1)
        .into_any_element()
    }

    /// The wait slot: ConfirmSend, or the notice.
    fn render_wait(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.asking
            && let Some(question) = self.state.confirm.clone()
        {
            let btn = |id: &'static str, label: SharedString, primary: bool| {
                focusable(
                    div()
                        .id(id)
                        .h(px(control::SM))
                        .px(px(space::LG))
                        .flex()
                        .flex_none()
                        .items_center()
                        .rounded(px(radius::BASE))
                        .border_1()
                        .text_size(px(size::UI))
                        .line_height(px(leading::UI))
                        .font_weight(weight::STRONG)
                        .cursor_pointer(),
                    false,
                    theme,
                )
                .map(|d| {
                    if primary {
                        d.bg(theme.c("accent-deep"))
                            .border_color(theme.c("accent-deep"))
                            .text_color(theme.c("accent-ink"))
                            .hover(|s| s.bg(theme.c("accent-hover")))
                    } else {
                        d.bg(theme.c("surface"))
                            .border_color(theme.c("line-strong"))
                            .text_color(theme.c("ink"))
                            .hover(|s| s.bg(theme.c("btn-hover")))
                    }
                })
                .child(label)
            };
            return Some(
                div()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .justify_between()
                    .gap(px(space::MD))
                    .mb(px(space::MD))
                    .px(px(space::LG))
                    .py(px(space::SM))
                    .rounded(px(radius::MD))
                    .bg(theme.c("warn-bg"))
                    .text_color(theme.c("warn-ink"))
                    .text_size(px(size::SMALL))
                    .line_height(px(leading::SMALL))
                    .font_weight(weight::STRONG)
                    .child(question)
                    .child(
                        div()
                            .flex()
                            .gap(px(space::MD))
                            .child(
                                btn("composer-send-anyway", t(cx, "messages.sendAnyway"), true)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send(true, window, cx)
                                    })),
                            )
                            .child(
                                btn("composer-cancel", t(cx, "messages.cancel"), false).on_click(
                                    cx.listener(|this, _, _, cx| {
                                        this.asking = false;
                                        cx.notify();
                                    }),
                                ),
                            ),
                    )
                    .into_any_element(),
            );
        }
        let notice = self.state.notice.as_ref()?;
        Some(
            // The same quiet row as a channel's alerts: 32 px, 13/400.
            div()
                .flex()
                .items_center()
                .min_h(px(control::MD))
                .mb(px(space::MD))
                .px(px(space::LG))
                .py(px(space::SM))
                .rounded(px(radius::MD))
                .bg(theme.c("warn-bg"))
                .text_color(theme.c("warn-ink"))
                .text_size(px(size::SMALL))
                .line_height(px(leading::SMALL))
                // In a narrow pane the sentence wraps; it is never cut (T7).
                .child(div().flex_1().min_w_0().child(notice_line(notice)))
                .into_any_element(),
        )
    }

    fn render_chips(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.uploads.is_empty() {
            return None;
        }
        let mut row = div()
            .flex()
            .flex_wrap()
            .gap(px(space::SM))
            .mb(px(space::MD));
        for (i, u) in self.uploads.iter().enumerate() {
            let path = u.path.clone();
            row = row.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::XS))
                    .h(px(leading::BODY))
                    .max_w(px(MENTIONS_WIDTH))
                    .pl(px(space::SM))
                    .pr(px(space::HAIR))
                    .rounded(px(radius::SM))
                    .text_size(px(size::META))
                    .line_height(px(leading::META))
                    .bg(theme.c("chip"))
                    .text_color(theme.c("chip-ink"))
                    .child(ui::icon(
                        "file-text",
                        icon_size::INLINE,
                        theme.c("chip-ink"),
                    ))
                    .child(div().min_w_0().truncate().child(u.name.clone()))
                    .child(
                        focusable(
                            div()
                                .id(("composer-upload-remove", i))
                                .flex()
                                .flex_none()
                                .items_center()
                                .justify_center()
                                .size(px(leading::SMALL))
                                .rounded(px(radius::SM))
                                .cursor_pointer()
                                .hover(|s| s.bg(theme.c("hover-2"))),
                            true,
                            theme,
                        )
                        .child(ui::icon("x", icon_size::INLINE, theme.c("chip-ink")))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.uploads.retain(|x| x.path != path);
                            cx.notify();
                        })),
                    ),
            );
        }
        Some(row.into_any_element())
    }

    fn render_foot(&self, theme: &Theme) -> Option<AnyElement> {
        if let Some(error) = &self.error {
            return Some(
                ui::text(
                    error.clone(),
                    size::SMALL,
                    weight::REGULAR,
                    theme.c("red-ink"),
                )
                .mt(px(space::SM))
                .into_any_element(),
            );
        }
        let hint = self.state.hint.as_ref()?;
        Some(
            ui::text(hint.clone(), size::META, weight::REGULAR, theme.c("ink-3"))
                .mt(px(space::SM))
                .into_any_element(),
        )
    }

    /// The field with the placeholder drawn in the console's ink.
    fn render_field(&self, empty: bool, theme: &Theme) -> AnyElement {
        // The smallest kit size pads the text 4px on each side and none above:
        // pull it out by those 4px so the text starts where the box says.
        div()
            .flex_1()
            .min_w_0()
            .relative()
            .flex()
            .flex_col()
            .child(
                Textarea::new(&self.input)
                    .appearance(false)
                    .with_size(Size::XSmall)
                    .mx(px(-space::XS))
                    .aria_label(self.placeholder.clone())
                    .font_family(SANS)
                    .text_size(px(size::BODY))
                    .line_height(px(LINE_HEIGHT))
                    .text_color(theme.c("ink")),
            )
            .when(empty, |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .font_family(SANS)
                        .text_size(px(size::BODY))
                        .line_height(px(LINE_HEIGHT))
                        .text_color(theme.c("ink-4"))
                        .truncate()
                        .child(self.placeholder.clone()),
                )
            })
            .into_any_element()
    }
}

/// Why serve refused, in the composer's words (status and `reason` only).
fn refusal(e: &crate::api::ApiError, cx: &App) -> String {
    let reason = e
        .body
        .get("reason")
        .and_then(serde_json::Value::as_str)
        .or(e.code.as_deref());
    reply_error(e.status, reason, cx)
}

/// What the browser would have sent as the file's type.
fn mime_for(name: &str) -> &'static str {
    let ext = name
        .rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "md" | "markdown" => "text/markdown",
        "txt" | "log" => "text/plain",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "csv" => "text/csv",
        _ => "application/octet-stream",
    }
}

/// Read a picked file and store it through serve (`POST /attachments?name=`).
/// The error is serve's `message`, else its `error`, else `HTTP {status}`.
fn upload(path: &std::path::Path, name: &str) -> Result<Upload, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let answer = crate::api::post_bytes(
        &format!("/attachments?name={}", crate::api::enc(name)),
        &bytes,
        mime_for(name),
    )
    .map_err(|e| {
        let field = |k: &str| {
            e.body
                .get(k)
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
        };
        field("message")
            .or_else(|| field("error"))
            .or_else(|| e.status.map(|s| format!("HTTP {s}")))
            .unwrap_or(e.message)
    })?;
    let stored = answer
        .get("path")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    if stored.is_empty() {
        return Err("HTTP 200".into());
    }
    Ok(Upload {
        path: stored.to_string(),
        name: answer
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or(name)
            .to_string(),
    })
}

impl Render for Composer {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let boxed = self.state.boxed;
        self.focused = self.input.focus_handle(cx).is_focused(window);

        if let Some(blocked) = self.state.blocked.clone() {
            return div()
                .flex_none()
                .when(!boxed, |d| d.px(px(space::GUTTER)).pb(px(space::XL)))
                .child(
                    div()
                        .p(px(space::LG))
                        .border_1()
                        .border_dashed()
                        .border_color(theme.c("line-strong"))
                        .rounded(px(radius::MD))
                        .text_color(theme.c("ink-3"))
                        .font_family(SANS)
                        .text_size(px(size::UI))
                        .line_height(px(leading::UI))
                        .child(format!("{blocked}.")),
                )
                .into_any_element();
        }

        let text = self.text(cx);
        let empty = text.is_empty();
        let ready = !full_body(&text, &self.paths()).is_empty();
        let (_, shown) = self.mentions(cx);
        let wait = self.render_wait(&theme, cx);
        let chips = self.render_chips(&theme, cx);
        let foot = self.render_foot(&theme);
        let mentions = (!shown.is_empty()).then(|| self.render_mentions(&shown, &theme, cx));

        let root = div()
            .id("composer")
            .flex_none()
            .relative()
            .flex()
            .flex_col()
            .font_family(SANS)
            .text_size(px(size::BODY))
            .line_height(px(leading::BODY))
            .text_color(theme.c("ink"))
            .capture_action(cx.listener(Self::key_enter))
            .capture_action(cx.listener(Self::key_tab))
            .capture_action(cx.listener(Self::key_escape))
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| this.key_move(false, cx)))
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| this.key_move(true, cx)))
            // The caret moved without an edit: re-read the `@word` under it.
            .on_key_up(cx.listener(|_, _, _, cx| cx.notify()))
            .on_mouse_up(MouseButton::Left, cx.listener(|_, _, _, cx| cx.notify()));

        // The field's keyboard focus: the edge in `focus-line` and a ring.
        let focus_line = theme.c("focus-line");
        let rest_line = theme.c("line-strong");
        let field_ring = move |focused: bool, d: gpui::Stateful<gpui::Div>| {
            if focused {
                d.border_color(focus_line).shadow(vec![gpui::BoxShadow {
                    color: focus_line,
                    offset: gpui::point(px(0.), px(0.)),
                    blur_radius: px(0.),
                    spread_radius: px(1.),
                    inset: false,
                }])
            } else {
                d.border_color(rest_line)
            }
        };

        if boxed {
            let primary_ok = !self.busy && ready;
            let actions = self.actions.clone().map(|f| f(window, cx));
            return root
                .children(wait)
                .children(mentions)
                .child(
                    div()
                        .id("composer-box-field")
                        .w_full()
                        .min_h(px(BOX_MIN_HEIGHT))
                        .p(px(space::LG))
                        .rounded(px(radius::MD))
                        .border_1()
                        .map(|d| field_ring(self.focused, d))
                        .bg(theme.c("surface"))
                        .cursor_text()
                        .on_click(cx.listener(|this, _, window, cx| this.focus(window, cx)))
                        .child(self.render_field(empty, &theme)),
                )
                .when_some(chips, |d, c| d.child(div().mt(px(space::MD)).child(c)))
                .children(foot)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::MD))
                        .mt(px(space::XL))
                        .child(
                            focusable(
                                div()
                                    .id("composer-reply")
                                    .h(px(control::MD))
                                    .px(px(space::XL))
                                    .flex()
                                    .flex_none()
                                    .items_center()
                                    .rounded(px(radius::BASE))
                                    .bg(theme.c("accent-deep"))
                                    .text_color(theme.c("accent-ink"))
                                    .text_size(px(size::UI))
                                    .line_height(px(leading::UI))
                                    .font_weight(weight::STRONG)
                                    .tooltip(tip_keys(t(cx, "messages.reply"), &["⌘", "↵"])),
                                false,
                                &theme,
                            )
                            .when(!primary_ok, |d| d.opacity(0.5))
                            .when(primary_ok, |d| {
                                d.cursor_pointer()
                                    .hover(|s| s.bg(theme.c("accent-hover")))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.send(false, window, cx)
                                    }))
                            })
                            .child(t(cx, "messages.reply")),
                        )
                        .children(actions),
                )
                .into_any_element();
        }

        let plus_open = self.plus != Plus::Closed;
        let plus_menu = plus_open.then(|| self.render_plus_menu(&theme, cx));
        let send_ready = ready;
        let busy = self.busy;
        // The box starts at the gutter; its two buttons stay on the last
        // line as the text grows.
        root.px(px(space::GUTTER))
            .pb(px(space::XL))
            .children(wait)
            .children(chips)
            .child(
                div()
                    .id("composer-box")
                    .flex()
                    .items_end()
                    .gap(px(space::SM))
                    .p(px(space::SM))
                    .rounded(px(radius::MD))
                    .border_1()
                    .bg(theme.c("surface"))
                    .map(|d| field_ring(self.focused, d))
                    .child(
                        div()
                            .id("composer-plus-wrap")
                            .relative()
                            .flex_none()
                            .size(px(control::MD))
                            .when(plus_open, |d| {
                                d.on_mouse_down_out(cx.listener(|this, _, _, cx| {
                                    this.plus = Plus::Closed;
                                    this.plus_at = None;
                                    cx.notify();
                                }))
                            })
                            .child(
                                focusable(
                                    div()
                                        .id("composer-plus")
                                        .size(px(control::MD))
                                        .rounded(px(radius::BASE))
                                        .flex()
                                        .items_center()
                                        .justify_center()
                                        .cursor_pointer()
                                        .when(plus_open, |d| d.bg(theme.c("control")))
                                        .hover(|s| s.bg(theme.c("control")))
                                        .tooltip(tip(t(cx, "messages.add"))),
                                    false,
                                    &theme,
                                )
                                .child(ui::icon("plus", icon_size::RAIL, theme.c("ink-3")))
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.toggle_plus(window, cx)),
                                ),
                            )
                            .children(plus_menu),
                    )
                    .child(
                        div()
                            .id("composer-field")
                            .flex_1()
                            .min_w_0()
                            .min_h(px(control::MD))
                            .flex()
                            .items_center()
                            .cursor_text()
                            .on_click(cx.listener(|this, _, window, cx| this.focus(window, cx)))
                            .child(self.render_field(empty, &theme)),
                    )
                    // K5: the send key is shown once, in this tooltip.
                    .child(
                        focusable(
                            div()
                                .id("composer-send")
                                .flex_none()
                                .size(px(control::MD))
                                .rounded(px(radius::BASE))
                                .flex()
                                .items_center()
                                .justify_center()
                                .when(send_ready, |d| d.bg(theme.c("send")))
                                .tooltip(tip_keys(t(cx, "messages.send"), &["↵"])),
                            false,
                            &theme,
                        )
                        .child(ui::icon(
                            "send-horizontal",
                            icon_size::RAIL,
                            if send_ready {
                                theme.c("send-ink")
                            } else {
                                theme.c("ink-4")
                            },
                        ))
                        .when(!busy && send_ready, |d| d.cursor_pointer())
                        .when(!busy, |d| {
                            d.on_click(
                                cx.listener(|this, _, window, cx| this.send(false, window, cx)),
                            )
                        }),
                    ),
            )
            .children(foot)
            .children(mentions)
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachments_carry_the_type_a_browser_would_send() {
        assert_eq!(mime_for("shot.PNG"), "image/png");
        assert_eq!(mime_for("notes.md"), "text/markdown");
        assert_eq!(mime_for("change.diff"), "application/octet-stream");
        assert_eq!(mime_for("README"), "application/octet-stream");
    }

    #[test]
    fn a_gate_becomes_the_composer_state() {
        let gate = ComposerGate {
            notice: Some("wait".into()),
            ..Default::default()
        };
        let state = ComposerState::from_gate(gate);
        assert_eq!(state.notice.as_deref(), Some("wait"));
        assert!(state.blocked.is_none() && !state.boxed);
    }
}
