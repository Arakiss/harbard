//! Channel membership (F3): who receives a channel's traffic.
//!
//! ## Why membership is anchored to a checkout, not a session
//!
//! Session ids are born and die constantly — anchoring there would mean every
//! new agent has to remember to re-join, which is exactly the kind of rule that
//! decays. Anchoring to the checkout means one `join` in a
//! repo subscribes every session that ever works there, including sessions that
//! do not exist yet. Session and harness anchors stay available for the cases
//! a checkout does not describe.
//!
//! ## Why a snapshot and not just the log
//!
//! `join`/`leave` are also appended as events, because the narrative belongs in
//! the conversation ("X joined #incidents"). But the log is garbage-collected
//! after `retention_days`, and a subscription must not silently expire with the
//! event that declared it. So the snapshot under `~/.harbard/channels.json` is
//! the authority for delivery, and the events are the story. When they ever
//! disagree, the snapshot wins — it is the one that was written to last.

use crate::clock::now_ms;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::PathBuf;

/// Where agents present themselves. Exists in every workspace.
pub const LOBBY: &str = "lobby";
/// The agents' own open room, for anything that is not work.
pub const COMMONS: &str = "commons";
/// What agents report about harbard itself.
pub const FEEDBACK: &str = "feedback";
/// Rooms every workspace has without anyone creating them.
pub const BUILT_IN_ROOMS: [&str; 3] = [LOBBY, COMMONS, FEEDBACK];

/// True for a room that exists without being created and cannot be removed.
pub fn is_built_in(name: &str) -> bool {
    BUILT_IN_ROOMS.contains(&normalize(name).as_str())
}

/// What a built-in room is for: the English text, and the console's code.
pub fn built_in_purpose(name: &str) -> Option<(&'static str, &'static str)> {
    match normalize(name).as_str() {
        LOBBY => Some(("Where agents present themselves.", "room_purpose_lobby")),
        COMMONS => Some((
            "The agents' own open room, for anything that is not work.",
            "room_purpose_commons",
        )),
        FEEDBACK => Some((
            "What agents report about harbard itself.",
            "room_purpose_feedback",
        )),
        _ => None,
    }
}

fn refuse_built_in(name: &str) -> Result<(), String> {
    if is_built_in(name) {
        return Err(format!(
            "#{} is built in: it cannot be archived, deleted or edited",
            normalize(name)
        ));
    }
    Ok(())
}

/// What a subscription is attached to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AnchorKind {
    /// Every session working in this checkout (the default, and the durable one).
    Repo,
    /// One session, for the length of its life.
    Session,
    /// Every session of a harness.
    Harness,
}

impl AnchorKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AnchorKind::Repo => "repo",
            AnchorKind::Session => "session",
            AnchorKind::Harness => "harness",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "repo" => Some(AnchorKind::Repo),
            "session" => Some(AnchorKind::Session),
            "harness" => Some(AnchorKind::Harness),
            _ => None,
        }
    }
}

/// One subscription: "this anchor receives this channel".
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Member {
    pub kind: AnchorKind,
    pub value: String,
}

/// Per-channel activity anchors for archive suggestions.
///
/// Legacy channels may lack both fields — treat age as unknown and exclude
/// them from eligibility (false negatives beat invented abandonment).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelMeta {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_note_ms: Option<u64>,
    /// Human reason this Room exists. Optional for legacy snapshots.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// Product-surface visibility. This is not a filesystem security boundary.
    #[serde(default)]
    pub visibility: RoomVisibility,
    /// The workspace the operator created this room in. `None` for a room
    /// from before workspaces had rooms: it lives where its checkouts do.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
}

/// How widely a Room appears in Harbard's reading surfaces.
///
/// `Limited` means member-scoped product views plus the operator's audit. A
/// process with direct access to the local append-only store remains outside
/// this boundary, so the product must not call this encrypted or secret.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RoomVisibility {
    #[default]
    Shared,
    Limited,
}

impl RoomVisibility {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "shared" => Some(Self::Shared),
            "limited" => Some(Self::Limited),
            _ => None,
        }
    }
}

/// The durable membership snapshot.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Memberships {
    /// channel name (lowercased) -> its members.
    #[serde(default)]
    pub channels: BTreeMap<String, Vec<Member>>,
    /// Names hidden from the default rail/list. Events stay on the bus;
    /// membership still delivers if anyone remains subscribed. Empty channels
    /// are the usual archive target — hygiene, not soft-delete.
    #[serde(default)]
    pub archived: BTreeSet<String>,
    /// Activity timestamps keyed by normalized channel name.
    #[serde(default)]
    pub meta: BTreeMap<String, ChannelMeta>,
    /// Operator-global read watermark per Room. The local Harbard store has
    /// one operator; persisting here keeps desktop and browser windows aligned.
    #[serde(default)]
    pub reads: BTreeMap<String, String>,
    /// Repositories whose project channel the operator archived, by repository
    /// identity: a project channel id can change, its repository cannot.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub archived_repos: BTreeSet<String>,
}

/// Channel names are lowercase, no whitespace — one name, one channel, however
/// it was typed.
pub fn normalize(name: &str) -> String {
    name.trim()
        .trim_start_matches('#')
        .trim()
        .to_lowercase()
        .replace(char::is_whitespace, "-")
}

/// True for a name that can be addressed and displayed without surprises.
pub fn is_valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 48
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

fn path(store: &Store) -> PathBuf {
    store.root().join("channels.json")
}

/// Read the snapshot; missing or corrupt reads as empty (never a hard failure —
/// a broken membership file must not break a session).
pub fn read(store: &Store) -> Memberships {
    static C: crate::stamped::Shared<Memberships> = crate::stamped::Shared::new(Default::default);
    crate::stamped::json(&C, &path(store))
}

/// Persist the snapshot, creating the store root if needed.
pub fn write(store: &Store, m: &Memberships) -> std::io::Result<()> {
    fs::create_dir_all(store.root())?;
    let json = serde_json::to_string_pretty(m)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    // Write-then-rename so a crash mid-write cannot truncate the file that
    // decides delivery.
    // The temp name carries our pid: a fixed `*.json.tmp` is a shared file,
    // so two processes writing at once clobber each other's staging area and
    // one rename lands a half-written or foreign snapshot.
    let tmp = path(store).with_extension(format!("json.tmp.{}", std::process::id()));
    fs::write(&tmp, json.as_bytes())?;
    fs::rename(&tmp, path(store))
}

