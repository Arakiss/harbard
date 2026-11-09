//! Canonical machine interface for future harnesses.
//!
//! Finite operations emit exactly one `harbard/cli/v1` JSON envelope. A bus
//! subscription emits one `harbard/stream/v1` frame per line. MCP consumes the
//! same operation dispatcher and therefore cannot grow an exclusive verb.

use crate::cli::Common;
use crate::context::Context;
use harbard_core::protocol::{MACHINE_OPERATIONS, ProtocolDepth, catalog};
use harbard_core::{Event, EventKind, Store, Window};
use serde_json::{Value, json};
use std::collections::HashSet;
use std::io::Write;
use std::process::Command;
use std::thread;
use std::time::{Duration, Instant};

pub const CLI_WIRE_VERSION: &str = "harbard/cli/v1";
pub const STREAM_WIRE_VERSION: &str = "harbard/stream/v1";
const EVENT_KIND_IDS: &[&str] = &[
    "hello",
    "intent",
    "note",
    "ack",
    "dependency",
    "bye",
    "join",
    "leave",
    "device_join",
    "device_leave",
    "feedback",
    "rule",
    "unknown",
];

pub fn cmd_call(common: &Common, operation: &str, input: &str) -> i32 {
    let args: Value = match serde_json::from_str(input) {
        Ok(Value::Object(map)) => Value::Object(map),
        Ok(_) => {
            return print_call_error(
                operation,
                "invalid_input",
                "--input must be a JSON object",
                None,
                2,
            );
        }
        Err(error) => {
            return print_call_error(
                operation,
                "invalid_json",
                &format!("could not parse --input: {error}"),
                None,
                2,
            );
        }
    };

    match invoke_operation(common, operation, &args) {
        Ok((text, false)) => {
            let data = serde_json::from_str(&text).unwrap_or(Value::String(text));
            println!(
                "{}",
                serde_json::to_string(&json!({
                    "v": CLI_WIRE_VERSION,
                    "ok": true,
                    "operation": operation,
                    "data": data,
                }))
                .unwrap_or_else(|_| "{}".into())
            );
            0
        }
        Ok((text, true)) => {
            let details = serde_json::from_str::<Value>(&text).unwrap_or(Value::String(text));
            let status = match (operation, details.get("exit_code").and_then(Value::as_i64)) {
                ("dependency.await", Some(code @ (3 | 4))) => code as i32,
                _ => 1,
            };
            print_call_error(
                operation,
                "operation_failed",
                "the CLI operation returned an error",
                Some(details),
                status,
            )
        }
        Err(error) => print_call_error(operation, "invalid_operation", &error, None, 2),
    }
}

/// Operations available through the canonical CLI machine interface.
/// MCP derives its tool surface from this closed set and only adapts envelopes.
pub(crate) const OPERATION_IDS: &[&str] = MACHINE_OPERATIONS;

pub(crate) fn invoke_operation(
    common: &Common,
    operation: &str,
    args: &Value,
) -> Result<(String, bool), String> {
    if !OPERATION_IDS.contains(&operation) {
        return Err(format!("unknown operation: {operation}"));
    }
    dispatch_operation(common, operation, args).map_err(|error| {
        error
            .get("message")
            .and_then(Value::as_str)
            .unwrap_or("operation dispatch failed")
            .to_string()
    })
}

