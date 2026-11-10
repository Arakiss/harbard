//! Pairing: one credential per paired device and the three roles (design
//! "harbard across the operator's machines", section 6).
//!
//! Every test runs a real hub (`serve` on an ephemeral loopback port) over a
//! throwaway home, and simulates each satellite on this same Mac with a
//! throwaway home of its own. Nothing here pairs with another machine, reads
//! the operator's home or touches a LaunchAgent: `pair` always runs with
//! `--no-install --no-daemon`.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde_json::{Value, json};

static SEQ: AtomicU64 = AtomicU64::new(0);
/// The hub's own key: what the local console and CLI present.
const OPERATOR_KEY: &str = "harbard-pairing-operator-key";

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "harbard-pairing-{tag}-{}-{}",
            std::process::id(),
            n
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("os-home")).expect("create temp home");
        Dir(dir)
    }

    fn store(&self) -> PathBuf {
        self.0.join("store")
    }

    /// The `HOME` the child sees: never the operator's.
    fn os_home(&self) -> PathBuf {
        self.0.join("os-home")
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn harbard(dir: &Dir, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_harbard"));
    cmd.env_clear()
        .env("HOME", dir.os_home())
        .env("PATH", "/usr/bin:/bin")
        .arg("--home")
        .arg(dir.store())
        .args(args)
        .stdin(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("spawn harbard")
}

fn ok_json(out: &Output) -> Value {
    assert!(
        out.status.success(),
        "exit {:?}\nstdout {}\nstderr {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text
        .lines()
        .rev()
        .find(|l| l.trim_start().starts_with('{'))
        .unwrap_or_else(|| panic!("no JSON in stdout: {text}"));
    serde_json::from_str(line).expect("json stdout")
}

struct Hub {
    child: Child,
    port: u16,
    startup: Value,
}

impl Hub {
    fn start(dir: &Dir, args: &[&str]) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .env("HOME", dir.os_home())
            .env("PATH", "/usr/bin:/bin")
            .arg("--home")
            .arg(dir.store())
            .args(["serve", "--port", "0"])
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn serve");
        let stdout = child.stdout.take().expect("piped stdout");
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        reader.read_line(&mut line).expect("read startup line");
        let startup: Value = serde_json::from_str(line.trim()).expect("startup json");
        let port = startup["port"].as_u64().expect("port") as u16;
        Hub {
            child,
            port,
            startup,
        }
    }

    fn with_key(dir: &Dir) -> Self {
        Hub::start(dir, &["--token", OPERATOR_KEY])
    }

    fn url(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }
}

impl Drop for Hub {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

struct Resp {
    status: u16,
    body: String,
}

impl Resp {
    fn json(&self) -> Value {
        serde_json::from_str(&self.body)
            .unwrap_or_else(|e| panic!("body is not JSON ({e}): {}", self.body))
    }
}

fn send(
    port: u16,
    method: &str,
    path: &str,
    token: Option<&str>,
    headers: &[(&str, &str)],
    body: &str,
) -> Resp {
    let mut sock = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n");
    if let Some(t) = token {
        req.push_str(&format!("Authorization: Bearer {t}\r\n"));
    }
    for (k, v) in headers {
        req.push_str(&format!("{k}: {v}\r\n"));
    }
    if method == "POST" {
        req.push_str("Content-Type: application/json\r\n");
        req.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    req.push_str("\r\n");
    req.push_str(body);
    sock.write_all(req.as_bytes()).expect("write");
    let mut raw = Vec::new();
    let _ = sock.read_to_end(&mut raw);
    let raw = String::from_utf8_lossy(&raw).to_string();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap_or((raw.as_str(), ""));
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse().ok())
        .unwrap_or_else(|| panic!("no status in {raw:?}"));
    Resp {
        status,
        body: body.to_string(),
    }
}

fn get(port: u16, path: &str, token: &str) -> Resp {
    send(port, "GET", path, Some(token), &[], "")
}

fn post(port: u16, path: &str, token: &str, body: &str) -> Resp {
    send(port, "POST", path, Some(token), &[], body)
}

/// Mint an invite on the hub's own home, the way `harbard hub invite` does
/// on the Mac, and return the invite line.
fn invite(hub_dir: &Dir, hub: &Hub, name: &str, role: &str) -> String {
    let port = hub.port.to_string();
    let out = harbard(
        hub_dir,
        &[
            "--json", "hub", "invite", "--name", name, "--role", role, "--port", &port,
        ],
        &[],
    );
    let v = ok_json(&out);
    let line = v["invite"].as_str().expect("invite line").to_string();
    // The tests talk to the hub on loopback; the line names whatever host
    // the hub would advertise, so point it at the ephemeral port here.
    let token = line.split_once('#').expect("#token").1.to_string();
    format!("{}#{token}", hub.url())
}

/// Pair a throwaway satellite home and return `(device id, its token)`.
fn pair(sat: &Dir, invite_line: &str, name: &str) -> (String, String) {
    let out = harbard(
        sat,
        &[
            "--json",
            "pair",
            invite_line,
            "--name",
            name,
            "--no-install",
            "--no-daemon",
        ],
        &[],
    );
    let v = ok_json(&out);
    let id = v["device"]["id"].as_str().expect("device id").to_string();
    let raw = std::fs::read_to_string(sat.store().join("satellite.json")).expect("satellite.json");
    let cfg: Value = serde_json::from_str(&raw).unwrap();
    (id, cfg["token"].as_str().unwrap().to_string())
}

fn hub_note(hub_dir: &Dir, session: &str, text: &str) {
    let out = harbard(
        hub_dir,
        &["hello", "--session", session, "--harness", "codex"],
        &[],
    );
    assert!(out.status.success());
    let out = harbard(
        hub_dir,
        &["note", text, "--session", session, "--harness", "codex"],
        &[],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn note_event(session: &str, text: &str, to: Option<&str>) -> String {
    let mut ev = json!({
        "v": 1,
        "id": "",
        "ts": "2026-11-07T10:00:00Z",
        "event": "note",
        "session": session,
        "harness": "claude-code",
        "host": "ignored",
        "cwd": "/tmp",
        "repo": null,
        "branch": null,
        "role": "main",
        "text": text,
        "kind": "fyi",
    });
    if let Some(to) = to {
        ev["to"] = json!({ "kind": "session", "value": to });
    }
    ev.to_string()
}

fn refusal(resp: &Resp, what: &str) {
    assert_eq!(resp.status, 403, "{what} must be refused: {}", resp.body);
    assert_eq!(
        resp.json()["error"],
        "role_forbids",
        "{what}: {}",
        resp.body
    );
}

#[test]
fn a_workshop_token_is_refused_everything_private_and_keeps_its_own_lane() {
    let hub_dir = Dir::new("hub");
    let hub = Hub::with_key(&hub_dir);
    hub_note(
        &hub_dir,
        "hub-local",
        "a note on the hub, for nobody remote",
    );
    let sat = Dir::new("workshop");
    let (id, token) = pair(
        &sat,
        &invite(&hub_dir, &hub, "borealis", "workshop"),
        "borealis",
    );
    assert_ne!(token, OPERATOR_KEY, "a device never holds the hub key");

    let p = hub.port;
    // Section 6: profile, photo, digests, feedback, retro, live views, desk
    // writes and settings.
    for path in [
        "/api/v1/profile",
        "/api/v1/profile/photo",
        "/api/v1/digests",
        "/api/v1/digests/pending",
        "/api/v1/feedback",
        "/api/v1/retro",
        "/api/v1/live/some-agent",
        "/api/v1/workspace",
        "/api/v1/sessions",
        "/api/v1/snapshot",
        "/api/v1/stream",
        "/api/v1/devices",
        "/api/v1/setup",
        "/api/v1/theme",
    ] {
        refusal(&get(p, path, &token), path);
    }
    for path in [
        "/api/v1/workspace/read",
        "/api/v1/workspace/save",
        "/api/v1/profile",
        "/api/v1/operator",
        "/api/v1/notes",
        "/api/v1/setup/install",
        "/api/v1/digests/read",
        "/api/v1/operator/reads",
        "/api/v1/channels",
    ] {
        refusal(&post(p, path, &token, "{}"), path);
    }

    // Its own lane: publish a session, read back only what is its own or
    // addressed to it.
    let mine = format!("borealis-{id}");
    // A session is the device's own once its hello published it.
    say_hello(p, &token, &mine);
    let r = post(
        p,
        "/api/v1/events",
        &token,
        &note_event(&mine, "hello from the workshop", None),
    );
    assert_eq!(r.status, 201, "{}", r.body);
    // The hub tails its store every 250 ms; give the read a moment.
    let read_own = || {
        let events = get(p, "/api/v1/events", &token);
        assert_eq!(events.status, 200, "{}", events.body);
        let texts: Vec<String> = events.json()["events"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["text"].as_str().unwrap_or("").to_string())
            .collect();
        (events, texts)
    };
    let (mut events, mut texts) = read_own();
    for _ in 0..30 {
        if texts.iter().any(|t| t == "hello from the workshop") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
        (events, texts) = read_own();
    }
    assert!(
        texts.iter().any(|t| t == "hello from the workshop"),
        "{texts:?} {} devices={} all={}",
        events.body,
        get(p, "/api/v1/devices", OPERATOR_KEY).body,
        get(p, "/api/v1/events", OPERATOR_KEY).body
    );
    assert!(
        !texts.iter().any(|t| t.contains("for nobody remote")),
        "a Taller device never reads another machine's notes: {texts:?}"
    );
    // It cannot speak as a session that is not its own.
    let forged = post(
        p,
        "/api/v1/events",
        &token,
        &note_event("hub-local", "suplantado", None),
    );
    assert_eq!(forged.status, 403, "{}", forged.body);
    assert_eq!(get(p, "/api/v1/meta", &token).status, 200);
}

#[test]
fn a_display_token_reads_the_summary_and_nothing_else() {
    let hub_dir = Dir::new("hub");
    let hub = Hub::with_key(&hub_dir);
    hub_note(&hub_dir, "hub-local", "a body a display must never see");
    let sat = Dir::new("display");
    let (_, token) = pair(&sat, &invite(&hub_dir, &hub, "strip", "display"), "strip");
    let p = hub.port;

    let summary = get(p, "/api/v1/summary", &token);
    assert_eq!(summary.status, 200, "{}", summary.body);
    assert!(
        !summary.body.contains("a body a display must never see"),
        "the summary carries no message body: {}",
        summary.body
    );
    let v = summary.json();
    assert!(v["agents"].is_array(), "{v}");
    assert!(v["needs"].is_number(), "{v}");

    for path in [
        "/api/v1/events",
        "/api/v1/feed",
        "/api/v1/sessions",
        "/api/v1/snapshot",
        "/api/v1/workspace",
        "/api/v1/workspace?only=activity",
        "/api/v1/stream",
        "/api/v1/live/some-agent",
        "/api/v1/history",
        "/api/v1/search?q=body",
        "/api/v1/attachments/01ABC/x.md",
        "/api/v1/profile",
        "/api/v1/profile/brief",
    ] {
        refusal(&get(p, path, &token), path);
    }
    for path in [
        "/api/v1/notes",
        "/api/v1/events",
        "/api/v1/devices",
        "/api/v1/workspace/read",
        "/api/v1/profile",
        "/api/v1/channels",
        "/api/v1/evals",
        "/api/v1/attachments?name=x.md",
        "/api/v1/operator",
        "/api/v1/operator/reads",
        "/api/v1/digests/read",
        "/api/v1/setup/install",
        "/api/v1/setup/complete",
    ] {
        refusal(&post(p, path, &token, "{}"), path);
    }
}

#[test]
fn a_revoked_token_fails_at_once_and_the_others_keep_working() {
    let hub_dir = Dir::new("hub");
    let hub = Hub::with_key(&hub_dir);
    let a = Dir::new("a");
    let b = Dir::new("b");
    let (a_id, a_token) = pair(&a, &invite(&hub_dir, &hub, "uno", "workshop"), "uno");
    let (_, b_token) = pair(&b, &invite(&hub_dir, &hub, "dos", "workshop"), "dos");
    assert_ne!(a_token, b_token, "one credential per device");
    assert_eq!(get(hub.port, "/api/v1/meta", &a_token).status, 200);

    let out = harbard(&hub_dir, &["--json", "hub", "revoke", &a_id], &[]);
    ok_json(&out);
    // No restart, no cache: the very next request is refused.
    let r = get(hub.port, "/api/v1/meta", &a_token);
    assert_eq!(r.status, 401, "{}", r.body);
    assert_eq!(r.json()["error"], "revoked");
    assert_eq!(get(hub.port, "/api/v1/meta", &b_token).status, 200);
    assert_eq!(get(hub.port, "/api/v1/meta", OPERATOR_KEY).status, 200);
    // The registry says so.
    let devices = get(hub.port, "/api/v1/devices", OPERATOR_KEY).json();
    let card = devices["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == a_id.as_str())
        .cloned()
        .unwrap();
    assert!(card["revoked_ms"].as_u64().unwrap() > 0, "{card}");
}

#[test]
fn no_shared_token_remains_after_migration() {
    let hub_dir = Dir::new("hub");
    // Today's single-Mac setup: a hub.token, and a satellite card paired
    // with that same shared secret before per-device credentials.
    std::fs::create_dir_all(hub_dir.store()).unwrap();
    std::fs::write(
        hub_dir.store().join("hub.token"),
        format!("{OPERATOR_KEY}\n"),
    )
    .unwrap();
    std::fs::write(
        hub_dir.store().join("devices.json"),
        json!({ "devices": [{
            "id": "legacy-laptop", "name": "laptop", "host": "laptop.local",
            "role": "satellite", "os": "macos", "arch": "aarch64",
            "harnesses": ["claude-code"], "joined_ms": 1, "last_seen_ms": 2
        }]})
        .to_string(),
    )
    .unwrap();
    // serve with no --token: it reads hub.token, as the LaunchAgent does.
    let hub = Hub::start(&hub_dir, &[]);
    let p = hub.port;

    // The local console and CLI keep working with no action.
    assert_eq!(get(p, "/api/v1/meta", OPERATOR_KEY).status, 200);
    assert_eq!(get(p, "/api/v1/workspace", OPERATOR_KEY).status, 200);

    // The shared secret no longer speaks for any device.
    let r = send(
        p,
        "POST",
        "/api/v1/events",
        Some(OPERATOR_KEY),
        &[("X-Harbard-Device", "legacy-laptop")],
        &note_event("legacy-s", "old pairing", None),
    );
    assert_eq!(r.status, 403, "{}", r.body);
    assert_eq!(r.json()["error"], "shared_token_retired");

    // The legacy card is a Workshop with no credential; the hub is Operator.
    let devices = get(p, "/api/v1/devices", OPERATOR_KEY).json();
    let list = devices["devices"].as_array().unwrap();
    let legacy = list.iter().find(|d| d["id"] == "legacy-laptop").unwrap();
    assert_eq!(legacy["access"], "workshop");
    assert_eq!(legacy["created_ms"], 0);
    let own = list.iter().find(|d| d["role"] == "hub").unwrap();
    assert_eq!(own["access"], "operator");

    // A new device gets its own token, stored hashed at 0600 on the hub.
    let sat = Dir::new("new");
    let (_, token) = pair(&sat, &invite(&hub_dir, &hub, "new", "workshop"), "new");
    assert_ne!(token, OPERATOR_KEY);
    let creds = hub_dir.store().join("credentials.json");
    let raw = std::fs::read_to_string(&creds).unwrap();
    assert!(
        !raw.contains(&token),
        "the hub keeps a hash, never the token"
    );
    assert!(!raw.contains(OPERATOR_KEY));
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&creds).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let sat_mode = std::fs::metadata(sat.store().join("satellite.json"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(sat_mode, 0o600);
}

#[test]
fn promoting_to_operator_is_deliberate() {
    let hub_dir = Dir::new("hub");
    let hub = Hub::with_key(&hub_dir);
    let sat = Dir::new("mbp");
    let (id, token) = pair(
        &sat,
        &invite(&hub_dir, &hub, "macbook", "workshop"),
        "macbook",
    );
    refusal(&get(hub.port, "/api/v1/profile", &token), "profile before");

    // An invite never hands out Operator.
    let port = hub.port.to_string();
    let out = harbard(
        &hub_dir,
        &[
            "hub", "invite", "--name", "x", "--role", "operator", "--port", &port,
        ],
        &[],
    );
    assert!(!out.status.success());

    // Without the explicit confirmation nothing changes.
    let out = harbard(&hub_dir, &["hub", "role", &id, "operator"], &[]);
    assert!(!out.status.success());
    refusal(
        &get(hub.port, "/api/v1/profile", &token),
        "profile unconfirmed",
    );

    ok_json(&harbard(
        &hub_dir,
        &["--json", "hub", "role", &id, "operator", "--yes"],
        &[],
    ));
    assert_eq!(get(hub.port, "/api/v1/profile", &token).status, 200);

    // A device cannot promote itself by posting its card.
    ok_json(&harbard(
        &hub_dir,
        &["--json", "hub", "role", &id, "workshop"],
        &[],
    ));
    let card = json!({
        "id": id, "name": "macbook", "host": "h", "role": "satellite",
        "joined_ms": 0, "last_seen_ms": 0, "access": "operator"
    });
    assert_eq!(
        post(hub.port, "/api/v1/devices", &token, &card.to_string()).status,
        200
    );
    refusal(&get(hub.port, "/api/v1/profile", &token), "self-promotion");
}

#[test]
fn the_serve_bind_list_contains_no_wildcard() {
    let hub_dir = Dir::new("hub");
    // The retired LAN bind now means loopback (plus the tailnet address,
    // when one is configured; none is here).
    let hub = Hub::start(&hub_dir, &["--token", OPERATOR_KEY, "--bind", "lan"]);
    let binds: Vec<String> = hub.startup["binds"]
        .as_array()
        .unwrap_or_else(|| panic!("startup line lists its binds: {}", hub.startup))
        .iter()
        .map(|b| b.as_str().unwrap().to_string())
        .collect();
    assert_eq!(binds, vec!["127.0.0.1".to_string()]);
    assert!(!binds.iter().any(|b| b == "0.0.0.0" || b == "::"));
}

#[test]
fn a_remote_transcript_never_reaches_the_hub_store() {
    let hub_dir = Dir::new("hub");
    let hub = Hub::with_key(&hub_dir);
    let sat = Dir::new("workshop");
    // A transcript on the satellite, where its harness would write it.
    let marker = format!("TRANSCRIPT-MARKER-{}", std::process::id());
    let transcript = sat.os_home().join(".claude/projects/-work/s1.jsonl");
    std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
    std::fs::write(
        &transcript,
        format!("{{\"type\":\"assistant\",\"text\":\"{marker}\"}}\n"),
    )
    .unwrap();
    let (_, token) = pair(
        &sat,
        &invite(&hub_dir, &hub, "borealis", "workshop"),
        "borealis",
    );
    // The satellite's agent speaks through the hub, as a paired CLI does.
    let out = harbard(
        &sat,
        &["hello", "--session", "s1", "--harness", "claude-code"],
        &[],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let out = harbard(
        &sat,
        &[
            "note",
            "work done",
            "--session",
            "s1",
            "--harness",
            "claude-code",
        ],
        &[],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(get(hub.port, "/api/v1/meta", &token).status, 200);

    // Byte for byte: no file under the hub's store holds the transcript.
    fn walk(dir: &Path, marker: &[u8], hits: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, marker, hits);
            } else if let Ok(bytes) = std::fs::read(&path)
                && bytes.windows(marker.len()).any(|w| w == marker)
            {
                hits.push(path);
            }
        }
    }
    let mut hits = Vec::new();
    walk(&hub_dir.store(), marker.as_bytes(), &mut hits);
    assert!(
        hits.is_empty(),
        "transcript bytes in the hub store: {hits:?}"
    );
    // And the note did arrive, so the path was exercised.
    let events = get(hub.port, "/api/v1/events", OPERATOR_KEY);
    assert!(events.body.contains("work done"), "{}", events.body);
}

// ---------------------------------------------------------------------------
// Preferences › Pairing (step 2)
// ---------------------------------------------------------------------------

fn pairing_row(port: u16, id: &str) -> Value {
    let list = get(port, "/api/v1/pairing", OPERATOR_KEY);
    assert_eq!(list.status, 200, "{}", list.body);
    list.json()["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["id"] == id)
        .cloned()
        .unwrap_or_else(|| panic!("{id} in {}", list.body))
}

#[test]
fn every_remote_read_and_write_lands_in_that_devices_log() {
    let hub_dir = Dir::new("hub");
    let hub = Hub::with_key(&hub_dir);
    let a = Dir::new("a");
    let b = Dir::new("b");
    let (a_id, a_token) = pair(
        &a,
        &invite(&hub_dir, &hub, "borealis", "workshop"),
        "borealis",
    );
    let (b_id, b_token) = pair(&b, &invite(&hub_dir, &hub, "tele", "display"), "tele");
    let p = hub.port;

    assert_eq!(get(p, "/api/v1/meta?x=secreto", &a_token).status, 200);
    let mine = format!("borealis-{a_id}");
    say_hello(p, &a_token, &mine);
    assert_eq!(
        post(
            p,
            "/api/v1/events",
            &a_token,
            &note_event(&mine, "hello", None)
        )
        .status,
        201
    );
    assert_eq!(get(p, "/api/v1/profile", &a_token).status, 403);
    assert_eq!(get(p, "/api/v1/summary", &b_token).status, 200);

    // The line is written once the answer is sent: give it a moment.
    let mut log = get(p, &format!("/api/v1/pairing/{a_id}/log"), OPERATOR_KEY);
    for _ in 0..30 {
        if log.json()["actions"]
            .as_array()
            .is_some_and(|a| a.len() >= 5)
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
        log = get(p, &format!("/api/v1/pairing/{a_id}/log"), OPERATOR_KEY);
    }
    assert_eq!(log.status, 200, "{}", log.body);
    let actions = log.json()["actions"].as_array().unwrap().clone();
    let seen: Vec<(String, String, u64, bool)> = actions
        .iter()
        .map(|a| {
            (
                a["method"].as_str().unwrap().to_string(),
                a["path"].as_str().unwrap().to_string(),
                a["status"].as_u64().unwrap(),
                a["write"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        &seen[..],
        &[
            ("GET".into(), "/api/v1/profile".into(), 403, false),
            ("POST".into(), "/api/v1/events".into(), 201, true),
            ("POST".into(), "/api/v1/events".into(), 201, true),
            ("GET".into(), "/api/v1/meta".into(), 200, false),
            ("POST".into(), "/api/v1/devices".into(), 201, true),
        ],
        "newest first, path only, refusals included: {}",
        log.body
    );
    assert!(!log.body.contains("secreto"), "never the query");
    // Each device keeps its own log: its pairing, then its own reads.
    let b_log = get(p, &format!("/api/v1/pairing/{b_id}/log"), OPERATOR_KEY).json();
    let b_paths: Vec<&str> = b_log["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["path"].as_str().unwrap())
        .collect();
    assert_eq!(b_paths, ["/api/v1/summary", "/api/v1/devices"], "{b_log}");

    let row = pairing_row(p, &a_id);
    assert_eq!(row["name"], "borealis");
    assert_eq!(row["access"], "workshop");
    assert_eq!(row["credential"], true);
    assert!(row["last_seen_ms"].as_u64().unwrap() > 0);
    assert!(row["created_ms"].as_u64().unwrap() > 0);
    assert_eq!(row["recent"].as_array().unwrap().len(), 3);
    assert_eq!(row["writes"], 3, "pairing, hello and note");
    let own = get(p, "/api/v1/pairing", OPERATOR_KEY).json()["devices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["hub"] == true)
        .cloned()
        .unwrap();
    assert_eq!(own["access"], "operator");
}

#[test]
fn pairing_is_the_operators_and_its_writes_are_the_desks() {
    let hub_dir = Dir::new("hub");
    let hub = Hub::with_key(&hub_dir);
    let sat = Dir::new("t");
    let (id, token) = pair(
        &sat,
        &invite(&hub_dir, &hub, "borealis", "workshop"),
        "borealis",
    );
    let p = hub.port;
    refusal(&get(p, "/api/v1/pairing", &token), "equipos list");
    refusal(
        &get(p, &format!("/api/v1/pairing/{id}/log"), &token),
        "own log",
    );
    refusal(
        &post(
            p,
            &format!("/api/v1/pairing/{id}/role"),
            &token,
            r#"{"role":"operator","confirm":"borealis"}"#,
        ),
        "self-promotion",
    );
    // The hub key with a device id never reaches the desk either.
    let r = send(
        p,
        "POST",
        &format!("/api/v1/pairing/{id}/revoke"),
        Some(OPERATOR_KEY),
        &[("Origin", "https://evil.example")],
        "{}",
    );
    assert_eq!(r.status, 403, "{}", r.body);
    assert_eq!(get(p, "/api/v1/meta", &token).status, 200, "still paired");
}

#[test]
fn operator_from_pairing_needs_the_devices_name_typed() {
    let hub_dir = Dir::new("hub");
    let hub = Hub::with_key(&hub_dir);
    let sat = Dir::new("mbp");
    let (id, token) = pair(
        &sat,
        &invite(&hub_dir, &hub, "MacBook", "workshop"),
        "MacBook",
    );
    let p = hub.port;
    let role = format!("/api/v1/pairing/{id}/role");

    let bare = post(p, &role, OPERATOR_KEY, r#"{"role":"operator"}"#);
    assert_eq!(bare.status, 422, "{}", bare.body);
    assert_eq!(bare.json()["error"], "confirm_required");
    let wrong = post(
        p,
        &role,
        OPERATOR_KEY,
        r#"{"role":"operator","confirm":"BOREALIS"}"#,
    );
    assert_eq!(wrong.status, 422);
    refusal(&get(p, "/api/v1/profile", &token), "not yet");

    let ok = post(
        p,
        &role,
        OPERATOR_KEY,
        r#"{"role":"operator","confirm":" macbook "}"#,
    );
    assert_eq!(ok.status, 200, "{}", ok.body);
    assert_eq!(get(p, "/api/v1/profile", &token).status, 200);
    assert_eq!(pairing_row(p, &id)["access"], "operator");

    // Down is never gated.
    assert_eq!(
        post(p, &role, OPERATOR_KEY, r#"{"role":"display"}"#).status,
        200
    );
    refusal(&get(p, "/api/v1/profile", &token), "pantalla again");
    assert_eq!(get(p, "/api/v1/summary", &token).status, 200);
    assert_eq!(
        post(p, &role, OPERATOR_KEY, r#"{"role":"jefe"}"#).status,
        400
    );
}

#[test]
fn revoke_from_pairing_cuts_one_device_at_once() {
    let hub_dir = Dir::new("hub");
    let hub = Hub::with_key(&hub_dir);
    let a = Dir::new("a");
    let b = Dir::new("b");
    let (a_id, a_token) = pair(&a, &invite(&hub_dir, &hub, "uno", "workshop"), "uno");
    let (_, b_token) = pair(&b, &invite(&hub_dir, &hub, "dos", "workshop"), "dos");
    let p = hub.port;

    let r = post(
        p,
        &format!("/api/v1/pairing/{a_id}/revoke"),
        OPERATOR_KEY,
        "",
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(get(p, "/api/v1/meta", &a_token).status, 401);
    assert_eq!(get(p, "/api/v1/meta", &b_token).status, 200);
    let row = pairing_row(p, &a_id);
    assert!(row["revoked_ms"].as_u64().is_some(), "{row}");
    assert_eq!(row["credential"], false);
    // A revoked device keeps no role.
    assert_eq!(
        post(
            p,
            &format!("/api/v1/pairing/{a_id}/role"),
            OPERATOR_KEY,
            r#"{"role":"workshop"}"#
        )
        .status,
        409
    );

    // The hub itself is never revoked.
    let hub_id = hub.startup["device"].as_str().unwrap().to_string();
    let own = post(
        p,
        &format!("/api/v1/pairing/{hub_id}/revoke"),
        OPERATOR_KEY,
        "",
    );
    assert_eq!(own.status, 409, "{}", own.body);
    assert_eq!(get(p, "/api/v1/meta", OPERATOR_KEY).status, 200);
}

#[test]
fn a_device_id_that_would_name_a_path_cannot_pair() {
    let hub_dir = Dir::new("hub");
    let hub = Hub::with_key(&hub_dir);
    let line = invite(&hub_dir, &hub, "x", "workshop");
    let token = line.split_once('#').unwrap().1;
    let card = json!({
        "id": "../evil", "name": "x", "host": "h", "role": "satellite",
        "joined_ms": 0, "last_seen_ms": 0
    });
    let r = post(hub.port, "/api/v1/devices", token, &card.to_string());
    assert!(
        r.status == 400 || r.status == 403,
        "{} {}",
        r.status,
        r.body
    );
    assert!(!hub_dir.store().join("evil.jsonl").exists());
    assert!(!hub_dir.0.join("evil.jsonl").exists());
}

// ---------------------------------------------------------------------------
// The profile split (step 3)
// ---------------------------------------------------------------------------

/// A PNG's first bytes: enough for serve to take it as a photo.
const PNG: &[u8] = &[
    0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 13, b'I', b'H', b'D', b'R', 0, 0,
    0, 1, 0, 0, 0, 1, 8, 6, 0, 0, 0,
];

fn seed_profile(port: u16) {
    let r = post(
        port,
        "/api/v1/profile",
        OPERATOR_KEY,
        &json!({
            "name": "Dana",
            "address": "first name",
            "time_zone": "Etc/UTC",
            "hours": {"start": "09:00", "end": "18:00", "days": [1, 2, 3, 4, 5]},
            "context": "I build harbard. I prefer small, verified changes.",
            "state": {"id": "focused"},
            "dnd_default": "1h",
            "away_suggest": false,
        })
        .to_string(),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    // The photo, by its bytes, the way the console uploads it.
    let mut sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let head = format!(
        "POST /api/v1/profile/photo HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nAuthorization: Bearer {OPERATOR_KEY}\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\n\r\n",
        PNG.len()
    );
    sock.write_all(head.as_bytes()).unwrap();
    sock.write_all(PNG).unwrap();
    let mut raw = String::new();
    let _ = sock.read_to_string(&mut raw);
    assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
}

#[test]
fn a_workshop_device_reads_the_brief_block_and_nothing_else_of_the_profile() {
    let hub_dir = Dir::new("hub");
    let hub = Hub::with_key(&hub_dir);
    seed_profile(hub.port);
    let sat = Dir::new("workshop");
    let (_, token) = pair(
        &sat,
        &invite(&hub_dir, &hub, "borealis", "workshop"),
        "borealis",
    );
    let p = hub.port;

    let brief = get(p, "/api/v1/profile/brief", &token);
    assert_eq!(brief.status, 200, "{}", brief.body);
    let v = brief.json();
    let keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    assert_eq!(keys, vec!["block", "prompt", "state", "now_ms"], "{v}");
    let block = v["block"].as_str().expect("a block");
    // State, rules, context and hours: what its agents need to respect them.
    assert!(block.contains("Focused"), "{block}");
    assert!(block.contains("Rules:"), "{block}");
    assert!(block.contains("I prefer small"), "{block}");
    assert!(block.contains("09:00"), "{block}");
    assert_eq!(v["state"], "focused");
    assert!(v["prompt"].as_str().unwrap().contains("Focused"));
    // Exactly what an agent on the hub reads at SessionStart, nothing more.
    let store = harbard_core::Store::at(hub_dir.store());
    let desk = harbard_core::workspace::DeskState::load(&store);
    let local = harbard_core::profile::Profile::load(store.root())
        .brief_block(&desk.config.operator_name, v["now_ms"].as_u64().unwrap());
    assert_eq!(Some(block.to_string()), local);
    // None of the raw profile: no settings, no photo, no file names.
    for private in [
        "dnd_default",
        "away_suggest",
        "utc_offset_min",
        "photo",
        "updated_ms",
        "PNG",
    ] {
        assert!(
            !brief.body.contains(private),
            "{private} leaked: {}",
            brief.body
        );
    }

    // Everything else of the profile stays Operator's.
    for path in [
        "/api/v1/profile",
        "/api/v1/profile/photo",
        "/api/v1/digests",
        "/api/v1/digests/pending",
        "/api/v1/operator",
    ] {
        refusal(&get(p, path, &token), path);
    }
    refusal(
        &post(p, "/api/v1/profile", &token, r#"{"context":"x"}"#),
        "profile write",
    );
    // The operator still reads the whole profile.
    assert_eq!(get(p, "/api/v1/profile", OPERATOR_KEY).status, 200);
    assert_eq!(get(p, "/api/v1/profile/photo", OPERATOR_KEY).status, 200);
    assert_eq!(get(p, "/api/v1/profile/brief", OPERATOR_KEY).status, 200);
}

#[test]
fn a_display_device_reads_only_the_summary_never_the_profile() {
    let hub_dir = Dir::new("hub");
    let hub = Hub::with_key(&hub_dir);
    seed_profile(hub.port);
    let sat = Dir::new("display");
    let (_, token) = pair(&sat, &invite(&hub_dir, &hub, "tele", "display"), "tele");
    let p = hub.port;
    for path in [
        "/api/v1/profile/brief",
        "/api/v1/profile",
        "/api/v1/profile/photo",
        "/api/v1/digests",
        "/api/v1/operator",
        "/api/v1/protocol",
    ] {
        refusal(&get(p, path, &token), path);
    }
    let summary = get(p, "/api/v1/summary", &token);
    assert_eq!(summary.status, 200);
    for private in ["I prefer small", "Dana", "focused", "Etc/UTC"] {
        assert!(
            !summary.body.contains(private),
            "{private} in the summary: {}",
            summary.body
        );
    }
}

#[test]
fn an_empty_profile_gives_a_workshop_device_no_block() {
    let hub_dir = Dir::new("hub");
    let hub = Hub::with_key(&hub_dir);
    let sat = Dir::new("workshop");
    let (_, token) = pair(
        &sat,
        &invite(&hub_dir, &hub, "borealis", "workshop"),
        "borealis",
    );
    let v = get(hub.port, "/api/v1/profile/brief", &token).json();
    assert_eq!(v["block"], Value::Null, "nothing is invented: {v}");
    assert_eq!(v["prompt"], Value::Null);
    assert_eq!(v["state"], "available");
}

#[test]
fn a_workshop_satellites_agents_read_the_operators_brief_block_from_the_hub() {
    let hub_dir = Dir::new("hub");
    let hub = Hub::with_key(&hub_dir);
    seed_profile(hub.port);
    let sat = Dir::new("workshop");
    pair(
        &sat,
        &invite(&hub_dir, &hub, "borealis", "workshop"),
        "borealis",
    );
    let cwd = sat.os_home();
    let cwd = cwd.to_str().unwrap();

    // SessionStart on the satellite: the same block an agent on the hub reads.
    let out = harbard(
        &sat,
        &[
            "brief",
            "--session",
            "s1",
            "--harness",
            "claude-code",
            "--cwd",
            cwd,
        ],
        &[],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("Context for Dana"), "{text}");
    assert!(text.contains("I prefer small"), "{text}");
    // Kept at 0600 for the next prompts, and only the block.
    let cache = sat.store().join("profile-brief.json");
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(&cache).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let raw = std::fs::read_to_string(&cache).unwrap();
    assert!(
        !raw.contains("dnd_default") && !raw.contains("photo"),
        "{raw}"
    );

    // Any other prompt: the state line, while it is not Available.
    let out = harbard(
        &sat,
        &[
            "brief",
            "--touch",
            "--session",
            "s1",
            "--harness",
            "claude-code",
            "--cwd",
            cwd,
        ],
        &[],
    );
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    assert!(text.contains("Focused"), "{text}");
    assert!(!text.contains("I prefer small"), "{text}");
}

// ---------------------------------------------------------------------------
// Review fixes: the security review of Pairing (Clara's probes, ported)
// ---------------------------------------------------------------------------

/// A card as a device might post it, claiming everything the hub decides.
fn greedy_card(id: &str, sessions: &[&str]) -> String {
    json!({
        "id": id, "name": "intruso", "host": "intruso.local", "role": "satellite",
        "os": "macos", "arch": "aarch64", "harnesses": ["claude-code"],
        "joined_ms": 0, "last_seen_ms": 0,
        "access": "operator", "created_ms": 0,
        "sessions": sessions,
    })
    .to_string()
}

fn invite_token(line: &str) -> String {
    line.split_once('#').unwrap().1.to_string()
}

/// The hub tails its store every 250 ms.
fn settle() {
    std::thread::sleep(Duration::from_millis(1500));
}

/// A session's `hello`: the one event that makes it a device's own.
fn hello_event(session: &str) -> String {
    let mut ev: Value = serde_json::from_str(&note_event(session, "", None)).unwrap();
    ev["event"] = json!("hello");
    ev.to_string()
}

fn say_hello(port: u16, token: &str, session: &str) {
    let r = post(port, "/api/v1/events", token, &hello_event(session));
    assert_eq!(r.status, 201, "hello {session}: {}", r.body);
}

/// H1. Session ownership never comes from the card a device posts.
#[test]
fn a_card_never_hands_a_device_the_hubs_sessions() {
    let hub_dir = Dir::new("h1");
    let hub = Hub::with_key(&hub_dir);
    hub_note(&hub_dir, "hub-sess-1", "HUB-PRIVATE-NOTE-H1");
    settle();
    let token = invite_token(&invite(&hub_dir, &hub, "intruso", "workshop"));
    let r = post(
        hub.port,
        "/api/v1/devices",
        &token,
        &greedy_card("intruso01", &["hub-sess-1"]),
    );
    assert!(
        r.status < 300,
        "pairing itself works: {} {}",
        r.status,
        r.body
    );
    assert_eq!(r.json()["device"]["sessions"], json!(null), "{}", r.body);
    assert_eq!(r.json()["device"]["access"], "workshop", "{}", r.body);

    let read = get(hub.port, "/api/v1/events?limit=500", &token)
        .body
        .contains("HUB-PRIVATE-NOTE-H1");
    let r = post(
        hub.port,
        "/api/v1/events",
        &token,
        &note_event("hub-sess-1", "forged as the hub session", None),
    );
    assert!(
        !read && r.status == 403,
        "declared hub-sess-1 in its card: reads the hub note {read} · posts as it {}",
        r.status
    );
    // Nor by saying hello as it: the hub already knows that session.
    let r = post(
        hub.port,
        "/api/v1/events",
        &token,
        &hello_event("hub-sess-1"),
    );
    assert_eq!(r.status, 403, "hello as a hub session: {}", r.body);
    // The heartbeat cannot add one either.
    let r = post(
        hub.port,
        "/api/v1/devices",
        &token,
        &greedy_card("intruso01", &["hub-sess-1"]),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.json()["device"]["sessions"], json!(null), "{}", r.body);
    assert!(
        !get(hub.port, "/api/v1/events?limit=500", &token)
            .body
            .contains("HUB-PRIVATE-NOTE-H1")
    );
}

/// H1. A device owns a session only once its own hello published it, and
/// never another device's.
#[test]
fn a_device_owns_only_the_sessions_its_own_hello_published() {
    let hub_dir = Dir::new("h1b");
    let hub = Hub::with_key(&hub_dir);
    let a = Dir::new("h1b-a");
    let b = Dir::new("h1b-b");
    let (_, a_token) = pair(&a, &invite(&hub_dir, &hub, "uno", "workshop"), "uno");
    let (_, b_token) = pair(&b, &invite(&hub_dir, &hub, "dos", "workshop"), "dos");
    let p = hub.port;
    // No hello, no voice.
    let r = post(
        p,
        "/api/v1/events",
        &a_token,
        &note_event("a-s1", "no hello", None),
    );
    assert_eq!(r.status, 403, "{}", r.body);
    assert_eq!(r.json()["error"], "not_your_session");
    say_hello(p, &a_token, "a-s1");
    let r = post(
        p,
        "/api/v1/events",
        &a_token,
        &note_event("a-s1", "A-OWN-NOTE", Some("a-s1")),
    );
    assert_eq!(r.status, 201, "{}", r.body);
    // Another device can neither greet nor speak as it.
    for ev in [hello_event("a-s1"), note_event("a-s1", "suplantado", None)] {
        let r = post(p, "/api/v1/events", &b_token, &ev);
        assert_eq!(r.status, 403, "{}", r.body);
    }
    settle();
    assert!(
        get(p, "/api/v1/events?limit=500", &a_token)
            .body
            .contains("A-OWN-NOTE")
    );
    assert!(
        !get(p, "/api/v1/events?limit=500", &b_token)
            .body
            .contains("A-OWN-NOTE")
    );
}

/// M1. An invite never takes over a device id the hub already holds.
#[test]
fn an_invite_cannot_take_over_another_paired_device() {
    let hub_dir = Dir::new("m1");
    let hub = Hub::with_key(&hub_dir);
    let laptop = Dir::new("m1-laptop");
    let (laptop_id, laptop_token) = pair(
        &laptop,
        &invite(&hub_dir, &hub, "laptop", "workshop"),
        "laptop",
    );
    say_hello(hub.port, &laptop_token, "laptop-s1");
    let r = post(
        hub.port,
        "/api/v1/events",
        &laptop_token,
        &note_event("laptop-s1", "LAPTOP-NOTE-M1", Some("laptop-s1")),
    );
    assert_eq!(r.status, 201, "{}", r.body);

    let intruder = invite_token(&invite(&hub_dir, &hub, "tv", "workshop"));
    let r = post(
        hub.port,
        "/api/v1/devices",
        &intruder,
        &greedy_card(&laptop_id, &[]),
    );
    let took_over = r.status < 300;
    let laptop_after = get(hub.port, "/api/v1/meta", &laptop_token).status;
    let leaked = get(hub.port, "/api/v1/events?limit=500", &intruder)
        .body
        .contains("LAPTOP-NOTE-M1");
    assert!(
        !took_over && laptop_after == 200 && !leaked,
        "claim by id: status {} {} · laptop token now {} · intruder reads laptop notes {}",
        r.status,
        r.body,
        laptop_after,
        leaked
    );
    assert_eq!(r.json()["error"], "device_id_taken", "{}", r.body);
    // The invite was not spent: it still pairs a device of its own.
    let r = post(
        hub.port,
        "/api/v1/devices",
        &intruder,
        &greedy_card("tv01", &[]),
    );
    assert_eq!(r.status, 201, "{}", r.body);

    // Once the operator revokes the laptop, a new invite may re-pair that
    // id, and it starts with nothing the old card held.
    assert!(
        harbard(&hub_dir, &["hub", "revoke", &laptop_id], &[])
            .status
            .success()
    );
    let again = invite_token(&invite(&hub_dir, &hub, "laptop", "display"));
    let r = post(
        hub.port,
        "/api/v1/devices",
        &again,
        &greedy_card(&laptop_id, &[]),
    );
    assert!(r.status < 300, "{}", r.body);
    let card = &r.json()["device"];
    assert_eq!(card["access"], "display");
    assert_eq!(card["sessions"], json!(null), "{card}");
    assert_eq!(card["name"], "laptop");
    assert_eq!(get(hub.port, "/api/v1/meta", &laptop_token).status, 401);
}

/// Open a stream as `token`, wait for a note through it, run `cut`, and
/// return whether a note posted after the cut still arrived.
fn stream_survives(hub_dir: &Dir, hub: &Hub, token: &str, cut: impl FnOnce()) -> (bool, String) {
    let mut sock = TcpStream::connect(("127.0.0.1", hub.port)).unwrap();
    sock.set_read_timeout(Some(Duration::from_millis(300)))
        .unwrap();
    let req = format!(
        "GET /api/v1/stream HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nAccept: text/event-stream\r\n\r\n"
    );
    sock.write_all(req.as_bytes()).unwrap();
    let mut seen = String::new();
    let read_for = |sock: &mut TcpStream, ms: u64, seen: &mut String| {
        let until = std::time::Instant::now() + Duration::from_millis(ms);
        let mut buf = [0u8; 8192];
        while std::time::Instant::now() < until {
            match sock.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => seen.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(_) => {}
            }
        }
    };
    read_for(&mut sock, 1000, &mut seen);
    assert!(
        seen.starts_with("HTTP/1.1 200"),
        "stream opened: {seen:.200}"
    );
    hub_note(hub_dir, "hub-s", "BEFORE-CUT");
    read_for(&mut sock, 3000, &mut seen);
    assert!(seen.contains("BEFORE-CUT"), "control: the stream delivers");
    cut();
    hub_note(hub_dir, "hub-s", "AFTER-CUT");
    read_for(&mut sock, 3000, &mut seen);
    (seen.contains("AFTER-CUT"), seen)
}

/// M2. Revocation closes an open stream at once.
#[test]
fn revocation_closes_an_open_stream() {
    let hub_dir = Dir::new("m2");
    let hub = Hub::with_key(&hub_dir);
    let sat = Dir::new("m2-sat");
    let (id, token) = pair(&sat, &invite(&hub_dir, &hub, "mbp", "workshop"), "mbp");
    let out = harbard(&hub_dir, &["hub", "role", &id, "operator", "--yes"], &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let (leaked, seen) = stream_survives(&hub_dir, &hub, &token, || {
        assert!(
            harbard(&hub_dir, &["hub", "revoke", &id], &[])
                .status
                .success()
        );
        assert_eq!(get(hub.port, "/api/v1/meta", &token).status, 401);
    });
    assert!(
        !leaked,
        "a revoked device still received a note body on its open stream"
    );
    assert!(
        seen.contains("revoked"),
        "the stream says why it closed: {seen:.2000}"
    );
}

/// M2. Demotion from Operator closes an open stream at once too.
#[test]
fn demotion_closes_an_open_stream() {
    let hub_dir = Dir::new("m2b");
    let hub = Hub::with_key(&hub_dir);
    let sat = Dir::new("m2b-sat");
    let (id, token) = pair(&sat, &invite(&hub_dir, &hub, "mbp", "workshop"), "mbp");
    assert!(
        harbard(&hub_dir, &["hub", "role", &id, "operator", "--yes"], &[])
            .status
            .success()
    );
    let (leaked, _) = stream_survives(&hub_dir, &hub, &token, || {
        assert!(
            harbard(&hub_dir, &["hub", "role", &id, "workshop"], &[])
                .status
                .success()
        );
    });
    assert!(
        !leaked,
        "a demoted device still received a note body on its open stream"
    );
}

/// M3. A Workshop device reads an attachment only when someone else, not one
/// of its own sessions, cited it in a note addressed to it.
#[test]
fn a_self_addressed_citation_never_unlocks_an_attachment() {
    let hub_dir = Dir::new("m3");
    let hub = Hub::with_key(&hub_dir);
    let r = send(
        hub.port,
        "POST",
        "/api/v1/attachments?name=nomina.txt",
        Some(OPERATOR_KEY),
        &[],
        "ATTACHMENT-SECRET-M3",
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let att = r.json()["id"].as_str().unwrap().to_string();
    let dir = att.split('/').next().unwrap().to_string();

    let sat = Dir::new("m3-sat");
    let (_, token) = pair(
        &sat,
        &invite(&hub_dir, &hub, "borealis", "workshop"),
        "borealis",
    );
    let other = Dir::new("m3-other");
    let (_, other_token) = pair(&other, &invite(&hub_dir, &hub, "otro", "workshop"), "otro");
    let path = format!("/api/v1/attachments/{att}");
    assert_eq!(
        get(hub.port, &path, &token).status,
        403,
        "control: refused at first"
    );
    say_hello(hub.port, &token, "t-s1");
    let cite = format!("see attachments/{dir}/");
    let r = post(
        hub.port,
        "/api/v1/events",
        &token,
        &note_event("t-s1", &cite, Some("t-s1")),
    );
    assert_eq!(r.status, 201, "{}", r.body);
    // Nor through a second device it might also hold.
    say_hello(hub.port, &other_token, "o-s1");
    let r = post(
        hub.port,
        "/api/v1/events",
        &other_token,
        &note_event("o-s1", &cite, Some("t-s1")),
    );
    assert_eq!(r.status, 201, "{}", r.body);
    settle();
    let r = get(hub.port, &path, &token);
    assert_ne!(
        r.status, 200,
        "Taller read an attachment nobody addressed to it: {}",
        r.body
    );

    // A hub agent addressing it to the device unlocks it.
    hub_note(&hub_dir, "hub-s", "hello");
    let out = harbard(
        &hub_dir,
        &[
            "note",
            &cite,
            "--session",
            "hub-s",
            "--harness",
            "codex",
            "--to",
            "session:t-s1",
        ],
        &[],
    );
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    settle();
    let r = get(hub.port, &path, &token);
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(r.body.contains("ATTACHMENT-SECRET-M3"));
}

/// M4. A tailnet address that is not up never stops serve: loopback answers,
/// and the tailnet bind is logged and retried.
#[test]
fn an_absent_tailnet_address_keeps_loopback_serving() {
    let hub_dir = Dir::new("m4");
    std::fs::create_dir_all(hub_dir.store()).unwrap();
    // Inside 100.64.0.0/10, so it is taken, but on no interface here.
    std::fs::write(
        hub_dir.store().join("tailnet.json"),
        r#"{"addr":"100.100.100.123"}"#,
    )
    .unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .env("HOME", hub_dir.os_home())
        .env("PATH", "/usr/bin:/bin")
        .arg("--home")
        .arg(hub_dir.store())
        .args([
            "serve",
            "--port",
            "0",
            "--bind",
            "tailnet",
            "--token",
            OPERATOR_KEY,
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let started: Option<Value> = serde_json::from_str(line.trim()).ok();
    let meta = started
        .as_ref()
        .and_then(|v| v["port"].as_u64())
        .map(|port| get(port as u16, "/api/v1/meta", OPERATOR_KEY).status);
    let _ = child.kill();
    let out = child.wait_with_output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let started =
        started.unwrap_or_else(|| panic!("serve did not start on loopback; stderr: {stderr}"));
    assert_eq!(meta, Some(200), "loopback answers");
    assert_eq!(started["binds"], json!(["127.0.0.1"]), "{started}");
    assert_eq!(started["tailnet_pending"], "100.100.100.123", "{started}");
    assert!(stderr.contains("tailnet_bind_failed"), "{stderr}");
}

/// L1. `harness: "device"` is only the device's own presence, never a way
/// around session ownership.
#[test]
fn harness_device_never_skips_session_ownership() {
    let hub_dir = Dir::new("l1");
    let hub = Hub::with_key(&hub_dir);
    let sat = Dir::new("l1-sat");
    let (id, token) = pair(
        &sat,
        &invite(&hub_dir, &hub, "borealis", "workshop"),
        "borealis",
    );
    let as_device = |session: &str, kind: &str| {
        let mut ev: Value = serde_json::from_str(&note_event(
            session,
            "note with harness device",
            Some("hub-sess-1"),
        ))
        .unwrap();
        ev["harness"] = json!("device");
        ev["kind"] = json!("ask");
        ev["event"] = json!(kind);
        post(hub.port, "/api/v1/events", &token, &ev.to_string())
    };
    for (session, kind) in [
        (format!("lead-{id}"), "note"),
        (format!("device:{id}"), "note"),
        (format!("lead-{id}"), "hello"),
    ] {
        let r = as_device(&session, kind);
        assert_eq!(
            r.status, 403,
            "{kind} as {session} under harness=device: {}",
            r.body
        );
    }
    let r = as_device(&format!("device:{id}"), "device_leave");
    assert_eq!(r.status, 201, "its own presence: {}", r.body);
}

/// L2. The log records what identification refuses, and the pairing.
#[test]
fn the_log_records_refusals_before_routing_and_the_pairing() {
    let hub_dir = Dir::new("l2");
    let hub = Hub::with_key(&hub_dir);
    let sat = Dir::new("l2-sat");
    let (id, token) = pair(
        &sat,
        &invite(&hub_dir, &hub, "borealis", "workshop"),
        "borealis",
    );
    assert_eq!(get(hub.port, "/api/v1/meta", &token).status, 200);
    // Its credential with another device's id: refused, and logged as its own.
    let r = send(
        hub.port,
        "GET",
        "/api/v1/meta",
        Some(&token),
        &[("X-Harbard-Device", "someone-else")],
        "",
    );
    assert_eq!(r.status, 403, "{}", r.body);
    assert!(
        harbard(&hub_dir, &["hub", "revoke", &id], &[])
            .status
            .success()
    );
    assert_eq!(get(hub.port, "/api/v1/meta", &token).status, 401);
    // No sleep: the line is written before the answer goes out.
    let log = get(hub.port, &format!("/api/v1/pairing/{id}/log"), OPERATOR_KEY).json();
    let actions = log["actions"].as_array().unwrap();
    let paired = actions
        .iter()
        .any(|a| a["path"] == "/api/v1/devices" && a["method"] == "POST" && a["status"] == 201);
    let wrong = actions.iter().any(|a| a["status"] == 403);
    let refused = actions.iter().any(|a| a["status"] == 401);
    assert!(
        paired && wrong && refused,
        "pairing logged {paired}, wrong device {wrong}, revoked attempt {refused}: {actions:?}"
    );
}

/// L3. A pending invite is listed, and revoking it by name stops it pairing.
#[test]
fn an_unclaimed_invite_is_listed_and_can_be_revoked() {
    let hub_dir = Dir::new("l3");
    let hub = Hub::with_key(&hub_dir);
    let line = invite(&hub_dir, &hub, "filtrada", "workshop");
    let status = ok_json(&harbard(&hub_dir, &["--json", "hub", "status"], &[]));
    let pending = status["invites"].as_array().cloned().unwrap_or_default();
    assert!(
        pending
            .iter()
            .any(|i| i["name"] == "filtrada" && i["access"] == "workshop"),
        "{status}"
    );
    assert!(
        !status.to_string().contains(&invite_token(&line)),
        "never the token"
    );
    let listed = get(hub.port, "/api/v1/pairing", OPERATOR_KEY).json();
    assert_eq!(listed["invites"][0]["name"], "filtrada", "{listed}");

    let out = harbard(&hub_dir, &["hub", "revoke", "filtrada"], &[]);
    let token = invite_token(&line);
    let r = post(
        hub.port,
        "/api/v1/devices",
        &token,
        &greedy_card("ladron01", &[]),
    );
    assert!(
        out.status.success() && r.status >= 400,
        "revoke exit {:?} ({}) · pairing with the leaked invite {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stderr).trim(),
        r.status
    );
    let listed = get(hub.port, "/api/v1/pairing", OPERATOR_KEY).json();
    assert_eq!(listed["invites"], json!([]), "{listed}");

    // And from Preferences › Pairing, by its reference.
    let line = invite(&hub_dir, &hub, "otra", "display");
    let listed = get(hub.port, "/api/v1/pairing", OPERATOR_KEY).json();
    let reference = listed["invites"][0]["ref"].as_str().unwrap().to_string();
    let r = post(
        hub.port,
        &format!("/api/v1/pairing/invites/{reference}/revoke"),
        OPERATOR_KEY,
        "",
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let r = post(
        hub.port,
        "/api/v1/devices",
        &invite_token(&line),
        &greedy_card("ladron02", &[]),
    );
    assert!(r.status >= 400, "{}", r.body);
}

fn sat_brief(sat: &Dir) -> String {
    let cwd = sat.os_home().to_str().unwrap().to_string();
    let out = harbard(
        sat,
        &[
            "brief",
            "--session",
            "s1",
            "--harness",
            "claude-code",
            "--cwd",
            &cwd,
        ],
        &[],
    );
    String::from_utf8_lossy(&out.stdout).to_string()
}

/// L4. A revoked satellite drops its cached brief block.
#[test]
fn a_revoked_satellite_drops_the_cached_brief() {
    let hub_dir = Dir::new("l4");
    let hub = Hub::with_key(&hub_dir);
    let r = post(
        hub.port,
        "/api/v1/profile",
        OPERATOR_KEY,
        &json!({"name": "Dana", "context": "I prefer small, verified changes."}).to_string(),
    );
    assert_eq!(r.status, 200, "{}", r.body);
    let sat = Dir::new("l4-sat");
    let (id, _) = pair(
        &sat,
        &invite(&hub_dir, &hub, "borealis", "workshop"),
        "borealis",
    );
    assert!(
        sat_brief(&sat).contains("I prefer small"),
        "control: brief served"
    );
    assert!(
        harbard(&hub_dir, &["hub", "revoke", &id], &[])
            .status
            .success()
    );
    // Age the cache past its one-minute freshness, as two minutes would.
    let cache = sat.store().join("profile-brief.json");
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&cache).unwrap()).unwrap();
    v["fetched_ms"] = json!(v["fetched_ms"].as_u64().unwrap() - 120_000);
    std::fs::write(&cache, v.to_string()).unwrap();
    assert!(
        !sat_brief(&sat).contains("I prefer small"),
        "a revoked device still serves the operator's brief block from cache"
    );
    assert!(!cache.exists(), "the refused cache is deleted");
}

/// L4. A future-dated cache is stale, and a cache from another hub or
/// device is never read.
#[test]
fn a_future_or_foreign_brief_cache_is_not_fresh() {
    let hub_dir = Dir::new("l4b");
    let hub = Hub::with_key(&hub_dir);
    let sat = Dir::new("l4b-sat");
    pair(
        &sat,
        &invite(&hub_dir, &hub, "borealis", "workshop"),
        "borealis",
    );
    let cache = sat.store().join("profile-brief.json");
    // Written for this very pairing, so only its date gives it away.
    let cfg: Value =
        serde_json::from_str(&std::fs::read_to_string(sat.store().join("satellite.json")).unwrap())
            .unwrap();
    std::fs::write(
        &cache,
        json!({
            "fetched_ms": 9_000_000_000_000u64, "block": "BLOQUE-ENVENENADO", "prompt": null,
            "hub": cfg["hub"], "device": cfg["device_id"],
        })
        .to_string(),
    )
    .unwrap();
    assert!(
        !sat_brief(&sat).contains("BLOQUE-ENVENENADO"),
        "a future-dated cache was served instead of the hub's block"
    );
    // A fresh copy written for another hub is ignored too.
    let mut v: Value = serde_json::from_str(&std::fs::read_to_string(&cache).unwrap()).unwrap();
    v["block"] = json!("BLOQUE-DE-OTRO-HUB");
    v["hub"] = json!("http://127.0.0.1:1");
    std::fs::write(&cache, v.to_string()).unwrap();
    assert!(!sat_brief(&sat).contains("BLOQUE-DE-OTRO-HUB"));
}

/// L5. The per-device log stays 0600 after its first trim.
#[test]
fn the_log_stays_0600_after_a_trim() {
    use std::os::unix::fs::PermissionsExt;
    let hub_dir = Dir::new("l5");
    let hub = Hub::with_key(&hub_dir);
    let sat = Dir::new("l5-sat");
    let (id, token) = pair(&sat, &invite(&hub_dir, &hub, "tv", "display"), "tv");
    let log = hub_dir.store().join("pairing").join(format!("{id}.jsonl"));
    assert_eq!(get(hub.port, "/api/v1/meta", &token).status, 200);
    assert_eq!(
        std::fs::metadata(&log).unwrap().permissions().mode() & 0o777,
        0o600
    );
    // ~90 bytes a line: 1,600 requests pass the 128 KiB trim.
    for _ in 0..1_600 {
        get(hub.port, "/api/v1/meta", &token);
    }
    let mode = std::fs::metadata(&log).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "the trimmed log is {mode:o}");
    let lines = std::fs::read_to_string(&log).unwrap().lines().count();
    assert!(lines <= 1_600, "trimmed: {lines}");
}

/// L6. A device keeps the name the operator gave its invite and the host it
/// paired with, and never takes the hub's host.
#[test]
fn a_device_cannot_take_the_hubs_host_or_rename_itself() {
    let hub_dir = Dir::new("l6");
    let hub = Hub::with_key(&hub_dir);
    let hub_host = get(hub.port, "/api/v1/meta", OPERATOR_KEY).json()["host"]
        .as_str()
        .unwrap()
        .to_string();
    let p = hub.port;
    // Pairing declares the hub's own host: it is never stored as such.
    let token = invite_token(&invite(&hub_dir, &hub, "borealis", "workshop"));
    let mut card: Value = serde_json::from_str(&greedy_card("borealis01", &[])).unwrap();
    card["host"] = json!(hub_host);
    card["name"] = json!("otro nombre");
    let r = post(p, "/api/v1/devices", &token, &card.to_string());
    assert_eq!(r.status, 201, "{}", r.body);
    let paired_host = r.json()["device"]["host"].as_str().unwrap().to_string();
    assert_ne!(paired_host, hub_host, "{}", r.body);
    assert_eq!(r.json()["device"]["name"], "borealis", "the invite's name");

    // A heartbeat renames nothing and moves no host.
    card["name"] = json!("Mac de Dana (hub)");
    card["host"] = json!("hub-host.spoofed");
    let r = post(p, "/api/v1/devices", &token, &card.to_string());
    assert!(r.status < 300, "{}", r.body);
    say_hello(p, &token, "t-s1");
    let r = post(
        p,
        "/api/v1/events",
        &token,
        &note_event("t-s1", "hello", None),
    );
    assert_eq!(r.status, 201, "{}", r.body);
    let stamped = r.json()["event"]["host"].as_str().unwrap_or("").to_string();
    let row = pairing_row(p, "borealis01");
    assert!(
        row["name"] == "borealis" && stamped == paired_host && stamped != hub_host,
        "Equipos shows {} · events stamped {stamped} (paired {paired_host}, hub {hub_host})",
        row["name"]
    );
}

/// Controls from the review, kept as regressions: bad tailnet configs stay
/// on loopback, one invite pairs one device under a race, traversal ids are
/// refused, and the summary hands out no session id.
#[test]
fn review_controls_still_hold() {
    for raw in [
        r#"{"addr":"0.0.0.0"}"#,
        r#"{"addr":"::"}"#,
        r#"{"addr":"192.168.1.20"}"#,
        r#"{"addr":"100.64.0.1/10"}"#,
        r#"{"addr":" 0.0.0.0 "}"#,
        r#"{"addr":["100.64.0.1"]}"#,
        r#"{"addr":"::ffff:100.64.0.1"}"#,
        r#"{"addr":"0x64.0x40.0.1"}"#,
        "not json",
    ] {
        let hub_dir = Dir::new("c1");
        std::fs::create_dir_all(hub_dir.store()).unwrap();
        std::fs::write(hub_dir.store().join("tailnet.json"), raw).unwrap();
        let hub = Hub::start(&hub_dir, &["--bind", "tailnet", "--token", OPERATOR_KEY]);
        assert_eq!(hub.startup["binds"], json!(["127.0.0.1"]), "{raw}");
    }

    let hub_dir = Dir::new("c2");
    let hub = Hub::with_key(&hub_dir);
    let token = invite_token(&invite(&hub_dir, &hub, "race", "workshop"));
    let port = hub.port;
    let handles: Vec<_> = (0..8)
        .map(|i| {
            let token = token.clone();
            std::thread::spawn(move || {
                post(
                    port,
                    "/api/v1/devices",
                    &token,
                    &greedy_card(&format!("race{i:02}"), &[]),
                )
                .status
            })
        })
        .collect();
    let codes: Vec<u16> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_eq!(codes.iter().filter(|c| **c < 300).count(), 1, "{codes:?}");

    for bad in ["..", "../x", "a/b", "a%2fb", "x.jsonl", ".lock"] {
        let r = get(port, &format!("/api/v1/pairing/{bad}/log"), OPERATOR_KEY);
        assert_ne!(r.status, 200, "{bad}: {}", r.body);
        let token = invite_token(&invite(&hub_dir, &hub, "t", "workshop"));
        let r = post(port, "/api/v1/devices", &token, &greedy_card(bad, &[]));
        assert_eq!(r.status, 400, "{bad}: {}", r.body);
    }

    hub_note(&hub_dir, "3f2a9c1e-hub-session-uuid", "hello");
    settle();
    let tv = Dir::new("c4-tv");
    let (_, token) = pair(&tv, &invite(&hub_dir, &hub, "tv", "display"), "tv");
    let r = get(port, "/api/v1/summary", &token);
    assert_eq!(r.status, 200, "{}", r.body);
    assert!(!r.body.contains("3f2a9c1e-hub-session-uuid"), "{}", r.body);
}

/// H1, the satellite side. A session whose SessionStart never fired (a
/// headless run) says hello on its first touch, and that hello reaches the
/// hub too, so the session can speak there afterwards.
#[test]
fn a_satellite_session_that_only_touches_still_says_hello_to_the_hub() {
    let hub_dir = Dir::new("h1c");
    let hub = Hub::with_key(&hub_dir);
    let sat = Dir::new("h1c-sat");
    let (_, token) = pair(
        &sat,
        &invite(&hub_dir, &hub, "borealis", "workshop"),
        "borealis",
    );
    let cwd = sat.os_home().to_str().unwrap().to_string();
    let common = [
        "--session",
        "headless-s1",
        "--harness",
        "codex",
        "--cwd",
        &cwd,
    ];
    let mut touch = vec!["brief", "--touch"];
    touch.extend(common);
    let out = harbard(&sat, &touch, &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut note = vec!["note", "HEADLESS-NOTE"];
    note.extend(common);
    let out = harbard(&sat, &note, &[]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    settle();
    assert!(
        get(hub.port, "/api/v1/events?limit=500", &token)
            .body
            .contains("HEADLESS-NOTE")
    );
}

/// Second review, N2. A card paired with the old shared secret holds no
/// credential: its machine re-pairs with an invite under its own id, and
/// starts from nothing the old card held.
#[test]
fn a_legacy_card_can_re_pair_with_an_invite() {
    let hub_dir = Dir::new("n2");
    std::fs::create_dir_all(hub_dir.store()).unwrap();
    std::fs::write(
        hub_dir.store().join("devices.json"),
        json!({ "devices": [{
            "id": "legacy-laptop", "name": "laptop", "host": "laptop.local",
            "role": "satellite", "os": "macos", "arch": "aarch64",
            "harnesses": ["claude-code"], "joined_ms": 1, "last_seen_ms": 2,
            "sessions": ["legacy-s1"]
        }]})
        .to_string(),
    )
    .unwrap();
    let hub = Hub::with_key(&hub_dir);
    let token = invite_token(&invite(&hub_dir, &hub, "laptop", "workshop"));
    let r = post(
        hub.port,
        "/api/v1/devices",
        &token,
        &greedy_card("legacy-laptop", &[]),
    );
    assert!(r.status < 300, "legacy re-pair: {} {}", r.status, r.body);
    let card = &r.json()["device"];
    assert_eq!(card["sessions"], json!(null), "{card}");
    assert_eq!(card["access"], "workshop");
    assert_eq!(get(hub.port, "/api/v1/meta", &token).status, 200);
    // Once it holds a credential, another invite cannot take it.
    let other = invite_token(&invite(&hub_dir, &hub, "tv", "workshop"));
    let r = post(
        hub.port,
        "/api/v1/devices",
        &other,
        &greedy_card("legacy-laptop", &[]),
    );
    assert_eq!(r.status, 409, "{}", r.body);
}

/// Second review, N3. No two devices are ever stamped with the same host:
/// a device that declares another card's host (or a `~` form) is told
/// apart by its id.
#[test]
fn a_device_cannot_take_another_devices_host() {
    let hub_dir = Dir::new("n3");
    let hub = Hub::with_key(&hub_dir);
    let p = hub.port;
    let first = invite_token(&invite(&hub_dir, &hub, "laptop", "workshop"));
    let mut card: Value = serde_json::from_str(&greedy_card("laptop01", &[])).unwrap();
    card["host"] = json!("laptop.local");
    assert_eq!(
        post(p, "/api/v1/devices", &first, &card.to_string()).status,
        201
    );
    let a_host = pairing_row(p, "laptop01")["host"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(a_host, "laptop.local", "a unique host is kept as declared");

    for (id, declared) in [
        ("intruso02", "LAPTOP.local".to_string()),
        ("intruso03", format!("{a_host}~laptop01")),
    ] {
        let token = invite_token(&invite(&hub_dir, &hub, id, "workshop"));
        let mut card: Value = serde_json::from_str(&greedy_card(id, &[])).unwrap();
        card["host"] = json!(declared);
        let r = post(p, "/api/v1/devices", &token, &card.to_string());
        assert_eq!(r.status, 201, "{}", r.body);
        let session = format!("{id}-s1");
        say_hello(p, &token, &session);
        let r = post(
            p,
            "/api/v1/events",
            &token,
            &note_event(&session, "hello", None),
        );
        assert_eq!(r.status, 201, "{}", r.body);
        let stamped = r.json()["event"]["host"].as_str().unwrap_or("").to_string();
        assert!(
            !stamped.eq_ignore_ascii_case(&a_host) && stamped.ends_with(&format!("~{id}")),
            "{declared} was stamped {stamped}"
        );
    }
}
