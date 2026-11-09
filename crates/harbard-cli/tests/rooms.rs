//! Rooms, the lobby and agent feedback, through the real binary: reading a
//! room delivers nothing, the built-in rooms stay quiet and permanent, a room
//! keeps what is said in it, and a workspace's rooms stay in that workspace.

use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

const WRITER: &str = "11111111-1111-4111-8111-111111111111";
const READER: &str = "22222222-2222-4222-8222-222222222222";
const ROOMS_BUDGET: usize = 280;

struct Lab {
    root: PathBuf,
    home: PathBuf,
}

impl Drop for Lab {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Lab {
    fn new() -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("harbard-rooms-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let root = std::fs::canonicalize(&root).unwrap();
        let home = root.join("home");
        std::fs::create_dir_all(&home).unwrap();
        Lab { root, home }
    }

    fn command(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_harbard"));
        cmd.env_clear();
        cmd.env("PATH", std::env::var("PATH").unwrap_or_default());
        cmd.env("HOME", &self.home);
        cmd.env("GH_CONFIG_DIR", self.root.join("gh"));
        cmd.env("HARBARD_API", "http://127.0.0.1:9");
        cmd
    }

    fn as_session(&self, session: &str, cwd: &Path, args: &[&str]) -> Output {
        self.command()
            .args(args)
            .arg("--home")
            .arg(&self.home)
            .args(["--session", session, "--harness", "codex", "--cwd"])
            .arg(cwd)
            .output()
            .expect("spawn harbard")
    }

    fn run(&self, session: &str, args: &[&str]) -> Output {
        self.as_session(session, &self.root, args)
    }

    fn ok(&self, session: &str, args: &[&str]) -> String {
        let out = self.run(session, args);
        assert!(out.status.success(), "{args:?}: {}", said(&out));
        said(&out)
    }

    fn json(&self, session: &str, args: &[&str]) -> Value {
        let mut all = args.to_vec();
        all.push("--json");
        let out = self.run(session, &all);
        assert!(out.status.success(), "{args:?}: {}", said(&out));
        serde_json::from_slice(&out.stdout)
            .unwrap_or_else(|e| panic!("{args:?}: {e}: {}", said(&out)))
    }

