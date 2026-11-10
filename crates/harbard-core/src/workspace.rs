//! The workspace: the team view, the bus and the sluice folded into channels,
//! agents, messages and the cards that need the operator.
//!
//! This is the read model behind the console's chat-shaped shell. It derives
//! nothing the other projections do not already hold:
//!
//! - agents, their messages, labels, repeats and write spans come from
//!   [`crate::team`] (Claude Code transcripts, first lines only);
//! - whether an agent can be reached comes from the bus: a note reaches the
//!   agent's lead session only when that session is known and has not ended;
//! - "verified" comes from sluice receipts, never from what a message says;
//! - what became of each of the operator's notes (delivered, read,
//!   answered) comes from the bus: delivery cursors, acks and `--ref`
//!   replies, read by serve and passed in as [`NoteTrail`];
//! - attachments are files `harbard attach` copied under the harbard home;
//!   serve looks them up there and nowhere else ([`Inputs::attachment`]);
//! - what the operator resolved, saved or read lives in the desk file
//!   (`workspace.json`), the operator's own state. Nothing here is ever sent
//!   to an agent.
//!
//! Clock, repository resolution and every input arrive as parameters so the
//! tests inject them.

use crate::activity::Activity;
use crate::coded::Params;
use crate::human;
use crate::naming;
use crate::query::{Liveness, SessionView};

use crate::sluice::{Delivery, Receipt, Verdict};
use crate::store::Store;
use crate::team::{self, AgentRow, MessageRow, TeamView};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

/// Wire version of [`WorkspaceView`].
pub const WORKSPACE_VERSION: u32 = 1;
/// A message longer than this carries a warning (agent protocol, rule 4).
pub const LONG_MESSAGE_CHARS: usize = 1_200;
/// Default dormant thresholds, in minutes. Configurable in the desk file.
pub const DORMANT_AMBER_MIN: u64 = 15;
pub const DORMANT_RED_MIN: u64 = 45;
/// Microstate thresholds, in minutes: the console words a state that lasts
/// longer than this differently. Configurable in the desk file.
pub const THINKING_LONG_MIN: u64 = 2;
pub const WAITING_SLEEPY_MIN: u64 = 5;
pub const RUNNING_LONG_MIN: u64 = 10;
/// Reaction events older than this are not sent: the console plays a
/// reaction once, as it happens, never on a later load.
pub const REACTION_WINDOW_MS: u64 = 120_000;
/// Report-like files that render as a file card and may be opened.
pub const FILE_EXTS: [&str; 4] = [".md", ".html", ".json", ".png"];
/// Deliveries listed, newest first.
const DELIVERIES_SHOWN: usize = 100;
/// Entries kept in each of the desk's maps (resolved, reads, threads).
pub const DESK_MAP_MAX: usize = 500;
/// Saved items kept in the desk file.
const SAVED_MAX: usize = 500;
/// Prefix of an operator note addressed to one agent through its lead.
pub const AGENT_NOTE_PREFIX: &str = "To ";
/// Prefix of an operator note written in a channel.
pub const CHANNEL_NOTE_PREFIX: &str = "In #";
/// The same prefixes on notes already in the store, written before they
/// were English; read so old notes keep their addressee and channel.
pub const LEGACY_AGENT_NOTE_PREFIX: &str = "Para ";
pub const LEGACY_CHANNEL_NOTE_PREFIX: &str = "En #";
/// How the lead session is named in the console.
pub const LEAD_AUTHOR: &str = "Claude · lead";
/// The lead session's role word, what its `role` field carries.
pub const LEAD_ROLE: &str = "lead";
/// Default minutes before an operator note with no answer goes to "Needs you".
pub const UNANSWERED_MIN: u64 = 10;
/// A repository quiet this long moves to "More channels".
pub const CHANNEL_QUIET_MS: u64 = 14 * 24 * 3_600_000;
/// A room shows its newest messages here; older ones come from its history.
pub const ROOM_MESSAGES_SHOWN: usize = 200;
/// Most cards the Lobby lists.
pub const LOBBY_SHOWN: usize = 200;
/// The field line that names the in-process agent a note is for: the lead
/// relays it (`harbard protocol`, messages v2).
pub const TO_AGENT_FIELD: &str = "to_agent:";

// ---------------------------------------------------------------------------
// Desk state: the operator's own marks, persisted under the harbard home
// ---------------------------------------------------------------------------

/// Thresholds the operator may change by editing `workspace.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeskConfig {
    #[serde(default = "default_amber")]
    pub dormant_amber_min: u64,
    #[serde(default = "default_red")]
    pub dormant_red_min: u64,
    /// A lead whose transcript has a record this recent is alive.
    #[serde(default = "default_lead_alive")]
    pub lead_alive_min: u64,
    /// An agent whose own last record is this recent is alive, whatever its
    /// lead looks like.
    #[serde(default = "default_agent_recent")]
    pub agent_recent_min: u64,
    /// Two writers raise a warning only when both wrote this recently.
    #[serde(default = "default_writers_recent")]
    pub writers_recent_min: u64,
    /// Past this, an agent is lost even under a live lead: a teammate that
    /// stopped a day ago mid-turn in a long-lived lead session is gone, not
    /// dormant. The same 24 h the sidebar uses.
    #[serde(default = "default_agent_max_quiet")]
    pub agent_max_quiet_min: u64,
    /// Messages before this get no protocol flags (unlabelled, too long):
    /// history is not repainted by a rule it was not written under. Set to
    /// the moment this build first opened the desk.
    #[serde(default)]
    pub protocol_since_ms: u64,
    #[serde(default = "default_operator_name")]
    pub operator_name: String,
    #[serde(default = "default_operator_initials")]
    pub operator_initials: String,
    /// "Thinking a long while" past this.
    // The alias exists only to read old files.
    #[serde(default = "default_thinking_long", alias = "pensando_long_min")]
    pub thinking_long_min: u64,
    /// "Waiting, sleepy" past this; dormant at `dormant_amber_min`.
    // The alias exists only to read old files.
    #[serde(default = "default_waiting_sleepy", alias = "esperando_sleepy_min")]
    pub waiting_sleepy_min: u64,
    /// "Running something long" past this.
    // The alias exists only to read old files.
    #[serde(default = "default_running_long", alias = "ejecutando_long_min")]
    pub running_long_min: u64,
    /// An operator note with no answer this long goes to "Needs you".
    #[serde(default = "default_unanswered")]
    pub unanswered_min: u64,
    /// A BLOCKED nobody can read (its agent lost, or its lead off the bus)
    /// leaves "Needs you" after this; it stays in All as history.
    #[serde(default = "default_blocked_unreachable")]
    pub blocked_unreachable_min: u64,
    /// The memory sink, an absolute path to an executable
    /// (`docs/memory-sink.md`). Set by hand in `workspace.json`: no route may
    /// write it, or a request could choose what serve runs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_sink_command: Option<String>,
}

/// [`DeskConfig::blocked_unreachable_min`] by default: two hours.
pub const BLOCKED_UNREACHABLE_MIN: u64 = 120;

fn default_blocked_unreachable() -> u64 {
    BLOCKED_UNREACHABLE_MIN
}

fn default_unanswered() -> u64 {
    UNANSWERED_MIN
}

fn default_thinking_long() -> u64 {
    THINKING_LONG_MIN
}

fn default_waiting_sleepy() -> u64 {
    WAITING_SLEEPY_MIN
}

fn default_running_long() -> u64 {
    RUNNING_LONG_MIN
}

/// An alive lead, an alive agent, recent writers: the defaults, in minutes.
pub const LEAD_ALIVE_MIN: u64 = 15;
pub const AGENT_RECENT_MIN: u64 = 120;
pub const WRITERS_RECENT_MIN: u64 = 120;
/// Agents whose last record is older than this leave the badges.
pub const ACTIVE_MS: u64 = 24 * 3_600_000;

fn default_lead_alive() -> u64 {
    LEAD_ALIVE_MIN
}

fn default_agent_recent() -> u64 {
    AGENT_RECENT_MIN
}

fn default_writers_recent() -> u64 {
    WRITERS_RECENT_MIN
}

fn default_agent_max_quiet() -> u64 {
    ACTIVE_MS / 60_000
}

/// Whose desk this is, until the operator says otherwise in `config` or
/// Profile. The console shows this default in the operator's own language.
pub const DEFAULT_OPERATOR_NAME: &str = "Operator";
const DEFAULT_OPERATOR_INITIALS: &str = "OP";

fn default_operator_name() -> String {
    DEFAULT_OPERATOR_NAME.into()
}

fn default_operator_initials() -> String {
    DEFAULT_OPERATOR_INITIALS.into()
}

fn default_amber() -> u64 {
    DORMANT_AMBER_MIN
}

fn default_red() -> u64 {
    DORMANT_RED_MIN
}

impl Default for DeskConfig {
    fn default() -> Self {
        Self {
            dormant_amber_min: DORMANT_AMBER_MIN,
            dormant_red_min: DORMANT_RED_MIN,
            lead_alive_min: LEAD_ALIVE_MIN,
            agent_recent_min: AGENT_RECENT_MIN,
            writers_recent_min: WRITERS_RECENT_MIN,
            agent_max_quiet_min: ACTIVE_MS / 60_000,
            protocol_since_ms: 0,
            operator_name: default_operator_name(),
            operator_initials: default_operator_initials(),
            thinking_long_min: THINKING_LONG_MIN,
            waiting_sleepy_min: WAITING_SLEEPY_MIN,
            running_long_min: RUNNING_LONG_MIN,
            unanswered_min: UNANSWERED_MIN,
            blocked_unreachable_min: BLOCKED_UNREACHABLE_MIN,
            memory_sink_command: None,
        }
    }
}

/// Something the operator saved for later. The first line is copied because
/// the message itself leaves the team window after a few days.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedItem {
    /// A message id, or `file:<path>` for a report.
    pub id: String,
    /// `message` or `report`.
    pub kind: String,
    pub saved_ms: u64,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub ts_ms: Option<u64>,
    #[serde(default)]
    pub path: Option<String>,
}

/// `{harbard home}/workspace.json`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeskState {
    #[serde(default)]
    pub config: DeskConfig,
    /// Messages older than this count as read. Set once, when the desk file
    /// is first written, so a new desk does not open on every old message.
    #[serde(default)]
    pub baseline_ms: u64,
    /// Card id to the moment it was marked resolved.
    #[serde(default)]
    pub resolved: BTreeMap<String, u64>,
    #[serde(default)]
    pub saved: Vec<SavedItem>,
    /// Agent key to the newest message time the operator has read.
    #[serde(default)]
    pub reads: BTreeMap<String, u64>,
    /// Operator note id to the message it answers, for thread replies.
    #[serde(default)]
    pub threads: BTreeMap<String, String>,
    /// Channel id to the moment the operator marked it read.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub channel_reads: BTreeMap<String, u64>,
    /// "Mark everything as read": everything before this counts as read.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub read_all_ms: u64,
    /// "Notify in the Dock": the desktop app bounces its Dock icon once for a
    /// new "Needs you" item. None is on; the desktop shell reads it here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dock_attention: Option<bool>,
    /// The console theme by id (`crate::theme`); none is Aubergine. Kept as
    /// written: a hand-edited typo must never make the whole desk unreadable.
    // The adapter exists only to read old files.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::old_spelling::read_theme_id"
    )]
    pub theme: Option<String>,
    /// `light`, `dark` or `system`; none is `light`.
    // The adapter exists only to read old files.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::old_spelling::read_theme_mode"
    )]
    pub mode: Option<String>,
    /// `reduce` stops every animation of the console, whatever the OS
    /// says; none follows the OS (`prefers-reduced-motion`).
    // The adapter exists only to read old files.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::old_spelling::read_motion"
    )]
    pub motion: Option<String>,
    /// The strip dims after 22:00 when the OS is dark; `false` turns
    /// that off in Preferences. None is on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub night_dim: Option<bool>,
    /// Off unless turned on in Preferences, and without it the Retro tick
    /// calls nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_sink_allowed: Option<bool>,
    /// "Hide": agent key to the moment the operator hid it. A hidden
    /// agent leaves the console's lists, never its "Needs you" cards.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub hidden_agents: BTreeMap<String, u64>,
    /// The sidebar's Agents section folded; none is open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents_collapsed: Option<bool>,
    /// "Was it you?": what changed what every session reads, asked of the
    /// operator in Needs you until they keep or revoke it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub checks: Vec<DeskCheck>,
    /// When the rules were first sealed under the serve key
    /// ([`crate::seal`]): the one-time sealing of older rules never runs
    /// again, even if the key goes missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules_sealed_ms: Option<u64>,
}

/// Checks kept. Answered ones go first, oldest first; an unanswered one is
/// never pushed out (changes of the same kind are grouped instead).
pub const CHECKS_MAX: usize = 50;
/// How long an answered check stays in view (Retro, Preferences).
pub const CHECKS_SHOWN_MS: u64 = 7 * 24 * 3_600_000;

/// One "Was it you?": a change to what every session reads.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeskCheck {
    /// The card is `check:<id>`.
    pub id: String,
    /// `rule` (an approval), `context` (Profile › Context) or `memory_sink`
    /// (saving to the memory sink, switched on).
    pub what: String,
    /// The rule's text or the new context, as the card quotes it.
    #[serde(default)]
    pub text: String,
    /// The approving event, for Revoke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<String>,
    /// The context before, for Revoke.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<String>,
    pub since_ms: u64,
    /// `kept` (Keep, confirmed) or `revoked`; none is unanswered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answered_ms: Option<u64>,
}

impl DeskState {
    pub fn path(root: &Path) -> PathBuf {
        root.join("workspace.json")
    }

    pub fn load(store: &Store) -> Self {
        Self::load_path(&Self::path(store.root()))
    }

    /// A missing or unreadable file is an empty desk, never an error.
    pub fn load_path(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, store: &Store) -> std::io::Result<()> {
        self.save_path(&Self::path(store.root()))
    }

    /// Written to a temp file of this writer's own and renamed: a crash
    /// leaves the old desk, and two writers never share a temp file.
    pub fn save_path(&self, path: &Path) -> std::io::Result<()> {
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = path.with_extension(format!("json.tmp.{}.{n}", std::process::id()));
        if let Err(e) = fs::write(&tmp, bytes).and_then(|()| fs::rename(&tmp, path)) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        Ok(())
    }

    /// The desk at `path` for a write: missing is empty, but a file that
    /// exists and does not parse is an error, so no write ever replaces a
    /// desk it could not read.
    pub fn load_for_write(path: &Path) -> std::io::Result<Self> {
        match fs::read_to_string(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
            Ok(raw) => serde_json::from_str(&raw).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!(
                        "{} is not a valid desk file ({e}); it was left untouched",
                        path.display()
                    ),
                )
            }),
        }
    }

    /// Load, change and save the desk at `path` as one step, under an
    /// exclusive lock on `workspace.json.lock` that every writer takes
    /// (serve and `harbard theme`), across processes.
    pub fn update<T>(path: &Path, f: impl FnOnce(&mut DeskState) -> T) -> std::io::Result<T> {
        use fs2::FileExt;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path.with_extension("json.lock"))?;
        lock.lock_exclusive()?;
        let result = Self::load_for_write(path).and_then(|mut desk| {
            let out = f(&mut desk);
            desk.save_path(path).map(|()| out)
        });
        let _ = FileExt::unlock(&lock);
        result
    }

    /// Run `f` under the same cross-process desk lock [`DeskState::update`]
    /// takes, without loading or saving the desk: for a writer whose state
    /// lives elsewhere (feedback on the bus) but must not race the desk's
    /// other writers. A process-wide mutex covers threads of one process,
    /// where two descriptors of one file are not reliably exclusive.
    pub fn with_lock<T>(path: &Path, f: impl FnOnce() -> T) -> std::io::Result<T> {
        use fs2::FileExt;
        static IN_PROCESS: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = IN_PROCESS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(path.with_extension("json.lock"))?;
        lock.lock_exclusive()?;
        let out = f();
        let _ = FileExt::unlock(&lock);
        Ok(out)
    }

    pub fn resolve(&mut self, card: &str, now_ms: u64) {
        self.resolved.entry(card.to_string()).or_insert(now_ms);
        trim_oldest(&mut self.resolved, |v| *v);
    }

    pub fn reopen(&mut self, card: &str) {
        self.resolved.remove(card);
    }

    /// Ask "Was it you?" about a change, as a Needs you card.
    ///
    /// An unanswered check is never dropped. A new change of the context, or
    /// a new switch-on of the memory sink, while one of the same kind is still
    /// unanswered is grouped into it: one card with the newest text and the
    /// oldest "before", so Revoke goes back to before all of them. Over
    /// [`CHECKS_MAX`], answered checks go, oldest first.
    pub fn add_check(&mut self, mut check: DeskCheck) {
        self.checks.retain(|c| c.id != check.id);
        if check.what != "rule"
            && let Some(at) = self
                .checks
                .iter()
                .position(|c| c.what == check.what && c.outcome.is_none())
        {
            let open = self.checks.remove(at);
            self.resolved.remove(&format!("check:{}", open.id));
            check.previous = open.previous;
            check.since_ms = open.since_ms;
        }
        self.checks.push(check);
        while self.checks.len() > CHECKS_MAX {
            let Some(at) = self.checks.iter().position(|c| c.outcome.is_some()) else {
                break;
            };
            let gone = self.checks.remove(at);
            self.resolved.remove(&format!("check:{}", gone.id));
        }
    }

    /// Close a check with its answer (`kept` or `revoked`).
    pub fn answer_check(&mut self, card: &str, outcome: &str, now_ms: u64) {
        if let Some(id) = card.strip_prefix("check:")
            && let Some(c) = self.checks.iter_mut().find(|c| c.id == id)
        {
            c.outcome = Some(outcome.to_string());
            c.answered_ms = Some(now_ms);
        }
        self.resolve(card, now_ms);
    }

    /// The checks to show: unanswered ones, and those answered in the last
    /// [`CHECKS_SHOWN_MS`], newest first.
    pub fn recent_checks(&self, now_ms: u64) -> Vec<DeskCheck> {
        let mut out: Vec<DeskCheck> = self
            .checks
            .iter()
            .filter(|c| {
                c.answered_ms
                    .is_none_or(|at| now_ms.saturating_sub(at) <= CHECKS_SHOWN_MS)
            })
            .cloned()
            .collect();
        out.sort_by_key(|c| std::cmp::Reverse(c.since_ms));
        out
    }

    /// The check a `check:<id>` card stands for.
    pub fn check_of(&self, card: &str) -> Option<&DeskCheck> {
        let id = card.strip_prefix("check:")?;
        self.checks.iter().find(|c| c.id == id)
    }

    /// Note ids are ULIDs, so the smallest key is the oldest link.
    pub fn link_thread(&mut self, note: &str, message: &str) {
        self.threads.insert(note.to_string(), message.to_string());
        while self.threads.len() > DESK_MAP_MAX {
            let Some(oldest) = self.threads.keys().next().cloned() else {
                break;
            };
            self.threads.remove(&oldest);
        }
    }

    pub fn mark_channel_read(&mut self, channel: &str, ts_ms: u64) {
        let e = self.channel_reads.entry(channel.to_string()).or_insert(0);
        *e = (*e).max(ts_ms);
        trim_oldest(&mut self.channel_reads, |v| *v);
    }

    /// Everything up to `ts_ms` counts as read: channels, Direct, All.
    /// "Needs you" is untouched; it has its own "Mark resolved".
    pub fn mark_all_read(&mut self, ts_ms: u64) {
        self.read_all_ms = self.read_all_ms.max(ts_ms);
    }

    /// Hiding twice keeps the first time.
    pub fn hide_agent(&mut self, agent: &str, now_ms: u64) {
        self.hidden_agents
            .entry(agent.to_string())
            .or_insert(now_ms);
        trim_oldest(&mut self.hidden_agents, |v| *v);
    }

    pub fn show_agent(&mut self, agent: &str) {
        self.hidden_agents.remove(agent);
    }

    pub fn show_all_agents(&mut self) {
        self.hidden_agents.clear();
    }

    pub fn mark_read(&mut self, agent: &str, ts_ms: u64) {
        let seen = self.reads.entry(agent.to_string()).or_insert(0);
        *seen = (*seen).max(ts_ms);
        trim_oldest(&mut self.reads, |v| *v);
    }

    /// Saving twice keeps the first save.
    pub fn save_item(&mut self, item: SavedItem) {
        if self.saved.iter().any(|s| s.id == item.id) {
            return;
        }
        self.saved.push(item);
        if self.saved.len() > SAVED_MAX {
            let drop = self.saved.len() - SAVED_MAX;
            self.saved.drain(..drop);
        }
    }

    pub fn unsave(&mut self, id: &str) {
        self.saved.retain(|s| s.id != id);
    }
}

/// Drop the entries with the smallest timestamp until the map fits.
fn is_zero(v: &u64) -> bool {
    *v == 0
}

fn trim_oldest<V>(map: &mut BTreeMap<String, V>, ts: impl Fn(&V) -> u64) {
    while map.len() > DESK_MAP_MAX {
        let Some(oldest) = map
            .iter()
            .min_by_key(|(k, v)| (ts(v), (*k).clone()))
            .map(|(k, _)| k.clone())
        else {
            break;
        };
        map.remove(&oldest);
    }
}

// ---------------------------------------------------------------------------
// Inputs
// ---------------------------------------------------------------------------

/// What a path resolves to on disk: the checkout it is in, that checkout's
/// repository, and its branch. `None` from the resolver for a path in no
/// checkout (temp space, documents, a deleted tree with no telltale name).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckoutInfo {
    /// The checkout root: a worktree or a primary checkout.
    pub checkout: String,
    /// The primary checkout of the repository the path belongs to.
    pub repo: String,
    pub branch: Option<String>,
    /// Confirmed a git repository: by its `.git` on disk or by the bus. A
    /// lexical guess (a removed worktree's name) only joins a repository
    /// that is confirmed some other way; alone it is never a channel.
    pub real: bool,
}

/// What the workspace needs from one bus session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusSession {
    pub session: String,
    pub liveness: Liveness,
    /// The checkout the session works in, when it is in one.
    pub repo: Option<String>,
    pub last_ms: u64,
    pub first_ms: u64,
    pub cwd: String,
    /// The name it chose with `harbard name`, if any.
    pub name: Option<String>,
    /// How the bus calls it (Charles), the name it answers to.
    pub callsign: Option<String>,
    /// A top-level agent session (not the operator, not a herd child).
    pub main: bool,
    pub harness: String,
    /// Quiet but its process is confirmed alive.
    pub idle: bool,
    /// The rooms it joined, resolved from its anchors by serve.
    pub rooms: Vec<String>,
}

impl From<&SessionView> for BusSession {
    fn from(s: &SessionView) -> Self {
        BusSession {
            session: s.session.clone(),
            liveness: s.liveness,
            repo: s.repo.clone(),
            last_ms: s.last_ms,
            first_ms: s.first_ms,
            cwd: s.cwd.clone(),
            name: None,
            callsign: None,
            main: s.role == crate::event::Role::Main && s.harness != crate::event::HARNESS_HUMAN,
            harness: s.harness.clone(),
            idle: s.idle,
            rooms: Vec::new(),
        }
    }
}

/// A bus note for the operator from any session: addressed to the human or
/// to everyone, or answering another one by `ref`. Answers to the
/// operator's own notes ride [`NoteTrail::replies`] instead.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BusNote {
    pub id: String,
    pub ts_ms: u64,
    pub session: String,
    pub text: String,
    /// The repository the author wrote it from.
    pub repo: Option<String>,
    pub note_ref: Option<String>,
    /// `fyi`, `warning`, `ask` or `handoff`; a handoff is a Needs you card.
    pub kind: String,
}

/// An operator note addressed to one session.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperatorNote {
    pub id: String,
    pub ts_ms: u64,
    pub session: String,
    pub text: String,
    /// What the bus says became of it.
    pub trail: NoteTrail,
}

/// Bus evidence about one operator note, gathered by serve.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoteTrail {
    /// The session's brief, inbox or watch handed it over, after the session
    /// joined the bus (a note from before its join is never delivered).
    pub delivered: bool,
    /// The session acknowledged it (`harbard ack`), at this time.
    pub acked_ms: Option<u64>,
    /// Notes from anyone but the operator whose `ref` names it (or a re-send).
    pub replies: Vec<BusReply>,
    /// The operator re-sent it as an `ask` ("Resend to the lead").
    pub nudged_ms: Option<u64>,
}

/// A bus note that answers an operator note by `ref`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BusReply {
    pub id: String,
    pub ts_ms: u64,
    pub session: String,
    pub text: String,
}

/// One room a workspace shows ([`crate::rooms::rooms_in_workspace`]).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RoomInput {
    pub slug: String,
    pub purpose: Option<String>,
    pub purpose_code: Option<&'static str>,
    pub archived: bool,
    pub built_in: bool,
    pub created_ms: Option<u64>,
    pub last_note_ms: Option<u64>,
    /// The operator's read watermark, an event id.
    pub read: Option<String>,
}

/// A note addressed to a room, placed in the workspace this view is for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoomNote {
    pub id: String,
    pub ts_ms: u64,
    pub room: String,
    pub session: String,
    /// The name its author goes by; the id's short form when none is known.
    pub author: String,
    pub operator: bool,
    pub text: String,
    pub note_ref: Option<String>,
    pub repo: Option<String>,
    pub kind: String,
}

impl RoomNote {
    /// A `channel:<slug>` note as the view reads it.
    pub fn of(event: &crate::event::Event, author: String) -> Option<Self> {
        let to = event.to.as_ref()?;
        if event.event != crate::event::EventKind::Note
            || to.kind != crate::event::RecipientKind::Channel
        {
            return None;
        }
        Some(RoomNote {
            id: event.id.clone(),
            ts_ms: crate::query::event_ms(event),
            room: to.value.clone(),
            session: event.session.clone(),
            author,
            operator: event.is_operator(),
            text: event.text.clone(),
            note_ref: event.note_ref.clone(),
            repo: event.repository().map(str::to_string),
            kind: event.kind.as_str().to_string(),
        })
    }
}

/// The id a room's message carries in the view.
pub fn room_message_id(event_id: &str) -> String {
    format!("room:{event_id}")
}

/// The desk key of a room's read mark: a project channel's is its id.
pub fn room_desk_key(workspace: Option<&str>, slug: &str) -> String {
    match workspace {
        Some(id) => format!("{id}/{slug}"),
        None => slug.to_string(),
    }
}

pub struct Inputs<'a> {
    pub team: &'a TeamView,
    pub sessions: &'a [BusSession],
    pub notes: &'a [OperatorNote],
    pub bus_notes: &'a [BusNote],
    /// Extra temp dirs (`$TMPDIR`) besides [`naming::TEMP_DIRS`].
    pub temp_dirs: &'a [String],
    /// Workshop names by agent key ([`crate::workshop::Registry`]); an agent
    /// with none keeps its `<team> · <role>` name.
    pub names: &'a BTreeMap<String, crate::workshop::Entry>,
    pub deliveries: &'a [Delivery],
    pub receipts: &'a [Receipt],
    pub desk: &'a DeskState,
    pub resolve: &'a dyn Fn(&str) -> Option<CheckoutInfo>,
    /// A path a message names, when it is a file `harbard attach` copied
    /// under the harbard home; `None` for anything else, which keeps today's
    /// file card. Serve answers from the attachments folder only.
    pub attachment: &'a dyn Fn(&str) -> Option<Attachment>,
    pub now_ms: u64,
    pub operator: &'a str,
    /// How far back a bus session is a conversation: [`ACTIVE_MS`] for the
    /// shell, [`crate::history::WINDOW_MS`] for History.
    pub horizon_ms: u64,
    /// History keeps ended sessions (read-only there); the shell drops them.
    pub keep_ended: bool,
    /// Whether a repository (its identity, as [`CheckoutInfo::repo`]) is in
    /// the workspace this view is for. A repository outside it never becomes
    /// a channel here, whoever worked in it. Serve passes the workspace
    /// boundary; a hub with one workspace keeps every repository.
    pub in_workspace: &'a dyn Fn(&str) -> bool,
    /// Every repository the bus has seen, each a permanent channel when the
    /// resolver says it is real ([`crate::rooms::known_repos`]).
    pub known_repos: &'a [crate::rooms::KnownRepo],
    /// The rooms this workspace shows; built-ins included.
    pub rooms: &'a [RoomInput],
    /// Notes in rooms, placed in this workspace, oldest first.
    pub room_notes: &'a [RoomNote],
    /// Repositories whose project channel the operator archived.
    pub archived_repos: &'a BTreeSet<String>,
    /// The workspace this view is for; `None` on a hub with one.
    pub workspace: Option<&'a str>,
}

// ---------------------------------------------------------------------------
// Wire shape
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Default)]
pub struct WorkspaceView {
    pub v: u32,
    pub now_ms: u64,
    pub operator: String,
    pub operator_initials: String,
    pub thresholds: Thresholds,
    pub channels: Vec<Channel>,
    /// Who is around: one card per main session and per named agent.
    pub lobby: Vec<LobbyCard>,
    pub agents: Vec<Agent>,
    pub messages: Vec<Message>,
    pub cards: Vec<Card>,
    pub deliveries: Vec<DeliveryRow>,
    pub saved: Vec<SavedItem>,
    pub unread: Unread,
    pub team_root_found: bool,
    pub format_changed: bool,
    /// The operator's motion choice: `reduce`, or none to follow the OS.
    pub motion: Option<String>,
    /// Whether the strip dims at night (Preferences); on unless turned off.
    pub night_dim: bool,
    /// The sidebar's Agents section is folded (the operator's choice, on the desk).
    pub agents_collapsed: bool,
    /// "Deliveries": every DONE and where its commit is now
    /// ([`crate::shipments`]). Filled by serve's read, which asks git;
    /// empty from [`build`].
    pub shipments: Vec<crate::shipments::Shipment>,
    /// The commit serve was built from, when known: what "Installed"
    /// compares against.
    pub running_commit: Option<String>,
    /// Changes whenever a message, a reply, a delivery state or a card does:
    /// the faces' slice carries it, so an open console pulls the full view
    /// within one slice of a new answer.
    pub rev: String,
}

