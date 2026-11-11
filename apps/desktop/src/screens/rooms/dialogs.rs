//! The room dialogs (`CreateRoomDialog`, `DeleteRoomDialog`), in the shell
//! every small dialog of the app shares (`RoomDialog`, `.hb-small-dialog`):
//! a 56 px head with the title and a close X, then the body. Esc and a
//! click on the scrim close it; the control marked autofocus has the focus.

use super::{Tone, button, button_shell, post_channel_action, refusal};
use crate::derive::rooms::{normalize_room_name, room_name_note, room_name_ok};
use crate::derive::workspace::channel_route;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::nav::{self, Nav, Route};
use crate::theme::Theme;
use crate::ui::{self, control, icon_size, leading, radius, row, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    ParentElement, Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled,
    Subscription, Window, div, px,
};
use gpui_kit::component::WindowExt as _;
use gpui_kit::component::dialog::Dialog;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use serde_json::json;
use std::cell::Cell;

/// The purpose field's `maxLength`.
const PURPOSE_MAX: usize = 240;
/// A small dialog's width.
const WIDTH: f32 = 480.;

/// Open a `.hb-small-dialog` titled `title` around `body`; `focus` runs on
/// the first build only (the autofocus control).
fn open_small<V: Render>(
    title: SharedString,
    body: Entity<V>,
    focus: impl Fn(&mut Window, &mut App) + 'static,
    window: &mut Window,
    cx: &mut App,
) {
    let top = window.viewport_size().height * 0.12;
    let first = Cell::new(true);
    window.open_dialog(cx, move |dialog, window, cx| {
        if first.replace(false) {
            focus(window, cx);
        }
        small_dialog(dialog, title.clone(), top, cx).child(
            div()
                .px(px(space::GUTTER))
                .py(px(space::XL))
                .child(body.clone()),
        )
    });
}

/// `.hb-finder-scrim > .hb-small-dialog`: 480 px, 12 % from the top, the
/// raised surface, 12 px corners, the pop shadow, and its `.hb-pane-head`.
fn small_dialog(dialog: Dialog, title: SharedString, top: Pixels, cx: &App) -> Dialog {
    let theme = cx.global::<Theme>();
    dialog
        .w(px(WIDTH))
        .margin_top(top)
        .close_button(false)
        .p_0()
        .gap_0()
        .min_h_0()
        .bg(theme.c("surface-raised"))
        .border_0()
        .rounded(px(radius::LG))
        .shadow(theme.pop_shadow())
        .font_family(ui::SANS)
        .text_size(px(size::UI))
        .line_height(px(leading::UI))
        .text_color(theme.c("ink"))
        .child(pane_head(title, cx))
}

/// The dialog's head (T4): the title at 18/900 in a 56 px row, and the
/// close button with its key.
fn pane_head(title: SharedString, cx: &App) -> impl IntoElement {
    let theme = cx.global::<Theme>();
    let hover = theme.c("control");
    div()
        .h(px(row::HEADER))
        .flex_none()
        .flex()
        .items_center()
        .gap(px(space::MD))
        .pl(px(space::GUTTER))
        .pr(px(space::GUTTER - space::SM))
        .border_b_1()
        .border_color(theme.c("line"))
        .child(
            ui::text(title, size::TITLE, weight::HEAVY, theme.c("ink"))
                .min_w_0()
                .truncate(),
        )
        .child(
            crate::chat::focusable(
                div()
                    .id("hb-room-dialog-close")
                    .ml_auto()
                    .flex_none()
                    .size(px(control::SM))
                    .rounded(px(radius::BASE))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(move |s| s.bg(hover))
                    .tooltip(crate::ui::kit_theme::tip_inside(t(cx, "rooms.dismiss"), &["Esc"])),
                false,
                theme,
            )
            .on_click(|_, window, cx| window.close_dialog(cx))
            .child(ui::icon("x", icon_size::ROW, theme.c("ink-3"))),
        )
}

/// `.hb-room-dialog-actions`: right-aligned, 8 px apart, 16 px below.
fn actions() -> gpui::Div {
    div()
        .flex()
        .justify_end()
        .gap(px(space::MD))
        .mt(px(space::SECTION))
}

/// `p.hb-error[role=alert]`.
fn error_line(text: SharedString, theme: &Theme) -> impl IntoElement {
    ui::text(text, size::SMALL, weight::REGULAR, theme.c("red-ink"))
}

