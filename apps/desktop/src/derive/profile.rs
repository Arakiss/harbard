//! The operator's profile, status line and "Do not disturb" (`lib/profile.ts`):
//! the menu's choices turned into moments in the operator's own clock, and
//! those moments worded.

use crate::api::ApiError;
use crate::i18n::{t, tv};
use crate::model::prefs_profile::{ProfileHours, ProfileStatus, ProfileView};
use crate::ui::format::local_date;
use gpui::{App, SharedString};

const MIN: u64 = 60_000;

pub const CONTEXT_MAX: usize = 1500;
/// serve's `CONTEXT_MAX_QUOTED_BYTES`: the context as the brief quotes it.
const CONTEXT_MAX_QUOTED_BYTES: usize = 3200;

/// One coordination state, with the menu's copy keys and its tone.
#[derive(Clone, Copy, Debug)]
pub struct StateInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub hint: &'static str,
    /// green, accent, red, grey, blue or amber.
    pub tone: &'static str,
    /// The Lucide icon the console drew (`state-icon.tsx`).
    pub icon: &'static str,
}

pub const STATES: [StateInfo; 6] = [
    StateInfo {
        id: "available",
        label: "profile.stateAvailable",
        hint: "profile.stateAvailableHint",
        tone: "green",
        icon: "circle",
    },
    StateInfo {
        id: "focused",
        label: "profile.stateFocused",
        hint: "profile.stateFocusedHint",
        tone: "accent",
        icon: "focus",
    },
    StateInfo {
        id: "swamped",
        label: "profile.stateSwamped",
        hint: "profile.stateSwampedHint",
        tone: "red",
        icon: "flame",
    },
    StateInfo {
        id: "away",
        label: "profile.stateAway",
        hint: "profile.stateAwayHint",
        tone: "grey",
        icon: "moon",
    },
    StateInfo {
        id: "reviewing",
        label: "profile.stateReviewing",
        hint: "profile.stateReviewingHint",
        tone: "blue",
        icon: "scan-search",
    },
    StateInfo {
        id: "done",
        label: "profile.stateDoneForDay",
        hint: "profile.stateDoneForDayHint",
        tone: "amber",
        icon: "flag",
    },
];

/// The state in force at `now` (Available when none or ended).
pub fn state_of(view: Option<&ProfileView>, now: u64) -> &'static StateInfo {
    let ended = view.and_then(|v| v.state_until_ms).is_some_and(|until| now >= until);
    let id = match view {
        Some(v) if !ended => v.state.as_str(),
        _ => "available",
    };
    STATES.iter().find(|s| s.id == id).unwrap_or(&STATES[0])
}

/// The status in force at `now`.
pub fn effective_status(view: Option<&ProfileView>, now: u64) -> Option<&ProfileStatus> {
    let s = view?.status.as_ref()?;
    match s.clear_at_ms {
        Some(at) if now >= at => None,
        _ => Some(s),
    }
}

pub fn is_quiet(view: Option<&ProfileView>, now: u64) -> bool {
    match view {
        Some(v) if v.quiet => v.quiet_until_ms.is_none_or(|u| now < u),
        _ => false,
    }
}

/// A local wall-clock moment, `add_days` after `now`'s date, at `h:m`.
pub fn local_at(now: u64, add_days: i32, h: i32, m: i32) -> u64 {
    let (y, mo, d, _) = local_date(now);
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    tm.tm_year = y - 1900;
    tm.tm_mon = mo as i32 - 1;
    tm.tm_mday = d as i32 + add_days;
    tm.tm_hour = h;
    tm.tm_min = m;
    tm.tm_isdst = -1;
    // SAFETY: mktime reads and normalises `tm` only.
    let secs = unsafe { libc::mktime(&mut tm) };
    (secs.max(0) as u64) * 1000
}

/// Status "Clear after" choices: `nunca`, `30m`, `1h`, `4h`, `today`.
pub const CLEAR_AFTER: [(&str, &str); 5] = [
    ("nunca", "profile.clearNever"),
    ("30m", "profile.duration30Minutes"),
    ("1h", "profile.duration1Hour"),
    ("4h", "profile.duration4Hours"),
    ("today", "profile.clearToday"),
];

/// When a status chosen now clears; `None` never. "today" ends at midnight.
pub fn clear_at(choice: &str, now: u64) -> Option<u64> {
    match choice {
        "30m" => Some(now + 30 * MIN),
        "1h" => Some(now + 60 * MIN),
        "4h" => Some(now + 240 * MIN),
        "today" => Some(local_at(now, 1, 0, 0)),
        _ => None,
    }
}

/// "Do not disturb" default durations.
pub const DND_CHOICES: [(&str, &str); 3] = [
    ("30m", "profile.duration30Minutes"),
    ("1h", "profile.duration1Hour"),
    ("tomorrow", "profile.dndUntilTomorrow"),
];

