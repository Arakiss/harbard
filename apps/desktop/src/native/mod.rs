//! What the app does as a Mac app, beyond its windows' content: the app
//! menu, close-to-hide, the Dock tile and badge, the menu-bar item, the
//! summon shortcut, notifications, the away suggestion, one instance at a
//! time, and the Strip window's place on screen.
//!
//! Everything that reaches outside the app (single instance, tray, global
//! shortcut, Dock tile, OS notifications) passes [`gate`] first: it is live
//! only inside a `.app` bundle or when `HARBARD_APP_NATIVE` asks for it.
//!
//! Other lanes call: [`open_strip`], [`open_main`], [`show_main`],
//! [`set_mute`] / [`muted`], [`strip_mode`], [`logo::logo_state`].

pub mod appkit;
pub mod away;
pub mod dock;
pub mod doorbell;
pub mod gate;
pub mod logo;
pub mod rules;
pub mod single;
pub mod tray;
pub mod watch;

/// Tests that start a process take this, and so do the socket tests: a fork
/// holds every open descriptor until its exec, so a closed listener would
/// still accept a connection for that moment.
#[cfg(test)]
pub static FORK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

use crate::i18n::{I18n, tv};
use crate::nav::{self, Route};
use gate::{Feature, allows};
use gpui::{
    AnyWindowHandle, App, Application, Global, KeyBinding, Menu, MenuItem, OsAction, actions,
};
use std::path::PathBuf;

actions!(
    harbard_native,
    [
        About,
        CheckForUpdates,
        OpenOverview,
        ShowStrip,
        CloseStrip,
        CloseWindow,
        Quit,
        Hide,
        HideOthers,
        ShowAll,
        Minimize,
        Zoom,
        /// ⌥⌘P: the performance overlay.
        TogglePerformance,
    ]
);

/// What the menu-bar item, the shortcut and a second launch ask for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cmd {
    OpenOverview,
    ToggleMain,
    ToggleMute,
    ShowStrip,
    CloseStrip,
    Updates,
    Quit,
}

/// Where the Strip lives this run: `Wide` on a 2560×720 display (shown at
/// launch, borderless), `Window` as an ordinary window opened on demand.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StripMode {
    Wide,
    Window,
}

impl StripMode {
    /// Preferences' words for it (`desk_strip_mode`).
    pub fn as_str(self) -> &'static str {
        match self {
            StripMode::Wide => "wide",
            StripMode::Window => "window",
        }
    }
}

/// The app's native state: its two windows, the tray and the shortcut.
pub struct Native {
    pub main: Option<AnyWindowHandle>,
    pub strip: Option<AnyWindowHandle>,
    pub strip_mode: StripMode,
    tray: Option<tray::Tray>,
    hotkey: Option<global_hotkey::GlobalHotKeyManager>,
    summon_ok: bool,
    socket: Option<PathBuf>,
}

impl Global for Native {}

/// Hook the Dock click (`applicationShouldHandleReopen`) before launch:
/// `gpui_kit::application().with_assets(..).with_native().run(..)`.
pub trait WithNative {
    fn with_native(self) -> Self;
}

impl WithNative for Application {
    fn with_native(self) -> Self {
        self.on_reopen(show_main);
        self
    }
}

/// Start everything native. Call before the main window opens: a second
/// launch exits here, before it shows anything.
pub fn init(cx: &mut App) {
    let g = gate::gate();
    eprintln!("harbard: {}", g.describe());
    let socket = allows(Feature::SingleInstance)
        .then(|| claim_single_instance(cx))
        .flatten();
    // A dev build never takes over the 2560×720 display (the installed app
    // lives there) unless `HARBARD_APP_STRIP=wide` asks for it.
    let wide_ok = g.bundled || std::env::var("HARBARD_APP_STRIP").is_ok_and(|v| v == "wide");
    let strip_mode = if wide_ok && wide_screen().is_some() {
        StripMode::Wide
    } else {
        StripMode::Window
    };
    cx.set_global(Native {
        main: None,
        strip: None,
        strip_mode,
        tray: None,
        hotkey: None,
        summon_ok: false,
        socket,
    });
    bind(cx);
    set_menus(cx);
    let (tx, rx) = async_channel::unbounded::<Cmd>();
    if allows(Feature::Hotkey) {
        install_hotkey(tx.clone(), cx);
    }
    if allows(Feature::Tray) {
        let labels = tray_labels(cx);
        let summon_ok = cx.global::<Native>().summon_ok;
        cx.global_mut::<Native>().tray = tray::Tray::new(&labels, summon_ok, tx.clone());
    }
    cx.spawn(async move |cx: &mut gpui::AsyncApp| {
        while let Ok(cmd) = rx.recv().await {
            cx.update(|cx| run(cmd, cx));
        }
    })
    .detach();
    cx.on_system_notification_response(|response, cx| {
        if response.tag.starts_with("update") {
            open_updates(cx);
        } else if response.tag.starts_with("doorbell") {
            open_main(Some(Route::activity()), cx);
        } else {
            show_main(cx);
        }
    });
    cx.observe_global::<I18n>(|cx| {
        set_menus(cx);
        let labels = tray_labels(cx);
        let summon_ok = cx.global::<Native>().summon_ok;
        with_tray(cx, |t| {
            t.relabel(&labels, summon_ok);
            t.reset_count();
        });
        watch::show_count(cx);
    })
    .detach();
    cx.on_app_quit(|cx| {
        if allows(Feature::Dock) {
            appkit::set_tile(None);
        }
        if let Some(socket) = cx.global::<Native>().socket.clone() {
            single::release(&socket);
        }
        async {}
    })
    .detach();
    // The main window opens right after this returns.
    cx.defer(|cx| {
        adopt_main(cx);
        watch::init(cx);
        let mode = cx.global::<Native>().strip_mode;
        // `HARBARD_APP_STRIP=1` opens the Strip window at launch (to look at
        // it in a dev build without a 2560×720 display).
        if mode == StripMode::Wide || std::env::var_os("HARBARD_APP_STRIP").is_some() {
            crate::strip::open(mode, false, cx);
        }
    });
}

