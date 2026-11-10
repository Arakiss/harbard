//! `harbard serve`: the HTTP API over the store.
//!
//! Default bind is loopback (`127.0.0.1`). Every CLI-started server requires a
//! bearer token; one is generated in `~/.harbard/hub.token` when absent. That
//! key is the hub's own (loopback only); paired devices present their own
//! token and reach what their Pairing role allows (`serve/access.rs`). Serve
//! never binds a wildcard: the only other address is the configured tailnet
//! one ([`TAILNET_CONFIG`]).
//! SSE is a 1 s store poll — acceptable per the contract, and it keeps the dep
//! tree boring (no async runtime). Every store string crosses the wire as inert
//! JSON text; escaping/rendering is the console's job (F1 threat model, XSS).
//!
//! ## The one write a viewer has
//!
//! The viewer reads and never writes, with one exception, the human joining
//! the bus:
//!
//! - `POST /api/v1/notes` appends a note whose author is always the operator.
//!   The body carries `text`, an optional `to`, and an optional `kind`; it
//!   CANNOT choose its own `session`, `harness`, or `role`, so the console can
//!   never impersonate an agent. Every other endpoint stays GET/HEAD.
//! - Interrupting kinds (`warning`, `handoff`) are durable notes. A running
//!   harness can receive them at its next safe hook boundary; Harbard never
//!   starts or resumes a harness process on the operator's behalf.
//! - Nothing else mutates: no intent, no ack, no nudge, no delete. A viewer
//!   still cannot move an agent's store state, and `cursors/` remains
//!   untouchable.
//!
//! ## Hub
//!
//! - `POST /api/v1/events` lets a paired device append as the sessions it
//!   published.
//! - `POST/GET /api/v1/devices` manage the fleet card; an invite pairs with
//!   `POST`, and `GET /api/v1/summary` is what a Display device reads.
//! - First-run Setup: `GET /api/v1/setup`, `POST /api/v1/setup/install` and
//!   `POST /api/v1/setup/complete` wire local harness hooks. Mutators require a
//!   loopback peer so a remote console cannot rewrite another machine's configs.
//!
//! ## Workspace
//!
//! `GET /api/v1/workspace` folds the team view, the bus and the sluice into
//! the console's channel shell. `POST /api/v1/workspace/<action>` is
//! loopback-only and never accepts a satellite:
//!
//! - `resolve`, `reopen`, `read`, `save`, `unsave` write the operator's desk
//!   file (`workspace.json`) and nothing else; no agent is told.
//! - `reply` appends operator notes to an agent's lead session, the same
//!   event `POST /notes` writes, with the author stamped here. With no
//!   delivery path it refuses (409) and stores nothing.
//! - `open` hands a local path to the OS opener, only for a file the view
//!   showed as a card. Nothing is read or uploaded.
//!
//! Loopback is a network boundary, not an identity proof. The token excludes
//! callers that cannot read it; processes under the same OS account remain in
//! the local trust domain because they can read the private token file.

mod access;
mod feedback_route;
mod fleet_routes;
mod history_route;
mod hub_window;
mod live_route;
mod pairing_route;
mod profile_route;
mod retro_route;
pub(crate) mod team_route;
mod theme_route;
mod update_route;
pub(crate) mod workspace_route;
mod workspaces_route;

use hub_window::HubWindow;

use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread;
use std::time::{Duration, Instant};

use harbard_core::clock;
use harbard_core::query::{self, Liveness};
use harbard_core::{
    DeviceCard, DeviceRegistry, DeviceRole, Event, EventKind, HARNESS_DEVICE, LeaseStore,
    LocalDevice, NoteKind, OperatorProfile, ReadPosition, Recipient, RecipientKind, Role,
    SCHEMA_VERSION, Store, SystemLeaseClock, SystemPidChecker, Window, card_summary,
    sanitize_display_name,
};
use serde_json::{Value, json};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};
use ulid::Ulid;

/// Default port; `--port` overrides. `harbard` on a phone keypad is ~7419.
pub const DEFAULT_PORT: u16 = 7419;
/// Hard cap on `events?limit=`.
const EVENTS_MAX: usize = 500;
/// One process-wide incremental tail pass. Client count does not multiply it.
const POLL: Duration = Duration::from_millis(250);
/// SSE heartbeat comment interval.
const HEARTBEAT: Duration = Duration::from_secs(15);

/// How long the stream waits for an append before it checks for shutdown,
/// revocation, its age and the heartbeat.
const STREAM_TICK: Duration = Duration::from_secs(5);

/// A burst of appends shares one feed context for its `item` frames.
const STREAM_DEBOUNCE: Duration = Duration::from_millis(150);
/// Force intermediaries to release an SSE upstream periodically. Some HTTP
/// proxies keep the upstream socket alive after the browser-side EventSource
/// has gone away, so relying only on a failed heartbeat can leak connections.
const SSE_MAX_AGE: Duration = Duration::from_secs(5 * 60);
/// Keep SSE below the default macOS launchd soft limit (256 descriptors).
/// tiny_http duplicates accepted sockets internally, so each stream can occupy
/// more than one descriptor until its request thread exits.
const MAX_SSE_CONNECTIONS: usize = 32;
/// A slow browser may lag briefly, but it may never retain unbounded events or
/// block the process-wide tailer.
const SSE_QUEUE_CAPACITY: usize = 256;
/// Retain enough incremental read timings to report a useful local p95.
const READ_TIMING_WINDOW: usize = 128;
/// How often the accept loop wakes to check the shutdown flag.
const ACCEPT_TICK: Duration = Duration::from_millis(250);

/// Immutable serve configuration resolved once at startup.
#[derive(Debug, Clone)]
pub struct Config {
    pub port: u16,
    /// `127.0.0.1` (default) or the configured tailnet address.
    pub bind_host: String,
    /// When set, every request must present this token. CLI startup always sets it.
    pub token: Option<String>,
    pub stale_secs: u64,
    /// Also the days of history the hub keeps in memory for its projections
    /// (at least [`hub_window::MIN_DAYS`]). The store keeps everything.
    pub retention_days: u64,
    pub started_ms: u64,
    /// This machine's stable device id (hub card).
    pub device_id: String,
    /// The Mac's tailnet address, when configured (`{home}/tailnet.json`).
    /// Off by default: serve then listens on loopback only.
    pub tailnet_addr: Option<std::net::IpAddr>,
}

#[derive(Default)]
struct EventHubState {
    // Retained projection history is independent of the recent feed/replay.
    // Both are shared handles: readers clone the `Arc` and project without the
    // hub lock; the tailer copies on write only while a reader still holds one.
    // `events` is bounded by the hub window (`hub_window`); `horizon_ms` is
    // where that window starts, `trimmed_ms` when it last moved.
    events: Arc<Vec<Event>>,
    event_ids: HashSet<String>,
    horizon_ms: u64,
    trimmed_ms: u64,
    recent_events: Arc<Vec<Event>>,
    recent_ids: HashSet<String>,
    subscribers: HashMap<u64, Subscriber>,
    read_ms: VecDeque<u64>,
    last_read_ms: u64,
    last_event_ms: u64,
}

/// The only hot reader of the append-only bus.
///
/// Every SSE connection subscribes to this fan-out. The reader advances by a
/// byte position, so N browser tabs still cause one incremental disk read per
/// tick rather than N full recent-window parses.
struct EventHub {
    state: Mutex<EventHubState>,
    position: Mutex<Option<ReadPosition>>,
    window: HubWindow,
    next_subscriber: AtomicU64,
    active: AtomicUsize,
    reads: AtomicU64,
    dropped_slow: AtomicU64,
}

impl EventHub {
    fn start(store: Store, shutdown: Arc<AtomicBool>, window: HubWindow) -> Arc<Self> {
        // Capture the tail boundary before the snapshot. Appends racing the
        // full read are seen again by the tail and deduplicated, never skipped.
        let (recent, position) = store.read_since(None);
        let mut initial = store.read(Window::All);
        let mut seen = HashSet::new();
        initial.events.retain(|event| seen.insert(event.id.clone()));
        // The full read happens once, here; what the window leaves out is
        // released before the first request is served.
        let now = clock::now_ms();
        let horizon_ms = now.saturating_sub(window.span_ms);
        let events = hub_window::trim(&store, &initial.events, horizon_ms, now, window.stale_secs);
        drop(initial);
        let event_ids = events.iter().map(|event| event.id.clone()).collect();
        let recent_ids = recent.events.iter().map(|event| event.id.clone()).collect();
        let hub = Arc::new(Self {
            state: Mutex::new(EventHubState {
                last_event_ms: events.last().map_or(0, event_timestamp_ms),
                events: Arc::new(events),
                event_ids,
                horizon_ms,
                trimmed_ms: now,
                recent_events: Arc::new(recent.events),
                recent_ids,
                ..EventHubState::default()
            }),
            position: Mutex::new(position),
            window,
            next_subscriber: AtomicU64::new(1),
            active: AtomicUsize::new(0),
            reads: AtomicU64::new(1),
            dropped_slow: AtomicU64::new(0),
        });
        let reader = Arc::clone(&hub);
        thread::spawn(move || {
            while !shutdown.load(Ordering::Relaxed) {
                thread::sleep(POLL);
                reader.poll(&store);
            }
        });
        hub
    }

    fn poll(&self, store: &Store) {
        // This lock makes the background tailer and an immediate post-write
        // refresh one ordered reader. Two callers can never advance from the
        // same byte position and publish a duplicate event.
        let mut position = self.position.lock().unwrap_or_else(|e| e.into_inner());
        let started = Instant::now();
        let (mut read, next) = store.read_since(position.as_ref());
        let elapsed = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64;
        self.reads.fetch_add(1, Ordering::Relaxed);
        let reset_recent = match (position.as_ref(), next.as_ref()) {
            (Some(previous), Some(next)) => {
                previous.segment != next.segment || next.offset < previous.offset
            }
            _ => true,
        };
        *position = next;
        // Keep the reader lock through publication so a post-write refresh
        // cannot publish a later tail before this one has merged its snapshot.

        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.last_read_ms = elapsed;
        state.read_ms.push_back(elapsed);
        if state.read_ms.len() > READ_TIMING_WINDOW {
            state.read_ms.pop_front();
        }
        for event in &read.events {
            if !state.event_ids.contains(&event.id) {
                state.event_ids.insert(event.id.clone());
                Arc::make_mut(&mut state.events).push(event.clone());
            }
        }
        if reset_recent {
            // Rotation replaces only the presentation window. Original joins,
            // questions and answers remain in the authoritative history.
            for event in &read.events {
                if !state.recent_ids.contains(&event.id) {
                    Self::publish_locked(self, &mut state, std::slice::from_ref(event));
                }
            }
            state.recent_ids = read.events.iter().map(|event| event.id.clone()).collect();
            state.recent_events = Arc::new(read.events);
        } else {
            // Feed dedup is separate: appends seen in the startup snapshot
            // still reach subscribers when the captured tail reads them again.
            read.events.retain(|event| {
                if state.recent_ids.contains(&event.id) {
                    false
                } else {
                    state.recent_ids.insert(event.id.clone());
                    true
                }
            });
            Self::publish_locked(self, &mut state, &read.events);
            Arc::make_mut(&mut state.recent_events).extend(read.events);
        }
        drop(state);
        self.trim_if_due(store, clock::now_ms());
    }

    /// Move the horizon forward so a long-running hub does not regrow its
    /// history without bound.
    ///
    /// Only `poll` calls this, under the reader lock: `poll` is the one writer
    /// of `events`, so the history cannot change between the copy taken here
    /// and the swap below, and the trim runs without the hub lock.
    fn trim_if_due(&self, store: &Store, now: u64) {
        let events = {
            let state = self.snapshot();
            if now.saturating_sub(state.trimmed_ms) < hub_window::TRIM_EVERY_MS {
                return;
            }
            Arc::clone(&state.events)
        };
        let horizon_ms = now.saturating_sub(self.window.span_ms);
        let kept = hub_window::trim(store, &events, horizon_ms, now, self.window.stale_secs);
        drop(events);
        let mut state = self.snapshot();
        state.event_ids = kept.iter().map(|event| event.id.clone()).collect();
        state.events = Arc::new(kept);
        state.horizon_ms = horizon_ms;
        state.trimmed_ms = now;
    }

    /// Where the retained window starts. Older events are in the store only,
    /// apart from what `hub_window::trim` carried forward.
    fn horizon_ms(&self) -> u64 {
        self.snapshot().horizon_ms
    }

    fn publish_locked(hub: &Self, state: &mut EventHubState, events: &[Event]) {
        for event in events {
            state.last_event_ms = event_timestamp_ms(event);
            let mut remove = Vec::new();
            for (&id, subscriber) in &state.subscribers {
                match subscriber.tx.try_send(event.clone()) {
                    Ok(()) => {}
                    Err(TrySendError::Full(_)) => {
                        hub.dropped_slow.fetch_add(1, Ordering::Relaxed);
                        // Say why (§3): the stream thread reads this flag when
                        // the queue closes and writes `reset` before it exits.
                        subscriber.evicted.store(true, Ordering::Release);
                        remove.push(id);
                    }
                    Err(TrySendError::Disconnected(_)) => remove.push(id),
                }
            }
            for id in remove.drain(..) {
                state.subscribers.remove(&id);
            }
        }
    }

    fn snapshot(&self) -> MutexGuard<'_, EventHubState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// The retained history and the recent window, taken under a brief lock.
    ///
    /// Projections (`/sessions`, `/snapshot`, `/workspace`) walk tens of thousands
    /// of events. Holding the hub lock through that walk stalled the tailer,
    /// every other reader and `/health` behind it, until the watchdog took a
    /// slow health answer for a dead server and restarted it.
    fn history(&self) -> (Arc<Vec<Event>>, Arc<Vec<Event>>) {
        let state = self.snapshot();
        (Arc::clone(&state.events), Arc::clone(&state.recent_events))
    }

    /// Stream liveness from atomics only: never waits on the hub lock.
    fn liveness(&self) -> Value {
        json!({
            "active_streams": self.active.load(Ordering::Relaxed),
            "max_streams": MAX_SSE_CONNECTIONS,
            "incremental_reads": self.reads.load(Ordering::Relaxed),
        })
    }

    /// Subscribe, replaying recent events after the requested id.
    ///
    /// Reports whether the requested id was actually in the window: a client
    /// resuming from an id the hub has already dropped must be told (`reset`),
    /// never handed live frames as if no gap existed.
    fn subscribe(self: &Arc<Self>, after: Option<&str>) -> Option<Subscription> {
        self.active
            .try_update(Ordering::AcqRel, Ordering::Acquire, |count| {
                (count < MAX_SSE_CONNECTIONS).then_some(count + 1)
            })
            .ok()?;
        let id = self.next_subscriber.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::sync_channel(SSE_QUEUE_CAPACITY);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let found = after.and_then(|wanted| {
            state
                .recent_events
                .iter()
                .position(|event| event.id == wanted)
        });
        let start = found.map_or(state.recent_events.len(), |index| index + 1);
        let replay = state.recent_events[start..].to_vec();
        let resumed = after.is_none() || found.is_some();
        let evicted = Arc::new(AtomicBool::new(false));
        state.subscribers.insert(
            id,
            Subscriber {
                tx,
                evicted: Arc::clone(&evicted),
            },
        );
        drop(state);
        Some(Subscription {
            id,
            replay,
            resumed,
            receiver: rx,
            evicted,
            hub: Arc::clone(self),
        })
    }

    fn metrics(&self) -> Value {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut timings: Vec<_> = state.read_ms.iter().copied().collect();
        timings.sort_unstable();
        let p95 = if timings.is_empty() {
            0
        } else {
            timings[(timings.len() * 95).div_ceil(100).saturating_sub(1)]
        };
        json!({
            "active_streams": self.active.load(Ordering::Relaxed),
            "max_streams": MAX_SSE_CONNECTIONS,
            "subscribers": state.subscribers.len(),
            "incremental_reads": self.reads.load(Ordering::Relaxed),
            "last_read_ms": state.last_read_ms,
            "p95_read_ms": p95,
            "last_event_ms": state.last_event_ms,
            "dropped_slow_clients": self.dropped_slow.load(Ordering::Relaxed),
        })
    }
}

/// How often the hub settles lanes nobody is touching. A ttl is minutes long,
/// so half a minute late is within its own rounding; the PR turn inside has
/// its own, longer, shared interval.
const LANE_SWEEP_EVERY: Duration = Duration::from_secs(30);

/// The one background watcher: expires idle lanes, releases lanes whose PRs
/// settled, and takes the shared PR turn when it is due.
fn start_lane_watcher(store: Store, shutdown: Arc<AtomicBool>) {
    thread::spawn(move || {
        while !shutdown.load(Ordering::Relaxed) {
            crate::lane::watch_turn(&store, None, clock::now_ms());
            let started = Instant::now();
            while started.elapsed() < LANE_SWEEP_EVERY && !shutdown.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_secs(1));
            }
        }
    });
}

fn event_timestamp_ms(event: &Event) -> u64 {
    event.id.parse::<Ulid>().map_or(0, |id| id.timestamp_ms())
}

/// A live stream connection, from the hub's side.
///
/// `evicted` is how a slow client learns why it was dropped: the publisher
/// holds the hub lock and must never write to a socket, so it sets the flag
/// and closes the queue, and the stream thread — which owns the socket —
/// writes the `reset` frame itself before it goes.
struct Subscriber {
    tx: SyncSender<Event>,
    evicted: Arc<AtomicBool>,
}

struct Subscription {
    id: u64,
    replay: Vec<Event>,
    /// False when the client asked to resume from an id the window no longer
    /// holds — the gap is announced, never papered over.
    resumed: bool,
    receiver: Receiver<Event>,
    /// Set by the hub when this connection was dropped for falling behind.
    evicted: Arc<AtomicBool>,
    hub: Arc<EventHub>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.hub
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .subscribers
            .remove(&self.id);
        self.hub.active.fetch_sub(1, Ordering::AcqRel);
    }
}

impl Config {
    pub fn is_lan(&self) -> bool {
        self.bind_host != "127.0.0.1" && self.bind_host != "::1"
    }

    pub fn requires_auth(&self) -> bool {
        self.token.is_some()
    }

    /// Every address serve listens on: loopback, and the tailnet address
    /// when one is configured. Never a wildcard.
    pub fn binds(&self) -> Vec<String> {
        let mut out = vec![self.bind_host.clone()];
        if let Some(addr) = self.tailnet_addr {
            out.push(addr.to_string());
        }
        out
    }
}

/// The config point for the tailnet bind: `{home}/tailnet.json`, for
/// example `{"addr": "100.101.102.103"}`. Absent by default. Only an
/// address inside Tailscale's ranges (100.64.0.0/10, fd7a:115c:a1e0::/48)
/// is taken; anything else, a wildcard included, is ignored with a log
/// line, and serve stays on loopback.
pub const TAILNET_CONFIG: &str = "tailnet.json";

pub fn tailnet_addr(store: &Store) -> Option<std::net::IpAddr> {
    let raw = std::fs::read_to_string(store.root().join(TAILNET_CONFIG)).ok()?;
    let value: Value = serde_json::from_str(&raw).ok()?;
    let text = value.get("addr")?.as_str()?.trim().to_string();
    match parse_tailnet_addr(&text) {
        Some(addr) => Some(addr),
        None => {
            log_serve(
                "tailnet_ignored",
                json!({ "reason": "not a tailnet address", "addr": text }),
            );
            None
        }
    }
}

/// A tailnet address, or `None`: never a wildcard, loopback or LAN address.
pub fn parse_tailnet_addr(raw: &str) -> Option<std::net::IpAddr> {
    use std::net::IpAddr;
    match raw.trim().parse::<IpAddr>().ok()? {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            (o[0] == 100 && (64..=127).contains(&o[1])).then_some(IpAddr::V4(v4))
        }
        IpAddr::V6(v6) => {
            let s = v6.segments();
            (s[0] == 0xfd7a && s[1] == 0x115c && s[2] == 0xa1e0).then_some(IpAddr::V6(v6))
        }
    }
}

/// Bind, announce, and serve until `shutdown` flips.
///
/// Prints one JSON discovery line to stdout (`{port, pid, store, bind}`) so tools can
/// find the running server, then logs `method path` (never query values) to
/// stderr per request.
pub fn run(store: Store, cfg: Config, shutdown: Arc<AtomicBool>) -> io::Result<()> {
    if (cfg.is_lan() || cfg.tailnet_addr.is_some())
        && cfg.token.as_ref().map(|t| t.is_empty()).unwrap_or(true)
    {
        return Err(io::Error::other(
            "a bind beyond loopback requires a hub token (--token / HARBARD_HUB_TOKEN / ~/.harbard/hub.token). Tip: harbard hub enable",
        ));
    }

    // Ensure the hub has a local device identity and appears in the registry.
    let local = LocalDevice::load_or_create(&store, DeviceRole::Hub)?;
    let now = clock::now_ms();
    let card = crate::update::stamp(local.to_card(now, detect_local_harnesses()));
    // Hub identity is registry-only on boot — avoid a device_join session
    // polluting SessionView on every serve restart. Satellite joins still emit
    // a bus card via POST /api/v1/devices.
    harbard_core::pairing::update(&store, |reg, _| {
        let _ = reg.upsert(card, now);
    })?;
    // Pairing: the hub's own card is Operator; cards paired with the old
    // shared secret keep no credential.
    harbard_core::pairing::migrate(&store, &local.id)?;
    // The operator's desk is initialised here, once, so no GET ever writes.
    workspace_route::init_desk(&store, now);
    // Workspaces are seeded here, once, from the repositories the history
    // already names; afterwards only `harbard workspace` and the console's
    // workspace settings change them. An unreadable registry is left as it
    // is (delivery then separates by origin owner) and said once.
    match crate::workspaces_cmd::ensure(&store) {
        Ok((set, true)) => eprintln!(
            "harbard serve: seeded {} workspace(s) into {}",
            set.workspaces.len(),
            harbard_core::workspaces::Workspaces::path(&store).display()
        ),
        Ok(_) => {}
        Err(e) => eprintln!("harbard serve: {e}"),
    }
    // Rules count only when serve minted them; the first start with a key
    // to make seals the rules that were active before.
    workspace_route::seal_rules(&store, now);
    // The feedback index is derived: built here and by writes, never by a
    // GET. A read that finds none answers from the bus.
    if let Err(e) = harbard_core::feedback::ensure(&store) {
        eprintln!("harbard serve: the feedback index could not be built: {e}");
    }
    // The programs the faces may name: whatever is installed on PATH, read
    // once (names only, no TCC-protected folder).
    harbard_core::activity::set_path_programs(workspace_route::path_programs(
        &std::env::var("PATH").unwrap_or_default(),
        &std::env::var("HOME").unwrap_or_default(),
        &workspace_route::list_dir,
    ));

    let mut cfg = cfg;
    cfg.device_id = local.id.clone();

    let bind = format!("{}:{}", cfg.bind_host, cfg.port);
    let server = Server::http(&bind).map_err(|e| io::Error::other(format!("bind {bind}: {e}")))?;
    let addr: SocketAddr = server
        .server_addr()
        .to_ip()
        .ok_or_else(|| io::Error::other("no IP listen address"))?;
    // The tailnet address, when configured, on the same port. It is never
    // fatal: Tailscale may come up after serve, be logged out or have moved,
    // and the Mac itself must keep harbard on loopback. A failed bind is
    // logged and retried in the background.
    let tailnet_at = cfg.tailnet_addr.map(|ip| SocketAddr::new(ip, addr.port()));
    let tailnet_server = tailnet_at.and_then(|at| match Server::http(at) {
        Ok(server) => Some(server),
        Err(e) => {
            log_serve(
                "tailnet_bind_failed",
                json!({ "addr": at.to_string(), "error": e.to_string(), "retry_secs": TAILNET_RETRY.as_secs() }),
            );
            None
        }
    });
    // The startup line lists what is bound now; a tailnet address still
    // waiting for its interface is named apart.
    let tailnet_pending = match (tailnet_at, tailnet_server.as_ref()) {
        (Some(at), None) => Some(at.ip().to_string()),
        _ => None,
    };
    let binds: Vec<String> = cfg
        .binds()
        .into_iter()
        .filter(|b| Some(b) != tailnet_pending.as_ref())
        .collect();

    let store_path = store.root().display().to_string();
    let line = json!({
        "port": addr.port(),
        "pid": std::process::id(),
        "store": store_path,
        "bind": cfg.bind_host,
        "binds": binds,
        "tailnet_pending": tailnet_pending,
        "hub": cfg.requires_auth(),
        "device": cfg.device_id,
    });
    println!("{line}");
    io::stdout().flush().ok();
    log_serve(
        "started",
        json!({
            "address": format!("http://{addr}/api/v1"),
            "store": store_path,
            "bind": cfg.bind_host,
            "binds": binds,
            "tailnet_pending": tailnet_pending,
            "auth_required": cfg.requires_auth(),
            "pid": std::process::id(),
        }),
    );

    let server = Arc::new(server);
    let cfg = Arc::new(cfg);
    let event_hub = EventHub::start(
        store.clone(),
        Arc::clone(&shutdown),
        HubWindow::new(cfg.retention_days, cfg.stale_secs),
    );
    start_lane_watcher(store.clone(), Arc::clone(&shutdown));
    theme_route::start_watcher(store.clone(), Arc::clone(&shutdown));
    profile_route::start_hourly(store.clone(), Arc::clone(&shutdown));
    // Workshop names the reads assigned in memory, saved off the GET path.
    {
        let hub = Arc::clone(&event_hub);
        workspace_route::start_names(
            store.clone(),
            Arc::clone(&shutdown),
            cfg.stale_secs,
            move || hub.history().0,
        );
    }
    retro_route::start_hourly(store.clone(), cfg.stale_secs, Arc::clone(&shutdown));
    // Is there a newer harbard: two minutes after start, then every six hours.
    crate::update::start_checker(store.clone(), Arc::clone(&shutdown));
    // The transcript archive, when on: five minutes after start, then hourly.
    crate::archive::start_syncer(store.clone(), Arc::clone(&shutdown));
    if let Some(at) = tailnet_at {
        let store = store.clone();
        let cfg = Arc::clone(&cfg);
        let shutdown = Arc::clone(&shutdown);
        let event_hub = Arc::clone(&event_hub);
        thread::spawn(move || {
            serve_tailnet(at, tailnet_server, &store, &cfg, &shutdown, &event_hub)
        });
    }
    accept_loop(&server, &store, &cfg, &shutdown, &event_hub)
}

