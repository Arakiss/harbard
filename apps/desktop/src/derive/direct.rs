//! Direct messages in words and lists: `directAgents`, `hiddenAgents`,
//! `hideAction` (`lib/hidden.ts`), the composer's states, `noteReader`,
//! `agentRelation`, `dmHint`, `nameOrigin`, `deliveryReason`, `conversation`
//! (`lib/workspace.ts`), and History's `directMode`, `endingText`,
//! `spanText`, `lasted` (`lib/history.ts`).

use crate::derive::cards::{session_gone, session_gone_notice};
use crate::derive::{LEAD_AUTHOR, agent_name};
use crate::i18n::{t, tv};
use crate::model::{Agent, Message, WorkspaceView};
use crate::ui::format::{day_label, duration, hhmm, local_date};
use gpui::{App, SharedString};
use std::cmp::Ordering;

/// Lowercase for matching, the console's `toLocaleLowerCase("es")`.
pub fn fold(s: &str) -> String {
    s.to_lowercase()
}

/// The sidebar's and Direct's agent filter: by name, handle or team.
pub fn matches_agent(a: &Agent, q: &str) -> bool {
    let q = fold(q.trim());
    if q.is_empty() {
        return true;
    }
    let has = |s: Option<&str>| s.is_some_and(|s| fold(s).contains(&q));
    has(Some(&a.name)) || has(a.handle.as_deref()) || has(a.team.as_deref())
}

/// The time of an agent's last message, 0 when it has none in the view.
fn last_ts(view: &WorkspaceView, a: &Agent) -> u64 {
    a.last_message
        .as_deref()
        .and_then(|id| view.message(id))
        .map_or(0, |m| m.ts_ms)
}

/// Direct: agents with a conversation, newest message first, none hidden.
pub fn direct_agents<'a>(view: &'a WorkspaceView, query: &str) -> Vec<&'a Agent> {
    let mut out: Vec<&Agent> = view
        .agents
        .iter()
        .filter(|a| !a.temporary && !a.hidden)
        .filter(|a| a.last_message.is_some() || a.kind == "teammate" || a.kind == "session")
        .filter(|a| matches_agent(a, query))
        .collect();
    out.sort_by(|a, b| {
        last_ts(view, b)
            .cmp(&last_ts(view, a))
            .then_with(|| a.key.cmp(&b.key))
    });
    out
}

/// A collation close enough to `localeCompare(…, "es")`: case and accents
/// folded first, the raw text breaking ties.
pub fn collate(a: &str, b: &str) -> Ordering {
    let key = |s: &str| -> String {
        s.chars()
            .flat_map(char::to_lowercase)
            .map(|c| match c {
                'á' | 'à' | 'ä' | 'â' => 'a',
                'é' | 'è' | 'ë' | 'ê' => 'e',
                'í' | 'ì' | 'ï' | 'î' => 'i',
                'ó' | 'ò' | 'ö' | 'ô' => 'o',
                'ú' | 'ù' | 'ü' | 'û' => 'u',
                other => other,
            })
            .collect()
    };
    key(a).cmp(&key(b)).then_with(|| a.cmp(b))
}

/// "Hidden (N)": every hidden agent, by name.
pub fn hidden_agents(view: &WorkspaceView) -> Vec<&Agent> {
    let mut out: Vec<&Agent> = view.agents.iter().filter(|a| a.hidden).collect();
    out.sort_by(|a, b| collate(&a.name, &b.name).then_with(|| a.key.cmp(&b.key)));
    out
}

/// The ⋯ menu's item for an agent: its words and the action it posts.
pub fn hide_action(a: &Agent, cx: &App) -> (SharedString, &'static str) {
    if a.hidden {
        (t(cx, "shell.show"), "show-agent")
    } else {
        (t(cx, "shell.hide"), "hide-agent")
    }
}

/// One agent's conversation, flat, as a direct message reads.
pub fn conversation<'a>(view: &'a WorkspaceView, agent: &str) -> Vec<&'a Message> {
    view.messages
        .iter()
        .filter(|m| m.agent == agent && m.structured.is_none())
        .collect()
}

