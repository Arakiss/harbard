//! Closed-loop correction resolution (M5d).
//!
//! Delivery of a correction is not acceptance. Each actionable correction can
//! move through explicit stages until it is terminal.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Stages a correction receipt may occupy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReceiptStage {
    /// Injected into a harness envelope / brief.
    Delivered,
    /// Session showed evidence of seeing it (e.g. next tool after nudge).
    Observed,
    /// Remediation applied (intent written, ack sent, worktree created, …).
    Applied,
    /// Explicit rejection by the agent/operator.
    Rejected,
    /// Timed out without applied/rejected.
    Expired,
}

impl ReceiptStage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::Observed => "observed",
            Self::Applied => "applied",
            Self::Rejected => "rejected",
            Self::Expired => "expired",
        }
    }

    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Applied | Self::Rejected | Self::Expired)
    }
}

/// One resolution row for an actionable correction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resolution {
    pub id: String,
    pub code: String,
    pub session: String,
    pub stage: ReceiptStage,
    pub created_ms: u64,
    pub updated_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<String>,
}

/// Append-only resolution log under `{home}/resolutions/v1/`.
pub struct ResolutionStore {
    root: PathBuf,
}

impl ResolutionStore {
    pub fn at(home: &Path) -> Self {
        Self {
            root: home.join("resolutions").join("v1"),
        }
    }

    fn log_path(&self) -> PathBuf {
        self.root.join("resolutions.jsonl")
    }

    pub fn append(&self, row: &Resolution) -> std::io::Result<()> {
        fs::create_dir_all(&self.root)?;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.log_path())?;
        let mut line = serde_json::to_string(row).unwrap_or_else(|_| "{}".into());
        line.push('\n');
        f.write_all(line.as_bytes())?;
        Ok(())
    }

    /// Open one correction occurrence unless the same session/code already
    /// has a non-terminal occurrence. Each recurrence gets a distinct id, so
    /// a later violation cannot overwrite or reopen a terminal history row.
    pub fn deliver(
        &self,
        session: &str,
        code: &str,
        now_ms: u64,
        evidence: Option<&str>,
    ) -> std::io::Result<Option<String>> {
        if self
            .open_for_session(session)
            .iter()
            .any(|row| row.code == code)
        {
            return Ok(None);
        }
        let id = format!("{session}:{code}:{}", ulid::Ulid::generate());
        self.append(&Resolution {
            id: id.clone(),
            code: code.to_string(),
            session: session.to_string(),
            stage: ReceiptStage::Delivered,
            created_ms: now_ms,
            updated_ms: now_ms,
            evidence: evidence.map(str::to_string),
        })?;
        Ok(Some(id))
    }

    /// Advance the newest open occurrence for one session/code. Terminal
    /// occurrences stay terminal; a new recurrence must start with `deliver`.
    pub fn advance_latest(
        &self,
        session: &str,
        code: &str,
        stage: ReceiptStage,
        now_ms: u64,
        evidence: Option<&str>,
    ) -> std::io::Result<bool> {
        let Some(row) = self
            .open_for_session(session)
            .into_iter()
            .filter(|row| row.code == code)
            .max_by_key(|row| (row.updated_ms, row.created_ms))
        else {
            return Ok(false);
        };
        if stage == ReceiptStage::Delivered {
            return Ok(false);
        }
        if stage == ReceiptStage::Observed {
            self.append(&Resolution {
                id: row.id,
                code: row.code,
                session: row.session,
                stage,
                created_ms: row.created_ms,
                updated_ms: now_ms,
                evidence: evidence.map(str::to_string),
            })?;
            return Ok(true);
        }
        self.close_open(&row.id, stage, now_ms, evidence)
    }

    /// Latest stage per id (scan file order; last wins).
    pub fn latest(&self) -> Vec<Resolution> {
        let Ok(bytes) = fs::read(self.log_path()) else {
            return Vec::new();
        };
        let mut map = std::collections::BTreeMap::<String, Resolution>::new();
        for line in bytes.split(|b| *b == b'\n') {
            if line.is_empty() {
                continue;
            }
            if let Ok(row) = serde_json::from_slice::<Resolution>(line) {
                map.insert(row.id.clone(), row);
            }
        }
        map.into_values().collect()
    }

    /// Open (non-terminal) resolutions for a session.
    pub fn open_for_session(&self, session: &str) -> Vec<Resolution> {
        self.latest()
            .into_iter()
            .filter(|r| r.session == session && !r.stage.is_terminal())
            .collect()
    }

    /// All open (non-terminal) resolutions.
    pub fn open(&self) -> Vec<Resolution> {
        self.latest()
            .into_iter()
            .filter(|r| !r.stage.is_terminal())
            .collect()
    }

    /// If `id` is open, append a terminal stage. Returns true when a row was written.
    pub fn close_open(
        &self,
        id: &str,
        stage: ReceiptStage,
        now_ms: u64,
        evidence: Option<&str>,
    ) -> std::io::Result<bool> {
        if !stage.is_terminal() {
            return Ok(false);
        }
        let Some(row) = self.open().into_iter().find(|r| r.id == id) else {
            return Ok(false);
        };
        self.append(&Resolution {
            id: row.id,
            code: row.code,
            session: row.session,
            stage,
            created_ms: row.created_ms,
            updated_ms: now_ms,
            evidence: evidence.map(str::to_string),
        })?;
        Ok(true)
    }

    /// Append `Expired` for open rows whose `updated_ms` is older than `ttl_ms`.
    /// Returns how many rows were expired.
    pub fn expire_stale(&self, now_ms: u64, ttl_ms: u64) -> std::io::Result<u64> {
        let mut n = 0u64;
        for row in self.open() {
            if now_ms.saturating_sub(row.updated_ms) < ttl_ms {
                continue;
            }
            self.append(&Resolution {
                id: row.id,
                code: row.code,
                session: row.session,
                stage: ReceiptStage::Expired,
                created_ms: row.created_ms,
                updated_ms: now_ms,
                evidence: Some(format!("ttl_ms={ttl_ms}")),
            })?;
            n += 1;
        }
        Ok(n)
    }
}

