//! Pruning of per-session derived state: `cursors/` and `delivery-first/`.
//!
//! Both directories gain entries for every session that ever ran and nothing
//! removed them: 7,952 cursor files and 2,467 marker directories on the
//! machine that prompted this, for a fleet with a handful of live sessions.
//! Every `ls`, `brief --touch` and `fleet` reads two cursors per session ever
//! seen, so the dead ones were a cost paid on every prompt.
//!
//! The event log is NOT touched here and never is (see [`Store::gc`]). What is
//! pruned is state whose loss has a defined, harmless answer:
//!
//! - A cursor group (one session's `<id>`, `.brief`, `.live`, `.turn`,
//!   `.watch`, `.wake`, `.owed` files) is removed only when every file in it
//!   is older than [`SESSION_STATE_RETENTION_DAYS`] and no supervisor it
//!   records is still the same running process. A session idle that long is
//!   Ended or Stale either way: its heartbeats are far past any freshness
//!   threshold, and the process-bound path — the one way an old cursor still
//!   proves presence — is exactly what the supervisor check keeps. Losing the
//!   marks of a dead session only changes what that session would see if it
//!   were resumed after a month of silence (its brief and inbox start from
//!   its own history again), and drops its row from `/api/v1/delivery`.
//! - A `delivery-first/<session>` directory is removed when nothing was
//!   written to it for the same period. The markers are a cache:
//!   `learning::claim_first_injection` falls back to the append-only
//!   `learning.jsonl`, which is never collected, when a marker is missing.

use crate::cursor::{BriefState, LiveState};
use crate::lease::PidChecker;
use crate::store::Store;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Idle period after which a session's derived state may be pruned.
pub const SESSION_STATE_RETENTION_DAYS: u64 = 30;

/// How often the lazy pass on `hello` actually walks the directories.
pub const PRUNE_INTERVAL_MS: u64 = 86_400_000;

/// Cursor file suffixes; a file with none of them is the inbox cursor.
const CURSOR_SUFFIXES: [&str; 6] = [".brief", ".live", ".turn", ".watch", ".wake", ".owed"];

/// What a pass removed, or would remove under `dry_run`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct PruneReport {
    /// Sessions whose whole cursor group was (or would be) removed.
    pub cursor_sessions: u64,
    /// Individual cursor files in those groups.
    pub cursor_files: u64,
    /// Cursor groups kept only because their supervisor is still running.
    pub kept_live_supervisor: u64,
    /// `delivery-first/<session>` directories removed.
    pub delivery_sessions: u64,
}

/// Real wall-clock milliseconds. Pruning compares against file mtimes, which
/// a `HARBARD_NOW_MS` override does not move, so it must not use that clock.
pub fn wall_clock_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn stamp_path(store: &Store) -> PathBuf {
    store.root().join("prune.stamp")
}

/// The lazy pass: prune at most once per [`PRUNE_INTERVAL_MS`]. Best-effort;
/// returns `None` when the pass was not due.
pub fn run_due(store: &Store, now_ms: u64, pids: &dyn PidChecker) -> Option<PruneReport> {
    let last = fs::read_to_string(stamp_path(store))
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0);
    if now_ms.saturating_sub(last) < PRUNE_INTERVAL_MS {
        return None;
    }
    // Stamp first: two hooks racing into the pass both stop here next time,
    // and a pass that fails half-way is retried tomorrow, not on every hello.
    if !store.root().is_dir() || fs::write(stamp_path(store), now_ms.to_string()).is_err() {
        return None;
    }
    Some(prune(
        store,
        now_ms,
        SESSION_STATE_RETENTION_DAYS * 86_400_000,
        pids,
        false,
    ))
}

/// Prune session state idle for longer than `max_idle_ms`. With `dry_run`
/// nothing is removed and the report says what would be.
pub fn prune(
    store: &Store,
    now_ms: u64,
    max_idle_ms: u64,
    pids: &dyn PidChecker,
    dry_run: bool,
) -> PruneReport {
    let cutoff = now_ms.saturating_sub(max_idle_ms);
    let mut report = PruneReport::default();
    prune_cursors(store, cutoff, pids, dry_run, &mut report);
    prune_delivery_markers(store, cutoff, dry_run, &mut report);
    report
}

fn mtime_ms(path: &Path) -> Option<u64> {
    let modified = fs::symlink_metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
}

