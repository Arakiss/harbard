//! The main window's frame (`ShellBody`): a 40px title band across the top,
//! the 78px rail on the left and the rounded panel holding the screen.
//!
//! ```text
//! ┌──────────── topbar 40 (drag; traffic lights at 14,18) ──── update pill ┐
//! │ rail 78 │ panel (margin 0 6 6 0, radius 8)                             │
//! │         │   the screen for the current route, or calm mode           │
//! └─────────┴──────────────────────────────────────────────────────────────┘
//! ```
//!
//! Over it: the season's decorations, the "not responding" banner, the
//! desktop's away suggestion and "While you were away". Menus and dialogs
//! open from the rail ([`rail`]); the Finder (⌘K) from anywhere.
//!
//! What other lanes call: [`open_workspace_menu`] (the Home sidebar's
//! workspace name), [`drag_band`] (every 56px screen header), [`suggest`]
//! (the desktop's away watch), [`open_finder`], and the actions [`ReadAll`]
//! and [`ToggleFinder`] with their key bindings.

pub mod away;
pub mod brand;
pub mod calm;
pub mod decor;
pub mod dialogs;
pub mod emblem;
pub mod finder;
pub mod menu;
pub mod profile;
pub mod rail;
pub mod suggest;
pub mod update;
pub mod user_menu;

use crate::desk::Desk;
use crate::nav::Nav;
use crate::screens::ScreenHost;
use crate::theme::Theme;
use crate::ui;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Global, InteractiveElement, IntoElement, KeyBinding, MouseButton,
    ParentElement, Pixels, Point, Render, Styled, WeakEntity, Window, div, px,
};
use std::cell::Cell;

gpui_kit::actions!(
    harbard,
    [
        /// ⇧Esc: everything read except "Needs you".
        ReadAll,
        /// ⌘K: the Finder.
        ToggleFinder,
    ]
);

pub use suggest::Suggestion;

/// The shell's handles, so other lanes can reach the rail's menus and the
/// frame's overlays without holding them.
pub struct ShellUi {
    pub workspace: WeakEntity<Workspace>,
    pub rail: WeakEntity<rail::Rail>,
    /// The Swamped whose calm mode the operator left (its `since_ms`); kept
    /// for this run of the app as the console kept it for the tab.
    pub calm_exit: Option<u64>,
}

impl Global for ShellUi {}

pub struct Workspace {
    screens: Entity<ScreenHost>,
    rail: Entity<rail::Rail>,
    pill: Entity<update::UpdatePill>,
    decor: Entity<decor::SeasonDecor>,
    calm: Option<Entity<calm::CalmScreen>>,
    away: Entity<away::AwayCard>,
    suggest: Entity<suggest::SuggestCard>,
    /// Holds focus when nothing inside does, so ⌘K and ⇧Esc always reach the shell.
    focus: FocusHandle,
}

impl Workspace {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let desk = Desk::get(cx);
        cx.observe(&desk, |_, _, cx| cx.notify()).detach();
        cx.observe_global::<Nav>(|_, cx| cx.notify()).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        let profile = profile::ProfileStore::get(cx);
        cx.observe(&profile, |_, _, cx| cx.notify()).detach();
        // Back at the window: read the profile again and greet a return from Away.
        cx.observe_window_activation(window, |_, window, cx| {
            if window.is_window_active() {
                profile::ProfileStore::get(cx).update(cx, |p, cx| p.window_activated(cx));
            }
        })
        .detach();
        // "Same as system": follow the Mac's appearance as it changes.
        cx.observe_window_appearance(window, |_, _, cx| crate::theme::reload(cx)).detach();
        let screens = cx.new(|cx| ScreenHost::new(window, cx));
        let rail = cx.new(|cx| rail::Rail::new(window, cx));
        let pill = cx.new(|cx| update::UpdatePill::new(window, cx));
        let decor = cx.new(|cx| decor::SeasonDecor::new(window, cx));
        let away = cx.new(|cx| away::AwayCard::new(window, cx));
        let suggest = cx.new(|cx| suggest::SuggestCard::new(window, cx));
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let me = cx.entity().downgrade();
        cx.set_global(ShellUi {
            workspace: me,
            rail: rail.downgrade(),
            calm_exit: None,
        });
        Workspace {
            screens,
            rail,
            pill,
            decor,
            calm: None,
            away,
            suggest,
            focus,
        }
    }

    /// Calm mode: on in Swamped until "Exit", which holds for this Swamped.
    fn calm_on(cx: &App) -> bool {
        let store = profile::ProfileStore::get(cx).read(cx);
        let state = store.state_id(profile::now_ms());
        let since = store
            .view
            .as_ref()
            .and_then(|v| v.profile.state.as_ref())
            .map_or(0, |s| s.since_ms);
        calm::calm_on(state, since, cx.global::<ShellUi>().calm_exit)
    }

    /// Leave calm mode for this Swamped; the state stays Swamped.
    pub fn exit_calm(cx: &mut App) {
        let since = profile::ProfileStore::get(cx)
            .read(cx)
            .view
            .as_ref()
            .and_then(|v| v.profile.state.as_ref())
            .map_or(0, |s| s.since_ms);
        cx.global_mut::<ShellUi>().calm_exit = Some(since);
        cx.refresh_windows();
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>().clone();
        let desk = Desk::get(cx).read(cx);
        let stale = desk.view.is_some() && desk.error.is_some();
        let calm = Self::calm_on(cx);
        if calm && self.calm.is_none() {
            self.calm = Some(cx.new(|cx| calm::CalmScreen::new(window, cx)));
        } else if !calm {
            self.calm = None;
        }
        let panel: gpui::AnyView = match &self.calm {
            Some(c) => c.clone().into(),
            None => self.screens.clone().into(),
        };
        div()
            .id("hb-shell")
            .key_context("Harbard")
            .track_focus(&self.focus)
            .on_action(|_: &ReadAll, _, cx| read_all(cx))
            .on_action(|_: &ToggleFinder, window, cx| finder::toggle(window, cx))
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme.c("frame"))
            .font_family(ui::SANS)
            .text_size(px(ui::size::BODY))
            .line_height(gpui::relative(1.46))
            .text_color(theme.c("ink"))
            // Snow under the rail's buttons and the pill; the lights go over everything.
            .when(theme.decorations, |d| d.child(self.decor.read(cx).snow()))
            .child(
                drag_band()
                    .h(px(40.))
                    .w_full()
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_end()
                    .pl(px(78.))
                    .child(self.pill.clone()),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.rail.clone())
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .mr(px(6.))
                            .mb(px(6.))
                            .rounded(px(8.))
                            .overflow_hidden()
                            .bg(theme.c("surface"))
                            .border_1()
                            .border_color(theme.c("panel-ring"))
                            .child(panel),
                    ),
            )
            .when(theme.decorations, |d| d.child(self.decor.clone()))
            .when(stale, |d| {
                d.child(
                    div()
                        .absolute()
                        .right(px(20.))
                        .bottom(px(20.))
                        .px(px(14.))
                        .py(px(10.))
                        .rounded(px(8.))
                        .bg(theme.c("red-bg"))
                        .border_1()
                        .border_color(theme.c("red-line"))
                        .text_color(theme.c("banner-ink"))
                        .text_size(px(ui::size::SMALL))
                        .child(crate::i18n::t(cx, "shell.harbardNotResponding")),
                )
            })
            .child(self.suggest.clone())
            .child(self.away.clone())
    }
}

