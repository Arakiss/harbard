//! Read-side queries: session liveness, inbox delivery, store health.
//!
//! All functions take `now_ms`/`stale_secs` as parameters (never read the clock
//! themselves) so tests can inject a clock — acceptance row A9. Ordering is file
//! order, which the caller preserves by feeding events as read.

use crate::cursor;
use crate::event::{Event, EventKind, Role};
use crate::lease::{LeaseStats, LeaseStore, PidChecker, SystemLeaseClock, SystemPidChecker};
use crate::store::Store;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use ulid::Ulid;

/// Default freshness threshold; a confirmed supervisor can remain reachable beyond it.
pub const DEFAULT_STALE_SECS: u64 = 900;
/// Hard cap on events returned by a single `inbox` call.
pub const INBOX_WINDOW: usize = 500;
/// How long a herd child stays Live after `bye` while its parent is still Live.
///
/// Claude's UI keeps short Explore/Plan workers visible after stop; SubagentStop
/// used to drop them from harbard's Live rail immediately and undercount the
/// herd. Grace closes that gap without inventing presence.
pub const HERD_BYE_GRACE_MS: u64 = 10 * 60 * 1000;

/// Millisecond instant of an event, from its ULID (the store's own ids always
/// carry it). Foreign/corrupt ids fall back to 0 (treated as ancient).
pub fn event_ms(ev: &Event) -> u64 {
    Ulid::from_string(&ev.id)
        .map(|u| u.timestamp_ms())
        .unwrap_or(0)
}

/// Liveness verdict rendered by `ls`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Liveness {
    Live,
    Stale,
    Ended,
}

impl Liveness {
    pub fn as_str(self) -> &'static str {
        match self {
            Liveness::Live => "live",
            Liveness::Stale => "stale",
            Liveness::Ended => "ended",
        }
    }
}

/// Completeness of the observable hook lifecycle for one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LifecycleState {
    /// At least one hello and no bye yet.
    Active,
    /// At least one hello and one bye.
    Complete,
    /// No hello was observed; the row came from a partial or unstable signal.
    Incomplete,
}

/// The repository key worth carrying on a session row: `Some` only when this
/// event was written from a linked worktree, so the common case adds no bytes.
fn worktree_repo_key(ev: &Event) -> Option<String> {
    ev.repo_key
        .as_deref()
        .filter(|key| Some(*key) != ev.repo.as_deref())
        .map(str::to_string)
}

/// One session's collapsed view (latest metadata + liveness).
///
/// F1 wire fields plus herd `parent` and M5a `identity` confidence. `last_kind`
/// is internal (skipped) and never crosses the wire.
#[derive(Debug, Clone, Serialize)]
pub struct SessionView {
    pub session: String,
    pub harness: String,
    pub host: String,
    pub cwd: String,
    pub repo: Option<String>,
    /// Repository this checkout belongs to, shared by sibling worktrees.
    ///
    /// Present only for a session in a LINKED worktree, where it differs from
    /// `repo`; a primary checkout is its own repository and omits the field, so
    /// the frozen key set is unchanged for every row that had one before.
    /// Read it as `repo_key.or(repo)`. Additive wire field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_key: Option<String>,
    pub branch: Option<String>,
    pub role: Role,
    /// Parent session when this is a herd/subagent; always on the wire as null
    /// or a string so the console can nest without optional-key drift.
    pub parent: Option<String>,
    /// Lineage root derived from immediate parent links.
    pub root: String,
    /// Number of parent edges from this session to `root`.
    pub depth: u32,
    /// Number of recursive children below this session.
    pub descendants: u32,
    /// Hook lifecycle completeness after duplicate events are collapsed.
    pub lifecycle: LifecycleState,
    /// Raw events contributing to this collapsed row.
    pub event_count: u32,
    pub intent: Option<String>,
    /// Reachability for delivery. A confirmed supervisor can keep this Live
    /// without proving recent activity; see `idle` and `last_signal_ms`.
    pub liveness: Liveness,
    /// A confirmed supervisor is still running, but no signal is fresh.
    /// This does not distinguish a long autonomous turn from waiting for input.
    /// Omitted when false to preserve existing consumers of the three verdicts.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub idle: bool,
    /// Newest event, brief heartbeat, or inline hook check, in epoch milliseconds.
    pub last_signal_ms: u64,
    pub last_ms: u64,
    pub first_ms: u64,
    /// How sure we are about this session's identity (M5a). Additive wire field.
    pub identity: crate::topology::IdentityConfidence,
    /// The latest event's kind, kept only to drive the `bye` → ended verdict in
    /// `ls`. Not part of the frozen F1 SessionView contract, so never serialized.
    #[serde(skip)]
    pub last_kind: EventKind,
    /// This row said `bye` and is only still `Live` because of the herd grace
    /// window below. The grace exists so a fan-out's children do not blink out
    /// of the UI one by one, but laundering a farewell into presence made the
    /// live COUNT lie. Anything that counts or waits on presence must exclude these.
    /// Never serialized: the F1 `SessionView` contract is frozen, and this is
    /// an internal correction to presence counting, not a new wire field.
    #[serde(skip)]
    pub ending: bool,
    #[serde(skip)]
    pub saw_hello: bool,
    #[serde(skip)]
    pub saw_bye: bool,
}

