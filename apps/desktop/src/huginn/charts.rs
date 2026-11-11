//! Charts drawn by hand, as the console drew them in SVG: thin marks, a
//! recessive grid, one axis, a legend for more than one series, a tooltip on
//! every mark and the numbers one click away. Shapes go through a canvas;
//! text stays ordinary text so it is never scaled. Also the before/after
//! comparison, which shares the pointer maths.

use super::build::{B, arr, eid, n, s, series, txt};
use super::scale::{self, Domain};
use super::{HuginnView, ImageLoad};
use crate::ui::{self, control, icon_size, radius, size, space, weight};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Bounds, Context, Corners, Hsla, InteractiveElement, IntoElement, MouseMoveEvent,
    ParentElement, PathBuilder, Pixels, SharedString, StatefulInteractiveElement, Styled, canvas,
    div, fill, point, px, size,
};
use serde_json::Value;

const HEIGHT: f32 = 240.;
const TOP: f32 = 12.;
const RIGHT: f32 = 12.;
const BOTTOM: f32 = 28.;
const LEFT: f32 = 48.;
/// A legend's colour square, the donut, the label inside it, a heatmap cell.
const SWATCH: f32 = 10.;
const RING: f32 = 180.;
const RING_LABEL_W: f32 = 104.;
const CELL: f32 = 24.;
const SCALE_STEPS: usize = 5;
/// A picture's box while it loads.
const PLACEHOLDER_H: f32 = 120.;

#[derive(Clone)]
enum Shape {
    Rect {
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        r: f32,
        color: Hsla,
    },
    Line {
        points: Vec<(f32, f32)>,
        width: f32,
        color: Hsla,
    },
    Area {
        points: Vec<(f32, f32)>,
        color: Hsla,
    },
    Dot {
        x: f32,
        y: f32,
        r: f32,
        color: Hsla,
        ring: Option<Hsla>,
    },
    Arc {
        cx: f32,
        cy: f32,
        outer: f32,
        inner: f32,
        from: f32,
        to: f32,
        color: Hsla,
    },
}

