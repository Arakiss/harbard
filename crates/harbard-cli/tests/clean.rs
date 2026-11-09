//! `clean --finished`: the build dirs and worktrees of agents that are done,
//! listed with sizes, and removed only with `--yes`. Everything happens under
//! a throwaway `HOME`: a temp tree with its own `.cache`, its own git
//! repository and worktrees, and the team fixtures. The real `~/.cache` and
//! worktrees are never read or touched.
//!
//! Build dirs of finished agents accumulate and fill the disk.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const SID: &str = "0f0f0f0f-1111-2222-3333-444444444444";
/// 2026-11-06T10:31:00Z, as in `tests/team.rs`.
const NOW_MS: u64 = 1_793_961_060_000;

struct Tree(PathBuf);

/// What a throwaway tree holds besides the two fixture worktrees.
#[derive(Default)]
struct Setup<'a> {
    /// The target dir `notes` declares.
    notes_dir: &'a str,
    /// Where `notes` writes (its Edit and its records' cwd).
    notes_writes: &'a str,
    /// Where `notes` runs `git status`, when not where it writes.
    notes_reads: Option<&'a str>,
    /// More `harbard-wt-<name>` worktrees.
    worktrees: &'a [&'a str],
    /// Bash commands the lead runs, `(timestamp, command)`; `<root>` is the
    /// tree.
    lead_runs: &'a [(&'a str, &'a str)],
    /// Bash commands `dates` (working) runs, as `lead_runs`.
    dates_runs: &'a [(&'a str, &'a str)],
}

/// One Bash call by `agent` at `ts`, as Claude Code records it.
fn bash_record(agent: &str, uuid: &str, ts: &str, cmd: &str) -> String {
    let mut line = serde_json::json!({
        "parentUuid": null, "isSidechain": true, "agentId": agent,
        "type": "assistant", "uuid": uuid, "timestamp": ts,
        "userType": "external", "entrypoint": "cli", "cwd": "/work/demo",
        "sessionId": SID, "version": "2.1.318", "gitBranch": "main",
        "message": { "role": "assistant", "content": [{
            "type": "tool_use", "id": format!("toolu-{uuid}"), "name": "Bash",
            "input": { "command": cmd } }], "stop_reason": "tool_use" },
    })
    .to_string();
    line.push('\n');
    line
}

impl Drop for Tree {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let out = Command::new("git")
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/opt/homebrew/bin")
        .env("HOME", dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "init.defaultBranch=main",
        ])
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .expect("git");
    assert!(out.status.success(), "git {args:?}: {out:?}");
}

impl Tree {
    /// A throwaway home: a repository with two linked worktrees, one per
    /// fixture agent, and a build dir for each under its own `.cache`.
    /// `notes` (a1) is stopped by its lead (Closed); `dates` (a2) works.
    fn new(notes_dir: &str, notes_checkout: &str) -> Self {
        Self::build(&Setup {
            notes_dir,
            notes_writes: notes_checkout,
            ..Setup::default()
        })
    }

