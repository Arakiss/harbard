//! The operator's one-tap feedback, on the bus, with a derived SQLite index.
//!
//! Every tap in the console (a 👎 on a message, "Did not need me" on a
//! card, "This bothered me" on a Dock bounce…) is one `feedback` event on the
//! append-only bus: `{target kind, target id, value, optional reason}`. The
//! last tap can be undone by a second event that names the first. The bus is
//! the authority; `feedback/index.sqlite` answers the queries and is rebuilt
//! from the bus whenever it is missing, of another schema or failed a write.
//!
//! A name's 👍 / 👎 is not here: the workshop keeps those votes in
//! `names.json` ([`crate::workshop`]), the one source the naming generator
//! and the retro read.
//!
//! Only operator events count (`harness: human`, `role: operator`, which only
//! `serve` writes): an agent cannot vote on its own messages or forge a rule.
//!
//! Reasons are the operator's own words. They are stored as one bounded line
//! with every control and bidi character removed, and rendered as plain text
//! until the shared `redact` module lands. Samples carry a state and a safe
//! label, never transcript text.

use crate::event::{Event, EventKind, NoteKind};
use crate::store::{Store, Window};
use crate::workspace::DeskState;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

/// Payload version inside `text`.
pub const PAYLOAD_V: u32 = 1;
/// Index schema; a mismatch rebuilds.
const INDEX_SCHEMA: u32 = 1;
/// Characters of a reason kept (one line).
pub const MAX_REASON_CHARS: usize = 200;
/// Bytes of a target id.
pub const MAX_TARGET_ID_BYTES: usize = 200;
/// Characters of a sample state and of a sample label.
const MAX_STATE_CHARS: usize = 24;
const MAX_LABEL_CHARS: usize = 60;

/// What a tap was about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetKind {
    /// An agent message: 👍 / 👎 "I do not understand it".
    Message,
    /// A delivery (DONE): "Is this useful?" Yes / No / Something missing.
    Delivery,
    /// A "Needs you" card: "Did not need me".
    NeedsYou,
    /// A robot's derived state: "This is not what it is doing".
    RobotState,
    /// A Dock bounce or an alert: "This bothered me".
    Dock,
}

impl TargetKind {
    pub const ALL: [TargetKind; 5] = [
        TargetKind::Message,
        TargetKind::Delivery,
        TargetKind::NeedsYou,
        TargetKind::RobotState,
        TargetKind::Dock,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            TargetKind::Message => "message",
            TargetKind::Delivery => "delivery",
            TargetKind::NeedsYou => "needs_you",
            TargetKind::RobotState => "robot_state",
            TargetKind::Dock => "dock",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|k| k.as_str() == s)
    }

    /// The values a tap on this target may carry.
    pub fn values(self) -> &'static [&'static str] {
        match self {
            TargetKind::Message => &["up", "unclear"],
            TargetKind::Delivery => &["yes", "no", "missing"],
            TargetKind::NeedsYou => &["not_needed"],
            TargetKind::RobotState => &["wrong"],
            TargetKind::Dock => &["annoying"],
        }
    }
}

/// The value an undo event carries.
pub const UNDO: &str = "undo";

/// What the console saw when the tap happened: a state and a safe label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sample {
    // The adapter exists only to read old events, which are append-only.
    #[serde(deserialize_with = "crate::old_spelling::read_activity_state")]
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// The JSON in a `feedback` event's `text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload {
    pub v: u32,
    pub target_kind: TargetKind,
    pub target_id: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sample: Option<Sample>,
    /// For an undo: the feedback event it takes back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undoes: Option<String>,
}

/// One tap as the projection and the index keep it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Record {
    pub id: String,
    pub ms: u64,
    pub target_kind: TargetKind,
    pub target_id: String,
    pub value: String,
    pub reason: Option<String>,
    pub sample: Option<Sample>,
    pub undone: bool,
}

/// What a tap did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Recorded(Record),
    /// The target already had an answer: never asked twice, nothing appended.
    AlreadyGiven(Record),
}

impl Outcome {
    pub fn record(&self) -> &Record {
        match self {
            Outcome::Recorded(r) | Outcome::AlreadyGiven(r) => r,
        }
    }
}

