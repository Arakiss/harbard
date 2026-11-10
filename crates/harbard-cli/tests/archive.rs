//! `harbard archive` end to end: the real binary, a cleared environment, and
//! a scratch HOME holding fake Claude Code and Codex transcript trees.
//! Nothing reads or writes the operator's own home.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

struct Lab(PathBuf);

impl Lab {
    fn new() -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("harbard-archive-it-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("user")).unwrap();
        std::fs::create_dir_all(d.join("harbard")).unwrap();
        Lab(d)
    }
    fn user(&self) -> PathBuf {
        self.0.join("user")
    }
    fn archive(&self) -> PathBuf {
        self.0.join("archive")
    }
    fn write(&self, rel: &str, bytes: &[u8]) {
        let p = self.user().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, bytes).unwrap();
    }
    fn run(&self, args: &[&str]) -> Output {
        self.run_with(&[], args)
    }
    fn run_with(&self, env: &[(&str, &Path)], args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .env("HOME", self.user())
            .envs(env.iter().copied())
            .env("HARBARD_API", "http://127.0.0.1:9")
            .arg("--home")
            .arg(self.0.join("harbard"))
            .args(args)
            .output()
            .expect("spawn harbard")
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn read(p: &Path) -> String {
    std::fs::read_to_string(p).unwrap()
}

#[test]
fn enable_run_status_and_disable_end_to_end() {
    let lab = Lab::new();
    lab.write(".claude/projects/-w/s1.jsonl", b"lead\n");
    lab.write(".codex/sessions/2026/11/10/rollout-a.jsonl", b"codex\n");
    lab.write(".codex/archived_sessions/rollout-b.jsonl", b"old\n");

    // Off: run says what to do next.
    let off = lab.run(&["archive", "run"]);
    assert_eq!(off.status.code(), Some(1));
    assert!(
        err(&off).contains("harbard archive enable --dir"),
        "{}",
        err(&off)
    );

    let dir = lab.archive();
    let on = lab.run(&["archive", "enable", "--dir", dir.to_str().unwrap()]);
    assert_eq!(on.status.code(), Some(0), "{}", err(&on));
    assert!(dir.is_dir());
    let config: serde_json::Value =
        serde_json::from_str(&read(&lab.0.join("harbard/archive.json"))).unwrap();
    assert_eq!(config["dir"], dir.to_str().unwrap());

    let ran = lab.run(&["archive", "run"]);
    assert_eq!(ran.status.code(), Some(0), "{}", err(&ran));
    assert!(
        out(&ran).contains("3 file(s) seen: 3 copied"),
        "{}",
        out(&ran)
    );
    assert_eq!(read(&dir.join("claude/projects/-w/s1.jsonl")), "lead\n");
    assert_eq!(
        read(&dir.join("codex/sessions/2026/11/10/rollout-a.jsonl")),
        "codex\n"
    );
    assert_eq!(
        read(&dir.join("codex/archived_sessions/rollout-b.jsonl")),
        "old\n"
    );

    let status = lab.run(&["archive", "status", "--json"]);
    assert_eq!(status.status.code(), Some(0), "{}", err(&status));
    let v: serde_json::Value = serde_json::from_str(&out(&status)).unwrap();
    assert_eq!(v["enabled"], true);
    assert_eq!(v["running"], false);
    assert_eq!(v["state"]["copied"], 3);
    assert_eq!(v["state"]["error_count"], 0);

    let doctor = lab.run(&["doctor"]);
    assert!(
        out(&doctor)
            .lines()
            .any(|l| l.starts_with("archive ") && l.contains("on → ")),
        "{}",
        out(&doctor)
    );

    let disabled = lab.run(&["archive", "disable"]);
    assert_eq!(disabled.status.code(), Some(0));
    assert!(out(&disabled).contains("stay where they are"));
    assert!(dir.join("claude/projects/-w/s1.jsonl").is_file());
    let status = lab.run(&["archive", "status"]);
    assert!(
        out(&status).contains("off · harbard archive enable"),
        "{}",
        out(&status)
    );
}

#[test]
fn enable_refuses_a_folder_inside_the_transcripts() {
    let lab = Lab::new();
    lab.write(".claude/projects/-w/s1.jsonl", b"x\n");
    let inside = lab.user().join(".claude/projects/backup");
    let refused = lab.run(&["archive", "enable", "--dir", inside.to_str().unwrap()]);
    assert_eq!(refused.status.code(), Some(1));
    assert!(err(&refused).contains("inside"), "{}", err(&refused));
    assert!(!inside.exists());
    assert!(!lab.0.join("harbard/archive.json").exists());
}

/// Codex keeps its transcripts under `$CODEX_HOME` when it is set; the
/// archive reads them there and refuses a folder inside them, as Codex and
/// `harbard install` resolve the same directory.
#[test]
fn the_archive_reads_codex_transcripts_under_codex_home() {
    let lab = Lab::new();
    let codex = lab.0.join("codex-home");
    lab.write(
        ".codex/sessions/2026/11/10/rollout-stale.jsonl",
        b"not here\n",
    );
    let rel = "sessions/2026/11/10/rollout-a.jsonl";
    std::fs::create_dir_all(codex.join(rel).parent().unwrap()).unwrap();
    std::fs::write(codex.join(rel), b"codex\n").unwrap();
    let env = [("CODEX_HOME", codex.as_path())];

    let inside = codex.join("sessions/backup");
    let refused = lab.run_with(
        &env,
        &["archive", "enable", "--dir", inside.to_str().unwrap()],
    );
    assert_eq!(refused.status.code(), Some(1), "{}", err(&refused));
    assert!(err(&refused).contains("inside"), "{}", err(&refused));

    let dir = lab.archive();
    let on = lab.run_with(&env, &["archive", "enable", "--dir", dir.to_str().unwrap()]);
    assert_eq!(on.status.code(), Some(0), "{}", err(&on));
    let ran = lab.run_with(&env, &["archive", "run"]);
    assert_eq!(ran.status.code(), Some(0), "{}", err(&ran));
    assert_eq!(read(&dir.join("codex").join(rel)), "codex\n");
    assert!(
        !dir.join("codex/sessions/2026/11/10/rollout-stale.jsonl")
            .exists()
    );
}
