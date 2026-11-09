//! Lease CLI contract: stable JSON/human output, fencing, fail-closed state,
//! stable writer identity, and absence of arbitrary environment payloads.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Barrier};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp_home() -> PathBuf {
    let sequence = SEQ.fetch_add(1, Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "harbard-lease-cli-{}-{sequence}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("create temp home");
    path
}

fn run(home: &Path, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_harbard"));
    command
        .env_clear()
        .env("SECRET_SENTINEL", "must-never-enter-lease-state")
        .arg("--home")
        .arg(home)
        .args(args);
    command.output().expect("spawn harbard")
}

fn json(output: &Output) -> serde_json::Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|error| {
        panic!(
            "JSON output: {error}; stdout={}; stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn claim_service(home: &Path, name: &str) -> serde_json::Value {
    let output = run(
        home,
        &["claim", "--service", name, "--ttl-secs", "60", "--json"],
    );
    assert!(output.status.success(), "claim succeeds");
    json(&output)
}

fn run_at(home: &Path, now_ms: u64, args: &[&str]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_harbard"));
    command
        .env_clear()
        .env("HARBARD_NOW_MS", now_ms.to_string())
        .env("SECRET_SENTINEL", "must-never-enter-lease-state")
        .arg("--home")
        .arg(home)
        .args(args);
    command.output().expect("spawn harbard")
}

#[test]
fn claim_check_renew_release_has_stable_json_and_fencing() {
    let home = tmp_home();
    let first = run_at(
        &home,
        1_000,
        &[
            "claim",
            "--service",
            "operations.northwind.localhost",
            "--ttl-secs",
            "60",
            "--json",
            "--session",
            "owner-one",
        ],
    );
    assert!(first.status.success());
    let first = json(&first);
    assert_eq!(first["status"], "acquired");
    let first_id = first["lease"]["claim_id"].as_str().unwrap();
    assert!(ulid::Ulid::from_string(first_id).is_ok());
    assert_eq!(first["lease"]["resource"]["kind"], "service");
    let claimed_at = first["lease"]["claimed_at_ms"].as_u64().unwrap();
    let expires_at = first["lease"]["expires_at_ms"].as_u64().unwrap();
    assert!(
        claimed_at > 1_000_000_000_000,
        "F0 clock override is ignored"
    );
    assert_eq!(expires_at - claimed_at, 60_000);

    let repeated = run_at(
        &home,
        1_050,
        &[
            "claim",
            "--service",
            "operations.northwind.localhost",
            "--json",
            "--session",
            "owner-one",
        ],
    );
    assert!(repeated.status.success());
    let repeated = json(&repeated);
    assert_eq!(repeated["status"], "held");
    assert_eq!(repeated["lease"]["claim_id"], first_id);

    let conflict = run_at(
        &home,
        1_100,
        &[
            "claim",
            "--service",
            "operations.northwind.localhost",
            "--json",
            "--session",
            "owner-two",
        ],
    );
    assert_eq!(conflict.status.code(), Some(2));
    let conflict = json(&conflict);
    assert_eq!(conflict["status"], "conflict");
    assert!(conflict["lease"].get("claim_id").is_none());
    assert!(conflict["lease"].get("session").is_none());

    let wrong_id = "01ARZ3NDEKTSV4RRFFQ69G5FAV";
    let fenced = run_at(
        &home,
        1_200,
        &[
            "release",
            "--service",
            "operations.northwind.localhost",
            "--claim-id",
            wrong_id,
            "--json",
        ],
    );
    assert_eq!(fenced.status.code(), Some(2));
    let fenced = json(&fenced);
    assert_eq!(fenced["status"], "fenced");
    assert!(fenced["lease"].get("claim_id").is_none());

    let renewed = run_at(
        &home,
        2_000,
        &[
            "check",
            "--service",
            "operations.northwind.localhost",
            "--claim-id",
            first_id,
            "--renew",
            "--json",
        ],
    );
    assert!(renewed.status.success());
    let renewed = json(&renewed);
    assert_eq!(renewed["status"], "renewed");
    assert_eq!(renewed["matches_claim"], true);
    assert_eq!(
        renewed["lease"]["expires_at_ms"].as_u64().unwrap()
            - renewed["lease"]["renewed_at_ms"].as_u64().unwrap(),
        60_000
    );

    let released = run_at(
        &home,
        2_100,
        &[
            "release",
            "--service",
            "operations.northwind.localhost",
            "--claim-id",
            first_id,
            "--json",
        ],
    );
    assert!(released.status.success());
    assert_eq!(json(&released)["status"], "released");

    let again = run_at(
        &home,
        2_200,
        &[
            "release",
            "--service",
            "operations.northwind.localhost",
            "--claim-id",
            first_id,
            "--json",
        ],
    );
    assert!(again.status.success());
    assert_eq!(json(&again)["status"], "free");
}

#[test]
fn stale_lease_is_reclaimed_and_old_token_cannot_release_new_claim() {
    let home = tmp_home();
    let first = run_at(
        &home,
        1_000,
        &[
            "claim",
            "--service",
            "platform.northwind.localhost",
            "--ttl-secs",
            "1",
            "--json",
        ],
    );
    let first_id = json(&first)["lease"]["claim_id"]
        .as_str()
        .unwrap()
        .to_string();
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    let second = run_at(
        &home,
        2_000,
        &[
            "claim",
            "--service",
            "platform.northwind.localhost",
            "--ttl-secs",
            "60",
            "--json",
        ],
    );
    assert!(second.status.success());
    let second = json(&second);
    let second_id = second["lease"]["claim_id"].as_str().unwrap().to_string();
    assert_ne!(first_id, second_id);

    let old_release = run_at(
        &home,
        2_100,
        &[
            "release",
            "--service",
            "platform.northwind.localhost",
            "--claim-id",
            &first_id,
            "--json",
        ],
    );
    assert_eq!(old_release.status.code(), Some(2));
    let old_release = json(&old_release);
    assert_eq!(old_release["status"], "fenced");
    assert!(old_release["lease"].get("claim_id").is_none());
}

#[test]
fn writer_requires_stable_session_and_uses_git_common_dir_plus_worktree() {
    let home = tmp_home();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let repo_arg = repo.to_str().unwrap();
    let missing_session = run(&home, &["claim", "--writer", "--repo", repo_arg, "--json"]);
    assert_eq!(missing_session.status.code(), Some(1));
    assert_eq!(json(&missing_session)["error"], "invalid_input");

    // Explicit --pid keeps the lease bound to a live process the test controls.
    // Auto-resolution needs a harness ancestor; bare cargo test often has none.
    let supervisor = std::process::id().to_string();
    let claimed = run(
        &home,
        &[
            "claim",
            "--writer",
            "--repo",
            repo_arg,
            "--session",
            "stable-session",
            "--pid",
            &supervisor,
            "--json",
        ],
    );
    assert!(claimed.status.success());
    let claimed = json(&claimed);
    let resource = &claimed["lease"]["resource"];
    assert_eq!(resource["kind"], "writer");
    assert!(
        resource["git_common_dir"]
            .as_str()
            .unwrap()
            .ends_with("/.git")
    );
    assert_eq!(resource["worktree"], repo_arg);
    assert_eq!(claimed["lease"]["session"], "stable-session");
    assert_eq!(
        claimed["lease"]["pid"].as_u64(),
        Some(supervisor.parse().unwrap())
    );
}

#[test]
fn writer_without_pid_fails_closed_when_no_harness_supervisor() {
    let home = tmp_home();
    let repo = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let repo_arg = repo.to_str().unwrap();
    let output = run(
        &home,
        &[
            "claim",
            "--writer",
            "--repo",
            repo_arg,
            "--session",
            "stable-session",
            "--json",
        ],
    );
    // Under Cursor/Claude/Codex the ancestry walk may succeed; that is correct.
    // Outside a harness it must refuse rather than bind a dead tool-shell pid.
    if output.status.success() {
        let claimed = json(&output);
        assert!(claimed["lease"]["pid"].as_u64().is_some());
        assert_ne!(
            claimed["lease"]["pid"].as_u64(),
            Some(0),
            "supervisor pid must be live and non-zero"
        );
    } else {
        assert_eq!(output.status.code(), Some(1));
        assert_eq!(json(&output)["error"], "invalid_input");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("harness supervisor") || stderr.contains("--pid"),
            "actionable stderr: {stderr}"
        );
    }
}

