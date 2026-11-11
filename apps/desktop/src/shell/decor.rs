//! The season's decorations (`season-decor.tsx`, workspace.css `.hb-festive*`):
//! snow falling over the frame (the title band and the rail's empty space,
//! never the panel where text is read), a string of lights inside the title
//! band above the panel, and in the Lobby a small sleigh that crosses the
//! header now and then.
//!
//! They never cross text or a control (`DESIGN.md` §3): what the frame
//! holds (the rail's buttons and labels, the update pill) reports its box
//! with [`keep_clear`]; no flake is drawn inside one, and the lights stop
//! short of the pill.
//!
//! Everything is painted on canvases, no element per flake or bulb, and moves
//! at the console's step rates: ten steps a second, driven by one timer
//! ([`DecorClock`]) that notifies only the decoration views. The decorations
//! move only while the window is in front and in use: two minutes without a
//! pointer, a key or a scroll, the window behind another, the season's
//! decorations off or motion reduced, and the timer stops. Settled, the
//! lights stay lit and still and the snow and the sleigh leave, so nothing is
//! drawn again until the next input.
//!
//! Two views: [`SeasonSnow`] goes under the title band and the rail (the
//! rail's buttons and the update pill stay above the snow), [`SeasonDecor`]
//! over everything for the wire and the bulbs, and [`SeasonSleigh`] in the
//! Lobby's hero.

use crate::desk::Desk;
use crate::theme::Theme;
use crate::ui::space;
use gpui::{
    App, AppContext, Bounds, Context, Corners, DispatchPhase, Entity, Global, Hsla, IntoElement,
    MouseDownEvent, MouseMoveEvent, ParentElement, PathBuilder, Pixels, Point, Render, ScrollEvent,
    SharedString, Styled, Task, Window, canvas, div, fill, point, px, size,
};
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

/// Two minutes without a pointer or a key settle the decorations.
const IDLE: Duration = Duration::from_secs(120);
/// Ten steps a second: the snow's, the twinkle's and the sleigh's cadence.
const STEP_MS: u64 = 100;
/// The title band and the rail's width.
const TOP: f32 = super::BAND;
const RAIL: f32 = super::RAIL;
/// The room a flake leaves around a kept-clear box.
const CLEARANCE: f32 = 3.;

thread_local! {
    /// The boxes in the frame that decorations stay out of, by who reported them.
    static KEPT: RefCell<HashMap<SharedString, Bounds<Pixels>>> = RefCell::new(HashMap::new());
}

