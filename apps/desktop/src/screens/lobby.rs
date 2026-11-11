//! The Lobby (`/lobby`): everyone who has come through this workspace, in
//! their own words. Left, the stage: the title, the counts, the lineup of
//! robots, the filter box and the cards grouped Here now / Around / Gone.
//! Right, the `#lobby` room and its composer. Below 900px the two stack
//! into one column.

use super::rooms::RoomFeed;
use super::secondary::{self as kit, head, search_frame, sync_list, width_probe};
use crate::chat::{ComposeTarget, Composer, ComposerState};
use crate::derive::lobby::{
    Group, LOBBY, counts, filter, groups, harness_name, lineup, lobby_channel, presence_word,
    room_notice,
};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{Activity, Agent, LobbyEntry, WorkspaceView};
use crate::nav::{self, Route};
use crate::theme::Theme;
use crate::ui::{self, avatar_size, format::hace, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, InteractiveElement, IntoElement, ListAlignment,
    ListState, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, Window, div, list, px,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

/// How long a card stays lit after the lineup points at it.
const FLASH: Duration = Duration::from_millis(1600);
/// The feed column, and the width under which the two columns stack.
const FEED_W: f32 = 360.;
const NARROW: f32 = 900.;
/// The stacked feed's height when the columns are one.
const FEED_STACKED_H: f32 = 520.;
const CARD_MIN: f32 = 286.;
const GAP: f32 = space::XL;
const PAD: f32 = space::GUTTER;
/// The filter box's widest.
const FIND_W: f32 = 420.;
/// An introduction shows this many lines; the rest is in the agent's DM.
const INTRO_LINES: usize = 4;

#[derive(Clone)]
enum Row {
    Hero,
    Find,
    Quiet,
    NoMatch,
    Head(Group, usize),
    Cards(Vec<usize>),
    Feed,
    Bottom,
}

pub struct LobbyScreen {
    search: Entity<InputState>,
    feed: Entity<RoomFeed>,
    composer: Entity<Composer>,
    width: Rc<Cell<f32>>,
    list: ListState,
    rows: Vec<Row>,
    /// Columns the rows were cut for, and whether the feed is stacked.
    cols: usize,
    narrow: bool,
    view: Option<Arc<WorkspaceView>>,
    lit: Option<String>,
    find_focused: bool,
    seen_unread: u64,
    _flash: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl LobbyScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |this, _, cx| this.refresh(cx))
            .detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        cx.observe_global_in::<crate::i18n::I18n>(window, |this, window, cx| {
            let placeholder = t(cx, "rooms.lobbyFindPlaceholder");
            this.search
                .update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
            // The room's own words follow the language too.
            let write_in = tv(cx, "rooms.writeIn", &[("room", LOBBY)]);
            this.composer.update(cx, |c, cx| {
                c.set_target(ComposeTarget::Channel(LOBBY.into()), write_in, cx)
            });
            let empty = t(cx, "rooms.lobbyFeedEmpty");
            this.feed
                .update(cx, |f, cx| f.set_room(LOBBY.into(), empty, cx));
            this.refresh(cx)
        })
        .detach();
        let placeholder = t(cx, "rooms.lobbyFindPlaceholder");
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subs = vec![
            cx.subscribe(&search, |this, _, ev: &InputEvent, cx| match ev {
                InputEvent::Change => this.refresh(cx),
                InputEvent::Focus | InputEvent::Blur => {
                    this.find_focused = matches!(ev, InputEvent::Focus);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {}
            }),
        ];
        let feed =
            cx.new(|cx| RoomFeed::new(LOBBY.into(), false, t(cx, "rooms.lobbyFeedEmpty"), cx));
        let composer = cx.new(|cx| Composer::new(ComposeTarget::Channel(LOBBY.into()), window, cx));
        let write_in = tv(cx, "rooms.writeIn", &[("room", LOBBY)]);
        composer.update(cx, |c, cx| {
            c.set_target(ComposeTarget::Channel(LOBBY.into()), write_in, cx)
        });
        let mut this = LobbyScreen {
            search,
            feed,
            composer,
            width: Rc::new(Cell::new(0.)),
            list: ListState::new(0, ListAlignment::Top, px(600.)),
            rows: Vec::new(),
            cols: 1,
            narrow: false,
            view: None,
            lit: None,
            find_focused: false,
            seen_unread: 0,
            _flash: None,
            _subs: subs,
        };
        this.refresh(cx);
        this
    }

    fn query(&self, cx: &App) -> String {
        self.search.read(cx).value().to_string()
    }

    /// Whether the columns stack, and how many cards a row holds
    /// (`repeat(auto-fill, minmax(286px, 1fr))`).
    fn layout(&self) -> (bool, usize) {
        let avail = self.grid_width();
        let w = self.width.get();
        let narrow = w > 0. && w < NARROW;
        let cols = (((avail + GAP) / (CARD_MIN + GAP)).floor() as usize).max(1);
        (narrow, cols)
    }

    /// The width the card grid lays out in.
    fn grid_width(&self) -> f32 {
        let w = self.width.get();
        let narrow = w > 0. && w < NARROW;
        let stage = if narrow { w } else { w - FEED_W - 1. };
        (stage - 2. * PAD).max(CARD_MIN)
    }

    fn card_width(&self) -> f32 {
        let cols = self.cols.max(1) as f32;
        ((self.grid_width() - GAP * (cols - 1.)) / cols).floor()
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let view = Desk::get(cx).read(cx).view.clone();
        let (narrow, cols) = self.layout();
        self.narrow = narrow;
        self.cols = cols;
        self.rows = match &view {
            Some(v) => self.rows_for(v, cx),
            None => Vec::new(),
        };
        // Open is read, as in any other room.
        if let Some(v) = &view {
            let unread = lobby_channel(v).unread;
            if unread > 0 && unread != self.seen_unread {
                self.seen_unread = unread;
                Desk::get(cx).update(cx, |desk, cx| {
                    desk.post("read-channel", serde_json::json!({ "channel": LOBBY }), cx)
                        .detach();
                });
            }
        }
        // The composer's notice slot: a post here reaches no one's work.
        let notice = view
            .as_ref()
            .and_then(|v| room_notice(&lobby_channel(v), cx));
        self.composer.update(cx, |c, cx| {
            c.set_state(
                ComposerState {
                    notice,
                    ..Default::default()
                },
                cx,
            )
        });
        self.view = view;
        sync_list(&self.list, self.rows.len());
        cx.notify();
    }

    fn rows_for(&self, view: &WorkspaceView, cx: &App) -> Vec<Row> {
        let entries = &view.lobby;
        let mut rows = vec![Row::Hero];
        if entries.is_empty() {
            rows.push(Row::Quiet);
        } else {
            rows.push(Row::Find);
            let q = self.query(cx);
            let shown = filter(entries, &q, |h| harness_name(h, cx).to_string());
            let found = groups(&shown);
            if found.is_empty() {
                rows.push(Row::NoMatch);
            }
            let index =
                |e: &LobbyEntry| entries.iter().position(|x| std::ptr::eq(x, e)).unwrap_or(0);
            for (group, list) in found {
                rows.push(Row::Head(group, list.len()));
                for chunk in list.chunks(self.cols) {
                    rows.push(Row::Cards(chunk.iter().map(|e| index(e)).collect()));
                }
            }
        }
        rows.push(Row::Bottom);
        if self.narrow {
            rows.push(Row::Feed);
        }
        rows
    }

    /// Scroll to a card and light it for a moment.
    fn point(&mut self, key: String, cx: &mut Context<Self>) {
        let Some(view) = &self.view else { return };
        let Some(at) = view.lobby.iter().position(|e| e.key == key) else {
            return;
        };
        if let Some(ix) = self
            .rows
            .iter()
            .position(|r| matches!(r, Row::Cards(list) if list.contains(&at)))
        {
            self.list.scroll_to_reveal_item(ix);
        }
        self.lit = Some(key);
        let timer = cx.background_executor().timer(FLASH);
        self._flash = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |this, cx| {
                this.lit = None;
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn render_row(
        &mut self,
        ix: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let (Some(view), Some(row)) = (self.view.clone(), self.rows.get(ix).cloned()) else {
            return div().into_any_element();
        };
        let theme = cx.global::<Theme>().clone();
        match row {
            Row::Hero => self.hero(&view, &theme, cx),
            Row::Find => div()
                .pt(px(space::XL))
                .px(px(PAD))
                .child(
                    div().max_w(px(FIND_W)).child(search_frame(
                        Input::new(&self.search)
                            .appearance(false)
                            .text_size(px(size::UI)),
                        self.find_focused,
                        &theme,
                    )),
                )
                .into_any_element(),
            Row::Quiet => kit::empty_block(
                Some(t(cx, "rooms.lobbyEmptyTitle")),
                vec![
                    div()
                        .child(t(cx, "rooms.lobbyEmptyBody"))
                        .into_any_element(),
                    kit::mono("harbard intro").into_any_element(),
                ],
                &theme,
            )
            .into_any_element(),
            Row::NoMatch => kit::empty_block(
                None,
                vec![
                    tv(
                        cx,
                        "rooms.lobbyNoMatch",
                        &[("query", self.query(cx).trim())],
                    )
                    .into_any_element(),
                ],
                &theme,
            )
            .into_any_element(),
            Row::Head(group, n) => div()
                .px(px(PAD))
                .pt(px(space::SECTION))
                .pb(px(space::MD))
                .flex()
                .items_center()
                .gap(px(space::SM))
                .child(ui::section_label(group.title(cx), &theme))
                .child(ui::text(
                    n.to_string(),
                    size::META,
                    weight::REGULAR,
                    theme.c("ink-3"),
                ))
                .into_any_element(),
            Row::Cards(list) => {
                let w = self.card_width();
                let mut row = div().px(px(PAD)).pb(px(GAP)).flex().gap(px(GAP));
                for &i in &list {
                    row = row.child(div().w(px(w)).flex_none().flex().child(self.card(
                        &view,
                        &view.lobby[i],
                        &theme,
                        cx,
                    )));
                }
                row.into_any_element()
            }
            Row::Feed => div()
                .h(px(FEED_STACKED_H))
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(theme.c("line"))
                .child(self.feed_column(&theme, cx))
                .into_any_element(),
            Row::Bottom => div().h(px(space::XS)).into_any_element(),
        }
    }

    /// The counts and the robots of the moment in one line each.
    fn hero(&self, view: &WorkspaceView, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (here, around, gone) = counts(&view.lobby);
        let (shown, more) = lineup(&view.lobby);
        let count = |key: &str, n: usize, ink: &str| {
            ui::text(
                tv(cx, key, &[("count", &n.to_string())]),
                size::SMALL,
                weight::REGULAR,
                theme.c(ink),
            )
        };
        let counts = div()
            .flex()
            .flex_wrap()
            .gap_x(px(space::XL))
            .gap_y(px(space::XS))
            // Only "here now" is a live state (C3).
            .child(count(
                "rooms.lobbyCountHere",
                here,
                if here > 0 { "green-ink" } else { "ink-3" },
            ))
            .child(count("rooms.lobbyCountAround", around, "ink-3"))
            .child(count("rooms.lobbyCountGone", gone, "ink-3"));
        let shelf = (!shown.is_empty()).then(|| {
            let hover = theme.c("hover");
            // Each robot sits in a hair of padding for its hover wash; pull
            // the row back so the first robot stays on the gutter.
            let mut ul = div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap(px(space::XS))
                .mt(px(space::LG))
                .ml(px(-space::HAIR));
            for e in shown {
                let key = e.key.clone();
                let agent = view.agent(&e.key);
                ul = ul.child(
                    kit::row_focus(
                        div().id(SharedString::from(format!("stand-{}", e.key))),
                        theme,
                    )
                    .p(px(space::HAIR))
                    .rounded(px(ui::radius::MD))
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .tooltip(kit::tip(e.name.clone().into()))
                    .on_click(cx.listener(move |this, _, _, cx| this.point(key.clone(), cx)))
                    .child(portrait(e, agent, cx)),
                );
            }
            if more > 0 {
                ul = ul.child(
                    ui::text(
                        tv(cx, "rooms.lobbyLineupMore", &[("count", &more.to_string())]),
                        size::META,
                        weight::REGULAR,
                        theme.c("ink-3"),
                    )
                    .pl(px(space::XS)),
                );
            }
            ul
        });
        div()
            .pt(px(space::XL))
            .px(px(PAD))
            .child(counts)
            .children(shelf)
            .into_any_element()
    }

    /// One card: the robot beside the name, where it runs, what it said,
    /// when it was seen.
    fn card(
        &self,
        view: &WorkspaceView,
        e: &LobbyEntry,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let agent = view.agent(&e.key);
        let lit = self.lit.as_deref() == Some(e.key.as_str());
        let now = view.now_ms;
        // Where it runs: the harness, then the project.
        let mut runs = harness_name(&e.harness, cx).to_string();
        if let Some(project) = e.project.as_deref().filter(|p| !p.is_empty()) {
            runs.push_str(" · ");
            runs.push_str(project);
        }
        let said: AnyElement = match &e.intro {
            Some(intro) => ui::text(
                intro.clone(),
                size::SMALL,
                weight::REGULAR,
                theme.c("ink-2"),
            )
            .line_clamp(INTRO_LINES)
            .into_any_element(),
            None => div()
                .child(ui::text(
                    t(cx, "rooms.lobbyNoIntro"),
                    size::SMALL,
                    weight::REGULAR,
                    theme.c("ink-3"),
                ))
                .children(agent.and_then(|a| a.description.clone()).map(|what| {
                    ui::text(
                        tv(cx, "rooms.lobbyKnownDoing", &[("what", &what)]),
                        size::SMALL,
                        weight::REGULAR,
                        theme.c("ink-2"),
                    )
                    .line_clamp(INTRO_LINES - 1)
                }))
                .into_any_element(),
        };
        let mut seen = if e.presence == "live" {
            tv(
                cx,
                "rooms.lobbyFirstSeen",
                &[("when", &hace(e.first_ms, now, cx))],
            )
        } else {
            tv(
                cx,
                "rooms.lobbyLastSeen",
                &[("when", &hace(e.last_ms, now, cx))],
            )
        }
        .to_string();
        if let Some(ts) = e.intro_ts_ms.filter(|_| e.intro.is_some()) {
            seen.push_str(" · ");
            seen.push_str(&tv(cx, "rooms.lobbySaid", &[("when", &hace(ts, now, cx))]));
        }
        let hover = theme.c("hover");
        let target = agent.map(|a| a.key.clone());
        let base = div()
            .id(SharedString::from(format!("lobby-{}", e.key)))
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .gap(px(space::MD))
            .p(px(space::XL))
            .rounded(px(ui::radius::MD))
            .border_1()
            .border_color(theme.c("line"))
            // The lineup pointed here: the row wash, for a moment.
            .when(lit, |d| d.bg(theme.c("selected")));
        let base = match target {
            Some(key) => kit::row_focus(base, theme)
                .cursor_pointer()
                .hover(move |s| s.bg(hover))
                .on_click(move |_, _, cx| nav::navigate(Route::direct(Some(key.clone())), cx)),
            None => base,
        };
        base.child(
            div()
                .flex()
                .items_center()
                .gap(px(space::LG))
                .child(portrait(e, agent, cx))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(
                            div()
                                .flex()
                                .items_center()
                                .gap(px(space::MD))
                                .child(
                                    ui::text(
                                        e.name.clone(),
                                        size::UI,
                                        weight::HEAVY,
                                        theme.c("ink"),
                                    )
                                    .flex_1()
                                    .min_w_0()
                                    .truncate(),
                                )
                                .child(presence_mark(&e.presence, theme, cx)),
                        )
                        .child(
                            ui::text(runs, size::META, weight::REGULAR, theme.c("ink-3"))
                                .truncate(),
                        ),
                ),
        )
        .child(said)
        .child(
            ui::text(seen, size::META, weight::REGULAR, theme.c("ink-3"))
                .mt_auto()
                .truncate(),
        )
        .into_any_element()
    }

    /// `#lobby`: its head, the room's feed and the composer (with the notice).
    fn feed_column(&self, theme: &Theme, cx: &App) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(ui::pane_header(t(cx, "rooms.lobbyFeedTitle"), theme))
            .child(self.feed.clone())
            .child(self.composer.clone())
    }
}

