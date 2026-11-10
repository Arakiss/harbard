//! Authoritative local leases for mutually exclusive writers and named services.
//!
//! This store is intentionally separate from the fail-open F0 event bus. Lease
//! decisions fail closed: corrupt state is never skipped or overwritten.

#[cfg(not(unix))]
compile_error!("harbard authoritative leases currently require Unix file and process semantics");

use fs2::FileExt;
use nix::errno::Errno;
use nix::sys::signal;
use nix::unistd::Pid;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};
use ulid::Ulid;

pub const DEFAULT_LEASE_TTL_SECS: u64 = 900;
const LEASE_SCHEMA_VERSION: u8 = 1;
const MAX_LEASE_BYTES: u64 = 64 * 1024;
const TEMP_RETENTION_MS: u64 = 24 * 60 * 60 * 1000;

// `flock` behavior for two independently opened descriptors in one process is
// platform-specific. Keep the on-disk fs2 lock authoritative across processes,
// and serialize the tiny critical section inside this process as well.
static PROCESS_LEASE_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LeaseKind {
    Writer,
    Service,
}

impl LeaseKind {
    fn directory(self) -> &'static str {
        match self {
            Self::Writer => "writer",
            Self::Service => "service",
        }
    }
}

/// The resource key is deliberately narrow. No command, environment, free-form
/// intent, or secret-bearing payload has a place in this schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseResource {
    pub kind: LeaseKind,
    pub host: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_common_dir: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
}

impl LeaseResource {
    pub fn writer(
        host: impl Into<String>,
        git_common_dir: impl Into<String>,
        worktree: impl Into<String>,
    ) -> Result<Self, LeaseError> {
        let resource = Self {
            kind: LeaseKind::Writer,
            host: host.into(),
            service: None,
            git_common_dir: Some(git_common_dir.into()),
            worktree: Some(worktree.into()),
        };
        resource.validate()?;
        Ok(resource)
    }

    pub fn service(host: impl Into<String>, name: impl Into<String>) -> Result<Self, LeaseError> {
        let resource = Self {
            kind: LeaseKind::Service,
            host: host.into(),
            service: Some(name.into()),
            git_common_dir: None,
            worktree: None,
        };
        resource.validate()?;
        Ok(resource)
    }

    fn validate(&self) -> Result<(), LeaseError> {
        if !valid_identifier(&self.host) {
            return Err(LeaseError::InvalidResource("invalid host"));
        }
        match self.kind {
            LeaseKind::Writer => {
                if self.service.is_some()
                    || !valid_path_identity(self.git_common_dir.as_deref())
                    || !valid_path_identity(self.worktree.as_deref())
                {
                    return Err(LeaseError::InvalidResource(
                        "writer requires a Git common directory and worktree",
                    ));
                }
            }
            LeaseKind::Service => {
                if self.git_common_dir.is_some()
                    || self.worktree.is_some()
                    || !self.service.as_deref().is_some_and(valid_service_name)
                {
                    return Err(LeaseError::InvalidResource(
                        "service name must use letters, digits, '.', '_', ':', '/', or '-'",
                    ));
                }
            }
        }
        Ok(())
    }

    fn key_hash(&self) -> String {
        self.key_hash_with(b"harbard-lease-v1\0")
    }

    /// Pre-rename salt. Filenames written before the stefna→harbard cut still
    /// use this; doctor must not call them corrupt, and claim must find them.
    fn legacy_key_hash(&self) -> String {
        self.key_hash_with(b"stefna-lease-v1\0")
    }

    fn key_hash_with(&self, salt: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(salt);
        hasher.update(self.kind.directory().as_bytes());
        hasher.update(b"\0");
        hasher.update(self.host.as_bytes());
        hasher.update(b"\0");
        match self.kind {
            LeaseKind::Writer => {
                hasher.update(
                    self.git_common_dir
                        .as_deref()
                        .unwrap_or_default()
                        .as_bytes(),
                );
                hasher.update(b"\0");
                hasher.update(self.worktree.as_deref().unwrap_or_default().as_bytes());
            }
            LeaseKind::Service => {
                hasher.update(self.service.as_deref().unwrap_or_default().as_bytes());
            }
        }
        crate::hex(&hasher.finalize())
    }

