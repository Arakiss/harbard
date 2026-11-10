//! The Lobby (`/lobby`): everyone who has come through this workspace, in
//! their own words. Left, the stage: the intro with the counts, the lineup
//! of robots, the filter box and the cards grouped Here now / Around /
//! Gone. Right, the `#lobby` room and its composer. Below 900px the two
//! stack into one column.

use super::room_feed::RoomFeed;
use super::secondary::{self as kit, search_frame, sync_list, width_probe};
use crate::chat::{ComposeTarget, Composer};
use crate::derive::lobby::{
    Group, LOBBY, counts, filter, groups, harness_name, lineup, lobby_channel, presence_word, robot_tile, room_notice,
};
use crate::derive::robot_name;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{Activity, Agent, LobbyEntry, WorkspaceView};
use crate::nav::{self, Route};
use crate::theme::Theme;
use crate::ui::{self, format::hace, size};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, BoxShadow, Context, Entity, FontWeight, InteractiveElement, IntoElement,
    ListAlignment, ListState, ParentElement, PathBuilder, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, Window, canvas, div, linear_color_stop, linear_gradient, list, point, px,
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
const CARD_MIN: f32 = 286.;
const GAP: f32 = 16.;
const PAD: f32 = 32.;

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
    seen_unread: u64,
    /// Intros newer than this arrived while the screen was open.
    baseline: u64,
    _flash: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl LobbyScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |this, _, cx| this.refresh(cx)).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        cx.observe_global_in::<crate::i18n::I18n>(window, |this, window, cx| {
            let placeholder = t(cx, "rooms.lobbyFindPlaceholder");
            this.search
                .update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
            this.refresh(cx)
        })
        .detach();
        let placeholder = t(cx, "rooms.lobbyFindPlaceholder");
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let subs = vec![cx.subscribe(&search, |this, _, ev: &InputEvent, cx| {
            if matches!(ev, InputEvent::Change) {
                this.refresh(cx);
            }
        })];
        let feed = cx.new(|cx| RoomFeed::new(LOBBY.into(), false, t(cx, "rooms.lobbyFeedEmpty"), cx));
        let composer = cx.new(|cx| Composer::new(ComposeTarget::Channel(LOBBY.into()), window, cx));
        let write_in = tv(cx, "rooms.writeIn", &[("room", LOBBY)]);
        composer.update(cx, |c, cx| {
            c.set_target(ComposeTarget::Channel(LOBBY.into()), write_in, cx)
        });
        let baseline = Desk::get(cx).read(cx).now_ms();
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
            seen_unread: 0,
            baseline,
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
            let index = |e: &LobbyEntry| entries.iter().position(|x| std::ptr::eq(x, e)).unwrap_or(0);
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

    fn render_row(&mut self, ix: usize, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let (Some(view), Some(row)) = (self.view.clone(), self.rows.get(ix).cloned()) else {
            return div().into_any_element();
        };
        let theme = cx.global::<Theme>().clone();
        match row {
            Row::Hero => self.hero(&view, &theme, cx),
            Row::Find => div()
                .pt(px(18.))
                .px(px(PAD))
                .child(div().max_w(px(420.)).child(search_frame(
                    Input::new(&self.search).appearance(false).text_size(px(size::BODY)),
                    &theme,
                )))
                .into_any_element(),
            Row::Quiet => kit::empty_block(
                Some(t(cx, "rooms.lobbyEmptyTitle")),
                vec![
                    t(cx, "rooms.lobbyEmptyBody").into_any_element(),
                    div()
                        .flex()
                        .child(kit::mono("harbard intro"))
                        .child(".")
                        .into_any_element(),
                ],
                &theme,
            )
            .into_any_element(),
            Row::NoMatch => kit::empty_block(
                None,
                vec![tv(cx, "rooms.lobbyNoMatch", &[("query", self.query(cx).trim())]).into_any_element()],
                &theme,
            )
            .into_any_element(),
            Row::Head(group, n) => div()
                .px(px(PAD))
                .pt(px(24.))
                .pb(px(12.))
                .flex()
                .items_baseline()
                .gap(px(4.))
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::BLACK)
                        .child(group.title(cx)),
                )
                .child(
                    div()
                        .text_size(px(15.))
                        .text_color(theme.c("ink-3"))
                        .child(n.to_string()),
                )
                .into_any_element(),
            Row::Cards(list) => {
                let w = self.card_width();
                let mut row = div().px(px(PAD)).pb(px(GAP)).flex().items_start().gap(px(GAP));
                for &i in &list {
                    row = row.child(
                        div()
                            .w(px(w))
                            .flex_none()
                            .child(self.card(&view, &view.lobby[i], &theme, cx)),
                    );
                }
                row.into_any_element()
            }
            Row::Feed => div()
                .h(px(520.))
                .flex()
                .flex_col()
                .border_t_1()
                .border_color(theme.c("line"))
                .child(self.feed_column(&view, &theme, cx))
                .into_any_element(),
            Row::Bottom => div().h(px(28. - GAP)).into_any_element(),
        }
    }

    /// The intro, the counts and the robots of the moment on one shelf.
    fn hero(&self, view: &WorkspaceView, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let (here, around, gone) = counts(&view.lobby);
        let (shown, more) = lineup(&view.lobby);
        let shelf = (!shown.is_empty()).then(|| {
            let mut ul = div()
                .flex()
                .flex_wrap()
                .items_end()
                .gap_x(px(6.))
                .gap_y(px(4.))
                .mt(px(18.))
                .pb(px(2.))
                .border_b_2()
                .border_color(theme.c("line-strong"));
            for e in shown {
                let key = e.key.clone();
                let agent = view.agent(&e.key);
                ul = ul.child(
                    div()
                        .id(SharedString::from(format!("stand-{}", e.key)))
                        .relative()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(px(2.))
                        .w(px(84.))
                        .pt(px(4.))
                        .px(px(2.))
                        .pb(px(6.))
                        .cursor_pointer()
                        .hover(|s| s.top(px(-5.)))
                        .on_click(cx.listener(move |this, _, _, cx| this.point(key.clone(), cx)))
                        .child(portrait(e, agent, 72., cx))
                        .child(
                            div()
                                .max_w_full()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_size(px(size::META))
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.c("ink-2"))
                                .child(e.name.clone()),
                        ),
                );
            }
            if more > 0 {
                ul = ul.child(
                    div()
                        .self_center()
                        .px(px(10.))
                        .font_weight(FontWeight::BLACK)
                        .text_color(theme.c("ink-3"))
                        .child(tv(cx, "rooms.lobbyLineupMore", &[("count", &more.to_string())])),
                );
            }
            ul
        });
        div()
            .pt(px(26.))
            .px(px(PAD))
            .bg(linear_gradient(
                180.,
                linear_color_stop(theme.c("tint"), 0.),
                linear_color_stop(theme.c("surface"), 1.),
            ))
            .child(
                div()
                    .text_size(px(32.))
                    .line_height(px(35.))
                    .font_weight(FontWeight::BLACK)
                    .child(t(cx, "rooms.lobbyTitle")),
            )
            .child(
                div()
                    .mt(px(6.))
                    .text_size(px(16.))
                    .text_color(theme.c("ink-2"))
                    .child(t(cx, "rooms.lobbyTagline")),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_x(px(16.))
                    .gap_y(px(4.))
                    .mt(px(10.))
                    .text_size(px(size::SMALL))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.c("ink-3"))
                    .child(div().text_color(theme.c("green-ink")).child(tv(
                        cx,
                        "rooms.lobbyCountHere",
                        &[("count", &here.to_string())],
                    )))
                    .child(tv(cx, "rooms.lobbyCountAround", &[("count", &around.to_string())]))
                    .child(tv(cx, "rooms.lobbyCountGone", &[("count", &gone.to_string())])),
            )
            .children(shelf)
            .into_any_element()
    }

    /// One card: band in its robot's tile, the robot, name and tags, what it
    /// said, when it was seen.
    fn card(&self, view: &WorkspaceView, e: &LobbyEntry, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let agent = view.agent(&e.key);
        let tile = robot_tile(agent.map(robot_name).unwrap_or(&e.name));
        let lit = self.lit.as_deref() == Some(e.key.as_str());
        let fresh = e.intro_ts_ms.is_some_and(|ms| ms > self.baseline);
        let now = view.now_ms;
        let tag = |text: SharedString, mono: bool| {
            div()
                .flex()
                .items_center()
                .h(px(20.))
                .max_w_full()
                .px(px(6.))
                .rounded(px(5.))
                .bg(theme.c("chip"))
                .text_color(theme.c("chip-ink"))
                .text_size(px(size::META))
                .font_weight(FontWeight::BOLD)
                .overflow_hidden()
                .whitespace_nowrap()
                .when(mono, |d| d.font_family(ui::MONO))
                .child(text)
        };
        let bubble = match &e.intro {
            Some(intro) => div()
                .relative()
                .mt(px(12.))
                .mx(px(14.))
                .px(px(12.))
                .py(px(10.))
                .rounded(px(12.))
                .bg(theme.c("chip"))
                .text_size(px(15.))
                .line_height(px(21.))
                .when(fresh, |d| d.border_1().border_color(theme.c("accent-soft")))
                .child(tail(theme.c("chip")))
                .child(intro.clone())
                .child(
                    div()
                        .mt(px(4.))
                        .text_size(px(size::META))
                        .text_color(theme.c("ink-3"))
                        .child(tv(
                            cx,
                            "rooms.lobbySaid",
                            &[("when", &hace(e.intro_ts_ms.unwrap_or(now), now, cx))],
                        )),
                ),
            None => div()
                .mt(px(12.))
                .mx(px(14.))
                .px(px(12.))
                .py(px(10.))
                .rounded(px(12.))
                .border_1()
                .border_color(theme.c("line-strong"))
                .text_color(theme.c("ink-3"))
                .text_size(px(size::UI))
                .line_height(px(20.))
                .child(t(cx, "rooms.lobbyNoIntro"))
                .children(agent.and_then(|a| a.description.clone()).map(|what| {
                    div().mt(px(2.)).text_color(theme.c("ink-2")).child(tv(
                        cx,
                        "rooms.lobbyKnownDoing",
                        &[("what", &what)],
                    ))
                })),
        };
        let seen = if e.presence == "live" {
            tv(cx, "rooms.lobbyFirstSeen", &[("when", &hace(e.first_ms, now, cx))])
        } else {
            tv(cx, "rooms.lobbyLastSeen", &[("when", &hace(e.last_ms, now, cx))])
        };
        let ring = if lit { theme.c("accent") } else { theme.c("line") };
        let hover_ring = theme.c("line-hover");
        let lift = BoxShadow {
            color: gpui::hsla(0., 0., 0., if theme.dark { 0.35 } else { 0.12 }),
            offset: point(px(0.), px(6.)),
            blur_radius: px(18.),
            spread_radius: px(0.),
            inset: false,
        };
        let target = agent.map(|a| a.key.clone());
        div()
            .id(SharedString::from(format!("lobby-{}", e.key)))
            .relative()
            .w_full()
            .flex()
            .flex_col()
            .rounded(px(14.))
            .bg(theme.c("surface-raised"))
            .border_color(ring)
            .when(lit, |d| d.border_2())
            .when(!lit, |d| d.border_1())
            .hover(move |s| s.top(px(-3.)).border_color(hover_ring).shadow(vec![lift.clone()]))
            .when_some(target, |d, key| {
                d.cursor_pointer()
                    .on_click(move |_, _, cx| nav::navigate(Route::direct(Some(key.clone())), cx))
            })
            .child(
                div()
                    .h(px(72.))
                    .flex()
                    .justify_end()
                    .items_start()
                    .px(px(12.))
                    .py(px(10.))
                    .rounded_t(px(13.))
                    .bg(theme.c(&format!("robot-tile-{tile}")))
                    .child(presence_pill(&e.presence, theme, cx)),
            )
            .child(
                div()
                    .absolute()
                    .top(px(14.))
                    .left(px(16.))
                    .when(e.presence == "ended", |d| d.opacity(0.72))
                    .child(portrait(e, agent, 104., cx)),
            )
            .child(
                div()
                    .min_h(px(62.))
                    .pt(px(10.))
                    .pr(px(14.))
                    .pl(px(134.))
                    .child(
                        div()
                            .text_size(px(18.))
                            .line_height(px(21.6))
                            .font_weight(FontWeight::BLACK)
                            .child(e.name.clone()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap(px(4.))
                            .mt(px(4.))
                            .child(tag(harness_name(&e.harness, cx), false))
                            .children(e.project.clone().map(|p| tag(p.into(), true))),
                    ),
            )
            .child(bubble)
            .child(
                div()
                    .mt(px(8.))
                    .mx(px(14.))
                    .mb(px(12.))
                    .text_size(px(size::META))
                    .text_color(theme.c("ink-3"))
                    .child(seen),
            )
            .into_any_element()
    }

    /// `#lobby`: its head, the room's feed, the notice and the composer.
    fn feed_column(&self, view: &WorkspaceView, theme: &Theme, cx: &App) -> impl IntoElement {
        let notice = room_notice(&lobby_channel(view), cx);
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(
                div()
                    .h(px(56.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .pl(px(18.))
                    .pr(px(12.))
                    .border_b_1()
                    .border_color(theme.c("line"))
                    .child(
                        div()
                            .text_size(px(size::TITLE))
                            .font_weight(FontWeight::BLACK)
                            .child(t(cx, "rooms.lobbyFeedTitle")),
                    ),
            )
            .child(self.feed.clone())
            .children(notice.map(|n| {
                div()
                    .mx(px(20.))
                    .mb(px(-14.))
                    .px(px(10.))
                    .py(px(6.))
                    .rounded(px(6.))
                    .bg(theme.c("warn-bg"))
                    .text_color(theme.c("warn-ink"))
                    .text_size(px(size::SMALL))
                    .child(sentence(&n))
            }))
            .child(self.composer.clone())
    }
}

/// The notice as the composer shows it: a capital first, a full stop last.
fn sentence(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

/// The bubble's tail towards its robot.
fn tail(color: gpui::Hsla) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let o = bounds.origin;
            let mut path = PathBuilder::fill();
            path.move_to(point(o.x, o.y + px(8.)));
            path.line_to(point(o.x + px(9.), o.y));
            path.line_to(point(o.x + px(18.), o.y + px(8.)));
            path.close();
            if let Ok(p) = path.build() {
                window.paint_path(p, color);
            }
        },
    )
    .absolute()
    .top(px(-8.))
    .left(px(42.))
    .w(px(18.))
    .h(px(8.))
}

