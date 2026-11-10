//! Message wording and selection from the console's `lib/workspace.ts`:
//! the label prefix, folding, mentions, the operator note's delivery line,
//! days, repeats and replies.
//!
//! The logic returns data; the words come from the small functions that
//! take `&App`.

use crate::i18n::{t, tv};
use crate::model::{Agent, Message, NoteDelivery, WorkspaceView};
use crate::ui::format::{hhmm, local_day};
use gpui::{App, SharedString};

/// serve's fixed lead author and the name it shows.
const LEAD_AUTHOR: &str = super::LEAD_AUTHOR;
const LEAD_NAME: &str = "Claude";
const DEFAULT_OPERATOR_NAME: &str = "Operator";

/// The message as the reader should see it beside its label chip. Agents
/// write `<LABEL>[ vN] · <name> · <sentence>`; the chip already says the
/// label and the avatar the name, so the first line keeps only the
/// sentence. A text of another shape is shown as written.
pub fn without_label_prefix(text: &str, label: Option<&str>) -> String {
    let Some(label) = label else {
        return text.to_string();
    };
    let (first, rest) = match text.find('\n') {
        Some(i) => (&text[..i], &text[i..]),
        None => (text, ""),
    };
    let parts: Vec<&str> = first.split(" · ").collect();
    let head = parts[0].trim();
    let (word, version) = match head.split_once(' ') {
        Some((w, v)) => (w, Some(v)),
        None => (head, None),
    };
    let known = matches!(word, "START" | "BLOCKED" | "FINDING" | "DONE");
    let version_ok = version.is_none_or(|v| {
        v.len() > 1 && v.starts_with('v') && v[1..].chars().all(|c| c.is_ascii_digit())
    });
    if !known || !version_ok || word != label {
        return text.to_string();
    }
    let after = &parts[1..];
    // `DONE v2 · what changed` may drop the name; otherwise the name goes too.
    let kept = if version.is_some() && after.len() == 1 {
        after
    } else {
        after.get(1..).unwrap_or(&[])
    };
    let sentence = kept.join(" · ");
    let sentence = sentence.trim();
    if sentence.is_empty() {
        text.to_string()
    } else {
        format!("{sentence}{rest}")
    }
}

/// A message longer than this, in characters, folds in a channel.
pub const FOLD_CHARS: usize = 480;
/// Or longer than this, in lines.
const FOLD_LINES: usize = 8;

/// Where a long message folds in a channel: the part shown before "Show
/// more", cut at a word boundary, or None when it shows whole.
pub fn fold_text(text: &str) -> Option<String> {
    let lines: Vec<&str> = text.split('\n').collect();
    let mut head: String = if lines.len() > FOLD_LINES {
        lines[..FOLD_LINES].join("\n")
    } else {
        text.to_string()
    };
    if head.chars().count() > FOLD_CHARS {
        let cut: String = head.chars().take(FOLD_CHARS).collect();
        // The last whitespace followed only by non-whitespace (`/\s\S*$/`).
        let space = cut
            .char_indices()
            .filter(|(_, c)| c.is_whitespace())
            .map(|(i, _)| i)
            .next_back();
        head = match space {
            Some(i) if i > 0 => cut[..i].to_string(),
            _ => cut,
        };
    }
    let head = head.trim_end();
    (head.chars().count() < text.trim_end().chars().count()).then(|| head.to_string())
}

/// A piece of message text, and whether it is a highlighted `@mention`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub text: String,
    pub mention: bool,
}

fn token_char(c: char) -> bool {
    c.is_alphanumeric() || matches!(c, '.' | '_' | '-')
}

