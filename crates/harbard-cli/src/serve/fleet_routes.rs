//! The feed routes (SPEC-FLEET-VIEW): `/feed`, the stream's `item` frames
//! and the operator's read cursors.
//!
//! These live apart from `serve.rs` on purpose. The router there answers
//! fifteen legacy shapes and is long enough already; these routes are one
//! contract with one error envelope, and nothing in this file may change how
//! an existing response looks.

use harbard_core::feed::{self, FeedContext, Place};
use harbard_core::query;
use harbard_core::store::HistoryError;
use harbard_core::{Event, OperatorProfile, Store, clock};
use serde_json::{Value, json};
use std::sync::Mutex;

static OPERATOR_READS_UPDATE: Mutex<()> = Mutex::new(());

/// A refusal on the new routes: `{error, message, …detail}` (§5). Legacy routes
/// keep their bare `{error: "<sentence>"}` strings — a client that learned to
/// read those must not start seeing a code where a sentence used to be.
#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub code: &'static str,
    pub message: String,
    pub detail: Vec<(&'static str, Value)>,
}

impl ApiError {
    pub fn new(status: u16, code: &'static str, message: impl Into<String>) -> Self {
        ApiError {
            status,
            code,
            message: message.into(),
            detail: Vec::new(),
        }
    }

    pub fn with(mut self, key: &'static str, value: Value) -> Self {
        self.detail.push((key, value));
        self
    }

    pub fn bad_request(field: &'static str, message: impl Into<String>) -> Self {
        ApiError::new(400, "bad_request", message).with("field", json!(field))
    }

    pub fn body(&self) -> Value {
        let mut body = json!({ "error": self.code, "message": self.message });
        if let Some(object) = body.as_object_mut() {
            for (key, value) in &self.detail {
                object.insert((*key).to_string(), value.clone());
            }
        }
        body
    }
}

/// `GET /api/v1/feed?place=&before=&limit=`.
///
/// The page itself always comes from the store. Its envelopes (authors,
/// places, answers) come from the hub's window while the page stays inside
/// it; a page that reaches past `horizon_ms` is rendered again against the
/// whole store, so an old page reads exactly as it did before the window.
pub fn feed_body(
    store: &Store,
    events: &[Event],
    horizon_ms: u64,
    stale_secs: u64,
    place: Option<String>,
    before: Option<String>,
    limit: Option<String>,
) -> Result<Value, ApiError> {
    let raw_place = place.unwrap_or_else(|| "all".to_string());
    let place = Place::parse(&raw_place).map_err(|message| {
        ApiError::new(400, "unknown_place", message).with("place", json!(raw_place))
    })?;

    let limit = match limit {
        None => feed::DEFAULT_PAGE,
        Some(raw) => match raw.parse::<usize>() {
            Ok(value) if (1..=feed::MAX_PAGE).contains(&value) => value,
            Ok(_) => {
                return Err(ApiError::bad_request(
                    "limit",
                    format!("limit must be between 1 and {}", feed::MAX_PAGE),
                ));
            }
            Err(_) => return Err(ApiError::bad_request("limit", "limit must be a number")),
        },
    };

    let past_horizon = |id: Option<&str>| id.and_then(id_ms).is_some_and(|ms| ms < horizon_ms);
    let mut paged = None;
    if !past_horizon(before.as_deref()) {
        let context = context_for(store, events, stale_secs);
        let page = feed::page(store, &context, &place, before.as_deref(), limit);
        if !page
            .as_ref()
            .is_ok_and(|page| past_horizon(page.oldest.as_deref()))
        {
            paged = Some(page);
        }
    }
    let paged = paged.unwrap_or_else(|| {
        let all = store.read(harbard_core::Window::All);
        let context = context_for(store, &all.events, stale_secs);
        feed::page(store, &context, &place, before.as_deref(), limit)
    });
    match paged {
        Ok(page) => Ok(serde_json::to_value(page).unwrap_or_else(|_| json!({}))),
        Err(HistoryError::CursorExpired { kind, id }) => Err(ApiError::new(
            409,
            "cursor_expired",
            format!("the {kind} cursor is no longer in the store; refetch from the newest page"),
        )
        .with("cursor", json!(id))),
    }
}

/// The millisecond an event id was minted at, when it is a ULID.
fn id_ms(id: &str) -> Option<u64> {
    id.parse::<ulid::Ulid>().ok().map(|id| id.timestamp_ms())
}

