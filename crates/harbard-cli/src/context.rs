//! Resolve the calling session's identity and location from flags, environment,
//! and hook stdin — the only per-harness knowledge in the binary lives here and
//! in `install`.
//!
//! Session id precedence: --session, HARBARD_SESSION, hook stdin session_id /
//! agent_id, then the resolved harness's own session environment variable.
//! Markers inherited from a different harness never supply that identity.
//! hello still permits a hostname-pid fallback; inbox refuses unstable identity.
//!
//! Claude Code and Codex Task children share the parent's `session_id` on hook
//! stdin and carry their own id as `agent_id`. When `agent_id` is present we
//! record the child as the session and the parent under `parent`, with
//! `role = herd`. Codex can also heal orphans from `thread_spawn_edges`;
//! Claude has no equivalent DB — SubagentStart must fire for the link to land.

use crate::cli::Common;
use std::collections::HashSet;
use std::io::{IsTerminal, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The recognized harnesses (the parity set). `cursor` and `antigravity`
/// joined the set with the same verbs, each through its own hooks surface.
pub const HARNESSES: [&str; 5] = ["claude-code", "codex", "cursor", "antigravity", "other"];

/// Resolved calling context for building an event.
#[derive(Debug, Clone)]
pub struct Context {
    pub session: String,
    pub session_is_stable: bool,
    pub harness: String,
    pub host: String,
    pub cwd: String,
    pub repo: Option<String>,
    /// Identity of the REPOSITORY this checkout belongs to, shared by every
    /// sibling worktree. Distinct from `repo`, which stays the working tree the
    /// session actually sits in, so the desk keeps showing one berth per tree.
    pub repo_key: Option<String>,
    pub branch: Option<String>,
    pub role: harbard_core::Role,
    /// Parent session when this writer is a herd child (`agent_id` on stdin,
    /// or a Codex spawn-edge heal).
    pub parent: Option<String>,
    /// Best-effort nickname for herd hello alias claims (Codex agent nick, or
    /// Claude `agent_type`).
    pub nickname: Option<String>,
}

impl Context {
    /// Resolve from the shared CLI options, reading hook stdin JSON only when
    /// `--hook <harness>` is present and stdin is piped.
    pub fn resolve(common: &Common) -> Context {
        // Only consume stdin when a hook harness is declared. Cache it so the
        // gate (and any later helper in the same process) can re-read the
        // full tool payload after identity resolution.
        let hook_json = if common.hook.is_some() {
            hook_stdin().cloned()
        } else {
            None
        };
        let hook_str = |key: &str| -> Option<String> {
            hook_json
                .as_ref()
                .and_then(|v| v.get(key))
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };
        // Cursor's hooks send the stable id as `conversation_id` on every
        // event (`session_id` only rides sessionStart/sessionEnd), and the
        // workspace path as `workspace_roots[0]` — `cwd` exists only on the
        // tool events. Claude/Codex keys keep precedence where they overlap.
        // Antigravity's hooks speak camelCase: the stable id is
        // `conversationId` on every event, beside a real `cwd`.
        let hook_conversation = || {
            hook_str("conversation_id")
                .or_else(|| hook_str("conversationId").filter(|v| !v.trim().is_empty()))
        };
        let hook_workspace_root = || {
            hook_json
                .as_ref()
                .and_then(|v| v.get("workspace_roots"))
                .and_then(|v| v.as_array())
                .and_then(|roots| roots.first())
                .and_then(|v| v.as_str())
                .map(str::to_string)
        };

        let host = harbard_core::hostname();

        // Herd children (Claude + Codex): `session_id` is the parent;
        // `agent_id` is the child. Prefer the child as our session so the
        // board can nest.
        let agent_id = hook_str("agent_id").filter(|v| !v.trim().is_empty());
        let hook_session = hook_str("session_id").filter(|v| !v.trim().is_empty());
        let agent_type = hook_str("agent_type").filter(|v| !v.trim().is_empty());

        let harness = normalize_harness(
            common
                .harness
                .clone()
                .or_else(|| common.hook.clone())
                .or_else(|| env_nonempty("HARBARD_HARNESS"))
                .or_else(detect_harness),
        );
        let (session, session_is_stable, inferred_parent, inferred_herd) =
            if let Some(child) = agent_id.clone() {
                // Codex reports the lineage root as hook `session_id`, while
                // its state DB retains the immediate spawn edge. Prefer that
                // edge when present; Claude has no DB entry and falls back to
                // the hook parent unchanged.
                let parent = select_child_parent(hook_session.clone(), codex_parent_of(&child));
                (child, true, parent, true)
            } else {
                let stable_session = common
                    .session
                    .clone()
                    .filter(|value| !value.trim().is_empty())
                    .or_else(|| env_nonempty("HARBARD_SESSION"))
                    .or_else(|| hook_session.clone())
                    .or_else(hook_conversation)
                    // Inherited markers from another harness are not our identity.
                    .or_else(|| match harness.as_str() {
                        "claude-code" => env_nonempty("CLAUDE_CODE_SESSION_ID"),
                        "codex" => env_nonempty("CODEX_THREAD_ID"),
                        "cursor" => env_nonempty("CURSOR_CONVERSATION_ID"),
                        "antigravity" => env_nonempty("ANTIGRAVITY_CONVERSATION_ID"),
                        _ => None,
                    });
                let session_is_stable = stable_session.is_some();
                let session =
                    stable_session.unwrap_or_else(|| format!("{host}-{}", std::process::id()));
                // Heal orphans: a manual verb inside a Codex child has
                // CODEX_THREAD_ID = child id with no agent_id on stdin.
                let healed = codex_parent_of(&session);
                let herd = healed.is_some();
                (session, session_is_stable, healed, herd)
            };

        let cwd = common
            .cwd
            .clone()
            .or_else(|| hook_str("cwd"))
            .or_else(hook_workspace_root)
            .or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|p| p.display().to_string())
            })
            .unwrap_or_else(|| ".".to_string());

        let repo = match &common.repo {
            Some(r) => Some(canonical_repo(r)),
            None => git_root(Path::new(&cwd)).map(|p| p.display().to_string()),
        };

        let repo_key = repo.as_deref().and_then(|r| repo_key(Path::new(r)));

        let branch = common
            .branch
            .clone()
            .or_else(|| repo.as_deref().and_then(|r| git_branch(Path::new(r))));

        let role = common
            .role
            .clone()
            .or_else(|| env_nonempty("HARBARD_ROLE"))
            .map(|r| normalize_role(&r))
            .unwrap_or(if inferred_herd {
                harbard_core::Role::Herd
            } else {
                harbard_core::Role::Main
            });

        let nickname = if matches!(role, harbard_core::Role::Herd) {
            codex_nickname(&session).or_else(|| agent_type.map(|t| t.to_ascii_lowercase()))
        } else {
            None
        };

        Context {
            session,
            session_is_stable,
            harness,
            host,
            cwd,
            repo,
            repo_key,
            branch,
            role,
            parent: inferred_parent,
            nickname,
        }
    }
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

