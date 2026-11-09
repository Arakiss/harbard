//! Who is asking, and what their Pairing role lets them do.
//!
//! Every request is identified once, before routing:
//!
//! - **The hub's own key** (`hub.token`, or `--token`): the local console and
//!   CLI. Operator, but only from this machine and never on behalf of a
//!   device: a request that presents it with another device's id is the
//!   shared secret of the old pairing, and is refused.
//! - **A device token** (`harbard hub invite`): that device, with the role
//!   its card holds now. Read fresh on every request, so a revocation or a
//!   role change applies to the next one.
//! - **An unclaimed invite**: it may only pair (`POST /api/v1/devices`).
//!
//! Then [`permit`] is the one allowlist per role. Operator reaches every
//! route (each route keeps its own doors, such as the desk's loopback-only
//! rule). Workshop reaches its own lane; Display reaches the summary.

use harbard_core::pairing::{self, Credential, Verdict};
use harbard_core::{Access, DeviceCard, Store};
use serde_json::json;
use tiny_http::Method;

use super::fleet_routes::ApiError;

/// The caller of one request.
#[derive(Debug, Clone)]
pub enum Caller {
    /// The hub's own key, from this machine.
    Local,
    /// A paired device.
    Device(DeviceCard),
    /// An invite nobody claimed yet.
    Invite(Credential),
}

impl Caller {
    pub fn access(&self) -> Access {
        match self {
            Caller::Local => Access::Operator,
            Caller::Device(card) => card.access,
            Caller::Invite(c) => c.access,
        }
    }

    pub fn device(&self) -> Option<&DeviceCard> {
        match self {
            Caller::Device(card) => Some(card),
            _ => None,
        }
    }

    /// The device's id, for the per-device log and the heartbeat.
    pub fn device_id(&self) -> Option<&str> {
        self.device().map(|c| c.id.as_str())
    }
}

/// What the request presented.
pub struct Presented<'a> {
    pub token: Option<&'a str>,
    pub device: Option<&'a str>,
    pub loopback: bool,
}

/// The legacy refusal body, kept word for word: clients match on it.
fn unauthorized() -> ApiError {
    ApiError::new(
        401,
        "unauthorized",
        "unauthorized; present Authorization: Bearer <hub-token>",
    )
}

/// Identify the caller. `hub_key` is the hub's own key (`None` only for an
/// in-process server built without one); `hub_id` the hub's device id.
pub fn identify(
    store: &Store,
    hub_key: Option<&str>,
    hub_id: &str,
    presented: &Presented<'_>,
    now_ms: u64,
) -> Result<Caller, ApiError> {
    use subtle::ConstantTimeEq;

    let Some(hub_key) = hub_key.filter(|k| !k.is_empty()) else {
        return Ok(Caller::Local);
    };
    let Some(token) = presented.token.map(str::trim).filter(|t| !t.is_empty()) else {
        return Err(unauthorized());
    };
    let device = presented.device.map(str::trim).filter(|d| !d.is_empty());
    if bool::from(token.as_bytes().ct_eq(hub_key.as_bytes())) {
        if device.is_some_and(|d| d != hub_id) {
            return Err(ApiError::new(
                403,
                "shared_token_retired",
                "the hub's own key no longer speaks for a device; pair it with its own credential: harbard hub invite --name <device>",
            ));
        }
        if !presented.loopback {
            return Err(ApiError::new(
                403,
                "hub_key_is_local",
                "the hub's own key works only on this machine; pair the device: harbard hub invite --name <device>",
            ));
        }
        return Ok(Caller::Local);
    }
    match pairing::verify(store, token, now_ms) {
        Verdict::Device(card) => {
            if device.is_some_and(|d| d != card.id) {
                return Err(ApiError::new(
                    403,
                    "wrong_device",
                    "this credential belongs to another device",
                ));
            }
            Ok(Caller::Device(card))
        }
        Verdict::Invite(entry) => Ok(Caller::Invite(entry)),
        Verdict::Revoked => Err(ApiError::new(
            401,
            "revoked",
            "this device was revoked in Preferencias › Equipos; ask for a new invite",
        )),
        Verdict::Expired => Err(ApiError::new(
            401,
            "invite_expired",
            "this invite expired; ask for a new one: harbard hub invite",
        )),
        Verdict::Unknown => Err(unauthorized()),
    }
}

