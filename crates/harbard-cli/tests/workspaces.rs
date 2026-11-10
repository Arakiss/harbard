//! Workspaces end to end: real checkouts with real `origin` remotes, the
//! real binary, a fresh `--home`. The registry is seeded from the history,
//! repositories are placed by their origin's owner, and a move by hand wins.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

struct Lab {
    root: PathBuf,
    home: PathBuf,
    gh: PathBuf,
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Lab {
    fn new() -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("harbard-ws-it-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let root = std::fs::canonicalize(&root).unwrap();
        let home = root.join("home");
        let gh = root.join("gh");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&gh).unwrap();
        std::fs::write(
            gh.join("hosts.yml"),
            "github.com:\n    git_protocol: ssh\n    user: me\n",
        )
        .unwrap();
        Lab { root, home, gh }
    }

    /// A checkout at `name` whose origin is `url` (or none).
    fn repo(&self, name: &str, url: Option<&str>) -> String {
        let dir = self.root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q"]);
        if let Some(url) = url {
            git(&dir, &["remote", "add", "origin", url]);
        }
        dir.to_string_lossy().into_owned()
    }

    fn run(&self, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_harbard"));
        cmd.env_clear();
        cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
        cmd.env("GH_CONFIG_DIR", &self.gh);
        cmd.args(args);
        cmd.arg("--home").arg(&self.home);
        cmd.output().expect("spawn harbard")
    }

    fn ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "{args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn json(&self, args: &[&str]) -> Value {
        let mut all = args.to_vec();
        all.push("--json");
        serde_json::from_str(&self.ok(&all)).expect("json")
    }

    /// A session announces itself from `cwd`.
    fn hello(&self, session: &str, cwd: &str) {
        self.ok(&[
            "hello",
            "--session",
            session,
            "--harness",
            "claude-code",
            "--cwd",
            cwd,
        ]);
    }
}

fn git(dir: &Path, args: &[&str]) {
    let st = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .status()
        .expect("git");
    assert!(st.success(), "git {args:?}");
}

