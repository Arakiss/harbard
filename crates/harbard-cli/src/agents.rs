//! `harbard agents` and `harbard owed`: a lead's view of its own team, and
//! of everything it owes, without opening the console.
//!
//! Both read the workspace projection serve hands the console (the team
//! transcripts plus the bus), so the terminal and the screen agree. The
//! read is local: it never calls serve.

use crate::cli::Common;
use crate::context::Context;
use crate::serve::{team_route, workspace_route};
use harbard_core::workspace::{Agent, Message, TO_AGENT_FIELD, WorkspaceView};
use harbard_core::{Event, Store, Window, clock};
use serde::Serialize;
use std::io::Write;

/// Characters of a message line kept in a row.
const LINE_CHARS: usize = 140;

/// One agent as `harbard agents` prints it.
#[derive(Debug, Clone, Serialize)]
pub struct AgentLine {
    pub key: String,
    /// The workshop name the console shows.
    pub name: String,
    /// The name the lead launched it with (its SendMessage address).
    pub own_name: Option<String>,
    pub role: Option<String>,
    pub team: Option<String>,
    /// `<role> · <team>`, as the console writes it under the name.
    pub role_line: Option<String>,
    pub kind: &'static str,
    pub lead_session: String,
    /// The console face: `running`, `blocked`, `finished`…
    pub state: &'static str,
    pub detail: Option<String>,
    pub since_ms: u64,
    pub dormant: Option<&'static str>,
    pub last_line: Option<String>,
    /// `agent`, `lead`, `peer` or `operator`.
    pub last_from: Option<&'static str>,
    pub last_ms: Option<u64>,
    /// Its newest DONE is newer than anything the lead sent it.
    pub done_waiting: bool,
    pub done_ms: Option<u64>,
}

impl AgentLine {
    /// Where SendMessage reaches it.
    fn address(&self) -> &str {
        self.own_name.as_deref().unwrap_or(&self.name)
    }
}

/// One line of a note for `owed` and `agents`: the first, bounded, without
/// any hidden code point.
fn clip(text: &str) -> String {
    let text = harbard_core::brief::visible_text(text);
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() <= LINE_CHARS {
        return line.to_string();
    }
    let mut out: String = line.chars().take(LINE_CHARS - 1).collect();
    out.push('…');
    out
}

fn line_of(a: &Agent, messages: &[&Message]) -> AgentLine {
    let last = a
        .last_message
        .as_ref()
        .and_then(|id| messages.iter().find(|m| &m.id == id));
    let done = messages
        .iter()
        .filter(|m| m.from == "agent" && m.label == Some("DONE"))
        .max_by_key(|m| m.ts_ms);
    let done_waiting = done.is_some_and(|d| {
        !messages
            .iter()
            .any(|m| m.from == "lead" && m.ts_ms > d.ts_ms)
    });
    let role_line = a.role_line.clone().or_else(|| match (&a.role, &a.team) {
        (Some(r), Some(t)) => Some(format!("{r} · {t}")),
        (Some(r), None) => Some(r.clone()),
        (None, Some(t)) => Some(t.clone()),
        (None, None) => None,
    });
    AgentLine {
        key: a.key.clone(),
        name: a.name.clone(),
        own_name: a.own_name.clone(),
        role: a.role.clone(),
        team: a.team.clone(),
        role_line,
        kind: a.kind,
        lead_session: a.lead_session.clone(),
        state: a.activity.state,
        detail: a.activity.detail.clone(),
        since_ms: a.activity.since_ms,
        dormant: a.dormant,
        last_line: last.map(|m| clip(&m.text)),
        last_from: last.map(|m| m.from),
        last_ms: last.map(|m| m.ts_ms),
        done_waiting,
        done_ms: done.map(|m| m.ts_ms),
    }
}

/// The workspace projection, read here rather than asked of serve.
pub(crate) fn workspace(store: &Store, common: &Common, now: u64) -> (WorkspaceView, Vec<Event>) {
    let read = store.read(Window::All);
    let team = team_route::project(now, harbard_core::team::WINDOW_DEFAULT_MS);
    let view = workspace_route::view(store, &read.events, crate::stale_secs(common), &team, now);
    (view, read.events)
}

