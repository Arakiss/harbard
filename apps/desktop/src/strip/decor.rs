//! The season on the Strip (`season-decor.tsx` `StripDecor`, strip.css): snow
//! over the band, behind the mark and the title, and a string of 64 lights
//! hanging along the top edge of the sheet, over the panels' tops.
//!
//! The Strip is a window nobody focuses, on a screen left on all day, so it
//! cannot wait for input like the main window: the snow falls in a flurry,
//! the first 40 seconds of every five minutes by the wall clock, and settles
//! in between with the lights lit and still. Dimmed for the night, or with
//! motion reduced, nothing falls and the lights stand still.
//!
//! One timer ([`Flurry`]) runs at the console's ten steps a second only
//! during a flurry and otherwise wakes every couple of seconds to look at
//! the clock; it notifies the two layer views, never the Strip around them.

use super::model::STAGE_W;
use crate::theme::Theme;
use gpui::{
    App, AppContext, Bounds, Context, Corners, Entity, Hsla, IntoElement, ParentElement,
    PathBuilder, Pixels, Point, Render, Styled, Task, Window, canvas, div, fill, point, px, size,
};
use std::time::Duration;

/// Snow falls the first 40 s of every five minutes of the wall clock.
pub const FLURRY_EVERY_MS: u64 = 5 * 60_000;
pub const FLURRY_MS: u64 = 40_000;
/// Ten steps a second, as the console's `steps()`.
const STEP_MS: u64 = 100;
/// Between flurries the timer only looks at the clock this often.
const IDLE_LOOK: Duration = Duration::from_secs(2);

/// Whether the Strip's snow is falling at `wall_ms`.
pub fn in_flurry(wall_ms: u64) -> bool {
    wall_ms % FLURRY_EVERY_MS < FLURRY_MS
}

/// The decorations' state for both layers.
pub struct Flurry {
    /// Moving: a flurry, not dimmed, motion not reduced, decorations on.
    pub on: bool,
    /// Steps of 100 ms on the wall clock.
    pub step: u64,
    dim: bool,
    _ticker: Task<()>,
}

impl Flurry {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let ticker = cx.spawn(async move |this, cx| {
            loop {
                let Ok(wait) = this.update(cx, |f, cx| f.tick(cx)) else {
                    break;
                };
                cx.background_executor().timer(wait).await;
            }
        });
        Flurry {
            on: false,
            step: 0,
            dim: false,
            _ticker: ticker,
        }
    }

    /// The night dim, from the Strip.
    pub fn set_dim(&mut self, dim: bool, cx: &mut Context<Self>) {
        if dim != self.dim {
            self.dim = dim;
            self.tick(cx);
        }
    }

    /// Look at the clock; notify the layers when they look different; how
    /// long until the next look.
    fn tick(&mut self, cx: &mut Context<Self>) -> Duration {
        let wall = harbard_core::clock::now_ms();
        let on = cx.global::<Theme>().decorations
            && !self.dim
            && !crate::robot::clock::reduced(cx)
            && in_flurry(wall);
        let step = wall / STEP_MS;
        if on != self.on || (on && step != self.step) {
            self.on = on;
            self.step = step;
            cx.notify();
        }
        if on {
            Duration::from_millis(STEP_MS - wall % STEP_MS)
        } else {
            let to_next = FLURRY_EVERY_MS - wall % FLURRY_EVERY_MS;
            IDLE_LOOK.min(Duration::from_millis(to_next))
        }
    }
}

/// Which part of the decorations a layer draws.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// Under the band's mark and title.
    Snow,
    /// Over the panels' top edge.
    Lights,
}

/// One layer, laid over the whole stage; it scales itself to the stage.
pub struct Layer {
    part: Part,
    flurry: Entity<Flurry>,
}

impl Layer {
    pub fn new(part: Part, flurry: Entity<Flurry>, cx: &mut Context<Self>) -> Self {
        cx.observe(&flurry, |_, _, cx| cx.notify()).detach();
        cx.observe_global::<Theme>(|_, cx| cx.notify()).detach();
        Layer { part, flurry }
    }
}

/// Both layers and their clock, made once by the Strip.
pub fn layers(cx: &mut App) -> (Entity<Flurry>, Entity<Layer>, Entity<Layer>) {
    let flurry = cx.new(Flurry::new);
    let snow = cx.new(|cx| Layer::new(Part::Snow, flurry.clone(), cx));
    let lights = cx.new(|cx| Layer::new(Part::Lights, flurry.clone(), cx));
    (flurry, snow, lights)
}

