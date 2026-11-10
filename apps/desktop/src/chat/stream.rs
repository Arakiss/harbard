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
use super::message::item_with;
use crate::derive::legible::{self, MachineMark, StreamItem};
use crate::derive::messages as words;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{Message, WorkspaceView};
use crate::theme::Theme;
use crate::ui::{self, avatar, format::hhmm};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, ElementId, FollowMode, FontWeight, InteractiveElement, IntoElement, ListAlignment, ListOffset, ListState,
    ParentElement, RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div, list, px, relative,
};
use gpui_kit::component::tooltip::Tooltip;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

/// Called when the reader scrolls near the top (rooms load older pages).
pub type NearTop = Rc<dyn Fn(&mut Window, &mut App)>;

/// The stream (the contract's `chat::stream`).
pub fn stream(messages: &[&Message], view: &WorkspaceView, opts: &MessageOpts, cx: &App) -> AnyElement {
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
    Stream { data: Rc::new(data), opts: Rc::new(opts.clone()), head, on_near_top }.into_any_element()
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
    Day { key: i64, label: SharedString },
    Message(usize),
    Run { id: String, label: SharedString, older: Vec<usize> },
    Internal { id: String, label: SharedString, messages: Vec<usize>, faces: Vec<String> },
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
    fn new(messages: &[&Message], view: &WorkspaceView, opts: &MessageOpts, head: bool, cx: &App) -> Data {
        // Share the read when the stream draws from it; copy it otherwise.
        let shared = Desk::get(cx).read(cx).view.clone().filter(|v| std::ptr::eq(Arc::as_ptr(v), view));
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
            rows.push(Row::Day { key: day.key, label: day.label(now, cx) });
            for item in legible::runs(view, &day.items, opts.fold_internal) {
                match item {
                    StreamItem::Message(m) => rows.push(Row::Message(ix(m))),
                    StreamItem::Run { id, kind, count, team, older, newest } => {
                        rows.push(Row::Run {
                            id,
                            label: legible::run_label(kind, count, team.as_deref(), cx),
                            older: older.iter().map(|m| ix(m)).collect(),
                        });
                        rows.push(Row::Message(ix(newest)));
                    }
                    StreamItem::Internal { id, messages, names, faces } => rows.push(Row::Internal {
                        id,
                        label: legible::internal_label(messages.len(), &names, cx),
                        messages: messages.iter().map(|m| ix(m)).collect(),
                        faces,
                    }),
                }
            }
        }
        Data { marks: legible::machine_marks(messages), view: arc, own, order, rows }
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
        self.order.len().checked_sub(1).map(|i| self.message(i).id.as_str())
    }

    /// One message row with the stream's marker.
    fn item(&self, i: usize, opts: &MessageOpts, cx: &App) -> AnyElement {
        let m = self.message(i);
        item_with(m, &self.view, opts, Some(self.marks.get(&m.id).cloned()), cx)
    }
}

/// The list and what it showed last, kept across frames.
struct StreamState {
    list: ListState,
    keys: Vec<String>,
    first: Option<String>,
    last: Option<String>,
}

