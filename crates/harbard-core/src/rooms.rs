//! Rooms on the bus: which repositories the log has seen, the lifecycle
//! events, and reading a room's whole history.
//!
//! ## Why the repositories are indexed once
//!
//! A project channel is permanent, but the hub's memory holds only the last
//! `retention_days` of events and a read of every segment on each request
//! would cost what the store is worth. So the whole log is read once per
//! process, and after that only what was appended since ([`Store::read_since`]).
//!
//! Sessions are not the only evidence: an agent can write in a repository no
//! session of the bus ever sat in. The team view that places those agents
//! reaches back one day, so what it found is kept in a small ledger
//! (`project_repos.json`), written on serve's tick and never on a read.
//!
//! ## Nothing here reads a repository
//!
//! The index holds what the log said. Whether a path is a real repository,
//! and whether it may be looked at, stays the resolver's decision in serve.

use crate::clock::rfc3339_utc;
use crate::event::{Event, EventKind, NoteKind, Recipient, RecipientKind, Role, SCHEMA_VERSION};
use crate::membership::{self, Memberships};
use crate::query::event_ms;
use crate::store::{HistoryError, ReadPosition, ReadResult, Store, Window, read_segment};
use crate::workspace::RoomInput;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use ulid::Ulid;

/// Most messages one page of a room's history can hold.
pub const HISTORY_PAGE_MAX: usize = 200;

/// A placement older than this is written again: a ledger rewritten on every
/// tick would be churn for a number nobody reads closely.
const LEDGER_BUMP_MS: u64 = 3_600_000;

/// A repository the bus has seen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownRepo {
    /// Its identity: the primary checkout's path.
    pub repo: String,
    pub first_ms: u64,
    pub last_ms: u64,
    /// The machine it was last seen from, which places a satellite's
    /// checkout this hub cannot read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

#[derive(Debug, Default)]
struct RepoIndex {
    repos: BTreeMap<String, KnownRepo>,
    position: Option<ReadPosition>,
    scanned: bool,
}

impl RepoIndex {
    fn fold(&mut self, read: &ReadResult) {
        for event in &read.events {
            if event.is_operator() || event.role == Role::Operator {
                continue;
            }
            let Some(repo) = event.repository().filter(|r| r.starts_with('/')) else {
                continue;
            };
            let at = event_ms(event);
            self.note(repo, at, at, Some(event.host.as_str()));
        }
    }

    fn note(&mut self, repo: &str, first_ms: u64, last_ms: u64, host: Option<&str>) {
        let host = host.filter(|h| !h.is_empty()).map(str::to_string);
        match self.repos.get_mut(repo) {
            Some(known) => {
                known.first_ms = known.first_ms.min(first_ms);
                known.last_ms = known.last_ms.max(last_ms);
                if host.is_some() {
                    known.host = host;
                }
            }
            None => {
                self.repos.insert(
                    repo.to_string(),
                    KnownRepo {
                        repo: repo.to_string(),
                        first_ms,
                        last_ms,
                        host,
                    },
                );
            }
        }
    }
}

type Indexes = Mutex<HashMap<PathBuf, RepoIndex>>;

fn indexes() -> &'static Indexes {
    static INDEXES: OnceLock<Indexes> = OnceLock::new();
    INDEXES.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Every repository the bus has seen: the whole log once, then only its tail.
/// Requests that arrive during the first read wait for it rather than show a
/// channel list that is missing most of its channels.
pub fn known_repos(store: &Store) -> Vec<KnownRepo> {
    let mut all = indexes().lock().unwrap_or_else(|e| e.into_inner());
    let index = all.entry(store.root().to_path_buf()).or_default();
    let (tail, position) = store.read_since(index.position.as_ref());
    if !index.scanned {
        index.fold(&store.read(Window::All));
        for known in read_ledger(store) {
            index.note(
                &known.repo,
                known.first_ms,
                known.last_ms,
                known.host.as_deref(),
            );
        }
        index.scanned = true;
    }
    index.fold(&tail);
    index.position = position;
    index.repos.values().cloned().collect()
}

