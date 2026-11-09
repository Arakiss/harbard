//! History: the sessions and agents of the last month, newest first, one
//! page at a time, and the conversation of any of them.
//!
//! The shell keeps 24 hours; this is where the rest is read. It is the same
//! workspace read ([`crate::workspace::build`]) over a month of the bus and
//! of the transcripts, so a name, a role, a face or an ending here is the one
//! the shell would have shown. Nothing here invents a state: how an item
//! ended is the agent's own state ("Finished", "Closed", "verified") or,
//! when it went quiet without one, "no closure".
//!
//! Every text that leaves this module (the last human line, the brief, a
//! message and its detail) goes through [`crate::redact::mask_secrets`]
//! again: an archive is read long after the fact, by whoever opens it.

use crate::redact::mask_secrets;
use crate::workspace::{ACTIVE_MS, Agent, Message, WorkspaceView};
use serde::Serialize;
use std::collections::BTreeMap;

/// How far back History reads: a month and a day, so the same date last
/// month is still there.
pub const WINDOW_MS: u64 = 31 * 24 * 3_600_000;
/// Items in a page when the caller does not say.
pub const PAGE_DEFAULT: usize = 50;
/// The most a caller may ask for at once.
pub const PAGE_MAX: usize = 200;
/// The most messages an archived conversation carries, the newest ones.
pub const CONVERSATION_MAX: usize = 1_000;

/// How an item ended: its own state, or silence.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Ending {
    /// Its turn ended with nothing left to do ("Finished").
    Finished,
    /// Its lead stopped it (TaskStop) ("Closed").
    Closed,
    /// harbard verified what it delivered ("verified").
    Verified,
    /// It went quiet a day ago or its session closed, with none of those.
    NoClosure,
}

/// One past (or still open) session or agent.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Item {
    /// The workspace agent: name, role line, team, face, delivery.
    pub agent: Agent,
    /// The first channel it wrote in, and that channel's repository.
    pub channel: Option<String>,
    pub repo: Option<String>,
    pub start_ms: u64,
    pub end_ms: u64,
    pub duration_ms: u64,
    /// Its own last human line, masked.
    pub last_line: Option<String>,
    /// `None` while it is still working or waiting.
    pub ending: Option<Ending>,
    /// Nobody would read a note now: its session ended, or it has been
    /// quiet for longer than the shell keeps. Its conversation opens
    /// without a composer. While the shell still holds the agent, the
    /// console goes by the shell's copy instead (its lead may relay).
    pub read_only: bool,
}

/// A channel the filter offers, with how many items it holds.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ChannelCount {
    pub id: String,
    pub repo: String,
    pub items: usize,
}

/// What a page asks for.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Query {
    /// The `next` of the previous page.
    pub before: Option<String>,
    pub limit: Option<usize>,
    pub channel: Option<String>,
    pub q: Option<String>,
}

/// One page, newest first.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Page {
    pub items: Vec<Item>,
    /// The cursor of the next page; `None` on the last one.
    pub next: Option<String>,
    /// Items that match the filter and the search, over every page.
    pub total: usize,
    /// Every channel in the window, for the filter (not narrowed by it).
    pub channels: Vec<ChannelCount>,
    pub now_ms: u64,
    pub window_days: u64,
}

/// An archived conversation, read-only when its item is.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Conversation {
    pub item: Item,
    pub messages: Vec<Message>,
    /// More messages than [`CONVERSATION_MAX`]: the oldest were left out.
    pub truncated: bool,
    pub operator: String,
    pub operator_initials: String,
    pub now_ms: u64,
}

/// How an agent ended, from what the workspace already says about it.
pub fn ending(agent: &Agent, end_ms: u64, now_ms: u64) -> Option<Ending> {
    if agent.closed || agent.activity.detail.as_deref() == Some("closed") {
        return Some(Ending::Closed);
    }
    match agent.activity.state {
        "verified" => return Some(Ending::Verified),
        "finished" => return Some(Ending::Finished),
        _ => {}
    }
    // A lost agent reads as finished in the shell (grey), but nothing says
    // it finished: its process went away mid-turn.
    if agent.lost {
        return Some(Ending::NoClosure);
    }
    if agent.presence == "finished" {
        return Some(Ending::Finished);
    }
    let silent = now_ms.saturating_sub(end_ms) > ACTIVE_MS;
    (silent || agent.delivery == "ended").then_some(Ending::NoClosure)
}

