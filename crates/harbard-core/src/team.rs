//! Read-only team view over Claude Code transcripts: the teammates and
//! subagents a lead session runs, which the bus never sees.
//!
//! An in-process teammate or subagent shares its lead's `session_id`, so no
//! hook gives it an identity of its own (`context.rs`, SPEC E1). Claude Code
//! already writes everything the operator needs to see it, per lead session:
//!
//! ```text
//! ~/.claude/projects/<project>/<session>.jsonl                      the lead
//! ~/.claude/projects/<project>/<session>/subagents/agent-<id>.jsonl  one agent
//! ~/.claude/projects/<project>/<session>/subagents/agent-<id>.meta.json
//! ```
//!
//! This module projects those files into one panel per lead and one row per
//! agent. It is the same contract as [`crate::harness_trace`]: pull-only,
//! never written to the bus, never read outside the Claude projects root, and
//! it keeps no tool output and no v1 message body. What survives a pass is the
//! first line of each message (capped), its size, a hash to spot exact
//! repeats, the paths it names, write paths, and timestamps. A v2 message
//! (`crate::human`) also keeps its technical body, capped and with anything
//! that reads as a credential masked: the operator reads it under
//! "Technical details".
//!
//! The transcript format is not a contract. Unknown record types are skipped
//! and counted, and [`probe`] is the alarm for the day the format moves:
//! transcripts were written but no agent was recognised in them.

use crate::activity::{self, Activity, Call, ToolCall};
use crate::clock::parse_rfc3339_ms;
use serde::Serialize;
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs::{self, File};
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

/// Lead sessions written within this window are shown.
pub const WINDOW_DEFAULT_MS: u64 = 24 * 3_600_000;
/// The widest window a caller may ask for.
pub const WINDOW_MAX_MS: u64 = 7 * 24 * 3_600_000;
/// The format alarm looks at transcripts written this recently.
pub const FORMAT_ALARM_MS: u64 = 24 * 3_600_000;
/// A running agent with no action for longer than this is flagged.
pub const QUIET_RUNNING_MS: u64 = 15 * 60_000;
/// Older Claude Code versions close a turn with `stop_reason: null`. A
/// text-only reply that old with nothing after it is taken as the end of the
/// turn; a fresher one may still be streaming.
const END_GRACE_MS: u64 = 120_000;
/// Cap on any first line kept from a prompt or a message.
const LINE_CAP: usize = 200;
/// Cap on a report path or a write path kept in memory.
const PATH_CAP: usize = 400;
/// Messages listed per agent (the newest); the total is reported beside them.
const MESSAGES_SHOWN: usize = 20;
/// A transcript line larger than this is skipped unparsed and counted.
const LINE_BYTES_MAX: usize = 32 * 1024 * 1024;
/// The four protocol labels (`harbard protocol`, study §5).
pub const LABELS: [&str; 4] = ["START", "BLOCKED", "FINDING", "DONE"];

const LEAD_ID: &str = "lead";

/// Where the transcripts live. Tests point this at a temp directory.
#[derive(Debug, Clone)]
pub struct TeamRoots {
    /// Used only to expand `~` in shell commands and to exclude `~/.claude`
    /// from checkout inference. Nothing under it is read.
    pub home: PathBuf,
    pub claude_projects: PathBuf,
}

impl TeamRoots {
    pub fn from_env() -> Self {
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/"));
        Self {
            claude_projects: home.join(".claude").join("projects"),
            home,
        }
    }
}

// ---------------------------------------------------------------------------
// Wire shape
// ---------------------------------------------------------------------------

/// `GET /api/v1/team`.
#[derive(Debug, Clone, Serialize)]
pub struct TeamView {
    pub v: u32,
    pub now_ms: u64,
    pub window_ms: u64,
    pub quiet_running_ms: u64,
    /// False when the Claude projects root does not exist.
    pub root_found: bool,
    pub leads: Vec<LeadPanel>,
    pub format: FormatProbe,
}

/// Format-change alarm counters (see [`probe`]).
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct FormatProbe {
    /// Files under `*/*/subagents/` modified in the last [`FORMAT_ALARM_MS`].
    pub recent_subagent_files: usize,
    /// Of the agent transcripts among them, how many held a record the
    /// projection recognises.
    pub recent_agents_seen: usize,
    /// Lead sessions with such a recent file.
    pub recent_sessions: usize,
}

