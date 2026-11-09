//! Wall-clock helpers. Time is milliseconds since the Unix epoch throughout;
//! `ts` strings are RFC3339 UTC only for the stored, human-legible record.
//!
//! `HARBARD_NOW_MS` overrides "now" so tests and demos can inject a clock
//! (acceptance row A9). Ordering authority in the store is always file order,
//! never these timestamps (clock skew) — see the SPEC.

use std::time::{SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

/// Current time in milliseconds since the Unix epoch, honoring the
/// `HARBARD_NOW_MS` override when present.
pub fn now_ms() -> u64 {
    if let Ok(v) = std::env::var("HARBARD_NOW_MS")
        && let Ok(ms) = v.trim().parse::<u64>()
    {
        return ms;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Format epoch milliseconds as an RFC3339 UTC string.
pub fn rfc3339_utc(ms: u64) -> String {
    let nanos = i128::from(ms) * 1_000_000;
    OffsetDateTime::from_unix_timestamp_nanos(nanos)
        .ok()
        .and_then(|dt| dt.format(&Rfc3339).ok())
        .unwrap_or_else(|| "1970-01-01T00:00:00Z".to_string())
}

/// Parse an RFC3339 UTC instant (`2026-11-06T22:33:38.123Z`, fraction
/// optional) into epoch milliseconds. Offsets other than `Z` / `+00:00` are
/// rejected rather than guessed: harness transcripts write UTC, and a value
/// that is not UTC is a format change the caller should see, not absorb.
pub fn parse_rfc3339_ms(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' {
        return None;
    }
    if b[13] != b':' || b[16] != b':' {
        return None;
    }
    let num = |from: usize, to: usize| -> Option<i64> {
        s.get(from..to)?.bytes().try_fold(0i64, |acc, c| {
            c.is_ascii_digit().then(|| acc * 10 + i64::from(c - b'0'))
        })
    };
    let (year, month, day) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    let (hour, min, sec) = (num(11, 13)?, num(14, 16)?, num(17, 19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || hour > 23 || min > 59 || sec > 60 {
        return None;
    }
    let mut rest = &s[19..];
    let mut millis = 0i64;
    if let Some(frac) = rest.strip_prefix('.') {
        let digits = frac.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            return None;
        }
        let mut scale = 100;
        for c in frac[..digits.min(3)].bytes() {
            millis += i64::from(c - b'0') * scale;
            scale /= 10;
        }
        rest = &frac[digits..];
    }
    if rest != "Z" && rest != "+00:00" {
        return None;
    }
    // Days from the civil date (Howard Hinnant's algorithm), proleptic Gregorian.
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let secs = days * 86_400 + hour * 3600 + min * 60 + sec;
    u64::try_from(secs * 1000 + millis).ok()
}

/// Render a relative age like "3m ago" from a millisecond delta.
///
/// Deliberately coarse and deterministic: seconds under a minute, then
/// minutes, hours, days. Used by human `ls`/`brief` output.
pub fn relative(now_ms: u64, then_ms: u64) -> String {
    if now_ms <= then_ms {
        return "just now".to_string();
    }
    let secs = (now_ms - then_ms) / 1000;
    if secs < 5 {
        "just now".to_string()
    } else if secs < 60 {
        format!("{secs}s ago")
    } else if secs < 3600 {
        format!("{}m ago", secs / 60)
    } else if secs < 86_400 {
        format!("{}h ago", secs / 3600)
    } else {
        format!("{}d ago", secs / 86_400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc3339_is_utc() {
        assert_eq!(rfc3339_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339_utc(1000), "1970-01-01T00:00:01Z");
        assert_eq!(rfc3339_utc(86_400_000), "1970-01-02T00:00:00Z");
    }

    #[test]
    fn parses_rfc3339_round_trip_and_rejects_offsets() {
        for ms in [
            0u64,
            1_000,
            86_400_000,
            1_793_000_000_000,
            1_793_000_000_123,
        ] {
            let text = rfc3339_utc(ms);
            assert_eq!(parse_rfc3339_ms(&text), Some(ms), "{text}");
        }
        assert_eq!(
            parse_rfc3339_ms("2026-11-06T22:33:38.123Z"),
            Some(1_794_004_418_123)
        );
        assert_eq!(
            parse_rfc3339_ms("2026-11-06T22:33:38.5+00:00"),
            Some(1_794_004_418_500)
        );
        assert_eq!(parse_rfc3339_ms("2026-11-06T22:33:38+01:00"), None);
        assert_eq!(parse_rfc3339_ms("2026-13-06T22:33:38Z"), None);
        assert_eq!(parse_rfc3339_ms("yesterday"), None);
    }

    #[test]
    fn relative_buckets() {
        let now = 1_000_000_000;
        assert_eq!(relative(now, now), "just now");
        assert_eq!(relative(now, now - 30_000), "30s ago");
        assert_eq!(relative(now, now - 180_000), "3m ago");
        assert_eq!(relative(now, now - 7_200_000), "2h ago");
        assert_eq!(relative(now, now - 172_800_000), "2d ago");
    }
}