fn presence_pill(presence: &str, theme: &Theme, cx: &App) -> impl IntoElement {
    let dot = div().size(px(9.)).rounded_full().flex_none();
    let dot = match presence {
        "live" => dot.bg(theme.c("green")),
        "idle" => dot.bg(theme.c("amber")),
        "ended" => dot.border_2().border_color(theme.c("grey")),
        _ => dot.bg(theme.c("grey")),
    };
    div()
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(24.))
        .pl(px(8.))
        .pr(px(9.))
        .rounded_full()
        .bg(theme.c("surface-raised"))
        .text_size(px(size::META))
        .font_weight(FontWeight::BOLD)
        .text_color(theme.c("ink-2"))
        .child(dot)
        .child(presence_word(presence, cx))
}

/// Its live robot when the view has the agent; otherwise the same robot at
/// rest, by name, with a face for its presence.
fn portrait(e: &LobbyEntry, agent: Option<&Agent>, size: f32, cx: &App) -> AnyElement {
    if let Some(a) = agent {
        return ui::avatar::agent_avatar(a, size, false, cx);
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
    ui::avatar::agent_avatar(&resting, size, false, cx)
}

impl Render for LobbyScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        // The grid follows the width; cut the rows again when it changes.
        let (narrow, cols) = self.layout();
        if narrow != self.narrow || cols != self.cols {
            self.refresh(cx);
        }
        let stage = div().flex_1().min_w_0().min_h_0().child(
            list(
                self.list.clone(),
                cx.processor(|this: &mut Self, ix, window, cx| {
                    super::secondary::full_row(this.render_row(ix, window, cx))
                }),
            )
            .size_full(),
        );
        let mut body = div().relative().flex().flex_1().min_h_0().size_full().child(stage);
        if !self.narrow
            && let Some(view) = self.view.clone()
        {
            body = body.child(
                div()
                    .w(px(FEED_W))
                    .flex_none()
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .border_l_1()
                    .border_color(theme.c("line"))
                    .child(self.feed_column(&view, &theme, cx)),
            );
        }
        kit::main_column(&theme)
            .relative()
            .child(body)
            .child(width_probe(self.width.clone(), cx.entity_id()))
    }
}
