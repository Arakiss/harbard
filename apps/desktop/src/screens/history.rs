//! History (`/history`, under "More tools"): the month the shell does not
//! keep, one page at a time from `GET /history/agents`, the next asked for
//! as the end of the list comes near. The channel filter and the search
//! (250 ms after typing stops) are serve's, so they reach the whole month.
//! Deleted rooms sit on top.

use super::secondary::{self as kit, head, main_column, search_frame, sync_list, workspace};
use crate::api::enc;
use crate::derive::history::{
    PAGE_SIZE, by_day, day_title, ending_id, ending_text, merge_page, query, read_only, role_line, span_text,
};
use crate::desk::Desk;
use crate::i18n::{I18n, t, tv};
use crate::model::history::{DeletedRoom, HistoryChannel, HistoryItem, HistoryPage};
use crate::nav::{self, Nav, Route};
use crate::theme::Theme;
use crate::ui::{
    self,
    format::{at_date, short_path},
    size,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ListAlignment,
    ListState, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window,
    div, list, px,
};
use gpui_kit::component::IndexPath;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::component::select::{Select, SelectEvent, SelectState};
use std::time::Duration;

const DEBOUNCE: Duration = Duration::from_millis(250);

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
            cx.subscribe(&search, |this, _, ev: &InputEvent, cx| {
                if matches!(ev, InputEvent::Change) {
                    this.debounce(cx);
                }
            }),
            cx.subscribe(&select, |this, _, ev: &SelectEvent<Vec<ChannelOption>>, cx| {
                let SelectEvent::Confirm(value) = ev;
                let channel = value.clone().unwrap_or_default();
                if channel != this.channel {
                    this.channel = channel;
                    this.remember(cx);
                    this.load(None, cx);
                }
            }),
        ];
        cx.observe(&Desk::get(cx), |this, _, cx| this.rebuild(cx)).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        cx.observe_global_in::<I18n>(window, |this, window, cx| {
            let placeholder = t(cx, "history.searchPlaceholder");
            this.search
                .update(cx, |s, cx| s.set_placeholder(placeholder, window, cx));
            this.sync_select(window, cx);
            this.rebuild(cx);
        })
        .detach();
        let mut this = HistoryScreen {
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
            query(before.as_deref(), &self.channel, &self.query, PAGE_SIZE, ws.as_deref())
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
        let key: Vec<(String, SharedString)> = options.iter().map(|o| (o.id.clone(), o.label.clone())).collect();
        if key == self.synced {
            return;
        }
        self.synced = key;
        let at = options.iter().position(|o| o.id == self.channel).unwrap_or(0);
        self.select.update(cx, |s, cx| {
            s.set_items(options, window, cx);
            s.set_selected_index(Some(IndexPath::new(at)), window, cx);
        });
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
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

    fn render_row(&mut self, ix: usize, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
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
                    more_button("hist-retry", t(cx, "history.retry"), false, &theme)
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
                kit::empty_block(Some(title), vec![hint.into_any_element()], &theme).into_any_element()
            }
            Row::Day(ms) => day_divider(day_title(ms, now, cx), &theme).into_any_element(),
            Row::Item(i) => self.item_row(i, &theme, cx),
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
                div()
                    .flex()
                    .justify_center()
                    .px(px(20.))
                    .py(px(14.))
                    .child(
                        more_button("hist-more", label, self.loading, &theme).on_click(cx.listener(
                            move |this, _, _, cx| {
                                if !this.loading {
                                    this.load(next.clone(), cx);
                                }
                            },
                        )),
                    )
                    .into_any_element()
            }
            Row::End => div()
                .flex()
                .justify_center()
                .px(px(20.))
                .pt(px(14.))
                .pb(px(38.))
                .text_size(px(size::SMALL))
                .text_color(theme.c("ink-3"))
                .child(t(cx, "history.endOfMonth"))
                .into_any_element(),
        }
    }

    fn item_row(&self, i: usize, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let item = &self.items[i];
        let a = &item.agent;
        let view = Desk::get(cx).read(cx).view.clone();
        let live = view.as_ref().and_then(|v| v.agent(&a.key));
        let _read_only = read_only(live);
        let role = role_line(a, cx);
        let ending = ending_id(item).to_string();
        let (pill_bg, pill_ink, pill_edge) = match ending.as_str() {
            "verified" => (theme.c("ok-bg"), theme.c("ok-ink"), theme.c("ok-line")),
            "finished" => (theme.c("green-bg"), theme.c("green-ink"), theme.c("green-line")),
            "no_closure" => (theme.c("surface-2"), theme.c("ink-3"), theme.c("line")),
            "abierto" => (theme.c("surface-2"), theme.c("ink"), theme.c("line-strong")),
            _ => (theme.c("surface-2"), theme.c("ink-2"), theme.c("line")),
        };
        let key = a.key.clone();
        div()
            .id(SharedString::from(format!("hist-{}", a.key)))
            .w_full()
            .flex()
            .gap(px(12.))
            .px(px(20.))
            .py(px(10.))
            .cursor_pointer()
            .on_click(move |_, _, cx| nav::navigate(Route::direct(Some(key.clone())), cx))
            .child(ui::avatar::agent_avatar(a, 40., false, cx))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap(px(8.))
                            .min_w_0()
                            .child(
                                div()
                                    .flex_none()
                                    .font_weight(FontWeight::BLACK)
                                    .text_size(px(15.))
                                    .child(a.name.clone()),
                            )
                            .children(role.map(|r| {
                                div()
                                    .min_w_0()
                                    .text_size(px(size::SMALL))
                                    .text_color(theme.c("ink-3"))
                                    .whitespace_nowrap()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(r)
                            }))
                            .child(
                                div()
                                    .ml_auto()
                                    .flex_none()
                                    .px(px(8.))
                                    .py(px(1.))
                                    .rounded(px(10.))
                                    .border_1()
                                    .border_color(pill_edge)
                                    .when(ending == "no_closure", |d| d.border_dashed())
                                    .bg(pill_bg)
                                    .text_color(pill_ink)
                                    .text_size(px(size::META))
                                    .font_weight(FontWeight::BOLD)
                                    .child(ending_text(item, cx)),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_wrap()
                            .gap_x(px(12.))
                            .gap_y(px(4.))
                            .text_size(px(size::SMALL))
                            .text_color(theme.c("ink-3"))
                            .child(match &item.channel {
                                Some(ch) => SharedString::from(format!("#{ch}")),
                                None => t(cx, "history.noChannel"),
                            })
                            .children(
                                item.repo
                                    .as_deref()
                                    .filter(|r| !r.is_empty())
                                    .map(|r| kit::mono(short_path(r))),
                            )
                            .child(span_text(item, cx)),
                    )
                    .children(item.last_line.clone().filter(|l| !l.is_empty()).map(|line| {
                        div()
                            .text_size(px(size::UI))
                            .text_color(theme.c("ink-2"))
                            .whitespace_nowrap()
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(line)
                    })),
            )
            .into_any_element()
    }
}

