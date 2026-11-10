//! Durable learning corpus: did the fleet actually communicate via harbard?
//!
//! Ops (`ops/`) answer "where did this note die?" for a human with `trace`.
//! They are disposable and GC'd. This module keeps the facts worth learning
//! from — note landed, injected into a reader, acknowledged —
//! in an append-only `learning.jsonl` that is never collected.
//!
//! Same rules as ops for the write path: best-effort, never fatal,
//! never rendered into an agent's context. The consumer is the operator
//! (export, pulse, future acceptance), not another session.

use crate::clock;
use crate::event::MAX_LINE_BYTES;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

/// One compact fact about delivery. Flat optional fields, same discipline as
/// [`crate::oplog::OpEvent`]: old readers keep parsing new lines.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct LearningEvent {
    pub v: u8,
    pub ms: u64,
    /// note | injected | retry | ack. Legacy stores may also contain spawn.
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub harness: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub envelope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verb: Option<String>,
    /// Inject: ms − note ULID timestamp. The time-to-inject we actually learn.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    /// Wake inject only: `idle` when the note landed after the reader's turn
    /// ended, `mid_turn` when it landed while the turn ran and was handed over
    /// at its Stop. Separates waiting on the bus from waiting on the reader.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    /// Optional diagnostic note. Retained for backward-compatible reads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub msg: Option<String>,
}

/// Fleet-truth counters over a learning window.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FleetCounters {
    pub notes: u64,
    pub injected: u64,
    /// 0–100. Notes with ≥1 inject / notes. None when notes == 0.
    pub injected_pct: Option<f64>,
    /// Median time-to-inject among injected rows. None when none injected.
    pub time_to_inject_p50_ms: Option<u64>,
    /// Delivery attempts after the first injection for one note/reader pair.
    pub retry_attempts: u64,
    pub acks: u64,
    /// Handoff notes in the window with no later ack for that id.
    pub unacked_handoffs: u64,
}

/// Delivery truth attributed to the session that *authored* the notes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionDelivery {
    pub session: String,
    pub harness: String,
    pub notes: u64,
    pub injected: u64,
    pub injected_pct: Option<f64>,
    pub time_to_inject_p50_ms: Option<u64>,
    pub retry_attempts: u64,
    pub acks: u64,
    pub unacked_handoffs: u64,
}

/// Same counters rolled up by harness.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HarnessDelivery {
    pub harness: String,
    pub sessions: u64,
    pub notes: u64,
    pub injected: u64,
    pub injected_pct: Option<f64>,
    pub time_to_inject_p50_ms: Option<u64>,
    pub retry_attempts: u64,
    pub acks: u64,
    pub unacked_handoffs: u64,
}

/// Summarized window for export / API.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LearningSummary {
    pub since_ms: u64,
    pub now_ms: u64,
    pub counters: FleetCounters,
    pub by_session: Vec<SessionDelivery>,
    pub by_harness: Vec<HarnessDelivery>,
    pub events: Vec<LearningEvent>,
}

pub fn learning_path(store: &Store) -> PathBuf {
    store.root().join("learning.jsonl")
}

fn base(ms: u64, kind: &str) -> LearningEvent {
    LearningEvent {
        v: 1,
        ms,
        kind: kind.to_string(),
        ..Default::default()
    }
}

pub fn note(
    ms: u64,
    note_id: &str,
    note_kind: &str,
    to: Option<String>,
    via: &str,
    session: &str,
    harness: &str,
) -> LearningEvent {
    LearningEvent {
        note: Some(note_id.to_string()),
        note_kind: Some(note_kind.to_string()),
        to,
        via: Some(via.to_string()),
        session: Some(session.to_string()),
        harness: Some(harness.to_string()),
        ..base(ms, "note")
    }
}

pub fn injected(
    ms: u64,
    note_id: &str,
    latency_ms: u64,
    envelope: &str,
    verb: &str,
    session: &str,
    harness: &str,
) -> LearningEvent {
    LearningEvent {
        note: Some(note_id.to_string()),
        latency_ms: Some(latency_ms),
        envelope: Some(envelope.to_string()),
        verb: Some(verb.to_string()),
        session: Some(session.to_string()),
        harness: Some(harness.to_string()),
        ..base(ms, "injected")
    }
}

