//! A room's screen (`RoomView`): header, archived strip, feed and composer,
//! or the `#feedback` inbox. The channel screen around it marks the room
//! read and handles Esc.

use super::feed::RoomFeed;
use super::inbox::FeedbackInbox;
use super::{archived_strip, channel_menu};
use crate::chat::{ComposeTarget, Composer, ComposerState};
use crate::derive::rooms::{FEEDBACK, room_composer_notice};
use crate::derive::workspace::find_channel;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::screens::home::header::room_header;
use crate::theme::Theme;
use crate::ui;
use gpui::{
    AppContext as _, Context, Entity, IntoElement, ParentElement, Render, Styled, Subscription,
    Window, div, px,
};

pub struct RoomView {
    pub room: String,
    feed: Entity<RoomFeed>,
    composer: Entity<Composer>,
    /// Made the first time `#feedback` shows.
    inbox: Option<Entity<FeedbackInbox>>,
    _subs: Vec<Subscription>,
}

impl RoomView {
    pub fn new(room: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let empty = tv(cx, "rooms.emptyRoom", &[("room", &room)]);
        let feed = cx.new(|cx| RoomFeed::new(room.clone(), false, empty, cx));
        let composer = cx.new(|cx| Composer::new(ComposeTarget::Channel(room.clone()), window, cx));
        let subs = vec![cx.observe(&Desk::get(cx), |this, _, cx| {
            this.sync(cx);
            cx.notify();
        })];
        let mut view = RoomView {
            room,
            feed,
            composer,
            inbox: None,
            _subs: subs,
        };
        view.sync(cx);
        view
    }

    /// Show another room (the channel screen keeps one RoomView).
    pub fn set_room(&mut self, room: String, cx: &mut Context<Self>) {
        if self.room != room {
            self.room = room;
            self.sync(cx);
            cx.notify();
        }
    }

    /// Point the feed and the composer at the room, with what the composer
    /// says above the box (`roomComposerNotice`; never blocked).
    fn sync(&mut self, cx: &mut Context<Self>) {
        let room = self.room.clone();
        let empty = tv(cx, "rooms.emptyRoom", &[("room", &room)]);
        self.feed
            .update(cx, |f, cx| f.set_room(room.clone(), empty, cx));
        let notice = Desk::get(cx).read(cx).view.clone().and_then(|view| {
            find_channel(&view, &room).and_then(|ch| room_composer_notice(&ch, cx))
        });
        let placeholder = tv(cx, "rooms.writeIn", &[("room", &room)]);
        self.composer.update(cx, |c, cx| {
            c.set_target(ComposeTarget::Channel(room.clone()), placeholder, cx);
            c.set_state(
                ComposerState {
                    notice,
                    ..Default::default()
                },
                cx,
            );
        });
    }
}

impl Render for RoomView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let main = div()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .flex()
            .flex_col()
            .bg(theme.c("surface"));
        let Some(view) = Desk::get(cx).read(cx).view.clone() else {
            return main.into_any_element();
        };
        let Some(channel) = find_channel(&view, &self.room).map(|c| c.into_owned()) else {
            return main.into_any_element();
        };
        let header = room_header(&view, &channel, channel_menu(&channel, cx), cx);
        let strip = channel.archived.then(|| archived_strip(&channel, cx));
        if channel.id == FEEDBACK {
            let inbox = self
                .inbox
                .get_or_insert_with(|| cx.new(|cx| FeedbackInbox::new(window, cx)))
                .clone();
            return main
                .child(header)
                .children(strip)
                .child(inbox)
                .into_any_element();
        }
        let foot = if channel.archived {
            // div.hb-composer > div.hb-composer-none[role=note]
            div()
                .flex_none()
                .px(px(20.))
                .pb(px(18.))
                .child(
                    div()
                        .px(px(16.))
                        .py(px(14.))
                        .border_1()
                        .border_dashed()
                        .border_color(theme.c("line-strong"))
                        .rounded(px(10.))
                        .bg(theme.c("surface-2"))
                        .text_color(theme.c("ink-3"))
                        .font_family(ui::SANS)
                        .text_size(px(14.))
                        .child(t(cx, "rooms.archivedReadOnly")),
                )
                .into_any_element()
        } else {
            self.composer.clone().into_any_element()
        };
        main.child(header)
            .children(strip)
            .child(self.feed.clone())
            .child(foot)
            .into_any_element()
    }
}
