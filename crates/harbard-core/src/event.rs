//! Event schema (v1) and its typed refusals.
//!
//! One JSON object per line. Unknown fields are ignored (serde default),
//! unknown `event`/`role`/recipient kinds decode to `Unknown` so the line still
//! parses and is skipped at read time rather than counted as corrupt.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Maximum bytes for a serialized line including its trailing newline.
pub const MAX_LINE_BYTES: usize = 4096;
/// Maximum bytes for the `text` field (intent or note body).
pub const MAX_TEXT_BYTES: usize = 2000;
/// Current schema version.
pub const SCHEMA_VERSION: u32 = 1;

/// The four declared event kinds. `Unknown` absorbs forward-compatible types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EventKind {
    Hello,
    Intent,
    Note,
    Bye,
    /// Joined a named channel (F3). The channel is in `to`.
    Join,
    /// Left a named channel (F3). The channel is in `to`.
    Leave,
    /// Acknowledges a note (F5). `text` carries the acknowledged event id.
    Ack,
    /// A typed durable dependency action, encoded as JSON in `text`.
    Dependency,
    /// A machine joined the hub fleet (M4a). `session` is `device:<id>`.
    #[serde(rename = "device_join")]
    DeviceJoin,
    /// A machine left / unpaired from the hub fleet (M4a).
    #[serde(rename = "device_leave")]
    DeviceLeave,
    /// The operator's one-tap feedback on something the console showed, or
    /// the undo of one. JSON in `text` ([`crate::feedback`]).
    Feedback,
    /// A rule the operator approved, edited, discarded or retired. JSON in
    /// `text` ([`crate::rules`]).
    Rule,
    /// A room or project channel was created, updated, archived, unarchived
    /// or deleted. The channel is in `to`, JSON in `text`
    /// ([`crate::rooms::LifecycleBody`]). Older builds read it as `Unknown`.
    #[serde(rename = "channel_lifecycle")]
    ChannelLifecycle,
    /// Any event type this build does not recognize (skipped at read time).
    #[serde(other)]
    Unknown,
}

impl EventKind {
    /// Stable lowercase label for human/CLI rendering.
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Hello => "hello",
            EventKind::Intent => "intent",
            EventKind::Note => "note",
            EventKind::Bye => "bye",
            EventKind::Join => "join",
            EventKind::Leave => "leave",
            EventKind::Ack => "ack",
            EventKind::Dependency => "dependency",
            EventKind::DeviceJoin => "device_join",
            EventKind::DeviceLeave => "device_leave",
            EventKind::Feedback => "feedback",
            EventKind::Rule => "rule",
            EventKind::ChannelLifecycle => "channel_lifecycle",
            EventKind::Unknown => "unknown",
        }
    }
}

/// The session's role in the fleet. Only top-level sessions get hook wiring
/// (E1); `role` future-proofs herd/headless visibility.
///
/// `Operator` is the human (F2): the person joins the bus as a session like any
/// other rather than as a privileged plane above it. That is the whole design
/// — one event log, one delivery path, one place to read the conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    #[default]
    Main,
    Herd,
    Headless,
    Operator,
    #[serde(other)]
    Unknown,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Main => "main",
            Role::Herd => "herd",
            Role::Headless => "headless",
            Role::Operator => "operator",
            Role::Unknown => "unknown",
        }
    }
}

/// The harness string reserved for the human operator (F2).
pub const HARNESS_HUMAN: &str = "human";

/// Who is reading, for delivery decisions (F3).
///
/// The first three fields are properties a session already has; `channels` is
/// what it has declared, resolved by the caller from the membership store.
///
/// `repo` is the reader's own checkout — the working tree it sits in, which is
/// also what the desk shows and what its branch is resolved from. `repo_key` is
/// the identity of the REPOSITORY that checkout belongs to, shared by every
/// sibling worktree. The two differ exactly when the reader sits in a linked
/// worktree, and keeping both is what lets a `repo:` note reach the whole
/// repository without the desk losing track of which tree each agent is in.
#[derive(Debug, Clone, Copy)]
pub struct Reader<'a> {
    pub session: &'a str,
    pub harness: &'a str,
    pub repo: Option<&'a str>,
    /// Repository identity shared by every worktree; `None` outside a repo.
    pub repo_key: Option<&'a str>,
    pub channels: &'a [String],
    /// The reader's workspace boundary ([`crate::workspaces::Gate`]): what
    /// it may be handed at all, whatever the address says. Required, so no
    /// reader is built without deciding it.
    pub workspace: &'a crate::workspaces::Gate<'a>,
}

