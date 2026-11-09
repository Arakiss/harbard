//! Per-session cursors: derived, disposable state under `~/.harbard/cursors/`.
//!
//! Files per session:
//! - `<session>`        the inbox read cursor (last event id delivered by `inbox`)
//! - `<session>.brief`  the brief high-water mark + liveness heartbeat
//! - `<session>.live`   the inline channel's mark + last-checked stamp (F2)
//! - `<session>.wake`   scan position and explicit wake delivery receipts
//! - `<session>.watch`  which `watch` process currently owns the session
//! - `<session>.owed`   how many times `stop-check` held a stop per note
//! - `<session>.turn`   epoch ms at the end of the last turn (not a read cursor)
//!
//! Stop state lives separately from the brief state
//! because separate processes write them at the same moment: a Stop hook runs
//! `stop-check` and `watch` in parallel, and a prompt runs `brief --touch`
//! while the previous turn's watcher is still waiting. One file per writer
//! means no read-modify-write can erase another's update.
//!
//! The inline channel keeps its own mark on purpose: it fires on the hot
//! postToolUse path and delivers ONLY addressed notes, so advancing the brief's
//! mark there would silently swallow the turn-boundary digest's other content.
//!
//! Cursors are derived state, never authority over the store. Deleting them
//! can replay post-join mail, including quiet notes already delivered earlier;
//! it is not a routine recovery procedure.

use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// Brief's per-session state: what it has already surfaced, and the last
/// heartbeat (`brief --touch`) so reads double as liveness.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct BriefState {
    /// Newest event id the brief has already surfaced.
    #[serde(default)]
    pub seen: Option<String>,
    /// Last heartbeat, epoch ms.
    #[serde(default)]
    pub hb: u64,
    /// Long-lived harness process observed when this heartbeat was recorded.
    /// Local derived state only; never copied into the append-only event log.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor_pid: Option<u32>,
    /// Start marker of `supervisor_pid` when it was bound (see
    /// `process_start`). Present only beside a PID; it is what lets presence
    /// outlive `stale_secs` while that exact process runs. Absent on older
    /// cursors, which keep time-based presence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor_start: Option<u64>,
    /// Last time we asked this session to refresh its intent. Retired: the
    /// intent reminder is no longer emitted; kept so older cursors round-trip.
    #[serde(default)]
    pub status_nudge_ms: u64,
    /// Last time we asked this session to ack an open handoff.
    #[serde(default)]
    pub ack_nudge_ms: u64,
    /// Last time we warned about a contested checkout.
    #[serde(default)]
    pub checkout_nudge_ms: u64,
    /// Last time we asked this session to run doctor (0 = never; one-shot).
    #[serde(default)]
    pub doctor_nudge_ms: u64,
    /// Last time we asked this session to name itself. Retired with the
    /// intent reminder; kept so older cursors round-trip.
    #[serde(default)]
    pub name_nudge_ms: u64,
    /// Rival writers this session has been told about. Said once per rival:
    /// repeating it on every prompt carries nothing new.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rivals_warned: Vec<String>,
    /// The operator-state block this session last received, so a prompt
    /// carries it only when it changes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator_state: Option<String>,
}

/// The inline channel's per-session state (F2): what the hot path has already
/// delivered, and when it last looked at the store (the throttle stamp).
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct LiveState {
    /// Newest event id the inline channel has already delivered.
    #[serde(default)]
    pub seen: Option<String>,
    /// Last store check, epoch ms. Drives `--throttle`.
    #[serde(default)]
    pub checked_ms: u64,
    /// Long-lived harness process observed by the inline hook.
    /// When present, liveness is fenced by the process still existing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor_pid: Option<u32>,
    /// Start marker of `supervisor_pid` when it was bound; see
    /// [`BriefState::supervisor_start`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supervisor_start: Option<u64>,
    /// Where in the log this session stopped reading (F6). Absent on the first
    /// run and after a day boundary, which fall back to the full recent read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<crate::store::ReadPosition>,
}

