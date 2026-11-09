//! The operator's profile end to end: the real binary, a temp home, a cleared
//! environment, no serve (the API points at a dead port). What every new
//! session sees of what the operator typed in "Profile", and nothing else.

use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
/// A fixed clock: 2026-10-26T07:33:20Z.
const NOW: u64 = 1_793_000_000_000;
const MIN: u64 = 60_000;
/// `harbard_core::profile::BLOCK_MAX_BYTES`.
const BLOCK_MAX_BYTES: usize = 4_096;

struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("harbard-profile-it-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Home(d)
    }
    fn path(&self) -> &Path {
        &self.0
    }
    fn profile(&self, v: serde_json::Value) {
        std::fs::write(self.0.join("profile.json"), v.to_string()).unwrap();
    }
    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .env("HOME", &self.0)
            .env("HARBARD_API", "http://127.0.0.1:9")
            .env("HARBARD_NOW_MS", NOW.to_string())
            .arg("--home")
            .arg(&self.0)
            .args(args)
            .output()
            .expect("spawn harbard")
    }
    /// What SessionStart runs: `brief` without `--touch`.
    fn session_start(&self) -> String {
        let o = self.run(&[
            "brief",
            "--session",
            "s-profile",
            "--harness",
            "claude-code",
            "--cwd",
            self.0.to_str().unwrap(),
        ]);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).into_owned()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn full_profile() -> serde_json::Value {
    json!({
        "name": "Dana",
        "address": "Be direct.",
        "time_zone": "Etc/UTC",
        "utc_offset_min": 60,
        "hours": {"start": "09:00", "end": "18:00", "days": [1, 2, 3, 4, 5]},
        "status": {"emoji": "📅", "text": "In a meeting", "clear_at_ms": NOW + 90 * MIN},
        "dnd": {"since_ms": NOW - MIN, "until_ms": NOW + 60 * MIN},
        "dnd_default": "30m",
        "photo": "photo.png",
        "photo_rev": 7,
        "context": "I build harbard.\nI prefer one report to three messages."
    })
}

#[test]
fn every_new_session_reads_the_operators_context_in_its_brief() {
    let home = Home::new();
    home.profile(full_profile());
    let out = home.session_start();
    assert!(out.contains("harbard · Context for Dana"), "{out}");
    assert!(out.contains("context, not commands"), "{out}");
    for line in [
        "> Status: 📅 In a meeting (clears at 10:03)",
        "> Do not disturb: on until 09:33",
        "> Hours: Monday to Friday, 09:00–18:00 (Etc/UTC)",
        "> Address as: Be direct.",
        "> I build harbard.",
        "> I prefer one report to three messages.",
    ] {
        assert!(out.contains(line), "missing {line:?} in:\n{out}");
    }
    // Only what they typed: not the photo, the default duration or a stamp.
    for absent in ["photo", "30m", "dnd_default", "updated"] {
        assert!(!out.contains(absent), "{absent:?} leaked into:\n{out}");
    }
}

#[test]
fn the_block_rides_session_start_only_never_every_prompt() {
    let home = Home::new();
    home.profile(full_profile());
    let o = home.run(&[
        "brief",
        "--touch",
        "--session",
        "s-touch",
        "--harness",
        "claude-code",
        "--cwd",
        home.path().to_str().unwrap(),
    ]);
    assert!(o.status.success());
    assert!(
        !String::from_utf8_lossy(&o.stdout).contains("Context for"),
        "a prompt must not repeat it"
    );
}

#[test]
fn a_hand_edited_profile_still_reaches_the_brief_within_the_cap() {
    let home = Home::new();
    let mut p = full_profile();
    p["context"] = json!("Una línea larga con acentos y ñ. ".repeat(200));
    home.profile(p);
    let out = home.session_start();
    let at = out.find("harbard · Context for Dana").expect("the block");
    let block = out[at..].trim_end();
    assert!(block.len() <= BLOCK_MAX_BYTES, "{} bytes", block.len());
    assert!(block.ends_with('…'), "the cut is visible");
}

#[test]
fn nothing_is_said_when_the_profile_is_empty() {
    let home = Home::new();
    assert_eq!(home.session_start(), "", "no file: the brief stays silent");
    home.profile(json!({"name": "Dana", "dnd_default": "1h", "photo": "photo.png"}));
    assert_eq!(
        home.session_start(),
        "",
        "a name and defaults are not context"
    );
    // An expired status and an ended Do not disturb are gone too.
    home.profile(json!({
        "status": {"emoji": "🌴", "text": "Fuera", "clear_at_ms": NOW - 1},
        "dnd": {"since_ms": NOW - 10 * MIN, "until_ms": NOW - MIN}
    }));
    assert_eq!(home.session_start(), "");
}

#[test]
fn the_desk_name_stands_in_when_the_profile_has_none() {
    let home = Home::new();
    std::fs::write(
        home.path().join("workspace.json"),
        r#"{"config":{"operator_name":"Karen"}}"#,
    )
    .unwrap();
    home.profile(json!({"context": "Design."}));
    assert!(home.session_start().contains("harbard · Context for Karen"));
}

