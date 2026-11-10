//! The thread pane (`ThreadPane`, `home.tsx`): the parent message, its
//! replies and a composer that answers the parent's agent.

use crate::chat::{self, ComposeTarget, Composer, ComposerState, MessageOpts};
use crate::derive::composer::{composer_block, composer_confirm, composer_notice, dm_hint};
use crate::derive::messages::note_reader;
use crate::derive::workspace::thread;
use crate::desk::Desk;
use crate::i18n::{t, tv};
use crate::theme::Theme;
use crate::ui::{self, SANS};
use gpui::{
    App, AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Window, div, point, px,
};
use std::rc::Rc;

/// Closes the pane (the screen drops `?thread=`).
pub type ClosePane = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct ThreadPane {
    pub parent: Option<String>,
    /// The channel the pane was opened from (its sub-title, "New thread").
    pub channel: Option<String>,
    composer: Entity<Composer>,
    on_close: Option<ClosePane>,
    scroll: ScrollHandle,
    /// The newest reply drawn last time: a new one scrolls it into view.
    last_reply: Option<String>,
}

impl ThreadPane {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        let composer =
            cx.new(|cx| Composer::new(ComposeTarget::Channel(String::new()), window, cx));
        ThreadPane {
            parent: None,
            channel: None,
            composer,
            on_close: None,
            scroll: ScrollHandle::new(),
            last_reply: None,
        }
    }

    /// Point the pane at a parent message; call it every render of the screen.
    pub fn set(
        &mut self,
        parent: String,
        channel: Option<String>,
        on_close: ClosePane,
        cx: &mut Context<Self>,
    ) {
        if self.parent.as_deref() != Some(parent.as_str()) || self.channel != channel {
            if self.parent.as_deref() != Some(parent.as_str()) {
                // Another thread opens at its parent.
                self.scroll.set_offset(point(px(0.), px(0.)));
                self.last_reply = None;
            }
            self.parent = Some(parent);
            self.channel = channel;
            cx.notify();
        }
        self.on_close = Some(on_close);
    }

    /// The composer, so the channel screen can hand "New thread" drafts over.
    pub fn composer(&self) -> &Entity<Composer> {
        &self.composer
    }

    /// Offer "New thread" in the pane's composer (channel context only):
    /// `f` gets the draft.
    pub fn on_new_thread(
        &mut self,
        f: impl Fn(String, &mut Window, &mut App) + 'static,
        cx: &mut Context<Self>,
    ) {
        self.composer.update(cx, |c, _| c.on_new_thread(f));
    }
}

impl Render for ThreadPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let view = Desk::get(cx).read(cx).view.clone();
        let (Some(view), Some(id)) = (view, self.parent.clone()) else {
            return div().into_any_element();
        };
        // Renders nothing when the parent is unknown.
        let Some(parent) = view.message(&id) else {
            return div().into_any_element();
        };
        let replies = thread(&view, &id);

        let newest = replies.last().map(|m| m.id.clone());
        if newest != self.last_reply {
            if self.last_reply.is_some() {
                self.scroll.scroll_to_bottom();
            }
            self.last_reply = newest;
        }

        let sub = self
            .channel
            .clone()
            .or_else(|| parent.channels.first().cloned());
        let opts = MessageOpts {
            in_thread: true,
            ..Default::default()
        };
        let divider = if replies.len() == 1 {
            t(cx, "home.oneReply")
        } else {
            tv(cx, "home.replies", &[("count", &replies.len().to_string())])
        };

        // Answer the parent's agent, when it is in the view.
        let agent = view.agent(&parent.agent);
        if let Some(agent) = agent {
            let target = ComposeTarget::Thread {
                parent: id.clone(),
                channel: None,
                agent: Some(agent.key.clone()),
            };
            let reader = note_reader(&view.agents, &agent.key);
            let state = ComposerState {
                blocked: composer_block(&[agent], cx).map(Into::into),
                notice: composer_notice(&[agent], cx).map(Into::into),
                confirm: composer_confirm(&[agent], cx).map(Into::into),
                hint: Some(dm_hint(agent, reader.as_deref(), cx).into()),
                boxed: false,
            };
            let placeholder = t(cx, "home.replyPlaceholder");
            self.composer.update(cx, |c, cx| {
                c.set_target(target, placeholder, cx);
                c.set_state(state, cx);
            });
        }

        let on_close = self.on_close.clone();
        let head = div()
            .h(px(56.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(8.))
            .pl(px(18.))
            .pr(px(12.))
            .border_b_1()
            .border_color(theme.c("line"))
            .child(
                div()
                    .text_size(px(ui::size::TITLE))
                    .font_weight(FontWeight::BLACK)
                    .text_color(theme.c("ink"))
                    .child(t(cx, "home.thread")),
            )
            .children(sub.map(|c| {
                div()
                    .text_size(px(ui::size::SMALL))
                    .text_color(theme.c("ink-3"))
                    .child(SharedString::from(format!("# {c}")))
            }))
            .child(
                div()
                    .id("hb-thread-close")
                    .ml_auto()
                    .size(px(32.))
                    .flex_none()
                    .rounded(px(6.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .cursor_pointer()
                    .hover(|s| s.bg(theme.c("control")))
                    .child(ui::icon("x", 20., theme.c("ink-3")))
                    .on_click(move |_, window, cx| {
                        if let Some(close) = &on_close {
                            close(window, cx);
                        }
                    }),
            );

        let mut list = div()
            .id("hb-thread-stream")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .track_scroll(&self.scroll)
            .pt(px(8.))
            .pb(px(12.))
            .flex()
            .flex_col()
            .child(chat::message_item(parent, &view, &opts, cx))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(10.))
                    .px(px(20.))
                    .py(px(4.))
                    .text_size(px(ui::size::SMALL))
                    .text_color(theme.c("ink-3"))
                    .child(divider)
                    .child(div().flex_1().h(px(1.)).bg(theme.c("line"))),
            );
        for reply in &replies {
            list = list.child(chat::message_item(reply, &view, &opts, cx));
        }

        div()
            .w(px(400.))
            .h_full()
            .flex_none()
            .flex()
            .flex_col()
            .min_h_0()
            .border_l_1()
            .border_color(theme.c("line"))
            .bg(theme.c("surface"))
            .font_family(SANS)
            .text_color(theme.c("ink"))
            .child(head)
            // Clipped, so the first message's hover tools never cover the head.
            .child(div().flex_1().min_h_0().flex().flex_col().overflow_hidden().child(list))
            .children(agent.is_some().then(|| self.composer.clone()))
            .into_any_element()
    }
}