impl StreamState {
    /// Bring the list in line with the new rows: splice what changed so the
    /// rows on screen keep their place, and go to the bottom when the last
    /// message changed.
    fn sync(&mut self, keys: Vec<String>, first: Option<String>, last: Option<String>) {
        if keys != self.keys {
            let old = &self.keys;
            let top = self.list.logical_scroll_top();
            let prefix = old.iter().zip(&keys).take_while(|(a, b)| a == b).count();
            let room = old.len().min(keys.len()) - prefix;
            let suffix = old.iter().rev().zip(keys.iter().rev()).take(room).take_while(|(a, b)| a == b).count();
            self.list.splice(prefix..old.len() - suffix, keys.len() - prefix - suffix);
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
                    self.list.scroll_to(ListOffset { item_ix: ix, offset_in_item: offset });
                }
            }
            self.keys = keys;
        }
        if last != self.last {
            self.list.scroll_to_end();
        }
        self.first = first;
        self.last = last;
    }
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
        let state = window.use_keyed_state(ElementId::Name(format!("hb-stream-state-{key}").into()), cx, |_, _| StreamState {
            list: {
                let list = ListState::new(0, ListAlignment::Top, px(800.));
                list.set_follow_mode(FollowMode::Tail);
                list
            },
            keys: Vec::new(),
            first: None,
            last: None,
        });
        let data = self.data;
        let keys: Vec<String> = data.rows.iter().map(|r| r.key(&data)).collect();
        let (first, last) = (data.first_id().map(String::from), data.last_id().map(String::from));
        let list_state = state.update(cx, |s, _| {
            s.sync(keys, first, last);
            s.list.clone()
        });
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
        div()
            .id(ElementId::Name(format!("hb-stream-{key}").into()))
            .flex_1()
            .min_h_0()
            .size_full()
            .bg(surface)
            .child(
                list(list_state, move |ix, window, cx| render_row(&data, ix, &opts, &head, window, cx))
                    .size_full()
                    .pt(px(8.))
                    .pb(px(12.)),
            )
    }
}

