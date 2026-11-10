//! Repo-scoped turns on shared resources ("lanes"): one owner, a FIFO of
//! waiters, a ttl the owner keeps alive with heartbeats, and an optional
//! release tied to pull requests settling.
//!
//! Why this exists: sessions that share a migration ordinal, a merge queue, a
//! local stack or environment deliveries through loose notes name no verifiable
//! owner, nothing expires, nothing announces the release, and waiters end up
//! polling. A lane is that note turned into state the bus can enforce and expire.
//!
//! Storage follows [`crate::lease`]: one private file per lane, mutated only
//! under an exclusive lock, written by temp file + rename, and corrupt state is
//! refused rather than overwritten. Unlike a lease, a lane is bound to a
//! session and a clock rather than to a process: a turn on a migration ordinal
//! outlives any one tool shell, so the owner is whoever took it and keeps
//! heartbeating.
//!
//! Every mutation first *settles* the lane — an expired owner, or one whose
//! `release_on` pull requests have all settled, passes the lane to the next
//! waiter — and returns the [`Handover`]s it caused. The caller turns those
//! into notes. Settling happens under the lane lock, so each handover is
//! reported by exactly one caller however many race on it.

use crate::lease::{
    ensure_private_directory, open_private_lock, open_private_new, open_private_read,
    sync_directory, validate_private_regular,
};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// Nobody holds a shared resource for more than a working day without a
/// heartbeat; a longer ttl is a dead reservation waiting to happen.
pub const MAX_LANE_TTL_SECS: u64 = 24 * 60 * 60;
/// How long a promoted waiter has to show a sign of life before its turn
/// starts without it and the lane passes on.
///
/// A waiter is promoted while it may be asleep, and starting the ttl at
/// promotion would spend the turn on a session that cannot use it. So the ttl starts at the
/// waiter's first sign of life, and this bounds how long the lane waits for
/// one. Fifteen minutes is the stale window the board already uses: a session
/// woken by the promotion warning answers in seconds, one busy inside a long
/// tool call (ten minutes at most on Claude Code) still makes it, and one that
/// is not coming stops blocking the queue behind it.
pub const PROMOTION_GRACE_SECS: u64 = 15 * 60;
/// Far above the five sessions of the incident; a bound, not a target.
pub const MAX_QUEUE: usize = 32;
pub const MAX_RELEASE_ON: usize = 16;
const MAX_PURPOSE_BYTES: usize = 200;
const LANE_SCHEMA_VERSION: u8 = 1;
const MAX_LANE_BYTES: u64 = 256 * 1024;
const PR_STATE_FILE: &str = "prs.json";

static PROCESS_LANE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneHolder {
    pub session: String,
    pub purpose: String,
    pub ttl_secs: u64,
    pub taken_at_ms: u64,
    pub renewed_at_ms: u64,
    pub expires_at_ms: u64,
    /// Pull requests whose merge-or-close releases the lane on its own.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub release_on: Vec<u64>,
    /// Set while a promoted waiter has not yet shown a sign of life: the
    /// deadline ([`PROMOTION_GRACE_SECS`]) by which it must, and meanwhile its
    /// `expires_at_ms`. Cleared, and the ttl started, by [`LaneStore::claim`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_by_ms: Option<u64>,
}

