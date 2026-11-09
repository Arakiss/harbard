//! `agents` and `owed`: the lead's own team, and what the lead owes, from the
//! CLI. Synthetic Claude Code transcripts (`tests/fixtures/team`) under a temp
//! `HOME`, a cleared environment and a fixed clock, so the real
//! `~/.claude/projects` and `~/.harbard` are never read.
//!
//! A lead needs one view of its teammates and of what it owes: `harbard ls`
//! lists sessions, not teammates.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
/// The lead session the fixtures belong to.
const SID: &str = "0f0f0f0f-1111-2222-3333-444444444444";
const PEER: &str = "33333333-3333-4333-8333-333333333333";
/// 2026-11-06T10:31:00Z, as in `tests/team.rs`.
const NOW_MS: u64 = 1_793_961_060_000;
const BLOCKED: &str = "BLOCKED · notes · Which version number goes in the title?";

struct Home(PathBuf);

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Home {
    fn new() -> Self {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("harbard-agents-it-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Home(d)
    }

    fn store(&self) -> PathBuf {
        self.0.join(".harbard")
    }

    fn project(&self) -> PathBuf {
        self.0.join(".claude/projects/-work-demo")
    }

    /// Copy the transcripts in; `extra` lines are appended to agent a1's.
    fn install(&self, rewrite: impl Fn(&str) -> String, extra: &[String]) {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/team");
        let sub = self.project().join(SID).join("subagents");
        std::fs::create_dir_all(&sub).unwrap();
        for name in [
            "agent-a1.jsonl",
            "agent-a1.meta.json",
            "agent-a2.jsonl",
            "agent-a2.meta.json",
        ] {
            let raw = std::fs::read_to_string(fixtures.join(name)).unwrap();
            let mut body = if name.ends_with(".jsonl") {
                raw.lines().map(&rewrite).collect::<Vec<_>>().join("\n") + "\n"
            } else {
                raw
            };
            if name == "agent-a1.jsonl" {
                for line in extra {
                    body.push_str(line);
                    body.push('\n');
                }
            }
            std::fs::write(sub.join(name), body).unwrap();
        }
        std::fs::copy(
            fixtures.join("lead.jsonl"),
            self.project().join(format!("{SID}.jsonl")),
        )
        .unwrap();
    }

    fn run_at(&self, now_ms: u64, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_harbard"))
            .env_clear()
            .env("HOME", &self.0)
            .env("PATH", "/usr/bin:/bin")
            .env("HARBARD_NOW_MS", now_ms.to_string())
            // A dead port: nothing here may reach a running serve.
            .env("HARBARD_API", "http://127.0.0.1:9")
            .args(args)
            .arg("--home")
            .arg(self.store())
            .args(["--cwd", self.0.to_str().unwrap()])
            .output()
            .unwrap()
    }

    fn lead(&self, args: &[&str]) -> Output {
        let mut all = args.to_vec();
        all.extend(["--session", SID, "--harness", "claude-code"]);
        self.run_at(NOW_MS, &all)
    }

    fn json(&self, args: &[&str]) -> serde_json::Value {
        let out = self.lead(args);
        assert!(out.status.success(), "{out:?}");
        serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {out:?}"))
    }

    /// The lead and a peer on the bus a minute before now.
    fn bus(&self) {
        for (session, harness) in [(SID, "claude-code"), (PEER, "codex")] {
            let out = self.run_at(
                NOW_MS - 60_000,
                &["hello", "--session", session, "--harness", harness],
            );
            assert!(out.status.success(), "{out:?}");
        }
    }

    fn peer_ask(&self, at_ms: u64, text: &str) -> String {
        let to = format!("session:{SID}");
        let out = self.run_at(
            at_ms,
            &[
                "note",
                text,
                "--kind",
                "ask",
                "--to",
                &to,
                "--json",
                "--session",
                PEER,
                "--harness",
                "codex",
            ],
        );
        assert!(out.status.success(), "{out:?}");
        let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
        v["id"].as_str().unwrap().to_string()
    }

    /// A note the way serve writes the operator's.
    fn operator_ask(&self, at_ms: u64, text: &str) -> String {
        let id = ulid::Ulid::from_parts(at_ms, 7).to_string();
        let note = serde_json::json!({
            "v": 1, "id": id, "ts": rfc3339(at_ms), "event": "note",
            "session": "operator@h", "harness": "human", "role": "operator",
            "host": "h", "cwd": "/work/demo", "text": text,
            "kind": "ask", "to": { "kind": "session", "value": SID },
        });
        let dir = self.store().join("events");
        let mut segments: Vec<PathBuf> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        segments.sort();
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(segments.last().unwrap())
            .unwrap();
        writeln!(file, "{note}").unwrap();
        id
    }
}

fn rfc3339(ms: u64) -> String {
    // 2026-11-06T10:MM:SS.mmmZ within the fixture hour.
    let base = 1_793_959_200_000; // 2026-11-06T10:00:00Z
    let off = ms - base;
    format!(
        "2026-11-06T10:{:02}:{:02}.{:03}Z",
        off / 60_000,
        (off / 1000) % 60,
        off % 1000
    )
}

/// A message from the lead to agent a1, the way Claude Code records it.
fn lead_line(ts: &str) -> String {
    serde_json::json!({
        "parentUuid": null, "isSidechain": true, "agentId": "a1", "type": "user",
        "uuid": format!("u-{ts}"), "timestamp": ts, "userType": "external",
        "entrypoint": "cli", "cwd": "/work/demo", "sessionId": SID,
        "version": "2.1.318", "gitBranch": "main",
        "message": { "role": "user", "content":
            "<teammate-message teammate_id=\"team-lead\" summary=\"s\">\nGood, close it.\n</teammate-message>" },
    })
    .to_string()
}

fn as_done(line: &str) -> String {
    line.replace(BLOCKED, "DONE · notes · The release notes are ready.")
}

fn find<'a>(rows: &'a [serde_json::Value], own: &str) -> &'a serde_json::Value {
    rows.iter()
        .find(|r| r["own_name"] == own)
        .unwrap_or_else(|| panic!("no agent {own} in {rows:?}"))
}

