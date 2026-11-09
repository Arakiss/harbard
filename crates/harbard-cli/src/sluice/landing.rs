//! Preserve target checkout ownership and local files during a landing.

use super::{Repository, git, ownership};
use crate::context::Context;
use harbard_core::Store;
use std::path::Path;

/// Synchronize the actual target checkout without discarding local files.
pub(super) fn land_candidate(
    repository: &Repository,
    store: &Store,
    ctx: &Context,
    branch: &str,
    tip: &str,
    candidate: &str,
) -> Result<(), String> {
    // Checks can take minutes: refresh which checkout owns the target branch.
    let current = Repository::resolve(&repository.identity)?;
    let target = current.target_checkout(branch)?;
    if let Some(checkout) = target.as_deref() {
        ownership::with_writer(repository, store, ctx, checkout, || {
            advance_target(
                repository.root.as_path(),
                Some(checkout),
                branch,
                tip,
                candidate,
            )
        })
    } else {
        advance_target(repository.root.as_path(), None, branch, tip, candidate)
    }
}

fn advance_target(
    repo: &Path,
    target: Option<&Path>,
    branch: &str,
    tip: &str,
    candidate: &str,
) -> Result<(), String> {
    if let Some(checkout) = target {
        let status = git(checkout, &["status", "--porcelain", "--untracked-files=no"])?;
        if !status.is_empty() {
            return Err(format!(
                "{branch} is checked out with uncommitted changes; not moving its working tree"
            ));
        }
        preserve_local_files(checkout, candidate)?;
        // The two-tree merge also refuses tracked edits made after status.
        git(
            checkout,
            &["read-tree", "--dry-run", "-m", "-u", tip, candidate],
        )
        .map_err(|error| format!("target checkout cannot safely advance: {error}"))?;
    }
    let reference = format!("refs/heads/{branch}");
    git(repo, &["update-ref", &reference, candidate, tip])?;
    if let Some(checkout) = target {
        // Repeat immediately before updating files: checks and unrelated
        // processes may have created local files since the preflight.
        if let Err(error) = preserve_local_files(checkout, candidate) {
            git(repo, &["update-ref", &reference, tip, candidate])
                .map_err(|rollback| format!("{error}; branch rollback failed: {rollback}"))?;
            return Err(error);
        }
        if let Err(error) = git(checkout, &["read-tree", "-m", "-u", tip, candidate]) {
            // Fence rollback too: never overwrite a third party's newer tip.
            return match git(repo, &["update-ref", &reference, tip, candidate]) {
                Ok(_) => Err(format!("checkout update failed; branch restored: {error}")),
                Err(rollback) => Err(format!(
                    "checkout update failed: {error}; branch rollback also failed: {rollback}; inspect the checkout before continuing"
                )),
            };
        }
    }
    Ok(())
}

fn preserve_local_files(checkout: &Path, candidate: &str) -> Result<(), String> {
    // No --exclude-standard: ignored files are local work too. --directory
    // keeps a dependency directory to one entry rather than walking its files.
    let local = git(
        checkout,
        &[
            "ls-files",
            "--others",
            "--directory",
            "--no-empty-directory",
            "-z",
        ],
    )?;
    if local.is_empty() {
        return Ok(());
    }
    let tracked = git(checkout, &["ls-tree", "-r", "--name-only", "-z", candidate])?;
    for file in local.split('\0').filter(|path| !path.is_empty()) {
        let file = file.trim_end_matches('/');
        for target in tracked.split('\0').filter(|path| !path.is_empty()) {
            if file == target
                || file
                    .strip_prefix(target)
                    .is_some_and(|rest| rest.starts_with('/'))
                || target
                    .strip_prefix(file)
                    .is_some_and(|rest| rest.starts_with('/'))
            {
                return Err(format!(
                    "target checkout contains a local file or directory that the candidate would overwrite: {file}"
                ));
            }
        }
    }
    Ok(())
}