fn select_child_parent(
    hook_parent: Option<String>,
    immediate_parent: Option<String>,
) -> Option<String> {
    immediate_parent.or(hook_parent)
}

/// `$CODEX_HOME` or `~/.codex`, then the live state DB used for spawn edges.
fn codex_state_db() -> Option<PathBuf> {
    let db = harbard_core::harness_trace::codex_home()?.join("state_5.sqlite");
    if db.is_file() { Some(db) } else { None }
}

/// Best-effort: `thread_spawn_edges.parent_thread_id` for this child.
fn codex_parent_of(child: &str) -> Option<String> {
    let db = codex_state_db()?;
    sqlite_scalar(
        &db,
        &format!(
            "SELECT parent_thread_id FROM thread_spawn_edges WHERE child_thread_id = '{}';",
            sql_escape(child)
        ),
    )
}

/// Best-effort: Codex's agent nickname for a thread (Hilbert, Hooke, …).
pub fn codex_nickname(session: &str) -> Option<String> {
    let db = codex_state_db()?;
    sqlite_scalar(
        &db,
        &format!(
            "SELECT agent_nickname FROM threads WHERE id = '{}' AND agent_nickname IS NOT NULL AND agent_nickname != '';",
            sql_escape(session)
        ),
    )
}

fn sql_escape(s: &str) -> String {
    s.replace('\'', "''")
}

