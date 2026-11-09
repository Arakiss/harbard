//! harbard desktop — the console, plus what a browser tab cannot do.
//!
//! The console stays a web app on loopback (SPEC-API; the HTTP contract is the
//! only door). This shell loads it, adopts the running stack, and adds the
//! desktop's three superpowers: a menubar tray with the fleet's live count,
//! native notifications when the fleet answers the operator, and a global
//! shortcut that summons the window. Closing the window hides it — a monitor
//! lives in the tray; quit is an explicit menu act.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod away;
mod dock;
mod dock_shell;
mod i18n;
#[cfg(test)]
mod layered_stamp;
mod os_notifications;

use i18n::{tr, tr_with, Locale};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;
use tauri::menu::{Menu, MenuItem, MenuItemKind, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;

const API: &str = "http://127.0.0.1:7419/api/v1";
const POLL: Duration = Duration::from_secs(4);
/// Refresh fleet names at most this often — names almost never change mid-poll.
const ALIAS_TTL: Duration = Duration::from_secs(60);
/// Launchd / `just up` console (production Next on :3419).
const CONSOLE_LAUNCHD: &str = "http://127.0.0.1:3419";
/// Portless named host used by `bun run dev`.
const CONSOLE_PORTLESS: &str = "http://harbard.local:1355";
/// A 2560×720 panel exposes its full pixel grid as CSS pixels.
/// A modest zoom keeps type readable at desk distance. The fixed composition
/// is verified at the resulting 2133×600 CSS viewport as well as 2560×720.
const WIDE_DISPLAY_UI_ZOOM: f64 = 1.2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PulseWindowMode {
    Unavailable,
    WideDisplay,
    Fallback,
}

impl PulseWindowMode {
    fn doorbell_can_show(self) -> bool {
        self == Self::WideDisplay
    }
}

/// How the Strip window is built in each mode. On a wide display it is the
/// panel itself: borderless, fixed, edge to edge. Anywhere else it is an
/// ordinary window the operator can see in the Dock, move, resize and close
/// with its own button; the strip inside scales to whatever size it gets.
#[derive(Debug, PartialEq, Eq)]
struct PulseWindowFlags {
    title: &'static str,
    decorations: bool,
    resizable: bool,
    skip_taskbar: bool,
}

fn pulse_window_flags(mode: PulseWindowMode) -> PulseWindowFlags {
    match mode {
        PulseWindowMode::WideDisplay => PulseWindowFlags {
            title: "harbard · Strip",
            decorations: false,
            resizable: false,
            skip_taskbar: false,
        },
        PulseWindowMode::Fallback | PulseWindowMode::Unavailable => PulseWindowFlags {
            title: "harbard · Strip",
            decorations: true,
            resizable: true,
            skip_taskbar: false,
        },
    }
}

/// The strip's aspect: a 2560×720 display is 32:9.
const STRIP_ASPECT_W: f64 = 32.0;
const STRIP_ASPECT_H: f64 = 9.0;
/// The fallback window never grows past this width, so on a large monitor it
/// stays a strip and not a wall.
const FALLBACK_MAX_WIDTH: f64 = 1600.0;
/// The smallest fallback window, still 32:9.
const FALLBACK_MIN_WIDTH: f64 = 640.0;

#[derive(Clone, Copy)]
struct DeskFlags {
    summon_ok: bool,
    has_pulse: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct DeskPrefs {
    mute: bool,
    /// Kept as text: an unreadable value must not reset the rest of the file.
    locale: String,
}

impl DeskPrefs {
    fn locale(&self) -> Locale {
        Locale::parse(&self.locale).unwrap_or_default()
    }
}

#[derive(Deserialize)]
struct SessionsResponse {
    sessions: Vec<SessionView>,
}

#[derive(Deserialize)]
struct SessionView {
    harness: String,
    liveness: String,
}

#[derive(Deserialize)]
struct EventsResponse {
    events: Vec<BusEvent>,
}

#[derive(Deserialize)]
struct BusEvent {
    id: String,
    event: String,
    session: String,
    harness: String,
    text: String,
    #[serde(default)]
    kind: Option<String>,
}

/// One row of `GET /api/v1/fleet` — only the two fields the doorbell needs.
#[derive(Deserialize)]
struct FleetSessionCard {
    session: String,
    display_name: String,
}

/// The fleet projection, read for names only (SPEC-FLEET-VIEW §1.1).
///
/// The shell used to name a session from `/aliases`, which answers with the
/// alias ledger alone: a session with no alias fell back to its harness, so
/// every unnamed agent rang as "Claude Code". `/fleet` resolves a name for
/// every row it sends, the same name the console shows, so a notification and
/// the board cannot disagree about who spoke.
#[derive(Deserialize)]
struct FleetNames {
    #[serde(default)]
    sessions: Vec<FleetSessionCard>,
}

impl FleetNames {
    fn name_of(&self, session: &str) -> Option<&str> {
        self.sessions
            .iter()
            .find(|card| card.session == session)
            .map(|card| card.display_name.as_str())
    }
}

pub(crate) fn harbard_home() -> PathBuf {
    if let Ok(home) = std::env::var("HARBARD_HOME") {
        return PathBuf::from(home);
    }
    std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(".harbard"))
        .unwrap_or_else(|_| PathBuf::from(".harbard"))
}

fn desk_prefs_path() -> PathBuf {
    harbard_home().join("desk.json")
}

fn load_desk_prefs() -> DeskPrefs {
    let path = desk_prefs_path();
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return DeskPrefs::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

fn save_desk_prefs(prefs: &DeskPrefs) -> Result<(), String> {
    let path = desk_prefs_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let raw = serde_json::to_string_pretty(prefs).map_err(|e| e.to_string())?;
    std::fs::write(&path, raw).map_err(|e| e.to_string())
}

/// The hub credential serve expects, found where the console's proxy finds it
/// (`apps/console/src/lib/upstream.ts`). Without it every tray read answered
/// 401, so the live count and the native doorbell had nothing to go on.
fn hub_token() -> Option<String> {
    if let Ok(token) = std::env::var("HARBARD_HUB_TOKEN") {
        let token = token.trim();
        if !token.is_empty() {
            return Some(token.to_string());
        }
    }
    let raw = std::fs::read_to_string(harbard_home().join("hub.token")).ok()?;
    let token = raw.trim();
    (!token.is_empty()).then(|| token.to_string())
}

fn api_get<T: for<'de> Deserialize<'de>>(
    client: &reqwest::blocking::Client,
    path: &str,
) -> Option<T> {
    let mut request = client.get(format!("{API}{path}"));
    if let Some(token) = hub_token() {
        request = request.bearer_auth(token);
    }
    request
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json::<T>()
        .ok()
}

/// True when a loopback console answers within a short timeout.
///
/// Probes a static asset, never a page. The root is a full server render with
/// a 500-event snapshot behind it; probing it every 4 s kept the console at a
/// fifth of a core, and under load the render outran the timeout, so the shell
/// read a busy console as a dead one and went looking for another.
fn console_up(url: &str) -> bool {
    let Ok(client) = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(2))
        .build()
    else {
        return false;
    };
    client
        .get(format!("{}/icon.svg", url.trim_end_matches('/')))
        .send()
        .map(|r| r.status().is_success() || r.status().as_u16() == 304)
        .unwrap_or(false)
}

/// Where the shell should load the console from.
///
/// Order: `HARBARD_CONSOLE_URL` → launchd `:3419` → portless `harbard.local`
/// → launchd default (so a cold start still has a destination).
fn console_base() -> String {
    if let Ok(url) = std::env::var("HARBARD_CONSOLE_URL") {
        let url = url.trim().trim_end_matches('/').to_string();
        if !url.is_empty() {
            return url;
        }
    }
    if console_up(CONSOLE_LAUNCHD) {
        return CONSOLE_LAUNCHD.to_string();
    }
    if console_up(CONSOLE_PORTLESS) {
        return CONSOLE_PORTLESS.to_string();
    }
    CONSOLE_LAUNCHD.to_string()
}

fn console_page(base: &str, path: &str) -> String {
    format!("{}{}", base.trim_end_matches('/'), path)
}

/// Title case for a fleet name: `vidar` -> `Vidar` (alias::display in core).
fn display(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// First line, capped: a notification is a doorbell, not the message.
fn snippet(text: &str, max: usize) -> String {
    let first = text.lines().next().unwrap_or("");
    if first.chars().count() > max {
        let cut: String = first.chars().take(max - 1).collect();
        format!("{cut}…")
    } else {
        first.to_string()
    }
}

/// A name the operator recognizes: the one the fleet resolved, else the harness.
fn name_for(names: &FleetNames, session: &str, harness: &str) -> String {
    names
        .name_of(session)
        .map(display)
        .unwrap_or_else(|| display(harness))
}

/// What an operator would stop for: an ack to one of their handoffs, or an
/// interrupting note from the fleet. fyi chatter stays in the console.
/// Keep title/body grammar in sync with `apps/console/src/lib/doorbell.ts`
/// (`doorbellFor`). Web skips Notification when `?shell=desktop` so this
/// Tauri path is the sole native doorbell in the desktop shell.
fn notification_for(ev: &BusEvent, names: &FleetNames, locale: Locale) -> Option<(String, String)> {
    if ev.harness == "human" {
        return None;
    }
    let name = name_for(names, &ev.session, &ev.harness);
    let title = |key: &str| tr_with(locale, key, &[("name", name.as_str())]);
    match (ev.event.as_str(), ev.kind.as_deref()) {
        ("ack", _) => {
            let reply = ev.text.split_once(" — ").map(|(_, r)| r).unwrap_or("");
            Some((title("tookItOn"), snippet(reply, 90)))
        }
        ("note", Some("handoff")) => Some((title("handsOff"), snippet(&ev.text, 90))),
        ("note", Some("warning")) => Some((title("warnsFleet"), snippet(&ev.text, 90))),
        ("note", _)
            if mentions(&ev.text, &operator_handle())
                || mentions(&ev.text, DEFAULT_OPERATOR_HANDLE) =>
        {
            Some((title("mentionedYou"), snippet(&ev.text, 90)))
        }
        _ => None,
    }
}

/// How an `@` names the operator when no name is configured anywhere.
const DEFAULT_OPERATOR_HANDLE: &str = "operator";

/// How an `@` names the operator: Profile's name, else the desk's, else
/// [`DEFAULT_OPERATOR_HANDLE`].
fn operator_handle() -> String {
    #[derive(Deserialize, Default)]
    struct Profile {
        #[serde(default)]
        name: Option<String>,
    }
    #[derive(Deserialize, Default)]
    struct Config {
        #[serde(default)]
        operator_name: Option<String>,
    }
    #[derive(Deserialize, Default)]
    struct Desk {
        #[serde(default)]
        config: Config,
    }
    let home = harbard_home();
    let read = |file: &str| std::fs::read_to_string(home.join(file)).ok();
    read("profile.json")
        .and_then(|raw| serde_json::from_str::<Profile>(&raw).ok())
        .and_then(|profile| profile.name)
        .or_else(|| {
            read("workspace.json")
                .and_then(|raw| serde_json::from_str::<Desk>(&raw).ok())
                .and_then(|desk| desk.config.operator_name)
        })
        .and_then(|name| name.split_whitespace().next().map(str::to_lowercase))
        .unwrap_or_else(|| DEFAULT_OPERATOR_HANDLE.to_string())
}

/// Whether `text` holds `@handle` as a whole word, without case.
fn mentions(text: &str, handle: &str) -> bool {
    let text = text.to_lowercase();
    let wanted = format!("@{handle}");
    text.match_indices(&wanted).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + wanted.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '.')
            && !after.is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_')
    })
}