/// How often a tailnet bind that failed, or a listener that died, is tried
/// again.
const TAILNET_RETRY: Duration = Duration::from_secs(15);

/// Serve the tailnet address for as long as serve runs: bind it when it
/// can, and when it cannot (or its listener fails), log it and try again,
/// never touching loopback.
fn serve_tailnet(
    at: SocketAddr,
    mut server: Option<Server>,
    store: &Store,
    cfg: &Arc<Config>,
    shutdown: &Arc<AtomicBool>,
    event_hub: &Arc<EventHub>,
) {
    while !shutdown.load(Ordering::Relaxed) {
        match server.take() {
            Some(listener) => {
                if let Err(e) = accept_loop(&listener, store, cfg, shutdown, event_hub) {
                    log_serve(
                        "tailnet_listener_failed",
                        json!({ "addr": at.to_string(), "error": e.to_string() }),
                    );
                }
            }
            None => {
                let until = Instant::now() + TAILNET_RETRY;
                while Instant::now() < until && !shutdown.load(Ordering::Relaxed) {
                    thread::sleep(ACCEPT_TICK);
                }
                if shutdown.load(Ordering::Relaxed) {
                    return;
                }
                match Server::http(at) {
                    Ok(listener) => {
                        log_serve("tailnet_bound", json!({ "addr": at.to_string() }));
                        server = Some(listener);
                    }
                    Err(e) => log_serve(
                        "tailnet_bind_failed",
                        json!({ "addr": at.to_string(), "error": e.to_string(), "retry_secs": TAILNET_RETRY.as_secs() }),
                    ),
                }
            }
        }
    }
}

fn accept_loop(
    server: &Server,
    store: &Store,
    cfg: &Arc<Config>,
    shutdown: &Arc<AtomicBool>,
    event_hub: &Arc<EventHub>,
) -> io::Result<()> {
    while !shutdown.load(Ordering::Relaxed) {
        match server.recv_timeout(ACCEPT_TICK) {
            Ok(Some(req)) => {
                let store = store.clone();
                let cfg = Arc::clone(cfg);
                let shutdown = Arc::clone(shutdown);
                let event_hub = Arc::clone(event_hub);
                thread::spawn(move || {
                    let started = Instant::now();
                    let method = req.method().as_str().to_string();
                    let path = split_url(req.url()).0.to_string();
                    set_request_line(&store, &method, &path);
                    handle(req, &store, &cfg, &shutdown, event_hub);
                    let status = take_status();
                    // Pairing: a request that ended without an answer still
                    // lands in its device's log (the answered ones were
                    // written before their answer went out).
                    if let Some(device) = take_caller_device() {
                        let action = harbard_core::pairing::Action::new(
                            clock::now_ms(),
                            &method,
                            &path,
                            status.unwrap_or(0),
                        );
                        let _ = harbard_core::pairing::record(&store, &device, &action);
                    }
                    let _ = REQUEST_LINE.with(std::cell::RefCell::take);
                    if let Some(fields) =
                        request_log_fields(&method, &path, status, started.elapsed())
                    {
                        log_serve("request", fields);
                    }
                });
            }
            Ok(None) => continue,
            Err(e) => {
                log_serve("accept_failed", json!({ "error": e.to_string() }));
                // Returning an error terminates the process so launchd's
                // KeepAlive can restart it. Falling through as success can
                // leave request threads alive after the listener is gone: a
                // PID that looks healthy but refuses every new connection.
                return Err(e);
            }
        }
    }
    Ok(())
}

fn log_serve(event: &str, fields: Value) {
    eprintln!(
        "{}",
        json!({
            "ts_ms": clock::now_ms(),
            "component": "harbard-serve",
            "event": event,
            "fields": fields,
        })
    );
}

thread_local! {
    /// The status the current request was answered with. Each request runs on
    /// its own thread, so the responders record it here and the accept loop's
    /// worker reads it back for the request log.
    static RESPONSE_STATUS: std::cell::Cell<Option<u16>> = const { std::cell::Cell::new(None) };
}

/// Record the status the request is answered with. When a paired device
/// asked, its log line is written here, before the answer goes out, so
/// whoever reads the log after the reply finds the line already there.
fn record_status(status: u16) {
    RESPONSE_STATUS.with(|cell| cell.set(Some(status)));
    if let Some(device) = take_caller_device() {
        REQUEST_LINE.with(|cell| {
            if let Some((store, method, path)) = cell.borrow().as_ref() {
                let action =
                    harbard_core::pairing::Action::new(clock::now_ms(), method, path, status);
                let _ = harbard_core::pairing::record(store, &device, &action);
            }
        });
    }
}

thread_local! {
    /// The store, method and path of the request this thread answers, for
    /// the per-device log.
    static REQUEST_LINE: std::cell::RefCell<Option<(Store, String, String)>> =
        const { std::cell::RefCell::new(None) };
}

fn set_request_line(store: &Store, method: &str, path: &str) {
    REQUEST_LINE.with(|cell| {
        *cell.borrow_mut() = Some((store.clone(), method.to_string(), path.to_string()))
    });
}

fn take_status() -> Option<u16> {
    RESPONSE_STATUS.with(std::cell::Cell::take)
}

