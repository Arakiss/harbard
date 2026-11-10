//! Contract tests for `harbard serve` (docs/SPEC-API.md). Each spawns the
//! real binary bound to an ephemeral 127.0.0.1 port, seeds a real store through
//! the CLI, and speaks raw HTTP/1.1 over a socket — no HTTP client dependency,
//! and the same surface the console consumes.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static SEQ: AtomicU64 = AtomicU64::new(0);
const TEST_TOKEN: &str = "harbard-serve-test-token";

fn tmp_home() -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let d = std::env::temp_dir().join(format!("harbard-serve-it-{}-{}", std::process::id(), n));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("create temp home");
    d
}

/// Run a one-shot CLI verb against `home` (seeding the store).
fn cli(home: &PathBuf, args: &[&str]) {
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

/// A running `harbard serve` child, killed and cleaned on drop.
struct Serve {
    child: Child,
    port: u16,
    home: PathBuf,
}

impl Serve {
    fn start(home: PathBuf) -> Self {
        Self::start_with_args(home, &["--token", TEST_TOKEN])
    }

    fn start_with_generated_token(home: PathBuf) -> Self {
        Self::start_with_args(home, &[])
    }

    fn start_with_args(home: PathBuf, extra: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .arg("--home")
            .arg(&home)
            .args(["serve", "--port", "0"])
            .args(extra)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn serve");
        // The first stdout line is the discovery JSON: {port, pid, store}.
        let stdout = child.stdout.take().expect("piped stdout");
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        reader.read_line(&mut line).expect("read startup line");
        let v: serde_json::Value = serde_json::from_str(line.trim()).expect("startup json");
        let port = v["port"].as_u64().expect("port") as u16;
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

/// A parsed HTTP response.
struct Resp {
    status: u16,
    headers: String,
    body: String,
}

impl Resp {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.lines().find_map(|l| {
            let (k, v) = l.split_once(':')?;
            k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
        })
    }
    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).expect("json body")
    }
}

/// One request/response over a fresh connection (`Connection: close`).
fn request(port: u16, method: &str, path: &str, extra: &[(&str, &str)]) -> Resp {
    request_with_auth(port, method, path, extra, true)
}

fn request_with_auth(
    port: u16,
    method: &str,
    path: &str,
    extra: &[(&str, &str)],
    authenticated: bool,
) -> Resp {
    let mut sock = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    sock.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
    let own_host = extra.iter().any(|(k, _)| k.eq_ignore_ascii_case("host"));
    let mut req = if own_host {
        format!("{method} {path} HTTP/1.1\r\nConnection: close\r\n")
    } else {
        format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n")
    };
    if authenticated {
        req.push_str(&format!("Authorization: Bearer {TEST_TOKEN}\r\n"));
    }
    for (k, v) in extra {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    req.push_str("\r\n");
    sock.write_all(req.as_bytes()).expect("write request");

    let mut raw = String::new();
    sock.read_to_string(&mut raw).expect("read response");
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .expect("status code");
    Resp {
        status,
        headers: head.to_string(),
        body: body.to_string(),
    }
}

#[test]
fn loopback_serve_generates_and_requires_a_private_token() {
    let home = tmp_home();
    let serve = Serve::start_with_generated_token(home);
    let token = std::fs::read_to_string(serve.home.join("hub.token"))
        .unwrap()
        .trim()
        .to_string();
    assert!(token.len() >= 40);
    let permissions = std::fs::metadata(serve.home.join("hub.token")).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(permissions.permissions().mode() & 0o777, 0o600);
    }

    let unauthorized = request_with_auth(serve.port, "GET", "/api/v1/meta", &[], false);
    assert_eq!(unauthorized.status, 401);
    let authorization = format!("Bearer {token}");
    let authorized = request_with_auth(
        serve.port,
        "GET",
        "/api/v1/meta",
        &[("Authorization", authorization.as_str())],
        false,
    );
    assert_eq!(authorized.status, 200);
}

fn get(port: u16, path: &str) -> Resp {
    request(port, "GET", path, &[])
}