/// What `watch` has already woken this session for.
///
/// Kept apart from the brief's mark because a wake is not a turn boundary:
/// a session woken by a reminder starts a turn without a prompt, so
/// `UserPromptSubmit` never fires and the brief's mark never moves. Without a
/// mark of its own, the watcher armed at the end of that turn would find the
/// same notes unread and wake the session for them again, for ever.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct WakeState {
    /// Newest event id a wake has already carried or scanned past.
    #[serde(default)]
    pub seen: Option<String>,
    /// Actual note deliveries, unlike seen which also advances over quiet mail.
    /// Cleared behind inbox/brief marks as those channels catch up.
    #[serde(default)]
    pub delivered: std::collections::HashSet<String>,
}

/// How many times `stop-check` held this session's stop, per owed note.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct OwedState {
    /// Note id → number of stops it has held. Capped by the caller.
    #[serde(default)]
    pub blocks: std::collections::BTreeMap<String, u32>,
}

/// Encode a session id into a safe, **injective** cursor filename segment.
///
/// Alphanumeric, `.`, and `-` stay literal. Every other byte (including `_`)
/// becomes `_HH` hex so `a/b` and `a_b` never share a file.
pub fn safe_session_filename(session: &str) -> String {
    let mut out = String::with_capacity(session.len());
    for b in session.as_bytes() {
        let c = *b as char;
        if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
            out.push(c);
        } else {
            out.push('_');
            out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
            out.push(char::from_digit((b & 0x0f) as u32, 16).unwrap_or('0'));
        }
    }
    if out.is_empty() { "_".to_string() } else { out }
}

/// Pre-injective sanitizer (maps every disallowed char to `_`). Kept only so
/// existing cursor files remain readable after the encoding change.
fn safe_legacy(session: &str) -> String {
    let s: String = session
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() { "_".to_string() } else { s }
}

fn cursor_file(store: &Store, session: &str, suffix: &str) -> PathBuf {
    let dir = store.cursors_dir();
    let modern = dir.join(format!("{}{suffix}", safe_session_filename(session)));
    if modern.exists() {
        return modern;
    }
    let legacy = dir.join(format!("{}{suffix}", safe_legacy(session)));
    if legacy.exists() {
        return legacy;
    }
    modern
}

fn inbox_path(store: &Store, session: &str) -> PathBuf {
    cursor_file(store, session, "")
}

fn brief_path(store: &Store, session: &str) -> PathBuf {
    cursor_file(store, session, ".brief")
}

fn live_path(store: &Store, session: &str) -> PathBuf {
    cursor_file(store, session, ".live")
}

