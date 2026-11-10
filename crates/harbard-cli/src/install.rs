//! `install`: wire a harness's hooks to harbard verbs.
//!
//! Claude Code and Codex are wired at session level only: SessionStart →
//! `hello` + `protocol` + `brief`; UserPromptSubmit → `brief --touch`;
//! Stop → `stop-check` + `watch`; SessionEnd → `bye` (releases the writer
//! lease). Cursor additionally keeps `brief --inline` on postToolUse. The
//! per-tool-call PreToolUse brief and the SubagentStart/SubagentStop pair are
//! no longer wired on Claude Code or Codex: they were most of Harbard's hook
//! traffic, and in-process subagents report to their parent natively. The
//! `brief --inline` verb itself stays.
//!
//! Stop fires once per turn, not per tool call, and both of its verbs print
//! nothing unless the session owes an answer (`stop-check`) or something
//! addressed to it lands while it is idle (`watch`). Claude Code runs `watch`
//! as an `asyncRewake` hook, which wakes the idle session on exit 2; Codex has
//! no rewake, so its Stop runs `watch --detach` and the watcher delivers
//! through `codex queue`.
//!
//! The protocol card is the single agent-facing source of bus grammar, printed
//! on demand; SessionStart's `protocol --hook` injects only a one-line pointer
//! to it. The brief path speaks when a note is addressed to the session, a
//! rival writer shares its checkout, a dependency resolves or a handoff sits
//! unacked; contested checkout and unhealthy hooks are said at SessionStart
//! only. Appends only its own hook group (marked
//! `# harbard-managed`), is idempotent, and never touches other groups.
//! Install converges: a managed entry on any event the current wiring no
//! longer uses is removed, so reinstalling over an older config retires it.
//! A timestamped `.bak` is written only when an existing config will actually
//! change. Presence/brief commands are guarded with `|| true` so the bus never
//! breaks a session. Writer admission belongs to the host's
//! safety adapter, not to Harbard's presence and delivery hook group.
//!
//! Cursor: `stop` fires at the end of every agent *turn*, not the conversation.
//! Bye on `stop` marked sessions ended while the chat was still live. Bye
//! rides `sessionEnd` instead. Claude Code and Codex: same — bye rides
//! `SessionEnd`, never the per-turn `Stop`. `bye` itself is idempotent so
//! double end events are a no-op once the session has departed.
//!
//! Manual Cursor User Rules / cloud-synced rule storage are out of scope: the
//! installer only merges local hook configs (`~/.cursor/hooks.json` and peers).
//! SessionStart injects the protocol pointer, so a pasted User Rule is no
//! longer required — remove any leftover paste in Cursor Settings yourself.

use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Marker comment appended to every harbard-managed hook command. Harmless to
/// the shell (a trailing `#` comment), precise for idempotent detection.
const MARKER: &str = "# harbard-managed";
/// Pre-rename marker — still treated as ours so reinstall retires old entries.
const LEGACY_MARKER: &str = "# stefna-managed";
/// Per-command timeout in seconds (Claude / Cursor / most Codex events).
const TIMEOUT_SECS: u64 = 5;
/// Codex `SessionEnd` only: official docs default 1s, hard max 3s. Emitting 5
/// either gets clamped or warns; keep the generator inside the documented cap.
const CODEX_SESSION_END_TIMEOUT_SECS: u64 = 3;

/// Cursor's postToolUse inline brief only (Claude Code and Codex no longer
/// wire a per-tool-call hook): how stale an inline check may be before the
/// hot path reads the store again.
/// Chosen so the operator's message lands within a few tool calls while a busy
/// agent does not hit the store on every single one.
const INLINE_THROTTLE_SECS: u64 = 20;

/// Wired events and ordered verb commands. Claude Code and Codex share one
/// hook surface (PascalCase events, nested groups); Cursor's is its own
/// (camelCase events, flat entries) — the verbs are identical, only the
/// envelope differs.
///
/// Cursor specifics, by event:
/// - `sessionStart` injects `additional_context`, so the brief rides there.
/// - `beforeSubmitPrompt` has no injection field; the touch still runs for
///   its side effects (heartbeat, self-heal hello, alias naming) with its
///   render discarded.
/// - `postToolUse` carries inline delivery. Harbard installs no admission
///   control; host safety policy owns any deny-capable hook.
/// - `sessionEnd` (not `stop`) carries bye: `stop` is per-turn and would mark
///   the session ended after every reply.
fn wiring(target: &str) -> Vec<(&'static str, Vec<HookCmd>)> {
    let flag = harness_flag(target);
    if target == "cursor" {
        vec![
            (
                "sessionStart",
                vec![
                    soft(&format!("hello --hook {flag}")),
                    soft(&format!("protocol --hook {flag}")),
                    soft(&format!("brief --hook {flag}")),
                ],
            ),
            (
                "beforeSubmitPrompt",
                vec![soft(&format!("brief --touch --hook {flag}"))],
            ),
            (
                "postToolUse",
                vec![soft(&format!(
                    "brief --inline --throttle {INLINE_THROTTLE_SECS} --hook {flag}"
                ))],
            ),
            ("sessionEnd", vec![soft(&format!("bye --hook {flag}"))]),
        ]
    } else {
        // Claude Code and Codex: session-level wiring only. Stop fires every
        // turn on both, so bye rides SessionEnd, the real departure; Stop
        // carries only the two verbs that stay silent unless something is
        // owed or lands. No PreToolUse brief (it ran before every tool call)
        // and no hello or bye for a subagent: in-process subagents report to
        // their parent instead of joining the bus. Separately launched herd
        // children still fire their own SessionStart. On Claude Code a starting
        // subagent is told who reads its reports, and nothing else.
        let (session_end, watcher) = if target == "claude" {
            (
                soft(&format!("bye --hook {flag}")),
                rewake(
                    &format!("watch --hook {flag}"),
                    crate::watch::HOOK_TIMEOUT_SECS,
                ),
            )
        } else {
            (
                // Codex caps SessionEnd hooks at 3s (official hooks docs).
                soft_with_timeout(
                    &format!("bye --hook {flag}"),
                    CODEX_SESSION_END_TIMEOUT_SECS,
                ),
                soft(&format!("watch --detach --hook {flag}")),
            )
        };
        let mut wired = vec![
            (
                "SessionStart",
                vec![
                    soft(&format!("hello --hook {flag}")),
                    soft(&format!("protocol --hook {flag}")),
                    soft(&format!("brief --hook {flag}")),
                ],
            ),
            (
                "UserPromptSubmit",
                vec![soft(&format!("brief --touch --hook {flag}"))],
            ),
            (
                "Stop",
                vec![soft(&format!("stop-check --hook {flag}")), watcher],
            ),
            ("SessionEnd", vec![session_end]),
        ];
        if target == "claude" {
            wired.push((
                "SubagentStart",
                vec![soft(&format!("protocol --subagent --hook {flag}"))],
            ));
        }
        wired
    }
}

#[derive(Debug, Clone)]
struct HookCmd {
    command: String,
    timeout: u64,
    /// Claude Code `asyncRewake`: run in the background, wake on exit 2.
    rewake: bool,
}