/// When "Do not disturb" chosen now ends. "tomorrow" is tomorrow at 9:00.
pub fn dnd_until(choice: &str, now: u64) -> u64 {
    match choice {
        "30m" => now + 30 * MIN,
        "1h" => now + 60 * MIN,
        _ => local_at(now, 1, 9, 0),
    }
}

/// Done for the day ends at the next start of the working hours; without
/// hours, tomorrow at 9:00.
pub fn done_until(hours: Option<&ProfileHours>, now: u64) -> u64 {
    let Some(hours) = hours.filter(|h| !h.days.is_empty()) else {
        return local_at(now, 1, 9, 0);
    };
    let mut it = hours.start.split(':').map(|p| p.parse::<i32>().unwrap_or(0));
    let (h, m) = (it.next().unwrap_or(9), it.next().unwrap_or(0));
    for d in 0..=7 {
        let at = local_at(now, d, h, m);
        let (_, _, _, wd) = local_date(at);
        let iso = ((wd + 6) % 7) + 1;
        if at > now && hours.days.contains(&(iso as u8)) {
            return at;
        }
    }
    local_at(now, 1, 9, 0)
}

/// Characters as serve counts them (Unicode scalars), `\r\n` as one.
pub fn context_count(text: &str) -> usize {
    text.replace("\r\n", "\n").chars().count()
}

/// "> " and a newline per line, in UTF-8 bytes.
pub fn context_quoted_bytes(text: &str) -> usize {
    text.replace("\r\n", "\n").split('\n').map(|l| l.len() + 3).sum()
}

/// Within both caps: serve accepts it, and the agents get it whole.
pub fn context_fits(text: &str) -> bool {
    context_count(text) <= CONTEXT_MAX && context_quoted_bytes(text.trim()) <= CONTEXT_MAX_QUOTED_BYTES
}

/// `1,500` / `1.500` as `Intl.NumberFormat` writes it.
pub fn thousands(n: usize, spanish: bool) -> String {
    let s = n.to_string();
    // es-ES groups only from five digits.
    if spanish && s.len() <= 4 {
        return s;
    }
    let sep = if spanish { '.' } else { ',' };
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(sep);
        }
        out.push(c);
    }
    out
}

/// (ISO day, short key, long key).
pub const WEEKDAYS: [(u8, &str, &str); 7] = [
    (1, "profile.mondayShort", "profile.monday"),
    (2, "profile.tuesdayShort", "profile.tuesday"),
    (3, "profile.wednesdayShort", "profile.wednesday"),
    (4, "profile.thursdayShort", "profile.thursday"),
    (5, "profile.fridayShort", "profile.friday"),
    (6, "profile.saturdayShort", "profile.saturday"),
    (7, "profile.sundayShort", "profile.sunday"),
];

/// A weekday as libc numbers it, Sunday = 0.
fn weekday_name(wd: u32, cx: &App) -> SharedString {
    let iso = if wd == 0 { 7 } else { wd as u8 };
    WEEKDAYS
        .iter()
        .find(|w| w.0 == iso)
        .map(|w| t(cx, w.2))
        .unwrap_or_default()
}

/// `H:MM`, no leading zero on the hour.
fn hm(ms: u64) -> String {
    let (h, m) = crate::ui::format::local_hm(ms);
    format!("{h}:{m:02}")
}

enum DayOf {
    Today,
    Tomorrow,
    Dated,
}

fn day_of(until: u64, now: u64) -> DayOf {
    let same = |a: u64, b: u64| {
        let (y1, m1, d1, _) = local_date(a);
        let (y2, m2, d2, _) = local_date(b);
        (y1, m1, d1) == (y2, m2, d2)
    };
    if same(until, now) {
        DayOf::Today
    } else if same(until, local_at(now, 1, 12, 0)) {
        DayOf::Tomorrow
    } else {
        DayOf::Dated
    }
}

/// "until 18:30", "until tomorrow at 9:00", "until Thursday 12 at 9:00".
pub fn quiet_label(until: u64, now: u64, cx: &App) -> SharedString {
    let time = hm(until);
    match day_of(until, now) {
        DayOf::Today => tv(cx, "profile.untilTime", &[("time", &time)]),
        DayOf::Tomorrow => tv(cx, "profile.untilTomorrow", &[("time", &time)]),
        DayOf::Dated => {
            let (_, _, d, wd) = local_date(until);
            tv(
                cx,
                "profile.untilDated",
                &[
                    ("weekday", &weekday_name(wd, cx)),
                    ("day", &d.to_string()),
                    ("time", &time),
                ],
            )
        }
    }
}