impl LaneHolder {
    /// Start the turn's ttl now: the owner is demonstrably awake.
    fn renew(&mut self, now_ms: u64) {
        self.renewed_at_ms = now_ms;
        self.expires_at_ms = expiry(now_ms, self.ttl_secs);
        self.claim_by_ms = None;
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneWaiter {
    pub session: String,
    pub purpose: String,
    /// The ttl this waiter asked for; its clock starts when its turn does.
    pub ttl_secs: u64,
    pub joined_at_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub release_on: Vec<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneRecord {
    pub v: u8,
    /// Repository identity shared by every worktree (the primary root).
    pub repo: String,
    pub lane: String,
    pub owner: Option<LaneHolder>,
    #[serde(default)]
    pub queue: Vec<LaneWaiter>,
}

impl LaneRecord {
    fn empty(repo: &str, lane: &str) -> Self {
        Self {
            v: LANE_SCHEMA_VERSION,
            repo: repo.to_string(),
            lane: lane.to_string(),
            owner: None,
            queue: Vec::new(),
        }
    }

    /// 1-based place in the queue, the number a waiter is told.
    pub fn position_of(&self, session: &str) -> Option<usize> {
        self.queue
            .iter()
            .position(|waiter| waiter.session == session)
            .map(|index| index + 1)
    }

    pub fn owned_by(&self, session: &str) -> bool {
        self.owner
            .as_ref()
            .is_some_and(|owner| owner.session == session)
    }
}

/// What a session asks for when it takes or queues for a lane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneRequest<'a> {
    pub session: &'a str,
    pub purpose: &'a str,
    pub ttl_secs: u64,
    pub release_on: &'a [u64],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HandoverReason {
    /// The owner let go.
    Released,
    /// The owner stopped heartbeating.
    Expired,
    /// Every pull request the owner tied the lane to merged or closed.
    PrsSettled,
    /// The owner's session ended (`bye`).
    SessionEnded,
    /// The waiter it was promoted to showed no sign of life within
    /// [`PROMOTION_GRACE_SECS`].
    Unclaimed,
}

impl HandoverReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Released => "released",
            Self::Expired => "expired",
            Self::PrsSettled => "prs_settled",
            Self::SessionEnded => "session_ended",
            Self::Unclaimed => "unclaimed",
        }
    }
}