fn events_with_minimum_len(port: u16, minimum: usize) -> serde_json::Value {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        let events = get(port, "/api/v1/events").json();
        let len = events["events"].as_array().map_or(0, Vec::len);
        if len >= minimum {
            return events;
        }
        assert!(
            Instant::now() < deadline,
            "events projection did not reach {minimum} rows; last response: {events}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// One request with a body (the F2 write door).
fn post(port: u16, path: &str, body: &str, extra: &[(&str, &str)]) -> Resp {
    let mut sock = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    sock.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
    let mut req = format!("POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    req.push_str(&format!("Authorization: Bearer {TEST_TOKEN}\r\n"));
    req.push_str("Content-Type: application/json\r\n");
    req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    for (k, v) in extra {
        req.push_str(&format!("{k}: {v}\r\n"));
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
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .expect("status code");
    Resp {
        status,
        headers: head.to_string(),
        body: body.to_string(),
    }
}

/// F2 — the operator's note is appended with an identity the client cannot
/// choose, and it reaches the read side through the ordinary events endpoint.
#[test]
fn f2_operator_note_is_authored_by_the_server() {
    let home = tmp_home();
    let serve = Serve::start(home);

    let resp = post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"do not touch dev","session":"codex-real","harness":"codex","role":"main"}"#,
        &[("Origin", "http://localhost:3419")],
    );
    assert_eq!(resp.status, 201, "body: {}", resp.body);
    let ev = &resp.json()["event"];
    assert_eq!(ev["event"], "note");
    assert_eq!(ev["text"], "do not touch dev");
    // Impersonation is impossible: the client's session/harness/role are ignored.
    assert_eq!(ev["harness"], "human");
    assert_eq!(ev["role"], "operator");
    assert!(
        ev["session"].as_str().unwrap().starts_with("operator@"),
        "session was {}",
        ev["session"]
    );

    // It is a first-class store event: the read side sees it with no special case.
    let events = events_with_minimum_len(serve.port, 1);
    let list = events["events"].as_array().expect("events array");
    assert_eq!(list.len(), 1);
    assert_eq!(list[0]["harness"], "human");
}

/// A reply carries `ref` to the parent note id; bad refs are refused.
#[test]
fn f2_note_ref_nests_under_parent_and_rejects_garbage() {
    let home = tmp_home();
    let serve = Serve::start(home);

    let parent = post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"parent note","kind":"fyi"}"#,
        &[],
    );
    assert_eq!(parent.status, 201, "body: {}", parent.body);
    let parent_id = parent.json()["event"]["id"].as_str().unwrap().to_string();

    let bad = post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"nope","ref":"not-a-ulid"}"#,
        &[],
    );
    assert_eq!(bad.status, 400, "body: {}", bad.body);

    let reply = post(
        serve.port,
        "/api/v1/notes",
        &format!(r#"{{"text":"taking it","ref":"{parent_id}"}}"#),
        &[],
    );
    assert_eq!(reply.status, 201, "body: {}", reply.body);
    assert_eq!(reply.json()["event"]["ref"], parent_id);

    let events = events_with_minimum_len(serve.port, 2);
    let list = events["events"].as_array().expect("events array");
    assert_eq!(list.len(), 2);
    assert_eq!(list[1]["ref"], parent_id);
}

/// Every HTTP write door uses the same exact-or-unique rules as the CLI. A
/// browser or native client must never receive 201 for a note addressed to
/// nobody, nor silently attach a reply to the first prefix collision.
#[test]
fn f2_session_and_ref_prefixes_are_canonical_or_refused() {
    let home = tmp_home();
    for session in [
        "aaaaaaaa-1111-2222-3333-444444444441",
        "aaaaaaaa-1111-2222-3333-444444444442",
    ] {
        cli(
            &home,
            &[
                "--session",
                session,
                "--harness",
                "codex",
                "hello",
                "--intent",
                "seed",
            ],
        );
    }
    let serve = Serve::start(home);

    let unique = post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"unique","to":{"kind":"session","value":"aaaaaaaa-1111-2222-3333-444444444441"}}"#,
        &[],
    );
    assert_eq!(unique.status, 201, "{}", unique.body);
    assert_eq!(
        unique.json()["event"]["to"]["value"],
        "aaaaaaaa-1111-2222-3333-444444444441"
    );

    let ambiguous = post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"ambiguous","to":{"kind":"session","value":"aaaaaaaa-1111"}}"#,
        &[],
    );
    assert_eq!(ambiguous.status, 409, "{}", ambiguous.body);

    let unknown = post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"unknown","to":{"kind":"session","value":"bbbbbbbb"}}"#,
        &[],
    );
    assert_eq!(unknown.status, 404, "{}", unknown.body);

    let first = post(serve.port, "/api/v1/notes", r#"{"text":"first"}"#, &[]);
    let second = post(serve.port, "/api/v1/notes", r#"{"text":"second"}"#, &[]);
    let first_id = first.json()["event"]["id"].as_str().unwrap().to_string();
    let second_id = second.json()["event"]["id"].as_str().unwrap().to_string();
    let common = first_id
        .chars()
        .zip(second_id.chars())
        .take_while(|(a, b)| a == b)
        .count();
    assert!(common >= 6, "ULIDs should share a usable time prefix");
    let ref_prefix = &first_id[..common.min(8)];
    let collision = post(
        serve.port,
        "/api/v1/notes",
        &format!(r#"{{"text":"reply","ref":"{ref_prefix}"}}"#),
        &[],
    );
    assert_eq!(collision.status, 409, "{}", collision.body);

    let missing = post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"reply","ref":"01ARZ3NDEKTSV4RRFFQ69G5FAV"}"#,
        &[],
    );
    assert_eq!(missing.status, 404, "{}", missing.body);
}

/// Notes are durable delivery only. Interrupting kinds never create or resume
/// a harness process, and the HTTP contract never reports a spawn attempt.
#[test]
fn f2_interrupting_notes_never_spawn_session_targets() {
    let home = tmp_home();
    cli(
        &home,
        &[
            "--session",
            "sess-wake-1",
            "--harness",
            "other",
            "hello",
            "--intent",
            "seed",
        ],
    );
    let serve = Serve::start(home);

    let fyi = post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"quiet ping","kind":"fyi","to":{"kind":"session","value":"sess-wake-1"}}"#,
        &[],
    );
    assert_eq!(fyi.status, 201, "body: {}", fyi.body);
    assert!(
        fyi.json().get("spawn").is_none(),
        "fyi must stay pull-only: {}",
        fyi.body
    );

    let warning = post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"check this now","kind":"warning","to":{"kind":"session","value":"sess-wake-1"}}"#,
        &[],
    );
    assert_eq!(warning.status, 201, "body: {}", warning.body);
    assert!(
        warning.json().get("spawn").is_none(),
        "warning must not report or attempt a process resume: {}",
        warning.body
    );

    let repo = post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"checkout-wide","kind":"warning","to":{"kind":"repo","value":"/tmp/x"}}"#,
        &[],
    );
    // Repo may 400 if path isn't a git root. A successful note is still only
    // a durable event and never reports a process action.
    if repo.status == 201 {
        assert!(
            repo.json().get("spawn").is_none(),
            "repo targets stay pull-only: {}",
            repo.body
        );
    }
}