/// `GET /api/v1/workspace?only=activity`: what the faces and the logo need,
/// polled often; the rest of the view is polled at its own pace.
#[derive(Debug, Clone, Serialize)]
pub struct ActivityView {
    pub v: u32,
    pub now_ms: u64,
    pub thresholds: Thresholds,
    pub agents: Vec<AgentActivity>,
    pub unread: Unread,
    /// Open items in "Needs you" (the Activity badge), for the Dock.
    pub needs: usize,
    /// Agent messages unread in Direct, for the Dock.
    pub directs: usize,
    /// Open BLOCKED cards in "Needs you".
    pub blocked: usize,
    pub motion: Option<String>,
    /// [`WorkspaceView::rev`].
    pub rev: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct AgentActivity {
    pub key: String,
    pub presence: &'static str,
    pub dormant: Option<&'static str>,
    pub last_action_ms: Option<u64>,
    pub activity: Activity,
    pub reactions: Vec<Reaction>,
}

impl ActivityView {
    pub fn of(view: &WorkspaceView) -> Self {
        ActivityView {
            v: view.v,
            now_ms: view.now_ms,
            thresholds: view.thresholds,
            agents: view
                .agents
                .iter()
                .map(|a| AgentActivity {
                    key: a.key.clone(),
                    presence: a.presence,
                    dormant: a.dormant,
                    last_action_ms: a.last_action_ms,
                    activity: a.activity.clone(),
                    reactions: a.reactions.clone(),
                })
                .collect(),
            unread: view.unread,
            needs: view.unread.activity,
            directs: view.unread.directs,
            blocked: view
                .cards
                .iter()
                .filter(|c| c.kind == "blocked" && c.needs && c.resolved_ms.is_none())
                .count(),
            motion: view.motion.clone(),
            rev: view.rev.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
pub struct Thresholds {
    pub amber_ms: u64,
    pub red_ms: u64,
    pub thinking_long_ms: u64,
    pub waiting_sleepy_ms: u64,
    pub running_long_ms: u64,
}

/// A one-shot reaction the console animates once: `glance` (a new message
/// from the agent), `spark` (the operator's note reached its lead), `hop`
/// (a verified delivery), `shake` (a new BLOCKED), `boot` (a new agent).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Reaction {
    pub kind: &'static str,
    pub at_ms: u64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq, Default)]
pub struct Unread {
    /// Agent messages newer than what the operator read.
    pub directs: usize,
    /// Open cards in "Needs you".
    pub activity: usize,
    /// Messages in channels newer than each channel's read mark.
    pub channels: usize,
    /// Labelled agent messages unread (Activity's All).
    pub todo: usize,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct Channel {
    /// Lowercase repository name, or the room's slug; unique.
    pub id: String,
    /// `project` or `room`.
    pub kind: &'static str,
    /// Why a room exists; `None` for a project.
    pub purpose: Option<String>,
    /// The console's code for a built-in room's purpose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose_code: Option<&'static str>,
    /// Hidden from the rail by the operator; its messages stay.
    pub archived: bool,
    /// Exists without being created and cannot be removed.
    pub built_in: bool,
    /// Empty for a room.
    pub repo: String,
    pub checkouts: Vec<ChannelCheckout>,
    /// Agent keys that wrote or ran git in this repository.
    pub members: Vec<String>,
    pub last_ms: u64,
    pub warnings: Vec<WriterWarning>,
    /// Sessions a note in this channel is stored for: the members' leads and
    /// the bus sessions working in the repository, each once.
    pub readers: Vec<ChannelReader>,
    /// `ready` when a live session reads it now; `stale` when one may come
    /// back; `none` when only an ended or unknown session would get it;
    /// `nobody` when there is no session at all.
    pub reach: &'static str,
    /// Newest real activity: a write, a message, a live session. Running git
    /// to read a repository is not activity.
    pub active_ms: u64,
    /// No activity in [`CHANNEL_QUIET_MS`]: under "More channels".
    pub quiet: bool,
    /// Messages newer than the channel's read mark.
    pub unread: usize,
    /// Why the channel is here: the agents and sessions that wrote in it,
    /// or worked in it as a session, in the last 24 hours, newest first.
    /// Empty for a repository nobody works in now.
    pub recent: Vec<String>,
}

/// One card of the Lobby.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LobbyCard {
    /// Same as `agents[].key` when the agent is in `agents`.
    pub key: String,
    pub name: String,
    /// `claude-code`, `codex`, `cursor` or `other`.
    pub harness: &'static str,
    pub project: Option<String>,
    /// The latest thing it said in `#lobby`.
    pub intro: Option<String>,
    pub intro_ts_ms: Option<u64>,
    /// `live`, `idle`, `stale` or `ended`.
    pub presence: &'static str,
    pub first_ms: u64,
    pub last_ms: u64,
}

fn harness_label(harness: &str) -> &'static str {
    match harness {
        "claude-code" => "claude-code",
        "codex" => "codex",
        "cursor" => "cursor",
        _ => "other",
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChannelReader {
    pub session: String,
    /// `ready`, `stale`, `ended` or `not_on_bus`, as [`Agent::delivery`].
    pub delivery: &'static str,
    /// The lead session of an agent in this channel.
    pub lead: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChannelCheckout {
    pub path: String,
    pub branch: Option<String>,
}

/// Two or more agents writing in one checkout over overlapping spans.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WriterWarning {
    pub checkout: String,
    /// Agent keys; `lead:<session>` for a lead session itself.
    pub agents: Vec<String>,
    pub since_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct Agent {
    /// The transcript id: unique even when names repeat across leads.
    pub key: String,
    /// The agent's own name, or the first words of its brief when it has none.
    pub name: String,
    /// Type and short id, for the profile: `general-purpose · aa18a5f8`.
    pub label: String,
    /// Neither its lead nor itself has written recently: whatever its last
    /// record says, nothing is known since. Grey, never dormant.
    pub lost: bool,
    pub kind: &'static str,
    pub lead_session: String,
    /// How long the lead session has been silent on the bus, when known.
    pub lead_quiet_ms: Option<u64>,
    /// `ready` when a note can reach the lead session now; `stale` when it
    /// is stored and waits for a silent lead; else `ended` or `not_on_bus`.
    pub delivery: &'static str,
    pub state: &'static str,
    pub state_reason: &'static str,
    /// `working`, `waiting` or `finished` (green, amber, grey).
    pub presence: &'static str,
    /// `amber` past the first threshold, `red` past the second.
    pub dormant: Option<&'static str>,
    pub last_action_ms: Option<u64>,
    pub first_ms: Option<u64>,
    pub brief: Option<String>,
    pub description: Option<String>,
    pub checkout: Option<String>,
    pub branch: Option<String>,
    pub build_dir: Option<String>,
    pub channels: Vec<String>,
    pub deliveries: usize,
    pub verified_deliveries: usize,
    pub reports: Vec<String>,
    pub unread: usize,
    pub last_message: Option<String>,
    /// harbard has no way to stop a Claude Code teammate or subagent; the
    /// console shows no stop button while this is false.
    pub can_stop: bool,
    /// The face: the transcript's activity, with dormant, blocked, finished
    /// and verified laid over it from the same evidence the cards use.
    pub activity: Activity,
    /// Reactions newer than [`REACTION_WINDOW_MS`], the newest of each kind.
    pub reactions: Vec<Reaction>,
    /// The hover card: last steps, tool calls, commits and the last test
    /// counts, from the transcript ([`crate::progress::Progress`]).
    pub progress: crate::progress::Progress,
    /// `robots` of "robots · builder": what `@robots` reaches.
    pub team: Option<String>,
    pub role: Option<String>,
    /// What an `@` names it by: its own name, lower-cased.
    pub handle: String,
    /// Its lead session works in system temp space: never in Needs you,
    /// in a channel or dormant; listed under "Sesiones temporales".
    pub temporary: bool,
    /// The console's code for a `name` harbard worded (`None` for one
    /// someone chose), filled from `name_params`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_code: Option<&'static str>,
    #[serde(skip_serializing_if = "Params::is_empty")]
    pub name_params: Params,
    /// The line under a workshop name: `<role> · <team>`.
    pub role_line: Option<String>,
    /// The console's code for a `role_line` built from a known role, filled
    /// from `role_line_params`; `None` when the role is free text.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role_line_code: Option<&'static str>,
    #[serde(skip_serializing_if = "Params::is_empty")]
    pub role_line_params: Params,
    /// `auto`, `picked` (a session chose it) or `renamed` (the operator did).
    pub name_source: Option<String>,
    /// A session's one-line reason for the name it picked.
    pub name_reason: Option<String>,
    /// The name the agent itself goes by (its Agent tool name), for Profile.
    pub own_name: Option<String>,
    /// Its lead stopped it (TaskStop): "Closed", grey check.
    pub closed: bool,
    /// The operator hid it ("Hide"): out of the lists, its cards still raised.
    pub hidden: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct Message {
    pub id: String,
    /// The agent whose conversation this belongs to.
    pub agent: String,
    /// `agent`, `lead`, `peer` or `operator`.
    pub from: &'static str,
    pub author: String,
    pub ts_ms: u64,
    /// The human line: the sentence after the protocol prefix, or the whole
    /// first line.
    pub text: String,
    /// Messages v2: the technical detail after the blank line, shown under
    /// "Technical details". `None` for v1, which reads as it always did.
    pub body: Option<String>,
    pub label: Option<&'static str>,
    pub version: Option<u32>,
    /// `unlabeled`, `too_long`, `repeat`, `jargon`.
    pub flags: Vec<&'static str>,
    /// Why the human line reads as jargon ([`crate::human::jargon`]).
    pub jargon: Vec<&'static str>,
    /// A plain line for a human line written for machines
    /// ([`crate::readable::readable`]); the console shows it in its place and
    /// keeps the original under "Technical details". `None`: the line reads.
    pub readable: Option<Vec<crate::readable::Part>>,
    /// `delivery`, `review` or `blocked` ([`crate::readable::shape`]): the
    /// console groups a run of one kind from one team.
    pub shape: Option<&'static str>,
    pub repeat_of: Option<String>,
    pub chars: usize,
    pub files: Vec<String>,
    pub verified: Option<Verified>,
    /// Channels this message shows in (an operator note in a channel has one).
    pub channels: Vec<String>,
    pub reply_to: Option<String>,
    pub replies: Vec<String>,
    pub structured: Option<String>,
    /// For an operator note: whether the bus can carry it to the lead.
    pub undelivered: bool,
    /// Files `harbard attach` copied, named in the message.
    pub attachments: Vec<Attachment>,
    /// For an operator note: what became of it.
    pub delivery: Option<NoteDelivery>,
    /// The `@` tokens in it that name someone (highlighted).
    pub mentions: Vec<String>,
}

/// A file under the harbard home's attachments folder.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Attachment {
    /// `<ulid>/<name>`: what `GET /api/v1/attachments/<id>` serves.
    pub id: String,
    pub name: String,
    /// `image`, `markdown`, `diff`, `text`, `json` or `html`.
    pub kind: &'static str,
    /// `before`, `after`, `report` or `code` (`harbard attach --label`).
    pub label: Option<String>,
    pub bytes: u64,
    /// The path as the message named it.
    pub path: String,
}

/// Where an operator note stands, from the bus.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct NoteDelivery {
    /// `sent`, `delivered`, `leido` or `answered`.
    pub state: &'static str,
    /// The sessions it was stored for.
    pub sessions: Vec<String>,
    /// How many of them got it.
    pub delivered: usize,
    /// Whether every session is a lead session (worded "the coordinator").
    pub to_lead: bool,
    pub nudged_ms: Option<u64>,
    /// Who answered instead of the agent the note was for. `None` when
    /// nobody has, or when the answer was the agent's own.
    pub answered_by: Option<String>,
}

/// Evidence for "verified": a sluice receipt that landed this commit.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Verified {
    pub sha: String,
    pub branch: String,
    pub decided_ms: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Card {
    pub id: String,
    /// `blocked`, `approval`, `dormant`, `two_writers`, `unanswered`,
    /// `mention` (an agent wrote `@operator`), `handoff` (a session handed
    /// them work: "Handover") or `check` ("Was it you?").
    pub kind: &'static str,
    pub channel: Option<String>,
    pub agents: Vec<String>,
    pub message: Option<String>,
    pub checkout: Option<String>,
    pub since_ms: u64,
    pub resolved_ms: Option<u64>,
    /// In "Needs you" (and the badge). False for what is only history,
    /// such as an overlap of writers that both stopped hours ago.
    pub needs: bool,
    /// A "Was it you?" card (`check`): what changed and its text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<CardCheck>,
}

/// What a `check` card asks about.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CardCheck {
    /// `rule`, `context` or `memory_sink`.
    pub what: String,
    pub text: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeliveryRow {
    pub id: String,
    pub channel: Option<String>,
    pub sha: String,
    pub branch: String,
    /// `landed`, `conflict`, `rejected`, `failed` or `pending`.
    pub verdict: &'static str,
    pub created_ms: u64,
    pub decided_ms: Option<u64>,
    pub session: String,
}

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/// Every absolute path in `text` that names a file card: `.md`, `.html`,
/// `.json` or `.png`, in order, without duplicates.
pub fn file_paths(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split(char::is_whitespace) {
        let token = raw
            .trim_start_matches(['`', '\'', '"', '(', '[', '<', '*'])
            .trim_end_matches([
                '`', '\'', '"', ')', ']', '>', ',', ';', '*', '.', ':', '!', '?',
            ]);
        let token = token.split_once(':').map_or(token, |(p, _)| p);
        if !token.starts_with('/') || token.len() > 400 || token.contains("/../") {
            continue;
        }
        let lower = token.to_ascii_lowercase();
        if FILE_EXTS.iter().any(|e| lower.ends_with(e)) && !out.iter().any(|p| p == token) {
            out.push(token.to_string());
        }
    }
    out
}

/// Whether `path` may be opened: absolute, no traversal, a file-card type.
pub fn openable(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    path.starts_with('/')
        && !path.split('/').any(|s| s == "..")
        && !path.contains('\0')
        && FILE_EXTS.iter().any(|e| lower.ends_with(e))
}

/// The build dir an agent declared in its START line (`build dir <path>`,
/// `target dir <path>`, `CARGO_TARGET_DIR=<path>`).
pub fn declared_build_dir(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    for key in ["build dir ", "target dir ", "cargo_target_dir="] {
        if let Some(at) = lower.find(key) {
            let rest = &line[at + key.len()..];
            let token = rest
                .split(|c: char| c.is_whitespace() || c == ',' || c == ';')
                .next()
                .unwrap_or("")
                .trim_matches(['`', '\'', '"', '(', ')'])
                .trim_end_matches('.');
            if !token.is_empty() {
                return Some(token.to_string());
            }
        }
    }
    None
}

/// Folders macOS guards with a consent dialog (TCC), relative to home, and
/// mounted volumes. serve never stats or reads below them on its own: a
/// LaunchAgent touching them can raise a window nobody asked for.
pub const PROTECTED_UNDER_HOME: [&str; 10] = [
    "Documents",
    "Desktop",
    "Downloads",
    "Library/Mobile Documents",
    // File Provider (Google Drive, Dropbox, OneDrive) and app sandboxes:
    // reading them can raise a dialog or wake a sync client.
    "Library/CloudStorage",
    "Library/Containers",
    "Library/Group Containers",
    "Pictures",
    "Movies",
    "Music",
];

/// Is `path` at or below a TCC-protected folder or a mounted volume?
pub fn protected(path: &str, home: &str) -> bool {
    let under = |root: &str| path == root || path.starts_with(&format!("{root}/"));
    under("/Volumes")
        || (!home.is_empty()
            && PROTECTED_UNDER_HOME
                .iter()
                .any(|p| under(&format!("{}/{p}", home.trim_end_matches('/')))))
}

/// The worktree a path names by convention: `<repo>/.claude/worktrees/<x>`
/// or a `<repo>-wt-<x>` directory. Lexical.
pub fn worktree_component(path: &str) -> Option<String> {
    if let Some((repo, rest)) = path.split_once("/.claude/worktrees/") {
        let name = rest.split('/').next().filter(|n| !n.is_empty())?;
        return Some(format!("{repo}/.claude/worktrees/{name}"));
    }
    let mut prefix = String::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        prefix.push('/');
        prefix.push_str(part);
        if part.contains("-wt-") {
            return Some(prefix);
        }
    }
    None
}

/// `<repo>/.claude/worktrees/<x>` and `<repo>-wt-<x>` belong to `<repo>`.
pub fn by_convention(path: &str) -> String {
    if let Some((repo, _)) = path.split_once("/.claude/worktrees/") {
        return repo.to_string();
    }
    let (parent, name) = path.rsplit_once('/').unwrap_or(("", path));
    match name.split_once("-wt-") {
        Some((repo, _)) if !repo.is_empty() => format!("{parent}/{repo}"),
        _ => path.to_string(),
    }
}

/// A checkout root known only lexically (the team view's git evidence):
/// its worktree by name, its repository by convention, no branch.
pub fn by_name(root: &str) -> CheckoutInfo {
    let checkout = worktree_component(root).unwrap_or_else(|| root.to_string());
    CheckoutInfo {
        repo: by_convention(&checkout),
        checkout,
        branch: None,
        real: false,
    }
}

/// The folded checkout root (team view) that contains `path`.
fn folded_root<'r>(path: &str, roots: impl Iterator<Item = &'r str>) -> Option<&'r str> {
    roots
        .filter(|r| path == *r || path.starts_with(&format!("{r}/")))
        .max_by_key(|r| r.len())
}

/// Lowercase name of a repository root, the channel id.
fn repo_name(repo: &str) -> String {
    repo.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(repo)
        .to_ascii_lowercase()
}

fn label_of(row: &MessageRow) -> (Option<&'static str>, Option<u32>) {
    let Some(label) = row.label.as_deref() else {
        return (None, None);
    };
    let (word, version) = match label.split_once(" v") {
        Some((w, v)) => (w, v.parse().ok()),
        None => (label, None),
    };
    (team::LABELS.iter().find(|l| **l == word).copied(), version)
}

fn is_lead_peer(peer: &str) -> bool {
    matches!(peer, "team-lead" | "lead")
}

/// Is two spans' time overlapping?
fn overlaps(a: (u64, u64), b: (u64, u64)) -> bool {
    a.0 <= b.1 && b.0 <= a.1
}

// ---------------------------------------------------------------------------
// The build
// ---------------------------------------------------------------------------

struct AgentCtx<'a> {
    row: &'a AgentRow,
    lead: &'a str,
    /// Where its lead runs, when the agent has one: the repository a
    /// worktree the harness gave it was cut from.
    lead_cwd: Option<&'a str>,
    lead_alive: bool,
    /// The display name, `<team> · <role>`.
    name: String,
    /// The name the agent itself goes by (its teammate id, a note prefix).
    raw: String,
    team: Option<String>,
    role: Option<String>,
    /// The console's code for a name harbard worded, and what fills it.
    name_code: Option<&'static str>,
    name_params: Params,
    temporary: bool,
    /// The workshop name, when one is assigned.
    workshop: Option<&'a crate::workshop::Entry>,
}

/// Type and short id: what tells two unnamed agents apart.
fn id_label(row: &AgentRow) -> String {
    let id: String = row.id.chars().take(8).collect();
    match &row.agent_type {
        Some(t) => format!("{t} · {id}"),
        None => id,
    }
}

/// The agent's display name, `<team> · <role>` from its own name
/// ([`naming::display`]); an unnamed subagent is "<project> · session HH:MM",
/// never the first words of its brief. `project` is the lead's.
pub fn display_name(row: &AgentRow, project: &str) -> String {
    naming::display(row.name.as_deref(), project, row.first_ms).name
}

/// A bus session as a row the conversation code reads like an agent's: no
/// transcript, so no messages of its own; the bus says the rest.
fn session_row(s: &BusSession, lead: bool) -> AgentRow {
    let live = s.liveness == Liveness::Live;
    let state = match s.liveness {
        Liveness::Ended => "done",
        _ => "idle",
    };
    AgentRow {
        id: format!("session:{}", s.session),
        name: if lead {
            Some(
                s.callsign
                    .clone()
                    .unwrap_or_else(|| LEAD_AUTHOR.to_string()),
            )
        } else {
            s.name.clone()
        },
        kind: "session",
        agent_type: None,
        color: None,
        model: None,
        description: None,
        brief: None,
        state,
        state_reason: if live { "on_bus" } else { "quiet" },
        last_action_ms: Some(s.last_ms),
        seen_ms: s.last_ms,
        records: 0,
        checkouts: Vec::new(),
        unplaced_writes: 0,
        worktree: s.repo.clone(),
        messages: Vec::new(),
        messages_total: 0,
        last_label: None,
        first_ms: Some(s.first_ms),
        git_roots: Vec::new(),
        write_dirs: Vec::new(),
        git_dirs: Vec::new(),
        target_dirs: Vec::new(),
        activity: Activity::plain(state, s.last_ms),
        progress: Default::default(),
    }
}

/// The text of a note addressed to `name`, under the prefix of today or of
/// a note already in the store.
fn agent_note_body<'n>(text: &'n str, name: &str) -> Option<&'n str> {
    [AGENT_NOTE_PREFIX, LEGACY_AGENT_NOTE_PREFIX]
        .iter()
        .find_map(|prefix| text.strip_prefix(&format!("{prefix}{name}: ")))
}

/// The text of a channel note, under either prefix.
fn channel_note_body(text: &str) -> Option<&str> {
    text.strip_prefix(CHANNEL_NOTE_PREFIX)
        .or_else(|| text.strip_prefix(LEGACY_CHANNEL_NOTE_PREFIX))
}

/// Whether a DM note is for this agent: its `to_agent:` line names it, or
/// (older notes) it opens with the agent's name. Returns the text after the
/// prefix.
fn dm_body<'n>(text: &'n str, key: &str, names: &[&str]) -> Option<&'n str> {
    for n in names {
        if let Some(rest) = agent_note_body(text, n) {
            return Some(rest);
        }
    }
    let tagged = text.lines().skip(1).any(|l| {
        l.trim()
            .strip_prefix(TO_AGENT_FIELD)
            .is_some_and(|v| v.trim() == key)
    });
    tagged.then(|| text.split_once(": ").map_or(text, |(_, r)| r))
}

