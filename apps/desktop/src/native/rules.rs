//! When the desktop may call for the operator: the bounce, the alerts and
//! the away suggestion, from the operator's own files (`profile.json`,
//! `workspace.json`), read where serve writes them.
//!
//! The same table as `harbard_core::profile::dock_rules`: Focused stops only
//! the bounce; Swamped, Away and Done for the day are quiet; anything else
//! is normal. "Do not disturb" holds both. A missing or broken file never
//! silences harbard.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// The store's root (`HARBARD_HOME`, else `~/.harbard`).
pub fn home() -> PathBuf {
    harbard_core::store::Store::resolve().root().to_path_buf()
}

pub fn now_ms() -> u64 {
    harbard_core::clock::now_ms()
}

/// Whether a new "Needs you" may bounce the Dock icon: "Notify in the
/// Dock" is on, "Do not disturb" is off and the coordination state allows it
/// (not Focused, Swamped, Away or Done for the day). The items still reach
/// the badge and Needs you; only the bounce waits.
pub fn attention_allowed(home: &Path, now_ms: u64) -> bool {
    let profile = home.join("profile.json");
    attention_enabled_in(&home.join("workspace.json"))
        && !quiet_in(&profile, now_ms)
        && state_allows(&profile, now_ms, "bounce")
}

/// Whether the desktop's own notifications may show: "Do not disturb" is off
/// and the state is not one of the quiet ones (Swamped, Away, Done for the
/// day).
pub fn alerts_allowed(home: &Path, now_ms: u64) -> bool {
    let profile = home.join("profile.json");
    !quiet_in(&profile, now_ms) && state_allows(&profile, now_ms, "alerts")
}

/// The alerts may show right now (doorbells, a lost bus).
pub fn alerts_now() -> bool {
    alerts_allowed(&home(), now_ms())
}

/// The one-shots may play on the Dock right now (`profile.oneShots` in the
/// console): not under "Do not disturb" nor in a quiet state.
pub fn one_shots_now() -> bool {
    alerts_now()
}

/// What the away watch needs from `profile.json`: the state in force and
/// whether "Suggest Away" is on (missing or unreadable: on).
pub fn away_inputs(home: &Path, now_ms: u64) -> (String, bool) {
    let path = home.join("profile.json");
    let enabled = read_small(&path)
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|p| p.get("away_suggest").and_then(Value::as_bool))
        .unwrap_or(true);
    (state_in(&path, now_ms), enabled)
}