/// The name of whoever signs as a session: "Claude" for an unnamed lead.
pub fn lead_display_name(author: &str) -> &str {
    if author == LEAD_AUTHOR {
        "Claude"
    } else {
        author
    }
}

/// "Operator" for the default operator name.
pub fn operator_name(name: &str, cx: &App) -> SharedString {
    if name.is_empty() || name == "Operator" {
        t(cx, "workspace.operatorDefaultName")
    } else {
        name.to_string().into()
    }
}

/// Who reads what is written to `key`: the session itself, or the lead an
/// in-process agent depends on. `None` when the view has no such session.
pub fn note_reader<'a>(agents: &'a [Agent], key: &str) -> Option<&'a str> {
    let agent = agents.iter().find(|a| a.key == key)?;
    if agent.kind == "session" {
        return Some(lead_display_name(&agent.name));
    }
    let lead_key = format!("session:{}", agent.lead_session);
    agents
        .iter()
        .find(|a| a.key == lead_key)
        .map(|l| lead_display_name(&l.name))
}

/// "Main session", "Teammate of Claude", "Subagent of another session".
pub fn agent_relation(agent: &Agent, agents: &[Agent], cx: &App) -> SharedString {
    if agent.kind == "session" {
        return t(cx, "workspace.relationMainSession");
    }
    let reader = note_reader(agents, &agent.key)
        .map(|s| SharedString::from(s.to_string()))
        .or_else(|| {
            // The agent itself may not be in `agents` (History's copy).
            let lead_key = format!("session:{}", agent.lead_session);
            agents
                .iter()
                .find(|a| a.key == lead_key)
                .map(|l| SharedString::from(lead_display_name(&l.name).to_string()))
        })
        .unwrap_or_else(|| t(cx, "workspace.relationOtherSession"));
    if agent.kind == "teammate" {
        tv(cx, "workspace.relationTeammateOf", &[("name", &reader)])
    } else {
        tv(cx, "workspace.relationSubagentOf", &[("name", &reader)])
    }
}

/// What the DM composer says about the path a message takes.
pub fn dm_hint(agent: &Agent, reader: Option<&str>, cx: &App) -> SharedString {
    if session_gone(agent) {
        return format!("{}.", session_gone_notice(cx)).into();
    }
    let name = agent_name(agent, cx);
    if agent.kind == "session" {
        return tv(cx, "workspace.dmSession", &[("name", &name)]);
    }
    match reader {
        Some(reader) => tv(
            cx,
            "workspace.dmVia",
            &[("name", &name), ("reader", reader)],
        ),
        None => tv(cx, "workspace.dmLead", &[("name", &name)]),
    }
}

/// When every lead the composer would reach is stale: the note waits.
pub fn composer_notice(agents: &[&Agent], cx: &App) -> Option<SharedString> {
    if agents.is_empty() || agents.iter().any(|a| a.delivery == "ready") {
        return None;
    }
    let quiet = agents
        .iter()
        .filter(|a| a.delivery == "stale")
        .map(|a| a.lead_quiet_ms.unwrap_or(0))
        .min()?;
    Some(tv(
        cx,
        "workspace.composerStale",
        &[("time", &duration(quiet))],
    ))
}

/// What the composer says instead of a box: only when nobody at all is there.
pub fn composer_block(agents: &[&Agent], cx: &App) -> Option<SharedString> {
    agents.is_empty().then(|| t(cx, "workspace.noReader"))
}

/// The question before sending when no live session would read it.
pub fn composer_confirm(agents: &[&Agent], cx: &App) -> Option<SharedString> {
    if agents.is_empty()
        || agents
            .iter()
            .any(|a| a.delivery == "ready" || a.delivery == "stale")
    {
        return None;
    }
    Some(t(cx, "workspace.nobodyLive"))
}

/// Why nobody reads this agent, for a clause; empty while one can.
pub fn delivery_reason(delivery: &str, cx: &App) -> SharedString {
    match delivery {
        "stale" => t(cx, "workspace.deliveryReasonStale"),
        "ended" => t(cx, "workspace.deliveryReasonEnded"),
        "not_on_bus" => t(cx, "workspace.deliveryReasonNotOnBus"),
        _ => SharedString::default(),
    }
}