fn write_json<T: Serialize>(path: PathBuf, dir: PathBuf, value: &T) -> std::io::Result<()> {
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_string(value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    fs::write(path, json.as_bytes())
}

/// Read the wake mark; missing or corrupt resets to defaults (E4).
pub fn read_wake_state(store: &Store, session: &str) -> WakeState {
    fs::read_to_string(cursor_file(store, session, ".wake"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist the wake mark. Best-effort.
pub fn write_wake_state(store: &Store, session: &str, state: &WakeState) -> std::io::Result<()> {
    write_json(
        cursor_file(store, session, ".wake"),
        store.cursors_dir(),
        state,
    )
}

/// Read the stop-check block counts; missing or corrupt resets to defaults.
pub fn read_owed_state(store: &Store, session: &str) -> OwedState {
    fs::read_to_string(cursor_file(store, session, ".owed"))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist the stop-check block counts. Best-effort.
pub fn write_owed_state(store: &Store, session: &str, state: &OwedState) -> std::io::Result<()> {
    write_json(
        cursor_file(store, session, ".owed"),
        store.cursors_dir(),
        state,
    )
}

/// Last Stop boundary. Missing/corrupt means no proven idle interval.
pub fn read_turn_end(store: &Store, session: &str) -> u64 {
    fs::read_to_string(cursor_file(store, session, ".turn"))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Record the end of a turn independently of delivery marks.
pub fn write_turn_end(store: &Store, session: &str, now_ms: u64) -> std::io::Result<()> {
    fs::create_dir_all(store.cursors_dir())?;
    fs::write(cursor_file(store, session, ".turn"), now_ms.to_string())
}

/// The token of the `watch` process that currently owns this session.
pub fn read_watch_owner(store: &Store, session: &str) -> Option<String> {
    let raw = fs::read_to_string(cursor_file(store, session, ".watch")).ok()?;
    let trimmed = raw.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Make `token` the session's watcher. The previous owner sees the change on
/// its next poll and exits, so a session never has two watchers waking it for
/// the same note.
pub fn write_watch_owner(store: &Store, session: &str, token: &str) -> std::io::Result<()> {
    let dir = store.cursors_dir();
    fs::create_dir_all(&dir)?;
    fs::write(cursor_file(store, session, ".watch"), token.as_bytes())
}

/// Drop the owner file, but only while `token` still owns it: a watcher that
/// was replaced must not unregister its successor on the way out.
pub fn clear_watch_owner(store: &Store, session: &str, token: &str) {
    if read_watch_owner(store, session).as_deref() == Some(token) {
        let _ = fs::remove_file(cursor_file(store, session, ".watch"));
    }
}

/// The cursor directory's file names, listed once.
///
/// A projection over every session ever seen (`query::sessions`) looks up two
/// cursors per session, and most sessions have none: each lookup used to cost
/// two failed `stat`s and a failed `open`. One `readdir` answers "is there a
/// file" for all of them. A cursor created after the listing reads as missing,
/// which is the same answer a read a moment earlier would have given.
#[derive(Debug, Default)]
pub struct CursorNames {
    names: std::collections::HashSet<String>,
}

impl CursorNames {
    pub fn load(store: &Store) -> Self {
        let names = fs::read_dir(store.cursors_dir())
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        CursorNames { names }
    }

    /// [`cursor_file`]'s resolution against the listing: modern name first,
    /// then the legacy one, `None` when neither exists.
    fn path(&self, store: &Store, session: &str, suffix: &str) -> Option<PathBuf> {
        [
            format!("{}{suffix}", safe_session_filename(session)),
            format!("{}{suffix}", safe_legacy(session)),
        ]
        .into_iter()
        .find(|name| self.names.contains(name))
        .map(|name| store.cursors_dir().join(name))
    }

    /// [`read_brief_state`] without touching the disk for absent files.
    pub fn read_brief_state(&self, store: &Store, session: &str) -> BriefState {
        self.path(store, session, ".brief")
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// [`read_live_state`] without touching the disk for absent files.
    pub fn read_live_state(&self, store: &Store, session: &str) -> LiveState {
        self.path(store, session, ".live")
            .and_then(|p| fs::read_to_string(p).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
}

/// Read inline-channel state; missing or corrupt resets to defaults (E4).
pub fn read_live_state(store: &Store, session: &str) -> LiveState {
    fs::read_to_string(live_path(store, session))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist inline-channel state. Best-effort: a cursor write must never break
/// the tool call that triggered it.
pub fn write_live_state(store: &Store, session: &str, state: &LiveState) -> std::io::Result<()> {
    let dir = store.cursors_dir();
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_string(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    fs::write(live_path(store, session), json.as_bytes())
}

/// Read the inbox cursor (last delivered event id), if any.
pub fn read_inbox_cursor(store: &Store, session: &str) -> Option<String> {
    let raw = fs::read_to_string(inbox_path(store, session)).ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Persist the inbox cursor. Best-effort; failure never breaks a session.
pub fn write_inbox_cursor(store: &Store, session: &str, id: &str) -> std::io::Result<()> {
    let dir = store.cursors_dir();
    fs::create_dir_all(&dir)?;
    fs::write(inbox_path(store, session), id.as_bytes())
}

/// Read brief state; a missing or corrupt file resets to defaults (E4).
pub fn read_brief_state(store: &Store, session: &str) -> BriefState {
    fs::read_to_string(brief_path(store, session))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Persist brief state. Best-effort.
pub fn write_brief_state(store: &Store, session: &str, state: &BriefState) -> std::io::Result<()> {
    let dir = store.cursors_dir();
    fs::create_dir_all(&dir)?;
    let json = serde_json::to_string(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    fs::write(brief_path(store, session), json.as_bytes())
}

/// One session's delivery standing, on both channels.
///
/// `seen`/`checked_ms` are the INLINE channel's mark: the hot path handed
/// everything up to `seen` into the agent's context between tool calls —
/// but only notes whose kind interrupts. `brief_seen`/`brief_hb` are the
/// turn-boundary brief's mark, the channel an `fyi` actually rides.
///
/// Keeping both is the difference between "the store scanned past it" and
/// "the agent received it": the inline mark advances over every scanned
/// event, delivered or not, so a receipt that read it alone claimed pickup
/// for `fyi` notes nobody had been served. A receipt must now ask the
/// channel that matches the kind.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delivery {
    pub session: String,
    pub seen: Option<String>,
    pub checked_ms: u64,
    /// Newest event id the turn-boundary brief has surfaced (F6 receipts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub brief_seen: Option<String>,
    /// The brief channel's heartbeat, epoch ms.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub brief_hb: u64,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

/// Every session's delivery standing (F4).
///
/// This is the read that makes per-agent receipts possible: because delivery
/// runs through a cursor rather than a push, the store already knows exactly
/// how far each session has been served. A human chat client cannot answer
/// this — a person has no cursor — which is why "seen by" there is a guess and
/// here it is a fact.
///
/// Sessions are identified by the sanitized file name, so a session id that
/// needed sanitizing reports under that form; callers match on the same
/// transform.
pub fn delivery(store: &Store) -> Vec<Delivery> {
    let dir = store.cursors_dir();
    let Ok(rd) = fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<Delivery> = Vec::new();
    for entry in rd.filter_map(|e| e.ok()) {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(session) = name.strip_suffix(".live") else {
            continue;
        };
        let state: LiveState = fs::read_to_string(entry.path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        out.push(Delivery {
            session: session.to_string(),
            seen: state.seen,
            checked_ms: state.checked_ms,
            brief_seen: None,
            brief_hb: 0,
        });
    }
    // The turn-boundary marks join their sessions: one row per session, both
    // channels. A session whose brief never fired simply has no brief mark —
    // which is itself the answer an fyi receipt needs ("not yet, not even
    // once").
    for entry in fs::read_dir(&dir)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
    {
        let name = entry.file_name().to_string_lossy().into_owned();
        let Some(session) = name.strip_suffix(".brief") else {
            continue;
        };
        let state = read_brief_state(store, session);
        if let Some(row) = out.iter_mut().find(|d| d.session == session) {
            row.brief_seen = state.seen;
            row.brief_hb = state.hb;
        } else {
            out.push(Delivery {
                session: session.to_string(),
                seen: None,
                checked_ms: 0,
                brief_seen: state.seen,
                brief_hb: state.hb,
            });
        }
    }
    out.sort_by(|a, b| a.session.cmp(&b.session));
    out
}

/// Count cursor + brief-state files, for `doctor`.
pub fn count(store: &Store) -> u64 {
    match fs::read_dir(store.cursors_dir()) {
        Ok(rd) => rd.filter_map(|e| e.ok()).count() as u64,
        Err(_) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempHome;

    #[test]
    fn inbox_cursor_round_trips() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        assert_eq!(read_inbox_cursor(&store, "s1"), None);
        write_inbox_cursor(&store, "s1", "01AAA").unwrap();
        assert_eq!(read_inbox_cursor(&store, "s1").as_deref(), Some("01AAA"));
    }

    #[test]
    fn brief_state_defaults_when_absent_or_corrupt() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let st = read_brief_state(&store, "s1");
        assert!(st.seen.is_none());
        assert_eq!(st.hb, 0);
        // Corrupt file -> defaults, never panics.
        fs::create_dir_all(store.cursors_dir()).unwrap();
        fs::write(brief_path(&store, "s1"), b"{not json").unwrap();
        let st = read_brief_state(&store, "s1");
        assert!(st.seen.is_none());
    }

    #[test]
    fn session_id_with_slashes_is_sanitized() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        write_inbox_cursor(&store, "a/b/c", "01AAA").unwrap();
        assert_eq!(read_inbox_cursor(&store, "a/b/c").as_deref(), Some("01AAA"));
        assert!(
            store
                .cursors_dir()
                .join(safe_session_filename("a/b/c"))
                .exists()
        );
    }

    #[test]
    fn slash_and_underscore_sessions_do_not_collide() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        write_live_state(
            &store,
            "a/b",
            &LiveState {
                seen: Some("01AAA".into()),
                checked_ms: 1,
                ..Default::default()
            },
        )
        .unwrap();
        write_live_state(
            &store,
            "a_b",
            &LiveState {
                seen: Some("01BBB".into()),
                checked_ms: 2,
                ..Default::default()
            },
        )
        .unwrap();
        assert_ne!(safe_session_filename("a/b"), safe_session_filename("a_b"));
        assert_eq!(
            read_live_state(&store, "a/b").seen.as_deref(),
            Some("01AAA")
        );
        assert_eq!(
            read_live_state(&store, "a_b").seen.as_deref(),
            Some("01BBB")
        );
        let rows = delivery(&store);
        assert!(
            rows.iter()
                .any(|d| d.session == safe_session_filename("a/b"))
        );
        assert!(
            rows.iter()
                .any(|d| d.session == safe_session_filename("a_b"))
        );
    }

    #[test]
    fn legacy_sanitized_cursor_still_reads() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        fs::create_dir_all(store.cursors_dir()).unwrap();
        // Old layout: a/b → a_b
        fs::write(
            store.cursors_dir().join("a_b.live"),
            br#"{"seen":"01LEG","checked_ms":1}"#,
        )
        .unwrap();
        let st = read_live_state(&store, "a/b");
        assert_eq!(st.seen.as_deref(), Some("01LEG"));
    }

    #[test]
    fn cursor_names_read_what_the_direct_readers_read() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        write_brief_state(
            &store,
            "s1",
            &BriefState {
                hb: 7,
                ..Default::default()
            },
        )
        .unwrap();
        write_live_state(
            &store,
            "a/b",
            &LiveState {
                checked_ms: 9,
                ..Default::default()
            },
        )
        .unwrap();
        // Legacy-only layout, and a corrupt file.
        fs::write(store.cursors_dir().join("c_d.brief"), br#"{"hb":3}"#).unwrap();
        fs::write(store.cursors_dir().join("bad.live"), b"{not json").unwrap();
        let names = CursorNames::load(&store);
        for session in ["s1", "a/b", "c/d", "bad", "never-seen"] {
            assert_eq!(
                names.read_brief_state(&store, session).hb,
                read_brief_state(&store, session).hb,
                "{session}"
            );
            assert_eq!(
                names.read_live_state(&store, session).checked_ms,
                read_live_state(&store, session).checked_ms,
                "{session}"
            );
        }
        assert_eq!(names.read_brief_state(&store, "c/d").hb, 3);
        assert_eq!(names.read_live_state(&store, "a/b").checked_ms, 9);
        // No cursor directory at all is every session absent.
        let empty = Store::at(home.path().join("none"));
        assert_eq!(
            CursorNames::load(&empty).read_brief_state(&empty, "s1").hb,
            0
        );
    }

    #[test]
    fn delivery_joins_inline_and_brief_marks_per_session() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        write_live_state(
            &store,
            "s1",
            &LiveState {
                seen: Some("01AAA".to_string()),
                checked_ms: 5,
                ..Default::default()
            },
        )
        .unwrap();
        write_brief_state(
            &store,
            "s1",
            &BriefState {
                seen: Some("01999".to_string()),
                hb: 7,
                ..Default::default()
            },
        )
        .unwrap();
        // A session with only a brief mark (never fired the inline hook) still
        // gets a row — its fyi answer must exist.
        write_brief_state(
            &store,
            "s2",
            &BriefState {
                seen: Some("01BBB".to_string()),
                hb: 9,
                ..Default::default()
            },
        )
        .unwrap();

        let rows = delivery(&store);
        let s1 = rows.iter().find(|d| d.session == "s1").expect("s1 row");
        assert_eq!(s1.seen.as_deref(), Some("01AAA"));
        assert_eq!(s1.brief_seen.as_deref(), Some("01999"));
        assert_eq!(s1.brief_hb, 7);
        let s2 = rows.iter().find(|d| d.session == "s2").expect("s2 row");
        assert_eq!(s2.seen, None);
        assert_eq!(s2.brief_seen.as_deref(), Some("01BBB"));
    }
}