/// The agents of `lead` (every lead's with `None`), newest activity first.
fn lines(view: &WorkspaceView, lead: Option<&str>) -> Vec<AgentLine> {
    let mut out: Vec<AgentLine> = view
        .agents
        .iter()
        .filter(|a| a.kind != "session")
        .filter(|a| lead.is_none_or(|l| a.lead_session == l))
        .map(|a| {
            let messages: Vec<&Message> =
                view.messages.iter().filter(|m| m.agent == a.key).collect();
            line_of(a, &messages)
        })
        .collect();
    out.sort_by_key(|l| std::cmp::Reverse(l.since_ms));
    out
}

fn render_agents(rows: &[AgentLine], now: u64, all: bool) -> String {
    if rows.is_empty() {
        return if all {
            "No agents in the last day.\n".to_string()
        } else {
            "No agents of this session in the last day (harbard agents --all lists every lead's).\n"
                .to_string()
        };
    }
    let mut out = String::new();
    for r in rows {
        let state = match &r.detail {
            Some(d) => format!("{} ({d})", r.state),
            None => r.state.to_string(),
        };
        out.push_str(&format!(
            "{}{} · {} · {state} · {}{}\n",
            r.name,
            r.role_line
                .as_deref()
                .map(|l| format!(" ({l})"))
                .unwrap_or_default(),
            r.own_name.as_deref().unwrap_or(&r.key),
            clock::relative(now, r.since_ms),
            if r.done_waiting {
                " · DONE waiting for you"
            } else {
                ""
            },
        ));
        if let Some(line) = &r.last_line {
            out.push_str(&format!("    {}: {line}\n", r.last_from.unwrap_or("agent")));
        }
    }
    out
}

pub fn cmd_agents(
    store: &Store,
    common: &Common,
    all: bool,
    watch: bool,
    interval_secs: u64,
    frames: Option<u64>,
) -> i32 {
    let ctx = Context::resolve(common);
    let lead = (!all && ctx.session_is_stable).then(|| ctx.session.clone());
    let mut frame = 0u64;
    loop {
        let now = clock::now_ms();
        let (view, _) = workspace(store, common, now);
        let rows = lines(&view, lead.as_deref());
        let mut stdout = std::io::stdout().lock();
        if common.json {
            let v = serde_json::json!({
                "now_ms": now,
                "lead": lead,
                "agents": rows,
            });
            let text = if watch {
                serde_json::to_string(&v)
            } else {
                serde_json::to_string_pretty(&v)
            };
            let _ = writeln!(stdout, "{}", text.unwrap_or_default());
        } else {
            if watch {
                // Clear the screen and home the cursor, as `watch(1)` does.
                let _ = write!(stdout, "\x1b[2J\x1b[H");
            }
            let _ = write!(
                stdout,
                "{}",
                render_agents(&rows, now, all || lead.is_none())
            );
        }
        let _ = stdout.flush();
        frame += 1;
        if !watch || frames.is_some_and(|n| frame >= n) {
            return 0;
        }
        std::thread::sleep(std::time::Duration::from_secs(interval_secs));
    }
}

/// One thing the lead owes.
#[derive(Debug, Clone, Serialize)]
pub struct OwedItem {
    /// `answer`, `relay`, `blocked`, `done` or `stalled`.
    pub kind: &'static str,
    pub rank: u8,
    pub since_ms: u64,
    /// The note's id, for notes.
    pub id: Option<String>,
    /// `the operator`, or the session's callsign.
    pub from: Option<String>,
    /// The agent concerned (its SendMessage address).
    pub agent: Option<String>,
    pub text: String,
    /// The exact command that settles it.
    pub command: String,
}

/// The agent keys a note's `to_agent:` lines name.
pub(crate) fn to_agents(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|l| l.trim().strip_prefix(TO_AGENT_FIELD))
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .collect()
}

/// Where an answer owed sits in `harbard owed`, and so which one
/// `harbard reply last` takes: the operator's first, then the oldest.
pub(crate) fn answer_order(note: &Event) -> (u8, u64) {
    let operator = note.is_operator() || note.harness == harbard_core::HARNESS_HUMAN;
    (
        if operator { 0 } else { 3 },
        harbard_core::query::event_ms(note),
    )
}

fn send_message(to: &str, what: &str) -> String {
    format!("SendMessage({{to: \"{to}\", message: \"{what}\"}})")
}

