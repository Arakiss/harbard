//! Activity's lists (`lib/workspace.ts`: `needsItems`, `todoItems`,
//! `threads`, `activitySelection`, `readMarkOf`, `nudgeShown`, `thread`).

use crate::model::{Card, Message, WorkspaceView};
use std::collections::{HashMap, HashSet};

/// A row of Activity's lists: a card, or a message.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Entry<'a> {
    Card(&'a Card),
    Message(&'a Message),
}

/// The entry the detail showed last, kept across reads (a message being
/// read stays open when it leaves the list).
#[derive(Clone, Debug, PartialEq)]
pub enum Shown {
    Card(Box<Card>),
    Message(Box<Message>),
}

impl Shown {
    pub fn of(e: Entry<'_>) -> Shown {
        match e {
            Entry::Card(c) => Shown::Card(Box::new(c.clone())),
            Entry::Message(m) => Shown::Message(Box::new(m.clone())),
        }
    }

    pub fn id(&self) -> String {
        match self {
            Shown::Card(c) => format!("card:{}", c.id),
            Shown::Message(m) => format!("msg:{}", m.id),
        }
    }

    pub fn entry(&self) -> Entry<'_> {
        match self {
            Shown::Card(c) => Entry::Card(c),
            Shown::Message(m) => Entry::Message(m),
        }
    }
}

impl Entry<'_> {
    /// `card:<id>` or `msg:<id>`.
    pub fn id(&self) -> String {
        match self {
            Entry::Card(c) => format!("card:{}", c.id),
            Entry::Message(m) => format!("msg:{}", m.id),
        }
    }

    fn at(&self) -> u64 {
        match self {
            Entry::Card(c) => c.since_ms,
            Entry::Message(m) => m.ts_ms,
        }
    }
}

/// Most urgent first, the one order Needs you and the calm card share.
pub fn urgency(kind: &str) -> u8 {
    match kind {
        "blocked" => 0,
        "check" | "approval" => 1,
        "handoff" => 2,
        "warning" => 3,
        "mention" => 4,
        "ask" => 5,
        "unanswered" => 6,
        "two_writers" => 7,
        "dormant" => 8,
        _ => 9,
    }
}

/// Needs you: open cards by urgency, the newest first within a kind.
pub fn needs_items(view: &WorkspaceView) -> Vec<&Card> {
    let mut out: Vec<&Card> = view
        .cards
        .iter()
        .filter(|c| c.needs && c.resolved_ms.is_none())
        .collect();
    out.sort_by(|a, b| {
        urgency(&a.kind)
            .cmp(&urgency(&b.kind))
            .then(b.since_ms.cmp(&a.since_ms))
    });
    out
}

/// All, newest first: every card, open or not, and every message of an
/// agent, the operator or the bus that no card already shows.
pub fn todo_items(view: &WorkspaceView, max: usize) -> Vec<Entry<'_>> {
    let carded: HashSet<&str> = view
        .cards
        .iter()
        .filter_map(|c| c.message.as_deref())
        .collect();
    let mut out: Vec<Entry<'_>> = view.cards.iter().map(Entry::Card).collect();
    out.extend(
        view.messages
            .iter()
            .filter(|m| matches!(m.from.as_str(), "agent" | "operator" | "session"))
            .filter(|m| m.structured.is_none() && !carded.contains(m.id.as_str()))
            .map(Entry::Message),
    );
    // A stable sort, so equal times keep cards before messages as in the console.
    out.sort_by_key(|e| std::cmp::Reverse(e.at()));
    out.truncate(max);
    out
}

/// Messages that have a thread, the one answered last first.
pub fn threads(view: &WorkspaceView) -> Vec<&Message> {
    let at: HashMap<&str, u64> = view
        .messages
        .iter()
        .map(|m| (m.id.as_str(), m.ts_ms))
        .collect();
    let last = |m: &Message| {
        m.replies
            .iter()
            .map(|r| at.get(r.as_str()).copied().unwrap_or(0))
            .fold(m.ts_ms, u64::max)
    };
    let mut out: Vec<(&Message, u64)> = view
        .messages
        .iter()
        .filter(|m| !m.replies.is_empty())
        .map(|m| (m, last(m)))
        .collect();
    out.sort_by_key(|e| std::cmp::Reverse(e.1));
    out.into_iter().map(|(m, _)| m).collect()
}

