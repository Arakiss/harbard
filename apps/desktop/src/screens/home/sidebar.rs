//! Home's sidebar (`Sidebar`, `home.tsx`): the workspace name, Lobby,
//! Threads, Deliveries, channels, main sessions, agents, temporary
//! sessions, rooms, and "More tools" at the foot.
//!
//! ```text
//! ┌ 56  Personal ⌄ ─────────────────┐  head: opens the workspace menu
//! │ Lobby · Threads · Deliveries    │  ┐
//! │ CHANNELS            32 px rows  │  │ scrolls; keeps its offset
//! │ MAIN SESSIONS / AGENTS ⌄        │  │ agents: 44 px, idle 32 px
//! │ ROOMS                        +  │  ┘
//! ├─────────────────────────────────┤
//! │ ▦ More tools                  › │  pinned; its list opens upwards
//! └─────────────────────────────────┘
//! ```

use crate::chat::row_menu::{RowAction, RowRun, row_menu};
use crate::derive::rooms::{is_room, sidebar_rooms};
use crate::derive::workspace::{self as ws, SidebarAgents};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{ActivityView, Agent, Channel, WorkspaceView};
use crate::nav::{self, Nav, Route};
use crate::robot::{self, Ring};
use crate::theme::Theme;
use crate::ui;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, ElementId, Entity, FontWeight, Global, Hsla, InteractiveElement,
    IntoElement, ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription,
    Window, div, px, relative,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use serde_json::json;
use std::rc::Rc;

pub struct Sidebar {
    scroll: ScrollHandle,
    /// "More channels (n)" open.
    more_channels: bool,
    /// "See all (n)": every active agent, not only the first eight.
    see_all: bool,
    inactive: bool,
    temporary: bool,
    archived: bool,
    hidden: bool,
    more_tools: bool,
    filter: Entity<InputState>,
    query: String,
    /// The Agents fold as drawn: flips at once on a click, serve's answer follows.
    agents_folded: bool,
    /// The desk's `agents_collapsed` last seen: a change re-syncs the fold.
    folded_seen: Option<bool>,
    _subs: Vec<Subscription>,
}

struct Shared(Entity<Sidebar>);

impl Global for Shared {}

/// The one sidebar every Home screen shows, so its folds and filter stay
/// as they were when moving between Home, Threads, Deliveries and Lobby.
pub fn shared(window: &mut Window, cx: &mut App) -> Entity<Sidebar> {
    if let Some(s) = cx.try_global::<Shared>() {
        return s.0.clone();
    }
    let sidebar = cx.new(|cx| Sidebar::new(window, cx));
    cx.set_global(Shared(sidebar.clone()));
    sidebar
}

/// The workspace menu (`openWorkspaceMenu`), anchored under the name.
// TODO(shell): call `crate::shell::open_workspace_menu(window, cx)` once the shell lane lands it.
pub fn open_workspace_menu(window: &mut Window, cx: &mut App) {
    let _ = (window, cx);
}

/// More tools → Strip (`DesktopViewLink target="strip"`).
// TODO(desktop): call `crate::native::open_strip(cx)` once the desktop lane lands it.
pub fn open_strip(window: &mut Window, cx: &mut App) {
    let _ = (window, cx);
}

/// The agent hover card (`AgentHover`) around a sidebar row; another lane
/// owns the card, so the row comes back as it is until then.
pub fn agent_hover(agent: &Agent, row: AnyElement, cx: &App) -> AnyElement {
    let _ = (agent, cx);
    row
}

