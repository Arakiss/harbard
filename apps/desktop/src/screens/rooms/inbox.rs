//! `#feedback` as an inbox (`FeedbackInbox`, `InboxRow`): newest first,
//! each report with the kind its author gave it, filters by kind, older
//! pages at the bottom.

use super::feed::{RoomHistory, near_edge, older_control};
use crate::derive::messages::mention_segments;
use crate::derive::rooms::{
    FEEDBACK, FeedbackKind, FeedbackNote, feedback_kind_label, feedback_tone, merge_room_messages,
    parse_feedback, room_stream,
};
use crate::derive::shown_name;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::{Message, WorkspaceView};
use crate::theme::Theme;
use crate::ui::{self, avatar, format::at_date};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, FontWeight, HighlightStyle,
    InteractiveElement as _, IntoElement, ListAlignment, ListState, ParentElement, Render,
    SharedString, StatefulInteractiveElement as _, Styled, StyledText, Subscription, Window, div,
    list, px,
};
use std::ops::Range;
use std::rc::Rc;
use std::sync::Arc;

/// One report as the inbox shows it.
struct Item {
    message: Message,
    note: FeedbackNote,
}

/// The rows the list holds, as they were last drawn.
struct Shown {
    items: Vec<Item>,
    view: Arc<WorkspaceView>,
    has_messages: bool,
}

pub struct FeedbackInbox {
    history: Entity<RoomHistory>,
    /// The filter: `None` is All.
    kind: Option<FeedbackKind>,
    list: ListState,
    /// The list's row keys, last frame: the rows that did not change keep
    /// their place.
    keys: Vec<String>,
    shown: Rc<std::cell::RefCell<Option<Shown>>>,
    _subs: Vec<Subscription>,
}

impl FeedbackInbox {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let history = cx.new(|cx| RoomHistory::new(FEEDBACK.to_string(), cx));
        let list = ListState::new(0, ListAlignment::Top, px(600.));
        // The bottom row is the older-pages control: within about 300 px of
        // it, ask for the next page.
        let near = near_edge(&history);
        list.set_scroll_handler(move |ev, window, cx| {
            if ev.count > 0 && ev.visible_range.end + 3 >= ev.count {
                near(window, cx);
            }
        });
        let subs = vec![
            cx.observe(&history, |_, _, cx| cx.notify()),
            cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()),
        ];
        FeedbackInbox {
            history,
            kind: None,
            list,
            keys: Vec::new(),
            shown: Default::default(),
            _subs: subs,
        }
    }

    fn set_kind(&mut self, kind: Option<FeedbackKind>, cx: &mut Context<Self>) {
        if self.kind != kind {
            self.kind = kind;
            cx.notify();
        }
    }

    /// Tell the list which rows changed since the last frame.
    fn sync(&mut self, keys: Vec<String>) {
        if keys == self.keys {
            return;
        }
        if self.keys.is_empty() {
            self.list.reset(keys.len());
        } else {
            let (range, count) = changed(&self.keys, &keys);
            self.list.splice(range, count);
        }
        self.keys = keys;
    }
}

impl Render for FeedbackInbox {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let Some(view) = Desk::get(cx).read(cx).view.clone() else {
            return div().flex_1().into_any_element();
        };
        let h = self.history.read(cx);
        let loading = h.loading;
        let older_key = format!(
            "older:{}:{}:{}:{}",
            h.error,
            h.loading,
            h.next.is_some(),
            h.absent
        );
        let live = room_stream(&view, FEEDBACK);
        let mut newest_first = merge_room_messages(&h.older, &live);
        newest_first.reverse();
        let all: Vec<Item> = newest_first
            .into_iter()
            .map(|m| Item {
                note: parse_feedback(&m.text),
                message: m.clone(),
            })
            .collect();
        let count = |k: FeedbackKind| all.iter().filter(|i| i.note.kind == Some(k)).count();
        let counts: Vec<(FeedbackKind, usize)> =
            FeedbackKind::ALL.iter().map(|&k| (k, count(k))).collect();
        let total = all.len();
        let kind = self.kind;
        let items: Vec<Item> = all
            .into_iter()
            .filter(|i| kind.is_none() || i.note.kind == kind)
            .collect();
        let empty = items.is_empty() && !loading;

        let mut keys: Vec<String> = items.iter().map(|i| i.message.id.clone()).collect();
        keys.push(older_key);
        self.sync(keys);
        *self.shown.borrow_mut() = Some(Shown {
            items,
            view: view.clone(),
            has_messages: total > 0,
        });