thread_local! {
    /// The paired device the current request came from, if any: the accept
    /// loop's worker writes it to that device's log once the status is known.
    static CALLER_DEVICE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

fn set_caller_device(id: &str) {
    CALLER_DEVICE.with(|cell| *cell.borrow_mut() = Some(id.to_string()));
}

fn take_caller_device() -> Option<String> {
    CALLER_DEVICE.with(|cell| cell.borrow_mut().take())
}

/// One request-log line: method, path, status, duration. Never the query
/// string, a header or a body — that is where tokens and note text travel.
/// A healthy `/health` is skipped: the watchdog probes it every few seconds
/// and would bury every other line.
fn request_log_fields(
    method: &str,
    path: &str,
    status: Option<u16>,
    elapsed: Duration,
) -> Option<Value> {
    if path == "/api/v1/health" && status == Some(200) {
        return None;
    }
    Some(json!({
        "method": method,
        "path": path,
        "status": status,
        "duration_ms": elapsed.as_millis().min(u128::from(u64::MAX)) as u64,
    }))
}

fn detect_local_harnesses() -> Vec<String> {
    let mut out = Vec::new();
    if crate::install::target_present("claude") {
        out.push("claude-code".into());
    }
    if crate::install::target_present("codex") {
        out.push("codex".into());
    }
    if crate::install::target_present("cursor") {
        out.push("cursor".into());
    }
    out
}

/// Route one request. Enforces GET/HEAD, CORS, `/api/v1` paths, and SSE.
fn handle(
    req: Request,
    store: &Store,
    cfg: &Config,
    shutdown: &Arc<AtomicBool>,
    event_hub: Arc<EventHub>,
) {
    let method = req.method().clone();
    let raw_url = req.url().to_string();
    let (path, query) = split_url(&raw_url);
    let origin = header_value(&req, "origin");

    // Every request names a Host serve answers to: a loopback name, the
    // address it bound, or (a hub) a private address. A DNS-rebinding
    // page carries its own name and gets nothing, reads included.
    let host = header_value(&req, "host");
    let tailnet_host = cfg.tailnet_addr.is_some_and(|ip| {
        host.as_deref()
            .map(host_name)
            .is_some_and(|name| name.eq_ignore_ascii_case(&ip.to_string()))
    });
    if !tailnet_host && !host_allowed(host.as_deref(), &cfg.bind_host, allowed_hosts()) {
        let body = json!({ "error": "this server answers on its own address only" });
        respond_json(req, 421, &body, None, false);
        return;
    }

    if method == Method::Options {
        // Preflight only when the browser asks for a cross-origin method.
        // Bare OPTIONS on GET-only paths stay 405.
        if header_value(&req, "access-control-request-method").is_some()
            || matches!(
                path,
                "/api/v1/notes"
                    | "/api/v1/evals"
                    | "/api/v1/channels"
                    | "/api/v1/devices"
                    | "/api/v1/events"
                    | "/api/v1/setup/install"
                    | "/api/v1/setup/complete"
                    | "/api/v1/operator/reads"
            )
        {
            respond_preflight(req, origin.as_deref());
            return;
        }
    }

    // Pairing: who is asking, and whether their role reaches this route.
    let now = clock::now_ms();
    let presented_token = presented_token(&req);
    let presented_device = header_value(&req, "x-harbard-device");
    let caller = match access::identify(
        store,
        cfg.token.as_deref(),
        &cfg.device_id,
        &access::Presented {
            token: presented_token.as_deref(),
            device: presented_device.as_deref(),
            loopback: is_loopback_peer(&req),
        },
        now,
    ) {
        Ok(caller) => caller,
        Err(error) => {
            // A refused credential is still written to its device's log.
            if let Some(id) = presented_token
                .as_deref()
                .and_then(|t| harbard_core::pairing::credential_device(store, t))
            {
                set_caller_device(&id);
            }
            // The legacy 401 body stays a bare string: clients match on it.
            let body = if error.code == "unauthorized" {
                json!({ "error": error.message })
            } else {
                error.body()
            };
            respond_json(req, error.status, &body, origin.as_deref(), false);
            return;
        }
    };
    if let Some(id) = caller.device_id() {
        set_caller_device(id);
    }
    if let Err(error) = access::permit(&caller, &method, path) {
        respond_json(req, error.status, &error.body(), origin.as_deref(), false);
        return;
    }
    if let Some(id) = caller.device_id() {
        touch_device(store, id, now);
    }

    // Operator / satellite write doors. Checked before the GET/HEAD gate.
    // A request that presents X-Harbard-Device is a satellite voice: it may
    // append its own events, never speak as the
    // operator via /notes|/evals|/channels (CRITICAL 01KYS978XX).
    let device_hdr = header_value(&req, "x-harbard-device").unwrap_or_default();
    if path == "/api/v1/notes" && method == Method::Post {
        if !device_hdr.is_empty() {
            respond_json(
                req,
                403,
                &json!({
                    "error": "satellites cannot POST /notes; append as yourself via POST /events (X-Harbard-Device)"
                }),
                origin.as_deref(),
                false,
            );
            return;
        }
        post_note(req, store, cfg, &caller, origin.as_deref());
        event_hub.poll(store);
        return;
    }
    if path == "/api/v1/evals" && method == Method::Post {
        if !device_hdr.is_empty() {
            respond_json(
                req,
                403,
                &json!({ "error": "satellites cannot POST /evals; operator/console only" }),
                origin.as_deref(),
                false,
            );
            return;
        }
        post_eval(req, store, origin.as_deref());
        return;
    }
    if path == "/api/v1/channels" && method == Method::Post {
        if !device_hdr.is_empty() {
            respond_json(
                req,
                403,
                &json!({ "error": "satellites cannot POST /channels; operator/console only" }),
                origin.as_deref(),
                false,
            );
            return;
        }
        post_channel_action(
            req,
            store,
            cfg.stale_secs,
            origin.as_deref(),
            &event_hub,
            &caller,
        );
        event_hub.poll(store);
        return;
    }
    if path == "/api/v1/devices" && method == Method::Post {
        post_device(
            req,
            store,
            &caller,
            presented_token.as_deref(),
            origin.as_deref(),
        );
        event_hub.poll(store);
        return;
    }
    if path == "/api/v1/events" && method == Method::Post {
        post_satellite_event(req, store, &caller, &event_hub, origin.as_deref());
        event_hub.poll(store);
        return;
    }
    if path == "/api/v1/setup/install" && method == Method::Post {
        post_setup_install(req, store, cfg, origin.as_deref());
        return;
    }
    if path == "/api/v1/setup/complete" && method == Method::Post {
        post_setup_complete(req, store, cfg, origin.as_deref());
        return;
    }
    if path == "/api/v1/operator/reads" && method == Method::Post {
        post_operator_reads(req, store, cfg, origin.as_deref());
        return;
    }
    if path == "/api/v1/attachments" && method == Method::Post {
        post_attachment(req, store, cfg, origin.as_deref(), query);
        return;
    }
    if let Some(action) = path.strip_prefix("/api/v1/workspaces/")
        && method == Method::Post
    {
        let action = action.to_string();
        post_workspaces(req, store, cfg, origin.as_deref(), &action);
        return;
    }
    if let Some(action) = path.strip_prefix("/api/v1/workspace/")
        && method == Method::Post
    {
        let action = action.to_string();
        post_workspace(
            req,
            store,
            cfg,
            origin.as_deref(),
            &event_hub,
            &action,
            &caller,
        );
        event_hub.poll(store);
        return;
    }
    if (path == "/api/v1/profile" || path == "/api/v1/profile/photo") && method == Method::Post {
        post_profile(req, store, cfg, origin.as_deref(), path.ends_with("/photo"));
        return;
    }
    if let Some(rest) = path.strip_prefix("/api/v1/pairing/")
        && method == Method::Post
    {
        let rest = rest.to_string();
        post_pairing(req, store, cfg, origin.as_deref(), &rest);
        return;
    }
    if let Some(action) = path.strip_prefix("/api/v1/update/")
        && method == Method::Post
    {
        let action = action.to_string();
        post_update(req, store, cfg, origin.as_deref(), &action);
        return;
    }
    if path == "/api/v1/digests/read" && method == Method::Post {
        post_digest_read(req, store, cfg, origin.as_deref());
        return;
    }
    if path == "/api/v1/operator" && (method == Method::Post || method == Method::Patch) {
        if !device_hdr.is_empty() {
            respond_json(
                req,
                403,
                &json!({ "error": "satellites cannot mutate operator profile" }),
                origin.as_deref(),
                false,
            );
            return;
        }
        post_operator(req, store, cfg, origin.as_deref());
        return;
    }

    if !matches!(method, Method::Get | Method::Head) {
        let body = json!({
            "error": "method not allowed; allowlisted POSTs: /notes|/evals|/channels|/devices|/events|/setup/install|/setup/complete|/operator|/operator/reads|/workspace/<action>|/profile|/profile/photo|/digests/read|/pairing/<id>/<role|revoke>|/update/<check|install|prefs>"
        });
        respond_json(req, 405, &body, origin.as_deref(), false);
        return;
    }
    let head = method == Method::Head;

    if path == "/api/v1/channels/deleted" {
        let ws = query_param(query, "ws");
        match workspace_route::deleted_channels(store, ws.as_deref(), &caller) {
            Ok(body) => respond_json(req, 200, &body, origin.as_deref(), head),
            Err(e) => respond_json(req, e.status, &e.body(), origin.as_deref(), head),
        }
        return;
    }
    if let Some(slug) = path
        .strip_prefix("/api/v1/channels/")
        .and_then(|rest| rest.strip_suffix("/history"))
    {
        let ws = query_param(query, "ws");
        let before = query_param(query, "before").filter(|b| !b.is_empty());
        let limit = query_param(query, "limit").and_then(|l| l.parse::<usize>().ok());
        match workspace_route::channel_history(
            store,
            slug,
            ws.as_deref(),
            before.as_deref(),
            limit,
            &caller,
        ) {
            Ok(body) => respond_json(req, 200, &body, origin.as_deref(), head),
            Err(e) => respond_json(req, e.status, &e.body(), origin.as_deref(), head),
        }
        return;
    }

    match path {
        "/api/v1/stream" => {
            if head {
                respond_stream_head(req, origin.as_deref());
            } else {
                // `?after=` is the explicit override; `Last-Event-ID` is what a
                // reconnecting EventSource sends on its own, and ignoring it
                // was why a dropped connection silently resumed from now.
                let after = query_param(query, "after")
                    .or_else(|| header_value(&req, "last-event-id"))
                    .map(|id| id.trim().to_string())
                    .filter(|id| !id.is_empty());
                // A paired device's stream lasts only while it is still an
                // Operator: revocation or demotion closes it at once.
                let guard = caller
                    .device()
                    .and_then(|_| presented_token.clone())
                    .map(|token| StreamGuard {
                        store: store.clone(),
                        token,
                    });
                stream_sse(
                    req,
                    Arc::clone(shutdown),
                    origin,
                    event_hub,
                    after,
                    store.clone(),
                    cfg.stale_secs,
                    guard,
                );
            }
        }
        "/api/v1/feed" => {
            let body = fleet_routes::feed_body(
                store,
                &event_hub.history().0,
                event_hub.horizon_ms(),
                cfg.stale_secs,
                query_param(query, "place"),
                query_param(query, "before"),
                query_param(query, "limit"),
            );
            match body {
                Ok(body) => respond_json(req, 200, &body, origin.as_deref(), head),
                Err(error) => {
                    respond_json(req, error.status, &error.body(), origin.as_deref(), head)
                }
            }
        }
        "/api/v1/team" => {
            let body = team_route::team_body(query_param(query, "hours"));
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        path if path.starts_with("/api/v1/live/") => {
            let key = &path["/api/v1/live/".len()..];
            let (status, body) = live_route::live_body(
                key,
                query_param(query, "after"),
                query_param(query, "prose"),
                query_param(query, "only"),
            );
            respond_json(req, status, &body, origin.as_deref(), head);
        }
        "/api/v1/workspace" => {
            match workspace_route::workspace_body(
                store,
                &event_hub.history().0,
                cfg.stale_secs,
                query_param(query, "hours"),
                query_param(query, "only"),
                query_param(query, "console"),
                &caller,
                query_param(query, "ws"),
            ) {
                Ok(body) => respond_json(req, 200, &body, origin.as_deref(), head),
                Err(error) => {
                    respond_json(req, error.status, &error.body(), origin.as_deref(), head)
                }
            }
        }
        "/api/v1/workspaces" => {
            let body = workspaces_route::list_body(store, &event_hub.history().0);
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/retro" => {
            let engine = retro_route::Engine::serve(cfg.stale_secs);
            let scope = query_param(query, "scope");
            let lead = query_param(query, "lead");
            let body = retro_route::retro_route(store, &engine, scope.as_deref(), lead.as_deref());
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/retro/count" => {
            let engine = retro_route::Engine::serve(cfg.stale_secs);
            let body = retro_route::count_body(store, &engine);
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/feedback" => {
            let body = feedback_route::feedback_body(store);
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/theme" => {
            let body = theme_route::theme_body(store, query_param(query, "since"));
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        path if path.starts_with("/api/v1/attachments/") => {
            let rest = &path["/api/v1/attachments/".len()..];
            if let Some(card) = caller
                .device()
                .filter(|c| c.access == harbard_core::Access::Workshop)
                && !addressed_attachment(
                    card,
                    &event_hub.history().0,
                    rest,
                    &harbard_core::hostname(),
                )
            {
                let error = fleet_routes::ApiError::new(
                    403,
                    "role_forbids",
                    "a taller device reads only attachments addressed to its sessions",
                )
                .with("role", json!("workshop"));
                respond_json(req, error.status, &error.body(), origin.as_deref(), head);
                return;
            }
            let root = crate::attach::root(store);
            match workspace_route::attachment_file(&workspace_route::RealFs, &root, rest) {
                Ok((bytes, content_type)) => {
                    respond_attachment(req, &bytes, content_type, origin.as_deref(), head)
                }
                Err(error) => {
                    respond_json(req, error.status, &error.body(), origin.as_deref(), head)
                }
            }
        }
        "/api/v1/profile/brief" => {
            let body = profile_route::brief(store, clock::now_ms());
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/profile" => {
            let body = profile_route::body(store, clock::now_ms());
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/digests/pending" => {
            let body = profile_route::pending(store, clock::now_ms());
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/digests" => {
            let body = profile_route::digests(store, clock::now_ms());
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/profile/photo" => match profile_route::photo(store) {
            Some((bytes, kind)) => respond_bytes(req, &bytes, kind, origin.as_deref(), head),
            None => {
                let body = json!({ "error": "no_photo", "message": "no profile photo" });
                respond_json(req, 404, &body, origin.as_deref(), head);
            }
        },
        "/api/v1/sessions" => {
            let body = sessions_body_from(store, &event_hub.history().0, cfg.stale_secs);
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        path if path.starts_with("/api/v1/notes/") && path.ends_with("/trace") => {
            match note_trace_body(store, cfg.stale_secs, path) {
                Ok(body) => respond_json(req, 200, &body, origin.as_deref(), head),
                Err((status, body)) => respond_json(req, status, &body, origin.as_deref(), head),
            }
        }
        path if path.starts_with("/api/v1/notes/") && path.ends_with("/thread") => {
            match thread_body(store, path) {
                Ok(body) => respond_json(req, 200, &body, origin.as_deref(), head),
                Err((status, body)) => respond_json(req, status, &body, origin.as_deref(), head),
            }
        }
        path if path.starts_with("/api/v1/sessions/") && path.ends_with("/trace") => {
            let body = session_trace_body(
                store,
                &event_hub.history().0,
                event_hub.horizon_ms(),
                cfg.stale_secs,
                path,
                query,
            );
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/events" => {
            let recent = event_hub.history().1;
            let body = match caller
                .device()
                .filter(|c| c.access == harbard_core::Access::Workshop)
            {
                // A Workshop device reads its own sessions' events and the
                // notes addressed to them, nothing else.
                Some(card) => {
                    let placement = harbard_core::workspaces::Placement::load(store);
                    let granted = workspace_route::allowed(&placement, &caller);
                    let own: Vec<Event> = recent
                        .iter()
                        .filter(|e| workshop_sees(card, e, &placement, &granted))
                        .cloned()
                        .collect();
                    events_body_from(&own, query)
                }
                None => events_body_from(&recent, query),
            };
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/snapshot" => {
            let body = {
                let (events, recent_events) = event_hub.history();
                snapshot_body(store, &events, &recent_events, cfg.stale_secs)
            };
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/health" => {
            // The full report names paths in the home: Operator only.
            let full = caller.access() == harbard_core::Access::Operator
                && query_param(query, "full").is_some_and(|v| v == "1" || v == "true");
            let body = health_body(store, cfg, &event_hub, full);
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/leases" => {
            let body = leases_body(store);
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/dependencies" => match dependencies_body(store) {
            Ok(body) => respond_json(req, 200, &body, origin.as_deref(), head),
            Err((status, body)) => respond_json(req, status, &body, origin.as_deref(), head),
        },
        "/api/v1/history" => match history_body(store, query) {
            Ok(body) => respond_json(req, 200, &body, origin.as_deref(), head),
            Err((status, body)) => respond_json(req, status, &body, origin.as_deref(), head),
        },
        "/api/v1/search" => match search_body(store, query) {
            Ok(body) => respond_json(req, 200, &body, origin.as_deref(), head),
            Err((status, body)) => respond_json(req, status, &body, origin.as_deref(), head),
        },
        "/api/v1/history/sessions" => {
            let body = history_sessions_body(store, cfg.stale_secs);
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/history/agents" => {
            let body = history_route::agents_body(store, cfg.stale_secs, query);
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        path if path.starts_with("/api/v1/history/agents/") => {
            match history_route::conversation_body(store, cfg.stale_secs, path, query) {
                Ok(body) => respond_json(req, 200, &body, origin.as_deref(), head),
                Err((status, body)) => respond_json(req, status, &body, origin.as_deref(), head),
            }
        }
        "/api/v1/history/checkouts" => {
            let body = history_checkouts_body(store, cfg.stale_secs);
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/pairing" => {
            let body = pairing_route::list_body(store, clock::now_ms());
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        path if path.starts_with("/api/v1/pairing/") => {
            let rest = &path["/api/v1/pairing/".len()..];
            match pairing_route::log_body(store, rest, query_param(query, "limit")) {
                Ok(body) => respond_json(req, 200, &body, origin.as_deref(), head),
                Err(error) => {
                    respond_json(req, error.status, &error.body(), origin.as_deref(), head)
                }
            }
        }
        "/api/v1/summary" => {
            let body = workspace_route::summary_body(
                store,
                &event_hub.history().0,
                cfg.stale_secs,
                &caller,
            );
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/devices" => {
            let reg = DeviceRegistry::load(store);
            let creds = harbard_core::pairing::Credentials::load(store);
            let devices: Vec<Value> = reg
                .devices
                .iter()
                .map(|d| {
                    let mut v = serde_json::to_value(d).unwrap_or_else(|_| json!({}));
                    if let Some(o) = v.as_object_mut() {
                        // Session ids are the hub's business, not the list's.
                        o.remove("sessions");
                        o.insert(
                            "credential".into(),
                            json!(
                                d.role == DeviceRole::Hub
                                    || harbard_core::pairing::has_credential(&creds, &d.id)
                            ),
                        );
                    }
                    v
                })
                .collect();
            let body = json!({ "devices": devices, "now_ms": clock::now_ms() });
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/delivery" => {
            let rows = harbard_core::cursor::delivery(store);
            let body = json!({ "delivery": rows, "now_ms": clock::now_ms() });
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/aliases" => {
            // Read-only: the API never assigns a name. Assignment happens on
            // `hello`, so a browser refresh cannot mint identities. The
            // epithet rides along: it is derived, not state, so it is
            // computed here rather than persisted.
            let a = harbard_core::alias::read(store);
            let mut body = serde_json::to_value(&a).unwrap_or_else(|_| json!({}));
            let epithets: serde_json::Map<String, serde_json::Value> = a
                .by_session
                .keys()
                .map(|s| {
                    (
                        s.clone(),
                        serde_json::Value::String(harbard_core::alias::epithet(s)),
                    )
                })
                .collect();
            if let Some(obj) = body.as_object_mut() {
                obj.insert("epithets".to_string(), serde_json::Value::Object(epithets));
            }
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/channels" => {
            // Membership comes from the snapshot, not from folding join/leave
            // events: the log is garbage-collected and a subscription must not
            // expire with the event that declared it.
            let m = harbard_core::membership::read(store);
            let inactive_days = query_param(query, "inactive_days")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(7)
                .clamp(1, 90);
            let now = clock::now_ms();
            let candidates = harbard_core::membership::archive_candidates(&m, inactive_days, now);
            let mut body = serde_json::to_value(&m).unwrap_or_else(|_| json!({}));
            if let Some(obj) = body.as_object_mut() {
                obj.insert(
                    "candidates".into(),
                    serde_json::to_value(&candidates).unwrap_or_else(|_| json!([])),
                );
                // Rooms that deliver exactly what `--to repo:<path>` delivers.
                // The console shows the rail; without this it cannot tell a
                // room that earns its name from an alias of a checkout.
                obj.insert(
                    "repo_alias".into(),
                    serde_json::to_value(harbard_core::membership::repo_alias_channels(&m))
                        .unwrap_or_else(|_| json!([])),
                );
                obj.insert("inactive_days".into(), json!(inactive_days));
                obj.insert("now_ms".into(), json!(now));
            }
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/doctor" => {
            let report = query::doctor(store, cfg.retention_days);
            let mut body = serde_json::to_value(&report).unwrap_or_else(|_| json!({}));
            if let Some(obj) = body.as_object_mut() {
                obj.insert("hooks".into(), crate::install::health());
            }
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/setup" => {
            let body = crate::setup::status(store);
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/learning" => {
            // Fleet-truth over the durable learning corpus (default 7d).
            // Ops GC cannot erase these facts — they are written as delivery
            // happens, the same way signal.jsonl rescues operator words.
            let since_secs = query_param(query, "since")
                .and_then(|s| harbard_core::learning::parse_since(&s).ok())
                .unwrap_or(harbard_core::learning::DEFAULT_SINCE_SECS);
            let now = clock::now_ms();
            let since_ms = now.saturating_sub(since_secs.saturating_mul(1000));
            let summary = harbard_core::learning::summarize(store, since_ms);
            // Pulse needs counters; History needs by_session/by_harness.
            // Full event dump only when asked.
            let include_events =
                query_param(query, "events").is_some_and(|v| v == "1" || v == "true");
            let body = if include_events {
                serde_json::to_value(&summary).unwrap_or_else(|_| json!({}))
            } else {
                json!({
                    "since_ms": summary.since_ms,
                    "now_ms": summary.now_ms,
                    "counters": summary.counters,
                    "by_session": summary.by_session,
                    "by_harness": summary.by_harness,
                })
            };
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/evals" => {
            // Operator ratings of agents — durable, never applied to weights.
            let since_secs = query_param(query, "since")
                .and_then(|s| harbard_core::learning::parse_since(&s).ok())
                .unwrap_or(harbard_core::learning::DEFAULT_SINCE_SECS);
            let now = clock::now_ms();
            let since_ms = now.saturating_sub(since_secs.saturating_mul(1000));
            let summary = harbard_core::eval::summarize(store, since_ms);
            let include_events =
                query_param(query, "events").is_some_and(|v| v == "1" || v == "true");
            let body = if include_events {
                serde_json::to_value(&summary).unwrap_or_else(|_| json!({}))
            } else {
                json!({
                    "since_ms": summary.since_ms,
                    "now_ms": summary.now_ms,
                    "ratings": summary.ratings,
                    "avg_score": summary.avg_score,
                    "by_session": summary.by_session,
                })
            };
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/update" => {
            respond_json(
                req,
                200,
                &update_route::body(store),
                origin.as_deref(),
                head,
            );
        }
        "/api/v1/meta" => {
            let body = json!({
                "name": "harbard",
                "version": env!("CARGO_PKG_VERSION"),
                "host": harbard_core::hostname(),
                "started_ms": cfg.started_ms,
                "hub": cfg.requires_auth(),
                "bind": cfg.bind_host,
                "device": cfg.device_id,
                "protocol": harbard_core::PROTOCOL_VERSION,
                // The commit this serve was built from; null when unknown.
                "commit": workspace_route::running_commit(),
            });
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/operator" => {
            let profile = OperatorProfile::load(store);
            let body = json!({
                "session": format!("operator@{}", harbard_core::hostname()),
                "display_name": profile.display_name,
                "updated_ms": profile.updated_ms,
            });
            respond_json(req, 200, &body, origin.as_deref(), head);
        }
        "/api/v1/protocol" => {
            // M5b — same document as `harbard protocol --json` / MCP resource.
            let depth = match query_param(query, "depth").as_deref() {
                Some("full") => harbard_core::ProtocolDepth::Full,
                _ => harbard_core::ProtocolDepth::Compact,
            };
            let requested = query_param(query, "version");
            if let Some(v) = requested.as_deref()
                && v != harbard_core::PROTOCOL_VERSION
                && v != "harbard/protocol/v1"
                && v != "v1"
            {
                let body = json!({
                    "error": "unsupported protocol version",
                    "requested": v,
                    "supported": [harbard_core::PROTOCOL_VERSION],
                });
                respond_json(req, 406, &body, origin.as_deref(), head);
                return;
            }
            respond_json(
                req,
                200,
                &harbard_core::catalog(depth),
                origin.as_deref(),
                head,
            );
        }
        _ => {
            let body = json!({ "error": "not found" });
            respond_json(req, 404, &body, origin.as_deref(), head);
        }
    }
}

/// Hard cap on a posted body, before parsing. Generous against the 2000-byte
/// text limit, tight enough that a runaway client cannot exhaust memory.
const MAX_POST_BYTES: usize = 8 * 1024;

/// Why a request may not write the operator's desk, if it may not: a
/// satellite, a foreign origin or a peer that is not loopback.
fn desk_door(req: &Request, cfg: &Config, origin: Option<&str>) -> Option<fleet_routes::ApiError> {
    if !header_value(req, "x-harbard-device")
        .unwrap_or_default()
        .is_empty()
    {
        return Some(fleet_routes::ApiError::new(
            403,
            "forbidden_origin",
            "satellites cannot use the operator's desk",
        ));
    }
    if let Some(o) = origin
        && !is_allowed_origin(o, cfg.requires_auth())
    {
        return Some(fleet_routes::ApiError::new(
            403,
            "forbidden_origin",
            format!("origin {o} is not allowed"),
        ));
    }
    if !is_loopback_peer(req) {
        return Some(fleet_routes::ApiError::new(
            403,
            "forbidden_origin",
            "the operator's desk is loopback-only",
        ));
    }
    None
}

/// `POST /api/v1/profile[/photo]`: the operator's "Profile", status and No
/// molestar, and the photo's bytes (`serve/profile_route.rs`). The same door
/// as the desk: loopback only, never a satellite.
fn post_profile(mut req: Request, store: &Store, cfg: &Config, origin: Option<&str>, photo: bool) {
    let refuse = |req: Request, error: fleet_routes::ApiError| {
        respond_json(req, error.status, &error.body(), origin, false);
    };
    if let Some(error) = desk_door(&req, cfg, origin) {
        refuse(req, error);
        return;
    }
    let cap = if photo {
        harbard_core::profile::PHOTO_MAX_BYTES
    } else {
        profile_route::MAX_PROFILE_BYTES
    };
    let too_large =
        || fleet_routes::ApiError::new(413, "too_large", format!("body exceeds {cap} bytes"));
    if req.body_length().unwrap_or(0) > cap {
        refuse(req, too_large());
        return;
    }
    let mut raw = Vec::new();
    if req
        .as_reader()
        .take(cap as u64 + 1)
        .read_to_end(&mut raw)
        .is_err()
    {
        refuse(
            req,
            fleet_routes::ApiError::new(400, "bad_request", "unreadable body"),
        );
        return;
    }
    if raw.len() > cap {
        refuse(req, too_large());
        return;
    }
    let now = clock::now_ms();
    let out = if photo {
        profile_route::write_photo(store, now, &raw)
    } else {
        match serde_json::from_slice::<Value>(&raw) {
            Ok(body) => profile_route::write(store, now, &body),
            Err(e) => Err(fleet_routes::ApiError::bad_request(
                "body",
                format!("invalid JSON body: {e}"),
            )),
        }
    };
    match out {
        Ok(body) => respond_json(req, 200, &body, origin, false),
        Err(error) => refuse(req, error),
    }
}

/// `POST /api/v1/pairing/<id>/<role|revoke>`: Preferences › Pairing. The
/// desk's door: loopback, the operator's origin, never a device.
fn post_pairing(mut req: Request, store: &Store, cfg: &Config, origin: Option<&str>, rest: &str) {
    let refuse = |req: Request, error: fleet_routes::ApiError| {
        respond_json(req, error.status, &error.body(), origin, false);
    };
    if let Some(error) = desk_door(&req, cfg, origin) {
        refuse(req, error);
        return;
    }
    let Ok(raw) = read_body(&mut req, origin) else {
        refuse(
            req,
            fleet_routes::ApiError::new(
                413,
                "too_large",
                format!("body exceeds {MAX_POST_BYTES} bytes"),
            ),
        );
        return;
    };
    let body = if raw.trim().is_empty() {
        Ok(json!({}))
    } else {
        serde_json::from_str::<Value>(&raw).map_err(|e| {
            fleet_routes::ApiError::bad_request("body", format!("invalid JSON body: {e}"))
        })
    };
    match body.and_then(|body| pairing_route::write(store, rest, &body, clock::now_ms())) {
        Ok(body) => respond_json(req, 200, &body, origin, false),
        Err(error) => refuse(req, error),
    }
}

/// The refusal of an operator note carrying hidden code points
/// ([`harbard_core::brief::hidden_char`]), or `None`.
///
/// The operator's notes are refused, like their profile and their rules: they see
/// what they pasted. An agent's note (the CLI's note verb, a satellite's
/// event) is kept as written, as evidence of what was sent, and every path
/// that shows it to an agent drops those code points
/// (`brief::visible_text`); refusing it would fail an agent on a pasted log
/// with a BOM.
pub(crate) fn refuse_hidden_note(text: &str) -> Option<Value> {
    let found = harbard_core::brief::hidden_chars_in(text, 4);
    (!found.is_empty()).then(|| {
        json!({
            "error": "invisible_chars",
            "message": format!(
                "the note carries characters you cannot see ({}); they would reach the agents as text. Remove them, or retype it, and send again",
                found.join(", ")
            ),
        })
    })
}

/// `POST /api/v1/digests/read`: mark one digest read. The desk's door.
fn post_digest_read(mut req: Request, store: &Store, cfg: &Config, origin: Option<&str>) {
    let refuse = |req: Request, error: fleet_routes::ApiError| {
        respond_json(req, error.status, &error.body(), origin, false);
    };
    if let Some(error) = desk_door(&req, cfg, origin) {
        refuse(req, error);
        return;
    }
    let Ok(raw) = read_body(&mut req, origin) else {
        refuse(
            req,
            fleet_routes::ApiError::new(
                413,
                "too_large",
                format!("body exceeds {MAX_POST_BYTES} bytes"),
            ),
        );
        return;
    };
    let out = serde_json::from_str::<Value>(&raw)
        .map_err(|e| fleet_routes::ApiError::bad_request("body", format!("invalid JSON body: {e}")))
        .and_then(|body| profile_route::read_digest(store, clock::now_ms(), &body));
    match out {
        Ok(body) => respond_json(req, 200, &body, origin, false),
        Err(error) => refuse(req, error),
    }
}

/// `POST /api/v1/workspace/<action>`: the operator's desk marks, replies and
/// file opens (`serve/workspace_route.rs`). Loopback only, never a satellite.
/// `POST /api/v1/attachments?name=<file>[&label=…]`: the bytes of a file the
/// operator picked in the OS dialog ("Attach file"), stored like
/// `harbard attach` stores one. The operator's desk only: loopback, their
/// origin, never a satellite. Answers the path to cite.
fn post_attachment(
    mut req: Request,
    store: &Store,
    cfg: &Config,
    origin: Option<&str>,
    query: &str,
) {
    let refuse = |req: Request, error: fleet_routes::ApiError| {
        respond_json(req, error.status, &error.body(), origin, false);
    };
    let forbidden = !header_value(&req, "x-harbard-device")
        .unwrap_or_default()
        .is_empty()
        || origin.is_some_and(|o| !is_allowed_origin(o, cfg.requires_auth()))
        || !is_loopback_peer(&req);
    if forbidden {
        refuse(
            req,
            fleet_routes::ApiError::new(403, "forbidden_origin", "the operator's desk only"),
        );
        return;
    }
    let Some(name) = query_param(query, "name").filter(|n| !n.trim().is_empty()) else {
        refuse(
            req,
            fleet_routes::ApiError::bad_request("name", "field `name` is required"),
        );
        return;
    };
    let label = query_param(query, "label").and_then(|l| crate::attach::label_of(&l));
    let cap = crate::attach::MAX_BYTES;
    if req.body_length().is_some_and(|n| n as u64 > cap) {
        refuse(
            req,
            fleet_routes::ApiError::new(413, "too_large", "25 MB at most"),
        );
        return;
    }
    let mut bytes = Vec::new();
    if req
        .as_reader()
        .take(cap + 1)
        .read_to_end(&mut bytes)
        .is_err()
        || bytes.len() as u64 > cap
    {
        refuse(
            req,
            fleet_routes::ApiError::new(413, "too_large", "25 MB at most"),
        );
        return;
    }
    match crate::attach::attach_bytes(store, &name, &bytes, label) {
        Ok(a) => {
            let dir = a
                .path
                .parent()
                .and_then(|d| d.file_name())
                .map(|d| d.to_string_lossy().into_owned())
                .unwrap_or_default();
            let body = json!({
                "ok": true,
                "path": a.path.display().to_string(),
                "id": format!("{dir}/{}", a.name),
                "name": a.name,
                "kind": a.kind,
                "bytes": a.bytes,
                "label": label,
            });
            respond_json(req, 200, &body, origin, false);
        }
        Err(e) => refuse(req, fleet_routes::ApiError::new(422, "refused", e)),
    }
}

fn post_workspace(
    mut req: Request,
    store: &Store,
    cfg: &Config,
    origin: Option<&str>,
    event_hub: &EventHub,
    action: &str,
    caller: &access::Caller,
) {
    let refuse = |req: Request, error: fleet_routes::ApiError| {
        respond_json(req, error.status, &error.body(), origin, false);
    };
    if let Some(error) = desk_door(&req, cfg, origin) {
        refuse(req, error);
        return;
    }
    let Ok(raw) = read_body(&mut req, origin) else {
        refuse(
            req,
            fleet_routes::ApiError::new(
                413,
                "too_large",
                format!("body exceeds {MAX_POST_BYTES} bytes"),
            ),
        );
        return;
    };
    let body: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            refuse(
                req,
                fleet_routes::ApiError::bad_request("body", format!("invalid JSON body: {e}")),
            );
            return;
        }
    };
    let now = clock::now_ms();
    // The workspace the console wrote from: `workspace` in the body, else
    // the one a read without it would show.
    let placement = harbard_core::workspaces::Placement::load(store);
    let requested = body.get("workspace").and_then(Value::as_str);
    let gate = match workspace_route::select(&placement, caller, requested) {
        Ok(Some(id)) => placement.gate_of(&id),
        Ok(None) => harbard_core::workspaces::Gate::Open,
        Err(error) => {
            refuse(req, error);
            return;
        }
    };
    let events = event_hub.history().0;
    let team = || team_route::project(now, harbard_core::team::WINDOW_DEFAULT_MS);
    let mut open = workspace_route::os_open;
    let mut ctx = workspace_route::WriteCtx {
        store,
        events: &events,
        stale_secs: cfg.stale_secs,
        now,
        team: &team,
        open: &mut open,
        gate: &gate,
    };
    match workspace_route::write(&mut ctx, action, &body) {
        Ok(out) => respond_json(req, 200, &out, origin, false),
        Err(error) => refuse(req, error),
    }
}

/// `POST /api/v1/workspaces/<action>`: the operator's desk, like
/// `/workspace/<action>` (loopback, no device header).
fn post_workspaces(
    mut req: Request,
    store: &Store,
    cfg: &Config,
    origin: Option<&str>,
    action: &str,
) {
    let refuse = |req: Request, error: fleet_routes::ApiError| {
        respond_json(req, error.status, &error.body(), origin, false);
    };
    if let Some(error) = desk_door(&req, cfg, origin) {
        refuse(req, error);
        return;
    }
    let Ok(raw) = read_body(&mut req, origin) else {
        refuse(
            req,
            fleet_routes::ApiError::new(
                413,
                "too_large",
                format!("body exceeds {MAX_POST_BYTES} bytes"),
            ),
        );
        return;
    };
    let body: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            refuse(
                req,
                fleet_routes::ApiError::bad_request("body", format!("invalid JSON body: {e}")),
            );
            return;
        }
    };
    match workspaces_route::write(store, action, &body) {
        Ok(out) => respond_json(req, 200, &out, origin, false),
        Err(error) => refuse(req, error),
    }
}

/// `POST /api/v1/update/<check|install|prefs>`: the operator's desk only.
fn post_update(mut req: Request, store: &Store, cfg: &Config, origin: Option<&str>, action: &str) {
    let refuse = |req: Request, error: fleet_routes::ApiError| {
        respond_json(req, error.status, &error.body(), origin, false);
    };
    if let Some(error) = desk_door(&req, cfg, origin) {
        refuse(req, error);
        return;
    }
    let Ok(raw) = read_body(&mut req, origin) else {
        refuse(
            req,
            fleet_routes::ApiError::new(
                413,
                "too_large",
                format!("body exceeds {MAX_POST_BYTES} bytes"),
            ),
        );
        return;
    };
    let body: Value = if raw.trim().is_empty() {
        json!({})
    } else {
        match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                refuse(
                    req,
                    fleet_routes::ApiError::bad_request("body", format!("invalid JSON body: {e}")),
                );
                return;
            }
        }
    };
    match update_route::write(store, action, &body) {
        Ok((status, out)) => respond_json(req, status, &out, origin, false),
        Err(error) => refuse(req, error),
    }
}

