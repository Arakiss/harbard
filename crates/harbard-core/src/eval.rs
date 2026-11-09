//! Operator evaluations of agents — RLHF-shaped feedback that never touches
//! model weights.
//!
//! Append-only `evals.jsonl` under the store root. The operator scores a
//! session against what that session declared (intent) and what the bus can
//! prove (notes, delivery). Consumers are History / export — not another
//! agent's context, and not a trainer.

use crate::clock;
use crate::event::MAX_LINE_BYTES;
use crate::store::Store;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::PathBuf;

/// Score range: 1 (miss) … 5 (nailed it).
pub const SCORE_MIN: u8 = 1;
pub const SCORE_MAX: u8 = 5;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EvalEvent {
    pub v: u8,
    pub ms: u64,
    pub session: String,
    pub harness: String,
    /// 1..=5
    pub score: u8,
    /// Optional free-text: why, with their intent as the yardstick.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Snapshot of their declared intent at rate time (help, not a guess).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub intent: Option<String>,
    /// Operator host that wrote the rating.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionEvalSummary {
    pub session: String,
    pub harness: String,
    pub ratings: u64,
    pub avg_score: f64,
    pub last_score: u8,
    pub last_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalSummary {
    pub since_ms: u64,
    pub now_ms: u64,
    pub ratings: u64,
    pub avg_score: Option<f64>,
    pub by_session: Vec<SessionEvalSummary>,
    pub events: Vec<EvalEvent>,
}

pub fn eval_path(store: &Store) -> PathBuf {
    store.root().join("evals.jsonl")
}

pub fn record(store: &Store, ev: &EvalEvent) -> Result<(), String> {
    if !(SCORE_MIN..=SCORE_MAX).contains(&ev.score) {
        return Err(format!(
            "score must be {SCORE_MIN}..={SCORE_MAX}, got {}",
            ev.score
        ));
    }
    let line = serde_json::to_string(ev).map_err(|e| e.to_string())?;
    if line.len() > MAX_LINE_BYTES {
        return Err(format!("eval line exceeds {MAX_LINE_BYTES} bytes"));
    }
    let path = eval_path(store);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .map_err(|e| e.to_string())?;
    writeln!(f, "{line}").map_err(|e| e.to_string())?;
    Ok(())
}

pub fn read_all(store: &Store) -> Vec<EvalEvent> {
    let path = eval_path(store);
    let Ok(raw) = fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(ev) = serde_json::from_str::<EvalEvent>(line) {
            out.push(ev);
        }
    }
    out
}

pub fn read_since(store: &Store, since_ms: u64) -> Vec<EvalEvent> {
    read_all(store)
        .into_iter()
        .filter(|e| e.ms >= since_ms)
        .collect()
}

pub fn summarize(store: &Store, since_ms: u64) -> EvalSummary {
    let now_ms = clock::now_ms();
    let events = read_since(store, since_ms);
    let by_session = by_session(&events);
    let ratings = events.len() as u64;
    let avg_score = if ratings == 0 {
        None
    } else {
        let sum: u64 = events.iter().map(|e| e.score as u64).sum();
        Some(sum as f64 / ratings as f64)
    };
    EvalSummary {
        since_ms,
        now_ms,
        ratings,
        avg_score,
        by_session,
        events,
    }
}

pub fn by_session(events: &[EvalEvent]) -> Vec<SessionEvalSummary> {
    let mut map: HashMap<String, Vec<&EvalEvent>> = HashMap::new();
    for e in events {
        map.entry(e.session.clone()).or_default().push(e);
    }
    let mut out: Vec<SessionEvalSummary> = map
        .into_iter()
        .map(|(session, mut rows)| {
            rows.sort_by_key(|e| e.ms);
            let last = rows.last().expect("non-empty");
            let sum: u64 = rows.iter().map(|e| e.score as u64).sum();
            let n = rows.len() as u64;
            SessionEvalSummary {
                session,
                harness: last.harness.clone(),
                ratings: n,
                avg_score: sum as f64 / n as f64,
                last_score: last.score,
                last_ms: last.ms,
                last_note: last.note.clone(),
            }
        })
        .collect();
    out.sort_by(|a, b| {
        b.avg_score
            .partial_cmp(&a.avg_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.ratings.cmp(&a.ratings))
    });
    out
}

pub fn new_event(
    session: &str,
    harness: &str,
    score: u8,
    note: Option<String>,
    intent: Option<String>,
    host: Option<String>,
) -> Result<EvalEvent, String> {
    if !(SCORE_MIN..=SCORE_MAX).contains(&score) {
        return Err(format!(
            "score must be {SCORE_MIN}..={SCORE_MAX}, got {score}"
        ));
    }
    let note = note.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    if let Some(ref n) = note
        && n.len() > 2000
    {
        return Err("note exceeds 2000 bytes".into());
    }
    Ok(EvalEvent {
        v: 1,
        ms: clock::now_ms(),
        session: session.to_string(),
        harness: harness.to_string(),
        score,
        note,
        intent: intent
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty()),
        host,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    #[test]
    fn record_and_summarize() {
        let home = crate::test_util::TempHome::new();
        let store = Store::at(home.path());
        let a = new_event(
            "s1",
            "codex",
            5,
            Some("clean".into()),
            Some("fix x".into()),
            None,
        )
        .unwrap();
        let b = new_event("s1", "codex", 3, None, None, None).unwrap();
        let c = new_event("s2", "claude-code", 4, None, None, None).unwrap();
        record(&store, &a).unwrap();
        record(&store, &b).unwrap();
        record(&store, &c).unwrap();
        let sum = summarize(&store, 0);
        assert_eq!(sum.ratings, 3);
        assert_eq!(sum.by_session.len(), 2);
        let s1 = sum.by_session.iter().find(|s| s.session == "s1").unwrap();
        assert_eq!(s1.ratings, 2);
        assert!((s1.avg_score - 4.0).abs() < 0.01);
    }

    #[test]
    fn rejects_bad_score() {
        assert!(new_event("s", "codex", 0, None, None, None).is_err());
        assert!(new_event("s", "codex", 6, None, None, None).is_err());
    }
}
