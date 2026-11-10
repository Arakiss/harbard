//! `harbard archive`: keep a copy of the harness transcripts harbard does not
//! own (`harbard_core::transcript_archive`). Off until `archive enable`.

use crate::cli::{ArchiveCmd, Common};
use harbard_core::Store;
use harbard_core::clock;
use harbard_core::harness_trace::TraceRoots;
use harbard_core::transcript_archive::{self as archive, Outcome, State};
use serde_json::{Value, json};
use std::path::Path;

pub fn cmd_archive(store: &Store, common: &Common, action: &ArchiveCmd) -> i32 {
    let roots = TraceRoots::from_env();
    match action {
        ArchiveCmd::Enable { dir } => enable(store.root(), common, dir, &roots),
        ArchiveCmd::Disable => disable(store.root(), common),
        ArchiveCmd::Status => status(store.root(), common, &roots),
        ArchiveCmd::Run => run(store.root(), common, &roots),
    }
}

fn fail(msg: impl std::fmt::Display) -> i32 {
    eprintln!("harbard: {msg}");
    1
}

fn print_json(value: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(value).unwrap_or_else(|_| "{}".into())
    );
}

fn enable(home: &Path, common: &Common, dir: &Path, roots: &TraceRoots) -> i32 {
    let dir = match std::path::absolute(dir) {
        Ok(dir) => dir,
        Err(e) => return fail(format!("{}: {e}", dir.display())),
    };
    let config = match archive::enable(home, &dir, roots) {
        Ok(config) => config,
        Err(e) => return fail(format!("archive not enabled: {e}")),
    };
    if common.json {
        print_json(&json!({ "enabled": true, "dir": config.dir }));
    } else {
        println!("archive on: {}", config.dir.display());
        println!(
            "next: `harbard archive run` copies now; serve also syncs a few minutes after it starts, then hourly"
        );
    }
    0
}

fn disable(home: &Path, common: &Common) -> i32 {
    let dir = archive::load_config(home).ok().flatten().map(|c| c.dir);
    let was_on = match archive::disable(home) {
        Ok(was_on) => was_on,
        Err(e) => return fail(e),
    };
    if common.json {
        print_json(&json!({ "enabled": false, "was_enabled": was_on, "dir": dir }));
    } else if was_on {
        match dir {
            Some(dir) => println!(
                "archive off; the copies in {} stay where they are",
                dir.display()
            ),
            None => println!("archive off"),
        }
    } else {
        println!("archive was already off");
    }
    0
}

fn status(home: &Path, common: &Common, roots: &TraceRoots) -> i32 {
    let config = archive::load_config(home);
    let state = archive::load_state(home);
    let running = archive::busy(home);
    let sources = archive::sources(roots);
    if common.json {
        let (enabled, dir, error) = match &config {
            Ok(Some(c)) => (true, Some(c.dir.display().to_string()), None),
            Ok(None) => (false, None, None),
            Err(e) => (false, None, Some(e.clone())),
        };
        print_json(&json!({
            "enabled": enabled,
            "dir": dir,
            "config_error": error,
            "running": running,
            "sources": sources.iter().map(|s| json!({
                "label": s.label(),
                "root": s.root.display().to_string(),
                "exists": s.root.is_dir(),
            })).collect::<Vec<_>>(),
            "state": state,
        }));
        return i32::from(config.is_err());
    }
    let mut out = Vec::new();
    match &config {
        Ok(Some(c)) => out.push(format!("{:<12} on → {}", "archive", c.dir.display())),
        Ok(None) => out.push(format!(
            "{:<12} off · harbard archive enable --dir <path>",
            "archive"
        )),
        Err(e) => out.push(format!("{:<12} {e}", "archive")),
    }
    if running {
        out.push(format!("{:<12} a sync is running now", "running"));
    }
    match &state {
        Some(s) => out.push(last_run_line(s, clock::now_ms())),
        None => out.push(format!("{:<12} never", "last run")),
    }
    for s in &sources {
        let found = if s.root.is_dir() { "" } else { " (not found)" };
        out.push(format!(
            "{:<12} {} ← {}{found}",
            "source",
            s.label(),
            s.root.display()
        ));
    }
    if let Some(s) = &state
        && !s.errors.is_empty()
    {
        out.push(format!(
            "errors       last {} of {}:",
            s.errors.len(),
            s.error_count
        ));
        for e in &s.errors {
            out.push(format!("  {}: {}", e.path, e.error));
        }
    }
    println!("{}", out.join("\n"));
    i32::from(config.is_err())
}

fn last_run_line(state: &State, now: u64) -> String {
    let Some(start) = state.last_start_ms else {
        return format!("{:<12} never", "last run");
    };
    if state.running_or_interrupted() {
        return format!(
            "{:<12} started {} and has not finished (running, or interrupted)",
            "last run",
            ago(now, start)
        );
    }
    let took = state
        .last_end_ms
        .map(|end| std::time::Duration::from_millis(end.saturating_sub(start)));
    format!(
        "{:<12} {}: {}",
        "last run",
        ago(now, start),
        archive::summary(state, took)
    )
}

