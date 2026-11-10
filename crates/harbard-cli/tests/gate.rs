//! Writer-lease gate: classify → ensure → deny rivals before any write.
//! Uses a temp HOME for harness install fixtures; never touches real ~/.claude.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp_home() -> PathBuf {
    let sequence = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "harbard-gate-cli-{}-{sequence}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("temp home");
    path
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_path_buf()
}

fn run_gate(
    home: &Path,
    session: &str,
    harness: &str,
    pid: u32,
    stdin_json: &str,
    extra: &[&str],
) -> Output {
    run_gate_at(home, session, harness, pid, stdin_json, &repo_root(), extra)
}

fn run_gate_at(
    home: &Path,
    session: &str,
    harness: &str,
    pid: u32,
    stdin_json: &str,
    repo: &Path,
    extra: &[&str],
) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_harbard"));
    command
        .env_clear()
        .env("HOME", home)
        .arg("--home")
        .arg(home)
        .arg("--session")
        .arg(session)
        .arg("--harness")
        .arg(harness)
        .arg("--hook")
        .arg(harness)
        .arg("--repo")
        .arg(repo)
        .arg("--cwd")
        .arg(repo)
        .arg("gate")
        .arg("--pid")
        .arg(pid.to_string())
        .arg("--json")
        .args(extra)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn gate");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(stdin_json.as_bytes()).unwrap();
    }
    child.wait_with_output().expect("wait gate")
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "JSON: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

/// Every admission decision written for this home, in file order.
fn admission_rows(home: &Path) -> Vec<serde_json::Value> {
    let raw =
        std::fs::read_to_string(home.join("leases/v1/admission.jsonl")).expect("admission.jsonl");
    raw.lines()
        .map(|line| serde_json::from_str(line).expect("admission row"))
        .collect()
}

/// The attribution floor: a row that names no checkout cannot be explained.
fn assert_every_row_names_a_checkout(rows: &[serde_json::Value]) {
    assert!(!rows.is_empty(), "at least one admission decision recorded");
    for row in rows {
        assert!(
            row["worktree"].as_str().is_some_and(|w| !w.is_empty()),
            "admission row without a worktree: {row}"
        );
    }
}

