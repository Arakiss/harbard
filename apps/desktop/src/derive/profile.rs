//! What the Profile and Status dialogs add to the shell's profile wording
//! (`lib/profile.ts`): status "Clear after", the context's limits, weekdays
//! and time zones.

use crate::ui::format::local_date;

const MIN: u64 = 60_000;

pub const CONTEXT_MAX: usize = 1500;
/// serve's `CONTEXT_MAX_QUOTED_BYTES`: the context as the brief quotes it.
const CONTEXT_MAX_QUOTED_BYTES: usize = 3200;

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

/// Characters as serve counts them (Unicode scalars), `\r\n` as one.
pub fn context_count(text: &str) -> usize {
    text.replace("\r\n", "\n").chars().count()
}

/// "> " and a newline per line, in UTF-8 bytes.
pub fn context_quoted_bytes(text: &str) -> usize {
    text.replace("\r\n", "\n")
        .split('\n')
        .map(|l| l.len() + 3)
        .sum()
}

/// Within both caps: serve accepts it, and the agents get it whole.
pub fn context_fits(text: &str) -> bool {
    context_count(text) <= CONTEXT_MAX
        && context_quoted_bytes(text.trim()) <= CONTEXT_MAX_QUOTED_BYTES
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
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let path = format!("{prefix}/{name}");
            match e.file_type() {
                Ok(ft) if ft.is_dir() => walk(&e.path(), &path, out),
                Ok(_) if name.chars().next().is_some_and(|c| c.is_ascii_uppercase()) => {
                    out.push(path)
                }
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
