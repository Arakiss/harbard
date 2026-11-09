//! Device identity for the hub and its satellites.
//!
//! Agents are sessions; machines are devices. Each OS install keeps a stable
//! id in `device.json`. The hub keeps a registry in `devices.json` (join +
//! heartbeat). Join/leave also append bus events so the console can show a
//! business card without inventing a second presence model.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::store::Store;

/// Reserved harness string for device presence events on the bus.
pub const HARNESS_DEVICE: &str = "device";

/// How this machine relates to the hub.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum DeviceRole {
    #[default]
    Hub,
    Satellite,
}

impl DeviceRole {
    pub fn as_str(self) -> &'static str {
        match self {
            DeviceRole::Hub => "hub",
            DeviceRole::Satellite => "satellite",
        }
    }
}

/// What a paired device may do on the hub: the three roles of
/// Preferences › Pairing. A new device is [`Access::Workshop`]; Operator is
/// a deliberate promotion, never a default and never self-declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Access {
    /// A machine the operator sits at: everything.
    // The alias exists only to read old files.
    #[serde(alias = "operador")]
    Operator,
    /// A machine that runs agents: its own sessions, their notes, and the
    /// agent brief block of the profile.
    // The alias exists only to read old files.
    #[default]
    #[serde(alias = "taller")]
    Workshop,
    /// A display: the summary, never a write or a body.
    // The alias exists only to read old files.
    #[serde(alias = "pantalla")]
    Display,
}

impl Access {
    pub fn as_str(self) -> &'static str {
        match self {
            Access::Operator => "operator",
            Access::Workshop => "workshop",
            Access::Display => "display",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "operator" => Some(Access::Operator),
            "workshop" => Some(Access::Workshop),
            "display" => Some(Access::Display),
            // Exist only for installed tools and old files that still spell it this way.
            "operador" => Some(Access::Operator),
            "taller" => Some(Access::Workshop),
            "pantalla" => Some(Access::Display),
            _ => None,
        }
    }
}

/// Local identity written under `~/.harbard/device.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalDevice {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub role: DeviceRole,
}

/// Hub-visible business card.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceCard {
    pub id: String,
    pub name: String,
    pub host: String,
    pub role: DeviceRole,
    #[serde(default)]
    pub os: String,
    #[serde(default)]
    pub arch: String,
    #[serde(default)]
    pub harnesses: Vec<String>,
    pub joined_ms: u64,
    pub last_seen_ms: u64,
    /// The Pairing role. Kept by the hub: a card a device posts never sets it.
    #[serde(default)]
    pub access: Access,
    /// When the hub issued this device's credential (0: paired before
    /// per-device credentials, so it holds none).
    #[serde(default)]
    pub created_ms: u64,
    /// When the operator revoked it. A revoked device is refused at once.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revoked_ms: Option<u64>,
    /// Sessions this device published, newest last, at most
    /// [`DEVICE_SESSIONS_MAX`]. Only that device may speak as them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sessions: Vec<String>,
    /// The workspaces this device is granted (`harbard hub grant`). Kept by
    /// the hub, never posted. Empty means the hub's default workspace only;
    /// an Operator device is the operator's own machine and sees every one.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workspaces: Vec<String>,
    /// The harbard version the device runs, as it last declared it
    /// (empty: a build from before devices said so).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub version: String,
    /// The commit that build came from (empty when unknown). Pairing
    /// compares it with the hub's own to say who is behind.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub commit: String,
}

/// How many published sessions a card remembers.
pub const DEVICE_SESSIONS_MAX: usize = 256;

impl DeviceCard {
    /// Whether this device published `session`.
    pub fn owns_session(&self, session: &str) -> bool {
        self.sessions.iter().any(|s| s == session)
    }

    /// Only what a device may say about itself: its id, name, host, OS,
    /// architecture and harnesses. Everything the hub decides (role,
    /// pairing time, revocation, published sessions, join and last seen)
    /// is reset, so a posted card can never carry it in.
    pub fn declared(&self) -> DeviceCard {
        DeviceCard {
            id: self.id.clone(),
            name: self.name.clone(),
            host: self.host.clone(),
            role: DeviceRole::Satellite,
            os: self.os.clone(),
            arch: self.arch.clone(),
            harnesses: self.harnesses.clone(),
            joined_ms: 0,
            last_seen_ms: 0,
            access: Access::Workshop,
            created_ms: 0,
            revoked_ms: None,
            sessions: Vec::new(),
            workspaces: Vec::new(),
            version: self.version.clone(),
            commit: self.commit.clone(),
        }
    }
}

