//! End-to-end tests for the read-only Git hygiene audit. Every scenario uses
//! temporary repositories and drives the real `harbard` binary.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(target_os = "linux")]
use std::os::unix::ffi::{OsStrExt, OsStringExt};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

static SEQ: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(label: &str) -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("harbard-audit-{label}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("create temp directory");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git(root: &Path, args: &[&str]) -> Output {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("spawn git");
    assert!(
        output.status.success(),
        "git -C {} {} failed: {}",
        root.display(),
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn init_repo(root: &Path) {
    git(
        root.parent().expect("repo parent"),
        &["init", "-b", "main", root.to_str().unwrap()],
    );
    git(root, &["config", "user.name", "Harbard Test"]);
    git(root, &["config", "user.email", "harbard@example.invalid"]);
    std::fs::write(root.join("tracked.txt"), "base\n").expect("write tracked file");
    git(root, &["add", "tracked.txt"]);
    git(root, &["commit", "-m", "base"]);
}

fn harbard(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_harbard"))
        .args(args)
        .output()
        .expect("spawn harbard")
}

fn harbard_command() -> Command {
    Command::new(env!("CARGO_BIN_EXE_harbard"))
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON ({error}): stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn worktree_by_branch<'a>(report: &'a Value, branch: &str) -> &'a Value {
    report["worktrees"]
        .as_array()
        .expect("worktrees array")
        .iter()
        .find(|worktree| worktree["branch"] == branch)
        .unwrap_or_else(|| panic!("missing worktree for {branch}: {report}"))
}

fn canonical_text(path: &Path) -> String {
    path.canonicalize()
        .expect("canonical path")
        .to_string_lossy()
        .into_owned()
}

#[test]
fn audits_primary_and_linked_worktrees_with_divergence_and_dirty_state() {
    let temp = TempDir::new("siblings");
    let primary = temp.0.join("repo");
    let behind = temp.0.join("behind");
    let diverged = temp.0.join("diverged");
    init_repo(&primary);

    git(&primary, &["branch", "behind"]);
    git(
        &primary,
        &["worktree", "add", behind.to_str().unwrap(), "behind"],
    );
    git(&primary, &["branch", "diverged"]);
    git(
        &primary,
        &["worktree", "add", diverged.to_str().unwrap(), "diverged"],
    );

    std::fs::write(diverged.join("diverged.txt"), "unique\n").expect("write diverged file");
    git(&diverged, &["add", "diverged.txt"]);
    git(&diverged, &["commit", "-m", "diverge"]);
    std::fs::write(primary.join("tracked.txt"), "base\nmain\n").expect("advance main");
    git(&primary, &["add", "tracked.txt"]);
    git(&primary, &["commit", "-m", "advance main"]);
    std::fs::write(behind.join("untracked.txt"), "dirty\n").expect("dirty linked worktree");

    let output = harbard(&[
        "audit",
        "--repo",
        primary.to_str().unwrap(),
        "--target",
        "main",
        "--include-worktrees",
        "--json",
    ]);
    assert!(output.status.success(), "audit should succeed");
    let report = json(&output);
    assert_eq!(report["worktrees"].as_array().unwrap().len(), 3);

    let main = worktree_by_branch(&report, "main");
    assert_eq!(main["primary"], true);
    assert_eq!(main["ahead"], 0);
    assert_eq!(main["behind"], 0);

    let behind_report = worktree_by_branch(&report, "behind");
    assert_eq!(behind_report["primary"], false);
    assert_eq!(behind_report["dirty"], true);
    assert_eq!(behind_report["ahead"], 0);
    assert_eq!(behind_report["behind"], 1);

    let diverged_report = worktree_by_branch(&report, "diverged");
    assert_eq!(diverged_report["primary"], false);
    assert_eq!(diverged_report["ahead"], 1);
    assert_eq!(diverged_report["behind"], 1);
    assert_ne!(behind_report["root"], diverged_report["root"]);
}

#[test]
fn detects_configured_upstream_after_remote_branch_is_gone() {
    let temp = TempDir::new("gone-upstream");
    let primary = temp.0.join("repo");
    let linked = temp.0.join("obsolete");
    let remote = temp.0.join("remote.git");
    init_repo(&primary);
    git(&temp.0, &["init", "--bare", remote.to_str().unwrap()]);
    git(
        &primary,
        &["remote", "add", "origin", remote.to_str().unwrap()],
    );
    git(&primary, &["push", "-u", "origin", "main"]);
    git(&primary, &["branch", "obsolete"]);
    git(
        &primary,
        &["worktree", "add", linked.to_str().unwrap(), "obsolete"],
    );
    git(&linked, &["push", "-u", "origin", "obsolete"]);
    git(&remote, &["update-ref", "-d", "refs/heads/obsolete"]);
    git(
        &primary,
        &["update-ref", "-d", "refs/remotes/origin/obsolete"],
    );

    let output = harbard(&["audit", "--repo", linked.to_str().unwrap(), "--json"]);
    assert!(output.status.success(), "gone upstream is nonfatal");
    let report = json(&output);
    let worktree = &report["worktrees"][0];
    assert_eq!(worktree["branch"], "obsolete");
    assert_eq!(worktree["upstream"], "origin/obsolete");
    assert_eq!(worktree["upstreamGone"], true);
    assert_eq!(worktree["target"], "origin/obsolete");
    assert!(worktree["ahead"].is_null());
    assert!(worktree["behind"].is_null());
    assert!(!worktree["errors"].as_array().unwrap().is_empty());
}

#[test]
fn falls_back_to_origin_dev_and_reports_missing_explicit_target_nonfatally() {
    let temp = TempDir::new("targets");
    let primary = temp.0.join("repo");
    init_repo(&primary);
    let head = String::from_utf8(git(&primary, &["rev-parse", "HEAD"]).stdout).expect("UTF-8 SHA");
    git(
        &primary,
        &["update-ref", "refs/remotes/origin/dev", head.trim()],
    );

    let fallback = harbard(&["audit", "--repo", primary.to_str().unwrap(), "--json"]);
    assert!(fallback.status.success());
    let fallback_report = json(&fallback);
    assert_eq!(fallback_report["worktrees"][0]["target"], "origin/dev");
    assert_eq!(fallback_report["worktrees"][0]["ahead"], 0);
    assert_eq!(fallback_report["worktrees"][0]["behind"], 0);

    let missing = harbard(&[
        "audit",
        "--repo",
        primary.to_str().unwrap(),
        "--target",
        "missing/ref",
        "--json",
    ]);
    assert!(
        missing.status.success(),
        "worktree read errors are nonfatal"
    );
    let missing_report = json(&missing);
    let worktree = &missing_report["worktrees"][0];
    assert_eq!(worktree["target"], "missing/ref");
    assert!(worktree["targetHead"].is_null());
    assert!(!worktree["errors"].as_array().unwrap().is_empty());
}

#[test]
fn outside_git_returns_structured_error_and_nonzero_status() {
    let temp = TempDir::new("outside");
    let output = harbard(&[
        "audit",
        "--repo",
        temp.0.to_str().unwrap(),
        "--include-worktrees",
        "--json",
    ]);
    assert!(!output.status.success());
    let report = json(&output);
    assert!(report["commonDir"].is_null());
    assert!(report["worktrees"].as_array().unwrap().is_empty());
    assert!(!report["errors"].as_array().unwrap().is_empty());
}

#[test]
fn ignores_inherited_git_repository_and_config_environment() {
    let temp = TempDir::new("hostile-env");
    let requested = temp.0.join("requested");
    let decoy = temp.0.join("decoy");
    init_repo(&requested);
    init_repo(&decoy);
    let decoy_git_dir = String::from_utf8(git(&decoy, &["rev-parse", "--absolute-git-dir"]).stdout)
        .expect("UTF-8 Git directory");

    let marker = temp.0.join("config-marker");
    let mut command = harbard_command();
    let output = command
        .args(["audit", "--repo", requested.to_str().unwrap(), "--json"])
        .env("GIT_DIR", decoy_git_dir.trim())
        .env("GIT_WORK_TREE", &decoy)
        .env("GIT_COMMON_DIR", decoy.join(".git"))
        .env("GIT_INDEX_FILE", decoy.join(".git/index"))
        .env("GIT_OBJECT_DIRECTORY", decoy.join(".git/objects"))
        .env(
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            decoy.join(".git/objects"),
        )
        .env("GIT_PREFIX", "wrong/")
        .env("GIT_CONFIG_PARAMETERS", "malformed")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.fsmonitor")
        .env("GIT_CONFIG_VALUE_0", marker.as_os_str())
        .output()
        .expect("spawn harbard with hostile Git environment");

    assert!(output.status.success());
    let report = json(&output);
    assert_eq!(report["worktrees"][0]["root"], canonical_text(&requested));
    assert!(!marker.exists(), "injected config program must never run");
}

#[test]
fn resolves_upstream_through_custom_fetch_refspec() {
    let temp = TempDir::new("custom-refspec");
    let primary = temp.0.join("repo");
    let remote = temp.0.join("remote.git");
    init_repo(&primary);
    git(&temp.0, &["init", "--bare", remote.to_str().unwrap()]);
    git(
        &primary,
        &["remote", "add", "mirror", remote.to_str().unwrap()],
    );
    git(
        &primary,
        &[
            "config",
            "remote.mirror.fetch",
            "+refs/heads/*:refs/remotes/mirror/custom/*",
        ],
    );
    git(&primary, &["push", "mirror", "main"]);
    git(&primary, &["fetch", "mirror"]);
    git(&primary, &["config", "branch.main.remote", "mirror"]);
    git(
        &primary,
        &["config", "branch.main.merge", "refs/heads/main"],
    );

    let output = harbard(&["audit", "--repo", primary.to_str().unwrap(), "--json"]);
    assert!(output.status.success());
    let report = json(&output);
    let worktree = &report["worktrees"][0];
    assert_eq!(worktree["upstream"], "mirror/custom/main");
    assert_eq!(worktree["upstreamGone"], false);
    assert_eq!(worktree["target"], "mirror/custom/main");
}

#[test]
fn preserves_trailing_whitespace_in_repository_path() {
    let temp = TempDir::new("trailing-space");
    let primary = temp.0.join("repo with trailing space ");
    init_repo(&primary);

    let output = harbard(&["audit", "--repo", primary.to_str().unwrap(), "--json"]);
    assert!(output.status.success());
    let report = json(&output);
    assert_eq!(report["worktrees"][0]["root"], canonical_text(&primary));
}

#[test]
fn dirty_detection_covers_staged_modified_deleted_and_untracked_files() {
    for state in ["staged", "modified", "deleted", "untracked"] {
        let temp = TempDir::new(state);
        let primary = temp.0.join("repo");
        init_repo(&primary);
        match state {
            "staged" => {
                std::fs::write(primary.join("staged.txt"), "staged\n").unwrap();
                git(&primary, &["add", "staged.txt"]);
            }
            "modified" => {
                std::fs::write(primary.join("tracked.txt"), "modified\n").unwrap();
            }
            "deleted" => {
                std::fs::remove_file(primary.join("tracked.txt")).unwrap();
            }
            "untracked" => {
                std::fs::write(primary.join("untracked.txt"), "untracked\n").unwrap();
            }
            _ => unreachable!(),
        }

        let output = harbard(&["audit", "--repo", primary.to_str().unwrap(), "--json"]);
        assert!(output.status.success(), "audit {state}");
        let report = json(&output);
        assert_eq!(
            report["worktrees"][0]["dirty"], true,
            "{state} change must be dirty"
        );
    }
}

#[cfg(unix)]
#[test]
fn audit_is_index_immutable_and_never_executes_content_programs() {
    let temp = TempDir::new("read-only-programs");
    let primary = temp.0.join("repo");
    init_repo(&primary);
    std::fs::write(
        primary.join(".gitattributes"),
        "*.txt filter=sentinel diff=sentinel\n",
    )
    .expect("write attributes");
    std::os::unix::fs::symlink("tracked.txt", primary.join("tracked-link"))
        .expect("create tracked symlink");
    git(&primary, &["add", ".gitattributes", "tracked-link"]);
    git(&primary, &["commit", "-m", "attributes"]);

    let marker = temp.0.join("program-ran");
    let program = temp.0.join("sentinel.sh");
    std::fs::write(
        &program,
        format!("#!/bin/sh\nprintf ran >> '{}'\ncat\n", marker.display()),
    )
    .expect("write sentinel program");
    let mut permissions = std::fs::metadata(&program).unwrap().permissions();
    permissions.set_mode(0o755);
    std::fs::set_permissions(&program, permissions).expect("make sentinel executable");
    git(
        &primary,
        &["config", "core.fsmonitor", program.to_str().unwrap()],
    );
    git(
        &primary,
        &["config", "filter.sentinel.clean", program.to_str().unwrap()],
    );
    git(
        &primary,
        &[
            "config",
            "filter.sentinel.smudge",
            program.to_str().unwrap(),
        ],
    );
    git(
        &primary,
        &[
            "config",
            "filter.sentinel.process",
            program.to_str().unwrap(),
        ],
    );
    git(&primary, &["config", "filter.sentinel.required", "true"]);
    git(
        &primary,
        &["config", "diff.sentinel.command", program.to_str().unwrap()],
    );
    git(
        &primary,
        &[
            "config",
            "diff.sentinel.textconv",
            program.to_str().unwrap(),
        ],
    );
    git(
        &primary,
        &["config", "diff.external", program.to_str().unwrap()],
    );

    // Rewriting identical bytes after the index timestamp invalidates cached
    // stat data and forces a
    // content comparison without making the checkout semantically dirty.
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    std::fs::write(primary.join("tracked.txt"), "base\n").expect("touch tracked content");
    std::fs::remove_file(primary.join("tracked-link")).expect("replace tracked symlink");
    std::os::unix::fs::symlink("tracked.txt", primary.join("tracked-link"))
        .expect("recreate identical tracked symlink");
    let index_path = String::from_utf8(git(&primary, &["rev-parse", "--git-path", "index"]).stdout)
        .expect("UTF-8 index path");
    let index_path = PathBuf::from(index_path.trim_end());
    let index_path = if index_path.is_absolute() {
        index_path
    } else {
        primary.join(index_path)
    };
    let before = std::fs::read(&index_path).expect("read index before audit");
    assert!(
        !marker.exists(),
        "test setup must not execute sentinel programs"
    );

    let output = harbard(&["audit", "--repo", primary.to_str().unwrap(), "--json"]);
    assert!(output.status.success());
    let report = json(&output);
    assert_eq!(
        report["worktrees"][0]["dirty"],
        false,
        "unexpected dirty report: {report}; marker={}",
        std::fs::read_to_string(&marker).unwrap_or_default()
    );
    let after = std::fs::read(&index_path).expect("read index after audit");
    assert_eq!(before, after, "audit must not refresh or rewrite the index");
    assert!(
        !marker.exists(),
        "fsmonitor/filter/diff programs must not run"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn accepts_non_utf8_worktree_paths_without_retargeting_git() {
    let temp = TempDir::new("non-utf8");
    let primary = temp.0.join("repo");
    init_repo(&primary);
    git(&primary, &["branch", "binary-path"]);
    let mut bytes = temp.0.as_os_str().as_bytes().to_vec();
    bytes.extend_from_slice(b"/linked-");
    bytes.push(0xff);
    let linked = PathBuf::from(std::ffi::OsString::from_vec(bytes));
    let output = Command::new("git")
        .arg("-C")
        .arg(&primary)
        .args(["worktree", "add"])
        .arg(&linked)
        .arg("binary-path")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("add non-UTF-8 worktree");
    assert!(
        output.status.success(),
        "add non-UTF-8 worktree: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let output = harbard_command()
        .args(["audit", "--repo"])
        .arg(&linked)
        .args(["--include-worktrees", "--json"])
        .output()
        .expect("audit non-UTF-8 worktree");
    assert!(output.status.success());
    let report = json(&output);
    assert_eq!(report["worktrees"].as_array().unwrap().len(), 2);
    assert!(
        report["worktrees"]
            .as_array()
            .unwrap()
            .iter()
            .any(|worktree| worktree["branch"] == "binary-path")
    );
    let binary_worktree = worktree_by_branch(&report, "binary-path");
    assert!(binary_worktree["root"].as_str().unwrap().contains("\\xFF"));
    assert!(
        !binary_worktree["root"]
            .as_str()
            .unwrap()
            .contains('\u{fffd}')
    );
}

#[test]
fn submodule_reports_checkout_root_as_primary_not_internal_git_storage() {
    let temp = TempDir::new("submodule");
    let parent = temp.0.join("parent");
    let source = temp.0.join("source");
    init_repo(&parent);
    init_repo(&source);
    git(
        &parent,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            source.to_str().unwrap(),
            "modules/child",
        ],
    );
    git(&parent, &["commit", "-m", "submodule"]);
    let child = parent.join("modules/child");

    let output = harbard(&[
        "audit",
        "--repo",
        child.to_str().unwrap(),
        "--include-worktrees",
        "--json",
    ]);
    assert!(output.status.success());
    let report = json(&output);
    assert_eq!(report["worktrees"].as_array().unwrap().len(), 1);
    let worktree = &report["worktrees"][0];
    assert_eq!(worktree["root"], canonical_text(&child));
    assert_eq!(worktree["primary"], true);
    assert!(
        !worktree["root"]
            .as_str()
            .unwrap()
            .contains("/.git/modules/")
    );
}

#[test]
fn audit_defaults_to_cwd_git_root() {
    let temp = TempDir::new("cwd-default");
    let primary = temp.0.join("repo");
    init_repo(&primary);

    let output = harbard_command()
        .args(["audit", "--json"])
        .current_dir(&primary)
        .output()
        .expect("spawn harbard");
    assert!(
        output.status.success(),
        "stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report = json(&output);
    assert_eq!(report["worktrees"][0]["root"], canonical_text(&primary));
}