fn ledger_path(store: &Store) -> PathBuf {
    store.root().join("project_repos.json")
}

fn read_ledger(store: &Store) -> Vec<KnownRepo> {
    fs::read_to_string(ledger_path(store))
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Remember the repositories the view placed agents in: `(repository, newest
/// activity)`. Called from serve's tick. Writes only when something new
/// would be lost.
pub fn record_placements(store: &Store, placements: &[(String, u64)]) -> std::io::Result<()> {
    let mut ledger: BTreeMap<String, KnownRepo> = read_ledger(store)
        .into_iter()
        .map(|known| (known.repo.clone(), known))
        .collect();
    let mut changed = false;
    for (repo, ms) in placements {
        if !repo.starts_with('/') {
            continue;
        }
        match ledger.get_mut(repo) {
            Some(known) if ms.saturating_sub(known.last_ms) >= LEDGER_BUMP_MS => {
                known.last_ms = *ms;
                changed = true;
            }
            Some(_) => {}
            None => {
                ledger.insert(
                    repo.clone(),
                    KnownRepo {
                        repo: repo.clone(),
                        first_ms: *ms,
                        last_ms: *ms,
                        host: None,
                    },
                );
                changed = true;
            }
        }
    }
    {
        let mut all = indexes().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(index) = all.get_mut(store.root()) {
            for (repo, ms) in placements {
                if repo.starts_with('/') {
                    index.note(repo, *ms, *ms, None);
                }
            }
        }
    }
    if !changed {
        return Ok(());
    }
    fs::create_dir_all(store.root())?;
    let rows: Vec<&KnownRepo> = ledger.values().collect();
    let json = serde_json::to_string_pretty(&rows)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    let path = ledger_path(store);
    let tmp = path.with_extension(format!("json.tmp.{}", std::process::id()));
    fs::write(&tmp, json.as_bytes())?;
    fs::rename(&tmp, path)
}

// ---------------------------------------------------------------------------
// Lifecycle events
// ---------------------------------------------------------------------------

/// What happened to a channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LifecycleAction {
    Create,
    Update,
    Archive,
    Unarchive,
    Delete,
}

impl LifecycleAction {
    pub fn as_str(self) -> &'static str {
        match self {
            LifecycleAction::Create => "create",
            LifecycleAction::Update => "update",
            LifecycleAction::Archive => "archive",
            LifecycleAction::Unarchive => "unarchive",
            LifecycleAction::Delete => "delete",
        }
    }
}

/// The JSON a `channel_lifecycle` event carries in `text`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleBody {
    pub action: LifecycleAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// The repository of a project channel, whose id can change.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<String>,
}

/// Who did it, as the event will say.
#[derive(Debug, Clone, Default)]
pub struct LifecycleActor {
    pub session: String,
    pub harness: String,
    pub host: String,
    pub cwd: String,
    pub repo: Option<String>,
    pub repo_key: Option<String>,
    pub branch: Option<String>,
    pub role: Role,
}

impl LifecycleActor {
    /// The operator of this hub.
    pub fn operator() -> Self {
        let host = crate::hostname();
        LifecycleActor {
            session: format!("operator@{host}"),
            harness: crate::HARNESS_HUMAN.to_string(),
            host,
            role: Role::Operator,
            ..Default::default()
        }
    }
}

/// One channel change, to append to the log.
#[derive(Debug, Clone, Copy)]
pub struct Lifecycle<'a> {
    pub action: LifecycleAction,
    pub channel: &'a str,
    /// The workspace the operator did it in; ignored for anyone else, who is
    /// placed by their repository.
    pub workspace: Option<&'a str>,
    pub purpose: Option<&'a str>,
    /// The repository, when the channel is a project's.
    pub repo: Option<&'a str>,
}

