//! `GET /api/v1/workspaces`: the workspaces, their rail tiles and where each
//! repository sits (`serve/workspaces_route.rs`).

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct WorkspaceEntry {
    pub id: String,
    pub name: String,
    pub tile: String,
    pub icon: String,
    pub owners: Vec<String>,
    pub default: bool,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct PlacedRepo {
    /// The repository's path.
    pub repo: String,
    pub workspace: String,
    pub moved: bool,
    pub reason: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct WorkspacesList {
    pub single: bool,
    pub default: String,
    pub workspaces: Vec<WorkspaceEntry>,
    pub repos: Vec<PlacedRepo>,
}

/// `GET /api/v1/feedback`: only what Preferences reads from it.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct MemorySink {
    pub configured: bool,
    pub allowed: bool,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DeskCheck {
    pub id: String,
    pub what: String,
    pub text: Option<String>,
    pub since_ms: u64,
    pub outcome: Option<String>,
    pub answered_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct FeedbackPrefs {
    pub memory_sink: Option<MemorySink>,
    pub checks: Vec<DeskCheck>,
}
