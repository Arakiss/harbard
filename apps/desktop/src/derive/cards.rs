//! Activity's cards in words (`lib/workspace.ts`: `cardReason`, `cardBody`,
//! `checkBody`, `withoutLabelPrefix`, `ellipsizeWords`, `readAllNotice`,
//! `sessionGone`) and `cardHidden` (`lib/hidden.ts`).

use crate::derive::agent_label;
use crate::i18n::{t, tv};
use crate::model::{Agent, Card, CardCheck, Unread, WorkspaceView};
use crate::ui::format::{duration, hhmm};
use gpui::{App, SharedString};

/// The message beside its label chip: `<LABEL>[ vN] · <name> · <sentence>`
/// keeps only the sentence; a text of any other shape is shown as written.
pub fn without_label_prefix<'a>(text: &'a str, label: Option<&str>) -> std::borrow::Cow<'a, str> {
    use std::borrow::Cow;
    let Some(label) = label else { return Cow::Borrowed(text) };
    let (first, rest) = match text.find('\n') {
        Some(i) => (&text[..i], &text[i..]),
        None => (text, ""),
    };
    let parts: Vec<&str> = first.split(" · ").collect();
    let head = parts.first().map(|s| s.trim()).unwrap_or("");
    let (word, version) = match head.split_once(' ') {
        Some((w, v)) => (w, Some(v)),
        None => (head, None),
    };
    let known = matches!(word, "START" | "BLOCKED" | "FINDING" | "DONE");
    let version_ok = match version {
        None => true,
        Some(v) => v.strip_prefix('v').is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())),
    };
    if !known || !version_ok || word != label {
        return Cow::Borrowed(text);
    }
    let after = &parts[1..];
    // `DONE v2 · what changed` may drop the name; otherwise the name goes too.
    let kept: &[&str] = if version.is_some() && after.len() == 1 {
        after
    } else {
        after.get(1..).unwrap_or(&[])
    };
    let sentence = kept.join(" · ");
    let sentence = sentence.trim();
    if sentence.is_empty() {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(format!("{sentence}{rest}"))
    }
}

/// At most `max` characters, cut at a word boundary with "…".
pub fn ellipsize_words(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max.saturating_sub(1)).collect();
    // The last whitespace that is followed only by non-whitespace.
    let trimmed = match cut.char_indices().rev().find(|(_, c)| c.is_whitespace()) {
        Some((i, _)) if i > 0 => &cut[..i],
        _ => &cut[..],
    };
    format!("{}…", trimmed.trim_end())
}

/// The last segment of a path.
pub fn file_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or(path)
}

/// "a", "a and b", "a, b and c".
pub fn list_join(names: &[SharedString], cx: &App) -> SharedString {
    match names.len() {
        0 => SharedString::default(),
        1 => names[0].clone(),
        n => {
            let head: Vec<&str> = names[..n - 1].iter().map(|s| s.as_ref()).collect();
            tv(cx, "workspace.listJoin", &[("head", &head.join(", ")), ("last", &names[n - 1])])
        }
    }
}

/// Nobody reads this agent now: it is lost, or its lead is off the bus or ended.
pub fn session_gone(a: &Agent) -> bool {
    a.lost || a.delivery == "not_on_bus" || a.delivery == "ended"
}

pub fn session_gone_notice(cx: &App) -> SharedString {
    t(cx, "workspace.sessionGone")
}

/// "Blocked in #orchard", "Approval", "Was this you?".
pub fn card_reason(card: &Card, cx: &App) -> SharedString {
    let key = match card.kind.as_str() {
        "blocked" => "reasonBlocked",
        "approval" => "reasonApproval",
        "unanswered" => "reasonUnanswered",
        "mention" => "reasonMention",
        "handoff" => "reasonHandoff",
        "check" => return t(cx, "workspace.reasonCheck"),
        _ => "reasonAlert",
    };
    match &card.channel {
        Some(channel) => tv(cx, &format!("workspace.{key}In"), &[("channel", channel)]),
        None => t(cx, &format!("workspace.{key}")),
    }
}

