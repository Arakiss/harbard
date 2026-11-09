//! Writer-lease gate for harness PreToolUse hooks.
//!
//! Reads the hook JSON (already cached from stdin), classifies the pending
//! tool call, and either allows it or denies it before any write lands.
//!
//! Policy:
//! - **Read** — no lease; always allow.
//! - **Write** — must hold the writer lease; foreign holder → deny.
//! - **Ambiguous** — always allow, **never acquire**. Taking the lease on
//!   Task/opaque scripts made the first gatherer the exclusive writer and
//!   bricked every other session. Real mutators must classify as Write
//!   (compounds, `bash -c`, substitutions, absolute paths included).
//! - **Infra failure** on the Write path → allow with warning so a broken
//!   install cannot freeze agents. Cursor always emits
//!   `{"permission":"allow"|"deny"}` so `failClosed` never treats empty allow
//!   as a hard failure.

use crate::cli::Common;
use crate::context::{self, Context};
use harbard_core::{
    DEFAULT_LEASE_TTL_SECS, LeaseEnsure, LeaseError, LeaseMetadata, LeaseResource, LeaseStore,
    Store, SystemLeaseClock, SystemPidChecker, WriteClass, classify_tool,
    protocol::{
        correction_lease_held, correction_lease_unprotected, correction_write_scope_conflict,
    },
};
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// Run the gate. Exit codes:
/// - `0` — allow or deny-via-JSON (harness hook path; stdout carries the decision)
/// - `2` — deny under `--json` (machine contract for tests)
/// - `1` — unused on the hook path (infra fails open instead)
pub fn cmd_gate(store: &Store, common: &Common, pid_override: Option<u32>) -> i32 {
    let ctx = Context::resolve(common);
    let payload = context::hook_stdin().cloned().unwrap_or_else(|| json!({}));
    let (tool_name, tool_input) = extract_tool(&payload);
    let class = classify_tool(&tool_name, &tool_input);

    if !class.needs_lease() {
        // An ambiguous call is the one we could not read, and until now it left
        // no trace at all: the only way anyone learned that `gh api` was
        // blocking a peer was that peer writing a note about it. Three defects
        // this week were found that way, which means the ones nobody bothered
        // to report are still here. Record the SHAPE of what we could not
        // classify — never the command, which carries tokens and paths — so
        // `harbard friction` can rank what to teach the classifier next.
        if class == WriteClass::Ambiguous {
            admission_audit(
                store,
                &ctx,
                Admission {
                    decision: "ambiguous",
                    class: class_label(class),
                    worktree: "",
                    claim_id: None,
                    holder: None,
                    holder_expires_at_ms: None,
                    shape: Some(&command_shape(&tool_name, &tool_input)),
                },
            );
        }
        return allow(common, &ctx, class, None, None, None);
    }

    // Infra / identity problems: never brick. Known writes still cannot land
    // without a lease once identity is available — but if we cannot even ask
    // the store, prefer a live agent over a frozen one.
    if !ctx.session_is_stable {
        return soft_allow(
            common,
            &ctx,
            class,
            &correction_lease_unprotected("writer gate: no stable session id").line(),
        );
    }
    let worktree = match resolve_write_worktree(&ctx, &tool_name, &tool_input) {
        Ok(Some(worktree)) => worktree,
        // A structured write outside Git does not contend for a checkout.
        Ok(None) => return allow(common, &ctx, class, None, None, None),
        Err(contested) => {
            // A call spanning several checkouts is denied without ever
            // reaching the lease store, so there is no single decided
            // worktree — and before this it left no audit row at all, which is
            // the least attributable block of them all. One row per contested
            // checkout keeps "which blocks touched this tree" answerable.
            for worktree in &contested {
                admission_audit(
                    store,
                    &ctx,
                    Admission {
                        decision: "blocked",
                        class: class_label(class),
                        worktree,
                        claim_id: None,
                        holder: None,
                        holder_expires_at_ms: None,
                        shape: None,
                    },
                );
            }
            return deny(common, &ctx, &correction_write_scope_conflict().line());
        }
    };
    let worktree = worktree.as_str();
    if worktree.is_empty() {
        return soft_allow(
            common,
            &ctx,
            class,
            &correction_lease_unprotected("writer gate: no Git worktree").line(),
        );
    }
    let Some(common_dir) = context::git_common_dir(Path::new(worktree)) else {
        return soft_allow(
            common,
            &ctx,
            class,
            &correction_lease_unprotected("writer gate: could not resolve Git common dir").line(),
        );
    };
    let resource =
        match LeaseResource::writer(&ctx.host, common_dir.display().to_string(), worktree) {
            Ok(resource) => resource,
            Err(_) => {
                return soft_allow(
                    common,
                    &ctx,
                    class,
                    &correction_lease_unprotected("writer gate: invalid writer resource").line(),
                );
            }
        };
    let pid = match pid_override {
        Some(explicit) => Some(explicit),
        None => match context::supervised_harness_pid(&ctx.harness) {
            Some(pid) => Some(pid),
            None => {
                return soft_allow(
                    common,
                    &ctx,
                    class,
                    &correction_lease_unprotected("writer gate: no live harness supervisor pid")
                        .line(),
                );
            }
        },
    };
    let metadata = LeaseMetadata {
        session: ctx.session.clone(),
        harness: ctx.harness.clone(),
        pid,
        parent: ctx.parent.clone(),
        repository_id: Some(common_dir.display().to_string()),
        branch: context::git_branch(Path::new(worktree)),
        head_sha: context::git_head_sha(Path::new(worktree)),
    };
    let leases = LeaseStore::at(store.root());
    match leases.ensure(
        &resource,
        &metadata,
        lease_ttl_secs(&ctx, worktree),
        SystemLeaseClock,
        &SystemPidChecker,
    ) {
        Ok(LeaseEnsure::Acquired(lease)) => {
            admission_audit(
                store,
                &ctx,
                Admission {
                    decision: "acquired",
                    class: class_label(class),
                    worktree,
                    claim_id: Some(&lease.claim_id),
                    holder: None,
                    holder_expires_at_ms: None,
                    shape: None,
                },
            );
            allow(
                common,
                &ctx,
                class,
                Some(lease.claim_id),
                Some(worktree),
                None,
            )
        }
        Ok(LeaseEnsure::Held(lease)) => {
            admission_audit(
                store,
                &ctx,
                Admission {
                    decision: "held",
                    class: class_label(class),
                    worktree,
                    claim_id: Some(&lease.claim_id),
                    holder: None,
                    holder_expires_at_ms: None,
                    shape: None,
                },
            );
            allow(
                common,
                &ctx,
                class,
                Some(lease.claim_id),
                Some(worktree),
                None,
            )
        }
        Ok(LeaseEnsure::Conflict(holder)) => {
            let reason = correction_lease_held(
                &short(&holder.session),
                &holder.harness,
                holder.expires_at_ms,
            )
            .line();
            admission_audit(
                store,
                &ctx,
                Admission {
                    decision: "blocked",
                    class: class_label(class),
                    worktree,
                    claim_id: Some(&holder.claim_id),
                    holder: Some(&holder.session),
                    holder_expires_at_ms: Some(holder.expires_at_ms),
                    shape: None,
                },
            );
            // Only Write reaches ensure(); conflict ⇒ deny. (Ambiguous never
            // enters this path — needs_lease is Write-only.)
            deny(common, &ctx, &reason)
        }
        Err(LeaseError::DeadPid) => {
            admission_audit(
                store,
                &ctx,
                Admission {
                    decision: "unprotected",
                    class: class_label(class),
                    worktree,
                    claim_id: None,
                    holder: None,
                    holder_expires_at_ms: None,
                    shape: None,
                },
            );
            soft_allow(
                common,
                &ctx,
                class,
                &correction_lease_unprotected("writer gate: harness supervisor pid is not alive")
                    .line(),
            )
        }
        Err(error) => {
            admission_audit(
                store,
                &ctx,
                Admission {
                    decision: "unprotected",
                    class: class_label(class),
                    worktree,
                    claim_id: None,
                    holder: None,
                    holder_expires_at_ms: None,
                    shape: None,
                },
            );
            soft_allow(
                common,
                &ctx,
                class,
                &correction_lease_unprotected(&format!("writer gate store error ({error})")).line(),
            )
        }
    }
}