/// The host a device is paired with, fixed for good: what it declared,
/// unless that is empty, carries a `~`, or is already another card's host
/// (the hub's included), in which case its full id is appended. Ids are
/// unique among cards and only this rule writes a `~`, so no two cards ever
/// hold the same host, and a device's events can never be stamped like the
/// hub's or another device's.
pub fn device_host(declared: &str, taken: &[&str], device_id: &str) -> String {
    let declared = declared.trim();
    let clash = declared.is_empty()
        || declared.contains('~')
        || taken
            .iter()
            .any(|t| t.trim().eq_ignore_ascii_case(declared));
    if clash {
        format!("{declared}~{device_id}")
    } else {
        declared.to_string()
    }
}

/// The host stamped on a device's event: its paired host, unless the hub's
/// own name has since become the same (a renamed Mac), in which case its id
/// is appended, as at pairing.
pub fn stamped_host(paired: &str, hub_host: &str, device_id: &str) -> String {
    if paired.trim().eq_ignore_ascii_case(hub_host.trim()) {
        format!("{}~{device_id}", paired.trim())
    } else {
        paired.to_string()
    }
}

/// Hub registry (`devices.json`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceRegistry {
    #[serde(default)]
    pub devices: Vec<DeviceCard>,
}

impl DeviceRegistry {
    pub fn path(store: &Store) -> PathBuf {
        store.root().join("devices.json")
    }

