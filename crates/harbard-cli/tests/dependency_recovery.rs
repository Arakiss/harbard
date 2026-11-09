//! Recovery boundaries through the CLI, using only disposable local stores.

use harbard_core::{Event, EventKind, Store, Window};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const OWNER: &str = "recovery-requester";
const RESPONDER: &str = "recovery-responder";

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("harbard-recovery-{}", ulid::Ulid::generate()));
        fs::create_dir(&path).unwrap();
        let fixture = Self(path);
        fixture.ok(OWNER, &["hello"]);
        fixture.ok(RESPONDER, &["hello"]);
        fixture
    }

    fn command(&self, actor: &str, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_harbard"));
        command
            .env_clear()
            .current_dir(&self.0)
            .arg("--home")
            .arg(&self.0)
            .arg("--cwd")
            .arg(&self.0)
            .args(["--session", actor, "--json"])
            .args(args);
        command
    }

    fn run(&self, actor: &str, args: &[&str]) -> Output {
        self.command(actor, args).output().unwrap()
    }

    fn ok(&self, actor: &str, args: &[&str]) -> Value {
        let output = self.run(actor, args);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn open(&self) -> String {
        self.ok(
            OWNER,
            &[
                "dependency",
                "open",
                "--to",
                "session:recovery-responder",
                "--condition",
                "A result must be verified",
                "--resume",
                "Inspect the result",
            ],
        )["id"]
            .as_str()
            .unwrap()
            .into()
    }

    fn brief(&self) -> String {
        self.ok(OWNER, &["brief", "--touch"])["brief"]
            .as_str()
            .unwrap_or("")
            .into()
    }

    fn segment(&self) -> PathBuf {
        Store::at(&self.0).segments().pop().unwrap()
    }

    /// What one `harbard` process costs on this machine right now: the
    /// slowest of three `dependency show` runs. Under load a spawn can take
    /// seconds, and the fixed
    /// 2 s attach deadline and 3 s listener lifetime failed this suite.
    /// Budgets below are scaled from it, never padded with a sleep.
    fn spawn_cost(&self, id: &str) -> Duration {
        (0..3)
            .map(|_| {
                let t = Instant::now();
                self.ok(OWNER, &["dependency", "show", id]);
                t.elapsed()
            })
            .max()
            .unwrap()
    }

    /// Poll `done` until it holds, failing past `cap`. The condition ends
    /// the wait; the cap only bounds a broken run.
    fn until(&self, cap: Duration, what: &str, mut done: impl FnMut() -> bool) {
        let started = Instant::now();
        while !done() {
            assert!(started.elapsed() < cap, "{what} within {cap:?}");
            thread::sleep(Duration::from_millis(10));
        }
    }

    /// A listener that has attached. It lives `lifetime` unless killed (a
    /// test that waits for its timeout gives the shortest that still
    /// outlasts attaching on this machine).
    fn waiter_for(&self, id: &str, lifetime: Duration, cost: Duration) -> Waiter {
        let secs = lifetime.as_secs().max(1).to_string();
        let child = self
            .command(
                OWNER,
                &["await", id, "--timeout-secs", &secs, "--poll-ms", "60000"],
            )
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut waiter = Waiter(Some(child));
        self.until(
            Duration::from_secs(10) + cost * 20,
            "listener attaches",
            || {
                assert!(
                    waiter.0.as_mut().unwrap().try_wait().unwrap().is_none(),
                    "listener exited before attaching"
                );
                self.ok(OWNER, &["dependency", "show", id])["active_cli_wait"] == true
            },
        );
        waiter
    }

    /// A listener that times out on its own a few seconds after attaching.
    fn waiter(&self, id: &str) -> Waiter {
        let cost = self.spawn_cost(id);
        self.waiter_for(id, Duration::from_secs(3) + cost * 8, cost)
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Waiter(Option<Child>);
impl Waiter {
    fn finish(mut self) -> Output {
        self.0.take().unwrap().wait_with_output().unwrap()
    }
}
impl Drop for Waiter {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

/// Replace dependencies by ordinary notes without shrinking their serialized
/// records. A size-only checkpoint cannot distinguish this replacement.
fn without_dependencies(bytes: &[u8]) -> Vec<u8> {
    let mut result = Vec::new();
    for line in bytes.split_inclusive(|byte| *byte == b'\n') {
        let mut event: Event = serde_json::from_slice(line).unwrap();
        if event.event == EventKind::Dependency {
            event.event = EventKind::Note;
            event.text.clear();
            let empty = event.to_line().unwrap();
            event.text = "x".repeat(line.len() - empty.len() - 1);
            let replacement = format!("{}\n", event.to_line().unwrap());
            assert_eq!(replacement.len(), line.len());
            result.extend_from_slice(replacement.as_bytes());
        } else {
            result.extend_from_slice(line);
        }
    }
    result
}

#[test]
fn replacing_a_same_size_segment_cannot_leave_a_ghost_dependency_in_brief() {
    let fixture = Fixture::new();
    let id = fixture.open();
    assert!(fixture.brief().contains(&id));
    let segment = fixture.segment();
    let replacement = without_dependencies(&fs::read(&segment).unwrap());
    let temporary = fixture.0.join("replacement.jsonl");
    fs::write(&temporary, replacement).unwrap();
    fs::rename(temporary, &segment).unwrap();
    assert!(
        !fixture
            .run(OWNER, &["dependency", "show", &id])
            .status
            .success()
    );
    let brief = fixture.brief();
    assert!(
        !brief.contains(&id),
        "stale dependency came from the cache: {brief}"
    );
    assert!(!brief.contains("history unavailable"));
}

#[test]
fn same_inode_truncation_and_regrowth_rebuilds_from_the_log() {
    let fixture = Fixture::new();
    let id = fixture.open();
    assert!(fixture.brief().contains(&id));
    let segment = fixture.segment();
    let original = fs::read(&segment).unwrap();
    let mut replacement = without_dependencies(&original);
    replacement.extend_from_slice(
        original
            .split_inclusive(|byte| *byte == b'\n')
            .next()
            .unwrap(),
    );
    fs::write(&segment, replacement).unwrap();
    assert!(!fixture.brief().contains(&id));
    assert!(
        !fixture
            .run(OWNER, &["dependency", "show", &id])
            .status
            .success()
    );
}

#[test]
fn changing_an_older_segment_invalidates_the_dependency_cache() {
    let fixture = Fixture::new();
    let id = fixture.open();
    let current = fixture.segment();
    let old = current.parent().unwrap().join("events-2000-01-01.jsonl");
    fs::rename(&current, &old).unwrap();
    fixture.ok(OWNER, &["intent", "Inspect the result"]);
    assert!(fixture.brief().contains(&id));
    fs::write(&old, without_dependencies(&fs::read(&old).unwrap())).unwrap();
    assert!(!fixture.brief().contains(&id));
}

#[test]
fn syntactically_valid_cache_corruption_is_regenerated() {
    let fixture = Fixture::new();
    let id = fixture.open();
    assert!(fixture.brief().contains(&id));
    let path = fixture.0.join("dependency-index.json");
    let mut cache: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    cache["events"] = serde_json::json!([]);
    fs::write(&path, serde_json::to_vec(&cache).unwrap()).unwrap();
    assert!(
        fixture.brief().contains(&id),
        "cache lost authoritative pending work"
    );
}

#[test]
fn consuming_a_result_while_a_listener_sleeps_does_not_return_new_work() {
    let fixture = Fixture::new();
    let id = fixture.open();
    let waiter = fixture.waiter(&id);
    fixture.ok(
        RESPONDER,
        &[
            "dependency",
            "resolve",
            &id,
            "--outcome",
            "satisfied",
            "--evidence",
            "The result is available",
        ],
    );
    fixture.ok(OWNER, &["dependency", "consume", &id]);
    let output = waiter.finish();
    assert_eq!(
        output.status.code(),
        Some(1),
        "already consumed result was returned: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stderr).contains("already consumed"));
}

#[test]
fn two_listeners_cannot_wait_concurrently_and_a_dead_listener_releases_ownership() {
    let fixture = Fixture::new();
    let id = fixture.open();
    // Killed below, so it may live long: under load the duplicate must still
    // find it there, however slow each process is.
    let cost = fixture.spawn_cost(&id);
    let waiter = fixture.waiter_for(&id, Duration::from_secs(60) + cost * 40, cost);
    let duplicate = fixture.run(OWNER, &["await", &id, "--timeout-secs", "0"]);
    assert_eq!(duplicate.status.code(), Some(1), "{duplicate:?}");
    assert!(String::from_utf8_lossy(&duplicate.stderr).contains("another CLI listener"));
    drop(waiter);
    fixture.until(
        Duration::from_secs(10) + cost * 20,
        "a dead listener releases ownership",
        || fixture.ok(OWNER, &["dependency", "show", &id])["active_cli_wait"] == false,
    );
    assert_eq!(
        fixture
            .run(OWNER, &["await", &id, "--timeout-secs", "0"])
            .status
            .code(),
        Some(3)
    );
}

#[test]
fn configured_hub_without_credentials_cannot_silently_create_a_local_dependency() {
    let fixture = Fixture::new();
    let before = Store::at(&fixture.0).read(Window::All).events.len();
    let output = fixture
        .command(
            OWNER,
            &[
                "dependency",
                "open",
                "--to",
                "session:recovery-responder",
                "--condition",
                "A remote result",
                "--resume",
                "Inspect the result",
            ],
        )
        .env("HARBARD_HUB", "http://127.0.0.1:1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("remote hub transport is not supported")
    );
    assert_eq!(Store::at(&fixture.0).read(Window::All).events.len(), before);
}
