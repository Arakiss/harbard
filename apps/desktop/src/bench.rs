use harbard_cli::serve::access::Caller;
use harbard_cli::serve::hub_window;
use harbard_cli::serve::workspace_route::workspace_body;
use harbard_core::query::DEFAULT_STALE_SECS;
use harbard_core::store::{Store, Window};
use std::time::Instant;

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
    let rounds: usize = std::env::var("BENCH_ROUNDS").ok().and_then(|v| v.parse().ok()).unwrap_or(4);
    for round in 0..rounds {
        let t = Instant::now();
        let full = workspace_body(&store, &events, DEFAULT_STALE_SECS, None, None, None, &Caller::Local, None).unwrap();
        let tf = t.elapsed();
        let t = Instant::now();
        let act = workspace_body(&store, &events, DEFAULT_STALE_SECS, None, Some("activity".into()), None, &Caller::Local, None).unwrap();
        let ta = t.elapsed();
        let t = Instant::now();
        let bytes = serde_json::to_vec(&full).unwrap();
        let ts = t.elapsed();
        println!("round {round}: full {tf:?} ({} KB json, ser {ts:?}), activity {ta:?} ({} agents)", bytes.len() / 1024, act["agents"].as_array().map_or(0, |a| a.len()));
    }
}