/// What a note is for (F5).
///
/// Codex asked for this after living with the inline channel: "human notes
/// without context are noise on the hot path; directed notes with repo,
/// objective, SHA or state are useful". So the kind is not decoration — it
/// decides DELIVERY. `Fyi` waits for a turn boundary; `Warning` and `Handoff`
/// interrupt, because those are the ones that stop an agent overwriting
/// somebody or picking up work already done.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum NoteKind {
    /// Context. Real but not urgent — it can wait for the next turn.
    #[default]
    Fyi,
    /// Something that can break the reader's work if they do not see it now.
    Warning,
    /// Work being handed over. Expects an `ack`.
    Handoff,
    /// A question that owes an answer, from EVERY live session it reaches.
    ///
    /// The gap this fills, seen three times in two days: the operator wants a
    /// report from four agents at once. `handoff` creates the obligation but
    /// must name exactly one live session, because handing work over to a
    /// crowd hands it to nobody. `warning` reaches a crowd but creates no
    /// obligation by design — it interrupts and expects nothing. So the
    /// operator used `warning`, and an agent busy with its own work simply
    /// never answered, with nothing anywhere recording that it owed one.
    ///
    /// An `ask` interrupts like a warning and is owed like a handoff, by each
    /// recipient independently. Closure is causal — an `ack`, or a note whose
    /// `ref` names it — never a delivery cursor: a cursor proves a session
    /// scanned past the log position, not that anyone answered.
    Ask,
    #[serde(other)]
    Unknown,
}

impl NoteKind {
    pub fn as_str(self) -> &'static str {
        match self {
            NoteKind::Fyi => "fyi",
            NoteKind::Warning => "warning",
            NoteKind::Handoff => "handoff",
            NoteKind::Ask => "ask",
            NoteKind::Unknown => "unknown",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fyi" => Some(NoteKind::Fyi),
            "warning" => Some(NoteKind::Warning),
            "handoff" => Some(NoteKind::Handoff),
            "ask" => Some(NoteKind::Ask),
            _ => None,
        }
    }

    /// True when this note earns a place between tool calls.
    pub fn interrupts(self) -> bool {
        matches!(self, NoteKind::Warning | NoteKind::Handoff | NoteKind::Ask)
    }

    /// True when the reader owes the author an answer.
    ///
    /// A handoff is owed by its one named session; an ask is owed by every live
    /// session it reaches. Nothing else creates an obligation — an `fyi` and a
    /// `warning` are told, not asked.
    pub fn owes_an_answer(self) -> bool {
        matches!(self, NoteKind::Handoff | NoteKind::Ask)
    }
}

/// Kind of a note recipient. Unknown recipients are accepted and stored;
/// delivery is decided read-side, so an unknown kind is simply undeliverable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecipientKind {
    Harness,
    Session,
    Repo,
    /// A named channel (F3). Unlike the other three — which are properties a
    /// session already has — membership is declared, so delivery needs the
    /// reader's subscriptions to decide.
    Channel,
    #[serde(other)]
    Unknown,
}

impl RecipientKind {
    pub fn as_str(self) -> &'static str {
        match self {
            RecipientKind::Harness => "harness",
            RecipientKind::Session => "session",
            RecipientKind::Repo => "repo",
            RecipientKind::Channel => "channel",
            RecipientKind::Unknown => "unknown",
        }
    }
}

/// A note's optional target. Absent = broadcast.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recipient {
    pub kind: RecipientKind,
    pub value: String,
}

