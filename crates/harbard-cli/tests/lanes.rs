//! Lane CLI contract: one owner and a FIFO per repository, a queued take that
//! does not pass `&&`, handovers announced as notes, and turns that end with
//! the session that held them.

use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const MINUTE: u64 = 60_000;

/// The scripted `gh`: PRs 1, 2 and 2258 are merged, 9 was closed unmerged,
/// anything else is open. Every call is logged, so a test can count them.
const GH_DOUBLE: &str = r#"#!/bin/sh
echo "$*" >> "$GH_LOG"
[ "$1 $2 $4 $5 $6 $7" = "pr view --json state --jq .state" ] || exit 9
case "$3" in
  1|2|2258) echo MERGED ;;
  9) echo CLOSED ;;
  *) echo OPEN ;;
esac
"#;

struct Bench {
    home: PathBuf,
    repo: PathBuf,
    gh: PathBuf,
    gh_log: PathBuf,
}

impl Bench {
    fn new() -> Self {
        use std::os::unix::fs::PermissionsExt;
        let sequence = SEQ.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "harbard-lane-cli-{}-{sequence}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&root);
        let home = root.join("home");
        let repo = root.join("repo");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::create_dir_all(&repo).unwrap();
        let gh = root.join("gh");
        std::fs::write(&gh, GH_DOUBLE).unwrap();
        std::fs::set_permissions(&gh, std::fs::Permissions::from_mode(0o755)).unwrap();
        Self {
            home,
            repo,
            gh,
            gh_log: root.join("gh.log"),
        }
    }

    fn gh_calls(&self) -> usize {
        std::fs::read_to_string(&self.gh_log)
            .map(|log| log.lines().count())
            .unwrap_or(0)
    }

    fn command(&self, now_ms: u64, args: &[&str]) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_harbard"));
        command
            .env_clear()
            .env("HARBARD_GH", &self.gh)
            .env("GH_LOG", &self.gh_log)
            .env("HARBARD_NOW_MS", now_ms.to_string())
            .arg("--home")
            .arg(&self.home)
            .arg("--repo")
            .arg(&self.repo)
            .args(args);
        command
    }

    fn run(&self, session: &str, now_ms: u64, args: &[&str]) -> Output {
        let mut command = self.command(now_ms, args);
        command.arg("--session").arg(session);
        command.output().expect("spawn harbard")
    }

    fn json(&self, session: &str, now_ms: u64, args: &[&str]) -> (i32, serde_json::Value) {
        let mut all = args.to_vec();
        all.push("--json");
        let output = self.run(session, now_ms, &all);
        let value = serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
            panic!(
                "JSON output: {error}; stdout={}; stderr={}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            )
        });
        (output.status.code().unwrap_or(-1), value)
    }

    fn take(
        &self,
        session: &str,
        now_ms: u64,
        purpose: &str,
        ttl: &str,
    ) -> (i32, serde_json::Value) {
        self.json(
            session,
            now_ms,
            &["lane", "take", "migrations", "--for", purpose, "--ttl", ttl],
        )
    }

    /// Every note on the bus as (recipient, text).
    fn notes(&self) -> Vec<(String, String)> {
        let mut notes = Vec::new();
        let mut stack = vec![self.home.clone()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                if !(name.starts_with("events-") && name.ends_with(".jsonl")) {
                    continue;
                }
                for line in std::fs::read_to_string(&path).unwrap().lines() {
                    let event: serde_json::Value = serde_json::from_str(line).unwrap();
                    if event["event"] == "note" {
                        notes.push((
                            event["to"]["value"].as_str().unwrap_or("").to_string(),
                            event["text"].as_str().unwrap_or("").to_string(),
                        ));
                    }
                }
            }
        }
        notes
    }

    fn noted(&self, recipient: &str, needle: &str) -> bool {
        self.notes()
            .iter()
            .any(|(to, text)| to == recipient && text.contains(needle))
    }
}

fn owner(value: &serde_json::Value) -> Option<String> {
    value["lane"]["owner"]["session"]
        .as_str()
        .map(str::to_string)
}

