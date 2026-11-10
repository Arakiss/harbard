//! Small drawing kit for the faces and status icons: an affine map from an
//! SVG viewBox to the window, shapes painted with GPUI primitives, and the
//! CSS timing functions their keyframes use.

use gpui::{
    BoxShadow, Bounds, Corners, Hsla, PathBuilder, Pixels, Point, Window, fill, point, px, size,
};

/// `(x, y) → (a·x + c·y + e, b·x + d·y + f)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Affine {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
    pub e: f32,
    pub f: f32,
}

impl Affine {
    pub const ID: Affine = Affine { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };

    pub fn translate(x: f32, y: f32) -> Affine {
        Affine { e: x, f: y, ..Affine::ID }
    }

    pub fn scale(sx: f32, sy: f32) -> Affine {
        Affine { a: sx, d: sy, ..Affine::ID }
    }

    /// Scale about a point.
    pub fn scale_at(sx: f32, sy: f32, cx: f32, cy: f32) -> Affine {
        Affine::translate(cx, cy).then(Affine::scale(sx, sy)).then(Affine::translate(-cx, -cy))
    }

    /// Rotate by `deg` degrees (clockwise on screen, as SVG) about a point.
    pub fn rotate_at(deg: f32, cx: f32, cy: f32) -> Affine {
        let (s, c) = deg.to_radians().sin_cos();
        let r = Affine { a: c, b: s, c: -s, d: c, e: 0.0, f: 0.0 };
        Affine::translate(cx, cy).then(r).then(Affine::translate(-cx, -cy))
    }

    /// `self ∘ inner`: apply `inner` first, then `self` (SVG nesting order).
    pub fn then(self, inner: Affine) -> Affine {
        Affine {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            e: self.a * inner.e + self.c * inner.f + self.e,
            f: self.b * inner.e + self.d * inner.f + self.f,
        }
    }

    pub fn apply(&self, x: f32, y: f32) -> (f32, f32) {
        (self.a * x + self.c * y + self.e, self.b * x + self.d * y + self.f)
    }

    pub fn point(&self, x: f32, y: f32) -> Point<Pixels> {
        let (x, y) = self.apply(x, y);
        point(px(x), px(y))
    }

    /// The mean scale (for stroke widths and radii).
    pub fn unit(&self) -> f32 {
        (self.a * self.d - self.b * self.c).abs().sqrt()
    }

    fn axis_aligned(&self) -> bool {
        self.b.abs() < 1e-4 && self.c.abs() < 1e-4
    }
}

/// Paints shapes given in local units through `m`.
pub struct Pen {
    pub m: Affine,
}

impl Pen {
    /// A filled rounded rectangle; `glow` adds a soft light of `blur`
    /// local units around it (the face's SVG blur).
    #[allow(clippy::too_many_arguments)]
    pub fn round_rect(&self, window: &mut Window, x: f32, y: f32, w: f32, h: f32, r: f32, color: Hsla, glow: Option<f32>) {
        if self.m.axis_aligned() {
            let (x0, y0) = self.m.apply(x, y);
            let (x1, y1) = self.m.apply(x + w, y + h);
            let (left, top) = (x0.min(x1), y0.min(y1));
            let (bw, bh) = ((x1 - x0).abs(), (y1 - y0).abs());
            let radius = (r * self.m.a.abs()).min(r * self.m.d.abs()).min(bw / 2.0).min(bh / 2.0);
            let bounds = Bounds { origin: point(px(left), px(top)), size: size(px(bw), px(bh)) };
            let corners = Corners::all(px(radius));
            if let Some(blur) = glow {
                window.paint_drop_shadows(
                    bounds,
                    corners,
                    &[BoxShadow {
                        color: color.opacity(0.75),
                        offset: point(px(0.), px(0.)),
                        blur_radius: px(blur * 2.0 * self.m.unit()),
                        spread_radius: px(0.),
                        inset: false,
                    }],
                );
            }
            window.paint_quad(fill(bounds, color).corner_radii(corners));
            return;
        }
        let pts = round_rect_points(x, y, w, h, r);
        self.fill(window, &pts, color);
    }

