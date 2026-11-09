//! The native half of [`crate::dock`]: AppKit calls, the Tauri command and
//! the demo.
//!
//! The model is touched only on the main thread, through [`with_model`]:
//! the command, the bus poll, the demo and the end of an animation all
//! hand it a closure. That keeps tiles in the order the model decided them,
//! and no thread ever holds the model's lock while waiting on the main
//! thread (`is_focused`, the tray, the badge), so they cannot deadlock.

use crate::dock::{self, Dock, Host, Model, OneShot, Plan, Status, Tile};
use serde_json::{json, Value};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager};

/// The bundle id the Dock demo runs under (`tauri.dockdemo.conf.json`).
pub const DEMO_IDENTIFIER: &str = "com.harbard.desktop.dockdemo";

/// Run `f` with the model on the main thread, then apply its plan there.
/// Inline when called on the main thread, queued otherwise.
pub fn with_model<F>(app: &AppHandle, f: F)
where
    F: FnOnce(&mut Model, &NativeHost<'_>) -> Plan + Send + 'static,
{
    let handle = app.clone();
    let _ = app.run_on_main_thread(move || {
        let plan = {
            let dock = handle.state::<Dock>();
            let mut model = dock.lock();
            f(&mut model, &NativeHost(&handle))
        };
        apply(&handle, plan);
    });
}

/// Apply a plan, on the main thread: tile, badge, tray, the animation and
/// the bounce.
fn apply(app: &AppHandle, plan: Plan) {
    if let Some(tile) = plan.tile {
        set_tile_now(tile);
    }
    if let Some(label) = plan.badge {
        if let Some(main) = app.get_webview_window("main") {
            let _ = main.set_badge_label(label);
        }
    }
    if let Some(mark) = plan.tray {
        if let Some(tray) = app.tray_by_id("fleet") {
            if let Ok(image) = tauri::image::Image::from_bytes(mark.png()) {
                let _ = tray.set_icon(Some(image));
                let _ = tray.set_icon_as_template(true);
            }
        }
    }
    if let Some(event) = plan.animate {
        play(app.clone(), event);
    }
    if plan.bounce {
        if let Some(main) = app.get_webview_window("main") {
            let _ = main.request_user_attention(Some(tauri::UserAttentionType::Informational));
        }
    }
}

/// One one-shot animation on a thread of its own, which ends with it. The
/// frames queue onto the main thread; the last step asks the model there.
fn play(app: AppHandle, event: OneShot) {
    thread::spawn(move || {
        for frame in event.frames() {
            let _ = app.run_on_main_thread(move || set_tile_now(Some(frame)));
            thread::sleep(dock::FRAME);
        }
        with_model(&app, |model, _| Plan {
            tile: Some(model.animation_done()),
            ..Plan::default()
        });
    });
}

/// `NSApplication.applicationIconImage`; `None` restores the bundle icon.
/// Must run on the main thread; does nothing anywhere else.
#[cfg(target_os = "macos")]
pub fn set_tile_now(tile: Option<&'static Tile>) {
    use objc2::AllocAnyThread;
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSImage};
    use objc2_foundation::NSData;

    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let ns_app = NSApplication::sharedApplication(mtm);
    match tile {
        // SAFETY: nil is documented to restore the bundle's icon.
        None => unsafe { ns_app.setApplicationIconImage(None) },
        Some(tile) => {
            let data = NSData::with_bytes(tile.png);
            if let Some(image) = NSImage::initWithData(NSImage::alloc(), &data) {
                // SAFETY: a valid, retained image, on the main thread.
                unsafe { ns_app.setApplicationIconImage(Some(&image)) };
            }
        }
    }
}

#[cfg(not(target_os = "macos"))]
pub fn set_tile_now(_tile: Option<&'static Tile>) {}

/// The live answers to what a report needs to know. Built only inside
/// [`with_model`], so its questions are asked on the main thread.
pub struct NativeHost<'a>(&'a AppHandle);

