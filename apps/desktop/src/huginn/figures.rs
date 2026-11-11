//! Compare, Board, Bullet and Tree (data) and Annotated (work): the
//! components that lay out a decision, the state of pieces of work, a value
//! against its target, an outline and findings pinned on a screenshot. All
//! static: they draw again only when the view's data or width changes.

use super::ImageLoad;
use super::build::{
    B, CARD_BAND, DOT, IMAGE_MAX_H, KEY_MIN_W, Pal, arr, card_band, eid, flag, focusable, lead, n,
    s, step_mark, txt,
};
use crate::ui::{self, icon_size, leading, radius, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Div, Hsla, InteractiveElement as _, IntoElement, ParentElement,
    StatefulInteractiveElement as _, Styled, div, px, relative,
};
use serde_json::Value;

/// A Board column is never narrower than this; below it columns wrap.
const BOARD_TRACK: f32 = 120.;
/// The gap between Board columns, and between a Bullet's rows.
const BOARD_GAP: f32 = space::LG;
/// A Bullet's bar, and the target tick that stands over it.
const BULLET_BAR: f32 = space::MD;
const BULLET_TICK: f32 = space::XL;
/// One level of a Tree: its mark and the space after it, so a child's guide
/// runs under its parent's mark.
const TREE_INDENT: f32 = icon_size::ROW + space::MD;
/// A pin's numbered disc, on the image and in the legend.
const PIN: f32 = leading::UI;

/// How many Board columns share a row at this width: all of them when
/// they fit, else rows as even as they can be, down to one column.
pub(super) fn board_per_row(columns: usize, width: f32) -> usize {
    let columns = columns.max(1);
    let fit =
        (((width + BOARD_GAP) / (BOARD_TRACK + BOARD_GAP)).floor() as usize).clamp(1, columns);
    let rows = columns.div_ceil(fit);
    columns.div_ceil(rows)
}

/// The end of a Bullet's scale: `max`, or 1.25 × the larger of value and
/// target; 1 when that is not a positive number.
pub(super) fn bullet_top(value: f64, target: f64, max: Option<f64>) -> f64 {
    let top = max.unwrap_or(1.25 * value.max(target));
    if top.is_finite() && top > 0.0 {
        top
    } else {
        1.0
    }
}

/// Whether a value meets its target.
pub(super) fn bullet_meets(value: f64, target: f64, lower_is_better: bool) -> bool {
    if lower_is_better {
        value <= target
    } else {
        value >= target
    }
}

/// Where on the bar a value falls, 0 to 1.
fn share(value: f64, top: f64) -> f32 {
    (value / top).clamp(0.0, 1.0) as f32
}

/// A pin's disc and number colours: solid, readable on any screenshot and
/// in both modes (each state's ink over its own wash, the accent's ink over
/// the accent, and ink over the surface for no tone).
fn pin_colours(tone: &str, pal: &Pal) -> (Hsla, Hsla) {
    match tone {
        "success" => (pal.c("green-ink"), pal.c("green-bg")),
        "warning" => (pal.c("amber-ink"), pal.c("amber-bg")),
        "danger" => (pal.c("red-ink"), pal.c("red-bg")),
        "accent" | "info" => (pal.c("accent"), pal.c("accent-ink")),
        _ => (pal.c("ink"), pal.c("surface")),
    }
}

/// A numbered disc: a pin on the image, and its entry in the legend.
fn pin_disc(number: usize, tone: &str, pal: &Pal) -> Div {
    let (fill, ink) = pin_colours(tone, pal);
    div()
        .flex()
        .flex_none()
        .items_center()
        .justify_center()
        .size(px(PIN))
        .rounded_full()
        .bg(fill)
        .border_1()
        .border_color(pal.c("surface"))
        .text_color(ink)
        .text_size(px(size::MICRO))
        .line_height(px(leading::MICRO))
        .font_weight(weight::STRONG)
        .child(number.to_string())
}

