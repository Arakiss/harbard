//! The living Dock icon, its badge, the bounce and the tray's state mark.
//!
//! The console owns the state model (the living logo, `LogoState` in
//! `apps/console/src/lib/activity.ts`) and reports it through
//! `desk_set_status`. This module only shows it:
//!
//! - the Dock tile: the bundle icon at rest, a pre-rendered tile for every
//!   other lasting state, and a one-shot animation for an event;
//! - the red Dock badge with the count of what waits for the operator;
//! - one informational bounce when a new "Needs you" item appears while
//!   harbard is not frontmost, if "Notify in the Dock" is on;
//! - the menu-bar template icon: a dot, a ring when there is no bus, an
//!   exclamation when the operator is needed.
//!
//! Nothing here runs on a timer. The tile is set once per change; an
//! animation is a short thread that ends; identical reports change nothing.
//! The shell's own poll overrides the reported state with `disconnected`
//! while the bus is unreachable, so the Dock tells the truth with the console
//! window closed or the console itself down.

use serde_json::Value;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// The Dock badge never shows more than this; above it reads "99+".
pub const BADGE_SHOWN_MAX: u32 = 99;
/// Counts are clamped to this before anything else.
pub const BADGE_MAX: i64 = 9_999;
/// At most one animation in this window; later events in a burst only
/// update the state.
pub const ANIMATION_GAP: Duration = Duration::from_secs(3);
/// A console that has reported once and then says nothing for this long is
/// taken as gone: its counts are stale, so the Dock shows the outage ring.
/// The console repeats its state every 20 s (`KEEPALIVE_MS`).
pub const HEARTBEAT: Duration = Duration::from_secs(60);

/// Whether the console's heartbeat is lost. Armed only by a first report,
/// so a console that never speaks this protocol leaves the Dock alone.
pub fn heartbeat_expired(last_report: Option<Instant>, now: Instant) -> bool {
    last_report.is_some_and(|at| now.saturating_duration_since(at) >= HEARTBEAT)
}

/// One animation frame. Every animation stays under a second.
pub const FRAME: Duration = Duration::from_millis(80);

/// The living logo's lasting states, by the console's names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogoState {
    Idle,
    Working,
    Needs,
    Blocked,
    Connecting,
    Disconnected,
}

impl LogoState {
    #[cfg(test)]
    pub const ALL: [LogoState; 6] = [
        LogoState::Idle,
        LogoState::Working,
        LogoState::Needs,
        LogoState::Blocked,
        LogoState::Connecting,
        LogoState::Disconnected,
    ];

    pub fn parse(raw: &str) -> Option<Self> {
        Some(match raw {
            "idle" => Self::Idle,
            "working" => Self::Working,
            "needs" => Self::Needs,
            "blocked" => Self::Blocked,
            "connecting" => Self::Connecting,
            "disconnected" => Self::Disconnected,
            _ => return None,
        })
    }

    #[cfg(test)]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Idle => "idle",
            Self::Working => "working",
            Self::Needs => "needs",
            Self::Blocked => "blocked",
            Self::Connecting => "connecting",
            Self::Disconnected => "disconnected",
        }
    }

    /// The Dock tile, or `None` for the bundle icon. At rest the Dock shows
    /// the bundle's own icon, so macOS 26 draws its layered version in the
    /// operator's icon appearance instead of a flat image.
    pub fn tile(self) -> Option<&'static Tile> {
        match self {
            Self::Idle => None,
            Self::Working => Some(&WORKING),
            Self::Needs => Some(&NEEDS),
            Self::Blocked => Some(&BLOCKED),
            // Neither has a live bus behind it: the same hollow grey ring.
            Self::Connecting | Self::Disconnected => Some(&DISCONNECTED),
        }
    }

    /// The tray's shape. Template images are monochrome, so state is shape.
    pub fn tray(self) -> TrayMark {
        match self {
            Self::Idle | Self::Working => TrayMark::Dot,
            Self::Needs | Self::Blocked => TrayMark::Alert,
            Self::Connecting | Self::Disconnected => TrayMark::Ring,
        }
    }
}

