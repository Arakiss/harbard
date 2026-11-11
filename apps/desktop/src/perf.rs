//! Developer performance counters, read by the performance overlay (⌥⌘P,
//! View → Performance).
//!
//! Redraw requests are counted always, at the cost of one atomic add each.
//! Frames come from GPUI's own profiler (its `profiler` feature): every
//! window records how long each `Window::draw` took, from building the
//! element tree to painting it, in a histogram, and the overlay diffs that
//! histogram once a second. It covers the app's side of a frame; the GPU's
//! work and the presentation after it are not in it. Nothing here asks for
//! a frame.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Who asked for a redraw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The robot clock, for a robot's face, blink or breath.
    Robots,
    /// The robot clock, for a status icon's loop.
    Icons,
    /// The robot clock, for a status line's timer or words.
    Timers,
    /// A robot's reaction or hover tilt, or a blocked sign's shake, frame
    /// by frame.
    Reactions,
    /// The rail's living logo.
    Logo,
    /// The season lights and snow in the frame.
    Lights,
    /// Lobby's arrivals and lifts.
    Lobby,
    /// A new reading of the bus.
    Bus,
    /// The Strip window's clock.
    Strip,
    /// An update's step clock and its readings, while a run goes on.
    Update,
    /// This overlay's own once-a-second refresh.
    Overlay,
}

pub const SOURCES: [Source; 11] = [
    Source::Robots,
    Source::Icons,
    Source::Timers,
    Source::Reactions,
    Source::Logo,
    Source::Lights,
    Source::Lobby,
    Source::Bus,
    Source::Strip,
    Source::Update,
    Source::Overlay,
];

impl Source {
    fn index(self) -> usize {
        SOURCES.iter().position(|s| *s == self).unwrap_or(0)
    }

    pub fn label(self) -> &'static str {
        match self {
            Source::Robots => "robots",
            Source::Icons => "status icons",
            Source::Timers => "timers",
            Source::Reactions => "reactions",
            Source::Logo => "logo",
            Source::Lights => "lights",
            Source::Lobby => "lobby motion",
            Source::Bus => "bus updates",
            Source::Strip => "strip",
            Source::Update => "update",
            Source::Overlay => "this overlay",
        }
    }
}

static COUNTS: [AtomicU64; SOURCES.len()] = [const { AtomicU64::new(0) }; SOURCES.len()];

/// `source` asked for `n` redraws (a clock firing for `n` views counts `n`).
pub fn count(source: Source, n: u64) {
    COUNTS[source.index()].fetch_add(n, Ordering::Relaxed);
}

/// The frames a window drew between two readings.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Frames {
    pub count: u64,
    pub mean_ns: f64,
    pub max_ns: u64,
}

/// The counters at one moment.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub at: Instant,
    pub counts: [u64; SOURCES.len()],
    /// CPU time used by the whole process (all threads), in µs.
    pub cpu_us: u64,
}

pub fn snapshot() -> Snapshot {
    Snapshot {
        at: Instant::now(),
        counts: std::array::from_fn(|i| COUNTS[i].load(Ordering::Relaxed)),
        cpu_us: cpu_time_us(),
    }
}

/// One second (or so) of the app.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    pub fps: f32,
    pub avg_ms: f32,
    pub worst_ms: f32,
    /// Share of one core, as Activity Monitor shows it (100 = one core).
    pub cpu_pct: f32,
    /// Redraw requests by source, most first; sources at zero left out.
    pub sources: Vec<(Source, u64)>,
}

/// The counters between `prev` and `now`, with the `frames` the window drew
/// meanwhile.
pub fn sample(prev: &Snapshot, now: &Snapshot, frames: Frames) -> Sample {
    let secs = now
        .at
        .saturating_duration_since(prev.at)
        .max(Duration::from_millis(1))
        .as_secs_f32();
    let mut sources: Vec<(Source, u64)> = SOURCES
        .iter()
        .map(|s| {
            let i = s.index();
            (*s, now.counts[i].saturating_sub(prev.counts[i]))
        })
        .filter(|(_, n)| *n > 0)
        .collect();
    sources.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.index().cmp(&b.0.index())));
    Sample {
        fps: frames.count as f32 / secs,
        avg_ms: if frames.count == 0 {
            0.0
        } else {
            (frames.mean_ns / 1e6) as f32
        },
        worst_ms: frames.max_ns as f32 / 1e6,
        cpu_pct: now.cpu_us.saturating_sub(prev.cpu_us) as f32 / (secs * 1e6) * 100.0,
        sources,
    }
}

/// The last minute of frames per second, one value a sample.
#[derive(Default)]
pub struct History {
    fps: VecDeque<f32>,
}

impl History {
    pub const LEN: usize = 60;

    pub fn push(&mut self, fps: f32) {
        if self.fps.len() == Self::LEN {
            self.fps.pop_front();
        }
        self.fps.push_back(fps);
    }

    pub fn values(&self) -> Vec<f32> {
        self.fps.iter().copied().collect()
    }

    pub fn peak(&self) -> f32 {
        self.fps.iter().copied().fold(0.0, f32::max)
    }
}