#[test]
fn agents_lists_the_leads_teammates_with_state_since_and_last_line() {
    let home = Home::new();
    home.install(str::to_string, &[]);
    let v = home.json(&["agents", "--json"]);
    let rows = v["agents"].as_array().expect("agents array");
    assert_eq!(rows.len(), 2, "{v}");
    let notes = find(rows, "notes");
    assert!(!notes["name"].as_str().unwrap().is_empty(), "{notes}");
    assert_eq!(notes["team"], "demo", "{notes}");
    assert_eq!(notes["state"], "blocked", "{notes}");
    assert!(notes["since_ms"].as_u64().unwrap() > 0, "{notes}");
    assert_eq!(
        notes["last_line"], "Which version number goes in the title?",
        "{notes}"
    );
    assert_eq!(notes["last_from"], "agent", "{notes}");
    assert_eq!(notes["done_waiting"], false, "{notes}");
    assert_eq!(find(rows, "dates")["done_waiting"], false);
}

#[test]
fn agents_says_when_a_done_waits_for_the_lead_and_stops_once_the_lead_answers() {
    let home = Home::new();
    home.install(as_done, &[]);
    let v = home.json(&["agents", "--json"]);
    let notes = find(v["agents"].as_array().unwrap(), "notes");
    assert_eq!(notes["done_waiting"], true, "{notes}");

    let answered = Home::new();
    answered.install(as_done, &[lead_line("2026-11-06T10:12:00.000Z")]);
    let v = answered.json(&["agents", "--json"]);
    let notes = find(v["agents"].as_array().unwrap(), "notes");
    assert_eq!(notes["done_waiting"], false, "{notes}");
}