impl Sidebar {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let placeholder = t(cx, "home.filterAgents");
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let desk = Desk::get(cx);
        let folded = desk.read(cx).view.as_ref().map(|v| v.agents_collapsed);
        let subs = vec![
            cx.observe(&desk, |this: &mut Sidebar, desk, cx| {
                let folded = desk.read(cx).view.as_ref().map(|v| v.agents_collapsed);
                if folded.is_some() && folded != this.folded_seen {
                    this.folded_seen = folded;
                    this.agents_folded = folded == Some(true);
                }
                cx.notify();
            }),
            cx.observe_global::<Nav>(|_, cx| cx.notify()),
            cx.observe_global::<Theme>(|_, cx| cx.notify()),
            cx.subscribe(&filter, |this: &mut Sidebar, input, ev: &InputEvent, cx| {
                if let InputEvent::Change = ev {
                    this.query = input.read(cx).value().to_string();
                    cx.notify();
                }
            }),
        ];
        Sidebar {
            scroll: ScrollHandle::new(),
            more_channels: false,
            see_all: false,
            inactive: false,
            temporary: false,
            archived: false,
            hidden: false,
            more_tools: false,
            filter,
            query: String::new(),
            agents_folded: folded == Some(true),
            folded_seen: folded,
            _subs: subs,
        }
    }

    /// Fold or unfold Agents on the desk: at once here, back if serve refuses.
    fn toggle_agents(&mut self, cx: &mut Context<Self>) {
        let next = !self.agents_folded;
        self.agents_folded = next;
        cx.notify();
        let task = Desk::get(cx).update(cx, |d, cx| d.post("agents-collapsed", json!({ "collapsed": next }), cx));
        cx.spawn(async move |this, cx| {
            if task.await.is_err() {
                let _ = this.update(cx, |this, cx| {
                    this.agents_folded = !next;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// A fold's click: flips one of the local folds.
    fn flip(
        &self,
        field: fn(&mut Sidebar) -> &mut bool,
        cx: &mut Context<Self>,
    ) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
        cx.listener(move |this, _: &ClickEvent, _, cx| {
            let f = field(this);
            *f = !*f;
            cx.notify();
        })
    }
}

// ---------------------------------------------------------------------------
// What the sidebar decides
// ---------------------------------------------------------------------------

/// The row marked current (`aria-current="page"`).
#[derive(Debug, Clone, PartialEq, Eq)]
enum Current {
    Lobby,
    Threads,
    Deliveries,
    Channel(String),
    Nothing,
}

/// Home marks the default channel; a channel route marks its own.
fn current_of(route: &Route, view: &WorkspaceView) -> Current {
    match route {
        Route::Lobby => Current::Lobby,
        Route::Threads { .. } => Current::Threads,
        Route::Deliveries => Current::Deliveries,
        Route::Home { channel: Some(id), .. } => Current::Channel(id.clone()),
        Route::Home { channel: None, .. } => {
            ws::default_channel_id(view).map_or(Current::Nothing, |id| Current::Channel(id.to_string()))
        }
        _ => Current::Nothing,
    }
}

/// The agents listed under "Agents": with a filter every listed one that
/// matches, inactive ones too; else the first eight, or every active one
/// after "See all".
fn listed_agents<'a>(groups: &SidebarAgents<'a>, query: &str, all: bool) -> Vec<&'a Agent> {
    let mut pool: Vec<&Agent> = groups.shown.clone();
    if !query.trim().is_empty() {
        pool.extend(&groups.rest);
        pool.extend(&groups.inactive);
    } else if all {
        pool.extend(&groups.rest);
    }
    ws::filter_agents(&pool, query)
}

/// The face the row goes by: the faces slice is newer than the full read.
fn live_state<'a>(a: &'a Agent, live: Option<&'a ActivityView>) -> &'a str {
    live.and_then(|v| v.agents.iter().find(|x| x.key == a.key))
        .map(|x| x.activity.state.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(a.activity.state.as_str())
}

/// A main session's row shows its first channel where an agent's shows its role.
fn as_session(a: &Agent) -> Agent {
    let mut a = a.clone();
    a.role_line = a.channels.first().cloned();
    a.role_line_code = None;
    a
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

/// The sidebar's colours, read once per render.
#[derive(Clone, Copy)]
struct Ink {
    bg: Hsla,
    ink: Hsla,
    ink2: Hsla,
    hover: Hsla,
    line: Hsla,
    accent: Hsla,
    accent_ink: Hsla,
    accent_ink2: Hsla,
    amber_edge: Hsla,
    surface: Hsla,
    text: Hsla,
    link: Hsla,
    line_strong: Hsla,
    btn_hover: Hsla,
}

impl Ink {
    fn of(theme: &Theme) -> Ink {
        Ink {
            bg: theme.c("sidebar"),
            ink: theme.c("sidebar-ink"),
            ink2: theme.c("sidebar-ink-2"),
            hover: theme.c("sidebar-hover"),
            line: theme.c("sidebar-line"),
            accent: theme.c("accent"),
            accent_ink: theme.c("accent-ink"),
            accent_ink2: theme.c("accent-ink-2"),
            amber_edge: theme.c("sidebar-amber-edge"),
            surface: theme.c("surface"),
            text: theme.c("ink"),
            link: theme.c("link"),
            line_strong: theme.c("line-strong"),
            btn_hover: theme.c("btn-hover"),
        }
    }
}

fn eid(s: String) -> ElementId {
    ElementId::Name(s.into())
}

fn tip(text: SharedString) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static {
    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
}

/// `.hb-row`: 32 px, 15 px text, the accent when current.
fn row(id: ElementId, current: bool, ink: Ink) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.))
        .w_full()
        .h(px(32.))
        .px(px(10.))
        .rounded(px(6.))
        .text_size(px(15.))
        .cursor_pointer()
        .when(current, |d| d.bg(ink.accent).text_color(ink.accent_ink))
        .when(!current, |d| d.text_color(ink.ink).hover(move |s| s.bg(ink.hover)))
}

/// `.hb-row-count`: a plain bold number at the row's end.
fn row_count(n: usize, current: bool, ink: Ink) -> AnyElement {
    div()
        .ml_auto()
        .text_size(px(13.))
        .font_weight(FontWeight::BOLD)
        .text_color(if current { ink.accent_ink2 } else { ink.ink2 })
        .child(n.to_string())
        .into_any_element()
}

/// A destination row: Lobby, Threads, Deliveries.
fn nav_row(id: &str, icon: &str, label: SharedString, count: Option<usize>, current: bool, route: Route, ink: Ink) -> gpui::Stateful<gpui::Div> {
    let color = if current { ink.accent_ink } else { ink.ink };
    row(eid(format!("sb-{id}")), current, ink)
        .child(ui::icon(icon, 18., color))
        .child(div().min_w_0().truncate().child(label))
        .children(count.map(|n| row_count(n, current, ink)))
        .on_click(move |_, _, cx| nav::navigate(route.clone(), cx))
}

/// `div.hb-section`: small, spaced, upper case.
fn section(label: SharedString, ink: Ink) -> gpui::Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(4.))
        .min_h(px(24.))
        .mt(px(18.))
        .mb(px(2.))
        .px(px(10.))
        .text_size(px(12.))
        .font_weight(FontWeight::EXTRA_BOLD)
        .text_color(ink.ink2)
        .child(label.to_uppercase())
}

