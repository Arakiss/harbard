//! `GET|POST /api/v1/profile` and `GET|POST /api/v1/profile/photo`: the
//! operator's "Profile", status line and "Do not disturb"
//! (`harbard_core::profile`).
//!
//! The writes are the operator's desk, under the same in-process lock as
//! `workspace.json` and the same cross-process lock file, loopback-only and
//! never a satellite's. They touch `{home}/profile.json` and
//! `{home}/profile/` and nothing else.
//!
//! The photo arrives as the bytes the operator picked in the console's file
//! picker: serve never opens a path the browser names, so it reads no folder
//! outside the harbard home. PNG, JPEG or WebP by their first bytes, at most
//! 2 MB, never through a symlink.

use super::fleet_routes::ApiError;
use super::workspace_route::desk_guard;
use harbard_core::profile::{self, FieldError, PhotoError, Profile};
use harbard_core::{Store, Window, hold};
use serde_json::{Value, json};

/// A profile change: the 1,500-character context in four-byte characters,
/// escaped, with room for the rest.
pub const MAX_PROFILE_BYTES: usize = 16 * 1024;

/// What the console reads: the stored profile, what is in force now (an
/// expired status or Do not disturb is already gone) and the caps it enforces.
pub fn body(store: &Store, now: u64) -> Value {
    settle(store, now);
    view(&Profile::load(store.root()), now)
}

/// A timed state whose end has come is left at the first look after it
/// (`hold::settle_lapse`): these GETs, the desk's writes and the minute
/// tick. It is the one write a GET here may make. Time passing is the
/// event; the GET only notices it, and the digest it cuts is the same one
/// the tick would have cut a minute later. The log is read only when a cut
/// is due.
pub fn settle(store: &Store, now: u64) {
    let root = store.root();
    let p = Profile::load(root);
    if !hold::lapse_due(root, &p, now) {
        return;
    }
    let _guard = desk_guard();
    let events = store.read(Window::All).events;
    let _ = hold::settle_lapse(root, &events, &Profile::load(root), now);
}

/// `GET /api/v1/profile/brief`: the profile's split for a Workshop device.
///
/// Exactly what an agent on this Mac reads: the SessionStart block
/// ([`Profile::brief_block`]: state and its rules, Do not disturb, the message,
/// hours, how to address the operator and the context) and the per-prompt
/// state line ([`Profile::prompt_block`]), both already quoted and bounded,
/// and the state's id. No raw field, no setting, no photo: everything else
/// of the profile stays Operator's (`GET /api/v1/profile`).
pub fn brief(store: &Store, now: u64) -> Value {
    settle(store, now);
    let p = Profile::load(store.root());
    let desk = harbard_core::workspace::DeskState::load(store);
    let name = &desk.config.operator_name;
    json!({
        "block": p.brief_block(name, now),
        "prompt": p.prompt_block(name, now),
        "state": p.state_at(now),
        "now_ms": now,
    })
}

fn view(p: &Profile, now: u64) -> Value {
    let quiet = p.quiet_at(now);
    json!({
        "profile": p,
        "status": p.status_at(now),
        "quiet": quiet,
        "quiet_until_ms": if quiet { p.dnd.as_ref().and_then(|d| d.until_ms) } else { None },
        "state": p.state_at(now),
        "state_until_ms": p.state.as_ref().filter(|_| p.state_at(now) != "available").and_then(|s| s.until_ms),
        "dock": profile::dock_rules(p.state_at(now)),
        "now_ms": now,
        "limits": {
            "context": profile::CONTEXT_MAX_CHARS,
            "name": profile::NAME_MAX,
            "address": profile::ADDRESS_MAX,
            "status_text": profile::STATUS_TEXT_MAX,
            "photo_bytes": profile::PHOTO_MAX_BYTES,
        },
    })
}

fn field_refused(e: FieldError) -> ApiError {
    let field: &'static str = match e.field.as_str() {
        "name" => "name",
        "address" => "address",
        "time_zone" => "time_zone",
        "hours" => "hours",
        "context" => "context",
        "status" => "status",
        "dnd" => "dnd",
        "dnd_default" => "dnd_default",
        "state" => "state",
        "away_suggest" => "away_suggest",
        "utc_offset_min" => "utc_offset_min",
        "photo" => "photo",
        _ => "body",
    };
    ApiError::new(422, e.code, e.reason).with("field", json!(field))
}