#[test]
fn agents_shows_only_this_leads_team_unless_all() {
    let home = Home::new();
    home.install(str::to_string, &[]);
    let out = home.run_at(
        NOW_MS,
        &["agents", "--json", "--session", PEER, "--harness", "codex"],
    );
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["agents"].as_array().unwrap().len(), 0, "{v}");
    let out = home.run_at(
        NOW_MS,
        &[
            "agents",
            "--all",
            "--json",
            "--session",
            PEER,
            "--harness",
            "codex",
        ],
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(v["agents"].as_array().unwrap().len(), 2, "{v}");
}

#[test]
fn agents_prints_one_readable_line_per_agent() {
    let home = Home::new();
    home.install(as_done, &[]);
    let out = home.lead(&["agents"]);
    assert!(out.status.success(), "{out:?}");
    let text = String::from_utf8_lossy(&out.stdout);
    assert!(text.contains("DONE waiting"), "{text}");
    assert!(text.contains("The release notes are ready."), "{text}");
    assert!(text.contains("demo"), "{text}");
}

#[test]
fn agents_watch_redraws_until_stopped() {
    let home = Home::new();
    home.install(str::to_string, &[]);
    let out = home.lead(&[
        "agents",
        "--watch",
        "--json",
        "--interval-secs",
        "0",
        "--frames",
        "2",
    ]);
    assert!(out.status.success(), "{out:?}");
    let frames: Vec<serde_json::Value> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(frames.len(), 2, "one JSON line per frame");
    assert!(frames.iter().all(|f| f["agents"].is_array()));
}

#[test]
fn owed_orders_everything_the_lead_owes_with_the_command_that_settles_it() {
    let home = Home::new();
    home.install(str::to_string, &[]);
    home.bus();
    let peer = home.peer_ask(NOW_MS - 40_000, "Can you review my branch?");
    // To `dates`: a note to `notes` would answer its BLOCKED.
    let relay = home.operator_ask(NOW_MS - 30_000, "Para dates: corre la suite.\nto_agent: a2");
    let operator = home.operator_ask(NOW_MS - 20_000, "How is the team doing?");
    let v = home.json(&["owed", "--json"]);
    let items = v["items"].as_array().expect("items");
    let kinds: Vec<&str> = items.iter().map(|i| i["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["answer", "relay", "blocked", "answer"], "{v}");
    assert_eq!(items[0]["id"], operator);
    assert_eq!(items[0]["from"], "the operator");
    assert_eq!(
        items[0]["command"],
        format!("harbard reply {operator} \"…\"")
    );
    assert_eq!(items[1]["id"], relay);
    assert_eq!(items[1]["agent"], "dates");
    assert!(
        items[1]["command"]
            .as_str()
            .unwrap()
            .contains("SendMessage({to: \"dates\""),
        "{v}"
    );
    assert_eq!(items[2]["agent"], "notes");
    assert_eq!(items[2]["text"], "Which version number goes in the title?");
    assert_eq!(items[3]["id"], peer);
    assert_eq!(v["total"], 4);
}

#[test]
fn owed_lists_a_done_waiting_for_the_lead_and_is_bounded() {
    let home = Home::new();
    home.install(as_done, &[]);
    home.bus();
    for i in 0..3 {
        home.peer_ask(NOW_MS - 50_000 + i, &format!("Question {i}?"));
    }
    let v = home.json(&["owed", "--json", "--limit", "2"]);
    assert_eq!(v["total"], 4, "{v}");
    assert_eq!(v["items"].as_array().unwrap().len(), 2, "{v}");
    let all = home.json(&["owed", "--json"]);
    let last = all["items"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["kind"], "done", "{all}");
    assert_eq!(last["agent"], "notes");
    assert!(
        last["command"]
            .as_str()
            .unwrap()
            .starts_with("SendMessage({to: \"notes\""),
        "{last}"
    );
    let human = home.lead(&["owed", "--limit", "2"]);
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("and 2 more"), "{text}");
}

#[test]
fn owed_with_nothing_owed_says_so() {
    let home = Home::new();
    home.bus();
    let v = home.json(&["owed", "--json"]);
    assert_eq!(v["total"], 0, "{v}");
    let out = home.lead(&["owed"]);
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Nothing owed"));
}

#[test]
fn agents_and_owed_are_machine_operations() {
    let home = Home::new();
    home.install(str::to_string, &[]);
    home.bus();
    for op in ["agents", "owed"] {
        let v = home.json(&["call", op]);
        assert_eq!(v["v"], "harbard/cli/v1", "{v}");
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(v["operation"], op, "{v}");
    }
}

// ---------------------------------------------------------------------------
// Held notes and `relay`
// ---------------------------------------------------------------------------

/// The operator in a state that holds notes to them since ten minutes ago.
fn state(home: &Home, id: &str) {
    std::fs::create_dir_all(home.store()).unwrap();
    let p = serde_json::json!({ "state": { "id": id, "since_ms": NOW_MS - 600_000 } });
    std::fs::write(home.store().join("profile.json"), p.to_string()).unwrap();
}

fn lead_note_to_human(home: &Home, at_ms: u64, kind: &str, text: &str) -> String {
    let out = home.run_at(
        at_ms,
        &[
            "note",
            text,
            "--kind",
            kind,
            "--to",
            "harness:human",
            "--json",
            "--session",
            SID,
            "--harness",
            "claude-code",
        ],
    );
    assert!(out.status.success(), "{out:?}");
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    v["id"].as_str().unwrap().to_string()
}

#[test]
fn owed_lists_the_leads_notes_the_operators_state_holds_back() {
    let home = Home::new();
    home.bus();
    state(&home, "focused");
    let held = lead_note_to_human(&home, NOW_MS - 30_000, "fyi", "The team is fine.");
    let _warning = lead_note_to_human(&home, NOW_MS - 20_000, "warning", "CI is down.");
    let v = home.json(&["owed", "--json"]);
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "a warning is not held: {v}");
    assert_eq!(items[0]["kind"], "held", "{v}");
    assert_eq!(items[0]["id"], held);
    let command = items[0]["command"].as_str().unwrap();
    assert!(
        command.contains("--kind warning --to harness:human"),
        "{command}"
    );
    assert!(command.contains(&format!("--ref {held}")), "{command}");

    // Away queues everything; nothing gets through, so nothing to run.
    state(&home, "away");
    let v = home.json(&["owed", "--json"]);
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 2, "{v}");
    assert!(items.iter().all(|i| i["kind"] == "held"), "{v}");
    assert!(
        items[0]["command"].as_str().unwrap().contains("digest"),
        "{v}"
    );
}