/// An invisible layer that reports its parent's box as one the decorations
/// keep out of. Give it as a child to a button or a label in the frame.
pub fn keep_clear(id: impl Into<SharedString>) -> impl IntoElement {
    let id: SharedString = id.into();
    canvas(
        move |bounds, _, _| {
            KEPT.with(|k| k.borrow_mut().insert(id, bounds));
        },
        |_, _, _, _| (),
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// Forget the boxes whose id starts with `prefix` (they are about to be
/// reported again, or are gone).
pub fn forget_clear(prefix: &str) {
    KEPT.with(|k| k.borrow_mut().retain(|id, _| !id.starts_with(prefix)));
}

fn kept() -> Vec<Bounds<Pixels>> {
    KEPT.with(|k| k.borrow().values().copied().collect())
}

/// Whether a flake at `c` would touch one of `boxes`.
fn touches(c: Point<Pixels>, radius: f32, boxes: &[Bounds<Pixels>]) -> bool {
    let pad = px(radius + CLEARANCE);
    boxes.iter().any(|b| {
        c.x >= b.left() - pad
            && c.x <= b.right() + pad
            && c.y >= b.top() - pad
            && c.y <= b.bottom() + pad
    })
}

// ---------------------------------------------------------------------------
// The clock
// ---------------------------------------------------------------------------

/// The decorations' one timer, and whether they move at all.
pub struct DecorClock {
    /// Moving: the window in front, used lately, decorations on, motion not reduced.
    pub awake: bool,
    /// Motion reduced (the console's toggle): the lights stay, still; nothing falls or flies.
    pub reduce: bool,
    /// Steps of 100 ms since the clock started; the frame every view draws.
    pub step: u64,
    start: Instant,
    last_input: Rc<Cell<Instant>>,
    active: bool,
    enabled: bool,
    ticker: Option<Task<()>>,
}

#[derive(Clone)]
struct ClockHandle(Entity<DecorClock>);

impl Global for ClockHandle {}

impl DecorClock {
    /// The clock, made on first use for this window.
    pub fn get(window: &mut Window, cx: &mut App) -> Entity<DecorClock> {
        if let Some(h) = cx.try_global::<ClockHandle>() {
            return h.0.clone();
        }
        let clock = cx.new(|cx: &mut Context<DecorClock>| {
            cx.observe_window_activation(window, |c, window, cx| {
                c.active = window.is_window_active();
                c.check(cx);
            })
            .detach();
            cx.observe_global::<Theme>(|c, cx| {
                c.enabled = cx.global::<Theme>().decorations;
                c.check(cx);
            })
            .detach();
            cx.observe(&Desk::get(cx), |c, desk, cx| {
                let d = desk.read(cx);
                let motion = d
                    .activity
                    .as_ref()
                    .and_then(|a| a.motion.clone())
                    .or_else(|| d.view.as_ref().and_then(|v| v.motion.clone()));
                let reduce = motion.as_deref() == Some("reduce");
                if reduce != c.reduce {
                    c.reduce = reduce;
                    c.check(cx);
                    cx.notify();
                }
            })
            .detach();
            cx.observe_keystrokes(|c, _, _, cx| c.input(cx)).detach();
            let now = Instant::now();
            let mut c = DecorClock {
                awake: false,
                reduce: false,
                step: 0,
                start: now,
                last_input: Rc::new(Cell::new(now)),
                active: window.is_window_active(),
                enabled: cx.global::<Theme>().decorations,
                ticker: None,
            };
            c.check(cx);
            c
        });
        cx.set_global(ClockHandle(clock.clone()));
        clock
    }

    /// Input arrived: stamp it, and wake when settled.
    pub fn input(&mut self, cx: &mut Context<Self>) {
        self.last_input.set(Instant::now());
        if !self.awake {
            self.check(cx);
        }
    }

    fn should_move(&self) -> bool {
        self.enabled && self.active && !self.reduce && self.last_input.get().elapsed() < IDLE
    }

    /// Move or settle, and run the timer only while moving.
    fn check(&mut self, cx: &mut Context<Self>) {
        let awake = self.should_move();
        if awake == self.awake {
            return;
        }
        self.awake = awake;
        if awake {
            self.step = self.now_step();
            self.ticker = Some(cx.spawn(async move |this, cx| {
                loop {
                    let wait = {
                        let Ok(ms) = this.update(cx, |c, _| c.start.elapsed().as_millis() as u64)
                        else {
                            break;
                        };
                        STEP_MS - ms % STEP_MS
                    };
                    cx.background_executor()
                        .timer(Duration::from_millis(wait))
                        .await;
                    let go_on = this.update(cx, |c, cx| {
                        if !c.should_move() {
                            c.awake = false;
                            cx.notify();
                            return false;
                        }
                        c.step = c.now_step();
                        cx.notify();
                        true
                    });
                    if !matches!(go_on, Ok(true)) {
                        break;
                    }
                }
            }));
        } else {
            self.ticker = None;
        }
        cx.notify();
    }

    fn now_step(&self) -> u64 {
        self.start.elapsed().as_millis() as u64 / STEP_MS
    }
}

/// Wake the decorations on pointer and wheel input, from inside a paint.
/// Listens without taking the event; while moving it only stamps a time.
fn listen_for_input(clock: &Entity<DecorClock>, window: &mut Window, cx: &App) {
    let c = clock.read(cx);
    let stamp = c.last_input.clone();
    let awake = c.awake;
    let on = move |cx: &mut App, clock: &Entity<DecorClock>| {
        stamp.set(Instant::now());
        if !awake {
            clock.update(cx, |c, cx| c.input(cx));
        }
    };
    let on = Rc::new(on);
    {
        let (on, clock) = (on.clone(), clock.clone());
        window.on_mouse_event(move |_: &MouseMoveEvent, phase, _, cx| {
            if phase == DispatchPhase::Capture {
                on(cx, &clock);
            }
        });
    }
    {
        let (on, clock) = (on.clone(), clock.clone());
        window.on_mouse_event(move |_: &MouseDownEvent, phase, _, cx| {
            if phase == DispatchPhase::Capture {
                on(cx, &clock);
            }
        });
    }
    let clock = clock.clone();
    window.on_mouse_event(move |_: &ScrollEvent, phase, _, cx| {
        if phase == DispatchPhase::Capture {
            on(cx, &clock);
        }
    });
}

// ---------------------------------------------------------------------------
// Snow
// ---------------------------------------------------------------------------

/// One tiled layer of flakes: tile width, flakes (x, y as a fraction of the
/// tile's height), flake radius and opacity (`.hb-festive-snow-layer`).
struct SnowLayer {
    width: f32,
    flakes: &'static [(f32, f32)],
    radius: f32,
    opacity: f32,
}

const LAYERS: [SnowLayer; 3] = [
    SnowLayer {
        width: 140.,
        flakes: &[(12., 0.10), (58., 0.46), (104., 0.78)],
        radius: 1.5,
        opacity: 0.55,
    },
    SnowLayer {
        width: 110.,
        flakes: &[(30., 0.30), (84., 0.70)],
        radius: 2.0,
        opacity: 0.75,
    },
    SnowLayer {
        width: 170.,
        flakes: &[(44., 0.55), (128., 0.12)],
        radius: 2.45,
        opacity: 0.9,
    },
];

/// One band of snow: its tile height and each layer's (steps per loop, delay in steps).
struct Band {
    tile: f32,
    timing: [(u64, u64); 3],
}

/// The title band right of the rail: 6 s, 4.5 s (-3 s) and 3.5 s (-5 s).
const TOP_BAND: Band = Band {
    tile: 64.,
    timing: [(60, 0), (45, 30), (35, 50)],
};
/// The rail: 14 s, 10 s (-3 s) and 7 s (-5 s).
const RAIL_BAND: Band = Band {
    tile: 220.,
    timing: [(140, 0), (100, 30), (70, 50)],
};

/// The flakes of one band at `step`, as (centre, radius, opacity).
fn snow_flakes(band: &Band, area: Bounds<Pixels>, step: u64) -> Vec<(Point<Pixels>, f32, f32)> {
    let (x0, y0) = (f32::from(area.origin.x), f32::from(area.origin.y));
    let (w, h) = (f32::from(area.size.width), f32::from(area.size.height));
    let tile = band.tile;
    let mut out = Vec::new();
    for (layer, &(steps, delay)) in LAYERS.iter().zip(band.timing.iter()) {
        // Slides down one tile and starts over; the layer starts a tile above the band.
        let offset = ((step + delay) % steps) as f32 / steps as f32 * tile;
        let rows = (h / tile).ceil() as i32 + 1;
        let cols = (w / layer.width).ceil() as i32;
        for row in 0..rows {
            for col in 0..cols {
                for &(fx, fy) in layer.flakes {
                    let x = x0 + col as f32 * layer.width + fx;
                    let y = y0 - tile + row as f32 * tile + fy * tile + offset;
                    if x > x0 + w + 3. || y < y0 - 3. || y > y0 + h + 3. {
                        continue;
                    }
                    out.push((point(px(x), px(y)), layer.radius, layer.opacity));
                }
            }
        }
    }
    out
}

fn paint_flakes(
    area: Bounds<Pixels>,
    flakes: Vec<(Point<Pixels>, f32, f32)>,
    color: Hsla,
    window: &mut Window,
) {
    let boxes = kept();
    window.with_content_mask(Some(gpui::ContentMask { bounds: area }), |window| {
        for (c, r, a) in flakes {
            if touches(c, r, &boxes) {
                continue;
            }
            let b = Bounds::new(
                point(c.x - px(r), c.y - px(r)),
                size(px(r * 2.), px(r * 2.)),
            );
            window.paint_quad(fill(b, color.opacity(a)).corner_radii(px(r)));
        }
    });
}

/// The snow over the title band and the rail. Mount it under them: the
/// first child of the frame, absolute, so their buttons stay above it.
pub struct SeasonSnow {
    clock: Entity<DecorClock>,
}

impl SeasonSnow {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let clock = DecorClock::get(window, cx);
        cx.observe(&clock, |_, _, cx| cx.notify()).detach();
        SeasonSnow { clock }
    }
}

impl Render for SeasonSnow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let c = self.clock.read(cx);
        let layer = div().absolute().top_0().left_0().size_full();
        if !c.awake || c.reduce || !cx.global::<Theme>().decorations {
            return layer;
        }
        let step = c.step;
        let color = cx.global::<Theme>().c("festive-snow");
        layer.child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let o = bounds.origin;
                    let top = Bounds::new(
                        point(o.x + px(RAIL), o.y),
                        size(bounds.size.width - px(RAIL), px(TOP)),
                    );
                    let rail = Bounds::new(
                        point(o.x, o.y + px(TOP)),
                        size(px(RAIL), bounds.size.height - px(TOP)),
                    );
                    paint_flakes(top, snow_flakes(&TOP_BAND, top, step), color, window);
                    paint_flakes(rail, snow_flakes(&RAIL_BAND, rail, step), color, window);
                },
            )
            .size_full(),
        )
    }
}