        // div.hb-inbox-filters[role=group]
        let filters = div()
            .flex_none()
            .flex()
            .flex_wrap()
            .gap(px(6.))
            .px(px(20.))
            .py(px(10.))
            .border_b_1()
            .border_color(theme.c("line"))
            .child(
                filter(
                    "hb-inbox-all",
                    t(cx, "rooms.inboxAll"),
                    total,
                    kind.is_none(),
                    &theme,
                )
                .on_click(cx.listener(|this, _, _, cx| this.set_kind(None, cx))),
            )
            .children(counts.into_iter().map(|(k, n)| {
                filter(
                    SharedString::from(format!("hb-inbox-{}", k.as_str())),
                    feedback_kind_label(Some(k), cx),
                    n,
                    kind == Some(k),
                    &theme,
                )
                .on_click(cx.listener(move |this, _, _, cx| this.set_kind(Some(k), cx)))
            }));

        let shown = self.shown.clone();
        let history = self.history.clone();
        let rows = list(self.list.clone(), move |ix, _window, cx| {
            let shown = shown.borrow();
            let Some(s) = shown.as_ref() else {
                return div().into_any_element();
            };
            match s.items.get(ix) {
                Some(item) => inbox_row(item, &s.view, cx),
                None => older_control(&history, FEEDBACK, s.has_messages, cx),
            }
        })
        .flex_1()
        .min_h_0();

        // p.hb-inbox-foot: the command stays as typed in every language.
        let foot = div()
            .flex_none()
            .px(px(20.))
            .py(px(10.))
            .border_t_1()
            .border_color(theme.c("line"))
            .font_family(ui::SANS)
            .text_size(px(13.))
            .text_color(theme.c("ink-3"))
            .flex()
            .flex_wrap()
            .child(format!("{} ", t(cx, "rooms.inboxHowTo")))
            .child(div().font_family(ui::MONO).child("harbard feedback"))
            .child(format!(". {}", t(cx, "rooms.inboxLocal")));

        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme.c("surface"))
            .child(filters)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .when(empty, |d| {
                        d.child(
                            div()
                                .flex_none()
                                .px(px(28.))
                                .py(px(40.))
                                .flex()
                                .justify_center()
                                .font_family(ui::SANS)
                                .text_color(theme.c("ink-3"))
                                .child(if kind.is_some() {
                                    t(cx, "rooms.inboxEmptyKind")
                                } else {
                                    t(cx, "rooms.inboxEmpty")
                                }),
                        )
                    })
                    .child(rows),
            )
            .child(foot)
            .into_any_element()
    }
}

/// The rows of `old` that `new` replaces, and how many rows replace them:
/// what the two share at the start and at the end stays.
fn changed(old: &[String], new: &[String]) -> (Range<usize>, usize) {
    let head = old.iter().zip(new).take_while(|(a, b)| a == b).count();
    let room = old.len().min(new.len()) - head;
    let tail = old
        .iter()
        .rev()
        .zip(new.iter().rev())
        .take(room)
        .take_while(|(a, b)| a == b)
        .count();
    (head..old.len() - tail, new.len() - tail - head)
}

/// `button.hb-inbox-filter[aria-pressed]`: a 28 px pill, the label bold and
/// the count plain; pressed, the accent fill.
fn filter(
    id: impl Into<gpui::ElementId>,
    label: SharedString,
    count: usize,
    pressed: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let hover = theme.c("hover");
    let (bg, edge, ink, sub) = if pressed {
        (
            Some(theme.c("accent")),
            theme.c("accent"),
            theme.c("accent-ink"),
            theme.c("accent-ink"),
        )
    } else {
        (
            None,
            theme.c("line-strong"),
            theme.c("ink-2"),
            theme.c("ink-3"),
        )
    };
    div()
        .id(id.into())
        .flex()
        .items_center()
        .gap(px(6.))
        .h(px(28.))
        .px(px(10.))
        .rounded_full()
        .border_1()
        .border_color(edge)
        .when_some(bg, |d, bg| d.bg(bg))
        .when(!pressed, |d| d.hover(move |s| s.bg(hover)))
        .cursor_pointer()
        .font_family(ui::SANS)
        .text_size(px(13.))
        .font_weight(FontWeight::BOLD)
        .text_color(ink)
        .child(label)
        .child(
            div()
                .font_weight(FontWeight::NORMAL)
                .text_color(sub)
                .child(count.to_string()),
        )
}