/// Soft bus command: missing binary or verb failure must not break the session.
fn soft(verb_and_args: &str) -> HookCmd {
    soft_with_timeout(verb_and_args, TIMEOUT_SECS)
}

fn soft_with_timeout(verb_and_args: &str, timeout: u64) -> HookCmd {
    HookCmd {
        command: format!(
            "command -v harbard >/dev/null 2>&1 && harbard {verb_and_args} || true {MARKER}"
        ),
        timeout,
        rewake: false,
    }
}

/// A background command whose exit 2 is the message. The soft form cannot
/// carry it: `|| true` turns exit 2 into 0, and the wake never happens. A
/// missing binary still exits 0 and says nothing; `exec` leaves no shell
/// behind, so the harness's timeout and kill reach harbard itself.
fn rewake(verb_and_args: &str, timeout: u64) -> HookCmd {
    HookCmd {
        command: format!(
            "command -v harbard >/dev/null 2>&1 || exit 0; exec harbard {verb_and_args} {MARKER}"
        ),
        timeout,
        rewake: true,
    }
}

/// One hook entry `{type, command, timeout[, asyncRewake]}`.
fn entry(c: &HookCmd) -> Value {
    let mut v = json!({
        "type": "command",
        "command": c.command,
        "timeout": c.timeout,
    });
    if c.rewake {
        v["asyncRewake"] = json!(true);
    }
    v
}

/// Build one claude-style hook group `{ "hooks": [ {type,command,timeout} ] }`.
fn group(commands: &[HookCmd]) -> Value {
    let hooks: Vec<Value> = commands.iter().map(entry).collect();
    json!({ "hooks": hooks })
}

/// Cursor's flat entries: each command is its own `{command, timeout}` object
/// directly in the event array — no nested group, `type` defaults to command.
/// Harbard-managed delivery hooks are always fail-open: bus failure must not
/// break a harness session.
fn cursor_entries(commands: &[HookCmd]) -> Vec<Value> {
    commands
        .iter()
        .map(|c| json!({ "command": c.command, "timeout": c.timeout }))
        .collect()
}

/// The install target's harness flag value.
fn harness_flag(target: &str) -> &'static str {
    match target {
        "claude" => "claude-code",
        "cursor" => "cursor",
        _ => "codex",
    }
}

/// The exact JSON harbard would add, for `--dry-run`.
pub fn dry_run_json(target: &str) -> Value {
    let mut hooks = serde_json::Map::new();
    for (event, cmds) in wiring(target) {
        if target == "cursor" {
            hooks.insert(event.to_string(), json!(cursor_entries(&cmds)));
        } else {
            hooks.insert(event.to_string(), json!([group(&cmds)]));
        }
    }
    if target == "cursor" {
        json!({ "version": 1, "hooks": hooks })
    } else {
        json!({ "hooks": hooks })
    }
}

/// Default config path for a target: `~/.claude/settings.json` (claude),
/// `$CODEX_HOME/hooks.json` (codex, `~/.codex` by default), or
/// `~/.cursor/hooks.json` (cursor).
pub fn default_config_path(target: &str) -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    match target {
        "claude" => home.join(".claude").join("settings.json"),
        "cursor" => home.join(".cursor").join("hooks.json"),
        _ => codex_home().join("hooks.json"),
    }
}

/// Outcome of a real install.
#[derive(Debug)]
pub struct Applied {
    pub path: PathBuf,
    pub backup: Option<PathBuf>,
    pub events: Vec<String>,
    pub changed: bool,
}

/// Whether each install target still has harbard's managed hooks.
///
/// This is the DX answer to "will they hear the bus?": agents do not peer-DM;
/// they receive notes when a hook injects a brief. Missing wiring looks like a
/// broken bus in the console.
pub fn health() -> Value {
    let harnesses: Vec<Value> = ["claude", "codex", "cursor"]
        .into_iter()
        .map(health_one)
        .collect();
    let healthy = hooks_rollup_ok(&harnesses);
    json!({ "healthy": healthy, "harnesses": harnesses })
}

/// Presence + health for first-run Setup: which supported harnesses look
/// installed on this machine, and which still need `harbard install`.
pub fn detect() -> Value {
    let harnesses: Vec<Value> = ["claude", "codex", "cursor"]
        .into_iter()
        .map(|t| {
            let mut h = health_one(t);
            let present = target_present(t);
            let ok = h.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
            if let Some(obj) = h.as_object_mut() {
                obj.insert("present".into(), json!(present));
                obj.insert("recommended".into(), json!(present && !ok));
                obj.insert("blurb".into(), json!(target_blurb(t)));
            }
            h
        })
        .collect();
    json!({ "harnesses": harnesses })
}

fn target_blurb(target: &str) -> &'static str {
    match target {
        "claude" => {
            "Claude Code hooks call harbard at session start, on each prompt (presence and the turn brief), at the end of each turn (owed answers, and a background watcher that wakes the idle session), and at session end."
        }
        "codex" => {
            "Codex hooks call harbard at session start, on each prompt (presence and the turn brief), at the end of each turn (owed answers, and a detached watcher that wakes the idle thread through codex queue), and at SessionEnd."
        }
        "cursor" => {
            "Cursor hooks inject presence/protocol/briefs, adapt inline delivery on postToolUse, and mark departure on sessionEnd."
        }
        _ => "Wire this harness so agents on it can hear the bus.",
    }
}

/// True when the harness looks installed: binary on PATH and/or a config home.
pub fn target_present(target: &str) -> bool {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    match target {
        "claude" => which_bin("claude") || home.join(".claude").is_dir(),
        "codex" => which_bin("codex") || codex_home().is_dir(),
        "cursor" => {
            which_bin("cursor") || which_bin("cursor-agent") || home.join(".cursor").is_dir()
        }
        _ => false,
    }
}

fn which_bin(bin: &str) -> bool {
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .any(|dir| dir.join(bin).is_file())
}

/// Human-readable block appended to `harbard doctor` text output.
pub fn health_text(report: &Value) -> String {
    let Some(harnesses) = report.get("harnesses").and_then(|v| v.as_array()) else {
        return String::new();
    };
    let mut lines = String::from("\nhooks\n");
    for h in harnesses {
        let target = h.get("target").and_then(|v| v.as_str()).unwrap_or("?");
        let ok = h.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        let path = h.get("path").and_then(|v| v.as_str()).unwrap_or("-");
        if ok {
            lines.push_str(&format!("  {target:<8} ok  ({path})\n"));
        } else {
            let hint = h
                .get("hint")
                .and_then(|v| v.as_str())
                .unwrap_or("harbard install");
            lines.push_str(&format!("  {target:<8} NEED INSTALL — {hint}\n"));
            lines.push_str(&format!("           {path}\n"));
            if h.get("wake_timeout_short").and_then(Value::as_bool) == Some(true) {
                lines.push_str(
                    "           rewake timeout is shorter than the 24-hour idle horizon\n",
                );
            }
            for key in ["missing_commands", "unexpected_commands"] {
                for item in h
                    .get(key)
                    .and_then(|v| v.as_array())
                    .into_iter()
                    .flatten()
                    .filter_map(|v| v.as_str())
                {
                    let label = key.replace('_', " ");
                    lines.push_str(&format!("           {label}: {item}\n"));
                }
            }
        }
        if let Some(note) = h
            .get("wake")
            .and_then(|w| w.get("note"))
            .and_then(|v| v.as_str())
        {
            lines.push_str(&format!("           wake: {note}\n"));
        }
    }
    lines
}

