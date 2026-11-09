//! Operator profile (desk identity).
//!
//! The bus author is always `operator@{host}` / `harness:human` / `role:operator`.
//! This file is the human-readable name agents and the console should use —
//! not a second authorship channel.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::store::Store;

const MAX_DISPLAY_NAME_CHARS: usize = 48;

/// Persisted under `{home}/operator.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperatorProfile {
    pub display_name: String,
    #[serde(default)]
    pub updated_ms: u64,
    /// Where the operator has read up to, per place id (SPEC-FLEET-VIEW §4).
    ///
    /// The operator's own state, never an agent's: `cursors/` belongs to
    /// agents and this file must never touch it. One desk, one map — two
    /// browser windows share these, last write wins.
    #[serde(default)]
    pub reads: std::collections::BTreeMap<String, String>,
}

impl Default for OperatorProfile {
    fn default() -> Self {
        Self {
            display_name: crate::workspace::DEFAULT_OPERATOR_NAME.into(),
            updated_ms: 0,
            reads: std::collections::BTreeMap::new(),
        }
    }
}

impl OperatorProfile {
    pub fn path(store: &Store) -> PathBuf {
        store.root().join("operator.json")
    }

    pub fn load(store: &Store) -> Self {
        Self::load_path(&Self::path(store))
    }

    pub fn load_path(path: &Path) -> Self {
        match fs::read_to_string(path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, store: &Store) -> std::io::Result<()> {
        let path = Self::path(store);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, bytes)?;
        fs::rename(&tmp, path)?;
        Ok(())
    }
}

/// Sanitize a display name for storage. Empty → error.
pub fn sanitize_display_name(raw: &str) -> Result<String, &'static str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("display_name is empty");
    }
    let mut out = String::new();
    for ch in trimmed.chars() {
        if ch.is_control() {
            continue;
        }
        out.push(ch);
        if out.chars().count() >= MAX_DISPLAY_NAME_CHARS {
            break;
        }
    }
    let out = out.trim().to_string();
    if out.is_empty() {
        return Err("display_name is empty");
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Store;
    use crate::test_util::TempHome;

    #[test]
    fn round_trip_and_sanitize() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let name = sanitize_display_name("  Dana  ").unwrap();
        assert_eq!(name, "Dana");
        let profile = OperatorProfile {
            display_name: name,
            updated_ms: 42,
            ..OperatorProfile::default()
        };
        profile.save(&store).unwrap();
        let loaded = OperatorProfile::load(&store);
        assert_eq!(loaded.display_name, "Dana");
        assert!(sanitize_display_name("").is_err());
    }
}
