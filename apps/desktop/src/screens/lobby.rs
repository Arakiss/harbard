//! The Lobby (`/lobby`): everyone who has come through this workspace, in
//! their own words. Left, the stage: the title, the counts, the lineup of
//! robots, the filter box and the cards grouped Here now / Around / Gone.
//! Right, the `#lobby` room and its composer. Below 900px the two stack
//! into one column.
//!
//! A character surface (`DESIGN.md` §3.1): the console's `rooms.css` is the
//! reference here, sizes, bands, pills and motion included.

use super::lobby_motion::{self as motion, Hover, Pose};
use super::rooms::RoomFeed;
use super::secondary::{self as kit, search_frame, sync_list, width_probe};
use crate::chat::{ComposeTarget, Composer, ComposerState};
use crate::derive::lobby::{
    Group, LOBBY, counts, filter, groups, harness_name, lineup, lobby_channel, presence_word,
};
use crate::derive::rooms::room_composer_notice;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{Agent, LobbyEntry, WorkspaceView};
use crate::nav::{self, Route};
use crate::robot::{self, Face};
use crate::shell::decor::{SeasonSleigh, season_sleigh};
use crate::theme::Theme;
use crate::ui::pill_lobby::{PillDot, bubble, presence_pill, tag_chip};
use crate::ui::{self, format::hace, leading, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, BoxShadow, Context, Entity, Hsla, InteractiveElement, IntoElement,
    ListAlignment, ListState, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Task, Window, div, hsla, linear_color_stop, linear_gradient, list, point,
    px,
};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long a card stays lit after the lineup points at it.
const FLASH: Duration = Duration::from_millis(1600);
/// The feed column, and the width under which the two columns stack.
const FEED_W: f32 = 360.;
const NARROW: f32 = 900.;
/// The stacked feed's height when the columns are one.
const FEED_STACKED_H: f32 = 520.;
const CARD_MIN: f32 = 286.;
const GAP: f32 = space::XL;
/// The stage's inset (`.hb-lobby-hero`, `.hb-lobby-group`: 32 px).
const PAD: f32 = space::BREAK;
/// The filter box's widest, and its height.
const FIND_W: f32 = 420.;
const FIND_H: f32 = 40.;
/// An introduction shows this many lines; the rest is in the agent's DM.
const INTRO_LINES: usize = 4;
/// The hero: 26 px over the title, `Lobby` at 32/900, the tagline at 16.
const HERO_TOP: f32 = 26.;
const HERO_TITLE: f32 = 32.;
const HERO_TAGLINE: f32 = 16.;
/// The shelf: 72 px robots on 84 px stands over a 2 px line.
const STAND_ROBOT: f32 = 72.;
const STAND_W: f32 = 84.;
/// A card: 14 px corners, a 72 px band in the robot's tile colour, a 104 px
/// portrait 14 px down and 16 px in, the name 134 px in.
const CARD_RADIUS: f32 = 14.;
const BAND_H: f32 = 72.;
const PORTRAIT: f32 = 104.;
const PORTRAIT_TOP: f32 = 14.;
const PORTRAIT_LEFT: f32 = 16.;
const ID_LEFT: f32 = 134.;
const ID_MIN_H: f32 = 62.;
const CARD_INSET: f32 = 14.;
/// A gone agent's portrait fades a little.
const GONE_OPACITY: f32 = 0.72;