    fn filename_matches(&self, stem: &str) -> bool {
        stem == self.key_hash() || stem == self.legacy_key_hash()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseMetadata {
    pub session: String,
    pub harness: String,
    pub pid: Option<u32>,
    /// Parent session when this claimant is a herd child (M5c lineage share).
    pub parent: Option<String>,
    pub repository_id: Option<String>,
    pub branch: Option<String>,
    pub head_sha: Option<String>,
}

impl LeaseMetadata {
    fn validate(&self) -> Result<(), LeaseError> {
        if !valid_identifier(&self.session) {
            return Err(LeaseError::InvalidMetadata("invalid session"));
        }
        if !valid_identifier(&self.harness) {
            return Err(LeaseError::InvalidMetadata("invalid harness"));
        }
        if self.pid == Some(0) {
            return Err(LeaseError::InvalidMetadata("pid must be greater than zero"));
        }
        if self.parent.as_deref().is_some_and(|p| !valid_identifier(p)) {
            return Err(LeaseError::InvalidMetadata("invalid parent session"));
        }
        for value in [self.repository_id.as_deref(), self.branch.as_deref()] {
            if value
                .is_some_and(|value| value.is_empty() || value.len() > 4096 || has_control(value))
            {
                return Err(LeaseError::InvalidMetadata("invalid Git metadata"));
            }
        }
        if self.head_sha.as_deref().is_some_and(|value| {
            !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err(LeaseError::InvalidMetadata(
                "Git head must be a SHA-1 or SHA-256 object id",
            ));
        }
        Ok(())
    }

    fn from_record(record: &LeaseRecord) -> Self {
        Self {
            session: record.session.clone(),
            harness: record.harness.clone(),
            pid: record.pid,
            parent: record.parent.clone(),
            repository_id: record.repository_id.clone(),
            branch: record.branch.clone(),
            head_sha: record.head_sha.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub v: u8,
    pub claim_id: String,
    pub resource: LeaseResource,
    pub session: String,
    pub harness: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Lineage parent when the holder announced as a herd child (M5c).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repository_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    pub ttl_secs: u64,
    pub claimed_at_ms: u64,
    pub renewed_at_ms: u64,
    pub expires_at_ms: u64,
}

/// True when claimant and holder are the same execution tree (M5c).
///
/// Shares when: same session, same live supervisor pid, claimant's parent is
/// the holder, holder's parent is the claimant, or both share a parent id.
pub fn same_writer_lineage(
    holder_session: &str,
    holder_parent: Option<&str>,
    holder_pid: Option<u32>,
    claimant_session: &str,
    claimant_parent: Option<&str>,
    claimant_pid: Option<u32>,
    pid_alive: impl Fn(u32) -> bool,
) -> bool {
    if holder_session == claimant_session {
        return true;
    }
    if let (Some(held), Some(mine)) = (holder_pid, claimant_pid)
        && held == mine
        && pid_alive(held)
    {
        return true;
    }
    if claimant_parent == Some(holder_session) || holder_parent == Some(claimant_session) {
        return true;
    }
    matches!((holder_parent, claimant_parent), (Some(a), Some(b)) if a == b)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseClaim {
    Acquired(LeaseRecord),
    /// The claimant already held this lease: renewed in place, same claim id.
    Renewed(LeaseRecord),
    Conflict(LeaseRecord),
}

/// Result of [`LeaseStore::ensure`]: first writer acquires; the same session
/// renews in place; any other active holder is a conflict.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseEnsure {
    Acquired(LeaseRecord),
    Held(LeaseRecord),
    Conflict(LeaseRecord),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseCheck {
    Free,
    Active {
        lease: LeaseRecord,
        matches_claim: Option<bool>,
    },
    Stale {
        lease: LeaseRecord,
        matches_claim: Option<bool>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseRelease {
    Released(LeaseRecord),
    Free,
    Fenced(LeaseRecord),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LeaseStats {
    pub healthy: bool,
    pub total: u64,
    pub active: u64,
    pub stale: u64,
    pub corrupt: u64,
    pub temp_files: u64,
    pub inaccessible: u64,
}

impl Default for LeaseStats {
    fn default() -> Self {
        Self {
            healthy: true,
            total: 0,
            active: 0,
            stale: 0,
            corrupt: 0,
            temp_files: 0,
            inaccessible: 0,
        }
    }
}

/// One ACTIVE writer lease, redacted for read-only disclosure.
///
/// Deliberately NOT [`LeaseRecord`]: a wire shape that follows the on-disk
/// struct leaks every field added to it later. Two omissions are load-bearing:
///
/// - `pid` — a disclosed supervisor pid is a kill target, and no read surface
///   has ever published one. Liveness is already folded into the listing: a
///   dead-pid lease is not active, so it is simply absent.
/// - `head_sha` / `repository_id` — checkout ownership does not need them.
///
/// - `claim_id` — the argument for publishing it was that renewal and release
///   both prove ownership independently, so the id names a claim without
///   authorising it. That is true of [`LeaseStore::renew_owned`] and
///   [`LeaseStore::release_owned`], and false of [`LeaseStore::check`]: with
///   `renew = true` it extends the window on a matching claim id ALONE, with no
///   session and no pid (see its `renew && matches_claim == Some(true)` arm).
///   While that path exists the id is a capability, and a board does not need
///   one to draw a board.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WriterLeaseView {
    pub host: String,
    /// Exact worktree root the lease protects.
    pub worktree: String,
    /// Git common directory — the shared repository identity across worktrees.
    pub git_common_dir: String,
    /// Human label for `git_common_dir` (`/x/harbard/.git` → `harbard`).
    pub repo: String,
    pub session: String,
    pub harness: String,
    pub branch: Option<String>,
    pub claimed_at_ms: u64,
    pub renewed_at_ms: u64,
    pub expires_at_ms: u64,
}

impl WriterLeaseView {
    fn from_record(record: &LeaseRecord) -> Self {
        let git_common_dir = record.resource.git_common_dir.clone().unwrap_or_default();
        Self {
            host: record.resource.host.clone(),
            worktree: record.resource.worktree.clone().unwrap_or_default(),
            repo: repo_label(&git_common_dir),
            git_common_dir,
            session: record.session.clone(),
            harness: record.harness.clone(),
            branch: record.branch.clone(),
            claimed_at_ms: record.claimed_at_ms,
            renewed_at_ms: record.renewed_at_ms,
            expires_at_ms: record.expires_at_ms,
        }
    }
}

/// `/x/harbard/.git` → `harbard`; `/x/harbard.git` → `harbard`. Falls back to
/// the full path so the label is never empty.
fn repo_label(git_common_dir: &str) -> String {
    let path = Path::new(git_common_dir);
    let label = match path.file_name().and_then(|name| name.to_str()) {
        Some(".git") => path
            .parent()
            .and_then(|parent| parent.file_name())
            .and_then(|name| name.to_str()),
        Some(name) => Some(name.strip_suffix(".git").unwrap_or(name)),
        None => None,
    };
    label
        .filter(|label| !label.is_empty())
        .unwrap_or(git_common_dir)
        .to_string()
}

#[derive(Debug)]
pub enum LeaseError {
    Io(std::io::Error),
    CorruptState,
    InvalidResource(&'static str),
    InvalidMetadata(&'static str),
    InvalidTtl,
    DeadPid,
}

impl fmt::Display for LeaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "lease storage error: {error}"),
            Self::CorruptState => write!(f, "corrupt lease state; refusing to mutate"),
            Self::InvalidResource(message) | Self::InvalidMetadata(message) => f.write_str(message),
            Self::InvalidTtl => {
                f.write_str("ttl must be greater than zero and fit in milliseconds")
            }
            Self::DeadPid => f.write_str("pid is not alive; refusing to acquire a stale lease"),
        }
    }
}

impl std::error::Error for LeaseError {}

impl From<std::io::Error> for LeaseError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

pub trait PidChecker: Send + Sync {
    fn is_alive(&self, pid: u32) -> bool;

    /// Opaque start marker of `pid` (see [`crate::process_start`]); `None`
    /// when unknown. Pairs with the PID so a recycled PID is never mistaken
    /// for the process that was bound earlier. Checkers that cannot tell
    /// return `None`, which keeps every caller on time-based presence.
    fn start_time(&self, _pid: u32) -> Option<u64> {
        None
    }
}

pub trait LeaseClock: Send + Sync {
    fn now_ms(&self) -> u64;
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemLeaseClock;

impl LeaseClock for SystemLeaseClock {
    fn now_ms(&self) -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis() as u64)
            .unwrap_or(0)
    }
}

impl LeaseClock for u64 {
    fn now_ms(&self) -> u64 {
        *self
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct SystemPidChecker;

impl PidChecker for SystemPidChecker {
    fn is_alive(&self, pid: u32) -> bool {
        let Ok(raw) = i32::try_from(pid) else {
            return true;
        };
        match signal::kill(Pid::from_raw(raw), None) {
            Ok(()) => true,
            Err(Errno::ESRCH) => false,
            // EPERM and unexpected inspection errors are treated as alive so a
            // liveness probe can never steal a lease.
            Err(_) => true,
        }
    }

    fn start_time(&self, pid: u32) -> Option<u64> {
        crate::process_start::process_start_time(pid)
    }
}

#[derive(Debug, Clone)]
pub struct LeaseStore {
    home: PathBuf,
    root: PathBuf,
}

impl LeaseStore {
    pub fn at(harbard_home: impl AsRef<Path>) -> Self {
        let home = harbard_home.as_ref().to_path_buf();
        Self {
            root: home.join("leases").join("v1"),
            home,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn claim(
        &self,
        resource: &LeaseResource,
        metadata: &LeaseMetadata,
        ttl_secs: u64,
        clock: impl LeaseClock,
        pid_checker: &dyn PidChecker,
    ) -> Result<LeaseClaim, LeaseError> {
        self.migrate_legacy_filenames();
        resource.validate()?;
        metadata.validate()?;
        let ttl_ms = ttl_millis(ttl_secs)?;
        if metadata.pid.is_some_and(|pid| !pid_checker.is_alive(pid)) {
            return Err(LeaseError::DeadPid);
        }
        self.with_resource_lock(resource, &clock, |paths, now_ms| {
            if let Some(mut current) = read_record(&paths.state, Some(resource))?
                && lease_is_active(&current, now_ms, pid_checker)
            {
                // The holder asking again is not a conflict. A background
                // build, a hook, or a nested tool shell resolves a different
                // supervisor pid than the one that first claimed, so an
                // identity test stricter than lineage made every agent block
                // on ITSELF — and read that as a peer holding the checkout.
                // Renew in place and keep the claim id, so a receipt taken
                // before the second claim stays valid.
                let mine = if resource.kind == LeaseKind::Writer {
                    same_writer_lineage(
                        &current.session,
                        current.parent.as_deref(),
                        current.pid,
                        &metadata.session,
                        metadata.parent.as_deref(),
                        metadata.pid,
                        |pid| pid_checker.is_alive(pid),
                    )
                } else {
                    // A service lease is host-global: only the same session
                    // renews it, never a sibling that shares a parent.
                    current.session == metadata.session
                };
                if !mine {
                    return Ok(LeaseClaim::Conflict(current));
                }
                // The window belongs to whoever acquired it: a repeat claim
                // carrying nothing but the default ttl must not silently
                // stretch or shorten it. Renew on the RECORDED ttl, which also
                // keeps `read_record`'s expires == renewed + ttl invariant —
                // mixing the two writes a record harbard itself calls corrupt.
                let held_ttl_ms = ttl_millis(current.ttl_secs)?;
                let expires_at_ms = now_ms
                    .checked_add(held_ttl_ms)
                    .ok_or(LeaseError::InvalidTtl)?;
                current.renewed_at_ms = now_ms;
                current.expires_at_ms = expires_at_ms;
                atomic_write(paths, &current, now_ms)?;
                return Ok(LeaseClaim::Renewed(current));
            }
            if metadata.pid.is_some_and(|pid| !pid_checker.is_alive(pid)) {
                return Err(LeaseError::DeadPid);
            }
            let expires_at_ms = now_ms.checked_add(ttl_ms).ok_or(LeaseError::InvalidTtl)?;

            let record = LeaseRecord {
                v: LEASE_SCHEMA_VERSION,
                claim_id: new_claim_id(),
                resource: resource.clone(),
                session: metadata.session.clone(),
                harness: metadata.harness.clone(),
                pid: metadata.pid,
                parent: metadata.parent.clone(),
                repository_id: metadata.repository_id.clone(),
                branch: metadata.branch.clone(),
                head_sha: metadata.head_sha.clone(),
                ttl_secs,
                claimed_at_ms: now_ms,
                renewed_at_ms: now_ms,
                expires_at_ms,
            };
            atomic_write(paths, &record, now_ms)?;
            Ok(LeaseClaim::Acquired(record))
        })
    }

    pub fn check(
        &self,
        resource: &LeaseResource,
        claim_id: Option<&str>,
        renew: bool,
        clock: impl LeaseClock,
        pid_checker: &dyn PidChecker,
    ) -> Result<LeaseCheck, LeaseError> {
        resource.validate()?;
        if claim_id.is_some_and(|claim_id| !valid_claim_id(claim_id)) {
            return Err(LeaseError::InvalidMetadata("claim id must be a ULID"));
        }
        self.with_resource_lock(resource, &clock, |paths, now_ms| {
            let Some(mut current) = read_record(&paths.state, Some(resource))? else {
                return Ok(LeaseCheck::Free);
            };
            let matches_claim = claim_id.map(|claim_id| claim_id == current.claim_id);
            if !lease_is_active(&current, now_ms, pid_checker) {
                return Ok(LeaseCheck::Stale {
                    lease: current,
                    matches_claim,
                });
            }
            if renew && matches_claim == Some(true) {
                let ttl_ms = ttl_millis(current.ttl_secs)?;
                current.renewed_at_ms = now_ms;
                current.expires_at_ms = now_ms.checked_add(ttl_ms).ok_or(LeaseError::InvalidTtl)?;
                atomic_write(paths, &current, now_ms)?;
            }
            Ok(LeaseCheck::Active {
                lease: current,
                matches_claim,
            })
        })
    }

    /// Renew a matching claim only after proving writer ownership. Service
    /// leases continue to use the claim id as their capability token.
    pub fn renew_owned(
        &self,
        resource: &LeaseResource,
        claim_id: &str,
        requester: &LeaseMetadata,
        clock: impl LeaseClock,
        pid_checker: &dyn PidChecker,
    ) -> Result<LeaseCheck, LeaseError> {
        resource.validate()?;
        requester.validate()?;
        if !valid_claim_id(claim_id) {
            return Err(LeaseError::InvalidMetadata("claim id must be a ULID"));
        }
        self.with_resource_lock(resource, &clock, |paths, now_ms| {
            let Some(mut current) = read_record(&paths.state, Some(resource))? else {
                return Ok(LeaseCheck::Free);
            };
            if !lease_is_active(&current, now_ms, pid_checker) {
                let matches_claim = Some(claim_id == current.claim_id);
                return Ok(LeaseCheck::Stale {
                    lease: current,
                    matches_claim,
                });
            }
            let token_matches = claim_id == current.claim_id;
            let owner_matches = resource.kind != LeaseKind::Writer
                || (current.session == requester.session
                    && current.pid.is_some()
                    && current.pid == requester.pid
                    && current.pid.is_some_and(|pid| pid_checker.is_alive(pid)));
            if !token_matches || !owner_matches {
                return Ok(LeaseCheck::Active {
                    lease: current,
                    matches_claim: Some(false),
                });
            }
            let ttl_ms = ttl_millis(current.ttl_secs)?;
            current.renewed_at_ms = now_ms;
            current.expires_at_ms = now_ms.checked_add(ttl_ms).ok_or(LeaseError::InvalidTtl)?;
            atomic_write(paths, &current, now_ms)?;
            Ok(LeaseCheck::Active {
                lease: current,
                matches_claim: Some(true),
            })
        })
    }

    #[cfg(test)]
    fn release(
        &self,
        resource: &LeaseResource,
        claim_id: &str,
        clock: impl LeaseClock,
    ) -> Result<LeaseRelease, LeaseError> {
        resource.validate()?;
        if !valid_claim_id(claim_id) {
            return Err(LeaseError::InvalidMetadata("claim id must be a ULID"));
        }
        self.with_resource_lock(resource, &clock, |paths, _now_ms| {
            let Some(current) = read_record(&paths.state, Some(resource))? else {
                return Ok(LeaseRelease::Free);
            };
            if current.claim_id != claim_id {
                return Ok(LeaseRelease::Fenced(current));
            }
            fs::remove_file(&paths.state)?;
            sync_directory(&paths.directory)?;
            Ok(LeaseRelease::Released(current))
        })
    }

    /// Release a lease after proving the caller owns the writer execution
    /// domain. Service leases retain capability-token semantics; writer
    /// leases additionally require the exact session and live supervisor PID.
    pub fn release_owned(
        &self,
        resource: &LeaseResource,
        claim_id: &str,
        requester: &LeaseMetadata,
        clock: impl LeaseClock,
        pid_checker: &dyn PidChecker,
    ) -> Result<LeaseRelease, LeaseError> {
        resource.validate()?;
        requester.validate()?;
        if !valid_claim_id(claim_id) {
            return Err(LeaseError::InvalidMetadata("claim id must be a ULID"));
        }
        self.with_resource_lock(resource, &clock, |paths, _now_ms| {
            let Some(current) = read_record(&paths.state, Some(resource))? else {
                return Ok(LeaseRelease::Free);
            };
            if current.claim_id != claim_id {
                return Ok(LeaseRelease::Fenced(current));
            }
            if resource.kind == LeaseKind::Writer
                && (current.session != requester.session
                    || current.pid.is_none()
                    || current.pid != requester.pid
                    || !current.pid.is_some_and(|pid| pid_checker.is_alive(pid)))
            {
                return Ok(LeaseRelease::Fenced(current));
            }
            fs::remove_file(&paths.state)?;
            sync_directory(&paths.directory)?;
            Ok(LeaseRelease::Released(current))
        })
    }

    /// Release every writer lease owned by one exact session/supervisor pair.
    /// This closes leases acquired for target worktrees that differ from the
    /// session cwd (for example an absolute-path edit or `git -C`).
    pub fn release_owned_writers(
        &self,
        requester: &LeaseMetadata,
        clock: impl LeaseClock,
        pid_checker: &dyn PidChecker,
    ) -> Result<u64, LeaseError> {
        requester.validate()?;
        let Some(requester_pid) = requester.pid else {
            return Err(LeaseError::InvalidMetadata(
                "writer release requires supervisor pid",
            ));
        };
        if !pid_checker.is_alive(requester_pid) {
            return Err(LeaseError::DeadPid);
        }
        let directory = self.root.join(LeaseKind::Writer.directory());
        let entries = match fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(error) => return Err(LeaseError::Io(error)),
        };
        let mut owned = Vec::new();
        for entry in entries {
            let path = entry.map_err(LeaseError::Io)?.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let Some(record) = read_record(&path, None)? else {
                continue;
            };
            if record.resource.kind == LeaseKind::Writer
                && record.session == requester.session
                && record.pid == Some(requester_pid)
            {
                owned.push(record);
            }
        }
        let now_ms = clock.now_ms();
        let mut released = 0;
        for record in owned {
            if matches!(
                self.release_owned(
                    &record.resource,
                    &record.claim_id,
                    requester,
                    now_ms,
                    pid_checker
                )?,
                LeaseRelease::Released(_)
            ) {
                released += 1;
            }
        }
        Ok(released)
    }

    /// Acquire or renew the writer lease for `metadata.session`.
    ///
    /// Same session, same live supervisor pid, or identified parent/child
    /// lineage (M5c) → [`LeaseEnsure::Held`]. Free/stale →
    /// [`LeaseEnsure::Acquired`]. Verified foreign holder →
    /// [`LeaseEnsure::Conflict`] (fail-closed).
    pub fn ensure(
        &self,
        resource: &LeaseResource,
        metadata: &LeaseMetadata,
        ttl_secs: u64,
        clock: impl LeaseClock,
        pid_checker: &dyn PidChecker,
    ) -> Result<LeaseEnsure, LeaseError> {
        self.migrate_legacy_filenames();
        resource.validate()?;
        metadata.validate()?;
        let ttl_ms = ttl_millis(ttl_secs)?;
        if metadata.pid.is_some_and(|pid| !pid_checker.is_alive(pid)) {
            return Err(LeaseError::DeadPid);
        }
        self.with_resource_lock(resource, &clock, |paths, now_ms| {
            if let Some(mut current) = read_record(&paths.state, Some(resource))?
                && lease_is_active(&current, now_ms, pid_checker)
            {
                let shared = same_writer_lineage(
                    &current.session,
                    current.parent.as_deref(),
                    current.pid,
                    &metadata.session,
                    metadata.parent.as_deref(),
                    metadata.pid,
                    |pid| pid_checker.is_alive(pid),
                );
                if shared {
                    current.renewed_at_ms = now_ms;
                    current.expires_at_ms = now_ms
                        .checked_add(ttl_millis(current.ttl_secs)?)
                        .ok_or(LeaseError::InvalidTtl)?;
                    // Refresh supervisor pid / harness metadata on renew.
                    // Keep the original session as owner so release_session(parent)
                    // still works when a child renewed under the shared pid.
                    current.harness = metadata.harness.clone();
                    if metadata.pid.is_some() {
                        current.pid = metadata.pid;
                    }
                    if current.parent.is_none() {
                        current.parent = metadata.parent.clone();
                    }
                    atomic_write(paths, &current, now_ms)?;
                    return Ok(LeaseEnsure::Held(current));
                }
                return Ok(LeaseEnsure::Conflict(current));
            }
            if metadata.pid.is_some_and(|pid| !pid_checker.is_alive(pid)) {
                return Err(LeaseError::DeadPid);
            }
            let expires_at_ms = now_ms.checked_add(ttl_ms).ok_or(LeaseError::InvalidTtl)?;
            let record = LeaseRecord {
                v: LEASE_SCHEMA_VERSION,
                claim_id: new_claim_id(),
                resource: resource.clone(),
                session: metadata.session.clone(),
                harness: metadata.harness.clone(),
                pid: metadata.pid,
                parent: metadata.parent.clone(),
                repository_id: metadata.repository_id.clone(),
                branch: metadata.branch.clone(),
                head_sha: metadata.head_sha.clone(),
                ttl_secs,
                claimed_at_ms: now_ms,
                renewed_at_ms: now_ms,
                expires_at_ms,
            };
            atomic_write(paths, &record, now_ms)?;
            Ok(LeaseEnsure::Acquired(record))
        })
    }

    /// Release the resource when the active lease belongs to `session`.
    /// Idempotent when free or already held by someone else (fenced).
    pub fn release_session(
        &self,
        resource: &LeaseResource,
        session: &str,
        clock: impl LeaseClock,
    ) -> Result<LeaseRelease, LeaseError> {
        resource.validate()?;
        if !valid_identifier(session) {
            return Err(LeaseError::InvalidMetadata("invalid session"));
        }
        self.with_resource_lock(resource, &clock, |paths, _now_ms| {
            let Some(current) = read_record(&paths.state, Some(resource))? else {
                return Ok(LeaseRelease::Free);
            };
            if current.session != session {
                return Ok(LeaseRelease::Fenced(current));
            }
            fs::remove_file(&paths.state)?;
            sync_directory(&paths.directory)?;
            Ok(LeaseRelease::Released(current))
        })
    }

    /// Every ACTIVE writer lease, redacted for read-only disclosure.
    ///
    /// Read-only by construction. Unlike [`LeaseStore::stats`] it does NOT
    /// call `migrate_legacy_filenames`: a listing must never rename files on
    /// disk, and it does not need to — `filename_matches` still accepts the
    /// pre-rename salt, so a legacy file is listed rather than called corrupt.
    ///
    /// Classification reuses `read_record` + `lease_is_active`, so expired,
    /// dead-pid, corrupt, mismatched and unreadable records are all skipped.
    /// A degraded file is never an error: the caller sees the healthy holders
    /// it can act on, and `stats` remains the health surface.
    ///
    /// No lock is taken. That is the same posture as `release_owned_writers`
    /// and it is safe because every write lands via temp file + `rename`, so
    /// no torn record is observable. The consequence is that a record can
    /// vanish immediately after the scan — this listing is informational and
    /// must never back a mutation decision.
    pub fn active_writers(
        &self,
        clock: impl LeaseClock,
        pid_checker: &dyn PidChecker,
    ) -> Vec<WriterLeaseView> {
        let now_ms = clock.now_ms();
        let directory = self.root.join(LeaseKind::Writer.directory());
        let Ok(entries) = fs::read_dir(&directory) else {
            return Vec::new();
        };
        let mut views = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.ends_with(".json") {
                continue;
            }
            let Ok(Some(record)) = read_record(&path, None) else {
                continue;
            };
            // Same corruption rule as `stats`: a record whose resource does
            // not hash to its own filename is not trustworthy state.
            let named_for_itself = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .is_some_and(|stem| record.resource.filename_matches(stem));
            if !named_for_itself || record.resource.kind != LeaseKind::Writer {
                continue;
            }
            if !lease_is_active(&record, now_ms, pid_checker) {
                continue;
            }
            views.push(WriterLeaseView::from_record(&record));
        }
        // Most recently renewed first; worktree breaks ties so the order is
        // stable across requests instead of following directory order.
        views.sort_by(|a, b| {
            b.renewed_at_ms
                .cmp(&a.renewed_at_ms)
                .then_with(|| a.worktree.cmp(&b.worktree))
        });
        views
    }

    pub fn stats(&self, clock: impl LeaseClock, pid_checker: &dyn PidChecker) -> LeaseStats {
        self.migrate_legacy_filenames();
        let now_ms = clock.now_ms();
        let mut stats = LeaseStats::default();
        for kind in [LeaseKind::Writer, LeaseKind::Service] {
            let dir = self.root.join(kind.directory());
            let mut directories_valid = true;
            for ancestor in [self.home.join("leases"), self.root.clone(), dir.clone()] {
                match validate_existing_private_directory(&ancestor) {
                    Ok(true) => {}
                    Ok(false) => {
                        directories_valid = false;
                        break;
                    }
                    Err(_) => {
                        stats.inaccessible += 1;
                        directories_valid = false;
                        break;
                    }
                }
            }
            if !directories_valid {
                continue;
            }
            let entries = match fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => {
                    stats.inaccessible += 1;
                    continue;
                }
            };
            for entry in entries {
                let entry = match entry {
                    Ok(entry) => entry,
                    Err(_) => {
                        stats.inaccessible += 1;
                        continue;
                    }
                };
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if name.ends_with(".tmp") {
                    stats.temp_files += 1;
                    continue;
                }
                if !name.ends_with(".json") {
                    continue;
                }
                match read_record(&path, None) {
                    Ok(None) => {
                        // Empty placeholder — free slot, not a lease.
                    }
                    Ok(Some(record))
                        if !path
                            .file_stem()
                            .and_then(|stem| stem.to_str())
                            .is_some_and(|stem| record.resource.filename_matches(stem)) =>
                    {
                        stats.total += 1;
                        stats.corrupt += 1;
                    }
                    Ok(Some(record)) if lease_is_active(&record, now_ms, pid_checker) => {
                        stats.total += 1;
                        stats.active += 1;
                    }
                    Ok(Some(_)) => {
                        stats.total += 1;
                        stats.stale += 1;
                    }
                    Err(LeaseError::CorruptState) => {
                        stats.total += 1;
                        stats.corrupt += 1;
                    }
                    Err(_) => {
                        stats.total += 1;
                        stats.inaccessible += 1;
                    }
                }
            }
        }
        stats.healthy = stats.corrupt == 0 && stats.inaccessible == 0;
        stats
    }

    fn with_resource_lock<T>(
        &self,
        resource: &LeaseResource,
        clock: &dyn LeaseClock,
        operation: impl FnOnce(&ResourcePaths, u64) -> Result<T, LeaseError>,
    ) -> Result<T, LeaseError> {
        let _process_guard = PROCESS_LEASE_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let paths = self.resource_paths(resource);
        ensure_private_directory(&self.home)?;
        ensure_private_directory(self.root.parent().expect("lease root has parent"))?;
        ensure_private_directory(&self.root)?;
        ensure_private_directory(&paths.directory)?;
        let lock = open_private_lock(&paths.lock)?;
        lock.lock_exclusive()?;
        let now_ms = clock.now_ms();
        cleanup_resource_temps(&paths, now_ms);
        let result = operation(&paths, now_ms);
        let unlock_result = FileExt::unlock(&lock);
        match (result, unlock_result) {
            (Ok(value), Ok(())) => Ok(value),
            (Err(error), _) => Err(error),
            (Ok(_), Err(error)) => Err(LeaseError::Io(error)),
        }
    }

    fn resource_paths(&self, resource: &LeaseResource) -> ResourcePaths {
        let directory = self.root.join(resource.kind.directory());
        let hash = resource.key_hash();
        ResourcePaths {
            state: directory.join(format!("{hash}.json")),
            lock: directory.join(format!("{hash}.lock")),
            directory,
            hash,
        }
    }

    /// Rename legacy-salt lease files onto the current hash so doctor stops
    /// reporting them as corrupt and future lookups hit one path.
    fn migrate_legacy_filenames(&self) {
        for kind in [LeaseKind::Writer, LeaseKind::Service] {
            let dir = self.root.join(kind.directory());
            let Ok(entries) = fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                if !name.ends_with(".json") {
                    continue;
                }
                let Ok(Some(record)) = read_record(&path, None) else {
                    continue;
                };
                let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
                let current = record.resource.key_hash();
                if stem == current || stem != record.resource.legacy_key_hash() {
                    continue;
                }
                let new_state = dir.join(format!("{current}.json"));
                let new_lock = dir.join(format!("{current}.lock"));
                let old_lock = dir.join(format!("{stem}.lock"));
                if new_state.exists() {
                    // Current-hash file already won; drop the legacy duplicate.
                    let _ = fs::remove_file(&path);
                    let _ = fs::remove_file(&old_lock);
                    continue;
                }
                if fs::rename(&path, &new_state).is_ok() && old_lock.exists() {
                    let _ = fs::rename(&old_lock, &new_lock);
                }
            }
        }
    }
}

struct ResourcePaths {
    directory: PathBuf,
    state: PathBuf,
    lock: PathBuf,
    hash: String,
}

fn lease_is_active(record: &LeaseRecord, now_ms: u64, pid_checker: &dyn PidChecker) -> bool {
    if record.expires_at_ms <= now_ms {
        return false;
    }
    record.pid.is_none_or(|pid| pid_checker.is_alive(pid))
}

fn ttl_millis(ttl_secs: u64) -> Result<u64, LeaseError> {
    if ttl_secs == 0 {
        return Err(LeaseError::InvalidTtl);
    }
    ttl_secs.checked_mul(1000).ok_or(LeaseError::InvalidTtl)
}

fn new_claim_id() -> String {
    Ulid::generate().to_string()
}

fn valid_claim_id(value: &str) -> bool {
    Ulid::from_string(value).is_ok()
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= 255 && !has_control(value)
}

fn valid_path_identity(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.is_empty() && value.len() <= 4096 && !has_control(value))
}

fn valid_service_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'/' | b'-')
        })
}

