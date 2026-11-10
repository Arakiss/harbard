//! A room's messages with older pages from serve (`RoomFeed`,
//! `useRoomHistory` and `OlderControl` in `rooms.tsx`).
//!
//! The workspace read holds only recent messages; the room's log on disk is
//! what makes everything ever said reachable. [`RoomHistory`] asks serve
//! for it a page at a time, newest page first, and [`RoomFeed`] joins those
//! pages to the live read.

use crate::api;
use crate::chat::{self, MessageOpts, stream::NearTop};
use crate::derive::rooms::{RoomHistoryPage, merge_room_messages, room_stream};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::Message;
use crate::theme::Theme;
use crate::ui;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Div, ElementId, Entity, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Stateful,
    StatefulInteractiveElement as _, Styled, Subscription, Task, Window, div, px,
};
use std::rc::Rc;

/// Messages asked for at a time. The contract allows 200; a smaller page
/// keeps the first paint of a long room quick, and the next one is a scroll
/// away.
pub const HISTORY_PAGE: usize = 50;

/// `GET /channels/<room>/history?limit=50[&before=][&ws=]`.
pub fn history_path(room: &str, before: Option<&str>, workspace: Option<&str>) -> String {
    let mut path = format!("/channels/{}/history?limit={HISTORY_PAGE}", api::enc(room));
    if let Some(b) = before {
        path.push_str(&format!("&before={}", api::enc(b)));
    }
    if let Some(ws) = workspace {
        path.push_str(&format!("&ws={}", api::enc(ws)));
    }
    path
}

/// The workspace the reads and writes are for, when one is chosen
/// (`currentWorkspace()`; "all" is not one).
pub fn current_workspace(cx: &App) -> Option<String> {
    Desk::get(cx)
        .read(cx)
        .workspace
        .clone()
        .filter(|w| !w.is_empty() && w != "all")
}

/// The room's log on disk, a page at a time and newest page first
/// (`useRoomHistory`). A new room starts from nothing.
pub struct RoomHistory {
    pub room: String,
    /// Every page read so far, oldest first.
    pub older: Vec<Message>,
    /// The cursor for the page before the oldest one read; `None` at the
    /// very start of the room.
    pub next: Option<String>,
    pub loading: bool,
    pub error: bool,
    /// serve has no such route, or this room left no log: nothing further
    /// to ask.
    pub absent: bool,
    /// The page on its way; replacing it drops the answer of the old one.
    task: Option<Task<()>>,
}

impl RoomHistory {
    pub fn new(room: String, cx: &mut Context<Self>) -> Self {
        let mut history = RoomHistory {
            room,
            older: Vec::new(),
            next: None,
            loading: true,
            error: false,
            absent: false,
            task: None,
        };
        history.load(None, cx);
        history
    }

    /// Start over on another room.
    pub fn set_room(&mut self, room: String, cx: &mut Context<Self>) {
        if self.room == room {
            return;
        }
        self.room = room;
        self.older.clear();
        self.next = None;
        self.absent = false;
        self.load(None, cx);
    }

    /// Ask for the page before `before` (`None`: the newest page).
    pub fn load(&mut self, before: Option<String>, cx: &mut Context<Self>) {
        self.loading = true;
        self.error = false;
        cx.notify();
        let path = history_path(
            &self.room,
            before.as_deref(),
            current_workspace(cx).as_deref(),
        );
        let room = self.room.clone();
        let page = cx.background_spawn(async move { api::get_as::<RoomHistoryPage>(&path) });
        self.task = Some(cx.spawn(async move |this, cx| {
            let out = page.await;
            let _ = this.update(cx, |history, cx| {
                if history.room == room {
                    history.apply(out);
                    cx.notify();
                }
            });
        }));
    }

    fn apply(&mut self, out: Result<RoomHistoryPage, api::ApiError>) {
        self.loading = false;
        match out {
            Ok(page) => {
                let read: Vec<&Message> = self.older.iter().collect();
                let merged: Vec<Message> = merge_room_messages(&page.messages, &read)
                    .into_iter()
                    .cloned()
                    .collect();
                self.older = merged;
                self.next = page.next_before;
                self.error = false;
                self.absent = false;
            }
            Err(e) => {
                self.absent = e.missing();
                self.error = !self.absent;
            }
        }
    }

    /// Whether the edge of the list may ask for more: a next page, nothing
    /// on its way, no error showing.
    pub fn can_load_more(&self) -> bool {
        self.next.is_some() && !self.loading && !self.error
    }

    /// The reader came near the oldest message: the next page, if allowed.
    pub fn near_edge(&mut self, cx: &mut Context<Self>) {
        if self.can_load_more() {
            let next = self.next.clone();
            self.load(next, cx);
        }
    }

    /// "Try again" and "Load older messages": the same cursor again.
    pub fn retry(&mut self, cx: &mut Context<Self>) {
        let next = self.next.clone();
        self.load(next, cx);
    }
}