/// One change of owner. `to: None` means the lane is now free.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Handover {
    pub repo: String,
    pub lane: String,
    pub reason: HandoverReason,
    pub from: Option<String>,
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub prs: Vec<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum TakeOutcome {
    Acquired,
    /// Already the owner: the ttl restarts.
    Held,
    Queued {
        position: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneTake {
    pub outcome: TakeOutcome,
    pub record: LaneRecord,
    pub handovers: Vec<Handover>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseOutcome {
    Released,
    LeftQueue,
    NotHeld,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneRelease {
    pub outcome: ReleaseOutcome,
    pub record: Option<LaneRecord>,
    pub handovers: Vec<Handover>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HeartbeatOutcome {
    Renewed,
    NotOwner,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaneHeartbeat {
    pub outcome: HeartbeatOutcome,
    pub record: Option<LaneRecord>,
    pub handovers: Vec<Handover>,
}

#[derive(Debug)]
pub enum LaneError {
    Io(std::io::Error),
    CorruptState,
    InvalidLane(&'static str),
    InvalidRequest(&'static str),
    QueueFull,
}

impl fmt::Display for LaneError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "lane storage error: {error}"),
            Self::CorruptState => f.write_str("corrupt lane state; refusing to mutate"),
            Self::InvalidLane(message) | Self::InvalidRequest(message) => f.write_str(message),
            Self::QueueFull => write!(f, "lane queue is full ({MAX_QUEUE} waiters)"),
        }
    }
}

impl std::error::Error for LaneError {}

impl From<std::io::Error> for LaneError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

/// Which pull requests have settled (merged or closed). The lane store never
/// asks GitHub itself: it reads whatever the one shared watcher last recorded.
pub trait PrFacts {
    fn settled(&self, repo: &str, number: u64) -> bool;
}

/// For callers with no PR knowledge at hand; `release_on` simply never fires.
pub struct NoPrFacts;

impl PrFacts for NoPrFacts {
    fn settled(&self, _repo: &str, _number: u64) -> bool {
        false
    }
}

#[derive(Debug, Clone)]
pub struct LaneStore {
    home: PathBuf,
    root: PathBuf,
}

struct LanePaths {
    directory: PathBuf,
    state: PathBuf,
    lock: PathBuf,
    hash: String,
}

impl LaneStore {
    pub fn at(harbard_home: impl AsRef<Path>) -> Self {
        let home = harbard_home.as_ref().to_path_buf();
        Self {
            root: home.join("lanes").join("v1"),
            home,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Take the lane, or join its queue when someone else holds it.
    pub fn take(
        &self,
        repo: &str,
        lane: &str,
        request: &LaneRequest<'_>,
        prs: &dyn PrFacts,
        now_ms: u64,
    ) -> Result<LaneTake, LaneError> {
        validate_request(request)?;
        self.with_lane(repo, lane, |slot| {
            let record = slot.get_or_insert_with(|| LaneRecord::empty(repo, lane));
            let handovers = settle(record, prs, now_ms);
            let promoted_to_caller = handovers
                .iter()
                .any(|handover| handover.to.as_deref() == Some(request.session));
            let outcome = if record.owner.is_none() {
                record.owner = Some(holder_from(request, now_ms));
                TakeOutcome::Acquired
            } else if let Some(owner) = record
                .owner
                .as_mut()
                .filter(|owner| owner.session == request.session)
            {
                owner.purpose = request.purpose.trim().to_string();
                owner.ttl_secs = request.ttl_secs;
                if !request.release_on.is_empty() {
                    owner.release_on = request.release_on.to_vec();
                }
                owner.renew(now_ms);
                // Settling may have just handed the lane to this caller: from
                // where they stand that is an acquisition, not a renewal.
                if promoted_to_caller {
                    TakeOutcome::Acquired
                } else {
                    TakeOutcome::Held
                }
            } else if let Some(index) = record
                .queue
                .iter()
                .position(|waiter| waiter.session == request.session)
            {
                let waiter = &mut record.queue[index];
                waiter.purpose = request.purpose.trim().to_string();
                waiter.ttl_secs = request.ttl_secs;
                waiter.release_on = request.release_on.to_vec();
                TakeOutcome::Queued {
                    position: index + 1,
                }
            } else {
                if record.queue.len() >= MAX_QUEUE {
                    return Err(LaneError::QueueFull);
                }
                record.queue.push(LaneWaiter {
                    session: request.session.to_string(),
                    purpose: request.purpose.trim().to_string(),
                    ttl_secs: request.ttl_secs,
                    joined_at_ms: now_ms,
                    release_on: request.release_on.to_vec(),
                });
                TakeOutcome::Queued {
                    position: record.queue.len(),
                }
            };
            Ok(LaneTake {
                outcome,
                record: record.clone(),
                handovers,
            })
        })
    }

    /// Let go as owner (the next waiter takes over) or leave the queue.
    pub fn release(
        &self,
        repo: &str,
        lane: &str,
        session: &str,
        prs: &dyn PrFacts,
        now_ms: u64,
    ) -> Result<LaneRelease, LaneError> {
        self.release_as(repo, lane, session, HandoverReason::Released, prs, now_ms)
    }

    fn release_as(
        &self,
        repo: &str,
        lane: &str,
        session: &str,
        reason: HandoverReason,
        prs: &dyn PrFacts,
        now_ms: u64,
    ) -> Result<LaneRelease, LaneError> {
        validate_session(session)?;
        self.with_lane(repo, lane, |slot| {
            let Some(record) = slot.as_mut() else {
                return Ok(LaneRelease {
                    outcome: ReleaseOutcome::NotHeld,
                    record: None,
                    handovers: Vec::new(),
                });
            };
            let mut handovers = settle(record, prs, now_ms);
            let outcome = if record.owned_by(session) {
                record.owner = None;
                promote(record, now_ms);
                handovers.push(Handover {
                    repo: record.repo.clone(),
                    lane: record.lane.clone(),
                    reason,
                    from: Some(session.to_string()),
                    to: record.owner.as_ref().map(|owner| owner.session.clone()),
                    prs: Vec::new(),
                });
                // The new owner's own release_on may already be satisfied.
                handovers.extend(settle(record, prs, now_ms));
                ReleaseOutcome::Released
            } else if let Some(position) = record.position_of(session) {
                record.queue.remove(position - 1);
                ReleaseOutcome::LeftQueue
            } else {
                ReleaseOutcome::NotHeld
            };
            Ok(LaneRelease {
                outcome,
                record: Some(record.clone()),
                handovers,
            })
        })
    }

    /// Restart the owner's ttl. A caller that is not the owner — including one
    /// whose turn already expired and passed on — learns so instead.
    pub fn heartbeat(
        &self,
        repo: &str,
        lane: &str,
        session: &str,
        prs: &dyn PrFacts,
        now_ms: u64,
    ) -> Result<LaneHeartbeat, LaneError> {
        validate_session(session)?;
        self.with_lane(repo, lane, |slot| {
            let Some(record) = slot.as_mut() else {
                return Ok(LaneHeartbeat {
                    outcome: HeartbeatOutcome::NotOwner,
                    record: None,
                    handovers: Vec::new(),
                });
            };
            let handovers = settle(record, prs, now_ms);
            let outcome = match record.owner.as_mut() {
                Some(owner) if owner.session == session => {
                    owner.renew(now_ms);
                    HeartbeatOutcome::Renewed
                }
                _ => HeartbeatOutcome::NotOwner,
            };
            Ok(LaneHeartbeat {
                outcome,
                record: Some(record.clone()),
                handovers,
            })
        })
    }

    /// A sign of life from `session`: every lane promoted to it and not yet
    /// claimed starts its ttl now. Called on any event the session writes and
    /// on every hook turn, so a waiter promoted while asleep keeps its whole
    /// turn for when it wakes. Lock-free scan first: this sits on the hook path
    /// and almost every call finds nothing to do.
    pub fn claim(&self, session: &str, prs: &dyn PrFacts, now_ms: u64) -> Vec<Handover> {
        let mut handovers = Vec::new();
        for record in self.list(None) {
            let unclaimed = record
                .owner
                .as_ref()
                .is_some_and(|owner| owner.session == session && owner.claim_by_ms.is_some());
            if !unclaimed {
                continue;
            }
            let claimed = self.with_lane(&record.repo, &record.lane, |slot| {
                let Some(record) = slot.as_mut() else {
                    return Ok(Vec::new());
                };
                // Settled first: a grace period that already ran out is not
                // revived by a session that came back too late.
                let settled = settle(record, prs, now_ms);
                if let Some(owner) = record
                    .owner
                    .as_mut()
                    .filter(|owner| owner.session == session && owner.claim_by_ms.is_some())
                {
                    owner.renew(now_ms);
                }
                Ok(settled)
            });
            if let Ok(mut settled) = claimed {
                handovers.append(&mut settled);
            }
        }
        handovers
    }

    /// The settled state of one lane. `None` when nobody holds or waits on it.
    pub fn show(
        &self,
        repo: &str,
        lane: &str,
        prs: &dyn PrFacts,
        now_ms: u64,
    ) -> Result<(Option<LaneRecord>, Vec<Handover>), LaneError> {
        self.with_lane(repo, lane, |slot| {
            let Some(record) = slot.as_mut() else {
                return Ok((None, Vec::new()));
            };
            let handovers = settle(record, prs, now_ms);
            let view = (record.owner.is_some() || !record.queue.is_empty()).then(|| record.clone());
            Ok((view, handovers))
        })
    }

    /// Every lane on disk, optionally for one repository, as last written.
    ///
    /// Read-only and lock-free, like [`crate::LeaseStore::active_writers`]:
    /// writes land by rename, so no torn record is observable, but an owner
    /// shown here may already have expired. Informational surfaces (brief,
    /// `ls`) read this; anything that decides goes through a settling verb.
    pub fn list(&self, repo: Option<&str>) -> Vec<LaneRecord> {
        let Ok(entries) = fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut records = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(stem) = path
                .file_name()
                .and_then(|name| name.to_str())
                .and_then(|name| name.strip_suffix(".json"))
            else {
                continue;
            };
            if format!("{stem}.json") == PR_STATE_FILE {
                continue;
            }
            let Ok(Some(record)) = read_lane(&path, None) else {
                continue;
            };
            if lane_hash(&record.repo, &record.lane) != stem {
                continue;
            }
            if repo.is_none_or(|repo| repo == record.repo) {
                records.push(record);
            }
        }
        records.sort_by(|a, b| a.repo.cmp(&b.repo).then_with(|| a.lane.cmp(&b.lane)));
        records
    }

    /// Settle every lane: the watcher's pass, so a lane nobody touches still
    /// expires and still releases when its pull requests settle.
    pub fn sweep(&self, prs: &dyn PrFacts, now_ms: u64) -> Vec<Handover> {
        let mut handovers = Vec::new();
        for record in self.list(None) {
            if let Ok((_, mut settled)) = self.show(&record.repo, &record.lane, prs, now_ms) {
                handovers.append(&mut settled);
            }
        }
        handovers
    }

    /// The session ended: release what it owns and drop it from every queue,
    /// so the next waiter is not left behind a ttl nobody will renew.
    pub fn release_session(&self, session: &str, prs: &dyn PrFacts, now_ms: u64) -> Vec<Handover> {
        let mut handovers = Vec::new();
        for record in self.list(None) {
            let involved = record.owned_by(session) || record.position_of(session).is_some();
            if !involved {
                continue;
            }
            if let Ok(mut released) = self.release_as(
                &record.repo,
                &record.lane,
                session,
                HandoverReason::SessionEnded,
                prs,
                now_ms,
            ) {
                handovers.append(&mut released.handovers);
            }
        }
        handovers
    }

    fn with_lane<T>(
        &self,
        repo: &str,
        lane: &str,
        operation: impl FnOnce(&mut Option<LaneRecord>) -> Result<T, LaneError>,
    ) -> Result<T, LaneError> {
        validate_repo(repo)?;
        validate_lane_name(lane)?;
        let _process_guard = PROCESS_LANE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        ensure_private_directory(&self.home)?;
        ensure_private_directory(self.root.parent().expect("lane root has parent"))?;
        ensure_private_directory(&self.root)?;
        let paths = self.paths(repo, lane);
        let lock = open_private_lock(&paths.lock)?;
        lock.lock_exclusive()?;
        let result = (|| {
            let before = read_lane(&paths.state, Some((repo, lane)))?;
            let mut current = before.clone();
            let value = operation(&mut current)?;
            if current != before {
                persist(&paths, current.as_ref())?;
            }
            Ok(value)
        })();
        let unlock = FileExt::unlock(&lock);
        match (result, unlock) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(LaneError::Io(error)),
        }
    }

    fn paths(&self, repo: &str, lane: &str) -> LanePaths {
        let hash = lane_hash(repo, lane);
        LanePaths {
            state: self.root.join(format!("{hash}.json")),
            lock: self.root.join(format!("{hash}.lock")),
            directory: self.root.clone(),
            hash,
        }
    }
}

/// Pass the lane on for as long as the owner no longer deserves it. Bounded:
/// each round consumes a waiter or frees the lane.
fn settle(record: &mut LaneRecord, prs: &dyn PrFacts, now_ms: u64) -> Vec<Handover> {
    let mut handovers = Vec::new();
    for _ in 0..=MAX_QUEUE + 1 {
        let reason = match &record.owner {
            Some(owner) if owner.expires_at_ms <= now_ms && owner.claim_by_ms.is_some() => {
                HandoverReason::Unclaimed
            }
            Some(owner) if owner.expires_at_ms <= now_ms => HandoverReason::Expired,
            Some(owner)
                if !owner.release_on.is_empty()
                    && owner
                        .release_on
                        .iter()
                        .all(|number| prs.settled(&record.repo, *number)) =>
            {
                HandoverReason::PrsSettled
            }
            Some(_) => break,
            None if record.queue.is_empty() => break,
            // Never persisted (every release promotes at once), but a waiter
            // behind no owner must not wait forever if it ever is.
            None => HandoverReason::Released,
        };
        let previous = record.owner.take();
        promote(record, now_ms);
        handovers.push(Handover {
            repo: record.repo.clone(),
            lane: record.lane.clone(),
            reason,
            prs: match (&previous, reason) {
                (Some(owner), HandoverReason::PrsSettled) => owner.release_on.clone(),
                _ => Vec::new(),
            },
            from: previous.map(|owner| owner.session),
            to: record.owner.as_ref().map(|owner| owner.session.clone()),
        });
    }
    handovers
}

/// Hand the lane to the head of the queue. Its ttl does not start yet: the
/// waiter is usually not the one running this call and may be asleep, so the
/// turn waits [`PROMOTION_GRACE_SECS`] for it to [`LaneStore::claim`].
fn promote(record: &mut LaneRecord, now_ms: u64) {
    if record.queue.is_empty() {
        return;
    }
    let next = record.queue.remove(0);
    let claim_by = expiry(now_ms, PROMOTION_GRACE_SECS);
    record.owner = Some(LaneHolder {
        session: next.session,
        purpose: next.purpose,
        ttl_secs: next.ttl_secs,
        taken_at_ms: now_ms,
        renewed_at_ms: now_ms,
        expires_at_ms: claim_by,
        release_on: next.release_on,
        claim_by_ms: Some(claim_by),
    });
}

fn holder_from(request: &LaneRequest<'_>, now_ms: u64) -> LaneHolder {
    LaneHolder {
        session: request.session.to_string(),
        purpose: request.purpose.trim().to_string(),
        ttl_secs: request.ttl_secs,
        taken_at_ms: now_ms,
        renewed_at_ms: now_ms,
        expires_at_ms: expiry(now_ms, request.ttl_secs),
        release_on: request.release_on.to_vec(),
        claim_by_ms: None,
    }
}

fn expiry(now_ms: u64, ttl_secs: u64) -> u64 {
    now_ms.saturating_add(ttl_secs.saturating_mul(1000))
}

fn lane_hash(repo: &str, lane: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"harbard-lane-v1\0");
    hasher.update(repo.as_bytes());
    hasher.update(b"\0");
    hasher.update(lane.as_bytes());
    crate::hex(&hasher.finalize())
}

/// Lowercase letters, digits, `.`, `_` and `-`, starting with a letter or
/// digit: a lane name is typed by agents and read in a one-line brief.
pub fn validate_lane_name(lane: &str) -> Result<(), LaneError> {
    let valid = (1..=64).contains(&lane.len())
        && lane
            .bytes()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit())
        && lane.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_' | b'-')
        });
    if valid {
        Ok(())
    } else {
        Err(LaneError::InvalidLane(
            "lane name must be 1-64 lowercase letters, digits, '.', '_' or '-'",
        ))
    }
}