/// Messages an agent's conversation holds, oldest first.
fn own_messages<'v>(view: &'v WorkspaceView, key: &str) -> Vec<&'v Message> {
    let mut out: Vec<&Message> = view
        .messages
        .iter()
        .filter(|m| m.agent == key && m.structured.is_none())
        .collect();
    out.sort_by(|a, b| a.ts_ms.cmp(&b.ts_ms).then(a.id.cmp(&b.id)));
    out
}

fn masked(text: Option<&String>) -> Option<String> {
    text.map(|t| mask_secrets(t))
}

fn item(view: &WorkspaceView, a: &Agent) -> Item {
    let mine = own_messages(view, &a.key);
    let last_said = mine.last().map_or(0, |m| m.ts_ms);
    let end_ms = a.last_action_ms.unwrap_or(0).max(last_said);
    let start_ms = a
        .first_ms
        .or_else(|| mine.first().map(|m| m.ts_ms))
        .unwrap_or(end_ms)
        .min(end_ms);
    let last_line = mine
        .iter()
        .rev()
        .find(|m| matches!(m.from, "agent" | "session"))
        .map(|m| mask_secrets(&m.text));
    let channel = a.channels.first().cloned();
    let repo = channel
        .as_deref()
        .and_then(|id| view.channels.iter().find(|c| c.id == id))
        .map(|c| c.repo.clone());
    let ending = ending(a, end_ms, view.now_ms);
    let read_only = a.delivery == "ended" || view.now_ms.saturating_sub(end_ms) > ACTIVE_MS;
    let mut agent = a.clone();
    agent.brief = masked(a.brief.as_ref());
    agent.description = masked(a.description.as_ref());
    agent.name = mask_secrets(&a.name);
    Item {
        agent,
        channel,
        repo,
        start_ms,
        end_ms,
        duration_ms: end_ms - start_ms,
        last_line,
        ending,
        read_only,
    }
}

/// Every non-temporary agent of the view, newest end first.
pub fn items(view: &WorkspaceView) -> Vec<Item> {
    let mut out: Vec<Item> = view
        .agents
        .iter()
        .filter(|a| !a.temporary)
        .map(|a| item(view, a))
        .collect();
    out.sort_by(|a, b| b.end_ms.cmp(&a.end_ms).then(a.agent.key.cmp(&b.agent.key)));
    out
}

/// The cursor that resumes after `item`.
pub fn cursor(item: &Item) -> String {
    format!("{}:{}", item.end_ms, item.agent.key)
}

fn parse_cursor(s: &str) -> Option<(u64, &str)> {
    let (ms, key) = s.split_once(':')?;
    let ms = ms.parse().ok()?;
    (!key.is_empty()).then_some((ms, key))
}

/// Lower case without accents, so "Renée" is found by "brujula".
fn fold(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'á' | 'à' | 'â' | 'ä' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'í' | 'ì' | 'î' | 'ï' => 'i',
            'ó' | 'ò' | 'ô' | 'ö' => 'o',
            'ú' | 'ù' | 'û' | 'ü' => 'u',
            'ñ' => 'n',
            'ç' => 'c',
            other => other,
        })
        .collect()
}

fn matches(item: &Item, words: &[String]) -> bool {
    let a = &item.agent;
    let hay = fold(
        &[
            Some(a.name.as_str()),
            Some(a.handle.as_str()),
            a.role_line.as_deref(),
            a.team.as_deref(),
            a.role.as_deref(),
            a.description.as_deref(),
            a.brief.as_deref(),
            a.branch.as_deref(),
            item.last_line.as_deref(),
            item.repo.as_deref(),
        ]
        .into_iter()
        .flatten()
        .chain(a.channels.iter().map(String::as_str))
        .collect::<Vec<_>>()
        .join("\n"),
    );
    words.iter().all(|w| hay.contains(w.as_str()))
}

