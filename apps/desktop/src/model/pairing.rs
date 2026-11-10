//! `GET /api/v1/pairing`: the devices paired to this hub, their role and
//! their log of remote reads and writes (`serve/pairing_route.rs`).

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DeviceAction {
    pub ts_ms: u64,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub write: bool,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub host: String,
    pub os: String,
    /// This Mac: always Operator, never revoked.
    pub hub: bool,
    /// operator, workshop or display.
    pub access: String,
    pub created_ms: u64,
    pub joined_ms: u64,
    pub last_seen_ms: u64,
    pub revoked_ms: Option<u64>,
    pub credential: bool,
    pub recent: Vec<DeviceAction>,
    pub reads: u64,
    pub writes: u64,
    pub version: Option<String>,
    pub commit: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct Invite {
    #[serde(rename = "ref")]
    pub reference: String,
    pub name: String,
    pub access: String,
    pub created_ms: u64,
    pub expires_ms: Option<u64>,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct PairingView {
    pub devices: Vec<Device>,
    pub invites: Vec<Invite>,
    pub now_ms: u64,
}

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct DeviceLog {
    pub actions: Vec<DeviceAction>,
}