/// A one-shot animation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OneShot {
    Message,
    Verified,
}

impl OneShot {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "message" => Some(Self::Message),
            "verified" => Some(Self::Verified),
            _ => None,
        }
    }

    pub fn frames(self) -> &'static [Tile] {
        match self {
            Self::Message => &MESSAGE,
            Self::Verified => &VERIFIED,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayMark {
    Dot,
    Ring,
    Alert,
}

impl TrayMark {
    pub fn png(self) -> &'static [u8] {
        match self {
            Self::Dot => include_bytes!("../icons/tray-dot.png"),
            Self::Ring => include_bytes!("../icons/tray-ring.png"),
            Self::Alert => include_bytes!("../icons/tray-alert.png"),
        }
    }
}

/// One pre-rendered Dock image (`scripts/render-dock.py`). Only the 512 px
/// render is embedded: the Dock never draws a tile above 256 pt at 2x. The
/// 1024 px renders stay in the repo for the preview page and future use.
#[derive(PartialEq, Eq)]
pub struct Tile {
    pub name: &'static str,
    pub png: &'static [u8],
}

/// By name: a log line never carries the image.
impl std::fmt::Debug for Tile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Tile({})", self.name)
    }
}

macro_rules! tile {
    ($name:literal) => {
        Tile {
            name: $name,
            png: include_bytes!(concat!("../icons/dock/", $name, "-512.png")),
        }
    };
}

static WORKING: Tile = tile!("working");
static NEEDS: Tile = tile!("needs");
static BLOCKED: Tile = tile!("blocked");
static DISCONNECTED: Tile = tile!("disconnected");
static MESSAGE: [Tile; 10] = [
    tile!("message-00"),
    tile!("message-01"),
    tile!("message-02"),
    tile!("message-03"),
    tile!("message-04"),
    tile!("message-05"),
    tile!("message-06"),
    tile!("message-07"),
    tile!("message-08"),
    tile!("message-09"),
];
static VERIFIED: [Tile; 12] = [
    tile!("verified-00"),
    tile!("verified-01"),
    tile!("verified-02"),
    tile!("verified-03"),
    tile!("verified-04"),
    tile!("verified-05"),
    tile!("verified-06"),
    tile!("verified-07"),
    tile!("verified-08"),
    tile!("verified-09"),
    tile!("verified-10"),
    tile!("verified-11"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub state: LogoState,
    pub badge: u32,
    /// Open "Needs you" items. Optional: without it a bounce is rung when
    /// the state turns to `needs` or `blocked`.
    pub needs: Option<u32>,
    pub event: Option<OneShot>,
}

/// Any JSON number, clamped to 0..9999: a huge, negative or fractional
/// count is shown as the nearest count, never refused.
fn count(raw: &Value, name: &str) -> Result<u32, String> {
    let n = raw
        .as_f64()
        .filter(|n| n.is_finite())
        .ok_or_else(|| format!("Dock `{name}` must be a number."))?;
    Ok(n.clamp(0.0, BADGE_MAX as f64).floor() as u32)
}

/// The body of `desk_set_status`, exactly as the console's `invoke` sends
/// it: `{ state, badge, needs?, event? }`. Unknown keys, states and events
/// are refused by name; counts are clamped to 0..9999.
pub fn parse_status(body: &Value) -> Result<Status, String> {
    let fields = body
        .as_object()
        .ok_or_else(|| "Dock status must be an object.".to_string())?;
    if let Some(extra) = fields
        .keys()
        .find(|k| !["state", "badge", "needs", "event"].contains(&k.as_str()))
    {
        return Err(format!("Unknown Dock status field `{}`.", cap(extra)));
    }
    let raw_state = fields
        .get("state")
        .and_then(Value::as_str)
        .ok_or_else(|| "Dock `state` must be a string.".to_string())?;
    let state = LogoState::parse(raw_state)
        .ok_or_else(|| format!("Unknown Dock state `{}`.", cap(raw_state)))?;
    let badge = count(
        fields
            .get("badge")
            .ok_or_else(|| "Dock `badge` is required.".to_string())?,
        "badge",
    )?;
    let needs = match fields.get("needs") {
        None | Some(Value::Null) => None,
        Some(raw) => Some(count(raw, "needs")?),
    };
    let event = match fields.get("event") {
        None | Some(Value::Null) => None,
        Some(raw) => {
            let raw = raw
                .as_str()
                .ok_or_else(|| "Dock `event` must be a string.".to_string())?;
            Some(OneShot::parse(raw).ok_or_else(|| format!("Unknown Dock event `{}`.", cap(raw)))?)
        }
    };
    Ok(Status {
        state,
        badge,
        needs,
        event,
    })
}

/// An echo of refused input stays short and printable.
fn cap(raw: &str) -> String {
    raw.chars().filter(|c| !c.is_control()).take(32).collect()
}

/// The Dock badge text: none at 0, "99+" above 99.
pub fn badge_label(count: u32) -> Option<String> {
    match count {
        0 => None,
        n if n > BADGE_SHOWN_MAX => Some(format!("{BADGE_SHOWN_MAX}+")),
        n => Some(n.to_string()),
    }
}

/// What the platform must do after a report. `None` fields are unchanged.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    /// `Some(None)` restores the bundle icon.
    pub tile: Option<Option<&'static Tile>>,
    pub badge: Option<Option<String>>,
    pub tray: Option<TrayMark>,
    pub animate: Option<OneShot>,
    pub bounce: bool,
}

