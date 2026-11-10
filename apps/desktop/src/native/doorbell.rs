//! What rings the operator's doorbell, and in what words: an ack to one of
//! their handoffs, a handoff, a warning to the fleet, or a note that names
//! them with `@`. fyi chatter stays in the app.
//!
//! The events come from the app's own read of the bus (the hub keeps them),
//! so nothing is polled: each read that brings new events hands them here.
//! The first read only places the cursor, so history never rings.

use harbard_core::event::{Event, EventKind, NoteKind};
use std::collections::HashMap;

/// Translate a `native.*` key with holes filled.
pub type Tr<'a> = &'a dyn Fn(&str, &[(&str, &str)]) -> String;

/// Display names by session id, as the board shows them.
#[derive(Default, Clone, Debug)]
pub struct Names(pub HashMap<String, String>);

impl Names {
    /// The view's sessions (`agents[]` keyed `session:<id>`, kind `session`).
    pub fn from_view(view: &crate::model::WorkspaceView) -> Names {
        Names(
            view.agents
                .iter()
                .filter(|a| a.kind == "session")
                .filter_map(|a| {
                    a.key
                        .strip_prefix("session:")
                        .map(|id| (id.to_string(), a.name.clone()))
                })
                .collect(),
        )
    }

    pub fn name_of(&self, session: &str) -> Option<&str> {
        self.0.get(session).map(String::as_str)
    }

    pub fn knows(&self, session: &str) -> bool {
        self.0.contains_key(session)
    }
}

/// Title case for a fleet name: `vidar` -> `Vidar`.
pub fn display(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// First line, capped: a notification is a doorbell, not the message.
pub fn snippet(text: &str, max: usize) -> String {
    let first = text.lines().next().unwrap_or("");
    if first.chars().count() > max {
        let cut: String = first.chars().take(max - 1).collect();
        format!("{cut}…")
    } else {
        first.to_string()
    }
}

/// A name the operator recognizes: the one the board shows, else the harness.
fn name_for(names: &Names, session: &str, harness: &str) -> String {
    names.name_of(session).map(display).unwrap_or_else(|| display(harness))
}

/// Whether `text` holds `@handle` as a whole word, without case.
pub fn mentions(text: &str, handle: &str) -> bool {
    let text = text.to_lowercase();
    let wanted = format!("@{handle}");
    text.match_indices(&wanted).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + wanted.len()..].chars().next();
        !before.is_some_and(|c| c.is_alphanumeric() || c == '.')
            && !after.is_some_and(|c| c.is_alphanumeric() || c == '-' || c == '_')
    })
}

/// Whether an event may ring at all (before names are looked up).
pub fn may_ring(ev: &Event) -> bool {
    ev.harness != harbard_core::event::HARNESS_HUMAN && matches!(ev.event, EventKind::Ack | EventKind::Note)
}

/// The notification for one event, if it rings: (title, body).
pub fn notification_for(ev: &Event, names: &Names, handle: &str, tr: Tr) -> Option<(String, String)> {
    if !may_ring(ev) {
        return None;
    }
    let name = name_for(names, &ev.session, &ev.harness);
    let title = |key: &str| tr(key, &[("name", name.as_str())]);
    match (&ev.event, &ev.kind) {
        (EventKind::Ack, _) => {
            let reply = ev.text.split_once(" — ").map(|(_, r)| r).unwrap_or("");
            Some((title("tookItOn"), snippet(reply, 90)))
        }
        (EventKind::Note, NoteKind::Handoff) => Some((title("handsOff"), snippet(&ev.text, 90))),
        (EventKind::Note, NoteKind::Warning) => Some((title("warnsFleet"), snippet(&ev.text, 90))),
        (EventKind::Note, _)
            if mentions(&ev.text, handle) || mentions(&ev.text, crate::native::rules::DEFAULT_OPERATOR_HANDLE) =>
        {
            Some((title("mentionedYou"), snippet(&ev.text, 90)))
        }
        _ => None,
    }
}