/// A note's text without its `to_agent:` field lines.
pub fn without_fields(text: &str) -> String {
    text.lines()
        .filter(|l| !l.trim().starts_with(TO_AGENT_FIELD))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn build(inp: &Inputs<'_>) -> WorkspaceView {
    let now = inp.now_ms;
    let cfg = &inp.desk.config;
    let amber_ms = cfg.dormant_amber_min.max(1) * 60_000;
    let red_ms = (cfg.dormant_red_min * 60_000).max(amber_ms);
    let lead_alive_ms = cfg.lead_alive_min * 60_000;
    let agent_recent_ms = cfg.agent_recent_min * 60_000;
    let agent_max_quiet_ms = cfg.agent_max_quiet_min * 60_000;
    let writers_since = now.saturating_sub(cfg.writers_recent_min * 60_000);

    // Bus reachability of each lead session.
    let mut liveness: HashMap<&str, (Liveness, u64)> = HashMap::new();
    for s in inp.sessions {
        liveness.insert(s.session.as_str(), (s.liveness, s.last_ms));
    }
    // A stale lead may come back and read the note; it is not reachable now.
    let delivery_of = |lead: &str| match liveness.get(lead) {
        Some((Liveness::Ended, _)) => "ended",
        Some((Liveness::Stale, _)) => "stale",
        Some(_) => "ready",
        None => "not_on_bus",
    };
    let lead_quiet = |lead: &str| liveness.get(lead).map(|(_, ms)| now.saturating_sub(*ms));

    // Every agent across every lead.
    let mut agents_ctx: Vec<AgentCtx<'_>> = Vec::new();
    for lead in &inp.team.leads {
        let project = naming::project_of(lead.cwd.as_deref());
        let temporary = naming::is_temp_project(&lead.project, inp.temp_dirs)
            || lead
                .cwd
                .as_deref()
                .is_some_and(|c| naming::is_temp(c, inp.temp_dirs));
        for row in &lead.agents {
            let d = naming::display(row.name.as_deref(), &project, row.first_ms);
            let workshop = inp.names.get(&row.id);
            agents_ctx.push(AgentCtx {
                row,
                lead: lead.session.as_str(),
                lead_cwd: lead.cwd.as_deref(),
                lead_alive: lead
                    .lead_last_ms
                    .is_some_and(|ms| now.saturating_sub(ms) <= lead_alive_ms),
                raw: row.name.clone().unwrap_or_else(|| d.name.clone()),
                name: workshop.map_or(d.name, |w| w.name.clone()),
                team: d.team,
                role: d.role,
                name_code: workshop.is_none().then_some(d.code).flatten(),
                name_params: if workshop.is_none() {
                    d.params
                } else {
                    Params::new()
                },
                temporary,
                workshop,
            });
        }
    }
    // Bus sessions are conversations too: a separately launched session gets
    // a note directly. The lead sessions are "Claude · lead".
    let team_leads: BTreeSet<&str> = inp.team.leads.iter().map(|l| l.session.as_str()).collect();
    let session_rows: Vec<(AgentRow, &BusSession, bool)> = inp
        .sessions
        .iter()
        .filter(|s| {
            s.main
                && (inp.keep_ended || s.liveness != Liveness::Ended)
                && now.saturating_sub(s.last_ms) <= inp.horizon_ms
                && !naming::is_temp(&s.cwd, inp.temp_dirs)
                // A lead is always a conversation, and so is a session that
                // is running now, wherever it is; a past one only when it
                // worked in a repository.
                && (s.repo.is_some()
                    || s.liveness == Liveness::Live
                    || team_leads.contains(s.session.as_str()))
        })
        .map(|s| {
            let lead = team_leads.contains(s.session.as_str());
            (session_row(s, lead), s, lead)
        })
        .collect();
    for (row, s, lead) in &session_rows {
        let project = naming::project_of(s.repo.as_deref().or(Some(s.cwd.as_str())));
        let d = if *lead {
            naming::Display {
                name: s
                    .callsign
                    .clone()
                    .unwrap_or_else(|| LEAD_AUTHOR.to_string()),
                team: Some("Claude".into()),
                role: Some(LEAD_ROLE.into()),
                ..naming::Display::default()
            }
        } else if let Some(callsign) = &s.callsign {
            naming::Display {
                name: callsign.clone(),
                team: (!project.is_empty()).then(|| project.to_string()),
                role: None,
                ..naming::Display::default()
            }
        } else {
            naming::display(s.name.as_deref(), &project, Some(s.first_ms))
        };
        // A session goes by its callsign, unless someone chose it a name.
        let workshop = inp.names.get(&row.id).filter(|w| {
            !*lead && (s.callsign.is_none() || matches!(w.source.as_str(), "renamed" | "picked"))
        });
        agents_ctx.push(AgentCtx {
            row,
            lead: s.session.as_str(),
            lead_cwd: None,
            lead_alive: s.liveness == Liveness::Live,
            raw: d.name.clone(),
            name: workshop.map_or(d.name, |w| w.name.clone()),
            team: d.team,
            role: d.role,
            name_code: workshop.is_none().then_some(d.code).flatten(),
            name_params: if workshop.is_none() {
                d.params
            } else {
                Params::new()
            },
            temporary: false,
            workshop,
        });
    }

    // The lead sessions: what "the coordinator" means.
    let lead_sessions: BTreeSet<&str> = team_leads.iter().copied().collect();
    let session_name = |session: &str| -> String {
        agents_ctx
            .iter()
            .find(|a| a.row.kind == "session" && a.lead == session)
            .map(|a| a.name.clone())
            .unwrap_or_else(|| {
                if lead_sessions.contains(session) {
                    LEAD_AUTHOR.to_string()
                } else {
                    format!("session {}", session.chars().take(8).collect::<String>())
                }
            })
    };
    let unanswered_ms = cfg.unanswered_min.max(1) * 60_000;
    let blocked_unreachable_ms = cfg.blocked_unreachable_min.max(1) * 60_000;

    // Channels from checkouts: where each agent wrote, ran git or was given
    // a worktree, placed on disk; live bus sessions; sluice deliveries.
    let mut repo_cache: HashMap<String, Option<CheckoutInfo>> = HashMap::new();
    let mut resolve_any = |path: &str| -> Option<CheckoutInfo> {
        repo_cache
            .entry(path.to_string())
            .or_insert_with(|| (inp.resolve)(path))
            .clone()
    };
    // Every channel is made of a repository inside the workspace this read
    // is for ([`Inputs::in_workspace`]).
    let in_workspace = |info: &CheckoutInfo| (inp.in_workspace)(&info.repo);
    let mut repos: BTreeMap<String, RepoAcc> = BTreeMap::new();
    let mut agent_channels: HashMap<String, Vec<String>> = HashMap::new();
    // The repositories an agent wrote in (or a session works in): where its
    // conversation belongs. Running git to read one does not make it so.
    let mut agent_wrote: HashMap<String, Vec<String>> = HashMap::new();
    let mut agent_checkout: HashMap<String, (String, Option<String>)> = HashMap::new();
    // (checkout, writer key, first, last): the input of two-writer detection.
    let mut spans: Spans = BTreeMap::new();
    for a in agents_ctx.iter().filter(|a| !a.temporary) {
        let key = a.row.id.as_str();
        let seen = a.row.last_action_ms.unwrap_or(a.row.seen_ms);
        // Unfolded write directories when the projection has them; the
        // folded checkout roots otherwise.
        let writes: &[team::CheckoutUse] = if a.row.write_dirs.is_empty() {
            &a.row.checkouts
        } else {
            &a.row.write_dirs
        };
        let mut places: Vec<(String, u64, bool)> = Vec::new();
        for c in writes {
            places.push((c.path.clone(), c.last_ms, true));
        }
        let git: &[String] = if a.row.git_dirs.is_empty() {
            &a.row.git_roots
        } else {
            &a.row.git_dirs
        };
        for r in git {
            places.push((r.clone(), seen, false));
        }
        if let Some(w) = &a.row.worktree {
            places.push((w.clone(), seen, false));
        }
        let mut mine: Vec<String> = Vec::new();
        let mut wrote_in: Vec<String> = Vec::new();
        let mut newest_write: Option<(u64, String, Option<String>)> = None;
        let mut first_place: Option<(String, Option<String>)> = None;
        let folded: Vec<&str> = a
            .row
            .checkouts
            .iter()
            .map(|c| c.path.as_str())
            .chain(a.row.git_roots.iter().map(String::as_str))
            .collect();
        for (i, (path, ms, wrote)) in places.iter().enumerate() {
            // Placed on a known root, else by the team view's own lexical
            // root and the naming convention; never by looking at the disk
            // somewhere harbard was not told about.
            // A path that resolves to a repository of another workspace
            // stops there: the naming fallback must not turn it into one
            // of this workspace's.
            let placed = resolve_any(path).or_else(|| {
                folded_root(path, folded.iter().copied())
                    .map(|root| resolve_any(root).unwrap_or_else(|| by_name(root)))
            });
            // A worktree the harness gave this agent is a checkout of its
            // lead's repository. The transcript says so, which holds on a
            // volume serve must not read and after the worktree is removed.
            let info = match placed {
                Some(info) if info.real => Some(info),
                other => folded_root(path, a.row.worktree.iter().map(String::as_str))
                    .zip(a.lead_cwd.and_then(&mut resolve_any).filter(|l| l.real))
                    .map(|(worktree, lead)| CheckoutInfo {
                        checkout: worktree.to_string(),
                        repo: lead.repo,
                        branch: None,
                        real: true,
                    })
                    .or(other),
            };
            let Some(info) = info.filter(in_workspace) else {
                continue;
            };
            let acc = repos.entry(info.repo.clone()).or_default();
            acc.checkouts
                .insert(info.checkout.clone(), info.branch.clone());
            acc.members.insert(key.to_string());
            acc.real |= info.real;
            // A git read is membership, not activity: it neither keeps the
            // channel out of "More channels" nor lifts it in the list (an agent
            // that once ran `git -C` in a repository is seen every minute).
            if *wrote || a.row.kind == "session" {
                acc.last_ms = acc.last_ms.max(*ms);
                acc.active_ms = acc.active_ms.max(*ms);
                let worked = acc.workers.entry(key.to_string()).or_default();
                *worked = (*worked).max(*ms);
                if !wrote_in.contains(&info.repo) {
                    wrote_in.push(info.repo.clone());
                }
            }
            if !mine.contains(&info.repo) {
                mine.push(info.repo.clone());
            }
            if *wrote {
                let c = &writes[i];
                add_span(&mut spans, &info.checkout, key, c.first_ms, c.last_ms);
                if newest_write.as_ref().is_none_or(|(t, _, _)| c.last_ms > *t) {
                    newest_write = Some((c.last_ms, info.checkout.clone(), info.branch.clone()));
                }
            }
            if first_place.is_none() {
                first_place = Some((info.checkout.clone(), info.branch.clone()));
            }
        }
        if let Some(place) = newest_write.map(|(_, c, b)| (c, b)).or(first_place) {
            agent_checkout.insert(key.to_string(), place);
        }
        agent_channels.insert(key.to_string(), mine);
        agent_wrote.insert(key.to_string(), wrote_in);
    }
    // The lead writing in a checkout beside its own agents is a writer too.
    for lead in &inp.team.leads {
        let key = format!("lead:{}", lead.session);
        for c in &lead.lead_write_dirs {
            if let Some(info) = resolve_any(&c.path).filter(in_workspace) {
                add_span(&mut spans, &info.checkout, &key, c.first_ms, c.last_ms);
            }
        }
    }
    // Sessions working in each repository recently: who a channel note can
    // be stored for besides the members' leads.
    let mut session_repos: Vec<(String, String)> = Vec::new();
    for s in inp.sessions {
        let recent = now.saturating_sub(s.last_ms) <= ACTIVE_MS;
        if s.liveness != Liveness::Live && !recent {
            continue;
        }
        let Some(path) = s.repo.as_deref().filter(|r| r.starts_with('/')) else {
            continue;
        };
        let Some(info) = resolve_any(path).filter(|i| i.real && in_workspace(i)) else {
            continue;
        };
        if naming::is_temp(&s.cwd, inp.temp_dirs) {
            continue;
        }
        session_repos.push((s.session.clone(), info.repo.clone()));
        // Only a session live right now makes a checkout a channel by itself.
        if s.liveness != Liveness::Live {
            continue;
        }
        let acc = repos.entry(info.repo.clone()).or_default();
        acc.checkouts
            .insert(info.checkout.clone(), info.branch.clone());
        acc.last_ms = acc.last_ms.max(s.last_ms);
        acc.active_ms = acc.active_ms.max(s.last_ms);
        acc.real = true;
        let worked = acc
            .workers
            .entry(format!("session:{}", s.session))
            .or_default();
        *worked = (*worked).max(s.last_ms);
    }
    // Every repository the bus has seen is a channel for good, whoever worked
    // there and whether or not they still do. Under the same `real` rule as
    // everything else; silence only moves it to "More channels".
    for known in inp.known_repos {
        if naming::is_temp(&known.repo, inp.temp_dirs) {
            continue;
        }
        let Some(info) = resolve_any(&known.repo).filter(|i| i.real && in_workspace(i)) else {
            continue;
        };
        let acc = repos.entry(info.repo.clone()).or_default();
        acc.checkouts
            .entry(info.checkout.clone())
            .or_insert(info.branch.clone());
        acc.last_ms = acc.last_ms.max(known.last_ms);
        acc.active_ms = acc.active_ms.max(known.last_ms);
        acc.real = true;
    }
    // A channel is a real repository: a lexical guess alone (a folder named
    // like a project, a path under a protected folder) never makes one.
    repos.retain(|_, acc| acc.real);
    for list in agent_channels.values_mut() {
        list.retain(|r| repos.contains_key(r));
    }
    // Where an agent's messages show: the channels it wrote in; for an agent
    // that wrote in none (a reviewer reading with git), the ones it read
    // that have real activity of their own. Never a repository it only
    // glanced at: that made a quiet repository look busy, with unread.
    let message_channels: HashMap<String, Vec<String>> = agent_channels
        .iter()
        .map(|(key, read)| {
            let wrote: Vec<String> = agent_wrote
                .get(key)
                .map(|w| {
                    w.iter()
                        .filter(|r| repos.contains_key(*r))
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            let list = if wrote.is_empty() {
                read.iter()
                    .filter(|r| repos.get(*r).is_some_and(|acc| acc.active_ms > 0))
                    .cloned()
                    .collect()
            } else {
                wrote
            };
            (key.clone(), list)
        })
        .collect();
    let mut repo_of = |path: &str| resolve_any(path).map_or_else(|| path.to_string(), |i| i.repo);
    let delivery_repo: HashMap<&str, String> = inp
        .deliveries
        .iter()
        .map(|d| (d.id.as_str(), repo_of(&d.repo)))
        .collect();
    let receipt_repo: Vec<String> = inp.receipts.iter().map(|r| repo_of(&r.repo)).collect();

    // Channel ids: repository names, made unique.
    let mut channel_id: HashMap<String, String> = HashMap::new();
    // A room's slug is its address on the wire and cannot change; a project
    // channel's id can. So a room keeps its name, and a repository of the
    // same name takes the suffix.
    let room_slugs: BTreeSet<&str> = inp.rooms.iter().map(|r| r.slug.as_str()).collect();
    let mut taken: BTreeSet<String> = room_slugs.iter().map(|s| s.to_string()).collect();
    for repo in repos
        .keys()
        .chain(delivery_repo.values())
        .chain(receipt_repo.iter())
    {
        if channel_id.contains_key(repo) {
            continue;
        }
        let base = repo_name(repo);
        let mut id = base.clone();
        let mut n = 2;
        while taken.contains(&id) {
            id = format!("{base}-{n}");
            n += 1;
        }
        taken.insert(id.clone());
        channel_id.insert(repo.clone(), id);
    }
    let chan = |repo: &str| channel_id.get(repo).cloned();

    // Verified commits: landed receipts, keyed by channel.
    let landed: Vec<(&Receipt, Option<String>)> = inp
        .receipts
        .iter()
        .zip(receipt_repo.iter())
        .filter(|(r, _)| r.verdict == Verdict::Landed)
        .map(|(r, repo)| (r, chan(repo)))
        .collect();
    // Who offered each delivery: only the agent's own lead session counts.
    let delivered_by: HashMap<&str, &str> = inp
        .deliveries
        .iter()
        .map(|d| (d.id.as_str(), d.session.as_str()))
        .collect();
    // Evidence is a receipt that landed this commit after the DONE was sent,
    // for a delivery this agent's lead made, in the same repository. A SHA
    // merely named (a base, someone else's landing) is never evidence.
    let verify = |shas: &[String], channels: &[String], ts: u64, lead: &str| -> Option<Verified> {
        for sha in shas {
            for (r, ch) in &landed {
                if r.decided_ms < ts || delivered_by.get(r.delivery_id.as_str()) != Some(&lead) {
                    continue;
                }
                let hit = r.delivered_sha.starts_with(sha.as_str())
                    || r.candidate
                        .as_deref()
                        .is_some_and(|c| c.starts_with(sha.as_str()));
                let same_repo = ch.as_ref().is_some_and(|c| channels.contains(c));
                if hit && same_repo {
                    return Some(Verified {
                        sha: sha.clone(),
                        branch: r.branch.clone(),
                        decided_ms: r.decided_ms,
                    });
                }
            }
        }
        None
    };

    // Messages, per agent conversation.
    let names_by_lead: HashMap<&str, Vec<(&str, &str)>> = {
        let mut m: HashMap<&str, Vec<(&str, &str)>> = HashMap::new();
        for a in &agents_ctx {
            m.entry(a.lead)
                .or_default()
                .push((a.raw.as_str(), a.row.id.as_str()));
        }
        m
    };
    // The repository a lead's agents wrote in most, when one leads: the
    // team of a teammate that wrote nowhere (a reviewer), rather than the
    // name of the folder the lead runs in.
    let teammates_repo: HashMap<&str, String> = {
        let mut counts: HashMap<&str, BTreeMap<String, usize>> = HashMap::new();
        for a in agents_ctx.iter().filter(|a| a.row.kind != "session") {
            if let Some(repo) = agent_checkout
                .get(&a.row.id)
                .and_then(|(c, _)| chan(&repo_of(c)))
            {
                *counts.entry(a.lead).or_default().entry(repo).or_default() += 1;
            }
        }
        counts
            .into_iter()
            .filter_map(|(lead, repos)| {
                let top = repos.values().copied().max()?;
                let mut leaders = repos.into_iter().filter(|(_, n)| *n == top);
                let (repo, _) = leaders.next()?;
                leaders.next().is_none().then_some((lead, repo))
            })
            .collect()
    };
    let mut messages: Vec<Message> = Vec::new();
    let mut agents_out: Vec<Agent> = Vec::new();
    let mut cards: Vec<Card> = Vec::new();
    let mut todo_unread = 0usize;
    // What an `@` can name: agents by handle, teams, the channel, the operator.
    let operator_handle = inp
        .operator
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_lowercase();
    let mut handles: BTreeSet<String> = agents_ctx.iter().map(handle_of).collect();
    handles.extend(
        agents_ctx
            .iter()
            .filter_map(|a| a.team.as_ref().map(|t| t.to_lowercase())),
    );
    handles.extend(naming::CHANNEL_MENTIONS.iter().map(|s| s.to_string()));
    handles.insert(operator_handle.clone());
    handles.insert(naming::OPERATOR_MENTION.to_string());
    let names_operator = |mentions: &[String]| {
        mentions
            .iter()
            .any(|m| *m == operator_handle || m == naming::OPERATOR_MENTION)
    };
    for a in &agents_ctx {
        let key = a.row.id.clone();
        // Membership (every repository it ran in), and where its messages show.
        let member_of: Vec<String> = agent_channels
            .get(&key)
            .map(|repos| repos.iter().filter_map(|r| chan(r)).collect())
            .unwrap_or_default();
        let channels: Vec<String> = message_channels
            .get(&key)
            .map(|repos| repos.iter().filter_map(|r| chan(r)).collect())
            .unwrap_or_default();
        let siblings = names_by_lead.get(a.lead).cloned().unwrap_or_default();
        let mut conv: Vec<Message> = Vec::new();
        let mut first_by_digest: HashMap<&str, String> = HashMap::new();
        for (i, m) in a.row.messages.iter().enumerate() {
            // A message from a sibling teammate is already that sibling's
            // own outgoing message; show it once, there.
            if m.dir == "in" && siblings.iter().any(|(n, id)| *n == m.peer && *id != key) {
                continue;
            }
            let id = format!("{key}:{}:{}:{i}", m.ts_ms, m.dir);
            let (label, version) = label_of(m);
            let text = team::parse_label(&m.first_line)
                .map(|l| l.sentence)
                .unwrap_or_else(|| m.first_line.clone());
            let mut flags = Vec::new();
            let mut jargon = Vec::new();
            let mut repeat_of = None;
            if m.dir == "out" && m.structured.is_none() {
                let under_rule = m.ts_ms >= cfg.protocol_since_ms;
                if under_rule && label.is_none() {
                    flags.push("unlabeled");
                }
                if under_rule && m.chars > LONG_MESSAGE_CHARS {
                    flags.push("too_long");
                }
                if under_rule {
                    jargon = human::jargon(&text);
                    if !jargon.is_empty() {
                        flags.push("jargon");
                    }
                }
                if m.repeat {
                    flags.push("repeat");
                    repeat_of = first_by_digest.get(m.digest.as_str()).cloned();
                }
                if !m.digest.is_empty() {
                    first_by_digest
                        .entry(m.digest.as_str())
                        .or_insert(id.clone());
                }
            }
            let (from, author) = if m.dir == "out" {
                ("agent", a.name.clone())
            } else if is_lead_peer(&m.peer) {
                ("lead", session_name(a.lead))
            } else {
                ("peer", m.peer.clone())
            };
            let verified = if m.dir == "out" && label == Some("DONE") {
                verify(&m.shas, &channels, m.ts_ms, a.lead)
            } else {
                None
            };
            let (files, attachments) = files_and_attachments(m, inp.attachment);
            let readable = crate::readable::readable(&text, label);
            let shape = crate::readable::shape(&text, label);
            conv.push(Message {
                id,
                agent: key.clone(),
                from,
                author,
                ts_ms: m.ts_ms,
                text,
                body: m.body.clone(),
                label,
                version,
                flags,
                jargon,
                readable,
                shape,
                repeat_of,
                chars: m.chars,
                files,
                verified,
                channels: channels.clone(),
                reply_to: None,
                replies: Vec::new(),
                structured: m.structured.clone(),
                undelivered: m.undelivered,
                attachments,
                delivery: None,
                mentions: mentions_in(
                    &format!("{} {}", m.first_line, m.body.as_deref().unwrap_or("")),
                    &handles,
                ),
            });
        }
        // Operator notes addressed to this agent through its lead.
        let names: Vec<&str> = if a.raw == a.name {
            vec![a.name.as_str()]
        } else {
            vec![a.name.as_str(), a.raw.as_str()]
        };
        let mut bus_replies: Vec<(String, &BusReply)> = Vec::new();
        for n in inp.notes.iter().filter(|n| n.session == a.lead) {
            if channel_note_body(&n.text).is_some() {
                continue;
            }
            let Some(body) = dm_body(&n.text, &key, &names) else {
                continue;
            };
            // A longer sibling name that shares this prefix owns the note.
            if siblings.iter().any(|(other, id)| {
                *id != key
                    && names.iter().all(|n| other.len() > n.len())
                    && agent_note_body(&n.text, other).is_some()
            }) {
                continue;
            }
            let body = without_fields(body);
            let body = body.as_str();
            let id = format!("note:{}", n.id);
            bus_replies.extend(n.trail.replies.iter().map(|r| (id.clone(), r)));
            conv.push(Message {
                id,
                agent: key.clone(),
                from: "operator",
                author: inp.operator.to_string(),
                ts_ms: n.ts_ms,
                text: human::whole_text(body),
                body: None,
                label: None,
                version: None,
                flags: Vec::new(),
                jargon: Vec::new(),
                readable: None,
                shape: None,
                repeat_of: None,
                chars: body.chars().count(),
                files: file_paths(body),
                verified: None,
                channels: channels.clone(),
                reply_to: inp.desk.threads.get(&n.id).cloned(),
                replies: Vec::new(),
                structured: None,
                undelivered: false,
                attachments: operator_attachments(body, inp.attachment),
                delivery: Some(NoteDelivery {
                    answered_by: (a.row.kind != "session")
                        .then(|| answered_instead(&n.trail.replies, &names, &session_name))
                        .flatten(),
                    ..note_delivery(&[n], &lead_sessions)
                }),
                mentions: mentions_in(body, &handles),
            });
        }
        // What this session said to the operator on the bus, unprompted.
        if a.row.kind == "session" {
            for b in inp.bus_notes.iter().filter(|b| b.session == a.lead) {
                conv.push(bus_message(
                    b,
                    &key,
                    &channels,
                    &session_name,
                    &lead_sessions,
                    &handles,
                    inp.attachment,
                ));
            }
        }
        conv.sort_by(|x, y| x.ts_ms.cmp(&y.ts_ms).then(x.id.cmp(&y.id)));

        // Replies: what the lead or the operator sent after an agent message,
        // until the agent spoke again, unless a thread was named explicitly.
        let mut last_top: Option<usize> = None;
        let mut links: Vec<(usize, usize)> = Vec::new();
        // Hung under the agent's last message by position alone, not by a
        // ref: a thread for reading, never an answer to a BLOCKED (AC3).
        let mut by_position: HashSet<String> = HashSet::new();
        for (i, m) in conv.iter().enumerate() {
            if m.from == "agent" {
                last_top = Some(i);
            } else if let Some(target) = &m.reply_to {
                if let Some(t) = conv.iter().position(|x| &x.id == target) {
                    links.push((t, i));
                }
            } else if let Some(t) = last_top {
                links.push((t, i));
                by_position.insert(m.id.clone());
            }
        }
        for (t, i) in links {
            let child = conv[i].id.clone();
            let parent = conv[t].id.clone();
            conv[t].replies.push(child);
            conv[i].reply_to = Some(parent);
        }
        // Bus answers to the operator's notes, in the note's thread: under
        // the message the note answered, or under the note itself.
        for (note, r) in bus_replies {
            let Some(at) = conv.iter().position(|m| m.id == note) else {
                continue;
            };
            let parent = conv[at].reply_to.clone().unwrap_or(note);
            let reply = bus_reply_message(
                r,
                &parent,
                &key,
                &channels,
                &lead_sessions,
                &session_name,
                inp.attachment,
            );
            if let Some(p) = conv.iter_mut().find(|m| m.id == parent) {
                p.replies.push(reply.id.clone());
            }
            conv.push(reply);
        }
        conv.sort_by(|x, y| x.ts_ms.cmp(&y.ts_ms).then(x.id.cmp(&y.id)));

        // The operator's notes nobody answered: no bus reply and no word
        // from the agent since.
        for (i, m) in conv.iter().enumerate() {
            if m.from != "operator" {
                continue;
            }
            let replied_on_bus = m.delivery.as_ref().is_some_and(|d| d.state == "answered");
            let agent_spoke = conv[i + 1..]
                .iter()
                .any(|x| x.from == "agent" && x.ts_ms > m.ts_ms);
            if a.temporary {
                continue;
            }
            if let Some(c) = unanswered_card(
                m,
                replied_on_bus || agent_spoke,
                now,
                unanswered_ms,
                inp.desk,
            ) {
                cards.push(c);
            }
        }
        // An agent that writes `@operator` is asking the operator something.
        if !a.temporary {
            for (i, m) in conv.iter().enumerate() {
                if m.from != "agent" || !names_operator(&m.mentions) {
                    continue;
                }
                // Answered: the operator or the lead spoke after it.
                if conv[i + 1..]
                    .iter()
                    .any(|x| matches!(x.from, "lead" | "operator"))
                {
                    continue;
                }
                let mut c = card(
                    format!("mention:{}", m.id),
                    "mention",
                    channels.first().cloned(),
                    vec![key.clone()],
                    Some(m.id.clone()),
                    None,
                    m.ts_ms,
                    inp.desk,
                );
                c.needs = now.saturating_sub(m.ts_ms) <= ACTIVE_MS;
                cards.push(c);
            }
        }

        // Its lead stopped it with TaskStop: "Closed", and nothing it said
        // before asks the operator for anything any more.
        let closed = a.row.state == "stopped" && a.row.state_reason == "task_stop";
        // Presence and dormancy. An agent counts as working, waiting or
        // dormant only while its lead is alive or it wrote recently itself;
        // otherwise its process is most likely gone and nothing is known.
        let recent_self = a
            .row
            .last_action_ms
            .is_some_and(|ms| now.saturating_sub(ms) <= agent_recent_ms);
        let within_day = a
            .row
            .last_action_ms
            .is_some_and(|ms| now.saturating_sub(ms) <= agent_max_quiet_ms);
        let alive = recent_self || (a.lead_alive && within_day);
        let lost = matches!(a.row.state, "running" | "idle") && !alive;
        let presence = match a.row.state {
            _ if lost => "finished",
            "running" => "working",
            "idle" => "waiting",
            _ => "finished",
        };
        // BLOCKED, open until it is answered: a later word from the lead
        // (its one-to-one conversation with the agent), a note of the operator's
        // sent as a reply to it, or the agent speaking again. A note of theirs
        // that only follows it in Direct is not an answer (AC3).
        // Nobody reads the BLOCKED of an agent that is lost or whose lead is
        // off the bus: it leaves Needs you after
        // `blocked_unreachable_min` and stays as history (AC6).
        let unreachable = lost || matches!(delivery_of(a.lead), "ended" | "not_on_bus");
        let mut blocked_open: Option<(u64, String)> = None;
        for (i, m) in conv.iter().enumerate() {
            if m.from != "agent" || m.label != Some("BLOCKED") {
                continue;
            }
            let answered = conv[i + 1..].iter().any(|x| match x.from {
                "agent" | "lead" => true,
                "operator" => {
                    x.reply_to.as_deref() == Some(m.id.as_str()) && !by_position.contains(&x.id)
                }
                _ => false,
            });
            if answered || a.temporary || closed {
                continue;
            }
            let id = format!("blocked:{}", m.id);
            let mut c = card(
                id.clone(),
                "blocked",
                channels.first().cloned(),
                vec![key.clone()],
                Some(m.id.clone()),
                None,
                m.ts_ms,
                inp.desk,
            );
            if unreachable && now.saturating_sub(m.ts_ms) > blocked_unreachable_ms {
                c.needs = false;
            }
            if c.needs && !inp.desk.resolved.contains_key(&id) {
                blocked_open = Some((m.ts_ms, m.text.clone()));
            }
            cards.push(c);
        }

        // A turn that ended saying it waits for a background job is the
        // straggler case: it goes dormant on the same clock as a quiet turn.
        let waits = a.row.activity.state == "waiting";
        // A call in flight writes nothing until it returns: it is quiet, not
        // asleep, so its dot (like its face) turns only at the red threshold.
        let in_flight = a.row.state == "running" && in_flight(a.row.activity.state);
        let dormant = match (a.row.state, a.row.last_action_ms) {
            _ if lost || a.temporary => None,
            ("running", Some(last)) | ("idle" | "done", Some(last))
                if a.row.state == "running" || waits =>
            {
                let quiet = now.saturating_sub(last);
                if quiet > red_ms {
                    Some("red")
                } else if quiet > amber_ms && !in_flight {
                    Some("amber")
                } else {
                    None
                }
            }
            _ => None,
        };
        if dormant == Some("red")
            && let Some(last) = a.row.last_action_ms
        {
            cards.push(card(
                format!("dormant:{key}:{last}"),
                "dormant",
                channels.first().cloned(),
                vec![key.clone()],
                None,
                None,
                last,
                inp.desk,
            ));
        }

        let read_to = inp
            .desk
            .reads
            .get(&key)
            .copied()
            .unwrap_or(0)
            .max(inp.desk.baseline_ms)
            .max(inp.desk.read_all_ms);
        let unread = conv
            .iter()
            .filter(|m| m.ts_ms > read_to && is_incoming(m))
            .count();
        if !a.temporary {
            todo_unread += conv
                .iter()
                .filter(|m| m.from == "agent" && m.label.is_some() && m.ts_ms > read_to)
                .count();
        }
        let done: Vec<&Message> = conv
            .iter()
            .filter(|m| m.from == "agent" && m.label == Some("DONE"))
            .collect();
        let mut reports: Vec<String> = Vec::new();
        for m in conv.iter().filter(|m| m.from == "agent") {
            for f in &m.files {
                if !reports.contains(f) {
                    reports.push(f.clone());
                }
            }
        }
        let build_dir = conv
            .iter()
            .filter(|m| m.from == "agent" && m.label == Some("START"))
            .find_map(|m| declared_build_dir(&m.text));
        let (checkout, branch) = agent_checkout
            .get(&key)
            .cloned()
            .map_or((None, None), |(p, b)| (Some(p), b));
        let activity = face(
            &a.row.activity,
            closed,
            lost,
            dormant,
            blocked_open.as_ref(),
            done.last().and_then(|m| m.verified.as_ref()),
            a.row.last_action_ms,
        );
        let reactions = reactions_of(&conv, a.row.first_ms, delivery_of(a.lead) == "ready", now);
        // The team, in order: said outright (`harbard name --team`); the
        // suffix of its own name or of a one-word description
        // (`robots-builder`); the repository it wrote in; the one its
        // lead's other agents wrote in most; the lead's project. Never a
        // dot-folder: a bare workshop name ("Owen") under a lead in
        // `~/.claude` read ".claude · Owen".
        let explicit = a
            .workshop
            .filter(|w| w.team_explicit)
            .and_then(|w| w.team.clone());
        let named = a
            .row
            .name
            .as_deref()
            .and_then(naming::team_role)
            .or_else(|| {
                a.row
                    .description
                    .as_deref()
                    .filter(|d| !d.trim().contains(char::is_whitespace))
                    .and_then(naming::team_role)
            })
            .map(|(t, _)| t);
        let written = (a.row.kind != "session")
            .then(|| checkout.as_deref().and_then(|c| chan(&repo_of(c))))
            .flatten();
        let teammates = (a.row.kind != "session")
            .then(|| teammates_repo.get(a.lead).cloned())
            .flatten();
        let team = explicit
            .or(named)
            .or(written)
            .or(teammates)
            .or_else(|| a.team.clone())
            .filter(|t| !t.starts_with('.'));
        let role_line_coded = a.workshop.and_then(|w| {
            crate::workshop::role_line_coded(
                w.role.as_deref().or(a.role.as_deref()),
                team.as_deref()
                    .or(w.team.as_deref().filter(|t| !t.starts_with('.'))),
                w.gender,
            )
        });
        agents_out.push(Agent {
            key: key.clone(),
            name: a.name.clone(),
            name_code: a.name_code,
            name_params: a.name_params.clone(),
            label: id_label(a.row),
            lost,
            kind: a.row.kind,
            lead_session: a.lead.to_string(),
            lead_quiet_ms: lead_quiet(a.lead),
            delivery: delivery_of(a.lead),
            state: a.row.state,
            state_reason: a.row.state_reason,
            presence,
            dormant,
            last_action_ms: a.row.last_action_ms,
            first_ms: a.row.first_ms,
            brief: a.row.brief.clone(),
            description: a.row.description.clone(),
            checkout,
            branch,
            build_dir,
            channels: member_of.clone(),
            deliveries: done.len(),
            verified_deliveries: done.iter().filter(|m| m.verified.is_some()).count(),
            reports,
            unread,
            last_message: conv
                .iter()
                .rev()
                .find(|m| m.structured.is_none())
                .map(|m| m.id.clone()),
            can_stop: false,
            activity,
            reactions,
            progress: a.row.progress.clone(),
            role: a.role.clone(),
            handle: handle_of(a),
            temporary: a.temporary,
            role_line: a.workshop.and_then(|w| {
                crate::workshop::role_line(
                    w.role.as_deref().or(a.role.as_deref()),
                    team.as_deref()
                        .or(w.team.as_deref().filter(|t| !t.starts_with('.'))),
                )
            }),
            role_line_code: role_line_coded.as_ref().map(|(code, _)| *code),
            role_line_params: role_line_coded
                .map(|(_, values)| values)
                .unwrap_or_default(),
            team,
            name_source: a.workshop.map(|w| w.source.clone()),
            name_reason: a.workshop.and_then(|w| w.reason.clone()),
            own_name: a.row.name.clone(),
            closed,
            hidden: inp.desk.hidden_agents.contains_key(&key),
        });
        messages.extend(conv);
    }

    // Operator notes written in a channel: one per lead reached, shown once,
    // with what became of every copy.
    // (channel, text, 2-second bucket) to every copy of the note.
    type Copies<'n> = Vec<((String, String, u64), Vec<&'n OperatorNote>)>;
    let mut channel_notes: Copies<'_> = Vec::new();
    for n in inp.notes {
        let Some(rest) = channel_note_body(&n.text) else {
            continue;
        };
        let Some((ch, body)) = rest.split_once(": ") else {
            continue;
        };
        // In a room the note itself is the message; a copy to a session named
        // with `@` is only how it reached them.
        if !taken.contains(ch) || room_slugs.contains(ch) {
            continue;
        }
        let key = (ch.to_string(), body.to_string(), n.ts_ms / 2_000);
        match channel_notes.iter_mut().find(|(k, _)| *k == key) {
            Some((_, copies)) => copies.push(n),
            None => channel_notes.push((key, vec![n])),
        }
    }
    for ((ch, body, _), copies) in &channel_notes {
        let n = copies[0];
        let body = without_fields(body);
        let id = format!("note:{}", n.id);
        let channels = vec![ch.clone()];
        let delivery = note_delivery(copies, &lead_sessions);
        let mut note = Message {
            id: id.clone(),
            agent: String::new(),
            from: "operator",
            author: inp.operator.to_string(),
            ts_ms: n.ts_ms,
            text: human::whole_text(&body),
            body: None,
            label: None,
            version: None,
            flags: Vec::new(),
            jargon: Vec::new(),
            readable: None,
            shape: None,
            repeat_of: None,
            chars: body.chars().count(),
            files: file_paths(&body),
            verified: None,
            channels: channels.clone(),
            reply_to: None,
            replies: Vec::new(),
            structured: None,
            undelivered: false,
            attachments: operator_attachments(&body, inp.attachment),
            delivery: Some(delivery),
            mentions: mentions_in(&body, &handles),
        };
        for r in copies.iter().flat_map(|c| c.trail.replies.iter()) {
            let reply = bus_reply_message(
                r,
                &id,
                "",
                &channels,
                &lead_sessions,
                &session_name,
                inp.attachment,
            );
            note.replies.push(reply.id.clone());
            messages.push(reply);
        }
        let replied = note
            .delivery
            .as_ref()
            .is_some_and(|d| d.state == "answered");
        if let Some(c) = unanswered_card(&note, replied, now, unanswered_ms, inp.desk) {
            cards.push(c);
        }
        messages.push(note);
    }
    // Bus notes for the operator from sessions with no conversation here
    // (a herd child, a session that is not a lead): in their repository's
    // channel. A session with one already shows them there.
    for b in inp.bus_notes {
        if agents_ctx
            .iter()
            .any(|a| a.row.kind == "session" && a.lead == b.session)
        {
            continue;
        }
        let Some(ch) = b.repo.as_deref().and_then(|r| chan(&repo_of(r))) else {
            continue;
        };
        messages.push(bus_message(
            b,
            "",
            &[ch],
            &session_name,
            &lead_sessions,
            &handles,
            inp.attachment,
        ));
    }
    // What was said in rooms: the newest of each, kept whole and in order.
    let mut shown_per_room: HashMap<&str, usize> = HashMap::new();
    let mut room_notes: Vec<&RoomNote> = inp
        .room_notes
        .iter()
        .rev()
        .filter(|n| room_slugs.contains(n.room.as_str()))
        .filter(|n| {
            let shown = shown_per_room.entry(n.room.as_str()).or_default();
            *shown += 1;
            *shown <= ROOM_MESSAGES_SHOWN
        })
        .collect();
    room_notes.reverse();
    let room_start = messages.len();
    for n in &room_notes {
        messages.push(room_message(
            n,
            inp.operator,
            &handles,
            &lead_sessions,
            inp.attachment,
        ));
    }
    // A reply stays beside its parent; the parent lists it.
    let room_index: HashMap<String, usize> = messages[room_start..]
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id.clone(), room_start + i))
        .collect();
    for i in room_start..messages.len() {
        let Some(parent) = messages[i].reply_to.clone() else {
            continue;
        };
        if let Some(&at) = room_index.get(&parent) {
            let child = messages[i].id.clone();
            messages[at].replies.push(child);
        }
    }
    // "Was it you?": a rule approved, the context changed, the memory sink
    // switched on. In Needs you until they keep it (resolve) or revokes it.
    for check in &inp.desk.checks {
        let mut c = card(
            format!("check:{}", check.id),
            "check",
            None,
            Vec::new(),
            None,
            None,
            check.since_ms,
            inp.desk,
        );
        c.check = Some(CardCheck {
            what: check.what.clone(),
            text: check.text.clone(),
        });
        cards.push(c);
    }
    // A handoff to the operator (`note --kind handoff --to harness:human`)
    // hands them work: "Handover" in Needs you until they answer it in its
    // thread or resolves the card.
    for b in inp.bus_notes.iter().filter(|b| b.kind == "handoff") {
        let message = format!("bus:{}", b.id);
        let answered = inp.desk.threads.values().any(|m| *m == message);
        if answered {
            continue;
        }
        let shown = messages.iter().find(|m| m.id == message);
        let agents = shown
            .map(|m| m.agent.clone())
            .filter(|a| !a.is_empty())
            .into_iter()
            .collect();
        let channel = shown
            .and_then(|m| m.channels.first().cloned())
            .or_else(|| b.repo.as_deref().and_then(|r| chan(&repo_of(r))));
        cards.push(card(
            format!("handoff:{}", b.id),
            "handoff",
            channel,
            agents,
            Some(message),
            None,
            b.ts_ms,
            inp.desk,
        ));
    }
    // A session that writes `@operator` in any channel asks for the operator:
    // the same card a teammate's mention raises. A handoff has
    // its own card, and a scratch session never asks them for anything.
    for b in inp.bus_notes.iter().filter(|b| b.kind != "handoff") {
        let message = format!("bus:{}", b.id);
        let Some(shown) = messages
            .iter()
            .find(|m| m.id == message && names_operator(&m.mentions))
        else {
            continue;
        };
        let scratch = agents_ctx
            .iter()
            .any(|a| a.temporary && a.row.id == shown.agent);
        if scratch || inp.desk.threads.values().any(|m| *m == message) {
            continue;
        }
        let channel = shown
            .channels
            .first()
            .cloned()
            .or_else(|| b.repo.as_deref().and_then(|r| chan(&repo_of(r))));
        let agents = Some(shown.agent.clone())
            .filter(|a| !a.is_empty())
            .into_iter()
            .collect();
        let mut c = card(
            format!("mention:{message}"),
            "mention",
            channel,
            agents,
            Some(message),
            None,
            b.ts_ms,
            inp.desk,
        );
        c.needs = now.saturating_sub(b.ts_ms) <= ACTIVE_MS;
        cards.push(c);
    }
    // A bus note answering another one sits in its thread.
    let refs: HashMap<String, String> = inp
        .bus_notes
        .iter()
        .filter_map(|b| {
            b.note_ref
                .as_ref()
                .map(|r| (format!("bus:{}", b.id), format!("bus:{r}")))
        })
        .collect();
    let shown: BTreeSet<String> = messages.iter().map(|m| m.id.clone()).collect();
    let mut threads_of: Vec<(String, String)> = Vec::new();
    for m in messages.iter_mut() {
        if let Some(parent) = refs.get(&m.id).filter(|p| shown.contains(*p)) {
            m.reply_to = Some(parent.clone());
            threads_of.push((parent.clone(), m.id.clone()));
        }
    }
    for (parent, child) in threads_of {
        if let Some(p) = messages.iter_mut().find(|m| m.id == parent) {
            p.replies.push(child);
        }
    }
    messages.sort_by(|x, y| x.ts_ms.cmp(&y.ts_ms).then(x.id.cmp(&y.id)));

    // Two writers in one checkout, per agent: overlapping write spans of any
    // two agents, across leads, or of an agent and a lead.
    // A warning (banner, Needs you) only while both writers wrote in the
    // recent window; an older overlap stays in All as history.
    let mut writer_warnings: Vec<(Option<String>, WriterWarning, bool)> = Vec::new();
    for (checkout, list) in &spans {
        let mut recent: BTreeSet<String> = BTreeSet::new();
        let mut all: BTreeSet<String> = BTreeSet::new();
        let (mut since_recent, mut since_all) = (u64::MAX, u64::MAX);
        for (i, x) in list.iter().enumerate() {
            for y in &list[i + 1..] {
                if x.0 == y.0 || !overlaps((x.1, x.2), (y.1, y.2)) {
                    continue;
                }
                let since = x.1.max(y.1);
                all.insert(x.0.clone());
                all.insert(y.0.clone());
                since_all = since_all.min(since);
                if x.2 >= writers_since && y.2 >= writers_since {
                    recent.insert(x.0.clone());
                    recent.insert(y.0.clone());
                    since_recent = since_recent.min(since);
                }
            }
        }
        let (agents, since, live) = if recent.len() >= 2 {
            (recent, since_recent, true)
        } else if all.len() >= 2 {
            (all, since_all, false)
        } else {
            continue;
        };
        let repo = repo_of(checkout);
        writer_warnings.push((
            chan(&repo),
            WriterWarning {
                checkout: checkout.clone(),
                agents: agents.into_iter().collect(),
                since_ms: since,
            },
            live,
        ));
    }
    for (ch, w, live) in &writer_warnings {
        let mut c = card(
            format!("two_writers:{}:{}", w.checkout, w.agents.join(",")),
            "two_writers",
            ch.clone(),
            w.agents.clone(),
            None,
            Some(w.checkout.clone()),
            w.since_ms,
            inp.desk,
        );
        c.needs = *live;
        cards.push(c);
    }

    // Channels, newest activity first.
    let mut channels: Vec<Channel> = repos
        .into_iter()
        .filter_map(|(repo, acc)| {
            let id = chan(&repo)?;
            let last_msg = messages
                .iter()
                .filter(|m| m.channels.contains(&id))
                .map(|m| m.ts_ms)
                .max()
                .unwrap_or(0);
            let mut readers: Vec<ChannelReader> = Vec::new();
            let leads = agents_out
                .iter()
                .filter(|a| acc.members.contains(&a.key))
                .map(|a| a.lead_session.as_str());
            let here = session_repos
                .iter()
                .filter(|(_, r)| *r == repo)
                .map(|(s, _)| s.as_str());
            for session in leads.chain(here) {
                if readers.iter().all(|r| r.session != session) {
                    readers.push(ChannelReader {
                        session: session.to_string(),
                        delivery: delivery_of(session),
                        lead: lead_sessions.contains(session),
                    });
                }
            }
            let reach = if readers.iter().any(|r| r.delivery == "ready") {
                "ready"
            } else if readers.iter().any(|r| r.delivery == "stale") {
                "stale"
            } else if readers.is_empty() {
                "nobody"
            } else {
                "none"
            };
            let read_to = inp
                .desk
                .channel_reads
                .get(&id)
                .copied()
                .unwrap_or(0)
                .max(inp.desk.baseline_ms)
                .max(inp.desk.read_all_ms);
            let unread = messages
                .iter()
                .filter(|m| {
                    m.channels.contains(&id)
                        && m.ts_ms > read_to
                        && is_incoming(m)
                        && m.structured.is_none()
                })
                .count();
            // A message counts as activity here only when it was written to
            // this channel (a channel note, a bus note); an agent's message
            // shows in every repository it touched, read-only ones too.
            // Only what was said to the channel itself: a lead's brief to a
            // teammate is that teammate's conversation, not channel talk.
            let spoken = messages
                .iter()
                .filter(|m| m.channels.len() == 1 && m.channels[0] == id && said_in_channel(m))
                .map(|m| m.ts_ms)
                .max()
                .unwrap_or(0);
            let active_ms = acc.active_ms.max(spoken);
            let mut recent: Vec<(&String, &u64)> = acc
                .workers
                .iter()
                .filter(|(_, ms)| now.saturating_sub(**ms) <= ACTIVE_MS)
                .collect();
            recent.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
            let recent: Vec<String> = recent.into_iter().map(|(k, _)| k.clone()).collect();
            Some(Channel {
                kind: "project",
                purpose: None,
                purpose_code: None,
                archived: inp.archived_repos.contains(&repo),
                built_in: false,
                recent,
                readers,
                reach,
                active_ms,
                quiet: now.saturating_sub(active_ms) > CHANNEL_QUIET_MS,
                unread,
                warnings: writer_warnings
                    .iter()
                    .filter(|(c, _, live)| *live && c.as_deref() == Some(id.as_str()))
                    .map(|(_, w, _)| w.clone())
                    .collect(),
                id,
                repo,
                checkouts: acc
                    .checkouts
                    .into_iter()
                    .map(|(path, branch)| ChannelCheckout { path, branch })
                    .collect(),
                members: acc.members.into_iter().collect(),
                last_ms: acc.last_ms.max(last_msg),
            })
        })
        .collect();
    for room in inp.rooms {
        let slug = room.slug.as_str();
        let said: Vec<&Message> = messages
            .iter()
            .filter(|m| m.channels.len() == 1 && m.channels[0] == slug && m.id.starts_with("room:"))
            .collect();
        let newest = said.iter().map(|m| m.ts_ms).max().unwrap_or(0);
        let last_ms = newest
            .max(room.last_note_ms.unwrap_or(0))
            .max(room.created_ms.unwrap_or(0));
        let mut readers: Vec<ChannelReader> = Vec::new();
        for s in inp
            .sessions
            .iter()
            .filter(|s| s.rooms.iter().any(|r| r == slug))
        {
            readers.push(ChannelReader {
                session: s.session.clone(),
                delivery: delivery_of(&s.session),
                lead: lead_sessions.contains(s.session.as_str()),
            });
        }
        let reach = if readers.iter().any(|r| r.delivery == "ready") {
            "ready"
        } else if readers.iter().any(|r| r.delivery == "stale") {
            "stale"
        } else if readers.is_empty() {
            "nobody"
        } else {
            "none"
        };
        let read_to = inp
            .desk
            .channel_reads
            .get(&room_desk_key(inp.workspace, slug))
            .copied()
            .unwrap_or(0)
            .max(inp.desk.baseline_ms)
            .max(inp.desk.read_all_ms);
        let unread = said
            .iter()
            .filter(|m| m.ts_ms > read_to && is_incoming(m))
            .filter(|m| {
                let id = m.id.strip_prefix("room:").unwrap_or(&m.id);
                room.read.as_deref().is_none_or(|mark| id > mark)
            })
            .count();
        let members = agents_out
            .iter()
            .filter(|a| a.kind == "session" && readers.iter().any(|r| r.session == a.lead_session))
            .map(|a| a.key.clone())
            .collect();
        channels.push(Channel {
            id: room.slug.clone(),
            kind: "room",
            purpose: room.purpose.clone(),
            purpose_code: room.purpose_code,
            archived: room.archived,
            built_in: room.built_in,
            repo: String::new(),
            checkouts: Vec::new(),
            members,
            last_ms,
            warnings: Vec::new(),
            readers,
            reach,
            active_ms: last_ms,
            quiet: !room.built_in && now.saturating_sub(last_ms) > CHANNEL_QUIET_MS,
            unread,
            recent: Vec::new(),
        });
    }
    channels.sort_by(|a, b| b.last_ms.cmp(&a.last_ms).then(a.id.cmp(&b.id)));

    // Deliveries, newest first; a delivery is pending until a receipt names it.
    let mut by_delivery: HashMap<&str, &Receipt> = HashMap::new();
    for r in inp.receipts {
        by_delivery
            .entry(r.delivery_id.as_str())
            .and_modify(|seen| {
                if r.decided_ms >= seen.decided_ms {
                    *seen = r;
                }
            })
            .or_insert(r);
    }
    let mut deliveries: Vec<DeliveryRow> = inp
        .deliveries
        .iter()
        .map(|d| {
            let r = by_delivery.get(d.id.as_str());
            DeliveryRow {
                id: d.id.clone(),
                channel: delivery_repo.get(d.id.as_str()).and_then(|repo| chan(repo)),
                sha: d.sha.clone(),
                branch: d.branch.clone(),
                verdict: r.map_or("pending", |r| r.verdict.as_str()),
                created_ms: d.created_ms,
                decided_ms: r.map(|r| r.decided_ms),
                session: d.session.clone(),
            }
        })
        .collect();
    deliveries.sort_by(|a, b| b.created_ms.cmp(&a.created_ms).then(a.id.cmp(&b.id)));
    deliveries.truncate(DELIVERIES_SHOWN);

    cards.sort_by(|a, b| b.since_ms.cmp(&a.since_ms).then(a.id.cmp(&b.id)));
    let activity = cards
        .iter()
        .filter(|c| c.needs && c.resolved_ms.is_none())
        .count();
    let directs = agents_out
        .iter()
        .filter(|a| {
            !a.temporary
                && a.last_action_ms
                    .is_some_and(|ms| now.saturating_sub(ms) <= ACTIVE_MS)
        })
        .map(|a| a.unread)
        .sum();
    // The built-in rooms are the agents' own: what they say there does not
    // ask for the operator, so it never raises the badge.
    let channel_unread = channels
        .iter()
        .filter(|c| !c.quiet && !c.archived && !c.built_in)
        .map(|c| c.unread)
        .sum();
    agents_out.sort_by(|a, b| {
        presence_rank(a.presence)
            .cmp(&presence_rank(b.presence))
            .then(
                b.last_action_ms
                    .unwrap_or(0)
                    .cmp(&a.last_action_ms.unwrap_or(0)),
            )
            .then(a.key.cmp(&b.key))
    });

    let lobby = lobby_cards(inp, &agents_ctx, &agents_out, &mut repo_of);
    let rev = rev_of(
        inp.operator,
        &channels,
        &messages,
        &cards,
        directs,
        activity,
    );
    let mut saved = inp.desk.saved.clone();
    saved.sort_by_key(|s| std::cmp::Reverse(s.saved_ms));
    WorkspaceView {
        v: WORKSPACE_VERSION,
        now_ms: now,
        operator: inp.operator.to_string(),
        operator_initials: if inp.operator == cfg.operator_name {
            cfg.operator_initials.clone()
        } else {
            initials_of(inp.operator)
        },
        thresholds: Thresholds {
            amber_ms,
            red_ms,
            thinking_long_ms: cfg.thinking_long_min * 60_000,
            waiting_sleepy_ms: cfg.waiting_sleepy_min * 60_000,
            running_long_ms: cfg.running_long_min * 60_000,
        },
        channels,
        lobby,
        agents: agents_out,
        messages,
        cards,
        deliveries,
        saved,
        unread: Unread {
            directs,
            activity,
            channels: channel_unread,
            todo: todo_unread,
        },
        rev,
        team_root_found: inp.team.root_found,
        format_changed: inp.team.format.format_changed(),
        motion: inp.desk.motion.clone().filter(|m| m == "reduce"),
        night_dim: inp.desk.night_dim.unwrap_or(true),
        agents_collapsed: inp.desk.agents_collapsed.unwrap_or(false),
        shipments: Vec::new(),
        running_commit: None,
    }
}