/// The coordination state in `profile.json`, while it lasts; Available when
/// missing, ended or unreadable.
fn state_in(path: &Path, now_ms: u64) -> String {
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

/// The state as an older `profile.json` spelled it.
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

fn state_allows(path: &Path, now_ms: u64, what: &str) -> bool {
    match state_in(path, now_ms).as_str() {
        "focused" => what != "bounce",
        "swamped" | "away" | "done" => false,
        _ => true,
    }
}

/// "Do not disturb" never runs longer than this (`harbard_core::profile`).
const DND_MAX_MS: u64 = 14 * 24 * 3_600_000;

/// "Do not disturb": on while `dnd.until_ms` is in the future, or with no
/// end for two weeks after it began. Missing, unreadable or malformed is off.
pub fn quiet_in(path: &Path, now_ms: u64) -> bool {
    let Some(dnd) = read_small(path)
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|profile| profile.get("dnd").cloned())
        .filter(|dnd| dnd.is_object())
    else {
        return false;
    };
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

/// A small desk file, read only when it is a regular file of at most 64 KB:
/// a FIFO or a huge file in its place never blocks or floods a read.
pub fn read_small(path: &Path) -> Option<String> {
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
pub fn attention_enabled_in(path: &Path) -> bool {
    read_small(path)
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|desk| desk.get("dock_attention").and_then(Value::as_bool))
        .unwrap_or(true)
}

/// How an `@` names the operator when no name is configured anywhere.
pub const DEFAULT_OPERATOR_HANDLE: &str = "operator";

/// How an `@` names the operator: Profile's name, else the desk's
/// `config.operator_name`, else [`DEFAULT_OPERATOR_HANDLE`]; first word,
/// lower case.
pub fn operator_handle(home: &Path) -> String {
    let read = |file: &str| read_small(&home.join(file)).and_then(|raw| serde_json::from_str::<Value>(&raw).ok());
    read("profile.json")
        .and_then(|p| p.get("name").and_then(Value::as_str).map(str::to_string))
        .or_else(|| {
            read("workspace.json").and_then(|d| {
                d.get("config")
                    .and_then(|c| c.get("operator_name"))
                    .and_then(Value::as_str)
                    .map(str::to_string)
            })
        })
        .and_then(|name| name.split_whitespace().next().map(str::to_lowercase))
        .unwrap_or_else(|| DEFAULT_OPERATOR_HANDLE.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("harbard-native-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn an_old_profile_state_keeps_its_meaning() {
        assert_eq!(state_from_file("concentrado"), "focused");
        assert_eq!(state_from_file("fuera"), "away");
        assert_eq!(state_from_file("focused"), "focused");
        let d = dir("old");
        let path = d.join("profile.json");
        std::fs::write(&path, r#"{"state":{"id":"fuera","since_ms":1}}"#).unwrap();
        assert_eq!(state_in(&path, 2), "away");
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn notify_in_the_dock_is_on_unless_the_desk_says_false() {
        let d = dir("pref");
        let path = d.join("workspace.json");
        assert!(attention_enabled_in(&path), "no desk file");
        std::fs::write(&path, "not json").unwrap();
        assert!(attention_enabled_in(&path));
        std::fs::write(&path, r#"{"theme":"jade"}"#).unwrap();
        assert!(attention_enabled_in(&path));
        std::fs::write(&path, r#"{"dock_attention":true}"#).unwrap();
        assert!(attention_enabled_in(&path));
        std::fs::write(&path, r#"{"dock_attention":false}"#).unwrap();
        assert!(!attention_enabled_in(&path));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn no_disturb_pauses_the_bounce_until_it_ends() {
        let d = dir("dnd");
        let now = 1_793_000_000_000u64;
        assert!(attention_allowed(&d, now), "no profile: available");
        let profile = d.join("profile.json");
        std::fs::write(&profile, "{ broken").unwrap();
        assert!(attention_allowed(&d, now), "a broken profile never silences harbard");
        std::fs::write(
            &profile,
            format!(r#"{{"dnd":{{"since_ms":{now},"until_ms":{}}}}}"#, now + 60_000),
        )
        .unwrap();
        assert!(!attention_allowed(&d, now), "Do not disturb: no bounce");
        assert!(quiet_in(&profile, now + 59_999));
        assert!(attention_allowed(&d, now + 60_000), "it ends on time");
        std::fs::write(d.join("workspace.json"), r#"{"dock_attention":false}"#).unwrap();
        assert!(!attention_allowed(&d, now + 60_000));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn each_coordination_state_holds_the_bounce_and_the_alerts_as_it_says() {
        let d = dir("state");
        let now = 1_793_000_000_000u64;
        let profile = d.join("profile.json");
        let set = |state: &str, until: Option<u64>| {
            let until = until.map_or(String::new(), |u| format!(r#","until_ms":{u}"#));
            std::fs::write(
                &profile,
                format!(r#"{{"state":{{"id":"{state}","since_ms":{now}{until}}}}}"#),
            )
            .unwrap();
        };
        for (state, bounce, alerts) in [
            ("focused", false, true),
            ("swamped", false, false),
            ("away", false, false),
            ("reviewing", true, true),
            ("done", false, false),
        ] {
            set(state, None);
            assert_eq!(
                (attention_allowed(&d, now), alerts_allowed(&d, now)),
                (bounce, alerts),
                "{state}"
            );
        }
        set("done", Some(now + 60_000));
        assert!(!alerts_allowed(&d, now + 59_999));
        assert!(alerts_allowed(&d, now + 60_000) && attention_allowed(&d, now + 60_000));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn a_no_disturb_without_an_end_lapses_and_a_fifo_never_blocks() {
        let d = dir("lapse");
        let now = 1_793_000_000_000u64;
        let profile = d.join("profile.json");
        std::fs::write(&profile, r#"{"dnd":{}}"#).unwrap();
        assert!(!quiet_in(&profile, now), "no end and no start: long lapsed");
        std::fs::write(&profile, format!(r#"{{"dnd":{{"since_ms":{now}}}}}"#)).unwrap();
        assert!(quiet_in(&profile, now + 60_000));
        assert!(!quiet_in(&profile, now + 14 * 24 * 3_600_000), "two weeks at most");
        std::fs::remove_file(&profile).unwrap();
        let _fork = crate::native::FORK_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let status = std::process::Command::new("mkfifo").arg(&profile).status().unwrap();
        assert!(status.success());
        let (tx, rx) = std::sync::mpsc::channel();
        let (p2, d2) = (profile.clone(), d.clone());
        std::thread::spawn(move || {
            let _ = tx.send((quiet_in(&p2, now), attention_allowed(&d2, now)));
        });
        assert_eq!(rx.recv_timeout(Duration::from_secs(2)), Ok((false, true)));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn the_away_watch_reads_its_state_and_switch_from_the_profile() {
        let d = dir("away-in");
        let now = 1_793_000_000_000u64;
        assert_eq!(away_inputs(&d, now), ("available".to_string(), true), "no profile: on");
        std::fs::write(
            d.join("profile.json"),
            r#"{"away_suggest":false,"state":{"id":"fin","since_ms":1}}"#,
        )
        .unwrap();
        assert_eq!(away_inputs(&d, now), ("done".to_string(), false));
        std::fs::remove_dir_all(&d).ok();
    }

    #[test]
    fn the_operator_handle_is_the_first_word_of_a_configured_name() {
        let d = dir("handle");
        assert_eq!(operator_handle(&d), DEFAULT_OPERATOR_HANDLE);
        std::fs::write(d.join("workspace.json"), r#"{"config":{"operator_name":"Dana Sky"}}"#).unwrap();
        assert_eq!(operator_handle(&d), "dana");
        std::fs::write(d.join("profile.json"), r#"{"name":"Lee"}"#).unwrap();
        assert_eq!(operator_handle(&d), "lee");
        std::fs::remove_dir_all(&d).ok();
    }
}