#[test]
fn a_queued_take_exits_non_zero_so_and_and_chains_stop() {
    let bench = Bench::new();
    let (code, first) = bench.take("alpha", 1_000, "0234", "60m");
    assert_eq!(code, 0);
    assert_eq!(first["outcome"]["status"], "acquired");
    let (code, second) = bench.take("beta", 2_000, "0235", "60m");
    assert_eq!(code, 2, "queued is not a turn");
    assert_eq!(second["outcome"]["status"], "queued");
    assert_eq!(second["outcome"]["position"], 1);
    assert_eq!(owner(&second).as_deref(), Some("alpha"));
}

#[test]
fn lane_queue_behaves_as_take() {
    let bench = Bench::new();
    let (code, value) = bench.json(
        "alpha",
        1_000,
        &["lane", "queue", "dev-stack", "--for", "cold start"],
    );
    assert_eq!(code, 0);
    assert_eq!(value["outcome"]["status"], "acquired");
}

#[test]
fn release_hands_over_and_notes_only_the_new_owner() {
    let bench = Bench::new();
    bench.take("alpha", 1_000, "0234", "60m");
    bench.take("beta", 2_000, "0235", "60m");
    let (code, released) = bench.json("alpha", 3_000, &["lane", "release", "migrations"]);
    assert_eq!(code, 0);
    assert_eq!(released["outcome"], "released");
    assert_eq!(owner(&released).as_deref(), Some("beta"));
    assert!(bench.noted("beta", "lane migrations is yours now"));
    let repo_notes = bench
        .notes()
        .into_iter()
        .filter(|(_, text)| text.starts_with("lane migrations:"))
        .count();
    assert_eq!(repo_notes, 0, "a normal handover asks nothing of the repo");
    assert!(
        !bench.notes().iter().any(|(to, _)| to == "alpha"),
        "the owner who let go is not told what it did"
    );
}

#[test]
fn an_expiry_noticed_by_a_third_session_warns_both_owners() {
    let bench = Bench::new();
    bench.take("alpha", 1_000, "0234", "1m");
    bench.take("beta", 2_000, "0235", "60m");
    let (_, shown) = bench.json("gamma", 1_000 + MINUTE, &["lane", "show", "migrations"]);
    assert_eq!(shown["lanes"][0]["owner"]["session"].as_str(), Some("beta"));
    assert!(bench.noted("alpha", "no longer yours"));
    assert!(bench.noted("beta", "is yours now"));
}

/// A waiter may be asleep when promoted. The promoted waiter is told by a
/// warning to its own session, and its ttl waits for it to wake.
#[test]
fn a_waiter_promoted_while_asleep_keeps_its_turn_until_it_wakes() {
    let bench = Bench::new();
    bench.take("alpha", 1_000, "0234", "1m");
    bench.take("beta", 2_000, "0235", "5m");
    let (_, shown) = bench.json("gamma", 1_000 + MINUTE, &["lane", "show", "migrations"]);
    let owner = &shown["lanes"][0]["owner"];
    assert_eq!(owner["session"], "beta");
    assert!(owner["claim_by_ms"].is_u64(), "{owner}");
    assert!(bench.noted("beta", "Your ttl starts at your next prompt"));

    // Past its own five-minute ttl, still asleep: the turn is still beta's.
    let late = 1_000 + 12 * MINUTE;
    let (_, shown) = bench.json("gamma", late, &["lane", "show", "migrations"]);
    assert_eq!(shown["lanes"][0]["owner"]["session"], "beta");

    // Any event it writes is a sign of life: the whole ttl starts there.
    let woke = bench.run("beta", late, &["note", "renumbering 0235 now"]);
    assert!(woke.status.success(), "{woke:?}");
    let (_, shown) = bench.json("gamma", late, &["lane", "show", "migrations"]);
    let owner = &shown["lanes"][0]["owner"];
    assert!(owner["claim_by_ms"].is_null(), "{owner}");
    assert_eq!(owner["expires_at_ms"], late + 5 * MINUTE);
}