fn photo_refused(e: PhotoError) -> ApiError {
    let status = match e.code {
        "too_large" => 413,
        "unsupported_type" => 415,
        "symlink" => 409,
        _ => 500,
    };
    ApiError::new(status, e.code, e.reason)
}

/// The detail (it names paths in the home) goes to serve's log, not to the browser.
fn store_failed(e: std::io::Error) -> ApiError {
    eprintln!(
        "{}",
        json!({ "component": "harbard-serve", "event": "profile_store_failed", "fields": { "error": e.to_string() } })
    );
    ApiError::new(
        500,
        "store_failed",
        "the profile could not be read or written",
    )
}

/// `POST /api/v1/profile`: a partial change. `"photo": null` forgets the
/// photo; every other key is a profile field. A change of coordination
/// state that leaves a holding state answers with the digest of what it held
/// (`harbard_core::hold`), read from the whole log: a Away can last days.
pub fn write(store: &Store, now: u64, patch: &Value) -> Result<Value, ApiError> {
    let Some(map) = patch.as_object() else {
        return Err(ApiError::new(
            422,
            "bad_request",
            "a profile change is a JSON object",
        ));
    };
    let mut fields = map.clone();
    let forget_photo = match fields.remove("photo") {
        None => false,
        Some(Value::Null) => true,
        Some(_) => {
            return Err(ApiError::new(
                422,
                "bad_request",
                "a photo is sent to /profile/photo; here it can only be null",
            )
            .with("field", json!("photo")));
        }
    };
    let root = store.root();
    let changes_state = fields.contains_key("state");
    settle(store, now);
    let _guard = desk_guard();
    let before = Profile::load(root);
    let next = Profile::update(root, now, |p| {
        p.apply(&Value::Object(fields), now)?;
        if forget_photo {
            profile::remove_photo(root);
            p.photo = None;
            p.photo_rev = now;
        }
        Ok::<Profile, FieldError>(p.clone())
    })
    .map_err(store_failed)?
    .map_err(field_refused)?;
    let out = finish(store, root, &before, &next, changes_state, now)?;
    drop(_guard);
    // "Was it you?": the context reaches every session's brief.
    // After the desk guard is released: `with_desk` takes it again.
    if next.context != before.context {
        let check = harbard_core::workspace::DeskCheck {
            id: format!("context-{}", ulid::Ulid::generate()),
            what: "context".into(),
            text: next.context.clone().unwrap_or_default(),
            previous: before.context.clone(),
            since_ms: now,
            ..Default::default()
        };
        super::workspace_route::with_desk(store, |d| d.add_check(check))?;
    }
    Ok(out)
}

/// The answer to a profile write, with the digest a state change cut.
fn finish(
    store: &Store,
    root: &std::path::Path,
    before: &Profile,
    next: &Profile,
    changes_state: bool,
    now: u64,
) -> Result<Value, ApiError> {
    let mut out = view(next, now);
    if changes_state {
        let events = store.read(Window::All).events;
        let digest =
            hold::on_state_change(root, &events, before, next, now).map_err(store_failed)?;
        if let Some(o) = out.as_object_mut() {
            o.insert("digest".into(), json!(digest));
        }
    }
    Ok(out)
}

/// `GET /api/v1/digests`: what the states held, newest last.
pub fn digests(store: &Store, now: u64) -> Value {
    settle(store, now);
    json!(hold::Digests::load(store.root()))
}

/// `GET /api/v1/digests/pending`: what the current state holds so far, and
/// since when. Nothing is cut or marked.
pub fn pending(store: &Store, now: u64) -> Value {
    settle(store, now);
    let p = Profile::load(store.root());
    let events = store.read(Window::All).events;
    json!({
        "state": p.state_at(now),
        "since_ms": p.state.as_ref().filter(|_| p.state_at(now) != "available").map(|s| s.since_ms),
        "items": hold::pending(store.root(), &events, &p, now),
    })
}

/// `POST /api/v1/digests/read`: `{ "id": … }`.
pub fn read_digest(store: &Store, now: u64, body: &Value) -> Result<Value, ApiError> {
    settle(store, now);
    let id = body
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| ApiError::bad_request("id", "field `id` is required"))?;
    let _guard = desk_guard();
    if !hold::mark_read(store.root(), id).map_err(store_failed)? {
        return Err(ApiError::new(404, "unknown_item", "no such digest"));
    }
    Ok(json!({ "ok": true, "id": id }))
}