/// Where a workshop name came from, in Profile.
pub fn name_origin(agent: &Agent, cx: &App) -> Option<SharedString> {
    match agent.name_source.as_deref()? {
        "picked" => Some(match &agent.name_reason {
            Some(reason) => tv(cx, "workspace.originPickedWhy", &[("reason", reason)]),
            None => t(cx, "workspace.originPicked"),
        }),
        "renamed" => Some(t(cx, "workspace.originRenamed")),
        "launched" => Some(t(cx, "workspace.originLaunched")),
        "auto" => Some(t(cx, "workspace.originAuto")),
        _ => None,
    }
}

/// How Direct opens an agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Live,
    ReadOnly,
    Loading,
    Missing,
}

/// Live while the shell has it and its session has not ended; read-only
/// when it ended or only the archive has it. `archived` is `None` until
/// the archive answers, `Some(false)` when it has nothing.
pub fn direct_mode(live: Option<&Agent>, archived: Option<bool>) -> Mode {
    match (live, archived) {
        (Some(a), _) if a.delivery == "ended" => Mode::ReadOnly,
        (Some(_), _) => Mode::Live,
        (None, None) => Mode::Loading,
        (None, Some(true)) => Mode::ReadOnly,
        (None, Some(false)) => Mode::Missing,
    }
}

/// "Finished", "Closed", "Verified by harbard", "Never closed", "In progress".
pub fn ending_text(ending: Option<&str>, cx: &App) -> SharedString {
    match ending {
        Some("finished") => t(cx, "history.endingFinished"),
        Some("closed") => t(cx, "history.endingClosed"),
        Some("verified") => t(cx, "history.endingVerified"),
        Some(_) => t(cx, "history.endingNone"),
        None => t(cx, "history.endingOngoing"),
    }
}

const MIN: u64 = 60_000;
const HOUR: u64 = 60 * MIN;
const DAY: u64 = 24 * HOUR;

/// "45 min", "2 h 5 min", "3 d 4 h"; `None` under a minute.
pub fn lasted_plain(ms: u64) -> Option<String> {
    let m = ms / MIN;
    if m < 1 {
        return None;
    }
    if m < 60 {
        return Some(format!("{m} min"));
    }
    if ms < DAY {
        let (h, rest) = (m / 60, m % 60);
        return Some(if rest > 0 {
            format!("{h} h {rest} min")
        } else {
            format!("{h} h")
        });
    }
    let (d, h) = (ms / DAY, (ms % DAY) / HOUR);
    Some(if h > 0 {
        format!("{d} d {h} h")
    } else {
        format!("{d} d")
    })
}

pub fn lasted(ms: u64, cx: &App) -> SharedString {
    lasted_plain(ms)
        .map(Into::into)
        .unwrap_or_else(|| t(cx, "history.underOneMinute"))
}

fn short_date(ms: u64, cx: &App) -> String {
    let (_, mo, d, _) = local_date(ms);
    const EN: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    const ES: [&str; 12] = [
        "ene", "feb", "mar", "abr", "may", "jun", "jul", "ago", "sept", "oct", "nov", "dic",
    ];
    let es = cx.global::<crate::i18n::I18n>().locale == crate::i18n::Locale::Es;
    let month = if es { ES } else { EN }[(mo.max(1) - 1) as usize];
    format!("{d} {month}")
}

