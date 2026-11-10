//! `GET /api/v1/retro`: the week's (or a finished team's) numbers, the rules
//! the retro suggests and the rules in force (`harbard_core::retro`).

use super::Params;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct FindingClass {
    pub class: String,
    pub count: u64,
    pub branches: Vec<String>,
    pub repeats: bool,
    pub days: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DayPoint {
    pub day: String,
    pub findings: u64,
    pub not_understood: u64,
    pub stalls: u64,
    pub unanswered: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct RetroDelivery {
    pub brief_to_install_p50_min: Option<u64>,
    pub review_rounds: BTreeMap<String, u64>,
    pub findings: Vec<FindingClass>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct RetroMessages {
    pub not_understood: u64,
    pub jargon_chips: u64,
    pub what_does_it_mean: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Unanswered {
    pub note: String,
    pub at_ms: u64,
    pub open_min: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct RetroWaiting {
    pub unanswered: Vec<Unanswered>,
    pub answered: u64,
    pub answer_p50_min: Option<u64>,
    pub idle_min: u64,
    pub stragglers: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Stall {
    pub agent: String,
    pub tool: String,
    pub pattern: String,
    pub at_ms: u64,
    pub minutes: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct RetroHarness {
    pub stalls: Vec<Stall>,
    pub stall_min: u64,
    pub two_writers: u64,
    pub repeats: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct RetroNames {
    pub names_down: u64,
    pub renamed: Option<u64>,
    pub generated: Vec<String>,
    pub english_descriptions: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Retro {
    pub now_ms: u64,
    pub since_ms: u64,
    /// week, team
    pub scope: String,
    pub lead: Option<String>,
    pub agents: u64,
    pub delivery: RetroDelivery,
    pub messages: RetroMessages,
    pub waiting: RetroWaiting,
    pub harness: RetroHarness,
    pub names: RetroNames,
    /// kind → value → count
    pub feedback: BTreeMap<String, BTreeMap<String, u64>>,
    pub daily: Vec<DayPoint>,
}

/// A rule the retro suggests.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Lesson {
    pub key: String,
    pub title: String,
    pub evidence: String,
    pub evidence_code: Option<String>,
    pub evidence_params: Option<Params>,
    pub rule: String,
    pub count: u64,
    pub dates: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ActiveRule {
    pub id: String,
    pub key: String,
    pub title: String,
    pub text: String,
    pub approved_ms: u64,
    pub edited: bool,
    pub memory_sink: bool,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct MemorySinkState {
    pub configured: bool,
    pub allowed: Option<bool>,
}

/// "Was it you?": a change to what every session reads.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DeskCheck {
    pub id: String,
    /// rule, context, memory_sink
    pub what: String,
    pub text: Option<String>,
    pub since_ms: u64,
    /// kept, revoked; absent while unanswered
    pub outcome: Option<String>,
    pub answered_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct RetroBody {
    pub retro: Retro,
    pub candidates: Vec<Lesson>,
    pub rules: Vec<ActiveRule>,
    pub finished_teams: Vec<String>,
    pub memory_sink: Option<MemorySinkState>,
    pub computed_ms: Option<u64>,
    pub checks: Vec<DeskCheck>,
}

/// What became of saving a rule to the external memory.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct MemorySinkOutcome {
    pub saved: bool,
    pub code: Option<String>,
    pub note: Option<String>,
    pub params: Option<BTreeMap<String, String>>,
}
