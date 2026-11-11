//! The performance overlay (⌥⌘P, View → Performance): how often the main
//! window draws, how long a frame takes, who asked for the redraws, and what
//! the process costs in CPU and memory. For development; off at launch
//! (`HARBARD_APP_PERF=1` opens it), and it remembers nothing past the run.
//!
//! It reads [`crate::perf`]'s counters and GPUI's frame histogram for this
//! window once a second. That refresh is itself one frame a second, listed
//! as "this overlay"; closed, it asks for none.

use crate::perf::{self, Frames, History, Sample, Snapshot};
use crate::robot::clock;
use crate::theme::Theme;
use crate::ui::{self, icon_size, radius, size, space};
use gpui::prelude::FluentBuilder as _;
use gpui::profiler::FrameDurationSnapshot;
use gpui::{
    AnyWindowHandle, App, AppContext as _, Context, Global, Hsla, InteractiveElement, IntoElement,
    ParentElement, Render, StatefulInteractiveElement, Styled, Task, Window, div, px,
};
use std::time::Duration;

/// Whether the overlay is open, for this run of the app.
pub struct PerfOn(pub bool);

impl Global for PerfOn {}

pub fn init(cx: &mut App) {
    let on = std::env::var_os("HARBARD_APP_PERF").is_some_and(|v| v != "0");
    cx.set_global(PerfOn(on));
}

pub fn toggle(cx: &mut App) {
    let on = cx.try_global::<PerfOn>().is_some_and(|p| p.0);
    cx.set_global(PerfOn(!on));
}

fn is_on(cx: &App) -> bool {
    cx.try_global::<PerfOn>().is_some_and(|p| p.0)
}

/// Frames a second the sparkline is scaled to at least, so a quiet minute
/// reads as quiet.
const SPARK_FLOOR: f32 = 10.0;
const WIDTH: f32 = 272.0;

pub struct PerfHud {
    /// The window whose frames it counts: the one it is drawn in.
    window: AnyWindowHandle,
    ticker: Option<Task<()>>,
    prev: Option<Snapshot>,
    prev_frames: Option<FrameDurationSnapshot>,
    last: Option<Sample>,
    history: History,
    footprint: Option<u64>,
}

impl PerfHud {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        cx.observe_global::<PerfOn>(|h, cx| h.pace(cx)).detach();
        let mut hud = PerfHud {
            window: window.window_handle(),
            ticker: None,
            prev: None,
            prev_frames: None,
            last: None,
            history: History::default(),
            footprint: None,
        };
        hud.pace(cx);
        hud
    }

    /// Start the once-a-second reading while open; drop it when closed.
    fn pace(&mut self, cx: &mut Context<Self>) {
        if !is_on(cx) {
            self.ticker = None;
            self.prev = None;
            self.prev_frames = None;
            self.last = None;
            self.history = History::default();
            cx.notify();
            return;
        }
        if self.ticker.is_some() {
            return;
        }
        self.footprint = perf::footprint();
        let handle = self.window;
        self.ticker = Some(cx.spawn(async move |this, cx| {
            loop {
                let frames = cx
                    .update_window(handle, |_, window, _| window.frame_duration_snapshot())
                    .ok();
                let go_on = this.update(cx, |h, cx| {
                    h.read(frames);
                    perf::count(perf::Source::Overlay, 1);
                    cx.notify();
                });
                if go_on.is_err() {
                    break;
                }
                cx.background_executor().timer(Duration::from_secs(1)).await;
            }
        }));
        cx.notify();
    }

    /// Take this second's reading: the counters, and the draws GPUI timed
    /// since the last one.
    fn read(&mut self, frames: Option<FrameDurationSnapshot>) {
        let now = perf::snapshot();
        if let (Some(prev), Some(prev_frames), Some(frames)) =
            (&self.prev, &self.prev_frames, &frames)
        {
            let s = perf::sample(prev, &now, frames_between(prev_frames, frames));
            // `HARBARD_APP_PERF_LOG=1`: each reading on stderr too, to check
            // the overlay against GPUI's `ZED_MEASUREMENTS`.
            if std::env::var_os("HARBARD_APP_PERF_LOG").is_some() {
                eprintln!(
                    "perf: {:.1} fps, {:.2} ms avg, {:.2} ms worst, cpu {:.1} %",
                    s.fps, s.avg_ms, s.worst_ms, s.cpu_pct
                );
            }
            self.history.push(s.fps);
            self.last = Some(s);
        }
        self.prev = Some(now);
        self.prev_frames = frames;
        self.footprint = perf::footprint();
    }
}

