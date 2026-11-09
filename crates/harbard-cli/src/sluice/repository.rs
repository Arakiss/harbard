//! Repository-wide queue identity, retaining read access to older checkout queues.

use super::git;
use crate::context;
use harbard_core::sluice::{self, Delivery, Receipt};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

pub(super) struct Repository {
    pub identity: String,
    pub root: PathBuf,
    pub common_dir: PathBuf,
    worktrees: Vec<(PathBuf, Option<String>)>,
}

impl Repository {
    pub fn resolve(path: &str) -> Result<Self, String> {
        let top = git(Path::new(path), &["rev-parse", "--show-toplevel"])?;
        let identity = context::canonical_repo_recipient(Path::new(&top));
        let root = PathBuf::from(&identity);
        let common_dir =
            context::git_common_dir(&root).ok_or("cannot resolve the shared Git directory")?;
        // NUL separation preserves paths containing spaces or newlines.
        let listing = git(&root, &["worktree", "list", "--porcelain", "-z"])?;
        let mut worktrees = Vec::new();
        let mut current = None;
        for field in listing.split('\0') {
            if let Some(path) = field.strip_prefix("worktree ") {
                worktrees.push((PathBuf::from(path), None));
                current = Some(worktrees.len() - 1);
            } else if let Some(branch) = field.strip_prefix("branch refs/heads/")
                && let Some(index) = current
            {
                worktrees[index].1 = Some(branch.to_string());
            }
        }
        Ok(Self {
            identity,
            root,
            common_dir,
            worktrees,
        })
    }

    pub fn target_checkout(&self, branch: &str) -> Result<Option<PathBuf>, String> {
        let targets: Vec<_> = self
            .worktrees
            .iter()
            .filter(|(_, checked)| checked.as_deref() == Some(branch))
            .map(|(path, _)| path.clone())
            .collect();
        match targets.as_slice() {
            [] => Ok(None),
            [target] => Ok(Some(target.clone())),
            _ => Err(format!(
                "{branch} is checked out in multiple worktrees; refusing to choose one"
            )),
        }
    }

    fn queue_paths(&self) -> Vec<String> {
        let mut paths = vec![self.identity.clone()];
        paths.extend(
            self.worktrees
                .iter()
                .map(|(path, _)| path.display().to_string()),
        );
        let mut seen = HashSet::new();
        paths.retain(|path| seen.insert(path.clone()));
        paths
    }

    pub fn receipts(&self, home: &Path) -> Vec<Receipt> {
        let mut latest: HashMap<String, Receipt> = HashMap::new();
        for path in self.queue_paths() {
            for mut receipt in sluice::receipts(home, &path) {
                receipt.repo.clone_from(&self.identity);
                if latest
                    .get(&receipt.delivery_id)
                    .is_none_or(|old| old.decided_ms <= receipt.decided_ms)
                {
                    latest.insert(receipt.delivery_id.clone(), receipt);
                }
            }
        }
        let mut receipts: Vec<_> = latest.into_values().collect();
        receipts
            .sort_by(|a, b| (a.decided_ms, &a.delivery_id).cmp(&(b.decided_ms, &b.delivery_id)));
        receipts
    }

    pub fn pending(&self, home: &Path) -> Vec<Delivery> {
        let decided: HashSet<_> = self
            .receipts(home)
            .into_iter()
            .map(|r| r.delivery_id)
            .collect();
        let mut seen = HashSet::new();
        let mut pending = Vec::new();
        for path in self.queue_paths() {
            for mut delivery in sluice::deliveries(home, &path) {
                if !decided.contains(&delivery.id) && seen.insert(delivery.id.clone()) {
                    delivery.repo.clone_from(&self.identity);
                    pending.push(delivery);
                }
            }
        }
        pending.sort_by(|a, b| (a.created_ms, &a.id).cmp(&(b.created_ms, &b.id)));
        pending
    }
}