/// The same night, another session held two lanes that both expired before it
/// was next prompted. A waiter that never wakes must not hold the queue.
#[test]
fn an_unclaimed_promotion_passes_on_after_the_grace_period() {
    let bench = Bench::new();
    bench.take("alpha", 1_000, "0234", "1m");
    bench.take("beta", 2_000, "0235", "60m");
    bench.take("carol", 3_000, "0236", "60m");
    let promoted = 1_000 + MINUTE;
    bench.json("gamma", promoted, &["lane", "show", "migrations"]);
    let (_, shown) = bench.json(
        "gamma",
        promoted + 15 * MINUTE,
        &["lane", "show", "migrations"],
    );
    assert_eq!(shown["lanes"][0]["owner"]["session"], "carol");
    assert!(bench.noted("beta", "no sign of life within 15m"));
    assert!(bench.noted("carol", "is yours now"));
}

#[test]
fn heartbeat_keeps_the_turn_and_tells_a_lost_owner() {
    let bench = Bench::new();
    bench.take("alpha", 1_000, "0234", "1m");
    bench.take("beta", 2_000, "0235", "60m");
    let (code, beat) = bench.json("alpha", 50_000, &["heartbeat"]);
    assert_eq!(code, 0);
    assert_eq!(beat["renewed"][0], "migrations");
    let (_, still) = bench.json("gamma", 1_000 + MINUTE, &["lane", "show", "migrations"]);
    assert_eq!(still["lanes"][0]["owner"]["session"], "alpha");
    let (code, late) = bench.json("alpha", 50_000 + 2 * MINUTE, &["heartbeat", "migrations"]);
    assert_eq!(code, 2);
    assert_eq!(late["not_owner"][0], "migrations");
}

#[test]
fn bye_releases_the_sessions_lanes() {
    let bench = Bench::new();
    bench.take("alpha", 1_000, "0234", "60m");
    bench.take("beta", 2_000, "0235", "60m");
    let output = bench.run("alpha", 3_000, &["bye"]);
    assert!(output.status.success());
    let (_, shown) = bench.json("gamma", 4_000, &["lane", "show", "migrations"]);
    assert_eq!(shown["lanes"][0]["owner"]["session"], "beta");
    assert!(bench.noted("beta", "is yours now"));
}

#[test]
fn await_lane_times_out_keeping_its_place_then_returns_once_owned() {
    let bench = Bench::new();
    bench.take("alpha", 1_000, "0234", "60m");
    let (code, waited) = bench.json(
        "beta",
        2_000,
        &[
            "await",
            "lane",
            "migrations",
            "--for",
            "0235",
            "--timeout-secs",
            "0",
        ],
    );
    assert_eq!(code, 3);
    assert_eq!(waited["outcome"], "timeout");
    assert_eq!(waited["lane"]["queue"][0]["session"], "beta");
    bench.json("alpha", 3_000, &["lane", "release", "migrations"]);
    let (code, owned) = bench.json(
        "beta",
        4_000,
        &["await", "lane", "migrations", "--timeout-secs", "0"],
    );
    assert_eq!(code, 0);
    assert_eq!(owned["outcome"], "acquired");
}

#[test]
fn await_lane_without_a_place_or_a_purpose_is_refused() {
    let bench = Bench::new();
    bench.take("alpha", 1_000, "0234", "60m");
    let output = bench.run(
        "beta",
        2_000,
        &["await", "lane", "migrations", "--timeout-secs", "0"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("--for"));
}

#[test]
fn an_unstable_session_cannot_take_a_lane() {
    let bench = Bench::new();
    let output = bench
        .command(1_000, &["lane", "take", "migrations", "--for", "x"])
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("stable session"));
}