/// The reason's icon (`ReasonIcon`).
pub fn reason_icon(kind: &str) -> &'static str {
    match kind {
        "blocked" => "circle",
        "approval" => "list-checks",
        "unanswered" => "message-square-warning",
        _ => "triangle-alert",
    }
}

/// A title and a sentence.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Body {
    pub title: SharedString,
    pub text: SharedString,
}

/// "Was it you?": what changed what every session reads.
pub fn check_body(check: Option<&CardCheck>, cx: &App) -> Body {
    let short = |text: &str| ellipsize_words(&text.split_whitespace().collect::<Vec<_>>().join(" "), 160);
    match check.map(|c| c.what.as_str()) {
        Some("rule") => Body {
            title: tv(cx, "workspace.checkRuleTitle", &[("text", &short(&check.unwrap().text))]),
            text: t(cx, "workspace.checkRuleText"),
        },
        Some("context") if !check.unwrap().text.trim().is_empty() => Body {
            title: tv(cx, "workspace.checkContextTitle", &[("text", &short(&check.unwrap().text))]),
            text: t(cx, "workspace.checkContextText"),
        },
        Some("context") => Body {
            title: t(cx, "workspace.checkContextClearedTitle"),
            text: t(cx, "workspace.checkContextClearedText"),
        },
        Some("memory_sink") => Body {
            title: t(cx, "workspace.checkMemorySinkTitle"),
            text: t(cx, "workspace.checkMemorySinkText"),
        },
        _ => Body {
            title: t(cx, "workspace.checkOtherTitle"),
            text: SharedString::default(),
        },
    }
}

/// Title and sentence of a card (`cardBody`).
pub fn card_body(card: &Card, view: &WorkspaceView, now: u64, cx: &App) -> Body {
    let names: Vec<SharedString> = card.agents.iter().map(|k| agent_label(view, k, cx)).collect();
    let since = hhmm(card.since_ms);
    let message = card.message.as_deref().and_then(|id| view.message(id));
    let who = || names.first().cloned().unwrap_or_else(|| t(cx, "workspace.anAgent"));
    let quoted = || match message {
        Some(m) => tv(cx, "workspace.quotedAt", &[("text", &m.text), ("time", &since)]),
        None => tv(cx, "workspace.atTime", &[("time", &since)]),
    };
    match card.kind.as_str() {
        "two_writers" => {
            let title = if names.len() == 2 {
                t(cx, "workspace.twoWritersTitleTwo")
            } else {
                tv(cx, "workspace.twoWritersTitleMany", &[("n", &names.len().to_string())])
            };
            let text = tv(
                cx,
                "workspace.twoWritersText",
                &[("names", &list_join(&names, cx)), ("checkout", file_name(card.checkout.as_deref().unwrap_or("")))],
            );
            let text = if card.needs {
                text
            } else {
                format!("{text} {}", t(cx, "workspace.twoWritersHistory")).into()
            };
            Body { title, text }
        }
        "mention" => Body {
            title: tv(cx, "workspace.mentionTitle", &[("who", &who())]),
            text: quoted(),
        },
        "handoff" => Body {
            title: tv(cx, "workspace.handoffTitle", &[("who", &who())]),
            text: quoted(),
        },
        "unanswered" => {
            let agent = card.agents.first().map(|k| agent_label(view, k, cx));
            let title = match (&card.channel, &agent) {
                (Some(channel), None) => tv(cx, "workspace.unansweredInChannel", &[("channel", channel)]),
                (_, Some(agent)) => tv(cx, "workspace.unansweredToAgent", &[("agent", agent)]),
                (None, None) => t(cx, "workspace.unanswered"),
            };
            let quote = match message {
                Some(m) => tv(cx, "workspace.quotedAt", &[("text", &m.text), ("time", &since)]),
                None => tv(cx, "workspace.writtenAt", &[("time", &since)]),
            };
            let nudged = message
                .and_then(|m| m.delivery.as_ref())
                .and_then(|d| d.nudged_ms)
                .map(|ms| format!(" {}", tv(cx, "workspace.resentAt", &[("time", &hhmm(ms))])))
                .unwrap_or_default();
            Body {
                title,
                text: format!("{quote}{nudged}").into(),
            }
        }
        "check" => check_body(card.check.as_ref(), cx),
        "dormant" => Body {
            title: t(cx, "workspace.dormantTitle"),
            text: tv(
                cx,
                "workspace.dormantText",
                &[
                    ("name", &who()),
                    ("time", &since),
                    ("duration", &duration(now.saturating_sub(card.since_ms))),
                ],
            ),
        },
        "blocked" => {
            let gone = card
                .agents
                .first()
                .and_then(|k| view.agent(k))
                .is_some_and(session_gone);
            Body {
                title: names.first().cloned().unwrap_or_default(),
                text: if gone {
                    format!("{}.", session_gone_notice(cx)).into()
                } else {
                    SharedString::default()
                },
            }
        }
        _ => Body {
            title: names.first().cloned().unwrap_or_default(),
            text: SharedString::default(),
        },
    }
}

