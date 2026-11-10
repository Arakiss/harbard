//! History (`lib/history.ts`): how each item ended, how long it lasted, the
//! day groups and the page query. serve decides the facts; this words them.

use crate::i18n::{I18n, Locale, t};
use crate::model::Agent;
use crate::model::history::HistoryItem;
use crate::ui::format::{day_label, hhmm, local_date, local_day};
use gpui::{App, SharedString};

/// How many items a page asks for.
pub const PAGE_SIZE: usize = 50;

const MIN: u64 = 60_000;
const HOUR: u64 = 60 * MIN;
const DAY: u64 = 24 * HOUR;

/// The `data-ending` of a row: the ending, or "abierto" while it works.
pub fn ending_id(item: &HistoryItem) -> &str {
    item.ending.as_deref().unwrap_or("abierto")
}

pub fn ending_text(item: &HistoryItem, cx: &App) -> SharedString {
    match item.ending.as_deref() {
        Some("finished") => t(cx, "history.endingFinished"),
        Some("closed") => t(cx, "history.endingClosed"),
        Some("verified") => t(cx, "history.endingVerified"),
        Some("no_closure") => t(cx, "history.endingNone"),
        Some(other) => other.to_string().into(),
        None => t(cx, "history.endingOngoing"),
    }
}

/// "45 min", "2 h 5 min", "3 d 4 h"; `under` below a minute.
pub fn lasted(ms: u64, under: &str) -> String {
    let m = ms / MIN;
    if m < 1 {
        return under.to_string();
    }
    if m < 60 {
        return format!("{m} min");
    }
    if ms < DAY {
        let (h, rest) = (m / 60, m % 60);
        return if rest > 0 {
            format!("{h} h {rest} min")
        } else {
            format!("{h} h")
        };
    }
    let (d, h) = (ms / DAY, (ms % DAY) / HOUR);
    if h > 0 {
        format!("{d} d {h} h")
    } else {
        format!("{d} d")
    }
}

const SHORT_EN: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
const SHORT_ES: [&str; 12] = [
    "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic",
];
const LONG_EN: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const LONG_ES: [&str; 12] = [
    "enero",
    "febrero",
    "marzo",
    "abril",
    "mayo",
    "junio",
    "julio",
    "agosto",
    "septiembre",
    "octubre",
    "noviembre",
    "diciembre",
];

fn spanish(cx: &App) -> bool {
    cx.global::<I18n>().locale == Locale::Es
}

/// "5 Nov" / "5 nov".
pub fn short_date(ms: u64, cx: &App) -> String {
    let (_, mo, d, _) = local_date(ms);
    let names = if spanish(cx) { SHORT_ES } else { SHORT_EN };
    format!("{d} {}", names[(mo - 1) as usize])
}

/// "3 November" / "3 de noviembre" (`toLocaleDateString` day + long month).
pub fn long_day(ms: u64, cx: &App) -> String {
    let (_, mo, d, _) = local_date(ms);
    if spanish(cx) {
        format!("{d} de {}", LONG_ES[(mo - 1) as usize])
    } else {
        format!("{d} {}", LONG_EN[(mo - 1) as usize])
    }
}

/// `toLocaleDateString()`: "05/11/2026" (en-GB) or "5/11/2026" (es-ES).
pub fn numeric_date(ms: u64, cx: &App) -> String {
    let (y, mo, d, _) = local_date(ms);
    if spanish(cx) {
        format!("{d}/{mo}/{y}")
    } else {
        format!("{d:02}/{mo:02}/{y}")
    }
}

/// "08:30 – 10:00 · 1 h 30 min", with the dates when it crossed midnight.
pub fn span_text(item: &HistoryItem, cx: &App) -> String {
    let same = local_day(item.start_ms) == local_day(item.end_ms);
    let side = |ms: u64| {
        if same {
            hhmm(ms)
        } else {
            format!("{} {}", short_date(ms, cx), hhmm(ms))
        }
    };
    let under = t(cx, "history.underOneMinute");
    format!(
        "{} – {} · {}",
        side(item.start_ms),
        side(item.end_ms),
        lasted(item.duration_ms, &under)
    )
}

/// Items grouped by the local day each ended, newest first.
pub fn by_day(items: &[HistoryItem]) -> Vec<(i64, Vec<usize>)> {
    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| items[b].end_ms.cmp(&items[a].end_ms));
    let mut out: Vec<(i64, Vec<usize>)> = Vec::new();
    for i in order {
        let day = local_day(items[i].end_ms);
        match out.last_mut() {
            Some((d, list)) if *d == day => list.push(i),
            _ => out.push((day, vec![i])),
        }
    }
    out
}

