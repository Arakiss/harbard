//! Durable local dependencies and a bounded, non-executing CLI listener.

use crate::cli::{Common, DependencyCmd};
use crate::{Context, build_event, clock, fail, print_stdout};
use fs2::FileExt;
use harbard_core::dependency::{
    self, DependencyAction, DependencyError, DependencyOutcome, DependencyProjection,
    DependencyState, DependencyView,
};
use harbard_core::{Event, EventKind, Store};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::fs::{self, File, OpenOptions};
use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::time::{Duration, Instant};

pub(crate) fn local_only(store: &Store) -> Result<(), String> {
    let hub_configured = std::env::var("HARBARD_HUB").is_ok_and(|value| !value.trim().is_empty());
    let paired = match fs::metadata(store.root().join("satellite.json")) {
        Ok(_) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => return Err(format!("cannot inspect hub configuration: {error}")),
    };
    if hub_configured || paired {
        Err(
            "dependencies require a local shared store; remote hub transport is not supported — ask instead: harbard note --kind ask --to session:<id> \"…\""
                .into(),
        )
    } else {
        Ok(())
    }
}

fn output(value: &impl Serialize) -> i32 {
    match serde_json::to_string_pretty(value) {
        Ok(text) => {
            print_stdout(&text);
            0
        }
        Err(error) => crate::internal_failure("dependency", error),
    }
}

/// A dependency error, said with the step that follows from it. Broken
/// storage is harbard's fault; everything else is a refusal of this request.
fn fail_dependency(id: Option<&str>, error: DependencyError) -> i32 {
    let target = id.unwrap_or("<id>");
    match error {
        DependencyError::Io(_) | DependencyError::CorruptHistory { .. } => {
            crate::internal_failure("dependency", error)
        }
        DependencyError::Unknown(_) => fail(format!(
            "{error}; `harbard dependency list --all` shows them, with their full ids"
        )),
        DependencyError::Unauthorized(_) | DependencyError::Conflict(_) => fail(format!(
            "{error}; `harbard dependency show {target}` shows who holds it and its state"
        )),
        DependencyError::Invalid(_) => fail(format!(
            "{error}; `harbard dependency --help` has the expected form"
        )),
    }
}

pub(crate) fn cmd_dependency(store: &Store, common: &Common, action: &DependencyCmd) -> i32 {
    if let Err(error) = local_only(store) {
        return fail(error);
    }
    let ctx = Context::resolve(common);
    let now = clock::now_ms();
    let mutation = match action {
        DependencyCmd::Open {
            to,
            condition,
            continuation,
            deadline_secs,
        } => {
            let Some(responder) = to
                .strip_prefix("session:")
                .filter(|id| !id.trim().is_empty() && !id.contains(char::is_whitespace))
            else {
                return fail(
                    "dependency requires --to session:<exact-session-id>; `harbard ls` lists the sessions",
                );
            };
            if responder == ctx.session {
                return fail(
                    "dependency responder must be another session; name the peer that owes the result, from `harbard ls`",
                );
            }
            DependencyAction::Declare {
                responder: responder.into(),
                condition: condition.clone(),
                continuation: continuation.clone(),
                deadline_ms: deadline_secs
                    .map(|seconds| now.saturating_add(seconds.saturating_mul(1000))),
            }
        }
        DependencyCmd::Resolve {
            id,
            outcome,
            evidence,
        } => DependencyAction::Resolve {
            id: id.clone(),
            outcome: match outcome.as_str() {
                "satisfied" => DependencyOutcome::Satisfied,
                "failed" => DependencyOutcome::Failed,
                _ => return fail("outcome must be satisfied or failed"),
            },
            evidence: evidence.clone(),
        },
        DependencyCmd::Cancel { id, reason } => DependencyAction::Cancel {
            id: id.clone(),
            reason: reason.clone(),
        },
        DependencyCmd::Consume { id } => DependencyAction::Consume { id: id.clone() },
        DependencyCmd::Show { id } => {
            return match dependency::get(store, id) {
                Ok(view) => {
                    let active = listener_active(store, id);
                    let mut value = json!(view);
                    value["active_cli_wait"] = json!(active);
                    value["automatic_resume"] = json!(false);
                    output(&value)
                }
                Err(error) => fail_dependency(Some(id), error),
            };
        }
        DependencyCmd::List { all } => {
            return match dependency::list(store) {
                Ok(views) => output(
                    &views
                        .into_iter()
                        .filter(|view| {
                            *all || view.requester_session == ctx.session
                                || view.responder_session == ctx.session
                        })
                        .collect::<Vec<_>>(),
                ),
                Err(error) => fail_dependency(None, error),
            };
        }
    };
    let id = match action {
        DependencyCmd::Resolve { id, .. }
        | DependencyCmd::Cancel { id, .. }
        | DependencyCmd::Consume { id } => Some(id.as_str()),
        _ => None,
    };
    let event = build_event(&ctx, EventKind::Dependency, "", None, now);
    match dependency::append_action(store, event, mutation) {
        Ok(view) => output(&view),
        Err(error) => fail_dependency(id, error),
    }
}