/// A face that means a tool call is still running.
fn in_flight(state: &str) -> bool {
    matches!(
        state,
        "running" | "testing" | "delegating" | "editing" | "reading" | "web"
    )
}

/// A short fingerprint of what the operator would see change: messages,
/// their replies and delivery states, open cards, unread counts.
fn rev_of(
    operator: &str,
    channels: &[Channel],
    messages: &[Message],
    cards: &[Card],
    directs: usize,
    activity: usize,
) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    // A rename in Profile changes every message's signature, and no message.
    operator.hash(&mut h);
    // Creating or archiving a channel changes no message.
    for c in channels {
        c.id.hash(&mut h);
        c.archived.hash(&mut h);
    }
    for m in messages {
        m.id.hash(&mut h);
        m.replies.len().hash(&mut h);
        if let Some(d) = &m.delivery {
            d.state.hash(&mut h);
            d.delivered.hash(&mut h);
            d.nudged_ms.hash(&mut h);
        }
    }
    for c in cards.iter().filter(|c| c.needs && c.resolved_ms.is_none()) {
        c.id.hash(&mut h);
    }
    (directs, activity).hash(&mut h);
    format!("{:016x}", h.finish())
}

/// The face, by precedence: nothing is known (lost) is plain; an open
/// BLOCKED; a finished agent, verified only on a landed receipt; dormant;
/// else what the transcript says.
fn face(
    raw: &Activity,
    closed: bool,
    lost: bool,
    dormant: Option<&'static str>,
    blocked: Option<&(u64, String)>,
    verified: Option<&Verified>,
    last: Option<u64>,
) -> Activity {
    let source = raw.source_ms;
    let quiet_since = last.unwrap_or(source);
    // Its lead stopped it (TaskStop): closed, whatever its last record says.
    if closed {
        return Activity::new("finished", quiet_since, Some("closed".into()), source);
    }
    if lost {
        return Activity::new("normal", quiet_since, None, source);
    }
    // No label: the BLOCKED sentence is message text, which the card and
    // the conversation show; a face carries only safe labels.
    if let Some((ts, _)) = blocked {
        return Activity::new("blocked", *ts, None, source);
    }
    if raw.state == "finished" {
        return match verified {
            Some(v) => Activity::new("verified", v.decided_ms, None, source),
            None => raw.clone(),
        };
    }
    // A call in flight writes nothing until it returns: a long test run is
    // quiet, not asleep. It reads "a long while" from the console's
    // threshold, and asleep only at the red one.
    let in_flight = in_flight(raw.state);
    match dormant {
        Some("red") => Activity::new("asleep", quiet_since, None, source),
        Some(_) if !in_flight => Activity::new("asleep", quiet_since, None, source),
        _ => raw.clone(),
    }
}

/// The newest event of each kind in the reaction window.
fn reactions_of(
    conv: &[Message],
    first_ms: Option<u64>,
    lead_ready: bool,
    now: u64,
) -> Vec<Reaction> {
    let fresh = |ms: u64| ms <= now && now - ms <= REACTION_WINDOW_MS;
    let newest = |f: &dyn Fn(&Message) -> Option<u64>| conv.iter().filter_map(f).max();
    let mut out = Vec::new();
    let mut add = |kind: &'static str, at: Option<u64>| {
        if let Some(at) = at.filter(|ms| fresh(*ms)) {
            out.push(Reaction { kind, at_ms: at });
        }
    };
    add(
        "glance",
        newest(&|m| (m.from == "agent").then_some(m.ts_ms)),
    );
    add(
        "spark",
        newest(&|m| (lead_ready && m.from == "operator" && !m.undelivered).then_some(m.ts_ms)),
    );
    add(
        "hop",
        newest(&|m| m.verified.as_ref().map(|v| v.decided_ms)),
    );
    add(
        "shake",
        newest(&|m| (m.from == "agent" && m.label == Some("BLOCKED")).then_some(m.ts_ms)),
    );
    add("boot", first_ms);
    out
}

type Spans = BTreeMap<String, Vec<(String, u64, u64)>>;

/// Widen `key`'s span in `checkout`, or start it.
fn add_span(spans: &mut Spans, checkout: &str, key: &str, first: u64, last: u64) {
    let list = spans.entry(checkout.to_string()).or_default();
    match list.iter_mut().find(|x| x.0 == key) {
        Some(x) => {
            x.1 = x.1.min(first);
            x.2 = x.2.max(last);
        }
        None => list.push((key.to_string(), first, last)),
    }
}

#[derive(Default)]
struct RepoAcc {
    checkouts: BTreeMap<String, Option<String>>,
    members: BTreeSet<String>,
    last_ms: u64,
    /// Newest write or live session: what keeps it out of "More channels".
    active_ms: u64,
    /// Some path in it is a confirmed git repository.
    real: bool,
    /// Who wrote in it, or worked in it as a session, and when last.
    workers: BTreeMap<String, u64>,
}

fn presence_rank(p: &str) -> u8 {
    match p {
        "working" => 0,
        "waiting" => 1,
        _ => 2,
    }
}

#[allow(clippy::too_many_arguments)]
fn card(
    id: String,
    kind: &'static str,
    channel: Option<String>,
    agents: Vec<String>,
    message: Option<String>,
    checkout: Option<String>,
    since_ms: u64,
    desk: &DeskState,
) -> Card {
    Card {
        needs: true,
        check: None,
        resolved_ms: desk.resolved.get(&id).copied(),
        id,
        kind,
        channel,
        agents,
        message,
        checkout,
        since_ms,
    }
}

/// A message's file cards and attachments. A path `harbard attach` copied
/// renders as an attachment; an HTML attachment is never rendered in the
/// console, so it stays a file card the OS opens. Every other report path
/// keeps today's card, opened only on the operator's click.
fn files_and_attachments(
    m: &MessageRow,
    attachment: &dyn Fn(&str) -> Option<Attachment>,
) -> (Vec<String>, Vec<Attachment>) {
    let mut attachments: Vec<Attachment> = Vec::new();
    let named = m
        .paths
        .iter()
        .chain(m.report.iter())
        .cloned()
        .chain(file_paths(&m.first_line));
    for p in named {
        if attachments.iter().any(|a| a.path == p) {
            continue;
        }
        if let Some(a) = attachment(&p)
            && attachments.iter().all(|x| x.id != a.id)
        {
            attachments.push(a);
        }
    }
    let mut files: Vec<String> = Vec::new();
    for p in m.report.iter().cloned().chain(file_paths(&m.first_line)) {
        let attached = attachments.iter().any(|a| a.path == p);
        if openable(&p) && !attached && !files.contains(&p) {
            files.push(p);
        }
    }
    for a in attachments.iter().filter(|a| a.kind == "html") {
        if !files.contains(&a.path) {
            files.push(a.path.clone());
        }
    }
    attachments.retain(|a| a.kind != "html");
    (files, attachments)
}

/// Where one operator message stands, from every copy the bus holds:
/// `answered` when a reply names any copy, `leido` when a session acked
/// it, `delivered` when a session's brief, inbox or watch handed it over,
/// else `sent`.
fn note_delivery(copies: &[&OperatorNote], leads: &BTreeSet<&str>) -> NoteDelivery {
    let reached = |t: &NoteTrail| t.delivered || t.acked_ms.is_some() || !t.replies.is_empty();
    let state = if copies.iter().any(|n| !n.trail.replies.is_empty()) {
        "answered"
    } else if copies.iter().any(|n| n.trail.acked_ms.is_some()) {
        "read"
    } else if copies.iter().any(|n| reached(&n.trail)) {
        "delivered"
    } else {
        "sent"
    };
    let mut sessions: Vec<String> = Vec::new();
    for n in copies {
        if !sessions.contains(&n.session) {
            sessions.push(n.session.clone());
        }
    }
    NoteDelivery {
        state,
        delivered: copies.iter().filter(|n| reached(&n.trail)).count(),
        to_lead: sessions.iter().all(|s| leads.contains(s.as_str())),
        sessions,
        nudged_ms: copies.iter().filter_map(|n| n.trail.nudged_ms).max(),
        answered_by: None,
    }
}

/// Who answered a note for an in-process agent, when it was not the agent.
/// Its lead reads the note, so a reply is the lead's own unless the lead
/// signed it with the agent's name (`harbard reply --agent`).
fn answered_instead(
    replies: &[BusReply],
    names: &[&str],
    name: &dyn Fn(&str) -> String,
) -> Option<String> {
    let signed = |r: &BusReply| names.iter().any(|n| r.text.starts_with(&format!("{n}: ")));
    if replies.iter().any(signed) {
        return None;
    }
    replies.last().map(|r| name(&r.session))
}

/// A note's human line and, for a v1 note, the lines after it.
fn with_more(sentence: String, more: Option<String>) -> String {
    match more {
        Some(more) => format!("{sentence}\n{more}"),
        None => sentence,
    }
}

/// A note written to a channel or to the operator (an operator note, a bus
/// note, a reply on the bus), as opposed to a line of an agent's own
/// conversation with its lead.
fn said_in_channel(m: &Message) -> bool {
    ["note:", "bus:", "reply:", "room:"]
        .iter()
        .any(|p| m.id.starts_with(p))
}

/// A bus note answering an operator note, as a reply in its thread. A lead
/// session's answer is the coordinator's.
fn bus_reply_message(
    r: &BusReply,
    parent: &str,
    agent: &str,
    channels: &[String],
    leads: &BTreeSet<&str>,
    name: &dyn Fn(&str) -> String,
    attachment: &dyn Fn(&str) -> Option<Attachment>,
) -> Message {
    let lead = leads.contains(r.session.as_str());
    let note = human::note_text(&r.text);
    let text = with_more(
        team::parse_label(&note.first).map_or(note.first.clone(), |l| l.sentence),
        note.more,
    );
    let attachments = operator_attachments(&r.text, attachment);
    Message {
        id: format!("reply:{}", r.id),
        agent: agent.to_string(),
        from: if lead { "lead" } else { "session" },
        author: name(&r.session),
        ts_ms: r.ts_ms,
        text,
        body: without_attached(note.body, &attachments),
        label: None,
        version: None,
        flags: Vec::new(),
        jargon: Vec::new(),
        readable: None,
        shape: None,
        repeat_of: None,
        chars: r.text.chars().count(),
        files: file_paths(&r.text),
        verified: None,
        channels: channels.to_vec(),
        reply_to: Some(parent.to_string()),
        replies: Vec::new(),
        structured: None,
        undelivered: false,
        attachments,
        delivery: None,
        mentions: Vec::new(),
    }
}

/// A bus note from a session to the operator (or to everyone) as a message:
/// the lead's are the coordinator's. Its label and human line read as an
/// agent's would.
fn bus_message(
    b: &BusNote,
    agent: &str,
    channels: &[String],
    name: &dyn Fn(&str) -> String,
    leads: &BTreeSet<&str>,
    handles: &BTreeSet<String>,
    attachment: &dyn Fn(&str) -> Option<Attachment>,
) -> Message {
    // The whole note, never the 200-character first line a teammate row
    // keeps: a long line from the lead was cut short with a silent "…".
    let note = human::note_text(&b.text);
    let first = note.first.clone();
    let parsed = team::parse_label(&first);
    let label = parsed.as_ref().map(|l| l.label);
    let sentence = parsed.map_or(first.clone(), |l| l.sentence);
    let jargon = human::jargon(&sentence);
    let readable = crate::readable::readable(&sentence, label);
    let shape = crate::readable::shape(&sentence, label);
    let text = with_more(sentence, note.more);
    let row = MessageRow {
        dir: "out",
        peer: String::new(),
        ts_ms: b.ts_ms,
        first_line: first,
        label: None,
        chars: b.text.chars().count(),
        report: team::report_path(&b.text),
        repeat: false,
        undelivered: false,
        structured: None,
        digest: String::new(),
        shas: Vec::new(),
        body: None,
        paths: human::path_tokens(&b.text),
    };
    let (files, attachments) = files_and_attachments(&row, attachment);
    Message {
        id: format!("bus:{}", b.id),
        agent: agent.to_string(),
        from: if leads.contains(b.session.as_str()) {
            "lead"
        } else {
            "session"
        },
        author: name(&b.session),
        ts_ms: b.ts_ms,
        text,
        body: without_attached(note.body, &attachments),
        label,
        version: None,
        flags: if jargon.is_empty() {
            Vec::new()
        } else {
            vec!["jargon"]
        },
        jargon,
        readable,
        shape,
        repeat_of: None,
        chars: b.text.chars().count(),
        files,
        verified: None,
        channels: channels.to_vec(),
        reply_to: None,
        replies: Vec::new(),
        structured: None,
        undelivered: false,
        attachments,
        delivery: None,
        mentions: mentions_in(&b.text, handles),
    }
}

/// A note in a room as a message of the view (and of its history route).
pub fn room_message(
    n: &RoomNote,
    operator: &str,
    handles: &BTreeSet<String>,
    leads: &BTreeSet<&str>,
    attachment: &dyn Fn(&str) -> Option<Attachment>,
) -> Message {
    let bus = BusNote {
        id: n.id.clone(),
        ts_ms: n.ts_ms,
        session: n.session.clone(),
        text: n.text.clone(),
        repo: n.repo.clone(),
        note_ref: n.note_ref.clone(),
        kind: n.kind.clone(),
    };
    let author = if n.operator {
        operator.to_string()
    } else {
        n.author.clone()
    };
    let name = move |_: &str| author.clone();
    let room = [n.room.clone()];
    let mut m = bus_message(&bus, "", &room, &name, leads, handles, attachment);
    m.id = room_message_id(&n.id);
    m.reply_to = n.note_ref.as_deref().map(room_message_id);
    if n.operator {
        m.from = "operator";
        m.flags = Vec::new();
        m.jargon = Vec::new();
        m.readable = None;
        m.shape = None;
    }
    m
}

/// The agent whose intro this is, and the intro without its signature, when
/// its lead signed the note with that agent's name (`harbard intro --agent`,
/// signed the way `harbard reply --agent` signs: `<name>: <text>`).
fn signed_intro<'a, 'n>(note: &'n RoomNote, named: &[&'a Agent]) -> Option<(&'a Agent, &'n str)> {
    named
        .iter()
        .filter(|a| a.lead_session == note.session)
        .find_map(|a| {
            [Some(a.name.as_str()), a.own_name.as_deref()]
                .into_iter()
                .flatten()
                .find_map(|name| note.text.strip_prefix(&format!("{name}: ")))
                .map(|rest| (*a, rest))
        })
}

