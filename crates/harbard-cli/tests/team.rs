//! The team view end to end: `GET /api/v1/team` over synthetic Claude Code
//! transcripts (`tests/fixtures/team`), and the two `doctor` checks behind it.
//! Every child runs with a cleared environment and a temp `HOME`, so the real
//! `~/.claude/projects` is never read.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static SEQ: AtomicU64 = AtomicU64::new(0);
const TOKEN: &str = "harbard-team-test-token";
const SID: &str = "0f0f0f0f-1111-2222-3333-444444444444";
/// 2026-11-06T10:31:00Z: 16 minutes after `dates` last acted.
const NOW_MS: u64 = 1_793_961_060_000;

struct Home(PathBuf);

impl Home {
    fn new() -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("harbard-team-it-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Home(d)
    }
    fn store(&self) -> PathBuf {
        self.0.join(".harbard")
    }
    fn session(&self) -> PathBuf {
        self.0.join(".claude/projects/-work-demo").join(SID)
    }
    /// Copy the synthetic transcripts in, optionally rewriting each line.
    fn install(&self, rewrite: impl Fn(&str) -> String) {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/team");
        let sub = self.session().join("subagents");
        std::fs::create_dir_all(&sub).unwrap();
        for name in [
            "agent-a1.jsonl",
            "agent-a1.meta.json",
            "agent-a2.jsonl",
            "agent-a2.meta.json",
        ] {
            let raw = std::fs::read_to_string(fixtures.join(name)).unwrap();
            let body = if name.ends_with(".jsonl") {
                raw.lines().map(&rewrite).collect::<Vec<_>>().join("\n") + "\n"
            } else {
                raw
            };
            std::fs::write(sub.join(name), body).unwrap();
        }
        std::fs::copy(
            fixtures.join("lead.jsonl"),
            self.0
                .join(".claude/projects/-work-demo")
                .join(format!("{SID}.jsonl")),
        )
        .unwrap();
    }
    fn cmd(&self) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_harbard"));
        c.env_clear()
            .env("HOME", &self.0)
            .env("PATH", "/usr/bin:/bin")
            .arg("--home")
            .arg(self.store());
        c
    }
    fn doctor(&self, serve_url: Option<&str>) -> (Output, serde_json::Value) {
        let mut c = self.cmd();
        if let Some(url) = serve_url {
            c.env("HARBARD_SERVE_URL", url);
        }
        let out = c.args(["doctor", "--json"]).output().unwrap();
        let json = serde_json::from_slice(&out.stdout).expect("doctor --json");
        (out, json)
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

impl Serve {
    fn start(home: &Home, extra: &[&str], now_ms: Option<u64>) -> Self {
        let mut c = home.cmd();
        if let Some(ms) = now_ms {
            c.env("HARBARD_NOW_MS", ms.to_string());
        }
        let mut child = c
            .args(["serve", "--port", "0"])
            .args(extra)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
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
    fn get(&self, path: &str) -> (u16, String) {
        let mut sock = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        sock.set_read_timeout(Some(Duration::from_secs(10)))
            .unwrap();
        write!(
            sock,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nAuthorization: Bearer {TOKEN}\r\n\r\n"
        )
        .unwrap();
        let mut raw = String::new();
        sock.read_to_string(&mut raw).unwrap();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap();
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, body.to_string())
    }
}

impl Drop for Serve {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn find<'a>(agents: &'a [serde_json::Value], name: &str) -> &'a serde_json::Value {
    agents
        .iter()
        .find(|a| a["name"] == name)
        .unwrap_or_else(|| panic!("{name} missing"))
}

fn snapshot(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        if e.path().is_file() {
            out.push((e.path(), std::fs::read(e.path()).unwrap()));
        }
    }
    out.sort();
    out
}

fn event_lines(store: &Path) -> usize {
    std::fs::read_dir(store.join("events"))
        .map(|it| {
            it.flatten()
                .map(|e| {
                    std::fs::read_to_string(e.path())
                        .unwrap_or_default()
                        .lines()
                        .count()
                })
                .sum()
        })
        .unwrap_or(0)
}

