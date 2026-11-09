//! `GET /api/v1/workspace` and `POST /api/v1/workspace/<action>`: the
//! console's chat-shaped shell (`harbard_core::workspace`).
//!
//! The read folds the team projection, the bus sessions, the operator's own
//! notes and the sluice receipts into channels, agents, messages and cards.
//! It is pull-only, like `/team`: computed when the console asks.
//!
//! The writes are the operator's and nobody else's:
//!
//! - `resolve`, `reopen`, `read`, `save` and `unsave` change the desk file
//!   (`workspace.json`). Nothing is sent to an agent. So do `hide-agent`,
//!   `show-agent`, `show-all-agents` ("Hide") and `agents-collapsed`.
//! - `reply` appends operator notes addressed to an agent's lead session, the
//!   same event `POST /notes` writes. When the bus cannot carry it the request
//!   is refused with the reason, and nothing is stored.
//! - `open` asks the OS to open a local file the view rendered as a file
//!   card. It never uploads or reads the file, and it refuses any path the
//!   view did not show.
//! - `nudge` ("Resend to the lead") sends an unanswered operator note
//!   again, as an `ask` with a `ref` to it, to the sessions it was stored
//!   for: an ask wakes an idle session and is owed an answer.
//!
//! `GET /api/v1/attachments/<ulid>/<name>` serves a file `harbard attach`
//! copied, from `{home}/attachments` and nowhere else ([`attachment_file`]);
//! the read only ever looks there to tell an attachment from a file card
//! ([`attachment_lookup`]). Neither touches a path the operator's privacy
//! folders hold.
//! - `theme-*` choose, save, import and preview console themes, and keep
//!   "Notify in the Dock" (`theme-dock`) beside them
//!   (`serve/theme_route.rs`).

use super::fleet_routes::ApiError;
use super::team_route;
use crate::attach;
use harbard_core::query;
use harbard_core::team::TeamView;
use harbard_core::workspace::{
    self, ActivityView, Attachment, BusNote, BusReply, BusSession, CheckoutInfo, DeskState, Inputs,
    NoteTrail, OperatorNote, SavedItem, WorkspaceView,
};
use harbard_core::workspaces::{Gate, Placement};
use harbard_core::{Event, EventKind, NoteKind, Recipient, RecipientKind, Role, Store, clock};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// Sluice deliveries older than this are not listed.
const DELIVERY_WINDOW_MS: u64 = 7 * 24 * 3_600_000;
/// How long a resolved checkout (repo and branch) is trusted.
const CHECKOUT_TTL_MS: u64 = 60_000;
/// Longest reply the composer may store; the event schema caps the line.
const REPLY_MAX_CHARS: usize = 1_800;
/// Cap on the text copied into a saved item.
const SAVED_TEXT_CHARS: usize = 300;

/// Read-modify-write of the desk file happens under this lock.
static DESK: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// Repositories on disk
// ---------------------------------------------------------------------------

/// Every disk access the workspace read makes goes through this, so a test
/// can prove which paths are touched.
pub trait Fs: Sync {
    fn exists(&self, path: &Path) -> bool;
    fn read_to_string(&self, path: &Path) -> Option<String>;
    fn canonicalize(&self, path: &Path) -> Option<PathBuf>;
    /// The size of `root/dirs/name`, walked and opened by descriptor
    /// ([`harbard_core::safe_fs`]): `None` for a symlink anywhere on the way,
    /// a hardlink, anything but a regular file.
    fn len_under(&self, root: &Path, dirs: &[&str], name: &str) -> Option<u64>;
    /// The bytes of `root/dirs/name`, at most `cap`, by the same rules.
    fn read_under(&self, root: &Path, dirs: &[&str], name: &str, cap: u64) -> Option<Vec<u8>>;
}

pub struct RealFs;

impl Fs for RealFs {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }
    fn read_to_string(&self, path: &Path) -> Option<String> {
        std::fs::read_to_string(path).ok()
    }
    fn canonicalize(&self, path: &Path) -> Option<PathBuf> {
        std::fs::canonicalize(path).ok()
    }
    fn len_under(&self, root: &Path, dirs: &[&str], name: &str) -> Option<u64> {
        harbard_core::safe_fs::len_under(root, dirs, name).ok()
    }
    fn read_under(&self, root: &Path, dirs: &[&str], name: &str, cap: u64) -> Option<Vec<u8>> {
        harbard_core::safe_fs::read_under(root, dirs, name, cap).ok()
    }
}

/// `.git` of a checkout: a directory, or a `gitdir:` pointer (worktree).
fn git_dir(fs: &dyn Fs, repo: &Path) -> Option<PathBuf> {
    let git = repo.join(".git");
    if let Some(contents) = fs.read_to_string(&git) {
        let pointer = PathBuf::from(contents.strip_prefix("gitdir:")?.trim());
        let resolved = if pointer.is_absolute() {
            pointer
        } else {
            repo.join(pointer)
        };
        return Some(fs.canonicalize(&resolved).unwrap_or(resolved));
    }
    fs.exists(&git)
        .then(|| fs.canonicalize(&git).unwrap_or(git))
}

/// The primary checkout a checkout belongs to (the common `.git`'s parent).
fn repo_key(fs: &dyn Fs, repo: &Path) -> Option<String> {
    let dir = git_dir(fs, repo)?;
    let common = fs
        .read_to_string(&dir.join("commondir"))
        .map(|c| dir.join(c.trim()))
        .unwrap_or(dir);
    let common = fs.canonicalize(&common).unwrap_or(common);
    if common.file_name().and_then(|n| n.to_str()) != Some(".git") {
        return None;
    }
    let root = common.parent()?;
    Some(
        fs.canonicalize(root)
            .unwrap_or_else(|| root.to_path_buf())
            .display()
            .to_string(),
    )
}

fn git_branch(fs: &dyn Fs, repo: &Path) -> Option<String> {
    let head = fs.read_to_string(&git_dir(fs, repo)?.join("HEAD"))?;
    head.trim()
        .strip_prefix("ref: refs/heads/")
        .map(str::to_string)
}

/// A checkout harbard already knows about without looking: from a bus
/// session (its hooks ran in the terminal) or a cwd Claude Code recorded
/// with a `gitBranch`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Known {
    pub root: String,
    pub repo: Option<String>,
    pub branch: Option<String>,
}

/// Temp space: a clone there is scratch, not a project.
const TEMP_ROOTS: [&str; 4] = ["/tmp", "/private/tmp", "/private/var", "/var/folders"];

pub struct Resolver<'a> {
    pub home: String,
    pub temp_roots: &'a [&'a str],
    pub known: Vec<Known>,
    pub fs: &'a dyn Fs,
}

impl Resolver<'_> {
    /// The checkout, repository and branch of a path. Lexical unless the
    /// path lies under a known root outside the protected folders; then the
    /// root's `.git` files are read (never a git process).
    pub fn resolve(&self, path: &str) -> Option<CheckoutInfo> {
        let home = self.home.as_str();
        let under = |p: &str, root: &str| p == root || p.starts_with(&format!("{root}/"));
        let claude = format!("{home}/.claude");
        // Claude's own state and temp space are never a channel.
        if (!home.is_empty() && under(path, &claude))
            || self.temp_roots.iter().any(|r| under(path, r))
        {
            return None;
        }
        let known = self
            .known
            .iter()
            .filter(|k| under(path, &k.root))
            .max_by_key(|k| k.root.len());
        let named = workspace::worktree_component(path);
        // A worktree named inside a known root (`repo/.claude/worktrees/x`,
        // or a known parent of `repo-wt-x`) is its own checkout.
        let named_deeper = match (&named, known) {
            (Some(n), Some(k)) => n.len() > k.root.len(),
            (Some(_), None) => true,
            _ => false,
        };
        // Under a protected folder nothing is a channel, known root or not:
        // what lives there is the operator's, not a project of the fleet.
        if workspace::protected(path, home) {
            return None;
        }
        if named_deeper {
            // A worktree by its name: real only if its primary checkout is a
            // git repository on disk (outside the protected folders).
            return named.map(|n| {
                let mut info = workspace::by_name(&n);
                info.real = !workspace::protected(&info.repo, home)
                    && self.fs.exists(&Path::new(&info.repo).join(".git"));
                info
            });
        }
        // A path under no known root is looked at only inside the home
        // folder (never a protected folder): an agent that ran `git -C` in a
        // repository there names a real repository, quiet or not.
        if known.is_none() && (home.is_empty() || !under(path, home)) {
            return None;
        }
        let lexical = || {
            known.map(|k| CheckoutInfo {
                checkout: k.root.clone(),
                repo: k
                    .repo
                    .clone()
                    .unwrap_or_else(|| workspace::by_convention(&k.root)),
                branch: k.branch.clone(),
                // The bus names a session's repository from git itself.
                real: k.repo.is_some(),
            })
        };
        // Outside the protected folders: the nearest `.git` says which
        // checkout it is (a nested repository, or the root above a recorded
        // subdirectory). The walk stops at home and never enters a
        // protected folder.
        let mut dir = Some(Path::new(path));
        let mut depth = 0;
        while let Some(d) = dir {
            let shown = d.display().to_string();
            if depth > 24 || shown == "/" || shown == home || workspace::protected(&shown, home) {
                break;
            }
            if self.fs.exists(&d.join(".git")) {
                return Some(CheckoutInfo {
                    repo: repo_key(self.fs, d)
                        .or_else(|| known.and_then(|k| k.repo.clone()))
                        .unwrap_or_else(|| shown.clone()),
                    branch: git_branch(self.fs, d).or_else(|| known.and_then(|k| k.branch.clone())),
                    checkout: shown,
                    real: true,
                });
            }
            dir = d.parent();
            depth += 1;
        }
        lexical()
    }
}

/// Known roots from the bus sessions and the team view's recorded cwds.
fn known_roots(sessions: &[harbard_core::query::SessionView], team: &TeamView) -> Vec<Known> {
    let mut out: Vec<Known> = Vec::new();
    let mut push = |k: Known| {
        if k.root.starts_with('/') && !out.iter().any(|o| o.root == k.root) {
            out.push(k);
        }
    };
    for s in sessions {
        if let Some(repo) = &s.repo {
            push(Known {
                root: repo.clone(),
                repo: Some(s.repo_key.clone().unwrap_or_else(|| repo.clone())),
                branch: s.branch.clone(),
            });
        }
    }
    for lead in &team.leads {
        for cwd in &lead.git_cwds {
            push(Known {
                root: cwd.clone(),
                repo: None,
                branch: None,
            });
        }
    }
    out
}

type CheckoutCache = Mutex<HashMap<String, (Option<CheckoutInfo>, u64)>>;

fn checkout_cache() -> &'static CheckoutCache {
    static CACHE: OnceLock<CheckoutCache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

// ---------------------------------------------------------------------------
// The read
// ---------------------------------------------------------------------------

/// The session an operator note is addressed to, when it is one session.
fn note_session(e: &Event) -> Option<&str> {
    if e.event != EventKind::Note || e.role != Role::Operator {
        return None;
    }
    let to = e.to.as_ref()?;
    (to.kind == RecipientKind::Session).then_some(to.value.as_str())
}

/// The delivery marks of one session, read once.
struct Marks {
    brief: Option<String>,
    inbox: Option<String>,
    live: Option<String>,
    woke: std::collections::HashSet<String>,
}

impl Marks {
    fn read(store: &Store, session: &str) -> Self {
        Marks {
            brief: harbard_core::cursor::read_brief_state(store, session).seen,
            inbox: harbard_core::cursor::read_inbox_cursor(store, session),
            live: harbard_core::cursor::read_live_state(store, session).seen,
            woke: harbard_core::cursor::read_wake_state(store, session).delivered,
        }
    }

    /// The session was handed `e`: by its brief or inbox (any kind), by its
    /// watch (an explicit delivery receipt), or by the inline channel for a
    /// kind that interrupts (the only kinds it carries).
    fn carried(&self, e: &Event) -> bool {
        use harbard_core::wake::at_or_before;
        at_or_before(&e.id, self.brief.as_deref())
            || at_or_before(&e.id, self.inbox.as_deref())
            || self.woke.contains(&e.id)
            || (e.kind.interrupts() && at_or_before(&e.id, self.live.as_deref()))
    }
}

/// Operator notes addressed to one session, from the hub's warm window,
/// each with its trail: delivered (after the session joined the bus; a note
/// from before is never delivered), acked, answered by `ref`, re-sent.
/// A re-send ("Resend to the lead") is an operator note whose `ref`
/// names another: it is not listed, it adds to the original's trail.
fn operator_notes(store: &Store, events: &[Event]) -> Vec<OperatorNote> {
    let mut joined: HashMap<&str, usize> = HashMap::new();
    for (i, e) in events.iter().enumerate() {
        joined.entry(e.session.as_str()).or_insert(i);
    }
    let operator_ids: std::collections::HashSet<&str> = events
        .iter()
        .filter(|e| note_session(e).is_some())
        .map(|e| e.id.as_str())
        .collect();
    fn resend_of<'e>(
        e: &'e Event,
        operator_ids: &std::collections::HashSet<&str>,
    ) -> Option<&'e str> {
        e.note_ref
            .as_deref()
            .filter(|r| note_session(e).is_some() && operator_ids.contains(r))
    }
    let mut resends: HashMap<&str, Vec<(usize, &Event)>> = HashMap::new();
    let mut acks: HashMap<&str, u64> = HashMap::new();
    let mut replies: HashMap<&str, Vec<&Event>> = HashMap::new();
    for (i, e) in events.iter().enumerate() {
        if let Some(original) = resend_of(e, &operator_ids) {
            resends.entry(original).or_default().push((i, e));
        } else if let Some(id) = e.acknowledged_note_id() {
            let at = acks.entry(id).or_insert(u64::MAX);
            *at = (*at).min(query::event_ms(e));
        } else if e.event == EventKind::Note
            && !e.is_operator()
            && e.role != Role::Operator
            && let Some(r) = e.note_ref.as_deref()
        {
            replies.entry(r).or_default().push(e);
        }
    }
    let mut marks: HashMap<String, Marks> = HashMap::new();
    let mut out = Vec::new();
    for (i, e) in events.iter().enumerate() {
        let Some(session) = note_session(e) else {
            continue;
        };
        if resend_of(e, &operator_ids).is_some() {
            continue;
        }
        let family: Vec<(usize, &Event)> = std::iter::once((i, e))
            .chain(resends.get(e.id.as_str()).into_iter().flatten().copied())
            .collect();
        let m = marks
            .entry(session.to_string())
            .or_insert_with(|| Marks::read(store, session));
        let after_join = |at: usize| joined.get(session).is_none_or(|first| *first < at);
        let delivered = family.iter().any(|(at, n)| after_join(*at) && m.carried(n));
        let acked_ms = family
            .iter()
            .filter_map(|(_, n)| acks.get(n.id.as_str()).copied())
            .min();
        let mut answers: Vec<BusReply> = Vec::new();
        for (_, n) in &family {
            for r in replies.get(n.id.as_str()).into_iter().flatten() {
                if answers.iter().all(|a| a.id != r.id) {
                    answers.push(BusReply {
                        id: r.id.clone(),
                        ts_ms: query::event_ms(r),
                        session: r.session.clone(),
                        text: r.text.clone(),
                    });
                }
            }
        }
        out.push(OperatorNote {
            id: e.id.clone(),
            ts_ms: query::event_ms(e),
            session: session.to_string(),
            text: e.text.clone(),
            trail: NoteTrail {
                delivered,
                acked_ms,
                replies: answers,
                nudged_ms: family.iter().skip(1).map(|(_, n)| query::event_ms(n)).max(),
            },
        });
    }
    out
}

/// Every agent and non-lead session the view shows, as the workshop
/// registry sees it: its key, team and role, and whether it is live.
pub fn wanted_names(
    team: &TeamView,
    sessions: &[BusSession],
    temp_dirs: &[String],
    now: u64,
) -> Vec<harbard_core::workshop::Wanted> {
    use harbard_core::naming;
    let recent = |ms: Option<u64>| ms.is_some_and(|ms| now.saturating_sub(ms) <= 2 * 3_600_000);
    let mut out = Vec::new();
    let mut leads = std::collections::HashSet::new();
    for lead in &team.leads {
        leads.insert(lead.session.as_str());
        let project = naming::project_of(lead.cwd.as_deref());
        // A throwaway session gets no workshop name: it is never listed.
        if naming::is_temp_project(&lead.project, temp_dirs)
            || lead
                .cwd
                .as_deref()
                .is_some_and(|c| naming::is_temp(c, temp_dirs))
        {
            continue;
        }
        for row in &lead.agents {
            let d = naming::display(row.name.as_deref(), &project, row.first_ms);
            out.push(harbard_core::workshop::Wanted {
                key: row.id.clone(),
                team: d
                    .team
                    .or_else(|| (!project.is_empty()).then(|| project.clone())),
                role: d.role,
                // Done or stopped (TaskStop, shutdown) is finished, however
                // recent: a teammate launched again with its name is the one
                // at work.
                live: matches!(row.state, "running" | "idle") && recent(row.last_action_ms),
                first_ms: row.first_ms.unwrap_or(row.seen_ms),
                // Launched as "Laura": the registry holds it, so nobody else
                // gets it (AC4).
                own: row
                    .name
                    .as_deref()
                    .filter(|n| harbard_core::workshop::listed(n).is_some())
                    .map(str::to_string),
            });
        }
    }
    for s in sessions {
        if leads.contains(s.session.as_str()) || !s.main || s.repo.is_none() {
            continue;
        }
        out.push(harbard_core::workshop::Wanted {
            key: format!("session:{}", s.session),
            team: Some(naming::project_of(s.repo.as_deref())).filter(|p| !p.is_empty()),
            role: Some(naming::SESSION_ROLE.into()),
            live: s.liveness != harbard_core::query::Liveness::Ended && recent(Some(s.last_ms)),
            first_ms: s.first_ms,
            own: None,
        });
    }
    out
}

/// The workshop names, settled in memory only.
///
/// A read never writes. Repeats are reconciled and agents with no name yet
/// are named here ([`harbard_core::workshop::Registry::settle`]) exactly as
/// [`persist_names`] will save them, because settling is a pure function of
/// the registry and the agents (seeded by key). serve saves on its start and
/// every [`NAMES_TICK`]; a rename or a vote saves its own; the CLI's reads
/// (`agents`, `owed`, `clean` listing) only show it. Without a serve
/// running, a name holds as long as the registry and the agents do.
fn workshop_names(
    store: &Store,
    team: &TeamView,
    sessions: &[BusSession],
    temp_dirs: &[String],
    now: u64,
) -> std::collections::BTreeMap<String, harbard_core::workshop::Entry> {
    use harbard_core::workshop::Registry;
    let wanted = wanted_names(team, sessions, temp_dirs, now);
    // Settled in memory only (Emma's reconcile, then new names): the same
    // result persist_names saves.
    let mut reg = Registry::load(store.root());
    reg.settle(&wanted, &callsigns(store), now);
    reg.agents
}

/// How often serve saves the names its reads assigned.
pub const NAMES_TICK: std::time::Duration = std::time::Duration::from_secs(30);

/// serve's names tick: [`persist_names`] at start and every [`NAMES_TICK`].
pub fn start_names(
    store: Store,
    shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>,
    stale_secs: u64,
    history: impl Fn() -> std::sync::Arc<Vec<Event>> + Send + 'static,
) {
    std::thread::spawn(move || {
        while !shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            let now = harbard_core::clock::now_ms();
            let team = super::team_route::project(now, harbard_core::team::WINDOW_DEFAULT_MS);
            persist_names(&store, &history(), stale_secs, &team, now);
            std::thread::sleep(NAMES_TICK);
        }
    });
}

/// The sessions the view names, as it gathers them.
fn bus_sessions(store: &Store, events: &[Event], stale_secs: u64, now: u64) -> Vec<BusSession> {
    let aliases = harbard_core::alias::read(store);
    query::sessions(store, events, now, stale_secs)
        .iter()
        .map(|s| {
            let mut b = BusSession::from(s);
            // Only a name the session chose itself; a derived callsign
            // reads as noise next to "<project> · session HH:MM".
            if aliases.self_named.contains(&s.session) {
                b.name = harbard_core::alias::get(&aliases, &s.session);
            }
            b
        })
        .collect()
}

/// Save the names as the view settles them (repeats reconciled, new agents
/// named), when that changes anything. serve runs this on its start and on
/// a tick, never on a GET.
pub fn persist_names(store: &Store, events: &[Event], stale_secs: u64, team: &TeamView, now: u64) {
    use harbard_core::workshop::Registry;
    let sessions = bus_sessions(store, events, stale_secs, now);
    let wanted = wanted_names(team, &sessions, &temp_dirs(), now);
    let taken = callsigns(store);
    if !Registry::load(store.root()).needs_settle(&wanted, &taken, now) {
        return;
    }
    // Under the desk's in-process lock as well as the file's own (AC4).
    let _guard = desk_guard();
    let _ = Registry::update(store.root(), |r| {
        r.settle(&wanted, &taken, now);
    });
}

