//! The sluice: a queue for the right to move a branch.
//!
//! A writer lease answers "who may touch this tree" by making everyone else
//! wait. For a shared working tree that is right; for a fleet it is not, because
//! the tree is not the scarce thing — Git can hand out as many as you like. What
//! is scarce is the right to move `main`, and that is a queue.
//!
//! So an agent stops waiting for a checkout and hands over a commit instead.
//! This module keeps the queue and the receipts; the Git work lives in the CLI,
//! where the process boundary is.
//!
//! The one guarantee everything else exists to protect: **the SHA that lands is
//! the SHA that was checked.** Not a rebuild of it, not a rebase of it, not
//! "the same changes on top of a newer base". If the base moved under a
//! candidate, that candidate is rebuilt and re-checked before anything lands.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

/// One agent's offer: "this commit, onto that branch, if these checks pass".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    pub id: String,
    /// Canonical path of the repository this targets.
    pub repo: String,
    /// The exact commit being offered. Never a branch name — a name can move
    /// between the offer and the integration, and then what landed is not what
    /// was offered.
    pub sha: String,
    /// Branch the delivery wants to move.
    pub branch: String,
    /// The tip the agent built on, kept for diagnosis: a delivery that fails to
    /// merge is far easier to explain when you know how far behind it was.
    pub base: String,
    /// Shell command that decides whether this may land. Runs in the candidate
    /// worktree, never in the agent's.
    pub checks: String,
    /// Optional command that makes the candidate worktree runnable before the
    /// checks touch it.
    ///
    /// A fresh worktree has the repository's tracked files and nothing else —
    /// no `node_modules`, no `target`, no `.venv`. In a workspace monorepo the
    /// internal packages resolve through symlinks inside the root's
    /// `node_modules`, so the very first real delivery died on
    /// `Cannot find module '@…/service-auth'` while the same tests passed 12/12
    /// in the normal checkout. Every delivery running tests or a typecheck
    /// would have hit it.
    ///
    /// The sluice deliberately does not guess the fix: linking the original's
    /// dependencies is instant but checks the candidate against whatever was
    /// already installed, and a real install is faithful but slow and can touch
    /// a lockfile. Only the person delivering knows which trade their repo can
    /// take, so they declare it.
    ///
    /// Its failure is its own verdict, never a rejection: a broken environment
    /// is not a broken commit, and conflating them would reject good work.
    #[serde(default)]
    pub prepare: String,
    pub session: String,
    pub harness: String,
    pub created_ms: u64,
}

/// What happened when the sluice tried to land a delivery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    /// Checks passed on the candidate and the branch now points at it.
    Landed,
    /// The delivery could not be merged onto the current tip.
    Conflict,
    /// The candidate built, the checks ran, and they said no.
    Rejected,
    /// Something outside the delivery's control failed (Git, the worktree, IO).
    Failed,
}

impl Verdict {
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Landed => "landed",
            Verdict::Conflict => "conflict",
            Verdict::Rejected => "rejected",
            Verdict::Failed => "failed",
        }
    }
}

/// The durable answer to "what did this fleet land, and on what evidence".
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Receipt {
    pub delivery_id: String,
    pub repo: String,
    pub branch: String,
    /// What was offered.
    pub delivered_sha: String,
    /// The tip the candidate was actually built on — not the one the agent
    /// claimed, the one that was there at integration time.
    pub onto: String,
    /// The commit the checks ran against, and the only commit that may land.
    /// `None` when the candidate never built (conflict).
    pub candidate: Option<String>,
    pub verdict: Verdict,
    /// Tail of the check output. Enough to explain a rejection without keeping
    /// a build log forever.
    pub detail: String,
    pub session: String,
    pub decided_ms: u64,
}

fn repo_key(repo: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"harbard-sluice-v1\0");
    h.update(repo.as_bytes());
    format!("{:x}", h.finalize())
}

fn dir(home: &Path, repo: &str) -> PathBuf {
    home.join("sluice").join(repo_key(repo))
}

pub fn queue_path(home: &Path, repo: &str) -> PathBuf {
    dir(home, repo).join("queue.jsonl")
}

