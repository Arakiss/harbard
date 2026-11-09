//! Periodic protocol corrections on the brief/inline plane: an owed ack on
//! every prompt, and at SessionStart only a contested checkout and (once)
//! unhealthy hooks.
//!
//! Delivery cursors prove a handoff *reached* a reader without proving they
//! took it on. When an addressed handoff or ask sits without an answer, the
//! next brief carries a one-line harbard directive from [`crate::protocol`].
//!
//! The intent and name reminders (`intent_stale`, `name_required`) used to
//! ride here too. They were most of what the hooks ever said: 1,318 delivered
//! `intent_stale` lines against 8 `ack_pending`, and every one of them asked
//! for a bus call that changed nothing the session had to act on. A session
//! that never names itself already carries a derived callsign
//! ([`crate::alias::resolve`]), so neither reminder is emitted any more; the
//! codes stay in the v1 catalog.
//!
//! Not a new event kind and not a daemon: rides the hook plane that already
//! fires on every harness, throttled by stamps on the brief cursor.

use crate::event::{Event, EventKind, Reader, RecipientKind};
use crate::protocol::{LINE_ACK_PENDING, LINE_CONTESTED_CHECKOUT, LINE_HOOKS_UNHEALTHY};
use crate::query::SessionView;

/// How long an unanswered handoff nudge or a contested-checkout warning may
/// stand before harbard says it again.
pub const STATUS_INTERVAL_SECS: u64 = 20 * 60;

/// Ask the reader to close a handoff that already reached them.
pub const ACK_NUDGE: &str = LINE_ACK_PENDING;

/// Contested checkout — spawn placement / worktree directive.
pub const CHECKOUT_NUDGE: &str = LINE_CONTESTED_CHECKOUT;

/// Hooks or leases unhealthy — one-shot per session.
pub const DOCTOR_NUDGE: &str = LINE_HOOKS_UNHEALTHY;

/// True when we have not already asked within this interval.
pub fn nudge_due(last_nudge_ms: u64, now_ms: u64) -> bool {
    now_ms.saturating_sub(last_nudge_ms) >= STATUS_INTERVAL_SECS.saturating_mul(1000)
}

/// The notes this reader OWES an answer to, in the order they should be
/// answered: those addressed to this session by id first, then those that
/// reached it through its harness, repository or a channel it joined; oldest
/// first within each.
///
/// One definition serves `ack last`, the ack nudge, `stop-check` and `watch`:
/// a note is owed only if it would also have woken this reader
/// ([`crate::wake::calls_for`]). A broadcast ask is owed by nobody — counting
/// it would hold every session's Stop in the fleet for one remark addressed to
/// no one.
/// The same attention filter excludes pre-join, superseded and expired
/// ended-sender notes; retained history is not a new session's obligation.
///
/// Handoffs and asks both qualify. The nudge built on this is the antidote to
/// the failure the operator named: an agent whose attention is split does not
/// remember to answer, and until now nothing reminded it — a handoff was acked
/// 38.7% of the time, and a question sent to four agents at once could not even
/// be a handoff.
///
/// Closure is the same as the fleet's: an ack from this reader, or a note of
/// theirs whose `ref` names the obligation. Counting only acks kept a question
/// that had been answered in a reply listed as owed.
///
/// The order is what `ack last` closes. It used to close the newest note
/// addressed to the session, whatever its kind, so an fyi that landed after a
/// handoff took the ack and the handoff stayed open.
pub fn owed_notes<'a>(events: &'a [Event], reader: &Reader<'_>, now_ms: u64) -> Vec<&'a Event> {
    let mut ledger = crate::wake::Ledger::new(reader.session);
    for event in events {
        ledger.observe(event);
    }

    let mut owed: Vec<&Event> = events
        .iter()
        .filter(|ev| {
            ev.event == EventKind::Note
                && ev.kind.owes_an_answer()
                && ev.session != reader.session
                && crate::wake::calls_for(ev, reader)
                && ledger.actionable(ev, now_ms)
                && !ledger.answered(&ev.id)
        })
        .collect();
    // Stable: file order survives inside each tier.
    owed.sort_by_key(|ev| {
        !ev.to
            .as_ref()
            .is_some_and(|to| to.kind == RecipientKind::Session)
    });
    owed
}