/// Resolve the checkout that the pending write will actually mutate.
///
/// Structured file paths and shell working directories are stronger evidence
/// than the session cwd. `git -C` and destructive `git worktree` targets are
/// also honored. A call spanning multiple worktrees is rejected because the
/// current lease protocol cannot acquire multiple resources atomically; the
/// error carries the contested roots so the denial can still be attributed to
/// the checkouts it was made about.
fn resolve_write_worktree(
    ctx: &Context,
    tool_name: &str,
    tool_input: &Value,
) -> Result<Option<String>, Vec<String>> {
    let mut targets = Vec::<PathBuf>::new();
    let lower = tool_name.trim().to_ascii_lowercase();
    let base = tool_input
        .get("workdir")
        .or_else(|| tool_input.get("cwd"))
        .and_then(Value::as_str)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(&ctx.cwd));

    if matches!(
        lower.as_str(),
        "write"
            | "edit"
            | "multiedit"
            | "delete"
            | "notebookedit"
            | "notebook_edit"
            | "apply_patch"
            | "applypatch"
            | "editnotebook"
    ) {
        collect_structured_paths(tool_input, &base, &mut targets);
    }

    if matches!(
        lower.as_str(),
        "bash" | "shell" | "exec_command" | "powershell" | "terminal"
    ) && let Some(command) = tool_input
        .get("command")
        .and_then(Value::as_str)
        .or_else(|| tool_input.get("cmd").and_then(Value::as_str))
    {
        collect_git_scope_paths(command, &base, &mut targets);
    }

    // An explicit workdir scopes opaque shell writes even when the command
    // itself does not carry a path.
    if targets.is_empty()
        && (tool_input.get("workdir").is_some() || tool_input.get("cwd").is_some())
    {
        targets.push(base.clone());
    }

    if targets.is_empty() {
        return Ok(ctx.repo.clone());
    }

    let mut roots = BTreeSet::new();
    for target in targets {
        let resolved = if target.is_absolute() {
            target
        } else {
            base.join(target)
        };
        if let Some(root) = context::git_root(&resolved) {
            roots.insert(root.display().to_string());
        }
    }
    match roots.len() {
        0 => Ok(None),
        1 => Ok(roots.into_iter().next()),
        _ => Err(roots.into_iter().collect()),
    }
}

