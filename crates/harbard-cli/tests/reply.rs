//! `reply` contract: one short command answers a note. It threads the answer
//! under the note, addresses the note's author (the operator or the session
//! that wrote it) and works out who is answering when the caller did not say.
//!
//! Answering a note must not need `--session`, `--harness`, `--ref` and `--to`
//! spelled out: one wrong recipient sends an answer nowhere.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

const SENDER: &str = "11111111-1111-4111-8111-111111111111";
const READER: &str = "22222222-2222-4222-8222-222222222222";

fn tmp_home() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("harbard-reply-it-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("create temp home");
    d
}

fn bare(home: &Path, args: &[&str]) -> Output {
    let h = home.to_str().unwrap();
    Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        // A dead port: nothing here may reach a running serve.
        .env("HARBARD_API", "http://127.0.0.1:9")
        .env("HOME", h)
        .args(args)
        .args(["--home", h, "--cwd", h])
        .output()
        .expect("spawn harbard")
}

fn run(home: &Path, session: &str, args: &[&str]) -> Output {
    let mut all = args.to_vec();
    all.extend(["--session", session, "--harness", "codex"]);
    bare(home, &all)
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "JSON output: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn send(home: &Path, kind: &str, to: &str, text: &str) -> String {
    let out = run(
        home,
        SENDER,
        &["note", text, "--kind", kind, "--to", to, "--json"],
    );
    assert!(out.status.success(), "{out:?}");
    json(&out)["id"].as_str().unwrap().to_string()
}

fn bench() -> PathBuf {
    let home = tmp_home();
    for session in [SENDER, READER] {
        let out = run(&home, session, &["hello"]);
        assert!(out.status.success(), "{out:?}");
    }
    home
}

/// Append a note the way serve writes the operator's: harness `human`,
/// role `operator`.
fn operator_note(home: &Path, to_session: &str, text: &str) -> String {
    let stamp = json(&run(home, READER, &["intent", "stamp", "--json"]));
    let id = ulid::Ulid::generate().to_string();
    let note = serde_json::json!({
        "v": 1, "id": id, "ts": stamp["ts"], "event": "note",
        "session": "operator@h", "harness": "human", "role": "operator",
        "host": "h", "cwd": home.to_str().unwrap(), "text": text,
        "kind": "ask", "to": { "kind": "session", "value": to_session },
    });
    let mut segments: Vec<PathBuf> = std::fs::read_dir(home.join("events"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    segments.sort();
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(segments.last().unwrap())
        .unwrap();
    writeln!(file, "{note}").unwrap();
    id
}

#[test]
fn reply_threads_the_answer_and_addresses_the_session_that_wrote_the_note() {
    let home = bench();
    let ask = send(
        &home,
        "ask",
        &format!("session:{READER}"),
        "Which branch holds the fix?",
    );
    let out = run(&home, READER, &["reply", &ask, "The cli branch.", "--json"]);
    assert!(out.status.success(), "{out:?}");
    let v = json(&out);
    assert_eq!(v["ref"], ask, "{v}");
    assert_eq!(v["to"]["kind"], "session", "{v}");
    assert_eq!(v["to"]["value"], SENDER, "{v}");
    assert_eq!(v["session"], READER, "{v}");
    assert_eq!(v["text"], "The cli branch.", "{v}");
    assert_eq!(v["receipt"]["kind"], "fyi", "{v}");
}

#[test]
fn reply_to_an_operator_note_goes_to_the_human() {
    let home = bench();
    let id = operator_note(&home, READER, "How is it going?");
    let out = run(&home, READER, &["reply", &id, "Almost ready.", "--json"]);
    assert!(out.status.success(), "{out:?}");
    let v = json(&out);
    assert_eq!(v["ref"], id, "{v}");
    assert_eq!(v["to"]["kind"], "harness", "{v}");
    assert_eq!(v["to"]["value"], "human", "{v}");
}

/// The lead's own shell may carry no session marker. The note names the
/// session it was addressed to, and only that session answers it.
#[test]
fn reply_infers_the_answering_session_and_harness_from_the_note() {
    let home = bench();
    let id = operator_note(&home, READER, "Are you done?");
    let out = bare(&home, &["reply", &id, "Yes.", "--json"]);
    assert!(out.status.success(), "{out:?}");
    let v = json(&out);
    assert_eq!(v["session"], READER, "{v}");
    assert_eq!(v["harness"], "codex", "{v}");
    assert_eq!(v["to"]["value"], "human", "{v}");
}

#[test]
fn reply_last_takes_the_oldest_owed_answer_and_settles_it() {
    let home = bench();
    let _fyi = send(&home, "fyi", &format!("session:{READER}"), "Heads up.");
    let ask = send(
        &home,
        "ask",
        &format!("session:{READER}"),
        "Can you review it?",
    );
    let out = run(
        &home,
        READER,
        &["reply", "last", "Reviewing now.", "--json"],
    );
    assert!(out.status.success(), "{out:?}");
    assert_eq!(json(&out)["ref"], ask);

    // Settled: `ack last` now finds nothing owed.
    let again = run(&home, READER, &["ack", "last"]);
    assert!(!again.status.success(), "the reply settled the ask");
}

#[test]
fn reply_last_without_anything_addressed_to_you_refuses() {
    let home = bench();
    let out = run(&home, READER, &["reply", "last", "Hello?"]);
    assert!(!out.status.success(), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("nothing to reply to"), "{stderr}");
}

#[test]
fn reply_to_your_own_note_refuses_and_says_how_to_add_to_it() {
    let home = bench();
    let mine = send(&home, "fyi", &format!("session:{READER}"), "Mine.");
    let out = run(&home, SENDER, &["reply", &mine, "More."]);
    assert!(!out.status.success(), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("your own note"), "{stderr}");
    assert!(
        stderr.contains(&format!("harbard note --ref {mine}")),
        "{stderr}"
    );
}

#[test]
fn reply_to_an_unknown_note_refuses() {
    let home = bench();
    let out = run(&home, READER, &["reply", "01ZZZZZZZZZZZZZZZZZZZZZZZZ", "?"]);
    assert!(!out.status.success(), "{out:?}");
}

#[test]
fn reply_is_a_machine_operation() {
    let home = bench();
    let ask = send(&home, "ask", &format!("session:{READER}"), "Ready?");
    let input = serde_json::json!({ "target": ask, "text": "Yes." }).to_string();
    let out = run(&home, READER, &["call", "reply", "--input", &input]);
    assert!(out.status.success(), "{out:?}");
    let v = json(&out);
    assert_eq!(v["v"], "harbard/cli/v1", "{v}");
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["data"]["ref"], ask, "{v}");
}