    /// [`Tree::new`] with every knob: extra worktrees, where `notes` only
    /// reads, and commands the lead runs.
    fn build(setup: &Setup<'_>) -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root =
            std::env::temp_dir().join(format!("harbard-clean-it-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // Canonical, so the paths the agents name match what is on disk
        // (`/var` is `/private/var` on macOS).
        let root = std::fs::canonicalize(&root).unwrap();
        let t = Tree(root);
        let repo = t.path("w/harbard");
        std::fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-q"]);
        std::fs::write(repo.join("README.md"), "x\n").unwrap();
        // Build output is ignored, as `target/` is in a real repository; an
        // untracked file that is not ignored counts as uncommitted work.
        std::fs::write(repo.join(".gitignore"), "*.bin\n").unwrap();
        git(&repo, &["add", "."]);
        git(&repo, &["commit", "-qm", "init"]);
        for name in ["notes", "dates"].iter().chain(setup.worktrees) {
            let wt = t.path(&format!("w/harbard-wt-{name}"));
            git(
                &repo,
                &["worktree", "add", "-q", "-b", name, wt.to_str().unwrap()],
            );
            std::fs::write(wt.join("built.bin"), vec![0u8; 4096]).unwrap();
            let target = t.path(&format!(".cache/harbard-{name}-target/debug"));
            std::fs::create_dir_all(&target).unwrap();
            std::fs::write(target.join("big.rlib"), vec![0u8; 100_000]).unwrap();
        }
        t.install(setup);
        t
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.0.join(rel)
    }

    fn s(&self, rel: &str) -> String {
        self.path(rel).display().to_string()
    }

    /// The team fixtures, moved onto this tree: `notes` declares
    /// `notes_dir` as its target dir and works in `notes_checkout`; `dates`
    /// declares its own and works in its worktree.
    fn install(&self, setup: &Setup<'_>) {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/team");
        let project = self.path(".claude/projects/-work-demo");
        let sub = project.join(SID).join("subagents");
        std::fs::create_dir_all(&sub).unwrap();
        let notes_wt = self.s(setup.notes_writes);
        let notes_reads = self.s(setup.notes_reads.unwrap_or(setup.notes_writes));
        let dates_wt = self.s("w/harbard-wt-dates");
        let notes_target = self.s(setup.notes_dir);
        let dates_target = self.s(".cache/harbard-dates-target");
        let rewrite = |name: &str, raw: &str| -> String {
            if name == "agent-a1.jsonl" {
                // Its `git status` runs where it reads; its Edit, and its
                // records' cwd, where it writes.
                raw.replace("cd /work/demo", &format!("cd {notes_reads}"))
                    .replace("/work/demo", &notes_wt)
                    .replace(
                        "START · notes · Checkout",
                        &format!("START · notes · target dir {notes_target} · Checkout"),
                    )
            } else {
                raw.replace("/work/demo", &dates_wt).replace(
                    "Fixed it, see /work/out/date.md",
                    &format!("START · dates · target dir {dates_target}"),
                )
            }
        };
        for name in [
            "agent-a1.jsonl",
            "agent-a1.meta.json",
            "agent-a2.jsonl",
            "agent-a2.meta.json",
        ] {
            let raw = std::fs::read_to_string(fixtures.join(name)).unwrap();
            let mut text = rewrite(name, &raw);
            if name == "agent-a2.jsonl" {
                for (i, (ts, cmd)) in setup.dates_runs.iter().enumerate() {
                    let cmd = cmd.replace("<root>", &self.s(""));
                    text.push_str(&bash_record("a2", &format!("u-a2-run-{i}"), ts, &cmd));
                }
            }
            std::fs::write(sub.join(name), text).unwrap();
        }
        // The lead stops `notes` at 10:25: Closed.
        let mut lead = std::fs::read_to_string(fixtures.join("lead.jsonl")).unwrap();
        lead.push_str(
            &serde_json::json!({
                "parentUuid": null, "isSidechain": true, "agentId": "lead",
                "type": "assistant", "uuid": "u-stop", "timestamp": "2026-11-06T10:25:00.000Z",
                "userType": "external", "entrypoint": "cli", "cwd": "/work/demo",
                "sessionId": SID, "version": "2.1.318", "gitBranch": "main",
                "message": { "role": "assistant", "content": [{
                    "type": "tool_use", "id": "toolu-stop", "name": "TaskStop",
                    "input": { "task_id": "notes" } }], "stop_reason": "tool_use" },
            })
            .to_string(),
        );
        lead.push('\n');
        for (i, (ts, cmd)) in setup.lead_runs.iter().enumerate() {
            let cmd = cmd.replace("<root>", &self.s(""));
            lead.push_str(
                &serde_json::json!({
                    "parentUuid": null, "isSidechain": true, "agentId": "lead",
                    "type": "assistant", "uuid": format!("u-run-{i}"), "timestamp": ts,
                    "userType": "external", "entrypoint": "cli", "cwd": "/work/demo",
                    "sessionId": SID, "version": "2.1.318", "gitBranch": "main",
                    "message": { "role": "assistant", "content": [{
                        "type": "tool_use", "id": format!("toolu-run-{i}"), "name": "Bash",
                        "input": { "command": cmd } }], "stop_reason": "tool_use" },
                })
                .to_string(),
            );
            lead.push('\n');
        }
        std::fs::write(project.join(format!("{SID}.jsonl")), lead).unwrap();
    }

    fn clean(&self, extra: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .env("HOME", &self.0)
            .env("PATH", "/usr/bin:/bin:/opt/homebrew/bin")
            .env("HARBARD_NOW_MS", NOW_MS.to_string())
            .env("HARBARD_API", "http://127.0.0.1:9")
            .args(["clean", "--finished", "--json"])
            .args(extra)
            .arg("--home")
            .arg(self.path(".harbard"))
            .args(["--cwd", self.0.to_str().unwrap()])
            .args(["--session", SID, "--harness", "claude-code"])
            .output()
            .unwrap()
    }

    fn report(&self, extra: &[&str]) -> serde_json::Value {
        let out = self.clean(extra);
        assert!(out.status.success(), "{out:?}");
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {out:?}"))
    }
}

fn entry<'a>(v: &'a serde_json::Value, path: &Path) -> Option<&'a serde_json::Value> {
    v["paths"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["path"].as_str() == path.to_str())
}