#[test]
fn a_ttl_without_a_unit_is_refused() {
    let bench = Bench::new();
    let output = bench.run(
        "alpha",
        1_000,
        &["lane", "take", "migrations", "--for", "x", "--ttl", "60"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("unit"));
}

#[test]
fn the_brief_lists_held_lanes_with_owner_age_and_queue() {
    let bench = Bench::new();
    bench.take("alpha", 1_000, "0234", "60m");
    bench.take("beta", 2_000, "0235", "60m");
    let output = bench.run("gamma", 1_000 + 5 * MINUTE, &["brief"]);
    let text = String::from_utf8_lossy(&output.stdout);
    let line = text
        .lines()
        .find(|line| line.contains("lane migrations"))
        .unwrap_or_else(|| panic!("no lane line in brief: {text}"));
    assert!(line.contains("held 5m"), "{line}");
    assert!(line.contains("1 waiting"), "{line}");
}

#[test]
fn a_dependency_id_still_awaits_as_before() {
    let bench = Bench::new();
    let output = bench.run(
        "alpha",
        1_000,
        &["await", "not-an-id", "--timeout-secs", "0"],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("dependency"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn await_pr_merged_returns_once_the_shared_watcher_answers() {
    let bench = Bench::new();
    let (code, value) = bench.json(
        "alpha",
        1_000,
        &["await", "pr", "2258", "merged", "--timeout-secs", "5"],
    );
    assert_eq!(code, 0, "{value}");
    assert_eq!(value["outcome"], "satisfied");
    assert_eq!(value["state"], "merged");
    assert_eq!(bench.gh_calls(), 1);
}

#[test]
fn await_pr_merged_on_a_pr_closed_unmerged_fails_instead_of_waiting() {
    let bench = Bench::new();
    let (code, value) = bench.json(
        "alpha",
        1_000,
        &["await", "pr", "9", "merged", "--timeout-secs", "5"],
    );
    assert_eq!(code, 4);
    assert_eq!(value["outcome"], "closed_unmerged");
    let (code, closed) = bench.json(
        "alpha",
        1_000,
        &["await", "pr", "9", "closed", "--timeout-secs", "0"],
    );
    assert_eq!(code, 0);
    assert_eq!(closed["outcome"], "satisfied");
}

#[test]
fn many_waiters_on_an_open_pr_cost_one_gh_call_per_interval() {
    let bench = Bench::new();
    for (session, now) in [("alpha", 1_000), ("beta", 2_000), ("gamma", 60_000)] {
        let (code, value) = bench.json(
            session,
            now,
            &["await", "pr", "77", "merged", "--timeout-secs", "0"],
        );
        assert_eq!(code, 3);
        assert_eq!(value["outcome"], "timeout");
    }
    assert_eq!(bench.gh_calls(), 1, "one turn inside the interval");
    bench.json(
        "alpha",
        1_000 + 3 * MINUTE,
        &["await", "pr", "77", "merged", "--timeout-secs", "0"],
    );
    assert_eq!(
        bench.gh_calls(),
        2,
        "the next turn once the interval passed"
    );
}

#[test]
fn a_lane_tied_to_prs_passes_on_once_every_pr_settled() {
    let bench = Bench::new();
    let (code, _) = bench.json(
        "alpha",
        1_000,
        &[
            "lane",
            "take",
            "migrations",
            "--for",
            "0234-0236",
            "--release-on",
            "pr:1,pr:2",
        ],
    );
    assert_eq!(code, 0);
    bench.take("beta", 2_000, "0237", "60m");
    // Any waiter on a PR takes the shared turn, which also answers the PRs
    // lanes are tied to; `serve` does the same on its tick.
    bench.json(
        "gamma",
        3_000,
        &["await", "pr", "2258", "merged", "--timeout-secs", "0"],
    );
    let (_, shown) = bench.json("gamma", 4_000, &["lane", "show", "migrations"]);
    assert_eq!(shown["lanes"][0]["owner"]["session"], "beta");
    assert!(bench.noted("alpha", "PR #1, #2 settled"));
    assert!(bench.noted("beta", "is yours now"));
}

#[test]
fn release_on_refuses_anything_but_pr_numbers() {
    let bench = Bench::new();
    let output = bench.run(
        "alpha",
        1_000,
        &[
            "lane",
            "take",
            "migrations",
            "--for",
            "x",
            "--release-on",
            "2258",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("pr:<number>"));
}
