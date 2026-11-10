//! A room's messages (`RoomFeed`): the live view's lines for the room plus
//! the older ones serve keeps (`GET /channels/<room>/history`), each once,
//! oldest first, with "Load older messages" at the top. Rooms are flat:
//! replies stay inline. Used by the Lobby's `#lobby` column and by a
//! deleted room (read-only).

use super::secondary::{self as kit, workspace};
use crate::api::enc;
use crate::chat::{self, MessageOpts};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::Message;
use crate::model::history::RoomHistoryPage;
use crate::theme::Theme;
use crate::ui::size;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Task, Window, div, px,
};
use std::collections::HashMap;

pub struct RoomFeed {
    room: String,
    pub read_only: bool,
    empty: SharedString,
    older: Vec<Message>,
    next: Option<String>,
    loading: bool,
    error: bool,
    /// serve has no history for this room (an older serve, or no log).
    absent: bool,
    scroll: ScrollHandle,
    /// How many messages were shown last frame, to follow new ones.
    shown: usize,
    seen_older: usize,
    _load: Option<Task<()>>,
}

impl RoomFeed {
    pub fn new(room: String, read_only: bool, empty: SharedString, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()).detach();
        let mut feed = RoomFeed {
            room,
            read_only,
            empty,
            older: Vec::new(),
            next: None,
            loading: false,
            error: false,
            absent: false,
            scroll: ScrollHandle::new(),
            shown: 0,
            seen_older: 0,
            _load: None,
        };
        feed.load(None, cx);
        feed
    }

    fn load(&mut self, before: Option<String>, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = false;
        let mut path = format!("/channels/{}/history?limit=50", enc(&self.room));
        if let Some(b) = &before {
            path.push_str(&format!("&before={}", enc(b)));
        }
        if let Some(ws) = workspace(cx) {
            path.push_str(&format!("&ws={}", enc(&ws)));
        }
        let task = kit::fetch::<RoomHistoryPage>(path, cx);
        self._load = Some(cx.spawn(async move |this, cx| {
            let out = task.await;
            let _ = this.update(cx, |feed, cx| {
                feed.loading = false;
                match out {
                    Ok(page) => {
                        let mut merged = page.messages;
                        merged.append(&mut feed.older);
                        feed.older = merge(&merged, &[]);
                        feed.next = page.next_before;
                    }
                    Err(e) if e.missing() => feed.absent = true,
                    Err(_) => feed.error = true,
                }
                cx.notify();
            });
        }));
        cx.notify();
    }
}

/// History and the live view, each message once, oldest first; the live
/// copy wins. Structured lines from history are left out.
pub fn merge(history: &[Message], live: &[&Message]) -> Vec<Message> {
    let mut by_id: HashMap<&str, &Message> = HashMap::new();
    for m in history {
        if m.structured.is_none() {
            by_id.insert(&m.id, m);
        }
    }
    for m in live {
        by_id.insert(&m.id, m);
    }
    let mut out: Vec<Message> = by_id.into_values().cloned().collect();
    out.sort_by(|a, b| a.ts_ms.cmp(&b.ts_ms).then_with(|| a.id.cmp(&b.id)));
    out
}

impl Render for RoomFeed {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let Some(view) = Desk::get(cx).read(cx).view.clone() else {
            return div().flex_1().min_h_0().into_any_element();
        };
        let live: Vec<&Message> = view
            .messages
            .iter()
            .filter(|m| m.channels.contains(&self.room) && m.structured.is_none())
            .collect();
        let messages = merge(&self.older, &live);
        // Follow new lines when the reader is at the bottom.
        if messages.len() != self.shown {
            let at_bottom = self.scroll.offset().y <= -self.scroll.max_offset().y + px(40.);
            if self.shown == 0 || at_bottom {
                self.scroll.scroll_to_bottom();
            }
            self.shown = messages.len();
        }
        // Older pages arrive as the top comes near (300px), and growing
        // above the fold keeps what is on screen where it was.
        if self.older.len() != self.seen_older {
            let before = self.scroll.max_offset().y;
            let first = self.seen_older == 0;
            self.seen_older = self.older.len();
            if !first {
                let scroll = self.scroll.clone();
                window.on_next_frame(move |_, _| {
                    let delta = scroll.max_offset().y - before;
                    let at = scroll.offset();
                    scroll.set_offset(gpui::point(at.x, at.y - delta));
                });
            }
        }
        if let Some(next) = self.next.clone()
            && !self.loading
            && !self.error
            && self.shown > 0
            && self.scroll.offset().y > px(-300.)
        {
            let entity = cx.entity();
            window.on_next_frame(move |_, cx| {
                entity.update(cx, |feed, cx| {
                    if !feed.loading && feed.next.as_deref() == Some(next.as_str()) {
                        feed.load(Some(next.clone()), cx);
                    }
                });
            });
        }
        let refs: Vec<&Message> = messages.iter().collect();
        let head = older_control(self, !messages.is_empty(), &theme, cx);
        let body = if messages.is_empty() {
            if self.loading {
                div().into_any_element()
            } else {
                kit::empty_block(None, vec![self.empty.clone().into_any_element()], &theme).into_any_element()
            }
        } else {
            chat::stream(&refs, &view, &MessageOpts::default(), cx)
        };
        div()
            .id(SharedString::from(format!("room-feed-{}", self.room)))
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .pt(px(8.))
                    .pb(px(12.))
                    .children(head)
                    .child(body),
            )
            .into_any_element()
    }
}

/// "Load older messages", the error with "Try again", or "This is the very
/// start of #room".
fn older_control(feed: &RoomFeed, has: bool, theme: &Theme, cx: &mut Context<RoomFeed>) -> Option<gpui::AnyElement> {
    let row = || {
        div()
            .flex()
            .justify_center()
            .items_center()
            .gap(px(8.))
            .min_h(px(12.))
            .px(px(20.))
            .py(px(6.))
            .text_size(px(size::SMALL))
            .text_color(theme.c("ink-4"))
    };
    let more_button = |id: &'static str, label: SharedString, disabled: bool| {
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
            .font_weight(gpui::FontWeight::BOLD)
            .text_color(if disabled { theme.c("ink-4") } else { theme.c("ink") })
            .when(!disabled, |d| d.cursor_pointer())
            .child(label)
    };
    if feed.error {
        let next = feed.next.clone();
        return Some(
            row()
                .child(t(cx, "rooms.historyFailed"))
                .child(
                    more_button("room-retry", t(cx, "rooms.retry"), false)
                        .on_click(cx.listener(move |feed, _, _, cx| feed.load(next.clone(), cx))),
                )
                .into_any_element(),
        );
    }
    if let Some(next) = feed.next.clone() {
        let label = if feed.loading {
            t(cx, "rooms.loadingOlder")
        } else {
            t(cx, "rooms.loadOlder")
        };
        let busy = feed.loading;
        return Some(
            row()
                .child(
                    more_button("room-older", label, busy).on_click(cx.listener(move |feed, _, _, cx| {
                        if !feed.loading {
                            feed.load(Some(next.clone()), cx);
                        }
                    })),
                )
                .into_any_element(),
        );
    }
    if !feed.loading && !feed.absent && has {
        return Some(
            row()
                .child(
                    div()
                        .pt(px(10.))
                        .pb(px(2.))
                        .child(tv(cx, "rooms.startOfRoom", &[("room", &feed.room)])),
                )
                .into_any_element(),
        );
    }
    Some(row().into_any_element())
}