/// A message's text cut at the mentions it holds (`message.mentions`), for
/// highlighting. Without mentions, one plain segment.
pub fn mention_segments(text: &str, mentions: &[String]) -> Vec<Segment> {
    if mentions.is_empty() {
        return vec![Segment {
            text: text.to_string(),
            mention: false,
        }];
    }
    let known: Vec<String> = mentions.iter().map(|m| m.to_lowercase()).collect();
    let mut out = Vec::new();
    let mut last = 0;
    let mut prev: Option<char> = None;
    let mut iter = text.char_indices().peekable();
    while let Some((at, c)) = iter.next() {
        let starts =
            c == '@' && at >= last && prev.is_none_or(|p| !(p.is_alphanumeric() || p == '.'));
        prev = Some(c);
        if !starts {
            continue;
        }
        let start = at + 1;
        let mut end = start;
        while let Some(&(i, ch)) = iter.peek() {
            if !token_char(ch) {
                break;
            }
            end = i + ch.len_utf8();
            prev = Some(ch);
            iter.next();
        }
        if end == start {
            continue;
        }
        let token = text[start..end].trim_end_matches(['.', '_', '-']);
        if token.is_empty() || !known.contains(&token.to_lowercase()) {
            continue;
        }
        if at > last {
            out.push(Segment {
                text: text[last..at].to_string(),
                mention: false,
            });
        }
        out.push(Segment {
            text: format!("@{token}"),
            mention: true,
        });
        last = start + token.len();
    }
    if last < text.len() {
        out.push(Segment {
            text: text[last..].to_string(),
            mention: false,
        });
    }
    out
}

/// The name of whoever signs as a session: its callsign, or "Claude" for an
/// unnamed lead.
pub fn lead_display_name(author: &str) -> &str {
    if author == LEAD_AUTHOR {
        LEAD_NAME
    } else {
        author
    }
}

/// The operator's name, or "Operator" in this language when unset.
pub fn operator_name(name: &str, cx: &App) -> SharedString {
    if name.is_empty() || name == DEFAULT_OPERATOR_NAME {
        t(cx, "workspace.operatorDefaultName")
    } else {
        name.to_string().into()
    }
}

/// Who reads what is written in a conversation: the session itself, or the
/// lead an in-process agent depends on. None when the read has no such session.
pub fn note_reader(agents: &[Agent], key: &str) -> Option<String> {
    let agent = agents.iter().find(|a| a.key == key)?;
    if agent.kind == "session" {
        return Some(lead_display_name(&agent.name).to_string());
    }
    let lead = format!("session:{}", agent.lead_session);
    agents
        .iter()
        .find(|a| a.key == lead)
        .map(|a| lead_display_name(&a.name).to_string())
}

/// Who an operator note went to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Recipient {
    Named(String),
    Lead,
    Sessions { delivered: u64, total: usize },
    Session(String),
}

/// What became of an operator note.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery {
    RepliedBy(String),
    Replied,
    ReadBy(String),
    ReadByLead,
    Read,
    Delivered(Recipient),
    Sent,
}

/// The note's state and, when it was re-sent, when.
pub fn delivery_state(d: &NoteDelivery, reader: Option<&str>) -> (Delivery, Option<u64>) {
    let many = d.sessions.len() > 1;
    let named = if many { None } else { reader };
    let who = match named {
        Some(n) => Recipient::Named(n.to_string()),
        None if d.to_lead && !many => Recipient::Lead,
        None if many => Recipient::Sessions {
            delivered: d.delivered,
            total: d.sessions.len(),
        },
        None => Recipient::Session(
            d.sessions
                .first()
                .map(|s| s.chars().take(8).collect())
                .unwrap_or_default(),
        ),
    };
    let line = match d.state.as_str() {
        "answered" => match &d.answered_by {
            Some(by) => Delivery::RepliedBy(lead_display_name(by).to_string()),
            None => Delivery::Replied,
        },
        "read" => match named {
            Some(n) => Delivery::ReadBy(n.to_string()),
            None if d.to_lead && !many => Delivery::ReadByLead,
            None => Delivery::Read,
        },
        "delivered" => Delivery::Delivered(who),
        _ => Delivery::Sent,
    };
    (line, d.nudged_ms.filter(|&ms| ms > 0))
}