/// One toast per batch: the ring itself, or "{n} updates from the fleet"
/// with the first three titles and "+k more".
pub fn batch(rings: &[(String, String)], tr: Tr) -> Option<(String, String)> {
    match rings {
        [] => None,
        [one] => Some(one.clone()),
        many => {
            let n = many.len();
            let body = many
                .iter()
                .take(3)
                .map(|(t, _)| t.as_str())
                .collect::<Vec<_>>()
                .join(" · ");
            let body = if n > 3 {
                let more = tr("batchMore", &[("count", &(n - 3).to_string())]);
                format!("{body} · {more}")
            } else {
                body
            };
            Some((tr("batchTitle", &[("count", &n.to_string())]), body))
        }
    }
}

/// The new events since `cursor`, in log order, and the new cursor. The
/// first call (no cursor) returns nothing and places the cursor at the end.
pub fn since<'a>(events: &'a [Event], cursor: Option<&str>) -> (&'a [Event], Option<String>) {
    let last = events.last().map(|e| e.id.clone());
    let Some(cursor) = cursor else {
        return (&[], last);
    };
    // Ids are ULIDs: later ids sort after earlier ones. Scan back from the end.
    let start = events
        .iter()
        .rposition(|e| e.id.as_str() <= cursor)
        .map_or(0, |i| i + 1);
    (&events[start..], last.or_else(|| Some(cursor.to_string())))
}

