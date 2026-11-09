//! Reconstruct one note's delivery pipeline from the ops log.
//!
//! Shared by `harbard trace --note` and `GET /api/v1/notes/{id}/trace`. The
//! receipts on a cursor say how far a session got; this says WHY, and names
//! the stage that said no.

use crate::event::{Event, EventKind, Reader};
use crate::id;
use crate::membership;
use crate::oplog::{self, OpEvent};
use crate::query::{self, Liveness};
use crate::store::{Store, Window};
use serde::Serialize;

/// Milliseconds a ULID embeds — the append instant of any event id.
fn id_ms(id: &str) -> Option<u64> {
    ulid::Ulid::from_string(id).ok().map(|u| u.timestamp_ms())
}

fn age(now: u64, then: u64) -> String {
    let s = now.saturating_sub(then) / 1000;
    if s < 60 {
        format!("{s}s ago")
    } else if s < 3600 {
        format!("{}m ago", s / 60)
    } else if s < 86_400 {
        format!("{}h ago", s / 3600)
    } else {
        format!("{}d ago", s / 86_400)
    }
}

/// One delivery of one note to one session.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NoteDelivery {
    pub session: String,
    pub verb: String,
    pub ms: u64,
    pub envelope: String,
    /// First signal the session emitted after the injection, if any.
    pub continued_after_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PendingReader {
    pub session: String,
    pub harness: String,
    pub verdict: String,
}

/// Full per-note report: the pipeline with a stage per line.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NoteReport {
    pub note_id: String,
    /// The note's full body. A brief clips every quote to 140 bytes and `inbox`
    /// is a one-shot read, so this is the only way back to what was actually
    /// written once the fragment has scrolled past.
    pub text: String,
    pub kind: String,
    pub to: Option<String>,
    pub author: String,
    pub via: Option<String>,
    pub appended_ms: u64,
    pub deliveries: Vec<NoteDelivery>,
    pub pending: Vec<PendingReader>,
}

/// Every injection of `note_id` recorded in the ops log.
pub fn deliveries_for(ops: &[OpEvent], events: &[Event], note_id: &str) -> Vec<NoteDelivery> {
    let mut out: Vec<NoteDelivery> = Vec::new();
    for op in ops {
        if op.op != oplog::OP_INJECTED {
            continue;
        }
        let carries = op
            .delivered
            .as_ref()
            .is_some_and(|d| d.iter().any(|x| x == note_id));
        if !carries {
            continue;
        }
        let session = op.session.clone().unwrap_or_default();
        let continued = events
            .iter()
            .filter(|e| e.session == session)
            .filter_map(|e| id_ms(&e.id))
            .find(|ms| *ms > op.ms);
        out.push(NoteDelivery {
            session,
            verb: op.verb.clone().unwrap_or_default(),
            ms: op.ms,
            envelope: op.envelope.clone().unwrap_or_default(),
            continued_after_ms: continued,
        });
    }
    out.sort_by_key(|d| d.ms);
    out
}

/// The verdict for a session that should have a note but has no injection.
pub fn pending_verdict(
    note_kind: &str,
    hooks_after: usize,
    last_hook: Option<(&str, u64)>,
    held_quiet: bool,
    now: u64,
    liveness: Liveness,
) -> String {
    if liveness == Liveness::Ended {
        return format!(
            "this reader has ended — the note will never be picked up (last seen {})",
            match last_hook {
                Some((_, ms)) => age(now, ms),
                None => "never".to_string(),
            }
        );
    }
    let hook_state = match last_hook {
        Some((verb, ms)) => format!("hooks alive — last {verb} {}", age(now, ms)),
        None => "no hook activity ever recorded".to_string(),
    };
    if note_kind == "fyi" {
        return if held_quiet {
            format!(
                "sent quiet (fyi) — held on the hot path, rides their next turn boundary · {hook_state}"
            )
        } else {
            format!("sent quiet (fyi) — it waits for their next turn boundary · {hook_state}")
        };
    }
    if hooks_after == 0 {
        return format!(
            "no hook activity since you sent it — a long tool call, or hooks not wired · {hook_state}"
        );
    }
    format!("hooks alive but no interruption went out — the suspicious case · {hook_state}")
}

/// Build the report for one note against a store + already-read events/ops.
pub fn report(
    store: &Store,
    events: &[Event],
    ops: &[OpEvent],
    note: &Event,
    now: u64,
    stale: u64,
) -> NoteReport {
    let appended_ms = id_ms(&note.id).unwrap_or(0);
    let via = ops
        .iter()
        .find(|o| o.op == oplog::OP_NOTE_APPENDED && o.note.as_deref() == Some(note.id.as_str()))
        .and_then(|o| o.via.clone());
    let deliveries = deliveries_for(ops, events, &note.id);
    let delivered_sessions: Vec<&str> = deliveries.iter().map(|d| d.session.as_str()).collect();

    let memberships = membership::read(store);
    let views = query::sessions(store, events, now, stale);
    let placement = crate::workspaces::Placement::load(store);
    let mut pending = Vec::new();
    for view in &views {
        if view.session == note.session || delivered_sessions.contains(&view.session.as_str()) {
            continue;
        }
        let channels = membership::channels_for(
            &memberships,
            &view.session,
            &view.harness,
            view.repo.as_deref(),
        );
        let gate = placement.gate_of_session(view);
        let reader = Reader {
            session: &view.session,
            harness: &view.harness,
            repo: view.repo.as_deref(),
            repo_key: view.repo_key.as_deref().or(view.repo.as_deref()),
            channels: &channels,
            workspace: &gate,
        };
        if !note.is_addressed_to_reader(&reader) {
            continue;
        }
        let hooks_after = ops
            .iter()
            .filter(|o| {
                o.op == oplog::OP_HOOK
                    && o.session.as_deref() == Some(view.session.as_str())
                    && o.ms >= appended_ms
            })
            .count();
        let last_hook = ops
            .iter()
            .rfind(|o| {
                o.op == oplog::OP_HOOK && o.session.as_deref() == Some(view.session.as_str())
            })
            .map(|o| (o.verb.as_deref().unwrap_or("?"), o.ms));
        let held = ops.iter().any(|o| {
            o.op == oplog::OP_HELD_QUIET
                && o.session.as_deref() == Some(view.session.as_str())
                && o.ms >= appended_ms
        });
        pending.push(PendingReader {
            session: view.session.clone(),
            harness: view.harness.clone(),
            verdict: pending_verdict(
                note.kind.as_str(),
                hooks_after,
                last_hook,
                held,
                now,
                view.liveness,
            ),
        });
    }

    NoteReport {
        note_id: note.id.clone(),
        text: note.text.clone(),
        kind: note.kind.as_str().to_string(),
        to: note
            .to
            .as_ref()
            .map(|t| format!("{}:{}", t.kind.as_str(), t.value)),
        author: note.session.clone(),
        via,
        appended_ms,
        deliveries,
        pending,
    }
}

