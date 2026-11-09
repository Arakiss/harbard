//! After-the-fact ledger: what happened in each checkout.
//!
//! The live bus answers "who owns what right now". This projection answers
//! "what happened here" — occupants, their epitaph when they left, and the
//! last marks (intent, warning, handoff, bye) plus the last sluice receipt.

use crate::clock;
use crate::event::{Event, EventKind};
use crate::query::{self, Liveness, SessionView};
use crate::sluice;
use crate::store::{Store, Window};
use serde::Serialize;

const MARK_CAP: usize = 10;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeliveryStamp {
    pub sha: String,
    pub verdict: String,
    pub decided_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Occupant {
    pub session: String,
    pub harness: String,
    pub liveness: Liveness,
    pub intent: Option<String>,
    pub epitaph: Option<String>,
    pub last_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CheckoutMark {
    pub id: String,
    pub ms: u64,
    pub event: String,
    pub kind: String,
    pub session: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CheckoutLedger {
    pub repo: String,
    pub label: String,
    pub last_ms: u64,
    pub live: u32,
    pub ended: u32,
    pub occupants: Vec<Occupant>,
    pub marks: Vec<CheckoutMark>,
    pub last_delivery: Option<DeliveryStamp>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CheckoutHistory {
    pub checkouts: Vec<CheckoutLedger>,
    pub now_ms: u64,
}

fn label_for(repo: &str) -> String {
    repo.rsplit(['/', '\\'])
        .find(|s| !s.is_empty())
        .unwrap_or(repo)
        .to_string()
}

fn truncate(text: &str, max: usize) -> String {
    let t = text.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let mut out = String::new();
    for (i, ch) in t.chars().enumerate() {
        if i >= max.saturating_sub(1) {
            break;
        }
        out.push(ch);
    }
    out.push('…');
    out
}

/// One line a person can read after a session has gone.
///
/// Live sessions have no epitaph — their intent is the current mission.
/// Ended / bye: declared intent plus the last note they left, or an honest
/// "left no note".
pub fn session_epitaph(session: &str, events: &[Event], view: &SessionView) -> Option<String> {
    if view.liveness != Liveness::Ended && !view.saw_bye {
        return None;
    }
    let intent = view
        .intent
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("no declared intent");
    let last_note = events
        .iter()
        .rev()
        .find(|e| e.session == session && e.event == EventKind::Note && !e.text.trim().is_empty());
    Some(match last_note {
        Some(n) => format!("{intent} · last: {}", truncate(&n.text, 72)),
        None => format!("{intent} · left no note"),
    })
}

fn mark_ms(event: &Event) -> u64 {
    ulid::Ulid::from_string(&event.id)
        .ok()
        .map(|u| u.timestamp_ms())
        .unwrap_or(0)
}

/// Build the checkout ledger from surviving segments.
pub fn build(store: &Store, now: u64, stale: u64) -> CheckoutHistory {
    let read = store.read(Window::All);
    let views = query::sessions(store, &read.events, now, stale);
    let mut by_repo: std::collections::BTreeMap<String, Vec<&SessionView>> =
        std::collections::BTreeMap::new();
    for view in &views {
        if view.harness == crate::HARNESS_HUMAN {
            continue;
        }
        let key = view
            .repo
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("(no checkout)")
            .to_string();
        by_repo.entry(key).or_default().push(view);
    }

    let mut checkouts = Vec::new();
    for (repo, occupants_in) in by_repo {
        let mut occupants: Vec<Occupant> = occupants_in
            .iter()
            .map(|view| Occupant {
                session: view.session.clone(),
                harness: view.harness.clone(),
                liveness: view.liveness,
                intent: view.intent.clone(),
                epitaph: session_epitaph(&view.session, &read.events, view),
                last_ms: view.last_ms,
            })
            .collect();
        occupants.sort_by_key(|o| std::cmp::Reverse(o.last_ms));
        let live = occupants
            .iter()
            .filter(|o| o.liveness == Liveness::Live)
            .count() as u32;
        let ended = occupants
            .iter()
            .filter(|o| o.liveness == Liveness::Ended)
            .count() as u32;

        let mut marks: Vec<CheckoutMark> = read
            .events
            .iter()
            .filter(|e| {
                let same = e
                    .repo
                    .as_deref()
                    .map(|r| r == repo)
                    .unwrap_or(repo == "(no checkout)");
                same && matches!(
                    e.event,
                    EventKind::Note | EventKind::Intent | EventKind::Bye | EventKind::Ack
                )
            })
            .map(|e| CheckoutMark {
                id: e.id.clone(),
                ms: mark_ms(e),
                event: e.event.as_str().to_string(),
                kind: if e.event == EventKind::Note {
                    e.kind.as_str().to_string()
                } else {
                    e.event.as_str().to_string()
                },
                session: e.session.clone(),
                text: truncate(&e.text, 96),
            })
            .collect();
        marks.sort_by_key(|m| std::cmp::Reverse(m.ms));
        marks.truncate(MARK_CAP);

        let last_ms = occupants
            .iter()
            .map(|o| o.last_ms)
            .chain(marks.iter().map(|m| m.ms))
            .max()
            .unwrap_or(0);

        let last_delivery = if repo == "(no checkout)" {
            None
        } else {
            sluice::receipts(store.root(), &repo)
                .into_iter()
                .max_by_key(|r| r.decided_ms)
                .map(|r| DeliveryStamp {
                    sha: r
                        .candidate
                        .clone()
                        .unwrap_or_else(|| r.delivered_sha.clone()),
                    verdict: r.verdict.as_str().to_string(),
                    decided_ms: r.decided_ms,
                })
        };

        checkouts.push(CheckoutLedger {
            label: label_for(&repo),
            repo,
            last_ms,
            live,
            ended,
            occupants,
            marks,
            last_delivery,
        });
    }
    checkouts.sort_by_key(|c| std::cmp::Reverse(c.last_ms));
    CheckoutHistory {
        checkouts,
        now_ms: now,
    }
}

/// Convenience for serve: now from the clock.
pub fn build_now(store: &Store, stale: u64) -> CheckoutHistory {
    build(store, clock::now_ms(), stale)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempHome;
    use crate::{NoteKind, Role, SCHEMA_VERSION, Store};

    fn ev(id_ms: u64, session: &str, kind: EventKind, repo: &str, text: &str) -> Event {
        let id = ulid::Ulid::from_datetime(
            std::time::UNIX_EPOCH + std::time::Duration::from_millis(id_ms),
        )
        .to_string();
        Event {
            v: SCHEMA_VERSION,
            id,
            ts: String::new(),
            event: kind,
            session: session.to_string(),
            harness: "codex".to_string(),
            host: "mac".to_string(),
            cwd: repo.to_string(),
            repo: Some(repo.to_string()),
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

    #[test]
    fn epitaph_names_intent_and_last_note() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let hello = ev(1_000, "flint", EventKind::Hello, "/work/harbard", "");
        let intent = ev(
            2_000,
            "flint",
            EventKind::Intent,
            "/work/harbard",
            "cut the Edge",
        );
        let note = ev(
            3_000,
            "flint",
            EventKind::Note,
            "/work/harbard",
            "shipped the horizon instrument",
        );
        let bye = ev(4_000, "flint", EventKind::Bye, "/work/harbard", "");
        store.append(&hello).unwrap();
        store.append(&intent).unwrap();
        store.append(&note).unwrap();
        store.append(&bye).unwrap();

        let hist = build(&store, 10_000, 900);
        assert_eq!(hist.checkouts.len(), 1);
        let card = &hist.checkouts[0];
        assert_eq!(card.label, "harbard");
        assert_eq!(card.ended, 1);
        let epitaph = card.occupants[0].epitaph.as_deref().unwrap();
        assert!(epitaph.contains("cut the Edge"), "got {epitaph}");
        assert!(epitaph.contains("horizon instrument"), "got {epitaph}");
    }

    #[test]
    fn live_session_has_no_epitaph() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        store
            .append(&ev(1_000, "lumen", EventKind::Hello, "/work/atlas", ""))
            .unwrap();
        store
            .append(&ev(
                2_000,
                "lumen",
                EventKind::Intent,
                "/work/atlas",
                "rotate keys",
            ))
            .unwrap();
        let hist = build(&store, 2_500, 900);
        let card = &hist.checkouts[0];
        assert_eq!(card.live, 1);
        assert_eq!(card.occupants[0].epitaph, None);
        assert_eq!(card.occupants[0].intent.as_deref(), Some("rotate keys"));
    }
}
