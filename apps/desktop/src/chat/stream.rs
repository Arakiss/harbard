//! A scrolling, day-grouped list of messages (`Stream`, `RunGroup`,
//! `InternalGroup`, `parts.tsx`).
//!
//! The stream is virtualized (`gpui::list`): each day pill, message, run
//! and internal group is one row. It opens at the bottom and goes back to
//! the bottom whenever the last message changes, as the console did.
//! Rooms add a head row (the "older messages" control) and a callback near
//! the top that loads older pages; the rows on screen stay in place when
//! older ones are prepended.

use super::MessageOpts;
use super::focusable;
use super::message::item_with;
use crate::derive::legible::{self, MachineMark, StreamItem};
use crate::derive::messages as words;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{Message, WorkspaceView};
use crate::theme::Theme;
use crate::ui::{
    self, avatar, avatar_size, control, format::hhmm, icon_size, leading, motion,
    pill_chat as pill, radius, row, size, space, weight,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Animation, AnimationExt as _, AnyElement, App, ElementId, FollowMode, InteractiveElement,
    IntoElement, ListAlignment, ListOffset, ListState, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div, list, px,
};
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Called when the reader scrolls near the top (rooms load older pages).
pub type NearTop = Rc<dyn Fn(&mut Window, &mut App)>;

/// The stream (the contract's `chat::stream`).
pub fn stream(
    messages: &[&Message],
    view: &WorkspaceView,
    opts: &MessageOpts,
    cx: &App,
) -> AnyElement {
    stream_with(messages, view, opts, None, None, cx)
}

/// A stream with a `head` above the first message (rooms: "Load older
/// messages" or the start of the room) and `on_near_top`, called when the
/// reader is near the top. It may be called again while a page loads: the
/// caller ignores calls it cannot serve (no next page, already loading).
pub fn stream_with(
    messages: &[&Message],
    view: &WorkspaceView,
    opts: &MessageOpts,
    head: Option<AnyElement>,
    on_near_top: Option<NearTop>,
    cx: &App,
) -> AnyElement {
    let data = Data::new(messages, view, opts, head.is_some(), cx);
    Stream {
        data: Rc::new(data),
        opts: Rc::new(opts.clone()),
        head,
        on_near_top,
    }
    .into_any_element()
}

/// Where a stream message lives: in the read, or a copy (older room pages).
#[derive(Clone, Copy)]
enum At {
    View(usize),
    Own(usize),
}

/// One row of the list.
enum Row {
    Head,
    Empty,
    Day {
        key: i64,
        label: SharedString,
    },
    Message(usize),
    Run {
        id: String,
        label: SharedString,
        older: Vec<usize>,
    },
    Internal {
        id: String,
        label: SharedString,
        messages: Vec<usize>,
        faces: Vec<String>,
    },
}

impl Row {
    /// What a row is, across reads: the scroll position follows it.
    fn key(&self, data: &Data) -> String {
        match self {
            Row::Head => "head".into(),
            Row::Empty => "empty".into(),
            Row::Day { key, .. } => format!("day:{key}"),
            Row::Message(i) => data.message(*i).id.clone(),
            Row::Run { id, .. } | Row::Internal { id, .. } => id.clone(),
        }
    }
}

/// The stream's messages and rows, computed once per render.
struct Data {
    view: Arc<WorkspaceView>,
    own: Vec<Message>,
    order: Vec<At>,
    rows: Vec<Row>,
    marks: HashMap<String, MachineMark>,
}