/// "Clears at 17:30", or `None` when it stays.
pub fn status_expiry(clear: Option<u64>, now: u64, cx: &App) -> Option<SharedString> {
    let until = clear?;
    let time = hm(until);
    Some(match day_of(until, now) {
        DayOf::Today => tv(cx, "profile.clearsAt", &[("time", &time)]),
        DayOf::Tomorrow => tv(cx, "profile.clearsTomorrow", &[("time", &time)]),
        DayOf::Dated => {
            let (_, _, d, wd) = local_date(until);
            tv(
                cx,
                "profile.clearsDated",
                &[
                    ("weekday", &weekday_name(wd, cx)),
                    ("day", &d.to_string()),
                    ("time", &time),
                ],
            )
        }
    })
}

/// Minutes east of UTC now, for serve to say times in the operator's clock.
pub fn utc_offset_min(now: u64) -> i64 {
    let secs = (now / 1000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: localtime_r writes only into `tm`.
    unsafe { libc::localtime_r(&secs, &mut tm) };
    tm.tm_gmtoff as i64 / 60
}

/// serve's refusals in the operator's language; the rest as serve said it.
pub fn refusal_text(e: &ApiError, cx: &App) -> SharedString {
    match e.code.as_deref() {
        Some("too_large") => t(cx, "profile.photoTooLarge"),
        Some("unsupported_type") => t(cx, "profile.photoWrongType"),
        Some("symlink") => t(cx, "profile.profileFolderSymlink"),
        Some("too_long") => t(cx, "profile.tooLong"),
        _ if e.unreachable() => t(cx, "profile.couldNotSave"),
        _ => e.message.clone().into(),
    }
}

/// This Mac's IANA zone: `TZ`, else where `/etc/localtime` points.
pub fn this_mac_zone() -> String {
    if let Ok(tz) = std::env::var("TZ")
        && tz.contains('/')
    {
        return tz.trim_start_matches(':').to_string();
    }
    std::fs::read_link("/etc/localtime")
        .ok()
        .and_then(|p| {
            let s = p.to_string_lossy().to_string();
            s.split("zoneinfo/").nth(1).map(str::to_string)
        })
        .unwrap_or_else(|| "UTC".into())
}

/// Every IANA zone this Mac knows, the operator's first (`Intl.supportedValuesOf`).
pub fn time_zones(current: &str) -> Vec<String> {
    const AREAS: [&str; 10] = [
        "Africa",
        "America",
        "Antarctica",
        "Arctic",
        "Asia",
        "Atlantic",
        "Australia",
        "Europe",
        "Indian",
        "Pacific",
    ];
    let root = std::path::Path::new("/usr/share/zoneinfo");
    let mut all = Vec::new();
    fn walk(dir: &std::path::Path, prefix: &str, out: &mut Vec<String>) {
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let path = format!("{prefix}/{name}");
            match e.file_type() {
                Ok(ft) if ft.is_dir() => walk(&e.path(), &path, out),
                Ok(_) if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) => out.push(path),
                _ => {}
            }
        }
    }
    for area in AREAS {
        walk(&root.join(area), area, &mut all);
    }
    all.push("UTC".into());
    all.sort();
    all.dedup();
    let mut out = vec![current.to_string()];
    out.extend(all.into_iter().filter(|z| z != current));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_counts_like_serve() {
        assert_eq!(context_count("a\r\nb"), 3);
        assert_eq!(context_count("ñandú"), 5);
        assert!(context_fits("hello"));
        assert!(!context_fits(&"x".repeat(1501)));
        // Many short lines: under the character cap, over the quoted bytes.
        assert!(!context_fits(&"a\n".repeat(1000)));
    }

    #[test]
    fn clear_after_choices() {
        assert_eq!(clear_at("nunca", 1_000), None);
        assert_eq!(clear_at("30m", 0), Some(30 * MIN));
        let now = 1_762_800_000_000; // a November afternoon
        let midnight = clear_at("today", now).unwrap();
        assert!(midnight > now && midnight - now <= 24 * 60 * MIN);
        assert_eq!(crate::ui::format::local_hm(midnight), (0, 0));
    }

    #[test]
    fn done_for_the_day_waits_for_the_next_working_start() {
        let now = 1_762_800_000_000;
        let hours = ProfileHours::default();
        let at = done_until(Some(&hours), now);
        assert!(at > now);
        assert_eq!(crate::ui::format::local_hm(at), (9, 0));
        let (_, _, _, wd) = local_date(at);
        assert!((1..=5).contains(&wd));
    }

    #[test]
    fn thousands_like_intl() {
        assert_eq!(thousands(1500, false), "1,500");
        assert_eq!(thousands(1500, true), "1500");
        assert_eq!(thousands(12, false), "12");
        assert_eq!(thousands(12_345, true), "12.345");
    }

    #[test]
    fn zones_put_this_mac_first() {
        let z = time_zones("Europe/Madrid");
        assert_eq!(z[0], "Europe/Madrid");
        assert_eq!(z.iter().filter(|x| *x == "Europe/Madrid").count(), 1);
    }
}