/// `POST|PATCH /api/v1/operator` — set the human display name (not authorship).
fn post_operator(mut req: Request, store: &Store, cfg: &Config, origin: Option<&str>) {
    if let Some(o) = origin
        && !is_allowed_origin(o, cfg.requires_auth())
    {
        let body = json!({ "error": "forbidden origin" });
        respond_json(req, 403, &body, None, false);
        return;
    }
    let declared = req.body_length().unwrap_or(0);
    if declared > MAX_POST_BYTES {
        let body = json!({ "error": format!("body exceeds {MAX_POST_BYTES} bytes") });
        respond_json(req, 413, &body, origin, false);
        return;
    }
    let mut raw = String::new();
    if req
        .as_reader()
        .take(MAX_POST_BYTES as u64)
        .read_to_string(&mut raw)
        .is_err()
    {
        let body = json!({ "error": "unreadable body" });
        respond_json(req, 400, &body, origin, false);
        return;
    }
    let parsed: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(_) => {
            let body = json!({ "error": "invalid JSON" });
            respond_json(req, 400, &body, origin, false);
            return;
        }
    };
    let Some(raw_name) = parsed.get("display_name").and_then(|v| v.as_str()) else {
        let body = json!({ "error": "display_name required" });
        respond_json(req, 400, &body, origin, false);
        return;
    };
    let display_name = match sanitize_display_name(raw_name) {
        Ok(n) => n,
        Err(e) => {
            let body = json!({ "error": e });
            respond_json(req, 400, &body, origin, false);
            return;
        }
    };
    // Renaming the desk must not forget where the operator had read up to:
    // the profile file carries both, so load before overwriting.
    let profile = OperatorProfile {
        display_name,
        updated_ms: clock::now_ms(),
        ..OperatorProfile::load(store)
    };
    if let Err(e) = profile.save(store) {
        let body = json!({ "error": format!("could not save operator profile: {e}") });
        respond_json(req, 500, &body, origin, false);
        return;
    }
    let body = json!({
        "session": format!("operator@{}", harbard_core::hostname()),
        "display_name": profile.display_name,
        "updated_ms": profile.updated_ms,
    });
    respond_json(req, 200, &body, origin, false);
}

/// `POST /api/v1/operator/reads` — where the operator has read up to (§4).
///
/// Same door as `/operator`: loopback peers only, and a satellite is refused
/// outright. A paired device is another machine's browser; the read cursors are
/// this desk's state, and letting a satellite move them would mark work read on
/// a screen nobody was looking at.
fn post_operator_reads(mut req: Request, store: &Store, cfg: &Config, origin: Option<&str>) {
    let refuse = |req: Request, error: fleet_routes::ApiError| {
        respond_json(req, error.status, &error.body(), origin, false);
    };

    if !header_value(&req, "x-harbard-device")
        .unwrap_or_default()
        .is_empty()
    {
        refuse(
            req,
            fleet_routes::ApiError::new(
                403,
                "forbidden_origin",
                "satellites cannot move the operator's read cursors",
            ),
        );
        return;
    }
    if let Some(o) = origin
        && !is_allowed_origin(o, cfg.requires_auth())
    {
        respond_json(
            req,
            403,
            &fleet_routes::ApiError::new(
                403,
                "forbidden_origin",
                format!("origin {o} is not allowed"),
            )
            .body(),
            None,
            false,
        );
        return;
    }
    if !is_loopback_peer(&req) {
        refuse(
            req,
            fleet_routes::ApiError::new(
                403,
                "forbidden_origin",
                "the operator's read cursors are loopback-only",
            ),
        );
        return;
    }
    if req.body_length().unwrap_or(0) > MAX_POST_BYTES {
        refuse(
            req,
            fleet_routes::ApiError::new(
                413,
                "too_large",
                format!("body exceeds {MAX_POST_BYTES} bytes"),
            ),
        );
        return;
    }
    let Ok(raw) = read_body(&mut req, origin) else {
        refuse(
            req,
            fleet_routes::ApiError::new(400, "bad_request", "could not read the request body"),
        );
        return;
    };
    match fleet_routes::post_reads(store, &raw) {
        Ok(body) => respond_json(req, 200, &body, origin, false),
        Err(error) => refuse(req, error),
    }
}

/// `POST /api/v1/notes` — the operator speaks (F2).
///
/// Accepts `{"text": "...", "to": {"kind": "...", "value": "..."} | null,
/// "workspace"?: "<id>"}`. Author identity is assigned here and is not a
/// client input: the console can say what it wants, never who it is. A note
/// to a Room is stamped with the workspace it was written in, as
/// `/workspace/reply` stamps one (`workspace` in the body, else the one a read
/// without it would show), so it reaches that workspace's members only.
fn post_note(
    mut req: Request,
    store: &Store,
    cfg: &Config,
    caller: &access::Caller,
    origin: Option<&str>,
) {
    // Loopback-without-token: classic CSRF gate. Hub token mode: Bearer already
    // checked; allow private LAN origins for paired consoles.
    if let Some(o) = origin
        && !is_allowed_origin(o, cfg.requires_auth())
    {
        let body = json!({ "error": "forbidden origin" });
        respond_json(req, 403, &body, None, false);
        return;
    }

    let declared = req.body_length().unwrap_or(0);
    if declared > MAX_POST_BYTES {
        let body = json!({ "error": format!("body exceeds {MAX_POST_BYTES} bytes") });
        respond_json(req, 413, &body, origin, false);
        return;
    }

    let mut raw = String::new();
    if req
        .as_reader()
        .take(MAX_POST_BYTES as u64)
        .read_to_string(&mut raw)
        .is_err()
    {
        let body = json!({ "error": "unreadable body" });
        respond_json(req, 400, &body, origin, false);
        return;
    }

    let parsed: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            let body = json!({ "error": format!("invalid JSON body: {e}") });
            respond_json(req, 400, &body, origin, false);
            return;
        }
    };

    let Some(text) = parsed.get("text").and_then(Value::as_str) else {
        let body = json!({ "error": "field `text` is required and must be a string" });
        respond_json(req, 400, &body, origin, false);
        return;
    };
    let text = text.trim();
    if text.is_empty() {
        let body = json!({ "error": "field `text` must not be empty" });
        respond_json(req, 400, &body, origin, false);
        return;
    }
    // The operator's words reach agents: a hidden code point is refused,
    // named, as in Profile and the Retro.
    if let Some(why) = refuse_hidden_note(text) {
        respond_json(req, 422, &why, origin, false);
        return;
    }

    // Optional `kind`; anything unrecognized is refused rather than silently
    // downgraded, so a client cannot think it sent an interrupt that it did not.
    let note_kind = match parsed.get("kind").and_then(Value::as_str) {
        None => NoteKind::Fyi,
        Some(k) => match NoteKind::parse(k) {
            Some(v) => v,
            None => {
                let body = json!({ "error": format!("unknown kind \"{k}\"; expected fyi | warning | handoff") });
                respond_json(req, 400, &body, origin, false);
                return;
            }
        },
    };

    let to = match parse_recipient_value(parsed.get("to")) {
        Ok(to) => to,
        Err(message) => {
            let body = json!({ "error": message });
            respond_json(req, 400, &body, origin, false);
            return;
        }
    };
    let now = clock::now_ms();
    let to = match resolve_serve_session_recipient(store, cfg.stale_secs, to, now) {
        Ok(to) => to,
        Err((status, message)) => {
            let body = json!({ "error": message });
            respond_json(req, status, &body, origin, false);
            return;
        }
    };

    // warning → needs a named target; handoff → exactly one live session.
    if note_kind == NoteKind::Handoff {
        if let Err(c) = ensure_serve_handoff_one_live_session(store, cfg.stale_secs, to.as_ref()) {
            let body = json!({
                "error": c.line(),
                "correction": c,
            });
            respond_json(req, 400, &body, origin, false);
            return;
        }
    } else if matches!(note_kind, NoteKind::Warning | NoteKind::Ask) && to.is_none() {
        let c = harbard_core::protocol::correction_broadcast_handoff();
        let body = json!({
            "error": c.line(),
            "correction": c,
        });
        respond_json(req, 400, &body, origin, false);
        return;
    }

    if let Some(t) = to.as_ref()
        && t.kind == RecipientKind::Channel
    {
        let m = harbard_core::membership::read(store);
        if !harbard_core::membership::has_members(&m, &t.value) {
            let c = harbard_core::protocol::correction_channel_no_recipients(&t.value);
            let body = json!({
                "error": c.line(),
                "correction": c,
            });
            respond_json(req, 422, &body, origin, false);
            return;
        }
    }

    let note_ref = match parsed.get("ref") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            let s = s.trim();
            if s.is_empty() {
                None
            } else if !harbard_core::id::is_event_prefix(s) {
                let body = json!({ "error": format!("ref \"{s}\" is not an event id or leading ULID prefix") });
                respond_json(req, 400, &body, origin, false);
                return;
            } else {
                let read = store.read(harbard_core::Window::All);
                match harbard_core::id::event(&read.events, s, EventKind::Note) {
                    Ok(parent) => Some(parent.id.clone()),
                    Err(harbard_core::id::ResolveIdError::Unknown) => {
                        let body = json!({ "error": format!("no note id starts with \"{s}\"") });
                        respond_json(req, 404, &body, origin, false);
                        return;
                    }
                    Err(harbard_core::id::ResolveIdError::Ambiguous(count)) => {
                        let body = json!({ "error": format!("ref \"{s}\" matches {count} notes; use the full 26-character id") });
                        respond_json(req, 409, &body, origin, false);
                        return;
                    }
                }
            }
        }
        Some(_) => {
            let body = json!({ "error": "field `ref` must be a string event id or null" });
            respond_json(req, 400, &body, origin, false);
            return;
        }
    };

    // A Room is addressed by name, so the note is placed by its stamp
    // (`harbard_core::rooms`): without one it would pass every workspace's gate.
    let room_workspace = if to
        .as_ref()
        .is_some_and(|t| t.kind == RecipientKind::Channel)
    {
        let placement = harbard_core::workspaces::Placement::load(store);
        let requested = parsed.get("workspace").and_then(Value::as_str);
        match workspace_route::select(&placement, caller, requested) {
            Ok(id) => id,
            Err(error) => {
                respond_json(req, error.status, &error.body(), origin, false);
                return;
            }
        }
    } else {
        None
    };

    let mut event = operator_note_event(text, to, note_kind, note_ref, now);
    if let Some(id) = room_workspace {
        event.cwd = format!("{}{id}", harbard_core::event::WORKSPACE_STAMP);
    }

    // The schema's own bound refuses oversized text with a typed error, so the
    // 2000-byte limit is enforced in exactly one place for every writer.
    if let Err(e) = event.to_line() {
        let body = json!({ "error": e.to_string() });
        respond_json(req, 422, &body, origin, false);
        return;
    }
    if let Err(e) = store.append(&event) {
        let _ = harbard_core::oplog::append(
            store,
            &harbard_core::oplog::OpEvent::error(
                now,
                "serve:post_note",
                &e.to_string(),
                &event.session,
                &event.harness,
            ),
        );
        let body = json!({ "error": format!("append failed: {e}") });
        respond_json(req, 500, &body, origin, false);
        return;
    }
    if let Some(t) = event.to.as_ref()
        && t.kind == RecipientKind::Channel
    {
        let mut m = harbard_core::membership::read(store);
        harbard_core::membership::touch_note_activity(&mut m, &t.value, now);
        let _ = harbard_core::membership::write(store, &m);
        let session = format!("channel:{}", harbard_core::membership::normalize(&t.value));
        let _ = harbard_core::ResolutionStore::at(store.root()).advance_latest(
            &session,
            harbard_core::protocol::codes::CHANNEL_UNUSED,
            harbard_core::ReceiptStage::Applied,
            now,
            Some("first note"),
        );
    }
    let _ = harbard_core::oplog::append(
        store,
        &harbard_core::oplog::OpEvent::note_appended(
            now,
            &event.id,
            note_kind.as_str(),
            event
                .to
                .as_ref()
                .map(|t| format!("{}:{}", t.kind.as_str(), t.value)),
            "serve",
            &event.session,
            &event.harness,
        ),
    );
    harbard_core::learning::record(
        store,
        &harbard_core::learning::note(
            now,
            &event.id,
            note_kind.as_str(),
            event
                .to
                .as_ref()
                .map(|t| format!("{}:{}", t.kind.as_str(), t.value)),
            "serve",
            &event.session,
            &event.harness,
        ),
    );

    let event_body = serde_json::to_value(&event).unwrap_or_else(|_| json!({}));
    let body = json!({ "event": event_body });
    respond_json(req, 201, &body, origin, false);
}

/// `POST /api/v1/evals` — the operator rates a session (never model weights).
///
/// Body: `{"session","harness","score":1..=5,"note"?,"intent"?}`. Host is
/// stamped here. Appends to `evals.jsonl` under the store root.
fn post_eval(mut req: Request, store: &Store, origin: Option<&str>) {
    if let Some(o) = origin
        && !is_localhost_origin(o)
    {
        let body = json!({ "error": "forbidden origin" });
        respond_json(req, 403, &body, None, false);
        return;
    }

    let declared = req.body_length().unwrap_or(0);
    if declared > MAX_POST_BYTES {
        let body = json!({ "error": format!("body exceeds {MAX_POST_BYTES} bytes") });
        respond_json(req, 413, &body, origin, false);
        return;
    }

    let mut raw = String::new();
    if req
        .as_reader()
        .take(MAX_POST_BYTES as u64)
        .read_to_string(&mut raw)
        .is_err()
    {
        let body = json!({ "error": "unreadable body" });
        respond_json(req, 400, &body, origin, false);
        return;
    }

    let parsed: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            let body = json!({ "error": format!("invalid JSON body: {e}") });
            respond_json(req, 400, &body, origin, false);
            return;
        }
    };

    let Some(session) = parsed.get("session").and_then(Value::as_str).map(str::trim) else {
        let body = json!({ "error": "field `session` is required" });
        respond_json(req, 400, &body, origin, false);
        return;
    };
    if session.is_empty() {
        let body = json!({ "error": "field `session` must not be empty" });
        respond_json(req, 400, &body, origin, false);
        return;
    }

    let Some(harness) = parsed.get("harness").and_then(Value::as_str).map(str::trim) else {
        let body = json!({ "error": "field `harness` is required" });
        respond_json(req, 400, &body, origin, false);
        return;
    };
    if harness.is_empty() {
        let body = json!({ "error": "field `harness` must not be empty" });
        respond_json(req, 400, &body, origin, false);
        return;
    }

    let Some(score) = parsed.get("score").and_then(Value::as_u64) else {
        let body = json!({ "error": "field `score` is required (1..=5)" });
        respond_json(req, 400, &body, origin, false);
        return;
    };
    if score > 5 {
        let body = json!({ "error": "field `score` must be 1..=5" });
        respond_json(req, 400, &body, origin, false);
        return;
    }

    let note = parsed
        .get("note")
        .and_then(Value::as_str)
        .map(|s| s.to_string());
    let intent = parsed
        .get("intent")
        .and_then(Value::as_str)
        .map(|s| s.to_string());

    let ev = match harbard_core::eval::new_event(
        session,
        harness,
        score as u8,
        note,
        intent,
        Some(harbard_core::hostname()),
    ) {
        Ok(ev) => ev,
        Err(e) => {
            let body = json!({ "error": e });
            respond_json(req, 400, &body, origin, false);
            return;
        }
    };

    if let Err(e) = harbard_core::eval::record(store, &ev) {
        let body = json!({ "error": format!("append failed: {e}") });
        respond_json(req, 500, &body, origin, false);
        return;
    }

    let body = json!({ "eval": ev });
    respond_json(req, 201, &body, origin, false);
}

/// Parse a membership anchor from a channel action body.
///
/// Accepts either `{"member":{"kind":"…","value":"…"}}` or top-level
/// `{"kind":"…","value":"…"}` for console convenience.
fn parse_member_value(parsed: &Value) -> Result<harbard_core::membership::Member, String> {
    use harbard_core::membership::{AnchorKind, Member};

    let (kind_str, value_str) = if let Some(member) = parsed.get("member") {
        let obj = member
            .as_object()
            .ok_or_else(|| "field `member` must be an object".to_string())?;
        let kind = obj
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| "member.kind is required".to_string())?;
        let value = obj
            .get("value")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "member.value must be a non-empty string".to_string())?;
        (kind, value)
    } else {
        let kind = parsed.get("kind").and_then(Value::as_str).ok_or_else(|| {
            "field `kind` is required for join/leave (or pass a `member` object)".to_string()
        })?;
        let value = parsed
            .get("value")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| "field `value` must be a non-empty string".to_string())?;
        (kind, value)
    };
    let kind = AnchorKind::parse(kind_str)
        .ok_or_else(|| format!("unknown kind \"{kind_str}\"; expected repo | session | harness"))?;
    Ok(Member {
        kind,
        value: value_str.to_string(),
    })
}

/// Append a join/leave narrative event after a membership snapshot write.
fn append_operator_membership_event(
    store: &Store,
    kind: EventKind,
    channel: &str,
    member: &harbard_core::membership::Member,
) {
    let host = harbard_core::hostname();
    let now = clock::now_ms();
    let text = format!("as {}:{}", member.kind.as_str(), member.value);
    let event = Event {
        v: SCHEMA_VERSION,
        id: Ulid::generate().to_string(),
        ts: clock::rfc3339_utc(now),
        event: kind,
        session: format!("operator@{host}"),
        harness: harbard_core::HARNESS_HUMAN.to_string(),
        host: host.clone(),
        cwd: String::new(),
        repo: None,
        repo_key: None,
        branch: None,
        role: Role::Operator,
        parent: None,
        note_ref: None,
        text,
        to: Some(Recipient {
            kind: RecipientKind::Channel,
            value: channel.to_string(),
        }),
        kind: NoteKind::Fyi,
    };
    let _ = store.append(&event);
}

/// A refusal the console words from its code.
fn channel_refusal(status: u16, code: &'static str, message: String, name: &str) -> (u16, Value) {
    let e =
        fleet_routes::ApiError::new(status, code, message).with("params", json!({ "name": name }));
    (e.status, e.body())
}

/// Append a lifecycle event for an operator action. A failed append never
/// undoes the action: the snapshot was already written.
fn append_operator_lifecycle(
    store: &Store,
    action: harbard_core::rooms::LifecycleAction,
    channel: &str,
    workspace: Option<&str>,
    purpose: Option<&str>,
    repo: Option<&str>,
) {
    let _ = harbard_core::rooms::append_lifecycle(
        store,
        &harbard_core::rooms::LifecycleActor::operator(),
        &harbard_core::rooms::Lifecycle {
            action,
            channel,
            workspace,
            purpose,
            repo,
        },
        clock::now_ms(),
    );
}

/// The repository of project channel `id` in the view of this workspace.
fn project_channel_repo(
    store: &Store,
    stale_secs: u64,
    event_hub: &EventHub,
    gate: &harbard_core::workspaces::Gate<'_>,
    id: &str,
) -> Option<String> {
    let now = clock::now_ms();
    let team = team_route::project(now, harbard_core::team::WINDOW_DEFAULT_MS);
    let events = event_hub.history().0;
    workspace_route::view_in(store, &events, stale_secs, &team, now, gate)
        .channels
        .into_iter()
        .find(|c| c.id == id && c.kind == "project")
        .map(|c| c.repo)
}

