//! Rounded washes behind words inside a paragraph: `@mentions`
//! (`.hb-mention`) and commit codes, branches and paths (`.hb-tok`).
//!
//! A text highlight in GPUI can colour a run's background but cannot pad or
//! round it, so the chip is painted underneath: the text is laid out as
//! usual, and a canvas drawn first reads where each chip's characters
//! landed and fills a rounded box per line the chip spans. The padding is
//! real space in the text (a thin or hair space on each side, inside the
//! chip's range), so neighbouring words keep their distance from the box.

use gpui::{
    App, Bounds, Hsla, IntoElement, Pixels, Styled, TextLayout, Window, canvas, fill, point, px,
    size,
};
use std::ops::Range;

/// `.hb-mention { padding: 0 2px }`: a hair space each side.
pub const MENTION_PAD: char = '\u{200A}';
/// `.hb-tok { padding: 0 4px }`: a thin space each side.
pub const TOKEN_PAD: char = '\u{2009}';
/// The chip's height inside a 22 px line: the console's inline box.
const CHIP_H: f32 = 18.;

/// One chip to paint: the byte range it covers (padding included), its
/// fill and its corner radius.
#[derive(Clone)]
pub struct Chip {
    pub range: Range<usize>,
    pub fill: Hsla,
    pub radius: f32,
}

/// Append `word` to `out` between two `pad` characters and return the
/// range of the whole chip and the range of the word alone.
pub fn push_padded(out: &mut String, word: &str, pad: char) -> (Range<usize>, Range<usize>) {
    let start = out.len();
    out.push(pad);
    let word_start = out.len();
    out.push_str(word);
    let word_end = out.len();
    out.push(pad);
    (start..out.len(), word_start..word_end)
}

/// The horizontal spans a chip covers, one per line, from the positions of
/// its character boundaries (`x`, line top `y`). A boundary at a soft wrap
/// reports the end of the line before, so a zero-width span there is
/// dropped.
pub fn line_spans(points: &[(f32, f32)]) -> Vec<(f32, f32, f32)> {
    let mut spans: Vec<(f32, f32, f32)> = Vec::new();
    for &(x, y) in points {
        match spans.last_mut() {
            Some((top, x0, x1)) if (*top - y).abs() < 0.5 => {
                *x0 = x0.min(x);
                *x1 = x1.max(x);
            }
            _ => spans.push((y, x, x)),
        }
    }
    spans.retain(|(_, x0, x1)| x1 - x0 >= 1.);
    spans
}

/// Every character boundary of `range` in `text`, both ends included.
fn boundaries(text: &str, range: &Range<usize>) -> Vec<usize> {
    let mut out: Vec<usize> = text[range.clone()]
        .char_indices()
        .map(|(i, _)| range.start + i)
        .collect();
    out.push(range.end);
    out
}

/// The canvas that paints `chips` behind the text laid out in `layout`.
/// Put it before the text, absolutely positioned over the same box.
pub fn backdrop(layout: TextLayout, text: &str, chips: Vec<Chip>) -> impl IntoElement {
    let marks: Vec<(Vec<usize>, Hsla, f32)> = chips
        .into_iter()
        .filter(|c| c.range.end <= text.len() && c.range.start < c.range.end)
        .map(|c| (boundaries(text, &c.range), c.fill, c.radius))
        .collect();
    canvas(
        |_, _, _| {},
        move |_, _, window: &mut Window, _: &mut App| {
            let line_h = layout.line_height();
            let inset = (line_h - px(CHIP_H)).max(px(0.)) / 2.;
            for (ixs, color, radius) in &marks {
                let points: Vec<(f32, f32)> = ixs
                    .iter()
                    .filter_map(|&ix| layout.position_for_index(ix))
                    .map(|p| (f32::from(p.x), f32::from(p.y)))
                    .collect();
                for (y, x0, x1) in line_spans(&points) {
                    let bounds: Bounds<Pixels> = Bounds::new(
                        point(px(x0), px(y) + inset),
                        size(px(x1 - x0), line_h - inset * 2.),
                    );
                    window.paint_quad(fill(bounds, *color).corner_radii(px(*radius)));
                }
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chip_on_one_line_is_one_span() {
        let spans = line_spans(&[(10., 0.), (14., 0.), (30., 0.)]);
        assert_eq!(spans, vec![(0., 10., 30.)]);
    }

    #[test]
    fn a_chip_across_a_wrap_is_two_spans() {
        let spans = line_spans(&[(200., 0.), (210., 0.), (0., 22.), (12., 22.)]);
        assert_eq!(spans, vec![(0., 200., 210.), (22., 0., 12.)]);
    }

    #[test]
    fn a_boundary_at_the_wrap_leaves_no_empty_box() {
        // The first boundary sits at the end of the line above.
        let spans = line_spans(&[(300., 0.), (0., 22.), (9., 22.)]);
        assert_eq!(spans, vec![(22., 0., 9.)]);
    }

    #[test]
    fn padding_sits_inside_the_chip() {
        let mut s = String::from("ask ");
        let (chip, word) = push_padded(&mut s, "@cart", MENTION_PAD);
        assert_eq!(&s[word.clone()], "@cart");
        assert!(chip.start < word.start && chip.end > word.end);
        assert_eq!(chip.end, s.len());
        assert_eq!(boundaries(&s, &chip).len(), "@cart".len() + 3);
    }
}