/// `.hb-hist-more`: a quiet outlined button, 34 px.
pub fn hist_more(
    id: impl Into<ElementId>,
    label: SharedString,
    disabled: bool,
    theme: &Theme,
) -> Stateful<Div> {
    let hover = theme.c("hover");
    div()
        .id(id.into())
        .flex_none()
        .flex()
        .items_center()
        .h(px(34.))
        .px(px(14.))
        .rounded(px(8.))
        .border_1()
        .border_color(theme.c("line-strong"))
        .bg(theme.c("surface"))
        .text_color(if disabled {
            theme.c("ink-4")
        } else {
            theme.c("ink")
        })
        .font_family(ui::SANS)
        .text_size(px(14.))
        .font_weight(FontWeight::BOLD)
        .when(!disabled, |d| {
            d.cursor_pointer().hover(move |s| s.bg(hover))
        })
        .child(label)
}

/// `OlderControl` (`div.hb-room-older`): the error and Try again, "Load
/// older messages", or where the room begins. The button is also the
/// keyboard's way back.
pub fn older_control(
    history: &Entity<RoomHistory>,
    room: &str,
    has_messages: bool,
    cx: &App,
) -> AnyElement {
    let theme = cx.global::<Theme>();
    let h = history.read(cx);
    let base = div()
        .w_full()
        .flex()
        .justify_center()
        .min_h(px(12.))
        .px(px(20.))
        .py(px(6.))
        .font_family(ui::SANS)
        .text_size(px(13.))
        .text_color(theme.c("ink-4"));
    let retry = {
        let history = history.clone();
        move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut App| {
            history.update(cx, |h, cx| h.retry(cx))
        }
    };
    if h.error {
        return base
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(4.))
                    .child(t(cx, "rooms.historyFailed"))
                    .child(
                        hist_more("hb-room-older-retry", t(cx, "rooms.retry"), false, theme)
                            .on_click(retry),
                    ),
            )
            .into_any_element();
    }
    if h.next.is_some() {
        let label = if h.loading {
            t(cx, "rooms.loadingOlder")
        } else {
            t(cx, "rooms.loadOlder")
        };
        let loading = h.loading;
        return base
            .child(
                hist_more("hb-room-older-more", label, loading, theme)
                    .when(!loading, |d| d.on_click(retry)),
            )
            .into_any_element();
    }
    if !h.loading && !h.absent && has_messages {
        return base
            .child(div().pt(px(10.)).pb(px(2.)).child(tv(
                cx,
                "rooms.startOfRoom",
                &[("room", room)],
            )))
            .into_any_element();
    }
    base.into_any_element()
}

/// Calls the history's `near_edge` after the frame (the list calls it
/// while it lays out).
pub fn near_edge(history: &Entity<RoomHistory>) -> NearTop {
    let history = history.downgrade();
    Rc::new(move |_, cx| {
        let history = history.clone();
        cx.defer(move |cx| {
            let _ = history.update(cx, |h, cx| h.near_edge(cx));
        });
    })
}

/// A room's messages: the live read joined to the log on disk, older pages
/// arriving as the top comes near; the rows on screen stay where they were.
/// Also the read-only feed of a deleted room.
pub struct RoomFeed {
    pub room: String,
    pub read_only: bool,
    pub empty: SharedString,
    history: Entity<RoomHistory>,
    _subs: Vec<Subscription>,
}

impl RoomFeed {
    pub fn new(
        room: String,
        read_only: bool,
        empty: SharedString,
        cx: &mut Context<Self>,
    ) -> Self {
        let history = cx.new(|cx| RoomHistory::new(room.clone(), cx));
        let subs = vec![
            cx.observe(&history, |_, _, cx| cx.notify()),
            cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()),
        ];
        RoomFeed {
            room,
            read_only,
            empty,
            history,
            _subs: subs,
        }
    }

    /// Show another room from its newest page (the console remounted the
    /// feed keyed by room).
    pub fn set_room(&mut self, room: String, empty: SharedString, cx: &mut Context<Self>) {
        if self.empty != empty {
            self.empty = empty;
            cx.notify();
        }
        if self.room != room {
            self.room = room.clone();
            self.history.update(cx, |h, cx| h.set_room(room, cx));
            cx.notify();
        }
    }

    /// The pages read so far (for the screen around the feed).
    pub fn history(&self) -> &Entity<RoomHistory> {
        &self.history
    }
}

impl Render for RoomFeed {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(view) = Desk::get(cx).read(cx).view.clone() else {
            return div().flex_1().into_any_element();
        };
        let h = self.history.read(cx);
        let live = room_stream(&view, &self.room);
        let messages = merge_room_messages(&h.older, &live);
        let head = older_control(&self.history, &self.room, !messages.is_empty(), cx);
        let opts = MessageOpts {
            fold: true,
            read_only: self.read_only,
            empty: (!h.loading).then(|| self.empty.clone()),
            key: Some(format!("room-{}", self.room).into()),
            ..Default::default()
        };
        div()
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .child(chat::stream::stream_with(
                &messages,
                &view,
                &opts,
                Some(head),
                Some(near_edge(&self.history)),
                cx,
            ))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_path_encodes_the_room_and_the_cursor() {
        assert_eq!(
            history_path("commons", None, None),
            "/channels/commons/history?limit=50"
        );
        assert_eq!(
            history_path("a b", Some("c/d"), Some("ws 1")),
            "/channels/a%20b/history?limit=50&before=c%2Fd&ws=ws%201"
        );
    }
}
