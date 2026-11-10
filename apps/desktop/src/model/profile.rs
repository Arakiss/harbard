//! The operator's profile, status line and "Do not disturb", as
//! `GET /api/v1/profile` serves them (`harbard_core::profile`), plus the
//! digests "While you were away" shows (`/api/v1/digests`).

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProfileStatus {
    pub emoji: String,
    pub text: String,
    pub clear_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProfileHours {
    pub start: String,
    pub end: String,
    /// ISO weekdays, Monday = 1.
    pub days: Vec<u8>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Dnd {
    pub since_ms: u64,
    pub until_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct StateSpan {
    /// available, focused, swamped, away, reviewing, done
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
    pub dnd: Option<Dnd>,
    pub state: Option<StateSpan>,
    /// 30m, 1h, tomorrow
    pub dnd_default: Option<String>,
    pub digest_hourly: bool,
    /// `None` = never set (on).
    pub away_suggest: Option<bool>,
    pub utc_offset_min: Option<i32>,
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

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProfileLimits {
    pub context: u64,
    pub name: u64,
    pub address: u64,
    pub status_text: u64,
    pub photo_bytes: u64,
}

/// One held note in a digest (`harbard_core::hold::DigestItem`).
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DigestItem {
    pub id: String,
    pub session: String,
    pub harness: String,
    /// fyi, ask, warning, handoff
    pub kind: String,
    pub text: String,
    pub ts_ms: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Digest {
    pub id: String,
    pub state: String,
    /// left, reviewing, hourly
    pub reason: String,
    pub from_ms: u64,
    pub to_ms: u64,
    pub items: Vec<DigestItem>,
    pub read: bool,
}

/// `GET /api/v1/digests`.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Digests {
    pub items: Vec<Digest>,
    pub cut_ms: u64,
}

/// `GET /api/v1/digests/pending`: what Away holds so far.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct PendingDigest {
    pub items: Vec<DigestItem>,
}

/// `GET /api/v1/profile` (and every profile write's answer).
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct ProfileView {
    pub profile: Profile,
    /// In force now: an expired status is already null.
    pub status: Option<ProfileStatus>,
    pub quiet: bool,
    pub quiet_until_ms: Option<u64>,
    /// The coordination state in force (available when none or ended).
    pub state: String,
    pub state_until_ms: Option<u64>,
    pub dock: DockRules,
    /// On a write that changed state: what the state left held.
    pub digest: Option<Digest>,
    pub now_ms: u64,
    pub limits: ProfileLimits,
}