/// True when this reader owes at least one answer ([`owed_notes`]).
pub fn has_unacked_handoff(events: &[Event], reader: &Reader<'_>, now_ms: u64) -> bool {
    !owed_notes(events, reader, now_ms).is_empty()
}

/// Decide whether this hook fire should carry [`ACK_NUDGE`].
pub fn should_ack_nudge(
    events: &[Event],
    reader: &Reader<'_>,
    last_nudge_ms: u64,
    now_ms: u64,
) -> bool {
    has_unacked_handoff(events, reader, now_ms) && nudge_due(last_nudge_ms, now_ms)
}

/// True when two or more distinct live *writer domains* share `me_repo`.
///
/// Parent+herd and inferred host-pid `other` ghosts do not inflate this
/// (see [`crate::topology`]). Pass active writer-lease holder session ids so a
/// ghost that actually claimed still counts.
pub fn checkout_contested(views: &[SessionView], me_session: &str, me_repo: Option<&str>) -> bool {
    checkout_contested_with_holders(
        views,
        me_session,
        me_repo,
        &std::collections::HashSet::new(),
    )
}

/// Contested checkout with known writer-lease holders (M5a/M5c).
pub fn checkout_contested_with_holders(
    views: &[SessionView],
    me_session: &str,
    me_repo: Option<&str>,
    lease_holders: &std::collections::HashSet<&str>,
) -> bool {
    crate::topology::checkout_contested(views, me_session, me_repo, lease_holders)
}

/// Contested checkout nudge — same throttle as status/ack.
pub fn should_checkout_nudge(
    views: &[SessionView],
    me_session: &str,
    me_repo: Option<&str>,
    last_nudge_ms: u64,
    now_ms: u64,
) -> bool {
    checkout_contested(views, me_session, me_repo) && nudge_due(last_nudge_ms, now_ms)
}

/// One-shot doctor nudge: fire at most once per session (`last_nudge_ms == 0`).
pub fn should_doctor_nudge(unhealthy: bool, last_nudge_ms: u64) -> bool {
    unhealthy && last_nudge_ms == 0
}