/// Add a member to a channel. Returns false when it was already there, so the
/// caller can stay quiet instead of appending a duplicate `join` event.
/// Joining an archived channel unarchives it — somebody is using it again.
pub fn join(m: &mut Memberships, channel: &str, member: Member) -> bool {
    let name = normalize(channel);
    m.archived.remove(&name);
    let is_new = !m.channels.contains_key(&name);
    let entry = m.channels.entry(name.clone()).or_default();
    if entry.contains(&member) {
        return false;
    }
    entry.push(member);
    entry.sort();
    if is_new {
        let meta = m.meta.entry(name).or_default();
        if meta.created_ms.is_none() {
            meta.created_ms = Some(now_ms());
        }
    }
    true
}

/// Remove a member. Returns false when it was not a member. An emptied channel
/// is kept: it exists, it simply has nobody in it — and dropping it would make
/// a channel silently vanish from the console the moment its last member left.
pub fn leave(m: &mut Memberships, channel: &str, member: &Member) -> bool {
    let Some(entry) = m.channels.get_mut(&normalize(channel)) else {
        return false;
    };
    let before = entry.len();
    entry.retain(|x| x != member);
    before != entry.len()
}

/// Create an empty channel. Returns false when it already existed.
///
/// Not the happy path for new rooms — prefer [`join`], which creates and
/// subscribes atomically. Kept for tests and legacy repair of empty names;
/// CLI/serve must not expose orphan create as the recommended verb.
pub fn create(m: &mut Memberships, channel: &str) -> bool {
    let name = normalize(channel);
    if m.channels.contains_key(&name) {
        return false;
    }
    m.archived.remove(&name);
    m.channels.insert(name.clone(), Vec::new());
    let entry = m.meta.entry(name).or_default();
    if entry.created_ms.is_none() {
        entry.created_ms = Some(now_ms());
    }
    true
}

/// True when the named channel exists and has at least one member.
pub fn has_members(m: &Memberships, channel: &str) -> bool {
    m.channels
        .get(&normalize(channel))
        .is_some_and(|members| !members.is_empty())
}

/// Update Room metadata without changing its stable slug or membership.
pub fn update_room(
    m: &mut Memberships,
    channel: &str,
    purpose: Option<&str>,
    visibility: RoomVisibility,
) -> Result<bool, String> {
    let name = normalize(channel);
    refuse_built_in(&name)?;
    if !m.channels.contains_key(&name) {
        return Err(format!("unknown room \"{name}\""));
    }
    let purpose = purpose
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if purpose.as_ref().is_some_and(|value| value.len() > 160) {
        return Err("room purpose must be 160 characters or fewer".to_string());
    }
    let meta = m.meta.entry(name).or_default();
    let changed = meta.purpose != purpose || meta.visibility != visibility;
    meta.purpose = purpose;
    meta.visibility = visibility;
    Ok(changed)
}

/// Advance the operator's Room watermark. ULID lexical order is chronological;
/// stale windows can therefore never move the shared read position backwards.
pub fn mark_read(m: &mut Memberships, channel: &str, event_id: &str) -> Result<bool, String> {
    let name = normalize(channel);
    if !m.channels.contains_key(&name) {
        return Err(format!("unknown room \"{name}\""));
    }
    let event_id = event_id.trim();
    if event_id.len() != 26 || !event_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("room read watermark must be a 26-character event id".to_string());
    }
    if m.reads
        .get(&name)
        .is_some_and(|previous| previous.as_str() >= event_id)
    {
        return Ok(false);
    }
    m.reads.insert(name, event_id.to_string());
    Ok(true)
}

/// Which watermark holds a room's read position. A built-in room exists once
/// per workspace with different messages in each, so its read position does
/// too; `@` cannot appear in a room name, so the key cannot collide with one.
pub fn read_key(channel: &str, workspace: Option<&str>) -> String {
    let name = normalize(channel);
    match workspace {
        Some(id) if is_built_in(&name) => format!("{name}@{id}"),
        _ => name,
    }
}

/// [`mark_read`] for a room in a workspace. A built-in room needs no entry in
/// the snapshot, since it exists without being created.
pub fn mark_room_read(
    m: &mut Memberships,
    channel: &str,
    workspace: Option<&str>,
    event_id: &str,
) -> Result<bool, String> {
    let name = normalize(channel);
    if !is_built_in(&name) && !m.channels.contains_key(&name) {
        return Err(format!("unknown room \"{name}\""));
    }
    let event_id = event_id.trim();
    if event_id.len() != 26 || !event_id.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("room read watermark must be a 26-character event id".to_string());
    }
    let key = read_key(&name, workspace);
    if m.reads
        .get(&key)
        .is_some_and(|previous| previous.as_str() >= event_id)
    {
        return Ok(false);
    }
    m.reads.insert(key, event_id.to_string());
    Ok(true)
}

/// Hygiene state for a named channel (doctor / console rail).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelHygiene {
    /// Zero members, zero notes — invalid for new creates; legacy repair.
    Orphan,
    /// One or more members, never received a note.
    Armed,
    /// At least one note addressed to the channel.
    Active,
    /// Had activity, and none since the inactivity threshold.
    Dormant,
    /// Hidden from the default rail.
    Archived,
    /// Empty, no usable timestamps — exclude from age-based suggestions.
    LegacyUnknownAge,
}

/// Default grace before an armed channel earns a non-blocking `channel_unused`
/// correction (24h).
pub const CHANNEL_UNUSED_GRACE_MS: u64 = 24 * 60 * 60 * 1000;

