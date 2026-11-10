//! Calls to serve on loopback, for everything the app does not read itself.
//!
//! Writes always go here: serve validates them and appends to the log, and the
//! hub sees the append. Reads that are not the workspace view (profile, theme,
//! history pages, retro, setup, pairing, update) also come from here; each is
//! one request when a screen opens or the operator acts.
//!
//! Every call blocks, so run it off the UI thread:
//! `cx.background_spawn(async move { api::get("/profile") })`.

// Every call returns `ApiError` by value: it is built once per failed request
// and read by the screen that made it, so its size never matters on a hot path.
#![allow(clippy::result_large_err)]

use serde::de::DeserializeOwned;
use serde_json::Value;
use std::time::Duration;

/// Same port serve binds by default; `HARBARD_API_PORT` moves it.
fn base() -> String {
    let port = std::env::var("HARBARD_API_PORT")
        .ok()
        .filter(|p| p.parse::<u16>().is_ok())
        .unwrap_or_else(|| "7419".into());
    format!("http://127.0.0.1:{port}/api/v1")
}

/// The hub's own key: `HARBARD_HUB_TOKEN`, else `~/.harbard/hub.token`.
/// Read per call so a rotated token is picked up without a restart.
fn token() -> Option<String> {
    if let Ok(t) = std::env::var("HARBARD_HUB_TOKEN")
        && !t.trim().is_empty()
    {
        return Some(t.trim().to_string());
    }
    let store = harbard_core::store::Store::resolve();
    std::fs::read_to_string(store.root().join("hub.token"))
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

/// A refused or failed call: serve's status and its error body when it sent one.
#[derive(Debug, Clone)]
pub struct ApiError {
    /// `None` when serve could not be reached.
    pub status: Option<u16>,
    /// serve's `error` code (or `reason`), when present.
    pub code: Option<String>,
    pub message: String,
    pub body: Value,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

impl ApiError {
    pub fn unreachable(&self) -> bool {
        self.status.is_none()
    }

    /// 404: the route does not exist on this serve (an older build).
    pub fn missing(&self) -> bool {
        self.status == Some(404)
    }
}

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(2))
        .timeout(Duration::from_secs(45))
        .build()
}

fn request(method: &str, path: &str) -> ureq::Request {
    let url = format!("{}{}", base(), path);
    let mut req = agent()
        .request(method, &url)
        .set("accept", "application/json");
    if let Some(t) = token() {
        req = req.set("authorization", &format!("Bearer {t}"));
    }
    req
}

fn answer(res: Result<ureq::Response, ureq::Error>) -> Result<Value, ApiError> {
    match res {
        Ok(r) => {
            let text = r.into_string().unwrap_or_default();
            if text.trim().is_empty() {
                return Ok(Value::Null);
            }
            serde_json::from_str(&text).map_err(|e| ApiError {
                status: Some(200),
                code: None,
                message: e.to_string(),
                body: Value::Null,
            })
        }
        Err(ureq::Error::Status(status, r)) => {
            let body: Value = r.into_json().unwrap_or(Value::Null);
            let code = body
                .get("error")
                .or_else(|| body.get("reason"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let message = body
                .get("message")
                .and_then(Value::as_str)
                .map(str::to_string)
                .or_else(|| code.clone())
                .unwrap_or_else(|| format!("serve answered {status}"));
            Err(ApiError {
                status: Some(status),
                code,
                message,
                body,
            })
        }
        Err(e) => Err(ApiError {
            status: None,
            code: None,
            message: e.to_string(),
            body: Value::Null,
        }),
    }
}

/// `GET /api/v1{path}` (path includes any query string).
pub fn get(path: &str) -> Result<Value, ApiError> {
    answer(request("GET", path).call())
}

/// `GET` and read the answer into `T`.
pub fn get_as<T: DeserializeOwned>(path: &str) -> Result<T, ApiError> {
    let v = get(path)?;
    serde_json::from_value(v).map_err(|e| ApiError {
        status: Some(200),
        code: None,
        message: e.to_string(),
        body: Value::Null,
    })
}

/// `GET` raw bytes (attachments, the profile photo).
pub fn get_bytes(path: &str) -> Result<Vec<u8>, ApiError> {
    match request("GET", path).call() {
        Ok(r) => {
            let mut out = Vec::new();
            std::io::Read::read_to_end(&mut r.into_reader(), &mut out).map_err(|e| ApiError {
                status: Some(200),
                code: None,
                message: e.to_string(),
                body: Value::Null,
            })?;
            Ok(out)
        }
        Err(e) => Err(answer(Err(e)).err().unwrap_or(ApiError {
            status: None,
            code: None,
            message: "no answer".into(),
            body: Value::Null,
        })),
    }
}

/// `POST /api/v1{path}` with a JSON body.
pub fn post(path: &str, body: &Value) -> Result<Value, ApiError> {
    answer(
        request("POST", path)
            .set("content-type", "application/json")
            .send_json(body.clone()),
    )
}

/// `POST` raw bytes (attachments, the profile photo).
pub fn post_bytes(path: &str, bytes: &[u8], mime: &str) -> Result<Value, ApiError> {
    answer(
        request("POST", path)
            .set("content-type", mime)
            .send_bytes(bytes),
    )
}

/// Percent-encode one path segment or query value.
pub fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}