#[derive(Debug)]
pub enum FeedbackError {
    Invalid(&'static str, String),
    NothingToUndo,
    Io(std::io::Error),
    Sql(rusqlite::Error),
}

impl std::fmt::Display for FeedbackError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FeedbackError::Invalid(field, msg) => write!(f, "{field}: {msg}"),
            FeedbackError::NothingToUndo => write!(f, "there is no feedback to undo"),
            FeedbackError::Io(e) => write!(f, "{e}"),
            FeedbackError::Sql(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for FeedbackError {}

impl From<std::io::Error> for FeedbackError {
    fn from(e: std::io::Error) -> Self {
        FeedbackError::Io(e)
    }
}

impl From<rusqlite::Error> for FeedbackError {
    fn from(e: rusqlite::Error) -> Self {
        FeedbackError::Sql(e)
    }
}

// ---------------------------------------------------------------------------
// Validation
// ---------------------------------------------------------------------------

/// One line of the operator's own text: controls, line and paragraph
/// separators and bidi controls become spaces, runs of spaces collapse, and
/// the result is cut to `max_chars`. Empty is `None`.
pub fn one_line(s: &str, max_chars: usize) -> Option<String> {
    // The brief's sanitizer is the one that is adversarially reviewed; its
    // byte cap is set wide so the character cap below decides.
    let clean = crate::brief::sanitize_inline(s, s.len().max(1) * 4);
    let collapsed = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    let cut: String = collapsed.chars().take(max_chars).collect();
    let cut = cut.trim().to_string();
    (!cut.is_empty()).then_some(cut)
}

fn target_id_ok(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_TARGET_ID_BYTES
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.:@/#+=".contains(c))
}

fn str_field<'a>(body: &'a Value, name: &'static str) -> Result<&'a str, FeedbackError> {
    body.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| FeedbackError::Invalid(name, format!("field `{name}` is required")))
}

/// A sample's state is one word of the face vocabulary; its label is the
/// console's safe label, dropped when any word of it reads like a secret.
fn parse_sample(v: &Value) -> Result<Sample, FeedbackError> {
    let state = v
        .get("state")
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or("");
    let word = !state.is_empty()
        && state.chars().count() <= MAX_STATE_CHARS
        && state
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-');
    if !word {
        return Err(FeedbackError::Invalid(
            "sample",
            "a sample state is one short word".into(),
        ));
    }
    let label = v
        .get("label")
        .and_then(Value::as_str)
        .and_then(|l| one_line(l, MAX_LABEL_CHARS))
        .filter(|l| !l.split_whitespace().any(crate::activity::looks_secret));
    Ok(Sample {
        state: state.to_string(),
        label,
    })
}

/// Validate a tap from the console: `{target_kind, target_id, value,
/// reason?, sample?}`. Unknown fields are ignored and never stored.
pub fn parse_input(body: &Value) -> Result<Payload, FeedbackError> {
    let kind_s = str_field(body, "target_kind")?;
    let target_kind = TargetKind::parse(kind_s).ok_or_else(|| {
        FeedbackError::Invalid("target_kind", format!("unknown target kind \"{kind_s}\""))
    })?;
    let target_id = str_field(body, "target_id")?;
    if !target_id_ok(target_id) {
        return Err(FeedbackError::Invalid(
            "target_id",
            "a target id is a short token of letters, digits and - _ . : @ / # + =".into(),
        ));
    }
    let value = str_field(body, "value")?;
    if !target_kind.values().contains(&value) {
        return Err(FeedbackError::Invalid(
            "value",
            format!(
                "\"{value}\" is not a value for {} (one of: {})",
                target_kind.as_str(),
                target_kind.values().join(", ")
            ),
        ));
    }
    let sample = match body.get("sample") {
        None | Some(Value::Null) => None,
        Some(v) => Some(parse_sample(v)?),
    };
    if target_kind == TargetKind::RobotState && sample.is_none() {
        return Err(FeedbackError::Invalid(
            "sample",
            "feedback on a robot's state carries the state it showed".into(),
        ));
    }
    let reason = body
        .get("reason")
        .and_then(Value::as_str)
        .and_then(|r| one_line(r, MAX_REASON_CHARS));
    Ok(Payload {
        v: PAYLOAD_V,
        target_kind,
        target_id: target_id.to_string(),
        value: value.to_string(),
        reason,
        sample,
        undoes: None,
    })
}

// ---------------------------------------------------------------------------
// Projection
// ---------------------------------------------------------------------------

/// Every operator tap in log order, with undos applied.
#[derive(Debug, Clone, Default)]
pub struct Projection {
    records: Vec<Record>,
}

impl Projection {
    pub fn from_events(events: &[Event]) -> Self {
        let mut p = Self::default();
        for e in events {
            p.apply(e);
        }
        p
    }