/// The names sessions go by, which a new agent must not repeat.
fn callsigns(store: &Store) -> std::collections::BTreeSet<String> {
    harbard_core::alias::read(store)
        .by_session
        .into_values()
        .map(|name| name.to_lowercase())
        .collect()
}

/// `$TMPDIR`, and its `/private` spelling, as extra temp dirs.
fn temp_dirs() -> Vec<String> {
    let Ok(t) = std::env::var("TMPDIR") else {
        return Vec::new();
    };
    let t = t.trim_end_matches('/').to_string();
    if t.is_empty() {
        return Vec::new();
    }
    let private = if t.starts_with("/private/") {
        t.trim_start_matches("/private").to_string()
    } else {
        format!("/private{t}")
    };
    vec![t, private]
}

/// How long a bus note for the operator stays in the console.
const BUS_NOTE_WINDOW_MS: u64 = 24 * 3_600_000;

/// Bus notes for the operator from any session within `window_ms` (the
/// shell's day, [`BUS_NOTE_WINDOW_MS`], or History's month): addressed to
/// the human (`harness:human`, an operator session) or to everyone, and any
/// note answering one of those by `ref`. An answer to one of the operator's
/// own notes is not here: it rides that note's trail.
fn bus_notes_within(events: &[Event], now: u64, window_ms: u64) -> Vec<BusNote> {
    let operator_ids: std::collections::HashSet<&str> = events
        .iter()
        .filter(|e| e.event == EventKind::Note && e.is_operator())
        .map(|e| e.id.as_str())
        .collect();
    let mut kept: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for e in events {
        if e.event != EventKind::Note
            || e.is_operator()
            || e.role == Role::Operator
            || now.saturating_sub(query::event_ms(e)) > window_ms
        {
            continue;
        }
        if e.note_ref
            .as_deref()
            .is_some_and(|r| operator_ids.contains(r))
        {
            continue;
        }
        let for_human = match &e.to {
            None => true,
            Some(to) => match to.kind {
                RecipientKind::Harness => to.value == harbard_core::HARNESS_HUMAN,
                RecipientKind::Session => to.value.starts_with("operator@"),
                _ => false,
            },
        };
        let answers_one = e.note_ref.as_deref().is_some_and(|r| kept.contains(r));
        if !(for_human || answers_one) {
            continue;
        }
        kept.insert(e.id.clone());
        out.push(BusNote {
            id: e.id.clone(),
            ts_ms: query::event_ms(e),
            session: e.session.clone(),
            text: e.text.clone(),
            repo: e.repository().map(str::to_string),
            note_ref: e.note_ref.clone(),
            kind: e.kind.as_str().to_string(),
        });
    }
    out
}

// ---------------------------------------------------------------------------
// Attachments: `{home}/attachments` and nowhere else
// ---------------------------------------------------------------------------

/// `<ulid>/<name>` of a path under the attachments folder, lexically: a
/// path anywhere else is not an attachment and is never looked at.
fn attachment_id<'p>(root: &str, path: &'p str) -> Option<(&'p str, &'p str)> {
    let rest = path.strip_prefix(root)?.strip_prefix('/')?;
    let (dir, name) = rest.split_once('/')?;
    (attach::valid_dir(dir) && attach::valid_name(name)).then_some((dir, name))
}

/// What a message's path is, when it is an attachment: its kind, label
/// (from the folder's manifest) and size. Touches only the attachments
/// folder; `~/` is the home folder.
pub fn attachment_lookup(fs: &dyn Fs, root: &Path, home: &str, path: &str) -> Option<Attachment> {
    let expanded;
    let full = match path.strip_prefix("~/") {
        Some(rest) if !home.is_empty() => {
            expanded = format!("{home}/{rest}");
            expanded.as_str()
        }
        _ => path,
    };
    let root_s = root.display().to_string();
    let (dir, name) = attachment_id(&root_s, full)?;
    let (kind, _) = attach::kind_of(name)?;
    let bytes = fs.len_under(root, &[dir], name)?;
    let label = fs
        .read_under(root, &[dir], attach::MANIFEST, attach::MANIFEST_MAX)
        .and_then(|m| serde_json::from_slice::<Value>(&m).ok())
        .and_then(|m| m.get("label").and_then(Value::as_str).map(str::to_string))
        .and_then(|l| attach::label_of(&l).map(str::to_string));
    Some(Attachment {
        id: format!("{dir}/{name}"),
        name: name.to_string(),
        kind,
        label,
        bytes,
        path: path.to_string(),
    })
}

/// `GET /api/v1/attachments/<ulid>/<name>`: the bytes and content type of
/// a file under the attachments folder. Refused: anything that is not a
/// ULID folder and a name `attach` could have written, a symlink anywhere
/// on the way, HTML (opened with the OS, never served), a file over the cap.
pub fn attachment_file(
    fs: &dyn Fs,
    root: &Path,
    rest: &str,
) -> Result<(Vec<u8>, &'static str), ApiError> {
    let refused = || ApiError::new(404, "not_an_attachment", "no such attachment");
    let (dir, name) = rest.split_once('/').ok_or_else(refused)?;
    if !attach::valid_dir(dir) || !attach::valid_name(name) {
        return Err(refused());
    }
    let (kind, content_type) = attach::kind_of(name).ok_or_else(refused)?;
    if kind == "html" {
        return Err(ApiError::new(
            403,
            "opened_by_the_os",
            "an HTML attachment is opened with the OS, never served",
        ));
    }
    // By descriptor, every step: a symlink swapped in, a hardlink, a FIFO
    // or a file grown past the cap is refused, never followed.
    let bytes = fs
        .read_under(root, &[dir], name, attach::MAX_BYTES)
        .ok_or_else(refused)?;
    Ok((bytes, content_type))
}

/// Executable names in serve's PATH, read once at startup so the faces may
/// name a program that is installed (`harbard_core::activity`). Only
/// absolute directories outside the TCC-protected folders and /Volumes are
/// listed, and only their entries' names: no entry is opened or stat'ed.
/// A directory reached through a symlink (itself or any parent) is never
/// listed, whatever its target: `~/bin -> ~/Documents/bin` must not be read.
pub fn path_programs(path: &str, home: &str, list: &dyn Fn(&Path) -> Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for dir in path.split(':') {
        if !dir.starts_with('/')
            || dir.split('/').any(|c| c == "..")
            || workspace::protected(dir, home)
            || through_symlink(Path::new(dir))
        {
            continue;
        }
        out.extend(list(Path::new(dir)).into_iter().filter(|n| {
            !n.starts_with('.')
                && n.len() <= 64
                && n.chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '+' | '-'))
        }));
    }
    out.sort();
    out.dedup();
    out
}

/// Is `dir`, or any directory above it, a symlink? Each prefix is
/// `lstat`ed, so nothing a link points at is ever touched.
fn through_symlink(dir: &Path) -> bool {
    let mut prefix = PathBuf::from("/");
    for part in dir.components().skip(1) {
        prefix.push(part);
        match std::fs::symlink_metadata(&prefix) {
            Ok(m) if m.file_type().is_symlink() => return true,
            Ok(_) => {}
            // Missing or unreadable: nothing to list.
            Err(_) => return true,
        }
    }
    false
}

/// The names in one directory, nothing else.
pub fn list_dir(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Serve startup, once: a new desk does not open with every old message
/// unread, and the protocol flags start from the moment this build first
/// ran, so history is not painted with them. A GET never writes.
pub fn init_desk(store: &Store, now: u64) {
    let _guard = DESK.lock().unwrap_or_else(|e| e.into_inner());
    let desk = DeskState::load(store);
    if desk.baseline_ms != 0 && desk.config.protocol_since_ms != 0 {
        return;
    }
    // Under the cross-process desk lock; a desk that does not parse is left
    // as it is.
    let _ = DeskState::update(&DeskState::path(store.root()), |desk| {
        if desk.baseline_ms == 0 {
            desk.baseline_ms = now;
        }
        if desk.config.protocol_since_ms == 0 {
            desk.config.protocol_since_ms = now;
        }
    });
}

/// Seal the rules under the serve key, once ([`harbard_core::rules::seal_legacy`]).
///
/// The first start that has no key makes it, seals every rule that was
/// active before, and asks "Was it you?" about each: trust on first use,
/// with the operator's review behind it. When the desk says sealing already
/// happened and the key is gone, a new key is made and nothing is sealed:
/// the old rules stop counting until they approve them again, rather than a
/// deleted key blessing whatever was appended since.
pub fn seal_rules(store: &Store, now: u64) {
    let root = store.root();
    let desk = DeskState::load(store);
    let had_key = harbard_core::seal::load(root).is_some();
    if had_key {
        if desk.rules_sealed_ms.is_none() {
            let _ = with_desk(store, |d| d.rules_sealed_ms = Some(now));
        }
        return;
    }
    if desk.rules_sealed_ms.is_some() {
        eprintln!(
            "harbard serve: serve.key is missing although rules were sealed before; a new key is made and the earlier rules no longer count until they are approved again in Retro"
        );
        let _ = harbard_core::seal::ensure(root);
        return;
    }
    let base =
        || crate::serve::operator_note_event("", None, harbard_core::NoteKind::Fyi, None, now);
    match harbard_core::rules::seal_legacy(store, base) {
        Ok(sealed) => {
            let _ = with_desk(store, |d| {
                d.rules_sealed_ms = Some(now);
                for r in &sealed {
                    d.add_check(rule_check(r, now));
                }
            });
        }
        Err(e) => eprintln!("harbard serve: sealing the rules failed: {e}"),
    }
}

/// "A rule was approved: "…". Was it you?"
pub(super) fn rule_check(
    r: &harbard_core::rules::ActiveRule,
    now: u64,
) -> harbard_core::workspace::DeskCheck {
    harbard_core::workspace::DeskCheck {
        id: format!("rule-{}", r.id),
        what: "rule".into(),
        text: r.text.clone(),
        rule: Some(r.id.clone()),
        since_ms: now,
        ..Default::default()
    }
}

pub fn view(
    store: &Store,
    events: &[Event],
    stale_secs: u64,
    team: &TeamView,
    now: u64,
) -> WorkspaceView {
    let home = std::env::var("HOME").unwrap_or_default();
    let disk = Disk {
        fs: &RealFs,
        cached: true,
        home: &home,
        temp_roots: &TEMP_ROOTS,
    };
    view_with(store, events, stale_secs, team, now, &disk)
}

/// The read over History's month: every bus session in it, ended ones
/// too, and the operator's bus notes of the whole window.
pub fn archive_view(
    store: &Store,
    events: &[Event],
    stale_secs: u64,
    team: &TeamView,
    now: u64,
    gate: &Gate<'_>,
) -> WorkspaceView {
    let home = std::env::var("HOME").unwrap_or_default();
    let disk = Disk {
        fs: &RealFs,
        cached: true,
        home: &home,
        temp_roots: &TEMP_ROOTS,
    };
    view_scoped(
        store,
        events,
        stale_secs,
        team,
        now,
        &disk,
        Scope::ARCHIVE,
        gate,
    )
}

/// The read of one workspace: what the console shows when `gate` is the
/// workspace it selected. [`view`] is the whole hub.
pub fn view_in(
    store: &Store,
    events: &[Event],
    stale_secs: u64,
    team: &TeamView,
    now: u64,
    gate: &Gate<'_>,
) -> WorkspaceView {
    let home = std::env::var("HOME").unwrap_or_default();
    let disk = Disk {
        fs: &RealFs,
        cached: true,
        home: &home,
        temp_roots: &TEMP_ROOTS,
    };
    view_scoped(
        store,
        events,
        stale_secs,
        team,
        now,
        &disk,
        Scope::SHELL,
        gate,
    )
}

/// The read with an injected filesystem; `cached` is off in tests.
/// How the read may touch the disk: the filesystem, whether to use the
/// checkout cache (off in tests), the home folder and temp roots.
pub struct Disk<'a> {
    pub fs: &'a dyn Fs,
    pub cached: bool,
    pub home: &'a str,
    pub temp_roots: &'a [&'a str],
}

/// How far back the read reaches: the shell's day, or History's month.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Scope {
    /// Bus sessions silent for longer than this are not conversations.
    pub horizon_ms: u64,
    /// Ended sessions stay (History shows them read-only).
    pub keep_ended: bool,
    /// Bus notes for the operator older than this are left out.
    pub bus_notes_ms: u64,
}

impl Scope {
    pub const SHELL: Scope = Scope {
        horizon_ms: workspace::ACTIVE_MS,
        keep_ended: false,
        bus_notes_ms: BUS_NOTE_WINDOW_MS,
    };
    pub const ARCHIVE: Scope = Scope {
        horizon_ms: harbard_core::history::WINDOW_MS,
        keep_ended: true,
        bus_notes_ms: harbard_core::history::WINDOW_MS,
    };
}

pub fn view_with(
    store: &Store,
    events: &[Event],
    stale_secs: u64,
    team: &TeamView,
    now: u64,
    disk: &Disk<'_>,
) -> WorkspaceView {
    view_scoped(
        store,
        events,
        stale_secs,
        team,
        now,
        disk,
        Scope::SHELL,
        &Gate::OPEN,
    )
}

/// [`view_with`] over a [`Scope`], inside the workspace `gate` is for.
///
/// Everything the read folds is cut to the workspace before it is folded:
/// lead sessions by their working directory, bus sessions and bus notes by
/// their repository, the operator's notes by the session they were for,
/// sluice deliveries by repository, and every repository a channel could be
/// made of ([`Inputs::in_workspace`]). What is not handed to the read cannot
/// appear in it.
#[allow(clippy::too_many_arguments)]
pub fn view_scoped(
    store: &Store,
    events: &[Event],
    stale_secs: u64,
    team: &TeamView,
    now: u64,
    disk: &Disk<'_>,
    scope: Scope,
    gate: &Gate<'_>,
) -> WorkspaceView {
    let Disk {
        fs,
        cached,
        home,
        temp_roots,
    } = *disk;
    let mut session_views = query::sessions(store, events, now, stale_secs);
    // Checkouts are placed on disk against every root the hub knows, not
    // only this workspace's: a path is then resolved to its own repository
    // (and left out) instead of being renamed into one of this workspace's.
    let known = known_roots(&session_views, team);
    let scoped_team;
    let team = if matches!(gate, Gate::Open) {
        team
    } else {
        let mut t = team.clone();
        t.leads.retain(|lead| gate.holds(lead.cwd.as_deref()));
        scoped_team = t;
        &scoped_team
    };
    // The operator's notes follow the session they were for; a note for a
    // session no longer on the bus stays with the default workspace.
    let session_gate: HashMap<String, bool> = session_views
        .iter()
        .map(|v| (v.session.clone(), gate.holds_session(v)))
        .collect();
    session_views.retain(|v| session_gate.get(&v.session).copied().unwrap_or(false));
    let aliases = harbard_core::alias::read(store);
    let sessions: Vec<BusSession> = session_views
        .iter()
        .map(|s| {
            let mut b = BusSession::from(s);
            // Only a name the session chose (`harbard name`); a derived
            // callsign reads as noise next to "<project> · session HH:MM".
            if aliases.self_named.contains(&s.session) {
                b.name = harbard_core::alias::get(&aliases, &s.session);
            }
            let callsign = harbard_core::alias::get(&aliases, &s.session)
                .unwrap_or_else(|| harbard_core::alias::preferred(&s.session));
            b.callsign = Some(harbard_core::alias::display(&callsign));
            b
        })
        .collect();
    let mut notes = operator_notes(store, events);
    notes.retain(|n| {
        session_gate
            .get(&n.session)
            .copied()
            .unwrap_or_else(|| gate.holds(None))
    });
    let mut bus = bus_notes_within(events, now, scope.bus_notes_ms);
    bus.retain(|b| gate.holds(b.repo.as_deref()));
    let temp_dirs = temp_dirs();
    let names = workshop_names(store, team, &sessions, &temp_dirs, now);
    let since = now.saturating_sub(DELIVERY_WINDOW_MS);
    let (mut deliveries, mut receipts) = harbard_core::sluice::all(store.root());
    deliveries.retain(|d| d.created_ms >= since && gate.holds(Some(&d.repo)));
    receipts.retain(|r| r.decided_ms >= since && gate.holds(Some(&r.repo)));
    let desk = DeskState::load(store);
    // Profile is where the operator names themselves; the desk is the fallback.
    let operator =
        harbard_core::profile::Profile::load(store.root()).console_name(&desk.config.operator_name);
    let resolver = Resolver {
        home: home.to_string(),
        temp_roots,
        known,
        fs,
    };
    let resolve = |path: &str| -> Option<CheckoutInfo> {
        // The known roots differ by workspace (each read knows its own
        // sessions and leads), so the workspace is part of the key.
        let key = format!(
            "{}\u{0}{}\u{0}{path}",
            gate.workspace().unwrap_or(""),
            resolver.known.len()
        );
        if cached
            && let Some((info, at)) = checkout_cache()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get(&key)
            && now.saturating_sub(*at) < CHECKOUT_TTL_MS
        {
            return info.clone();
        }
        let info = resolver.resolve(path);
        if cached {
            checkout_cache()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(key, (info.clone(), now));
        }
        info
    };
    let attachments_root = attach::root(store);
    let attachment = |path: &str| attachment_lookup(fs, &attachments_root, home, path);
    let in_workspace = |repo: &str| gate.holds(Some(repo));
    workspace::build(&Inputs {
        team,
        sessions: &sessions,
        notes: &notes,
        bus_notes: &bus,
        temp_dirs: &temp_dirs,
        names: &names,
        deliveries: &deliveries,
        receipts: &receipts,
        desk: &desk,
        resolve: &resolve,
        attachment: &attachment,
        now_ms: now,
        operator: &operator,
        horizon_ms: scope.horizon_ms,
        keep_ended: scope.keep_ended,
        in_workspace: &in_workspace,
    })
}

// ---------------------------------------------------------------------------
// Workspaces: which one a read or a write is for, and the rail's tiles
// ---------------------------------------------------------------------------

/// The workspaces `caller` may see: every one for this machine and an
/// Operator device; a device's grants otherwise
/// ([`harbard_core::workspaces::device_grants`]).
pub fn allowed(placement: &Placement, caller: &super::access::Caller) -> Vec<String> {
    let Some(set) = placement.registry() else {
        return vec![placement.default_id().to_string()];
    };
    match caller.device() {
        Some(card) => harbard_core::workspaces::device_grants(card, set),
        None => set.workspaces.iter().map(|w| w.id.clone()).collect(),
    }
}

/// What the strip asks for: every workspace its caller may see, in
/// one read. It is the operator's one glance at the hub and has no switcher;
/// reading only the default workspace would hide agents running in another,
/// and that one's "Needs you" too.
pub const ALL_WORKSPACES: &str = "all";

/// True when `ws` asks for the whole hub and this caller may see all of it.
/// Anyone else, a device with grants included, reads as if it had not asked.
fn whole_hub(placement: &Placement, caller: &super::access::Caller, ws: Option<&str>) -> bool {
    ws.map(str::trim) == Some(ALL_WORKSPACES)
        && !placement.is_single()
        && placement.registry().is_some_and(|set| {
            set.get(ALL_WORKSPACES).is_none()
                && allowed(placement, caller).len() == set.workspaces.len()
        })
}

/// The workspace a request is for: the one it names, else the default when
/// the caller may see it, else the first it may. `Ok(None)` on a hub with
/// one workspace, where nothing is scoped.
pub fn select(
    placement: &Placement,
    caller: &super::access::Caller,
    requested: Option<&str>,
) -> Result<Option<String>, ApiError> {
    if placement.is_single() {
        return Ok(None);
    }
    let allowed = allowed(placement, caller);
    let known = |id: &str| placement.registry().and_then(|r| r.get(id)).is_some();
    let named = requested
        .map(str::trim)
        .filter(|r| !r.is_empty() && (*r != ALL_WORKSPACES || known(r)));
    match named {
        Some(id) if !known(id) => Err(ApiError::new(
            404,
            "unknown_workspace",
            format!("no workspace has the id \"{id}\""),
        )),
        Some(id) if !allowed.iter().any(|a| a == id) => Err(ApiError::new(
            403,
            "workspace_forbidden",
            "this device is not granted that workspace",
        )),
        Some(id) => Ok(Some(id.to_string())),
        None => Ok(allowed
            .iter()
            .find(|a| *a == placement.default_id())
            .or(allowed.first())
            .cloned()),
    }
}

/// The workspace an operator read names in its query: a known id, else the
/// default; `None` on a hub with one workspace. For routes only this
/// machine and Operator devices reach, which may read every workspace.
pub fn query_workspace(placement: &Placement, ws: Option<&str>) -> Option<String> {
    if placement.is_single() {
        return None;
    }
    let known = |id: &str| placement.registry().and_then(|r| r.get(id)).is_some();
    Some(
        ws.map(str::trim)
            .filter(|id| known(id))
            .unwrap_or(placement.default_id())
            .to_string(),
    )
}