fn render_row(
    data: &Rc<Data>,
    ix: usize,
    opts: &Rc<MessageOpts>,
    head: &Rc<RefCell<Option<AnyElement>>>,
    _window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let Some(row) = data.rows.get(ix) else { return div().into_any_element() };
    let theme = cx.global::<Theme>();
    match row {
        Row::Head => div().w_full().children(head.borrow_mut().take()).into_any_element(),
        Row::Empty => div()
            .w_full()
            .px(px(28.))
            .py(px(40.))
            .flex()
            .justify_center()
            .text_color(theme.c("ink-3"))
            .text_size(px(ui::size::BODY))
            .children(opts.empty.clone())
            .into_any_element(),
        Row::Day { label, .. } => day_pill(label.clone(), theme),
        Row::Message(i) => div().w_full().child(data.item(*i, opts, cx)).into_any_element(),
        Row::Run { id, label, older } => div().w_full().flex().child(RunGroup {
            data: data.clone(),
            opts: opts.clone(),
            id: id.clone(),
            label: label.clone(),
            older: older.clone(),
        })
        .into_any_element(),
        Row::Internal { id, label, messages, faces } => InternalGroup {
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

/// `.hb-day`: the day's name in a pill centred on a rule.
fn day_pill(label: SharedString, theme: &Theme) -> AnyElement {
    div()
        .relative()
        .w_full()
        .flex()
        .justify_center()
        .mt(px(8.))
        .mb(px(4.))
        .child(div().absolute().left_0().right_0().top(relative(0.5)).h(px(1.)).bg(theme.c("line")))
        .child(
            div()
                .px(px(14.))
                .py(px(2.))
                .rounded(px(14.))
                .border_1()
                .border_color(theme.c("line"))
                .bg(theme.c("surface"))
                .text_size(px(13.))
                .font_weight(FontWeight::BOLD)
                .text_color(theme.c("ink-2"))
                .child(label),
        )
        .into_any_element()
}

/// `.hb-fold`: "Show all (n)", "Show less".
fn fold_button(id: &'static str, label: SharedString, theme: &Theme) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .cursor_pointer()
        .font_weight(FontWeight::BOLD)
        .text_color(theme.c("link"))
        .hover(|s| s.underline())
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
        let state = window.use_keyed_state(ElementId::Name(format!("hb-run-{}", self.id).into()), cx, |_, _| RunState::default());
        let (all, open) = {
            let s = state.read(cx);
            (s.all, s.open.clone())
        };
        let cx: &App = cx;
        let theme = cx.global::<Theme>();
        let data = &self.data;
        let hidden = if all { 0 } else { self.older.len().saturating_sub(legible::RUN_SHOWN) };
        let toggle = (hidden > 0 || all).then(|| {
            let label = if all {
                t(cx, "messages.seeLess")
            } else {
                tv(cx, "messages.seeAllCount", &[("n", &(self.older.len() + 1).to_string())])
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
                Some(a) => avatar::agent_avatar(a, 20., false, cx),
                None => avatar::name_avatar(&name, 20., cx),
            };
            let line = match m.readable.as_ref().filter(|r| !r.is_empty()) {
                Some(parts) => legible::readable_text(parts, cx),
                None => m.text.clone(),
            };
            let line = line.lines().next().unwrap_or_default().to_string();
            let state = state.clone();
            let mid = m.id.clone();
            let hint = t(cx, "messages.seeWhole");
            div()
                .id(ElementId::Name(format!("hb-run-row-{mid}").into()))
                .flex()
                .items_center()
                .gap(px(8.))
                .w_full()
                .min_w_0()
                .px(px(12.))
                .py(px(3.))
                .text_size(px(14.))
                .text_color(theme.c("ink"))
                .cursor_pointer()
                .hover(|s| s.bg(theme.c("hover")))
                .tooltip(move |window, cx| Tooltip::new(hint.clone()).build(window, cx))
                .on_click(move |_, _, cx| {
                    state.update(cx, |s, cx| {
                        s.open.insert(mid.clone());
                        cx.notify();
                    })
                })
                .child(face)
                .child(div().flex_none().font_weight(FontWeight::BOLD).child(name))
                .child(div().flex_none().text_size(px(12.)).text_color(theme.c("ink-3")).child(hhmm(m.ts_ms)))
                .child(div().min_w_0().flex_1().truncate().child(line))
                .into_any_element()
        });
        let rows: Vec<AnyElement> = rows.collect();
        div()
            .flex_1()
            .mt(px(6.))
            .mb(px(2.))
            .ml(px(if self.opts.dm { 80. } else { 68. }))
            .mr(px(20.))
            .pt(px(6.))
            .pb(px(4.))
            .border_l_2()
            .border_color(theme.c("line"))
            .font_family(ui::SANS)
            .line_height(relative(1.46))
            .child(
                div()
                    .flex()
                    .items_baseline()
                    .gap(px(10.))
                    .px(px(12.))
                    .pb(px(4.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::BOLD)
                    .text_color(theme.c("ink-3"))
                    .child(self.label.clone())
                    .children(toggle),
            )
            .child(div().flex().flex_col().children(rows))
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
        let state = window.use_keyed_state(ElementId::Name(format!("hb-internal-{}", self.id).into()), cx, |_, _| false);
        let opened = *state.read(cx);
        let cx: &App = cx;
        let theme = cx.global::<Theme>();
        let data = &self.data;
        let selected = self.opts.selected.as_deref();
        let shown = opened || self.messages.iter().any(|&i| Some(data.message(i).id.as_str()) == selected);
        let faces = self.faces.iter().map(|key| match data.view.agent(key) {
            Some(a) => avatar::agent_avatar(a, 20., false, cx),
            None => avatar::name_avatar("", 20., cx),
        });
        let ink = theme.c("ink-3");
        let head = div()
            .id(ElementId::Name(format!("hb-internal-head-{}", self.id).into()))
            .flex()
            .items_center()
            .gap(px(8.))
            .w_full()
            .px(px(if self.opts.dm { 28. } else { 20. }))
            .py(px(4.))
            .text_size(px(13.))
            .text_color(ink)
            .cursor_pointer()
            .hover(|s| s.bg(theme.c("hover")))
            .on_click({
                let state = state.clone();
                move |_, _, cx| {
                    state.update(cx, |open, cx| {
                        *open = !shown;
                        cx.notify();
                    })
                }
            })
            .child(ui::icon(if shown { "chevron-down" } else { "chevron-right" }, 14., ink))
            .child(div().flex().gap(px(2.)).children(faces))
            .child(self.label.clone());
        div()
            .w_full()
            .flex()
            .flex_col()
            .font_family(ui::SANS)
            .line_height(relative(1.46))
            .child(head)
            .when(shown, |d| d.children(self.messages.iter().map(|&i| data.item(i, &self.opts, cx))))
    }
}