    pub fn apply(&mut self, event: &Event) {
        let Some(payload) = decode(event) else {
            return;
        };
        if payload.value == UNDO {
            if let Some(target) = payload.undoes.as_deref()
                && let Some(r) = self.records.iter_mut().find(|r| r.id == target)
            {
                r.undone = true;
            }
            return;
        }
        if self.records.iter().any(|r| r.id == event.id) {
            return;
        }
        self.records.push(Record {
            id: event.id.clone(),
            ms: crate::query::event_ms(event),
            target_kind: payload.target_kind,
            target_id: payload.target_id,
            value: payload.value,
            reason: payload.reason,
            sample: payload.sample,
            undone: false,
        });
    }

    pub fn records(&self) -> &[Record] {
        &self.records
    }

    pub fn active(&self) -> impl Iterator<Item = &Record> {
        self.records.iter().filter(|r| !r.undone)
    }

    pub fn last_active(&self) -> Option<&Record> {
        self.active().last()
    }
}

/// Decode an operator `feedback` event; anything else is `None`. A payload
/// that fails today's validation (a hand-edited line) is skipped, not trusted.
pub fn decode(event: &Event) -> Option<Payload> {
    if event.event != EventKind::Feedback || !event.is_operator() {
        return None;
    }
    let p: Payload = serde_json::from_str(&event.text).ok()?;
    if !target_id_ok(&p.target_id) {
        return None;
    }
    let value_ok = if p.value == UNDO {
        p.undoes.as_deref().is_some_and(crate::event::is_event_id)
    } else {
        p.target_kind.values().contains(&p.value.as_str())
    };
    value_ok.then_some(p)
}

// ---------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------

fn not_operator() -> FeedbackError {
    FeedbackError::Invalid("author", "only the operator gives feedback".into())
}

fn feedback_event(mut base: Event, payload: &Payload) -> Result<Event, FeedbackError> {
    base.event = EventKind::Feedback;
    base.kind = NoteKind::Fyi;
    base.to = None;
    base.note_ref = payload.undoes.clone();
    base.text = serde_json::to_string(payload)
        .map_err(|e| FeedbackError::Invalid("payload", e.to_string()))?;
    base.to_line()
        .map_err(|e| FeedbackError::Invalid("payload", e.to_string()))?;
    Ok(base)
}

/// Append, then index under the same lock. An index write that fails removes
/// the index, so the next read rebuilds it from the bus rather than answer
/// from a copy that missed an event.
fn append_indexed(store: &Store, event: &Event) -> Result<(), FeedbackError> {
    store.append(event)?;
    if with_conn(store, |conn| index_event(conn, event)).is_err() {
        drop_index(store);
    }
    Ok(())
}

pub(crate) fn locked<T>(
    store: &Store,
    f: impl FnOnce() -> Result<T, FeedbackError>,
) -> Result<T, FeedbackError> {
    DeskState::with_lock(&DeskState::path(store.root()), f)?
}

/// Record one tap under the desk lock. `base` is the operator event to send
/// (fresh id and timestamp); its kind, text and recipient are set here. A
/// target that already has a standing answer is never asked twice: the
/// earlier answer comes back and nothing is appended.
pub fn record(store: &Store, base: Event, payload: Payload) -> Result<Outcome, FeedbackError> {
    if !base.is_operator() {
        return Err(not_operator());
    }
    if payload.value == UNDO {
        return Err(FeedbackError::Invalid("value", "use undo".into()));
    }
    locked(store, || {
        ensure_unlocked(store)?;
        let standing = active_for_unlocked(
            store,
            payload.target_kind,
            std::slice::from_ref(&payload.target_id),
        )?;
        if let Some(r) = standing.into_iter().next() {
            return Ok(Outcome::AlreadyGiven(r));
        }
        let event = feedback_event(base, &payload)?;
        append_indexed(store, &event)?;
        Ok(Outcome::Recorded(Record {
            id: event.id.clone(),
            ms: crate::query::event_ms(&event),
            target_kind: payload.target_kind,
            target_id: payload.target_id,
            value: payload.value,
            reason: payload.reason,
            sample: payload.sample,
            undone: false,
        }))
    })
}

/// Take back `id`, or the newest tap still standing.
pub fn undo(store: &Store, base: Event, id: Option<&str>) -> Result<Record, FeedbackError> {
    if !base.is_operator() {
        return Err(not_operator());
    }
    locked(store, || {
        ensure_unlocked(store)?;
        let target = match id {
            Some(id) => with_conn(store, |c| row_by_id(c, id))?.filter(|r| !r.undone),
            None => with_conn(store, last_active_row)?,
        }
        .ok_or(FeedbackError::NothingToUndo)?;
        let payload = Payload {
            v: PAYLOAD_V,
            target_kind: target.target_kind,
            target_id: target.target_id.clone(),
            value: UNDO.to_string(),
            reason: None,
            sample: None,
            undoes: Some(target.id.clone()),
        };
        let event = feedback_event(base, &payload)?;
        append_indexed(store, &event)?;
        Ok(Record {
            undone: true,
            ..target
        })
    })
}

// ---------------------------------------------------------------------------
// Index
// ---------------------------------------------------------------------------

pub fn db_path(store: &Store) -> PathBuf {
    store.root().join("feedback").join("index.sqlite")
}

fn drop_index(store: &Store) {
    let path = db_path(store);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(format!("{}-wal", path.display()));
    let _ = fs::remove_file(format!("{}-shm", path.display()));
}

fn open(path: &std::path::Path) -> Result<Connection, FeedbackError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let conn = Connection::open(path)?;
    conn.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=NORMAL;",
    )?;
    Ok(conn)
}