impl Data {
    fn new(
        messages: &[&Message],
        view: &WorkspaceView,
        opts: &MessageOpts,
        head: bool,
        cx: &App,
    ) -> Data {
        // Share the read when the stream draws from it; copy it otherwise.
        let shared = Desk::get(cx)
            .read(cx)
            .view
            .clone()
            .filter(|v| std::ptr::eq(Arc::as_ptr(v), view));
        let arc = shared.unwrap_or_else(|| Arc::new(view.clone()));
        let base = view.messages.as_ptr_range();
        let mut own = Vec::new();
        let mut order = Vec::with_capacity(messages.len());
        let mut index: HashMap<*const Message, usize> = HashMap::with_capacity(messages.len());
        for (i, &m) in messages.iter().enumerate() {
            let ptr = m as *const Message;
            let at = if base.contains(&ptr) {
                // SAFETY: `ptr` lies inside `view.messages`, so both share one allocation.
                At::View(unsafe { ptr.offset_from(base.start) } as usize)
            } else {
                own.push(m.clone());
                At::Own(own.len() - 1)
            };
            order.push(at);
            index.insert(ptr, i);
        }
        let ix = |m: &Message| index[&(m as *const Message)];
        let now = Desk::get(cx).read(cx).now_ms();
        let mut rows = Vec::new();
        if head {
            rows.push(Row::Head);
        }
        if messages.is_empty() && opts.empty.is_some() {
            rows.push(Row::Empty);
        }
        for day in words::by_day(messages) {
            rows.push(Row::Day {
                key: day.key,
                label: day.label(now, cx),
            });
            for item in legible::runs(view, &day.items, opts.fold_internal) {
                match item {
                    StreamItem::Message(m) => rows.push(Row::Message(ix(m))),
                    StreamItem::Run {
                        id,
                        kind,
                        count,
                        team,
                        older,
                        newest,
                    } => {
                        rows.push(Row::Run {
                            id,
                            label: legible::run_label(kind, count, team.as_deref(), cx),
                            older: older.iter().map(|m| ix(m)).collect(),
                        });
                        rows.push(Row::Message(ix(newest)));
                    }
                    StreamItem::Internal {
                        id,
                        messages,
                        names,
                        faces,
                    } => rows.push(Row::Internal {
                        id,
                        label: legible::internal_label(messages.len(), &names, cx),
                        messages: messages.iter().map(|m| ix(m)).collect(),
                        faces,
                    }),
                }
            }
        }
        Data {
            marks: legible::machine_marks(messages),
            view: arc,
            own,
            order,
            rows,
        }
    }

    fn message(&self, i: usize) -> &Message {
        match self.order[i] {
            At::View(j) => &self.view.messages[j],
            At::Own(j) => &self.own[j],
        }
    }

    fn first_id(&self) -> Option<&str> {
        (!self.order.is_empty()).then(|| self.message(0).id.as_str())
    }

    fn last_id(&self) -> Option<&str> {
        self.order
            .len()
            .checked_sub(1)
            .map(|i| self.message(i).id.as_str())
    }

    /// One message row with the stream's marker.
    fn item(&self, i: usize, opts: &MessageOpts, cx: &App) -> AnyElement {
        let m = self.message(i);
        item_with(
            m,
            &self.view,
            opts,
            Some(self.marks.get(&m.id).cloned()),
            cx,
        )
    }
}

/// The messages that came in after the old newest one, in order; `None`
/// when the old newest is gone (another conversation, a first read), which
/// is a fresh start rather than an arrival. Older pages prepended above do
/// not count: they sit before it.
pub(crate) fn fresh_after(ids: &[&str], old_last: Option<&str>) -> Option<Vec<String>> {
    let old = old_last?;
    let at = ids.iter().rposition(|id| *id == old)?;
    Some(ids[at + 1..].iter().map(|id| id.to_string()).collect())
}

/// What arrived while the reader was scrolled up (§3: "a 'new messages'
/// line appears instead of a jump"): the line sits above `from`, and the
/// pill at the foot counts what has not been reached yet.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Unseen {
    pub from: Option<String>,
    pub count: usize,
}

impl Unseen {
    /// Messages came in at the bottom; `jump` when the stream follows them
    /// (the reader was at the newest, or wrote the message).
    pub fn arrive(&mut self, fresh: &[String], jump: bool) {
        let Some(first) = fresh.first() else {
            return;
        };
        if jump {
            *self = Unseen::default();
            return;
        }
        if self.from.is_none() {
            self.from = Some(first.clone());
        }
        self.count += fresh.len();
    }