fn has_control(value: &str) -> bool {
    value.chars().any(char::is_control)
}

fn read_record(
    path: &Path,
    expected_resource: Option<&LeaseResource>,
) -> Result<Option<LeaseRecord>, LeaseError> {
    let mut file = match open_private_read(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let metadata = file.metadata()?;
    validate_private_regular(&metadata)?;
    if metadata.len() > MAX_LEASE_BYTES {
        return Err(LeaseError::CorruptState);
    }
    let mut bytes = Vec::new();
    std::io::Read::by_ref(&mut file)
        .take(MAX_LEASE_BYTES + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_LEASE_BYTES {
        return Err(LeaseError::CorruptState);
    }
    // Empty state files appear when a lock was created and never claimed (or a
    // crash left a zero-byte placeholder). Treat as free, not corrupt — doctor
    // was marking every empty lease "unhealthy" and teaching operators to ignore it.
    if bytes.is_empty() {
        return Ok(None);
    }
    let record: LeaseRecord =
        serde_json::from_slice(&bytes).map_err(|_| LeaseError::CorruptState)?;
    let expected_expiry = ttl_millis(record.ttl_secs)
        .ok()
        .and_then(|ttl_ms| record.renewed_at_ms.checked_add(ttl_ms));
    if record.v != LEASE_SCHEMA_VERSION
        || !valid_claim_id(&record.claim_id)
        || record.resource.validate().is_err()
        || expected_resource.is_some_and(|expected| expected != &record.resource)
        || LeaseMetadata::from_record(&record).validate().is_err()
        || record.ttl_secs == 0
        || ttl_millis(record.ttl_secs).is_err()
        || expected_expiry != Some(record.expires_at_ms)
        || record.renewed_at_ms < record.claimed_at_ms
    {
        return Err(LeaseError::CorruptState);
    }
    Ok(Some(record))
}

fn atomic_write(
    paths: &ResourcePaths,
    record: &LeaseRecord,
    now_ms: u64,
) -> Result<(), LeaseError> {
    let bytes = serde_json::to_vec(record).map_err(|_| LeaseError::CorruptState)?;
    let temp = paths
        .directory
        .join(format!("{}.{}.{}.tmp", paths.hash, now_ms, record.claim_id));
    let mut file = open_private_new(&temp)?;
    if let Err(error) = (|| -> std::io::Result<()> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        fs::rename(&temp, &paths.state)?;
        sync_directory(&paths.directory)
    })() {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}

fn cleanup_resource_temps(paths: &ResourcePaths, now_ms: u64) {
    let Ok(entries) = fs::read_dir(&paths.directory) else {
        return;
    };
    let prefix = format!("{}.", paths.hash);
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let Some(rest) = name.strip_prefix(&prefix) else {
            continue;
        };
        let Some(timestamp) = rest
            .strip_suffix(".tmp")
            .and_then(|rest| rest.split('.').next())
            .and_then(|timestamp| timestamp.parse::<u64>().ok())
        else {
            continue;
        };
        if now_ms.saturating_sub(timestamp) > TEMP_RETENTION_MS {
            let _ = fs::remove_file(path);
        }
    }
}

pub(crate) fn ensure_private_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "lease directory must not be a symlink",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            if let Err(create_error) = fs::create_dir(path)
                && create_error.kind() != std::io::ErrorKind::AlreadyExists
            {
                return Err(create_error);
            }
        }
        Err(error) => return Err(error),
    }
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_DIRECTORY | nix::libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = directory.metadata()?;
    if !metadata.is_dir() || metadata.uid() != nix::unistd::geteuid().as_raw() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "lease directory must be owned by the current user",
        ));
    }
    directory.set_permissions(fs::Permissions::from_mode(0o700))
}