/// A field: 32 px, 14 px text, 12 px inside.
fn styled_input(state: &Entity<InputState>, invalid: bool, theme: &Theme) -> Input {
    Input::new(state)
        .h(px(control::MD))
        .px(px(space::LG))
        .rounded(px(radius::BASE))
        .border_color(if invalid {
            theme.c("red-line")
        } else {
            theme.c("line-strong")
        })
        .bg(theme.c("surface"))
        .text_color(theme.c("ink"))
        .text_size(px(size::UI))
}

// ---------------------------------------------------------------------------
// Create a room
// ---------------------------------------------------------------------------

/// A name, an optional purpose; the same rules the bus applies, said
/// before sending (`CreateRoomDialog`).
struct CreateRoom {
    name: Entity<InputState>,
    purpose: Entity<InputState>,
    /// A submit was tried: an empty name is now a problem worth saying.
    tried: bool,
    busy: bool,
    refused: Option<SharedString>,
    _subs: Vec<Subscription>,
}

impl CreateRoom {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let name = cx.new(|cx| InputState::new(window, cx));
        let placeholder = t(cx, "rooms.fieldPurposePlaceholder");
        let purpose = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(placeholder)
                .validate(|s, _| s.chars().count() <= PURPOSE_MAX)
        });
        let on_input = |this: &mut CreateRoom,
                        _: &Entity<InputState>,
                        ev: &InputEvent,
                        window: &mut Window,
                        cx: &mut Context<CreateRoom>| match ev {
            InputEvent::Change => cx.notify(),
            InputEvent::PressEnter { .. } => this.submit(window, cx),
            _ => {}
        };
        let subs = vec![
            cx.subscribe_in(&name, window, on_input),
            cx.subscribe_in(&purpose, window, on_input),
            cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()),
        ];
        CreateRoom {
            name,
            purpose,
            tried: false,
            busy: false,
            refused: None,
            _subs: subs,
        }
    }

    fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tried = true;
        cx.notify();
        let Some(view) = Desk::get(cx).read(cx).view.clone() else {
            return;
        };
        let raw = self.name.read(cx).value().to_string();
        if self.busy || !room_name_ok(&view, &raw) {
            return;
        }
        self.busy = true;
        self.refused = None;
        let name = normalize_room_name(&raw);
        let purpose = self.purpose.read(cx).value().trim().to_string();
        let mut body = json!({ "action": "create", "name": name });
        if !purpose.is_empty() {
            body["purpose"] = json!(purpose);
        }
        let task = post_channel_action(body, cx);
        cx.spawn_in(window, async move |this, cx| {
            let out = task.await;
            let _ = this.update_in(cx, |this, window, cx| match out {
                Ok(_) => {
                    window.close_dialog(cx);
                    nav::navigate(channel_route(&name), cx);
                }
                Err(e) => {
                    this.busy = false;
                    this.refused = Some(refusal(&e, cx));
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Render for CreateRoom {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let raw = self.name.read(cx).value().to_string();
        let note = Desk::get(cx)
            .read(cx)
            .view
            .clone()
            .map(|view| room_name_note(&view, &raw, self.tried, cx));
        let problem = note.as_ref().is_some_and(|n| n.problem);
        let label = |key: &str| ui::text(t(cx, key), size::UI, weight::STRONG, theme.c("ink"));
        div()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::SM))
                    .child(label("rooms.fieldName"))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap(px(space::MD))
                            .child(ui::icon("hash", icon_size::ROW, theme.c("ink-3")))
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(styled_input(&self.name, problem, &theme)),
                            ),
                    ),
            )
            .children(note.map(|n| {
                ui::text(
                    n.text,
                    size::SMALL,
                    weight::REGULAR,
                    if n.problem {
                        theme.c("red-ink")
                    } else {
                        theme.c("ink-3")
                    },
                )
                .mt(px(space::SM))
            }))
            .child(
                div()
                    .mt(px(space::XL))
                    .flex()
                    .flex_col()
                    .gap(px(space::SM))
                    .child(label("rooms.fieldPurpose"))
                    .child(styled_input(&self.purpose, false, &theme)),
            )
            .children(
                self.refused
                    .clone()
                    .map(|r| div().mt(px(space::LG)).child(error_line(r, &theme))),
            )
            .child(
                actions()
                    .child(
                        button(
                            "hb-room-create-cancel",
                            t(cx, "rooms.cancel"),
                            Tone::Plain,
                            false,
                            &theme,
                        )
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        button(
                            "hb-room-create-submit",
                            if self.busy {
                                t(cx, "rooms.creating")
                            } else {
                                t(cx, "rooms.create")
                            },
                            Tone::Primary,
                            self.busy,
                            &theme,
                        )
                        .when(!self.busy, |b| {
                            b.on_click(cx.listener(|this, _, window, cx| this.submit(window, cx)))
                        }),
                    ),
            )
    }
}