// ---------------------------------------------------------------------------
// Lights
// ---------------------------------------------------------------------------

const BULBS: usize = 36;
/// A bulb, the cap that holds it on the wire, and how far its glow reaches.
const BULB: (f32, f32) = (6., 9.);
const CAP: f32 = 3.;
const GLOW: f32 = 3.5;
/// The string hangs inside the title band: its wire high enough for the
/// lowest glow to end above the panel, its ends clear of the rail and of the
/// window's edge (or of the update pill, when that shows).
const LIGHTS_TOP: f32 = TOP - space::XS - GLOW - BULB.1 - CAP - 2.;
const LIGHTS_LEFT: f32 = RAIL + space::SM;
const LIGHTS_RIGHT: f32 = space::LG;
/// The update pill's box, as it reports it.
pub const PILL: &str = "pill";

/// A row's opacity at `step`: 1 → .4 → 1 over 32 steps (`hb-festive-twinkle`).
fn twinkle(step: u64) -> f32 {
    let t = (step % 32) as f32 / 32.;
    let k = if t < 0.5 { t / 0.5 } else { (1. - t) / 0.5 };
    1. - 0.6 * k
}

/// The x of each bulb's left edge across the row (`justify-content: space-between`).
fn bulb_xs(left: f32, width: f32) -> Vec<f32> {
    let gap = ((width - BULBS as f32 * BULB.0) / (BULBS as f32 - 1.)).max(0.);
    (0..BULBS)
        .map(|i| left + i as f32 * (BULB.0 + gap))
        .collect()
}