impl Plan {
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        *self == Plan::default()
    }
}

/// The questions a report may need answered, asked only when they matter.
pub trait Host {
    fn low_power(&self) -> bool;
    fn frontmost(&self) -> bool;
    fn attention_enabled(&self) -> bool;
}

/// What is on screen and what was last reported.
#[derive(Debug, Default)]
pub struct Model {
    /// The latest report's state, provisional or not.
    latest: Option<LogoState>,
    /// The latest report that carried real counts: not `connecting` (no
    /// data yet) and not `disconnected` (the console lost its data). It is
    /// the baseline a bounce is measured against, and the badge.
    settled: Option<Status>,
    /// The shell's own poll found the bus unreachable.
    offline: bool,
    /// The console stopped reporting ([`HEARTBEAT`]).
    silent: bool,
    /// `Some(x)` once a tile was set; `x` is the tile's name or `None`
    /// for the bundle icon.
    shown_tile: Option<Option<&'static str>>,
    shown_badge: Option<Option<String>>,
    shown_tray: Option<TrayMark>,
    animating: bool,
    last_animation: Option<Instant>,
}

/// A report whose counts mean nothing yet.
fn provisional(state: LogoState) -> bool {
    matches!(state, LogoState::Connecting | LogoState::Disconnected)
}

impl Model {
    /// What the Dock shows. `connecting` is not an outage: it keeps the
    /// last settled state (the bundle icon at launch), so a load or reload
    /// never flashes the grey ring.
    pub fn effective(&self) -> LogoState {
        if self.offline || self.silent {
            return LogoState::Disconnected;
        }
        match self.latest {
            None => LogoState::Idle,
            Some(LogoState::Connecting) => self.settled.map(|s| s.state).unwrap_or(LogoState::Idle),
            Some(state) => state,
        }
    }

