//! A plain line for a message written for machines.
//!
//! Many agents, above all other sessions' agents, ignore the protocol's human
//! line: their first line is engineer shorthand ("CAP-146 is done: head
//! acb3816f on fix/runner-pg-env-signatures, pushed."). [`readable`] turns it
//! into a line the operator can read, without an LLM: pattern rules over the
//! first line and the protocol label.
//!
//! - A known shape (a ticket done, pushed, approved, blocked…) becomes a plain
//!   sentence with no commit codes, branches or counts noise: "CAP-146 done
//!   and pushed", "CAP-52 pushed · 82 of 82 tests pass". Each piece carries a
//!   code, so the console can word it in the operator's language.
//! - Any other line shows its first sentence, with commit codes, branches
//!   and paths collapsed into small chips, cut at a word boundary.
//!
//! The original line stays where it was, under "Technical details". Lexical
//! only: it decides nothing about the work.

use crate::coded::{Param, Params, params};
use serde::Serialize;

/// Longest first sentence shown for an unknown shape, in characters, chips
/// counted at their shown length.
pub const SENTENCE_CAP: usize = 140;

/// One piece of a readable line.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Part {
    /// `text`, or a chip: `sha`, `branch` or `path`.
    pub kind: &'static str,
    /// What shows: the text, or the chip's short form.
    pub text: String,
    /// A chip's whole value, for its tooltip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full: Option<String>,
    /// The console's code for a harbard-worded `text`, filled from `params`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    #[serde(skip_serializing_if = "Params::is_empty")]
    pub params: Params,
}

impl Part {
    fn text(s: impl Into<String>) -> Self {
        Part {
            kind: "text",
            text: s.into(),
            full: None,
            code: None,
            params: Params::new(),
        }
    }

    fn coded(code: &'static str, text: impl Into<String>, values: Params) -> Self {
        Part {
            code: Some(code),
            params: values,
            ..Part::text(text)
        }
    }

    /// Every word a reader sees through the shared filter; a code is not one.
    fn masked(mut self) -> Self {
        self.text = crate::redact::mask_secrets(&self.text);
        for value in self.params.values_mut() {
            if let Param::Text(text) = value {
                *text = crate::redact::mask_secrets(text);
            }
        }
        self
    }
}

/// The readable line of a message whose human line reads as jargon, or
/// `None` when the line is fine as it is. `sentence` is the human line (the
/// label prefix already taken off); `label` is the protocol label, if any.
pub fn readable(sentence: &str, label: Option<&str>) -> Option<Vec<Part>> {
    let line = sentence.trim();
    if line.is_empty() || (crate::human::jargon(line).is_empty() && !crate::human::is_english(line))
    {
        return None;
    }
    // A known shape keeps only tickets and words of its own. Its status
    // words are English and it needs a subject, so a line in another
    // language, even one too short to read as English, never takes one.
    if let Some((parts, _)) = known_shape(line, label) {
        return Some(parts.into_iter().map(Part::masked).collect());
    }
    let parts = first_sentence(line);
    let unchanged = parts.len() == 1 && parts[0].kind == "text" && parts[0].text == line;
    if unchanged {
        return None;
    }
    // Every shown piece through the shared filter: a token that reads as a
    // credential is masked in the text and in a chip's tooltip; a chip shows
    // at most seven characters of a hash.
    Some(
        parts
            .into_iter()
            .map(|p| Part {
                text: if p.kind == "text" || p.kind == "branch" {
                    crate::redact::mask_secrets(&p.text)
                } else {
                    p.text
                },
                full: p.full.map(|f| crate::redact::mask_secrets(&f)),
                ..p
            })
            .collect(),
    )
}

/// What a message is, for grouping a run of them in a channel:
/// `delivery` (work done, pushed, fixed), `review` (approved, changes
/// requested) or `blocked`. The protocol label decides first (DONE,
/// BLOCKED); an unlabelled English line, by its known shape. `None` for
/// anything else.
pub fn shape(sentence: &str, label: Option<&str>) -> Option<&'static str> {
    match label {
        Some("DONE") => return Some("delivery"),
        Some("BLOCKED") => return Some("blocked"),
        Some(_) => return None,
        None => {}
    }
    let (_, status) = known_shape(sentence.trim(), label)?;
    Some(match status {
        Status::Blocked => "blocked",
        Status::Changes | Status::Approved | Status::Answered => "review",
        Status::FixPushed
        | Status::Fixed
        | Status::Done
        | Status::Merged
        | Status::Committed
        | Status::Pushed
        | Status::Green => "delivery",
    })
}

// ---------------------------------------------------------------------------
// Known shapes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Blocked,
    Changes,
    Approved,
    Answered,
    FixPushed,
    Fixed,
    Done,
    Merged,
    Committed,
    Pushed,
    Green,
}

