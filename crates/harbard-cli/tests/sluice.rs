//! The sluice, end to end against a real repository.
//!
//! These tests exist for one sentence: the commit that lands is the commit the
//! checks ran on. Everything else here is scaffolding to prove that under a
//! failing check, a conflict, and a branch that moves mid-flight.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn tmp_home() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "harbard-sluice-{}-{}",
        std::process::id(),
        ulid::Ulid::generate()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn git(repo: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// A repository with one commit on `main`.
fn repo_with_history() -> PathBuf {
    let dir = tmp_home().join("repo");
    std::fs::create_dir_all(&dir).unwrap();
    git(&dir, &["init", "--initial-branch=main"]);
    git(&dir, &["config", "user.email", "t@example.com"]);
    git(&dir, &["config", "user.name", "test"]);
    std::fs::write(dir.join("README.md"), "base\n").unwrap();
    git(&dir, &["add", "README.md"]);
    git(&dir, &["commit", "-m", "base"]);
    dir
}

/// Commit a file on a detached side branch and return its SHA.
fn side_commit(repo: &Path, file: &str, body: &str, message: &str) -> String {
    let head = git(repo, &["rev-parse", "HEAD"]);
    git(repo, &["checkout", "--quiet", "-b", message, &head]);
    std::fs::write(repo.join(file), body).unwrap();
    git(repo, &["add", file]);
    git(repo, &["commit", "-m", message]);
    let sha = git(repo, &["rev-parse", "HEAD"]);
    git(repo, &["checkout", "--quiet", "main"]);
    sha
}

fn run(home: &Path, args: &[&str]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_harbard"));
    cmd.env_clear()
        .env("HOME", home)
        .args(["--home", home.to_str().unwrap()])
        .args(["--session", "tester", "--harness", "codex"])
        .args(args);
    cmd.output().expect("spawn harbard")
}

fn json(out: &Output) -> serde_json::Value {
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| {
        panic!(
            "not json: {e}\nstdout={}\nstderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

#[test]
fn sibling_delivery_is_visible_and_integrates_from_either_checkout() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "shared.txt", "shared\n", "feature");
    let sibling = repo.parent().unwrap().join("sibling");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--detach",
            sibling.to_str().unwrap(),
            &feature,
        ],
    );
    let queued = json(&run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--checks",
            "test -f shared.txt",
            "--repo",
            sibling.to_str().unwrap(),
            "--json",
        ],
    ));
    assert_eq!(
        queued["delivery"]["repo"],
        std::fs::canonicalize(&repo).unwrap().to_str().unwrap()
    );
    let listed = json(&run(
        &home,
        &["deliveries", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(listed["pending"].as_array().unwrap().len(), 1, "{listed}");
    let drained = json(&run(
        &home,
        &["integrate", "--repo", sibling.to_str().unwrap(), "--json"],
    ));
    assert_eq!(drained["landed"], 1, "{drained}");
    assert_eq!(
        std::fs::read_to_string(repo.join("shared.txt")).unwrap(),
        "shared\n"
    );
    assert!(git(&repo, &["status", "--porcelain"]).is_empty());
    assert!(git(&repo, &["worktree", "list", "--porcelain"]).contains(sibling.to_str().unwrap()));
}

#[test]
fn a_target_branch_checked_out_in_a_sibling_keeps_its_files_in_sync() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "README.md", "delivered\n", "feature");
    git(&repo, &["checkout", "-b", "operator"]);
    let sibling = repo.parent().unwrap().join("target");
    git(
        &repo,
        &["worktree", "add", sibling.to_str().unwrap(), "main"],
    );
    run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--checks",
            "true",
            "--repo",
            repo.to_str().unwrap(),
        ],
    );
    let drained = json(&run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(drained["landed"], 1, "{drained}");
    assert_eq!(
        std::fs::read_to_string(sibling.join("README.md")).unwrap(),
        "delivered\n"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("README.md")).unwrap(),
        "base\n"
    );
    assert!(git(&sibling, &["status", "--porcelain"]).is_empty());
}

