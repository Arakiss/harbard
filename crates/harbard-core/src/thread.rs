//! Reading a conversation back (F5).
//!
//! `--ref` has been able to CREATE a thread since replies shipped, and the
//! console renders one, but nothing could read one back: `inbox` is a one-shot
//! cursor read, `search` needs a phrase you never saw, and `trace` answers about
//! a single note's delivery. So the operator could not copy a conversation out
//! of the UI, and an agent could not read the conversation it was replying into
//! — it answered a 140-byte fragment of the question and never saw its peers'
//! answers. Both wants are the same missing projection, and it lives here so the
//! CLI, the HTTP surface and the console all order a thread identically.
//!
//! Ordering is FILE ORDER, the store's own append order. ULID string order is
//! not append order — ids are generated independently by concurrent writers, so
//! two notes written in the same millisecond can sort against the log.

use crate::event::{Event, EventKind};

/// One conversation: the note that started it, and everything hanging off it.
#[derive(Debug, Clone)]
pub struct Thread {
    /// The note the conversation is about.
    pub root: Event,
    /// Replies and acks, in the order the store appended them.
    pub replies: Vec<Event>,
}

impl Thread {
    /// Root first, then replies — the order a reader expects.
    pub fn events(&self) -> impl Iterator<Item = &Event> {
        std::iter::once(&self.root).chain(self.replies.iter())
    }

    /// How many messages the conversation holds, the root included.
    pub fn len(&self) -> usize {
        1 + self.replies.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }
}

/// Walk up to the note that opened the conversation.
///
/// The schema documents one level of nesting, but a reply to a reply does carry
/// its parent's id, so this follows the chain rather than assuming depth. The
/// hop count is bounded: a `ref` cycle is not reachable through the CLI, and a
/// hand-edited log must not hang a read.
fn root_of<'a>(events: &'a [Event], start: &'a Event) -> &'a Event {
    let mut current = start;
    for _ in 0..16 {
        let Some(parent_id) = current.note_ref.as_deref() else {
            return current;
        };
        match events.iter().find(|e| e.id == parent_id) {
            Some(parent) if !std::ptr::eq(parent, current) => current = parent,
            // A ref to a note outside the window: the note we have IS the head
            // of what can be shown. Saying so beats returning nothing.
            _ => return current,
        }
    }
    current
}

/// Collect the conversation containing `note_id`, whichever member names it.
///
/// Answers `None` only when no event carries that id at all — replying to a
/// note that was garbage-collected still yields the thread its members form.
pub fn thread_for(events: &[Event], note_id: &str) -> Option<Thread> {
    let anchor = events.iter().find(|e| e.id == note_id)?;
    let root = root_of(events, anchor);

    // Transitive: children of the root, then children of those, so a
    // grandchild is never orphaned out of the conversation it belongs to.
    let mut ids = vec![root.id.clone()];
    let mut replies: Vec<Event> = Vec::new();
    let mut scanned = 0usize;
    while scanned < ids.len() {
        let parent = ids[scanned].clone();
        scanned += 1;
        for ev in events {
            if ev.id == root.id || ids.contains(&ev.id) {
                continue;
            }
            let is_child =
                ev.event != EventKind::Ack && ev.note_ref.as_deref() == Some(parent.as_str());
            let is_ack = ev.acknowledged_note_id() == Some(parent.as_str())
                && (ev.note_ref.is_some() || parent == root.id);
            if is_child || is_ack {
                ids.push(ev.id.clone());
                replies.push(ev.clone());
            }
        }
    }

    // File order, not discovery order: a breadth-first walk would interleave
    // branches against the log.
    let position = |id: &str| events.iter().position(|e| e.id == id).unwrap_or(usize::MAX);
    replies.sort_by_key(|e| position(&e.id));

    Some(Thread {
        root: root.clone(),
        replies,
    })
}

