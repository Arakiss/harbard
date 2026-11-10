//! A workspace's emblem (`lib/workspace-emblem.ts`): a small flat composition
//! drawn from its id, so it is the same on every machine and survives a
//! rename. Four big shapes on a 2×2 grid, in two colours over a third.
//!
//! [`emblem_of`] is the console's drawing, number for number (FNV-1a and
//! mulberry32); [`emblem`] paints it on a canvas at any size.

use gpui::{Bounds, Hsla, IntoElement, ParentElement, PathBuilder, Pixels, Point, Styled, canvas, div, point, px};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Palette {
    pub ground: u32,
    pub one: u32,
    pub two: u32,
}

pub const PALETTES: [Palette; 7] = [
    Palette { ground: 0xf2c14e, one: 0xd1495b, two: 0x1f3a5f },
    Palette { ground: 0x2a9d8f, one: 0xf4e3b2, two: 0x114b5f },
    Palette { ground: 0x5b5bd6, one: 0xffb4a2, two: 0x1e1b4b },
    Palette { ground: 0xef6f4c, one: 0xffe8c2, two: 0x5a1e3d },
    Palette { ground: 0x6dbe7b, one: 0xfdf6d8, two: 0x1e4d3b },
    Palette { ground: 0xe889b5, one: 0xfff1d6, two: 0x5b1f6e },
    Palette { ground: 0x6fa8dc, one: 0xffe08a, two: 0x1b3a6b },
];

const SIZE: f32 = 48.;
const CELL: f32 = SIZE / 2.;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shape {
    /// `M0 0H24A24 24 0 0 1 0 24Z`
    Quarter,
    /// `M0 0H12A12 12 0 0 1 24 12V24H12A12 12 0 0 1 0 12Z`
    Leaf,
    /// `M0 0H24A12 12 0 0 1 0 0Z`
    Half,
    /// `M12 3A9 9 0 1 1 12 21A9 9 0 1 1 12 3Z`
    Disc,
}

/// Quarters carry the composition; the rest are its accents.
const SHAPES: [Shape; 7] = [Shape::Quarter, Shape::Quarter, Shape::Quarter, Shape::Leaf, Shape::Leaf, Shape::Half, Shape::Disc];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placed {
    pub shape: Shape,
    pub color: u32,
    /// The cell's corner in the 48-unit box.
    pub x: u8,
    pub y: u8,
    /// Degrees, about the cell's centre.
    pub turn: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Emblem {
    pub palette: Palette,
    pub shapes: [Placed; 4],
}

/// FNV-1a over UTF-16 code units: a stable 32-bit number from any text.
pub fn seed_of(text: &str) -> u32 {
    let mut h: u32 = 0x811c9dc5;
    for c in text.encode_utf16() {
        h ^= c as u32;
        h = h.wrapping_mul(0x01000193);
    }
    h
}

/// mulberry32: small, seedable, the same as the console's.
struct Generator(u32);

impl Generator {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x6d2b79f5);
        let mut t = self.0;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        (t ^ (t >> 14)) as f64 / 4294967296.0
    }
}

fn pick<T: Copy>(list: &[T], r: f64) -> T {
    list[((r * list.len() as f64).floor() as usize).min(list.len() - 1)]
}

fn palette_index(id: &str) -> usize {
    ((Generator(seed_of(id)).next() * PALETTES.len() as f64).floor() as usize).min(PALETTES.len() - 1)
}

/// The palette of each workspace, in the hub's order: two never share one
/// while there are palettes to go round.
pub fn palettes_for<'a>(ids: impl IntoIterator<Item = &'a str>) -> HashMap<String, usize> {
    let mut out = HashMap::new();
    let mut used: Vec<usize> = Vec::new();
    for id in ids {
        if used.len() == PALETTES.len() {
            used.clear();
        }
        let mut at = palette_index(id);
        while used.contains(&at) {
            at = (at + 1) % PALETTES.len();
        }
        used.push(at);
        out.insert(id.to_string(), at);
    }
    out
}

/// The emblem of workspace `id`; `palette` from [`palettes_for`].
pub fn emblem_of(id: &str, palette: Option<usize>) -> Emblem {
    let mut random = Generator(seed_of(id));
    // The draw that picks the palette, so the drawing never depends on it.
    random.next();
    let colours = PALETTES[palette.unwrap_or_else(|| palette_index(id)) % PALETTES.len()];
    // Neighbours alternate, so the two colours always meet; which goes first is drawn.
    let flip = random.next() < 0.5;
    let mut shapes = [Placed {
        shape: Shape::Quarter,
        color: 0,
        x: 0,
        y: 0,
        turn: 0,
    }; 4];
    for (cell, slot) in shapes.iter_mut().enumerate() {
        let x = (cell % 2) as f32 * CELL;
        let y = (cell / 2) as f32 * CELL;
        let shape = pick(&SHAPES, random.next());
        let turn = (random.next() * 4.).floor() as u16 * 90;
        let even = (cell + cell / 2) % 2 == 0;
        *slot = Placed {
            shape,
            color: if even == flip { colours.one } else { colours.two },
            x: x as u8,
            y: y as u8,
            turn,
        };
    }
    Emblem { palette: colours, shapes }
}

