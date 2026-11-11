//! History (`/history`, under "More tools"): the month the shell does not
//! keep, one page at a time from `GET /history/agents`, the next asked for
//! as the end of the list comes near. The channel filter and the search
//! (250 ms after typing stops) are serve's, so they reach the whole month.
//! Deleted rooms sit on top.

use super::secondary::{self as kit, head, main_column, search_frame, sync_list, workspace};
use crate::api::enc;
use crate::derive::history::{
    PAGE_SIZE, by_day, day_title, ending_id, ending_text, ending_tone, merge_page, path_tail,
    query, read_only, role_line, span_text,
};
use crate::desk::Desk;
use crate::i18n::{I18n, t, tv};
use crate::model::history::{DeletedRoom, HistoryChannel, HistoryItem, HistoryPage};
use crate::nav::{self, Nav, Route};
use crate::theme::Theme;
use crate::ui::{
    self, avatar_size, control,
    format::{at_date, short_path},
    row, size, space, weight,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement,
    KeyDownEvent, ListAlignment, ListState, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, list, px,
};
use gpui_kit::component::IndexPath;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use std::time::Duration;

const DEBOUNCE: Duration = Duration::from_millis(250);
/// The search box never gets narrower than this; the filter wraps below.
const SEARCH_MIN: f32 = 320.;
/// A name longer than this is cut, so the state beside it stays in view.
const NAME_MAX: f32 = 280.;
/// The channel filter's menu.
const MENU_W: f32 = 280.;
const MENU_MAX_H: f32 = 360.;

/// One option of the channel filter.
#[derive(Clone)]
pub struct ChannelOption {
    id: String,
    label: SharedString,
}

impl SearchableListItem for ChannelOption {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &String {
        &self.id
    }
}

#[derive(Clone)]
enum Row {
    Deleted,
    Error,
    Empty,
    Day(u64),
    Item(usize),
    More,
    End,
}

pub struct HistoryScreen {
    search: Entity<InputState>,
    select: Entity<SelectState<Vec<ChannelOption>>>,
    channel: String,
    query: String,
    items: Vec<HistoryItem>,
    next: Option<String>,
    total: u64,
    channels: Vec<HistoryChannel>,
    now: u64,
    days: u64,
    loading: bool,
    error: Option<String>,
    deleted: Vec<DeletedRoom>,
    list: ListState,
    rows: Vec<Row>,
    /// The options the filter holds now.
    synced: Vec<(String, SharedString)>,
    search_focused: bool,
    focus: FocusHandle,
    /// The row the keyboard is on (K4), as an index into `rows`.
    cursor: Option<usize>,
    _load: Option<Task<()>>,
    _debounce: Option<Task<()>>,
    _deleted: Option<Task<()>>,
    _subs: Vec<Subscription>,
}