pub fn receipts_path(home: &Path, repo: &str) -> PathBuf {
    dir(home, repo).join("receipts.jsonl")
}

fn append(path: &Path, line: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    // One write, newline included: a torn line is a lost delivery, and the
    // queue has to survive being killed mid-integration.
    f.write_all(format!("{line}\n").as_bytes())?;
    f.sync_data()
}

/// Queue a delivery. Append-only: two offers from one agent both stand.
pub fn enqueue(home: &Path, delivery: &Delivery) -> std::io::Result<()> {
    let line = serde_json::to_string(delivery)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    append(&queue_path(home, &delivery.repo), &line)
}

/// Every delivery ever offered for this repo, oldest first.
pub fn deliveries(home: &Path, repo: &str) -> Vec<Delivery> {
    read_lines(&queue_path(home, repo))
}

/// Every decision ever taken for this repo, oldest first.
pub fn receipts(home: &Path, repo: &str) -> Vec<Receipt> {
    read_lines(&receipts_path(home, repo))
}

fn read_lines<T: for<'de> Deserialize<'de>>(path: &Path) -> Vec<T> {
    let Ok(raw) = fs::read_to_string(path) else {
        return Vec::new();
    };
    raw.lines()
        // A corrupt line is skipped, never fatal: one bad record must not make
        // the whole queue unreadable, because the queue is how work gets in.
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Deliveries with no receipt yet, oldest first — what integrate should work on.
///
/// Derived rather than stored: a "pending" flag would need a write per state
/// change, and every one of those writes is a chance to be killed holding a
/// half-truth. Receipts are the only state, and pending is what they do not
/// mention yet.
pub fn pending(home: &Path, repo: &str) -> Vec<Delivery> {
    let decided: std::collections::HashSet<String> = receipts(home, repo)
        .into_iter()
        .map(|r| r.delivery_id)
        .collect();
    deliveries(home, repo)
        .into_iter()
        .filter(|d| !decided.contains(&d.id))
        .collect()
}

/// Record a decision. Written once, after the branch has (or has not) moved.
pub fn record(home: &Path, receipt: &Receipt) -> std::io::Result<()> {
    let line = serde_json::to_string(receipt)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    append(&receipts_path(home, &receipt.repo), &line)
}

/// The newest receipt per repository, from one pass over the sluice directory.
///
/// The per-repo [`receipts`] reader needs the repo path to hash it into a
/// directory name, so asking it "which trees had a delivery?" is impossible
/// and asking it once per candidate checkout is a file read per card. The
/// receipt carries its own `repo`, so one walk answers both.
pub fn latest_receipt_by_repo(home: &Path) -> std::collections::HashMap<String, Receipt> {
    let mut out: std::collections::HashMap<String, Receipt> = std::collections::HashMap::new();
    let Ok(entries) = fs::read_dir(home.join("sluice")) else {
        return out;
    };
    for entry in entries.flatten() {
        for receipt in read_lines::<Receipt>(&entry.path().join("receipts.jsonl")) {
            match out.get(&receipt.repo) {
                Some(seen) if seen.decided_ms >= receipt.decided_ms => {}
                _ => {
                    out.insert(receipt.repo.clone(), receipt);
                }
            }
        }
    }
    out
}

/// Every delivery and every receipt across all repositories, oldest first
/// per repository, from one pass over the sluice directory.
pub fn all(home: &Path) -> (Vec<Delivery>, Vec<Receipt>) {
    let mut deliveries = Vec::new();
    let mut receipts = Vec::new();
    let Ok(entries) = fs::read_dir(home.join("sluice")) else {
        return (deliveries, receipts);
    };
    let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    dirs.sort();
    for dir in dirs {
        deliveries.extend(read_lines::<Delivery>(&dir.join("queue.jsonl")));
        receipts.extend(read_lines::<Receipt>(&dir.join("receipts.jsonl")));
    }
    (deliveries, receipts)
}

/// Keep the last `max` characters of check output, on a char boundary.
pub fn tail(text: &str, max: usize) -> String {
    let trimmed = text.trim_end();
    if trimmed.chars().count() <= max {
        return trimmed.to_string();
    }
    let skip = trimmed.chars().count() - max;
    trimmed.chars().skip(skip).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempHome;

    fn delivery(id: &str, sha: &str) -> Delivery {
        Delivery {
            id: id.into(),
            repo: "/repo".into(),
            sha: sha.into(),
            branch: "main".into(),
            base: "0000000".into(),
            checks: "true".into(),
            prepare: String::new(),
            session: "s1".into(),
            harness: "codex".into(),
            created_ms: 1_000,
        }
    }

    #[test]
    fn pending_is_derived_from_receipts_not_from_a_flag() {
        let home = TempHome::new();
        enqueue(home.path(), &delivery("d1", "aaa")).unwrap();
        enqueue(home.path(), &delivery("d2", "bbb")).unwrap();
        assert_eq!(pending(home.path(), "/repo").len(), 2);

        record(
            home.path(),
            &Receipt {
                delivery_id: "d1".into(),
                repo: "/repo".into(),
                branch: "main".into(),
                delivered_sha: "aaa".into(),
                onto: "ccc".into(),
                candidate: Some("ddd".into()),
                verdict: Verdict::Landed,
                detail: String::new(),
                session: "s1".into(),
                decided_ms: 2_000,
            },
        )
        .unwrap();

        let left = pending(home.path(), "/repo");
        assert_eq!(left.len(), 1);
        assert_eq!(left[0].id, "d2", "a decided delivery must not come back");
    }

    #[test]
    fn a_torn_line_costs_one_delivery_not_the_queue() {
        let home = TempHome::new();
        enqueue(home.path(), &delivery("d1", "aaa")).unwrap();
        let path = queue_path(home.path(), "/repo");
        let mut raw = fs::read_to_string(&path).unwrap();
        raw.push_str("{\"id\":\"d2\",\"repo\":\"/rep\n");
        fs::write(&path, raw).unwrap();
        enqueue(home.path(), &delivery("d3", "ccc")).unwrap();

        let all = deliveries(home.path(), "/repo");
        assert_eq!(all.len(), 2, "the good records still read");
        assert_eq!(all[1].id, "d3");
    }

    #[test]
    fn two_repos_never_share_a_queue() {
        let home = TempHome::new();
        enqueue(home.path(), &delivery("d1", "aaa")).unwrap();
        let mut other = delivery("d2", "bbb");
        other.repo = "/other".into();
        enqueue(home.path(), &other).unwrap();

        assert_eq!(deliveries(home.path(), "/repo").len(), 1);
        assert_eq!(deliveries(home.path(), "/other").len(), 1);
    }

    #[test]
    fn all_reads_every_repository_in_one_pass() {
        let home = TempHome::new();
        assert_eq!(all(home.path()), (Vec::new(), Vec::new()));
        enqueue(home.path(), &delivery("d1", "aaa")).unwrap();
        let mut other = delivery("d2", "bbb");
        other.repo = "/other".into();
        enqueue(home.path(), &other).unwrap();
        record(
            home.path(),
            &Receipt {
                delivery_id: "d2".into(),
                repo: "/other".into(),
                branch: "main".into(),
                delivered_sha: "bbb".into(),
                onto: "ccc".into(),
                candidate: Some("bbb".into()),
                verdict: Verdict::Landed,
                detail: String::new(),
                session: "s1".into(),
                decided_ms: 2_000,
            },
        )
        .unwrap();
        let (deliveries, receipts) = all(home.path());
        let mut ids: Vec<&str> = deliveries.iter().map(|d| d.id.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["d1", "d2"]);
        assert_eq!(receipts.len(), 1);
        assert_eq!(receipts[0].delivery_id, "d2");
    }

    #[test]
    fn tail_keeps_the_end_and_never_splits_a_character() {
        assert_eq!(tail("abc", 10), "abc");
        assert_eq!(tail("abcdef", 3), "def");
        // Multi-byte: slicing by bytes here would panic.
        assert_eq!(tail("áéíóú", 2), "óú");
    }
}