fn waiter_file(store: &Store, id: &str) -> Result<File, String> {
    if !harbard_core::is_event_id(id) {
        return Err("dependency ID must be a full event ID".into());
    }
    let directory = store.root().join("dependency-waiters");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join(format!("{id}.lock")))
        .map_err(|error| error.to_string())
}

pub(crate) fn listener_active(store: &Store, id: &str) -> bool {
    if !harbard_core::is_event_id(id) {
        return false;
    }
    let Ok(file) = OpenOptions::new().read(true).write(true).open(
        store
            .root()
            .join("dependency-waiters")
            .join(format!("{id}.lock")),
    ) else {
        return false;
    };
    match file.try_lock_exclusive() {
        Ok(()) => false,
        Err(error) => error.kind() == std::io::ErrorKind::WouldBlock,
    }
}

pub(crate) fn cmd_await(
    store: &Store,
    common: &Common,
    id: &str,
    timeout_secs: u64,
    poll_ms: u64,
) -> i32 {
    if let Err(error) = local_only(store) {
        return fail(error);
    }
    let ctx = Context::resolve(common);
    let mut reader = match DependencyReader::load(store) {
        Ok(reader) => reader,
        Err(error) => return fail(error),
    };
    let view = match reader.projection.get(id) {
        Ok(view) => view,
        Err(DependencyError::Unknown(_)) => {
            return fail(unknown_await_target(store, &reader.projection, id));
        }
        Err(error) => return fail(error),
    };
    if view.requester_session != ctx.session {
        return fail(format!(
            "only the requester can await this dependency; `harbard dependency show {id}` names it, and the responder settles it with `harbard dependency resolve {id}`"
        ));
    }
    if view.consumed {
        return fail("dependency was already consumed; inspect it with dependency show");
    }
    let listener = match waiter_file(store, id) {
        Ok(file) => file,
        Err(error) => return fail(error),
    };
    if let Err(error) = listener.try_lock_exclusive() {
        return fail(format!(
            "another CLI listener owns this dependency: {error}; end your turn and harbard wakes you when the result lands"
        ));
    }
    let start = Instant::now();
    loop {
        if let Err(error) = reader.refresh(store) {
            return fail(error);
        }
        let view = match reader.projection.get(id) {
            Ok(view) => view,
            Err(error) => return fail(error),
        };
        if view.consumed {
            return fail("dependency was already consumed; inspect it with dependency show");
        }
        let (outcome, code) = match view.state {
            DependencyState::Satisfied => ("satisfied", 0),
            DependencyState::Failed => ("failed", 4),
            DependencyState::Cancelled => ("cancelled", 4),
            DependencyState::Pending if start.elapsed() >= Duration::from_secs(timeout_secs) => {
                ("timeout", 3)
            }
            DependencyState::Pending => {
                std::thread::sleep(
                    Duration::from_millis(poll_ms)
                        .min(Duration::from_secs(timeout_secs).saturating_sub(start.elapsed())),
                );
                continue;
            }
        };
        output(
            &json!({"dependency": view, "outcome": outcome, "elapsed_ms": start.elapsed().as_millis(), "automatic_resume": false}),
        );
        return code;
    }
}