/// `CAP-146`: two to ten capitals or digits starting with a capital, a
/// hyphen, digits. A review code (`AC-005`) has the same look; only the
/// leading one names the work.
fn is_ticket(word: &str) -> bool {
    let Some((head, num)) = word.split_once('-') else {
        return false;
    };
    (2..=10).contains(&head.len())
        && head.starts_with(|c: char| c.is_ascii_uppercase())
        && head
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        && (1..=5).contains(&num.len())
        && num.bytes().all(|b| b.is_ascii_digit())
}

/// A short lower-case name with a hyphen (`py-surreal`, `ci-fix`).
fn is_kebab_name(word: &str) -> bool {
    let word = word.trim_matches('`');
    word.contains('-')
        && word.len() <= 30
        && word.starts_with(|c: char| c.is_ascii_lowercase())
        && word
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        && !word.ends_with('-')
}

fn bare(word: &str) -> &str {
    word.trim_matches(|c: char| matches!(c, ',' | '.' | ';' | ':' | '(' | ')' | '`'))
}

/// A list of tickets from the start of `words`: `CAP-147, CAP-145 and
/// CAP-148`. Returns the tickets and how many words it took.
fn tickets_at(words: &[&str]) -> (Vec<String>, usize) {
    let mut out = Vec::new();
    let mut i = 0;
    while i < words.len() {
        let w = words[i];
        if is_ticket(bare(w)) {
            out.push(bare(w).to_string());
            i += 1;
            // A list goes on after a comma or an "and".
            if w.ends_with(',') && words.get(i).is_some_and(|n| is_ticket(bare(n))) {
                continue;
            }
            if words.get(i).is_some_and(|n| *n == "and")
                && words.get(i + 1).is_some_and(|n| is_ticket(bare(n)))
            {
                i += 1;
                continue;
            }
            break;
        }
        break;
    }
    (out, i)
}

/// Who a known shape is about. `head` and `last` travel separately so each
/// language can join a list with its own word.
struct Subject {
    who: String,
    head: String,
    last: String,
    many: bool,
}

impl Subject {
    fn new(items: &[String]) -> Self {
        let (head, last) = match items {
            [] => (String::new(), String::new()),
            [init @ .., last] => (init.join(", "), last.clone()),
        };
        let many = items.len() > 1;
        let who = if many {
            format!("{head} and {last}")
        } else {
            last.clone()
        };
        Subject {
            who,
            head,
            last,
            many,
        }
    }

    fn params(&self) -> Params {
        params([
            ("who", self.who.as_str().into()),
            ("head", self.head.as_str().into()),
            ("last", self.last.as_str().into()),
        ])
    }

    /// The code for this subject: lists agree in number in some languages.
    fn code(&self, one: &'static str, many: &'static str) -> &'static str {
        if self.many { many } else { one }
    }
}

/// Whole-word (or whole-phrase) match on a lower-case haystack.
fn has(hay: &str, phrase: &str) -> bool {
    let mut from = 0;
    while let Some(at) = hay[from..].find(phrase) {
        let start = from + at;
        let end = start + phrase.len();
        let before = hay[..start].chars().next_back();
        let after = hay[end..].chars().next();
        let edge = |c: Option<char>| c.is_none_or(|c| !c.is_alphanumeric());
        if edge(before) && edge(after) {
            return true;
        }
        from = end;
    }
    false
}

fn has_any(hay: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|p| has(hay, p))
}

/// The first clause: up to the first sentence end or semicolon.
fn first_clause(line: &str) -> &str {
    let mut end = line.len();
    for pat in [". ", "; ", " — "] {
        if let Some(i) = line.find(pat) {
            end = end.min(i);
        }
    }
    line[..end].trim_end_matches('.')
}

fn status_of(clause: &str) -> Option<Status> {
    let c = clause.to_lowercase();
    if has(&c, "blocked") {
        return Some(Status::Blocked);
    }
    if has_any(
        &c,
        &["changes requested", "requested changes", "request changes"],
    ) {
        return Some(Status::Changes);
    }
    if has(&c, "approved")
        && !has_any(
            &c,
            &[
                "not approved",
                "approved none",
                "approved neither",
                "unapproved",
            ],
        )
    {
        return Some(Status::Approved);
    }
    if has(&c, "answered") {
        return Some(Status::Answered);
    }
    if has_any(&c, &["fix pushed", "fixes pushed"]) {
        return Some(Status::FixPushed);
    }
    if has(&c, "fixed") {
        return Some(Status::Fixed);
    }
    if has_any(
        &c,
        &[
            "done",
            "finished",
            "complete",
            "completed",
            "built",
            "implemented",
        ],
    ) {
        return Some(Status::Done);
    }
    if has(&c, "merged") && !has(&c, "not merged") {
        return Some(Status::Merged);
    }
    if has(&c, "committed") {
        return Some(Status::Committed);
    }
    if has(&c, "pushed") && !negated_push(&c) {
        return Some(Status::Pushed);
    }
    if has_any(&c, &["is green", "are green"]) {
        return Some(Status::Green);
    }
    None
}