/// Mirror web `shouldSkipBecauseFocused`: tab already shows the thread.
fn main_window_focused(app: &AppHandle) -> bool {
    main_is_front(app)
}

fn show_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        // Minimized main + visible pulse: Dock activation alone may only
        // focus pulse. Always deminiaturize + show + focus the console.
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Explicit view switches restore the existing window without navigating either
/// webview or creating a second copy of the selected view.
fn open_existing_view(app: &AppHandle, label: &str) -> Result<(), String> {
    let window = app
        .get_webview_window(label)
        .ok_or_else(|| tr(load_desk_prefs().locale(), "viewUnavailable"))?;
    window.unminimize().map_err(|error| error.to_string())?;
    window.show().map_err(|error| error.to_string())?;
    window.set_focus().map_err(|error| error.to_string())
}

#[tauri::command]
fn desk_open_main(app: AppHandle, destination: Option<MainDestination>) -> Result<(), String> {
    if let Some(destination) = destination {
        let script = main_destination_script(&destination)?;
        app.get_webview_window("main")
            .ok_or_else(|| tr(load_desk_prefs().locale(), "mainViewUnavailable"))?
            .eval(script)
            .map_err(|error| error.to_string())?;
    }
    open_existing_view(&app, "main")
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct MainDestination {
    view: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    session: Option<String>,
}

fn main_destination_script(destination: &MainDestination) -> Result<String, String> {
    // The workspace shell's two destinations; the previous console that read
    // the others was removed.
    if !matches!(destination.view.as_str(), "direct" | "activity" | "update")
        || [&destination.note, &destination.session]
            .into_iter()
            .flatten()
            .any(|value| {
                value.is_empty() || value.len() > 512 || value.chars().any(char::is_control)
            })
    {
        return Err("Invalid main view destination.".to_string());
    }
    let detail = serde_json::to_string(destination).map_err(|error| error.to_string())?;
    // Serialize data as JSON; never interpolate an identifier into JS syntax.
    // Setup can delay the listener; its mount consumes the latest destination.
    Ok(format!(
        "window.__HARBARD_PENDING_DESTINATION__={detail};window.dispatchEvent(new CustomEvent('harbard:navigate',{{detail:window.__HARBARD_PENDING_DESTINATION__}}));"
    ))
}

#[tauri::command]
fn desk_open_strip(app: AppHandle) -> Result<(), String> {
    open_existing_view(&app, "pulse")
}

/// Hide the Strip window, in either mode. Hiding, never closing: the strip
/// keeps its state and Show Strip view brings the same window back.
#[tauri::command]
fn desk_close_strip(app: AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("pulse")
        .ok_or_else(|| "The Strip view is not available.".to_string())?;
    window.hide().map_err(|error| error.to_string())
}

/// ⌘W hides whichever harbard window has focus. The predefined Close Window
/// item asks the window to close itself, and a borderless window (Strip on
/// a wide display) has no close button, so AppKit refuses with a beep; hiding it
/// here works for every window, and matches what the close button does.
fn hide_focused_window(app: &AppHandle) {
    if let Some(window) = app
        .webview_windows()
        .into_values()
        .find(|w| w.is_focused().unwrap_or(false))
    {
        let _ = window.hide();
    }
}

fn main_is_front(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .map(|w| {
            w.is_visible().unwrap_or(false)
                && !w.is_minimized().unwrap_or(false)
                && w.is_focused().unwrap_or(false)
        })
        .unwrap_or(false)
}

fn toggle_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let visible = window.is_visible().unwrap_or(false);
        let minimized = window.is_minimized().unwrap_or(false);
        if visible && !minimized {
            let _ = window.hide();
        } else {
            show_main(app);
        }
    }
}

