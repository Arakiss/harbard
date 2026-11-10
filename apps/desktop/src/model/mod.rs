//! The workspace read, as serve's `/api/v1/workspace` shapes it.
//!
//! The app computes the same JSON in-process (see [`crate::hub`]) and reads it
//! into these types. Every field is defaulted so an older or newer serve never
//! fails the read: a missing field is empty, an unknown one is ignored.
//! Enumerations stay strings, matching the console, which compared literals.

use serde::Deserialize;
use std::collections::BTreeMap;

pub mod profile;

pub type Params = BTreeMap<String, serde_json::Value>;

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Thresholds {
    pub amber_ms: u64,
    pub red_ms: u64,
    pub thinking_long_ms: u64,
    pub waiting_sleepy_ms: u64,
    pub running_long_ms: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Activity {
    /// normal, thinking, running, testing, editing, reading, web, delegating,
    /// waiting, asleep, blocked, finished, verified, disconnected
    pub state: String,
    pub since_ms: u64,
    pub detail: Option<String>,
    pub source_ms: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Reaction {
    /// glance, spark, hop, shake, boot
    pub kind: String,
    pub at_ms: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TestCounts {
    pub passed: u64,
    pub failed: u64,
    pub skipped: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct TestRun {
    pub passed: u64,
    pub failed: u64,
    pub skipped: u64,
    pub ok: bool,
    pub at_ms: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct StepRow {
    pub state: String,
    pub tool: Option<String>,
    pub detail: Option<String>,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
    /// running, ok, error, stopped
    pub status: String,
    pub test: Option<TestCounts>,
    /// The step made a commit.
    pub commit: bool,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Progress {
    pub tool_calls: u64,
    pub commits: u64,
    pub test: Option<TestRun>,
    pub steps: Vec<StepRow>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Checkout {
    pub path: String,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct WriterWarning {
    pub checkout: String,
    pub agents: Vec<String>,
    pub since_ms: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ChannelReader {
    pub session: String,
    /// ready, stale, ended, not_on_bus
    pub delivery: String,
    pub lead: bool,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Channel {
    pub id: String,
    pub repo: String,
    pub checkouts: Vec<Checkout>,
    pub members: Vec<String>,
    pub last_ms: u64,
    pub warnings: Vec<WriterWarning>,
    pub readers: Vec<ChannelReader>,
    /// ready, stale, none, nobody
    pub reach: Option<String>,
    pub active_ms: Option<u64>,
    pub quiet: bool,
    pub unread: u64,
    pub recent: Vec<String>,
    /// project, room
    pub kind: Option<String>,
    pub purpose: Option<String>,
    pub archived: bool,
    pub built_in: bool,
}

impl Channel {
    pub fn is_room(&self) -> bool {
        self.kind.as_deref() == Some("room")
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    /// image, markdown, diff, text, json, huginn
    pub kind: String,
    /// before, after, report, code
    pub label: Option<String>,
    pub bytes: u64,
    pub path: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct NoteDelivery {
    /// sent, delivered, read, answered
    pub state: String,
    pub sessions: Vec<String>,
    pub delivered: u64,
    pub to_lead: bool,
    pub nudged_ms: Option<u64>,
    pub answered_by: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Agent {
    pub workspace: Option<String>,
    pub key: String,
    pub name: String,
    pub label: String,
    pub lost: bool,
    /// teammate, subagent, session
    pub kind: String,
    pub lead_session: String,
    pub lead_quiet_ms: Option<u64>,
    pub delivery: String,
    pub state: String,
    pub state_reason: String,
    /// working, waiting, finished
    pub presence: String,
    /// amber, red
    pub dormant: Option<String>,
    pub last_action_ms: Option<u64>,
    pub first_ms: Option<u64>,
    pub brief: Option<String>,
    pub description: Option<String>,
    pub checkout: Option<String>,
    pub branch: Option<String>,
    pub build_dir: Option<String>,
    pub channels: Vec<String>,
    pub deliveries: u64,
    pub verified_deliveries: u64,
    pub reports: Vec<String>,
    pub unread: u64,
    pub last_message: Option<String>,
    pub can_stop: bool,
    pub activity: Activity,
    pub reactions: Vec<Reaction>,
    pub progress: Option<Progress>,
    pub team: Option<String>,
    pub role: Option<String>,
    pub handle: Option<String>,
    pub temporary: bool,
    pub name_code: Option<String>,
    pub name_params: Option<Params>,
    pub role_line: Option<String>,
    pub role_line_code: Option<String>,
    pub role_line_params: Option<Params>,
    /// auto, picked, renamed, launched
    pub name_source: Option<String>,
    pub name_reason: Option<String>,
    pub own_name: Option<String>,
    pub closed: bool,
    pub hidden: bool,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ReadablePart {
    /// text, sha, branch, path
    pub kind: String,
    pub text: String,
    pub full: Option<String>,
    pub code: Option<String>,
    pub params: Option<Params>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Verified {
    pub sha: String,
    pub branch: String,
    pub decided_ms: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Message {
    pub id: String,
    pub agent: String,
    /// agent, lead, peer, operator, session
    pub from: String,
    pub author: String,
    pub ts_ms: u64,
    pub text: String,
    pub body: Option<String>,
    /// START, BLOCKED, FINDING, DONE
    pub label: Option<String>,
    pub version: Option<u64>,
    pub flags: Vec<String>,
    pub jargon: Vec<String>,
    pub readable: Option<Vec<ReadablePart>>,
    /// delivery, review, blocked
    pub shape: Option<String>,
    pub repeat_of: Option<String>,
    pub chars: u64,
    pub files: Vec<String>,
    pub verified: Option<Verified>,
    pub channels: Vec<String>,
    pub reply_to: Option<String>,
    pub replies: Vec<String>,
    pub structured: Option<String>,
    pub undelivered: bool,
    pub attachments: Vec<Attachment>,
    pub delivery: Option<NoteDelivery>,
    pub mentions: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct CardCheck {
    /// rule, context, memory_sink
    pub what: String,
    pub text: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Card {
    pub workspace: Option<String>,
    pub id: String,
    /// blocked, approval, dormant, two_writers, unanswered, mention, handoff, check
    pub kind: String,
    pub channel: Option<String>,
    pub agents: Vec<String>,
    pub message: Option<String>,
    pub checkout: Option<String>,
    pub since_ms: u64,
    pub resolved_ms: Option<u64>,
    pub needs: bool,
    pub check: Option<CardCheck>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DeliveryRow {
    pub id: String,
    pub channel: Option<String>,
    pub sha: String,
    pub branch: String,
    /// landed, conflict, rejected, failed, pending
    pub verdict: String,
    pub created_ms: u64,
    pub decided_ms: Option<u64>,
    pub session: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Saved {
    pub id: String,
    /// message, report
    pub kind: String,
    pub saved_ms: u64,
    pub agent: Option<String>,
    pub channel: Option<String>,
    pub text: String,
    pub ts_ms: Option<u64>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Unread {
    pub directs: u64,
    pub activity: u64,
    pub channels: u64,
    pub todo: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ReviewStep {
    pub agent: String,
    pub message: String,
    pub ts_ms: u64,
    /// reviewing, fixes, reviewed
    pub kind: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Shipment {
    pub id: String,
    pub agent: String,
    pub ts_ms: u64,
    pub text: String,
    pub channel: Option<String>,
    pub sha: Option<String>,
    pub commit: Option<String>,
    pub landed_as: Option<String>,
    /// delivered, in_review, with_fixes, integrated, installed
    pub state: String,
    pub last_state: String,
    pub pending: bool,
    /// no_commit, no_repository, unknown_commit
    pub untracked: Option<String>,
    pub review: Vec<ReviewStep>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct WorkspaceTile {
    pub id: String,
    pub name: String,
    pub tile: String,
    pub default: bool,
    pub current: bool,
    pub unread: u64,
    pub needs: u64,
    pub working: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct LobbyEntry {
    pub key: String,
    pub name: String,
    /// claude-code, codex, cursor, other
    pub harness: String,
    pub project: Option<String>,
    pub intro: Option<String>,
    pub intro_ts_ms: Option<u64>,
    /// live, idle, stale, ended
    pub presence: String,
    pub first_ms: u64,
    pub last_ms: u64,
}

/// `GET /api/v1/workspace`: the whole shell.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct WorkspaceView {
    pub v: u64,
    pub now_ms: u64,
    pub operator: String,
    pub operator_initials: String,
    pub thresholds: Thresholds,
    pub channels: Vec<Channel>,
    pub agents: Vec<Agent>,
    pub messages: Vec<Message>,
    pub cards: Vec<Card>,
    pub deliveries: Vec<DeliveryRow>,
    pub saved: Vec<Saved>,
    pub unread: Unread,
    pub team_root_found: bool,
    pub agents_collapsed: bool,
    pub shipments: Vec<Shipment>,
    pub running_commit: Option<String>,
    pub format_changed: bool,
    pub motion: Option<String>,
    pub night_dim: bool,
    pub rev: Option<String>,
    pub workspace: Option<String>,
    pub workspaces: Vec<WorkspaceTile>,
    pub lobby: Vec<LobbyEntry>,
}

impl WorkspaceView {
    pub fn agent(&self, key: &str) -> Option<&Agent> {
        self.agents.iter().find(|a| a.key == key)
    }

    pub fn channel(&self, id: &str) -> Option<&Channel> {
        self.channels.iter().find(|c| c.id == id)
    }

    pub fn message(&self, id: &str) -> Option<&Message> {
        self.messages.iter().find(|m| m.id == id)
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct AgentActivity {
    pub key: String,
    pub presence: String,
    pub dormant: Option<String>,
    pub last_action_ms: Option<u64>,
    pub activity: Activity,
    pub reactions: Vec<Reaction>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ActivityUnread {
    pub directs: u64,
    pub activity: u64,
}

/// `GET /api/v1/workspace?only=activity`: faces, reactions and badges.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ActivityView {
    pub v: u64,
    pub now_ms: u64,
    pub thresholds: Thresholds,
    pub agents: Vec<AgentActivity>,
    pub unread: ActivityUnread,
    pub needs: u64,
    pub directs: u64,
    pub blocked: u64,
    pub motion: Option<String>,
    pub rev: Option<String>,
    pub workspace: Option<String>,
}