fn owed_items(
    store: &Store,
    ctx: &Context,
    view: &WorkspaceView,
    events: &[Event],
    now: u64,
) -> Vec<OwedItem> {
    let mine = lines(view, Some(&ctx.session));
    let channels = crate::my_channels(store, ctx);
    let placement = harbard_core::workspaces::Placement::load(store);
    let gate = placement.gate(ctx.repo_key.as_deref().or(ctx.repo.as_deref()));
    let reader = harbard_core::Reader {
        session: &ctx.session,
        harness: &ctx.harness,
        repo: ctx.repo.as_deref(),
        repo_key: ctx.repo_key.as_deref().or(ctx.repo.as_deref()),
        channels: &channels,
        workspace: &gate,
    };
    let aliases = harbard_core::alias::read(store);
    let mut items: Vec<OwedItem> = Vec::new();
    for note in harbard_core::status::owed_notes(events, &reader, now) {
        let operator = note.is_operator() || note.harness == harbard_core::HARNESS_HUMAN;
        let from = if operator {
            "the operator".to_string()
        } else {
            harbard_core::alias::get(&aliases, &note.session)
                .map(|a| harbard_core::alias::display(&a))
                .unwrap_or_else(|| crate::short(&note.session))
        };
        let body = harbard_core::workspace::without_fields(&note.text);
        let since_ms = harbard_core::query::event_ms(note);
        let targets = to_agents(&note.text);
        if !targets.is_empty() {
            let names: Vec<String> = targets
                .iter()
                .map(|k| {
                    mine.iter()
                        .find(|a| &a.key == k)
                        .map(|a| a.address().to_string())
                        .unwrap_or_else(|| k.clone())
                })
                .collect();
            let forward: Vec<String> = names
                .iter()
                .map(|n| send_message(n, "<the note, verbatim>"))
                .collect();
            items.push(OwedItem {
                kind: "relay",
                rank: 1,
                since_ms,
                id: Some(note.id.clone()),
                from: Some(from),
                agent: Some(names.join(", ")),
                text: clip(&body),
                command: format!(
                    "{}, then harbard relay --sent {}; its answer: harbard reply {} --agent {} \"…\"",
                    forward.join(" and "),
                    note.id,
                    note.id,
                    names.first().map_or("<name>", String::as_str)
                ),
            });
            continue;
        }
        let (rank, since_ms) = answer_order(note);
        items.push(OwedItem {
            kind: "answer",
            rank,
            since_ms,
            id: Some(note.id.clone()),
            from: Some(from),
            agent: None,
            text: clip(&body),
            command: format!("harbard reply {} \"…\"", note.id),
        });
    }
    for a in &mine {
        let (kind, rank, what) = match a.state {
            "blocked" => ("blocked", 2, "<the answer it is blocked on>"),
            _ if a.done_waiting => ("done", 4, "<the next step, or that it can stop>"),
            "asleep" => ("stalled", 5, "<ask how it is going>"),
            _ => continue,
        };
        let text = if kind == "done" {
            view.messages
                .iter()
                .filter(|m| m.agent == a.key && m.from == "agent" && m.label == Some("DONE"))
                .max_by_key(|m| m.ts_ms)
                .map(|m| clip(&m.text))
        } else {
            a.last_line.clone()
        };
        items.push(OwedItem {
            kind,
            rank,
            since_ms: if kind == "done" {
                a.done_ms.unwrap_or(a.since_ms)
            } else {
                a.since_ms
            },
            id: None,
            from: None,
            agent: Some(a.address().to_string()),
            text: text.unwrap_or_default(),
            command: send_message(a.address(), what),
        });
    }
    items.extend(held_items(store, ctx, events, now));
    items.sort_by_key(|i| (i.rank, i.since_ms));
    items
}

/// This session's notes to the operator that their coordination state holds
/// back (`harbard_core::hold`), with what can still get one through.
fn held_items(store: &Store, ctx: &Context, events: &[Event], now: u64) -> Vec<OwedItem> {
    use harbard_core::hold::{self, Hold};
    let profile = harbard_core::profile::Profile::load(store.root());
    let state = profile.state_at(now);
    let label = harbard_core::profile::state_label(state);
    hold::pending(store.root(), events, &profile, now)
        .into_iter()
        .filter(|item| item.session == ctx.session)
        .map(|item| {
            let command = match hold::hold_for(state) {
                Hold::NonBlocking => format!(
                    "harbard note --kind warning --to harness:human --ref {} \"…\" — only if it cannot wait for their digest ({label})",
                    item.id
                ),
                _ => format!(
                    "nothing to run: {label} queues every note, and it reaches them in their digest"
                ),
            };
            OwedItem {
                kind: "held",
                rank: 6,
                since_ms: item.ts_ms,
                id: Some(item.id),
                from: None,
                agent: None,
                text: clip(&item.text),
                command,
            }
        })
        .collect()
}