/// `POST read-all`: everything read except "Needs you" (⇧Esc, the menu).
pub fn read_all(cx: &mut App) {
    let desk = Desk::get(cx);
    desk.update(cx, |d, cx| d.post("read-all", serde_json::json!({}), cx)).detach();
}

/// Open the workspace menu under the Home sidebar's workspace name
/// (`openWorkspaceMenu`). With several workspaces it is the only way in.
pub fn open_workspace_menu(window: &mut Window, cx: &mut App) {
    if let Some(rail) = cx.global::<ShellUi>().rail.upgrade() {
        // Under the sidebar head: rail 78 + 10, title band 40 + head 56 - 8 + 4.
        rail.update(cx, |r, cx| r.open_ws_menu(Some(gpui::point(px(88.), px(92.))), window, cx));
    }
}

/// Toggle the ⌘K Finder.
pub fn open_finder(window: &mut Window, cx: &mut App) {
    finder::toggle(window, cx);
}

/// The desktop's away watch suggests a state; only the operator's click
/// changes it (`harbard:suggest`).
pub fn suggest(s: Suggestion, cx: &mut App) {
    suggest::show(s, cx);
}

thread_local! {
    /// Where a press on a drag zone began; a move past a few pixels drags.
    static DRAG_FROM: Cell<Option<Point<Pixels>>> = const { Cell::new(None) };
}

/// A zone that drags the window, as the console's title band, rail and 56px
/// screen headers did (`desktop-drag.ts`); a double click zooms. A press
/// drags only once the pointer moves 3px, so a click inside still clicks.
/// Give buttons inside it `.occlude()` so a press on them never arms a drag.
pub fn drag_band() -> gpui::Div {
    div()
        .on_mouse_down(MouseButton::Left, |ev, window, _| {
            if ev.click_count == 2 {
                DRAG_FROM.set(None);
                window.titlebar_double_click();
            } else {
                DRAG_FROM.set(Some(ev.position));
            }
        })
        .on_mouse_up(MouseButton::Left, |_, _, _| DRAG_FROM.set(None))
        .on_mouse_up_out(MouseButton::Left, |_, _, _| DRAG_FROM.set(None))
        .on_mouse_move(|ev, window, _| {
            let Some(from) = DRAG_FROM.get() else { return };
            if ev.pressed_button != Some(MouseButton::Left) {
                DRAG_FROM.set(None);
                return;
            }
            let d = ev.position - from;
            if d.x.abs() + d.y.abs() >= px(3.) {
                DRAG_FROM.set(None);
                window.start_window_move();
            }
        })
}

/// serve's theme read is cheap and in-process; the console looked again every
/// 1.5 s. A change of theme, mode, season or decorations (from Preferences,
/// another window or a theme file) repaints the app.
const THEME_POLL: std::time::Duration = std::time::Duration::from_millis(1_500);

fn theme_key(state: &serde_json::Value) -> String {
    format!(
        "{}|{}|{}|{}|{}",
        state["revision"], state["mode"], state["theme"], state["season"]["active"], state["season"]["decorations"]
    )
}

fn watch_theme(cx: &mut App) {
    let mut last = theme_key(&cx.global::<Theme>().state);
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(THEME_POLL).await;
            let state = cx.background_executor().spawn(async { crate::theme::read_state() }).await;
            let key = theme_key(&state);
            if key != last {
                last = key;
                cx.update(crate::theme::reload);
            }
        }
    })
    .detach();
}

pub fn init(cx: &mut App) {
    watch_theme(cx);
    profile::init(cx);
    update::init(cx);
    cx.bind_keys([
        KeyBinding::new("shift-escape", ReadAll, Some("Harbard")),
        KeyBinding::new("cmd-k", ToggleFinder, Some("Harbard")),
        KeyBinding::new("ctrl-k", ToggleFinder, Some("Harbard")),
    ]);
}