/// One tile of the rail.
#[derive(Debug, Clone, serde::Serialize, PartialEq, Eq)]
pub struct Tile {
    pub id: String,
    pub name: String,
    /// The tile's one or two characters.
    pub tile: String,
    pub default: bool,
    pub current: bool,
    /// Messages unread in its channels and Direct.
    pub unread: usize,
    /// Open items in its "Needs you".
    pub needs: usize,
    /// Agents running in it now, so a quiet workspace can say where the work is.
    pub working: usize,
}

fn working(v: &WorkspaceView) -> usize {
    v.agents.iter().filter(|a| a.state == "running").count()
}

fn badges(v: &WorkspaceView) -> (usize, usize) {
    (v.unread.channels + v.unread.directs, v.unread.activity)
}

/// The tiles `caller` sees, the current one's badges from `current_view`
/// and every other's from its own read.
#[allow(clippy::too_many_arguments)]
fn tiles(
    store: &Store,
    events: &[Event],
    stale_secs: u64,
    team: &TeamView,
    now: u64,
    placement: &Placement,
    allowed: &[String],
    current: &str,
    current_view: &WorkspaceView,
) -> Vec<Tile> {
    let Some(set) = placement.registry() else {
        return Vec::new();
    };
    set.workspaces
        .iter()
        .filter(|w| allowed.contains(&w.id))
        .map(|w| {
            let other;
            let read = if w.id == current {
                current_view
            } else {
                other = view_in(
                    store,
                    events,
                    stale_secs,
                    team,
                    now,
                    &placement.gate_of(&w.id),
                );
                &other
            };
            let (unread, needs) = badges(read);
            Tile {
                id: w.id.clone(),
                name: w.name.clone(),
                tile: w.tile(),
                default: w.id == set.default,
                current: w.id == current,
                unread,
                needs,
                working: working(read),
            }
        })
        .collect()
}

/// The tile of a hub with one workspace: its name is the sidebar's head.
fn single_tile(placement: &Placement, v: &WorkspaceView) -> Tile {
    let def = placement
        .registry()
        .and_then(|r| r.workspaces.first())
        .cloned()
        .unwrap_or_else(|| harbard_core::workspaces::Workspaces::single().workspaces[0].clone());
    let (unread, needs) = badges(v);
    Tile {
        tile: def.tile(),
        id: def.id,
        name: def.name,
        default: true,
        current: true,
        unread,
        needs,
        working: working(v),
    }
}

/// `only=activity` answers the faces' slice ([`ActivityView`]), the same
/// read with a fraction of the bytes, for the console's short poll.
/// `console` is the commit the console was built from: "Installed" then
/// needs it in both builds. `ws` names the workspace; the full read also
/// carries every tile the caller may see, with its badges.
#[allow(clippy::too_many_arguments)]
pub fn workspace_body(
    store: &Store,
    events: &[Event],
    stale_secs: u64,
    hours: Option<String>,
    only: Option<String>,
    console: Option<String>,
    caller: &super::access::Caller,
    ws: Option<String>,
) -> Result<Value, ApiError> {
    let now = clock::now_ms();
    let placement = Placement::load(store);
    let whole = whole_hub(&placement, caller, ws.as_deref());
    let current = if whole {
        None
    } else {
        select(&placement, caller, ws.as_deref())?
    };
    let gate = match &current {
        Some(id) => placement.gate_of(id),
        None => Gate::Open,
    };
    let team = team_route::project(now, team_route::window_of(hours));
    let mut v = view_in(store, events, stale_secs, &team, now, &gate);
    let current_id = if whole {
        ALL_WORKSPACES.to_string()
    } else {
        current
            .clone()
            .unwrap_or_else(|| placement.default_id().to_string())
    };
    let mut out = if only.as_deref() == Some("activity") {
        serde_json::to_value(ActivityView::of(&v))
    } else {
        let running = running_commit();
        let console = console.filter(|c| is_commit_id(c));
        let home = std::env::var("HOME").unwrap_or_default();
        attach_shipments(&mut v, running.as_deref(), console.as_deref(), &home);
        serde_json::to_value(&v)
    }
    .unwrap_or_else(|e| json!({ "error": e.to_string() }));
    out["workspace"] = json!(current_id);
    if only.as_deref() != Some("activity") {
        let tiles = match &current {
            Some(id) => tiles(
                store,
                events,
                stale_secs,
                &team,
                now,
                &placement,
                &allowed(&placement, caller),
                id,
                &v,
            ),
            None => vec![single_tile(&placement, &v)],
        };
        out["workspaces"] = json!(tiles);
        if whole {
            // A tap on the strip opens the agent or card in the main window,
            // which shows one workspace: each row says which.
            for id in allowed(&placement, caller) {
                let part = view_in(
                    store,
                    events,
                    stale_secs,
                    &team,
                    now,
                    &placement.gate_of(&id),
                );
                let agents: HashSet<&str> = part.agents.iter().map(|a| a.key.as_str()).collect();
                let cards: HashSet<&str> = part.cards.iter().map(|c| c.id.as_str()).collect();
                tag_home(&mut out["agents"], "key", &agents, &id);
                tag_home(&mut out["cards"], "id", &cards, &id);
            }
        }
    }
    Ok(out)
}

/// Marks each row of `rows` whose `field` is in `held` as living in `home`.
fn tag_home(rows: &mut Value, field: &str, held: &HashSet<&str>, home: &str) {
    for row in rows.as_array_mut().into_iter().flatten() {
        if row[field].as_str().is_some_and(|k| held.contains(k)) {
            row["workspace"] = json!(home);
        }
    }
}

/// `GET /api/v1/summary`: what a Display device reads. The agents by name
/// with their presence and face state, and the counts of "Needs you",
/// Direct and blocked cards. No message body, no live prose, no detail
/// line, no path: only labels and numbers.
/// A display never learns a session id: an agent keyed by its session is
/// shown under a stable digest of it instead.
fn summary_key(key: &str) -> String {
    match key.strip_prefix("session:") {
        Some(_) => {
            let digest = harbard_core::pairing::token_hash(key);
            format!("agent:{}", &digest[..12])
        }
        None => key.to_string(),
    }
}

pub fn summary_body(
    store: &Store,
    events: &[Event],
    stale_secs: u64,
    caller: &super::access::Caller,
) -> Value {
    let now = clock::now_ms();
    let team = team_route::project(now, team_route::window_of(None));
    // A display sees the workspaces it is granted and nothing else: one read
    // per workspace, summed.
    let placement = Placement::load(store);
    let reads: Vec<WorkspaceView> = if placement.is_single() {
        vec![view(store, events, stale_secs, &team, now)]
    } else {
        allowed(&placement, caller)
            .iter()
            .map(|id| {
                view_in(
                    store,
                    events,
                    stale_secs,
                    &team,
                    now,
                    &placement.gate_of(id),
                )
            })
            .collect()
    };
    let slices: Vec<ActivityView> = reads.iter().map(ActivityView::of).collect();
    let mut a = slices
        .first()
        .cloned()
        .unwrap_or_else(|| ActivityView::of(&WorkspaceView::default()));
    for more in slices.iter().skip(1) {
        a.agents.extend(more.agents.iter().cloned());
        a.needs += more.needs;
        a.blocked += more.blocked;
        a.directs += more.directs;
    }
    let agents: Vec<Value> = a
        .agents
        .iter()
        .map(|agent| {
            json!({
                "key": summary_key(&agent.key),
                "presence": agent.presence,
                "state": agent.activity.state,
                "since_ms": agent.activity.since_ms,
            })
        })
        .collect();
    json!({
        "v": 1,
        "now_ms": now,
        "agents": agents,
        "needs": a.needs,
        "blocked": a.blocked,
        "directs": a.directs,
    })
}

// ---------------------------------------------------------------------------
// "Deliveries": what git says about each delivered commit
// ---------------------------------------------------------------------------

/// The commit this serve was built from (`build.rs`), when known.
pub fn running_commit() -> Option<String> {
    let c = env!("HARBARD_COMMIT");
    is_commit_id(c).then(|| c.to_string())
}

/// 7 to 40 hex digits: the only thing ever passed to git as a revision.
fn is_commit_id(s: &str) -> bool {
    (7..=40).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// How far back the history sets reach.
const HISTORY_MAX: &str = "20000";

/// Read-only git in `repo`: no optional locks, no prompt, no fsmonitor.
/// `None` when git fails.
fn git_out(repo: &Path, args: &[&str], input: Option<&str>) -> Option<String> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(["-c", "core.fsmonitor=false"])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    if let (Some(text), Some(mut stdin)) = (input, child.stdin.take()) {
        let _ = stdin.write_all(text.as_bytes());
    }
    let out = child.wait_with_output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A change by what a rebase keeps: its author time and subject.
type Change = (u64, String);

/// The commits reachable from a tip, and the change each one carries.
#[derive(Default)]
struct History {
    ids: std::collections::HashSet<String>,
    by_change: HashMap<Change, String>,
}

fn parse_log(text: &str) -> Vec<(String, Change)> {
    text.lines()
        .filter_map(|l| {
            let mut f = l.splitn(3, '\u{0}');
            let id = f.next()?.to_string();
            let at = f.next()?.parse().ok()?;
            let subject = f.next().unwrap_or("").to_string();
            is_commit_id(&id).then_some((id, (at, subject)))
        })
        .collect()
}

fn history_of(repo: &Path, tip: &str) -> History {
    let text = git_out(
        repo,
        &[
            "log",
            "--format=%H%x00%at%x00%s",
            "--max-count",
            HISTORY_MAX,
            tip,
        ],
        None,
    )
    .unwrap_or_default();
    let mut h = History::default();
    for (id, change) in parse_log(&text) {
        h.by_change.entry(change).or_insert_with(|| id.clone());
        h.ids.insert(id);
    }
    h
}

/// Histories by `(repository, tip)`: fixed for a tip, so kept until the
/// cache is full, then dropped whole.
type Histories = Mutex<HashMap<(PathBuf, String), std::sync::Arc<History>>>;

fn history_cached(repo: &Path, tip: &str) -> std::sync::Arc<History> {
    static C: OnceLock<Histories> = OnceLock::new();
    let cache = C.get_or_init(|| Mutex::new(HashMap::new()));
    let key = (repo.to_path_buf(), tip.to_string());
    if let Some(h) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return h.clone();
    }
    let h = std::sync::Arc::new(history_of(repo, tip));
    let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
    if c.len() >= 32 {
        c.clear();
    }
    c.insert(key, h.clone());
    h
}

/// What git says, for one repository, about the SHAs its DONEs named:
/// four git processes at most per read, whatever the number of SHAs.
pub struct RepoGit {
    resolved: HashMap<String, String>,
    main: Option<std::sync::Arc<History>>,
    running: Option<std::sync::Arc<History>>,
    console: Option<std::sync::Arc<History>>,
    /// A delivered commit that is not on main, to the main commit carrying
    /// the same change (a rebase onto `integ` keeps author time and subject).
    rebased: HashMap<String, String>,
}

impl RepoGit {
    /// `running` is serve's commit; `console`, the console's when it said.
    /// "Installed" applies only where serve's commit is known.
    pub fn load(
        repo: &Path,
        shas: &[String],
        running: Option<&str>,
        console: Option<&str>,
    ) -> RepoGit {
        // One batch resolves main, the running builds and every SHA.
        let mut names: Vec<String> = vec!["refs/heads/main".into()];
        names.push(
            running
                .filter(|c| is_commit_id(c))
                .unwrap_or("-")
                .to_string(),
        );
        names.push(
            console
                .filter(|c| is_commit_id(c))
                .unwrap_or("-")
                .to_string(),
        );
        names.extend(shas.iter().filter(|s| is_commit_id(s)).cloned());
        let input: String = names.iter().map(|n| format!("{n}\n")).collect();
        let out = git_out(
            repo,
            &["cat-file", "--batch-check=%(objectname) %(objecttype)"],
            Some(&input),
        )
        .unwrap_or_default();
        let lines: Vec<&str> = out.lines().collect();
        let commit_at = |i: usize| -> Option<String> {
            let l = lines.get(i)?;
            let (id, kind) = l.split_once(' ')?;
            (kind == "commit" && is_commit_id(id) && id.len() == 40).then(|| id.to_string())
        };
        let (tip, run, con) = (commit_at(0), commit_at(1), commit_at(2));
        let mut resolved = HashMap::new();
        for (i, name) in names.iter().enumerate().skip(3) {
            if let Some(id) = commit_at(i) {
                resolved.insert(name.clone(), id);
            }
        }
        let main = tip.as_deref().map(|t| history_cached(repo, t));
        let running = run.as_deref().map(|r| history_cached(repo, r));
        let console = con.as_deref().map(|c| history_cached(repo, c));
        // The delivered commits main does not hold: is the same change there?
        let mut rebased = HashMap::new();
        if let Some(m) = &main {
            let off: Vec<&str> = resolved
                .values()
                .filter(|id| !m.ids.contains(*id))
                .map(String::as_str)
                .collect();
            if !off.is_empty() {
                let mut args = vec!["log", "--no-walk=unsorted", "--format=%H%x00%at%x00%s"];
                args.extend(off.iter().copied());
                let text = git_out(repo, &args, None).unwrap_or_default();
                for (id, change) in parse_log(&text) {
                    if let Some(on_main) = m.by_change.get(&change) {
                        rebased.insert(id, on_main.clone());
                    }
                }
            }
        }
        RepoGit {
            resolved,
            main,
            running,
            console,
            rebased,
        }
    }

    pub fn facts(&self, sha: &str) -> Option<harbard_core::shipments::ShaFacts> {
        let commit = self.resolved.get(sha)?.clone();
        let landed = match &self.main {
            Some(m) if m.ids.contains(&commit) => Some(commit.clone()),
            _ => self.rebased.get(&commit).cloned(),
        };
        let installed = self.running.as_ref().map(|r| {
            landed.as_ref().is_some_and(|l| {
                r.ids.contains(l) && self.console.as_ref().is_none_or(|c| c.ids.contains(l))
            })
        });
        Some(harbard_core::shipments::ShaFacts {
            landed_as: landed.as_ref().filter(|l| **l != commit).cloned(),
            on_main: landed.is_some(),
            commit,
            installed,
        })
    }
}

/// Fill `v.shipments` and `v.running_commit`. Git runs only in a channel's
/// repository outside the protected folders, and only for SHAs a DONE named.
fn attach_shipments(
    v: &mut WorkspaceView,
    running: Option<&str>,
    console: Option<&str>,
    home: &str,
) {
    let repos: HashMap<String, RepoGit> = harbard_core::shipments::wanted(v)
        .into_iter()
        .filter(|(repo, _)| {
            !workspace::protected(repo, home) && Path::new(repo).join(".git").exists()
        })
        .map(|(repo, shas)| {
            let g = RepoGit::load(Path::new(&repo), &shas, running, console);
            (repo, g)
        })
        .collect();
    let facts = |repo: &str, sha: &str| repos.get(repo).and_then(|g| g.facts(sha));
    v.shipments = harbard_core::shipments::shipments(v, &facts);
    v.running_commit = running.map(str::to_string);
}

// ---------------------------------------------------------------------------
// The writes
// ---------------------------------------------------------------------------

/// What a write needs besides its body. `open` is the OS opener; tests pass
/// a recorder.
pub struct WriteCtx<'a> {
    pub store: &'a Store,
    pub events: &'a [Event],
    pub stale_secs: u64,
    pub now: u64,
    /// The team projection; the process-wide projector in serve.
    pub team: &'a dyn Fn() -> TeamView,
    pub open: &'a mut dyn FnMut(&Path, &[&str]) -> std::io::Result<()>,
    /// The workspace the console wrote from. Every target a write resolves
    /// (an agent, a channel's readers, an `@`, a card, a file) is looked up
    /// in that workspace's read only, so a write never reaches past it.
    pub gate: &'a Gate<'a>,
}

impl WriteCtx<'_> {
    /// The read this write resolves its targets in.
    pub(super) fn view(&self) -> WorkspaceView {
        view_in(
            self.store,
            self.events,
            self.stale_secs,
            &(self.team)(),
            self.now,
            self.gate,
        )
    }
}

fn field<'a>(body: &'a Value, name: &'static str) -> Result<&'a str, ApiError> {
    body.get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| ApiError::bad_request(name, format!("field `{name}` is required")))
}