#[test]
fn clean_lists_a_finished_agents_build_dir_and_worktree_and_removes_nothing() {
    let t = Tree::new(".cache/harbard-notes-target", "w/harbard-wt-notes");
    let v = t.report(&[]);
    let target = t.path(".cache/harbard-notes-target");
    let wt = t.path("w/harbard-wt-notes");
    let build = entry(&v, &target).unwrap_or_else(|| panic!("{v}"));
    assert_eq!(build["kind"], "build_dir", "{v}");
    assert_eq!(build["verdict"], "remove", "{v}");
    assert!(build["bytes"].as_u64().unwrap() >= 100_000, "{v}");
    let tree = entry(&v, &wt).unwrap_or_else(|| panic!("{v}"));
    assert_eq!(tree["kind"], "worktree", "{v}");
    assert_eq!(tree["verdict"], "remove", "{v}");
    assert_eq!(v["removed"], 0);
    assert!(
        target.exists() && wt.exists(),
        "nothing removed without --yes"
    );
}

#[test]
fn clean_yes_removes_only_the_finished_agents_paths() {
    let t = Tree::new(".cache/harbard-notes-target", "w/harbard-wt-notes");
    let v = t.report(&["--yes"]);
    assert_eq!(v["removed"], 2, "{v}");
    assert!(!t.path(".cache/harbard-notes-target").exists());
    assert!(!t.path("w/harbard-wt-notes").exists());
    // The working agent's are untouched, and so is the source checkout.
    assert!(
        t.path(".cache/harbard-dates-target/debug/big.rlib")
            .exists()
    );
    assert!(t.path("w/harbard-wt-dates/built.bin").exists());
    assert!(t.path("w/harbard/README.md").exists());
    // The repository forgot the removed worktree.
    let list = Command::new("git")
        .arg("-C")
        .arg(t.path("w/harbard"))
        .args(["worktree", "list"])
        .output()
        .unwrap();
    let list = String::from_utf8_lossy(&list.stdout);
    assert!(!list.contains("harbard-wt-notes"), "{list}");
    assert!(list.contains("harbard-wt-dates"), "{list}");
}