#[test]
fn serve_projects_teammates_from_transcripts_without_writing_anywhere() {
    let home = Home::new();
    home.install(str::to_string);
    let before = snapshot(&home.session().join("subagents"));
    let serve = Serve::start(&home, &["--token", TOKEN], Some(NOW_MS));

    let (status, body) = serve.get("/api/v1/team");
    assert_eq!(status, 200, "{body}");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["root_found"], true);
    let leads = v["leads"].as_array().unwrap();
    assert_eq!(leads.len(), 1, "{v}");
    let lead = &leads[0];
    assert_eq!(lead["session"], SID);
    assert_eq!(lead["lead_found"], true);
    let agents = lead["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 2);

    let notes = find(agents, "notes");
    assert_eq!(notes["kind"], "teammate");
    assert_eq!(notes["color"], "green");
    assert_eq!(notes["brief"], "Draft the release notes for the demo app.");
    assert_eq!(notes["state"], "idle");
    assert_eq!(notes["last_action_ms"], 1_793_959_805_000u64);
    assert_eq!(notes["checkouts"][0]["path"], "/work/demo");
    let labels: Vec<_> = notes["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["label"].as_str().unwrap_or("-").to_string())
        .collect();
    assert_eq!(labels, vec!["START", "BLOCKED"]);
    assert_eq!(notes["messages"][0]["report"], "/work/out/notes.md");

    let dates = find(agents, "dates");
    assert_eq!(dates["state"], "running");
    assert_eq!(dates["checkouts"][0]["writes"], 2);

    let mut kinds: Vec<(String, String)> = lead["warnings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| {
            (
                w["kind"].as_str().unwrap().to_string(),
                w["agents"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|a| a.as_str().unwrap())
                    .collect::<Vec<_>>()
                    .join(","),
            )
        })
        .collect();
    kinds.sort();
    let expect: Vec<(String, String)> = [
        ("blocked_unanswered", "a1"),
        ("quiet_running", "a2"),
        ("repeat", "a2"),
        ("two_writers", "a1,a2"),
        ("unlabeled", "a2"),
    ]
    .iter()
    .map(|(k, a)| (k.to_string(), a.to_string()))
    .collect();
    assert_eq!(kinds, expect, "{}", lead["warnings"]);

    // Nothing beyond first lines leaves the transcripts.
    for secret in [
        "SYNTHETIC-TOOL-OUTPUT",
        "SYNTHETIC-BODY",
        "Use the changelog",
        "Synthetic operator prompt",
    ] {
        assert!(!body.contains(secret), "{secret} leaked");
    }

    // A second pull reads only what was appended: the lead answers BLOCKED.
    let answer = "{\"type\":\"user\",\"timestamp\":\"2026-11-06T10:30:00.000Z\",\"cwd\":\"/work/demo\",\"gitBranch\":\"main\",\"message\":{\"role\":\"user\",\"content\":\"<teammate-message teammate_id=\\\"team-lead\\\" summary=\\\"s\\\">\\nUse 1.2.0.\\n</teammate-message>\"}}\n";
    std::fs::OpenOptions::new()
        .append(true)
        .open(home.session().join("subagents/agent-a1.jsonl"))
        .unwrap()
        .write_all(answer.as_bytes())
        .unwrap();
    let (_, body) = serve.get("/api/v1/team");
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    let warnings = v["leads"][0]["warnings"].as_array().unwrap();
    assert!(
        !warnings.iter().any(|w| w["kind"] == "blocked_unanswered"),
        "{warnings:?}"
    );
    assert_eq!(
        find(v["leads"][0]["agents"].as_array().unwrap(), "notes")["state"],
        "running"
    );

    // Read-only: no bus event, transcripts untouched apart from our append.
    drop(serve);
    assert_eq!(event_lines(&home.store()), 0);
    let after = snapshot(&home.session().join("subagents"));
    assert_eq!(after.len(), before.len());
    for ((p, a), (_, b)) in before.iter().zip(after.iter()) {
        if p.ends_with("agent-a1.jsonl") {
            assert!(b.starts_with(a));
        } else {
            assert_eq!(a, b, "{} changed", p.display());
        }
    }
}

#[test]
fn doctor_fails_when_recent_transcripts_yield_no_agent() {
    // Today's format: recognised.
    let home = Home::new();
    home.install(str::to_string);
    let (out, json) = home.doctor(None);
    assert_eq!(json["team"]["ok"], true, "{}", json["team"]);
    assert_eq!(json["team"]["recent_agents_seen"], 2);
    assert!(out.status.success(), "{}", json);

    // The record type moved to another key: written today, nothing recognised.
    let home = Home::new();
    home.install(|line| line.replacen("\"type\":", "\"kind\":", 1));
    let (out, json) = home.doctor(None);
    assert_eq!(json["team"]["ok"], false, "{}", json["team"]);
    assert_eq!(json["team"]["recent_subagent_files"], 4);
    assert_eq!(json["team"]["recent_agents_seen"], 0);
    assert!(!out.status.success());
    let text = home.cmd().arg("doctor").output().unwrap();
    assert!(!text.status.success());
    assert!(
        String::from_utf8_lossy(&text.stdout).contains("FORMAT CHANGED"),
        "{}",
        String::from_utf8_lossy(&text.stdout)
    );

    // No Claude Code transcripts at all is not a failure.
    let home = Home::new();
    let (out, json) = home.doctor(None);
    assert_eq!(json["team"]["recent_subagent_files"], 0);
    assert!(out.status.success(), "{json}");
}

#[test]
fn doctor_fails_when_serve_does_not_answer() {
    let home = Home::new();
    // A port nobody listens on.
    let free = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let (out, json) = home.doctor(Some(&format!("http://127.0.0.1:{free}")));
    assert_eq!(json["serve"]["probed"], true);
    assert_eq!(json["serve"]["ok"], false, "{}", json["serve"]);
    assert!(!out.status.success());

    // serve up, with the token it created in the store: answering.
    let serve = Serve::start(&home, &[], None);
    let (out, json) = home.doctor(Some(&format!("http://127.0.0.1:{}", serve.port)));
    assert_eq!(json["serve"]["ok"], true, "{}", json["serve"]);
    assert_eq!(json["serve"]["status"], 200);
    assert!(out.status.success(), "{json}");

    // Not installed and not named: not probed, not failed.
    drop(serve);
    let (out, json) = home.doctor(None);
    assert_eq!(json["serve"]["probed"], false);
    assert!(out.status.success(), "{json}");
}