fn cursor_stem(name: &str) -> &str {
    CURSOR_SUFFIXES
        .iter()
        .find_map(|suffix| name.strip_suffix(suffix))
        .unwrap_or(name)
}

fn prune_cursors(
    store: &Store,
    cutoff: u64,
    pids: &dyn PidChecker,
    dry_run: bool,
    report: &mut PruneReport,
) {
    let dir = store.cursors_dir();
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    // stem -> (newest mtime in the group, files)
    let mut groups: BTreeMap<String, (u64, Vec<PathBuf>)> = BTreeMap::new();
    for entry in entries.filter_map(|e| e.ok()) {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        // An unreadable mtime keeps the whole group: never delete on a guess.
        let mtime = mtime_ms(&path).unwrap_or(u64::MAX);
        let group = groups.entry(cursor_stem(&name).to_string()).or_default();
        group.0 = group.0.max(mtime);
        group.1.push(path);
    }
    for (stem, (newest, files)) in groups {
        if newest >= cutoff {
            continue;
        }
        if supervisor_running(&dir, &stem, pids) {
            report.kept_live_supervisor += 1;
            continue;
        }
        report.cursor_sessions += 1;
        for file in files {
            if dry_run || fs::remove_file(&file).is_ok() {
                report.cursor_files += 1;
            }
        }
    }
}

/// True when the group's brief or live mark binds a supervisor that is still
/// the same running process — `query::sessions` would call it Live on that
/// alone, however old its files are. A PID without a start marker cannot be
/// told from a recycled one, so a live PID without one also keeps the group.
fn supervisor_running(dir: &Path, stem: &str, pids: &dyn PidChecker) -> bool {
    let read = |suffix: &str| fs::read_to_string(dir.join(format!("{stem}{suffix}"))).ok();
    let brief = read(".brief")
        .and_then(|s| serde_json::from_str::<BriefState>(&s).ok())
        .map(|b| (b.supervisor_pid, b.supervisor_start));
    let live = read(".live")
        .and_then(|s| serde_json::from_str::<LiveState>(&s).ok())
        .map(|l| (l.supervisor_pid, l.supervisor_start));
    [brief, live]
        .into_iter()
        .flatten()
        .any(|(pid, start)| match pid {
            Some(pid) if pids.is_alive(pid) => match start {
                Some(start) => pids.start_time(pid).is_none_or(|now| now == start),
                None => true,
            },
            _ => false,
        })
}