/// Once a minute: "every 60 min if they ask for it", and the end of a timed
/// state. The log is read only when a digest is due.
pub fn start_hourly(store: Store, shutdown: std::sync::Arc<std::sync::atomic::AtomicBool>) {
    std::thread::spawn(move || {
        while !shutdown.load(std::sync::atomic::Ordering::Relaxed) {
            let now = harbard_core::clock::now_ms();
            let p = Profile::load(store.root());
            if hold::hourly_due(store.root(), &p, now) || hold::lapse_due(store.root(), &p, now) {
                let _guard = desk_guard();
                let events = store.read(Window::All).events;
                let _ = hold::hourly(store.root(), &events, &p, now);
            }
            std::thread::sleep(std::time::Duration::from_secs(60));
        }
    });
}

/// `POST /api/v1/profile/photo`: the picked image's bytes.
pub fn write_photo(store: &Store, now: u64, bytes: &[u8]) -> Result<Value, ApiError> {
    let root = store.root();
    let _guard = desk_guard();
    let next = Profile::update(root, now, |p| {
        let name = profile::store_photo(root, bytes)?;
        p.photo = Some(name.to_string());
        p.photo_rev = now;
        Ok::<Profile, PhotoError>(p.clone())
    })
    .map_err(store_failed)?
    .map_err(photo_refused)?;
    Ok(view(&next, now))
}

/// `GET /api/v1/profile/photo`: the bytes and their type, or nothing.
pub fn photo(store: &Store) -> Option<(Vec<u8>, &'static str)> {
    let p = Profile::load(store.root());
    profile::read_photo(store.root(), p.photo.as_deref()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_store() -> (Store, std::path::PathBuf) {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "harbard-profile-route-{}-{}-{}",
            std::process::id(),
            harbard_core::clock::now_ms(),
            N.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (Store::at(&dir), dir)
    }

    /// Done for the day ends at their next working hours on its own.
    /// The first look after the end delivers what it queued, once.
    #[test]
    fn the_first_look_after_a_timed_state_ends_delivers_what_it_held() {
        let (store, dir) = temp_store();
        let now = harbard_core::clock::now_ms();
        write(
            &store,
            now - 120_000,
            &json!({"state": {"id": "done", "until_ms": now - 1_000}}),
        )
        .unwrap();
        let note: harbard_core::event::Event = serde_json::from_value(json!({
            "v": 1,
            "id": ulid::Ulid::from_parts(now - 60_000, 1).to_string(),
            "ts": harbard_core::clock::rfc3339_utc(now - 60_000),
            "event": "note",
            "session": "s-agent",
            "harness": "claude-code",
            "host": "mac",
            "cwd": "/w",
            "text": "PR 12 is still pending",
            "to": {"kind": "harness", "value": "human"},
            "kind": "fyi",
        }))
        .unwrap();
        store.append(&note).unwrap();
        assert_eq!(body(&store, now)["state"], "available");
        let all = digests(&store, now);
        let items = all["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "{all}");
        assert_eq!(items[0]["state"], "done");
        assert_eq!(items[0]["reason"], "left");
        assert_eq!(items[0]["items"][0]["text"], "PR 12 is still pending");
        assert_eq!(
            digests(&store, now + 1)["items"].as_array().unwrap().len(),
            1
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The route writes the profile, its folder and the desk's check, and nothing else in the
    /// home: no other file appears beside them.
    #[test]
    fn the_writes_touch_only_the_profile_file_its_folder_and_the_desk_lock() {
        let (store, dir) = temp_store();
        let before: std::collections::BTreeSet<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        write(&store, 1, &json!({"context": "x"})).unwrap();
        write_photo(&store, 2, b"\x89PNG\r\n\x1a\n....").unwrap();
        let after: std::collections::BTreeSet<_> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        let new: Vec<_> = after
            .iter()
            .filter(|n| !before.contains(std::ffi::OsStr::new(n.as_str())))
            .cloned()
            .collect();
        // The desk too: a new context asks "Was it you?" there.
        assert_eq!(
            new,
            vec![
                "profile",
                "profile.json",
                "workspace.json",
                "workspace.json.lock"
            ]
        );
        let desk = harbard_core::workspace::DeskState::load(&store);
        assert_eq!(desk.checks.len(), 1);
        assert_eq!(
            (desk.checks[0].what.as_str(), desk.checks[0].text.as_str()),
            ("context", "x")
        );
        assert_eq!(desk.checks[0].previous, None);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