fn claim_single_instance(cx: &mut App) -> Option<PathBuf> {
    let id = if gate::gate().bundled {
        single::BUNDLE_ID
    } else {
        single::DEV_ID
    };
    let socket = single::socket_path(id);
    match single::claim(&socket) {
        single::Claim::Handed => {
            eprintln!("harbard: already running; showing it instead");
            std::process::exit(0);
        }
        single::Claim::First(listener) => {
            let (tx, rx) = async_channel::unbounded::<()>();
            single::listen(listener, move || {
                let _ = tx.send_blocking(());
            });
            cx.spawn(async move |cx: &mut gpui::AsyncApp| {
                while rx.recv().await.is_ok() {
                    cx.update(show_main);
                }
            })
            .detach();
            Some(socket)
        }
        single::Claim::Alone => None,
    }
}

fn install_hotkey(tx: async_channel::Sender<Cmd>, cx: &mut App) {
    use global_hotkey::hotkey::{Code, HotKey, Modifiers};
    use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
    let Ok(manager) = GlobalHotKeyManager::new() else {
        eprintln!("harbard: summon shortcut unavailable");
        return;
    };
    let summon = HotKey::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::Space);
    match manager.register(summon) {
        Ok(()) => {
            GlobalHotKeyEvent::set_event_handler(Some(move |e: GlobalHotKeyEvent| {
                if e.state == HotKeyState::Pressed {
                    let _ = tx.send_blocking(Cmd::ToggleMain);
                }
            }));
            let n = cx.global_mut::<Native>();
            n.summon_ok = true;
            n.hotkey = Some(manager);
        }
        Err(e) => eprintln!("harbard: summon shortcut unavailable: {e}"),
    }
}

fn run(cmd: Cmd, cx: &mut App) {
    match cmd {
        Cmd::OpenOverview => show_main(cx),
        Cmd::ToggleMain => toggle_main(cx),
        Cmd::ToggleMute => set_mute(!muted(), cx),
        Cmd::ShowStrip => open_strip(cx),
        Cmd::CloseStrip => close_strip(cx),
        Cmd::Updates => open_updates(cx),
        Cmd::Quit => cx.quit(),
    }
}

// ---------------------------------------------------------------------------
// Words
// ---------------------------------------------------------------------------

/// A `native.*` string.
pub fn tr(cx: &App, key: &str, vars: &[(&str, &str)]) -> String {
    tv(cx, &format!("native.{key}"), vars).to_string()
}

fn tray_labels(cx: &App) -> tray::Labels {
    let summon_ok = cx.global::<Native>().summon_ok;
    tray::Labels {
        open: tr(cx, "openOverview", &[]),
        mute: tr(
            cx,
            if muted() {
                "unmuteNotifications"
            } else {
                "muteNotifications"
            },
            &[],
        ),
        show_strip: tr(cx, "showStrip", &[]),
        close_strip: tr(cx, "closeStrip", &[]),
        summon: tr(
            cx,
            if summon_ok {
                "summonShortcut"
            } else {
                "summonUnavailable"
            },
            &[],
        ),
        updates: tr(cx, "checkForUpdates", &[]),
        quit: tr(cx, "quit", &[]),
    }
}

