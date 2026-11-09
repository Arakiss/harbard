//! Pairing: one credential per paired device, never a shared one.
//!
//! The hub issues a bearer token for each device it invites (`harbard hub
//! invite`). The device keeps it at 0600 in its own `satellite.json`; the
//! hub keeps only its SHA-256 in `{home}/credentials.json`, also at 0600, so
//! the hub's own files never hold a token a thief could present. No
//! Keychain, so nothing raises a dialog.
//!
//! What the hub decides about a device (its [`Access`] role, when it was
//! paired, whether it is revoked, which sessions it published) lives on its
//! card in `devices.json`. Both files change together, under one lock
//! (`devices.json.lock`), and are read again on every request: revoking a
//! device takes effect on its very next request, without a restart.
//!
//! `hub.token` stays what the local console and CLI present: the hub's own
//! key, valid only from this machine and never on behalf of a device.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::device::{Access, DEVICE_SESSIONS_MAX, DeviceCard, DeviceRegistry, DeviceRole};
use crate::store::Store;

/// Every device token starts with this, so a log line or a paste can be
/// told apart from the hub's own key at a glance.
pub const TOKEN_PREFIX: &str = "hbd_";
/// An invite nobody claimed stops working after a day.
pub const INVITE_TTL_MS: u64 = 24 * 60 * 60 * 1000;

/// One issued token: its hash, and the device that claimed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    /// SHA-256 of the token, lowercase hex.
    pub hash: String,
    /// The device that claimed the invite. `None` until it pairs.
    #[serde(default)]
    pub device_id: Option<String>,
    /// The name the operator gave the invite.
    #[serde(default)]
    pub name: String,
    /// The role the device gets when it claims the invite.
    #[serde(default)]
    pub access: Access,
    pub created_ms: u64,
    /// Only for an unclaimed invite.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_ms: Option<u64>,
    /// The workspaces the device is granted when it claims the invite, so a
    /// teammate's device never sees the default workspace for a moment
    /// before its grant. Empty: the default workspace, as with any device
    /// that holds no grant.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credentials {
    #[serde(default)]
    pub v: u32,
    #[serde(default)]
    pub entries: Vec<Credential>,
}

impl Credentials {
    pub fn path(store: &Store) -> PathBuf {
        store.root().join("credentials.json")
    }