/// The wire, the two rows of bulbs and their twinkle, inside the title band.
/// Also listens for the input that wakes the decorations.
pub struct SeasonDecor {
    clock: Entity<DecorClock>,
    snow: Entity<SeasonSnow>,
}

impl SeasonDecor {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let clock = DecorClock::get(window, cx);
        cx.observe(&clock, |_, _, cx| cx.notify()).detach();
        let snow = cx.new(|cx| SeasonSnow::new(window, cx));
        SeasonDecor { clock, snow }
    }

    /// The snow layer, for the frame to mount under the title band and the rail.
    pub fn snow(&self) -> Entity<SeasonSnow> {
        self.snow.clone()
    }
}

impl Render for SeasonDecor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let c = self.clock.read(cx);
        // Settled or reduced: lit and still.
        let (a, b) = if c.awake {
            (twinkle(c.step), twinkle(c.step + 16))
        } else {
            (1., 1.)
        };
        let tones = [
            theme.c("festive-bulb-1"),
            theme.c("festive-bulb-2"),
            theme.c("festive-bulb-3"),
            theme.c("festive-bulb-4"),
        ];
        let wire = theme.c("festive-wire");
        let clock = self.clock.clone();
        div().absolute().top_0().left_0().size_full().child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, cx| {
                    listen_for_input(&clock, window, cx);
                    let o = bounds.origin;
                    let left = f32::from(o.x) + LIGHTS_LEFT;
                    let pill = KEPT.with(|k| k.borrow().get(PILL).map(|b| f32::from(b.left())));
                    let right = pill.unwrap_or(f32::from(bounds.right())) - LIGHTS_RIGHT;
                    let width = (right - left).max(0.);
                    let top = f32::from(o.y) + LIGHTS_TOP;
                    // The wire sags between the two ends (border-radius 0 0 50% 50% / 0 0 100% 100%).
                    let mut path = PathBuilder::stroke(px(1.5));
                    path.move_to(point(px(left), px(top)));
                    path.arc_to(
                        point(px(width / 2.), px(4.25)),
                        px(0.),
                        false,
                        false,
                        point(px(left + width), px(top)),
                    );
                    if let Ok(p) = path.build() {
                        window.paint_path(p, wire);
                    }
                    let xs = bulb_xs(left, width);
                    for (set, alpha) in [(0usize, a), (1usize, b)] {
                        for (i, &x) in xs.iter().enumerate() {
                            if i % 2 != set {
                                continue;
                            }
                            let tone = tones[i % 4];
                            let y = top + CAP + 1.;
                            // The glow (box-shadow 0 1px 5px): two soft rings under the bulb.
                            for (grow, glow) in [(GLOW, 0.14_f32), (GLOW / 2., 0.24)] {
                                let g = Bounds::new(
                                    point(px(x - grow), px(y + 1. - grow)),
                                    size(px(BULB.0 + grow * 2.), px(BULB.1 + grow * 2.)),
                                );
                                window.paint_quad(
                                    fill(g, tone.opacity(glow * alpha)).corner_radii(px(3. + grow)),
                                );
                            }
                            let bulb =
                                Bounds::new(point(px(x), px(y)), size(px(BULB.0), px(BULB.1)));
                            window.paint_quad(fill(bulb, tone.opacity(alpha)).corner_radii(
                                Corners {
                                    top_left: px(2.7),
                                    top_right: px(2.7),
                                    bottom_right: px(3.),
                                    bottom_left: px(3.),
                                },
                            ));
                            // The cap that holds the bulb on the wire.
                            let cap = Bounds::new(
                                point(px(x + 1.), px(y - CAP)),
                                size(px(BULB.0 - 2.), px(CAP)),
                            );
                            window.paint_quad(fill(cap, wire.opacity(alpha)).corner_radii(px(1.)));
                        }
                    }
                },
            )
            .size_full(),
        )
    }
}

