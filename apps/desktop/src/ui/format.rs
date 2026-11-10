//! The console's time and name wording (`lib/workspace.ts`): `hace`,
//! `duration`, `hhmm`, `atDate`, `dayLabel`, `monogram`. Times are local.

use crate::i18n::{t, tv};
use gpui::{App, SharedString};

const MIN: u64 = 60_000;
const HOUR: u64 = 60 * MIN;
const DAY: u64 = 24 * HOUR;

/// The local UTC offset in minutes at `ms`.
fn offset_min(ms: u64) -> i64 {
    let secs = (ms / 1000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: localtime_r writes only into `tm`.
    unsafe { libc::localtime_r(&secs, &mut tm) };
    tm.tm_gmtoff as i64 / 60
}

/// Days since the epoch in local time.
pub fn local_day(ms: u64) -> i64 {
    let local = ms as i64 + offset_min(ms) * MIN as i64;
    local.div_euclid(DAY as i64)
}

/// (hour, minute) in local time.
pub fn local_hm(ms: u64) -> (u32, u32) {
    let local = ms as i64 + offset_min(ms) * MIN as i64;
    let in_day = local.rem_euclid(DAY as i64) as u64;
    ((in_day / HOUR) as u32, ((in_day % HOUR) / MIN) as u32)
}

/// (year, month 1-12, day, weekday 0=Sunday) in local time.
pub fn local_date(ms: u64) -> (i32, u32, u32, u32) {
    let secs = (ms / 1000) as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: localtime_r writes only into `tm`.
    unsafe { libc::localtime_r(&secs, &mut tm) };
    (tm.tm_year + 1900, tm.tm_mon as u32 + 1, tm.tm_mday as u32, tm.tm_wday as u32)
}

/// "just now", "5 min ago", "3 h ago", "2 d ago".
pub fn hace(ms: u64, now: u64, cx: &App) -> SharedString {
    let delta = now.saturating_sub(ms);
    if delta < 45_000 {
        return t(cx, "workspace.justNow");
    }
    let n = if delta < HOUR {
        return tv(cx, "workspace.agoMinutes", &[("n", &((delta as f64 / MIN as f64).round().max(1.0) as u64).to_string())]);
    } else if delta < DAY {
        ("workspace.agoHours", (delta as f64 / HOUR as f64).round() as u64)
    } else {
        ("workspace.agoDays", (delta as f64 / DAY as f64).round() as u64)
    };
    tv(cx, n.0, &[("n", &n.1.to_string())])
}

/// "12 min", "5 h", "3 d".
pub fn duration(ms: u64) -> String {
    let m = (ms as f64 / MIN as f64).round() as u64;
    if m < 90 {
        return format!("{m} min");
    }
    let h = (ms as f64 / HOUR as f64).round() as u64;
    if h < 48 {
        return format!("{h} h");
    }
    format!("{} d", (ms as f64 / DAY as f64).round() as u64)
}

/// "09:41".
pub fn hhmm(ms: u64) -> String {
    let (h, m) = local_hm(ms);
    format!("{h:02}:{m:02}")
}

const MONTHS_EN: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const MONTHS_ES: [&str; 12] = ["ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic"];
const MONTHS_LONG_EN: [&str; 12] = [
    "January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November",
    "December",
];
const MONTHS_LONG_ES: [&str; 12] = [
    "enero", "febrero", "marzo", "abril", "mayo", "junio", "julio", "agosto", "septiembre", "octubre", "noviembre",
    "diciembre",
];
const DAYS_EN: [&str; 7] = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const DAYS_ES: [&str; 7] = ["domingo", "lunes", "martes", "miércoles", "jueves", "viernes", "sábado"];

fn spanish(cx: &App) -> bool {
    cx.global::<crate::i18n::I18n>().locale == crate::i18n::Locale::Es
}

/// "today at 09:41", "yesterday at 18:02", "3 Nov at 10:15".
pub fn at_date(ms: u64, now: u64, cx: &App) -> SharedString {
    let day = local_day(ms);
    let time = hhmm(ms);
    if day == local_day(now) {
        return tv(cx, "workspace.todayAt", &[("time", &time)]);
    }
    if day == local_day(now.saturating_sub(DAY)) {
        return tv(cx, "workspace.yesterdayAt", &[("time", &time)]);
    }
    let (_, mo, d, _) = local_date(ms);
    let date = if spanish(cx) {
        format!("{d} {}", MONTHS_ES[(mo - 1) as usize])
    } else {
        format!("{d} {}", MONTHS_EN[(mo - 1) as usize])
    };
    tv(cx, "workspace.dateAt", &[("date", &date), ("time", &time)])
}

/// "Today", "Yesterday", "Monday 3 November" (the console formats English as en-GB).
pub fn day_label(ms: u64, now: u64, cx: &App) -> SharedString {
    let day = local_day(ms);
    if day == local_day(now) {
        return t(cx, "workspace.dayToday");
    }
    if day == local_day(now.saturating_sub(DAY)) {
        return t(cx, "workspace.dayYesterday");
    }
    let (_, mo, d, wd) = local_date(ms);
    if spanish(cx) {
        format!("{}, {d} de {}", DAYS_ES[wd as usize], MONTHS_LONG_ES[(mo - 1) as usize]).into()
    } else {
        format!("{} {d} {}", DAYS_EN[wd as usize], MONTHS_LONG_EN[(mo - 1) as usize]).into()
    }
}

/// Two letters for an avatar: initials of two words, else the first letter
/// and the first consonant after it.
pub fn monogram(name: &str) -> String {
    let words: Vec<&str> = name
        .split(['-', '_', ' ', '.', '·'])
        .filter(|w| w.chars().any(|c| c.is_alphabetic()))
        .collect();
    if words.len() >= 2 {
        let a = words[0].chars().next().unwrap_or(' ');
        let b = words[1].chars().next().unwrap_or(' ');
        return format!("{a}{b}").to_uppercase();
    }
    let word = words.first().copied().unwrap_or(name);
    let mut chars = word.chars();
    let first = chars.next().unwrap_or(' ');
    let rest: String = chars.collect();
    let consonant = rest
        .chars()
        .find(|c| "bcdfgjklmnpqrstvwxz".contains(c.to_ascii_lowercase()))
        .or_else(|| rest.chars().next())
        .map(String::from)
        .unwrap_or_default();
    format!("{first}{consonant}").to_uppercase()
}

/// `/Users/x/...` → `~/...`.
pub fn short_path(path: &str) -> String {
    if let Ok(home) = std::env::var("HOME")
        && let Some(rest) = path.strip_prefix(&home)
    {
        return format!("~{rest}");
    }
    path.to_string()
}

/// "B", "kB", "MB" like the console's `size`.
pub fn size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{} kB", (bytes as f64 / 1024.0).round() as u64)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monogram_follows_the_console() {
        assert_eq!(monogram("Adrian"), "AD");
        assert_eq!(monogram("Claude lead"), "CL");
        assert_eq!(monogram("Barry"), "BR");
    }

    #[test]
    fn duration_switches_units_like_the_console() {
        assert_eq!(duration(5 * MIN), "5 min");
        assert_eq!(duration(3 * HOUR), "3 h");
        assert_eq!(duration(3 * DAY), "3 d");
    }
}