    fn tile_change(&mut self) -> Option<Option<&'static Tile>> {
        if self.animating {
            return None;
        }
        let tile = self.effective().tile();
        let name = tile.map(|t| t.name);
        if self.shown_tile == Some(name) {
            return None;
        }
        self.shown_tile = Some(name);
        Some(tile)
    }

    fn badge_and_tray(&mut self, plan: &mut Plan) {
        let label = badge_label(self.settled.map(|s| s.badge).unwrap_or(0));
        if self.shown_badge.as_ref() != Some(&label) {
            self.shown_badge = Some(label.clone());
            plan.badge = Some(label);
        }
        let tray = self.effective().tray();
        if self.shown_tray != Some(tray) {
            self.shown_tray = Some(tray);
            plan.tray = Some(tray);
        }
    }

    /// A report from the console.
    pub fn report(&mut self, status: Status, now: Instant, host: &dyn Host) -> Plan {
        // Any report is the heartbeat coming back.
        self.silent = false;
        self.latest = Some(status.state);
        let rose = if provisional(status.state) {
            false
        } else {
            match self.settled.replace(status) {
                // The first settled report describes what already waits.
                None => false,
                Some(prev) => match (prev.needs, status.needs) {
                    (Some(before), Some(after)) => after > before,
                    _ => {
                        let needs =
                            |s: LogoState| matches!(s, LogoState::Needs | LogoState::Blocked);
                        needs(status.state) && !needs(prev.state)
                    }
                },
            }
        };
        let bounce = rose && !host.frontmost() && host.attention_enabled();
        let mut plan = Plan {
            bounce,
            ..Plan::default()
        };
        if let Some(event) = status.event {
            let coalesced = self.animating
                || self
                    .last_animation
                    .is_some_and(|at| now.duration_since(at) < ANIMATION_GAP);
            let outage = self.effective() == LogoState::Disconnected;
            if !coalesced && !outage && !host.low_power() {
                self.animating = true;
                self.last_animation = Some(now);
                plan.animate = Some(event);
            }
        }
        plan.tile = self.tile_change();
        self.badge_and_tray(&mut plan);
        plan
    }

    /// The shell's poll saw the bus go away or come back.
    pub fn set_offline(&mut self, offline: bool) -> Plan {
        if self.offline == offline && self.shown_tray.is_some() {
            return Plan::default();
        }
        self.offline = offline;
        let mut plan = Plan {
            tile: self.tile_change(),
            ..Plan::default()
        };
        self.badge_and_tray(&mut plan);
        plan
    }

    /// The console's heartbeat was lost: the outage ring, the last badge.
    pub fn set_silent(&mut self, silent: bool) -> Plan {
        if self.silent == silent {
            return Plan::default();
        }
        self.silent = silent;
        let mut plan = Plan {
            tile: self.tile_change(),
            ..Plan::default()
        };
        self.badge_and_tray(&mut plan);
        plan
    }

    /// The animation ended: the tile for whatever the state is now.
    pub fn animation_done(&mut self) -> Option<&'static Tile> {
        self.animating = false;
        let tile = self.effective().tile();
        self.shown_tile = Some(tile.map(|t| t.name));
        tile
    }
}

/// The model behind a lock, as Tauri state.
#[derive(Default)]
pub struct Dock(pub Mutex<Model>);

