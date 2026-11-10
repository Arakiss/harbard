//! The thread pane (`ThreadPane`, `home.tsx`): the parent message, its
//! replies and a composer that answers the parent's agent.

use crate::chat::{ComposeTarget, Composer};
use gpui::{App, AppContext, Context, Entity, IntoElement, ParentElement, Render, Styled, Window, div, px};
use std::rc::Rc;

/// Closes the pane (the screen drops `?thread=`).
pub type ClosePane = Rc<dyn Fn(&mut Window, &mut App)>;

pub struct ThreadPane {
    pub parent: Option<String>,
    /// The channel the pane was opened from (its sub-title, "New thread").
    pub channel: Option<String>,
    composer: Entity<Composer>,
    on_close: Option<ClosePane>,
}

impl ThreadPane {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let composer = cx.new(|cx| Composer::new(ComposeTarget::Channel(String::new()), window, cx));
        ThreadPane {
            parent: None,
            channel: None,
            composer,
            on_close: None,
        }
    }

    /// Point the pane at a parent message; call it every render of the screen.
    pub fn set(&mut self, parent: String, channel: Option<String>, on_close: ClosePane, cx: &mut Context<Self>) {
        if self.parent.as_deref() != Some(parent.as_str()) || self.channel != channel {
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
}

impl Render for ThreadPane {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().w(px(400.)).h_full().flex_none().child(self.composer.clone())
    }
}