/// The header of a day group.
pub fn day_title(end_ms: u64, now: u64, cx: &App) -> SharedString {
    day_label(end_ms, now, cx)
}

/// The query string of a page: only what was chosen.
pub fn query(
    before: Option<&str>,
    channel: &str,
    q: &str,
    limit: usize,
    ws: Option<&str>,
) -> String {
    use crate::api::enc;
    let mut parts = vec![format!("limit={limit}")];
    if let Some(b) = before.filter(|b| !b.is_empty()) {
        parts.push(format!("before={}", enc(b)));
    }
    if !channel.trim().is_empty() {
        parts.push(format!("channel={}", enc(channel.trim())));
    }
    if !q.trim().is_empty() {
        parts.push(format!("q={}", enc(q.trim())));
    }
    if let Some(ws) = ws.filter(|w| !w.is_empty()) {
        parts.push(format!("ws={}", enc(ws)));
    }
    format!("?{}", parts.join("&"))
}

/// The next page after the ones shown; an item seen twice stays where it was first.
pub fn merge_page(shown: &mut Vec<HistoryItem>, next: Vec<HistoryItem>) {
    let seen: std::collections::HashSet<String> =
        shown.iter().map(|i| i.agent.key.clone()).collect();
    shown.extend(next.into_iter().filter(|i| !seen.contains(&i.agent.key)));
}

/// Read-only the way Direct opens it: the live copy when the shell has one
/// (ended → read-only), else the archive, which is always read-only.
pub fn read_only(live: Option<&Agent>) -> bool {
    match live {
        Some(a) => a.delivery == "ended",
        None => true,
    }
}

/// The line under a name: serve's role line, else "role · team".
/// TODO: role words are not localised until the shared `roleVars` lands.
pub fn role_line(a: &Agent, cx: &App) -> Option<SharedString> {
    if let Some(line) = a.role_line.as_deref() {
        return Some(crate::i18n::serve_text(
            cx,
            a.role_line_code.as_deref(),
            a.role_line_params.as_ref(),
            line,
        ));
    }
    let parts: Vec<&str> = [a.role.as_deref(), a.team.as_deref()]
        .into_iter()
        .flatten()
        .filter(|s| !s.is_empty())
        .collect();
    (!parts.is_empty()).then(|| parts.join(" · ").into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lasted_follows_the_console() {
        assert_eq!(lasted(30_000, "under 1 min"), "under 1 min");
        assert_eq!(lasted(45 * MIN, "u"), "45 min");
        assert_eq!(lasted(2 * HOUR + 5 * MIN, "u"), "2 h 5 min");
        assert_eq!(lasted(3 * HOUR, "u"), "3 h");
        assert_eq!(lasted(3 * DAY + 4 * HOUR + 10 * MIN, "u"), "3 d 4 h");
        assert_eq!(lasted(2 * DAY, "u"), "2 d");
    }

    fn item(key: &str, end: u64) -> HistoryItem {
        HistoryItem {
            agent: Agent {
                key: key.into(),
                ..Default::default()
            },
            end_ms: end,
            ..Default::default()
        }
    }

    #[test]
    fn by_day_is_newest_first() {
        let base = 1_794_300_000_000;
        let items = [
            item("a", base),
            item("b", base + 3 * DAY),
            item("c", base + 3 * DAY + 1000),
        ];
        let groups = by_day(&items);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].1, vec![2, 1]);
        assert_eq!(groups[1].1, vec![0]);
    }

    #[test]
    fn merge_keeps_the_first_copy() {
        let mut shown = vec![item("a", 1), item("b", 2)];
        merge_page(&mut shown, vec![item("b", 9), item("c", 3)]);
        assert_eq!(
            shown
                .iter()
                .map(|i| (i.agent.key.as_str(), i.end_ms))
                .collect::<Vec<_>>(),
            vec![("a", 1), ("b", 2), ("c", 3)]
        );
    }

    #[test]
    fn query_has_only_what_was_chosen() {
        assert_eq!(query(None, "", "  ", 50, None), "?limit=50");
        assert_eq!(
            query(Some("cur"), " harbard ", "two words", 50, Some("ws1")),
            "?limit=50&before=cur&channel=harbard&q=two%20words&ws=ws1"
        );
    }

    #[test]
    fn read_only_like_direct() {
        let live = Agent {
            delivery: "ready".into(),
            ..Default::default()
        };
        let ended = Agent {
            delivery: "ended".into(),
            ..Default::default()
        };
        assert!(!read_only(Some(&live)));
        assert!(read_only(Some(&ended)));
        assert!(read_only(None));
    }
}