fn ids(v: &Value) -> Vec<String> {
    v["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w["id"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn the_registry_is_seeded_from_history_and_placed_by_origin_owner() {
    let lab = Lab::new();
    let app = lab.repo("app", Some("git@github.com:Acme/app.git"));
    let tool = lab.repo("tool", Some("https://github.com/me/tool.git"));
    let local = lab.repo("local", None);
    lab.hello("s-app", &app);
    lab.hello("s-tool", &tool);
    lab.hello("s-local", &local);

    let v = lab.json(&["workspace", "ls"]);
    assert_eq!(ids(&v), vec!["personal", "acme"]);
    assert_eq!(v["default"], "personal");
    let ws = v["workspaces"].as_array().unwrap();
    assert_eq!(ws[0]["name"], "Personal");
    assert_eq!(ws[1]["name"], "Acme");
    assert_eq!(ws[1]["repos"], serde_json::json!([app]));
    let personal: Vec<&str> = ws[0]["repos"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    assert!(personal.contains(&tool.as_str()) && personal.contains(&local.as_str()));

    let w = lab.json(&["workspace", "where", &app]);
    assert_eq!(w["workspace"], "acme");
    assert_eq!(w["reason"], "origin owner Acme");

    // A move by hand wins over the rule, and a reset gives it back.
    lab.ok(&["workspace", "move", &local, "acme"]);
    assert_eq!(
        lab.json(&["workspace", "where", &local])["workspace"],
        "acme"
    );
    lab.ok(&["workspace", "reset", &local]);
    assert_eq!(
        lab.json(&["workspace", "where", &local])["workspace"],
        "personal"
    );

    lab.ok(&["workspace", "rename", "acme", "Trabajo"]);
    lab.ok(&["workspace", "icon", "acme", "T"]);
    let v = lab.json(&["workspace"]);
    assert_eq!(v["workspaces"][1]["name"], "Trabajo");
    assert_eq!(v["workspaces"][1]["tile"], "T");

    // Refusals say why and change nothing.
    let out = lab.run(&["workspace", "move", &local, "ghost"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("no workspace has the id"));
    let out = lab.run(&["workspace", "remove", "personal"]);
    assert!(!out.status.success());
}

impl Lab {
    /// Run a hook verb as Claude Code does: hook JSON on stdin.
    fn hook(&self, args: &[&str], session: &str, cwd: &str) -> Output {
        use std::io::Write;
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_harbard"));
        cmd.env_clear();
        cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
        cmd.env("GH_CONFIG_DIR", &self.gh);
        cmd.args(args)
            .args(["--hook", "claude-code", "--home"])
            .arg(&self.home)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = cmd.spawn().expect("spawn harbard");
        let payload = serde_json::json!({
            "session_id": session,
            "cwd": cwd,
            "transcript_path": "/tmp/t.jsonl",
            "hook_event_name": "Stop",
            "stop_hook_active": false,
        });
        child
            .stdin
            .take()
            .unwrap()
            .write_all(payload.to_string().as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn note_from(&self, session: &str, cwd: &str, kind: &str, to: &str, text: &str) -> Output {
        self.run(&[
            "note",
            "--kind",
            kind,
            "--to",
            to,
            "--session",
            session,
            "--harness",
            "codex",
            "--cwd",
            cwd,
            text,
        ])
    }
}

fn text(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

/// The hooks, through the real binary: `brief`, the inline channel, `inbox`,
/// `watch` and `stop-check` hand a session its own workspace's notes and
/// nothing from the other one, in both directions.
/// Two workspaces with two sessions each; a2 and b1 share the `ops`
/// channel and the `claude-code` harness. Each side writes an ask to both.
fn two_sided() -> (Lab, String, String) {
    let lab = Lab::new();
    let app = lab.repo("app", Some("git@github.com:Acme/app.git"));
    let tool = lab.repo("tool", Some("git@github.com:me/tool.git"));
    let tool2 = lab.repo("tool2", Some("git@github.com:me/tool2.git"));
    for (s, cwd) in [("a1", &app), ("a2", &app), ("b1", &tool), ("b2", &tool2)] {
        lab.hello(s, cwd);
    }
    lab.ok(&["workspace", "ls"]);
    for (s, cwd) in [("a2", &app), ("b1", &tool)] {
        lab.ok(&[
            "join",
            "ops",
            "--as",
            "session",
            "--session",
            s,
            "--harness",
            "claude-code",
            "--cwd",
            cwd,
        ]);
    }
    // A real end of turn for both readers, so `watch` treats what lands
    // next as arriving while they are idle.
    for (s, cwd) in [("a2", &app), ("b1", &tool)] {
        assert!(lab.hook(&["brief", "--touch"], s, cwd).status.success());
        assert!(lab.hook(&["stop-check"], s, cwd).status.success());
    }
    std::thread::sleep(std::time::Duration::from_millis(5));
    for (from, cwd, tag) in [("a1", &app, "ACME"), ("b2", &tool2, "MINE")] {
        for to in ["harness:claude-code", "channel:ops"] {
            let out = lab.note_from(from, cwd, "ask", to, &format!("{tag} via {to}"));
            assert!(out.status.success(), "{}", text(&out));
        }
    }
    (lab, app, tool)
}

/// The hooks, through the real binary: `brief`, the inline channel, `inbox`,
/// `watch` and `stop-check` hand a session its own workspace's notes and
/// nothing from the other one, in both directions. Each verb reads a fresh
/// hub: one verb's delivery mark would otherwise hide the note from the next.
#[test]
fn every_hook_keeps_to_its_workspace_in_both_directions() {
    let verbs: [(&str, &[&str]); 5] = [
        ("brief --inline", &["brief", "--inline"]),
        ("brief --touch", &["brief", "--touch"]),
        ("inbox", &["inbox", "--json"]),
        ("stop-check", &["stop-check"]),
        (
            "watch",
            &[
                "watch",
                "--lifetime-secs",
                "2",
                "--poll-ms",
                "50",
                "--coalesce-ms",
                "0",
            ],
        ),
    ];
    for (verb, args) in verbs {
        let (lab, app, tool) = two_sided();
        for (reader, cwd, own, other) in
            [("b1", &tool, "MINE", "ACME"), ("a2", &app, "ACME", "MINE")]
        {
            let said = text(&lab.hook(args, reader, cwd));
            assert!(
                !said.contains(other),
                "{verb} for {reader} carried the other workspace: {said}"
            );
            assert!(
                said.contains(own),
                "{verb} for {reader} silenced its own workspace: {said}"
            );
        }
    }
}

/// A note to a session or a repository of the other workspace is refused
/// at write, with the reason, instead of getting a receipt nobody can read.
#[test]
fn a_note_to_the_other_workspace_is_refused_with_the_reason() {
    let lab = Lab::new();
    let app = lab.repo("app", Some("git@github.com:Acme/app.git"));
    let tool = lab.repo("tool", Some("git@github.com:me/tool.git"));
    lab.hello("a1", &app);
    lab.hello("b1", &tool);
    lab.ok(&["workspace", "rename", "acme", "Trabajo"]);
    for to in ["session:b1".to_string(), format!("repo:{tool}")] {
        let out = lab.note_from("a1", &app, "warning", &to, "over the wall");
        assert!(!out.status.success(), "{to}: {}", text(&out));
        let said = text(&out);
        assert!(said.contains("notes do not cross workspaces"), "{said}");
        assert!(
            said.contains("\"Personal\"") && said.contains("\"Trabajo\""),
            "{said}"
        );
    }
    // Inside the workspace it is written as before.
    lab.hello("a2", &app);
    let out = lab.note_from("a1", &app, "warning", "session:a2", "next door");
    assert!(out.status.success(), "{}", text(&out));
}

/// The serve API: the read names its workspace and carries the rail's tiles,
/// an unknown workspace is refused, and the operator's desk moves a
/// repository. Serve seeds the registry at start from the history.
#[test]
fn serve_seeds_selects_and_moves_through_the_api() {
    use std::io::{BufRead, BufReader, Read, Write};
    const TOKEN: &str = "ws-test-token";
    let lab = Lab::new();
    let app = lab.repo("app", Some("git@github.com:Acme/app.git"));
    let tool = lab.repo("tool", Some("git@github.com:me/tool.git"));
    lab.hello("a1", &app);
    lab.hello("b1", &tool);
    assert!(!lab.home.join("workspaces.json").exists());

    /// Kills serve however the test ends: a failed assertion must not leave
    /// it running.
    struct Reaped(std::process::Child);
    impl Drop for Reaped {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let mut child = Command::new(env!("CARGO_BIN_EXE_harbard"))
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("GH_CONFIG_DIR", &lab.gh)
        .arg("--home")
        .arg(&lab.home)
        .args(["serve", "--port", "0", "--token", TOKEN])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn serve");
    let stdout = child.stdout.take().unwrap();
    let _serve = Reaped(child);
    let mut line = String::new();
    BufReader::new(stdout).read_line(&mut line).unwrap();
    let port = serde_json::from_str::<Value>(line.trim()).unwrap()["port"]
        .as_u64()
        .unwrap() as u16;
    let call = |method: &str, path: &str, body: Option<Value>| -> (u16, Value) {
        let mut sock = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        sock.set_read_timeout(Some(std::time::Duration::from_secs(20)))
            .unwrap();
        let body = body.map(|b| b.to_string()).unwrap_or_default();
        let req = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nAuthorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        sock.write_all(req.as_bytes()).unwrap();
        let mut raw = String::new();
        sock.read_to_string(&mut raw).unwrap();
        let (head, body) = raw.split_once("\r\n\r\n").unwrap();
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        (status, serde_json::from_str(body).unwrap_or(Value::Null))
    };

    let (status, v) = call("GET", "/api/v1/workspace", None);
    assert_eq!(status, 200);
    assert_eq!(v["workspace"], "personal");
    let tiles: Vec<&str> = v["workspaces"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(tiles, vec!["personal", "acme"]);
    let (status, v) = call("GET", "/api/v1/workspace?ws=acme&only=activity", None);
    assert_eq!((status, v["workspace"].as_str()), (200, Some("acme")));
    // The strip's read: the whole hub, under its own name.
    let (status, v) = call("GET", "/api/v1/workspace?ws=all", None);
    assert_eq!((status, v["workspace"].as_str()), (200, Some("all")));
    let (status, v) = call("GET", "/api/v1/workspace?ws=ghost", None);
    assert_eq!(
        (status, v["error"].as_str()),
        (404, Some("unknown_workspace"))
    );

    let (status, v) = call("GET", "/api/v1/workspaces", None);
    assert_eq!(status, 200);
    let placed = |v: &Value, repo: &str| {
        v["repos"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["repo"] == repo)
            .map(|r| r["workspace"].as_str().unwrap().to_string())
    };
    assert_eq!(placed(&v, &app).as_deref(), Some("acme"));
    assert_eq!(placed(&v, &tool).as_deref(), Some("personal"));
    let (status, _) = call(
        "POST",
        "/api/v1/workspaces/move",
        Some(serde_json::json!({ "repo": tool, "workspace": "acme" })),
    );
    assert_eq!(status, 200);
    let (_, v) = call("GET", "/api/v1/workspaces", None);
    assert_eq!(placed(&v, &tool).as_deref(), Some("acme"));
    // The move holds for the bus at once: a note from app now reaches tool.
    assert_eq!(
        lab.json(&["workspace", "where", &tool])["workspace"],
        "acme"
    );
}

/// A running `harbard serve` on an ephemeral port, killed however the test
/// ends.
struct Served {
    child: std::process::Child,
    port: u16,
}

impl Drop for Served {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const SERVE_TOKEN: &str = "ws-test-token";

impl Lab {
    fn serve(&self) -> Served {
        use std::io::{BufRead, BufReader};
        let mut child = Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap_or_default())
            .env("GH_CONFIG_DIR", &self.gh)
            .arg("--home")
            .arg(&self.home)
            .args(["serve", "--port", "0", "--token", SERVE_TOKEN])
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("spawn serve");
        let stdout = child.stdout.take().unwrap();
        let mut served = Served { child, port: 0 };
        let mut line = String::new();
        BufReader::new(stdout).read_line(&mut line).unwrap();
        served.port = serde_json::from_str::<Value>(line.trim()).unwrap()["port"]
            .as_u64()
            .unwrap() as u16;
        served
    }
}

fn post(port: u16, path: &str, body: &Value) -> (u16, Value) {
    use std::io::{Read, Write};
    let mut sock = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    sock.set_read_timeout(Some(std::time::Duration::from_secs(20)))
        .unwrap();
    let body = body.to_string();
    let req = format!(
        "POST {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nAuthorization: Bearer {SERVE_TOKEN}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );
    sock.write_all(req.as_bytes()).unwrap();
    let mut raw = String::new();
    sock.read_to_string(&mut raw).unwrap();
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

/// `POST /api/v1/notes` to a Room is placed in the workspace it names, as
/// `/workspace/reply` places one: the Room's members in that workspace read
/// it, its members in the other workspace never do.
#[test]
fn an_operator_note_to_a_room_stays_in_the_workspace_it_names() {
    let lab = Lab::new();
    let app = lab.repo("app", Some("git@github.com:Acme/app.git"));
    let tool = lab.repo("tool", Some("git@github.com:me/tool.git"));
    for (s, cwd) in [("a1", &app), ("b1", &tool)] {
        lab.hello(s, cwd);
        lab.ok(&[
            "join",
            "ops",
            "--as",
            "session",
            "--session",
            s,
            "--harness",
            "claude-code",
            "--cwd",
            cwd,
        ]);
    }
    let serve = lab.serve();
    for (ws, text) in [("acme", "FOR ACME"), ("personal", "FOR PERSONAL")] {
        let (status, v) = post(
            serve.port,
            "/api/v1/notes",
            &serde_json::json!({
                "text": text,
                "to": { "kind": "channel", "value": "ops" },
                "workspace": ws,
            }),
        );
        assert_eq!(status, 201, "{v}");
        assert_eq!(v["event"]["cwd"], format!("workspace:{ws}"));
    }
    let (status, v) = post(
        serve.port,
        "/api/v1/notes",
        &serde_json::json!({
            "text": "nowhere",
            "to": { "kind": "channel", "value": "ops" },
            "workspace": "ghost",
        }),
    );
    assert_eq!(
        (status, v["error"].as_str()),
        (404, Some("unknown_workspace"))
    );

    for (reader, cwd, own, other) in [
        ("a1", &app, "FOR ACME", "FOR PERSONAL"),
        ("b1", &tool, "FOR PERSONAL", "FOR ACME"),
    ] {
        let said = text(&lab.hook(&["inbox", "--json"], reader, cwd));
        assert!(
            said.contains(own),
            "{reader} lost its own room note: {said}"
        );
        assert!(
            !said.contains(other),
            "{reader} read across workspaces: {said}"
        );
        assert!(!said.contains("nowhere"), "{said}");
    }
}