pub fn retry(
    ms: u64,
    note_id: &str,
    envelope: &str,
    verb: &str,
    session: &str,
    harness: &str,
) -> LearningEvent {
    LearningEvent {
        note: Some(note_id.to_string()),
        envelope: Some(envelope.to_string()),
        verb: Some(verb.to_string()),
        session: Some(session.to_string()),
        harness: Some(harness.to_string()),
        ..base(ms, "retry")
    }
}

pub fn ack(ms: u64, note_id: &str, session: &str, harness: &str) -> LearningEvent {
    LearningEvent {
        note: Some(note_id.to_string()),
        session: Some(session.to_string()),
        harness: Some(harness.to_string()),
        ..base(ms, "ack")
    }
}

/// Append one learning line. Best-effort: never breaks the verb that observed.
pub fn append(store: &Store, ev: &LearningEvent) -> std::io::Result<()> {
    let mut line = serde_json::to_string(ev)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    if line.len() > MAX_LINE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("learning line exceeds {MAX_LINE_BYTES} bytes"),
        ));
    }
    line.push('\n');
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(learning_path(store))?;
    f.write_all(line.as_bytes())?;
    Ok(())
}

/// Best-effort record — callers use this so a full disk never wedges a hook.
pub fn record(store: &Store, ev: &LearningEvent) {
    let _ = append(store, ev);
}

/// Record exactly one first-injection line per note/reader pair. Later
/// attempts remain observable as `retry` without corrupting time-to-inject.
pub fn record_injections(
    store: &Store,
    ms: u64,
    delivered: &[String],
    envelope: &str,
    verb: &str,
    session: &str,
    harness: &str,
) {
    record_injections_in_phase(store, ms, delivered, envelope, verb, session, harness, None);
}

/// [`record_injections`] for a wake, with the [`LearningEvent::phase`] its
/// notes landed in.
#[allow(clippy::too_many_arguments)]
pub fn record_injections_in_phase(
    store: &Store,
    ms: u64,
    delivered: &[String],
    envelope: &str,
    verb: &str,
    session: &str,
    harness: &str,
    phase: Option<&str>,
) {
    for id in delivered {
        if claim_first_injection(store, id, session) {
            let latency = note_age_ms(id, ms);
            record(
                store,
                &LearningEvent {
                    phase: phase.map(str::to_string),
                    ..injected(ms, id, latency, envelope, verb, session, harness)
                },
            );
        } else {
            record(store, &retry(ms, id, envelope, verb, session, harness));
        }
    }
}

fn claim_first_injection(store: &Store, note_id: &str, session: &str) -> bool {
    let session_key = session
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let dir = store.root().join("delivery-first").join(session_key);
    let path = dir.join(note_id);
    if path.exists() {
        return false;
    }

    // Upgrade compatibility: the append-only corpus may predate markers.
    let already_recorded = read_all(store).iter().any(|event| {
        event.kind == "injected"
            && event.note.as_deref() == Some(note_id)
            && event.session.as_deref() == Some(session)
    });
    let _ = fs::create_dir_all(&dir);
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut marker) => {
            let _ = marker.write_all(b"1\n");
            !already_recorded
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        // Derived marker failure must never break delivery. Recording another
        // first attempt is preferable to hiding a delivery completely.
        Err(_) => !already_recorded,
    }
}

fn note_age_ms(note_id: &str, now_ms: u64) -> u64 {
    ulid::Ulid::from_string(note_id)
        .ok()
        .map(|u| now_ms.saturating_sub(u.timestamp_ms()))
        .unwrap_or(0)
}

/// Read every learning line (skip-and-count corrupt). Never fatal.
pub fn read_all(store: &Store) -> Vec<LearningEvent> {
    let path = learning_path(store);
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            continue;
        }
        if let Ok(ev) = serde_json::from_str::<LearningEvent>(line) {
            out.push(ev);
        }
    }
    out
}

pub fn read_since(store: &Store, since_ms: u64) -> Vec<LearningEvent> {
    read_all(store)
        .into_iter()
        .filter(|e| e.ms >= since_ms)
        .collect()
}