/// Classify one named channel for doctor / rail presentation.
pub fn classify_channel(
    m: &Memberships,
    name: &str,
    inactive_days: u64,
    now: u64,
) -> Option<ChannelHygiene> {
    let name = normalize(name);
    let members = m.channels.get(&name)?;
    if m.archived.contains(&name) {
        return Some(ChannelHygiene::Archived);
    }
    let meta = m.meta.get(&name).cloned().unwrap_or_default();
    let member_count = members.len();
    let had_note = meta.last_note_ms.is_some();
    if member_count == 0 && !had_note {
        return Some(match meta.created_ms {
            Some(_) => ChannelHygiene::Orphan,
            None => ChannelHygiene::LegacyUnknownAge,
        });
    }
    if !had_note {
        return Some(ChannelHygiene::Armed);
    }

    // Dormancy is decided by silence, not by an empty member list.
    //
    // It used to require zero members, and that made it unreachable for every
    // repo-anchored channel. A channel anchored to its repo outlives the session
    // that opened it, and a repo anchor never leaves, so `member_count` never
    // reaches zero and the channel could not age out.
    //
    // An anchor is an address, not attendance. Whether anyone is listening is a
    // question about live sessions, which this module cannot see and does not
    // need to: silence is the evidence, and silence is already recorded here.
    let threshold_ms = inactive_days.saturating_mul(86_400_000);
    match meta.last_note_ms.or(meta.created_ms) {
        Some(t) if now.saturating_sub(t) >= threshold_ms => Some(ChannelHygiene::Dormant),
        Some(_) => Some(ChannelHygiene::Active),
        None => Some(ChannelHygiene::LegacyUnknownAge),
    }
}

/// The checkout a channel is merely an alias of, when it is one.
///
/// A channel earns its name by describing an audience a checkout cannot. When
/// every member is a `repo` anchor on the SAME checkout, the room delivers to
/// exactly the set `--to repo:<path>` already reaches — and charges a `join`
/// to remember plus a zero-member failure mode the repo target does not have.
/// Same audience, strictly more ways to fail.
///
/// Returns the checkout when the room is an alias, `None` when the membership
/// genuinely exceeds one repo (several checkouts, a harness, or a session) and
/// `None` for an empty room, which has a different problem and its own code.
pub fn repo_alias(m: &Memberships, channel: &str) -> Option<String> {
    let name = normalize(channel);
    let members = m.channels.get(&name)?;
    let (first, rest) = members.split_first()?;
    if first.kind != AnchorKind::Repo {
        return None;
    }
    if rest
        .iter()
        .any(|other| other.kind != AnchorKind::Repo || other.value != first.value)
    {
        return None;
    }
    Some(first.value.clone())
}

/// Every channel that is an alias of a single checkout, name and checkout.
pub fn repo_alias_channels(m: &Memberships) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = m
        .channels
        .keys()
        .filter(|name| !m.archived.contains(*name))
        .filter_map(|name| repo_alias(m, name).map(|repo| (name.clone(), repo)))
        .collect();
    out.sort();
    out
}

/// Aggregate hygiene buckets for doctor (diagnose only — never auto-archive).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelHygieneReport {
    pub orphan: Vec<String>,
    pub armed: Vec<String>,
    pub dormant: Vec<String>,
    pub legacy_unknown_age: Vec<String>,
    /// Armed channels past grace that may earn `channel_unused`.
    pub armed_past_grace: Vec<String>,
    /// Live channels whose whole membership is one repo anchor: `(name, repo)`.
    /// They deliver exactly what `--to repo:<path>` delivers, so every note
    /// sent there pays a subscription for an audience it already had.
    pub repo_alias: Vec<(String, String)>,
}

/// Scan membership into hygiene buckets.
pub fn hygiene_report(
    m: &Memberships,
    inactive_days: u64,
    now: u64,
    unused_grace_ms: u64,
) -> ChannelHygieneReport {
    let mut report = ChannelHygieneReport::default();
    // A built-in room exists whether or not anyone is in it: nothing to report.
    for name in m.channels.keys().filter(|name| !is_built_in(name)) {
        match classify_channel(m, name, inactive_days, now) {
            Some(ChannelHygiene::Orphan) => report.orphan.push(name.clone()),
            Some(ChannelHygiene::Armed) => {
                report.armed.push(name.clone());
                let meta = m.meta.get(name).cloned().unwrap_or_default();
                // A missing birth date used to grant permanent immunity: the
                // channel could never age into a suggestion, so it sat on the
                // rail forever.
                // No date AND no note means it predates the bookkeeping that
                // would have dated it, which makes it older than any grace,
                // not younger.
                let past_grace = meta
                    .created_ms
                    .is_none_or(|created| now.saturating_sub(created) >= unused_grace_ms);
                if past_grace {
                    report.armed_past_grace.push(name.clone());
                }
            }
            Some(ChannelHygiene::Dormant) => report.dormant.push(name.clone()),
            Some(ChannelHygiene::LegacyUnknownAge) => report.legacy_unknown_age.push(name.clone()),
            _ => {}
        }
    }
    report.orphan.sort();
    report.armed.sort();
    report.dormant.sort();
    report.legacy_unknown_age.sort();
    report.armed_past_grace.sort();
    report.repo_alias = repo_alias_channels(m);
    report
}

/// Resolution id for one channel's unused correction (stable, one per channel).
pub fn channel_unused_resolution_id(channel: &str) -> String {
    format!("channel_unused:{}", normalize(channel))
}

/// Fill in each channel's last note from the event log, which is the record
/// that cannot be wrong.
///
/// `last_note_ms` is maintained incrementally as notes are written, and an
/// incrementally-maintained number is a second source of truth: it can only be
/// as old as the code that started writing it, and a channel that never
/// registers a note can never age out of anything.
///
/// Scans newest segment first and stops as soon as reading further cannot
/// change an answer: every channel resolved, or the segments have aged past
/// the horizon, beyond which the exact date no longer matters because
/// everything unresolved is dormant either way.
pub fn reconcile_activity(
    m: &mut Memberships,
    store: &crate::store::Store,
    horizon_ms: u64,
    now: u64,
) -> usize {
    let mut pending: std::collections::BTreeSet<String> = m
        .channels
        .keys()
        .filter(|name| {
            m.meta
                .get(*name)
                .is_none_or(|meta| meta.last_note_ms.is_none())
        })
        .cloned()
        .collect();
    if pending.is_empty() {
        return 0;
    }

    let mut healed = 0usize;
    for segment in store.segments().into_iter().rev() {
        if pending.is_empty() {
            break;
        }
        let Ok(body) = std::fs::read_to_string(&segment) else {
            continue;
        };
        let mut newest_in_segment = 0u64;
        for line in body.lines() {
            let Ok(event) = serde_json::from_str::<crate::event::Event>(line) else {
                continue;
            };
            let at = crate::query::event_ms(&event);
            newest_in_segment = newest_in_segment.max(at);
            if event.event != crate::event::EventKind::Note {
                continue;
            }
            let Some(to) = event.to.as_ref() else {
                continue;
            };
            if to.kind != crate::event::RecipientKind::Channel {
                continue;
            }
            let name = normalize(&to.value);
            if !pending.contains(&name) {
                continue;
            }
            let meta = m.meta.entry(name.clone()).or_default();
            if meta.created_ms.is_none() {
                meta.created_ms = Some(at);
            }
            meta.last_note_ms = Some(at.max(meta.last_note_ms.unwrap_or(0)));
        }
        // Anything found in this segment is now as recent as it will get:
        // later segments were already read.
        let resolved: Vec<String> = pending
            .iter()
            .filter(|name| {
                m.meta
                    .get(*name)
                    .is_some_and(|meta| meta.last_note_ms.is_some())
            })
            .cloned()
            .collect();
        healed += resolved.len();
        for name in resolved {
            pending.remove(&name);
        }
        if newest_in_segment > 0 && now.saturating_sub(newest_in_segment) >= horizon_ms {
            // Past the horizon the exact date stops mattering: whatever is
            // still unresolved is dormant on any reading of it.
            break;
        }
    }
    healed
}