fn hex(c: u32) -> Hsla {
    gpui::rgb(c).into()
}

/// One shape's outline in its cell, turned about the cell's centre and
/// placed, then scaled from the 48-unit box to `bounds`.
fn path(p: &Placed, bounds: Bounds<Pixels>) -> Option<gpui::Path<Pixels>> {
    let k = bounds.size.width / SIZE;
    let (sin, cos) = (p.turn as f32).to_radians().sin_cos();
    let at = |x: f32, y: f32| -> Point<Pixels> {
        let (dx, dy) = (x - 12., y - 12.);
        let (rx, ry) = (dx * cos - dy * sin + 12., dx * sin + dy * cos + 12.);
        point(bounds.origin.x + k * (rx + p.x as f32), bounds.origin.y + k * (ry + p.y as f32))
    };
    let r = |v: f32| point(k * v, k * v);
    let mut b = PathBuilder::fill();
    match p.shape {
        Shape::Quarter => {
            b.move_to(at(0., 0.));
            b.line_to(at(24., 0.));
            b.arc_to(r(24.), px(0.), false, true, at(0., 24.));
        }
        Shape::Leaf => {
            b.move_to(at(0., 0.));
            b.line_to(at(12., 0.));
            b.arc_to(r(12.), px(0.), false, true, at(24., 12.));
            b.line_to(at(24., 24.));
            b.line_to(at(12., 24.));
            b.arc_to(r(12.), px(0.), false, true, at(0., 12.));
        }
        Shape::Half => {
            b.move_to(at(0., 0.));
            b.line_to(at(24., 0.));
            b.arc_to(r(12.), px(0.), false, true, at(0., 0.));
        }
        Shape::Disc => {
            b.move_to(at(12., 3.));
            b.arc_to(r(9.), px(0.), true, true, at(12., 21.));
            b.arc_to(r(9.), px(0.), true, true, at(12., 3.));
        }
    }
    b.close();
    b.build().ok()
}

/// The emblem filling a `size`px square with `radius` corners.
pub fn emblem(id: &str, palette: Option<usize>, size: f32, radius: f32) -> impl IntoElement {
    let e = emblem_of(id, palette);
    div().size(px(size)).rounded(px(radius)).overflow_hidden().child(
        canvas(
            |_, _, _| {},
            move |bounds, _, window, _| {
                window.paint_quad(gpui::fill(bounds, hex(e.palette.ground)));
                for p in &e.shapes {
                    if let Some(path) = path(p, bounds) {
                        window.paint_path(path, hex(p.color));
                    }
                }
            },
        )
        .size_full(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    // Reference values from the console's own code (bun, lib/workspace-emblem.ts).
    #[test]
    fn matches_the_console_drawing() {
        assert_eq!(seed_of("personal"), 205118861);
        assert_eq!(seed_of("acme-42-platform"), 1385464364);
        let e = emblem_of("personal", None);
        assert_eq!(e.palette, PALETTES[3]);
        let got: Vec<(Shape, u32, u8, u8, u16)> = e.shapes.iter().map(|s| (s.shape, s.color, s.x, s.y, s.turn)).collect();
        assert_eq!(
            got,
            vec![
                (Shape::Disc, 0xffe8c2, 0, 0, 270),
                (Shape::Half, 0x5a1e3d, 24, 0, 180),
                (Shape::Quarter, 0x5a1e3d, 0, 24, 0),
                (Shape::Disc, 0xffe8c2, 24, 24, 180),
            ]
        );
        let e = emblem_of("acme-42-platform", None);
        assert_eq!(e.palette, PALETTES[1]);
        let got: Vec<(Shape, u16)> = e.shapes.iter().map(|s| (s.shape, s.turn)).collect();
        assert_eq!(
            got,
            vec![(Shape::Quarter, 270), (Shape::Quarter, 0), (Shape::Half, 270), (Shape::Leaf, 270)]
        );
    }

    #[test]
    fn palettes_go_round_before_repeating() {
        let p = palettes_for(["personal", "acme-42-platform", "x"]);
        assert_eq!(p["personal"], 3);
        assert_eq!(p["acme-42-platform"], 1);
        assert_eq!(p["x"], 4);
        let ids: Vec<String> = (0..PALETTES.len()).map(|i| format!("workspace-{i}")).collect();
        let all = palettes_for(ids.iter().map(String::as_str));
        let distinct: std::collections::HashSet<_> = all.values().collect();
        assert_eq!(distinct.len(), PALETTES.len());
    }

    #[test]
    fn both_colours_always_meet() {
        for id in ["personal", "acme-42-platform", "acme", "x", "lab"] {
            let colours: std::collections::HashSet<u32> = emblem_of(id, None).shapes.iter().map(|s| s.color).collect();
            assert_eq!(colours.len(), 2, "{id}");
        }
    }
}