#[test]
fn a_landing_never_overwrites_a_colliding_untracked_file() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "local.txt", "delivery\n", "feature");
    let before = git(&repo, &["rev-parse", "main"]);
    std::fs::write(repo.join("local.txt"), "operator work\n").unwrap();
    run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--checks",
            "true",
            "--repo",
            repo.to_str().unwrap(),
        ],
    );
    let drained = json(&run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(drained["landed"], 0, "{drained}");
    assert_eq!(git(&repo, &["rev-parse", "main"]), before);
    assert_eq!(
        std::fs::read_to_string(repo.join("local.txt")).unwrap(),
        "operator work\n"
    );
}

#[test]
fn legacy_sibling_queues_remain_visible_without_rewriting_the_log() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "old.txt", "legacy\n", "feature");
    let sibling = repo.parent().unwrap().join("legacy");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "--detach",
            sibling.to_str().unwrap(),
            &feature,
        ],
    );
    let delivery = harbard_core::sluice::Delivery {
        id: ulid::Ulid::generate().to_string(),
        repo: std::fs::canonicalize(&sibling)
            .unwrap()
            .to_str()
            .unwrap()
            .into(),
        sha: feature,
        branch: "main".into(),
        base: git(&repo, &["rev-parse", "main"]),
        checks: "true".into(),
        prepare: String::new(),
        session: "old-writer".into(),
        harness: "codex".into(),
        created_ms: 1,
    };
    harbard_core::sluice::enqueue(&home, &delivery).unwrap();
    let queue = harbard_core::sluice::queue_path(&home, &delivery.repo);
    let before = std::fs::read(&queue).unwrap();
    let listed = json(&run(
        &home,
        &["deliveries", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(listed["pending"].as_array().unwrap().len(), 1, "{listed}");
    let drained = json(&run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(drained["landed"], 1, "{drained}");
    let listed = json(&run(
        &home,
        &["deliveries", "--repo", sibling.to_str().unwrap(), "--json"],
    ));
    assert!(listed["pending"].as_array().unwrap().is_empty(), "{listed}");
    assert_eq!(
        std::fs::read(&queue).unwrap(),
        before,
        "legacy JSONL is append-only"
    );
}

#[test]
fn ignored_local_files_are_also_preserved_on_a_collision() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "local.txt", "delivery\n", "feature");
    let before = git(&repo, &["rev-parse", "main"]);
    std::fs::write(repo.join(".git/info/exclude"), "local.txt\n").unwrap();
    std::fs::write(repo.join("local.txt"), "ignored operator work\n").unwrap();
    run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--checks",
            "true",
            "--repo",
            repo.to_str().unwrap(),
        ],
    );
    let drained = json(&run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(drained["landed"], 0, "{drained}");
    assert_eq!(git(&repo, &["rev-parse", "main"]), before);
    assert_eq!(
        std::fs::read_to_string(repo.join("local.txt")).unwrap(),
        "ignored operator work\n"
    );
}

#[test]
fn checks_cannot_validate_modified_files_and_land_the_original_candidate() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "checked.txt", "original\n", "feature");
    let before = git(&repo, &["rev-parse", "main"]);
    run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--checks",
            "echo modified > checked.txt; test -f checked.txt",
            "--repo",
            repo.to_str().unwrap(),
        ],
    );
    let drained = json(&run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(drained["landed"], 0, "{drained}");
    assert!(
        drained["receipts"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("modified the candidate")
    );
    assert_eq!(git(&repo, &["rev-parse", "main"]), before);
}

#[test]
fn an_integrator_process_lock_cannot_be_reentered_by_the_same_session() {
    use fs2::FileExt;
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "exclusive.txt", "exclusive\n", "feature");
    run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--checks",
            "true",
            "--repo",
            repo.to_str().unwrap(),
        ],
    );
    let directory = repo.join(".git/harbard-sluice");
    std::fs::create_dir_all(&directory).unwrap();
    let holder = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("integrator.lock"))
        .unwrap();
    holder.lock_exclusive().unwrap();
    let blocked = run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    );
    assert!(
        !blocked.status.success(),
        "a second process must not drain the queue"
    );
    assert!(
        json(&blocked)["error"]
            .as_str()
            .unwrap()
            .contains("exclusively own")
    );
    drop(holder);
    let drained = json(&run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(
        drained["landed"], 1,
        "the lock is released with its owner: {drained}"
    );
}

