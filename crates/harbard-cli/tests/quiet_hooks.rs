//! The hook plane speaks only when the session has something to act on.
//!
//! Each test drives the verbs the way `harbard install` wires them — hook JSON
//! on stdin, `--hook <harness>` — against an isolated store, a fresh `HOME` and
//! an injected clock, so a one-hour session runs in milliseconds.

use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const MINUTE: u64 = 60_000;
const HARNESSES: [&str; 3] = ["claude-code", "codex", "cursor"];

struct Fixture {
    root: PathBuf,
    t0: u64,
}

impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "harbard-quiet-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        for dir in ["store", "home", "me", "peer"] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        Self {
            root,
            t0: harbard_core::clock::now_ms(),
        }
    }

    fn path(&self, dir: &str) -> PathBuf {
        self.root.join(dir)
    }

    fn command(&self, minute: u64) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_harbard"));
        cmd.env_clear()
            .env("HOME", self.path("home"))
            .env("HARBARD_NOW_MS", (self.t0 + minute * MINUTE).to_string())
            .arg("--home")
            .arg(self.path("store"));
        cmd
    }

    /// One hook invocation for session `me`, as the harness makes it.
    fn hook(&self, harness: &str, minute: u64, args: &[&str]) -> String {
        let payload = serde_json::json!({
            "session_id": "me",
            "cwd": self.path("me"),
        });
        let mut child = self
            .command(minute)
            .args(args)
            .args(["--hook", harness])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn harbard");
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(out.status.success(), "{args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap()
    }

    /// Everything SessionStart injects, in wiring order.
    fn session_start(&self, harness: &str, minute: u64) -> String {
        ["hello", "protocol", "brief"]
            .iter()
            .map(|verb| self.hook(harness, minute, &[verb]))
            .collect()
    }

    fn prompt(&self, harness: &str, minute: u64) -> String {
        self.hook(harness, minute, &["brief", "--touch"])
    }

    /// A busy session in another checkout.
    fn peer(&self, minute: u64, args: &[&str]) -> Output {
        let out = self
            .command(minute)
            .args(args)
            .args(["--session", "peer", "--harness", "codex", "--cwd"])
            .arg(self.path("peer"))
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}: {out:?}");
        out
    }

    /// An operator note as `POST /api/v1/notes` writes it; the CLI refuses
    /// to impersonate the operator, so it goes straight into the segment.
    fn operator_note(&self, minute: u64, text: &str) {
        let ms = self.t0 + minute * MINUTE;
        let ts = harbard_core::clock::rfc3339_utc(ms);
        let event = serde_json::json!({
            "v": 1, "id": ulid::Ulid::from_parts(ms, 1).to_string(), "ts": ts,
            "event": "note", "session": "operator@h", "harness": "human",
            "role": "operator", "host": "h", "cwd": "", "text": text,
            "to": { "kind": "session", "value": "me" },
        });
        let events = self.path("store").join("events");
        std::fs::create_dir_all(&events).unwrap();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(events.join(format!("events-{}.jsonl", ts[..10].replace('-', ""))))
            .unwrap();
        writeln!(file, "{event}").unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// The text a hook injects, unwrapping Cursor's `additional_context` envelope.
fn injected(harness: &str, stdout: &str) -> String {
    if harness != "cursor" {
        return stdout.to_string();
    }
    stdout
        .lines()
        .map(|line| {
            let v: serde_json::Value = serde_json::from_str(line).expect("cursor envelope");
            v["additional_context"].as_str().unwrap().to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn assert_no_nags(text: &str) {
    for nag in [
        "name_required",
        "intent_stale",
        "harbard name",
        "harbard intent",
    ] {
        assert!(!text.contains(nag), "nag {nag:?} in {text:?}");
    }
}

#[test]
fn a_quiet_session_hears_one_line_at_start_and_nothing_on_any_prompt() {
    for harness in HARNESSES {
        let f = Fixture::new();
        f.peer(0, &["hello", "--intent", "refactoring the store"]);

        let start = injected(harness, &f.session_start(harness, 0));
        assert_eq!(start.lines().count(), 1, "{harness}: {start:?}");
        assert!(start.contains("harbard protocol --full"), "{start:?}");
        assert!(start.len() < 200, "{harness}: {} bytes", start.len());
        assert_no_nags(&start);

        // Past every 20-minute nudge interval, with a peer changing its intent
        // before each prompt: its activity is not this session's business.
        for minute in [1, 21, 42, 63] {
            f.peer(minute - 1, &["intent", &format!("store step {minute}")]);
            let out = f.prompt(harness, minute);
            assert!(out.is_empty(), "{harness} prompt at +{minute}m: {out:?}");
        }
    }
}

#[test]
fn the_card_stays_available_on_demand() {
    let f = Fixture::new();
    let full = f.hook("claude-code", 0, &["protocol", "--full"]);
    assert!(full.contains("## Identity and mission"), "{full}");
    let compact = String::from_utf8(f.command(0).arg("protocol").output().unwrap().stdout).unwrap();
    assert!(
        compact.starts_with("harbard · protocol (compact)"),
        "{compact}"
    );
}

#[test]
fn a_starting_subagent_is_told_who_reads_its_reports() {
    let f = Fixture::new();
    let out = f.hook("claude-code", 0, &["protocol", "--subagent"]);
    let v: serde_json::Value = serde_json::from_str(out.trim()).expect("one JSON envelope");
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "SubagentStart");
    let told = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(
        told.contains("default English") && told.contains("LABEL"),
        "{told}"
    );
}

#[test]
fn an_addressed_warning_reaches_the_next_prompt_on_every_harness() {
    for harness in HARNESSES {
        let f = Fixture::new();
        f.session_start(harness, 0);
        f.peer(
            5,
            &[
                "note",
                "--kind",
                "warning",
                "--to",
                "session:me",
                "The dev database is being restored; hold your migration.",
            ],
        );
        let out = injected(harness, &f.prompt(harness, 6));
        assert!(out.contains("hold your migration"), "{harness}: {out:?}");
        assert!(out.contains("1 unread"), "{harness}: {out:?}");
        assert_no_nags(&out);
        // Delivered once.
        assert!(f.prompt(harness, 7).is_empty(), "{harness} repeated");
    }
}

#[test]
fn an_operator_note_reaches_the_next_prompt() {
    let f = Fixture::new();
    f.session_start("claude-code", 0);
    assert!(f.prompt("claude-code", 1).is_empty());
    f.operator_note(2, "Come look at the console before you merge.");
    let out = f.prompt("claude-code", 3);
    assert!(out.contains("Come look at the console"), "{out:?}");
    assert_no_nags(&out);
}

#[test]
fn an_owed_answer_is_named_until_it_is_given() {
    let f = Fixture::new();
    f.session_start("claude-code", 0);
    f.peer(
        1,
        &[
            "note",
            "--kind",
            "handoff",
            "--to",
            "session:me",
            "Please take over the release notes.",
        ],
    );
    let first = f.prompt("claude-code", 2);
    assert!(first.contains("take over the release notes"), "{first:?}");
    assert!(first.contains("ack_pending"), "{first:?}");
    // Throttled inside the interval, said again after it.
    assert!(f.prompt("claude-code", 3).is_empty());
    assert!(f.prompt("claude-code", 25).contains("ack_pending"));

    f.hook(
        "claude-code",
        26,
        &["ack", "last", "--note", "Taking the release notes now."],
    );
    assert!(f.prompt("claude-code", 50).is_empty());
}

#[test]
fn unhealthy_hooks_are_said_once_at_session_start_and_never_per_prompt() {
    let f = Fixture::new();
    // A Claude config that exists but carries none of the managed hooks.
    let claude = f.path("home").join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    std::fs::write(claude.join("settings.json"), "{}").unwrap();

    let start = f.session_start("claude-code", 0);
    assert!(start.contains("hooks_unhealthy"), "{start:?}");
    for minute in [1, 21, 42] {
        let out = f.prompt("claude-code", minute);
        assert!(!out.contains("hooks_unhealthy"), "+{minute}m: {out:?}");
    }
    // A resumed session starts again; the one-shot holds.
    let resumed = f.session_start("claude-code", 60);
    assert!(!resumed.contains("hooks_unhealthy"), "{resumed:?}");
}

#[test]
fn a_cursor_tool_call_carries_neither_nags_nor_standing_conditions() {
    let f = Fixture::new();
    let claude = f.path("home").join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    std::fs::write(claude.join("settings.json"), "{}").unwrap();
    f.hook("cursor", 0, &["hello"]);
    for minute in [1, 21, 42] {
        let out = f.hook("cursor", minute, &["brief", "--inline"]);
        assert!(out.is_empty(), "inline at +{minute}m: {out:?}");
    }
}