// ---------------------------------------------------------------------------
// The Lobby's sleigh
// ---------------------------------------------------------------------------

/// A tiny SVG path reader for the sleigh's drawing: M L H V C Z, absolute and
/// relative, with implicit repeats and numbers written tight ("1.4.6").
fn svg_path(d: &str, builder: &mut PathBuilder, scale: f32, at: Point<Pixels>) {
    let mut nums: Vec<f32> = Vec::new();
    let mut cmd = ' ';
    let (mut cur, mut start) = ((0f32, 0f32), (0f32, 0f32));
    let to = |p: (f32, f32)| point(at.x + px(p.0 * scale), at.y + px(p.1 * scale));
    let flush = |cmd: char,
                 nums: &mut Vec<f32>,
                 b: &mut PathBuilder,
                 cur: &mut (f32, f32),
                 start: &mut (f32, f32)| {
        let rel = cmd.is_ascii_lowercase();
        let base = |c: &(f32, f32)| if rel { *c } else { (0., 0.) };
        match cmd.to_ascii_uppercase() {
            'M' | 'L' => {
                for (k, pair) in nums.chunks(2).enumerate() {
                    if pair.len() < 2 {
                        break;
                    }
                    let o = base(cur);
                    *cur = (o.0 + pair[0], o.1 + pair[1]);
                    if cmd.eq_ignore_ascii_case(&'M') && k == 0 {
                        b.move_to(to(*cur));
                        *start = *cur;
                    } else {
                        b.line_to(to(*cur));
                    }
                }
            }
            'H' => {
                for &x in nums.iter() {
                    cur.0 = if rel { cur.0 + x } else { x };
                    b.line_to(to(*cur));
                }
            }
            'V' => {
                for &y in nums.iter() {
                    cur.1 = if rel { cur.1 + y } else { y };
                    b.line_to(to(*cur));
                }
            }
            'C' => {
                for six in nums.chunks(6) {
                    if six.len() < 6 {
                        break;
                    }
                    let o = base(cur);
                    let c1 = (o.0 + six[0], o.1 + six[1]);
                    let c2 = (o.0 + six[2], o.1 + six[3]);
                    let end = (o.0 + six[4], o.1 + six[5]);
                    b.cubic_bezier_to(to(end), to(c1), to(c2));
                    *cur = end;
                }
            }
            'Z' => {
                b.close();
                *cur = *start;
            }
            _ => {}
        }
        nums.clear();
    };
    let mut token = String::new();
    let push = |token: &mut String, nums: &mut Vec<f32>| {
        if !token.is_empty() {
            if let Ok(n) = token.parse() {
                nums.push(n);
            }
            token.clear();
        }
    };
    for ch in d.chars() {
        if ch.is_ascii_alphabetic() {
            push(&mut token, &mut nums);
            if cmd != ' ' {
                flush(cmd, &mut nums, builder, &mut cur, &mut start);
            }
            cmd = ch;
            if ch.eq_ignore_ascii_case(&'z') {
                flush(cmd, &mut nums, builder, &mut cur, &mut start);
                cmd = ' ';
            }
        } else if ch == '-' {
            push(&mut token, &mut nums);
            token.push(ch);
        } else if ch == '.' {
            if token.contains('.') {
                push(&mut token, &mut nums);
            }
            token.push(ch);
        } else if ch.is_ascii_digit() {
            token.push(ch);
        } else {
            push(&mut token, &mut nums);
        }
    }
    push(&mut token, &mut nums);
    if cmd != ' ' {
        flush(cmd, &mut nums, builder, &mut cur, &mut start);
    }
}