/// A fold or "See all": a 14 px row in the second ink, with its chevron
/// (`Some(open)`) or without.
fn fold_row(
    id: &str,
    label: SharedString,
    chevron: Option<bool>,
    ink: Ink,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    row(eid(format!("sb-fold-{id}")), false, ink)
        .text_size(px(14.))
        .text_color(ink.ink2)
        .children(chevron.map(|open| ui::icon(if open { "chevron-down" } else { "chevron-right" }, 16., ink.ink2)))
        .child(label)
        .on_click(on_click)
        .into_any_element()
}

/// A line of text in the list (`<p class="px-[10px] text-[14px]">`).
fn note(text: SharedString, ink: Ink) -> gpui::Div {
    div()
        .flex()
        .flex_wrap()
        .flex_none()
        .px(px(10.))
        .text_size(px(14.))
        .text_color(ink.ink2)
        .child(text)
}

/// `ChannelRow`: `#` or a room's bubble, the name, a warning, the unread
/// count; ⋯ offers "Mark as read" and the channel's actions.
fn channel_row(c: &Channel, current: bool, ink: Ink, cx: &App) -> AnyElement {
    let id = c.id.clone();
    let color = if current { ink.accent_ink } else { ink.ink };
    let hash: AnyElement = if is_room(c) {
        ui::icon("message-circle", 16., color).into_any_element()
    } else {
        div().text_size(px(18.)).line_height(px(18.)).child("#").into_any_element()
    };
    let title = ws::channel_title(c);
    let route = ws::channel_route(&id);
    let mut el = row(eid(format!("sb-ch-{id}")), current, ink)
        .when(c.unread > 0, |d| d.font_weight(FontWeight::BLACK))
        .child(div().flex().flex_none().justify_center().w(px(18.)).opacity(0.85).child(hash))
        .child(div().min_w_0().truncate().child(id.clone()))
        .when(!c.warnings.is_empty(), |d| d.child(div().ml_auto().child(ui::icon("triangle-alert", 16., ink.amber_edge))))
        .when(c.unread > 0, |d| {
            d.child(
                div()
                    .ml_auto()
                    .flex_none()
                    .min_w(px(20.))
                    .px(px(6.))
                    .rounded(px(10.))
                    .bg(ink.accent)
                    .text_color(ink.accent_ink)
                    .text_size(px(12.))
                    .font_weight(FontWeight::BOLD)
                    .flex()
                    .justify_center()
                    .child(c.unread.to_string()),
            )
        })
        .on_click(move |_, _, cx| nav::navigate(route.clone(), cx));
    if !title.is_empty() {
        el = el.tooltip(tip(title.into()));
    }
    let read_id = id.clone();
    let on_read: RowRun = Rc::new(move |_, cx| {
        let channel = read_id.clone();
        Desk::get(cx).update(cx, |d, cx| d.post("read-channel", json!({ "channel": channel }), cx)).detach();
    });
    let actions = crate::screens::rooms::channel_actions(c, cx);
    row_menu(eid(format!("sb-chm-{id}")), format!("#{id}").into(), Some(on_read), actions, el.into_any_element(), cx)
}

