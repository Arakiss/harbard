//! History's way to a deleted room (`DeletedRooms`, `section.hb-hist-deleted`).
//! Absent while there are none, or when serve predates them.

use super::feed::current_workspace;
use crate::api;
use crate::derive::rooms::DeletedRoom;
use crate::desk::Desk;
use crate::i18n::t;
use crate::nav::{self, Route};
use crate::theme::Theme;
use crate::ui::{self, format::at_date, radius, row, size, space, weight};
use gpui::{
    App, AppContext as _, Context, InteractiveElement as _, IntoElement, ParentElement, Render,
    SharedString, StatefulInteractiveElement as _, Styled, Task, Window, div, px,
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
        let hover = theme.c("hover");
        // A section of the page, not a card on it (C4): its label and
        // sentence at the gutter, then one row per room.
        div()
            .px(px(space::GUTTER))
            .mt(px(space::SECTION))
            .mb(px(space::XL))
            .font_family(ui::SANS)
            .child(ui::section_label(t(cx, "rooms.deletedTitle"), &theme))
            .child(
                ui::text(
                    t(cx, "rooms.deletedHint"),
                    size::SMALL,
                    weight::REGULAR,
                    theme.c("ink-3"),
                )
                .mt(px(space::HAIR))
                .mb(px(space::MD)),
            )
            .children(self.rooms.iter().map(|room| {
                let name = room.name.clone();
                crate::chat::focusable(
                    div()
                        .id(SharedString::from(format!("hb-hist-deleted-{name}")))
                        .flex()
                        .items_center()
                        .gap(px(space::LG))
                        .h(px(row::MENU))
                        .mx(px(-space::MD))
                        .px(px(space::MD))
                        .rounded(px(radius::BASE))
                        .cursor_pointer()
                        .hover(move |s| s.bg(hover)),
                    true,
                    &theme,
                )
                .on_click(move |_, _, cx| {
                    nav::navigate(Route::DeletedRoom { name: name.clone() }, cx)
                })
                .child(
                    ui::text(
                        format!("#{}", room.name),
                        size::UI,
                        weight::STRONG,
                        theme.c("ink"),
                    )
                    .flex_none(),
                )
                .child(
                    ui::text(
                        deleted_meta(room, now, cx),
                        size::SMALL,
                        weight::REGULAR,
                        theme.c("ink-3"),
                    )
                    .min_w_0()
                    .truncate(),
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