#[test]
fn integration_refuses_a_clean_target_reserved_by_another_session() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "owned.txt", "owned\n", "feature");
    let before = git(&repo, &["rev-parse", "main"]);
    let root = std::fs::canonicalize(&repo).unwrap();
    let resource = harbard_core::LeaseResource::writer(
        harbard_core::hostname(),
        root.join(".git").display().to_string(),
        root.display().to_string(),
    )
    .unwrap();
    let metadata = harbard_core::LeaseMetadata {
        session: "other-owner".into(),
        harness: "claude-code".into(),
        pid: Some(std::process::id()),
        parent: None,
        repository_id: None,
        branch: Some("main".into()),
        head_sha: Some(before.clone()),
    };
    let leases = harbard_core::LeaseStore::at(&home);
    leases
        .claim(
            &resource,
            &metadata,
            900,
            harbard_core::SystemLeaseClock,
            &harbard_core::SystemPidChecker,
        )
        .unwrap();
    run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--checks",
            "true",
            "--repo",
            repo.to_str().unwrap(),
        ],
    );
    let drained = json(&run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(drained["landed"], 0, "{drained}");
    assert!(
        drained["receipts"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("reserved by session other-owner")
    );
    assert_eq!(git(&repo, &["rev-parse", "main"]), before);
    assert!(!repo.join("owned.txt").exists());
}

#[test]
fn integration_keeps_the_callers_existing_writer_reservation() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "retained.txt", "retained\n", "feature");
    let root = std::fs::canonicalize(&repo).unwrap();
    let resource = harbard_core::LeaseResource::writer(
        harbard_core::hostname(),
        root.join(".git").display().to_string(),
        root.display().to_string(),
    )
    .unwrap();
    let metadata = harbard_core::LeaseMetadata {
        session: "tester".into(),
        harness: "codex".into(),
        pid: Some(std::process::id()),
        parent: None,
        repository_id: None,
        branch: Some("main".into()),
        head_sha: None,
    };
    let leases = harbard_core::LeaseStore::at(&home);
    let harbard_core::LeaseClaim::Acquired(held) = leases
        .claim(
            &resource,
            &metadata,
            900,
            harbard_core::SystemLeaseClock,
            &harbard_core::SystemPidChecker,
        )
        .unwrap()
    else {
        panic!("fresh reservation");
    };
    run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--checks",
            "true",
            "--repo",
            repo.to_str().unwrap(),
        ],
    );
    let drained = json(&run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(drained["landed"], 1, "{drained}");
    let checked = leases
        .check(
            &resource,
            Some(&held.claim_id),
            false,
            harbard_core::SystemLeaseClock,
            &harbard_core::SystemPidChecker,
        )
        .unwrap();
    assert!(
        matches!(
            checked,
            harbard_core::LeaseCheck::Active {
                matches_claim: Some(true),
                ..
            }
        ),
        "existing reservation must survive: {checked:?}"
    );
}