fn validate_existing_private_directory(path: &Path) -> std::io::Result<bool> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "lease directory must not be a symlink",
        ));
    }
    let directory = OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_DIRECTORY | nix::libc::O_NOFOLLOW)
        .open(path)?;
    let metadata = directory.metadata()?;
    if metadata.uid() != nix::unistd::geteuid().as_raw()
        || metadata.permissions().mode() & 0o777 != 0o700
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "lease directory must be private and owned by the current user",
        ));
    }
    Ok(true)
}

pub(crate) fn open_private_lock(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(path)?;
    validate_owned_regular(&file.metadata()?)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    Ok(file)
}

pub(crate) fn open_private_new(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(path)?;
    validate_owned_regular(&file.metadata()?)?;
    Ok(file)
}

pub(crate) fn open_private_read(path: &Path) -> std::io::Result<File> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_NOFOLLOW | nix::libc::O_NONBLOCK | nix::libc::O_CLOEXEC)
        .open(path)
}

fn validate_owned_regular(metadata: &fs::Metadata) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    if !metadata.is_file() || metadata.uid() != nix::unistd::geteuid().as_raw() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "lease file must be regular and owned by the current user",
        ));
    }
    Ok(())
}

pub(crate) fn validate_private_regular(metadata: &fs::Metadata) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    validate_owned_regular(metadata)?;
    if metadata.mode() & 0o777 != 0o600 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "lease state must have mode 0600",
        ));
    }
    Ok(())
}

pub(crate) fn sync_directory(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .custom_flags(nix::libc::O_DIRECTORY | nix::libc::O_NOFOLLOW | nix::libc::O_CLOEXEC)
        .open(path)?
        .sync_all()
}

#[cfg(test)]
#[path = "lease/tests.rs"]
mod tests;