fn collect_structured_paths(input: &Value, base: &Path, targets: &mut Vec<PathBuf>) {
    for key in ["path", "file_path", "notebook_path"] {
        if let Some(path) = input.get(key).and_then(Value::as_str) {
            targets.push(PathBuf::from(path));
        }
    }
    for key in ["paths", "file_paths"] {
        if let Some(paths) = input.get(key).and_then(Value::as_array) {
            targets.extend(paths.iter().filter_map(Value::as_str).map(PathBuf::from));
        }
    }
    let direct = input.as_str().into_iter();
    let object_values = input
        .as_object()
        .into_iter()
        .flat_map(|map| map.values())
        .filter_map(Value::as_str);
    for text in direct.chain(object_values) {
        for line in text.lines() {
            let path = [
                "*** Add File: ",
                "*** Update File: ",
                "*** Delete File: ",
                "*** Move to: ",
            ]
            .iter()
            .find_map(|prefix| line.strip_prefix(prefix));
            if let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) {
                let path = PathBuf::from(path);
                targets.push(if path.is_absolute() {
                    path
                } else {
                    base.join(path)
                });
            }
        }
    }
}

fn collect_git_scope_paths(command: &str, base: &Path, targets: &mut Vec<PathBuf>) {
    let tokens = shell_tokens(command);
    for segment in tokens.split(|token| token == "\n") {
        let Some(git_index) = segment.iter().position(|token| {
            Path::new(token)
                .file_name()
                .is_some_and(|name| name == "git")
        }) else {
            continue;
        };
        let mut index = git_index + 1;
        while index < segment.len() {
            let token = &segment[index];
            if token == "-C" {
                if let Some(path) = segment.get(index + 1) {
                    targets.push(resolve_against(base, path));
                }
                index += 2;
                continue;
            }
            if let Some(path) = token.strip_prefix("-C")
                && !path.is_empty()
            {
                targets.push(resolve_against(base, path));
                index += 1;
                continue;
            }
            if token == "-c" {
                index += 2;
                continue;
            }
            if token.starts_with('-') {
                index += 1;
                continue;
            }
            break;
        }
        if segment.get(index).map(String::as_str) == Some("worktree")
            && matches!(
                segment.get(index + 1).map(String::as_str),
                Some("remove" | "move" | "lock" | "unlock")
            )
            && let Some(path) = segment[index + 2..]
                .iter()
                .find(|candidate| !candidate.starts_with('-'))
        {
            targets.push(resolve_against(base, path));
        }
    }
}