/// An agent a relayed note is for.
#[derive(Debug, Clone, Serialize)]
pub struct RelayAgent {
    pub key: String,
    /// The workshop name the console shows; the answer goes back under it.
    pub name: String,
    /// The SendMessage address.
    pub address: String,
    pub send: RelaySend,
}

#[derive(Debug, Clone, Serialize)]
pub struct RelaySend {
    pub to: String,
    pub message: String,
}

/// A `to_agent` note this lead has not relayed yet.
#[derive(Debug, Clone, Serialize)]
pub struct RelayItem {
    pub id: String,
    pub from: String,
    pub since_ms: u64,
    /// The note as it is forwarded: verbatim, without its field lines.
    pub text: String,
    pub agents: Vec<RelayAgent>,
    /// Run after sending.
    pub sent: String,
    /// Run when the agent has answered.
    pub answer: String,
}

/// The agent `key` as the view knows it: (display name, SendMessage address).
/// The shown name and SendMessage address of `lead`'s agent `key`. An
/// agent of another lead is not this lead's to name or address: its key
/// stands for both.
pub(crate) fn agent_names(view: &WorkspaceView, lead: &str, key: &str) -> (String, String) {
    match view
        .agents
        .iter()
        .find(|a| a.key == key && a.kind != "session" && a.lead_session == lead)
    {
        Some(a) => (
            a.name.clone(),
            a.own_name.clone().unwrap_or_else(|| a.name.clone()),
        ),
        None => (key.to_string(), key.to_string()),
    }
}

/// Owed notes with `to_agent:` lines: relayed ones are acked, so they are
/// no longer owed and drop out.
fn pending_relays(
    store: &Store,
    ctx: &Context,
    view: &WorkspaceView,
    events: &[Event],
    now: u64,
) -> Vec<RelayItem> {
    let channels = crate::my_channels(store, ctx);
    let placement = harbard_core::workspaces::Placement::load(store);
    let gate = placement.gate(ctx.repo_key.as_deref().or(ctx.repo.as_deref()));
    let reader = harbard_core::Reader {
        session: &ctx.session,
        harness: &ctx.harness,
        repo: ctx.repo.as_deref(),
        repo_key: ctx.repo_key.as_deref().or(ctx.repo.as_deref()),
        channels: &channels,
        workspace: &gate,
    };
    let aliases = harbard_core::alias::read(store);
    harbard_core::status::owed_notes(events, &reader, now)
        .into_iter()
        .filter_map(|note| {
            let keys = to_agents(&note.text);
            if keys.is_empty() {
                return None;
            }
            // Forwarded verbatim to an agent: no hidden code point.
            let text = harbard_core::brief::visible_text(
                harbard_core::workspace::without_fields(&note.text).trim_end(),
            );
            let from = if note.is_operator() || note.harness == harbard_core::HARNESS_HUMAN {
                "the operator".to_string()
            } else {
                harbard_core::alias::get(&aliases, &note.session)
                    .map(|a| harbard_core::alias::display(&a))
                    .unwrap_or_else(|| crate::short(&note.session))
            };
            let agents = keys
                .iter()
                .map(|key| {
                    let (name, address) = agent_names(view, &ctx.session, key);
                    RelayAgent {
                        key: key.clone(),
                        name,
                        send: RelaySend {
                            to: address.clone(),
                            message: text.clone(),
                        },
                        address,
                    }
                })
                .collect();
            Some(RelayItem {
                id: note.id.clone(),
                from,
                since_ms: harbard_core::query::event_ms(note),
                text,
                agents,
                sent: format!("harbard relay --sent {}", note.id),
                answer: format!(
                    "harbard reply {} --agent {} \"<its answer>\"",
                    note.id,
                    keys.first().map_or_else(
                        || "<name>".to_string(),
                        |k| agent_names(view, &ctx.session, k).0
                    )
                ),
            })
        })
        .collect()
}