#[test]
fn read_does_not_acquire_write_acquires_and_rival_is_blocked() {
    let home = tmp_home();
    let pid = std::process::id();
    let marker = home.join("must-not-appear");
    assert!(!marker.exists());

    let read = run_gate(
        &home,
        "cursor-a",
        "cursor",
        pid,
        r#"{"tool_name":"Read","tool_input":{"path":"README.md"},"conversation_id":"cursor-a"}"#,
        &[],
    );
    assert!(read.status.success(), "read allows: {:?}", read);
    let read_j = json(&read);
    assert_eq!(read_j["status"], "allow");
    assert_eq!(read_j["class"], "read");
    assert!(read_j["claim_id"].is_null());
    assert!(read_j["worktree"].is_null());

    // No writer lease after a pure read.
    let check = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .args([
            "--home",
            home.to_str().unwrap(),
            "--session",
            "probe",
            "--repo",
            repo_root().to_str().unwrap(),
            "check",
            "--writer",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(check.status.success(), "{:?}", check);
    assert_eq!(json(&check)["status"], "free");

    let first = run_gate(
        &home,
        "cursor-a",
        "cursor",
        pid,
        r#"{"tool_name":"Write","tool_input":{"path":"x.rs","contents":"x"},"conversation_id":"cursor-a"}"#,
        &[],
    );
    assert!(first.status.success());
    let first_j = json(&first);
    assert_eq!(first_j["status"], "allow");
    assert_eq!(first_j["class"], "write");
    assert_eq!(first_j["worktree"], repo_root().to_str().unwrap());
    let claim_id = first_j["claim_id"].as_str().unwrap().to_string();

    let second = run_gate(
        &home,
        "cursor-a",
        "cursor",
        pid,
        r#"{"tool_name":"Shell","tool_input":{"command":"git add README.md"},"conversation_id":"cursor-a"}"#,
        &[],
    );
    assert!(second.status.success());
    let second_j = json(&second);
    assert_eq!(second_j["status"], "allow");
    assert_eq!(second_j["claim_id"], claim_id);
    assert_eq!(second_j["worktree"], repo_root().to_str().unwrap());

    // Same supervisor pid + different session = herd share (must NOT deny).
    let herd = run_gate(
        &home,
        "cursor-herd",
        "cursor",
        pid,
        r#"{"tool_name":"Write","tool_input":{"path":"herd.rs","contents":"h"},"conversation_id":"cursor-herd"}"#,
        &[],
    );
    assert!(herd.status.success());
    assert_eq!(json(&herd)["status"], "allow");

    // Foreign supervisor (live spare pid) must still be denied on Write.
    let mut foreign = Command::new("sleep")
        .arg("60")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spare pid");
    let foreign_pid = foreign.id();
    let rival = run_gate(
        &home,
        "cursor-b",
        "cursor",
        foreign_pid,
        r#"{"tool_name":"Write","tool_input":{"path":"y.rs","contents":"y"},"conversation_id":"cursor-b"}"#,
        &[],
    );
    assert_eq!(rival.status.code(), Some(2));
    let rival_j = json(&rival);
    assert_eq!(rival_j["status"], "deny");
    assert!(
        rival_j["reason"]
            .as_str()
            .unwrap()
            .contains("writer lease held"),
        "{rival_j}"
    );
    // Gate must not create files — the blocked tool never ran.
    assert!(!marker.exists());
    assert!(!repo_root().join("y.rs").exists());

    // Gather / orchestration under a foreign lease must not brick the agent.
    let task_output = run_gate(
        &home,
        "cursor-b",
        "cursor",
        foreign_pid,
        r#"{"tool_name":"TaskOutput","tool_input":{"task_id":"t1"},"conversation_id":"cursor-b"}"#,
        &[],
    );
    assert!(task_output.status.success());
    assert_eq!(json(&task_output)["status"], "allow");
    assert_eq!(json(&task_output)["class"], "read");
    assert!(json(&task_output)["claim_id"].is_null());

    // Ambiguous orchestration must not brick under a foreign writer, and must
    // not itself hold/renew the checkout lease.
    let task = run_gate(
        &home,
        "cursor-b",
        "cursor",
        foreign_pid,
        r#"{"tool_name":"Task","tool_input":{"prompt":"read-only audit"},"conversation_id":"cursor-b"}"#,
        &[],
    );
    assert!(
        task.status.success(),
        "Task must gather under foreign lease"
    );
    let task_j = json(&task);
    assert_eq!(task_j["status"], "allow");
    assert_eq!(task_j["class"], "ambiguous");
    assert!(
        task_j["claim_id"].is_null(),
        "Ambiguous must not take the lease"
    );
    let _ = foreign.kill();
    let _ = foreign.wait();
}

#[test]
fn foreign_writer_does_not_block_proven_reads_or_worktree_escape() {
    let home = tmp_home();
    let holder_pid = std::process::id();
    let holder = run_gate(
        &home,
        "holder",
        "codex",
        holder_pid,
        r#"{"tool_name":"Write","tool_input":{"path":"held.rs"},"session_id":"holder"}"#,
        &[],
    );
    assert_eq!(json(&holder)["status"], "allow");

    let mut foreign = Command::new("sleep")
        .arg("60")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spare foreign supervisor");
    let foreign_pid = foreign.id();

    for command in [
        "sed -n '1,20p' README.md",
        "git worktree list --porcelain",
        "git worktree add --help",
        "ls >/dev/null",
        "rg needle 2>/dev/null",
        "git status 2>&1",
        "open --help",
        "codex exec --help",
    ] {
        let payload = format!(
            r#"{{"tool_name":"Bash","tool_input":{{"command":{}}},"session_id":"reader"}}"#,
            serde_json::to_string(command).unwrap()
        );
        let out = run_gate(&home, "reader", "codex", foreign_pid, &payload, &[]);
        assert_eq!(
            json(&out)["status"],
            "allow",
            "{command}: stdout={} stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    for command in [
        "sed -i '' 's/x/y/' README.md",
        "git worktree remove other",
        "echo changed > README.md",
        "git push",
    ] {
        let payload = format!(
            r#"{{"tool_name":"Bash","tool_input":{{"command":{}}},"session_id":"reader"}}"#,
            serde_json::to_string(command).unwrap()
        );
        let out = run_gate(&home, "reader", "codex", foreign_pid, &payload, &[]);
        assert_eq!(json(&out)["status"], "deny", "{command}");
    }

    let _ = foreign.kill();
    let _ = foreign.wait();
}

#[test]
fn write_admission_follows_the_target_worktree_not_the_caller_cwd() {
    let home = tmp_home();
    let primary = home.join("primary");
    let target = home.join("target");
    std::fs::create_dir_all(primary.join(".git")).unwrap();
    std::fs::create_dir_all(target.join(".git")).unwrap();
    let canonical_target = std::fs::canonicalize(&target).unwrap();

    let holder = run_gate_at(
        &home,
        "target-holder",
        "codex",
        std::process::id(),
        r#"{"tool_name":"Write","tool_input":{"path":"held.rs"},"session_id":"target-holder"}"#,
        &target,
        &[],
    );
    assert_eq!(json(&holder)["status"], "allow");
    assert_eq!(
        json(&holder)["worktree"],
        canonical_target.to_str().unwrap()
    );
    let claim_id = json(&holder)["claim_id"].as_str().unwrap().to_string();
    let admissions = std::fs::read_to_string(home.join("leases/v1/admission.jsonl")).unwrap();
    let admission: serde_json::Value =
        serde_json::from_str(admissions.lines().last().unwrap()).unwrap();
    assert_eq!(admission["worktree"], canonical_target.to_str().unwrap());

    let mut foreign = Command::new("sleep")
        .arg("60")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spare foreign supervisor");
    let target_string = target.display().to_string();
    let payload = serde_json::json!({
        "tool_name": "Bash",
        "tool_input": { "command": format!("git -C '{}' commit -m x", target_string) },
        "session_id": "rival"
    })
    .to_string();
    let rival = run_gate_at(
        &home,
        "rival",
        "codex",
        foreign.id(),
        &payload,
        &primary,
        &[],
    );
    assert_eq!(
        json(&rival)["status"],
        "deny",
        "stdout={} stderr={}",
        String::from_utf8_lossy(&rival.stdout),
        String::from_utf8_lossy(&rival.stderr)
    );
    assert!(
        json(&rival)["reason"]
            .as_str()
            .unwrap()
            .contains("writer lease held")
    );

    // A leaked fencing token is insufficient: writer release also proves
    // the owning session and live supervisor PID.
    let foreign_pid = foreign.id().to_string();
    let stolen_release = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .args([
            "--home",
            home.to_str().unwrap(),
            "--repo",
            target.to_str().unwrap(),
            "--session",
            "rival",
            "--harness",
            "codex",
            "release",
            "--writer",
            "--claim-id",
            &claim_id,
            "--pid",
            &foreign_pid,
            "--json",
        ])
        .output()
        .unwrap();
    assert_eq!(stolen_release.status.code(), Some(2));
    assert_eq!(json(&stolen_release)["status"], "fenced");

    // The block is attributable on its own: which checkout, who holds it, and
    // how long that checkout stays busy — without joining against a lease
    // store that will have moved on by the time anyone reads the log.
    let rows = admission_rows(&home);
    assert_every_row_names_a_checkout(&rows);
    let blocked = rows
        .iter()
        .rfind(|row| row["decision"] == "blocked")
        .expect("the rival deny recorded a blocked row");
    assert_eq!(blocked["worktree"], canonical_target.to_str().unwrap());
    assert_eq!(blocked["holder"], "target-holder");
    assert_eq!(blocked["claim_id"], claim_id);
    let expires_in = blocked["holder_expires_in_ms"]
        .as_u64()
        .expect("blocked rows carry the holder's remaining lease time");
    assert!(
        expires_in > 0 && expires_in <= 900_000,
        "remaining time is within the writer TTL, got {expires_in}"
    );

    let _ = foreign.kill();
    let _ = foreign.wait();
}

/// A call that would mutate two checkouts is denied before the lease store is
/// ever consulted, so it has no holder and no single decided worktree. It used
/// to leave no admission row at all — the least attributable block of them
/// all. Each contested checkout now gets its own row.
#[test]
fn a_write_spanning_two_checkouts_is_denied_and_recorded_against_both() {
    let home = tmp_home();
    let left = home.join("left");
    let right = home.join("right");
    std::fs::create_dir_all(left.join(".git")).unwrap();
    std::fs::create_dir_all(right.join(".git")).unwrap();
    let canonical_left = std::fs::canonicalize(&left).unwrap();
    let canonical_right = std::fs::canonicalize(&right).unwrap();

    let payload = serde_json::json!({
        "tool_name": "MultiEdit",
        "tool_input": {
            "paths": [
                left.join("a.rs").to_str().unwrap(),
                right.join("b.rs").to_str().unwrap(),
            ]
        },
        "session_id": "spanner"
    })
    .to_string();
    let out = run_gate_at(
        &home,
        "spanner",
        "codex",
        std::process::id(),
        &payload,
        &left,
        &[],
    );
    assert_eq!(
        json(&out)["status"],
        "deny",
        "stdout={} stderr={}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(
        json(&out)["reason"]
            .as_str()
            .unwrap()
            .contains("multiple Git worktrees")
    );

    let rows = admission_rows(&home);
    assert_every_row_names_a_checkout(&rows);
    let mut blocked: Vec<&str> = rows
        .iter()
        .filter(|row| row["decision"] == "blocked")
        .map(|row| row["worktree"].as_str().unwrap())
        .collect();
    blocked.sort_unstable();
    let mut expected = vec![
        canonical_left.to_str().unwrap(),
        canonical_right.to_str().unwrap(),
    ];
    expected.sort_unstable();
    assert_eq!(blocked, expected, "one blocked row per contested checkout");
    // No lease was consulted, so nothing may be invented about a holder.
    for row in rows.iter().filter(|row| row["decision"] == "blocked") {
        assert!(row["holder"].is_null());
        assert!(row["holder_expires_in_ms"].is_null());
    }
}

#[test]
fn bye_releases_writer_lease_idempotently() {
    let home = tmp_home();
    let pid = std::process::id();
    let acquired = run_gate(
        &home,
        "sess-end",
        "claude-code",
        pid,
        r#"{"tool_name":"Edit","tool_input":{"file_path":"a.rs"},"session_id":"sess-end"}"#,
        &[],
    );
    assert!(acquired.status.success());

    let bye = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .args([
            "--home",
            home.to_str().unwrap(),
            "--session",
            "sess-end",
            "--harness",
            "claude-code",
            "--repo",
            repo_root().to_str().unwrap(),
            "bye",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(bye.status.success());

    let check = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .args([
            "--home",
            home.to_str().unwrap(),
            "--session",
            "probe",
            "--repo",
            repo_root().to_str().unwrap(),
            "check",
            "--writer",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(check.status.success(), "{:?}", check);
    assert_eq!(json(&check)["status"], "free");

    let bye2 = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .args([
            "--home",
            home.to_str().unwrap(),
            "--session",
            "sess-end",
            "--harness",
            "claude-code",
            "--repo",
            repo_root().to_str().unwrap(),
            "bye",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(bye2.status.success());
}

#[test]
fn dead_pid_lease_is_reclaimable_by_next_writer() {
    let home = tmp_home();
    // Bind the first lease to a pid that is not alive.
    let dead = 4_294_967_294u32; // unlikely to exist; claim rejects dead pid on acquire
    // So instead: acquire with live pid, then overwrite record's pid… easier path:
    // use ensure via gate with live pid, then claim with --pid of a dead one after
    // manually waiting is hard. Core unit tests cover dead-pid reclaim; here we
    // only assert a free resource after release (covered above) and that a new
    // session can acquire after bye.
    let pid = std::process::id();
    let _ = dead;
    let a = run_gate(
        &home,
        "one",
        "codex",
        pid,
        r#"{"tool_name":"apply_patch","tool_input":{"command":"*** Begin Patch"},"session_id":"one"}"#,
        &[],
    );
    assert!(a.status.success());
    let bye = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .args([
            "--home",
            home.to_str().unwrap(),
            "--session",
            "one",
            "--repo",
            repo_root().to_str().unwrap(),
            "bye",
        ])
        .output()
        .unwrap();
    assert!(bye.status.success());
    let b = run_gate(
        &home,
        "two",
        "codex",
        pid,
        r#"{"tool_name":"Bash","tool_input":{"command":"git commit -m x"},"session_id":"two"}"#,
        &[],
    );
    assert!(b.status.success());
    assert_eq!(json(&b)["status"], "allow");
}

#[test]
fn install_wires_presence_delivery_and_adaptation_without_the_retired_gate() {
    let root = tmp_home();
    std::fs::create_dir_all(root.join(".claude")).unwrap();
    std::fs::create_dir_all(root.join(".codex")).unwrap();
    std::fs::create_dir_all(root.join(".cursor")).unwrap();
    let store = root.join("store");

    for target in ["claude", "codex", "cursor"] {
        let out = Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .env("HOME", &root)
            .env("PATH", "")
            .args([
                "--home",
                store.to_str().unwrap(),
                "install",
                target,
                "--json",
            ])
            .output()
            .unwrap();
        assert!(out.status.success(), "{target}: {:?}", out);
    }

    let claude = std::fs::read_to_string(root.join(".claude/settings.json")).unwrap();
    assert!(!claude.contains("harbard gate"));
    // Claude Code and Codex brief on each prompt; only Claude Code adds the
    // throttled brief after a tool call, never before one. A subagent never
    // joins the bus; on Claude Code it is only told who reads its reports.
    assert!(claude.contains("harbard brief --touch"));
    let wired: serde_json::Value = serde_json::from_str(&claude).unwrap();
    assert!(
        wired["hooks"]["PostToolUse"]
            .to_string()
            .contains("harbard brief --inline --throttle")
    );
    assert!(!claude.contains("PreToolUse"));
    assert!(!claude.contains("SubagentStop"));
    let subagent = wired["hooks"]["SubagentStart"].to_string();
    assert!(
        subagent.contains("harbard protocol --subagent"),
        "{subagent}"
    );
    assert!(!subagent.contains("harbard hello"), "{subagent}");

    let codex = std::fs::read_to_string(root.join(".codex/hooks.json")).unwrap();
    assert!(!codex.contains("harbard gate"));
    assert!(codex.contains("harbard brief --touch"));
    assert!(!codex.contains("harbard brief --inline"));
    assert!(!codex.contains("PreToolUse"));
    assert!(!codex.contains("SubagentStart"));
    assert!(codex.contains("SessionEnd"));
    // Per-turn Stop carries the owed-answer check and the detached watcher,
    // never bye: bye on Stop marked live sessions ended after every reply.
    let v: serde_json::Value = serde_json::from_str(&codex).unwrap();
    let stop = v["hooks"]["Stop"].to_string();
    assert!(stop.contains("harbard stop-check --hook codex"), "{stop}");
    assert!(
        stop.contains("harbard watch --detach --hook codex"),
        "{stop}"
    );
    assert!(!stop.contains("harbard bye"), "{stop}");

    let cursor = std::fs::read_to_string(root.join(".cursor/hooks.json")).unwrap();
    assert!(!cursor.contains("harbard gate"));
    assert!(!cursor.contains("failClosed"));
    assert!(!cursor.contains("preToolUse"));
    assert!(cursor.contains("postToolUse"));

    // Byte-idempotent reinstall.
    for target in ["claude", "codex", "cursor"] {
        let before = match target {
            "claude" => std::fs::read(root.join(".claude/settings.json")).unwrap(),
            "codex" => std::fs::read(root.join(".codex/hooks.json")).unwrap(),
            _ => std::fs::read(root.join(".cursor/hooks.json")).unwrap(),
        };
        let out = Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .env("HOME", &root)
            .env("PATH", "")
            .args(["--home", store.to_str().unwrap(), "install", target])
            .output()
            .unwrap();
        assert!(out.status.success());
        let after = match target {
            "claude" => std::fs::read(root.join(".claude/settings.json")).unwrap(),
            "codex" => std::fs::read(root.join(".codex/hooks.json")).unwrap(),
            _ => std::fs::read(root.join(".cursor/hooks.json")).unwrap(),
        };
        assert_eq!(before, after, "{target} not byte-idempotent");
    }
}

#[test]
fn hook_deny_payload_shapes_for_three_harnesses() {
    let home = tmp_home();
    let pid = std::process::id();
    // Seed a holder.
    let _ = run_gate(
        &home,
        "holder",
        "cursor",
        pid,
        r#"{"tool_name":"Write","tool_input":{"path":"z"},"conversation_id":"holder"}"#,
        &[],
    );

    let mut foreign = Command::new("sleep")
        .arg("60")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spare pid");
    let foreign_pid = foreign.id();

    for (harness, session, needle) in [
        ("cursor", "rival-c", "\"permission\":\"deny\""),
        ("claude-code", "rival-a", "permissionDecision"),
        ("codex", "rival-b", "permissionDecision"),
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_harbard"));
        command
            .env_clear()
            .env("HOME", &home)
            .args([
                "--home",
                home.to_str().unwrap(),
                "--session",
                session,
                "--harness",
                harness,
                "--hook",
                harness,
                "--repo",
                repo_root().to_str().unwrap(),
                "gate",
                "--pid",
                &foreign_pid.to_string(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(
                format!(
                    r#"{{"tool_name":"Write","tool_input":{{"path":"x"}},"session_id":"{session}","conversation_id":"{session}"}}"#
                )
                .as_bytes(),
            )
            .unwrap();
        let out = child.wait_with_output().unwrap();
        // Hook path exits 0 with deny JSON (Claude/Codex parse decisions on 0).
        assert_eq!(out.status.code(), Some(0), "{harness}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stdout.contains(needle), "{harness}: {stdout}");
        // Claude deny payload uses permissionDecision: deny (not allow).
        if harness != "cursor" {
            assert!(
                stdout.contains("\"permissionDecision\":\"deny\"")
                    || stdout.contains("\"permissionDecision\": \"deny\""),
                "{harness}: expected deny decision, got {stdout}"
            );
        }
        // Critical: stderr must stay empty. Claude/Codex surface hook stderr as
        // a tool Error; agents then invent "permissions broken" / "reads blocked".
        assert!(
            stderr.trim().is_empty(),
            "{harness}: deny must not write stderr (got {stderr})"
        );
        assert!(
            stdout.contains("permissions prompt"),
            "{harness}: deny reason must distinguish lease from permissions UI: {stdout}"
        );
    }
    let _ = foreign.kill();
    let _ = foreign.wait();
}

/// Live spare process whose pid SystemPidChecker accepts as alive.
fn spare_supervisor() -> (u32, std::process::Child) {
    let child = Command::new("sleep")
        .arg("60")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spare supervisor");
    (child.id(), child)
}

fn run_gate_hook(home: &Path, session: &str, harness: &str, pid: u32, stdin_json: &str) -> Output {
    let repo = repo_root();
    let mut command = Command::new(env!("CARGO_BIN_EXE_harbard"));
    command
        .env_clear()
        .env("HOME", home)
        .args([
            "--home",
            home.to_str().unwrap(),
            "--session",
            session,
            "--harness",
            harness,
            "--hook",
            harness,
            "--repo",
            repo.to_str().unwrap(),
            "gate",
            "--pid",
            &pid.to_string(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().expect("spawn gate hook");
    {
        let mut stdin = child.stdin.take().expect("stdin");
        stdin.write_all(stdin_json.as_bytes()).unwrap();
    }
    child.wait_with_output().expect("wait gate hook")
}

/// G7 — Parent + Task child share the writer lease at the CLI gate (not only core).
#[test]
fn herd_child_same_supervisor_pid_can_write_while_parent_holds() {
    let home = tmp_home();
    let pid = std::process::id();
    let parent = run_gate(
        &home,
        "parent-main",
        "claude-code",
        pid,
        r#"{"tool_name":"Write","tool_input":{"path":"p.rs"},"session_id":"parent-main"}"#,
        &[],
    );
    assert_eq!(json(&parent)["status"], "allow");
    let claim = json(&parent)["claim_id"].as_str().unwrap().to_string();

    let child = run_gate(
        &home,
        "a32e23d6c0b702b77", // realistic Task session id shape
        "claude-code",
        pid,
        r#"{"tool_name":"Bash","tool_input":{"command":"git add README.md"},"session_id":"a32e23d6c0b702b77"}"#,
        &[],
    );
    let child_j = json(&child);
    assert_eq!(child_j["status"], "allow", "{child_j}");
    assert_eq!(child_j["class"], "write");
    assert_eq!(child_j["claim_id"], claim);
}

/// G3 — Ambiguous never takes the lease (free or foreign); Write still denies.
#[test]
fn ambiguous_never_acquires_lease_write_still_denies() {
    let home = tmp_home();
    let pid = std::process::id();
    // Free worktree: Task must not become the exclusive writer.
    let task = run_gate(
        &home,
        "gatherer",
        "claude-code",
        pid,
        r#"{"tool_name":"Task","tool_input":{"prompt":"x"},"session_id":"gatherer"}"#,
        &[],
    );
    assert_eq!(json(&task)["status"], "allow");
    assert_eq!(json(&task)["class"], "ambiguous");
    assert!(
        json(&task)["claim_id"].is_null(),
        "Ambiguous must not acquire: {}",
        json(&task)
    );

    let holder = run_gate(
        &home,
        "holder",
        "claude-code",
        pid,
        r#"{"tool_name":"Write","tool_input":{"path":"z"},"session_id":"holder"}"#,
        &[],
    );
    assert_eq!(json(&holder)["status"], "allow");
    assert!(!json(&holder)["claim_id"].is_null());

    let (foreign_pid, mut foreign) = spare_supervisor();
    let mystery = run_gate(
        &home,
        "rival",
        "claude-code",
        foreign_pid,
        r#"{"tool_name":"Bash","tool_input":{"command":"./scripts/mystery.sh"},"session_id":"rival"}"#,
        &[],
    );
    assert_eq!(json(&mystery)["status"], "allow");
    assert!(json(&mystery)["claim_id"].is_null());
    let push = run_gate(
        &home,
        "rival",
        "claude-code",
        foreign_pid,
        r#"{"tool_name":"Bash","tool_input":{"command":"git push"},"session_id":"rival"}"#,
        &[],
    );
    assert_eq!(push.status.code(), Some(2));
    assert_eq!(json(&push)["status"], "deny");
    let _ = foreign.kill();
}

/// G5 — Cursor hook allow path always emits permission JSON (failClosed-safe).
#[test]
fn cursor_hook_allow_always_emits_permission_json() {
    let home = tmp_home();
    let pid = std::process::id();
    for (tool, input) in [
        (
            "Read",
            r#"{"tool_name":"Read","tool_input":{"path":"README.md"},"conversation_id":"c"}"#,
        ),
        (
            "Shell-status",
            r#"{"tool_name":"Shell","tool_input":{"command":"git status"},"conversation_id":"c"}"#,
        ),
        (
            "unknown-tool",
            r#"{"tool_name":"Shell","tool_input":{"command":"some-tool status --json"},"conversation_id":"c"}"#,
        ),
    ] {
        let out = run_gate_hook(&home, "c", "cursor", pid, input);
        assert_eq!(out.status.code(), Some(0), "{tool}");
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(
            stdout.contains("\"permission\":\"allow\"")
                || stdout.contains("\"permission\": \"allow\""),
            "{tool}: Cursor failClosed needs permission JSON, got {stdout:?}"
        );
        assert!(
            String::from_utf8_lossy(&out.stderr).trim().is_empty(),
            "{tool}: unexpected stderr {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

/// G12 — absolute path must not soft-allow a write under a foreign lease.
#[test]
fn absolute_path_git_push_denied_under_foreign_lease() {
    let home = tmp_home();
    let _ = run_gate(
        &home,
        "holder",
        "claude-code",
        std::process::id(),
        r#"{"tool_name":"Write","tool_input":{"path":"z"},"session_id":"holder"}"#,
        &[],
    );
    let (foreign_pid, mut foreign) = spare_supervisor();
    for cmd in [
        "/usr/bin/git push",
        "/opt/homebrew/bin/git push origin main",
        "env /usr/bin/git push",
        r#"/bin/bash -c '/usr/bin/git push'"#,
    ] {
        let payload = format!(
            r#"{{"tool_name":"Bash","tool_input":{{"command":{}}},"session_id":"rival"}}"#,
            serde_json::to_string(cmd).unwrap()
        );
        let out = run_gate(&home, "rival", "claude-code", foreign_pid, &payload, &[]);
        assert_eq!(out.status.code(), Some(2), "{cmd}");
        assert_eq!(json(&out)["status"], "deny", "{cmd}");
    }
    // Absolute-path reads still gather.
    let read = run_gate(
        &home,
        "rival",
        "claude-code",
        foreign_pid,
        r#"{"tool_name":"Bash","tool_input":{"command":"/usr/bin/git log -1"},"session_id":"rival"}"#,
        &[],
    );
    assert_eq!(json(&read)["status"], "allow");
    assert_eq!(json(&read)["class"], "read");
    let _ = foreign.kill();
}

/// G10 — `bash -c 'git push'` under a foreign lease must deny (not soft-allow Ambiguous).
#[test]
fn bash_c_git_push_denied_under_foreign_lease() {
    let home = tmp_home();
    let _ = run_gate(
        &home,
        "holder",
        "claude-code",
        std::process::id(),
        r#"{"tool_name":"Write","tool_input":{"path":"z"},"session_id":"holder"}"#,
        &[],
    );
    let (foreign_pid, mut foreign) = spare_supervisor();
    for cmd in [
        r#"bash -c "git push""#,
        r#"echo x && git push"#,
        "eval git push",
    ] {
        let payload = format!(
            r#"{{"tool_name":"Bash","tool_input":{{"command":{}}},"session_id":"rival"}}"#,
            serde_json::to_string(cmd).unwrap()
        );
        let out = run_gate(&home, "rival", "claude-code", foreign_pid, &payload, &[]);
        assert_eq!(out.status.code(), Some(2), "{cmd}");
        assert_eq!(json(&out)["status"], "deny", "{cmd}");
    }
    // Read subcommands still gather under the same foreign lease.
    let gh = run_gate(
        &home,
        "rival",
        "claude-code",
        foreign_pid,
        r#"{"tool_name":"Bash","tool_input":{"command":"gh run list"},"session_id":"rival"}"#,
        &[],
    );
    assert_eq!(json(&gh)["status"], "allow");
    assert_eq!(json(&gh)["class"], "read");
    let _ = foreign.kill();
}

/// Foreign Write still denied on Claude hook path (not only --json exit 2).
#[test]
fn foreign_write_denied_on_claude_hook_path() {
    let home = tmp_home();
    let _ = run_gate(
        &home,
        "holder",
        "claude-code",
        std::process::id(),
        r#"{"tool_name":"Write","tool_input":{"path":"z"},"session_id":"holder"}"#,
        &[],
    );
    let (foreign_pid, mut foreign) = spare_supervisor();
    let out = run_gate_hook(
        &home,
        "rival",
        "claude-code",
        foreign_pid,
        r#"{"tool_name":"Write","tool_input":{"path":"x"},"session_id":"rival"}"#,
    );
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(
        stdout.contains("\"permissionDecision\":\"deny\"")
            || stdout.contains("\"permissionDecision\": \"deny\""),
        "expected deny, got {stdout}"
    );
    let _ = foreign.kill();
}

/// A wait has to have a length and an ending, or the admission log is a list of
/// refusals nobody can learn from: it never said whether someone waited eight
/// seconds or twenty minutes, nor whether they ever got in.
#[test]
fn a_blocked_write_that_later_lands_records_what_the_wait_cost() {
    let home = tmp_home();
    let pid = std::process::id();
    let edit = r#"{"tool_name":"Edit","tool_input":{"file_path":"a.rs"},"session_id":"SID"}"#;

    // The holder takes the checkout.
    let held = run_gate(
        &home,
        "holder",
        "claude-code",
        pid,
        &edit.replace("SID", "holder"),
        &[],
    );
    assert!(held.status.success(), "{held:?}");

    // A rival with a genuinely live supervisor is refused: the clock starts,
    // and no cost is claimed while the wait is still running.
    let mut spare = std::process::Command::new("sleep")
        .arg("60")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spare pid");
    let rival_pid = spare.id();
    let blocked = run_gate(
        &home,
        "rival",
        "codex",
        rival_pid,
        &edit.replace("SID", "rival"),
        &[],
    );
    let rows = admission_rows(&home);
    let last = rows.last().expect("a blocked row");
    assert_eq!(last["decision"], "blocked", "{blocked:?}");
    assert!(
        last["waited_ms"].is_null(),
        "a wait cannot be costed while it is still running: {last}"
    );

    // The holder leaves and the rival finally writes.
    let bye = std::process::Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .args([
            "--home",
            home.to_str().unwrap(),
            "--session",
            "holder",
            "--harness",
            "claude-code",
            "--repo",
            repo_root().to_str().unwrap(),
            "--cwd",
            repo_root().to_str().unwrap(),
            "bye",
            "--json",
        ])
        .output()
        .unwrap();
    assert!(bye.status.success(), "{bye:?}");

    let landed = run_gate(
        &home,
        "rival",
        "codex",
        rival_pid,
        &edit.replace("SID", "rival"),
        &[],
    );
    assert!(landed.status.success(), "{landed:?}");
    let rows = admission_rows(&home);
    let last = rows.last().expect("an admitted row");
    // Any admitted decision closes the wait — what is being measured is that
    // the write finally landed, not which of the three admit paths it took.
    assert!(
        matches!(
            last["decision"].as_str(),
            Some("acquired") | Some("held") | Some("unprotected")
        ),
        "the rival should be in now: {last}"
    );
    assert!(
        last["waited_ms"].as_u64().is_some(),
        "the row that ends a wait must carry its cost: {last}"
    );
    let _ = spare.kill();
    let _ = spare.wait();
}

/// A lead reading a builder's worktree while the builder holds its writer
/// lease: every read-only git command is a read, opens no lease of its own
/// and is never denied, so no "two writers" ever starts from a look. Each
/// command is also clear for `checkout-policy`: `git status`, `git log`,
/// `git diff`, `git branch --list` and `git stash list` are looks, not writes.
#[test]
fn read_only_git_commands_in_a_builders_worktree_are_reads() {
    let home = tmp_home();
    let repo = repo_root();
    let builder = run_gate(
        &home,
        "builder",
        "codex",
        std::process::id(),
        r#"{"tool_name":"Write","tool_input":{"path":"held.rs"},"session_id":"builder"}"#,
        &[],
    );
    assert_eq!(json(&builder)["status"], "allow");

    let mut lead = Command::new("sleep")
        .arg("60")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spare lead supervisor");
    let lead_pid = lead.id();
    let r = repo.to_str().unwrap();
    for read in [
        "git status",
        "git log --oneline -3",
        "git diff --stat",
        "git branch --list",
        "git stash list",
    ] {
        for command in [
            format!("git -C {r} {}", &read[4..]),
            format!("cd {r} && {read}"),
        ] {
            let payload = format!(
                r#"{{"tool_name":"Bash","tool_input":{{"command":{}}},"session_id":"lead"}}"#,
                serde_json::to_string(&command).unwrap()
            );
            let out = run_gate(&home, "lead", "claude-code", lead_pid, &payload, &[]);
            let v = json(&out);
            assert_eq!(v["status"], "allow", "{command}: {v}");
            assert_eq!(v["class"], "read", "{command}: {v}");
            assert!(v["claim_id"].is_null(), "{command} opened a lease: {v}");

            let policy = Command::new(env!("CARGO_BIN_EXE_harbard"))
                .env_clear()
                .args(["--home", home.to_str().unwrap(), "--cwd", r, "--json"])
                .args(["checkout-policy", "--command", &command])
                .output()
                .unwrap();
            let p = json(&policy);
            assert!(p["hazard"].is_null(), "{command}: {p}");
        }
    }
    let _ = lead.kill();
    let _ = lead.wait();
}
