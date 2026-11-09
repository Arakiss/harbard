//! `harbard friction` — what the gate is costing this fleet, without asking it.
//!
//! Every defect fixed today was found because a blocked session sat down and
//! wrote a note about it. That is a terrible detection channel: it only reports
//! what somebody had the patience to describe, it arrives hours late, and it is
//! silent about the sessions that just gave up. The admission log has been
//! recording these decisions all along; nothing read it back.
//!
//! Three questions, answered from the log alone:
//!   - who is being blocked, and by whom
//!   - how long people wait before they get in
//!   - which calls the classifier cannot read, ranked by how often they happen
//!
//! That last one is the useful one: it turns every ambiguous call into a
//! candidate rule instead of a guess about what to teach next.

use crate::cli::Common;
use harbard_core::Store;
use serde_json::{Value, json};
use std::collections::BTreeMap;

#[derive(Default)]
struct Tally {
    acquired: u64,
    held: u64,
    blocked: u64,
    ambiguous: u64,
    unprotected: u64,
    waits_ms: Vec<u64>,
    /// Ambiguous call shapes → how often. The vocabulary the classifier lacks.
    shapes: BTreeMap<String, u64>,
    /// Who blocked whom, and how many times.
    blockers: BTreeMap<String, u64>,
    /// Blocks we cannot attribute to a checkout.
    unplaced_blocks: u64,
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx]
}

fn human_ms(ms: u64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else if ms < 3_600_000 {
        format!("{:.1}m", ms as f64 / 60_000.0)
    } else if ms < 86_400_000 {
        format!("{:.1}h", ms as f64 / 3_600_000.0)
    } else {
        // The age of this log is measured in weeks. Printing that as 22,849.4m
        // makes the reader do the division, which is how a stale report passes
        // for a current one.
        format!("{:.1}d", ms as f64 / 86_400_000.0)
    }
}

pub fn cmd_friction(store: &Store, common: &Common, limit: usize) -> i32 {
    let path = store
        .root()
        .join("leases")
        .join("v1")
        .join("admission.jsonl");
    let Ok(raw) = std::fs::read_to_string(&path) else {
        if common.json {
            println!("{}", json!({ "status": "empty", "rows": 0 }));
        } else {
            println!("no admission log yet at {}", path.display());
        }
        return 0;
    };

    let mut t = Tally::default();
    let mut rows = 0u64;
    // The window these numbers describe: without it the report reads as current
    // no matter how old it is.
    let mut first_ms = u64::MAX;
    let mut last_ms = 0u64;
    for line in raw.lines() {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        rows += 1;
        if let Some(ms) = v.get("ts_ms").and_then(Value::as_u64) {
            first_ms = first_ms.min(ms);
            last_ms = last_ms.max(ms);
        }
        match v
            .get("decision")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "acquired" => t.acquired += 1,
            "held" => t.held += 1,
            "unprotected" => t.unprotected += 1,
            "ambiguous" => {
                t.ambiguous += 1;
                if let Some(shape) = v.get("shape").and_then(Value::as_str) {
                    *t.shapes.entry(shape.to_string()).or_default() += 1;
                }
            }
            "blocked" => {
                t.blocked += 1;
                match v.get("worktree").and_then(Value::as_str) {
                    Some(w) if !w.is_empty() => {}
                    _ => t.unplaced_blocks += 1,
                }
                if let Some(holder) = v.get("holder").and_then(Value::as_str) {
                    let short = holder.get(..13).unwrap_or(holder).to_string();
                    *t.blockers.entry(short).or_default() += 1;
                }
            }
            _ => {}
        }
        if let Some(waited) = v.get("waited_ms").and_then(Value::as_u64) {
            t.waits_ms.push(waited);
        }
    }
    t.waits_ms.sort_unstable();

    let mut shapes: Vec<(&String, &u64)> = t.shapes.iter().collect();
    shapes.sort_by(|a, b| b.1.cmp(a.1));
    let mut blockers: Vec<(&String, &u64)> = t.blockers.iter().collect();
    blockers.sort_by(|a, b| b.1.cmp(a.1));

    if common.json {
        println!(
            "{}",
            json!({
                "status": "ok",
                "rows": rows,
                "first_ms": (rows > 0 && first_ms != u64::MAX).then_some(first_ms),
                "last_ms": (last_ms > 0).then_some(last_ms),
                "acquired": t.acquired,
                "held": t.held,
                "blocked": t.blocked,
                "ambiguous": t.ambiguous,
                "unprotected": t.unprotected,
                "unplaced_blocks": t.unplaced_blocks,
                "waits": {
                    "count": t.waits_ms.len(),
                    "p50_ms": percentile(&t.waits_ms, 0.50),
                    "p90_ms": percentile(&t.waits_ms, 0.90),
                    "max_ms": t.waits_ms.last().copied().unwrap_or(0),
                },
                "unreadable_calls": shapes.iter().take(limit)
                    .map(|(s, n)| json!({ "shape": s, "count": n })).collect::<Vec<_>>(),
                "blockers": blockers.iter().take(limit)
                    .map(|(s, n)| json!({ "holder": s, "blocked": n })).collect::<Vec<_>>(),
            })
        );
        return 0;
    }

    println!("admission decisions: {rows}");
    if last_ms > 0 && first_ms != u64::MAX {
        let now = harbard_core::clock::now_ms();
        println!(
            "  recorded {} .. {} · last decision {} ago",
            harbard_core::clock::rfc3339_utc(first_ms),
            harbard_core::clock::rfc3339_utc(last_ms),
            human_ms(now.saturating_sub(last_ms)),
        );
    }
    println!(
        "  {} acquired · {} held · {} blocked · {} unreadable · {} unprotected",
        t.acquired, t.held, t.blocked, t.ambiguous, t.unprotected
    );
    if !t.waits_ms.is_empty() {
        println!(
            "\nwaits that ended in a write: {} · median {} · p90 {} · worst {}",
            t.waits_ms.len(),
            human_ms(percentile(&t.waits_ms, 0.50)),
            human_ms(percentile(&t.waits_ms, 0.90)),
            human_ms(t.waits_ms.last().copied().unwrap_or(0)),
        );
    }
    if t.unplaced_blocks > 0 {
        println!(
            "\n{} blocks name no checkout — those cannot be attributed to a tree",
            t.unplaced_blocks
        );
    }
    if !shapes.is_empty() {
        // The point of the whole verb: what to teach the classifier next.
        println!("\ncalls the classifier cannot read (teach these first):");
        for (shape, n) in shapes.iter().take(limit) {
            println!("  {n:5}  {shape}");
        }
    }
    if !blockers.is_empty() {
        println!("\nwho blocked others most:");
        for (holder, n) in blockers.iter().take(limit) {
            println!("  {n:5}  {holder}");
        }
    }
    0
}