/// Resolve a note prefix and build its report, or name why not.
pub fn report_for_id(
    store: &Store,
    note_id: &str,
    now: u64,
    stale: u64,
) -> Result<NoteReport, id::ResolveIdError> {
    let events = store.read(Window::All);
    let note = id::event(&events.events, note_id, EventKind::Note)?;
    let ops = oplog::read(store, Window::All);
    Ok(report(store, &events.events, &ops.ops, note, now, stale))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempHome;
    use crate::{NoteKind, Role, SCHEMA_VERSION};

    fn ev(id: &str, session: &str, kind: NoteKind) -> Event {
        Event {
            v: SCHEMA_VERSION,
            id: id.to_string(),
            ts: String::new(),
            event: EventKind::Note,
            session: session.to_string(),
            harness: "codex".to_string(),
            host: "mac".to_string(),
            cwd: String::new(),
            repo: None,
            repo_key: None,
            branch: None,
            role: Role::Main,
            parent: None,
            note_ref: None,
            text: "body".to_string(),
            to: None,
            kind,
        }
    }

    #[test]
    fn deliveries_find_only_the_notes_injections() {
        let note = "01JNOTE1";
        let ops = vec![
            OpEvent::injected(100, "inline", vec![note.to_string()], "hook", "s1", "codex"),
            OpEvent::injected(
                200,
                "turn",
                vec!["01JOTHER".to_string()],
                "stdout",
                "s2",
                "codex",
            ),
        ];
        let later_id = ulid::Ulid::from_datetime(
            std::time::UNIX_EPOCH + std::time::Duration::from_millis(101_000),
        )
        .to_string();
        let mut cont = ev("01JCONT", "s1", NoteKind::Fyi);
        cont.event = EventKind::Intent;
        cont.id = later_id;
        let ds = deliveries_for(&ops, &[cont], note);
        assert_eq!(ds.len(), 1);
        assert_eq!(ds[0].session, "s1");
        assert_eq!(ds[0].continued_after_ms, Some(101_000));
    }

    #[test]
    fn quiet_notes_get_the_quiet_verdict() {
        let v = pending_verdict(
            "fyi",
            3,
            Some(("inline", 2_000)),
            true,
            10_000,
            Liveness::Live,
        );
        assert!(v.contains("sent quiet"));
        assert!(v.contains("hooks alive"));
    }

    #[test]
    fn interrupting_notes_without_hooks_get_the_silence_verdict() {
        let v = pending_verdict(
            "handoff",
            0,
            Some(("turn", 500)),
            false,
            10_000,
            Liveness::Live,
        );
        assert!(v.contains("no hook activity since you sent it"));
    }

    #[test]
    fn ended_reader_is_not_a_hook_failure() {
        let dead = pending_verdict(
            "handoff",
            0,
            Some(("turn", 500)),
            false,
            10_000,
            Liveness::Ended,
        );
        assert!(dead.contains("ended"), "got {dead}");
        assert!(!dead.contains("hooks"), "got {dead}");
    }

    #[test]
    fn report_names_who_got_it_and_who_still_owes() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let note_id = ulid::Ulid::from_datetime(
            std::time::UNIX_EPOCH + std::time::Duration::from_millis(1_000_000),
        )
        .to_string();
        let note = ev(&note_id, "operator@mac", NoteKind::Handoff);
        let mut s1_hello = ev(&note_id, "s1", NoteKind::Fyi);
        s1_hello.event = EventKind::Hello;
        s1_hello.id = ulid::Ulid::from_datetime(
            std::time::UNIX_EPOCH + std::time::Duration::from_millis(500_000),
        )
        .to_string();
        let mut s2_hello = s1_hello.clone();
        s2_hello.session = "s2".to_string();
        let events = vec![note.clone(), s1_hello, s2_hello];
        let ops = vec![
            OpEvent::note_appended(
                1_000_000,
                &note_id,
                "handoff",
                None,
                "serve",
                "operator@mac",
                "human",
            ),
            OpEvent::injected(
                1_100_000,
                "inline",
                vec![note_id.clone()],
                "hook",
                "s1",
                "codex",
            ),
            OpEvent::hook(1_050_000, "inline", "s2", "codex"),
        ];
        let built = report(&store, &events, &ops, &note, 2_000_000, 900);
        assert_eq!(built.deliveries.len(), 1);
        assert_eq!(built.pending.len(), 1);
        assert_eq!(built.pending[0].session, "s2");
    }
}
