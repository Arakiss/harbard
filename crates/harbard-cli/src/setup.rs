//! First-run setup: wizard state (`setup.json`) plus the CLI orchestrator
//! `harbard setup` — detect present harnesses, wire them via `install::apply`,
//! mark complete. Same surfaces the console Setup API uses; no parallel path.

use std::fs;
use std::io;
use std::path::PathBuf;

use harbard_core::Store;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::install;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SetupState {
    /// When the wizard was finished or skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_ms: Option<u64>,
    /// Targets the operator chose not to wire.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<String>,
    /// Targets successfully wired during Setup (informational).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub wired: Vec<String>,
}

fn path(store: &Store) -> PathBuf {
    store.root().join("setup.json")
}

pub fn read(store: &Store) -> SetupState {
    let p = path(store);
    let Ok(raw) = fs::read_to_string(&p) else {
        return SetupState::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn write(store: &Store, state: &SetupState) -> io::Result<()> {
    fs::create_dir_all(store.root())?;
    let pretty = serde_json::to_string_pretty(state)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let tmp = store.root().join("setup.json.tmp");
    fs::write(&tmp, format!("{pretty}\n"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600));
    }
    fs::rename(&tmp, path(store))?;
    Ok(())
}

/// Full setup payload for `GET /api/v1/setup`.
///
/// `needs_setup` is true when the wizard has not completed and there is still
/// something useful to do — either a present harness missing hooks, or a cold
/// machine with nothing detected yet. Already-wired machines (CLI install)
/// skip the wizard without writing `setup.json`.
pub fn status(store: &Store) -> Value {
    let state = read(store);
    let detect = crate::install::detect();
    let harnesses = detect
        .get("harnesses")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let any_present = harnesses
        .iter()
        .any(|h| h.get("present").and_then(|v| v.as_bool()).unwrap_or(false));
    let any_needs_wire = harnesses.iter().any(|h| {
        let present = h.get("present").and_then(|v| v.as_bool()).unwrap_or(false);
        let ok = h.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        present && !ok
    });

    let completed = state.completed_ms.is_some();
    let needs_setup = if completed {
        false
    } else if any_needs_wire {
        true
    } else if any_present {
        // Present and already wired via CLI — no wizard nag.
        false
    } else {
        // Cold machine: still offer the welcome once.
        true
    };

    json!({
        "needs_setup": needs_setup,
        "completed_ms": state.completed_ms,
        "skipped": state.skipped,
        "wired": state.wired,
        "harnesses": harnesses,
        "loopback_install": true,
    })
}

const TARGETS: [&str; 3] = ["claude", "codex", "cursor"];

/// Hooks map the operator (or `--json` consumer) sees in a setup plan.
fn hooks_plan(target: &str) -> Value {
    let plan = install::dry_run_json(target);
    plan.get("hooks").cloned().unwrap_or_else(|| json!({}))
}

fn summary_from(harnesses: &[Value]) -> Value {
    let present = harnesses
        .iter()
        .filter(|h| h.get("present").and_then(|v| v.as_bool()).unwrap_or(false))
        .count();
    let absent = harnesses
        .iter()
        .filter(|h| !h.get("present").and_then(|v| v.as_bool()).unwrap_or(false))
        .count();
    // Present + healthy (or dry-run projected healthy). Never counts absent.
    let wired_ok = harnesses
        .iter()
        .filter(|h| {
            let present = h.get("present").and_then(|v| v.as_bool()).unwrap_or(false);
            let ok = h.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            present && ok
        })
        .count();
    let status = format!("{wired_ok} of {present}");
    json!({
        "present": present,
        "wired_ok": wired_ok,
        "absent": absent,
        "status": status,
    })
}

/// Detect and wire one harness. Both the CLI orchestrator and the local Setup
/// API call this function, so the desktop wizard cannot drift from `harbard
/// setup`.
pub fn wire_target(target: &str, dry_run: bool) -> io::Result<Value> {
    if !TARGETS.contains(&target) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "unknown target; expected claude|codex|cursor",
        ));
    }
    let present = install::target_present(target);
    let hooks = hooks_plan(target);
    let path = install::default_config_path(target);

    if !present {
        return Ok(json!({
            "target": target,
            "present": false,
            "action": "absent",
            "ok": false,
            "hooks": hooks,
            "path": path,
        }));
    }

    if dry_run {
        let changed = install::would_change(&path, target)?;
        let ok = changed
            || install::health_one(target)
                .get("ok")
                .and_then(Value::as_bool)
                .unwrap_or(false);
        return Ok(json!({
            "target": target,
            "present": true,
            "action": if changed { "would_install" } else { "unchanged" },
            "ok": ok,
            "hooks": hooks,
            "path": path,
        }));
    }

    let applied = install::apply(&path, target)?;
    let action = if applied.changed {
        "installed"
    } else {
        "unchanged"
    };
    let ok = install::health_one(target)
        .get("ok")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if !ok {
        return Err(io::Error::other(format!(
            "setup: {target} applied but hooks are not healthy — run harbard doctor"
        )));
    }
    Ok(json!({
        "target": target,
        "present": true,
        "action": action,
        "ok": true,
        "hooks": hooks,
        "path": applied.path,
        "backup": applied.backup,
        "events": applied.events,
    }))
}

/// Run professional setup: wire every *present* harness, preserve foreign
/// hooks, stay byte-idempotent, persist `setup.json` when not dry-run.
///
/// Absent harnesses are reported and skipped — never listed under `wired`.
/// Report ends with `summary.status` like `"3 of 3"` (present only).
/// The setup target a `--harness` flag names: `claude-code` (or `claude`),
/// `codex` or `cursor`. Anything else cannot be wired.
pub fn target_of(harness: &str) -> Result<&'static str, String> {
    match harness {
        "claude-code" | "claude" => Ok("claude"),
        "codex" => Ok("codex"),
        "cursor" => Ok("cursor"),
        other => Err(format!(
            "setup wires claude-code, codex or cursor; \"{other}\" is none of them"
        )),
    }
}