/// Record that a note was addressed to `channel:<name>`.
pub fn touch_note_activity(m: &mut Memberships, channel: &str, at_ms: u64) {
    let name = normalize(channel);
    if name.is_empty() || !m.channels.contains_key(&name) {
        // Still record activity for unknown names so a later create/join can
        // inherit last_note_ms if we seed the channel into the map.
        if name.is_empty() {
            return;
        }
        m.channels.entry(name.clone()).or_default();
    }
    let entry = m.meta.entry(name).or_default();
    if entry.created_ms.is_none() {
        entry.created_ms = Some(at_ms);
    }
    entry.last_note_ms = Some(at_ms.max(entry.last_note_ms.unwrap_or(0)));
}

/// Archive a channel: drop it from the default rail/list. Refuses when it still
/// has members — leave them first. Events and the empty name remain.
pub fn archive(m: &mut Memberships, channel: &str) -> Result<(), String> {
    let name = normalize(channel);
    refuse_built_in(&name)?;
    let Some(members) = m.channels.get(&name) else {
        return Err(format!("unknown channel \"{name}\""));
    };
    if !members.is_empty() {
        return Err(format!(
            "#{name} still has {} member(s) — `harbard leave` them first",
            members.len()
        ));
    }
    m.archived.insert(name);
    Ok(())
}

/// Archive a room from the console: whoever was subscribed is unsubscribed,
/// and is returned so the caller can tell the story on the log.
pub fn archive_unsubscribing(m: &mut Memberships, channel: &str) -> Result<Vec<Member>, String> {
    let name = normalize(channel);
    refuse_built_in(&name)?;
    let Some(members) = m.channels.get_mut(&name) else {
        return Err(format!("unknown channel \"{name}\""));
    };
    let gone = std::mem::take(members);
    m.archived.insert(name);
    Ok(gone)
}

/// Bring an archived channel back onto the default rail.
pub fn unarchive(m: &mut Memberships, channel: &str) -> Result<(), String> {
    let name = normalize(channel);
    if !m.channels.contains_key(&name) {
        return Err(format!("unknown channel \"{name}\""));
    }
    if !m.archived.remove(&name) {
        return Err(format!("#{name} is not archived"));
    }
    Ok(())
}

/// Permanently remove an empty archived channel from the membership snapshot.
/// Historical bus events remain append-only and are never touched.
pub fn delete_archived(m: &mut Memberships, channel: &str) -> Result<(), String> {
    let name = normalize(channel);
    refuse_built_in(&name)?;
    let Some(members) = m.channels.get(&name) else {
        return Err(format!("unknown channel \"{name}\""));
    };
    if !members.is_empty() {
        return Err(format!(
            "#{name} still has {} member(s) — `harbard leave` them first",
            members.len()
        ));
    }
    if !m.archived.contains(&name) {
        return Err(format!(
            "#{name} is not archived — `harbard channels --archive {name}` first"
        ));
    }
    m.channels.remove(&name);
    m.archived.remove(&name);
    m.meta.remove(&name);
    m.reads.remove(&name);
    Ok(())
}

/// True when the channel name is in the archived set.
pub fn is_archived(m: &Memberships, channel: &str) -> bool {
    m.archived.contains(&normalize(channel))
}

/// Why a channel is not an archive candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IneligibleReason {
    HasMembers,
    AlreadyArchived,
    UnknownAge,
    ActiveRecently,
}

/// One named channel evaluated for archive suggestions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveCandidate {
    pub name: String,
    pub member_count: usize,
    pub created_ms: Option<u64>,
    pub last_note_ms: Option<u64>,
    pub inactive_days: Option<u64>,
    pub eligible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ineligible_reason: Option<IneligibleReason>,
}

