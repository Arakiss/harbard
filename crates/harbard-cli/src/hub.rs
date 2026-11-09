//! Satellite → hub HTTP client (M4a).
//!
//! When `HARBARD_HUB` (+ token) is set — or `~/.harbard/satellite.json` from
//! `harbard pair` — write verbs and the satellite loop talk to the mini's
//! serve instead of a local conversational store.

use std::env;
use std::fs;
use std::io;
use std::time::Duration;

use harbard_core::{DeviceCard, Event, Store};
use serde_json::Value;

use crate::hub_dx::{SatelliteConfig, read_hub_token};

const TIMEOUT: Duration = Duration::from_secs(10);
/// The brief block is fetched inside a hook: never wait long for it.
const BRIEF_TIMEOUT: Duration = Duration::from_millis(1500);

/// The satellite's copy of the hub's brief block, `{home}/profile-brief.json`
/// (0600): fresh for a minute, then refetched; when the hub cannot be
/// reached, a copy up to a day old still serves, so the agents keep
/// respecting the operator through a short outage.
const BRIEF_FRESH_MS: u64 = 60_000;
const BRIEF_STALE_MS: u64 = 24 * 60 * 60 * 1000;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct RemoteBrief {
    pub fetched_ms: u64,
    pub block: Option<String>,
    pub prompt: Option<String>,
    /// The hub and the device the copy was fetched for: a copy written for
    /// another pairing is never read.
    #[serde(default)]
    pub hub: String,
    #[serde(default)]
    pub device: String,
}

fn brief_cache_path(store: &Store) -> std::path::PathBuf {
    store.root().join("profile-brief.json")
}

/// Why the brief block could not be fetched.
#[derive(Debug)]
pub enum BriefError {
    /// The hub answered 401 or 403: this device was revoked, demoted or
    /// never paired. Nothing cached may be served any more.
    Refused,
    /// The hub could not be reached, or answered something else.
    Unreachable,
}

/// The operator's brief block as the hub serves it, when this machine is a
/// paired satellite; `None` when it is not (the local profile applies).
///
/// The copy on disk is fresh for a minute and, when the hub cannot be
/// reached, usable for a day. A copy from the future (a bad clock or a bad
/// write) is stale, and a copy for another hub or device is ignored. When
/// the hub refuses this device, the copy is deleted and nothing is served.
pub fn remote_brief(store: &Store, now_ms: u64) -> Option<RemoteBrief> {
    let client = HubClient::resolve(store)?;
    let path = brief_cache_path(store);
    let cached: Option<RemoteBrief> = fs::read_to_string(&path)
        .ok()
        .and_then(|raw| serde_json::from_str::<RemoteBrief>(&raw).ok())
        .filter(|c| c.hub == client.base && c.device == client.device_id)
        .filter(|c| c.fetched_ms <= now_ms);
    let age = |c: &RemoteBrief| now_ms - c.fetched_ms;
    if let Some(c) = cached.as_ref().filter(|c| age(c) < BRIEF_FRESH_MS) {
        return Some(c.clone());
    }
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    match client.profile_brief() {
        Ok(v) => {
            let fresh = RemoteBrief {
                fetched_ms: now_ms,
                block: text(&v, "block"),
                prompt: text(&v, "prompt"),
                hub: client.base.clone(),
                device: client.device_id.clone(),
            };
            if let Ok(raw) = serde_json::to_string(&fresh) {
                let _ = crate::hub_dx::write_private(&path, raw);
            }
            Some(fresh)
        }
        Err(BriefError::Refused) => {
            let _ = fs::remove_file(&path);
            Some(RemoteBrief::default())
        }
        Err(BriefError::Unreachable) => Some(
            cached
                .filter(|c| age(c) < BRIEF_STALE_MS)
                .unwrap_or_default(),
        ),
    }
}

#[derive(Debug, Clone)]
pub struct HubClient {
    pub base: String,
    pub token: String,
    pub device_id: String,
}

impl HubClient {
    pub fn resolve(store: &Store) -> Option<Self> {
        let sat = SatelliteConfig::load(store);
        let base = env::var("HARBARD_HUB")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| sat.as_ref().map(|c| c.hub.clone()))
            .filter(|s| !s.is_empty())?;