/// One page of `all` (already newest first, as [`items`] returns them).
pub fn page(all: &[Item], q: &Query, now_ms: u64) -> Page {
    let mut counts: BTreeMap<&str, (String, usize)> = BTreeMap::new();
    for i in all {
        for c in &i.agent.channels {
            let e = counts
                .entry(c.as_str())
                .or_insert_with(|| (String::new(), 0));
            e.1 += 1;
            if e.0.is_empty()
                && i.channel.as_deref() == Some(c.as_str())
                && let Some(r) = &i.repo
            {
                e.0 = r.clone();
            }
        }
    }
    let mut channels: Vec<ChannelCount> = counts
        .into_iter()
        .map(|(id, (repo, items))| ChannelCount {
            id: id.to_string(),
            repo,
            items,
        })
        .collect();
    channels.sort_by(|a, b| b.items.cmp(&a.items).then(a.id.cmp(&b.id)));

    let words: Vec<String> =
        q.q.as_deref()
            .map(fold)
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_string)
            .collect();
    let channel = q
        .channel
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    let kept: Vec<&Item> = all
        .iter()
        .filter(|i| channel.is_none_or(|c| i.agent.channels.iter().any(|x| x == c)))
        .filter(|i| matches(i, &words))
        .collect();
    let total = kept.len();
    let after = q.before.as_deref().and_then(parse_cursor);
    let start = after.map_or(0, |(ms, key)| {
        kept.iter()
            .position(|i| i.end_ms < ms || (i.end_ms == ms && i.agent.key.as_str() > key))
            .unwrap_or(kept.len())
    });
    let limit = q.limit.unwrap_or(PAGE_DEFAULT).clamp(1, PAGE_MAX);
    let items: Vec<Item> = kept[start..]
        .iter()
        .take(limit)
        .map(|i| (*i).clone())
        .collect();
    let next = (start + items.len() < kept.len())
        .then(|| items.last().map(cursor))
        .flatten();
    Page {
        items,
        next,
        total,
        channels,
        now_ms,
        window_days: WINDOW_MS / (24 * 3_600_000),
    }
}

/// The conversation of `key`, masked, or `None` when the window has no such agent.
pub fn conversation(view: &WorkspaceView, all: &[Item], key: &str) -> Option<Conversation> {
    let item = all.iter().find(|i| i.agent.key == key)?.clone();
    let mine = own_messages(view, key);
    let truncated = mine.len() > CONVERSATION_MAX;
    let messages = mine
        .into_iter()
        .rev()
        .take(CONVERSATION_MAX)
        .rev()
        .map(|m| {
            let mut m = m.clone();
            m.text = mask_secrets(&m.text);
            m.body = m.body.as_deref().map(mask_secrets);
            m
        })
        .collect();
    Some(Conversation {
        item,
        messages,
        truncated,
        operator: view.operator.clone(),
        operator_initials: view.operator_initials.clone(),
        now_ms: view.now_ms,
    })
}

#[cfg(test)]
mod tests {
    //! Synthetic agents only, field by field.

    use super::*;
    use crate::activity::Activity;
    use crate::workspace::{Channel, Thresholds};

    const NOW: u64 = 1_794_000_000_000;
    const H: u64 = 3_600_000;
    const D: u64 = 24 * H;

    fn agent(key: &str, name: &str, state: &'static str, last: u64) -> Agent {
        Agent {
            key: key.into(),
            name: name.into(),
            label: format!("general-purpose · {key}"),
            kind: "teammate",
            lead_session: "lead-1".into(),
            delivery: "ready",
            state,
            state_reason: "end_turn",
            presence: if matches!(state, "finished" | "stopped") {
                "finished"
            } else {
                "working"
            },
            last_action_ms: Some(last),
            first_ms: Some(last - 2 * H),
            brief: Some("Build the archive".into()),
            description: Some("Historial".into()),
            channels: vec!["harbard".into()],
            activity: Activity::new("finished", last, None, last),
            team: Some("harbard".into()),
            role: Some("constructora".into()),
            handle: name.to_lowercase(),
            role_line: Some("constructora · harbard".into()),
            name_source: Some("picked".into()),
            ..Default::default()
        }
    }