fn paint(shapes: Vec<Shape>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds: Bounds<Pixels>, _, window, _| {
            let (ox, oy) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
            let at = |x: f32, y: f32| point(px(ox + x), px(oy + y));
            for shape in &shapes {
                match shape {
                    Shape::Rect {
                        x,
                        y,
                        w,
                        h,
                        r,
                        color,
                    } => {
                        let mut q = fill(Bounds::new(at(*x, *y), size(px(*w), px(*h))), *color);
                        q.corner_radii = Corners::all(px(*r));
                        window.paint_quad(q);
                    }
                    Shape::Line {
                        points,
                        width,
                        color,
                    } => {
                        let mut b = PathBuilder::stroke(px(*width));
                        for (i, (x, y)) in points.iter().enumerate() {
                            if i == 0 {
                                b.move_to(at(*x, *y));
                            } else {
                                b.line_to(at(*x, *y));
                            }
                        }
                        if let Ok(path) = b.build() {
                            window.paint_path(path, *color);
                        }
                        // Round joins and caps, as the SVG stroke had.
                        if *width > 1.0 {
                            for (x, y) in points {
                                let r = width / 2.0;
                                let mut q = fill(
                                    Bounds::new(at(x - r, y - r), size(px(r * 2.), px(r * 2.))),
                                    *color,
                                );
                                q.corner_radii = Corners::all(px(r));
                                window.paint_quad(q);
                            }
                        }
                    }
                    Shape::Area { points, color } => {
                        let mut b = PathBuilder::fill();
                        for (i, (x, y)) in points.iter().enumerate() {
                            if i == 0 {
                                b.move_to(at(*x, *y));
                            } else {
                                b.line_to(at(*x, *y));
                            }
                        }
                        b.close();
                        if let Ok(path) = b.build() {
                            window.paint_path(path, *color);
                        }
                    }
                    Shape::Dot {
                        x,
                        y,
                        r,
                        color,
                        ring,
                    } => {
                        let outer = r + if ring.is_some() { 1.0 } else { 0.0 };
                        let mut q = fill(
                            Bounds::new(
                                at(x - outer, y - outer),
                                size(px(outer * 2.), px(outer * 2.)),
                            ),
                            *color,
                        );
                        q.corner_radii = Corners::all(px(outer));
                        if let Some(ring) = ring {
                            q.border_widths = gpui::Edges::all(px(2.));
                            q.border_color = *ring;
                        }
                        window.paint_quad(q);
                    }
                    Shape::Arc {
                        cx,
                        cy,
                        outer,
                        inner,
                        from,
                        to,
                        color,
                    } => {
                        let pt = |r: f32, a: f32| at(cx + r * a.sin(), cy - r * a.cos());
                        let large = to - from > std::f32::consts::PI;
                        let mut b = PathBuilder::fill();
                        b.move_to(pt(*outer, *from));
                        b.arc_to(
                            point(px(*outer), px(*outer)),
                            px(0.),
                            large,
                            true,
                            pt(*outer, *to),
                        );
                        b.line_to(pt(*inner, *to));
                        b.arc_to(
                            point(px(*inner), px(*inner)),
                            px(0.),
                            large,
                            false,
                            pt(*inner, *from),
                        );
                        b.close();
                        if let Ok(path) = b.build() {
                            window.paint_path(path, *color);
                        }
                    }
                }
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

fn y_scale(domain: &Domain) -> impl Fn(f64) -> f32 + Clone + use<> {
    let (min, max) = (domain.min, domain.max);
    move |v| {
        let span = if max - min == 0.0 { 1.0 } else { max - min };
        TOP + (1.0 - ((v - min) / span) as f32) * (HEIGHT - TOP - BOTTOM)
    }
}

fn values(series: &Value) -> Vec<Option<f64>> {
    arr(series, "values").iter().map(Value::as_f64).collect()
}

/// One tooltip line: swatch, name, bold value.
struct TipLine {
    name: String,
    value: String,
    colour: Option<Hsla>,
}

impl<'a, 'w> B<'a, 'w> {
    fn legend(
        &self,
        names: &[String],
        vertical: bool,
        extra: Option<Vec<SharedString>>,
    ) -> AnyElement {
        let pal = self.pal;
        div()
            .flex()
            .when(vertical, |d| d.flex_col().gap(px(space::XS)))
            .when(!vertical, |d| {
                d.flex_wrap().gap_x(px(space::LG)).gap_y(px(space::XS))
            })
            .text_size(px(size::SMALL))
            .text_color(pal.c("ink-2"))
            .children(names.iter().enumerate().map(|(i, name)| {
                let tail = extra.as_ref().and_then(|e| e.get(i).cloned());
                div()
                    .flex()
                    .items_center()
                    .gap(px(space::XS))
                    .child(
                        div()
                            .size(px(SWATCH))
                            .mr(px(space::HAIR))
                            .rounded(px(radius::SM))
                            .bg(series(i)),
                    )
                    .child(name.clone())
                    .when_some(tail, |d, t| {
                        d.child(
                            div()
                                .font_weight(weight::STRONG)
                                .text_color(pal.c("ink"))
                                .child(t),
                        )
                    })
            }))
            .into_any_element()
    }

    /// Title, legend, the plot, and the numbers behind a `<details>`.
    fn chart_frame(
        &mut self,
        id: &str,
        title: &str,
        legend: Option<&[String]>,
        head: Vec<String>,
        rows: Vec<Vec<String>>,
        plot: AnyElement,
    ) -> AnyElement {
        let open = self.details_open(&format!("{id}#data"), false);
        let summary = self.summary(&format!("{id}#data"), self.tr("showData"), open);
        let data = open.then(|| {
            let head_el: Vec<AnyElement> = head
                .iter()
                .map(|h| div().child(h.clone()).into_any_element())
                .collect();
            let end = vec![false; head.len()];
            self.grid_table(
                &format!("{id}#datatable"),
                head_el,
                &head,
                &rows,
                &rows,
                &end,
            )
        });
        let legend = legend
            .filter(|l| l.len() > 1)
            .map(|l| self.legend(l, false, None));
        div()
            .flex()
            .flex_col()
            .gap(px(space::SM))
            .min_w_0()
            .child(div().font_weight(weight::STRONG).child(title.to_string()))
            .children(legend)
            .child(plot)
            .child(div().flex().flex_col().child(summary).children(data))
            .into_any_element()
    }

    fn tooltip(&self, x: f32, y: f32, width: f32, title: &str, lines: Vec<TipLine>) -> AnyElement {
        let pal = self.pal;
        let flip = x > width - 160.0;
        let body = div()
            .flex()
            .flex_col()
            .gap(px(space::HAIR))
            .px(px(space::MD))
            .py(px(space::SM))
            .rounded(px(radius::BASE))
            .border_1()
            .border_color(pal.c("line"))
            .bg(pal.c("surface-raised"))
            // A tip floats over the plot, so it keeps the pop shadow.
            .shadow(self.cx.global::<crate::theme::Theme>().pop_shadow())
            .text_size(px(size::SMALL))
            .whitespace_nowrap()
            .child(div().font_weight(weight::STRONG).child(title.to_string()))
            .children(lines.into_iter().map(|l| {
                div()
                    .flex()
                    .items_center()
                    .when_some(l.colour, |d, c| {
                        d.child(
                            div()
                                .size(px(SWATCH))
                                .mr(px(space::SM))
                                .rounded(px(radius::SM))
                                .bg(c),
                        )
                    })
                    .child(format!("{}: ", l.name))
                    .child(div().font_weight(weight::STRONG).child(l.value))
            }));
        // Flipped tips hang from their right edge: a full-width row aligned to the end.
        if flip {
            div()
                .absolute()
                .top(px(y))
                .left_0()
                .w(px(x - 12.0))
                .flex()
                .justify_end()
                .child(body)
                .into_any_element()
        } else {
            div()
                .absolute()
                .top(px(y))
                .left(px(x + 12.0))
                .child(body)
                .into_any_element()
        }
    }

    /// Gridlines, Y ticks, thinned X labels and the unit.
    fn axes(
        &self,
        domain: &Domain,
        width: f32,
        labels: &[String],
        x: impl Fn(usize) -> f32,
        shapes: &mut Vec<Shape>,
    ) -> Vec<AnyElement> {
        let pal = self.pal;
        let y = y_scale(domain);
        let mut out = Vec::new();
        for tick in &domain.ticks {
            let ty = y(*tick);
            let color = if *tick == 0.0 {
                pal.c("line-hover")
            } else {
                pal.c("line")
            };
            shapes.insert(
                0,
                Shape::Rect {
                    x: LEFT,
                    y: ty - 0.5,
                    w: (width - RIGHT - LEFT).max(0.0),
                    h: 1.0,
                    r: 0.0,
                    color,
                },
            );
            out.push(
                div()
                    .absolute()
                    .left_0()
                    .w(px(LEFT - 6.0))
                    .top(px(ty - 8.0))
                    .h(px(16.))
                    .flex()
                    .items_center()
                    .justify_end()
                    .text_size(px(size::META))
                    .text_color(pal.c("ink-3"))
                    .whitespace_nowrap()
                    .child(self.num(Some(*tick), None))
                    .into_any_element(),
            );
        }
        let keep = scale::thinned(labels.len(), width - LEFT - RIGHT, 64.0);
        for (i, label) in labels.iter().enumerate() {
            if !keep(i) {
                continue;
            }
            out.push(
                div()
                    .absolute()
                    .left(px(x(i) - 80.0))
                    .w(px(160.))
                    .top(px(HEIGHT - 8.0 - 12.0))
                    .h(px(16.))
                    .flex()
                    .justify_center()
                    .text_size(px(size::META))
                    .text_color(pal.c("ink-3"))
                    .whitespace_nowrap()
                    .child(label.clone())
                    .into_any_element(),
            );
        }
        out
    }

    fn unit_label(&self, unit: Option<&str>) -> Option<AnyElement> {
        unit.map(|u| {
            div()
                .absolute()
                .left(px(LEFT))
                .top(px(-3.))
                .text_size(px(size::META))
                .text_color(self.pal.c("ink-3"))
                .child(u.to_string())
                .into_any_element()
        })
    }

    fn data_rows(
        &self,
        labels: &[String],
        all: &[(String, Vec<Option<f64>>)],
        unit: Option<&str>,
    ) -> (Vec<String>, Vec<Vec<String>>) {
        let mut head = vec![String::new()];
        head.extend(all.iter().map(|(name, _)| name.clone()));
        let rows = labels
            .iter()
            .enumerate()
            .map(|(i, label)| {
                let mut row = vec![label.clone()];
                row.extend(
                    all.iter()
                        .map(|(_, v)| self.num(v.get(i).copied().flatten(), unit)),
                );
                row
            })
            .collect();
        (head, rows)
    }

    /// The plot box: probe, canvas, text, tooltip, pointer tracking.
    fn plot(
        &mut self,
        id: &str,
        width_key: &str,
        shapes: Vec<Shape>,
        text: Vec<AnyElement>,
        tip: Option<AnyElement>,
        hit: impl Fn(f32, f32, f32) -> Option<usize> + 'static,
    ) -> AnyElement {
        let (_, probe) = self.width_probe(width_key, 640.);
        let bounds = self.bounds_cell(&format!("{id}#plot"));
        let probe_bounds = bounds.clone();
        let track = canvas(move |b, _, _| probe_bounds.set(b), |_, _, _, _| {})
            .absolute()
            .top_0()
            .left_0()
            .size_full();
        let key = id.to_string();
        let leave = id.to_string();
        div()
            .id(eid(format!("{id}-plot")))
            .relative()
            .w_full()
            .h(px(HEIGHT))
            .child(probe)
            .child(track)
            .child(paint(shapes))
            .children(text)
            .children(tip)
            .on_mouse_move(self.cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                let b = bounds.get();
                let x = f32::from(ev.position.x - b.origin.x);
                let y = f32::from(ev.position.y - b.origin.y);
                let next = hit(x, y, f32::from(b.size.width));
                let now = this.ui.hover.get(&key).copied();
                if next != now {
                    match next {
                        Some(i) => this.ui.hover.insert(key.clone(), i),
                        None => this.ui.hover.remove(&key),
                    };
                    cx.notify();
                }
            }))
            .on_hover(self.cx.listener(move |this, hovered: &bool, _, cx| {
                if !*hovered && this.ui.hover.remove(&leave).is_some() {
                    cx.notify();
                }
            }))
            .into_any_element()
    }

    fn series_of(p: &Value) -> Vec<(String, Vec<Option<f64>>)> {
        arr(p, "series")
            .iter()
            .map(|s| {
                (
                    super::build::s(s, "name").unwrap_or_default().to_string(),
                    values(s),
                )
            })
            .collect()
    }

    fn labels_of(p: &Value, key: &str) -> Vec<String> {
        arr(p, key)
            .iter()
            .map(|v| v.as_str().unwrap_or_default().to_string())
            .collect()
    }

    pub(super) fn bar_chart(&mut self, p: &Value, id: &str) -> AnyElement {
        let labels = Self::labels_of(p, "labels");
        let all = Self::series_of(p);
        let unit = s(p, "unit");
        let stacked = super::build::flag(p, "stacked");
        let width_key = format!("{id}#chart");
        let width = self
            .ui
            .widths
            .get(&width_key)
            .map(|c| c.get())
            .unwrap_or(640.)
            .max(200.);
        let stacks: Vec<Vec<scale::Segment>> = (0..labels.len())
            .map(|i| {
                scale::stack(
                    &all.iter()
                        .map(|(_, v)| v.get(i).copied().flatten())
                        .collect::<Vec<_>>(),
                )
            })
            .collect();
        let extent: Vec<Option<f64>> = if stacked {
            stacks
                .iter()
                .flatten()
                .flat_map(|s| [Some(s.start), Some(s.end)])
                .collect()
        } else {
            all.iter().flat_map(|(_, v)| v.iter().copied()).collect()
        };
        let domain = scale::nice_domain(&extent, true);
        let y = y_scale(&domain);
        let n_labels = labels.len().max(1) as f32;
        let band = (width - LEFT - RIGHT) / n_labels;
        let centre = move |i: usize| LEFT + band * (i as f32 + 0.5);
        let group = (band * 0.72).min(72.0);
        let count = all.len().max(1) as f32;
        let bar = if stacked {
            group
        } else {
            ((group - (count - 1.0) * 2.0) / count).max(2.0)
        };
        let mut shapes = Vec::new();
        for (i, column) in stacks.iter().enumerate() {
            for (si, (_, vals)) in all.iter().enumerate() {
                let Some(value) = vals.get(i).copied().flatten() else {
                    continue;
                };
                let seg = column[si];
                let (from, to) = if stacked {
                    (seg.start, seg.end)
                } else {
                    (0.0, value)
                };
                let top = y(from).min(y(to));
                let left = if stacked {
                    centre(i) - bar / 2.0
                } else {
                    centre(i) - group / 2.0 + si as f32 * (bar + 2.0)
                };
                let h = ((y(from) - y(to)).abs() - if stacked { 2.0 } else { 0.0 }).max(1.0);
                shapes.push(Shape::Rect {
                    x: left,
                    y: top,
                    w: bar,
                    h,
                    r: 3f32.min(bar / 2.0),
                    color: series(si),
                });
            }
        }
        let mut text = self.axes(&domain, width, &labels, centre, &mut shapes);
        text.extend(self.unit_label(unit));
        let tip = self
            .ui
            .hover
            .get(id)
            .copied()
            .filter(|i| *i < labels.len())
            .map(|i| {
                let lines = all
                    .iter()
                    .enumerate()
                    .map(|(si, (name, v))| TipLine {
                        name: name.clone(),
                        value: self.num(v.get(i).copied().flatten(), unit),
                        colour: Some(series(si)),
                    })
                    .collect();
                self.tooltip(centre(i), TOP, width, &labels[i], lines)
            });
        let n = labels.len();
        let plot = self.plot(id, &width_key, shapes, text, tip, move |x, y, w| {
            let band = (w.max(200.) - LEFT - RIGHT) / n.max(1) as f32;
            if !(TOP..=HEIGHT - BOTTOM).contains(&y) || x < LEFT {
                return None;
            }
            let i = ((x - LEFT) / band).floor() as usize;
            (i < n).then_some(i)
        });
        let (head, rows) = self.data_rows(&labels, &all, unit);
        let names: Vec<String> = all.iter().map(|(n, _)| n.clone()).collect();
        self.chart_frame(
            id,
            s(p, "title").unwrap_or_default(),
            Some(&names),
            head,
            rows,
            plot,
        )
    }

    pub(super) fn line_chart(&mut self, p: &Value, id: &str) -> AnyElement {
        let labels = Self::labels_of(p, "labels");
        let all = Self::series_of(p);
        let unit = s(p, "unit");
        let area = super::build::flag(p, "area");
        let width_key = format!("{id}#chart");
        let width = self
            .ui
            .widths
            .get(&width_key)
            .map(|c| c.get())
            .unwrap_or(640.)
            .max(200.);
        let every: Vec<Option<f64>> = all.iter().flat_map(|(_, v)| v.iter().copied()).collect();
        let domain = scale::nice_domain(&every, area);
        let y = y_scale(&domain);
        let inner = width - LEFT - RIGHT;
        let n = labels.len();
        let x = move |i: usize| {
            LEFT + if n > 1 {
                inner * i as f32 / (n - 1) as f32
            } else {
                inner / 2.0
            }
        };
        let baseline = y(domain.min.max(0.0));
        let hover = self.ui.hover.get(id).copied().filter(|i| *i < n);
        let mut shapes = Vec::new();
        for (si, (_, vals)) in all.iter().enumerate() {
            let mut runs: Vec<Vec<(f32, f32)>> = Vec::new();
            let mut current = Vec::new();
            for (i, v) in vals.iter().enumerate() {
                match v {
                    Some(v) => current.push((x(i), y(*v))),
                    None if !current.is_empty() => runs.push(std::mem::take(&mut current)),
                    None => {}
                }
            }
            if !current.is_empty() {
                runs.push(current);
            }
            if area && all.len() == 1 {
                for run in &runs {
                    let mut pts = run.clone();
                    pts.push((run[run.len() - 1].0, baseline));
                    pts.push((run[0].0, baseline));
                    shapes.push(Shape::Area {
                        points: pts,
                        color: series(si).opacity(0.14),
                    });
                }
            }
            for run in runs {
                if run.len() == 1 {
                    shapes.push(Shape::Dot {
                        x: run[0].0,
                        y: run[0].1,
                        r: 3.0,
                        color: series(si),
                        ring: None,
                    });
                } else {
                    shapes.push(Shape::Line {
                        points: run,
                        width: 2.0,
                        color: series(si),
                    });
                }
            }
        }
        if let Some(h) = hover {
            shapes.push(Shape::Rect {
                x: x(h) - 0.5,
                y: TOP,
                w: 1.0,
                h: HEIGHT - TOP - BOTTOM,
                r: 0.0,
                color: self.pal.c("line-hover"),
            });
            for (si, (_, vals)) in all.iter().enumerate() {
                if let Some(v) = vals.get(h).copied().flatten() {
                    shapes.push(Shape::Dot {
                        x: x(h),
                        y: y(v),
                        r: 4.5,
                        color: series(si),
                        ring: Some(self.pal.c("surface")),
                    });
                }
            }
        }
        let mut text = self.axes(&domain, width, &labels, x, &mut shapes);
        text.extend(self.unit_label(unit));
        let tip = hover.map(|i| {
            let lines = all
                .iter()
                .enumerate()
                .map(|(si, (name, v))| TipLine {
                    name: name.clone(),
                    value: self.num(v.get(i).copied().flatten(), unit),
                    colour: Some(series(si)),
                })
                .collect();
            self.tooltip(x(i), TOP, width, &labels[i], lines)
        });
        let plot = self.plot(id, &width_key, shapes, text, tip, move |px_, _, w| {
            let inner = w.max(200.) - LEFT - RIGHT;
            if n == 0 {
                return None;
            }
            let ratio = (px_ - LEFT) / inner;
            Some(((ratio * (n.saturating_sub(1)) as f32).round().max(0.0) as usize).min(n - 1))
        });
        let (head, rows) = self.data_rows(&labels, &all, unit);
        let names: Vec<String> = all.iter().map(|(n, _)| n.clone()).collect();
        self.chart_frame(
            id,
            s(p, "title").unwrap_or_default(),
            Some(&names),
            head,
            rows,
            plot,
        )
    }

    pub(super) fn sparkline(&mut self, p: &Value, _id: &str) -> AnyElement {
        let vals: Vec<f64> = arr(p, "values").iter().filter_map(Value::as_f64).collect();
        let (w, h) = (120.0f32, 28.0f32);
        let min = vals.iter().copied().fold(f64::INFINITY, f64::min);
        let max = vals.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let span = if max - min == 0.0 { 1.0 } else { max - min };
        let n = vals.len();
        let points: Vec<(f32, f32)> = vals
            .iter()
            .enumerate()
            .map(|(i, v)| {
                (
                    2.0 + i as f32 * (w - 4.0) / (n.max(2) - 1) as f32,
                    h - 3.0 - ((v - min) / span) as f32 * (h - 6.0),
                )
            })
            .collect();
        let last = vals.last().copied();
        let shapes = if points.len() == 1 {
            vec![Shape::Line {
                points: vec![points[0], points[0]],
                width: 2.0,
                color: series(0),
            }]
        } else {
            vec![Shape::Line {
                points,
                width: 2.0,
                color: series(0),
            }]
        };
        div()
            .flex()
            .items_center()
            .gap(px(space::MD))
            .child(txt(
                s(p, "label").unwrap_or_default().to_string(),
                size::SMALL,
                weight::REGULAR,
                self.pal.c("ink-3"),
            ))
            .child(
                div()
                    .relative()
                    .w(px(w))
                    .h(px(h))
                    .flex_none()
                    .child(paint(shapes)),
            )
            .child(
                div()
                    .font_weight(weight::STRONG)
                    .child(self.num(last, s(p, "unit"))),
            )
            .into_any_element()
    }

    pub(super) fn donut(&mut self, p: &Value, id: &str) -> AnyElement {
        let segments: Vec<(String, f64)> = arr(p, "segments")
            .iter()
            .map(|s| {
                (
                    super::build::s(s, "label").unwrap_or_default().to_string(),
                    n(s, "value").unwrap_or(0.0),
                )
            })
            .collect();
        let unit = s(p, "unit");
        let total: f64 = segments.iter().map(|(_, v)| v).sum();
        let tau = std::f64::consts::TAU;
        let gap = if segments.iter().filter(|(_, v)| *v > 0.0).count() > 1 {
            0.02
        } else {
            0.0
        };
        let mut from = 0.0;
        let arcs: Vec<(f64, f64)> = segments
            .iter()
            .map(|(_, v)| {
                let sweep = if total > 0.0 { v / total * tau } else { 0.0 };
                let arc = (from, from + sweep);
                from += sweep;
                arc
            })
            .collect();
        let hover = self
            .ui
            .hover
            .get(id)
            .copied()
            .filter(|i| *i < segments.len());
        let shapes: Vec<Shape> = arcs
            .iter()
            .enumerate()
            .filter(|(_, (a, b))| b - a > gap)
            .map(|(i, (a, b))| {
                let (mut f, mut t) = (a + gap / 2.0, b - gap / 2.0);
                if t - f >= tau {
                    t = f + tau - 0.0001;
                }
                if t < f {
                    std::mem::swap(&mut f, &mut t);
                }
                let color = if hover.is_none() || hover == Some(i) {
                    series(i)
                } else {
                    series(i).opacity(0.45)
                };
                Shape::Arc {
                    cx: 90.,
                    cy: 90.,
                    outer: 86.,
                    inner: 58.,
                    from: f as f32,
                    to: t as f32,
                    color,
                }
            })
            .collect();
        let (centre_value, centre_label) = match hover {
            Some(i) => (
                self.num(Some(segments[i].1), unit),
                SharedString::from(segments[i].0.clone()),
            ),
            None => (self.num(Some(total), unit), self.tr("total")),
        };
        let bounds = self.bounds_cell(&format!("{id}#ring"));
        let probe_bounds = bounds.clone();
        let key = id.to_string();
        let leave = id.to_string();
        let ranges = arcs.clone();
        let ring = div()
            .id(eid(format!("{id}-ring")))
            .relative()
            .size(px(RING))
            .flex_none()
            .child(
                canvas(move |b, _, _| probe_bounds.set(b), |_, _, _, _| {})
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
            )
            .child(paint(shapes))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .flex()
                    .flex_col()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_size(px(size::TITLE))
                            .font_weight(weight::STRONG)
                            .child(centre_value),
                    )
                    .child(
                        div()
                            .max_w(px(RING_LABEL_W))
                            .truncate()
                            .text_size(px(size::META))
                            .text_color(self.pal.c("ink-3"))
                            .child(centre_label),
                    ),
            )
            .on_mouse_move(self.cx.listener(move |this, ev: &MouseMoveEvent, _, cx| {
                let b = bounds.get();
                let dx = f32::from(ev.position.x - b.origin.x) - 90.0;
                let dy = f32::from(ev.position.y - b.origin.y) - 90.0;
                let r = (dx * dx + dy * dy).sqrt();
                let next = if (58.0..=86.0).contains(&r) {
                    let mut a = (dx as f64).atan2(-dy as f64);
                    if a < 0.0 {
                        a += std::f64::consts::TAU;
                    }
                    ranges.iter().position(|(f, t)| a >= *f && a < *t)
                } else {
                    None
                };
                if next != this.ui.hover.get(&key).copied() {
                    match next {
                        Some(i) => this.ui.hover.insert(key.clone(), i),
                        None => this.ui.hover.remove(&key),
                    };
                    cx.notify();
                }
            }))
            .on_hover(self.cx.listener(move |this, hovered: &bool, _, cx| {
                if !*hovered && this.ui.hover.remove(&leave).is_some() {
                    cx.notify();
                }
            }));
        let names: Vec<String> = segments.iter().map(|(l, _)| l.clone()).collect();
        let pcts: Vec<SharedString> = segments
            .iter()
            .map(|(_, v)| {
                self.percent(if total > 0.0 {
                    (v / total * 100.0).round()
                } else {
                    0.0
                })
            })
            .collect();
        let legend = self.legend(&names, true, Some(pcts.clone()));
        let plot = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(px(space::SECTION))
            .child(ring)
            .child(legend)
            .into_any_element();
        let head = vec![
            String::new(),
            unit.map(str::to_string)
                .unwrap_or_else(|| self.tr("value").to_string()),
            "%".to_string(),
        ];
        let rows = segments
            .iter()
            .zip(&pcts)
            .map(|((l, v), pct)| vec![l.clone(), self.num(Some(*v), None), pct.to_string()])
            .collect();
        self.chart_frame(
            id,
            s(p, "title").unwrap_or_default(),
            None,
            head,
            rows,
            plot,
        )
    }

    pub(super) fn heatmap(&mut self, p: &Value, id: &str) -> AnyElement {
        let rows = Self::labels_of(p, "rows");
        let columns = Self::labels_of(p, "columns");
        let unit = s(p, "unit");
        let grid: Vec<Vec<Option<f64>>> = arr(p, "values")
            .iter()
            .map(|r| {
                r.as_array()
                    .map(|r| r.iter().map(Value::as_f64).collect())
                    .unwrap_or_default()
            })
            .collect();
        let known: Vec<f64> = grid.iter().flatten().flatten().copied().collect();
        let min = known.iter().copied().fold(f64::INFINITY, f64::min);
        let max = known.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let (min, max) = if known.is_empty() {
            (0.0, 1.0)
        } else {
            (min, max)
        };
        let pal = self.pal;
        let surface = pal.c("surface");
        let strength = |v: f64| {
            (12.0 + (v - min) / (if max - min == 0.0 { 1.0 } else { max - min }) * 88.0).round()
                as f32
                / 100.0
        };
        let head_w = rows
            .iter()
            .map(|r| self.measure(r, size::META, weight::REGULAR, ui::SANS))
            .fold(0.0f32, f32::max)
            .ceil()
            + 8.0;
        let head_w = head_w.max(48.0);
        let mut lines: Vec<AnyElement> = Vec::new();
        lines.push(
            div()
                .flex()
                .gap(px(space::HAIR))
                .child(div().w(px(head_w)).flex_none())
                .children(columns.iter().map(|c| {
                    div()
                        .flex_1()
                        .min_w(px(CELL))
                        .truncate()
                        .flex()
                        .justify_center()
                        .text_color(pal.c("ink-3"))
                        .child(c.clone())
                }))
                .into_any_element(),
        );
        for (r, row) in rows.iter().enumerate() {
            let cells: Vec<AnyElement> = columns
                .iter()
                .enumerate()
                .map(|(c, col)| {
                    let value = grid.get(r).and_then(|row| row.get(c)).copied().flatten();
                    let title = format!("{row} · {col}: {}", self.num(value, unit));
                    div()
                        .id(eid(format!("{id}-cell-{r}-{c}")))
                        .flex_1()
                        .min_w(px(CELL))
                        .min_h(px(CELL))
                        .rounded(px(radius::SM))
                        .map(|d| match value {
                            Some(v) => d.bg(mix_oklab(series(0), surface, strength(v))),
                            None => d
                                .bg(pal.c("surface-2"))
                                .border_1()
                                .border_color(pal.c("line")),
                        })
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(title.clone())
                                .build(window, cx)
                        })
                        .into_any_element()
                })
                .collect();
            lines.push(
                div()
                    .flex()
                    .gap(px(space::HAIR))
                    .child(
                        div()
                            .w(px(head_w))
                            .flex_none()
                            .pr(px(space::MD))
                            .whitespace_nowrap()
                            .text_color(pal.c("ink-3"))
                            .child(row.clone()),
                    )
                    .children(cells)
                    .into_any_element(),
            );
        }
        let scale_row = div()
            .flex()
            .items_center()
            .gap(px(space::MD))
            .text_size(px(size::META))
            .text_color(pal.c("ink-3"))
            .child(self.num(Some(min), unit))
            .child(
                // The scale in the same flat steps the cells are painted in.
                div()
                    .flex()
                    .gap(px(space::HAIR))
                    .children((0..SCALE_STEPS).map(|i| {
                        let t = i as f32 / (SCALE_STEPS - 1) as f32;
                        div()
                            .w(px(space::XL))
                            .h(px(space::MD))
                            .rounded(px(radius::SM))
                            .bg(mix_oklab(series(0), surface, 0.12 + t * 0.88))
                    })),
            )
            .child(self.num(Some(max), unit));
        let plot = div()
            .flex()
            .flex_col()
            .gap(px(space::SM))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(space::HAIR))
                    .text_size(px(size::META))
                    .children(lines),
            )
            .child(scale_row)
            .into_any_element();
        let mut head = vec![String::new()];
        head.extend(columns.iter().cloned());
        let table = rows
            .iter()
            .enumerate()
            .map(|(r, row)| {
                let mut out = vec![row.clone()];
                out.extend((0..columns.len()).map(|c| {
                    self.num(grid.get(r).and_then(|x| x.get(c)).copied().flatten(), unit)
                }));
                out
            })
            .collect();
        self.chart_frame(
            id,
            s(p, "title").unwrap_or_default(),
            None,
            head,
            table,
            plot,
        )
    }

    // ------------------------------------------------------------ compare

    /// An image at an exact width, its height from its own proportions.
    fn picture_at(&self, file: &crate::model::Attachment, width: f32) -> gpui::Div {
        let pal = self.pal;
        match self.ui.images.get(&file.id) {
            Some(ImageLoad::Ready(image, w, h)) => {
                let height = width * *h as f32 / (*w as f32).max(1.0);
                div().child(
                    gpui::img(image.clone())
                        .w(px(width))
                        .h(px(height))
                        .rounded(px(radius::BASE))
                        .border_1()
                        .border_color(pal.c("line"))
                        .bg(pal.c("surface")),
                )
            }
            Some(ImageLoad::Failed) => div().child(self.missing(&file.name)),
            _ => div()
                .w(px(width))
                .h(px(PLACEHOLDER_H))
                .rounded(px(radius::BASE))
                .border_1()
                .border_color(pal.c("line")),
        }
    }

    pub(super) fn before_after(&mut self, p: &Value, id: &str) -> AnyElement {
        let before_name = s(p, "before").unwrap_or_default();
        let after_name = s(p, "after").unwrap_or_default();
        let Some(before) = self.image_file(before_name) else {
            return self.missing(before_name);
        };
        let Some(after) = self.image_file(after_name) else {
            return self.missing(after_name);
        };
        let pal = self.pal;
        let (slider, at) = *self
            .ui
            .compare
            .entry(id.to_string())
            .or_insert((s(p, "mode") == Some("slider"), 50.0));
        let (width, probe) = self.width_probe(&format!("{id}#compare"), 600.);
        let tab =
            |b: &mut Self, on: bool, icon: &'static str, label: SharedString, to_slider: bool| {
                let key = id.to_string();
                div()
                    .id(eid(format!("{id}-mode-{to_slider}")))
                    .h(px(control::SM))
                    .px(px(space::MD))
                    .flex()
                    .items_center()
                    .gap(px(space::SM))
                    .rounded(px(radius::BASE))
                    .text_size(px(size::SMALL))
                    .font_weight(if on { weight::STRONG } else { weight::REGULAR })
                    .text_color(if on { pal.c("ink") } else { pal.c("ink-3") })
                    .cursor_pointer()
                    .when(on, |d| d.bg(pal.c("surface")))
                    .child(ui::icon(
                        icon,
                        icon_size::ROW,
                        if on { pal.c("ink") } else { pal.c("ink-3") },
                    ))
                    .child(label)
                    .on_click(b.cx.listener(
                        move |this: &mut HuginnView, _, _, cx: &mut Context<HuginnView>| {
                            this.ui
                                .compare
                                .entry(key.clone())
                                .or_insert((false, 50.0))
                                .0 = to_slider;
                            cx.notify();
                        },
                    ))
            };
        let side_label = crate::i18n::t(self.cx, "attachments.sideBySide");
        let slider_label = crate::i18n::t(self.cx, "attachments.slider");
        let tabs = div().flex().child(
            div()
                .flex()
                .gap(px(space::HAIR))
                .p(px(space::HAIR))
                .mb(px(space::MD))
                .rounded(px(radius::MD))
                .bg(pal.c("control"))
                .child(tab(self, !slider, "columns-2", side_label, false))
                .child(tab(self, slider, "chevrons-left-right", slider_label, true)),
        );
        let caption = |b: &Self, a: &crate::model::Attachment| {
            let key = if a.label.as_deref() == Some("before") {
                "attachments.before"
            } else {
                "attachments.after"
            };
            txt(
                crate::i18n::t(b.cx, key),
                size::SMALL,
                weight::STRONG,
                pal.c("ink-3"),
            )
            .mt(px(space::XS))
        };
        let body: AnyElement = if !slider {
            let col = ((width - 8.0) / 2.0).max(1.0);
            div()
                .flex()
                .gap(px(space::MD))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .w(px(col))
                        .child(self.picture_at(before, col))
                        .child(caption(self, before)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .w(px(col))
                        .child(self.picture_at(after, col))
                        .child(caption(self, after)),
                )
                .into_any_element()
        } else {
            let cut = width * at / 100.0;
            let key = format!("{id}#range");
            let id_owned = id.to_string();
            let range = self.slider_track(
                &key,
                at as f64,
                0.0,
                100.0,
                1.0,
                false,
                move |this, v, cx| {
                    this.ui
                        .compare
                        .entry(id_owned.clone())
                        .or_insert((true, 50.0))
                        .1 = v as f32;
                    cx.notify();
                },
            );
            div()
                .flex()
                .flex_col()
                .child(
                    div()
                        .relative()
                        .child(self.picture_at(before, width))
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(px(cut))
                                .w(px((width - cut).max(0.0)))
                                .overflow_hidden()
                                .child(
                                    div()
                                        .absolute()
                                        .top_0()
                                        .left(px(-cut))
                                        .child(self.picture_at(after, width)),
                                ),
                        )
                        .child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(px(cut - 1.0))
                                .w(px(2.))
                                .bg(pal.c("accent")),
                        ),
                )
                .child(
                    div()
                        .mt(px(space::XS))
                        .flex()
                        .items_center()
                        .gap(px(space::MD))
                        .text_size(px(size::SMALL))
                        .font_weight(weight::STRONG)
                        .text_color(pal.c("ink-3"))
                        .child(crate::i18n::t(self.cx, "attachments.before"))
                        .child(div().flex_1().child(range))
                        .child(crate::i18n::t(self.cx, "attachments.after")),
                )
                .into_any_element()
        };
        div()
            .p(px(space::MD))
            .rounded(px(radius::MD))
            .border_1()
            .border_color(pal.c("line-strong"))
            .bg(pal.c("surface-2"))
            .flex()
            .flex_col()
            .child(tabs)
            .child(div().relative().w_full().child(probe).child(body))
            .into_any_element()
    }
}