fn dispatch_operation(
    common: &Common,
    operation: &str,
    args: &Value,
) -> Result<(String, bool), Value> {
    match operation {
        "protocol" => {
            let depth = if args.get("full").and_then(Value::as_bool).unwrap_or(false) {
                ProtocolDepth::Full
            } else {
                ProtocolDepth::Compact
            };
            Ok((
                serde_json::to_string_pretty(&catalog(depth)).unwrap_or_else(|_| "{}".into()),
                false,
            ))
        }
        "hello" => match args.get("intent").and_then(Value::as_str) {
            Some(intent) => run_verb(common, &["hello", "--intent", intent]),
            None => run_verb(common, &["hello"]),
        },
        "name" => {
            let mut argv = vec!["name".to_string(), require_str(args, "name")?.to_string()];
            if let Some(intro) = args.get("intro").and_then(Value::as_str) {
                argv.extend(["--intro".into(), intro.into()]);
            }
            run_owned(common, &argv)
        }
        "intro" => run_verb(common, &["intro", require_str(args, "text")?]),
        "feedback" => {
            let mut argv = vec![
                "feedback".to_string(),
                require_str(args, "text")?.to_string(),
                "--kind".into(),
                require_str(args, "kind")?.to_string(),
            ];
            if let Some(verb) = args.get("verb").and_then(Value::as_str) {
                argv.extend(["--verb".into(), verb.into()]);
            }
            run_owned(common, &argv)
        }
        "feedback.list" => {
            let mut argv = vec!["feedback".to_string(), "list".to_string()];
            if let Some(kind) = args.get("kind").and_then(Value::as_str) {
                argv.extend(["--kind".into(), kind.into()]);
            }
            if args.get("limit").is_some() {
                let limit = bounded_u64(args, "limit", 1, 500, 50)?;
                argv.extend(["--limit".into(), limit.to_string()]);
            }
            run_owned(common, &argv)
        }
        "channel.read" => {
            let mut argv = vec![
                "channel".to_string(),
                "read".to_string(),
                require_str(args, "name")?.to_string(),
            ];
            if args.get("limit").is_some() {
                let limit = bounded_u64(args, "limit", 1, 200, 30)?;
                argv.extend(["--limit".into(), limit.to_string()]);
            }
            if let Some(before) = args.get("before").and_then(Value::as_str) {
                argv.extend(["--before".into(), before.into()]);
            }
            run_owned(common, &argv)
        }
        "channel.post" => {
            let mut argv = vec![
                "channel".to_string(),
                "post".to_string(),
                require_str(args, "name")?.to_string(),
                require_str(args, "text")?.to_string(),
            ];
            for (key, flag) in [("kind", "--kind"), ("ref", "--ref")] {
                if let Some(value) = args.get(key).and_then(Value::as_str) {
                    argv.extend([flag.into(), value.into()]);
                }
            }
            run_owned(common, &argv)
        }
        "intent" => run_verb(common, &["intent", require_str(args, "text")?]),
        "note" => {
            let mut argv = vec![
                "note".to_string(),
                require_str(args, "text")?.to_string(),
                "--kind".into(),
                args.get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("fyi")
                    .into(),
            ];
            for (key, flag) in [
                ("to", "--to"),
                ("ref", "--ref"),
                ("body_file", "--body-file"),
            ] {
                if let Some(value) = args.get(key).and_then(Value::as_str) {
                    argv.extend([flag.into(), value.into()]);
                }
            }
            for path in args
                .get("attach")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                argv.extend(["--attach".into(), path.into()]);
            }
            run_owned(common, &argv)
        }
        "ack" => {
            let mut argv = vec!["ack".to_string(), require_str(args, "target")?.to_string()];
            if let Some(note) = args.get("note").and_then(Value::as_str) {
                argv.extend(["--note".into(), note.into()]);
            }
            run_owned(common, &argv)
        }
        "reply" => {
            let mut argv = vec![
                "reply".to_string(),
                require_str(args, "target")?.to_string(),
                require_str(args, "text")?.to_string(),
            ];
            for (key, flag) in [("kind", "--kind"), ("agent", "--agent")] {
                if let Some(value) = args.get(key).and_then(Value::as_str) {
                    argv.extend([flag.into(), value.into()]);
                }
            }
            run_owned(common, &argv)
        }
        "brief" => {
            let mut argv = vec!["brief".to_string()];
            if args.get("touch").and_then(Value::as_bool).unwrap_or(false) {
                argv.push("--touch".into());
            }
            run_owned(common, &argv)
        }
        "bye" => run_verb(common, &["bye"]),
        "claim" => {
            let ttl = args
                .get("ttl_secs")
                .and_then(Value::as_u64)
                .map(|v| v.to_string());
            match ttl.as_deref() {
                Some(ttl) => run_verb(common, &["claim", "--writer", "--ttl-secs", ttl]),
                None => run_verb(common, &["claim", "--writer"]),
            }
        }
        "fleet" => run_verb(common, &["fleet"]),
        "agents" => {
            let mut argv = vec!["agents".to_string()];
            if args.get("all").and_then(Value::as_bool).unwrap_or(false) {
                argv.push("--all".into());
            }
            run_owned(common, &argv)
        }
        "clean" => {
            let mut argv = vec!["clean".to_string(), "--finished".to_string()];
            if args.get("yes").and_then(Value::as_bool).unwrap_or(false) {
                argv.push("--yes".into());
            }
            run_owned(common, &argv)
        }
        "relay" => {
            let mut argv = vec!["relay".to_string()];
            if let Some(id) = args.get("sent").and_then(Value::as_str) {
                argv.extend(["--sent".into(), id.into()]);
            }
            if let Some(limit) = args.get("limit").and_then(Value::as_u64) {
                argv.extend(["--limit".into(), limit.to_string()]);
            }
            run_owned(common, &argv)
        }
        "owed" => {
            let mut argv = vec!["owed".to_string()];
            if let Some(limit) = args.get("limit").and_then(Value::as_u64) {
                argv.extend(["--limit".into(), limit.to_string()]);
            }
            run_owned(common, &argv)
        }
        "ls" => {
            let mut argv = vec!["ls".to_string()];
            if args.get("all").and_then(Value::as_bool).unwrap_or(false) {
                argv.push("--all".into());
            }
            run_owned(common, &argv)
        }
        "inbox" => run_verb(common, &["inbox"]),
        operation if operation.starts_with("dependency.") => {
            run_owned(common, &dependency_args(operation, args)?)
        }
        "join" | "leave" => {
            let mut argv = vec![
                operation.to_string(),
                require_str(args, "channel")?.to_string(),
            ];
            if let Some(anchor) = args.get("anchor").and_then(Value::as_str) {
                argv.extend(["--as".into(), anchor.into()]);
            }
            run_owned(common, &argv)
        }
        "channels" => {
            let mut argv = vec!["channels".to_string()];
            if args.get("all").and_then(Value::as_bool).unwrap_or(false) {
                argv.push("--all".into());
            }
            run_owned(common, &argv)
        }
        "search" => {
            let mut argv = vec![
                "search".to_string(),
                require_str(args, "query")?.to_string(),
            ];
            if let Some(limit) = args.get("limit").and_then(Value::as_u64) {
                argv.extend(["--limit".into(), limit.to_string()]);
            }
            run_owned(common, &argv)
        }
        "doctor" => run_verb(common, &["doctor"]),
        "trace" => {
            let mut argv = vec!["trace".to_string()];
            for (key, flag) in [("note", "--note"), ("session", "--session")] {
                if let Some(value) = args.get(key).and_then(Value::as_str) {
                    argv.extend([flag.into(), value.into()]);
                }
            }
            run_owned(common, &argv)
        }
        "audit" => {
            let mut argv = vec!["audit".to_string()];
            if let Some(target) = args.get("target").and_then(Value::as_str) {
                argv.extend(["--target".into(), target.into()]);
            }
            if args
                .get("include_worktrees")
                .and_then(Value::as_bool)
                .unwrap_or(false)
            {
                argv.push("--include-worktrees".into());
            }
            run_owned(common, &argv)
        }
        _ => unreachable!("operation checked against OPERATION_IDS"),
    }
}