impl SessionView {
    /// Human activity label; reachability remains available in `liveness`.
    pub fn activity_label(&self) -> &'static str {
        if self.idle {
            "idle"
        } else {
            self.liveness.as_str()
        }
    }
}

/// Collapse events (file order) into one view per session, newest wins.
///
/// Heartbeats (`brief --touch`) are read per session from the store, so a
/// session that only touches still counts as a fresh signal.
pub fn sessions(store: &Store, events: &[Event], now_ms: u64, stale_secs: u64) -> Vec<SessionView> {
    sessions_with_pid_checker(store, events, now_ms, stale_secs, &SystemPidChecker)
}

fn sessions_with_pid_checker(
    store: &Store,
    events: &[Event],
    now_ms: u64,
    stale_secs: u64,
    pid_checker: &dyn PidChecker,
) -> Vec<SessionView> {
    // Preserve first-seen order for determinism, update fields as we go.
    let mut order: Vec<String> = Vec::new();
    let mut map: BTreeMap<String, SessionView> = BTreeMap::new();

    for ev in events {
        let ms = event_ms(ev);
        let entry = map.entry(ev.session.clone()).or_insert_with(|| {
            order.push(ev.session.clone());
            SessionView {
                session: ev.session.clone(),
                harness: ev.harness.clone(),
                host: ev.host.clone(),
                cwd: ev.cwd.clone(),
                repo: ev.repo.clone(),
                repo_key: worktree_repo_key(ev),
                branch: ev.branch.clone(),
                role: ev.role,
                parent: ev.parent.clone(),
                root: ev.session.clone(),
                depth: 0,
                descendants: 0,
                lifecycle: LifecycleState::Incomplete,
                event_count: 0,
                intent: None,
                liveness: Liveness::Stale,
                idle: false,
                last_signal_ms: ms,
                last_ms: ms,
                first_ms: ms,
                identity: crate::topology::infer_identity_confidence(
                    &ev.session,
                    &ev.host,
                    &ev.harness,
                    ev.parent.as_deref(),
                ),
                last_kind: ev.event,
                ending: false,
                saw_hello: false,
                saw_bye: false,
            }
        });
        entry.event_count = entry.event_count.saturating_add(1);
        // A session belongs to one harness for its whole life. `other` is the
        // absence of an answer, so a real harness may still fill it in — but a
        // named harness is never overwritten by a different one. Cursor runs
        // Claude Code's hook file beside its own, which gave one conversation
        // two identities three milliseconds apart; newest-wins handed every
        // Cursor session to Claude Code, and the foreign `bye` at each turn
        // boundary buried it while it was still working.
        if entry.harness == "other" && ev.harness != "other" {
            entry.harness = ev.harness.clone();
        }
        // An echo still proves the session is running (it counts toward
        // `last_ms` below), but it may not speak for its lifecycle or metadata.
        let echo = ev.harness != entry.harness;
        if echo {
            if ms >= entry.last_ms {
                entry.last_ms = ms;
            }
            entry.first_ms = entry.first_ms.min(ms);
            continue;
        }
        entry.saw_hello |= ev.event == EventKind::Hello;
        entry.saw_bye |= ev.event == EventKind::Bye;
        // Latest event owns most metadata, but herd identity is sticky: a later
        // line that omits `parent` / downgrades `role` must not orphan a child
        // that already announced itself via SubagentStart.
        entry.host = ev.host.clone();
        entry.cwd = ev.cwd.clone();
        entry.repo = ev.repo.clone();
        entry.repo_key = worktree_repo_key(ev);
        entry.branch = ev.branch.clone();
        if let Some(p) = ev.parent.clone() {
            entry.parent = Some(p);
        }
        if matches!(ev.role, Role::Herd) {
            entry.role = Role::Herd;
        } else if entry.role != Role::Herd {
            entry.role = ev.role;
        }
        entry.identity = crate::topology::infer_identity_confidence(
            &entry.session,
            &entry.host,
            &entry.harness,
            entry.parent.as_deref(),
        );
        entry.last_kind = ev.event;
        if ms >= entry.last_ms {
            entry.last_ms = ms;
        }
        // Earliest signal wins first_ms (file order is ascending, but stay robust
        // to a foreign/zero id landing out of order).
        entry.first_ms = entry.first_ms.min(ms);
        // Intent is the text of the most recent intent (or hello-with-intent).
        if matches!(ev.event, EventKind::Intent | EventKind::Hello) && !ev.text.is_empty() {
            entry.intent = Some(ev.text.clone());
        }
    }

    let threshold = stale_secs.saturating_mul(1000);
    let mut out: Vec<SessionView> = order.into_iter().filter_map(|s| map.remove(&s)).collect();
    let mut pid_probe: HashMap<u32, (bool, Option<u64>)> = HashMap::new();
    let cursors = cursor::CursorNames::load(store);
    for view in &mut out {
        // Presence is evidence (§2), and the hot path IS evidence: the inline
        // hook's check stamp proves the session is running right now just as
        // surely as a turn-boundary heartbeat does. Counting only the brief
        // mark declared every long autonomous run dead fifteen minutes in —
        // while it was firing hooks continuously.
        let brief = cursors.read_brief_state(store, &view.session);
        let inline = cursors.read_live_state(store, &view.session);
        let hb = brief.hb;
        let checked = inline.checked_ms;
        let last_signal = view.last_ms.max(hb).max(checked);
        let fresh = now_ms.saturating_sub(last_signal) <= threshold || last_signal >= now_ms;
        // New hook writers bind each heartbeat to the long-lived harness
        // process that emitted it. A fresh timestamp from a process that has
        // since exited is not presence. Older/remote cursors have no PID and
        // retain the time-based fallback until they emit a process-bound mark.
        // The start marker always travels with the PID from the same cursor:
        // a marker from one cursor must never vouch for the other's PID.
        let supervisor = match (brief.supervisor_pid, inline.supervisor_pid) {
            (Some(pid), _) if hb >= checked => Some((pid, brief.supervisor_start)),
            (_, Some(pid)) => Some((pid, inline.supervisor_start)),
            (Some(pid), None) => Some((pid, brief.supervisor_start)),
            (None, None) => None,
        };
        let mut probe = |pid: u32| {
            *pid_probe.entry(pid).or_insert_with(|| {
                let alive = pid_checker.is_alive(pid);
                (alive, alive.then(|| pid_checker.start_time(pid)).flatten())
            })
        };
        let process_alive = !fresh || supervisor.is_none_or(|(pid, _)| probe(pid).0);
        // Process-bound presence. Session-level hooks (SessionStart, prompt
        // submit, SessionEnd) leave a long autonomous turn with no signal at
        // all, so the clock alone declared a working session Stale fifteen
        // minutes into its turn and refused it handoffs. A heartbeat bound to
        // a supervisor that is still running — the SAME process, proven by its
        // start marker, never by PID alone since PIDs are recycled — is
        // presence for as long as that process lives. Cursors written before
        // the marker existed, and platforms that cannot read one, stay on the
        // clock.
        let process_bound = supervisor.is_some_and(|(pid, recorded)| {
            recorded.is_some_and(|start| {
                let (alive, current) = probe(pid);
                alive && current == Some(start)
            })
        });
        // A bye ends it only when NOTHING newer came after — not even a hook
        // check, which would make it a zombie rather than a departure.
        let ended =
            view.last_kind == EventKind::Bye && hb <= view.last_ms && checked <= view.last_ms;
        view.liveness = if ended {
            Liveness::Ended
        } else if (fresh && process_alive) || process_bound {
            Liveness::Live
        } else {
            Liveness::Stale
        };
        view.last_signal_ms = last_signal;
        view.idle = view.liveness == Liveness::Live && !fresh && process_bound;
        view.lifecycle = if view.saw_hello && view.saw_bye {
            LifecycleState::Complete
        } else if view.saw_hello {
            LifecycleState::Active
        } else {
            LifecycleState::Incomplete
        };
    }

    // A single unknown host-pid row with no parent is shell/process noise, not
    // an addressable agent. Preserve stable sessions and every linked child.
    out.retain(|view| {
        !(view.event_count == 1
            && view.identity == crate::topology::IdentityConfidence::Unknown
            && view.parent.is_none()
            && view.harness == "other")
    });

    // Derive root/depth and recursive descendant counts from immediate edges.
    // Broken/missing parent links terminate at the last known id; cycles are
    // bounded by `seen` and remain visible instead of hanging the projection.
    let parents: HashMap<String, Option<String>> = out
        .iter()
        .map(|view| (view.session.clone(), view.parent.clone()))
        .collect();
    for view in &mut out {
        let mut current = view.session.clone();
        let mut root = current.clone();
        let mut depth = 0u32;
        let mut seen = HashSet::from([current.clone()]);
        while let Some(parent) = parents.get(&current).and_then(Clone::clone) {
            if !seen.insert(parent.clone()) {
                break;
            }
            root = parent.clone();
            depth = depth.saturating_add(1);
            current = parent;
        }
        view.root = root;
        view.depth = depth;
    }
    let indices: HashMap<String, usize> = out
        .iter()
        .enumerate()
        .map(|(index, view)| (view.session.clone(), index))
        .collect();
    for view in &out.clone() {
        let mut current = view.parent.clone();
        let mut seen = HashSet::new();
        while let Some(parent) = current {
            if !seen.insert(parent.clone()) {
                break;
            }
            let Some(index) = indices.get(&parent).copied() else {
                break;
            };
            out[index].descendants = out[index].descendants.saturating_add(1);
            current = parents.get(&parent).and_then(Clone::clone);
        }
    }

    // Herd grace: short-lived Claude/Codex children stay Live for a few minutes
    // after bye while the parent is still Live — matches the harness UI and
    // stops the Live rail from undercounting mid-swarm.
    let live_parents: BTreeSet<String> = out
        .iter()
        .filter(|v| v.liveness == Liveness::Live)
        .map(|v| v.session.clone())
        .collect();
    for view in &mut out {
        if view.role != Role::Herd || view.liveness == Liveness::Live {
            continue;
        }
        let Some(parent) = view.parent.as_deref() else {
            continue;
        };
        if !live_parents.contains(parent) {
            continue;
        }
        if view.last_kind != EventKind::Bye {
            continue;
        }
        if now_ms.saturating_sub(view.last_ms) <= HERD_BYE_GRACE_MS {
            view.liveness = Liveness::Live;
            view.ending = true;
        }
    }
    out
}