/// Every file a harness's wiring writes or copies: the config, and its
/// backup when an existing config changes.
fn files_of(result: &Value) -> Vec<Value> {
    let action = result["action"].as_str().unwrap_or("");
    if !matches!(action, "would_install" | "installed") {
        return Vec::new();
    }
    let path = result["path"].as_str().unwrap_or("").to_string();
    let mut files = vec![json!({ "path": path, "action": "write" })];
    match result.get("backup").and_then(Value::as_str) {
        Some(backup) => files.push(json!({ "path": backup, "action": "backup" })),
        None if action == "would_install" && std::path::Path::new(&path).exists() => {
            files.push(json!({ "path": format!("{path}.bak-<ms>"), "action": "backup" }))
        }
        None => {}
    }
    files
}

/// `only`: wire this target alone (`harbard setup --harness <h>`); the
/// others are neither touched nor reported, and `setup.json` keeps what it
/// said about them.
pub fn run(store: &Store, dry_run: bool, only: Option<&str>, now_ms: u64) -> io::Result<Value> {
    let targets: Vec<&str> = TARGETS
        .iter()
        .copied()
        .filter(|t| only.is_none_or(|o| o == *t))
        .collect();
    let mut harnesses = Vec::with_capacity(targets.len());
    let mut wired = Vec::new();
    let mut absent = Vec::new();
    let mut files = Vec::new();

    for target in &targets {
        let result = wire_target(target, dry_run)?;
        if !result["present"].as_bool().unwrap_or(false) {
            absent.push(target.to_string());
        } else if !dry_run && result["ok"].as_bool().unwrap_or(false) {
            wired.push(target.to_string());
        }
        files.extend(files_of(&result));
        harnesses.push(result);
    }
    files.push(json!({ "path": path(store), "action": "write" }));

    let summary = summary_from(&harnesses);
    let all_present_ok = summary
        .get("wired_ok")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        == summary.get("present").and_then(|v| v.as_u64()).unwrap_or(0);

    if !dry_run {
        // A pass over some targets keeps what setup.json said of the others.
        let before = if only.is_some() {
            read(store)
        } else {
            SetupState::default()
        };
        let keep = |list: &[String]| -> Vec<String> {
            list.iter()
                .filter(|t| !targets.contains(&t.as_str()))
                .cloned()
                .collect()
        };
        let mut skipped = keep(&before.skipped);
        // Absent = not wired this pass (not operator "skip", but not installed).
        skipped.extend(absent.iter().cloned());
        let mut all_wired = keep(&before.wired);
        all_wired.extend(wired.iter().cloned());
        write(
            store,
            &SetupState {
                completed_ms: Some(now_ms),
                skipped,
                wired: all_wired,
            },
        )?;
    }

    Ok(json!({
        "dry_run": dry_run,
        "ok": all_present_ok,
        "summary": summary,
        "harnesses": harnesses,
        "wired": if dry_run { Value::Null } else { json!(wired) },
        "absent": absent,
        "files": files,
    }))
}

/// Human lines for `harbard setup` without `--json`.
pub fn render_text(report: &Value) -> String {
    let dry = report
        .get("dry_run")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let mut out = if dry {
        String::from("setup · dry-run (no writes)\n")
    } else {
        String::from("setup · wiring present harnesses\n")
    };
    if let Some(list) = report.get("harnesses").and_then(|v| v.as_array()) {
        for h in list {
            let target = h.get("target").and_then(|v| v.as_str()).unwrap_or("?");
            let action = h.get("action").and_then(|v| v.as_str()).unwrap_or("?");
            let present = h.get("present").and_then(|v| v.as_bool()).unwrap_or(false);
            if !present {
                out.push_str(&format!(
                    "  {target:8} absent — not on this machine (not installed)\n"
                ));
            } else {
                out.push_str(&format!("  {target:8} {action}\n"));
            }
        }
    }
    for f in report
        .get("files")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let path = f.get("path").and_then(Value::as_str).unwrap_or("?");
        let verb = match (f.get("action").and_then(Value::as_str), dry) {
            (Some("backup"), true) => "would back up to",
            (Some("backup"), false) => "backed up to",
            (_, true) => "would write",
            (_, false) => "wrote",
        };
        out.push_str(&format!("  {verb} {path}\n"));
    }
    let status = report
        .pointer("/summary/status")
        .and_then(|v| v.as_str())
        .unwrap_or("?");
    let absent_n = report
        .pointer("/summary/absent")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if absent_n > 0 {
        out.push_str(&format!(
            "doctor · hooks {status} present ok · {absent_n} absent\n"
        ));
    } else {
        out.push_str(&format!("doctor · hooks {status}\n"));
    }
    if !dry {
        out.push_str("done · setup.json written\n");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_setup_state() {
        let dir = std::env::temp_dir().join(format!(
            "harbard-setup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let store = Store::at(&dir);
        assert!(read(&store).completed_ms.is_none());
        write(
            &store,
            &SetupState {
                completed_ms: Some(42),
                skipped: vec!["cursor".into()],
                wired: vec!["claude".into()],
            },
        )
        .unwrap();
        let again = read(&store);
        assert_eq!(again.completed_ms, Some(42));
        assert_eq!(again.skipped, vec!["cursor"]);
        assert_eq!(again.wired, vec!["claude"]);
        let _ = fs::remove_dir_all(&dir);
    }
}
