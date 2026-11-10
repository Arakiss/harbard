//! The status line under a robot on the shelf: the microstate, the words
//! and the timer (`microState`, `statusParts`, `elapsed` in
//! `lib/activity.ts`).
//!
//! TODO(chat lane): the robot and its status line live in `src/robot/`; when
//! it exports these, the strip should call them and this file can go.

use crate::i18n::{t, tv};
use crate::model::{Activity, Thresholds};
use crate::ui::format::hhmm;
use gpui::App;

/// A state with its microstate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Micro {
    pub sleepy: bool,
    /// A microstate that asks for attention (long run, too long asleep).
    pub long: bool,
    /// "Thinking for a long time", or None when the state reads as itself.
    pub label: Option<String>,
}

pub fn micro_state(a: &Activity, now: u64, limits: &Thresholds, cx: &App) -> Micro {
    let age = now.saturating_sub(a.since_ms);
    let label = |k: &str| Some(t(cx, &format!("activity.{k}")).to_string());
    match a.state.as_str() {
        "thinking" if age >= limits.thinking_long_ms => Micro {
            label: label("microThinkingLong"),
            ..Micro::default()
        },
        "waiting" if age >= limits.amber_ms => Micro {
            label: label("microAsleepWaiting"),
            ..Micro::default()
        },
        "waiting" if age >= limits.waiting_sleepy_ms => Micro {
            sleepy: true,
            label: label("microSleepyWaiting"),
            ..Micro::default()
        },
        "running" | "testing" if age >= limits.running_long_ms => Micro {
            long: true,
            label: label("microRunningLong"),
            ..Micro::default()
        },
        "asleep" if age >= limits.red_ms => Micro {
            long: true,
            label: label("microAsleepLong"),
            ..Micro::default()
        },
        _ => Micro::default(),
    }
}

/// "8 s", "2 min", "1 h", "1 h 5 min".
pub fn elapsed(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        return format!("{s} s");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m} min");
    }
    let h = m / 60;
    if m.is_multiple_of(60) {
        format!("{h} h")
    } else {
        format!("{h} h {} min", m % 60)
    }
}

/// The status line's words as one string (the strip shows code labels in
/// the same face): "Running cargo test", "On the web · docs.rs".
pub fn status_text(a: &Activity, reports: usize, micro: &Micro, cx: &App) -> String {
    let k = |key: &str| t(cx, &format!("activity.{key}")).to_string();
    let kv = |key: &str, vars: &[(&str, &str)]| tv(cx, &format!("activity.{key}"), vars).to_string();
    let d = a.detail.as_deref();
    let micro_tail = micro
        .label
        .as_ref()
        .map(|l| format!(" · {}", l.to_lowercase()))
        .unwrap_or_default();
    let with_code = |prefix: String| match d {
        Some(d) => format!("{prefix} {d}"),
        None => prefix,
    };
    match a.state.as_str() {
        "thinking" => match &micro.label {
            Some(l) => format!("{l}…"),
            None => k("statusThinking"),
        },
        "testing" => match d {
            Some(d) => format!("{} · {d}{micro_tail}", k("statusRunningTests")),
            None => format!("{}{micro_tail}", k("statusRunningTests")),
        },
        "running" => match d {
            Some(_) => format!("{}{micro_tail}", with_code(k("statusRunning"))),
            None => format!("{}{micro_tail}", k("statusRunningCommand")),
        },
        "editing" => with_code(k("statusEditing")),
        "reading" => with_code(k("statusReading")),
        "web" => match d {
            Some(host) => kv("statusOnWeb", &[("host", host)]),
            None => k("statusSearchingWeb"),
        },
        "delegating" => {
            let n = d.and_then(|d| d.parse::<u64>().ok()).filter(|n| *n > 0).unwrap_or(1);
            kv(
                if n == 1 {
                    "statusDelegatingOne"
                } else {
                    "statusDelegatingMany"
                },
                &[("count", &n.to_string())],
            )
        }
        "waiting" => kv(
            if micro.sleepy {
                "statusWaitingSleepy"
            } else {
                "statusWaiting"
            },
            &[("time", &hhmm(a.since_ms))],
        ),
        "asleep" => kv(
            if micro.long {
                "statusAsleepLong"
            } else {
                "statusIdleSince"
            },
            &[("time", &hhmm(a.since_ms))],
        ),
        "blocked" => match d {
            Some(detail) => kv("statusBlockedDetail", &[("detail", detail)]),
            None => k("statusBlocked"),
        },
        "finished" => {
            if d == Some("closed") {
                return k("statusClosed");
            }
            match reports {
                0 => k("statusFinished"),
                1 => kv("statusFinishedReportsOne", &[("count", "1")]),
                n => kv("statusFinishedReportsMany", &[("count", &n.to_string())]),
            }
        }
        "verified" => k("statusVerified"),
        "disconnected" => k("statusDisconnected"),
        _ => k("statusIdle"),
    }
}

/// States whose line already says since when: no timer.
pub fn timeless(state: &str) -> bool {
    matches!(state, "disconnected" | "waiting" | "asleep" | "normal")
}

/// The chip's icon for a state (a Lucide stand-in for `StatusIcon`).
pub fn icon(state: &str) -> &'static str {
    match state {
        "thinking" => "ellipsis",
        "running" => "square-terminal",
        "testing" => "flask-conical",
        "editing" => "pencil",
        "reading" => "book-open",
        "web" => "globe",
        "delegating" => "users",
        "waiting" => "hourglass",
        "asleep" => "moon",
        "blocked" => "octagon-alert",
        "finished" => "check",
        "verified" => "badge-check",
        "disconnected" => "wifi-off",
        _ => "circle",
    }
}

/// The chip's background token for a state.
pub fn chip_token(state: &str) -> &'static str {
    match state {
        "thinking" | "delegating" => "accent-soft",
        "testing" | "finished" | "verified" => "green-bg",
        "editing" | "running" | "waiting" | "blocked" => "amber-bg",
        "reading" | "web" => "robot-tile-2",
        "asleep" => "red-bg",
        _ => "chip",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_timer_reads_like_the_console() {
        assert_eq!(elapsed(8_400), "8 s");
        assert_eq!(elapsed(125_000), "2 min");
        assert_eq!(elapsed(3_600_000), "1 h");
        assert_eq!(elapsed(3_900_000), "1 h 5 min");
        assert!(timeless("waiting") && !timeless("running"));
    }
}