#[test]
fn service_claim_does_not_infer_pid() {
    let home = tmp_home();
    let claimed = claim_service(&home, "no-auto-pid-service");
    assert!(
        claimed["lease"]["pid"].is_null(),
        "service leases must not bind the ephemeral CLI pid"
    );
}

#[test]
fn corrupt_state_is_reported_without_overwrite() {
    let home = tmp_home();
    let claimed = claim_service(&home, "identity.northwind.localhost");
    assert_eq!(claimed["status"], "acquired");
    let service_dir = home.join("leases/v1/service");
    let state = std::fs::read_dir(&service_dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    std::fs::write(&state, b"corrupt-sentinel").unwrap();

    let output = run(
        &home,
        &[
            "claim",
            "--service",
            "identity.northwind.localhost",
            "--json",
        ],
    );
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(json(&output)["error"], "corrupt_state");
    assert_eq!(std::fs::read(&state).unwrap(), b"corrupt-sentinel");

    let doctor = run(&home, &["doctor", "--json"]);
    assert_eq!(doctor.status.code(), Some(1));
    let doctor = json(&doctor);
    assert_eq!(doctor["leases"]["healthy"], false);
    assert_eq!(doctor["leases"]["corrupt"], 1);
}

#[test]
fn lease_files_do_not_capture_argv_environment_or_free_text() {
    let home = tmp_home();
    let output = run(
        &home,
        &[
            "claim",
            "--service",
            "safe-service-name",
            "--session",
            "safe-session",
            "--json",
        ],
    );
    assert!(output.status.success());
    let state = std::fs::read_dir(home.join("leases/v1/service"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .unwrap();
    let contents = std::fs::read_to_string(state).unwrap();
    assert!(!contents.contains("must-never-enter-lease-state"));
    assert!(!contents.contains("SECRET_SENTINEL"));
    assert!(!contents.contains("argv"));
    assert!(!contents.contains("env"));
    assert!(!contents.contains("text"));
    let lease: serde_json::Value = serde_json::from_str(&contents).unwrap();
    assert!(lease.get("repository_id").is_none());
    assert!(lease.get("branch").is_none());
    assert!(lease.get("head_sha").is_none());
}

#[test]
fn doctor_reports_lease_counts() {
    let home = tmp_home();
    claim_service(&home, "doctor-service");
    let doctor = run(&home, &["doctor", "--json"]);
    assert!(doctor.status.success());
    let doctor = json(&doctor);
    assert_eq!(doctor["leases"]["total"], 1);
    assert_eq!(doctor["leases"]["active"], 1);
    assert_eq!(doctor["leases"]["corrupt"], 0);
    assert_eq!(doctor["leases"]["healthy"], true);
}

#[test]
fn invalid_check_token_and_dead_pid_are_input_errors() {
    let home = tmp_home();
    claim_service(&home, "validation-service");
    let invalid = run(
        &home,
        &[
            "check",
            "--service",
            "validation-service",
            "--claim-id",
            "not-a-ulid",
            "--json",
        ],
    );
    assert_eq!(invalid.status.code(), Some(1));
    assert_eq!(json(&invalid)["error"], "invalid_input");

    let dead = run(
        &home,
        &[
            "claim",
            "--service",
            "dead-pid-service",
            "--pid",
            "2147483647",
            "--json",
        ],
    );
    assert_eq!(dead.status.code(), Some(1));
    assert_eq!(json(&dead)["error"], "invalid_input");
}

#[test]
fn check_hides_token_without_exact_proof() {
    let home = tmp_home();
    let claimed = claim_service(&home, "redaction-service");
    let claim_id = claimed["lease"]["claim_id"].as_str().unwrap();

    let anonymous = run(
        &home,
        &["check", "--service", "redaction-service", "--json"],
    );
    assert!(anonymous.status.success());
    assert!(json(&anonymous)["lease"].get("claim_id").is_none());

    let wrong = run(
        &home,
        &[
            "check",
            "--service",
            "redaction-service",
            "--claim-id",
            "01ARZ3NDEKTSV4RRFFQ69G5FAV",
            "--json",
        ],
    );
    assert_eq!(wrong.status.code(), Some(2));
    assert!(json(&wrong)["lease"].get("claim_id").is_none());

    let owner = run(
        &home,
        &[
            "check",
            "--service",
            "redaction-service",
            "--claim-id",
            claim_id,
            "--json",
        ],
    );
    assert!(owner.status.success());
    assert_eq!(json(&owner)["lease"]["claim_id"], claim_id);
}

#[test]
fn one_of_one_hundred_processes_acquires_shared_service() {
    let home = Arc::new(tmp_home());
    let barrier = Arc::new(Barrier::new(100));
    let mut handles = Vec::new();
    for index in 0..100 {
        let home = Arc::clone(&home);
        let barrier = Arc::clone(&barrier);
        handles.push(std::thread::spawn(move || {
            barrier.wait();
            run(
                &home,
                &[
                    "claim",
                    "--service",
                    "multiprocess-service",
                    "--session",
                    &format!("process-{index}"),
                    "--json",
                ],
            )
        }));
    }
    let outputs: Vec<_> = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect();
    assert_eq!(
        outputs
            .iter()
            .filter(|output| output.status.success())
            .count(),
        1
    );
    assert_eq!(
        outputs
            .iter()
            .filter(|output| output.status.code() == Some(2))
            .count(),
        99
    );
    assert_eq!(
        outputs
            .iter()
            .filter(|output| json(output)["status"] == "acquired")
            .count(),
        1
    );
    assert_eq!(
        outputs
            .iter()
            .filter(|output| json(output)["status"] == "conflict")
            .count(),
        99
    );

    let states: Vec<_> = std::fs::read_dir(home.join("leases/v1/service"))
        .unwrap()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    assert_eq!(states.len(), 1);
    let _: serde_json::Value = serde_json::from_slice(&std::fs::read(&states[0]).unwrap()).unwrap();
}

#[test]
fn human_output_is_stable_and_concise() {
    let home = tmp_home();
    let output = run(&home, &["claim", "--service", "human-service"]);
    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.starts_with("acquired service "));
    assert!(
        stdout
            .trim_end()
            .ends_with(|character: char| character.is_ascii_digit())
    );
}
