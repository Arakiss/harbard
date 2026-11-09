//! The ops log: the delivery pipeline made visible.
//!
//! Answers the question the operator asks when a message goes unanswered —
//! "where did it die?" — from facts, not guesses: the note was appended here,
//! the reader's hook fired there, the render selected these ids, the envelope
//! went out through that channel.
//!
//! Same rules as the event store it watches: append-only
//! JSONL daily segments under `~/.harbard/ops/`, one `write_all` per line,
//! `<= MAX_LINE_BYTES`, skip-and-count on read, never fatal. Telemetry is
//! even more disposable than traffic — every caller treats an append failure
//! as `let _ =`, because the log must never break the session it observes.
//!
//! Ops content is for humans diagnosing delivery (`harbard trace`); it is
//! never rendered into any agent's context.

use crate::event::MAX_LINE_BYTES;
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// One structured record of something the pipeline did.
///
/// Flat optional fields rather than per-op payloads: lines stay small, old
/// readers keep parsing new lines, and the field set grows without a schema
/// version. Unknown `op` values are kept on read (forward tolerance), only
/// malformed JSON is skipped and counted.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct OpEvent {
    /// Schema version of the ops line itself (1).
    pub v: u8,
    /// When it happened, epoch ms UTC.
    pub ms: u64,
    /// What happened: a stable verb from the OP_* constants.
    #[serde(rename = "op")]
    pub op: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    /// The note this record is about (correlation id across the pipeline).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Note kind (fyi | warning | handoff) for note_appended.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Addressee as `kind:value` (absent = broadcast).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Which door the write came through: cli | serve.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    /// Which verb fired, for OP_HOOK: hello | turn | inline | bye.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verb: Option<String>,
    /// Throttle: ms since the reader's previous store check.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ago_ms: Option<u64>,
    /// Addressed notes held back because their kind waits for a turn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quiet: Option<u64>,
    /// Note/ack ids the digest actually carried — the delivery fact.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivered: Option<Vec<String>>,
    /// How the digest left: hook | plain | none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envelope: Option<String>,
    /// Error text for OP_ERROR.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

/// A note landed in the store (fields: note, kind, to, via, session, harness).
pub const OP_NOTE_APPENDED: &str = "note_appended";
/// A hook-path verb ran (fields: verb, session, harness) — evidence the
/// session's hooks are alive at all, the baseline every other stage needs.
pub const OP_HOOK: &str = "hook";
/// The hot path short-circuited: a check happened inside the throttle window
/// (fields: ago_ms, session).
pub const OP_THROTTLED: &str = "throttled";
/// The inline render held addressed notes back because they ride at turn
/// boundaries, not between tool calls (fields: quiet, session).
pub const OP_HELD_QUIET: &str = "held_quiet";
/// A digest went out (fields: delivered, envelope, session, verb).
pub const OP_INJECTED: &str = "injected";
/// Something failed (fields: verb, msg, session).
pub const OP_ERROR: &str = "error";
/// A hook declared one harness while carrying another's payload and was
/// dropped unanswered (fields: verb = declared harness, harness = real one).
/// Hosts that run a foreign hook file — Cursor runs Claude Code's — otherwise
/// give one conversation two identities.
pub const OP_CROSS_HOOK: &str = "cross_hook";

impl OpEvent {
    fn base(ms: u64, op: &str, session: &str, harness: &str) -> Self {
        OpEvent {
            v: 1,
            ms,
            op: op.to_string(),
            session: (!session.is_empty()).then(|| session.to_string()),
            harness: (!harness.is_empty()).then(|| harness.to_string()),
            ..Default::default()
        }
    }

    /// The note write, stamped at append time — the pipeline's start line.
    pub fn note_appended(
        ms: u64,
        note_id: &str,
        kind: &str,
        to: Option<String>,
        via: &str,
        session: &str,
        harness: &str,
    ) -> Self {
        OpEvent {
            note: Some(note_id.to_string()),
            kind: Some(kind.to_string()),
            to,
            via: Some(via.to_string()),
            ..Self::base(ms, OP_NOTE_APPENDED, session, harness)
        }
    }