impl HistoryScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (channel, q) = match cx.global::<Nav>().route.clone() {
            Route::History { channel, q } => (channel.unwrap_or_default(), q),
            _ => (String::new(), String::new()),
        };
        let placeholder = t(cx, "history.searchPlaceholder");
        let search = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .default_value(q.clone())
        });
        let select = cx.new(|cx| SelectState::new(Vec::<ChannelOption>::new(), None, window, cx));
        let subs = vec![
            cx.subscribe(&search, |this, _, ev: &InputEvent, cx| match ev {
                InputEvent::Change => this.debounce(cx),
                InputEvent::Focus | InputEvent::Blur => {
                    this.search_focused = matches!(ev, InputEvent::Focus);
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {}
            }),
            cx.subscribe(
                &select,
                |this, _, ev: &SelectEvent<Vec<ChannelOption>>, cx| {
                    let SelectEvent::Confirm(value) = ev;
                    let channel = value.clone().unwrap_or_default();
                    if channel != this.channel {
                        this.channel = channel;
                        this.remember(cx);
                        this.load(None, cx);
                    }
                },
            ),
        ];
        cx.observe(&Desk::get(cx), |this, _, cx| this.rebuild(cx))
            .detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        cx.observe_global_in::<I18n>(window, |this, window, cx| {
            let placeholder = t(cx, "history.searchPlaceholder");
            this.search
                .update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
            this.sync_select(window, cx);
            this.rebuild(cx);
        })
        .detach();
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut this = HistoryScreen {
            search_focused: false,
            focus,
            cursor: None,
            search,
            select,
            channel,
            query: q,
            items: Vec::new(),
            next: None,
            total: 0,
            channels: Vec::new(),
            now: 0,
            days: 31,
            loading: true,
            error: None,
            deleted: Vec::new(),
            list: ListState::new(0, ListAlignment::Top, px(600.)),
            rows: Vec::new(),
            synced: Vec::new(),
            _load: None,
            _debounce: None,
            _deleted: None,
            _subs: subs,
        };
        this.load(None, cx);
        this.load_deleted(cx);
        this.sync_select(window, cx);
        this
    }

    /// The search waits for a pause in typing.
    fn debounce(&mut self, cx: &mut Context<Self>) {
        let timer = cx.background_executor().timer(DEBOUNCE);
        self._debounce = Some(cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |this, cx| {
                let q = this.search.read(cx).value().to_string();
                if q != this.query {
                    this.query = q;
                    this.remember(cx);
                    this.load(None, cx);
                }
            });
        }));
    }

    /// The route keeps both choices, like the console's URL.
    fn remember(&self, cx: &mut App) {
        let channel = (!self.channel.is_empty()).then(|| self.channel.clone());
        nav::replace(
            Route::History {
                channel,
                q: self.query.trim().to_string(),
            },
            cx,
        );
    }

    /// One page; a new load drops the one in flight.
    fn load(&mut self, before: Option<String>, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = None;
        if before.is_none() {
            self.items.clear();
            self.next = None;
        }
        let ws = workspace(cx);
        let path = format!(
            "/history/agents{}",
            query(
                before.as_deref(),
                &self.channel,
                &self.query,
                PAGE_SIZE,
                ws.as_deref()
            )
        );
        let task = kit::fetch::<HistoryPage>(path, cx);
        let more = before.is_some();
        self._load = Some(cx.spawn(async move |this, cx| {
            let out = task.await;
            let _ = this.update(cx, |this, cx| {
                this.loading = false;
                match out {
                    Ok(page) => {
                        if more {
                            merge_page(&mut this.items, page.items);
                        } else {
                            this.items = page.items;
                        }
                        this.next = page.next;
                        this.total = page.total;
                        this.channels = page.channels;
                        this.now = page.now_ms;
                        this.days = page.window_days;
                    }
                    Err(e) => this.error = Some(e.message),
                }
                this.rebuild(cx);
            });
        }));
        self.rebuild(cx);
    }

    fn load_deleted(&mut self, cx: &mut Context<Self>) {
        let path = match workspace(cx) {
            Some(ws) => format!("/channels/deleted?ws={}", enc(&ws)),
            None => "/channels/deleted".to_string(),
        };
        let task = kit::fetch::<Vec<DeletedRoom>>(path, cx);
        self._deleted = Some(cx.spawn(async move |this, cx| {
            let rooms = task.await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.deleted = rooms;
                this.rebuild(cx);
            });
        }));
    }

    /// The filter's options: every channel of the month, and the chosen one
    /// even when the month no longer lists it.
    fn sync_select(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut options = vec![ChannelOption {
            id: String::new(),
            label: t(cx, "history.allChannels"),
        }];
        options.extend(self.channels.iter().map(|c| ChannelOption {
            id: c.id.clone(),
            label: format!("#{} ({})", c.id, c.items).into(),
        }));
        if !self.channel.is_empty() && !self.channels.iter().any(|c| c.id == self.channel) {
            options.push(ChannelOption {
                id: self.channel.clone(),
                label: format!("#{}", self.channel).into(),
            });
        }
        let key: Vec<(String, SharedString)> = options
            .iter()
            .map(|o| (o.id.clone(), o.label.clone()))
            .collect();
        if key == self.synced {
            return;
        }
        self.synced = key;
        let at = options
            .iter()
            .position(|o| o.id == self.channel)
            .unwrap_or(0);
        self.select.update(cx, |s, cx| {
            s.set_items(options, window, cx);
            s.set_selected_index(Some(IndexPath::new(at)), window, cx);
        });
    }

    /// `J`/`K` and the arrows move the focus row, `Enter` opens it, `Esc`
    /// lets go (K4). Only while the list itself holds the keyboard, so the
    /// search box and the channel menu keep their own keys.
    fn on_key(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if ev.keystroke.modifiers.modified() || !self.focus.is_focused(window) {
            return;
        }
        let forward = match ev.keystroke.key.as_str() {
            "j" | "down" => true,
            "k" | "up" => false,
            "enter" => {
                if let Some(Row::Item(i)) = self.cursor.and_then(|ix| self.rows.get(ix)) {
                    let key = self.items[*i].agent.key.clone();
                    nav::navigate(Route::direct(Some(key)), cx);
                }
                return;
            }
            "escape" if self.cursor.is_some() => {
                self.cursor = None;
                cx.notify();
                return;
            }
            _ => return,
        };
        let is_item = |ix: &usize| matches!(self.rows.get(*ix), Some(Row::Item(_)));
        let next = match (self.cursor, forward) {
            (None, _) => (0..self.rows.len()).find(is_item),
            (Some(at), true) => (at + 1..self.rows.len()).find(is_item),
            (Some(at), false) => (0..at).rev().find(is_item),
        };
        if let Some(ix) = next {
            self.cursor = Some(ix);
            self.list.scroll_to_reveal_item(ix);
            cx.notify();
        }
        cx.stop_propagation();
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        // The keyboard stays on the same agent when rows are added.
        let held = match self.cursor.and_then(|ix| self.rows.get(ix)) {
            Some(Row::Item(i)) => self.items.get(*i).map(|it| it.agent.key.clone()),
            _ => None,
        };
        let mut rows = Vec::new();
        if !self.deleted.is_empty() {
            rows.push(Row::Deleted);
        }
        if self.error.is_some() {
            rows.push(Row::Error);
        }
        if !self.loading && self.error.is_none() && self.items.is_empty() {
            rows.push(Row::Empty);
        }
        for (_, list) in by_day(&self.items) {
            rows.push(Row::Day(self.items[list[0]].end_ms));
            rows.extend(list.into_iter().map(Row::Item));
        }
        if self.next.is_some() {
            rows.push(Row::More);
        } else if !self.items.is_empty() {
            rows.push(Row::End);
        }
        self.cursor = held.and_then(|key| {
            rows.iter()
                .position(|r| matches!(r, Row::Item(i) if self.items[*i].agent.key == key))
        });
        self.rows = rows;
        sync_list(&self.list, self.rows.len());
        cx.notify();
    }

    fn now(&self, cx: &App) -> u64 {
        if self.now > 0 {
            self.now
        } else {
            Desk::get(cx).read(cx).now_ms()
        }
    }

    fn render_row(
        &mut self,
        ix: usize,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(row) = self.rows.get(ix).cloned() else {
            return div().into_any_element();
        };
        let theme = cx.global::<Theme>().clone();
        let now = self.now(cx);
        match row {
            Row::Deleted => deleted_rooms(&self.deleted, now, &theme, cx),
            Row::Error => kit::empty_block(
                Some(t(cx, "history.loadFailed")),
                vec![
                    self.error.clone().unwrap_or_default().into_any_element(),
                    kit::button("hist-retry", t(cx, "history.retry"), false, false, &theme)
                        .on_click(cx.listener(|this, _, _, cx| this.load(None, cx)))
                        .into_any_element(),
                ],
                &theme,
            )
            .into_any_element(),
            Row::Empty => {
                let filtered = !self.channel.is_empty() || !self.query.trim().is_empty();
                let (title, hint) = if filtered {
                    (t(cx, "history.noMatches"), t(cx, "history.noMatchesHint"))
                } else {
                    (t(cx, "history.emptyTitle"), t(cx, "history.emptyHint"))
                };
                kit::empty_block(Some(title), vec![hint.into_any_element()], &theme)
                    .into_any_element()
            }
            Row::Day(ms) => day_divider(day_title(ms, now, cx), &theme).into_any_element(),
            Row::Item(i) => self.item_row(i, self.cursor == Some(ix), &theme, cx),
            Row::More => {
                // The end of the list is near: ask for the next page.
                if !self.loading
                    && self.error.is_none()
                    && let Some(next) = self.next.clone()
                {
                    let entity = cx.entity();
                    cx.defer(move |cx| {
                        entity.update(cx, |this, cx| {
                            if !this.loading && this.next.as_deref() == Some(next.as_str()) {
                                this.load(Some(next), cx);
                            }
                        })
                    });
                }
                let label = if self.loading {
                    t(cx, "history.loading")
                } else {
                    t(cx, "history.loadMore")
                };
                let next = self.next.clone();
                let loading = self.loading;
                div()
                    .flex()
                    .justify_center()
                    .px(px(space::GUTTER))
                    .py(px(space::LG))
                    .child(
                        kit::button("hist-more", label, false, loading, &theme).when(
                            !loading,
                            |b| {
                                b.on_click(cx.listener(move |this, _, _, cx| {
                                    if !this.loading {
                                        this.load(next.clone(), cx);
                                    }
                                }))
                            },
                        ),
                    )
                    .into_any_element()
            }
            Row::End => div()
                .flex()
                .justify_center()
                .px(px(space::GUTTER))
                .pt(px(space::LG))
                .pb(px(space::BREAK))
                .child(ui::text(
                    t(cx, "history.endOfMonth"),
                    size::SMALL,
                    weight::REGULAR,
                    theme.c("ink-3"),
                ))
                .into_any_element(),
        }
    }

    /// One session: who and how it ended, where and when, and (when it
    /// left one) its last line. Every row has the same first two lines.
    fn item_row(
        &self,
        i: usize,
        focused: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let item = &self.items[i];
        let a = &item.agent;
        let view = Desk::get(cx).read(cx).view.clone();
        let live = view.as_ref().and_then(|v| v.agent(&a.key));
        let _read_only = read_only(live);
        let role = role_line(a, cx);
        let key = a.key.clone();
        let hover = theme.c("hover");
        let meta = |text: SharedString| {
            ui::text(text, size::SMALL, weight::REGULAR, theme.c("ink-3"))
                .flex_none()
                .whitespace_nowrap()
        };
        kit::row_focus(
            div().id(SharedString::from(format!("hist-{}", a.key))),
            theme,
        )
        .w_full()
        .flex()
        .gap(px(space::LG))
        .px(px(space::GUTTER))
        .py(px(space::LG))
        .cursor_pointer()
        .hover(move |s| s.bg(hover))
        .when(focused, |d| d.shadow(vec![ui::focus_ring(theme, true)]))
        .on_click(move |_, _, cx| nav::navigate(Route::direct(Some(key.clone())), cx))
        .child(ui::avatar::agent_avatar(a, avatar_size::ROW, false, cx))
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(space::HAIR))
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::MD))
                        .min_w_0()
                        .child(
                            ui::text(a.name.clone(), size::UI, weight::HEAVY, theme.c("ink"))
                                .flex_none()
                                .max_w(px(NAME_MAX))
                                .truncate(),
                        )
                        .child(kit::chip(
                            ending_text(item, cx),
                            ending_tone(ending_id(item)),
                            theme,
                        ))
                        .children(role.map(|r| {
                            ui::text(r, size::SMALL, weight::REGULAR, theme.c("ink-3"))
                                .min_w_0()
                                .truncate()
                        })),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(space::LG))
                        .min_w_0()
                        .child(meta(match &item.channel {
                            Some(ch) => SharedString::from(format!("#{ch}")),
                            None => t(cx, "history.noChannel"),
                        }))
                        .children(item.repo.as_deref().filter(|r| !r.is_empty()).map(|r| {
                            kit::mono(path_tail(&short_path(r)))
                                .text_color(theme.c("ink-3"))
                                .min_w_0()
                                .truncate()
                        }))
                        .child(meta(span_text(item, cx).into())),
                )
                .children(
                    item.last_line
                        .clone()
                        .filter(|l| !l.is_empty())
                        .map(|line| {
                            ui::text(line, size::UI, weight::REGULAR, theme.c("ink-2")).truncate()
                        }),
                ),
        )
        .into_any_element()
    }
}