const DEER: [&str; 2] = [
    "M44 12h8l2-3 2 3h3l1-4 1.4.6-.8 4.4-2 3h-1l.6-2.6h-6.6l-1.4 2.6h-1l.8-2.6-2.6-.4z",
    "M30 12h8l2-3 2 3h3l1-4 1.4.6-.8 4.4-2 3h-1l.6-2.6h-6.6l-1.4 2.6h-1l.8-2.6-2.6-.4z",
];
const BODY: &str = "M4 8h14l4 3-2 4H6z";
const LINES: [&str; 4] = [
    "M57 8l-1-3m2 3 1-3.4m-2.4 1.2-1.6-.8m3.8.6 1.8-.8",
    "M43 8l-1-3m2 3 1-3.4m-2.4 1.2-1.6-.8m3.8.6 1.8-.8",
    "M31 13.5 22 11",
    "M2 17h19c2 0 3-1 3.6-2.4",
];

/// The sleigh drawn 86×24 (viewBox 72×20) with its top-left at `at`.
fn paint_sleigh(at: Point<Pixels>, ink: Hsla, nose: Hsla, sack: Hsla, window: &mut Window) {
    let s = 86. / 72.;
    let filled = |d: &str, color: Hsla, window: &mut Window| {
        let mut b = PathBuilder::fill();
        svg_path(d, &mut b, s, at);
        if let Ok(p) = b.build() {
            window.paint_path(p, color);
        }
    };
    for d in DEER {
        filled(d, ink, window);
    }
    filled(BODY, ink, window);
    for d in LINES {
        let mut b = PathBuilder::stroke(px(1.2 * s));
        svg_path(d, &mut b, s, at);
        if let Ok(p) = b.build() {
            window.paint_path(p, ink);
        }
    }
    let dot = |cx_: f32, cy: f32, r: f32, color: Hsla, window: &mut Window| {
        let b = Bounds::new(
            point(at.x + px((cx_ - r) * s), at.y + px((cy - r) * s)),
            size(px(2. * r * s), px(2. * r * s)),
        );
        window.paint_quad(fill(b, color).corner_radii(px(r * s)));
    };
    dot(10., 6., 3.4, sack, window);
    dot(62.6, 9.2, 1.3, nose, window);
}

/// Where the sleigh is `secs` into its 50 s loop (delay -42 s already added),
/// across a sky `width` wide: off stage until 86 %, then a seven-second
/// crossing with a gentle rise and fall (`hb-festive-sleigh`).
fn sleigh_at(secs: f32, width: f32) -> Option<(f32, f32)> {
    let p = secs / 50.;
    let keys = [
        (0.86, -120., 2.),
        (0.90, 0.3 * width - 30., 0.),
        (0.95, 0.7 * width - 60., 2.),
        (1.00, width + 10., 0.),
    ];
    if p < keys[0].0 {
        return None;
    }
    for w in keys.windows(2) {
        let (a, b) = (w[0], w[1]);
        if p <= b.0 {
            let k = (p - a.0) / (b.0 - a.0);
            return Some((a.1 + (b.1 - a.1) * k, a.2 + (b.2 - a.2) * k));
        }
    }
    None
}

/// The Lobby's visitor: a sleigh and two reindeer crossing the sky above the
/// hero's title once every 50 seconds, a second after the Lobby opens. Mount
/// it absolute at the hero's top (top 1, left 0, right 0); it is 26px tall,
/// draws nothing while the decorations are off or settled, and takes no input.
pub struct SeasonSleigh {
    clock: Entity<DecorClock>,
    start: Instant,
}

