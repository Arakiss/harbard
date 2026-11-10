//! One screen per route kind. The host keeps the current screen's entity and
//! builds a new one when the kind changes; a screen reads its parameters from
//! `Nav::route(cx)` in `render` and re-renders when they change.
//!
//! To add a screen: write `screens/<name>.rs` with a `pub struct` that
//! implements `Render` and a `pub fn new(window, cx) -> Self`, then add one
//! arm to [`ScreenHost::build`].

pub mod activity;
pub mod direct;
pub mod inbox;
pub mod later;
pub mod live;
pub mod search;

use crate::nav::{Nav, Route};
use crate::theme::Theme;
use crate::ui;
use gpui::{AnyView, AppContext, Context, IntoElement, ParentElement, Render, Styled, Window, div};

pub struct ScreenHost {
    kind: &'static str,
    view: Option<AnyView>,
}

impl ScreenHost {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<Nav>(|_, cx| cx.notify()).detach();
        ScreenHost { kind: "", view: None }
    }

    fn build(route: &Route, _window: &mut Window, cx: &mut Context<Self>) -> AnyView {
        let kind = route.kind();
        match kind {
            "direct" => cx.new(|cx| direct::DirectsScreen::new(_window, cx)).into(),
            "activity" => cx.new(|cx| activity::ActivityScreen::new(_window, cx)).into(),
            "later" => cx.new(|cx| later::LaterScreen::new(_window, cx)).into(),
            "search" => cx.new(|cx| search::SearchScreen::new(_window, cx)).into(),
            _ => cx.new(|_| Placeholder { kind }).into(),
        }
    }
}

impl Render for ScreenHost {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let route = cx.global::<Nav>().route.clone();
        if self.view.is_none() || self.kind != route.kind() {
            self.kind = route.kind();
            self.view = Some(Self::build(&route, window, cx));
        }
        div().size_full().flex().children(self.view.clone())
    }
}

/// Stands in for a screen not ported yet.
struct Placeholder {
    kind: &'static str,
}

impl Render for Placeholder {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let desk = crate::desk::Desk::get(cx).read(cx);
        let stats = match &desk.view {
            Some(v) => format!(
                "{} channels · {} agents · {} messages · read in {} ms · operator {}",
                v.channels.len(),
                v.agents.len(),
                v.messages.len(),
                desk.took.as_millis(),
                v.operator_initials
            ),
            None => format!("no read yet{}", desk.error.as_ref().map(|e| format!(": {e}")).unwrap_or_default()),
        };
        let theme = cx.global::<Theme>();
        ui::empty(format!("{} — native screen coming · {stats}", self.kind), theme)
    }
}