/// A single bus event. Field order follows the SPEC schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Event {
    pub v: u32,
    pub id: String,
    pub ts: String,
    pub event: EventKind,
    pub session: String,
    pub harness: String,
    pub host: String,
    pub cwd: String,
    #[serde(default)]
    pub repo: Option<String>,
    /// Identity of the REPOSITORY `repo` belongs to, shared by every sibling
    /// worktree (the primary working tree's root).
    ///
    /// Written only when it differs from `repo` — that is, only by a session
    /// sitting in a linked worktree. Every line stays within
    /// [`MAX_LINE_BYTES`], and the common case pays nothing. Read it through
    /// [`Event::repository`], never directly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo_key: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub role: Role,
    /// Parent session when this writer is a herd/subagent (Codex Task child).
    /// Absent on top-level sessions and on all pre-herd lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<Recipient>,
    /// What this note is for (F5). Absent in events written before F5, which
    /// decode as `Fyi` — the quiet default, so old traffic never gains the
    /// right to interrupt retroactively.
    #[serde(default, skip_serializing_if = "is_default_kind")]
    pub kind: NoteKind,
    /// Parent note id when this note replies to another note (wire: `ref`).
    /// Absent on top-level notes and on all pre-reply traffic. One level only
    /// — children nest under the parent; grandchildren stay flat under theirs.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "ref")]
    pub note_ref: Option<String>,
}

fn is_default_kind(k: &NoteKind) -> bool {
    *k == NoteKind::Fyi
}

/// Prefix of the `cwd` of an operator event written inside one workspace.
///
/// The operator has no repository, so nothing else on the line can place what
/// they wrote. Only an operator event is read this way ([`Event::workspace_stamp`]):
/// any other writer is placed by the repository it works in.
pub const WORKSPACE_STAMP: &str = "workspace:";

/// True when `s` looks like a harbard event id (ULID: 26 Crockford chars).
pub fn is_event_id(s: &str) -> bool {
    s.len() == 26
        && s.bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'A'..=b'Z' | b'a'..=b'z'))
}

impl Event {
    /// The note acknowledged by this ACK, independent of its display text.
    /// Explicit references take precedence; legacy CLI ACKs carry an event ID
    /// followed by optional whitespace and a comment (including ` — note`).
    pub fn acknowledged_note_id(&self) -> Option<&str> {
        if self.event != EventKind::Ack {
            return None;
        }
        let target = match self.note_ref.as_deref() {
            Some(reference) => reference,
            None => self.text.split_ascii_whitespace().next()?,
        };
        is_event_id(target).then_some(target)
    }

    /// The workspace the operator wrote this in, when they wrote it inside one.
    pub fn workspace_stamp(&self) -> Option<&str> {
        if !self.is_operator() {
            return None;
        }
        self.cwd
            .strip_prefix(WORKSPACE_STAMP)
            .filter(|id| !id.is_empty())
    }

    /// The repository this event was written from: `repo_key` when the writer
    /// sat in a linked worktree, otherwise the checkout path itself.
    pub fn repository(&self) -> Option<&str> {
        self.repo_key.as_deref().or(self.repo.as_deref())
    }

    /// Serialize to a single JSONL line (no trailing newline).
    ///
    /// Returns [`EventError::TextTooLong`] when `text` exceeds
    /// [`MAX_TEXT_BYTES`] and [`EventError::LineTooLong`] when the whole line
    /// (with its future newline) would exceed [`MAX_LINE_BYTES`]. No silent
    /// truncation, ever.
    pub fn to_line(&self) -> Result<String, EventError> {
        if self.text.len() > MAX_TEXT_BYTES {
            return Err(EventError::TextTooLong {
                got: self.text.len(),
                max: MAX_TEXT_BYTES,
            });
        }
        let line = serde_json::to_string(self).map_err(EventError::Serialize)?;
        if line.len() + 1 > MAX_LINE_BYTES {
            return Err(EventError::LineTooLong {
                got: line.len() + 1,
                max: MAX_LINE_BYTES,
            });
        }
        Ok(line)
    }

    /// True when the operator (the human) wrote this event, not an agent.
    ///
    /// Both markers must agree: a stray `harness: "human"` from an agent that
    /// mis-set its context does not become an operator message on its own.
    pub fn is_operator(&self) -> bool {
        self.harness == HARNESS_HUMAN && self.role == Role::Operator
    }

    /// True when this event was addressed to (or broadcast toward) the reader
    /// identified by `session`, `harness`, `repo`. Own events are excluded by
    /// the caller.
    ///
    /// Channel-addressed events are never delivered through this entry point —
    /// it has no way to know what the reader is subscribed to. Use
    /// [`Event::is_addressed_to_reader`] where channels matter; keeping the
    /// narrow answer here means a caller that has not been taught about
    /// channels under-delivers rather than leaking a channel's traffic to
    /// everyone.
    pub fn is_addressed_to(
        &self,
        session: &str,
        harness: &str,
        repo: Option<&str>,
        repo_key: Option<&str>,
        workspace: &crate::workspaces::Gate<'_>,
    ) -> bool {
        self.is_addressed_to_reader(&Reader {
            session,
            harness,
            repo,
            repo_key: repo_key.or(repo),
            channels: &[],
            workspace,
        })
    }

