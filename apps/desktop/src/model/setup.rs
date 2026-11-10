//! First-run Setup: `GET /api/v1/setup`, `POST /setup/install`,
//! `POST /setup/complete`.

use serde::Deserialize;

/// One harness: its hook health plus whether the tool is on this Mac.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct SetupHarness {
    /// claude, codex, cursor
    pub target: String,
    /// claude-code, codex, cursor
    pub harness: String,
    pub path: String,
    pub config_exists: bool,
    /// Already wired.
    pub ok: bool,
    pub missing_events: Vec<String>,
    pub retired_lingering: Vec<String>,
    pub hint: Option<String>,
    /// The tool is installed.
    pub present: bool,
    pub recommended: bool,
    pub blurb: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct SetupStatus {
    pub needs_setup: bool,
    pub completed_ms: Option<u64>,
    pub skipped: Vec<String>,
    pub wired: Vec<String>,
    pub harnesses: Vec<SetupHarness>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct SetupInstallResult {
    pub target: String,
    pub ok: bool,
    pub path: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct SetupInstallResponse {
    pub ok: bool,
    pub results: Vec<SetupInstallResult>,
    pub setup: SetupStatus,
}