fn sqlite_scalar(db: &Path, sql: &str) -> Option<String> {
    let out = Command::new("sqlite3")
        .args(["-readonly", "-batch", db.to_str()?, sql])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

/// Cursor executes Claude Code's hook file (`~/.claude/settings.json`) beside
/// its own, so one Cursor conversation fires both hook sets with the same id —
/// the Claude line landing milliseconds after the Cursor one. Those echoes
/// re-branded live Cursor sessions as `claude-code` and buried them under a
/// `bye` at every turn boundary. A hook that declares one harness while
/// carrying another's payload is not a session signal; the caller drops it.
///
/// The tell is `workspace_roots`, which Cursor sends in place of `cwd` and no
/// other harness sends at all. `conversation_id` is deliberately NOT part of
/// the signature: it is a plausible field name for any harness, and a false
/// positive here silences a real hook. Absent the signature we say nothing —
/// a payload we cannot attribute is left to the declared harness.
pub fn cross_harness_hook(declared: &str, payload: Option<&serde_json::Value>) -> bool {
    let Some(payload) = payload else {
        return false;
    };
    payload_is_cursor(payload) && normalize_harness(Some(declared.to_string())) != "cursor"
}

/// Where an Antigravity `PreInvocation` falls in its turn. The event fires
/// before every model call: `invocationNum` 0 is the first call of a turn,
/// the prompt boundary, and every later one sits between two steps of the
/// same turn. `None` for any other payload, which is not split at all.
pub fn antigravity_turn_start(payload: Option<&serde_json::Value>) -> Option<bool> {
    let payload = payload?;
    if payload.get("hookEventName").and_then(|v| v.as_str()) != Some("PreInvocation") {
        return None;
    }
    let invocation = payload
        .get("invocationNum")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    Some(invocation == 0)
}

/// Cursor's hook signature: the workspace roots it sends in place of `cwd`.
fn payload_is_cursor(payload: &serde_json::Value) -> bool {
    payload
        .get("workspace_roots")
        .and_then(|v| v.as_array())
        .is_some_and(|roots| !roots.is_empty())
}

/// Detect the harness from ambient environment when neither `--hook`,
/// `--harness`, nor `$HARBARD_HARNESS` is given (M1.1c). A hand-run verb inside a
/// Codex shell must record `harness: codex`, not degrade to `other`.
///
/// Claude Code exposes `CLAUDE_CODE_SESSION_ID`/`CLAUDECODE`; Codex injects
/// `CODEX_*` markers. Cursor Agent shells export `CURSOR_CONVERSATION_ID` +
/// `CURSOR_AGENT` — conversation-scoped, unlike `CURSOR_VERSION` which every
/// child of the IDE inherits (including Claude/Codex in the integrated
/// terminal). Never key Cursor off `CURSOR_VERSION`.
///
/// Antigravity (`agy`) agent shells export `ANTIGRAVITY_AGENT=1` beside
/// `ANTIGRAVITY_CONVERSATION_ID`; the same pair-only rule applies, so its
/// install-wide variables (`ANTIGRAVITY_APP_DATA_DIR`, `ANTIGRAVITY_LS_VERSION`)
/// never claim a shell on their own.
fn detect_harness() -> Option<String> {
    let claude_session = env_nonempty("CLAUDE_CODE_SESSION_ID").is_some();
    let codex_session = env_nonempty("CODEX_THREAD_ID").is_some();
    let cursor_session = is_cursor_agent_shell();
    let antigravity_session = is_antigravity_agent_shell();
    let identities = usize::from(claude_session)
        + usize::from(codex_session)
        + usize::from(cursor_session)
        + usize::from(antigravity_session);
    if identities == 1 {
        return classify_harness(
            claude_session,
            codex_session,
            cursor_session,
            antigravity_session,
        )
        .map(str::to_string);
    }
    if identities > 1 {
        // Nested harnesses inherit each other's markers. Actual ancestry wins;
        // with no attributable parent the caller must supply --harness/--session.
        return process_ancestry()
            .and_then(nearest_harness_ancestor)
            .map(|(_, harness)| harness.to_string());
    }
    classify_harness(
        env_nonempty("CLAUDECODE").is_some(),
        env_has_prefix("CODEX_"),
        false,
        false,
    )
    .map(str::to_string)
}

/// Cursor Agent tool shells (not the bare IDE terminal): conversation id + agent flag.
fn is_cursor_agent_shell() -> bool {
    if env_nonempty("CURSOR_CONVERSATION_ID").is_none() {
        return false;
    }
    flag_set(env_nonempty("CURSOR_AGENT").as_deref())
}

/// Antigravity agent tool shells: conversation id + agent flag, both scoped to
/// one conversation. A hook process carries the id without the flag, but a
/// hook always declares `--hook antigravity` and never needs detection.
fn is_antigravity_agent_shell() -> bool {
    antigravity_agent_shell(
        env_nonempty("ANTIGRAVITY_CONVERSATION_ID").as_deref(),
        env_nonempty("ANTIGRAVITY_AGENT").as_deref(),
    )
}

/// Pure form of [`is_antigravity_agent_shell`].
fn antigravity_agent_shell(conversation: Option<&str>, agent: Option<&str>) -> bool {
    conversation.is_some_and(|id| !id.trim().is_empty()) && flag_set(agent)
}

/// An environment flag that is present and not switched off.
fn flag_set(value: Option<&str>) -> bool {
    matches!(value, Some(v) if !v.is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"))
}

/// Pure precedence: Claude > Codex > Cursor > Antigravity agent shell; no
/// marker → `other`. With more than one marker live, ancestry decides before
/// this order is consulted.
fn classify_harness(
    claude: bool,
    codex: bool,
    cursor: bool,
    antigravity: bool,
) -> Option<&'static str> {
    if claude {
        Some("claude-code")
    } else if codex {
        Some("codex")
    } else if cursor {
        Some("cursor")
    } else if antigravity {
        Some("antigravity")
    } else {
        None
    }
}

/// True when any live environment variable name starts with `prefix`.
fn env_has_prefix(prefix: &str) -> bool {
    any_key_has_prefix(
        std::env::vars_os().map(|(k, _)| k.to_string_lossy().into_owned()),
        prefix,
    )
}

/// Pure prefix scan over a set of variable names (testable without touching the
/// process environment).
fn any_key_has_prefix<I: IntoIterator<Item = String>>(keys: I, prefix: &str) -> bool {
    keys.into_iter().any(|k| k.starts_with(prefix))
}

/// Clamp an arbitrary harness label to the parity set; unknown becomes `other`.
pub fn normalize_harness(h: Option<String>) -> String {
    match h {
        Some(v) if HARNESSES.contains(&v.as_str()) => v,
        Some(v) if v == "claude" => "claude-code".to_string(),
        _ => "other".to_string(),
    }
}

/// Find the long-lived harness process that supervises this short CLI
/// invocation. Agent shell tools commonly run `harbard` through an ephemeral
/// shell, so leasing that shell's pid makes the writer claim stale as soon as
/// the tool call returns. The first recognized harness ancestor is the stable
/// lifecycle boundary we want.
pub fn supervised_harness_pid(harness: &str) -> Option<u32> {
    resolve_supervised_pid(harness, process_ancestry()?)
}

/// Pick a stable supervisor from an already-collected ancestry chain.
/// Prefer the calling context's harness; when context is `other`, accept the
/// first Claude Code / Codex / Cursor / Antigravity ancestor (Cursor is often
/// explicit-only).
pub fn resolve_supervised_pid(harness: &str, ancestry: &[(u32, String)]) -> Option<u32> {
    if let Some(pid) = select_harness_ancestor(harness, ancestry) {
        return Some(pid);
    }
    if harness == "other" {
        return nearest_harness_ancestor(ancestry).map(|(pid, _)| pid);
    }
    None
}

fn nearest_harness_ancestor(ancestry: &[(u32, String)]) -> Option<(u32, &'static str)> {
    ancestry.iter().find_map(|(pid, command)| {
        let normalized = command.to_ascii_lowercase();
        ["claude-code", "codex", "cursor", "antigravity"]
            .into_iter()
            .find(|harness| normalized_command_matches_harness(harness, &normalized))
            .map(|harness| (*pid, harness))
    })
}

/// Walk from this process toward PID 1. Each entry is `(ancestor_pid, command)`
/// for that ancestor — never the child's command paired with the parent's pid.
fn process_ancestry() -> Option<&'static [(u32, String)]> {
    static ANCESTRY: OnceLock<Option<Vec<(u32, String)>>> = OnceLock::new();
    ANCESTRY.get_or_init(read_process_ancestry).as_deref()
}