impl Dock {
    pub fn lock(&self) -> std::sync::MutexGuard<'_, Model> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        low_power: bool,
        frontmost: bool,
        attention: bool,
    }

    impl Host for Fake {
        fn low_power(&self) -> bool {
            self.low_power
        }
        fn frontmost(&self) -> bool {
            self.frontmost
        }
        fn attention_enabled(&self) -> bool {
            self.attention
        }
    }

    const AWAY: Fake = Fake {
        low_power: false,
        frontmost: false,
        attention: true,
    };

    use serde_json::json;

    /// The body the console's `invoke("desk_set_status", …)` sends.
    fn body(state: &str, badge: i64) -> Value {
        json!({ "state": state, "badge": badge })
    }

    fn status(state: LogoState, badge: u32, needs: Option<u32>) -> Status {
        Status {
            state,
            badge,
            needs,
            event: None,
        }
    }

    #[test]
    fn unknown_states_and_events_are_refused_by_name() {
        let err = parse_status(&body("asleep", 0)).unwrap_err();
        assert!(err.contains("asleep"), "{err}");
        let bad = json!({ "state": "idle", "badge": 0, "event": "fiesta" });
        assert!(parse_status(&bad).unwrap_err().contains("fiesta"));
        // The echo is capped and printable.
        let long = parse_status(&body(&format!("{}\n", "x".repeat(200)), 0)).unwrap_err();
        assert!(long.len() < 80 && !long.contains('\n'), "{long}");
        for state in LogoState::ALL {
            assert_eq!(parse_status(&body(state.as_str(), 1)).unwrap().state, state);
        }
        let ok = json!({ "state": "working", "badge": 0, "event": "verified" });
        assert_eq!(parse_status(&ok).unwrap().event, Some(OneShot::Verified));
        let nulls = json!({ "state": "idle", "badge": 0, "needs": null, "event": null });
        assert_eq!(parse_status(&nulls).unwrap().event, None);
    }

    #[test]
    fn unknown_fields_and_wrong_types_are_refused() {
        let extra = json!({ "state": "idle", "badge": 0, "colour": "red" });
        assert!(parse_status(&extra).unwrap_err().contains("colour"));
        for bad in [
            json!({ "state": "idle" }),
            json!({ "state": "idle", "badge": "3" }),
            json!({ "state": 3, "badge": 0 }),
            json!({ "state": "idle", "badge": 0, "event": 1 }),
            json!(["idle", 0]),
        ] {
            assert!(parse_status(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn counts_are_clamped_to_0_through_9999() {
        assert_eq!(parse_status(&body("idle", -4)).unwrap().badge, 0);
        assert_eq!(parse_status(&body("idle", 12_000)).unwrap().badge, 9_999);
        assert_eq!(parse_status(&body("idle", i64::MAX)).unwrap().badge, 9_999);
        let needs = json!({ "state": "needs", "badge": 3, "needs": -1 });
        assert_eq!(parse_status(&needs).unwrap().needs, Some(0));
        let half = json!({ "state": "needs", "badge": 2.5 });
        assert_eq!(parse_status(&half).unwrap().badge, 2);
    }

    #[test]
    fn the_badge_hides_at_zero_and_caps_at_99_plus() {
        assert_eq!(badge_label(0), None);
        assert_eq!(badge_label(1).as_deref(), Some("1"));
        assert_eq!(badge_label(99).as_deref(), Some("99"));
        assert_eq!(badge_label(100).as_deref(), Some("99+"));
        assert_eq!(badge_label(9_999).as_deref(), Some("99+"));
    }

    #[test]
    fn every_state_maps_to_its_tile_and_tray_shape() {
        let name = |s: LogoState| s.tile().map(|t| t.name);
        assert_eq!(name(LogoState::Idle), None, "rest is the bundle icon");
        assert_eq!(name(LogoState::Working), Some("working"));
        assert_eq!(name(LogoState::Needs), Some("needs"));
        assert_eq!(name(LogoState::Blocked), Some("blocked"));
        assert_eq!(name(LogoState::Connecting), Some("disconnected"));
        assert_eq!(name(LogoState::Disconnected), Some("disconnected"));
        assert_eq!(format!("{:?}", NEEDS), "Tile(needs)", "logs carry no bytes");
        assert_eq!(LogoState::Idle.tray(), TrayMark::Dot);
        assert_eq!(LogoState::Working.tray(), TrayMark::Dot);
        assert_eq!(LogoState::Needs.tray(), TrayMark::Alert);
        assert_eq!(LogoState::Blocked.tray(), TrayMark::Alert);
        assert_eq!(LogoState::Connecting.tray(), TrayMark::Ring);
        assert_eq!(LogoState::Disconnected.tray(), TrayMark::Ring);
    }

    fn png_size(png: &[u8]) -> (u32, u32) {
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let be = |at: usize| u32::from_be_bytes(png[at..at + 4].try_into().unwrap());
        (be(16), be(20))
    }

    #[test]
    fn every_embedded_image_is_a_png_of_its_size() {
        let mut tiles: Vec<&Tile> = LogoState::ALL.iter().filter_map(|s| s.tile()).collect();
        tiles.extend(MESSAGE.iter().chain(VERIFIED.iter()));
        for tile in tiles {
            assert_eq!(png_size(tile.png), (512, 512), "{}", tile.name);
        }
        for mark in [TrayMark::Dot, TrayMark::Ring, TrayMark::Alert] {
            assert_eq!(png_size(mark.png()), (44, 44));
        }
        assert_ne!(TrayMark::Dot.png(), TrayMark::Ring.png());
        assert_ne!(TrayMark::Ring.png(), TrayMark::Alert.png());
    }

    #[test]
    fn animations_are_8_to_12_frames_and_under_a_second() {
        for event in [OneShot::Message, OneShot::Verified] {
            let n = event.frames().len();
            assert!((8..=12).contains(&n), "{event:?} has {n} frames");
            assert!(FRAME * n as u32 <= Duration::from_secs(1));
        }
    }

    #[test]
    fn the_first_report_sets_everything_and_a_repeat_sets_nothing() {
        let mut m = Model::default();
        let now = Instant::now();
        let plan = m.report(status(LogoState::Needs, 3, Some(2)), now, &AWAY);
        assert_eq!(plan.tile, Some(Some(&NEEDS)));
        assert_eq!(plan.badge, Some(Some("3".into())));
        assert_eq!(plan.tray, Some(TrayMark::Alert));
        assert!(!plan.bounce, "what waits at launch is not new");
        assert!(m
            .report(status(LogoState::Needs, 3, Some(2)), now, &AWAY)
            .is_empty());
    }

    #[test]
    fn rest_restores_the_bundle_icon() {
        let mut m = Model::default();
        let now = Instant::now();
        m.report(status(LogoState::Blocked, 1, Some(1)), now, &AWAY);
        let plan = m.report(status(LogoState::Idle, 0, Some(0)), now, &AWAY);
        assert_eq!(plan.tile, Some(None));
        assert_eq!(plan.badge, Some(None));
        assert_eq!(plan.tray, Some(TrayMark::Dot));
    }

    #[test]
    fn a_new_te_needs_item_bounces_once_when_harbard_is_not_frontmost() {
        let now = Instant::now();
        let mut m = Model::default();
        m.report(status(LogoState::Working, 0, Some(0)), now, &AWAY);
        assert!(
            m.report(status(LogoState::Needs, 1, Some(1)), now, &AWAY)
                .bounce
        );
        assert!(
            !m.report(status(LogoState::Needs, 1, Some(1)), now, &AWAY)
                .bounce
        );
        // An unread Directo raises the badge, not the needs: no bounce.
        assert!(
            !m.report(status(LogoState::Needs, 2, Some(1)), now, &AWAY)
                .bounce
        );
        assert!(
            m.report(status(LogoState::Blocked, 3, Some(2)), now, &AWAY)
                .bounce
        );
        // Fewer needs never bounce.
        assert!(
            !m.report(status(LogoState::Needs, 1, Some(1)), now, &AWAY)
                .bounce
        );

        let front = Fake {
            frontmost: true,
            ..AWAY
        };
        assert!(
            !m.report(status(LogoState::Needs, 3, Some(3)), now, &front)
                .bounce
        );
        let off = Fake {
            attention: false,
            ..AWAY
        };
        assert!(
            !m.report(status(LogoState::Needs, 4, Some(4)), now, &off)
                .bounce
        );
    }

    #[test]
    fn without_counts_entering_needs_bounces() {
        let now = Instant::now();
        let mut m = Model::default();
        m.report(status(LogoState::Idle, 0, None), now, &AWAY);
        assert!(
            m.report(status(LogoState::Needs, 1, None), now, &AWAY)
                .bounce
        );
        assert!(
            !m.report(status(LogoState::Blocked, 2, None), now, &AWAY)
                .bounce
        );
    }

    #[test]
    fn bursts_coalesce_to_one_animation_every_three_seconds() {
        let t0 = Instant::now();
        let mut m = Model::default();
        let mut with = |event, at| {
            let mut s = status(LogoState::Working, 0, None);
            s.event = Some(event);
            m.report(s, at, &AWAY)
        };
        let first = with(OneShot::Message, t0);
        assert_eq!(first.animate, Some(OneShot::Message));
        assert_eq!(first.tile, None, "the animation owns the tile");
        assert_eq!(
            with(OneShot::Message, t0 + Duration::from_millis(500)).animate,
            None
        );
        m.animation_done();
        let mut s = status(LogoState::Working, 0, None);
        s.event = Some(OneShot::Verified);
        assert_eq!(
            m.report(s, t0 + Duration::from_millis(2_900), &AWAY)
                .animate,
            None
        );
        assert_eq!(
            m.report(s, t0 + Duration::from_millis(3_100), &AWAY)
                .animate,
            Some(OneShot::Verified)
        );
    }

    #[test]
    fn low_power_and_no_bus_skip_the_animation_but_keep_the_state() {
        let now = Instant::now();
        let mut m = Model::default();
        let saver = Fake {
            low_power: true,
            ..AWAY
        };
        let mut s = status(LogoState::Blocked, 1, None);
        s.event = Some(OneShot::Message);
        let plan = m.report(s, now, &saver);
        assert_eq!(plan.animate, None);
        assert_eq!(plan.tile, Some(Some(&BLOCKED)));
        m.set_offline(true);
        s.state = LogoState::Working;
        assert_eq!(m.report(s, now + ANIMATION_GAP * 2, &AWAY).animate, None);
    }

    #[test]
    fn a_state_change_during_an_animation_lands_when_it_ends() {
        let now = Instant::now();
        let mut m = Model::default();
        let mut s = status(LogoState::Working, 0, None);
        s.event = Some(OneShot::Message);
        m.report(s, now, &AWAY);
        let plan = m.report(status(LogoState::Blocked, 1, None), now, &AWAY);
        assert_eq!(plan.tile, None);
        assert_eq!(plan.tray, Some(TrayMark::Alert), "the tray does not wait");
        assert_eq!(m.animation_done(), Some(&BLOCKED));
        assert!(m
            .report(status(LogoState::Blocked, 1, None), now, &AWAY)
            .is_empty());
    }

    #[test]
    fn an_unreachable_bus_overrides_the_reported_state_until_it_returns() {
        let now = Instant::now();
        let mut m = Model::default();
        m.report(status(LogoState::Needs, 2, Some(2)), now, &AWAY);
        let down = m.set_offline(true);
        assert_eq!(down.tile, Some(Some(&DISCONNECTED)));
        assert_eq!(down.tray, Some(TrayMark::Ring));
        assert_eq!(down.badge, None, "the last count stays on the badge");
        assert!(m.set_offline(true).is_empty());
        assert!(
            m.report(status(LogoState::Needs, 2, Some(2)), now, &AWAY)
                .tile
                .is_none(),
            "a stale console does not paint over the outage"
        );
        let up = m.set_offline(false);
        assert_eq!(up.tile, Some(Some(&NEEDS)));
        assert_eq!(up.tray, Some(TrayMark::Alert));
    }

    #[test]
    fn with_no_console_the_shell_still_shows_rest_or_the_outage() {
        let mut m = Model::default();
        let plan = m.set_offline(false);
        assert_eq!(plan.tile, Some(None));
        assert_eq!(plan.tray, Some(TrayMark::Dot));
        assert_eq!(m.set_offline(true).tile, Some(Some(&DISCONNECTED)));
    }

    // A provisional report (no data yet, or no bus) is no baseline.
    #[test]
    fn a_launch_that_reports_connecting_first_does_not_bounce_for_old_items() {
        let now = Instant::now();
        let mut m = Model::default();
        assert!(
            !m.report(status(LogoState::Connecting, 0, Some(0)), now, &AWAY)
                .bounce
        );
        let plan = m.report(status(LogoState::Needs, 3, Some(3)), now, &AWAY);
        assert!(!plan.bounce, "three items already waited at launch");
        assert!(
            m.report(status(LogoState::Needs, 4, Some(4)), now, &AWAY)
                .bounce
        );
    }

    #[test]
    fn a_reconnect_does_not_bounce_for_the_same_items_but_does_for_new_ones() {
        let now = Instant::now();
        let mut m = Model::default();
        m.report(status(LogoState::Needs, 2, Some(2)), now, &AWAY);
        m.report(status(LogoState::Disconnected, 0, Some(0)), now, &AWAY);
        assert!(
            !m.report(status(LogoState::Needs, 2, Some(2)), now, &AWAY)
                .bounce
        );
        m.report(status(LogoState::Connecting, 0, Some(0)), now, &AWAY);
        assert!(
            m.report(status(LogoState::Needs, 3, Some(3)), now, &AWAY)
                .bounce,
            "one is new"
        );
    }

    #[test]
    fn a_provisional_report_keeps_the_last_settled_badge() {
        let now = Instant::now();
        let mut m = Model::default();
        m.report(status(LogoState::Needs, 5, Some(2)), now, &AWAY);
        let plan = m.report(status(LogoState::Disconnected, 0, Some(0)), now, &AWAY);
        assert_eq!(
            plan.badge, None,
            "zeros from a console with no data are not counts"
        );
        assert_eq!(plan.tile, Some(Some(&DISCONNECTED)));
    }

    // Conectando is not an outage; no grey ring on every load.
    #[test]
    fn connecting_keeps_what_is_shown_instead_of_flashing_the_outage_ring() {
        let now = Instant::now();
        let mut m = Model::default();
        let first = m.report(status(LogoState::Connecting, 0, None), now, &AWAY);
        assert_eq!(first.tile, Some(None), "at launch: the bundle icon");
        assert_eq!(first.tray, Some(TrayMark::Dot));
        m.report(status(LogoState::Blocked, 1, Some(1)), now, &AWAY);
        let reload = m.report(status(LogoState::Connecting, 0, Some(0)), now, &AWAY);
        assert_eq!(reload.tile, None, "a reload keeps the red tile");
        assert_eq!(reload.tray, None);
    }

    // A huge badge is clamped, not refused, on the real path.
    #[test]
    fn huge_or_fractional_counts_are_clamped_not_refused() {
        for raw in [
            r#"{"state":"idle","badge":9223372036854775808}"#,
            r#"{"state":"idle","badge":1e21}"#,
        ] {
            let value: Value = serde_json::from_str(raw).unwrap();
            assert_eq!(parse_status(&value).map(|s| s.badge), Ok(9_999), "{raw}");
        }
    }

    // Heartbeat: a console that stops reporting must not leave stale counts
    // looking live.
    #[test]
    fn a_silent_console_shows_the_outage_ring_and_keeps_the_last_badge() {
        let t0 = Instant::now();
        let mut m = Model::default();
        assert!(!heartbeat_expired(None, t0), "armed only by a first report");
        m.report(status(LogoState::Needs, 4, Some(2)), t0, &AWAY);
        let last = Some(t0);
        assert!(!heartbeat_expired(last, t0 + Duration::from_secs(59)));
        assert!(heartbeat_expired(last, t0 + HEARTBEAT));
        let silent = m.set_silent(true);
        assert_eq!(silent.tile, Some(Some(&DISCONNECTED)));
        assert_eq!(silent.tray, Some(TrayMark::Ring));
        assert_eq!(silent.badge, None, "the last badge stays");
        assert!(m.set_silent(true).is_empty());
        // The next call recovers.
        let back = m.report(
            status(LogoState::Needs, 4, Some(2)),
            t0 + HEARTBEAT * 2,
            &AWAY,
        );
        assert_eq!(back.tile, Some(Some(&NEEDS)));
        assert_eq!(back.tray, Some(TrayMark::Alert));
        assert!(!back.bounce, "the same items, not new ones");
    }
}