fn resolve_against(base: &Path, value: &str) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

/// Small shell lexer for path-bearing Git options. It deliberately ignores
/// expansions; unresolved dynamic paths fall back to the explicit workdir/cwd.
fn shell_tokens(command: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut quote = None;
    let mut escaped = false;
    for ch in command.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && quote != Some('\'') {
            escaped = true;
            continue;
        }
        if let Some(active) = quote {
            if ch == active {
                quote = None;
            } else {
                current.push(ch);
            }
            continue;
        }
        if ch == '\'' || ch == '"' {
            quote = Some(ch);
        } else if ch.is_whitespace() {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        } else if matches!(ch, ';' | '|' | '&') {
            if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
            if tokens.last().is_none_or(|token| token != "\n") {
                tokens.push("\n".to_string());
            }
        } else {
            current.push(ch);
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// One writer-admission decision, as recorded in `admission.jsonl`.
///
/// `worktree` is deliberately NOT optional. A row without one cannot be
/// attributed to a checkout, which is the state the first generation of rows
/// was in: 102 of the first 127 blocked writes named no checkout at all, so a
/// block could be counted but never explained. Making the field mandatory
/// moves that guarantee from review to the compiler.
struct Admission<'a> {
    decision: &'a str,
    class: &'a str,
    /// The checkout the decision was made for.
    worktree: &'a str,
    claim_id: Option<&'a str>,
    holder: Option<&'a str>,
    /// Blocked rows: when the holder's lease expires. Recorded as the
    /// remaining time so a row answers "how long is this checkout busy" on its
    /// own, without joining against the lease store that has since moved on.
    holder_expires_at_ms: Option<u64>,
    /// What KIND of call this was — never the call itself. A command line
    /// carries tokens, absolute paths and prose; its shape (`gh api`,
    /// `python3`, `Edit`) carries everything needed to decide what the
    /// classifier should learn next and nothing worth leaking.
    shape: Option<&'a str>,
}

/// A safe, groupable label for a tool call: the binary and its first
/// subcommand, or the tool's own name for structured calls.
///
/// Everything that could carry a secret is dropped — flags, anything with an
/// `=`, anything that looks like a path. What survives is the vocabulary the
/// classifier is missing, which is the only thing this needs to answer.
fn command_shape(tool_name: &str, tool_input: &Value) -> String {
    let command = tool_input
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if command.trim().is_empty() {
        return tool_name.to_string();
    }
    let mut parts = command
        .split_whitespace()
        .filter(|t| !t.starts_with('-') && !t.contains('=') && !t.contains('/'));
    let base = parts.next().unwrap_or_default();
    if base.is_empty() {
        return tool_name.to_string();
    }
    match parts.next() {
        Some(sub) => format!("{base} {sub}"),
        None => base.to_string(),
    }
}

/// Where a still-unresolved wait is remembered, keyed by session and worktree.
///
/// The admission log records decisions; it never recorded CONSEQUENCES. A
/// blocked row said someone was refused, and nothing anywhere said whether they
/// waited eight seconds or twenty minutes, or whether they ever got in at all.
/// Those two facts are what turn a pile of events into something you can learn
/// from: an outcome, and a cost.
fn waiting_path(store: &Store) -> PathBuf {
    store.root().join("leases").join("v1").join("waiting.json")
}

fn waiting_key(session: &str, worktree: &str) -> String {
    format!("{session}\u{1f}{worktree}")
}

fn read_waiting(store: &Store) -> serde_json::Map<String, Value> {
    std::fs::read_to_string(waiting_path(store))
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default()
}

fn write_waiting(store: &Store, map: &serde_json::Map<String, Value>) {
    let path = waiting_path(store);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(body) = serde_json::to_string(map) {
        let _ = std::fs::write(path, body);
    }
}

/// Mark the start of a wait, or close one and return what it cost.
///
/// Called on every admission, so it stays a single small file read: the common
/// case (an uncontested `held`) finds no file and does nothing.
fn resolve_wait(
    store: &Store,
    ctx: &Context,
    admission: &Admission<'_>,
    ts_ms: u64,
) -> Option<u64> {
    let key = waiting_key(&ctx.session, admission.worktree);
    let mut map = read_waiting(store);
    if admission.decision == "blocked" {
        // Only the FIRST refusal starts the clock; the retries that follow are
        // the wait itself, not new waits.
        if !map.contains_key(&key) {
            map.insert(key, json!(ts_ms));
            write_waiting(store, &map);
        }
        return None;
    }
    let started = map.remove(&key)?.as_u64()?;
    write_waiting(store, &map);
    Some(ts_ms.saturating_sub(started))
}

/// Auditable writer-admission decisions (M5c): observed acquired/held/blocked.
/// How long this write should hold the checkout.
///
/// A session writing in its OWN checkout is going to keep writing, so the full
/// window is right: renewing on every keystroke would be churn, and nobody else
/// should be in there anyway.
///
/// A write to someone else's checkout is a different act. It happens when an
/// tool records something in a shared store — a ledger appending a decision,
/// memory writing a file under the config repo — and it is over the moment the
/// tool call returns. Holding that resource for the full window afterwards only
/// blocks other sessions from their own short writes to the same store.
///
/// So a foreign checkout is leased for as long as the write plausibly takes and
/// not a second longer. It still serializes two simultaneous writers — which is
/// the whole point — and it stops being a wall the moment the writer is done.
fn lease_ttl_secs(ctx: &Context, worktree: &str) -> u64 {
    let own = ctx
        .repo
        .as_deref()
        .is_some_and(|repo| same_checkout(repo, worktree));
    if own {
        DEFAULT_LEASE_TTL_SECS
    } else {
        FOREIGN_WRITE_TTL_SECS
    }
}

/// A write into a checkout that is not the session's own outlives the tool call
/// by this much, and no more.
const FOREIGN_WRITE_TTL_SECS: u64 = 30;

fn same_checkout(a: &str, b: &str) -> bool {
    let canonical = |p: &str| std::fs::canonicalize(p).unwrap_or_else(|_| PathBuf::from(p));
    canonical(a) == canonical(b)
}

fn admission_audit(store: &Store, ctx: &Context, admission: Admission<'_>) {
    let dir = store.root().join("leases").join("v1");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("admission.jsonl");
    let ts_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    // Closes the open wait, if this session had one on this checkout. On a
    // `blocked` row it opens one instead and returns None.
    let waited_ms = resolve_wait(store, ctx, &admission, ts_ms);
    let row = json!({
        "ts_ms": ts_ms,
        "decision": admission.decision,
        // Present only on the row where a wait ENDED: how long this session was
        // refused this checkout before finally writing to it. Absent everywhere
        // else, so its presence is itself the "they got in" label.
        "waited_ms": waited_ms,
        "session": ctx.session,
        "harness": ctx.harness,
        "class": admission.class,
        "claim_id": admission.claim_id,
        "holder": admission.holder,
        // Derived against this row's own timestamp so the two can never
        // disagree about the instant the decision was made.
        "holder_expires_in_ms": admission
            .holder_expires_at_ms
            .map(|expires| expires.saturating_sub(ts_ms)),
        "parent": ctx.parent,
        "worktree": admission.worktree,
        "shape": admission.shape,
    });
    if let Ok(mut line) = serde_json::to_string(&row) {
        line.push('\n');
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
    }
}

/// Best-effort release of this session's writer lease (SessionEnd / Stop / bye).
pub fn release_writer_for_session(store: &Store, common: &Common) {
    let ctx = Context::resolve(common);
    if let Some(pid) = context::supervised_harness_pid(&ctx.harness) {
        let requester = LeaseMetadata {
            session: ctx.session.clone(),
            harness: ctx.harness.clone(),
            pid: Some(pid),
            parent: ctx.parent.clone(),
            repository_id: None,
            branch: None,
            head_sha: None,
        };
        let leases = LeaseStore::at(store.root());
        // Releasing NOTHING is not success. This path selects leases whose
        // recorded pid equals the supervisor pid resolved right now, so a
        // session whose lease was taken under a different pid released zero
        // and returned here as if done — leaving the checkout marked busy
        // until the TTL burned down, with its owner already gone. Fall
        // through to the session-scoped release, which only ever removes
        // this session's own lease on this worktree.
        if leases
            .release_owned_writers(&requester, SystemLeaseClock, &SystemPidChecker)
            .is_ok_and(|released| released > 0)
        {
            return;
        }
    }
    let Some(worktree) = ctx.repo.as_deref() else {
        return;
    };
    let Some(common_dir) = context::git_common_dir(std::path::Path::new(worktree)) else {
        return;
    };
    let Ok(resource) = LeaseResource::writer(&ctx.host, common_dir.display().to_string(), worktree)
    else {
        return;
    };
    let leases = LeaseStore::at(store.root());
    let _ = leases.release_session(&resource, &ctx.session, SystemLeaseClock);
}

fn extract_tool(payload: &Value) -> (String, Value) {
    // Claude Code / Codex: tool_name + tool_input.
    if let Some(name) = payload.get("tool_name").and_then(|v| v.as_str()) {
        let input = payload
            .get("tool_input")
            .cloned()
            .unwrap_or_else(|| json!({}));
        return (name.to_string(), input);
    }
    // Cursor preToolUse uses the same keys; beforeShellExecution sends `command`.
    if let Some(command) = payload.get("command").and_then(|v| v.as_str()) {
        let event = payload
            .get("hook_event_name")
            .or_else(|| payload.get("hookEventName"))
            .and_then(|v| v.as_str())
            .unwrap_or("beforeShellExecution");
        if event.eq_ignore_ascii_case("beforeShellExecution")
            || event.eq_ignore_ascii_case("before_shell_execution")
        {
            return ("Shell".to_string(), json!({ "command": command }));
        }
    }
    // Cursor may nest under tool_input still — already handled. Unknown → ambiguous.
    ("Unknown".to_string(), payload.clone())
}

fn allow(
    common: &Common,
    ctx: &Context,
    class: WriteClass,
    claim_id: Option<String>,
    worktree: Option<&str>,
    agent_message: Option<&str>,
) -> i32 {
    if common.json {
        println!(
            "{}",
            json!({
                "status": "allow",
                "class": class_label(class),
                "claim_id": claim_id,
                "session": ctx.session,
                "worktree": worktree,
                "warning": agent_message,
            })
        );
        return 0;
    }
    // Cursor failClosed treats empty stdout as failure — always emit permission.
    if ctx.harness == "cursor" {
        let mut body = json!({ "permission": "allow" });
        if let Some(msg) = agent_message {
            body["agent_message"] = json!(msg);
        }
        println!("{body}");
        return 0;
    }
    // Claude / Codex: silent allow is valid; surface soft warnings when present.
    if let Some(msg) = agent_message {
        println!(
            "{}",
            json!({
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "permissionDecision": "allow",
                    "permissionDecisionReason": msg,
                }
            })
        );
    }
    let _ = ctx;
    0
}