fn unknown_await_target(store: &Store, projection: &DependencyProjection, id: &str) -> String {
    let durable_wait = "For a durable wait, run `harbard dependency open --to session:<id> --condition '...' --resume '...'`.";
    let dependency_prefix = projection.list().iter().any(|view| view.id.starts_with(id));
    if harbard_core::id::is_event_prefix(id) && !dependency_prefix {
        let read = store.read(harbard_core::Window::All);
        match harbard_core::id::event(&read.events, id, EventKind::Note) {
            Ok(note) => {
                return format!(
                    "`harbard await` requires a full dependency ID, not a note ID. Use `harbard thread {}` to inspect replies. {durable_wait}",
                    note.id
                );
            }
            Err(harbard_core::id::ResolveIdError::Ambiguous(count)) => {
                return format!(
                    "`harbard await` requires a full dependency ID; this prefix matches {count} notes. Use `harbard thread <full-note-id>` to inspect replies. {durable_wait}"
                );
            }
            Err(harbard_core::id::ResolveIdError::Unknown) => {}
        }
    }
    format!(
        "unknown dependency: `harbard await` requires the full dependency ID returned by `harbard dependency open`. Use `harbard dependency list` to find it, then `harbard await <full-dependency-id>`. {durable_wait}"
    )
}

/// File identity and metadata are cheap to compare across the complete
/// inventory. The current segment may grow; older segments must stay intact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct SegmentStamp {
    name: String,
    device: u64,
    inode: u64,
    length: u64,
    modified: (i64, i64),
    changed: (i64, i64),
}

impl SegmentStamp {
    fn new(name: String, metadata: &fs::Metadata) -> Self {
        Self {
            name,
            device: metadata.dev(),
            inode: metadata.ino(),
            length: metadata.len(),
            modified: (metadata.mtime(), metadata.mtime_nsec()),
            changed: (metadata.ctime(), metadata.ctime_nsec()),
        }
    }

    fn same_file(&self, other: &Self) -> bool {
        self.name == other.name && self.device == other.device && self.inode == other.inode
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct HistoryCheckpoint {
    segments: Vec<SegmentStamp>,
    /// Bytes immediately before the exact consumed offset. This also detects
    /// truncation followed by regrowth of the same inode between polls.
    anchor: Vec<u8>,
}

const ANCHOR_BYTES: usize = 256;

fn inventory(store: &Store) -> Result<Vec<SegmentStamp>, String> {
    let entries = match fs::read_dir(store.events_dir()) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(format!("cannot read dependency history: {error}")),
    };
    let mut segments = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.starts_with("events-") || !name.ends_with(".jsonl") {
            continue;
        }
        let metadata = entry.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err(format!("dependency segment {name} is not a file"));
        }
        segments.push(SegmentStamp::new(name, &metadata));
    }
    segments.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(segments)
}

/// Read only bytes within the captured boundary. Store::read_since is a
/// best-effort bus reader: a dependency cannot advance over an I/O failure or
/// checkpoint bytes that raced in after its captured offset.
fn read_range(store: &Store, segment: &SegmentStamp, start: u64) -> Result<Vec<u8>, String> {
    let mut file = File::open(store.events_dir().join(&segment.name))
        .map_err(|error| format!("cannot read dependency segment {}: {error}", segment.name))?;
    let actual = SegmentStamp::new(
        segment.name.clone(),
        &file.metadata().map_err(|e| e.to_string())?,
    );
    if !segment.same_file(&actual) || actual.length < segment.length || start > segment.length {
        return Err("dependency history changed during reading; retry from the log".into());
    }
    let size = usize::try_from(segment.length - start)
        .map_err(|_| "dependency segment is too large to read".to_string())?;
    let mut bytes = vec![0; size];
    file.seek(SeekFrom::Start(start))
        .map_err(|error| error.to_string())?;
    file.read_exact(&mut bytes)
        .map_err(|error| format!("incomplete dependency segment {}: {error}", segment.name))?;
    Ok(bytes)
}

fn parse_dependency_events(bytes: &[u8]) -> Result<Vec<Event>, String> {
    if !bytes.is_empty() && !bytes.ends_with(b"\n") {
        return Err(
            "dependency history has an incomplete JSONL record; retry after the writer finishes"
                .into(),
        );
    }
    let mut events = Vec::new();
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        if line.len() > harbard_core::MAX_LINE_BYTES {
            return Err("dependency history has an oversized record".into());
        }
        let event: Event = serde_json::from_slice(line)
            .map_err(|error| format!("dependency history has an invalid record: {error}"))?;
        if event.event == EventKind::Unknown {
            return Err("dependency history contains an unknown event kind".into());
        }
        if event.event == EventKind::Dependency {
            events.push(event);
        }
    }
    Ok(events)
}