/// Append a `channel_lifecycle` event. The CLI's channel verbs call this.
pub fn append_lifecycle(
    store: &Store,
    actor: &LifecycleActor,
    change: &Lifecycle<'_>,
    now_ms: u64,
) -> std::io::Result<Event> {
    let body = LifecycleBody {
        action: change.action,
        purpose: change.purpose.map(str::to_string),
        repo: change.repo.map(str::to_string),
    };
    let text = serde_json::to_string(&body)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    let cwd = match change.workspace {
        Some(id) if actor.role == Role::Operator => {
            format!("{}{id}", crate::event::WORKSPACE_STAMP)
        }
        _ => actor.cwd.clone(),
    };
    let event = Event {
        v: SCHEMA_VERSION,
        // The id carries the time every reader sorts by.
        id: Ulid::from_parts(now_ms, Ulid::generate().random()).to_string(),
        ts: rfc3339_utc(now_ms),
        event: EventKind::ChannelLifecycle,
        session: actor.session.clone(),
        harness: actor.harness.clone(),
        host: actor.host.clone(),
        cwd,
        repo: actor.repo.clone(),
        repo_key: actor.repo_key.clone(),
        branch: actor.branch.clone(),
        role: actor.role,
        parent: None,
        note_ref: None,
        text,
        to: Some(Recipient {
            kind: RecipientKind::Channel,
            value: membership::normalize(change.channel),
        }),
        kind: NoteKind::Fyi,
    };
    store.append(&event)?;
    Ok(event)
}

/// The action of a lifecycle event, when its body decodes.
pub fn lifecycle_of(event: &Event) -> Option<LifecycleBody> {
    if event.event != EventKind::ChannelLifecycle {
        return None;
    }
    serde_json::from_str(&event.text).ok()
}

// ---------------------------------------------------------------------------
// The rooms of one workspace
// ---------------------------------------------------------------------------

