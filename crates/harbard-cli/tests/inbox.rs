//! Inbox delivery uses the identity exported to Bash tools and the same
//! session boundary/high-water marks as harness hooks. All homes are isolated.

use harbard_core::Event;
use harbard_core::cursor;
use harbard_core::store::Store;
use serde_json::Value;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const READER: &str = "22222222-2222-4222-8222-222222222222";
const SENDER: &str = "11111111-1111-4111-8111-111111111111";

struct Bench {
    home: PathBuf,
    store: Store,
}

impl Bench {
    fn new() -> Self {
        let home = std::env::temp_dir().join(format!(
            "harbard-inbox-it-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&home).unwrap();
        let store = Store::at(home.join(".harbard"));
        Self { home, store }
    }

    // Real Bash export, not a flag standing in for the tool's environment.
    // No profiles or inherited variables can resolve a real user home.
    fn run(&self, export: Option<(&str, &str)>, args: &[&str], stdin: Option<&str>) -> Output {
        let mut cmd = Command::new("/bin/bash");
        cmd.env_clear()
            .env("HOME", &self.home)
            .env("PATH", "/usr/bin:/bin")
            .current_dir(&self.home)
            .args(["--noprofile", "--norc", "-c"]);
        if let Some((key, value)) = export {
            cmd.args([
                "export \"$1=$2\"; shift 2; exec \"$@\"",
                "inbox-test",
                key,
                value,
            ]);
        } else {
            cmd.args(["exec \"$@\"", "inbox-test"]);
        }
        cmd.arg(env!("CARGO_BIN_EXE_harbard"))
            .arg("--home")
            .arg(self.home.join(".harbard"))
            .arg("--cwd")
            .arg(&self.home)
            .arg("--repo")
            .arg(&self.home)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        if let Some(payload) = stdin {
            child
                .stdin
                .take()
                .unwrap()
                .write_all(payload.as_bytes())
                .unwrap();
        } else {
            drop(child.stdin.take());
        }
        child.wait_with_output().unwrap()
    }

    fn hello(&self, session: &str) -> Value {
        json(&self.run(
            None,
            &["hello", "--hook", "claude-code", "--json"],
            Some(&self.payload(session)),
        ))
    }

    fn payload(&self, session: &str) -> String {
        serde_json::json!({"session_id": session, "cwd": self.home, "stop_hook_active": false})
            .to_string()
    }

    fn note(&self, to: &str, text: &str) -> Event {
        let out = self.run(
            None,
            &[
                "note",
                text,
                "--kind",
                "warning",
                "--to",
                to,
                "--session",
                SENDER,
                "--harness",
                "codex",
                "--json",
            ],
            None,
        );
        serde_json::from_value(json(&out)).unwrap()
    }

    fn inbox(&self, export: Option<(&str, &str)>, extra: &[&str]) -> Value {
        let mut args = vec!["inbox", "--json", "--harness", "claude-code"];
        args.extend_from_slice(extra);
        json(&self.run(export, &args, None))
    }

    fn watch(&self, session: &str, extra: &[&str]) -> Output {
        let mut args = vec![
            "watch",
            "--hook",
            "claude-code",
            "--lifetime-secs",
            "1",
            "--poll-ms",
            "10",
            "--coalesce-ms",
            "0",
        ];
        args.extend_from_slice(extra);
        self.run(None, &args, Some(&self.payload(session)))
    }
}

impl Drop for Bench {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.home).unwrap();
    }
}

fn json(out: &Output) -> Value {
    assert!(out.status.success(), "{out:?}");
    serde_json::from_slice(&out.stdout).unwrap_or_else(|error| panic!("{error}: {out:?}"))
}