pub(crate) fn with_conn<T>(
    store: &Store,
    f: impl FnOnce(&Connection) -> Result<T, FeedbackError>,
) -> Result<T, FeedbackError> {
    let conn = open(&db_path(store))?;
    schema(&conn)?;
    f(&conn)
}

fn schema(conn: &Connection) -> Result<(), FeedbackError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS meta (
            key TEXT PRIMARY KEY NOT NULL,
            value TEXT NOT NULL
         );
         CREATE TABLE IF NOT EXISTS feedback (
            id TEXT PRIMARY KEY NOT NULL,
            ms INTEGER NOT NULL,
            target_kind TEXT NOT NULL,
            target_id TEXT NOT NULL,
            value TEXT NOT NULL,
            reason TEXT,
            sample_state TEXT,
            sample_label TEXT,
            undone INTEGER NOT NULL DEFAULT 0
         );
         CREATE INDEX IF NOT EXISTS feedback_target ON feedback(target_kind, target_id);
         CREATE TABLE IF NOT EXISTS rule_events (
            id TEXT PRIMARY KEY NOT NULL,
            ms INTEGER NOT NULL,
            body TEXT NOT NULL
         );",
    )?;
    Ok(())
}

fn set_meta(conn: &Connection, key: &str, value: &str) -> Result<(), FeedbackError> {
    conn.execute(
        "INSERT INTO meta(key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![key, value],
    )?;
    Ok(())
}

fn schema_version(conn: &Connection) -> Option<u32> {
    conn.query_row(
        "SELECT value FROM meta WHERE key = 'schema_version'",
        [],
        |r| r.get::<_, String>(0),
    )
    .optional()
    .ok()
    .flatten()
    .and_then(|v| v.parse().ok())
}

/// Index one bus event: a tap, an undo, or a rule event (kept whole for
/// [`crate::rules`] to project).
fn index_event(conn: &Connection, event: &Event) -> Result<(), FeedbackError> {
    if event.event == EventKind::Rule {
        if event.is_operator() {
            conn.execute(
                "INSERT OR IGNORE INTO rule_events (id, ms, body) VALUES (?1, ?2, ?3)",
                params![event.id, crate::query::event_ms(event) as i64, event.text],
            )?;
        }
        return Ok(());
    }
    let Some(p) = decode(event) else {
        return Ok(());
    };
    if p.value == UNDO {
        conn.execute(
            "UPDATE feedback SET undone = 1 WHERE id = ?1",
            params![p.undoes],
        )?;
        return Ok(());
    }
    let (state, label) = match &p.sample {
        Some(s) => (Some(s.state.clone()), s.label.clone()),
        None => (None, None),
    };
    conn.execute(
        "INSERT OR IGNORE INTO feedback
            (id, ms, target_kind, target_id, value, reason, sample_state, sample_label, undone)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0)",
        params![
            event.id,
            crate::query::event_ms(event) as i64,
            p.target_kind.as_str(),
            p.target_id,
            p.value,
            p.reason,
            state,
            label,
        ],
    )?;
    Ok(())
}