fn negated_push(lower: &str) -> bool {
    has_any(
        lower,
        &[
            "not pushed",
            "nothing pushed",
            "nothing is pushed",
            "no push",
            "not yet pushed",
            "without pushing",
        ],
    )
}

/// Pushed, not pushed, or not said: the first clause decides, then the line.
fn pushed(clause: &str, line: &str) -> Option<bool> {
    let c = clause.to_lowercase();
    let l = line.to_lowercase();
    if negated_push(&c) {
        return Some(false);
    }
    if has(&c, "pushed") {
        return Some(true);
    }
    if negated_push(&l) {
        return Some(false);
    }
    has(&l, "pushed").then_some(true)
}

fn tests_count(passed: u64, total: u64) -> Part {
    Part::coded(
        "readable.tests.count",
        format!(" · {passed} of {total} tests pass"),
        params([("passed", passed.into()), ("total", total.into())]),
    )
}

/// What the line says about the tests: `82 of 82 tests pass` (a count, after
/// " · ") or `the criteria tests pass` (after "; ").
fn tests_of(line: &str) -> Option<Part> {
    let l = line.to_lowercase();
    let words: Vec<&str> = l.split_whitespace().collect();
    let num = |w: &str| bare(w).parse::<u64>().ok();
    // `82/82 passing`, `19 of 19 passing`.
    for (i, w) in words.iter().enumerate() {
        let next = words.get(i + 1).map(|n| bare(n));
        if let Some((a, b)) = bare(w).split_once('/')
            && let (Ok(a), Ok(b)) = (a.parse::<u64>(), b.parse::<u64>())
            && matches!(next, Some("passing" | "pass" | "passed"))
        {
            return Some(tests_count(a, b));
        }
        if let (Some(a), Some("of"), Some(b)) =
            (num(w), next, words.get(i + 2).and_then(|n| num(n)))
            && words
                .get(i + 3)
                .is_some_and(|n| matches!(bare(n), "passing" | "pass" | "passed" | "tests"))
        {
            return Some(tests_count(a, b));
        }
    }
    let criteria = l.split(". ").any(|s| {
        (s.contains("criterion") || s.contains("criteria"))
            && (has_any(s, &["pass", "passes", "passed", "passing", "green"]))
            && !has(s, "fail")
            || (s.contains("criterion") || s.contains("criteria"))
                && has_any(s, &["pass", "passes"])
                && has_any(s, &["0 fail", "0 failed"])
    });
    if criteria {
        return Some(Part::coded(
            "readable.tests.criteria",
            "; the criteria tests pass",
            Params::new(),
        ));
    }
    // `8 pass, 0 fail`, `303 pass / 14 fail`.
    for (i, w) in words.iter().enumerate() {
        if let (Some(a), Some("pass")) = (num(w), words.get(i + 1).map(|n| bare(n))) {
            let mut j = i + 2;
            if words.get(j) == Some(&"/") {
                j += 1;
            }
            if let (Some(f), Some("fail" | "failed")) = (
                words.get(j).and_then(|n| num(n)),
                words.get(j + 1).map(|n| bare(n)),
            ) {
                return Some(tests_count(a, a + f));
            }
        }
    }
    None
}