pub fn health_one(target: &str) -> Value {
    let mut report = health_at(&default_config_path(target), target);
    if target == "codex"
        && let Some(obj) = report.as_object_mut()
    {
        obj.insert("wake".into(), codex_wake_health(&codex_home()));
    }
    report
}

/// Events whose every wired command must be present, not just one of them.
///
/// Stop's two verbs are the whole feature: a config holding only one of them,
/// or an older managed command such as the retired bye-on-Stop, would read
/// healthy under the per-event rule while waking nobody. Session events keep
/// the looser rule because the operator has trimmed them by hand before and
/// that was accepted as healthy.
const COMPLETE_EVENTS: [&str; 1] = ["Stop"];

/// The identity of one managed hook entry: its command and whether it
/// rewakes. Timeouts are left out so a tuned timeout is not a miswiring.
fn signature(entry: &Value) -> Option<(String, bool)> {
    let command = entry.get("command")?.as_str()?;
    let rewake = entry
        .get("asyncRewake")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    Some((command.to_string(), rewake))
}

/// Managed entries on one event, flattened across both envelopes.
fn managed_signatures(groups: &Value) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for g in groups.as_array().into_iter().flatten() {
        if let Some(hooks) = g.get("hooks").and_then(|h| h.as_array()) {
            out.extend(hooks.iter().filter_map(signature));
        } else if let Some(sig) = signature(g) {
            out.push(sig);
        }
    }
    out.retain(|(command, _)| command.contains(MARKER) || command.contains(LEGACY_MARKER));
    out
}

fn health_at(path: &Path, target: &str) -> Value {
    let wired = wiring(target);
    let expected: Vec<String> = wired.iter().map(|(event, _)| event.to_string()).collect();
    let hint = format!("harbard install {target}");

    if !path.exists() {
        return json!({
            "target": target,
            "harness": harness_flag(target),
            "path": path.display().to_string(),
            "config_exists": false,
            "ok": false,
            "missing_events": expected,
            "missing_commands": [],
            "unexpected_commands": [],
            "retired_lingering": [],
            "hint": hint,
        });
    }

    let root: Value = match std::fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => serde_json::from_str(&s).unwrap_or(json!({})),
        _ => json!({}),
    };
    let hooks = root.get("hooks").cloned().unwrap_or(json!({}));

    let has_ours = |event: &str| {
        hooks
            .get(event)
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().any(is_ours))
            .unwrap_or(false)
    };
    let missing_events: Vec<String> = expected
        .iter()
        .filter(|event| !has_ours(event))
        .cloned()
        .collect();
    // Any managed entry outside the current wiring is a leftover from an older
    // install (bye-on-Stop, PreToolUse inline brief, Subagent pair, gate).
    let retired_lingering: Vec<String> = hooks
        .as_object()
        .map(|events| {
            events
                .keys()
                .filter(|event| !expected.contains(event) && has_ours(event))
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    // Exactly the wired set: on a wired event, a managed command the current
    // wiring does not produce is a leftover, and on a complete event every
    // wired command must be there.
    let mut missing_commands: Vec<String> = Vec::new();
    let mut unexpected_commands: Vec<String> = Vec::new();
    for (event, cmds) in &wired {
        let want: Vec<(String, bool)> =
            cmds.iter().map(|c| (c.command.clone(), c.rewake)).collect();
        let have = managed_signatures(hooks.get(*event).unwrap_or(&Value::Null));
        for sig in &have {
            if !want.contains(sig) {
                unexpected_commands.push(format!("{event}: {}", sig.0));
            }
        }
        if COMPLETE_EVENTS.contains(event) {
            for sig in &want {
                if !have.contains(sig) {
                    missing_commands.push(format!("{event}: {}", sig.0));
                }
            }
        }
    }
    // Command names alone are insufficient: a shorter asyncRewake timeout
    // kills the watcher before its promised idle horizon even with correct verbs.
    let wake_timeout_short = target == "claude"
        && hooks
            .get("Stop")
            .and_then(Value::as_array)
            .is_some_and(|groups| {
                groups.iter().any(|group| {
                    group
                        .get("hooks")
                        .and_then(Value::as_array)
                        .is_some_and(|entries| {
                            entries.iter().any(|entry| {
                                is_ours(entry)
                                    && entry.get("asyncRewake").and_then(Value::as_bool)
                                        == Some(true)
                                    && entry.get("timeout").and_then(Value::as_u64).unwrap_or(600)
                                        < crate::watch::HOOK_TIMEOUT_SECS
                            })
                        })
                })
            });

    let ok = missing_events.is_empty()
        && retired_lingering.is_empty()
        && missing_commands.is_empty()
        && unexpected_commands.is_empty()
        && !wake_timeout_short;
    json!({
        "target": target,
        "harness": harness_flag(target),
        "path": path.display().to_string(),
        "config_exists": true,
        "ok": ok,
        "missing_events": missing_events,
        "missing_commands": missing_commands,
        "unexpected_commands": unexpected_commands,
        "retired_lingering": retired_lingering,
        "wake_timeout_short": wake_timeout_short,
        "hint": if ok { Value::Null } else { json!(hint) },
    })
}

/// `$CODEX_HOME`, else `~/.codex` ([`harbard_core::harness_trace::codex_home`]).
fn codex_home() -> PathBuf {
    harbard_core::harness_trace::codex_home().unwrap_or_else(|| PathBuf::from(".codex"))
}

/// True when Codex's app-server daemon answers on its control socket.
///
/// Codex decides the same way, with a connect (codex-rs `tui/src/lib.rs`
/// `maybe_probe_default_daemon_socket`); a socket file alone can be stale.
fn codex_daemon_running(codex_home: &Path) -> bool {
    #[cfg(unix)]
    {
        let socket = codex_home
            .join("app-server-control")
            .join("app-server-control.sock");
        std::os::unix::net::UnixStream::connect(socket).is_ok()
    }
    #[cfg(not(unix))]
    {
        let _ = codex_home;
        false
    }
}

/// How a Codex wake will travel. Informational: a missing daemon slows a wake
/// down but does not stop it, so it never fails the hooks rollup.
fn codex_wake_health(codex_home: &Path) -> Value {
    let on_path = which_bin("codex");
    let daemon = codex_daemon_running(codex_home);
    let note = if !on_path {
        "`codex` is not on PATH: an idle Codex session cannot be woken; it hears the bus at its next prompt"
    } else if daemon {
        "codex queue reaches the thread through the running app-server daemon"
    } else {
        "the Codex app-server daemon is not running: codex queue writes the message through an embedded server, and an open Codex session picks it up on its own poll (about 10 s)"
    };
    json!({
        "via": "codex queue",
        "codex_on_path": on_path,
        "daemon": if daemon { "running" } else { "absent" },
        "note": note,
    })
}

/// Aggregate health: missing configs are reported but do not fail the rollup
/// (an operator who only runs Claude is fine without Cursor hooks). A present
/// config that is missing our events *does* fail — that is miswired.
fn hooks_rollup_ok(harnesses: &[Value]) -> bool {
    harnesses.iter().all(|h| {
        let exists = h
            .get("config_exists")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let ok = h.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
        !exists || ok
    })
}

/// True when a hook entry is one harbard manages (carries the marker), in
/// either envelope: claude's nested group or Cursor's flat `{command}`.
/// Also matches the pre-rename `# stefna-managed` marker so reinstall retires
/// leftover bye-on-stop entries.
fn is_ours(g: &Value) -> bool {
    command_strings(g)
        .iter()
        .any(|c| c.contains(MARKER) || c.contains(LEGACY_MARKER))
}

fn command_strings(g: &Value) -> Vec<&str> {
    let mut out = Vec::new();
    if let Some(c) = g.get("command").and_then(|c| c.as_str()) {
        out.push(c);
    }
    if let Some(hooks) = g.get("hooks").and_then(|h| h.as_array()) {
        for hk in hooks {
            if let Some(c) = hk.get("command").and_then(|c| c.as_str()) {
                out.push(c);
            }
        }
    }
    out
}

fn merged_config(original: Option<&str>, target: &str) -> std::io::Result<(String, Vec<String>)> {
    let mut root: Value = match original {
        Some(s) if !s.trim().is_empty() => serde_json::from_str(s).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("existing config is not valid JSON: {e}"),
            )
        })?,
        _ => json!({}),
    };

    if !root.is_object() {
        root = json!({});
    }
    // Cursor's schema is versioned; claude's settings.json has no such key.
    if target == "cursor" && root.get("version").is_none() {
        root["version"] = json!(1);
    }
    let obj = root.as_object_mut().expect("object");
    let hooks = obj.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let hooks = hooks.as_object_mut().expect("object");

    let wired = wiring(target);
    // Converge first: strip managed entries from every event the current
    // wiring no longer uses (bye-on-Stop, the PreToolUse inline brief, the
    // Subagent pair, a gate). Foreign entries stay. An event array that held
    // only our entries is dropped instead of being left as `[]`.
    let unwired: Vec<String> = hooks
        .keys()
        .filter(|event| !wired.iter().any(|(w, _)| w == event))
        .cloned()
        .collect();
    for event in unwired {
        let Some(arr) = hooks.get_mut(&event).and_then(|v| v.as_array_mut()) else {
            continue;
        };
        let before = arr.len();
        arr.retain(|g| !is_ours(g));
        if arr.is_empty() && before > 0 {
            hooks.remove(&event);
        }
    }

    let mut touched = Vec::new();
    for (event, cmds) in wired {
        let arr = hooks.entry(event.to_string()).or_insert_with(|| json!([]));
        if !arr.is_array() {
            *arr = json!([]);
        }
        let arr = arr.as_array_mut().expect("array");
        arr.retain(|g| !is_ours(g)); // drop our previous entries; keep foreign
        if target == "cursor" {
            arr.extend(cursor_entries(&cmds));
        } else {
            arr.push(group(&cmds));
        }
        touched.push(event.to_string());
    }

    let pretty = serde_json::to_string_pretty(&root)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    Ok((format!("{pretty}\n"), touched))
}

