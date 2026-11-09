//! `harbard checkout-policy` — read-only classification of a shell command
//! against the checkout lifecycle.
//!
//! Deliberately separate from `gate`: this verb takes nothing, records
//! nothing, and touches no lease. It answers one question about one string, so
//! it can be called before the gate without any risk of a side effect on a
//! command that is about to be denied anyway.
//!
//! What it does NOT decide is the escape hatch. The operator's one-shot
//! branch-switch approval lives in the operator's own directory, is created
//! only by the operator, and is consumed by the hook. A library that could
//! waive its own denials would be a library with a back door.

use crate::cli::Common;
use crate::context::Context;
use harbard_core::lifecycle::{self, Hazard, Policy};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Exit codes:
/// - `0` — no hazard
/// - `2` — a hazard, or a command that could not be read (fail closed)
/// - `1` — the repository could not be inspected, so nothing was classified
pub fn cmd_checkout_policy(common: &Common, command: &str) -> i32 {
    let here = common
        .cwd
        .as_deref()
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    let Some(checkout) = git_line(&here, &["rev-parse", "--show-toplevel"]).map(PathBuf::from)
    else {
        return infrastructure_failure(
            common,
            "The session checkout could not be resolved; nothing was classified.",
        );
    };
    let Some(all) = registered_worktrees(&checkout) else {
        return infrastructure_failure(
            common,
            "Registered worktrees could not be inspected; nothing was classified.",
        );
    };
    // A lane opened for this session is not another checkout — it is the lane
    // it was given. Without this, `worktree-lease.sh create` produced a
    // worktree its own creator could not enter, could not work in, and could
    // not release, which is how sessions ended up with verified work stranded
    // in the primary tree and an orphan lane nobody could clean up.
    let session = Context::resolve(common).session;
    let worktrees: Vec<PathBuf> = all
        .into_iter()
        .filter(|root| !lane_is_open_for(root, &session))
        .collect();
    let aliases = git_lines(&checkout, &["--list-cmds=alias"]).unwrap_or_default();

    let policy = Policy::new(Some(&checkout), &here, &worktrees, &aliases);
    match lifecycle::classify(command, &policy) {
        Ok(None) => {
            emit(common, None, "clear");
            0
        }
        Ok(Some(hazard)) => {
            emit(common, Some(hazard), hazard.reason());
            2
        }
        Err(_) => {
            emit(
                common,
                None,
                "The command could not be parsed well enough to clear it. Rewrite it as separate, \
                 quoted commands.",
            );
            2
        }
    }
}

fn emit(common: &Common, hazard: Option<Hazard>, reason: &str) {
    if common.json {
        println!(
            "{}",
            json!({
                "hazard": hazard.map(Hazard::as_str),
                "readable": !(hazard.is_none() && reason != "clear"),
                "reason": reason,
            })
        );
        return;
    }
    match hazard {
        Some(hazard) => println!("{}: {reason}", hazard.as_str()),
        None if reason == "clear" => println!("clear"),
        None => println!("unreadable: {reason}"),
    }
}

fn infrastructure_failure(common: &Common, reason: &str) -> i32 {
    if common.json {
        println!(
            "{}",
            json!({"hazard": null, "readable": false, "reason": reason})
        );
    } else {
        eprintln!("{reason}");
    }
    1
}

/// Every worktree registered for this repository. `list --porcelain` is the
/// stable machine form; the human form has been reformatted before.
fn registered_worktrees(checkout: &Path) -> Option<Vec<PathBuf>> {
    let lines = git_lines(checkout, &["worktree", "list", "--porcelain"])?;
    let roots: Vec<PathBuf> = lines
        .iter()
        .filter_map(|line| line.strip_prefix("worktree "))
        .map(|path| {
            let path = PathBuf::from(path);
            std::fs::canonicalize(&path).unwrap_or(path)
        })
        .collect();
    // A repository always reports at least its own worktree. An empty list
    // means the inspection failed in a way `git` did not report as an error.
    (!roots.is_empty()).then_some(roots)
}

/// Whether this worktree's lifecycle receipt names `session` as its live
/// owner.
///
/// The receipt is the same evidence the governed lifecycle already trusts to
/// decide lane ownership, written by `worktree-lease.sh create` and by nothing
/// else — an agent cannot forge one without first passing this very gate to
/// write inside the lane. Both fields are required: a released lane keeps its
/// session line, and treating that as ownership would reopen a closed door.
fn lane_is_open_for(worktree: &Path, session: &str) -> bool {
    if session.is_empty() {
        return false;
    }
    let Ok(receipt) = std::fs::read_to_string(worktree.join(".WORKTREE-OWNER.md")) else {
        return false;
    };
    field(&receipt, "Session").is_some_and(|owner| owner == session)
        && field(&receipt, "State").is_some_and(|state| state == "active")
}

/// Read one `| Name | `value` |` row out of the receipt table.
fn field<'a>(receipt: &'a str, name: &str) -> Option<&'a str> {
    receipt
        .lines()
        .find_map(|line| {
            let mut cells = line.split('|').map(str::trim);
            cells.next()?;
            (cells.next()? == name).then(|| cells.next())?
        })
        .map(|value| value.trim_matches('`').trim())
}

fn git_line(cwd: &Path, args: &[&str]) -> Option<String> {
    git_lines(cwd, args)?.into_iter().next()
}

fn git_lines(cwd: &Path, args: &[&str]) -> Option<Vec<String>> {
    let output = Command::new("git")
        .arg("-C")
        .arg(cwd)
        .args(args)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(|line| line.trim_end().to_string())
            .filter(|line| !line.is_empty())
            .collect(),
    )
}