/// The rooms a workspace shows: the three built-ins, always, then every named
/// room `belongs` places here. `workspace` is `None` on a hub with one.
pub fn rooms_in_workspace(
    m: &Memberships,
    workspace: Option<&str>,
    belongs: &dyn Fn(&str) -> bool,
) -> Vec<RoomInput> {
    let mut out: Vec<RoomInput> = membership::BUILT_IN_ROOMS
        .iter()
        .map(|slug| {
            let (purpose, code) = membership::built_in_purpose(slug).unwrap_or(("", ""));
            RoomInput {
                slug: (*slug).to_string(),
                purpose: Some(purpose.to_string()),
                purpose_code: Some(code),
                archived: false,
                built_in: true,
                created_ms: None,
                last_note_ms: None,
                read: m.reads.get(&membership::read_key(slug, workspace)).cloned(),
            }
        })
        .collect();
    for slug in m.channels.keys() {
        if membership::is_built_in(slug) || !belongs(slug) {
            continue;
        }
        let meta = m.meta.get(slug).cloned().unwrap_or_default();
        out.push(RoomInput {
            slug: slug.clone(),
            purpose: meta.purpose,
            purpose_code: None,
            archived: m.archived.contains(slug),
            built_in: false,
            created_ms: meta.created_ms,
            last_note_ms: meta.last_note_ms,
            read: m.reads.get(slug).cloned(),
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Reading a room
// ---------------------------------------------------------------------------

/// True for a note written to the room `slug`.
pub fn is_note_in(event: &Event, slug: &str) -> bool {
    event.event == EventKind::Note
        && event
            .to
            .as_ref()
            .is_some_and(|to| to.kind == RecipientKind::Channel && to.value == slug)
}

/// One page of a room, oldest first.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoomPage {
    pub notes: Vec<Event>,
    /// Pass as `before` for the page older than this one.
    pub next_before: Option<String>,
}

/// A page of the notes in room `slug` that `keep` admits, newest page first.
///
/// Walks every segment from the newest back and stops as soon as the page is
/// full plus one, so an old room costs what it costs and a recent one little.
/// Archived and deleted rooms read the same: the log is the room.
pub fn room_history(
    store: &Store,
    slug: &str,
    keep: &dyn Fn(&Event) -> bool,
    before: Option<&str>,
    limit: usize,
) -> Result<RoomPage, HistoryError> {
    let limit = limit.clamp(1, HISTORY_PAGE_MAX);
    let mut newest_first: Vec<Event> = Vec::with_capacity(limit + 1);
    let mut collecting = before.is_none();
    'segments: for segment in store.segments().into_iter().rev() {
        let mut page = ReadResult::default();
        if read_segment(&segment, &mut page).is_err() {
            continue;
        }
        for event in page.events.into_iter().rev() {
            if !collecting {
                collecting = before == Some(event.id.as_str());
                continue;
            }
            if is_note_in(&event, slug) && keep(&event) {
                newest_first.push(event);
                if newest_first.len() > limit {
                    break 'segments;
                }
            }
        }
    }
    if !collecting {
        return Err(HistoryError::CursorExpired {
            kind: "before",
            id: before.unwrap_or_default().to_string(),
        });
    }
    let more = newest_first.len() > limit;
    newest_first.truncate(limit);
    let next_before = if more {
        newest_first.last().map(|e| e.id.clone())
    } else {
        None
    };
    newest_first.reverse();
    Ok(RoomPage {
        notes: newest_first,
        next_before,
    })
}

/// A room the operator deleted, whose messages are still on the log.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeletedRoom {
    pub name: String,
    pub deleted_ms: u64,
    pub messages: usize,
}

/// Every room whose last lifecycle event `keep` admits is a delete, newest
/// first, with how many of its notes `keep` admits. A room deleted before
/// lifecycle events existed left no record and is not listed.
pub fn deleted_rooms(store: &Store, keep: &dyn Fn(&Event) -> bool) -> Vec<DeletedRoom> {
    let read = store.read(Window::All);
    let mut last: BTreeMap<String, (LifecycleAction, u64)> = BTreeMap::new();
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for event in read.events.iter().filter(|e| keep(e)) {
        let Some(to) = event
            .to
            .as_ref()
            .filter(|t| t.kind == RecipientKind::Channel)
        else {
            continue;
        };
        if let Some(body) = lifecycle_of(event) {
            last.insert(to.value.clone(), (body.action, event_ms(event)));
        } else if event.event == EventKind::Note {
            *counts.entry(to.value.clone()).or_default() += 1;
        }
    }
    let mut out: Vec<DeletedRoom> = last
        .into_iter()
        .filter(|(_, (action, _))| *action == LifecycleAction::Delete)
        .map(|(name, (_, deleted_ms))| DeletedRoom {
            messages: counts.get(&name).copied().unwrap_or(0),
            name,
            deleted_ms,
        })
        .collect();
    out.sort_by(|a, b| b.deleted_ms.cmp(&a.deleted_ms).then(a.name.cmp(&b.name)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::NoteKind;
    use crate::test_util::TempHome;

    fn event(id_ms: u64, session: &str, repo: Option<&str>, to: Option<&str>) -> Event {
        Event {
            v: SCHEMA_VERSION,
            id: Ulid::from_parts(id_ms, id_ms as u128).to_string(),
            ts: rfc3339_utc(id_ms),
            event: EventKind::Note,
            session: session.into(),
            harness: "claude-code".into(),
            host: "h".into(),
            cwd: repo.unwrap_or("").into(),
            repo: repo.map(str::to_string),
            repo_key: None,
            branch: None,
            role: Role::Main,
            parent: None,
            note_ref: None,
            text: format!("note {id_ms}"),
            to: to.map(|value| Recipient {
                kind: RecipientKind::Channel,
                value: value.into(),
            }),
            kind: NoteKind::Fyi,
        }
    }

    #[test]
    fn the_index_keeps_every_repository_and_then_follows_the_tail() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        store
            .append(&event(1_000, "a", Some("/w/old"), None))
            .unwrap();
        store.append(&event(2_000, "b", None, None)).unwrap();
        let first = known_repos(&store);
        assert_eq!(first.len(), 1);
        assert_eq!((first[0].first_ms, first[0].last_ms), (1_000, 1_000));

        store
            .append(&event(3_000, "c", Some("/w/new"), None))
            .unwrap();
        store
            .append(&event(4_000, "a", Some("/w/old"), None))
            .unwrap();
        let next = known_repos(&store);
        assert_eq!(
            next.iter().map(|k| k.repo.as_str()).collect::<Vec<_>>(),
            vec!["/w/new", "/w/old"]
        );
        assert_eq!(next[1].last_ms, 4_000, "the tail moved it forward");
    }

    #[test]
    fn an_operator_event_is_no_evidence_of_a_repository() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let mut op = event(1_000, "operator@h", Some("/w/typed"), None);
        op.harness = crate::HARNESS_HUMAN.into();
        op.role = Role::Operator;
        store.append(&op).unwrap();
        assert!(known_repos(&store).is_empty());
    }

    #[test]
    fn a_placement_survives_in_the_ledger_and_only_writes_when_it_matters() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        record_placements(&store, &[("/w/agent-only".into(), 5_000)]).unwrap();
        let written = fs::metadata(ledger_path(&store))
            .unwrap()
            .modified()
            .unwrap();

        // A later process: nothing in memory, the ledger answers.
        indexes().lock().unwrap().remove(store.root());
        assert_eq!(known_repos(&store)[0].repo, "/w/agent-only");

        // A small advance is not worth a write.
        record_placements(&store, &[("/w/agent-only".into(), 6_000)]).unwrap();
        assert_eq!(
            fs::metadata(ledger_path(&store))
                .unwrap()
                .modified()
                .unwrap(),
            written
        );
    }

    #[test]
    fn history_pages_backwards_and_reads_oldest_first() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        for ms in 1..=5u64 {
            store
                .append(&event(ms * 1_000, "a", Some("/w/r"), Some("ops")))
                .unwrap();
        }
        store
            .append(&event(9_000, "a", Some("/w/r"), Some("elsewhere")))
            .unwrap();

        let page = room_history(&store, "ops", &|_| true, None, 2).unwrap();
        assert_eq!(
            page.notes.iter().map(event_ms).collect::<Vec<_>>(),
            vec![4_000, 5_000]
        );
        let older = room_history(&store, "ops", &|_| true, page.next_before.as_deref(), 2).unwrap();
        assert_eq!(
            older.notes.iter().map(event_ms).collect::<Vec<_>>(),
            vec![2_000, 3_000]
        );
        let last = room_history(&store, "ops", &|_| true, older.next_before.as_deref(), 2).unwrap();
        assert_eq!(last.notes.len(), 1);
        assert_eq!(last.next_before, None);

        assert!(matches!(
            room_history(
                &store,
                "ops",
                &|_| true,
                Some("01ZZZZZZZZZZZZZZZZZZZZZZZZ"),
                2
            ),
            Err(HistoryError::CursorExpired { .. })
        ));
    }

    #[test]
    fn a_deleted_room_is_listed_until_it_comes_back_and_keeps_its_messages() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        store
            .append(&event(1_000, "a", Some("/w/r"), Some("old-room")))
            .unwrap();
        let actor = LifecycleActor::operator();
        let change = |action, channel| Lifecycle {
            action,
            channel,
            workspace: None,
            purpose: None,
            repo: None,
        };
        append_lifecycle(
            &store,
            &actor,
            &change(LifecycleAction::Create, "old-room"),
            2_000,
        )
        .unwrap();
        append_lifecycle(
            &store,
            &actor,
            &change(LifecycleAction::Delete, "old-room"),
            3_000,
        )
        .unwrap();
        append_lifecycle(
            &store,
            &actor,
            &change(LifecycleAction::Delete, "back"),
            4_000,
        )
        .unwrap();
        append_lifecycle(
            &store,
            &actor,
            &change(LifecycleAction::Create, "back"),
            5_000,
        )
        .unwrap();

        let deleted = deleted_rooms(&store, &|_| true);
        assert_eq!(
            deleted,
            vec![DeletedRoom {
                name: "old-room".into(),
                deleted_ms: 3_000,
                messages: 1,
            }]
        );
        let page = room_history(&store, "old-room", &|_| true, None, 10).unwrap();
        assert_eq!(page.notes.len(), 1, "deleting a room keeps what was said");
    }

    #[test]
    fn a_lifecycle_event_round_trips_under_its_wire_name() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let event = append_lifecycle(
            &store,
            &LifecycleActor::operator(),
            &Lifecycle {
                action: LifecycleAction::Archive,
                channel: "#Ops",
                workspace: Some("acme"),
                purpose: None,
                repo: None,
            },
            1_000,
        )
        .unwrap();
        let line = event.to_line().unwrap();
        assert!(line.contains(r#""event":"channel_lifecycle""#), "{line}");
        let back: Event = serde_json::from_str(&line).unwrap();
        assert_eq!(back.event, EventKind::ChannelLifecycle);
        assert_eq!(back.workspace_stamp(), Some("acme"));
        assert_eq!(back.to.as_ref().unwrap().value, "ops");
    }

    fn two_workspaces() -> crate::workspaces::Placement {
        use crate::workspaces::{Placement, RepoFacts, Workspaces};
        let mut set = Workspaces::single();
        set.workspaces.clear();
        set.add("personal", "Personal", &["me".into()]).unwrap();
        set.add("acme", "Acme", &["Acme".into()]).unwrap();
        set.default = "personal".into();
        Placement::with(Some(set), |path| RepoFacts {
            root: Some(path.to_string()),
            owner: Some(
                if path.starts_with("/acme") {
                    "Acme"
                } else {
                    "me"
                }
                .to_string(),
            ),
        })
    }

    fn operator_note(room: &str, workspace: Option<&str>) -> Event {
        let mut note = event(1_000, "operator@h", None, Some(room));
        note.harness = crate::HARNESS_HUMAN.into();
        note.role = Role::Operator;
        if let Some(id) = workspace {
            note.cwd = format!("{}{id}", crate::event::WORKSPACE_STAMP);
        }
        note
    }

    fn reader<'a>(
        repo: &'a str,
        channels: &'a [String],
        gate: &'a crate::workspaces::Gate<'a>,
    ) -> crate::event::Reader<'a> {
        crate::event::Reader {
            session: "reader",
            harness: "claude-code",
            repo: Some(repo),
            repo_key: Some(repo),
            channels,
            workspace: gate,
        }
    }

    /// `Gate::admits` lets the operator through on purpose, for notes that the
    /// console already aimed at one session. A room is addressed by name, and
    /// the same name exists in every workspace, so without a stamp that rule
    /// would carry a room note across the boundary.
    #[test]
    fn an_operator_note_in_a_room_stays_in_the_workspace_it_was_written_in() {
        let placement = two_workspaces();
        let joined = [membership::LOBBY.to_string()];
        let note = operator_note(membership::LOBBY, Some("acme"));

        let acme = placement.gate_of("acme");
        let mine = placement.gate_of("personal");
        assert!(note.is_addressed_to_reader(&reader("/acme/app", &joined, &acme)));
        assert!(
            !note.is_addressed_to_reader(&reader("/me/tool", &joined, &mine)),
            "the same-named room of the other workspace must not hear it"
        );
        assert!(acme.places(&note));
        assert!(!mine.places(&note));
        assert_eq!(placement.of_event(&note), "acme");
    }

    #[test]
    fn an_unstamped_operator_room_note_belongs_to_the_default_workspace_only() {
        let placement = two_workspaces();
        let note = operator_note("ops", None);
        assert!(placement.gate_of("personal").places(&note));
        assert!(!placement.gate_of("acme").places(&note));
    }

    #[test]
    fn only_the_operator_can_stamp_a_workspace() {
        let placement = two_workspaces();
        let mut forged = event(1_000, "agent", Some("/me/tool"), Some("ops"));
        forged.cwd = format!("{}acme", crate::event::WORKSPACE_STAMP);
        assert_eq!(forged.workspace_stamp(), None);
        assert_eq!(placement.of_event(&forged), "personal");
    }

    /// Step 6 of the rooms contract: the built-in rooms are where agents talk
    /// among themselves, so nothing said there may reach a session that did
    /// not ask for it, by any delivery path, however urgent its kind.
    #[test]
    fn the_built_in_rooms_never_wake_or_brief_a_session_that_did_not_join() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let open = crate::workspaces::Gate::OPEN;
        let now = 10_000;
        for room in membership::BUILT_IN_ROOMS {
            for kind in [
                NoteKind::Fyi,
                NoteKind::Warning,
                NoteKind::Ask,
                NoteKind::Handoff,
            ] {
                let mut note = event(1_000, "someone", Some("/w/r"), Some(room));
                note.kind = kind;
                note.text = format!("said in {room}");
                let evs = vec![note.clone()];

                // Same checkout, same harness, nothing joined.
                let none: [String; 0] = [];
                let outsider = reader("/w/r", &none, &open);
                assert!(!note.is_addressed_to_reader(&outsider), "{room} {kind:?}");
                assert!(!crate::wake::calls_for(&note, &outsider), "{room} {kind:?}");
                let inline = crate::brief::render_inline(
                    &evs,
                    "reader",
                    "claude-code",
                    Some("/w/r"),
                    Some("/w/r"),
                    &none,
                    None,
                    None,
                    now,
                    &open,
                );
                assert!(inline.text.is_none(), "{room} {kind:?} entered the inline");
                let brief = crate::brief::render(
                    &store,
                    &evs,
                    "reader",
                    "claude-code",
                    Some("/w/r"),
                    Some("/w/r"),
                    &none,
                    None,
                    now,
                    86_400,
                    &std::collections::HashSet::new(),
                    &open,
                );
                assert!(
                    !brief.text.clone().unwrap_or_default().contains(&note.text),
                    "{room} {kind:?} entered the brief: {:?}",
                    brief.text
                );

                // Having joined, it behaves like any other room.
                let joined = [room.to_string()];
                let member = reader("/w/r", &joined, &open);
                assert!(note.is_addressed_to_reader(&member), "{room} {kind:?}");
                let brief = crate::brief::render(
                    &store,
                    &evs,
                    "reader",
                    "claude-code",
                    Some("/w/r"),
                    Some("/w/r"),
                    &joined,
                    None,
                    now,
                    86_400,
                    &std::collections::HashSet::new(),
                    &open,
                );
                assert!(
                    brief.text.unwrap_or_default().contains(&note.text),
                    "{room} {kind:?}"
                );
                assert_eq!(
                    crate::wake::calls_for(&note, &member),
                    matches!(kind, NoteKind::Warning | NoteKind::Ask),
                    "{room} {kind:?}"
                );
            }
        }
    }

    #[test]
    fn a_room_list_has_the_built_ins_and_only_what_belongs_here() {
        let mut m = Memberships::default();
        membership::create(&mut m, "here");
        membership::create(&mut m, "there");
        m.archived.insert("here".into());
        let rooms = rooms_in_workspace(&m, Some("acme"), &|slug| slug == "here");
        let slugs: Vec<&str> = rooms.iter().map(|r| r.slug.as_str()).collect();
        assert_eq!(slugs, vec!["lobby", "commons", "feedback", "here"]);
        assert!(rooms[0].built_in && !rooms[0].archived);
        assert_eq!(rooms[0].purpose_code, Some("room_purpose_lobby"));
        assert!(rooms[3].archived);
    }
}