fn tail_anchor(bytes: &[u8]) -> Vec<u8> {
    bytes[bytes.len().saturating_sub(ANCHOR_BYTES)..].to_vec()
}

fn checkpoint_matches(
    store: &Store,
    checkpoint: &HistoryCheckpoint,
    current: &[SegmentStamp],
) -> Result<bool, String> {
    if checkpoint.segments.len() != current.len() {
        return Ok(false);
    }
    let Some((previous, old_segments)) = checkpoint.segments.split_last() else {
        return Ok(true);
    };
    let (latest, earlier) = current.split_last().expect("matching nonempty inventory");
    if old_segments != earlier || !previous.same_file(latest) || latest.length < previous.length {
        return Ok(false);
    }
    // A same-length mutation is not an append, even when its tail is unchanged.
    if previous.length == latest.length && previous != latest {
        return Ok(false);
    }
    let start = previous.length.saturating_sub(ANCHOR_BYTES as u64);
    Ok(read_range(store, previous, start)? == checkpoint.anchor)
}

/// A complete initial snapshot followed by exact append-only tails. A changed
/// inventory rebuilds every segment, including days between tool calls.
struct DependencyReader {
    projection: DependencyProjection,
    checkpoint: HistoryCheckpoint,
    events: Vec<Event>,
}

impl DependencyReader {
    fn load(store: &Store) -> Result<Self, String> {
        for _ in 0..3 {
            let segments = inventory(store)?;
            let mut events = Vec::new();
            let mut anchor = Vec::new();
            for segment in &segments {
                let bytes = read_range(store, segment, 0)?;
                events.extend(parse_dependency_events(&bytes)?);
                anchor = tail_anchor(&bytes);
            }
            let checkpoint = HistoryCheckpoint { segments, anchor };
            if !checkpoint_matches(store, &checkpoint, &inventory(store)?)? {
                continue;
            }
            let projection =
                DependencyProjection::from_events(&events).map_err(|error| error.to_string())?;
            return Ok(Self {
                projection,
                checkpoint,
                events,
            });
        }
        Err("dependency history kept changing during reconstruction; retry".into())
    }

    fn refresh(&mut self, store: &Store) -> Result<(), String> {
        let segments = inventory(store)?;
        if !checkpoint_matches(store, &self.checkpoint, &segments)? {
            *self = Self::load(store)?;
            return Ok(());
        }
        let Some(latest) = segments.last() else {
            return Ok(());
        };
        let previous = self
            .checkpoint
            .segments
            .last()
            .expect("matching checkpoint");
        if previous.length == latest.length {
            return Ok(());
        }
        let bytes = read_range(store, latest, previous.length)?;
        let events = parse_dependency_events(&bytes)?;
        let mut prefix = self.checkpoint.anchor.clone();
        prefix.extend_from_slice(&bytes);
        let checkpoint = HistoryCheckpoint {
            segments,
            anchor: tail_anchor(&prefix),
        };
        if !checkpoint_matches(store, &checkpoint, &inventory(store)?)? {
            *self = Self::load(store)?;
            return Ok(());
        }
        let mut projection = self.projection.clone();
        for event in &events {
            projection.apply(event).map_err(|error| error.to_string())?;
        }
        self.projection = projection;
        self.events.extend(events);
        self.checkpoint = checkpoint;
        Ok(())
    }
}

/// Regenerable cache of dependency events only. The event log remains the
/// authority; malformed caches are discarded and tails are replayed under lock.
#[derive(Serialize, Deserialize)]
struct CachedDependencies {
    version: u8,
    checkpoint: HistoryCheckpoint,
    events: Vec<Event>,
    checksum: u64,
}

impl CachedDependencies {
    /// Detect accidental edits, not an authentication boundary. Any process
    /// able to rewrite this cache can also rewrite the local authoritative log.
    fn checksum(&self) -> Result<u64, String> {
        let bytes = serde_json::to_vec(&(self.version, &self.checkpoint, &self.events))
            .map_err(|error| error.to_string())?;
        let mut hasher = DefaultHasher::new();
        bytes.hash(&mut hasher);
        Ok(hasher.finish())
    }
}