    /// The reader reached the newest message: the pill goes, the line
    /// stays where the new ones started.
    pub fn reached_end(&mut self) {
        self.count = 0;
    }
}

/// How long a row that just arrived keeps its wash (`motion::ARRIVAL`).
const ARRIVAL: Duration = motion::ARRIVAL;

/// The list and what it showed last, kept across frames.
struct StreamState {
    list: ListState,
    keys: Vec<String>,
    first: Option<String>,
    last: Option<String>,
    /// Rows that arrived while the stream was on screen, and when.
    arrived: HashMap<String, Instant>,
    unseen: Unseen,
}

impl StreamState {
    /// Bring the list in line with the new rows: splice what changed so the
    /// rows on screen keep their place. New messages at the bottom take the
    /// reader there when they were already at the newest one (or wrote it);
    /// otherwise they are counted and marked with a line, and nothing moves.
    fn sync(
        &mut self,
        keys: Vec<String>,
        ids: &[&str],
        own_last: bool,
        first: Option<String>,
        last: Option<String>,
    ) {
        let at_end = self.list.is_scrolled_to_end().unwrap_or(true);
        if keys != self.keys {
            let old = &self.keys;
            let top = self.list.logical_scroll_top();
            let prefix = old.iter().zip(&keys).take_while(|(a, b)| a == b).count();
            let room = old.len().min(keys.len()) - prefix;
            let suffix = old
                .iter()
                .rev()
                .zip(keys.iter().rev())
                .take(room)
                .take_while(|(a, b)| a == b)
                .count();
            self.list
                .splice(prefix..old.len() - suffix, keys.len() - prefix - suffix);
            // Older pages came in above: keep the first row the reader saw.
            let prepended = !old.is_empty() && first != self.first && last == self.last;
            if prepended {
                let anchor = match old.get(top.item_ix).map(String::as_str) {
                    Some("head") => old.get(top.item_ix + 1).map(|k| (k, px(0.))),
                    Some(_) => old.get(top.item_ix).map(|k| (k, top.offset_in_item)),
                    None => None,
                };
                if let Some((key, offset)) = anchor
                    && let Some(ix) = keys.iter().position(|k| k == key)
                {
                    self.list.scroll_to(ListOffset {
                        item_ix: ix,
                        offset_in_item: offset,
                    });
                }
            }
            self.keys = keys;
        }
        if last != self.last {
            match fresh_after(ids, self.last.as_deref()) {
                Some(fresh) => {
                    let now = Instant::now();
                    self.arrived
                        .retain(|_, at| now.duration_since(*at) < ARRIVAL * 4);
                    for id in &fresh {
                        self.arrived.insert(id.clone(), now);
                    }
                    let jump = at_end || own_last;
                    self.unseen.arrive(&fresh, jump);
                    if jump {
                        self.list.scroll_to_end();
                    }
                }
                None => {
                    self.arrived.clear();
                    self.unseen = Unseen::default();
                    self.list.scroll_to_end();
                }
            }
        } else if self.unseen.count > 0 && at_end {
            self.unseen.reached_end();
        }
        self.first = first;
        self.last = last;
    }

    /// The marks rows read while drawing: who just arrived, where the line
    /// goes.
    fn marks(&self) -> Marks {
        let now = Instant::now();
        Marks {
            arrived: self
                .arrived
                .iter()
                .filter(|(_, at)| now.duration_since(**at) < ARRIVAL)
                .map(|(id, _)| id.clone())
                .collect(),
            line_before: self.unseen.from.clone(),
        }
    }
}

/// What the rows of one frame need from the stream's state.
#[derive(Default)]
struct Marks {
    arrived: HashSet<String>,
    line_before: Option<String>,
}

