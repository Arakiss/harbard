//! Updates end to end (docs/SPEC-UPDATES.md): a real `origin` with newer
//! commits, a source checkout behind it, the real binary and a fresh
//! `--home`. The check counts and lists what is new; the run fast-forwards
//! the checkout, runs its install script and skips what is not installed;
//! a dirty checkout is refused before anything moves.
//!
//! The LaunchAgent labels point at jobs that do not exist, so no real
//! service is ever restarted, and the checkout's install script is a stub.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static SEQ: AtomicU64 = AtomicU64::new(0);

struct Lab {
    root: PathBuf,
    home: PathBuf,
    origin: PathBuf,
    source: PathBuf,
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_AUTHOR_NAME", "t")
        .env("GIT_AUTHOR_EMAIL", "t@t")
        .env("GIT_COMMITTER_NAME", "t")
        .env("GIT_COMMITTER_EMAIL", "t@t")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn commit(dir: &Path, file: &str, text: &str, subject: &str) {
    std::fs::write(dir.join(file), text).unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", subject]);
}

const CHANGELOG_OLD: &str = "# Changelog\n\n## Unreleased\n\n- an old line.\n";
const CHANGELOG_NEW: &str = "# Changelog\n\n## Unreleased\n\n### Updates\n\n- harbard says when it is behind.\n\n- an old line.\n";

impl Lab {
    fn new() -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("harbard-up-it-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let root = std::fs::canonicalize(&root).unwrap();
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        // The upstream, as a working repository the test commits into.
        let upstream = root.join("upstream");
        std::fs::create_dir_all(upstream.join("scripts")).unwrap();
        git(&upstream, &["init", "-q", "-b", "main"]);
        std::fs::write(
            upstream.join("Cargo.toml"),
            "[workspace.package]\nversion = \"0.3.0-alpha.9\"\n",
        )
        .unwrap();
        std::fs::write(
            upstream.join("scripts/install-cli.sh"),
            "#!/usr/bin/env bash\necho \"stub install from $(git rev-parse --short HEAD)\"\necho ran > \"$(dirname \"$0\")/../installed.marker\"\n",
        )
        .unwrap();
        std::fs::write(upstream.join(".gitignore"), "installed.marker\n").unwrap();
        commit(&upstream, "CHANGELOG.md", CHANGELOG_OLD, "chore: start");
        let origin = root.join("origin.git");
        Command::new("git")
            .args(["clone", "-q", "--bare"])
            .arg(&upstream)
            .arg(&origin)
            .status()
            .unwrap();
        let source = root.join("source");
        Command::new("git")
            .args(["clone", "-q"])
            .arg(&origin)
            .arg(&source)
            .status()
            .unwrap();
        // Two new commits upstream, pushed to origin: the source is behind.
        commit(
            &upstream,
            "CHANGELOG.md",
            CHANGELOG_NEW,
            "feat(console): an update banner",
        );
        commit(&upstream, "a.txt", "a", "fix(cli): a typo");
        git(&upstream, &["push", "-q", origin.to_str().unwrap(), "main"]);
        git(&upstream, &["tag", "v0.3.0-alpha.10"]);
        git(
            &upstream,
            &["push", "-q", origin.to_str().unwrap(), "v0.3.0-alpha.10"],
        );
        Lab {
            root,
            home,
            origin,
            source,
        }
    }

    fn harbard(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_harbard"))
            .args(args)
            .arg("--home")
            .arg(&self.home)
            .env("HARBARD_UPDATE_SOURCE", &self.source)
            .env(
                "HARBARD_UPDATE_SERVE_LABEL",
                "com.harbard.test.nothing-serve",
            )
            .env(
                "HARBARD_UPDATE_CONSOLE_LABEL",
                "com.harbard.test.nothing-console",
            )
            .env_remove("HARBARD_HOME")
            .output()
            .unwrap()
    }

    fn view(&self) -> Value {
        let out = self.harbard(&["update", "--check", "--json"]);
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)))
    }
}

