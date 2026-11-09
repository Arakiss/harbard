//! Contract tests for the fleet-view routes (SPEC-FLEET-VIEW).
//! Each spawns the real binary on an ephemeral loopback port, seeds a
//! real store through the CLI, and speaks raw HTTP — the same surface the
//! console consumes.
//!
//! The `/fleet` body is checked against the committed JSON Schema by walking
//! the schema itself (required keys, types, enums, `additionalProperties`)
//! rather than pulling a validator crate into the dependency graph. A field
//! added to the projection without a schema entry fails here, which is the
//! point of the row.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;

static SEQ: AtomicU64 = AtomicU64::new(0);
const TEST_TOKEN: &str = "harbard-fleet-test-token";

fn tmp_home() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("harbard-fleet-it-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp home");
    dir
}

fn cli(home: &Path, args: &[&str]) {
    let status = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .arg("--home")
        .arg(home)
        .args(args)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("spawn harbard");
    assert!(status.success(), "cli {args:?} failed");
}

struct Serve {
    child: Child,
    port: u16,
    home: PathBuf,
}

impl Serve {
    fn start(home: PathBuf) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .arg("--home")
            .arg(&home)
            .args(["serve", "--port", "0", "--token", TEST_TOKEN])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn serve");
        let stdout = child.stdout.take().expect("piped stdout");
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        reader.read_line(&mut line).expect("read startup line");
        let value: Value = serde_json::from_str(line.trim()).expect("startup json");
        let port = value["port"].as_u64().expect("port") as u16;
        Serve { child, port, home }
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

struct Resp {
    status: u16,
    body: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|error| panic!("body is not JSON ({error}): {}", self.body))
    }
}

fn request(port: u16, method: &str, path: &str, extra: &[(&str, &str)]) -> Resp {
    let mut sock = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    sock.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    req.push_str(&format!("Authorization: Bearer {TEST_TOKEN}\r\n"));
    for (key, value) in extra {
        req.push_str(&format!("{key}: {value}\r\n"));
    }
    req.push_str("\r\n");
    sock.write_all(req.as_bytes()).expect("write request");
    let mut raw = String::new();
    sock.read_to_string(&mut raw).expect("read response");
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .expect("status code");
    Resp {
        status,
        body: body.to_string(),
    }
}

fn get(port: u16, path: &str) -> Resp {
    request(port, "GET", path, &[])
}

fn post(port: u16, path: &str, body: &str, extra: &[(&str, &str)]) -> Resp {
    let mut sock = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    sock.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
    let mut req = format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    req.push_str(&format!("Authorization: Bearer {TEST_TOKEN}\r\n"));
    req.push_str("Content-Type: application/json\r\n");
    req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    for (key, value) in extra {
        req.push_str(&format!("{key}: {value}\r\n"));
    }
    req.push_str("\r\n");
    req.push_str(body);
    sock.write_all(req.as_bytes()).expect("write request");
    let mut raw = String::new();
    sock.read_to_string(&mut raw).expect("read response");
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok())
        .expect("status code");
    Resp {
        status,
        body: body.to_string(),
    }
}

