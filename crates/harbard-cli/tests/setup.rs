//! Professional setup contract: detect the three supported harnesses, wire
//! only those present, preserve foreign hooks, and stay byte-idempotent.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_root() -> PathBuf {
    let sequence = SEQ.fetch_add(1, Ordering::Relaxed);
    let root = std::env::temp_dir().join(format!(
        "harbard-setup-cli-{}-{sequence}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).expect("create temp root");
    root
}

fn run(root: &Path, store: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_harbard"));
    command
        .env_clear()
        .env("HOME", root)
        .env("PATH", "")
        .arg("--home")
        .arg(store)
        .args(args);
    command.output().expect("spawn harbard")
}

fn seed_present_harnesses(root: &Path) {
    std::fs::create_dir_all(root.join(".claude")).unwrap();
    std::fs::create_dir_all(root.join(".codex")).unwrap();
    std::fs::create_dir_all(root.join(".cursor")).unwrap();
}

fn backup_count(path: &Path) -> usize {
    let parent = path.parent().unwrap();
    let prefix = format!("{}.bak-", path.file_name().unwrap().to_string_lossy());
    std::fs::read_dir(parent)
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with(&prefix))
        .count()
}

#[test]
fn setup_dry_run_detects_all_three_without_writing() {
    let root = temp_root();
    let store = root.join("store");
    seed_present_harnesses(&root);

    let output = run(&root, &store, &["setup", "--dry-run", "--json"]);
    assert!(output.status.success());
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["dry_run"], true);
    assert_eq!(report["ok"], true);
    assert_eq!(report["summary"]["status"], "3 of 3");
    assert_eq!(report["summary"]["present"], 3);
    assert_eq!(report["summary"]["wired_ok"], 3);
    assert_eq!(report["summary"]["absent"], 0);
    assert!(report["absent"].as_array().unwrap().is_empty());
    let harnesses = report["harnesses"].as_array().unwrap();
    assert_eq!(harnesses.len(), 3);
    for target in ["claude", "codex", "cursor"] {
        let item = harnesses
            .iter()
            .find(|item| item["target"] == target)
            .unwrap();
        assert_eq!(item["present"], true);
        assert_eq!(item["action"], "would_install");
        assert_eq!(item["ok"], true);
        assert!(item["hooks"].is_object());
    }
    assert!(!root.join(".claude/settings.json").exists());
    assert!(!root.join(".codex/hooks.json").exists());
    assert!(!root.join(".cursor/hooks.json").exists());
    assert!(!store.join("setup.json").exists());
}

#[test]
fn setup_is_byte_idempotent_and_preserves_foreign_hooks() {
    let root = temp_root();
    let store = root.join("store");
    seed_present_harnesses(&root);
    let claude = root.join(".claude/settings.json");
    std::fs::write(
        &claude,
        r#"{
  "model": "operator-choice",
  "hooks": {
    "SessionStart": [
      {
        "hooks": [
          {
            "type": "command",
            "command": "foreign-tool start",
            "timeout": 3
          }
        ]
      }
    ]
  }
}
"#,
    )
    .unwrap();

    let first = run(&root, &store, &["setup", "--json"]);
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    let first_report: serde_json::Value = serde_json::from_slice(&first.stdout).unwrap();
    assert_eq!(first_report["ok"], true);
    assert_eq!(first_report["summary"]["status"], "3 of 3");
    assert_eq!(
        first_report["wired"].as_array().unwrap().len(),
        3,
        "only present harnesses are wired"
    );
    assert!(
        first_report["harnesses"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["action"] == "installed" && item["ok"] == true)
    );

    let paths = [
        claude.clone(),
        root.join(".codex/hooks.json"),
        root.join(".cursor/hooks.json"),
    ];
    let first_bytes: Vec<String> = paths
        .iter()
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect();
    assert!(first_bytes[0].contains("foreign-tool start"));
    assert!(first_bytes[0].contains("\"model\": \"operator-choice\""));
    let first_backups: Vec<usize> = paths.iter().map(|path| backup_count(path)).collect();

    let second = run(&root, &store, &["setup", "--json"]);
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    let second_report: serde_json::Value = serde_json::from_slice(&second.stdout).unwrap();
    assert_eq!(second_report["summary"]["status"], "3 of 3");
    assert!(
        second_report["harnesses"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["action"] == "unchanged" && item["ok"] == true)
    );
    for (index, path) in paths.iter().enumerate() {
        assert_eq!(std::fs::read_to_string(path).unwrap(), first_bytes[index]);
        assert_eq!(
            backup_count(path),
            first_backups[index],
            "unchanged setup must not create another backup for {}",
            path.display()
        );
    }
    assert!(store.join("setup.json").exists());

    let dry_run = run(&root, &store, &["setup", "--dry-run", "--json"]);
    assert!(dry_run.status.success());
    let dry_run_report: serde_json::Value = serde_json::from_slice(&dry_run.stdout).unwrap();
    assert_eq!(dry_run_report["summary"]["status"], "3 of 3");
    assert!(
        dry_run_report["harnesses"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["action"] == "unchanged")
    );
}

