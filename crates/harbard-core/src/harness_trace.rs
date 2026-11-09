//! Read-only projection of harness-native transcripts (Claude / Codex / Cursor).
//!
//! These files live on disk (sometimes behind a symlink). They are **never**
//! written to the bus: CoT stays out of `events/`, `brief`, and SSE. Serve
//! exposes a pull-only snapshot for the console detail panel.

use crate::query::SessionView;
use serde::Serialize;
use serde_json::Value;
use std::fs::{self, File};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

/// Hard cap on lines returned in one response.
pub const TRACE_LIMIT_MAX: usize = 200;
/// Default when the caller omits `limit`.
pub const TRACE_LIMIT_DEFAULT: usize = 40;
/// Cap per extracted text blob so a single monster thinking block cannot blow
/// the HTTP response (bus line limit is a different contract; this is UI).
const TEXT_CAP: usize = 2_000;

/// One projected line from a harness transcript.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TraceLine {
    pub kind: &'static str,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t: Option<String>,
}

/// Result of resolving and reading a session's harness transcript.
#[derive(Debug, Clone, Serialize)]
pub struct SessionTrace {
    pub session: String,
    pub harness: String,
    pub found: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub limit: usize,
    pub lines: Vec<TraceLine>,
}

/// Resolve home roots (overridable in tests).
#[derive(Debug, Clone)]
pub struct TraceRoots {
    pub home: PathBuf,
    pub claude_projects: PathBuf,
    pub codex_sessions: PathBuf,
    pub codex_archived: PathBuf,
    pub cursor_projects: PathBuf,
}

impl TraceRoots {
    pub fn from_env() -> Self {
        let home = dirs_home();
        Self {
            claude_projects: home.join(".claude").join("projects"),
            codex_sessions: home.join(".codex").join("sessions"),
            codex_archived: home.join(".codex").join("archived_sessions"),
            cursor_projects: home.join(".cursor").join("projects"),
            home,
        }
    }
}

fn dirs_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Load the last `limit` thinking/status lines for a session view.
pub fn load_trace(view: &SessionView, limit: usize, roots: &TraceRoots) -> SessionTrace {
    let limit = limit.clamp(1, TRACE_LIMIT_MAX);
    let path = resolve_path(view, roots);
    let Some(path) = path else {
        return SessionTrace {
            session: view.session.clone(),
            harness: view.harness.clone(),
            found: false,
            path: None,
            limit,
            lines: vec![],
        };
    };
    let lines = extract_from_file(&path, &view.harness, limit).unwrap_or_default();
    SessionTrace {
        session: view.session.clone(),
        harness: view.harness.clone(),
        found: true,
        path: Some(path.display().to_string()),
        limit,
        lines,
    }
}

fn resolve_path(view: &SessionView, roots: &TraceRoots) -> Option<PathBuf> {
    let id = view.session.as_str();
    if id.is_empty() {
        return None;
    }
    match view.harness.as_str() {
        "claude-code" | "claude" => resolve_claude(view, roots),
        "codex" => resolve_codex(id, roots),
        "cursor" => resolve_cursor(view, roots),
        _ => None,
    }
}

fn encode_claude_cwd(cwd: &str) -> String {
    cwd.replace(['/', '.'], "-")
}

fn encode_cursor_cwd(cwd: &str) -> String {
    cwd.trim_start_matches('/').replace('/', "-")
}

fn resolve_claude(view: &SessionView, roots: &TraceRoots) -> Option<PathBuf> {
    let id = &view.session;
    let root = &roots.claude_projects;
    if !root.is_dir() || !crate::reach::readable(root) {
        return None;
    }
    // Herd child: …/{parent}/subagents/agent-{id}.jsonl
    if let Some(parent) = view.parent.as_deref() {
        let encoded = encode_claude_cwd(&view.cwd);
        let p = root
            .join(&encoded)
            .join(parent)
            .join("subagents")
            .join(format!("agent-{id}.jsonl"));
        if p.is_file() {
            return Some(p);
        }
        if let Some(p) = find_named(root, &format!("agent-{id}.jsonl"), 4) {
            return Some(p);
        }
    }
    // Main session: …/{encode(cwd)}/{sessionId}.jsonl
    let encoded = encode_claude_cwd(&view.cwd);
    let direct = root.join(&encoded).join(format!("{id}.jsonl"));
    if direct.is_file() {
        return Some(direct);
    }
    find_named(root, &format!("{id}.jsonl"), 3)
}

fn resolve_codex(id: &str, roots: &TraceRoots) -> Option<PathBuf> {
    let needle = format!("-{id}.jsonl");
    for root in [&roots.codex_sessions, &roots.codex_archived] {
        if !root.is_dir() || !crate::reach::readable(root) {
            continue;
        }
        if let Some(p) = find_suffix(root, &needle, 5) {
            return Some(p);
        }
    }
    None
}

