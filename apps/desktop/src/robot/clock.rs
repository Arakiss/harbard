//! The robot clock: one timer for every robot and status icon in the app.
//!
//! The console drives its robots at 8 frames a second (a blink advances on
//! every tick, the slow loops on every second one) and runs reactions at
//! full rate, once. Here each robot, when it is drawn, says when it next
//! looks different ([`wake_at`]); the clock keeps one timer for the earliest
//! of those times and, when it fires, notifies only the views that asked.
//! A robot at rest between blinks asks for nothing, so nothing is drawn;
//! a robot no longer drawn never asks again. A hidden window parks its
//! robots until it shows again. Reduce motion (the workspace setting or the
//! macOS one) draws still faces and asks for no frames, and so does a window
//! in the background ([`still`]), until it is active again; only the Strip,
//! made to be watched from other apps, keeps moving there.
//!
//! Robots and status icons are small nested views ([`nested`]) cached at a
//! fixed size, so a tick redraws just them, never the screen around them.

use crate::perf::Source;
use gpui::{
    AnyElement, App, AppContext, ElementId, Entity, EntityId, Global, IntoElement, Render,
    StyleRefinement, Subscription, Task, Window, WindowId,
};
use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// One tick of the 8 FPS clock.
pub const TICK_MS: f64 = 125.0;
/// The slow loops (breathing, dots, scans) advance every second tick.
pub const LOOP_MS: f64 = 250.0;

/// Counters for measuring: robot and icon paints, and clock firings.
pub static PAINTS: AtomicU64 = AtomicU64::new(0);
pub static WAKES: AtomicU64 = AtomicU64::new(0);

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

/// The clock's time in ms (since the robots started).
pub fn clock_ms(at: Instant) -> f64 {
    at.saturating_duration_since(epoch()).as_secs_f64() * 1000.0
}

pub fn instant_at(ms: f64) -> Instant {
    epoch() + Duration::from_secs_f64(ms.max(0.0) / 1000.0)
}

/// The first grid line of `step` ms strictly after `ms`.
pub fn next_grid(ms: f64, step: f64) -> f64 {
    ((ms / step).floor() + 1.0) * step
}

/// The first grid line of `step` ms after `ms` (itself on the grid) at
/// which `look` differs from `look(ms)`, looking at most `horizon` ms
/// ahead. A loop that draws the same picture on most ticks then wakes only
/// for the ticks that change it.
pub fn next_change<T: PartialEq>(ms: f64, step: f64, horizon: f64, look: impl Fn(f64) -> T) -> f64 {
    let now = look(ms);
    let mut next = ms + step;
    while next < ms + horizon && look(next) == now {
        next += step;
    }
    next
}

/// `ms` rounded down to the grid.
pub fn on_grid(ms: f64, step: f64) -> f64 {
    (ms / step).floor() * step
}

/// When the app's robots started, on the wall clock: reactions older than
/// this happened before the app opened and never play.
pub fn baseline_ms() -> u64 {
    static BASE: OnceLock<u64> = OnceLock::new();
    *BASE.get_or_init(harbard_core::clock::now_ms)
}

/// Start the clock's epoch and baseline at app start.
pub fn init() {
    let _ = epoch();
    let _ = baseline_ms();
}

pub fn trace() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("HARBARD_ROBOT_TRACE").is_some())
}

#[derive(Default)]
struct Clock {
    due: HashMap<EntityId, Instant>,
    armed: Option<(Instant, Task<()>)>,
    parked: HashMap<WindowId, (Vec<EntityId>, Subscription)>,
    /// What each due view is woken for, for the performance overlay.
    why: HashMap<EntityId, Source>,
    /// Windows whose robots move while another app is in front.
    ambient: HashSet<WindowId>,
    /// One activation watch per window that has drawn a robot.
    watched: HashMap<WindowId, Entity<ActivationWatch>>,
}

/// Draws its window again when the window gains or loses focus, so its
/// robots start or stop moving.
struct ActivationWatch {
    _sub: Subscription,
}

/// Mark `window` as ambient: its robots move even with the app behind.
pub fn set_ambient(window: &Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    cx.default_global::<Clock>().ambient.insert(id);
}