fn read_process_ancestry() -> Option<Vec<(u32, String)>> {
    let mut ancestry = Vec::new();
    let (mut current, _) = process_ppid_and_command(std::process::id())?;
    let mut seen = HashSet::new();

    for _ in 0..32 {
        if current == 0 || !seen.insert(current) {
            break;
        }
        let (parent, command) = process_ppid_and_command(current)?;
        ancestry.push((current, command));
        if parent == 0 || parent == current {
            break;
        }
        current = parent;
    }

    Some(ancestry)
}

fn process_ppid_and_command(pid: u32) -> Option<(u32, String)> {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "ppid=", "-o", "command="])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let line = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let mut parts = line.splitn(2, char::is_whitespace);
    let parent = parts.next()?.trim().parse().ok()?;
    let command = parts.next().unwrap_or_default().trim().to_string();
    Some((parent, command))
}

fn select_harness_ancestor(harness: &str, ancestry: &[(u32, String)]) -> Option<u32> {
    ancestry
        .iter()
        .find(|(_, command)| {
            normalized_command_matches_harness(harness, &command.to_ascii_lowercase())
        })
        .map(|(pid, _)| *pid)
}

fn normalized_command_matches_harness(harness: &str, command: &str) -> bool {
    match harness {
        "codex" => {
            command
                .split_whitespace()
                .next()
                .is_some_and(|binary| binary.ends_with("/codex") || binary == "codex")
                || command.contains("/node_modules/@openai/codex/")
        }
        "claude-code" => {
            command
                .split_whitespace()
                .next()
                .is_some_and(|binary| binary.ends_with("/claude") || binary == "claude")
                || command.contains("/node_modules/@anthropic-ai/claude-code/")
        }
        "cursor" => {
            command.contains("/cursor.app/")
                || command.split_whitespace().next().is_some_and(|binary| {
                    binary.ends_with("/cursor")
                        || binary == "cursor"
                        || binary.ends_with("/cursor-agent")
                        || binary == "cursor-agent"
                })
        }
        // `agy` runs hook commands as its direct children, whether it was
        // started by name (`agy`) or by path (`/…/bin/agy -p …`).
        "antigravity" => command
            .split_whitespace()
            .next()
            .is_some_and(|binary| binary.ends_with("/agy") || binary == "agy"),
        _ => false,
    }
}

/// Whether this hook runs under a one-shot `claude -p`: a session that ends
/// after its turn, has no human on the other side, and so can never be woken.
///
/// Claude Code (2.1.310, `T()` in the bundled CLI) sets
/// `CLAUDE_CODE_ENTRYPOINT=sdk-cli` when the CLI runs non-interactively, and
/// rewrites an inherited `cli` to `sdk-cli`, so a `claude -p` launched from an
/// interactive session's shell still reads `sdk-cli`. Desktop, VS Code, remote
/// and SDK hosts keep their own entrypoint, and an interactive CLI reads `cli`.
/// The hook environment also carries `CLAUDE_PID`, the harness process itself;
/// its command line can only veto: no `-p`/`--print`, or a streaming input
/// that keeps the session open for more turns, keeps the watcher.
pub fn claude_print_turn() -> bool {
    let entrypoint = std::env::var("CLAUDE_CODE_ENTRYPOINT").ok();
    if entrypoint.as_deref() != Some("sdk-cli") {
        return false;
    }
    let harness_command = std::env::var("CLAUDE_PID")
        .ok()
        .and_then(|pid| pid.trim().parse::<u32>().ok())
        .and_then(process_ppid_and_command)
        .map(|(_, command)| command);
    one_shot_print(entrypoint.as_deref(), harness_command.as_deref())
}

/// Pure decision behind [`claude_print_turn`]. An unreadable harness command
/// leaves the entrypoint alone to decide.
pub fn one_shot_print(entrypoint: Option<&str>, harness_command: Option<&str>) -> bool {
    if entrypoint != Some("sdk-cli") {
        return false;
    }
    let Some(command) = harness_command else {
        return true;
    };
    let args: Vec<&str> = command.split_whitespace().skip(1).collect();
    let print = args.iter().any(|a| *a == "-p" || *a == "--print");
    let streaming = args.iter().enumerate().any(|(i, a)| {
        *a == "--input-format=stream-json"
            || (*a == "--input-format" && args.get(i + 1) == Some(&"stream-json"))
    });
    print && !streaming
}

fn normalize_role(r: &str) -> harbard_core::Role {
    match r {
        "herd" => harbard_core::Role::Herd,
        "headless" => harbard_core::Role::Headless,
        _ => harbard_core::Role::Main,
    }
}

/// Cached hook stdin JSON for this process (one hook invocation = one payload).
static HOOK_STDIN: OnceLock<Option<serde_json::Value>> = OnceLock::new();

/// Return the hook payload read from stdin, if any. Safe to call repeatedly.
pub fn hook_stdin() -> Option<&'static serde_json::Value> {
    HOOK_STDIN.get_or_init(read_hook_stdin).as_ref()
}

/// Read hook JSON from stdin (bounded to a piped, non-terminal source).
fn read_hook_stdin() -> Option<serde_json::Value> {
    let mut stdin = std::io::stdin();
    if stdin.is_terminal() {
        return None;
    }
    let mut buf = String::new();
    stdin.read_to_string(&mut buf).ok()?;
    if buf.trim().is_empty() {
        return None;
    }
    serde_json::from_str(&buf).ok()
}

