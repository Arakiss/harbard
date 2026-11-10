//! Ported from the console's `lib/work-items.ts`: a channel read by piece of
//! work instead of by arrival. One row per ticket: what it is, how it goes,
//! and the last thing said about it that a person can read. An agent
//! carries its ticket in its role, description, brief or branch; the title
//! is the ticket's own, in the brief between «».

use super::workspace::Words;
use crate::model::{Agent, Message, WorkspaceView};
use gpui::SharedString;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum WorkStatus {
    Needs,
    Blocked,
    Working,
    Paused,
    Finished,
}

impl WorkStatus {
    /// `data-tone`: amber, green or grey.
    pub fn tone(self) -> &'static str {
        match self {
            WorkStatus::Needs | WorkStatus::Blocked => "amber",
            WorkStatus::Working => "green",
            WorkStatus::Paused | WorkStatus::Finished => "grey",
        }
    }

    /// "Needs you", "Blocked", "Working", "Paused", "Finished".
    pub fn label(self, w: &impl Words) -> SharedString {
        let key = match self {
            WorkStatus::Needs => "workspace.workStatusNeeds",
            WorkStatus::Blocked => "workspace.workStatusBlocked",
            WorkStatus::Working => "workspace.workStatusWorking",
            WorkStatus::Paused => "workspace.workStatusPaused",
            WorkStatus::Finished => "workspace.workStatusFinished",
        };
        w.text(key, &[])
    }
}

#[derive(Debug, Clone)]
pub struct WorkItem<'a> {
    /// The ticket's number, or `pr-<n>` for work known only by its PR.
    /// The `?t=` of the route.
    pub key: String,
    pub title: String,
    pub status: WorkStatus,
    pub agents: Vec<&'a Agent>,
    pub last_ms: u64,
    /// The newest line written for a person; None when nobody wrote one.
    pub line: Option<String>,
}

/// A finished piece of work stays this long, then leaves the list.
pub const WORK_RECENT_MS: u64 = 3 * 3_600_000;
/// Rows shown before "Show all".
pub const WORK_SHOWN: usize = 5;