/// Whether robots in `window` hold still: motion reduced, or the window
/// not the one in front (the Strip excepted). Words, timers and faces still
/// follow the agents; only the loops, blinks and breaths stop. Saves the
/// frames, and the battery, of an app nobody is looking at.
pub fn still(window: &mut Window, cx: &mut App) -> bool {
    watch_activation(window, cx);
    let id = window.window_handle().window_id();
    let behind = !window.is_window_active() && !cx.default_global::<Clock>().ambient.contains(&id);
    behind || reduced(cx)
}

fn watch_activation(window: &mut Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    if cx.default_global::<Clock>().watched.contains_key(&id) {
        return;
    }
    let watch = cx.new(|cx: &mut gpui::Context<ActivationWatch>| ActivationWatch {
        _sub: cx.observe_window_activation(window, |_, window, _| window.refresh()),
    });
    cx.default_global::<Clock>().watched.insert(id, watch);
}

impl Global for Clock {}

/// Ask for `view` to be drawn again at `at`, for `why` (counted for the
/// performance overlay).
#[track_caller]
pub fn wake_at(view: EntityId, at: Instant, why: Source, window: &Window, cx: &mut App) {
    if trace() {
        let ms = at.saturating_duration_since(Instant::now()).as_millis();
        eprintln!(
            "robot wake in {ms} ms from {}",
            std::panic::Location::caller()
        );
    }
    if !window.is_visible() {
        park(view, window, cx);
        return;
    }
    let clock = cx.default_global::<Clock>();
    let slot = clock.due.entry(view).or_insert(at);
    if at <= *slot {
        *slot = at;
        clock.why.insert(view, why);
    }
    arm(cx);
}

fn park(view: EntityId, window: &Window, cx: &mut App) {
    let id = window.window_handle().window_id();
    let clock = cx.default_global::<Clock>();
    if let Some((views, _)) = clock.parked.get_mut(&id) {
        if !views.contains(&view) {
            views.push(view);
        }
        return;
    }
    if trace() {
        eprintln!("robot clock: window hidden, robots parked");
    }
    let sub = window.observe_window_visibility(move |_, window, cx| {
        if !window.is_visible() {
            return;
        }
        let parked = cx.default_global::<Clock>().parked.remove(&id);
        if let Some((views, _)) = parked {
            if trace() {
                eprintln!("robot clock: window shown, {} views resume", views.len());
            }
            for v in views {
                cx.notify(v);
            }
        }
    });
    cx.default_global::<Clock>()
        .parked
        .insert(id, (vec![view], sub));
}

fn arm(cx: &mut App) {
    let clock = cx.default_global::<Clock>();
    let Some(&next) = clock.due.values().min() else {
        clock.armed = None;
        return;
    };
    if clock.armed.as_ref().is_some_and(|(at, _)| *at <= next) {
        return;
    }
    let wait = next.saturating_duration_since(Instant::now());
    let task = cx.spawn(async move |cx| {
        cx.background_executor().timer(wait).await;
        cx.update(fire);
    });
    cx.default_global::<Clock>().armed = Some((next, task));
}

fn fire(cx: &mut App) {
    // A couple of ms early counts as due: timers land a little late or early.
    let now = Instant::now() + Duration::from_millis(3);
    let clock = cx.default_global::<Clock>();
    clock.armed = None;
    let ready: Vec<EntityId> = clock
        .due
        .iter()
        .filter(|(_, at)| **at <= now)
        .map(|(v, _)| *v)
        .collect();
    for v in &ready {
        clock.due.remove(v);
        let why = clock.why.remove(v).unwrap_or(Source::Robots);
        crate::perf::count(why, 1);
    }
    WAKES.fetch_add(1, Ordering::Relaxed);
    if trace() {
        eprintln!(
            "robot clock {:.0}: {} views, {} paints so far",
            clock_ms(Instant::now()),
            ready.len(),
            PAINTS.load(Ordering::Relaxed)
        );
    }
    for v in ready {
        cx.notify(v);
    }
    arm(cx);
}

// ---------------------------------------------------------------------------
// Reduce motion
// ---------------------------------------------------------------------------