    /// Missing or unreadable is empty: no device authenticates, which is the
    /// safe failure.
    pub fn load(store: &Store) -> Self {
        fs::read_to_string(Self::path(store))
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    fn find(&self, hash: &str) -> Option<&Credential> {
        self.entries.iter().find(|c| c.hash == hash)
    }
}

/// SHA-256 of a token, lowercase hex.
pub fn token_hash(token: &str) -> String {
    let digest = Sha256::digest(token.trim().as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// A fresh device token: the prefix and two ULIDs (160 random bits).
pub fn new_token() -> String {
    format!(
        "{TOKEN_PREFIX}{}{}",
        ulid::Ulid::generate().to_string().to_lowercase(),
        ulid::Ulid::generate().to_string().to_lowercase()
    )
}

/// What a presented token is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// A paired device, with its card as the hub keeps it.
    Device(DeviceCard),
    /// An invite nobody claimed yet: it may only pair.
    Invite(Credential),
    Revoked,
    Expired,
    Unknown,
}

/// Look a token up, reading both files fresh.
pub fn verify(store: &Store, token: &str, now_ms: u64) -> Verdict {
    let creds = Credentials::load(store);
    let Some(entry) = creds.find(&token_hash(token)) else {
        return Verdict::Unknown;
    };
    if entry.revoked_ms.is_some() {
        return Verdict::Revoked;
    }
    match &entry.device_id {
        Some(id) => match DeviceRegistry::load(store).get(id) {
            Some(card) if card.revoked_ms.is_some() => Verdict::Revoked,
            Some(card) => Verdict::Device(card.clone()),
            None => Verdict::Unknown,
        },
        None if entry.expires_ms.is_some_and(|at| now_ms >= at) => Verdict::Expired,
        None => Verdict::Invite(entry.clone()),
    }
}

/// Change the registry and the credentials as one step, under one lock.
pub fn update<T>(
    store: &Store,
    f: impl FnOnce(&mut DeviceRegistry, &mut Credentials) -> T,
) -> io::Result<T> {
    use fs2::FileExt;
    fs::create_dir_all(store.root())?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(store.root().join("devices.json.lock"))?;
    lock.lock_exclusive()?;
    let result = (|| {
        let mut reg = DeviceRegistry::load(store);
        let mut creds = Credentials::load(store);
        let before = (reg.clone(), creds.clone());
        let out = f(&mut reg, &mut creds);
        if reg != before.0 {
            write_atomic(&DeviceRegistry::path(store), &reg, false)?;
        }
        if creds != before.1 {
            creds.v = 1;
            write_atomic(&Credentials::path(store), &creds, true)?;
        }
        Ok(out)
    })();
    let _ = FileExt::unlock(&lock);
    result
}

fn write_atomic<T: Serialize>(path: &Path, value: &T, private: bool) -> io::Result<()> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let raw = serde_json::to_vec_pretty(value).map_err(io::Error::other)?;
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = path.with_extension(format!("json.tmp.{}.{n}", std::process::id()));
    let written = (|| {
        use std::io::Write;
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(if private { 0o600 } else { 0o644 });
        }
        let mut file = opts.open(&tmp)?;
        file.write_all(&raw)?;
        file.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    written
}

/// Why an invite could not be issued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IssueError {
    /// Operator is never handed out by an invite: it is a promotion.
    OperatorByInvite,
    EmptyName,
}

/// Issue an invite for one device. Returns the token, shown once.
pub fn issue(
    store: &Store,
    name: &str,
    access: Access,
    now_ms: u64,
) -> io::Result<Result<String, IssueError>> {
    issue_granting(store, name, access, &[], now_ms)
}

/// [`issue`], with the workspaces the device is granted when it pairs.
pub fn issue_granting(
    store: &Store,
    name: &str,
    access: Access,
    workspaces: &[String],
    now_ms: u64,
) -> io::Result<Result<String, IssueError>> {
    if access == Access::Operator {
        return Ok(Err(IssueError::OperatorByInvite));
    }
    let name = name.trim();
    if name.is_empty() {
        return Ok(Err(IssueError::EmptyName));
    }
    let token = new_token();
    let entry = Credential {
        hash: token_hash(&token),
        device_id: None,
        name: name.to_string(),
        access,
        created_ms: now_ms,
        expires_ms: Some(now_ms + INVITE_TTL_MS),
        revoked_ms: None,
        workspaces: workspaces.to_vec(),
    };
    update(store, |_, creds| {
        prune_expired(creds, now_ms);
        creds.entries.push(entry);
    })?;
    Ok(Ok(token))
}

/// A device id the hub accepts: it names the device's log file, so only
/// `[A-Za-z0-9_-]`, at most 64 characters.
pub fn valid_device_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Why a device could not pair with an invite.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClaimError {
    /// The card's id is not a [`valid_device_id`].
    BadId,
    Unknown,
    Revoked,
    Expired,
    /// The invite was claimed by another device.
    Taken,
    /// That id is the hub's own.
    Hub,
    /// Another device already holds that id with a live credential: an
    /// invite never takes over such a card. The operator revokes it first.
    IdTaken,
}

impl ClaimError {
    pub fn code(self) -> &'static str {
        match self {
            ClaimError::BadId => "bad_device_id",
            ClaimError::Unknown => "unknown_invite",
            ClaimError::Revoked => "revoked",
            ClaimError::Expired => "invite_expired",
            ClaimError::Taken => "invite_taken",
            ClaimError::Hub => "hub_id",
            ClaimError::IdTaken => "device_id_taken",
        }
    }
}

