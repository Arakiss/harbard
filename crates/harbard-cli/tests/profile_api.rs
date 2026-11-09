//! `/api/v1/profile` and `/api/v1/profile/photo` end to end: the real serve on
//! a free port, a temp home, a throwaway token, a cleared environment.
//! Nothing here reads or writes the operator's own home.

use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static SEQ: AtomicU64 = AtomicU64::new(0);
const TOKEN: &str = "harbard-profile-test-token";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01";

struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let d =
            std::env::temp_dir().join(format!("harbard-profile-api-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Home(d)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Serve {
    child: Child,
    port: u16,
}

struct Reply {
    status: u16,
    head: String,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }
}

impl Serve {
    fn start(home: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .env("HOME", home)
            .arg("--home")
            .arg(home)
            .args(["serve", "--port", "0", "--token", TOKEN])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn serve");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut line)
            .unwrap();
        let v: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        Serve {
            child,
            port: v["port"].as_u64().unwrap() as u16,
        }
    }

    fn send(&self, method: &str, path: &str, kind: &str, body: &[u8], extra: &str) -> Reply {
        let mut sock = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let head = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: {kind}\r\nContent-Length: {}\r\n{extra}\r\n",
            body.len()
        );
        sock.write_all(head.as_bytes()).unwrap();
        // A refused upload may close before reading it all; that is the point.
        let _ = sock.write_all(body);
        let mut raw = Vec::new();
        let _ = sock.read_to_end(&mut raw);
        let at = raw
            .windows(4)
            .position(|w| w == b"\r\n\r\n")
            .expect("a response");
        let head = String::from_utf8_lossy(&raw[..at]).into_owned();
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        Reply {
            status,
            head,
            body: raw[at + 4..].to_vec(),
        }
    }

    fn get(&self, path: &str) -> Reply {
        self.send("GET", path, "application/json", b"", "")
    }

    fn patch(&self, body: serde_json::Value) -> Reply {
        self.send(
            "POST",
            "/api/v1/profile",
            "application/json",
            body.to_string().as_bytes(),
            "",
        )
    }

    fn photo(&self, bytes: &[u8]) -> Reply {
        self.send(
            "POST",
            "/api/v1/profile/photo",
            "application/octet-stream",
            bytes,
            "",
        )
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[test]
fn the_profile_is_stored_in_profile_json_and_survives_a_restart() {
    let home = Home::new();
    let serve = Serve::start(&home.0);
    let empty = serve.get("/api/v1/profile").json();
    assert_eq!(empty["quiet"], false, "{empty}");
    assert!(empty["status"].is_null());
    assert_eq!(empty["limits"]["context"], 1500);
    let r = serve.patch(json!({
        "name": "Dana",
        "context": "I build harbard.",
        "status": {"emoji": "🎯", "text": "Concentrado"},
        "dnd": {"until_ms": now_ms() + 3_600_000},
    }));
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(r.json()["quiet"], true);
    let file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.0.join("profile.json")).unwrap())
            .unwrap();
    assert_eq!(file["context"], "I build harbard.");
    assert!(
        home.0.join("workspace.json.lock").exists(),
        "written under the desk lock"
    );
    drop(serve);
    let again = Serve::start(&home.0);
    let back = again.get("/api/v1/profile").json();
    assert_eq!(back["profile"]["name"], "Dana");
    assert_eq!(back["status"]["text"], "Concentrado");
    assert_eq!(back["quiet"], true);
}

#[test]
fn the_name_in_profile_is_the_one_the_console_signs_with() {
    let home = Home::new();
    let serve = Serve::start(&home.0);
    let before = serve.get("/api/v1/workspace").json();
    assert_eq!(before["operator"], "Operator", "{before}");
    let r = serve.patch(json!({"name": "Adonay"}));
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let after = serve.get("/api/v1/workspace").json();
    assert_eq!(after["operator"], "Adonay");
    assert_eq!(after["operator_initials"], "AD");
    assert_ne!(before["rev"], after["rev"], "the console must notice");
}

#[test]
fn a_refused_field_says_which_and_writes_nothing() {
    let home = Home::new();
    let serve = Serve::start(&home.0);
    let r = serve.patch(json!({"context": "x".repeat(1501)}));
    assert_eq!(r.status, 422);
    assert_eq!(
        (r.json()["error"].as_str(), r.json()["field"].as_str()),
        (Some("too_long"), Some("context"))
    );
    let r = serve.patch(json!({"agent": "a1"}));
    assert_eq!(r.status, 422);
    assert!(!home.0.join("profile.json").exists(), "nothing written");
}

#[test]
fn a_satellite_or_a_foreign_origin_cannot_touch_the_profile() {
    let home = Home::new();
    let serve = Serve::start(&home.0);
    let body = json!({"name": "x"}).to_string();
    let sat = serve.send(
        "POST",
        "/api/v1/profile",
        "application/json",
        body.as_bytes(),
        "X-Harbard-Device: dev1\r\n",
    );
    assert_eq!(sat.status, 403);
    let foreign = serve.send(
        "POST",
        "/api/v1/profile",
        "application/json",
        body.as_bytes(),
        "Origin: https://evil.example\r\n",
    );
    assert_eq!(foreign.status, 403);
    let photo = serve.send(
        "POST",
        "/api/v1/profile/photo",
        "image/png",
        PNG,
        "X-Harbard-Device: dev1\r\n",
    );
    assert_eq!(photo.status, 403);
    assert!(!home.0.join("profile.json").exists());
    assert!(!home.0.join("profile").exists());
}