fn dependency_args(operation: &str, args: &Value) -> Result<Vec<String>, Value> {
    let mut argv = match operation {
        "dependency.await" => vec!["await".into(), require_str(args, "id")?.into()],
        _ => vec![
            "dependency".into(),
            operation.trim_start_matches("dependency.").into(),
        ],
    };
    match operation {
        "dependency.open" => {
            for (key, flag) in [
                ("to", "--to"),
                ("condition", "--condition"),
                ("resume", "--resume"),
            ] {
                argv.extend([flag.into(), require_str(args, key)?.into()]);
            }
            if args.get("deadline_secs").is_some() {
                let deadline = bounded_u64(args, "deadline_secs", 1, 604_800, 1)?;
                argv.extend(["--deadline-secs".into(), deadline.to_string()]);
            }
        }
        "dependency.resolve" => {
            let outcome = require_str(args, "outcome")?;
            if !matches!(outcome, "satisfied" | "failed") {
                return Err(
                    json!({"code": -32602, "message": "outcome must be satisfied or failed"}),
                );
            }
            argv.extend([
                require_str(args, "id")?.into(),
                "--outcome".into(),
                outcome.into(),
                "--evidence".into(),
                require_str(args, "evidence")?.into(),
            ]);
        }
        "dependency.cancel" => argv.extend([
            require_str(args, "id")?.into(),
            "--reason".into(),
            require_str(args, "reason")?.into(),
        ]),
        "dependency.consume" | "dependency.show" => argv.push(require_str(args, "id")?.into()),
        "dependency.list" => match args.get("all") {
            Some(Value::Bool(true)) => argv.push("--all".into()),
            None | Some(Value::Bool(false)) => {}
            _ => return Err(json!({"code": -32602, "message": "all must be a boolean"})),
        },
        "dependency.await" => {
            let timeout = bounded_u64(args, "timeout_secs", 0, 86_400, 300)?;
            let poll = bounded_u64(args, "poll_ms", 10, 60_000, 250)?;
            argv.extend([
                "--timeout-secs".into(),
                timeout.to_string(),
                "--poll-ms".into(),
                poll.to_string(),
            ]);
        }
        _ => return Err(json!({"code": -32602, "message": "unknown dependency operation"})),
    }
    Ok(argv)
}