/// `POST /api/v1/channels` — Room membership, metadata, reading, and hygiene.
///
/// Body: `{"action":"create"|"update"|"archive"|"unarchive"|"delete"|"join"|"leave"|"mark_read","name":"…","workspace":"<id>", …}`.
/// Every lifecycle action appends a `channel_lifecycle` event; join/leave
/// append theirs when membership changes. Nothing here removes an event.
fn post_channel_action(
    mut req: Request,
    store: &Store,
    stale_secs: u64,
    origin: Option<&str>,
    event_hub: &EventHub,
    caller: &access::Caller,
) {
    if let Some(o) = origin
        && !is_localhost_origin(o)
    {
        let body = json!({ "error": "forbidden origin" });
        respond_json(req, 403, &body, None, false);
        return;
    }

    let declared = req.body_length().unwrap_or(0);
    if declared > MAX_POST_BYTES {
        let body = json!({ "error": format!("body exceeds {MAX_POST_BYTES} bytes") });
        respond_json(req, 413, &body, origin, false);
        return;
    }

    let mut raw = String::new();
    if req
        .as_reader()
        .take(MAX_POST_BYTES as u64)
        .read_to_string(&mut raw)
        .is_err()
    {
        let body = json!({ "error": "unreadable body" });
        respond_json(req, 400, &body, origin, false);
        return;
    }

    let parsed: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            let body = json!({ "error": format!("invalid JSON body: {e}") });
            respond_json(req, 400, &body, origin, false);
            return;
        }
    };

    let Some(action) = parsed.get("action").and_then(Value::as_str) else {
        let body = json!({
            "error": "field `action` is required (archive | unarchive | delete | archive_bulk | join | leave | create | update | mark_read)"
        });
        respond_json(req, 400, &body, origin, false);
        return;
    };

    let mut m = harbard_core::membership::read(store);
    let placement = harbard_core::workspaces::Placement::load(store);
    let workspace = match workspace_route::select(
        &placement,
        caller,
        parsed.get("workspace").and_then(Value::as_str),
    ) {
        Ok(id) => id,
        Err(e) => {
            respond_json(req, e.status, &e.body(), origin, false);
            return;
        }
    };
    let ws = workspace.as_deref();
    let gate = match ws {
        Some(id) => placement.gate_of(id),
        None => harbard_core::workspaces::Gate::Open,
    };
    let home = std::env::var("HOME").unwrap_or_default();
    // A room in another workspace is out of this one's hands.
    let elsewhere = |m: &harbard_core::membership::Memberships, name: &str| {
        ws.is_some_and(|id| {
            !harbard_core::membership::room_workspaces(m, name, &placement, &home).contains(id)
        })
    };

    if action == "archive_bulk" {
        let names: Vec<String> = match parsed.get("names").and_then(Value::as_array) {
            Some(arr) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .collect(),
            None => {
                let body = json!({ "error": "field `names` must be a non-empty string array" });
                respond_json(req, 400, &body, origin, false);
                return;
            }
        };
        if names.is_empty() {
            let body = json!({ "error": "field `names` must be a non-empty string array" });
            respond_json(req, 400, &body, origin, false);
            return;
        }
        let items = harbard_core::membership::archive_bulk(&mut m, &names);
        for item in items
            .iter()
            .filter(|i| i.status == harbard_core::BulkArchiveStatus::Archived)
        {
            append_operator_lifecycle(
                store,
                harbard_core::rooms::LifecycleAction::Archive,
                &item.name,
                ws,
                None,
                None,
            );
        }
        if items
            .iter()
            .any(|i| i.status == harbard_core::BulkArchiveStatus::Archived)
            && let Err(e) = harbard_core::membership::write(store, &m)
        {
            let body = json!({ "error": format!("write failed: {e}") });
            respond_json(req, 500, &body, origin, false);
            return;
        }
        let body = json!({
            "ok": true,
            "action": "archive_bulk",
            "items": items,
            "archived": m.archived,
        });
        respond_json(req, 200, &body, origin, false);
        return;
    }

    let Some(name) = parsed.get("name").and_then(Value::as_str).map(str::trim) else {
        let body = json!({ "error": "field `name` is required" });
        respond_json(req, 400, &body, origin, false);
        return;
    };
    if name.is_empty() {
        let body = json!({ "error": "field `name` must not be empty" });
        respond_json(req, 400, &body, origin, false);
        return;
    }

    match action {
        "update" => {
            if harbard_core::membership::is_built_in(name) {
                let (status, body) = channel_refusal(
                    422,
                    "room_built_in",
                    format!("#{name} is built in and cannot be edited"),
                    name,
                );
                respond_json(req, status, &body, origin, false);
                return;
            }
            if elsewhere(&m, name) {
                let (status, body) = channel_refusal(
                    409,
                    "room_other_workspace",
                    format!("#{name} belongs to another workspace"),
                    name,
                );
                respond_json(req, status, &body, origin, false);
                return;
            }
            let visibility = match parsed.get("visibility").and_then(Value::as_str) {
                Some(raw) => match harbard_core::RoomVisibility::parse(raw) {
                    Some(value) => value,
                    None => {
                        let body =
                            json!({ "error": "field `visibility` must be shared | limited" });
                        respond_json(req, 400, &body, origin, false);
                        return;
                    }
                },
                // The console's rename of a purpose leaves visibility alone.
                None => m
                    .meta
                    .get(&harbard_core::membership::normalize(name))
                    .map(|meta| meta.visibility)
                    .unwrap_or_default(),
            };
            let purpose = match parsed.get("purpose") {
                None | Some(Value::Null) => None,
                Some(Value::String(value)) => Some(value.as_str()),
                Some(_) => {
                    let body = json!({ "error": "field `purpose` must be a string or null" });
                    respond_json(req, 400, &body, origin, false);
                    return;
                }
            };
            let changed = match harbard_core::update_room(&mut m, name, purpose, visibility) {
                Ok(changed) => changed,
                Err(error) => {
                    let body = json!({ "error": error });
                    respond_json(req, 422, &body, origin, false);
                    return;
                }
            };
            if changed && let Err(error) = harbard_core::membership::write(store, &m) {
                let body = json!({ "error": format!("write failed: {error}") });
                respond_json(req, 500, &body, origin, false);
                return;
            }
            let name = harbard_core::membership::normalize(name);
            if changed {
                append_operator_lifecycle(
                    store,
                    harbard_core::rooms::LifecycleAction::Update,
                    &name,
                    ws,
                    purpose,
                    None,
                );
            }
            let body = json!({
                "ok": true,
                "action": action,
                "name": name,
                "changed": changed,
                "meta": m.meta.get(&name),
            });
            respond_json(req, 200, &body, origin, false);
        }
        "mark_read" => {
            let Some(event_id) = parsed
                .get("event_id")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|value| !value.is_empty())
            else {
                let body = json!({ "error": "field `event_id` is required" });
                respond_json(req, 400, &body, origin, false);
                return;
            };
            let name = harbard_core::membership::normalize(name);
            let is_room_event = store
                .read(harbard_core::Window::All)
                .events
                .iter()
                .any(|event| {
                    event.id == event_id
                        && gate.places(event)
                        && harbard_core::rooms::is_note_in(event, &name)
                });
            if !is_room_event {
                let body = json!({
                    "error": format!("event \"{event_id}\" is not a note in room \"{name}\"")
                });
                respond_json(req, 422, &body, origin, false);
                return;
            }
            let changed = match harbard_core::mark_room_read(&mut m, &name, ws, event_id) {
                Ok(changed) => changed,
                Err(error) => {
                    let body = json!({ "error": error });
                    respond_json(req, 422, &body, origin, false);
                    return;
                }
            };
            if changed && let Err(error) = harbard_core::membership::write(store, &m) {
                let body = json!({ "error": format!("write failed: {error}") });
                respond_json(req, 500, &body, origin, false);
                return;
            }
            let body = json!({
                "ok": true,
                "action": action,
                "name": name,
                "changed": changed,
                "event_id": m.reads.get(&harbard_core::membership::read_key(&name, ws)),
            });
            respond_json(req, 200, &body, origin, false);
        }
        "archive" | "unarchive" | "delete" => {
            use harbard_core::rooms::LifecycleAction;
            let name = harbard_core::membership::normalize(name);
            if harbard_core::membership::is_built_in(&name) {
                let (status, body) = channel_refusal(
                    422,
                    "room_built_in",
                    format!("#{name} is built in: it cannot be archived or deleted"),
                    &name,
                );
                respond_json(req, status, &body, origin, false);
                return;
            }
            let lifecycle = match action {
                "archive" => LifecycleAction::Archive,
                "unarchive" => LifecycleAction::Unarchive,
                _ => LifecycleAction::Delete,
            };
            // Not a room: a project channel, named by its id. Those can be
            // hidden from the rail, never deleted.
            if !m.channels.contains_key(&name) {
                let Some(repo) = project_channel_repo(store, stale_secs, event_hub, &gate, &name)
                else {
                    let body = json!({ "error": format!("unknown channel \"{name}\"") });
                    respond_json(req, 422, &body, origin, false);
                    return;
                };
                if lifecycle == LifecycleAction::Delete {
                    let (status, body) = channel_refusal(
                        422,
                        "project_channel",
                        format!("#{name} is a project channel: it can be archived, not deleted"),
                        &name,
                    );
                    respond_json(req, status, &body, origin, false);
                    return;
                }
                let changed = if lifecycle == LifecycleAction::Archive {
                    m.archived_repos.insert(repo.clone())
                } else {
                    m.archived_repos.remove(&repo)
                };
                if changed {
                    if let Err(e) = harbard_core::membership::write(store, &m) {
                        let body = json!({ "error": format!("write failed: {e}") });
                        respond_json(req, 500, &body, origin, false);
                        return;
                    }
                    append_operator_lifecycle(store, lifecycle, &name, ws, None, Some(&repo));
                }
                let body = json!({
                    "ok": true,
                    "action": action,
                    "name": name,
                    "changed": changed,
                    "project": true,
                });
                respond_json(req, 200, &body, origin, false);
                return;
            }
            if elsewhere(&m, &name) {
                let (status, body) = channel_refusal(
                    409,
                    "room_other_workspace",
                    format!("#{name} belongs to another workspace"),
                    &name,
                );
                respond_json(req, status, &body, origin, false);
                return;
            }
            let mut unsubscribed = Vec::new();
            let result = match lifecycle {
                LifecycleAction::Archive => {
                    harbard_core::membership::archive_unsubscribing(&mut m, &name).map(|gone| {
                        unsubscribed = gone;
                    })
                }
                LifecycleAction::Unarchive => harbard_core::membership::unarchive(&mut m, &name),
                _ => {
                    if !harbard_core::membership::is_archived(&m, &name) {
                        let (status, body) = channel_refusal(
                            409,
                            "room_not_archived",
                            format!("#{name} must be archived before it can be deleted"),
                            &name,
                        );
                        respond_json(req, status, &body, origin, false);
                        return;
                    }
                    harbard_core::membership::delete_archived(&mut m, &name)
                }
            };
            if let Err(e) = result {
                let body = json!({ "error": e });
                respond_json(req, 422, &body, origin, false);
                return;
            }
            if let Err(e) = harbard_core::membership::write(store, &m) {
                let body = json!({ "error": format!("write failed: {e}") });
                respond_json(req, 500, &body, origin, false);
                return;
            }
            for member in &unsubscribed {
                append_operator_membership_event(store, EventKind::Leave, &name, member);
            }
            append_operator_lifecycle(store, lifecycle, &name, ws, None, None);
            if action == "archive" {
                let session = format!("channel:{name}");
                let _ = harbard_core::ResolutionStore::at(store.root()).advance_latest(
                    &session,
                    harbard_core::protocol::codes::CHANNEL_UNUSED,
                    harbard_core::ReceiptStage::Applied,
                    clock::now_ms(),
                    Some("archived from the console"),
                );
            }
            let body = json!({
                "ok": true,
                "action": action,
                "name": name,
                "archived": m.archived,
                "unsubscribed": unsubscribed.len(),
            });
            respond_json(req, 200, &body, origin, false);
        }
        "create" => {
            // A room can be created empty: the operator names it, agents join.
            // An initial member, when given, joins in the same write.
            let name = harbard_core::membership::normalize(name);
            if !harbard_core::membership::is_valid_name(&name) {
                let body = json!({
                    "error": format!(
                        "invalid channel name \"{name}\"; use letters, digits, - _ . (max 48)"
                    )
                });
                respond_json(req, 422, &body, origin, false);
                return;
            }
            if harbard_core::membership::is_built_in(&name) {
                let (status, body) = channel_refusal(
                    422,
                    "room_built_in",
                    format!("#{name} is built in and already exists"),
                    &name,
                );
                respond_json(req, status, &body, origin, false);
                return;
            }
            if !m.channels.contains_key(&name)
                && project_channel_repo(store, stale_secs, event_hub, &gate, &name).is_some()
            {
                let (status, body) = channel_refusal(
                    409,
                    "project_channel",
                    format!("#{name} is already this workspace's project channel"),
                    &name,
                );
                respond_json(req, status, &body, origin, false);
                return;
            }
            let existing_home = m.meta.get(&name).and_then(|meta| meta.workspace.clone());
            if let (Some(id), Some(home_id)) = (ws, existing_home.as_deref())
                && id != home_id
            {
                let (status, body) = channel_refusal(
                    409,
                    "room_name_taken",
                    format!("#{name} already exists in another workspace"),
                    &name,
                );
                respond_json(req, status, &body, origin, false);
                return;
            }
            let member = if parsed.get("member").is_some() || parsed.get("kind").is_some() {
                match parse_member_value(&parsed) {
                    Ok(member) => Some(member),
                    Err(message) => {
                        let body = json!({ "error": message });
                        respond_json(req, 400, &body, origin, false);
                        return;
                    }
                }
            } else {
                None
            };
            let mut changed = !m.channels.contains_key(&name) || m.archived.contains(&name);
            match &member {
                Some(member) => {
                    changed |= harbard_core::membership::join(&mut m, &name, member.clone());
                }
                None => {
                    m.archived.remove(&name);
                    harbard_core::membership::create(&mut m, &name);
                }
            }
            if let Some(id) = ws {
                m.meta.entry(name.clone()).or_default().workspace = Some(id.to_string());
            }
            let purpose = parsed.get("purpose").and_then(Value::as_str);
            if purpose.is_some() {
                let visibility = m
                    .meta
                    .get(&name)
                    .map(|meta| meta.visibility)
                    .unwrap_or_default();
                if let Err(error) = harbard_core::update_room(&mut m, &name, purpose, visibility) {
                    let body = json!({ "error": error });
                    respond_json(req, 422, &body, origin, false);
                    return;
                }
            }
            if let Err(e) = harbard_core::membership::write(store, &m) {
                let body = json!({ "error": format!("write failed: {e}") });
                respond_json(req, 500, &body, origin, false);
                return;
            }
            if let Some(member) = &member {
                append_operator_membership_event(store, EventKind::Join, &name, member);
            }
            if changed {
                append_operator_lifecycle(
                    store,
                    harbard_core::rooms::LifecycleAction::Create,
                    &name,
                    ws,
                    purpose,
                    None,
                );
            }
            let body = json!({
                "ok": true,
                "action": "create",
                "name": name,
                "changed": changed,
                "joined": member.is_some(),
            });
            respond_json(req, 200, &body, origin, false);
        }
        "join" | "leave" => {
            let name = harbard_core::membership::normalize(name);
            if !harbard_core::membership::is_valid_name(&name) {
                let body = json!({
                    "error": format!(
                        "invalid channel name \"{name}\"; use letters, digits, - _ . (max 48)"
                    )
                });
                respond_json(req, 422, &body, origin, false);
                return;
            }
            let member = match parse_member_value(&parsed) {
                Ok(m) => m,
                Err(message) => {
                    let body = json!({ "error": message });
                    respond_json(req, 400, &body, origin, false);
                    return;
                }
            };
            let changed = if action == "join" {
                harbard_core::membership::join(&mut m, &name, member.clone())
            } else {
                harbard_core::membership::leave(&mut m, &name, &member)
            };
            if changed {
                if let Err(e) = harbard_core::membership::write(store, &m) {
                    let body = json!({ "error": format!("write failed: {e}") });
                    respond_json(req, 500, &body, origin, false);
                    return;
                }
                let kind = if action == "join" {
                    EventKind::Join
                } else {
                    EventKind::Leave
                };
                append_operator_membership_event(store, kind, &name, &member);
                if action == "leave" {
                    let emptied = m
                        .channels
                        .get(&name)
                        .is_none_or(|members| members.is_empty());
                    if emptied {
                        let session = format!("channel:{name}");
                        let _ = harbard_core::ResolutionStore::at(store.root()).advance_latest(
                            &session,
                            harbard_core::protocol::codes::CHANNEL_UNUSED,
                            harbard_core::ReceiptStage::Rejected,
                            clock::now_ms(),
                            Some("left membership"),
                        );
                    }
                }
            }
            let body = json!({
                "ok": true,
                "action": action,
                "name": name,
                "changed": changed,
            });
            respond_json(req, 200, &body, origin, false);
        }
        other => {
            let body = json!({
                "error": format!(
                    "unknown action \"{other}\"; expected archive | unarchive | delete | join | leave | create | update | mark_read"
                )
            });
            respond_json(req, 400, &body, origin, false);
        }
    }
}

/// Handoff must name exactly one known `session:<id>` (cardinality).
fn ensure_serve_handoff_one_live_session(
    store: &Store,
    stale_secs: u64,
    recipient: Option<&Recipient>,
) -> Result<(), harbard_core::Correction> {
    let c = harbard_core::protocol::correction_handoff_target_cardinality();
    let Some(to) = recipient else {
        return Err(c);
    };
    if to.kind != RecipientKind::Session {
        return Err(c);
    }
    let now = clock::now_ms();
    let read = store.read(harbard_core::Window::All);
    let views = query::sessions(store, &read.events, now, stale_secs);
    let known = views
        .iter()
        .any(|v| v.session == to.value && v.harness != harbard_core::HARNESS_HUMAN);
    if !known {
        return Err(c);
    }
    Ok(())
}

/// Decode an optional `to` object into a [`Recipient`], refusing malformed
/// shapes loudly instead of silently broadcasting to the whole fleet.
fn parse_recipient_value(value: Option<&Value>) -> Result<Option<Recipient>, String> {
    let Some(v) = value else { return Ok(None) };
    if v.is_null() {
        return Ok(None);
    }
    let obj = v
        .as_object()
        .ok_or_else(|| "field `to` must be an object or null".to_string())?;
    let kind = obj
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| "`to.kind` must be one of harness | session | repo".to_string())?;
    let value = obj
        .get("value")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "`to.value` must be a non-empty string".to_string())?;
    let kind = match kind {
        "harness" => RecipientKind::Harness,
        "session" => RecipientKind::Session,
        "repo" => RecipientKind::Repo,
        "channel" => RecipientKind::Channel,
        other => {
            return Err(format!(
                "unknown `to.kind` \"{other}\"; expected harness | session | repo | channel"
            ));
        }
    };
    // Channel names normalize at every door, so one name is one channel
    // however the client typed it.
    let value = if kind == RecipientKind::Channel {
        let name = harbard_core::membership::normalize(value);
        if !harbard_core::membership::is_valid_name(&name) {
            return Err(format!("invalid channel name \"{value}\""));
        }
        name
    } else if kind == RecipientKind::Repo {
        // Same door, same canonicalization as the CLI. This path did nothing
        // but trim, so a checkout path typed one way by the console and another
        // way by an agent addressed two different populations, and a note to a
        // worktree reached nobody at all.
        crate::context::canonical_repo_recipient(std::path::Path::new(value.trim()))
    } else {
        value.trim().to_string()
    };
    Ok(Some(Recipient { kind, value }))
}

/// Session recipients are canonicalized before append. A short value is safe
/// only when it has exactly one expansion; an invented or ambiguous suffix is
/// a refusal, never a successful note to nobody.
fn resolve_serve_session_recipient(
    store: &Store,
    stale_secs: u64,
    recipient: Option<Recipient>,
    now: u64,
) -> Result<Option<Recipient>, (u16, String)> {
    let Some(mut recipient) = recipient else {
        return Ok(None);
    };
    if recipient.kind != RecipientKind::Session {
        return Ok(Some(recipient));
    }
    let read = store.read(harbard_core::Window::All);
    let sessions = query::sessions(store, &read.events, now, stale_secs);
    match harbard_core::id::session(&sessions, &recipient.value) {
        Ok(session) => {
            recipient.value.clone_from(&session.session);
            Ok(Some(recipient))
        }
        Err(harbard_core::id::ResolveIdError::Unknown) => Err((
            404,
            format!(
                "no session on this bus starts with \"{}\"; a note to an unknown id reaches nobody",
                recipient.value
            ),
        )),
        Err(harbard_core::id::ResolveIdError::Ambiguous(count)) => Err((
            409,
            format!(
                "session prefix \"{}\" matches {count} sessions; use the full id",
                recipient.value
            ),
        )),
    }
}

/// A note authored by the operator. Identity is ours, not the caller's: one
/// stable operator session per host, so the console's messages thread like
/// any other participant. The caller chooses only text, addressee, kind and
/// the note it answers.
pub(crate) fn operator_note_event(
    text: &str,
    to: Option<Recipient>,
    kind: NoteKind,
    note_ref: Option<String>,
    now: u64,
) -> Event {
    let host = harbard_core::hostname();
    Event {
        v: SCHEMA_VERSION,
        id: Ulid::generate().to_string(),
        ts: clock::rfc3339_utc(now),
        event: EventKind::Note,
        session: format!("operator@{host}"),
        harness: harbard_core::HARNESS_HUMAN.to_string(),
        host,
        cwd: String::new(),
        repo: None,
        repo_key: None,
        branch: None,
        role: Role::Operator,
        parent: None,
        note_ref,
        text: text.to_string(),
        to,
        // The console posts context, not interrupts. An operator who needs to
        // stop an agent mid-task picks the kind explicitly.
        kind,
    }
}

/// Answer a CORS preflight for the one write door.
fn respond_preflight(req: Request, origin: Option<&str>) {
    let mut headers = Vec::new();
    push_cors(&mut headers, origin);
    headers.push(header("Access-Control-Allow-Headers", "content-type"));
    headers.push(header("Access-Control-Max-Age", "600"));
    let response = Response::new(StatusCode(204), headers, io::empty(), Some(0), None);
    record_status(204);
    let _ = req.respond(response);
}

/// `GET /api/v1/sessions` body: every windowed session, newest signal first.
fn sessions_body_from(store: &Store, events: &[Event], stale_secs: u64) -> Value {
    let now = clock::now_ms();
    let mut views = query::sessions(store, events, now, stale_secs);
    for v in &mut views {
        // The contract's liveness is binary (live|stale); a cleanly-ended
        // session is simply not live.
        if v.liveness == Liveness::Ended {
            v.liveness = Liveness::Stale;
        }
    }
    // Newest signal first; stable so ties keep first-seen order.
    views.sort_by_key(|v| std::cmp::Reverse(v.last_ms));
    json!({ "sessions": views, "now_ms": now })
}

/// `GET /api/v1/leases` body: who currently owns which checkout.
///
/// Strictly informational, and it stays that way: the lease store is
/// authoritative state whose every mutation needs a resource lock plus owner
/// proof, so no lease verb belongs on this surface. A listing takes no lock, so a holder can
/// vanish the instant after the scan — read it as a board, never as a
/// precondition.
///
/// Only ACTIVE writer leases appear. Expired and dead-supervisor records are
/// filtered in core, so a lapsed claim can never be shown as an owner, and
/// host-global service leases are not checkout ownership at all. The holder's
/// pid is never disclosed: `/api/v1/sessions` has never published one, and a
/// paired console holding the hub token should not be handed kill targets.
fn leases_body(store: &Store) -> Value {
    let leases = LeaseStore::at(store.root());
    let writers = leases.active_writers(SystemLeaseClock, &SystemPidChecker);
    json!({ "leases": writers, "now_ms": clock::now_ms() })
}

/// The local dependency projection is informational. A listener means a CLI
/// invocation is waiting, never that a harness will automatically resume.
fn dependencies_body(store: &Store) -> Result<Value, (u16, Value)> {
    if crate::dependency::local_only(store).is_err() {
        return Err((
            501,
            json!({
                "available": false,
                "dependencies": [],
                "error": "Dependencies require a local shared store; remote hub transport is not supported.",
                "code": "dependency_remote_unavailable",
            }),
        ));
    }
    let views = crate::dependency::cached_views(store).map_err(|error| (500, json!({
        "available": false, "dependencies": [], "error": error, "code": "dependency_projection_failed",
    })))?;
    let dependencies: Vec<_> = views
        .into_iter()
        .map(|view| {
            let active_cli_wait = crate::dependency::listener_active(store, &view.id);
            let mut value = json!(view);
            value["active_cli_wait"] = json!(active_cli_wait);
            value["automatic_resume"] = json!(false);
            value
        })
        .collect();
    Ok(json!({ "available": true, "dependencies": dependencies }))
}

/// `GET /api/v1/notes/{id}/trace` — same reconstruction as `harbard trace --note`.
/// Distinct from `/sessions/{id}/trace`, which is harness CoT.
/// `GET /api/v1/notes/{id}/thread`: a conversation, whole.
///
/// Reads the store rather than the hub's in-memory window. The console keeps
/// only the last 500 events, so serialising a thread from the client would
/// silently drop the head of a long or older conversation — and a copy button
/// that quietly truncates is worse than no copy button, because the operator
/// pastes it somewhere and never learns what is missing.
fn thread_body(store: &Store, path: &str) -> Result<Value, (u16, Value)> {
    let id = path
        .strip_prefix("/api/v1/notes/")
        .and_then(|rest| rest.strip_suffix("/thread"))
        .unwrap_or("")
        .trim();
    if id.is_empty() || id.contains('/') {
        return Err((
            400,
            json!({ "error": "expected /api/v1/notes/{id}/thread" }),
        ));
    }
    let read = store.read(Window::All);
    let Some(thread) = harbard_core::thread::thread_for(&read.events, id) else {
        return Err((404, json!({ "error": "unknown_note", "id": id })));
    };
    let aliases = harbard_core::alias::read(store);
    let messages: Vec<Value> = thread
        .events()
        .map(|e| {
            json!({
                "id": e.id,
                "ts": e.ts,
                "event": e.event.as_str(),
                "kind": e.kind.as_str(),
                "session": e.session,
                "name": harbard_core::alias::get(&aliases, &e.session)
                    .map(|a| harbard_core::alias::display(&a)),
                "harness": e.harness,
                "to": e.to.as_ref().map(|r| format!("{}:{}", r.kind.as_str(), r.value)),
                "ref": e.note_ref,
                "text": e.text,
            })
        })
        .collect();
    Ok(json!({
        "thread": thread.root.id,
        "count": thread.len(),
        "messages": messages,
    }))
}

