//! `harbard clean --finished [--yes]`: the disk finished agents leave behind.
//!
//! Build dirs of finished agents fill the disk. This lists the build dirs and
//! worktrees of agents the console shows
//! as Closed or finished, with their sizes, and removes them only with
//! `--yes`.
//!
//! A path belongs to the agents that WROTE in it (the write attribution
//! channels use) or, for a build dir, that declared it. A reader never owns
//! a path: a reviewer that ran `git diff` in a worktree is not its owner.
//!
//! What it may remove is fenced on every side, and each refusal is reported
//! with its reason, checked in this order:
//! - `outside`: only a directory under `~/.cache` whose first component
//!   starts with `harbard`, or a linked git worktree whose folder is
//!   `harbard-wt-*`; never `~/.cache` itself, `~/.harbard`, the home, a
//!   source checkout or anything else;
//! - `in_use`: a working agent's path, any directory the lead ran a command
//!   in (its cwd, a `cd`, a `-C`) in the last 24 hours, a build dir the
//!   lead or a working agent named in a command (`CARGO_TARGET_DIR=`,
//!   `--target-dir`), a live bus session's checkout or cwd, or this shell's
//!   cwd, is the path or inside it;
//! - `process_inside`: a running process has its cwd inside it (lsof, under
//!   a UTF-8 locale, its `\xNN` escapes decoded); `unverifiable` when that
//!   cannot be checked or a cwd cannot be read back, or when the transcript
//!   format moved and the team view cannot say who works where;
//! - `several_owners`, `unknown_owner`: more than one writer, or a writer
//!   the view does not know;
//! - `not_a_build_dir`: a build dir that carries neither cargo's
//!   `CACHEDIR.TAG` nor a `debug/` or `release/` layout: a fixture, a demo
//!   home or anything else an agent happened to name;
//! - `unmerged`: the worktree's branch has commits not on main;
//! - `uncommitted`, `unreadable`: changes or untracked files, whatever the
//!   repository's `status.showUntrackedFiles` says, or git cannot read the
//!   worktree;
//! - `nested_checkout`: any `.git` (file or directory) below it, other than
//!   a worktree's own, or a worktree of the same repository living inside
//!   it. Ignored paths are invisible to `git status`, so a worktree under an
//!   ignored `.worktrees/` would otherwise go with its outer one. The walk
//!   follows no symlink; a folder it cannot read is `unreadable`.
//!
//! Removal is one `remove_dir_all` on the literal canonical path, no shell
//! and no glob; a removed worktree is then pruned from its repository.

use crate::cli::Common;
use crate::context::Context;
use harbard_core::{Liveness, Store, Window, query};
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// A command the lead ran in a directory this recently keeps it.
const LEAD_RECENT_MS: u64 = 24 * 3_600_000;

#[derive(Debug, Clone, Serialize)]
pub struct CleanPath {
    pub path: String,
    /// `build_dir` or `worktree`.
    pub kind: &'static str,
    /// The finished agent it belongs to (its writers, when several).
    pub agent: String,
    pub bytes: u64,
    /// `remove` (listed), `removed`, `kept` or `failed`.
    pub verdict: &'static str,
    /// Why it is kept (see the module doc), or the error that stopped a
    /// removal.
    pub reason: Option<String>,
}

/// What a walk of a candidate found, without following symlinks.
struct Scan {
    bytes: u64,
    /// A `.git` below it other than a worktree's own top-level one.
    nested_git: bool,
    /// A folder or entry it could not read.
    unreadable: bool,
}

/// Walk `dir` once: its size, any nested `.git`, anything unreadable.
/// `own_git` is a linked worktree's top-level `.git` file, which is its own.
fn scan(dir: &Path, own_git: bool) -> Scan {
    let mut out = Scan {
        bytes: 0,
        nested_git: false,
        unreadable: false,
    };
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            out.unreadable = true;
            continue;
        };
        for e in entries {
            let Ok(e) = e else {
                out.unreadable = true;
                continue;
            };
            let path = e.path();
            if e.file_name() == ".git" && !(own_git && d == dir) {
                out.nested_git = true;
            }
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                out.unreadable = true;
                continue;
            };
            if meta.is_dir() {
                stack.push(path);
            } else {
                out.bytes += meta.len();
            }
        }
    }
    out
}

/// The tag cargo writes at the top of every target dir.
const CACHEDIR_SIGNATURE: &[u8] = b"Signature: 8a477f597d28d172789f06886806bc55";