/// Poll `/fleet` until the projection has folded in what the CLI just wrote.
fn fleet_with_sessions(port: u16, minimum: usize) -> Value {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let body = get(port, "/api/v1/fleet").json();
        if body["sessions"].as_array().map_or(0, Vec::len) >= minimum {
            return body;
        }
        assert!(
            Instant::now() < deadline,
            "the fleet never reached {minimum} sessions; last body: {body}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

// ---------------------------------------------------------------------------
// A small JSON Schema walker: enough of draft 2020-12 for this contract.
// ---------------------------------------------------------------------------

fn schema() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../spec/harbard/2026-09-06/fleet.schema.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_str(&raw).expect("the committed schema is valid JSON")
}

fn resolve<'a>(root: &'a Value, node: &'a Value) -> &'a Value {
    let Some(reference) = node.get("$ref").and_then(Value::as_str) else {
        return node;
    };
    let mut current = root;
    for segment in reference.trim_start_matches("#/").split('/') {
        current = current
            .get(segment)
            .unwrap_or_else(|| panic!("unresolved $ref segment {segment} in {reference}"));
    }
    current
}

fn type_matches(expected: &str, value: &Value) -> bool {
    match expected {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "boolean" => value.is_boolean(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "number" => value.is_number(),
        "null" => value.is_null(),
        other => panic!("unsupported schema type: {other}"),
    }
}

/// Validate `value` against `node`, collecting every failure with its path.
fn validate(root: &Value, node: &Value, value: &Value, path: &str, errors: &mut Vec<String>) {
    let node = resolve(root, node);

    if let Some(options) = node.get("oneOf").and_then(Value::as_array) {
        let matched = options.iter().any(|option| {
            let mut probe = Vec::new();
            validate(root, option, value, path, &mut probe);
            probe.is_empty()
        });
        if !matched {
            errors.push(format!("{path}: matches none of the oneOf branches"));
        }
        return;
    }

    match node.get("type") {
        Some(Value::String(expected)) => {
            if !type_matches(expected, value) {
                errors.push(format!("{path}: expected {expected}, got {value}"));
                return;
            }
        }
        Some(Value::Array(options)) => {
            let ok = options
                .iter()
                .filter_map(Value::as_str)
                .any(|expected| type_matches(expected, value));
            if !ok {
                errors.push(format!("{path}: expected one of {options:?}, got {value}"));
                return;
            }
        }
        _ => {}
    }

    if let Some(expected) = node.get("const")
        && value != expected
    {
        errors.push(format!("{path}: expected const {expected}, got {value}"));
    }
    if let Some(allowed) = node.get("enum").and_then(Value::as_array)
        && !allowed.contains(value)
    {
        errors.push(format!("{path}: {value} is not one of {allowed:?}"));
    }
    if let Some(minimum) = node.get("minimum").and_then(Value::as_i64)
        && value.as_i64().is_some_and(|actual| actual < minimum)
    {
        errors.push(format!("{path}: {value} is below the minimum {minimum}"));
    }
    if let Some(maximum) = node.get("maximum").and_then(Value::as_i64)
        && value.as_i64().is_some_and(|actual| actual > maximum)
    {
        errors.push(format!("{path}: {value} is above the maximum {maximum}"));
    }
    if let Some(min_len) = node.get("minLength").and_then(Value::as_u64)
        && value
            .as_str()
            .is_some_and(|text| (text.len() as u64) < min_len)
    {
        errors.push(format!("{path}: {value} is shorter than {min_len}"));
    }

    if let Some(object) = value.as_object() {
        for required in node
            .get("required")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
        {
            let key = required.as_str().unwrap_or_default();
            if !object.contains_key(key) {
                errors.push(format!("{path}: missing required key {key}"));
            }
        }
        let properties = node.get("properties").and_then(Value::as_object);
        for (key, child) in object {
            match properties.and_then(|properties| properties.get(key)) {
                Some(child_schema) => {
                    validate(root, child_schema, child, &format!("{path}.{key}"), errors)
                }
                None => {
                    if node.get("additionalProperties") == Some(&Value::Bool(false)) {
                        errors.push(format!("{path}.{key}: not in the schema"));
                    } else if let Some(additional) = node.get("additionalProperties") {
                        validate(root, additional, child, &format!("{path}.{key}"), errors);
                    }
                }
            }
        }
    }

    if let Some(array) = value.as_array() {
        if let Some(min_items) = node.get("minItems").and_then(Value::as_u64)
            && (array.len() as u64) < min_items
        {
            errors.push(format!("{path}: needs at least {min_items} items"));
        }
        if let Some(items) = node.get("items") {
            for (index, child) in array.iter().enumerate() {
                validate(root, items, child, &format!("{path}[{index}]"), errors);
            }
        }
    }
}

fn assert_valid(body: &Value) {
    let root = schema();
    let mut errors = Vec::new();
    validate(&root, &root, body, "fleet", &mut errors);
    assert!(errors.is_empty(), "schema violations: {errors:#?}");
}

/// V1 — the live body carries every §1 field with the documented types.
#[test]
fn v1_fleet_matches_the_committed_schema() {
    let home = tmp_home();
    cli(&home, &["hello", "--session", "one", "--harness", "codex"]);
    cli(
        &home,
        &["intent", "cutting the fleet view", "--session", "one"],
    );
    cli(
        &home,
        &["hello", "--session", "two", "--harness", "claude-code"],
    );
    let serve = Serve::start(home);

    let body = fleet_with_sessions(serve.port, 2);
    assert_valid(&body);

    assert_eq!(body["v"], 1);
    assert!(body["now_ms"].as_u64().unwrap() > 0);
    assert!(body["as_of"].as_str().is_some(), "the resume anchor is set");
    assert_eq!(body["stale_secs"], 900);
    for card in body["sessions"].as_array().unwrap() {
        assert_ne!(
            card["display_name"], card["session"],
            "no row renders as a raw id"
        );
        assert!(card["signal_age_ms"].as_u64().is_some());
    }
    assert!(body["counts"]["live"].as_u64().unwrap() >= 1);
}

/// V1 — a schema key the projection stopped serving is a contract break.
#[test]
fn v1_schema_and_projection_agree_on_the_key_set() {
    let home = tmp_home();
    cli(&home, &["hello", "--session", "one", "--harness", "codex"]);
    let serve = Serve::start(home);
    let body = fleet_with_sessions(serve.port, 1);

    let root = schema();
    let required: Vec<&str> = root["required"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    for key in required {
        assert!(body.get(key).is_some(), "the body dropped {key}");
    }
}

/// V6 (over the wire) — every item a place returns claims that place.
#[test]
fn v6_feed_returns_only_items_that_claim_the_place() {
    let home = tmp_home();
    cli(&home, &["hello", "--session", "one", "--harness", "codex"]);
    let serve = Serve::start(home);
    post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"the operator speaks"}"#,
        &[],
    );

    let all = get(serve.port, "/api/v1/feed?place=all&limit=50").json();
    assert_eq!(all["place"], "all");
    assert!(!all["items"].as_array().unwrap().is_empty());
    for item in all["items"].as_array().unwrap() {
        assert!(
            item["places"]
                .as_array()
                .unwrap()
                .contains(&Value::from("all"))
        );
        assert!(item["author"]["display_name"].as_str().is_some());
        assert!(item["event"]["id"].as_str().is_some());
    }

    let session = get(serve.port, "/api/v1/feed?place=session:one&limit=50").json();
    for item in session["items"].as_array().unwrap() {
        assert!(
            item["places"]
                .as_array()
                .unwrap()
                .contains(&Value::from("session:one")),
            "an item that does not claim the place was returned: {item}"
        );
    }
}

/// V10 — every refusal on the new routes carries a code and a sentence.
#[test]
fn v10_new_routes_refuse_with_error_and_message() {
    let home = tmp_home();
    cli(&home, &["hello", "--session", "one", "--harness", "codex"]);
    let serve = Serve::start(home);

    let cases = [
        ("/api/v1/feed?place=harness:codex", 400, "unknown_place"),
        ("/api/v1/feed?place=all&limit=9999", 400, "bad_request"),
        ("/api/v1/feed?place=all&limit=abc", 400, "bad_request"),
        (
            "/api/v1/feed?place=all&before=00000000000000000000000000",
            409,
            "cursor_expired",
        ),
    ];
    for (path, status, code) in cases {
        let resp = get(serve.port, path);
        assert_eq!(resp.status, status, "{path} -> {}", resp.body);
        let body = resp.json();
        assert_eq!(body["error"], code, "{path}");
        assert!(
            body["message"].as_str().is_some_and(|m| !m.is_empty()),
            "{path} has no human message: {body}"
        );
    }

    let bad_read = post(
        serve.port,
        "/api/v1/operator/reads",
        r#"{"event":"x"}"#,
        &[],
    );
    assert_eq!(bad_read.status, 400);
    assert_eq!(bad_read.json()["error"], "bad_request");
    assert_eq!(bad_read.json()["field"], "place");
}

/// V9 — the read cursor persists, `unread` recomputes, satellites are refused.
#[test]
fn v9_operator_reads_persist_and_satellites_are_refused() {
    let home = tmp_home();
    cli(&home, &["hello", "--session", "one", "--harness", "codex"]);
    let serve = Serve::start(home);
    post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"something to read"}"#,
        &[],
    );

    let feed = get(serve.port, "/api/v1/feed?place=all&limit=5").json();
    let newest = feed["items"][0]["event"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    let refused = post(
        serve.port,
        "/api/v1/operator/reads",
        &format!(r#"{{"place":"all","event":"{newest}"}}"#),
        &[("X-Harbard-Device", "satellite-1")],
    );
    assert_eq!(
        refused.status, 403,
        "a satellite cannot move the desk's cursor"
    );
    // The hub's own key presented for another device is the retired shared
    // secret (Pairing): refused before the desk's own door is reached.
    assert_eq!(refused.json()["error"], "shared_token_retired");
    assert!(refused.json()["message"].as_str().is_some());

    let accepted = post(
        serve.port,
        "/api/v1/operator/reads",
        &format!(r#"{{"place":"all","event":"{newest}"}}"#),
        &[],
    );
    assert_eq!(accepted.status, 200, "{}", accepted.body);
    assert_eq!(accepted.json()["reads"]["all"], newest.as_str());

    // It survives the request that wrote it: the projection reports it back.
    let body = get(serve.port, "/api/v1/fleet").json();
    assert_eq!(body["reads"]["all"], newest.as_str());
    assert_valid(&body);

    // And it is on disk, in the operator's own file, not in cursors/.
    let raw = std::fs::read_to_string(serve.home.join("operator.json")).expect("operator.json");
    assert!(raw.contains(&newest));
    assert!(
        !serve.home.join("cursors").join("all.json").exists(),
        "the operator's reading position is not an agent cursor"
    );
}

/// V7 — the stream pushes `item` and `fleet` after a note, and an unknown
/// resume id yields `reset` rather than silently starting from now.
#[test]
fn v7_stream_pushes_item_and_fleet_frames() {
    let home = tmp_home();
    cli(&home, &["hello", "--session", "one", "--harness", "codex"]);
    let serve = Serve::start(home);

    let mut sock = TcpStream::connect(("127.0.0.1", serve.port)).expect("connect stream");
    sock.set_read_timeout(Some(Duration::from_secs(6))).unwrap();
    let request = format!(
        "GET /api/v1/stream HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {TEST_TOKEN}\r\nAccept: text/event-stream\r\n\r\n"
    );
    sock.write_all(request.as_bytes()).unwrap();
    std::thread::sleep(Duration::from_millis(300));

    post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"live while streaming"}"#,
        &[],
    );

    let text = read_until(&mut sock, |seen| {
        seen.contains("event: item") && seen.contains("event: fleet")
    });
    assert!(
        text.contains("event: bus"),
        "legacy frame still shipped: {text}"
    );
    assert!(text.contains("event: item"), "no item frame: {text}");
    assert!(text.contains("event: fleet"), "no fleet frame: {text}");
    assert!(
        text.contains("live while streaming"),
        "the note never reached the stream: {text}"
    );
    drop(sock);

    // An id the hub never saw is a gap, and the client is told.
    let mut sock = TcpStream::connect(("127.0.0.1", serve.port)).expect("connect stream");
    sock.set_read_timeout(Some(Duration::from_secs(6))).unwrap();
    let request = format!(
        "GET /api/v1/stream HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {TEST_TOKEN}\r\nAccept: text/event-stream\r\nLast-Event-ID: 00000000000000000000000000\r\n\r\n"
    );
    sock.write_all(request.as_bytes()).unwrap();
    let text = read_until(&mut sock, |seen| seen.contains("event: reset"));
    assert!(text.contains("event: reset"), "no reset frame: {text}");
    assert!(
        text.contains("cursor_expired"),
        "reset has no reason: {text}"
    );
}

