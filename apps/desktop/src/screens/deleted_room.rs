//! A deleted room (`/history/rooms/<name>`), read-only, on the same history
//! as any other room: back to History, the room's name with a red
//! "Deleted", the strip saying when, and its messages. No composer.

use super::room_feed::RoomFeed;
use super::secondary::{self as kit, head_bar, main_column, workspace};
use crate::api::enc;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::history::DeletedRoom;
use crate::nav::{self, Nav, Route};
use crate::theme::Theme;
use crate::ui::{self, format::at_date, size};
use gpui::{
    AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Task, Window, div, px,
};

pub struct DeletedRoomScreen {
    room: String,
    /// `None` while loading; `Some(None)` when no deleted room has the name.
    known: Option<Option<DeletedRoom>>,
    feed: Option<Entity<RoomFeed>>,
    _load: Option<Task<()>>,
}

impl DeletedRoomScreen {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        cx.observe_global::<Nav>(|this, cx| {
            if let Route::DeletedRoom { name } = cx.global::<Nav>().route.clone()
                && name != this.room
            {
                this.open(name, cx);
            }
        })
        .detach();
        let room = match cx.global::<Nav>().route.clone() {
            Route::DeletedRoom { name } => name,
            _ => String::new(),
        };
        let mut this = DeletedRoomScreen {
            room: String::new(),
            known: None,
            feed: None,
            _load: None,
        };
        this.open(room, cx);
        this
    }

    fn open(&mut self, room: String, cx: &mut Context<Self>) {
        self.room = room.clone();
        self.known = None;
        self.feed = None;
        let path = match workspace(cx) {
            Some(ws) => format!("/channels/deleted?ws={}", enc(&ws)),
            None => "/channels/deleted".to_string(),
        };
        let task = kit::fetch::<Vec<DeletedRoom>>(path, cx);
        self._load = Some(cx.spawn(async move |this, cx| {
            let found = task.await.ok().and_then(|all| all.into_iter().find(|r| r.name == room));
            let _ = this.update(cx, |this, cx| {
                if found.is_some() {
                    let empty = tv(cx, "rooms.emptyRoom", &[("room", &this.room)]);
                    let name = this.room.clone();
                    this.feed = Some(cx.new(|cx| RoomFeed::new(name, true, empty, cx)));
                }
                this.known = Some(found);
                cx.notify();
            });
        }));
        cx.notify();
    }
}

impl Render for DeletedRoomScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let now = Desk::get(cx).read(cx).now_ms();
        let hover = theme.c("hover");
        let back = div()
            .id("deleted-back")
            .flex()
            .items_center()
            .justify_center()
            .size(px(32.))
            .rounded(px(6.))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .on_click(|_, _, cx| {
                nav::navigate(
                    Route::History {
                        channel: None,
                        q: String::new(),
                    },
                    cx,
                )
            })
            .child(ui::icon("arrow-left", 20., theme.c("ink-2")));
        let mut screen = main_column(&theme).child(
            head_bar(&theme)
                .child(back)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(6.))
                        .text_size(px(19.))
                        .font_weight(FontWeight::BLACK)
                        .child(ui::icon("hash", 20., theme.c("ink")))
                        .child(self.room.clone()),
                )
                .child(kit::chip(t(cx, "rooms.deletedBadge"), "red", &theme)),
        );
        match &self.known {
            Some(Some(room)) => {
                screen = screen.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .px(px(20.))
                        .py(px(8.))
                        .bg(theme.c("warn-bg"))
                        .text_color(theme.c("warn-ink"))
                        .text_size(px(size::UI))
                        .child(tv(
                            cx,
                            "rooms.deletedStrip",
                            &[("when", &at_date(room.deleted_ms, now, cx))],
                        )),
                );
                if let Some(feed) = &self.feed {
                    screen = screen.child(div().flex().flex_col().flex_1().min_h_0().child(feed.clone()));
                }
            }
            Some(None) => {
                screen = screen.child(kit::empty_block(
                    None,
                    vec![tv(cx, "rooms.deletedMissing", &[("room", &self.room)]).into_any_element()],
                    &theme,
                ));
            }
            None => {}
        }
        screen
    }
}
