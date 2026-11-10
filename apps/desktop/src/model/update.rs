//! `GET /api/v1/update` (docs/SPEC-UPDATES.md): the installed version, the
//! newer one, and a run that installs it.

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct UpdateError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct UpdateTarget {
    pub version: String,
    pub commit: String,
    pub tag: Option<String>,
    pub title: Option<String>,
    pub date_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct UpdateChange {
    pub commit: String,
    /// feat, fix, perf, refactor, docs, test, build, chore, other.
    pub kind: String,
    pub scope: Option<String>,
    pub subject: String,
    pub date_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct UpdateCheck {
    pub checked_ms: u64,
    /// main or releases.
    pub channel: String,
    /// current, available or error.
    pub status: String,
    pub error: Option<UpdateError>,
    pub target: Option<UpdateTarget>,
    pub behind: u64,
    pub notes: Option<String>,
    pub changes: Vec<UpdateChange>,
    pub changes_truncated: bool,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct UpdateStep {
    /// fetch, cli, hooks, serve, console, desktop.
    pub id: String,
    /// pending, running, done, failed, skipped.
    pub state: String,
    pub started_ms: Option<u64>,
    pub finished_ms: Option<u64>,
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct UpdateRun {
    pub id: String,
    /// running, done or failed.
    pub state: String,
    pub started_ms: u64,
    pub finished_ms: Option<u64>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub error: Option<UpdateError>,
    pub steps: Vec<UpdateStep>,
    pub log_tail: Vec<String>,
    pub notes: Option<String>,
    pub changes: Option<Vec<UpdateChange>>,
    pub changes_truncated: Option<bool>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Installed {
    pub version: String,
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct UpdatePrefs {
    pub channel: String,
    pub auto_check: bool,
}

impl Default for UpdatePrefs {
    fn default() -> Self {
        UpdatePrefs {
            channel: "main".into(),
            auto_check: true,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct UpdateSource {
    pub path: Option<String>,
    /// no_checkout, dirty, not_on_main, diverged.
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct UpdateView {
    pub installed: Installed,
    pub prefs: UpdatePrefs,
    pub source: UpdateSource,
    pub check: Option<UpdateCheck>,
    pub run: Option<UpdateRun>,
}

/// `GET /api/v1/meta`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Meta {
    pub name: String,
    pub version: String,
    pub host: String,
    pub started_ms: u64,
    pub protocol: Option<String>,
    pub commit: Option<String>,
}
