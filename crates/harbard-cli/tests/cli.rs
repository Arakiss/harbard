//! End-to-end CLI tests for the F0.1 field fixes (M1.1b, M1.1c). Each runs the
//! real `harbard` binary with a fresh `--home`, so behavior is verified through
//! the same surface the hooks use, not an in-process shortcut. The child env is
//! cleared so ambient harness markers from the test runner never leak in.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp_home() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("harbard-cli-it-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("create temp home");
    d
}

/// Run the binary with a cleared environment. When `codex` is set, inject a
/// single `CODEX_*` marker to mimic a Codex-executed shell.
fn run(args: &[&str], codex: bool) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_harbard"));
    cmd.env_clear();
    if codex {
        cmd.env("CODEX_SANDBOX", "seatbelt");
    }
    cmd.args(args);
    cmd.output().expect("spawn harbard")
}

#[test]
fn wake_command_is_not_exposed() {
    let output = run(&["wake", "session:any"], false);
    assert!(!output.status.success(), "removed command must fail closed");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unrecognized subcommand 'wake'"),
        "{stderr}"
    );
}

#[test]
fn brief_touch_self_heals_missing_hello() {
    // M1.1b: a session that never announced itself appears on its first touch.
    let home = tmp_home();
    let hs = home.to_str().unwrap();

    let first = run(
        &[
            "brief",
            "--touch",
            "--home",
            hs,
            "--session",
            "mig-sess",
            "--harness",
            "codex",
            "--cwd",
            hs,
        ],
        false,
    );
    assert!(first.status.success(), "brief --touch exits 0");

    // ls now reports the self-healed session with its resolved harness.
    let ls = run(&["ls", "--json", "--all", "--home", hs], false);
    let sessions: serde_json::Value = serde_json::from_slice(&ls.stdout).expect("ls --json");
    let arr = sessions.as_array().expect("array");
    assert_eq!(arr.len(), 1, "exactly one session");
    assert_eq!(arr[0]["session"], "mig-sess");
    assert_eq!(arr[0]["harness"], "codex");

    // A second touch must not append a duplicate hello.
    run(
        &[
            "brief",
            "--touch",
            "--home",
            hs,
            "--session",
            "mig-sess",
            "--harness",
            "codex",
            "--cwd",
            hs,
        ],
        false,
    );
    let doctor = run(&["doctor", "--json", "--home", hs], false);
    let report: serde_json::Value = serde_json::from_slice(&doctor.stdout).expect("doctor --json");
    assert_eq!(
        report["events"].as_u64(),
        Some(1),
        "exactly one self-healed hello, no duplicate"
    );
}

#[test]
fn manual_verb_in_codex_shell_detects_codex() {
    // M1.1c: a hand-run verb inside a Codex shell records harness: codex.
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    let out = run(
        &[
            "hello",
            "--json",
            "--home",
            hs,
            "--session",
            "cdx",
            "--cwd",
            hs,
        ],
        true,
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(r#""harness":"codex""#),
        "codex markers must resolve to codex, got: {stdout}"
    );
}

#[test]
fn a_session_introduces_itself_once_after_automatic_hello() {
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    let hello = run(
        &[
            "hello",
            "--home",
            hs,
            "--session",
            "root",
            "--harness",
            "codex",
            "--cwd",
            hs,
        ],
        false,
    );
    assert!(hello.status.success());
    let unnamed_brief = run(
        &[
            "brief",
            "--home",
            hs,
            "--session",
            "root",
            "--harness",
            "codex",
            "--cwd",
            hs,
        ],
        false,
    );
    // Unnamed is not a fault: hello already derived a callsign, and the brief
    // never asks for one.
    assert!(
        unnamed_brief.stdout.is_empty(),
        "{}",
        String::from_utf8_lossy(&unnamed_brief.stdout)
    );
    let derived: serde_json::Value =
        serde_json::from_slice(&std::fs::read(home.join("aliases.json")).expect("aliases ledger"))
            .expect("aliases json");
    assert!(derived["by_session"]["root"].is_string(), "{derived}");

    let named = run(
        &[
            "name",
            "Nova",
            "--home",
            hs,
            "--session",
            "root",
            "--harness",
            "codex",
            "--cwd",
            hs,
        ],
        false,
    );
    assert!(
        named.status.success(),
        "{}",
        String::from_utf8_lossy(&named.stderr)
    );
    let aliases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(home.join("aliases.json")).expect("aliases ledger"))
            .expect("aliases json");
    assert_eq!(aliases["by_session"]["root"], "nova");
    assert_eq!(aliases["self_named"][0], "root");
    let named_brief = run(
        &[
            "brief",
            "--home",
            hs,
            "--session",
            "root",
            "--harness",
            "codex",
            "--cwd",
            hs,
        ],
        false,
    );
    assert!(!String::from_utf8_lossy(&named_brief.stdout).contains("name_required"));

    let rename = run(
        &[
            "name",
            "Kora",
            "--home",
            hs,
            "--session",
            "root",
            "--harness",
            "codex",
            "--cwd",
            hs,
        ],
        false,
    );
    assert!(!rename.status.success());
    assert!(String::from_utf8_lossy(&rename.stderr).contains("already introduced"));
}

