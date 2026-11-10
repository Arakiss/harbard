//! The hub's bounded projection history.
//!
//! `/sessions`, `/snapshot` and the workspace read fold the hub's retained events on
//! every request. Retaining every segment on disk made that fold grow with the
//! age of the store, while the board only shows what is happening now. The hub
//! keeps a window of recent days, plus the few older events the projections
//! still read. The store itself keeps everything, and the history endpoints
//! read it directly.
//!
//! What a projection reads from before the window, and why it is carried:
//!
//! - Per session and harness: the first and latest event, the latest `hello`,
//!   `bye`, intent text, `parent` and herd role. `query::sessions` folds
//!   exactly these into identity, name, intent, lifecycle and liveness, and
//!   the first event is the session's join for `after_join`.
//! - Those anchors are carried for sessions that still matter: any session
//!   with an event inside the window, and any older session that is live now
//!   (a heartbeat or a bound supervisor can keep a quiet session live).
//! - Notes that owe an answer and could still be pending: the sender matters,
//!   or a session that matters joined before the note and the sender has not
//!   said `bye`. Every ack or reply that closes a carried note comes with it,
//!   and so do the anchors of their authors, so their rows fold as before.
//!
//! Leases, dependencies, membership and aliases are not folded from events in
//! the hub, so the window does not touch them.

use std::collections::{HashMap, HashSet};

use harbard_core::query::{self, Liveness};
use harbard_core::{Event, EventKind, Role, Store};

/// The shortest window the hub accepts. An operator debt from a sender that is
/// gone still stands for a week, so a shorter window would drop debts the
/// board must still show.
pub const MIN_DAYS: u64 = 7;

const DAY_MS: u64 = 24 * 60 * 60 * 1000;

const _: () = assert!(MIN_DAYS * DAY_MS >= harbard_core::fleet::ORPHANED_OPERATOR_NOTE_MAX_AGE_MS);

/// How often a running hub moves its horizon. An hour of extra history is
/// noise against the window, and the trim stays off the per-tick path.
pub const TRIM_EVERY_MS: u64 = 60 * 60 * 1000;

/// The window a hub retains, resolved once at startup.
#[derive(Debug, Clone, Copy)]
pub struct HubWindow {
    pub span_ms: u64,
    pub stale_secs: u64,
}

impl HubWindow {
    pub fn new(days: u64, stale_secs: u64) -> Self {
        HubWindow {
            span_ms: days.max(MIN_DAYS).saturating_mul(DAY_MS),
            stale_secs,
        }
    }
}

/// The events of one session under one harness that `query::sessions` reads.
#[derive(Debug, Default)]
struct Anchors {
    first: Option<usize>,
    latest: Option<usize>,
    hello: Option<usize>,
    bye: Option<usize>,
    intent: Option<usize>,
    parent: Option<usize>,
    herd: Option<usize>,
}

impl Anchors {
    fn observe(&mut self, index: usize, event: &Event) {
        self.first.get_or_insert(index);
        self.latest = Some(index);
        match event.event {
            EventKind::Hello => self.hello = Some(index),
            EventKind::Bye => self.bye = Some(index),
            _ => {}
        }
        if matches!(event.event, EventKind::Intent | EventKind::Hello) && !event.text.is_empty() {
            self.intent = Some(index);
        }
        if event.parent.is_some() {
            self.parent = Some(index);
        }
        if event.role == Role::Herd {
            self.herd = Some(index);
        }
    }

    fn indices(&self) -> impl Iterator<Item = usize> + '_ {
        [
            self.first,
            self.latest,
            self.hello,
            self.bye,
            self.intent,
            self.parent,
            self.herd,
        ]
        .into_iter()
        .flatten()
    }
}