fn resolve_cursor(view: &SessionView, roots: &TraceRoots) -> Option<PathBuf> {
    let id = &view.session;
    let root = &roots.cursor_projects;
    if !root.is_dir() {
        return None;
    }
    let encoded = encode_cursor_cwd(&view.cwd);
    let direct = root
        .join(&encoded)
        .join("agent-transcripts")
        .join(id)
        .join(format!("{id}.jsonl"));
    if direct.is_file() {
        return Some(direct);
    }
    // Subagent under parent
    if let Some(parent) = view.parent.as_deref() {
        let p = root
            .join(&encoded)
            .join("agent-transcripts")
            .join(parent)
            .join("subagents")
            .join(format!("{id}.jsonl"));
        if p.is_file() {
            return Some(p);
        }
    }
    find_named(root, &format!("{id}.jsonl"), 5)
}

fn find_named(root: &Path, filename: &str, max_depth: usize) -> Option<PathBuf> {
    find_walk(root, max_depth, &mut |p| {
        p.file_name().and_then(|n| n.to_str()) == Some(filename)
    })
}

fn find_suffix(root: &Path, suffix: &str, max_depth: usize) -> Option<PathBuf> {
    find_walk(root, max_depth, &mut |p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(suffix))
    })
}

fn find_walk(
    root: &Path,
    max_depth: usize,
    pred: &mut dyn FnMut(&Path) -> bool,
) -> Option<PathBuf> {
    fn walk(
        dir: &Path,
        depth: usize,
        max_depth: usize,
        pred: &mut dyn FnMut(&Path) -> bool,
    ) -> Option<PathBuf> {
        if depth > max_depth {
            return None;
        }
        let entries = fs::read_dir(dir).ok()?;
        let mut dirs = Vec::new();
        for ent in entries.flatten() {
            let p = ent.path();
            if p.is_file() && pred(&p) {
                return Some(p);
            }
            if p.is_dir() {
                dirs.push(p);
            }
        }
        for d in dirs {
            if let Some(hit) = walk(&d, depth + 1, max_depth, pred) {
                return Some(hit);
            }
        }
        None
    }
    walk(root, 0, max_depth, pred)
}

fn extract_from_file(path: &Path, harness: &str, limit: usize) -> std::io::Result<Vec<TraceLine>> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut acc: Vec<TraceLine> = Vec::new();
    for line in reader.lines() {
        let Ok(line) = line else { continue };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match harness {
            "claude-code" | "claude" => push_claude(&v, &mut acc),
            "codex" => push_codex(&v, &mut acc),
            "cursor" => push_cursor(&v, &mut acc),
            _ => {}
        }
        // Keep a rolling window so we don't hold the whole file.
        if acc.len() > limit.saturating_mul(4) {
            let drop = acc.len() - limit.saturating_mul(2);
            acc.drain(0..drop);
        }
    }
    if acc.len() > limit {
        acc = acc.split_off(acc.len() - limit);
    }
    Ok(acc)
}

fn cap_text(s: &str) -> String {
    let mut out = s.trim().to_string();
    if out.chars().count() > TEXT_CAP {
        out = out.chars().take(TEXT_CAP).collect();
        out.push('…');
    }
    out
}

fn push_line(acc: &mut Vec<TraceLine>, kind: &'static str, text: &str, t: Option<String>) {
    let text = cap_text(text);
    if text.is_empty() {
        return;
    }
    acc.push(TraceLine { kind, text, t });
}

fn push_claude(v: &Value, acc: &mut Vec<TraceLine>) {
    let t = v
        .get("timestamp")
        .and_then(|x| x.as_str())
        .map(str::to_string);
    let typ = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    if typ != "assistant" {
        // Tool use / progress sometimes appears as user tool_result — skip.
        if (typ == "progress" || typ == "system")
            && let Some(msg) = v.get("message").and_then(|m| m.as_str())
        {
            push_line(acc, "status", msg, t);
        }
        return;
    }
    let Some(content) = v.pointer("/message/content").and_then(|c| c.as_array()) else {
        return;
    };
    for block in content {
        let btype = block.get("type").and_then(|x| x.as_str()).unwrap_or("");
        match btype {
            "thinking" => {
                if let Some(text) = block.get("thinking").and_then(|x| x.as_str()) {
                    push_line(acc, "thinking", text, t.clone());
                }
            }
            "text" => {
                if let Some(text) = block.get("text").and_then(|x| x.as_str()) {
                    // Short assistant texts as status pulse; long ones are answers.
                    if text.chars().count() <= 240 {
                        push_line(acc, "status", text, t.clone());
                    }
                }
            }
            "tool_use" => {
                let name = block.get("name").and_then(|x| x.as_str()).unwrap_or("tool");
                push_line(acc, "status", &format!("tool · {name}"), t.clone());
            }
            _ => {}
        }
    }
}