/// Evaluate archive eligibility for every named channel.
///
/// Eligible = empty members, not archived, and inactivity older than
/// `inactive_days`. Inactivity anchors on `last_note_ms`, falling back to
/// `created_ms` when the room was never messaged. Only legacy rows with
/// neither timestamp stay `unknown_age` (false negatives beat invented age).
pub fn archive_candidates(m: &Memberships, inactive_days: u64, now: u64) -> Vec<ArchiveCandidate> {
    let threshold_ms = inactive_days.saturating_mul(86_400_000);
    let mut out: Vec<ArchiveCandidate> = m
        .channels
        .iter()
        .filter(|(name, _)| !is_built_in(name))
        .map(|(name, members)| {
            let meta = m.meta.get(name).cloned().unwrap_or_default();
            let member_count = members.len();
            let archived = m.archived.contains(name);
            let (eligible, reason, inactive) = if archived {
                (false, Some(IneligibleReason::AlreadyArchived), None)
            } else if member_count > 0 {
                (false, Some(IneligibleReason::HasMembers), None)
            } else {
                match meta.last_note_ms.or(meta.created_ms) {
                    None => (false, Some(IneligibleReason::UnknownAge), None),
                    Some(anchor) => {
                        let age = now.saturating_sub(anchor);
                        let days = age / 86_400_000;
                        if age >= threshold_ms {
                            (true, None, Some(days))
                        } else {
                            (false, Some(IneligibleReason::ActiveRecently), Some(days))
                        }
                    }
                }
            };
            ArchiveCandidate {
                name: name.clone(),
                member_count,
                created_ms: meta.created_ms,
                last_note_ms: meta.last_note_ms,
                inactive_days: inactive,
                eligible,
                ineligible_reason: reason,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.eligible
            .cmp(&a.eligible)
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// Result of one channel in a bulk archive attempt.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BulkArchiveStatus {
    Archived,
    Skipped,
    Conflict,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BulkArchiveItem {
    pub name: String,
    pub status: BulkArchiveStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Archive many empty channels in one serialized membership write.
/// Revalidates each name; never force-archives non-empty channels.
pub fn archive_bulk(m: &mut Memberships, names: &[String]) -> Vec<BulkArchiveItem> {
    let mut items = Vec::with_capacity(names.len());
    for raw in names {
        let name = normalize(raw);
        if name.is_empty() {
            items.push(BulkArchiveItem {
                name: raw.clone(),
                status: BulkArchiveStatus::Skipped,
                reason: Some("empty name".into()),
            });
            continue;
        }
        match archive(m, &name) {
            Ok(()) => items.push(BulkArchiveItem {
                name,
                status: BulkArchiveStatus::Archived,
                reason: None,
            }),
            Err(e) => {
                let status = if e.contains("member") {
                    BulkArchiveStatus::Conflict
                } else {
                    BulkArchiveStatus::Skipped
                };
                items.push(BulkArchiveItem {
                    name,
                    status,
                    reason: Some(e),
                });
            }
        }
    }
    items
}

/// Every channel this session receives, resolved from its anchors.
pub fn channels_for(
    m: &Memberships,
    session: &str,
    harness: &str,
    repo: Option<&str>,
) -> Vec<String> {
    let mut out: Vec<String> = m
        .channels
        .iter()
        .filter(|(_, members)| {
            members.iter().any(|mem| match mem.kind {
                AnchorKind::Session => mem.value == session,
                AnchorKind::Harness => mem.value == harness,
                AnchorKind::Repo => repo.is_some_and(|r| {
                    // One `join` covers the checkout and every worktree under it
                    // — worktrees are the same project on another path string.
                    r == mem.value || r.starts_with(&format!("{}/", mem.value))
                }),
            })
        })
        .map(|(name, _)| name.clone())
        .collect();
    out.sort();
    out
}

/// Rooms discoverable from one agent context. Shared Rooms are listed to the
/// fleet; Limited Rooms are listed only when this context is a member. The
/// operator audit may deliberately bypass this projection.
pub fn visible_rooms_for(
    m: &Memberships,
    session: &str,
    harness: &str,
    repo: Option<&str>,
) -> Vec<String> {
    let mine: BTreeSet<String> = channels_for(m, session, harness, repo)
        .into_iter()
        .collect();
    m.channels
        .keys()
        .filter(|name| {
            m.meta
                .get(*name)
                .is_none_or(|meta| meta.visibility == RoomVisibility::Shared)
                || mine.contains(*name)
        })
        .cloned()
        .collect()
}

/// The workspaces a named room lives in: where the operator created it, and
/// where its checkouts are; the default workspace when nothing says.
///
/// `home` keeps placement away from the protected folders, where even reading
/// a `.git` could raise a consent dialog.
pub fn room_workspaces(
    m: &Memberships,
    channel: &str,
    placement: &crate::workspaces::Placement,
    home: &str,
) -> BTreeSet<String> {
    let name = normalize(channel);
    let mut out = BTreeSet::new();
    if let Some(id) = m.meta.get(&name).and_then(|meta| meta.workspace.clone()) {
        out.insert(id);
    }
    for member in m.channels.get(&name).into_iter().flatten() {
        if member.kind == AnchorKind::Repo && !crate::workspace::protected(&member.value, home) {
            out.insert(placement.of_repo(Some(&member.value)));
        }
    }
    if out.is_empty() {
        out.insert(placement.default_id().to_string());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempHome;

    fn repo(v: &str) -> Member {
        Member {
            kind: AnchorKind::Repo,
            value: v.to_string(),
        }
    }

    #[test]
    fn names_normalize_to_one_channel() {
        assert_eq!(normalize("  #Incidents "), "incidents");
        assert_eq!(normalize("Deploy Train"), "deploy-train");
        assert!(is_valid_name("incidents"));
        assert!(!is_valid_name(""));
        assert!(!is_valid_name("has spaces"));
        assert!(!is_valid_name("emoji🔥"));
    }

    #[test]
    fn a_checkout_join_covers_sessions_that_do_not_exist_yet() {
        // The whole point of anchoring to the repo: nobody has to remember.
        let mut m = Memberships::default();
        assert!(join(&mut m, "#Incidents", repo("/r")));
        assert!(!join(&mut m, "incidents", repo("/r")), "idempotent");

        let future = channels_for(&m, "session-born-tomorrow", "codex", Some("/r"));
        assert_eq!(future, vec!["incidents"]);
        // A session elsewhere hears nothing.
        assert!(channels_for(&m, "other", "codex", Some("/elsewhere")).is_empty());
        // A worktree under the joined checkout is the same room.
        assert_eq!(
            channels_for(&m, "wt", "codex", Some("/r/.claude/worktrees/feat")),
            vec!["incidents"]
        );
        // A sibling path that merely shares a prefix must NOT inherit.
        assert!(channels_for(&m, "sib", "codex", Some("/r-ci")).is_empty());
    }

    #[test]
    fn a_room_subscribed_by_one_checkout_is_that_checkout() {
        // The failure this names is not misuse, it is redundancy: `#deploys`
        // joined only from /r delivers to the same sessions `--to repo:/r`
        // delivers to.
        let mut m = Memberships::default();
        join(&mut m, "deploys", repo("/r"));
        assert_eq!(repo_alias(&m, "deploys").as_deref(), Some("/r"));
        assert_eq!(
            channels_for(&m, "s", "codex", Some("/r")),
            vec!["deploys"],
            "the alias still delivers — this is advice, not a refusal"
        );

        // A second checkout is an audience no single repo target can name.
        join(&mut m, "deploys", repo("/other"));
        assert!(repo_alias(&m, "deploys").is_none());

        // So is a harness anchor.
        let mut h = Memberships::default();
        join(&mut h, "adversarial", repo("/r"));
        join(
            &mut h,
            "adversarial",
            Member {
                kind: AnchorKind::Harness,
                value: "codex".into(),
            },
        );
        assert!(repo_alias(&h, "adversarial").is_none());

        // An empty room is a different diagnosis (channel_no_recipients) and
        // must not be reported as an alias of nothing.
        let mut e = Memberships::default();
        join(&mut e, "empty", repo("/r"));
        leave(&mut e, "empty", &repo("/r"));
        assert!(repo_alias(&e, "empty").is_none());
        assert!(repo_alias(&e, "never-existed").is_none());
    }

    #[test]
    fn hygiene_lists_alias_rooms_but_never_archived_ones() {
        let mut m = Memberships::default();
        join(&mut m, "alias", repo("/r"));
        join(&mut m, "wide", repo("/r"));
        join(&mut m, "wide", repo("/r2"));
        join(&mut m, "hidden", repo("/r"));
        leave(&mut m, "hidden", &repo("/r"));
        archive(&mut m, "hidden").expect("empty room archives");
        // Re-subscribing an archived room must not resurrect it in the report.
        join(&mut m, "hidden", repo("/r"));
        m.archived.insert("hidden".into());

        let report = hygiene_report(&m, 7, 10_000, 0);
        assert_eq!(report.repo_alias, vec![("alias".into(), "/r".into())]);
    }

    #[test]
    fn leaving_keeps_the_channel_alive() {
        let mut m = Memberships::default();
        join(&mut m, "ops", repo("/r"));
        assert!(leave(&mut m, "ops", &repo("/r")));
        assert!(!leave(&mut m, "ops", &repo("/r")), "already gone");
        assert!(
            m.channels.contains_key("ops"),
            "an empty channel still exists"
        );
        assert!(channels_for(&m, "s", "codex", Some("/r")).is_empty());
    }

    #[test]
    fn anchors_resolve_independently() {
        let mut m = Memberships::default();
        join(&mut m, "ops", repo("/r"));
        join(
            &mut m,
            "ops",
            Member {
                kind: AnchorKind::Harness,
                value: "codex".to_string(),
            },
        );
        // Either anchor is enough.
        assert_eq!(channels_for(&m, "s", "codex", None), vec!["ops"]);
        assert_eq!(
            channels_for(&m, "s", "claude-code", Some("/r")),
            vec!["ops"]
        );
        assert!(channels_for(&m, "s", "claude-code", Some("/x")).is_empty());
    }

    #[test]
    fn limited_rooms_are_discoverable_only_by_members() {
        let mut m = Memberships::default();
        join(&mut m, "shared", repo("/r"));
        join(&mut m, "private-ops", repo("/r"));
        update_room(&mut m, "private-ops", None, RoomVisibility::Limited).unwrap();

        assert_eq!(
            visible_rooms_for(&m, "member", "codex", Some("/r")),
            vec!["private-ops", "shared"]
        );
        assert_eq!(
            visible_rooms_for(&m, "outsider", "codex", Some("/elsewhere")),
            vec!["shared"]
        );
    }

    #[test]
    fn snapshot_round_trips_and_survives_corruption() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let mut m = Memberships::default();
        join(&mut m, "incidents", repo("/r"));
        write(&store, &m).unwrap();
        assert_eq!(read(&store).channels["incidents"], vec![repo("/r")]);

        // A corrupt file reads as empty rather than breaking a session.
        std::fs::write(store.root().join("channels.json"), b"{not json").unwrap();
        assert!(read(&store).channels.is_empty());
    }

    #[test]
    fn legacy_snapshots_gain_room_defaults_without_migration() {
        let json = r#"{
          "channels": {"incidents": [{"kind": "repo", "value": "/r"}]},
          "meta": {"incidents": {"created_ms": 42}}
        }"#;

        let m: Memberships = serde_json::from_str(json).unwrap();
        let meta = &m.meta["incidents"];
        assert_eq!(meta.purpose, None);
        assert_eq!(meta.visibility, RoomVisibility::Shared);
        assert!(m.reads.is_empty());
    }

    #[test]
    fn room_metadata_is_bounded_and_idempotent() {
        let mut m = Memberships::default();
        join(&mut m, "incidents", repo("/r"));

        assert!(
            update_room(
                &mut m,
                "incidents",
                Some("  Coordinate production incidents  "),
                RoomVisibility::Limited,
            )
            .unwrap()
        );
        let meta = &m.meta["incidents"];
        assert_eq!(
            meta.purpose.as_deref(),
            Some("Coordinate production incidents")
        );
        assert_eq!(meta.visibility, RoomVisibility::Limited);
        assert!(
            !update_room(
                &mut m,
                "incidents",
                Some("Coordinate production incidents"),
                RoomVisibility::Limited,
            )
            .unwrap(),
            "the same metadata must not create another write"
        );

        assert!(
            update_room(
                &mut m,
                "incidents",
                Some(&"x".repeat(161)),
                RoomVisibility::Shared,
            )
            .is_err()
        );
        assert!(update_room(&mut m, "missing", None, RoomVisibility::Shared).is_err());
    }

    #[test]
    fn room_read_watermarks_only_move_forward() {
        let mut m = Memberships::default();
        join(&mut m, "incidents", repo("/r"));
        let first = "01J00000000000000000000000";
        let later = "01J00000000000000000000001";

        assert!(mark_read(&mut m, "incidents", first).unwrap());
        assert!(!mark_read(&mut m, "incidents", first).unwrap());
        assert!(mark_read(&mut m, "incidents", later).unwrap());
        assert!(!mark_read(&mut m, "incidents", first).unwrap());
        assert_eq!(m.reads["incidents"], later);
        assert!(mark_read(&mut m, "incidents", "not-an-event").is_err());
        assert!(mark_read(&mut m, "missing", later).is_err());
    }

    #[test]
    fn archive_empty_only_and_join_unarchives() {
        let mut m = Memberships::default();
        create(&mut m, "ghosts");
        assert!(archive(&mut m, "ghosts").is_ok());
        assert!(is_archived(&m, "ghosts"));
        assert!(archive(&mut m, "ghosts").is_ok(), "idempotent");

        join(&mut m, "busy", repo("/r"));
        assert!(archive(&mut m, "busy").is_err());

        // Re-join ghosts → back on the rail.
        assert!(join(&mut m, "ghosts", repo("/r")));
        assert!(!is_archived(&m, "ghosts"));
        leave(&mut m, "ghosts", &repo("/r"));
        archive(&mut m, "ghosts").unwrap();
        unarchive(&mut m, "ghosts").unwrap();
        assert!(!is_archived(&m, "ghosts"));
    }

    #[test]
    fn delete_requires_empty_archived_channel_and_preserves_other_rooms() {
        let mut m = Memberships::default();
        create(&mut m, "obsolete");
        create(&mut m, "active");
        join(&mut m, "active", repo("/r"));
        m.reads
            .insert("obsolete".into(), "01J00000000000000000000000".into());

        assert!(delete_archived(&mut m, "obsolete").is_err());
        archive(&mut m, "obsolete").unwrap();
        delete_archived(&mut m, "obsolete").unwrap();

        assert!(!m.channels.contains_key("obsolete"));
        assert!(!m.archived.contains("obsolete"));
        assert!(!m.meta.contains_key("obsolete"));
        assert!(!m.reads.contains_key("obsolete"));
        assert!(m.channels.contains_key("active"));
        assert!(delete_archived(&mut m, "active").is_err());
    }

    #[test]
    fn archive_candidates_use_created_ms_when_never_noted() {
        let mut m = Memberships::default();
        // Legacy: neither timestamp → unknown age (false negative).
        create(&mut m, "legacy");
        m.meta.get_mut("legacy").unwrap().created_ms = None;
        m.meta.get_mut("legacy").unwrap().last_note_ms = None;
        // Never-used but stamped at create → inactivity from created_ms.
        create(&mut m, "never-used");
        m.meta.get_mut("never-used").unwrap().created_ms = Some(1_000);
        m.meta.get_mut("never-used").unwrap().last_note_ms = None;
        create(&mut m, "stale");
        touch_note_activity(&mut m, "stale", 1_000);
        create(&mut m, "fresh");
        touch_note_activity(&mut m, "fresh", 1_000 + 86_400_000 * 9); // 1d ago
        join(&mut m, "busy", repo("/r"));
        touch_note_activity(&mut m, "busy", 1);

        let now = 1_000 + 86_400_000 * 10;
        let cands = archive_candidates(&m, 7, now);
        let by: BTreeMap<_, _> = cands.into_iter().map(|c| (c.name.clone(), c)).collect();
        assert!(!by["legacy"].eligible);
        assert_eq!(
            by["legacy"].ineligible_reason,
            Some(IneligibleReason::UnknownAge)
        );
        assert!(
            by["never-used"].eligible,
            "created_ms fallback should age never-used empties"
        );
        assert!(by["stale"].eligible);
        assert!(!by["fresh"].eligible);
        assert!(!by["busy"].eligible);
        assert_eq!(
            by["busy"].ineligible_reason,
            Some(IneligibleReason::HasMembers)
        );
    }

    #[test]
    fn hygiene_classifies_orphan_armed_active() {
        let mut m = Memberships::default();
        create(&mut m, "ghost");
        join(&mut m, "ready", repo("/r"));
        join(&mut m, "talk", repo("/r"));
        touch_note_activity(&mut m, "talk", 5_000);
        let now = 5_000 + CHANNEL_UNUSED_GRACE_MS + 1;
        // Backdate armed create so it is past grace.
        m.meta.get_mut("ready").unwrap().created_ms = Some(1);
        let report = hygiene_report(&m, 7, now, CHANNEL_UNUSED_GRACE_MS);
        assert_eq!(report.orphan, vec!["ghost"]);
        assert_eq!(report.armed, vec!["ready"]);
        assert_eq!(report.armed_past_grace, vec!["ready"]);

        // A channel with members, no note ever, and no recorded birth date is
        // the shape 18 of 19 real channels had. It must age out like any
        // other; the missing field is not a shield.
        let mut undated = Memberships::default();
        join(
            &mut undated,
            "legacy",
            Member {
                kind: AnchorKind::Session,
                value: "s1".into(),
            },
        );
        undated.meta.remove("legacy");
        let undated_report = hygiene_report(&undated, 7, now, CHANNEL_UNUSED_GRACE_MS);
        assert_eq!(undated_report.armed, vec!["legacy"]);
        assert_eq!(
            undated_report.armed_past_grace,
            vec!["legacy"],
            "an undated, never-used channel cannot be immortal"
        );
        assert!(classify_channel(&m, "talk", 7, now) == Some(ChannelHygiene::Active));
    }

    /// The common shape, and the one dormancy could not see: anchored to its
    /// repo, worked hard for a day, silent ever since. The anchor never leaves,
    /// so a rule that waited for an empty member list waited forever.
    #[test]
    fn a_repo_anchored_channel_goes_dormant_on_silence_alone() {
        let mut m = Memberships::default();
        join(&mut m, "sprint", repo("/work/northwind"));
        touch_note_activity(&mut m, "sprint", 1_000);

        let day = 86_400_000u64;
        // Still within the window: worked yesterday, still the live topic.
        let report = hygiene_report(&m, 7, 1_000 + 6 * day, CHANNEL_UNUSED_GRACE_MS);
        assert!(report.dormant.is_empty());

        // Past it: the anchor is still there and still not listening.
        let report = hygiene_report(&m, 7, 1_000 + 8 * day, CHANNEL_UNUSED_GRACE_MS);
        assert_eq!(report.dormant, vec!["sprint"]);
        assert!(
            has_members(&m, "sprint"),
            "dormancy must not unsubscribe anyone"
        );
    }

    /// A channel that was worked hard before the activity counter existed must
    /// still be judged on what it did, not on what the counter remembers.
    #[test]
    fn activity_is_reconciled_from_the_log_the_counter_missed() {
        use crate::event::{Event, EventKind, Recipient, RecipientKind};
        let home = crate::test_util::TempHome::new();
        let store = crate::store::Store::at(home.path());
        let day = 86_400_000u64;

        let note = |channel: &str, at: u64| {
            store
                .append(&Event {
                    v: crate::event::SCHEMA_VERSION,
                    id: ulid::Ulid::from_parts(at, 0).to_string(),
                    ts: crate::clock::rfc3339_utc(at),
                    event: EventKind::Note,
                    session: "s1".into(),
                    harness: "codex".into(),
                    host: "h".into(),
                    cwd: "/tmp".into(),
                    repo: Some("/r".into()),
                    repo_key: None,
                    branch: None,
                    role: crate::event::Role::Main,
                    parent: None,
                    note_ref: None,
                    text: "…".into(),
                    to: Some(Recipient {
                        kind: RecipientKind::Channel,
                        value: channel.to_string(),
                    }),
                    kind: crate::event::NoteKind::Fyi,
                })
                .expect("append");
        };
        note("worked", 10 * day);
        note("worked", 12 * day);

        let mut m = Memberships::default();
        join(&mut m, "worked", repo("/r"));
        join(&mut m, "never", repo("/r"));
        // Exactly the state found in the wild: subscribed, busy, no metadata.
        m.meta.clear();

        let now = 13 * day;
        let healed = reconcile_activity(&mut m, &store, 7 * day, now);
        assert_eq!(healed, 1, "only the channel with notes can be healed");
        assert_eq!(
            m.meta.get("worked").and_then(|x| x.last_note_ms),
            Some(12 * day)
        );
        assert_eq!(
            classify_channel(&m, "worked", 7, now),
            Some(ChannelHygiene::Active),
            "a channel noted yesterday is not armed"
        );
        assert_eq!(
            classify_channel(&m, "never", 7, now),
            Some(ChannelHygiene::Armed),
            "a channel that truly never received a note stays armed"
        );

        // And once the traffic ages out, silence carries it to dormant.
        assert_eq!(
            classify_channel(&m, "worked", 7, 20 * day),
            Some(ChannelHygiene::Dormant)
        );
    }

    /// Silence is the only evidence dormancy uses, so an anchor of any kind
    /// ages the same way. Otherwise the rule would just move to a different
    /// permanent member.
    #[test]
    fn every_anchor_kind_ages_alike() {
        let day = 86_400_000u64;
        for kind in [AnchorKind::Repo, AnchorKind::Session, AnchorKind::Harness] {
            let mut m = Memberships::default();
            join(
                &mut m,
                "topic",
                Member {
                    kind,
                    value: "whatever".into(),
                },
            );
            touch_note_activity(&mut m, "topic", 1_000);
            let report = hygiene_report(&m, 7, 1_000 + 8 * day, CHANNEL_UNUSED_GRACE_MS);
            assert_eq!(report.dormant, vec!["topic"], "{}", kind.as_str());
        }
    }

    #[test]
    fn has_members_false_for_empty_or_missing() {
        let mut m = Memberships::default();
        create(&mut m, "empty");
        assert!(!has_members(&m, "empty"));
        assert!(!has_members(&m, "missing"));
        join(&mut m, "room", repo("/r"));
        assert!(has_members(&m, "room"));
    }

    #[test]
    fn archive_bulk_revalidates_members() {
        let mut m = Memberships::default();
        create(&mut m, "a");
        create(&mut m, "b");
        join(&mut m, "b", repo("/r"));
        let items = archive_bulk(&mut m, &["a".into(), "b".into(), "missing".into()]);
        assert_eq!(items[0].status, BulkArchiveStatus::Archived);
        assert_eq!(items[1].status, BulkArchiveStatus::Conflict);
        assert_eq!(items[2].status, BulkArchiveStatus::Skipped);
        assert!(is_archived(&m, "a"));
    }

    #[test]
    fn a_built_in_room_cannot_be_archived_deleted_or_edited() {
        let mut m = Memberships::default();
        join(&mut m, LOBBY, repo("/r"));
        assert!(archive(&mut m, "#Lobby").unwrap_err().contains("built in"));
        assert!(archive_unsubscribing(&mut m, LOBBY).is_err());
        assert!(delete_archived(&mut m, COMMONS).is_err());
        assert!(update_room(&mut m, FEEDBACK, Some("x"), RoomVisibility::Shared).is_err());
        assert!(has_members(&m, LOBBY), "nobody was unsubscribed");
        assert!(hygiene_report(&m, 7, 1, 0).armed.is_empty());
        assert!(archive_candidates(&m, 7, 1).is_empty());
    }

    #[test]
    fn archiving_from_the_console_unsubscribes_and_reports_who() {
        let mut m = Memberships::default();
        join(&mut m, "ops", repo("/r"));
        join(&mut m, "ops", repo("/s"));
        let gone = archive_unsubscribing(&mut m, "ops").unwrap();
        assert_eq!(gone.len(), 2);
        assert!(is_archived(&m, "ops"));
        assert!(!has_members(&m, "ops"));
        delete_archived(&mut m, "ops").unwrap();
    }

    #[test]
    fn a_built_in_room_keeps_one_read_position_per_workspace() {
        let mut m = Memberships::default();
        let first = "01J00000000000000000000000";
        assert!(mark_room_read(&mut m, LOBBY, Some("acme"), first).unwrap());
        assert_eq!(m.reads["lobby@acme"], first);
        assert!(!m.reads.contains_key("lobby@personal"));
        assert!(!m.channels.contains_key(LOBBY), "reading creates no room");
        create(&mut m, "ops");
        assert!(mark_room_read(&mut m, "ops", Some("acme"), first).unwrap());
        assert_eq!(m.reads["ops"], first, "a named room has one home");
    }

    #[test]
    fn a_room_lives_where_it_was_created_and_where_its_checkouts_are() {
        use crate::workspaces::{Placement, Workspaces};
        let mut set = Workspaces::single();
        set.workspaces.clear();
        set.add("personal", "Personal", &["me".into()]).unwrap();
        set.add("acme", "Acme", &["Acme".into()]).unwrap();
        set.default = "personal".into();
        let placement = Placement::with(Some(set), |path| crate::workspaces::RepoFacts {
            root: Some(path.to_string()),
            owner: Some(
                if path.starts_with("/acme") {
                    "Acme"
                } else {
                    "me"
                }
                .to_string(),
            ),
        });

        let mut m = Memberships::default();
        // Nothing says where: the default workspace.
        create(&mut m, "loose");
        assert_eq!(
            room_workspaces(&m, "loose", &placement, ""),
            BTreeSet::from(["personal".to_string()])
        );
        // Created in one, with a checkout in the other: both.
        join(&mut m, "ops", repo("/acme/app"));
        m.meta.get_mut("ops").unwrap().workspace = Some("personal".into());
        assert_eq!(
            room_workspaces(&m, "ops", &placement, ""),
            BTreeSet::from(["acme".to_string(), "personal".to_string()])
        );
    }
}