/// Replies of a message, in order.
pub fn thread<'a>(view: &'a WorkspaceView, id: &str) -> Vec<&'a Message> {
    let Some(parent) = view.message(id) else {
        return Vec::new();
    };
    let mut out: Vec<&Message> = parent
        .replies
        .iter()
        .filter_map(|r| view.message(r))
        .collect();
    out.sort_by_key(|m| m.ts_ms);
    out
}

/// The entries of a tab.
pub fn entries<'a>(view: &'a WorkspaceView, tab: &str) -> Vec<Entry<'a>> {
    match tab {
        "todo" => todo_items(view, 200),
        "threads" => threads(view).into_iter().map(Entry::Message).collect(),
        _ => needs_items(view).into_iter().map(Entry::Card).collect(),
    }
}

/// What Activity opens. The address wins; else the entry shown last, read
/// fresh from the list (a message stays open when it left it); else the one
/// now at the old position.
pub fn activity_selection<'a>(
    list: &[Entry<'a>],
    picked: Option<&str>,
    shown: Option<&'a Shown>,
    position: usize,
) -> Option<Entry<'a>> {
    let listed = |id: &str| list.iter().copied().find(|e| e.id() == id);
    if let Some(e) = picked.and_then(listed) {
        return Some(e);
    }
    if let Some(shown) = shown {
        let sid = shown.id();
        if picked.is_none() || picked == Some(sid.as_str()) {
            if let Some(fresh) = listed(&sid) {
                return Some(fresh);
            }
            if matches!(shown, Shown::Message(_)) {
                return Some(shown.entry());
            }
        }
    }
    if list.is_empty() {
        return None;
    }
    list.get(position.min(list.len() - 1)).copied()
}

/// What Esc marks read for the open item: its agent, up to the item's time.
pub fn read_mark_of(view: &WorkspaceView, entry: Entry<'_>) -> Option<(String, u64)> {
    match entry {
        Entry::Message(m) => Some((m.agent.clone(), m.ts_ms)),
        Entry::Card(c) => {
            let m = c.message.as_deref().and_then(|id| view.message(id));
            let agent = m
                .map(|m| m.agent.clone())
                .or_else(|| c.agents.first().cloned())?;
            view.agent(&agent)?;
            Some((agent, m.map_or(c.since_ms, |m| m.ts_ms)))
        }
    }
}

/// How long after a re-send "Resend" stays off: serve's own window.
pub const NUDGE_WINDOW_MS: u64 = 10 * 60_000;