fn note_trace_body(store: &Store, stale_secs: u64, path: &str) -> Result<Value, (u16, Value)> {
    let id = path
        .strip_prefix("/api/v1/notes/")
        .and_then(|rest| rest.strip_suffix("/trace"))
        .unwrap_or("")
        .trim();
    if id.is_empty() || id.contains('/') {
        return Err((400, json!({ "error": "expected /api/v1/notes/{id}/trace" })));
    }
    match harbard_core::note_trace::report_for_id(store, id, clock::now_ms(), stale_secs) {
        Ok(report) => Ok(serde_json::to_value(&report).unwrap_or_else(|_| json!({}))),
        Err(harbard_core::id::ResolveIdError::Unknown) => {
            Err((404, json!({ "error": "unknown_note", "id": id })))
        }
        Err(harbard_core::id::ResolveIdError::Ambiguous(count)) => Err((
            409,
            json!({ "error": "ambiguous_note", "id": id, "matches": count }),
        )),
    }
}

/// `GET /api/v1/history/checkouts` — after-the-fact ledger per tree.
/// Keeps real `Ended` (unlike `/history/sessions`, which rewrites it to Stale).
fn history_checkouts_body(store: &Store, stale_secs: u64) -> Value {
    let hist = harbard_core::checkout_history::build_now(store, stale_secs);
    serde_json::to_value(&hist).unwrap_or_else(|_| json!({ "checkouts": [], "now_ms": 0 }))
}

/// `GET /api/v1/sessions/{id}/trace` — last N thinking/status lines from the
/// harness transcript on disk. Pull-only; never appends to the bus.
fn session_trace_body(
    store: &Store,
    events: &[Event],
    horizon_ms: u64,
    stale_secs: u64,
    path: &str,
    query: &str,
) -> Value {
    let id = path
        .strip_prefix("/api/v1/sessions/")
        .and_then(|rest| rest.strip_suffix("/trace"))
        .unwrap_or("")
        .trim();
    if id.is_empty() || id.contains('/') {
        return json!({
            "error": "expected /api/v1/sessions/{id}/trace",
            "found": false,
            "lines": [],
        });
    }
    let limit = query_param(query, "limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(harbard_core::harness_trace::TRACE_LIMIT_DEFAULT);
    let now = clock::now_ms();
    let mut view = query::sessions(store, events, now, stale_secs)
        .into_iter()
        .find(|v| v.session == id);
    // A session silent for longer than the hub window is only in the store.
    if view.is_none() && horizon_ms > 0 {
        let read = store.read(Window::All);
        view = query::sessions(store, &read.events, now, stale_secs)
            .into_iter()
            .find(|v| v.session == id);
    }
    let Some(view) = view else {
        return json!({
            "session": id,
            "harness": "",
            "found": false,
            "limit": limit,
            "lines": [],
        });
    };
    let roots = harbard_core::harness_trace::TraceRoots::from_env();
    let tr = harbard_core::harness_trace::load_trace(&view, limit, &roots);
    serde_json::to_value(&tr).unwrap_or_else(|_| json!({ "found": false, "lines": [] }))
}

/// `GET /api/v1/search?q=` — local FTS5 over note text (cold path; not poll_loop).
fn search_body(store: &Store, query: &str) -> Result<Value, (u16, Value)> {
    let q = query_param(query, "q").unwrap_or_default();
    if q.trim().len() < 2 {
        return Err((
            400,
            json!({ "error": "empty_query", "hint": "q needs at least one 2+ char token" }),
        ));
    }
    let limit = query_param(query, "limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(harbard_core::fts_default_limit())
        .min(64);
    match harbard_core::fts::search(store, &q, limit) {
        Ok(hits) => Ok(json!({
            "query": q,
            "hits": hits,
            "returned": hits.len(),
            "limit": limit,
            "now_ms": clock::now_ms(),
        })),
        Err(harbard_core::fts::FtsError::EmptyQuery) => Err((
            400,
            json!({ "error": "empty_query", "hint": "q needs at least one 2+ char token" }),
        )),
        Err(e) => Err((
            500,
            json!({ "error": "fts_failed", "message": e.to_string() }),
        )),
    }
}

/// `GET /api/v1/history` — snapshot-stable archive page (newest-first).
/// Cold path only; never used by live `useBus` / SSE.
fn history_body(store: &Store, query: &str) -> Result<Value, (u16, Value)> {
    let limit = query_param(query, "limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(EVENTS_MAX)
        .min(EVENTS_MAX);
    let before = query_param(query, "before");
    let snapshot = query_param(query, "snapshot");
    let mut filter = harbard_core::HistoryFilter::default();
    if let Some(h) = query_param(query, "harness") {
        filter.harness = Some(h);
    }
    if let Some(s) = query_param(query, "session") {
        filter.session = Some(s);
    }
    if let Some(c) = query_param(query, "channel") {
        filter.channel = Some(harbard_core::membership::normalize(&c));
    }
    if let Some(s) = query_param(query, "to_session") {
        filter.to_session = Some(s);
    }
    if let Some(h) = query_param(query, "to_harness") {
        filter.to_harness = Some(h);
    }
    if let Some(kinds) = query_param(query, "kinds") {
        let parsed: Vec<_> = kinds
            .split(',')
            .filter_map(|k| match k.trim() {
                "hello" => Some(EventKind::Hello),
                "intent" => Some(EventKind::Intent),
                "note" => Some(EventKind::Note),
                "bye" => Some(EventKind::Bye),
                "join" => Some(EventKind::Join),
                "leave" => Some(EventKind::Leave),
                "ack" => Some(EventKind::Ack),
                _ => None,
            })
            .collect();
        if !parsed.is_empty() {
            filter.kinds = Some(parsed);
        }
    }
    match store.read_history_page(limit, before.as_deref(), snapshot.as_deref(), &filter) {
        Ok(page) => Ok(json!({
            "events": page.events,
            "oldest_id": page.oldest_id,
            "newest_id": page.newest_id,
            "has_more": page.has_more,
            "returned": page.returned,
            "snapshot_newest_id": page.snapshot_newest_id,
            "retention_days": page.retention_days,
            "window": "all",
        })),
        Err(harbard_core::HistoryError::CursorExpired { kind, id }) => Err((
            409,
            json!({
                "error": "cursor_expired",
                "kind": kind,
                "id": id,
            }),
        )),
    }
}

/// Sessions over surviving segments (History → Sessions tab). Cold path.
fn history_sessions_body(store: &Store, stale_secs: u64) -> Value {
    let now = clock::now_ms();
    let read = store.read(Window::All);
    let mut views = query::sessions(store, &read.events, now, stale_secs);
    for v in &mut views {
        if v.liveness == Liveness::Ended {
            v.liveness = Liveness::Stale;
        }
    }
    views.sort_by_key(|v| std::cmp::Reverse(v.last_ms));
    json!({
        "sessions": views,
        "now_ms": now,
        "window": "all",
        "retention_days": harbard_core::DEFAULT_RETENTION_DAYS,
    })
}

/// `GET /api/v1/events` body: file-ordered window with `after`/`limit` and the
/// total/returned/newest_id disclosure.
#[cfg(test)]
fn events_body(store: &Store, query: &str) -> Value {
    let read = store.read(Window::Recent);
    events_body_from(&read.events, query)
}

fn events_body_from(all: &[Event], query: &str) -> Value {
    let newest_id = all.last().map(|e| e.id.clone());

    let after = query_param(query, "after");
    let limit = query_param(query, "limit")
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(EVENTS_MAX)
        .min(EVENTS_MAX);

    let start = match after.as_deref() {
        Some(id) => all
            .iter()
            .position(|e| e.id == id)
            .map(|p| p + 1)
            .unwrap_or(0),
        None => 0,
    };
    let candidate = &all[start.min(all.len())..];
    let total = candidate.len();
    let returned = total.min(limit);
    // The newest `returned` events (tail of the file-ordered window), so a feed
    // shows the latest and pagination discloses what was withheld.
    let slice = &candidate[total - returned..];

    json!({
        "events": slice,
        "total": total,
        "returned": returned,
        "newest_id": newest_id,
    })
}

/// One coherent live projection for both the RSC first paint and browser
/// revalidation. Both slices are borrowed under the hub lock, which is released
/// before network IO; retained history never gets cloned for each browser tab.
fn snapshot_body(
    store: &Store,
    events: &[Event],
    recent_events: &[Event],
    stale_secs: u64,
) -> Value {
    let now = clock::now_ms();
    let sessions = sessions_body_from(store, events, stale_secs);
    let event_window = events_body_from(recent_events, "limit=500");
    let membership = harbard_core::membership::read(store);
    let aliases = harbard_core::alias::read(store);
    let epithets: serde_json::Map<String, Value> = aliases
        .by_session
        .keys()
        .map(|session| {
            (
                session.clone(),
                Value::String(harbard_core::alias::epithet(session)),
            )
        })
        .collect();
    let leases = LeaseStore::at(store.root()).active_writers(SystemLeaseClock, &SystemPidChecker);

    json!({
        "sessions": sessions.get("sessions").cloned().unwrap_or_else(|| json!([])),
        "events": event_window.get("events").cloned().unwrap_or_else(|| json!([])),
        "total": event_window.get("total").cloned().unwrap_or_else(|| json!(0)),
        "returned": event_window.get("returned").cloned().unwrap_or_else(|| json!(0)),
        "newest_id": event_window.get("newest_id").cloned().unwrap_or(Value::Null),
        "channels": membership.channels,
        "archived": membership.archived,
        "channel_meta": membership.meta,
        "channel_reads": membership.reads,
        "delivery": harbard_core::cursor::delivery(store),
        "by_session": aliases.by_session,
        "epithets": epithets,
        "leases": leases,
        "leases_known": true,
        "now_ms": now,
    })
}

/// Liveness by default: answers from atomics and the clock, so a watchdog's
/// short timeout measures whether the server accepts work, not how long the
/// diagnostics below take. `?full=1` adds the process, stream and delivery
/// figures, which spawn `ps` and summarize a week of learning records.
fn health_body(store: &Store, cfg: &Config, hub: &EventHub, full: bool) -> Value {
    let pid = std::process::id();
    if !full {
        return json!({
            "ok": true,
            "pid": pid,
            "started_ms": cfg.started_ms,
            "uptime_ms": clock::now_ms().saturating_sub(cfg.started_ms),
            "stream": hub.liveness(),
            "now_ms": clock::now_ms(),
        });
    }
    let fd_count = std::fs::read_dir("/dev/fd")
        .map(|entries| entries.count() as u64)
        .ok();
    let pid_arg = pid.to_string();
    let rss_kib = command_stdout("ps", &["-o", "rss=", "-p", &pid_arg])
        .and_then(|stdout| stdout.trim().parse::<u64>().ok());
    // macOS `ps` has no `thcount`/`nlwp` output keyword. `ps -M` prints one
    // row per Mach thread after a single header, so count those rows instead.
    // Keep this independent from RSS: one unsupported metric must not erase
    // the other from the health response.
    let threads = command_stdout("ps", &["-M", "-p", &pid_arg])
        .and_then(|stdout| {
            stdout
                .lines()
                .next()
                .map(|_| stdout.lines().skip(1).count() as u64)
        })
        .filter(|count| *count > 0);
    let process = json!({ "rss_kib": rss_kib, "threads": threads });
    let since_ms = clock::now_ms().saturating_sub(7 * 24 * 60 * 60 * 1000);
    let learning = harbard_core::learning::summarize(store, since_ms);
    let now = clock::now_ms();
    let (_, recent_events) = hub.history();
    let acked: HashSet<_> = recent_events
        .iter()
        .filter(|event| event.event == EventKind::Ack)
        .filter_map(|event| event.note_ref.as_deref())
        .collect();
    let old_handoffs = recent_events
        .iter()
        .filter(|event| {
            event.event == EventKind::Note
                && event.kind == NoteKind::Handoff
                && !acked.contains(event.id.as_str())
                && event_timestamp_ms(event) <= now.saturating_sub(60 * 60 * 1000)
        })
        .count();
    json!({
        "ok": true,
        "pid": pid,
        "started_ms": cfg.started_ms,
        "uptime_ms": clock::now_ms().saturating_sub(cfg.started_ms),
        "fd_count": fd_count,
        "process": process,
        "stream": hub.metrics(),
        "delivery": {
            "unacked_handoffs_7d": learning.counters.unacked_handoffs,
            "unacked_handoffs_older_than_1h": old_handoffs,
            "retry_attempts_7d": learning.counters.retry_attempts,
        },
        "now_ms": clock::now_ms(),
    })
}

fn command_stdout(command: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(command).args(args).output().ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())?
}

/// Write a JSON response with the API content-type and CORS headers. When
/// `head` is set the body is omitted (HEAD probe).
fn respond_json(req: Request, status: u16, body: &Value, origin: Option<&str>, head: bool) {
    let payload = if head {
        String::new()
    } else {
        serde_json::to_string(body).unwrap_or_else(|_| "{}".to_string())
    };
    let mut headers = vec![header("Content-Type", "application/json; charset=utf-8")];
    push_cors(&mut headers, origin);
    let response = Response::new(
        StatusCode(status),
        headers,
        payload.as_bytes(),
        Some(payload.len()),
        None,
    );
    record_status(status);
    let _ = req.respond(response);
}

/// An attachment's bytes: its own content type (text is always plain text,
/// never HTML), never sniffed, sandboxed if a browser opens it directly, and
/// cacheable because an attachment folder is never rewritten.
fn respond_attachment(
    req: Request,
    bytes: &[u8],
    content_type: &str,
    origin: Option<&str>,
    head: bool,
) {
    let mut headers = vec![
        header("Content-Type", content_type),
        header("X-Content-Type-Options", "nosniff"),
        header("Content-Security-Policy", "default-src 'none'; sandbox"),
        header("Cache-Control", "private, max-age=86400, immutable"),
    ];
    push_cors(&mut headers, origin);
    let body: &[u8] = if head { &[] } else { bytes };
    let response = Response::new(StatusCode(200), headers, body, Some(body.len()), None);
    record_status(200);
    let _ = req.respond(response);
}

/// Bytes of a fixed type (the profile photo): never sniffed, never cached.
fn respond_bytes(req: Request, bytes: &[u8], kind: &str, origin: Option<&str>, head: bool) {
    let mut headers = vec![
        header("Content-Type", kind),
        header("Cache-Control", "no-store"),
        header("X-Content-Type-Options", "nosniff"),
    ];
    push_cors(&mut headers, origin);
    let body: &[u8] = if head { &[] } else { bytes };
    let response = Response::new(StatusCode(200), headers, body, Some(body.len()), None);
    record_status(200);
    let _ = req.respond(response);
}

/// Respond to a HEAD probe of the SSE stream with the stream content-type and
/// no body.
fn respond_stream_head(req: Request, origin: Option<&str>) {
    let mut headers = vec![header("Content-Type", "text/event-stream")];
    push_cors(&mut headers, origin);
    let response = Response::new(StatusCode(200), headers, io::empty(), Some(0), None);
    record_status(200);
    let _ = req.respond(response);
}

/// The credential an open stream was opened with, read again before each
/// frame (`pairing::verify` reads both files fresh).
struct StreamGuard {
    store: Store,
    token: String,
}

impl StreamGuard {
    fn still_operator(&self) -> bool {
        matches!(
            harbard_core::pairing::verify(&self.store, &self.token, clock::now_ms()),
            harbard_core::pairing::Verdict::Device(card)
                if card.access == harbard_core::Access::Operator
        )
    }
}

/// Stream Server-Sent Events by taking the raw connection writer: one
/// `event: bus` and one `event: item` per appended store event, a heartbeat
/// comment every 15 s.
///
/// tiny_http's `Response` chunk-buffers 8 KB before flushing, which never
/// streams a live fleet's small frames; `into_writer` hands us the socket so we
/// flush each frame ourselves. The body is delimited by connection close
/// (`Connection: close`), the standard SSE framing. Blocks the request thread
/// until the client disconnects (write error) or shutdown flips.
#[allow(clippy::too_many_arguments)]
fn stream_sse(
    req: Request,
    shutdown: Arc<AtomicBool>,
    origin: Option<String>,
    hub: Arc<EventHub>,
    after: Option<String>,
    store: Store,
    stale_secs: u64,
    guard: Option<StreamGuard>,
) {
    let Some(subscription) = hub.subscribe(after.as_deref()) else {
        respond_json(
            req,
            503,
            &json!({ "error": "too many live event streams; reconnect shortly" }),
            origin.as_deref(),
            false,
        );
        return;
    };
    record_status(200);
    let mut w = req.into_writer();

    let mut head = String::from("HTTP/1.1 200 OK\r\n");
    head.push_str("Content-Type: text/event-stream\r\n");
    head.push_str("Cache-Control: no-cache\r\n");
    head.push_str("Connection: close\r\n");
    head.push_str("X-Accel-Buffering: no\r\n"); // defeat proxy buffering
    if let Some(o) = origin.as_deref().filter(|o| is_localhost_origin(o)) {
        head.push_str(&format!("Access-Control-Allow-Origin: {o}\r\n"));
        head.push_str("Access-Control-Allow-Methods: GET, HEAD\r\n");
        head.push_str("Vary: Origin\r\n");
    }
    head.push_str("\r\n");
    if w.write_all(head.as_bytes()).is_err() || w.flush().is_err() {
        return;
    }

    // A client that asked to resume from an id the window has dropped is told
    // so before anything else: it refetches what it shows, and the gap is
    // never silently swallowed.
    if !subscription.resumed && !write_reset(&mut w, "cursor_expired") {
        return;
    }

    let opened = Instant::now();
    let mut projection = ItemFrames::new(store, stale_secs);
    for event in &subscription.replay {
        if !write_sse_event(&mut w, event) || !projection.write_item(&mut w, &hub, event) {
            return;
        }
    }
    projection.end_tick();

    // Checked before every frame: nothing more reaches a device the operator
    // revoked or demoted, even on a stream it opened before.
    let cut = |w: &mut dyn Write| -> bool {
        match guard.as_ref() {
            Some(g) if !g.still_operator() => {
                let _ = write_reset(w, "revoked");
                true
            }
            _ => false,
        }
    };
    while !shutdown.load(Ordering::Relaxed) {
        if opened.elapsed() >= SSE_MAX_AGE {
            // Say why before closing, so the client reconnects deliberately
            // rather than discovering a dead socket.
            let _ = write_reset(&mut w, "max_age");
            return;
        }
        let next = subscription.receiver.recv_timeout(STREAM_TICK);
        if cut(&mut w) {
            return;
        }
        match next {
            Ok(event) => {
                if !write_sse_event(&mut w, &event) || !projection.write_item(&mut w, &hub, &event)
                {
                    return;
                }
                // Debounce: a burst of appends shares one feed context.
                let deadline = Instant::now() + STREAM_DEBOUNCE;
                while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
                    match subscription.receiver.recv_timeout(remaining) {
                        Ok(event) => {
                            if cut(&mut w) {
                                return;
                            }
                            if !write_sse_event(&mut w, &event)
                                || !projection.write_item(&mut w, &hub, &event)
                            {
                                return;
                            }
                        }
                        Err(RecvTimeoutError::Timeout) => break,
                        Err(RecvTimeoutError::Disconnected) => {
                            close_stream(&mut w, &subscription);
                            return;
                        }
                    }
                }
                projection.end_tick();
                if cut(&mut w) {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {
                if projection.since_heartbeat() >= HEARTBEAT {
                    if w.write_all(b": heartbeat\n\n").is_err() || w.flush().is_err() {
                        return;
                    }
                    projection.beat();
                }
            }
            Err(RecvTimeoutError::Disconnected) => {
                close_stream(&mut w, &subscription);
                return;
            }
        }
    }
}

/// Last words on a closing stream: a client dropped for falling behind is told
/// so (§3), rather than discovering a dead socket and resuming from now.
fn close_stream(w: &mut dyn Write, subscription: &Subscription) {
    if subscription.evicted.load(Ordering::Acquire) {
        let _ = write_reset(w, "slow_client");
    }
}

/// The `item` frames' state for one connection, and when it last spoke.
struct ItemFrames {
    store: Store,
    stale_secs: u64,
    last_beat: Instant,
    /// The feed context for the tick being written, shared by its `item` frames.
    tick_context: Option<harbard_core::feed::FeedContext>,
}

impl ItemFrames {
    fn new(store: Store, stale_secs: u64) -> Self {
        ItemFrames {
            store,
            stale_secs,
            last_beat: Instant::now(),
            tick_context: None,
        }
    }

    fn since_heartbeat(&self) -> Duration {
        self.last_beat.elapsed()
    }

    /// Something went out: the next heartbeat is due a full interval later.
    fn beat(&mut self) {
        self.last_beat = Instant::now();
    }

    /// One `item` frame per accepted event, on the same tick as `bus`.
    ///
    /// The envelope is computed against the hub's whole window, never the one
    /// event that triggered it: `places` needs the sibling worktrees an event
    /// reaches, `thread.replies` needs the rest of the thread, `obligation.open`
    /// needs the acks, and the author's presence needs their other signals. A
    /// one-event slice answered "0 replies, still open, ended author" every time.
    ///
    /// The context is built once per tick and reused across the events of that
    /// tick, so a burst costs one projection rather than one per event.
    fn write_item(&mut self, w: &mut dyn Write, hub: &EventHub, event: &Event) -> bool {
        let context = self.tick_context.get_or_insert_with(|| {
            fleet_routes::context_for(&self.store, &hub.history().0, self.stale_secs)
        });
        let Ok(js) = serde_json::to_string(&fleet_routes::item_value(context, event)) else {
            return true;
        };
        self.beat();
        write_frame(w, &event.id, "item", &js)
    }

    /// Drop the tick's cached context, so the next burst re-reads the hub.
    fn end_tick(&mut self) {
        self.tick_context = None;
    }
}

fn write_reset(w: &mut dyn Write, reason: &str) -> bool {
    let frame = format!("event: reset\ndata: {{\"reason\":\"{reason}\"}}\n\n");
    w.write_all(frame.as_bytes()).is_ok() && w.flush().is_ok()
}

fn write_frame(w: &mut dyn Write, id: &str, event: &str, data: &str) -> bool {
    let frame = if id.is_empty() {
        format!("event: {event}\ndata: {data}\n\n")
    } else {
        format!("id: {id}\nevent: {event}\ndata: {data}\n\n")
    };
    w.write_all(frame.as_bytes()).is_ok() && w.flush().is_ok()
}

fn write_sse_event(w: &mut dyn Write, event: &Event) -> bool {
    let Ok(js) = serde_json::to_string(event) else {
        return true;
    };
    write_frame(w, &event.id, "bus", &js)
}

/// Build a static header; the inputs are constants, so this never fails.
fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("valid static header")
}

/// Append CORS headers when the request's Origin is a localhost dev server.
fn push_cors(headers: &mut Vec<Header>, origin: Option<&str>) {
    if let Some(o) = origin.filter(|o| is_localhost_origin(o)) {
        if let Ok(h) = Header::from_bytes(&b"Access-Control-Allow-Origin"[..], o.as_bytes()) {
            headers.push(h);
        }
        headers.push(header("Access-Control-Allow-Methods", "GET, HEAD, POST"));
        headers.push(header("Vary", "Origin"));
    }
}

/// The bearer the request presented: `Authorization: Bearer …`, else
/// `X-Harbard-Token`.
fn presented_token(req: &Request) -> Option<String> {
    header_value(req, "authorization")
        .and_then(|auth| {
            auth.strip_prefix("Bearer ")
                .or_else(|| auth.strip_prefix("bearer "))
                .map(|t| t.trim().to_string())
        })
        .filter(|t| !t.is_empty())
        .or_else(|| header_value(req, "x-harbard-token").map(|t| t.trim().to_string()))
        .filter(|t| !t.is_empty())
}

/// Last seen, at most every [`TOUCH_EVERY_MS`] per device: a display polls
/// every few seconds and the registry need not be rewritten each time.
const TOUCH_EVERY_MS: u64 = 30_000;

fn touch_device(store: &Store, id: &str, now: u64) {
    static LAST: std::sync::OnceLock<Mutex<HashMap<String, u64>>> = std::sync::OnceLock::new();
    let mut last = LAST
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    if last
        .get(id)
        .is_some_and(|at| now.saturating_sub(*at) < TOUCH_EVERY_MS)
    {
        return;
    }
    last.insert(id.to_string(), now);
    drop(last);
    let _ = harbard_core::pairing::touch(store, id, now);
}

/// What a Workshop device reads on `/events`: its own sessions, and notes
/// addressed to them from a workspace it is granted (or from the operator).
fn workshop_sees(
    card: &DeviceCard,
    event: &Event,
    placement: &harbard_core::workspaces::Placement,
    granted: &[String],
) -> bool {
    card.owns_session(&event.session)
        || (event
            .to
            .as_ref()
            .is_some_and(|to| to.kind == RecipientKind::Session && card.owns_session(&to.value))
            && (placement.is_single()
                || event.is_operator()
                || granted.contains(&placement.of_event(event))))
}

/// Whether an attachment folder was cited, by someone on the hub, in a note
/// addressed to one of this device's sessions. A note a device wrote never
/// counts, its own or another device's: serve stamps every device event with
/// that device's host, which is never the hub's ([`harbard_core::stamped_host`]),
/// so the requester can never unlock a folder by citing it itself.
fn addressed_attachment(card: &DeviceCard, events: &[Event], rest: &str, hub_host: &str) -> bool {
    let Some(dir) = rest.split('/').next().filter(|d| !d.is_empty()) else {
        return false;
    };
    let cited = format!("attachments/{dir}/");
    events.iter().any(|e| {
        e.event == EventKind::Note
            && e.harness != HARNESS_DEVICE
            && e.host.eq_ignore_ascii_case(hub_host)
            && !card.owns_session(&e.session)
            && e.to
                .as_ref()
                .is_some_and(|to| to.kind == RecipientKind::Session && card.owns_session(&to.value))
            && e.text.contains(&cited)
    })
}

/// `HARBARD_ALLOWED_HOSTS=harbard.localhost,foo.local`: names serve answers
/// besides loopback and its bind address, read once. Empty by default: the
/// named dev host is an explicit opt-in, the same one the console reads.
fn allowed_hosts() -> &'static [String] {
    static HOSTS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    HOSTS
        .get_or_init(|| parse_allowed_hosts(std::env::var("HARBARD_ALLOWED_HOSTS").ok().as_deref()))
}

fn parse_allowed_hosts(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or("")
        .split(',')
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| !h.is_empty())
        .collect()
}