/// Walk up from `start` to the first `.git` (dir or worktree file). Returns the
/// canonicalized repo root, or `None` outside a repo.
pub fn git_root(start: &Path) -> Option<PathBuf> {
    let mut dir = std::fs::canonicalize(start).unwrap_or_else(|_| start.to_path_buf());
    loop {
        if dir.join(".git").exists() {
            // `start` is commonly a file that does not exist yet. In that
            // case its ancestors may still contain symlinked path segments
            // (`/var` vs `/private/var` on macOS). Canonicalize the root at
            // the point where it is known to exist so one checkout cannot
            // acquire two differently spelled writer resources.
            return Some(std::fs::canonicalize(&dir).unwrap_or(dir));
        }
        dir = dir.parent()?.to_path_buf();
    }
}

/// Canonicalize a path to its git root, or to the absolute path when not a repo.
pub fn canonical_repo(path: &Path) -> String {
    if let Some(root) = git_root(path) {
        return root.display().to_string();
    }
    std::fs::canonicalize(path)
        .unwrap_or_else(|_| path.to_path_buf())
        .display()
        .to_string()
}

/// Identity of the repository a path belongs to: the root of its PRIMARY
/// working tree, which every sibling worktree resolves to alike.
///
/// A linked worktree's `.git` is a pointer file, and its `commondir` leads back
/// to the primary tree's `.git`; the primary root is that directory's parent.
/// This is the same shared-git-dir identity the lease subsystem already uses to
/// tell sibling worktrees apart from unrelated repositories.
///
/// Returns `None` when the common dir is not a `.git` directory inside its
/// working tree — a bare repository (`/srv/repo.git`, whose parent is an
/// unrelated directory) or a submodule (`/primary/.git/modules/<name>`). The
/// caller then keeps the plain checkout path, which is never wrong, only narrow.
pub fn repo_key(path: &Path) -> Option<String> {
    let common = git_common_dir(path)?;
    if common.file_name().and_then(|n| n.to_str()) != Some(".git") {
        return None;
    }
    let root = common.parent()?;
    if !root.join(".git").exists() {
        return None;
    }
    Some(
        std::fs::canonicalize(root)
            .unwrap_or_else(|_| root.to_path_buf())
            .display()
            .to_string(),
    )
}

/// Canonicalize a `repo:` recipient to the repository it names, so a note sent
/// to any worktree of a repository reaches every session working in it.
pub fn canonical_repo_recipient(path: &Path) -> String {
    repo_key(path).unwrap_or_else(|| canonical_repo(path))
}

/// Resolve the current branch of a repo root from its `.git/HEAD`, handling
/// worktrees (`.git` as a `gitdir:` pointer file). `None` when detached.
pub fn git_branch(repo: &Path) -> Option<String> {
    let head_dir = git_dir(repo)?;
    let head = std::fs::read_to_string(head_dir.join("HEAD")).ok()?;
    let head = head.trim();
    head.strip_prefix("ref: refs/heads/").map(str::to_string)
}

/// Resolve the shared Git directory used to identify sibling worktrees.
pub fn git_common_dir(repo: &Path) -> Option<PathBuf> {
    let git_dir = git_dir(repo)?;
    let common = std::fs::read_to_string(git_dir.join("commondir"))
        .ok()
        .map(|value| git_dir.join(value.trim()))
        .unwrap_or(git_dir);
    std::fs::canonicalize(&common).ok().or(Some(common))
}

/// Resolve the exact commit currently checked out without invoking Git.
pub fn git_head_sha(repo: &Path) -> Option<String> {
    let git_dir = git_dir(repo)?;
    let common_dir = git_common_dir(repo)?;
    let head = std::fs::read_to_string(git_dir.join("HEAD")).ok()?;
    let head = head.trim();
    if !head.starts_with("ref: ") {
        return valid_sha(head).then(|| head.to_string());
    }
    let reference = head.strip_prefix("ref: ")?;
    for base in [&git_dir, &common_dir] {
        if let Ok(value) = std::fs::read_to_string(base.join(reference)) {
            let value = value.trim();
            if valid_sha(value) {
                return Some(value.to_string());
            }
        }
    }
    let packed = std::fs::read_to_string(common_dir.join("packed-refs")).ok()?;
    packed.lines().find_map(|line| {
        let (sha, name) = line.split_once(' ')?;
        (name == reference && valid_sha(sha)).then(|| sha.to_string())
    })
}

fn git_dir(repo: &Path) -> Option<PathBuf> {
    let git = repo.join(".git");
    if git.is_dir() {
        return std::fs::canonicalize(&git).ok().or(Some(git));
    }
    let contents = std::fs::read_to_string(&git).ok()?;
    let pointer = contents.strip_prefix("gitdir:")?.trim();
    let pointer = PathBuf::from(pointer);
    let resolved = if pointer.is_absolute() {
        pointer
    } else {
        repo.join(pointer)
    };
    std::fs::canonicalize(&resolved).ok().or(Some(resolved))
}

