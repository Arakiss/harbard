//! Explicit dependencies, reconstructed from the append-only event log.
//!
//! A satisfied result is the responder's claim, not verified completion.
//! Consuming a result records the requester's observation, not execution.
//! Deadlines are metadata: a caller timing out never resolves a dependency.

use crate::{Event, EventKind, NoteKind, Recipient, RecipientKind, Store, Window};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::sync::Mutex;

// Match the lease store's process guard plus cross-process file lock pattern.
static PROCESS_LOCK: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyOutcome {
    Satisfied,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DependencyState {
    Pending,
    Satisfied,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum DependencyAction {
    Declare {
        responder: String,
        condition: String,
        continuation: String,
        deadline_ms: Option<u64>,
    },
    Resolve {
        id: String,
        outcome: DependencyOutcome,
        evidence: String,
    },
    Cancel {
        id: String,
        reason: String,
    },
    Consume {
        id: String,
    },
}

impl DependencyAction {
    fn id<'a>(&'a self, event: &'a Event) -> &'a str {
        match self {
            Self::Declare { .. } => &event.id,
            Self::Resolve { id, .. } | Self::Cancel { id, .. } | Self::Consume { id } => id,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DependencyView {
    pub id: String,
    pub requester_session: String,
    pub responder_session: String,
    pub condition: String,
    pub continuation: String,
    pub deadline_ms: Option<u64>,
    pub state: DependencyState,
    pub resolution_event_id: Option<String>,
    /// The responder's evidence, or the requester's cancellation reason.
    pub evidence: Option<String>,
    pub consumed: bool,
}

#[derive(Debug)]
pub enum DependencyError {
    Io(std::io::Error),
    Invalid(String),
    Unknown(String),
    Unauthorized(String),
    Conflict(String),
    CorruptHistory {
        corrupt_lines: u64,
        unknown_events: u64,
    },
}

impl fmt::Display for DependencyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "dependency store: {error}"),
            Self::Invalid(message) => write!(f, "invalid dependency: {message}"),
            Self::Unknown(id) => write!(f, "unknown dependency: {id}"),
            Self::Unauthorized(message) => write!(f, "dependency participant mismatch: {message}"),
            Self::Conflict(message) => write!(f, "dependency conflict: {message}"),
            Self::CorruptHistory {
                corrupt_lines,
                unknown_events,
            } => write!(
                f,
                "dependency history is incomplete: {corrupt_lines} corrupt lines, {unknown_events} unknown events"
            ),
        }
    }
}

impl std::error::Error for DependencyError {}

impl From<std::io::Error> for DependencyError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

/// A regenerable read model. Start with a complete snapshot, then apply new
/// events in append order. Never use the bounded recent window as a snapshot.
#[derive(Debug, Clone, Default)]
pub struct DependencyProjection {
    views: BTreeMap<String, DependencyView>,
    seen: BTreeMap<String, Event>,
}

impl DependencyProjection {
    pub fn from_events(events: &[Event]) -> Result<Self, DependencyError> {
        let mut projection = Self::default();
        for event in events {
            projection.apply(event)?;
        }
        Ok(projection)
    }

    /// Reject a skipped/corrupt input rather than treating an absent result as
    /// an unresolved wait. Use this with `Store::read_since` in a watcher.
    pub fn apply_read(&mut self, read: &crate::ReadResult) -> Result<(), DependencyError> {
        check_read(read)?;
        if read.events.is_empty() {
            return Ok(());
        }
        let mut next = self.clone();
        for event in &read.events {
            next.apply(event)?;
        }
        *self = next;
        Ok(())
    }

    /// Apply a canonical event. Notes and acknowledgements never change state.
    /// Replaying the exact same event is harmless; reusing its ID is not.
    pub fn apply(&mut self, event: &Event) -> Result<(), DependencyError> {
        if event.event == EventKind::Unknown {
            return Err(DependencyError::Invalid("unknown event kind".into()));
        }
        if event.event != EventKind::Dependency {
            return Ok(());
        }
        if let Some(previous) = self.seen.get(&event.id) {
            return if previous == event {
                Ok(())
            } else {
                Err(DependencyError::Conflict(format!(
                    "event ID {} was reused",
                    event.id
                )))
            };
        }
        if event.v != crate::SCHEMA_VERSION || !crate::is_event_id(&event.id) {
            return Err(DependencyError::Invalid(
                "event schema or ID is invalid".into(),
            ));
        }
        nonempty(&event.session, "session")?;
        if event.kind != NoteKind::Fyi || event.note_ref.is_some() {
            return Err(DependencyError::Invalid(
                "dependency is not a note or reply".into(),
            ));
        }
        let action: DependencyAction = serde_json::from_str(&event.text)
            .map_err(|error| DependencyError::Invalid(format!("{}: {error}", event.id)))?;
        let expected = self.recipient(event, &action)?;
        if event.to.as_ref() != Some(&expected) {
            return Err(DependencyError::Invalid(
                "recipient does not match dependency participants".into(),
            ));
        }
        let next = self.transition(event, &action)?;
        self.views.insert(next.id.clone(), next);
        self.seen.insert(event.id.clone(), event.clone());
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<DependencyView, DependencyError> {
        self.views
            .get(id)
            .cloned()
            .ok_or_else(|| DependencyError::Unknown(id.into()))
    }

    pub fn list(&self) -> Vec<DependencyView> {
        self.views.values().cloned().collect()
    }

    fn recipient(
        &self,
        event: &Event,
        action: &DependencyAction,
    ) -> Result<Recipient, DependencyError> {
        let session = match action {
            DependencyAction::Declare { responder, .. } => responder.clone(),
            DependencyAction::Resolve { id, .. } => self.get(id)?.requester_session,
            DependencyAction::Cancel { id, .. } | DependencyAction::Consume { id } => {
                self.get(id)?.responder_session
            }
        };
        nonempty(&event.session, "session")?;
        nonempty(&session, "responder")?;
        Ok(Recipient {
            kind: RecipientKind::Session,
            value: session,
        })
    }

    fn transition(
        &self,
        event: &Event,
        action: &DependencyAction,
    ) -> Result<DependencyView, DependencyError> {
        match action {
            DependencyAction::Declare {
                responder,
                condition,
                continuation,
                deadline_ms,
            } => {
                nonempty(condition, "condition")?;
                nonempty(continuation, "continuation")?;
                if self.views.contains_key(&event.id) {
                    return Err(DependencyError::Conflict(
                        "dependency already declared".into(),
                    ));
                }
                Ok(DependencyView {
                    id: event.id.clone(),
                    requester_session: event.session.clone(),
                    responder_session: responder.clone(),
                    condition: condition.clone(),
                    continuation: continuation.clone(),
                    deadline_ms: *deadline_ms,
                    state: DependencyState::Pending,
                    resolution_event_id: None,
                    evidence: None,
                    consumed: false,
                })
            }
            DependencyAction::Resolve {
                id,
                outcome,
                evidence,
            } => {
                let mut view = self.get(id)?;
                participant(&event.session, &view.responder_session)?;
                nonempty(evidence, "evidence")?;
                let state = match outcome {
                    DependencyOutcome::Satisfied => DependencyState::Satisfied,
                    DependencyOutcome::Failed => DependencyState::Failed,
                };
                if view.state == state && view.evidence.as_deref() == Some(evidence) {
                    return Ok(view);
                }
                pending(&view)?;
                view.state = state;
                view.evidence = Some(evidence.clone());
                view.resolution_event_id = Some(event.id.clone());
                Ok(view)
            }
            DependencyAction::Cancel { id, reason } => {
                let mut view = self.get(id)?;
                participant(&event.session, &view.requester_session)?;
                nonempty(reason, "reason")?;
                if view.state == DependencyState::Cancelled
                    && view.evidence.as_deref() == Some(reason)
                {
                    return Ok(view);
                }
                pending(&view)?;
                view.state = DependencyState::Cancelled;
                view.evidence = Some(reason.clone());
                view.resolution_event_id = Some(event.id.clone());
                Ok(view)
            }
            DependencyAction::Consume { id } => {
                let mut view = self.get(id)?;
                participant(&event.session, &view.requester_session)?;
                if view.state == DependencyState::Pending {
                    return Err(DependencyError::Conflict(
                        "pending dependency has no result to consume".into(),
                    ));
                }
                view.consumed = true;
                Ok(view)
            }
        }
    }
}

fn nonempty(value: &str, field: &str) -> Result<(), DependencyError> {
    if value.trim().is_empty() {
        Err(DependencyError::Invalid(format!(
            "{field} must not be empty"
        )))
    } else {
        Ok(())
    }
}

fn participant(actual: &str, expected: &str) -> Result<(), DependencyError> {
    if actual == expected {
        Ok(())
    } else {
        Err(DependencyError::Unauthorized(format!(
            "expected {expected}, got {actual}"
        )))
    }
}

fn pending(view: &DependencyView) -> Result<(), DependencyError> {
    if view.state == DependencyState::Pending {
        Ok(())
    } else {
        Err(DependencyError::Conflict(format!(
            "{} already has a terminal result",
            view.id
        )))
    }
}

fn check_read(read: &crate::ReadResult) -> Result<(), DependencyError> {
    if read.corrupt_lines != 0 || read.unknown_events != 0 {
        Err(DependencyError::CorruptHistory {
            corrupt_lines: read.corrupt_lines,
            unknown_events: read.unknown_events,
        })
    } else {
        Ok(())
    }
}

fn load(store: &Store) -> Result<DependencyProjection, DependencyError> {
    // Store's ordinary reader is best-effort; dependency state must fail closed
    // when an existing segment cannot be read.
    match fs::read_dir(store.events_dir()) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(DependencyProjection::default());
        }
        Err(error) => return Err(error.into()),
    }
    let expected_segments = store.segments().len() as u64;
    let read = store.read(Window::All);
    if read.segments_read < expected_segments {
        return Err(DependencyError::Invalid(
            "dependency history contains unreadable segments".into(),
        ));
    }
    check_read(&read)?;
    DependencyProjection::from_events(&read.events)
}