#[derive(IntoElement)]
struct Stream {
    data: Rc<Data>,
    opts: Rc<MessageOpts>,
    head: Option<AnyElement>,
    on_near_top: Option<NearTop>,
}

impl RenderOnce for Stream {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let key = self.opts.key.clone().unwrap_or_else(|| "hb-stream".into());
        let state = window.use_keyed_state(
            ElementId::Name(format!("hb-stream-state-{key}").into()),
            cx,
            |_, _| StreamState {
                list: {
                    let list = ListState::new(0, ListAlignment::Top, px(800.));
                    list.set_follow_mode(FollowMode::Tail);
                    list
                },
                keys: Vec::new(),
                first: None,
                last: None,
                arrived: HashMap::new(),
                unseen: Unseen::default(),
            },
        );
        let data = self.data;
        let keys: Vec<String> = data.rows.iter().map(|r| r.key(&data)).collect();
        let ids: Vec<&str> = (0..data.order.len())
            .map(|i| data.message(i).id.as_str())
            .collect();
        let own_last = data
            .order
            .len()
            .checked_sub(1)
            .is_some_and(|i| data.message(i).from == "operator");
        let (first, last) = (
            data.first_id().map(String::from),
            data.last_id().map(String::from),
        );
        let (list_state, marks, unseen) = state.update(cx, |s, _| {
            s.sync(keys, &ids, own_last, first, last);
            (s.list.clone(), Rc::new(s.marks()), s.unseen.count)
        });
        drop(ids);
        if let Some(near_top) = self.on_near_top.clone() {
            // The head row and the first rows: within about 300 px of the top.
            let on_scroll = near_top.clone();
            list_state.set_scroll_handler(move |event, window, cx| {
                if event.visible_range.start <= 3 {
                    on_scroll(window, cx);
                }
            });
            // A room shorter than the window never scrolls: ask at once.
            if list_state.max_offset_for_scrollbar().y <= px(0.) && !data.order.is_empty() {
                window.defer(cx, move |window, cx| near_top(window, cx));
            }
        }
        let theme = cx.global::<Theme>();
        let surface = theme.c("surface");
        let head = Rc::new(RefCell::new(self.head));
        let opts = self.opts;
        let below = (unseen > 0).then(|| new_below(unseen, list_state.clone(), state.clone(), cx));
        div()
            .id(ElementId::Name(format!("hb-stream-{key}").into()))
            .relative()
            .flex_1()
            .min_h_0()
            .size_full()
            .bg(surface)
            .child(
                list(list_state, move |ix, window, cx| {
                    render_row(&data, ix, &opts, &head, &marks, window, cx)
                })
                .size_full()
                .pt(px(space::MD))
                .pb(px(space::LG)),
            )
            .children(below)
    }
}

fn render_row(
    data: &Rc<Data>,
    ix: usize,
    opts: &Rc<MessageOpts>,
    head: &Rc<RefCell<Option<AnyElement>>>,
    marks: &Rc<Marks>,
    _window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let Some(row) = data.rows.get(ix) else {
        return div().into_any_element();
    };
    let theme = cx.global::<Theme>();
    match row {
        Row::Head => div()
            .w_full()
            .children(head.borrow_mut().take())
            .into_any_element(),
        // F3: one sentence of 14/400 in `ink-3`, `BREAK` below the header.
        Row::Empty => div()
            .w_full()
            .px(px(space::GUTTER))
            .pt(px(space::BREAK))
            .flex()
            .justify_center()
            .children(opts.empty.clone().map(|line| {
                ui::text(line, size::UI, weight::REGULAR, theme.c("ink-3")).text_center()
            }))
            .into_any_element(),
        Row::Day { label, .. } => pill::day_pill(label.clone(), theme),
        Row::Message(i) => {
            let id = data.message(*i).id.clone();
            let line = (marks.line_before.as_deref() == Some(id.as_str()))
                .then(|| new_line(t(cx, "messages.newLine"), theme));
            let wash = theme.c("selected");
            let arrived = marks.arrived.contains(&id) && !crate::robot::clock::reduced(cx);
            let item = data.item(*i, opts, cx);
            let row = div().w_full().children(line);
            if arrived {
                // M1: the wash behind a row that just arrived fades out in
                // `ARRIVAL`, linear; reduced motion shows the row at rest.
                row.child(div().w_full().child(item).with_animation(
                    ElementId::Name(format!("hb-arrive-{id}").into()),
                    Animation::new(ARRIVAL),
                    move |d, t| d.bg(wash.opacity(1. - t)),
                ))
                .into_any_element()
            } else {
                row.child(item).into_any_element()
            }
        }
        Row::Run { id, label, older } => div()
            .w_full()
            .flex()
            .child(RunGroup {
                data: data.clone(),
                opts: opts.clone(),
                id: id.clone(),
                label: label.clone(),
                older: older.clone(),
            })
            .into_any_element(),
        Row::Internal {
            id,
            label,
            messages,
            faces,
        } => InternalGroup {
            data: data.clone(),
            opts: opts.clone(),
            id: id.clone(),
            label: label.clone(),
            messages: messages.clone(),
            faces: faces.clone(),
        }
        .into_any_element(),
    }
}