fn valid_sha(value: &str) -> bool {
    matches!(value.len(), 40 | 64) && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The one recipient without a `kind:`: the human reading the app. It is
/// stored exactly as `harness:human`, so nothing downstream tells them apart.
pub const OPERATOR_TARGET: &str = "operator";

/// Parse a `--to` recipient string (`kind:value`, or `operator`).
pub fn parse_recipient(s: &str) -> Result<harbard_core::Recipient, String> {
    use harbard_core::{Recipient, RecipientKind};
    if s.trim() == OPERATOR_TARGET {
        return Ok(Recipient {
            kind: RecipientKind::Harness,
            value: harbard_core::HARNESS_HUMAN.to_string(),
        });
    }
    let (kind, value) = s
        .split_once(':')
        .ok_or_else(|| {
            format!(
                "recipient must be kind:value or operator, got \"{s}\"; for example operator (the human reading the app), session:<id> (from `harbard ls`), repo:., harness:codex or channel:<name>"
            )
        })?;
    match kind {
        "harness" => {
            // Strict validation here (unlike the session's own harness, which
            // clamps): a note's recipient harness must be a known value.
            // `human` is the person at the console: no session runs under it,
            // but it is the documented way to answer the operator, and the
            // Stop-hook wake text tells agents to use it.
            let v = if value == "claude" {
                "claude-code"
            } else {
                value
            };
            if !HARNESSES.contains(&v) && v != harbard_core::HARNESS_HUMAN {
                return Err(format!(
                    "unknown harness \"{value}\"; expected one of {}, {}",
                    HARNESSES.join(", "),
                    harbard_core::HARNESS_HUMAN
                ));
            }
            Ok(Recipient {
                kind: RecipientKind::Harness,
                value: v.to_string(),
            })
        }
        "session" => Ok(Recipient {
            kind: RecipientKind::Session,
            value: value.to_string(),
        }),
        "repo" => Ok(Recipient {
            kind: RecipientKind::Repo,
            value: canonical_repo_recipient(Path::new(value)),
        }),
        "channel" => {
            // Normalized at the door so `#Incidents`, `incidents` and
            // `Incidents` address one channel, not three.
            let name = harbard_core::membership::normalize(value);
            if !harbard_core::membership::is_valid_name(&name) {
                return Err(format!(
                    "invalid channel name \"{value}\"; use letters, digits, - _ . (max 48)"
                ));
            }
            Ok(Recipient {
                kind: RecipientKind::Channel,
                value: name,
            })
        }
        other => Err(format!(
            "unknown recipient kind \"{other}\"; expected operator, harness, session, repo, or channel"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_shot_print_is_the_sdk_cli_entrypoint_running_print() {
        let print = Some("claude -p --model haiku di hello");
        assert!(one_shot_print(Some("sdk-cli"), print));
        assert!(one_shot_print(
            Some("sdk-cli"),
            Some("claude --print hello")
        ));
        // The harness command is unreadable: the entrypoint decides alone.
        assert!(one_shot_print(Some("sdk-cli"), None));
    }

    #[test]
    fn sessions_someone_can_wake_are_never_one_shot() {
        // Interactive terminal, Desktop, VS Code, SDK hosts and no variable.
        for entrypoint in [Some("cli"), Some("claude-desktop"), Some("claude-vscode")]
            .into_iter()
            .chain([Some("sdk-ts"), Some("remote"), None])
        {
            assert!(!one_shot_print(entrypoint, Some("claude -p hello")));
        }
        // `sdk-cli` without a print flag on the harness: ambiguous, keep watching.
        assert!(!one_shot_print(Some("sdk-cli"), Some("claude")));
        // A streaming session stays open for more turns.
        for streaming in [
            "claude -p --input-format stream-json --output-format stream-json",
            "claude --print --input-format=stream-json",
        ] {
            assert!(!one_shot_print(Some("sdk-cli"), Some(streaming)));
        }
    }

    #[test]
    fn harness_normalization() {
        assert_eq!(normalize_harness(Some("codex".into())), "codex");
        assert_eq!(normalize_harness(Some("claude".into())), "claude-code");
        assert_eq!(normalize_harness(Some("cursor".into())), "cursor");
        assert_eq!(normalize_harness(Some("weird".into())), "other");
        assert_eq!(normalize_harness(None), "other");
    }

    #[test]
    fn harness_detection_precedence() {
        // M1.1c: a Codex marker resolves to `codex`, not `other`.
        assert_eq!(classify_harness(false, true, false, false), Some("codex"));
        assert_eq!(
            classify_harness(true, false, false, false),
            Some("claude-code")
        );
        // You are only ever inside one harness; Claude wins the impossible tie.
        assert_eq!(
            classify_harness(true, true, true, true),
            Some("claude-code")
        );
        // Codex in a Cursor-hosted terminal still wins over Cursor ambient.
        assert_eq!(classify_harness(false, true, true, false), Some("codex"));
        // Cursor Agent shell (CURSOR_CONVERSATION_ID + CURSOR_AGENT).
        assert_eq!(classify_harness(false, false, true, false), Some("cursor"));
        // Antigravity agent shell (ANTIGRAVITY_CONVERSATION_ID + ANTIGRAVITY_AGENT).
        assert_eq!(
            classify_harness(false, false, false, true),
            Some("antigravity")
        );
        // No marker → `other` upstream. CURSOR_VERSION alone is never enough.
        assert_eq!(classify_harness(false, false, false, false), None);
    }

    #[test]
    fn an_antigravity_shell_needs_both_conversation_markers() {
        let id = Some("2af27492-0000-0000-0000-000000000000");
        assert!(antigravity_agent_shell(id, Some("1")));
        // A hook process carries the id without the agent flag.
        assert!(!antigravity_agent_shell(id, None));
        assert!(!antigravity_agent_shell(id, Some("0")));
        assert!(!antigravity_agent_shell(id, Some("false")));
        // The flag without a conversation is not a session either.
        assert!(!antigravity_agent_shell(None, Some("1")));
        assert!(!antigravity_agent_shell(Some("  "), Some("1")));
    }

    #[test]
    fn antigravity_is_a_recognized_harness_and_recipient() {
        assert_eq!(normalize_harness(Some("antigravity".into())), "antigravity");
        assert_eq!(
            parse_recipient("harness:antigravity").unwrap().value,
            "antigravity"
        );
    }

    #[test]
    fn an_antigravity_pre_invocation_is_split_by_its_invocation_number() {
        let call = |n: u64| {
            serde_json::json!({
                "conversationId": "c",
                "hookEventName": "PreInvocation",
                "invocationNum": n,
            })
        };
        assert_eq!(antigravity_turn_start(Some(&call(0))), Some(true));
        assert_eq!(antigravity_turn_start(Some(&call(1))), Some(false));
        assert_eq!(antigravity_turn_start(Some(&call(7))), Some(false));
        // Any other event, or no payload, is not split.
        let start = serde_json::json!({ "hookEventName": "SessionStart" });
        assert_eq!(antigravity_turn_start(Some(&start)), None);
        assert_eq!(antigravity_turn_start(None), None);
    }

    #[test]
    fn a_cursor_payload_under_a_foreign_hook_flag_is_a_cross_hook() {
        // Cursor runs `~/.claude/settings.json` beside its own hook file, so
        // the Claude-declared line arrives carrying Cursor's payload.
        let cursor_payload = serde_json::json!({
            "conversation_id": "c0nv-0000-0000-0000",
            "workspace_roots": ["/repo"],
        });
        assert!(cross_harness_hook("claude-code", Some(&cursor_payload)));
        assert!(cross_harness_hook("claude", Some(&cursor_payload)));
        assert!(cross_harness_hook("codex", Some(&cursor_payload)));
        // Cursor answering its own payload is the normal path.
        assert!(!cross_harness_hook("cursor", Some(&cursor_payload)));
    }

    #[test]
    fn a_conversation_id_alone_is_not_enough_to_disown_a_hook() {
        // `conversation_id` is a plausible field for any harness to adopt, and
        // a false positive silences a real hook rather than a duplicate one.
        // Only `workspace_roots` — sent in place of `cwd` — attributes a
        // payload to Cursor.
        let ambiguous = serde_json::json!({
            "session_id": "sess-0000-0000-0000",
            "conversation_id": "sess-0000-0000-0000",
            "tool_name": "Write",
        });
        assert!(!cross_harness_hook("claude-code", Some(&ambiguous)));
        assert!(!cross_harness_hook("codex", Some(&ambiguous)));
    }

    #[test]
    fn a_payload_we_cannot_attribute_is_left_to_the_declared_harness() {
        // Claude Code's own SessionStart: no Cursor signature, nothing to fix.
        let claude_payload = serde_json::json!({
            "session_id": "sess-0000-0000-0000",
            "cwd": "/repo",
        });
        assert!(!cross_harness_hook("claude-code", Some(&claude_payload)));
        assert!(!cross_harness_hook("claude-code", None));
        // Empty fields are not a signature.
        let hollow = serde_json::json!({ "conversation_id": "", "workspace_roots": [] });
        assert!(!cross_harness_hook("claude-code", Some(&hollow)));
    }

    #[test]
    fn env_prefix_match() {
        // Detection keys off any `CODEX_`-prefixed variable name.
        let vars = || {
            ["PATH", "HOME", "CODEX_SANDBOX", "CODEX_HOME"]
                .into_iter()
                .map(String::from)
        };
        assert!(any_key_has_prefix(vars(), "CODEX_"));
        assert!(!any_key_has_prefix(
            ["PATH", "HOME"].into_iter().map(String::from),
            "CODEX_"
        ));
    }

    #[test]
    fn stable_supervisor_selection_skips_ephemeral_shells() {
        let ancestry = vec![
            (101, "/bin/zsh -lc harbard claim --writer".to_string()),
            (
                102,
                "/opt/codex/vendor/aarch64-apple-darwin/bin/codex -c hooks=true".to_string(),
            ),
            (1, "/sbin/launchd".to_string()),
        ];
        assert_eq!(resolve_supervised_pid("codex", &ancestry), Some(102));
        assert_eq!(resolve_supervised_pid("claude-code", &ancestry), None);
    }

    #[test]
    fn stable_supervisor_selection_supports_all_wired_harnesses() {
        assert_eq!(
            resolve_supervised_pid(
                "claude-code",
                &[(201, "/usr/local/bin/claude --resume abc".to_string())]
            ),
            Some(201)
        );
        assert_eq!(
            resolve_supervised_pid(
                "claude-code",
                &[(
                    202,
                    "node /opt/node_modules/@anthropic-ai/claude-code/cli.js".to_string()
                )]
            ),
            Some(202)
        );
        assert_eq!(
            resolve_supervised_pid(
                "cursor",
                &[(
                    301,
                    "/Applications/Cursor.app/Contents/MacOS/Cursor".to_string()
                )]
            ),
            Some(301)
        );
        // Context `other` still accepts a Cursor ancestor; bare select does not.
        assert_eq!(
            select_harness_ancestor("other", &[(401, "cursor-agent".to_string())]),
            None
        );
        assert_eq!(
            resolve_supervised_pid("other", &[(401, "cursor-agent".to_string())]),
            Some(401)
        );
        // Antigravity, as `ps` shows it: by name (interactive) or by path
        // (`agy -p`). A process that merely mentions agy is not one.
        for command in [
            "agy",
            "/Users/somebody/.local/bin/agy -p hello --print-timeout 150s",
        ] {
            assert_eq!(
                resolve_supervised_pid(
                    "antigravity",
                    &[
                        (
                            501,
                            "/bin/sh -c harbard hello --hook antigravity".to_string()
                        ),
                        (502, command.to_string()),
                    ]
                ),
                Some(502),
                "{command}"
            );
        }
        assert_eq!(
            resolve_supervised_pid("other", &[(601, "/opt/bin/agy".to_string())]),
            Some(601)
        );
        assert_eq!(
            resolve_supervised_pid("antigravity", &[(701, "/usr/bin/legacy agy".to_string())]),
            None
        );
    }

    #[test]
    fn supervised_pid_fails_closed_without_harness_ancestor() {
        let ancestry = vec![
            (101, "/bin/zsh -lc harbard claim --writer".to_string()),
            (1, "/sbin/launchd".to_string()),
        ];
        assert_eq!(resolve_supervised_pid("codex", &ancestry), None);
        assert_eq!(resolve_supervised_pid("other", &ancestry), None);
    }

    #[test]
    fn nested_harness_uses_the_nearest_supervisor_not_the_outer_harness() {
        let ancestry = [
            (101, "/bin/bash -c harbard inbox".into()),
            (102, "/opt/bin/codex".into()),
            (103, "/opt/bin/claude".into()),
        ];
        assert_eq!(nearest_harness_ancestor(&ancestry), Some((102, "codex")));
        assert_eq!(resolve_supervised_pid("other", &ancestry), Some(102));
        assert_eq!(resolve_supervised_pid("claude-code", &ancestry), Some(103));
    }

    #[test]
    fn recipient_parsing() {
        assert!(parse_recipient("harness:codex").is_ok());
        assert!(parse_recipient("session:abc").is_ok());
        assert!(parse_recipient("harness:nope").is_err());
        // The operator's address, as the wake text prescribes it.
        let human = parse_recipient("harness:human").unwrap();
        assert_eq!(human.value, harbard_core::HARNESS_HUMAN);
        // `operator` names the same reader and is stored the same way.
        assert_eq!(parse_recipient("operator").unwrap(), human);
        assert!(parse_recipient("operator:x").is_err());
        assert!(parse_recipient("bogus:x").is_err());
        assert!(parse_recipient("noколон").is_err());
    }

    #[test]
    fn immediate_spawn_parent_wins_over_the_hook_root() {
        assert_eq!(
            select_child_parent(Some("root".into()), Some("immediate".into())).as_deref(),
            Some("immediate")
        );
        assert_eq!(
            select_child_parent(Some("root".into()), None).as_deref(),
            Some("root")
        );
    }

    #[cfg(unix)]
    #[test]
    fn git_root_canonicalizes_when_the_target_file_does_not_exist() {
        use std::os::unix::fs::symlink;

        let root =
            std::env::temp_dir().join(format!("harbard-context-root-{}", std::process::id()));
        let real = root.join("real");
        let alias = root.join("alias");
        std::fs::create_dir_all(real.join("repo/.git")).unwrap();
        symlink(&real, &alias).unwrap();

        let found = git_root(&alias.join("repo/new/file.rs")).unwrap();
        assert_eq!(found, std::fs::canonicalize(real.join("repo")).unwrap());

        let _ = std::fs::remove_dir_all(root);
    }

    /// A unique throwaway directory, cleaned by the caller. Same idiom as
    /// `git_root_canonicalizes_when_the_target_file_does_not_exist` above.
    fn scratch(tag: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "harbard-repo-key-{tag}-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn a_linked_worktree_resolves_to_the_repository_it_belongs_to() {
        // Proves the resolution against real Git, not a hand-built fixture:
        // the identity a `repo:` note is addressed to must be the same string
        // from the primary checkout and from any worktree hanging off it.
        let home = scratch("worktree");
        let primary = home.join("app");
        std::fs::create_dir_all(&primary).unwrap();
        let git = |dir: &Path, args: &[&str]| {
            let ok = std::process::Command::new("git")
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .current_dir(dir)
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_SYSTEM", "/dev/null")
                .output()
                .expect("git runs");
            assert!(ok.status.success(), "git {args:?}: {ok:?}");
        };
        git(&primary, &["init", "-q", "-b", "main"]);
        git(&primary, &["config", "user.email", "t@example.com"]);
        git(&primary, &["config", "user.name", "t"]);
        std::fs::write(primary.join("f"), "x").unwrap();
        git(&primary, &["add", "f"]);
        git(&primary, &["commit", "-qm", "init"]);

        let worktree = home.join("wt");
        git(
            &primary,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "side",
                worktree.to_str().unwrap(),
            ],
        );

        let expected = std::fs::canonicalize(&primary)
            .unwrap()
            .display()
            .to_string();
        assert_eq!(repo_key(&primary).as_deref(), Some(expected.as_str()));
        assert_eq!(
            repo_key(&worktree).as_deref(),
            Some(expected.as_str()),
            "a worktree names the repository it belongs to, not itself"
        );
        assert_eq!(
            canonical_repo_recipient(&worktree),
            canonical_repo_recipient(&primary),
            "both spellings address one audience"
        );
        // The checkout path itself stays distinct: the desk still shows one
        // berth per working tree, and each keeps its own branch.
        assert_ne!(canonical_repo(&worktree), canonical_repo(&primary));

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_path_outside_any_repository_keeps_its_own_name() {
        let home = scratch("loose");
        let plain = home.join("loose");
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(repo_key(&plain), None, "no repository, no repository key");
        assert_eq!(canonical_repo_recipient(&plain), canonical_repo(&plain));

        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn a_bare_repository_does_not_claim_its_parent_directory() {
        // `/srv/repo.git`'s parent is `/srv`, which is somebody else's. The
        // resolver refuses rather than addressing an unrelated directory.
        let home = scratch("bare");
        let bare = home.join("repo.git");
        std::fs::create_dir_all(&bare).unwrap();
        let ok = std::process::Command::new("git")
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .current_dir(&home)
            .args(["init", "-q", "--bare", "repo.git"])
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_SYSTEM", "/dev/null")
            .output()
            .expect("git runs");
        assert!(ok.status.success());
        assert_eq!(repo_key(&bare), None);

        let _ = std::fs::remove_dir_all(&home);
    }
}