#[test]
fn relay_prints_each_pending_dm_ready_to_send() {
    let home = Home::new();
    home.install(str::to_string, &[]);
    home.bus();
    let id = home.operator_ask(NOW_MS - 30_000, "Para dates: corre la suite.\nto_agent: a2");
    let _peer = home.peer_ask(NOW_MS - 20_000, "Not a DM.");
    let v = home.json(&["relay", "--json"]);
    let items = v["items"].as_array().unwrap();
    assert_eq!(items.len(), 1, "{v}");
    let r = &items[0];
    assert_eq!(r["id"], id);
    assert_eq!(r["from"], "the operator");
    assert_eq!(r["agents"][0]["address"], "dates", "{v}");
    assert!(!r["agents"][0]["name"].as_str().unwrap().is_empty());
    assert_eq!(
        r["text"], "Para dates: corre la suite.",
        "fields are not sent"
    );
    assert_eq!(r["agents"][0]["send"]["to"], "dates");
    assert_eq!(
        r["agents"][0]["send"]["message"],
        "Para dates: corre la suite."
    );
    assert_eq!(r["sent"], format!("harbard relay --sent {id}"));

    let human = home.lead(&["relay"]);
    let text = String::from_utf8_lossy(&human.stdout);
    assert!(text.contains("SendMessage to dates"), "{text}");
    assert!(text.contains("   | Para dates: corre la suite."), "{text}");
    assert!(
        text.contains(&format!("harbard relay --sent {id}")),
        "{text}"
    );
}