/// The envelope context, built from the same window the projection folds.
pub fn context_for(store: &Store, events: &[Event], stale_secs: u64) -> FeedContext {
    let views = query::sessions(store, events, clock::now_ms(), stale_secs);
    FeedContext::build(store, &views, events)
}

/// One item envelope, for the `item` stream frame.
pub fn item_value(context: &FeedContext, event: &Event) -> Value {
    serde_json::to_value(context.item(event)).unwrap_or_else(|_| json!({}))
}

/// `POST /api/v1/operator/reads` — where the operator has read up to.
///
/// The operator's own state, never an agent's: this writes `operator.json` and
/// never touches `cursors/`, which belongs to the sessions themselves.
pub fn post_reads(store: &Store, body: &str) -> Result<Value, ApiError> {
    let parsed: Value = serde_json::from_str(body)
        .map_err(|error| ApiError::bad_request("body", format!("invalid JSON: {error}")))?;

    let place = parsed
        .get("place")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("place", "place is required"))?;
    // Refuse a place nobody can name rather than storing a cursor for a screen
    // that does not exist.
    let place = Place::parse(place)
        .map_err(|message| ApiError::new(400, "unknown_place", message))?
        .id();

    let event = parsed
        .get("event")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::bad_request("event", "event is required"))?;
    if !harbard_core::is_event_id(event) {
        return Err(ApiError::bad_request(
            "event",
            "event must be a full event id (26 characters)",
        ));
    }

    // Two windows may post together. Keep comparison and persistence in one
    // update so a request loaded before a newer read cannot overwrite it.
    let _update = OPERATOR_READS_UPDATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut profile = OperatorProfile::load(store);
    let advances = profile.reads.get(&place).is_none_or(|previous| {
        if previous.eq_ignore_ascii_case(event) {
            return false;
        }
        let retained = store.read(harbard_core::Window::All);
        let observed = |id: &str| {
            retained
                .events
                .iter()
                .any(|entry| entry.id.eq_ignore_ascii_case(id))
        };
        if observed(previous) && observed(event) {
            harbard_core::wake::read_index(&retained.events, &[Some(previous)])
                < harbard_core::wake::read_index(&retained.events, &[Some(event)])
        } else {
            // An absent mark has no append position. Only an earlier timestamp
            // proves progress; random suffixes at the same instant prove none.
            previous.parse::<ulid::Ulid>().is_ok()
                && event.parse::<ulid::Ulid>().is_ok()
                && harbard_core::wake::at_or_before(previous, Some(event))
        }
    });
    if advances {
        profile.reads.insert(place, event.to_string());
    }
    // The place and the id were both accepted; what failed is the disk. Saying
    // `not_found` here sends the console looking for a missing place that is
    // not missing, and hides the one thing an operator can act on.
    profile.save(store).map_err(|error| {
        ApiError::new(
            500,
            "store_unwritable",
            format!("could not save the operator profile: {error}"),
        )
    })?;
    Ok(json!({ "reads": profile.reads }))
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ReadHome(std::path::PathBuf);

    impl ReadHome {
        fn new() -> Self {
            let path = std::env::temp_dir()
                .join(format!("harbard-operator-reads-{}", ulid::Ulid::generate()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn store(&self) -> Store {
            Store::at(&self.0)
        }
    }

    impl Drop for ReadHome {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn append_read_note(store: &Store, id: &str) {
        let event: Event = serde_json::from_value(json!({
            "v": 1,
            "id": id,
            "ts": "2026-01-01T00:00:00Z",
            "event": "note",
            "session": "test-reader",
            "harness": "codex",
            "host": "test",
            "cwd": "/test/project",
            "text": "A synthetic warning to read.",
            "kind": "warning",
        }))
        .unwrap();
        store.append(&event).unwrap();
    }

    fn read_all(store: &Store, id: &str) -> Value {
        post_reads(store, &json!({ "place": "all", "event": id }).to_string())
            .expect("a valid operator read request succeeds")
    }

    #[test]
    fn operator_reads_follow_retained_append_order_without_regression() {
        let home = ReadHome::new();
        let store = home.store();
        let earlier = format!("01M1VTHYX6{}", "Z".repeat(16));
        let later = format!("01M1VTHYX6{}", "0".repeat(16));
        assert!(earlier > later, "ULID suffixes oppose append order");
        append_read_note(&store, &earlier);
        append_read_note(&store, &later);
        let segment = store.segments().into_iter().next().unwrap();
        let original_log = std::fs::read(&segment).unwrap();

        assert_eq!(read_all(&store, &earlier)["reads"]["all"], earlier);
        assert_eq!(read_all(&store, &later)["reads"]["all"], later);
        assert_eq!(
            read_all(&store, &earlier)["reads"]["all"],
            later,
            "a delayed older read cannot regress the shared cursor"
        );
        assert_eq!(read_all(&store, &later)["reads"]["all"], later);
        assert_eq!(OperatorProfile::load(&store).reads["all"], later);
        assert_eq!(std::fs::read(segment).unwrap(), original_log);
        assert!(!store.cursors_dir().exists(), "agent delivery is untouched");
    }

    #[test]
    fn operator_reads_preserve_an_unobserved_prior_cursor_until_order_is_proven() {
        let home = ReadHome::new();
        let store = home.store();
        let prior = format!("01M1VTHYX6{}", "0".repeat(16));
        let same_time = format!("01M1VTHYX6{}", "Z".repeat(16));
        let later_time = format!("01M1VTHYX7{}", "0".repeat(16));
        append_read_note(&store, &same_time);
        append_read_note(&store, &later_time);

        assert_eq!(read_all(&store, &prior)["reads"]["all"], prior);
        assert_eq!(
            read_all(&store, &same_time)["reads"]["all"],
            prior,
            "an absent cursor's random suffix cannot establish ordering"
        );
        assert_eq!(read_all(&store, &later_time)["reads"]["all"], later_time);
        assert_eq!(OperatorProfile::load(&store).reads["all"], later_time);
    }

    #[test]
    fn operator_reads_from_concurrent_windows_keep_the_furthest_cursor() {
        let home = ReadHome::new();
        let store = home.store();
        let earlier = format!("01M1VTHYX6{}", "Z".repeat(16));
        let later = format!("01M1VTHYX6{}", "0".repeat(16));
        append_read_note(&store, &earlier);
        append_read_note(&store, &later);
        let ready = std::sync::Barrier::new(2);

        std::thread::scope(|scope| {
            let first = scope.spawn(|| {
                ready.wait();
                read_all(&store, &earlier);
            });
            let second = scope.spawn(|| {
                ready.wait();
                read_all(&store, &later);
            });
            first.join().unwrap();
            second.join().unwrap();
        });

        assert_eq!(OperatorProfile::load(&store).reads["all"], later);
    }

    #[test]
    fn errors_carry_a_code_and_a_sentence() {
        let error = ApiError::bad_request("limit", "limit must be a number");
        let body = error.body();
        assert_eq!(body["error"], "bad_request");
        assert_eq!(body["field"], "limit");
        assert!(body["message"].as_str().is_some_and(|m| !m.is_empty()));
    }

    /// §5: a disk that refuses the write says so. Reporting `not_found` for a
    /// place that was just validated sends the console hunting something that
    /// is not missing, and hides the only thing the operator can act on.
    #[test]
    fn an_unwritable_store_is_not_a_missing_place() {
        let home = std::env::temp_dir().join(format!(
            "harbard-reads-unwritable-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let store = Store::at(&home);

        // save() writes operator.json.tmp then renames it. A directory in that
        // exact spot cannot be written over, so the save fails on the disk.
        std::fs::create_dir_all(home.join("operator.json.tmp")).unwrap();

        let refused = post_reads(
            &store,
            r#"{"place":"all","event":"01M1VTHYX6D1000000000000AB"}"#,
        )
        .expect_err("an unwritable store cannot store a cursor");
        assert_eq!(refused.code, "store_unwritable");
        assert_eq!(refused.status, 500);
        assert!(
            refused.body()["message"]
                .as_str()
                .is_some_and(|m| !m.is_empty()),
            "every error body carries a sentence"
        );

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn reads_refuse_a_place_or_an_id_they_cannot_trust() {
        let home = std::env::temp_dir().join(format!("harbard-reads-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        let store = Store::at(&home);

        let bad_place = post_reads(
            &store,
            r#"{"place":"nowhere","event":"01M1VTHYX6D1000000000000AB"}"#,
        );
        assert_eq!(bad_place.err().map(|e| e.code), Some("unknown_place"));

        let bad_id = post_reads(&store, r#"{"place":"all","event":"01M1"}"#);
        assert_eq!(bad_id.err().map(|e| e.code), Some("bad_request"));

        let ok = post_reads(
            &store,
            r#"{"place":"all","event":"01M1VTHYX6D1000000000000AB"}"#,
        )
        .expect("a full id on a real place is stored");
        assert_eq!(ok["reads"]["all"], "01M1VTHYX6D1000000000000AB");
        let _ = std::fs::remove_dir_all(&home);
    }
}