fn bounded_u64(args: &Value, key: &str, min: u64, max: u64, default: u64) -> Result<u64, Value> {
    match args.get(key) {
        None => Ok(default),
        Some(value) => value.as_u64().filter(|value| (min..=max).contains(value)).ok_or_else(||
            json!({"code": -32602, "message": format!("{key} must be an integer between {min} and {max}")})),
    }
}

fn run_owned(common: &Common, args: &[String]) -> Result<(String, bool), Value> {
    let refs = args.iter().map(String::as_str).collect::<Vec<_>>();
    run_verb(common, &refs)
}

fn require_str<'a>(args: &'a Value, key: &str) -> Result<&'a str, Value> {
    args.get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(
            || json!({ "code": -32602, "message": format!("missing string argument: {key}") }),
        )
}

fn run_verb(common: &Common, args: &[&str]) -> Result<(String, bool), Value> {
    let exe = std::env::current_exe()
        .map_err(|error| json!({ "code": -32000, "message": error.to_string() }))?;
    let mut command = Command::new(exe);
    command.arg("--json");
    for (flag, value) in [
        ("--home", common.home.as_deref()),
        ("--session", common.session.as_deref()),
        ("--harness", common.harness.as_deref()),
        ("--cwd", common.cwd.as_deref()),
        ("--repo", common.repo.as_deref()),
    ] {
        if let Some(value) = value {
            command.args([flag, value]);
        }
    }
    command.args(args);
    let output = command
        .output()
        .map_err(|error| json!({ "code": -32000, "message": format!("spawn harbard: {error}") }))?;
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Ok(normalize_output(
        output.status.code().unwrap_or(1),
        &stdout,
        &stderr,
    ))
}

