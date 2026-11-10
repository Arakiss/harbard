//! `GET /api/v1/profile`: who the operator is for the agents, their status
//! line, coordination state and "Do not disturb" (`harbard_core::profile`).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct ProfileStatus {
    pub emoji: String,
    pub text: String,
    pub clear_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct ProfileHours {
    /// `HH:MM`.
    pub start: String,
    pub end: String,
    /// ISO weekdays, Monday = 1.
    pub days: Vec<u8>,
}

impl Default for ProfileHours {
    fn default() -> Self {
        ProfileHours {
            start: "09:00".into(),
            end: "18:00".into(),
            days: vec![1, 2, 3, 4, 5],
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Since {
    pub since_ms: u64,
    pub until_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct StateMark {
    pub id: String,
    pub since_ms: u64,
    pub until_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Profile {
    pub name: Option<String>,
    pub address: Option<String>,
    pub time_zone: Option<String>,
    pub hours: Option<ProfileHours>,
    pub context: Option<String>,
    pub status: Option<ProfileStatus>,
    pub dnd: Option<Since>,
    pub state: Option<StateMark>,
    /// `30m`, `1h` or `tomorrow`.
    pub dnd_default: Option<String>,
    pub digest_hourly: Option<bool>,
    pub away_suggest: Option<bool>,
    pub utc_offset_min: Option<i64>,
    pub photo: Option<String>,
    pub photo_rev: u64,
    pub updated_ms: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DockRules {
    pub bounce: bool,
    pub alerts: bool,
    pub one_shots: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProfileLimits {
    pub context: u64,
    pub name: u64,
    pub address: u64,
    pub status_text: u64,
    pub photo_bytes: u64,
}

impl Default for ProfileLimits {
    fn default() -> Self {
        ProfileLimits {
            context: 1500,
            name: 60,
            address: 120,
            status_text: 100,
            photo_bytes: 2 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DigestItem {
    pub id: String,
    pub session: String,
    pub harness: String,
    pub kind: String,
    pub text: String,
    pub ts_ms: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Digest {
    pub id: String,
    pub state: String,
    /// `left`, `reviewing` or `hourly`.
    pub reason: String,
    pub from_ms: u64,
    pub to_ms: u64,
    pub items: Vec<DigestItem>,
    pub read: bool,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProfileView {
    pub profile: Profile,
    /// In force now: an expired status is already `None`.
    pub status: Option<ProfileStatus>,
    pub quiet: bool,
    pub quiet_until_ms: Option<u64>,
    /// available, focused, swamped, away, reviewing, done.
    pub state: String,
    pub state_until_ms: Option<u64>,
    pub dock: DockRules,
    /// Present on a write that changed state: what the state left held.
    pub digest: Option<Digest>,
    pub now_ms: u64,
    pub limits: ProfileLimits,
}
