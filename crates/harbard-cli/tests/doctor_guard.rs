//! `doctor` says whether a guard for Claude Code's rm safety prompt is wired.
//! A temp `HOME` with its own `~/.claude`; the real one is never read.
//!
//! Claude Code asks before an `rm` it cannot verify, even in
//! bypassPermissions, and an unattended agent waits on that prompt.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn home() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("harbard-doctor-guard-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join(".claude/hooks")).unwrap();
    d
}

fn doctor(home: &Path) -> serde_json::Value {
    let out = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .env("HOME", home)
        .env("PATH", "/usr/bin:/bin")
        .args(["doctor", "--json", "--home"])
        .arg(home.join(".harbard"))
        .output()
        .unwrap();
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {out:?}"))
}

fn settings(home: &Path, command: &str) {
    let v = serde_json::json!({ "hooks": { "PreToolUse": [{
        "matcher": "Bash",
        "hooks": [{ "type": "command", "command": command }],
    }]}});
    std::fs::write(home.join(".claude/settings.json"), v.to_string()).unwrap();
}

#[test]
fn doctor_reports_a_wired_rm_guard() {
    let h = home();
    std::fs::write(
        h.join(".claude/hooks/rm-guard.py"),
        "#!/usr/bin/env python3\n",
    )
    .unwrap();
    settings(&h, "python3 ~/.claude/hooks/rm-guard.py");
    let v = doctor(&h);
    assert_eq!(v["rm_guard"]["wired"], true, "{v}");
    assert_eq!(v["rm_guard"]["status"], "wired", "{v}");
    let _ = std::fs::remove_dir_all(&h);
}

#[test]
fn doctor_explains_a_missing_rm_guard() {
    let h = home();
    settings(&h, "harbard gate --hook claude-code");
    let v = doctor(&h);
    assert_eq!(v["rm_guard"]["wired"], false, "{v}");
    assert_eq!(v["rm_guard"]["status"], "not_wired", "{v}");
    let why = v["rm_guard"]["why"].as_str().unwrap();
    assert!(why.contains("rm"), "{why}");
    assert!(why.contains("prompt"), "{why}");

    // Wired, but the script is not there: it guards nothing.
    settings(&h, "python3 ~/.claude/hooks/rm-guard.py");
    let v = doctor(&h);
    assert_eq!(v["rm_guard"]["status"], "script_missing", "{v}");
    assert_eq!(v["rm_guard"]["wired"], false, "{v}");
    let _ = std::fs::remove_dir_all(&h);
}

#[test]
fn doctor_says_the_guard_does_not_apply_without_claude_code() {
    let h = home();
    let _ = std::fs::remove_dir_all(h.join(".claude"));
    let v = doctor(&h);
    assert_eq!(v["rm_guard"]["status"], "no_claude_code", "{v}");
    let _ = std::fs::remove_dir_all(&h);
}