/// `color-mix(in oklab, a t, b)`.
pub(super) fn mix_oklab(a: Hsla, b: Hsla, t: f32) -> Hsla {
    fn to_lin(c: f32) -> f32 {
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    }
    fn from_lin(c: f32) -> f32 {
        if c <= 0.003_130_8 {
            c * 12.92
        } else {
            1.055 * c.powf(1.0 / 2.4) - 0.055
        }
    }
    fn to_lab(c: gpui::Rgba) -> [f32; 3] {
        let (r, g, b) = (to_lin(c.r), to_lin(c.g), to_lin(c.b));
        let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
        let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
        let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
        [
            0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
            1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
            0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
        ]
    }
    fn from_lab(lab: [f32; 3]) -> [f32; 3] {
        let l = (lab[0] + 0.396_337_78 * lab[1] + 0.215_803_76 * lab[2]).powi(3);
        let m = (lab[0] - 0.105_561_346 * lab[1] - 0.063_854_17 * lab[2]).powi(3);
        let s = (lab[0] - 0.089_484_18 * lab[1] - 1.291_485_5 * lab[2]).powi(3);
        [
            from_lin(4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s),
            from_lin(-1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s),
            from_lin(-0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s),
        ]
    }
    let (ra, rb) = (a.to_rgb(), b.to_rgb());
    let (la, lb) = (to_lab(ra), to_lab(rb));
    let mixed = [0, 1, 2].map(|i| la[i] * t + lb[i] * (1.0 - t));
    let [r, g, bl] = from_lab(mixed);
    gpui::Rgba {
        r: r.clamp(0., 1.),
        g: g.clamp(0., 1.),
        b: bl.clamp(0., 1.),
        a: ra.a * t + rb.a * (1.0 - t),
    }
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oklab_mix_ends_at_its_inputs() {
        let a: Hsla = gpui::rgb(0x2a78d6).into();
        let b: Hsla = gpui::rgb(0xffffff).into();
        let full = mix_oklab(a, b, 1.0).to_rgb();
        let ar = a.to_rgb();
        assert!(
            (full.r - ar.r).abs() < 0.01
                && (full.g - ar.g).abs() < 0.01
                && (full.b - ar.b).abs() < 0.01
        );
        let none = mix_oklab(a, b, 0.0).to_rgb();
        assert!(none.r > 0.99 && none.g > 0.99 && none.b > 0.99);
    }
}