/// Default TTL for open correction resolutions (48h).
pub const RESOLUTION_TTL_MS: u64 = 48 * 60 * 60 * 1000;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::TempHome;

    #[test]
    fn expire_stale_appends_terminal_rows() {
        let home = TempHome::new();
        let store = ResolutionStore::at(home.path());
        store
            .append(&Resolution {
                id: "r-old".into(),
                code: "intent_stale".into(),
                session: "s1".into(),
                stage: ReceiptStage::Delivered,
                created_ms: 1,
                updated_ms: 1,
                evidence: None,
            })
            .unwrap();
        store
            .append(&Resolution {
                id: "r-fresh".into(),
                code: "ack_pending".into(),
                session: "s1".into(),
                stage: ReceiptStage::Delivered,
                created_ms: 980,
                updated_ms: 980,
                evidence: None,
            })
            .unwrap();
        let n = store.expire_stale(1_000, 50).unwrap();
        assert_eq!(n, 1);
        let open = store.open();
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].id, "r-fresh");
        let old = store
            .latest()
            .into_iter()
            .find(|r| r.id == "r-old")
            .unwrap();
        assert_eq!(old.stage, ReceiptStage::Expired);
    }

    #[test]
    fn stages_round_trip_and_open_filter() {
        let home = TempHome::new();
        let store = ResolutionStore::at(home.path());
        store
            .append(&Resolution {
                id: "r1".into(),
                code: "intent_stale".into(),
                session: "s1".into(),
                stage: ReceiptStage::Delivered,
                created_ms: 1,
                updated_ms: 1,
                evidence: None,
            })
            .unwrap();
        store
            .append(&Resolution {
                id: "r1".into(),
                code: "intent_stale".into(),
                session: "s1".into(),
                stage: ReceiptStage::Applied,
                created_ms: 1,
                updated_ms: 2,
                evidence: Some("intent written".into()),
            })
            .unwrap();
        assert!(store.open_for_session("s1").is_empty());
        let latest = store.latest();
        assert_eq!(latest.len(), 1);
        assert_eq!(latest[0].stage, ReceiptStage::Applied);
    }

    #[test]
    fn terminal_occurrence_never_reopens_and_recurrence_gets_a_new_id() {
        let home = TempHome::new();
        let store = ResolutionStore::at(home.path());
        let first = store
            .deliver("s1", "intent_stale", 1, Some("brief"))
            .unwrap()
            .unwrap();
        assert!(
            store
                .advance_latest(
                    "s1",
                    "intent_stale",
                    ReceiptStage::Applied,
                    2,
                    Some("intent")
                )
                .unwrap()
        );
        let second = store
            .deliver("s1", "intent_stale", 3, Some("brief"))
            .unwrap()
            .unwrap();
        assert_ne!(first, second);
        let latest = store.latest();
        assert_eq!(latest.len(), 2);
        assert_eq!(
            latest.iter().find(|row| row.id == first).unwrap().stage,
            ReceiptStage::Applied
        );
        assert_eq!(
            latest.iter().find(|row| row.id == second).unwrap().stage,
            ReceiptStage::Delivered
        );
    }
}