/// Reconstruct the complete history once. Watchers should retain a projection
/// and apply incremental reads instead of calling this on every poll or hook.
pub fn list(store: &Store) -> Result<Vec<DependencyView>, DependencyError> {
    Ok(load(store)?.list())
}

pub fn get(store: &Store, id: &str) -> Result<DependencyView, DependencyError> {
    load(store)?.get(id)
}

/// Serialize validation and append across processes. An equivalent terminal
/// action is a successful retry and does not append another event.
pub fn append_action(
    store: &Store,
    mut event: Event,
    action: DependencyAction,
) -> Result<DependencyView, DependencyError> {
    let _process_guard = PROCESS_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    fs::create_dir_all(store.root())?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(store.root().join("dependency.lock"))?;
    lock.lock_exclusive()?;
    // Dropping the handle releases the lock on all error paths too.
    let mut projection = load(store)?;
    event.event = EventKind::Dependency;
    event.kind = NoteKind::Fyi;
    event.note_ref = None;
    event.to = Some(projection.recipient(&event, &action)?);
    event.text = serde_json::to_string(&action)
        .map_err(|error| DependencyError::Invalid(error.to_string()))?;
    event
        .to_line()
        .map_err(|error| DependencyError::Invalid(error.to_string()))?;
    let id = action.id(&event).to_string();
    let before = projection.views.get(&id).cloned();
    projection.apply(&event)?;
    let view = projection.get(&id)?;
    if before.as_ref() != Some(&view) {
        store.append(&event)?;
    }
    FileExt::unlock(&lock)?;
    Ok(view)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempHome;
    use std::io::Write;
    use std::sync::{Arc, Barrier};

    fn event(session: &str) -> Event {
        Event {
            v: crate::SCHEMA_VERSION,
            id: ulid::Ulid::generate().to_string(),
            ts: "2026-09-13T12:00:00Z".into(),
            event: EventKind::Note,
            session: session.into(),
            harness: "test".into(),
            host: "localhost".into(),
            cwd: "/test".into(),
            repo: None,
            repo_key: None,
            branch: None,
            role: crate::Role::Main,
            parent: None,
            text: String::new(),
            to: None,
            kind: NoteKind::Fyi,
            note_ref: None,
        }
    }

    fn declare(store: &Store) -> DependencyView {
        append_action(
            store,
            event("requester"),
            DependencyAction::Declare {
                responder: "responder".into(),
                condition: "Publish the reviewed revision".into(),
                continuation: "Verify the revision before continuing".into(),
                deadline_ms: Some(1),
            },
        )
        .unwrap()
    }

    fn resolve(id: &str, outcome: DependencyOutcome) -> DependencyAction {
        DependencyAction::Resolve {
            id: id.into(),
            outcome,
            evidence: "revision abc123 is available".into(),
        }
    }

    #[test]
    fn response_before_watching_survives_reconstruction_and_expired_deadline() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let view = declare(&store);
        assert_eq!(
            get(&store, &view.id).unwrap().state,
            DependencyState::Pending
        );
        let result = append_action(
            &store,
            event("responder"),
            resolve(&view.id, DependencyOutcome::Satisfied),
        )
        .unwrap();
        let read = store.read(Window::All);
        let mut watcher = DependencyProjection::default();
        watcher.apply_read(&read).unwrap();
        assert_eq!(watcher.get(&view.id).unwrap(), result);
        assert_eq!(result.state, DependencyState::Satisfied);
        assert!(!result.consumed);
        assert_eq!(result.deadline_ms, Some(1));
        assert_eq!(read.events[0].to.as_ref().unwrap().value, "responder");
        assert_eq!(read.events[1].to.as_ref().unwrap().value, "requester");
    }

    #[test]
    fn only_exact_participants_can_resolve_cancel_or_consume() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let view = declare(&store);
        for actor in ["requester", "other-responder", "responder-extra"] {
            assert!(matches!(
                append_action(
                    &store,
                    event(actor),
                    resolve(&view.id, DependencyOutcome::Satisfied)
                ),
                Err(DependencyError::Unauthorized(_))
            ));
        }
        assert!(matches!(
            append_action(
                &store,
                event("responder"),
                DependencyAction::Cancel {
                    id: view.id.clone(),
                    reason: "cancel".into()
                }
            ),
            Err(DependencyError::Unauthorized(_))
        ));
        assert!(matches!(
            append_action(
                &store,
                event("requester"),
                DependencyAction::Consume {
                    id: view.id.clone()
                }
            ),
            Err(DependencyError::Conflict(_))
        ));
        append_action(
            &store,
            event("responder"),
            resolve(&view.id, DependencyOutcome::Satisfied),
        )
        .unwrap();
        assert!(matches!(
            append_action(
                &store,
                event("other"),
                resolve(&view.id, DependencyOutcome::Satisfied)
            ),
            Err(DependencyError::Unauthorized(_))
        ));
        assert!(matches!(
            append_action(
                &store,
                event("responder"),
                DependencyAction::Consume {
                    id: view.id.clone()
                }
            ),
            Err(DependencyError::Unauthorized(_))
        ));
        assert_eq!(store.read(Window::All).events.len(), 2);
    }

    #[test]
    fn equivalent_retries_do_not_append_or_replace_first_evidence() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let view = declare(&store);
        let action = resolve(&view.id, DependencyOutcome::Satisfied);
        let first = append_action(&store, event("responder"), action.clone()).unwrap();
        assert_eq!(
            append_action(&store, event("responder"), action).unwrap(),
            first
        );
        assert!(matches!(
            append_action(
                &store,
                event("responder"),
                resolve(&view.id, DependencyOutcome::Failed)
            ),
            Err(DependencyError::Conflict(_))
        ));
        assert!(matches!(
            append_action(
                &store,
                event("responder"),
                DependencyAction::Resolve {
                    id: view.id.clone(),
                    outcome: DependencyOutcome::Satisfied,
                    evidence: "different evidence".into()
                }
            ),
            Err(DependencyError::Conflict(_))
        ));
        let consumed = append_action(
            &store,
            event("requester"),
            DependencyAction::Consume {
                id: view.id.clone(),
            },
        )
        .unwrap();
        assert!(consumed.consumed);
        assert_eq!(consumed.resolution_event_id, first.resolution_event_id);
        assert_eq!(
            append_action(
                &store,
                event("requester"),
                DependencyAction::Consume {
                    id: view.id.clone()
                }
            )
            .unwrap(),
            consumed
        );
        assert_eq!(store.read(Window::All).events.len(), 3);
    }

    #[test]
    fn cancelled_dependency_is_terminal_and_consumable() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let view = declare(&store);
        let cancel = DependencyAction::Cancel {
            id: view.id.clone(),
            reason: "No longer needed".into(),
        };
        let first = append_action(&store, event("requester"), cancel.clone()).unwrap();
        assert_eq!(first.state, DependencyState::Cancelled);
        assert_eq!(
            append_action(&store, event("requester"), cancel).unwrap(),
            first
        );
        assert!(matches!(
            append_action(
                &store,
                event("requester"),
                DependencyAction::Cancel {
                    id: view.id.clone(),
                    reason: "Different reason".into()
                }
            ),
            Err(DependencyError::Conflict(_))
        ));
        assert!(matches!(
            append_action(
                &store,
                event("responder"),
                resolve(&view.id, DependencyOutcome::Satisfied)
            ),
            Err(DependencyError::Conflict(_))
        ));
        assert!(
            append_action(
                &store,
                event("requester"),
                DependencyAction::Consume {
                    id: view.id.clone()
                }
            )
            .unwrap()
            .consumed
        );
        assert_eq!(store.read(Window::All).events.len(), 3);
    }

    #[test]
    fn notes_and_acknowledgements_cannot_resolve_a_dependency() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let view = declare(&store);
        for kind in [EventKind::Note, EventKind::Ack] {
            let mut message = event("responder");
            message.event = kind;
            message.text =
                serde_json::to_string(&resolve(&view.id, DependencyOutcome::Satisfied)).unwrap();
            message.note_ref = Some(view.id.clone());
            store.append(&message).unwrap();
        }
        assert_eq!(get(&store, &view.id).unwrap(), view);
    }

    #[test]
    fn failed_result_is_terminal_and_an_unknown_dependency_cannot_be_resolved() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        assert!(matches!(
            append_action(
                &store,
                event("responder"),
                resolve("missing", DependencyOutcome::Failed)
            ),
            Err(DependencyError::Unknown(_))
        ));
        assert!(list(&store).unwrap().is_empty());
        let view = declare(&store);
        let failed = append_action(
            &store,
            event("responder"),
            resolve(&view.id, DependencyOutcome::Failed),
        )
        .unwrap();
        assert_eq!(failed.state, DependencyState::Failed);
        assert!(matches!(
            append_action(
                &store,
                event("responder"),
                resolve(&view.id, DependencyOutcome::Satisfied)
            ),
            Err(DependencyError::Conflict(_))
        ));
        assert_eq!(get(&store, &view.id).unwrap(), failed);
    }

    #[test]
    fn incremental_batch_failure_leaves_the_prior_projection_intact() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let view = declare(&store);
        let mut projection = DependencyProjection::default();
        projection.apply_read(&store.read(Window::All)).unwrap();
        append_action(
            &store,
            event("responder"),
            resolve(&view.id, DependencyOutcome::Satisfied),
        )
        .unwrap();
        let mut read = store.read(Window::All);
        let mut invalid = event("responder");
        invalid.event = EventKind::Dependency;
        invalid.text = "unknown action".into();
        read.events.push(invalid);
        assert!(projection.apply_read(&read).is_err());
        assert_eq!(projection.get(&view.id).unwrap(), view);
    }

    #[test]
    fn replay_deduplicates_exact_events_and_rejects_reused_ids() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        declare(&store);
        let read = store.read(Window::All);
        let mut projection = DependencyProjection::from_events(&read.events).unwrap();
        projection.apply_read(&read).unwrap();
        assert_eq!(projection.list().len(), 1);
        let mut reused = read.events[0].clone();
        reused.session = "other".into();
        assert!(matches!(
            projection.apply(&reused),
            Err(DependencyError::Conflict(_))
        ));
        assert_eq!(projection.list()[0].requester_session, "requester");
    }

    #[test]
    fn malformed_unknown_and_misaddressed_actions_fail_closed() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let view = declare(&store);
        let initial = store.read(Window::All);
        for payload in ["not json", r#"{"action":"future_action"}"#] {
            let mut projection = DependencyProjection::from_events(&initial.events).unwrap();
            let mut invalid = event("responder");
            invalid.event = EventKind::Dependency;
            invalid.text = payload.into();
            assert!(matches!(
                projection.apply(&invalid),
                Err(DependencyError::Invalid(_))
            ));
            assert_eq!(projection.get(&view.id).unwrap(), view);
        }
        let mut projection = DependencyProjection::from_events(&initial.events).unwrap();
        let mut invalid = event("responder");
        invalid.event = EventKind::Dependency;
        invalid.text =
            serde_json::to_string(&resolve(&view.id, DependencyOutcome::Satisfied)).unwrap();
        invalid.to = Some(Recipient {
            kind: RecipientKind::Session,
            value: "other".into(),
        });
        assert!(matches!(
            projection.apply(&invalid),
            Err(DependencyError::Invalid(_))
        ));
        store.append(&invalid).unwrap();
        assert!(list(&store).is_err());
        assert!(
            append_action(
                &store,
                event("responder"),
                resolve(&view.id, DependencyOutcome::Satisfied)
            )
            .is_err()
        );
    }

    #[test]
    fn corrupt_or_unknown_envelopes_cannot_silently_disappear_from_waits() {
        for line in [
            "{invalid json}\n",
            "{\"v\":1,\"id\":\"future\",\"ts\":\"now\",\"event\":\"future_dependency\",\"session\":\"s\",\"harness\":\"test\",\"host\":\"h\",\"cwd\":\"/\"}\n",
        ] {
            let home = TempHome::new();
            let store = Store::at(home.path());
            let view = declare(&store);
            let path = store.segments().pop().unwrap();
            OpenOptions::new()
                .append(true)
                .open(path)
                .unwrap()
                .write_all(line.as_bytes())
                .unwrap();
            assert!(matches!(
                get(&store, &view.id),
                Err(DependencyError::CorruptHistory { .. })
            ));
            assert!(matches!(
                DependencyProjection::default().apply_read(&store.read(Window::All)),
                Err(DependencyError::CorruptHistory { .. })
            ));
        }
    }

    #[test]
    fn simultaneous_resolution_and_cancellation_have_one_terminal_winner() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let view = declare(&store);
        let barrier = Arc::new(Barrier::new(3));
        let handles: Vec<_> = [false, true]
            .into_iter()
            .map(|cancel| {
                let store = store.clone();
                let id = view.id.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    if cancel {
                        append_action(
                            &store,
                            event("requester"),
                            DependencyAction::Cancel {
                                id,
                                reason: "Cancelled".into(),
                            },
                        )
                    } else {
                        append_action(
                            &store,
                            event("responder"),
                            resolve(&id, DependencyOutcome::Satisfied),
                        )
                    }
                })
            })
            .collect();
        barrier.wait();
        let outcomes: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(outcomes.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(
            outcomes
                .iter()
                .filter(|result| matches!(result, Err(DependencyError::Conflict(_))))
                .count(),
            1
        );
        assert_eq!(store.read(Window::All).events.len(), 2);
        assert_ne!(
            get(&store, &view.id).unwrap().state,
            DependencyState::Pending
        );
    }
}