/// Append an operator `rule` event and index it, under a lock the caller
/// holds ([`locked`]).
pub(crate) fn append_rule_unlocked(store: &Store, event: &Event) -> Result<(), FeedbackError> {
    ensure_unlocked(store)?;
    append_indexed(store, event)
}

/// Rule event bodies, in log order, as the index keeps them. A read: it
/// never builds the index.
pub(crate) fn rule_event_rows(store: &Store) -> Result<Vec<(String, u64, String)>, FeedbackError> {
    with_read(store, rule_rows_in)
}

/// The same, for a caller that already holds the lock ([`locked`]).
pub(crate) fn rule_event_rows_unlocked(
    store: &Store,
) -> Result<Vec<(String, u64, String)>, FeedbackError> {
    ensure_unlocked(store)?;
    rule_rows_of(store)
}

fn rule_rows_of(store: &Store) -> Result<Vec<(String, u64, String)>, FeedbackError> {
    with_conn(store, rule_rows_in)
}

fn rule_rows_in(conn: &Connection) -> Result<Vec<(String, u64, String)>, FeedbackError> {
    let mut stmt = conn.prepare("SELECT id, ms, body FROM rule_events ORDER BY id")?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, i64>(1)? as u64,
            r.get::<_, String>(2)?,
        ))
    })?;
    Ok(rows.collect::<Result<Vec<_>, _>>()?)
}

/// Index every feedback and rule event of `events` into `conn`, and stamp
/// the schema. Returns the feedback events indexed.
fn fill(conn: &Connection, events: &[Event]) -> Result<u64, FeedbackError> {
    let mut indexed = 0u64;
    let tx = conn.unchecked_transaction()?;
    for ev in events {
        match ev.event {
            EventKind::Feedback if decode(ev).is_some() => {
                index_event(&tx, ev)?;
                indexed += 1;
            }
            EventKind::Rule => index_event(&tx, ev)?,
            _ => {}
        }
    }
    set_meta(&tx, "schema_version", &INDEX_SCHEMA.to_string())?;
    tx.commit()?;
    Ok(indexed)
}

/// The index on disk is there and of this schema.
fn fresh(path: &std::path::Path) -> bool {
    path.exists()
        && open(path)
            .ok()
            .is_some_and(|c| schema(&c).is_ok() && schema_version(&c) == Some(INDEX_SCHEMA))
}

/// A read: the index when it is there and current; otherwise a
/// throwaway copy in memory, built from the bus, so that a GET, a hook's
/// brief or a CLI read never creates or rebuilds the file. serve builds it
/// at start, and every write keeps it ([`ensure`]).
fn with_read<T>(
    store: &Store,
    f: impl FnOnce(&Connection) -> Result<T, FeedbackError>,
) -> Result<T, FeedbackError> {
    if fresh(&db_path(store)) {
        return with_conn(store, f);
    }
    let conn = Connection::open_in_memory()?;
    schema(&conn)?;
    fill(&conn, &store.read(Window::All).events)?;
    f(&conn)
}

/// Rebuild the index from every feedback and rule event on the bus, into a
/// sibling file renamed into place. Returns the feedback events indexed.
pub fn rebuild(store: &Store) -> Result<u64, FeedbackError> {
    locked(store, || rebuild_unlocked(store))
}