/// Default learning window: seven days.
pub const DEFAULT_SINCE_SECS: u64 = 7 * 86_400;

pub fn summarize(store: &Store, since_ms: u64) -> LearningSummary {
    let now_ms = clock::now_ms();
    let events = read_since(store, since_ms);
    let counters = fleet_counters(&events);
    let by_session = delivery_by_session(&events);
    let by_harness = delivery_by_harness(&by_session);
    LearningSummary {
        since_ms,
        now_ms,
        counters,
        by_session,
        by_harness,
        events,
    }
}

fn pct(part: u64, whole: u64) -> Option<f64> {
    if whole == 0 {
        None
    } else {
        Some((part as f64) * 100.0 / (whole as f64))
    }
}

fn median(mut xs: Vec<u64>) -> Option<u64> {
    if xs.is_empty() {
        return None;
    }
    xs.sort_unstable();
    Some(xs[xs.len() / 2])
}

/// Attribute inject/ack outcomes to the session that authored each note.
pub fn delivery_by_session(events: &[LearningEvent]) -> Vec<SessionDelivery> {
    #[derive(Default)]
    struct Acc {
        harness: String,
        notes: HashSet<String>,
        note_kinds: HashMap<String, String>,
        injected: HashSet<String>,
        latencies: Vec<u64>,
        retry_attempts: u64,
        acked: HashSet<String>,
    }

    let mut authors: HashMap<String, (String, String)> = HashMap::new(); // note -> (session, harness)
    let mut by: HashMap<String, Acc> = HashMap::new();

    for e in events {
        if e.kind == "note" {
            let (Some(id), Some(session), Some(harness)) = (&e.note, &e.session, &e.harness) else {
                continue;
            };
            if harness == "human" {
                continue;
            }
            authors.insert(id.clone(), (session.clone(), harness.clone()));
            let acc = by.entry(session.clone()).or_default();
            acc.harness = harness.clone();
            acc.notes.insert(id.clone());
            if let Some(k) = &e.note_kind {
                acc.note_kinds.insert(id.clone(), k.clone());
            }
        }
    }

    let mut seen_pairs: HashSet<(String, String)> = HashSet::new();
    for e in events {
        match e.kind.as_str() {
            "injected" => {
                let Some(id) = &e.note else { continue };
                let Some((session, _)) = authors.get(id) else {
                    continue;
                };
                let acc = by.entry(session.clone()).or_default();
                acc.injected.insert(id.clone());
                let reader = e.session.clone().unwrap_or_default();
                if seen_pairs.insert((id.clone(), reader)) {
                    if let Some(lat) = e.latency_ms {
                        acc.latencies.push(lat);
                    }
                } else {
                    acc.retry_attempts = acc.retry_attempts.saturating_add(1);
                }
            }
            "retry" => {
                let Some(id) = &e.note else { continue };
                let Some((session, _)) = authors.get(id) else {
                    continue;
                };
                let acc = by.entry(session.clone()).or_default();
                acc.retry_attempts = acc.retry_attempts.saturating_add(1);
            }
            "ack" => {
                let Some(id) = &e.note else { continue };
                let Some((session, _)) = authors.get(id) else {
                    continue;
                };
                let acc = by.entry(session.clone()).or_default();
                acc.acked.insert(id.clone());
            }
            _ => {}
        }
    }

    let mut out: Vec<SessionDelivery> = by
        .into_iter()
        .map(|(session, acc)| {
            let notes = acc.notes.len() as u64;
            let injected = acc.injected.len() as u64;
            let unacked_handoffs = acc
                .note_kinds
                .iter()
                .filter(|(id, k)| k.as_str() == "handoff" && !acc.acked.contains(*id))
                .count() as u64;
            SessionDelivery {
                session,
                harness: acc.harness,
                notes,
                injected,
                injected_pct: pct(injected, notes),
                time_to_inject_p50_ms: median(acc.latencies),
                retry_attempts: acc.retry_attempts,
                acks: acc.acked.len() as u64,
                unacked_handoffs,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.injected_pct
            .partial_cmp(&a.injected_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.notes.cmp(&a.notes))
    });
    out
}

pub fn delivery_by_harness(sessions: &[SessionDelivery]) -> Vec<HarnessDelivery> {
    let mut map: HashMap<String, Vec<&SessionDelivery>> = HashMap::new();
    for s in sessions {
        map.entry(s.harness.clone()).or_default().push(s);
    }
    let mut out: Vec<HarnessDelivery> = map
        .into_iter()
        .map(|(harness, rows)| {
            let notes: u64 = rows.iter().map(|r| r.notes).sum();
            let injected: u64 = rows.iter().map(|r| r.injected).sum();
            let acks: u64 = rows.iter().map(|r| r.acks).sum();
            let unacked_handoffs: u64 = rows.iter().map(|r| r.unacked_handoffs).sum();
            let retry_attempts: u64 = rows.iter().map(|r| r.retry_attempts).sum();
            let lats: Vec<u64> = rows
                .iter()
                .filter_map(|r| r.time_to_inject_p50_ms)
                .collect();
            HarnessDelivery {
                harness,
                sessions: rows.len() as u64,
                notes,
                injected,
                injected_pct: pct(injected, notes),
                time_to_inject_p50_ms: median(lats),
                retry_attempts,
                acks,
                unacked_handoffs,
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.injected_pct
            .partial_cmp(&a.injected_pct)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.notes.cmp(&a.notes))
    });
    out
}

pub fn fleet_counters(events: &[LearningEvent]) -> FleetCounters {
    let mut notes: HashSet<String> = HashSet::new();
    let mut note_kinds: HashMap<String, String> = HashMap::new();
    let mut injected_notes: HashSet<String> = HashSet::new();
    let mut latencies: Vec<u64> = Vec::new();
    let mut seen_pairs: HashSet<(String, String)> = HashSet::new();
    let mut retry_attempts = 0u64;
    let mut acked: HashSet<String> = HashSet::new();

    for e in events {
        match e.kind.as_str() {
            "note" => {
                if let Some(id) = &e.note {
                    notes.insert(id.clone());
                    if let Some(k) = &e.note_kind {
                        note_kinds.insert(id.clone(), k.clone());
                    }
                }
            }
            "injected" => {
                if let Some(id) = &e.note {
                    injected_notes.insert(id.clone());
                    let reader = e.session.clone().unwrap_or_default();
                    if seen_pairs.insert((id.clone(), reader)) {
                        if let Some(lat) = e.latency_ms {
                            latencies.push(lat);
                        }
                    } else {
                        retry_attempts = retry_attempts.saturating_add(1);
                    }
                }
            }
            "retry" => retry_attempts = retry_attempts.saturating_add(1),
            "ack" => {
                if let Some(id) = &e.note {
                    acked.insert(id.clone());
                }
            }
            _ => {}
        }
    }

    let note_n = notes.len() as u64;
    let injected_n = injected_notes.len() as u64;
    let injected_pct = if note_n == 0 {
        None
    } else {
        Some((injected_n as f64) * 100.0 / (note_n as f64))
    };

    latencies.sort_unstable();
    let p50 = if latencies.is_empty() {
        None
    } else {
        Some(latencies[latencies.len() / 2])
    };

    let unacked_handoffs = note_kinds
        .iter()
        .filter(|(id, k)| k.as_str() == "handoff" && !acked.contains(*id))
        .count() as u64;

    FleetCounters {
        notes: note_n,
        injected: injected_n,
        injected_pct,
        time_to_inject_p50_ms: p50,
        retry_attempts,
        acks: acked.len() as u64,
        unacked_handoffs,
    }
}

/// Parse `--since 7d` / `24h` / `3600` (seconds) into a duration in seconds.
pub fn parse_since(spec: &str) -> Result<u64, String> {
    let s = spec.trim();
    if s.is_empty() {
        return Ok(DEFAULT_SINCE_SECS);
    }
    if let Some(num) = s.strip_suffix('d').or_else(|| s.strip_suffix('D')) {
        let n: u64 = num
            .trim()
            .parse()
            .map_err(|_| format!("bad --since \"{spec}\""))?;
        return Ok(n.saturating_mul(86_400));
    }
    if let Some(num) = s.strip_suffix('h').or_else(|| s.strip_suffix('H')) {
        let n: u64 = num
            .trim()
            .parse()
            .map_err(|_| format!("bad --since \"{spec}\""))?;
        return Ok(n.saturating_mul(3_600));
    }
    if let Some(num) = s.strip_suffix('m').or_else(|| s.strip_suffix('M')) {
        let n: u64 = num
            .trim()
            .parse()
            .map_err(|_| format!("bad --since \"{spec}\""))?;
        return Ok(n.saturating_mul(60));
    }
    s.parse::<u64>()
        .map_err(|_| format!("bad --since \"{spec}\"; expected 7d | 24h | 30m | seconds"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_since_accepts_units() {
        assert_eq!(parse_since("7d").unwrap(), 7 * 86_400);
        assert_eq!(parse_since("24h").unwrap(), 24 * 3_600);
        assert_eq!(parse_since("30m").unwrap(), 30 * 60);
        assert_eq!(parse_since("90").unwrap(), 90);
    }

    #[test]
    fn counters_join_note_inject_ack() {
        let events = vec![
            note(1000, "n1", "handoff", None, "serve", "op", "human"),
            note(1100, "n2", "fyi", None, "cli", "s1", "codex"),
            injected(1500, "n1", 500, "hook", "inline", "s1", "codex"),
            ack(2000, "n1", "s1", "codex"),
        ];
        let c = fleet_counters(&events);
        assert_eq!(c.notes, 2);
        assert_eq!(c.injected, 1);
        assert!((c.injected_pct.unwrap() - 50.0).abs() < 0.01);
        assert_eq!(c.time_to_inject_p50_ms, Some(500));
        assert_eq!(c.unacked_handoffs, 0);
        assert_eq!(c.acks, 1);
        assert_eq!(c.retry_attempts, 0);
    }

    #[test]
    fn duplicate_injections_count_as_retries_and_do_not_skew_latency() {
        let events = vec![
            note(1_000, "n1", "fyi", None, "cli", "author", "codex"),
            injected(1_100, "n1", 100, "hook", "inline", "reader", "codex"),
            injected(9_000, "n1", 8_000, "hook", "inline", "reader", "codex"),
            retry(10_000, "n1", "hook", "inline", "reader", "codex"),
        ];
        let counters = fleet_counters(&events);
        assert_eq!(counters.injected, 1);
        assert_eq!(counters.retry_attempts, 2);
        assert_eq!(counters.time_to_inject_p50_ms, Some(100));
        let by_session = delivery_by_session(&events);
        assert_eq!(by_session[0].retry_attempts, 2);
        assert_eq!(by_session[0].time_to_inject_p50_ms, Some(100));
    }

    #[test]
    fn record_injections_persists_one_first_attempt_per_reader() {
        let home = crate::test_util::TempHome::new();
        let store = Store::at(home.path());
        let note_id = ulid::Ulid::from_parts(1_000, 1).to_string();
        let delivered = vec![note_id];
        record_injections(
            &store, 1_100, &delivered, "hook", "inline", "reader", "codex",
        );
        record_injections(
            &store, 9_000, &delivered, "hook", "inline", "reader", "codex",
        );
        let events = read_all(&store);
        assert_eq!(
            events
                .iter()
                .filter(|event| event.kind == "injected")
                .count(),
            1
        );
        assert_eq!(
            events.iter().filter(|event| event.kind == "retry").count(),
            1
        );
        assert_eq!(fleet_counters(&events).time_to_inject_p50_ms, Some(100));
    }

    #[test]
    fn unacked_handoff_counts() {
        let events = vec![note(
            1000,
            "n1",
            "handoff",
            Some("session:s1".into()),
            "serve",
            "op",
            "human",
        )];
        assert_eq!(fleet_counters(&events).unacked_handoffs, 1);
    }

    #[test]
    fn append_and_summarize_round_trip() {
        let home = crate::test_util::TempHome::new();
        let store = Store::at(home.path());
        record(
            &store,
            &note(1_000, "n1", "warning", None, "serve", "op", "human"),
        );
        record(
            &store,
            &injected(1_400, "n1", 400, "hook", "inline", "s1", "cursor"),
        );
        let sum = summarize(&store, 0);
        assert_eq!(sum.counters.notes, 1);
        assert_eq!(sum.counters.injected, 1);
        assert_eq!(sum.events.len(), 2);
    }
}
