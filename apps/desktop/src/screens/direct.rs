//! Direct messages (`/direct`, `/direct/<agent>`; `direct-messages.tsx`):
//! the agent list on the left (420px), one agent's conversation with its
//! composer in the middle, and its Profile on the right (400px). An agent
//! the shell no longer holds opens read-only from History; "Watch live"
//! puts the agent's live view in place of the conversation.

use crate::chat::RowMenuItem;
use crate::chat::{self, ComposeTarget, Composer, ComposerState, MessageOpts};
use crate::derive::direct::{
    Mode, agent_relation, composer_block, composer_confirm, composer_notice, conversation,
    delivery_reason, direct_agents, direct_mode, dm_hint, ending_text, hide_action, name_origin,
    note_reader, operator_name, span_text,
};
use crate::derive::{agent_name, dot_tone, presence_line, presence_text};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::inbox::HistoryConversation;
use crate::model::{Agent, Message, WorkspaceView};
use crate::nav::{self, Nav, Route};
use crate::screens::inbox::{self as parts, ListKey, RowState};
use crate::screens::live::LiveView;
use crate::theme::Theme;
use crate::ui::agent_card::{StatusStyle, agent_hover, post, second_tick, status_line};
use crate::ui::format::{hhmm, short_path};
use crate::ui::{
    self, MONO, avatar_size, control, icon, icon_size, leading, radius, size, space, weight,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, Div, Entity, FocusHandle, Focusable as _,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Render, ScrollHandle,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, div, px,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use serde_json::json;
use std::rc::Rc;
use std::sync::Arc;

/// The Profile column's width.
const PROFILE_W: f32 = 400.;

/// What History answered for an agent the shell no longer has.
enum Archive {
    Loading,
    Found(Arc<HistoryConversation>),
    Missing,
    Failed(String),
}

pub struct DirectsScreen {
    query: Entity<InputState>,
    show_finished: bool,
    hidden_open: bool,
    /// `?profile=1` is read once; the button toggles it after.
    profile: bool,
    profile_read_for: Option<String>,
    composer: Entity<Composer>,
    archive: Option<(String, Archive)>,
    archive_task: Option<Task<()>>,
    live: Option<(String, Entity<LiveView>)>,
    /// The last read mark posted on opening, so it is posted once.
    marked: Option<(String, u64)>,
    stream_scroll: ScrollHandle,
    stream_last: Option<String>,
    rename: Option<(String, Entity<InputState>)>,
    rename_error: Option<SharedString>,
    votes: Option<(String, &'static str)>,
    /// The list's focus: its keys act while it holds it.
    focus: FocusHandle,
    /// The keyboard's focus row, an index into `row_keys`.
    cursor: Option<usize>,
    /// The agents the list shows, in order; the first `open_rows` are not finished.
    row_keys: Vec<String>,
    open_rows: usize,
    list_scroll: ScrollHandle,
    _tick: Task<()>,
    _subs: Vec<Subscription>,
}

fn route_parts(cx: &App) -> (Option<String>, bool, bool) {
    match Nav::route(cx) {
        Route::Direct {
            agent,
            profile,
            live,
        } => (agent.clone(), *profile, *live),
        _ => (None, false, false),
    }
}

impl DirectsScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query =
            cx.new(|cx| InputState::new(window, cx).placeholder(t(cx, "direct.searchAgents")));
        let composer = cx.new(|cx| Composer::new(ComposeTarget::Agent(String::new()), window, cx));
        let focus = cx.focus_handle();
        window.focus(&focus, cx);
        cx.set_global(parts::ListFocus(focus.clone()));
        let mut subs = vec![
            cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()),
            cx.observe_global::<Nav>(|_, cx| cx.notify()),
            cx.observe_global::<Theme>(|_, cx| cx.notify()),
            cx.subscribe(&query, |_, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    cx.notify();
                }
            }),
        ];
        // Esc marks the open conversation read up to now.
        subs.push(cx.observe_keystrokes(|this, ev, _, cx| {
            let k = &ev.keystroke;
            if k.key == "escape" && !k.modifiers.shift && this.rename.is_none() {
                let (agent, _, _) = route_parts(cx);
                if let Some(agent) = agent
                    && Desk::get(cx)
                        .read(cx)
                        .view
                        .as_ref()
                        .is_some_and(|v| v.agent(&agent).is_some())
                {
                    let now = harbard_core::clock::now_ms();
                    post("read", json!({ "agent": agent, "ts_ms": now }), cx).detach();
                }
            }
        }));
        DirectsScreen {
            query,
            show_finished: false,
            hidden_open: false,
            profile: false,
            profile_read_for: None,
            composer,
            archive: None,
            archive_task: None,
            live: None,
            marked: None,
            stream_scroll: ScrollHandle::new(),
            stream_last: None,
            rename: None,
            rename_error: None,
            votes: None,
            focus,
            cursor: None,
            row_keys: Vec::new(),
            open_rows: 0,
            list_scroll: ScrollHandle::new(),
            _tick: second_tick(window, cx),
            _subs: subs,
        }
    }

    /// Ask History for an agent the shell does not hold (once per key).
    fn look_up(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.archive.as_ref().is_some_and(|(k, _)| k == key) {
            return;
        }
        self.archive = Some((key.to_string(), Archive::Loading));
        let desk = Desk::get(cx).read(cx);
        let store = desk.hub.store().clone();
        let query = match desk.workspace.as_deref() {
            Some(ws) if ws != "all" => format!("ws={}", crate::api::enc(ws)),
            _ => String::new(),
        };
        let path = format!("/api/v1/history/agents/{}", crate::api::enc(key));
        let key = key.to_string();
        self.archive_task = Some(cx.spawn(async move |this, cx| {
            let answer = cx
                .background_spawn(async move {
                    harbard_cli::serve::history_route::conversation_body(
                        &store,
                        harbard_core::query::DEFAULT_STALE_SECS,
                        &path,
                        &query,
                    )
                })
                .await;
            let found = match answer {
                Ok(body) => match serde_json::from_value::<HistoryConversation>(body) {
                    Ok(conv) => Archive::Found(Arc::new(conv)),
                    Err(e) => Archive::Failed(e.to_string()),
                },
                Err((404, _)) => Archive::Missing,
                Err((_, body)) => Archive::Failed(
                    body.get("error")
                        .and_then(|e| e.as_str())
                        .unwrap_or("History did not answer")
                        .to_string(),
                ),
            };
            let _ = this.update(cx, |this, cx| {
                if this.archive.as_ref().is_some_and(|(k, _)| *k == key) {
                    this.archive = Some((key, found));
                    cx.notify();
                }
            });
        }));
    }

    fn start_rename(&mut self, agent: &Agent, window: &mut Window, cx: &mut Context<Self>) {
        if agent.name_source.is_none() {
            return;
        }
        let name = agent.name.clone();
        let input = cx.new(|cx| {
            let mut s = InputState::new(window, cx);
            s.set_value(name.clone(), window, cx);
            s
        });
        let key = agent.key.clone();
        let sub = cx.subscribe_in(
            &input,
            window,
            move |this, input, ev: &InputEvent, window, cx| match ev {
                InputEvent::PressEnter { .. } | InputEvent::Blur => {
                    let value = input.read(cx).value().trim().to_string();
                    this.finish_rename(&key, value, window, cx);
                }
                _ => {}
            },
        );
        self._subs.push(sub);
        input.update(cx, |s, cx| s.focus(window, cx));
        self.rename = Some((agent.key.clone(), input));
        self.rename_error = None;
        cx.notify();
    }

    fn finish_rename(
        &mut self,
        key: &str,
        value: String,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.rename.as_ref().is_none_or(|(k, _)| k != key) {
            return;
        }
        self.rename = None;
        cx.notify();
        let unchanged = Desk::get(cx)
            .read(cx)
            .view
            .as_ref()
            .and_then(|v| v.agent(key).map(|a| a.name == value))
            .unwrap_or(true);
        if value.is_empty() || unchanged {
            return;
        }
        let task = post("rename-agent", json!({ "agent": key, "name": value }), cx);
        cx.spawn(async move |this, cx| {
            if let Err(e) = task.await {
                let _ = this.update(cx, |this, cx| {
                    this.rename_error = Some(if e.status == Some(409) {
                        t(cx, "messages.nameTaken")
                    } else {
                        t(cx, "messages.nameSaveFailed")
                    });
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        self.rename = None;
        cx.notify();
    }

    /// The keys of the list (K4), while the list itself holds the focus; from
    /// the search field, the down arrow steps into the list and Esc leaves it.
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = parts::list_key(ev) else {
            return;
        };
        if self.query.focus_handle(cx).is_focused(window) {
            let arrow = ev.keystroke.key == "down";
            if arrow || key == ListKey::Back {
                window.focus(&self.focus, cx);
                if arrow {
                    self.move_cursor(Some(0));
                }
                cx.stop_propagation();
                cx.notify();
            }
            return;
        }
        if !self.focus.is_focused(window) {
            return;
        }
        match key {
            ListKey::Next | ListKey::Prev => {
                let (agent, _, _) = route_parts(cx);
                let from = self
                    .cursor
                    .or_else(|| self.row_keys.iter().position(|k| Some(k) == agent.as_ref()));
                self.move_cursor(parts::step(from, key, self.row_keys.len()));
            }
            ListKey::Open => {
                if let Some(key) = self.cursor.and_then(|ix| self.row_keys.get(ix)).cloned() {
                    nav::navigate(Route::direct(Some(key)), cx);
                }
            }
            // The keystroke observer marks the open conversation read on Esc.
            ListKey::Back => self.cursor = None,
            ListKey::Read => {
                let (agent, _, _) = route_parts(cx);
                let key = self
                    .cursor
                    .and_then(|ix| self.row_keys.get(ix).cloned())
                    .or(agent);
                if let Some(agent) = key {
                    let now = harbard_core::clock::now_ms();
                    post("read", json!({ "agent": agent, "ts_ms": now }), cx).detach();
                }
            }
        }
        cx.stop_propagation();
        cx.notify();
    }

    /// Put the focus row on a row and bring it into view. The fold toggle
    /// sits between the open rows and the finished ones.
    fn move_cursor(&mut self, to: Option<usize>) {
        self.cursor = to;
        if let Some(ix) = to {
            let child = if ix < self.open_rows { ix } else { ix + 1 };
            self.list_scroll.scroll_to_item(child);
        }
    }
}

impl Render for DirectsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let (route_agent, route_profile, route_live) = route_parts(cx);
        let desk = Desk::get(cx).read(cx);
        let now = desk.now_ms();
        let Some(view) = desk.view.clone() else {
            return div()
                .size_full()
                .bg(theme.c("surface"))
                .child(parts::loading(cx))
                .into_any_element();
        };
        // `?profile=1` opens the pane once for the agent it came with.
        if self.profile_read_for != route_agent {
            self.profile_read_for = route_agent.clone();
            if route_profile {
                self.profile = true;
            }
        }
        let selected: Option<Agent> = route_agent.as_deref().and_then(|k| view.agent(k)).cloned();
        if let (Some(key), None) = (&route_agent, &selected) {
            self.look_up(key, cx);
        }
        let archived = match (&route_agent, &self.archive) {
            (Some(k), Some((ak, a))) if k == ak => match a {
                Archive::Loading => None,
                Archive::Found(_) => Some(true),
                Archive::Missing | Archive::Failed(_) => Some(false),
            },
            _ => None,
        };
        let mode = direct_mode(selected.as_ref(), archived);

        // Opening a conversation with unread messages marks them read, up to its last word.
        if let Some(a) = &selected {
            let last = conversation(&view, &a.key)
                .iter()
                .rev()
                .find(|m| m.from == "agent")
                .map_or(0, |m| m.ts_ms);
            if a.unread > 0 && last > 0 && self.marked != Some((a.key.clone(), last)) {
                self.marked = Some((a.key.clone(), last));
                post("read", json!({ "agent": a.key, "ts_ms": last }), cx).detach();
            }
        }

        let list = self.agent_list(&view, route_agent.as_deref(), window, &theme, cx);
        let detail = match (&selected, route_live) {
            (Some(a), true) => {
                if self.live.as_ref().is_none_or(|(k, _)| *k != a.key) {
                    let key = a.key.clone();
                    let live = cx.new(|cx| LiveView::new(key.clone(), window, cx));
                    self.live = Some((key, live));
                }
                self.live
                    .as_ref()
                    .map(|(_, v)| v.clone().into_any_element())
                    .unwrap_or_else(|| div().into_any_element())
            }
            (Some(a), false) => {
                self.live = None;
                self.conversation(a, &view, mode, now, window, &theme, cx)
            }
            (None, _) => {
                self.live = None;
                match (&route_agent, &self.archive) {
                    (Some(k), Some((ak, Archive::Found(conv)))) if k == ak => {
                        let conv = conv.clone();
                        self.archived(&conv, &view, &theme, cx)
                    }
                    (Some(_), _) if mode == Mode::Loading => detail_main(&theme)
                        .child(parts::empty(
                            None,
                            Some(t(cx, "direct.searchingHistory")),
                            &theme,
                        ))
                        .into_any_element(),
                    (Some(k), archive) => {
                        let error = match archive {
                            Some((ak, Archive::Failed(e))) if ak == k => {
                                Some(SharedString::from(e.clone()))
                            }
                            _ => None,
                        };
                        detail_main(&theme)
                            .child(parts::empty(
                                Some(t(cx, "direct.notInLastMonth")),
                                Some(error.unwrap_or_else(|| t(cx, "direct.agentNotFound"))),
                                &theme,
                            ))
                            .into_any_element()
                    }
                    (None, _) => detail_main(&theme)
                        .child(parts::empty(
                            Some(t(cx, "direct.chooseAgent")),
                            Some(t(cx, "direct.chooseAgentHint")),
                            &theme,
                        ))
                        .into_any_element(),
                }
            }
        };
        let profile = match &selected {
            Some(a) if self.profile && !route_live => {
                Some(self.profile_pane(a, &view, now, &theme, cx))
            }
            _ => None,
        };
        div()
            .id("hb-direct")
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .size_full()
            .flex()
            .min_h_0()
            .child(list)
            .child(detail)
            .children(profile)
            .into_any_element()
    }
}

/// `main.hb-detail`.
fn detail_main(theme: &Theme) -> Div {
    div()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .flex()
        .flex_col()
        .bg(theme.c("surface"))
}

/// Two facts on one line.
fn joined(parts: [Option<SharedString>; 2]) -> Option<SharedString> {
    let said: Vec<String> = parts
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();
    (!said.is_empty()).then(|| said.join(" · ").into())
}

impl DirectsScreen {
    fn agent_list(
        &mut self,
        view: &Arc<WorkspaceView>,
        active: Option<&str>,
        window: &mut Window,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let q = self.query.read(cx).value().to_string();
        let searching = !q.trim().is_empty();
        let list = direct_agents(view, &q);
        let (open, finished): (Vec<&Agent>, Vec<&Agent>) =
            list.iter().partition(|a| a.presence != "finished");
        let unfolded = self.show_finished || searching;
        // The rows the keyboard walks, in the order they are drawn.
        let keys: Vec<String> = open
            .iter()
            .chain(finished.iter().filter(|_| unfolded))
            .map(|a| a.key.clone())
            .collect();
        if keys != self.row_keys {
            self.cursor = self.cursor.filter(|ix| *ix < keys.len());
            self.row_keys = keys;
        }
        self.open_rows = open.len();
        let head = parts::list_head(t(cx, "direct.title"), theme)
            .child(if view.unread.directs > 0 {
                parts::head_note(
                    tv(
                        cx,
                        "direct.unreadCount",
                        &[("count", &view.unread.directs.to_string())],
                    ),
                    theme,
                )
            } else {
                div().ml_auto()
            })
            .child(
                parts::icon_btn("hb-dm-new", "square-pen", false, theme)
                    .tooltip(parts::tip(t(cx, "direct.newMessage"), &[]))
                    .on_click(|_, _, cx| nav::navigate(Route::direct(None), cx)),
            );
        let rows: Vec<AnyElement> = open
            .iter()
            .enumerate()
            .map(|(ix, a)| self.direct_row(view, a, ix, active, theme, cx))
            .collect();
        let finished_rows: Vec<AnyElement> = if unfolded {
            finished
                .iter()
                .enumerate()
                .map(|(ix, a)| self.direct_row(view, a, open.len() + ix, active, theme, cx))
                .collect()
        } else {
            Vec::new()
        };
        let hidden = if searching {
            None
        } else {
            parts::hidden_group(
                view,
                self.hidden_open,
                {
                    let this = cx.entity().downgrade();
                    move |_, cx| {
                        let _ = this.update(cx, |this, cx| {
                            this.hidden_open = !this.hidden_open;
                            cx.notify();
                        });
                    }
                },
                cx,
            )
        };
        let searching_now = self.query.focus_handle(cx).is_focused(window);
        // A child per row, then the fold toggle, then the finished rows: the
        // keyboard scrolls to a row by its place here (`move_cursor`).
        let fold = if finished.is_empty() {
            // Keeps the finished rows' places when there is no toggle.
            div().into_any_element()
        } else {
            parts::fold_toggle(
                "hb-dm-finished",
                tv(
                    cx,
                    "direct.finishedCount",
                    &[("count", &finished.len().to_string())],
                ),
                unfolded,
                theme,
            )
            .on_click(cx.listener(|this, _, _, cx| {
                this.show_finished = !this.show_finished;
                cx.notify();
            }))
            .into_any_element()
        };
        parts::list_pane(theme)
            .child(head)
            .child(
                div()
                    .flex_none()
                    .px(px(space::GUTTER))
                    .py(px(space::MD))
                    .child(parts::search_box(&self.query, searching_now, theme)),
            )
            .child(
                div()
                    .id("hb-dm-list")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.list_scroll)
                    .border_t_1()
                    .border_color(theme.c("line"))
                    .children(rows)
                    .child(fold)
                    .children(finished_rows)
                    .when(list.is_empty(), |d| {
                        d.child(parts::empty(
                            None,
                            Some(t(cx, "direct.noAgentsMatch")),
                            theme,
                        ))
                    })
                    .children(hidden),
            )
            .into_any_element()
    }

    /// One conversation in the list (`DirectRow`): the name and what the
    /// agent is, its live status, and its last words on one line.
    fn direct_row(
        &self,
        view: &WorkspaceView,
        a: &Agent,
        ix: usize,
        active: Option<&str>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let last: Option<&Message> = a.last_message.as_deref().and_then(|id| view.message(id));
        let key = a.key.clone();
        let unread = a.unread > 0;
        // Unread shows its count; a dot would say it twice (F2).
        let right = if unread {
            Some(parts::count(a.unread, theme).ml_auto())
        } else {
            last.map(|m| parts::item_time(hhmm(m.ts_ms), theme))
        };
        let preview: SharedString = match last {
            Some(m) if m.from == "agent" => m.text.clone().into(),
            Some(m) if m.from == "operator" => {
                format!("{}: {}", operator_name(&m.author, cx), m.text).into()
            }
            Some(m) => format!("{}: {}", m.author, m.text).into(),
            None => a
                .description
                .clone()
                .map(Into::into)
                .unwrap_or_else(|| presence_text(&a.presence, cx)),
        };
        let state = RowState {
            active: active == Some(key.as_str()),
            cursor: self.cursor == Some(ix),
            dot: false,
        };
        let row = parts::item_row(SharedString::from(format!("hb-dm-row-{key}")), state, theme)
            .on_click(cx.listener({
                let key = key.clone();
                move |this, _, _, cx| {
                    this.cursor = None;
                    nav::navigate(Route::direct(Some(key.clone())), cx)
                }
            }))
            .child(ui::avatar::agent_avatar(a, avatar_size::ROW, true, cx))
            .child(
                parts::item_body()
                    .child(
                        parts::item_line()
                            .child(parts::item_title(agent_name(a, cx), unread, theme).flex_none())
                            .children(role_line(a, cx).map(|r| parts::item_aside(r, theme)))
                            .children(right),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .h(px(leading::SMALL))
                            .overflow_hidden()
                            .child(chat::agent_status_line(
                                a,
                                Desk::get(cx).read(cx).now_ms(),
                                cx,
                            )),
                    )
                    .child(parts::item_preview(preview, 1, theme)),
            );
        let (hide_label, hide_action_name) = hide_action(a, cx);
        let read_key = key.clone();
        let hide_key = key.clone();
        let items = vec![
            RowMenuItem {
                label: t(cx, "messages.markRead"),
                icon: None,
                action: Rc::new(move |_, cx: &mut App| {
                    let now = harbard_core::clock::now_ms();
                    post("read", json!({ "agent": read_key, "ts_ms": now }), cx).detach();
                }),
            },
            RowMenuItem {
                label: hide_label,
                icon: None,
                action: Rc::new(move |_, cx: &mut App| {
                    post(hide_action_name, json!({ "agent": hide_key }), cx).detach();
                }),
            },
        ];
        let menu = parts::row_menu(
            SharedString::from(format!("hb-dm-menu-{key}")),
            row,
            items,
            cx,
        );
        agent_hover(
            SharedString::from(format!("hb-dm-hover-{key}")),
            &key,
            true,
            menu,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn conversation(
        &mut self,
        a: &Agent,
        view: &Arc<WorkspaceView>,
        mode: Mode,
        now: u64,
        window: &mut Window,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let messages = conversation(view, &a.key);
        let last_id = messages.last().map(|m| m.id.clone());
        if last_id != self.stream_last {
            self.stream_last = last_id;
            self.stream_scroll.scroll_to_bottom();
        }
        let name = agent_name(a, cx);
        let status = match self.rename_error.clone() {
            Some(e) => div().child(ui::text(
                e,
                size::SMALL,
                weight::REGULAR,
                theme.c("red-ink"),
            )),
            None => div()
                .child(parts::presence_dot(
                    dot_tone(&a.presence, a.dormant.as_deref()),
                    theme,
                ))
                .child(if a.lost {
                    ui::text(
                        presence_line(a, now, cx),
                        size::SMALL,
                        weight::REGULAR,
                        theme.c("ink-3"),
                    )
                    .truncate()
                } else {
                    status_line(
                        a,
                        StatusStyle {
                            size: size::SMALL,
                            color: theme.c("ink-3"),
                            time_right: false,
                            show_time: true,
                        },
                        cx,
                    )
                }),
        };
        let key_live = a.key.clone();
        let head = parts::agent_head(
            ui::avatar::agent_avatar(a, avatar_size::ROW, false, cx),
            self.name_editable(a, theme, window, cx),
            joined([role_line(a, cx), Some(agent_relation(a, &view.agents, cx))]),
            status,
            theme,
        )
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(px(space::XS))
                .child(
                    parts::icon_btn("hb-dm-live", "radio", false, theme)
                        .tooltip(parts::tip(t(cx, "direct.watchLive"), &[]))
                        .on_click(move |_, _, cx| {
                            nav::replace(
                                Route::Direct {
                                    agent: Some(key_live.clone()),
                                    profile: false,
                                    live: true,
                                },
                                cx,
                            )
                        }),
                )
                .child(
                    parts::icon_btn("hb-dm-profile", "user-round", self.profile, theme)
                        .tooltip(parts::tip(t(cx, "direct.profile"), &[]))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.profile = !this.profile;
                            cx.notify();
                        })),
                ),
        );
        let read_only = mode == Mode::ReadOnly;
        let opts = MessageOpts {
            in_thread: true,
            fold: false,
            continued: false,
            dm: true,
            ..Default::default()
        };
        let refs: Vec<&Message> = messages.to_vec();
        let stream = div()
            .id("hb-dm-stream")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.stream_scroll)
            .pt(px(space::MD))
            .pb(px(space::LG))
            .when(refs.is_empty(), |d| {
                d.child(parts::empty(
                    None,
                    Some(tv(cx, "direct.noMessagesYet", &[("name", &name)])),
                    theme,
                ))
            })
            .when(!refs.is_empty(), |d| {
                d.child(chat::stream(&refs, view, &opts, cx))
            });
        let foot = if read_only {
            read_only_bar(None, theme, cx).into_any_element()
        } else {
            let target = ComposeTarget::Agent(a.key.clone());
            let placeholder = tv(cx, "direct.writeTo", &[("name", &name)]);
            self.composer
                .update(cx, |c, cx| c.set_target(target, placeholder, cx));
            let state = ComposerState {
                blocked: composer_block(&[a], cx),
                notice: composer_notice(&[a], cx),
                confirm: composer_confirm(&[a], cx),
                hint: Some(dm_hint(a, note_reader(&view.agents, &a.key), cx)),
                boxed: false,
            };
            self.composer.update(cx, |c, cx| c.set_state(state, cx));
            div()
                .flex_none()
                .child(self.composer.clone())
                .into_any_element()
        };
        detail_main(theme)
            .child(head)
            .child(stream)
            .child(foot)
            .into_any_element()
    }

    /// The name as the pane's title, editable with a double click when it
    /// has a workshop name.
    fn name_editable(
        &mut self,
        a: &Agent,
        theme: &Theme,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if let Some((key, input)) = &self.rename
            && *key == a.key
        {
            let input = input.clone();
            return div()
                .on_key_down(cx.listener(|this, ev: &KeyDownEvent, _, cx| {
                    if ev.keystroke.key == "escape" {
                        cx.stop_propagation();
                        this.cancel_rename(cx);
                    }
                }))
                .text_size(px(size::TITLE))
                .font_weight(weight::HEAVY)
                .rounded(px(radius::SM))
                .shadow(vec![ui::focus_ring(theme, false)])
                .child(Input::new(&input).appearance(false))
                .into_any_element();
        }
        let agent = a.clone();
        div()
            .id(SharedString::from(format!("hb-name-{}", a.key)))
            .flex_none()
            .max_w_full()
            .child(
                ui::text(
                    agent_name(a, cx),
                    size::TITLE,
                    weight::HEAVY,
                    theme.c("ink"),
                )
                .truncate(),
            )
            .when(a.name_source.is_some(), |d| {
                d.tooltip(parts::tip(t(cx, "messages.doubleClickRename"), &[]))
                    .on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
                        if ev.click_count() == 2 {
                            this.start_rename(&agent, window, cx);
                        }
                    }))
            })
            .into_any_element()
    }

    fn archived(
        &mut self,
        conv: &Arc<HistoryConversation>,
        view: &Arc<WorkspaceView>,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let a = &conv.item.agent;
        // The stream looks names and avatars up in a view: the shell's, with this agent and its messages.
        let mut copy: WorkspaceView = (**view).clone();
        copy.agents.retain(|x| x.key != a.key);
        copy.agents.insert(0, a.clone());
        copy.messages = conv.messages.clone();
        copy.saved = Vec::new();
        let role: Option<String> = role_line(a, cx).map(|s| s.to_string()).or_else(|| {
            let parts: Vec<String> = [a.role.clone(), a.team.clone()]
                .into_iter()
                .flatten()
                .collect();
            (!parts.is_empty()).then(|| parts.join(" · "))
        });
        let line: Vec<String> = [
            Some(ending_text(conv.item.ending.as_deref(), cx).to_string()),
            Some(
                span_text(
                    conv.item.start_ms,
                    conv.item.end_ms,
                    conv.item.duration_ms,
                    conv.now_ms,
                    cx,
                )
                .to_string(),
            ),
        ]
        .into_iter()
        .flatten()
        .collect();
        let head = parts::agent_head(
            ui::avatar::agent_avatar(a, avatar_size::ROW, false, cx),
            ui::text(a.name.clone(), size::TITLE, weight::HEAVY, theme.c("ink"))
                .truncate()
                .into_any_element(),
            role.map(Into::into),
            div().child(parts::presence_dot("grey", theme)).child(
                ui::text(
                    line.join(" · "),
                    size::SMALL,
                    weight::REGULAR,
                    theme.c("ink-3"),
                )
                .truncate(),
            ),
            theme,
        );
        let refs: Vec<&Message> = conv.messages.iter().collect();
        let opts = MessageOpts {
            in_thread: true,
            fold: false,
            continued: false,
            dm: true,
            ..Default::default()
        };
        let stream = div()
            .id("hb-dm-archived")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .pt(px(space::MD))
            .pb(px(space::LG))
            .when(refs.is_empty(), |d| {
                d.child(parts::empty(
                    None,
                    Some(tv(cx, "direct.noMessagesThisMonth", &[("name", &a.name)])),
                    theme,
                ))
            })
            .when(!refs.is_empty(), |d| {
                d.child(chat::stream(&refs, &copy, &opts, cx))
            });
        let worked = conv
            .item
            .channel
            .as_ref()
            .map(|c| tv(cx, "direct.workedIn", &[("channel", c)]));
        detail_main(theme)
            .child(head)
            .child(stream)
            .when(conv.truncated, |d| {
                d.child(
                    div()
                        .flex()
                        .justify_center()
                        .px(px(space::GUTTER))
                        .py(px(space::MD))
                        .child(ui::text(
                            t(cx, "direct.onlyRecent"),
                            size::SMALL,
                            weight::REGULAR,
                            theme.c("ink-3"),
                        )),
                )
            })
            .child(read_only_bar(worked, theme, cx))
            .into_any_element()
    }

    fn profile_pane(
        &mut self,
        a: &Agent,
        view: &Arc<WorkspaceView>,
        now: u64,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let field = |label: SharedString, value: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap(px(space::HAIR))
                .child(ui::section_label(label, theme))
                .child(value)
        };
        let text = |s: SharedString| {
            ui::text(s, size::UI, weight::REGULAR, theme.c("ink")).into_any_element()
        };
        // Mono is for what a person may copy, two pixels under the text (T6).
        let mono = |s: String| {
            ui::text(s, size::META, weight::REGULAR, theme.c("ink"))
                .font_family(MONO)
                .into_any_element()
        };
        let mut fields: Vec<AnyElement> = Vec::new();
        if let Some(origin) = name_origin(a, cx) {
            let calls = a
                .own_name
                .as_ref()
                .map(|n| format!(" · {}", tv(cx, "direct.callsItself", &[("name", n)])))
                .unwrap_or_default();
            fields.push(
                field(
                    t(cx, "direct.fieldName"),
                    text(format!("{origin}{calls}").into()),
                )
                .into_any_element(),
            );
        }
        fields.push(
            field(
                t(cx, "direct.fieldIdentifier"),
                div()
                    .flex()
                    .flex_col()
                    .child(mono(a.key.clone()))
                    .child(
                        ui::text(
                            a.label.clone(),
                            size::META,
                            weight::REGULAR,
                            theme.c("ink-3"),
                        )
                        .font_family(MONO),
                    )
                    .into_any_element(),
            )
            .into_any_element(),
        );
        let session_label = if a.kind == "session" {
            "direct.fieldSession"
        } else {
            "direct.fieldLeadSession"
        };
        fields.push(field(t(cx, session_label), mono(a.lead_session.clone())).into_any_element());
        if let Some(handle) = &a.handle {
            let team = a
                .team
                .as_ref()
                .map(|team| format!("· {}", tv(cx, "direct.teamMention", &[("team", team)])))
                .unwrap_or_default();
            fields.push(
                field(
                    t(cx, "direct.fieldMention"),
                    div()
                        .flex()
                        .items_baseline()
                        .gap(px(space::XS))
                        .child(mono(format!("@{handle}")))
                        .child(text(team.into()))
                        .into_any_element(),
                )
                .into_any_element(),
            );
        }
        let brief: SharedString = a
            .brief
            .clone()
            .or_else(|| a.description.clone())
            .map(Into::into)
            .unwrap_or_else(|| t(cx, "direct.noData"));
        fields.push(field(t(cx, "direct.fieldBrief"), text(brief)).into_any_element());
        if let (Some(desc), Some(_)) = (&a.description, &a.brief) {
            fields.push(
                field(t(cx, "direct.fieldTask"), text(desc.clone().into())).into_any_element(),
            );
        }
        fields.push(
            field(
                t(cx, "direct.fieldCopy"),
                match &a.checkout {
                    Some(c) => mono(short_path(c)),
                    None => text(t(cx, "direct.noCopy")),
                },
            )
            .into_any_element(),
        );
        fields.push(
            field(
                t(cx, "direct.fieldBranch"),
                match &a.branch {
                    Some(b) => mono(b.clone()),
                    None => text(t(cx, "direct.noData")),
                },
            )
            .into_any_element(),
        );
        if let Some(dir) = &a.build_dir {
            fields.push(field(t(cx, "direct.fieldBuildDir"), mono(dir.clone())).into_any_element());
        }
        fields.push(
            field(
                t(cx, "direct.fieldNow"),
                chat::agent_status_line(a, now, cx),
            )
            .into_any_element(),
        );
        let started = a
            .first_ms
            .filter(|ms| *ms > 0)
            .map(|ms| format!("; {}", tv(cx, "direct.startedAt", &[("time", &hhmm(ms))])))
            .unwrap_or_default();
        fields.push(
            field(
                t(cx, "direct.fieldStatus"),
                text(format!("{}{started}", presence_line(a, now, cx)).into()),
            )
            .into_any_element(),
        );
        let deliveries: SharedString = if a.deliveries == 0 {
            t(cx, "direct.deliveriesNone")
        } else {
            let d = tv(
                cx,
                if a.deliveries == 1 {
                    "direct.deliveryOne"
                } else {
                    "direct.deliveryMany"
                },
                &[("count", &a.deliveries.to_string())],
            );
            let v = tv(
                cx,
                if a.verified_deliveries == 1 {
                    "direct.verifiedOne"
                } else {
                    "direct.verifiedMany"
                },
                &[("count", &a.verified_deliveries.to_string())],
            );
            tv(
                cx,
                "direct.deliveriesLine",
                &[("deliveries", &d), ("verified", &v)],
            )
        };
        fields.push(field(t(cx, "direct.fieldDeliveries"), text(deliveries)).into_any_element());
        let reports = if a.reports.is_empty() {
            text(t(cx, "direct.reportsNone"))
        } else {
            div()
                .flex()
                .flex_col()
                .gap(px(space::XS))
                .children(a.reports.iter().map(|r| chat::file_card(r, cx)))
                .into_any_element()
        };
        fields.push(field(t(cx, "direct.fieldReports"), reports).into_any_element());
        let messages: SharedString = match a.delivery.as_str() {
            "ready" => dm_hint(a, note_reader(&view.agents, &a.key), cx),
            "stale" => tv(
                cx,
                "direct.messagesSavedAsNote",
                &[("notice", &composer_notice(&[a], cx).unwrap_or_default())],
            ),
            other => tv(
                cx,
                "direct.messagesSavedUnread",
                &[("reason", &delivery_reason(other, cx))],
            ),
        };
        fields.push(field(t(cx, "direct.fieldMessages"), text(messages)).into_any_element());

        let voted = self
            .votes
            .as_ref()
            .filter(|(k, _)| *k == a.key)
            .map(|(_, v)| *v);
        // "Do you like the name?", for a workshop name: two icon buttons.
        let votes = a.name_source.is_some().then(|| {
            let key_up = a.key.clone();
            let key_down = a.key.clone();
            div()
                .flex()
                .flex_none()
                .gap(px(space::HAIR))
                .child(
                    parts::icon_btn("hb-vote-up", "thumbs-up", voted == Some("up"), theme)
                        .tooltip(parts::tip(t(cx, "messages.nameUp"), &[]))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.votes = Some((key_up.clone(), "up"));
                            post("name-vote", json!({ "agent": key_up, "vote": "up" }), cx)
                                .detach();
                            cx.notify();
                        })),
                )
                .child(
                    parts::icon_btn("hb-vote-down", "thumbs-down", voted == Some("down"), theme)
                        .tooltip(parts::tip(t(cx, "messages.nameDown"), &[]))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.votes = Some((key_down.clone(), "down"));
                            post(
                                "name-vote",
                                json!({ "agent": key_down, "vote": "down" }),
                                cx,
                            )
                            .detach();
                            cx.notify();
                        })),
                )
        });

        let header = div()
            .flex()
            .items_center()
            .gap(px(space::LG))
            .child(ui::avatar::agent_avatar(a, avatar_size::ROW, true, cx))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .child(self.name_editable_small(a, theme, cx))
                    .child(
                        ui::text(
                            agent_relation(a, &view.agents, cx),
                            size::SMALL,
                            weight::REGULAR,
                            theme.c("ink-3"),
                        )
                        .truncate(),
                    ),
            )
            .children(votes);
        div()
            .w(px(PROFILE_W))
            .flex_none()
            .flex()
            .flex_col()
            .min_h_0()
            .border_l_1()
            .border_color(theme.c("line"))
            .bg(theme.c("surface"))
            .child(
                ui::pane_header(t(cx, "direct.profile"), theme).child(
                    parts::icon_btn("hb-profile-close", "x", false, theme)
                        .ml_auto()
                        .tooltip(parts::tip(t(cx, "direct.closeProfile"), &[]))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.profile = false;
                            cx.notify();
                        })),
                ),
            )
            .child(
                div()
                    .id("hb-profile")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p(px(space::GUTTER))
                    .flex()
                    .flex_col()
                    .gap(px(space::XL))
                    .child(header)
                    .children(fields),
            )
            .into_any_element()
    }

    /// The Profile's name line, editable like the header's.
    fn name_editable_small(
        &mut self,
        a: &Agent,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let name =
            ui::text(agent_name(a, cx), size::BODY, weight::HEAVY, theme.c("ink")).truncate();
        // One editor at a time: the header holds it.
        if self.rename.as_ref().is_some_and(|(k, _)| *k == a.key) {
            return name.into_any_element();
        }
        let agent = a.clone();
        div()
            .id(SharedString::from(format!("hb-prof-name-{}", a.key)))
            .min_w_0()
            .child(name)
            .when(a.name_source.is_some(), |d| {
                d.on_click(cx.listener(move |this, ev: &gpui::ClickEvent, window, cx| {
                    if ev.click_count() == 2 {
                        this.start_rename(&agent, window, cx);
                    }
                }))
            })
            .into_any_element()
    }
}

