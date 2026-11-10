//! The operator's local date, for the seasons: in the profile's time zone
//! when it names one, otherwise in the machine's.
//!
//! An IANA zone is read from the system's compiled zone files (TZif, RFC
//! 8536) under `/usr/share/zoneinfo`: the last transition at or before the
//! moment gives the offset. Past the last transition a file lists, the
//! footer's fixed offset is used when it has no daylight rule, otherwise the
//! last transition's offset. The fat files macOS ships list transitions to
//! 2037; a season starts at midnight, so the worst case of that fallback is
//! a season that turns on an hour early or late.

use std::path::Path;
use time::{Date, OffsetDateTime, UtcOffset};

const ZONEINFO: &str = "/usr/share/zoneinfo";

/// A name `profile.json` accepts (`Area/Location`, letters, digits, `_`,
/// `-`, `+`), and never a path that climbs out of the zone folder.
fn plausible(zone: &str) -> bool {
    !zone.is_empty()
        && zone.len() <= 64
        && !zone.starts_with('/')
        && zone
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && zone
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '/' | '_' | '-' | '+'))
}

/// The offset east of UTC, in seconds, of `zone` at `at_secs`; none when
/// the zone is unknown or its file unreadable.
pub fn zone_offset_secs(zone: &str, at_secs: i64) -> Option<i64> {
    if !plausible(zone) {
        return None;
    }
    let bytes = std::fs::read(Path::new(ZONEINFO).join(zone)).ok()?;
    tzif_offset(&bytes, at_secs)
}

/// The machine's offset east of UTC at `at_secs`. `time`'s own local offset
/// refuses in a process with more than one thread, and serve has many, so
/// the C library answers instead.
#[cfg(unix)]
pub fn machine_offset_secs(at_secs: i64) -> i64 {
    let secs = at_secs as libc::time_t;
    // SAFETY: `tm` is a valid out-parameter for `localtime_r`, which is
    // thread-safe; it is only read after the call reports success.
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        if libc::localtime_r(&secs, &mut tm).is_null() {
            0
        } else {
            tm.tm_gmtoff as i64
        }
    }
}

#[cfg(not(unix))]
pub fn machine_offset_secs(_at_secs: i64) -> i64 {
    0
}

/// The operator's calendar date at `now_ms`: in `zone` when it is known,
/// otherwise the machine's.
pub fn local_date(zone: Option<&str>, now_ms: u64) -> Date {
    let secs = (now_ms / 1000) as i64;
    let offset = zone
        .and_then(|z| zone_offset_secs(z, secs))
        .unwrap_or_else(|| machine_offset_secs(secs));
    let offset = UtcOffset::from_whole_seconds(offset as i32).unwrap_or(UtcOffset::UTC);
    OffsetDateTime::from_unix_timestamp(secs)
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
        .to_offset(offset)
        .date()
}