/// The macOS "Reduce motion" setting, read at most every two seconds.
pub fn os_reduce_motion() -> bool {
    thread_local! {
        static LAST: Cell<Option<(Instant, bool)>> = const { Cell::new(None) };
    }
    LAST.with(|last| {
        if let Some((at, v)) = last.get()
            && at.elapsed() < Duration::from_secs(2)
        {
            return v;
        }
        let v = read_os_reduce_motion();
        last.set(Some((Instant::now(), v)));
        v
    })
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)] // the objc macros test a feature this crate does not declare
fn read_os_reduce_motion() -> bool {
    use objc::runtime::{BOOL, NO, Object};
    use objc::{class, msg_send, sel, sel_impl};
    // SAFETY: plain AppKit property reads on the main thread.
    unsafe {
        let ws: *mut Object = msg_send![class!(NSWorkspace), sharedWorkspace];
        if ws.is_null() {
            return false;
        }
        let on: BOOL = msg_send![ws, accessibilityDisplayShouldReduceMotion];
        on != NO
    }
}

#[cfg(not(target_os = "macos"))]
fn read_os_reduce_motion() -> bool {
    false
}

/// Whether robots hold still: Reduce motion in Preferences or in macOS.
pub fn reduced(cx: &App) -> bool {
    let desk = crate::desk::Desk::get(cx).read(cx);
    let setting = desk
        .activity
        .as_ref()
        .and_then(|a| a.motion.as_deref())
        .or_else(|| desk.view.as_ref().and_then(|v| v.motion.as_deref()));
    setting == Some("reduce") || cx.reduce_motion() || os_reduce_motion()
}

// ---------------------------------------------------------------------------
// Nested views
// ---------------------------------------------------------------------------

/// A small view drawn from props: a robot or a status icon.
pub trait Nested: Render {
    type Props: PartialEq + 'static;
    fn from_props(props: Self::Props) -> Self;
    fn props_mut(&mut self) -> &mut Self::Props;
}

thread_local! {
    static GENERATION: Cell<u64> = const { Cell::new(0) };
    static PAINTED: Cell<bool> = const { Cell::new(false) };
}

/// A nested view painted: the next render pass is a new frame.
pub fn note_painted() {
    PAINTED.with(|p| p.set(true));
    PAINTS.fetch_add(1, Ordering::Relaxed);
}

fn generation() -> u64 {
    GENERATION.with(|g| {
        if PAINTED.with(|p| p.replace(false)) {
            g.set(g.get() + 1);
        }
        g.get()
    })
}

struct Slot<V: 'static> {
    entity: Entity<V>,
    generation: u64,
}

/// `V` as a cached nested view laid out at `style`, kept across frames
/// under `key`. When the same key is drawn twice in one frame (the same
/// agent twice in a list without ids), the second gets its props back to
/// draw inline instead.
pub fn nested<V: Nested>(
    key: ElementId,
    style: StyleRefinement,
    props: V::Props,
    window: &mut Window,
    cx: &mut App,
) -> Result<AnyElement, V::Props> {
    let generation = generation();
    let entity = window.with_global_id(key, |id, window| {
        window.with_element_state(id, |slot: Option<Slot<V>>, _| match slot {
            Some(slot) if slot.generation == generation => (Err(props), slot),
            Some(slot) => {
                slot.entity.update(cx, |view, cx| {
                    let current = view.props_mut();
                    if *current != props {
                        *current = props;
                        cx.notify();
                    }
                });
                let entity = slot.entity.clone();
                (
                    Ok(entity),
                    Slot {
                        entity: slot.entity,
                        generation,
                    },
                )
            }
            None => {
                let entity = cx.new(|_| V::from_props(props));
                (Ok(entity.clone()), Slot { entity, generation })
            }
        })
    })?;
    Ok(entity.cached(style).into_any_element())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_grid_is_strictly_ahead() {
        assert_eq!(next_grid(0.0, 125.0), 125.0);
        assert_eq!(next_grid(124.9, 125.0), 125.0);
        assert_eq!(next_grid(125.0, 125.0), 250.0);
        assert_eq!(on_grid(260.0, 250.0), 250.0);
    }
}