fn read(method: &Method) -> bool {
    matches!(method, Method::Get | Method::Head)
}

/// Workshop: publish its own sessions, send and receive their notes, read the
/// agent brief block and attachments addressed to it, and the harmless
/// metadata. Routes that need the caller filter further themselves.
fn workshop_may(method: &Method, path: &str) -> bool {
    if read(method) {
        return matches!(
            path,
            "/api/v1/meta"
                | "/api/v1/health"
                | "/api/v1/protocol"
                | "/api/v1/events"
                | "/api/v1/profile/brief"
        ) || path.starts_with("/api/v1/attachments/");
    }
    *method == Method::Post && matches!(path, "/api/v1/devices" | "/api/v1/events")
}

/// Display: the summary and the harmless metadata. No write, no body.
fn display_may(method: &Method, path: &str) -> bool {
    read(method) && matches!(path, "/api/v1/meta" | "/api/v1/health" | "/api/v1/summary")
}

/// Whether `caller` may reach `method path` at all.
pub fn permit(caller: &Caller, method: &Method, path: &str) -> Result<(), ApiError> {
    let allowed = match caller {
        Caller::Invite(_) => *method == Method::Post && path == "/api/v1/devices",
        _ => match caller.access() {
            Access::Operator => true,
            Access::Workshop => workshop_may(method, path),
            Access::Display => display_may(method, path),
        },
    };
    if allowed {
        return Ok(());
    }
    let role = match caller {
        Caller::Invite(_) => "invite",
        _ => caller.access().as_str(),
    };
    Err(ApiError::new(
        403,
        "role_forbids",
        format!("a {role} device cannot reach {} {path}", method.as_str()),
    )
    .with("role", json!(role)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device(access: Access) -> Caller {
        let mut card = harbard_core::LocalDevice {
            id: "d".into(),
            name: "d".into(),
            role: harbard_core::DeviceRole::Satellite,
        }
        .to_card(0, vec![]);
        card.access = access;
        Caller::Device(card)
    }

    #[test]
    fn operator_reaches_every_route() {
        for path in [
            "/api/v1/profile",
            "/api/v1/live/x",
            "/api/v1/workspace/save",
        ] {
            assert!(permit(&device(Access::Operator), &Method::Get, path).is_ok());
            assert!(permit(&Caller::Local, &Method::Post, path).is_ok());
        }
    }

    #[test]
    fn workshop_reaches_its_lane_only() {
        let t = device(Access::Workshop);
        assert!(permit(&t, &Method::Post, "/api/v1/events").is_ok());
        assert!(permit(&t, &Method::Get, "/api/v1/profile/brief").is_ok());
        for path in [
            "/api/v1/profile",
            "/api/v1/retro",
            "/api/v1/feedback",
            "/api/v1/live/x",
        ] {
            let e = permit(&t, &Method::Get, path).unwrap_err();
            assert_eq!((e.status, e.code), (403, "role_forbids"), "{path}");
        }
        assert!(permit(&t, &Method::Post, "/api/v1/notes").is_err());
    }

    #[test]
    fn display_never_writes() {
        let p = device(Access::Display);
        assert!(permit(&p, &Method::Get, "/api/v1/summary").is_ok());
        for path in ["/api/v1/devices", "/api/v1/events", "/api/v1/notes"] {
            assert!(permit(&p, &Method::Post, path).is_err(), "{path}");
        }
        assert!(permit(&p, &Method::Get, "/api/v1/events").is_err());
    }

    #[test]
    fn an_invite_may_only_pair() {
        let invite = Caller::Invite(Credential {
            hash: "h".into(),
            device_id: None,
            name: "n".into(),
            access: Access::Workshop,
            created_ms: 0,
            expires_ms: None,
            revoked_ms: None,
            workspaces: Vec::new(),
        });
        assert!(permit(&invite, &Method::Post, "/api/v1/devices").is_ok());
        assert!(permit(&invite, &Method::Get, "/api/v1/meta").is_err());
    }
}
