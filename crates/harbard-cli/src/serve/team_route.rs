//! `GET /api/v1/team`: teammates and subagents of each Claude Code lead
//! session, projected from its transcripts (`harbard_core::team`).
//!
//! Pull-only. It is computed when the console asks, and only then: nothing
//! here runs on the stream tick, writes to `events/`, or reaches an agent. One
//! projector lives for the life of the process so a request reads only the
//! bytes appended since the previous one.

use harbard_core::clock;
use harbard_core::team::{TeamProjector, TeamRoots, TeamView, WINDOW_DEFAULT_MS, WINDOW_MAX_MS};
use serde_json::{Value, json};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};

fn projector() -> &'static Mutex<TeamProjector> {
    static PROJECTOR: OnceLock<Mutex<TeamProjector>> = OnceLock::new();
    PROJECTOR.get_or_init(|| Mutex::new(TeamProjector::new(TeamRoots::from_env())))
}

/// The shell's projector, held: a test proves a reader does not wait for it.
#[cfg(test)]
pub fn hold_projector_for_test() -> std::sync::MutexGuard<'static, TeamProjector> {
    projector().lock().unwrap_or_else(|e| e.into_inner())
}

/// `?hours=` widens the window (24 h by default, at most 7 days).
pub fn team_body(hours: Option<String>) -> Value {
    let window = window_of(hours);
    let view = project(clock::now_ms(), window);
    serde_json::to_value(&view).unwrap_or_else(|e| json!({ "error": e.to_string() }))
}

/// One pass of the process-wide projector, shared with the workspace route so
/// both read each transcript's appended bytes once.
pub fn project(now_ms: u64, window_ms: u64) -> TeamView {
    projector()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .project(now_ms, window_ms)
}

/// A key the projector does not hold runs a pass of its own at most this
/// often, so a poll for an agent that is gone cannot hold the projector a
/// second at a time.
pub const MISS_PASS_EVERY_MS: u64 = 5_000;

static LAST_MISS_PASS: AtomicU64 = AtomicU64::new(0);

/// May a miss run a pass now? Claims the slot when it may.
fn claim_miss_pass(last: &AtomicU64, now_ms: u64) -> bool {
    let prev = last.load(Ordering::Relaxed);
    now_ms.saturating_sub(prev) >= MISS_PASS_EVERY_MS
        && last
            .compare_exchange(prev, now_ms, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
}

/// The transcript of agent `id`, from the process-wide projector: only a
/// file a pass already read. A serve that has not projected it yet runs one
/// pass, the same read `/team` makes, at most every
/// [`MISS_PASS_EVERY_MS`].
pub fn transcript_of(id: &str) -> Option<std::path::PathBuf> {
    let found = projector()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .transcript_of(id);
    if found.is_some() {
        return found;
    }
    let now = clock::now_ms();
    if !claim_miss_pass(&LAST_MISS_PASS, now) {
        return None;
    }
    project(now, WINDOW_DEFAULT_MS);
    projector()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .transcript_of(id)
}

/// `?hours=` as a window in milliseconds.
pub fn window_of(hours: Option<String>) -> u64 {
    hours
        .and_then(|h| h.trim().parse::<u64>().ok())
        .map_or(WINDOW_DEFAULT_MS, |h| {
            h.saturating_mul(3_600_000).min(WINDOW_MAX_MS)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_miss_runs_a_pass_at_most_every_five_seconds() {
        let last = AtomicU64::new(0);
        let t0 = 1_800_000_000_000;
        assert!(claim_miss_pass(&last, t0));
        assert!(!claim_miss_pass(&last, t0 + 1));
        assert!(!claim_miss_pass(&last, t0 + MISS_PASS_EVERY_MS - 1));
        assert!(claim_miss_pass(&last, t0 + MISS_PASS_EVERY_MS));
    }
}