/// A build dir by its contents: cargo's `CACHEDIR.TAG`, or the `debug/` or
/// `release/` it builds into.
fn looks_built(dir: &Path) -> bool {
    let tag = std::fs::read(dir.join("CACHEDIR.TAG"))
        .is_ok_and(|raw| raw.starts_with(CACHEDIR_SIGNATURE));
    tag || ["debug", "release"]
        .iter()
        .any(|d| std::fs::symlink_metadata(dir.join(d)).is_ok_and(|m| m.is_dir()))
}

/// Another worktree of the same repository lives inside `path`, as
/// `git worktree list --porcelain` sees it; `None` when git cannot say.
fn worktree_inside(path: &Path) -> Option<bool> {
    let list = git(path, &["worktree", "list", "--porcelain"])?;
    Some(
        list.lines()
            .filter_map(|l| l.strip_prefix("worktree "))
            .map(|w| std::fs::canonicalize(w).unwrap_or_else(|_| PathBuf::from(w)))
            .any(|w| w != path && w.starts_with(path)),
    )
}

fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A linked worktree: its `.git` is a file pointing into the main repository.
fn linked_worktree(path: &Path) -> bool {
    std::fs::symlink_metadata(path.join(".git")).is_ok_and(|m| m.is_file())
}

/// Whether `path` is one of the two places clean may ever remove.
fn allowed(path: &Path, kind: &str, home: &Path, cache: &Path, store: &Path) -> bool {
    if path == home || path == cache || path.starts_with(store) || store.starts_with(path) {
        return false;
    }
    match kind {
        "build_dir" => path
            .strip_prefix(cache)
            .ok()
            .and_then(|rest| rest.components().next())
            .is_some_and(|first| first.as_os_str().to_string_lossy().starts_with("harbard")),
        _ => {
            path.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("harbard-wt-"))
                && linked_worktree(path)
        }
    }
}

/// `active` is inside `candidate`, or is it: removing the candidate would
/// pull the floor from under whoever uses `active`.
fn holds(candidate: &Path, active: &Path) -> bool {
    active.starts_with(candidate)
}

/// The nearest `harbard-wt-*` folder holding `dir`, if any (lexical).
fn worktree_of(dir: &str) -> Option<String> {
    Path::new(dir)
        .ancestors()
        .find(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("harbard-wt-"))
        })
        .map(|p| p.display().to_string())
}

/// Where an agent WROTE: its write spans, the checkouts they fold into, and
/// the worktree it was launched in. Reads (git dirs, roots) are not here.
fn written_dirs(r: &harbard_core::team::AgentRow) -> Vec<&str> {
    let mut dirs: Vec<&str> = r.worktree.as_deref().into_iter().collect();
    dirs.extend(r.checkouts.iter().map(|c| c.path.as_str()));
    dirs.extend(r.write_dirs.iter().map(|c| c.path.as_str()));
    dirs
}

/// Everything an agent touched, reads included: what keeps a path while
/// the agent still works.
fn touched_dirs(r: &harbard_core::team::AgentRow) -> Vec<&str> {
    let mut dirs = written_dirs(r);
    dirs.extend(r.git_dirs.iter().map(String::as_str));
    dirs.extend(r.git_roots.iter().map(String::as_str));
    dirs.extend(r.target_dirs.iter().map(String::as_str));
    dirs
}

fn canonical(raw: &str, home: &Path) -> Option<PathBuf> {
    let expanded = match raw.strip_prefix("~/") {
        Some(rest) => home.join(rest),
        None => PathBuf::from(raw),
    };
    expanded
        .is_absolute()
        .then(|| std::fs::canonicalize(&expanded).ok())
        .flatten()
}