/// True when applying the current wiring would alter the target config.
/// Reads and merges in memory only: setup dry-runs must predict the real write
/// byte-for-byte without creating configs or backups.
pub fn would_change(path: &Path, target: &str) -> std::io::Result<bool> {
    let original = std::fs::read_to_string(path).ok();
    let (rendered, _) = merged_config(original.as_deref(), target)?;
    Ok(original.as_deref() != Some(rendered.as_str()))
}

/// Merge harbard's wiring into `path`, idempotently. Backs up an existing file
/// only when the rendered config changes. Only harbard-managed entries are
/// added or replaced.
pub fn apply(path: &Path, target: &str) -> std::io::Result<Applied> {
    let original = std::fs::read_to_string(path).ok();
    let (rendered, events) = merged_config(original.as_deref(), target)?;
    let changed = original.as_deref() != Some(rendered.as_str());
    let backup = if changed && path.exists() {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let bak = PathBuf::from(format!("{}.bak-{ms}", path.display()));
        std::fs::copy(path, &bak)?;
        Some(bak)
    } else {
        None
    };
    if changed {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, rendered)?;
    }

    Ok(Applied {
        path: path.to_path_buf(),
        backup,
        events,
        changed,
    })
}

/// The config with every harbard-managed entry removed, and the events that
/// lost one. Foreign entries and keys stay exactly where they were.
fn stripped_config(original: &str) -> std::io::Result<(String, Vec<String>)> {
    let mut root: Value = serde_json::from_str(original).map_err(|e| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("existing config is not valid JSON: {e}"),
        )
    })?;
    let mut touched = Vec::new();
    if let Some(hooks) = root.get_mut("hooks").and_then(|h| h.as_object_mut()) {
        let events: Vec<String> = hooks.keys().cloned().collect();
        for event in events {
            let Some(arr) = hooks.get_mut(&event).and_then(|v| v.as_array_mut()) else {
                continue;
            };
            let before = arr.len();
            arr.retain(|g| !is_ours(g));
            if arr.len() != before {
                touched.push(event.clone());
                if arr.is_empty() {
                    hooks.remove(&event);
                }
            }
        }
    }
    let pretty = serde_json::to_string_pretty(&root)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    Ok((format!("{pretty}\n"), touched))
}