#[test]
fn relay_sent_marks_it_relayed_and_settles_the_debt() {
    let home = Home::new();
    home.install(str::to_string, &[]);
    home.bus();
    let id = home.operator_ask(NOW_MS - 30_000, "Para dates: corre la suite.\nto_agent: a2");
    let out = home.lead(&["relay", "--sent", &id, "--json"]);
    assert!(out.status.success(), "{out:?}");
    let v = home.json(&["relay", "--json"]);
    assert_eq!(v["items"].as_array().unwrap().len(), 0, "{v}");
    let owed = home.json(&["owed", "--json"]);
    assert!(
        owed["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|i| i["kind"] != "relay"),
        "{owed}"
    );
}

#[test]
fn relay_sent_refuses_a_note_that_is_not_for_an_agent() {
    let home = Home::new();
    home.install(str::to_string, &[]);
    home.bus();
    let peer = home.peer_ask(NOW_MS - 20_000, "Not a DM.");
    let out = home.lead(&["relay", "--sent", &peer]);
    assert!(!out.status.success(), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("no to_agent line"), "{stderr}");
}

#[test]
fn reply_on_a_relayed_note_carries_the_agents_answer_under_its_name() {
    let home = Home::new();
    home.install(str::to_string, &[]);
    home.bus();
    let id = home.operator_ask(NOW_MS - 30_000, "Para dates: corre la suite.\nto_agent: a2");
    let name = home.json(&["relay", "--json"])["items"][0]["agents"][0]["name"]
        .as_str()
        .unwrap()
        .to_string();
    let v = home.json(&["reply", &id, "Pasa entera.", "--agent", &name, "--json"]);
    assert_eq!(v["text"], format!("{name}: Pasa entera."), "{v}");
    assert_eq!(v["ref"], id);
    assert_eq!(v["to"]["value"], "human");

    // Already prefixed: not twice.
    let v = home.json(&[
        "reply",
        &id,
        &format!("{name}: Otra vez."),
        "--agent",
        &name,
        "--json",
    ]);
    assert_eq!(v["text"], format!("{name}: Otra vez."), "{v}");

    // Without --agent the answer is the lead's own, unsigned.
    let v = home.json(&["reply", &id, "Lo miro yo.", "--json"]);
    assert_eq!(v["text"], "Lo miro yo.", "{v}");
}

#[test]
fn reply_on_a_note_for_two_agents_needs_to_know_whose_answer_it_is() {
    let home = Home::new();
    home.install(str::to_string, &[]);
    home.bus();
    let id = home.operator_ask(
        NOW_MS - 30_000,
        "En #demo: @dates @notes ¿listos?\nto_agent: a1\nto_agent: a2",
    );
    // An agent that is not on the note is refused.
    let out = home.lead(&["reply", &id, "Yes.", "--agent", "nobody"]);
    assert!(!out.status.success(), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--agent nobody is none of them"),
        "{stderr}"
    );
    let v = home.json(&["reply", &id, "Yes.", "--agent", "notes", "--json"]);
    assert!(v["text"].as_str().unwrap().ends_with(": Yes."), "{v}");
    assert_ne!(v["text"], "Yes.", "{v}");
}

#[test]
fn relay_is_a_machine_operation() {
    let home = Home::new();
    home.install(str::to_string, &[]);
    home.bus();
    let id = home.operator_ask(NOW_MS - 30_000, "Para dates: corre la suite.\nto_agent: a2");
    let v = home.json(&["call", "relay"]);
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["data"]["items"][0]["id"], id, "{v}");
    let input = serde_json::json!({ "sent": id }).to_string();
    let v = home.json(&["call", "relay", "--input", &input]);
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(
        home.json(&["call", "relay"])["data"]["items"],
        serde_json::json!([])
    );
}

