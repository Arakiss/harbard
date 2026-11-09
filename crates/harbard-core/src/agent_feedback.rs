//! What agents report about harbard itself, as notes in `#feedback`.
//!
//! Distinct from [`crate::feedback`], the operator's one-tap votes. An agent
//! writes one sentence and harbard adds the rest: harness, session and
//! repository are already on the event, and the version is added here.
//!
//! The note is the whole record: its first line is `[<kind>] <text>`, then
//! `harbard <version>`, then `verb: <verb>` when one was named. It stays on
//! this machine; nothing here sends anything anywhere.

use serde::Serialize;

/// What a report can be.
pub const KINDS: [&str; 4] = ["bug", "friction", "idea", "praise"];

/// One report, read back from a note's text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentFeedback {
    pub kind: String,
    pub text: String,
    pub version: Option<String>,
    pub verb: Option<String>,
}

/// One line, however it was typed: the first line of the note is the format.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The note text for a report. Refuses an unknown kind or an empty report.
pub fn compose(
    kind: &str,
    text: &str,
    verb: Option<&str>,
    version: &str,
) -> Result<String, String> {
    if !KINDS.contains(&kind) {
        return Err(format!(
            "unknown feedback kind \"{kind}\"; use one of {}",
            KINDS.join(", ")
        ));
    }
    let text = one_line(text);
    if text.is_empty() {
        return Err(
            "feedback needs one sentence: harbard feedback --kind friction \"what got in your way\""
                .to_string(),
        );
    }
    let mut body = format!("[{kind}] {text}\nharbard {version}");
    if let Some(verb) = verb.map(one_line).filter(|v| !v.is_empty()) {
        let verb = verb.strip_prefix("harbard ").unwrap_or(&verb);
        body.push_str(&format!("\nverb: {verb}"));
    }
    Ok(body)
}

/// Read a report back. `None` for a note in `#feedback` that was not written
/// by `harbard feedback`, so a hand-written line is never misread as one.
pub fn parse(body: &str) -> Option<AgentFeedback> {
    let mut lines = body.lines();
    let first = lines.next()?;
    let (kind, text) = first.strip_prefix('[')?.split_once("] ")?;
    if !KINDS.contains(&kind) {
        return None;
    }
    let mut report = AgentFeedback {
        kind: kind.to_string(),
        text: text.to_string(),
        version: None,
        verb: None,
    };
    for line in lines {
        if let Some(version) = line.strip_prefix("harbard ") {
            report.version.get_or_insert_with(|| version.to_string());
        } else if let Some(verb) = line.strip_prefix("verb: ") {
            report.verb.get_or_insert_with(|| verb.to_string());
        }
    }
    Some(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_reads_back_as_it_was_written() {
        let body = compose("bug", "inbox printed nothing", Some("inbox"), "1.2.3").unwrap();
        assert_eq!(
            body,
            "[bug] inbox printed nothing\nharbard 1.2.3\nverb: inbox"
        );
        let report = parse(&body).unwrap();
        assert_eq!(report.kind, "bug");
        assert_eq!(report.text, "inbox printed nothing");
        assert_eq!(report.version.as_deref(), Some("1.2.3"));
        assert_eq!(report.verb.as_deref(), Some("inbox"));
    }

    #[test]
    fn the_first_line_stays_one_line_however_it_was_typed() {
        let body = compose("idea", "two\n\nlines  here", None, "1").unwrap();
        assert_eq!(body.lines().next(), Some("[idea] two lines here"));
    }

    #[test]
    fn the_verb_may_be_written_with_the_binary_name() {
        let body = compose("friction", "slow", Some("harbard note"), "1").unwrap();
        assert!(body.ends_with("verb: note"), "{body}");
    }

    #[test]
    fn an_unknown_kind_and_an_empty_report_are_refused() {
        assert!(compose("rant", "x", None, "1").is_err());
        assert!(compose("bug", "   ", None, "1").is_err());
    }

    #[test]
    fn a_hand_written_note_is_not_a_report() {
        assert_eq!(parse("the lobby is nice"), None);
        assert_eq!(parse("[rant] loud"), None);
    }
}