pub fn nudge_shown(nudged_ms: Option<u64>, now: u64) -> bool {
    nudged_ms.is_some_and(|ms| now.saturating_sub(ms) < NUDGE_WINDOW_MS)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str, kind: &str, since: u64) -> Card {
        Card {
            id: id.into(),
            kind: kind.into(),
            since_ms: since,
            needs: true,
            ..Default::default()
        }
    }

    fn msg(id: &str, from: &str, ts: u64) -> Message {
        Message {
            id: id.into(),
            from: from.into(),
            ts_ms: ts,
            agent: "a1".into(),
            ..Default::default()
        }
    }

    #[test]
    fn needs_you_puts_an_old_block_before_a_fresh_dormant_agent() {
        let view = WorkspaceView {
            cards: vec![
                card("d", "dormant", 9_000),
                card("b", "blocked", 1_000),
                card("m", "mention", 5_000),
                {
                    let mut c = card("r", "blocked", 8_000);
                    c.resolved_ms = Some(8_500);
                    c
                },
            ],
            ..Default::default()
        };
        let ids: Vec<&str> = needs_items(&view).iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["b", "m", "d"]);
    }

    #[test]
    fn all_shows_an_open_block_once_as_its_card_and_never_a_lead_word() {
        let mut blocked = card("c1", "blocked", 2_000);
        blocked.message = Some("m1".into());
        let mut structured = msg("m4", "agent", 4_000);
        structured.structured = Some("x".into());
        let view = WorkspaceView {
            cards: vec![blocked],
            messages: vec![
                msg("m1", "agent", 2_000),
                msg("m2", "operator", 3_000),
                msg("m3", "lead", 5_000),
                structured,
                msg("m5", "session", 1_000),
            ],
            ..Default::default()
        };
        let ids: Vec<String> = todo_items(&view, 200).iter().map(Entry::id).collect();
        assert_eq!(ids, ["msg:m2", "card:c1", "msg:m5"]);
    }

    #[test]
    fn threads_sort_by_the_last_reply() {
        let mut a = msg("a", "agent", 1_000);
        a.replies = vec!["r1".into()];
        let mut b = msg("b", "agent", 2_000);
        b.replies = vec!["r2".into()];
        let view = WorkspaceView {
            messages: vec![
                a,
                b,
                msg("r1", "operator", 9_000),
                msg("r2", "operator", 3_000),
            ],
            ..Default::default()
        };
        let ids: Vec<&str> = threads(&view).iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["a", "b"]);
    }

    #[test]
    fn a_resolved_card_leaves_and_the_next_takes_its_place() {
        let cards = [
            card("1", "blocked", 1),
            card("2", "blocked", 2),
            card("3", "blocked", 3),
        ];
        let list: Vec<Entry> = cards.iter().map(Entry::Card).collect();
        let gone = Shown::Card(Box::new(card("x", "blocked", 0)));
        // "x" was resolved at position 1: the one now at 1 opens.
        assert_eq!(
            activity_selection(&list, None, Some(&gone), 1).map(|e| e.id()),
            Some("card:2".into())
        );
        // The last one resolved: the one above.
        assert_eq!(
            activity_selection(&list, None, Some(&gone), 7).map(|e| e.id()),
            Some("card:3".into())
        );
        assert_eq!(activity_selection(&[], None, Some(&gone), 0), None);
    }

    #[test]
    fn a_message_being_read_stays_open_and_the_address_wins() {
        let cards = [card("1", "blocked", 1)];
        let list: Vec<Entry> = cards.iter().map(Entry::Card).collect();
        let reading = Shown::Message(Box::new(msg("m", "agent", 5)));
        assert_eq!(
            activity_selection(&list, None, Some(&reading), 0).map(|e| e.id()),
            Some("msg:m".into())
        );
        assert_eq!(
            activity_selection(&list, Some("card:1"), Some(&reading), 0).map(|e| e.id()),
            Some("card:1".into())
        );
    }

    #[test]
    fn esc_marks_the_open_item_read_up_to_its_own_time() {
        let mut c = card("c", "blocked", 7);
        c.agents = vec!["a1".into()];
        let view = WorkspaceView {
            agents: vec![crate::model::Agent {
                key: "a1".into(),
                ..Default::default()
            }],
            messages: vec![msg("m", "agent", 42)],
            ..Default::default()
        };
        assert_eq!(
            read_mark_of(&view, Entry::Message(&view.messages[0])),
            Some(("a1".into(), 42))
        );
        assert_eq!(read_mark_of(&view, Entry::Card(&c)), Some(("a1".into(), 7)));
        c.agents = vec!["gone".into()];
        assert_eq!(read_mark_of(&view, Entry::Card(&c)), None);
    }

    #[test]
    fn resend_stays_off_for_ten_minutes() {
        assert!(nudge_shown(Some(1_000), 1_000 + NUDGE_WINDOW_MS - 1));
        assert!(!nudge_shown(Some(1_000), 1_000 + NUDGE_WINDOW_MS));
        assert!(!nudge_shown(None, 5));
    }
}