impl Render for Layer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.global::<Theme>();
        let layer = div().absolute().top_0().left_0().size_full();
        if !theme.decorations {
            return layer;
        }
        let f = self.flurry.read(cx);
        let (on, step) = (f.on, f.step);
        match self.part {
            Part::Snow => {
                if !on {
                    return layer;
                }
                let color = theme.c("festive-snow");
                layer.child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, _, window, _| paint_snow(bounds, step, color, window),
                    )
                    .size_full(),
                )
            }
            Part::Lights => {
                let tones = [1, 2, 3, 4].map(|i| theme.c(&format!("festive-bulb-{i}")));
                let wire = theme.c("festive-wire");
                let (a, b) = if on {
                    (twinkle(step), twinkle(step + 16))
                } else {
                    (1., 1.)
                };
                layer.child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, _, window, _| {
                            paint_lights(bounds, [a, b], tones, wire, window)
                        },
                    )
                    .size_full(),
                )
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Snow over the band (strip.css `.hb-festive-snow[data-where="strip"]`)
// ---------------------------------------------------------------------------

/// The band's height and the snow's tile, in stage pixels.
const BAND: f32 = 96.;

/// One tiled layer: tile width, flakes (x, y as a share of the tile, radius),
/// opacity, and its loop as (steps, delay in steps).
struct SnowLayer {
    width: f32,
    flakes: &'static [(f32, f32, f32)],
    opacity: f32,
    steps: u64,
    delay: u64,
}

/// Larger than the main window's: the stage is seen from across the desk.
const SNOW: [SnowLayer; 3] = [
    SnowLayer {
        width: 230.,
        flakes: &[(24., 0.12, 2.4), (100., 0.52, 2.4), (176., 0.82, 2.4)],
        opacity: 0.55,
        steps: 90,
        delay: 0,
    },
    SnowLayer {
        width: 310.,
        flakes: &[(60., 0.30, 3.4), (220., 0.70, 3.4)],
        opacity: 0.75,
        steps: 70,
        delay: 30,
    },
    SnowLayer {
        width: 410.,
        flakes: &[(90., 0.55, 4.4), (300., 0.15, 4.0)],
        opacity: 0.9,
        steps: 50,
        delay: 20,
    },
];

/// The flakes at `step` over a band `width` stage pixels wide, as (x, y,
/// radius, opacity) in stage pixels. Each layer slides down one tile and
/// starts over, so the loop has no seam.
pub fn snow_flakes(width: f32, step: u64) -> Vec<(f32, f32, f32, f32)> {
    let mut out = Vec::new();
    for l in &SNOW {
        let offset = ((step + l.delay) % l.steps) as f32 / l.steps as f32 * BAND;
        let cols = (width / l.width).ceil() as i32;
        for row in 0..2 {
            for col in 0..cols {
                for &(fx, fy, r) in l.flakes {
                    let x = col as f32 * l.width + fx;
                    let y = (row - 1) as f32 * BAND + fy * BAND + offset;
                    if x - r > width || y + r < 0. || y - r > BAND {
                        continue;
                    }
                    out.push((x, y, r, l.opacity));
                }
            }
        }
    }
    out
}

fn stage_k(bounds: Bounds<Pixels>) -> f32 {
    f32::from(bounds.size.width) / STAGE_W
}

fn at(bounds: Bounds<Pixels>, k: f32, x: f32, y: f32) -> Point<Pixels> {
    point(bounds.origin.x + px(x * k), bounds.origin.y + px(y * k))
}

fn paint_snow(bounds: Bounds<Pixels>, step: u64, color: Hsla, window: &mut Window) {
    let k = stage_k(bounds);
    let band = Bounds::new(bounds.origin, size(bounds.size.width, px(BAND * k)));
    window.with_content_mask(Some(gpui::ContentMask { bounds: band }), |window| {
        for (x, y, r, a) in snow_flakes(STAGE_W, step) {
            let c = at(bounds, k, x, y);
            let r = r * k;
            let b = Bounds::new(
                point(c.x - px(r), c.y - px(r)),
                size(px(r * 2.), px(r * 2.)),
            );
            window.paint_quad(fill(b, color.opacity(a)).corner_radii(px(r)));
        }
    });
}

// ---------------------------------------------------------------------------
// The string of lights (strip.css `.hb-festive-lights[data-where="strip"]`)
// ---------------------------------------------------------------------------

/// The Strip's stage is 2560 wide: the main window's spacing takes 64 bulbs.
pub const BULBS: usize = 64;
/// The string's box: top, and the inset from each side.
const TOP: f32 = 92.;
const INSET: f32 = 40.;
/// The wire's box height and stroke: it sags this far in the middle.
const WIRE_H: f32 = 10.;
const WIRE_W: f32 = 2.5;
/// A bulb, its offset below the box's top, and its cap.
const BULB: (f32, f32) = (11., 16.);
const BULB_TOP: f32 = 8.;
const CAP: (f32, f32, f32) = (2., 7., 5.);

