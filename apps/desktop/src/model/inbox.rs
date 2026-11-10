//! Reads the inbox screens fetch on their own: one agent's live log
//! (`GET /api/v1/live/<key>`, `harbard_core::live`) and an archived
//! conversation (`GET /api/v1/history/agents/<key>`, `harbard_core::history`).

use super::{Agent, Message, Progress, StepRow};
use serde::Deserialize;

/// One tool call of the timeline: `n` is its place, `rev` when it last changed.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct LiveStep {
    pub n: u64,
    pub rev: u64,
    #[serde(flatten)]
    pub step: StepRow,
}

/// One message of the agent's own prose, masked by serve.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Prose {
    pub n: u64,
    pub rev: u64,
    pub ts_ms: u64,
    pub text: String,
}

/// What changed since the cursor `after`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct LiveDelta {
    pub v: u64,
    pub key: String,
    pub now_ms: u64,
    pub rev: u64,
    pub reset: bool,
    pub first_ms: Option<u64>,
    pub last_ms: Option<u64>,
    pub steps: Vec<LiveStep>,
    pub prose: Vec<Prose>,
    pub progress: Progress,
}

/// One item of History: an agent of the last month and how it ended.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct HistoryItem {
    pub agent: Agent,
    pub channel: Option<String>,
    pub repo: Option<String>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub duration_ms: u64,
    pub last_line: Option<String>,
    /// finished, closed, verified, no_closure; `None` while it works.
    pub ending: Option<String>,
    pub read_only: bool,
}

/// One archived conversation.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct HistoryConversation {
    pub item: HistoryItem,
    pub messages: Vec<Message>,
    pub truncated: bool,
    pub operator: String,
    pub operator_initials: String,
    pub now_ms: u64,
}