    fn message(id: &str, agent: &str, from: &'static str, ts: u64, text: &str) -> Message {
        Message {
            id: id.into(),
            agent: agent.into(),
            from,
            author: agent.into(),
            ts_ms: ts,
            text: text.into(),
            chars: text.chars().count(),
            ..Default::default()
        }
    }

    fn channel(id: &str) -> Channel {
        Channel {
            id: id.into(),
            repo: format!("/work/{id}"),
            last_ms: NOW,
            reach: "nobody",
            active_ms: NOW,
            ..Default::default()
        }
    }

    fn view(agents: Vec<Agent>, messages: Vec<Message>) -> WorkspaceView {
        WorkspaceView {
            v: 1,
            now_ms: NOW,
            operator: "Dana".into(),
            operator_initials: "DR".into(),
            thresholds: Thresholds {
                amber_ms: 1,
                red_ms: 2,
                thinking_long_ms: 3,
                waiting_sleepy_ms: 4,
                running_long_ms: 5,
            },
            channels: vec![channel("harbard"), channel("contoso")],
            agents,
            messages,
            team_root_found: true,
            night_dim: true,
            ..Default::default()
        }
    }

    /// Sixty agents, one every three hours back from now, alternating channels.
    fn month() -> WorkspaceView {
        let agents = (0..60)
            .map(|i| {
                let mut a = agent(
                    &format!("a{i:02}"),
                    &format!("Robot{i:02}"),
                    "finished",
                    NOW - i * 3 * H,
                );
                if i % 2 == 1 {
                    a.channels = vec!["contoso".into()];
                }
                a
            })
            .collect();
        view(agents, Vec::new())
    }

    #[test]
    fn items_are_newest_first_with_start_end_and_duration() {
        let v = view(
            vec![
                agent("old", "Vieja", "finished", NOW - 3 * D),
                agent("new", "Nueva", "finished", NOW - H),
            ],
            Vec::new(),
        );
        let all = items(&v);
        assert_eq!(
            all.iter().map(|i| i.agent.key.as_str()).collect::<Vec<_>>(),
            ["new", "old"]
        );
        assert_eq!(all[0].end_ms, NOW - H);
        assert_eq!(all[0].start_ms, NOW - 3 * H);
        assert_eq!(all[0].duration_ms, 2 * H);
        assert_eq!(all[0].channel.as_deref(), Some("harbard"));
        assert_eq!(all[0].repo.as_deref(), Some("/work/harbard"));
    }

    #[test]
    fn a_temporary_agent_is_not_history() {
        let mut t = agent("tmp", "Temporal", "finished", NOW - H);
        t.temporary = true;
        assert!(items(&view(vec![t], Vec::new())).is_empty());
    }

    #[test]
    fn how_it_ended_is_its_own_state_or_silence() {
        let done = agent("done", "Hecha", "finished", NOW - 2 * D);
        let mut closed = agent("closed", "Cerrada", "stopped", NOW - 2 * D);
        closed.closed = true;
        closed.activity =
            Activity::new("finished", NOW - 2 * D, Some("closed".into()), NOW - 2 * D);
        let mut verified = agent("verified", "Verificada", "finished", NOW - 2 * D);
        verified.activity = Activity::new("verified", NOW - 2 * D, None, NOW - 2 * D);
        let mut quiet = agent("quiet", "Callada", "running", NOW - 2 * D);
        quiet.presence = "waiting";
        quiet.activity = Activity::new("asleep", NOW - 2 * D, None, NOW - 2 * D);
        let mut lost = agent("lost", "Perdida", "running", NOW - 3 * H);
        lost.lost = true;
        lost.presence = "finished";
        lost.activity = Activity::new("thinking", NOW - 3 * H, None, NOW - 3 * H);
        let mut working = agent("working", "Activa", "running", NOW - 60_000);
        working.activity = Activity::new("editing", NOW - 60_000, None, NOW - 60_000);
        let all = items(&view(
            vec![done, closed, verified, quiet, lost, working],
            Vec::new(),
        ));
        let of = |k: &str| all.iter().find(|i| i.agent.key == k).unwrap().ending;
        assert_eq!(of("done"), Some(Ending::Finished));
        assert_eq!(of("closed"), Some(Ending::Closed));
        assert_eq!(of("verified"), Some(Ending::Verified));
        assert_eq!(of("quiet"), Some(Ending::NoClosure));
        assert_eq!(
            of("lost"),
            Some(Ending::NoClosure),
            "grey in the shell, but it never said it finished"
        );
        assert_eq!(of("working"), None);
    }