    pub fn circle(&self, window: &mut Window, cx: f32, cy: f32, r: f32, color: Hsla) {
        self.dot(window, cx, cy, r, color, None);
    }

    /// A circle with an optional soft light of `glow` local units.
    pub fn dot(&self, window: &mut Window, cx: f32, cy: f32, r: f32, color: Hsla, glow: Option<f32>) {
        let (x, y) = self.m.apply(cx, cy);
        let rr = r * self.m.unit();
        let bounds = Bounds { origin: point(px(x - rr), px(y - rr)), size: size(px(rr * 2.0), px(rr * 2.0)) };
        let corners = Corners::all(px(rr));
        if let Some(blur) = glow {
            window.paint_drop_shadows(
                bounds,
                corners,
                &[BoxShadow {
                    color: color.opacity(0.75),
                    offset: point(px(0.), px(0.)),
                    blur_radius: px(blur * 2.0 * self.m.unit()),
                    spread_radius: px(0.),
                    inset: false,
                }],
            );
        }
        window.paint_quad(fill(bounds, color).corner_radii(corners));
    }

    /// A filled polygon.
    pub fn fill(&self, window: &mut Window, pts: &[(f32, f32)], color: Hsla) {
        if pts.len() < 3 {
            return;
        }
        let mut b = PathBuilder::fill();
        b.move_to(self.m.point(pts[0].0, pts[0].1));
        for &(x, y) in &pts[1..] {
            b.line_to(self.m.point(x, y));
        }
        b.close();
        if let Ok(path) = b.build() {
            window.paint_path(path, color);
        }
    }

    /// A polyline stroke `width` local units wide with round ends (and
    /// round joins), as `stroke-linecap: round` draws it.
    pub fn stroke(&self, window: &mut Window, pts: &[(f32, f32)], width: f32, color: Hsla, round: bool) {
        if pts.len() < 2 {
            return;
        }
        let w = width * self.m.unit();
        let mut b = PathBuilder::stroke(px(w));
        b.move_to(self.m.point(pts[0].0, pts[0].1));
        for &(x, y) in &pts[1..] {
            b.line_to(self.m.point(x, y));
        }
        if let Ok(path) = b.build() {
            window.paint_path(path, color);
        }
        if round {
            // Few points are true corners (a check mark); a sampled curve
            // joins smoothly and needs caps only at its ends.
            let corners = pts.len() <= 4;
            for (i, &(x, y)) in pts.iter().enumerate() {
                if corners || i == 0 || i == pts.len() - 1 {
                    self.circle(window, x, y, width / 2.0, color);
                }
            }
        }
    }

    /// The stroke plus a faint wider halo (the SVG blur, approximated).
    pub fn glow_stroke(&self, window: &mut Window, pts: &[(f32, f32)], width: f32, color: Hsla, glow: bool) {
        if glow {
            self.stroke(window, pts, width + 3.6, color.opacity(0.22), false);
        }
        self.stroke(window, pts, width, color, true);
    }
}

/// A rounded rectangle as a polygon (for rotated shapes).
pub fn round_rect_points(x: f32, y: f32, w: f32, h: f32, r: f32) -> Vec<(f32, f32)> {
    let r = r.min(w / 2.0).min(h / 2.0);
    let mut out = Vec::with_capacity(28);
    let corners = [(x + w - r, y + r, -90.0f32), (x + w - r, y + h - r, 0.0), (x + r, y + h - r, 90.0), (x + r, y + r, 180.0)];
    for (cx, cy, start) in corners {
        for i in 0..=6 {
            let a = (start + 90.0 * i as f32 / 6.0).to_radians();
            out.push((cx + r * a.cos(), cy + r * a.sin()));
        }
    }
    out
}

/// A quadratic curve sampled into points.
pub fn quad(p0: (f32, f32), c: (f32, f32), p1: (f32, f32), n: usize) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            let u = 1.0 - t;
            (u * u * p0.0 + 2.0 * u * t * c.0 + t * t * p1.0, u * u * p0.1 + 2.0 * u * t * c.1 + t * t * p1.1)
        })
        .collect()
}

