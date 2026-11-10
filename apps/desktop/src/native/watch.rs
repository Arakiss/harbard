//! The native side's one observer of the desk. Each time the read changes it
//! moves the Dock tile, badge and bounce, the menu-bar mark and count, and
//! rings the doorbell for new events. A few slow clocks run beside it: the
//! serve probe, the update check and the away watch. Nothing here polls the
//! bus: the events arrive with the read.

use super::away::{AwayWatch, Suggest};
use super::dock::{self, Host, Model, OneShot, Plan};
use super::gate::{Feature, allows};
use super::{appkit, doorbell, logo, rules, tr};
use crate::desk::{Connection, Desk};
use gpui::{App, AppContext as _, AsyncApp, Global, SharedString, SystemNotification};
use harbard_core::event::Event;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How often serve is asked whether it answers (the Dock's outage ring, the
/// "Bus disconnected" notice). One loopback request.
const SERVE_EVERY: Duration = Duration::from_secs(8);
/// The away watch's step (`ioreg` twice).
const AWAY_EVERY: Duration = Duration::from_secs(16);
/// The first update check after launch, then every ten minutes.
const UPDATE_FIRST: Duration = Duration::from_secs(12);
const UPDATE_EVERY: Duration = Duration::from_secs(600);
/// Names for sessions outside the current workspace: one read of the
/// whole hub at most this often.
const NAMES_TTL: Duration = Duration::from_secs(60);
/// The live count is recounted at most this often.
const COUNT_EVERY: Duration = Duration::from_secs(2);

pub struct Watch {
    pub dock: Model,
    once: logo::OneShots,
    cursor: Option<String>,
    events_seen: usize,
    all_names: doorbell::Names,
    names_at: Option<Instant>,
    serve_up: Option<bool>,
    disconnect_rang: bool,
    away: AwayWatch,
    counted_at: Option<Instant>,
    counting: bool,
    pub live: usize,
}

impl Global for Watch {}

impl Watch {
    fn new() -> Watch {
        Watch {
            dock: Model::default(),
            once: logo::OneShots::default(),
            cursor: None,
            events_seen: 0,
            all_names: doorbell::Names::default(),
            names_at: None,
            serve_up: None,
            disconnect_rang: false,
            away: AwayWatch::default(),
            counted_at: None,
            counting: false,
            live: 0,
        }
    }
}

struct NativeHost;

impl Host for NativeHost {
    fn low_power(&self) -> bool {
        appkit::low_power()
    }
    fn frontmost(&self) -> bool {
        appkit::app_active()
    }
    fn attention_enabled(&self) -> bool {
        rules::attention_allowed(&rules::home(), rules::now_ms())
    }
}

pub fn init(cx: &mut App) {
    cx.set_global(Watch::new());
    let desk = Desk::get(cx);
    cx.observe(&desk, |_, cx| on_desk(cx)).detach();
    // The first plan sets the tray mark and restores the bundle icon.
    let plan = cx.global_mut::<Watch>().dock.set_offline(false);
    apply(plan, cx);
    cx.spawn(async move |cx: &mut AsyncApp| serve_loop(cx).await).detach();
    cx.spawn(async move |cx: &mut AsyncApp| away_loop(cx).await).detach();
    cx.spawn(async move |cx: &mut AsyncApp| update_loop(cx).await).detach();
}

fn on_desk(cx: &mut App) {
    let desk = Desk::get(cx).read(cx);
    let state = logo::logo_state(desk);
    let needs = logo::hub_needs(desk);
    let activity = desk.activity.clone();
    let events = desk.hub.events();
    let names = desk.view.as_deref().map(doorbell::Names::from_view).unwrap_or_default();
    let store = desk.hub.store().clone();
    let w = cx.global_mut::<Watch>();
    let event = match activity.as_deref() {
        Some(a) => w.once.next(a).filter(|_| rules::one_shots_now()),
        None => None,
    };
    let plan = w
        .dock
        .report(dock::Status::new(state, needs, event), Instant::now(), &NativeHost);
    // A bounce the Dock really rang is offered for feedback ("The Dock
    // bounced at 14:02 · This bothered me").
    let rang = plan.bounce && allows(Feature::Dock);
    apply(plan, cx);
    if rang {
        crate::ui::feedback::FeedbackStore::record_bounce(rules::now_ms(), state.as_str(), cx);
    }
    doorbells(&events, names, store.clone(), cx);
    recount(events, store, cx);
}

/// Draw a plan: the Dock tile, badge, bounce and animation, and the tray mark.
pub fn apply(plan: Plan, cx: &mut App) {
    if plan.is_empty() {
        return;
    }
    if trace() {
        eprintln!("harbard: dock {plan:?}");
    }
    if let Some(mark) = plan.tray {
        super::with_tray(cx, |t| t.set_mark(mark));
    }
    if !allows(Feature::Dock) {
        return;
    }
    if let Some(tile) = plan.tile {
        appkit::set_tile(tile);
    }
    if let Some(label) = plan.badge {
        appkit::set_badge(label.as_deref());
    }
    if plan.bounce {
        appkit::bounce();
    }
    if let Some(event) = plan.animate {
        play(event, cx);
    }
}

/// One one-shot, frame by frame, then the tile for whatever the state is now.
fn play(event: OneShot, cx: &mut App) {
    cx.spawn(async move |cx: &mut AsyncApp| {
        for frame in event.frames() {
            cx.update(|_| appkit::set_tile(Some(frame)));
            cx.background_executor().timer(dock::FRAME).await;
        }
        cx.update(|cx| {
            let tile = cx.global_mut::<Watch>().dock.animation_done();
            appkit::set_tile(tile);
        });
    })
    .detach();
}

// ---------------------------------------------------------------------------
// Doorbells
// ---------------------------------------------------------------------------

fn doorbells(events: &Arc<Vec<Event>>, names: doorbell::Names, store: harbard_core::store::Store, cx: &mut App) {
    let w = cx.global_mut::<Watch>();
    // The hub only appends (and trims the front): a same-length list with the
    // same last id brought nothing new.
    let last = events.last().map(|e| e.id.as_str());
    if events.len() == w.events_seen && last == w.cursor.as_deref() {
        return;
    }
    w.events_seen = events.len();
    let (new, cursor) = doorbell::since(events, w.cursor.as_deref());
    w.cursor = cursor;
    let candidates: Vec<Event> = new.iter().filter(|e| doorbell::may_ring(e)).cloned().collect();
    if candidates.is_empty() {
        return;
    }
    let unknown = candidates
        .iter()
        .any(|e| !names.knows(&e.session) && !w.all_names.knows(&e.session));
    let stale = w.names_at.is_none_or(|at| at.elapsed() >= NAMES_TTL);
    if unknown && stale {
        // A session in another workspace: read the whole hub for its name.
        w.names_at = Some(Instant::now());
        let events = events.clone();
        let task = cx.background_spawn(async move { all_names(&store, &events) });
        cx.spawn(async move |cx: &mut AsyncApp| {
            let all = task.await;
            cx.update(|cx| {
                if let Some(all) = all {
                    cx.global_mut::<Watch>().all_names = all;
                }
                ring(&candidates, &names, cx);
            });
        })
        .detach();
    } else {
        ring(&candidates, &names, cx);
    }
}

fn all_names(store: &harbard_core::store::Store, events: &[Event]) -> Option<doorbell::Names> {
    let v = crate::hub::read(store, events, Some("all".into()), false).ok()?;
    let view: crate::model::WorkspaceView = serde_json::from_value(v).ok()?;
    Some(doorbell::Names::from_view(&view))
}

fn ring(events: &[Event], names: &doorbell::Names, cx: &mut App) {
    if super::muted() || !rules::alerts_now() || super::main_in_front(cx) {
        return;
    }
    let mut merged = cx.global::<Watch>().all_names.clone();
    merged.0.extend(names.0.iter().map(|(k, v)| (k.clone(), v.clone())));
    let handle = rules::operator_handle(&rules::home());
    let rings: Vec<(String, String)> = {
        let tr = |k: &str, v: &[(&str, &str)]| tr(cx, k, v);
        events
            .iter()
            .filter_map(|e| doorbell::notification_for(e, &merged, &handle, &tr))
            .collect()
    };
    let batch = {
        let tr = |k: &str, v: &[(&str, &str)]| tr(cx, k, v);
        doorbell::batch(&rings, &tr)
    };
    if let Some((title, body)) = batch {
        notify("doorbell", &title, &body, cx);
        super::summon_strip(cx);
    }
}

/// Deliver one notification: silent, never when muted; in a build whose gate
/// is closed, a line on stderr instead. True when the OS was handed it.
pub fn notify(tag: &str, title: &str, body: &str, cx: &mut App) -> bool {
    if super::muted() {
        return false;
    }
    if !allows(Feature::Notify) || !super::gate::gate().bundled {
        eprintln!("harbard: would notify: {title} — {body}");
        return false;
    }
    cx.show_system_notification(SystemNotification {
        tag: SharedString::from(format!("{tag}-{}", rules::now_ms())),
        title: title.to_string().into(),
        body: body.to_string().into(),
        actions: Vec::new(),
    });
    true
}

// ---------------------------------------------------------------------------
// The live count
// ---------------------------------------------------------------------------

fn recount(events: Arc<Vec<Event>>, store: harbard_core::store::Store, cx: &mut App) {
    let w = cx.global_mut::<Watch>();
    if w.counting || w.counted_at.is_some_and(|at| at.elapsed() < COUNT_EVERY) {
        return;
    }
    w.counting = true;
    w.counted_at = Some(Instant::now());
    let task = cx.background_spawn(async move {
        let now = harbard_core::clock::now_ms();
        harbard_core::query::sessions(&store, &events, now, harbard_core::query::DEFAULT_STALE_SECS)
            .iter()
            .filter(|s| {
                s.liveness == harbard_core::query::Liveness::Live && s.harness != harbard_core::event::HARNESS_HUMAN
            })
            .count()
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let live = task.await;
        cx.update(|cx| {
            let w = cx.global_mut::<Watch>();
            w.counting = false;
            if trace() && w.live != live {
                eprintln!("harbard: live sessions {live}");
            }
            w.live = live;
            show_count(cx);
        });
    })
    .detach();
}

/// The tray's count and tooltip, in the current language.
pub fn show_count(cx: &mut App) {
    let live = cx.global::<Watch>().live;
    let tip = if live > 0 {
        tr(cx, "trayLive", &[("count", &live.to_string())])
    } else {
        tr(cx, "trayFleet", &[])
    };
    super::with_tray(cx, |t| t.set_count(live, &tip));
}

// ---------------------------------------------------------------------------
// serve, away, update
// ---------------------------------------------------------------------------

async fn serve_loop(cx: &mut AsyncApp) {
    loop {
        let up = cx.background_spawn(async { crate::api::get("/health").is_ok() }).await;
        cx.update(|cx| serve_seen(up, cx));
        cx.background_executor().timer(SERVE_EVERY).await;
    }
}

fn serve_seen(up: bool, cx: &mut App) {
    let w = cx.global_mut::<Watch>();
    let was = w.serve_up.replace(up);
    if was == Some(up) {
        return;
    }
    let plan = w.dock.set_offline(!up);
    let ring = !up && was == Some(true) && !w.disconnect_rang;
    if up {
        w.disconnect_rang = false;
    } else if ring {
        w.disconnect_rang = true;
    }
    apply(plan, cx);
    if ring && rules::alerts_now() {
        let (title, body) = (tr(cx, "busDisconnected", &[]), tr(cx, "busDisconnectedBody", &[]));
        notify("bus", &title, &body, cx);
        super::summon_strip(cx);
    }
}

async fn away_loop(cx: &mut AsyncApp) {
    loop {
        cx.background_executor().timer(AWAY_EVERY).await;
        let reading = cx
            .background_spawn(async {
                let reading = super::away::read_now()?;
                Some((reading, rules::away_inputs(&rules::home(), rules::now_ms())))
            })
            .await;
        let Some(((idle_ms, locked), (state, enabled))) = reading else {
            continue;
        };
        cx.update(|cx| {
            let w = cx.global_mut::<Watch>();
            let Some(suggest) = w.away.step(idle_ms, locked, &state, enabled) else {
                return;
            };
            super::suggest(suggest, cx);
            if suggest == Suggest::Away && rules::alerts_now() {
                let (title, body) = (tr(cx, "awayTitle", &[]), tr(cx, "awayBody", &[]));
                notify("away", &title, &body, cx);
            }
        });
    }
}

async fn update_loop(cx: &mut AsyncApp) {
    cx.background_executor().timer(UPDATE_FIRST).await;
    loop {
        if let Ok(body) = cx.background_spawn(async { crate::api::get("/update") }).await {
            cx.update(|cx| ring_update(&body, cx));
        }
        cx.background_executor().timer(UPDATE_EVERY).await;
    }
}

/// The update the operator was last told about, so a restart never rings
/// the same one twice.
fn update_notified_path() -> std::path::PathBuf {
    rules::home().join("update").join("desktop-notified")
}

fn ring_update(body: &serde_json::Value, cx: &mut App) {
    let notice = {
        let tr = |k: &str, v: &[(&str, &str)]| tr(cx, k, v);
        doorbell::update_notice(body, &tr)
    };
    let Some((commit, title, text)) = notice else { return };
    let path = update_notified_path();
    if std::fs::read_to_string(&path).is_ok_and(|seen| seen.trim() == commit) {
        return;
    }
    // Only a notice the operator could see is remembered: a dev build with
    // its gate closed must not use up the installed app's notice.
    if !notify("update", &title, &text, cx) {
        return;
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, commit);
}

/// The connection the logo reads, for callers outside the desk.
pub fn connection(cx: &App) -> Connection {
    Desk::get(cx).read(cx).connection()
}

/// `HARBARD_APP_TRACE`: log what the Dock and the tray would show.
fn trace() -> bool {
    std::env::var_os("HARBARD_APP_TRACE").is_some()
}
