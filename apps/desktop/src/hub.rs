//! The app's own reader of the bus.
//!
//! One background thread keeps the hub's window of events in memory, the way
//! serve does, and wakes the moment the store changes ([`LogWatch`]). On a
//! change it recomputes the workspace read with serve's own code
//! (`workspace_body`) and hands it to the UI. Nothing here goes through HTTP:
//! an append reaches the screen in the time it takes to fold the read.
//!
//! Writes still go to serve (see [`crate::api`]); serve appends to the same
//! log, so the result of a write arrives here like any other append.

use crate::model::{ActivityView, WorkspaceView};
use harbard_cli::serve::access::Caller;
use harbard_cli::serve::hub_window::{self, HubWindow};
use harbard_cli::serve::workspace_route::workspace_body;
use harbard_core::event::Event;
use harbard_core::log_watch::LogWatch;
use harbard_core::query::DEFAULT_STALE_SECS;
use harbard_core::store::{ReadPosition, Store, Window};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// What the reader sends the UI.
pub enum HubMsg {
    Activity(Arc<ActivityView>),
    Full(Arc<WorkspaceView>, Duration),
    Failed(String),
}

/// Faces age with the clock (thinking → sleepy), so the light read runs at
/// least this often even when nothing is appended.
const ACTIVITY_EVERY: Duration = Duration::from_secs(5);
/// The full read's clock-driven fields (dormant, quiet channels) move slowly.
const FULL_EVERY: Duration = Duration::from_secs(30);
/// How long a quiet wait lasts before the clock checks above run.
const WAIT: Duration = Duration::from_millis(500);

/// Shared with the UI: which workspace to read, and a nudge to read now.
#[derive(Default)]
struct Wanted {
    workspace: Option<String>,
    dirty: bool,
}

#[derive(Clone)]
pub struct Hub {
    store: Store,
    events: Arc<Mutex<Arc<Vec<Event>>>>,
    wanted: Arc<Mutex<Wanted>>,
}