fn ago(now: u64, then: u64) -> String {
    let secs = now.saturating_sub(then) / 1000;
    match secs {
        0..60 => format!("{secs} s ago"),
        60..3600 => format!("{} min ago", secs / 60),
        3600..86_400 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86_400),
    }
}

fn run(home: &Path, common: &Common, roots: &TraceRoots) -> i32 {
    match archive::run(home, roots) {
        Err(e) => fail(format!("archive run failed: {e}")),
        Ok(Outcome::Disabled) => fail(
            "the transcript archive is off; turn it on with `harbard archive enable --dir <path>`",
        ),
        Ok(Outcome::Busy) => fail(
            "a sync is already running (serve or another `harbard archive run`); see `harbard archive status`",
        ),
        Ok(Outcome::Ran { state, took }) => {
            if common.json {
                print_json(&json!({ "took_ms": took.as_millis() as u64, "state": state }));
            } else {
                println!("archive → {}", state.dir);
                println!("{}", archive::summary(&state, Some(took)));
                for e in &state.errors {
                    println!("  {}: {}", e.path, e.error);
                }
                if state.error_count > 0 {
                    println!("next: `harbard archive status` keeps the last errors");
                }
            }
            i32::from(state.error_count > 0)
        }
    }
}

/// One `harbard doctor` line. Advisory: the archive never fails doctor.
pub fn doctor_text(store: &Store) -> String {
    let home = store.root();
    let what = match archive::load_config(home) {
        Ok(None) => "off · harbard archive enable --dir <path>".to_string(),
        Err(e) => e,
        Ok(Some(c)) => {
            let dir = c.dir.display();
            match archive::load_state(home) {
                None => format!("on → {dir}; never ran · harbard archive run"),
                Some(s) if s.running_or_interrupted() => {
                    format!("on → {dir}; a run started and has not finished")
                }
                Some(s) => {
                    let when = s
                        .last_start_ms
                        .map_or_else(|| "never".into(), |t| ago(clock::now_ms(), t));
                    let tail = if s.error_count > 0 {
                        format!("{} error(s) · harbard archive status", s.error_count)
                    } else {
                        "0 errors".into()
                    };
                    format!(
                        "on → {dir}; last run {when}: {} file(s), {} copied or refreshed, {tail}",
                        s.seen,
                        s.copied + s.refreshed + s.versioned
                    )
                }
            }
        }
    };
    format!("{:<12} {what}", "archive")
}

pub fn doctor_json(store: &Store) -> Value {
    let home = store.root();
    match archive::load_config(home) {
        Ok(config) => json!({
            "enabled": config.is_some(),
            "dir": config.map(|c| c.dir),
            "state": archive::load_state(home),
        }),
        Err(e) => json!({ "enabled": false, "config_error": e }),
    }
}

/// serve: one sync a few minutes after start, then hourly, in one thread of
/// its own at background priority. Never on a request path; the lock keeps
/// it from overlapping an `archive run`. One log line per run.
pub fn start_syncer(store: Store, shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};
    const FIRST_AFTER: Duration = Duration::from_secs(5 * 60);
    const EVERY: Duration = Duration::from_secs(60 * 60);
    let spawned = std::thread::Builder::new()
        .name("harbard-archive".into())
        .spawn(move || {
            archive::lower_thread_priority();
            let wait = |d: Duration| {
                let until = Instant::now() + d;
                while Instant::now() < until && !shutdown.load(Ordering::Relaxed) {
                    std::thread::sleep(Duration::from_secs(1));
                }
            };
            wait(FIRST_AFTER);
            while !shutdown.load(Ordering::Relaxed) {
                let roots = TraceRoots::from_env();
                match archive::run(store.root(), &roots) {
                    Ok(Outcome::Disabled) => {}
                    Ok(Outcome::Busy) => {
                        eprintln!("harbard serve: archive: skipped, another sync is running")
                    }
                    Ok(Outcome::Ran { state, took }) => eprintln!(
                        "harbard serve: archive: {}",
                        archive::summary(&state, Some(took))
                    ),
                    Err(e) => eprintln!("harbard serve: archive: {e}"),
                }
                wait(EVERY);
            }
        });
    if let Err(e) = spawned {
        eprintln!("harbard serve: archive: the sync thread did not start: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_read_in_the_largest_whole_unit() {
        assert_eq!(ago(10_000, 5_000), "5 s ago");
        assert_eq!(ago(3 * 60_000, 0), "3 min ago");
        assert_eq!(ago(2 * 3_600_000 + 1, 0), "2 h ago");
        assert_eq!(ago(3 * 86_400_000, 0), "3 d ago");
        assert_eq!(ago(0, 10), "0 s ago");
    }
}