fn be32(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn be64(b: &[u8], at: usize) -> Option<i64> {
    Some(i64::from_be_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

struct Header {
    isut: usize,
    isstd: usize,
    leap: usize,
    times: usize,
    types: usize,
    chars: usize,
}

fn header(b: &[u8], at: usize) -> Option<Header> {
    if b.get(at..at + 4)? != b"TZif" {
        return None;
    }
    let n = |i: usize| be32(b, at + 20 + 4 * i).map(|v| v as usize);
    Some(Header {
        isut: n(0)?,
        isstd: n(1)?,
        leap: n(2)?,
        times: n(3)?,
        types: n(4)?,
        chars: n(5)?,
    })
}

impl Header {
    fn data_len(&self, time_size: usize) -> usize {
        self.times * time_size
            + self.times
            + self.types * 6
            + self.chars
            + self.leap * (time_size + 4)
            + self.isstd
            + self.isut
    }
}

/// The offset a TZif file gives at `at`.
fn tzif_offset(b: &[u8], at: i64) -> Option<i64> {
    let v1 = header(b, 0)?;
    // Version 2 and later repeat the data with 64-bit times, then a footer.
    let (h, data, size) = if b.get(4).copied().unwrap_or(0) >= b'2' {
        let second = 44 + v1.data_len(4);
        (header(b, second)?, second + 44, 8)
    } else {
        (v1, 44, 4)
    };
    let time = |i: usize| -> Option<i64> {
        if size == 8 {
            be64(b, data + 8 * i)
        } else {
            be32(b, data + 4 * i).map(|v| i64::from(v as i32))
        }
    };
    let index_at = data + h.times * size;
    let types_at = index_at + h.times;
    let utoff = |t: usize| -> Option<i64> {
        if t >= h.types {
            return None;
        }
        be32(b, types_at + 6 * t).map(|v| i64::from(v as i32))
    };
    if h.types == 0 {
        return None;
    }
    let mut last = None;
    for i in 0..h.times {
        if time(i)? <= at {
            last = Some(i);
        } else {
            break;
        }
    }
    match last {
        None => utoff(0),
        Some(i) if i + 1 == h.times && size == 8 => {
            let footer_at = data + h.data_len(8);
            fixed_footer(b.get(footer_at..)?).or_else(|| utoff(usize::from(*b.get(index_at + i)?)))
        }
        Some(i) => utoff(usize::from(*b.get(index_at + i)?)),
    }
}

/// The offset of a footer without a daylight rule (`<+03>-3`, `JST-9`).
/// POSIX writes it west of UTC: `-3` is three hours east.
fn fixed_footer(b: &[u8]) -> Option<i64> {
    let text = std::str::from_utf8(b).ok()?.trim_matches('\n');
    if text.is_empty() || text.contains(',') {
        return None;
    }
    let rest = if let Some(r) = text.strip_prefix('<') {
        &r[r.find('>')? + 1..]
    } else {
        text.trim_start_matches(|c: char| c.is_ascii_alphabetic())
    };
    // A daylight name after the offset means a rule is implied.
    let end = rest
        .find(|c: char| c.is_ascii_alphabetic() || c == '<')
        .unwrap_or(rest.len());
    if end != rest.len() {
        return None;
    }
    let (sign, digits) = match rest.strip_prefix('-') {
        Some(d) => (1, d),
        None => (-1, rest.strip_prefix('+').unwrap_or(rest)),
    };
    let mut parts = digits.split(':').map(|p| p.parse::<i64>().ok());
    let hours = parts.next()??;
    let minutes = parts.next().flatten().unwrap_or(0);
    let seconds = parts.next().flatten().unwrap_or(0);
    Some(sign * (hours * 3600 + minutes * 60 + seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-12-01 00:30 in Madrid is 2026-11-30 23:30 UTC.
    const MADRID_DEC_1_0030: u64 = 1_796_081_400_000;

    #[test]
    fn a_zone_from_the_system_files_moves_the_date() {
        if !Path::new(ZONEINFO).join("Europe/Madrid").exists() {
            return;
        }
        assert_eq!(zone_offset_secs("Europe/Madrid", 1_796_081_400), Some(3600));
        let madrid = local_date(Some("Europe/Madrid"), MADRID_DEC_1_0030);
        assert_eq!((madrid.month() as u8, madrid.day()), (12, 1));
        let utc = local_date(Some("Etc/UTC"), MADRID_DEC_1_0030);
        assert_eq!((utc.month() as u8, utc.day()), (11, 30));
        // Summer time in July.
        assert_eq!(zone_offset_secs("Europe/Madrid", 1_783_000_000), Some(7200));
    }

    #[test]
    fn a_path_is_never_a_zone() {
        for bad in ["../../etc/passwd", "/etc/passwd", "Europe/../..", "", "a b"] {
            assert_eq!(zone_offset_secs(bad, 0), None, "{bad}");
        }
    }

    #[test]
    fn fixed_footers_parse_and_rules_do_not() {
        assert_eq!(fixed_footer(b"\n<+03>-3\n"), Some(3 * 3600));
        assert_eq!(fixed_footer(b"\nJST-9\n"), Some(9 * 3600));
        assert_eq!(fixed_footer(b"\nUTC0\n"), Some(0));
        assert_eq!(fixed_footer(b"\n<-0330>3:30\n"), Some(-(3 * 3600 + 1800)));
        assert_eq!(fixed_footer(b"\nCET-1CEST,M3.5.0,M10.5.0/3\n"), None);
        assert_eq!(fixed_footer(b"\nEST5EDT\n"), None);
    }
}
