//! The Threads screen (`ThreadsScreen`, `home.tsx`): `/threads`.
//!
//! Every message with replies, the one answered last first, and the thread
//! pane on the right when `?thread=` names one. No "New thread" here.

use super::empty_state;
use super::thread_pane::{ClosePane, ThreadPane};
use crate::chat::{self, MessageOpts, ThreadOpener};
use crate::derive::workspace::threads;
use crate::desk::Desk;
use crate::i18n::t;
use crate::model::{Message, WorkspaceView};
use crate::nav::{self, Nav, Route};
use crate::theme::Theme;
use crate::ui::{self, SANS};
use gpui::{
    AppContext, Context, Entity, FontWeight, IntoElement, ListAlignment, ListState, ParentElement,
    Render, Styled, Window, div, list, px,
};
use std::rc::Rc;
use std::sync::Arc;

pub struct ThreadsScreen {
    pane: Entity<ThreadPane>,
    list: ListState,
    /// The parent ids the list shows, in order.
    ids: Vec<String>,
}

impl ThreadsScreen {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe(&Desk::get(cx), |_, _, cx| cx.notify()).detach();
        cx.observe_global::<Nav>(|_, cx| cx.notify()).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        ThreadsScreen {
            pane: cx.new(|cx| ThreadPane::new(window, cx)),
            list: ListState::new(0, ListAlignment::Top, px(800.)),
            ids: Vec::new(),
        }
    }

    /// Bring the list in line with `ids`, splicing only what changed so the
    /// rows on screen keep their place.
    fn sync(&mut self, ids: Vec<String>) {
        if ids == self.ids {
            return;
        }
        let old = &self.ids;
        let prefix = old.iter().zip(&ids).take_while(|(a, b)| a == b).count();
        let room = old.len().min(ids.len()) - prefix;
        let suffix = old
            .iter()
            .rev()
            .zip(ids.iter().rev())
            .take(room)
            .take_while(|(a, b)| a == b)
            .count();
        self.list
            .splice(prefix..old.len() - suffix, ids.len() - prefix - suffix);
        self.ids = ids;
    }
}

/// Where a message of the read sits in `view.messages`.
fn index_of(view: &WorkspaceView, m: &Message) -> usize {
    let base = view.messages.as_ptr() as usize;
    (m as *const Message as usize - base) / std::mem::size_of::<Message>().max(1)
}

impl Render for ThreadsScreen {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let Some(view) = Desk::get(cx).read(cx).view.clone() else {
            return div().flex_1().into_any_element();
        };
        let thread = match Nav::route(cx) {
            Route::Threads { thread } => thread.clone(),
            _ => None,
        };

        let parents = threads(&view);
        let at: Rc<Vec<usize>> = Rc::new(parents.iter().map(|m| index_of(&view, m)).collect());
        self.sync(parents.iter().map(|m| m.id.clone()).collect());

        let opts = Rc::new(MessageOpts {
            selected: thread.clone(),
            on_thread: Some(ThreadOpener::new(|id, _, cx| {
                nav::replace(
                    Route::Threads {
                        thread: Some(id.to_string()),
                    },
                    cx,
                )
            })),
            ..Default::default()
        });

        let head = div()
            .h(px(56.))
            .flex_none()
            .flex()
            .items_center()
            .gap(px(10.))
            .pl(px(20.))
            .pr(px(16.))
            .border_b_1()
            .border_color(theme.c("line"))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(19.))
                    .font_weight(FontWeight::BLACK)
                    .text_color(theme.c("ink"))
                    .child(ui::icon("message-square", 20., theme.c("ink")))
                    .child(t(cx, "home.threads")),
            );

        let body = if parents.is_empty() {
            empty_state(None, t(cx, "home.noThreadsYet"), &theme).into_any_element()
        } else {
            let shared: Arc<WorkspaceView> = view.clone();
            list(self.list.clone(), move |ix, _window, cx| {
                match at.get(ix).and_then(|&i| shared.messages.get(i)) {
                    Some(m) => div()
                        .w_full()
                        .child(chat::message_item(m, &shared, &opts, cx))
                        .into_any_element(),
                    None => div().into_any_element(),
                }
            })
            .size_full()
            .pt(px(8.))
            .pb(px(12.))
            .into_any_element()
        };

        let pane = thread.filter(|t| view.message(t).is_some()).map(|t| {
            let close: ClosePane =
                Rc::new(|_, cx| nav::replace(Route::Threads { thread: None }, cx));
            self.pane.update(cx, |p, cx| p.set(t, None, close, cx));
            self.pane.clone()
        });

        div()
            .flex_1()
            .min_w_0()
            .h_full()
            .flex()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(theme.c("surface"))
                    .font_family(SANS)
                    .text_color(theme.c("ink"))
                    .child(head)
                    .child(div().flex_1().min_h_0().flex().flex_col().child(body)),
            )
            .children(pane)
            .into_any_element()
    }
}