/// `GET /api/v1/update`: the toast for an available update, once per
/// target commit: (commit, title, body).
pub fn update_notice(body: &serde_json::Value, tr: Tr) -> Option<(String, String, String)> {
    let check = body.get("check")?;
    if check.get("status")?.as_str()? != "available" {
        return None;
    }
    let commit = check.get("target")?.get("commit")?.as_str()?.to_string();
    let behind = check.get("behind").and_then(serde_json::Value::as_u64).unwrap_or(0);
    let what = match behind {
        0 | 1 => tr("updateOneChange", &[]),
        n => tr("updateManyChanges", &[("count", &n.to_string())]),
    };
    Some((commit, tr("updateTitle", &[]), tr("updateBody", &[("what", &what)])))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serde_json::json;

    /// The `native` dictionary of a language, read from the bundled file.
    pub fn dict(lang: &str) -> HashMap<String, String> {
        let raw = crate::assets::Assets::local(&format!("locales/{lang}/native.json")).expect("native.json");
        serde_json::from_slice(&raw).expect("native.json is flat")
    }

    pub fn tr_for(lang: &str) -> impl Fn(&str, &[(&str, &str)]) -> String {
        let d = dict(lang);
        let en = dict("en");
        move |key, vars| {
            let s = d
                .get(key)
                .or_else(|| en.get(key))
                .cloned()
                .unwrap_or_else(|| key.to_string());
            crate::i18n::fill(&s, vars)
        }
    }

    fn event(kind: &str, note: &str, session: &str, harness: &str, text: &str) -> Event {
        serde_json::from_value(json!({
            "v": 1, "id": "01JABCDEFGHJKMNPQRSTVWXYZ0", "ts": "2026-11-10T10:00:00Z",
            "event": kind, "session": session, "harness": harness, "host": "h", "cwd": "/",
            "text": text, "kind": note,
        }))
        .unwrap()
    }

    fn names() -> Names {
        Names(
            [("aurora-4f2c9a".to_string(), "aurora".to_string())]
                .into_iter()
                .collect(),
        )
    }

    #[test]
    fn both_languages_have_the_same_keys_and_holes() {
        use std::collections::BTreeSet;
        let holes = |text: &str| -> BTreeSet<String> {
            text.split('{')
                .skip(1)
                .filter_map(|r| r.split_once('}').map(|(n, _)| n.to_string()))
                .collect()
        };
        let (en, es) = (dict("en"), dict("es"));
        assert_eq!(en.keys().collect::<BTreeSet<_>>(), es.keys().collect::<BTreeSet<_>>());
        for (key, text) in &en {
            assert_eq!(holes(text), holes(&es[key]), "{key}");
        }
        assert_eq!(tr_for("es")("batchMore", &[("count", "3")]), "+3 más");
    }

    #[test]
    fn a_notification_names_the_session_the_way_the_board_does() {
        let tr = tr_for("en");
        let ev = event(
            "note",
            "handoff",
            "aurora-4f2c9a",
            "claude-code",
            "hold the merge\nmore",
        );
        let (title, body) = notification_for(&ev, &names(), "dana", &tr).expect("a handoff rings");
        assert_eq!(title, "Aurora hands off to you");
        assert_eq!(body, "hold the merge");
    }

    #[test]
    fn an_unknown_session_falls_back_to_its_harness() {
        let tr = tr_for("en");
        let ev = event("note", "warning", "ghost-01", "codex", "the branch is gone");
        let (title, _) = notification_for(&ev, &Names::default(), "dana", &tr).unwrap();
        assert_eq!(title, "Codex warns the fleet");
    }

    #[test]
    fn an_ack_rings_with_its_reply() {
        let tr = tr_for("es");
        let ev = event(
            "ack",
            "fyi",
            "aurora-4f2c9a",
            "codex",
            "01JABCDEFGHJKMNPQRSTVWXYZ0 — on it",
        );
        let (title, body) = notification_for(&ev, &names(), "dana", &tr).unwrap();
        assert_eq!(title, "Aurora se ha puesto con ello");
        assert_eq!(body, "on it");
    }

    #[test]
    fn a_mention_is_a_whole_word_without_case() {
        assert!(mentions("FINDING · @Dana continue?", "dana"));
        assert!(mentions("@dana, look", "dana"));
        assert!(!mentions("mail a x@dana.es", "dana"));
        assert!(!mentions("@danas lo hizo", "dana"));
        assert!(!mentions("@dana-bot lo hizo", "dana"));
        assert!(mentions("@operator continue?", "operator"));
        let tr = tr_for("en");
        let ev = event("note", "fyi", "aurora-4f2c9a", "codex", "@Dana look");
        assert_eq!(
            notification_for(&ev, &names(), "dana", &tr).unwrap().0,
            "Aurora mentioned you"
        );
    }

    #[test]
    fn the_operator_never_rings_themselves_and_fyi_stays_in_the_app() {
        let tr = tr_for("en");
        let own = event("note", "handoff", "operator@host", "human", "do it");
        assert!(notification_for(&own, &names(), "dana", &tr).is_none());
        let fyi = event("note", "fyi", "aurora-4f2c9a", "claude-code", "rebased");
        assert!(notification_for(&fyi, &names(), "dana", &tr).is_none());
    }

    #[test]
    fn several_rings_batch_into_one_toast() {
        let tr = tr_for("en");
        let ring = |n: usize| (format!("t{n}"), "b".to_string());
        assert_eq!(batch(&[], &tr), None);
        assert_eq!(batch(&[ring(1)], &tr), Some(ring(1)));
        let (title, body) = batch(&[ring(1), ring(2), ring(3), ring(4), ring(5)], &tr).unwrap();
        assert_eq!(title, "5 updates from the fleet");
        assert_eq!(body, "t1 · t2 · t3 · +2 more");
    }

    #[test]
    fn the_first_read_only_places_the_cursor() {
        let mut a = event("note", "fyi", "s", "codex", "x");
        let mut b = a.clone();
        a.id = "01AAAAAAAAAAAAAAAAAAAAAAAA".into();
        b.id = "01BBBBBBBBBBBBBBBBBBBBBBBB".into();
        let events = vec![a.clone(), b.clone()];
        let (new, cursor) = since(&events, None);
        assert!(new.is_empty());
        assert_eq!(cursor.as_deref(), Some(b.id.as_str()));
        let (new, _) = since(&events, Some(&a.id));
        assert_eq!(new.len(), 1);
        assert_eq!(new[0].id, b.id);
        let (new, cursor) = since(&events, Some(&b.id));
        assert!(new.is_empty());
        assert_eq!(cursor.as_deref(), Some(b.id.as_str()));
    }

    #[test]
    fn an_available_update_counts_its_changes() {
        let body = json!({
            "installed": { "version": "0.3.0-alpha.9" },
            "check": { "status": "available", "behind": 11, "target": { "commit": "abc", "version": "x" } },
        });
        let (commit, title, text) = update_notice(&body, &tr_for("en")).unwrap();
        assert_eq!(commit, "abc");
        assert_eq!(title, "A new version of harbard is available");
        assert!(text.starts_with("There are 11 new changes."), "{text}");
        let (_, title, text) = update_notice(&body, &tr_for("es")).unwrap();
        assert_eq!(title, "Hay una versión nueva de harbard");
        assert!(text.starts_with("Hay 11 cambios nuevos."), "{text}");
        assert!(update_notice(&json!({ "check": { "status": "current" } }), &tr_for("en")).is_none());
        assert!(update_notice(&json!({ "check": null }), &tr_for("en")).is_none());
    }
}