/// Learning corpus: posting a note writes durable fleet-truth; the API
/// surfaces counters without requiring ops retention.
#[test]
fn learning_endpoint_counts_posted_notes() {
    let home = tmp_home();
    let serve = Serve::start(home);
    let before = get(serve.port, "/api/v1/learning").json();
    assert!(before["counters"].is_object(), "{}", before);

    let resp = post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"learn me","kind":"fyi"}"#,
        &[],
    );
    assert_eq!(resp.status, 201, "{}", resp.body);

    let after = get(serve.port, "/api/v1/learning?since=1d").json();
    assert!(
        after["counters"]["notes"].as_u64().unwrap_or(0) >= 1,
        "posted note must enter learning corpus: {after}"
    );
}

/// F2 — the write door is exactly one door: nothing else mutates, and a
/// cross-origin page cannot speak as the operator through the user's browser.
#[test]
fn f2_write_door_is_narrow_and_origin_checked() {
    let home = tmp_home();
    let serve = Serve::start(home);

    // A foreign origin is refused outright (CSRF).
    let evil = post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"I am the operator"}"#,
        &[("Origin", "http://evil.com")],
    );
    assert_eq!(evil.status, 403);

    // No other endpoint accepts a write, whatever the origin.
    for path in ["/api/v1/intent", "/api/v1/sessions", "/api/v1/doctor"] {
        let r = post(
            serve.port,
            path,
            "{}",
            &[("Origin", "http://localhost:3419")],
        );
        assert_eq!(r.status, 405, "{path} must not accept POST");
    }

    // Malformed input is refused loudly, never coerced into a broadcast.
    let bad_to = post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"x","to":{"kind":"everyone","value":"*"}}"#,
        &[],
    );
    assert_eq!(bad_to.status, 400);
    let empty = post(serve.port, "/api/v1/notes", r#"{"text":"   "}"#, &[]);
    assert_eq!(empty.status, 400);
    let oversized = format!(r#"{{"text":"{}"}}"#, "a".repeat(2500));
    assert_eq!(
        post(serve.port, "/api/v1/notes", &oversized, &[]).status,
        422
    );

    // Nothing above was appended.
    let events = get(serve.port, "/api/v1/events").json();
    assert_eq!(events["total"], 0);
}

/// F2 — the console's JSON POST is a preflighted request; the preflight must
/// name the method and the content-type header or the browser blocks it.
#[test]
fn f2_preflight_allows_the_console_to_post() {
    let home = tmp_home();
    let serve = Serve::start(home);
    let resp = request(
        serve.port,
        "OPTIONS",
        "/api/v1/notes",
        &[
            ("Origin", "http://localhost:3419"),
            ("Access-Control-Request-Method", "POST"),
        ],
    );
    assert_eq!(resp.status, 204);
    assert!(
        resp.header("Access-Control-Allow-Methods")
            .is_some_and(|v| v.contains("POST")),
        "headers: {}",
        resp.headers
    );
    assert!(
        resp.header("Access-Control-Allow-Headers")
            .is_some_and(|v| v.to_ascii_lowercase().contains("content-type")),
        "headers: {}",
        resp.headers
    );
}

#[test]
fn b1_endpoints_match_the_frozen_contract() {
    let home = tmp_home();
    cli(
        &home,
        &[
            "hello",
            "--session",
            "s-alpha",
            "--harness",
            "codex",
            "--cwd",
            "/tmp",
            "--intent",
            "refactoring the store",
        ],
    );
    cli(
        &home,
        &[
            "hello",
            "--session",
            "s-beta",
            "--harness",
            "claude-code",
            "--cwd",
            "/tmp",
        ],
    );
    cli(
        &home,
        &[
            "note",
            "heads up",
            "--session",
            "s-alpha",
            "--harness",
            "codex",
            "--cwd",
            "/tmp",
        ],
    );
    let serve = Serve::start(home);
    let port = serve.port;

    // /sessions: shape + exact SessionView key set, newest signal first.
    let r = get(port, "/api/v1/sessions");
    assert_eq!(r.status, 200);
    assert_eq!(
        r.header("Content-Type"),
        Some("application/json; charset=utf-8")
    );
    let v = r.json();
    let sessions = v["sessions"].as_array().expect("sessions array");
    assert_eq!(sessions.len(), 2);
    assert!(v["now_ms"].is_u64());
    let keys: Vec<&str> = sessions[0]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    for k in [
        "session",
        "harness",
        "host",
        "cwd",
        "repo",
        "branch",
        "role",
        "parent",
        "root",
        "depth",
        "descendants",
        "lifecycle",
        "event_count",
        "intent",
        "liveness",
        "last_signal_ms",
        "last_ms",
        "first_ms",
        "identity",
    ] {
        assert!(keys.contains(&k), "SessionView missing {k}");
    }
    assert_eq!(
        keys.len(),
        19,
        "SessionView has exactly the contract fields"
    );
    // Newest signal first: the note-bearing session leads.
    let first_last = sessions[0]["last_ms"].as_u64().unwrap();
    let second_last = sessions[1]["last_ms"].as_u64().unwrap();
    assert!(first_last >= second_last, "sorted newest signal first");
    // liveness is binary live|stale.
    for s in sessions {
        let l = s["liveness"].as_str().unwrap();
        assert!(
            l == "live" || l == "stale",
            "liveness is live|stale, got {l}"
        );
    }

    let snapshot = get(port, "/api/v1/snapshot").json();
    assert_eq!(snapshot["sessions"].as_array().unwrap().len(), 2);
    assert!(snapshot["events"].is_array());
    assert!(snapshot["channels"].is_object());
    assert!(snapshot["delivery"].is_array());
    assert!(snapshot["leases"].is_array());
    assert_eq!(snapshot["leases_known"], true);

    let health = get(port, "/api/v1/health").json();
    assert_eq!(health["ok"], true);
    assert!(health["pid"].is_u64());
    assert!(health["stream"]["incremental_reads"].is_u64());
    assert_eq!(health["stream"]["max_streams"], 32);

    // /events: file order + total/returned/newest_id disclosure.
    let r = get(port, "/api/v1/events?limit=2");
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["returned"], 2);
    assert!(v["total"].as_u64().unwrap() >= 3, "three seeded events");
    assert!(v["newest_id"].is_string());
    assert_eq!(v["events"].as_array().unwrap().len(), 2);

    // /doctor: store health.
    let r = get(port, "/api/v1/doctor");
    assert_eq!(r.status, 200);
    let v = r.json();
    assert!(v["segments"].as_u64().unwrap() >= 1);
    assert!(v["store_root"].is_string());

    // /leases: a store where nobody holds a checkout answers with an empty
    // board, never a 500 over a lease directory that was never created.
    let r = get(port, "/api/v1/leases");
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["leases"].as_array().unwrap().len(), 0);
    assert!(v["now_ms"].is_u64());

    // /meta.
    let r = get(port, "/api/v1/meta");
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["name"], "harbard");
    assert!(v["version"].is_string());
    assert!(v["started_ms"].is_u64());
    assert_eq!(v["protocol"], "harbard/protocol/v1");
    // The commit serve was built from (build.rs): what "Installed" compares.
    let commit = v["commit"].as_str().expect("meta.commit");
    assert!((7..=40).contains(&commit.len()) && commit.bytes().all(|b| b.is_ascii_hexdigit()));

    // /protocol (M5b) — same catalog codes as CLI / MCP.
    let r = get(port, "/api/v1/protocol");
    assert_eq!(r.status, 200);
    let v = r.json();
    assert_eq!(v["version"], "harbard/protocol/v1");
    assert_eq!(v["spec"], "harbard/2026-08-02");
    let codes = v["correction_codes"].as_array().unwrap();
    assert!(codes.iter().any(|c| c == "intent_stale"));
    assert!(codes.iter().any(|c| c == "ack_pending"));
    assert_eq!(v["deprecated_brief_aliases"]["status"], "intent_stale");
    let r = get(port, "/api/v1/protocol?version=nope");
    assert_eq!(r.status, 406);

    // Unknown path -> 404 JSON {error}.
    let r = get(port, "/api/v1/nope");
    assert_eq!(r.status, 404);
    assert!(r.json()["error"].is_string());
}

