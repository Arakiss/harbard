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
use crate::ui::{
    self, avatar, avatar_size, control, format::at_date, leading, radius, size, space, weight,
};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, HighlightStyle, InteractiveElement as _,
    IntoElement, ListAlignment, ListState, ParentElement, Render, SharedString,
    StatefulInteractiveElement as _, Styled, StyledText, Subscription, Window, div, list, px,
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
            .gap(px(space::SM))
            .px(px(space::GUTTER))
            .py(px(space::MD))
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
            .px(px(space::GUTTER))
            .py(px(space::MD))
            .border_t_1()
            .border_color(theme.c("line"))
            .font_family(ui::SANS)
            .text_size(px(size::SMALL))
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
                            ui::empty(
                                if kind.is_some() {
                                    t(cx, "rooms.inboxEmptyKind")
                                } else {
                                    t(cx, "rooms.inboxEmpty")
                                },
                                &theme,
                            )
                            .flex_none(),
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

/// A filter: a 28 px quiet button, its count as metadata. Pressed, the
/// label is 700 on the control wash (a filter is not an accent job, C2).
fn filter(
    id: impl Into<gpui::ElementId>,
    label: SharedString,
    count: usize,
    pressed: bool,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    let hover = theme.c("hover");
    crate::chat::focusable(
        div()
            .id(id.into())
            .flex()
            .flex_none()
            .items_center()
            .gap(px(space::SM))
            .h(px(control::SM))
            .px(px(space::MD))
            .rounded(px(radius::BASE))
            .when(pressed, |d| d.bg(theme.c("control")))
            .when(!pressed, |d| d.hover(move |s| s.bg(hover)))
            .cursor_pointer()
            .font_family(ui::SANS)
            .text_size(px(size::UI))
            .line_height(px(leading::UI))
            .font_weight(if pressed {
                weight::STRONG
            } else {
                weight::REGULAR
            })
            .text_color(if pressed {
                theme.c("ink")
            } else {
                theme.c("ink-2")
            }),
        false,
        theme,
    )
    .child(label)
    .child(ui::text(
        count.to_string(),
        size::META,
        weight::REGULAR,
        theme.c("ink-3"),
    ))
}

/// The report's kind as the shared chip; a kind without a state is neutral.
fn kind_chip(kind: Option<FeedbackKind>, cx: &App) -> gpui::Div {
    let theme = cx.global::<Theme>();
    let label = feedback_kind_label(kind, cx);
    crate::chat::message::chip(label, feedback_tone(kind), theme)
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
        font_weight: Some(weight::STRONG),
        ..Default::default()
    };
    StyledText::new(out).with_highlights(ranges.into_iter().map(move |r| (r, style)))
}

/// One report: the avatar without a dot; who, the kind, when, the
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
        Some(a) => avatar::agent_avatar(a, avatar_size::ROW, false, cx),
        None => avatar::name_avatar(&who, avatar_size::ROW, cx),
    };
    let now = Desk::get(cx).read(cx).now_ms();
    div()
        .w_full()
        .flex()
        .gap(px(space::LG))
        .px(px(space::GUTTER))
        .py(px(space::LG))
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
                        .gap(px(space::MD))
                        .mb(px(space::HAIR))
                        .child(ui::text(who, size::BODY, weight::HEAVY, theme.c("ink")))
                        .child(kind_chip(item.note.kind, cx))
                        .child(ui::text(
                            at_date(m.ts_ms, now, cx),
                            size::META,
                            weight::REGULAR,
                            theme.c("ink-3"),
                        ))
                        .children(item.note.version.as_ref().map(|v| {
                            ui::text(
                                tv(cx, "rooms.inboxVersion", &[("version", v)]),
                                size::META,
                                weight::REGULAR,
                                theme.c("ink-3"),
                            )
                            .ml_auto()
                            .font_family(ui::MONO)
                        })),
                )
                .child(
                    div()
                        .text_size(px(size::BODY))
                        .line_height(px(leading::BODY))
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