fn rebuild_unlocked(store: &Store) -> Result<u64, FeedbackError> {
    let path = db_path(store);
    let tmp = path.with_extension(format!("sqlite.tmp.{}", std::process::id()));
    let tmp_s = tmp.display().to_string();
    for p in [
        tmp_s.clone(),
        format!("{tmp_s}-wal"),
        format!("{tmp_s}-shm"),
    ] {
        let _ = fs::remove_file(p);
    }
    let read = store.read(Window::All);
    let indexed;
    {
        let conn = open(&tmp)?;
        schema(&conn)?;
        indexed = fill(&conn, &read.events)?;
        conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")?;
    }
    drop_index(store);
    fs::rename(&tmp, &path)?;
    Ok(indexed)
}

fn ensure_unlocked(store: &Store) -> Result<(), FeedbackError> {
    if !fresh(&db_path(store)) {
        rebuild_unlocked(store)?;
    }
    Ok(())
}

/// Open the index, rebuilding it when missing, unreadable or of another
/// schema. For serve's start and the writes; reads go through
/// [`with_read`] and never build it.
pub fn ensure(store: &Store) -> Result<(), FeedbackError> {
    if fresh(&db_path(store)) {
        return Ok(());
    }
    locked(store, || ensure_unlocked(store))
}

fn row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Record> {
    let kind: String = r.get(2)?;
    let state: Option<String> = r.get(6)?;
    let label: Option<String> = r.get(7)?;
    Ok(Record {
        id: r.get(0)?,
        ms: r.get::<_, i64>(1)? as u64,
        target_kind: TargetKind::parse(&kind).unwrap_or(TargetKind::Message),
        target_id: r.get(3)?,
        value: r.get(4)?,
        reason: r.get(5)?,
        sample: state.map(|state| Sample {
            state: crate::old_spelling::activity_state(&state).to_string(),
            label,
        }),
        undone: r.get::<_, i64>(8)? != 0,
    })
}

const COLS: &str =
    "id, ms, target_kind, target_id, value, reason, sample_state, sample_label, undone";

fn row_by_id(conn: &Connection, id: &str) -> Result<Option<Record>, FeedbackError> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM feedback WHERE id = ?1"),
            params![id],
            row,
        )
        .optional()?)
}

fn last_active_row(conn: &Connection) -> Result<Option<Record>, FeedbackError> {
    Ok(conn
        .query_row(
            &format!("SELECT {COLS} FROM feedback WHERE undone = 0 ORDER BY id DESC LIMIT 1"),
            [],
            row,
        )
        .optional()?)
}

fn active_for_unlocked(
    store: &Store,
    kind: TargetKind,
    ids: &[String],
) -> Result<Vec<Record>, FeedbackError> {
    with_conn(store, |conn| active_for_in(conn, kind, ids))
}

fn active_for_in(
    conn: &Connection,
    kind: TargetKind,
    ids: &[String],
) -> Result<Vec<Record>, FeedbackError> {
    let mut stmt = conn.prepare(&format!(
        "SELECT {COLS} FROM feedback
         WHERE undone = 0 AND target_kind = ?1 AND target_id = ?2
         ORDER BY id DESC LIMIT 1"
    ))?;
    let mut out = Vec::new();
    for id in ids {
        if let Some(r) = stmt.query_row(params![kind.as_str(), id], row).optional()? {
            out.push(r);
        }
    }
    Ok(out)
}

/// The standing answer for each of `ids` of one kind (ids with none are
/// absent from the result).
pub fn active_for(
    store: &Store,
    kind: TargetKind,
    ids: &[String],
) -> Result<Vec<Record>, FeedbackError> {
    with_read(store, |conn| active_for_in(conn, kind, ids))
}

/// Every standing tap of one kind, newest first, at most `limit`.
pub fn active_of_kind(
    store: &Store,
    kind: TargetKind,
    limit: usize,
) -> Result<Vec<Record>, FeedbackError> {
    with_read(store, |conn| {
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS} FROM feedback WHERE undone = 0 AND target_kind = ?1
             ORDER BY id DESC LIMIT ?2"
        ))?;
        let rows = stmt.query_map(params![kind.as_str(), limit as i64], row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    })
}

/// The newest tap still standing: what "Undo" takes back.
pub fn last_active(store: &Store) -> Result<Option<Record>, FeedbackError> {
    with_read(store, last_active_row)
}