#[test]
fn name_refuses_generic_misshapen_and_recently_used_names_with_a_reason() {
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    let as_session = |verb: &[&str], session: &str| {
        let mut args = verb.to_vec();
        args.extend([
            "--home",
            hs,
            "--session",
            session,
            "--harness",
            "codex",
            "--cwd",
            hs,
        ]);
        run(&args, false)
    };
    assert!(as_session(&["hello"], "first").status.success());
    assert!(as_session(&["name", "Vesper"], "first").status.success());
    assert!(as_session(&["bye"], "first").status.success());

    assert!(as_session(&["hello"], "second").status.success());
    for (name, reason) in [
        ("Wrench", "generic word"),
        ("R2D2", "3-12 letters"),
        ("Vesper", "last 90 days"),
    ] {
        let refused = as_session(&["name", name], "second");
        assert_eq!(refused.status.code(), Some(2), "{name}");
        let stderr = String::from_utf8_lossy(&refused.stderr);
        assert!(stderr.contains(reason), "{name}: {stderr}");
        assert!(stderr.contains("choose"), "{name}: {stderr}");
    }
    let accepted = as_session(&["name", "Bramwell"], "second");
    assert!(
        accepted.status.success(),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );

    let aliases: serde_json::Value =
        serde_json::from_slice(&std::fs::read(home.join("aliases.json")).expect("aliases ledger"))
            .expect("aliases json");
    assert!(
        aliases["by_session"].get("first").is_none(),
        "ended session released"
    );
    assert_eq!(aliases["history"]["vesper"]["session"], "first");
}

#[test]
fn ls_json_names_every_session_as_the_fleet_board_does() {
    // A status line reads `ls --json`; a desktop app reads the fleet board. If
    // they resolved names separately, the same session would carry two names.
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    let as_session = |verb: &[&str], session: &str| {
        let mut args = verb.to_vec();
        args.extend([
            "--home",
            hs,
            "--session",
            session,
            "--harness",
            "codex",
            "--cwd",
            hs,
        ]);
        run(&args, false)
    };
    let named = "019a2b3c-0000-7000-8000-00000000aaaa";
    let unnamed = "019a2b3c-0000-7000-8000-00000000bbbb";
    assert!(as_session(&["hello"], named).status.success());
    assert!(as_session(&["name", "Bramwell"], named).status.success());
    assert!(as_session(&["hello"], unnamed).status.success());

    let ls = run(&["ls", "--json", "--all", "--home", hs], false);
    let rows: serde_json::Value = serde_json::from_slice(&ls.stdout).expect("ls --json");
    let ls_name = |session: &str| -> String {
        rows.as_array()
            .expect("array")
            .iter()
            .find(|row| row["session"] == session)
            .and_then(|row| row["display_name"].as_str())
            .unwrap_or_else(|| panic!("{session} has a display_name in ls --json: {rows}"))
            .to_string()
    };
    assert_eq!(ls_name(named), "Bramwell", "a chosen name wins");
    let derived = ls_name(unnamed);
    assert!(!derived.is_empty());
    assert_ne!(derived, unnamed, "a uuid is not a name");

    let fleet = run(&["fleet", "--json", "--home", hs], false);
    let board: serde_json::Value = serde_json::from_slice(&fleet.stdout).expect("fleet --json");
    let cards = board["sessions"].as_array().expect("fleet sessions");
    for session in [named, unnamed] {
        let card = cards
            .iter()
            .find(|card| card["session"] == session)
            .unwrap_or_else(|| panic!("{session} on the board"));
        assert_eq!(card["display_name"], ls_name(session), "{session}");
    }
}

#[test]
fn manual_verb_without_markers_is_other() {
    // The fallback stays `other` when no harness marker is present.
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    let out = run(
        &[
            "hello",
            "--json",
            "--home",
            hs,
            "--session",
            "pln",
            "--cwd",
            hs,
        ],
        false,
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains(r#""harness":"other""#),
        "no markers must resolve to other, got: {stdout}"
    );
}

#[test]
fn call_emits_exactly_one_stable_json_envelope() {
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    let out = run(&["--home", hs, "call", "protocol", "--input", "{}"], false);
    assert!(out.status.success());
    assert_eq!(out.stdout.iter().filter(|byte| **byte == b'\n').count(), 1);
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["v"], "harbard/cli/v1");
    assert_eq!(value["ok"], true);
    assert_eq!(value["operation"], "protocol");
    assert_eq!(value["data"]["version"], "harbard/protocol/v1");
    let operations = value["data"]["machine"]["finite"]["operations"]
        .as_array()
        .unwrap();
    assert!(operations.iter().any(|operation| operation == "hello"));
    assert!(operations.iter().any(|operation| operation == "name"));
    assert!(operations.iter().any(|operation| operation == "note"));
    assert!(operations.iter().any(|operation| operation == "bye"));
}

#[test]
fn call_errors_are_stable_json_not_free_form_stderr() {
    let out = run(&["call", "missing", "--input", "[]"], false);
    assert_eq!(out.status.code(), Some(2));
    assert!(out.stderr.is_empty());
    let value: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(value["v"], "harbard/cli/v1");
    assert_eq!(value["ok"], false);
    assert_eq!(value["error"]["code"], "invalid_input");
}

#[test]
fn subscribe_snapshot_is_json_lines_with_terminal_end_frame() {
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    let _ = run(
        &[
            "--home",
            hs,
            "note",
            "hello",
            "--session",
            "sender",
            "--harness",
            "codex",
        ],
        false,
    );
    let out = run(
        &["--home", hs, "subscribe", "--once", "--event", "note"],
        false,
    );
    assert!(out.status.success());
    let frames = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(frames.first().unwrap()["type"], "ready");
    assert_eq!(frames.last().unwrap()["type"], "end");
    assert!(
        frames
            .iter()
            .any(|frame| frame["type"] == "event" && frame["event"]["text"] == "hello")
    );
    assert!(frames.iter().all(|frame| frame["v"] == "harbard/stream/v1"));
}

#[test]
fn subscribe_rejects_unknown_event_filter_with_jsonl_error() {
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    let out = run(
        &["--home", hs, "subscribe", "--once", "--event", "bogus"],
        false,
    );
    assert_eq!(out.status.code(), Some(2));
    let frame: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(frame["v"], "harbard/stream/v1");
    assert_eq!(frame["type"], "error");
    assert_eq!(frame["error"]["code"], "invalid_filter");
}