/// A one-line chart of `values`: eight block heights, scaled to the larger
/// of their peak and `floor` (so a quiet minute reads as quiet, not as a
/// wall of full blocks), zero as the lowest block.
pub fn sparkline(values: &[f32], floor: f32) -> String {
    const BARS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let top = values
        .iter()
        .copied()
        .fold(floor, f32::max)
        .max(f32::EPSILON);
    values
        .iter()
        .map(|v| {
            let k = ((v.max(0.0) / top) * 7.0).round() as usize;
            BARS[k.min(7)]
        })
        .collect()
}

/// `values` two by two, each pair as its larger value (a lone last value
/// stands for itself): half as many bars, and no spike lost.
pub fn pair_peaks(values: &[f32]) -> Vec<f32> {
    values
        .chunks(2)
        .map(|c| c.iter().copied().fold(f32::MIN, f32::max))
        .collect()
}

/// `412 MB`, `1.2 GB`.
pub fn fmt_bytes(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    if mb >= 1024.0 {
        format!("{:.1} GB", mb / 1024.0)
    } else {
        format!("{mb:.0} MB")
    }
}

/// CPU time the process has used, user and system, in µs.
pub fn cpu_time_us() -> u64 {
    // SAFETY: getrusage fills the struct it is given; zeroed is a valid start.
    let mut ru: libc::rusage = unsafe { std::mem::zeroed() };
    if unsafe { libc::getrusage(libc::RUSAGE_SELF, &mut ru) } != 0 {
        return 0;
    }
    let us = |t: libc::timeval| t.tv_sec as u64 * 1_000_000 + t.tv_usec as u64;
    us(ru.ru_utime) + us(ru.ru_stime)
}

/// The process's memory footprint, the figure Activity Monitor's Memory
/// column shows (`phys_footprint`).
#[cfg(target_os = "macos")]
pub fn footprint() -> Option<u64> {
    // SAFETY: proc_pid_rusage writes a rusage_info_v2 into the buffer.
    let mut info: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::proc_pid_rusage(
            std::process::id() as i32,
            libc::RUSAGE_INFO_V2,
            (&mut info as *mut libc::rusage_info_v2).cast(),
        )
    };
    (rc == 0).then_some(info.ri_phys_footprint)
}

#[cfg(not(target_os = "macos"))]
pub fn footprint() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(at: Instant, cpu_us: u64) -> Snapshot {
        Snapshot {
            at,
            counts: [0; SOURCES.len()],
            cpu_us,
        }
    }

    #[test]
    fn a_sample_turns_two_snapshots_into_rates() {
        let t0 = Instant::now();
        let a = snap(t0, 1_000_000);
        let mut b = snap(t0 + Duration::from_secs(2), 1_100_000);
        b.counts[Source::Robots.index()] = 9;
        b.counts[Source::Timers.index()] = 2;
        b.counts[Source::Overlay.index()] = 2;
        let frames = Frames {
            count: 12,
            mean_ns: 2_000_000.0,
            max_ns: 5_000_000,
        };
        let s = sample(&a, &b, frames);
        assert_eq!(s.fps, 6.0);
        assert_eq!(s.avg_ms, 2.0);
        assert_eq!(s.worst_ms, 5.0);
        assert!((s.cpu_pct - 5.0).abs() < 1e-3, "{}", s.cpu_pct);
        // Most first; ties keep the declared order.
        assert_eq!(
            s.sources,
            vec![
                (Source::Robots, 9),
                (Source::Timers, 2),
                (Source::Overlay, 2)
            ]
        );
    }

    #[test]
    fn a_still_second_has_no_frames_and_no_sources() {
        let t0 = Instant::now();
        let a = snap(t0, 10);
        let b = snap(t0 + Duration::from_secs(1), 10);
        let s = sample(&a, &b, Frames::default());
        assert_eq!((s.fps, s.avg_ms, s.cpu_pct), (0.0, 0.0, 0.0));
        assert!(s.sources.is_empty());
    }

    #[test]
    fn the_history_keeps_the_last_minute() {
        let mut h = History::default();
        for i in 0..75 {
            h.push(i as f32);
        }
        let v = h.values();
        assert_eq!(v.len(), History::LEN);
        assert_eq!((v[0], v[59]), (15.0, 74.0));
        assert_eq!(h.peak(), 74.0);
    }

    #[test]
    fn the_sparkline_scales_to_its_peak_or_floor() {
        assert_eq!(sparkline(&[0.0, 30.0, 60.0], 10.0), "▁▅█");
        // Under the floor, small values stay low.
        assert_eq!(sparkline(&[0.0, 1.0, 2.0], 20.0), "▁▁▂");
        assert_eq!(sparkline(&[], 10.0), "");
    }

    #[test]
    fn pairs_keep_their_peak() {
        assert_eq!(pair_peaks(&[1.0, 9.0, 4.0, 2.0, 7.0]), vec![9.0, 4.0, 7.0]);
        assert!(pair_peaks(&[]).is_empty());
        // A full minute is thirty bars.
        assert_eq!(pair_peaks(&[0.0; History::LEN]).len(), 30);
    }

    #[test]
    fn bytes_read_in_megabytes_then_gigabytes() {
        assert_eq!(fmt_bytes(235 * 1024 * 1024), "235 MB");
        assert_eq!(fmt_bytes(1536 * 1024 * 1024), "1.5 GB");
    }

    #[test]
    fn this_process_reports_cpu_time_and_memory() {
        assert!(cpu_time_us() > 0);
        #[cfg(target_os = "macos")]
        assert!(footprint().is_some_and(|b| b > 1024 * 1024));
    }
}
