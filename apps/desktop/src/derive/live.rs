//! One agent, step by step: the live view ("Watch live") as serve folds it
//! (`lib/live-agent.ts`): merging deltas, the timeline's order, its words,
//! the poller's pace, and the prose blocks (`live-pane.tsx`).

use crate::i18n::t;
use crate::model::inbox::{LiveDelta, LiveStep, Prose};
use crate::model::Progress;
use gpui::{App, SharedString};
use std::collections::BTreeMap;
use std::time::Duration;

/// The open view asks serve this often while visible.
pub const LIVE_POLL: Duration = Duration::from_millis(1_000);
/// The longest wait between two asks after failures.
pub const LIVE_BACKOFF_MAX: Duration = Duration::from_millis(30_000);
const STEPS_KEPT: usize = 400;
const PROSE_KEPT: usize = 150;

/// What the view holds for one open agent.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LiveState {
    pub key: String,
    pub rev: u64,
    /// serve's clock at the newest answer.
    pub server_now: u64,
    pub first_ms: Option<u64>,
    pub last_ms: Option<u64>,
    pub steps: Vec<LiveStep>,
    pub prose: Vec<Prose>,
    pub progress: Progress,
}

fn by_n<T: Clone>(old: &[T], fresh: &[T], n: impl Fn(&T) -> u64, cap: usize) -> Vec<T> {
    if fresh.is_empty() {
        return old.to_vec();
    }
    let mut map: BTreeMap<u64, T> = old.iter().map(|x| (n(x), x.clone())).collect();
    for x in fresh {
        map.insert(n(x), x.clone());
    }
    let out: Vec<T> = map.into_values().collect();
    if out.len() > cap { out[out.len() - cap..].to_vec() } else { out }
}

/// Fold one answer into what the view holds. A reset or a new key starts over.
pub fn merge_live(prev: Option<&LiveState>, d: &LiveDelta) -> LiveState {
    let base = prev.filter(|p| !d.reset && p.key == d.key);
    LiveState {
        key: d.key.clone(),
        rev: d.rev,
        server_now: d.now_ms,
        first_ms: d.first_ms,
        last_ms: d.last_ms,
        steps: by_n(base.map_or(&[][..], |b| &b.steps), &d.steps, |s| s.n, STEPS_KEPT),
        prose: by_n(base.map_or(&[][..], |b| &b.prose), &d.prose, |p| p.n, PROSE_KEPT),
        progress: d.progress.clone(),
    }
}

/// One row of the timeline: a tool call or a paragraph group.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LiveItem<'a> {
    Step(&'a LiveStep),
    Prose(&'a Prose),
}

/// Steps by start, prose by time; prose first when both carry the same instant.
pub fn live_items(s: &LiveState) -> Vec<LiveItem<'_>> {
    let mut items: Vec<(u64, u8, LiveItem<'_>)> = s
        .steps
        .iter()
        .map(|st| (st.step.started_ms, 1, LiveItem::Step(st)))
        .chain(s.prose.iter().map(|p| (p.ts_ms, 0, LiveItem::Prose(p))))
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    items.into_iter().map(|(_, _, i)| i).collect()
}

/// "Finished" once its turn ended with nothing left to do, else "Live".
pub fn live_heading(state: &str, presence: &str) -> bool {
    !(state == "finished" || state == "verified" || presence == "finished")
}

/// "running", "ok", "error", "no response".
pub fn status_text(status: &str, cx: &App) -> SharedString {
    match status {
        "running" => t(cx, "agents.statusRunning"),
        "ok" => t(cx, "agents.statusOk"),
        "error" => t(cx, "agents.statusError"),
        _ => t(cx, "agents.statusStopped"),
    }
}

/// "0.4 s" (`0,4 s` in Spanish), "12 s", "3 min 5 s", "1 h 2 min".
pub fn duration(ms: u64, decimal_comma: bool) -> String {
    if ms < 10_000 {
        let tenths = (ms + 50) / 100;
        let sep = if decimal_comma { ',' } else { '.' };
        return format!("{}{sep}{} s", tenths / 10, tenths % 10);
    }
    let s = ms / 1000;
    if s < 60 {
        return format!("{s} s");
    }
    let m = s / 60;
    if m < 60 {
        return if !s.is_multiple_of(60) { format!("{m} min {} s", s % 60) } else { format!("{m} min") };
    }
    let h = m / 60;
    if !m.is_multiple_of(60) { format!("{h} h {} min", m % 60) } else { format!("{h} h") }
}

/// How long to wait before the next ask, after `failures` failures in a row.
pub fn next_delay(failures: u32) -> Duration {
    if failures == 0 {
        return LIVE_POLL;
    }
    let ms = LIVE_POLL.as_millis() as u64 * 2u64.saturating_pow(failures.min(8));
    Duration::from_millis(ms.min(LIVE_BACKOFF_MAX.as_millis() as u64))
}

/// A run of inline text: plain, `code` or **bold**.
#[derive(Clone, Debug, PartialEq)]
pub enum Inline {
    Text(String),
    Code(String),
    Bold(String),
}

/// One block of the agent's prose.
#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// Lines of one paragraph (shown with line breaks between them).
    Para(Vec<Vec<Inline>>),
    List(Vec<Vec<Inline>>),
    Heading(Vec<Inline>),
}