/// The Host header's name without its port: `127.0.0.1:7419` → `127.0.0.1`,
/// `[::1]:7419` → `::1`.
fn host_name(host: &str) -> &str {
    let host = host.trim();
    if let Some(rest) = host.strip_prefix('[') {
        return rest.split(']').next().unwrap_or(rest);
    }
    host.rsplit_once(':')
        .filter(|(_, port)| port.bytes().all(|b| b.is_ascii_digit()))
        .map_or(host, |(name, _)| name)
}

/// Whether serve answers a request with this Host: loopback (`127.x`,
/// `localhost`, `::1`, `*.localhost`), the address it bound, or a private
/// IPv4 address when it is bound beyond loopback (a hub its satellites and
/// paired consoles reach by LAN address). Nothing else: no public name, no
/// missing Host.
fn host_allowed(host: Option<&str>, bind: &str, extra: &[String]) -> bool {
    let Some(name) = host.map(host_name).filter(|n| !n.is_empty()) else {
        return false;
    };
    let name = name.to_ascii_lowercase();
    // The named dev host, only when opted in (`HARBARD_ALLOWED_HOSTS`).
    if extra.contains(&name) {
        return true;
    }
    let loopback = name == "localhost"
        || name.ends_with(".localhost")
        || name == "::1"
        || name.split('.').next() == Some("127")
            && name.split('.').count() == 4
            && name.split('.').all(|p| p.parse::<u8>().is_ok());
    if loopback || name == bind.to_ascii_lowercase() {
        return true;
    }
    let bound_beyond_loopback = !matches!(bind, "127.0.0.1" | "localhost" | "::1" | "");
    bound_beyond_loopback && is_private_ipv4(&name)
}

fn is_allowed_origin(origin: &str, hub_mode: bool) -> bool {
    if is_localhost_origin(origin) {
        return true;
    }
    if !hub_mode {
        return false;
    }
    // Paired LAN consoles: private IPv4 literals only.
    let Some(rest) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    else {
        return false;
    };
    let host = rest.split(':').next().unwrap_or(rest);
    is_private_ipv4(host)
}

fn is_private_ipv4(host: &str) -> bool {
    let Ok(parts) = host
        .split('.')
        .map(|p| p.parse::<u8>())
        .collect::<Result<Vec<_>, _>>()
    else {
        return false;
    };
    if parts.len() != 4 {
        return false;
    }
    matches!(parts[0], 10)
        || (parts[0] == 192 && parts[1] == 168)
        || (parts[0] == 172 && (16..=31).contains(&parts[1]))
}

fn post_device(
    mut req: Request,
    store: &Store,
    caller: &access::Caller,
    token: Option<&str>,
    origin: Option<&str>,
) {
    let raw = match read_body(&mut req, origin) {
        Ok(r) => r,
        Err(()) => return,
    };
    let parsed: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            respond_json(
                req,
                400,
                &json!({ "error": format!("invalid JSON: {e}") }),
                origin,
                false,
            );
            return;
        }
    };
    let card: DeviceCard = match serde_json::from_value(parsed) {
        Ok(c) => c,
        Err(e) => {
            respond_json(
                req,
                400,
                &json!({ "error": format!("invalid device card: {e}") }),
                origin,
                false,
            );
            return;
        }
    };
    if !harbard_core::pairing::valid_device_id(&card.id) {
        respond_json(
            req,
            400,
            &json!({ "error": "device id required: letters, digits, - or _, at most 64" }),
            origin,
            false,
        );
        return;
    }
    // Only what a device may say about itself is read from the body: role,
    // pairing time, revocation and published sessions are the hub's.
    let mut card = card.declared();
    let now = clock::now_ms();
    // Pairing: an invite pairs the device that claims it; a paired device
    // posts only its own card; the hub's own key may still register one by
    // hand. What the hub decides (role, revocation) never comes from the body.
    let saved = match caller {
        access::Caller::Invite(_) => {
            let hash = harbard_core::pairing::token_hash(token.unwrap_or_default());
            match harbard_core::pairing::claim(store, &hash, card, now) {
                Ok(Ok(v)) => {
                    // The pairing itself is the first line of its log.
                    set_caller_device(&v.0.id);
                    Ok(v)
                }
                Ok(Err(e)) => {
                    let (status, message) = match e {
                        harbard_core::pairing::ClaimError::IdTaken => (
                            409,
                            "another device holds this id; revoke it first: harbard hub revoke <device>",
                        ),
                        _ => (403, "this invite cannot pair this device"),
                    };
                    let error = fleet_routes::ApiError::new(status, e.code(), message);
                    respond_json(req, error.status, &error.body(), origin, false);
                    return;
                }
                Err(e) => Err(e),
            }
        }
        access::Caller::Device(own) => {
            if own.id != card.id {
                let error = fleet_routes::ApiError::new(
                    403,
                    "wrong_device",
                    "a device posts only its own card",
                );
                respond_json(req, error.status, &error.body(), origin, false);
                return;
            }
            harbard_core::pairing::heartbeat(store, card, now).map(|c| (c, false))
        }
        access::Caller::Local => harbard_core::pairing::update(store, |reg, _| {
            let existing_hub = reg.get(&card.id).is_some_and(|c| c.role == DeviceRole::Hub);
            if existing_hub {
                card.role = DeviceRole::Hub;
            }
            reg.upsert(card, now)
        }),
    };
    let (card, is_new) = match saved {
        Ok(v) => v,
        Err(e) => {
            respond_json(req, 500, &json!({ "error": e.to_string() }), origin, false);
            return;
        }
    };
    if is_new {
        let join = Event {
            v: SCHEMA_VERSION,
            id: Ulid::generate().to_string(),
            ts: clock::rfc3339_utc(now),
            event: EventKind::DeviceJoin,
            session: format!("device:{}", card.id),
            harness: HARNESS_DEVICE.to_string(),
            host: card.host.clone(),
            cwd: String::new(),
            repo: None,
            repo_key: None,
            branch: None,
            role: Role::Main,
            parent: None,
            text: card_summary(&card, "joined"),
            to: None,
            kind: NoteKind::Fyi,
            note_ref: None,
        };
        let _ = store.append(&join);
    }
    respond_json(
        req,
        if is_new { 201 } else { 200 },
        &json!({ "device": card, "joined": is_new }),
        origin,
        false,
    );
}

fn post_satellite_event(
    mut req: Request,
    store: &Store,
    caller: &access::Caller,
    event_hub: &EventHub,
    origin: Option<&str>,
) {
    let device_id = caller
        .device_id()
        .map(str::to_string)
        .or_else(|| header_value(&req, "x-harbard-device"))
        .unwrap_or_default();
    if device_id.is_empty() {
        respond_json(
            req,
            400,
            &json!({ "error": "X-Harbard-Device header required" }),
            origin,
            false,
        );
        return;
    }
    let reg = DeviceRegistry::load(store);
    let Some(card) = reg.get(&device_id).cloned() else {
        respond_json(
            req,
            403,
            &json!({ "error": "device not registered; POST /api/v1/devices first" }),
            origin,
            false,
        );
        return;
    };

    let raw = match read_body(&mut req, origin) {
        Ok(r) => r,
        Err(()) => return,
    };
    let mut event: Event = match serde_json::from_str(&raw) {
        Ok(e) => e,
        Err(e) => {
            respond_json(
                req,
                400,
                &json!({ "error": format!("invalid event: {e}") }),
                origin,
                false,
            );
            return;
        }
    };

    // Mitigations from adversarial pass: stamp host from registry; refuse
    // forging the hub operator session or another device's device_* events.
    if event.session.starts_with("operator@") || event.role == Role::Operator {
        respond_json(
            req,
            403,
            &json!({ "error": "satellites cannot post as the operator; use POST /notes on the hub" }),
            origin,
            false,
        );
        return;
    }
    // `harness: "device"` is a device's own presence, nothing more: for a
    // paired device, only presence kinds on exactly `device:<its id>`.
    let presence = matches!(
        event.event,
        EventKind::Hello | EventKind::Bye | EventKind::DeviceJoin | EventKind::DeviceLeave
    );
    let forged_card = if caller.device().is_some() {
        event.harness == HARNESS_DEVICE
            && (!presence || event.session != format!("device:{}", card.id))
    } else {
        event.harness == HARNESS_DEVICE && !event.session.ends_with(&card.id)
    };
    if forged_card {
        respond_json(
            req,
            403,
            &json!({ "error": "cannot forge another device card" }),
            origin,
            false,
        );
        return;
    }
    // Pairing: a device speaks only as the sessions it published, and a
    // session becomes its own only through its own `hello`, recorded here.
    // A session the hub already knows from elsewhere (the Mac's own, or
    // another device's) is nobody's to take.
    if caller.device().is_some() && event.harness != HARNESS_DEVICE {
        let mine = if event.event == EventKind::Hello {
            let seen_elsewhere = !card.owns_session(&event.session)
                && event_hub
                    .history()
                    .0
                    .iter()
                    .any(|e| e.session == event.session);
            harbard_core::pairing::claim_session(store, &card.id, &event.session, seen_elsewhere)
        } else {
            Ok(card.owns_session(&event.session))
        };
        match mine {
            Ok(true) => {}
            Ok(false) => {
                let error = fleet_routes::ApiError::new(
                    403,
                    "not_your_session",
                    "a device speaks only as the sessions its own hello published",
                );
                respond_json(req, error.status, &error.body(), origin, false);
                return;
            }
            Err(e) => {
                respond_json(req, 500, &json!({ "error": e.to_string() }), origin, false);
                return;
            }
        }
    }
    // Stamped by the hub; a paired device never carries the hub's own host.
    event.host = if caller.device().is_some() {
        harbard_core::stamped_host(&card.host, &harbard_core::hostname(), &card.id)
    } else {
        card.host.clone()
    };
    event.v = SCHEMA_VERSION;
    if event.id.trim().is_empty() {
        event.id = Ulid::generate().to_string();
    }
    if let Err(e) = event.to_line() {
        respond_json(req, 422, &json!({ "error": e.to_string() }), origin, false);
        return;
    }
    if let Err(e) = store.append(&event) {
        respond_json(
            req,
            500,
            &json!({ "error": format!("append failed: {e}") }),
            origin,
            false,
        );
        return;
    }

    // Touch heartbeat.
    let _ = harbard_core::pairing::touch(store, &card.id, clock::now_ms());

    let body = json!({ "event": event });
    respond_json(req, 201, &body, origin, false);
}

/// Wire selected harness hooks on this machine. Loopback-only: writing
/// `~/.claude` / `~/.codex` / `~/.cursor` must happen on the machine that owns
/// those homes, never via a LAN peer talking to a hub.
fn post_setup_install(mut req: Request, store: &Store, cfg: &Config, origin: Option<&str>) {
    if !is_loopback_peer(&req) {
        respond_json(
            req,
            403,
            &json!({
                "error": "setup install is loopback-only; run harbard setup on this machine, or open the console against local serve"
            }),
            origin,
            false,
        );
        return;
    }
    if let Some(o) = origin
        && !is_allowed_origin(o, cfg.requires_auth())
    {
        respond_json(
            req,
            403,
            &json!({ "error": "forbidden origin" }),
            None,
            false,
        );
        return;
    }
    if !device_hdr_empty(&req) {
        respond_json(
            req,
            403,
            &json!({ "error": "satellites cannot run setup install; use harbard setup locally" }),
            origin,
            false,
        );
        return;
    }

    let Ok(raw) = read_body(&mut req, origin) else {
        respond_json(
            req,
            413,
            &json!({ "error": "body too large or unreadable" }),
            origin,
            false,
        );
        return;
    };
    let parsed: Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => {
            respond_json(
                req,
                400,
                &json!({ "error": format!("invalid JSON body: {e}") }),
                origin,
                false,
            );
            return;
        }
    };
    let Some(targets) = parsed.get("targets").and_then(|v| v.as_array()) else {
        respond_json(
            req,
            400,
            &json!({ "error": "field `targets` is required (array of claude|codex|cursor)" }),
            origin,
            false,
        );
        return;
    };
    if targets.is_empty() {
        respond_json(
            req,
            400,
            &json!({ "error": "field `targets` must not be empty" }),
            origin,
            false,
        );
        return;
    }

    let mut results = Vec::new();
    let mut wired = Vec::new();
    for t in targets {
        let Some(target) = t.as_str() else {
            results.push(json!({ "target": t, "ok": false, "error": "target must be a string" }));
            continue;
        };
        if !matches!(target, "claude" | "codex" | "cursor") {
            results.push(json!({
                "target": target,
                "ok": false,
                "error": "unknown target; expected claude|codex|cursor",
            }));
            continue;
        }
        match crate::setup::wire_target(target, false) {
            Ok(result) => {
                let ok = result
                    .get("ok")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                let present = result
                    .get("present")
                    .and_then(|value| value.as_bool())
                    .unwrap_or(false);
                if ok && present {
                    wired.push(target.to_string());
                }
                results.push(result);
            }
            Err(e) => {
                let path = crate::install::default_config_path(target);
                results.push(json!({
                    "target": target,
                    "present": crate::install::target_present(target),
                    "ok": false,
                    "action": "error",
                    "error": e.to_string(),
                    "path": path.display().to_string(),
                }));
            }
        }
    }

    // Refresh setup state wired list without completing (operator may continue).
    if !wired.is_empty() {
        let mut state = crate::setup::read(store);
        for t in &wired {
            if !state.wired.iter().any(|w| w == t) {
                state.wired.push(t.clone());
            }
        }
        let _ = crate::setup::write(store, &state);
    }

    let all_ok = results
        .iter()
        .all(|r| r.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
    respond_json(
        req,
        if all_ok { 200 } else { 207 },
        &json!({
            "ok": all_ok,
            "results": results,
            "setup": crate::setup::status(store),
        }),
        origin,
        false,
    );
}

/// Mark first-run Setup finished (wired and/or skipped).
fn post_setup_complete(mut req: Request, store: &Store, cfg: &Config, origin: Option<&str>) {
    if !is_loopback_peer(&req) {
        respond_json(
            req,
            403,
            &json!({ "error": "setup complete is loopback-only" }),
            origin,
            false,
        );
        return;
    }
    if let Some(o) = origin
        && !is_allowed_origin(o, cfg.requires_auth())
    {
        respond_json(
            req,
            403,
            &json!({ "error": "forbidden origin" }),
            None,
            false,
        );
        return;
    }
    if !device_hdr_empty(&req) {
        respond_json(
            req,
            403,
            &json!({ "error": "satellites cannot complete host setup" }),
            origin,
            false,
        );
        return;
    }

    let raw = match read_body(&mut req, origin) {
        Ok(s) => s,
        Err(()) => {
            respond_json(
                req,
                413,
                &json!({ "error": "body too large or unreadable" }),
                origin,
                false,
            );
            return;
        }
    };
    let parsed: Value = if raw.trim().is_empty() {
        json!({})
    } else {
        match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                respond_json(
                    req,
                    400,
                    &json!({ "error": format!("invalid JSON body: {e}") }),
                    origin,
                    false,
                );
                return;
            }
        }
    };

    let mut state = crate::setup::read(store);
    state.completed_ms = Some(clock::now_ms());
    if let Some(arr) = parsed.get("skipped").and_then(|v| v.as_array()) {
        state.skipped = arr
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .filter(|s| matches!(s.as_str(), "claude" | "codex" | "cursor"))
            .collect();
    }
    if let Some(arr) = parsed.get("wired").and_then(|v| v.as_array()) {
        for t in arr.iter().filter_map(|v| v.as_str()) {
            if matches!(t, "claude" | "codex" | "cursor") && !state.wired.iter().any(|w| w == t) {
                state.wired.push(t.to_string());
            }
        }
    }

    if let Err(e) = crate::setup::write(store, &state) {
        respond_json(
            req,
            500,
            &json!({ "error": format!("could not write setup.json: {e}") }),
            origin,
            false,
        );
        return;
    }

    respond_json(
        req,
        200,
        &json!({ "ok": true, "setup": crate::setup::status(store) }),
        origin,
        false,
    );
}

fn device_hdr_empty(req: &Request) -> bool {
    header_value(req, "x-harbard-device")
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
}

fn is_loopback_peer(req: &Request) -> bool {
    match req.remote_addr() {
        Some(addr) => addr.ip().is_loopback(),
        // Fail closed for mutators that rewrite local harness configs.
        None => false,
    }
}

fn read_body(req: &mut Request, _origin: Option<&str>) -> Result<String, ()> {
    let declared = req.body_length().unwrap_or(0);
    if declared > MAX_POST_BYTES {
        return Err(());
    }
    let mut raw = String::new();
    if req
        .as_reader()
        .take(MAX_POST_BYTES as u64)
        .read_to_string(&mut raw)
        .is_err()
    {
        return Err(());
    }
    Ok(raw)
}

/// True for loopback browser origins — and for portless named URLs.
///
/// Allowed:
/// - `http://localhost[:port]` / `http://127.0.0.1[:port]` (classic)
/// - `http(s)://{name}.localhost[:port]` / `http(s)://{name}.local[:port]`
///   including worktree subdomains (`fix.harbard.localhost`)
///
/// CORS is the browser gate. Portless is
/// the named front door for the console, so its origin has to clear this
/// check or every `fetch` to the bus dies as a foreign origin.
fn is_localhost_origin(origin: &str) -> bool {
    let authority = if let Some(rest) = origin.strip_prefix("http://") {
        rest
    } else if let Some(rest) = origin.strip_prefix("https://") {
        // https only for named portless hosts — plain https://localhost was
        // never a console origin and stays rejected.
        rest
    } else {
        return false;
    };
    if authority.contains('/') {
        return false; // an Origin has no path
    }
    let (host, port_ok) = match authority.split_once(':') {
        Some((h, p)) => (h, !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit())),
        None => (authority, true),
    };
    if !port_ok || host.is_empty() || host.starts_with('.') || host.contains("..") {
        return false;
    }
    if matches!(host, "localhost" | "127.0.0.1") {
        return origin.starts_with("http://");
    }
    // Portless TLDs: `.localhost` (default) and `.local` (common proxy config).
    host.ends_with(".localhost") || host.ends_with(".local")
}

/// Split a request URL into `(path, query)`; query excludes the `?`.
fn split_url(url: &str) -> (&str, &str) {
    match url.split_once('?') {
        Some((p, q)) => (p, q),
        None => (url, ""),
    }
}

/// First value of `key` in a `a=b&c=d` query string, if present.
fn query_param(query: &str, key: &str) -> Option<String> {
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| v.to_string())
    })
}

/// Case-insensitive request header lookup.
fn header_value(req: &Request, field: &str) -> Option<String> {
    req.headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(field))
        .map(|h| h.value.as_str().to_string())
}

#[cfg(test)]
mod host_tests {
    use super::{host_allowed, host_name, parse_allowed_hosts, parse_tailnet_addr};

    #[test]
    fn the_tailnet_config_point_never_takes_a_wildcard_or_a_lan_address() {
        for refused in [
            "0.0.0.0",
            "::",
            "127.0.0.1",
            "192.168.1.20",
            "10.0.0.5",
            "100.63.255.255",
            "100.128.0.1",
            "8.8.8.8",
            "fd00::1",
            "nonsense",
        ] {
            assert_eq!(parse_tailnet_addr(refused), None, "{refused}");
        }
        for taken in [
            "100.64.0.1",
            "100.101.102.103",
            "100.127.255.254",
            "fd7a:115c:a1e0::1",
        ] {
            assert!(parse_tailnet_addr(taken).is_some(), "{taken}");
        }
    }

    #[test]
    fn the_bind_list_is_loopback_unless_a_tailnet_address_is_set() {
        let mut cfg = super::Config {
            port: 0,
            bind_host: "127.0.0.1".into(),
            token: Some("k".into()),
            stale_secs: 60,
            retention_days: 7,
            started_ms: 0,
            device_id: String::new(),
            tailnet_addr: None,
        };
        assert_eq!(cfg.binds(), vec!["127.0.0.1".to_string()]);
        cfg.tailnet_addr = parse_tailnet_addr("100.101.102.103");
        assert_eq!(
            cfg.binds(),
            vec!["127.0.0.1".to_string(), "100.101.102.103".to_string()]
        );
        assert!(!cfg.binds().iter().any(|b| b == "0.0.0.0" || b == "::"));
    }

