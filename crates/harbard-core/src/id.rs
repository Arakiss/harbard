//! Exact-or-unique identifier resolution for every human-facing bus door.
//!
//! Renderers may shorten identifiers for display, but no caller may guess the
//! hidden suffix or silently choose the first collision.

use crate::{Event, EventKind, SessionView};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolveIdError {
    Unknown,
    Ambiguous(usize),
}

pub fn is_event_prefix(input: &str) -> bool {
    !input.is_empty()
        && input.len() <= 26
        && input.bytes().all(|byte| {
            matches!(
                byte,
                b'0'..=b'9'
                    | b'A'..=b'H'
                    | b'J'
                    | b'K'
                    | b'M'
                    | b'N'
                    | b'P'..=b'T'
                    | b'V'..=b'Z'
            )
        })
}

pub fn event<'a>(
    events: &'a [Event],
    input: &str,
    kind: EventKind,
) -> Result<&'a Event, ResolveIdError> {
    let mut matches = events
        .iter()
        .filter(|event| event.event == kind && event.id.starts_with(input));
    let Some(first) = matches.next() else {
        return Err(ResolveIdError::Unknown);
    };
    let count = 1 + matches.count();
    if count == 1 {
        Ok(first)
    } else {
        Err(ResolveIdError::Ambiguous(count))
    }
}

pub fn session<'a>(
    sessions: &'a [SessionView],
    input: &str,
) -> Result<&'a SessionView, ResolveIdError> {
    let mut matches = sessions
        .iter()
        .filter(|session| session.session.starts_with(input));
    let Some(first) = matches.next() else {
        return Err(ResolveIdError::Unknown);
    };
    let count = 1 + matches.count();
    if count == 1 {
        Ok(first)
    } else {
        Err(ResolveIdError::Ambiguous(count))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NoteKind, Role, SCHEMA_VERSION};

    fn note(id: &str) -> Event {
        Event {
            v: SCHEMA_VERSION,
            id: id.into(),
            ts: "2026-08-06T00:00:00Z".into(),
            event: EventKind::Note,
            session: "writer".into(),
            harness: "codex".into(),
            host: "host".into(),
            cwd: "/repo".into(),
            repo: None,
            repo_key: None,
            branch: None,
            role: Role::Main,
            parent: None,
            note_ref: None,
            text: "note".into(),
            to: None,
            kind: NoteKind::Fyi,
        }
    }

    #[test]
    fn event_prefix_must_be_unique() {
        let events = vec![
            note("01KZA0BW38AAAAAAAAAAAAAAAA"),
            note("01KZA0BW3NBBBBBBBBBBBBBBBB"),
        ];
        assert_eq!(
            event(&events, "01KZA0BW", EventKind::Note),
            Err(ResolveIdError::Ambiguous(2))
        );
        assert_eq!(
            event(&events, "01KZA0BW38", EventKind::Note)
                .expect("unique")
                .id,
            events[0].id
        );
        assert_eq!(
            event(&events, "01NOPE", EventKind::Note),
            Err(ResolveIdError::Unknown)
        );
    }
}