/// A row's opacity at `step`: 1 → .4 → 1 over 32 steps (`hb-festive-twinkle`).
pub fn twinkle(step: u64) -> f32 {
    let t = (step % 32) as f32 / 32.;
    let k = if t < 0.5 { t / 0.5 } else { (1. - t) / 0.5 };
    1. - 0.6 * k
}

/// The left edge of each bulb across the row (`justify-content: space-between`).
pub fn bulb_xs(left: f32, width: f32) -> Vec<f32> {
    let gap = ((width - BULBS as f32 * BULB.0) / (BULBS as f32 - 1.)).max(0.);
    (0..BULBS)
        .map(|i| left + i as f32 * (BULB.0 + gap))
        .collect()
}

fn paint_lights(
    bounds: Bounds<Pixels>,
    alpha: [f32; 2],
    tones: [Hsla; 4],
    wire: Hsla,
    window: &mut Window,
) {
    let k = stage_k(bounds);
    let (left, width) = (INSET, STAGE_W - 2. * INSET);
    // The wire: the bottom border of a box rounded 0 0 50% 50% / 0 0 100% 100%.
    let sag = WIRE_H - WIRE_W / 2.;
    let mut path = PathBuilder::stroke(px(WIRE_W * k));
    path.move_to(at(bounds, k, left, TOP));
    path.arc_to(
        point(px(width / 2. * k), px(sag * k)),
        px(0.),
        false,
        false,
        at(bounds, k, left + width, TOP),
    );
    if let Ok(p) = path.build() {
        window.paint_path(p, wire);
    }
    for (i, x) in bulb_xs(left, width).into_iter().enumerate() {
        let a = alpha[i % 2];
        let tone = tones[i % 4];
        let y = TOP + BULB_TOP;
        let bulb = Bounds::new(at(bounds, k, x, y), size(px(BULB.0 * k), px(BULB.1 * k)));
        // box-shadow 0 1px 5px: the glow stays 5 px, not scaled with the stage.
        window.paint_drop_shadows(
            bulb,
            Corners::all(px(BULB.0 * 0.45 * k)),
            &[gpui::BoxShadow {
                color: tone.opacity(a),
                offset: point(px(0.), px(1.)),
                blur_radius: px(5.),
                spread_radius: px(0.),
                inset: false,
            }],
        );
        window.paint_quad(fill(bulb, tone.opacity(a)).corner_radii(Corners {
            top_left: px(BULB.0 * 0.45 * k),
            top_right: px(BULB.0 * 0.45 * k),
            bottom_right: px(BULB.0 * 0.5 * k),
            bottom_left: px(BULB.0 * 0.5 * k),
        }));
        let cap = Bounds::new(
            at(bounds, k, x + CAP.0, y - CAP.2),
            size(px(CAP.1 * k), px(CAP.2 * k)),
        );
        window.paint_quad(fill(cap, wire.opacity(a)).corner_radii(px(k)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flurry_is_the_first_forty_seconds_of_every_five_minutes() {
        assert!(in_flurry(0));
        assert!(in_flurry(39_999));
        assert!(!in_flurry(40_000));
        assert!(!in_flurry(299_999));
        assert!(in_flurry(300_000 + 5_000));
    }

    #[test]
    fn sixty_four_bulbs_span_the_string() {
        let xs = bulb_xs(INSET, STAGE_W - 2. * INSET);
        assert_eq!(xs.len(), 64);
        assert_eq!(xs[0], INSET);
        assert!((xs[63] + BULB.0 - (STAGE_W - INSET)).abs() < 1e-3);
    }

    #[test]
    fn the_lights_hang_over_the_sheets_top_edge() {
        // The sheet starts at 96: the bulbs hang across its edge, not in the band.
        let bottom = TOP + BULB_TOP + BULB.1;
        assert!(TOP < 96. && bottom > 96. && bottom < 96. + 28.);
    }

    #[test]
    fn snow_stays_in_the_band_and_moves_each_step() {
        for step in [0, 7, 45, 89] {
            let flakes = snow_flakes(STAGE_W, step);
            assert!(!flakes.is_empty());
            for (x, y, r, _) in &flakes {
                assert!(*y + r >= 0. && *y - r <= BAND && *x - r <= STAGE_W);
            }
        }
        assert_ne!(snow_flakes(STAGE_W, 0), snow_flakes(STAGE_W, 1));
        // A layer's loop ends where it began.
        assert_eq!(snow_flakes(STAGE_W, 0), snow_flakes(STAGE_W, 90 * 70 * 50));
    }

    #[test]
    fn the_twinkle_dims_to_four_tenths_and_back() {
        assert_eq!(twinkle(0), 1.);
        assert!((twinkle(16) - 0.4).abs() < 1e-6);
        assert_eq!(twinkle(32), 1.);
    }
}