/// A device claims the invite whose token hashes to `hash`, with the card it
/// says about itself. Only what the device may declare is taken from it
/// ([`DeviceCard::declared`]): its name is the one the operator gave the
/// invite, its host is fixed here and never the hub's, its role comes from
/// the invite, and it starts with no session of its own.
///
/// An invite pairs an id the hub does not hold yet, or one the operator
/// revoked; it never takes over a live card. Returns the card and whether a
/// new pairing happened.
pub fn claim(
    store: &Store,
    hash: &str,
    card: DeviceCard,
    now_ms: u64,
) -> io::Result<Result<(DeviceCard, bool), ClaimError>> {
    if !valid_device_id(&card.id) {
        return Ok(Err(ClaimError::BadId));
    }
    update(store, |reg, creds| {
        prune_expired(creds, now_ms);
        let Some(at) = creds.entries.iter().position(|c| c.hash == hash) else {
            return Err(ClaimError::Unknown);
        };
        let entry = creds.entries[at].clone();
        if entry.revoked_ms.is_some() {
            return Err(ClaimError::Revoked);
        }
        if entry.device_id.is_some() {
            return Err(ClaimError::Taken);
        }
        if entry.expires_ms.is_some_and(|at| now_ms >= at) {
            return Err(ClaimError::Expired);
        }
        match reg.get(&card.id) {
            Some(c) if c.role == DeviceRole::Hub => return Err(ClaimError::Hub),
            // A live card that holds a credential is someone's. A revoked
            // card, or one paired with the old shared secret (no
            // credential), is free to re-pair, starting from nothing.
            Some(c) if c.revoked_ms.is_none() && has_credential(creds, &c.id) => {
                return Err(ClaimError::IdTaken);
            }
            _ => {}
        }
        let hub_host = crate::hostname();
        let mut taken: Vec<&str> = reg
            .devices
            .iter()
            .filter(|d| d.id != card.id)
            .map(|d| d.host.as_str())
            .collect();
        taken.push(&hub_host);
        let mut fresh = card.declared();
        fresh.name = entry.name.clone();
        fresh.host = crate::device::device_host(&fresh.host, &taken, &fresh.id);
        fresh.access = entry.access;
        fresh.created_ms = now_ms;
        fresh.joined_ms = now_ms;
        fresh.last_seen_ms = now_ms;
        fresh.workspaces = entry.workspaces.clone();
        // A revoked card's credentials are revoked already; any left over
        // for that id stop here, so the device holds exactly one.
        for other in creds.entries.iter_mut() {
            if other.device_id.as_deref() == Some(card.id.as_str()) && other.revoked_ms.is_none() {
                other.revoked_ms = Some(now_ms);
            }
        }
        let e = &mut creds.entries[at];
        e.device_id = Some(fresh.id.clone());
        e.expires_ms = None;
        match reg.get_mut(&fresh.id) {
            Some(existing) => *existing = fresh.clone(),
            None => reg.devices.push(fresh.clone()),
        }
        Ok((fresh, true))
    })
}

/// A paired device posts its card again (heartbeat). Only its OS,
/// architecture and harnesses may change: its name and host stay what the
/// hub fixed at pairing, and nothing the hub decides comes from the body.
pub fn heartbeat(store: &Store, card: DeviceCard, now_ms: u64) -> io::Result<DeviceCard> {
    let mut card = card.declared();
    update(store, |reg, _| {
        if let Some(existing) = reg.get(&card.id) {
            card.name = existing.name.clone();
            card.host = existing.host.clone();
        }
        reg.upsert(card, now_ms).0
    })
}

/// Last seen, for a request that is not a heartbeat.
pub fn touch(store: &Store, device_id: &str, now_ms: u64) -> io::Result<()> {
    update(store, |reg, _| {
        if let Some(card) = reg.get_mut(device_id) {
            card.last_seen_ms = now_ms;
        }
    })
}

/// Revoke a device: its card and every credential it holds. Returns the
/// card, or `None` when no such device exists or it is the hub itself.
pub fn revoke(store: &Store, device_id: &str, now_ms: u64) -> io::Result<Option<DeviceCard>> {
    update(store, |reg, creds| {
        let card = reg.get_mut(device_id)?;
        if card.role == DeviceRole::Hub {
            return None;
        }
        card.revoked_ms.get_or_insert(now_ms);
        let out = card.clone();
        for c in creds.entries.iter_mut() {
            if c.device_id.as_deref() == Some(device_id) {
                c.revoked_ms.get_or_insert(now_ms);
            }
        }
        Some(out)
    })
}

/// Unclaimed invites past their day are dropped: they can no longer pair,
/// and the operator has nothing left to cancel.
fn prune_expired(creds: &mut Credentials, now_ms: u64) {
    creds
        .entries
        .retain(|c| c.device_id.is_some() || c.expires_ms.is_none_or(|at| now_ms < at));
}