/// Allow with a harness warning (infra failure or foreign-lease gather).
///
/// Do **not** write the reason to stderr: Claude Code / Codex surface hook
/// stderr as a tool error even when `permissionDecision` is allow, which made
/// agents believe gather tools were blocked.
fn soft_allow(common: &Common, ctx: &Context, class: WriteClass, reason: &str) -> i32 {
    allow(common, ctx, class, None, None, Some(reason))
}

fn deny(common: &Common, ctx: &Context, reason: &str) -> i32 {
    if common.json {
        // Machine contract for tests / operators: non-zero means blocked.
        // Reason lives on stdout only — never stderr (harnesses surface stderr
        // as a tool Error and agents misread it as a Claude permissions ask).
        println!(
            "{}",
            json!({
                "status": "deny",
                "reason": reason,
                "session": ctx.session,
            })
        );
        return 2;
    }
    // Hook path: Claude Code and Codex only honor decision JSON on exit 0;
    // exit 2 ignores stdout. Cursor accepts JSON `permission: deny` the same way.
    // Keep stderr empty — same soft_allow discipline.
    let body = deny_payload(&ctx.harness, reason);
    println!("{body}");
    0
}

fn deny_payload(harness: &str, reason: &str) -> String {
    match harness {
        "cursor" => json!({
            "permission": "deny",
            "agent_message": reason,
            "user_message": reason,
        })
        .to_string(),
        // Claude Code + Codex: nested hookSpecificOutput.
        _ => json!({
            "hookSpecificOutput": {
                "hookEventName": "PreToolUse",
                "permissionDecision": "deny",
                "permissionDecisionReason": reason,
            }
        })
        .to_string(),
    }
}