/// Result of an `inbox` read.
#[derive(Debug, Clone)]
pub struct InboxResult {
    /// Events addressed to me, in file order, capped at [`INBOX_WINDOW`].
    pub events: Vec<Event>,
    /// Total addressed events after the cursor (before the window cap).
    pub total: usize,
    /// Number actually returned.
    pub returned: usize,
    /// Id to persist as the new cursor, or `None` to leave it unchanged.
    pub new_cursor: Option<String>,
}

/// Compute the inbox for `me` from events in file order, after `cursor`.
///
/// Addressed = broadcast or targeted at me, excluding my own events. A missing
/// cursor id replays from the start of the window (bounded), never from epoch.
#[allow(clippy::too_many_arguments)]
pub fn inbox(
    events: &[Event],
    me_session: &str,
    me_harness: &str,
    me_repo: Option<&str>,
    me_repo_key: Option<&str>,
    cursor: Option<&str>,
    received: &std::collections::HashSet<String>,
    workspace: &crate::workspaces::Gate<'_>,
) -> InboxResult {
    let start = match cursor {
        Some(c) => match events.iter().position(|e| e.id == c) {
            Some(pos) => pos + 1,
            None => 0, // reset/replay, bounded by the window the caller passed
        },
        None => 0,
    };
    let window = &events[start.min(events.len())..];
    // Only notes are deliverable messages; hello/intent/bye are presence and
    // surface in `ls`/`brief`, not the inbox (the SPEC's "events addressed to
    // me" — only notes carry a recipient).
    let addressed: Vec<&Event> = window
        .iter()
        .filter(|e| {
            e.event == EventKind::Note
                && e.session != me_session
                && !received.contains(&e.id)
                && e.is_addressed_to(me_session, me_harness, me_repo, me_repo_key, workspace)
        })
        .collect();
    let total = addressed.len();
    let returned = total.min(INBOX_WINDOW);
    let returned_events: Vec<Event> = addressed[..returned].iter().map(|e| (*e).clone()).collect();

    let new_cursor = if window.is_empty() {
        None
    } else if total > returned {
        // Windowed: do not skip undelivered addressed events.
        returned_events.last().map(|e| e.id.clone())
    } else {
        // Fully delivered: skip past everything examined, incl. non-addressed.
        window.last().map(|e| e.id.clone())
    };

    InboxResult {
        events: returned_events,
        total,
        returned,
        new_cursor,
    }
}