pub fn with_tray(cx: &mut App, f: impl FnOnce(&mut tray::Tray)) {
    if let Some(t) = cx.global_mut::<Native>().tray.as_mut() {
        f(t);
    }
}

// ---------------------------------------------------------------------------
// Menus and keys
// ---------------------------------------------------------------------------

fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("cmd-1", OpenOverview, None),
        KeyBinding::new("cmd-2", ShowStrip, None),
        KeyBinding::new("cmd-w", CloseWindow, None),
        KeyBinding::new("cmd-q", Quit, None),
        KeyBinding::new("cmd-h", Hide, None),
        KeyBinding::new("cmd-alt-h", HideOthers, None),
        KeyBinding::new("cmd-m", Minimize, None),
        KeyBinding::new("cmd-alt-p", TogglePerformance, None),
    ]);
    cx.on_action(|_: &TogglePerformance, cx| crate::shell::perf_hud::toggle(cx));
    cx.on_action(|_: &About, cx| with_main(cx, crate::prefs::open_about));
    cx.on_action(|_: &CheckForUpdates, cx| open_updates(cx));
    cx.on_action(|_: &OpenOverview, cx| show_main(cx));
    cx.on_action(|_: &ShowStrip, cx| open_strip(cx));
    cx.on_action(|_: &CloseStrip, cx| close_strip(cx));
    cx.on_action(|_: &CloseWindow, cx| hide_focused(cx));
    cx.on_action(|_: &Quit, cx| cx.quit());
    cx.on_action(|_: &Hide, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    cx.on_action(|_: &Minimize, cx| {
        if let Some(w) = cx.active_window() {
            cx.defer(move |cx| {
                let _ = w.update(cx, |_, window, _| window.minimize_window());
            });
        }
    });
    cx.on_action(|_: &Zoom, cx| {
        if let Some(w) = cx.active_window() {
            cx.defer(move |cx| {
                let _ = w.update(cx, |_, window, _| window.zoom_window());
            });
        }
    });
}

/// The app menu: About and Check for updates…; Edit for text fields; View
/// with Open overview ⌘1, Show Strip view ⌘2, Close Strip and Performance
/// ⌥⌘P; Window with
/// Close Window ⌘W, which hides the window instead of closing it.
fn set_menus(cx: &mut App) {
    use gpui_kit::component::input::{Copy, Cut, Paste, Redo, SelectAll, Undo};
    let t = |k: &str| tr(cx, k, &[]);
    let menus = vec![
        Menu::new("harbard").items([
            MenuItem::action(t("about"), About),
            MenuItem::action(t("checkForUpdates"), CheckForUpdates),
            MenuItem::separator(),
            MenuItem::os_submenu("Services", gpui::SystemMenuType::Services),
            MenuItem::separator(),
            MenuItem::action(t("hide"), Hide),
            MenuItem::action(t("hideOthers"), HideOthers),
            MenuItem::action(t("showAll"), ShowAll),
            MenuItem::separator(),
            MenuItem::action(t("quitHarbard"), Quit),
        ]),
        Menu::new(t("edit")).items([
            MenuItem::os_action(t("undo"), Undo, OsAction::Undo),
            MenuItem::os_action(t("redo"), Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action(t("cut"), Cut, OsAction::Cut),
            MenuItem::os_action(t("copy"), Copy, OsAction::Copy),
            MenuItem::os_action(t("paste"), Paste, OsAction::Paste),
            MenuItem::os_action(t("selectAll"), SelectAll, OsAction::SelectAll),
        ]),
        Menu::new(t("view")).items([
            MenuItem::action(t("openOverview"), OpenOverview),
            MenuItem::action(t("showStrip"), ShowStrip),
            MenuItem::action(t("closeStrip"), CloseStrip),
            MenuItem::separator(),
            MenuItem::action(t("performance"), TogglePerformance),
        ]),
        Menu::new(t("window")).items([
            MenuItem::action(t("minimize"), Minimize),
            MenuItem::action(t("zoom"), Zoom),
            MenuItem::separator(),
            MenuItem::action(t("closeWindow"), CloseWindow),
        ]),
    ];
    cx.set_menus(menus);
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

/// The window `main.rs` opened: remembered, and its close button hides it.
fn adopt_main(cx: &mut App) {
    let Some(main) = cx.windows().into_iter().next() else {
        return;
    };
    cx.global_mut::<Native>().main = Some(main);
    let _ = main.update(cx, |_, window, cx| {
        window.set_window_title(&tr(cx, "mainTitle", &[]));
        window.on_window_should_close(cx, |window, _| {
            appkit::hide(window);
            false
        });
    });
}

fn with_window(
    handle: Option<AnyWindowHandle>,
    cx: &mut App,
    f: impl FnOnce(&mut gpui::Window, &mut App) + 'static,
) {
    let Some(handle) = handle else { return };
    // Deferred: an action may arrive while that very window is updating.
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| f(window, cx));
    });
}