#[test]
fn clean_never_removes_an_active_agents_paths() {
    // `notes` is done, but it declared the build dir and worked in the
    // worktree that `dates` still uses.
    let t = Tree::new(".cache/harbard-dates-target", "w/harbard-wt-dates");
    let v = t.report(&["--yes"]);
    assert_eq!(v["removed"], 0, "{v}");
    for p in [".cache/harbard-dates-target", "w/harbard-wt-dates"] {
        let e = entry(&v, &t.path(p)).unwrap_or_else(|| panic!("{p}: {v}"));
        assert_eq!(e["verdict"], "kept", "{p}: {v}");
        assert_eq!(e["reason"], "in_use", "{p}: {v}");
        assert!(t.path(p).exists(), "{p}");
    }
}

#[test]
fn clean_never_removes_anything_outside_its_two_places() {
    for (dir, why) in [
        (".harbard", "outside"),
        (".cache", "outside"),
        (".cache/other-target", "outside"),
        ("precious", "outside"),
        ("w/harbard", "outside"),
    ] {
        let t = Tree::new(dir, "w/harbard");
        std::fs::create_dir_all(t.path(dir)).unwrap();
        std::fs::write(t.path(dir).join("keep.txt"), "keep").unwrap();
        let v = t.report(&["--yes"]);
        let e = entry(&v, &t.path(dir)).unwrap_or_else(|| panic!("{dir}: {v}"));
        assert_eq!(e["verdict"], "kept", "{dir}: {v}");
        assert_eq!(e["reason"], why, "{dir}: {v}");
        assert!(t.path(dir).join("keep.txt").exists(), "{dir} must survive");
        assert!(
            t.path("w/harbard/README.md").exists(),
            "{dir}: source survives"
        );
        assert!(t.path(".cache/harbard-dates-target").exists(), "{dir}");
    }
}

#[test]
fn clean_keeps_a_worktree_with_uncommitted_work() {
    let t = Tree::new(".cache/harbard-notes-target", "w/harbard-wt-notes");
    std::fs::write(t.path("w/harbard-wt-notes/unsaved.rs"), "fn main() {}\n").unwrap();
    let v = t.report(&["--yes"]);
    let e = entry(&v, &t.path("w/harbard-wt-notes")).unwrap();
    assert_eq!(e["verdict"], "kept", "{v}");
    assert_eq!(e["reason"], "uncommitted", "{v}");
    assert!(t.path("w/harbard-wt-notes/unsaved.rs").exists());
    // The build dir has nothing to lose and goes.
    assert!(!t.path(".cache/harbard-notes-target").exists());
}

#[test]
fn clean_without_finished_refuses() {
    let t = Tree::new(".cache/harbard-notes-target", "w/harbard-wt-notes");
    let out = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .env("HOME", &t.0)
        .args(["clean", "--yes", "--home"])
        .arg(t.path(".harbard"))
        .output()
        .unwrap();
    assert!(!out.status.success(), "{out:?}");
    assert!(t.path(".cache/harbard-notes-target").exists());
}

#[test]
fn clean_is_a_machine_operation_that_lists_by_default() {
    let t = Tree::new(".cache/harbard-notes-target", "w/harbard-wt-notes");
    let out = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .env("HOME", &t.0)
        .env("PATH", "/usr/bin:/bin:/opt/homebrew/bin")
        .env("HARBARD_NOW_MS", NOW_MS.to_string())
        .args(["call", "clean", "--home"])
        .arg(t.path(".harbard"))
        .args(["--cwd", t.0.to_str().unwrap()])
        .args(["--session", SID, "--harness", "claude-code"])
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["v"], "harbard/cli/v1", "{v}");
    assert_eq!(v["data"]["dry_run"], true, "{v}");
    assert!(t.path(".cache/harbard-notes-target").exists());
}