/// True when an ack's words already stand in `events` as a `--ref` reply from
/// the same session to the same note.
///
/// `ack --note` writes both since the answer became a message: the reply
/// carries the words and the ack keeps them only so older readers can still
/// tell which note it closed. A reader that printed both would say everything
/// twice, so it shows such an ack as the receipt it is.
pub fn ack_is_echoed(ack: &Event, events: &[Event]) -> bool {
    let Some(acked) = ack.acknowledged_note_id() else {
        return false;
    };
    let Some((_, words)) = ack.text.split_once(" — ") else {
        return false;
    };
    let words = words.trim();
    events.iter().any(|e| {
        e.event == EventKind::Note
            && e.session == ack.session
            && e.note_ref.as_deref() == Some(acked)
            && e.text.trim() == words
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{NoteKind, Recipient, RecipientKind, Role, SCHEMA_VERSION};

    fn note(id: &str, session: &str, text: &str, r: Option<&str>) -> Event {
        Event {
            v: SCHEMA_VERSION,
            id: id.to_string(),
            ts: "2026-08-31T10:00:00Z".to_string(),
            event: EventKind::Note,
            session: session.to_string(),
            harness: "codex".to_string(),
            host: "h".to_string(),
            cwd: "/tmp".to_string(),
            repo: None,
            repo_key: None,
            branch: None,
            role: Role::Main,
            parent: None,
            note_ref: r.map(str::to_string),
            text: text.to_string(),
            to: None,
            kind: NoteKind::Fyi,
        }
    }

    fn log() -> Vec<Event> {
        let mut root = note("R", "operator@h", "report your progress", None);
        root.kind = NoteKind::Warning;
        root.to = Some(Recipient {
            kind: RecipientKind::Harness,
            value: "codex".to_string(),
        });
        vec![
            note("A", "someone", "unrelated chatter", None),
            root,
            note("B", "soot", "tizne answers", Some("R")),
            note("C", "quicio", "quicio answers", Some("R")),
            note("D", "other", "unrelated again", None),
            note("E", "soot", "a follow-up to my own answer", Some("B")),
        ]
    }

    #[test]
    fn a_thread_reads_back_from_any_member() {
        // The operator copies from the note they can see, which is as likely to
        // be a reply as the question.
        for anchor in ["R", "B", "C", "E"] {
            let t = thread_for(&log(), anchor).expect("thread");
            assert_eq!(t.root.id, "R", "anchored at {anchor}");
            let ids: Vec<&str> = t.replies.iter().map(|e| e.id.as_str()).collect();
            assert_eq!(ids, vec!["B", "C", "E"], "anchored at {anchor}");
        }
    }

    #[test]
    fn a_thread_keeps_the_stores_own_order() {
        let t = thread_for(&log(), "R").unwrap();
        let ids: Vec<&str> = t.events().map(|e| e.id.as_str()).collect();
        // E replies to B, but it was appended last and reads last.
        assert_eq!(ids, vec!["R", "B", "C", "E"]);
        assert_eq!(t.len(), 4);
    }

    #[test]
    fn a_thread_excludes_everything_that_is_not_in_it() {
        let t = thread_for(&log(), "R").unwrap();
        assert!(
            !t.replies.iter().any(|e| e.id == "A" || e.id == "D"),
            "unrelated notes are not part of the conversation"
        );
    }

    #[test]
    fn an_ack_that_names_the_note_belongs_to_its_thread() {
        let id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        for (text, reference, included) in [
            (id.to_string(), None, true),
            (format!("{id} — taken on"), None, true),
            (format!("{id} taken on"), None, true),
            ("taken on".to_string(), Some(id), true),
            (format!("{id}extra — taken on"), None, false),
            (format!("{id}— taken on"), None, false),
            (format!("{id} — taken on"), Some("invalid"), false),
        ] {
            let mut ack = note("F", "reader", &text, reference);
            ack.event = EventKind::Ack;
            let events = vec![note(id, "author", "request", None), ack];
            let thread = thread_for(&events, id).unwrap();
            assert_eq!(thread.replies.len(), usize::from(included), "{text:?}");
        }
    }

    #[test]
    fn an_ack_whose_words_are_already_a_reply_is_only_a_receipt() {
        let id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        let mut ack = note("F", "reader", &format!("{id} — taken on"), Some(id));
        ack.event = EventKind::Ack;
        let reply = note("G", "reader", "taken on", Some(id));
        let other = note("H", "someone", "taken on", Some(id));
        assert!(ack_is_echoed(&ack, std::slice::from_ref(&reply)));
        assert!(!ack_is_echoed(&ack, &[other]), "another session's words");
        let mut bare = ack.clone();
        bare.text = id.to_string();
        assert!(!ack_is_echoed(&bare, &[reply]), "a bare ack says nothing");
    }

    #[test]
    fn a_reply_whose_parent_is_gone_is_still_a_thread() {
        // Retention is 14 days; a long conversation can outlive its own head.
        let events = vec![note("B", "soot", "answer", Some("GONE"))];
        let t = thread_for(&events, "B").expect("still readable");
        assert_eq!(t.root.id, "B");
        assert!(t.replies.is_empty());
    }

    #[test]
    fn an_unknown_id_is_a_refusal_not_an_empty_thread() {
        assert!(thread_for(&log(), "NOPE").is_none());
    }
}