impl Host for NativeHost<'_> {
    /// Cheap: one property read on `NSProcessInfo`.
    fn low_power(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            objc2_foundation::NSProcessInfo::processInfo().isLowPowerModeEnabled()
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }

    fn frontmost(&self) -> bool {
        self.0
            .webview_windows()
            .values()
            .any(|w| w.is_focused().unwrap_or(false))
    }

    fn attention_enabled(&self) -> bool {
        attention_allowed(&crate::harbard_home(), now_ms())
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Whether a new "Needs you" may bounce the Dock icon: "Notify in the
/// Dock" is on, "Do not disturb" is off and the coordination state allows it
/// (not Focused, Swamped, Away or Done for the day). The items still
/// reach the badge and Needs you; only the bounce waits.
pub fn attention_allowed(home: &std::path::Path, now_ms: u64) -> bool {
    let profile = home.join("profile.json");
    attention_enabled_in(&home.join("workspace.json"))
        && !quiet_in(&profile, now_ms)
        && state_allows(&profile, now_ms, "bounce")
}

/// Whether the desktop's own notifications may show: "Do not disturb" is off
/// and the state is not one of the quiet ones (Swamped, Away, Fin de
/// jornada).
pub fn alerts_allowed(home: &std::path::Path, now_ms: u64) -> bool {
    let profile = home.join("profile.json");
    !quiet_in(&profile, now_ms) && state_allows(&profile, now_ms, "alerts")
}

/// The alerts may show right now (doorbells, a lost bus).
pub fn alerts_now() -> bool {
    alerts_allowed(&crate::harbard_home(), now_ms())
}

/// What the away watch needs from `profile.json`: the state in force and
/// whether "Suggest Away" is on (missing or unreadable: on).
pub fn away_inputs(home: &std::path::Path, now_ms: u64) -> (String, bool) {
    let path = home.join("profile.json");
    let enabled = read_small(&path)
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|p| p.get("away_suggest").and_then(Value::as_bool))
        .unwrap_or(true);
    (state_in(&path, now_ms), enabled)
}

/// Milliseconds since the epoch, for the away watch.
pub fn now_ms_pub() -> u64 {
    now_ms()
}

/// The coordination state in `profile.json` (`harbard_core::profile`), while
/// it lasts; Available when missing, ended or unreadable.
fn state_in(path: &std::path::Path, now_ms: u64) -> String {
    read_small(path)
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|profile| profile.get("state").cloned())
        .filter(|state| match state.get("until_ms") {
            None | Some(Value::Null) => true,
            Some(until) => until.as_u64().is_some_and(|until| now_ms < until),
        })
        .and_then(|state| {
            state
                .get("id")
                .and_then(Value::as_str)
                .map(|id| state_from_file(id).to_string())
        })
        .unwrap_or_else(|| "available".to_string())
}

/// The state as an older `profile.json` spelled it. Exists only to read old
/// files; the shell does not link `harbard_core`, whose table this repeats.
fn state_from_file(stored: &str) -> &str {
    match stored {
        "disponible" => "available",
        "concentrado" => "focused",
        "saturado" => "swamped",
        "fuera" => "away",
        "revisando" => "reviewing",
        "fin" => "done",
        other => other,
    }
}

/// The same table as `harbard_core::profile::dock_rules`: Focused stops
/// only the bounce; Swamped, Away and Done for the day are quiet; anything
/// else is normal.
fn state_allows(path: &std::path::Path, now_ms: u64, what: &str) -> bool {
    match state_in(path, now_ms).as_str() {
        "focused" => what != "bounce",
        "swamped" | "away" | "done" => false,
        _ => true,
    }
}