/// A short, stable reference to an invite: the first 12 hex digits of its
/// hash. Enough to name it in Pairing, never enough to present.
pub fn invite_ref(entry: &Credential) -> String {
    entry.hash.chars().take(12).collect()
}

/// Invites nobody claimed yet, still able to pair.
pub fn pending_invites(creds: &Credentials, now_ms: u64) -> Vec<&Credential> {
    creds
        .entries
        .iter()
        .filter(|c| {
            c.device_id.is_none()
                && c.revoked_ms.is_none()
                && c.expires_ms.is_none_or(|at| now_ms < at)
        })
        .collect()
}

/// Revoke the pending invites named `query` (case-insensitive) or whose
/// reference starts with it (at least 8 characters). A leaked pair line
/// stops working at once. Returns the invites revoked.
pub fn revoke_invite(store: &Store, query: &str, now_ms: u64) -> io::Result<Vec<Credential>> {
    let q = query.trim().to_string();
    update(store, |_, creds| {
        prune_expired(creds, now_ms);
        let mut out = Vec::new();
        for c in creds.entries.iter_mut() {
            let pending = c.device_id.is_none()
                && c.revoked_ms.is_none()
                && c.expires_ms.is_none_or(|at| now_ms < at);
            let by_name = !q.is_empty() && c.name.eq_ignore_ascii_case(&q);
            let by_ref = q.len() >= 8 && c.hash.starts_with(&q.to_ascii_lowercase());
            if pending && (by_name || by_ref) {
                c.revoked_ms = Some(now_ms);
                out.push(c.clone());
            }
        }
        out
    })
}

/// The device a token's credential belongs to, whatever its state: a
/// refused request is still written to that device's log.
pub fn credential_device(store: &Store, token: &str) -> Option<String> {
    Credentials::load(store)
        .find(&token_hash(token))
        .and_then(|c| c.device_id.clone())
        .filter(|id| valid_device_id(id))
}

/// Why a role change was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RoleError {
    NotFound,
    /// The hub is always Operator.
    Hub,
    Revoked,
}

/// Change a device's role. The caller is responsible for the deliberate
/// confirmation Operator needs.
pub fn set_access(
    store: &Store,
    device_id: &str,
    access: Access,
) -> io::Result<Result<DeviceCard, RoleError>> {
    update(store, |reg, _| {
        let card = reg.get_mut(device_id).ok_or(RoleError::NotFound)?;
        if card.role == DeviceRole::Hub {
            return Err(RoleError::Hub);
        }
        if card.revoked_ms.is_some() {
            return Err(RoleError::Revoked);
        }
        card.access = access;
        Ok(card.clone())
    })
}

/// Whether `device_id` may speak as `session`, and if nobody holds it yet,
/// give it to that device. `seen_elsewhere` says the session already wrote
/// to the hub without being any device's (a session of the hub itself).
pub fn claim_session(
    store: &Store,
    device_id: &str,
    session: &str,
    seen_elsewhere: bool,
) -> io::Result<bool> {
    update(store, |reg, _| {
        if reg
            .devices
            .iter()
            .any(|d| d.id != device_id && d.owns_session(session))
        {
            return false;
        }
        let Some(card) = reg.get_mut(device_id) else {
            return false;
        };
        if card.owns_session(session) {
            return true;
        }
        if seen_elsewhere {
            return false;
        }
        card.sessions.push(session.to_string());
        if card.sessions.len() > DEVICE_SESSIONS_MAX {
            let drop = card.sessions.len() - DEVICE_SESSIONS_MAX;
            card.sessions.drain(..drop);
        }
        true
    })
}

/// Serve's start: the hub's own card is Operator. Cards paired before
/// per-device credentials keep their Workshop default and hold no credential,
/// so the shared secret they were given speaks for none of them.
pub fn migrate(store: &Store, hub_id: &str) -> io::Result<()> {
    update(store, |reg, _| {
        if let Some(card) = reg.get_mut(hub_id) {
            card.access = Access::Operator;
            card.role = DeviceRole::Hub;
        }
    })
}