/// The lead's integration worktree, exactly as on the operator's Mac: a finished
/// agent (dock) wrote in it earlier, and the lead merges and runs gates in
/// it every few minutes. It was listed as "remove (worktree of dock)".
#[test]
fn a_worktree_the_lead_works_in_is_kept_although_a_finished_agent_wrote_there() {
    for runs in [
        &[(
            "2026-11-06T10:28:00.000Z",
            "cd <root>/w/harbard-wt-integ && cargo test --workspace",
        )][..],
        &[(
            "2026-11-06T10:29:00.000Z",
            "git -C <root>/w/harbard-wt-integ merge --no-edit dates",
        )][..],
        &[(
            "2026-11-06T10:29:30.000Z",
            "git -C <root>/w/harbard-wt-integ log --oneline -3",
        )][..],
    ] {
        let t = Tree::build(&Setup {
            notes_dir: ".cache/harbard-notes-target",
            notes_writes: "w/harbard-wt-integ",
            worktrees: &["integ"],
            lead_runs: runs,
            ..Setup::default()
        });
        let v = t.report(&["--yes"]);
        let e = entry(&v, &t.path("w/harbard-wt-integ")).unwrap_or_else(|| panic!("{v}"));
        assert_eq!(e["verdict"], "kept", "{runs:?}: {v}");
        assert_eq!(e["reason"], "in_use", "{runs:?}: {v}");
        assert!(t.path("w/harbard-wt-integ/README.md").exists(), "{runs:?}");
    }
}

/// A reviewer that only read a worktree (`git status`, `git diff`) never
/// owns it: harbard-wt-protocolo was listed as Hazel's.
#[test]
fn a_worktree_a_finished_agent_only_read_is_never_its_own() {
    let t = Tree::build(&Setup {
        notes_dir: ".cache/harbard-notes-target",
        notes_writes: "w/harbard-wt-notes",
        notes_reads: Some("w/harbard-wt-protocol"),
        worktrees: &["protocol"],
        ..Setup::default()
    });
    let v = t.report(&["--yes"]);
    assert!(
        entry(&v, &t.path("w/harbard-wt-protocol")).is_none(),
        "a reader is not an owner: {v}"
    );
    assert!(t.path("w/harbard-wt-protocol/README.md").exists());
    // Its own worktree, where it wrote, is still its own.
    assert!(!t.path("w/harbard-wt-notes").exists(), "{v}");
}

#[test]
fn a_worktree_with_commits_not_on_main_is_kept() {
    let t = Tree::new(".cache/harbard-notes-target", "w/harbard-wt-notes");
    let wt = t.path("w/harbard-wt-notes");
    std::fs::write(wt.join("work.rs"), "fn main() {}\n").unwrap();
    git(&wt, &["add", "work.rs"]);
    git(&wt, &["commit", "-qm", "work"]);
    let v = t.report(&["--yes"]);
    let e = entry(&v, &wt).unwrap();
    assert_eq!(e["verdict"], "kept", "{v}");
    assert_eq!(e["reason"], "unmerged", "{v}");
    assert!(wt.join("work.rs").exists());
}

#[test]
fn a_worktree_a_process_runs_in_is_kept() {
    let t = Tree::new(".cache/harbard-notes-target", "w/harbard-wt-notes");
    let wt = t.path("w/harbard-wt-notes");
    let mut sleeper = Command::new("sleep")
        .arg("60")
        .current_dir(wt.join("."))
        .spawn()
        .unwrap();
    let v = t.report(&["--yes"]);
    let _ = sleeper.kill();
    let _ = sleeper.wait();
    let e = entry(&v, &wt).unwrap();
    assert_eq!(e["verdict"], "kept", "{v}");
    assert_eq!(e["reason"], "process_inside", "{v}");
    assert!(wt.exists());
}