#[test]
fn a_passing_delivery_lands_exactly_the_commit_that_was_checked() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "a.txt", "a\n", "feature");
    let before = git(&repo, &["rev-parse", "main"]);

    let queued = run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--branch",
            "main",
            "--checks",
            "test -f a.txt",
            "--repo",
            repo.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(queued.status.success(), "{queued:?}");
    assert_eq!(json(&queued)["status"], "queued");

    let drained = run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    );
    assert!(drained.status.success(), "{drained:?}");
    let v = json(&drained);
    assert_eq!(v["landed"], 1, "{v}");

    let receipt = &v["receipts"][0];
    assert_eq!(receipt["verdict"], "landed");
    let candidate = receipt["candidate"].as_str().expect("a candidate");

    // The whole point: the branch is at the commit the checks ran on.
    let after = git(&repo, &["rev-parse", "main"]);
    assert_eq!(after, candidate, "main must be the checked candidate");
    assert_ne!(after, before, "main should have moved");

    // And that candidate really is the merge of the two, not a rewrite.
    let parents = git(&repo, &["rev-list", "--parents", "-n", "1", &after]);
    assert!(
        parents.contains(&before),
        "candidate keeps the old tip: {parents}"
    );
    assert!(
        parents.contains(&feature),
        "candidate keeps the delivery: {parents}"
    );
}

#[test]
fn integration_uses_the_repository_identity_and_keeps_its_commit_guard() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "identity.txt", "identity\n", "identity-feature");
    git(&repo, &["config", "user.name", "Repository Operator"]);
    git(&repo, &["config", "user.email", "operator@example.test"]);
    let hook = repo.join(".git/hooks/commit-msg");
    std::fs::write(
        &hook,
        "#!/bin/sh\nset -eu\ncase \"$(git var GIT_AUTHOR_IDENT)\" in\n  \"Repository Operator <operator@example.test> \"*) ;;\n  *) echo 'unexpected author identity' >&2; exit 1 ;;\nesac\ncase \"$(git var GIT_COMMITTER_IDENT)\" in\n  \"Repository Operator <operator@example.test> \"*) ;;\n  *) echo 'unexpected committer identity' >&2; exit 1 ;;\nesac\n",
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let queued = run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--checks",
            "test -f identity.txt",
            "--repo",
            repo.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(queued.status.success(), "{queued:?}");

    let integrated = run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    );
    let result = json(&integrated);
    assert_eq!(result["landed"], 1, "{result}");
    let candidate = result["receipts"][0]["candidate"].as_str().unwrap();
    assert_eq!(git(&repo, &["rev-parse", "main"]), candidate);
    assert_eq!(
        git(
            &repo,
            &["show", "-s", "--format=%an <%ae>|%cn <%ce>", candidate]
        ),
        "Repository Operator <operator@example.test>|Repository Operator <operator@example.test>"
    );
}

#[test]
fn integration_refuses_to_guess_an_unconfigured_identity() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "identity.txt", "identity\n", "identity-feature");
    let before = git(&repo, &["rev-parse", "main"]);
    git(&repo, &["config", "--unset", "user.name"]);
    git(&repo, &["config", "--unset", "user.email"]);
    std::fs::write(home.join(".gitconfig"), "[user]\n\tuseConfigOnly = false\n").unwrap();
    let queued = run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--repo",
            repo.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(queued.status.success(), "{queued:?}");

    let integrated = run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    );
    let result = json(&integrated);
    assert_eq!(result["landed"], 0, "{result}");
    assert!(
        result["receipts"][0]["detail"]
            .as_str()
            .unwrap()
            .contains("identity"),
        "{result}"
    );
    assert_eq!(git(&repo, &["rev-parse", "main"]), before);
}

#[test]
fn a_failing_check_leaves_the_branch_exactly_where_it_was() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "b.txt", "b\n", "feature");
    let before = git(&repo, &["rev-parse", "main"]);

    run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--checks",
            "exit 3",
            "--repo",
            repo.to_str().unwrap(),
            "--json",
        ],
    );
    let drained = run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    );
    let v = json(&drained);
    assert_eq!(v["landed"], 0, "{v}");
    assert_eq!(v["receipts"][0]["verdict"], "rejected");
    assert_eq!(
        git(&repo, &["rev-parse", "main"]),
        before,
        "a rejected delivery must not move the branch"
    );
}