    fn repo(&self, name: &str, url: &str) -> PathBuf {
        let dir = self.root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        for args in [vec!["init", "-q"], vec!["remote", "add", "origin", url]] {
            let status = Command::new("git")
                .current_dir(&dir)
                .args(&args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .status()
                .unwrap();
            assert!(status.success());
        }
        dir
    }
}

fn said(out: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn texts(view: &Value) -> Vec<String> {
    view["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["text"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn an_intro_lands_in_the_lobby_and_wakes_nobody_who_has_not_joined() {
    let lab = Lab::new();
    lab.ok(READER, &["hello"]);
    lab.ok(WRITER, &["hello"]);
    lab.ok(WRITER, &["intro", "I review the billing changes"]);
    let lobby = lab.json(READER, &["channel", "read", "lobby"]);
    assert_eq!(texts(&lobby), vec!["I review the billing changes"]);
    // Not joined: it is in the room, and nowhere near this session.
    assert!(!said(&lab.run(READER, &["inbox"])).contains("billing"));
    assert!(!said(&lab.run(READER, &["brief"])).contains("billing"));

    // Joined: the same quiet note reaches it, as any channel note does.
    lab.ok(READER, &["join", "lobby", "--as", "session"]);
    lab.ok(WRITER, &["intro", "I also keep the exporter"]);
    assert!(said(&lab.run(READER, &["brief"])).contains("exporter"));
}

#[test]
fn reading_a_room_advances_no_delivery_position() {
    let lab = Lab::new();
    lab.ok(READER, &["hello"]);
    lab.ok(WRITER, &["hello"]);
    lab.ok(READER, &["join", "commons", "--as", "session"]);
    lab.ok(
        WRITER,
        &["channel", "post", "commons", "tide tables are fun"],
    );
    for _ in 0..3 {
        let room = lab.json(READER, &["channel", "read", "commons"]);
        assert_eq!(texts(&room), vec!["tide tables are fun"]);
    }
    assert!(said(&lab.run(READER, &["brief"])).contains("tide tables"));
    // And delivering it does not take it out of the room.
    let room = lab.json(READER, &["channel", "read", "commons"]);
    assert_eq!(texts(&room).len(), 1);
}

#[test]
fn an_intro_over_the_cap_is_refused_before_anything_is_written() {
    let lab = Lab::new();
    lab.ok(WRITER, &["hello"]);
    let exact = "x".repeat(ROOMS_BUDGET);
    lab.ok(WRITER, &["intro", &exact]);
    let long = "x".repeat(ROOMS_BUDGET + 1);
    let out = lab.run(WRITER, &["intro", &long]);
    assert!(!out.status.success());
    assert!(said(&out).contains("at most 280"), "{}", said(&out));
    assert!(said(&out).contains("harbard intent"), "{}", said(&out));
    assert_eq!(
        texts(&lab.json(WRITER, &["channel", "read", "lobby"])).len(),
        1
    );

    // `name --intro` is one request: an over-long intro leaves no name.
    let out = lab.run(READER, &["name", "Ottoline", "--intro", &long]);
    assert!(!out.status.success());
    let named = lab.json(READER, &["name", "Ottoline", "--intro", "I read the room"]);
    assert_eq!(named["changed"], true);
    assert_eq!(
        texts(&lab.json(READER, &["channel", "read", "lobby"])).len(),
        2
    );
}

#[test]
fn feedback_carries_its_context_and_lists_by_kind() {
    let lab = Lab::new();
    lab.ok(WRITER, &["hello"]);
    lab.ok(
        WRITER,
        &[
            "feedback",
            "--kind",
            "bug",
            "--verb",
            "inbox",
            "inbox printed nothing",
        ],
    );
    lab.ok(WRITER, &["feedback", "--kind", "idea", "group the help"]);

    let room = lab.json(READER, &["channel", "read", "feedback"]);
    let first = &texts(&room)[0];
    let mut lines = first.lines();
    assert_eq!(lines.next(), Some("[bug] inbox printed nothing"));
    assert_eq!(
        lines.next().map(str::to_string),
        Some(format!("harbard {}", env!("CARGO_PKG_VERSION")))
    );
    assert_eq!(lines.next(), Some("verb: inbox"));

    let bugs = lab.json(READER, &["feedback", "list", "--kind", "bug"]);
    let rows = bugs["feedback"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["kind"], "bug");
    assert_eq!(rows[0]["verb"], "inbox");
    assert_eq!(rows[0]["harness"], "codex");
    assert_eq!(rows[0]["session"], WRITER);
    let all = lab.json(READER, &["feedback", "list"]);
    assert_eq!(all["feedback"].as_array().unwrap().len(), 2);
    assert_eq!(all["feedback"][0]["kind"], "idea", "newest first");
}

/// Written to this machine's store even when a hub is configured: `feedback`
/// sends nothing anywhere.
#[test]
fn feedback_stays_on_this_machine_when_a_hub_is_configured() {
    let lab = Lab::new();
    let dead_hub = |args: &[&str]| {
        lab.command()
            .env("HARBARD_HUB", "http://127.0.0.1:9")
            .env("HARBARD_HUB_TOKEN", "unused")
            .args(args)
            .arg("--home")
            .arg(&lab.home)
            .args(["--session", WRITER, "--harness", "codex", "--cwd"])
            .arg(&lab.root)
            .output()
            .unwrap()
    };
    let out = dead_hub(&["feedback", "--kind", "idea", "keep this local"]);
    assert!(out.status.success(), "{}", said(&out));
    let listed = lab.json(READER, &["feedback", "list"]);
    assert_eq!(listed["feedback"].as_array().unwrap().len(), 1);
    // A note through the same dead hub fails, so the check above meant it.
    assert!(!dead_hub(&["note", "goes to the hub"]).status.success());
}

#[test]
fn a_rooms_changes_stay_on_the_log_after_it_is_deleted() {
    let lab = Lab::new();
    lab.ok(WRITER, &["hello"]);
    lab.ok(WRITER, &["channel", "create", "standup"]);
    lab.ok(WRITER, &["leave", "standup", "--as", "session"]);
    lab.ok(WRITER, &["channel", "archive", "standup"]);
    lab.ok(WRITER, &["channel", "delete", "standup"]);
    let log: String = std::fs::read_dir(lab.home.join("events"))
        .unwrap()
        .map(|entry| std::fs::read_to_string(entry.unwrap().path()).unwrap())
        .collect();
    let actions: Vec<&str> = ["create", "archive", "delete"]
        .into_iter()
        .filter(|action| {
            log.lines().any(|line| {
                line.contains("channel_lifecycle")
                    && line.contains("standup")
                    && line.contains(action)
            })
        })
        .collect();
    assert_eq!(actions, vec!["create", "archive", "delete"], "{log}");
}

#[test]
fn the_built_in_rooms_are_permanent_and_take_only_fyi() {
    let lab = Lab::new();
    lab.ok(WRITER, &["hello"]);
    for room in ["lobby", "commons", "feedback"] {
        for verb in ["archive", "delete"] {
            let out = lab.run(WRITER, &["channel", verb, room]);
            assert!(!out.status.success(), "{verb} {room}");
            assert!(said(&out).contains("built in"), "{}", said(&out));
        }
    }
    let listed = lab.json(WRITER, &["channel", "list"]);
    let names: Vec<&str> = listed["rooms"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, vec!["lobby", "commons", "feedback"]);

    let out = lab.run(
        WRITER,
        &["note", "--kind", "ask", "--to", "channel:lobby", "who?"],
    );
    assert!(!out.status.success());
    assert!(said(&out).contains("session:<id>"), "{}", said(&out));
    // No member is no refusal for a built-in room, and it is not reported
    // as a note that nobody will read.
    let out = lab.run(WRITER, &["note", "--to", "channel:commons", "hello"]);
    assert!(out.status.success(), "{}", said(&out));
    assert!(!said(&out).contains("nobody will read"), "{}", said(&out));
}

#[test]
fn a_room_keeps_what_was_said_after_it_is_archived_and_deleted() {
    let lab = Lab::new();
    lab.ok(WRITER, &["hello"]);
    lab.ok(
        WRITER,
        &[
            "channel",
            "create",
            "deploy-train",
            "--purpose",
            "who is shipping",
        ],
    );
    lab.ok(WRITER, &["channel", "post", "deploy-train", "first"]);
    let out = lab.run(READER, &["channel", "delete", "deploy-train"]);
    assert!(!out.status.success());
    assert!(said(&out).contains("channel archive"), "{}", said(&out));

    // The author need not leave first; somebody else's subscription stops it.
    lab.ok(READER, &["join", "deploy-train", "--as", "session"]);
    let out = lab.run(WRITER, &["channel", "archive", "deploy-train"]);
    assert!(!out.status.success());
    assert!(said(&out).contains("harbard leave"), "{}", said(&out));
    lab.ok(READER, &["leave", "deploy-train", "--as", "session"]);
    lab.ok(WRITER, &["channel", "archive", "deploy-train"]);

    let out = lab.run(WRITER, &["channel", "post", "deploy-train", "late"]);
    assert!(!out.status.success());
    assert!(said(&out).contains("channel unarchive"), "{}", said(&out));
    lab.ok(WRITER, &["channel", "delete", "deploy-train"]);
    let room = lab.json(READER, &["channel", "read", "deploy-train"]);
    assert_eq!(texts(&room), vec!["first"]);
}

#[test]
fn a_room_pages_from_the_newest_back() {
    let lab = Lab::new();
    lab.ok(WRITER, &["hello"]);
    for word in ["one", "two", "three"] {
        lab.ok(WRITER, &["channel", "post", "commons", word]);
        std::thread::sleep(std::time::Duration::from_millis(3));
    }
    let page = lab.json(WRITER, &["channel", "read", "commons", "--limit", "2"]);
    assert_eq!(texts(&page), vec!["two", "three"]);
    let before = page["next_before"].as_str().unwrap();
    let older = lab.json(
        WRITER,
        &[
            "channel", "read", "commons", "--limit", "2", "--before", before,
        ],
    );
    assert_eq!(texts(&older), vec!["one"]);
    assert_eq!(older["next_before"], Value::Null);
}

#[test]
fn what_is_said_in_a_room_stays_in_its_workspace() {
    let lab = Lab::new();
    let acme = lab.repo("app", "git@github.com:Acme/app.git");
    let mine = lab.repo("tool", "git@github.com:me/tool.git");
    std::fs::create_dir_all(lab.root.join("gh")).unwrap();
    std::fs::write(
        lab.root.join("gh/hosts.yml"),
        "github.com:\n    git_protocol: ssh\n    user: me\n",
    )
    .unwrap();
    for (session, cwd) in [(WRITER, &acme), (READER, &mine)] {
        let out = lab.as_session(session, cwd, &["hello"]);
        assert!(out.status.success(), "{}", said(&out));
    }
    assert!(lab.run(WRITER, &["workspace", "ls"]).status.success());
    let post = |session: &str, cwd: &Path, text: &str| {
        let out = lab.as_session(session, cwd, &["intro", text]);
        assert!(out.status.success(), "{}", said(&out));
    };
    post(WRITER, &acme, "from acme");
    post(READER, &mine, "from mine");

    let read = |session: &str, cwd: &Path| {
        let out = lab.as_session(session, cwd, &["channel", "read", "lobby", "--json"]);
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    assert_eq!(texts(&read(WRITER, &acme)), vec!["from acme"]);
    assert_eq!(texts(&read(READER, &mine)), vec!["from mine"]);
}

#[test]
fn the_machine_interface_reaches_the_new_verbs() {
    let lab = Lab::new();
    lab.ok(WRITER, &["hello"]);
    let call = |operation: &str, input: &str| {
        let out = lab.run(WRITER, &["call", operation, "--input", input]);
        assert!(out.status.success(), "{operation}: {}", said(&out));
        serde_json::from_slice::<Value>(&out.stdout).unwrap()
    };
    call("intro", r#"{"text":"I keep the exporter"}"#);
    call(
        "feedback",
        r#"{"text":"the card is good","kind":"praise","verb":"protocol"}"#,
    );
    call("channel.post", r#"{"name":"commons","text":"hello"}"#);

    let read = call("channel.read", r#"{"name":"lobby","limit":5}"#);
    assert_eq!(read["ok"], true);
    assert_eq!(read["data"]["messages"][0]["text"], "I keep the exporter");
    let list = call("feedback.list", r#"{"kind":"praise"}"#);
    assert_eq!(list["data"]["feedback"][0]["verb"], "protocol");
}

/// The report line is for a fault in harbard, so a refusal must not carry it.
#[test]
fn only_a_real_failure_offers_the_report_line() {
    let lab = Lab::new();
    lab.ok(WRITER, &["hello"]);
    let refused = lab.run(WRITER, &["channel", "archive", "lobby"]);
    assert!(!refused.status.success());
    assert!(
        !said(&refused).contains("harbard feedback"),
        "{}",
        said(&refused)
    );
    assert!(lab.run(WRITER, &["reply", "last", "x"]).status.code() != Some(0));

    // The store root is a file, so appending to it cannot work.
    let broken = lab.root.join("not-a-directory");
    std::fs::write(&broken, "").unwrap();
    let out = lab
        .command()
        .args(["note", "hello", "--home"])
        .arg(&broken)
        .args(["--session", WRITER, "--harness", "codex", "--cwd"])
        .arg(&lab.root)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(
        said(&out).contains("harbard feedback --kind bug --verb \"note\""),
        "{}",
        said(&out)
    );
}

#[test]
fn a_verb_shows_its_own_flags_before_the_global_ones() {
    let lab = Lab::new();
    let top = lab.ok(WRITER, &["--help"]);
    for heading in [
        "Start here:",
        "Everyday:",
        "Coordination:",
        "Delivery:",
        "Rooms:",
        "Setup and diagnostics:",
        "Global options:",
    ] {
        assert!(top.contains(heading), "{heading}\n{top}");
    }
    let help = lab.ok(WRITER, &["channel", "read", "--help"]);
    let own = help.find("--before").unwrap();
    let global = help.find("Global options:").unwrap();
    assert!(own < global, "{help}");
    assert!(help.contains("Example:"), "{help}");
}