fn with_main(cx: &mut App, f: fn(&mut gpui::Window, &mut App)) {
    let main = cx.global::<Native>().main;
    with_window(main, cx, move |window, cx| {
        appkit::show(window);
        f(window, cx);
    });
}

/// Bring the main window forward (un-minimized, focused).
pub fn show_main(cx: &mut App) {
    let main = cx.global::<Native>().main;
    with_window(main, cx, |window, _| appkit::show(window));
}

/// Hide the main window when it is on screen, else bring it forward.
pub fn toggle_main(cx: &mut App) {
    let main = cx.global::<Native>().main;
    with_window(main, cx, |window, _| {
        if appkit::visible(window) {
            appkit::hide(window);
        } else {
            appkit::show(window);
        }
    });
}

/// Show the main window at `route` (the Strip's taps, a doorbell's click).
pub fn open_main(route: Option<Route>, cx: &mut App) {
    if let Some(route) = route {
        nav::navigate(route, cx);
    }
    show_main(cx);
}

/// The main window with the update dialog open ("Check for updates…").
pub fn open_updates(cx: &mut App) {
    with_main(cx, crate::prefs::open_update_dialog);
}

/// ⌘W: hide whichever harbard window has focus. A borderless Strip has no
/// close button, so AppKit's own Close would only beep.
fn hide_focused(cx: &mut App) {
    let active = cx.active_window();
    with_window(active, cx, |window, _| appkit::hide(window));
}

/// Whether the main window is in front with focus (a doorbell then stays
/// quiet: the operator is looking).
pub fn main_in_front(cx: &App) -> bool {
    let main = cx.global::<Native>().main;
    main.is_some() && cx.active_window() == main && appkit::app_active()
}

/// Show the Strip window (⌘2, the tray, Preferences, the workspace menu).
pub fn open_strip(cx: &mut App) {
    let mode = cx.global::<Native>().strip_mode;
    match cx.global::<Native>().strip {
        Some(handle) => with_window(Some(handle), cx, |window, _| appkit::show(window)),
        None => crate::strip::open(mode, true, cx),
    }
}

/// Hide the Strip window, in either mode. Hiding, never closing: it keeps
/// its state and Show Strip view brings the same window back.
pub fn close_strip(cx: &mut App) {
    let strip = cx.global::<Native>().strip;
    with_window(strip, cx, |window, _| appkit::hide(window));
}

/// A doorbell shows the Strip only where it is a panel of its own (the wide
/// display), and never takes the keyboard.
pub fn summon_strip(cx: &mut App) {
    let n = cx.global::<Native>();
    if n.strip_mode != StripMode::Wide {
        return;
    }
    let strip = n.strip;
    with_window(strip, cx, |window, _| {
        if !appkit::visible(window) {
            appkit::show_quietly(window);
        }
    });
}

/// Where the Strip lives this run.
pub fn strip_mode(cx: &App) -> StripMode {
    cx.global::<Native>().strip_mode
}

/// The 2560×720 display the Strip fills, if one is connected.
pub fn wide_screen() -> Option<appkit::Screen> {
    appkit::screens()
        .into_iter()
        .find(|s| s.pixels() == (2560, 720))
}

// ---------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------

/// Notifications are muted (`desk.json` `mute`).
pub fn muted() -> bool {
    crate::desk::desk_prefs().mute
}

/// `desk.json` `mute` as last written. One switch with two faces (the
/// tray's "Mute notifications" and Preferences' "Show macOS alerts"):
/// observe this global to follow a change made from the other one.
pub struct Mute(pub bool);

impl Global for Mute {}

/// Mute or unmute the desktop's notifications; the tray's item and
/// Preferences follow. [`Mute`] holds what `desk.json` says afterwards, so
/// a write that failed shows as unchanged.
pub fn set_mute(mute: bool, cx: &mut App) {
    crate::desk::save_desk_prefs(|p| p.mute = mute);
    cx.set_global(Mute(muted()));
    let labels = tray_labels(cx);
    let summon_ok = cx.global::<Native>().summon_ok;
    with_tray(cx, |t| t.relabel(&labels, summon_ok));
}

// ---------------------------------------------------------------------------
// The away suggestion
// ---------------------------------------------------------------------------

/// The away watch suggested a state: the main window's corner card offers
/// it, and only the operator's click changes the state.
pub fn suggest(s: away::Suggest, cx: &mut App) {
    let s = match s {
        away::Suggest::Away => crate::shell::Suggestion::Away,
        away::Suggest::Available => crate::shell::Suggestion::Available,
    };
    crate::shell::suggest(s, cx);
}