/// Uninstall: remove every harbard-managed hook from `path`, whatever wiring
/// generation wrote it. Backs up first when the file changes; a config with
/// nothing of ours is left byte-for-byte alone.
pub fn remove(path: &Path, _target: &str) -> std::io::Result<Applied> {
    let Ok(original) = std::fs::read_to_string(path) else {
        return Ok(Applied {
            path: path.to_path_buf(),
            backup: None,
            events: Vec::new(),
            changed: false,
        });
    };
    let (rendered, events) = stripped_config(&original)?;
    let changed = !events.is_empty();
    let backup = if changed {
        let ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let bak = PathBuf::from(format!("{}.bak-{ms}", path.display()));
        std::fs::copy(path, &bak)?;
        std::fs::write(path, rendered)?;
        Some(bak)
    } else {
        None
    };
    Ok(Applied {
        path: path.to_path_buf(),
        backup,
        events,
        changed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static SEQ: AtomicU64 = AtomicU64::new(0);

    fn tmp() -> PathBuf {
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let d = std::env::temp_dir().join(format!("harbard-install-{}-{}", std::process::id(), n));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("settings.json")
    }

    #[test]
    fn dry_run_has_the_session_events_and_the_subagent_brief() {
        let v = dry_run_json("claude");
        let hooks = v.get("hooks").unwrap().as_object().unwrap();
        assert!(hooks.contains_key("SessionStart"));
        assert!(hooks.contains_key("UserPromptSubmit"));
        assert!(hooks.contains_key("Stop"));
        assert!(hooks.contains_key("SessionEnd"));
        let told = hooks["SubagentStart"][0]["hooks"].as_array().unwrap();
        assert_eq!(told.len(), 1, "a subagent is told one thing");
        assert!(
            told[0]["command"]
                .as_str()
                .unwrap()
                .contains("protocol --subagent")
        );
        assert_eq!(hooks.len(), 5, "and no other event");
        // SessionStart wires hello + protocol + brief.
        let ss = hooks["SessionStart"][0]["hooks"].as_array().unwrap();
        assert_eq!(ss.len(), 3);
        assert!(ss[0]["command"].as_str().unwrap().contains("harbard hello"));
        assert!(
            ss[1]["command"]
                .as_str()
                .unwrap()
                .contains("harbard protocol")
        );
        assert!(ss[2]["command"].as_str().unwrap().contains("harbard brief"));
    }

    /// Protocol arrives via SessionStart on every wired harness. That retires
    /// the old "paste into Cursor User Rules" path without touching cloud
    /// storage — install only writes local hook JSON.
    #[test]
    fn session_start_injects_protocol_on_all_three_harnesses() {
        for (target, event) in [
            ("claude", "SessionStart"),
            ("codex", "SessionStart"),
            ("cursor", "sessionStart"),
        ] {
            let planned = dry_run_json(target);
            let commands = session_start_commands(&planned["hooks"][event], target);
            assert!(
                commands
                    .iter()
                    .any(|c| c.contains("harbard protocol --hook")),
                "{target} {event} must plan protocol inject: {commands:?}"
            );

            let path = tmp();
            let seed = if target == "cursor" {
                r#"{"version":1,"hooks":{}}"#
            } else {
                r#"{"hooks":{}}"#
            };
            std::fs::write(&path, seed).unwrap();
            apply(&path, target).unwrap();
            let written: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let applied = session_start_commands(&written["hooks"][event], target);
            assert!(
                applied
                    .iter()
                    .any(|c| c.contains("harbard protocol --hook")),
                "{target} apply must write protocol on {event}: {applied:?}"
            );
        }
    }

    fn session_start_commands(event: &Value, target: &str) -> Vec<String> {
        if target == "cursor" {
            event
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|entry| entry["command"].as_str().map(str::to_string))
                .collect()
        } else {
            event
                .as_array()
                .and_then(|groups| groups.first())
                .and_then(|group| group["hooks"].as_array())
                .into_iter()
                .flatten()
                .filter_map(|entry| entry["command"].as_str().map(str::to_string))
                .collect()
        }
    }

    /// Claude Code and Codex get exactly the session-level set: no per-tool-call
    /// brief and no Subagent pair. Stop is per turn, not per tool call.
    #[test]
    fn only_claude_adds_the_subagent_brief_to_the_session_events() {
        for target in ["claude", "codex"] {
            let v = dry_run_json(target);
            let mut events: Vec<&str> = v["hooks"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            events.sort_unstable();
            let wanted: &[&str] = if target == "claude" {
                &[
                    "SessionEnd",
                    "SessionStart",
                    "Stop",
                    "SubagentStart",
                    "UserPromptSubmit",
                ]
            } else {
                &["SessionEnd", "SessionStart", "Stop", "UserPromptSubmit"]
            };
            assert_eq!(events, wanted, "{target} wiring");
            let encoded = v.to_string();
            assert!(
                !encoded.contains("--inline"),
                "{target} wires no inline brief"
            );
        }
    }

    fn stop_entries(target: &str) -> Vec<Value> {
        dry_run_json(target)["hooks"]["Stop"][0]["hooks"]
            .as_array()
            .unwrap()
            .clone()
    }

    /// I1 / W1 / W5: Claude Code's Stop holds `stop-check` (sync) and `watch`
    /// as an `asyncRewake` hook whose exit 2 survives the wrapper and whose
    /// timeout outlasts the watcher's own lifetime.
    #[test]
    fn claude_stop_runs_stop_check_and_a_rewake_watcher() {
        let stop = stop_entries("claude");
        assert_eq!(stop.len(), 2);
        let check = stop[0]["command"].as_str().unwrap();
        assert!(check.contains("harbard stop-check --hook claude-code"));
        assert!(
            stop[0].get("asyncRewake").is_none(),
            "stop-check must block synchronously"
        );

        let watcher = &stop[1];
        let command = watcher["command"].as_str().unwrap();
        assert!(command.contains("exec harbard watch --hook claude-code"));
        assert!(command.contains(MARKER));
        assert!(
            !command.contains("|| true"),
            "|| true would turn exit 2 into 0: {command}"
        );
        assert_eq!(watcher["asyncRewake"], json!(true));
        assert!(
            watcher.get("async").is_none(),
            "asyncRewake alone runs it in the background"
        );
        let timeout = watcher["timeout"].as_u64().unwrap();
        assert!(
            timeout > crate::watch::LIFETIME_SECS,
            "the hook timeout {timeout} must cover the watcher lifetime"
        );
    }

    /// I1 / W6: Codex has no rewake; its Stop arms a detached watcher.
    #[test]
    fn codex_stop_runs_stop_check_and_a_detached_watcher() {
        let stop = stop_entries("codex");
        assert_eq!(stop.len(), 2);
        assert!(
            stop[0]["command"]
                .as_str()
                .unwrap()
                .contains("harbard stop-check --hook codex")
        );
        assert!(
            stop[1]["command"]
                .as_str()
                .unwrap()
                .contains("harbard watch --detach --hook codex")
        );
        for entry in &stop {
            assert!(entry.get("asyncRewake").is_none());
            assert_eq!(entry["timeout"], json!(TIMEOUT_SECS));
        }
    }

    #[test]
    fn dry_run_codex_session_end_respects_the_timeout_cap() {
        let v = dry_run_json("codex");
        let hooks = v.get("hooks").unwrap().as_object().unwrap();
        assert!(
            !hooks["Stop"].to_string().contains("harbard bye"),
            "bye never rides the per-turn Stop"
        );
        // Codex caps SessionEnd at 3s; the generator must not emit 5.
        let session_end = hooks["SessionEnd"][0]["hooks"].as_array().unwrap();
        assert_eq!(session_end[0]["timeout"], 3);
        assert!(
            session_end[0]["command"]
                .as_str()
                .unwrap()
                .contains("harbard bye")
        );
    }

    /// Harbard hooks carry presence and delivery only. Admission policy belongs
    /// to the host safety layer and an install must retire managed gate entries.
    #[test]
    fn every_harness_wires_delivery_without_admission_control() {
        for target in ["claude", "codex"] {
            let v = dry_run_json(target);
            assert!(v["hooks"].get("PreToolUse").is_none());
            assert!(!v.to_string().contains("harbard gate"));
        }
        let cursor = dry_run_json("cursor");
        assert!(cursor["hooks"].get("preToolUse").is_none());
        let post = session_start_commands(&cursor["hooks"]["postToolUse"], "cursor");
        assert!(
            post.iter()
                .any(|command| command.contains("harbard brief --inline"))
        );
        assert!(post.iter().all(|command| !command.contains("harbard gate")));
    }

    #[test]
    fn managed_gate_entries_are_retired_on_reinstall() {
        for target in ["claude", "codex", "cursor"] {
            let mut existing = dry_run_json(target);
            let event = if target == "cursor" {
                "preToolUse"
            } else {
                "PreToolUse"
            };
            existing["hooks"][event] = if target == "cursor" {
                json!([{ "command": "harbard gate # harbard-managed", "timeout": 10 }])
            } else {
                json!([{ "hooks": [{ "type": "command", "command": "harbard gate # harbard-managed", "timeout": 10 }] }])
            };
            let original = serde_json::to_string_pretty(&existing).unwrap();
            let (encoded, _) = merged_config(Some(&original), target).unwrap();
            assert!(
                !encoded.contains("harbard gate"),
                "{target} retained the retired gate"
            );
        }
    }

    fn managed(command: &str) -> Value {
        json!({ "hooks": [{
            "type": "command",
            "command": format!("command -v harbard >/dev/null 2>&1 && harbard {command} || true # harbard-managed"),
            "timeout": 5
        }] })
    }

    fn foreign(command: &str) -> Value {
        json!({ "hooks": [{ "type": "command", "command": command, "timeout": 3 }] })
    }

    /// Reinstalling over the older wiring (PreToolUse inline brief plus the
    /// Subagent pair) removes harbard's entries from those events and leaves
    /// every foreign hook exactly where it was.
    #[test]
    fn reinstall_converges_older_wiring_and_keeps_foreign_hooks() {
        for (target, flag) in [("claude", "claude-code"), ("codex", "codex")] {
            let path = tmp();
            let seed = json!({
                "hooks": {
                    "SessionStart": [managed(&format!("hello --hook {flag}"))],
                    "PreToolUse": [
                        foreign("other-guard check"),
                        managed(&format!("brief --inline --throttle 20 --hook {flag}"))
                    ],
                    "SubagentStart": [
                        managed(&format!("hello --hook {flag}")),
                        foreign("other-tool subagent")
                    ],
                    "SubagentStop": [managed(&format!("bye --hook {flag}"))],
                    "PostToolUse": [foreign("other-tool post")]
                }
            });
            std::fs::write(&path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();
            let applied = apply(&path, target).unwrap();
            assert!(applied.changed);
            assert!(applied.backup.is_some(), "a changed config is backed up");

            let root: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            let hooks = root["hooks"].as_object().unwrap();
            let pre = hooks["PreToolUse"].as_array().unwrap();
            assert_eq!(pre, &vec![foreign("other-guard check")], "{target}");
            let start = hooks["SubagentStart"].as_array().unwrap();
            assert_eq!(start[0], foreign("other-tool subagent"), "{target}");
            let told = start.iter().filter(|g| is_ours(g)).collect::<Vec<_>>();
            assert_eq!(told.len(), usize::from(target == "claude"), "{target}");
            assert!(
                told.iter()
                    .all(|g| g.to_string().contains("protocol --subagent")),
                "{target}: told who reads it, never joined to the bus"
            );
            assert!(
                !hooks.contains_key("SubagentStop"),
                "{target}: an event left with no entries is dropped"
            );
            assert_eq!(
                hooks["PostToolUse"].as_array().unwrap(),
                &vec![foreign("other-tool post")],
                "{target}: untouched foreign event"
            );
            for event in ["SessionStart", "UserPromptSubmit", "Stop", "SessionEnd"] {
                let ours = hooks[event].as_array().unwrap();
                assert_eq!(ours.iter().filter(|g| is_ours(g)).count(), 1, "{event}");
            }
            assert!(!would_change(&path, target).unwrap(), "{target} converged");
        }
    }

    /// An empty event array the operator left behind is not ours to delete.
    #[test]
    fn reinstall_leaves_preexisting_empty_event_arrays_alone() {
        let path = tmp();
        std::fs::write(&path, r#"{"hooks":{"PreToolUse":[]}}"#).unwrap();
        apply(&path, "claude").unwrap();
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["hooks"]["PreToolUse"], json!([]));
    }

    #[test]
    fn dry_run_cursor_uses_cursor_surface_and_shape() {
        let v = dry_run_json("cursor");
        // Versioned schema, flat entries, Cursor's own event names — with the
        // identical verbs.
        assert_eq!(v.get("version").unwrap(), &json!(1));
        let hooks = v.get("hooks").unwrap().as_object().unwrap();
        for event in [
            "sessionStart",
            "beforeSubmitPrompt",
            "postToolUse",
            "sessionEnd",
        ] {
            assert!(hooks.contains_key(event), "cursor wiring misses {event}");
        }
        assert!(
            !hooks.contains_key("stop")
                || hooks["stop"]
                    .as_array()
                    .map(|a| a.is_empty())
                    .unwrap_or(true),
            "dry-run must not wire bye on per-turn stop"
        );
        let end = hooks["sessionEnd"].as_array().unwrap();
        assert_eq!(end.len(), 1);
        assert!(
            end[0]["command"]
                .as_str()
                .unwrap()
                .contains("harbard bye --hook cursor")
        );
        let ss = hooks["sessionStart"].as_array().unwrap();
        assert_eq!(ss.len(), 3);
        assert!(
            ss[0]["command"]
                .as_str()
                .unwrap()
                .contains("harbard hello --hook cursor")
        );
        assert!(
            ss[1]["command"]
                .as_str()
                .unwrap()
                .contains("harbard protocol --hook cursor")
        );
        assert!(
            ss[2]["command"]
                .as_str()
                .unwrap()
                .contains("harbard brief --hook cursor")
        );
        // Every entry is harbard-managed and timeout-bounded.
        for entry in hooks.values().flat_map(|e| e.as_array().unwrap().iter()) {
            let cmd = entry["command"].as_str().unwrap();
            assert!(cmd.contains(MARKER), "unmanaged command: {cmd}");
            assert!(entry["timeout"].is_number());
        }
    }

    #[test]
    fn cursor_install_retires_bye_on_stop() {
        let path = tmp();
        let seed = json!({
            "version": 1,
            "hooks": {
                "stop": [
                    { "command": "other-tool stop || true", "timeout": 3 },
                    { "command": "command -v stefna >/dev/null 2>&1 && stefna bye --hook cursor || true # stefna-managed", "timeout": 5 }
                ]
            }
        });
        std::fs::write(&path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();
        apply(&path, "cursor").unwrap();
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let stop = root["hooks"]["stop"].as_array().unwrap();
        assert!(
            stop.iter().all(|g| !is_ours(g)),
            "managed bye must leave stop: {stop:?}"
        );
        assert!(stop.iter().any(|g| !is_ours(g)), "foreign stop entry kept");
        let end = root["hooks"]["sessionEnd"].as_array().unwrap();
        assert!(
            end.iter().any(|g| {
                g["command"]
                    .as_str()
                    .is_some_and(|c| c.contains("harbard bye") && c.contains(MARKER))
            }),
            "bye must ride sessionEnd"
        );
    }

    #[test]
    fn claude_install_retires_bye_on_stop() {
        let path = tmp();
        let seed = json!({
            "hooks": {
                "Stop": [
                    {
                        "hooks": [
                            {
                                "type": "command",
                                "command": "other-tool stop || true",
                                "timeout": 3
                            }
                        ]
                    },
                    {
                        "hooks": [
                            {
                                "type": "command",
                                "command": "command -v harbard >/dev/null 2>&1 && harbard bye --hook claude-code || true # harbard-managed",
                                "timeout": 5
                            }
                        ]
                    }
                ]
            }
        });
        std::fs::write(&path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();
        apply(&path, "claude").unwrap();
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let stop = root["hooks"]["Stop"].as_array().unwrap();
        assert!(
            !root["hooks"]["Stop"].to_string().contains("harbard bye"),
            "managed bye must leave Stop: {stop:?}"
        );
        assert_eq!(
            stop.iter().filter(|g| is_ours(g)).count(),
            1,
            "Stop carries exactly one managed group (stop-check + watch)"
        );
        assert!(stop.iter().any(|g| !is_ours(g)), "foreign Stop entry kept");
        let end = root["hooks"]["SessionEnd"].as_array().unwrap();
        assert!(
            end.iter().any(|g| {
                g.get("hooks")
                    .and_then(|h| h.as_array())
                    .is_some_and(|hooks| {
                        hooks.iter().any(|hk| {
                            hk["command"]
                                .as_str()
                                .is_some_and(|c| c.contains("harbard bye") && c.contains(MARKER))
                        })
                    })
            }),
            "bye must ride SessionEnd"
        );
    }

    #[test]
    fn cursor_install_is_idempotent_and_preserves_foreign() {
        let path = tmp();
        // A foreign Cursor entry and a foreign top-level key must survive.
        let seed = json!({
            "version": 1,
            "hooks": {
                "sessionStart": [ { "command": "other-tool run || true", "timeout": 3 } ]
            }
        });
        std::fs::write(&path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        let first_apply = apply(&path, "cursor").unwrap();
        assert!(first_apply.changed);
        let first = std::fs::read_to_string(&path).unwrap();
        let second_apply = apply(&path, "cursor").unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(first, second, "second install must not change the config");
        assert!(!second_apply.changed);
        assert!(
            second_apply.backup.is_none(),
            "an unchanged install must not create backup noise"
        );

        let root: Value = serde_json::from_str(&second).unwrap();
        assert_eq!(root["version"], json!(1), "schema version preserved");
        let ss = root["hooks"]["sessionStart"].as_array().unwrap();
        assert!(ss.iter().any(|g| !is_ours(g)), "foreign entry kept");
        assert_eq!(
            ss.iter().filter(|g| is_ours(g)).count(),
            3,
            "hello + protocol + brief, exactly once"
        );
    }

    #[test]
    fn install_is_idempotent() {
        // A6: a re-run leaves the file byte-identical (no duplicate groups).
        let path = tmp();
        let first_apply = apply(&path, "claude").unwrap();
        assert!(first_apply.changed);
        let first = std::fs::read_to_string(&path).unwrap();
        let second_apply = apply(&path, "claude").unwrap();
        let second = std::fs::read_to_string(&path).unwrap();
        assert_eq!(first, second, "second install must not change the config");
        assert!(!second_apply.changed);
        assert!(
            second_apply.backup.is_none(),
            "an unchanged install must not create backup noise"
        );

        // Exactly one harbard group per event.
        let root: Value = serde_json::from_str(&second).unwrap();
        for event in ["SessionStart", "UserPromptSubmit", "Stop", "SessionEnd"] {
            let arr = root["hooks"][event].as_array().unwrap();
            let ours = arr.iter().filter(|g| is_ours(g)).count();
            assert_eq!(ours, 1, "{event} has one harbard group");
        }
    }

    #[test]
    fn preview_detects_legacy_duplicates_even_when_health_is_green() {
        let path = tmp();
        apply(&path, "codex").unwrap();
        assert!(!would_change(&path, "codex").unwrap());

        let mut root: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        root["hooks"]["SessionStart"]
            .as_array_mut()
            .unwrap()
            .push(json!({
                "hooks": [{
                    "type": "command",
                    "command": "stefna brief --hook codex || true # stefna-managed",
                    "timeout": 5
                }]
            }));
        std::fs::write(
            &path,
            format!("{}\n", serde_json::to_string_pretty(&root).unwrap()),
        )
        .unwrap();

        assert!(
            would_change(&path, "codex").unwrap(),
            "dry-run must predict legacy cleanup instead of claiming unchanged"
        );
    }

    #[test]
    fn install_preserves_foreign_key_order() {
        // Managed hook changes must not alphabetize unrelated operator config.
        let input = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/claude-settings.input.json"
        );
        let path = tmp();
        std::fs::copy(input, &path).unwrap();
        apply(&path, "claude").unwrap();

        let got = std::fs::read_to_string(&path).unwrap();

        // Regression guard: without preserve_order these keys alphabetize
        // (hooks < model < permissions) instead of keeping file order.
        let model = got.find("\"model\"").unwrap();
        let hooks = got.find("\"hooks\"").unwrap();
        let permissions = got.find("\"permissions\"").unwrap();
        assert!(
            model < hooks && hooks < permissions,
            "top-level key order preserved, not alphabetized"
        );
    }

    #[test]
    fn install_preserves_foreign_groups_and_backs_up() {
        let path = tmp();
        // Pre-seed a foreign hook group and a foreign top-level key.
        let seed = json!({
            "model": "opus",
            "hooks": {
                "SessionStart": [
                    { "hooks": [ { "type": "command", "command": "other-tool run || true", "timeout": 3 } ] }
                ]
            }
        });
        std::fs::write(&path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();

        let applied = apply(&path, "codex").unwrap();
        assert!(applied.backup.is_some(), "a backup was written");

        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["model"], "opus", "foreign top-level key preserved");
        let ss = root["hooks"]["SessionStart"].as_array().unwrap();
        // Foreign group + our group.
        assert_eq!(ss.len(), 2);
        assert!(ss.iter().any(|g| !is_ours(g)), "foreign group kept");
        assert!(ss.iter().any(is_ours), "our group added");
    }

    #[test]
    fn health_reports_missing_and_wired() {
        let dir = {
            let n = SEQ.fetch_add(1, Ordering::Relaxed);
            let d =
                std::env::temp_dir().join(format!("harbard-health-{}-{}", std::process::id(), n));
            let _ = std::fs::remove_dir_all(&d);
            std::fs::create_dir_all(&d).unwrap();
            d
        };
        let path = dir.join("settings.json");
        // Point HOME at the temp dir so default_config_path resolves here.
        // SAFETY: test-only, single-threaded for this env mutation.
        let prev = std::env::var_os("HOME");
        // claude → ~/.claude/settings.json
        std::fs::create_dir_all(dir.join(".claude")).unwrap();
        let claude_path = dir.join(".claude").join("settings.json");
        unsafe { std::env::set_var("HOME", &dir) };

        let missing = health_one("claude");
        assert_eq!(missing["config_exists"], false);
        assert_eq!(missing["ok"], false);

        apply(&claude_path, "claude").unwrap();
        let wired = health_one("claude");
        assert_eq!(wired["config_exists"], true);
        assert_eq!(wired["ok"], true);
        assert!(wired["missing_events"].as_array().unwrap().is_empty());

        // The operator's hand-trimmed config: exactly the session-level set
        // plus foreign hooks on other events (including PreToolUse) is healthy.
        let trimmed = json!({
            "hooks": {
                "SessionStart": [managed("hello --hook claude-code")],
                "UserPromptSubmit": [managed("brief --touch --hook claude-code")],
                "Stop": [dry_run_json("claude")["hooks"]["Stop"][0].clone()],
                "SessionEnd": [managed("bye --hook claude-code")],
                "SubagentStart": [managed("protocol --subagent --hook claude-code")],
                "PreToolUse": [foreign("other-guard check")]
            }
        });
        std::fs::write(&claude_path, trimmed.to_string()).unwrap();
        let trimmed_health = health_one("claude");
        assert_eq!(trimmed_health["ok"], true, "{trimmed_health}");

        // A managed entry left on an unwired event is reported, not ignored.
        let mut lingering = trimmed.clone();
        lingering["hooks"]["SubagentStop"] = json!([managed("bye --hook claude-code")]);
        std::fs::write(&claude_path, lingering.to_string()).unwrap();
        let stale = health_one("claude");
        assert_eq!(stale["ok"], false);
        assert_eq!(stale["retired_lingering"], json!(["SubagentStop"]));
        apply(&claude_path, "claude").unwrap();
        assert_eq!(health_one("claude")["ok"], true, "install converges");

        match prev {
            Some(v) => unsafe { std::env::set_var("HOME", v) },
            None => unsafe { std::env::remove_var("HOME") },
        }
        let _ = path; // silence
    }

    fn stop_health(stop: Value) -> Value {
        let path = tmp();
        let mut root = dry_run_json("claude");
        root["hooks"]["Stop"] = stop;
        std::fs::write(&path, root.to_string()).unwrap();
        health_at(&path, "claude")
    }

    /// I1: health accepts exactly the wired Stop set. Half of it, a leftover
    /// managed command, or a watcher that lost `asyncRewake` all read as
    /// miswired, because each of them wakes nobody.
    #[test]
    fn health_accepts_exactly_the_stop_set() {
        let full = dry_run_json("claude")["hooks"]["Stop"].clone();
        assert_eq!(stop_health(full.clone())["ok"], true);

        let mut half = full.clone();
        half[0]["hooks"].as_array_mut().unwrap().pop();
        let report = stop_health(half);
        assert_eq!(report["ok"], false);
        assert!(
            report["missing_commands"][0]
                .as_str()
                .unwrap()
                .contains("harbard watch")
        );

        let mut leftover = full.clone();
        leftover
            .as_array_mut()
            .unwrap()
            .push(managed("bye --hook claude-code"));
        let report = stop_health(leftover);
        assert_eq!(report["ok"], false);
        assert!(
            report["unexpected_commands"][0]
                .as_str()
                .unwrap()
                .contains("harbard bye")
        );

        let mut no_rewake = full;
        no_rewake[0]["hooks"][1]
            .as_object_mut()
            .unwrap()
            .remove("asyncRewake");
        assert_eq!(stop_health(no_rewake)["ok"], false);
    }

    /// I1: uninstall removes every managed entry from every event, keeps the
    /// foreign ones and the rest of the file, and is a no-op the second time.
    #[test]
    fn uninstall_removes_every_managed_hook_and_keeps_foreign() {
        for target in ["claude", "codex", "cursor"] {
            let path = tmp();
            let seed = if target == "cursor" {
                json!({ "version": 1, "hooks": { "stop": [{ "command": "other-tool stop", "timeout": 3 }] } })
            } else {
                json!({
                    "model": "opus",
                    "hooks": {
                        "Stop": [foreign("other-tool stop")],
                        "PreToolUse": [managed("brief --inline --hook x")]
                    }
                })
            };
            std::fs::write(&path, serde_json::to_string_pretty(&seed).unwrap()).unwrap();
            apply(&path, target).unwrap();

            let removed = remove(&path, target).unwrap();
            assert!(removed.changed, "{target}");
            assert!(
                removed.backup.is_some(),
                "{target}: uninstall backs up first"
            );
            let text = std::fs::read_to_string(&path).unwrap();
            assert!(!text.contains(MARKER), "{target}: {text}");
            assert!(
                text.contains("other-tool stop"),
                "{target}: foreign hook kept"
            );
            let root: Value = serde_json::from_str(&text).unwrap();
            if target != "cursor" {
                assert_eq!(root["model"], "opus");
                assert!(
                    root["hooks"].get("PreToolUse").is_none(),
                    "emptied event dropped"
                );
            }
            assert!(
                health_at(&path, target)["ok"] == false,
                "{target}: health reports the removal"
            );

            let again = remove(&path, target).unwrap();
            assert!(
                !again.changed && again.backup.is_none(),
                "{target}: idempotent"
            );
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        }
    }

    /// W6: the daemon probe is a connect, as Codex's own is — a socket that
    /// answers is running, a missing one is absent.
    #[cfg(unix)]
    #[test]
    fn codex_daemon_probe_connects_to_the_control_socket() {
        // Short on purpose: a Unix socket path is capped near 104 bytes on
        // macOS, and the system temp dir alone spends half of that.
        let n = SEQ.fetch_add(1, Ordering::Relaxed);
        let home = PathBuf::from("/tmp").join(format!("hb{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        assert!(!codex_daemon_running(&home));
        let absent = codex_wake_health(&home);
        assert_eq!(absent["daemon"], "absent");
        assert!(absent["note"].as_str().is_some());

        let dir = home.join("app-server-control");
        std::fs::create_dir_all(&dir).unwrap();
        let _listener =
            std::os::unix::net::UnixListener::bind(dir.join("app-server-control.sock")).unwrap();
        assert!(codex_daemon_running(&home));
        assert_eq!(codex_wake_health(&home)["daemon"], "running");
        let _ = std::fs::remove_dir_all(&home);
    }
    #[test]
    fn a_short_rewake_timeout_is_unhealthy_and_install_repairs_it() {
        let path = super::tests::tmp();
        let mut config = dry_run_json("claude");
        let watcher = config["hooks"]["Stop"][0]["hooks"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|entry| entry["asyncRewake"] == true)
            .unwrap();
        watcher["timeout"] = json!(4 * 60 * 60 + 60);
        std::fs::write(&path, config.to_string()).unwrap();
        let short = health_at(&path, "claude");
        assert_eq!(short["ok"], false);
        assert_eq!(short["wake_timeout_short"], true);
        assert_eq!(short["hint"], "harbard install claude");
        apply(&path, "claude").unwrap();
        assert_eq!(health_at(&path, "claude")["ok"], true);
    }
}
