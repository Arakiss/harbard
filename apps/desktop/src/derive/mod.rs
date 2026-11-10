//! Pure wording and selection over the read: the console's `lib/*.ts`
//! derivations, ported. Each screen lane adds its own file here
//! (`derive/<area>.rs`); the names below are shared by every screen.

pub mod composer;
pub mod legible;
pub mod markdown;
pub mod members;
pub mod messages;
pub mod rooms;
pub mod work_items;
pub mod workspace;

use crate::i18n::{serve_text, t, tv};
use crate::model::{Agent, WorkspaceView};
use crate::ui::format::{hace, hhmm, local_day};
use gpui::{App, SharedString};

/// serve's fixed lead author, worded by the dictionary.
pub const LEAD_AUTHOR: &str = "Claude · lead";
pub const LEAD_HANDLE: &str = "coordinator";

/// A name as shown: serve's fixed names (the unnamed lead, a bare session)
/// worded in this language.
pub fn shown_name(name: &str, cx: &App) -> SharedString {
    if name == LEAD_AUTHOR {
        return t(cx, "workspace.leadAuthor");
    }
    if let Some(id) = name.strip_prefix("session ")
        && !id.is_empty()
        && id.len() <= 8
        && !id.contains(' ')
    {
        let mut p = crate::model::Params::new();
        p.insert("id".into(), serde_json::Value::String(id.into()));
        return serve_text(cx, Some("name.session.id"), Some(&p), name);
    }
    name.to_string().into()
}

/// An agent's name as shown (`agentName`): `name.team_role` params have
/// their role worded first (`roleVars`).
pub fn agent_name(a: &Agent, cx: &App) -> SharedString {
    workspace::agent_name_with(a, cx)
}

/// The name an agent key shows; `lead:<session>` is a lead session itself.
pub fn agent_label(view: &WorkspaceView, key: &str, cx: &App) -> SharedString {
    if let Some(id) = key.strip_prefix("lead:") {
        return tv(cx, "workspace.leadSessionNamed", &[("id", id.get(..8).unwrap_or(id))]);
    }
    match view.agent(key) {
        Some(a) => agent_name(a, cx),
        None => shown_name(key.get(..8).unwrap_or(key), cx),
    }
}

/// What an agent's robot is drawn from: a lead session by its key, anyone
/// else by its name, so it looks the same everywhere.
pub fn robot_name(a: &Agent) -> &str {
    if a.kind == "session" && a.handle.as_deref() == Some(LEAD_HANDLE) {
        &a.key
    } else {
        &a.name
    }
}

/// "working", "waiting", "finished".
pub fn presence_text(presence: &str, cx: &App) -> SharedString {
    match presence {
        "working" => t(cx, "workspace.presenceWorking"),
        "waiting" => t(cx, "workspace.presenceWaiting"),
        _ => t(cx, "workspace.presenceFinished"),
    }
}

fn since_date(ms: u64, now: u64, cx: &App) -> SharedString {
    if local_day(ms) == local_day(now) {
        return tv(cx, "workspace.sinceTime", &[("time", &hhmm(ms))]);
    }
    let date = crate::ui::format::at_date(ms, now, cx);
    tv(cx, "workspace.sinceDate", &[("date", &date), ("time", &hhmm(ms))])
}

/// "working · 1 min ago"; "no activity since 22:33" when dormant;
/// "no data since 22:33" when lost.
pub fn presence_line(a: &Agent, now: u64, cx: &App) -> SharedString {
    if a.lost {
        return match a.last_action_ms {
            Some(ms) => tv(cx, "workspace.noDataSince", &[("since", &since_date(ms, now, cx))]),
            None => t(cx, "workspace.noData"),
        };
    }
    if a.dormant.is_some()
        && let Some(ms) = a.last_action_ms
    {
        return tv(cx, "workspace.noActivitySince", &[("time", &hhmm(ms))]);
    }
    let base = presence_text(&a.presence, cx);
    match a.last_action_ms {
        Some(ms) => format!("{base} · {}", hace(ms, now, cx)).into(),
        None => base,
    }
}

/// The dot colour: dormant overrides presence. green, amber, red or grey.
pub fn dot_tone(presence: &str, dormant: Option<&str>) -> &'static str {
    match dormant {
        Some("red") => "red",
        Some("amber") => "amber",
        _ => match presence {
            "working" => "green",
            "waiting" => "amber",
            _ => "grey",
        },
    }
}

/// A message label's chip: START neutral, FINDING amber, BLOCKED red, DONE green.
pub fn label_chip(label: &str, cx: &App) -> (SharedString, &'static str) {
    match label {
        "BLOCKED" => (t(cx, "workspace.labelBlocked"), "red"),
        "FINDING" => (t(cx, "workspace.labelFinding"), "amber"),
        "DONE" => (t(cx, "workspace.labelDone"), "green"),
        _ => (t(cx, "workspace.labelStart"), "neutral"),
    }
}