#[test]
fn a_photo_is_copied_into_the_profile_folder_and_served_back() {
    let home = Home::new();
    let serve = Serve::start(&home.0);
    let r = serve.photo(PNG);
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    assert_eq!(
        std::fs::read(home.0.join("profile/photo.png")).unwrap(),
        PNG
    );
    let view = serve.get("/api/v1/profile").json();
    assert_eq!(view["profile"]["photo"], "photo.png");
    assert!(view["profile"]["photo_rev"].as_u64().unwrap() > 0);
    let got = serve.get("/api/v1/profile/photo");
    assert_eq!(got.status, 200);
    assert_eq!(got.body, PNG);
    let head = got.head.to_ascii_lowercase();
    assert!(head.contains("content-type: image/png"), "{head}");
    assert!(head.contains("x-content-type-options: nosniff"), "{head}");
    // And it can be removed.
    assert_eq!(serve.patch(json!({"photo": null})).status, 200);
    assert!(!home.0.join("profile/photo.png").exists());
    assert_eq!(serve.get("/api/v1/profile/photo").status, 404);
}

#[test]
fn a_photo_of_the_wrong_type_or_size_is_refused() {
    let home = Home::new();
    let serve = Serve::start(&home.0);
    let svg = serve.photo(b"<svg xmlns='http://www.w3.org/2000/svg'/>");
    assert_eq!(
        (svg.status, svg.json()["error"].as_str()),
        (415, Some("unsupported_type"))
    );
    let mut big = PNG.to_vec();
    big.resize(2 * 1024 * 1024 + 1, 0);
    let r = serve.photo(&big);
    assert_eq!(r.status, 413);
    assert!(!home.0.join("profile/photo.png").exists());
    assert!(serve.get("/api/v1/profile").json()["profile"]["photo"].is_null());
}

#[cfg(unix)]
#[test]
fn a_symlinked_profile_folder_is_refused() {
    let home = Home::new();
    let elsewhere = Home::new();
    std::os::unix::fs::symlink(&elsewhere.0, home.0.join("profile")).unwrap();
    let serve = Serve::start(&home.0);
    let r = serve.photo(PNG);
    assert_eq!(
        (r.status, r.json()["error"].as_str()),
        (409, Some("symlink"))
    );
    assert!(std::fs::read_dir(&elsewhere.0).unwrap().next().is_none());
}

#[test]
fn a_coordination_state_is_stored_and_answers_with_its_dock_rules() {
    let home = Home::new();
    let serve = Serve::start(&home.0);
    assert_eq!(serve.get("/api/v1/profile").json()["state"], "available");
    let r = serve.patch(json!({"state": {"id": "swamped"}}));
    assert_eq!(r.status, 200, "{}", String::from_utf8_lossy(&r.body));
    let v = r.json();
    assert_eq!(v["state"], "swamped");
    assert_eq!(
        v["dock"],
        json!({"bounce": false, "alerts": false, "one_shots": false})
    );
    let file: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.0.join("profile.json")).unwrap())
            .unwrap();
    assert_eq!(file["state"]["id"], "swamped");
    let r = serve.patch(json!({"state": {"id": "focused"}}));
    assert_eq!(r.json()["dock"]["bounce"], false);
    assert_eq!(r.json()["dock"]["alerts"], true);
    assert_eq!(serve.patch(json!({"state": {"id": "asleep"}})).status, 422);
    assert_eq!(
        serve.patch(json!({"state": {"id": "available"}})).json()["state"],
        "available"
    );
}

#[test]
fn leaving_focused_delivers_one_digest_of_the_held_notes() {
    let home = Home::new();
    let serve = Serve::start(&home.0);
    assert_eq!(serve.patch(json!({"state": {"id": "focused"}})).status, 200);
    // An agent writes to the human while they are Focused.
    for (kind, text) in [
        ("fyi", "Themes installed"),
        ("ask", "Large or small?"),
        ("warning", "The branch has drifted"),
    ] {
        let o = Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .env("HOME", &home.0)
            .env("HARBARD_API", "http://127.0.0.1:9")
            .args([
                "--home",
                home.0.to_str().unwrap(),
                "note",
                "--session",
                "s-a",
                "--harness",
                "claude-code",
                "--cwd",
                "/tmp",
                "--to",
                "harness:human",
                "--kind",
                kind,
                text,
            ])
            .output()
            .unwrap();
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    let so_far = serve.get("/api/v1/digests/pending").json();
    assert_eq!(so_far["state"], "focused");
    assert_eq!(so_far["items"].as_array().unwrap().len(), 2, "{so_far}");
    let r = serve.patch(json!({"state": {"id": "available"}}));
    assert_eq!(r.status, 200);
    let digest = &r.json()["digest"];
    assert_eq!(digest["state"], "focused");
    let texts: Vec<&str> = digest["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["text"].as_str().unwrap())
        .collect();
    assert_eq!(
        texts,
        vec!["Themes installed", "Large or small?"],
        "the warning was not held"
    );
    let list = serve.get("/api/v1/digests").json();
    assert_eq!(list["items"].as_array().unwrap().len(), 1);
    let id = list["items"][0]["id"].as_str().unwrap().to_string();
    let read = serve.send(
        "POST",
        "/api/v1/digests/read",
        "application/json",
        json!({"id": id}).to_string().as_bytes(),
        "",
    );
    assert_eq!(read.status, 200);
    assert_eq!(
        serve.get("/api/v1/digests").json()["items"][0]["read"],
        true
    );
    // A satellite cannot mark it.
    let sat = serve.send(
        "POST",
        "/api/v1/digests/read",
        "application/json",
        json!({"id": id}).to_string().as_bytes(),
        "X-Harbard-Device: d\r\n",
    );
    assert_eq!(sat.status, 403);
}
