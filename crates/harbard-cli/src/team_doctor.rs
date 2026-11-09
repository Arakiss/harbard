//! The two `harbard doctor` checks behind the team view (study §6.5):
//!
//! - **format**: Claude Code wrote transcripts with `subagents/` in the last
//!   24 hours and the projection recognised no agent in them. The transcript
//!   format is not a contract; this is how a silent change gets noticed.
//! - **serve**: the API the console reads does not answer. Probed when serve
//!   is expected (its LaunchAgent is installed) or named explicitly with
//!   `HARBARD_SERVE_URL`; a machine that never installed serve is not failed
//!   for it.

use harbard_core::Store;
use harbard_core::clock;
use harbard_core::team::{self, TeamRoots};
use serde_json::{Value, json};
use std::time::Duration;

const SERVE_TIMEOUT: Duration = Duration::from_secs(3);

pub struct Check {
    pub ok: bool,
    pub json: Value,
    pub text: String,
}

pub fn format_check() -> Check {
    let roots = TeamRoots::from_env();
    let p = team::probe(&roots, clock::now_ms());
    let ok = !p.format_changed();
    let text = if p.recent_subagent_files == 0 {
        "team view    no Claude Code agent transcripts in the last 24 h".to_string()
    } else if ok {
        format!(
            "team view    {} agent(s) recognised in {} session(s) written in the last 24 h",
            p.recent_agents_seen, p.recent_sessions
        )
    } else {
        format!(
            "team view    FORMAT CHANGED: {} file(s) under subagents/ written in the last 24 h, 0 agents recognised; the transcript format moved",
            p.recent_subagent_files
        )
    };
    Check {
        ok,
        json: json!({
            "ok": ok,
            "root": roots.claude_projects.display().to_string(),
            "recent_subagent_files": p.recent_subagent_files,
            "recent_agents_seen": p.recent_agents_seen,
            "recent_sessions": p.recent_sessions,
        }),
        text,
    }
}

/// Where serve should answer, if anywhere.
fn serve_target() -> Option<(String, &'static str)> {
    if let Some(url) = std::env::var("HARBARD_SERVE_URL")
        .ok()
        .map(|u| u.trim().trim_end_matches('/').to_string())
        .filter(|u| !u.is_empty())
    {
        return Some((url, "HARBARD_SERVE_URL"));
    }
    crate::hub_dx::launchd_serve_plist().is_file().then(|| {
        (
            format!("http://127.0.0.1:{}", crate::serve::DEFAULT_PORT),
            "LaunchAgent",
        )
    })
}

pub fn serve_check(store: &Store) -> Check {
    let Some((url, why)) = serve_target() else {
        return Check {
            ok: true,
            json: json!({ "ok": true, "probed": false }),
            text: "serve        not installed (no LaunchAgent, no HARBARD_SERVE_URL); not probed"
                .into(),
        };
    };
    let agent = ureq::AgentBuilder::new().timeout(SERVE_TIMEOUT).build();
    let mut req = agent.get(&format!("{url}/api/v1/health"));
    if let Some(token) = crate::hub_dx::read_hub_token(store) {
        req = req.set("Authorization", &format!("Bearer {token}"));
    }
    let (ok, status, detail) = match req.call() {
        Ok(resp) => (resp.status() == 200, Some(resp.status()), None),
        Err(ureq::Error::Status(code, _)) => (false, Some(code), None),
        Err(e) => (false, None, Some(e.kind().to_string())),
    };
    let text = match (ok, status) {
        (true, _) => format!("serve        answering at {url} ({why})"),
        (false, Some(code)) => format!("serve        NOT HEALTHY: {url} answered HTTP {code}"),
        (false, None) => format!(
            "serve        NOT ANSWERING: {url} ({})",
            detail.as_deref().unwrap_or("no response")
        ),
    };
    Check {
        ok,
        json: json!({
            "ok": ok,
            "probed": true,
            "url": url,
            "expected_by": why,
            "status": status,
            "error": detail,
        }),
        text,
    }
}

/// Why the rm guard matters, said wherever the check is not green.
const RM_GUARD_WHY: &str = "Claude Code asks before an rm it cannot verify (a variable path, a glob), even in bypassPermissions, and an unattended agent then waits on a prompt nobody sees. Wire a PreToolUse Bash hook that refuses it and says how to write it instead (a literal path or \"${VAR:?}\"), so no prompt is raised.";

/// Whether a guard for Claude Code's rm safety prompt is wired: a
/// PreToolUse hook on Bash whose command names an `rm-guard` script that
/// exists. Advisory: it is outside harbard, so it never fails `doctor`.
pub fn rm_guard_check() -> Check {
    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let claude = home.join(".claude");
    let status = if !claude.is_dir() {
        "no_claude_code"
    } else {
        let mut commands: Vec<String> = Vec::new();
        for name in ["settings.json", "settings.local.json"] {
            let Some(v) = std::fs::read_to_string(claude.join(name))
                .ok()
                .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
            else {
                continue;
            };
            for group in v
                .pointer("/hooks/PreToolUse")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                let matcher = group.get("matcher").and_then(Value::as_str).unwrap_or("");
                if !(matcher.is_empty() || matcher == "*" || matcher.contains("Bash")) {
                    continue;
                }
                for h in group
                    .get("hooks")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    if let Some(c) = h.get("command").and_then(Value::as_str) {
                        commands.push(c.to_string());
                    }
                }
            }
        }
        let guard = |w: &str| w.contains("rm-guard") || w.contains("rm_guard");
        let scripts: Vec<std::path::PathBuf> = commands
            .iter()
            .flat_map(|c| c.split_whitespace().map(str::to_string).collect::<Vec<_>>())
            .filter(|w| guard(w))
            .map(|w| {
                let w = w.trim_matches(['"', '\'']);
                w.strip_prefix("~/")
                    .or_else(|| w.strip_prefix("$HOME/"))
                    .map_or_else(|| std::path::PathBuf::from(w), |rest| home.join(rest))
            })
            .collect();
        if scripts.is_empty() {
            "not_wired"
        } else if scripts.iter().any(|p| p.is_file()) {
            "wired"
        } else {
            "script_missing"
        }
    };
    let wired = status == "wired";
    let text = match status {
        "wired" => "rm guard     wired: Claude Code's rm prompt is answered by a hook".to_string(),
        "no_claude_code" => "rm guard     no ~/.claude here; not applicable".to_string(),
        "script_missing" => format!(
            "rm guard     WIRED TO A MISSING SCRIPT: the PreToolUse hook names an rm-guard that is not on disk. {RM_GUARD_WHY}"
        ),
        _ => format!("rm guard     not wired. {RM_GUARD_WHY}"),
    };
    Check {
        ok: wired || status == "no_claude_code",
        json: json!({
            "wired": wired,
            "status": status,
            "why": (!wired).then_some(RM_GUARD_WHY),
        }),
        text,
    }
}