#[test]
fn a_conflict_is_a_verdict_and_the_queue_moves_on() {
    let home = tmp_home();
    let repo = repo_with_history();
    // Two deliveries touching the same line: the second cannot merge.
    let first = side_commit(&repo, "README.md", "one\n", "first");
    let second = side_commit(&repo, "README.md", "two\n", "second");

    for sha in [&first, &second] {
        run(
            &home,
            &[
                "deliver",
                "--sha",
                sha,
                "--checks",
                "true",
                "--repo",
                repo.to_str().unwrap(),
                "--json",
            ],
        );
    }
    let v = json(&run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    let verdicts: Vec<&str> = v["receipts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["verdict"].as_str().unwrap())
        .collect();
    assert_eq!(verdicts.len(), 2, "both were decided: {v}");
    assert!(verdicts.contains(&"landed"), "{verdicts:?}");
    assert!(
        verdicts.contains(&"conflict"),
        "the second must be refused, not crash: {verdicts:?}"
    );
}

#[test]
fn a_delivery_naming_a_commit_that_does_not_exist_is_refused_before_it_queues() {
    let home = tmp_home();
    let repo = repo_with_history();
    let out = run(
        &home,
        &[
            "deliver",
            "--sha",
            "0000000000000000000000000000000000000000",
            "--repo",
            repo.to_str().unwrap(),
            "--json",
        ],
    );
    assert!(!out.status.success(), "{out:?}");

    let listed = json(&run(
        &home,
        &["deliveries", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(
        listed["pending"].as_array().map(Vec::len),
        Some(0),
        "a refused delivery must leave nothing queued: {listed}"
    );
}

#[test]
fn receipts_survive_and_explain_what_happened() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "c.txt", "c\n", "feature");
    run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--checks",
            "true",
            "--repo",
            repo.to_str().unwrap(),
            "--json",
        ],
    );
    run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    );

    let listed = json(&run(
        &home,
        &["deliveries", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(listed["pending"].as_array().map(Vec::len), Some(0));
    let r = &listed["receipts"][0];
    for field in ["delivered_sha", "onto", "candidate", "verdict", "session"] {
        assert!(!r[field].is_null(), "receipt is missing {field}: {r}");
    }
    // Draining again is a no-op: a decided delivery never returns.
    let again = json(&run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(
        again["receipts"].as_array().map(Vec::len),
        Some(0),
        "{again}"
    );
}

/// A worktree is not an environment.
///
/// The first real delivery through the sluice was rejected because the
/// candidate worktree had no `node_modules`: in a workspace monorepo the
/// internal packages resolve through symlinks in the root's, and a fresh
/// worktree has tracked files and nothing else. The same tests passed 12/12 in
/// the normal checkout. Every delivery that runs tests would have hit it.
#[test]
fn a_delivery_can_declare_how_to_make_the_candidate_runnable() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "e.txt", "e\n", "feature");

    // Stands in for `node_modules`: untracked, so it exists in the checkout and
    // is absent from a fresh worktree — exactly the shape that broke the first
    // real delivery.
    std::fs::write(repo.join("dep.txt"), "dep\n").unwrap();
    let link = format!("ln -s {}/dep.txt dep.txt", repo.to_str().unwrap());

    run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--prepare",
            &link,
            "--checks",
            "test -f dep.txt && test -f e.txt",
            "--repo",
            repo.to_str().unwrap(),
            "--json",
        ],
    );
    let v = json(&run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(
        v["landed"], 1,
        "preparation should have made the checks pass: {v}"
    );
}

/// A broken environment is not a broken commit. Calling it `rejected` would
/// blame the delivery for a worktree that was never runnable.
#[test]
fn a_failed_preparation_is_reported_as_failed_not_as_a_rejection() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "f.txt", "f\n", "feature");
    let before = git(&repo, &["rev-parse", "main"]);

    run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--prepare",
            "exit 7",
            "--checks",
            "true",
            "--repo",
            repo.to_str().unwrap(),
            "--json",
        ],
    );
    let v = json(&run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    let r = &v["receipts"][0];
    assert_eq!(r["verdict"], "failed", "{r}");
    assert!(
        r["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("preparation"),
        "the receipt must say the environment failed, not the code: {r}"
    );
    assert_eq!(git(&repo, &["rev-parse", "main"]), before);
}

/// Deliveries queued before `prepare` existed must still integrate.
#[test]
fn a_delivery_written_without_the_prepare_field_still_reads() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "g.txt", "g\n", "feature");
    run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--checks",
            "true",
            "--repo",
            repo.to_str().unwrap(),
            "--json",
        ],
    );

    // Rewrite the queue as an older harbard would have written it.
    let listed = json(&run(
        &home,
        &["deliveries", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    let mut line = listed["pending"][0].clone();
    line.as_object_mut().unwrap().remove("prepare");
    // Found rather than recomputed: this test should not have to agree with the
    // store's hashing scheme to prove an older record still reads.
    let queue = std::fs::read_dir(home.join("sluice"))
        .expect("sluice dir")
        .filter_map(Result::ok)
        .map(|e| e.path().join("queue.jsonl"))
        .find(|p| p.exists())
        .expect("a queue file");
    std::fs::write(
        &queue,
        format!("{}\n", serde_json::to_string(&line).unwrap()),
    )
    .unwrap();

    let v = json(&run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(v["landed"], 1, "an older record must still integrate: {v}");
}

/// Moving a ref does not move a working tree. Landing into the branch this
/// checkout has open must bring the files with it — otherwise the operator
/// reads the old contents while Git insists the branch advanced.
#[test]
fn landing_into_the_checked_out_branch_moves_the_files_too() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "README.md", "changed\n", "feature");

    run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--checks",
            "true",
            "--repo",
            repo.to_str().unwrap(),
            "--json",
        ],
    );
    let v = json(&run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(v["landed"], 1, "{v}");
    assert_eq!(
        std::fs::read_to_string(repo.join("README.md")).unwrap(),
        "changed\n",
        "the working tree must follow the branch it has checked out"
    );
    assert!(
        git(&repo, &["status", "--porcelain"]).is_empty(),
        "and it must be clean afterwards"
    );
}