/// `.hb-hist-more`: the 34px "Load more" / "Try again" button.
pub fn more_button(id: &'static str, label: SharedString, disabled: bool, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .h(px(34.))
        .px(px(14.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface"))
        .text_size(px(size::UI))
        .font_weight(FontWeight::BOLD)
        .text_color(if disabled { theme.c("ink-4") } else { theme.c("ink") })
        .when(!disabled, |d| d.cursor_pointer())
        .child(label)
}

/// `.hb-day`: the day's name in a pill on a hairline.
fn day_divider(label: SharedString, theme: &Theme) -> impl IntoElement {
    div()
        .relative()
        .flex()
        .justify_center()
        .mt(px(8.))
        .mb(px(4.))
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .top(px(13.))
                .h(px(1.))
                .bg(theme.c("line")),
        )
        .child(
            div()
                .px(px(14.))
                .py(px(2.))
                .rounded(px(14.))
                .border_1()
                .border_color(theme.c("line"))
                .bg(theme.c("surface"))
                .text_size(px(size::SMALL))
                .font_weight(FontWeight::BOLD)
                .text_color(theme.c("ink-2"))
                .child(label),
        )
}

/// History's way to a deleted room.
fn deleted_rooms(rooms: &[DeletedRoom], now: u64, theme: &Theme, cx: &App) -> AnyElement {
    let hover = theme.c("hover-2");
    div()
        .pt(px(8.))
        .px(px(20.))
        .pb(px(16.))
        .child(
            div()
                .px(px(16.))
                .py(px(14.))
                .rounded(px(10.))
                .bg(theme.c("tint"))
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::BLACK)
                        .child(t(cx, "rooms.deletedTitle")),
                )
                .child(
                    div()
                        .mt(px(2.))
                        .mb(px(8.))
                        .text_size(px(size::SMALL))
                        .text_color(theme.c("ink-3"))
                        .child(t(cx, "rooms.deletedHint")),
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
                    div()
                        .id(SharedString::from(format!("deleted-{}", room.name)))
                        .flex()
                        .flex_wrap()
                        .items_baseline()
                        .gap_x(px(12.))
                        .gap_y(px(4.))
                        .px(px(8.))
                        .py(px(6.))
                        .rounded(px(6.))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover))
                        .on_click(move |_, _, cx| nav::navigate(Route::DeletedRoom { name: name.clone() }, cx))
                        .child(div().font_weight(FontWeight::BOLD).child(format!("#{}", room.name)))
                        .child(
                            div()
                                .text_size(px(size::SMALL))
                                .text_color(theme.c("ink-3"))
                                .child(meta),
                        )
                })),
        )
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
                &[("count", &self.total.to_string()), ("days", &self.days.to_string())],
            )
        };
        main_column(&theme)
            .child(head(
                "rotate-ccw-clock",
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
                    .gap(px(10.))
                    .pt(px(12.))
                    .px(px(20.))
                    .pb(px(10.))
                    .border_b_1()
                    .border_color(theme.c("line"))
                    .child(div().flex_1().min_w(px(320.)).child(search_frame(
                        Input::new(&self.search).appearance(false).text_size(px(size::BODY)),
                        &theme,
                    )))
                    .child(
                        div().flex_none().h(px(40.)).text_size(px(size::UI)).child(
                            Select::new(&self.select)
                                .id("hist-channel")
                                .menu_width(px(280.))
                                .menu_max_h(px(360.)),
                        ),
                    )
                    .child(
                        div()
                            .ml_auto()
                            .text_size(px(size::SMALL))
                            .text_color(theme.c("ink-3"))
                            .child(count),
                    ),
            )
            .child(
                div().flex_1().min_h_0().pt(px(8.)).child(
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
