//! Regression coverage through the real CLI, with isolated stores only.

use harbard_core::Store;
use harbard_core::clock::{now_ms, rfc3339_utc};
use harbard_core::cursor::{BriefState, write_brief_state};
use serde_json::{Value, json};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const DAY: u64 = 86_400_000;

struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        Self(std::env::temp_dir().join(format!(
            "harbard-ls-regression-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        )))
    }

    fn event(&self, session: &str, ms: u64) {
        fs::create_dir_all(self.0.join("events")).unwrap();
        let ts = rfc3339_utc(ms);
        let date = ts[..10].replace('-', "");
        let event = json!({
            "v": 1, "id": ulid::Ulid::from_parts(ms, 0).to_string(), "ts": ts,
            "event": "hello", "session": session, "harness": "claude-code",
            "host": "test-host", "cwd": self.0, "repo": self.0,
            "branch": "main", "role": "main", "text": "Test session."
        });
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.0.join("events").join(format!("events-{date}.jsonl")))
            .unwrap();
        writeln!(file, "{event}").unwrap();
    }

    fn run(&self, args: &[&str]) -> Output {
        let output = Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .args(args)
            .args(["--home", self.0.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(output.status.success(), "{:?}", output);
        output
    }

    fn ls(&self, all: bool) -> Vec<Value> {
        let args = if all {
            vec!["ls", "--json", "--all"]
        } else {
            vec!["ls", "--json"]
        };
        serde_json::from_slice(&self.run(&args).stdout).unwrap()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn ls_default_only_reads_today_and_yesterday_even_with_an_old_sessions_fresh_cursor() {
    let home = Home::new();
    let now = now_ms();
    home.event("ancient", now - 96 * DAY);
    home.event("old-with-fresh-cursor", now - 3 * DAY);
    home.event("yesterday", now - DAY);
    home.event("today", now);
    write_brief_state(
        &Store::at(&home.0),
        "old-with-fresh-cursor",
        &BriefState {
            hb: now,
            ..Default::default()
        },
    )
    .unwrap();

    let recent = home.ls(false);
    let sessions: Vec<_> = recent
        .iter()
        .map(|row| row["session"].as_str().unwrap())
        .collect();
    assert_eq!(sessions, ["yesterday", "today"]);
    assert_eq!(home.ls(true).len(), 4, "--all keeps retained history");
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn ls_and_brief_distinguish_a_quiet_running_process_from_recent_activity() {
    use harbard_core::{PidChecker, SystemPidChecker};

    let home = Home::new();
    let now = now_ms();
    home.event("prompt-observer", now - DAY);
    home.event("quiet", now - 23 * 60 * 60 * 1000);
    home.event("active", now);
    let signal = now - 22 * 60 * 60 * 1000;
    let store = Store::at(&home.0);
    let pid = std::process::id();
    let start = SystemPidChecker
        .start_time(pid)
        .expect("test process start marker");
    let bind = |hb, marker| {
        write_brief_state(
            &store,
            "quiet",
            &BriefState {
                hb,
                supervisor_pid: Some(pid),
                supervisor_start: Some(marker),
                ..Default::default()
            },
        )
        .unwrap();
    };
    bind(signal, start);

    let rows = home.ls(false);
    let quiet = rows.iter().find(|row| row["session"] == "quiet").unwrap();
    assert_eq!(quiet["liveness"], "live", "process remains addressable");
    assert_eq!(
        quiet["idle"], true,
        "a running PID does not prove recent activity"
    );
    assert_eq!(quiet["last_signal_ms"], signal);
    let text = String::from_utf8(home.run(&["ls"]).stdout).unwrap();
    assert!(
        text.lines()
            .any(|line| line.starts_with("idle ") && line.contains("quiet")),
        "{text}"
    );
    assert!(
        text.contains("22h ago"),
        "age follows the cursor, not just the event: {text}"
    );

    let observe = || {
        String::from_utf8(
            home.run(&[
                "brief",
                "--session",
                "observer",
                "--harness",
                "codex",
                "--cwd",
                home.0.to_str().unwrap(),
            ])
            .stdout,
        )
        .unwrap()
    };
    // Other sessions being alive asks nothing of the reader.
    let silent = observe();
    assert!(silent.is_empty(), "{silent}");
    // A note does, and the digest that carries it counts live and idle apart.
    home.run(&[
        "note",
        "The staging deploy is paused until noon.",
        "--session",
        "active",
        "--harness",
        "claude-code",
        "--cwd",
        home.0.to_str().unwrap(),
    ]);
    let brief = observe();
    assert!(brief.contains("1 live · 1 idle"), "{brief}");
    assert!(brief.contains("> idle ["), "{brief}");
    let touched = String::from_utf8(
        home.run(&[
            "brief",
            "--touch",
            "--session",
            "prompt-observer",
            "--harness",
            "codex",
            "--cwd",
            home.0.to_str().unwrap(),
        ])
        .stdout,
    )
    .unwrap();
    assert!(touched.contains("1 live · 1 idle"), "{touched}");

    home.run(&[
        "note",
        "--kind",
        "handoff",
        "--to",
        "session:quiet",
        "Please review the result.",
        "--session",
        "observer",
        "--harness",
        "codex",
        "--cwd",
        home.0.to_str().unwrap(),
    ]);

    // A fresh hook signal is active even if the last event is from yesterday.
    bind(now_ms(), start);
    let rows = home.ls(false);
    let quiet = rows.iter().find(|row| row["session"] == "quiet").unwrap();
    assert_eq!(quiet["liveness"], "live");
    assert_ne!(quiet["idle"], true);

    // PID reuse cannot turn an expired signal into process-bound presence.
    bind(signal, start.wrapping_add(1));
    let rows = home.ls(false);
    let quiet = rows.iter().find(|row| row["session"] == "quiet").unwrap();
    assert_eq!(quiet["liveness"], "stale");
    assert_ne!(quiet["idle"], true);
}