fn cap(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// The in-process desk lock, for a desk file that is not `workspace.json`
/// (`profile.json`); its writer takes the cross-process lock file itself.
pub(super) fn desk_guard() -> std::sync::MutexGuard<'static, ()> {
    DESK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Read-modify-write of the desk: the in-process lock, then
/// [`DeskState::update`]'s cross-process one (`harbard theme` writes too).
pub(super) fn with_desk<T>(
    store: &Store,
    f: impl FnOnce(&mut DeskState) -> T,
) -> Result<T, ApiError> {
    let _guard = DESK.lock().unwrap_or_else(|e| e.into_inner());
    DeskState::update(&DeskState::path(store.root()), f)
        .map_err(|e| ApiError::new(500, "store_failed", format!("desk write failed: {e}")))
}

/// One write. `Ok(body)` is answered 200.
pub fn write(ctx: &mut WriteCtx<'_>, action: &str, body: &Value) -> Result<Value, ApiError> {
    match action {
        "resolve" | "reopen" => {
            let card = field(body, "card")?.to_string();
            let v = ctx.view();
            if !v.cards.iter().any(|c| c.id == card) {
                return Err(ApiError::new(
                    404,
                    "unknown_item",
                    "that card is not in the view",
                ));
            }
            let now = ctx.now;
            // "Was it you?" › Keep takes the same fresh confirmation as
            // the change it keeps: a bare POST must not close the backstop
            // (Clara's review of A2).
            let check = card.starts_with("check:");
            if check && action == "resolve" {
                let d = super::feedback_route::digest(&["resolve", &card]);
                if !super::feedback_route::confirmed(body, &d, now) {
                    return super::feedback_route::ask_confirmation(
                        d,
                        now,
                        json!({ "card": card }),
                    );
                }
            }
            with_desk(ctx.store, |d| {
                if action == "resolve" && check {
                    d.answer_check(&card, "kept", now);
                } else if action == "resolve" {
                    d.resolve(&card, now);
                } else {
                    if let Some(id) = card.strip_prefix("check:")
                        && let Some(c) = d.checks.iter_mut().find(|c| c.id == id)
                    {
                        c.outcome = None;
                        c.answered_ms = None;
                    }
                    d.reopen(&card);
                }
            })?;
            Ok(json!({ "ok": true, "card": card }))
        }
        // "Was it you?" › Revoke: undo the change the check card asks about,
        // and close the card. Keep is `resolve`.
        "check-revoke" => {
            let card = field(body, "card")?.to_string();
            let Some(check) = DeskState::load(ctx.store).check_of(&card).cloned() else {
                return Err(ApiError::new(
                    404,
                    "unknown_item",
                    "that card is not a check in the view",
                ));
            };
            match check.what.as_str() {
                "rule" => {
                    if let Some(id) = &check.rule {
                        let base = crate::serve::operator_note_event(
                            "",
                            None,
                            harbard_core::NoteKind::Fyi,
                            None,
                            ctx.now,
                        );
                        // Already retired is already revoked.
                        let _ = harbard_core::rules::retire(ctx.store, base, id);
                    }
                }
                "context" => {
                    let _guard = desk_guard();
                    let restored =
                        harbard_core::profile::Profile::update(ctx.store.root(), ctx.now, |p| {
                            p.context = check.previous.clone();
                            Ok::<(), std::convert::Infallible>(())
                        });
                    if let Err(e) = restored {
                        return Err(ApiError::new(500, "store_failed", e.to_string()));
                    }
                }
                "memory_sink" => {}
                other => {
                    return Err(ApiError::new(
                        422,
                        "unknown_check",
                        format!("no way to revoke \"{other}\""),
                    ));
                }
            }
            let now = ctx.now;
            with_desk(ctx.store, |d| {
                if check.what == "memory_sink" {
                    d.memory_sink_allowed = None;
                }
                d.answer_check(&card, "revoked", now);
            })?;
            Ok(json!({ "ok": true, "card": card, "revoked": check.what }))
        }
        "read" => {
            let agent = field(body, "agent")?.to_string();
            let v = ctx.view();
            if !v.agents.iter().any(|a| a.key == agent) {
                return Err(ApiError::new(
                    404,
                    "unknown_item",
                    "that agent is not in the view",
                ));
            }
            let ts = body
                .get("ts_ms")
                .and_then(Value::as_u64)
                .ok_or_else(|| ApiError::bad_request("ts_ms", "field `ts_ms` is required"))?
                .min(ctx.now);
            with_desk(ctx.store, |d| d.mark_read(&agent, ts))?;
            Ok(json!({ "ok": true }))
        }
        "save" => {
            let id = field(body, "id")?.to_string();
            let v = ctx.view();
            let item = saved_item(&v, &id, ctx.now)?;
            with_desk(ctx.store, |d| d.save_item(item))?;
            Ok(json!({ "ok": true, "id": id }))
        }
        "unsave" => {
            let id = field(body, "id")?.to_string();
            with_desk(ctx.store, |d| d.unsave(&id))?;
            Ok(json!({ "ok": true }))
        }
        "reply" => reply(ctx, body),
        "nudge" => nudge(ctx, body),
        "read-channel" => {
            let channel = field(body, "channel")?.to_string();
            let v = ctx.view();
            if !v.channels.iter().any(|c| c.id == channel) {
                return Err(ApiError::new(
                    404,
                    "unknown_item",
                    "that channel is not in the view",
                ));
            }
            let now = ctx.now;
            with_desk(ctx.store, |d| d.mark_channel_read(&channel, now))?;
            Ok(json!({ "ok": true, "channel": channel }))
        }
        "rename-agent" | "name-vote" => {
            let agent = field(body, "agent")?.to_string();
            let v = ctx.view();
            let a = v
                .agents
                .iter()
                .find(|a| a.key == agent && a.name_source.is_some())
                .ok_or_else(|| {
                    ApiError::new(404, "unknown_item", "that agent has no workshop name")
                })?;
            // Unique among every agent the console shows.
            let live_keys: Vec<String> = v.agents.iter().map(|x| x.key.clone()).collect();
            let live: std::collections::BTreeSet<&str> =
                live_keys.iter().map(String::as_str).collect();
            let now = ctx.now;
            let key = a.key.clone();
            if action == "rename-agent" {
                let name = field(body, "name")?.to_string();
                let out = harbard_core::workshop::Registry::update(ctx.store.root(), |r| {
                    r.rename(&key, &name, &live, now)
                })
                .map_err(|e| ApiError::new(500, "store_failed", e.to_string()))?;
                let name =
                    out.map_err(|why| ApiError::new(409, why, "that name is not available"))?;
                Ok(json!({ "ok": true, "agent": key, "name": name }))
            } else {
                let up = match body.get("vote").and_then(Value::as_str) {
                    Some("up") => true,
                    Some("down") => false,
                    _ => return Err(ApiError::bad_request("vote", "field `vote` is up or down")),
                };
                let name = harbard_core::workshop::Registry::update(ctx.store.root(), |r| {
                    r.vote(&key, up, &live, now)
                })
                .map_err(|e| ApiError::new(500, "store_failed", e.to_string()))?;
                Ok(json!({ "ok": true, "agent": key, "name": name }))
            }
        }
        "hide-agent" => {
            let agent = field(body, "agent")?.to_string();
            let v = ctx.view();
            if !v.agents.iter().any(|a| a.key == agent) {
                return Err(ApiError::new(
                    404,
                    "unknown_item",
                    "that agent is not in the view",
                ));
            }
            let now = ctx.now;
            with_desk(ctx.store, |d| d.hide_agent(&agent, now))?;
            Ok(json!({ "ok": true, "agent": agent }))
        }
        // Showing needs no view: an agent hidden long ago may be gone from it.
        "show-agent" => {
            let agent = field(body, "agent")?.to_string();
            with_desk(ctx.store, |d| d.show_agent(&agent))?;
            Ok(json!({ "ok": true, "agent": agent }))
        }
        "show-all-agents" => {
            with_desk(ctx.store, DeskState::show_all_agents)?;
            Ok(json!({ "ok": true }))
        }
        "agents-collapsed" => {
            let collapsed = body
                .get("collapsed")
                .and_then(Value::as_bool)
                .ok_or_else(|| {
                    ApiError::bad_request("collapsed", "field `collapsed` must be true or false")
                })?;
            // Open is the default, so it is stored as the absence of a choice.
            with_desk(ctx.store, |d| {
                d.agents_collapsed = collapsed.then_some(true)
            })?;
            Ok(json!({ "ok": true, "agents_collapsed": collapsed }))
        }
        "read-all" => {
            let now = ctx.now;
            with_desk(ctx.store, |d| d.mark_all_read(now))?;
            Ok(json!({ "ok": true, "read_all_ms": now }))
        }
        "resolve-all" | "reopen-all" => {
            let v = ctx.view();
            // A "Was it you?" card is answered one by one, confirmed.
            let ids: Vec<String> = open_cards_named(&v, body)?
                .into_iter()
                .filter(|id| !id.starts_with("check:"))
                .collect();
            let now = ctx.now;
            let resolve = action == "resolve-all";
            with_desk(ctx.store, |d| {
                for id in &ids {
                    if resolve {
                        d.resolve(id, now);
                    } else {
                        d.reopen(id);
                    }
                }
            })?;
            Ok(json!({ "ok": true, "cards": ids }))
        }
        "motion" => {
            let reduce = body.get("reduce").and_then(Value::as_bool).ok_or_else(|| {
                ApiError::bad_request("reduce", "field `reduce` must be true or false")
            })?;
            with_desk(ctx.store, |d| {
                d.motion = reduce.then(|| "reduce".to_string());
            })?;
            Ok(
                json!({ "ok": true, "motion": if reduce { Value::from("reduce") } else { Value::Null } }),
            )
        }
        "night-dim" => {
            let on = body
                .get("on")
                .and_then(Value::as_bool)
                .ok_or_else(|| ApiError::bad_request("on", "field `on` must be true or false"))?;
            // On is the default, so it is stored as the absence of a choice.
            with_desk(ctx.store, |d| d.night_dim = (!on).then_some(false))?;
            Ok(json!({ "ok": true, "night_dim": on }))
        }
        "open" => {
            let path = field(body, "path")?.to_string();
            let v = ctx.view();
            open_checked(&path, &workspace::file_set(&v), ctx.open)?;
            Ok(json!({ "ok": true, "path": path }))
        }
        fb if fb == "feedback"
            || fb == "feedback-undo"
            || fb == "memory-sink-allow"
            || fb.starts_with("rule-") =>
        {
            super::feedback_route::write(ctx, fb, body)
        }
        theme if theme.starts_with("theme-") => {
            super::theme_route::write(ctx.store, theme, body, ctx.open)
        }
        _ => Err(ApiError::new(
            404,
            "unknown_action",
            format!("no workspace action \"{action}\""),
        )),
    }
}

fn saved_item(v: &WorkspaceView, id: &str, now: u64) -> Result<SavedItem, ApiError> {
    if let Some(path) = id.strip_prefix("file:") {
        if !workspace::file_set(v).contains(path) {
            return Err(ApiError::new(
                404,
                "unknown_item",
                "that report is not in the view",
            ));
        }
        return Ok(SavedItem {
            id: id.to_string(),
            kind: "report".into(),
            saved_ms: now,
            agent: None,
            channel: None,
            text: path.rsplit('/').next().unwrap_or(path).to_string(),
            ts_ms: None,
            path: Some(path.to_string()),
        });
    }
    let m = v
        .messages
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| ApiError::new(404, "unknown_item", "that message is not in the view"))?;
    let agent = v
        .agents
        .iter()
        .find(|a| a.key == m.agent)
        .map(|a| a.name.clone());
    Ok(SavedItem {
        id: id.to_string(),
        kind: "message".into(),
        saved_ms: now,
        agent: agent.or_else(|| Some(m.author.clone())),
        channel: m.channels.first().cloned(),
        text: cap(&m.text, SAVED_TEXT_CHARS),
        ts_ms: Some(m.ts_ms),
        path: m.files.first().cloned(),
    })
}

/// Who an `@` in the operator's text reaches: `(session, agent)` where the
/// agent is an in-process one its lead relays to (`to_agent:`), or `None`
/// for a session that reads the bus itself. `@channel` is every reader of
/// `channel`.
fn mention_targets(
    v: &WorkspaceView,
    text: &str,
    channel: Option<&str>,
    anyway: bool,
) -> Vec<(String, Option<String>)> {
    let mut out: Vec<(String, Option<String>)> = Vec::new();
    let mut push = |t: (String, Option<String>)| {
        if !out.contains(&t) {
            out.push(t);
        }
    };
    for token in harbard_core::naming::mention_tokens(text) {
        if harbard_core::naming::CHANNEL_MENTIONS.contains(&token.as_str()) {
            if let Some(ch) = channel {
                for s in workspace::channel_route(v, ch, anyway) {
                    push((s, None));
                }
            }
            continue;
        }
        for a in v.agents.iter().filter(|a| {
            !a.temporary
                && !a.closed
                && (a.handle == token
                    || a.team.as_deref().map(str::to_lowercase).as_deref() == Some(token.as_str()))
        }) {
            let agent = (a.kind != "session").then(|| a.key.clone());
            push((a.lead_session.clone(), agent));
        }
    }
    out
}

/// The `to_agent:` field lines for the in-process agents a note is for.
fn to_agent_lines(agents: &[String]) -> String {
    agents
        .iter()
        .map(|k| format!("\n{} {k}", workspace::TO_AGENT_FIELD))
        .collect()
}

/// Store the operator's reply as notes on the bus, or refuse with why.
///
/// - To an agent: a note of kind `ask` (it wakes the reader and is owed an
///   answer). An in-process agent has no hooks, so the note goes to its lead
///   with `to_agent: <id>` and the lead relays it; a separately launched
///   session gets it directly.
/// - In a channel: a note per reader; an `fyi`, except for the readers an
///   `@` names, who get an `ask`.
/// - Anyone an `@` names gets an `ask` too, wherever they are.
fn reply(ctx: &mut WriteCtx<'_>, body: &Value) -> Result<Value, ApiError> {
    let text = field(body, "text")?;
    if let Some(why) = crate::serve::refuse_hidden_note(text) {
        return Err(ApiError::new(
            422,
            "invisible_chars",
            why["message"].as_str().unwrap_or_default().to_string(),
        ));
    }
    if text.chars().count() > REPLY_MAX_CHARS {
        return Err(ApiError::new(
            422,
            "too_long",
            format!("a reply holds at most {REPLY_MAX_CHARS} characters"),
        ));
    }
    let v = ctx.view();
    let reply_to = body
        .get("reply_to")
        .and_then(Value::as_str)
        .map(str::to_string);
    if let Some(target) = &reply_to
        && !v.messages.iter().any(|m| &m.id == target)
    {
        return Err(ApiError::new(
            404,
            "unknown_item",
            "that message is not in the view",
        ));
    }
    // "Enviar samemente": the operator was told nobody live reads it.
    let anyway = body.get("anyway").and_then(Value::as_bool) == Some(true);
    let channel = body.get("channel").and_then(Value::as_str);
    let mentioned = mention_targets(&v, text, channel, anyway);
    let mention_sessions: Vec<&str> = mentioned.iter().map(|(s, _)| s.as_str()).collect();
    // (lead session, agent key): who relays to which in-process agent. Each
    // lead's copy names only its own agents, so no lead is asked to relay to
    // another lead's agent, and no agent gets a note twice.
    let mut relays: Vec<(String, String)> = mentioned
        .iter()
        .filter_map(|(s, a)| a.clone().map(|a| (s.clone(), a)))
        .collect();
    let for_lead = |relays: &[(String, String)], session: &str| -> String {
        let mine: Vec<String> = relays
            .iter()
            .filter(|(s, _)| s == session)
            .map(|(_, k)| k.clone())
            .fold(Vec::new(), |mut out, k| {
                if !out.contains(&k) {
                    out.push(k);
                }
                out
            });
        to_agent_lines(&mine)
    };
    // (session, text, kind)
    let mut notes: Vec<(String, String, NoteKind)> = Vec::new();
    let path;
    if let Some(agent) = body.get("agent").and_then(Value::as_str) {
        let a = workspace::reply_route(&v, agent, anyway).map_err(|reason| {
            ApiError::new(
                409,
                "no_delivery_path",
                "this agent cannot receive messages from harbard",
            )
            .with("reason", json!(reason))
        })?;
        if a.kind != "session" {
            relays.insert(0, (a.lead_session.clone(), a.key.clone()));
        }
        let line = format!("{}{}: {text}", workspace::AGENT_NOTE_PREFIX, a.name);
        path = if a.kind == "session" {
            "direct"
        } else {
            "relay"
        };
        let lead_line = format!("{line}{}", for_lead(&relays, &a.lead_session));
        notes.push((a.lead_session.clone(), lead_line, NoteKind::Ask));
        for s in &mention_sessions {
            if notes.iter().all(|(n, _, _)| n != s) {
                let copy = format!("{line}{}", for_lead(&relays, s));
                notes.push((s.to_string(), copy, NoteKind::Ask));
            }
        }
    } else if let Some(channel) = channel {
        let readers = workspace::channel_route(&v, channel, anyway);
        if readers.is_empty() && mention_sessions.is_empty() {
            return Err(ApiError::new(
                409,
                "no_delivery_path",
                "no agent in this channel can receive messages from harbard",
            )
            .with("reason", json!("no_reachable_member")));
        }
        let line = format!("{}{channel}: {text}", workspace::CHANNEL_NOTE_PREFIX);
        path = "channel";
        for s in readers {
            let kind = if mention_sessions.contains(&s.as_str()) {
                NoteKind::Ask
            } else {
                NoteKind::Fyi
            };
            let copy = format!("{line}{}", for_lead(&relays, &s));
            notes.push((s, copy, kind));
        }
        for s in &mention_sessions {
            if notes.iter().all(|(n, _, _)| n != s) {
                let copy = format!("{line}{}", for_lead(&relays, s));
                notes.push((s.to_string(), copy, NoteKind::Ask));
            }
        }
    } else {
        return Err(ApiError::bad_request(
            "agent",
            "name an `agent` or a `channel`",
        ));
    }
    let mut stored = Vec::new();
    for (session, line, kind) in notes {
        let to = Recipient {
            kind: RecipientKind::Session,
            value: session.clone(),
        };
        let event = super::operator_note_event(&line, Some(to), kind, None, ctx.now);
        event
            .to_line()
            .map_err(|e| ApiError::new(422, "too_long", e.to_string()))?;
        ctx.store
            .append(&event)
            .map_err(|e| ApiError::new(500, "store_failed", format!("append failed: {e}")))?;
        stored.push(json!({ "note": event.id, "session": session, "kind": kind.as_str() }));
        if let Some(target) = &reply_to {
            let id = event.id.clone();
            let target = target.clone();
            with_desk(ctx.store, |d| {
                d.link_thread(&id, &target);
            })?;
        }
    }
    Ok(json!({ "ok": true, "path": path, "stored": stored }))
}

/// The cards a "Resolve all" names that are open in the view.
fn open_cards_named(v: &WorkspaceView, body: &Value) -> Result<Vec<String>, ApiError> {
    let ids = body.get("cards").and_then(Value::as_array).ok_or_else(|| {
        ApiError::bad_request("cards", "field `cards` must be a list of card ids")
    })?;
    Ok(ids
        .iter()
        .filter_map(Value::as_str)
        .filter(|id| v.cards.iter().any(|c| c.id == *id))
        .map(str::to_string)
        .collect())
}

/// Longest quote of the original note in a re-send.
const NUDGE_QUOTE_CHARS: usize = 600;

/// "Resend to the lead": the note of an open "no reply" card,
/// sent again as an `ask` (which wakes an idle session and is owed an
/// answer) with a `ref` to each copy, to every session it was stored for.
fn nudge(ctx: &mut WriteCtx<'_>, body: &Value) -> Result<Value, ApiError> {
    let card = field(body, "card")?.to_string();
    let v = ctx.view();
    let c = v
        .cards
        .iter()
        .find(|c| c.id == card && c.kind == "unanswered")
        .ok_or_else(|| ApiError::new(404, "unknown_item", "that card is not in the view"))?;
    let m = c
        .message
        .as_deref()
        .and_then(|id| v.messages.iter().find(|m| m.id == id))
        .ok_or_else(|| ApiError::new(404, "unknown_item", "that message is not in the view"))?;
    // One re-send per window, whatever the clicks.
    let window = DeskState::load(ctx.store).config.unanswered_min.max(1) * 60_000;
    if let Some(at) = m.delivery.as_ref().and_then(|d| d.nudged_ms)
        && ctx.now.saturating_sub(at) < window
    {
        return Err(ApiError::new(
            409,
            "already_nudged",
            "this note was re-sent a moment ago; it waits for an answer",
        )
        .with("nudged_ms", json!(at)));
    }
    let original = m.id.strip_prefix("note:").unwrap_or(&m.id);
    // Every copy of the note: a channel note is stored once per session.
    let first = ctx
        .events
        .iter()
        .find(|e| e.id == original)
        .ok_or_else(|| ApiError::new(404, "unknown_item", "that note is not on the bus"))?;
    let all: Vec<&Event> = ctx
        .events
        .iter()
        .filter(|e| {
            note_session(e).is_some()
                && e.note_ref.is_none()
                && e.text == first.text
                && query::event_ms(e).abs_diff(query::event_ms(first)) < 2_000
        })
        .collect();
    // To the coordinator: the copies stored for a lead session, and the
    // others only when none was.
    let leads: std::collections::HashSet<&str> = v
        .agents
        .iter()
        .filter(|a| a.kind != "session")
        .map(|a| a.lead_session.as_str())
        .collect();
    let to_leads: Vec<&Event> = all
        .iter()
        .copied()
        .filter(|e| note_session(e).is_some_and(|s| leads.contains(s)))
        .collect();
    let copies = if to_leads.is_empty() { all } else { to_leads };
    let minutes = (ctx.now.saturating_sub(m.ts_ms) + 30_000) / 60_000;
    let place = match m.channels.first() {
        Some(ch) if m.agent.is_empty() => format!("in #{ch}"),
        _ => "to one of your agents".to_string(),
    };
    let quote = cap(&m.text, NUDGE_QUOTE_CHARS);
    let mut stored = Vec::new();
    for copy in copies {
        let Some(session) = note_session(copy) else {
            continue;
        };
        let text = format!(
            "{} wrote {place} {minutes} min ago and nobody has answered: \"{quote}\". Answer with harbard ack last --note \"…\" (or harbard note --ref {} \"…\").",
            v.operator, copy.id
        );
        let to = Recipient {
            kind: RecipientKind::Session,
            value: session.to_string(),
        };
        let event = super::operator_note_event(
            &text,
            Some(to),
            NoteKind::Ask,
            Some(copy.id.clone()),
            ctx.now,
        );
        event
            .to_line()
            .map_err(|e| ApiError::new(422, "too_long", e.to_string()))?;
        ctx.store
            .append(&event)
            .map_err(|e| ApiError::new(500, "store_failed", format!("append failed: {e}")))?;
        stored.push(json!({ "note": event.id, "session": session, "ref": copy.id }));
    }
    Ok(json!({ "ok": true, "stored": stored }))
}

/// Open `path` with the OS only if the view showed it as a file card.
pub fn open_checked(
    path: &str,
    shown: &std::collections::BTreeSet<String>,
    open: &mut dyn FnMut(&Path, &[&str]) -> std::io::Result<()>,
) -> Result<(), ApiError> {
    if !workspace::openable(path) || !shown.contains(path) {
        return Err(ApiError::new(
            403,
            "not_a_file_card",
            "only a file the workspace shows as a card can be opened",
        ));
    }
    // Only now, on the operator's own click, is the file looked at: never a
    // symlink, and whatever it resolves to must still be a card type.
    let p = PathBuf::from(path);
    let refused = || {
        ApiError::new(
            403,
            "not_a_file_card",
            "only a plain file the workspace shows as a card can be opened",
        )
    };
    let meta = std::fs::symlink_metadata(&p)
        .map_err(|_| ApiError::new(404, "missing_file", "the file is not on this Mac any more"))?;
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(refused());
    }
    let real = std::fs::canonicalize(&p).map_err(|_| refused())?;
    if !workspace::openable(&real.display().to_string()) || !real.is_file() {
        return Err(refused());
    }
    open(&p, &opener_args(path))
        .map_err(|e| ApiError::new(500, "open_failed", format!("the OS opener failed: {e}")))
}

/// Text reports open in the default text editor, which always exists, so
/// the OS never asks which application to use.
fn opener_args(path: &str) -> Vec<&'static str> {
    let lower = path.to_ascii_lowercase();
    if lower.ends_with(".md") || lower.ends_with(".json") {
        vec!["-t"]
    } else {
        Vec::new()
    }
}