/// The agent's role line (`agentRoleLine`), when it has one.
fn role_line(a: &Agent, cx: &App) -> Option<SharedString> {
    let line = a.role_line.as_deref()?;
    Some(crate::i18n::serve_text(
        cx,
        a.role_line_code.as_deref(),
        a.role_line_params.as_ref(),
        line,
    ))
}

/// Where a read-only conversation has its composer (`ReadOnlyBar`).
fn read_only_bar(line: Option<SharedString>, theme: &Theme, cx: &App) -> Div {
    div()
        .flex_none()
        .px(px(space::GUTTER))
        .pb(px(space::XL))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(space::MD))
                .min_h(px(control::LG))
                .px(px(space::LG))
                .py(px(space::MD))
                .border_1()
                .border_dashed()
                .border_color(theme.c("line-strong"))
                .rounded(px(radius::MD))
                .bg(theme.c("surface-2"))
                .child(icon("archive", icon_size::ROW, theme.c("ink-3")))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .flex()
                        .flex_col()
                        .child(ui::text(
                            t(cx, "history.readOnlyNotice"),
                            size::SMALL,
                            weight::REGULAR,
                            theme.c("ink-2"),
                        ))
                        .children(
                            line.map(|l| {
                                ui::text(l, size::SMALL, weight::REGULAR, theme.c("ink-3"))
                            }),
                        ),
                )
                .child(
                    parts::quiet_btn(
                        "hb-readonly-history",
                        Some("rotate-ccw-clock"),
                        t(cx, "direct.history"),
                        theme,
                    )
                    .on_click(|_, _, cx| {
                        nav::navigate(
                            Route::History {
                                channel: None,
                                q: String::new(),
                            },
                            cx,
                        )
                    }),
                ),
        )
}