    pub fn load(store: &Store) -> Self {
        let path = Self::path(store);
        match fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    pub fn save(&self, store: &Store) -> io::Result<()> {
        let path = Self::path(store);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::other(format!("devices serialize: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, raw)?;
        fs::rename(tmp, path)?;
        Ok(())
    }

    /// Upsert by id. Returns `(card, is_new_join)`.
    ///
    /// What a device says about itself (name, host, harnesses) is taken;
    /// what the hub decides about it (access, created, revoked, sessions) is
    /// kept from the stored card, so a posted card can never promote itself.
    pub fn upsert(&mut self, mut card: DeviceCard, now_ms: u64) -> (DeviceCard, bool) {
        if let Some(existing) = self.devices.iter_mut().find(|d| d.id == card.id) {
            existing.name = card.name.clone();
            existing.host = card.host.clone();
            existing.role = card.role;
            existing.os = card.os.clone();
            existing.arch = card.arch.clone();
            existing.harnesses = card.harnesses.clone();
            // A build that does not say keeps what an earlier one said.
            if !card.version.is_empty() {
                existing.version = card.version.clone();
                existing.commit = card.commit.clone();
            }
            existing.last_seen_ms = now_ms;
            return (existing.clone(), false);
        }
        card.joined_ms = now_ms;
        card.last_seen_ms = now_ms;
        self.devices.push(card.clone());
        (card, true)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut DeviceCard> {
        self.devices.iter_mut().find(|d| d.id == id)
    }

    pub fn get(&self, id: &str) -> Option<&DeviceCard> {
        self.devices.iter().find(|d| d.id == id)
    }
}

impl LocalDevice {
    pub fn path(store: &Store) -> PathBuf {
        store.root().join("device.json")
    }

    /// Load or create a stable local device identity.
    pub fn load_or_create(store: &Store, role: DeviceRole) -> io::Result<Self> {
        let path = Self::path(store);
        if path.is_file() {
            let raw = fs::read_to_string(&path)?;
            if let Ok(mut d) = serde_json::from_str::<LocalDevice>(&raw) {
                if d.id.trim().is_empty() {
                    d.id = new_device_id();
                }
                if d.name.trim().is_empty() {
                    d.name = default_device_name();
                }
                // Role may be upgraded when this machine becomes the hub.
                if role == DeviceRole::Hub {
                    d.role = DeviceRole::Hub;
                }
                Self::write(&path, &d)?;
                return Ok(d);
            }
        }
        let d = LocalDevice {
            id: new_device_id(),
            name: default_device_name(),
            role,
        };
        Self::write(&path, &d)?;
        Ok(d)
    }

    fn write(path: &Path, d: &LocalDevice) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let raw = serde_json::to_string_pretty(d)
            .map_err(|e| io::Error::other(format!("device serialize: {e}")))?;
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, raw)?;
        fs::rename(tmp, path)?;
        Ok(())
    }

    pub fn to_card(&self, now_ms: u64, harnesses: Vec<String>) -> DeviceCard {
        DeviceCard {
            id: self.id.clone(),
            name: self.name.clone(),
            host: crate::hostname(),
            role: self.role,
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
            harnesses,
            joined_ms: now_ms,
            last_seen_ms: now_ms,
            // The hub is the machine the operator sits at; anything else is
            // decided by the hub when it pairs.
            access: if self.role == DeviceRole::Hub {
                Access::Operator
            } else {
                Access::Workshop
            },
            created_ms: 0,
            revoked_ms: None,
            sessions: Vec::new(),
            workspaces: Vec::new(),
            // The CLI adds the commit it was built from (`build.rs`).
            version: env!("CARGO_PKG_VERSION").to_string(),
            commit: String::new(),
        }
    }
}

fn new_device_id() -> String {
    // Prefer ULID when available via the same crate the bus uses elsewhere.
    ulid::Ulid::generate().to_string().to_lowercase()
}

fn default_device_name() -> String {
    let host = crate::hostname();
    // Drop .local / long domains for a short card label.
    host.split('.').next().unwrap_or(&host).to_string()
}

/// One-line card text for a device_join / device_leave bus event.
pub fn card_summary(card: &DeviceCard, verb: &str) -> String {
    let harnesses = if card.harnesses.is_empty() {
        "no harnesses".to_string()
    } else {
        card.harnesses.join(", ")
    };
    format!(
        "{verb} · {} ({}) · {} · {}",
        card.name,
        card.role.as_str(),
        card.host,
        harnesses
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::test_util::TempHome;

    #[test]
    fn local_device_stable_across_loads() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let a = LocalDevice::load_or_create(&store, DeviceRole::Hub).unwrap();
        let b = LocalDevice::load_or_create(&store, DeviceRole::Hub).unwrap();
        assert_eq!(a.id, b.id);
        assert_eq!(a.role, DeviceRole::Hub);
    }

    #[test]
    fn registry_upsert_marks_first_join() {
        let home = TempHome::new();
        let store = Store::at(home.path());
        let mut reg = DeviceRegistry::default();
        let card = DeviceCard {
            id: "dev1".into(),
            name: "laptop".into(),
            host: "laptop.local".into(),
            role: DeviceRole::Satellite,
            os: "macos".into(),
            arch: "aarch64".into(),
            harnesses: vec!["claude-code".into()],
            joined_ms: 0,
            last_seen_ms: 0,
            access: Access::Workshop,
            created_ms: 0,
            revoked_ms: None,
            sessions: Vec::new(),
            workspaces: Vec::new(),
            version: String::new(),
            commit: String::new(),
        };
        let (_, is_new) = reg.upsert(card.clone(), 100);
        assert!(is_new);
        let (_, again) = reg.upsert(card, 200);
        assert!(!again);
        assert_eq!(reg.devices[0].last_seen_ms, 200);
        assert_eq!(reg.devices[0].joined_ms, 100);
        reg.save(&store).unwrap();
        let loaded = DeviceRegistry::load(&store);
        assert_eq!(loaded.devices.len(), 1);
    }

    #[test]
    fn a_device_never_carries_the_hubs_host_or_another_devices() {
        let taken = ["mac.local", "laptop.local"];
        assert_eq!(
            device_host("borealis.local", &taken, "d9"),
            "borealis.local"
        );
        assert_eq!(device_host("MAC.local ", &taken, "d9"), "MAC.local~d9");
        assert_eq!(device_host("Laptop.local", &taken, "d9"), "Laptop.local~d9");
        assert_eq!(
            device_host("laptop.local~d1", &taken, "d9"),
            "laptop.local~d1~d9"
        );
        assert_eq!(device_host("", &taken, "d9"), "~d9");
        assert_eq!(stamped_host("mac.local", "MAC.local", "d9"), "mac.local~d9");
        assert_eq!(
            stamped_host("laptop.local", "mac.local", "d9"),
            "laptop.local"
        );
    }
}