/// The name with where it works on one line (`.hb-sidebar .hb-agent-name`):
/// the name keeps up to 60% of the text column, the role line the rest.
/// The 60% is in points: a percentage width under truncation can be laid
/// out against a zero-wide column and stay "…".
fn inline_name(a: &Agent, idle: bool, ink: Ink, cx: &App) -> gpui::Div {
    // The text column: 300 − 2×10 (list) − 2×10 (row) − robot − 10 gap; an
    // idle row also gives about 52 points to its timer.
    let column = if idle { 300. - 40. - 24. - 10. - 52. } else { 300. - 40. - 32. - 10. };
    let role = a.role_line.as_ref().map(|_| ws::agent_role_line(a, cx));
    div()
        .flex()
        .items_end()
        .gap(px(6.))
        .min_w_0()
        .child(
            div()
                .flex_none()
                .max_w(px(column * 0.6))
                .truncate()
                .text_size(px(15.))
                .font_weight(if idle { FontWeight::NORMAL } else { FontWeight::BOLD })
                .text_color(ink.ink)
                .child(crate::derive::agent_name(a, cx)),
        )
        .children(role.map(|r| {
            div().flex_1().min_w_0().truncate().text_size(px(12.)).font_weight(FontWeight::NORMAL).text_color(ink.ink2).child(r)
        }))
}