        let token = env::var("HARBARD_HUB_TOKEN")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                sat.as_ref()
                    .map(|c| c.token.clone())
                    .filter(|t| !t.is_empty())
            })
            .or_else(|| read_hub_token(store));

        let Some(token) = token.filter(|t| !t.is_empty()) else {
            eprintln!(
                "harbard: hub URL is set but no token — run `harbard pair '…'` or set HARBARD_HUB_TOKEN"
            );
            return None;
        };

        let device_id = env::var("HARBARD_DEVICE_ID")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or_else(|| {
                sat.as_ref()
                    .map(|c| c.device_id.clone())
                    .filter(|d| !d.is_empty())
            })
            .or_else(|| read_device_id(store))
            .unwrap_or_default();

        Some(HubClient {
            base: base.trim_end_matches('/').to_string(),
            token,
            device_id,
        })
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base, path)
    }

    fn agent(&self) -> ureq::Agent {
        ureq::AgentBuilder::new().timeout(TIMEOUT).build()
    }

    fn auth(&self, req: ureq::Request) -> ureq::Request {
        let mut req = req.set("Authorization", &format!("Bearer {}", self.token));
        if !self.device_id.is_empty() {
            req = req.set("X-Harbard-Device", &self.device_id);
        }
        req
    }

    pub fn append_event(&self, event: &Event) -> io::Result<Event> {
        let body = serde_json::to_value(event)
            .map_err(|e| io::Error::other(format!("serialize event: {e}")))?;
        let resp = self
            .auth(self.agent().post(&self.url("/api/v1/events")))
            .send_json(body)
            .map_err(|e| io::Error::other(e.to_string()))?;
        let v: Value = resp
            .into_json()
            .map_err(|e| io::Error::other(e.to_string()))?;
        if let Some(ev) = v.get("event") {
            serde_json::from_value(ev.clone())
                .map_err(|e| io::Error::other(format!("bad event response: {e}")))
        } else if let Some(err) = v.get("error").and_then(|e| e.as_str()) {
            Err(io::Error::other(err.to_string()))
        } else {
            Ok(event.clone())
        }
    }

    /// Post an event inside a hook: the brief timeout, the answer ignored.
    pub fn announce(&self, event: &Event) -> io::Result<()> {
        let body = serde_json::to_value(event)
            .map_err(|e| io::Error::other(format!("serialize event: {e}")))?;
        self.auth(
            ureq::AgentBuilder::new()
                .timeout(BRIEF_TIMEOUT)
                .build()
                .post(&self.url("/api/v1/events")),
        )
        .send_json(body)
        .map(|_| ())
        .map_err(|e| io::Error::other(e.to_string()))
    }

    pub fn register_device(&self, card: &DeviceCard) -> io::Result<(DeviceCard, bool)> {
        let resp = self
            .auth(self.agent().post(&self.url("/api/v1/devices")))
            .send_json(serde_json::to_value(card).map_err(|e| io::Error::other(e.to_string()))?)
            .map_err(|e| io::Error::other(e.to_string()))?;
        let v: Value = resp
            .into_json()
            .map_err(|e| io::Error::other(e.to_string()))?;
        let card: DeviceCard = serde_json::from_value(
            v.get("device")
                .cloned()
                .ok_or_else(|| io::Error::other("missing device in response"))?,
        )
        .map_err(|e| io::Error::other(e.to_string()))?;
        let is_new = v.get("joined").and_then(|j| j.as_bool()).unwrap_or(false);
        Ok((card, is_new))
    }

    pub fn list_devices(&self) -> io::Result<Vec<DeviceCard>> {
        let resp = self
            .auth(self.agent().get(&self.url("/api/v1/devices")))
            .call()
            .map_err(|e| io::Error::other(e.to_string()))?;
        let v: Value = resp
            .into_json()
            .map_err(|e| io::Error::other(e.to_string()))?;
        let devices = v.get("devices").cloned().unwrap_or(Value::Array(vec![]));
        serde_json::from_value(devices).map_err(|e| io::Error::other(e.to_string()))
    }

    /// `GET /api/v1/profile/brief`: the operator's brief block, the one part
    /// of the profile a Workshop device reads. A short timeout: it runs in the
    /// agents' hooks.
    pub fn profile_brief(&self) -> Result<Value, BriefError> {
        let resp = self
            .auth(
                ureq::AgentBuilder::new()
                    .timeout(BRIEF_TIMEOUT)
                    .build()
                    .get(&self.url("/api/v1/profile/brief")),
            )
            .call()
            .map_err(|e| match e {
                ureq::Error::Status(401 | 403, _) => BriefError::Refused,
                _ => BriefError::Unreachable,
            })?;
        resp.into_json().map_err(|_| BriefError::Unreachable)
    }
}

fn read_device_id(store: &Store) -> Option<String> {
    let raw = fs::read_to_string(store.root().join("device.json")).ok()?;
    let v: Value = serde_json::from_str(&raw).ok()?;
    v.get("id")
        .and_then(|id| id.as_str())
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}