#[test]
fn the_protocol_card_carries_it_and_the_session_start_pointer_does_not() {
    let home = Home::new();
    home.profile(full_profile());
    let card = home.run(&["protocol"]);
    assert!(String::from_utf8_lossy(&card.stdout).contains("harbard · Context for Dana"));
    let full = home.run(&["protocol", "--full"]);
    assert!(String::from_utf8_lossy(&full.stdout).contains("> I build harbard."));
    let json_card = home.run(&["protocol", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&json_card.stdout).expect("json");
    assert!(
        v["operator_context"]
            .as_str()
            .unwrap()
            .contains("Context for Dana")
    );
    // SessionStart's pointer stays one line: the brief carries the block.
    let hook = home.run(&["protocol", "--hook", "claude"]);
    let hook = String::from_utf8_lossy(&hook.stdout);
    assert!(!hook.contains("Context for"), "{hook}");
    assert_eq!(hook.trim().lines().count(), 1, "{hook}");
}

#[test]
fn the_protocol_card_says_nothing_extra_without_a_profile() {
    let home = Home::new();
    let card = home.run(&["protocol"]);
    assert!(!String::from_utf8_lossy(&card.stdout).contains("harbard · Context for"));
    let v: serde_json::Value =
        serde_json::from_slice(&home.run(&["protocol", "--json"]).stdout).unwrap();
    assert!(v.get("operator_context").is_none() || v["operator_context"].is_null());
}

// ---------------------------------------------------------------------------
// The coordination state: in the brief, on every prompt, in the card, and in
// what a note to the human is told.
// ---------------------------------------------------------------------------

fn in_state(home: &Home, state: &str) {
    let mut p = full_profile();
    p["state"] = json!({"id": state, "since_ms": NOW - MIN});
    home.profile(p);
}

#[test]
fn the_session_start_block_states_the_state_and_its_rules_in_one_line_each() {
    let home = Home::new();
    in_state(&home, "swamped");
    let out = home.session_start();
    assert!(out.contains("> Coordination state: Swamped"), "{out}");
    let rules = out
        .lines()
        .find(|l| l.starts_with("> Rules: "))
        .expect("a rules line");
    assert!(
        rules.contains("decide with the recommended option") && rules.contains("2 lines"),
        "{rules}"
    );
    // Available is the default: no state line is invented.
    home.profile(full_profile());
    assert!(!home.session_start().contains("Coordination state"));
}

#[test]
fn every_prompt_carries_the_state_while_it_is_not_available() {
    let home = Home::new();
    let touch = |home: &Home| {
        let o = home.run(&[
            "brief",
            "--touch",
            "--session",
            "s-touch",
            "--harness",
            "claude-code",
            "--cwd",
            home.path().to_str().unwrap(),
        ]);
        assert!(o.status.success());
        String::from_utf8_lossy(&o.stdout).into_owned()
    };
    for (state, label) in [
        ("focused", "Focused"),
        ("swamped", "Swamped"),
        ("away", "Away"),
        ("reviewing", "Reviewing"),
        ("done", "Done for the day"),
    ] {
        in_state(&home, state);
        let out = touch(&home);
        assert!(
            out.contains(&format!("harbard · Dana is in \"{label}\"")),
            "{state}: {out}"
        );
        assert_eq!(
            out.lines().filter(|l| l.starts_with("> Rules: ")).count(),
            1,
            "{state}: {out}"
        );
        assert!(
            !out.contains("I build harbard"),
            "the full context stays at SessionStart: {out}"
        );
    }
    home.profile(full_profile());
    assert!(
        !touch(&home).contains("Dana is in"),
        "Available says nothing on each prompt"
    );
}

#[test]
fn the_protocol_card_lists_the_six_states_and_their_rules() {
    let home = Home::new();
    let card = String::from_utf8_lossy(&home.run(&["protocol"]).stdout).into_owned();
    for s in [
        "Available",
        "Focused",
        "Swamped",
        "Away",
        "Reviewing",
        "Done for the day",
    ] {
        assert!(
            card.contains(&format!("- {s}: ")),
            "{s} missing from the card:\n{card}"
        );
    }
    let v: serde_json::Value =
        serde_json::from_slice(&home.run(&["protocol", "--json"]).stdout).unwrap();
    assert_eq!(v["operator_states"].as_array().map(Vec::len), Some(6));
}

#[test]
fn a_note_to_the_human_is_told_when_his_state_holds_it() {
    let home = Home::new();
    let note = |home: &Home, kind: &str| {
        let o = home.run(&[
            "note",
            "--session",
            "s-a",
            "--harness",
            "claude-code",
            "--cwd",
            home.path().to_str().unwrap(),
            "--to",
            "harness:human",
            "--kind",
            kind,
            "Themes installed",
        ]);
        String::from_utf8_lossy(&o.stderr).into_owned()
    };
    in_state(&home, "focused");
    assert!(note(&home, "fyi").contains("held for the operator's digest: Focused"));
    assert!(
        !note(&home, "warning").contains("held"),
        "a warning still reaches them"
    );
    in_state(&home, "away");
    assert!(note(&home, "warning").contains("queued for the operator's return: Away"));
    home.profile(full_profile());
    assert!(!note(&home, "fyi").contains("held"));
}