#[test]
fn b2_methods_and_cors_are_locked_down() {
    let home = tmp_home();
    cli(
        &home,
        &[
            "hello",
            "--session",
            "s1",
            "--harness",
            "codex",
            "--cwd",
            "/tmp",
        ],
    );
    let serve = Serve::start(home);
    let port = serve.port;

    // POST and OPTIONS -> 405 everywhere except the one write door
    // (`/api/v1/notes`, covered by the f2_* tests).
    assert_eq!(request(port, "POST", "/api/v1/sessions", &[]).status, 405);
    assert_eq!(
        request(port, "OPTIONS", "/api/v1/sessions", &[]).status,
        405
    );
    assert_eq!(request(port, "PUT", "/api/v1/notes", &[]).status, 405);
    assert_eq!(request(port, "DELETE", "/api/v1/notes", &[]).status, 405);

    // HEAD -> 200, no body.
    let r = request(port, "HEAD", "/api/v1/sessions", &[]);
    assert_eq!(r.status, 200);
    assert!(r.body.is_empty(), "HEAD has no body");

    // CORS: a localhost origin is echoed; a foreign origin gets no ACAO.
    let r = request(
        port,
        "GET",
        "/api/v1/meta",
        &[("Origin", "http://localhost:3000")],
    );
    assert_eq!(
        r.header("Access-Control-Allow-Origin"),
        Some("http://localhost:3000")
    );
    assert_eq!(
        r.header("Access-Control-Allow-Methods"),
        Some("GET, HEAD, POST")
    );

    let r = request(
        port,
        "GET",
        "/api/v1/meta",
        &[("Origin", "http://evil.example.com")],
    );
    assert_eq!(
        r.header("Access-Control-Allow-Origin"),
        None,
        "no CORS for foreign origin"
    );
}