    /// A hook-path verb fired.
    pub fn hook(ms: u64, verb: &str, session: &str, harness: &str) -> Self {
        OpEvent {
            verb: Some(verb.to_string()),
            ..Self::base(ms, OP_HOOK, session, harness)
        }
    }

    /// A foreign hook file spoke for a harness that is not its own; we dropped
    /// it rather than write a second identity for a session already announced.
    pub fn cross_hook(ms: u64, declared: &str, real: &str) -> Self {
        OpEvent {
            verb: Some(declared.to_string()),
            ..Self::base(ms, OP_CROSS_HOOK, "", real)
        }
    }

    /// The hot path declined to look at the store.
    pub fn throttled(ms: u64, ago_ms: u64, session: &str, harness: &str) -> Self {
        OpEvent {
            ago_ms: Some(ago_ms),
            ..Self::base(ms, OP_THROTTLED, session, harness)
        }
    }

    /// Addressed notes are waiting for the reader's next turn boundary.
    pub fn held_quiet(ms: u64, quiet: u64, session: &str, harness: &str) -> Self {
        OpEvent {
            quiet: Some(quiet),
            ..Self::base(ms, OP_HELD_QUIET, session, harness)
        }
    }

    /// The delivery fact: these ids went out through this envelope.
    pub fn injected(
        ms: u64,
        verb: &str,
        delivered: Vec<String>,
        envelope: &str,
        session: &str,
        harness: &str,
    ) -> Self {
        OpEvent {
            verb: Some(verb.to_string()),
            delivered: Some(delivered),
            envelope: Some(envelope.to_string()),
            ..Self::base(ms, OP_INJECTED, session, harness)
        }
    }

    /// A failure worth finding later.
    pub fn error(ms: u64, verb: &str, msg: &str, session: &str, harness: &str) -> Self {
        OpEvent {
            verb: Some(verb.to_string()),
            msg: Some(msg.chars().take(200).collect()),
            ..Self::base(ms, OP_ERROR, session, harness)
        }
    }

    /// Serialize to one bounded JSONL line (no trailing newline).
    pub fn to_line(&self) -> std::io::Result<String> {
        let line = serde_json::to_string(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        if line.len() > MAX_LINE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("op line exceeds {MAX_LINE_BYTES} bytes"),
            ));
        }
        Ok(line)
    }
}

/// Append one op line via an `O_APPEND` single write — atomic under the
/// fleet's concurrent writers for `<= MAX_LINE_BYTES` lines (A5).
pub fn append(store: &crate::store::Store, ev: &OpEvent) -> std::io::Result<()> {
    let dir = ops_dir(store.root());
    fs::create_dir_all(&dir)?;
    let mut line = ev.to_line()?;
    line.push('\n');
    let path = dir.join(segment_name(ms_to_ymd(ev.ms)));
    let mut f = OpenOptions::new().create(true).append(true).open(path)?;
    f.write_all(line.as_bytes())?;
    Ok(())
}

/// Best-effort read: parsed ops plus skip-and-count tallies. Never fatal.
#[derive(Debug, Default)]
pub struct OpRead {
    pub ops: Vec<OpEvent>,
    pub corrupt_lines: u64,
    pub segments_read: u64,
}

/// Read ops across the window (Recent = today+yesterday, All = everything).
pub fn read(store: &crate::store::Store, window: crate::store::Window) -> OpRead {
    let mut segs = segments(store.root());
    if window == crate::store::Window::Recent {
        let today = ymd_int(ms_to_ymd(crate::clock::now_ms()));
        let yesterday = ymd_int(ms_to_ymd(crate::clock::now_ms().saturating_sub(86_400_000)));
        segs.retain(|p| segment_ymd(p).is_some_and(|d| d == today || d == yesterday));
    }
    let mut out = OpRead::default();
    for seg in segs {
        let Ok(text) = fs::read_to_string(&seg) else {
            continue;
        };
        out.segments_read += 1;
        for line in text.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<OpEvent>(line) {
                Ok(ev) => out.ops.push(ev),
                Err(_) => out.corrupt_lines += 1,
            }
        }
    }
    out
}