/// One step of the away watch: when it suggests a state, the main window
/// shows the suggestion (`harbard:suggest`, kept pending while it loads) and,
/// for Away, a silent notification says so; only the operator's click in
/// the console changes the state.
fn suggest_away(app: &AppHandle, watch: &mut away::AwayWatch) {
    let Some((idle_ms, locked)) = away::read_now() else {
        return;
    };
    let (state, enabled) = dock_shell::away_inputs(&harbard_home(), dock_shell::now_ms_pub());
    let Some(suggest) = watch.step(idle_ms, locked, &state, enabled) else {
        return;
    };
    let detail = serde_json::json!({ "state": suggest.state() }).to_string();
    let script = format!(
        "window.__HARBARD_PENDING_SUGGESTION__={detail};window.dispatchEvent(new CustomEvent('harbard:suggest',{{detail:window.__HARBARD_PENDING_SUGGESTION__}}));"
    );
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.eval(script);
    }
    let prefs = load_desk_prefs();
    if suggest == away::Suggest::Away && !prefs.mute && dock_shell::alerts_now() {
        let locale = prefs.locale();
        show_toast(app, tr(locale, "awayTitle"), tr(locale, "awayBody"));
    }
}

/// Soft cue on the pulse strip for a doorbell — never steal keyboard focus.
/// The OS notification already rang; yanking focus from the operator's
/// current app was the invasive failure mode.
fn summon_pulse(app: &AppHandle) {
    // The larger fallback is operator-only; notifications still arrive.
    if !app
        .try_state::<PulseWindowMode>()
        .is_some_and(|mode| mode.doorbell_can_show())
    {
        return;
    }
    if let Some(pulse) = app.get_webview_window("pulse") {
        if !pulse.is_visible().unwrap_or(false) {
            let _ = pulse.show();
        }
    }
}

/// Deliver one toast. Respects desk mute in `~/.harbard/desk.json`; the
/// alerts (doorbells, a lost bus) also wait out "Do not disturb" and the quiet
/// states (`dock_shell::alerts_now`); the answer to the operator's own click
/// does not.
/// Always silent — there is no alert chime in this product.
///
/// macOS notification clicks are not wired here: `tauri-plugin-notification`
/// uses notify-rust on desktop, which has no click callback. Use tray
/// **Open overview** (or ⌘⇧Space when registered) to summon the console.
fn show_toast(app: &AppHandle, title: impl AsRef<str>, body: impl AsRef<str>) {
    let prefs = load_desk_prefs();
    if prefs.mute {
        return;
    }
    let _ = app
        .notification()
        .builder()
        .title(title.as_ref())
        .body(body.as_ref())
        .silent()
        .show();
}

/// One OS toast per poll batch — ship rule: notify once, summon once.
fn show_batch_doorbell(app: &AppHandle, rings: &[(String, String)]) {
    let prefs = load_desk_prefs();
    if prefs.mute || !dock_shell::alerts_now() {
        return;
    }
    let locale = prefs.locale();
    if main_window_focused(app) {
        return;
    }
    match rings {
        [] => {}
        [(title, body)] => show_toast(app, title, body),
        many => {
            let n = many.len();
            let body = many
                .iter()
                .take(3)
                .map(|(t, _)| t.as_str())
                .collect::<Vec<_>>()
                .join(" · ");
            let body = if n > 3 {
                let more = tr_with(locale, "batchMore", &[("count", &(n - 3).to_string())]);
                format!("{body} · {more}")
            } else {
                body
            };
            let title = tr_with(locale, "batchTitle", &[("count", &n.to_string())]);
            show_toast(app, title, body);
        }
    }
}

/// `GET /api/v1/update`: only what the doorbell needs (docs/SPEC-UPDATES.md).
#[derive(Debug, Deserialize)]
struct UpdateBody {
    #[serde(default)]
    check: Option<UpdateCheck>,
}

