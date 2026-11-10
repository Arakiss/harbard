//! The channel screen (`ChannelScreen`, `home.tsx`): `/` and `/c/<id>`.
//!
//! A project channel shows its header, strips, the work panel, the stream
//! and the composer, with the thread pane on the right when `?thread=`
//! names a message. A room hands the whole column to [`RoomView`].
//! Opening a channel with unread messages marks it read; Esc does it again.

use super::thread_pane::{ClosePane, ThreadPane};
use super::{empty_state, header, work_panel};
use crate::chat::{self, ComposeTarget, Composer, ComposerState, MessageOpts, ThreadOpener};
use crate::derive::composer::channel_composer;
use crate::derive::work_items::{about_work, work_items};
use crate::derive::workspace::{channel_stream, default_channel_id, find_channel};
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::model::Message;
use crate::nav::{self, Nav, Route};
use crate::screens::rooms::{self, RoomView};
use crate::theme::Theme;
use crate::ui::SANS;
use gpui::{
    AnyWindowHandle, App, AppContext, Context, Entity, IntoElement, ParentElement, Render,
    SharedString, Styled, Subscription, WeakEntity, Window, div,
};
use gpui_kit::component::WindowExt as _;
use serde_json::json;
use std::rc::Rc;

pub struct ChannelScreen {
    /// The channel's composer; kept across channels (its target follows).
    composer: Entity<Composer>,
    pane: Entity<ThreadPane>,
    /// One room view, pointed at whichever room is open.
    room: Option<Entity<RoomView>>,
    /// The channel on screen, for Esc.
    current: Option<String>,
    /// `"{id}:{unread}"` already marked read on open.
    seen_unread: String,
    _subs: Vec<Subscription>,
}

impl ChannelScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()).detach();
        cx.observe_global::<Nav>(|_, cx| cx.notify()).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        let composer =
            cx.new(|cx| Composer::new(ComposeTarget::Channel(String::new()), window, cx));
        let pane = cx.new(|cx| ThreadPane::new(window, cx));

        // "New thread" in the pane: close it and carry the draft into the
        // channel's own composer as a new top-level message.
        let main = composer.clone();
        let reply = pane.read(cx).composer().downgrade();
        pane.update(cx, |p, cx| {
            p.on_new_thread(
                move |draft, window, cx| {
                    close_thread(cx);
                    main.update(cx, |c, cx| {
                        c.set_draft(draft, window, cx);
                        c.focus(window, cx);
                    });
                    // The pane's box starts empty next time, as a fresh one did.
                    let reply = reply.clone();
                    window.defer(cx, move |window, cx| {
                        let _ = reply.update(cx, |c, cx| c.set_draft("", window, cx));
                    });
                },
                cx,
            )
        });

        let me = cx.entity().downgrade();
        let handle = window.window_handle();
        let esc =
            cx.intercept_keystrokes(move |ev, window, cx| on_escape(&me, handle, ev, window, cx));

        ChannelScreen {
            composer,
            pane,
            room: None,
            current: None,
            seen_unread: String::new(),
            _subs: vec![esc],
        }
    }
}

/// `POST read-channel`.
fn read_channel(id: String, cx: &mut App) {
    Desk::get(cx)
        .update(cx, |d, cx| {
            d.post("read-channel", json!({ "channel": id }), cx)
        })
        .detach();
}

/// The window's Esc marks the channel read (`useEscRead`): not with Shift,
/// nor while a lightbox, the report pane, a header popover or a dialog
/// is open (they take that Esc).
fn on_escape(
    me: &WeakEntity<ChannelScreen>,
    handle: AnyWindowHandle,
    ev: &gpui::KeystrokeEvent,
    window: &mut Window,
    cx: &mut App,
) {
    let key = &ev.keystroke;
    if key.key != "escape" || key.modifiers.shift || window.window_handle() != handle {
        return;
    }
    if !matches!(Nav::route(cx), Route::Home { .. })
        || chat::attachments::overlay_open(cx)
        || header::popover_open(cx)
        || window.has_active_dialog(cx)
    {
        return;
    }
    let Some(screen) = me.upgrade() else { return };
    let screen = screen.read(cx);
    let composers = [screen.composer.clone(), screen.pane.read(cx).composer().clone()];
    if composers.iter().any(|c| c.read(cx).takes_escape(cx)) {
        return;
    }
    if let Some(id) = screen.current.clone() {
        read_channel(id, cx);
    }
}

/// The route without `?thread=` (keeps the channel and the work item).
fn close_thread(cx: &mut App) {
    if let Route::Home { channel, item, .. } = Nav::route(cx).clone() {
        nav::replace(
            Route::Home {
                channel,
                thread: None,
                item,
            },
            cx,
        );
    }
}