/// `.hb-day`: the day's name, as the label of the rows under it.
fn day_divider(label: SharedString, theme: &Theme) -> impl IntoElement {
    div()
        .pt(px(space::XL))
        .px(px(space::GUTTER))
        .pb(px(space::XS))
        .child(ui::section_label(label, theme))
}

/// History's way to a deleted room: a labelled list of rows, no card.
fn deleted_rooms(rooms: &[DeletedRoom], now: u64, theme: &Theme, cx: &App) -> AnyElement {
    let hover = theme.c("hover");
    div()
        .pt(px(space::XL))
        .pb(px(space::MD))
        .child(
            div()
                .px(px(space::GUTTER))
                .pb(px(space::XS))
                .child(ui::section_label(t(cx, "rooms.deletedTitle"), theme))
                .child(ui::text(
                    t(cx, "rooms.deletedHint"),
                    size::SMALL,
                    weight::REGULAR,
                    theme.c("ink-3"),
                )),
        )
        .children(rooms.iter().map(|room| {
            let when = at_date(room.deleted_ms, now, cx);
            let meta = if room.messages == 1 {
                tv(cx, "rooms.deletedRowMetaOne", &[("when", &when)])
            } else {
                tv(
                    cx,
                    "rooms.deletedRowMeta",
                    &[("when", &when), ("count", &room.messages.to_string())],
                )
            };
            let name = room.name.clone();
            kit::row_focus(
                div().id(SharedString::from(format!("deleted-{}", room.name))),
                theme,
            )
            .flex()
            .items_center()
            .gap(px(space::LG))
            .h(px(row::MENU))
            .px(px(space::GUTTER))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_click(move |_, _, cx| nav::navigate(Route::DeletedRoom { name: name.clone() }, cx))
            .child(
                ui::text(
                    format!("#{}", room.name),
                    size::UI,
                    weight::STRONG,
                    theme.c("ink"),
                )
                .flex_none(),
            )
            .child(
                ui::text(meta, size::SMALL, weight::REGULAR, theme.c("ink-3"))
                    .min_w_0()
                    .truncate(),
            )
        }))
        .into_any_element()
}

