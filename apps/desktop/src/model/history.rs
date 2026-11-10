//! History's reads: `GET /api/v1/history/agents` (the month of sessions and
//! agents), `GET /api/v1/channels/deleted` and a room's
//! `GET /api/v1/channels/<room>/history`.

use super::Agent;
use serde::Deserialize;

/// One session or agent of the month, as serve closed it.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct HistoryItem {
    pub agent: Agent,
    /// The first channel it wrote in, and that channel's repository.
    pub channel: Option<String>,
    pub repo: Option<String>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub duration_ms: u64,
    /// Its own last human line, masked by serve.
    pub last_line: Option<String>,
    /// finished, closed, verified, no_closure; `None` while it still works.
    pub ending: Option<String>,
    pub read_only: bool,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct HistoryChannel {
    pub id: String,
    pub repo: String,
    pub items: u64,
}

/// One page, newest first.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct HistoryPage {
    pub items: Vec<HistoryItem>,
    /// The cursor of the next page; `None` on the last one.
    pub next: Option<String>,
    /// Items matching the filter and the search, over every page.
    pub total: u64,
    pub channels: Vec<HistoryChannel>,
    pub now_ms: u64,
    pub window_days: u64,
}

/// One line of `GET /api/v1/channels/deleted`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DeletedRoom {
    pub name: String,
    pub deleted_ms: u64,
    pub messages: u64,
}