#[derive(Debug, Deserialize)]
struct UpdateCheck {
    status: String,
    #[serde(default)]
    behind: u32,
    #[serde(default)]
    target: Option<UpdateTarget>,
}

#[derive(Debug, Deserialize)]
struct UpdateTarget {
    commit: String,
}

/// How often the shell asks serve whether a newer harbard exists: serve
/// does the fetching, so this is a cheap read of its cached answer.
const UPDATE_EVERY_TICKS: u64 = 150;

/// The target commit and the toast for an available update, if any.
fn update_notice(body: &UpdateBody, locale: Locale) -> Option<(String, String, String)> {
    let check = body.check.as_ref()?;
    if check.status != "available" {
        return None;
    }
    let commit = check.target.as_ref()?.commit.clone();
    let what = match check.behind {
        0 | 1 => tr(locale, "updateOneChange"),
        n => tr_with(locale, "updateManyChanges", &[("count", &n.to_string())]),
    };
    Some((
        commit,
        tr(locale, "updateTitle"),
        tr_with(locale, "updateBody", &[("what", &what)]),
    ))
}

/// The update the operator was last told about, so a restart never rings
/// the same one twice.
fn update_notified_path() -> PathBuf {
    harbard_home().join("update").join("desktop-notified")
}

fn ring_update(app: &AppHandle, client: &reqwest::blocking::Client) {
    let Some(body) = api_get::<UpdateBody>(client, "/update") else {
        return;
    };
    let Some((commit, title, text)) = update_notice(&body, load_desk_prefs().locale()) else {
        return;
    };
    let path = update_notified_path();
    if std::fs::read_to_string(&path).is_ok_and(|seen| seen.trim() == commit) {
        return;
    }
    show_toast(app, title, text);
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, commit);
}

/// Show the main window on the console's update dialog.
fn open_updates(app: &AppHandle) {
    let destination = MainDestination {
        view: "update".into(),
        note: None,
        session: None,
    };
    if let Err(error) = desk_open_main(app.clone(), Some(destination)) {
        show_toast(
            app,
            tr(load_desk_prefs().locale(), "couldNotOpenHarbard"),
            error,
        );
    }
}