#[test]
fn f2_inline_uses_the_hook_envelope_and_throttles() {
    // The hot path's delivery contract. Plain stdout is not delivery under
    // PreToolUse — a harness logs it and the agent never sees it — so
    // `--inline --hook` must emit the context-injection envelope.
    let home = tmp_home();
    let hs = home.to_str().unwrap();

    // F5: the kind decides delivery, not just display. An fyi is real but
    // waits for a turn boundary; only a warning or a handoff earns a place
    // between tool calls.
    run(
        &[
            "--home",
            hs,
            "note",
            "background context",
            "--session",
            "other",
            "--harness",
            "codex",
            "--cwd",
            "/tmp",
        ],
        false,
    );
    // The reader joins the way SessionStart joins it: a bare hello, no
    // intent and no chosen name, neither of which is a reason to speak.
    run(
        &[
            "--home",
            hs,
            "hello",
            "--session",
            "me",
            "--harness",
            "claude-code",
            "--cwd",
            "/tmp",
        ],
        false,
    );
    let quiet = run(
        &[
            "--home",
            hs,
            "--session",
            "me",
            "--harness",
            "claude-code",
            "--cwd",
            "/tmp",
            "brief",
            "--inline",
            "--hook",
            "claude-code",
        ],
        false,
    );
    assert!(
        String::from_utf8_lossy(&quiet.stdout).trim().is_empty(),
        "an fyi must not interrupt between tool calls"
    );

    run(
        &[
            "--home",
            hs,
            "note",
            "mind the branch",
            "--kind",
            "warning",
            "--to",
            "session:me",
            "--session",
            "other",
            "--harness",
            "codex",
            "--cwd",
            "/tmp",
        ],
        false,
    );

    let out = run(
        &[
            "--home",
            hs,
            "--session",
            "me",
            "--harness",
            "claude-code",
            "--cwd",
            "/tmp",
            "brief",
            "--inline",
            "--hook",
            "claude-code",
        ],
        false,
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let v: serde_json::Value = serde_json::from_str(stdout.trim()).expect("hook envelope is JSON");
    assert_eq!(v["hookSpecificOutput"]["hookEventName"], "PreToolUse");
    let ctx = v["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .expect("additionalContext");
    assert!(ctx.contains("mind the branch"), "got {ctx}");

    // Delivered once: the same message must not repeat on every tool call.
    let again = run(
        &[
            "--home",
            hs,
            "--session",
            "me",
            "--harness",
            "claude-code",
            "--cwd",
            "/tmp",
            "brief",
            "--inline",
            "--hook",
            "claude-code",
        ],
        false,
    );
    assert!(
        String::from_utf8_lossy(&again.stdout).trim().is_empty(),
        "an already-delivered message must not repeat"
    );

    // A fresh note is withheld while the throttle window is open, so the hot
    // path does not read the store on every single tool call.
    run(
        &[
            "--home",
            hs,
            "note",
            "second message",
            "--kind",
            "warning",
            "--to",
            "session:me",
            "--session",
            "other",
            "--harness",
            "codex",
            "--cwd",
            "/tmp",
        ],
        false,
    );
    let throttled = run(
        &[
            "--home",
            hs,
            "--session",
            "me",
            "--harness",
            "claude-code",
            "--cwd",
            "/tmp",
            "--json",
            "brief",
            "--inline",
            "--throttle",
            "300",
        ],
        false,
    );
    let v: serde_json::Value = serde_json::from_slice(&throttled.stdout).expect("json output");
    assert_eq!(v["throttled"], true);
    assert!(v["brief"].is_null());
}

#[test]
fn brief_never_asks_for_intent_or_a_name() {
    // The intent and name reminders were most of what the hooks ever said,
    // and neither asked anything the session had to act on. A session with
    // no declared work and no chosen name stays quiet on every prompt.
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    let base = 1_800_000_000_000u64;
    let at = |minutes: u64, args: &[&str]| {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_harbard"));
        cmd.env_clear()
            .env("HARBARD_NOW_MS", (base + minutes * 60_000).to_string())
            .args(args)
            .args([
                "--home",
                hs,
                "--session",
                "quiet-sess",
                "--harness",
                "codex",
                "--cwd",
                hs,
            ]);
        cmd.output().expect("spawn harbard")
    };
    assert!(at(0, &["hello"]).status.success());
    for minutes in [0, 1, 21, 42] {
        let out = at(minutes, &["brief", "--touch", "--json"]);
        assert!(out.status.success());
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).expect("json");
        assert!(v["brief"].is_null(), "+{minutes}m: {v}");
    }
}

#[test]
fn brief_asks_for_ack_when_handoff_unacked() {
    let home = tmp_home();
    let hs = home.to_str().unwrap();

    let author = run(
        &[
            "hello",
            "--home",
            hs,
            "--session",
            "author",
            "--harness",
            "codex",
            "--cwd",
            hs,
        ],
        false,
    );
    assert!(author.status.success());
    let reader_hello = run(
        &[
            "hello",
            "--home",
            hs,
            "--session",
            "reader",
            "--harness",
            "codex",
            "--cwd",
            hs,
        ],
        false,
    );
    assert!(reader_hello.status.success());

    let note = run(
        &[
            "note",
            "please take this",
            "--kind",
            "handoff",
            "--to",
            "session:reader",
            "--home",
            hs,
            "--session",
            "author",
            "--harness",
            "codex",
            "--cwd",
            hs,
        ],
        false,
    );
    assert!(note.status.success());

    let brief = run(
        &[
            "brief",
            "--touch",
            "--json",
            "--home",
            hs,
            "--session",
            "reader",
            "--harness",
            "codex",
            "--cwd",
            hs,
        ],
        false,
    );
    assert!(brief.status.success());
    let v: serde_json::Value = serde_json::from_slice(&brief.stdout).expect("json");
    let text = v["brief"].as_str().unwrap_or("");
    // The nudge no longer names "handoff": an `ask` creates the same obligation
    // for every live reader it names, and one wording has to cover both.
    assert!(
        text.contains("ack_pending") && text.contains("waiting on your answer"),
        "expected ack_pending nudge, got: {text:?}"
    );
}

#[test]
fn bye_is_idempotent_when_already_departed() {
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    assert!(
        run(
            &[
                "hello",
                "--home",
                hs,
                "--session",
                "s1",
                "--harness",
                "codex",
                "--cwd",
                hs
            ],
            false,
        )
        .status
        .success()
    );
    assert!(
        run(
            &[
                "bye",
                "--home",
                hs,
                "--session",
                "s1",
                "--harness",
                "codex",
                "--cwd",
                hs
            ],
            false,
        )
        .status
        .success()
    );
    let second = run(
        &[
            "bye",
            "--json",
            "--home",
            hs,
            "--session",
            "s1",
            "--harness",
            "codex",
            "--cwd",
            hs,
        ],
        false,
    );
    assert!(second.status.success());
    let v: serde_json::Value = serde_json::from_slice(&second.stdout).expect("json");
    assert_eq!(v["bye"], "already");
}

#[test]
fn channels_create_retired_join_is_happy_path() {
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    let create = run(
        &[
            "channels",
            "--create",
            "ghosts",
            "--home",
            hs,
            "--session",
            "s1",
            "--harness",
            "codex",
            "--cwd",
            hs,
        ],
        false,
    );
    assert!(
        !create.status.success(),
        "orphan --create must fail: {}",
        String::from_utf8_lossy(&create.stderr)
    );
    let err = String::from_utf8_lossy(&create.stderr);
    assert!(err.contains("retired") || err.contains("join"), "got {err}");

    let join = run(
        &[
            "join",
            "ops",
            "--home",
            hs,
            "--session",
            "s1",
            "--harness",
            "codex",
            "--cwd",
            hs,
            "--as",
            "session",
        ],
        false,
    );
    assert!(
        join.status.success(),
        "{}",
        String::from_utf8_lossy(&join.stderr)
    );
}

#[test]
fn channel_delete_requires_leave_and_archive_then_removes_the_name() {
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    let common = [
        "--home",
        hs,
        "--session",
        "s1",
        "--harness",
        "codex",
        "--cwd",
        hs,
    ];

    let join = run(
        &[
            "join", "obsolete", "--as", "session", common[0], common[1], common[2], common[3],
            common[4], common[5], common[6], common[7],
        ],
        false,
    );
    assert!(join.status.success());

    let busy_delete = run(
        &[
            "channels", "--delete", "obsolete", common[0], common[1], common[2], common[3],
            common[4], common[5], common[6], common[7],
        ],
        false,
    );
    assert!(!busy_delete.status.success());

    for args in [
        vec!["leave", "obsolete", "--as", "session"],
        vec!["channels", "--archive", "obsolete"],
        vec!["channels", "--delete", "obsolete"],
    ] {
        let mut full = args;
        full.extend(common);
        let output = run(&full, false);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let list = run(
        &[
            "channels", "--all", "--json", common[0], common[1], common[2], common[3], common[4],
            common[5], common[6], common[7],
        ],
        false,
    );
    assert!(list.status.success());
    let value: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert!(value["channels"].get("obsolete").is_none());
    assert!(
        !value["archived"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| name == "obsolete")
    );
}

#[test]
fn rooms_alias_hides_limited_rooms_from_non_members() {
    use harbard_core::Store;
    use harbard_core::membership::{self, AnchorKind, Member, Memberships, RoomVisibility};

    let home = tmp_home();
    let hs = home.to_str().unwrap();
    let store = Store::at(&home);
    let mut rooms = Memberships::default();
    let member = Member {
        kind: AnchorKind::Session,
        value: "member".into(),
    };
    membership::join(&mut rooms, "shared", member.clone());
    membership::join(&mut rooms, "private-ops", member);
    membership::update_room(
        &mut rooms,
        "private-ops",
        Some("Sensitive coordination"),
        RoomVisibility::Limited,
    )
    .unwrap();
    membership::write(&store, &rooms).unwrap();

    let common = [
        "--json",
        "--home",
        hs,
        "--session",
        "outsider",
        "--harness",
        "codex",
        "--cwd",
        hs,
    ];
    let visible = run(
        &[
            "rooms", common[0], common[1], common[2], common[3], common[4], common[5], common[6],
            common[7], common[8],
        ],
        false,
    );
    assert!(
        visible.status.success(),
        "{}",
        String::from_utf8_lossy(&visible.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&visible.stdout).unwrap();
    assert!(value["channels"].get("shared").is_some());
    assert!(value["channels"].get("private-ops").is_none());

    let audit = run(
        &[
            "rooms", "--all", common[0], common[1], common[2], common[3], common[4], common[5],
            common[6], common[7], common[8],
        ],
        false,
    );
    assert!(
        audit.status.success(),
        "{}",
        String::from_utf8_lossy(&audit.stderr)
    );
    let value: serde_json::Value = serde_json::from_slice(&audit.stdout).unwrap();
    assert_eq!(value["meta"]["private-ops"]["visibility"], "limited");
}

#[test]
fn joining_a_room_as_the_only_checkout_says_it_is_a_repo_alias() {
    // The default anchor is `repo`, so the default join builds a room that
    // delivers exactly what `--to repo:<path>` delivers. Warn at the join —
    // the only moment the agent is still choosing — and keep it non-blocking:
    // the room is real, it is just redundant.
    let home = tmp_home();
    let hs = home.to_str().unwrap();

    let first = run(
        &[
            "join",
            "deploys",
            "--home",
            hs,
            "--session",
            "s1",
            "--harness",
            "codex",
            "--cwd",
            hs,
            "--repo",
            "/r",
        ],
        false,
    );
    assert!(first.status.success(), "advice never fails the join");
    let err = String::from_utf8_lossy(&first.stderr);
    assert!(err.contains("channel_repo_alias"), "got {err}");
    assert!(
        err.contains("--to repo:"),
        "names the cheaper address: {err}"
    );

    // A second checkout gives the room an audience no repo target can name,
    // and the warning must stop.
    let second = run(
        &[
            "join",
            "deploys",
            "--home",
            hs,
            "--session",
            "s2",
            "--harness",
            "claude-code",
            "--cwd",
            hs,
            "--repo",
            "/r2",
        ],
        false,
    );
    assert!(second.status.success());
    let err2 = String::from_utf8_lossy(&second.stderr);
    assert!(!err2.contains("channel_repo_alias"), "got {err2}");
}

#[test]
fn note_to_empty_channel_fails_without_append() {
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    // Seed an empty channel via snapshot write is not exposed; leave after join.
    assert!(
        run(
            &[
                "join",
                "emptyroom",
                "--home",
                hs,
                "--session",
                "s1",
                "--harness",
                "codex",
                "--cwd",
                hs,
                "--as",
                "session",
            ],
            false,
        )
        .status
        .success()
    );
    assert!(
        run(
            &[
                "leave",
                "emptyroom",
                "--home",
                hs,
                "--session",
                "s1",
                "--harness",
                "codex",
                "--cwd",
                hs,
                "--as",
                "session",
            ],
            false,
        )
        .status
        .success()
    );
    let note = run(
        &[
            "note",
            "--to",
            "channel:emptyroom",
            "nobody home",
            "--home",
            hs,
            "--session",
            "s1",
            "--harness",
            "codex",
            "--cwd",
            hs,
        ],
        false,
    );
    assert!(!note.status.success());
    let err = String::from_utf8_lossy(&note.stderr);
    assert!(
        err.contains("channel_no_recipients"),
        "expected channel_no_recipients, got {err}"
    );
    let doctor = run(&["doctor", "--json", "--home", hs], false);
    let report: serde_json::Value = serde_json::from_slice(&doctor.stdout).expect("doctor");
    // join + leave events only — note must not have appended.
    let events = report["events"].as_u64().unwrap_or(0);
    assert!(
        events <= 2,
        "note must not append on zero-member channel; events={events}"
    );
}

#[test]
fn handoff_rejects_non_session_targets() {
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    assert!(
        run(
            &[
                "hello",
                "--home",
                hs,
                "--session",
                "peer",
                "--harness",
                "codex",
                "--cwd",
                hs,
            ],
            false,
        )
        .status
        .success()
    );
    assert!(
        run(
            &[
                "join",
                "room",
                "--home",
                hs,
                "--session",
                "peer",
                "--harness",
                "codex",
                "--cwd",
                hs,
                "--as",
                "session",
            ],
            false,
        )
        .status
        .success()
    );

    for to in ["channel:room", "harness:codex", "repo:/tmp/x"] {
        let out = run(
            &[
                "note",
                "--kind",
                "handoff",
                "--to",
                to,
                "take this",
                "--home",
                hs,
                "--session",
                "peer",
                "--harness",
                "codex",
                "--cwd",
                hs,
            ],
            false,
        );
        assert!(!out.status.success(), "handoff to {to} must fail");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(
            err.contains("handoff_target_cardinality"),
            "to={to} got {err}"
        );
    }

    let ok = run(
        &[
            "note",
            "--kind",
            "handoff",
            "--to",
            "session:peer",
            "take this",
            "--home",
            hs,
            "--session",
            "other",
            "--harness",
            "cursor",
            "--cwd",
            hs,
        ],
        false,
    );
    assert!(
        ok.status.success(),
        "handoff to live session: {}",
        String::from_utf8_lossy(&ok.stderr)
    );

    assert!(
        run(
            &[
                "bye",
                "--home",
                hs,
                "--session",
                "peer",
                "--harness",
                "codex",
                "--cwd",
                hs,
            ],
            false,
        )
        .status
        .success()
    );
    let ended = run(
        &[
            "note",
            "--kind",
            "handoff",
            "--to",
            "session:peer",
            "cannot be closed",
            "--home",
            hs,
            "--session",
            "other",
            "--harness",
            "cursor",
            "--cwd",
            hs,
        ],
        false,
    );
    assert!(!ended.status.success());
    assert!(String::from_utf8_lossy(&ended.stderr).contains("handoff_target_cardinality"));
}

/// Session-level hooks leave a long autonomous turn without any signal, so the
/// clock alone called the recipient Stale and refused it handoffs. A heartbeat
/// bound to a supervisor that is still running — same PID AND same start
/// marker — keeps it addressable. `--stale-secs 0` stands in for the long turn;
/// this test process stands in for the harness.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[test]
fn a_handoff_reaches_a_long_turn_bound_to_a_running_supervisor() {
    use harbard_core::cursor::{LiveState, write_live_state};
    use harbard_core::{PidChecker, Store, SystemPidChecker};

    let home = tmp_home();
    let hs = home.to_str().unwrap();
    assert!(
        run(
            &[
                "hello",
                "--home",
                hs,
                "--session",
                "peer",
                "--harness",
                "codex",
                "--cwd",
                hs,
            ],
            false,
        )
        .status
        .success()
    );
    std::thread::sleep(std::time::Duration::from_millis(20));

    let me = std::process::id();
    let start = SystemPidChecker.start_time(me).expect("own start marker");
    let store = Store::at(&home);
    let bind = |marker: u64| {
        let state = LiveState {
            checked_ms: 1,
            supervisor_pid: Some(me),
            supervisor_start: Some(marker),
            ..Default::default()
        };
        write_live_state(&store, "peer", &state).expect("write live cursor");
    };
    let handoff = || {
        run(
            &[
                "note",
                "--kind",
                "handoff",
                "--to",
                "session:peer",
                "take this",
                "--stale-secs",
                "0",
                "--home",
                hs,
                "--session",
                "other",
                "--harness",
                "cursor",
                "--cwd",
                hs,
            ],
            false,
        )
    };

    // Same PID, different start: a recycled PID is not the bound process.
    bind(start.wrapping_add(1));
    let refused = handoff();
    assert!(!refused.status.success(), "recycled pid must not be live");
    assert!(String::from_utf8_lossy(&refused.stderr).contains("handoff_target_cardinality"));

    bind(start);
    let ok = handoff();
    assert!(
        ok.status.success(),
        "handoff to a process-bound session: {}",
        String::from_utf8_lossy(&ok.stderr)
    );
}

/// A note to a session id nobody owns used to exit 0 with a normal receipt and
/// reach nobody, which is how an "announce before you touch" warning silently
/// failed to warn anyone. Observed in this repo: the id was off by four
/// characters in the middle of a uuid.
#[test]
fn a_note_to_an_unknown_session_is_refused_and_a_prefix_expands() {
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    let real = "aaaaaaaa-1111-2222-3333-444444444444";

    // The peer has to exist on the bus before it can be addressed.
    assert!(
        run(
            &[
                "hello",
                "--home",
                hs,
                "--session",
                real,
                "--harness",
                "codex",
                "--cwd",
                hs,
            ],
            false,
        )
        .status
        .success()
    );

    let bogus = run(
        &[
            "note",
            "warning about an imminent write",
            "--to",
            "session:aaaaaaaa-1111-2222-3333-999999999999",
            "--kind",
            "warning",
            "--home",
            hs,
            "--session",
            "me",
            "--harness",
            "claude-code",
            "--cwd",
            hs,
        ],
        false,
    );
    assert!(
        !bogus.status.success(),
        "an unknown session id must not be accepted: {bogus:?}"
    );
    let err = String::from_utf8_lossy(&bogus.stderr) + String::from_utf8_lossy(&bogus.stdout);
    assert!(
        err.contains("no session on this bus"),
        "the refusal must say why: {err}"
    );

    // Every surface prints a truncated id, so a prefix is the natural paste.
    let prefix = run(
        &[
            "note",
            "same warning, addressed by prefix",
            "--to",
            "session:aaaaaaaa-1111",
            "--kind",
            "warning",
            "--home",
            hs,
            "--session",
            "me",
            "--harness",
            "claude-code",
            "--cwd",
            hs,
            "--json",
        ],
        false,
    );
    assert!(prefix.status.success(), "{prefix:?}");
    let out = String::from_utf8_lossy(&prefix.stdout);
    assert!(
        out.contains(real),
        "a unique prefix must be stored expanded, got {out}"
    );
}

fn callsign_command(home: &std::path::Path, session: &str, args: &[&str]) -> Output {
    let hs = home.to_str().unwrap();
    let mut command = vec![
        "--home",
        hs,
        "--cwd",
        hs,
        "--session",
        session,
        "--harness",
        "codex",
        "--json",
    ];
    command.extend_from_slice(args);
    run(&command, false)
}

#[test]
fn unknown_lease_command_explains_reservations_without_touching_the_store() {
    let home = tmp_home();
    std::fs::write(home.join("sentinel"), b"unchanged").unwrap();
    let snapshot = || {
        let mut files: Vec<_> = std::fs::read_dir(&home)
            .unwrap()
            .map(|entry| {
                let path = entry.unwrap().path();
                (
                    path.file_name().unwrap().to_owned(),
                    std::fs::read(&path).unwrap(),
                )
            })
            .collect();
        files.sort();
        files
    };
    let before = snapshot();
    for args in [
        vec!["lease", "--help"],
        vec!["lease", "check"],
        vec!["lease", "check", "--service", "fixture-service"],
        vec!["lease", "--writer"],
    ] {
        let output = callsign_command(&home, "sender", &args);
        assert_eq!(output.status.code(), Some(2), "{output:?}");
        assert!(output.stdout.is_empty(), "{output:?}");
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        for expected in [
            "unrecognized subcommand 'lease'",
            "harbard claim --writer",
            "harbard claim --service NAME",
            "harbard check --writer",
            "harbard check --service NAME",
            "harbard release --writer",
            "harbard release --service NAME",
            "harbard claim --help",
        ] {
            assert!(
                diagnostic.contains(expected),
                "missing {expected}: {diagnostic}"
            );
        }
        assert_eq!(snapshot(), before);
    }
    let unrelated = callsign_command(&home, "lease", &["unrelated-unknown"]);
    assert_eq!(unrelated.status.code(), Some(2), "{unrelated:?}");
    assert!(!String::from_utf8_lossy(&unrelated.stderr).contains("Reservations use"));
    let help = callsign_command(&home, "sender", &["claim", "--help"]);
    assert!(help.status.success(), "{help:?}");
    assert!(String::from_utf8_lossy(&help.stdout).contains("--writer"));
    assert!(String::from_utf8_lossy(&help.stdout).contains("--service <NAME>"));
    assert_eq!(snapshot(), before);
}

fn callsign_ledger(home: &std::path::Path, entries: &[(&str, &str)]) -> Vec<u8> {
    let by_session: serde_json::Map<String, serde_json::Value> = entries
        .iter()
        .map(|(session, name)| ((*session).into(), serde_json::json!(name)))
        .collect();
    let bytes =
        serde_json::to_vec(&serde_json::json!({"by_session": by_session, "self_named": []}))
            .unwrap();
    std::fs::write(home.join("aliases.json"), &bytes).unwrap();
    bytes
}

fn callsign_note_count(home: &std::path::Path) -> usize {
    harbard_core::Store::at(home)
        .read(harbard_core::Window::All)
        .events
        .iter()
        .filter(|event| event.event == harbard_core::EventKind::Note)
        .count()
}

#[test]
fn callsign_note_resolves_case_insensitively_without_writing_aliases() {
    let home = tmp_home();
    let peer = "aaaaaaaa-1111-2222-3333-444444444444";
    for session in [peer, "sender"] {
        assert!(
            callsign_command(&home, session, &["hello"])
                .status
                .success()
        );
    }
    let bytes = callsign_ledger(&home, &[(peer, "northstar")]);
    for kind in ["fyi", "handoff"] {
        let output = callsign_command(
            &home,
            "sender",
            &[
                "note",
                "Synthetic task",
                "--to",
                "session:NoRtHsTaR",
                "--kind",
                kind,
            ],
        );
        assert!(output.status.success(), "{output:?}");
        assert!(String::from_utf8_lossy(&output.stdout).contains(peer));
        assert_eq!(std::fs::read(home.join("aliases.json")).unwrap(), bytes);
    }
    assert_eq!(callsign_note_count(&home), 2);
}

#[test]
fn callsign_note_rejects_reused_unknown_partial_and_inactive_aliases() {
    let home = tmp_home();
    for session in ["live-peer", "other-live-peer", "ended-peer", "sender"] {
        assert!(
            callsign_command(&home, session, &["hello"])
                .status
                .success()
        );
    }
    assert!(
        callsign_command(&home, "ended-peer", &["bye"])
            .status
            .success()
    );
    let cases = [
        (
            vec![("live-peer", "northstar"), ("other-live-peer", "NORTHSTAR")],
            "northstar",
            "ambiguous callsign",
        ),
        (
            vec![("live-peer", "northstar"), ("ended-peer", "NORTHSTAR")],
            "northstar",
            "ambiguous callsign",
        ),
        (
            vec![("live-peer", "northstar"), ("missing-peer", "NORTHSTAR")],
            "northstar",
            "ambiguous callsign",
        ),
        (
            vec![("missing-peer", "northstar")],
            "northstar",
            "no current session",
        ),
        (vec![("ended-peer", "northstar")], "northstar", "not live"),
        (
            vec![("live-peer", "northstar")],
            "north",
            "no session on this bus",
        ),
    ];
    for (entries, target, diagnostic) in cases {
        let bytes = callsign_ledger(&home, &entries);
        let recipient = format!("session:{target}");
        let output = callsign_command(
            &home,
            "sender",
            &["note", "Synthetic task", "--to", &recipient],
        );
        assert!(!output.status.success(), "{output:?}");
        assert!(
            String::from_utf8_lossy(&output.stderr).contains(diagnostic),
            "{output:?}"
        );
        assert_eq!(callsign_note_count(&home), 0);
        assert_eq!(std::fs::read(home.join("aliases.json")).unwrap(), bytes);
    }
    let historical = callsign_command(
        &home,
        "sender",
        &["note", "Historical FYI", "--to", "session:ended-peer"],
    );
    assert!(historical.status.success(), "{historical:?}");
}

#[test]
fn callsign_note_rejects_stale_sessions() {
    let home = tmp_home();
    assert!(
        callsign_command(&home, "sender", &["hello"])
            .status
            .success()
    );
    let store = harbard_core::Store::at(&home);
    let mut peer = store.read(harbard_core::Window::All).events[0].clone();
    peer.id = "00000000000000000000000001".into();
    peer.session = "peer".into();
    peer.ts = "1970-01-01T00:00:00.000Z".into();
    assert_eq!(harbard_core::query::event_ms(&peer), 0);
    store.append(&peer).unwrap();
    callsign_ledger(&home, &[("peer", "northstar")]);
    let output = callsign_command(
        &home,
        "sender",
        &["note", "Synthetic task", "--to", "session:northstar"],
    );
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("not live"),
        "{output:?}"
    );
    assert_eq!(callsign_note_count(&home), 0);
}

#[test]
fn callsign_note_rejects_a_child_in_ending_grace() {
    let home = tmp_home();
    assert!(
        callsign_command(&home, "parent", &["hello"])
            .status
            .success()
    );
    let store = harbard_core::Store::at(&home);
    let mut child = store.read(harbard_core::Window::All).events[0].clone();
    child.id = ulid::Ulid::generate().to_string();
    child.session = "child".into();
    child.role = harbard_core::Role::Herd;
    child.parent = Some("parent".into());
    child.event = harbard_core::EventKind::Bye;
    store.append(&child).unwrap();
    let read = store.read(harbard_core::Window::All);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let views = harbard_core::query::sessions(&store, &read.events, now, 900);
    let view = views.iter().find(|view| view.session == "child").unwrap();
    assert_eq!(view.liveness, harbard_core::Liveness::Live);
    assert!(view.ending);
    callsign_ledger(&home, &[("child", "northstar")]);
    let output = callsign_command(
        &home,
        "sender",
        &["note", "Synthetic task", "--to", "session:northstar"],
    );
    assert!(!output.status.success(), "{output:?}");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("not live"),
        "{output:?}"
    );
    assert_eq!(callsign_note_count(&home), 0);
}

#[test]
fn callsign_note_preserves_exact_id_and_prefix_precedence() {
    let home = tmp_home();
    for session in [
        "northstar",
        "northstar-longer",
        "prefix-peer",
        "ambiguous-one",
        "ambiguous-two",
        "alias-peer",
        "sender",
    ] {
        assert!(
            callsign_command(&home, session, &["hello"])
                .status
                .success()
        );
    }
    for (name, target, expected) in [
        ("northstar", "northstar", "northstar"),
        ("prefix", "prefix", "prefix-peer"),
    ] {
        callsign_ledger(&home, &[("alias-peer", name)]);
        let recipient = format!("session:{target}");
        let output = callsign_command(
            &home,
            "sender",
            &["note", "Synthetic task", "--to", &recipient],
        );
        assert!(output.status.success(), "{output:?}");
        let store = harbard_core::Store::at(&home);
        let read = store.read(harbard_core::Window::All);
        let event = read
            .events
            .iter()
            .rev()
            .find(|event| event.event == harbard_core::EventKind::Note)
            .unwrap();
        assert_eq!(event.to.as_ref().unwrap().value, expected);
    }
    callsign_ledger(&home, &[("alias-peer", "ambiguous")]);
    let output = callsign_command(
        &home,
        "sender",
        &["note", "Synthetic task", "--to", "session:ambiguous"],
    );
    assert!(!output.status.success(), "{output:?}");
    assert!(String::from_utf8_lossy(&output.stderr).contains("matches 2 sessions"));
    assert_eq!(callsign_note_count(&home), 2);
}

#[test]
fn actionable_ids_are_full_and_ack_trace_never_guess() {
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    let reader = "11111111-1111-4111-8111-111111111111";
    let reader_collision = "11111111-1111-4111-8111-111111111112";
    let sender = "22222222-2222-4222-8222-222222222222";
    let third = "33333333-3333-4333-8333-333333333333";

    for session in [reader, reader_collision, sender, third] {
        let hello = run(
            &[
                "hello",
                "--home",
                hs,
                "--session",
                session,
                "--harness",
                "codex",
                "--cwd",
                hs,
            ],
            false,
        );
        assert!(hello.status.success());
    }

    let sent = run(
        &[
            "note",
            "needs an answer",
            "--to",
            &format!("session:{reader}"),
            "--kind",
            "warning",
            "--home",
            hs,
            "--session",
            sender,
            "--harness",
            "codex",
            "--cwd",
            hs,
            "--json",
        ],
        false,
    );
    assert!(sent.status.success(), "{sent:?}");
    let event: serde_json::Value = serde_json::from_slice(&sent.stdout).expect("note json");
    let note_id = event["id"].as_str().expect("note id");

    let brief = run(
        &[
            "brief",
            "--home",
            hs,
            "--session",
            reader,
            "--harness",
            "codex",
            "--cwd",
            hs,
        ],
        false,
    );
    let brief_text = String::from_utf8_lossy(&brief.stdout);
    assert!(
        brief_text.contains(note_id),
        "full ref missing: {brief_text}"
    );
    assert!(
        brief_text.contains(sender),
        "full sender id missing: {brief_text}"
    );

    let foreign_ack = run(
        &[
            "ack",
            note_id,
            "--home",
            hs,
            "--session",
            third,
            "--harness",
            "codex",
            "--cwd",
            hs,
        ],
        false,
    );
    assert!(!foreign_ack.status.success(), "foreign ack must be refused");

    let prefix_ack = run(
        &[
            "ack",
            &note_id[..12],
            "--home",
            hs,
            "--session",
            reader,
            "--harness",
            "codex",
            "--cwd",
            hs,
            "--json",
        ],
        false,
    );
    assert!(prefix_ack.status.success(), "{prefix_ack:?}");
    let ack: serde_json::Value = serde_json::from_slice(&prefix_ack.stdout).expect("ack json");
    assert_eq!(ack["acked"], note_id);

    let second = run(
        &[
            "note",
            "another",
            "--home",
            hs,
            "--session",
            sender,
            "--harness",
            "codex",
            "--cwd",
            hs,
            "--json",
        ],
        false,
    );
    let second_json: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    let second_id = second_json["id"].as_str().unwrap();
    let common = note_id
        .chars()
        .zip(second_id.chars())
        .take_while(|(a, b)| a == b)
        .count();
    let ambiguous_prefix = &note_id[..common.min(8)];
    let trace = run(&["trace", "--note", ambiguous_prefix, "--home", hs], false);
    assert!(!trace.status.success(), "trace must reject collisions");

    let ambiguous_session = run(&["trace", "--session", "111", "--home", hs], false);
    assert!(!ambiguous_session.status.success());
    let unknown_session = run(
        &["trace", "--session", "does-not-exist", "--home", hs],
        false,
    );
    assert!(!unknown_session.status.success());
}

#[test]
fn hello_prunes_dead_session_cursors_once_a_day_and_keeps_the_log() {
    let home = tmp_home();
    let hs = home.to_str().unwrap();
    let cursors = home.join("cursors");
    std::fs::create_dir_all(&cursors).unwrap();
    let month_ago = std::time::SystemTime::now() - std::time::Duration::from_secs(40 * 86_400);
    for name in ["dead", "dead.brief"] {
        let path = cursors.join(name);
        std::fs::write(&path, b"{}").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(month_ago)
            .unwrap();
    }
    std::fs::write(cursors.join("recent.brief"), b"{}").unwrap();

    let hello = |session: &str| {
        run(
            &[
                "hello",
                "--home",
                hs,
                "--session",
                session,
                "--harness",
                "codex",
                "--cwd",
                hs,
            ],
            false,
        )
    };
    assert!(hello("s1").status.success());
    assert!(!cursors.join("dead").exists());
    assert!(!cursors.join("dead.brief").exists());
    assert!(cursors.join("recent.brief").exists());
    assert!(home.join("prune.stamp").exists());

    // Within the interval the pass does not run again.
    std::fs::write(cursors.join("dead"), b"{}").unwrap();
    std::fs::File::options()
        .write(true)
        .open(cursors.join("dead"))
        .unwrap()
        .set_modified(month_ago)
        .unwrap();
    assert!(hello("s2").status.success());
    assert!(cursors.join("dead").exists());

    let ls = run(&["ls", "--json", "--all", "--home", hs], false);
    let sessions: serde_json::Value = serde_json::from_slice(&ls.stdout).expect("ls --json");
    assert_eq!(
        sessions.as_array().map(Vec::len),
        Some(2),
        "both hellos kept"
    );
}