fn known_shape(line: &str, label: Option<&str>) -> Option<(Vec<Part>, Status)> {
    let mut words: Vec<&str> = line.split_whitespace().collect();
    // A one-word speaker prefix: `Revisor: …`, `Ledger: …`, `Blocked: …`.
    let mut prefix: Option<String> = None;
    if let Some(first) = words.first()
        && let Some(name) = first.strip_suffix(':')
        && name.len() >= 3
        && name.starts_with(|c: char| c.is_ascii_uppercase())
        && name.chars().all(|c| c.is_alphabetic() || c == '-')
        && !is_ticket(name)
    {
        if name.eq_ignore_ascii_case("blocked") {
            return Some((
                vec![Part::coded(
                    "readable.blocked_bare",
                    "Blocked · the reason is in \"Technical details\"",
                    Params::new(),
                )],
                Status::Blocked,
            ));
        }
        prefix = Some(name.to_string());
        words.remove(0);
    }
    let rest = words.join(" ");
    let clause = first_clause(&rest);
    let clause_words: Vec<&str> = clause.split_whitespace().collect();
    // The subject: leading tickets, or a short hyphenated name before is/are.
    let (mut subject, _) = tickets_at(&clause_words);
    let mut named_later = false;
    if subject.is_empty()
        && let [name, verb, ..] = clause_words.as_slice()
        && is_kebab_name(bare(name))
        && matches!(*verb, "is" | "are")
    {
        subject = vec![bare(name).trim_matches('`').to_string()];
    }
    let mut status = status_of(clause);
    if status.is_none() && !subject.is_empty() && label == Some("DONE") {
        status = Some(Status::Done);
    }
    let status = status?;
    // A review verdict may name its tickets later: `changes requested on
    // CAP-147, CAP-146 and CAP-145`.
    if subject.is_empty() && matches!(status, Status::Changes | Status::Approved) {
        let at = clause_words.iter().position(|w| is_ticket(bare(w)))?;
        subject = tickets_at(&clause_words[at..]).0;
        named_later = true;
    }
    if subject.is_empty() {
        return None;
    }
    let subject = Subject::new(&subject);
    let who = subject.who.as_str();
    let push = pushed(clause, &rest);
    // (one subject's code, a list's code, the English sentence)
    let delivery = |word: &str, codes: [[&'static str; 2]; 3]| match push {
        Some(true) => (codes[0][0], codes[0][1], format!("{who} {word} and pushed")),
        Some(false) => (
            codes[1][0],
            codes[1][1],
            format!("{who} {word}, not pushed"),
        ),
        None => (codes[2][0], codes[2][1], format!("{who} {word}")),
    };
    let ((one, list, english), delivered) = match status {
        Status::Blocked => (
            (
                "readable.blocked",
                "readable.blocked.many",
                format!("{who} blocked · the reason is in \"Technical details\""),
            ),
            false,
        ),
        Status::Changes if named_later && prefix.is_some() => (
            (
                "readable.changes_on",
                "readable.changes_on.many",
                format!("changes requested on {who}"),
            ),
            false,
        ),
        Status::Changes if named_later => (
            (
                "readable.changes_on.start",
                "readable.changes_on.start.many",
                format!("Changes requested on {who}"),
            ),
            false,
        ),
        Status::Changes => (
            (
                "readable.changes",
                "readable.changes.many",
                format!("{who}: changes requested"),
            ),
            false,
        ),
        Status::Approved => (
            (
                "readable.approved",
                "readable.approved.many",
                format!("{who} approved"),
            ),
            false,
        ),
        Status::Answered => (
            (
                "readable.answered",
                "readable.answered.many",
                format!("{who}: review answered"),
            ),
            false,
        ),
        Status::FixPushed => (
            (
                "readable.fix_pushed",
                "readable.fix_pushed.many",
                format!("{who}: fix pushed"),
            ),
            true,
        ),
        Status::Fixed => (
            delivery(
                "fixed",
                [
                    ["readable.fixed.pushed", "readable.fixed.pushed.many"],
                    ["readable.fixed.unpushed", "readable.fixed.unpushed.many"],
                    ["readable.fixed", "readable.fixed.many"],
                ],
            ),
            true,
        ),
        Status::Done => (
            delivery(
                "done",
                [
                    ["readable.done.pushed", "readable.done.pushed.many"],
                    ["readable.done.unpushed", "readable.done.unpushed.many"],
                    ["readable.done", "readable.done.many"],
                ],
            ),
            true,
        ),
        Status::Merged => (
            (
                "readable.merged",
                "readable.merged.many",
                format!("{who} merged"),
            ),
            true,
        ),
        Status::Committed => (
            delivery(
                "committed",
                [
                    [
                        "readable.committed.pushed",
                        "readable.committed.pushed.many",
                    ],
                    [
                        "readable.committed.unpushed",
                        "readable.committed.unpushed.many",
                    ],
                    ["readable.committed", "readable.committed.many"],
                ],
            ),
            true,
        ),
        Status::Pushed => (
            (
                "readable.pushed",
                "readable.pushed.many",
                format!("{who} pushed"),
            ),
            true,
        ),
        Status::Green => (
            (
                "readable.green",
                "readable.green.many",
                if subject.many {
                    format!("{who} are green")
                } else {
                    format!("{who} is green")
                },
            ),
            true,
        ),
    };
    let mut parts = Vec::new();
    if let Some(p) = prefix {
        parts.push(Part::text(format!("{p}: ")));
    }
    parts.push(Part::coded(
        subject.code(one, list),
        english,
        subject.params(),
    ));
    if delivered && status != Status::FixPushed {
        parts.extend(tests_of(&rest));
    }
    Some((parts, status))
}

// ---------------------------------------------------------------------------
// Unknown shapes: the first sentence, with chips
// ---------------------------------------------------------------------------

/// A lower-case git hash: 7 to 40 hex digits, at least one digit and one
/// letter.
fn is_sha(token: &str) -> bool {
    (7..=40).contains(&token.len())
        && token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        && token.bytes().any(|b| b.is_ascii_digit())
        && token.bytes().any(|b| b.is_ascii_alphabetic())
}

/// `feat/value`, `origin/main`: lower-case segments joined by slashes, with
/// at least one letter and no file extension on the last segment.
fn is_branch(token: &str) -> bool {
    let segs: Vec<&str> = token.split('/').collect();
    (2..=4).contains(&segs.len())
        && segs.iter().all(|s| {
            !s.is_empty()
                && s.bytes().all(|b| {
                    b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_' | b'.')
                })
        })
        && token.bytes().any(|b| b.is_ascii_lowercase())
        && !has_extension(segs[segs.len() - 1])
        && !token.contains("..")
}

/// `harness.ts`, `report.md`: a file name, not a branch.
fn has_extension(seg: &str) -> bool {
    seg.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && (1..=5).contains(&ext.len())
            && ext.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

/// A path: absolute (`/…`, `~/…`) or a relative one to a file
/// (`apps/app/test/e2e-harness.ts`).
fn is_path(token: &str) -> bool {
    let absolute = (token.starts_with('/') || token.starts_with("~/")) && token.len() >= 3;
    let relative = token.contains('/')
        && !token.starts_with('/')
        && token.split('/').all(|s| !s.is_empty())
        && has_extension(token.rsplit('/').next().unwrap_or(""));
    absolute || relative
}

fn chip(core: &str) -> Option<Part> {
    if is_sha(core) {
        return Some(Part {
            kind: "sha",
            text: core[..7].to_string(),
            full: Some(core.to_string()),
            ..Part::text("")
        });
    }
    if is_path(core) {
        let name = core
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or(core);
        return Some(Part {
            kind: "path",
            text: name.to_string(),
            full: Some(core.to_string()),
            ..Part::text("")
        });
    }
    if is_branch(core) {
        return Some(Part {
            kind: "branch",
            text: core.to_string(),
            full: Some(core.to_string()),
            ..Part::text("")
        });
    }
    None
}

/// The first sentence of `line`: up to `. ` or `; `, keeping its period.
fn sentence_of(line: &str) -> &str {
    let mut end = line.len();
    if let Some(i) = line.find(". ") {
        end = end.min(i + 1);
    }
    if let Some(i) = line.find("; ") {
        end = end.min(i);
    }
    line[..end].trim_end()
}

fn first_sentence(line: &str) -> Vec<Part> {
    let sentence = sentence_of(line);
    let mut parts: Vec<Part> = Vec::new();
    let mut shown = 0usize;
    let push_text = |parts: &mut Vec<Part>, s: &str| {
        if let Some(last) = parts.last_mut()
            && last.kind == "text"
        {
            last.text.push_str(s);
        } else {
            parts.push(Part::text(s));
        }
    };
    let mut cut = false;
    for (n, word) in sentence.split(' ').enumerate() {
        let lead = word
            .find(|c: char| !matches!(c, '(' | '[' | '`' | '"' | '\'' | '«'))
            .unwrap_or(word.len());
        let core_end = word
            .trim_end_matches(|c: char| {
                matches!(
                    c,
                    ',' | '.' | ';' | ':' | ')' | ']' | '`' | '"' | '\'' | '»' | '!' | '?'
                )
            })
            .len()
            .max(lead);
        let (open, core, close) = (&word[..lead], &word[lead..core_end], &word[core_end..]);
        let piece = chip(core);
        let width = piece.as_ref().map_or(word.chars().count(), |p| {
            open.chars().count() + p.text.chars().count() + close.chars().count()
        }) + usize::from(n > 0);
        if shown + width > SENTENCE_CAP && shown > 0 {
            cut = true;
            break;
        }
        shown += width;
        if n > 0 {
            push_text(&mut parts, " ");
        }
        match piece {
            Some(p) => {
                if !open.is_empty() {
                    push_text(&mut parts, open);
                }
                parts.push(p);
                if !close.is_empty() {
                    push_text(&mut parts, close);
                }
            }
            None => push_text(&mut parts, word),
        }
    }
    if cut {
        // Cut at a word boundary: drop a dangling comma or colon, add `…`.
        if let Some(last) = parts.last_mut()
            && last.kind == "text"
        {
            let t = last
                .text
                .trim_end_matches([' ', ',', ';', ':', '('])
                .to_string();
            last.text = t;
        }
        push_text(&mut parts, "…");
    }
    parts.retain(|p| !(p.kind == "text" && p.text.is_empty()));
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chips as `[x]`, to read a line in one assertion.
    fn shown(parts: &[Part]) -> String {
        parts
            .iter()
            .map(|p| match p.kind {
                "text" => p.text.clone(),
                _ => format!("[{}]", p.text),
            })
            .collect()
    }

    fn r(line: &str) -> String {
        shown(&readable(line, None).unwrap_or_else(|| panic!("no readable line for {line:?}")))
    }

    /// The #acme screenshot, word for word.
    #[test]
    fn the_acme_screenshot_reads_as_a_plain_line() {
        assert_eq!(
            r(
                "CAP-146 is done: head acb3816f on fix/runner-pg-env-signatures, pushed. The slots freed (5/32 in use; I did not free them), so the earlier blocker is gone."
            ),
            "CAP-146 done and pushed"
        );
        assert_eq!(
            r(
                "CAP-143 is done: head d8493113 on fix/e2e-dist-cleanup, pushed. All 5 criterion commands pass as written, and all 19 harness mutations plus 1 real-tree mutation were caught."
            ),
            "CAP-143 done and pushed; the criteria tests pass"
        );
        assert_eq!(
            r(
                "CAP-143 AC-005 fix pushed: fix/e2e-dist-cleanup at a9aac889 (on top of d849311). Only apps/app/test/e2e-harness.ts and e2e-dist-cleanup.test.ts changed; the existing assertions are untouched."
            ),
            "CAP-143: fix pushed"
        );
        assert_eq!(
            r(
                "CAP-52 is pushed: head ee140405 on ui/structured-descriptions, 82/82 passing on the criterion command."
            ),
            "CAP-52 pushed · 82 of 82 tests pass"
        );
        assert_eq!(
            r(
                "CAP-82 is built and pushed. Branch feat/value, head 3a9808e4ace2d53cd91e17639e8675ba559468f4, one commit on top of integration 027993c (31 files, +1973/−29, no AI attribution). All five criterion comm…"
            ),
            "CAP-82 done and pushed"
        );
        assert_eq!(
            r(
                "CAP-150 is built and pushed: head 7e78934d on fix/operator-agent-name (origin), one commit on top of integration 027993c. Both criterion commands pass exactly as written: AC-001 2 pass / 0 fail, AC-00…"
            ),
            "CAP-150 done and pushed; the criteria tests pass"
        );
        assert_eq!(
            r(
                "CAP-149 is built and pushed: head 37c87ae30a30cdb8979b5e21ca526aa43617adb5 on fix/lease-record-accepted, one commit on integration 027993c. Both criterion commands pass exactly as written: AC-001 3 pa…"
            ),
            "CAP-149 done and pushed; the criteria tests pass"
        );
        assert_eq!(
            r(
                "CAP-86 is built and pushed: feat/merge-ahead at 0ddb0953c5ece26d1940fa520706568f932cd3e9 (one commit on 3eda782; 21 files, +1489/-17). All four criteria pass. 19 of 19 mutations were caught."
            ),
            "CAP-86 done and pushed; the criteria tests pass"
        );
    }

    /// Real first lines from the operator's transcripts (read through a throwaway
    /// serve; personal paths shortened to `/Users/p`).
    const CORPUS: [(&str, &str); 26] = [
        (
            "CAP-146 is blocked on shared memory, not on code: the machine is at 28/32 System V segments and 23 of them have nattch 0 (dead clusters nobody will cl…",
            "CAP-146 blocked · the reason is in \"Technical details\"",
        ),
        (
            "Revisor: changes requested on CAP-147, CAP-146 and CAP-145. Each request includes a replacement-criteria proposal (criteria apply CAP-n).",
            "Revisor: changes requested on CAP-147, CAP-146 and CAP-145",
        ),
        (
            "CAP-148: changes requested. AC-001 has to compare the files against what init's own planner (planHarness, hooks part, for claude and codex) would writ…",
            "CAP-148: changes requested",
        ),
        (
            "CAP-146 approved (now ready); CAP-147, CAP-145 and CAP-148 are not approved, because `criteria apply` replaced their whole criteria sets and dropped t…",
            "CAP-146 approved",
        ),
        (
            "CAP-147, CAP-145 and CAP-148 are approved: each has 3 criteria, identical in text and command to the full set I proposed, and all three are now ready.",
            "CAP-147, CAP-145 and CAP-148 approved",
        ),
        (
            "CAP-138 is built at commit 9d147dd on feat/cli-workspace-admin (not pushed). Both criterion commands pass as written: 5 pass, 0 fail, 88 expect() calls, and typecheck is clean.",
            "CAP-138 done, not pushed; the criteria tests pass",
        ),
        (
            "CAP-135 is built and committed on feat/cli-setup: 804e0fa (not pushed, nothing submitted).",
            "CAP-135 done, not pushed",
        ),
        (
            "CAP-139 is committed on feat/cli-views-events at ca656fc (not pushed), and both criteria commands pass on the committed tree: 8 pass, 0 fail, typecheck clean. chain verify CAP-n needed a new API route…",
            "CAP-139 committed, not pushed; the criteria tests pass",
        ),
        (
            "CAP-78 AC-004 is fixed on feat/flow-time at 28c8f03 (parent 7637b5d). Nothing pushed, claimed or submitted.",
            "CAP-78 fixed, not pushed",
        ),
        (
            "Revisor: CAP-144 is approved; it is now ready and claimable.",
            "Revisor: CAP-144 approved",
        ),
        (
            "I requested changes on both CAP-149 and CAP-150 and approved neither. My reasons are recorded with each request; I sent no --file proposal, so the existing criteria stay as they are.",
            "Changes requested on CAP-149 and CAP-150",
        ),
        (
            "CAP-137 is finished and pushed. feat/cli-task-acts head b4063865877e5126c8fb39629b26b089953d35ec. Every approved criterion command, run exactly as written, passes.",
            "CAP-137 done and pushed; the criteria tests pass",
        ),
        (
            "CAP-133 is done and pushed: fix/keyhelper-reaps-pg at 14513a0da62f4948a1293cc81ee0183bd363335a. All five criteria pass, plus the pinned, reproducible and strict tests.",
            "CAP-133 done and pushed; the criteria tests pass",
        ),
        (
            "CAP-145 is built and pushed. Head a16cda13f50135befad228a42ffaa9edfc00e800 on feat/cli-flow-progress (from 9fee085, one commit, no AI attribution). It was not claimed, submitted or heartbeated.",
            "CAP-145 done and pushed",
        ),
        (
            "CAP-148 is built and pushed: chore/self-guard @ 1d4a3241186c253219e1a3f3798cd390deba54df (one commit, no AI attribution). It merges cleanly into the c…",
            "CAP-148 done and pushed",
        ),
        (
            "py-surreal is done: 7 commits on work/py-surreal in /Users/p/Projects/contoso-wt-py-surreal (cut from 148ce54). Not pushed, worktree clean, release binary rebuilt.",
            "py-surreal done, not pushed",
        ),
        (
            "ci-fix is green on CI. Run 42229429371 passed all four jobs: Rust, Linux clean room (67/0, and 50/0 with --no-service), macOS clean room (79/0) and Eval. The branch is pushed at 7635cb2 and is not mer…",
            "ci-fix is green",
        ),
        (
            "CAP-136 and CAP-135 review answered: criteria rewritten, code and tests extended, both branches committed and pushed. Nothing claimed, submitted or approved.",
            "CAP-136 and CAP-135: review answered",
        ),
        (
            "CAP-141 is built on feat/progress (worktree progress), two commits, all five criteria green; one trivial tsconfig.json conflict with the integration that moved (CAP-91).",
            "CAP-141 done; the criteria tests pass",
        ),
        (
            "Blocked: my isolated worktree is a checkout of ~/.claude (dotfiles), not Contoso, so the ledger task can't start. Path: /Users/p/.claude/.claude/worktrees/•••• HEAD…",
            "Blocked · the reason is in \"Technical details\"",
        ),
        // Unknown shapes: the first sentence, with chips.
        (
            "CAP-136: I updated the coverage map to your assignments in commit 3d9222c on feat/cli-route-coverage (no push).",
            "CAP-136: I updated the coverage map to your assignments in commit [3d9222c] on [feat/cli-route-coverage] (no push).",
        ),
        (
            "CAP-144 está hecho y subido: feat/hook-guard en 76260c0ea7624724a333420e6444639238b59437, un solo commit sobre 03e030a, sin atribución a IA. Los 7 criterios pasan con sus comandos tal cual y…",
            "CAP-144 está hecho y subido: [feat/hook-guard] en [76260c0], un solo commit sobre [03e030a], sin atribución a IA.",
        ),
        (
            "Both review findings are fixed and strip is rebased onto robots fcf628c; the new head is 1582702, frozen.",
            "Both review findings are fixed and strip is rebased onto robots [fcf628c]",
        ),
        (
            "Team view built on branch team-view (f7a6e0d over b42ddae), report at /Users/p/Documents/notes/2026-11-06/harbard-view.md",
            "Team view built on branch team-view ([f7a6e0d] over [b42ddae]), report at [harbard-view.md]",
        ),
        (
            "Loop signal event kinds are on branch loop-core, commit d0138ca (pushed). Rules in crates/orchard-core/src/loops.rs (validate, validate_envelope); store derives one `signal` row per loop even…",
            "Loop signal event kinds are on branch loop-core, commit [d0138ca] (pushed).",
        ),
        (
            "Applied your CAP-148 proposal unchanged. Please approve it too if it stands (with 147, 146, 145).",
            "Applied your CAP-148 proposal unchanged.",
        ),
    ];

    #[test]
    fn twenty_six_real_lines_read_as_expected() {
        let mut wrong = Vec::new();
        for (line, want) in CORPUS {
            // The fixtures went through the shared filter: nothing in them
            // reads as a credential but the full git hashes, which it masks
            // as it would any 40-hex token.
            let hashes_aside: String = line
                .split(' ')
                .map(|w| {
                    let core = w.trim_end_matches(['.', ',', ';', ')']);
                    if core.len() == 40 && is_sha(core) {
                        "<sha>"
                    } else {
                        w
                    }
                })
                .collect::<Vec<_>>()
                .join(" ");
            assert_eq!(
                crate::redact::mask_secrets(&hashes_aside),
                hashes_aside,
                "fixture masks: {line}"
            );
            let got = readable(line, None).map(|p| shown(&p));
            if got.as_deref() != Some(want) {
                wrong.push(format!("{line}\n  want {want}\n  got  {got:?}"));
            }
        }
        assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    }

    #[test]
    fn shape_names_what_a_message_is() {
        let s = |line: &str, label: Option<&str>| shape(line, label);
        assert_eq!(
            s("Ya puedes probar el panel nuevo.", Some("DONE")),
            Some("delivery")
        );
        assert_eq!(
            s("No puedo seguir sin tu clave.", Some("BLOCKED")),
            Some("blocked")
        );
        assert_eq!(s("Empiezo con la cabecera.", Some("START")), None);
        assert_eq!(
            s("CAP-146 is done: head acb3816f on fix/x, pushed.", None),
            Some("delivery")
        );
        assert_eq!(
            s("CAP-143 AC-005 fix pushed: fix/e2e at a9aac889.", None),
            Some("delivery")
        );
        assert_eq!(
            s(
                "CAP-52 is pushed: head ee140405 on ui/x, 82/82 passing.",
                None
            ),
            Some("delivery")
        );
        assert_eq!(
            s("CAP-149 is approved and now ready and claimable.", None),
            Some("review")
        );
        assert_eq!(
            s(
                "Revisor: changes requested on CAP-147, CAP-146 and CAP-145.",
                None
            ),
            Some("review")
        );
        assert_eq!(
            s("CAP-146 is blocked on shared memory, not on code.", None),
            Some("blocked")
        );
        assert_eq!(
            s(
                "Blocked: my isolated worktree is a checkout of ~/.claude.",
                None
            ),
            Some("blocked")
        );
        assert_eq!(s("Applied your CAP-148 proposal unchanged.", None), None);
        assert_eq!(s("", None), None);
    }

    #[test]
    fn a_plain_spanish_line_is_left_alone() {
        for line in [
            "Ya puedes ver las capturas de antes y después de la consola.",
            "acme ya sabe qué trabajo se ha integrado sin verificar.",
            "",
        ] {
            assert_eq!(readable(line, None), None, "{line}");
        }
        // Jargon with nothing to collapse and one sentence: as it is.
        assert_eq!(readable("He hecho los commits que faltaban", None), None);
    }

    #[test]
    fn the_label_stands_in_for_a_missing_status() {
        assert_eq!(
            readable("CAP-12 at 1a2b3c4d on feat/x, all of it now.", Some("DONE"))
                .map(|p| shown(&p)),
            Some("CAP-12 done".to_string())
        );
    }

    #[test]
    fn a_long_unknown_sentence_is_cut_at_a_word_boundary() {
        let line = "The calendar fix stops recreating finished calendar events on every daily run and keeps the operator's manual entries where they were, on work/alerts-dashboard with 1e74575 and more words after that";
        let got = r(line);
        assert!(got.ends_with('…'), "{got}");
        assert!(!got.contains("1e74575 and more"), "{got}");
        let visible: usize = readable(line, None)
            .unwrap()
            .iter()
            .map(|p| p.text.chars().count())
            .sum();
        assert!(visible <= SENTENCE_CAP + 1, "{visible}");
        // A word is never split.
        assert!(
            got.trim_end_matches('…').split(' ').all(|w| !w.is_empty()),
            "{got}"
        );
    }

    #[test]
    fn chips_keep_the_whole_value_for_the_tooltip() {
        let parts = readable(
            "Loop kinds are on branch loop-core, commit d0138ca4f2 in /Users/p/a/b.md (pushed).",
            None,
        )
        .unwrap();
        let sha = parts.iter().find(|p| p.kind == "sha").unwrap();
        assert_eq!(
            (sha.text.as_str(), sha.full.as_deref()),
            ("d0138ca", Some("d0138ca4f2"))
        );
        let path = parts.iter().find(|p| p.kind == "path").unwrap();
        assert_eq!(
            (path.text.as_str(), path.full.as_deref()),
            ("b.md", Some("/Users/p/a/b.md"))
        );
    }

    #[test]
    fn a_secret_in_the_line_never_reaches_a_part() {
        let s = concat!("ghp_abcd", "efghijklmnop1234567890ABCD");
        let parts = readable(
            &format!("The token {s} is pushed on feat/x at 1a2b3c4d."),
            None,
        )
        .unwrap();
        assert!(
            parts
                .iter()
                .all(|p| !p.text.contains(s) && !p.full.as_deref().unwrap_or("").contains(s))
        );
    }
}
