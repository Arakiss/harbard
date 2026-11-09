//! End-to-end tests for the Stop-hook verbs: `watch` wakes an idle session
//! when something addressed to it lands, `stop-check` holds a stop while the
//! session owes an answer. Each runs the real binary against a fresh store
//! with a cleared environment, through the same stdin the harness hooks send.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp_home() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("harbard-wake-it-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("create temp home");
    d
}

fn harbard(home: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_harbard"));
    cmd.env_clear();
    cmd.env("HOME", home);
    cmd.env("PATH", "/usr/bin:/bin");
    cmd.args(["--home", home.join(".harbard").to_str().unwrap()]);
    cmd
}

fn run(home: &Path, args: &[&str]) -> Output {
    harbard(home).args(args).output().expect("spawn harbard")
}

/// `hello` for one session in `home`, on the given harness.
fn hello(home: &Path, session: &str, harness: &str) {
    let out = run(
        home,
        &[
            "hello",
            "--session",
            session,
            "--harness",
            harness,
            "--cwd",
            home.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "hello {session}");
}

/// The prompt hook: advances the session's read position.
fn touch(home: &Path, session: &str) {
    let out = run(
        home,
        &[
            "brief",
            "--touch",
            "--session",
            session,
            "--harness",
            "claude-code",
            "--cwd",
            home.to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "touch {session}");
}

/// Write a note as `from` and return its id.
fn note(home: &Path, from: &str, kind: &str, to: Option<&str>, text: &str) -> String {
    let mut args = vec![
        "note",
        "--json",
        "--kind",
        kind,
        "--session",
        from,
        "--harness",
        "codex",
        "--cwd",
        home.to_str().unwrap(),
    ];
    if let Some(to) = to {
        args.extend(["--to", to]);
    }
    args.push(text);
    let out = run(home, &args);
    assert!(
        out.status.success(),
        "note failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("note --json");
    v["id"].as_str().expect("id").to_string()
}

fn stop_payload(home: &Path, session: &str) -> String {
    serde_json::json!({
        "session_id": session,
        "transcript_path": "/tmp/t.jsonl",
        "cwd": home.to_str().unwrap(),
        "hook_event_name": "Stop",
        "stop_hook_active": false,
    })
    .to_string()
}

/// Start `watch` the way Claude Code's asyncRewake hook does: Stop stdin JSON.
fn spawn_watch(home: &Path, session: &str, extra: &[&str]) -> Child {
    spawn_watch_env(home, session, extra, &[])
}

/// [`spawn_watch`] with the variables Claude Code puts in a hook's environment.
fn spawn_watch_env(home: &Path, session: &str, extra: &[&str], env: &[(&str, &str)]) -> Child {
    let mut child = harbard(home)
        .args(["watch", "--hook", "claude-code", "--poll-ms", "100"])
        .args(["--repo", home.to_str().unwrap()])
        .args(extra)
        .envs(env.iter().copied())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn watch");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stop_payload(home, session).as_bytes())
        .unwrap();
    child
}

/// Wait for the watcher to exit, failing the test past `limit`.
fn finish(mut child: Child, limit: Duration) -> (i32, String, String, Duration) {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            let out = child.wait_with_output().unwrap();
            return (
                status.code().unwrap_or(-1),
                String::from_utf8_lossy(&out.stdout).into_owned(),
                String::from_utf8_lossy(&out.stderr).into_owned(),
                started.elapsed(),
            );
        }
        if started.elapsed() > limit {
            let _ = child.kill();
            panic!("watch did not exit within {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A session that has spoken and read once, plus a live peer.
fn two_sessions() -> PathBuf {
    let home = tmp_home();
    hello(&home, "me", "claude-code");
    hello(&home, "peer", "codex");
    touch(&home, "me");
    // Establish a real end-of-turn boundary before publishing idle mail.
    let stopped = stop_check(&home, "claude-code", &stop_payload(&home, "me"));
    assert!(stopped.status.success() && stopped.stdout.is_empty());
    home
}

/// Give a freshly spawned watcher time to take its snapshot.
fn armed() {
    std::thread::sleep(Duration::from_millis(600));
}

/// What one `harbard` process costs on this machine right now: the slowest
/// of three `ls --json` runs against this store. Under load a single spawn can
/// take seconds, and fixed deadlines would fail the timing tests.
/// Deadlines below are scaled from this, never padded with a longer sleep.
fn spawn_cost(home: &Path) -> Duration {
    (0..3)
        .map(|_| {
            let t = Instant::now();
            let out = run(home, &["ls", "--json"]);
            assert!(out.status.success(), "ls");
            t.elapsed()
        })
        .max()
        .unwrap()
}

/// Wait, by polling, until the watcher owns the session's watch slot (it has
/// started and is inside its wait loop), or has already exited. The cap is
/// generous because the condition, not the cap, ends the wait.
fn armed_by_condition(home: &Path, child: &mut Child, cap: Duration) {
    let dir = home.join(".harbard").join("cursors");
    let started = Instant::now();
    loop {
        let owned = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .any(|e| {
                e.file_name().to_string_lossy().ends_with(".watch")
                    && std::fs::metadata(e.path()).is_ok_and(|m| m.len() > 0)
            });
        if owned || child.try_wait().unwrap().is_some() {
            return;
        }
        assert!(
            started.elapsed() < cap,
            "watcher did not arm within {cap:?}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// W1: a warning addressed to the idle session wakes it within 5 s, and the
/// message is the whole body — not a 140-byte clip — with sender, kind, note
/// id and the exact reply command.
#[test]
fn watch_wakes_within_five_seconds_with_the_full_text() {
    let home = two_sessions();
    let cost = spawn_cost(&home);
    let mut child = spawn_watch(&home, "me", &["--lifetime-secs", "120"]);
    armed_by_condition(&home, &mut child, Duration::from_secs(60) + cost * 20);
    let body = format!("migrations lane is yours now. {}", "detail ".repeat(60));
    let id = note(&home, "peer", "warning", Some("session:me"), &body);
    // From the moment the note is on disk: the five seconds are the
    // watcher's (poll + coalesce + render), plus what one more process
    // costs on this machine right now.
    let sent = Instant::now();
    let (code, stdout, stderr, _) = finish(child, Duration::from_secs(60) + cost * 20);
    let latency = sent.elapsed();
    assert_eq!(code, 2, "exit 2 wakes the session: {stderr}");
    let budget = Duration::from_secs(5) + cost * 2;
    assert!(
        latency < budget,
        "woke after {latency:?}, budget {budget:?} (one spawn costs {cost:?})"
    );
    assert!(stdout.is_empty(), "the message rides stderr: {stdout}");
    assert!(stderr.contains(body.trim_end()), "whole body: {stderr}");
    assert!(stderr.contains(&format!("note {id}")), "{stderr}");
    assert!(stderr.contains("Warning from"), "{stderr}");
    assert!(stderr.contains("codex session peer"), "{stderr}");
    assert!(
        stderr.contains(&format!("harbard reply {id} \"<your answer>\"")),
        "{stderr}"
    );
}

/// Every op the store recorded, as raw text.
fn ops_text(home: &Path) -> String {
    let dir = home.join(".harbard").join("ops");
    let mut text = String::new();
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        text.push_str(&std::fs::read_to_string(entry.path()).unwrap_or_default());
    }
    text
}

/// A one-shot `claude -p` turn ends with its Stop hooks, and the CLI waits
/// for an asyncRewake hook before it exits: a watcher there held each process
/// open for its whole lifetime. It must leave at once,
/// silently, and leave nothing behind, so the mail stays for a real reader.
#[test]
fn watch_leaves_a_claude_print_turn_at_once_without_a_trace() {
    let home = two_sessions();
    let ops_before = ops_text(&home);
    let child = spawn_watch_env(
        &home,
        "me",
        &["--lifetime-secs", "60"],
        &[("CLAUDE_CODE_ENTRYPOINT", "sdk-cli")],
    );
    let (code, stdout, stderr, took) = finish(child, Duration::from_secs(10));
    assert_eq!(code, 0, "{stderr}");
    assert!(took < Duration::from_secs(3), "left after {took:?}");
    assert!(stdout.is_empty() && stderr.is_empty(), "{stdout}{stderr}");
    assert_eq!(ops_text(&home), ops_before, "no op recorded");

    // The skipped watcher took nothing: an ordinary one still delivers.
    let child = spawn_watch(&home, "me", &["--lifetime-secs", "30"]);
    armed();
    let id = note(
        &home,
        "peer",
        "warning",
        Some("session:me"),
        "lane is yours",
    );
    let (code, _, stderr, _) = finish(child, Duration::from_secs(10));
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains(&format!("note {id}")), "{stderr}");
}

/// Negative of the above: an interactive terminal (`cli`), a Desktop session,
/// and a `claude -p` that streams its input (more turns to come) can all be
/// woken, so their watcher keeps watching and wakes them.
#[test]
fn watch_keeps_watching_sessions_that_can_be_woken() {
    let mut streaming_harness = Command::new("/bin/sh")
        .args(["-c", "sleep 30; :", "claude", "-p"])
        .args(["--input-format", "stream-json"])
        .spawn()
        .expect("spawn stand-in harness");
    let streaming_pid = streaming_harness.id().to_string();
    let cases: [&[(&str, &str)]; 3] = [
        &[("CLAUDE_CODE_ENTRYPOINT", "cli")],
        &[("CLAUDE_CODE_ENTRYPOINT", "claude-desktop")],
        &[
            ("CLAUDE_CODE_ENTRYPOINT", "sdk-cli"),
            ("CLAUDE_PID", &streaming_pid),
        ],
    ];
    for env in cases {
        let home = two_sessions();
        let child = spawn_watch_env(&home, "me", &["--lifetime-secs", "30"], env);
        armed();
        let id = note(
            &home,
            "peer",
            "warning",
            Some("session:me"),
            "lane is yours",
        );
        let (code, _, stderr, _) = finish(child, Duration::from_secs(10));
        assert_eq!(code, 2, "{env:?} must still wake: {stderr}");
        assert!(stderr.contains(&format!("note {id}")), "{env:?}: {stderr}");
    }
    let _ = streaming_harness.kill();
    let _ = streaming_harness.wait();
}

/// W1: an ask to the session's harness, a reply to its own note, and a
/// resolved dependency it opened each wake it.
#[test]
fn watch_wakes_for_harness_asks_replies_and_resolved_dependencies() {
    // Ask to the harness.
    let home = two_sessions();
    let child = spawn_watch(
        &home,
        "me",
        &["--lifetime-secs", "30", "--coalesce-ms", "200"],
    );
    armed();
    note(
        &home,
        "peer",
        "ask",
        Some("harness:claude-code"),
        "who holds dev?",
    );
    let (code, _, stderr, _) = finish(child, Duration::from_secs(10));
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("who holds dev?"));

    // A reply to my note, even a quiet one.
    let home = two_sessions();
    let mine = note(&home, "me", "fyi", None, "pushed the fix");
    touch(&home, "me");
    let child = spawn_watch(
        &home,
        "me",
        &["--lifetime-secs", "30", "--coalesce-ms", "200"],
    );
    armed();
    let out = run(
        &home,
        &[
            "note",
            "--ref",
            &mine,
            "--session",
            "peer",
            "--harness",
            "codex",
            "--cwd",
            home.to_str().unwrap(),
            "verified on staging",
        ],
    );
    assert!(out.status.success());
    let (code, _, stderr, _) = finish(child, Duration::from_secs(10));
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains(&format!("to your note {mine}")), "{stderr}");
    assert!(stderr.contains("verified on staging"));

    // A dependency I opened, resolved by the peer.
    let home = two_sessions();
    let open = run(
        &home,
        &[
            "dependency",
            "open",
            "--json",
            "--to",
            "session:peer",
            "--condition",
            "CI green",
            "--resume",
            "merge",
            "--session",
            "me",
            "--harness",
            "claude-code",
            "--cwd",
            home.to_str().unwrap(),
        ],
    );
    assert!(
        open.status.success(),
        "{}",
        String::from_utf8_lossy(&open.stderr)
    );
    let opened: serde_json::Value = serde_json::from_slice(&open.stdout).unwrap();
    let dep = opened["id"]
        .as_str()
        .or_else(|| opened["dependency"]["id"].as_str())
        .expect("dependency id")
        .to_string();
    touch(&home, "me");
    let child = spawn_watch(
        &home,
        "me",
        &["--lifetime-secs", "30", "--coalesce-ms", "200"],
    );
    armed();
    let resolved = run(
        &home,
        &[
            "dependency",
            "resolve",
            &dep,
            "--outcome",
            "satisfied",
            "--evidence",
            "run 4412 passed",
            "--session",
            "peer",
            "--harness",
            "codex",
            "--cwd",
            home.to_str().unwrap(),
        ],
    );
    assert!(
        resolved.status.success(),
        "{}",
        String::from_utf8_lossy(&resolved.stderr)
    );
    let (code, _, stderr, _) = finish(child, Duration::from_secs(10));
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains(&format!("Dependency {dep}")), "{stderr}");
    assert!(stderr.contains("run 4412 passed"));
    assert!(stderr.contains(&format!("harbard dependency consume {dep}")));
}

/// W2: a note that landed after the turn ended but before the watcher armed
/// is still delivered — the watcher starts at the session's read position.
#[test]
fn watch_delivers_what_landed_before_it_armed() {
    let home = two_sessions();
    note(
        &home,
        "peer",
        "ask",
        Some("session:me"),
        "landed between turns",
    );
    let child = spawn_watch(&home, "me", &["--lifetime-secs", "30"]);
    let (code, _, stderr, elapsed) = finish(child, Duration::from_secs(10));
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("landed between turns"));
    assert!(elapsed < Duration::from_secs(5), "{elapsed:?}");
}

/// W3: fyi traffic — broadcast, to the harness, even to the session — and
/// the session's own notes never wake it.
#[test]
fn quiet_traffic_and_own_notes_do_not_wake() {
    let home = two_sessions();
    let child = spawn_watch(&home, "me", &["--lifetime-secs", "3"]);
    armed();
    note(&home, "peer", "fyi", None, "broadcast fyi");
    note(
        &home,
        "peer",
        "fyi",
        Some("harness:claude-code"),
        "harness fyi",
    );
    note(&home, "peer", "fyi", Some("session:me"), "session fyi");
    let out = run(
        &home,
        &[
            "note",
            "--kind",
            "warning",
            "--to",
            "harness:claude-code",
            "--session",
            "me",
            "--harness",
            "claude-code",
            "--cwd",
            home.to_str().unwrap(),
            "my own warning",
        ],
    );
    assert!(out.status.success());
    let (code, stdout, stderr, _) = finish(child, Duration::from_secs(10));
    assert_eq!(code, 0, "{stderr}");
    assert!(
        stdout.is_empty() && stderr.is_empty(),
        "silent: {stdout}{stderr}"
    );
}

/// W4: a burst becomes one wake carrying every body.
#[test]
fn a_burst_becomes_one_wake() {
    let home = two_sessions();
    // Three notes are three processes. The burst window must hold all three
    // on this machine now, so it is scaled from what one costs (the default
    // 1.5 s held them only while a spawn cost under half a second).
    let cost = spawn_cost(&home);
    let coalesce = (cost * 6).max(Duration::from_millis(1_500));
    let coalesce_ms = coalesce.as_millis().to_string();
    let mut child = spawn_watch(
        &home,
        "me",
        &["--lifetime-secs", "120", "--coalesce-ms", &coalesce_ms],
    );
    armed_by_condition(&home, &mut child, Duration::from_secs(60) + cost * 20);
    for n in 1..=3 {
        note(
            &home,
            "peer",
            "warning",
            Some("session:me"),
            &format!("burst body {n}"),
        );
    }
    let (code, _, stderr, _) = finish(child, Duration::from_secs(60) + coalesce + cost * 20);
    assert_eq!(code, 2);
    assert!(stderr.contains("3 messages"), "{stderr}");
    for n in 1..=3 {
        assert!(stderr.contains(&format!("burst body {n}")), "{stderr}");
    }
}

/// A wake is not repeated: the next watcher, armed at the end of the woken
/// turn, does not fire again for what the last one carried.
#[test]
fn a_delivered_wake_is_not_repeated() {
    let home = two_sessions();
    note(&home, "peer", "warning", Some("session:me"), "once only");
    let (code, _, _, _) = finish(
        spawn_watch(&home, "me", &["--lifetime-secs", "30"]),
        Duration::from_secs(10),
    );
    assert_eq!(code, 2);
    let (code, _, stderr, _) = finish(
        spawn_watch(&home, "me", &["--lifetime-secs", "2"]),
        Duration::from_secs(10),
    );
    assert_eq!(code, 0, "no second wake: {stderr}");
}

/// W5: `bye` ends the watcher silently.
#[test]
fn bye_ends_the_watcher() {
    let home = two_sessions();
    let child = spawn_watch(&home, "me", &["--lifetime-secs", "60"]);
    armed();
    let out = run(
        &home,
        &[
            "bye",
            "--session",
            "me",
            "--harness",
            "claude-code",
            "--cwd",
            home.to_str().unwrap(),
        ],
    );
    assert!(out.status.success());
    let (code, stdout, stderr, elapsed) = finish(child, Duration::from_secs(10));
    assert_eq!(code, 0);
    assert!(stdout.is_empty() && stderr.is_empty());
    assert!(elapsed < Duration::from_secs(4), "{elapsed:?}");
}

/// W5: an ended session never arms a watcher.
#[test]
fn an_ended_session_gets_no_watcher() {
    let home = two_sessions();
    run(
        &home,
        &[
            "bye",
            "--session",
            "me",
            "--harness",
            "claude-code",
            "--cwd",
            home.to_str().unwrap(),
        ],
    );
    let (code, _, _, elapsed) = finish(
        spawn_watch(&home, "me", &["--lifetime-secs", "60"]),
        Duration::from_secs(10),
    );
    assert_eq!(code, 0);
    assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
}

/// W5: the max lifetime ends the watcher.
#[test]
fn the_lifetime_ends_the_watcher() {
    let home = two_sessions();
    let (code, _, _, elapsed) = finish(
        spawn_watch(&home, "me", &["--lifetime-secs", "1"]),
        Duration::from_secs(10),
    );
    assert_eq!(code, 0);
    assert!(elapsed < Duration::from_secs(4), "{elapsed:?}");
}

/// W5: the watcher exits when the harness process it serves is gone.
#[test]
fn a_dead_supervisor_ends_the_watcher() {
    let home = two_sessions();
    let mut harness = Command::new("/bin/sleep").arg("60").spawn().unwrap();
    let pid = harness.id().to_string();
    let child = spawn_watch(
        &home,
        "me",
        &["--lifetime-secs", "60", "--supervisor-pid", &pid],
    );
    armed();
    harness.kill().unwrap();
    harness.wait().unwrap();
    let (code, _, _, elapsed) = finish(child, Duration::from_secs(15));
    assert_eq!(code, 0);
    assert!(elapsed < Duration::from_secs(8), "{elapsed:?}");
}

/// W5: one watcher per session — a newer Stop's watcher replaces the older
/// one, which leaves without waking anyone; the newer one still wakes.
#[test]
fn a_newer_watcher_replaces_the_older_one() {
    let home = two_sessions();
    let old = spawn_watch(&home, "me", &["--lifetime-secs", "60"]);
    armed();
    let new = spawn_watch(&home, "me", &["--lifetime-secs", "60"]);
    let (code, _, stderr, elapsed) = finish(old, Duration::from_secs(10));
    assert_eq!(code, 0, "{stderr}");
    assert!(elapsed < Duration::from_secs(3), "{elapsed:?}");
    armed();
    note(
        &home,
        "peer",
        "warning",
        Some("session:me"),
        "for the new watcher",
    );
    let (code, _, stderr, _) = finish(new, Duration::from_secs(10));
    assert_eq!(code, 2);
    assert!(stderr.contains("for the new watcher"));
}

fn stop_check(home: &Path, harness: &str, payload: &str) -> Output {
    let mut child = harbard(home)
        .args(["stop-check", "--hook", harness])
        .args(["--repo", home.to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(payload.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn codex_stop_payload(home: &Path, session: &str, active: bool) -> String {
    serde_json::json!({
        "session_id": session,
        "turn_id": "turn-1",
        "transcript_path": null,
        "cwd": home.to_str().unwrap(),
        "hook_event_name": "Stop",
        "model": "gpt-5.5",
        "permission_mode": "default",
        "stop_hook_active": active,
        "last_assistant_message": "done",
    })
    .to_string()
}

/// O1: an owed ask holds the stop, on both harness shapes, with a reason that
/// names the sender, the note id, the whole text and the exact command.
#[test]
fn stop_check_holds_a_stop_while_an_answer_is_owed() {
    for harness in ["claude-code", "codex"] {
        let home = tmp_home();
        hello(&home, "me", harness);
        hello(&home, "peer", "codex");
        let body = format!("what is left on the migration? {}", "context ".repeat(40));
        let id = note(&home, "peer", "ask", Some("session:me"), &body);
        let payload = if harness == "codex" {
            codex_stop_payload(&home, "me", false)
        } else {
            stop_payload(&home, "me")
        };
        let out = stop_check(&home, harness, &payload);
        assert!(out.status.success(), "{harness}");
        let decision: serde_json::Value =
            serde_json::from_slice(&out.stdout).expect("block JSON on stdout");
        assert_eq!(decision["decision"], "block", "{harness}");
        let reason = decision["reason"].as_str().unwrap();
        assert!(reason.contains(body.trim_end()), "{harness}: {reason}");
        assert!(reason.contains(&format!("note {id}")), "{reason}");
        assert!(reason.contains("codex session peer"), "{reason}");
        assert!(
            reason.contains(&format!("harbard reply {id} \"<your answer>\"")),
            "{reason}"
        );
        assert!(reason.contains(&format!("harbard ack {id}")), "{reason}");
    }
}

/// O1: `stop_hook_active` is never held again, the same note holds at most
/// two stops, an answered note holds none, and nothing owed is silence.
#[test]
fn stop_check_respects_its_limits() {
    let home = tmp_home();
    hello(&home, "me", "codex");
    hello(&home, "peer", "codex");

    let quiet = stop_check(&home, "codex", &codex_stop_payload(&home, "me", false));
    assert!(
        quiet.status.success() && quiet.stdout.is_empty(),
        "nothing owed"
    );

    let id = note(&home, "peer", "ask", Some("session:me"), "ready to merge?");
    let continued = stop_check(&home, "codex", &codex_stop_payload(&home, "me", true));
    assert!(
        continued.stdout.is_empty(),
        "stop_hook_active is never held"
    );

    for n in 1..=2 {
        let out = stop_check(&home, "codex", &codex_stop_payload(&home, "me", false));
        assert!(!out.stdout.is_empty(), "hold {n}");
    }
    let third = stop_check(&home, "codex", &codex_stop_payload(&home, "me", false));
    assert!(third.stdout.is_empty(), "held twice already");

    // A fresh ask holds again; answering it releases.
    let second = note(&home, "peer", "ask", Some("session:me"), "and the docs?");
    let out = stop_check(&home, "codex", &codex_stop_payload(&home, "me", false));
    let reason = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        reason.contains(&second) && !reason.contains(&id),
        "{reason}"
    );
    let ack = run(
        &home,
        &[
            "ack",
            &second,
            "--session",
            "me",
            "--harness",
            "codex",
            "--cwd",
            home.to_str().unwrap(),
        ],
    );
    assert!(ack.status.success());
    let after = stop_check(&home, "codex", &codex_stop_payload(&home, "me", false));
    assert!(
        after.stdout.is_empty(),
        "answered: {}",
        String::from_utf8_lossy(&after.stdout)
    );
}

/// Append a note line the CLI refuses to write (an operator's note, a
/// broadcast ask from before `ask` required `--to`) straight into today's
/// segment, stamped like a fresh event. Returns its id.
fn append_raw(home: &Path, mut note: serde_json::Value) -> String {
    let stamp = run(
        home,
        &[
            "hello",
            "--json",
            "--session",
            "clock",
            "--harness",
            "codex",
            "--cwd",
            home.to_str().unwrap(),
        ],
    );
    let stamp: serde_json::Value = serde_json::from_slice(&stamp.stdout).unwrap();
    let id = ulid::Ulid::generate().to_string();
    note["v"] = serde_json::json!(1);
    note["id"] = serde_json::json!(id);
    note["ts"] = stamp["ts"].clone();
    note["event"] = serde_json::json!("note");
    note["host"] = serde_json::json!("h");
    note["cwd"] = serde_json::json!(home.to_str().unwrap());
    let events = home.join(".harbard").join("events");
    let mut segments: Vec<PathBuf> = std::fs::read_dir(&events)
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

/// The operator's fyi to the session wakes it — the console composer
/// defaults to fyi — while an agent's fyi to the session does not.
#[test]
fn an_operator_fyi_wakes_the_session_and_an_agent_fyi_does_not() {
    let home = two_sessions();
    let child = spawn_watch(&home, "me", &["--lifetime-secs", "3"]);
    armed();
    note(&home, "peer", "fyi", Some("session:me"), "agent fyi");
    let (code, _, stderr, _) = finish(child, Duration::from_secs(10));
    assert_eq!(code, 0, "an agent fyi never wakes: {stderr}");

    let child = spawn_watch(&home, "me", &["--lifetime-secs", "30"]);
    armed();
    let id = append_raw(
        &home,
        serde_json::json!({
            "session": "operator@h", "harness": "human", "role": "operator",
            "text": "come look at the console", "to": { "kind": "session", "value": "me" },
        }),
    );
    let (code, _, stderr, _) = finish(child, Duration::from_secs(10));
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("come look at the console"), "{stderr}");
    assert!(stderr.contains("from the operator"), "{stderr}");
    assert!(
        stderr.contains(&format!("harbard reply {id} \"<your answer>\"")),
        "{stderr}"
    );

    // The reply the wake text prescribes must be one the CLI accepts.
    let reply = run(
        &home,
        &[
            "reply",
            &id,
            "--json",
            "--session",
            "me",
            "--harness",
            "claude-code",
            "--cwd",
            home.to_str().unwrap(),
            "on my way",
        ],
    );
    assert!(
        reply.status.success(),
        "the prescribed reply was refused: {}",
        String::from_utf8_lossy(&reply.stderr)
    );
    let written: serde_json::Value = serde_json::from_slice(&reply.stdout).expect("json");
    assert_eq!(written["to"]["kind"], "harness", "{written}");
    assert_eq!(written["to"]["value"], "human", "{written}");
}

/// A channel ask wakes a subscribed member and leaves a non-member asleep.
#[test]
fn a_channel_ask_wakes_members_only() {
    let home = two_sessions();
    hello(&home, "other", "claude-code");
    touch(&home, "other");
    let join = run(
        &home,
        &[
            "join",
            "ops",
            "--as",
            "session",
            "--session",
            "me",
            "--harness",
            "claude-code",
            "--cwd",
            home.to_str().unwrap(),
        ],
    );
    assert!(
        join.status.success(),
        "{}",
        String::from_utf8_lossy(&join.stderr)
    );
    let member = spawn_watch(&home, "me", &["--lifetime-secs", "30"]);
    let outsider = spawn_watch(&home, "other", &["--lifetime-secs", "3"]);
    armed();
    note(
        &home,
        "peer",
        "ask",
        Some("channel:ops"),
        "ops: who is on call?",
    );
    let (code, _, stderr, _) = finish(member, Duration::from_secs(10));
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("ops: who is on call?"));
    let (code, _, stderr, _) = finish(outsider, Duration::from_secs(10));
    assert_eq!(code, 0, "a non-member is not woken: {stderr}");
}

/// A broadcast ask is owed by everyone on the board, but calls for nobody in
/// particular: it neither wakes a session nor holds its Stop.
#[test]
fn a_broadcast_ask_neither_wakes_nor_holds_a_stop() {
    let home = two_sessions();
    let child = spawn_watch(&home, "me", &["--lifetime-secs", "3"]);
    armed();
    append_raw(
        &home,
        serde_json::json!({
            "session": "peer", "harness": "codex", "kind": "ask",
            "text": "everyone: report status",
        }),
    );
    let (code, _, stderr, _) = finish(child, Duration::from_secs(10));
    assert_eq!(code, 0, "{stderr}");
    let out = stop_check(&home, "claude-code", &stop_payload(&home, "me"));
    assert!(out.status.success());
    assert!(
        out.stdout.is_empty(),
        "a broadcast ask holds no Stop: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// A fake `codex` that records its argv, one per line, in `$CODEX_FAKE_OUT`.
fn fake_codex(home: &Path) -> (PathBuf, PathBuf) {
    let bin = home.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = bin.join("codex");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$CODEX_FAKE_OUT\"\n",
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    (bin, home.join("codex-argv.txt"))
}

fn detach(home: &Path, path: &str, out: &Path) -> Output {
    let mut child = harbard(home)
        .env("PATH", path)
        .env("CODEX_FAKE_OUT", out)
        .args([
            "watch",
            "--detach",
            "--hook",
            "codex",
            "--lifetime-secs",
            "20",
            "--poll-ms",
            "100",
            "--coalesce-ms",
            "200",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(codex_stop_payload(home, "me", false).as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

/// W6: with no `codex` to deliver through, nothing crashes and the note stays
/// unread for the next watcher instead of being marked delivered.
#[test]
fn a_failed_codex_delivery_leaves_the_note_unread() {
    let home = tmp_home();
    hello(&home, "me", "codex");
    hello(&home, "peer", "codex");
    stop_check(&home, "codex", &codex_stop_payload(&home, "me", false));
    let out = home.join("never.txt");
    let hook = detach(&home, "/usr/bin:/bin", &out);
    assert!(hook.status.success() && hook.stdout.is_empty());
    armed();
    note(&home, "peer", "warning", Some("session:me"), "still unread");
    std::thread::sleep(Duration::from_secs(2));
    let (code, _, stderr, _) = finish(
        spawn_watch(&home, "me", &["--lifetime-secs", "10"]),
        Duration::from_secs(15),
    );
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("still unread"));
}

/// W6: doctor reports how a Codex wake travels, daemon absent included,
/// without failing or crashing.
#[test]
fn doctor_reports_the_codex_wake_route() {
    let home = tmp_home();
    let (bin, _) = fake_codex(&home);
    let path = format!("{}:/usr/bin:/bin", bin.display());
    let config = home.join(".codex").join("hooks.json");
    let install = harbard(&home)
        .env("PATH", &path)
        .args(["install", "codex", "--config", config.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(install.status.success());

    let json = harbard(&home)
        .env("PATH", &path)
        .args(["doctor", "--json"])
        .output()
        .unwrap();
    let report: serde_json::Value = serde_json::from_slice(&json.stdout).expect("doctor --json");
    let codex = report["hooks"]["harnesses"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["target"] == "codex")
        .unwrap()
        .clone();
    assert_eq!(codex["ok"], true, "{codex}");
    assert_eq!(codex["wake"]["daemon"], "absent");
    assert_eq!(codex["wake"]["codex_on_path"], true);

    let text = harbard(&home)
        .env("PATH", &path)
        .args(["doctor"])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(text.contains("daemon is not running"), "{text}");
}

/// What `stop-check` already showed on this Stop is not repeated by the
/// watcher armed by the same Stop.
#[test]
fn watch_does_not_repeat_what_stop_check_showed() {
    let home = two_sessions();
    note(&home, "peer", "ask", Some("session:me"), "shown once");
    let out = stop_check(&home, "claude-code", &stop_payload(&home, "me"));
    assert!(!out.stdout.is_empty());
    let (code, _, stderr, _) = finish(
        spawn_watch(&home, "me", &["--lifetime-secs", "3"]),
        Duration::from_secs(10),
    );
    assert_eq!(code, 0, "{stderr}");
}

#[test]
fn a_new_session_never_inherits_repository_or_harness_backlog() {
    let home = tmp_home();
    hello(&home, "A", "codex");
    let repo = format!("repo:{}", home.display());
    let old_warning = note(&home, "A", "warning", Some(&repo), "old repository warning");
    let old_ask = note(
        &home,
        "A",
        "ask",
        Some("harness:claude-code"),
        "old harness ask",
    );
    let old_repo_ask = note(&home, "A", "ask", Some(&repo), "old repository ask");
    run(&home, &["bye", "--session", "A", "--harness", "codex"]);
    hello(&home, "me", "claude-code");
    // Model the older brief/inbox cursor that triggered historical replay.
    std::fs::create_dir_all(home.join(".harbard/cursors")).unwrap();
    std::fs::write(home.join(".harbard/cursors/me"), &old_warning).unwrap();
    let stopped = stop_check(&home, "claude-code", &stop_payload(&home, "me"));
    assert!(
        stopped.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&stopped.stdout)
    );
    let (code, _, stderr, _) = finish(
        spawn_watch(&home, "me", &["--lifetime-secs", "2"]),
        Duration::from_secs(5),
    );
    assert_eq!(code, 0, "{stderr}");
    assert!(!stderr.contains(&old_ask) && !stderr.contains(&old_repo_ask));
    let fresh = note(
        &home,
        "A",
        "ask",
        Some("harness:claude-code"),
        "new harness ask",
    );
    let fresh_repo = note(&home, "A", "ask", Some(&repo), "new repository ask");
    let stopped = stop_check(&home, "claude-code", &stop_payload(&home, "me"));
    let decision: serde_json::Value = serde_json::from_slice(&stopped.stdout).unwrap();
    let reason = decision["reason"].as_str().unwrap();
    assert!(reason.contains(&fresh));
    assert!(reason.contains(&fresh_repo));
    assert!(!reason.contains(&old_ask) && !reason.contains(&old_repo_ask));
}

#[test]
fn active_turn_notes_do_not_become_idle_arrivals() {
    let home = two_sessions();
    touch(&home, "me");
    let busy = note(
        &home,
        "peer",
        "warning",
        Some("session:me"),
        "arrived during work",
    );
    stop_check(&home, "claude-code", &stop_payload(&home, "me"));
    let child = spawn_watch(
        &home,
        "me",
        &["--lifetime-secs", "10", "--coalesce-ms", "200"],
    );
    armed();
    let idle = note(
        &home,
        "peer",
        "warning",
        Some("session:me"),
        "arrived after stop",
    );
    let (code, _, stderr, _) = finish(child, Duration::from_secs(5));
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains(&idle));
    assert!(!stderr.contains(&busy), "{stderr}");
}

#[test]
fn inbox_read_during_coalescing_prevents_a_duplicate_wake() {
    let home = two_sessions();
    let child = spawn_watch(
        &home,
        "me",
        &["--lifetime-secs", "4", "--coalesce-ms", "1800"],
    );
    armed();
    let id = note(
        &home,
        "peer",
        "warning",
        Some("session:me"),
        "read from Bash",
    );
    let inbox = harbard(&home)
        .env("CLAUDE_CODE_SESSION_ID", "me")
        .args([
            "inbox",
            "--json",
            "--harness",
            "claude-code",
            "--cwd",
            home.to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(
        inbox.status.success(),
        "{}",
        String::from_utf8_lossy(&inbox.stderr)
    );
    let delivered: serde_json::Value = serde_json::from_slice(&inbox.stdout).unwrap();
    assert_eq!(delivered["events"][0]["id"], id);
    let (code, _, stderr, _) = finish(child, Duration::from_secs(7));
    assert_eq!(code, 0, "{stderr}");
    // Reading is delivery, not accepting a debt: fresh asks still require ack/reply.
    let ask = note(
        &home,
        "peer",
        "ask",
        Some("session:me"),
        "need an explicit answer",
    );
    run(
        &home,
        &["inbox", "--session", "me", "--harness", "claude-code"],
    );
    let held = stop_check(&home, "claude-code", &stop_payload(&home, "me"));
    assert!(String::from_utf8_lossy(&held.stdout).contains(&ask));
}

#[test]
fn a_sender_update_replaces_its_warning_before_delivery() {
    let home = two_sessions();
    let child = spawn_watch(
        &home,
        "me",
        &["--lifetime-secs", "3", "--coalesce-ms", "1200"],
    );
    armed();
    let old = note(
        &home,
        "peer",
        "warning",
        Some("session:me"),
        "do not enqueue",
    );
    let update = run(
        &home,
        &[
            "note",
            "--session",
            "peer",
            "--harness",
            "codex",
            "--kind",
            "fyi",
            "--to",
            "session:me",
            "--ref",
            &old,
            "you may enqueue now",
        ],
    );
    assert!(
        update.status.success(),
        "{}",
        String::from_utf8_lossy(&update.stderr)
    );
    let (code, _, stderr, _) = finish(child, Duration::from_secs(6));
    assert_eq!(
        code, 0,
        "quiet replacement cancels the obsolete warning: {stderr}"
    );
}

#[test]
fn a_new_prompt_disarms_the_old_idle_watcher() {
    let home = two_sessions();
    let child = spawn_watch(&home, "me", &["--lifetime-secs", "10"]);
    armed();
    touch(&home, "me");
    note(&home, "peer", "warning", Some("session:me"), "busy again");
    let (code, _, stderr, _) = finish(child, Duration::from_secs(4));
    assert_eq!(code, 0, "{stderr}");
}

#[test]
fn prompt_delivery_does_not_paginate_pre_join_context() {
    let home = tmp_home();
    hello(&home, "peer", "codex");
    let older = note(
        &home,
        "peer",
        "warning",
        Some("harness:claude-code"),
        "old first notice",
    );
    let old = note(
        &home,
        "peer",
        "ask",
        Some("harness:claude-code"),
        "old remaining context",
    );
    hello(&home, "me", "claude-code");
    let fresh = note(
        &home,
        "peer",
        "warning",
        Some("harness:claude-code"),
        "current notice",
    );
    std::fs::create_dir_all(home.join(".harbard/cursors")).unwrap();
    std::fs::write(
        home.join(".harbard/cursors/me.brief"),
        serde_json::json!({"seen": older}).to_string(),
    )
    .unwrap();
    let prompt = run(
        &home,
        &[
            "brief",
            "--touch",
            "--json",
            "--session",
            "me",
            "--harness",
            "claude-code",
            "--cwd",
            home.to_str().unwrap(),
        ],
    );
    assert!(prompt.status.success());
    let json: serde_json::Value = serde_json::from_slice(&prompt.stdout).unwrap();
    let text = json["brief"].as_str().unwrap();
    assert!(text.contains(&fresh), "{text}");
    assert!(!text.contains(&old), "{text}");
}

#[test]
fn a_wake_does_not_discard_quiet_mail_it_scanned_past() {
    let home = two_sessions();
    let quiet = note(
        &home,
        "peer",
        "fyi",
        Some("harness:claude-code"),
        "quiet collaboration detail",
    );
    let loud = note(
        &home,
        "peer",
        "warning",
        Some("harness:claude-code"),
        "urgent change",
    );
    let (code, _, stderr, _) = finish(
        spawn_watch(
            &home,
            "me",
            &["--lifetime-secs", "5", "--coalesce-ms", "200"],
        ),
        Duration::from_secs(7),
    );
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains(&loud) && !stderr.contains(&quiet));
    let inbox = run(
        &home,
        &[
            "inbox",
            "--json",
            "--session",
            "me",
            "--harness",
            "claude-code",
        ],
    );
    let inbox: serde_json::Value = serde_json::from_slice(&inbox.stdout).unwrap();
    assert_eq!(inbox["events"][0]["id"], quiet);
    assert_eq!(
        inbox["returned"], 1,
        "the urgent note was already delivered"
    );
}

#[test]
fn a_prompt_receives_quiet_mail_but_not_the_already_delivered_wake() {
    let home = two_sessions();
    let quiet = note(
        &home,
        "peer",
        "fyi",
        Some("harness:claude-code"),
        "quiet for next prompt",
    );
    let loud = note(
        &home,
        "peer",
        "warning",
        Some("harness:claude-code"),
        "already woke me",
    );
    let (code, _, _, _) = finish(
        spawn_watch(
            &home,
            "me",
            &["--lifetime-secs", "5", "--coalesce-ms", "200"],
        ),
        Duration::from_secs(7),
    );
    assert_eq!(code, 2);
    let prompt = run(
        &home,
        &[
            "brief",
            "--touch",
            "--json",
            "--session",
            "me",
            "--harness",
            "claude-code",
        ],
    );
    let prompt: serde_json::Value = serde_json::from_slice(&prompt.stdout).unwrap();
    let text = prompt["brief"].as_str().unwrap();
    assert!(text.contains(&quiet), "{text}");
    assert!(!text.contains(&loud), "{text}");
}

fn long_session(home: &Path) -> String {
    use harbard_core::{Event, Store, clock};
    let store = Store::at(home.join(".harbard"));
    let began = clock::now_ms() - 3 * 86_400_000;
    let event = |ms: u64, event: &str, session: &str, harness: &str| -> Event {
        serde_json::from_value(serde_json::json!({
            "v": 1, "id": ulid::Ulid::from_parts(ms, 0).to_string(),
            "ts": clock::rfc3339_utc(ms), "event": event,
            "session": session, "harness": harness, "host": "h", "cwd": home,
        }))
        .unwrap()
    };
    store
        .append(&event(began, "hello", "me", "claude-code"))
        .unwrap();
    store
        .append(&event(began + 1, "hello", "peer", "codex"))
        .unwrap();
    let mut question = event(began + 2, "note", "me", "claude-code");
    question.text = "a question opened three days ago".into();
    question.kind = harbard_core::NoteKind::Ask;
    store.append(&question).unwrap();
    stop_check(home, "claude-code", &stop_payload(home, "me"));
    question.id
}

#[test]
fn a_long_session_keeps_its_join_and_can_wake_for_an_old_question_reply() {
    let home = tmp_home();
    let parent = long_session(&home);
    let child = spawn_watch(
        &home,
        "me",
        &["--lifetime-secs", "5", "--coalesce-ms", "200"],
    );
    armed();
    let reply = run(
        &home,
        &[
            "note",
            "--session",
            "peer",
            "--harness",
            "codex",
            "--ref",
            &parent,
            "answer after a calendar rollover",
        ],
    );
    assert!(reply.status.success());
    let (code, _, stderr, _) = finish(child, Duration::from_secs(7));
    assert_eq!(code, 2, "{stderr}");
    assert!(
        stderr.contains(&parent) && stderr.contains("answer after a calendar rollover"),
        "{stderr}"
    );
}

#[test]
fn a_long_session_receives_fresh_asks_and_can_ack_them() {
    let home = tmp_home();
    long_session(&home);
    let ask = note(
        &home,
        "peer",
        "ask",
        Some("session:me"),
        "fresh ask for an older session",
    );
    let held = stop_check(&home, "claude-code", &stop_payload(&home, "me"));
    assert!(String::from_utf8_lossy(&held.stdout).contains(&ask));
    let ack = run(
        &home,
        &["ack", "last", "--session", "me", "--harness", "claude-code"],
    );
    assert!(
        ack.status.success(),
        "{}",
        String::from_utf8_lossy(&ack.stderr)
    );
    assert!(
        stop_check(&home, "claude-code", &stop_payload(&home, "me"))
            .stdout
            .is_empty()
    );
}

#[test]
fn a_long_session_receives_quiet_mail_at_its_next_prompt() {
    let home = tmp_home();
    long_session(&home);
    let id = note(
        &home,
        "peer",
        "fyi",
        Some("harness:claude-code"),
        "quiet after three days",
    );
    let prompt = run(
        &home,
        &[
            "brief",
            "--touch",
            "--json",
            "--session",
            "me",
            "--harness",
            "claude-code",
        ],
    );
    let prompt: serde_json::Value = serde_json::from_slice(&prompt.stdout).unwrap();
    assert!(prompt["brief"].as_str().unwrap().contains(&id), "{prompt}");
}

#[test]
fn heartbeat_keeps_a_long_session_in_the_addressable_audience() {
    let home = tmp_home();
    long_session(&home);
    touch(&home, "me");
    let listed = run(&home, &["ls", "--json"]);
    assert!(listed.status.success());
    let rows: serde_json::Value = serde_json::from_slice(&listed.stdout).unwrap();
    let me = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["session"] == "me")
        .expect("an active heartbeat must not disappear at a calendar cutoff");
    assert_eq!(me["liveness"], "live", "{me}");
    let sent = run(
        &home,
        &[
            "note",
            "--session",
            "peer",
            "--harness",
            "codex",
            "--kind",
            "warning",
            "--to",
            "session:me",
            "a live long-running recipient",
        ],
    );
    assert!(sent.status.success());
    assert!(String::from_utf8_lossy(&sent.stderr).contains("1 live session(s) can read this"));
}

#[test]
fn a_live_long_session_keeps_its_callsign_across_calendar_rollover() {
    let home = tmp_home();
    long_session(&home);
    let named = harbard(&home)
        .env(
            "HARBARD_NOW_MS",
            (harbard_core::clock::now_ms() - 3 * 86_400_000).to_string(),
        )
        .args([
            "name",
            "Keeper",
            "--session",
            "me",
            "--harness",
            "claude-code",
        ])
        .output()
        .unwrap();
    assert!(
        named.status.success(),
        "{}",
        String::from_utf8_lossy(&named.stderr)
    );
    touch(&home, "me");
    hello(&home, "newcomer", "codex");
    let stolen = run(
        &home,
        &[
            "name",
            "Keeper",
            "--session",
            "newcomer",
            "--harness",
            "codex",
        ],
    );
    assert!(
        !stolen.status.success(),
        "a live callsign cannot be retired by the recent window"
    );
}

#[test]
fn inbox_marks_and_wake_receipts_follow_append_order_for_same_millisecond_notes() {
    use harbard_core::{Event, Store, clock};
    let home = two_sessions();
    stop_check(&home, "claude-code", &stop_payload(&home, "me"));
    std::thread::sleep(Duration::from_millis(10));
    let ms = clock::now_ms();
    let event = |random: u128, kind: &str, text: &str| -> Event {
        serde_json::from_value(serde_json::json!({
            "v": 1, "id": ulid::Ulid::from_parts(ms, random).to_string(),
            "ts": clock::rfc3339_utc(ms), "event": "note", "kind": kind,
            "session": "peer", "harness": "codex", "host": "h", "cwd": home,
            "to": {"kind": "session", "value": "me"}, "text": text,
        }))
        .unwrap()
    };
    let store = Store::at(home.join(".harbard"));
    let quiet = event(100, "fyi", "already read quiet note");
    let urgent = event(1, "warning", "later append with a smaller random suffix");
    store.append(&quiet).unwrap();
    let read = run(
        &home,
        &[
            "inbox",
            "--session",
            "me",
            "--harness",
            "claude-code",
            "--json",
        ],
    );
    assert!(read.status.success());
    assert!(String::from_utf8_lossy(&read.stdout).contains(&quiet.id));
    store.append(&urgent).unwrap();
    let (code, _, stderr, _) = finish(
        spawn_watch(&home, "me", &["--lifetime-secs", "2", "--coalesce-ms", "0"]),
        Duration::from_secs(5),
    );
    assert_eq!(code, 2, "fresh append must wake: {stderr}");
    assert!(stderr.contains(&urgent.id));
    note(
        &home,
        "peer",
        "warning",
        Some("session:me"),
        "another idle arrival",
    );
    let (code, _, stderr, _) = finish(
        spawn_watch(&home, "me", &["--lifetime-secs", "2", "--coalesce-ms", "0"]),
        Duration::from_secs(5),
    );
    assert_eq!(code, 2, "second arrival must wake: {stderr}");
    let read = run(
        &home,
        &[
            "inbox",
            "--session",
            "me",
            "--harness",
            "claude-code",
            "--json",
        ],
    );
    let inbox: serde_json::Value = serde_json::from_slice(&read.stdout).unwrap();
    assert_eq!(
        inbox["total"], 0,
        "both explicit wake receipts must survive"
    );
}

#[test]
fn cursor_first_inline_delivery_skips_pre_join_repository_notices() {
    let home = tmp_home();
    hello(&home, "peer", "codex");
    note(
        &home,
        "peer",
        "warning",
        Some(&format!("repo:{}", home.display())),
        "old repo warning before reader joined",
    );
    hello(&home, "me", "cursor");
    let args = [
        "brief",
        "--inline",
        "--throttle",
        "0",
        "--session",
        "me",
        "--harness",
        "cursor",
        "--cwd",
        home.to_str().unwrap(),
        "--repo",
        home.to_str().unwrap(),
        "--json",
    ];
    let first = run(&home, &args);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert!(
        !first["brief"]
            .as_str()
            .unwrap_or_default()
            .contains("old repo warning"),
        "pre-join context is SessionStart-only: {first}"
    );
    note(
        &home,
        "peer",
        "warning",
        Some("session:me"),
        "fresh post-join warning",
    );
    let next = run(&home, &args);
    let next: serde_json::Value = serde_json::from_slice(&next.stdout).unwrap();
    let text = next["brief"]
        .as_str()
        .expect("fresh warning must be delivered");
    assert!(text.contains("fresh post-join warning"));
    assert!(!text.contains("old repo warning"));
}