/// Store health report for `doctor`.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    pub store_root: String,
    pub store_exists: bool,
    pub segments: u64,
    pub total_lines: u64,
    pub events: u64,
    pub corrupt_lines: u64,
    pub unknown_events: u64,
    pub cursors: u64,
    pub retention_days: u64,
    pub oldest_segment: Option<String>,
    pub newest_segment: Option<String>,
    pub leases: LeaseStats,
    /// Open (non-terminal) correction resolutions after stale expiry.
    pub open_resolutions: u64,
    /// Rows expired during this doctor pass.
    pub expired_resolutions: u64,
    /// Named-channel hygiene (diagnose only — never auto-archive).
    pub channels: crate::membership::ChannelHygieneReport,
    /// Open `channel_unused` correction ids (channel names).
    pub open_channel_unused: Vec<String>,
    /// How many `channel_unused` resolutions were armed this pass.
    pub armed_channel_unused: u64,
    /// Corrections withdrawn because the log disagreed with them.
    pub retired_channel_unused: u64,
    /// Disposable note FTS index (same log, better index).
    pub fts: crate::fts::FtsStats,
}

/// Gather store health across all segments (recomputes corrupt counts live).
pub fn doctor(store: &Store, retention_days: u64) -> DoctorReport {
    let read = store.read(crate::store::Window::All);
    let segs = store.segments();
    let seg_name = |p: &std::path::Path| -> Option<String> {
        p.file_name().and_then(|n| n.to_str()).map(str::to_string)
    };
    let resolutions = crate::resolution::ResolutionStore::at(store.root());
    let now = crate::clock::now_ms();
    let expired_resolutions = resolutions
        .expire_stale(now, crate::resolution::RESOLUTION_TTL_MS)
        .unwrap_or(0);
    let mut membership = crate::membership::read(store);
    // Before judging a channel by its recorded activity, make the record true.
    // The incremental counter only knows about notes written since it existed;
    // the log knows about all of them.
    const CHANNEL_INACTIVE_DAYS: u64 = 7;
    let healed = crate::membership::reconcile_activity(
        &mut membership,
        store,
        CHANNEL_INACTIVE_DAYS * 86_400_000,
        now,
    );
    if healed > 0 {
        let _ = crate::membership::write(store, &membership);
    }
    let channels = crate::membership::hygiene_report(
        &membership,
        CHANNEL_INACTIVE_DAYS,
        now,
        crate::membership::CHANNEL_UNUSED_GRACE_MS,
    );
    let open = resolutions.open();
    let mut open_channel_unused: Vec<String> = open
        .iter()
        .filter(|r| r.code == crate::protocol::codes::CHANNEL_UNUSED)
        .filter_map(|r| r.session.strip_prefix("channel:").map(str::to_string))
        .collect();
    open_channel_unused.sort();
    open_channel_unused.dedup();

    // Retire the ones that are no longer true before arming new ones.
    //
    // `channel_unused` says "you opened this and never used it", and it closes
    // when the first note arrives. That closure never fired for the channels
    // armed off a counter that had not seen their traffic: 18 open corrections,
    // 17 of them naming channels with notes — one of them 111 notes. A standing
    // correction that stopped being true is worse than none, because the next
    // reader learns to skim the whole list.
    let mut retired_channel_unused = 0u64;
    for name in open_channel_unused.clone() {
        if channels.armed.contains(&name) {
            continue;
        }
        let _ = resolutions.advance_latest(
            &format!("channel:{name}"),
            crate::protocol::codes::CHANNEL_UNUSED,
            crate::resolution::ReceiptStage::Applied,
            now,
            Some("reconciled: the log shows notes to this channel"),
        );
        retired_channel_unused += 1;
        open_channel_unused.retain(|open| open != &name);
    }

    // Arm non-blocking channel_unused once per channel past grace.
    let mut armed_channel_unused = 0u64;
    for name in &channels.armed_past_grace {
        let session = format!("channel:{name}");
        if open
            .iter()
            .any(|row| row.session == session && row.code == crate::protocol::codes::CHANNEL_UNUSED)
        {
            continue;
        }
        let _ = resolutions.deliver(
            &session,
            crate::protocol::codes::CHANNEL_UNUSED,
            now,
            Some(&format!(
                "armed past grace; join then note or leave #{name}"
            )),
        );
        armed_channel_unused += 1;
        if !open_channel_unused.contains(name) {
            open_channel_unused.push(name.clone());
        }
    }
    open_channel_unused.sort();

    let open_resolutions = resolutions.open().len() as u64;
    // Ensure the disposable FTS exists and matches schema; never fails doctor.
    let fts = crate::fts::ensure(store).unwrap_or_else(|_| crate::fts::stats(store));
    DoctorReport {
        store_root: store.root().display().to_string(),
        store_exists: store.events_dir().exists(),
        segments: segs.len() as u64,
        total_lines: read.total_lines,
        events: read.events.len() as u64,
        corrupt_lines: read.corrupt_lines,
        unknown_events: read.unknown_events,
        cursors: cursor::count(store),
        retention_days,
        oldest_segment: segs.first().and_then(|p| seg_name(p)),
        newest_segment: segs.last().and_then(|p| seg_name(p)),
        leases: LeaseStore::at(store.root()).stats(SystemLeaseClock, &SystemPidChecker),
        open_resolutions,
        expired_resolutions,
        channels,
        open_channel_unused,
        armed_channel_unused,
        retired_channel_unused,
        fts,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Recipient, RecipientKind, SCHEMA_VERSION};
    use crate::test_util::TempHome;

    fn mk(id_ms: u64, kind: EventKind, session: &str, text: &str) -> Event {
        Event {
            v: SCHEMA_VERSION,
            id: Ulid::from_parts(id_ms, 0).to_string(),
            ts: crate::clock::rfc3339_utc(id_ms),
            event: kind,
            session: session.to_string(),
            harness: "codex".to_string(),
            host: "h".to_string(),
            cwd: "/tmp".to_string(),
            repo: Some("/repo".to_string()),
            repo_key: None,
            branch: Some("main".to_string()),
            role: Role::Main,
            parent: None,
            note_ref: None,
            text: text.to_string(),
            to: None,
            kind: crate::event::NoteKind::Fyi,
        }
    }

    struct FixedPidChecker {
        alive: bool,
        start: Option<u64>,
    }

    impl FixedPidChecker {
        fn alive(alive: bool) -> Self {
            Self { alive, start: None }
        }

        fn running(start: u64) -> Self {
            Self {
                alive: true,
                start: Some(start),
            }
        }
    }

    impl PidChecker for FixedPidChecker {
        fn is_alive(&self, _pid: u32) -> bool {
            self.alive
        }

        fn start_time(&self, _pid: u32) -> Option<u64> {
            self.start
        }
    }

    fn mk_harness(id_ms: u64, kind: EventKind, session: &str, harness: &str) -> Event {
        Event {
            harness: harness.to_string(),
            ..mk(id_ms, kind, session, "")
        }
    }

    #[test]
    fn a_foreign_hook_echo_cannot_rebrand_or_bury_a_session() {
        // Cursor runs Claude Code's hook file beside its own: one conversation,
        // two hellos milliseconds apart, then a `bye` at every turn boundary
        // from the harness that was never running it.
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let evs = vec![
            mk_harness(base, EventKind::Hello, "s1", "cursor"),
            mk_harness(base + 3, EventKind::Hello, "s1", "claude-code"),
            mk_harness(base + 60_000, EventKind::Bye, "s1", "claude-code"),
        ];
        let v = sessions(&store, &evs, base + 60_100, 900);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].harness, "cursor");
        assert_eq!(v[0].liveness, Liveness::Live);
        assert!(!v[0].saw_bye);
    }

    #[test]
    fn an_unattributed_session_still_takes_the_first_harness_that_names_it() {
        // `other` is the absence of an answer, not a competing identity: a
        // session that starts unattributed accepts the first real harness.
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let evs = vec![
            mk_harness(base, EventKind::Hello, "s1", "other"),
            mk_harness(base + 10, EventKind::Intent, "s1", "cursor"),
            mk_harness(base + 20, EventKind::Note, "s1", "claude-code"),
        ];
        let v = sessions(&store, &evs, base + 100, 900);
        assert_eq!(v[0].harness, "cursor");
    }

    #[test]
    fn the_owning_harness_still_ends_its_own_session() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let evs = vec![
            mk_harness(base, EventKind::Hello, "s1", "cursor"),
            mk_harness(base + 60_000, EventKind::Bye, "s1", "cursor"),
        ];
        let v = sessions(&store, &evs, base + 60_100, 900);
        assert_eq!(v[0].liveness, Liveness::Ended);
        assert!(v[0].saw_bye);
    }

    #[test]
    fn liveness_flips_at_threshold() {
        // A9: with an injected clock, a session with no signal for T is stale.
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let evs = vec![mk(base, EventKind::Hello, "s1", "hi")];
        let fresh = sessions(&store, &evs, base + 100_000, 900);
        assert_eq!(fresh[0].liveness, Liveness::Live);
        let stale = sessions(&store, &evs, base + 1_000_000, 900);
        assert_eq!(stale[0].liveness, Liveness::Stale);
    }

    #[test]
    fn a_fresh_inline_check_keeps_a_long_run_alive() {
        // The hot path is presence evidence: an old event plus a live-channel
        // check from seconds ago is a working session, not a stale one.
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let evs = vec![mk(base, EventKind::Hello, "s1", "hi")];
        cursor::write_live_state(
            &store,
            "s1",
            &cursor::LiveState {
                seen: None,
                checked_ms: base + 995_000,
                ..Default::default()
            },
        )
        .unwrap();
        let v = sessions(&store, &evs, base + 1_000_000, 900);
        assert_eq!(v[0].liveness, Liveness::Live);
        // And a bye newer than every check still ends it.
        let mut evs2 = evs.clone();
        evs2.push(mk(base + 999_000, EventKind::Bye, "s1", ""));
        let v2 = sessions(&store, &evs2, base + 1_000_000, 900);
        assert_eq!(v2[0].liveness, Liveness::Ended);
    }

    #[test]
    fn a_dead_supervisor_fences_a_fresh_hook_signal() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let evs = vec![mk(base, EventKind::Hello, "s1", "hi")];
        cursor::write_live_state(
            &store,
            "s1",
            &cursor::LiveState {
                seen: None,
                checked_ms: base + 995_000,
                supervisor_pid: Some(4242),
                ..Default::default()
            },
        )
        .unwrap();

        let live = sessions_with_pid_checker(
            &store,
            &evs,
            base + 1_000_000,
            900,
            &FixedPidChecker::alive(true),
        );
        assert_eq!(live[0].liveness, Liveness::Live);

        let dead = sessions_with_pid_checker(
            &store,
            &evs,
            base + 1_000_000,
            900,
            &FixedPidChecker::alive(false),
        );
        assert_eq!(dead[0].liveness, Liveness::Stale);
    }

    /// A long autonomous turn: hello and the last heartbeat are both older than
    /// `stale_secs`, the only hooks are session-level, and the harness is still
    /// working. `bind` writes the heartbeat with the given supervisor marker.
    fn long_turn(bind: Option<u64>, checker: &FixedPidChecker) -> (Vec<SessionView>, TempHome) {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let evs = vec![mk(base, EventKind::Hello, "s1", "hi")];
        cursor::write_brief_state(
            &store,
            "s1",
            &cursor::BriefState {
                hb: base + 1_000,
                supervisor_pid: Some(4242),
                supervisor_start: bind,
                ..Default::default()
            },
        )
        .unwrap();
        // Forty minutes later, well past the 900 s threshold.
        let views = sessions_with_pid_checker(&store, &evs, base + 2_400_000, 900, checker);
        (views, home)
    }

    #[test]
    fn a_running_bound_supervisor_keeps_a_long_turn_live() {
        let (v, _home) = long_turn(Some(77), &FixedPidChecker::running(77));
        assert_eq!(v[0].liveness, Liveness::Live);
    }

    #[test]
    fn a_recycled_pid_with_another_start_is_not_presence() {
        let (v, _home) = long_turn(Some(77), &FixedPidChecker::running(78));
        assert_eq!(v[0].liveness, Liveness::Stale);
        // Alive but unreadable start: never trust the PID alone.
        let (v, _home) = long_turn(Some(77), &FixedPidChecker::alive(true));
        assert_eq!(v[0].liveness, Liveness::Stale);
    }

    #[test]
    fn a_dead_bound_supervisor_lets_a_long_turn_go_stale() {
        let checker = FixedPidChecker {
            alive: false,
            start: Some(77),
        };
        let (v, _home) = long_turn(Some(77), &checker);
        assert_eq!(v[0].liveness, Liveness::Stale);
    }

    #[test]
    fn a_legacy_cursor_without_a_start_marker_stays_on_the_clock() {
        // Same live PID as the bound case, but the cursor predates markers.
        let (v, _home) = long_turn(None, &FixedPidChecker::running(77));
        assert_eq!(v[0].liveness, Liveness::Stale);
        // Within the window it is still Live, exactly as before.
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let evs = vec![mk(base, EventKind::Hello, "s1", "hi")];
        cursor::write_brief_state(
            &store,
            "s1",
            &cursor::BriefState {
                hb: base + 1_000,
                supervisor_pid: Some(4242),
                ..Default::default()
            },
        )
        .unwrap();
        let v = sessions_with_pid_checker(
            &store,
            &evs,
            base + 600_000,
            900,
            &FixedPidChecker::running(77),
        );
        assert_eq!(v[0].liveness, Liveness::Live);
    }

    #[test]
    fn a_bye_still_ends_a_process_bound_session() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let evs = vec![
            mk(base, EventKind::Hello, "s1", "hi"),
            mk(base + 2_000, EventKind::Bye, "s1", ""),
        ];
        cursor::write_brief_state(
            &store,
            "s1",
            &cursor::BriefState {
                hb: base + 1_000,
                supervisor_pid: Some(4242),
                supervisor_start: Some(77),
                ..Default::default()
            },
        )
        .unwrap();
        let v = sessions_with_pid_checker(
            &store,
            &evs,
            base + 2_400_000,
            900,
            &FixedPidChecker::running(77),
        );
        assert_eq!(v[0].liveness, Liveness::Ended);
    }

    #[test]
    fn session_view_serializes_the_frozen_contract_fields() {
        // B1: `serve` serializes SessionView verbatim, so its key set is frozen
        // (docs/SPEC-API.md). Guard against silent drift.
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let evs = vec![mk(base, EventKind::Hello, "s1", "hi")];
        let views = sessions(&store, &evs, base + 100, 900);
        let value = serde_json::to_value(&views[0]).unwrap();
        let obj = value.as_object().unwrap();

        let mut got: Vec<&str> = obj.keys().map(String::as_str).collect();
        got.sort_unstable();
        let mut want = vec![
            "session",
            "harness",
            "host",
            "cwd",
            "repo",
            "branch",
            "role",
            "parent",
            "intent",
            "root",
            "depth",
            "descendants",
            "lifecycle",
            "event_count",
            "liveness",
            "last_signal_ms",
            "last_ms",
            "first_ms",
            "identity",
        ];
        want.sort_unstable();
        assert_eq!(got, want, "SessionView key set is the frozen contract");
        assert!(
            !obj.contains_key("last_kind"),
            "internal field stays off wire"
        );
        // A single-event session has first_ms == last_ms.
        assert_eq!(obj["first_ms"], obj["last_ms"]);
        assert!(obj["parent"].is_null());
    }

    #[test]
    fn sessions_keep_latest_parent() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let mut child = mk(base, EventKind::Hello, "child", "");
        child.role = Role::Herd;
        child.parent = Some("parent".to_string());
        let mut later = mk(base + 1_000, EventKind::Intent, "child", "working");
        later.role = Role::Herd;
        later.parent = Some("parent".to_string());
        let views = sessions(&store, &[child, later], base + 2_000, 900);
        assert_eq!(views[0].parent.as_deref(), Some("parent"));
        assert_eq!(views[0].role, Role::Herd);
        assert_eq!(views[0].intent.as_deref(), Some("working"));
    }

    #[test]
    fn sessions_project_depth_root_descendants_and_lifecycle() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let root = mk(base, EventKind::Hello, "root", "running");
        let mut child = mk(base + 1, EventKind::Hello, "child", "");
        child.role = Role::Herd;
        child.parent = Some("root".into());
        let mut grandchild = mk(base + 2, EventKind::Hello, "grandchild", "");
        grandchild.role = Role::Herd;
        grandchild.parent = Some("child".into());
        let mut child_bye = mk(base + 3, EventKind::Bye, "child", "");
        child_bye.role = Role::Herd;
        child_bye.parent = Some("root".into());

        let views = sessions(
            &store,
            &[root, child, grandchild, child_bye],
            base + 10,
            900,
        );
        let root = views.iter().find(|v| v.session == "root").unwrap();
        let child = views.iter().find(|v| v.session == "child").unwrap();
        let grandchild = views.iter().find(|v| v.session == "grandchild").unwrap();
        assert_eq!(root.root, "root");
        assert_eq!(root.depth, 0);
        assert_eq!(root.descendants, 2);
        assert_eq!(child.root, "root");
        assert_eq!(child.depth, 1);
        assert_eq!(child.lifecycle, LifecycleState::Complete);
        assert_eq!(grandchild.root, "root");
        assert_eq!(grandchild.depth, 2);
        assert_eq!(grandchild.lifecycle, LifecycleState::Active);
    }

    #[test]
    fn unstable_single_event_host_pid_ghost_is_filtered() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let mut ghost = mk(base, EventKind::Note, "mac-4242", "noise");
        ghost.host = "mac".into();
        ghost.harness = "other".into();
        assert!(sessions(&store, &[ghost], base + 1, 900).is_empty());
    }

    #[test]
    fn sessions_preserve_herd_when_later_event_omits_parent() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let mut child = mk(base, EventKind::Hello, "child", "");
        child.role = Role::Herd;
        child.parent = Some("parent".into());
        // A later verb without herd fields must not orphan the child.
        let later = mk(base + 1_000, EventKind::Note, "child", "still me");
        let views = sessions(&store, &[child, later], base + 2_000, 900);
        assert_eq!(views[0].parent.as_deref(), Some("parent"));
        assert_eq!(views[0].role, Role::Herd);
    }

    #[test]
    fn herd_bye_stays_live_while_parent_live_within_grace() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let parent = mk(base, EventKind::Hello, "parent", "running");
        let mut child_hi = mk(base + 1_000, EventKind::Hello, "child", "");
        child_hi.role = Role::Herd;
        child_hi.parent = Some("parent".into());
        let mut child_bye = mk(base + 2_000, EventKind::Bye, "child", "");
        child_bye.role = Role::Herd;
        child_bye.parent = Some("parent".into());
        let now = base + 2_000 + 60_000; // one minute after bye
        let views = sessions(&store, &[parent, child_hi, child_bye], now, 900);
        let child = views.iter().find(|v| v.session == "child").unwrap();
        let parent_v = views.iter().find(|v| v.session == "parent").unwrap();
        assert_eq!(parent_v.liveness, Liveness::Live);
        assert_eq!(
            child.liveness,
            Liveness::Live,
            "herd child stays live during grace while parent is live"
        );
    }

    #[test]
    fn herd_bye_ends_after_grace_or_when_parent_dead() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let mut child_hi = mk(base + 1_000, EventKind::Hello, "child", "");
        child_hi.role = Role::Herd;
        child_hi.parent = Some("parent".into());
        let mut child_bye = mk(base + 2_000, EventKind::Bye, "child", "");
        child_bye.role = Role::Herd;
        child_bye.parent = Some("parent".into());

        // Parent still live after grace window — child must end.
        let after_grace = base + 2_000 + HERD_BYE_GRACE_MS + 1;
        let parent_fresh = mk(after_grace - 1_000, EventKind::Intent, "parent", "still");
        let views = sessions(
            &store,
            &[
                mk(base, EventKind::Hello, "parent", "running"),
                child_hi.clone(),
                child_bye.clone(),
                parent_fresh,
            ],
            after_grace,
            900,
        );
        let child = views.iter().find(|v| v.session == "child").unwrap();
        assert_eq!(
            child.liveness,
            Liveness::Ended,
            "grace expires even if parent is still live"
        );

        // Parent ended: no grace even one minute after child bye.
        let parent_bye = mk(base + 3_000, EventKind::Bye, "parent", "");
        let now = base + 3_000 + 30_000;
        let views = sessions(
            &store,
            &[
                mk(base, EventKind::Hello, "parent", ""),
                child_hi,
                child_bye,
                parent_bye,
            ],
            now,
            900,
        );
        let child = views.iter().find(|v| v.session == "child").unwrap();
        assert_eq!(child.liveness, Liveness::Ended);
    }

    #[test]
    fn bye_marks_ended() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let base = 1_000_000_000_000;
        let evs = vec![
            mk(base, EventKind::Hello, "s1", "hi"),
            mk(base + 1000, EventKind::Bye, "s1", ""),
        ];
        let v = sessions(&store, &evs, base + 2000, 900);
        assert_eq!(v[0].liveness, Liveness::Ended);
    }

    #[test]
    fn inbox_delivers_once_and_excludes_own() {
        // A2 in miniature (full flow tested via the CLI).
        let base = 1_000_000_000_000;
        let mut note = mk(base + 10, EventKind::Note, "sender", "hello reader");
        note.to = Some(Recipient {
            kind: RecipientKind::Session,
            value: "reader".to_string(),
        });
        let evs = vec![mk(base, EventKind::Hello, "reader", "hi"), note];

        // Sender never sees their own note.
        let s = inbox(
            &evs,
            "sender",
            "codex",
            Some("/repo"),
            None,
            None,
            &Default::default(),
            &crate::workspaces::Gate::OPEN,
        );
        assert_eq!(s.returned, 0);

        // Reader gets it once.
        let r = inbox(
            &evs,
            "reader",
            "codex",
            Some("/repo"),
            None,
            None,
            &Default::default(),
            &crate::workspaces::Gate::OPEN,
        );
        assert_eq!(r.returned, 1);
        assert_eq!(r.events[0].text, "hello reader");
        let cur = r.new_cursor.clone().unwrap();

        // Second call is empty.
        let r2 = inbox(
            &evs,
            "reader",
            "codex",
            Some("/repo"),
            None,
            Some(&cur),
            &Default::default(),
            &crate::workspaces::Gate::OPEN,
        );
        assert_eq!(r2.returned, 0);
    }

    #[test]
    fn inbox_window_caps_and_discloses_total() {
        let base = 1_000_000_000_000;
        let mut evs = Vec::new();
        for i in 0..(INBOX_WINDOW as u64 + 5) {
            evs.push(mk(base + i, EventKind::Note, "sender", "n"));
        }
        let r = inbox(
            &evs,
            "reader",
            "codex",
            None,
            None,
            None,
            &Default::default(),
            &crate::workspaces::Gate::OPEN,
        );
        assert_eq!(r.total, INBOX_WINDOW + 5);
        assert_eq!(r.returned, INBOX_WINDOW);
        // Cursor did not skip the undelivered tail.
        let cur = r.new_cursor.unwrap();
        let r2 = inbox(
            &evs,
            "reader",
            "codex",
            None,
            None,
            Some(&cur),
            &Default::default(),
            &crate::workspaces::Gate::OPEN,
        );
        assert_eq!(r2.returned, 5);
    }

    #[test]
    fn lost_cursor_replays_bounded_not_epoch() {
        // E4: an unknown cursor id replays from the window start, once.
        let base = 1_000_000_000_000;
        let mut note = mk(base + 10, EventKind::Note, "sender", "hi");
        note.to = None;
        let evs = vec![note];
        let r = inbox(
            &evs,
            "reader",
            "codex",
            None,
            None,
            Some("01UNKNOWNCURSORVALUE00000"),
            &Default::default(),
            &crate::workspaces::Gate::OPEN,
        );
        assert_eq!(r.returned, 1);
    }
}