    /// True when this event reaches `reader`, channels included, and was
    /// written inside the reader's workspace (or by the operator).
    pub fn is_addressed_to_reader(&self, reader: &Reader<'_>) -> bool {
        self.matches_address(reader) && reader.workspace.admits(self)
    }

    /// The address alone, before the workspace boundary.
    fn matches_address(&self, reader: &Reader<'_>) -> bool {
        match &self.to {
            None => true, // broadcast
            Some(r) => match r.kind {
                RecipientKind::Harness => r.value == reader.harness,
                RecipientKind::Session => r.value == reader.session,
                // Matched on BOTH the reader's own checkout and its
                // repository identity. Comparing only the checkout path meant a
                // note addressed to a worktree never reached the primary, and a
                // note to the primary never reached a worktree: the two spellings
                // are never equal, so 1,074 of the 1,081 notes ever addressed to
                // a worktree path died without a single reader. That contradicted
                // the house rule that says to open your own worktree rather than
                // queue for someone else's checkout — every agent obeying it
                // built itself a mailbox it was the only member of.
                //
                // The `repo` arm stays so events written before this change,
                // which stored a worktree path, still reach the session actually
                // sitting in that worktree. The match is strictly wider than it
                // was; nothing that used to be delivered stops being delivered.
                RecipientKind::Repo => {
                    reader.repo.is_some_and(|me| me == r.value)
                        || reader.repo_key.is_some_and(|key| key == r.value)
                }
                RecipientKind::Channel => reader
                    .channels
                    .iter()
                    .any(|c| c.eq_ignore_ascii_case(&r.value)),
                RecipientKind::Unknown => false,
            },
        }
    }
}

/// Typed refusals for event construction/serialization.
#[derive(Debug)]
pub enum EventError {
    TextTooLong { got: usize, max: usize },
    LineTooLong { got: usize, max: usize },
    Serialize(serde_json::Error),
}

impl fmt::Display for EventError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventError::TextTooLong { got, max } => write!(
                f,
                "text is {got} bytes; the limit is {max} bytes (no silent truncation)"
            ),
            EventError::LineTooLong { got, max } => {
                write!(f, "event line is {got} bytes; the limit is {max} bytes")
            }
            EventError::Serialize(e) => write!(f, "serialize event: {e}"),
        }
    }
}