/// "Do not disturb" from the operator's profile (`profile.json`, written by
/// serve, `harbard_core::profile`): on while `dnd.until_ms` is in the
/// future, or with no end. Missing, unreadable or malformed is off, so a
/// broken file can never leave the operator silently unreachable.
pub fn quiet_in(path: &std::path::Path, now_ms: u64) -> bool {
    let Some(dnd) = read_small(path)
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|profile| profile.get("dnd").cloned())
        .filter(|dnd| dnd.is_object())
    else {
        return false;
    };
    // Without an end (only a hand-edited file has none), it lapses two weeks
    // after it began, as in `harbard_core::profile::Dnd::end_ms`.
    let end = match dnd.get("until_ms") {
        None | Some(Value::Null) => dnd
            .get("since_ms")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            .saturating_add(DND_MAX_MS),
        Some(until) => until.as_u64().unwrap_or(0),
    };
    now_ms < end
}

/// "Do not disturb" never runs longer than this (`harbard_core::profile`).
const DND_MAX_MS: u64 = 14 * 24 * 3_600_000;

/// A small desk file, read only when it is a regular file of at most 64 KB:
/// a FIFO or a huge file in its place never blocks or floods the poll.
fn read_small(path: &std::path::Path) -> Option<String> {
    use std::io::Read;
    const MAX: u64 = 64 * 1024;
    let meta = std::fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX {
        return None;
    }
    let mut raw = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX)
        .read_to_string(&mut raw)
        .ok()?;
    Some(raw)
}

/// "Notify in the Dock", stored by serve with the theme in `workspace.json`
/// (`dock_attention`). Missing, unreadable or anything but `false` is on.
pub fn attention_enabled_in(path: &std::path::Path) -> bool {
    read_small(path)
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|desk| desk.get("dock_attention").and_then(Value::as_bool))
        .unwrap_or(true)
}

/// When the console last reported, for the heartbeat; `None` until it has.
static LAST_REPORT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
/// Whether the Dock already shows the lost heartbeat.
static SILENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Called by the shell's existing 4 s bus poll: no timer of its own. It
/// reaches the main thread only when the heartbeat is lost, not every tick.
pub fn check_heartbeat(app: &AppHandle) {
    use std::sync::atomic::Ordering;
    let last = *LAST_REPORT.lock().unwrap_or_else(|e| e.into_inner());
    if dock::heartbeat_expired(last, Instant::now()) && !SILENT.swap(true, Ordering::SeqCst) {
        eprintln!("harbard-desktop: no Dock status for 60 s; showing the outage ring");
        with_model(app, |model, _| model.set_silent(true));
    }
}

/// Show a validated report.
pub fn report(app: &AppHandle, status: Status) {
    *LAST_REPORT.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
    SILENT.store(false, std::sync::atomic::Ordering::SeqCst);
    with_model(app, move |model, host| {
        model.report(status, Instant::now(), host)
    });
}

/// The shell's poll saw the bus go away or come back.
pub fn set_offline(app: &AppHandle, offline: bool) {
    with_model(app, move |model, _| model.set_offline(offline));
}

/// `desk_set_status({ state, badge, needs?, event? })` from the console.
/// The body is parsed whole by [`dock::parse_status`], so what the tests
/// feed it is exactly what `invoke` sends.
#[tauri::command]
pub fn desk_set_status(app: AppHandle, request: tauri::ipc::Request<'_>) -> Result<(), String> {
    let tauri::ipc::InvokeBody::Json(body) = request.body() else {
        return Err("Dock status must be JSON.".to_string());
    };
    report(&app, dock::parse_status(body)?);
    Ok(())
}

/// `HARBARD_DOCK_DEMO=1`: every state and event once, then rest.
pub fn demo_enabled() -> bool {
    std::env::var("HARBARD_DOCK_DEMO").is_ok_and(|v| v == "1")
}

/// The demo runs only as its own bundle: under the installed app's id it
/// would meet that app on its single-instance socket and its window would
/// load the production console.
pub fn demo_may_run(identifier: &str) -> bool {
    identifier == DEMO_IDENTIFIER
}