impl Render for ChannelScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let (view, now) = {
            let desk = Desk::get(cx).read(cx);
            (desk.view.clone(), desk.now_ms())
        };
        let Some(view) = view else {
            return div().flex_1().into_any_element();
        };
        let (route_channel, thread, item) = match Nav::route(cx).clone() {
            Route::Home {
                channel,
                thread,
                item,
            } => (channel, thread, item),
            _ => (None, None, None),
        };
        let id = route_channel
            .clone()
            .or_else(|| default_channel_id(&view).map(String::from));
        let main = || {
            div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .flex_col()
                .bg(theme.c("surface"))
                .font_family(SANS)
                .text_color(theme.c("ink"))
        };

        let Some(ch) = id.as_deref().and_then(|id| find_channel(&view, id)) else {
            self.current = None;
            let title = match &id {
                Some(id) => tv(cx, "home.noSuchChannel", &[("id", id)]),
                None => t(cx, "home.noChannelsYet"),
            };
            let mut body = t(cx, "home.channelAppearsWhen").to_string();
            if !view.team_root_found {
                body.push(' ');
                body.push_str(&t(cx, "home.claudeProjectsMissing"));
            }
            return main()
                .child(empty_state(Some(title), SharedString::from(body), &theme))
                .into_any_element();
        };
        self.current = Some(ch.id.clone());

        // Open is read, as in a chat app; Esc does it again on demand.
        let seen = format!("{}:{}", ch.id, ch.unread);
        if ch.unread > 0 && self.seen_unread != seen {
            self.seen_unread = seen;
            let id = ch.id.clone();
            cx.defer(move |cx| read_channel(id, cx));
        }

        if ch.is_room() {
            let room = match &self.room {
                Some(room) => {
                    room.update(cx, |r, cx| r.set_room(ch.id.clone(), cx));
                    room.clone()
                }
                None => {
                    let name = ch.id.clone();
                    let room = cx.new(|cx| RoomView::new(name, window, cx));
                    self.room = Some(room.clone());
                    room
                }
            };
            return div()
                .flex_1()
                .min_w_0()
                .h_full()
                .flex()
                .child(room)
                .into_any_element();
        }

        let whole = channel_stream(&view, &ch.id);
        let items = work_items(&view, &ch.id, &whole, now);
        let chosen = item
            .as_deref()
            .and_then(|k| items.iter().find(|i| i.key == k));
        let shown: Vec<&Message> = match chosen {
            Some(c) => whole.iter().copied().filter(|m| about_work(m, c)).collect(),
            None => whole.clone(),
        };

        let select: work_panel::SelectWork = {
            let (channel, thread) = (route_channel.clone(), thread.clone());
            Rc::new(move |key, _, cx| {
                nav::replace(
                    Route::Home {
                        channel: channel.clone(),
                        thread: thread.clone(),
                        item: key,
                    },
                    cx,
                )
            })
        };
        let open_thread = {
            let (channel, item) = (route_channel.clone(), item.clone());
            ThreadOpener::new(move |id, _, cx| {
                nav::replace(
                    Route::Home {
                        channel: channel.clone(),
                        thread: Some(id.to_string()),
                        item: item.clone(),
                    },
                    cx,
                )
            })
        };
        let opts = MessageOpts {
            fold: true,
            fold_internal: chosen.is_none(),
            selected: thread.clone(),
            on_thread: Some(open_thread),
            empty: Some(tv(cx, "home.nobodyWroteYet", &[("channel", &ch.id)])),
            key: Some(format!("channel-{}", ch.id).into()),
            ..Default::default()
        };

        let head = header::channel_header(&view, &ch, rooms::channel_menu(&ch, cx), cx);
        let archived = ch.archived.then(|| rooms::archived_strip(&ch, cx));
        let warnings = header::warning_strips(&view, &ch, cx);
        let work =
            work_panel::work_panel(&view, &items, chosen.map(|c| c.key.as_str()), select, cx);
        let stream = chat::stream(&shown, &view, &opts, cx);

        let target = ComposeTarget::Channel(ch.id.clone());
        let placeholder = tv(cx, "home.writeInChannel", &[("channel", &ch.id)]);
        let state = ComposerState::from_gate(channel_composer(&ch, cx));
        self.composer.update(cx, |c, cx| {
            c.set_target(target, placeholder, cx);
            c.set_state(state, cx);
        });

        let pane = thread.filter(|t| view.message(t).is_some()).map(|t| {
            let close: ClosePane = Rc::new(|_, cx| close_thread(cx));
            let channel = Some(ch.id.clone());
            self.pane.update(cx, |p, cx| p.set(t, channel, close, cx));
            self.pane.clone()
        });

        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .child(
                main()
                    .child(head)
                    .children(archived)
                    .children(warnings)
                    .children(work)
                    .child(div().flex_1().min_h_0().flex().flex_col().child(stream))
                    .child(self.composer.clone()),
            )
            .children(pane)
            .into_any_element()
    }
}