/// Presence at the end of the name line: the dot, and the word where the
/// group's title does not already say it ("Idle" and "Gone quiet" share
/// the Around group).
fn presence_mark(presence: &str, theme: &Theme, cx: &App) -> impl IntoElement {
    let dot = div().size(px(space::MD)).rounded_full().flex_none();
    let dot = match presence {
        "live" => dot.bg(theme.c("green")),
        "idle" => dot.bg(theme.c("amber")),
        "ended" => dot.border_1().border_color(theme.c("grey")),
        _ => dot.bg(theme.c("grey")),
    };
    let word = matches!(Group::of(presence), Group::Around).then(|| {
        ui::text(
            presence_word(presence, cx),
            size::META,
            weight::REGULAR,
            theme.c("ink-3"),
        )
        .whitespace_nowrap()
    });
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(space::SM))
        .children(word)
        .child(dot)
}

/// Its live robot when the view has the agent; otherwise the same robot at
/// rest, by name, with a face for its presence. One size everywhere (Z4).
fn portrait(e: &LobbyEntry, agent: Option<&Agent>, cx: &App) -> AnyElement {
    if let Some(a) = agent {
        return ui::avatar::agent_avatar(a, avatar_size::ROW, false, cx);
    }
    let face = match e.presence.as_str() {
        "stale" => "asleep",
        "ended" => "finished",
        _ => "normal",
    };
    let resting = Agent {
        key: e.key.clone(),
        name: e.name.clone(),
        kind: "teammate".into(),
        presence: if e.presence == "ended" {
            "finished".into()
        } else {
            "waiting".into()
        },
        activity: Activity {
            state: face.into(),
            ..Default::default()
        },
        ..Default::default()
    };
    ui::avatar::agent_avatar(&resting, avatar_size::ROW, false, cx)
}

impl Render for LobbyScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        // The grid follows the width; cut the rows again when it changes.
        let (narrow, cols) = self.layout();
        if narrow != self.narrow || cols != self.cols {
            self.refresh(cx);
        }
        let stage = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(head(
                t(cx, "rooms.lobbyTitle"),
                Some(t(cx, "rooms.lobbyTagline")),
                &theme,
            ))
            .child(
                div().flex_1().min_h_0().child(
                    list(
                        self.list.clone(),
                        cx.processor(|this: &mut Self, ix, window, cx| {
                            super::secondary::full_row(this.render_row(ix, window, cx))
                        }),
                    )
                    .size_full(),
                ),
            );
        let mut body = div()
            .relative()
            .flex()
            .flex_1()
            .min_h_0()
            .size_full()
            .child(stage);
        if !self.narrow && self.view.is_some() {
            body = body.child(
                div()
                    .w(px(FEED_W))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .border_l_1()
                    .border_color(theme.c("line"))
                    .child(self.feed_column(&theme, cx)),
            );
        }
        kit::main_column(&theme)
            .relative()
            .child(body)
            .child(width_probe(self.width.clone(), cx.entity_id()))
    }
}