    #[test]
    fn an_ended_session_or_a_day_of_silence_is_read_only() {
        let mut ended = agent("ended", "Session", "finished", NOW - H);
        ended.kind = "session";
        ended.delivery = "ended";
        let old = agent("old", "Antigua", "finished", NOW - 2 * D);
        let today = agent("today", "Hoy", "finished", NOW - H);
        let mut working = agent("working", "Activa", "running", NOW - 60_000);
        working.activity = Activity::new("editing", NOW - 60_000, None, NOW - 60_000);
        let all = items(&view(vec![ended, old, today, working], Vec::new()));
        let ro = |k: &str| all.iter().find(|i| i.agent.key == k).unwrap().read_only;
        assert!(ro("ended"), "its session closed: nobody reads a note");
        assert!(ro("old"), "older than the shell keeps: archive only");
        assert!(!ro("today"), "its lead can still relay a note");
        assert!(!ro("working"));
    }

    #[test]
    fn the_last_human_line_is_its_own_and_masked() {
        let a = agent("a", "Alice", "finished", NOW - H);
        let msgs = vec![
            message("m1", "a", "agent", NOW - 3 * H, "Starting with the history"),
            message(
                "m2",
                "a",
                "agent",
                NOW - 2 * H,
                concat!(
                    "Listo, la clave era ghp_abcd",
                    "efghijklmnopqrstuvwxyz0123456789"
                ),
            ),
            message("m3", "a", "operator", NOW - H, "Thanks"),
        ];
        let all = items(&view(vec![a], msgs));
        let line = all[0].last_line.as_deref().unwrap();
        assert!(line.starts_with("Listo, la clave era "), "{line}");
        assert!(
            !line.contains(concat!("ghp_abcd", "efghijklmnopqrstuvwxyz0123456789")),
            "{line}"
        );
    }

    #[test]
    fn pages_follow_the_cursor_without_gaps_or_repeats() {
        let v = month();
        let all = items(&v);
        let mut seen = Vec::new();
        let mut q = Query {
            limit: Some(25),
            ..Query::default()
        };
        let mut pages = 0;
        loop {
            let p = page(&all, &q, NOW);
            pages += 1;
            assert_eq!(p.total, 60);
            assert!(p.items.len() <= 25);
            seen.extend(p.items.iter().map(|i| i.agent.key.clone()));
            match p.next {
                Some(next) => q.before = Some(next),
                None => break,
            }
        }
        assert_eq!(pages, 3);
        let want: Vec<String> = (0..60).map(|i| format!("a{i:02}")).collect();
        assert_eq!(seen, want);
    }

    #[test]
    fn a_page_is_never_larger_than_the_cap_and_defaults_to_fifty() {
        let all = items(&month());
        assert_eq!(page(&all, &Query::default(), NOW).items.len(), PAGE_DEFAULT);
        let q = Query {
            limit: Some(10_000),
            ..Query::default()
        };
        assert_eq!(page(&all, &q, NOW).items.len(), 60.min(PAGE_MAX));
        let q = Query {
            limit: Some(0),
            ..Query::default()
        };
        assert_eq!(
            page(&all, &q, NOW).items.len(),
            1,
            "zero means one, never an empty loop"
        );
    }

    #[test]
    fn a_new_item_on_top_does_not_shift_the_next_page() {
        let mut v = month();
        let first = page(
            &items(&v),
            &Query {
                limit: Some(10),
                ..Query::default()
            },
            NOW,
        );
        v.agents.push(agent("fresh", "Fresca", "finished", NOW));
        let q = Query {
            limit: Some(10),
            before: first.next.clone(),
            ..Query::default()
        };
        let second = page(&items(&v), &q, NOW);
        assert_eq!(second.items[0].agent.key, "a10");
    }