/// Keep every event at or after `horizon_ms`, and from before it only what the
/// projections still read (see the module doc). File order is preserved, so a
/// carried event still precedes everything that came after it.
pub fn trim(
    store: &Store,
    events: &[Event],
    horizon_ms: u64,
    now_ms: u64,
    stale_secs: u64,
) -> Vec<Event> {
    let before = |event: &Event| query::event_ms(event) < horizon_ms;

    let mut in_window: HashSet<&str> = HashSet::new();
    let mut anchors: HashMap<(&str, &str), Anchors> = HashMap::new();
    let mut lifecycle: HashMap<&str, EventKind> = HashMap::new();
    for (index, event) in events.iter().enumerate() {
        if before(event) {
            anchors
                .entry((event.session.as_str(), event.harness.as_str()))
                .or_default()
                .observe(index, event);
            if matches!(event.event, EventKind::Hello | EventKind::Bye) {
                lifecycle.insert(event.session.as_str(), event.event);
            }
        } else {
            in_window.insert(event.session.as_str());
        }
    }

    let mut by_session: HashMap<&str, Vec<usize>> = HashMap::new();
    for ((session, _), anchor) in &anchors {
        by_session
            .entry(session)
            .or_default()
            .extend(anchor.indices());
    }

    // A session with nothing inside the window stays only while it is live.
    // Its anchors fold to the same liveness its whole history would.
    let mut quiet: Vec<usize> = by_session
        .iter()
        .filter(|(session, _)| !in_window.contains(*session))
        .flat_map(|(_, indices)| indices.iter().copied())
        .collect();
    quiet.sort_unstable();
    quiet.dedup();
    let quiet: Vec<Event> = quiet.into_iter().map(|i| events[i].clone()).collect();
    let live: HashSet<String> = query::sessions(store, &quiet, now_ms, stale_secs)
        .into_iter()
        .filter(|view| view.liveness == Liveness::Live)
        .map(|view| view.session)
        .collect();
    let matters = |session: &str| in_window.contains(session) || live.contains(session);

    // The earliest join among sessions that matter. A note before it reached
    // none of them, so it cannot be owed by anyone the board still shows.
    let earliest_join = by_session
        .iter()
        .filter(|(session, _)| matters(session))
        .filter_map(|(_, indices)| indices.iter().min().copied())
        .min();

    let mut keep = vec![false; events.len()];
    let mut carried_notes: HashSet<&str> = HashSet::new();
    let mut authors: HashSet<&str> = HashSet::new();
    for (index, event) in events.iter().enumerate() {
        if !before(event) {
            keep[index] = true;
            continue;
        }
        let closes = match event.event {
            EventKind::Ack => event.acknowledged_note_id(),
            EventKind::Note => event.note_ref.as_deref(),
            _ => None,
        }
        .is_some_and(|target| carried_notes.contains(target));
        // A sender that said `bye` stops its notes being current four hours
        // later, and a pre-window note is older than that.
        let open = event.event == EventKind::Note
            && event.kind.owes_an_answer()
            && (matters(&event.session)
                || (lifecycle.get(event.session.as_str()) != Some(&EventKind::Bye)
                    && earliest_join.is_some_and(|join| join < index)));
        if closes || open {
            keep[index] = true;
            if event.event == EventKind::Note {
                carried_notes.insert(event.id.as_str());
            }
            authors.insert(event.session.as_str());
        }
    }
    for (session, indices) in &by_session {
        if matters(session) || authors.contains(session) {
            for &index in indices {
                keep[index] = true;
            }
        }
    }

    events
        .iter()
        .zip(keep)
        .filter(|(_, keep)| *keep)
        .map(|(event, _)| event.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use harbard_core::{NoteKind, Recipient, RecipientKind, SCHEMA_VERSION, clock, cursor};
    use ulid::Ulid;

    const NOW: u64 = 2_000_000_000_000;
    const HORIZON: u64 = NOW - 14 * DAY_MS;

    fn ev(ms: u64, session: &str, kind: EventKind, text: &str) -> Event {
        Event {
            v: SCHEMA_VERSION,
            id: Ulid::from_datetime(std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms))
                .to_string(),
            ts: clock::rfc3339_utc(ms),
            event: kind,
            session: session.into(),
            harness: "codex".into(),
            host: "mac".into(),
            cwd: "/work/harbard".into(),
            repo: Some("/work/harbard".into()),
            repo_key: None,
            branch: Some("main".into()),
            role: Role::Main,
            parent: None,
            note_ref: None,
            text: text.into(),
            to: None,
            kind: NoteKind::Fyi,
        }
    }

    fn store() -> (Store, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("harbard-hub-window-{}", Ulid::generate()));
        std::fs::create_dir_all(&dir).unwrap();
        (Store::at(&dir), dir)
    }

    fn ids(events: &[Event]) -> HashSet<String> {
        events.iter().map(|event| event.id.clone()).collect()
    }

    #[test]
    fn a_live_session_older_than_the_window_keeps_its_identity() {
        let (store, dir) = store();
        let old = HORIZON - 5 * DAY_MS;
        let mut hello = ev(old, "veteran", EventKind::Hello, "");
        hello.parent = Some("lead".into());
        hello.role = Role::Herd;
        let intent = ev(
            old + 1_000,
            "veteran",
            EventKind::Intent,
            "migrate the store",
        );
        let chatter: Vec<Event> = (0..50)
            .map(|i| ev(old + 2_000 + i, "veteran", EventKind::Note, "progress"))
            .collect();
        let recent = ev(NOW - 60_000, "veteran", EventKind::Note, "still here");
        let mut events = vec![hello.clone(), intent.clone()];
        events.extend(chatter.iter().cloned());
        events.push(recent.clone());

        let kept = trim(&store, &events, HORIZON, NOW, 900);
        let kept_ids = ids(&kept);
        assert!(kept_ids.contains(&hello.id) && kept_ids.contains(&intent.id));
        assert!(kept_ids.contains(&recent.id));
        assert!(kept.len() < 6, "old chatter was retained: {}", kept.len());

        let full = query::sessions(&store, &events, NOW, 900);
        let windowed = query::sessions(&store, &kept, NOW, 900);
        assert_eq!(windowed.len(), 1);
        let (full, windowed) = (&full[0], &windowed[0]);
        assert_eq!(windowed.intent.as_deref(), Some("migrate the store"));
        assert_eq!(windowed.parent, full.parent);
        assert_eq!(windowed.role, full.role);
        assert_eq!(windowed.first_ms, full.first_ms);
        assert_eq!(windowed.last_ms, full.last_ms);
        assert_eq!(windowed.liveness, full.liveness);
        assert_eq!(windowed.identity, full.identity);
        assert_eq!(windowed.lifecycle, full.lifecycle);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_quiet_session_stays_while_live_and_leaves_when_it_is_not() {
        let (store, dir) = store();
        let old = HORIZON - DAY_MS;
        let events = vec![
            ev(old, "quiet-live", EventKind::Hello, "long run"),
            ev(old, "quiet-gone", EventKind::Hello, "crashed"),
            ev(old + 1, "ended", EventKind::Hello, ""),
            ev(old + 2, "ended", EventKind::Bye, ""),
        ];
        cursor::write_live_state(
            &store,
            "quiet-live",
            &cursor::LiveState {
                seen: None,
                checked_ms: NOW - 5_000,
                ..Default::default()
            },
        )
        .unwrap();

        let kept = trim(&store, &events, HORIZON, NOW, 900);
        let sessions: HashSet<&str> = kept.iter().map(|event| event.session.as_str()).collect();
        assert_eq!(sessions, HashSet::from(["quiet-live"]));
        let view = &query::sessions(&store, &kept, NOW, 900)[0];
        assert_eq!(view.liveness, Liveness::Live);
        assert_eq!(view.intent.as_deref(), Some("long run"));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_open_debt_and_what_closes_it_cross_the_horizon_together() {
        let (store, dir) = store();
        let old = HORIZON - 3 * DAY_MS;
        let reader = ev(old, "reader", EventKind::Hello, "");
        let mut ask = ev(old + 1_000, "asker", EventKind::Note, "which port?");
        ask.kind = NoteKind::Ask;
        ask.to = Some(Recipient {
            kind: RecipientKind::Session,
            value: "reader".into(),
        });
        let mut reply = ev(old + 2_000, "helper", EventKind::Note, "7419");
        reply.note_ref = Some(ask.id.clone());
        let helper_bye = ev(old + 3_000, "helper", EventKind::Bye, "");
        let mut before_join = ev(old - 1_000, "early", EventKind::Note, "too early");
        before_join.kind = NoteKind::Ask;
        let fyi = ev(old + 4_000, "asker", EventKind::Note, "just so you know");
        let recent = ev(NOW - 1_000, "reader", EventKind::Intent, "answering");
        let events = vec![
            before_join.clone(),
            reader.clone(),
            ask.clone(),
            reply.clone(),
            helper_bye.clone(),
            fyi.clone(),
            recent.clone(),
        ];

        let kept = ids(&trim(&store, &events, HORIZON, NOW, 900));
        assert!(
            kept.contains(&ask.id),
            "an open ask after the join is carried"
        );
        assert!(kept.contains(&reply.id), "its reply travels with it");
        assert!(
            kept.contains(&helper_bye.id),
            "the replier folds to the same row"
        );
        assert!(
            !kept.contains(&before_join.id),
            "nobody who matters had joined"
        );
        assert!(kept.contains(&reader.id) && kept.contains(&recent.id));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_window_never_drops_below_a_week() {
        assert_eq!(HubWindow::new(1, 900).span_ms, MIN_DAYS * DAY_MS);
        assert_eq!(HubWindow::new(30, 900).span_ms, 30 * DAY_MS);
    }
}