impl FormatProbe {
    /// The alarm: transcripts with subagents were written, none was read.
    pub fn format_changed(&self) -> bool {
        self.recent_subagent_files > 0 && self.recent_agents_seen == 0
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LeadPanel {
    pub session: String,
    /// The encoded project directory under `~/.claude/projects`.
    pub project: String,
    /// The lead's working directory as its transcript records it.
    pub cwd: Option<String>,
    /// Newest modification of the lead transcript or any agent file.
    pub last_write_ms: u64,
    /// Newest record in the lead transcript, when one was recognised.
    pub lead_last_ms: Option<u64>,
    pub lead_found: bool,
    pub agents: Vec<AgentRow>,
    pub warnings: Vec<Warning>,
    /// Transcript lines skipped as unreadable or of an unknown shape.
    pub skipped_lines: usize,
    /// The lead's own write spans by directory (see [`AgentRow::write_dirs`]).
    pub lead_write_dirs: Vec<CheckoutUse>,
    /// Every directory the lead ran a command in, and when it last did
    /// (newest [`DIRS_KEPT`]). Not on the wire: `harbard clean` reads it.
    #[serde(skip)]
    pub lead_cmd_dirs: Vec<(String, u64)>,
    /// Working directories Claude Code recorded with a `gitBranch`, for the
    /// lead and its agents: checkouts known without looking at the disk.
    pub git_cwds: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgentRow {
    /// The `<id>` of `agent-<id>.jsonl`.
    pub id: String,
    pub name: Option<String>,
    /// `teammate` (taskKind `in_process_teammate`) or `subagent`.
    pub kind: &'static str,
    pub agent_type: Option<String>,
    pub color: Option<String>,
    pub model: Option<String>,
    /// The short task description the lead gave at launch (`meta.json`).
    pub description: Option<String>,
    /// First line of the launch prompt.
    pub brief: Option<String>,
    pub state: &'static str,
    /// Which evidence decided `state` (stable code, rendered by the console).
    pub state_reason: &'static str,
    pub last_action_ms: Option<u64>,
    /// Modification time of the agent transcript: the newest moment anything
    /// is known about this agent.
    pub seen_ms: u64,
    pub records: usize,
    pub checkouts: Vec<CheckoutUse>,
    /// Edit/Write paths that matched no inferred checkout.
    pub unplaced_writes: usize,
    /// `worktreePath` when the agent was launched with `isolation: worktree`.
    pub worktree: Option<String>,
    pub messages: Vec<MessageRow>,
    pub messages_total: usize,
    /// The newest labelled message this agent sent.
    pub last_label: Option<LabelSeen>,
    /// Oldest record in the agent transcript: when it started.
    pub first_ms: Option<u64>,
    /// Checkouts where this agent ran git without writing (reviews, reads),
    /// placed onto the inferred checkout roots.
    pub git_roots: Vec<String>,
    /// Write spans by the directory written in, before folding into a
    /// checkout root: a consumer that may look at the disk can place them
    /// precisely (a nested worktree, a repository inside another). The most
    /// recent [`DIRS_KEPT`].
    pub write_dirs: Vec<CheckoutUse>,
    /// Directories git ran in, unfolded, at most [`DIRS_KEPT`].
    pub git_dirs: Vec<String>,
    /// Build dirs its commands named (`CARGO_TARGET_DIR=`, `--target-dir`),
    /// at most [`DIRS_KEPT`]. Not on the wire: `harbard clean` reads it.
    #[serde(skip)]
    pub target_dirs: Vec<String>,
    /// What the agent is doing now, from its transcript alone
    /// ([`crate::activity`]); the workspace adds what needs the bus, the
    /// sluice or the desk (dormant, blocked, verified).
    pub activity: Activity,
    /// The hover card's last steps and real progress signals
    /// ([`crate::progress::Progress`]).
    pub progress: crate::progress::Progress,
}

/// Directories kept per agent in `write_dirs` and `git_dirs`.
pub const DIRS_KEPT: usize = 64;

/// The directory of a written path; a git write target is itself one.
fn dir_of(path: &str, is_dir: bool) -> String {
    if is_dir {
        return path.to_string();
    }
    match path.rsplit_once('/') {
        Some(("", _)) | None => "/".to_string(),
        Some((dir, _)) => dir.to_string(),
    }
}

fn dir_spans(writes: &BTreeMap<String, Span>, git_targets: &BTreeSet<String>) -> Vec<CheckoutUse> {
    let mut by_dir: BTreeMap<String, CheckoutUse> = BTreeMap::new();
    for (path, span) in writes {
        let dir = dir_of(path, git_targets.contains(path));
        let c = by_dir.entry(dir.clone()).or_insert(CheckoutUse {
            path: dir,
            writes: 0,
            first_ms: span.first,
            last_ms: span.last,
        });
        c.writes += span.count;
        c.first_ms = c.first_ms.min(span.first);
        c.last_ms = c.last_ms.max(span.last);
    }
    let mut out: Vec<CheckoutUse> = by_dir.into_values().collect();
    out.sort_by_key(|c| std::cmp::Reverse(c.last_ms));
    out.truncate(DIRS_KEPT);
    out
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CheckoutUse {
    pub path: String,
    pub writes: usize,
    pub first_ms: u64,
    pub last_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LabelSeen {
    pub label: String,
    pub version: Option<u32>,
    pub ts_ms: u64,
    pub sentence: String,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct MessageRow {
    /// `out` (this agent sent it) or `in` (it was delivered to this agent).
    pub dir: &'static str,
    /// Recipient for `out`, sender for `in`.
    pub peer: String,
    pub ts_ms: u64,
    pub first_line: String,
    pub label: Option<String>,
    pub chars: usize,
    pub report: Option<String>,
    /// Byte-identical to an earlier message this agent sent.
    pub repeat: bool,
    /// The lead sent it after the agent's last record: not delivered yet.
    pub undelivered: bool,
    /// A structured protocol reply (`shutdown_response`, …), not prose.
    pub structured: Option<String>,
    /// A short hash of the whole body, so exact repeats can be grouped
    /// without keeping the body. Empty for a structured reply.
    pub digest: String,
    /// Commit-like hex tokens (7 to 40 characters) named anywhere in the
    /// body, at most [`SHAS_KEPT`]. Matched against sluice receipts; a token
    /// alone proves nothing.
    pub shas: Vec<String>,
    /// Messages v2: the technical detail after the blank line that follows
    /// the human line ([`crate::human::technical_body`]). `None` for a v1
    /// message, which keeps only its first line as it always did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// The attachment paths named in the message
    /// ([`crate::human::attachment_paths`]): only `…/attachments/<ULID>/<name>`,
    /// never another path from the body, and never on the wire; the
    /// workspace resolves them into attachments.
    #[serde(skip)]
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Warning {
    /// `two_writers`, `blocked_unanswered`, `unlabeled`, `repeat`, `quiet_running`.
    pub kind: &'static str,
    /// Agent ids involved; `lead` stands for the lead session itself.
    pub agents: Vec<String>,
    pub checkout: Option<String>,
    pub since_ms: u64,
    pub count: usize,
}

// ---------------------------------------------------------------------------
// Incremental reading
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub(crate) struct Tail {
    offset: u64,
}

/// Feed every complete line appended since `tail` to `f`. A file that shrank
/// was replaced: the caller resets its accumulator and reads from zero.
pub(crate) fn read_appended(
    path: &Path,
    tail: &mut Tail,
    f: impl FnMut(&[u8]),
) -> Result<(), Shrunk> {
    let Ok(meta) = fs::metadata(path) else {
        return Ok(());
    };
    read_appended_to(path, tail, meta.len(), f)
}

/// [`read_appended`] up to `len`, the size a stat the caller just made gave.
pub(crate) fn read_appended_to(
    path: &Path,
    tail: &mut Tail,
    len: u64,
    mut f: impl FnMut(&[u8]),
) -> Result<(), Shrunk> {
    if len < tail.offset {
        return Err(Shrunk);
    }
    if len == tail.offset {
        return Ok(());
    }
    let Ok(mut file) = File::open(path) else {
        return Ok(());
    };
    if file.seek(SeekFrom::Start(tail.offset)).is_err() {
        return Ok(());
    }
    let mut reader = BufReader::with_capacity(256 * 1024, file.take(len - tail.offset));
    let mut buf = Vec::new();
    loop {
        buf.clear();
        let Ok(n) = reader.read_until(b'\n', &mut buf) else {
            break;
        };
        if n == 0 || buf.last() != Some(&b'\n') {
            // A partial last line is still being written; read it next pass.
            break;
        }
        tail.offset += n as u64;
        f(&buf);
    }
    Ok(())
}

pub(crate) struct Shrunk;

fn mtime_ms(path: &Path) -> Option<u64> {
    let modified = fs::metadata(path).ok()?.modified().ok()?;
    Some(modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as u64)
}

// ---------------------------------------------------------------------------
// One transcript, folded
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Step {
    #[default]
    Nothing,
    /// The model asked for a tool and is waiting on it.
    ToolCall,
    /// A tool answered; the model is thinking about it.
    ToolResult,
    /// A prompt or a message arrived.
    Prompt,
    /// A thinking block with no closing reason yet.
    Thinking,
    /// A reply with no tool call. `closed` when Claude Code wrote a stop reason.
    Reply {
        closed: bool,
    },
    Refused,
    ApiError,
    Interrupted,
}

#[derive(Debug, Clone, Copy, Default)]
struct Span {
    count: usize,
    first: u64,
    last: u64,
}

impl Span {
    fn add(&mut self, ms: u64) {
        if self.count == 0 || ms < self.first {
            self.first = ms;
        }
        self.last = self.last.max(ms);
        self.count += 1;
    }
}

#[derive(Debug, Clone)]
struct Msg {
    dir: &'static str,
    peer: String,
    ts: u64,
    first_line: String,
    label: Option<Label>,
    chars: usize,
    report: Option<String>,
    hash: u64,
    repeat: bool,
    structured: Option<String>,
    shas: Vec<String>,
    body: Option<String>,
    paths: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Label {
    pub label: &'static str,
    pub version: Option<u32>,
    pub name: Option<String>,
    pub sentence: String,
}

#[derive(Debug, Clone, Default)]
struct Transcript {
    tail: Tail,
    is_lead: bool,
    records: usize,
    skipped: usize,
    brief: Option<String>,
    prompt_seen: bool,
    first_action: Option<u64>,
    last_action: Option<u64>,
    step: Step,
    step_ms: u64,
    /// Working directories Claude Code recorded inside a git checkout.
    cwds: BTreeSet<String>,
    /// Directories git ran in (`git -C`, or `cd` then `git`).
    roots: BTreeSet<String>,
    /// Every directory a command ran in, with when it last did: the
    /// record's own cwd, each `cd` target and each `-C` target, and each
    /// build dir a command named. What `harbard clean` must never pull from
    /// under the session.
    cmd_dirs: BTreeMap<String, u64>,
    /// Build dirs a command named (`CARGO_TARGET_DIR=`, `--target-dir`).
    target_dirs: BTreeSet<String>,
    /// `git worktree add` targets: separate checkouts even when nested.
    worktrees: BTreeSet<String>,
    writes: BTreeMap<String, Span>,
    /// Keys of `writes` that are checkouts a git verb changed, not files.
    git_targets: BTreeSet<String>,
    out: Vec<Msg>,
    inbound: Vec<Msg>,
    /// Times a message reached this transcript (kept for the lead too).
    inbound_ms: Vec<u64>,
    /// Times any prompt or message (not a tool result) started a turn.
    prompts_ms: Vec<u64>,
    shutdown_ms: Option<u64>,
    /// Lead only: `TaskStop` targets and when.
    stops: Vec<(String, u64)>,
    /// Tool calls with no result yet, by `tool_use` id.
    pending: Vec<(String, Call)>,
    /// The newest tool call that got its result, and when.
    last_tool: Option<(Call, u64)>,
    /// When the model last got something to think about: a prompt, a
    /// message or a tool result.
    input_ms: u64,
    /// The newest reply with text says it waits for a background job or a
    /// wake-up ([`activity::says_wait`]). Only this bit is kept.
    waits: bool,
    /// Tool calls, commits, the last test counts and the last finished
    /// steps ([`crate::progress::Tally`]): labels and numbers only.
    tally: crate::progress::Tally,
}

/// Calls kept in flight per transcript; a turn never has this many.
const PENDING_MAX: usize = 64;

impl Transcript {
    fn new(is_lead: bool) -> Self {
        Self {
            is_lead,
            ..Self::default()
        }
    }

    fn feed(&mut self, line: &[u8], home: &str) {
        let Some(v) = parse_line(line, &mut self.skipped) else {
            return;
        };
        let Some((typ, ts)) = recognise(&v) else {
            return;
        };
        self.records += 1;
        // Claude Code records `gitBranch` only when the cwd is in a checkout.
        let in_git = v
            .get("gitBranch")
            .and_then(Value::as_str)
            .is_some_and(|b| !b.is_empty());
        if let Some(cwd) = v.get("cwd").and_then(Value::as_str)
            && cwd.starts_with('/')
            && cwd.len() <= PATH_CAP
        {
            self.seen_dir(normalize(cwd), ts);
            if in_git && !self.cwds.contains(cwd) {
                self.cwds.insert(normalize(cwd));
            }
        }
        self.last_action = Some(self.last_action.map_or(ts, |l| l.max(ts)));
        self.first_action = Some(self.first_action.map_or(ts, |f| f.min(ts)));
        match typ {
            "assistant" => self.assistant(&v, ts, home),
            _ => self.user(&v, ts),
        }
    }

    fn assistant(&mut self, v: &Value, ts: u64, home: &str) {
        let msg = v.get("message");
        let stop = msg
            .and_then(|m| m.get("stop_reason"))
            .and_then(Value::as_str);
        let blocks = msg
            .and_then(|m| m.get("content"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let mut tool = false;
        let mut text = false;
        let mut thinking = false;
        let mut said = String::new();
        for block in blocks {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => {
                    tool = true;
                    self.tool_use(block, ts, home);
                    self.track_call(block, ts);
                }
                Some("text") => {
                    text = true;
                    if let Some(t) = block.get("text").and_then(Value::as_str) {
                        said.push_str(t);
                        said.push(' ');
                    }
                }
                Some("thinking") | Some("redacted_thinking") => thinking = true,
                _ => {}
            }
        }
        let api_error = v
            .get("isApiErrorMessage")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if !said.trim().is_empty() {
            self.waits = activity::says_wait(&said);
        }
        self.step = if tool {
            Step::ToolCall
        } else if stop == Some("refusal") {
            Step::Refused
        } else if api_error {
            Step::ApiError
        } else if stop.is_some_and(|s| s != "tool_use") {
            Step::Reply { closed: true }
        } else if text {
            Step::Reply { closed: false }
        } else if thinking {
            Step::Thinking
        } else {
            self.step
        };
        if matches!(
            self.step,
            Step::Reply { closed: true } | Step::Refused | Step::ApiError
        ) {
            for (_, call) in std::mem::take(&mut self.pending) {
                self.tally.stopped(&call, ts);
            }
        }
        self.step_ms = ts;
    }

    /// A call starts: it is in flight until a result names its id.
    fn track_call(&mut self, block: &Value, ts: u64) {
        let name = block.get("name").and_then(Value::as_str).unwrap_or("");
        let input = block.get("input").unwrap_or(&Value::Null);
        let id = block.get("id").and_then(Value::as_str).unwrap_or("");
        let call = Call::of(name, input, ts);
        self.tally.called();
        self.pending.retain(|(p, _)| p != id);
        if self.pending.len() >= PENDING_MAX {
            self.pending.remove(0);
        }
        self.pending.push((cap(id, 128), call));
    }

    /// A command ran in `dir` at `ts`; the newest [`DIRS_KEPT`] are kept.
    fn seen_dir(&mut self, dir: String, ts: u64) {
        let at = self.cmd_dirs.entry(dir).or_insert(ts);
        *at = (*at).max(ts);
        if self.cmd_dirs.len() > DIRS_KEPT
            && let Some(oldest) = self
                .cmd_dirs
                .iter()
                .min_by_key(|(_, ms)| **ms)
                .map(|(d, _)| d.clone())
        {
            self.cmd_dirs.remove(&oldest);
        }
    }

    fn tool_use(&mut self, block: &Value, ts: u64, home: &str) {
        let name = block.get("name").and_then(Value::as_str).unwrap_or("");
        let input = block.get("input").unwrap_or(&Value::Null);
        match name {
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" => {
                let path = input
                    .get("file_path")
                    .or_else(|| input.get("notebook_path"))
                    .and_then(Value::as_str);
                if let Some(path) = path.and_then(|p| absolute(p, home)) {
                    self.writes.entry(path).or_default().add(ts);
                }
            }
            "Bash" => {
                if let Some(cmd) = input.get("command").and_then(Value::as_str) {
                    let shell = shell_targets(cmd, home);
                    for d in &shell.dirs {
                        self.seen_dir(d.clone(), ts);
                    }
                    for d in shell.target_dirs {
                        self.seen_dir(d.clone(), ts);
                        if self.target_dirs.len() < DIRS_KEPT {
                            self.target_dirs.insert(d);
                        }
                    }
                    self.roots.extend(shell.roots);
                    self.worktrees.extend(shell.worktrees);
                    for target in shell.git_writes {
                        self.writes.entry(target.clone()).or_default().add(ts);
                        self.git_targets.insert(target);
                    }
                }
            }
            "SendMessage" => self.send(input, ts),
            "TaskStop" if self.is_lead => {
                if let Some(id) = input.get("task_id").and_then(Value::as_str) {
                    self.stops.push((cap(id, LINE_CAP), ts));
                }
            }
            _ => {}
        }
    }

    fn send(&mut self, input: &Value, ts: u64) {
        let to = input
            .get("to")
            .or_else(|| input.get("recipient"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let summary = input.get("summary").and_then(Value::as_str);
        let msg = match input.get("message").or_else(|| input.get("content")) {
            Some(Value::String(body)) => {
                let first = first_line(body)
                    .or_else(|| summary.and_then(first_line))
                    .unwrap_or_default();
                let hash = hash_of(body);
                let repeat = self.out.iter().any(|m| m.hash == hash);
                Msg {
                    dir: "out",
                    peer: cap(to, LINE_CAP),
                    ts,
                    label: parse_label(&first),
                    shas: commit_tokens(&first),
                    first_line: first,
                    chars: body.chars().count(),
                    report: visible_report(body),
                    hash,
                    repeat,
                    structured: None,
                    body: crate::human::technical_body(body),
                    paths: crate::human::attachment_paths(body),
                }
            }
            Some(Value::Object(obj)) => {
                let kind = obj
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("structured");
                if kind == "shutdown_response"
                    && obj.get("approve").and_then(Value::as_bool) == Some(true)
                {
                    self.shutdown_ms = Some(ts);
                }
                Msg {
                    dir: "out",
                    peer: cap(to, LINE_CAP),
                    ts,
                    first_line: String::new(),
                    label: None,
                    chars: 0,
                    report: None,
                    hash: 0,
                    repeat: false,
                    structured: Some(cap(kind, 64)),
                    shas: Vec::new(),
                    body: None,
                    paths: Vec::new(),
                }
            }
            _ => return,
        };
        self.out.push(msg);
    }

    fn user(&mut self, v: &Value, ts: u64) {
        // Hook and skill context Claude Code adds on its own (`isMeta`) is
        // neither a prompt nor a result: it starts no turn and ends no call.
        if v.get("isMeta").and_then(Value::as_bool) == Some(true) {
            return;
        }
        let content = v.pointer("/message/content");
        let mut texts: Vec<&str> = Vec::new();
        let mut tool_result = false;
        match content {
            Some(Value::String(s)) => texts.push(s),
            Some(Value::Array(blocks)) => {
                for block in blocks {
                    match block.get("type").and_then(Value::as_str) {
                        Some("tool_result") => {
                            tool_result = true;
                            let id = block
                                .get("tool_use_id")
                                .and_then(Value::as_str)
                                .unwrap_or("");
                            if let Some(at) = self.pending.iter().position(|(p, _)| p == id) {
                                let (_, mut call) = self.pending.remove(at);
                                self.tally.finish(&call, ts, block);
                                // Only a quick tool's face outlives its
                                // result; nothing else of a finished call is
                                // kept.
                                if !activity::lingers(&call.name) {
                                    call.arg = None;
                                }
                                self.last_tool = Some((call, ts));
                            }
                        }
                        Some("text") => {
                            if let Some(t) = block.get("text").and_then(Value::as_str) {
                                texts.push(t);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        self.input_ms = ts;
        if tool_result {
            self.step = Step::ToolResult;
            self.step_ms = ts;
            return;
        }
        // A prompt or a message: the wait claim is answered. A call still
        // in flight keeps running (a teammate message can land mid-tool);
        // only its result or the end of the turn closes it.
        self.waits = false;
        let interrupted = texts.iter().any(|t| t.contains("[Request interrupted"));
        self.prompts_ms.push(ts);
        let first_prompt = !self.prompt_seen;
        self.prompt_seen = true;
        for text in &texts {
            let wrapped = teammate_messages(text);
            if wrapped.is_empty() {
                if first_prompt && self.brief.is_none() {
                    self.brief = first_line(text);
                }
                continue;
            }
            for (i, (from, body)) in wrapped.into_iter().enumerate() {
                if first_prompt && i == 0 && self.brief.is_none() {
                    // The launch prompt: it is the brief, not a message.
                    self.brief = first_line(body);
                    continue;
                }
                self.inbound_ms.push(ts);
                if self.is_lead {
                    // A teammate's report to the lead is already that
                    // teammate's outgoing message; keep only the time.
                    continue;
                }
                let first = first_line(body).unwrap_or_default();
                let report = report_path(&first);
                self.inbound.push(Msg {
                    dir: "in",
                    peer: cap(from, LINE_CAP),
                    ts,
                    label: parse_label(&first),
                    shas: commit_tokens(&first),
                    first_line: first,
                    chars: body.chars().count(),
                    report,
                    hash: hash_of(body),
                    repeat: false,
                    structured: None,
                    // A lead's brief or correction is not the agent's report:
                    // its body stays in the transcript.
                    body: None,
                    paths: crate::human::attachment_paths(body),
                });
            }
        }
        self.step = if interrupted {
            Step::Interrupted
        } else {
            Step::Prompt
        };
        self.step_ms = ts;
    }
}

/// Parse one transcript line, cheaply refusing the large API request records.
pub(crate) fn parse_line(line: &[u8], skipped: &mut usize) -> Option<Value> {
    let line = line.trim_ascii();
    if line.is_empty() {
        return None;
    }
    // `api-request*` records carry the whole request body and are irrelevant
    // here; they always open with their type, so they are refused unparsed.
    if line.starts_with(br#"{"type":"api-request"#) {
        return None;
    }
    if line.len() > LINE_BYTES_MAX {
        *skipped += 1;
        return None;
    }
    match serde_json::from_slice::<Value>(line) {
        Ok(v) if v.is_object() => Some(v),
        _ => {
            *skipped += 1;
            None
        }
    }
}

/// The records the projection understands: `assistant` and `user`, with a
/// UTC timestamp. Everything else (attachments, system, future types) is
/// context Claude Code keeps for itself.
pub(crate) fn recognise(v: &Value) -> Option<(&str, u64)> {
    let typ = v.get("type").and_then(Value::as_str)?;
    if typ != "assistant" && typ != "user" {
        return None;
    }
    let ts = v
        .get("timestamp")
        .and_then(Value::as_str)
        .and_then(parse_rfc3339_ms)?;
    v.get("message")?;
    Some((typ, ts))
}

// ---------------------------------------------------------------------------
// Text helpers: first lines, labels, report paths, teammate envelopes
// ---------------------------------------------------------------------------

fn cap(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// The first non-empty line, capped. `None` for an empty text.
pub fn first_line(text: &str) -> Option<String> {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(|l| cap(l, LINE_CAP))
}

/// `<LABEL>[ vN] · <name> · <sentence>` (study §5, rule 1).
pub fn parse_label(first: &str) -> Option<Label> {
    let mut parts = first.split(" · ").map(str::trim);
    let head = parts.next()?;
    let rest: Vec<&str> = parts.collect();
    let (word, version) = match head.split_once(' ') {
        Some((word, v)) => {
            let n = v.trim().strip_prefix('v')?.parse::<u32>().ok()?;
            (word, Some(n))
        }
        None => (head, None),
    };
    let label = *LABELS.iter().find(|l| **l == word)?;
    match (rest.as_slice(), version) {
        ([name, sentence @ ..], _) if !name.is_empty() && !sentence.is_empty() => {
            let sentence = sentence.join(" · ");
            if sentence.is_empty() {
                return None;
            }
            Some(Label {
                label,
                version,
                name: Some(name.to_string()),
                sentence,
            })
        }
        // `DONE v2 · what changed`: a revision may drop the name.
        ([sentence], Some(_)) if !sentence.is_empty() => Some(Label {
            label,
            version,
            name: None,
            sentence: sentence.to_string(),
        }),
        _ => None,
    }
}

const REPORT_EXTS: [&str; 8] = [
    ".md",
    ".markdown",
    ".txt",
    ".json",
    ".html",
    ".pdf",
    ".log",
    ".csv",
];

/// The first absolute path in the message that names a report-like file,
/// preferring Markdown.
pub fn report_path(text: &str) -> Option<String> {
    let mut other = None;
    for raw in text.split(|c: char| c.is_whitespace()) {
        // Quotes, brackets and sentence punctuation around a path, in any order.
        let token = raw
            .trim_start_matches(['`', '\'', '"', '(', '[', '<', '*'])
            .trim_end_matches([
                '`', '\'', '"', ')', ']', '>', ',', ';', '*', '.', ':', '!', '?',
            ]);
        if !(token.starts_with('/') || token.starts_with("~/")) || token.len() > PATH_CAP {
            continue;
        }
        let token = token.split_once(":").map_or(token, |(p, _)| p);
        let lower = token.to_ascii_lowercase();
        if lower.ends_with(".md") || lower.ends_with(".markdown") {
            return Some(token.to_string());
        }
        if other.is_none() && REPORT_EXTS.iter().any(|e| lower.ends_with(e)) {
            other = Some(token.to_string());
        }
    }
    other
}

/// The report a message names, from what may leave the transcript: its
/// first line, and a v2 message's technical body. A v1 body's paths stay
/// in the transcript.
fn visible_report(body: &str) -> Option<String> {
    let first = first_line(body).unwrap_or_default();
    let shown = match crate::human::technical_body(body) {
        Some(tech) => format!("{first}\n{tech}"),
        None => first,
    };
    report_path(&shown)
}

/// `<teammate-message teammate_id="x" …>body</teammate-message>` envelopes in
/// a user record, as `(sender, body)`.
fn teammate_messages(text: &str) -> Vec<(&str, &str)> {
    const OPEN: &str = "<teammate-message";
    const CLOSE: &str = "</teammate-message>";
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        let after = &rest[start + OPEN.len()..];
        let Some(gt) = after.find('>') else { break };
        let attrs = &after[..gt];
        let body_and_more = &after[gt + 1..];
        let (body, next) = match body_and_more.find(CLOSE) {
            Some(end) => (&body_and_more[..end], &body_and_more[end + CLOSE.len()..]),
            None => (body_and_more, ""),
        };
        let from = attr(attrs, "teammate_id").unwrap_or("");
        out.push((from, body));
        rest = next;
    }
    out
}

fn attr<'a>(attrs: &'a str, key: &str) -> Option<&'a str> {
    let needle = format!("{key}=\"");
    let start = attrs.find(&needle)? + needle.len();
    let len = attrs[start..].find('"')?;
    Some(&attrs[start..start + len])
}

fn hash_of(text: &str) -> u64 {
    let mut h = DefaultHasher::new();
    text.hash(&mut h);
    h.finish()
}

/// Commit-like tokens kept per message.
pub const SHAS_KEPT: usize = 4;

/// Hex tokens of 7 to 40 characters with at least one digit and one letter,
/// in order of appearance. Lexical: a token is a candidate, never evidence.
pub fn commit_tokens(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in text.split(|c: char| !c.is_ascii_alphanumeric()) {
        if !(7..=40).contains(&raw.len()) || !raw.bytes().all(|b| b.is_ascii_hexdigit()) {
            continue;
        }
        let has_digit = raw.bytes().any(|b| b.is_ascii_digit());
        let has_letter = raw.bytes().any(|b| b.is_ascii_alphabetic());
        let token = raw.to_ascii_lowercase();
        if has_digit && has_letter && !out.contains(&token) {
            out.push(token);
            if out.len() == SHAS_KEPT {
                break;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Paths and shell: where an agent writes
// ---------------------------------------------------------------------------

/// Collapse `.`/`..`/`//` and drop a trailing slash. Purely lexical.
fn normalize(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    let mut out = String::from("/");
    out.push_str(&parts.join("/"));
    cap(&out, PATH_CAP)
}

fn absolute(path: &str, home: &str) -> Option<String> {
    let path = path.trim().trim_matches(|c| c == '\'' || c == '"');
    if path.is_empty() || path.contains(['$', '`', '*', '?']) {
        let expanded = path
            .strip_prefix("${HOME}")
            .or_else(|| path.strip_prefix("$HOME"))
            .map(|r| format!("{home}{r}"))?;
        return absolute(&expanded, home);
    }
    if path == "~" {
        return Some(normalize(home));
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return Some(normalize(&format!("{home}/{rest}")));
    }
    path.starts_with('/').then(|| normalize(path))
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ShellTargets {
    roots: Vec<String>,
    git_writes: Vec<String>,
    worktrees: Vec<String>,
    /// Every `cd` and `-C` target, git or not.
    dirs: Vec<String>,
    /// Build dirs named by `CARGO_TARGET_DIR=<dir>` (as a prefix or an
    /// `export`) or `--target-dir <dir>` / `--target-dir=<dir>`.
    target_dirs: Vec<String>,
}

/// Git verbs that can change a checkout's working tree, index or branch.
/// A verb here is a write only when the gate's classifier agrees for the
/// whole command ([`crate::write_class::classify_shell`]): `git stash list`
/// reads, `git stash` writes.
const GIT_WRITES: [&str; 17] = [
    "add",
    "am",
    "apply",
    "checkout",
    "cherry-pick",
    "clean",
    "commit",
    "merge",
    "mv",
    "pull",
    "rebase",
    "reset",
    "restore",
    "revert",
    "rm",
    "stash",
    "switch",
];

/// Lines of a command with heredoc bodies removed: a `cat > f <<'EOF'` body
/// is file content, not shell, and must not read as `cd`/`git`.
pub(crate) fn without_heredocs(cmd: &str) -> String {
    let mut out = String::with_capacity(cmd.len());
    let mut until: Option<String> = None;
    for line in cmd.lines() {
        if let Some(end) = &until {
            if line.trim() == end {
                until = None;
            }
            continue;
        }
        out.push_str(line);
        out.push('\n');
        if let Some(at) = line.find("<<") {
            let word = line[at + 2..]
                .trim_start_matches('-')
                .trim_start()
                .split(|c: char| c.is_whitespace() || c == ';' || c == ')')
                .next()
                .unwrap_or("")
                .trim_matches(|c| c == '\'' || c == '"');
            if !word.is_empty() && !word.starts_with('<') {
                until = Some(word.to_string());
            }
        }
    }
    out
}

/// Where git ran (`git -C <abs>`, or a `cd <abs>` followed by `git`), the
/// checkouts a mutating git verb touched, and `git worktree add` targets.
/// Lexical only: nothing is resolved on disk.
fn shell_targets(cmd: &str, home: &str) -> ShellTargets {
    let mut out = ShellTargets::default();
    let mut cur: Option<String> = None;
    let mut seg = String::new();
    let mut segments = Vec::new();
    let cmd = without_heredocs(cmd);
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            ';' | '\n' | '(' | ')' => segments.push(std::mem::take(&mut seg)),
            '&' | '|' => {
                if chars.peek() == Some(&c) {
                    chars.next();
                }
                segments.push(std::mem::take(&mut seg));
            }
            _ => seg.push(c),
        }
    }
    segments.push(seg);
    for segment in segments {
        let tokens: Vec<&str> = segment.split_whitespace().collect();
        for (k, tok) in tokens.iter().enumerate() {
            let named = tok
                .strip_prefix("CARGO_TARGET_DIR=")
                .or_else(|| tok.strip_prefix("--target-dir="))
                .or_else(|| (*tok == "--target-dir").then(|| tokens.get(k + 1).copied())?);
            out.target_dirs
                .extend(named.and_then(|p| absolute(p, home)));
        }
        let mut i = 0;
        while i < tokens.len() && tokens[i].contains('=') && !tokens[i].starts_with('-') {
            i += 1; // VAR=value prefixes
        }
        if tokens.get(i) == Some(&"command") || tokens.get(i) == Some(&"exec") {
            i += 1;
        }
        match tokens.get(i).copied() {
            Some("cd") | Some("pushd") => {
                // A relative `cd` leaves the directory unknown, not unchanged.
                cur = tokens.get(i + 1).and_then(|p| absolute(p, home));
                out.dirs.extend(cur.clone());
            }
            Some("git") => {
                let mut j = i + 1;
                let mut target = cur.clone();
                while let Some(tok) = tokens.get(j) {
                    match *tok {
                        "-C" => {
                            target = tokens.get(j + 1).and_then(|p| absolute(p, home));
                            out.dirs.extend(target.clone());
                            j += 2;
                        }
                        "-c" => j += 2,
                        t if t.starts_with('-') => j += 1,
                        _ => break,
                    }
                }
                let Some(target) = target else { continue };
                out.roots.push(target.clone());
                let verb = tokens.get(j).copied().unwrap_or("");
                if GIT_WRITES.contains(&verb)
                    && crate::write_class::classify_shell(&segment)
                        == crate::write_class::WriteClass::Write
                {
                    out.git_writes.push(target.clone());
                }
                if verb == "worktree" && tokens.get(j + 1) == Some(&"add") {
                    let mut k = j + 2;
                    while let Some(tok) = tokens.get(k) {
                        match *tok {
                            "-b" | "-B" | "--reason" => k += 2,
                            t if t.starts_with('-') => k += 1,
                            _ => break,
                        }
                    }
                    if let Some(path) = tokens.get(k) {
                        let joined = if path.starts_with(['/', '~', '$']) {
                            path.to_string()
                        } else {
                            format!("{target}/{path}")
                        };
                        out.worktrees.extend(absolute(&joined, home));
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// Directories that are never a checkout: temp space and Claude's own state.
fn excluded_root(root: &str, home: &str) -> bool {
    let claude = format!("{home}/.claude");
    root == "/"
        || root == home
        || [
            "/tmp",
            "/private/tmp",
            "/private/var",
            "/var/folders",
            claude.as_str(),
        ]
        .iter()
        .any(|p| root == *p || root.starts_with(&format!("{p}/")))
}

/// Is `inner` strictly inside `outer`?
fn nested(inner: &str, outer: &str) -> bool {
    inner.len() > outer.len() && inner.starts_with(outer) && inner.as_bytes()[outer.len()] == b'/'
}

/// Candidate checkout roots, outermost wins: a subdirectory git ran in
/// (`cd repo/src && git diff`) is the same checkout as `repo`. A nested root
/// stays separate only with evidence that it is its own worktree.
fn checkout_roots(
    candidates: BTreeSet<String>,
    worktrees: &BTreeSet<String>,
    home: &str,
) -> Vec<String> {
    let candidates: Vec<String> = candidates
        .into_iter()
        .filter(|r| !excluded_root(r, home))
        .collect();
    let separate = |r: &str| {
        worktrees.contains(r) || r.contains("/.claude/worktrees/") || r.contains("/.git/")
    };
    candidates
        .iter()
        .filter(|r| separate(r) || !candidates.iter().any(|o| nested(r, o)))
        .cloned()
        .collect()
}

/// The deepest candidate root that contains `path`.
fn place(path: &str, roots: &[String]) -> Option<String> {
    roots
        .iter()
        .filter(|r| path == r.as_str() || nested(path, r))
        .max_by_key(|r| r.len())
        .cloned()
}

// ---------------------------------------------------------------------------
// Meta
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct Meta {
    name: Option<String>,
    description: Option<String>,
    agent_type: Option<String>,
    teammate: bool,
    color: Option<String>,
    model: Option<String>,
    finished: bool,
    worktree: Option<String>,
}

fn read_meta(path: &Path) -> Option<Meta> {
    let raw = fs::read(path).ok()?;
    if raw.len() > 1024 * 1024 {
        return None;
    }
    let v: Value = serde_json::from_slice(&raw).ok()?;
    let s = |k: &str, max: usize| v.get(k).and_then(Value::as_str).map(|x| cap(x, max));
    Some(Meta {
        name: s("name", 80),
        description: s("description", LINE_CAP),
        agent_type: s("agentType", 80),
        teammate: v.get("taskKind").and_then(Value::as_str) == Some("in_process_teammate"),
        color: s("color", 24),
        model: s("model", 80),
        finished: v.get("finished").and_then(Value::as_bool).unwrap_or(false),
        worktree: v.get("worktreePath").and_then(Value::as_str).map(normalize),
    })
}

// ---------------------------------------------------------------------------
// The projector
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct AgentFile {
    transcript: Transcript,
    meta: Option<Meta>,
    meta_mtime: u64,
}

/// Holds per-file read positions so each pass reads only appended bytes.
/// Files that leave the window are forgotten.
#[derive(Debug)]
pub struct TeamProjector {
    roots: TeamRoots,
    agents: HashMap<PathBuf, AgentFile>,
    leads: HashMap<PathBuf, Transcript>,
    scan: crate::team_scan::ScanCache,
}

use crate::team_scan::SessionDir;

/// Every `<project>/<session>/subagents` directory under the root.
fn session_dirs(root: &Path) -> Vec<SessionDir> {
    let mut out = Vec::new();
    // A root on a volume macOS has not cleared yet would block here for as
    // long as the system dialog stays unanswered.
    if !crate::reach::readable(root) {
        return out;
    }
    let Ok(projects) = fs::read_dir(root) else {
        return out;
    };
    for project in projects.flatten() {
        let ppath = project.path();
        if !ppath.is_dir() {
            continue;
        }
        let Ok(sessions) = fs::read_dir(&ppath) else {
            continue;
        };
        for session in sessions.flatten() {
            let spath = session.path();
            let subagents = spath.join("subagents");
            if !subagents.is_dir() {
                continue;
            }
            let name = session.file_name().to_string_lossy().into_owned();
            out.push(SessionDir {
                project: project.file_name().to_string_lossy().into_owned(),
                lead: ppath.join(format!("{name}.jsonl")),
                session: name,
                subagents,
                stamp: None,
            });
        }
    }
    out
}

/// `(id, transcript path, meta path)` for each `agent-<id>.jsonl`.
fn agent_files(dir: &Path) -> Vec<(String, PathBuf, PathBuf)> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let Some(id) = name
            .strip_prefix("agent-")
            .and_then(|r| r.strip_suffix(".jsonl"))
        else {
            continue;
        };
        out.push((
            id.to_string(),
            e.path(),
            dir.join(format!("agent-{id}.meta.json")),
        ));
    }
    out.sort();
    out
}

/// Files in a subagents directory modified since `since`.
fn recent_files(dir: &Path, since: u64) -> usize {
    fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .filter(|e| e.path().is_file())
                .filter(|e| mtime_ms(&e.path()).is_some_and(|m| m >= since))
                .count()
        })
        .unwrap_or(0)
}

/// The format-change alarm, without building the view: every subagent file
/// written in the last [`FORMAT_ALARM_MS`], and how many of the agent
/// transcripts among them hold one record the projection recognises.
pub fn probe(roots: &TeamRoots, now_ms: u64) -> FormatProbe {
    let since = now_ms.saturating_sub(FORMAT_ALARM_MS);
    let mut out = FormatProbe::default();
    for dir in session_dirs(&roots.claude_projects) {
        let recent = recent_files(&dir.subagents, since);
        if recent == 0 {
            continue;
        }
        out.recent_sessions += 1;
        out.recent_subagent_files += recent;
        for (_, path, _) in agent_files(&dir.subagents) {
            if mtime_ms(&path).is_none_or(|m| m < since) {
                continue;
            }
            if holds_a_record(&path) {
                out.recent_agents_seen += 1;
            }
        }
    }
    out
}

fn holds_a_record(path: &Path) -> bool {
    let Ok(file) = File::open(path) else {
        return false;
    };
    let mut reader = BufReader::with_capacity(256 * 1024, file);
    let mut buf = Vec::new();
    let mut skipped = 0;
    loop {
        buf.clear();
        match reader.read_until(b'\n', &mut buf) {
            Ok(0) | Err(_) => return false,
            Ok(_) => {
                if let Some(v) = parse_line(&buf, &mut skipped)
                    && recognise(&v).is_some()
                {
                    return true;
                }
            }
        }
    }
}

impl TeamProjector {
    pub fn new(roots: TeamRoots) -> Self {
        Self {
            roots,
            agents: HashMap::new(),
            leads: HashMap::new(),
            scan: Default::default(),
        }
    }

    /// How long a session outside the window keeps its stats when nothing
    /// visible moved in it ([`crate::team_scan::COLD_RESCAN`] by default).
    pub fn with_cold_rescan(mut self, every: std::time::Duration) -> Self {
        self.scan.cold_rescan = every;
        self
    }

    pub fn roots(&self) -> &TeamRoots {
        &self.roots
    }

    /// The transcript of agent `id` that the last pass read, if any. The
    /// live view of one agent reads this file and no other: a path the
    /// projection already reads, under the Claude projects root.
    pub fn transcript_of(&self, id: &str) -> Option<PathBuf> {
        let file = format!("agent-{id}.jsonl");
        self.agents
            .keys()
            .find(|p| p.file_name().is_some_and(|n| n == file.as_str()))
            .cloned()
    }

    /// One pass: read what was appended since the last pass and project.
    pub fn project(&mut self, now_ms: u64, window_ms: u64) -> TeamView {
        self.project_within(now_ms, window_ms.clamp(FORMAT_ALARM_MS, WINDOW_MAX_MS))
    }

    /// One pass over [`crate::history::WINDOW_MS`], for History. A
    /// projector of its own: a pass keeps only the files of its window, so
    /// sharing one with the shell's 24 h pass would read a month again on
    /// every turn.
    pub fn project_archive(&mut self, now_ms: u64) -> TeamView {
        self.project_within(now_ms, crate::history::WINDOW_MS)
    }

    fn project_within(&mut self, now_ms: u64, window_ms: u64) -> TeamView {
        let since = now_ms.saturating_sub(window_ms);
        let alarm_since = now_ms.saturating_sub(FORMAT_ALARM_MS);
        let home = self
            .roots
            .home
            .to_string_lossy()
            .trim_end_matches('/')
            .to_string();
        let root_found = crate::reach::readable(&self.roots.claude_projects);
        let mut format = FormatProbe::default();
        let mut leads = Vec::new();
        let mut live_agents: HashSet<PathBuf> = HashSet::new();
        let mut live_leads: HashSet<PathBuf> = HashSet::new();

        let dirs = self
            .scan
            .session_dirs(&self.roots.claude_projects, root_found);
        let mut found: HashSet<PathBuf> = HashSet::new();
        for dir in dirs {
            found.insert(dir.subagents.clone());
            let lead_meta = fs::metadata(&dir.lead).ok();
            let lead_mtime = lead_meta
                .as_ref()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64);
            let scan = self.scan.subagents(&dir, lead_meta.as_ref(), since);
            let files = scan.agents.clone();
            let last_write = scan.last_write(lead_mtime);
            let recent = scan.recent(alarm_since);
            if recent > 0 {
                format.recent_sessions += 1;
                format.recent_subagent_files += recent;
            }
            if last_write < since {
                continue;
            }

            let lead = if lead_mtime.is_some() {
                live_leads.insert(dir.lead.clone());
                let t = self
                    .leads
                    .entry(dir.lead.clone())
                    .or_insert_with(|| Transcript::new(true));
                refresh(
                    &dir.lead,
                    lead_meta.as_ref().map(fs::Metadata::len),
                    t,
                    true,
                    &home,
                );
                Some(t.clone())
            } else {
                None
            };

            let mut agents = Vec::new();
            for crate::team_scan::AgentStat {
                id,
                path,
                meta_path,
                mtime,
                len,
                meta_mtime,
            } in files
            {
                live_agents.insert(path.clone());
                let entry = self.agents.entry(path.clone()).or_default();
                let meta_mtime = meta_mtime.unwrap_or(0);
                if entry.meta.is_none() || meta_mtime != entry.meta_mtime {
                    entry.meta = read_meta(&meta_path);
                    entry.meta_mtime = meta_mtime;
                }
                refresh(&path, len, &mut entry.transcript, false, &home);
                let seen_ms = mtime.unwrap_or(0);
                if seen_ms >= alarm_since && entry.transcript.records > 0 {
                    format.recent_agents_seen += 1;
                }
                agents.push((
                    id,
                    entry.meta.clone().unwrap_or_default(),
                    entry.transcript.clone(),
                    seen_ms,
                ));
            }
            leads.push(build_panel(
                &dir,
                lead.as_ref(),
                agents,
                last_write,
                now_ms,
                &home,
            ));
        }

        self.agents.retain(|p, _| live_agents.contains(p));
        self.leads.retain(|p, _| live_leads.contains(p));
        self.scan.retain(&found);
        leads.sort_by(|a, b| {
            b.last_write_ms
                .cmp(&a.last_write_ms)
                .then(a.session.cmp(&b.session))
        });
        TeamView {
            v: 1,
            now_ms,
            window_ms,
            quiet_running_ms: QUIET_RUNNING_MS,
            root_found,
            leads,
            format,
        }
    }
}

/// Fold what was appended to `path` into `t`; start over if the file shrank.
/// `len` is the file's size from this pass's stat; `None` when it had none
/// (the file is gone), which reads nothing.
fn refresh(path: &Path, len: Option<u64>, t: &mut Transcript, is_lead: bool, home: &str) {
    let Some(len) = len else {
        return;
    };
    let mut tail = std::mem::take(&mut t.tail);
    let shrunk = read_appended_to(path, &mut tail, len, |line| t.feed(line, home)).is_err();
    t.tail = tail;
    if shrunk {
        *t = Transcript::new(is_lead);
        let mut tail = Tail::default();
        let _ = read_appended_to(path, &mut tail, len, |line| t.feed(line, home));
        t.tail = tail;
    }
}

// ---------------------------------------------------------------------------
// Panel
// ---------------------------------------------------------------------------

fn state_of(
    meta: &Meta,
    t: &Transcript,
    lead: Option<&Transcript>,
    name: Option<&str>,
    id: &str,
    now_ms: u64,
) -> (&'static str, &'static str) {
    if t.records == 0 {
        return ("unknown", "no_records");
    }
    let inbound_after = |ms: u64| t.inbound_ms.iter().any(|m| *m > ms);
    if let Some(ms) = t.shutdown_ms
        && !inbound_after(ms)
    {
        return ("stopped", "shutdown_approved");
    }
    // A TaskStop ends the turn it lands in. Claude Code still writes what
    // that turn had in flight (a thinking block, a tool call) after the
    // stop, so the stop wins over later records until a new prompt or
    // message starts another turn. A stop from before the agent's first
    // record belongs to an earlier agent with the same name.
    let first = t.first_action.unwrap_or(0);
    if let Some(lead) = lead
        && lead.stops.iter().any(|(target, ms)| {
            (Some(target.as_str()) == name || target == id)
                && *ms >= first
                && !t.prompts_ms.iter().any(|p| p > ms)
        })
    {
        return ("stopped", "task_stop");
    }
    match t.step {
        Step::Refused => return ("stopped", "refusal"),
        Step::ApiError => return ("stopped", "api_error"),
        Step::Interrupted => return ("stopped", "interrupted"),
        _ => {}
    }
    let ended = match t.step {
        Step::Reply { closed: true } => true,
        Step::Reply { closed: false } => now_ms.saturating_sub(t.step_ms) > END_GRACE_MS,
        _ => false,
    };
    if !meta.teammate {
        if meta.finished {
            return ("done", "finished");
        }
        if ended {
            return ("done", "turn_ended");
        }
        return ("running", "mid_turn");
    }
    if ended {
        let last_label = t
            .out
            .iter()
            .rev()
            .find_map(|m| m.label.as_ref().map(|l| (l, m.ts)));
        if let Some((label, ms)) = last_label
            && label.label == "DONE"
            && !inbound_after(ms)
        {
            return ("done", "done_sent");
        }
        return ("idle", "turn_ended");
    }
    ("running", "mid_turn")
}

/// What the transcript alone says the agent is doing. Mid-turn: the call
/// in flight that ranks highest, else a quick tool for [`activity::LINGER_MS`]
/// after its result, else thinking since its last input. A turn that ended
/// saying it waits is `waiting`; a finished one is `finished`. The
/// workspace overlays dormant, blocked and verified.
fn transcript_activity(t: &Transcript, state: &str, reason: &str, now_ms: u64) -> Activity {
    let source = t.last_action.unwrap_or(0);
    let since_input = t.input_ms.max(t.first_action.unwrap_or(0));
    match state {
        "running" => {
            // Labels are built here, for the calls still in flight only.
            let pending: Vec<ToolCall> = t.pending.iter().map(|(_, c)| c.classify()).collect();
            if let Some(best) = pending
                .iter()
                .max_by_key(|c| (c.kind.rank(), std::cmp::Reverse(c.started_ms)))
            {
                let same: Vec<&ToolCall> = pending.iter().filter(|c| c.kind == best.kind).collect();
                let since = same
                    .iter()
                    .map(|c| c.started_ms)
                    .min()
                    .unwrap_or(best.started_ms);
                let detail = match best.kind {
                    activity::ToolKind::Delegate => Some(same.len().to_string()),
                    activity::ToolKind::Other => None,
                    _ => best.detail.clone(),
                };
                return Activity::new(best.kind.state(), since, detail, source);
            }
            // A quick tool keeps its face for LINGER_MS after its result,
            // through the thinking and text records that follow, until a new
            // prompt or a new call.
            if t.input_ms <= t.last_tool.as_ref().map_or(0, |(_, done)| *done)
                && let Some((last, done)) = &t.last_tool
                && activity::lingers(&last.name)
                && now_ms.saturating_sub(*done) <= activity::LINGER_MS
            {
                let call = last.classify();
                return Activity::new(
                    call.kind.state(),
                    call.started_ms,
                    call.detail.clone(),
                    source,
                );
            }
            Activity::new("thinking", since_input, None, source)
        }
        "idle" | "done" if t.waits && reason == "turn_ended" => {
            Activity::new("waiting", t.step_ms, None, source)
        }
        "done" => Activity::new("finished", t.step_ms, None, source),
        _ => Activity::new("normal", source, None, source),
    }
}

fn row(
    id: String,
    meta: Meta,
    t: &Transcript,
    lead: Option<&Transcript>,
    roots: &[String],
    seen_ms: u64,
    now_ms: u64,
) -> AgentRow {
    let (state, state_reason) = state_of(&meta, t, lead, meta.name.as_deref(), &id, now_ms);
    let activity = transcript_activity(t, state, state_reason, now_ms);
    let mut checkouts: BTreeMap<String, CheckoutUse> = BTreeMap::new();
    let mut unplaced = 0;
    for (path, span) in &t.writes {
        match place(path, roots) {
            Some(root) => {
                let c = checkouts.entry(root.clone()).or_insert(CheckoutUse {
                    path: root,
                    writes: 0,
                    first_ms: span.first,
                    last_ms: span.last,
                });
                c.writes += span.count;
                c.first_ms = c.first_ms.min(span.first);
                c.last_ms = c.last_ms.max(span.last);
            }
            None => unplaced += span.count,
        }
    }
    let mut checkouts: Vec<CheckoutUse> = checkouts.into_values().collect();
    checkouts.sort_by_key(|c| std::cmp::Reverse(c.last_ms));
    // Where git ran for this agent. The recorded cwd is left out: an
    // in-process teammate records its lead's cwd, which says nothing about
    // where the teammate works.
    let git_roots: Vec<String> = t
        .roots
        .iter()
        .filter_map(|r| place(r, roots))
        .collect::<BTreeSet<String>>()
        .into_iter()
        .filter(|r| !checkouts.iter().any(|c| &c.path == r))
        .collect();

    let mut msgs: Vec<MessageRow> = t
        .out
        .iter()
        .chain(t.inbound.iter())
        .map(|m| message_row(m, false))
        .collect();
    if let (Some(lead), Some(name)) = (lead, meta.name.as_deref()) {
        let last = t.last_action.unwrap_or(0);
        for m in lead.out.iter().filter(|m| m.peer == name && m.ts > last) {
            let mut r = message_row(m, true);
            r.dir = "in";
            r.peer = LEAD_ID.to_string();
            r.repeat = false;
            msgs.push(r);
        }
    }
    msgs.sort_by_key(|m| m.ts_ms);
    let total = msgs.len();
    if msgs.len() > MESSAGES_SHOWN {
        msgs.drain(..msgs.len() - MESSAGES_SHOWN);
    }
    let last_label = t.out.iter().rev().find_map(|m| {
        m.label.as_ref().map(|l| LabelSeen {
            label: l.label.to_string(),
            version: l.version,
            ts_ms: m.ts,
            sentence: cap(&l.sentence, LINE_CAP),
        })
    });
    AgentRow {
        id,
        name: meta.name,
        kind: if meta.teammate {
            "teammate"
        } else {
            "subagent"
        },
        agent_type: meta.agent_type,
        color: meta.color,
        model: meta.model,
        description: meta.description,
        brief: t.brief.clone(),
        state,
        state_reason,
        last_action_ms: t.last_action,
        seen_ms,
        records: t.records,
        checkouts,
        unplaced_writes: unplaced,
        worktree: meta.worktree,
        messages: msgs,
        messages_total: total,
        last_label,
        first_ms: t.first_action,
        git_roots,
        write_dirs: dir_spans(&t.writes, &t.git_targets),
        git_dirs: t.roots.iter().take(DIRS_KEPT).cloned().collect(),
        target_dirs: t.target_dirs.iter().cloned().collect(),
        activity,
        progress: t.tally.progress(t.pending.iter().map(|(_, c)| c)),
    }
}

fn message_row(m: &Msg, undelivered: bool) -> MessageRow {
    MessageRow {
        dir: m.dir,
        peer: m.peer.clone(),
        ts_ms: m.ts,
        first_line: m.first_line.clone(),
        label: m.label.as_ref().map(|l| match l.version {
            Some(v) => format!("{} v{v}", l.label),
            None => l.label.to_string(),
        }),
        chars: m.chars,
        report: m.report.clone(),
        repeat: m.repeat,
        undelivered,
        structured: m.structured.clone(),
        digest: if m.structured.is_some() {
            String::new()
        } else {
            format!("{:016x}", m.hash)
        },
        shas: m.shas.clone(),
        body: m.body.clone(),
        paths: m.paths.clone(),
    }
}

fn build_panel(
    dir: &SessionDir,
    lead: Option<&Transcript>,
    agents: Vec<(String, Meta, Transcript, u64)>,
    last_write: u64,
    now_ms: u64,
    home: &str,
) -> LeadPanel {
    // Candidate checkout roots need git evidence: git ran there, Claude Code
    // recorded a branch for that cwd, or it is a worktree an agent was given
    // or created. A directory merely `cd`'d into is not a checkout.
    let mut roots: BTreeSet<String> = BTreeSet::new();
    let mut worktrees: BTreeSet<String> = BTreeSet::new();
    for t in lead.into_iter().chain(agents.iter().map(|a| &a.2)) {
        roots.extend(t.roots.iter().cloned());
        roots.extend(t.cwds.iter().cloned());
        worktrees.extend(t.worktrees.iter().cloned());
    }
    for (_, meta, _, _) in &agents {
        worktrees.extend(meta.worktree.iter().cloned());
    }
    roots.extend(worktrees.iter().cloned());
    let roots = checkout_roots(roots, &worktrees, home);

    let mut skipped = lead.map_or(0, |l| l.skipped);
    let mut rows: Vec<AgentRow> = Vec::new();
    let mut warnings = Vec::new();
    for (id, meta, t, seen_ms) in &agents {
        skipped += t.skipped;
        let r = row(id.clone(), meta.clone(), t, lead, &roots, *seen_ms, now_ms);
        agent_warnings(&r, t, lead, now_ms, &mut warnings);
        rows.push(r);
    }

    // Two writers in one checkout: write spans that overlap in time.
    let mut writers: BTreeMap<String, Vec<(String, u64, u64)>> = BTreeMap::new();
    for r in &rows {
        for c in &r.checkouts {
            writers
                .entry(c.path.clone())
                .or_default()
                .push((r.id.clone(), c.first_ms, c.last_ms));
        }
    }
    if let Some(lead) = lead {
        for (path, span) in &lead.writes {
            if let Some(root) = place(path, &roots) {
                let list = writers.entry(root).or_default();
                match list.iter_mut().find(|w| w.0 == LEAD_ID) {
                    Some(w) => {
                        w.1 = w.1.min(span.first);
                        w.2 = w.2.max(span.last);
                    }
                    None => list.push((LEAD_ID.to_string(), span.first, span.last)),
                }
            }
        }
    }
    for (checkout, list) in writers {
        let mut involved: BTreeSet<String> = BTreeSet::new();
        let mut since = u64::MAX;
        for (i, a) in list.iter().enumerate() {
            for b in &list[i + 1..] {
                if a.1 <= b.2 && b.1 <= a.2 {
                    involved.insert(a.0.clone());
                    involved.insert(b.0.clone());
                    since = since.min(a.1.max(b.1));
                }
            }
        }
        if !involved.is_empty() {
            warnings.push(Warning {
                kind: "two_writers",
                count: involved.len(),
                agents: involved.into_iter().collect(),
                checkout: Some(checkout),
                since_ms: since,
            });
        }
    }

    rows.sort_by(|a, b| {
        b.last_action_ms
            .unwrap_or(b.seen_ms)
            .cmp(&a.last_action_ms.unwrap_or(a.seen_ms))
            .then(a.id.cmp(&b.id))
    });
    LeadPanel {
        session: dir.session.clone(),
        project: dir.project.clone(),
        cwd: lead.and_then(|l| l.cwds.iter().next().cloned()),
        git_cwds: lead
            .into_iter()
            .chain(agents.iter().map(|a| &a.2))
            .flat_map(|t| t.cwds.iter().cloned())
            .collect::<BTreeSet<String>>()
            .into_iter()
            .take(DIRS_KEPT)
            .collect(),
        last_write_ms: last_write,
        lead_last_ms: lead.and_then(|l| l.last_action),
        lead_found: lead.is_some_and(|l| l.records > 0),
        agents: rows,
        warnings,
        skipped_lines: skipped,
        lead_write_dirs: lead.map_or_else(Vec::new, |l| dir_spans(&l.writes, &l.git_targets)),
        lead_cmd_dirs: lead.map_or_else(Vec::new, |l| {
            l.cmd_dirs.iter().map(|(d, ms)| (d.clone(), *ms)).collect()
        }),
    }
}

fn agent_warnings(
    r: &AgentRow,
    t: &Transcript,
    lead: Option<&Transcript>,
    now_ms: u64,
    out: &mut Vec<Warning>,
) {
    let one = |kind, since_ms, count| Warning {
        kind,
        agents: vec![r.id.clone()],
        checkout: None,
        since_ms,
        count,
    };
    // BLOCKED with no answer: nothing reached the agent after it, and the
    // lead sent nothing to it either.
    for m in &t.out {
        if m.label.as_ref().is_some_and(|l| l.label == "BLOCKED") {
            let answered = t.inbound_ms.iter().any(|ms| *ms > m.ts)
                || lead.is_some_and(|l| {
                    l.out
                        .iter()
                        .any(|o| Some(o.peer.as_str()) == r.name.as_deref() && o.ts > m.ts)
                });
            if !answered {
                out.push(one("blocked_unanswered", m.ts, 1));
            }
        }
    }
    let unlabeled: Vec<&Msg> = t
        .out
        .iter()
        .filter(|m| m.structured.is_none() && m.label.is_none())
        .collect();
    if let Some(first) = unlabeled.first() {
        out.push(one("unlabeled", first.ts, unlabeled.len()));
    }
    let repeats: Vec<&Msg> = t.out.iter().filter(|m| m.repeat).collect();
    if let Some(first) = repeats.first() {
        out.push(one("repeat", first.ts, repeats.len()));
    }
    if r.state == "running"
        && let Some(last) = r.last_action_ms
        && now_ms.saturating_sub(last) > QUIET_RUNNING_MS
    {
        out.push(one("quiet_running", last, 1));
    }
}

#[cfg(test)]
mod tests {
    //! Synthetic transcripts only, shaped like what Claude Code 2.1.3xx writes:
    //! one JSON object per line, `type` + `timestamp` + `message`, teammate
    //! messages wrapped in `<teammate-message teammate_id=…>`, and the large
    //! `api-request*` records that open with their type. No real transcript
    //! text is used anywhere here.

    use super::*;
    use crate::clock::rfc3339_utc;
    use crate::test_util::TempHome;
    use serde_json::json;
    use std::io::Write as _;
    use std::time::{Duration, SystemTime};

    const NOW: u64 = 1_794_000_000_000;
    const MIN: u64 = 60_000;
    const SID: &str = "11111111-2222-3333-4444-555555555555";

    struct Fixture {
        home: TempHome,
        session: PathBuf,
        lead: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let home = TempHome::new();
            let project = home.path().join(".claude/projects/-work-lead");
            let session = project.join(SID);
            fs::create_dir_all(session.join("subagents")).unwrap();
            let lead = project.join(format!("{SID}.jsonl"));
            File::create(&lead).unwrap();
            Fixture {
                home,
                session,
                lead,
            }
        }

        fn roots(&self) -> TeamRoots {
            TeamRoots {
                home: self.home.path().to_path_buf(),
                claude_projects: self.home.path().join(".claude/projects"),
            }
        }

        fn meta(&self, id: &str, meta: Value) {
            let p = self.session.join(format!("subagents/agent-{id}.meta.json"));
            fs::write(p, serde_json::to_vec(&meta).unwrap()).unwrap();
        }

        fn teammate(&self, id: &str, name: &str) {
            self.meta(
                id,
                json!({"agentType":"general-purpose","description":"Synthetic task","name":name,
                       "taskKind":"in_process_teammate","teamName":"t","color":"blue","model":"opus",
                       "spawnDepth":1,"requestShape":"x","requestNonInteractive":false}),
            );
        }

        fn agent_path(&self, id: &str) -> PathBuf {
            self.session.join(format!("subagents/agent-{id}.jsonl"))
        }

        fn append(&self, path: &Path, records: &[Value]) {
            let mut f = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .unwrap();
            for r in records {
                writeln!(f, "{}", serde_json::to_string(r).unwrap()).unwrap();
            }
        }

        fn agent(&self, id: &str, records: &[Value]) {
            self.append(&self.agent_path(id), records);
        }

        fn view(&self) -> TeamView {
            TeamProjector::new(self.roots()).project(NOW, WINDOW_DEFAULT_MS)
        }
    }

    fn base(typ: &str, ms: u64) -> Value {
        json!({"parentUuid":null,"isSidechain":true,"agentId":"a1","type":typ,"uuid":"u",
               "timestamp":rfc3339_utc(ms),"userType":"external","entrypoint":"cli",
               "cwd":"/work/lead","sessionId":SID,"version":"2.1.318","gitBranch":"main"})
    }

    fn assistant(ms: u64, blocks: Value, stop: Option<&str>) -> Value {
        let mut v = base("assistant", ms);
        v["message"] = json!({"role":"assistant","content":blocks,"stop_reason":stop});
        v
    }

    fn tool(ms: u64, name: &str, input: Value) -> Value {
        assistant(
            ms,
            json!([{"type":"tool_use","id":"t","name":name,"input":input}]),
            Some("tool_use"),
        )
    }

    fn reply(ms: u64, text: &str) -> Value {
        assistant(ms, json!([{"type":"text","text":text}]), Some("end_turn"))
    }

    fn user_text(ms: u64, text: &str) -> Value {
        let mut v = base("user", ms);
        v["message"] = json!({"role":"user","content":text});
        v
    }

    fn tool_result(ms: u64, output: &str) -> Value {
        let mut v = base("user", ms);
        v["message"] = json!({"role":"user","content":[{"type":"tool_result","tool_use_id":"t","content":output}]});
        v["toolUseResult"] = json!({"stdout":output,"stderr":"","interrupted":false});
        v
    }

    fn envelope(from: &str, body: &str) -> String {
        format!(
            "<teammate-message teammate_id=\"{from}\" summary=\"s\">\n{body}\n</teammate-message>"
        )
    }

    fn send(ms: u64, to: &str, body: &str) -> Value {
        tool(
            ms,
            "SendMessage",
            json!({"to":to,"message":body,"summary":"s","type":"message","recipient":to,"content":body}),
        )
    }

    fn noise(ms: u64) -> Vec<Value> {
        vec![
            json!({"type":"api-request","sessionId":SID,"agentId":"a1","id":"r","timestamp":rfc3339_utc(ms),"params":{}}),
            json!({"type":"api-request-blob","sessionId":SID,"agentId":"a1","hash":"h","message":{"x":"y"}}),
            {
                let mut v = base("attachment", ms);
                v["attachment"] = json!({"type":"hook_success"});
                v
            },
        ]
    }

    fn panel(view: &TeamView) -> &LeadPanel {
        assert_eq!(view.leads.len(), 1, "{view:?}");
        &view.leads[0]
    }

    fn agent<'a>(view: &'a TeamView, id: &str) -> &'a AgentRow {
        panel(view)
            .agents
            .iter()
            .find(|a| a.id == id)
            .unwrap_or_else(|| panic!("agent {id} missing"))
    }

    fn kinds(view: &TeamView) -> Vec<&'static str> {
        let mut k: Vec<_> = panel(view).warnings.iter().map(|w| w.kind).collect();
        k.sort_unstable();
        k
    }

    /// A lead reading a builder's checkout is not writing in it, so it must not
    /// raise "Two agents in the same copy". The verdict is the gate's
    /// own classifier ([`crate::write_class`]), one definition for both.
    #[test]
    fn read_only_git_commands_never_count_as_writes() {
        for read in [
            "git status",
            "git status --short",
            "git log --oneline -5",
            "git diff --stat main",
            "git branch --list",
            "git stash list",
            "git stash show -p",
        ] {
            for cmd in [
                format!("git -C /w/builder {}", &read[4..]),
                format!("cd /w/builder && {read}"),
            ] {
                let t = shell_targets(&cmd, "/h");
                assert_eq!(t.roots, vec!["/w/builder".to_string()], "{cmd}");
                assert!(t.git_writes.is_empty(), "{cmd} is a read: {t:?}");
            }
        }
        for write in [
            "git stash",
            "git stash push -m x",
            "git stash pop",
            "git commit -m x",
            "git add .",
            "git switch -c x",
        ] {
            let t = shell_targets(&format!("cd /w/builder && {write}"), "/h");
            assert_eq!(t.git_writes, vec!["/w/builder".to_string()], "{write}");
        }
    }

    #[test]
    fn a_teammate_row_carries_name_brief_state_messages_and_checkout() {
        let fx = Fixture::new();
        fx.teammate("a1", "widget");
        let t0 = NOW - 30 * MIN;
        let mut records = vec![
            user_text(
                t0,
                &envelope(
                    "team-lead",
                    "Build the widget report.\nSecond line stays out.",
                ),
            ),
            tool(
                t0 + MIN,
                "Bash",
                json!({"command":"cd /work/alpha && git status"}),
            ),
            tool_result(t0 + MIN, "TOOL-OUTPUT-SENTINEL"),
            send(
                t0 + 2 * MIN,
                "team-lead",
                "START · widget · Worktree /work/alpha, branch w.",
            ),
            tool(
                t0 + 3 * MIN,
                "Edit",
                json!({"file_path":"/work/alpha/src/lib.rs","old_string":"a","new_string":"b"}),
            ),
            tool(
                t0 + 4 * MIN,
                "Write",
                json!({"file_path":"/work/alpha/docs/x.md","content":"BODY-SENTINEL"}),
            ),
            send(
                t0 + 5 * MIN,
                "team-lead",
                "DONE · widget · Report at /work/reports/widget.md, SHA abc.\nBODY-SENTINEL detail line",
            ),
            reply(t0 + 6 * MIN, "Finished."),
        ];
        records.extend(noise(t0 + 6 * MIN));
        fx.agent("a1", &records);

        let view = fx.view();
        let row = agent(&view, "a1");
        assert_eq!(row.name.as_deref(), Some("widget"));
        assert_eq!(row.kind, "teammate");
        assert_eq!(row.color.as_deref(), Some("blue"));
        assert_eq!(row.brief.as_deref(), Some("Build the widget report."));
        assert_eq!((row.state, row.state_reason), ("done", "done_sent"));
        assert_eq!(row.last_action_ms, Some(t0 + 6 * MIN));
        assert_eq!(
            row.checkouts,
            vec![CheckoutUse {
                path: "/work/alpha".into(),
                writes: 2,
                first_ms: t0 + 3 * MIN,
                last_ms: t0 + 4 * MIN
            }]
        );
        let labels: Vec<_> = row.messages.iter().map(|m| m.label.as_deref()).collect();
        assert_eq!(labels, vec![Some("START"), Some("DONE")]);
        assert_eq!(
            row.messages[1].report.as_deref(),
            Some("/work/reports/widget.md")
        );
        assert_eq!(row.last_label.as_ref().unwrap().label, "DONE");
        assert!(
            panel(&view).warnings.is_empty(),
            "{:?}",
            panel(&view).warnings
        );

        // Only first lines survive: no body or tool output reaches the wire.
        let wire = serde_json::to_string(&view).unwrap();
        assert!(!wire.contains("BODY-SENTINEL"), "{wire}");
        assert!(!wire.contains("TOOL-OUTPUT-SENTINEL"));
        assert!(!wire.contains("Second line stays out"));
    }

    #[test]
    fn a_task_stop_wins_over_records_its_turn_wrote_after_it() {
        // Tonight's shape (harbard-study): the lead stops the teammate by
        // name while it is mid-turn; Claude Code then writes the in-flight
        // thinking block, or a tool call, a few seconds after the stop.
        let fx = Fixture::new();
        let stop_at = NOW - 60 * MIN;
        for (id, name, tail) in [
            (
                "think",
                "harbard-study",
                assistant(
                    stop_at + 6_000,
                    json!([{"type":"thinking","thinking":""}]),
                    None,
                ),
            ),
            (
                "call",
                "recall",
                tool(stop_at + 700, "Bash", json!({"command":"cargo test"})),
            ),
        ] {
            fx.teammate(id, name);
            fx.agent(
                id,
                &[
                    user_text(stop_at - 30 * MIN, &envelope("team-lead", "Task.")),
                    tool(stop_at - 4_000, "Bash", json!({"command":"ls"})),
                    tool_result(stop_at - 3_800, "x"),
                    tail,
                ],
            );
        }
        // Stopped, then given new work: running again.
        fx.teammate("again", "revived");
        fx.agent(
            "again",
            &[
                user_text(stop_at - 30 * MIN, &envelope("team-lead", "Task.")),
                tool(stop_at + 1_000, "Bash", json!({"command":"ls"})),
                user_text(stop_at + 60_000, &envelope("team-lead", "One more thing.")),
                tool(NOW - MIN, "Bash", json!({"command":"ls"})),
            ],
        );
        // A new agent that reuses a stopped name after the stop is not stopped.
        fx.teammate("later", "reused");
        fx.agent(
            "later",
            &[
                user_text(stop_at + 5 * MIN, &envelope("team-lead", "Task.")),
                tool(NOW - MIN, "Bash", json!({"command":"ls"})),
            ],
        );
        fx.append(
            &fx.lead,
            &["harbard-study", "recall", "revived", "reused"]
                .map(|n| tool(stop_at, "TaskStop", json!({"task_id": n}))),
        );
        let view = fx.view();
        let state = |id| {
            let a = agent(&view, id);
            (a.state, a.state_reason)
        };
        assert_eq!(state("think"), ("stopped", "task_stop"));
        assert_eq!(state("call"), ("stopped", "task_stop"));
        assert_eq!(state("again"), ("running", "mid_turn"));
        assert_eq!(state("later"), ("running", "mid_turn"));
        assert!(
            panel(&view)
                .warnings
                .iter()
                .all(|w| w.kind != "quiet_running"
                    || !w.agents.iter().any(|a| a == "think" || a == "call")),
            "a stopped agent is never quiet-running"
        );
    }

    #[test]
    fn states_follow_the_last_record_and_its_age() {
        let fx = Fixture::new();
        // idle: the turn ended without DONE.
        fx.teammate("idle", "idle");
        fx.agent(
            "idle",
            &[
                user_text(NOW - 10 * MIN, &envelope("team-lead", "Task.")),
                reply(NOW - 9 * MIN, "ok"),
            ],
        );
        // running: waiting on a tool, recently.
        fx.teammate("run", "run");
        fx.agent(
            "run",
            &[
                user_text(NOW - 5 * MIN, &envelope("team-lead", "Task.")),
                tool(NOW - MIN, "Bash", json!({"command":"cargo test"})),
            ],
        );
        // running but quiet for 20 minutes.
        fx.teammate("quiet", "quiet");
        fx.agent(
            "quiet",
            &[
                user_text(NOW - 40 * MIN, &envelope("team-lead", "Task.")),
                tool_result(NOW - 20 * MIN, "x"),
            ],
        );
        // stopped by the lead with TaskStop, by name.
        fx.teammate("halt", "halted");
        fx.agent(
            "halt",
            &[
                user_text(NOW - 30 * MIN, &envelope("team-lead", "Task.")),
                tool(NOW - 25 * MIN, "Bash", json!({"command":"sleep 1"})),
            ],
        );
        fx.append(
            &fx.lead,
            &[tool(
                NOW - 24 * MIN,
                "TaskStop",
                json!({"task_id":"halted"}),
            )],
        );
        // stopped: approved its shutdown.
        fx.teammate("bye", "bye");
        fx.agent(
            "bye",
            &[
                user_text(NOW - 30 * MIN, &envelope("team-lead", "Task.")),
                tool(
                    NOW - 29 * MIN,
                    "SendMessage",
                    json!({"to":"team-lead","message":{"type":"shutdown_response","request_id":"r","approve":true}}),
                ),
            ],
        );
        // A subagent: finished in its meta.
        fx.meta(
            "sub",
            json!({"agentType":"Explore","description":"Look","finished":true,"spawnDepth":1}),
        );
        fx.agent(
            "sub",
            &[
                user_text(NOW - 8 * MIN, "Find the thing."),
                reply(NOW - 7 * MIN, "Found."),
            ],
        );
        // An agent file with nothing recognisable in it.
        fx.teammate("empty", "empty");
        fx.agent("empty", &noise(NOW - MIN));
        // Older Claude Code: a text reply without stop reason closes the turn
        // once it is older than the grace period.
        fx.teammate("old", "old");
        fx.agent(
            "old",
            &[
                user_text(NOW - 10 * MIN, &envelope("team-lead", "Task.")),
                assistant(NOW - 5 * MIN, json!([{"type":"text","text":"done"}]), None),
            ],
        );

        let view = fx.view();
        let state = |id| {
            let a = agent(&view, id);
            (a.state, a.state_reason)
        };
        assert_eq!(state("idle"), ("idle", "turn_ended"));
        assert_eq!(state("run"), ("running", "mid_turn"));
        assert_eq!(state("quiet"), ("running", "mid_turn"));
        assert_eq!(state("halt"), ("stopped", "task_stop"));
        assert_eq!(state("bye"), ("stopped", "shutdown_approved"));
        assert_eq!(state("sub"), ("done", "finished"));
        assert_eq!(agent(&view, "sub").kind, "subagent");
        assert_eq!(
            agent(&view, "sub").brief.as_deref(),
            Some("Find the thing.")
        );
        assert_eq!(state("empty"), ("unknown", "no_records"));
        assert_eq!(agent(&view, "empty").last_action_ms, None);
        assert_eq!(state("old"), ("idle", "turn_ended"));
        let quiet: Vec<_> = panel(&view)
            .warnings
            .iter()
            .filter(|w| w.kind == "quiet_running")
            .collect();
        assert_eq!(quiet.len(), 1, "{quiet:?}");
        assert_eq!(quiet[0].agents, vec!["quiet".to_string()]);
        assert_eq!(quiet[0].since_ms, NOW - 20 * MIN);
    }

    #[test]
    fn two_writers_in_one_checkout_only_when_their_spans_overlap() {
        let fx = Fixture::new();
        for (id, name, from, to) in [
            ("a", "a", 40, 20), // writes 40..20 min ago
            ("b", "b", 30, 10), // overlaps a
            ("c", "c", 9, 5),   // after both: no overlap
        ] {
            fx.teammate(id, name);
            fx.agent(
                id,
                &[
                    user_text(NOW - 50 * MIN, &envelope("team-lead", "Task.")),
                    tool(
                        NOW - 50 * MIN,
                        "Bash",
                        json!({"command":"cd /work/shared && git status"}),
                    ),
                    tool(
                        NOW - from * MIN,
                        "Write",
                        json!({"file_path":format!("/work/shared/{id}.txt"),"content":""}),
                    ),
                    tool(
                        NOW - to * MIN,
                        "Edit",
                        json!({"file_path":format!("/work/shared/{id}.txt")}),
                    ),
                    reply(NOW - to * MIN + 1, "ok"),
                ],
            );
        }
        // A separate worktree nested inside the shared checkout is its own checkout.
        fx.teammate("w", "w");
        fx.agent(
            "w",
            &[
                user_text(NOW - 50 * MIN, &envelope("team-lead", "Task.")),
                tool(
                    NOW - 35 * MIN,
                    "Bash",
                    json!({"command":"git -C /work/shared/.claude/worktrees/w commit -m x"}),
                ),
                tool(
                    NOW - 34 * MIN,
                    "Edit",
                    json!({"file_path":"/work/shared/.claude/worktrees/w/a.txt"}),
                ),
                reply(NOW - 33 * MIN, "ok"),
            ],
        );
        let view = fx.view();
        let two: Vec<_> = panel(&view)
            .warnings
            .iter()
            .filter(|w| w.kind == "two_writers")
            .collect();
        assert_eq!(two.len(), 1, "{two:?}");
        assert_eq!(two[0].checkout.as_deref(), Some("/work/shared"));
        assert_eq!(two[0].agents, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(two[0].since_ms, NOW - 30 * MIN);
        assert_eq!(
            agent(&view, "w").checkouts[0].path,
            "/work/shared/.claude/worktrees/w"
        );
        assert_eq!(agent(&view, "w").checkouts[0].writes, 2);
    }

    #[test]
    fn seven_teammates_of_one_lead_in_one_checkout_are_seven_writers() {
        // Tonight's case: the bus sees one session (the lead); the transcripts
        // show seven teammates writing in the same worktree at once.
        let fx = Fixture::new();
        for i in 1..=7u64 {
            let id = format!("m{i}");
            fx.teammate(&id, &format!("mate-{i}"));
            fx.agent(
                &id,
                &[
                    user_text(NOW - 60 * MIN, &envelope("team-lead", "Task.")),
                    tool(
                        NOW - 59 * MIN,
                        "Bash",
                        json!({"command":"git -C /work/repo-wt-x status"}),
                    ),
                    tool(
                        NOW - (40 - i) * MIN,
                        "Write",
                        json!({"file_path":format!("/work/repo-wt-x/f{i}.rs"),"content":""}),
                    ),
                    tool(
                        NOW - 5 * MIN,
                        "Edit",
                        json!({"file_path":format!("/work/repo-wt-x/f{i}.rs")}),
                    ),
                ],
            );
        }
        let view = fx.view();
        let two: Vec<&Warning> = panel(&view)
            .warnings
            .iter()
            .filter(|w| w.kind == "two_writers")
            .collect();
        assert_eq!(two.len(), 1, "{two:?}");
        assert_eq!(two[0].count, 7);
        assert_eq!(two[0].checkout.as_deref(), Some("/work/repo-wt-x"));
        assert!(!two[0].agents.contains(&"lead".to_string()));
    }

    #[test]
    fn a_v2_body_is_kept_masked_and_a_v1_body_is_not() {
        let fx = Fixture::new();
        fx.teammate("h", "human");
        fx.agent(
            "h",
            &[
                user_text(NOW - 30 * MIN, &envelope("team-lead", "Task.")),
                send(
                    NOW - 10 * MIN,
                    "team-lead",
                    concat!("DONE · human · You can now see the screenshots.\n\nRebased onto main at 4e5d6c7.\nScreenshot /h/.harbard/attachments/01K9Z8Y7X6W5V4T3S2R1Q0P9N8/before.png\nto", "ken ghp_abcd", "efghijklmnop1234"),
                ),
                send(
                    NOW - 9 * MIN,
                    "team-lead",
                    "DONE · human · Hecho.\nV1-BODY-SENTINEL",
                ),
            ],
        );
        let view = fx.view();
        let m = &agent(&view, "h").messages;
        let body = m[0].body.as_deref().unwrap();
        assert!(body.starts_with("Rebased onto main at 4e5d6c7."), "{body}");
        assert!(!body.contains("ghp_"), "{body}");
        assert_eq!(
            m[0].paths,
            vec!["/h/.harbard/attachments/01K9Z8Y7X6W5V4T3S2R1Q0P9N8/before.png".to_string()]
        );
        // The SHA in the body is detail, never verification evidence.
        assert!(m[0].shas.is_empty());
        assert_eq!(m[1].body, None);
        let wire = serde_json::to_string(&view).unwrap();
        assert!(!wire.contains("V1-BODY-SENTINEL"), "{wire}");
        assert!(!wire.contains("ghp_"), "{wire}");
    }

    /// A v1 body's paths never leave the transcript, only
    /// attachment paths are kept (and never serialized), and a body is kept
    /// only for what the agent sent.
    #[test]
    fn paths_and_bodies_stay_in_the_transcript() {
        let fx = Fixture::new();
        fx.teammate("p", "private");
        fx.agent(
            "p",
            &[
                user_text(
                    NOW - 30 * MIN,
                    &envelope("team-lead", "Task.\n\nThe lead's own brief, with /Users/p/secret-brief.md"),
                ),
                user_text(
                    NOW - 20 * MIN,
                    &envelope("team-lead", "Course correction.\n\nDetail /Users/p/lead-detail.md"),
                ),
                send(
                    NOW - 10 * MIN,
                    "team-lead",
                    "DONE · private · Hecho.\n/Users/p/clients/acme-confidential.md",
                ),
                send(
                    NOW - 9 * MIN,
                    "team-lead",
                    "DONE · private · Mira.\n\nScreenshot /h/.harbard/attachments/01K9Z8Y7X6W5V4T3S2R1Q0P9N8/before.png y /Users/p/otro.md",
                ),
            ],
        );
        let view = fx.view();
        let m = &agent(&view, "p").messages;
        let inbound = m.iter().find(|x| x.dir == "in").unwrap();
        assert_eq!(
            inbound.body, None,
            "a lead's body is not the agent's report"
        );
        assert!(inbound.paths.is_empty());
        let out: Vec<&MessageRow> = m.iter().filter(|x| x.dir == "out").collect();
        assert!(out[0].paths.is_empty(), "{:?}", out[0].paths);
        assert_eq!(
            out[1].paths,
            vec!["/h/.harbard/attachments/01K9Z8Y7X6W5V4T3S2R1Q0P9N8/before.png".to_string()]
        );
        let wire = serde_json::to_string(&view).unwrap();
        // The v2 body itself is shown by design; nothing else is.
        for leaked in ["acme-despido", "lead-detail", "secret-brief", "\"paths\""] {
            assert!(!wire.contains(leaked), "{leaked} reached the wire");
        }
    }

    #[test]
    fn commit_tokens_come_from_the_first_line_only() {
        // A hex secret further down a body must never be served as a SHA.
        let fx = Fixture::new();
        fx.teammate("s", "secretive");
        fx.agent(
            "s",
            &[
                user_text(NOW - 30 * MIN, &envelope("team-lead", "Task.")),
                send(
                    NOW - 10 * MIN,
                    "team-lead",
                    "DONE · secretive · Hecho en 4e5d6c7.\nclave 9f8e7d6c5b4a39281706f5e4d3c2b1a0deadbeef",
                ),
            ],
        );
        let view = fx.view();
        let out = &agent(&view, "s").messages[0];
        assert_eq!(out.shas, vec!["4e5d6c7"]);
    }

    #[test]
    fn rows_carry_start_time_git_roots_digests_and_commit_tokens() {
        let fx = Fixture::new();
        fx.teammate("r", "reviewer");
        let body = "DONE · reviewer · Reviewed 4e5d6c7 and 1234567890abcdef; report /work/r.md";
        fx.agent(
            "r",
            &[
                user_text(NOW - 30 * MIN, &envelope("team-lead", "Review.")),
                tool(
                    NOW - 29 * MIN,
                    "Bash",
                    json!({"command":"git -C /work/other log --oneline"}),
                ),
                send(NOW - 10 * MIN, "team-lead", body),
                send(NOW - 9 * MIN, "team-lead", body),
                user_text(NOW - 8 * MIN, &envelope("team-lead", "Gracias.")),
                reply(NOW - 7 * MIN, "ok"),
            ],
        );
        let view = fx.view();
        let a = agent(&view, "r");
        assert_eq!(a.first_ms, Some(NOW - 30 * MIN));
        assert_eq!(a.git_roots, vec!["/work/other".to_string()]);
        let out: Vec<&MessageRow> = a.messages.iter().filter(|m| m.dir == "out").collect();
        assert_eq!(out[0].shas, vec!["4e5d6c7", "1234567890abcdef"]);
        assert_eq!(out[0].digest, out[1].digest);
        assert_eq!(out[0].digest.len(), 16);
        assert!(out[1].repeat);
        let inbound = a.messages.iter().find(|m| m.dir == "in").unwrap();
        assert_eq!(inbound.digest.len(), 16);
        // Words and short or digit-only tokens are not commits.
        assert!(commit_tokens("deadbeef cafe 1234567 abcdefa decade").is_empty());
        assert_eq!(
            commit_tokens("a1b2c3d, then 0123456789 and facade1"),
            vec!["a1b2c3d", "facade1"]
        );
    }

    #[test]
    fn a_checkout_needs_git_evidence_and_subdirectories_fold_into_it() {
        let fx = Fixture::new();
        for id in ["a", "b"] {
            fx.teammate(id, id);
            fx.agent(
                id,
                &[
                    user_text(NOW - 30 * MIN, &envelope("team-lead", "Task.")),
                    // A notes folder: entered, written, never a git checkout.
                    tool(
                        NOW - 20 * MIN,
                        "Bash",
                        json!({"command":"cd /notes/today && ls"}),
                    ),
                    tool(
                        NOW - 19 * MIN,
                        "Write",
                        json!({"file_path":format!("/notes/today/{id}.md"),"content":""}),
                    ),
                    // A repository seen from a subdirectory.
                    tool(
                        NOW - 18 * MIN,
                        "Bash",
                        json!({"command":"cd /work/repo/packages/cli && git diff"}),
                    ),
                    tool(
                        NOW - 17 * MIN,
                        "Bash",
                        json!({"command":"git -C /work/repo status"}),
                    ),
                    tool(
                        NOW - 16 * MIN,
                        "Edit",
                        json!({"file_path":format!("/work/repo/packages/cli/{id}.ts")}),
                    ),
                ],
            );
        }
        let view = fx.view();
        let a = agent(&view, "a");
        assert_eq!(a.checkouts.len(), 1, "{:?}", a.checkouts);
        assert_eq!(a.checkouts[0].path, "/work/repo");
        assert_eq!(a.unplaced_writes, 1);
        let two: Vec<_> = panel(&view)
            .warnings
            .iter()
            .filter(|w| w.kind == "two_writers")
            .map(|w| w.checkout.clone().unwrap())
            .collect();
        assert_eq!(two, vec!["/work/repo".to_string()]);
    }

    #[test]
    fn the_lead_counts_as_a_writer() {
        let fx = Fixture::new();
        fx.teammate("a", "a");
        fx.agent(
            "a",
            &[
                user_text(NOW - 20 * MIN, &envelope("team-lead", "Task.")),
                tool(
                    NOW - 15 * MIN,
                    "Edit",
                    json!({"file_path":"/work/lead/src/x.rs"}),
                ),
                tool(
                    NOW - 5 * MIN,
                    "Edit",
                    json!({"file_path":"/work/lead/src/x.rs"}),
                ),
            ],
        );
        fx.append(
            &fx.lead,
            &[tool(
                NOW - 10 * MIN,
                "Write",
                json!({"file_path":"/work/lead/README.md","content":""}),
            )],
        );
        let view = fx.view();
        let w = &panel(&view).warnings;
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0].kind, "two_writers");
        assert_eq!(w[0].agents, vec!["a".to_string(), "lead".to_string()]);
        assert_eq!(w[0].checkout.as_deref(), Some("/work/lead"));
    }

    #[test]
    fn blocked_unlabeled_and_repeated_messages_are_flagged() {
        let fx = Fixture::new();
        // BLOCKED and never answered.
        fx.teammate("b1", "stuck");
        fx.agent(
            "b1",
            &[
                user_text(NOW - 20 * MIN, &envelope("team-lead", "Task.")),
                send(NOW - 10 * MIN, "team-lead", "BLOCKED · stuck · Which port?"),
                reply(NOW - 10 * MIN, "waiting"),
            ],
        );
        // BLOCKED, then answered: the lead's reply reached the agent.
        fx.teammate("b2", "freed");
        fx.agent(
            "b2",
            &[
                user_text(NOW - 20 * MIN, &envelope("team-lead", "Task.")),
                send(NOW - 10 * MIN, "team-lead", "BLOCKED · freed · Which port?"),
                reply(NOW - 10 * MIN, "waiting"),
                user_text(NOW - 8 * MIN, &envelope("team-lead", "Use 17419.")),
                tool(NOW - 7 * MIN, "Bash", json!({"command":"true"})),
            ],
        );
        // BLOCKED, answered by the lead but not delivered yet.
        fx.teammate("b3", "pending");
        fx.agent(
            "b3",
            &[
                user_text(NOW - 20 * MIN, &envelope("team-lead", "Task.")),
                send(
                    NOW - 10 * MIN,
                    "team-lead",
                    "BLOCKED · pending · Which port?",
                ),
                reply(NOW - 10 * MIN, "waiting"),
            ],
        );
        fx.append(&fx.lead, &[send(NOW - 2 * MIN, "pending", "Use 17419.")]);
        // Unlabeled and repeated.
        fx.teammate("n", "noisy");
        fx.agent(
            "n",
            &[
                user_text(NOW - 20 * MIN, &envelope("team-lead", "Task.")),
                send(
                    NOW - 9 * MIN,
                    "team-lead",
                    "Here is my report at /work/r.md",
                ),
                send(
                    NOW - 8 * MIN,
                    "team-lead",
                    "Here is my report at /work/r.md",
                ),
                send(NOW - 7 * MIN, "team-lead", "DONE v2 · fixed the typo"),
                reply(NOW - 7 * MIN, "ok"),
            ],
        );
        let view = fx.view();
        let w = &panel(&view).warnings;
        let blocked: Vec<_> = w
            .iter()
            .filter(|w| w.kind == "blocked_unanswered")
            .collect();
        assert_eq!(blocked.len(), 1, "{w:?}");
        assert_eq!(blocked[0].agents, vec!["b1".to_string()]);
        let unlabeled = w.iter().find(|w| w.kind == "unlabeled").unwrap();
        assert_eq!((unlabeled.agents[0].as_str(), unlabeled.count), ("n", 2));
        let repeat = w.iter().find(|w| w.kind == "repeat").unwrap();
        assert_eq!(
            (repeat.agents[0].as_str(), repeat.since_ms),
            ("n", NOW - 8 * MIN)
        );
        assert_eq!(
            kinds(&view),
            vec!["blocked_unanswered", "repeat", "unlabeled"]
        );

        let pending = agent(&view, "b3");
        let last = pending.messages.last().unwrap();
        assert_eq!(
            (last.dir, last.peer.as_str(), last.undelivered),
            ("in", "lead", true)
        );
        let noisy = agent(&view, "n");
        assert_eq!(noisy.last_label.as_ref().unwrap().version, Some(2));
        assert_eq!(noisy.state, "done");
        assert!(noisy.messages[1].repeat && !noisy.messages[0].repeat);
    }

    #[test]
    fn appended_bytes_are_read_incrementally_and_a_replaced_file_from_zero() {
        let fx = Fixture::new();
        fx.teammate("a", "a");
        fx.agent(
            "a",
            &[
                user_text(NOW - 20 * MIN, &envelope("team-lead", "Task.")),
                tool(NOW - 10 * MIN, "Bash", json!({"command":"true"})),
            ],
        );
        let mut projector = TeamProjector::new(fx.roots());
        assert_eq!(
            agent(&projector.project(NOW, WINDOW_DEFAULT_MS), "a").state,
            "running"
        );

        // A partial line is left for the next pass.
        let path = fx.agent_path("a");
        let full = serde_json::to_string(&reply(NOW - MIN, "ok")).unwrap();
        let (head, tail) = full.split_at(full.len() / 2);
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(head.as_bytes())
            .unwrap();
        assert_eq!(
            agent(&projector.project(NOW, WINDOW_DEFAULT_MS), "a").state,
            "running"
        );
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(format!("{tail}\n").as_bytes())
            .unwrap();
        let view = projector.project(NOW, WINDOW_DEFAULT_MS);
        assert_eq!(agent(&view, "a").state, "idle");
        assert_eq!(agent(&view, "a").records, 3);
        assert_eq!(panel(&view).skipped_lines, 0);

        // Replaced by a shorter file: start again.
        fs::write(
            &path,
            format!(
                "{}\n",
                serde_json::to_string(&user_text(NOW - MIN, &envelope("team-lead", "New task.")))
                    .unwrap()
            ),
        )
        .unwrap();
        let view = projector.project(NOW, WINDOW_DEFAULT_MS);
        assert_eq!(agent(&view, "a").records, 1);
        assert_eq!(agent(&view, "a").brief.as_deref(), Some("New task."));
    }

    #[test]
    fn sessions_outside_the_window_are_not_shown_and_paths_outside_checkouts_are_unplaced() {
        let fx = Fixture::new();
        fx.teammate("a", "a");
        fx.agent(
            "a",
            &[
                user_text(NOW - 20 * MIN, &envelope("team-lead", "Task.")),
                tool(
                    NOW - 19 * MIN,
                    "Bash",
                    json!({"command":"cd /private/tmp/scratch && git add ."}),
                ),
                tool(
                    NOW - 18 * MIN,
                    "Write",
                    json!({"file_path":"/private/tmp/scratch/x","content":""}),
                ),
                tool(
                    NOW - 17 * MIN,
                    "Write",
                    json!({"file_path":"/elsewhere/y.md","content":""}),
                ),
            ],
        );
        let view = fx.view();
        let a = agent(&view, "a");
        assert!(a.checkouts.is_empty(), "{:?}", a.checkouts);
        assert_eq!(a.unplaced_writes, 3);

        // A projection asked about a moment two days later shows nothing; the
        // files themselves are untouched.
        let later = TeamProjector::new(fx.roots()).project(
            mtime_ms(&fx.agent_path("a")).unwrap() + 2 * WINDOW_DEFAULT_MS,
            WINDOW_DEFAULT_MS,
        );
        assert!(later.leads.is_empty());
    }

    fn age(path: &Path, by: Duration) {
        let f = File::options().write(true).open(path).unwrap();
        f.set_modified(SystemTime::now() - by).unwrap();
    }

    /// A session two days old: lead, one agent and its meta aged out of
    /// the window. The projector has seen it once.
    fn old_session(fx: &Fixture, now: u64) -> TeamProjector {
        fx.teammate("a", "a");
        fx.agent(
            "a",
            &[user_text(
                now - 2 * WINDOW_DEFAULT_MS,
                &envelope("team-lead", "Task."),
            )],
        );
        let two_days = Duration::from_millis(2 * WINDOW_DEFAULT_MS);
        age(&fx.agent_path("a"), two_days);
        age(&fx.session.join("subagents/agent-a.meta.json"), two_days);
        age(&fx.lead, two_days);
        TeamProjector::new(fx.roots())
    }

    #[test]
    fn a_session_outside_the_window_comes_back_when_its_lead_writes() {
        let now = crate::clock::now_ms();
        let fx = Fixture::new();
        let mut p = old_session(&fx, now);
        assert!(p.project(now, WINDOW_DEFAULT_MS).leads.is_empty());
        assert!(p.project(now, WINDOW_DEFAULT_MS).leads.is_empty());
        fx.append(&fx.lead, &[user_text(now, "Again.")]);
        let view = p.project(now, WINDOW_DEFAULT_MS);
        assert_eq!(agent(&view, "a").id, "a");
    }

    #[test]
    fn a_new_agent_in_a_session_outside_the_window_shows_on_the_next_pass() {
        let now = crate::clock::now_ms();
        let fx = Fixture::new();
        let mut p = old_session(&fx, now);
        assert!(p.project(now, WINDOW_DEFAULT_MS).leads.is_empty());
        fx.teammate("b", "b");
        fx.agent("b", &[user_text(now, &envelope("team-lead", "Task."))]);
        let view = p.project(now, WINDOW_DEFAULT_MS);
        assert_eq!(agent(&view, "b").id, "b");
    }

    #[test]
    fn a_wider_window_reads_a_session_the_narrow_one_kept() {
        let now = crate::clock::now_ms();
        let fx = Fixture::new();
        let mut p = old_session(&fx, now);
        assert!(p.project(now, WINDOW_DEFAULT_MS).leads.is_empty());
        let view = p.project(now, WINDOW_MAX_MS);
        assert_eq!(agent(&view, "a").id, "a");
    }

    #[test]
    fn an_old_agent_written_again_alone_shows_within_the_cold_rescan() {
        let now = crate::clock::now_ms();
        let fx = Fixture::new();
        // Kept for the default interval: the append moves neither the lead
        // nor the directory, so this pass still reads the kept stats.
        let mut kept = old_session(&fx, now);
        assert!(kept.project(now, WINDOW_DEFAULT_MS).leads.is_empty());
        let mut every = TeamProjector::new(fx.roots()).with_cold_rescan(Duration::ZERO);
        assert!(every.project(now, WINDOW_DEFAULT_MS).leads.is_empty());
        fx.agent("a", &[tool(now, "Bash", json!({"command":"true"}))]);
        assert!(kept.project(now, WINDOW_DEFAULT_MS).leads.is_empty());
        // Past the interval (zero here) the session is stat'ed again.
        let view = every.project(now, WINDOW_DEFAULT_MS);
        assert_eq!(agent(&view, "a").records, 2);
    }

    #[test]
    fn a_session_directory_created_after_a_pass_is_found() {
        let now = crate::clock::now_ms();
        let fx = Fixture::new();
        let mut p = TeamProjector::new(fx.roots());
        assert_eq!(p.project(now, WINDOW_DEFAULT_MS).leads.len(), 1);
        // A second session of the same project, then a new project.
        for (project, sid) in [("-work-lead", "second"), ("-work-other", "third")] {
            let dir = fx.home.path().join(".claude/projects").join(project);
            fs::create_dir_all(dir.join(sid).join("subagents")).unwrap();
            fx.append(&dir.join(format!("{sid}.jsonl")), &[user_text(now, "Hi.")]);
            fx.append(
                &dir.join(sid).join("subagents/agent-x.jsonl"),
                &[user_text(now, &envelope("team-lead", "Task."))],
            );
        }
        let view = p.project(now, WINDOW_DEFAULT_MS);
        let mut sessions: Vec<&str> = view.leads.iter().map(|l| l.session.as_str()).collect();
        sessions.sort_unstable();
        assert_eq!(sessions, [SID, "second", "third"]);
    }

    #[test]
    fn the_format_alarm_fires_when_recent_transcripts_yield_no_agent() {
        let now = crate::clock::now_ms();
        // Nothing at all: no alarm.
        let fx = Fixture::new();
        assert_eq!(probe(&fx.roots(), now), FormatProbe::default());
        assert!(!probe(&fx.roots(), now).format_changed());

        // Today's format: seen.
        fx.teammate("ok", "ok");
        fx.agent(
            "ok",
            &[user_text(now - MIN, &envelope("team-lead", "Task."))],
        );
        let p = probe(&fx.roots(), now);
        assert_eq!(
            (
                p.recent_subagent_files,
                p.recent_agents_seen,
                p.recent_sessions
            ),
            (2, 1, 1)
        );
        assert!(!p.format_changed());

        // A renamed record type: written today, recognised nowhere.
        let fx = Fixture::new();
        fx.teammate("moved", "moved");
        let mut rec = user_text(now - MIN, &envelope("team-lead", "Task."));
        let typ = rec.as_object_mut().unwrap().remove("type").unwrap();
        rec["kind"] = typ;
        fx.agent("moved", &[rec]);
        let p = probe(&fx.roots(), now);
        assert_eq!((p.recent_subagent_files, p.recent_agents_seen), (2, 0));
        assert!(p.format_changed());
        // The view reports the same counters.
        let view = TeamProjector::new(fx.roots()).project(now, WINDOW_DEFAULT_MS);
        assert!(view.format.format_changed(), "{:?}", view.format);
        assert_eq!(agent(&view, "moved").state, "unknown");

        // Renamed files: the agent transcripts are no longer `agent-*.jsonl`.
        let fx = Fixture::new();
        fs::write(fx.session.join("subagents/worker-1.ndjson"), "{}\n").unwrap();
        assert!(probe(&fx.roots(), now).format_changed());

        // The same breakage two days old is history, not an alarm.
        let old = fx.session.join("subagents/worker-1.ndjson");
        age(&old, Duration::from_secs(2 * 86_400));
        assert!(!probe(&fx.roots(), now).format_changed());
    }

    #[test]
    fn labels_follow_the_protocol_first_line() {
        let l = parse_label("DONE · harbard-view · Report at /x.md").unwrap();
        assert_eq!(
            (l.label, l.version, l.name.as_deref()),
            ("DONE", None, Some("harbard-view"))
        );
        assert_eq!(l.sentence, "Report at /x.md");
        assert_eq!(
            parse_label("BLOCKED · a · b · c").unwrap().sentence,
            "b · c"
        );
        assert_eq!(
            parse_label("DONE v2 · what changed").unwrap().version,
            Some(2)
        );
        assert_eq!(
            parse_label("DONE v2 · n · what changed")
                .unwrap()
                .name
                .as_deref(),
            Some("n")
        );
        for bad in [
            "DONE",
            "DONE · only-name",
            "done · a · b",
            "**DONE** · a · b",
            "FINISHED · a · b",
            "DONE vX · a · b",
            "Report ready",
            "",
        ] {
            assert!(parse_label(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn report_paths_and_shell_targets_are_lexical() {
        assert_eq!(
            report_path("see `/a/b/report.md`, and /a/c.json").as_deref(),
            Some("/a/b/report.md")
        );
        assert_eq!(
            report_path("log at (/a/run.log).").as_deref(),
            Some("/a/run.log")
        );
        assert_eq!(report_path("/a/src/main.rs:12 only").as_deref(), None);
        assert_eq!(report_path("no paths here").as_deref(), None);

        let t = shell_targets(
            "cd /w/one && git status; GIT_X=1 git -C ~/w/two -c a=b commit -m x | cat\ncd rel && git add .",
            "/h",
        );
        assert_eq!(t.roots, vec!["/w/one".to_string(), "/h/w/two".to_string()]);
        assert_eq!(t.git_writes, vec!["/h/w/two".to_string()]);
        let t = shell_targets("cd \"$HOME/w/three\" && git commit -am x", "/h");
        assert_eq!(t.git_writes, vec!["/h/w/three".to_string()]);
        // A build dir a command names is a use of it.
        let t = shell_targets(
            "CARGO_TARGET_DIR=$HOME/.cache/a cargo test; export CARGO_TARGET_DIR=${HOME}/.cache/b\ncargo build --target-dir ~/.cache/c && cargo check --target-dir=/x/d; cargo run --target-dir rel",
            "/h",
        );
        assert_eq!(
            t.target_dirs,
            vec!["/h/.cache/a", "/h/.cache/b", "/h/.cache/c", "/x/d"]
        );
        // A bare `cd` is not git evidence; a heredoc body is not shell.
        let t = shell_targets(
            "cd /docs/notes && ls\ncat > /w/t.rs <<'EOF'\ngit -C /w/fake commit -m x\nEOF\ngit -C /w/real add .",
            "/h",
        );
        assert_eq!(t.roots, vec!["/w/real".to_string()]);
        assert_eq!(t.git_writes, vec!["/w/real".to_string()]);
        let t = shell_targets(
            "cd /w/repo && git worktree add -b feat ../repo-wt main && git -C /w/repo worktree add --detach /w/repo/.wt/b",
            "/h",
        );
        assert_eq!(
            t.worktrees,
            vec!["/w/repo-wt".to_string(), "/w/repo/.wt/b".to_string()]
        );
        // Outermost root wins unless the nested one is its own worktree.
        let roots: BTreeSet<String> = [
            "/w/repo",
            "/w/repo/src",
            "/w/repo/.wt/b",
            "/w/repo/.claude/worktrees/x",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        let worktrees: BTreeSet<String> = [String::from("/w/repo/.wt/b")].into();
        assert_eq!(
            checkout_roots(roots, &worktrees, "/h"),
            vec!["/w/repo", "/w/repo/.claude/worktrees/x", "/w/repo/.wt/b"]
        );
        assert_eq!(normalize("/a/./b//c/../d/"), "/a/b/d");
        assert!(excluded_root("/h/.claude/projects", "/h"));
        assert!(excluded_root("/private/tmp/x", "/h"));
        assert!(!excluded_root("/h/Projects/x", "/h"));
    }

    #[test]
    fn unknown_records_are_skipped_and_counted_never_fatal() {
        let fx = Fixture::new();
        fx.teammate("a", "a");
        let path = fx.agent_path("a");
        fx.agent(
            "a",
            &[user_text(NOW - 2 * MIN, &envelope("team-lead", "Task."))],
        );
        fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"not json\n{\"type\":\"future-record\",\"timestamp\":\"x\"}\n[1,2]\n")
            .unwrap();
        fx.agent("a", &[tool(NOW - MIN, "Bash", json!({"command":"true"}))]);
        let view = fx.view();
        assert_eq!(agent(&view, "a").records, 2);
        assert_eq!(agent(&view, "a").state, "running");
        assert_eq!(panel(&view).skipped_lines, 2);
    }

    #[test]
    fn a_missing_root_is_reported_not_invented() {
        let home = TempHome::new();
        let view = TeamProjector::new(TeamRoots {
            home: home.path().into(),
            claude_projects: home.path().join("nope"),
        })
        .project(NOW, WINDOW_DEFAULT_MS);
        assert!(!view.root_found);
        assert!(view.leads.is_empty());
    }

    // -----------------------------------------------------------------------
    // Activity: one test per face the transcript decides, and the
    // transitions between them on a fake clock.
    // -----------------------------------------------------------------------

    fn call(ms: u64, id: &str, name: &str, input: Value) -> Value {
        assistant(
            ms,
            json!([{"type":"tool_use","id":id,"name":name,"input":input}]),
            Some("tool_use"),
        )
    }

    fn calls(ms: u64, list: &[(&str, &str, Value)]) -> Value {
        let blocks: Vec<Value> = list
            .iter()
            .map(|(id, name, input)| json!({"type":"tool_use","id":id,"name":name,"input":input}))
            .collect();
        assistant(ms, Value::Array(blocks), Some("tool_use"))
    }

    fn result(ms: u64, id: &str) -> Value {
        let mut v = base("user", ms);
        v["message"] = json!({"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":"ok"}]});
        v
    }

    /// One teammate with `records`, projected at `now`.
    fn face_at(records: &[Value], now: u64) -> Activity {
        let fx = Fixture::new();
        fx.teammate("a", "a");
        let mut all = vec![user_text(NOW - 30 * MIN, &envelope("team-lead", "Task."))];
        all.extend_from_slice(records);
        fx.agent("a", &all);
        let view = TeamProjector::new(fx.roots()).project(now, WINDOW_DEFAULT_MS);
        agent(&view, "a").activity.clone()
    }

    fn face(records: &[Value]) -> Activity {
        face_at(records, NOW)
    }

    #[test]
    fn activity_thinking_after_a_tool_result_or_a_message() {
        let a = face(&[
            call(NOW - 3 * MIN, "t1", "Bash", json!({"command":"ls"})),
            result(NOW - 2 * MIN, "t1"),
        ]);
        assert_eq!(
            (a.state, a.since_ms, a.detail.as_deref()),
            ("thinking", NOW - 2 * MIN, None)
        );
        let m = face(&[user_text(
            NOW - MIN,
            &envelope("team-lead", "One more thing."),
        )]);
        assert_eq!((m.state, m.since_ms), ("thinking", NOW - MIN));
        // A thinking block is still thinking, since the input.
        let t = face(&[
            result(NOW - 90_000, "x"),
            assistant(
                NOW - 60_000,
                json!([{"type":"thinking","thinking":"…"}]),
                None,
            ),
        ]);
        assert_eq!((t.state, t.since_ms), ("thinking", NOW - 90_000));
        assert_eq!(t.source_ms, NOW - 60_000);
    }

    #[test]
    fn activity_running_while_a_bash_call_has_no_result() {
        let a = face(&[call(
            NOW - 4 * MIN,
            "t1",
            "Bash",
            json!({"command":"cd /x && bun run build 2>&1 | tail"}),
        )]);
        assert_eq!(
            (a.state, a.since_ms, a.detail.as_deref()),
            ("running", NOW - 4 * MIN, Some("bun run build"))
        );
    }

    #[test]
    fn activity_testing_when_the_command_is_a_test_runner() {
        let a = face(&[call(
            NOW - 2 * MIN,
            "t1",
            "Bash",
            json!({"command":"CARGO_TARGET_DIR=/t cargo test --workspace"}),
        )]);
        assert_eq!(
            (a.state, a.detail.as_deref()),
            ("testing", Some("cargo test"))
        );
    }

    #[test]
    fn activity_editing_in_flight_and_for_twenty_seconds_after() {
        let edit = json!({"file_path":"/w/apps/console/src/components/workspace/workspace.css","old_string":"a","new_string":"b"});
        let a = face(&[call(NOW - 5_000, "e1", "Edit", edit.clone())]);
        assert_eq!(
            (a.state, a.detail.as_deref()),
            ("editing", Some("workspace.css"))
        );
        let done = [
            call(NOW - 15_000, "e1", "Edit", edit.clone()),
            result(NOW - 14_000, "e1"),
        ];
        let b = face(&done);
        assert_eq!((b.state, b.since_ms), ("editing", NOW - 15_000));
        // Past the linger, the agent is thinking again.
        let c = face_at(&done, NOW - 14_000 + activity::LINGER_MS + 1);
        assert_eq!(c.state, "thinking");
        for name in ["Write", "MultiEdit"] {
            assert_eq!(
                face(&[call(NOW - 1_000, "w", name, json!({"file_path":"/w/x.rs"}))]).state,
                "editing"
            );
        }
        let nb = face(&[call(
            NOW - 1_000,
            "n",
            "NotebookEdit",
            json!({"notebook_path":"/w/n.ipynb"}),
        )]);
        assert_eq!(
            (nb.state, nb.detail.as_deref()),
            ("editing", Some("n.ipynb"))
        );
    }

    #[test]
    fn activity_reading_for_read_grep_and_glob() {
        let r = face(&[call(
            NOW - 2_000,
            "r",
            "Read",
            json!({"file_path":"/w/crates/harbard-core/src/team.rs"}),
        )]);
        assert_eq!((r.state, r.detail.as_deref()), ("reading", Some("team.rs")));
        let g = face(&[call(
            NOW - 2_000,
            "g",
            "Grep",
            json!({"pattern":"fn state_of","path":"/w"}),
        )]);
        assert_eq!(
            (g.state, g.detail.as_deref()),
            ("reading", Some("fn state_of"))
        );
        let gl = face(&[call(
            NOW - 2_000,
            "g",
            "Glob",
            json!({"pattern":"src/**/*.tsx"}),
        )]);
        assert_eq!((gl.state, gl.detail.as_deref()), ("reading", Some("*.tsx")));
    }

    #[test]
    fn activity_web_shows_the_host_only() {
        let w = face(&[call(
            NOW - 3_000,
            "w",
            "WebFetch",
            json!({"url":"https://docs.rs/serde/latest/serde/?search=x&token=abc","prompt":"p"}),
        )]);
        assert_eq!((w.state, w.detail.as_deref()), ("web", Some("docs.rs")));
        let s = face(&[call(
            NOW - 3_000,
            "w",
            "WebSearch",
            json!({"query":"secret plans"}),
        )]);
        assert_eq!((s.state, s.detail), ("web", None));
    }

    #[test]
    fn activity_delegating_counts_the_agents_in_flight_and_outranks_the_rest() {
        let a = face(&[calls(
            NOW - 4 * MIN,
            &[
                ("d1", "Agent", json!({"prompt":"a"})),
                ("d2", "Task", json!({"prompt":"b"})),
                ("b1", "Bash", json!({"command":"cargo test"})),
            ],
        )]);
        assert_eq!((a.state, a.detail.as_deref()), ("delegating", Some("2")));
        // One returns: still delegating, to one agent.
        let b = face(&[
            calls(
                NOW - 4 * MIN,
                &[("d1", "Agent", json!({})), ("d2", "Agent", json!({}))],
            ),
            result(NOW - MIN, "d1"),
        ]);
        assert_eq!((b.state, b.detail.as_deref()), ("delegating", Some("1")));
    }

    #[test]
    fn activity_waiting_when_the_turn_ends_saying_it_waits() {
        let a = face(&[reply(
            NOW - 3 * MIN,
            "Tests are running in the background; I'll wait for the notification.",
        )]);
        assert_eq!((a.state, a.since_ms), ("waiting", NOW - 3 * MIN));
        let es = face(&[reply(NOW - 3 * MIN, "Lo dejo en segundo plano y te aviso.")]);
        assert_eq!(es.state, "waiting");
        // A plain end of turn is not waiting.
        assert_eq!(face(&[reply(NOW - 3 * MIN, "All set.")]).state, "normal");
        // A new message clears the claim.
        let c = face(&[
            reply(NOW - 3 * MIN, "Waiting for the build."),
            user_text(NOW - MIN, &envelope("team-lead", "Check it now.")),
            reply(NOW - 30_000, "Checked: it passed."),
        ]);
        assert_eq!(c.state, "normal");
    }

    #[test]
    fn activity_finished_after_done_with_nothing_later() {
        let a = face(&[
            send(NOW - 5 * MIN, "team-lead", "DONE · a · finished."),
            reply(NOW - 4 * MIN, "Sent."),
        ]);
        assert_eq!((a.state, a.since_ms), ("finished", NOW - 4 * MIN));
        // A message after the DONE: no longer finished.
        let b = face(&[
            send(NOW - 5 * MIN, "team-lead", "DONE · a · finished."),
            reply(NOW - 4 * MIN, "Sent."),
            user_text(NOW - 3 * MIN, &envelope("team-lead", "One more.")),
        ]);
        assert_eq!(b.state, "thinking");
    }

    #[test]
    fn activity_follows_each_transition_on_a_fake_clock() {
        let fx = Fixture::new();
        fx.teammate("a", "a");
        let mut p = TeamProjector::new(fx.roots());
        let t0 = NOW - 10 * MIN;
        let mut step = |records: &[Value], now: u64| {
            fx.agent("a", records);
            let view = p.project(now, WINDOW_DEFAULT_MS);
            let a = agent(&view, "a").activity.clone();
            (a.state, a.detail)
        };
        let s = |x: &str| Some(x.to_string());
        assert_eq!(
            step(
                &[user_text(t0, &envelope("team-lead", "Task."))],
                t0 + 1_000
            ),
            ("thinking", None)
        );
        assert_eq!(
            step(
                &[call(
                    t0 + 2_000,
                    "r",
                    "Read",
                    json!({"file_path":"/w/a.rs"})
                )],
                t0 + 2_500
            ),
            ("reading", s("a.rs"))
        );
        assert_eq!(
            step(&[result(t0 + 3_000, "r")], t0 + 4_000),
            ("reading", s("a.rs"))
        );
        assert_eq!(step(&[], t0 + 30_000), ("thinking", None));
        assert_eq!(
            step(
                &[call(
                    t0 + 31_000,
                    "b",
                    "Bash",
                    json!({"command":"cargo test -p x"})
                )],
                t0 + 32_000
            ),
            ("testing", s("cargo test"))
        );
        assert_eq!(
            step(&[result(t0 + 90_000, "b")], t0 + 91_000),
            ("thinking", None)
        );
        assert_eq!(
            step(
                &[call(
                    t0 + 92_000,
                    "e",
                    "Edit",
                    json!({"file_path":"/w/b.rs"})
                )],
                t0 + 92_500
            ),
            ("editing", s("b.rs"))
        );
        assert_eq!(
            step(&[result(t0 + 93_000, "e")], t0 + 100_000),
            ("editing", s("b.rs"))
        );
        assert_eq!(
            step(
                &[call(
                    t0 + 101_000,
                    "w",
                    "WebFetch",
                    json!({"url":"https://example.org/x?a=1"})
                )],
                t0 + 101_500
            ),
            ("web", s("example.org"))
        );
        assert_eq!(
            step(
                &[
                    result(t0 + 102_000, "w"),
                    call(t0 + 103_000, "d", "Agent", json!({}))
                ],
                t0 + 104_000
            ),
            ("delegating", s("1"))
        );
        assert_eq!(
            step(
                &[
                    result(t0 + 200_000, "d"),
                    reply(t0 + 201_000, "Running in the background, waiting for it.")
                ],
                t0 + 202_000
            ),
            ("waiting", None)
        );
        assert_eq!(
            step(
                &[user_text(t0 + 300_000, &envelope("team-lead", "Go on."))],
                t0 + 301_000
            ),
            ("thinking", None)
        );
        assert_eq!(
            step(
                &[
                    send(t0 + 302_000, "team-lead", "DONE · a · ok."),
                    reply(t0 + 303_000, "Sent.")
                ],
                t0 + 304_000
            ),
            ("finished", None)
        );
    }

    #[test]
    fn a_bash_command_with_secrets_never_reaches_the_team_json() {
        let (gh, bearer, aws) = (
            format!("ghp_FAKE{}", "x".repeat(20)),
            format!("sk-fake{}", "x".repeat(11)),
            format!("AKIAFAKE{}", "x".repeat(8)),
        );
        let cmd = format!(
            "GH_TOKEN={gh} curl -sS --password hunter2-secret \
                   -H 'Authorization: Bearer {bearer}' 'https://api.example.com/v1/x?key={aws}'"
        );
        let fx = Fixture::new();
        fx.teammate("a", "a");
        fx.agent(
            "a",
            &[
                user_text(NOW - 5 * MIN, &envelope("team-lead", "Task.")),
                call(NOW - MIN, "b", "Bash", json!({"command": cmd})),
            ],
        );
        let view = fx.view();
        let a = agent(&view, "a");
        assert_eq!(
            (a.activity.state, a.activity.detail.as_deref()),
            ("running", Some("curl"))
        );
        let json = serde_json::to_string(&view).unwrap();
        for secret in [
            "ghp_FAKE",
            "hunter2",
            "Bearer",
            "sk-fake",
            "AKIAFAKE",
            "GH_TOKEN",
            "api.example.com",
            "key=",
        ] {
            assert!(!json.contains(secret), "{secret} reached the team JSON");
        }
    }

    // The review's cases (harbard-robots-review.md, M4 and L4).

    fn thinking(ms: u64) -> Value {
        assistant(ms, json!([{"type":"thinking","thinking":"…"}]), None)
    }

    fn meta_record(ms: u64) -> Value {
        let mut v = base("user", ms);
        v["message"] = json!({"role":"user","content":"hook context"});
        v["isMeta"] = json!(true);
        v
    }

    #[test]
    fn the_linger_survives_the_thinking_and_text_after_a_result() {
        let edit = json!({"file_path":"/w/workspace.css","old_string":"a","new_string":"b"});
        let a = face(&[
            call(NOW - 8_000, "e1", "Edit", edit.clone()),
            result(NOW - 7_000, "e1"),
            thinking(NOW - 4_000),
        ]);
        assert_eq!(
            (a.state, a.detail.as_deref()),
            ("editing", Some("workspace.css"))
        );
        let b = face(&[
            call(NOW - 8_000, "r1", "Read", json!({"file_path":"/w/team.rs"})),
            result(NOW - 7_000, "r1"),
            assistant(
                NOW - 3_000,
                json!([{"type":"text","text":"Now the next part."}]),
                None,
            ),
        ]);
        assert_eq!(b.state, "reading");
        // Past 20 s since the result, thinking again.
        let c = face_at(
            &[
                call(NOW - 8_000, "e1", "Edit", edit),
                result(NOW - 7_000, "e1"),
                thinking(NOW - 4_000),
            ],
            NOW - 7_000 + activity::LINGER_MS + 1,
        );
        assert_eq!(c.state, "thinking");
    }

    #[test]
    fn a_hook_record_or_a_message_mid_tool_keeps_the_call_in_flight() {
        let a = face(&[
            call(NOW - 60_000, "b3", "Bash", json!({"command":"bun test"})),
            meta_record(NOW - 59_000),
        ]);
        assert_eq!(
            (a.state, a.detail.as_deref()),
            ("testing", Some("bun test"))
        );
        let b = face(&[
            call(NOW - 60_000, "b3", "Bash", json!({"command":"cargo build"})),
            user_text(NOW - 30_000, &envelope("team-lead", "How is it going?")),
        ]);
        assert_eq!(
            (b.state, b.detail.as_deref()),
            ("running", Some("cargo build"))
        );
        // A hook record is not a prompt: it does not clear a wait claim either.
        let c = face(&[
            reply(NOW - 3 * MIN, "Waiting for the build in the background."),
            meta_record(NOW - MIN),
        ]);
        assert_eq!(c.state, "waiting");
    }

    fn result_of(ms: u64, id: &str, output: &str, is_error: bool) -> Value {
        let mut v = base("user", ms);
        v["message"] = json!({"role":"user","content":[{"type":"tool_result","tool_use_id":id,"content":output,"is_error":is_error}]});
        v
    }

    /// The hover card's real signals ride on each row: tool calls, commits,
    /// the last test counts, the newest five steps, and no output text.
    #[test]
    fn a_row_carries_its_last_steps_calls_commits_and_test_counts() {
        let fx = Fixture::new();
        fx.teammate("a", "direct");
        fx.agent(
            "a",
            &[
                user_text(NOW - 30 * MIN, &envelope("team-lead", "Build the card.")),
                call(NOW - 20 * MIN, "r1", "Read", json!({"file_path":"/w/src/team.rs"})),
                result(NOW - 20 * MIN + 100, "r1"),
                call(NOW - 15 * MIN, "c1", "Bash", json!({"command":"git add -A && git commit -m 'feat: card'"})),
                result_of(NOW - 15 * MIN + 900, "c1", "[directo 1a2b3c4] feat: card", false),
                call(NOW - 10 * MIN, "t1", "Bash", json!({"command":"cargo test --workspace 2>&1 | tee /tmp/t.log"})),
                result_of(
                    NOW - 8 * MIN,
                    "t1",
                    "OUTPUT-SENTINEL\ntest result: ok. 809 passed; 0 failed; 3 ignored\ntest result: ok. 12 passed; 0 failed; 0 ignored\n",
                    false,
                ),
                call(NOW - 5 * MIN, "e1", "Edit", json!({"file_path":"/w/src/card.tsx","old_string":"INPUT-SENTINEL"})),
                result(NOW - 5 * MIN + 50, "e1"),
                call(NOW - 4 * MIN, "w1", "WebSearch", json!({"query":"QUERY-SENTINEL"})),
                result(NOW - 4 * MIN + 50, "w1"),
                call(NOW - MIN, "b1", "Bash", json!({"command":"bun run build"})),
            ],
        );
        // A call its turn ended on, with no result, is a stopped step.
        fx.teammate("s", "sender");
        fx.agent(
            "s",
            &[
                user_text(NOW - 30 * MIN, &envelope("team-lead", "Report.")),
                send(NOW - 9 * MIN, "team-lead", "DONE · sender · Hecho."),
                reply(NOW - 9 * MIN + 10, "Sent."),
            ],
        );
        let view = fx.view();
        let stopped = &agent(&view, "s").progress;
        assert_eq!(stopped.tool_calls, 1);
        assert_eq!(
            stopped
                .steps
                .iter()
                .map(|s| (s.tool, s.status))
                .collect::<Vec<_>>(),
            vec![(Some("SendMessage"), "stopped")]
        );
        let p = &agent(&view, "a").progress;
        assert_eq!((p.tool_calls, p.commits), (6, 1));
        let run = p.test.unwrap();
        assert_eq!(
            (
                run.counts.passed,
                run.counts.failed,
                run.counts.skipped,
                run.ok
            ),
            (821, 0, 3, true)
        );
        let steps: Vec<(&str, Option<&str>, &str)> = p
            .steps
            .iter()
            .map(|s| (s.state, s.detail.as_deref(), s.status))
            .collect();
        assert_eq!(
            steps,
            vec![
                ("running", Some("bun run build"), "running"),
                ("web", None, "ok"),
                ("editing", Some("card.tsx"), "ok"),
                ("testing", Some("cargo test"), "ok"),
                ("running", Some("git add"), "ok"),
            ]
        );
        let wire = serde_json::to_string(&view).unwrap();
        for leak in [
            "OUTPUT-SENTINEL",
            "INPUT-SENTINEL",
            "QUERY-SENTINEL",
            "/tmp/t.log",
            "feat: card",
        ] {
            assert!(!wire.contains(leak), "{leak} reached the team JSON");
        }
    }
}