/// Only `` `code` `` and `**bold**`; never HTML.
pub fn inline(text: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut rest = text;
    let mut plain = String::new();
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('`')
            && let Some(end) = after.find('`')
            && end > 0
        {
            if !plain.is_empty() {
                out.push(Inline::Text(std::mem::take(&mut plain)));
            }
            out.push(Inline::Code(after[..end].to_string()));
            rest = &after[end + 1..];
            continue;
        }
        if let Some(after) = rest.strip_prefix("**")
            && let Some(end) = after.find("**")
            && end > 0
        {
            if !plain.is_empty() {
                out.push(Inline::Text(std::mem::take(&mut plain)));
            }
            out.push(Inline::Bold(after[..end].to_string()));
            rest = &after[end + 2..];
            continue;
        }
        let c = rest.chars().next().unwrap_or(' ');
        plain.push(c);
        rest = &rest[c.len_utf8()..];
    }
    if !plain.is_empty() {
        out.push(Inline::Text(plain));
    }
    out
}

/// A list line (`-`, `*`, `•`, `1.`) without its marker, or `None`.
fn list_line(line: &str) -> Option<&str> {
    let l = line.trim_start();
    for m in ["- ", "* ", "• "] {
        if let Some(rest) = l.strip_prefix(m) {
            return Some(rest);
        }
    }
    let digits = l.bytes().take_while(u8::is_ascii_digit).count();
    if digits > 0 && l[digits..].starts_with(". ") {
        return Some(&l[digits + 2..]);
    }
    None
}

/// `#` to `######` and a space.
fn is_heading(line: &str) -> bool {
    let hashes = line.bytes().take_while(|b| *b == b'#').count();
    (1..=6).contains(&hashes) && line[hashes..].starts_with(char::is_whitespace)
}