/// The line under one of the operator's notes, like a chat app's sent/seen.
/// `reader` is [`note_reader`]'s.
pub fn delivery_line(d: &NoteDelivery, reader: Option<&str>, cx: &App) -> SharedString {
    let (state, nudged) = delivery_state(d, reader);
    let line = match state {
        Delivery::RepliedBy(name) => tv(cx, "workspace.repliedBy", &[("name", &name)]),
        Delivery::Replied => t(cx, "workspace.replied"),
        Delivery::ReadBy(name) => tv(cx, "workspace.readBy", &[("name", &name)]),
        Delivery::ReadByLead => t(cx, "workspace.readByLead"),
        Delivery::Read => t(cx, "workspace.read"),
        Delivery::Delivered(who) => {
            let who = match who {
                Recipient::Named(name) => tv(cx, "workspace.toNamed", &[("name", &name)]),
                Recipient::Lead => t(cx, "workspace.toLead"),
                Recipient::Sessions { delivered, total } => tv(
                    cx,
                    "workspace.toSessions",
                    &[
                        ("delivered", &delivered.to_string()),
                        ("total", &total.to_string()),
                    ],
                ),
                Recipient::Session(id) => tv(cx, "workspace.toSession", &[("id", &id)]),
            };
            tv(cx, "workspace.deliveredTo", &[("who", &who)])
        }
        Delivery::Sent => t(cx, "workspace.sent"),
    };
    match nudged {
        Some(ms) => tv(
            cx,
            "workspace.lineResent",
            &[("line", &line), ("time", &hhmm(ms))],
        ),
        None => line,
    }
}

/// Items of one local day, in input order.
#[derive(Clone, Debug)]
pub struct Day<'a> {
    /// Days since the epoch, local time.
    pub key: i64,
    /// The first item's time, for the label.
    pub first_ms: u64,
    pub items: Vec<&'a Message>,
}

impl Day<'_> {
    /// "Today", "Yesterday" or the date.
    pub fn label(&self, now: u64, cx: &App) -> SharedString {
        crate::ui::format::day_label(self.first_ms, now, cx)
    }
}

/// Messages grouped by the operator's day; consecutive same-day items stay
/// together, input order kept.
pub fn by_day<'a>(items: &[&'a Message]) -> Vec<Day<'a>> {
    let mut out: Vec<Day<'a>> = Vec::new();
    for &m in items {
        let key = local_day(m.ts_ms);
        match out.last_mut() {
            Some(day) if day.key == key => day.items.push(m),
            _ => out.push(Day {
                key,
                first_ms: m.ts_ms,
                items: vec![m],
            }),
        }
    }
    out
}

/// Copies of a message collapsed under it, by `repeat_of`.
pub fn repeats_of<'a>(view: &'a WorkspaceView, id: &str) -> Vec<&'a Message> {
    view.messages
        .iter()
        .filter(|m| m.repeat_of.as_deref() == Some(id))
        .collect()
}

/// "1 reply", "3 replies".
pub fn reply_count(n: usize, cx: &App) -> SharedString {
    if n == 1 {
        t(cx, "workspace.repliesOne")
    } else {
        tv(cx, "workspace.repliesMany", &[("n", &n.to_string())])
    }
}