/// Prune ops segments older than `retention_days` relative to `now`.
/// Ops are disposable by definition — no rescue pass like the event store's.
pub fn gc(store: &crate::store::Store, retention_days: u64, now: u64) -> u64 {
    let cutoff = ymd_int(ms_to_ymd(now.saturating_sub(retention_days * 86_400_000)));
    let mut removed = 0;
    for seg in segments(store.root()) {
        if let Some(d) = segment_ymd(&seg)
            && d < cutoff
            && fs::remove_file(&seg).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

/// The ops directory for a harbard home.
pub fn ops_dir(root: &Path) -> PathBuf {
    root.join("ops")
}

/// List ops segment paths in ascending date order.
pub fn segments(root: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = match fs::read_dir(ops_dir(root)) {
        Ok(rd) => rd
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("ops-") && n.ends_with(".jsonl"))
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    out.sort();
    out
}

/// `(year, month, day)` for a millisecond instant, UTC.
fn ms_to_ymd(ms: u64) -> (i32, u8, u8) {
    let nanos = i128::from(ms) * 1_000_000;
    match time::OffsetDateTime::from_unix_timestamp_nanos(nanos) {
        Ok(dt) => (dt.year(), dt.month() as u8, dt.day()),
        Err(_) => (1970, 1, 1),
    }
}

fn ymd_int((y, m, d): (i32, u8, u8)) -> i64 {
    i64::from(y) * 10_000 + i64::from(m) * 100 + i64::from(d)
}

fn segment_name((y, m, d): (i32, u8, u8)) -> String {
    format!("ops-{y:04}{m:02}{d:02}.jsonl")
}

fn segment_ymd(path: &Path) -> Option<i64> {
    let name = path.file_name()?.to_str()?;
    let digits = name.strip_prefix("ops-")?.strip_suffix(".jsonl")?;
    digits.parse::<i64>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempHome;

    #[test]
    fn append_then_read_round_trips() {
        let home = TempHome::new();
        let store = crate::store::Store::at(home.path());
        let ev = OpEvent::note_appended(
            1_752_200_000_000,
            "01JNOTE",
            "handoff",
            Some("session:abc".to_string()),
            "serve",
            "operator",
            "human",
        );
        append(&store, &ev).expect("append");
        let read = read(&store, crate::store::Window::All);
        assert_eq!(read.ops.len(), 1);
        assert_eq!(read.corrupt_lines, 0);
        let got = &read.ops[0];
        assert_eq!(got.op, OP_NOTE_APPENDED);
        assert_eq!(got.note.as_deref(), Some("01JNOTE"));
        assert_eq!(got.to.as_deref(), Some("session:abc"));
    }

    #[test]
    fn corrupt_lines_are_skipped_and_counted() {
        let home = TempHome::new();
        let store = crate::store::Store::at(home.path());
        append(&store, &OpEvent::hook(1, "inline", "s1", "codex")).expect("append");
        let seg = segments(home.path()).pop().expect("segment");
        let mut f = OpenOptions::new().append(true).open(&seg).expect("open");
        f.write_all(b"{not json\n").expect("write corrupt");
        let read = read(&store, crate::store::Window::All);
        assert_eq!(read.ops.len(), 1);
        assert_eq!(read.corrupt_lines, 1);
    }

    #[test]
    fn gc_prunes_old_segments_only() {
        let home = TempHome::new();
        let store = crate::store::Store::at(home.path());
        let day = 86_400_000_u64;
        let now = 1_752_200_000_000_u64;
        append(
            &store,
            &OpEvent::hook(now - 20 * day, "hello", "s", "codex"),
        )
        .expect("old");
        append(&store, &OpEvent::hook(now, "hello", "s", "codex")).expect("new");
        assert_eq!(segments(home.path()).len(), 2);
        let removed = gc(&store, 14, now);
        assert_eq!(removed, 1);
        let read = read(&store, crate::store::Window::All);
        assert_eq!(read.ops.len(), 1);
        assert_eq!(read.ops[0].ms, now);
    }

    #[test]
    fn oversized_line_is_refused_not_truncated() {
        // No constructor truncates arbitrary correlation fields, so the bound
        // is enforced at the line, loudly.
        let big = "x".repeat(MAX_LINE_BYTES);
        let ev = OpEvent {
            note: Some(big),
            ..OpEvent::hook(1, "inline", "s", "h")
        };
        assert!(ev.to_line().is_err());
    }
}