fn validate_repo(repo: &str) -> Result<(), LaneError> {
    if repo.is_empty() || repo.len() > 4096 || repo.chars().any(char::is_control) {
        return Err(LaneError::InvalidLane("lanes need a repository identity"));
    }
    Ok(())
}

fn validate_session(session: &str) -> Result<(), LaneError> {
    if session.trim().is_empty() || session.len() > 255 || session.chars().any(char::is_control) {
        return Err(LaneError::InvalidRequest("invalid session"));
    }
    Ok(())
}

fn validate_request(request: &LaneRequest<'_>) -> Result<(), LaneError> {
    validate_session(request.session)?;
    let purpose = request.purpose.trim();
    if purpose.is_empty() {
        return Err(LaneError::InvalidRequest(
            "say what the lane is for (--for \"…\")",
        ));
    }
    if purpose.len() > MAX_PURPOSE_BYTES || purpose.chars().any(char::is_control) {
        return Err(LaneError::InvalidRequest(
            "--for must be one line of at most 200 bytes",
        ));
    }
    if request.ttl_secs == 0 || request.ttl_secs > MAX_LANE_TTL_SECS {
        return Err(LaneError::InvalidRequest(
            "ttl must be between 1 second and 24 hours",
        ));
    }
    if request.release_on.len() > MAX_RELEASE_ON || request.release_on.contains(&0) {
        return Err(LaneError::InvalidRequest(
            "--release-on takes at most 16 pull request numbers",
        ));
    }
    Ok(())
}

