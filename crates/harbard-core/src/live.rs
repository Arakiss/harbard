//! The private live view of one agent ("See it live"), and the hover
//! card's fresh steps while it is open.
//!
//! [`LiveLog`] folds one transcript, the one the operator opened, into a timeline
//! of every tool call and the agent's own prose. serve keeps it only while
//! that view is open, and reads no file the team projection does not
//! already read.
//!
//! What never leaves here: thinking blocks, tool input beyond the safe label
//! of [`crate::activity`], and tool output (from a test run only the counts,
//! [`crate::progress::test_counts`]). Prose is the agent's own words, and
//! every word that reads as a credential is masked
//! ([`crate::redact::mask_secrets`]).

use crate::activity::Call;
use crate::progress::{Progress, StepRow, Tally, running_step};
use crate::team::{Tail, parse_line, read_appended, recognise};
use serde::Serialize;
use serde_json::Value;
use std::collections::VecDeque;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Steps a live timeline keeps (the newest).
pub const STEPS_KEPT: usize = 400;
/// Prose messages a live view keeps (the newest).
pub const PROSE_KEPT: usize = 150;
/// Characters kept of one prose message.
pub const PROSE_CHARS: usize = 8_000;

/// Every change gets a number from one process-wide counter, so a log that
/// was dropped and read again never answers a cursor from the old one.
static REV: AtomicU64 = AtomicU64::new(1);

fn next_rev() -> u64 {
    REV.fetch_add(1, Ordering::Relaxed)
}

/// One tool call of the timeline. `n` is its place in the transcript; `rev`
/// is when it last changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LiveStep {
    pub n: u64,
    pub rev: u64,
    #[serde(flatten)]
    pub step: StepRow,
}

/// One message of the agent's own prose, masked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Prose {
    pub n: u64,
    pub rev: u64,
    pub ts_ms: u64,
    pub text: String,
}

/// `GET /api/v1/live/<agent>`: what changed since the cursor `after`.
#[derive(Debug, Clone, Serialize)]
pub struct LiveDelta {
    pub v: u32,
    /// The cursor for the next request.
    pub rev: u64,
    /// The client drops what it holds: this is everything kept.
    pub reset: bool,
    pub first_ms: Option<u64>,
    pub last_ms: Option<u64>,
    pub steps: Vec<LiveStep>,
    pub prose: Vec<Prose>,
    pub progress: Progress,
}

/// One transcript folded into a timeline and prose, read incrementally.
#[derive(Debug, Clone)]
pub struct LiveLog {
    tail: Tail,
    base_rev: u64,
    rev: u64,
    next_n: u64,
    next_prose: u64,
    steps: VecDeque<LiveStep>,
    prose: VecDeque<Prose>,
    /// Calls with no result yet: `tool_use` id, step number, call.
    pending: Vec<(String, u64, Call)>,
    tally: Tally,
    /// `message.id` of the newest prose, to join the records of one reply.
    last_message: Option<String>,
    first_ms: Option<u64>,
    last_ms: Option<u64>,
    skipped: usize,
}

impl Default for LiveLog {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveLog {
    pub fn new() -> Self {
        let rev = next_rev();
        LiveLog {
            tail: Tail::default(),
            base_rev: rev,
            rev,
            next_n: 0,
            next_prose: 0,
            steps: VecDeque::new(),
            prose: VecDeque::new(),
            pending: Vec::new(),
            tally: Tally::default(),
            last_message: None,
            first_ms: None,
            last_ms: None,
            skipped: 0,
        }
    }

    /// Read what was appended to `path`; start over if it shrank.
    pub fn refresh(&mut self, path: &Path) {
        let mut tail = std::mem::take(&mut self.tail);
        let shrunk = read_appended(path, &mut tail, |line| self.feed(line)).is_err();
        self.tail = tail;
        if shrunk {
            *self = LiveLog::new();
            let mut tail = Tail::default();
            let _ = read_appended(path, &mut tail, |line| self.feed(line));
            self.tail = tail;
        }
    }