/// One card per main session and per named agent, newest first.
fn lobby_cards(
    inp: &Inputs<'_>,
    agents_ctx: &[AgentCtx<'_>],
    agents_out: &[Agent],
    repo_of: &mut dyn FnMut(&str) -> String,
) -> Vec<LobbyCard> {
    let named: Vec<&Agent> = agents_out
        .iter()
        .filter(|a| {
            a.kind != "session"
                && !a.temporary
                && !a.hidden
                && (a.own_name.is_some()
                    || matches!(a.name_source.as_deref(), Some("picked" | "renamed")))
        })
        .collect();
    // The latest intro per session, and per named agent its lead signed one
    // for; a signed intro is the agent's, never its lead's.
    let mut intros: HashMap<&str, (String, u64)> = HashMap::new();
    for n in inp
        .room_notes
        .iter()
        .filter(|n| n.room == crate::membership::LOBBY)
    {
        let (who, text) = match signed_intro(n, &named) {
            Some((agent, rest)) => (agent.key.as_str(), rest),
            None => (n.session.as_str(), n.text.as_str()),
        };
        intros.insert(who, (human::note_text(text).first, n.ts_ms));
    }
    let intro_of = |key: &str| -> (Option<String>, Option<u64>) {
        match intros.get(key) {
            Some((text, ts)) => (Some(text.clone()), Some(*ts)),
            None => (None, None),
        }
    };
    let mut cards: Vec<LobbyCard> = Vec::new();
    for s in inp
        .sessions
        .iter()
        .filter(|s| s.main && !naming::is_temp(&s.cwd, inp.temp_dirs))
    {
        let key = format!("session:{}", s.session);
        let name = agents_ctx
            .iter()
            .find(|a| a.row.id == key)
            .map(|a| a.name.clone())
            .or_else(|| s.name.clone())
            .or_else(|| s.callsign.clone())
            .unwrap_or_else(|| {
                format!("session {}", s.session.chars().take(8).collect::<String>())
            });
        let presence = match s.liveness {
            Liveness::Live if s.idle => "idle",
            Liveness::Live => "live",
            Liveness::Stale => "stale",
            Liveness::Ended => "ended",
        };
        let (intro, intro_ts_ms) = intro_of(&s.session);
        cards.push(LobbyCard {
            key,
            name,
            harness: harness_label(&s.harness),
            project: s
                .repo
                .as_deref()
                .filter(|r| r.starts_with('/'))
                .map(|r| repo_name(&repo_of(r))),
            intro,
            intro_ts_ms,
            presence,
            first_ms: s.first_ms,
            last_ms: s.last_ms,
        });
    }
    for a in named {
        let (intro, intro_ts_ms) = intro_of(&a.key);
        cards.push(LobbyCard {
            key: a.key.clone(),
            name: a.name.clone(),
            harness: "claude-code",
            project: a.channels.first().cloned(),
            intro,
            intro_ts_ms,
            presence: match (a.lost, a.presence) {
                (true, _) => "stale",
                (_, "working") => "live",
                (_, "waiting") => "idle",
                _ => "ended",
            },
            first_ms: a.first_ms.unwrap_or(0),
            last_ms: a.last_action_ms.unwrap_or(0),
        });
    }
    cards.sort_by(|a, b| b.last_ms.cmp(&a.last_ms).then(a.key.cmp(&b.key)));
    cards.truncate(LOBBY_SHOWN);
    cards
}

/// A note's technical body without the lines that only cite a file shown
/// above it; `--attach` puts those there, and the operator already sees the file.
fn without_attached(body: Option<String>, attachments: &[Attachment]) -> Option<String> {
    let kept: Vec<&str> = body
        .as_deref()?
        .lines()
        .filter(|line| !attachments.iter().any(|a| a.path == line.trim()))
        .collect();
    let joined = kept.join("\n");
    let trimmed = joined.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// The attachments an operator note names (a file they attached from the
/// composer's "+").
fn operator_attachments(
    body: &str,
    attachment: &dyn Fn(&str) -> Option<Attachment>,
) -> Vec<Attachment> {
    let mut out: Vec<Attachment> = Vec::new();
    for p in human::path_tokens(body) {
        if let Some(a) = attachment(&p)
            && a.kind != "html"
            && out.iter().all(|x| x.id != a.id)
        {
            out.push(a);
        }
    }
    out
}

/// "Ana Ruiz" → "AR", "Adonay" → "AD": for a name the desk has no initials for.
fn initials_of(name: &str) -> String {
    let mut words = name.split_whitespace();
    let first: Vec<char> = words.next().unwrap_or_default().chars().collect();
    let second = words
        .next()
        .and_then(|w| w.chars().next())
        .or(first.get(1).copied());
    first
        .first()
        .copied()
        .into_iter()
        .chain(second)
        .flat_map(char::to_uppercase)
        .collect()
}

/// The `@` tokens of a text that name someone harbard knows.
fn mentions_in(text: &str, handles: &BTreeSet<String>) -> Vec<String> {
    naming::mention_tokens(text)
        .into_iter()
        .filter(|t| handles.contains(t))
        .collect()
}

/// What an `@` names an agent by.
fn handle_of(a: &AgentCtx<'_>) -> String {
    if a.row.kind == "session" && a.name == LEAD_AUTHOR {
        return "coordinator".into();
    }
    // A workshop name is what the operator reads and types: `@nora`.
    if let Some(w) = a.workshop {
        return w.name.to_lowercase().replace(' ', "-");
    }
    match &a.row.name {
        Some(n) => n.to_lowercase().replace(' ', "-"),
        // A session that goes by its callsign.
        None if a.row.kind == "session" && !a.name.contains('·') => {
            a.name.to_lowercase().replace(' ', "-")
        }
        None => a.row.id.chars().take(8).collect::<String>().to_lowercase(),
    }
}

/// A message the operator has to read: an agent's, or a session's on the bus.
fn is_incoming(m: &Message) -> bool {
    m.from == "agent"
        || m.id.starts_with("bus:")
        || m.id.starts_with("reply:")
        || (m.id.starts_with("room:") && m.from != "operator")
}

/// "Nobody has answered your message": an operator note older than the
/// threshold with no answer. Only the last day's notes ask for anything;
/// older ones are history.
fn unanswered_card(
    m: &Message,
    answered: bool,
    now: u64,
    after_ms: u64,
    desk: &DeskState,
) -> Option<Card> {
    let age = now.saturating_sub(m.ts_ms);
    if answered || age < after_ms || age > ACTIVE_MS {
        return None;
    }
    Some(card(
        format!("unanswered:{}", m.id),
        "unanswered",
        m.channels.first().cloned(),
        if m.agent.is_empty() {
            Vec::new()
        } else {
            vec![m.agent.clone()]
        },
        Some(m.id.clone()),
        None,
        m.ts_ms,
        desk,
    ))
}

/// The ids of every file card in a view: the only paths `open` accepts.
pub fn file_set(view: &WorkspaceView) -> BTreeSet<String> {
    let mut out: BTreeSet<String> = BTreeSet::new();
    for m in &view.messages {
        out.extend(m.files.iter().cloned());
    }
    for s in &view.saved {
        out.extend(s.path.iter().cloned());
    }
    out
}

/// Where a reply goes. `Ok(session)` for the agent's lead; `Err(reason)`
/// when the bus cannot carry it. With `anyway` (the operator confirmed
/// "enviar samemente") an ended or unknown lead gets it stored too: it is
/// read if that session comes back, and "Resend to the lead" can send
/// it again later.
pub fn reply_route<'v>(
    view: &'v WorkspaceView,
    agent: &str,
    anyway: bool,
) -> Result<&'v Agent, &'static str> {
    let a = view
        .agents
        .iter()
        .find(|a| a.key == agent)
        .ok_or("unknown_agent")?;
    // A stale lead gets the note stored; it waits for the session to return.
    if anyway || matches!(a.delivery, "ready" | "stale") {
        Ok(a)
    } else {
        Err(a.delivery)
    }
}

/// The sessions a channel note is stored for: the channel's readers (the
/// members' leads and the sessions working in the repository) that the bus
/// can carry a note to, each once; with `anyway`, every reader.
pub fn channel_route(view: &WorkspaceView, channel: &str, anyway: bool) -> Vec<String> {
    let Some(ch) = view.channels.iter().find(|c| c.id == channel) else {
        return Vec::new();
    };
    let reachable = |r: &&ChannelReader| anyway || matches!(r.delivery, "ready" | "stale");
    // The lead sessions first, then every other session working in the
    // repository: the bus does not record who launched a session, and in
    // practice they are independent sessions.
    let mut out: Vec<String> = ch
        .readers
        .iter()
        .filter(|r| r.lead)
        .filter(reachable)
        .map(|r| r.session.clone())
        .collect();
    for r in ch.readers.iter().filter(|r| !r.lead).filter(reachable) {
        if !out.contains(&r.session) {
            out.push(r.session.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    //! Synthetic data only: agent rows are built field by field, never read
    //! from a real transcript.

    use super::*;
    use crate::team::{CheckoutUse, FormatProbe, LeadPanel};
    use crate::test_util::TempHome;

    const NOW: u64 = 1_794_000_000_000;
    const MIN: u64 = 60_000;
    const LEAD: &str = "11111111-2222-3333-4444-555555555555";

    fn msg(dir: &'static str, peer: &str, ts: u64, first: &str) -> MessageRow {
        MessageRow {
            dir,
            peer: peer.into(),
            ts_ms: ts,
            first_line: first.into(),
            label: team::parse_label(first).map(|l| match l.version {
                Some(v) => format!("{} v{v}", l.label),
                None => l.label.to_string(),
            }),
            chars: first.chars().count(),
            report: team::report_path(first),
            repeat: false,
            undelivered: false,
            structured: None,
            digest: format!("{:016x}", first.len() as u64 * 31 + ts % 7),
            shas: team::commit_tokens(first),
            body: None,
            paths: crate::human::path_tokens(first),
        }
    }

    fn agent(id: &str, name: &str, state: &'static str, last: u64) -> AgentRow {
        AgentRow {
            id: id.into(),
            name: Some(name.into()),
            kind: "teammate",
            agent_type: Some("general-purpose".into()),
            color: None,
            model: None,
            description: Some("Synthetic task".into()),
            brief: Some("Synthetic brief".into()),
            state,
            state_reason: "mid_turn",
            last_action_ms: Some(last),
            seen_ms: last,
            records: 10,
            checkouts: Vec::new(),
            unplaced_writes: 0,
            worktree: None,
            messages: Vec::new(),
            messages_total: 0,
            last_label: None,
            first_ms: Some(last.saturating_sub(60 * MIN)),
            git_roots: Vec::new(),
            write_dirs: Vec::new(),
            git_dirs: Vec::new(),
            target_dirs: Vec::new(),
            activity: crate::activity::Activity::plain(state, last),
            progress: Default::default(),
        }
    }

    fn writes(path: &str, first: u64, last: u64) -> CheckoutUse {
        CheckoutUse {
            path: path.into(),
            writes: 3,
            first_ms: first,
            last_ms: last,
        }
    }

    fn lead(agents: Vec<AgentRow>) -> LeadPanel {
        LeadPanel {
            session: LEAD.into(),
            project: "-work-lead".into(),
            cwd: Some("/work/lead".into()),
            last_write_ms: NOW,
            lead_last_ms: Some(NOW),
            lead_found: true,
            agents,
            warnings: Vec::new(),
            skipped_lines: 0,
            lead_write_dirs: Vec::new(),
            lead_cmd_dirs: Vec::new(),
            git_cwds: Vec::new(),
        }
    }

    fn team_view(leads: Vec<LeadPanel>) -> TeamView {
        TeamView {
            v: 1,
            now_ms: NOW,
            window_ms: 24 * 60 * MIN,
            quiet_running_ms: 15 * MIN,
            root_found: true,
            leads,
            format: FormatProbe::default(),
        }
    }

    fn session(id: &str, liveness: Liveness) -> BusSession {
        BusSession {
            session: id.into(),
            liveness,
            repo: Some("/work/lead".into()),
            last_ms: NOW,
            first_ms: NOW - 60 * MIN,
            cwd: "/work/lead".into(),
            name: None,
            callsign: None,
            main: false,
            harness: "claude-code".into(),
            idle: false,
            rooms: Vec::new(),
        }
    }

    /// `/work/<repo>-wt-<x>/…` is a worktree of `/work/<repo>`; anything
    /// outside `/work` is in no checkout.
    fn resolver(path: &str) -> Option<CheckoutInfo> {
        let base = path.strip_prefix("/work/")?;
        let top = base.split('/').next().unwrap_or(base);
        let repo = top.split("-wt-").next().unwrap_or(top);
        Some(CheckoutInfo {
            checkout: format!("/work/{top}"),
            repo: format!("/work/{repo}"),
            branch: Some(if top.contains("-wt-") {
                "feature".into()
            } else {
                "main".into()
            }),
            real: true,
        })
    }

    fn no_attachment(_: &str) -> Option<Attachment> {
        None
    }

    struct World {
        team: TeamView,
        sessions: Vec<BusSession>,
        notes: Vec<OperatorNote>,
        bus_notes: Vec<BusNote>,
        names: BTreeMap<String, crate::workshop::Entry>,
        deliveries: Vec<Delivery>,
        receipts: Vec<Receipt>,
        desk: DeskState,
        now: u64,
        horizon_ms: u64,
        keep_ended: bool,
        known_repos: Vec<crate::rooms::KnownRepo>,
        rooms: Vec<RoomInput>,
        room_notes: Vec<RoomNote>,
        archived_repos: BTreeSet<String>,
        workspace: Option<String>,
    }

    impl World {
        fn new(agents: Vec<AgentRow>) -> Self {
            World {
                team: team_view(vec![lead(agents)]),
                sessions: vec![session(LEAD, Liveness::Live)],
                notes: Vec::new(),
                bus_notes: Vec::new(),
                names: BTreeMap::new(),
                deliveries: Vec::new(),
                receipts: Vec::new(),
                desk: DeskState::default(),
                now: NOW,
                horizon_ms: ACTIVE_MS,
                keep_ended: false,
                known_repos: Vec::new(),
                rooms: Vec::new(),
                room_notes: Vec::new(),
                archived_repos: BTreeSet::new(),
                workspace: None,
            }
        }

        fn view(&self) -> WorkspaceView {
            build(&Inputs {
                team: &self.team,
                sessions: &self.sessions,
                notes: &self.notes,
                bus_notes: &self.bus_notes,
                temp_dirs: &[],
                names: &self.names,
                deliveries: &self.deliveries,
                receipts: &self.receipts,
                desk: &self.desk,
                resolve: &resolver,
                attachment: &no_attachment,
                now_ms: self.now,
                operator: "Dana",
                horizon_ms: self.horizon_ms,
                keep_ended: self.keep_ended,
                in_workspace: &|_| true,
                known_repos: &self.known_repos,
                rooms: &self.rooms,
                room_notes: &self.room_notes,
                archived_repos: &self.archived_repos,
                workspace: self.workspace.as_deref(),
            })
        }
    }

    #[test]
    fn history_keeps_an_ended_session_of_last_week_and_the_shell_does_not() {
        let mut w = World::new(Vec::new());
        let mut old = session("22222222-3333-4444-5555-666666666666", Liveness::Ended);
        old.main = true;
        old.repo = Some("/work/harbard".into());
        old.cwd = "/work/harbard".into();
        old.last_ms = NOW - 6 * 24 * 60 * MIN;
        old.first_ms = old.last_ms - 90 * MIN;
        w.sessions.push(old);
        let shell = w.view();
        assert!(
            !shell
                .agents
                .iter()
                .any(|a| a.lead_session.starts_with("22222222"))
        );
        w.horizon_ms = crate::history::WINDOW_MS;
        w.keep_ended = true;
        let archive = w.view();
        let a = archive
            .agents
            .iter()
            .find(|a| a.lead_session.starts_with("22222222"))
            .expect("the archive keeps it");
        assert_eq!(a.delivery, "ended");
        assert_eq!(a.kind, "session");
    }

    fn by_id<'a>(view: &'a WorkspaceView, id: &str) -> &'a Message {
        view.messages.iter().find(|m| m.id == id).unwrap()
    }

    fn of_agent<'a>(view: &'a WorkspaceView, key: &str) -> Vec<&'a Message> {
        view.messages.iter().filter(|m| m.agent == key).collect()
    }

    fn open_cards(view: &WorkspaceView) -> Vec<&Card> {
        view.cards
            .iter()
            .filter(|c| c.needs && c.resolved_ms.is_none())
            .collect()
    }

    /// Work by agents in isolated worktrees on a mounted volume must still give
    /// the workspace a channel, though the path was never read and the worktrees
    /// are gone.
    #[test]
    fn an_isolated_worktree_counts_for_its_leads_repository_without_reading_it() {
        let worktree = "/Volumes/External/worktrees/orchard/agent-a1";
        let mut given = agent("a1", "builder", "done", NOW - 30 * MIN);
        given.worktree = Some(worktree.into());
        given.checkouts = vec![writes(worktree, NOW - 90 * MIN, NOW - 30 * MIN)];
        // The same path without a worktree from the harness proves nothing.
        let mut stray = agent("a2", "stray", "done", NOW - 30 * MIN);
        stray.checkouts = vec![writes(
            "/Volumes/External/elsewhere",
            NOW - 90 * MIN,
            NOW - 30 * MIN,
        )];
        let mut w = World::new(vec![given, stray]);
        w.team.leads[0].cwd = Some("/work/orchard".into());
        w.sessions.clear();
        let v = w.view();
        let orchard = v.channels.iter().find(|c| c.id == "orchard").unwrap();
        assert_eq!(orchard.members, vec!["a1".to_string()]);
        assert!(orchard.checkouts.iter().any(|c| c.path == worktree));
        let a1 = v.agents.iter().find(|a| a.key == "a1").unwrap();
        assert_eq!(a1.channels, vec!["orchard".to_string()]);
        let a2 = v.agents.iter().find(|a| a.key == "a2").unwrap();
        assert!(a2.channels.is_empty());

        // A lead outside any real repository vouches for nothing.
        w.team.leads[0].cwd = Some("/Volumes/External/orchard".into());
        assert!(w.view().channels.is_empty());
    }

    #[test]
    fn channels_come_from_checkouts_with_worktrees_folded_into_their_repo() {
        let mut a = agent("a1", "keychain", "running", NOW - MIN);
        a.checkouts = vec![writes(
            "/work/orchard-wt-identity",
            NOW - 30 * MIN,
            NOW - MIN,
        )];
        let mut b = agent("a2", "keychain-review-2", "idle", NOW - 5 * MIN);
        b.git_roots = vec!["/work/orchard".into()];
        let mut c = agent("a3", "harbard-view", "done", NOW - 50 * MIN);
        c.worktree = Some("/work/harbard-wt-view".into());
        let d = agent("a4", "loner", "idle", NOW - 2 * MIN);
        let w = World::new(vec![a, b, c, d]);
        let v = w.view();
        let ids: Vec<&str> = v.channels.iter().map(|c| c.id.as_str()).collect();
        // The lead session's own repo is a channel too: it is a live checkout.
        assert!(ids.contains(&"orchard") && ids.contains(&"harbard") && ids.contains(&"lead"));
        assert!(!ids.iter().any(|i| i.contains("-wt-")), "{ids:?}");
        let orchard = v.channels.iter().find(|c| c.id == "orchard").unwrap();
        assert_eq!(orchard.members, vec!["a1".to_string(), "a2".to_string()]);
        assert_eq!(orchard.checkouts.len(), 2);
        let wt = orchard
            .checkouts
            .iter()
            .find(|c| c.path.ends_with("-wt-identity"))
            .unwrap();
        assert_eq!(wt.branch.as_deref(), Some("feature"));
        // An agent with no evidence of a checkout belongs to no channel.
        let loner = v.agents.iter().find(|a| a.key == "a4").unwrap();
        assert!(loner.channels.is_empty());
        // Presence: running green, idle amber, done grey.
        let presence: BTreeMap<&str, &str> = v
            .agents
            .iter()
            .map(|a| (a.name.as_str(), a.presence))
            .collect();
        assert_eq!(presence["lead · keychain"], "working");
        assert_eq!(presence["keychain · reviewer 2"], "waiting");
        assert_eq!(presence["harbard · view"], "finished");
    }

    #[test]
    fn labels_are_parsed_and_unlabeled_long_and_repeated_messages_are_flagged() {
        let mut a = agent("a1", "keychain", "running", NOW - MIN);
        a.checkouts = vec![writes("/work/orchard", NOW - 30 * MIN, NOW - MIN)];
        let start = msg(
            "out",
            "team-lead",
            NOW - 40 * MIN,
            "START · keychain · Trabajo en orchard-wt-identity, build dir /work/orchard-wt-identity/target.",
        );
        let plain = msg(
            "out",
            "team-lead",
            NOW - 30 * MIN,
            "finished, see the report",
        );
        let mut long = msg(
            "out",
            "team-lead",
            NOW - 20 * MIN,
            "FINDING · keychain · Un hallazgo muy largo.",
        );
        long.chars = 1_201;
        let first = msg(
            "out",
            "team-lead",
            NOW - 15 * MIN,
            "DONE · keychain · Hecho.",
        );
        let mut again = first.clone();
        again.ts_ms = NOW - 10 * MIN;
        again.repeat = true;
        a.messages = vec![start, plain, long, first, again];
        let v = World::new(vec![a]).view();
        let m = of_agent(&v, "a1");
        assert_eq!(m[0].label, Some("START"));
        assert_eq!(
            m[0].text,
            "Trabajo en orchard-wt-identity, build dir /work/orchard-wt-identity/target."
        );
        assert!(m[0].flags.is_empty());
        assert_eq!(m[1].label, None);
        assert_eq!(m[1].flags, vec!["unlabeled"]);
        assert_eq!(m[2].label, Some("FINDING"));
        assert_eq!(m[2].flags, vec!["too_long"]);
        assert!(m[3].flags.is_empty());
        assert_eq!(m[4].flags, vec!["repeat"]);
        assert_eq!(m[4].repeat_of.as_deref(), Some(m[3].id.as_str()));
        // Exactly 1,200 characters is still within the rule.
        let mut edge = agent("a2", "edge", "running", NOW - MIN);
        let mut at_limit = msg(
            "out",
            "team-lead",
            NOW - MIN,
            "DONE · edge · Right at the limit.",
        );
        at_limit.chars = 1_200;
        edge.messages = vec![at_limit];
        let edge_view = World::new(vec![edge]).view();
        assert!(of_agent(&edge_view, "a2")[0].flags.is_empty());
        let profile = v.agents.iter().find(|a| a.key == "a1").unwrap();
        assert_eq!(
            profile.build_dir.as_deref(),
            Some("/work/orchard-wt-identity/target")
        );
        assert_eq!(profile.deliveries, 2);
        assert_eq!(profile.channels, vec!["orchard".to_string()]);
    }

    #[test]
    fn a_report_path_becomes_a_file_card_and_only_card_types_open() {
        let mut a = agent("a1", "harbard-view", "done", NOW - MIN);
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - MIN,
            "DONE · harbard-view · View ready: /Users/x/Documents/harbard-view.md and screenshot /Users/x/shot.png.",
        )];
        let v = World::new(vec![a]).view();
        let m = &of_agent(&v, "a1")[0];
        assert_eq!(
            m.files,
            vec!["/Users/x/Documents/harbard-view.md", "/Users/x/shot.png"]
        );
        let files = file_set(&v);
        assert!(files.contains("/Users/x/Documents/harbard-view.md"));
        assert!(openable("/Users/x/r.html") && openable("/Users/x/r.json"));
        assert!(!openable("/Users/x/r.sh") && !openable("relative/r.md"));
        assert!(!openable("/Users/x/../etc/r.md"));
        assert_eq!(
            file_paths("see `~/r.md` and (/a/b.md), /a/c.txt"),
            vec!["/a/b.md"]
        );
    }

    #[test]
    fn replies_from_the_lead_and_the_operator_thread_under_the_agent_message_in_order() {
        let mut a = agent("a1", "keychain-review-2", "idle", NOW - MIN);
        a.messages = vec![
            msg(
                "out",
                "team-lead",
                NOW - 30 * MIN,
                "FINDING · keychain-review-2 · Un llavero plantado pasa como probado.",
            ),
            msg("in", "team-lead", NOW - 25 * MIN, "With which test?"),
            msg("in", "team-lead", NOW - 20 * MIN, "Add the case."),
            msg(
                "out",
                "team-lead",
                NOW - 10 * MIN,
                "DONE · keychain-review-2 · Case added.",
            ),
        ];
        let mut w = World::new(vec![a]);
        w.notes.push(OperatorNote {
            id: "01NOTE".into(),
            ts_ms: NOW - 22 * MIN,
            session: LEAD.into(),
            text: "Para keychain-review-2: thanks".into(),
            ..Default::default()
        });
        let v = w.view();
        let m = of_agent(&v, "a1");
        let finding = m.iter().find(|x| x.label == Some("FINDING")).unwrap();
        assert_eq!(finding.replies.len(), 3);
        let texts: Vec<&str> = finding
            .replies
            .iter()
            .map(|id| by_id(&v, id).text.as_str())
            .collect();
        assert_eq!(texts, vec!["With which test?", "thanks", "Add the case."]);
        assert_eq!(by_id(&v, &finding.replies[1]).from, "operator");
        assert_eq!(by_id(&v, &finding.replies[1]).author, "Dana");
        let done = m.iter().find(|x| x.label == Some("DONE")).unwrap();
        assert!(done.replies.is_empty() && done.reply_to.is_none());
    }

    #[test]
    fn an_explicit_thread_reply_attaches_to_the_named_message() {
        let mut a = agent("a1", "keychain", "idle", NOW - MIN);
        a.messages = vec![
            msg(
                "out",
                "team-lead",
                NOW - 30 * MIN,
                "FINDING · keychain · Uno.",
            ),
            msg(
                "out",
                "team-lead",
                NOW - 20 * MIN,
                "FINDING · keychain · Dos.",
            ),
        ];
        let mut w = World::new(vec![a]);
        let first = World::new(w.team.leads[0].agents.clone()).view();
        let target = of_agent(&first, "a1")[0].id.clone();
        w.notes.push(OperatorNote {
            id: "01REPLY".into(),
            ts_ms: NOW - 5 * MIN,
            session: LEAD.into(),
            text: "Para keychain: sobre el primero".into(),
            ..Default::default()
        });
        w.desk.threads.insert("01REPLY".into(), target.clone());
        let v = w.view();
        assert_eq!(by_id(&v, &target).replies, vec!["note:01REPLY".to_string()]);
    }

    #[test]
    fn a_blocked_card_stays_until_the_lead_or_the_operator_answers() {
        let mut a = agent("a1", "keychain", "idle", NOW - MIN);
        a.checkouts = vec![writes("/work/orchard", NOW - 60 * MIN, NOW - 30 * MIN)];
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 20 * MIN,
            "BLOCKED · keychain · Can a rotation ask for the password twice?",
        )];
        let mut w = World::new(vec![a.clone()]);
        let v = w.view();
        let cards = open_cards(&v);
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].kind, "blocked");
        assert_eq!(cards[0].channel.as_deref(), Some("orchard"));
        assert_eq!(v.unread.activity, 1);

        // A sibling teammate's message is not an answer.
        let mut peer = a.clone();
        peer.messages.push(msg(
            "in",
            "someone-else",
            NOW - 10 * MIN,
            "I do not know either",
        ));
        w.team = team_view(vec![lead(vec![peer])]);
        assert_eq!(open_cards(&w.view()).len(), 1);

        // The lead answers: the card is gone.
        let mut answered = a.clone();
        answered
            .messages
            .push(msg("in", "team-lead", NOW - 5 * MIN, "Yes, twice."));
        w.team = team_view(vec![lead(vec![answered])]);
        assert!(open_cards(&w.view()).is_empty());

        // The operator answers instead, as a reply to it: gone too.
        w.team = team_view(vec![lead(vec![a])]);
        let blocked = open_cards(&w.view())[0].message.clone().unwrap();
        w.notes.push(OperatorNote {
            id: "01N".into(),
            ts_ms: NOW - 2 * MIN,
            session: LEAD.into(),
            text: "Para keychain: sí".into(),
            ..Default::default()
        });
        w.desk.threads.insert("01N".into(), blocked);
        assert!(open_cards(&w.view()).is_empty());
    }

    #[test]
    fn a_desk_file_without_a_memory_sink_leaves_it_off_and_writes_nothing_for_it() {
        let parsed: DeskState = serde_json::from_str("{}").unwrap();
        assert_eq!(parsed.memory_sink_allowed, None);
        assert_eq!(parsed.config.memory_sink_command, None);
        let written = serde_json::to_string(&parsed).unwrap();
        assert!(!written.contains("memory_sink"), "{written}");
    }

    #[test]
    fn mark_resolved_is_stored_in_the_desk_file_and_survives_a_restart() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let mut a = agent("a1", "keychain", "idle", NOW - MIN);
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 20 * MIN,
            "BLOCKED · keychain · Yes or no?",
        )];
        let mut w = World::new(vec![a]);
        let card_id = w.view().cards[0].id.clone();

        let mut desk = DeskState::load(&store);
        desk.resolve(&card_id, NOW);
        desk.save(&store).unwrap();
        // A new process reads the same file.
        w.desk = DeskState::load(&store);
        let v = w.view();
        assert_eq!(v.cards[0].resolved_ms, Some(NOW));
        assert!(open_cards(&v).is_empty());
        assert_eq!(v.unread.activity, 0);
        // Nothing about it reached the bus: the desk is a file of its own.
        assert!(DeskState::path(store.root()).exists());
        assert!(!store.root().join("events").exists());

        w.desk.reopen(&card_id);
        assert_eq!(open_cards(&w.view()).len(), 1);
    }

    #[test]
    fn dormant_thresholds_of_15_and_45_minutes_give_amber_then_red_with_a_fake_clock() {
        let last = NOW;
        let mut w = World::new(vec![agent("a1", "keychain", "running", last)]);
        let at = |w: &mut World, minutes: u64| {
            w.now = last + minutes * MIN;
            let v = w.view();
            let a = v.agents[0].clone();
            (
                a.dormant,
                open_cards(&v).iter().any(|c| c.kind == "dormant"),
            )
        };
        assert_eq!(at(&mut w, 15), (None, false));
        assert_eq!(at(&mut w, 16), (Some("amber"), false));
        assert_eq!(at(&mut w, 45), (Some("amber"), false));
        assert_eq!(at(&mut w, 46), (Some("red"), true));
        // An agent waiting for input is not dormant, however long.
        w.team = team_view(vec![lead(vec![agent("a1", "keychain", "idle", last)])]);
        assert_eq!(at(&mut w, 120), (None, false));
        // Thresholds come from the desk config.
        w.team = team_view(vec![lead(vec![agent("a1", "keychain", "running", last)])]);
        w.desk.config = DeskConfig {
            dormant_amber_min: 5,
            dormant_red_min: 10,
            ..DeskConfig::default()
        };
        assert_eq!(at(&mut w, 6), (Some("amber"), false));
        assert_eq!(at(&mut w, 11), (Some("red"), true));
    }

    #[test]
    fn two_writers_are_detected_per_agent_including_seven_teammates_of_one_lead() {
        // Tonight's case: one lead session, seven teammates, one checkout. The
        // bus sees one session; the workspace sees seven writers.
        let agents: Vec<AgentRow> = (1..=7)
            .map(|i| {
                let mut a = agent(&format!("t{i}"), &format!("mate-{i}"), "running", NOW - MIN);
                a.checkouts = vec![writes(
                    "/work/orchard-wt-identity",
                    NOW - 60 * MIN + i * MIN,
                    NOW - MIN,
                )];
                a
            })
            .collect();
        let v = World::new(agents).view();
        let ch = v.channels.iter().find(|c| c.id == "orchard").unwrap();
        assert_eq!(ch.warnings.len(), 1);
        assert_eq!(ch.warnings[0].agents.len(), 7);
        assert_eq!(ch.warnings[0].checkout, "/work/orchard-wt-identity");
        let cards: Vec<&Card> = open_cards(&v)
            .into_iter()
            .filter(|c| c.kind == "two_writers")
            .collect();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].channel.as_deref(), Some("orchard"));
        assert_eq!(cards[0].agents.len(), 7);

        // Sequential writers in separate worktrees are not a warning.
        let mut a = agent("x1", "one", "done", NOW - MIN);
        a.checkouts = vec![writes("/work/orchard-wt-a", NOW - 60 * MIN, NOW - 40 * MIN)];
        let mut b = agent("x2", "two", "running", NOW - MIN);
        b.checkouts = vec![writes("/work/orchard-wt-a", NOW - 30 * MIN, NOW - MIN)];
        let mut c = agent("x3", "three", "running", NOW - MIN);
        c.checkouts = vec![writes("/work/orchard-wt-b", NOW - 30 * MIN, NOW - MIN)];
        let v = World::new(vec![a, b, c]).view();
        assert!(v.cards.iter().all(|c| c.kind != "two_writers"));

        // Agents of two different leads in one checkout are caught as well.
        let mut p = agent("p1", "left", "running", NOW - MIN);
        p.checkouts = vec![writes("/work/harbard", NOW - 30 * MIN, NOW - MIN)];
        let mut q = agent("q1", "right", "running", NOW - MIN);
        q.checkouts = vec![writes("/work/harbard", NOW - 20 * MIN, NOW - 2 * MIN)];
        let mut other = lead(vec![q]);
        other.session = "99999999-2222-3333-4444-555555555555".into();
        let mut w = World::new(vec![p]);
        w.team.leads.push(other);
        let v = w.view();
        let card = v.cards.iter().find(|c| c.kind == "two_writers").unwrap();
        assert_eq!(card.agents, vec!["p1".to_string(), "q1".to_string()]);

        // The lead writing beside its own teammate counts as a writer.
        let mut solo = agent("s1", "solo", "running", NOW - MIN);
        solo.checkouts = vec![writes("/work/acme", NOW - 30 * MIN, NOW - MIN)];
        let mut panel = lead(vec![solo]);
        panel.lead_write_dirs = vec![writes("/work/acme/src", NOW - 20 * MIN, NOW - 10 * MIN)];
        let mut w = World::new(Vec::new());
        w.team = team_view(vec![panel]);
        let v = w.view();
        let card = v.cards.iter().find(|c| c.kind == "two_writers").unwrap();
        assert_eq!(card.agents, vec![format!("lead:{LEAD}"), "s1".to_string()]);
    }

    #[test]
    fn unfolded_write_dirs_place_an_agent_in_the_worktree_it_really_wrote_in() {
        // The team view folded both writers into one outer root; on disk they
        // are two worktrees of two repositories, so no warning and two
        // channels.
        let mut a = agent("a1", "one", "running", NOW - MIN);
        a.checkouts = vec![writes("/work/outer", NOW - 30 * MIN, NOW - MIN)];
        a.write_dirs = vec![writes("/work/orchard-wt-a/src", NOW - 30 * MIN, NOW - MIN)];
        let mut b = agent("a2", "two", "running", NOW - MIN);
        b.checkouts = vec![writes("/work/outer", NOW - 20 * MIN, NOW - MIN)];
        b.write_dirs = vec![writes(
            "/work/harbard-wt-b/crates",
            NOW - 20 * MIN,
            NOW - MIN,
        )];
        b.git_dirs = vec!["/work/harbard".into(), "/tmp/scratch".into()];
        let v = World::new(vec![a, b]).view();
        assert!(v.cards.iter().all(|c| c.kind != "two_writers"));
        let ids: Vec<&str> = v.channels.iter().map(|c| c.id.as_str()).collect();
        assert!(
            ids.contains(&"orchard") && ids.contains(&"harbard") && !ids.contains(&"outer"),
            "{ids:?}"
        );
        let two = v.agents.iter().find(|x| x.key == "a2").unwrap();
        assert_eq!(two.checkout.as_deref(), Some("/work/harbard-wt-b"));
        assert_eq!(two.branch.as_deref(), Some("feature"));
        assert_eq!(two.channels, vec!["harbard".to_string()]);
    }

    #[test]
    fn an_agent_of_a_dead_lead_is_lost_grey_and_raises_no_card() {
        // Running for hours, lead quiet for a day: nothing is known since.
        let mut w = World::new(vec![
            agent("a1", "legibility-qa", "running", NOW - 3 * 60 * MIN),
            agent("a2", "waiting-one", "idle", NOW - 3 * 60 * MIN),
        ]);
        w.team.leads[0].lead_last_ms = Some(NOW - 26 * 60 * MIN);
        let v = w.view();
        for a in &v.agents {
            assert!(a.lost, "{}", a.name);
            assert_eq!(a.presence, "finished");
            assert_eq!(a.dormant, None);
        }
        assert!(v.cards.iter().all(|c| c.kind != "dormant"));
        assert_eq!(v.unread.activity, 0);

        // The same agent with its lead alive (a record 10 minutes ago) is red.
        w.team.leads[0].lead_last_ms = Some(NOW - 10 * MIN);
        let v = w.view();
        let a = v.agents.iter().find(|a| a.key == "a1").unwrap();
        assert!(!a.lost);
        assert_eq!(a.dormant, Some("red"));
        assert!(open_cards(&v).iter().any(|c| c.kind == "dormant"));

        // A dead lead, but the agent itself wrote 50 minutes ago: still red.
        w.team = team_view(vec![lead(vec![agent(
            "a1",
            "recent",
            "running",
            NOW - 50 * MIN,
        )])]);
        w.team.leads[0].lead_last_ms = Some(NOW - 5 * 60 * MIN);
        let v = w.view();
        assert_eq!(v.agents[0].dormant, Some("red"));
        assert!(!v.agents[0].lost);

        // Under a live lead, an agent quiet for more than a day is lost too.
        w.team = team_view(vec![lead(vec![agent(
            "a1",
            "recall",
            "running",
            NOW - 29 * 60 * MIN,
        )])]);
        w.team.leads[0].lead_last_ms = Some(NOW - MIN);
        assert!(w.view().agents[0].lost);
        w.team.leads[0].agents[0].last_action_ms = Some(NOW - 145 * MIN);
        assert_eq!(w.view().agents[0].dormant, Some("red"));
        w.team = team_view(vec![lead(vec![agent(
            "a1",
            "recent",
            "running",
            NOW - 50 * MIN,
        )])]);
        w.team.leads[0].lead_last_ms = Some(NOW - 5 * 60 * MIN);

        // Two hours and one minute: lost. The thresholds come from config.
        w.team.leads[0].agents[0].last_action_ms = Some(NOW - 121 * MIN);
        assert!(w.view().agents[0].lost);
        w.desk.config.agent_recent_min = 180;
        assert!(!w.view().agents[0].lost);
    }

    #[test]
    fn an_overlap_of_writers_that_stopped_hours_ago_is_history_not_a_warning() {
        let mut a = agent("a1", "one", "done", NOW - 3 * 60 * MIN);
        a.checkouts = vec![writes(
            "/work/orchard",
            NOW - 5 * 60 * MIN,
            NOW - 3 * 60 * MIN,
        )];
        let mut b = agent("a2", "two", "done", NOW - 3 * 60 * MIN);
        b.checkouts = vec![writes(
            "/work/orchard",
            NOW - 4 * 60 * MIN,
            NOW - 3 * 60 * MIN,
        )];
        let mut w = World::new(vec![a.clone(), b.clone()]);
        let v = w.view();
        let card = v
            .cards
            .iter()
            .find(|c| c.kind == "two_writers")
            .expect("in Todo");
        assert!(!card.needs);
        assert!(open_cards(&v).is_empty());
        assert_eq!(v.unread.activity, 0);
        let ch = v.channels.iter().find(|c| c.id == "orchard").unwrap();
        assert!(ch.warnings.is_empty(), "no banner for history");

        // Both wrote within the last two hours: banner and Needs you.
        a.checkouts[0].last_ms = NOW - 30 * MIN;
        b.checkouts[0].last_ms = NOW - 20 * MIN;
        w.team = team_view(vec![lead(vec![a.clone(), b.clone()])]);
        let v = w.view();
        assert!(open_cards(&v).iter().any(|c| c.kind == "two_writers"));
        assert_eq!(
            v.channels
                .iter()
                .find(|c| c.id == "orchard")
                .unwrap()
                .warnings
                .len(),
            1
        );

        // Only one of them is recent: history again.
        a.checkouts[0].last_ms = NOW - 3 * 60 * MIN;
        a.checkouts[0].first_ms = NOW - 5 * 60 * MIN;
        b.checkouts[0].first_ms = NOW - 4 * 60 * MIN;
        w.team = team_view(vec![lead(vec![a, b])]);
        assert!(open_cards(&w.view()).is_empty());
    }

    #[test]
    fn protocol_flags_apply_only_after_the_configured_cutoff_repeats_always() {
        let mut a = agent("a1", "keychain", "running", NOW - MIN);
        let old = msg("out", "team-lead", NOW - 30 * MIN, "sin etiqueta, de antes");
        let mut old_long = msg(
            "out",
            "team-lead",
            NOW - 25 * MIN,
            "DONE · keychain · Largo de antes.",
        );
        old_long.chars = 5_000;
        let mut old_repeat = old.clone();
        old_repeat.ts_ms = NOW - 22 * MIN;
        old_repeat.repeat = true;
        let new = msg("out", "team-lead", NOW - 5 * MIN, "no label, from after");
        a.messages = vec![old, old_long, old_repeat, new];
        let mut w = World::new(vec![a]);
        w.desk.config.protocol_since_ms = NOW - 20 * MIN;
        let v = w.view();
        let m = of_agent(&v, "a1");
        assert!(m[0].flags.is_empty());
        assert!(m[1].flags.is_empty());
        assert_eq!(m[2].flags, vec!["repeat"]);
        assert_eq!(m[3].flags, vec!["unlabeled"]);
    }

    #[test]
    fn names_are_team_and_role_and_an_unnamed_agent_is_never_its_prompt() {
        // Today's odd names: cap136, "Codex adversarial review of n…",
        // harbard-dock, robots-builder, LD.
        const FIRST_MS: u64 = 1_794_068_760_000;
        let mut a = agent("aa18a5f8aaaa", "x", "running", NOW - MIN);
        a.name = None;
        a.kind = "subagent";
        a.description = Some("Codex adversarial review of orchard".into());
        a.brief = Some("Audit the acme token scopes against the RFC".into());
        a.first_ms = Some(FIRST_MS);
        let b = agent("bb000000bbbb", "robots-builder", "running", NOW - MIN);
        let c = agent("cc000000cccc", "robots-review", "running", NOW - MIN);
        let d = agent("dd000000dddd", "harbard-dock", "running", NOW - MIN);
        let e = agent("ee000000eeee", "cap136", "running", NOW - MIN);
        let mut w = World::new(vec![a, b, c, d, e]);
        w.team.leads[0].agents[1].messages =
            vec![msg("in", "team-lead", NOW - MIN, "Carry on with the plan.")];
        let v = w.view();
        let by = |k: &str| v.agents.iter().find(|x| x.key == k).unwrap().clone();
        assert_eq!(
            by("aa18a5f8aaaa").name,
            format!("lead · session {}", naming::hhmm_local(FIRST_MS))
        );
        assert!(!by("aa18a5f8aaaa").name.contains("Codex"));
        // The full id stays for Profile.
        assert_eq!(by("aa18a5f8aaaa").label, "general-purpose · aa18a5f8");
        assert_eq!(by("bb000000bbbb").name, "robots · builder");
        assert_eq!(by("cc000000cccc").name, "robots · reviewer");
        assert_eq!(
            (
                by("cc000000cccc").team.as_deref(),
                by("cc000000cccc").handle.as_str()
            ),
            (Some("robots"), "robots-review")
        );
        assert_eq!(by("dd000000dddd").name, "harbard · dock");
        assert_eq!(by("ee000000eeee").name, "lead · cap136");
        // The lead is the coordinator, never "LD".
        let lead = of_agent(&v, "bb000000bbbb");
        assert_eq!(lead[0].author, LEAD_AUTHOR);
    }

    #[test]
    fn the_operator_has_a_neutral_default_and_is_read_from_config() {
        let v = World::new(Vec::new()).view();
        assert_eq!(v.operator_initials, "DA");
        let desk = DeskState::default();
        assert_eq!(desk.config.operator_name, DEFAULT_OPERATOR_NAME);
        assert_eq!(desk.config.operator_initials, "OP");
        let parsed: DeskState = serde_json::from_str(
            r#"{"config":{"operator_name":"Karen","operator_initials":"KA"}}"#,
        )
        .unwrap();
        assert_eq!(
            (
                parsed.config.operator_name.as_str(),
                parsed.config.operator_initials.as_str()
            ),
            ("Karen", "KA")
        );
        assert_eq!(parsed.config.dormant_red_min, 45);
    }

    #[test]
    fn badges_count_only_what_meets_the_rules() {
        // An agent quiet for two days keeps its unread out of the badge; a
        // stale overlap and a resolved card stay out of the activity count.
        let mut old = agent("o1", "old", "done", NOW - 48 * 60 * MIN);
        old.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 48 * 60 * MIN,
            "DONE · old · Hecho.",
        )];
        let mut fresh = agent("f1", "fresh", "idle", NOW - MIN);
        fresh.messages = vec![msg("out", "team-lead", NOW - MIN, "BLOCKED · fresh · Yes?")];
        let v = World::new(vec![old, fresh]).view();
        assert_eq!(v.agents.iter().find(|a| a.key == "o1").unwrap().unread, 1);
        assert_eq!(v.unread.directs, 1);
        assert_eq!(v.unread.activity, 1);
    }

    fn receipt(sha: &str, verdict: Verdict, repo: &str) -> Receipt {
        Receipt {
            delivery_id: format!("d-{sha}"),
            repo: repo.into(),
            branch: "main".into(),
            delivered_sha: format!("{sha}{}", "0".repeat(40 - sha.len())),
            onto: "f".repeat(40),
            candidate: Some(format!("{sha}{}", "1".repeat(40 - sha.len()))),
            verdict,
            detail: String::new(),
            session: LEAD.into(),
            decided_ms: NOW - MIN,
        }
    }

    #[test]
    fn verified_needs_a_landed_receipt_never_the_agents_own_claim() {
        let mut a = agent("a1", "keychain", "done", NOW - MIN);
        a.checkouts = vec![writes(
            "/work/orchard-wt-identity",
            NOW - 60 * MIN,
            NOW - 30 * MIN,
        )];
        a.messages = vec![
            // Refusal: a claim of passing checks with no SHA at all.
            msg(
                "out",
                "team-lead",
                NOW - 25 * MIN,
                "DONE · keychain · Arreglos hechos, pruebas pasan, 505 pruebas pasan.",
            ),
            // A SHA that no receipt landed.
            msg(
                "out",
                "team-lead",
                NOW - 20 * MIN,
                "DONE · keychain · Commit 9f3c2ab ready, tests pass.",
            ),
            // A SHA a receipt landed in this repository.
            msg(
                "out",
                "team-lead",
                NOW - 10 * MIN,
                "DONE · keychain · Commit 4e5d6c7 integrado.",
            ),
            // The same landed SHA in a FINDING is not a delivery.
            msg(
                "out",
                "team-lead",
                NOW - 5 * MIN,
                "FINDING · keychain · 4e5d6c7 rompe algo.",
            ),
        ];
        let mut w = World::new(vec![a]);
        w.receipts = vec![
            receipt("4e5d6c7", Verdict::Landed, "/work/orchard"),
            receipt("9f3c2ab", Verdict::Rejected, "/work/orchard"),
        ];
        w.deliveries = vec![
            delivery_of("4e5d6c7", LEAD, NOW - 12 * MIN),
            delivery_of("9f3c2ab", LEAD, NOW - 12 * MIN),
        ];
        let v = w.view();
        let m = of_agent(&v, "a1");
        assert!(m[0].verified.is_none(), "a claim is not evidence");
        assert!(
            m[1].verified.is_none(),
            "a rejected receipt is not evidence"
        );
        let ok = m[2].verified.as_ref().expect("landed receipt");
        assert_eq!(ok.sha, "4e5d6c7");
        assert_eq!(ok.branch, "main");
        assert!(m[3].verified.is_none());
        let profile = &v.agents[0];
        assert_eq!((profile.deliveries, profile.verified_deliveries), (3, 1));

        // A landed receipt in another repository does not count.
        w.receipts = vec![receipt("4e5d6c7", Verdict::Landed, "/work/acme")];
        assert!(of_agent(&w.view(), "a1")[2].verified.is_none());
    }

    fn delivery_of(sha: &str, session: &str, created_ms: u64) -> Delivery {
        Delivery {
            id: format!("d-{sha}"),
            repo: "/work/orchard".into(),
            sha: format!("{sha}{}", "0".repeat(40 - sha.len())),
            branch: "main".into(),
            base: "f".into(),
            checks: "true".into(),
            prepare: String::new(),
            session: session.into(),
            harness: "claude-code".into(),
            created_ms,
        }
    }

    /// The reviewer's probe, inverted: a DONE that quotes a SHA an integrate
    /// landed two hours earlier ("Rebased onto …") is not evidence.
    #[test]
    fn review_probe_old_landed_sha_in_a_done_is_never_verified() {
        let mut a = agent("a1", "x", "done", NOW - MIN);
        a.checkouts = vec![writes("/work/orchard", NOW - 60 * MIN, NOW - 30 * MIN)];
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 60 * MIN,
            "DONE · x · Rebased onto 4e5d6c7, pruebas pasan.",
        )];
        let mut w = World::new(vec![a]);
        let mut landed = receipt("4e5d6c7", Verdict::Landed, "/work/orchard");
        landed.decided_ms = NOW - 3 * 60 * MIN;
        w.receipts = vec![landed];
        w.deliveries = vec![delivery_of("4e5d6c7", LEAD, NOW - 4 * 60 * MIN)];
        let v = w.view();
        assert!(of_agent(&v, "a1")[0].verified.is_none());
        assert_eq!(v.agents[0].verified_deliveries, 0);
    }

    /// AC3: the operator writes to a BLOCKED agent about something else, in
    /// Direct, without naming the question. The view hangs that note under
    /// the agent's last message, the BLOCKED, but it is not an answer.
    #[test]
    fn a_blocked_stays_open_when_operator_writes_without_naming_it() {
        let mut a = agent("a1", "rele", "idle", NOW - MIN);
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 30 * MIN,
            "BLOCKED · rele · Should I use the new API or the old one?",
        )];
        let mut w = World::new(vec![a]);
        w.notes.push(OperatorNote {
            id: "01ASIDE".into(),
            ts_ms: NOW - 10 * MIN,
            session: LEAD.into(),
            text: "Para rele: ¿Cómo vas con la exportación?".into(),
            ..Default::default()
        });
        let v = w.view();
        let blocked: Vec<_> = open_cards(&v)
            .into_iter()
            .filter(|c| c.kind == "blocked")
            .collect();
        assert_eq!(blocked.len(), 1, "an aside is not an answer");

        // The same note, sent as a reply to the BLOCKED: answered.
        let target = blocked[0].message.clone().unwrap();
        w.desk.threads.insert("01ASIDE".into(), target);
        let v = w.view();
        assert!(open_cards(&v).iter().all(|c| c.kind != "blocked"));
    }

    /// AC3: the agent speaking again (it decided, or asks something else)
    /// closes its BLOCKED: the question is no longer where it stands.
    #[test]
    fn the_agents_own_next_message_closes_its_blocked() {
        let mut a = agent("a1", "rele", "idle", NOW - MIN);
        a.messages = vec![
            msg(
                "out",
                "team-lead",
                NOW - 30 * MIN,
                "BLOCKED · rele · Should I use the new API or the old one?",
            ),
            msg(
                "out",
                "team-lead",
                NOW - 5 * MIN,
                "START · rele · Carrying on with the new API",
            ),
        ];
        let v = World::new(vec![a]).view();
        assert!(open_cards(&v).iter().all(|c| c.kind != "blocked"));
    }

    /// AC6: the BLOCKED of an agent nobody can reach any more (lost, or its
    /// lead not on the bus) leaves Needs you after two hours, and after
    /// the configured time when the operator sets one; it stays as history.
    #[test]
    fn an_unreachable_agents_blocked_ages_out_of_te_needs() {
        let blocked_at = |ago: u64| {
            let mut a = agent("a1", "coil-spring", "idle", NOW - ago);
            a.messages = vec![msg(
                "out",
                "team-lead",
                NOW - ago,
                "BLOCKED · resorte · Should I drop the old table?",
            )];
            a
        };
        let needs = |w: &World| {
            let v = w.view();
            v.cards
                .iter()
                .find(|c| c.kind == "blocked")
                .map(|c| c.needs)
        };
        // Its lead is not on the bus.
        let mut w = World::new(vec![blocked_at(60 * MIN)]);
        w.sessions.clear();
        assert_eq!(needs(&w), Some(true), "an hour: still asks");
        w.team = team_view(vec![lead(vec![blocked_at(3 * 60 * MIN)])]);
        assert_eq!(needs(&w), Some(false), "three hours: history");
        w.team = team_view(vec![lead(vec![blocked_at(60 * MIN)])]);
        w.desk.config.blocked_unreachable_min = 30;
        assert_eq!(needs(&w), Some(false), "configured to half an hour");

        // Reachable: no clock on it.
        let w = World::new(vec![blocked_at(3 * 60 * MIN)]);
        assert_eq!(needs(&w), Some(true));
    }

    /// The reviewer's probe, kept as it passed: any later message from the
    /// lead closes a BLOCKED, a structured `shutdown_request` included,
    /// which is the spec's letter ("no later message from the operator or the lead").
    #[test]
    fn review_probe_a_later_lead_shutdown_request_closes_a_blocked() {
        let mut a = agent("a1", "keychain", "idle", NOW - MIN);
        a.messages = vec![
            msg(
                "out",
                "team-lead",
                NOW - 20 * MIN,
                "BLOCKED · keychain · Yes or no?",
            ),
            msg(
                "in",
                "team-lead",
                NOW - 10 * MIN,
                r#"{"type":"shutdown_request","reason":"done"}"#,
            ),
        ];
        let v = World::new(vec![a]).view();
        assert!(open_cards(&v).iter().all(|c| c.kind != "blocked"));
    }

    #[test]
    fn a_sha_another_session_delivered_is_never_verified() {
        let mut a = agent("a1", "x", "done", NOW - MIN);
        a.checkouts = vec![writes("/work/orchard", NOW - 60 * MIN, NOW - 30 * MIN)];
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 20 * MIN,
            "DONE · x · Commit 4e5d6c7 integrado.",
        )];
        let mut w = World::new(vec![a]);
        // Landed after the message, but delivered by someone else's session.
        w.receipts = vec![receipt("4e5d6c7", Verdict::Landed, "/work/orchard")];
        w.deliveries = vec![delivery_of("4e5d6c7", "someone-else", NOW - 15 * MIN)];
        assert!(of_agent(&w.view(), "a1")[0].verified.is_none());
        // No delivery record at all: no evidence of who delivered it.
        w.deliveries.clear();
        assert!(of_agent(&w.view(), "a1")[0].verified.is_none());
        // Delivered by this agent's lead and landed after the DONE: verified.
        w.deliveries = vec![delivery_of("4e5d6c7", LEAD, NOW - 15 * MIN)];
        assert!(of_agent(&w.view(), "a1")[0].verified.is_some());
    }

    #[test]
    fn a_stale_lead_is_its_own_delivery_reason_and_the_note_waits() {
        let mut a = agent("a1", "keychain", "idle", NOW - MIN);
        a.checkouts = vec![writes("/work/orchard", NOW - 30 * MIN, NOW - MIN)];
        let mut w = World::new(vec![a]);
        let mut stale = session(LEAD, Liveness::Stale);
        stale.last_ms = NOW - 5 * 60 * MIN;
        w.sessions = vec![stale];
        let v = w.view();
        assert_eq!(v.agents[0].delivery, "stale");
        assert_eq!(v.agents[0].lead_quiet_ms, Some(5 * 60 * MIN));
        // The note is stored and waits; it is not presented as delivered.
        assert_eq!(reply_route(&v, "a1", false).unwrap().delivery, "stale");
        assert_eq!(channel_route(&v, "orchard", false), vec![LEAD.to_string()]);
    }

    #[test]
    fn replies_route_to_the_lead_session_only_when_the_bus_can_carry_them() {
        let mut a = agent("a1", "keychain", "running", NOW - MIN);
        a.checkouts = vec![writes("/work/orchard", NOW - 30 * MIN, NOW - MIN)];
        let mut w = World::new(vec![a]);
        let v = w.view();
        assert_eq!(reply_route(&v, "a1", false).unwrap().lead_session, LEAD);
        assert_eq!(channel_route(&v, "orchard", false), vec![LEAD.to_string()]);
        assert_eq!(
            reply_route(&v, "nobody", false).unwrap_err(),
            "unknown_agent"
        );

        w.sessions = vec![session(LEAD, Liveness::Ended)];
        let v = w.view();
        assert_eq!(reply_route(&v, "a1", false).unwrap_err(), "ended");
        assert!(channel_route(&v, "orchard", false).is_empty());
        // "Enviar samemente": stored for the ended lead, honestly worded.
        assert_eq!(reply_route(&v, "a1", true).unwrap().lead_session, LEAD);
        assert_eq!(channel_route(&v, "orchard", true), vec![LEAD.to_string()]);
        assert_eq!(
            v.channels.iter().find(|c| c.id == "orchard").unwrap().reach,
            "none"
        );

        w.sessions.clear();
        let v = w.view();
        assert_eq!(reply_route(&v, "a1", false).unwrap_err(), "not_on_bus");
        assert!(v.agents.iter().all(|a| !a.can_stop));
    }

    #[test]
    fn unread_counts_agent_messages_after_the_read_mark_and_the_baseline() {
        let mut a = agent("a1", "keychain", "running", NOW - MIN);
        a.messages = vec![
            msg(
                "out",
                "team-lead",
                NOW - 30 * MIN,
                "START · keychain · Uno.",
            ),
            msg("in", "team-lead", NOW - 25 * MIN, "ok"),
            msg(
                "out",
                "team-lead",
                NOW - 10 * MIN,
                "BLOCKED · keychain · Dos.",
            ),
        ];
        let mut w = World::new(vec![a]);
        assert_eq!(w.view().unread.directs, 2);
        w.desk.baseline_ms = NOW - 20 * MIN;
        assert_eq!(w.view().unread.directs, 1);
        w.desk.mark_read("a1", NOW - 10 * MIN);
        assert_eq!(w.view().agents[0].unread, 0);
    }

    #[test]
    fn channel_notes_show_once_and_a_saved_item_is_kept_once() {
        let mut a = agent("a1", "keychain", "running", NOW - MIN);
        a.checkouts = vec![writes("/work/orchard", NOW - 30 * MIN, NOW - MIN)];
        let mut w = World::new(vec![a]);
        for (i, s) in ["s1", "s2"].iter().enumerate() {
            w.notes.push(OperatorNote {
                id: format!("01C{i}"),
                ts_ms: NOW - MIN,
                session: (*s).into(),
                text: "En #orchard: todos a la vez".into(),
                ..Default::default()
            });
        }
        let v = w.view();
        let notes: Vec<&Message> = v.messages.iter().filter(|m| m.from == "operator").collect();
        assert_eq!(notes.len(), 1);
        assert_eq!(notes[0].channels, vec!["orchard".to_string()]);

        let mut desk = DeskState::default();
        let item = SavedItem {
            id: "m1".into(),
            kind: "message".into(),
            saved_ms: NOW,
            agent: Some("keychain".into()),
            channel: Some("orchard".into()),
            text: "Uno".into(),
            ts_ms: Some(NOW - MIN),
            path: None,
        };
        desk.save_item(item.clone());
        desk.save_item(SavedItem {
            saved_ms: NOW + 1,
            ..item
        });
        assert_eq!(desk.saved.len(), 1);
        assert_eq!(desk.saved[0].saved_ms, NOW);
        desk.unsave("m1");
        assert!(desk.saved.is_empty());
    }

    #[test]
    fn desk_maps_are_capped_keeping_the_newest() {
        let mut desk = DeskState::default();
        for i in 0..(DESK_MAP_MAX as u64 + 50) {
            desk.resolve(&format!("card-{i:05}"), NOW + i);
            desk.mark_read(&format!("agent-{i:05}"), NOW + i);
            desk.link_thread(&format!("note-{i:05}"), "m");
        }
        assert_eq!(desk.resolved.len(), DESK_MAP_MAX);
        assert_eq!(desk.reads.len(), DESK_MAP_MAX);
        assert_eq!(desk.threads.len(), DESK_MAP_MAX);
        // The oldest marks go first.
        assert!(!desk.resolved.contains_key("card-00000"));
        assert!(
            desk.resolved
                .contains_key(&format!("card-{:05}", DESK_MAP_MAX as u64 + 49))
        );
        assert!(!desk.reads.contains_key("agent-00000"));
    }

    #[test]
    fn build_dirs_are_read_only_when_declared() {
        assert_eq!(
            declared_build_dir(
                "Worktree x, build dir harbard-wt-chat/target (own CARGO_TARGET_DIR)"
            ),
            Some("harbard-wt-chat/target".into())
        );
        assert_eq!(
            declared_build_dir("CARGO_TARGET_DIR=/tmp/t cargo test"),
            Some("/tmp/t".into())
        );
        assert_eq!(declared_build_dir("Trabajo en orchard-wt-identity."), None);
    }

    #[test]
    fn deliveries_are_pending_until_a_receipt_names_them() {
        let mut w = World::new(Vec::new());
        w.deliveries = vec![
            Delivery {
                id: "d-4e5d6c7".into(),
                repo: "/work/orchard".into(),
                sha: "4e5d6c7".into(),
                branch: "main".into(),
                base: "f".into(),
                checks: "true".into(),
                prepare: String::new(),
                session: LEAD.into(),
                harness: "claude-code".into(),
                created_ms: NOW - 3 * MIN,
            },
            Delivery {
                id: "d-new".into(),
                repo: "/work/orchard-wt-x".into(),
                sha: "abc1234".into(),
                branch: "main".into(),
                base: "f".into(),
                checks: "true".into(),
                prepare: String::new(),
                session: LEAD.into(),
                harness: "claude-code".into(),
                created_ms: NOW - MIN,
            },
        ];
        w.receipts = vec![receipt("4e5d6c7", Verdict::Landed, "/work/orchard")];
        let v = w.view();
        assert_eq!(v.deliveries[0].verdict, "pending");
        assert_eq!(v.deliveries[0].channel.as_deref(), Some("orchard"));
        assert_eq!(v.deliveries[1].verdict, "landed");
    }

    // -----------------------------------------------------------------------
    // Faces: what the workspace lays over the transcript's activity, the
    // microstate thresholds, and one test per reaction event.
    // -----------------------------------------------------------------------

    fn with_face(
        mut row: AgentRow,
        state: &'static str,
        since: u64,
        detail: Option<&str>,
    ) -> AgentRow {
        row.activity = Activity::new(state, since, detail.map(str::to_string), since);
        row
    }

    fn face_of(view: &WorkspaceView, key: &str) -> Activity {
        view.agents
            .iter()
            .find(|a| a.key == key)
            .unwrap()
            .activity
            .clone()
    }

    #[test]
    fn face_asleep_follows_the_dormant_rule_on_a_fake_clock() {
        let last = NOW;
        let row = with_face(
            agent("a1", "keychain", "running", last),
            "running",
            last,
            Some("cargo build"),
        );
        let mut w = World::new(vec![row]);
        let at = |w: &mut World, minutes: u64| {
            w.now = last + minutes * MIN;
            let f = face_of(&w.view(), "a1");
            (f.state, f.since_ms)
        };
        assert_eq!(at(&mut w, 10), ("running", last));
        // A call in flight is quiet, not asleep, until the red threshold.
        assert_eq!(at(&mut w, 16), ("running", last));
        assert_eq!(at(&mut w, 50), ("asleep", last));
        // An agent of a dead lead is lost: nothing is known, a plain face.
        w.team.leads[0].lead_last_ms = Some(last - 10 * 60 * MIN);
        let mut row = with_face(
            agent("a1", "keychain", "running", last - 3 * 60 * MIN),
            "thinking",
            last,
            None,
        );
        row.seen_ms = last - 3 * 60 * MIN;
        w.team.leads[0].agents = vec![row];
        w.now = last;
        let v = w.view();
        assert!(v.agents[0].lost);
        assert_eq!(face_of(&v, "a1").state, "normal");
    }

    #[test]
    fn face_waiting_dozes_and_goes_dormant_at_fifteen_minutes() {
        let last = NOW;
        let mut row = agent("a1", "acme-runner", "idle", last);
        row.state_reason = "turn_ended";
        let row = with_face(row, "waiting", last, None);
        let mut w = World::new(vec![row]);
        let at = |w: &mut World, minutes: u64| {
            w.now = last + minutes * MIN;
            let v = w.view();
            let a = &v.agents[0];
            (
                a.activity.state,
                a.presence,
                a.dormant,
                open_cards(&v).iter().any(|c| c.kind == "dormant"),
            )
        };
        // Amber from the first minute: it is waiting, not working.
        assert_eq!(at(&mut w, 1), ("waiting", "waiting", None, false));
        assert_eq!(at(&mut w, 6), ("waiting", "waiting", None, false));
        assert_eq!(at(&mut w, 16), ("asleep", "waiting", Some("amber"), false));
        assert_eq!(at(&mut w, 46), ("asleep", "waiting", Some("red"), true));
        // Without the wait claim an idle agent never dozes.
        w.team.leads[0].agents = vec![agent("a1", "acme-runner", "idle", last)];
        assert_eq!(at(&mut w, 120), ("normal", "waiting", None, false));
    }

    #[test]
    fn face_blocked_while_the_blocked_card_is_open() {
        let mut a = agent("a1", "keychain-review", "idle", NOW - MIN);
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 7 * MIN,
            "BLOCKED · keychain-review · Can it ask for the password twice?",
        )];
        let mut w = World::new(vec![a.clone()]);
        let f = face_of(&w.view(), "a1");
        assert_eq!(
            (f.state, f.since_ms, f.detail.as_deref()),
            ("blocked", NOW - 7 * MIN, None)
        );
        // Blocked wins over dormant.
        w.now = NOW + 60 * MIN;
        assert_eq!(face_of(&w.view(), "a1").state, "blocked");
        w.now = NOW;
        // Marked resolved: the card is closed, so is the face.
        let card = w.view().cards[0].id.clone();
        w.desk.resolve(&card, NOW);
        assert_eq!(face_of(&w.view(), "a1").state, "normal");
        w.desk.reopen(&card);
        // Answered by the lead: no longer blocked.
        let mut answered = a;
        answered
            .messages
            .push(msg("in", "team-lead", NOW - 2 * MIN, "Yes."));
        w.team = team_view(vec![lead(vec![answered])]);
        assert_eq!(face_of(&w.view(), "a1").state, "normal");
    }

    /// Criterion 2: the robot's "verified" face follows the same evidence
    /// rule as the message pill, never the agent's own words.
    #[test]
    fn face_verified_only_on_the_evidence_rule_never_on_a_claim() {
        let mut a = agent("a1", "keychain", "done", NOW - MIN);
        a.checkouts = vec![writes("/work/orchard", NOW - 60 * MIN, NOW - 30 * MIN)];
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 20 * MIN,
            "DONE · keychain · Commit 4e5d6c7 verified, all tests pass.",
        )];
        let mut w = World::new(vec![a]);
        // The claim alone: finished, not verified.
        assert_eq!(face_of(&w.view(), "a1").state, "finished");
        // A rejected receipt: still not.
        w.receipts = vec![receipt("4e5d6c7", Verdict::Rejected, "/work/orchard")];
        w.deliveries = vec![delivery_of("4e5d6c7", LEAD, NOW - 15 * MIN)];
        assert_eq!(face_of(&w.view(), "a1").state, "finished");
        // Landed, but delivered by another session.
        w.receipts = vec![receipt("4e5d6c7", Verdict::Landed, "/work/orchard")];
        w.deliveries = vec![delivery_of("4e5d6c7", "someone-else", NOW - 15 * MIN)];
        assert_eq!(face_of(&w.view(), "a1").state, "finished");
        // Landed in another repository.
        w.receipts = vec![receipt("4e5d6c7", Verdict::Landed, "/work/acme")];
        w.deliveries = vec![delivery_of("4e5d6c7", LEAD, NOW - 15 * MIN)];
        assert_eq!(face_of(&w.view(), "a1").state, "finished");
        // The evidence rule holds: this lead delivered it, it landed here
        // after the DONE. Then, and only then, the face is verified.
        w.receipts = vec![receipt("4e5d6c7", Verdict::Landed, "/work/orchard")];
        let v = w.view();
        assert!(of_agent(&v, "a1")[0].verified.is_some());
        let f = face_of(&v, "a1");
        assert_eq!((f.state, f.since_ms), ("verified", NOW - MIN));
        // The face never says verified unless a message carries evidence.
        for a in &v.agents {
            if a.activity.state == "verified" {
                assert!(
                    v.messages
                        .iter()
                        .any(|m| m.agent == a.key && m.verified.is_some())
                );
            }
        }
    }

    #[test]
    fn microstate_thresholds_come_from_the_desk_config() {
        let mut w = World::new(vec![agent("a1", "x", "running", NOW)]);
        let t = w.view().thresholds;
        assert_eq!(
            (
                t.thinking_long_ms,
                t.waiting_sleepy_ms,
                t.amber_ms,
                t.running_long_ms,
                t.red_ms
            ),
            (2 * MIN, 5 * MIN, 15 * MIN, 10 * MIN, 45 * MIN)
        );
        w.desk.config = serde_json::from_str(
            r#"{"thinking_long_min":3,"waiting_sleepy_min":4,"running_long_min":20,"dormant_amber_min":12,"dormant_red_min":30}"#,
        )
        .unwrap();
        let t = w.view().thresholds;
        assert_eq!(
            (
                t.thinking_long_ms,
                t.waiting_sleepy_ms,
                t.amber_ms,
                t.running_long_ms,
                t.red_ms
            ),
            (3 * MIN, 4 * MIN, 12 * MIN, 20 * MIN, 30 * MIN)
        );
        // An older desk file without them reads the defaults.
        let old: DeskConfig = serde_json::from_str(r#"{"dormant_amber_min":15}"#).unwrap();
        assert_eq!(
            (
                old.thinking_long_min,
                old.waiting_sleepy_min,
                old.running_long_min
            ),
            (2, 5, 10)
        );
    }

    fn reactions(view: &WorkspaceView, key: &str) -> Vec<(&'static str, u64)> {
        let mut r: Vec<_> = view
            .agents
            .iter()
            .find(|a| a.key == key)
            .unwrap()
            .reactions
            .iter()
            .map(|r| (r.kind, r.at_ms))
            .collect();
        r.sort();
        r
    }

    /// An agent that started long ago, so `boot` stays out of the way.
    fn veteran(id: &str, name: &str, state: &'static str) -> AgentRow {
        let mut a = agent(id, name, state, NOW - 5 * MIN);
        a.first_ms = Some(NOW - 5 * 60 * MIN);
        a
    }

    #[test]
    fn reaction_glance_on_a_new_message_from_the_agent() {
        let mut a = veteran("a1", "keychain", "running");
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 10_000,
            "FINDING · keychain · algo",
        )];
        let mut w = World::new(vec![a]);
        assert_eq!(reactions(&w.view(), "a1"), vec![("glance", NOW - 10_000)]);
        // Past the window it is history: never replayed.
        w.now = NOW + REACTION_WINDOW_MS;
        assert!(reactions(&w.view(), "a1").is_empty());
    }

    #[test]
    fn reaction_spark_when_the_operator_reply_reaches_the_lead() {
        let mut a = veteran("a1", "keychain", "running");
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 30 * MIN,
            "FINDING · keychain · algo",
        )];
        let mut w = World::new(vec![a]);
        w.notes.push(OperatorNote {
            id: "01N".into(),
            ts_ms: NOW - 5_000,
            session: LEAD.into(),
            text: "Para keychain: adelante".into(),
            ..Default::default()
        });
        assert_eq!(reactions(&w.view(), "a1"), vec![("spark", NOW - 5_000)]);
        // A lead the bus cannot reach got nothing: no spark.
        w.sessions = vec![session(LEAD, Liveness::Ended)];
        assert!(reactions(&w.view(), "a1").is_empty());
    }

    #[test]
    fn reaction_hop_on_a_verified_delivery() {
        let mut a = veteran("a1", "keychain", "done");
        a.checkouts = vec![writes("/work/orchard", NOW - 60 * MIN, NOW - 30 * MIN)];
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 20 * MIN,
            "DONE · keychain · Commit 4e5d6c7 integrado.",
        )];
        let mut w = World::new(vec![a]);
        let mut landed = receipt("4e5d6c7", Verdict::Landed, "/work/orchard");
        landed.decided_ms = NOW - 3_000;
        w.receipts = vec![landed];
        w.deliveries = vec![delivery_of("4e5d6c7", LEAD, NOW - 15 * MIN)];
        assert_eq!(reactions(&w.view(), "a1"), vec![("hop", NOW - 3_000)]);
        // A claim with no receipt never hops.
        w.receipts.clear();
        assert!(reactions(&w.view(), "a1").is_empty());
    }

    #[test]
    fn reaction_shake_on_a_new_blocked() {
        let mut a = veteran("a1", "keychain", "idle");
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 4_000,
            "BLOCKED · keychain · Yes or no?",
        )];
        let v = World::new(vec![a]).view();
        assert_eq!(
            reactions(&v, "a1"),
            vec![("glance", NOW - 4_000), ("shake", NOW - 4_000)]
        );
    }

    #[test]
    fn reaction_boot_when_a_new_agent_appears() {
        let mut a = agent("a1", "keychain", "running", NOW - 2_000);
        a.first_ms = Some(NOW - 8_000);
        let mut w = World::new(vec![a]);
        assert_eq!(reactions(&w.view(), "a1"), vec![("boot", NOW - 8_000)]);
        w.now = NOW + REACTION_WINDOW_MS;
        assert!(reactions(&w.view(), "a1").is_empty());
    }

    #[test]
    fn the_activity_slice_carries_faces_without_messages_and_no_secret_detail() {
        let cmd =
            "export API_TOKEN=tok_FAKE0123456789abcdef; mysql --password=hunter2 -h db.internal";
        let call =
            crate::activity::classify("Bash", &serde_json::json!({ "command": cmd }), NOW - MIN);
        let mut row = agent("a1", "keychain", "running", NOW - MIN);
        row.activity = Activity::new(call.kind.state(), call.started_ms, call.detail, NOW - MIN);
        row.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 30 * MIN,
            "FINDING · keychain · long message text",
        )];
        let v = World::new(vec![row]).view();
        let slice = ActivityView::of(&v);
        assert_eq!(slice.agents.len(), 1);
        assert_eq!(slice.agents[0].activity.detail.as_deref(), Some("mysql"));
        let full = serde_json::to_string(&v).unwrap();
        let small = serde_json::to_string(&slice).unwrap();
        assert!(
            !small.contains("texto largo"),
            "no message text in the slice"
        );
        assert!(small.len() < full.len());
        for json in [&full, &small] {
            for secret in [
                "tok_FAKE",
                "hunter2",
                "API_TOKEN",
                "db.internal",
                "password",
            ] {
                assert!(!json.contains(secret), "{secret} reached the API");
            }
        }
    }

    // The review's cases (harbard-robots-review.md, M2, M3) and the Dock's
    // two counts in the slice.

    #[test]
    fn a_call_in_flight_is_never_asleep_at_amber_only_at_red() {
        let start = NOW;
        let row = with_face(
            agent("a1", "long-tests", "running", start),
            "testing",
            start,
            Some("cargo test"),
        );
        let mut w = World::new(vec![row]);
        let at = |w: &mut World, minutes: u64| {
            w.now = start + minutes * MIN;
            face_of(&w.view(), "a1").state
        };
        assert_eq!(at(&mut w, 16), "testing");
        assert_eq!(at(&mut w, 44), "testing");
        assert_eq!(at(&mut w, 46), "asleep");
        // A quiet turn with nothing in flight still dozes at amber.
        w.team.leads[0].agents = vec![with_face(
            agent("a1", "quiet", "running", start),
            "thinking",
            start,
            None,
        )];
        assert_eq!(at(&mut w, 16), "asleep");
    }

    #[test]
    fn a_blocked_face_carries_no_message_text() {
        let mut a = agent("a1", "blocked-text", "idle", NOW - MIN);
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 2 * MIN,
            "BLOCKED · blocked-text · the API rejects GH_TOKEN=ghp_FAKEFAKEFAKE0123456789 with 401",
        )];
        let v = World::new(vec![a]).view();
        let f = face_of(&v, "a1");
        assert_eq!((f.state, f.detail.as_deref()), ("blocked", None));
        let slice = serde_json::to_string(&ActivityView::of(&v)).unwrap();
        assert!(
            !slice.contains("ghp_FAKE") && !slice.contains("rejects"),
            "{slice}"
        );
    }

    #[test]
    fn the_slice_carries_the_needs_and_directs_counts() {
        let mut a = agent("a1", "keychain", "idle", NOW - MIN);
        // The FINDING first: a later word of the agent's would close the
        // BLOCKED.
        a.messages = vec![
            msg(
                "out",
                "team-lead",
                NOW - 2 * MIN,
                "FINDING · keychain · algo",
            ),
            msg(
                "out",
                "team-lead",
                NOW - MIN,
                "BLOCKED · keychain · Yes or no?",
            ),
        ];
        let v = World::new(vec![a]).view();
        let slice: serde_json::Value = serde_json::to_value(ActivityView::of(&v)).unwrap();
        assert_eq!(slice["needs"], v.unread.activity);
        assert_eq!(slice["directs"], v.unread.directs);
        assert_eq!(slice["needs"], 1);
        assert_eq!(slice["directs"], 2);
    }

    /// Round 2, R2-a: while a call runs, the presence dot agrees with the
    /// face: no amber at 15 min, red (with its card) only at red_ms. A quiet
    /// turn with nothing in flight keeps amber.
    #[test]
    fn the_dot_of_a_call_in_flight_agrees_with_its_face() {
        let start = NOW;
        let row = with_face(
            agent("a1", "long-tests", "running", start),
            "testing",
            start,
            Some("cargo test"),
        );
        let mut w = World::new(vec![row]);
        let at = |w: &mut World, minutes: u64| {
            w.now = start + minutes * MIN;
            let v = w.view();
            let a = &v.agents[0];
            (
                a.dormant,
                a.activity.state,
                open_cards(&v).iter().any(|c| c.kind == "dormant"),
            )
        };
        assert_eq!(at(&mut w, 16), (None, "testing", false));
        assert_eq!(at(&mut w, 44), (None, "testing", false));
        assert_eq!(at(&mut w, 46), (Some("red"), "asleep", true));
        w.team.leads[0].agents = vec![with_face(
            agent("a1", "quiet", "running", start),
            "thinking",
            start,
            None,
        )];
        assert_eq!(at(&mut w, 16), (Some("amber"), "asleep", false));
    }

    // -----------------------------------------------------------------------
    // Messages v2, attachments and the operator's notes (protocolo humano)
    // -----------------------------------------------------------------------

    fn v2(dir: &'static str, ts: u64, text: &str) -> MessageRow {
        let first = team::first_line(text).unwrap();
        let mut m = msg(dir, "team-lead", ts, &first);
        m.body = human::technical_body(text);
        m.paths = human::path_tokens(text);
        m.chars = text.chars().count();
        m
    }

    #[test]
    fn the_human_line_is_the_message_and_the_body_its_technical_detail() {
        let mut a = agent("a1", "harbard-view", "idle", NOW - MIN);
        a.messages = vec![
            v2(
                "out",
                NOW - 3 * MIN,
                "DONE · harbard-view · You can now see the console screenshots.\n\nRebased onto main; head 1582702.\nInforme /work/r.md",
            ),
            // v1: no blank line, reads as it always did.
            v2(
                "out",
                NOW - 2 * MIN,
                "DONE · harbard-view · Done.\nsecond line of v1",
            ),
        ];
        let v = World::new(vec![a]).view();
        let m = of_agent(&v, "a1");
        assert_eq!(m[0].text, "You can now see the console screenshots.");
        assert_eq!(
            m[0].body.as_deref(),
            Some("Rebased onto main; head 1582702.\nInforme /work/r.md")
        );
        // The jargon is in the body, where it belongs: no chip.
        assert!(m[0].flags.is_empty(), "{:?}", m[0].flags);
        assert_eq!(m[1].text, "Done.");
        assert_eq!(m[1].body, None);
        assert!(m[1].flags.is_empty());
        let wire = serde_json::to_string(&v).unwrap();
        assert!(!wire.contains("second line of v1"), "{wire}");
    }

    #[test]
    fn a_jargon_human_line_gets_the_chip_with_its_reasons() {
        let mut a = agent("a1", "harbard-strip", "idle", NOW - MIN);
        a.messages = vec![
            msg(
                "out",
                "team-lead",
                NOW - 5 * MIN,
                "DONE · harbard-strip · rebased onto robots fcf628c; the new head is 1582702, frozen",
            ),
            msg(
                "out",
                "team-lead",
                NOW - 4 * MIN,
                "FINDING · harbard-strip · Cerrados R2-N1 a N6 y R2-a",
            ),
            msg(
                "out",
                "team-lead",
                NOW - 3 * MIN,
                "DONE · harbard-strip · You can now try the new panel; it loads faster.",
            ),
            // The lead's own words to the agent are not the agent's line.
            msg(
                "in",
                "team-lead",
                NOW - 2 * MIN,
                "Rebase onto main and freeze.",
            ),
        ];
        let v = World::new(vec![a]).view();
        let m = of_agent(&v, "a1");
        assert_eq!(m[0].flags, vec!["jargon"]);
        assert_eq!(m[0].jargon, vec!["sha", "word"]);
        assert_eq!(m[1].flags, vec!["jargon"]);
        assert_eq!(m[1].jargon, vec!["review_code"]);
        assert!(m[2].flags.is_empty() && m[2].jargon.is_empty());
        assert!(m[3].flags.is_empty());
        // History written before the rule is not repainted.
        let mut w = World::new(vec![agent("a2", "old", "idle", NOW - MIN)]);
        w.team.leads[0].agents[0].messages = vec![msg(
            "out",
            "team-lead",
            NOW - 5 * MIN,
            "DONE · old · merged fcf628c",
        )];
        w.desk.config.protocol_since_ms = NOW - MIN;
        assert!(of_agent(&w.view(), "a2")[0].flags.is_empty());
    }

    /// Another session's agent ignores the protocol: its English line gets a
    /// plain one, history included (no protocol date involved); the
    /// original stays as the text, and a plain line gets none.
    #[test]
    fn an_english_line_gets_a_readable_line() {
        let mut a = agent("a1", "cap146", "idle", NOW - MIN);
        a.messages = vec![
            msg(
                "out",
                "team-lead",
                NOW - 5 * MIN,
                "CAP-146 is done: head acb3816f on fix/runner-pg-env-signatures, pushed.",
            ),
            msg(
                "out",
                "team-lead",
                NOW - 3 * MIN,
                "DONE · cap146 · You can now try the new panel.",
            ),
        ];
        let mut w = World::new(vec![a]);
        w.desk.config.protocol_since_ms = NOW;
        let v = w.view();
        let m = of_agent(&v, "a1");
        let line = m[0].readable.as_ref().expect("a readable line");
        assert_eq!(line[0].text, "CAP-146 done and pushed");
        assert!(m[0].text.starts_with("CAP-146 is done: head acb3816f"));
        assert_eq!(m[1].readable, None);
        // Both are deliveries: the console groups a run of them.
        assert_eq!(
            (m[0].shape, m[1].shape),
            (Some("delivery"), Some("delivery"))
        );
    }

    #[test]
    fn the_lead_is_the_coordinator() {
        let mut a = agent("a1", "keychain", "idle", NOW - MIN);
        a.messages = vec![msg("in", "team-lead", NOW - MIN, "Carry on with the plan.")];
        let v = World::new(vec![a]).view();
        let m = of_agent(&v, "a1");
        assert_eq!((m[0].from, m[0].author.as_str()), ("lead", LEAD_AUTHOR));
    }

    fn attached(path: &str) -> Option<Attachment> {
        let rest = path.strip_prefix("/h/.harbard/attachments/")?;
        let (dir, name) = rest.split_once('/')?;
        let kind = match name.rsplit('.').next()? {
            "png" => "image",
            "md" => "markdown",
            "html" => "html",
            _ => return None,
        };
        Some(Attachment {
            id: format!("{dir}/{name}"),
            name: name.into(),
            kind,
            label: name.split('.').next().map(str::to_string),
            bytes: 10,
            path: path.into(),
        })
    }

    #[test]
    fn a_reply_on_the_bus_shows_what_it_attached_and_not_its_path() {
        let mut w = channel_world();
        w.notes
            .push(note("01ASK", NOW - 2 * MIN, "En #harbard: ¿Cómo vas?"));
        w.notes[0].trail.replies.push(BusReply {
            id: "01VIEW".into(),
            ts_ms: NOW - MIN,
            session: LEAD.into(),
            text: "Here it is.\n\n/h/.harbard/attachments/01R/status.md".into(),
        });
        let v = build(&Inputs {
            team: &w.team,
            sessions: &w.sessions,
            notes: &w.notes,
            bus_notes: &w.bus_notes,
            temp_dirs: &[],
            names: &w.names,
            deliveries: &w.deliveries,
            receipts: &w.receipts,
            desk: &w.desk,
            resolve: &resolver,
            attachment: &attached,
            now_ms: w.now,
            operator: "Dana",
            horizon_ms: ACTIVE_MS,
            keep_ended: false,
            in_workspace: &|_| true,
            known_repos: &[],
            rooms: &[],
            room_notes: &[],
            archived_repos: &BTreeSet::new(),
            workspace: None,
        });
        let reply = by_id(&v, "reply:01VIEW");
        let ids: Vec<&str> = reply.attachments.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(ids, vec!["01R/status.md"]);
        assert_eq!(
            reply.body, None,
            "the path must not repeat under Technical details"
        );
    }

    #[test]
    fn attached_files_render_as_attachments_and_html_stays_a_card() {
        let mut a = agent("a1", "harbard-view", "idle", NOW - MIN);
        a.messages = vec![v2(
            "out",
            NOW - MIN,
            "DONE · harbard-view · See the before and after.\n\n/h/.harbard/attachments/01A/before.png\n/h/.harbard/attachments/01B/after.png\n/h/.harbard/attachments/01C/report.md\n/h/.harbard/attachments/01D/page.html\n/work/otro.md",
        )];
        let mut w = World::new(vec![a]);
        w.team.leads[0].agents[0].messages[0].report = Some("/work/otro.md".into());
        let v = build(&Inputs {
            team: &w.team,
            sessions: &w.sessions,
            notes: &w.notes,
            bus_notes: &w.bus_notes,
            temp_dirs: &[],
            names: &w.names,
            deliveries: &w.deliveries,
            receipts: &w.receipts,
            desk: &w.desk,
            resolve: &resolver,
            attachment: &attached,
            now_ms: w.now,
            operator: "Dana",
            horizon_ms: ACTIVE_MS,
            keep_ended: false,
            in_workspace: &|_| true,
            known_repos: &[],
            rooms: &[],
            room_notes: &[],
            archived_repos: &BTreeSet::new(),
            workspace: None,
        });
        let m = of_agent(&v, "a1");
        let ids: Vec<&str> = m[0].attachments.iter().map(|a| a.id.as_str()).collect();
        assert_eq!(
            ids,
            vec!["01A/before.png", "01B/after.png", "01C/report.md"]
        );
        // HTML is never rendered in the console: a card the OS opens. A
        // path outside the attachments folder keeps today's card.
        assert_eq!(
            m[0].files,
            vec![
                "/work/otro.md".to_string(),
                "/h/.harbard/attachments/01D/page.html".to_string()
            ]
        );
        assert!(file_set(&v).contains("/h/.harbard/attachments/01D/page.html"));
    }

    fn note(id: &str, ts: u64, text: &str) -> OperatorNote {
        OperatorNote {
            id: id.into(),
            ts_ms: ts,
            session: LEAD.into(),
            text: text.into(),
            ..Default::default()
        }
    }

    fn channel_world() -> World {
        let mut a = agent("a1", "keychain", "idle", NOW - 30 * MIN);
        a.checkouts = vec![writes("/work/harbard", NOW - 60 * MIN, NOW - 30 * MIN)];
        World::new(vec![a])
    }

    fn state_of(v: &WorkspaceView, id: &str) -> &'static str {
        by_id(v, id).delivery.as_ref().unwrap().state
    }

    #[test]
    fn each_operator_note_says_sent_delivered_read_or_answered() {
        let mut w = channel_world();
        let text = "En #harbard: Probando, quiero saber si esto realmente funciona ya";
        w.notes.push(note("01SENT", NOW - 3 * MIN, text));
        let v = w.view();
        assert_eq!(state_of(&v, "note:01SENT"), "sent");
        let d = by_id(&v, "note:01SENT").delivery.clone().unwrap();
        assert_eq!(
            (d.sessions, d.delivered, d.to_lead),
            (vec![LEAD.to_string()], 0, true)
        );

        w.notes[0].trail.delivered = true;
        assert_eq!(state_of(&w.view(), "note:01SENT"), "delivered");

        w.notes[0].trail.acked_ms = Some(NOW - 2 * MIN);
        assert_eq!(state_of(&w.view(), "note:01SENT"), "read");

        w.notes[0].trail.replies.push(BusReply {
            id: "01REPLY".into(),
            ts_ms: NOW - MIN,
            session: LEAD.into(),
            text: "Yes, it works: your message reached me.\n\nvia brief".into(),
        });
        let v = w.view();
        assert_eq!(state_of(&v, "note:01SENT"), "answered");
        // The answer is visible, in the note's thread, as the coordinator.
        let reply = by_id(&v, "reply:01REPLY");
        assert_eq!(reply.reply_to.as_deref(), Some("note:01SENT"));
        assert_eq!((reply.from, reply.author.as_str()), ("lead", LEAD_AUTHOR));
        assert_eq!(reply.text, "Yes, it works: your message reached me.");
        assert_eq!(reply.body.as_deref(), Some("via brief"));
        assert_eq!(
            by_id(&v, "note:01SENT").replies,
            vec!["reply:01REPLY".to_string()]
        );

        // A note to one agent says the same, and its answer joins its thread.
        let mut w = channel_world();
        w.team.leads[0].agents[0].messages = vec![msg(
            "out",
            "team-lead",
            NOW - 10 * MIN,
            "BLOCKED · keychain · Yes or no?",
        )];
        let mut n = note("01DM", NOW - 5 * MIN, "Para keychain: sí");
        n.trail.delivered = true;
        w.notes.push(n);
        let v = w.view();
        assert_eq!(state_of(&v, "note:01DM"), "delivered");
    }

    /// The lead reads a note for its agent, so a reply can be either of two
    /// voices; only one signed with the agent's name is the agent's.
    #[test]
    fn a_note_answered_by_the_lead_does_not_read_as_answered_by_its_agent() {
        let answered = |text: &str| {
            let mut w = channel_world();
            let mut n = note("01DM", NOW - 5 * MIN, "Para keychain: ¿me recibes?");
            n.trail.delivered = true;
            n.trail.replies.push(BusReply {
                id: "01R".into(),
                ts_ms: NOW - MIN,
                session: LEAD.into(),
                text: text.into(),
            });
            w.notes.push(n);
            by_id(&w.view(), "note:01DM")
                .delivery
                .clone()
                .expect("a delivery")
        };

        let lead = answered("Passed it on; no answer yet.");
        assert_eq!(lead.state, "answered");
        assert_eq!(lead.answered_by.as_deref(), Some(LEAD_AUTHOR));

        let own = answered("keychain: yes, I read you.");
        assert_eq!(own.state, "answered");
        assert_eq!(own.answered_by, None);
    }

    #[test]
    fn an_unanswered_note_goes_to_te_needs_after_ten_minutes() {
        let text = "En #harbard: Probando, quiero saber si esto realmente funciona ya";
        let mut w = channel_world();
        w.notes.push(note("01ASK", NOW - 9 * MIN, text));
        let unanswered = |v: &WorkspaceView| {
            open_cards(v)
                .into_iter()
                .filter(|c| c.kind == "unanswered")
                .map(|c| (c.id.clone(), c.channel.clone(), c.message.clone()))
                .collect::<Vec<_>>()
        };
        assert!(unanswered(&w.view()).is_empty(), "nine minutes is not ten");
        // The clock moves on: eleven minutes.
        w.now = NOW + 2 * MIN;
        assert_eq!(
            unanswered(&w.view()),
            vec![(
                "unanswered:note:01ASK".to_string(),
                Some("harbard".to_string()),
                Some("note:01ASK".to_string())
            )]
        );
        assert_eq!(w.view().unread.activity, 1);
        // Configurable.
        w.desk.config.unanswered_min = 30;
        assert!(unanswered(&w.view()).is_empty());
        w.desk.config.unanswered_min = 10;
        // A delivery or a read is not an answer; a reply is.
        w.notes[0].trail.delivered = true;
        w.notes[0].trail.acked_ms = Some(NOW);
        assert_eq!(unanswered(&w.view()).len(), 1);
        w.notes[0].trail.replies.push(BusReply {
            id: "01R".into(),
            ts_ms: NOW,
            session: LEAD.into(),
            text: "Funciona.".into(),
        });
        assert!(unanswered(&w.view()).is_empty());
        // Marked resolved, it leaves too; a day later it is history.
        w.notes[0].trail.replies.clear();
        w.desk.resolve("unanswered:note:01ASK", NOW);
        assert!(unanswered(&w.view()).is_empty());
        w.desk.resolved.clear();
        w.now = NOW + 25 * 60 * MIN;
        assert!(unanswered(&w.view()).is_empty());

        // A note to one agent is answered when the agent speaks after it.
        let mut w = channel_world();
        w.notes
            .push(note("01DM", NOW - 20 * MIN, "Para keychain: ¿cómo vas?"));
        assert_eq!(unanswered(&w.view()).len(), 1);
        w.team.leads[0].agents[0].messages = vec![msg(
            "out",
            "team-lead",
            NOW - 15 * MIN,
            "FINDING · keychain · Voy bien.",
        )];
        assert!(unanswered(&w.view()).is_empty());
    }

    #[test]
    fn a_channel_reaches_the_live_session_working_in_it() {
        // Today's case: the members' lead is not on the bus, but a live
        // session works in the repository. The composer must not claim
        // nobody can read the channel.
        let mut a = agent("a1", "keychain", "idle", NOW - 30 * MIN);
        a.checkouts = vec![writes("/work/harbard", NOW - 60 * MIN, NOW - 30 * MIN)];
        let mut w = World::new(vec![a]);
        w.sessions = vec![BusSession {
            session: "live-in-harbard".into(),
            liveness: Liveness::Live,
            repo: Some("/work/harbard".into()),
            last_ms: NOW,
            first_ms: NOW - 60 * MIN,
            cwd: "/work/harbard".into(),
            name: None,
            callsign: None,
            main: false,
            harness: "claude-code".into(),
            idle: false,
            rooms: Vec::new(),
        }];
        let v = w.view();
        let ch = v.channels.iter().find(|c| c.id == "harbard").unwrap();
        assert_eq!(ch.reach, "ready");
        assert_eq!(
            ch.readers
                .iter()
                .map(|r| (r.session.as_str(), r.delivery, r.lead))
                .collect::<Vec<_>>(),
            vec![
                (LEAD, "not_on_bus", true),
                ("live-in-harbard", "ready", false)
            ]
        );
        assert_eq!(
            channel_route(&v, "harbard", false),
            vec!["live-in-harbard".to_string()]
        );
        // With nobody live: honest, and "enviar samemente" stores for all.
        w.sessions.clear();
        let v = w.view();
        let ch = v.channels.iter().find(|c| c.id == "harbard").unwrap();
        assert_eq!(ch.reach, "none");
        assert!(channel_route(&v, "harbard", false).is_empty());
        assert_eq!(channel_route(&v, "harbard", true), vec![LEAD.to_string()]);
    }

    #[test]
    fn a_channel_note_reaches_every_session_working_in_the_repository() {
        // The members' lead is live, and so is another session
        // working in the same repository without agents of its own. Both
        // read the channel; the lead comes first.
        let mut a = agent("a1", "keychain", "running", NOW - MIN);
        a.checkouts = vec![writes("/work/harbard", NOW - 60 * MIN, NOW - MIN)];
        let mut w = World::new(vec![a]);
        let other = |id: &str| BusSession {
            session: id.into(),
            liveness: Liveness::Live,
            repo: Some("/work/harbard".into()),
            last_ms: NOW,
            first_ms: NOW - 60 * MIN,
            cwd: "/work/harbard".into(),
            name: None,
            callsign: None,
            main: false,
            harness: "claude-code".into(),
            idle: false,
            rooms: Vec::new(),
        };
        w.sessions = vec![
            session(LEAD, Liveness::Live),
            other("solo-1"),
            other("solo-2"),
        ];
        let v = w.view();
        assert_eq!(
            channel_route(&v, "harbard", false),
            vec![LEAD.to_string(), "solo-1".to_string(), "solo-2".to_string()]
        );
        // An ended session is not a reader unless sent anyway; nobody twice.
        w.sessions = vec![session(LEAD, Liveness::Live), other("solo-1"), {
            let mut s = other("gone");
            s.liveness = Liveness::Ended;
            s
        }];
        let v = w.view();
        assert_eq!(
            channel_route(&v, "harbard", false),
            vec![LEAD.to_string(), "solo-1".to_string()]
        );
        let anyway = channel_route(&v, "harbard", true);
        assert!(anyway.contains(&"gone".to_string()));
        let mut unique = anyway.clone();
        unique.dedup();
        assert_eq!(unique.len(), anyway.len());
    }

    // -----------------------------------------------------------------------
    // Parts 4 to 7: bus replies, names, real channels, mentions, relays,
    // read marks, temporary sessions
    // -----------------------------------------------------------------------

    fn live(id: &str, repo: &str, main: bool) -> BusSession {
        BusSession {
            callsign: None,
            session: id.into(),
            liveness: Liveness::Live,
            repo: Some(repo.into()),
            last_ms: NOW,
            first_ms: NOW - 60 * MIN,
            cwd: repo.into(),
            name: None,
            main,
            harness: "claude-code".into(),
            idle: false,
            rooms: Vec::new(),
        }
    }

    fn bus(id: &str, session: &str, ts: u64, text: &str, note_ref: Option<&str>) -> BusNote {
        BusNote {
            id: id.into(),
            ts_ms: ts,
            session: session.into(),
            text: text.into(),
            repo: Some("/work/harbard".into()),
            note_ref: note_ref.map(str::to_string),
            kind: String::new(),
        }
    }

    /// A teammate launched with a bare workshop name ("Owen") by a lead
    /// whose cwd is `~/.claude`: its team is the repository it wrote in,
    /// never the dot-folder (`harbard agents` showed ".claude · Owen").
    #[test]
    fn a_bare_workshop_name_takes_its_team_from_the_repository_it_wrote_in() {
        let mut a = agent("a1", "Owen", "running", NOW - 2 * MIN);
        a.checkouts = vec![writes("/work/harbard", NOW - 30 * MIN, NOW - 2 * MIN)];
        let mut w = World::new(vec![a]);
        w.team.leads[0].cwd = Some("/Users/p/.claude".into());
        let v = w.view();
        let r = v.agents.iter().find(|x| x.key == "a1").unwrap();
        assert_eq!(r.team.as_deref(), Some("harbard"), "{r:?}");
        assert!(!r.name.contains(".claude"), "{}", r.name);
        assert!(
            !r.role_line.as_deref().unwrap_or("").contains(".claude"),
            "{:?}",
            r.role_line
        );

        // Nothing written anywhere: no team, never the dot-folder.
        let w = {
            let mut w = World::new(vec![agent("a1", "Owen", "running", NOW - 2 * MIN)]);
            w.team.leads[0].cwd = Some("/Users/p/.claude".into());
            w
        };
        let v = w.view();
        let r = v.agents.iter().find(|x| x.key == "a1").unwrap();
        assert!(!r.team.as_deref().unwrap_or("").starts_with('.'), "{r:?}");
        assert!(!r.name.contains(".claude"), "{}", r.name);

        // The name's own suffix still wins over where it wrote.
        let mut a = agent("a1", "robots-builder", "running", NOW - 2 * MIN);
        a.checkouts = vec![writes("/work/harbard", NOW - 30 * MIN, NOW - 2 * MIN)];
        let v = World::new(vec![a]).view();
        let r = v.agents.iter().find(|x| x.key == "a1").unwrap();
        assert_eq!(r.team.as_deref(), Some("robots"));
    }

    /// Another session's reviewers (#acme): their lead runs
    /// in a folder called `memory`, they never wrote anywhere, and their
    /// teammates all wrote in acme. Their team is acme, not the
    /// folder's name; with no teammate's repository to go by, the lead's
    /// project stays.
    #[test]
    fn an_agent_that_wrote_nothing_takes_its_teammates_repository_as_team() {
        let mut built = agent("a1", "cap146", "idle", NOW - 2 * MIN);
        built.checkouts = vec![writes("/work/acme", NOW - 30 * MIN, NOW - 2 * MIN)];
        let mut other = agent("a2", "cap143", "idle", NOW - 2 * MIN);
        other.checkouts = vec![writes("/work/acme", NOW - 20 * MIN, NOW - 3 * MIN)];
        let reviewer = agent("a3", "revisor6", "idle", NOW - 2 * MIN);
        let mut w = World::new(vec![built, other, reviewer]);
        w.team.leads[0].cwd = Some("/Users/p/memory".into());
        let v = w.view();
        let team = |k: &str| v.agents.iter().find(|x| x.key == k).unwrap().team.clone();
        assert_eq!(team("a1").as_deref(), Some("acme"));
        assert_eq!(team("a3").as_deref(), Some("acme"));

        // Alone under the lead: nothing to go by but the lead's project.
        let mut w = World::new(vec![agent("a3", "revisor6", "idle", NOW - 2 * MIN)]);
        w.team.leads[0].cwd = Some("/Users/p/memory".into());
        let v = w.view();
        let r = v.agents.iter().find(|x| x.key == "a3").unwrap();
        assert_eq!(r.team.as_deref(), Some("memory"), "{r:?}");
    }

    /// `harbard name --team` is the operator-facing word on it: it wins over
    /// every derivation.
    #[test]
    fn an_explicit_team_wins_over_every_derivation() {
        let mut a = agent("a1", "robots-builder", "running", NOW - 2 * MIN);
        a.checkouts = vec![writes("/work/harbard", NOW - 30 * MIN, NOW - 2 * MIN)];
        let mut w = World::new(vec![a]);
        w.names.insert(
            "a1".into(),
            crate::workshop::Entry {
                name: "Nora".into(),
                gender: crate::workshop::Gender::F,
                team: Some("cli".into()),
                role: Some("constructor".into()),
                source: "picked".into(),
                reason: None,
                assigned_ms: NOW,
                team_explicit: true,
            },
        );
        let v = w.view();
        let r = v.agents.iter().find(|x| x.key == "a1").unwrap();
        assert_eq!(r.team.as_deref(), Some("cli"), "{r:?}");
    }

    /// A lead hands work to the operator (`note --kind handoff --to harness:human`):
    /// a Needs you card of its own kind, "Handover" in the console, open
    /// until they answer it in its thread or resolves the card.
    #[test]
    fn a_handoff_to_the_operator_is_a_card_until_he_answers_it() {
        let mut w = harbard_world();
        let mut h = bus(
            "01HAND",
            LEAD,
            NOW - 5 * MIN,
            "Handing over the 303 signature.",
            None,
        );
        h.kind = "handoff".into();
        let fyi = bus("01FYI", LEAD, NOW - 4 * MIN, "Para que lo sepas.", None);
        w.bus_notes = vec![h, fyi];
        let v = w.view();
        let open: Vec<&Card> = open_cards(&v)
            .into_iter()
            .filter(|c| c.kind == "handoff")
            .collect();
        assert_eq!(open.len(), 1, "{:?}", v.cards);
        assert_eq!(open[0].id, "handoff:01HAND");
        assert_eq!(open[0].message.as_deref(), Some("bus:01HAND"));
        assert_eq!(open[0].since_ms, NOW - 5 * MIN);

        // They answer it in its thread: no longer needed.
        w.desk.link_thread("01REPLY", "bus:01HAND");
        let v = w.view();
        assert!(
            open_cards(&v).iter().all(|c| c.kind != "handoff"),
            "{:?}",
            v.cards
        );
    }

    fn harbard_world() -> World {
        let mut a = agent("a1", "harbard-view", "idle", NOW - 30 * MIN);
        a.checkouts = vec![writes("/work/harbard", NOW - 60 * MIN, NOW - 30 * MIN)];
        let mut w = World::new(vec![a]);
        // The lead session works in harbard and is on the bus.
        w.sessions = vec![live(LEAD, "/work/harbard", true)];
        w
    }

    /// A 600-character line, words and a closing mark, as the lead writes them.
    fn long_line() -> String {
        let mut line = String::new();
        while line.chars().count() < 590 {
            line.push_str("I will review it calmly and ");
        }
        line.push_str("DONE-DEL-TEXTO");
        line
    }

    #[test]
    fn a_running_session_outside_any_repository_can_still_be_reached() {
        let mut w = harbard_world();
        let mut here = live("here", "/work/x", true);
        here.repo = None;
        here.callsign = Some("Jesse".into());
        let mut gone = live("gone", "/work/x", true);
        gone.repo = None;
        gone.liveness = Liveness::Stale;
        w.sessions.push(here);
        w.sessions.push(gone);
        let v = w.view();
        let found = v.agents.iter().find(|a| a.key == "session:here").unwrap();
        assert_eq!(
            (found.name.as_str(), found.handle.as_str()),
            ("Jesse", "jesse")
        );
        assert!(v.agents.iter().all(|a| a.key != "session:gone"));
    }

    #[test]
    fn a_lead_goes_by_its_callsign_and_answers_to_it() {
        let mut w = harbard_world();
        w.sessions[0].callsign = Some("Charles".into());
        w.bus_notes = vec![bus(
            "01A",
            LEAD,
            NOW - MIN,
            "FINDING · lead · Carrying on with this.",
            None,
        )];
        let v = w.view();
        let lead = v.agents.iter().find(|a| a.kind == "session").unwrap();
        assert_eq!(lead.name, "Charles");
        assert_eq!(lead.handle, "charles");
        assert_eq!(by_id(&v, "bus:01A").author, "Charles");
    }

    #[test]
    fn the_initials_come_from_the_name_when_the_desk_has_none_for_it() {
        assert_eq!(initials_of("Ana Ruiz"), "AR");
        assert_eq!(initials_of("Adonay"), "AD");
        assert_eq!(initials_of(""), "");
    }

    #[test]
    fn a_session_that_names_the_operator_in_a_channel_raises_a_card() {
        let mut w = harbard_world();
        w.bus_notes = vec![
            bus(
                "01ASK",
                LEAD,
                NOW - MIN,
                "FINDING · lead · @dana continue?",
                None,
            ),
            bus(
                "01NOT",
                LEAD,
                NOW - MIN,
                "FINDING · lead · No mention.",
                None,
            ),
            bus(
                "01OLD",
                LEAD,
                NOW - ACTIVE_MS - MIN,
                "FINDING · lead · @dana and this?",
                None,
            ),
        ];
        let v = w.view();
        let cards: Vec<_> = v.cards.iter().filter(|c| c.kind == "mention").collect();
        let open: Vec<_> = cards.iter().filter(|c| c.needs).collect();
        assert_eq!(open.len(), 1, "{cards:?}");
        assert_eq!(open[0].message.as_deref(), Some("bus:01ASK"));
        assert!(open[0].channel.is_some(), "it opens in its channel");
        assert!(
            cards
                .iter()
                .any(|c| !c.needs && c.message.as_deref() == Some("bus:01OLD")),
            "an old one stays as history"
        );

        // Answered in its thread, it stops asking.
        w.desk.threads.insert("01R".into(), "bus:01ASK".into());
        let v = w.view();
        assert!(
            v.cards
                .iter()
                .all(|c| c.message.as_deref() != Some("bus:01ASK"))
        );
    }

    #[test]
    fn a_long_bus_note_and_its_reply_keep_every_character() {
        let line = long_line();
        assert!(line.chars().count() >= 600);
        let mut w = harbard_world();
        w.bus_notes = vec![
            bus("01LONG", LEAD, NOW - 5 * MIN, &line, None),
            bus(
                "01MULTI",
                LEAD,
                NOW - 4 * MIN,
                &format!("FINDING · lead · First line.\n{line}\nLast line."),
                None,
            ),
            bus(
                "01V2",
                LEAD,
                NOW - 3 * MIN,
                &format!("FINDING · lead · {line}\n\ntechnical detail"),
                None,
            ),
        ];
        let v = w.view();
        // v1, one line: all of it, no ellipsis.
        let one = by_id(&v, "bus:01LONG");
        assert_eq!(one.text, line);
        assert!(!one.text.contains('…'));
        // v1, several lines: every line belongs to the text.
        let multi = by_id(&v, "bus:01MULTI");
        assert_eq!(multi.text, format!("First line.\n{line}\nLast line."));
        assert_eq!(multi.body, None);
        // v2: the human line in full, the technical body apart.
        let v2 = by_id(&v, "bus:01V2");
        assert_eq!(v2.text, line);
        assert_eq!(v2.body.as_deref(), Some("technical detail"));

        // The lead's answer in an operator note's thread, the case the operator saw.
        let mut w = channel_world();
        w.notes
            .push(note("01ASK", NOW - 3 * MIN, "En #harbard: ¿qué pasa?"));
        w.notes[0].trail.replies.push(BusReply {
            id: "01ANSWER".into(),
            ts_ms: NOW - MIN,
            session: LEAD.into(),
            text: line.clone(),
        });
        let v = w.view();
        let answer = by_id(&v, "reply:01ANSWER");
        assert_eq!(answer.text, line);
        assert!(answer.text.ends_with("DONE-DEL-TEXTO"));
    }

    #[test]
    fn a_long_operator_note_keeps_every_line() {
        let line = long_line();
        let mut w = channel_world();
        let text = format!("En #harbard: {line}\nAnd a second line.");
        w.notes.push(note("01OPLONG", NOW - 3 * MIN, &text));
        let v = w.view();
        let n = by_id(&v, "note:01OPLONG");
        assert!(
            n.text.ends_with(&format!("{line}\nAnd a second line.")),
            "{}",
            n.text
        );
        assert!(!n.text.contains('…'));
    }

    #[test]
    fn a_note_is_still_bounded_at_the_body_cap() {
        let huge = "x".repeat(human::BODY_CAP + 100);
        let mut w = harbard_world();
        w.bus_notes = vec![bus("01HUGE", LEAD, NOW - MIN, &huge, None)];
        let v = w.view();
        assert_eq!(
            by_id(&v, "bus:01HUGE").text.chars().count(),
            human::BODY_CAP + 1
        );
    }

    #[test]
    fn a_bus_note_to_the_human_shows_in_its_channel_with_its_thread() {
        let mut w = harbard_world();
        w.bus_notes = vec![
            bus(
                "01A",
                LEAD,
                NOW - 5 * MIN,
                "Dana: the channel works now.\n\nvia note --to harness:human",
                None,
            ),
            bus(
                "01B",
                "herd-child",
                NOW - 4 * MIN,
                "Confirming from another session.",
                Some("01A"),
            ),
        ];
        w.bus_notes[1].repo = Some("/work/harbard-wt-x".into());
        let v = w.view();
        let top = by_id(&v, "bus:01A");
        assert_eq!((top.from, top.author.as_str()), ("lead", LEAD_AUTHOR));
        assert_eq!(top.channels, vec!["harbard".to_string()]);
        assert_eq!(top.body.as_deref(), Some("via note --to harness:human"));
        // The answer to it sits in its thread: "1 reply".
        assert_eq!(top.replies, vec!["bus:01B".to_string()]);
        let child = by_id(&v, "bus:01B");
        assert_eq!(child.reply_to.as_deref(), Some("bus:01A"));
        assert_eq!(child.from, "session");
        // The coordinator's conversation holds it too, as unread.
        let coord = v
            .agents
            .iter()
            .find(|a| a.key == format!("session:{LEAD}"))
            .unwrap();
        assert_eq!((coord.name.as_str(), coord.kind), (LEAD_AUTHOR, "session"));
        assert_eq!(coord.unread, 1);
        let ch = v.channels.iter().find(|c| c.id == "harbard").unwrap();
        assert_eq!(ch.unread, 2);
    }

    #[test]
    fn a_new_answer_changes_the_rev_the_slice_carries() {
        let mut w = harbard_world();
        w.notes
            .push(note("01N", NOW - 3 * MIN, "En #harbard: ¿funciona?"));
        let before = w.view();
        assert_eq!(ActivityView::of(&before).rev, before.rev);
        w.notes[0].trail.replies.push(BusReply {
            id: "01R".into(),
            ts_ms: NOW,
            session: LEAD.into(),
            text: "Yes.".into(),
        });
        let after = w.view();
        assert_ne!(
            before.rev, after.rev,
            "the console pulls the full view on a new rev"
        );
        assert_eq!(ActivityView::of(&after).rev, after.rev);
    }

    /// #acme-config: a live builder that once ran
    /// `git -C ~/…/acme-config log` while writing in harbard, and a
    /// researcher that read it with git two days ago and wrote only under
    /// Documents. Nobody works there, so the channel must not look busy.
    #[test]
    fn a_repository_only_read_with_git_is_quiet_with_no_messages_and_no_unread() {
        let mut builder = agent("protocol", "harbard-protocol", "running", NOW - MIN);
        // Its last write was half an hour ago; it is seen every minute.
        builder.checkouts = vec![writes(
            "/work/harbard-wt-protocol",
            NOW - 90 * MIN,
            NOW - 30 * MIN,
        )];
        builder.git_roots = vec!["/work/acme-config".into()];
        builder.messages = vec![
            msg(
                "in",
                "team-lead",
                NOW - 80 * MIN,
                "START · lead · Arregla Strip.",
            ),
            msg(
                "out",
                "team-lead",
                NOW - 40 * MIN,
                "DONE · harbard-protocol · Strip ya se ve entero.",
            ),
            msg(
                "in",
                "team-lead",
                NOW - 30 * MIN,
                "START · lead · On to the next thing.",
            ),
            msg(
                "out",
                "team-lead",
                NOW - 10 * MIN,
                "DONE · harbard-protocol · Los mensajes largos salen enteros.",
            ),
            msg(
                "in",
                "team-lead",
                NOW - 5 * MIN,
                "START · lead · Ahora #acme-config.",
            ),
        ];
        let mut researcher = agent("brace", "tern-integration", "done", NOW - 2 * 24 * 60 * MIN);
        researcher.write_dirs = vec![writes(
            "/home/p/Documents/notes/2026-11-05",
            NOW - 2 * 24 * 60 * MIN,
            NOW - 2 * 24 * 60 * MIN,
        )];
        researcher.git_roots = vec!["/work/acme-config".into()];
        researcher.messages = vec![
            msg(
                "in",
                "team-lead",
                NOW - 2 * 24 * 60 * MIN - 20 * MIN,
                "Estudia Tern.",
            ),
            msg(
                "out",
                "team-lead",
                NOW - 2 * 24 * 60 * MIN,
                "DONE · tern-integration · Report ready.",
            ),
            msg(
                "in",
                "team-lead",
                NOW - 2 * 24 * 60 * MIN + MIN,
                "Gracias, cierro.",
            ),
        ];
        let w = World::new(vec![builder, researcher]);
        let resolve = |path: &str| -> Option<CheckoutInfo> {
            if path.contains("/Documents/") {
                return None;
            }
            resolver(path)
        };
        let v = build(&Inputs {
            team: &w.team,
            sessions: &w.sessions,
            notes: &w.notes,
            bus_notes: &w.bus_notes,
            temp_dirs: &[],
            names: &w.names,
            deliveries: &w.deliveries,
            receipts: &w.receipts,
            desk: &w.desk,
            resolve: &resolve,
            attachment: &no_attachment,
            now_ms: w.now,
            operator: "Dana",
            horizon_ms: ACTIVE_MS,
            keep_ended: false,
            in_workspace: &|_| true,
            known_repos: &[],
            rooms: &[],
            room_notes: &[],
            archived_repos: &BTreeSet::new(),
            workspace: None,
        });
        let config_repo = v.channels.iter().find(|c| c.id == "acme-config").unwrap();
        // Still a channel (both ran git there), but quiet, with nothing unread,
        // nothing said in it, nobody working there, and last in the list.
        assert!(config_repo.quiet, "active_ms {}", config_repo.active_ms);
        assert_eq!(config_repo.active_ms, 0);
        assert_eq!(config_repo.unread, 0);
        assert!(config_repo.recent.is_empty());
        assert_eq!(
            config_repo.members,
            vec!["brace".to_string(), "protocol".to_string()]
        );
        assert!(
            v.messages
                .iter()
                .all(|m| !m.channels.contains(&config_repo.id)),
            "{:?}",
            v.messages
                .iter()
                .filter(|m| m.channels.contains(&config_repo.id))
                .map(|m| &m.text)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            v.channels.last().map(|c| c.id.as_str()),
            Some("acme-config")
        );
        let at = |id: &str| v.channels.iter().position(|c| c.id == id).unwrap();
        assert!(
            at("acme-config") > at("harbard"),
            "a git read does not lift it"
        );
        // The builder's conversation belongs where it writes, and it says why
        // harbard is here; its membership of config_repo is still shown on the agent.
        let harbard = v.channels.iter().find(|c| c.id == "harbard").unwrap();
        assert!(
            harbard.recent.contains(&"protocol".to_string()),
            "{:?}",
            harbard.recent
        );
        assert!(
            of_agent(&v, "protocol")
                .iter()
                .all(|m| m.channels == vec!["harbard".to_string()])
        );
        let builder = v.agents.iter().find(|a| a.key == "protocol").unwrap();
        assert!(builder.channels.contains(&"acme-config".to_string()));
        // The researcher wrote nowhere a channel lives: its messages stay in
        // its own conversation.
        assert!(of_agent(&v, "brace").iter().all(|m| m.channels.is_empty()));
    }

    /// A lead's brief to a teammate is the teammate's conversation: it does
    /// not wake a channel the teammate last wrote in three weeks ago.
    #[test]
    fn a_leads_brief_is_not_talk_in_the_channel() {
        let mut old = agent("old", "file", "idle", NOW - 60 * MIN);
        old.checkouts = vec![writes(
            "/work/file",
            NOW - 21 * 24 * 60 * MIN,
            NOW - 21 * 24 * 60 * MIN,
        )];
        old.messages = vec![msg(
            "in",
            "team-lead",
            NOW - 60 * MIN,
            "START · lead · Still there?",
        )];
        let v = World::new(vec![old]).view();
        let file = v.channels.iter().find(|c| c.id == "file").unwrap();
        assert!(file.quiet, "active_ms {}", file.active_ms);
        assert!(file.recent.is_empty());
    }

    /// A reviewer that only reads with git still talks in the channel of the
    /// repository the team writes in.
    #[test]
    fn a_reviewer_that_reads_an_active_repository_talks_in_its_channel() {
        let mut writer = agent("w", "harbard-view", "running", NOW - MIN);
        writer.checkouts = vec![writes("/work/harbard-wt-view", NOW - 30 * MIN, NOW - MIN)];
        let mut reviewer = agent("r", "harbard-review", "running", NOW - 2 * MIN);
        reviewer.git_roots = vec!["/work/harbard-wt-view".into()];
        reviewer.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 3 * MIN,
            "FINDING · harbard-review · Falta un test.",
        )];
        let v = World::new(vec![writer, reviewer]).view();
        assert!(
            of_agent(&v, "r")
                .iter()
                .all(|m| m.channels == vec!["harbard".to_string()])
        );
        let harbard = v.channels.iter().find(|c| c.id == "harbard").unwrap();
        // Reading is membership, not work: the header counts the writer only.
        assert!(
            harbard.recent.contains(&"w".to_string()) && !harbard.recent.contains(&"r".to_string())
        );
    }

    #[test]
    fn a_channel_is_a_real_repository_and_a_quiet_one_moves_to_more_channels() {
        // Orphan names. `v7` and `drafts` live under Documents,
        // `acme-config` is a real repository nobody wrote in for weeks.
        let mut a = agent("a1", "harbard-view", "idle", NOW - 30 * MIN);
        a.checkouts = vec![
            writes("/work/harbard-wt-view", NOW - 60 * MIN, NOW - 30 * MIN),
            writes(
                "/home/p/Documents/notes/2026-11-06/sketches/v7",
                NOW - 50 * MIN,
                NOW - 40 * MIN,
            ),
        ];
        a.git_roots = vec![
            "/home/p/Documents/notes/2026-11-05/drafts".into(),
            "/work/acme-config".into(),
        ];
        let w = World::new(vec![a]);
        // The resolver confirms /work as git; Documents is never resolved,
        // and a lexical guess alone is not a repository.
        let resolve = |path: &str| -> Option<CheckoutInfo> {
            if path.contains("/Documents/") {
                return None;
            }
            resolver(path)
        };
        let v = build(&Inputs {
            team: &w.team,
            sessions: &w.sessions,
            notes: &w.notes,
            bus_notes: &w.bus_notes,
            temp_dirs: &[],
            names: &w.names,
            deliveries: &w.deliveries,
            receipts: &w.receipts,
            desk: &w.desk,
            resolve: &resolve,
            attachment: &no_attachment,
            now_ms: w.now,
            operator: "Dana",
            horizon_ms: ACTIVE_MS,
            keep_ended: false,
            in_workspace: &|_| true,
            known_repos: &[],
            rooms: &[],
            room_notes: &[],
            archived_repos: &BTreeSet::new(),
            workspace: None,
        });
        let ids: Vec<&str> = v.channels.iter().map(|c| c.id.as_str()).collect();
        assert!(!ids.contains(&"v7") && !ids.contains(&"drafts"), "{ids:?}");
        assert!(!ids.iter().any(|i| i.contains("-wt-")), "{ids:?}");
        let harbard = v.channels.iter().find(|c| c.id == "harbard").unwrap();
        assert!(!harbard.quiet, "written in today");
        // Only git read it: in the list, but under "More channels".
        let config_repo = v.channels.iter().find(|c| c.id == "acme-config").unwrap();
        assert!(config_repo.quiet);
        // A lexical guess for a removed folder never makes a channel.
        let lexical = |path: &str| -> Option<CheckoutInfo> {
            resolver(path).map(|mut i| {
                i.real = !path.starts_with("/work/acme");
                i
            })
        };
        let v = build(&Inputs {
            team: &w.team,
            sessions: &[],
            notes: &w.notes,
            bus_notes: &w.bus_notes,
            temp_dirs: &[],
            names: &w.names,
            deliveries: &w.deliveries,
            receipts: &w.receipts,
            desk: &w.desk,
            resolve: &lexical,
            attachment: &no_attachment,
            now_ms: w.now,
            operator: "Dana",
            horizon_ms: ACTIVE_MS,
            keep_ended: false,
            in_workspace: &|_| true,
            known_repos: &[],
            rooms: &[],
            room_notes: &[],
            archived_repos: &BTreeSet::new(),
            workspace: None,
        });
        assert!(!v.channels.iter().any(|c| c.id == "acme-config"));
    }

    #[test]
    fn a_temporary_session_never_asks_for_anything() {
        // A sandbox eval: its project dir is under the system temp folder and
        // its process is gone. No card, no channel, no dormant warning.
        let mut a = agent("a1", "fix-bun", "running", NOW - 72 * MIN);
        a.name = None;
        a.kind = "subagent";
        a.description = Some("Fix failing bun test".into());
        a.checkouts = vec![writes("/work/sandbox", NOW - 80 * MIN, NOW - 72 * MIN)];
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 75 * MIN,
            "BLOCKED · x · Continue?",
        )];
        let mut w = World::new(vec![a]);
        w.team.leads[0].project = "-private-var-folders-xy-abc-T-sandbox-l1-3-ws".into();
        w.team.leads[0].cwd = Some("/private/var/folders/xy/abc/T/sandbox-l1-3-ws".into());
        let v = w.view();
        let x = &v.agents[0];
        assert!(x.temporary);
        assert_eq!(x.dormant, None);
        assert!(open_cards(&v).is_empty(), "{:?}", open_cards(&v));
        assert!(!v.channels.iter().any(|c| c.id == "sandbox"));
        assert_eq!(v.unread.directs, 0);
    }

    #[test]
    fn mentions_are_recognised_and_an_agent_naming_operator_goes_to_te_needs() {
        let mut a = agent("a1", "robots-builder", "idle", NOW - MIN);
        a.checkouts = vec![writes("/work/harbard", NOW - 60 * MIN, NOW - MIN)];
        a.messages = vec![
            msg(
                "out",
                "team-lead",
                NOW - 3 * MIN,
                "FINDING · robots-builder · @dana blue robot or green?",
            ),
            msg(
                "out",
                "team-lead",
                NOW - 2 * MIN,
                "FINDING · robots-builder · Carrying on with @robots and @nobody.",
            ),
        ];
        let b = agent("a2", "robots-review", "idle", NOW - MIN);
        let mut w = World::new(vec![a, b]);
        w.notes.push(note(
            "01M",
            NOW - MIN,
            "En #harbard: @robots ¿cómo vais? avisa a @canal",
        ));
        let v = w.view();
        let m = of_agent(&v, "a1");
        assert_eq!(m[0].mentions, vec!["dana".to_string()]);
        assert_eq!(
            m[1].mentions,
            vec!["robots".to_string()],
            "an unknown @ is not a mention"
        );
        let cards: Vec<_> = open_cards(&v)
            .into_iter()
            .filter(|c| c.kind == "mention")
            .collect();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].message.as_deref(), Some(m[0].id.as_str()));
        // Answered, it leaves Needs you.
        let mut w2 = World::new(w.team.leads[0].agents.clone());
        w2.notes
            .push(note("01P", NOW - MIN / 2, "Para robots · builder: el azul"));
        assert!(open_cards(&w2.view()).iter().all(|c| c.kind != "mention"));
        assert_eq!(
            by_id(&v, "note:01M").mentions,
            vec!["robots".to_string(), "canal".to_string()]
        );
        let handles: Vec<(&str, Option<&str>)> = v
            .agents
            .iter()
            .map(|a| (a.handle.as_str(), a.team.as_deref()))
            .collect();
        assert!(handles.contains(&("robots-builder", Some("robots"))));
    }

    /// An agent that does not know the operator's configured name can still ask.
    #[test]
    fn at_operator_reaches_the_operator_whatever_their_name() {
        let mut a = agent("a1", "robots-builder", "idle", NOW - MIN);
        a.checkouts = vec![writes("/work/harbard", NOW - 60 * MIN, NOW - MIN)];
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - MIN,
            "FINDING · robots-builder · @operator continue?",
        )];
        let v = World::new(vec![a]).view();
        assert_eq!(of_agent(&v, "a1")[0].mentions, vec!["operator".to_string()]);
        assert_eq!(
            open_cards(&v)
                .iter()
                .filter(|c| c.kind == "mention")
                .count(),
            1
        );
    }

    #[test]
    fn a_relayed_note_reaches_its_agent_by_its_to_agent_line() {
        let a = agent("a1", "robots-builder", "idle", NOW - MIN);
        let b = agent("a2", "robots-review", "idle", NOW - MIN);
        let mut w = World::new(vec![a, b]);
        w.notes.push(note(
            "01D",
            NOW - 2 * MIN,
            "Para robots · builder: ¿cómo vas?\nto_agent: a1",
        ));
        // A mention of the reviewer rides the same note to the same lead.
        w.notes.push(note(
            "01E",
            NOW - MIN,
            "Para robots · builder: pásaselo a @robots-review\nto_agent: a1\nto_agent: a2",
        ));
        let v = w.view();
        let one: Vec<&str> = of_agent(&v, "a1").iter().map(|m| m.text.as_str()).collect();
        assert_eq!(one, vec!["¿cómo vas?", "pásaselo a @robots-review"]);
        let two: Vec<&str> = of_agent(&v, "a2").iter().map(|m| m.id.as_str()).collect();
        assert_eq!(two, vec!["note:01E"]);
        assert!(v.messages.iter().all(|m| !m.text.contains("to_agent")));
        // A separately launched session is a conversation of its own.
        w.sessions.push(live("codex-1", "/work/harbard", true));
        w.sessions[1].name = Some("codex-review".into());
        let v = w.view();
        let s = v
            .agents
            .iter()
            .find(|a| a.key == "session:codex-1")
            .unwrap();
        assert_eq!(
            (s.kind, s.name.as_str(), s.lead_session.as_str()),
            ("session", "codex · reviewer", "codex-1")
        );
        assert_eq!(
            reply_route(&v, "session:codex-1", false)
                .unwrap()
                .lead_session,
            "codex-1"
        );
    }

    #[test]
    fn read_marks_clear_channels_directs_and_todo_but_never_te_needs() {
        let mut a = agent("a1", "keychain", "idle", NOW - MIN);
        a.checkouts = vec![writes("/work/orchard", NOW - 30 * MIN, NOW - MIN)];
        // The FINDING first: a later word of the agent's would close the
        // BLOCKED.
        a.messages = vec![
            msg(
                "out",
                "team-lead",
                NOW - 3 * MIN,
                "FINDING · keychain · algo",
            ),
            msg(
                "out",
                "team-lead",
                NOW - 2 * MIN,
                "BLOCKED · keychain · Yes or no?",
            ),
        ];
        let mut w = World::new(vec![a]);
        let v = w.view();
        let ch = |v: &WorkspaceView| {
            v.channels
                .iter()
                .find(|c| c.id == "orchard")
                .unwrap()
                .unread
        };
        assert_eq!(
            (ch(&v), v.unread.directs, v.unread.todo, v.unread.activity),
            (2, 2, 2, 1)
        );
        // One channel.
        w.desk.mark_channel_read("orchard", NOW);
        let v = w.view();
        assert_eq!((ch(&v), v.unread.directs), (0, 2));
        // Everything: the Dock's `directs` goes to 0 through the slice.
        w.desk.mark_all_read(NOW);
        let v = w.view();
        let slice = ActivityView::of(&v);
        assert_eq!((slice.directs, v.unread.todo, v.unread.channels), (0, 0, 0));
        // Needs you keeps its own "Mark resolved".
        assert_eq!(slice.needs, 1);
        // Stored in the desk file: a reload reads it back.
        let home = TempHome::new();
        let path = DeskState::path(home.path());
        w.desk.save_path(&path).unwrap();
        let back = DeskState::load_path(&path);
        assert_eq!(
            (back.read_all_ms, back.channel_reads.get("orchard").copied()),
            (NOW, Some(NOW))
        );
    }

    #[test]
    fn a_workshop_name_is_shown_with_its_role_and_answers_its_at() {
        use crate::workshop::{Entry, Gender};
        let a = agent("a1", "protocol-review", "idle", NOW - MIN);
        let b = agent("a2", "protocol-builder", "idle", NOW - MIN);
        let mut w = World::new(vec![a, b]);
        let entry = |name: &str, gender, role: &str| Entry {
            name: name.into(),
            gender,
            team: Some("protocol".into()),
            role: Some(role.into()),
            source: "auto".into(),
            reason: None,
            assigned_ms: NOW,
            team_explicit: false,
        };
        w.names
            .insert("a1".into(), entry("Nora", Gender::F, "revisor"));
        w.names
            .insert("a2".into(), entry("Owen", Gender::M, "constructor"));
        w.notes.push(note(
            "01C",
            NOW - MIN,
            "Para Nora: ¿lo revisas?\nto_agent: a1",
        ));
        w.team.leads[0].agents[1].messages = vec![msg("in", "team-lead", NOW - MIN, "Sigue.")];
        let v = w.view();
        let by = |k: &str| v.agents.iter().find(|x| x.key == k).unwrap().clone();
        assert_eq!(
            (by("a1").name.as_str(), by("a1").role_line.as_deref()),
            ("Nora", Some("reviewer · protocol"))
        );
        assert_eq!(
            (by("a2").name.as_str(), by("a2").role_line.as_deref()),
            ("Owen", Some("builder · protocol"))
        );
        assert_eq!(by("a1").handle, "nora");
        assert_eq!(
            of_agent(&v, "a1")
                .iter()
                .find(|m| m.from == "operator")
                .unwrap()
                .text,
            "¿lo revisas?"
        );
        // The lead stays the coordinator.
        assert_eq!(of_agent(&v, "a2")[0].author, LEAD_AUTHOR);
    }

    #[test]
    fn an_agent_its_lead_stopped_reads_closed_and_asks_for_nothing() {
        let mut a = agent("a1", "harbard-dock", "stopped", NOW - 20 * MIN);
        a.state_reason = "task_stop";
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 25 * MIN,
            "BLOCKED · harbard-dock · Continue?",
        )];
        let b = agent("a2", "harbard-view", "idle", NOW - 20 * MIN);
        let v = World::new(vec![a, b]).view();
        let x = v.agents.iter().find(|a| a.key == "a1").unwrap();
        assert!(x.closed);
        assert_eq!(
            (x.activity.state, x.activity.detail.as_deref()),
            ("finished", Some("closed"))
        );
        assert!(
            open_cards(&v).iter().all(|c| c.kind != "blocked"),
            "a closed agent's BLOCKED asks nothing"
        );
        let y = v.agents.iter().find(|a| a.key == "a2").unwrap();
        assert!(!y.closed && y.activity.detail.as_deref() != Some("closed"));
    }

    /// "Hide": a hidden agent leaves the lists (the console's job) but
    /// never its "Needs you": a BLOCKED or an `@operator` still raises a card.
    #[test]
    fn a_hidden_agent_still_raises_its_blocked_and_mention_cards() {
        let mut a = agent("a1", "harbard-dock", "idle", NOW - 2 * MIN);
        a.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 3 * MIN,
            "BLOCKED · harbard-dock · Continue?",
        )];
        let mut b = agent("a2", "harbard-view", "idle", NOW - 2 * MIN);
        b.messages = vec![msg(
            "out",
            "team-lead",
            NOW - 3 * MIN,
            "FINDING · harbard-view · @dana look at this",
        )];
        let c = agent("a3", "harbard-themes", "idle", NOW - 2 * MIN);
        let mut w = World::new(vec![a, b, c]);
        w.desk.hide_agent("a1", NOW - MIN);
        w.desk.hide_agent("a2", NOW - MIN);
        let v = w.view();
        let by = |k: &str| v.agents.iter().find(|x| x.key == k).unwrap();
        assert!(by("a1").hidden && by("a2").hidden && !by("a3").hidden);
        let open = open_cards(&v);
        assert!(
            open.iter()
                .any(|c| c.kind == "blocked" && c.agents == ["a1"]),
            "a hidden agent's BLOCKED still needs the operator"
        );
        assert!(
            open.iter()
                .any(|c| c.kind == "mention" && c.agents == ["a2"]),
            "a hidden agent's @dana still needs the operator"
        );
        assert!(v.unread.activity >= 2);
    }

    #[test]
    fn hide_show_and_show_all_change_only_the_hidden_set() {
        let mut d = DeskState::default();
        d.hide_agent("a1", 10);
        d.hide_agent("a2", 20);
        d.hide_agent("a1", 30);
        assert_eq!(
            d.hidden_agents.get("a1"),
            Some(&10),
            "hiding twice keeps the first time"
        );
        d.show_agent("a1");
        assert!(!d.hidden_agents.contains_key("a1") && d.hidden_agents.contains_key("a2"));
        d.show_all_agents();
        assert!(d.hidden_agents.is_empty());
        // An empty set is not written: an old desk file reads the same.
        let raw = serde_json::to_string(&d).unwrap();
        assert!(!raw.contains("hidden_agents") && !raw.contains("agents_collapsed"));
        let old: DeskState = serde_json::from_str("{}").unwrap();
        assert!(old.hidden_agents.is_empty() && old.agents_collapsed.is_none());
    }

    // -----------------------------------------------------------------
    // "Deliveries": each state from fixtures (crate::shipments)
    // -----------------------------------------------------------------

    /// Git as a table: `(sha prefix, on main, installed)`; anything else
    /// is unknown there.
    fn git_table(
        rows: &'static [(&'static str, bool, Option<bool>)],
    ) -> impl Fn(&str, &str) -> Option<crate::shipments::ShaFacts> {
        move |repo: &str, sha: &str| {
            assert_eq!(repo, "/work/harbard", "asked about another repository");
            rows.iter()
                .find(|(s, _, _)| s.starts_with(sha) || sha.starts_with(s))
                .map(|(s, on_main, installed)| crate::shipments::ShaFacts {
                    commit: format!("{s}{}", "0".repeat(40 - s.len())),
                    landed_as: None,
                    on_main: *on_main,
                    installed: *installed,
                })
        }
    }

    fn builder(id: &str, name: &str, lines: &[(u64, &str)]) -> AgentRow {
        let mut a = agent(id, name, "idle", NOW - MIN);
        a.checkouts = vec![writes(
            &format!("/work/harbard-wt-{id}"),
            NOW - 90 * MIN,
            NOW - 2 * MIN,
        )];
        a.messages = lines
            .iter()
            .map(|(ago, line)| msg("out", "team-lead", NOW - ago * MIN, line))
            .collect();
        a
    }

    fn shipments_of(
        agents: Vec<AgentRow>,
        git: &dyn Fn(&str, &str) -> Option<crate::shipments::ShaFacts>,
    ) -> Vec<crate::shipments::Shipment> {
        crate::shipments::shipments(&World::new(agents).view(), git)
    }

    #[test]
    fn deliveries_each_state_from_its_evidence() {
        static GIT: [(&str, bool, Option<bool>); 5] = [
            ("aaaa111", false, Some(false)),
            ("bbbb222", false, Some(false)),
            ("cccc333", false, Some(false)),
            ("dddd444", true, Some(false)),
            ("eeee555", true, Some(true)),
        ];
        let git = git_table(&GIT);
        let agents = vec![
            builder(
                "b1",
                "Marcus",
                &[(
                    50,
                    "DONE · Marcus · The header says who is working; SHA aaaa111",
                )],
            ),
            builder(
                "b2",
                "Laura",
                &[(
                    40,
                    "DONE · Laura · Ocultar agentes; SHA bbbb222 sobre 9f9f9f9",
                )],
            ),
            builder(
                "b3",
                "Alice",
                &[(30, "DONE · Alice · Los temas; SHA cccc333")],
            ),
            builder(
                "b4",
                "Iris",
                &[(20, "DONE · Iris · El perfil; SHA dddd444")],
            ),
            builder(
                "b5",
                "Owen",
                &[(10, "DONE · Owen · El protocolo; SHA eeee555")],
            ),
            builder(
                "r1",
                "harbard-review",
                &[
                    (35, "START · harbard-review · Reviso bbbb222"),
                    (25, "START · harbard-review · Reviso cccc333"),
                    (22, "FINDING · harbard-review · cccc333 breaks dark mode"),
                    (
                        15,
                        "DONE · harbard-review · dddd444 revisado, sin hallazgos",
                    ),
                ],
            ),
        ];
        let out = shipments_of(agents, &git);
        let state = |agent: &str| out.iter().find(|s| s.agent == agent).map(|s| s.state);
        assert_eq!(state("b1"), Some("delivered"), "a DONE nobody reviewed yet");
        assert_eq!(state("b2"), Some("in_review"), "a review agent named it");
        assert_eq!(state("b3"), Some("with_fixes"), "the review found things");
        assert_eq!(
            state("b4"),
            Some("integrated"),
            "on main, not in the running build"
        );
        assert_eq!(
            state("b5"),
            Some("installed"),
            "on main and in the running build"
        );
        // The reviewer's DONE is a review of b4, not a delivery of its own.
        assert!(out.iter().all(|s| s.agent != "r1"));
        // Every one of them reaches "Installed" in serve's own repository.
        assert!(out.iter().all(|s| s.last_state == "installed"));
        let pending: Vec<&str> = out
            .iter()
            .filter(|s| s.pending)
            .map(|s| s.agent.as_str())
            .collect();
        assert_eq!(
            pending,
            vec!["b4", "b3", "b2", "b1"],
            "newest first, the installed one is not pending"
        );
        let b2 = out.iter().find(|s| s.agent == "b2").unwrap();
        assert_eq!(
            b2.sha.as_deref(),
            Some("bbbb222"),
            "the SHA after «SHA», not the base"
        );
        assert_eq!(b2.review.len(), 1);
        assert_eq!(b2.review[0].kind, "reviewing");
        let b4 = out.iter().find(|s| s.agent == "b4").unwrap();
        assert_eq!(b4.review[0].kind, "reviewed");
        assert_eq!(b4.text, "El perfil; SHA dddd444", "the human line");
    }

    #[test]
    fn deliveries_outside_serves_repository_end_at_integrated() {
        static GIT: [(&str, bool, Option<bool>); 1] = [("abcd123", true, None)];
        let out = shipments_of(
            vec![builder(
                "b1",
                "Marcus",
                &[(5, "DONE · Marcus · Hecho; SHA abcd123")],
            )],
            &git_table(&GIT),
        );
        assert_eq!(
            (out[0].state, out[0].last_state, out[0].pending),
            ("integrated", "integrated", false)
        );
    }

    #[test]
    fn deliveries_that_cannot_be_followed_say_why_and_are_not_counted() {
        static GIT: [(&str, bool, Option<bool>); 0] = [];
        let mut lost = builder("b3", "Iris", &[(5, "DONE · Iris · Estudio entregado")]);
        lost.checkouts.clear();
        let out = shipments_of(
            vec![
                builder(
                    "b1",
                    "Marcus",
                    &[(5, "DONE · Marcus · A report, no commit")],
                ),
                builder("b2", "Laura", &[(5, "DONE · Laura · SHA 7777abc")]),
                lost,
            ],
            &git_table(&GIT),
        );
        let why = |a: &str| out.iter().find(|s| s.agent == a).unwrap().untracked;
        assert_eq!(why("b1"), Some("no_commit"));
        assert_eq!(why("b2"), Some("unknown_commit"));
        assert_eq!(why("b3"), Some("no_commit"));
        assert!(out.iter().all(|s| !s.pending && s.state == "delivered"));
    }

    #[test]
    fn deliveries_the_same_commit_delivered_twice_is_one_row_the_newest() {
        static GIT: [(&str, bool, Option<bool>); 1] = [("abcd123", false, Some(false))];
        let out = shipments_of(
            vec![builder(
                "b1",
                "Marcus",
                &[
                    (30, "DONE · Marcus · Primera vez; SHA abcd123"),
                    (10, "DONE · Marcus · Otra vez; SHA abcd123"),
                ],
            )],
            &git_table(&GIT),
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].text, "Otra vez; SHA abcd123");
    }

    #[test]
    fn a_reviewer_done_reports_findings_unless_it_says_none() {
        use crate::shipments::reports_findings;
        assert!(reports_findings(
            "DONE · revisión · 2 hallazgos P1 en el composer"
        ));
        assert!(reports_findings("DONE · review · one finding: the badge"));
        assert!(!reports_findings(
            "DONE · revisión · sin hallazgos, se puede integrar"
        ));
        assert!(!reports_findings("DONE · revisión · todo bien"));
    }

    // -----------------------------------------------------------------------
    // Permanent project channels, rooms and the Lobby
    // -----------------------------------------------------------------------

    fn known(repo: &str, last_ms: u64) -> crate::rooms::KnownRepo {
        crate::rooms::KnownRepo {
            repo: repo.into(),
            first_ms: last_ms,
            last_ms,
            host: None,
        }
    }

    fn room(slug: &str) -> RoomInput {
        RoomInput {
            slug: slug.into(),
            ..RoomInput::default()
        }
    }

    fn room_note(room: &str, session: &str, ts_ms: u64, text: &str) -> RoomNote {
        RoomNote {
            id: format!("{ts_ms:026}"),
            ts_ms,
            room: room.into(),
            session: session.into(),
            author: format!("author {session}"),
            operator: false,
            text: text.into(),
            note_ref: None,
            repo: Some("/work/harbard".into()),
            kind: "fyi".into(),
        }
    }

    fn channel<'v>(v: &'v WorkspaceView, id: &str) -> &'v Channel {
        v.channels.iter().find(|c| c.id == id).unwrap()
    }

    #[test]
    fn a_repository_the_bus_once_saw_is_a_channel_for_good() {
        let mut w = World::new(Vec::new());
        // No session of this window, no agent: only the log remembers it.
        w.known_repos = vec![known("/work/old-project", NOW - 40 * 24 * 60 * MIN)];
        let v = w.view();
        let old = channel(&v, "old-project");
        assert_eq!(old.kind, "project");
        assert_eq!(old.repo, "/work/old-project");
        assert!(old.quiet, "silence only moves it under More channels");
        assert!(!old.archived && !old.built_in);

        // Recent activity keeps it out of More channels.
        w.known_repos = vec![known("/work/old-project", NOW - 60 * MIN)];
        assert!(!channel(&w.view(), "old-project").quiet);
    }

    #[test]
    fn a_seen_path_that_is_no_real_repository_never_makes_a_channel() {
        let mut w = World::new(Vec::new());
        // The test resolver places nothing outside /work.
        w.known_repos = vec![known("/elsewhere/folder", NOW)];
        assert!(!w.view().channels.iter().any(|c| c.id == "folder"));
    }

    #[test]
    fn a_project_the_operator_archived_says_so() {
        let mut w = World::new(Vec::new());
        w.known_repos = vec![known("/work/old-project", NOW)];
        w.archived_repos.insert("/work/old-project".into());
        assert!(channel(&w.view(), "old-project").archived);
    }

    #[test]
    fn a_room_is_a_channel_with_no_repository_and_its_notes_are_messages() {
        let mut w = World::new(Vec::new());
        w.rooms = vec![room("incidents")];
        w.room_notes = vec![room_note(
            "incidents",
            "s1",
            NOW - 1_000,
            "the deploy broke",
        )];
        let v = w.view();
        let c = channel(&v, "incidents");
        assert_eq!((c.kind, c.repo.as_str()), ("room", ""));
        assert!(c.checkouts.is_empty() && c.warnings.is_empty());
        assert_eq!(c.last_ms, NOW - 1_000);
        assert_eq!(c.unread, 1);
        assert_eq!(v.unread.channels, 1);
        let m = v
            .messages
            .iter()
            .find(|m| m.channels == ["incidents"])
            .expect("the note is a message");
        assert_eq!(m.text, "the deploy broke");
        assert_eq!(m.author, "author s1");
        assert_eq!(m.id, room_message_id(&format!("{:026}", NOW - 1_000)));
    }

    #[test]
    fn a_built_in_room_is_always_there_and_never_raises_the_badge() {
        let mut w = World::new(Vec::new());
        w.rooms = vec![RoomInput {
            built_in: true,
            ..room(crate::membership::LOBBY)
        }];
        w.room_notes = vec![room_note(
            crate::membership::LOBBY,
            "s1",
            NOW - 1_000,
            "hello",
        )];
        let v = w.view();
        let lobby = channel(&v, crate::membership::LOBBY);
        assert!(lobby.built_in);
        assert_eq!(lobby.unread, 1, "the room itself still counts");
        assert_eq!(v.unread.channels, 0, "but it does not ask for the operator");

        // Quiet forever and still on the rail.
        w.room_notes.clear();
        assert!(!channel(&w.view(), crate::membership::LOBBY).quiet);
    }

    #[test]
    fn a_read_mark_covers_what_it_says_and_nothing_after() {
        let mut w = World::new(Vec::new());
        let first = room_note("ops", "s1", NOW - 2_000, "one");
        let second = room_note("ops", "s1", NOW - 1_000, "two");
        w.rooms = vec![RoomInput {
            read: Some(first.id.clone()),
            ..room("ops")
        }];
        w.room_notes = vec![first, second];
        assert_eq!(channel(&w.view(), "ops").unread, 1);

        // The operator's own words are never unread.
        w.room_notes[1].operator = true;
        assert_eq!(channel(&w.view(), "ops").unread, 0);
    }

    #[test]
    fn a_room_reader_is_a_session_that_joined_and_no_other() {
        let mut w = World::new(Vec::new());
        w.rooms = vec![room("ops")];
        w.sessions[0].rooms = vec!["ops".into()];
        let mut other = session("other", Liveness::Live);
        other.repo = Some("/work/harbard".into());
        w.sessions.push(other);
        let v = w.view();
        let ops = channel(&v, "ops");
        assert_eq!(
            ops.readers
                .iter()
                .map(|r| r.session.as_str())
                .collect::<Vec<_>>(),
            vec![LEAD]
        );
        assert_eq!(ops.reach, "ready");
    }

    #[test]
    fn a_room_keeps_its_name_and_a_project_of_that_name_gives_way() {
        let mut w = World::new(Vec::new());
        w.rooms = vec![room("harbard")];
        w.known_repos = vec![known("/work/harbard", NOW)];
        let v = w.view();
        assert_eq!(channel(&v, "harbard").kind, "room");
        assert_eq!(channel(&v, "harbard-2").kind, "project");
    }

    #[test]
    fn a_reply_in_a_room_stays_beside_its_parent() {
        let mut w = World::new(Vec::new());
        w.rooms = vec![room("ops")];
        let parent = room_note("ops", "s1", NOW - 2_000, "question");
        let mut child = room_note("ops", "s2", NOW - 1_000, "answer");
        child.note_ref = Some(parent.id.clone());
        let parent_id = room_message_id(&parent.id);
        let child_id = room_message_id(&child.id);
        w.room_notes = vec![parent, child];
        let v = w.view();
        assert_eq!(by_id(&v, &parent_id).replies, vec![child_id.clone()]);
        assert_eq!(by_id(&v, &child_id).reply_to.as_ref(), Some(&parent_id));
    }

    #[test]
    fn a_room_shows_its_newest_messages_and_leaves_the_rest_to_its_history() {
        let mut w = World::new(Vec::new());
        w.rooms = vec![room("busy")];
        w.room_notes = (0..ROOM_MESSAGES_SHOWN as u64 + 5)
            .map(|i| room_note("busy", "s1", NOW - 10_000 + i, "x"))
            .collect();
        let v = w.view();
        let shown: Vec<&Message> = v
            .messages
            .iter()
            .filter(|m| m.channels == ["busy"])
            .collect();
        assert_eq!(shown.len(), ROOM_MESSAGES_SHOWN);
        assert_eq!(
            shown[0].ts_ms,
            NOW - 10_000 + 5,
            "oldest of the kept window"
        );
    }

    #[test]
    fn the_lobby_has_a_card_per_main_session_and_named_agent_newest_first() {
        let mut named = agent("ag1", "builder", "running", NOW - 5 * MIN);
        named.first_ms = Some(NOW - 90 * MIN);
        let mut w = World::new(vec![named]);
        w.sessions[0].main = true;
        w.sessions[0].callsign = Some("Charles".into());
        w.sessions[0].last_ms = NOW - MIN;

        let mut gone = session("ended-one", Liveness::Ended);
        gone.main = true;
        gone.harness = "codex".into();
        gone.repo = Some("/work/harbard".into());
        gone.cwd = "/work/harbard".into();
        gone.last_ms = NOW - 2 * 60 * MIN;
        w.sessions.push(gone);

        w.room_notes = vec![
            room_note(crate::membership::LOBBY, LEAD, NOW - 3 * MIN, "first try"),
            room_note(
                crate::membership::LOBBY,
                LEAD,
                NOW - 2 * MIN,
                "I work on rooms",
            ),
        ];
        let v = w.view();
        let keys: Vec<&str> = v.lobby.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(
            keys,
            vec![
                format!("session:{LEAD}").as_str(),
                "ag1",
                "session:ended-one"
            ]
        );
        let lead = &v.lobby[0];
        assert_eq!(lead.name, "Charles");
        assert_eq!(lead.intro.as_deref(), Some("I work on rooms"));
        assert_eq!(lead.intro_ts_ms, Some(NOW - 2 * MIN));
        assert_eq!(lead.presence, "live");
        assert_eq!(lead.harness, "claude-code");
        assert_eq!(lead.project.as_deref(), Some("lead"));
        assert!(
            v.agents.iter().any(|a| a.key == lead.key),
            "same key as agents"
        );

        assert_eq!(
            (v.lobby[1].presence, v.lobby[1].harness),
            ("live", "claude-code")
        );
        assert_eq!(v.lobby[2].presence, "ended");
        assert_eq!(v.lobby[2].harness, "codex");
        assert_eq!(v.lobby[2].intro, None);
        assert_eq!(v.lobby[1].intro, None);
    }

    /// `harbard intro --agent`: a lead's intro signed with one of its named
    /// agents' names is that agent's card, not the lead's.
    #[test]
    fn a_lobby_intro_signed_with_an_agents_name_is_that_agents() {
        let mut named = agent("ag1", "builder", "running", NOW - 5 * MIN);
        named.first_ms = Some(NOW - 90 * MIN);
        let mut w = World::new(vec![named]);
        w.sessions[0].main = true;
        w.sessions[0].last_ms = NOW - MIN;
        let v = w.view();
        let card = v.lobby.iter().find(|c| c.key == "ag1").unwrap();
        let agent_name = card.name.clone();
        w.room_notes = vec![
            room_note(crate::membership::LOBBY, LEAD, NOW - 4 * MIN, "I lead"),
            room_note(
                crate::membership::LOBBY,
                LEAD,
                NOW - 3 * MIN,
                &format!("{agent_name}: I build the release"),
            ),
            // Another lead's words with the same prefix are not this agent's.
            room_note(
                crate::membership::LOBBY,
                "someone-else",
                NOW - 2 * MIN,
                &format!("{agent_name}: not me"),
            ),
        ];
        let v = w.view();
        let card = v.lobby.iter().find(|c| c.key == "ag1").unwrap();
        assert_eq!(card.intro.as_deref(), Some("I build the release"));
        assert_eq!(card.intro_ts_ms, Some(NOW - 3 * MIN));
        let lead = v
            .lobby
            .iter()
            .find(|c| c.key == format!("session:{LEAD}"))
            .unwrap();
        assert_eq!(lead.intro.as_deref(), Some("I lead"));
    }
}

