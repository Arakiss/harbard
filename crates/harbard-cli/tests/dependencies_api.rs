//! Real HTTP reads over a temporary dependency store; no console fixtures.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

const TOKEN: &str = "dependency-api-test-token";

struct Fixture {
    home: PathBuf,
    server: Child,
    port: u16,
}

impl Fixture {
    fn new(remote: bool) -> Self {
        Self::with_remote_credentials(remote, true)
    }

    fn with_remote_credentials(remote: bool, token: bool) -> Self {
        let home =
            std::env::temp_dir().join(format!("harbard-dependency-api-{}", ulid::Ulid::generate()));
        std::fs::create_dir_all(&home).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_harbard"));
        command
            .env_clear()
            .arg("--home")
            .arg(&home)
            .args(["serve", "--port", "0", "--token", TOKEN])
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        if remote {
            command.env("HARBARD_HUB", "http://127.0.0.1:1");
            if token {
                command.env("HARBARD_HUB_TOKEN", "remote-test-token");
            }
        }
        let mut server = command.spawn().unwrap();
        let mut startup = String::new();
        BufReader::new(server.stdout.take().unwrap())
            .read_line(&mut startup)
            .unwrap();
        let info: Value = serde_json::from_str(&startup).expect("server discovery JSON");
        Self {
            home,
            server,
            port: info["port"].as_u64().unwrap() as u16,
        }
    }

    fn command(&self, session: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_harbard"));
        command.env_clear().arg("--home").arg(&self.home).args([
            "--session",
            session,
            "--harness",
            "test",
            "--json",
        ]);
        command
    }

    fn cli(&self, session: &str, args: &[&str]) -> Value {
        let output = self.command(session).args(args).output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn open(&self) -> String {
        self.cli(
            "requester",
            &[
                "dependency",
                "open",
                "--to",
                "session:responder",
                "--condition",
                "Checks pass",
                "--resume",
                "Inspect the recorded result",
            ],
        )["id"]
            .as_str()
            .unwrap()
            .to_string()
    }

    fn request(&self, method: &str, authenticated: bool) -> (u16, String) {
        let mut socket = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = format!(
            "{method} /api/v1/dependencies HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n"
        );
        if authenticated {
            request.push_str(&format!("Authorization: Bearer {TOKEN}\r\n"));
        }
        request.push_str("\r\n");
        socket.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        socket.read_to_string(&mut response).unwrap();
        let (headers, body) = response.split_once("\r\n\r\n").unwrap();
        let status = headers
            .lines()
            .next()
            .unwrap()
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        (status, body.to_string())
    }

    fn dependencies(&self) -> Value {
        let (status, body) = self.request("GET", true);
        assert_eq!(status, 200, "{body}");
        serde_json::from_str(&body).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.server.kill();
        let _ = self.server.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

struct Waiter(Child);
impl Drop for Waiter {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn dependencies_require_authentication_and_reject_writes() {
    let fixture = Fixture::new(false);
    assert_eq!(fixture.request("GET", false).0, 401);
    assert_eq!(fixture.request("POST", true).0, 405);
    assert_eq!(
        fixture.dependencies(),
        json!({"available": true, "dependencies": []})
    );
    let (status, body) = fixture.request("HEAD", true);
    assert_eq!(status, 200);
    assert!(body.is_empty());
}

#[test]
fn dependency_reads_follow_real_resolution_and_consumption_without_mutating_them() {
    let fixture = Fixture::new(false);
    let id = fixture.open();
    let pending = fixture.dependencies();
    assert_eq!(pending["dependencies"][0]["id"], id);
    assert_eq!(pending["dependencies"][0]["state"], "pending");
    assert_eq!(pending["dependencies"][0]["active_cli_wait"], false);
    assert_eq!(pending["dependencies"][0]["automatic_resume"], false);
    assert_eq!(fixture.dependencies(), pending);
    fixture.cli(
        "responder",
        &[
            "dependency",
            "resolve",
            &id,
            "--outcome",
            "satisfied",
            "--evidence",
            "Verified checks",
        ],
    );
    let resolved = fixture.dependencies();
    assert_eq!(resolved["dependencies"][0]["state"], "satisfied");
    assert_eq!(resolved["dependencies"][0]["evidence"], "Verified checks");
    assert_eq!(resolved["dependencies"][0]["consumed"], false);
    fixture.cli("requester", &["dependency", "consume", &id]);
    assert_eq!(fixture.dependencies()["dependencies"][0]["consumed"], true);
    assert_eq!(
        fixture.dependencies()["dependencies"][0]["automatic_resume"],
        false
    );
}

#[test]
fn active_wait_reflects_a_real_listener_and_clears_after_process_exit() {
    let fixture = Fixture::new(false);
    let id = fixture.open();
    let mut waiter = Waiter(
        fixture
            .command("requester")
            .args(["await", &id, "--timeout-secs", "30", "--poll-ms", "10"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if fixture.dependencies()["dependencies"][0]["active_cli_wait"] == true {
            break;
        }
        assert!(Instant::now() < deadline, "listener never became active");
        std::thread::sleep(Duration::from_millis(10));
    }
    waiter.0.kill().unwrap();
    waiter.0.wait().unwrap();
    let after = fixture.dependencies();
    assert_eq!(after["dependencies"][0]["active_cli_wait"], false);
    assert_eq!(after["dependencies"][0]["state"], "pending");
    assert_eq!(after["dependencies"][0]["automatic_resume"], false);
}

#[test]
fn remote_pairing_reports_unavailability_instead_of_an_empty_local_result() {
    let fixture = Fixture::new(true);
    let (status, body) = fixture.request("GET", true);
    assert_eq!(status, 501);
    let result: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(result["available"], false);
    assert_eq!(result["code"], "dependency_remote_unavailable");
    assert_eq!(result["dependencies"], json!([]));
}

#[test]
fn a_configured_hub_without_credentials_cannot_fall_back_to_local_dependencies() {
    let fixture = Fixture::with_remote_credentials(true, false);
    let (status, body) = fixture.request("GET", true);
    assert_eq!(status, 501);
    let result: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(result["available"], false);
    assert_eq!(result["code"], "dependency_remote_unavailable");
}