/// Two writers, one of them the lead itself: nobody can say whose it is.
#[test]
fn a_worktree_with_several_writers_is_kept() {
    let t = Tree::build(&Setup {
        notes_dir: ".cache/harbard-notes-target",
        notes_writes: "w/harbard-wt-notes",
        // A git write by the lead long ago: an owner, but not a recent use.
        lead_runs: &[(
            "2026-11-05T09:00:00.000Z",
            "git -C <root>/w/harbard-wt-notes commit --allow-empty -m x",
        )],
        ..Setup::default()
    });
    let v = t.report(&["--yes"]);
    let e = entry(&v, &t.path("w/harbard-wt-notes")).unwrap_or_else(|| panic!("{v}"));
    assert_eq!(e["verdict"], "kept", "{v}");
    assert_eq!(e["reason"], "several_owners", "{v}");
}

/// A finished agent declared the lead's shared build dir in its START
/// line ("target dir ~/.cache/harbard-integ-target"). The lead builds there a
/// minute ago with CARGO_TARGET_DIR. Nothing keeps it: lead_cmd_dirs records
/// only cd / -C / cwd, never CARGO_TARGET_DIR, and a build dir has no content
/// guard at all. Kept (in_use).
#[test]
fn the_leads_shared_build_dir_is_kept() {
    let t = Tree::build(&Setup {
        notes_dir: ".cache/harbard-integ-target",
        notes_writes: "w/harbard-wt-notes",
        worktrees: &["integ"],
        lead_runs: &[(
            "2026-11-06T10:30:00.000Z",
            "CARGO_TARGET_DIR=<root>/.cache/harbard-integ-target cargo test --workspace",
        )],
        ..Setup::default()
    });
    let target = t.path(".cache/harbard-integ-target");
    let v = t.report(&["--yes"]);
    let e = entry(&v, &target).unwrap_or_else(|| panic!("{v}"));
    assert_eq!(e["verdict"], "kept", "the lead's build dir went: {v}");
    assert!(target.join("debug/big.rlib").exists());
}

/// A worktree nested inside the finished agent's worktree, under a
/// path the repository ignores (`.worktrees/`, a common convention), with
/// uncommitted work. The outer `git status --porcelain` does not list
/// ignored paths, so the outer worktree reads clean and `remove_dir_all`
/// takes the inner one with it. Kept.
#[test]
fn a_nested_worktree_with_uncommitted_work_survives() {
    let t = Tree::new(".cache/harbard-notes-target", "w/harbard-wt-notes");
    let repo = t.path("w/harbard");
    std::fs::write(repo.join(".git/info/exclude"), ".worktrees/\n").unwrap();
    let inner = t.path("w/harbard-wt-notes/.worktrees/fix");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "fix",
            inner.to_str().unwrap(),
        ],
    );
    std::fs::write(inner.join("wip.rs"), "fn unsaved() {}\n").unwrap();
    let v = t.report(&["--yes"]);
    assert!(
        inner.join("wip.rs").exists(),
        "uncommitted work in a nested worktree was deleted: {v}"
    );
}

/// `status.showUntrackedFiles=no` in the repository hides an untracked
/// file from `git status --porcelain`, so a worktree with new, never-added
/// work reads clean and is removed. Kept (uncommitted).
#[test]
fn untracked_work_hidden_by_repo_config_survives() {
    let t = Tree::new(".cache/harbard-notes-target", "w/harbard-wt-notes");
    git(
        &t.path("w/harbard"),
        &["config", "status.showUntrackedFiles", "no"],
    );
    let wt = t.path("w/harbard-wt-notes");
    std::fs::write(wt.join("new_module.rs"), "pub fn work() {}\n").unwrap();
    let v = t.report(&["--yes"]);
    assert!(
        wt.join("new_module.rs").exists(),
        "untracked work deleted: {v}"
    );
}