/// `AgentRow`: the robot, the name with where it works, what it is doing.
/// One with nothing to do takes a single line (24 px robot, name and time).
fn agent_row(a: &Agent, idle: bool, ink: Ink, cx: &App) -> AnyElement {
    let key = a.key.clone();
    let size = if idle { 24. } else { 32. };
    let name = inline_name(a, idle, ink, cx);
    let status = robot::status_line(a).color(ink.ink2).long_color(ink.ink).text_size(13.);
    let text = if idle {
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_end()
            .gap(px(8.))
            .child(div().flex().flex_1().min_w_0().child(name))
            .child(status.time_only())
    } else {
        div().flex().flex_col().flex_1().min_w_0().child(name).child(status)
    };
    let route = Route::direct(Some(key.clone()));
    let el = div()
        .id(eid(format!("sb-ag-{key}")))
        .flex()
        .flex_none()
        .items_center()
        .gap(px(10.))
        .w_full()
        .px(px(10.))
        .rounded(px(6.))
        .text_size(px(15.))
        .text_color(ink.ink)
        .cursor_pointer()
        .hover(move |s| s.bg(ink.hover))
        .when(idle, |d| d.h(px(32.)))
        .when(!idle, |d| d.min_h(px(44.)).py(px(4.)))
        .child(robot::robot(crate::derive::robot_name(a).to_string(), size).agent(a).dot(true).ring(Ring::Sidebar))
        .child(text)
        .on_click(move |_, _, cx| nav::navigate(route.clone(), cx));
    let hide = ws::hide_action(a, cx);
    let action = hide.action;
    let menu = vec![RowAction::new(hide.label, move |_, cx| {
        let agent = key.clone();
        Desk::get(cx).update(cx, |d, cx| d.post(action, json!({ "agent": agent }), cx)).detach();
    })];
    let row = row_menu(eid(format!("sb-agm-{}", a.key)), a.name.clone().into(), None, menu, el.into_any_element(), cx);
    agent_hover(a, row, cx)
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let _ = window;
        let ink = Ink::of(cx.global::<Theme>());
        let desk = Desk::get(cx).read(cx);
        let now = desk.now_ms();
        let live = desk.activity.clone();
        let Some(view) = desk.view.clone() else {
            return div().w(px(300.)).h_full().flex_none().bg(ink.bg).border_r_1().border_color(ink.line).into_any_element();
        };
        let current = current_of(Nav::route(cx), &view);
        let name: SharedString = ws::current_name(&view.workspaces).to_string().into();

        // The head: the workspace name opens the workspace menu.
        let head = div()
            .id("sb-ws-name")
            .group("sb-ws-name")
            .flex()
            .flex_none()
            .items_center()
            .h(px(56.))
            .px(px(18.))
            .cursor_pointer()
            .text_size(px(20.))
            .font_weight(FontWeight::BLACK)
            .text_color(ink.ink)
            .tooltip(tip(tv(cx, "home.menuOf", &[("name", &name)])))
            .on_click(|_, window, cx| open_workspace_menu(window, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .ml(px(-8.))
                    .px(px(8.))
                    .py(px(4.))
                    .rounded(px(8.))
                    .group_hover("sb-ws-name", move |s| s.bg(ink.hover))
                    .child(name.clone())
                    .child(ui::icon("chevron-down", 20., ink.ink)),
            );

        let mut list: Vec<AnyElement> = Vec::new();

        // Lobby, Threads, Deliveries.
        let here = ws::lobby_here_count(&view);
        list.push(
            nav_row("lobby", "users", t(cx, "rooms.lobbyEntry"), (here > 0).then_some(here), current == Current::Lobby, Route::Lobby, ink)
                .into_any_element(),
        );
        list.push(
            nav_row("threads", "message-square", t(cx, "home.threads"), None, current == Current::Threads, Route::Threads { thread: None }, ink)
                .into_any_element(),
        );
        list.push(
            nav_row(
                "deliveries",
                "package-check",
                t(cx, "home.deliveries"),
                Some(ws::pending_count(&view)),
                current == Current::Deliveries,
                Route::Deliveries,
                ink,
            )
            .tooltip(tip(t(cx, "deliveries.hint")))
            .into_any_element(),
        );

        // Channels.
        let is_current = |id: &str| matches!(&current, Current::Channel(c) if c == id);
        let channels = ws::sidebar_channels(&view);
        list.push(section(t(cx, "home.channels"), ink).into_any_element());
        if channels.shown.is_empty() && channels.more.is_empty() {
            list.push(note(t(cx, "home.noCheckoutsYet"), ink).into_any_element());
        }
        for c in &channels.shown {
            list.push(channel_row(c, is_current(&c.id), ink, cx));
        }
        if !channels.more.is_empty() {
            let label = tv(cx, "home.moreChannels", &[("count", &channels.more.len().to_string())]);
            list.push(fold_row("more-channels", label, Some(self.more_channels), ink, self.flip(|s| &mut s.more_channels, cx)));
            if self.more_channels {
                for c in &channels.more {
                    list.push(channel_row(c, is_current(&c.id), ink, cx));
                }
            }
        }

        // Main sessions.
        let groups = ws::sidebar_agents(&view, now);
        let idle = |a: &Agent| live_state(a, live.as_deref()) == "normal";
        if !groups.sessions.is_empty() {
            list.push(
                section(t(cx, "home.mainSessions"), ink)
                    .id("sb-sessions")
                    .tooltip(tip(t(cx, "home.mainSessionsHint")))
                    .into_any_element(),
            );
            for a in &groups.sessions {
                let s = as_session(a);
                list.push(agent_row(&s, idle(a), ink, cx));
            }
        }

        // Agents: folded on the desk.
        let folded = self.agents_folded;
        list.push(
            section(t(cx, "home.agents"), ink)
                .id("sb-agents-fold")
                .rounded(px(6.))
                .cursor_pointer()
                .hover(move |s| s.bg(ink.hover))
                .child(div().ml_auto().child(ui::icon(if folded { "chevron-right" } else { "chevron-down" }, 16., ink.ink2)))
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.toggle_agents(cx)))
                .into_any_element(),
        );
        if !folded {
            let query = self.query.trim().to_string();
            if groups.listed() > ws::FILTER_FROM || !self.query.is_empty() {
                list.push(
                    div()
                        .flex_none()
                        .pt(px(2.))
                        .px(px(10.))
                        .pb(px(6.))
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .h(px(30.))
                                .rounded(px(6.))
                                .border_1()
                                .border_color(ink.line)
                                .bg(ink.surface)
                                .text_color(ink.text)
                                .text_size(px(14.))
                                .child(
                                    Input::new(&self.filter)
                                        .appearance(false)
                                        .px(px(10.))
                                        .py_0()
                                        .h_full()
                                        .text_size(px(14.)),
                                ),
                        )
                        .into_any_element(),
                );
            }
            let agents = listed_agents(&groups, &self.query, self.see_all);
            if agents.is_empty() {
                let el = if query.is_empty() {
                    note(t(cx, "home.noAgentsRecently"), ink)
                        .children(super::header::work_elsewhere(&view.workspaces, " ", cx))
                } else {
                    note(tv(cx, "home.noAgentMatches", &[("query", &query)]), ink)
                };
                list.push(el.into_any_element());
            }
            for a in &agents {
                list.push(agent_row(a, idle(a), ink, cx));
            }
            if !groups.rest.is_empty() && query.is_empty() {
                let label = if self.see_all {
                    t(cx, "home.seeLess")
                } else {
                    tv(cx, "home.seeAll", &[("count", &(groups.shown.len() + groups.rest.len()).to_string())])
                };
                list.push(fold_row("see-all", label, None, ink, self.flip(|s| &mut s.see_all, cx)));
            }
            if !groups.inactive.is_empty() && query.is_empty() {
                let label = tv(cx, "home.inactiveCount", &[("count", &groups.inactive.len().to_string())]);
                list.push(fold_row("inactive", label, Some(self.inactive), ink, self.flip(|s| &mut s.inactive, cx)));
                if self.inactive {
                    for a in &groups.inactive {
                        list.push(agent_row(a, idle(a), ink, cx));
                    }
                }
            }
            list.extend(self.hidden_group(&view, ink, cx));
        }

        // Temporary sessions.
        let temporary = ws::temporary_agents(&view);
        if !temporary.is_empty() && !folded {
            let label = tv(cx, "home.temporarySessions", &[("count", &temporary.len().to_string())]);
            list.push(fold_row("temporary", label, None, ink, self.flip(|s| &mut s.temporary, cx)));
            if self.temporary {
                for a in &temporary {
                    list.push(agent_row(a, idle(a), ink, cx));
                }
            }
        }

        // Rooms, and "+" to create one.
        let rooms = sidebar_rooms(&view);
        let create = t(cx, "rooms.createRoom");
        list.push(
            section(t(cx, "rooms.sectionRooms"), ink)
                .justify_between()
                .child(
                    div()
                        .id("sb-create-room")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size(px(24.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .text_color(ink.ink2)
                        .hover(move |s| s.bg(ink.hover).text_color(ink.ink))
                        .child(ui::icon("plus", 16., ink.ink2))
                        .tooltip(tip(create))
                        .on_click(|_, window, cx| crate::screens::rooms::open_create_room(window, cx)),
                )
                .into_any_element(),
        );
        for c in &rooms.rooms {
            list.push(channel_row(c, is_current(&c.id), ink, cx));
        }
        if !rooms.archived.is_empty() {
            let label = tv(cx, "rooms.archivedCount", &[("count", &rooms.archived.len().to_string())]);
            list.push(fold_row("archived", label, Some(self.archived), ink, self.flip(|s| &mut s.archived, cx)));
            if self.archived {
                for c in &rooms.archived {
                    list.push(channel_row(c, is_current(&c.id), ink, cx));
                }
            }
        }

        let scroll = div()
            .id("sb-scroll")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pt(px(4.))
            .px(px(10.))
            .pb(px(12.))
            .children(list);

        div()
            .w(px(300.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .min_h_0()
            .bg(ink.bg)
            .border_r_1()
            .border_color(ink.line)
            .font_family(ui::SANS)
            .line_height(relative(1.46))
            .child(head)
            .child(scroll)
            .child(self.more_tools(ink, cx))
            .into_any_element()
    }
}

impl Sidebar {
    /// `HiddenGroup`: "Hidden (n)" folded; open, each with "Show", and "Show all".
    fn hidden_group(&self, view: &WorkspaceView, ink: Ink, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let hidden = ws::hidden_agents(view);
        if hidden.is_empty() {
            return Vec::new();
        }
        let label = tv(cx, "shell.hiddenCount", &[("count", &hidden.len().to_string())]);
        let mut out = vec![fold_row("hidden", label, Some(self.hidden), ink, self.flip(|s| &mut s.hidden, cx))];
        if !self.hidden {
            return out;
        }
        let show = t(cx, "shell.show");
        let button = move |id: ElementId, label: SharedString| {
            div()
                .id(id)
                .flex_none()
                .px(px(8.))
                .py(px(2.))
                .rounded(px(4.))
                .border_1()
                .border_color(ink.line_strong)
                .bg(ink.surface)
                .text_size(px(13.))
                .font_weight(FontWeight::BOLD)
                .text_color(ink.link)
                .cursor_pointer()
                .hover(move |s| s.bg(ink.btn_hover))
                .child(label)
        };
        let mut rows = div().flex().flex_col().flex_none().mb(px(6.));
        for a in hidden {
            let key = a.key.clone();
            rows = rows.child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .min_h(px(32.))
                    .py(px(2.))
                    .pr(px(10.))
                    .pl(px(28.))
                    .text_size(px(14.))
                    .text_color(ink.ink2)
                    .child(robot::robot(crate::derive::robot_name(a).to_string(), 24.).agent(a))
                    .child(div().flex_1().min_w_0().truncate().child(a.name.clone()))
                    .child(
                        button(eid(format!("sb-show-{key}")), show.clone())
                            .tooltip(tip(tv(cx, "shell.showAgent", &[("name", &a.name)])))
                            .on_click(move |_, _, cx| {
                                let agent = key.clone();
                                Desk::get(cx).update(cx, |d, cx| d.post("show-agent", json!({ "agent": agent }), cx)).detach();
                            }),
                    ),
            );
        }
        rows = rows.child(div().flex().mt(px(4.)).ml(px(28.)).child(button("sb-show-all".into(), t(cx, "shell.showAll")).on_click(
            |_, _, cx| {
                Desk::get(cx).update(cx, |d, cx| d.post("show-all-agents", json!({}), cx)).detach();
            },
        )));
        out.push(rows.into_any_element());
        out
    }

    /// `MoreTools`: pinned at the foot; its list opens upwards.
    fn more_tools(&self, ink: Ink, cx: &mut Context<Self>) -> AnyElement {
        let open = self.more_tools;
        let button = row("sb-more".into(), false, ink)
            .font_weight(FontWeight::BOLD)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(20.))
                    .rounded(px(5.))
                    .bg(ink.hover)
                    .child(ui::icon("layout-grid", 16., ink.ink)),
            )
            .child(t(cx, "shell.moreTools"))
            .child(div().ml_auto().child(ui::icon(if open { "chevron-down" } else { "chevron-right" }, 16., ink.ink)))
            .on_click(self.flip(|s| &mut s.more_tools, cx));

        let mut foot = div()
            .flex()
            .flex_col()
            .flex_none()
            .pt(px(6.))
            .px(px(10.))
            .pb(px(10.))
            .border_t_1()
            .border_color(ink.line);
        if open {
            type Go = fn(&mut Window, &mut App);
            let tools: [(&str, &str, SharedString, SharedString, Go); 3] = [
                ("strip", "monitor-play", "Strip".into(), t(cx, "workspace.toolStripDescription"), |w, cx| open_strip(w, cx)),
                ("history", "rotate-ccw-clock", t(cx, "workspace.toolHistoryTitle"), t(cx, "workspace.toolHistoryDescription"), |_, cx| {
                    nav::navigate(Route::History { channel: None, q: String::new() }, cx)
                }),
                ("setup", "plug", t(cx, "workspace.toolSetupTitle"), t(cx, "workspace.toolSetupDescription"), |_, cx| {
                    nav::navigate(Route::Setup, cx)
                }),
            ];
            let mut items = div().flex().flex_col().gap(px(2.)).mb(px(4.));
            for (id, icon, title, desc, go) in tools {
                let close = cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.more_tools = false;
                    cx.notify();
                });
                items = items.child(
                    div()
                        .id(eid(format!("sb-tool-{id}")))
                        .flex()
                        .items_start()
                        .gap(px(10.))
                        .pt(px(8.))
                        .pr(px(10.))
                        .pb(px(8.))
                        .pl(px(12.))
                        .rounded(px(6.))
                        .text_color(ink.ink)
                        .cursor_pointer()
                        .hover(move |s| s.bg(ink.hover))
                        .child(div().mt(px(2.)).child(ui::icon(icon, 18., ink.ink)))
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .min_w_0()
                                .child(div().text_size(px(15.)).font_weight(FontWeight::BOLD).child(title))
                                .child(div().text_size(px(13.)).line_height(relative(1.35)).text_color(ink.ink2).child(desc)),
                        )
                        .on_click(move |ev, window, cx| {
                            close(ev, window, cx);
                            go(window, cx);
                        }),
                );
            }
            foot = foot.child(items);
        }
        foot.child(button).into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derive::workspace::fixtures::{MIN, NOW, agent, channel, face, view};
    use crate::model::Channel;

    fn keys(list: &[&Agent]) -> Vec<String> {
        list.iter().map(|a| a.key.clone()).collect()
    }

    fn project(id: &str) -> Channel {
        Channel { repo: format!("/tmp/{id}"), ..channel(id) }
    }

    #[test]
    fn home_marks_the_default_channel_and_a_channel_route_its_own() {
        let mut v = view(vec![], vec![]);
        v.channels = vec![
            Channel { kind: Some("room".into()), ..channel("commons") },
            project("orchard"),
            project("harbor"),
        ];
        assert_eq!(current_of(&Route::home(), &v), Current::Channel("orchard".into()));
        assert_eq!(current_of(&Route::channel("harbor"), &v), Current::Channel("harbor".into()));
        assert_eq!(current_of(&Route::channel("lobby"), &v), Current::Lobby);
        assert_eq!(current_of(&Route::Threads { thread: Some("m1".into()) }, &v), Current::Threads);
        assert_eq!(current_of(&Route::Deliveries, &v), Current::Deliveries);
        assert_eq!(current_of(&Route::direct(None), &v), Current::Nothing);
    }

    #[test]
    fn home_without_a_project_channel_marks_nothing() {
        let v = view(vec![], vec![]);
        assert_eq!(current_of(&Route::home(), &v), Current::Nothing);
    }

    #[test]
    fn the_list_shows_eight_then_all_and_a_filter_searches_inactive_ones_too() {
        let mut agents: Vec<Agent> = (0..10)
            .map(|i| {
                let mut a = agent(&format!("a{i}"), &format!("Agent {i}"));
                a.last_action_ms = Some(NOW - i as u64 * 1000);
                a
            })
            .collect();
        let mut old = agent("old", "Dormant Dora");
        old.activity = face("normal", None);
        old.last_action_ms = Some(NOW - 60 * MIN);
        agents.push(old);
        let v = view(agents, vec![]);
        let groups = ws::sidebar_agents(&v, NOW);
        assert_eq!(groups.inactive.len(), 1);
        assert_eq!(listed_agents(&groups, "", false).len(), 8);
        assert_eq!(listed_agents(&groups, "", true).len(), 10);
        assert_eq!(keys(&listed_agents(&groups, "dora", false)), vec!["old"]);
        assert_eq!(keys(&listed_agents(&groups, "  agent 9 ", false)), vec!["a9"]);
    }

    #[test]
    fn a_session_row_shows_its_first_channel() {
        let mut a = agent("s1", "Adrian");
        a.channels = vec!["harbard".into(), "memories".into()];
        a.role_line = Some("lead".into());
        a.role_line_code = Some("role.lead".into());
        let s = as_session(&a);
        assert_eq!(s.role_line.as_deref(), Some("harbard"));
        assert_eq!(s.role_line_code, None);
    }

    #[test]
    fn the_faces_slice_decides_whether_a_row_is_idle() {
        let mut a = agent("a1", "Ann");
        a.activity = face("running", None);
        assert_eq!(live_state(&a, None), "running");
        let mut live = ActivityView::default();
        live.agents.push(crate::model::AgentActivity {
            key: "a1".into(),
            activity: face("normal", None),
            ..Default::default()
        });
        assert_eq!(live_state(&a, Some(&live)), "normal");
    }
}