fn normalize_output(exit_code: i32, stdout: &str, stderr: &str) -> (String, bool) {
    if exit_code != 0 {
        let message = if stderr.is_empty() {
            format!("harbard exited {exit_code}")
        } else {
            stderr.to_string()
        };
        return (
            json!({ "ok": false, "error": message, "stdout": stdout,
                "exit_code": exit_code,
                "data": serde_json::from_str::<Value>(stdout).unwrap_or(Value::String(stdout.into()))
            }).to_string(),
            true,
        );
    }
    (
        if stdout.is_empty() {
            if stderr.is_empty() {
                json!({ "ok": true }).to_string()
            } else {
                stderr.to_string()
            }
        } else {
            stdout.to_string()
        },
        false,
    )
}

fn print_call_error(
    operation: &str,
    code: &str,
    message: &str,
    details: Option<Value>,
    status: i32,
) -> i32 {
    println!(
        "{}",
        serde_json::to_string(&json!({
            "v": CLI_WIRE_VERSION,
            "ok": false,
            "operation": operation,
            "data": details.as_ref().and_then(|value| value.get("data")).cloned(),
            "error": {
                "code": code,
                "message": message,
                "details": details,
            },
        }))
        .unwrap_or_else(|_| "{}".into())
    );
    status
}

#[allow(clippy::too_many_arguments)]
pub fn cmd_subscribe(
    store: &Store,
    common: &Common,
    after: Option<&str>,
    once: bool,
    poll_ms: u64,
    heartbeat_ms: u64,
    event_filters: &[String],
    addressed: bool,
) -> i32 {
    if let Some(invalid) = event_filters.iter().find(|filter| {
        !EVENT_KIND_IDS
            .iter()
            .any(|kind| filter.eq_ignore_ascii_case(kind))
    }) {
        let _ = emit_stream(json!({
            "v": STREAM_WIRE_VERSION,
            "type": "error",
            "cursor": after,
            "error": { "code": "invalid_filter", "message": format!("unknown event kind: {invalid}") },
        }));
        return 2;
    }
    let ctx = Context::resolve(common);
    // Read once: a move takes effect at the next subscription.
    let placement = harbard_core::workspaces::Placement::load(store);
    let gate = placement.gate(ctx.repo_key.as_deref().or(ctx.repo.as_deref()));
    // Capture the append position before the snapshot. Events racing into the
    // snapshot after this position are de-duplicated on the first tail read;
    // capturing it afterwards could skip them entirely.
    let (_, mut position) = store.read_since(None);
    let initial = store.read(if after.is_some() {
        Window::All
    } else {
        Window::Recent
    });
    let (tail, found) = events_after(&initial.events, after);
    let initial_ids = initial
        .events
        .iter()
        .map(|event| event.id.clone())
        .collect::<HashSet<_>>();
    if after.is_some() && !found {
        let _ = emit_stream(json!({
            "v": STREAM_WIRE_VERSION,
            "type": "error",
            "cursor": after,
            "error": { "code": "cursor_expired", "message": "resume event is not in retained bus history" },
        }));
        return 2;
    }

    let mut cursor = after.map(str::to_string);
    if !emit_stream(json!({
        "v": STREAM_WIRE_VERSION,
        "type": "ready",
        "cursor": cursor,
        "mode": if once { "snapshot" } else { "follow" },
    })) {
        return 0;
    }
    for event in tail {
        if matches_event(event, event_filters, addressed, &ctx, &gate) {
            cursor = Some(event.id.clone());
            if !emit_event(event, cursor.as_deref()) {
                return 0;
            }
        } else {
            cursor = Some(event.id.clone());
        }
    }

    if once {
        let _ = emit_stream(json!({ "v": STREAM_WIRE_VERSION, "type": "end", "cursor": cursor }));
        return 0;
    }

    let poll = Duration::from_millis(poll_ms.max(10));
    let heartbeat = Duration::from_millis(heartbeat_ms);
    let mut last_output = Instant::now();
    let mut first_tail = true;
    loop {
        thread::sleep(poll);
        let (read, next) = store.read_since(position.as_ref());
        position = next;
        for event in &read.events {
            if first_tail && initial_ids.contains(&event.id) {
                continue;
            }
            cursor = Some(event.id.clone());
            if matches_event(event, event_filters, addressed, &ctx, &gate) {
                if !emit_event(event, cursor.as_deref()) {
                    return 0;
                }
                last_output = Instant::now();
            }
        }
        first_tail = false;
        if heartbeat_ms > 0 && last_output.elapsed() >= heartbeat {
            if !emit_stream(
                json!({ "v": STREAM_WIRE_VERSION, "type": "heartbeat", "cursor": cursor }),
            ) {
                return 0;
            }
            last_output = Instant::now();
        }
    }
}

