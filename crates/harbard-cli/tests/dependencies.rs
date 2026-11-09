//! Durable dependencies through the real CLI, with an isolated store per test.
//! Child environments never inherit the operator's live bus or session.

use serde_json::Value;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const REQUESTER: &str = "dependency-requester";
const RESPONDER: &str = "dependency-responder";
const OUTSIDER: &str = "dependency-outsider";
const CONDITION: &str = "The fixture is available";
const CONTINUATION: &str = "Run the pending fixture checks";
const EVIDENCE: &str = "Fixture checks passed";

#[test]
fn await_note_ids_explain_thread_inspection_and_durable_dependencies() {
    let fixture = Fixture::new();
    let body = "Synthetic private note body must not appear in the diagnostic";
    fixture.ok(
        REQUESTER,
        &["note", body, "--to", "session:dependency-responder"],
    );
    let store = harbard_core::Store::at(&fixture.home);
    let read = store.read(harbard_core::Window::All);
    let note = read
        .events
        .iter()
        .find(|event| event.event == harbard_core::EventKind::Note)
        .unwrap();
    for id in [note.id.as_str(), &note.id[..20]] {
        let output = fixture.await_result(id, "0");
        assert_exit(&output, 1);
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(diagnostic.contains("full dependency ID"), "{diagnostic}");
        assert!(
            diagnostic.contains(&format!("harbard thread {}", note.id)),
            "{diagnostic}"
        );
        assert!(
            diagnostic.contains("harbard dependency open --to session:<id>"),
            "{diagnostic}"
        );
        assert!(
            diagnostic.contains("--condition") && diagnostic.contains("--resume"),
            "{diagnostic}"
        );
        assert!(!diagnostic.contains(body));
    }
    assert_eq!(
        fixture.ok(REQUESTER, &["dependency", "list"]),
        serde_json::json!([])
    );
}

#[test]
fn await_ambiguous_note_prefix_does_not_select_a_thread() {
    let fixture = Fixture::new();
    for body in ["First synthetic note", "Second synthetic note"] {
        fixture.ok(
            REQUESTER,
            &["note", body, "--to", "session:dependency-responder"],
        );
    }
    let read = harbard_core::Store::at(&fixture.home).read(harbard_core::Window::All);
    let notes: Vec<_> = read
        .events
        .iter()
        .filter(|event| event.event == harbard_core::EventKind::Note)
        .collect();
    let prefix = notes[0]
        .id
        .chars()
        .zip(notes[1].id.chars())
        .take_while(|(a, b)| a == b)
        .map(|(a, _)| a)
        .collect::<String>();
    let output = fixture.await_result(&prefix, "0");
    assert_exit(&output, 1);
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(diagnostic.contains("matches 2 notes"), "{diagnostic}");
    assert!(
        diagnostic.contains("harbard thread <full-note-id>"),
        "{diagnostic}"
    );
    for note in notes {
        assert!(!diagnostic.contains(&note.id), "{diagnostic}");
    }
}

#[test]
fn await_unknown_and_dependency_prefix_ids_explain_full_id_lookup() {
    let fixture = Fixture::new();
    let dependency = fixture.open();
    let full_id = dependency["id"].as_str().unwrap();
    let unknown = ulid::Ulid::generate().to_string();
    for id in [unknown.as_str(), &full_id[..20], "not-an-id"] {
        let output = fixture.await_result(id, "0");
        assert_exit(&output, 1);
        let diagnostic = String::from_utf8_lossy(&output.stderr);
        assert!(diagnostic.contains("full dependency ID"), "{diagnostic}");
        assert!(
            diagnostic.contains("harbard dependency list"),
            "{diagnostic}"
        );
    }
    assert_eq!(fixture.show(full_id)["state"], "pending");
}

#[test]
fn await_prefix_shared_by_a_note_and_dependency_keeps_full_id_guidance() {
    let fixture = Fixture::new();
    let dependency = fixture.open();
    fixture.ok(
        REQUESTER,
        &[
            "note",
            "Synthetic task",
            "--to",
            "session:dependency-responder",
        ],
    );
    let full_id = dependency["id"].as_str().unwrap();
    let output = fixture.await_result(&full_id[..1], "0");
    assert_exit(&output, 1);
    let diagnostic = String::from_utf8_lossy(&output.stderr);
    assert!(
        diagnostic.contains("harbard dependency list"),
        "{diagnostic}"
    );
    assert!(!diagnostic.contains("not a note ID"), "{diagnostic}");
    assert!(!diagnostic.contains("harbard thread"), "{diagnostic}");
    assert_eq!(fixture.show(full_id)["state"], "pending");
}

