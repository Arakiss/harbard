//! Preferences › Pairing: the paired devices, their roles, revocation and
//! the per-device log (`harbard_core::pairing`).
//!
//! - `GET /api/v1/pairing`: every device with its role, when it was paired
//!   and last seen, whether it holds a credential, and its latest actions.
//! - `GET /api/v1/pairing/<id>/log?limit=`: that device's remote reads and
//!   writes, newest first.
//! - `POST /api/v1/pairing/<id>/role` `{role, confirm?}`: Operator needs
//!   `confirm` to be the device's name, typed by the operator.
//! - `POST /api/v1/pairing/<id>/revoke`: refused from its next request.
//! - `POST /api/v1/pairing/invites/<ref>/revoke`: a pending invite (a leaked
//!   pair line) stops pairing at once. The list carries each pending invite
//!   by name and `ref`, never its token or full hash.
//!
//! Reads are Operator's (the role allowlist keeps the others out). Writes
//! go through the desk's door: loopback, the operator's origin, never a
//! device.

use harbard_core::pairing::{self, RoleError};
use harbard_core::{Access, DeviceRole, Store};
use serde_json::{Value, json};

use super::fleet_routes::ApiError;

/// Actions shown on each row of the list.
const RECENT: usize = 3;
/// Default and largest page of one device's log.
const LOG_DEFAULT: usize = 100;
const LOG_MAX: usize = 500;

pub fn list_body(store: &Store, now_ms: u64) -> Value {
    let reg = harbard_core::DeviceRegistry::load(store);
    let creds = pairing::Credentials::load(store);
    let devices: Vec<Value> = reg
        .devices
        .iter()
        .map(|d| {
            let hub = d.role == DeviceRole::Hub;
            let (reads, writes) = pairing::action_counts(store, &d.id);
            json!({
                "id": d.id,
                "name": d.name,
                "host": d.host,
                "os": d.os,
                "hub": hub,
                "access": if hub { Access::Operator } else { d.access },
                "created_ms": d.created_ms,
                "joined_ms": d.joined_ms,
                "last_seen_ms": d.last_seen_ms,
                "revoked_ms": d.revoked_ms,
                "credential": hub || pairing::has_credential(&creds, &d.id),
                "recent": pairing::actions(store, &d.id, RECENT),
                "reads": reads,
                "writes": writes,
            })
        })
        .collect();
    let invites: Vec<Value> = pairing::pending_invites(&creds, now_ms)
        .into_iter()
        .map(|c| {
            json!({
                "ref": pairing::invite_ref(c),
                "name": c.name,
                "access": c.access,
                "created_ms": c.created_ms,
                "expires_ms": c.expires_ms,
            })
        })
        .collect();
    json!({ "devices": devices, "invites": invites, "now_ms": now_ms })
}

fn unknown(id: &str) -> ApiError {
    ApiError::new(404, "unknown_device", "no such device").with("device", json!(id))
}

/// `rest` is what follows `/api/v1/pairing/`: `<id>/log`.
pub fn log_body(store: &Store, rest: &str, limit: Option<String>) -> Result<Value, ApiError> {
    let id = rest
        .strip_suffix("/log")
        .filter(|id| pairing::valid_device_id(id))
        .ok_or_else(|| ApiError::new(404, "not_found", "not found"))?;
    let reg = harbard_core::DeviceRegistry::load(store);
    let card = reg.get(id).ok_or_else(|| unknown(id))?;
    let limit = match limit {
        None => LOG_DEFAULT,
        Some(raw) => raw
            .parse::<usize>()
            .ok()
            .filter(|n| (1..=LOG_MAX).contains(n))
            .ok_or_else(|| {
                ApiError::bad_request("limit", format!("limit must be between 1 and {LOG_MAX}"))
            })?,
    };
    Ok(json!({
        "device": { "id": card.id, "name": card.name },
        "actions": pairing::actions(store, id, limit),
    }))
}

/// `POST /api/v1/pairing/<id>/<action>`.
pub fn write(store: &Store, rest: &str, body: &Value, now_ms: u64) -> Result<Value, ApiError> {
    if let Some(reference) = rest
        .strip_prefix("invites/")
        .and_then(|r| r.strip_suffix("/revoke"))
    {
        return revoke_invite(store, reference, now_ms);
    }
    let (id, action) = rest
        .split_once('/')
        .filter(|(id, _)| pairing::valid_device_id(id))
        .ok_or_else(|| ApiError::new(404, "not_found", "not found"))?;
    let reg = harbard_core::DeviceRegistry::load(store);
    let card = reg.get(id).cloned().ok_or_else(|| unknown(id))?;
    if card.role == DeviceRole::Hub {
        return Err(ApiError::new(
            409,
            "hub_itself",
            "this Mac is the hub: always Operador, never revoked",
        ));
    }
    match action {
        "role" => {
            let raw = body.get("role").and_then(Value::as_str).unwrap_or("");
            let access = Access::parse(raw).ok_or_else(|| {
                ApiError::bad_request("role", "role must be operador, taller or pantalla")
            })?;
            if access == Access::Operator {
                // The deliberate act: the operator types the device's name.
                let typed = body.get("confirm").and_then(Value::as_str).unwrap_or("");
                if !typed.trim().eq_ignore_ascii_case(card.name.trim()) {
                    return Err(ApiError::new(
                        422,
                        "confirm_required",
                        "to make a device Operador, confirm with its name",
                    )
                    .with("field", json!("confirm")));
                }
            }
            match pairing::set_access(store, id, access).map_err(store_failed)? {
                Ok(card) => {
                    Ok(json!({ "ok": true, "device": { "id": card.id, "access": card.access } }))
                }
                Err(RoleError::Revoked) => Err(ApiError::new(
                    409,
                    "revoked",
                    "a revoked device keeps no role; pair it again",
                )),
                Err(RoleError::NotFound) => Err(unknown(id)),
                Err(RoleError::Hub) => Err(ApiError::new(409, "hub_itself", "this Mac is the hub")),
            }
        }
        "revoke" => match pairing::revoke(store, id, now_ms).map_err(store_failed)? {
            Some(card) => Ok(
                json!({ "ok": true, "device": { "id": card.id, "revoked_ms": card.revoked_ms } }),
            ),
            None => Err(unknown(id)),
        },
        _ => Err(ApiError::new(404, "not_found", "not found")),
    }
}

/// Cancel one pending invite by its `ref` (exactly the 12 digits the list
/// shows), never by name: the desk names exactly one.
fn revoke_invite(store: &Store, reference: &str, now_ms: u64) -> Result<Value, ApiError> {
    let reference = reference.trim().to_ascii_lowercase();
    if reference.len() != 12 || !reference.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(ApiError::new(
            404,
            "unknown_invite",
            "no such pending invite",
        ));
    }
    let revoked = pairing::revoke_invite(store, &reference, now_ms).map_err(store_failed)?;
    match revoked.as_slice() {
        [one] => Ok(
            json!({ "ok": true, "invite": { "ref": pairing::invite_ref(one), "name": one.name } }),
        ),
        [] => Err(ApiError::new(
            404,
            "unknown_invite",
            "no such pending invite",
        )),
        _ => Ok(json!({ "ok": true, "revoked": revoked.len() })),
    }
}

fn store_failed(e: std::io::Error) -> ApiError {
    eprintln!(
        "{}",
        json!({ "component": "harbard-serve", "event": "pairing_store_failed", "fields": { "error": e.to_string() } })
    );
    ApiError::new(
        500,
        "store_failed",
        "the device registry could not be written",
    )
}