/// Split on blank lines: a block of list lines is a list, one `#` line a
/// heading, anything else a paragraph.
pub fn prose_blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut chunk: Vec<&str> = Vec::new();
    let flush = |chunk: &mut Vec<&str>, out: &mut Vec<Block>| {
        if chunk.is_empty() {
            return;
        }
        if chunk.iter().all(|l| list_line(l).is_some()) {
            out.push(Block::List(chunk.iter().map(|l| inline(list_line(l).unwrap_or(l))).collect()));
        } else if chunk.len() == 1 && is_heading(chunk[0]) {
            out.push(Block::Heading(inline(chunk[0].trim_start_matches('#').trim_start())));
        } else {
            out.push(Block::Para(chunk.iter().map(|l| inline(l)).collect()));
        }
        chunk.clear();
    };
    for line in text.lines() {
        if line.trim().is_empty() {
            flush(&mut chunk, &mut out);
        } else {
            chunk.push(line);
        }
    }
    flush(&mut chunk, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::StepRow;

    fn step(n: u64, started: u64, status: &str) -> LiveStep {
        LiveStep {
            n,
            rev: n,
            step: StepRow { started_ms: started, status: status.into(), ..Default::default() },
        }
    }

    fn delta(key: &str, rev: u64, reset: bool, steps: Vec<LiveStep>, prose: Vec<Prose>) -> LiveDelta {
        LiveDelta { key: key.into(), rev, reset, steps, prose, ..Default::default() }
    }

    #[test]
    fn a_delta_updates_a_step_in_place_and_a_reset_starts_over() {
        let a = merge_live(None, &delta("k", 1, true, vec![step(1, 10, "running"), step(2, 20, "running")], vec![]));
        let b = merge_live(Some(&a), &delta("k", 2, false, vec![step(2, 20, "ok"), step(3, 30, "running")], vec![]));
        let ns: Vec<(u64, &str)> = b.steps.iter().map(|s| (s.n, s.step.status.as_str())).collect();
        assert_eq!(ns, [(1, "running"), (2, "ok"), (3, "running")]);
        assert_eq!(b.rev, 2);
        let c = merge_live(Some(&b), &delta("k", 3, true, vec![step(9, 90, "ok")], vec![]));
        assert_eq!(c.steps.len(), 1);
        let d = merge_live(Some(&c), &delta("other", 4, false, vec![], vec![]));
        assert!(d.steps.is_empty());
    }

    #[test]
    fn prose_comes_before_the_calls_it_announces() {
        let s = LiveState {
            steps: vec![step(1, 100, "ok"), step(2, 50, "ok")],
            prose: vec![Prose { n: 1, ts_ms: 100, text: "x".into(), ..Default::default() }],
            ..Default::default()
        };
        let order: Vec<String> = live_items(&s)
            .iter()
            .map(|i| match i {
                LiveItem::Step(st) => format!("s{}", st.n),
                LiveItem::Prose(p) => format!("p{}", p.n),
            })
            .collect();
        assert_eq!(order, ["s2", "p1", "s1"]);
    }

    #[test]
    fn durations_as_shown() {
        assert_eq!(duration(200, false), "0.2 s");
        assert_eq!(duration(400, true), "0,4 s");
        assert_eq!(duration(12_300, false), "12 s");
        assert_eq!(duration(48_000, false), "48 s");
        assert_eq!(duration(185_000, false), "3 min 5 s");
        assert_eq!(duration(180_000, false), "3 min");
        assert_eq!(duration(3_720_000, false), "1 h 2 min");
    }

    #[test]
    fn after_failures_it_asks_less_and_less_often() {
        assert_eq!(next_delay(0), Duration::from_secs(1));
        assert_eq!(next_delay(1), Duration::from_secs(2));
        assert_eq!(next_delay(3), Duration::from_secs(8));
        assert_eq!(next_delay(8), Duration::from_secs(30));
    }

    #[test]
    fn prose_is_paragraphs_lists_and_headings_never_html() {
        let blocks = prose_blocks("# Plan\n\n- one `a`\n- two\n\nSome **bold**\nnext <b>x</b>");
        assert_eq!(blocks[0], Block::Heading(vec![Inline::Text("Plan".into())]));
        assert_eq!(
            blocks[1],
            Block::List(vec![
                vec![Inline::Text("one ".into()), Inline::Code("a".into())],
                vec![Inline::Text("two".into())]
            ])
        );
        assert_eq!(
            blocks[2],
            Block::Para(vec![
                vec![Inline::Text("Some ".into()), Inline::Bold("bold".into())],
                vec![Inline::Text("next <b>x</b>".into())]
            ])
        );
    }

    #[test]
    fn the_heading_says_finished_once_the_turn_ended() {
        assert!(live_heading("running", "working"));
        assert!(!live_heading("finished", "working"));
        assert!(!live_heading("verified", "working"));
        assert!(!live_heading("normal", "finished"));
    }
}