/// The session that wrote a lead message: the message's own session, or the
/// lead of the agent it was written to. None when neither is known.
pub fn lead_key_of(agents: &[Agent], agent_key: &str) -> Option<String> {
    if agent_key.is_empty() {
        return None;
    }
    if agent_key.starts_with("session:") {
        return Some(agent_key.to_string());
    }
    agents
        .iter()
        .find(|a| a.key == agent_key)
        .map(|a| a.lead_session.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| format!("session:{s}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_labelled_message_keeps_only_its_sentence() {
        assert_eq!(
            without_label_prefix(
                "DONE · cart · The cart drawer updates its total.",
                Some("DONE")
            ),
            "The cart drawer updates its total."
        );
        assert_eq!(
            without_label_prefix("DONE v2 · Fixed the rounding.", Some("DONE")),
            "Fixed the rounding."
        );
        assert_eq!(
            without_label_prefix(
                "FINDING · i18n · Totals · German prices round early.",
                Some("FINDING")
            ),
            "Totals · German prices round early."
        );
        assert_eq!(
            without_label_prefix(
                "BLOCKED · migrations · May I lock it?\nDetails follow.",
                Some("BLOCKED")
            ),
            "May I lock it?\nDetails follow."
        );
    }

    #[test]
    fn an_unlabelled_message_or_another_shape_is_shown_as_written() {
        assert_eq!(
            without_label_prefix("DONE · cart · x", None),
            "DONE · cart · x"
        );
        assert_eq!(
            without_label_prefix("Done with the cart", Some("DONE")),
            "Done with the cart"
        );
        assert_eq!(
            without_label_prefix("START · cart · x", Some("DONE")),
            "START · cart · x"
        );
        assert_eq!(
            without_label_prefix("DONE · cart", Some("DONE")),
            "DONE · cart"
        );
    }

    #[test]
    fn long_text_folds_at_a_word_and_short_text_does_not() {
        assert_eq!(fold_text("short"), None);
        let long = "word ".repeat(200);
        let head = fold_text(&long).expect("folds");
        assert!(head.chars().count() <= FOLD_CHARS);
        assert!(head.ends_with("word"));
        let lines = (1..=12)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(fold_text(&lines).unwrap().lines().count(), 8);
        let eight = (1..=8)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert_eq!(fold_text(&eight), None);
        assert_eq!(fold_text(&format!("{eight}\n\n")), None);
    }

    #[test]
    fn only_known_mentions_are_highlighted() {
        let segs = mention_segments(
            "hi @dana and @nobody, ask @Cart-team.",
            &["dana".into(), "cart-team".into()],
        );
        assert_eq!(
            segs,
            vec![
                Segment {
                    text: "hi ".into(),
                    mention: false
                },
                Segment {
                    text: "@dana".into(),
                    mention: true
                },
                Segment {
                    text: " and @nobody, ask ".into(),
                    mention: false
                },
                Segment {
                    text: "@Cart-team".into(),
                    mention: true
                },
                Segment {
                    text: ".".into(),
                    mention: false
                },
            ]
        );
        assert_eq!(
            mention_segments("a@dana", &["dana".into()]),
            vec![Segment {
                text: "a@dana".into(),
                mention: false
            }]
        );
        assert_eq!(
            mention_segments("@dana", &[]),
            vec![Segment {
                text: "@dana".into(),
                mention: false
            }]
        );
        assert_eq!(
            mention_segments("@dana", &["dana".into()]),
            vec![Segment {
                text: "@dana".into(),
                mention: true
            }]
        );
    }

    fn agent(key: &str, name: &str, kind: &str, lead: &str) -> Agent {
        Agent {
            key: key.into(),
            name: name.into(),
            kind: kind.into(),
            lead_session: lead.into(),
            ..Default::default()
        }
    }

    #[test]
    fn the_reader_is_the_session_itself_or_the_lead_of_an_agent() {
        let agents = vec![
            agent("session:lead-1", "Walter", "session", "lead-1"),
            agent("a1", "Benjamin", "teammate", "lead-1"),
            agent("a2", "Nora", "teammate", "gone"),
        ];
        assert_eq!(
            note_reader(&agents, "session:lead-1").as_deref(),
            Some("Walter")
        );
        assert_eq!(note_reader(&agents, "a1").as_deref(), Some("Walter"));
        assert_eq!(note_reader(&agents, "a2"), None);
        assert_eq!(note_reader(&agents, ""), None);
    }

    fn note(state: &str) -> NoteDelivery {
        NoteDelivery {
            state: state.into(),
            sessions: vec!["lead-1".into()],
            delivered: if state == "sent" { 0 } else { 1 },
            to_lead: true,
            ..Default::default()
        }
    }

    #[test]
    fn a_note_says_what_became_of_it() {
        let mut answered = note("answered");
        answered.answered_by = Some("Walter".into());
        assert_eq!(
            delivery_state(&answered, Some("Walter")).0,
            Delivery::RepliedBy("Walter".into())
        );
        assert_eq!(
            delivery_state(&note("answered"), Some("Walter")).0,
            Delivery::Replied
        );
        assert_eq!(
            delivery_state(&note("delivered"), Some("Walter")).0,
            Delivery::Delivered(Recipient::Named("Walter".into()))
        );
        assert_eq!(
            delivery_state(&note("read"), Some("Walter")).0,
            Delivery::ReadBy("Walter".into())
        );
        assert_eq!(delivery_state(&note("sent"), None).0, Delivery::Sent);
        assert_eq!(
            delivery_state(&note("delivered"), None).0,
            Delivery::Delivered(Recipient::Lead)
        );
        assert_eq!(delivery_state(&note("read"), None).0, Delivery::ReadByLead);
        let mut many = note("delivered");
        many.sessions = vec!["a".into(), "b".into()];
        assert_eq!(
            delivery_state(&many, Some("Walter")).0,
            Delivery::Delivered(Recipient::Sessions {
                delivered: 1,
                total: 2
            })
        );
        let mut one = note("delivered");
        one.to_lead = false;
        one.sessions = vec!["0123456789".into()];
        assert_eq!(
            delivery_state(&one, None).0,
            Delivery::Delivered(Recipient::Session("01234567".into()))
        );
        let mut nudged = note("sent");
        nudged.nudged_ms = Some(42);
        assert_eq!(delivery_state(&nudged, None), (Delivery::Sent, Some(42)));
    }

    #[test]
    fn the_unnamed_lead_reads_as_claude() {
        assert_eq!(lead_display_name("Claude · lead"), "Claude");
        assert_eq!(lead_display_name("Walter"), "Walter");
    }

    fn at(id: &str, ts: u64) -> Message {
        Message {
            id: id.into(),
            ts_ms: ts,
            ..Default::default()
        }
    }

    #[test]
    fn days_group_consecutive_items_in_order() {
        const MIN: u64 = 60_000;
        let now: u64 = 1_793_918_460_000; // 2026-11-06 22:41 UTC
        let ms = [
            at("a", now - 25 * 60 * MIN),
            at("b", now - 2 * MIN),
            at("c", now - MIN),
        ];
        let refs: Vec<&Message> = ms.iter().collect();
        let groups = by_day(&refs);
        assert_eq!(
            groups.iter().map(|g| g.items.len()).collect::<Vec<_>>(),
            vec![1, 2]
        );
        assert_eq!(groups[1].first_ms, now - 2 * MIN);
    }

    #[test]
    fn repeats_collapse_under_the_first_copy() {
        let mut again = at("m2", 2);
        again.repeat_of = Some("m1".into());
        let v = WorkspaceView {
            messages: vec![at("m1", 1), again],
            ..Default::default()
        };
        assert_eq!(
            repeats_of(&v, "m1")
                .iter()
                .map(|m| m.id.as_str())
                .collect::<Vec<_>>(),
            vec!["m2"]
        );
    }

    #[test]
    fn a_lead_message_names_its_session() {
        let agents = vec![agent("a1", "Benjamin", "teammate", "lead-1")];
        assert_eq!(
            lead_key_of(&agents, "session:x").as_deref(),
            Some("session:x")
        );
        assert_eq!(
            lead_key_of(&agents, "a1").as_deref(),
            Some("session:lead-1")
        );
        assert_eq!(lead_key_of(&agents, "zz"), None);
        assert_eq!(lead_key_of(&agents, ""), None);
    }
}