fn ids(value: &Value) -> Vec<String> {
    value["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|event| event["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn inbox_without_stable_identity_refuses_without_guessing_repo_or_writing_cursor() {
    let bench = Bench::new();
    bench.hello(READER);
    bench.note(
        &format!("repo:{}", bench.home.display()),
        "old repo warning",
    );
    std::fs::create_dir_all(bench.store.cursors_dir()).unwrap();
    let before: Vec<_> = std::fs::read_dir(bench.store.cursors_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    for export in [None, Some(("CLAUDE_CODE_SESSION_ID", ""))] {
        let out = bench.run(export, &["inbox", "--harness", "claude-code"], None);
        assert!(!out.status.success(), "{out:?}");
        assert!(out.stdout.is_empty(), "{out:?}");
        let error = String::from_utf8_lossy(&out.stderr);
        for hint in [
            "stable session",
            "--session",
            "HARBARD_SESSION",
            "CLAUDE_CODE_SESSION_ID",
            "CODEX_THREAD_ID",
        ] {
            assert!(error.contains(hint), "missing {hint}: {error}");
        }
    }
    let after: Vec<_> = std::fs::read_dir(bench.store.cursors_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(before, after, "no host-pid fallback cursor may be written");
}

#[test]
fn exported_claude_identity_skips_before_hello_history_and_shares_hook_delivery() {
    let bench = Bench::new();
    bench.note("harness:claude-code", "before hello harness warning");
    bench.note(
        &format!("repo:{}", bench.home.display()),
        "before hello repo warning",
    );
    assert_eq!(bench.hello(READER)["session"], READER);
    let stopped = bench.run(
        None,
        &["stop-check", "--hook", "claude-code"],
        Some(&bench.payload(READER)),
    );
    assert!(stopped.status.success(), "{stopped:?}");
    std::thread::sleep(std::time::Duration::from_millis(5));
    let note = bench.note("harness:claude-code", "fresh warning delivered by inbox");
    let inbox = bench.inbox(Some(("CLAUDE_CODE_SESSION_ID", READER)), &[]);
    assert_eq!(ids(&inbox), vec![note.id.clone()]);
    assert_eq!(
        cursor::read_inbox_cursor(&bench.store, READER),
        Some(note.id)
    );
    let hook = bench.watch(READER, &[]);
    assert!(
        hook.status.success(),
        "inbox delivery woke hook again: {hook:?}"
    );
    assert!(hook.stdout.is_empty() && hook.stderr.is_empty(), "{hook:?}");
    let fresh = bench.note("harness:claude-code", "fresh warning delivered by hook");
    let hook = bench.watch(READER, &[]);
    assert_eq!(hook.status.code(), Some(2), "{hook:?}");
    assert!(
        String::from_utf8_lossy(&hook.stderr).contains(&fresh.text),
        "{hook:?}"
    );
    assert_eq!(
        bench.inbox(Some(("CLAUDE_CODE_SESSION_ID", READER)), &[])["returned"],
        0
    );
}

#[test]
fn explicit_session_overrides_export_and_hook_identity_with_one_delivery_cursor() {
    let bench = Bench::new();
    bench.hello(READER);
    bench.hello("another-session");
    let stopped = bench.run(
        None,
        &["stop-check", "--hook", "claude-code", "--session", READER],
        Some(&bench.payload("another-session")),
    );
    assert!(stopped.status.success(), "{stopped:?}");
    std::thread::sleep(std::time::Duration::from_millis(5));
    let note = bench.note("harness:claude-code", "explicit-session warning");
    let inbox = bench.inbox(
        Some(("CLAUDE_CODE_SESSION_ID", "another-session")),
        &["--session", READER],
    );
    assert_eq!(ids(&inbox), vec![note.id.clone()]);
    assert_eq!(
        cursor::read_inbox_cursor(&bench.store, READER),
        Some(note.id)
    );
    assert_eq!(
        cursor::read_inbox_cursor(&bench.store, "another-session"),
        None
    );
    let hook = bench.watch("another-session", &["--session", READER]);
    assert!(
        hook.status.success(),
        "explicit identity differs between tool and hook: {hook:?}"
    );
    assert!(hook.stdout.is_empty() && hook.stderr.is_empty(), "{hook:?}");
}

#[test]
fn stable_identity_without_a_session_start_does_not_deliver_history() {
    let bench = Bench::new();
    bench.note("harness:claude-code", "no session start yet");
    let inbox = bench.inbox(Some(("HARBARD_SESSION", READER)), &[]);
    assert_eq!(inbox["returned"], 0);
    assert_eq!(cursor::read_inbox_cursor(&bench.store, READER), None);
}

#[test]
fn inbox_paginates_more_than_500_new_notes_without_replay_or_loss() {
    let bench = Bench::new();
    bench.note("harness:claude-code", "before hello");
    bench.hello(READER);
    let seed = bench.note("harness:claude-code", "new note 0");
    let mut expected = vec![seed.id.clone()];
    let base = harbard_core::clock::now_ms() + 1;
    for index in 1..507_u64 {
        let mut event = seed.clone();
        event.id = ulid::Ulid::from(((base + index) as u128) << 80).to_string();
        event.text = format!("new note {index}");
        bench.store.append(&event).unwrap();
        expected.push(event.id);
    }
    let export = Some(("CLAUDE_CODE_SESSION_ID", READER));
    let first = bench.inbox(export, &[]);
    assert_eq!(first["total"], 507);
    assert_eq!(first["returned"], 500);
    assert_eq!(ids(&first), expected[..500]);
    assert_eq!(
        cursor::read_inbox_cursor(&bench.store, READER).as_deref(),
        Some(expected[499].as_str())
    );
    let second = bench.inbox(export, &[]);
    assert_eq!(second["total"], 7);
    assert_eq!(ids(&second), expected[500..]);
    assert_eq!(
        cursor::read_inbox_cursor(&bench.store, READER).as_deref(),
        Some(expected[506].as_str())
    );
    assert_eq!(bench.inbox(export, &[])["returned"], 0);
}

#[test]
fn declared_harness_uses_its_own_identity_despite_inherited_markers() {
    for (harness, own_key) in [
        ("claude-code", "CLAUDE_CODE_SESSION_ID"),
        ("codex", "CODEX_THREAD_ID"),
        ("cursor", "CURSOR_CONVERSATION_ID"),
    ] {
        let bench = Bench::new();
        bench.hello(READER);
        let note = bench.note(&format!("session:{READER}"), "for this harness only");
        let mut command = Command::new(env!("CARGO_BIN_EXE_harbard"));
        command
            .env_clear()
            .env("HOME", &bench.home)
            .env("PATH", "/usr/bin:/bin")
            .env("CLAUDE_CODE_SESSION_ID", SENDER)
            .env("CODEX_THREAD_ID", SENDER)
            .env("CURSOR_CONVERSATION_ID", SENDER)
            .env(own_key, READER)
            .args(["inbox", "--json", "--harness", harness, "--home"])
            .arg(bench.store.root());
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{harness}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let inbox = json(&output);
        assert_eq!(ids(&inbox), vec![note.id], "{harness}");
        assert_eq!(cursor::read_inbox_cursor(&bench.store, SENDER), None);
    }
}

#[test]
fn a_specific_session_identity_beats_an_inherited_generic_marker() {
    for (harness, own_key, inherited_key) in [
        ("codex", "CODEX_THREAD_ID", "CLAUDECODE"),
        ("claude-code", "CLAUDE_CODE_SESSION_ID", "CODEX_HOME"),
        ("cursor", "CURSOR_CONVERSATION_ID", "CLAUDECODE"),
    ] {
        let bench = Bench::new();
        bench.hello(READER);
        let note = bench.note(
            &format!("harness:{harness}"),
            "addressed to the actual harness",
        );
        let output = Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .env("HOME", &bench.home)
            .env("PATH", "/usr/bin:/bin")
            .env(own_key, READER)
            .env(inherited_key, "inherited")
            .env("CURSOR_AGENT", "1")
            .args(["inbox", "--json", "--home"])
            .arg(bench.store.root())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{harness}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(ids(&json(&output)), vec![note.id], "{harness}");
    }
}