#[test]
fn name_team_says_the_team_outright() {
    let home = Home::new();
    home.bus();
    let out = home.lead(&[
        "name",
        "Nora",
        "--team",
        "cli",
        "--reason",
        "Chispas al soldar",
    ]);
    assert!(out.status.success(), "{out:?}");
    let names: serde_json::Value =
        serde_json::from_slice(&std::fs::read(home.store().join("names.json")).unwrap()).unwrap();
    let e = &names["agents"][format!("session:{SID}")];
    assert_eq!(e["team"], "cli", "{names}");
    assert_eq!(e["team_explicit"], true, "{names}");

    // Only a workshop name carries a console team.
    let out = home.run_at(
        NOW_MS,
        &[
            "name",
            "Ottoline",
            "--team",
            "cli",
            "--session",
            PEER,
            "--harness",
            "codex",
        ],
    );
    assert!(!out.status.success(), "{out:?}");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("--team"),
        "{out:?}"
    );
}

/// The lead owes an older note relayed to an agent and
/// the operator's newer direct question. `reply last` answers the direct
/// question, as `harbard owed` lists it first, and puts no agent's name on
/// the lead's words.
#[test]
fn reply_last_answers_what_owed_lists_first_and_never_a_relayed_note() {
    let home = Home::new();
    home.install(str::to_string, &[]);
    home.bus();
    let relayed = home.operator_ask(
        NOW_MS - 40_000,
        "Para Owen: ¿revisas el informe?\nto_agent: a1",
    );
    let direct = home.operator_ask(NOW_MS - 30_000, "Should I stop today's integration?");
    let owed = home.json(&["owed", "--json"]);
    assert_eq!(owed["items"][0]["id"], direct.as_str(), "{owed}");
    let v = home.json(&["reply", "last", "Yes, I will stop it.", "--json"]);
    assert_eq!(v["ref"], direct.as_str(), "{v}");
    assert_ne!(v["ref"], relayed.as_str(), "{v}");
    assert_eq!(v["text"], "Yes, I will stop it.", "{v}");
    // Answered: the relayed note is still never `last`.
    let v = home.json(&["reply", "last", "Y otra cosa.", "--json"]);
    assert_ne!(v["ref"], relayed.as_str(), "{v}");
}

/// A session cannot take the workshop name an
/// in-process agent holds, whether the console shows it or it was named
/// within the day.
#[test]
fn a_session_cannot_take_a_name_an_agent_holds() {
    let home = Home::new();
    home.install(str::to_string, &[]);
    home.bus();
    let names = home.store().join("names.json");
    let write = |key: &str, at: u64| {
        std::fs::write(
            &names,
            serde_json::json!({"agents": {key: {
                "name": "Nora", "gender": "f", "source": "auto", "assigned_ms": at,
            }}, "votes": []})
            .to_string(),
        )
        .unwrap();
    };
    // a1 is in the team view; a9 only in names.json, named an hour ago.
    for (key, at) in [("a1", 1), ("a9", NOW_MS - 3_600_000)] {
        write(key, at);
        let out = home.lead(&["name", "Nora"]);
        assert_eq!(out.status.code(), Some(2), "{key}: {out:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("name_taken"), "{key}: {stderr}");
        let after: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&names).unwrap()).unwrap();
        assert_eq!(after["agents"].as_object().unwrap().len(), 1, "{after}");
    }
    // Named long ago and not shown: free again.
    write("a9", 1);
    let out = home.lead(&["name", "Nora"]);
    assert!(out.status.success(), "{out:?}");
}

/// The CLI's reads name agents in memory and write
/// nothing in the home: no names.json, no feedback index.
#[test]
fn reading_agents_and_owed_writes_nothing() {
    let home = Home::new();
    home.install(str::to_string, &[]);
    home.bus();
    let names = home.store().join("names.json");
    let index = home.store().join("feedback");
    let listed = home.json(&["agents", "--json"]);
    assert!(listed.to_string().contains("a1"), "{listed}");
    home.json(&["owed", "--json"]);
    assert!(!names.exists(), "a read wrote names.json");
    assert!(!index.exists(), "a read built the feedback index");
}