impl std::error::Error for EventError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(text: &str) -> Event {
        Event {
            v: SCHEMA_VERSION,
            id: "01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string(),
            ts: "2026-07-15T10:00:00Z".to_string(),
            event: EventKind::Note,
            session: "s1".to_string(),
            harness: "claude-code".to_string(),
            host: "h".to_string(),
            cwd: "/tmp".to_string(),
            repo: None,
            repo_key: None,
            branch: None,
            role: Role::Main,
            parent: None,
            note_ref: None,
            text: text.to_string(),
            to: None,
            kind: NoteKind::Fyi,
        }
    }

    #[test]
    fn ack_references_accept_cli_comments_and_prefer_explicit_refs() {
        let id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        for text in [
            id.to_string(),
            format!("{id} — taken on"),
            format!("  {id} taken on  "),
        ] {
            let mut ack = sample(&text);
            ack.event = EventKind::Ack;
            assert_eq!(ack.acknowledged_note_id(), Some(id));
        }
        let mut ack = sample("01ARZ3NDEKTSV4RRFFQ69G5FAW — another note");
        ack.event = EventKind::Ack;
        ack.note_ref = Some(id.to_string());
        assert_eq!(ack.acknowledged_note_id(), Some(id));
        ack.note_ref = Some("invalid".to_string());
        assert_eq!(
            ack.acknowledged_note_id(),
            None,
            "invalid refs must not fall back to text"
        );
    }

    #[test]
    fn ack_references_reject_malformed_ids_and_non_ack_events() {
        let id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
        for text in [
            String::new(),
            "short — comment".into(),
            format!("{id}extra"),
            format!("{id}— comment"),
            format!("{id}: comment"),
            "01ARZ3NDEKTSV4RRFFQ69G5FA! — comment".into(),
        ] {
            let mut ack = sample(&text);
            ack.event = EventKind::Ack;
            assert_eq!(ack.acknowledged_note_id(), None, "{text:?}");
        }
        for kind in [EventKind::Note, EventKind::Dependency] {
            let mut event = sample(id);
            event.event = kind;
            event.note_ref = Some(id.to_string());
            assert_eq!(event.acknowledged_note_id(), None);
        }
    }

    #[test]
    fn note_ref_round_trips_and_omits_when_absent() {
        let mut with = sample("reply");
        with.note_ref = Some("01ARZ3NDEKTSV4RRFFQ69G5FAV".to_string());
        let line = with.to_line().unwrap();
        assert!(line.contains(r#""ref":"01ARZ3NDEKTSV4RRFFQ69G5FAV""#));
        assert!(!line.contains("note_ref"));
        let back: Event = serde_json::from_str(&line).unwrap();
        assert_eq!(back.note_ref.as_deref(), Some("01ARZ3NDEKTSV4RRFFQ69G5FAV"));

        let bare = sample("hi");
        let bare_line = bare.to_line().unwrap();
        assert!(!bare_line.contains("\"ref\""));
        let bare_back: Event = serde_json::from_str(&bare_line).unwrap();
        assert_eq!(bare_back.note_ref, None);
    }

    #[test]
    fn is_event_id_accepts_ulid_shape() {
        assert!(is_event_id("01ARZ3NDEKTSV4RRFFQ69G5FAV"));
        assert!(!is_event_id("short"));
        assert!(!is_event_id("01ARZ3NDEKTSV4RRFFQ69G5FA!"));
    }

    #[test]
    fn parent_round_trips_and_omits_when_absent() {
        let mut with = sample("hi");
        with.parent = Some("parent-session".to_string());
        with.role = Role::Herd;
        let line = with.to_line().unwrap();
        assert!(line.contains(r#""parent":"parent-session""#));
        let back: Event = serde_json::from_str(&line).unwrap();
        assert_eq!(back.parent.as_deref(), Some("parent-session"));
        assert_eq!(back.role, Role::Herd);

        let bare = sample("hi");
        let bare_line = bare.to_line().unwrap();
        assert!(!bare_line.contains("parent"));
        let bare_back: Event = serde_json::from_str(&bare_line).unwrap();
        assert_eq!(bare_back.parent, None);
    }

    #[test]
    fn unknown_event_kind_decodes_to_unknown_not_error() {
        let line = r#"{"v":1,"id":"x","ts":"t","event":"reaction","session":"s","harness":"codex","host":"h","cwd":"/","role":"main","text":""}"#;
        let ev: Event = serde_json::from_str(line).expect("parses");
        assert_eq!(ev.event, EventKind::Unknown);
    }

    #[test]
    fn unknown_fields_ignored() {
        let line = r#"{"v":1,"id":"x","ts":"t","event":"hello","session":"s","harness":"codex","host":"h","cwd":"/","role":"main","text":"","future_field":42}"#;
        let ev: Event = serde_json::from_str(line).expect("parses");
        assert_eq!(ev.event, EventKind::Hello);
    }

    #[test]
    fn oversized_text_is_typed_error() {
        let big = "a".repeat(MAX_TEXT_BYTES + 1);
        let err = sample(&big).to_line().unwrap_err();
        assert!(matches!(err, EventError::TextTooLong { .. }));
    }

    #[test]
    fn addressing_matches_broadcast_and_targets() {
        let mut ev = sample("hi");
        assert!(ev.is_addressed_to(
            "me",
            "codex",
            Some("/r"),
            None,
            &crate::workspaces::Gate::OPEN
        ));
        ev.to = Some(Recipient {
            kind: RecipientKind::Harness,
            value: "codex".to_string(),
        });
        assert!(ev.is_addressed_to("me", "codex", None, None, &crate::workspaces::Gate::OPEN));
        assert!(!ev.is_addressed_to(
            "me",
            "claude-code",
            None,
            None,
            &crate::workspaces::Gate::OPEN
        ));
        ev.to = Some(Recipient {
            kind: RecipientKind::Unknown,
            value: "??".to_string(),
        });
        assert!(!ev.is_addressed_to(
            "me",
            "codex",
            Some("/r"),
            None,
            &crate::workspaces::Gate::OPEN
        ));
    }

    fn repo_note(value: &str) -> Event {
        let mut ev = sample("hi");
        ev.to = Some(Recipient {
            kind: RecipientKind::Repo,
            value: value.to_string(),
        });
        ev
    }

    fn reader<'a>(repo: &'a str, repo_key: &'a str) -> Reader<'a> {
        Reader {
            workspace: &crate::workspaces::Gate::OPEN,
            session: "me",
            harness: "codex",
            repo: Some(repo),
            repo_key: Some(repo_key),
            channels: &[],
        }
    }

    #[test]
    fn a_repo_note_reaches_every_worktree_of_that_repository() {
        // The defect this pins: `repo:` compared the checkout path by string
        // equality, so a worktree and its primary — which never spell the same
        // — could not reach each other. 1,074 of the 1,081 notes ever addressed
        // to a worktree path died without a single reader.
        let note = repo_note("/srv/app");
        let in_worktree = reader("/srv/app/.wt/feature", "/srv/app");
        let in_primary = reader("/srv/app", "/srv/app");

        assert!(
            note.is_addressed_to_reader(&in_worktree),
            "a note to the repository reaches an agent in its worktree"
        );
        assert!(note.is_addressed_to_reader(&in_primary));
    }

    #[test]
    fn a_note_addressed_to_a_worktree_path_still_reaches_that_worktree() {
        // Backward compatibility: every note written before this change stored
        // the worktree path. The session sitting in it must keep receiving them.
        let note = repo_note("/srv/app/.wt/feature");
        assert!(note.is_addressed_to_reader(&reader("/srv/app/.wt/feature", "/srv/app")));
    }

    #[test]
    fn a_repo_note_stops_at_the_repository_boundary() {
        let note = repo_note("/srv/app");
        assert!(!note.is_addressed_to_reader(&reader("/srv/other", "/srv/other")));
        assert!(
            !note.is_addressed_to_reader(&Reader {
                workspace: &crate::workspaces::Gate::OPEN,
                session: "me",
                harness: "codex",
                repo: None,
                repo_key: None,
                channels: &[],
            }),
            "a reader outside any repository is not in one"
        );
    }

    #[test]
    fn the_repository_of_an_event_falls_back_to_its_checkout() {
        let mut ev = sample("hi");
        ev.repo = Some("/srv/app".to_string());
        assert_eq!(ev.repository(), Some("/srv/app"));
        ev.repo = Some("/srv/app/.wt/feature".to_string());
        ev.repo_key = Some("/srv/app".to_string());
        assert_eq!(ev.repository(), Some("/srv/app"));
    }

    #[test]
    fn the_repository_key_stays_off_the_wire_when_it_adds_nothing() {
        let ev = sample("hi");
        assert!(!ev.to_line().unwrap().contains("repo_key"));
    }

    #[test]
    fn an_ask_interrupts_and_is_owed_an_answer() {
        assert!(NoteKind::Ask.interrupts());
        assert!(NoteKind::Ask.owes_an_answer());
        assert!(NoteKind::Handoff.owes_an_answer());
        // Told, not asked.
        assert!(!NoteKind::Warning.owes_an_answer());
        assert!(!NoteKind::Fyi.owes_an_answer());
    }

    #[test]
    fn an_ask_round_trips_and_an_older_build_reads_it_quietly() {
        let mut ev = sample("report your progress");
        ev.kind = NoteKind::Ask;
        let line = ev.to_line().unwrap();
        assert!(line.contains(r#""kind":"ask""#));
        let back: Event = serde_json::from_str(&line).unwrap();
        assert_eq!(back.kind, NoteKind::Ask);

        // A build that predates this kind must not choke on the line: two
        // devices are paired on this LAN and one can be older. It decodes to
        // Unknown, which does not interrupt — a stranger never gains the right
        // to cut into a session it cannot name.
        let older: NoteKind = serde_json::from_str(r#""ask""#).unwrap();
        assert_eq!(older, NoteKind::Ask);
        let stranger: NoteKind = serde_json::from_str(r#""something-new""#).unwrap();
        assert_eq!(stranger, NoteKind::Unknown);
        assert!(!stranger.interrupts());
        assert!(!stranger.owes_an_answer());
    }
}
