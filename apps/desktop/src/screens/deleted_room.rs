//! A deleted room (`/history/rooms/<name>`), read-only, on the same history
//! as any other room: back to History, the room's name, the strip saying
//! it was deleted and when, and its messages. No composer.

use super::rooms::RoomFeed;
use super::secondary::{self as kit, head_bar, main_column, workspace};
use crate::api::enc;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::history::DeletedRoom;
use crate::nav::{self, Nav, Route};
use crate::theme::Theme;
use crate::ui::{self, control, format::at_date, icon_size, radius, size, space, weight};
use gpui::{
    AppContext, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
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
            let found = task
                .await
                .ok()
                .and_then(|all| all.into_iter().find(|r| r.name == room));
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
        let back = kit::control_focus(div().id("deleted-back"), &theme)
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(control::SM))
            .rounded(px(radius::BASE))
            .cursor_pointer()
            .hover(move |s| s.bg(hover))
            .tooltip(kit::tip(t(cx, "rooms.backToHistory")))
            .on_click(|_, _, cx| {
                nav::navigate(
                    Route::History {
                        channel: None,
                        q: String::new(),
                    },
                    cx,
                )
            })
            .child(ui::icon("arrow-left", icon_size::ROW, theme.c("ink-2")));
        // The strip below says it was deleted, and when; the header only
        // names the room.
        let mut screen = main_column(&theme).child(
            head_bar(&theme).gap(px(space::MD)).child(back).child(
                ui::text(
                    format!("#{}", self.room),
                    size::TITLE,
                    weight::HEAVY,
                    theme.c("ink"),
                )
                .min_w_0()
                .truncate(),
            ),
        );
        match &self.known {
            Some(Some(room)) => {
                screen = screen.child(
                    div()
                        .flex_none()
                        .flex()
                        .items_center()
                        .min_h(px(control::MD))
                        .px(px(space::GUTTER))
                        .py(px(space::SM))
                        .bg(theme.c("warn-bg"))
                        .child(ui::text(
                            tv(
                                cx,
                                "rooms.deletedStrip",
                                &[("when", &at_date(room.deleted_ms, now, cx))],
                            ),
                            size::SMALL,
                            weight::REGULAR,
                            theme.c("warn-ink"),
                        )),
                );
                if let Some(feed) = &self.feed {
                    screen = screen.child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .child(feed.clone()),
                    );
                }
            }
            Some(None) => {
                screen = screen.child(kit::empty_block(
                    None,
                    vec![
                        tv(cx, "rooms.deletedMissing", &[("room", &self.room)]).into_any_element(),
                    ],
                    &theme,
                ));
            }
            None => {}
        }
        screen
    }
}