fn prune_delivery_markers(store: &Store, cutoff: u64, dry_run: bool, report: &mut PruneReport) {
    let Ok(entries) = fs::read_dir(store.root().join("delivery-first")) else {
        return;
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        // Markers are only ever created, so the directory's own mtime is the
        // last write; the newest marker is checked too in case it is not.
        let newest = fs::read_dir(&path)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok())
            .map(|e| mtime_ms(&e.path()).unwrap_or(u64::MAX))
            .chain(std::iter::once(mtime_ms(&path).unwrap_or(u64::MAX)))
            .max()
            .unwrap_or(u64::MAX);
        if newest >= cutoff {
            continue;
        }
        if dry_run || fs::remove_dir_all(&path).is_ok() {
            report.delivery_sessions += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::{
        read_brief_state, write_brief_state, write_inbox_cursor, write_live_state,
    };
    use crate::test_util::TempHome;
    use std::time::{Duration, SystemTime};

    const DAY: u64 = 86_400_000;
    // The real clock: file mtimes are real, so "now" must be too.
    fn now() -> u64 {
        wall_clock_ms()
    }

    struct Pids(Vec<(u32, u64)>);
    impl PidChecker for Pids {
        fn is_alive(&self, pid: u32) -> bool {
            self.0.iter().any(|(p, _)| *p == pid)
        }
        fn start_time(&self, pid: u32) -> Option<u64> {
            self.0.iter().find(|(p, _)| *p == pid).map(|(_, s)| *s)
        }
    }

    fn age(path: &Path, days: u64) {
        let when = SystemTime::now() - Duration::from_millis(days * DAY);
        fs::File::options()
            .write(true)
            .open(path)
            .unwrap()
            .set_modified(when)
            .unwrap();
    }

    fn age_dir(path: &Path, days: u64) {
        let when = SystemTime::now() - Duration::from_millis(days * DAY);
        fs::File::open(path).unwrap().set_modified(when).unwrap();
    }

    fn cursor_files(store: &Store) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(store.cursors_dir())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn brief(pid: Option<u32>, start: Option<u64>) -> BriefState {
        BriefState {
            hb: 1,
            supervisor_pid: pid,
            supervisor_start: start,
            ..Default::default()
        }
    }

    #[test]
    fn idle_groups_go_whole_and_recent_ones_stay() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        for s in ["old", "fresh", "mixed"] {
            write_inbox_cursor(&store, s, "01AAA").unwrap();
            write_brief_state(&store, s, &brief(None, None)).unwrap();
            write_live_state(&store, s, &LiveState::default()).unwrap();
        }
        for name in ["old", "old.brief", "old.live", "mixed", "mixed.brief"] {
            age(&store.cursors_dir().join(name), 40);
        }
        // `mixed.live` was written yesterday: the session is not idle.
        age(&store.cursors_dir().join("mixed.live"), 1);

        let dry = prune(&store, now(), 30 * DAY, &Pids(vec![]), true);
        assert_eq!(dry.cursor_sessions, 1);
        assert_eq!(dry.cursor_files, 3);
        assert_eq!(cursor_files(&store).len(), 9, "dry run removes nothing");

        let done = prune(&store, now(), 30 * DAY, &Pids(vec![]), false);
        assert_eq!(done, dry);
        assert_eq!(
            cursor_files(&store),
            [
                "fresh",
                "fresh.brief",
                "fresh.live",
                "mixed",
                "mixed.brief",
                "mixed.live"
            ]
        );
    }

    #[test]
    fn a_running_supervisor_keeps_an_old_group() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        write_brief_state(&store, "bound", &brief(Some(41), Some(7))).unwrap();
        write_brief_state(&store, "recycled", &brief(Some(42), Some(7))).unwrap();
        write_brief_state(&store, "unmarked", &brief(Some(43), None)).unwrap();
        write_brief_state(&store, "exited", &brief(Some(44), Some(7))).unwrap();
        for s in ["bound", "recycled", "unmarked", "exited"] {
            age(&store.cursors_dir().join(format!("{s}.brief")), 90);
        }
        // 41 is the same process; 42 is alive but a different one (recycled
        // PID); 43 is alive with no marker to tell; 44 has exited.
        let pids = Pids(vec![(41, 7), (42, 8), (43, 1)]);
        let r = prune(&store, now(), 30 * DAY, &pids, false);
        assert_eq!(r.kept_live_supervisor, 2);
        assert_eq!(r.cursor_sessions, 2);
        assert_eq!(cursor_files(&store), ["bound.brief", "unmarked.brief"]);
        assert_eq!(read_brief_state(&store, "bound").supervisor_pid, Some(41));
    }

    #[test]
    fn delivery_markers_go_when_idle_and_the_event_log_is_never_touched() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let root = store.root().join("delivery-first");
        for (session, days) in [("6f6c64", 45), ("6e6577", 2)] {
            let dir = root.join(session);
            fs::create_dir_all(&dir).unwrap();
            let marker = dir.join("01NOTE");
            fs::write(&marker, b"1\n").unwrap();
            age(&marker, days);
            age_dir(&dir, days);
        }
        let events = store.events_dir();
        fs::create_dir_all(&events).unwrap();
        let segment = events.join("events-20200101.jsonl");
        fs::write(&segment, b"").unwrap();
        age(&segment, 2000);

        let r = prune(&store, now(), 30 * DAY, &Pids(vec![]), false);
        assert_eq!(r.delivery_sessions, 1);
        assert!(!root.join("6f6c64").exists());
        assert!(root.join("6e6577").join("01NOTE").exists());
        assert!(segment.exists(), "the event log is permanent");
    }

    #[test]
    fn the_lazy_pass_runs_once_per_interval() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        write_inbox_cursor(&store, "old", "01AAA").unwrap();
        age(&store.cursors_dir().join("old"), 40);
        let t = now();
        let first = run_due(&store, t, &Pids(vec![])).expect("first pass is due");
        assert_eq!(first.cursor_sessions, 1);
        assert_eq!(run_due(&store, t + 1000, &Pids(vec![])), None);
        assert!(run_due(&store, t + PRUNE_INTERVAL_MS, &Pids(vec![])).is_some());
    }

    #[test]
    fn a_missing_home_is_left_alone() {
        let home = TempHome::new();
        let store = Store::at(home.path().join("absent"));
        assert_eq!(run_due(&store, now(), &Pids(vec![])), None);
        assert!(!store.root().exists());
    }
}