/// Standing taps per target kind and value.
pub fn totals(store: &Store) -> Result<BTreeMap<String, BTreeMap<String, u64>>, FeedbackError> {
    with_read(store, |conn| {
        let mut stmt = conn.prepare(
            "SELECT target_kind, value, COUNT(*) FROM feedback WHERE undone = 0
             GROUP BY target_kind, value",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, i64>(2)? as u64,
            ))
        })?;
        let mut out: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
        for r in rows {
            let (k, v, n) = r?;
            out.entry(k).or_default().insert(v, n);
        }
        Ok(out)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{HARNESS_HUMAN, Role, SCHEMA_VERSION};
    use crate::test_util::TempHome;
    use serde_json::json;
    use ulid::Ulid;

    const T0: u64 = 1_793_000_000_000;

    fn base(ms: u64, operator: bool) -> Event {
        Event {
            v: SCHEMA_VERSION,
            id: Ulid::from_parts(ms, 1).to_string(),
            ts: crate::clock::rfc3339_utc(ms),
            event: EventKind::Note,
            session: if operator {
                "operator@test".into()
            } else {
                "agent-1".into()
            },
            harness: if operator {
                HARNESS_HUMAN.into()
            } else {
                "claude-code".into()
            },
            host: "test".into(),
            cwd: String::new(),
            repo: None,
            repo_key: None,
            branch: None,
            role: if operator { Role::Operator } else { Role::Main },
            parent: None,
            text: String::new(),
            to: None,
            kind: NoteKind::Fyi,
            note_ref: None,
        }
    }

    fn tap(kind: &str, id: &str, value: &str) -> Payload {
        parse_input(&json!({"target_kind": kind, "target_id": id, "value": value})).unwrap()
    }

    #[test]
    fn one_line_strips_controls_and_bidi_and_caps() {
        assert_eq!(
            one_line("  no\nlo\u{2028}entiendo\u{202E} ", 200).as_deref(),
            Some("no lo entiendo")
        );
        assert_eq!(one_line(" \n\t ", 200), None);
        let long = "a".repeat(500);
        assert_eq!(one_line(&long, 200).unwrap().chars().count(), 200);
    }

    #[test]
    fn input_is_validated_per_target_kind() {
        let ok = tap("delivery", "d-abc123", "missing");
        assert_eq!(ok.target_kind, TargetKind::Delivery);
        assert_eq!(ok.value, "missing");
        for bad in [
            json!({"target_kind": "nope", "target_id": "x", "value": "up"}),
            json!({"target_kind": "message", "target_id": "x", "value": "missing"}),
            json!({"target_kind": "message", "target_id": "", "value": "up"}),
            json!({"target_kind": "message", "target_id": "a b", "value": "up"}),
            json!({"target_kind": "message", "target_id": "x", "value": "undo"}),
            json!({"target_kind": "robot_state", "target_id": "agent-1", "value": "wrong"}),
        ] {
            assert!(parse_input(&bad).is_err(), "{bad}");
        }
        let long_id = "x".repeat(MAX_TARGET_ID_BYTES + 1);
        assert!(
            parse_input(&json!({"target_kind": "message", "target_id": long_id, "value": "up"}))
                .is_err()
        );
    }

    #[test]
    fn a_name_is_not_a_feedback_target_the_workshop_owns_its_votes() {
        // 👍 / 👎 on a name are workshop votes (names.json), the one source
        // the naming generator and the retro read.
        assert!(
            parse_input(&json!({"target_kind": "name", "target_id": "a1", "value": "down"}))
                .is_err()
        );
    }

    #[test]
    fn a_robot_sample_keeps_only_a_state_and_a_safe_label() {
        let p = parse_input(&json!({
            "target_kind": "robot_state",
            "target_id": "agent-1",
            "value": "wrong",
            "sample": {"state": "running", "label": "cargo test", "transcript": "SECRET BODY"},
            "reason": "is waiting\nfor a permission",
        }))
        .unwrap();
        let s = p.sample.unwrap();
        assert_eq!(s.state, "running");
        assert_eq!(s.label.as_deref(), Some("cargo test"));
        assert_eq!(p.reason.as_deref(), Some("is waiting for a permission"));
        // A label that reads like a credential is dropped, the state kept.
        let p = parse_input(&json!({
            "target_kind": "robot_state",
            "target_id": "agent-1",
            "value": "wrong",
            "sample": {"state": "web", "label": concat!("ghp_abcd", "efghijklmnopqrstuvwxyz0123456789")},
        }))
        .unwrap();
        assert_eq!(p.sample.unwrap().label, None);
        // A state is a word, not prose.
        assert!(
            parse_input(&json!({
                "target_kind": "robot_state",
                "target_id": "agent-1",
                "value": "wrong",
                "sample": {"state": "rm -rf /"},
            }))
            .is_err()
        );
    }

    #[test]
    fn a_tap_appends_to_the_bus_and_is_indexed() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let out = record(&store, base(T0, true), tap("message", "m-1", "unclear")).unwrap();
        assert!(matches!(out, Outcome::Recorded(_)));
        let read = store.read(Window::All);
        let fb: Vec<&Event> = read
            .events
            .iter()
            .filter(|e| e.event == EventKind::Feedback)
            .collect();
        assert_eq!(fb.len(), 1, "one feedback event on the bus");
        assert_eq!(decode(fb[0]).unwrap().value, "unclear");
        let got = active_for(&store, TargetKind::Message, &["m-1".into()]).unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].value, "unclear");
        assert_eq!(totals(&store).unwrap()["message"]["unclear"], 1);
    }

    #[test]
    fn the_same_target_is_never_asked_twice() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        record(
            &store,
            base(T0, true),
            tap("needs_you", "card-1", "not_needed"),
        )
        .unwrap();
        let again = record(
            &store,
            base(T0 + 1, true),
            tap("needs_you", "card-1", "not_needed"),
        )
        .unwrap();
        assert!(matches!(again, Outcome::AlreadyGiven(_)));
        let n = store
            .read(Window::All)
            .events
            .iter()
            .filter(|e| e.event == EventKind::Feedback)
            .count();
        assert_eq!(n, 1, "nothing appended for a second tap");
    }

    #[test]
    fn undo_takes_back_the_last_tap_and_the_index_agrees() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        record(
            &store,
            base(T0, true),
            tap("needs_you", "card-9", "not_needed"),
        )
        .unwrap();
        record(
            &store,
            base(T0 + 10, true),
            tap("dock", "bounce-1", "annoying"),
        )
        .unwrap();
        let undone = undo(&store, base(T0 + 20, true), None).unwrap();
        assert_eq!(undone.target_kind, TargetKind::Dock);
        assert!(
            active_for(&store, TargetKind::Dock, &["bounce-1".into()])
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            last_active(&store).unwrap().map(|r| r.target_id),
            Some("card-9".into())
        );
        // Undone, the target may be answered again.
        let again = record(
            &store,
            base(T0 + 30, true),
            tap("dock", "bounce-1", "annoying"),
        )
        .unwrap();
        assert!(matches!(again, Outcome::Recorded(_)));
        // Undoing what is already undone, or nothing, is refused.
        assert!(matches!(
            undo(&store, base(T0 + 40, true), Some(&undone.id)),
            Err(FeedbackError::NothingToUndo)
        ));
    }

    #[test]
    fn the_index_rebuilds_from_the_bus_alone() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        record(&store, base(T0, true), tap("message", "m-1", "up")).unwrap();
        record(&store, base(T0 + 1, true), tap("delivery", "d-1", "no")).unwrap();
        undo(&store, base(T0 + 2, true), None).unwrap();
        let before = totals(&store).unwrap();
        fs::remove_file(db_path(&store)).unwrap();
        assert_eq!(totals(&store).unwrap(), before, "ensure rebuilt it");
        // A corrupt index is replaced too.
        fs::write(db_path(&store), b"not sqlite").unwrap();
        assert_eq!(rebuild(&store).unwrap(), 3);
        assert_eq!(totals(&store).unwrap(), before);
        assert_eq!(before["message"]["up"], 1);
        assert!(!before.contains_key("delivery"), "the undone tap is gone");
    }

    #[test]
    fn only_the_operator_counts() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let mut forged = base(T0, false);
        forged.event = EventKind::Feedback;
        forged.text = serde_json::to_string(&tap("message", "m-1", "up")).unwrap();
        store.append(&forged).unwrap();
        assert!(decode(&forged).is_none());
        assert_eq!(rebuild(&store).unwrap(), 0);
        assert!(totals(&store).unwrap().is_empty());
        // And an agent cannot record through the API either.
        assert!(record(&store, base(T0 + 1, false), tap("message", "m-2", "up")).is_err());
    }
}