/// Called first in `main()`: refuse the demo outside its bundle, before the
/// single-instance plugin or any console probe can run.
pub fn refuse_demo_outside_its_bundle(identifier: &str) {
    if demo_enabled() && !demo_may_run(identifier) {
        eprintln!(
            "harbard-desktop: HARBARD_DOCK_DEMO runs only in the {DEMO_IDENTIFIER} build \
             (just desktop-dock-demo); this is {identifier}."
        );
        std::process::exit(2);
    }
}

/// The demo's steps: the pause before each, and the body reported.
pub fn demo_steps() -> Vec<(Duration, Value)> {
    let step = |ms: u64, state: &str, badge: i64, needs: i64, event: Option<&str>| {
        let mut body = json!({ "state": state, "badge": badge, "needs": needs });
        if let Some(event) = event {
            body["event"] = json!(event);
        }
        (Duration::from_millis(ms), body)
    };
    vec![
        step(1_500, "idle", 0, 0, None),
        step(2_500, "working", 0, 0, None),
        step(2_500, "working", 1, 0, Some("message")),
        step(3_500, "needs", 2, 1, None),
        step(3_000, "needs", 5, 1, None),
        step(2_500, "blocked", 12, 1, None),
        step(2_500, "blocked", 120, 1, None),
        step(2_500, "working", 0, 0, Some("verified")),
        step(3_500, "connecting", 0, 0, None),
        step(2_500, "disconnected", 0, 0, None),
        step(2_500, "idle", 0, 0, None),
    ]
}

pub fn run_demo(app: AppHandle) {
    thread::spawn(move || {
        eprintln!("harbard-desktop: Dock demo, {} steps", demo_steps().len());
        for (pause, body) in demo_steps() {
            thread::sleep(pause);
            match dock::parse_status(&body) {
                Ok(status) => with_model(&app, move |model, host| {
                    let plan = model.report(status, Instant::now(), host);
                    eprintln!("harbard-desktop: demo {body} -> {plan:?}");
                    plan
                }),
                Err(e) => eprintln!("harbard-desktop: demo step refused: {e}"),
            }
        }
        eprintln!("harbard-desktop: Dock demo done");
    });
}