impl Hub {
    /// Read the store once and start the reader thread.
    pub fn start(tx: async_channel::Sender<HubMsg>) -> Hub {
        let store = Store::resolve();
        let hub = Hub {
            store: store.clone(),
            events: Arc::new(Mutex::new(Arc::new(Vec::new()))),
            wanted: Arc::new(Mutex::new(Wanted {
                workspace: crate::desk::saved_workspace(),
                dirty: true,
            })),
        };
        let reader = hub.clone();
        std::thread::Builder::new()
            .name("harbard-hub".into())
            .spawn(move || reader.run(tx))
            .expect("start the hub reader");
        hub
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    /// Switch the workspace the reads are for; the reader recomputes at once.
    pub fn set_workspace(&self, id: Option<String>) {
        let mut w = self.wanted.lock().unwrap_or_else(|e| e.into_inner());
        w.workspace = id;
        w.dirty = true;
    }

    /// Ask for a fresh read (after a write that may not touch the log).
    pub fn refresh(&self) {
        self.wanted.lock().unwrap_or_else(|e| e.into_inner()).dirty = true;
    }

    /// The events the reads fold, for screens that need raw history.
    pub fn events(&self) -> Arc<Vec<Event>> {
        self.events.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn run(self, tx: async_channel::Sender<HubMsg>) {
        let window = HubWindow::new(hub_window::MIN_DAYS, DEFAULT_STALE_SECS);
        let (mut cache, mut position) = load(&self.store, window);
        *self.events.lock().unwrap_or_else(|e| e.into_inner()) = Arc::new(cache.events.clone());
        let mut watch = LogWatch::new(&self.store).ok();
        let mut last_activity = Instant::now() - ACTIVITY_EVERY;
        let mut last_full = Instant::now() - FULL_EVERY;
        let mut changed = true;
        loop {
            let (workspace, nudged) = {
                let mut w = self.wanted.lock().unwrap_or_else(|e| e.into_inner());
                let nudged = std::mem::take(&mut w.dirty);
                (w.workspace.clone(), nudged)
            };
            if changed || nudged {
                let events = Arc::new(cache.events.clone());
                *self.events.lock().unwrap_or_else(|e| e.into_inner()) = events.clone();
                if !self.activity(&events, workspace.clone(), &tx) || !self.full(&events, workspace, &tx) {
                    return;
                }
                last_activity = Instant::now();
                last_full = Instant::now();
            } else if last_full.elapsed() >= FULL_EVERY {
                let events = self.events();
                if !self.full(&events, workspace, &tx) {
                    return;
                }
                last_full = Instant::now();
                last_activity = Instant::now();
            } else if last_activity.elapsed() >= ACTIVITY_EVERY {
                let events = self.events();
                if !self.activity(&events, workspace, &tx) {
                    return;
                }
                last_activity = Instant::now();
            }
            match watch.as_mut() {
                Some(w) => {
                    w.wait(WAIT);
                }
                None => std::thread::sleep(WAIT),
            }
            changed = cache.tail(&self.store, &mut position, window);
        }
    }

    /// Returns false when the UI is gone.
    fn activity(&self, events: &[Event], ws: Option<String>, tx: &async_channel::Sender<HubMsg>) -> bool {
        let msg = match read(&self.store, events, ws, true) {
            Ok(v) => match serde_path_to_error::deserialize::<_, ActivityView>(v) {
                Ok(view) => HubMsg::Activity(Arc::new(view)),
                Err(e) => HubMsg::Failed(e.to_string()),
            },
            Err(e) => HubMsg::Failed(e),
        };
        let ok = tx.send_blocking(msg).is_ok();
        if std::env::var_os("HARBARD_APP_TRACE").is_some() {
            eprintln!("hub: activity sent ok={ok}");
        }
        ok
    }

    fn full(&self, events: &[Event], ws: Option<String>, tx: &async_channel::Sender<HubMsg>) -> bool {
        let started = Instant::now();
        let msg = match read(&self.store, events, ws, false) {
            Ok(v) => match serde_path_to_error::deserialize::<_, WorkspaceView>(v) {
                Ok(view) => HubMsg::Full(Arc::new(view), started.elapsed()),
                Err(e) => HubMsg::Failed(e.to_string()),
            },
            Err(e) => HubMsg::Failed(e),
        };
        tx.send_blocking(msg).is_ok()
    }
}

/// serve's read of one workspace (`None` = the hub's default).
pub fn read(store: &Store, events: &[Event], ws: Option<String>, activity: bool) -> Result<serde_json::Value, String> {
    workspace_body(
        store,
        events,
        DEFAULT_STALE_SECS,
        None,
        activity.then(|| "activity".to_string()),
        option_env!("HARBARD_APP_COMMIT").map(str::to_string),
        &Caller::Local,
        ws,
    )
    .map_err(|e| e.body().to_string())
}

/// The hub's window of events, deduplicated by id, in file order.
struct Cache {
    events: Vec<Event>,
    ids: HashSet<String>,
}

impl Cache {
    /// Fold what was appended since `position`. True when anything was new.
    fn tail(&mut self, store: &Store, position: &mut Option<ReadPosition>, window: HubWindow) -> bool {
        let (read, next) = store.read_since(position.as_ref());
        *position = next;
        let mut changed = false;
        for event in read.events {
            if self.ids.insert(event.id.clone()) {
                self.events.push(event);
                changed = true;
            }
        }
        if changed {
            let now = harbard_core::clock::now_ms();
            let horizon = now.saturating_sub(window.span_ms);
            if self.events.first().is_some_and(|e| event_ms(e) < horizon) {
                self.events = hub_window::trim(store, &self.events, horizon, now, window.stale_secs);
                self.ids = self.events.iter().map(|e| e.id.clone()).collect();
            }
        }
        changed
    }
}

fn event_ms(e: &Event) -> u64 {
    e.id.parse::<ulid::Ulid>().map_or(0, |id| id.timestamp_ms())
}

fn load(store: &Store, window: HubWindow) -> (Cache, Option<ReadPosition>) {
    // Take the tail position first: appends racing the full read are seen
    // again by the tail and deduplicated, never skipped.
    let (_, position) = store.read_since(None);
    let mut all = store.read(Window::All);
    let mut ids = HashSet::new();
    all.events.retain(|e| ids.insert(e.id.clone()));
    let now = harbard_core::clock::now_ms();
    let events = hub_window::trim(store, &all.events, now.saturating_sub(window.span_ms), now, window.stale_secs);
    let ids = events.iter().map(|e| e.id.clone()).collect();
    (Cache { events, ids }, position)
}