/// V7 — `Last-Event-ID` resumes after the id it names.
#[test]
fn v7_last_event_id_resumes_where_the_client_left_off() {
    let home = tmp_home();
    cli(&home, &["hello", "--session", "one", "--harness", "codex"]);
    let serve = Serve::start(home);
    post(serve.port, "/api/v1/notes", r#"{"text":"first word"}"#, &[]);
    std::thread::sleep(Duration::from_millis(400));
    post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"second word"}"#,
        &[],
    );
    std::thread::sleep(Duration::from_millis(400));

    let feed = get(serve.port, "/api/v1/feed?place=all&limit=10").json();
    let items = feed["items"].as_array().unwrap();
    let first = items
        .iter()
        .find(|item| item["event"]["text"] == "first word")
        .expect("the first note is on the feed");
    let first_id = first["event"]["id"].as_str().unwrap();

    let mut sock = TcpStream::connect(("127.0.0.1", serve.port)).expect("connect stream");
    sock.set_read_timeout(Some(Duration::from_secs(6))).unwrap();
    let request = format!(
        "GET /api/v1/stream HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {TEST_TOKEN}\r\nAccept: text/event-stream\r\nLast-Event-ID: {first_id}\r\n\r\n"
    );
    sock.write_all(request.as_bytes()).unwrap();
    let text = read_until(&mut sock, |seen| seen.contains("second word"));
    assert!(
        text.contains("second word"),
        "the replay did not resume: {text}"
    );
    assert!(
        !text.contains("event: reset"),
        "a known id is not a gap: {text}"
    );
    assert!(
        !text.contains("first word"),
        "resume is exclusive of the id it names: {text}"
    );
}