#[test]
fn setup_skips_absent_harnesses_without_marking_installed() {
    let root = temp_root();
    let store = root.join("store");
    // Only Claude is present — Codex/Cursor must stay absent, never "wired".
    std::fs::create_dir_all(root.join(".claude")).unwrap();

    let output = run(&root, &store, &["setup", "--json"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["ok"], true);
    assert_eq!(report["summary"]["present"], 1);
    assert_eq!(report["summary"]["wired_ok"], 1);
    assert_eq!(report["summary"]["absent"], 2);
    assert_eq!(report["summary"]["status"], "1 of 1");
    assert_eq!(report["wired"], serde_json::json!(["claude"]));
    assert_eq!(report["absent"], serde_json::json!(["codex", "cursor"]));

    let harnesses = report["harnesses"].as_array().unwrap();
    let claude = harnesses.iter().find(|h| h["target"] == "claude").unwrap();
    assert_eq!(claude["present"], true);
    assert_eq!(claude["action"], "installed");
    assert_eq!(claude["ok"], true);

    for target in ["codex", "cursor"] {
        let item = harnesses.iter().find(|h| h["target"] == target).unwrap();
        assert_eq!(item["present"], false);
        assert_eq!(item["action"], "absent");
        assert_eq!(item["ok"], false);
        assert!(
            !report["wired"]
                .as_array()
                .unwrap()
                .iter()
                .any(|v| v == target),
            "{target} must not appear in wired"
        );
    }
    assert!(root.join(".claude/settings.json").exists());
    assert!(!root.join(".codex/hooks.json").exists());
    assert!(!root.join(".cursor/hooks.json").exists());
}

/// `harbard setup --harness claude-code` limits setup to that harness: it does
/// not wire Codex or Cursor.
#[test]
fn setup_harness_flag_limits_setup_to_that_harness() {
    let root = temp_root();
    let store = root.join("store");
    seed_present_harnesses(&root);

    let output = run(
        &root,
        &store,
        &["setup", "--harness", "claude-code", "--json"],
    );
    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let harnesses = report["harnesses"].as_array().unwrap();
    assert_eq!(harnesses.len(), 1, "{report}");
    assert_eq!(harnesses[0]["target"], "claude");
    assert_eq!(report["summary"]["status"], "1 of 1");
    assert!(root.join(".claude/settings.json").exists());
    assert!(!root.join(".codex/hooks.json").exists(), "codex untouched");
    assert!(
        !root.join(".cursor/hooks.json").exists(),
        "cursor untouched"
    );

    for (flag, target) in [
        ("codex", "codex"),
        ("cursor", "cursor"),
        ("claude", "claude"),
    ] {
        let out = run(
            &root,
            &store,
            &["setup", "--harness", flag, "--dry-run", "--json"],
        );
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        assert_eq!(v["harnesses"].as_array().unwrap().len(), 1, "{flag}: {v}");
        assert_eq!(v["harnesses"][0]["target"], target, "{flag}");
    }
}

#[test]
fn setup_refuses_a_harness_it_cannot_wire_and_writes_nothing() {
    let root = temp_root();
    let store = root.join("store");
    seed_present_harnesses(&root);
    let output = run(&root, &store, &["setup", "--harness", "other"]);
    assert!(!output.status.success(), "{output:?}");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("claude-code, codex or cursor"), "{stderr}");
    assert!(!root.join(".claude/settings.json").exists());
    assert!(!store.join("setup.json").exists());
}

/// A dry run names every file the real run would write or copy.
#[test]
fn setup_dry_run_lists_every_file_it_would_touch() {
    let root = temp_root();
    let store = root.join("store");
    seed_present_harnesses(&root);
    // An existing config is backed up before it is rewritten.
    std::fs::write(root.join(".claude/settings.json"), "{}\n").unwrap();

    let output = run(
        &root,
        &store,
        &["setup", "--dry-run", "--harness", "claude-code", "--json"],
    );
    assert!(output.status.success(), "{output:?}");
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let files = report["files"].as_array().expect("files");
    let has = |path: &Path, action: &str| {
        files
            .iter()
            .any(|f| f["path"].as_str() == path.to_str() && f["action"] == action)
    };
    let settings = root.join(".claude/settings.json");
    assert!(has(&settings, "write"), "{report}");
    assert!(
        files.iter().any(|f| f["action"] == "backup"
            && f["path"]
                .as_str()
                .unwrap()
                .starts_with(&format!("{}.bak-", settings.display()))),
        "{report}"
    );
    assert!(has(&store.join("setup.json"), "write"), "{report}");
    assert_eq!(files.len(), 3, "nothing for codex or cursor: {report}");
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        "{}\n",
        "dry run"
    );

    let human = run(
        &root,
        &store,
        &["setup", "--dry-run", "--harness", "claude-code"],
    );
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(
        text.contains(&format!("would write {}", settings.display())),
        "{text}"
    );
    assert!(text.contains("would back up"), "{text}");
    assert!(
        text.contains(&format!(
            "would write {}",
            store.join("setup.json").display()
        )),
        "{text}"
    );

    // A full dry run lists all three configs, and nothing is ever written.
    let all = run(&root, &store, &["setup", "--dry-run", "--json"]);
    let v: serde_json::Value = serde_json::from_slice(&all.stdout).unwrap();
    let writes = v["files"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["action"] == "write")
        .count();
    assert_eq!(writes, 4, "three configs and setup.json: {v}");
    assert!(!store.join("setup.json").exists());
}