/// Append harbard-authored nudge lines under a digest, dropping lines that
/// would blow the budget (notes win; a dropped nudge retries next interval).
pub fn merge_lines(digest: Option<String>, lines: &[&str], max_bytes: usize) -> Option<String> {
    let mut out = digest;
    for line in lines {
        match &out {
            None => out = Some((*line).to_string()),
            Some(d) => {
                let combined = format!("{d}\n{line}");
                if combined.len() <= max_bytes {
                    out = Some(combined);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::NoteKind;
    use crate::event::{Recipient, RecipientKind, Role, SCHEMA_VERSION};
    use crate::query::Liveness;
    use ulid::Ulid;

    fn mk(id_ms: u64, kind: EventKind, session: &str, text: &str) -> Event {
        Event {
            v: SCHEMA_VERSION,
            id: Ulid::from_parts(id_ms, 0).to_string(),
            ts: crate::clock::rfc3339_utc(id_ms),
            event: kind,
            session: session.to_string(),
            harness: "codex".to_string(),
            host: "h".to_string(),
            cwd: "/tmp".to_string(),
            repo: Some("/repo".to_string()),
            repo_key: None,
            branch: Some("main".to_string()),
            role: Role::Main,
            parent: None,
            note_ref: None,
            text: text.to_string(),
            to: None,
            kind: NoteKind::Fyi,
        }
    }

    fn handoff(id_ms: u64, from: &str, to_session: &str) -> Event {
        let mut e = mk(id_ms, EventKind::Note, from, "take this");
        e.kind = NoteKind::Handoff;
        e.to = Some(Recipient {
            kind: RecipientKind::Session,
            value: to_session.to_string(),
        });
        e
    }

    fn attention_reader() -> Reader<'static> {
        Reader {
            workspace: &crate::workspaces::Gate::OPEN,
            session: "reader",
            harness: "codex",
            repo: Some("/repo"),
            repo_key: Some("/repo"),
            channels: &[],
        }
    }

    #[test]
    fn owed_notes_excludes_retained_history_and_unknown_joins() {
        let reader = attention_reader();
        let old = handoff(100, "author", "reader");
        assert!(owed_notes(std::slice::from_ref(&old), &reader, 1000).is_empty());
        let hello = mk(200, EventKind::Hello, "reader", "start");
        let after = handoff(300, "author", "reader");
        let events = [old, hello, after];
        let owed = owed_notes(&events, &reader, 1000);
        assert_eq!(owed.len(), 1);
        assert_eq!(owed[0].id, events[2].id);
    }

    #[test]
    fn owed_notes_uses_file_order_for_same_millisecond_session_join() {
        let ms = 100;
        let mut before = handoff(ms, "author", "reader");
        before.id = Ulid::from_parts(ms, 100).to_string();
        let mut hello = mk(ms, EventKind::Hello, "reader", "start");
        hello.id = Ulid::from_parts(ms, 50).to_string();
        let mut after = handoff(ms, "author", "reader");
        after.id = Ulid::from_parts(ms, 1).to_string();
        let events = [before, hello, after];
        let owed = owed_notes(&events, &attention_reader(), ms);
        assert_eq!(owed.len(), 1);
        assert_eq!(owed[0].id, events[2].id);
    }

    #[test]
    fn owed_notes_uses_ended_age_and_latest_sender_lifecycle() {
        let now = crate::wake::ENDED_NOTE_MAX_AGE_MS + 10_000;
        let reader = attention_reader();
        let mut events = vec![
            mk(1, EventKind::Hello, "reader", "start"),
            handoff(10_000, "author", "reader"),
            handoff(10_001, "author", "reader"),
            mk(now - 1, EventKind::Bye, "author", "done"),
        ];
        let owed = owed_notes(&events, &reader, now);
        assert_eq!(owed.len(), 1);
        assert_eq!(owed[0].id, events[2].id);
        events.push(mk(now, EventKind::Hello, "author", "resumed"));
        assert_eq!(owed_notes(&events, &reader, now).len(), 2);
    }

    #[test]
    fn quiet_cancellation_closes_root_and_sibling_without_hiding_other_work() {
        let reader = attention_reader();
        let root = handoff(10, "author", "reader");
        let mut sibling = handoff(20, "author", "reader");
        sibling.note_ref = Some(root.id.clone());
        let independent = handoff(30, "author", "reader");
        let other_sender = handoff(40, "other", "reader");
        let mut cancel = handoff(50, "author", "reader");
        cancel.kind = NoteKind::Fyi;
        cancel.note_ref = Some(root.id.clone());
        let events = [
            mk(1, EventKind::Hello, "reader", "start"),
            root,
            sibling,
            independent,
            other_sender,
            cancel,
        ];
        let owed = owed_notes(&events, &reader, 100);
        assert_eq!(owed.len(), 2);
        assert_eq!(owed[0].id, events[3].id);
        assert_eq!(owed[1].id, events[4].id);
    }

    #[test]
    fn owed_notes_preserves_direct_priority_and_reply_closure() {
        let reader = attention_reader();
        let mut repo_ask = handoff(10, "author", "reader");
        repo_ask.kind = NoteKind::Ask;
        repo_ask.to = Some(Recipient {
            kind: RecipientKind::Repo,
            value: "/repo".into(),
        });
        let direct = handoff(20, "author", "reader");
        let mut events = vec![mk(1, EventKind::Hello, "reader", "start"), repo_ask, direct];
        let owed = owed_notes(&events, &reader, 100);
        assert_eq!(owed.len(), 2);
        assert_eq!(owed[0].id, events[2].id);
        assert_eq!(owed[1].id, events[1].id);
        let mut reply = mk(30, EventKind::Note, "reader", "handled");
        reply.note_ref = Some(events[2].id.clone());
        events.push(reply);
        let owed = owed_notes(&events, &reader, 100);
        assert_eq!(owed.len(), 1);
        assert_eq!(owed[0].id, events[1].id);
    }

    #[test]
    fn nudge_throttled_within_interval() {
        let now = 1_000_000_000;
        assert!(!nudge_due(now - 60_000, now));
        assert!(nudge_due(now - STATUS_INTERVAL_SECS * 1000 - 1, now));
        assert!(nudge_due(0, now));
    }

    #[test]
    fn unacked_handoff_detected_until_ack() {
        let base = 1_000_000_000_000;
        let h = handoff(base, "author", "reader");
        let note_id = h.id.clone();
        let reader = Reader {
            workspace: &crate::workspaces::Gate::OPEN,
            session: "reader",
            harness: "codex",
            repo: Some("/repo"),
            repo_key: Some("/repo"),
            channels: &[],
        };
        let hello = mk(base - 1, EventKind::Hello, "reader", "start");
        assert!(has_unacked_handoff(
            &[hello.clone(), h.clone()],
            &reader,
            base + 1
        ));

        let mut ack = mk(base + 1, EventKind::Ack, "reader", &note_id);
        ack.to = Some(Recipient {
            kind: RecipientKind::Session,
            value: "author".into(),
        });
        assert!(!has_unacked_handoff(&[hello, h, ack], &reader, base + 1));
    }

    #[test]
    fn merge_lines_stacks_nudges() {
        let out = merge_lines(None, &[CHECKOUT_NUDGE, ACK_NUDGE], 2000).unwrap();
        assert!(out.contains("contested_checkout"));
        assert!(out.contains("ack_pending"));
    }

    #[test]
    fn contested_needs_two_writer_domains_same_repo() {
        let a = SessionView {
            session: "a".into(),
            harness: "cursor".into(),
            host: "h".into(),
            cwd: "/r".into(),
            repo: Some("/r".into()),
            repo_key: None,
            branch: None,
            role: Role::Main,
            parent: None,
            root: "a".into(),
            depth: 0,
            descendants: 0,
            lifecycle: crate::query::LifecycleState::Active,
            event_count: 1,
            intent: None,
            liveness: Liveness::Live,
            idle: false,
            last_signal_ms: 1,
            last_ms: 1,
            first_ms: 1,
            identity: crate::topology::IdentityConfidence::Observed,
            last_kind: EventKind::Intent,
            ending: false,
            saw_hello: true,
            saw_bye: false,
        };
        let mut b = a.clone();
        b.session = "b".into();
        assert!(checkout_contested(&[a.clone(), b.clone()], "a", Some("/r")));
        assert!(!checkout_contested(
            std::slice::from_ref(&a),
            "a",
            Some("/r")
        ));
        assert!(!checkout_contested(&[a.clone(), b], "a", None));

        // Parent + herd child = one domain, not contested.
        let mut child = a.clone();
        child.session = "child".into();
        child.role = Role::Herd;
        child.parent = Some("a".into());
        assert!(!checkout_contested(&[a, child], "a", Some("/r")));
    }

    #[test]
    fn doctor_nudge_is_one_shot() {
        assert!(should_doctor_nudge(true, 0));
        assert!(!should_doctor_nudge(true, 1));
        assert!(!should_doctor_nudge(false, 0));
    }
}