impl<'a, 'w> B<'a, 'w> {
    /// Options as columns, criteria as rows; the recommended column carries
    /// the accent band and a badge, and a light wash down its cells.
    pub(super) fn compare(&mut self, p: &Value) -> AnyElement {
        let pal = self.pal;
        let options: Vec<String> = arr(p, "options")
            .iter()
            .map(|o| o.as_str().unwrap_or_default().to_string())
            .collect();
        let recommended = n(p, "recommended")
            .map(|r| r as usize)
            .filter(|r| (1..=options.len()).contains(r))
            .map(|r| r - 1);
        let badge = self.tr("recommended");
        let label_cell = || {
            div()
                .w(relative(0.28))
                .min_w(px(KEY_MIN_W))
                .flex_none()
                .px(px(space::MD))
                .py(px(space::SM))
        };
        let option_cell = move |i: usize| {
            div()
                .relative()
                .flex_1()
                .min_w_0()
                .px(px(space::MD))
                .py(px(space::SM))
                .when(recommended == Some(i), |d| d.bg(pal.c("tint")))
        };
        let head = div()
            .flex()
            .border_b_1()
            .border_color(pal.c("line-strong"))
            .child(label_cell())
            .children(options.iter().enumerate().map(|(i, option)| {
                let on = recommended == Some(i);
                option_cell(i)
                    .flex()
                    .flex_col()
                    .items_start()
                    .gap(px(space::XS))
                    .when(on, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top_0()
                                .left_0()
                                .right_0()
                                .h(px(CARD_BAND))
                                .rounded_t(px(radius::SM))
                                .bg(pal.c("accent")),
                        )
                    })
                    .child(div().font_weight(weight::STRONG).child(option.clone()))
                    .when(on, |d| {
                        let (_, ink, bg) = pal.tone("accent");
                        d.child(
                            div()
                                .flex()
                                .items_center()
                                .h(px(leading::META))
                                .px(px(space::SM))
                                .rounded(px(radius::SM))
                                .bg(bg)
                                .text_color(ink)
                                .text_size(px(size::META))
                                .line_height(px(leading::META))
                                .font_weight(weight::STRONG)
                                .whitespace_nowrap()
                                .child(badge.clone()),
                        )
                    })
            }));
        let criteria = arr(p, "criteria");
        let last = criteria.len().saturating_sub(1);
        let rows = criteria.iter().enumerate().map(|(r, row)| {
            div()
                .flex()
                .when(r < last, |d| d.border_b_1().border_color(pal.c("line")))
                .child(
                    label_cell()
                        .text_color(pal.c("ink-3"))
                        .child(s(row, "label").unwrap_or_default().to_string()),
                )
                .children(arr(row, "cells").iter().enumerate().map(|(i, cell)| {
                    let tone = s(cell, "tone").unwrap_or("neutral");
                    let (edge, _, _) = pal.tone(tone);
                    option_cell(i)
                        .flex()
                        .gap(px(space::SM))
                        .when(tone != "neutral", |d| {
                            d.child(lead(
                                div().size(px(DOT)).rounded_full().bg(edge),
                                leading::UI,
                            ))
                        })
                        .child(
                            div()
                                .min_w_0()
                                .child(s(cell, "text").unwrap_or_default().to_string()),
                        )
                }))
        });
        div()
            .flex()
            .flex_col()
            .gap(px(space::SM))
            .min_w_0()
            .when_some(s(p, "title"), |d, title| {
                d.child(div().font_weight(weight::STRONG).child(title.to_string()))
            })
            .child(
                div()
                    .flex()
                    .flex_col()
                    .text_size(px(size::UI))
                    .line_height(px(leading::UI))
                    .child(head)
                    .children(rows),
            )
            .into_any_element()
    }

    /// Columns side by side, each a title with its count over its cards;
    /// the cards are Card's box at a smaller scale.
    pub(super) fn board(&mut self, p: &Value, id: &str) -> AnyElement {
        let pal = self.pal;
        let (width, probe) = self.width_probe(&format!("{id}#board"), 640.);
        let empty = self.tr("boardEmpty");
        let columns: Vec<AnyElement> = arr(p, "columns")
            .iter()
            .map(|column| {
                let tone = s(column, "tone").unwrap_or("neutral");
                let (edge, _, _) = pal.tone(tone);
                let cards = arr(column, "cards");
                let head = div()
                    .flex()
                    .items_center()
                    .gap(px(space::SM))
                    .pb(px(space::SM))
                    .border_b_1()
                    .border_color(pal.c("line"))
                    .when(tone != "neutral", |d| {
                        d.child(div().flex_none().size(px(DOT)).rounded_full().bg(edge))
                    })
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .font_weight(weight::STRONG)
                            .child(s(column, "title").unwrap_or_default().to_string()),
                    )
                    .child(
                        txt(
                            cards.len().to_string(),
                            size::META,
                            weight::REGULAR,
                            pal.c("ink-3"),
                        )
                        .flex_none(),
                    );
                let body: Vec<AnyElement> = if cards.is_empty() {
                    vec![
                        txt(empty.clone(), size::SMALL, weight::REGULAR, pal.c("ink-4"))
                            .into_any_element(),
                    ]
                } else {
                    cards.iter().map(|card| self.board_card(card)).collect()
                };
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(space::SM))
                    .child(head)
                    .children(body)
                    .into_any_element()
            })
            .collect();
        let per_row = board_per_row(columns.len(), width);
        let mut rows: Vec<AnyElement> = Vec::new();
        let mut it = columns.into_iter().peekable();
        while it.peek().is_some() {
            let mut cells: Vec<AnyElement> = it.by_ref().take(per_row).collect();
            while cells.len() < per_row {
                cells.push(div().flex_1().min_w_0().into_any_element());
            }
            rows.push(
                div()
                    .flex()
                    .items_start()
                    .gap(px(BOARD_GAP))
                    .children(cells)
                    .into_any_element(),
            );
        }
        div()
            .relative()
            .flex()
            .flex_col()
            .gap(px(space::XL))
            .min_w_0()
            .text_size(px(size::UI))
            .line_height(px(leading::UI))
            .child(probe)
            .children(rows)
            .into_any_element()
    }

    fn board_card(&self, card: &Value) -> AnyElement {
        let pal = self.pal;
        let tone = s(card, "tone").unwrap_or("neutral");
        div()
            .relative()
            .flex()
            .flex_col()
            .items_start()
            .gap(px(space::XS))
            .py(px(space::MD))
            .px(px(space::MD))
            .pl(px(space::MD + CARD_BAND))
            .border_1()
            .border_color(pal.c("line"))
            .rounded(px(radius::MD))
            .bg(pal.c("surface-2"))
            .child(
                div()
                    .absolute()
                    .top(px(-1.))
                    .bottom(px(-1.))
                    .left(px(-1.))
                    .w(px(CARD_BAND))
                    .rounded_l(px(radius::MD))
                    .bg(card_band(tone, pal)),
            )
            .child(div().child(s(card, "title").unwrap_or_default().to_string()))
            .when_some(s(card, "detail"), |d, detail| {
                d.child(txt(
                    detail.to_string(),
                    size::SMALL,
                    weight::REGULAR,
                    pal.c("ink-3"),
                ))
            })
            .when_some(s(card, "tag"), |d, tag| {
                d.child(
                    div()
                        .flex()
                        .items_center()
                        .h(px(leading::META))
                        .px(px(space::SM))
                        .rounded(px(radius::SM))
                        .bg(pal.c("chip"))
                        .text_color(pal.c("chip-ink"))
                        .text_size(px(size::META))
                        .line_height(px(leading::META))
                        .font_weight(weight::STRONG)
                        .whitespace_nowrap()
                        .child(tag.to_string()),
                )
            })
            .into_any_element()
    }

    /// One value against its target: a bar in the verdict's colour, a tick
    /// at the target, and the verdict said in words beside its icon.
    pub(super) fn bullet(&mut self, p: &Value) -> AnyElement {
        let pal = self.pal;
        let value = n(p, "value").unwrap_or(0.0);
        let target = n(p, "target").unwrap_or(0.0);
        let unit = s(p, "unit");
        let lower = flag(p, "lowerIsBetter");
        let top = bullet_top(value, target, n(p, "max"));
        let meets = bullet_meets(value, target, lower);
        let (edge, ink, _) = pal.tone(if meets { "success" } else { "danger" });
        let target_text = self.num(Some(target), unit);
        let verdict = self.trv(
            match (meets, lower) {
                (true, _) => "bulletMet",
                (false, false) => "bulletBelow",
                (false, true) => "bulletAbove",
            },
            &[("target", &target_text)],
        );
        let value_text = self.num(Some(value), unit);
        div()
            .flex()
            .flex_col()
            .gap(px(space::XS))
            .text_size(px(size::UI))
            .line_height(px(leading::UI))
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap(px(space::LG))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(s(p, "label").unwrap_or_default().to_string()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_weight(weight::STRONG)
                            .child(value_text),
                    ),
            )
            .child(
                div()
                    .relative()
                    .h(px(BULLET_TICK))
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .w_full()
                            .h(px(BULLET_BAR))
                            .rounded(px(radius::SM))
                            .bg(pal.c("chip"))
                            .overflow_hidden()
                            .child(
                                div()
                                    .h_full()
                                    .rounded(px(radius::SM))
                                    .bg(edge)
                                    .w(relative(share(value, top))),
                            ),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .bottom_0()
                            .left(relative(share(target, top)))
                            .ml(px(-space::HAIR / 2.))
                            .w(px(space::HAIR))
                            .bg(pal.c("ink")),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap(px(space::XS))
                    .text_size(px(size::SMALL))
                    .line_height(px(leading::SMALL))
                    .text_color(ink)
                    .child(lead(
                        ui::icon(if meets { "check" } else { "x" }, icon_size::INLINE, ink),
                        leading::SMALL,
                    ))
                    .child(div().min_w_0().child(verdict)),
            )
            .when_some(s(p, "hint"), |d, hint| {
                d.child(txt(
                    hint.to_string(),
                    size::SMALL,
                    weight::REGULAR,
                    pal.c("ink-3"),
                ))
            })
            .into_any_element()
    }

    /// An outline: each level indents by one mark, with a guide line down
    /// from the parent's mark; a state shows Steps' icon, else the tone's dot.
    pub(super) fn tree(&mut self, p: &Value) -> AnyElement {
        let pal = self.pal;
        let nodes = arr(p, "nodes");
        let last = nodes.len().saturating_sub(1);
        let rows: Vec<AnyElement> = nodes
            .iter()
            .enumerate()
            .map(|(r, node)| {
                let depth = n(node, "depth").unwrap_or(0.0).clamp(0.0, 6.0) as usize;
                let state = s(node, "state");
                let mark = match state {
                    Some(state) => {
                        let (icon, color, _) = step_mark(state, pal);
                        ui::icon(icon, icon_size::ROW, color).into_any_element()
                    }
                    None => {
                        let (edge, _, _) = pal.tone(s(node, "tone").unwrap_or("neutral"));
                        div()
                            .size(px(DOT))
                            .rounded_full()
                            .bg(edge)
                            .into_any_element()
                    }
                };
                let quiet = state == Some("todo");
                let current = state == Some("current");
                div()
                    .flex()
                    .children((0..depth).map(|_| {
                        div().relative().flex_none().w(px(TREE_INDENT)).child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(px(icon_size::ROW / 2. - 0.5))
                                .w(px(1.))
                                .bg(pal.c("line-strong")),
                        )
                    }))
                    .child(
                        div()
                            .flex_none()
                            .w(px(icon_size::ROW))
                            .h(px(leading::UI))
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(mark),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pl(px(space::MD))
                            .when(r < last, |d| d.pb(px(space::SM)))
                            .flex()
                            .flex_wrap()
                            .items_baseline()
                            .gap_x(px(space::SM))
                            .child(
                                div()
                                    .when(current, |d| d.font_weight(weight::STRONG))
                                    .when(quiet, |d| d.text_color(pal.c("ink-3")))
                                    .child(s(node, "label").unwrap_or_default().to_string()),
                            )
                            .when_some(s(node, "detail"), |d, detail| {
                                d.child(txt(
                                    detail.to_string(),
                                    size::SMALL,
                                    weight::REGULAR,
                                    pal.c("ink-3"),
                                ))
                            }),
                    )
                    .into_any_element()
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .text_size(px(size::UI))
            .line_height(px(leading::UI))
            .children(rows)
            .into_any_element()
    }

    /// The image at Image's width, a numbered disc at each pin, and under
    /// it the legend in the same numbers. A click opens the image.
    pub(super) fn annotated(&mut self, p: &Value, id: &str) -> AnyElement {
        let pal = self.pal;
        let name = s(p, "file").unwrap_or_default();
        let Some(file) = self.image_file(name) else {
            return self.missing(name);
        };
        let (width, probe) = self.width_probe(&format!("{id}#annotated"), 640.);
        let pins = arr(p, "pins");
        let picture = match self.ui.images.get(&file.id) {
            Some(ImageLoad::Ready(image, w, h)) => {
                let (w, h) = (*w as f32, *h as f32);
                let scale = (width / w).min(1.0).min(IMAGE_MAX_H / h);
                let (dw, dh) = ((w * scale).max(1.0), (h * scale).max(1.0));
                let half = PIN / 2.0;
                let at = |fraction: f64, extent: f32| {
                    (fraction as f32 * extent).clamp(half, (extent - half).max(half)) - half
                };
                let open = file.clone();
                focusable(div().id(eid(format!("{id}-image"))), pal, false)
                    .relative()
                    .flex_none()
                    .w(px(dw))
                    .h(px(dh))
                    .rounded(px(radius::BASE))
                    .cursor_pointer()
                    .child(
                        gpui::img(image.clone())
                            .w(px(dw))
                            .h(px(dh))
                            .rounded(px(radius::BASE))
                            .border_1()
                            .border_color(pal.c("line"))
                            .bg(pal.c("surface")),
                    )
                    .children(pins.iter().enumerate().map(|(i, pin)| {
                        pin_disc(i + 1, s(pin, "tone").unwrap_or("neutral"), pal)
                            .absolute()
                            .left(px(at(n(pin, "x").unwrap_or(0.0), dw)))
                            .top(px(at(n(pin, "y").unwrap_or(0.0), dh)))
                    }))
                    .on_click(move |_, window, cx| {
                        crate::chat::attachments::open_lightbox(&open, window, cx)
                    })
                    .into_any_element()
            }
            _ => self
                .picture(file, width, Some(IMAGE_MAX_H))
                .into_any_element(),
        };
        let legend = pins.iter().enumerate().map(|(i, pin)| {
            div()
                .flex()
                .gap(px(space::MD))
                .child(lead(
                    pin_disc(i + 1, s(pin, "tone").unwrap_or("neutral"), pal),
                    leading::UI,
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(s(pin, "label").unwrap_or_default().to_string())
                        .when_some(s(pin, "detail"), |d, detail| {
                            d.child(txt(
                                detail.to_string(),
                                size::SMALL,
                                weight::REGULAR,
                                pal.c("ink-3"),
                            ))
                        }),
                )
        });
        div()
            .relative()
            .flex()
            .flex_col()
            .child(probe)
            .child(picture)
            .when_some(s(p, "caption"), |d, c| {
                d.child(
                    txt(c.to_string(), size::SMALL, weight::REGULAR, pal.c("ink-3"))
                        .pt(px(space::XS)),
                )
            })
            .child(
                div()
                    .pt(px(space::MD))
                    .flex()
                    .flex_col()
                    .gap(px(space::SM))
                    .text_size(px(size::UI))
                    .line_height(px(leading::UI))
                    .children(legend),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn board_columns_share_a_row_until_the_pane_is_narrow() {
        assert_eq!(board_per_row(3, 700.), 3);
        assert_eq!(board_per_row(5, 700.), 5);
        // Four of five fit: two even rows instead of four and one.
        assert_eq!(board_per_row(5, 560.), 3);
        assert_eq!(board_per_row(4, 200.), 1);
        assert_eq!(board_per_row(1, 1000.), 1);
        assert_eq!(board_per_row(0, 1000.), 1);
    }

    #[test]
    fn a_bullet_scale_covers_the_value_and_the_target() {
        assert_eq!(bullet_top(80., 100., None), 125.);
        assert_eq!(bullet_top(150., 100., None), 187.5);
        assert_eq!(bullet_top(80., 100., Some(200.)), 200.);
        assert_eq!(bullet_top(-3., -1., None), 1.);
        assert_eq!(bullet_top(0., 0., None), 1.);
        assert_eq!(share(300., 200.), 1.0);
        assert_eq!(share(-5., 200.), 0.0);
    }

    #[test]
    fn a_bullet_meets_its_target_from_the_right_side() {
        assert!(bullet_meets(200., 200., false));
        assert!(!bullet_meets(199., 200., false));
        assert!(bullet_meets(180., 200., true));
        assert!(!bullet_meets(201., 200., true));
    }
}