/// A card about a hidden agent still shows, with a small "hidden".
pub fn card_hidden(view: &WorkspaceView, card: &Card) -> bool {
    card.agents
        .iter()
        .any(|k| view.agents.iter().any(|a| &a.key == k && a.hidden))
}

/// After "Mark all as read": what it cleared, and that Needs you does not change.
pub fn read_all_notice(before: &Unread, cx: &App) -> SharedString {
    let tail = t(cx, "workspace.readAllTail");
    if before.directs + before.channels == 0 {
        return tv(cx, "workspace.readAllNothing", &[("tail", &tail)]);
    }
    tv(
        cx,
        "workspace.readAllDone",
        &[
            ("directs", &before.directs.to_string()),
            ("channels", &before.channels.to_string()),
            ("tail", &tail),
        ],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_label_prefix_goes_when_it_matches_the_chip() {
        assert_eq!(without_label_prefix("DONE · Adrian · shipped it", Some("DONE")), "shipped it");
        assert_eq!(without_label_prefix("DONE v2 · what changed", Some("DONE")), "what changed");
        assert_eq!(without_label_prefix("DONE v2 · Adrian · what changed", Some("DONE")), "what changed");
        assert_eq!(
            without_label_prefix("BLOCKED · Adrian · stuck\nmore", Some("BLOCKED")),
            "stuck\nmore"
        );
        // A different label, an unknown head or nothing after the name: as written.
        assert_eq!(without_label_prefix("DONE · Adrian · x", Some("START")), "DONE · Adrian · x");
        assert_eq!(without_label_prefix("hello · there", Some("DONE")), "hello · there");
        assert_eq!(without_label_prefix("DONE · Adrian", Some("DONE")), "DONE · Adrian");
        assert_eq!(without_label_prefix("DONE · Adrian · x", None), "DONE · Adrian · x");
    }

    #[test]
    fn ellipsis_cuts_at_a_word() {
        assert_eq!(ellipsize_words("short", 40), "short");
        assert_eq!(ellipsize_words("one two three four", 10), "one two…");
        assert_eq!(ellipsize_words("abcdefghijklmnop", 6), "abcde…");
    }

    #[test]
    fn file_name_is_the_last_segment() {
        assert_eq!(file_name("/Users/x/repo"), "repo");
        assert_eq!(file_name("repo/"), "repo");
        assert_eq!(file_name(""), "");
    }

    #[test]
    fn hidden_card_names_a_hidden_agent() {
        let view = WorkspaceView {
            agents: vec![
                Agent { key: "a".into(), hidden: true, ..Default::default() },
                Agent { key: "b".into(), ..Default::default() },
            ],
            ..Default::default()
        };
        let card = |k: &str| Card { agents: vec![k.into()], ..Default::default() };
        assert!(card_hidden(&view, &card("a")));
        assert!(!card_hidden(&view, &card("b")));
    }

    #[test]
    fn a_session_is_gone_when_lost_or_its_lead_is_off_the_bus_or_ended() {
        let a = |lost: bool, delivery: &str| Agent { lost, delivery: delivery.into(), ..Default::default() };
        assert!(session_gone(&a(true, "ready")));
        assert!(session_gone(&a(false, "not_on_bus")));
        assert!(session_gone(&a(false, "ended")));
        assert!(!session_gone(&a(false, "stale")));
        assert!(!session_gone(&a(false, "ready")));
    }
}
