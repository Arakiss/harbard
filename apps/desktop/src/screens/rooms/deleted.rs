//! History's way to a deleted room (`DeletedRooms`, `section.hb-hist-deleted`).
//! Absent while there are none, or when serve predates them.

use super::feed::current_workspace;
use crate::api;
use crate::derive::rooms::DeletedRoom;
use crate::desk::Desk;
use crate::i18n::t;
use crate::nav::{self, Route};
use crate::theme::Theme;
use crate::ui::{self, format::at_date};
use gpui::{
    App, AppContext as _, Context, FontWeight, InteractiveElement as _, IntoElement, ParentElement,
    Render, SharedString, StatefulInteractiveElement as _, Styled, Task, Window, div, px,
};

/// `GET /channels/deleted[?ws=]`.
pub fn deleted_path(workspace: Option<&str>) -> String {
    match workspace {
        Some(ws) => format!("/channels/deleted?ws={}", api::enc(ws)),
        None => "/channels/deleted".into(),
    }
}

/// "Deleted {when} · {count} messages" / "Deleted {when} · 1 message".
pub fn deleted_meta(room: &DeletedRoom, now: u64, cx: &App) -> SharedString {
    let when = at_date(room.deleted_ms, now, cx);
    if room.messages == 1 {
        crate::i18n::tv(cx, "rooms.deletedRowMetaOne", &[("when", &when)])
    } else {
        crate::i18n::tv(
            cx,
            "rooms.deletedRowMeta",
            &[("when", &when), ("count", &room.messages.to_string())],
        )
    }
}

pub struct DeletedRooms {
    rooms: Vec<DeletedRoom>,
    _load: Task<()>,
}

impl DeletedRooms {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let path = deleted_path(current_workspace(cx).as_deref());
        let read = cx.background_spawn(async move { api::get_as::<Vec<DeletedRoom>>(&path) });
        let load = cx.spawn(async move |this, cx| {
            let rooms = read.await.unwrap_or_default();
            let _ = this.update(cx, |this, cx| {
                this.rooms = rooms;
                cx.notify();
            });
        });
        DeletedRooms {
            rooms: Vec::new(),
            _load: load,
        }
    }
}

impl Render for DeletedRooms {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.rooms.is_empty() {
            return div().into_any_element();
        }
        let theme = cx.global::<Theme>().clone();
        let now = Desk::get(cx).read(cx).now_ms();
        let hover = theme.c("hover-2");
        div()
            .mx(px(20.))
            .mt(px(8.))
            .mb(px(16.))
            .px(px(16.))
            .py(px(14.))
            .rounded(px(10.))
            .bg(theme.c("tint"))
            .font_family(ui::SANS)
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
                    .text_size(px(13.))
                    .text_color(theme.c("ink-3"))
                    .child(t(cx, "rooms.deletedHint")),
            )
            .children(self.rooms.iter().map(|room| {
                let name = room.name.clone();
                div()
                    .id(SharedString::from(format!("hb-hist-deleted-{name}")))
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
                    .on_click(move |_, _, cx| {
                        nav::navigate(Route::DeletedRoom { name: name.clone() }, cx)
                    })
                    .child(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .text_color(theme.c("ink"))
                            .child(format!("#{}", room.name)),
                    )
                    .child(
                        div()
                            .text_size(px(13.))
                            .text_color(theme.c("ink-3"))
                            .child(deleted_meta(room, now, cx)),
                    )
            }))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::deleted_path;

    #[test]
    fn deleted_path_carries_the_workspace() {
        assert_eq!(deleted_path(None), "/channels/deleted");
        assert_eq!(deleted_path(Some("a b")), "/channels/deleted?ws=a%20b");
    }
}