/// The real opener: `open` on macOS, `xdg-open` elsewhere. Waits for it to
/// hand the file over; never reads the file itself.
pub fn os_open(path: &Path, args: &[&str]) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("/usr/bin/open");
        c.args(args);
        c
    } else {
        Command::new("xdg-open")
    };
    let status = cmd
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other(format!("exit {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip_placement(ids: &[&str]) -> Placement {
        let mut set = harbard_core::workspaces::Workspaces::single();
        set.default = ids[0].to_string();
        set.workspaces = ids
            .iter()
            .map(|id| harbard_core::workspaces::WorkspaceDef {
                id: (*id).to_string(),
                name: (*id).to_string(),
                icon: String::new(),
                owners: Vec::new(),
            })
            .collect();
        Placement::with(Some(set), |_| Default::default())
    }

    fn paired(access: harbard_core::Access, grants: &[&str]) -> super::super::access::Caller {
        let mut card = harbard_core::LocalDevice {
            id: "d".into(),
            name: "d".into(),
            role: harbard_core::DeviceRole::Satellite,
        }
        .to_card(0, vec![]);
        card.access = access;
        card.workspaces = grants.iter().map(|g| (*g).to_string()).collect();
        super::super::access::Caller::Device(card)
    }

    #[test]
    fn the_whole_hub_read_is_only_for_who_sees_every_workspace() {
        use super::super::access::Caller;
        use harbard_core::Access;
        let two = strip_placement(&["personal", "work"]);
        let all = Some(ALL_WORKSPACES);
        assert!(whole_hub(&two, &Caller::Local, all));
        assert!(whole_hub(&two, &paired(Access::Operator, &[]), all));
        assert!(!whole_hub(&two, &Caller::Local, None));
        assert!(!whole_hub(&two, &Caller::Local, Some("work")));

        let granted = paired(Access::Workshop, &["work"]);
        assert!(!whole_hub(&two, &granted, all), "a grant is never widened");
        // It reads as if it had not asked: its own workspace, not a 404.
        assert_eq!(
            select(&two, &granted, all).unwrap().as_deref(),
            Some("work")
        );

        let named = strip_placement(&["personal", ALL_WORKSPACES]);
        assert!(!whole_hub(&named, &Caller::Local, all));
        assert_eq!(
            select(&named, &Caller::Local, all).unwrap().as_deref(),
            Some(ALL_WORKSPACES),
            "a workspace really called that stays reachable"
        );
    }

    #[test]
    fn each_row_of_the_whole_hub_read_says_its_workspace() {
        let mut rows = json!([{"key": "a"}, {"key": "b"}, {"key": "gone"}]);
        tag_home(&mut rows, "key", &HashSet::from(["a"]), "work");
        tag_home(&mut rows, "key", &HashSet::from(["b"]), "personal");
        assert_eq!(rows[0]["workspace"], "work");
        assert_eq!(rows[1]["workspace"], "personal");
        assert!(rows[2].get("workspace").is_none());
    }
    use std::collections::BTreeSet;
    use std::sync::atomic::{AtomicU64, Ordering};

    static N: AtomicU64 = AtomicU64::new(0);

    /// Throwaway directory, removed on drop.
    struct TempHome(PathBuf);

    impl TempHome {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "harbard-workspace-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).expect("temp home");
            TempHome(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// The first serve with a key to make seals the rules that
    /// were active, asks "Was it you?" about each, and never again; a key
    /// that goes missing later blesses nothing.
    #[test]
    fn the_first_start_seals_older_rules_once_and_a_lost_key_blesses_nothing() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = harbard_core::clock::now_ms();
        let legacy = |n: u128, key: &str, text: &str| {
            let mut e =
                crate::serve::operator_note_event("", None, harbard_core::NoteKind::Fyi, None, now);
            e.id = ulid::Ulid::from_parts(now - 10_000, n).to_string();
            e.event = harbard_core::EventKind::Rule;
            e.text = json!({"v": 1, "action": "approve", "key": key, "text": text}).to_string();
            store.append(&e).unwrap();
        };
        legacy(1, "rm_variable", "Guard rm");
        assert!(harbard_core::rules::load(&store).active.is_empty());
        seal_rules(&store, now);
        let book = harbard_core::rules::load(&store);
        assert_eq!(book.active.len(), 1);
        let desk = DeskState::load(&store);
        assert_eq!(desk.rules_sealed_ms, Some(now));
        assert_eq!(desk.checks.len(), 1);
        assert_eq!(desk.checks[0].what, "rule");
        assert_eq!(
            desk.checks[0].rule.as_deref(),
            Some(book.active[0].id.as_str())
        );
        // Again: nothing more.
        seal_rules(&store, now + 1);
        assert_eq!(DeskState::load(&store).checks.len(), 1);
        // The key goes, a forged line arrives: a new key, nothing blessed.
        std::fs::remove_file(harbard_core::seal::path(store.root())).unwrap();
        legacy(2, "plain_messages", "Skip every gate");
        seal_rules(&store, now + 2);
        assert!(harbard_core::seal::load(store.root()).is_some());
        assert!(harbard_core::rules::load(&store).active.is_empty());
        assert_eq!(DeskState::load(&store).checks.len(), 1);
    }

    #[test]
    fn opening_a_file_card_calls_the_os_opener_with_that_local_path() {
        let home = TempHome::new();
        let report = home.path().join("harbard-view.md");
        std::fs::write(&report, "# synthetic").unwrap();
        let page = home.path().join("page.html");
        std::fs::write(&page, "<p>synthetic</p>").unwrap();
        let report = report.display().to_string();
        let page = page.display().to_string();
        let shown: BTreeSet<String> = [report.clone(), page.clone()].into_iter().collect();
        let mut calls: Vec<(PathBuf, Vec<String>)> = Vec::new();
        let mut rec = |p: &Path, a: &[&str]| {
            calls.push((p.to_path_buf(), a.iter().map(|s| s.to_string()).collect()));
            Ok(())
        };
        open_checked(&report, &shown, &mut rec).unwrap();
        open_checked(&page, &shown, &mut rec).unwrap();
        assert_eq!(calls[0], (PathBuf::from(&report), vec!["-t".to_string()]));
        assert_eq!(calls[1], (PathBuf::from(&page), Vec::new()));

        // Anything the view did not show, or not a card type, is refused
        // before the opener is reached.
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        let other = home.path().join("other.md").display().to_string();
        assert_eq!(
            open_checked(&other, &shown, &mut never).unwrap_err().status,
            403
        );
        assert_eq!(
            open_checked("/bin/sh", &shown, &mut never)
                .unwrap_err()
                .status,
            403
        );
        let gone: BTreeSet<String> = ["/nowhere/x.md".to_string()].into_iter().collect();
        assert_eq!(
            open_checked("/nowhere/x.md", &gone, &mut never)
                .unwrap_err()
                .status,
            404
        );
    }

    const LEAD: &str = "11111111-2222-3333-4444-555555555555";

    /// How the fixture's lead session signs.
    fn lead_callsign() -> String {
        harbard_core::alias::display(&harbard_core::alias::preferred(LEAD))
    }

    /// A real git repository named `orchard`, outside temp space (serve
    /// never makes a channel of a temp dir): beside the test binary.
    fn test_repo() -> String {
        static REPO: OnceLock<String> = OnceLock::new();
        REPO.get_or_init(|| {
            // Inside the crate, not beside the test binary: a target directory
            // on an external volume (`target` -> /Volumes/...) is a protected
            // folder, where nothing is a channel, and every test built on this
            // repository failed with it. `target/` is ignored by git.
            let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target")
                .join("ws-test-repos")
                .join("orchard");
            std::fs::create_dir_all(dir.join(".git")).unwrap();
            dir.display().to_string()
        })
        .clone()
    }

    fn synthetic_team(now: u64) -> TeamView {
        use harbard_core::team::{AgentRow, CheckoutUse, FormatProbe, LeadPanel, MessageRow};
        let first = "BLOCKED · keychain · Yes or no?";
        let row = AgentRow {
            id: "a1".into(),
            name: Some("keychain".into()),
            kind: "teammate",
            agent_type: None,
            color: None,
            model: None,
            description: None,
            brief: None,
            state: "idle",
            state_reason: "turn_ended",
            last_action_ms: Some(now - 60_000),
            seen_ms: now - 60_000,
            records: 3,
            checkouts: vec![CheckoutUse {
                path: test_repo(),
                writes: 1,
                first_ms: now - 600_000,
                last_ms: now - 300_000,
            }],
            unplaced_writes: 0,
            worktree: None,
            messages: vec![MessageRow {
                dir: "out",
                peer: "team-lead".into(),
                ts_ms: now - 120_000,
                first_line: first.into(),
                label: Some("BLOCKED".into()),
                chars: first.chars().count(),
                report: None,
                repeat: false,
                undelivered: false,
                structured: None,
                digest: "0000000000000001".into(),
                shas: Vec::new(),
                body: None,
                paths: Vec::new(),
            }],
            messages_total: 1,
            last_label: None,
            first_ms: Some(now - 900_000),
            git_roots: Vec::new(),
            write_dirs: Vec::new(),
            git_dirs: Vec::new(),
            target_dirs: Vec::new(),
            activity: harbard_core::activity::Activity::plain("idle", now - 60_000),
            progress: Default::default(),
        };
        TeamView {
            v: 1,
            now_ms: now,
            window_ms: harbard_core::team::WINDOW_DEFAULT_MS,
            quiet_running_ms: 900_000,
            root_found: true,
            leads: vec![LeadPanel {
                session: LEAD.into(),
                project: "-p".into(),
                cwd: None,
                last_write_ms: now,
                lead_last_ms: Some(now),
                lead_found: true,
                agents: vec![row],
                warnings: Vec::new(),
                skipped_lines: 0,
                lead_write_dirs: Vec::new(),
                lead_cmd_dirs: Vec::new(),
                git_cwds: vec![test_repo()],
            }],
            format: FormatProbe::default(),
        }
    }

    fn hello(store: &Store, session: &str, now: u64) {
        use harbard_core::SCHEMA_VERSION;
        store
            .append(&Event {
                v: SCHEMA_VERSION,
                id: ulid::Ulid::from_parts(now - 1_000, 7).to_string(),
                ts: clock::rfc3339_utc(now - 1_000),
                event: EventKind::Hello,
                session: session.into(),
                harness: "claude-code".into(),
                host: "h".into(),
                cwd: "/nowhere/lead".into(),
                repo: None,
                repo_key: None,
                branch: None,
                role: Role::Main,
                parent: None,
                note_ref: None,
                text: String::new(),
                to: None,
                kind: NoteKind::Fyi,
            })
            .unwrap();
    }

    fn all_events(store: &Store) -> Vec<Event> {
        store.read(harbard_core::Window::All).events
    }

    #[test]
    fn a_reply_is_a_note_to_the_lead_session_and_answers_the_blocked_card() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        hello(&store, LEAD, now);
        let team = || synthetic_team(now);
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        let events = all_events(&store);
        let before = view(&store, &events, 900, &team(), now);
        let blocked = before
            .cards
            .iter()
            .find(|c| c.kind == "blocked")
            .unwrap()
            .clone();
        let mut ctx = WriteCtx {
            gate: &harbard_core::workspaces::Gate::OPEN,
            store: &store,
            events: &events,
            stale_secs: 900,
            now,
            team: &team,
            open: &mut never,
        };
        let out = write(
            &mut ctx,
            "reply",
            &json!({"agent":"a1","text":"Yes, twice.","reply_to":blocked.message}),
        )
        .unwrap();
        assert_eq!(out["stored"][0]["session"], LEAD);

        // Exactly one new event: an operator note to the lead, nothing else.
        let after = all_events(&store);
        assert_eq!(after.len(), events.len() + 1);
        let note = after.last().unwrap();
        assert_eq!(note.event, EventKind::Note);
        assert_eq!(note.role, Role::Operator);
        assert_eq!(note.to.as_ref().unwrap().kind, RecipientKind::Session);
        assert_eq!(note.to.as_ref().unwrap().value, LEAD);
        // An in-process agent: an ask to its lead, which relays it, under
        // the workshop name the console shows.
        let shown = before
            .agents
            .iter()
            .find(|a| a.key == "a1")
            .unwrap()
            .name
            .clone();
        assert_eq!(note.text, format!("To {shown}: Yes, twice.\nto_agent: a1"));
        assert_eq!(note.kind, NoteKind::Ask);
        assert_eq!(out["path"], "relay");

        let v = view(&store, &after, 900, &team(), now);
        assert!(v.cards.iter().all(|c| c.kind != "blocked"), "answered");
        let m = v.messages.iter().find(|m| m.from == "operator").unwrap();
        assert_eq!(m.reply_to, blocked.message);
    }

    #[test]
    fn with_no_delivery_path_nothing_is_stored() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        let team = || synthetic_team(now);
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        let events = all_events(&store);
        let mut ctx = WriteCtx {
            gate: &harbard_core::workspaces::Gate::OPEN,
            store: &store,
            events: &events,
            stale_secs: 900,
            now,
            team: &team,
            open: &mut never,
        };
        let err = write(&mut ctx, "reply", &json!({"agent":"a1","text":"hello"})).unwrap_err();
        assert_eq!(err.status, 409);
        assert_eq!(err.code, "no_delivery_path");
        let err = write(
            &mut ctx,
            "reply",
            &json!({"channel":"orchard","text":"hello"}),
        )
        .unwrap_err();
        assert_eq!(err.status, 409);
        assert!(all_events(&store).is_empty());
        let v = view(&store, &[], 900, &team(), now);
        assert_eq!(v.agents[0].delivery, "not_on_bus");
    }

    #[test]
    fn mark_resolved_persists_across_a_serve_restart() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        let team = || synthetic_team(now);
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        let card = view(&store, &[], 900, &team(), now).cards[0].id.clone();
        let mut ctx = WriteCtx {
            gate: &harbard_core::workspaces::Gate::OPEN,
            store: &store,
            events: &[],
            stale_secs: 900,
            now,
            team: &team,
            open: &mut never,
        };
        write(&mut ctx, "resolve", &json!({"card": card})).unwrap();
        // A restarted serve has no memory but the files: a fresh store handle.
        let reopened = Store::at(home.path());
        let v = view(&reopened, &[], 900, &team(), now + 1);
        assert_eq!(v.cards[0].resolved_ms, Some(now));
        assert_eq!(v.unread.activity, 0);
        assert!(all_events(&reopened).is_empty(), "nothing went to the bus");
        let mut ctx = WriteCtx {
            gate: &harbard_core::workspaces::Gate::OPEN,
            store: &reopened,
            events: &[],
            stale_secs: 900,
            now,
            team: &team,
            open: &mut never,
        };
        write(&mut ctx, "save", &json!({"id": v.messages[0].id})).unwrap();
        let v = view(&reopened, &[], 900, &team(), now + 2);
        assert_eq!(v.saved.len(), 1);
        // The cutoff is set at serve startup (init_desk), never by a view.
        let desk = DeskState::load(&reopened);
        assert_eq!(desk.config.protocol_since_ms, 0);
        assert_eq!(v.operator, harbard_core::workspace::DEFAULT_OPERATOR_NAME);
        assert_eq!(v.saved[0].text, "Yes or no?");
        assert_eq!(
            write(&mut ctx, "save", &json!({"id": "nope"}))
                .unwrap_err()
                .status,
            404
        );
        assert_eq!(
            write(&mut ctx, "launch", &json!({})).unwrap_err().status,
            404
        );
    }

    /// The reviewer's probe, inverted: a card that is a symlink to a
    /// command never reaches the OS opener, nor does one that resolves to
    /// another type through a symlinked directory.
    #[test]
    fn review_probe_a_symlinked_card_never_reaches_the_opener() {
        let home = TempHome::new();
        let payload = home.path().join("payload.command");
        std::fs::write(&payload, "#!/bin/sh\necho synthetic\n").unwrap();
        let card = home.path().join("report.html");
        std::os::unix::fs::symlink(&payload, &card).unwrap();
        let dir = home.path().join("linked");
        std::fs::create_dir_all(home.path().join("real")).unwrap();
        std::fs::write(home.path().join("real/notes.md"), "# synthetic").unwrap();
        std::os::unix::fs::symlink(home.path().join("real"), &dir).unwrap();
        let through = dir.join("notes.md");
        let card = card.display().to_string();
        let through = through.display().to_string();
        let shown: BTreeSet<String> = [card.clone(), through.clone()].into_iter().collect();
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        assert_eq!(
            open_checked(&card, &shown, &mut never).unwrap_err().code,
            "not_a_file_card"
        );
        // A symlink to another report of the same type is refused as well.
        let alias = home.path().join("alias.md");
        std::os::unix::fs::symlink(home.path().join("real/notes.md"), &alias).unwrap();
        let alias = alias.display().to_string();
        let shown: BTreeSet<String> = [alias.clone(), through.clone()].into_iter().collect();
        assert_eq!(
            open_checked(&alias, &shown, &mut never).unwrap_err().code,
            "not_a_file_card"
        );
        // A real file reached through a symlinked folder keeps its type and opens.
        let mut opened = Vec::new();
        let mut rec = |p: &Path, _: &[&str]| {
            opened.push(p.to_path_buf());
            Ok(())
        };
        open_checked(&through, &shown, &mut rec).unwrap();
        assert_eq!(opened.len(), 1);
    }

    #[test]
    fn a_get_never_writes_the_desk_and_startup_initialises_it_once() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        let _ = view(&store, &[], 900, &synthetic_team(now), now);
        assert!(
            !DeskState::path(store.root()).exists(),
            "GET wrote the desk"
        );
        init_desk(&store, now);
        let first = DeskState::load(&store);
        assert_eq!(
            (first.baseline_ms, first.config.protocol_since_ms),
            (now, now)
        );
        init_desk(&store, now + 5_000);
        assert_eq!(
            DeskState::load(&store),
            first,
            "a restart keeps the first values"
        );
    }

    #[test]
    fn desk_writes_take_only_ids_the_view_holds() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        hello(&store, LEAD, now);
        let events = all_events(&store);
        let team = || synthetic_team(now);
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        let mut ctx = WriteCtx {
            gate: &harbard_core::workspaces::Gate::OPEN,
            store: &store,
            events: &events,
            stale_secs: 900,
            now,
            team: &team,
            open: &mut never,
        };
        for (action, body) in [
            ("resolve", json!({"card": "bogus"})),
            ("reopen", json!({"card": "bogus"})),
            ("read", json!({"agent": "bogus", "ts_ms": now})),
            (
                "reply",
                json!({"agent": "a1", "text": "hello", "reply_to": "bogus"}),
            ),
        ] {
            assert_eq!(
                write(&mut ctx, action, &body).unwrap_err().status,
                404,
                "{action}"
            );
        }
        assert_eq!(
            all_events(&store).len(),
            events.len(),
            "a refused reply stores nothing"
        );
        let desk = DeskState::load(&store);
        assert!(desk.resolved.is_empty() && desk.reads.is_empty() && desk.threads.is_empty());
    }

    #[test]
    fn removed_worktrees_fold_into_their_repository_by_name() {
        use harbard_core::workspace::by_convention;
        assert_eq!(by_convention("/w/orchard-wt-identity"), "/w/orchard");
        assert_eq!(
            by_convention("/w/harbard/.claude/worktrees/agent-1"),
            "/w/harbard"
        );
        assert_eq!(by_convention("/w/contoso"), "/w/contoso");
    }

    /// Records every path the resolver hands to the filesystem.
    #[derive(Default)]
    struct Recorder(Mutex<Vec<PathBuf>>);

    impl Recorder {
        fn touched(&self) -> Vec<PathBuf> {
            self.0.lock().unwrap().clone()
        }
    }

    impl Fs for Recorder {
        fn exists(&self, path: &Path) -> bool {
            self.0.lock().unwrap().push(path.to_path_buf());
            RealFs.exists(path)
        }
        fn read_to_string(&self, path: &Path) -> Option<String> {
            self.0.lock().unwrap().push(path.to_path_buf());
            RealFs.read_to_string(path)
        }
        fn canonicalize(&self, path: &Path) -> Option<PathBuf> {
            self.0.lock().unwrap().push(path.to_path_buf());
            RealFs.canonicalize(path)
        }
        fn len_under(&self, root: &Path, dirs: &[&str], name: &str) -> Option<u64> {
            let mut p = root.to_path_buf();
            self.0.lock().unwrap().push(p.clone());
            for d in dirs {
                p = p.join(d);
            }
            self.0.lock().unwrap().push(p.join(name));
            RealFs.len_under(root, dirs, name)
        }
        fn read_under(&self, root: &Path, dirs: &[&str], name: &str, cap: u64) -> Option<Vec<u8>> {
            let mut p = root.to_path_buf();
            self.0.lock().unwrap().push(p.clone());
            for d in dirs {
                p = p.join(d);
            }
            self.0.lock().unwrap().push(p.join(name));
            RealFs.read_under(root, dirs, name, cap)
        }
    }

    fn git_repo(root: &Path, branch: &str) {
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(
            root.join(".git/HEAD"),
            format!("ref: refs/heads/{branch}\n"),
        )
        .unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
    }

    #[test]
    fn a_known_worktree_resolves_to_its_primary_checkout_from_git_files() {
        let home = TempHome::new();
        let primary = home.path().join("repo");
        let wt_git = primary.join(".git/worktrees/repo-wt-x");
        std::fs::create_dir_all(&wt_git).unwrap();
        std::fs::write(primary.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        std::fs::write(wt_git.join("HEAD"), "ref: refs/heads/feature\n").unwrap();
        std::fs::write(wt_git.join("commondir"), "../..\n").unwrap();
        let wt = home.path().join("repo-wt-x");
        std::fs::create_dir_all(wt.join("src")).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", wt_git.display())).unwrap();
        let known = |p: &Path| Known {
            root: p.display().to_string(),
            repo: None,
            branch: None,
        };
        let r = Resolver {
            home: "/nohome".into(),
            temp_roots: &[],
            known: vec![known(&primary), known(&wt)],
            fs: &RealFs,
        };
        let canon = std::fs::canonicalize(&primary)
            .unwrap()
            .display()
            .to_string();
        let info = r.resolve(&wt.join("src").display().to_string()).unwrap();
        assert_eq!(info.repo, canon);
        assert_eq!(info.checkout, wt.display().to_string());
        assert_eq!(info.branch.as_deref(), Some("feature"));
        let info = r.resolve(&primary.display().to_string()).unwrap();
        assert_eq!(info.branch.as_deref(), Some("main"));
        // Unknown, but named like a worktree: its repository by name.
        let gone = home.path().join("repo-wt-gone/src").display().to_string();
        let info = r.resolve(&gone).unwrap();
        assert_eq!(
            info.checkout,
            home.path().join("repo-wt-gone").display().to_string()
        );
        assert_eq!(info.repo, primary.display().to_string());
        // Unknown and unnamed: no channel. Temp space and ~/.claude: never.
        assert!(
            r.resolve(&home.path().join("notes").display().to_string())
                .is_none()
        );
        let temp = Resolver {
            temp_roots: &TEMP_ROOTS,
            ..r
        };
        assert!(temp.resolve("/private/tmp/x/clone/src").is_none());
        let fake_home = home.path().display().to_string();
        std::fs::create_dir_all(home.path().join(".claude/.git")).unwrap();
        let claude = Resolver {
            home: fake_home.clone(),
            temp_roots: &[],
            known: vec![known(&home.path().join(".claude"))],
            fs: &RealFs,
        };
        assert!(
            claude
                .resolve(&format!("{fake_home}/.claude/memory"))
                .is_none()
        );
    }

    /// H2: serve must never stat or read under the TCC-protected folders or
    /// /Volumes, known root or not; and rendering file cards stats nothing.
    #[test]
    fn the_workspace_read_never_touches_tcc_protected_folders() {
        let tmp = TempHome::new();
        let home = tmp.path().display().to_string();
        let h = |rel: &str| format!("{home}/{rel}");
        for rel in ["Documents/proj", "Desktop/d-wt-a", "Projects/repo"] {
            git_repo(&tmp.path().join(rel), "main");
        }
        std::fs::create_dir_all(
            tmp.path()
                .join("Library/Mobile Documents/com~apple~CloudDocs/x/.git"),
        )
        .unwrap();
        std::fs::write(tmp.path().join("Documents/report.md"), "# synthetic").unwrap();
        let rec = Recorder::default();
        let r = Resolver {
            home: home.clone(),
            temp_roots: &[],
            known: vec![
                Known {
                    root: h("Documents/proj"),
                    repo: Some(h("Documents/proj")),
                    branch: Some("main".into()),
                },
                Known {
                    root: h("Projects/repo"),
                    repo: None,
                    branch: None,
                },
            ],
            fs: &rec,
        };
        // Under a protected folder nothing is a channel: not a known root,
        // not a worktree by its name (`# v7`, `# drafts`).
        assert!(r.resolve(&h("Documents/proj/src")).is_none());
        assert!(r.resolve(&h("Documents/foo-wt-b/src")).is_none());
        assert!(r.resolve(&h("Desktop/d-wt-a")).is_none());
        assert!(r.resolve(&h("Documents/other/x")).is_none());
        assert!(
            r.resolve(&h("Library/Mobile Documents/com~apple~CloudDocs/x/y"))
                .is_none()
        );
        assert!(r.resolve("/Volumes/ext/repo/src").is_none());
        for rel in ["Downloads/a", "Pictures/a", "Movies/a", "Music/a"] {
            assert!(r.resolve(&h(rel)).is_none());
        }
        // Home itself known as a root (a session started there) still never
        // reaches into Documents; a repository nested under a known parent is
        // found by its own .git.
        let wide = Resolver {
            home: home.clone(),
            temp_roots: &[],
            known: vec![
                Known {
                    root: home.clone(),
                    repo: None,
                    branch: None,
                },
                Known {
                    root: h("Projects"),
                    repo: None,
                    branch: None,
                },
            ],
            fs: &rec,
        };
        assert!(wide.resolve(&h("Documents/other/x")).is_none());
        assert_eq!(
            wide.resolve(&h("Projects/repo/src")).unwrap().checkout,
            h("Projects/repo")
        );
        // A known root elsewhere is read from disk.
        assert_eq!(
            r.resolve(&h("Projects/repo/src"))
                .unwrap()
                .branch
                .as_deref(),
            Some("main")
        );
        assert!(
            rec.touched()
                .iter()
                .any(|p| p.starts_with(tmp.path().join("Projects")))
        );

        // The whole read, with a file card and writes under Documents.
        let store_home = TempHome::new();
        let store = Store::at(store_home.path());
        let now = clock::now_ms();
        let mut team = synthetic_team(now);
        let row = &mut team.leads[0].agents[0];
        row.checkouts[0].path = h("Documents/proj");
        row.write_dirs = vec![harbard_core::team::CheckoutUse {
            path: h("Documents/proj/src"),
            writes: 1,
            first_ms: now - 600_000,
            last_ms: now - 300_000,
        }];
        row.git_dirs = vec![h("Library/Mobile Documents/com~apple~CloudDocs/x")];
        row.messages[0].first_line =
            format!("DONE · keychain · report {}", h("Documents/report.md"));
        row.messages[0].report = Some(h("Documents/report.md"));
        team.leads[0].git_cwds = vec![h("Documents/proj")];
        let disk = Disk {
            fs: &rec,
            cached: false,
            home: &home,
            temp_roots: &[],
        };
        let v = view_with(&store, &[], 900, &team, now, &disk);
        assert!(
            v.messages[0].files.contains(&h("Documents/report.md")),
            "the card is still shown"
        );
        assert!(
            !v.channels.iter().any(|c| c.id == "proj"),
            "no channel from a protected folder"
        );

        // The faces' slice (`?only=activity`) is the same read: it touches
        // no path the full view did not.
        let before: std::collections::BTreeSet<PathBuf> = rec.touched().into_iter().collect();
        let slice = ActivityView::of(&view_with(&store, &[], 900, &team, now, &disk));
        assert_eq!(slice.agents.len(), v.agents.len());
        let after: std::collections::BTreeSet<PathBuf> = rec.touched().into_iter().collect();
        assert_eq!(before, after, "the activity slice read new paths");

        // Attachments: a message naming a screenshot in Documents and one
        // `harbard attach` copied. The read looks only in the attachments
        // folder; the Documents path is not an attachment and is not read.
        std::fs::write(tmp.path().join("Documents/shot.png"), b"png").unwrap();
        let src = TempHome::new();
        std::fs::write(src.path().join("before.png"), b"png").unwrap();
        let copied =
            crate::attach::attach(&store, &[src.path().join("before.png")], Some("before"))
                .unwrap();
        let copied = copied[0].path.display().to_string();
        team.leads[0].agents[0].messages[0].paths = vec![
            h("Documents/shot.png"),
            "~/Documents/shot.png".into(),
            copied.clone(),
        ];
        let v = view_with(&store, &[], 900, &team, now, &disk);
        let atts: Vec<&str> = v.messages[0]
            .attachments
            .iter()
            .map(|a| a.path.as_str())
            .collect();
        assert_eq!(atts, vec![copied.as_str()]);
        assert_eq!(
            v.messages[0].attachments[0].label.as_deref(),
            Some("before")
        );
        // The route serves from the attachments folder only, through the
        // same filesystem, so the recorder sees every path it touches.
        let root = crate::attach::root(&store);
        let id = &v.messages[0].attachments[0].id;
        assert_eq!(
            attachment_file(&rec, &root, id).unwrap(),
            (b"png".to_vec(), "image/png")
        );
        for rest in [
            "../Documents/shot.png",
            &format!("{}/../../Documents/shot.png", id.split('/').next().unwrap()),
            &h("Documents/shot.png"),
            "Documents/shot.png",
        ] {
            assert!(attachment_file(&rec, &root, rest).is_err(), "{rest}");
        }

        let protected: Vec<PathBuf> = rec
            .touched()
            .into_iter()
            .filter(|p| workspace::protected(&p.display().to_string(), &home))
            .collect();
        assert!(
            protected.is_empty(),
            "touched protected paths: {protected:?}"
        );
    }

    #[test]
    fn reduce_motion_is_stored_in_the_desk_and_shown_in_both_reads() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        let team = || synthetic_team(now);
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        assert_eq!(view(&store, &[], 900, &team(), now).motion, None);
        let mut ctx = WriteCtx {
            gate: &harbard_core::workspaces::Gate::OPEN,
            store: &store,
            events: &[],
            stale_secs: 900,
            now,
            team: &team,
            open: &mut never,
        };
        write(&mut ctx, "motion", &json!({"reduce": true})).unwrap();
        let reopened = Store::at(home.path());
        let v = view(&reopened, &[], 900, &team(), now);
        assert_eq!(v.motion.as_deref(), Some("reduce"));
        assert_eq!(ActivityView::of(&v).motion.as_deref(), Some("reduce"));
        assert!(all_events(&reopened).is_empty(), "nothing went to the bus");
        assert_eq!(
            write(&mut ctx, "motion", &json!({"reduce": "yes"}))
                .unwrap_err()
                .status,
            400
        );
        write(&mut ctx, "motion", &json!({"reduce": false})).unwrap();
        assert_eq!(view(&store, &[], 900, &team(), now).motion, None);
        // A hand-typed value that is not `reduce` follows the OS.
        DeskState::update(&DeskState::path(store.root()), |d| {
            d.motion = Some("mucho".into())
        })
        .unwrap();
        assert_eq!(view(&store, &[], 900, &team(), now).motion, None);
    }

    /// The review's five quoting cases (H1), a commit that mentions tests
    /// (M1) and a BLOCKED that quotes a credential (M3), from transcripts on
    /// disk through the team projector to both API bodies: none of the
    /// secret words may reach the JSON.
    #[test]
    fn review_cases_never_reach_the_workspace_or_the_activity_slice() {
        use harbard_core::team::{TeamProjector, TeamRoots, WINDOW_DEFAULT_MS};
        let home = TempHome::new();
        let now = clock::now_ms();
        let sid = "33333333-4444-5555-6666-777777777777";
        let session = home.path().join(".claude/projects/-cases").join(sid);
        std::fs::create_dir_all(session.join("subagents")).unwrap();
        let rec = |typ: &str, ms: u64, content: Value| {
            let mut v = json!({"type":typ,"timestamp":clock::rfc3339_utc(ms),"uuid":"u","sessionId":sid,"cwd":"/tmp/cases","isSidechain":true,"message":{"role":typ,"content":content}});
            if typ == "assistant" {
                v["message"]["stop_reason"] = json!("tool_use");
            }
            v.to_string()
        };
        std::fs::write(
            home.path()
                .join(".claude/projects/-cases")
                .join(format!("{sid}.jsonl")),
            format!("{}\n", rec("user", now - 60_000, json!("Lead task."))),
        )
        .unwrap();
        let commands = [
            r#"DB_PASSWORD="correct horse battery staple" psql -h db.internal"#,
            r#"env PGPASSWORD='open sesame now' pg_dump prod"#,
            r#"bash -c "API_KEY='abc hunter two' ./deploy.sh""#,
            r#"echo "note; hunter2 is the pin""#,
            "printf '%s\\n' \"header\nopensesame pin\" > /tmp/x",
            r#"git commit -m "fix flaky case; cargo test passes now""#,
        ];
        for (i, cmd) in commands.iter().enumerate() {
            let id = format!("c{i}");
            std::fs::write(
                session.join(format!("subagents/agent-{id}.meta.json")),
                json!({"name": format!("case-{i}"), "taskKind": "in_process_teammate"}).to_string(),
            )
            .unwrap();
            let lines = [
                rec(
                    "user",
                    now - 50_000,
                    json!("<teammate-message teammate_id=\"team-lead\">Task.</teammate-message>"),
                ),
                rec(
                    "assistant",
                    now - 20_000,
                    json!([{"type":"tool_use","id":"b","name":"Bash","input":{"command":cmd}}]),
                ),
            ];
            std::fs::write(
                session.join(format!("subagents/agent-{id}.jsonl")),
                lines.join("\n") + "\n",
            )
            .unwrap();
        }
        std::fs::write(
            session.join("subagents/agent-c9.meta.json"),
            json!({"name":"blocked-text","taskKind":"in_process_teammate"}).to_string(),
        )
        .unwrap();
        let blocked = [
            rec(
                "user",
                now - 50_000,
                json!("<teammate-message teammate_id=\"team-lead\">Task.</teammate-message>"),
            ),
            rec(
                "assistant",
                now - 40_000,
                json!([{"type":"tool_use","id":"s","name":"SendMessage","input":{"to":"team-lead","message":"BLOCKED · blocked-text · the API rejects GH_TOKEN=ghp_FAKEFAKEFAKE0123456789 with 401"}}]),
            ),
        ];
        std::fs::write(
            session.join("subagents/agent-c9.jsonl"),
            blocked.join("\n") + "\n",
        )
        .unwrap();

        let team = TeamProjector::new(TeamRoots {
            home: home.path().to_path_buf(),
            claude_projects: home.path().join(".claude/projects"),
        })
        .project(now, WINDOW_DEFAULT_MS);
        let store_home = TempHome::new();
        let store = Store::at(store_home.path());
        let disk = Disk {
            fs: &RealFs,
            cached: false,
            home: &home.path().display().to_string(),
            temp_roots: &[],
        };
        let v = view_with(&store, &[], 900, &team, now, &disk);
        let face = |name: &str| {
            let a = v
                .agents
                .iter()
                .find(|a| a.own_name.as_deref() == Some(name))
                .unwrap();
            (a.activity.state, a.activity.detail.clone())
        };
        assert_eq!(face("case-0"), ("running", Some("psql".into())));
        assert_eq!(face("case-1"), ("running", Some("pg_dump".into())));
        assert_eq!(face("case-3"), ("running", Some("echo".into())));
        assert_eq!(face("case-5"), ("running", Some("git commit".into())));
        assert_eq!(face("blocked-text").1, None);
        let full = serde_json::to_string(&v).unwrap();
        let slice = serde_json::to_string(&ActivityView::of(&v)).unwrap();
        for word in [
            "horse",
            "battery",
            "sesame",
            "hunter",
            "opensesame",
            "\"cargo test\"",
            "testing",
        ] {
            assert!(!slice.contains(word), "{word} reached the activity slice");
        }
        for word in ["horse", "battery", "sesame", "hunter", "opensesame"] {
            assert!(!full.contains(word), "{word} reached the workspace");
        }
        assert!(
            !slice.contains("ghp_FAKE"),
            "the BLOCKED text reached the slice"
        );
    }

    #[test]
    fn the_strip_night_dim_is_on_by_default_and_stored_when_turned_off() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        let team = || synthetic_team(now);
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        assert!(view(&store, &[], 900, &team(), now).night_dim);
        let mut ctx = WriteCtx {
            gate: &harbard_core::workspaces::Gate::OPEN,
            store: &store,
            events: &[],
            stale_secs: 900,
            now,
            team: &team,
            open: &mut never,
        };
        write(&mut ctx, "night-dim", &json!({"on": false})).unwrap();
        let reopened = Store::at(home.path());
        assert!(!view(&reopened, &[], 900, &team(), now).night_dim);
        assert_eq!(DeskState::load(&reopened).night_dim, Some(false));
        assert!(all_events(&reopened).is_empty(), "nothing went to the bus");
        assert_eq!(
            write(&mut ctx, "night-dim", &json!({"on": "no"}))
                .unwrap_err()
                .status,
            400
        );
        write(&mut ctx, "night-dim", &json!({"on": true})).unwrap();
        assert!(view(&store, &[], 900, &team(), now).night_dim);
        assert_eq!(DeskState::load(&store).night_dim, None);
    }

    /// Round 3: the PATH scan behind the faces' program names lists only
    /// absolute directories outside the TCC-protected folders, and never
    /// opens what it finds there.
    #[test]
    fn the_path_scan_never_touches_tcc_protected_folders() {
        // A real home with no symlink on its way (the temp dir's own
        // /var -> /private/var link resolved away).
        let tmp = TempHome::new();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let home = root.display().to_string();
        let home = home.as_str();
        for d in [
            "opt/homebrew/bin",
            ".cargo/bin",
            "Documents/bin",
            "Library/Mobile Documents/x",
            "Desktop",
        ] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        let path = format!(
            "{home}/opt/homebrew/bin:{home}/Documents/bin:relative/bin:{home}/Library/Mobile Documents/x:/Volumes/ext/bin:{home}/.cargo/bin:{home}/Desktop:/usr/bin/../etc"
        );
        let seen = std::sync::Mutex::new(Vec::<String>::new());
        let list = |dir: &Path| {
            seen.lock().unwrap().push(dir.display().to_string());
            vec![
                "rg".to_string(),
                ".hidden".to_string(),
                "bad name".to_string(),
                "zz-tool".to_string(),
            ]
        };
        let names = path_programs(&path, home, &list);
        let seen = seen.into_inner().unwrap();
        assert_eq!(
            seen,
            vec![
                format!("{home}/opt/homebrew/bin"),
                format!("{home}/.cargo/bin")
            ]
        );
        assert!(seen.iter().all(|d| !workspace::protected(d, home)));
        assert_eq!(names, vec!["rg".to_string(), "zz-tool".to_string()]);
    }

    /// Round 4, R4-N1: a PATH entry that is a symlink into Documents, and a
    /// File Provider folder (Library/CloudStorage), are never listed. Real
    /// files in a temp home stand for the protected folders.
    #[test]
    fn the_path_scan_follows_no_symlink_and_skips_cloud_storage() {
        let tmp = TempHome::new();
        let root = std::fs::canonicalize(tmp.path()).unwrap();
        let home = root.display().to_string();
        let docs = root.join("Documents/bin");
        let cloud = root.join("Library/CloudStorage/GoogleDrive-x/bin");
        let plain = root.join("tools/bin");
        for (dir, name) in [
            (&docs, "zz-docs-tool"),
            (&cloud, "zz-cloud-tool"),
            (&plain, "zz-plain-tool"),
        ] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(dir.join(name), "#!/bin/sh\n").unwrap();
        }
        std::os::unix::fs::symlink(&docs, root.join("bin")).unwrap();
        // A symlink to a harmless folder is skipped too: no read_dir through one.
        std::os::unix::fs::symlink(&plain, root.join("alias-bin")).unwrap();
        let path = format!(
            "{home}/bin:{}:{home}/alias-bin:{}",
            cloud.display(),
            plain.display()
        );
        let listed = std::sync::Mutex::new(Vec::<String>::new());
        let list = |dir: &Path| {
            listed.lock().unwrap().push(dir.display().to_string());
            list_dir(dir)
        };
        let names = path_programs(&path, &home, &list);
        assert_eq!(
            names,
            vec!["zz-plain-tool".to_string()],
            "listed {:?}",
            listed.lock().unwrap()
        );
        assert_eq!(*listed.lock().unwrap(), vec![plain.display().to_string()]);
    }

    fn bus(
        store: &Store,
        session: &str,
        kind: EventKind,
        text: &str,
        note_ref: Option<&str>,
    ) -> String {
        use harbard_core::SCHEMA_VERSION;
        let id = ulid::Ulid::generate().to_string();
        store
            .append(&Event {
                v: SCHEMA_VERSION,
                id: id.clone(),
                ts: clock::rfc3339_utc(clock::now_ms()),
                event: kind,
                session: session.into(),
                harness: "claude-code".into(),
                host: "h".into(),
                cwd: "/nowhere/orchard".into(),
                repo: None,
                repo_key: None,
                branch: None,
                role: Role::Main,
                parent: None,
                note_ref: note_ref.map(str::to_string),
                text: text.into(),
                to: None,
                kind: NoteKind::Fyi,
            })
            .unwrap();
        id
    }

    fn channel_note_state(
        store: &Store,
        now: u64,
    ) -> (String, Option<workspace::NoteDelivery>, Vec<String>) {
        let v = view(store, &all_events(store), 3_600, &synthetic_team(now), now);
        let m = v
            .messages
            .iter()
            .find(|m| m.from == "operator")
            .expect("the note is shown");
        (m.id.clone(), m.delivery.clone(), m.replies.clone())
    }

    #[test]
    fn a_note_reads_sent_delivered_read_and_answered_from_the_bus() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        hello(&store, LEAD, now);
        let team = || synthetic_team(now);
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        let events = all_events(&store);
        let mut ctx = WriteCtx {
            gate: &harbard_core::workspaces::Gate::OPEN,
            store: &store,
            events: &events,
            stale_secs: 3_600,
            now,
            team: &team,
            open: &mut never,
        };
        let out = write(
            &mut ctx,
            "reply",
            &json!({"channel":"orchard","text":"Probando, quiero saber si esto realmente funciona ya"}),
        )
        .unwrap();
        let note = out["stored"][0]["note"].as_str().unwrap().to_string();
        let state = |store: &Store| channel_note_state(store, now).1.unwrap().state;
        assert_eq!(state(&store), "sent");

        // The lead's brief surfaced it: delivered.
        harbard_core::cursor::write_brief_state(
            &store,
            LEAD,
            &harbard_core::cursor::BriefState {
                seen: Some(note.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(state(&store), "delivered");
        let d = channel_note_state(&store, now).1.unwrap();
        assert_eq!((d.delivered, d.to_lead), (1, true));

        // It acked it: read.
        bus(&store, LEAD, EventKind::Ack, &note, Some(&note));
        assert_eq!(state(&store), "read");

        // It answered with --ref: answered, and the answer is in the thread.
        let reply = bus(
            &store,
            LEAD,
            EventKind::Note,
            "Yes, it works: it reached me.",
            Some(&note),
        );
        let (id, d, replies) = channel_note_state(&store, now);
        assert_eq!(d.unwrap().state, "answered");
        assert_eq!(replies, vec![format!("reply:{reply}")]);
        let v = view(&store, &all_events(&store), 3_600, &team(), now);
        let r = v
            .messages
            .iter()
            .find(|m| m.id == format!("reply:{reply}"))
            .unwrap();
        assert_eq!(r.reply_to.as_deref(), Some(id.as_str()));
        assert_eq!(r.author, lead_callsign());
    }

    #[test]
    fn a_note_stored_before_the_session_joined_is_never_delivered() {
        // Today, 17:26: the note reached the lead's id before its hooks
        // were wired. A mark past it later is not a delivery.
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        let team = || synthetic_team(now);
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        let events = all_events(&store);
        let mut ctx = WriteCtx {
            gate: &harbard_core::workspaces::Gate::OPEN,
            store: &store,
            events: &events,
            stale_secs: 3_600,
            now,
            team: &team,
            open: &mut never,
        };
        // Nobody live: refused, unless the operator confirms.
        let err = write(
            &mut ctx,
            "reply",
            &json!({"channel":"orchard","text":"hello"}),
        )
        .unwrap_err();
        assert_eq!(err.status, 409);
        let out = write(
            &mut ctx,
            "reply",
            &json!({"channel":"orchard","text":"hello","anyway":true}),
        )
        .unwrap();
        assert_eq!(out["stored"][0]["session"], LEAD);
        std::thread::sleep(std::time::Duration::from_millis(3));
        let joined = bus(&store, LEAD, EventKind::Hello, "", None);
        harbard_core::cursor::write_brief_state(
            &store,
            LEAD,
            &harbard_core::cursor::BriefState {
                seen: Some(joined),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(channel_note_state(&store, now).1.unwrap().state, "sent");
    }

    #[test]
    fn resend_sends_an_ask_that_refs_the_note_and_counts_on_it() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        hello(&store, LEAD, now);
        let later = now + 12 * 60_000;
        let team = || synthetic_team(later);
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        let events = all_events(&store);
        let mut ctx = WriteCtx {
            gate: &harbard_core::workspaces::Gate::OPEN,
            store: &store,
            events: &events,
            stale_secs: 3_600,
            now,
            team: &team,
            open: &mut never,
        };
        let out = write(
            &mut ctx,
            "reply",
            &json!({"channel":"orchard","text":"Probando, quiero saber si esto realmente funciona ya"}),
        )
        .unwrap();
        let note = out["stored"][0]["note"].as_str().unwrap().to_string();

        // Twelve minutes later nobody has answered: a card, and a re-send.
        let events = all_events(&store);
        let v = view(&store, &events, 3_600, &team(), later);
        let card = v
            .cards
            .iter()
            .find(|c| c.kind == "unanswered" && c.needs && c.resolved_ms.is_none())
            .expect("no answer")
            .clone();
        assert_eq!(card.channel.as_deref(), Some("orchard"));
        let mut ctx = WriteCtx {
            gate: &harbard_core::workspaces::Gate::OPEN,
            store: &store,
            events: &events,
            stale_secs: 3_600,
            now: later,
            team: &team,
            open: &mut never,
        };
        assert_eq!(
            write(&mut ctx, "nudge", &json!({"card":"unanswered:note:nope"}))
                .unwrap_err()
                .status,
            404
        );
        let out = write(&mut ctx, "nudge", &json!({"card": card.id})).unwrap();
        assert_eq!(out["stored"].as_array().unwrap().len(), 1);
        let after = all_events(&store);
        assert_eq!(after.len(), events.len() + 1);
        let ask = after.last().unwrap();
        assert_eq!(ask.event, EventKind::Note);
        assert_eq!(ask.kind, NoteKind::Ask, "an ask wakes the lead");
        assert_eq!(ask.role, Role::Operator);
        assert_eq!(ask.note_ref.as_deref(), Some(note.as_str()));
        assert_eq!(ask.to.as_ref().unwrap().value, LEAD);
        assert!(ask.text.contains("in #orchard 12 min ago"), "{}", ask.text);
        assert!(
            ask.text
                .contains("\"Probando, quiero saber si esto realmente funciona ya\"")
        );

        // The re-send is not a second message; it marks the first.
        let v = view(&store, &after, 3_600, &team(), later);
        let ops: Vec<_> = v.messages.iter().filter(|m| m.from == "operator").collect();
        assert_eq!(ops.len(), 1);
        assert!(ops[0].delivery.as_ref().unwrap().nudged_ms.is_some());
        assert!(v.cards.iter().any(|c| c.id == card.id), "still unanswered");

        // `harbard ack last --note` answers the ask: answered, card gone.
        bus(&store, LEAD, EventKind::Note, "Sorry, done.", Some(&ask.id));
        let v = view(&store, &all_events(&store), 3_600, &team(), later);
        let op = v.messages.iter().find(|m| m.from == "operator").unwrap();
        assert_eq!(op.delivery.as_ref().unwrap().state, "answered");
        assert!(v.cards.iter().all(|c| c.id != card.id));
    }

    #[cfg(unix)]
    #[test]
    fn attachments_are_served_from_their_folder_only() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let src = TempHome::new();
        for (name, body) in [
            ("a.png", "png"),
            ("r.md", "# <script>x</script>"),
            ("p.html", "<b>"),
        ] {
            std::fs::write(src.path().join(name), body).unwrap();
        }
        let out = crate::attach::attach(
            &store,
            &["a.png", "r.md", "p.html"].map(|n| src.path().join(n)),
            None,
        )
        .unwrap();
        let root = crate::attach::root(&store);
        let id = |i: usize| {
            let p = &out[i].path;
            format!(
                "{}/{}",
                p.parent().unwrap().file_name().unwrap().to_string_lossy(),
                p.file_name().unwrap().to_string_lossy()
            )
        };
        assert_eq!(
            attachment_file(&RealFs, &root, &id(0)).unwrap().1,
            "image/png"
        );
        // Markdown is plain text on the wire, never HTML.
        assert_eq!(
            attachment_file(&RealFs, &root, &id(1)).unwrap().1,
            "text/plain; charset=utf-8"
        );
        assert_eq!(
            attachment_file(&RealFs, &root, &id(2)).unwrap_err().status,
            403
        );
        // A symlink planted in the folder, or a folder that is a symlink.
        let dir = out[0].path.parent().unwrap();
        std::os::unix::fs::symlink(src.path().join("a.png"), dir.join("planted.png")).unwrap();
        let ulid = dir.file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(
            attachment_file(&RealFs, &root, &format!("{ulid}/planted.png"))
                .unwrap_err()
                .status,
            404
        );
        let fake = ulid::Ulid::generate().to_string();
        std::os::unix::fs::symlink(src.path(), root.join(&fake)).unwrap();
        assert!(attachment_file(&RealFs, &root, &format!("{fake}/a.png")).is_err());
        for bad in [
            "",
            "x",
            "../a.png",
            &format!("{ulid}/../{ulid}/a.png"),
            &format!("{ulid}/missing.png"),
        ] {
            assert!(attachment_file(&RealFs, &root, bad).is_err(), "{bad}");
        }
        // The view's lookup: only paths under the folder, and only real files.
        let home_s = home.path().display().to_string();
        assert!(
            attachment_lookup(&RealFs, &root, &home_s, &out[0].path.display().to_string())
                .is_some()
        );
        assert!(
            attachment_lookup(
                &RealFs,
                &root,
                &home_s,
                &src.path().join("a.png").display().to_string()
            )
            .is_none()
        );
        assert!(
            attachment_lookup(
                &RealFs,
                &root,
                &home_s,
                &dir.join("planted.png").display().to_string()
            )
            .is_none()
        );
    }

    fn ctx_for<'a>(
        store: &'a Store,
        events: &'a [Event],
        now: u64,
        team: &'a dyn Fn() -> TeamView,
        open: &'a mut dyn FnMut(&Path, &[&str]) -> std::io::Result<()>,
    ) -> WriteCtx<'a> {
        WriteCtx {
            gate: &harbard_core::workspaces::Gate::OPEN,
            store,
            events,
            stale_secs: 3_600,
            now,
            team,
            open,
        }
    }

    fn to_human(store: &Store, session: &str, text: &str, note_ref: Option<&str>) -> String {
        let id = bus(store, session, EventKind::Note, text, note_ref);
        // Rewrite the recipient: `--to harness:human`.
        let path = store.events_dir();
        for entry in std::fs::read_dir(&path).unwrap() {
            let p = entry.unwrap().path();
            let body = std::fs::read_to_string(&p).unwrap();
            let fixed = body.replace(
                &format!("\"id\":\"{id}\""),
                &format!("\"id\":\"{id}\",\"to\":{{\"kind\":\"harness\",\"value\":\"human\"}}"),
            );
            std::fs::write(&p, fixed).unwrap();
        }
        id
    }

    #[test]
    fn bus_notes_for_the_human_reach_the_console_and_answers_to_his_notes_ride_the_trail() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        hello(&store, LEAD, now);
        // A note for the human, a reply to it, and one for another session.
        let first = to_human(&store, LEAD, "Dana, the channel answers now.", None);
        let answer = bus(
            &store,
            "peer-session",
            EventKind::Note,
            "Confirmado.",
            Some(&first),
        );
        let elsewhere = bus(
            &store,
            "peer-session",
            EventKind::Note,
            "only for another",
            None,
        );
        std::fs::write(store.events_dir().join("x.tmp"), "").unwrap();
        let events = all_events(&store);
        let mut notes = bus_notes_within(&events, now, BUS_NOTE_WINDOW_MS);
        // `bus` writes broadcasts; this one is for another session.
        notes.retain(|n| n.id != elsewhere);
        let ids: Vec<&str> = notes.iter().map(|n| n.id.as_str()).collect();
        assert!(ids.contains(&first.as_str()) && ids.contains(&answer.as_str()));
        // A note addressed to one other session is not the operator's.
        let mut addressed = events.clone();
        if let Some(e) = addressed.iter_mut().find(|e| e.id == elsewhere) {
            e.to = Some(Recipient {
                kind: RecipientKind::Session,
                value: "someone".into(),
            });
        }
        assert!(
            bus_notes_within(&addressed, now, BUS_NOTE_WINDOW_MS)
                .iter()
                .all(|n| n.id != elsewhere)
        );
        // An answer to one of their own notes is never listed twice.
        let team = || synthetic_team(now);
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        let mut ctx = ctx_for(&store, &events, now, &team, &mut never);
        let out = write(
            &mut ctx,
            "reply",
            &json!({"channel":"orchard","text":"Can you hear me?"}),
        )
        .unwrap();
        let mine = out["stored"][0]["note"].as_str().unwrap().to_string();
        let reply = to_human(&store, LEAD, "Te oigo.", Some(&mine));
        let events = all_events(&store);
        assert!(
            bus_notes_within(&events, now, BUS_NOTE_WINDOW_MS)
                .iter()
                .all(|n| n.id != reply)
        );
        let v = view(&store, &events, 3_600, &team(), now);
        let r = v
            .messages
            .iter()
            .find(|m| m.id == format!("reply:{reply}"))
            .unwrap();
        assert_eq!(r.reply_to.as_deref(), Some(format!("note:{mine}").as_str()));
        assert_eq!(r.author, lead_callsign());
    }

    #[test]
    fn a_mention_sends_an_ask_and_a_dm_picks_relay_or_direct() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        hello(&store, LEAD, now);
        let team = || synthetic_team(now);
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        let events = all_events(&store);
        let mut ctx = ctx_for(&store, &events, now, &team, &mut never);
        // In the channel: the lead reads it; an @ to the agent's workshop
        // name makes it an ask with a to_agent line the lead relays.
        let handle = view(&store, &events, 3_600, &team(), now)
            .agents
            .iter()
            .find(|a| a.key == "a1")
            .unwrap()
            .handle
            .clone();
        let out = write(
            &mut ctx,
            "reply",
            &json!({"channel":"orchard","text": format!("@{handle} can you look at it?")}),
        )
        .unwrap();
        assert_eq!(out["stored"].as_array().unwrap().len(), 1);
        assert_eq!(out["stored"][0]["kind"], "ask");
        let e = all_events(&store).pop().unwrap();
        assert_eq!(e.kind, NoteKind::Ask);
        assert_eq!(
            e.text,
            format!("In #orchard: @{handle} can you look at it?\nto_agent: a1")
        );
        // Without a mention, the channel note stays quiet.
        let out = write(
            &mut ctx,
            "reply",
            &json!({"channel":"orchard","text":"Para que conste."}),
        )
        .unwrap();
        assert_eq!(out["stored"][0]["kind"], "fyi");
        let v = view(&store, &all_events(&store), 3_600, &team(), now);
        let note = v
            .messages
            .iter()
            .find(|m| m.text.starts_with(&format!("@{handle}")))
            .unwrap();
        assert_eq!(note.mentions, vec![handle.clone()]);
        assert!(!note.text.contains("to_agent"));

        // A DM to an in-process agent: relayed by its lead.
        let out = write(&mut ctx, "reply", &json!({"agent":"a1","text":"hello"})).unwrap();
        assert_eq!(out["path"], "relay");
        let e = all_events(&store).pop().unwrap();
        assert_eq!(
            (e.kind, e.to.unwrap().value.as_str()),
            (NoteKind::Ask, LEAD)
        );
        assert!(e.text.ends_with("\nto_agent: a1"));
        // A DM to the lead session itself, a separately launched session:
        // direct, no relay line.
        let lead = format!("session:{LEAD}");
        let v = view(&store, &all_events(&store), 3_600, &team(), now);
        assert!(
            v.agents.iter().any(|a| a.key == lead),
            "the lead is a conversation"
        );
        let out = write(
            &mut ctx,
            "reply",
            &json!({"agent": lead, "text":"how is it going?"}),
        )
        .unwrap();
        assert_eq!(out["path"], "direct");
        let e = all_events(&store).pop().unwrap();
        assert_eq!(e.kind, NoteKind::Ask);
        assert_eq!(e.text, format!("To {}: how is it going?", lead_callsign()));
        let v = view(&store, &all_events(&store), 3_600, &team(), now);
        let mine: Vec<&str> = v
            .messages
            .iter()
            .filter(|m| m.agent == lead && m.from == "operator")
            .map(|m| m.text.as_str())
            .collect();
        assert_eq!(mine, vec!["how is it going?"]);
        let to_agent: Vec<&str> = v
            .messages
            .iter()
            .filter(|m| m.agent == "a1" && m.from == "operator")
            .map(|m| m.text.as_str())
            .collect();
        assert_eq!(
            to_agent,
            vec!["hello"],
            "the agent's own DM, not the lead's"
        );
    }

    #[test]
    fn read_marks_and_resolve_all_are_stored_and_undone_through_the_desk() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        hello(&store, LEAD, now);
        let team = || synthetic_team(now);
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        let events = all_events(&store);
        let before = view(&store, &events, 3_600, &team(), now);
        assert!(before.unread.directs > 0);
        let mut ctx = ctx_for(&store, &events, now, &team, &mut never);
        write(&mut ctx, "read-channel", &json!({"channel":"orchard"})).unwrap();
        assert_eq!(
            write(&mut ctx, "read-channel", &json!({"channel":"nope"}))
                .unwrap_err()
                .status,
            404
        );
        write(&mut ctx, "read-all", &json!({})).unwrap();
        // A fresh read (a reload, another device) sees it from the desk file.
        let after = view(&store, &events, 3_600, &team(), now + 1);
        assert_eq!(after.unread.directs, 0);
        assert_eq!(ActivityView::of(&after).directs, 0);
        assert!(DeskState::load(&store).read_all_ms >= now);
        // Needs you is not read marks: the BLOCKED card is still open.
        let open: Vec<String> = after
            .cards
            .iter()
            .filter(|c| c.needs && c.resolved_ms.is_none())
            .map(|c| c.id.clone())
            .collect();
        assert!(!open.is_empty());
        let out = write(&mut ctx, "resolve-all", &json!({"cards": open})).unwrap();
        assert_eq!(out["cards"].as_array().unwrap().len(), open.len());
        let v = view(&store, &events, 3_600, &team(), now);
        assert_eq!(v.unread.activity, 0);
        // "Undo": one step back.
        write(&mut ctx, "reopen-all", &json!({"cards": open})).unwrap();
        let v = view(&store, &events, 3_600, &team(), now);
        assert_eq!(v.unread.activity, open.len());
    }

    /// A scratch repository: commits a and b on main, c on a branch, and
    /// d on a branch whose change reached main rebased as d2.
    struct Scratch {
        repo: PathBuf,
        a: String,
        b: String,
        c: String,
        d: String,
        d2: String,
    }

    fn scratch_repo(root: &Path) -> Scratch {
        let repo = root.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str], date: &str| -> String {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(&repo)
                .args([
                    "-c",
                    "user.name=Synthetic",
                    "-c",
                    "user.email=synthetic@example.test",
                ])
                .args([
                    "-c",
                    "commit.gpgsign=false",
                    "-c",
                    "init.defaultBranch=main",
                ])
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .env("GIT_AUTHOR_DATE", date)
                .env("GIT_COMMITTER_DATE", date)
                .output()
                .expect("git");
            assert!(
                out.status.success(),
                "git {args:?}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).trim().to_string()
        };
        let head = |date: &str| git(&["rev-parse", "HEAD"], date);
        git(&["init", "-q"], "1790000000 +0000");
        git(
            &["commit", "-q", "--allow-empty", "-m", "a"],
            "1790000001 +0000",
        );
        let a = head("1790000001 +0000");
        git(&["switch", "-q", "-c", "feature"], "1790000002 +0000");
        git(
            &["commit", "-q", "--allow-empty", "-m", "c"],
            "1790000003 +0000",
        );
        let c = head("1790000003 +0000");
        git(
            &["switch", "-q", "-c", "rebased", "main"],
            "1790000004 +0000",
        );
        git(
            &["commit", "-q", "--allow-empty", "-m", "d"],
            "1790000005 +0000",
        );
        let d = head("1790000005 +0000");
        git(&["switch", "-q", "main"], "1790000006 +0000");
        git(
            &["commit", "-q", "--allow-empty", "-m", "b"],
            "1790000007 +0000",
        );
        let b = head("1790000007 +0000");
        // d onto main: same author time and subject, a new id.
        git(&["cherry-pick", "--allow-empty", &d], "1790000008 +0000");
        let d2 = head("1790000008 +0000");
        Scratch {
            repo,
            a,
            b,
            c,
            d,
            d2,
        }
    }

    #[test]
    fn installed_means_the_commit_is_in_the_build_serve_and_the_console_run() {
        let home = TempHome::new();
        let r = scratch_repo(home.path());
        let short = |s: &str| s[..7].to_string();
        let shas: Vec<String> = [&r.a, &r.b, &r.c, &r.d]
            .iter()
            .map(|s| short(s))
            .chain(["deadbee".to_string()])
            .collect();
        // serve runs a: a is installed, b is on main but not installed yet.
        let g = RepoGit::load(&r.repo, &shas, Some(&r.a), None);
        let fa = g.facts(&short(&r.a)).unwrap();
        assert_eq!((fa.on_main, fa.installed), (true, Some(true)));
        assert_eq!(fa.commit, r.a, "the full id");
        let fb = g.facts(&short(&r.b)).unwrap();
        assert_eq!((fb.on_main, fb.installed), (true, Some(false)));
        // On a branch only: not on main, so not installed.
        let fc = g.facts(&short(&r.c)).unwrap();
        assert_eq!((fc.on_main, fc.installed), (false, Some(false)));
        // A SHA git does not know.
        assert!(g.facts("deadbee").is_none());
        // serve runs main's tip (d2): b and the rebased d are installed.
        let g = RepoGit::load(&r.repo, &shas, Some(&r.d2), None);
        assert_eq!(g.facts(&short(&r.b)).unwrap().installed, Some(true));
        let fd = g.facts(&short(&r.d)).unwrap();
        assert_eq!(
            (fd.on_main, fd.installed),
            (true, Some(true)),
            "rebased onto main as d2"
        );
        assert_eq!(fd.landed_as.as_deref(), Some(r.d2.as_str()));
        // serve runs d2 but the console still runs a: b is not installed.
        let g = RepoGit::load(&r.repo, &shas, Some(&r.d2), Some(&short(&r.a)));
        assert_eq!(g.facts(&short(&r.b)).unwrap().installed, Some(false));
        assert_eq!(g.facts(&short(&r.a)).unwrap().installed, Some(true));
        // serve's commit unknown in this repository: "Installed" does not apply.
        let g = RepoGit::load(
            &r.repo,
            &shas,
            Some("0123456789abcdef0123456789abcdef01234567"),
            None,
        );
        assert_eq!(g.facts(&short(&r.a)).unwrap().installed, None);
        assert_eq!(
            RepoGit::load(&r.repo, &shas, None, None)
                .facts(&short(&r.a))
                .unwrap()
                .installed,
            None
        );
        // Nothing that is not a commit id is ever handed to git.
        let g = RepoGit::load(
            &r.repo,
            &["--all".into(), "main".into(), "HEAD".into()],
            Some(&r.a),
            None,
        );
        assert!(
            g.facts("--all").is_none() && g.facts("main").is_none() && g.facts("HEAD").is_none()
        );
    }

    #[test]
    fn the_workspace_carries_shipments_and_the_running_commit() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        let team = synthetic_team(now);
        let mut v = view(&store, &[], 3_600, &team, now);
        attach_shipments(
            &mut v,
            Some("0123456789abcdef0123456789abcdef01234567"),
            None,
            home.path().to_str().unwrap(),
        );
        assert_eq!(
            v.running_commit.as_deref(),
            Some("0123456789abcdef0123456789abcdef01234567")
        );
        let raw = serde_json::to_value(&v).unwrap();
        assert!(raw["shipments"].is_array());
        assert!(raw["running_commit"].is_string());
    }

    #[test]
    fn hiding_an_agent_is_stored_on_the_desk_and_keeps_its_blocked_card() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        hello(&store, LEAD, now);
        let team = || synthetic_team(now);
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        let events = all_events(&store);
        let mut ctx = ctx_for(&store, &events, now, &team, &mut never);
        assert_eq!(
            write(&mut ctx, "hide-agent", &json!({"agent":"nope"}))
                .unwrap_err()
                .status,
            404,
            "only an agent the view holds"
        );
        write(&mut ctx, "hide-agent", &json!({"agent":"a1"})).unwrap();
        // A fresh read (a reload, another device) sees it from the desk file.
        let reopened = Store::at(home.path());
        let v = view(&reopened, &events, 3_600, &team(), now + 1);
        let a1 = v.agents.iter().find(|a| a.key == "a1").unwrap();
        assert!(a1.hidden);
        assert!(DeskState::load(&reopened).hidden_agents.contains_key("a1"));
        assert!(
            v.cards.iter().any(|c| c.kind == "blocked"
                && c.agents == ["a1"]
                && c.needs
                && c.resolved_ms.is_none()),
            "hidden, its BLOCKED still needs the operator"
        );
        let raw = serde_json::to_value(&v).unwrap();
        let wire = raw["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["key"] == "a1")
            .unwrap();
        assert_eq!(wire["hidden"], json!(true));
        write(&mut ctx, "show-agent", &json!({"agent":"a1"})).unwrap();
        let shown = view(&store, &events, 3_600, &team(), now);
        assert!(!shown.agents.iter().find(|a| a.key == "a1").unwrap().hidden);
        write(&mut ctx, "hide-agent", &json!({"agent":"a1"})).unwrap();
        write(&mut ctx, "show-all-agents", &json!({})).unwrap();
        assert!(DeskState::load(&store).hidden_agents.is_empty());
        // The Agents section folds, on the desk too.
        assert!(!view(&store, &events, 3_600, &team(), now).agents_collapsed);
        write(&mut ctx, "agents-collapsed", &json!({"collapsed": true})).unwrap();
        assert!(view(&store, &events, 3_600, &team(), now).agents_collapsed);
        assert_eq!(
            write(&mut ctx, "agents-collapsed", &json!({"collapsed": "yes"}))
                .unwrap_err()
                .status,
            400
        );
        write(&mut ctx, "agents-collapsed", &json!({"collapsed": false})).unwrap();
        assert_eq!(DeskState::load(&store).agents_collapsed, None);
        assert!(
            all_events(&store).len() == events.len(),
            "nothing went to the bus"
        );
    }

    /// Folders are not channels: `# v7` and `# drafts` sit under Documents, and
    /// `# acme-config` is a real repository nobody wrote in.
    #[test]
    fn todays_orphan_channels_resolve_to_nothing_or_to_a_real_repository() {
        let tmp = TempHome::new();
        let home = tmp.path().display().to_string();
        let h = |rel: &str| format!("{home}/{rel}");
        git_repo(&tmp.path().join("Projects/open-source/acme-config"), "main");
        git_repo(&tmp.path().join("Projects/harbard"), "main");
        git_repo(
            &tmp.path().join("Documents/notes/2026-11-06/sketches/v7"),
            "main",
        );
        std::fs::create_dir_all(tmp.path().join("Documents/notes/2026-11-05/drafts")).unwrap();
        let rec = Recorder::default();
        let r = Resolver {
            home: home.clone(),
            temp_roots: &[],
            known: Vec::new(),
            fs: &rec,
        };
        let config_repo = r.resolve(&h("Projects/open-source/acme-config")).unwrap();
        assert!(config_repo.real);
        assert!(
            config_repo
                .repo
                .ends_with("/Projects/open-source/acme-config"),
            "{}",
            config_repo.repo
        );
        assert!(
            r.resolve(&h("Documents/notes/2026-11-06/sketches/v7"))
                .is_none()
        );
        assert!(r.resolve(&h("Documents/notes/2026-11-05/drafts")).is_none());
        // A removed worktree folds into its repository, real because the
        // primary checkout is a git repository.
        let wt = r.resolve(&h("Projects/harbard-wt-protocol/src")).unwrap();
        assert!(wt.repo.ends_with("/Projects/harbard"), "{}", wt.repo);
        assert!(wt.real);
        // A folder that is no repository is not a channel.
        std::fs::create_dir_all(tmp.path().join("Projects/scratch")).unwrap();
        assert!(r.resolve(&h("Projects/scratch")).is_none());
        // Outside home and with no known root, nothing is looked at.
        assert!(r.resolve("/opt/homebrew/Library").is_none());
        let protected: Vec<PathBuf> = rec
            .touched()
            .into_iter()
            .filter(|p| workspace::protected(&p.display().to_string(), &home))
            .collect();
        assert!(protected.is_empty(), "{protected:?}");
    }

    /// A symlink swapped
    /// over an attachment between the check and the read, and a hardlink to
    /// a file elsewhere. Neither may ever serve the other file's bytes.
    #[cfg(unix)]
    #[test]
    fn a_swapped_symlink_or_a_hardlink_never_serves_another_file() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let home = TempHome::new();
        let store = Store::at(home.path());
        let outside = TempHome::new();
        std::fs::write(outside.path().join("secret.png"), b"OUTSIDE-SECRET").unwrap();
        let src = TempHome::new();
        std::fs::write(src.path().join("a.png"), b"inside").unwrap();
        let out = crate::attach::attach(&store, &[src.path().join("a.png")], None).unwrap();
        let dir = out[0].path.parent().unwrap().to_path_buf();
        let ulid = dir.file_name().unwrap().to_string_lossy().to_string();
        let root = crate::attach::root(&store);
        // L3: a hardlink planted beside it.
        std::fs::hard_link(outside.path().join("secret.png"), dir.join("b.png")).unwrap();
        assert!(
            attachment_file(&RealFs, &root, &format!("{ulid}/b.png")).is_err(),
            "a hardlink was served"
        );
        assert!(
            attachment_lookup(&RealFs, &root, "", &dir.join("b.png").display().to_string())
                .is_none()
        );
        // H1: swap a symlink over a.png, back and forth, while serving it.
        let stop = Arc::new(AtomicBool::new(false));
        let swapper = {
            let (dir, secret, stop) =
                (dir.clone(), outside.path().join("secret.png"), stop.clone());
            std::thread::spawn(move || {
                let mut n = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    let l = dir.join(format!(".l{n}"));
                    let f = dir.join(format!(".f{n}"));
                    let _ = std::os::unix::fs::symlink(&secret, &l);
                    let _ = std::fs::rename(&l, dir.join("a.png"));
                    let _ = std::fs::write(&f, b"inside");
                    let _ = std::fs::rename(&f, dir.join("a.png"));
                    n += 1;
                }
            })
        };
        let mut leaks = 0;
        for _ in 0..3_000 {
            if let Ok((bytes, _)) = attachment_file(&RealFs, &root, &format!("{ulid}/a.png"))
                && bytes == b"OUTSIDE-SECRET"
            {
                leaks += 1;
            }
        }
        stop.store(true, Ordering::Relaxed);
        swapper.join().unwrap();
        assert_eq!(leaks, 0, "the swap served the outside file {leaks} times");
    }

    /// A second "Resend" inside the window is refused by serve. A channel note
    /// reaches every session working in the repository, lead first; the nudge
    /// still goes to the coordinator only, once per window.
    #[test]
    fn one_nudge_per_window_and_only_to_the_coordinator() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        hello(&store, LEAD, now);
        // A second, unrelated session working in the same repository.
        {
            use harbard_core::SCHEMA_VERSION;
            store
                .append(&Event {
                    v: SCHEMA_VERSION,
                    id: ulid::Ulid::generate().to_string(),
                    ts: clock::rfc3339_utc(now),
                    event: EventKind::Hello,
                    session: "other-session".into(),
                    harness: "codex".into(),
                    host: "h".into(),
                    cwd: test_repo(),
                    repo: Some(test_repo()),
                    repo_key: None,
                    branch: None,
                    role: Role::Main,
                    parent: None,
                    note_ref: None,
                    text: String::new(),
                    to: None,
                    kind: NoteKind::Fyi,
                })
                .unwrap();
        }
        let later = now + 12 * 60_000;
        let team = || synthetic_team(later);
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        let events = all_events(&store);
        let mut ctx = ctx_for(&store, &events, now, &team, &mut never);
        let out = write(
            &mut ctx,
            "reply",
            &json!({"channel":"orchard","text":"Anyone there?"}),
        )
        .unwrap();
        let sessions: Vec<&str> = out["stored"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["session"].as_str().unwrap())
            .collect();
        assert_eq!(
            sessions,
            vec![LEAD, "other-session"],
            "every session working in the repository reads it, the lead first"
        );
        // A note from twelve minutes ago that was stored for both sessions
        // (as before this fix): the re-send goes to the coordinator only.
        let old = now - 12 * 60_000;
        for session in [LEAD, "other-session"] {
            let mut e = super::super::operator_note_event(
                "In #orchard: Can anyone hear me?",
                Some(Recipient {
                    kind: RecipientKind::Session,
                    value: session.into(),
                }),
                NoteKind::Fyi,
                None,
                old,
            );
            e.id = ulid::Ulid::from_parts(old, u128::from(session.len() as u64)).to_string();
            store.append(&e).unwrap();
        }
        let team = || synthetic_team(clock::now_ms());
        let events = all_events(&store);
        let now = clock::now_ms();
        let v = view(&store, &events, 3_600, &team(), now);
        let card = v
            .cards
            .iter()
            .find(|c| c.kind == "unanswered" && c.needs)
            .expect("no answer")
            .id
            .clone();
        let mut ctx = ctx_for(&store, &events, now, &team, &mut never);
        let first = write(&mut ctx, "nudge", &json!({"card": card})).unwrap();
        let targets: Vec<&str> = first["stored"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["session"].as_str().unwrap())
            .collect();
        assert_eq!(targets, vec![LEAD]);
        // Clicked again a second later: refused by serve, nothing stored.
        let events = all_events(&store);
        let mut ctx = ctx_for(&store, &events, clock::now_ms() + 1_000, &team, &mut never);
        let again = write(&mut ctx, "nudge", &json!({"card": card})).unwrap_err();
        assert_eq!((again.status, again.code), (409, "already_nudged"));
        let asks = all_events(&store)
            .iter()
            .filter(|e| e.kind == NoteKind::Ask)
            .count();
        assert_eq!(asks, 1, "one ask, however many clicks");
    }

    /// AC4: the operator's names.json already repeats names. The read settles
    /// them by age in memory and the view shows each name once; the read
    /// writes nothing, and serve's names tick saves
    /// the same result, once.
    #[test]
    fn a_names_json_with_repeats_is_numbered_once_on_the_read() {
        use harbard_core::workshop::{Entry, Gender, Registry};
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        hello(&store, LEAD, now);
        let mut team = synthetic_team(now);
        let base = team.leads[0].agents[0].clone();
        team.leads[0].agents = (1..=3)
            .map(|i| {
                let mut row = base.clone();
                row.id = format!("a{i}");
                row.first_ms = Some(now - i * 60_000);
                row
            })
            .collect();
        let mut reg = Registry::default();
        for i in 1..=3 {
            reg.agents.insert(
                format!("a{i}"),
                Entry {
                    name: "Sarah".into(),
                    gender: Gender::F,
                    team: None,
                    role: None,
                    source: "auto".into(),
                    reason: None,
                    assigned_ms: 1,
                    team_explicit: false,
                },
            );
        }
        reg.save(home.path()).unwrap();
        let events = all_events(&store);
        let v = view(&store, &events, 3_600, &team, now);
        let name =
            |v: &WorkspaceView, k: &str| v.agents.iter().find(|a| a.key == k).unwrap().name.clone();
        assert_eq!(name(&v, "a3"), "Sarah", "the oldest keeps it");
        let (a2, a1) = (name(&v, "a2"), name(&v, "a1"));
        for n in [&a2, &a1] {
            assert!(
                harbard_core::workshop::listed(n).is_some(),
                "a free name of the list: {n}"
            );
            assert_ne!(n, "Sarah");
        }
        assert_ne!(a2, a1);
        let path = Registry::path(home.path());
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(
            !on_disk.contains(&format!("\"{a1}\"")),
            "the read wrote nothing"
        );
        persist_names(&store, &events, 3_600, &team, now);
        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains(&format!("\"{a1}\"")), "serve persisted");
        assert!(
            written.contains(&format!("\"{a2}\"")),
            "the same names the read showed"
        );
        let v = view(&store, &events, 3_600, &team, now);
        assert_eq!(name(&v, "a1"), a1);
        assert_eq!(name(&v, "a2"), a2);
        persist_names(&store, &events, 3_600, &team, now + 1);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), written, "once");
    }

    /// Workshop names: assigned on the read, unique among live agents,
    /// renamed by double click and voted, all persisted in names.json.
    #[test]
    fn workshop_names_are_assigned_renamed_and_voted_and_persist() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        hello(&store, LEAD, now);
        let team = || synthetic_team(now);
        let events = all_events(&store);
        let v = view(&store, &events, 3_600, &team(), now);
        let a = v.agents.iter().find(|a| a.key == "a1").unwrap();
        assert!(
            harbard_core::workshop::listed(&a.name).is_some(),
            "{}",
            a.name
        );
        assert_eq!(a.name_source.as_deref(), Some("auto"));
        let first = a.name.clone();
        // Stable: a second read, and a restart, keep it.
        assert_eq!(
            view(&store, &events, 3_600, &team(), now)
                .agents
                .iter()
                .find(|a| a.key == "a1")
                .unwrap()
                .name,
            first
        );
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        let mut ctx = ctx_for(&store, &events, now, &team, &mut never);
        let out = write(
            &mut ctx,
            "rename-agent",
            &json!({"agent":"a1","name":"clara"}),
        )
        .unwrap();
        assert_eq!(out["name"], "Clara");
        let reopened = Store::at(home.path());
        let v = view(&reopened, &events, 3_600, &team(), now);
        assert_eq!(
            v.agents.iter().find(|a| a.key == "a1").unwrap().name,
            "Clara"
        );
        // 👎: a new name now, and the old one never again.
        let out = write(&mut ctx, "name-vote", &json!({"agent":"a1","vote":"down"})).unwrap();
        assert_ne!(out["name"], "Clara");
        let reg = harbard_core::workshop::Registry::load(home.path());
        assert!(reg.rejected().contains("clara"));
        assert_eq!(
            write(&mut ctx, "name-vote", &json!({"agent":"a1","vote":"meh"}))
                .unwrap_err()
                .status,
            400
        );
        // The lead session has no workshop name to vote on.
        assert_eq!(
            write(
                &mut ctx,
                "name-vote",
                &json!({"agent": format!("session:{LEAD}"), "vote":"up"})
            )
            .unwrap_err()
            .status,
            404
        );
    }

    // A DM to lead A's agent that @mentions lead B's agent
    // writes ONE text with BOTH `to_agent:` lines, and stores it for both
    // leads. Each lead's `harbard relay` then forwards the note to an agent
    // that is not its own (by that agent's SendMessage name, in its own team
    // namespace), and both agents get it twice.
    // Each lead's copy names only that lead's agents.
    #[test]
    fn each_lead_copy_names_only_its_own_agents() {
        const LEAD_B: &str = "99999999-8888-7777-6666-555555555555";
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        hello(&store, LEAD, now);
        hello(&store, LEAD_B, now);
        let two = move || {
            let mut t = synthetic_team(now);
            let mut b = t.leads[0].clone();
            b.session = LEAD_B.into();
            b.agents[0].id = "b1".into();
            b.agents[0].name = Some("profile-review".into());
            t.leads.push(b);
            t
        };
        let events = all_events(&store);
        let v = view(&store, &events, 3_600, &two(), now);
        let b1 = v.agents.iter().find(|a| a.key == "b1").expect("b1 in view");
        let handle = b1.handle.clone();
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        let mut ctx = ctx_for(&store, &events, now, &two, &mut never);
        let out = write(
            &mut ctx,
            "reply",
            &json!({"agent": "a1", "text": format!("See what @{handle} says")}),
        )
        .unwrap();
        assert_eq!(out["path"], "relay", "{out}");
        let all = all_events(&store);
        let for_a = all
            .iter()
            .rev()
            .find(|e| e.to.as_ref().is_some_and(|t| t.value == LEAD))
            .unwrap();
        let for_b = all
            .iter()
            .rev()
            .find(|e| e.to.as_ref().is_some_and(|t| t.value == LEAD_B))
            .unwrap();
        assert!(
            !for_a.text.contains("to_agent: b1"),
            "lead A is asked to relay to lead B's agent:\n{}",
            for_a.text
        );
        assert!(
            !for_b.text.contains("to_agent: a1"),
            "lead B is asked to relay to lead A's agent:\n{}",
            for_b.text
        );
        assert!(for_a.text.contains("to_agent: a1"), "{}", for_a.text);
        assert!(for_b.text.contains("to_agent: b1"), "{}", for_b.text);
    }

    /// Clara's review of M5: the operator's reply carrying hidden code
    /// points is refused, named, and nothing is stored.
    #[test]
    fn a_reply_with_hidden_characters_is_refused_and_stores_nothing() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        hello(&store, LEAD, now);
        let events = all_events(&store);
        let before = events.len();
        let mut never = |_: &Path, _: &[&str]| -> std::io::Result<()> { panic!("opened") };
        let team = move || synthetic_team(now);
        let mut ctx = ctx_for(&store, &events, now, &team, &mut never);
        let err = write(
            &mut ctx,
            "reply",
            &json!({"agent": "a1", "text": "Vale\u{E0049}\u{FE00}"}),
        )
        .unwrap_err();
        assert_eq!((err.status, err.code), (422, "invisible_chars"));
        assert!(err.message.contains("U+E0049"), "{}", err.message);
        assert_eq!(all_events(&store).len(), before);
    }

    /// The view names new agents without writing
    /// names.json; serve persists the same names on its tick, so a later
    /// read shows them unchanged.
    #[test]
    fn the_view_names_new_agents_without_writing_and_serve_keeps_the_same_names() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let now = clock::now_ms();
        hello(&store, LEAD, now);
        let events = all_events(&store);
        let names_file = harbard_core::workshop::Registry::path(store.root());
        let shown = view(&store, &events, 3_600, &synthetic_team(now), now);
        assert!(!names_file.exists(), "a read wrote names.json");
        let a1 = shown
            .agents
            .iter()
            .find(|a| a.key == "a1")
            .unwrap()
            .name
            .clone();
        persist_names(&store, &events, 3_600, &synthetic_team(now), now);
        assert!(names_file.exists());
        let again = view(&store, &events, 3_600, &synthetic_team(now), now + 1);
        assert_eq!(
            again.agents.iter().find(|a| a.key == "a1").unwrap().name,
            a1,
            "the persisted name is the one the read showed"
        );
    }
}

#[cfg(test)]
mod scope_tests;