/// Whether a device holds a live credential (a card paired before
/// per-device credentials holds none).
pub fn has_credential(creds: &Credentials, device_id: &str) -> bool {
    creds
        .entries
        .iter()
        .any(|c| c.device_id.as_deref() == Some(device_id) && c.revoked_ms.is_none())
}

/// A device by id, id prefix (at least 4 characters) or exact name.
pub fn find_device<'a>(reg: &'a DeviceRegistry, query: &str) -> Result<&'a DeviceCard, usize> {
    let q = query.trim();
    if let Some(card) = reg.devices.iter().find(|d| d.id == q) {
        return Ok(card);
    }
    let named: Vec<&DeviceCard> = reg
        .devices
        .iter()
        .filter(|d| d.name.eq_ignore_ascii_case(q))
        .collect();
    if named.len() == 1 {
        return Ok(named[0]);
    }
    let prefixed: Vec<&DeviceCard> = if q.len() >= 4 {
        reg.devices.iter().filter(|d| d.id.starts_with(q)).collect()
    } else {
        Vec::new()
    };
    match (named.len(), prefixed.len()) {
        (_, 1) => Ok(prefixed[0]),
        (n, p) => Err(n.max(p)),
    }
}

// ---------------------------------------------------------------------------
// The per-device log: every remote read and write, by device
// ---------------------------------------------------------------------------

/// `{home}/pairing/`: one `<device id>.jsonl` per device.
pub const LOG_DIR: &str = "pairing";
/// The folder older builds logged to. It exists only so those logs stay
/// readable next to the new ones; nothing writes it any more.
const OLD_LOG_DIR: &str = "equipos";
/// A log past this size keeps only its newest [`LOG_KEEP`] lines.
const LOG_MAX_BYTES: u64 = 128 * 1024;
const LOG_KEEP: usize = 500;

/// One remote request from a device: what, when, and how serve answered.
/// The path only, never the query, a header or a body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub ts_ms: u64,
    pub method: String,
    pub path: String,
    pub status: u16,
    /// Anything but GET or HEAD.
    pub write: bool,
}

impl Action {
    pub fn new(ts_ms: u64, method: &str, path: &str, status: u16) -> Self {
        let path: String = path
            .split('?')
            .next()
            .unwrap_or("")
            .chars()
            .take(200)
            .collect();
        Action {
            ts_ms,
            method: method.to_ascii_uppercase(),
            path,
            status,
            write: !matches!(method.to_ascii_uppercase().as_str(), "GET" | "HEAD"),
        }
    }
}

fn log_dir(store: &Store) -> PathBuf {
    store.root().join(LOG_DIR)
}

/// Append one action to the device's log, trimming it when it grows past
/// 128 KB. An invalid id is never a file name.
pub fn record(store: &Store, device_id: &str, action: &Action) -> io::Result<()> {
    use fs2::FileExt;
    use std::io::Write;
    if !valid_device_id(device_id) {
        return Err(io::Error::other("invalid device id"));
    }
    let dir = log_dir(store);
    fs::create_dir_all(&dir)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(dir.join(".lock"))?;
    lock.lock_exclusive()?;
    let result = (|| {
        let path = dir.join(format!("{device_id}.jsonl"));
        let mut line = serde_json::to_vec(action).map_err(io::Error::other)?;
        line.push(b'\n');
        let mut opts = fs::OpenOptions::new();
        opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
        }
        let mut file = opts.open(&path)?;
        file.write_all(&line)?;
        if file.metadata()?.len() > LOG_MAX_BYTES {
            let kept = read_log(store, device_id);
            let start = kept.len().saturating_sub(LOG_KEEP);
            let mut out = Vec::new();
            for a in &kept[start..] {
                out.extend(serde_json::to_vec(a).map_err(io::Error::other)?);
                out.push(b'\n');
            }
            // The trimmed copy is born 0600, like the log it replaces.
            let tmp = dir.join(format!(".{device_id}.jsonl.tmp.{}", std::process::id()));
            let _ = fs::remove_file(&tmp);
            let mut opts = fs::OpenOptions::new();
            opts.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600).custom_flags(libc::O_NOFOLLOW);
            }
            let written = (|| {
                let mut f = opts.open(&tmp)?;
                f.write_all(&out)?;
                f.sync_all()?;
                fs::rename(&tmp, &path)
            })();
            if written.is_err() {
                let _ = fs::remove_file(&tmp);
            }
            written?;
        }
        Ok(())
    })();
    let _ = FileExt::unlock(&lock);
    result
}