fn class_label(class: WriteClass) -> &'static str {
    match class {
        WriteClass::Read => "read",
        WriteClass::Write => "write",
        WriteClass::Ambiguous => "ambiguous",
    }
}

fn short(session: &str) -> String {
    let s = session.trim();
    if s.len() <= 12 {
        s.to_string()
    } else {
        format!("{}…", &s[..12])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The measured failure: an organ appends one line to a shared ledger and
    /// the checkout it borrowed stays locked for fifteen minutes, blocking
    /// every other session that needs to record anything of its own.
    #[test]
    fn a_write_into_someone_elses_checkout_is_leased_briefly() {
        let mine = std::env::temp_dir().join("harbard-ttl-mine");
        let theirs = std::env::temp_dir().join("harbard-ttl-theirs");
        std::fs::create_dir_all(&mine).expect("mine");
        std::fs::create_dir_all(&theirs).expect("theirs");
        let ctx = context_at(&mine);

        assert_eq!(
            lease_ttl_secs(&ctx, &mine.display().to_string()),
            DEFAULT_LEASE_TTL_SECS,
            "a session keeps writing in its own tree; the full window is right"
        );
        let borrowed = lease_ttl_secs(&ctx, &theirs.display().to_string());
        assert_eq!(borrowed, FOREIGN_WRITE_TTL_SECS);
        assert!(
            borrowed < DEFAULT_LEASE_TTL_SECS,
            "a borrowed checkout must not be held for a working session's window"
        );
        assert!(
            borrowed > 0,
            "it still has to serialize two simultaneous writers"
        );

        // Spelling the same directory differently is the same checkout.
        let spelled = format!("{}/.", mine.display());
        assert_eq!(lease_ttl_secs(&ctx, &spelled), DEFAULT_LEASE_TTL_SECS);

        let _ = std::fs::remove_dir_all(&mine);
        let _ = std::fs::remove_dir_all(&theirs);
    }

    fn context_at(path: &Path) -> Context {
        Context {
            session: "session".into(),
            session_is_stable: true,
            harness: "codex".into(),
            host: "host".into(),
            cwd: path.display().to_string(),
            repo: Some(path.display().to_string()),
            repo_key: None,
            branch: None,
            role: harbard_core::Role::Main,
            parent: None,
            nickname: None,
        }
    }

    #[test]
    fn extract_prefers_tool_name() {
        let (name, input) = extract_tool(&json!({
            "tool_name": "Bash",
            "tool_input": { "command": "git status" }
        }));
        assert_eq!(name, "Bash");
        assert_eq!(input["command"], "git status");
    }

    #[test]
    fn extract_cursor_shell_event() {
        let (name, input) = extract_tool(&json!({
            "hook_event_name": "beforeShellExecution",
            "command": "rm -rf src"
        }));
        assert_eq!(name, "Shell");
        assert_eq!(input["command"], "rm -rf src");
    }

    #[test]
    fn deny_payload_shapes() {
        let cursor = deny_payload("cursor", "blocked");
        assert!(cursor.contains("\"permission\":\"deny\""));
        let claude = deny_payload("claude-code", "blocked");
        assert!(claude.contains("permissionDecision"));
        assert!(claude.contains("PreToolUse"));
    }

    #[test]
    fn write_scope_uses_structured_paths_workdir_and_git_c() {
        let root = std::env::temp_dir().join(format!("harbard-gate-scope-{}", std::process::id()));
        let primary = root.join("primary");
        let target = root.join("target");
        std::fs::create_dir_all(primary.join(".git")).unwrap();
        std::fs::create_dir_all(target.join(".git")).unwrap();
        let ctx = context_at(&primary);
        let target_file = target.join("src/lib.rs");
        let expected = std::fs::canonicalize(&target)
            .unwrap()
            .display()
            .to_string();

        for (tool, input) in [
            ("Write", json!({"path": target_file})),
            ("exec_command", json!({"cmd": "touch x", "workdir": target})),
            (
                "Bash",
                json!({"command": format!("git -C '{}' commit -m x", target.display())}),
            ),
            (
                "apply_patch",
                json!({"patch": format!("*** Begin Patch\n*** Update File: {}\n", target_file.display())}),
            ),
            (
                "apply_patch",
                Value::String(format!(
                    "*** Begin Patch\n*** Update File: {}\n",
                    target_file.display()
                )),
            ),
        ] {
            assert_eq!(
                resolve_write_worktree(&ctx, tool, &input).unwrap(),
                Some(expected.clone()),
                "{tool}: {input}"
            );
        }

        assert_eq!(
            resolve_write_worktree(
                &ctx,
                "Bash",
                &json!({"command": format!("cargo -C '{}' build", target.display())})
            )
            .unwrap(),
            ctx.repo,
            "a non-Git -C flag must not retarget the writer lease"
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn one_patch_spanning_worktrees_is_rejected() {
        let root =
            std::env::temp_dir().join(format!("harbard-gate-multiscope-{}", std::process::id()));
        let primary = root.join("primary");
        let target = root.join("target");
        std::fs::create_dir_all(primary.join(".git")).unwrap();
        std::fs::create_dir_all(target.join(".git")).unwrap();
        let ctx = context_at(&primary);
        let input = json!({
            "patch": format!(
                "*** Begin Patch\n*** Update File: {}\n*** Update File: {}\n",
                primary.join("a.rs").display(),
                target.join("b.rs").display()
            )
        });
        assert!(resolve_write_worktree(&ctx, "apply_patch", &input).is_err());

        let _ = std::fs::remove_dir_all(root);
    }
}