#[test]
fn a_checkout_behind_origin_is_told_what_changed_and_updated() {
    let lab = Lab::new();
    let behind = lab.harbard(&["update", "--check", "--quiet"]);
    assert_eq!(behind.status.code(), Some(10), "available exits 10");

    let v = lab.view();
    let check = &v["check"];
    assert_eq!(check["status"], "available");
    assert_eq!(check["behind"], 2);
    assert_eq!(check["channel"], "main");
    let kinds: Vec<&str> = check["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["fix", "feat"], "newest first");
    assert_eq!(check["changes"][1]["scope"], "console");
    assert_eq!(
        check["notes"], "### Updates\n\n- harbard says when it is behind.",
        "only what the installed changelog did not say"
    );
    assert_eq!(v["source"]["problem"], Value::Null);
    assert_eq!(
        v["source"]["path"].as_str().unwrap(),
        lab.source.to_str().unwrap()
    );

    // Install: the run is detached; `update --yes` follows it to the end.
    let out = lab.harbard(&["update", "--yes"]);
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(
        out.status.success(),
        "{text}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(text.contains("harbard is up to date at"), "{text}");
    assert_eq!(
        git(&lab.source, &["rev-parse", "HEAD"]),
        git(&lab.origin, &["rev-parse", "main"]),
        "the checkout was fast-forwarded"
    );
    assert!(
        lab.source.join("installed.marker").is_file(),
        "the install script ran"
    );

    let v: Value = {
        let out = lab.harbard(&["update", "--json", "--check"]);
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let run = &v["run"];
    assert_eq!(run["state"], "done");
    let steps: Vec<(String, String)> = run["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| {
            (
                s["id"].as_str().unwrap().to_string(),
                s["state"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let steps: Vec<(&str, &str)> = steps
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    assert_eq!(
        steps,
        [
            ("fetch", "done"),
            ("cli", "done"),
            ("hooks", "done"),
            ("serve", "skipped"),
            ("console", "skipped"),
            ("desktop", "skipped"),
        ]
    );
    assert_eq!(run["steps"][3]["detail"], "not_installed");
    assert_eq!(
        run["changes"].as_array().unwrap().len(),
        2,
        "the run keeps what it brought"
    );
    assert_eq!(
        run["notes"],
        "### Updates\n\n- harbard says when it is behind."
    );
    assert!(
        run["log_tail"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap().starts_with("stub install from")),
        "the log tail carries the script's output: {}",
        run["log_tail"]
    );
}

#[test]
fn a_dirty_checkout_is_refused_before_anything_moves() {
    let lab = Lab::new();
    let head = git(&lab.source, &["rev-parse", "HEAD"]);
    std::fs::write(lab.source.join("CHANGELOG.md"), "edited by hand\n").unwrap();
    assert_eq!(lab.view()["source"]["problem"], "dirty");
    let out = lab.harbard(&["update", "--yes"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("(dirty)"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        git(&lab.source, &["rev-parse", "HEAD"]),
        head,
        "nothing moved"
    );
    assert!(!lab.source.join("installed.marker").exists());
}

#[test]
fn the_releases_channel_follows_tags_only() {
    let lab = Lab::new();
    let out = lab.harbard(&["update", "--check", "--channel", "releases", "--json"]);
    let v: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["prefs"]["channel"], "releases");
    assert_eq!(v["check"]["status"], "available");
    assert_eq!(v["check"]["target"]["tag"], "v0.3.0-alpha.10");
    assert_eq!(v["check"]["target"]["version"], "0.3.0-alpha.10");
    // The choice is remembered.
    let prefs: Value =
        serde_json::from_str(&std::fs::read_to_string(lab.home.join("update.json")).unwrap())
            .unwrap();
    assert_eq!(prefs["channel"], "releases");
}

#[test]
fn a_detached_run_survives_its_caller_and_reports_through_the_api_body() {
    let lab = Lab::new();
    let out = lab.harbard(&["update", "--yes", "--detach"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let v = lab.view();
        if v["run"]["state"] == "done" {
            break;
        }
        assert_ne!(v["run"]["state"], "failed", "{}", v["run"]);
        assert!(
            Instant::now() < deadline,
            "the run never finished: {}",
            v["run"]
        );
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(lab.source.join("installed.marker").is_file());
}