/// The "new messages" line: a hairline in `red` with its label at the
/// trailing edge, above the first message that came in while the reader
/// was scrolled up.
fn new_line(label: SharedString, theme: &Theme) -> AnyElement {
    let ink = theme.c("red-ink");
    div()
        .w_full()
        .flex()
        .items_center()
        .gap(px(space::MD))
        .px(px(space::GUTTER))
        .pt(px(space::XS))
        .child(div().flex_1().h(px(1.)).bg(theme.c("red")))
        .child(ui::text(label, size::META, weight::STRONG, ink).flex_none())
        .into_any_element()
}

/// The pill at the foot of a stream scrolled up while messages came in:
/// how many, and a click takes the reader to the newest.
fn new_below(
    count: usize,
    list: ListState,
    state: gpui::Entity<StreamState>,
    cx: &App,
) -> AnyElement {
    let theme = cx.global::<Theme>();
    let label = if count == 1 {
        t(cx, "messages.newBelowOne")
    } else {
        tv(cx, "messages.newBelowMany", &[("n", &count.to_string())])
    };
    let ink = theme.c("accent-ink");
    div()
        .absolute()
        .bottom(px(space::LG))
        .left_0()
        .right_0()
        .flex()
        .justify_center()
        .child(
            focusable(
                div()
                    .id("hb-stream-new-below")
                    .flex()
                    .items_center()
                    .gap(px(space::SM))
                    .h(px(control::SM))
                    .px(px(space::LG))
                    .rounded_full()
                    .bg(theme.c("accent-deep"))
                    .shadow(theme.pop_shadow())
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.c("accent-hover")))
                    .tooltip(ui::kit_theme::tip(t(cx, "messages.newBelowHint"))),
                false,
                theme,
            )
            .on_click(move |_, _, cx| {
                list.scroll_to_end();
                state.update(cx, |s, cx| {
                    s.unseen.reached_end();
                    cx.notify();
                });
            })
            .child(ui::icon("arrow-down", icon_size::INLINE, ink))
            .child(ui::text(label, size::SMALL, weight::STRONG, ink)),
        )
        .into_any_element()
}

/// "Show all (n)", "Show less": a quiet link.
fn fold_button(id: &'static str, label: SharedString, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    focusable(
        div()
            .id(id)
            .rounded(px(radius::SM))
            .cursor_pointer()
            .font_weight(weight::REGULAR)
            .text_color(theme.c("link"))
            .hover(|s| s.underline()),
        false,
        theme,
    )
    .child(label)
}

#[derive(Default)]
struct RunState {
    all: bool,
    open: HashSet<String>,
}