/// V7 — an `item` frame is computed against the hub's window, not the one
/// event that triggered it. A one-event slice could not see the thread it
/// replies to, the places its siblings reach, or its author's presence.
#[test]
fn v7_item_frames_carry_the_same_context_the_feed_does() {
    let home = tmp_home();
    cli(&home, &["hello", "--session", "one", "--harness", "codex"]);
    let serve = Serve::start(home);

    // A note that owes an answer, so the reply has a thread to belong to.
    let root = post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"the question","kind":"ask","to":{"kind":"harness","value":"codex"}}"#,
        &[],
    )
    .json();
    let root_id = root["id"]
        .as_str()
        .or_else(|| root["event"]["id"].as_str())
        .expect("the root note has an id")
        .to_string();
    std::thread::sleep(Duration::from_millis(300));

    let mut sock = TcpStream::connect(("127.0.0.1", serve.port)).expect("connect stream");
    sock.set_read_timeout(Some(Duration::from_secs(6))).unwrap();
    let request = format!(
        "GET /api/v1/stream HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {TEST_TOKEN}\r\nAccept: text/event-stream\r\n\r\n"
    );
    sock.write_all(request.as_bytes()).unwrap();
    std::thread::sleep(Duration::from_millis(300));

    // A first reply, before the stream frame under test: the second reply's
    // envelope can only count it by looking past its own event.
    let first = format!(r#"{{"text":"an earlier answer","ref":"{root_id}"}}"#);
    let first = post(serve.port, "/api/v1/notes", &first, &[]);
    assert_eq!(
        first.status, 201,
        "the first reply was refused: {}",
        first.body
    );
    std::thread::sleep(Duration::from_millis(300));

    let body = format!(r#"{{"text":"the answer","ref":"{root_id}"}}"#);
    let reply = post(serve.port, "/api/v1/notes", &body, &[]);
    assert_eq!(reply.status, 201, "the reply was refused: {}", reply.body);

    // The reply's own frame, not the replay of the note it answers.
    // "the answer" shows up on the `bus` frame first; wait for the `item` one.
    let text = read_until(&mut sock, |seen| {
        seen.split("event: item\n")
            .skip(1)
            .filter_map(|rest| rest.lines().find(|line| line.starts_with("data: ")))
            .any(|line| line.contains("the answer"))
    });
    let item = text
        .split("event: item\n")
        .skip(1)
        .filter_map(|rest| rest.lines().find(|line| line.starts_with("data: ")))
        .filter_map(|line| serde_json::from_str::<Value>(line.trim_start_matches("data: ")).ok())
        .find(|frame| frame["event"]["text"] == "the answer")
        .unwrap_or_else(|| panic!("no item frame for the reply: {text}"));
    drop(sock);

    let thread = item["thread"].as_object().expect("the reply has a thread");
    assert!(
        thread["replies"].as_u64().unwrap_or(0) >= 2,
        "the frame counted only its own event, not the thread: {item}"
    );

    // The same envelope the feed builds for that id.
    let reply_id = item["event"]["id"].as_str().expect("the reply has an id");
    let feed = get(serve.port, "/api/v1/feed?place=all&limit=50").json();
    let from_feed = feed["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["event"]["id"] == reply_id)
        .expect("the reply is on the feed")
        .clone();
    assert_eq!(
        item["places"], from_feed["places"],
        "the stream and the feed disagree on where an item belongs"
    );
    assert_eq!(
        item["author"]["presence"], from_feed["author"]["presence"],
        "the stream saw a different author presence than the feed"
    );
}

/// Read from an open SSE socket until `done` or the deadline.
fn read_until(sock: &mut TcpStream, done: impl Fn(&str) -> bool) -> String {
    let deadline = Instant::now() + Duration::from_secs(6);
    let mut acc = Vec::new();
    let mut buf = [0u8; 4096];
    while Instant::now() < deadline {
        match sock.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                acc.extend_from_slice(&buf[..n]);
                let seen = String::from_utf8_lossy(&acc);
                if done(&seen) {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    String::from_utf8_lossy(&acc).to_string()
}

/// V12 — the CLI prints the same projection the route serves (every product surface has a machine form, and they agree).
#[test]
fn v12_cli_fleet_matches_the_served_projection() {
    let home = tmp_home();
    cli(&home, &["hello", "--session", "one", "--harness", "codex"]);
    cli(
        &home,
        &[
            "intent",
            "holding the rudder",
            "--session",
            "one",
            "--harness",
            "codex",
        ],
    );

    let output = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .arg("--home")
        .arg(&home)
        .args(["fleet", "--json"])
        .output()
        .expect("run harbard fleet --json");
    assert!(output.status.success(), "fleet --json failed");
    let from_cli: Value =
        serde_json::from_slice(&output.stdout).expect("fleet --json emits one object");
    assert_valid(&from_cli);

    let serve = Serve::start(home);
    let from_route = fleet_with_sessions(serve.port, 1);

    // Everything a clock moves differs by construction; the board does not.
    for key in [
        "v",
        "stale_secs",
        "sessions",
        "checkouts",
        "rooms",
        "counts",
    ] {
        let cli_side = strip_clock(&from_cli[key]);
        let route_side = strip_clock(&from_route[key]);
        assert_eq!(cli_side, route_side, "{key} differs between CLI and route");
    }

    // And the human form is readable, not a dump.
    let human = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .arg("--home")
        .arg(&serve.home)
        .args(["fleet"])
        .output()
        .expect("run harbard fleet");
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("1 live"), "no count line: {text}");
    assert!(text.contains("holding the rudder"), "no intent: {text}");
    assert!(
        !text.contains("\"session\""),
        "the plain form is not JSON: {text}"
    );
}

/// Drop every field a clock moves, so two reads taken milliseconds apart can be
/// compared for the thing that actually matters: the shape and the state.
fn strip_clock(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .filter(|(key, _)| {
                    !matches!(
                        key.as_str(),
                        "now_ms" | "signal_age_ms" | "last_signal_ms" | "unread" | "unread_all"
                    )
                })
                .map(|(key, child)| (key.clone(), strip_clock(child)))
                .collect(),
        ),
        Value::Array(array) => Value::Array(array.iter().map(strip_clock).collect()),
        other => other.clone(),
    }
}