/// "08:30 – 10:00 · 1 h 30 min", with the dates when it crossed midnight.
pub fn span_text(start_ms: u64, end_ms: u64, duration_ms: u64, now: u64, cx: &App) -> SharedString {
    let same = day_label(start_ms, now, cx) == day_label(end_ms, now, cx);
    let at = |ms: u64| {
        if same {
            hhmm(ms)
        } else {
            format!("{} {}", short_date(ms, cx), hhmm(ms))
        }
    };
    format!(
        "{} – {} · {}",
        at(start_ms),
        at(end_ms),
        lasted(duration_ms, cx)
    )
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(key: &str, name: &str) -> Agent {
        Agent {
            key: key.into(),
            name: name.into(),
            kind: "teammate".into(),
            ..Default::default()
        }
    }

    #[test]
    fn direct_lists_the_visible_ones_newest_first_and_filters() {
        let mut hidden = agent("h", "Hidden");
        hidden.hidden = true;
        let mut sub = agent("s", "Sub");
        sub.kind = "subagent".into();
        let mut talked = agent("t", "Talked");
        talked.last_message = Some("m".into());
        let mut temp = agent("x", "Temp");
        temp.temporary = true;
        let view = WorkspaceView {
            agents: vec![
                agent("b", "Barry"),
                hidden,
                sub,
                talked,
                agent("a", "Adrian"),
                temp,
            ],
            messages: vec![Message {
                id: "m".into(),
                ts_ms: 5,
                ..Default::default()
            }],
            ..Default::default()
        };
        let keys: Vec<&str> = direct_agents(&view, "")
            .iter()
            .map(|a| a.key.as_str())
            .collect();
        assert_eq!(keys, ["t", "a", "b"]);
        let keys: Vec<&str> = direct_agents(&view, "BAR")
            .iter()
            .map(|a| a.key.as_str())
            .collect();
        assert_eq!(keys, ["b"]);
        let hidden: Vec<&str> = hidden_agents(&view)
            .iter()
            .map(|a| a.key.as_str())
            .collect();
        assert_eq!(hidden, ["h"]);
    }

    #[test]
    fn hidden_sorts_by_name_ignoring_case_and_accents() {
        let mut a = agent("1", "Álvaro");
        a.hidden = true;
        let mut b = agent("2", "beto");
        b.hidden = true;
        let mut c = agent("3", "Zoe");
        c.hidden = true;
        let view = WorkspaceView {
            agents: vec![c, b, a],
            ..Default::default()
        };
        let names: Vec<&str> = hidden_agents(&view)
            .iter()
            .map(|a| a.name.as_str())
            .collect();
        assert_eq!(names, ["Álvaro", "beto", "Zoe"]);
    }

    #[test]
    fn the_reader_is_the_session_or_its_lead() {
        let mut lead = agent("session:L1", LEAD_AUTHOR);
        lead.kind = "session".into();
        let mut mate = agent("m", "Mate");
        mate.lead_session = "L1".into();
        let mut orphan = agent("o", "Orphan");
        orphan.lead_session = "nope".into();
        let agents = vec![lead, mate, orphan];
        assert_eq!(note_reader(&agents, "session:L1"), Some("Claude"));
        assert_eq!(note_reader(&agents, "m"), Some("Claude"));
        assert_eq!(note_reader(&agents, "o"), None);
        assert_eq!(note_reader(&agents, "missing"), None);
    }

    #[test]
    fn direct_mode_follows_the_session_and_the_archive() {
        let live = agent("a", "A");
        let mut ended = agent("e", "E");
        ended.delivery = "ended".into();
        assert_eq!(direct_mode(Some(&live), None), Mode::Live);
        assert_eq!(direct_mode(Some(&ended), None), Mode::ReadOnly);
        assert_eq!(direct_mode(None, None), Mode::Loading);
        assert_eq!(direct_mode(None, Some(true)), Mode::ReadOnly);
        assert_eq!(direct_mode(None, Some(false)), Mode::Missing);
    }

    #[test]
    fn lasted_reads_like_history() {
        assert_eq!(lasted_plain(30_000), None);
        assert_eq!(lasted_plain(45 * MIN), Some("45 min".into()));
        assert_eq!(lasted_plain(2 * HOUR + 5 * MIN), Some("2 h 5 min".into()));
        assert_eq!(lasted_plain(2 * HOUR), Some("2 h".into()));
        assert_eq!(lasted_plain(3 * DAY + 4 * HOUR), Some("3 d 4 h".into()));
        assert_eq!(lasted_plain(3 * DAY), Some("3 d".into()));
    }

    #[test]
    fn conversation_is_one_agents_messages_without_records() {
        let m = |id: &str, agent: &str, structured: bool| Message {
            id: id.into(),
            agent: agent.into(),
            structured: structured.then(|| "x".into()),
            ..Default::default()
        };
        let view = WorkspaceView {
            messages: vec![m("1", "a", false), m("2", "b", false), m("3", "a", true)],
            ..Default::default()
        };
        let ids: Vec<&str> = conversation(&view, "a")
            .iter()
            .map(|m| m.id.as_str())
            .collect();
        assert_eq!(ids, ["1"]);
    }
}