/// A process whose cwd has a non-ASCII name (worktrees are named after
/// teams; workshop names are Hazel, Victor…) is invisible to the lsof check
/// when lsof runs without a UTF-8 locale (env_clear here, a LaunchAgent or a
/// hook in real life): lsof prints `pi\xc3\xb1\xc3\xb3n`, canonicalize fails,
/// and the raw escaped path never matches. Kept (process_inside).
#[test]
fn a_process_in_a_unicode_worktree_is_seen() {
    let t = Tree::build(&Setup {
        notes_dir: ".cache/harbard-notes-target",
        notes_writes: "w/harbard-wt-pinion",
        worktrees: &["pinion"],
        ..Setup::default()
    });
    let wt = t.path("w/harbard-wt-pinion");
    let mut sleeper = Command::new("sleep")
        .arg("60")
        .current_dir(&wt)
        .spawn()
        .unwrap();
    let v = t.report(&["--yes"]);
    let _ = sleeper.kill();
    let _ = sleeper.wait();
    assert!(wt.exists(), "removed under a running process: {v}");
}

/// A build dir is known by its contents. A folder under
/// `~/.cache/harbard*` that an agent named but that carries neither cargo's
/// `CACHEDIR.TAG` nor a `debug/` or `release/` layout (a fixture home, a
/// demo) is kept.
#[test]
fn a_named_cache_folder_that_is_not_a_build_dir_is_kept() {
    let t = Tree::new(".cache/harbard-feedback-fixture", "w/harbard-wt-notes");
    let fixture = t.path(".cache/harbard-feedback-fixture");
    std::fs::create_dir_all(&fixture).unwrap();
    std::fs::write(fixture.join("profile.json"), "{}").unwrap();
    let v = t.report(&["--yes"]);
    let e = entry(&v, &fixture).unwrap_or_else(|| panic!("{v}"));
    assert_eq!(e["reason"], "not_a_build_dir", "{v}");
    assert!(fixture.join("profile.json").exists());
    // A cargo target dir with only its tag still goes.
    let tagged = Tree::new(".cache/harbard-tagged-target", "w/harbard-wt-notes");
    let dir = tagged.path(".cache/harbard-tagged-target");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("CACHEDIR.TAG"),
        "Signature: 8a477f597d28d172789f06886806bc55\n# cargo\n",
    )
    .unwrap();
    let v = tagged.report(&["--yes"]);
    assert_eq!(entry(&v, &dir).unwrap()["verdict"], "removed", "{v}");
}

/// A working agent that builds into a finished agent's target
/// dir with `--target-dir` uses it.
#[test]
fn a_build_dir_a_working_agent_builds_into_is_kept() {
    let t = Tree::build(&Setup {
        notes_dir: ".cache/harbard-notes-target",
        notes_writes: "w/harbard-wt-notes",
        dates_runs: &[(
            "2026-11-06T10:16:00.000Z",
            "cargo test --target-dir <root>/.cache/harbard-notes-target",
        )],
        ..Setup::default()
    });
    let target = t.path(".cache/harbard-notes-target");
    let v = t.report(&["--yes"]);
    let e = entry(&v, &target).unwrap_or_else(|| panic!("{v}"));
    assert_eq!(e["reason"], "in_use", "{v}");
    assert!(target.join("debug/big.rlib").exists());
}

/// A plain repository cloned inside a finished agent's worktree
/// (a vendored checkout, ignored by the outer one) is kept with it, even
/// when it is clean: what lives there is not the worktree's to lose.
#[test]
fn a_repository_inside_a_worktree_keeps_it() {
    let t = Tree::new(".cache/harbard-notes-target", "w/harbard-wt-notes");
    let wt = t.path("w/harbard-wt-notes");
    std::fs::write(t.path("w/harbard/.git/info/exclude"), "vendor/\n").unwrap();
    let inner = wt.join("vendor/lib");
    std::fs::create_dir_all(&inner).unwrap();
    git(&inner, &["init", "-q"]);
    let v = t.report(&["--yes"]);
    let e = entry(&v, &wt).unwrap_or_else(|| panic!("{v}"));
    assert_eq!(e["reason"], "nested_checkout", "{v}");
    assert!(inner.join(".git").exists());
}