fn build_tray_menu(app: &AppHandle, flags: DeskFlags) -> tauri::Result<Menu<tauri::Wry>> {
    let prefs = load_desk_prefs();
    let locale = prefs.locale();
    let mute_label = tr(
        locale,
        if prefs.mute {
            "unmuteNotifications"
        } else {
            "muteNotifications"
        },
    );
    let open = MenuItem::with_id(app, "open", tr(locale, "openOverview"), true, None::<&str>)?;
    let mute = MenuItem::with_id(app, "mute", mute_label, true, None::<&str>)?;
    let summon_label = tr(
        locale,
        if flags.summon_ok {
            "summonShortcut"
        } else {
            "summonUnavailable"
        },
    );
    let summon = MenuItem::with_id(app, "summon", summon_label, flags.summon_ok, None::<&str>)?;
    let updates = MenuItem::with_id(
        app,
        "updates",
        tr(locale, "checkForUpdates"),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", tr(locale, "quit"), true, None::<&str>)?;
    if flags.has_pulse {
        let show_pulse = MenuItem::with_id(
            app,
            "show_pulse",
            tr(locale, "showStrip"),
            true,
            None::<&str>,
        )?;
        let hide_pulse = MenuItem::with_id(
            app,
            "hide_pulse",
            tr(locale, "closeStrip"),
            true,
            None::<&str>,
        )?;
        Menu::with_items(
            app,
            &[
                &open,
                &mute,
                &show_pulse,
                &hide_pulse,
                &summon,
                &updates,
                &quit,
            ],
        )
    } else {
        Menu::with_items(app, &[&open, &mute, &summon, &updates, &quit])
    }
}

fn build_app_menu(app: &AppHandle, has_pulse: bool) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::default(app)?;
    let locale = load_desk_prefs().locale();
    let overview = MenuItem::with_id(
        app,
        "desk_open_main",
        tr(locale, "openOverview"),
        true,
        Some("CmdOrCtrl+1"),
    )?;
    let strip = MenuItem::with_id(
        app,
        "desk_open_strip",
        tr(locale, "showStrip"),
        has_pulse,
        Some("CmdOrCtrl+2"),
    )?;
    let close_strip = MenuItem::with_id(
        app,
        "desk_close_strip",
        tr(locale, "closeStrip"),
        has_pulse,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    replace_close_window(app, &menu)?;
    // macOS convention: "Check for updates…" right under "About" in the
    // application menu, the first submenu.
    if let Some(MenuItemKind::Submenu(app_menu)) = menu.items()?.into_iter().next() {
        let updates = MenuItem::with_id(
            app,
            "desk_updates",
            tr(locale, "checkForUpdates"),
            true,
            None::<&str>,
        )?;
        app_menu.insert(&updates, 1)?;
    }
    let view_menu = menu.items()?.into_iter().find_map(|item| match item {
        MenuItemKind::Submenu(submenu) if submenu.text().ok().as_deref() == Some("View") => {
            Some(submenu)
        }
        _ => None,
    });
    if let Some(view) = view_menu {
        view.prepend_items(&[&overview, &strip, &close_strip, &separator])?;
    } else {
        menu.append(&Submenu::with_items(
            app,
            tr(locale, "view"),
            true,
            &[&overview, &strip, &close_strip],
        )?)?;
    }
    Ok(menu)
}

/// Swap the default menu's predefined Close Window items (File and Window)
/// for one of ours with the same ⌘W, which hides the focused window: see
/// `hide_focused_window`. The first one keeps the shortcut; any other is
/// dropped, so ⌘W has exactly one owner.
fn replace_close_window(app: &AppHandle, menu: &Menu<tauri::Wry>) -> tauri::Result<()> {
    let mut placed = false;
    for item in menu.items()? {
        let MenuItemKind::Submenu(submenu) = item else {
            continue;
        };
        for (index, child) in submenu.items()?.into_iter().enumerate() {
            let MenuItemKind::Predefined(predefined) = child else {
                continue;
            };
            if predefined.text().ok().as_deref() != Some("Close Window") {
                continue;
            }
            submenu.remove(&predefined)?;
            if !placed {
                let close = MenuItem::with_id(
                    app,
                    "desk_close_window",
                    tr(load_desk_prefs().locale(), "closeWindow"),
                    true,
                    Some("CmdOrCtrl+W"),
                )?;
                submenu.insert(&close, index)?;
                placed = true;
            }
        }
    }
    Ok(())
}

fn refresh_tray_menu(app: &AppHandle) {
    let flags = match app.try_state::<Mutex<DeskFlags>>() {
        Some(state) => *state.lock().unwrap(),
        None => DeskFlags {
            summon_ok: false,
            has_pulse: app.get_webview_window("pulse").is_some(),
        },
    };
    if let Ok(menu) = build_tray_menu(app, flags) {
        if let Some(tray) = app.tray_by_id("fleet") {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn toggle_desk_mute(app: &AppHandle) {
    let mut prefs = load_desk_prefs();
    prefs.mute = !prefs.mute;
    if save_desk_prefs(&prefs).is_ok() {
        refresh_tray_menu(app);
    }
}

/// Re-point main (and pulse) at a live console if cold start hit a dead URL.
fn rescue_console(app: &AppHandle, last_base: &mut String) {
    let next = console_base();
    if next == *last_base && console_up(last_base) {
        return;
    }
    if !console_up(&next) {
        return;
    }
    *last_base = next.clone();
    reload_console_windows(app, &next);
}

/// Point both windows at the console again.
///
/// Separate from `rescue_console` because there are two different reasons to
/// do it, and only one of them was covered. `rescue_console` fires when the
/// console MOVES, and it returns early when the address is unchanged — which
/// is right for a move and useless for the case that actually happens: the
/// console restarts in place. `just update` takes it down for about two
/// seconds, whatever was loading in that window gets WebKit's "This page
/// couldn't load", and the console comes back at the same address. From then
/// on the supervisor sees a healthy console and never touches the window
/// again, so the panel stays on an error page with a Reload button nobody is
/// standing next to.
fn reload_console_windows(app: &AppHandle, base: &str) {
    for (label, path) in [
        ("main", "/?shell=desktop"),
        ("pulse", "/pulse?shell=desktop"),
    ] {
        if let Some(window) = app.get_webview_window(label) {
            if let Ok(parsed) = console_page(base, path).parse() {
                let _ = window.navigate(parsed);
            }
        }
    }
}

/// The tray's senses: poll the loopback API, update the live count, and
/// fire the doorbell for events that deserve it. Loopback only — the shell
/// obeys the console's own no-egress rule.
fn poll_loop(app: AppHandle, initial_base: String) {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return,
    };
    let mut last_event: Option<String> = None;
    // Mirror web `useDoorbell`: seed the cursor from the tip event, never ring it.
    let mut hydrated = false;
    let mut last_live: Option<usize> = None;
    let mut api_was_up = false;
    let mut disconnect_rang = false;
    let mut dock_reachable: Option<bool> = None;
    let mut console_base_url = initial_base;
    let mut fleet_names = FleetNames {
        sessions: Vec::new(),
    };
    let mut names_fetched_at = std::time::Instant::now()
        .checked_sub(ALIAS_TTL)
        .unwrap_or_else(std::time::Instant::now);
    // A console that was unreachable and is now answering has almost certainly
    // just restarted under whatever was showing in the window. Seeing it come
    // back is the only signal we get that the page needs loading again.
    let mut console_was_down = !console_up(&console_base_url);
    // "Suggest Away": ioreg every fourth tick (16 s), never a switch.
    let mut away = away::AwayWatch::default();
    let mut tick: u64 = 0;
    loop {
        tick = tick.wrapping_add(1);
        if tick.is_multiple_of(4) {
            suggest_away(&app, &mut away);
        }
        if console_up(&console_base_url) {
            if console_was_down {
                console_was_down = false;
                reload_console_windows(&app, &console_base_url);
            }
        } else {
            console_was_down = true;
            rescue_console(&app, &mut console_base_url);
        }

        dock_shell::check_heartbeat(&app);
        // A newer harbard: shortly after start, then every ten minutes.
        if tick == 3 || tick.is_multiple_of(UPDATE_EVERY_TICKS) {
            ring_update(&app, &client);
        }
        let sessions = api_get::<SessionsResponse>(&client, "/sessions");
        // The Dock shows an outage even when no console is left to report it.
        let reachable = sessions.is_some();
        if dock_reachable != Some(reachable) {
            dock_reachable = Some(reachable);
            dock_shell::set_offline(&app, !reachable);
        }
        match &sessions {
            None => {
                if api_was_up && !disconnect_rang && dock_shell::alerts_now() {
                    show_toast(
                        &app,
                        tr(load_desk_prefs().locale(), "busDisconnected"),
                        tr(load_desk_prefs().locale(), "busDisconnectedBody"),
                    );
                    summon_pulse(&app);
                    disconnect_rang = true;
                }
                api_was_up = false;
            }
            Some(sessions) => {
                api_was_up = true;
                disconnect_rang = false;
                let live = sessions
                    .sessions
                    .iter()
                    .filter(|s| s.liveness == "live" && s.harness != "human")
                    .count();
                if Some(live) != last_live {
                    last_live = Some(live);
                    if let Some(tray) = app.tray_by_id("fleet") {
                        let title = if live > 0 {
                            format!("{live}")
                        } else {
                            String::new()
                        };
                        let _ = tray.set_title(Some(&title));
                        let tip = if live > 0 {
                            format!("harbard — {live} live")
                        } else {
                            "harbard — the fleet".into()
                        };
                        let _ = tray.set_tooltip(Some(&tip));
                    }
                }
            }
        }

        let path = match &last_event {
            Some(id) => format!("/events?after={id}"),
            None => "/events?limit=1".to_string(),
        };
        if let Some(events) = api_get::<EventsResponse>(&client, &path) {
            if !hydrated {
                if let Some(last) = events.events.last() {
                    last_event = Some(last.id.clone());
                }
                hydrated = true;
            } else {
                let needs_names = events.events.iter().any(|ev| {
                    matches!(ev.event.as_str(), "ack")
                        || matches!(ev.kind.as_deref(), Some("handoff") | Some("warning"))
                });
                if needs_names && names_fetched_at.elapsed() >= ALIAS_TTL {
                    if let Some(fresh) = api_get::<FleetNames>(&client, "/fleet") {
                        fleet_names = fresh;
                    }
                    names_fetched_at = std::time::Instant::now();
                }
                let mut rings: Vec<(String, String)> = Vec::new();
                for ev in &events.events {
                    if let Some(ring) =
                        notification_for(ev, &fleet_names, load_desk_prefs().locale())
                    {
                        rings.push(ring);
                    }
                }
                if !rings.is_empty() {
                    show_batch_doorbell(&app, &rings);
                    summon_pulse(&app);
                }
                if let Some(last) = events.events.last() {
                    last_event = Some(last.id.clone());
                }
            }
        }
        thread::sleep(POLL);
    }
}

fn create_pulse_window(app: &AppHandle, base: &str) -> tauri::Result<PulseWindowMode> {
    let pulse_url = console_page(base, "/pulse?shell=desktop");
    let url = pulse_url.parse().expect("valid pulse url");

    if let Some(edge) = app
        .available_monitors()?
        .into_iter()
        .find(|m| m.size().width == 2560 && m.size().height == 720)
    {
        let flags = pulse_window_flags(PulseWindowMode::WideDisplay);
        let mut builder =
            tauri::WebviewWindowBuilder::new(app, "pulse", tauri::WebviewUrl::External(url))
                .title(flags.title)
                .decorations(flags.decorations)
                .resizable(flags.resizable)
                .skip_taskbar(flags.skip_taskbar)
                .always_on_top(false);
        builder = builder.position(edge.position().x.into(), edge.position().y.into());
        builder = builder.inner_size(2560.0, 720.0);
        let pulse = builder.build()?;
        pulse.set_zoom(WIDE_DISPLAY_UI_ZOOM)?;
        return Ok(PulseWindowMode::WideDisplay);
    }

    let monitor = app
        .primary_monitor()?
        .or_else(|| app.available_monitors().ok()?.into_iter().next());
    let Some(monitor) = monitor else {
        return Ok(PulseWindowMode::Unavailable);
    };
    let area = monitor.work_area();
    let scale = monitor.scale_factor();
    let (width, height) = fallback_pulse_dimensions(area.size.width, area.size.height, scale);
    let physical_width = (width * scale).round() as u32;
    let physical_height = (height * scale).round() as u32;
    let x = area.position.x + (area.size.width.saturating_sub(physical_width) / 2) as i32;
    let y = area.position.y + (area.size.height.saturating_sub(physical_height) / 2) as i32;

    let flags = pulse_window_flags(PulseWindowMode::Fallback);
    let pulse = tauri::WebviewWindowBuilder::new(app, "pulse", tauri::WebviewUrl::External(url))
        .title(flags.title)
        .decorations(flags.decorations)
        .resizable(flags.resizable)
        .skip_taskbar(flags.skip_taskbar)
        .always_on_top(false)
        .visible(false)
        .inner_size(width, height)
        .min_inner_size(
            FALLBACK_MIN_WIDTH,
            FALLBACK_MIN_WIDTH * STRIP_ASPECT_H / STRIP_ASPECT_W,
        )
        .build()?;
    // Keep physical placement correct on mixed-density monitor layouts.
    pulse.set_size(tauri::PhysicalSize::new(physical_width, physical_height))?;
    pulse.set_position(tauri::PhysicalPosition::new(x, y))?;
    pulse.set_zoom(1.0)?;
    Ok(PulseWindowMode::Fallback)
}

/// The fallback window's logical size: 32:9 like a wide display, as wide as 90%
/// of the work area allows (in width and in height), at most 1600 wide, and
/// a whole multiple of 32 so the height is a whole number too.
fn fallback_pulse_dimensions(width: u32, height: u32, scale: f64) -> (f64, f64) {
    let area_w = width as f64 / scale;
    let area_h = height as f64 / scale;
    let fit = (area_w * 0.9)
        .min(area_h * 0.9 * STRIP_ASPECT_W / STRIP_ASPECT_H)
        .min(FALLBACK_MAX_WIDTH);
    let w = ((fit / STRIP_ASPECT_W).floor() * STRIP_ASPECT_W).max(STRIP_ASPECT_W);
    (w, w * STRIP_ASPECT_H / STRIP_ASPECT_W)
}

#[tauri::command]
fn desk_get_prefs() -> DeskPrefs {
    load_desk_prefs()
}

#[tauri::command(async)]
fn desk_os_notifications() -> Option<os_notifications::OsNotifications> {
    os_notifications::read()
}

#[tauri::command]
fn desk_open_notification_settings(app: AppHandle) -> Result<(), String> {
    os_notifications::open_settings(&app.config().identifier)
}

#[tauri::command]
fn desk_set_mute(app: AppHandle, mute: bool) -> Result<(), String> {
    let mut prefs = load_desk_prefs();
    prefs.mute = mute;
    save_desk_prefs(&prefs)?;
    refresh_tray_menu(&app);
    Ok(())
}

/// Called on every page load, so an unchanged language must cost nothing:
/// rebuilding the menus or reloading Strip here would loop with that load.
#[tauri::command]
fn desk_set_locale(app: AppHandle, locale: String) -> Result<(), String> {
    let chosen = Locale::parse(&locale).ok_or_else(|| "Unknown language.".to_string())?;
    let mut prefs = load_desk_prefs();
    if prefs.locale() == chosen {
        return Ok(());
    }
    prefs.locale = chosen.code().to_string();
    save_desk_prefs(&prefs)?;
    refresh_tray_menu(&app);
    let pulse = app.get_webview_window("pulse");
    let menu = build_app_menu(&app, pulse.is_some()).map_err(|error| error.to_string())?;
    app.set_menu(menu).map_err(|error| error.to_string())?;
    // Only the window that changed the language reloads itself.
    if let Some(pulse) = pulse {
        let _ = pulse.eval("window.location.reload()");
    }
    Ok(())
}

fn main() {
    let context = tauri::generate_context!();
    dock_shell::refuse_demo_outside_its_bundle(&context.config().identifier);
    // The demo is self-contained: no Strip window, no shortcut, no poll of
    // the bus and no console probe, so it can run beside an installed copy.
    let demo = dock_shell::demo_enabled();
    let summon = Shortcut::new(Some(Modifiers::SUPER | Modifiers::SHIFT), Code::Space);
    let base = if demo { String::new() } else { console_base() };
    if !demo {
        eprintln!("harbard-desktop: console at {base}");
    }

    tauri::Builder::default()
        // Both installation paths share the bundle identifier, so a second
        // launch must reach the first app before tray or shortcut setup runs.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app);
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, _shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        toggle_window(app);
                    }
                })
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            desk_get_prefs,
            desk_os_notifications,
            desk_open_notification_settings,
            desk_set_mute,
            desk_set_locale,
            desk_open_main,
            desk_open_strip,
            desk_close_strip,
            dock_shell::desk_set_status,
        ])
        .on_menu_event(|app, event| {
            let result = match event.id().as_ref() {
                "desk_open_main" => desk_open_main(app.clone(), None),
                "desk_open_strip" => desk_open_strip(app.clone()),
                "desk_close_strip" => desk_close_strip(app.clone()),
                "desk_updates" => {
                    open_updates(app);
                    Ok(())
                }
                "desk_close_window" => {
                    hide_focused_window(app);
                    Ok(())
                }
                _ => return,
            };
            if let Err(error) = result {
                show_toast(
                    app,
                    tr(load_desk_prefs().locale(), "couldNotOpenView"),
                    error,
                );
            }
        })
        .manage(dock::Dock::default())
        .setup(move |app| {
            let pulse_mode = if demo {
                PulseWindowMode::Unavailable
            } else {
                create_pulse_window(app.handle(), &base).unwrap_or(PulseWindowMode::Unavailable)
            };
            let has_pulse = pulse_mode != PulseWindowMode::Unavailable;
            app.manage(pulse_mode);
            let summon_ok = !demo
                && match app.global_shortcut().register(summon) {
                    Ok(()) => true,
                    Err(err) => {
                        eprintln!("harbard-desktop: summon shortcut unavailable: {err}");
                        false
                    }
                };
            app.manage(Mutex::new(DeskFlags {
                summon_ok,
                has_pulse,
            }));
            app.set_menu(build_app_menu(app.handle(), has_pulse)?)?;

            let flags = DeskFlags {
                summon_ok,
                has_pulse,
            };
            let menu = build_tray_menu(app.handle(), flags)?;
            TrayIconBuilder::with_id("fleet")
                .icon(tauri::image::Image::from_bytes(include_bytes!(
                    "../icons/tray.png"
                ))?)
                .icon_as_template(true)
                .title("")
                .tooltip("harbard — the fleet")
                .menu(&menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "open" => show_main(app),
                    "mute" => toggle_desk_mute(app),
                    "show_pulse" => {
                        if let Err(error) = desk_open_strip(app.clone()) {
                            let title = tr(load_desk_prefs().locale(), "couldNotOpenStrip");
                            show_toast(app, title, error);
                        }
                    }
                    "hide_pulse" => {
                        let _ = desk_close_strip(app.clone());
                    }
                    "summon" => toggle_window(app),
                    "updates" => open_updates(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        toggle_window(tray.app_handle());
                    }
                })
                .build(app)?;

            // Point the main window at whichever console is actually up —
            // tauri.conf.json's url is only a cold-start default.
            if let Some(main) = app.get_webview_window("main").filter(|_| !demo) {
                main.set_title("harbard — Main")?;
                // `shell=desktop` tells the console to inset past macOS traffic
                // lights (Overlay titlebar). Loopback is a remote origin: the
                // capability `remote.urls` grants IPC so `data-tauri-drag-region`
                // can toggle-maximize on titlebar double-click. The query string
                // remains the reliable shell mark even if IPC is slow to bind.
                let url = console_page(&base, "/?shell=desktop");
                if let Ok(parsed) = url.parse() {
                    let _ = main.navigate(parsed);
                }
            }

            let handle = app.handle().clone();
            if demo {
                if let Some(main) = app.get_webview_window("main") {
                    let _ = main.hide();
                }
                dock_shell::run_demo(handle);
            } else {
                let poll_base = base.clone();
                thread::spawn(move || poll_loop(handle, poll_base));
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            // Close hides: the bus keeps running, the tray keeps watching.
            // Only the requested window — hiding pulse must not swallow main.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(context)
        .expect("error building harbard desktop")
        .run(|app_handle, event| {
            // macOS Dock click → applicationShouldHandleReopen. Pulse can still
            // be visible while main is minimized/hidden; do not gate on
            // has_visible_windows or Dock only surfaces the strip.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                show_main(app_handle);
            }
            // macOS shows the bundle icon once the app is gone; restore it
            // here as well, so nothing depends on the order of teardown.
            if let tauri::RunEvent::Exit = event {
                dock_shell::set_tile_now(None);
            }
            let _ = (app_handle, event);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doorbells_only_show_the_actual_wide_display_window() {
        assert!(PulseWindowMode::WideDisplay.doorbell_can_show());
        assert!(!PulseWindowMode::Fallback.doorbell_can_show());
        assert!(!PulseWindowMode::Unavailable.doorbell_can_show());
    }

    #[test]
    fn fallback_strip_is_32_by_9_and_fits_the_logical_work_area_at_each_scale() {
        // A 14-inch MacBook's work area at 2x: 1440 logical wide, 90% is 1296.
        assert_eq!(fallback_pulse_dimensions(2880, 1800, 2.0), (1280.0, 360.0));
        // A small screen: 90% of its width, 921.6, rounded down to 896.
        assert_eq!(fallback_pulse_dimensions(1024, 560, 1.0), (896.0, 252.0));
        // A large monitor: capped at 1600.
        assert_eq!(fallback_pulse_dimensions(1920, 1080, 1.0), (1600.0, 450.0));
        assert_eq!(fallback_pulse_dimensions(5120, 2880, 2.0), (1600.0, 450.0));
        // A very wide, very short work area: the height decides.
        assert_eq!(fallback_pulse_dimensions(3440, 300, 1.0), (960.0, 270.0));
        for (w, h, s) in [
            (2880, 1800, 2.0),
            (1024, 560, 1.0),
            (3024, 1890, 2.0),
            (1366, 768, 1.0),
        ] {
            let (fw, fh) = fallback_pulse_dimensions(w, h, s);
            assert_eq!(fw * 9.0, fh * 32.0, "32:9 at {w}x{h}@{s}");
            assert!(fw <= w as f64 / s && fh <= h as f64 / s, "fits {w}x{h}@{s}");
            assert_eq!(fh.fract(), 0.0, "whole logical height at {w}x{h}@{s}");
        }
    }

    #[test]
    fn the_fallback_strip_is_an_ordinary_window_and_the_wide_display_stays_borderless() {
        assert_eq!(
            pulse_window_flags(PulseWindowMode::Fallback),
            PulseWindowFlags {
                title: "harbard · Strip",
                decorations: true,
                resizable: true,
                skip_taskbar: false,
            }
        );
        assert_eq!(
            pulse_window_flags(PulseWindowMode::WideDisplay),
            PulseWindowFlags {
                title: "harbard · Strip",
                decorations: false,
                resizable: false,
                skip_taskbar: false,
            }
        );
    }

    #[test]
    fn main_navigation_serializes_identifiers_as_data_and_rejects_invalid_destinations() {
        let mut destination = MainDestination {
            view: "activity".into(),
            note: Some("note\";window.close();//".into()),
            session: None,
        };
        let script = main_destination_script(&destination).unwrap();
        assert!(script.starts_with(&format!(
            "window.__HARBARD_PENDING_DESTINATION__={};",
            serde_json::to_string(&destination).unwrap()
        )));
        assert!(script.contains("harbard:navigate"));
        assert!(script.contains("detail:window.__HARBARD_PENDING_DESTINATION__"));
        assert!(script.contains(r#"note\";window.close();//"#));
        destination.view = "reload".into();
        assert!(main_destination_script(&destination).is_err());
        // The strip opens an agent's Direct conversation and an
        // Activity item in the workspace shell.
        for view in ["direct", "activity"] {
            destination.view = view.into();
            assert!(main_destination_script(&destination).is_ok());
        }
        // The previous console's views are gone: nothing consumes them.
        for view in [
            "overview",
            "inbox",
            "messages",
            "timeline",
            "history",
            "dependencies",
        ] {
            destination.view = view.into();
            assert!(main_destination_script(&destination).is_err(), "{view}");
        }
        destination.view = "activity".into();
        destination.note = Some("bad\nidentifier".into());
        assert!(main_destination_script(&destination).is_err());
        destination.note = Some("x".repeat(513));
        assert!(main_destination_script(&destination).is_err());
        assert!(
            serde_json::from_str::<MainDestination>(r#"{"view":"activity","extra":"reload"}"#)
                .is_err()
        );
    }

    fn card(session: &str, display_name: &str) -> FleetSessionCard {
        FleetSessionCard {
            session: session.to_string(),
            display_name: display_name.to_string(),
        }
    }

    fn note(session: &str, harness: &str, kind: &str, text: &str) -> BusEvent {
        BusEvent {
            id: "01ABC".into(),
            event: "note".into(),
            session: session.into(),
            harness: harness.into(),
            text: text.into(),
            kind: Some(kind.into()),
        }
    }

    #[test]
    fn an_available_update_rings_once_per_target_and_counts_its_changes() {
        let body: UpdateBody = serde_json::from_value(serde_json::json!({
            "installed": { "version": "0.3.0-alpha.9" },
            "check": { "status": "available", "behind": 11, "target": { "commit": "abc", "version": "x" } },
            "run": null
        }))
        .unwrap();
        let (commit, title, text) = update_notice(&body, Locale::En).unwrap();
        assert_eq!(commit, "abc");
        assert_eq!(title, "A new version of harbard is available");
        assert!(text.starts_with("There are 11 new changes."), "{text}");
        let (_, title, text) = update_notice(&body, Locale::Es).unwrap();
        assert_eq!(title, "Hay una versión nueva de harbard");
        assert!(text.starts_with("Hay 11 cambios nuevos."), "{text}");
        let current: UpdateBody =
            serde_json::from_value(serde_json::json!({ "check": { "status": "current" } }))
                .unwrap();
        assert!(update_notice(&current, Locale::En).is_none());
        let none: UpdateBody =
            serde_json::from_value(serde_json::json!({ "check": null })).unwrap();
        assert!(update_notice(&none, Locale::En).is_none());
    }

    #[test]
    fn the_update_destination_is_a_main_view() {
        let script = main_destination_script(&MainDestination {
            view: "update".into(),
            note: None,
            session: None,
        })
        .unwrap();
        assert!(script.contains(r#"{"view":"update"}"#), "{script}");
    }

    #[test]
    fn a_notification_names_the_session_the_way_the_fleet_does() {
        let names = FleetNames {
            sessions: vec![card("aurora-4f2c9a", "aurora")],
        };
        let (title, _) = notification_for(
            &note("aurora-4f2c9a", "claude-code", "handoff", "hold the merge"),
            &names,
            Locale::En,
        )
        .expect("a handoff rings");
        assert_eq!(title, "Aurora hands off to you");
    }

    #[test]
    fn an_unknown_session_falls_back_to_its_harness() {
        // The projection names every row it sends, so this is only reached for
        // a session it has not seen — a note older than its window.
        let names = FleetNames { sessions: vec![] };
        let (title, _) = notification_for(
            &note("ghost-01", "codex", "warning", "the branch is gone"),
            &names,
            Locale::En,
        )
        .expect("a warning rings");
        assert_eq!(title, "Codex warns the fleet");
    }

    #[test]
    fn a_mention_is_a_whole_word_without_case() {
        assert!(mentions("FINDING · @Dana continue?", "dana"));
        assert!(mentions("@dana, look", "dana"));
        assert!(!mentions("mail a x@dana.es", "dana"));
        assert!(!mentions("@danas lo hizo", "dana"));
        assert!(!mentions("@dana-bot lo hizo", "dana"));
        assert!(mentions("@operator continue?", DEFAULT_OPERATOR_HANDLE));
    }

    #[test]
    fn the_operator_never_rings_themselves() {
        let names = FleetNames { sessions: vec![] };
        assert!(notification_for(
            &note("operator@host", "human", "handoff", "do it"),
            &names,
            Locale::En
        )
        .is_none());
    }

    #[test]
    fn fyi_chatter_stays_in_the_console() {
        let names = FleetNames {
            sessions: vec![card("aurora-4f2c9a", "aurora")],
        };
        assert!(notification_for(
            &note("aurora-4f2c9a", "claude-code", "fyi", "rebased"),
            &names,
            Locale::En
        )
        .is_none());
    }

    #[test]
    fn a_fleet_body_parses_into_the_names_the_doorbell_needs() {
        // The shape is the server's; only two fields are read, and the rest of
        // the projection must not make this fail to deserialize.
        let body = r#"{
            "v": 1,
            "counts": { "live": 1 },
            "sessions": [
                { "session": "s-1", "display_name": "lantern", "harness": "codex", "depth": 0 }
            ]
        }"#;
        let names: FleetNames = serde_json::from_str(body).expect("parses");
        assert_eq!(names.name_of("s-1"), Some("lantern"));
        assert_eq!(names.name_of("missing"), None);
    }
}
