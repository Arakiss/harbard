//! One screen per route kind. The host keeps the current screen's entity and
//! builds a new one when the kind changes; a screen reads its parameters from
//! `Nav::route(cx)` in `render` and re-renders when they change.
//!
//! To add a screen: write `screens/<name>.rs` with a `pub struct` that
//! implements `Render` and a `pub fn new(window, cx) -> Self`, then add one
//! arm to [`ScreenHost::build`].

mod deleted_room;
mod deliveries;
mod history;
mod lobby;
mod retro;
mod room_feed;
pub mod secondary;
mod setup;
pub mod activity;
pub mod direct;
pub mod inbox;
pub mod later;
pub mod live;
pub mod search;
pub mod home;
pub mod rooms;

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

    fn build(route: &Route, window: &mut Window, cx: &mut Context<Self>) -> AnyView {
        match route {
            Route::Home { .. } => {
                let screen: AnyView = cx.new(|cx| home::channel::ChannelScreen::new(window, cx)).into();
                cx.new(|cx| home::HomeShell::new(screen, window, cx)).into()
            }
            Route::Threads { .. } => {
                let screen: AnyView = cx.new(|cx| home::threads::ThreadsScreen::new(window, cx)).into();
                cx.new(|cx| home::HomeShell::new(screen, window, cx)).into()
            }
            // Lobby and Deliveries sit beside Home's sidebar too (`HomeShell`).
            Route::Lobby | Route::Deliveries => {
                let screen: AnyView = match route {
                    Route::Lobby => cx.new(|cx| lobby::LobbyScreen::new(window, cx)).into(),
                    _ => cx.new(|cx| deliveries::DeliveriesScreen::new(window, cx)).into(),
                };
                cx.new(|cx| home::HomeShell::new(screen, window, cx)).into()
            }
            Route::History { .. } => cx.new(|cx| history::HistoryScreen::new(window, cx)).into(),
            Route::DeletedRoom { .. } => cx.new(|cx| deleted_room::DeletedRoomScreen::new(window, cx)).into(),
            Route::Retro => cx.new(|cx| retro::RetroScreen::new(window, cx)).into(),
            Route::Setup => cx.new(|cx| setup::SetupScreen::new(window, cx)).into(),
            Route::Direct { .. } => cx.new(|cx| direct::DirectsScreen::new(window, cx)).into(),
            Route::Activity { .. } => cx.new(|cx| activity::ActivityScreen::new(window, cx)).into(),
            Route::Later => cx.new(|cx| later::LaterScreen::new(window, cx)).into(),
            Route::Search { .. } => cx.new(|cx| search::SearchScreen::new(window, cx)).into(),
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