/// An untracked file is not uncommitted work.
///
/// Every real repository has them — `node_modules`, `target/`, a stray note —
/// and treating them as a dirty tree would mean such a repo could never receive
/// a delivery at all. A reset to the candidate leaves them untouched, so they
/// have nothing to lose.
#[test]
fn untracked_files_do_not_block_a_landing() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "h.txt", "h\n", "feature");
    std::fs::create_dir_all(repo.join("node_modules")).unwrap();
    std::fs::write(repo.join("node_modules").join("x.js"), "x\n").unwrap();

    run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--checks",
            "true",
            "--repo",
            repo.to_str().unwrap(),
            "--json",
        ],
    );
    let v = json(&run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(v["landed"], 1, "untracked files must not block: {v}");
    assert!(
        repo.join("node_modules").join("x.js").exists(),
        "and they must survive the landing"
    );
}

/// A branch someone is actively working on is not the sluice's to move.
#[test]
fn a_dirty_checkout_refuses_the_landing_instead_of_stealing_the_branch() {
    let home = tmp_home();
    let repo = repo_with_history();
    let feature = side_commit(&repo, "d.txt", "d\n", "feature");
    let before = git(&repo, &["rev-parse", "main"]);

    // The operator is mid-edit on the target branch.
    std::fs::write(repo.join("README.md"), "work in progress\n").unwrap();

    run(
        &home,
        &[
            "deliver",
            "--sha",
            &feature,
            "--checks",
            "true",
            "--repo",
            repo.to_str().unwrap(),
            "--json",
        ],
    );
    let v = json(&run(
        &home,
        &["integrate", "--repo", repo.to_str().unwrap(), "--json"],
    ));
    assert_eq!(v["landed"], 0, "{v}");
    assert_eq!(
        git(&repo, &["rev-parse", "main"]),
        before,
        "the branch must not move under an uncommitted tree"
    );
    assert_eq!(
        std::fs::read_to_string(repo.join("README.md")).unwrap(),
        "work in progress\n",
        "and the operator's edit must survive"
    );
}