fn push_codex(v: &Value, acc: &mut Vec<TraceLine>) {
    let typ = v.get("type").and_then(|x| x.as_str()).unwrap_or("");
    let payload = v.get("payload").unwrap_or(v);
    let t = v
        .get("timestamp")
        .or_else(|| payload.get("timestamp"))
        .and_then(|x| x.as_str())
        .map(str::to_string);
    let ptype = payload.get("type").and_then(|x| x.as_str()).unwrap_or("");

    if typ == "event_msg" && ptype == "agent_reasoning" {
        if let Some(text) = payload.get("text").and_then(|x| x.as_str()) {
            push_line(acc, "thinking", text, t);
        }
        return;
    }
    if typ == "response_item" && ptype == "reasoning" {
        if let Some(summary) = payload.get("summary").and_then(|x| x.as_array()) {
            for item in summary {
                if let Some(text) = item.get("text").and_then(|x| x.as_str()) {
                    push_line(acc, "thinking", text, t.clone());
                }
            }
        }
        return;
    }
    if typ == "event_msg"
        && ptype == "agent_message"
        && let Some(msg) = payload.get("message").and_then(|x| x.as_str())
        && msg.chars().count() <= 240
    {
        push_line(acc, "status", msg, t);
    }
}

fn push_cursor(v: &Value, acc: &mut Vec<TraceLine>) {
    let role = v.get("role").and_then(|x| x.as_str()).unwrap_or("");
    if role != "assistant" {
        return;
    }
    let t = v
        .get("timestamp")
        .and_then(|x| x.as_str())
        .map(str::to_string);
    let Some(content) = v
        .get("message")
        .and_then(|m| m.get("content"))
        .or_else(|| v.get("content"))
        .and_then(|c| c.as_array())
    else {
        return;
    };
    for block in content {
        let btype = block.get("type").and_then(|x| x.as_str()).unwrap_or("");
        match btype {
            "text" => {
                if let Some(text) = block.get("text").and_then(|x| x.as_str())
                    && text.chars().count() <= 240
                {
                    push_line(acc, "status", text, t.clone());
                }
            }
            "tool_use" => {
                let name = block.get("name").and_then(|x| x.as_str()).unwrap_or("tool");
                push_line(acc, "status", &format!("tool · {name}"), t.clone());
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventKind, Role};
    use crate::query::{Liveness, SessionView};
    use crate::test_util::TempHome;
    use std::io::Write as _;

    fn view(session: &str, harness: &str, cwd: &str, parent: Option<&str>) -> SessionView {
        SessionView {
            session: session.into(),
            harness: harness.into(),
            host: "h".into(),
            cwd: cwd.into(),
            repo: None,
            repo_key: None,
            branch: None,
            role: if parent.is_some() {
                Role::Herd
            } else {
                Role::Main
            },
            parent: parent.map(str::to_string),
            root: parent.unwrap_or(session).into(),
            depth: u32::from(parent.is_some()),
            descendants: 0,
            lifecycle: crate::query::LifecycleState::Active,
            event_count: 1,
            intent: None,
            liveness: Liveness::Live,
            idle: false,
            last_signal_ms: 1,
            last_ms: 1,
            first_ms: 1,
            identity: crate::topology::infer_identity_confidence(session, "h", harness, parent),
            last_kind: EventKind::Hello,
            ending: false,
            saw_hello: true,
            saw_bye: false,
        }
    }

    #[test]
    fn extracts_claude_thinking_and_tool_status() {
        let home = TempHome::new();
        let projects = home.path().join("claude/projects");
        let cwd = "/tmp/demo";
        let enc = encode_claude_cwd(cwd);
        let dir = projects.join(&enc);
        fs::create_dir_all(&dir).unwrap();
        let sid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let path = dir.join(format!("{sid}.jsonl"));
        let mut f = File::create(&path).unwrap();
        f.write_all(
            br#"{"type":"assistant","timestamp":"t1","message":{"content":[{"type":"thinking","thinking":"consider the bus"},{"type":"tool_use","name":"Bash"}]}}
{"type":"assistant","timestamp":"t2","message":{"content":[{"type":"text","text":"short status"}]}}
"#,
        )
        .unwrap();

        let roots = TraceRoots {
            home: home.path().into(),
            claude_projects: projects,
            codex_sessions: home.path().join("codex/sessions"),
            codex_archived: home.path().join("codex/archived"),
            cursor_projects: home.path().join("cursor/projects"),
        };
        let tr = load_trace(&view(sid, "claude-code", cwd, None), 40, &roots);
        assert!(tr.found);
        assert_eq!(tr.lines.len(), 3);
        assert_eq!(tr.lines[0].kind, "thinking");
        assert!(tr.lines[0].text.contains("bus"));
        assert_eq!(tr.lines[1].kind, "status");
        assert!(tr.lines[1].text.contains("Bash"));
        assert_eq!(tr.lines[2].text, "short status");
    }

    #[test]
    fn missing_file_is_found_false() {
        let home = TempHome::new();
        let roots = TraceRoots {
            home: home.path().into(),
            claude_projects: home.path().join("missing"),
            codex_sessions: home.path().join("missing"),
            codex_archived: home.path().join("missing"),
            cursor_projects: home.path().join("missing"),
        };
        let tr = load_trace(&view("nope", "claude-code", "/tmp/x", None), 10, &roots);
        assert!(!tr.found);
        assert!(tr.lines.is_empty());
    }
}