struct Fixture {
    home: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let home =
            std::env::temp_dir().join(format!("harbard-dependency-cli-{}", ulid::Ulid::generate()));
        std::fs::create_dir(&home).expect("create isolated store");
        let fixture = Self { home };
        for session in [REQUESTER, RESPONDER, OUTSIDER] {
            fixture.ok(session, &["hello"]);
        }
        fixture
    }

    fn command(&self, session: &str, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_harbard"));
        command
            .env_clear()
            .current_dir(&self.home)
            .args(["--home"])
            .arg(&self.home)
            .args(["--cwd"])
            .arg(&self.home)
            .args(["--session", session, "--json"])
            .args(args);
        command
    }

    fn run(&self, session: &str, args: &[&str]) -> Output {
        self.command(session, args).output().expect("run CLI")
    }

    fn ok(&self, session: &str, args: &[&str]) -> Value {
        let output = self.run(session, args);
        assert_exit(&output, 0);
        json(&output)
    }

    fn open(&self) -> Value {
        self.ok(
            REQUESTER,
            &[
                "dependency",
                "open",
                "--to",
                "session:dependency-responder",
                "--condition",
                CONDITION,
                "--resume",
                CONTINUATION,
                "--deadline-secs",
                "60",
            ],
        )
    }

    fn show(&self, id: &str) -> Value {
        self.ok(REQUESTER, &["dependency", "show", id])
    }

    fn resolve(&self, id: &str, outcome: &str, evidence: &str) -> Value {
        self.ok(
            RESPONDER,
            &[
                "dependency",
                "resolve",
                id,
                "--outcome",
                outcome,
                "--evidence",
                evidence,
            ],
        )
    }

    fn await_result(&self, id: &str, timeout: &str) -> Output {
        self.run(
            REQUESTER,
            &["await", id, "--timeout-secs", timeout, "--poll-ms", "20"],
        )
    }

    fn spawn_awaiter(&self, id: &str) -> Awaiter {
        let child = self
            .command(
                REQUESTER,
                &["await", id, "--timeout-secs", "5", "--poll-ms", "20"],
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn waiting CLI");
        Awaiter(Some(child))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

struct Awaiter(Option<Child>);

impl Awaiter {
    fn assert_waiting(&mut self) {
        // Keep the dependency pending while the other process enters its wait.
        // No result can appear before the explicit responder command below.
        thread::sleep(Duration::from_millis(150));
        assert!(self.0.as_mut().unwrap().try_wait().unwrap().is_none());
    }

    fn finish(mut self) -> Output {
        let deadline = Instant::now() + Duration::from_secs(7);
        loop {
            if self.0.as_mut().unwrap().try_wait().unwrap().is_some() {
                return self
                    .0
                    .take()
                    .unwrap()
                    .wait_with_output()
                    .expect("collect waiter");
            }
            assert!(
                Instant::now() < deadline,
                "awaiter exceeded its bounded timeout"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for Awaiter {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn assert_exit(output: &Output, code: i32) {
    assert_eq!(
        output.status.code(),
        Some(code),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "invalid JSON: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn id(view: &Value) -> &str {
    view["id"].as_str().expect("dependency id")
}

fn assert_idle_show(mut shown: Value, expected: &Value) {
    assert_eq!(shown["active_cli_wait"], false);
    assert_eq!(shown["automatic_resume"], false);
    let object = shown.as_object_mut().expect("shown dependency");
    object.remove("active_cli_wait");
    object.remove("automatic_resume");
    assert_eq!(&shown, expected);
}

fn assert_await(output: &Output, code: i32, outcome: &str, dependency_id: &str) -> Value {
    assert_exit(output, code);
    let result = json(output);
    assert_eq!(result["outcome"], outcome);
    assert_eq!(result["dependency"]["id"], dependency_id);
    result
}

#[test]
fn open_show_and_list_preserve_the_resume_contract() {
    let fixture = Fixture::new();
    let opened = fixture.open();
    assert!(ulid::Ulid::from_string(id(&opened)).is_ok());
    assert_eq!(opened["requester_session"], REQUESTER);
    assert_eq!(opened["responder_session"], RESPONDER);
    assert_eq!(opened["condition"], CONDITION);
    assert_eq!(opened["continuation"], CONTINUATION);
    assert_eq!(opened["state"], "pending");
    assert_eq!(opened["consumed"], false);
    assert!(opened["resolution_event_id"].is_null());
    assert!(opened["deadline_ms"].as_u64().is_some());
    assert_idle_show(fixture.show(id(&opened)), &opened);
    let listed = fixture.ok(REQUESTER, &["dependency", "list"]);
    assert!(
        listed
            .as_array()
            .expect("dependency array")
            .iter()
            .any(|item| item["id"] == opened["id"])
    );
}

#[test]
fn response_before_await_is_returned_without_consuming_it() {
    let fixture = Fixture::new();
    let opened = fixture.open();
    let resolved = fixture.resolve(id(&opened), "satisfied", EVIDENCE);
    let result = assert_await(
        &fixture.await_result(id(&opened), "2"),
        0,
        "satisfied",
        id(&opened),
    );
    assert_eq!(result["dependency"], resolved);
    assert_eq!(result["dependency"]["evidence"], EVIDENCE);
    assert_eq!(fixture.show(id(&opened))["consumed"], false);
}

#[test]
fn responder_process_resolves_a_dependency_while_requester_is_waiting() {
    let fixture = Fixture::new();
    let opened = fixture.open();
    let mut awaiter = fixture.spawn_awaiter(id(&opened));
    awaiter.assert_waiting();
    let resolved = fixture.resolve(id(&opened), "satisfied", EVIDENCE);
    let result = assert_await(&awaiter.finish(), 0, "satisfied", id(&opened));
    assert_eq!(
        result["dependency"]["resolution_event_id"],
        resolved["resolution_event_id"]
    );
    assert_eq!(result["dependency"]["continuation"], CONTINUATION);
}

#[test]
fn acknowledging_a_note_does_not_satisfy_the_dependency() {
    let fixture = Fixture::new();
    let opened = fixture.open();
    let note = fixture.ok(
        REQUESTER,
        &[
            "note",
            "Please inspect the fixture",
            "--to",
            "session:dependency-responder",
        ],
    );
    fixture.ok(
        RESPONDER,
        &[
            "ack",
            note["id"].as_str().expect("note id"),
            "--note",
            "Received",
        ],
    );
    assert_eq!(fixture.show(id(&opened))["state"], "pending");
    let result = assert_await(
        &fixture.await_result(id(&opened), "1"),
        3,
        "timeout",
        id(&opened),
    );
    assert_eq!(result["dependency"]["state"], "pending");
}

#[test]
fn brief_retains_work_after_inbox_and_ack_until_the_requester_consumes_it() {
    let fixture = Fixture::new();
    let opened = fixture.open();
    let dependency_id = id(&opened);
    for _ in 0..2 {
        let brief = fixture.ok(REQUESTER, &["brief", "--touch"]);
        assert!(
            brief["brief"]
                .as_str()
                .unwrap_or("")
                .contains(dependency_id)
        );
    }
    let note = fixture.ok(
        RESPONDER,
        &[
            "note",
            "The fixture is being inspected",
            "--to",
            "session:dependency-requester",
        ],
    );
    fixture.ok(REQUESTER, &["inbox"]);
    fixture.ok(REQUESTER, &["ack", note["id"].as_str().expect("note id")]);
    let pending = fixture.ok(REQUESTER, &["brief", "--touch"]);
    assert!(
        pending["brief"]
            .as_str()
            .unwrap_or("")
            .contains(dependency_id)
    );
    assert_eq!(fixture.show(dependency_id)["state"], "pending");
    assert_exit(
        &fixture.run(REQUESTER, &["dependency", "consume", dependency_id]),
        1,
    );
    fixture.resolve(dependency_id, "satisfied", EVIDENCE);
    fixture.ok(REQUESTER, &["inbox"]);
    let resolved = fixture.ok(REQUESTER, &["brief", "--touch"]);
    assert!(
        resolved["brief"]
            .as_str()
            .unwrap_or("")
            .contains(dependency_id)
    );
    fixture.ok(REQUESTER, &["dependency", "consume", dependency_id]);
    let consumed = fixture.ok(REQUESTER, &["brief", "--touch"]);
    assert!(
        !consumed["brief"]
            .as_str()
            .unwrap_or("")
            .contains(dependency_id)
    );
    let history = fixture.ok(REQUESTER, &["dependency", "list", "--all"]);
    assert!(
        history
            .as_array()
            .expect("dependency history")
            .iter()
            .any(|item| { item["id"] == dependency_id && item["consumed"] == true })
    );
}

#[test]
fn wrong_actors_cannot_resolve_cancel_or_consume() {
    let fixture = Fixture::new();
    let opened = fixture.open();
    for actor in [REQUESTER, OUTSIDER] {
        assert_exit(
            &fixture.run(
                actor,
                &[
                    "dependency",
                    "resolve",
                    id(&opened),
                    "--outcome",
                    "satisfied",
                    "--evidence",
                    EVIDENCE,
                ],
            ),
            1,
        );
    }
    for actor in [RESPONDER, OUTSIDER] {
        assert_exit(
            &fixture.run(
                actor,
                &["dependency", "cancel", id(&opened), "--reason", "Not mine"],
            ),
            1,
        );
    }
    assert_eq!(fixture.show(id(&opened))["state"], "pending");
    fixture.resolve(id(&opened), "satisfied", EVIDENCE);
    for actor in [RESPONDER, OUTSIDER] {
        assert_exit(
            &fixture.run(actor, &["dependency", "consume", id(&opened)]),
            1,
        );
    }
    assert_eq!(fixture.show(id(&opened))["consumed"], false);
    let consumed = fixture.ok(REQUESTER, &["dependency", "consume", id(&opened)]);
    assert_eq!(consumed["consumed"], true);
    assert_eq!(consumed["state"], "satisfied");
}

#[test]
fn a_local_timeout_preserves_the_dependency_for_a_later_response() {
    let fixture = Fixture::new();
    let opened = fixture.open();
    let timeout = assert_await(
        &fixture.await_result(id(&opened), "1"),
        3,
        "timeout",
        id(&opened),
    );
    assert_eq!(timeout["dependency"]["state"], "pending");
    assert_eq!(timeout["dependency"]["consumed"], false);
    assert_eq!(fixture.show(id(&opened))["state"], "pending");
    fixture.resolve(
        id(&opened),
        "satisfied",
        "The fixture became available after the wait ended",
    );
    assert_await(
        &fixture.await_result(id(&opened), "2"),
        0,
        "satisfied",
        id(&opened),
    );
}

#[test]
fn equivalent_resolution_is_idempotent_and_terminal_contradictions_are_rejected() {
    let fixture = Fixture::new();
    let opened = fixture.open();
    let first = fixture.resolve(id(&opened), "satisfied", EVIDENCE);
    assert!(first["resolution_event_id"].as_str().is_some());
    let repeated = fixture.resolve(id(&opened), "satisfied", EVIDENCE);
    assert_eq!(repeated, first);
    assert_exit(
        &fixture.run(
            RESPONDER,
            &[
                "dependency",
                "resolve",
                id(&opened),
                "--outcome",
                "failed",
                "--evidence",
                "Contradictory result",
            ],
        ),
        1,
    );
    assert_idle_show(fixture.show(id(&opened)), &first);
}

#[test]
fn killing_the_waiter_does_not_lose_the_dependency_or_its_later_resolution() {
    let fixture = Fixture::new();
    let opened = fixture.open();
    let mut awaiter = fixture.spawn_awaiter(id(&opened));
    awaiter.assert_waiting();
    drop(awaiter);
    assert_eq!(fixture.show(id(&opened))["state"], "pending");
    fixture.resolve(id(&opened), "satisfied", EVIDENCE);
    let resumed = assert_await(
        &fixture.await_result(id(&opened), "2"),
        0,
        "satisfied",
        id(&opened),
    );
    assert_eq!(resumed["dependency"]["continuation"], CONTINUATION);
    assert_eq!(resumed["dependency"]["consumed"], false);
}

#[test]
fn failed_and_cancelled_results_are_distinct_from_local_timeout() {
    let fixture = Fixture::new();
    let failed = fixture.open();
    fixture.resolve(id(&failed), "failed", "The fixture cannot be provided");
    let result = assert_await(
        &fixture.await_result(id(&failed), "2"),
        4,
        "failed",
        id(&failed),
    );
    assert_eq!(
        result["dependency"]["evidence"],
        "The fixture cannot be provided"
    );
    let cancelled = fixture.open();
    fixture.ok(
        REQUESTER,
        &[
            "dependency",
            "cancel",
            id(&cancelled),
            "--reason",
            "The pending work was withdrawn",
        ],
    );
    let result = assert_await(
        &fixture.await_result(id(&cancelled), "2"),
        4,
        "cancelled",
        id(&cancelled),
    );
    assert_eq!(result["dependency"]["state"], "cancelled");
    assert_eq!(
        result["dependency"]["evidence"],
        "The pending work was withdrawn"
    );
    assert_exit(
        &fixture.run(
            RESPONDER,
            &[
                "dependency",
                "resolve",
                id(&cancelled),
                "--outcome",
                "satisfied",
                "--evidence",
                EVIDENCE,
            ],
        ),
        1,
    );
    assert_eq!(fixture.show(id(&cancelled))["state"], "cancelled");
}

#[test]
fn broadcast_dependency_targets_are_rejected_without_creating_work() {
    let fixture = Fixture::new();
    let output = fixture.run(
        REQUESTER,
        &[
            "dependency",
            "open",
            "--to",
            "harness:other",
            "--condition",
            CONDITION,
            "--resume",
            CONTINUATION,
        ],
    );
    assert_exit(&output, 1);
    let listed = fixture.ok(REQUESTER, &["dependency", "list", "--all"]);
    assert!(listed.as_array().expect("dependency array").is_empty());
}
