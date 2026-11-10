//! `harbard-app --bench`: times the workspace reads the hub makes on every
//! bus change, against whatever `HARBARD_HOME` points at.
//!
//! `BENCH_ROUNDS=N` sets the number of rounds (4 by default). `BENCH_OUT=<dir>`
//! writes the full and activity bodies of the first and last round there, so
//! two builds can be compared byte for byte under a fixed `HARBARD_NOW_MS`.

use harbard_cli::serve::access::Caller;
use harbard_cli::serve::hub_window;
use harbard_cli::serve::workspace_route::workspace_body;
use harbard_core::query::DEFAULT_STALE_SECS;
use harbard_core::store::{Store, Window};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// CPU time of the calling thread: steadier than the wall clock on a
/// loaded machine (it leaves out time spent waiting for a core).
fn cpu() -> Duration {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: a valid out-pointer for the duration of the call.
    unsafe { libc::clock_gettime(libc::CLOCK_THREAD_CPUTIME_ID, &mut ts) };
    Duration::new(ts.tv_sec as u64, ts.tv_nsec as u32)
}

pub fn run() {
    let store = Store::resolve();
    let t = Instant::now();
    let mut all = store.read(Window::All);
    let mut seen = std::collections::HashSet::new();
    all.events.retain(|e| seen.insert(e.id.clone()));
    let now = harbard_core::clock::now_ms();
    let window = hub_window::HubWindow::new(hub_window::MIN_DAYS, DEFAULT_STALE_SECS);
    let events = hub_window::trim(&store, &all.events, now.saturating_sub(window.span_ms), now, DEFAULT_STALE_SECS);
    println!("load {} events (kept {}) in {:?}", all.events.len(), events.len(), t.elapsed());
    let rounds: usize = std::env::var("BENCH_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(4).max(1);
    let out = std::env::var_os("BENCH_OUT").map(PathBuf::from);
    let mut fulls = Vec::with_capacity(rounds);
    let mut acts = Vec::with_capacity(rounds);
    let mut full_cpu = Vec::with_capacity(rounds);
    let mut act_cpu = Vec::with_capacity(rounds);
    for round in 0..rounds {
        let (t, c) = (Instant::now(), cpu());
        let full = workspace_body(&store, &events, DEFAULT_STALE_SECS, None, None, None, &Caller::Local, None).unwrap();
        let tf = t.elapsed();
        full_cpu.push(cpu() - c);
        let (t, c) = (Instant::now(), cpu());
        let act = workspace_body(&store, &events, DEFAULT_STALE_SECS, None, Some("activity".into()), None, &Caller::Local, None).unwrap();
        let ta = t.elapsed();
        act_cpu.push(cpu() - c);
        let t = Instant::now();
        let bytes = serde_json::to_vec(&full).unwrap();
        let ts = t.elapsed();
        if rounds <= 8 || round < 2 || round + 1 == rounds {
            println!("round {round}: full {tf:?} ({} KB json, ser {ts:?}), activity {ta:?} ({} agents)", bytes.len() / 1024, act["agents"].as_array().map_or(0, |a| a.len()));
        }
        if let Some(dir) = &out
            && (round == 0 || round + 1 == rounds)
        {
            let tag = if round == 0 { "first" } else { "last" };
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(format!("full-{tag}.json")), serde_json::to_vec_pretty(&full).unwrap()).unwrap();
            std::fs::write(dir.join(format!("activity-{tag}.json")), serde_json::to_vec_pretty(&act).unwrap()).unwrap();
        }
        fulls.push(tf);
        acts.push(ta);
    }
    // Steady state: every round after the first.
    let steady = |v: &mut Vec<Duration>| {
        let skip = 1.min(v.len() - 1);
        let rest = &mut v[skip..];
        rest.sort();
        (rest[rest.len() / 2], rest[rest.len() * 9 / 10])
    };
    let (fm, f90) = steady(&mut fulls);
    let (am, a90) = steady(&mut acts);
    println!("steady: full median {fm:?} p90 {f90:?}; activity median {am:?} p90 {a90:?}");
    let (fc, _) = steady(&mut full_cpu);
    let (ac, _) = steady(&mut act_cpu);
    println!("steady cpu: full median {fc:?}; activity median {ac:?}");
    if std::env::var_os("BENCH_PARTS").is_some() {
        parts(&store, &events, rounds);
    }
}

/// The pieces of one read, each timed on its own (median of `rounds`).
fn parts(store: &Store, events: &[harbard_core::event::Event], rounds: usize) {
    use harbard_cli::serve::{team_route, workspace_route};
    let now = harbard_core::clock::now_ms();
    let median = |f: &mut dyn FnMut()| {
        let mut t: Vec<Duration> = (0..rounds)
            .map(|_| {
                let s = Instant::now();
                f();
                s.elapsed()
            })
            .collect();
        t.sort();
        t[t.len() / 2]
    };
    let team = team_route::project(now, harbard_core::team::WINDOW_DEFAULT_MS);
    let gate = harbard_core::workspaces::Gate::OPEN;
    // BENCH_PARTS=view|team: run that piece alone until killed, for a profiler.
    match std::env::var("BENCH_PARTS").as_deref() {
        Ok("view") => loop {
            drop(workspace_route::view_in(store, events, DEFAULT_STALE_SECS, &team, now, &gate));
        },
        Ok("team") => loop {
            drop(team_route::project(now, harbard_core::team::WINDOW_DEFAULT_MS));
        },
        _ => {}
    }
    println!("part team projection {:?}", median(&mut || drop(team_route::project(now, harbard_core::team::WINDOW_DEFAULT_MS))));
    println!("part bus sessions {:?}", median(&mut || drop(harbard_core::query::sessions(store, events, now, DEFAULT_STALE_SECS))));
    println!("part view_in (open gate) {:?}", median(&mut || drop(workspace_route::view_in(store, events, DEFAULT_STALE_SECS, &team, now, &gate))));
}
