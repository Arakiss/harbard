//! `GET /api/v1/theme` and the `theme-*` writes (`harbard_core::theme`).
//! The app paints from the raw answer (`crate::theme`); Preferences reads
//! these typed views of the same answer.

use crate::model::Params;
use serde::Deserialize;
use std::collections::BTreeMap;

pub type Tokens = BTreeMap<String, String>;

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ThemeEntry {
    pub id: String,
    pub label: String,
    pub builtin: bool,
    pub file: Option<String>,
    pub light: Tokens,
    pub dark: Tokens,
    /// `harbard-theme:v1:<id>:<hex>,...`
    pub share: String,
    /// The file is refused now; these are its last accepted tokens.
    pub stale: bool,
    /// A seasonal theme: painted by its season, never offered in the picker.
    pub season: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct SeasonState {
    pub id: String,
    pub theme: String,
    /// auto, on or off.
    pub choice: String,
    pub active: bool,
    pub in_window: bool,
    pub decorations: bool,
    /// `MM-DD`, both included.
    pub from: String,
    pub to: String,
    pub today: String,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ThemeRefusal {
    pub file: String,
    pub line: Option<u64>,
    pub key: Option<String>,
    pub code: String,
    pub message: String,
    pub fix: String,
    pub message_code: Option<String>,
    pub fix_code: Option<String>,
    pub params: Option<Params>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ThemeState {
    pub version: u64,
    pub revision: String,
    pub theme: String,
    /// light, dark or system.
    pub mode: String,
    pub themes: Vec<ThemeEntry>,
    pub errors: Vec<ThemeRefusal>,
    pub notice: Option<String>,
    pub notice_code: Option<String>,
    pub notice_params: Option<Params>,
    pub dir: String,
    pub dock_attention: Option<bool>,
    pub season: Option<SeasonState>,
}

/// `theme-preview`'s answer.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ThemePreview {
    pub ok: bool,
    pub theme: Option<ThemeEntry>,
    pub refusal: Option<ThemeRefusal>,
}