fn events_after<'a>(events: &'a [Event], after: Option<&str>) -> (&'a [Event], bool) {
    let Some(after) = after else {
        return (events, true);
    };
    match events.iter().position(|event| event.id == after) {
        Some(index) => (&events[index + 1..], true),
        None => (&[], false),
    }
}

fn matches_event(
    event: &Event,
    filters: &[String],
    addressed: bool,
    ctx: &Context,
    workspace: &harbard_core::workspaces::Gate<'_>,
) -> bool {
    let kind = event_kind_name(event.event);
    (filters.is_empty()
        || filters
            .iter()
            .any(|wanted| wanted.eq_ignore_ascii_case(kind)))
        && (!addressed
            || event.is_addressed_to(
                &ctx.session,
                &ctx.harness,
                ctx.repo.as_deref(),
                ctx.repo_key.as_deref(),
                workspace,
            ))
}

fn event_kind_name(kind: EventKind) -> &'static str {
    match kind {
        EventKind::Hello => "hello",
        EventKind::Intent => "intent",
        EventKind::Note => "note",
        EventKind::Ack => "ack",
        EventKind::Dependency => "dependency",
        EventKind::Bye => "bye",
        EventKind::Join => "join",
        EventKind::Leave => "leave",
        EventKind::DeviceJoin => "device_join",
        EventKind::DeviceLeave => "device_leave",
        EventKind::Feedback => "feedback",
        EventKind::Rule => "rule",
        EventKind::ChannelLifecycle => "channel_lifecycle",
        EventKind::Unknown => "unknown",
    }
}

fn emit_event(event: &Event, cursor: Option<&str>) -> bool {
    emit_stream(json!({
        "v": STREAM_WIRE_VERSION,
        "type": "event",
        "cursor": cursor,
        "event": event,
    }))
}