/// The single-instance socket `tauri-plugin-single-instance` binds on macOS
/// for a bundle id (its `socket_path`, without the `semver` feature, which
/// this app does not enable). Kept here so a test can pin that a demo build
/// and the installed app can never meet on one socket.
#[cfg(test)]
pub fn single_instance_socket(identifier: &str) -> String {
    format!("/tmp/{}_si.sock", identifier.replace(['.', '-'], "_"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dock::{LogoState, OneShot};

    #[test]
    fn an_old_profile_state_keeps_its_meaning() {
        assert_eq!(state_from_file("concentrado"), "focused");
        assert_eq!(state_from_file("fuera"), "away");
        assert_eq!(state_from_file("focused"), "focused");
        let dir = std::env::temp_dir().join(format!(
            "harbard-dock-old-{}-{}",
            std::process::id(),
            line!()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("profile.json");
        std::fs::write(&path, r#"{"state":{"id":"fuera","since_ms":1}}"#).unwrap();
        assert_eq!(state_in(&path, 2), "away");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_demo_cycles_every_state_and_event_and_ends_at_rest() {
        let steps = demo_steps();
        let parsed: Vec<Status> = steps
            .iter()
            .map(|(_, input)| dock::parse_status(input).expect("a valid demo step"))
            .collect();
        for state in LogoState::ALL {
            assert!(parsed.iter().any(|s| s.state == state), "{state:?}");
        }
        for event in [OneShot::Message, OneShot::Verified] {
            assert_eq!(parsed.iter().filter(|s| s.event == Some(event)).count(), 1);
        }
        assert!(parsed.iter().any(|s| s.badge > 99), "shows 99+");
        assert_eq!(parsed.last().map(|s| s.state), Some(LogoState::Idle));
        // Each event is at least the coalescing gap after the previous one.
        let mut since_event = Duration::MAX;
        for ((pause, _), status) in steps.iter().zip(&parsed) {
            since_event = since_event.saturating_add(*pause);
            if status.event.is_some() {
                assert!(since_event >= dock::ANIMATION_GAP);
                since_event = Duration::ZERO;
            }
        }
    }

    #[test]
    fn the_demo_bounces_once() {
        use crate::dock::Model;
        struct Away;
        impl Host for Away {
            fn low_power(&self) -> bool {
                false
            }
            fn frontmost(&self) -> bool {
                false
            }
            fn attention_enabled(&self) -> bool {
                true
            }
        }
        let mut model = Model::default();
        let mut at = Instant::now();
        let mut bounces = 0;
        for (pause, input) in demo_steps() {
            at += pause;
            bounces += usize::from(
                model
                    .report(dock::parse_status(&input).unwrap(), at, &Away)
                    .bounce,
            );
            model.animation_done();
        }
        assert_eq!(bounces, 1);
    }

    #[test]
    fn the_demo_build_has_its_own_bundle_id_socket_and_a_local_window() {
        let base: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        let demo: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.dockdemo.conf.json")).unwrap();
        let (installed, overlay) = (
            base["identifier"].as_str().unwrap(),
            demo["identifier"].as_str().unwrap(),
        );
        assert_eq!(installed, "com.harbard.desktop");
        assert_ne!(overlay, installed);
        assert_eq!(
            single_instance_socket(installed),
            "/tmp/com_harbard_desktop_si.sock"
        );
        assert_ne!(
            single_instance_socket(overlay),
            single_instance_socket(installed)
        );
        // `_` and `.` fold together: the overlay must differ by more than that.
        assert!(!overlay
            .replace(['.', '-'], "_")
            .eq(&installed.replace(['.', '-'], "_")));
        let window = &demo["app"]["windows"][0];
        assert_eq!(window["label"], "main");
        assert_eq!(window["visible"], false);
        assert!(
            !window["url"].as_str().unwrap().contains("://"),
            "the demo window loads a local page, never serve or the console"
        );
        // The semver feature would add the version to the socket name.
        assert!(!include_str!("../Cargo.toml").contains("features = [\"semver\"]"));
    }

    #[test]
    fn notify_en_el_dock_is_on_unless_the_desk_says_false() {
        let dir = std::env::temp_dir().join(format!("harbard-dock-pref-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("workspace.json");
        let _ = std::fs::remove_file(&path);
        assert!(attention_enabled_in(&path), "no desk file");
        std::fs::write(&path, "not json").unwrap();
        assert!(attention_enabled_in(&path));
        std::fs::write(&path, r#"{"theme":"jade"}"#).unwrap();
        assert!(attention_enabled_in(&path));
        std::fs::write(&path, r#"{"dock_attention":true}"#).unwrap();
        assert!(attention_enabled_in(&path));
        std::fs::write(&path, r#"{"dock_attention":false}"#).unwrap();
        assert!(!attention_enabled_in(&path));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn no_disturb_pauses_the_bounce_until_it_ends() {
        let dir = std::env::temp_dir().join(format!("harbard-dock-dnd-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let now = 1_793_000_000_000u64;
        assert!(attention_allowed(&dir, now), "no profile: available");
        let profile = dir.join("profile.json");
        std::fs::write(&profile, "{ broken").unwrap();
        assert!(
            attention_allowed(&dir, now),
            "a broken profile never silences harbard"
        );
        std::fs::write(
            &profile,
            format!(
                r#"{{"dnd":{{"since_ms":{now},"until_ms":{}}}}}"#,
                now + 60_000
            ),
        )
        .unwrap();
        assert!(!attention_allowed(&dir, now), "No molestar: no bounce");
        assert!(quiet_in(&profile, now + 59_999));
        assert!(attention_allowed(&dir, now + 60_000), "it ends on time");
        // "Notify in the Dock" off stays off whatever Do not disturb says.
        std::fs::write(dir.join("workspace.json"), r#"{"dock_attention":false}"#).unwrap();
        assert!(!attention_allowed(&dir, now + 60_000));
        // And through the model: the same rising "Needs you" bounces only
        // when attention is allowed.
        struct Away(bool);
        impl Host for Away {
            fn low_power(&self) -> bool {
                false
            }
            fn frontmost(&self) -> bool {
                false
            }
            fn attention_enabled(&self) -> bool {
                self.0
            }
        }
        let rise = |allowed: bool| {
            let mut model = Model::default();
            let at = Instant::now();
            let calm = json!({ "state": "idle", "badge": 0, "needs": 0 });
            let needs = json!({ "state": "needs", "badge": 1, "needs": 1 });
            model.report(dock::parse_status(&calm).unwrap(), at, &Away(allowed));
            model
                .report(
                    dock::parse_status(&needs).unwrap(),
                    at + Duration::from_secs(1),
                    &Away(allowed),
                )
                .bounce
        };
        assert!(rise(true));
        assert!(!rise(false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn each_coordination_state_holds_the_bounce_and_the_alerts_as_it_says() {
        let dir = std::env::temp_dir().join(format!("harbard-dock-state-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let now = 1_793_000_000_000u64;
        let profile = dir.join("profile.json");
        let set = |state: &str, until: Option<u64>| {
            let until = until.map_or(String::new(), |u| format!(r#","until_ms":{u}"#));
            std::fs::write(
                &profile,
                format!(r#"{{"state":{{"id":"{state}","since_ms":{now}{until}}}}}"#),
            )
            .unwrap();
        };
        // (bounce, alerts) per state.
        for (state, bounce, alerts) in [
            ("focused", false, true),
            ("swamped", false, false),
            ("away", false, false),
            ("reviewing", true, true),
            ("done", false, false),
        ] {
            set(state, None);
            assert_eq!(
                (attention_allowed(&dir, now), alerts_allowed(&dir, now)),
                (bounce, alerts),
                "{state}"
            );
        }
        // Done for the day is silent until its end, the start of the working hours.
        set("done", Some(now + 60_000));
        assert!(!alerts_allowed(&dir, now + 59_999));
        assert!(alerts_allowed(&dir, now + 60_000) && attention_allowed(&dir, now + 60_000));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_no_disturb_without_an_end_lapses_and_a_fifo_never_blocks_the_poll() {
        let dir = std::env::temp_dir().join(format!("harbard-dock-l6-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let now = 1_793_000_000_000u64;
        let profile = dir.join("profile.json");
        std::fs::write(&profile, r#"{"dnd":{}}"#).unwrap();
        assert!(!quiet_in(&profile, now), "no end and no start: long lapsed");
        std::fs::write(&profile, format!(r#"{{"dnd":{{"since_ms":{now}}}}}"#)).unwrap();
        assert!(quiet_in(&profile, now + 60_000));
        assert!(
            !quiet_in(&profile, now + 14 * 24 * 3_600_000),
            "two weeks at most"
        );
        // A FIFO in place of the file: the 4 s poll must not hang on it.
        std::fs::remove_file(&profile).unwrap();
        let status = std::process::Command::new("mkfifo")
            .arg(&profile)
            .status()
            .unwrap();
        assert!(status.success());
        let (tx, rx) = std::sync::mpsc::channel();
        let (p2, d2) = (profile.clone(), dir.clone());
        std::thread::spawn(move || {
            let _ = tx.send((quiet_in(&p2, now), attention_allowed(&d2, now)));
        });
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(2)),
            Ok((false, true)),
            "a FIFO is not a profile: available, and the read never blocks"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_away_watch_reads_its_state_and_switch_from_the_profile() {
        let dir = std::env::temp_dir().join(format!("harbard-away-in-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let now = 1_793_000_000_000u64;
        assert_eq!(
            away_inputs(&dir, now),
            ("available".to_string(), true),
            "no profile: on"
        );
        std::fs::write(
            dir.join("profile.json"),
            r#"{"away_suggest":false,"state":{"id":"fin","since_ms":1}}"#,
        )
        .unwrap();
        assert_eq!(away_inputs(&dir, now), ("done".to_string(), false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_command_has_its_permission_and_its_capability_grants_it() {
        let toml = include_str!("../permissions/desk-set-status.toml");
        assert!(toml.contains(r#"identifier = "allow-desk-set-status""#));
        assert!(toml.contains(r#"commands.allow = ["desk_set_status"]"#));
        let caps: serde_json::Value =
            serde_json::from_str(include_str!("../capabilities/dock-status.json")).unwrap();
        assert!(caps["permissions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p == "allow-desk-set-status"));
    }

    fn here(path: &str) -> String {
        let at = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path);
        std::fs::read_to_string(&at).unwrap_or_else(|e| panic!("{}: {e}", at.display()))
    }

    // The demo can
    // neither meet the installed app on its socket nor load serve or the
    // console, whichever build it is started from.
    #[test]
    fn the_demo_refuses_to_start_outside_its_own_bundle_before_single_instance_runs() {
        let main_rs = here("src/main.rs");
        let main_fn = &main_rs[main_rs.find("fn main()").unwrap()..];
        let guard = main_fn
            .find("refuse_demo_outside_its_bundle")
            .expect("main() checks the demo's bundle id");
        let plugin = main_fn.find("single_instance::init").unwrap();
        let probe = main_fn.find("console_base()").unwrap();
        assert!(
            guard < plugin,
            "the guard runs before single-instance can hand off"
        );
        assert!(guard < probe, "the guard runs before any console probe");
        let base: serde_json::Value = serde_json::from_str(&here("tauri.conf.json")).unwrap();
        let installed = base["identifier"].as_str().unwrap();
        let demo: serde_json::Value =
            serde_json::from_str(&here("tauri.dockdemo.conf.json")).unwrap();
        assert!(!demo_may_run(installed), "never as the installed app");
        assert!(demo_may_run(demo["identifier"].as_str().unwrap()));
    }

    // Only the main window may report the Dock status.
    #[test]
    fn only_the_main_window_may_set_the_dock_status() {
        let default: serde_json::Value =
            serde_json::from_str(&here("capabilities/default.json")).unwrap();
        assert!(
            !default["permissions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|p| p == "allow-desk-set-status"),
            "pulse must not hold it"
        );
        let dock: serde_json::Value =
            serde_json::from_str(&here("capabilities/dock-status.json")).unwrap();
        assert_eq!(dock["windows"], serde_json::json!(["main"]));
        assert_eq!(
            dock["permissions"],
            serde_json::json!(["allow-desk-set-status"])
        );
        assert_eq!(
            dock["remote"], default["remote"],
            "the same console origin only"
        );
        // The strip, in `pulse`, still opens Direct and Activity in
        // the main window through the default capability.
        assert!(default["windows"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w == "pulse"));
        for needed in ["allow-desk-open-main", "allow-desk-open-strip"] {
            assert!(
                default["permissions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|p| p == needed),
                "{needed}"
            );
        }
    }

    // The committed Assets.car was compiled from today's sources.
    #[test]
    fn the_committed_assets_car_matches_its_sources() {
        let stamp = here("icons/Assets.car.sources");
        let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../../assets/brand/harbard.icon");
        let current = crate::layered_stamp::stamp(&icon).unwrap();
        assert_eq!(stamp.trim(), current, "re-run scripts/render-brand.sh");
        assert!(current.starts_with("fnv1a64:") && current.len() == 24);
    }

    // WebKit suspends a hidden webview's timers by default
    // ("suspend"), so the console's hidden poll would never run. The main
    // window keeps working, throttled, while it is closed (hidden).
    #[test]
    fn the_main_window_keeps_its_timers_while_hidden() {
        for file in ["tauri.conf.json", "tauri.dockdemo.conf.json"] {
            let conf: serde_json::Value = serde_json::from_str(&here(file)).unwrap();
            let main = conf["app"]["windows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|w| w["label"] == "main")
                .unwrap()
                .clone();
            assert_eq!(main["backgroundThrottling"], "throttle", "{file}");
        }
    }
}