    fn bump(&mut self) -> u64 {
        self.rev = next_rev();
        self.rev
    }

    /// Fold one transcript line.
    pub fn feed(&mut self, line: &[u8]) {
        let Some(v) = parse_line(line, &mut self.skipped) else {
            return;
        };
        let Some((typ, ts)) = recognise(&v) else {
            return;
        };
        self.first_ms = Some(self.first_ms.map_or(ts, |f| f.min(ts)));
        self.last_ms = Some(self.last_ms.map_or(ts, |l| l.max(ts)));
        if typ == "assistant" {
            self.assistant(&v, ts);
        } else {
            self.user(&v, ts);
        }
    }

    fn assistant(&mut self, v: &Value, ts: u64) {
        let msg = v.get("message");
        let blocks = msg
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let api_error = v.get("isApiErrorMessage").and_then(Value::as_bool) == Some(true);
        let id = msg
            .and_then(|m| m.get("id"))
            .and_then(Value::as_str)
            .map(|s| s.chars().take(128).collect::<String>());
        let mut tool = false;
        let mut said = String::new();
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    tool = true;
                    let name = block.get("name").and_then(Value::as_str).unwrap_or("");
                    let input = block.get("input").unwrap_or(&Value::Null);
                    let call_id: String = block
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .chars()
                        .take(128)
                        .collect();
                    let call = Call::of(name, input, ts);
                    self.tally.called();
                    let n = self.next_n;
                    self.next_n += 1;
                    let rev = self.bump();
                    self.push_step(LiveStep {
                        n,
                        rev,
                        step: running_step(&call),
                    });
                    self.pending.retain(|(p, _, _)| *p != call_id);
                    if self.pending.len() >= 64 {
                        self.pending.remove(0);
                    }
                    self.pending.push((call_id, n, call));
                }
                // Only the agent's own words; thinking stays hidden.
                Some("text") if !api_error => {
                    if let Some(t) = block.get("text").and_then(Value::as_str) {
                        if !said.is_empty() {
                            said.push_str("\n\n");
                        }
                        said.push_str(t);
                    }
                }
                _ => {}
            }
        }
        if !said.trim().is_empty() {
            self.add_prose(said.trim(), id, ts);
        }
        let stop = msg
            .and_then(|m| m.get("stop_reason"))
            .and_then(Value::as_str);
        let closed = !tool && (api_error || stop.is_some_and(|s| s != "tool_use"));
        if closed {
            self.stop_pending(ts);
        }
    }

    fn add_prose(&mut self, text: &str, id: Option<String>, ts: u64) {
        let masked = crate::redact::mask_secrets(text);
        let rev = self.bump();
        if id.is_some()
            && id == self.last_message
            && let Some(last) = self.prose.back_mut()
        {
            last.text = cap(&format!("{}\n\n{masked}", last.text));
            last.rev = rev;
            return;
        }
        if self.prose.len() >= PROSE_KEPT {
            self.prose.pop_front();
        }
        let n = self.next_prose;
        self.next_prose += 1;
        self.prose.push_back(Prose {
            n,
            rev,
            ts_ms: ts,
            text: cap(&masked),
        });
        self.last_message = id;
    }

    fn push_step(&mut self, step: LiveStep) {
        if self.steps.len() >= STEPS_KEPT {
            self.steps.pop_front();
        }
        self.steps.push_back(step);
    }

    /// The turn ended with calls unanswered.
    fn stop_pending(&mut self, ts: u64) {
        for (_, n, call) in std::mem::take(&mut self.pending) {
            self.tally.stopped(&call, ts);
            let rev = self.bump();
            if let Some(s) = self.steps.iter_mut().rev().find(|s| s.n == n) {
                s.step.status = "stopped";
                s.step.ended_ms = Some(ts);
                s.rev = rev;
            }
        }
    }

    fn user(&mut self, v: &Value, ts: u64) {
        if v.get("isMeta").and_then(Value::as_bool) == Some(true) {
            return;
        }
        match v.pointer("/message/content") {
            Some(Value::Array(blocks)) => {
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("tool_result") => {
                            let id = block
                                .get("tool_use_id")
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            let Some(at) = self.pending.iter().position(|(p, _, _)| p == id) else {
                                continue;
                            };
                            let (_, n, call) = self.pending.remove(at);
                            let done = self.tally.finish(&call, ts, block);
                            let rev = self.bump();
                            if let Some(s) = self.steps.iter_mut().rev().find(|s| s.n == n) {
                                s.step = done;
                                s.rev = rev;
                            }
                        }
                        Some("text")
                            if block
                                .get("text")
                                .and_then(Value::as_str)
                                .is_some_and(|t| t.contains("[Request interrupted")) =>
                        {
                            self.stop_pending(ts);
                        }
                        _ => {}
                    }
                }
            }
            Some(Value::String(s)) if s.contains("[Request interrupted") => self.stop_pending(ts),
            _ => {}
        }
    }

    /// What changed after the cursor `after` (everything when `after` is
    /// none or from another log). Prose only when asked for.
    pub fn delta(&self, after: Option<u64>, with_prose: bool) -> LiveDelta {
        let reset = after.is_none_or(|a| a < self.base_rev || a > self.rev);
        let after = if reset { 0 } else { after.unwrap_or(0) };
        LiveDelta {
            v: 1,
            rev: self.rev,
            reset,
            first_ms: self.first_ms,
            last_ms: self.last_ms,
            steps: self
                .steps
                .iter()
                .filter(|s| s.rev > after)
                .cloned()
                .collect(),
            prose: if with_prose {
                self.prose
                    .iter()
                    .filter(|p| p.rev > after)
                    .cloned()
                    .collect()
            } else {
                Vec::new()
            },
            progress: self.tally.progress(self.pending.iter().map(|(_, _, c)| c)),
        }
    }
}