pub fn cmd_relay(
    store: &Store,
    common: &Common,
    sent: Option<&str>,
    limit: usize,
    now: u64,
) -> i32 {
    let ctx = Context::resolve(common);
    if !ctx.session_is_stable {
        return crate::fail(
            "relay needs to know who you are — pass --session <id> (your harness's session id)",
        );
    }
    if let Some(target) = sent {
        let read = store.read(Window::All);
        let note =
            match harbard_core::id::event(&read.events, target, harbard_core::EventKind::Note) {
                Ok(note) => note.clone(),
                Err(harbard_core::id::ResolveIdError::Unknown) => {
                    return crate::fail(format!("no note id starts with \"{target}\""));
                }
                Err(harbard_core::id::ResolveIdError::Ambiguous(n)) => {
                    return crate::fail(format!(
                        "note id \"{target}\" matches {n} notes — use the full 26-character id"
                    ));
                }
            };
        if to_agents(&note.text).is_empty() {
            return crate::fail(format!(
                "note {} has no to_agent line, so it is not for an agent of yours — answer it with harbard reply {} \"…\"",
                note.id, note.id
            ));
        }
        // Relayed is "I have it": the ack tells the operator their note reached
        // the agent, and the lead no longer owes it while the agent works.
        return crate::cmd_ack(store, common, &note.id, None, now);
    }
    let (view, events) = workspace(store, common, now);
    let items = pending_relays(store, &ctx, &view, &events, now);
    let total = items.len();
    let shown: Vec<&RelayItem> = items.iter().take(limit).collect();
    if common.json {
        let v = serde_json::json!({ "total": total, "items": shown });
        crate::print_stdout(&serde_json::to_string_pretty(&v).unwrap_or_default());
        return 0;
    }
    if total == 0 {
        crate::print_stdout("Nothing to relay.");
        return 0;
    }
    let mut out = String::new();
    for (n, r) in shown.iter().enumerate() {
        out.push_str(&format!(
            "{}. note {} · from {} · {}\n",
            n + 1,
            r.id,
            r.from,
            clock::relative(now, r.since_ms)
        ));
        for a in &r.agents {
            out.push_str(&format!("   SendMessage to {} ({}):\n", a.address, a.name));
        }
        for line in r.text.lines() {
            out.push_str(&format!("   | {line}\n"));
        }
        out.push_str(&format!("   Sent: {} · its answer: {}\n", r.sent, r.answer));
    }
    if total > shown.len() {
        out.push_str(&format!(
            "… and {} more (harbard relay --limit {total})\n",
            total - shown.len()
        ));
    }
    crate::print_stdout(out.trim_end());
    0
}

pub fn cmd_owed(store: &Store, common: &Common, limit: usize, now: u64) -> i32 {
    let ctx = Context::resolve(common);
    if !ctx.session_is_stable {
        return crate::fail(
            "owed needs to know who you are — pass --session <id> (your harness's session id)",
        );
    }
    let (view, events) = workspace(store, common, now);
    let items = owed_items(store, &ctx, &view, &events, now);
    let total = items.len();
    let shown: Vec<&OwedItem> = items.iter().take(limit).collect();
    if common.json {
        let v = serde_json::json!({
            "session": ctx.session,
            "total": total,
            "items": shown,
        });
        crate::print_stdout(&serde_json::to_string_pretty(&v).unwrap_or_default());
        return 0;
    }
    let mut out = String::new();
    if total == 0 {
        out.push_str("Nothing owed.\n");
    }
    for (n, i) in shown.iter().enumerate() {
        let who = match (&i.from, &i.agent) {
            (Some(f), Some(a)) => format!("{f} for {a}"),
            (Some(f), None) => f.clone(),
            (None, Some(a)) => a.clone(),
            (None, None) => String::new(),
        };
        out.push_str(&format!(
            "{}. {} · {who} · {}\n   {}\n   → {}\n",
            n + 1,
            i.kind,
            clock::relative(now, i.since_ms),
            i.text,
            i.command
        ));
    }
    if total > shown.len() {
        out.push_str(&format!(
            "… and {} more (harbard owed --limit {total})\n",
            total - shown.len()
        ));
    }
    crate::print_stdout(out.trim_end());
    0
}