#[test]
fn sse_streams_an_event_appended_while_open() {
    let home = tmp_home();
    cli(
        &home,
        &[
            "hello",
            "--session",
            "s1",
            "--harness",
            "codex",
            "--cwd",
            "/tmp",
        ],
    );
    let serve = Serve::start(home);
    let port = serve.port;

    let mut sock = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    sock.set_read_timeout(Some(Duration::from_millis(500)))
        .unwrap();
    let stream_request = format!(
        "GET /api/v1/stream HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {TEST_TOKEN}\r\nAccept: text/event-stream\r\n\r\n"
    );
    sock.write_all(stream_request.as_bytes()).unwrap();

    // Give the stream a moment to capture its cursor, then append an event.
    std::thread::sleep(Duration::from_millis(300));
    cli(
        &serve.home,
        &[
            "note",
            "live while streaming",
            "--session",
            "s1",
            "--harness",
            "codex",
            "--cwd",
            "/tmp",
        ],
    );

    // Read until the frame arrives or ~4 s elapse (1 s poll + slack).
    let mut acc = Vec::new();
    let mut buf = [0u8; 4096];
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        match sock.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                acc.extend_from_slice(&buf[..n]);
                if String::from_utf8_lossy(&acc).contains("live while streaming") {
                    break;
                }
            }
            Err(_) => {} // read timeout: keep polling until the deadline
        }
    }
    let text = String::from_utf8_lossy(&acc);
    assert!(
        text.contains("Content-Type: text/event-stream"),
        "SSE headers: {text}"
    );
    assert!(
        text.contains("event: bus"),
        "an event frame arrived: {text}"
    );
    assert!(
        text.contains("live while streaming"),
        "the appended event streamed: {text}"
    );
}

/// A broken proxy must not be able to consume the process' descriptor table
/// with upstream SSE sockets that never observe the browser disconnect.
#[test]
fn sse_connections_are_bounded() {
    let home = tmp_home();
    let serve = Serve::start(home);
    let mut streams = Vec::new();

    for _ in 0..32 {
        let mut sock = TcpStream::connect(("127.0.0.1", serve.port)).expect("connect stream");
        sock.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
        let request = format!(
            "GET /api/v1/stream HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {TEST_TOKEN}\r\nAccept: text/event-stream\r\n\r\n"
        );
        sock.write_all(request.as_bytes()).unwrap();
        let mut head = [0_u8; 256];
        let read = sock.read(&mut head).expect("read stream headers");
        let text = String::from_utf8_lossy(&head[..read]);
        assert!(text.starts_with("HTTP/1.1 200"), "stream rejected: {text}");
        streams.push(sock);
    }

    let rejected = get(serve.port, "/api/v1/stream");
    assert_eq!(rejected.status, 503, "body: {}", rejected.body);
    assert!(
        rejected.body.contains("too many live event streams"),
        "unexpected rejection: {}",
        rejected.body
    );

    drop(streams);
}

/// Channel membership via the operator write door mirrors CLI join/leave;
/// orphan create is gone — create requires an initial member (atomic join).
#[test]
fn channels_join_leave_create_via_post() {
    let home = tmp_home();
    let serve = Serve::start(home);
    let port = serve.port;
    let origin = &[("Origin", "http://localhost:3419")];

    // The operator can name an empty room; agents join it later.
    let empty = post(
        port,
        "/api/v1/channels",
        r#"{"action":"create","name":"Empty-room"}"#,
        origin,
    );
    assert_eq!(empty.status, 200, "{}", empty.body);
    assert_eq!(empty.json()["joined"], false);

    let create = post(
        port,
        "/api/v1/channels",
        r#"{"action":"create","name":"Incidents","kind":"repo","value":"/tmp/harbard-it"}"#,
        origin,
    );
    assert_eq!(create.status, 200, "{}", create.body);
    assert_eq!(create.json()["changed"], true);
    assert_eq!(create.json()["joined"], true);

    let join = post(
        port,
        "/api/v1/channels",
        r#"{"action":"join","name":"incidents","kind":"repo","value":"/tmp/harbard-it"}"#,
        origin,
    );
    assert_eq!(join.status, 200, "{}", join.body);
    assert_eq!(join.json()["changed"], false);

    let dup = post(
        port,
        "/api/v1/channels",
        r#"{"action":"join","name":"incidents","member":{"kind":"repo","value":"/tmp/harbard-it"}}"#,
        origin,
    );
    assert_eq!(dup.status, 200);
    assert_eq!(dup.json()["changed"], false);

    let list = get(port, "/api/v1/channels");
    assert_eq!(list.status, 200);
    let members = &list.json()["channels"]["incidents"];
    assert!(members.as_array().unwrap().len() == 1);

    let events = get(port, "/api/v1/events?limit=50");
    let events_json = events.json();
    let join_ev = events_json["events"]
        .as_array()
        .unwrap()
        .iter()
        .find(|e| e["event"] == "join");
    assert!(join_ev.is_some(), "join event on bus");
    assert!(
        join_ev.unwrap()["session"]
            .as_str()
            .unwrap()
            .starts_with("operator@")
    );

    let leave = post(
        port,
        "/api/v1/channels",
        r#"{"action":"leave","name":"incidents","kind":"repo","value":"/tmp/harbard-it"}"#,
        origin,
    );
    assert_eq!(leave.status, 200);
    assert_eq!(leave.json()["changed"], true);

    let archive = post(
        port,
        "/api/v1/channels",
        r#"{"action":"archive","name":"incidents"}"#,
        origin,
    );
    assert_eq!(archive.status, 200);
    assert!(
        archive.json()["archived"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n == "incidents")
    );

    let bad = post(
        port,
        "/api/v1/channels",
        r#"{"action":"nope","name":"x"}"#,
        origin,
    );
    assert_eq!(bad.status, 400);
    assert!(
        bad.json()["error"]
            .as_str()
            .unwrap()
            .contains("unknown action")
    );
}

