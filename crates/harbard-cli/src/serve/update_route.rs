//! `GET /api/v1/update` and `POST /api/v1/update/<check|install|prefs>`:
//! whether a newer harbard exists, what changed, and the one-click install
//! (docs/SPEC-UPDATES.md). The writes are the operator's desk: loopback,
//! never a satellite, same origin.

use harbard_core::Store;
use serde_json::Value;

use super::fleet_routes::ApiError;
use crate::update::{self, Channel, Prefs, run};

/// `GET /api/v1/update`.
pub fn body(store: &Store) -> Value {
    update::view(store)
}

/// `POST /api/v1/update/<action>`.
pub fn write(store: &Store, action: &str, body: &Value) -> Result<(u16, Value), ApiError> {
    match action {
        "check" => {
            if run::busy(store) {
                return Err(busy());
            }
            update::check(store, &Prefs::load(store));
            Ok((200, update::view(store)))
        }
        "install" => {
            let prefs = Prefs::load(store);
            match run::start(store, &prefs) {
                Ok(_) => Ok((202, update::view(store))),
                Err(e) => {
                    let code = leak(&e.code);
                    Err(ApiError::new(409, code, e.message))
                }
            }
        }
        "prefs" => {
            let mut prefs = Prefs::load(store);
            let mut rechecked = false;
            if let Some(raw) = body.get("channel") {
                let channel = raw.as_str().and_then(Channel::parse).ok_or_else(|| {
                    ApiError::bad_request("channel", "`channel` is main or releases")
                })?;
                rechecked = channel != prefs.channel;
                prefs.channel = channel;
            }
            if let Some(raw) = body.get("auto_check") {
                prefs.auto_check = raw.as_bool().ok_or_else(|| {
                    ApiError::bad_request("auto_check", "`auto_check` is a boolean")
                })?;
            }
            prefs
                .save(store)
                .map_err(|e| ApiError::new(500, "write_failed", e.to_string()))?;
            if rechecked && !run::busy(store) {
                update::check(store, &prefs);
            }
            Ok((200, update::view(store)))
        }
        _ => Err(ApiError::new(
            404,
            "unknown_action",
            format!("no update action {action:?}; use check, install or prefs"),
        )),
    }
}

fn busy() -> ApiError {
    ApiError::new(409, "busy", update::problem_message("busy"))
}

/// The codes a refusal can carry, as the `'static` strings `ApiError` keeps.
fn leak(code: &str) -> &'static str {
    match code {
        "busy" => "busy",
        "no_checkout" => "no_checkout",
        "not_on_main" => "not_on_main",
        "dirty" => "dirty",
        "diverged" => "diverged",
        "fetch_failed" => "fetch_failed",
        _ => "step_failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_actions_and_bad_prefs_are_refused() {
        let dir = std::env::temp_dir().join(format!(
            "harbard-update-route-{}-{}",
            std::process::id(),
            ulid::Ulid::generate()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let store = Store::at(&dir);
        assert_eq!(write(&store, "nope", &json!({})).unwrap_err().status, 404);
        let e = write(&store, "prefs", &json!({ "channel": "nightly" })).unwrap_err();
        assert_eq!((e.status, e.code), (400, "bad_request"));
        let e = write(&store, "prefs", &json!({ "auto_check": "yes" })).unwrap_err();
        assert_eq!(e.status, 400);
        let (status, view) = write(&store, "prefs", &json!({ "auto_check": false })).unwrap();
        assert_eq!(status, 200);
        assert_eq!(view["prefs"]["auto_check"], false);
        assert_eq!(view["prefs"]["channel"], "main");
        assert!(view["installed"]["version"].is_string());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