/// The device's log in file order (oldest first). Read by descriptor: no
/// symlink, no FIFO, never more than twice the trim size.
fn read_log(store: &Store, device_id: &str) -> Vec<Action> {
    if !valid_device_id(device_id) {
        return Vec::new();
    }
    let name = format!("{device_id}.jsonl");
    // Older than everything in the new folder, so it comes first.
    [store.root().join(OLD_LOG_DIR), log_dir(store)]
        .iter()
        .flat_map(|dir| {
            crate::safe_fs::read_under(dir, &[], &name, LOG_MAX_BYTES * 2)
                .map(|bytes| {
                    String::from_utf8_lossy(&bytes)
                        .lines()
                        .filter_map(|l| serde_json::from_str::<Action>(l).ok())
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default()
        })
        .collect()
}

/// The newest `limit` actions, newest first.
pub fn actions(store: &Store, device_id: &str, limit: usize) -> Vec<Action> {
    let mut all = read_log(store, device_id);
    all.reverse();
    all.truncate(limit);
    all
}

/// How many reads and writes the log holds.
pub fn action_counts(store: &Store, device_id: &str) -> (usize, usize) {
    let all = read_log(store, device_id);
    let writes = all.iter().filter(|a| a.write).count();
    (all.len() - writes, writes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::device::LocalDevice;
    use crate::test_util::TempHome;

    fn card(id: &str) -> DeviceCard {
        let mut c = LocalDevice {
            id: id.into(),
            name: id.into(),
            role: DeviceRole::Satellite,
        }
        .to_card(0, vec![]);
        c.access = Access::Operator; // what a device might claim about itself
        c
    }

    #[test]
    fn an_invite_pairs_one_device_with_its_own_role() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let token = issue(&store, "borealis", Access::Workshop, 10)
            .unwrap()
            .unwrap();
        assert!(token.starts_with(TOKEN_PREFIX));
        let Verdict::Invite(entry) = verify(&store, &token, 11) else {
            panic!("an unclaimed invite");
        };
        let (paired, new) = claim(&store, &entry.hash, card("d1"), 12).unwrap().unwrap();
        assert!(new);
        assert_eq!(paired.access, Access::Workshop, "never what the card says");
        assert_eq!(paired.created_ms, 12);
        assert!(matches!(verify(&store, &token, 13), Verdict::Device(c) if c.id == "d1"));
        // Another device cannot reuse it.
        assert_eq!(
            claim(&store, &entry.hash, card("d2"), 14).unwrap(),
            Err(ClaimError::Taken)
        );
    }

    #[test]
    fn an_invite_grants_its_workspaces_when_the_device_pairs() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let grants = vec!["work".to_string()];
        let token = issue_granting(&store, "laptop", Access::Workshop, &grants, 0)
            .unwrap()
            .unwrap();
        let mut asked = card("laptop");
        asked.workspaces = vec!["personal".to_string()];
        let (paired, _) = claim(&store, &token_hash(&token), asked, 1)
            .unwrap()
            .unwrap();
        assert_eq!(paired.workspaces, grants, "the hub decides, not the body");
        let saved = DeviceRegistry::load(&store);
        assert_eq!(saved.get("laptop").unwrap().workspaces, grants);
    }

    #[test]
    fn an_invite_never_hands_out_operator_and_expires() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        assert_eq!(
            issue(&store, "x", Access::Operator, 0).unwrap(),
            Err(IssueError::OperatorByInvite)
        );
        let token = issue(&store, "x", Access::Display, 0).unwrap().unwrap();
        assert_eq!(verify(&store, &token, INVITE_TTL_MS), Verdict::Expired);
    }

    #[test]
    fn revocation_is_read_on_the_next_lookup() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let token = issue(&store, "a", Access::Workshop, 0).unwrap().unwrap();
        claim(&store, &token_hash(&token), card("d1"), 1)
            .unwrap()
            .unwrap();
        assert!(revoke(&store, "d1", 2).unwrap().is_some());
        assert_eq!(verify(&store, &token, 3), Verdict::Revoked);
        assert_eq!(
            set_access(&store, "d1", Access::Operator).unwrap(),
            Err(RoleError::Revoked)
        );
    }

    #[test]
    fn the_hub_keeps_hashes_at_0600() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let token = issue(&store, "a", Access::Workshop, 0).unwrap().unwrap();
        let raw = fs::read_to_string(Credentials::path(&store)).unwrap();
        assert!(!raw.contains(&token));
        assert!(raw.contains(&token_hash(&token)));
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(Credentials::path(&store))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn a_session_belongs_to_the_first_device_that_published_it() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        for id in ["d1", "d2"] {
            let token = issue(&store, id, Access::Workshop, 0).unwrap().unwrap();
            claim(&store, &token_hash(&token), card(id), 1)
                .unwrap()
                .unwrap();
        }
        assert!(claim_session(&store, "d1", "s", false).unwrap());
        assert!(claim_session(&store, "d1", "s", false).unwrap());
        assert!(!claim_session(&store, "d2", "s", false).unwrap());
        assert!(!claim_session(&store, "d2", "hub-local", true).unwrap());
    }

    #[test]
    fn a_device_id_never_escapes_its_log_folder() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let token = issue(&store, "a", Access::Workshop, 0).unwrap().unwrap();
        assert_eq!(
            claim(&store, &token_hash(&token), card("../evil"), 1).unwrap(),
            Err(ClaimError::BadId)
        );
        assert!(record(&store, "../evil", &Action::new(0, "GET", "/x", 200)).is_err());
        assert!(!home.path().join("evil.jsonl").exists());
    }

    #[test]
    fn the_log_keeps_reads_and_writes_newest_first_and_stays_bounded() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        record(
            &store,
            "d1",
            &Action::new(1, "GET", "/api/v1/meta?x=secret", 200),
        )
        .unwrap();
        record(&store, "d1", &Action::new(2, "POST", "/api/v1/events", 201)).unwrap();
        let got = actions(&store, "d1", 10);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].path, "/api/v1/events");
        assert!(got[0].write);
        assert_eq!(got[1].path, "/api/v1/meta", "never the query");
        assert_eq!(action_counts(&store, "d1"), (1, 1));
        for i in 0..2_000 {
            record(
                &store,
                "d1",
                &Action::new(10 + i, "GET", "/api/v1/summary", 200),
            )
            .unwrap();
        }
        let len = fs::metadata(home.path().join("pairing/d1.jsonl"))
            .unwrap()
            .len();
        assert!(len <= LOG_MAX_BYTES, "{len}");
        assert_eq!(actions(&store, "d1", 1)[0].ts_ms, 2_009);
    }

    #[test]
    fn re_pairing_drops_the_older_credential() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let first = issue(&store, "a", Access::Workshop, 0).unwrap().unwrap();
        claim(&store, &token_hash(&first), card("d1"), 1)
            .unwrap()
            .unwrap();
        revoke(&store, "d1", 2).unwrap();
        let second = issue(&store, "a", Access::Display, 3).unwrap().unwrap();
        let (c, _) = claim(&store, &token_hash(&second), card("d1"), 4)
            .unwrap()
            .unwrap();
        assert_eq!(c.access, Access::Display);
        assert_eq!(c.revoked_ms, None);
        assert_eq!(verify(&store, &first, 5), Verdict::Revoked);
        assert!(matches!(verify(&store, &second, 5), Verdict::Device(_)));
    }

    #[test]
    fn a_claim_takes_only_what_a_device_may_declare() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let token = issue(&store, "borealis", Access::Workshop, 0)
            .unwrap()
            .unwrap();
        let mut posted = card("d1");
        posted.name = "Mac de Dana (hub)".into();
        posted.sessions = vec!["hub-session".into()];
        posted.revoked_ms = Some(1);
        let (c, new) = claim(&store, &token_hash(&token), posted, 5)
            .unwrap()
            .unwrap();
        assert!(new);
        assert_eq!(c.name, "borealis", "the invite's name, not the card's");
        assert!(c.sessions.is_empty(), "no session comes from the body");
        assert_eq!(
            (c.access, c.created_ms, c.revoked_ms),
            (Access::Workshop, 5, None)
        );
        // A heartbeat neither renames it nor moves its host.
        let mut beat = card("d1");
        beat.name = "otro".into();
        beat.host = "spoofed".into();
        beat.sessions = vec!["hub-session".into()];
        let after = heartbeat(&store, beat, 6).unwrap();
        assert_eq!(
            (after.name.as_str(), after.host.as_str()),
            ("borealis", c.host.as_str())
        );
        assert!(after.sessions.is_empty());
    }

    #[test]
    fn an_invite_never_takes_over_a_live_device_id() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let first = issue(&store, "laptop", Access::Workshop, 0)
            .unwrap()
            .unwrap();
        claim(&store, &token_hash(&first), card("d1"), 1)
            .unwrap()
            .unwrap();
        assert!(claim_session(&store, "d1", "s1", false).unwrap());
        let other = issue(&store, "tv", Access::Display, 2).unwrap().unwrap();
        assert_eq!(
            claim(&store, &token_hash(&other), card("d1"), 3).unwrap(),
            Err(ClaimError::IdTaken)
        );
        assert!(matches!(verify(&store, &first, 4), Verdict::Device(c) if c.owns_session("s1")));
        assert!(
            matches!(verify(&store, &other, 4), Verdict::Invite(_)),
            "the refused invite is not spent"
        );
        // Revoked first, the id re-pairs, with nothing of the old card.
        revoke(&store, "d1", 5).unwrap();
        let (c, _) = claim(&store, &token_hash(&other), card("d1"), 6)
            .unwrap()
            .unwrap();
        assert_eq!((c.name.as_str(), c.access), ("tv", Access::Display));
        assert!(c.sessions.is_empty());
    }

    #[test]
    fn a_pending_invite_is_listed_revoked_by_name_or_ref_and_pruned() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let leaked = issue(&store, "filtrada", Access::Workshop, 0)
            .unwrap()
            .unwrap();
        let other = issue(&store, "otra", Access::Display, 0).unwrap().unwrap();
        let creds = Credentials::load(&store);
        let pending = pending_invites(&creds, 1);
        assert_eq!(pending.len(), 2);
        let other_ref = invite_ref(pending[1]);
        assert_eq!(other_ref.len(), 12);
        assert_eq!(revoke_invite(&store, "FILTRADA", 2).unwrap().len(), 1);
        assert_eq!(verify(&store, &leaked, 3), Verdict::Revoked);
        assert_eq!(
            claim(&store, &token_hash(&leaked), card("x1"), 3).unwrap(),
            Err(ClaimError::Revoked)
        );
        assert!(
            revoke_invite(&store, &other_ref[..7], 2)
                .unwrap()
                .is_empty(),
            "a ref needs 8 characters or more"
        );
        assert_eq!(revoke_invite(&store, &other_ref, 4).unwrap().len(), 1);
        assert_eq!(verify(&store, &other, 5), Verdict::Revoked);
        // A day later both are gone from the file.
        issue(&store, "new", Access::Workshop, INVITE_TTL_MS + 1)
            .unwrap()
            .unwrap();
        let names: Vec<String> = Credentials::load(&store)
            .entries
            .into_iter()
            .map(|c| c.name)
            .collect();
        assert_eq!(names, ["new"]);
    }

    #[test]
    fn a_refused_credential_still_names_its_device() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let token = issue(&store, "a", Access::Workshop, 0).unwrap().unwrap();
        assert_eq!(
            credential_device(&store, &token),
            None,
            "an invite has no device yet"
        );
        claim(&store, &token_hash(&token), card("d1"), 1)
            .unwrap()
            .unwrap();
        revoke(&store, "d1", 2).unwrap();
        assert_eq!(credential_device(&store, &token).as_deref(), Some("d1"));
        assert_eq!(credential_device(&store, "hbd_unknown"), None);
    }

    #[test]
    fn the_trimmed_log_stays_0600() {
        use std::os::unix::fs::PermissionsExt;
        let home = TempHome::new();
        let store = Store::at(home.path());
        for i in 0..2_000 {
            record(&store, "d1", &Action::new(i, "GET", "/api/v1/summary", 200)).unwrap();
        }
        let path = home.path().join("pairing/d1.jsonl");
        assert!(fs::metadata(&path).unwrap().len() <= LOG_MAX_BYTES);
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{mode:o}");
    }
}