fn emit_stream(frame: Value) -> bool {
    let line = serde_json::to_string(&frame).unwrap_or_else(|_| "{}".into());
    let stdout = std::io::stdout();
    let mut out = stdout.lock();
    writeln!(out, "{line}").and_then(|_| out.flush()).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use harbard_core::{NoteKind, Role, SCHEMA_VERSION};

    fn event(id: &str, kind: EventKind) -> Event {
        Event {
            v: SCHEMA_VERSION,
            id: id.into(),
            ts: "2026-08-01T00:00:00Z".into(),
            event: kind,
            session: "s".into(),
            harness: "codex".into(),
            host: "h".into(),
            cwd: "/r".into(),
            repo: Some("/r".into()),
            repo_key: None,
            branch: Some("main".into()),
            role: Role::Main,
            parent: None,
            text: String::new(),
            to: None,
            kind: NoteKind::Fyi,
            note_ref: None,
        }
    }

    #[test]
    fn resume_is_strictly_after_cursor() {
        let events = vec![
            event("01A", EventKind::Hello),
            event("01B", EventKind::Note),
        ];
        let (tail, found) = events_after(&events, Some("01A"));
        assert!(found);
        assert_eq!(tail.len(), 1);
        assert_eq!(tail[0].id, "01B");
    }

    #[test]
    fn unknown_cursor_is_expired() {
        let events = vec![event("01A", EventKind::Hello)];
        let (tail, found) = events_after(&events, Some("missing"));
        assert!(!found);
        assert!(tail.is_empty());
    }

    #[test]
    fn dependency_open_keeps_recorded_text_as_literal_arguments() {
        let argv = dependency_args("dependency.open", &json!({
            "to": "session:responder", "condition": "The checks passed", "resume": "inspect $(do-not-run) ; continue", "deadline_secs": 60
        })).unwrap();
        assert_eq!(
            argv,
            [
                "dependency",
                "open",
                "--to",
                "session:responder",
                "--condition",
                "The checks passed",
                "--resume",
                "inspect $(do-not-run) ; continue",
                "--deadline-secs",
                "60"
            ]
        );
    }

    #[test]
    fn dependency_operations_map_to_the_same_cli_verbs() {
        for (operation, input, expected) in [
            (
                "dependency.resolve",
                json!({"id": "dep", "outcome": "satisfied", "evidence": "Verified"}),
                vec![
                    "dependency",
                    "resolve",
                    "dep",
                    "--outcome",
                    "satisfied",
                    "--evidence",
                    "Verified",
                ],
            ),
            (
                "dependency.cancel",
                json!({"id": "dep", "reason": "No longer needed"}),
                vec![
                    "dependency",
                    "cancel",
                    "dep",
                    "--reason",
                    "No longer needed",
                ],
            ),
            (
                "dependency.show",
                json!({"id": "dep"}),
                vec!["dependency", "show", "dep"],
            ),
            (
                "dependency.consume",
                json!({"id": "dep"}),
                vec!["dependency", "consume", "dep"],
            ),
            (
                "dependency.list",
                json!({"all": true}),
                vec!["dependency", "list", "--all"],
            ),
            (
                "dependency.await",
                json!({"id": "dep"}),
                vec!["await", "dep", "--timeout-secs", "300", "--poll-ms", "250"],
            ),
        ] {
            assert_eq!(dependency_args(operation, &input).unwrap(), expected);
        }
        assert!(
            dependency_args(
                "dependency.resolve",
                &json!({"id": "dep", "outcome": "acknowledged", "evidence": "Seen"})
            )
            .is_err()
        );
        assert!(
            dependency_args(
                "dependency.open",
                &json!({"to": "session:responder", "condition": "Checks"})
            )
            .is_err()
        );
    }

    #[test]
    fn dependency_wait_rejects_invalid_or_unbounded_arguments() {
        for (key, value) in [
            ("timeout_secs", json!(-1)),
            ("timeout_secs", json!(86_401)),
            ("timeout_secs", json!("300")),
            ("timeout_secs", Value::Null),
            ("poll_ms", json!(9)),
            ("poll_ms", json!(60_001)),
            ("poll_ms", json!(1.5)),
        ] {
            let mut args = json!({"id": "dep"});
            args[key] = value;
            assert!(
                dependency_args("dependency.await", &args).is_err(),
                "invalid {key}"
            );
        }
        let argv = dependency_args(
            "dependency.await",
            &json!({"id": "dep", "timeout_secs": 0, "poll_ms": 10}),
        )
        .unwrap();
        assert_eq!(
            argv,
            ["await", "dep", "--timeout-secs", "0", "--poll-ms", "10"]
        );
        assert!(dependency_args("dependency.list", &json!({"all": "true"})).is_err());
    }

    #[test]
    fn timeout_and_failed_resolution_keep_structured_evidence() {
        for (code, outcome) in [(3, "timeout"), (4, "failed"), (4, "cancelled")] {
            let data = json!({"outcome": outcome, "dependency": {"id": "dep", "continuation": "Resume review", "evidence": "Recorded evidence"}});
            let (text, is_error) = normalize_output(code, &data.to_string(), "");
            assert!(is_error);
            let result: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(result["exit_code"], code);
            assert_eq!(result["data"], data);
        }
    }

    #[test]
    fn dependency_events_are_available_to_filtered_subscribers() {
        assert!(EVENT_KIND_IDS.contains(&"dependency"));
        assert_eq!(event_kind_name(EventKind::Dependency), "dependency");
    }
}