/// The draws recorded between two of GPUI's cumulative histograms.
fn frames_between(prev: &FrameDurationSnapshot, now: &FrameDurationSnapshot) -> Frames {
    let mut d = now.draw_duration_histogram.clone();
    if d.subtract(&prev.draw_duration_histogram).is_err() || d.is_empty() {
        return Frames::default();
    }
    Frames {
        count: d.len(),
        mean_ns: d.mean(),
        max_ns: d.max(),
    }
}

/// What the robots are doing in this window, in the overlay's words.
fn motion(window: &Window, cx: &App) -> &'static str {
    if clock::reduced(cx) {
        "still · reduced motion"
    } else if !window.is_window_active() {
        "still · in background"
    } else {
        "moving"
    }
}

impl Render for PerfHud {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !is_on(cx) {
            return div().into_any_element();
        }
        let theme = cx.global::<Theme>().clone();
        let ink = theme.c("ink");
        let dim = theme.c("ink-3");
        let line = |label: &str, value: String, color: Hsla| {
            div()
                .flex()
                .gap(px(space::MD))
                .child(
                    div()
                        .w(px(56.))
                        .flex_none()
                        .text_color(dim)
                        .child(label.to_string()),
                )
                .child(div().flex_1().min_w_0().text_color(color).child(value))
        };
        let s = self.last.clone();
        let fps = s.as_ref().map_or(0.0, |s| s.fps);
        let values = self.history.values();
        // Two seconds a bar, so the minute fits on one line.
        let spark = perf::sparkline(&perf::pair_peaks(&values), SPARK_FLOOR);
        // A frame over one display refresh (16.7 ms) would show as a stutter.
        let worst = s.as_ref().map_or(0.0, |s| s.worst_ms);
        let worst_tone = if worst > 16.7 {
            theme.c("red")
        } else if worst > 8.0 {
            theme.c("amber")
        } else {
            ink
        };
        let frame = s.as_ref().map_or_else(
            || "reading…".to_string(),
            |s| format!("{:.1} ms avg · ", s.avg_ms),
        );
        let sources = s.as_ref().map(|s| s.sources.clone()).unwrap_or_default();

        div()
            .id("hb-perf")
            .absolute()
            .left(px(super::RAIL + space::MD))
            .bottom(px(space::LG))
            .w(px(WIDTH))
            .occlude()
            .flex()
            .flex_col()
            .gap(px(space::XS))
            .px(px(space::LG))
            .py(px(space::MD))
            .rounded(px(radius::MD))
            .bg(theme.c("surface-raised"))
            .shadow(theme.pop_shadow())
            .font_family(ui::MONO)
            .text_size(px(size::META))
            .line_height(px(ui::leading::META))
            .text_color(ink)
            .child(
                div()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .text_color(dim)
                            .text_size(px(size::MICRO))
                            .child("PERFORMANCE · ⌥⌘P"),
                    )
                    .child(
                        div()
                            .id("hb-perf-close")
                            .cursor_pointer()
                            .child(ui::icon("x", icon_size::INLINE, dim))
                            .on_click(|_, _, cx| toggle(cx)),
                    ),
            )
            .child(
                div()
                    .text_size(px(size::UI))
                    .text_color(ink)
                    .child(format!("{fps:.1} fps")),
            )
            .child(
                div()
                    .w_full()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_color(theme.c("ink-2"))
                    .child(spark),
            )
            .child(
                div()
                    .text_color(dim)
                    .text_size(px(size::MICRO))
                    .child(format!(
                        "last {} s · peak {:.0} fps",
                        values.len(),
                        self.history.peak()
                    )),
            )
            .child(
                div()
                    .flex()
                    .gap(px(space::MD))
                    .child(div().w(px(56.)).flex_none().text_color(dim).child("frame"))
                    .child(div().text_color(ink).child(frame))
                    .when_some(s.as_ref(), |d, _| {
                        d.child(
                            div()
                                .text_color(worst_tone)
                                .child(format!("{worst:.1} ms worst")),
                        )
                    }),
            )
            .child(line(
                "cpu",
                s.as_ref()
                    .map_or("—".into(), |s| format!("{:.1} %", s.cpu_pct)),
                ink,
            ))
            .child(line(
                "memory",
                self.footprint.map_or("—".into(), perf::fmt_bytes),
                ink,
            ))
            .child(line("motion", motion(window, cx).to_string(), ink))
            .child(
                div()
                    .pt(px(space::XS))
                    .text_color(dim)
                    .text_size(px(size::MICRO))
                    .child("redraws asked, last second"),
            )
            .children(if sources.is_empty() {
                vec![div().text_color(dim).child("none").into_any_element()]
            } else {
                sources
                    .into_iter()
                    .map(|(src, n)| {
                        div()
                            .flex()
                            .child(div().flex_1().text_color(dim).child(src.label()))
                            .child(div().text_color(ink).child(n.to_string()))
                            .into_any_element()
                    })
                    .collect()
            })
            .into_any_element()
    }
}
