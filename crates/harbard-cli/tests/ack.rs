//! `ack` contract: `ack last` closes the oldest answer the session owes, never
//! an fyi, and an ack that carries words also lands as a `--ref` reply so the
//! answer is a message rather than a receipt.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

const SENDER: &str = "11111111-1111-4111-8111-111111111111";
const READER: &str = "22222222-2222-4222-8222-222222222222";

fn tmp_home() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("harbard-ack-it-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("create temp home");
    d
}

fn run(home: &Path, session: &str, args: &[&str]) -> Output {
    let home = home.to_str().unwrap();
    Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .args(args)
        .args([
            "--home",
            home,
            "--session",
            session,
            "--harness",
            "codex",
            "--cwd",
            home,
        ])
        .output()
        .expect("spawn harbard")
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

fn ack_last(home: &Path, extra: &[&str]) -> Output {
    let mut args = vec!["ack", "last", "--json"];
    args.extend_from_slice(extra);
    run(home, READER, &args)
}

/// Every event on the bus, in file order.
fn events(home: &Path) -> Vec<serde_json::Value> {
    let mut files = Vec::new();
    let mut stack = vec![home.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("events-"))
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
        .iter()
        .flat_map(|path| {
            std::fs::read_to_string(path)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect::<Vec<_>>()
        })
        .collect()
}

fn bench() -> PathBuf {
    let home = tmp_home();
    for session in [SENDER, READER] {
        assert!(run(&home, session, &["hello"]).status.success());
    }
    home
}

/// `ack last` must not take the newest note of any kind: an fyi that arrives
/// after a handoff would swallow the ack and leave the handoff open.
#[test]
fn ack_last_closes_the_oldest_owed_answer_session_addressed_first() {
    let home = bench();
    let ask = send(&home, "ask", "harness:codex", "Which PR are you on?");
    let handoff = send(
        &home,
        "handoff",
        &format!("session:{READER}"),
        "Take over the migration renumbering.",
    );
    let fyi = send(
        &home,
        "fyi",
        &format!("session:{READER}"),
        "The dev stack is back up.",
    );

    let first = ack_last(&home, &[]);
    assert!(first.status.success(), "{first:?}");
    assert_eq!(
        json(&first)["acked"],
        handoff.as_str(),
        "session-addressed first"
    );

    let second = ack_last(&home, &[]);
    assert!(second.status.success(), "{second:?}");
    assert_eq!(json(&second)["acked"], ask.as_str(), "then the harness ask");

    let third = ack_last(&home, &[]);
    assert!(!third.status.success(), "nothing owed must refuse");
    let why = String::from_utf8_lossy(&third.stderr);
    assert!(why.contains("nothing owed"), "{why}");
    assert!(why.contains(&fyi), "names what it would have acked: {why}");
    assert!(
        why.contains(&format!("harbard reply {fyi}")),
        "says where an answer goes: {why}"
    );
    let acks = events(&home).iter().filter(|e| e["event"] == "ack").count();
    assert_eq!(acks, 2, "the fyi was never acked");
}

#[test]
fn a_reply_by_ref_counts_as_answered() {
    let home = bench();
    let ask = send(&home, "ask", "harness:codex", "Which PR are you on?");
    let replied = run(
        &home,
        READER,
        &["note", "PR 2258, rebasing now.", "--ref", &ask],
    );
    assert!(replied.status.success(), "{replied:?}");
    let refused = ack_last(&home, &[]);
    assert!(!refused.status.success(), "{refused:?}");
}

/// 73% of acks carried the real answer, rendered as a receipt under the note.
#[test]
fn an_ack_with_words_also_lands_as_a_ref_reply() {
    let home = bench();
    let handoff = send(
        &home,
        "handoff",
        &format!("session:{READER}"),
        "Take over the migration renumbering.",
    );
    let out = ack_last(&home, &["--note", "Taking it; PR within the hour."]);
    assert!(out.status.success(), "{out:?}");
    let receipt = json(&out);
    let reply_id = receipt["reply"].as_str().expect("reply id").to_string();

    let all = events(&home);
    let reply = all.iter().find(|e| e["id"] == reply_id.as_str()).unwrap();
    assert_eq!(reply["event"], "note");
    assert_eq!(reply["ref"], handoff.as_str());
    assert_eq!(reply["text"], "Taking it; PR within the hour.");
    assert_eq!(reply["to"]["value"], SENDER);

    let ack = all.iter().find(|e| e["event"] == "ack").unwrap();
    assert_eq!(ack["ref"], handoff.as_str(), "the ack names what it closes");
    assert_eq!(
        ack["text"],
        format!("{handoff} — Taking it; PR within the hour."),
        "the stored text keeps its legacy shape"
    );

    let thread = run(&home, SENDER, &["thread", &handoff, "--json"]);
    let thread = json(&thread);
    assert_eq!(thread["count"], 3, "root, reply and ack: {thread}");
}

#[test]
fn a_bare_ack_writes_no_reply() {
    let home = bench();
    send(
        &home,
        "handoff",
        &format!("session:{READER}"),
        "Take over the migration renumbering.",
    );
    let out = ack_last(&home, &["--note", "  "]);
    assert!(out.status.success(), "{out:?}");
    assert!(json(&out)["reply"].is_null());
    let notes = events(&home)
        .iter()
        .filter(|e| e["event"] == "note" && e["session"] == READER)
        .count();
    assert_eq!(notes, 0);
}