/// "5 deliveries from acme": the older messages of a run, one line each,
/// the last few shown and "Show all" for the rest. A line opens its
/// message whole, in place; the newest is the next row.
#[derive(IntoElement)]
struct RunGroup {
    data: Rc<Data>,
    opts: Rc<MessageOpts>,
    id: String,
    label: SharedString,
    older: Vec<usize>,
}

impl RenderOnce for RunGroup {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(
            ElementId::Name(format!("hb-run-{}", self.id).into()),
            cx,
            |_, _| RunState::default(),
        );
        let (all, open) = {
            let s = state.read(cx);
            (s.all, s.open.clone())
        };
        let cx: &App = cx;
        let theme = cx.global::<Theme>();
        let data = &self.data;
        let hidden = if all {
            0
        } else {
            self.older.len().saturating_sub(legible::RUN_SHOWN)
        };
        let toggle = (hidden > 0 || all).then(|| {
            let label = if all {
                t(cx, "messages.seeLess")
            } else {
                tv(
                    cx,
                    "messages.seeAllCount",
                    &[("n", &(self.older.len() + 1).to_string())],
                )
            };
            let state = state.clone();
            fold_button("hb-run-all", label, theme).on_click(move |_, _, cx| {
                state.update(cx, |s, cx| {
                    s.all = !s.all;
                    cx.notify();
                })
            })
        });
        let selected = self.opts.selected.as_deref();
        let rows = self.older[hidden..].iter().map(|&i| {
            let m = data.message(i);
            if open.contains(&m.id) || selected == Some(m.id.as_str()) {
                return data.item(i, &self.opts, cx);
            }
            let agent = data.view.agent(&m.agent);
            let name = match agent {
                Some(a) => crate::derive::agent_name(a, cx),
                None => crate::derive::shown_name(&m.author, cx),
            };
            let face = match agent {
                Some(a) => avatar::agent_avatar(a, avatar_size::STACK, false, cx),
                None => avatar::name_avatar(&name, avatar_size::STACK, cx),
            };
            let line = match m.readable.as_ref().filter(|r| !r.is_empty()) {
                Some(parts) => legible::readable_text(parts, cx),
                None => m.text.clone(),
            };
            let line = line.lines().next().unwrap_or_default().to_string();
            let state = state.clone();
            let mid = m.id.clone();
            let hint = t(cx, "messages.seeWhole");
            focusable(
                div()
                    .id(ElementId::Name(format!("hb-run-row-{mid}").into()))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(px(space::MD))
                    .w_full()
                    .min_w_0()
                    .h(px(row::MENU))
                    .px(px(space::LG))
                    .rounded(px(radius::BASE))
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.c("hover")))
                    .tooltip(ui::kit_theme::tip(hint.clone())),
                true,
                theme,
            )
            .on_click(move |_, _, cx| {
                state.update(cx, |s, cx| {
                    s.open.insert(mid.clone());
                    cx.notify();
                })
            })
            .child(face)
            .child(ui::text(name, size::UI, weight::STRONG, theme.c("ink")).flex_none())
            .child(
                ui::text(hhmm(m.ts_ms), size::META, weight::REGULAR, theme.c("ink-3")).flex_none(),
            )
            .child(
                ui::text(line, size::UI, weight::REGULAR, theme.c("ink-2"))
                    .min_w_0()
                    .flex_1()
                    .truncate(),
            )
            .into_any_element()
        });
        let rows: Vec<AnyElement> = rows.collect();
        // Indented to the messages' text column, behind one hairline.
        div()
            .flex_1()
            .min_w_0()
            .mt(px(space::SM))
            .mb(px(space::HAIR))
            .ml(px(space::GUTTER + avatar_size::ROW + space::LG))
            .mr(px(space::GUTTER))
            .border_l_1()
            .border_color(theme.c("line"))
            .font_family(ui::SANS)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::LG))
                    .h(px(row::MENU))
                    .px(px(space::LG))
                    .text_size(px(size::META))
                    .line_height(px(leading::META))
                    .child(ui::section_label(self.label.clone(), theme))
                    .children(toggle),
            )
            .child(div().flex().flex_col().gap(px(space::HAIR)).children(rows))
    }
}