/// Room metadata and the operator read position are server-backed. A client
/// may only advance the watermark to a real note in that exact Room.
#[test]
fn rooms_update_metadata_and_mark_real_activity_read() {
    let home = tmp_home();
    let serve = Serve::start(home);
    let origin = &[("Origin", "http://localhost:3419")];

    let create = post(
        serve.port,
        "/api/v1/channels",
        r#"{"action":"create","name":"Incidents","kind":"repo","value":"/tmp/harbard-it"}"#,
        origin,
    );
    assert_eq!(create.status, 200, "{}", create.body);

    let update = post(
        serve.port,
        "/api/v1/channels",
        r#"{"action":"update","name":"Incidents","purpose":"  Coordinate incidents  ","visibility":"limited"}"#,
        origin,
    );
    assert_eq!(update.status, 200, "{}", update.body);
    assert_eq!(update.json()["meta"]["purpose"], "Coordinate incidents");
    assert_eq!(update.json()["meta"]["visibility"], "limited");

    let note = post(
        serve.port,
        "/api/v1/notes",
        r#"{"text":"database recovered","to":{"kind":"channel","value":"Incidents"}}"#,
        origin,
    );
    assert_eq!(note.status, 201, "{}", note.body);
    let event_id = note.json()["event"]["id"].as_str().unwrap().to_string();

    let wrong_room = post(
        serve.port,
        "/api/v1/channels",
        &format!(r#"{{"action":"mark_read","name":"missing","event_id":"{event_id}"}}"#),
        origin,
    );
    assert_eq!(wrong_room.status, 422, "{}", wrong_room.body);

    let mark = post(
        serve.port,
        "/api/v1/channels",
        &format!(r#"{{"action":"mark_read","name":"Incidents","event_id":"{event_id}"}}"#),
        origin,
    );
    assert_eq!(mark.status, 200, "{}", mark.body);
    assert_eq!(mark.json()["changed"], true);
    assert_eq!(mark.json()["event_id"], event_id);

    let list = get(serve.port, "/api/v1/channels");
    assert_eq!(list.status, 200, "{}", list.body);
    assert_eq!(list.json()["meta"]["incidents"]["visibility"], "limited");
    assert_eq!(list.json()["reads"]["incidents"], event_id);
}

/// `/leases` answers "who owns which checkout". The exact key set is asserted
/// on purpose: the wire shape must not drift with `LeaseRecord`, and the
/// holder's `pid` must never reach a reader — `/api/v1/sessions` has never
/// published one, and a paired console holding the hub token would get a
/// kill target for free.
#[test]
fn leases_endpoint_lists_the_active_writer_and_stays_read_only() {
    let home = tmp_home();
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo = manifest.parent().and_then(|p| p.parent()).unwrap();
    let repo_arg = repo.to_str().unwrap();
    // An explicit live pid: auto-resolution walks for a harness supervisor
    // ancestor, which `cargo test` does not have.
    let supervisor = std::process::id().to_string();
    cli(
        &home,
        &[
            "claim",
            "--writer",
            "--repo",
            repo_arg,
            "--session",
            "s-writer",
            "--harness",
            "codex",
            "--pid",
            &supervisor,
            "--json",
        ],
    );

    let serve = Serve::start(home);
    let port = serve.port;

    let r = get(port, "/api/v1/leases");
    assert_eq!(r.status, 200);
    assert_eq!(
        r.header("Content-Type"),
        Some("application/json; charset=utf-8")
    );
    let v = r.json();
    assert!(v["now_ms"].is_u64());
    let leases = v["leases"].as_array().expect("leases array");
    assert_eq!(leases.len(), 1, "exactly the one seeded writer lease");
    let lease = &leases[0];

    assert_eq!(lease["worktree"], repo_arg);
    assert_eq!(lease["session"], "s-writer");
    assert_eq!(lease["harness"], "codex");
    let common_dir = lease["git_common_dir"].as_str().expect("git_common_dir");
    assert!(common_dir.ends_with("/.git"), "common dir: {common_dir}");
    let expected_repo = PathBuf::from(common_dir)
        .parent()
        .and_then(|p| p.file_name())
        .and_then(|n| n.to_str())
        .unwrap()
        .to_string();
    assert_eq!(lease["repo"], expected_repo);
    assert!(
        lease.get("claim_id").is_none(),
        "the claim id is a capability on the check --renew path: never on the wire"
    );
    assert!(lease["expires_at_ms"].as_u64().unwrap() > v["now_ms"].as_u64().unwrap());

    let mut keys: Vec<&str> = lease
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "branch",
            "claimed_at_ms",
            "expires_at_ms",
            "git_common_dir",
            "harness",
            "host",
            "renewed_at_ms",
            "repo",
            "session",
            "worktree",
        ],
        "WriterLeaseView has exactly the contract fields"
    );
    assert!(
        lease.get("pid").is_none(),
        "the holder's supervisor pid is never disclosed"
    );

    // HEAD carries the contract headers with an empty body, like every GET.
    let head = request(port, "HEAD", "/api/v1/leases", &[]);
    assert_eq!(head.status, 200);
    assert!(head.body.is_empty());

    // No lease verb on this surface: mutation stays with the local CLI.
    let mutation = post(port, "/api/v1/leases", r#"{"action":"release"}"#, &[]);
    assert_eq!(mutation.status, 405);

    // Same token gate as every other endpoint — nothing is exempt.
    let unauthorized = request_with_auth(port, "GET", "/api/v1/leases", &[], false);
    assert_eq!(unauthorized.status, 401);
}

#[test]
fn long_session_history_matches_cli_and_survives_answer_rotation() {
    use harbard_core::{
        Event, EventKind, NoteKind, Recipient, RecipientKind, Role, SCHEMA_VERSION, Store, clock,
        cursor,
    };
    let home = tmp_home();
    let store = Store::at(&home);
    let now = clock::now_ms();
    let old = now - 4 * 86_400_000;
    let event = |ms: u64, session: &str, kind: EventKind, text: &str| Event {
        v: SCHEMA_VERSION,
        id: ulid::Ulid::from_parts(ms, u128::from(ms)).to_string(),
        ts: clock::rfc3339_utc(ms),
        event: kind,
        session: session.into(),
        harness: "codex".into(),
        host: "test".into(),
        cwd: "/work/long".into(),
        repo: Some("/work/long".into()),
        repo_key: None,
        branch: Some("main".into()),
        role: Role::Main,
        parent: None,
        note_ref: None,
        text: text.into(),
        to: None,
        kind: NoteKind::Fyi,
    };
    let joined = event(old, "long-reader", EventKind::Hello, "");
    let sender = event(old + 1, "sender", EventKind::Hello, "");
    let intent = event(
        old + 2,
        "long-reader",
        EventKind::Intent,
        "working across days",
    );
    let mut answered = event(
        old + 3,
        "sender",
        EventKind::Note,
        "question answered today",
    );
    answered.kind = NoteKind::Ask;
    answered.to = Some(Recipient {
        kind: RecipientKind::Session,
        value: "long-reader".into(),
    });
    let mut outstanding = event(old + 4, "sender", EventKind::Note, "question still open");
    outstanding.kind = NoteKind::Ask;
    outstanding.to = answered.to.clone();
    std::fs::create_dir_all(store.events_dir()).unwrap();
    let mut lines = String::new();
    for event in [&joined, &sender, &intent, &answered, &outstanding] {
        lines.push_str(&event.to_line().unwrap());
        lines.push('\n');
    }
    std::fs::write(
        store.events_dir().join(format!(
            "events-{}.jsonl",
            clock::rfc3339_utc(old)[..10].replace('-', "")
        )),
        lines,
    )
    .unwrap();
    cursor::write_brief_state(
        &store,
        "long-reader",
        &cursor::BriefState {
            hb: now,
            ..Default::default()
        },
    )
    .unwrap();
    let serve = Serve::start(home);
    let snapshot = get(serve.port, "/api/v1/snapshot").json();
    let reader = snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["session"] == "long-reader")
        .unwrap();
    assert_eq!(reader["first_ms"], old);
    assert_eq!(reader["intent"], "working across days");
    assert_eq!(reader["liveness"], "live");
    assert_eq!(
        snapshot["total"], 0,
        "retained history is not the recent feed"
    );
    assert_eq!(get(serve.port, "/api/v1/events").json()["total"], 0);
    let trace = get(serve.port, "/api/v1/sessions/long-reader/trace").json();
    assert_eq!(trace["session"], "long-reader");
    assert_eq!(
        trace["harness"], "codex",
        "the old join still resolves the harness"
    );

    // The fleet projection (`harbard fleet`) reads retained history too.
    let cli_fleet = || -> serde_json::Value {
        let output = Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .env("HARBARD_NOW_MS", now.to_string())
            .args(["--home", serve.home.to_str().unwrap(), "--json", "fleet"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let fleet: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let reader = fleet["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["session"] == "long-reader")
            .unwrap();
        assert_eq!(reader["first_ms"], old, "the old join is kept");
        assert_eq!(reader["intent"], "working across days");
        fleet
    };
    let fleet = cli_fleet();
    assert_eq!(fleet["pending"].as_array().unwrap().len(), 2);

    // The first current-day append rotates the tail away from the old join.
    let mut answer = event(now, "long-reader", EventKind::Note, "the actual answer");
    answer.note_ref = Some(answered.id.clone());
    answer.to = Some(Recipient {
        kind: RecipientKind::Session,
        value: "sender".into(),
    });
    store.append(&answer).unwrap();
    let feed = events_with_minimum_len(serve.port, 1);
    assert_eq!(feed["total"], 1);
    assert_eq!(feed["events"][0]["id"], answer.id);
    let fleet = cli_fleet();
    assert_eq!(fleet["pending"].as_array().unwrap().len(), 1);
    assert_eq!(fleet["pending"][0]["note"], outstanding.id);
    let snapshot = get(serve.port, "/api/v1/snapshot").json();
    let reader = snapshot["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["session"] == "long-reader")
        .unwrap();
    assert_eq!(reader["first_ms"], old);
    assert_eq!(reader["intent"], "working across days");
    assert_eq!(reader["liveness"], "live");
    assert_eq!(snapshot["total"], 1);
}

/// `harbard attach` copies into `{home}/attachments/<ulid>/`, and serve
/// hands those bytes back with their own type; nothing outside the folder is
/// served, whatever the path says.
#[test]
fn attach_copies_into_the_home_and_serve_serves_only_from_there() {
    let home = tmp_home();
    let src = tmp_home();
    // The raw socket reader is text-only: synthetic bytes, valid UTF-8.
    std::fs::write(src.join("before.png"), b"PNG synthetic").unwrap();
    std::fs::write(
        src.join("report.md"),
        "# Informe\n<script>alert(1)</script>",
    )
    .unwrap();
    std::fs::write(src.join("secret.png"), b"outside").unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .arg("--home")
        .arg(&home)
        .args(["attach", "--label", "before"])
        .arg(src.join("before.png"))
        .arg(src.join("report.md"))
        .output()
        .expect("spawn attach");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let printed: Vec<PathBuf> = String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(PathBuf::from)
        .collect();
    assert_eq!(printed.len(), 2);
    for p in &printed {
        assert!(p.starts_with(home.join("attachments")), "{}", p.display());
    }
    // Refusals exit non-zero and write nothing.
    let refused = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .arg("--home")
        .arg(&home)
        .args(["attach"])
        .arg(src.join("missing.png"))
        .output()
        .unwrap();
    assert!(!refused.status.success());
    assert_eq!(
        std::fs::read_dir(home.join("attachments")).unwrap().count(),
        1
    );

    let serve = Serve::start(home.clone());
    let rel = |p: &PathBuf| {
        p.strip_prefix(home.join("attachments"))
            .unwrap()
            .display()
            .to_string()
    };
    let png = get(
        serve.port,
        &format!("/api/v1/attachments/{}", rel(&printed[0])),
    );
    assert_eq!(png.status, 200);
    assert_eq!(png.header("content-type"), Some("image/png"));
    assert_eq!(png.header("x-content-type-options"), Some("nosniff"));
    let md = get(
        serve.port,
        &format!("/api/v1/attachments/{}", rel(&printed[1])),
    );
    assert_eq!(md.status, 200);
    assert_eq!(md.header("content-type"), Some("text/plain; charset=utf-8"));
    assert!(
        md.body.contains("<script>"),
        "served as text, rendered nowhere"
    );
    let ulid = rel(&printed[0]).split('/').next().unwrap().to_string();
    for path in [
        format!(
            "/api/v1/attachments/{ulid}/../../../{}",
            src.join("secret.png").display()
        ),
        format!("/api/v1/attachments/{ulid}/%2e%2e%2fsecret.png"),
        format!("/api/v1/attachments/{}", src.join("secret.png").display()),
        "/api/v1/attachments/".to_string(),
    ] {
        let r = get(serve.port, &path);
        assert!(matches!(r.status, 403 | 404), "{path}: {}", r.status);
        assert!(!r.body.contains("outside"), "{path}");
    }
    let _ = std::fs::remove_dir_all(&src);
}

/// "Attach file": the console posts the bytes of the file the operator
/// picked; serve stores them like `harbard attach` and nowhere else.
#[test]
fn an_uploaded_file_lands_in_the_attachments_folder() {
    let home = tmp_home();
    let serve = Serve::start(home.clone());
    let r = post(
        serve.port,
        "/api/v1/attachments?name=nota%20de%20prueba.md&label=report",
        "# Hola",
        &[],
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let v = r.json();
    let path = PathBuf::from(v["path"].as_str().unwrap());
    assert!(
        path.starts_with(home.join("attachments")),
        "{}",
        path.display()
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "# Hola");
    assert_eq!(v["label"], "report");
    let got = get(
        serve.port,
        &format!("/api/v1/attachments/{}", v["id"].as_str().unwrap()),
    );
    assert_eq!((got.status, got.body.as_str()), (200, "# Hola"));
    // A type outside the list, a satellite, a foreign origin: refused.
    assert_eq!(
        post(serve.port, "/api/v1/attachments?name=x.sh", "echo", &[]).status,
        422
    );
    assert_eq!(
        post(
            serve.port,
            "/api/v1/attachments?name=x.md",
            "#",
            &[("X-Harbard-Device", "d1")]
        )
        .status,
        403
    );
    assert_eq!(
        post(
            serve.port,
            "/api/v1/attachments?name=x.md",
            "#",
            &[("Origin", "https://evil.example")]
        )
        .status,
        403
    );
    assert_eq!(
        std::fs::read_dir(home.join("attachments")).unwrap().count(),
        1
    );
}

/// A request whose Host is not serve's own gets
/// nothing, even with the token, reads included.
#[test]
fn a_foreign_host_is_refused_on_every_route_even_with_the_token() {
    let home = tmp_home();
    let serve = Serve::start(home);
    for path in [
        "/api/v1/meta",
        "/api/v1/workspace",
        "/api/v1/attachments/x/y.png",
        "/api/v1/feedback",
        "/api/v1/retro",
    ] {
        let ok = get(serve.port, path);
        assert_ne!(ok.status, 421, "{path} refused its own host");
        let foreign = request(serve.port, "GET", path, &[("Host", "rebind.evil.example")]);
        assert_eq!(foreign.status, 421, "{path}: {}", foreign.body);
    }
}

/// The named dev host is an explicit opt-in: refused by default, answered
/// when `HARBARD_ALLOWED_HOSTS` names it.
#[test]
fn a_named_host_is_answered_only_when_opted_in() {
    let off = Serve::start(tmp_home());
    let r = request(
        off.port,
        "GET",
        "/api/v1/meta",
        &[("Host", "foo.local:7419")],
    );
    assert_eq!(r.status, 421);
    drop(off);
    let home = tmp_home();
    let mut child = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .env("HARBARD_ALLOWED_HOSTS", "foo.local")
        .arg("--home")
        .arg(&home)
        .args(["serve", "--port", "0", "--token", TEST_TOKEN])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn serve");
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let port = serde_json::from_str::<serde_json::Value>(line.trim()).unwrap()["port"]
        .as_u64()
        .unwrap() as u16;
    let on = Serve { child, port, home };
    assert_eq!(
        request(
            on.port,
            "GET",
            "/api/v1/meta",
            &[("Host", "foo.local:7419")]
        )
        .status,
        200
    );
    assert_eq!(
        request(
            on.port,
            "GET",
            "/api/v1/meta",
            &[("Host", "bar.local:7419")]
        )
        .status,
        421
    );
}