fn is_word(c: Option<char>) -> bool {
    c.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The run of ASCII digits at the start of `s`.
fn digits(s: &str) -> &str {
    &s[..s.bytes().take_while(u8::is_ascii_digit).count()]
}

/// `/(?:AB#|lane-)(\d{5,6})\b/i`: the first ticket number in a text.
fn ticket(text: &str) -> Option<&str> {
    let lower = text.to_ascii_lowercase();
    for (i, _) in lower.char_indices() {
        let rest = &lower[i..];
        let after = rest
            .strip_prefix("ab#")
            .or_else(|| rest.strip_prefix("lane-"));
        let Some(after) = after else { continue };
        let n = digits(after);
        if (5..=6).contains(&n.len()) && !is_word(after[n.len()..].chars().next()) {
            let start = text.len() - after.len();
            return Some(&text[start..start + n.len()]);
        }
    }
    None
}

/// `/-(\d{5,6})$/`: a ticket number at the end of a branch.
fn branch_ticket(branch: &str) -> Option<&str> {
    let n = branch.bytes().rev().take_while(u8::is_ascii_digit).count();
    let (head, num) = branch.split_at(branch.len() - n);
    ((5..=6).contains(&n) && head.ends_with('-')).then_some(num)
}

/// `/\b(?:qa-|PR ?#?)(\d{4})\b/i`: every PR number in a text, in order.
fn pr_numbers(text: &str) -> Vec<&str> {
    let lower = text.to_ascii_lowercase();
    let mut out = Vec::new();
    for (i, _) in lower.char_indices() {
        if is_word(lower[..i].chars().next_back()) {
            continue;
        }
        let rest = &lower[i..];
        let after = if let Some(a) = rest.strip_prefix("qa-") {
            a
        } else if let Some(a) = rest.strip_prefix("pr") {
            let a = a.strip_prefix(' ').unwrap_or(a);
            a.strip_prefix('#').unwrap_or(a)
        } else {
            continue;
        };
        let n = digits(after);
        if n.len() == 4 && !is_word(after[4..].chars().next()) {
            let start = text.len() - after.len();
            out.push(&text[start..start + 4]);
        }
    }
    out
}

/// Which piece of work an agent is on, or None when it does not say:
/// a ticket in role, description or brief; else a branch ending in one;
/// else a PR (`pr-<n>`).
pub fn work_key(a: &Agent) -> Option<String> {
    let fields = [
        a.role.as_deref(),
        a.description.as_deref(),
        a.brief.as_deref(),
    ];
    if let Some(hit) = fields.iter().flatten().find_map(|f| ticket(f)) {
        return Some(hit.to_string());
    }
    if let Some(hit) = a.branch.as_deref().and_then(branch_ticket) {
        return Some(hit.to_string());
    }
    fields
        .iter()
        .flatten()
        .find_map(|f| pr_numbers(f).first().map(|n| format!("pr-{n}")))
}

/// `AB#123`/`QA` and a separator at the start of a description.
fn strip_ticket_prefix(text: &str) -> &str {
    let lower = text.to_ascii_lowercase();
    let after = if let Some(rest) = lower.strip_prefix("ab#") {
        let n = digits(rest).len();
        if n == 0 {
            return text;
        }
        &text[3 + n..]
    } else if lower.starts_with("qa") {
        &text[2..]
    } else {
        return text;
    };
    let after = after.trim_start();
    let after = after.strip_prefix([':', '·', '-']).unwrap_or(after);
    after.trim_start()
}

fn title_of(agents: &[&Agent], key: &str) -> String {
    for a in agents {
        if let Some(brief) = a.brief.as_deref() {
            let quoted = brief
                .split('«')
                .skip(1)
                .map(|rest| rest.split('»').next().unwrap_or(""))
                .find(|q| !q.is_empty());
            if let Some(inside) = quoted {
                return inside.trim().to_string();
            }
        }
    }
    if let Some(described) = agents
        .iter()
        .find_map(|a| a.description.as_deref().filter(|d| !d.is_empty()))
    {
        let plain = strip_ticket_prefix(described).trim();
        if !plain.is_empty() {
            return plain.to_string();
        }
    }
    match key.strip_prefix("pr-") {
        Some(n) => format!("PR {n}"),
        None => format!("AB#{key}"),
    }
}

/// Whether a message is about this piece of work: by one of its agents, or
/// naming its ticket or PR.
pub fn about_work(m: &Message, item: &WorkItem) -> bool {
    about(m, &item.key, &item.agents)
}

fn about(m: &Message, key: &str, agents: &[&Agent]) -> bool {
    if agents.iter().any(|a| a.key == m.agent) {
        return true;
    }
    match key.strip_prefix("pr-") {
        Some(n) => pr_numbers(&m.text).contains(&n),
        None => m.text.contains(&format!("AB#{key}")),
    }
}

/// Written for a person: it carries a label, or nothing flagged it.
fn reads_as_human(m: &Message) -> bool {
    m.from != "operator" && (m.label.is_some() || m.flags.is_empty())
}

/// The channel's pieces of work (`workItems`): what needs the operator
/// first, then blocked, working, paused, finished; newest first inside.
/// `stream` is the channel's stream ([`super::workspace::channel_stream`]).
pub fn work_items<'a>(
    view: &'a WorkspaceView,
    channel: &str,
    stream: &[&Message],
    now: u64,
) -> Vec<WorkItem<'a>> {
    let mut groups: Vec<(String, Vec<&Agent>)> = Vec::new();
    for a in view
        .agents
        .iter()
        .filter(|a| a.channels.iter().any(|c| c == channel))
    {
        let Some(key) = work_key(a) else { continue };
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.push(a),
            None => groups.push((key, vec![a])),
        }
    }
    let open = view
        .cards
        .iter()
        .filter(|c| c.needs && c.resolved_ms.is_none());
    let mut asking: HashSet<&str> = HashSet::new();
    let mut blocked: HashSet<&str> = HashSet::new();
    for c in open {
        let set = if c.kind == "blocked" {
            &mut blocked
        } else {
            &mut asking
        };
        set.extend(c.agents.iter().map(String::as_str));
    }
    let mut items = Vec::new();
    for (key, agents) in groups {
        let status = if agents.iter().any(|a| asking.contains(a.key.as_str())) {
            WorkStatus::Needs
        } else if agents.iter().any(|a| blocked.contains(a.key.as_str())) {
            WorkStatus::Blocked
        } else if agents.iter().any(|a| a.state == "running") {
            WorkStatus::Working
        } else if agents.iter().any(|a| a.state == "idle") {
            WorkStatus::Paused
        } else {
            WorkStatus::Finished
        };
        let last_ms = agents
            .iter()
            .map(|a| a.last_action_ms.unwrap_or(0))
            .max()
            .unwrap_or(0);
        if status == WorkStatus::Finished && now.saturating_sub(last_ms) > WORK_RECENT_MS {
            continue;
        }
        let line = stream
            .iter()
            .rev()
            .find(|m| about(m, &key, &agents) && reads_as_human(m))
            .map(|m| m.text.clone());
        items.push(WorkItem {
            title: title_of(&agents, &key),
            key,
            status,
            agents,
            last_ms,
            line,
        });
    }
    items.sort_by(|a, b| a.status.cmp(&b.status).then(b.last_ms.cmp(&a.last_ms)));
    items
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derive::workspace::test_words::en;
    use crate::model::Card;

    const NOW: u64 = 1_794_086_000_000;

    fn agent(key: &str) -> Agent {
        Agent {
            key: key.into(),
            name: key.into(),
            state: "running".into(),
            channels: vec!["northwind".into()],
            last_action_ms: Some(NOW),
            ..Default::default()
        }
    }

    fn with(key: &str, f: impl FnOnce(&mut Agent)) -> Agent {
        let mut a = agent(key);
        f(&mut a);
        a
    }

    fn message(id: &str, who: &str) -> Message {
        Message {
            id: id.into(),
            agent: who.into(),
            from: "agent".into(),
            text: format!("text {id}"),
            ..Default::default()
        }
    }

    fn view(agents: Vec<Agent>, cards: Vec<Card>) -> WorkspaceView {
        WorkspaceView {
            agents,
            cards,
            ..Default::default()
        }
    }

    fn card(kind: &str, agent: &str) -> Card {
        Card {
            kind: kind.into(),
            needs: true,
            agents: vec![agent.into()],
            ..Default::default()
        }
    }

    #[test]
    fn the_ticket_wherever_the_agent_carries_it() {
        assert_eq!(
            work_key(&with("a", |a| a.role = Some("lane-100001".into()))).as_deref(),
            Some("100001")
        );
        assert_eq!(
            work_key(&with("a", |a| a.description =
                Some("AB#100002 expired owner invite".into())))
            .as_deref(),
            Some("100002")
        );
        assert_eq!(
            work_key(&with("a", |a| a.branch =
                Some("fix/no-workspace-page-288834".into())))
            .as_deref(),
            Some("288834")
        );
    }

    #[test]
    fn a_ticket_beats_a_pr_a_pr_alone_still_groups_nothing_is_nothing() {
        let both = with("a", |a| {
            a.role = Some("qa-4001".into());
            a.brief = Some("Test AB#100003 in the PR".into());
        });
        assert_eq!(work_key(&both).as_deref(), Some("100003"));
        assert_eq!(
            work_key(&with("a", |a| a.role = Some("qa-4001".into()))).as_deref(),
            Some("pr-4001")
        );
        assert_eq!(
            work_key(&with("a", |a| a.role = Some("detail".into()))),
            None
        );
        assert_eq!(
            work_key(&with("a", |a| a.role = Some("AB#1234567".into()))),
            None
        );
        assert_eq!(
            work_key(&with("a", |a| a.brief = Some("see PR #4242.".into()))).as_deref(),
            Some("pr-4242")
        );
    }

    fn lane() -> Agent {
        with("a", |a| {
            a.role = Some("lane-100001".into());
            a.brief = Some("Work on AB#100001 («Invitations can be seen»), P1".into());
        })
    }

    #[test]
    fn takes_the_tickets_own_title_and_joins_the_agents_on_one_ticket() {
        let qa = with("b", |a| {
            a.role = Some("qa-4001".into());
            a.brief = Some("Review AB#100001".into());
            a.state = "idle".into();
        });
        let v = view(vec![lane(), qa], vec![]);
        let items = work_items(&v, "northwind", &[], NOW);
        assert_eq!(items[0].title, "Invitations can be seen");
        assert_eq!(
            items[0]
                .agents
                .iter()
                .map(|a| a.key.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(items[0].status, WorkStatus::Working);
    }

    #[test]
    fn without_a_quoted_title_the_description_or_the_key() {
        let d = with("d", |a| {
            a.description = Some("AB#100005 · Fix the invite".into())
        });
        let v = view(
            vec![d, with("p", |a| a.role = Some("qa-4002".into()))],
            vec![],
        );
        let items = work_items(&v, "northwind", &[], NOW);
        let titles: Vec<&str> = items.iter().map(|i| i.title.as_str()).collect();
        assert!(titles.contains(&"Fix the invite"));
        assert!(titles.contains(&"PR 4002"));
    }

    #[test]
    fn the_line_is_the_newest_one_written_for_a_person() {
        let mut done = message("1", "a");
        done.label = Some("DONE".into());
        done.text = "Invitations now show.".into();
        let mut chatter = message("2", "a");
        chatter.flags = vec!["unlabeled".into(), "jargon".into()];
        chatter.text = "rebased on 57442e7".into();
        let v = view(vec![lane()], vec![]);
        let items = work_items(&v, "northwind", &[&done, &chatter], NOW);
        assert_eq!(items[0].line.as_deref(), Some("Invitations now show."));
    }

    #[test]
    fn what_needs_them_comes_first_and_old_finished_work_leaves() {
        let a = with("a", |a| a.role = Some("lane-111111".into()));
        let b = with("b", |a| {
            a.role = Some("lane-222222".into());
            a.state = "done".into();
            a.last_action_ms = Some(NOW - 1000);
        });
        let c = with("c", |a| {
            a.role = Some("lane-333333".into());
            a.state = "done".into();
            a.last_action_ms = Some(NOW - WORK_RECENT_MS - 1);
        });
        let d = with("d", |a| {
            a.role = Some("lane-444444".into());
            a.state = "idle".into();
        });
        let v = view(vec![a, b, c, d], vec![card("mention", "d")]);
        let got: Vec<(String, WorkStatus)> = work_items(&v, "northwind", &[], NOW)
            .into_iter()
            .map(|i| (i.key, i.status))
            .collect();
        assert_eq!(
            got,
            [
                ("444444".to_string(), WorkStatus::Needs),
                ("111111".to_string(), WorkStatus::Working),
                ("222222".to_string(), WorkStatus::Finished)
            ]
        );
    }

    #[test]
    fn a_block_is_said_as_a_block_below_what_asks_for_them() {
        let a = with("a", |a| a.role = Some("lane-111111".into()));
        let b = with("b", |a| a.role = Some("lane-222222".into()));
        let v = view(vec![a, b], vec![card("blocked", "a"), card("mention", "b")]);
        let got: Vec<WorkStatus> = work_items(&v, "northwind", &[], NOW)
            .iter()
            .map(|i| i.status)
            .collect();
        assert_eq!(got, [WorkStatus::Needs, WorkStatus::Blocked]);
        assert_eq!(WorkStatus::Needs.label(&en()), "Needs you");
        assert_eq!(WorkStatus::Working.tone(), "green");
    }

    #[test]
    fn a_message_belongs_by_its_author_or_by_naming_the_ticket() {
        let lane = lane();
        let item = WorkItem {
            key: "100001".into(),
            title: String::new(),
            status: WorkStatus::Working,
            agents: vec![&lane],
            last_ms: 0,
            line: None,
        };
        assert!(about_work(&message("1", "a"), &item));
        let mut named = message("2", "z");
        named.text = "AB#100001: ready".into();
        assert!(about_work(&named, &item));
        assert!(!about_work(&message("3", "z"), &item));
        let pr = WorkItem {
            key: "pr-4001".into(),
            agents: vec![],
            ..item
        };
        let mut m = message("4", "z");
        m.text = "qa-4001 passes".into();
        assert!(about_work(&m, &pr));
    }
}