/// `.hb-chip[data-tone]`, with the accent tone rooms add.
fn kind_chip(kind: Option<FeedbackKind>, cx: &App) -> gpui::Div {
    let theme = cx.global::<Theme>();
    let label = feedback_kind_label(kind, cx);
    match feedback_tone(kind) {
        "accent" => div()
            .flex_none()
            .flex()
            .items_center()
            .h(px(22.))
            .px(px(8.))
            .rounded(px(6.))
            .bg(theme.c("accent-soft"))
            .text_color(theme.c("accent-text"))
            .text_size(px(13.))
            .font_weight(FontWeight::BOLD)
            .child(label),
        tone => crate::chat::message::chip(label, tone, theme),
    }
}

/// `MentionText`: the text with its known `@mentions` in the mention style.
fn mention_text(text: &str, mentions: &[String], theme: &Theme) -> StyledText {
    let mut out = String::with_capacity(text.len());
    let mut ranges: Vec<Range<usize>> = Vec::new();
    for seg in mention_segments(text, mentions) {
        let start = out.len();
        out.push_str(&seg.text);
        if seg.mention {
            ranges.push(start..out.len());
        }
    }
    let style = HighlightStyle {
        color: Some(theme.c("accent-text")),
        background_color: Some(theme.c("accent-soft")),
        font_weight: Some(FontWeight::BOLD),
        ..Default::default()
    };
    StyledText::new(out).with_highlights(ranges.into_iter().map(move |r| (r, style)))
}

/// `li.hb-inbox-row`: avatar 32 without a dot; the kind, who, when, the
/// version; the report.
fn inbox_row(item: &Item, view: &WorkspaceView, cx: &App) -> AnyElement {
    let theme = cx.global::<Theme>();
    let m = &item.message;
    let agent = view.agent(&m.agent);
    let who = match agent {
        Some(a) => crate::derive::agent_name(a, cx),
        None => shown_name(&m.author, cx),
    };
    let face = match agent {
        Some(a) => avatar::agent_avatar(a, 32., false, cx),
        None => avatar::name_avatar(&who, 32., cx),
    };
    let now = Desk::get(cx).read(cx).now_ms();
    div()
        .w_full()
        .flex()
        .gap(px(12.))
        .px(px(20.))
        .py(px(14.))
        .border_b_1()
        .border_color(theme.c("line"))
        .font_family(ui::SANS)
        .child(div().flex_none().child(face))
        .child(
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .items_center()
                        .gap(px(8.))
                        .mb(px(4.))
                        .child(kind_chip(item.note.kind, cx))
                        .child(
                            div()
                                .text_size(px(14.))
                                .font_weight(FontWeight::BOLD)
                                .text_color(theme.c("ink"))
                                .child(who),
                        )
                        .child(
                            div()
                                .text_size(px(12.))
                                .text_color(theme.c("ink-3"))
                                .child(at_date(m.ts_ms, now, cx)),
                        )
                        .children(item.note.version.as_ref().map(|v| {
                            div()
                                .ml_auto()
                                .font_family(ui::MONO)
                                .text_size(px(12.))
                                .text_color(theme.c("ink-4"))
                                .child(tv(cx, "rooms.inboxVersion", &[("version", v)]))
                        })),
                )
                .child(
                    div()
                        .mt(px(2.))
                        .text_size(px(ui::size::BODY))
                        .line_height(gpui::relative(1.46))
                        .text_color(theme.c("ink"))
                        .child(mention_text(&item.note.text, &m.mentions, theme)),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::changed;

    fn keys(s: &str) -> Vec<String> {
        s.split_whitespace().map(String::from).collect()
    }

    #[test]
    fn an_older_page_replaces_only_the_bottom_control() {
        assert_eq!(
            changed(&keys("a b older:1"), &keys("a b c d older:2")),
            (2..3, 3)
        );
    }

    #[test]
    fn a_new_report_lands_on_top() {
        assert_eq!(changed(&keys("a b older"), &keys("n a b older")), (0..0, 1));
    }

    #[test]
    fn nothing_changed_replaces_nothing() {
        assert_eq!(changed(&keys("a b older"), &keys("a b older")), (3..3, 0));
    }

    #[test]
    fn a_filter_replaces_the_middle() {
        assert_eq!(changed(&keys("a b c older"), &keys("b older")), (0..3, 1));
    }
}