#[derive(Clone)]
enum Row {
    Hero,
    Find,
    Quiet,
    NoMatch,
    Head(Group, usize),
    /// Each card's entry, and its place in the arrival.
    Cards(Vec<(usize, usize)>),
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
    sleigh: Entity<SeasonSleigh>,
    /// When each robot first showed up on this screen (the arrival), and
    /// when the screen first had something to show.
    arrived: HashMap<String, Instant>,
    opened: Option<Instant>,
    /// The pointer on a card (`card:<key>`) or a stand (`stand:<key>`).
    hovers: HashMap<String, Hover>,
    /// Introductions said after the app started: their time, and when this
    /// screen first showed them.
    said: HashMap<String, (u64, Instant)>,
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
            sleigh: season_sleigh(window, cx),
            arrived: HashMap::new(),
            opened: None,
            hovers: HashMap::new(),
            said: HashMap::new(),
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
            .and_then(|v| room_composer_notice(&lobby_channel(v), &**cx));
        self.composer.update(cx, |c, cx| {
            c.set_state(
                ComposerState {
                    notice,
                    ..Default::default()
                },
                cx,
            )
        });
        if let Some(v) = &view {
            self.note_arrivals(v);
        }
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
            let mut order = 0;
            for (group, list) in found {
                rows.push(Row::Head(group, list.len()));
                for chunk in list.chunks(self.cols) {
                    let cards = chunk
                        .iter()
                        .map(|e| {
                            order += 1;
                            (index(e), order - 1)
                        })
                        .collect();
                    rows.push(Row::Cards(cards));
                }
            }
        }
        rows.push(Row::Bottom);
        if self.narrow {
            rows.push(Row::Feed);
        }
        rows
    }

    /// Note who is new on this screen (they arrive) and which
    /// introductions are new since the app started (they pop).
    fn note_arrivals(&mut self, view: &WorkspaceView) {
        let now = Instant::now();
        self.opened.get_or_insert(now);
        let baseline = robot::clock::baseline_ms();
        for e in &view.lobby {
            self.arrived.entry(e.key.clone()).or_insert(now);
            let said = e.intro_ts_ms.filter(|_| e.intro.is_some());
            if let Some(ts) = said.filter(|_| motion::is_fresh(said, baseline))
                && self.said.get(&e.key).is_none_or(|(at, _)| *at != ts)
            {
                self.said.insert(e.key.clone(), (ts, now));
            }
        }
    }

    /// The pointer came onto (or left) a card or a stand.
    fn hovered(&mut self, id: String, on: bool, cx: &mut Context<Self>) {
        let reduce = robot::clock::reduced(cx);
        let from = self.lift_of(&id, id.starts_with("stand:"), reduce);
        self.hovers.insert(
            id,
            Hover {
                on,
                at: Instant::now(),
                from,
            },
        );
        cx.notify();
    }

    /// How far up (0..=1) a card or stand stands now.
    fn lift_of(&self, id: &str, stand: bool, reduce: bool) -> f32 {
        match self.hovers.get(id) {
            None => 0.,
            Some(h) if reduce => f32::from(u8::from(h.on)),
            Some(h) => motion::lift(h, since(h.at), stand),
        }
    }

    /// The portrait's hello while the pointer has just come onto its card.
    fn hello_of(&self, id: &str, reduce: bool) -> Pose {
        match self.hovers.get(id) {
            Some(h) if h.on && !reduce => motion::hello(since(h.at), PORTRAIT),
            _ => Pose::REST,
        }
    }

    /// Where a robot or card is in its arrival: staggered by `order` when
    /// it came with the screen, at once when it came later.
    fn arrival_of(&self, key: &str, order: usize, step: f32, reduce: bool) -> Pose {
        let (Some(opened), Some(&at)) = (self.opened, self.arrived.get(key)) else {
            return Pose::REST;
        };
        if reduce {
            return Pose::REST;
        }
        let order = if at == opened { order } else { 0 };
        motion::arrival(since(at), order, step)
    }

    /// A fresh introduction's pop.
    fn said_of(&self, key: &str, reduce: bool) -> Pose {
        match self.said.get(key) {
            Some((_, at)) if !reduce => motion::said(since(*at)),
            _ => Pose::REST,
        }
    }

    /// Whether anything on the stage is still moving.
    fn moving(&self, reduce: bool) -> bool {
        if reduce {
            return false;
        }
        let n = self.view.as_ref().map_or(0, |v| v.lobby.len());
        let arriving = self.opened.is_some_and(|at| {
            since(at) < motion::arrival_ends(n, motion::CARD_STEP_MS.max(motion::STAND_STEP_MS))
        }) || self
            .arrived
            .values()
            .any(|at| since(*at) < motion::ARRIVE_MS);
        let hovering = self
            .hovers
            .values()
            .any(|h| since(h.at) < motion::HELLO_MS.max(motion::LIFT_MS));
        let saying = self
            .said
            .values()
            .any(|(_, at)| since(*at) < motion::SAID_MS);
        arriving || hovering || saying
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
            .position(|r| matches!(r, Row::Cards(list) if list.iter().any(|&(i, _)| i == at)))
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
                .pt(px(18.))
                .px(px(PAD))
                .child(
                    // `.hb-search` at the console's size here: 40 px, 8 px corners.
                    div().max_w(px(FIND_W)).child(
                        search_frame(
                            Input::new(&self.search)
                                .appearance(false)
                                .text_size(px(size::BODY)),
                            self.find_focused,
                            &theme,
                        )
                        .h(px(FIND_H))
                        .px(px(space::LG))
                        .rounded(px(ui::radius::MD)),
                    ),
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
            // `.hb-lobby-group h2`: 15/900, its count light beside it.
            Row::Head(group, n) => div()
                .px(px(PAD))
                .pt(px(space::SECTION))
                .pb(px(space::LG))
                .flex()
                .items_baseline()
                .gap(px(space::XS))
                .child(ui::text(
                    group.title(cx),
                    size::BODY,
                    weight::HEAVY,
                    theme.c("ink"),
                ))
                .child(ui::text(
                    n.to_string(),
                    size::BODY,
                    weight::REGULAR,
                    theme.c("ink-3"),
                ))
                .into_any_element(),
            Row::Cards(list) => {
                let w = self.card_width();
                let mut row = div().px(px(PAD)).pb(px(GAP)).flex().gap(px(GAP));
                for &(i, order) in &list {
                    row = row.child(div().w(px(w)).flex_none().flex().child(self.card(
                        &view,
                        &view.lobby[i],
                        order,
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

    /// The hero (`.hb-lobby-hero`): a band shading from `tint` to the
    /// surface, the title, the tagline, the counts and the shelf of robots
    /// of the moment, with the season's sleigh crossing its top.
    fn hero(&self, view: &WorkspaceView, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (here, around, gone) = counts(&view.lobby);
        let (shown, more) = lineup(&view.lobby);
        let count = |key: &str, n: usize, ink: &str| {
            ui::text(
                tv(cx, key, &[("count", &n.to_string())]),
                size::SMALL,
                weight::STRONG,
                theme.c(ink),
            )
        };
        let counts = div()
            .mt(px(10.))
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
        let reduce = robot::clock::reduced(cx);
        let shelf = (!shown.is_empty()).then(|| {
            let mut ul = div()
                .mt(px(18.))
                .pb(px(space::HAIR))
                .flex()
                .flex_wrap()
                .items_end()
                .gap_x(px(space::SM))
                .gap_y(px(space::XS))
                .border_b_2()
                .border_color(theme.c("line-strong"));
            for (i, e) in shown.into_iter().enumerate() {
                let key = e.key.clone();
                let id = format!("stand:{}", e.key);
                let arrive = self.arrival_of(&e.key, i, motion::STAND_STEP_MS, reduce);
                let up = self.lift_of(&id, true, reduce) * motion::STAND_LIFT;
                let agent = view.agent(&e.key);
                ul = ul.child(
                    div()
                        .relative()
                        .top(px(arrive.dy - up))
                        .opacity(arrive.opacity)
                        .child(
                            kit::control_focus(div().id(SharedString::from(id.clone())), theme)
                                .w(px(STAND_W))
                                .pt(px(space::XS))
                                .px(px(space::HAIR))
                                .pb(px(space::SM))
                                .flex()
                                .flex_col()
                                .items_center()
                                .gap(px(space::HAIR))
                                .rounded_t(px(10.))
                                .cursor_pointer()
                                .tooltip(kit::tip(e.name.clone().into()))
                                .on_hover(cx.listener(move |this, on: &bool, _, cx| {
                                    this.hovered(id.clone(), *on, cx)
                                }))
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.point(key.clone(), cx)),
                                )
                                .child(portrait(e, agent, STAND_ROBOT))
                                .child(
                                    ui::text(
                                        e.name.clone(),
                                        size::META,
                                        weight::STRONG,
                                        theme.c("ink-2"),
                                    )
                                    .max_w_full()
                                    .truncate(),
                                ),
                        ),
                );
            }
            if more > 0 {
                ul = ul.child(
                    ui::text(
                        tv(cx, "rooms.lobbyLineupMore", &[("count", &more.to_string())]),
                        size::UI,
                        weight::HEAVY,
                        theme.c("ink-3"),
                    )
                    .self_center()
                    .px(px(10.)),
                );
            }
            ul
        });
        div()
            .relative()
            .pt(px(HERO_TOP))
            .px(px(PAD))
            .bg(linear_gradient(
                180.,
                linear_color_stop(theme.c("tint"), 0.),
                linear_color_stop(theme.c("surface"), 1.),
            ))
            .child(self.sleigh.clone())
            .child(
                div()
                    .font_family(ui::SANS)
                    .text_size(px(HERO_TITLE))
                    .line_height(px((HERO_TITLE * 1.1).round()))
                    .font_weight(weight::HEAVY)
                    .text_color(theme.c("ink"))
                    .child(t(cx, "rooms.lobbyTitle")),
            )
            .child(
                div()
                    .mt(px(space::SM))
                    .font_family(ui::SANS)
                    .text_size(px(HERO_TAGLINE))
                    .line_height(px(leading::BODY))
                    .text_color(theme.c("ink-2"))
                    .child(t(cx, "rooms.lobbyTagline")),
            )
            .child(counts)
            .children(shelf)
            .into_any_element()
    }

    /// One card (`.hb-lobby-card`): a band in its robot's tile colour with
    /// the presence pill, the 104 px portrait over it, the name and its
    /// chips, what it said in a bubble, when it was seen.
    fn card(
        &self,
        view: &WorkspaceView,
        e: &LobbyEntry,
        order: usize,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let agent = view.agent(&e.key);
        let lit = self.lit.as_deref() == Some(e.key.as_str());
        let now = view.now_ms;
        let reduce = robot::clock::reduced(cx);
        let id = format!("card:{}", e.key);
        let hover = self.hovers.get(&id).is_some_and(|h| h.on);
        let arrive = self.arrival_of(&e.key, order, motion::CARD_STEP_MS, reduce);
        let up = self.lift_of(&id, false, reduce) * motion::CARD_LIFT;
        let hello = self.hello_of(&id, reduce);
        let said_pose = self.said_of(&e.key, reduce);
        let name = agent.map_or(e.name.as_str(), |a| crate::derive::robot_name(a));
        let tile = theme.c(&format!("robot-tile-{}", robot::robot_for(name).tile));

        let tags = div()
            .mt(px(space::XS))
            .flex()
            .flex_wrap()
            .gap(px(space::XS))
            .child(tag_chip(harness_name(&e.harness, cx), false, theme))
            .children(
                e.project
                    .as_deref()
                    .filter(|p| !p.is_empty())
                    .map(|p| tag_chip(p.to_string(), true, theme)),
            );
        let ident = div()
            .min_h(px(ID_MIN_H))
            .pt(px(10.))
            .pr(px(CARD_INSET))
            .pl(px(ID_LEFT))
            .child(
                ui::text(e.name.clone(), size::TITLE, weight::HEAVY, theme.c("ink"))
                    .line_height(px((size::TITLE * 1.2).round()))
                    .truncate(),
            )
            .child(tags);

        let said: AnyElement = match &e.intro {
            Some(intro) => bubble(false, theme)
                .relative()
                .top(px(said_pose.dy))
                .opacity(said_pose.opacity)
                .text_size(px(size::BODY))
                .line_height(px(21.))
                .text_color(theme.c("ink"))
                .child(
                    div()
                        .child(intro.clone())
                        .line_clamp(INTRO_LINES)
                        .text_ellipsis(),
                )
                .children(e.intro_ts_ms.map(|ts| {
                    ui::text(
                        tv(cx, "rooms.lobbySaid", &[("when", &hace(ts, now, cx))]),
                        size::META,
                        weight::REGULAR,
                        theme.c("ink-3"),
                    )
                    .mt(px(space::XS))
                }))
                .into_any_element(),
            None => bubble(true, theme)
                .text_size(px(size::UI))
                .line_height(px(leading::UI))
                .child(t(cx, "rooms.lobbyNoIntro"))
                .children(agent.and_then(|a| a.description.clone()).map(|what| {
                    div()
                        .mt(px(space::HAIR))
                        .text_color(theme.c("ink-2"))
                        .child(tv(cx, "rooms.lobbyKnownDoing", &[("what", &what)]))
                        .line_clamp(INTRO_LINES - 1)
                        .text_ellipsis()
                }))
                .into_any_element(),
        };
        let seen = if e.presence == "live" {
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
        };

        let band = div()
            .h(px(BAND_H))
            .flex()
            .justify_end()
            .items_start()
            .py(px(10.))
            .px(px(space::LG))
            .rounded_t(px(CARD_RADIUS - 1.))
            .bg(tile)
            .child(presence_pill(
                presence_word(&e.presence, cx),
                pill_dot(&e.presence),
                theme,
            ));
        let face = div()
            .absolute()
            .top(px(PORTRAIT_TOP + hello.dy))
            .left(px(PORTRAIT_LEFT + hello.dx))
            .when(e.presence == "ended", |d| d.opacity(GONE_OPACITY))
            .child(portrait(e, agent, PORTRAIT));

        let target = agent.map(|a| a.key.clone());
        let card = div()
            .id(SharedString::from(format!("lobby-{}", e.key)))
            .relative()
            .top(px(arrive.dy - up))
            .opacity(arrive.opacity)
            .flex_1()
            .min_w_0()
            .flex()
            .flex_col()
            .rounded(px(CARD_RADIUS))
            .bg(theme.c("surface-raised"))
            .border_1()
            .border_color(theme.c(if hover { "line-hover" } else { "line" }))
            .when(hover, |d| d.shadow(tools_shadow(theme)))
            .on_hover(cx.listener(move |this, on: &bool, _, cx| this.hovered(id.clone(), *on, cx)));
        let card = match target {
            Some(key) => kit::control_focus(card, theme)
                .cursor_pointer()
                .on_click(move |_, _, cx| nav::navigate(Route::direct(Some(key.clone())), cx)),
            None => card,
        };
        card.child(band)
            .child(ident)
            .child(div().mt(px(space::LG)).mx(px(CARD_INSET)).child(said))
            .child(
                ui::text(seen, size::META, weight::REGULAR, theme.c("ink-3"))
                    .mt(px(space::MD))
                    .mx(px(CARD_INSET))
                    .mb(px(space::LG))
                    .truncate(),
            )
            .child(face)
            // The lineup pointed here: a 2 px accent ring, for a moment.
            .when(lit, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(-1.))
                        .left(px(-1.))
                        .right(px(-1.))
                        .bottom(px(-1.))
                        .rounded(px(CARD_RADIUS))
                        .border_2()
                        .border_color(theme.c("accent")),
                )
            })
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

/// The dot in a card's presence pill: green here, amber idle, grey gone
/// quiet, a grey ring gone.
fn pill_dot(presence: &str) -> PillDot {
    match presence {
        "live" => PillDot::Filled("green"),
        "idle" => PillDot::Filled("amber"),
        "ended" => PillDot::Ring,
        _ => PillDot::Filled("grey"),
    }
}

/// `--hb-tools-shadow` under a lifted card: `0 1px 3px`, 8 % black in a
/// light theme, 40 % in a dark one.
fn tools_shadow(theme: &Theme) -> Vec<BoxShadow> {
    let color: Hsla = hsla(0., 0., 0., if theme.dark { 0.4 } else { 0.08 });
    vec![BoxShadow {
        color,
        offset: point(px(0.), px(1.)),
        blur_radius: px(3.),
        spread_radius: px(0.),
        inset: false,
    }]
}

/// The face a robot without a live agent rests with, by its presence.
fn resting_face(presence: &str) -> Face {
    match presence {
        "stale" => Face::Asleep,
        "ended" => Face::Finished,
        _ => Face::Normal,
    }
}

/// Its live robot when the view has the agent; otherwise the same robot at
/// rest, by name, with a face for its presence.
fn portrait(e: &LobbyEntry, agent: Option<&Agent>, size: f32) -> AnyElement {
    match agent {
        Some(a) => robot::big(Some(a), &e.name, size).into_any_element(),
        None => robot::robot(e.name.clone(), size)
            .face(resting_face(&e.presence))
            .into_any_element(),
    }
}

impl Render for LobbyScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        // The arrival, a lift, a hello or a pop under way: draw the next frame.
        if self.moving(robot::clock::reduced(cx)) {
            crate::perf::count(crate::perf::Source::Lobby, 1);
            window.request_animation_frame();
        }
        // The grid follows the width; cut the rows again when it changes.
        let (narrow, cols) = self.layout();
        if narrow != self.narrow || cols != self.cols {
            self.refresh(cx);
        }
        let stage = div().flex().flex_col().flex_1().min_w_0().min_h_0().child(
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

/// Milliseconds since `at`.
fn since(at: Instant) -> f32 {
    at.elapsed().as_secs_f32() * 1000.
}