/// Agents' work chatter, folded: nothing in it was written for the operator.
#[derive(IntoElement)]
struct InternalGroup {
    data: Rc<Data>,
    opts: Rc<MessageOpts>,
    id: String,
    label: SharedString,
    messages: Vec<usize>,
    faces: Vec<String>,
}

impl RenderOnce for InternalGroup {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(
            ElementId::Name(format!("hb-internal-{}", self.id).into()),
            cx,
            |_, _| false,
        );
        let opened = *state.read(cx);
        let cx: &App = cx;
        let theme = cx.global::<Theme>();
        let data = &self.data;
        let selected = self.opts.selected.as_deref();
        let shown = opened
            || self
                .messages
                .iter()
                .any(|&i| Some(data.message(i).id.as_str()) == selected);
        let faces = self.faces.iter().map(|key| match data.view.agent(key) {
            Some(a) => avatar::agent_avatar(a, avatar_size::STACK, false, cx),
            None => avatar::name_avatar("", avatar_size::STACK, cx),
        });
        let ink = theme.c("ink-3");
        let head = focusable(
            div()
                .id(ElementId::Name(
                    format!("hb-internal-head-{}", self.id).into(),
                ))
                .flex()
                .items_center()
                .gap(px(space::MD))
                .w_full()
                .h(px(row::MENU))
                .px(px(space::GUTTER))
                .text_size(px(size::SMALL))
                .line_height(px(leading::SMALL))
                .text_color(ink)
                .cursor_pointer()
                .hover(|s| s.bg(theme.c("hover"))),
            true,
            theme,
        )
        .on_click({
            let state = state.clone();
            move |_, _, cx| {
                state.update(cx, |open, cx| {
                    *open = !shown;
                    cx.notify();
                })
            }
        })
        .child(ui::icon(
            if shown {
                "chevron-down"
            } else {
                "chevron-right"
            },
            icon_size::INLINE,
            ink,
        ))
        .child(
            div()
                .flex()
                .flex_none()
                .gap(px(space::HAIR))
                .children(faces),
        )
        .child(div().min_w_0().truncate().child(self.label.clone()));
        div()
            .w_full()
            .flex()
            .flex_col()
            .font_family(ui::SANS)
            .child(head)
            .when(shown, |d| {
                d.children(self.messages.iter().map(|&i| data.item(i, &self.opts, cx)))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn messages_after_the_old_newest_are_fresh() {
        let now = ["a", "b", "c", "d"];
        assert_eq!(fresh_after(&now, Some("b")), Some(ids(&["c", "d"])));
        assert_eq!(fresh_after(&now, Some("d")), Some(vec![]));
    }

    #[test]
    fn older_pages_above_are_not_arrivals() {
        let now = ["x", "y", "a", "b"];
        assert_eq!(fresh_after(&now, Some("b")), Some(vec![]));
    }

    #[test]
    fn another_conversation_is_a_fresh_start() {
        assert_eq!(fresh_after(&["p", "q"], Some("b")), None);
        assert_eq!(fresh_after(&["p", "q"], None), None);
    }

    #[test]
    fn scrolled_up_counts_and_keeps_the_first_new_one() {
        let mut u = Unseen::default();
        u.arrive(&ids(&["c"]), false);
        u.arrive(&ids(&["d", "e"]), false);
        assert_eq!(u.from.as_deref(), Some("c"));
        assert_eq!(u.count, 3);
        u.reached_end();
        assert_eq!(u.count, 0);
        assert_eq!(u.from.as_deref(), Some("c"));
    }

    #[test]
    fn following_the_bottom_clears_the_line() {
        let mut u = Unseen {
            from: Some("c".into()),
            count: 0,
        };
        u.arrive(&ids(&["f"]), true);
        assert_eq!(u, Unseen::default());
        u.arrive(&[], false);
        assert_eq!(u, Unseen::default());
    }
}