/// The first `len` local units of a polyline.
pub fn partial(pts: &[(f32, f32)], len: f32) -> Vec<(f32, f32)> {
    let mut out = vec![pts[0]];
    let mut left = len;
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let seg = ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt();
        if left >= seg {
            out.push(b);
            left -= seg;
        } else {
            if left > 0.0 {
                let t = left / seg;
                out.push((a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t));
            }
            break;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// CSS timing
// ---------------------------------------------------------------------------

/// A CSS `cubic-bezier(x1, y1, x2, y2)`.
#[derive(Clone, Copy, Debug)]
pub struct Ease(pub f32, pub f32, pub f32, pub f32);

pub const EASE: Ease = Ease(0.25, 0.1, 0.25, 1.0);
pub const EASE_IN_OUT: Ease = Ease(0.42, 0.0, 0.58, 1.0);
pub const EASE_OUT: Ease = Ease(0.0, 0.0, 0.58, 1.0);
pub const LINEAR: Ease = Ease(0.0, 0.0, 1.0, 1.0);

impl Ease {
    pub fn at(self, t: f32) -> f32 {
        let Ease(x1, y1, x2, y2) = self;
        let t = t.clamp(0.0, 1.0);
        let bez = |p1: f32, p2: f32, s: f32| {
            let u = 1.0 - s;
            3.0 * u * u * s * p1 + 3.0 * u * s * s * p2 + s * s * s
        };
        // Solve x(s) = t by bisection: plenty for a few frames a second.
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if bez(x1, x2, mid) < t {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        bez(y1, y2, (lo + hi) / 2.0)
    }
}

/// A keyframe track: `(offset, value)` pairs from 0 to 1, each segment eased.
pub fn track(t: f32, frames: &[(f32, f32)], ease: Ease) -> f32 {
    let t = t.clamp(0.0, 1.0);
    for w in frames.windows(2) {
        let ((o0, v0), (o1, v1)) = (w[0], w[1]);
        if t <= o1 {
            if o1 <= o0 {
                return v1;
            }
            return v0 + (v1 - v0) * ease.at((t - o0) / (o1 - o0));
        }
    }
    frames.last().map_or(0.0, |f| f.1)
}

/// The fraction of a loop of `period` ms at time `t` ms, started `delay` ms in.
pub fn phase(t: f64, period: f64, delay: f64) -> f32 {
    ((t - delay).rem_euclid(period) / period) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_functions_match_css_at_the_ends_and_middle() {
        assert!((EASE_IN_OUT.at(0.5) - 0.5).abs() < 1e-3);
        assert!(EASE.at(0.0).abs() < 1e-3 && (EASE.at(1.0) - 1.0).abs() < 1e-3);
        assert!((LINEAR.at(0.3) - 0.3).abs() < 1e-3);
        // The hop overshoots.
        assert!(Ease(0.3, 1.6, 0.5, 1.0).at(0.6) > 1.0);
    }

    #[test]
    fn tracks_hold_and_interpolate() {
        let blink = [(0.0, 1.0), (0.43, 0.1), (1.0, 1.0)];
        assert!((track(0.43, &blink, LINEAR) - 0.1).abs() < 1e-4);
        assert!((track(1.0, &blink, LINEAR) - 1.0).abs() < 1e-4);
        assert_eq!(phase(2500.0, 1000.0, 0.0), 0.5);
        assert_eq!(phase(0.0, 1000.0, 250.0), 0.75);
    }

    #[test]
    fn affine_composes_in_svg_order() {
        let m = Affine::translate(10.0, 0.0).then(Affine::scale(2.0, 2.0));
        assert_eq!(m.apply(1.0, 1.0), (12.0, 2.0));
        let r = Affine::rotate_at(90.0, 0.0, 0.0).apply(1.0, 0.0);
        assert!(r.0.abs() < 1e-5 && (r.1 - 1.0).abs() < 1e-5);
        let p = partial(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)], 15.0);
        assert_eq!(p, [(0.0, 0.0), (10.0, 0.0), (10.0, 5.0)]);
    }
}