pub(crate) fn cached_views(store: &Store) -> Result<Vec<DependencyView>, String> {
    local_only(store)?;
    fs::create_dir_all(store.root()).map_err(|error| error.to_string())?;
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(store.root().join("dependency-index.lock"))
        .map_err(|error| error.to_string())?;
    lock.lock_exclusive().map_err(|error| error.to_string())?;
    let path = store.root().join("dependency-index.json");
    let cached = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<CachedDependencies>(&bytes).ok())
        .filter(|cache| cache.version == 2 && cache.checksum().ok() == Some(cache.checksum));
    let mut reader = if let Some(cache) = cached {
        match DependencyProjection::from_events(&cache.events) {
            Ok(projection) => DependencyReader {
                projection,
                checkpoint: cache.checkpoint,
                events: cache.events,
            },
            Err(_) => DependencyReader::load(store)?,
        }
    } else {
        DependencyReader::load(store)?
    };
    reader.refresh(store)?;
    let views = reader.projection.list();
    let mut cache = CachedDependencies {
        version: 2,
        checkpoint: reader.checkpoint,
        events: reader.events,
        checksum: 0,
    };
    cache.checksum = cache.checksum()?;
    let bytes = serde_json::to_vec(&cache).map_err(|error| error.to_string())?;
    let temporary = store
        .root()
        .join(format!("dependency-index.{}.tmp", std::process::id()));
    fs::write(&temporary, bytes).map_err(|error| error.to_string())?;
    fs::rename(temporary, path).map_err(|error| error.to_string())?;
    Ok(views)
}

pub(crate) fn brief_hint(store: &Store, session: &str) -> Option<String> {
    if local_only(store).is_err() {
        return None;
    }
    match cached_views(store) {
        Ok(views) => {
            let relevant: Vec<_> = views
                .into_iter()
                .filter(|view| {
                    !view.consumed
                        && (view.requester_session == session
                            || (view.responder_session == session
                                && view.state == DependencyState::Pending))
                })
                .collect();
            if relevant.is_empty() {
                return None;
            }
            let mut lines = vec![format!(
                "harbard · {} durable dependencies; inspect with dependency list. Peer claims need verification; no automatic resume.",
                relevant.len()
            )];
            for view in relevant.into_iter().rev().take(3) {
                let next = if view.requester_session != session {
                    "inspect condition and resolve with evidence"
                } else if view.state == DependencyState::Pending {
                    "await or continue independent work"
                } else {
                    "verify result, then consume"
                };
                lines.push(format!(
                    "dependency {} · {:?} · {}",
                    view.id, view.state, next
                ));
            }
            Some(lines.join("\n"))
        }
        Err(error) => Some(format!(
            "harbard · dependency history unavailable: {error}. Inspect before continuing dependent work."
        )),
    }
}

#[cfg(test)]
mod recovery_tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn captured_read_boundary_never_consumes_a_racing_append() {
        let home =
            std::env::temp_dir().join(format!("harbard-read-boundary-{}", ulid::Ulid::generate()));
        let store = Store::at(&home);
        fs::create_dir_all(store.events_dir()).unwrap();
        let path = store.events_dir().join("events-2000-01-01.jsonl");
        fs::write(&path, b"first\n").unwrap();
        let captured = inventory(&store).unwrap().pop().unwrap();
        OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"racing\n")
            .unwrap();
        assert_eq!(read_range(&store, &captured, 0).unwrap(), b"first\n");
        let after = inventory(&store).unwrap().pop().unwrap();
        assert_eq!(
            read_range(&store, &after, captured.length).unwrap(),
            b"racing\n"
        );
        fs::remove_dir_all(home).unwrap();
    }

    #[test]
    fn a_failed_incremental_read_keeps_the_previous_checkpoint() {
        let home =
            std::env::temp_dir().join(format!("harbard-read-failure-{}", ulid::Ulid::generate()));
        let store = Store::at(&home);
        fs::create_dir_all(store.events_dir()).unwrap();
        let path = store.events_dir().join("events-2000-01-01.jsonl");
        fs::write(&path, b"\n").unwrap();
        let mut reader = DependencyReader::load(&store).unwrap();
        let before = serde_json::to_vec(&reader.checkpoint).unwrap();
        fs::remove_file(&path).unwrap();
        fs::create_dir(&path).unwrap();
        assert!(reader.refresh(&store).is_err());
        assert_eq!(serde_json::to_vec(&reader.checkpoint).unwrap(), before);
        fs::remove_dir_all(home).unwrap();
    }
}
