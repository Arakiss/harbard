//! A temporary way to look at one view full-window before conversations
//! embed them: `HARBARD_APP_HUGINN=<ulid>/<name>` (or just the ulid) opens a
//! second window with the view of the message that carries that attachment.

use super::HuginnView;
use crate::desk::Desk;
use crate::model::{Attachment, Message};
use crate::theme::Theme;
use gpui::{
    App, AppContext, Bounds, Context, Entity, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, WindowBounds, WindowOptions, div, px, size,
};

pub fn open_debug_window(cx: &mut App) {
    let Ok(wanted) = std::env::var("HARBARD_APP_HUGINN") else { return };
    if wanted.trim().is_empty() {
        return;
    }
    let bounds = Bounds::centered(None, size(px(880.), px(860.)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        ..Default::default()
    };
    gpui_kit::open_window(options, cx, |window, cx| cx.new(|cx| DebugView::new(wanted.trim().to_string(), window, cx))).ok();
}

struct DebugView {
    wanted: String,
    view: Option<Entity<HuginnView>>,
}

impl DebugView {
    fn new(wanted: String, window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_in(&Desk::get(cx), window, |this, _, window, cx| this.find(window, cx)).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        let mut me = DebugView { wanted, view: None };
        me.find(window, cx);
        me
    }

    fn find(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.view.is_some() {
            return;
        }
        let found: Option<(Attachment, Message)> = {
            let desk = Desk::get(cx).read(cx);
            desk.view.as_ref().and_then(|v| {
                v.messages.iter().find_map(|m| {
                    m.attachments
                        .iter()
                        .find(|a| a.kind == "huginn" && (a.id == self.wanted || a.id.starts_with(&format!("{}/", self.wanted))))
                        .map(|a| (a.clone(), m.clone()))
                })
            })
        };
        if let Some((attachment, message)) = found {
            self.view = Some(cx.new(|cx| HuginnView::new(attachment, message, window, cx)));
            cx.notify();
        }
    }
}

impl Render for DebugView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        div()
            .id("huginn-debug")
            .size_full()
            .overflow_y_scroll()
            .bg(theme.c("list"))
            .text_color(theme.c("ink"))
            .font_family(crate::ui::SANS)
            .pt(px(40.))
            .px(px(20.))
            .pb(px(20.))
            .children(self.view.clone())
            .when_none(&self.view, |d| d.child(format!("waiting for a message carrying {}", self.wanted)))
    }
}

use gpui::prelude::FluentBuilder as _;