    #[test]
    fn serve_answers_only_its_own_host_names() {
        assert_eq!(host_name("127.0.0.1:7419"), "127.0.0.1");
        assert_eq!(host_name("[::1]:7419"), "::1");
        assert_eq!(host_name("localhost"), "localhost");
        for h in [
            "127.0.0.1:7419",
            "localhost:7419",
            "[::1]:7419",
            "harbard.localhost:7419",
            "127.0.0.2",
        ] {
            assert!(host_allowed(Some(h), "127.0.0.1", &[]), "{h}");
        }
        for h in [
            "evil.example:7419",
            "127.0.0.1.evil.example",
            "192.168.1.20:7419",
            "",
            "localhost.evil.example",
        ] {
            assert!(!host_allowed(Some(h), "127.0.0.1", &[]), "{h}");
        }
        assert!(
            !host_allowed(None, "127.0.0.1", &[]),
            "a request with no Host"
        );
        // A hub: its own address and private addresses, still no names.
        assert!(host_allowed(Some("192.168.1.20:7419"), "0.0.0.0", &[]));
        assert!(host_allowed(Some("10.0.0.5:7419"), "10.0.0.5", &[]));
        assert!(!host_allowed(Some("evil.example:7419"), "0.0.0.0", &[]));
        assert!(!host_allowed(Some("8.8.8.8:7419"), "0.0.0.0", &[]));
        // The named dev host: off by default, exactly the opted-in names.
        assert!(!host_allowed(
            Some("harbard.localhost.evil.example"),
            "127.0.0.1",
            &[]
        ));
        let opted = parse_allowed_hosts(Some(" Foo.local , harbard.dev "));
        assert_eq!(
            opted,
            vec!["foo.local".to_string(), "harbard.dev".to_string()]
        );
        assert!(!host_allowed(Some("foo.local:7419"), "127.0.0.1", &[]));
        assert!(host_allowed(Some("foo.local:7419"), "127.0.0.1", &opted));
        assert!(!host_allowed(
            Some("foo.local.evil.example"),
            "127.0.0.1",
            &opted
        ));
        assert!(parse_allowed_hosts(None).is_empty());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Workshop device reads notes addressed to its sessions only when they
    /// were written in a workspace it is granted, or by the operator.
    #[test]
    fn a_workshop_device_never_reads_a_note_from_a_workspace_it_is_not_granted() {
        use harbard_core::workspaces::{Placement, RepoFacts, Workspaces};
        let mut set = Workspaces::single();
        set.workspaces.clear();
        set.add("personal", "Personal", &["me".into()]).unwrap();
        set.add("acme", "Acme", &["acme".into()]).unwrap();
        set.default = "personal".into();
        let placement = Placement::with(Some(set), |path: &str| RepoFacts {
            root: Some(path.to_string()),
            owner: path
                .strip_prefix("/w/")
                .map(|o| o.split('/').next().unwrap().to_string()),
        });
        let mut card = LocalDevice {
            id: "d1".into(),
            name: "laptop".into(),
            role: DeviceRole::Satellite,
        }
        .to_card(0, vec![]);
        card.sessions = vec!["mine".into()];
        let granted = vec!["acme".to_string()];
        let note = |repo: &str, operator: bool| {
            let mut e = operator_note_event(
                "x",
                Some(Recipient {
                    kind: RecipientKind::Session,
                    value: "mine".into(),
                }),
                NoteKind::Ask,
                None,
                0,
            );
            if !operator {
                e.harness = "claude-code".into();
                e.role = Role::Main;
                e.session = "peer".into();
                e.repo = Some(repo.into());
            }
            e
        };
        assert!(workshop_sees(
            &card,
            &note("/w/acme/app", false),
            &placement,
            &granted
        ));
        assert!(!workshop_sees(
            &card,
            &note("/w/me/tool", false),
            &placement,
            &granted
        ));
        assert!(workshop_sees(&card, &note("", true), &placement, &granted));
    }

    #[test]
    fn localhost_origins_only() {
        assert!(is_localhost_origin("http://localhost:3000"));
        assert!(is_localhost_origin("http://127.0.0.1:7419"));
        assert!(is_localhost_origin("http://localhost"));
        // Portless named URLs — the console's stable local front door.
        assert!(is_localhost_origin("http://harbard.local:1355"));
        assert!(is_localhost_origin("https://harbard.localhost"));
        assert!(is_localhost_origin("http://fix-ui.harbard.localhost:1355"));
        // Not localhost, wrong scheme on bare host, or a forged host.
        assert!(!is_localhost_origin("https://localhost:3000"));
        assert!(!is_localhost_origin("http://evil.com"));
        assert!(!is_localhost_origin("http://localhost.evil.com"));
        assert!(!is_localhost_origin("http://evil.localhost.attacker.com"));
        assert!(!is_localhost_origin("http://127.0.0.1:notaport"));
        assert!(!is_localhost_origin("http://localhost:3000/path"));
    }

    #[test]
    fn url_and_query_parsing() {
        assert_eq!(
            split_url("/api/v1/events?after=X&limit=10"),
            ("/api/v1/events", "after=X&limit=10")
        );
        assert_eq!(split_url("/api/v1/sessions"), ("/api/v1/sessions", ""));
        let q = "after=01ABC&limit=25";
        assert_eq!(query_param(q, "after").as_deref(), Some("01ABC"));
        assert_eq!(query_param(q, "limit").as_deref(), Some("25"));
        assert_eq!(query_param(q, "missing"), None);
    }

    #[test]
    fn events_body_windows_and_discloses() {
        use harbard_core::{Event, EventKind, Role, SCHEMA_VERSION};
        use ulid::Ulid;

        let dir = std::env::temp_dir().join(format!("harbard-serve-ut-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::at(&dir);

        let base = 1_000_000_000_000u64;
        let mut ids = Vec::new();
        for i in 0..5u64 {
            let id = Ulid::from_parts(base + i, i as u128).to_string();
            ids.push(id.clone());
            store
                .append(&Event {
                    v: SCHEMA_VERSION,
                    id,
                    ts: clock::rfc3339_utc(base + i),
                    event: EventKind::Note,
                    session: "s".to_string(),
                    harness: "codex".to_string(),
                    host: "h".to_string(),
                    cwd: "/tmp".to_string(),
                    repo: None,
                    repo_key: None,
                    branch: None,
                    role: Role::Main,
                    parent: None,
                    note_ref: None,
                    text: format!("n{i}"),
                    to: None,
                    kind: NoteKind::Fyi,
                })
                .unwrap();
        }

        // limit caps the returned set; total discloses the full window.
        let body = events_body(&store, "limit=3");
        assert_eq!(body["returned"], 3);
        assert_eq!(body["total"], 5);
        assert_eq!(body["newest_id"], ids[4]);
        // The newest three are returned, file-ordered (tail).
        let evs = body["events"].as_array().unwrap();
        assert_eq!(evs.len(), 3);
        assert_eq!(evs[2]["id"], ids[4]);
        // `after` returns only newer events.
        let body = events_body(&store, &format!("after={}", ids[2]));
        assert_eq!(body["total"], 2);
        assert_eq!(body["returned"], 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn one_incremental_reader_fans_out_and_bounds_subscribers() {
        let dir = std::env::temp_dir().join(format!(
            "harbard-hub-ut-{}-{}",
            std::process::id(),
            Ulid::generate()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::at(&dir);
        let shutdown = Arc::new(AtomicBool::new(false));
        let hub = EventHub::start(store.clone(), Arc::clone(&shutdown), test_window());

        let subscribers: Vec<_> = (0..MAX_SSE_CONNECTIONS)
            .map(|_| hub.subscribe(None).expect("within cap"))
            .collect();
        for _ in MAX_SSE_CONNECTIONS..64 {
            assert!(hub.subscribe(None).is_none());
        }

        let event = Event {
            v: SCHEMA_VERSION,
            id: Ulid::generate().to_string(),
            ts: clock::rfc3339_utc(clock::now_ms()),
            event: EventKind::Note,
            session: "fanout".into(),
            harness: "codex".into(),
            host: "host".into(),
            cwd: "/tmp".into(),
            repo: None,
            repo_key: None,
            branch: None,
            role: Role::Main,
            parent: None,
            note_ref: None,
            text: "one read".into(),
            to: None,
            kind: NoteKind::Fyi,
        };
        store.append(&event).unwrap();
        for subscriber in &subscribers {
            assert_eq!(
                subscriber
                    .receiver
                    .recv_timeout(Duration::from_secs(2))
                    .unwrap()
                    .id,
                event.id
            );
        }
        assert!(hub.reads.load(Ordering::Relaxed) < 10);

        shutdown.store(true, Ordering::Relaxed);
        drop(subscribers);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// §3: a client dropped for falling behind is told why. Before this the
    /// hub removed the subscriber in silence and the stream thread saw only a
    /// closed channel, so the browser resumed from now and never knew it had
    /// lost the gap.
    #[test]
    fn a_slow_client_is_told_why_before_the_socket_closes() {
        let dir = std::env::temp_dir().join(format!(
            "harbard-slow-ut-{}-{}",
            std::process::id(),
            Ulid::generate()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::at(&dir);
        let shutdown = Arc::new(AtomicBool::new(false));
        let hub = EventHub::start(store.clone(), Arc::clone(&shutdown), test_window());

        // A reader that never reads: its queue fills and then overflows.
        let subscription = hub.subscribe(None).expect("within cap");
        for index in 0..(SSE_QUEUE_CAPACITY + 8) {
            let event = Event {
                v: SCHEMA_VERSION,
                id: Ulid::generate().to_string(),
                ts: clock::rfc3339_utc(clock::now_ms()),
                event: EventKind::Note,
                session: "flood".into(),
                harness: "codex".into(),
                host: "host".into(),
                cwd: "/tmp".into(),
                repo: None,
                repo_key: None,
                branch: None,
                role: Role::Main,
                parent: None,
                note_ref: None,
                text: format!("flood {index}"),
                to: None,
                kind: NoteKind::Fyi,
            };
            store.append(&event).unwrap();
        }

        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && !subscription.evicted.load(Ordering::Acquire) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            subscription.evicted.load(Ordering::Acquire),
            "an overflowing queue must mark the subscriber, not drop it in silence"
        );

        // The stream thread owns the socket, so it is the one that says why.
        let mut written: Vec<u8> = Vec::new();
        close_stream(&mut written, &subscription);
        let text = String::from_utf8_lossy(&written).to_string();
        assert!(text.contains("event: reset"), "no reset frame: {text}");
        assert!(text.contains("slow_client"), "reset has no reason: {text}");

        shutdown.store(true, Ordering::Relaxed);
        drop(subscription);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn snapshot_uses_one_coherent_event_projection() {
        let dir = std::env::temp_dir().join(format!(
            "harbard-snapshot-ut-{}-{}",
            std::process::id(),
            Ulid::generate()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::at(&dir);
        let event = Event {
            v: SCHEMA_VERSION,
            id: Ulid::generate().to_string(),
            ts: clock::rfc3339_utc(clock::now_ms()),
            event: EventKind::Hello,
            session: "snapshot".into(),
            harness: "codex".into(),
            host: "host".into(),
            cwd: "/tmp".into(),
            repo: None,
            repo_key: None,
            branch: None,
            role: Role::Main,
            parent: None,
            note_ref: None,
            text: String::new(),
            to: None,
            kind: NoteKind::Fyi,
        };
        let body = snapshot_body(
            &store,
            std::slice::from_ref(&event),
            std::slice::from_ref(&event),
            60,
        );
        assert_eq!(body["events"][0]["id"], event.id);
        assert_eq!(body["sessions"][0]["session"], "snapshot");
        assert!(body.get("channels").is_some());
        assert!(body.get("delivery").is_some());
        assert!(body.get("leases").is_some());
        assert_eq!(body["leases_known"], true);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn append_ev(
        store: &Store,
        id_ms: u64,
        session: &str,
        kind: EventKind,
        repo: &str,
        text: &str,
    ) {
        let id =
            Ulid::from_datetime(std::time::UNIX_EPOCH + std::time::Duration::from_millis(id_ms))
                .to_string();
        store
            .append(&Event {
                v: SCHEMA_VERSION,
                id,
                ts: clock::rfc3339_utc(id_ms),
                event: kind,
                session: session.into(),
                harness: "codex".into(),
                host: "mac".into(),
                cwd: repo.into(),
                repo: Some(repo.into()),
                repo_key: None,
                branch: Some("main".into()),
                role: Role::Main,
                parent: None,
                note_ref: None,
                text: text.into(),
                to: None,
                kind: NoteKind::Fyi,
            })
            .unwrap();
    }

    #[test]
    fn history_checkouts_keeps_ended_and_writes_an_epitaph() {
        let dir = std::env::temp_dir().join(format!(
            "harbard-checkouts-ut-{}-{}",
            std::process::id(),
            Ulid::generate()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::at(&dir);
        append_ev(
            &store,
            1_000,
            "flint",
            EventKind::Hello,
            "/work/harbard",
            "",
        );
        append_ev(
            &store,
            2_000,
            "flint",
            EventKind::Intent,
            "/work/harbard",
            "cut the Edge",
        );
        append_ev(
            &store,
            3_000,
            "flint",
            EventKind::Note,
            "/work/harbard",
            "shipped the horizon",
        );
        append_ev(&store, 4_000, "flint", EventKind::Bye, "/work/harbard", "");

        let body = history_checkouts_body(&store, 900);
        let card = &body["checkouts"][0];
        assert_eq!(card["label"], "harbard");
        assert_eq!(card["ended"], 1);
        assert_eq!(card["occupants"][0]["liveness"], "ended");
        let epitaph = card["occupants"][0]["epitaph"].as_str().unwrap();
        assert!(epitaph.contains("cut the Edge"), "got {epitaph}");
        assert!(epitaph.contains("horizon"), "got {epitaph}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn note_trace_unknown_is_404() {
        let dir = std::env::temp_dir().join(format!(
            "harbard-notetrace-ut-{}-{}",
            std::process::id(),
            Ulid::generate()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::at(&dir);
        let err = note_trace_body(
            &store,
            900,
            "/api/v1/notes/01NOTFOUND0000000000000000/trace",
        )
        .expect_err("unknown note");
        assert_eq!(err.0, 404);
        assert_eq!(err.1["error"], "unknown_note");
        let _ = std::fs::remove_dir_all(&dir);
    }
    #[test]
    fn retained_hub_history_survives_rotation_without_replaying_old_events() {
        let dir = std::env::temp_dir().join(format!("harbard-hub-history-{}", Ulid::generate()));
        let store = Store::at(&dir);
        let now = clock::now_ms();
        let old = now - 4 * 86_400_000;
        append_ev(&store, old, "long", EventKind::Hello, "/work/long", "");
        append_ev(
            &store,
            old + 1,
            "long",
            EventKind::Intent,
            "/work/long",
            "long-running task",
        );
        std::fs::rename(
            store.segments().last().unwrap(),
            store.events_dir().join(format!(
                "events-{}.jsonl",
                clock::rfc3339_utc(old)[..10].replace('-', "")
            )),
        )
        .unwrap();
        let yesterday = now - 86_400_000;
        append_ev(
            &store,
            yesterday,
            "long",
            EventKind::Note,
            "/work/long",
            "yesterday",
        );
        std::fs::rename(
            store.segments().last().unwrap(),
            store.events_dir().join(format!(
                "events-{}.jsonl",
                clock::rfc3339_utc(yesterday)[..10].replace('-', "")
            )),
        )
        .unwrap();
        let hub = EventHub::start(
            store.clone(),
            Arc::new(AtomicBool::new(true)),
            test_window(),
        );
        let old_id = hub.snapshot().events[0].id.clone();
        let yesterday_id = hub.snapshot().recent_events[0].id.clone();
        let expired = hub.subscribe(Some(&old_id)).unwrap();
        assert!(!expired.resumed);
        assert!(expired.replay.is_empty());
        let subscription = hub.subscribe(Some(&yesterday_id)).unwrap();
        assert!(subscription.resumed);
        assert!(subscription.replay.is_empty());
        append_ev(&store, now, "long", EventKind::Note, "/work/long", "today");
        hub.poll(&store);
        let delivered = subscription.receiver.try_recv().unwrap();
        assert_eq!(event_timestamp_ms(&delivered), now);
        hub.poll(&store);
        assert!(subscription.receiver.try_recv().is_err());
        let state = hub.snapshot();
        assert_eq!(state.events.len(), 4);
        assert_eq!(state.recent_events.len(), 2);
        let body = snapshot_body(&store, &state.events, &state.recent_events, 900);
        assert_eq!(body["sessions"][0]["first_ms"], old);
        assert_eq!(body["sessions"][0]["intent"], "long-running task");
        assert_eq!(body["sessions"][0]["liveness"], "live");
        assert_eq!(body["total"], 2);
        drop(state);
        drop(subscription);
        drop(expired);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn startup_snapshot_overlap_merges_and_fans_out_the_tail_once() {
        let dir = std::env::temp_dir().join(format!("harbard-hub-overlap-{}", Ulid::generate()));
        let store = Store::at(&dir);
        let now = clock::now_ms();
        append_ev(&store, now - 1, "long", EventKind::Hello, "/work/long", "");
        let hub = EventHub::start(
            store.clone(),
            Arc::new(AtomicBool::new(true)),
            test_window(),
        );
        let initial_id = hub.snapshot().recent_events[0].id.clone();
        let subscription = hub.subscribe(Some(&initial_id)).unwrap();
        append_ev(
            &store,
            now,
            "long",
            EventKind::Note,
            "/work/long",
            "racing append",
        );
        // Model an append between capturing the byte position and reading All:
        // retained history contains it, while the recent feed and tail do not.
        let raced = store.read(Window::All).events.pop().unwrap();
        {
            let mut state = hub.snapshot();
            state.event_ids.insert(raced.id.clone());
            Arc::make_mut(&mut state.events).push(raced.clone());
        }
        let readers: Vec<_> = (0..2)
            .map(|_| {
                let hub = Arc::clone(&hub);
                let store = store.clone();
                thread::spawn(move || hub.poll(&store))
            })
            .collect();
        for reader in readers {
            reader.join().unwrap();
        }
        assert_eq!(subscription.receiver.try_recv().unwrap().id, raced.id);
        assert!(subscription.receiver.try_recv().is_err());
        assert_eq!(hub.snapshot().events.len(), 2);
        assert_eq!(hub.snapshot().recent_events.len(), 2);
        let resumed = hub.subscribe(Some(&initial_id)).unwrap();
        assert_eq!(resumed.replay.len(), 1);
        assert_eq!(resumed.replay[0].id, raced.id);
        drop(resumed);
        drop(subscription);
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn test_window() -> HubWindow {
        HubWindow::new(harbard_core::DEFAULT_RETENTION_DAYS, 900)
    }

    /// Write an event into the daily segment of the day it was minted, the
    /// way a store that has been running for weeks holds it.
    fn append_dated(store: &Store, id_ms: u64, session: &str, kind: EventKind, text: &str) {
        let event = Event {
            v: SCHEMA_VERSION,
            id: Ulid::from_datetime(std::time::UNIX_EPOCH + Duration::from_millis(id_ms))
                .to_string(),
            ts: clock::rfc3339_utc(id_ms),
            event: kind,
            session: session.into(),
            harness: "codex".into(),
            host: "mac".into(),
            cwd: "/work/long".into(),
            repo: Some("/work/long".into()),
            repo_key: None,
            branch: Some("main".into()),
            role: Role::Main,
            parent: None,
            note_ref: None,
            text: text.into(),
            to: None,
            kind: NoteKind::Fyi,
        };
        std::fs::create_dir_all(store.events_dir()).unwrap();
        let segment = store.events_dir().join(format!(
            "events-{}.jsonl",
            clock::rfc3339_utc(id_ms)[..10].replace('-', "")
        ));
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(segment)
            .unwrap();
        writeln!(file, "{}", event.to_line().unwrap()).unwrap();
    }

    #[test]
    fn the_hub_keeps_a_live_session_whose_hello_predates_the_window() {
        let dir = std::env::temp_dir().join(format!("harbard-hub-window-{}", Ulid::generate()));
        let store = Store::at(&dir);
        let now = clock::now_ms();
        let old = now - 20 * 86_400_000;
        append_dated(&store, old, "veteran", EventKind::Hello, "");
        append_dated(
            &store,
            old + 1,
            "veteran",
            EventKind::Intent,
            "port the hub",
        );
        append_dated(&store, old + 2, "departed", EventKind::Hello, "");
        for i in 0..40 {
            append_dated(&store, old + 10 + i, "departed", EventKind::Note, "chatter");
        }
        append_dated(&store, old + 100, "departed", EventKind::Bye, "");
        append_dated(
            &store,
            now - 1_000,
            "veteran",
            EventKind::Note,
            "still here",
        );

        let hub = EventHub::start(
            store.clone(),
            Arc::new(AtomicBool::new(true)),
            test_window(),
        );
        let (events, _) = hub.history();
        assert!(
            events.iter().all(|event| event.session == "veteran"),
            "an ended session older than the window is not retained"
        );
        assert_eq!(events.len(), 3);
        assert_eq!(hub.snapshot().event_ids.len(), 3);

        let body = sessions_body_from(&store, &events, 900);
        let sessions = body["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0]["session"], "veteran");
        assert_eq!(sessions[0]["intent"], "port the hub");
        assert_eq!(sessions[0]["first_ms"], old);
        assert_eq!(sessions[0]["liveness"], "live");
        assert_eq!(sessions[0]["lifecycle"], "active");

        // The store still answers for what the hub left out.
        let trace = session_trace_body(
            &store,
            &events,
            hub.horizon_ms(),
            900,
            "/api/v1/sessions/departed/trace",
            "",
        );
        assert_eq!(trace["session"], "departed");
        assert_ne!(trace["harness"], "");
        drop(events);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_long_running_hub_lets_old_history_go() {
        let dir = std::env::temp_dir().join(format!("harbard-hub-trim-{}", Ulid::generate()));
        let store = Store::at(&dir);
        let now = clock::now_ms();
        for i in 0..10 {
            append_dated(&store, now - 1_000 + i, "brief", EventKind::Note, "today");
        }
        let hub = EventHub::start(
            store.clone(),
            Arc::new(AtomicBool::new(true)),
            test_window(),
        );
        assert_eq!(hub.history().0.len(), 10);

        // Not due yet: an hour has not passed since the startup trim.
        hub.trim_if_due(&store, now + 60_000);
        assert_eq!(hub.history().0.len(), 10);

        // A month later the same process holds none of it: nothing in it is
        // live, and nothing it said is still owed.
        let later = now + 30 * 86_400_000;
        hub.trim_if_due(&store, later);
        assert!(hub.history().0.is_empty());
        let state = hub.snapshot();
        assert!(state.event_ids.is_empty());
        assert_eq!(state.horizon_ms, later - test_window().span_ms);
        drop(state);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn the_request_log_names_the_route_and_never_its_query() {
        let fields = request_log_fields(
            "GET",
            "/api/v1/sessions",
            Some(200),
            Duration::from_millis(12),
        )
        .unwrap();
        assert_eq!(
            fields,
            json!({ "method": "GET", "path": "/api/v1/sessions", "status": 200, "duration_ms": 12 })
        );
        // The watchdog's healthy probe stays out of the log; a failing one does not.
        assert!(request_log_fields("GET", "/api/v1/health", Some(200), Duration::ZERO).is_none());
        assert!(request_log_fields("GET", "/api/v1/health", Some(503), Duration::ZERO).is_some());

        record_status(404);
        assert_eq!(take_status(), Some(404));
        assert_eq!(
            take_status(),
            None,
            "a status never leaks into the next request"
        );
    }
}