    #[test]
    fn a_cursor_that_does_not_parse_starts_from_the_top() {
        let all = items(&month());
        let q = Query {
            before: Some("nonsense".into()),
            limit: Some(5),
            ..Query::default()
        };
        assert_eq!(page(&all, &q, NOW).items[0].agent.key, "a00");
    }

    #[test]
    fn the_channel_filter_keeps_only_that_channel_and_offers_every_channel() {
        let all = items(&month());
        let q = Query {
            channel: Some("contoso".into()),
            limit: Some(200),
            ..Query::default()
        };
        let p = page(&all, &q, NOW);
        assert_eq!(p.total, 30);
        assert!(
            p.items
                .iter()
                .all(|i| i.agent.channels.contains(&"contoso".to_string()))
        );
        let ids: Vec<&str> = p.channels.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(
            ids,
            ["contoso", "harbard"],
            "the filter lists every channel, not only the chosen one"
        );
        assert_eq!(p.channels[0].items, 30);
        assert_eq!(p.channels[0].repo, "/work/contoso");
    }

    #[test]
    fn search_matches_name_role_line_and_last_line_without_case_or_accents() {
        let mut a = agent("a", "Alice", "finished", NOW - H);
        a.role_line = Some("constructora · harbard".into());
        let mut b = agent("b", "Renée", "finished", NOW - 2 * H);
        b.role_line = Some("revisora · protocolo".into());
        b.channels = vec!["contoso".into()];
        let c = agent("c", "Laura", "finished", NOW - 3 * H);
        let msgs = vec![message(
            "m",
            "c",
            "agent",
            NOW - 3 * H,
            "Migración terminada",
        )];
        let all = items(&view(vec![a, b, c], msgs));
        let keys = |q: &str| {
            page(
                &all,
                &Query {
                    q: Some(q.into()),
                    ..Query::default()
                },
                NOW,
            )
            .items
            .into_iter()
            .map(|i| i.agent.key)
            .collect::<Vec<_>>()
        };
        assert_eq!(keys("renee"), ["b"]);
        assert_eq!(keys("REVISORA"), ["b"]);
        assert_eq!(keys("migracion"), ["c"]);
        assert_eq!(keys("contoso"), ["b"], "the channel is searched too");
        assert_eq!(keys("  "), ["a", "b", "c"], "a blank search is no search");
        assert!(keys("nobody").is_empty());
    }

    #[test]
    fn search_and_filter_combine() {
        let all = items(&month());
        let q = Query {
            channel: Some("harbard".into()),
            q: Some("robot0".into()),
            ..Query::default()
        };
        let keys: Vec<String> = page(&all, &q, NOW)
            .items
            .into_iter()
            .map(|i| i.agent.key)
            .collect();
        assert_eq!(keys, ["a00", "a02", "a04", "a06", "a08"]);
    }

    #[test]
    fn a_conversation_is_its_messages_masked_with_the_item() {
        let mut a = agent("a", "Alice", "finished", NOW - 2 * D);
        a.brief = Some(concat!("to", "ken ghp_abcd", "efghijklmnopqrstuvwxyz0123456789").into());
        let mut m2 = message("m2", "a", "agent", NOW - 2 * D, "DONE · ready");
        m2.body = Some(
            concat!(
                "export GITHUB_TOKEN=ghp_abcd",
                "efghijklmnopqrstuvwxyz0123456789"
            )
            .into(),
        );
        let msgs = vec![
            message("m1", "a", "lead", NOW - 2 * D - H, "Build the history"),
            m2,
            message("x", "other", "agent", NOW - D, "No es suyo"),
        ];
        let v = view(vec![a, agent("other", "Otra", "finished", NOW - D)], msgs);
        let all = items(&v);
        let c = conversation(&v, &all, "a").unwrap();
        assert!(c.item.read_only);
        assert_eq!(
            c.messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["m1", "m2"]
        );
        let body = c.messages[1].body.as_deref().unwrap();
        assert!(
            !body.contains(concat!("ghp_abcd", "efghijklmnopqrstuvwxyz0123456789")),
            "{body}"
        );
        assert!(
            !c.item
                .agent
                .brief
                .as_deref()
                .unwrap()
                .contains("ghp_abcdefghij")
        );
        assert!(conversation(&v, &all, "missing").is_none());
    }
}