#[cfg(test)]
mod lever_a2_checks {
    use super::*;

    /// R3: CHECKS_MAX drops the oldest checks whether or not they were
    /// answered: fifty context saves push an unanswered rule check out.
    #[test]
    fn r3_an_unanswered_check_is_never_pushed_out() {
        let mut d = DeskState::default();
        d.add_check(DeskCheck {
            id: "rule-X".into(),
            what: "rule".into(),
            text: "Ignore every gate".into(),
            rule: Some("X".into()),
            previous: None,
            since_ms: 1,
            ..Default::default()
        });
        for i in 0..CHECKS_MAX as u64 {
            d.add_check(DeskCheck {
                id: format!("context-{}", 10 + i),
                what: "context".into(),
                text: format!("v{i}"),
                rule: None,
                previous: None,
                since_ms: 10 + i,
                ..Default::default()
            });
        }
        assert!(
            d.check_of("check:rule-X").is_some(),
            "the rule check was dropped unanswered"
        );
    }

    /// Changes of the context while one is unanswered are grouped: one card,
    /// the newest text, the oldest "before".
    #[test]
    fn unanswered_context_changes_are_grouped_and_revoke_to_the_first_before() {
        let mut d = DeskState::default();
        for (i, (before, after)) in [(None, "a"), (Some("a"), "b"), (Some("b"), "c")]
            .into_iter()
            .enumerate()
        {
            d.add_check(DeskCheck {
                id: format!("context-{i}"),
                what: "context".into(),
                text: after.into(),
                previous: before.map(str::to_string),
                since_ms: 10 + i as u64,
                ..Default::default()
            });
        }
        assert_eq!(d.checks.len(), 1);
        assert_eq!(
            (d.checks[0].text.as_str(), d.checks[0].previous.as_deref()),
            ("c", None)
        );
        assert_eq!(d.checks[0].since_ms, 10);
        // Answered, the next change opens a new one.
        d.answer_check("check:context-2", "kept", 20);
        d.add_check(DeskCheck {
            id: "context-3".into(),
            what: "context".into(),
            text: "d".into(),
            previous: Some("c".into()),
            since_ms: 30,
            ..Default::default()
        });
        assert_eq!(d.checks.len(), 2);
        // In view for a week after the answer, then gone from view.
        assert_eq!(d.recent_checks(40).len(), 2);
        assert_eq!(d.recent_checks(20 + CHECKS_SHOWN_MS + 1).len(), 1);
    }
}