fn read_lane(path: &Path, expected: Option<(&str, &str)>) -> Result<Option<LaneRecord>, LaneError> {
    let mut file = match open_private_read(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    validate_private_regular(&metadata)?;
    if metadata.len() > MAX_LANE_BYTES {
        return Err(LaneError::CorruptState);
    }
    let mut bytes = Vec::new();
    Read::by_ref(&mut file)
        .take(MAX_LANE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let record: LaneRecord = serde_json::from_slice(&bytes).map_err(|_| LaneError::CorruptState)?;
    let mismatched =
        expected.is_some_and(|(repo, lane)| record.repo != repo || record.lane != lane);
    if record.v != LANE_SCHEMA_VERSION
        || mismatched
        || validate_lane_name(&record.lane).is_err()
        || validate_repo(&record.repo).is_err()
        || record.queue.len() > MAX_QUEUE
    {
        return Err(LaneError::CorruptState);
    }
    Ok(Some(record))
}

/// An empty lane leaves no file behind: the directory lists what is held.
fn persist(paths: &LanePaths, record: Option<&LaneRecord>) -> Result<(), LaneError> {
    let Some(record) = record.filter(|record| record.owner.is_some() || !record.queue.is_empty())
    else {
        match fs::remove_file(&paths.state) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        sync_directory(&paths.directory)?;
        return Ok(());
    };
    let bytes = serde_json::to_vec(record).map_err(|_| LaneError::CorruptState)?;
    Ok(write_private_atomic(
        &paths.directory,
        &paths.hash,
        &paths.state,
        &bytes,
    )?)
}

pub(crate) fn write_private_atomic(
    directory: &Path,
    stem: &str,
    target: &Path,
    bytes: &[u8],
) -> std::io::Result<()> {
    let temp = directory.join(format!(
        "{stem}.{}.{}.tmp",
        std::process::id(),
        ulid::Ulid::generate()
    ));
    let mut file = open_private_new(&temp)?;
    if let Err(error) = (|| -> std::io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, target)?;
        sync_directory(directory)
    })() {
        let _ = fs::remove_file(&temp);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
#[path = "lane/tests.rs"]
mod tests;