impl Render for HistoryScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        // The channel list comes with each page; keep the filter's options in step.
        self.sync_select(window, cx);
        let count = if self.loading && self.items.is_empty() {
            t(cx, "history.readingMonth")
        } else {
            let base = if self.total == 1 {
                "history.countOne"
            } else {
                "history.countMany"
            };
            tv(
                cx,
                base,
                &[
                    ("count", &self.total.to_string()),
                    ("days", &self.days.to_string()),
                ],
            )
        };
        main_column(&theme)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(Self::on_key))
            .child(head(
                t(cx, "history.title"),
                Some(t(cx, "history.topic")),
                &theme,
            ))
            .child(
                div()
                    .flex_none()
                    .flex()
                    .flex_wrap()
                    .items_center()
                    .gap(px(space::MD))
                    .pt(px(space::LG))
                    .px(px(space::GUTTER))
                    .pb(px(space::XS))
                    .child(
                        div().flex_1().min_w(px(SEARCH_MIN)).child(search_frame(
                            Input::new(&self.search)
                                .appearance(false)
                                .text_size(px(size::UI)),
                            self.search_focused,
                            &theme,
                        )),
                    )
                    .child(
                        div()
                            .flex_none()
                            .h(px(control::MD))
                            .text_size(px(size::UI))
                            .child(
                                Select::new(&self.select)
                                    .id("hist-channel")
                                    .menu_width(px(MENU_W))
                                    .menu_max_h(px(MENU_MAX_H)),
                            ),
                    )
                    .child(
                        ui::text(count, size::SMALL, weight::REGULAR, theme.c("ink-3")).ml_auto(),
                    ),
            )
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
            )
    }
}