fn cap(text: &str) -> String {
    if text.chars().count() <= PROSE_CHARS {
        return text.to_string();
    }
    let mut out: String = text.chars().take(PROSE_CHARS).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clock::rfc3339_utc;
    use crate::progress::TestCounts;
    use serde_json::json;

    const NOW: u64 = 1_800_000_000_000;

    fn counts(passed: u32, failed: u32, skipped: u32) -> Option<TestCounts> {
        Some(TestCounts {
            passed,
            failed,
            skipped,
        })
    }

    // ---------------------------------------------------------------------
    // The live log
    // ---------------------------------------------------------------------

    fn rec(typ: &str, ms: u64, message: Value) -> Vec<u8> {
        let v = json!({"type":typ,"timestamp":rfc3339_utc(ms),"message":message});
        let mut line = serde_json::to_vec(&v).unwrap();
        line.push(b'\n');
        line
    }

    fn said(ms: u64, id: &str, text: &str) -> Vec<u8> {
        rec(
            "assistant",
            ms,
            json!({"id":id,"role":"assistant","stop_reason":null,"content":[{"type":"text","text":text}]}),
        )
    }

    #[test]
    fn the_live_log_shows_calls_and_prose_but_no_thinking_or_output() {
        let mut log = LiveLog::new();
        log.feed(&rec(
            "user",
            NOW,
            json!({"role":"user","content":"Build it."}),
        ));
        log.feed(&rec(
            "assistant",
            NOW + 1_000,
            json!({"id":"m1","role":"assistant","content":[{"type":"thinking","thinking":"THINKING-SENTINEL","signature":"s"}]}),
        ));
        log.feed(&said(NOW + 2_000, "m1", "Starting with the tests."));
        log.feed(&rec(
            "assistant",
            NOW + 3_000,
            json!({"id":"m1","role":"assistant","stop_reason":"tool_use","content":[
                {"type":"tool_use","id":"t1","name":"Bash","input":{"command": format!("API_KEY={} bun test src", "x".repeat(30))}},
                {"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"/Users/p/.env.local"}}]}),
        ));
        log.feed(&rec(
            "user",
            NOW + 9_000,
            json!({"role":"user","content":[
                {"type":"tool_result","tool_use_id":"t1","content":"OUTPUT-SENTINEL\n 10 pass\n 1 fail\n","is_error":true},
                {"type":"tool_result","tool_use_id":"t2","content":"SECRET=FILE-SENTINEL"}]}),
        ));
        log.feed(&rec(
            "assistant",
            NOW + 10_000,
            json!({"id":"m2","role":"assistant","stop_reason":"tool_use","content":[
                {"type":"tool_use","id":"t3","name":"Edit","input":{"file_path":"/w/src/card.tsx","old_string":"INPUT-SENTINEL"}}]}),
        ));
        let d = log.delta(None, true);
        assert!(d.reset);
        type Row<'a> = (u64, &'a str, Option<&'a str>, &'a str, Option<TestCounts>);
        let steps: Vec<Row> = d
            .steps
            .iter()
            .map(|s| {
                (
                    s.n,
                    s.step.state,
                    s.step.detail.as_deref(),
                    s.step.status,
                    s.step.test,
                )
            })
            .collect();
        assert_eq!(
            steps,
            vec![
                (0, "testing", Some("bun test"), "error", counts(10, 1, 0)),
                (1, "reading", None, "ok", None),
                (2, "editing", Some("card.tsx"), "running", None),
            ]
        );
        assert_eq!(d.steps[0].step.started_ms, NOW + 3_000);
        assert_eq!(d.steps[0].step.ended_ms, Some(NOW + 9_000));
        assert_eq!(d.prose.len(), 1);
        assert_eq!(d.prose[0].text, "Starting with the tests.");
        assert_eq!(d.progress.tool_calls, 3);
        assert_eq!(d.progress.test.unwrap().counts, counts(10, 1, 0).unwrap());
        let wire = serde_json::to_string(&d).unwrap();
        for leak in [
            "THINKING-SENTINEL",
            "OUTPUT-SENTINEL",
            "FILE-SENTINEL",
            "INPUT-SENTINEL",
            "sk-live",
            "API_KEY",
            ".env",
            "Build it",
        ] {
            assert!(!wire.contains(leak), "{leak} reached the live view: {wire}");
        }
    }

    #[test]
    fn the_records_of_one_reply_join_and_a_turn_end_stops_unanswered_calls() {
        let mut log = LiveLog::new();
        log.feed(&said(NOW, "m1", "First."));
        log.feed(&said(NOW + 100, "m1", "Second."));
        log.feed(&said(NOW + 200, "m2", "Another answer."));
        log.feed(&rec(
            "assistant",
            NOW + 300,
            json!({"id":"m3","role":"assistant","stop_reason":"tool_use","content":[
                {"type":"tool_use","id":"t1","name":"Agent","input":{"prompt":"PROMPT-SENTINEL"}}]}),
        ));
        let mid = log.delta(None, true);
        assert_eq!(
            mid.prose
                .iter()
                .map(|p| p.text.as_str())
                .collect::<Vec<_>>(),
            vec!["First.\n\nSecond.", "Another answer."]
        );
        log.feed(&rec(
            "user",
            NOW + 400,
            json!({"role":"user","content":[{"type":"text","text":"[Request interrupted by user]"}]}),
        ));
        let d = log.delta(Some(mid.rev), true);
        assert_eq!(d.steps.len(), 1);
        assert_eq!(
            (d.steps[0].step.state, d.steps[0].step.status),
            ("delegating", "stopped")
        );
        assert_eq!(
            d.steps[0].step.ended_ms,
            Some(NOW + 400),
            "a stopped call ends when its turn did"
        );
        assert_eq!(
            d.progress
                .steps
                .iter()
                .map(|s| s.status)
                .collect::<Vec<_>>(),
            vec!["stopped"],
            "the card's steps agree with the timeline"
        );
        assert!(d.prose.is_empty());
        assert!(
            !serde_json::to_string(&d)
                .unwrap()
                .contains("PROMPT-SENTINEL")
        );
    }

    // ---------------------------------------------------------------------
    // The redaction property and the review's matrix, over the live JSON
    // (the generators live with the filter, `crate::redact::testkit`).
    // ---------------------------------------------------------------------

    use crate::redact::testkit::{REVIEW_TEMPLATES, Rng, WORDS, review_secret, secret};

    #[test]
    fn property_no_secret_in_the_live_prose_and_never_the_whole_message() {
        let mut r = Rng(0x9E37_79B9_7F4A_7C15);
        for case in 0..3_000 {
            let mut text = String::new();
            let mut values = Vec::new();
            let mut plain_words = Vec::new();
            let n = 4 + r.below(30);
            for i in 0..n {
                if i > 0 {
                    text.push_str(r.pick(&[" ", " ", " ", "\n", "\n\n", "  ", "\t"]));
                }
                if r.below(5) == 0 {
                    let (shown, value) = secret(&mut r);
                    text.push_str(&shown);
                    values.push(value);
                    // The word after a secret's name may be taken as its value.
                    text.push(' ');
                    text.push('y');
                } else {
                    let w = r.pick(WORDS);
                    text.push_str(w);
                    plain_words.push(w);
                }
            }
            let mut log = LiveLog::new();
            log.feed(&said(NOW, &format!("m{case}"), &text));
            let wire = serde_json::to_string(&log.delta(None, true)).unwrap();
            let shown = &log.delta(None, true).prose[0].text;
            for v in &values {
                assert!(
                    !wire.contains(v.as_str()),
                    "case {case}: {v} leaked from {text:?} as {shown:?}"
                );
                // Nor the first or last 16 characters of a long one.
                if v.len() >= 24 {
                    assert!(
                        !shown.contains(&v[..16]),
                        "case {case}: head of {v} in {shown:?}"
                    );
                    assert!(
                        !shown.contains(&v[v.len() - 16..]),
                        "case {case}: tail of {v} in {shown:?}"
                    );
                }
            }
            // Ordinary words survive: the message is never masked whole. A
            // word right after a secret's name may be taken for its value.
            let kept = plain_words.iter().filter(|w| shown.contains(*w)).count();
            let lost = plain_words.len() - kept;
            assert!(
                lost <= 1 + plain_words.len() / 5,
                "case {case}: only {kept} of {} plain words kept in {shown:?}",
                plain_words.len()
            );
            assert!(!shown.trim().is_empty() || text.trim().is_empty());
        }
    }

    #[test]
    fn the_reviews_matrix_of_shapes_never_leaks_a_secret() {
        let mut r = Rng(0xD1EC_7000_0BAD_F00D);
        let mut leaks = Vec::new();
        for _ in 0..40 {
            for (name, t) in REVIEW_TEMPLATES {
                let (class, v) = review_secret(&mut r);
                let text = format!(
                    "Done. {} and I carry on with the branch.",
                    t.replace("{}", &v)
                );
                let mut log = LiveLog::new();
                log.feed(&said(NOW, "m", &text));
                let d = log.delta(None, true);
                let wire = serde_json::to_string(&d).unwrap();
                let shown = &d.prose[0].text;
                if wire.contains(&v) || shown.contains(&v) {
                    leaks.push(format!("{name}/{class}: {text:?} -> {shown:?}"));
                }
                assert!(
                    shown.starts_with("Done. ")
                        && shown.ends_with(" and I carry on with the branch."),
                    "{shown:?}"
                );
            }
        }
        assert!(
            leaks.is_empty(),
            "{} leaks:\n{}",
            leaks.len(),
            leaks.join("\n")
        );
    }
}