/// Open "Create a room" with the caret in the name.
pub fn open_create_room(window: &mut Window, cx: &mut App) {
    let state = cx.new(|cx| CreateRoom::new(window, cx));
    let name = state.read(cx).name.clone();
    let title = t(cx, "rooms.dialogCreateTitle");
    open_small(
        title,
        state,
        move |window, cx| name.update(cx, |s, cx| s.focus(window, cx)),
        window,
        cx,
    );
}

// ---------------------------------------------------------------------------
// Delete a room
// ---------------------------------------------------------------------------

/// Delete, in the app: what goes, and in the same breath that the history
/// does not (`DeleteRoomDialog`).
struct DeleteRoom {
    room: String,
    busy: bool,
    error: Option<SharedString>,
    /// Cancel has the focus when the dialog opens.
    cancel: FocusHandle,
}

impl DeleteRoom {
    fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let room = self.room.clone();
        let task = post_channel_action(json!({ "action": "delete", "name": room }), cx);
        cx.spawn_in(window, async move |this, cx| {
            let out = task.await;
            let _ = this.update_in(cx, |this, window, cx| match out {
                Ok(_) => {
                    window.close_dialog(cx);
                    deleted(&room, cx);
                }
                Err(e) => {
                    this.busy = false;
                    this.error = Some(refusal(&e, cx));
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

/// After a delete from inside the room, the operator lands on its
/// read-only history: the proof that it was kept.
fn deleted(room: &str, cx: &mut App) {
    let here = matches!(Nav::route(cx), Route::Home { channel: Some(c), .. } if c == room);
    if here {
        nav::replace(
            Route::DeletedRoom {
                name: room.to_string(),
            },
            cx,
        );
    }
}

impl Render for DeleteRoom {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let focus_ring = ui::focus_ring(&theme, false);
        div()
            .flex()
            .flex_col()
            .child(div().child(t(cx, "rooms.deleteBody")))
            .child(
                div()
                    .mt(px(space::LG))
                    .px(px(space::LG))
                    .py(px(space::MD))
                    .rounded(px(radius::MD))
                    .bg(theme.c("ok-bg"))
                    .text_color(theme.c("ok-ink"))
                    .text_size(px(size::SMALL))
                    .line_height(px(leading::SMALL))
                    .child(t(cx, "rooms.deleteKept")),
            )
            .children(
                self.error
                    .clone()
                    .map(|e| div().mt(px(space::MD)).child(error_line(e, &theme))),
            )
            .child(
                actions()
                    .child(
                        button(
                            "hb-room-delete-cancel",
                            t(cx, "rooms.cancel"),
                            Tone::Plain,
                            false,
                            &theme,
                        )
                        .track_focus(&self.cancel)
                        .focus(move |s| s.shadow(vec![focus_ring]))
                        .on_key_down(|ev, window, cx| {
                            if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                                window.close_dialog(cx);
                            }
                        })
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        button_shell("hb-room-delete-confirm", Tone::Danger, self.busy, &theme)
                            .child(ui::icon("trash", icon_size::ROW, theme.c("red-ink")))
                            .child(if self.busy {
                                t(cx, "rooms.deleting")
                            } else {
                                t(cx, "rooms.deleteConfirm")
                            })
                            .when(!self.busy, |b| {
                                b.on_click(
                                    cx.listener(|this, _, window, cx| this.confirm(window, cx)),
                                )
                            }),
                    ),
            )
    }
}

/// Ask before deleting `name` (from the channel menu).
pub fn open_delete_room(name: String, window: &mut Window, cx: &mut App) {
    let title = tv(cx, "rooms.deleteTitle", &[("room", &name)]);
    let state = cx.new(|cx| DeleteRoom {
        room: name,
        busy: false,
        error: None,
        cancel: cx.focus_handle(),
    });
    let cancel = state.read(cx).cancel.clone();
    open_small(
        title,
        state,
        move |window, cx| cancel.focus(window, cx),
        window,
        cx,
    );
}