impl SeasonSleigh {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let clock = DecorClock::get(window, cx);
        cx.observe(&clock, |_, _, cx| cx.notify()).detach();
        SeasonSleigh {
            clock,
            start: Instant::now(),
        }
    }
}

impl Render for SeasonSleigh {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let c = self.clock.read(cx);
        let sky = div()
            .absolute()
            .top(px(1.))
            .left_0()
            .right_0()
            .h(px(26.))
            .overflow_hidden();
        if !c.awake || c.reduce || !theme.decorations {
            return sky;
        }
        // Stepped like the console: ten positions a second.
        let tenths = self.start.elapsed().as_millis() as u64 / STEP_MS;
        let secs = ((tenths + 420) % 500) as f32 / 10.;
        let (ink, nose, sack) = (
            theme.c("ink-3"),
            theme.c("festive-bulb-2"),
            theme.c("accent"),
        );
        sky.child(
            canvas(
                |_, _, _| (),
                move |bounds, _, window, _| {
                    let width = f32::from(bounds.size.width);
                    if let Some((x, y)) = sleigh_at(secs, width) {
                        let at = point(bounds.origin.x + px(x), bounds.origin.y + px(y));
                        window.with_content_mask(Some(gpui::ContentMask { bounds }), |window| {
                            paint_sleigh(at, ink, nose, sack, window);
                        });
                    }
                },
            )
            .size_full(),
        )
    }
}

/// The sleigh as an element for the Lobby hero: `season_sleigh(window, cx)`
/// once, then keep the entity and render it.
pub fn season_sleigh(window: &mut Window, cx: &mut App) -> Entity<SeasonSleigh> {
    cx.new(|cx| SeasonSleigh::new(window, cx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_twinkle_dims_to_four_tenths_and_back() {
        assert_eq!(twinkle(0), 1.);
        assert!((twinkle(16) - 0.4).abs() < 1e-6);
        assert_eq!(twinkle(32), 1.);
        // The second row runs half a loop behind.
        assert!((twinkle(16 + 16) - 1.).abs() < 1e-6);
    }

    #[test]
    fn bulbs_spread_across_the_row() {
        let xs = bulb_xs(84., 1344.);
        assert_eq!(xs.len(), 36);
        assert_eq!(xs[0], 84.);
        assert!((xs[35] + 6. - (84. + 1344.)).abs() < 1e-3);
    }

    #[test]
    fn the_sleigh_is_off_stage_most_of_the_loop() {
        assert_eq!(sleigh_at(0., 1000.), None);
        assert_eq!(sleigh_at(42., 1000.), None);
        assert_eq!(sleigh_at(43., 1000.), Some((-120., 2.)));
        let (x, _) = sleigh_at(49.99, 1000.).unwrap();
        assert!(x > 1000.);
    }

    #[test]
    fn the_lights_hang_inside_the_title_band() {
        // The lowest thing drawn is the glow under a bulb.
        let lowest = LIGHTS_TOP + CAP + 1. + 1. + BULB.1 + GLOW;
        assert!(lowest < TOP, "the glow ends at {lowest}, the band at {TOP}");
        const { assert!(LIGHTS_TOP > 0.) };
        const { assert!(LIGHTS_LEFT > RAIL) };
    }

    #[test]
    fn no_flake_touches_a_kept_box() {
        let label = Bounds::new(point(px(3.), px(100.)), size(px(72.), px(56.)));
        assert!(touches(point(px(40.), px(120.)), 2., &[label]));
        // Just outside its edge still counts: the flake has a size.
        assert!(touches(point(px(1.), px(120.)), 2., &[label]));
        assert!(!touches(point(px(40.), px(200.)), 2., &[label]));
        assert!(!touches(point(px(40.), px(120.)), 2., &[]));
    }

    #[test]
    fn snow_stays_inside_its_band() {
        let band = Bounds::new(point(px(78.), px(0.)), size(px(1000.), px(40.)));
        for step in [0, 13, 59] {
            for (c, _, _) in snow_flakes(&TOP_BAND, band, step) {
                assert!(f32::from(c.y) >= -3. && f32::from(c.y) <= 43.);
                assert!(f32::from(c.x) >= 78.);
            }
        }
        assert!(!snow_flakes(&TOP_BAND, band, 0).is_empty());
    }
}