/// lsof's escapes undone: `\xNN`, `\b \f \t \n \r` and `^@`…`^_`, `^?`.
/// Anything else is kept as written.
fn lsof_unescape(raw: &str) -> Vec<u8> {
    let b = raw.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let hex = |at: usize| {
            b.get(at..at + 2)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        };
        match (b[i], b.get(i + 1).copied()) {
            (b'\\', Some(b'x')) if hex(i + 2).is_some() => {
                out.extend(hex(i + 2));
                i += 4;
            }
            (b'\\', Some(c @ (b'b' | b'f' | b't' | b'n' | b'r'))) => {
                out.push(match c {
                    b'b' => 0x08,
                    b'f' => 0x0c,
                    b't' => b'\t',
                    b'n' => b'\n',
                    _ => b'\r',
                });
                i += 2;
            }
            (b'^', Some(c @ b'@'..=b'_')) => {
                out.push(c - b'@');
                i += 2;
            }
            (b'^', Some(b'?')) => {
                out.push(0x7f);
                i += 2;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    out
}

/// One cwd as lsof printed it, read back: decoded, then as written. `None`
/// when neither exists and the line carried an escape, so the path itself
/// is unknown.
fn lsof_path(raw: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let decoded = PathBuf::from(std::ffi::OsString::from_vec(lsof_unescape(raw)));
    if let Ok(p) = std::fs::canonicalize(&decoded) {
        return Some(p);
    }
    if let Ok(p) = std::fs::canonicalize(raw) {
        return Some(p);
    }
    // Gone since (a deleted cwd): the name is still exact unless escaped.
    (!raw.contains(['\\', '^'])).then_some(decoded)
}

/// The cwd of every running process this user can see, or `None` when it
/// cannot be read (then nothing is removed). lsof runs under a UTF-8 locale,
/// so a name like `harbard-wt-piñón` prints as itself even from a
/// LaunchAgent or a hook with no locale; what it still escapes is decoded,
/// and one cwd that cannot be read back makes the whole check unverifiable.
fn process_cwds() -> Option<Vec<PathBuf>> {
    for lsof in ["/usr/sbin/lsof", "lsof"] {
        let Ok(out) = Command::new(lsof)
            .env("LC_ALL", "en_US.UTF-8")
            .env("LANG", "en_US.UTF-8")
            .args(["-a", "-d", "cwd", "-Fn"])
            .output()
        else {
            continue;
        };
        let text = String::from_utf8_lossy(&out.stdout);
        let cwds: Option<Vec<PathBuf>> = text
            .lines()
            .filter_map(|l| l.strip_prefix('n'))
            .filter(|p| p.starts_with('/'))
            .map(|p| (!p.contains('\u{fffd}')).then(|| lsof_path(p)).flatten())
            .collect();
        let cwds = cwds?;
        if !cwds.is_empty() {
            return Some(cwds);
        }
    }
    // Linux without lsof: /proc/<pid>/cwd.
    let entries = std::fs::read_dir("/proc").ok()?;
    let cwds: Vec<PathBuf> = entries
        .flatten()
        .filter_map(|e| std::fs::read_link(e.path().join("cwd")).ok())
        .collect();
    (!cwds.is_empty()).then_some(cwds)
}

/// Whether the worktree's branch has commits that are not on main.
fn unmerged(path: &Path) -> Option<bool> {
    let base = ["main", "master", "origin/main"].into_iter().find(|b| {
        git(
            path,
            &[
                "rev-parse",
                "--verify",
                "--quiet",
                &format!("{b}^{{commit}}"),
            ],
        )
        .is_some()
    })?;
    let count = git(path, &["rev-list", "--count", &format!("{base}..HEAD")])?;
    Some(count.trim().parse::<u64>().ok()? > 0)
}

pub fn cmd_clean(store: &Store, common: &Common, finished: bool, yes: bool, now: u64) -> i32 {
    if !finished {
        return crate::fail(
            "clean only knows --finished: the build dirs and worktrees of agents that are done (add --yes to remove them)",
        );
    }
    let Some(home) = std::env::var_os("HOME")
        .map(PathBuf::from)
        .and_then(|h| std::fs::canonicalize(h).ok())
    else {
        return crate::fail("clean needs HOME to find ~/.cache");
    };
    let cache = home.join(".cache");
    let store_root =
        std::fs::canonicalize(store.root()).unwrap_or_else(|_| store.root().to_path_buf());
    let ctx = Context::resolve(common);
    let read = store.read(Window::All);
    let events = read.events;
    let team = crate::serve::team_route::project(now, harbard_core::team::WINDOW_DEFAULT_MS);
    let view =
        crate::serve::workspace_route::view(store, &events, crate::stale_secs(common), &team, now);
    let rows: BTreeMap<&str, &harbard_core::team::AgentRow> = team
        .leads
        .iter()
        .flat_map(|l| l.agents.iter())
        .map(|r| (r.id.as_str(), r))
        .collect();
    // Agent key → (finished, shown name).
    let agents: BTreeMap<&str, (bool, String)> = view
        .agents
        .iter()
        .filter(|a| a.kind != "session")
        .map(|a| (a.key.as_str(), (a.presence == "finished", a.name.clone())))
        .collect();

    // Owners: who wrote in each worktree, who declared each build dir.
    // The leads' own writes make them owners too.
    let mut owners: BTreeMap<(PathBuf, &'static str), BTreeSet<String>> = BTreeMap::new();
    let mut active: Vec<PathBuf> = Vec::new();
    for a in view.agents.iter().filter(|a| a.kind != "session") {
        let finished = a.presence == "finished";
        if let Some(p) = a.build_dir.as_deref().and_then(|b| canonical(b, &home)) {
            if !finished {
                active.push(p.clone());
            }
            owners
                .entry((p, "build_dir"))
                .or_default()
                .insert(a.key.clone());
        }
        let Some(row) = rows.get(a.key.as_str()) else {
            continue;
        };
        for d in written_dirs(row) {
            if let Some(p) = worktree_of(d).and_then(|w| canonical(&w, &home)) {
                owners
                    .entry((p, "worktree"))
                    .or_default()
                    .insert(a.key.clone());
            }
        }
        if !finished {
            active.extend(
                touched_dirs(row)
                    .into_iter()
                    .filter_map(|d| canonical(d, &home)),
            );
        }
    }
    for lead in &team.leads {
        let key = format!("lead:{}", lead.session);
        for c in &lead.lead_write_dirs {
            if let Some(p) = worktree_of(&c.path).and_then(|w| canonical(&w, &home)) {
                owners
                    .entry((p, "worktree"))
                    .or_default()
                    .insert(key.clone());
            }
        }
        // Where the lead ran anything lately is the lead's: it merges and
        // runs gates in an integration worktree another agent once wrote in.
        active.extend(
            lead.lead_cmd_dirs
                .iter()
                .filter(|(_, ms)| now.saturating_sub(*ms) <= LEAD_RECENT_MS)
                .filter_map(|(d, _)| canonical(d, &home)),
        );
    }
    for s in query::sessions(store, &events, now, crate::stale_secs(common)) {
        if s.liveness == Liveness::Live {
            active.extend(s.repo.as_deref().and_then(|r| canonical(r, &home)));
            active.extend(canonical(&s.cwd, &home));
        }
    }
    active.extend(canonical(&ctx.cwd, &home));
    // A transcript format the projection no longer reads hides who works
    // where: then nothing can be shown unused.
    let processes = process_cwds().filter(|_| !team.format.format_changed());

    let mut out: Vec<CleanPath> = Vec::new();
    for ((path, kind), who) in owners {
        // Only what a finished agent owns is a candidate at all.
        if !who
            .iter()
            .any(|k| agents.get(k.as_str()).is_some_and(|(done, _)| *done))
        {
            continue;
        }
        let names: Vec<String> = who
            .iter()
            .map(|k| {
                agents
                    .get(k.as_str())
                    .map(|(_, n)| n.clone())
                    .unwrap_or_else(|| k.clone())
            })
            .collect();
        let mut entry = CleanPath {
            path: path.display().to_string(),
            kind,
            agent: names.join(", "),
            bytes: 0,
            verdict: "kept",
            reason: None,
        };
        let inside = |cwds: &Vec<PathBuf>| cwds.iter().any(|c| holds(&path, c));
        let reason = if !allowed(&path, kind, &home, &cache, &store_root) {
            Some("outside")
        } else if active.iter().any(|p| holds(&path, p))
            || who
                .iter()
                .any(|k| agents.get(k.as_str()).is_some_and(|(done, _)| !*done))
        {
            Some("in_use")
        } else if processes.is_none() {
            Some("unverifiable")
        } else if processes.as_ref().is_some_and(inside) {
            Some("process_inside")
        } else if who.len() > 1 {
            Some("several_owners")
        } else if who.iter().any(|k| !agents.contains_key(k.as_str())) {
            Some("unknown_owner")
        } else if kind == "build_dir" && !looks_built(&path) {
            Some("not_a_build_dir")
        } else if kind == "worktree" {
            // Untracked files listed one by one and submodules looked into,
            // whatever the repository's own status config hides.
            let status = git(
                &path,
                &[
                    "status",
                    "--porcelain",
                    "--untracked-files=all",
                    "--ignore-submodules=none",
                ],
            );
            match (unmerged(&path), status, worktree_inside(&path)) {
                (None, _, _) | (_, None, _) | (_, _, None) => Some("unreadable"),
                (Some(true), _, _) => Some("unmerged"),
                (_, Some(s), _) if !s.trim().is_empty() => Some("uncommitted"),
                (_, _, Some(true)) => Some("nested_checkout"),
                _ => None,
            }
        } else {
            None
        };
        let walk = reason.is_none().then(|| scan(&path, kind == "worktree"));
        let reason = reason.or(match &walk {
            Some(w) if w.nested_git => Some("nested_checkout"),
            Some(w) if w.unreadable => Some("unreadable"),
            _ => None,
        });
        if let Some(r) = reason {
            entry.reason = Some(r.to_string());
            out.push(entry);
            continue;
        }
        entry.bytes = walk.map_or(0, |w| w.bytes);
        entry.verdict = "remove";
        if yes {
            // The repository is found before the folder goes, so its stale
            // worktree entry can be pruned after.
            let common_dir = (kind == "worktree")
                .then(|| {
                    git(
                        &path,
                        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
                    )
                })
                .flatten();
            match std::fs::remove_dir_all(&path) {
                Ok(()) => {
                    entry.verdict = "removed";
                    if let Some(dir) = common_dir {
                        let _ = Command::new("git")
                            .arg("--git-dir")
                            .arg(dir.trim())
                            .args(["worktree", "prune"])
                            .output();
                    }
                }
                Err(e) => {
                    entry.verdict = "failed";
                    entry.reason = Some(e.to_string());
                }
            }
        }
        out.push(entry);
    }

    let removable: u64 = out
        .iter()
        .filter(|c| matches!(c.verdict, "remove" | "removed"))
        .map(|c| c.bytes)
        .sum();
    let removed = out.iter().filter(|c| c.verdict == "removed").count();
    let failed = out.iter().any(|c| c.verdict == "failed");
    if common.json {
        let v = serde_json::json!({
            "dry_run": !yes,
            "paths": out,
            "bytes": removable,
            "removed": removed,
        });
        crate::print_stdout(&serde_json::to_string_pretty(&v).unwrap_or_default());
    } else {
        crate::print_stdout(&render(&out, yes, removable));
    }
    i32::from(failed)
}

fn human_bytes(b: u64) -> String {
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    let b = b as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else {
        format!("{:.0} MB", (b / MB).max(0.0))
    }
}

fn render(out: &[CleanPath], yes: bool, bytes: u64) -> String {
    if out.is_empty() {
        return "Nothing to clean: no finished agent left a build dir or worktree.".into();
    }
    let mut text = String::new();
    for c in out {
        let what = if c.kind == "build_dir" {
            "build dir"
        } else {
            "worktree"
        };
        let line = match (c.verdict, c.reason.as_deref()) {
            ("kept", Some(r)) => format!("  kept     {}  ({what} of {}: {r})", c.path, c.agent),
            ("failed", Some(r)) => format!("  failed   {}  ({r})", c.path),
            (v, _) => format!(
                "  {:8} {}  {}  ({what} of {})",
                v,
                human_bytes(c.bytes),
                c.path,
                c.agent
            ),
        };
        text.push_str(&line);
        text.push('\n');
    }
    if yes {
        text.push_str(&format!("Freed {}.", human_bytes(bytes)));
    } else if bytes > 0 {
        text.push_str(&format!(
            "harbard clean --finished --yes removes the paths marked remove ({}).",
            human_bytes(bytes)
        ));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What lsof escapes without a UTF-8 locale reads back.
    #[test]
    fn lsof_escapes_decode_to_the_bytes_of_the_name() {
        assert_eq!(
            lsof_unescape(r"/w/harbard-wt-pi\xc3\xb1\xc3\xb3n"),
            "/w/harbard-wt-piñón".as_bytes()
        );
        assert_eq!(lsof_unescape(r"/a\tb^Ac"), b"/a\tb\x01c");
        assert_eq!(lsof_unescape(r"/plain/x^y"), b"/plain/x^y");
    }

    #[test]
    fn a_cwd_that_cannot_be_read_back_is_unknown() {
        let dir = std::env::temp_dir().join(format!("harbard-clean-lsof-{}", std::process::id()));
        let named = dir.join("piñón");
        std::fs::create_dir_all(&named).unwrap();
        let escaped = format!("{}/pi\\xc3\\xb1\\xc3\\xb3n", dir.display());
        assert_eq!(
            lsof_path(&escaped),
            Some(std::fs::canonicalize(&named).unwrap())
        );
        // Gone, and escaped: the name is not known.
        assert_eq!(lsof_path("/nowhere/harbard-wt-x\\xc3"), None);
        // Gone, and plain: still exact.
        assert_eq!(
            lsof_path("/nowhere/plain"),
            Some(PathBuf::from("/nowhere/plain"))
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
