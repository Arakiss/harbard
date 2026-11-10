//! Ported from the console's `lib/members.ts`: the channel header's members,
//! how many worked in a channel, how many work there now, and the popover's
//! order. "Now" is serve's `working` without a dormant mark: a lost or
//! dormant agent does not count.

use super::workspace::{Words, agent_role_line, collate, local_role};
use crate::model::{Agent, Channel, WorkspaceView};
use gpui::SharedString;

#[derive(Debug, Default, Clone)]
pub struct Members<'a> {
    /// Working now, newest first.
    pub working: Vec<&'a Agent>,
    /// The others, by last activity.
    pub rest: Vec<&'a Agent>,
    /// `working` then `rest`: the popover's order.
    pub ordered: Vec<&'a Agent>,
    /// The stacked avatars: the first three of `ordered`.
    pub stack: Vec<&'a Agent>,
    pub total: usize,
    /// How many work here now.
    pub now: usize,
}

pub fn is_working_now(a: &Agent) -> bool {
    a.presence == "working" && !a.lost && a.dormant.is_none()
}

/// `members` in `lib/workspace.ts`: the channel's member keys that are in
/// the view.
pub fn channel_agents<'a>(view: &'a WorkspaceView, channel: &Channel) -> Vec<&'a Agent> {
    channel
        .members
        .iter()
        .filter_map(|k| view.agent(k))
        .collect()
}

/// Teammates in no channel of their own count in their lead's channel.
fn unplaced<'a>(view: &'a WorkspaceView, channel: &Channel) -> Vec<&'a Agent> {
    view.agents
        .iter()
        .filter(|a| {
            if !a.channels.is_empty()
                || a.lead_session.is_empty()
                || channel.members.contains(&a.key)
            {
                return false;
            }
            let lead = format!("session:{}", a.lead_session);
            view.agent(&lead)
                .is_some_and(|l| l.channels.contains(&channel.id))
        })
        .collect()
}

/// `channelMembers`. The header hides the button when `total` is 0.
pub fn channel_members<'a>(view: &'a WorkspaceView, channel: &Channel) -> Members<'a> {
    let mut all = channel_agents(view, channel);
    all.extend(unplaced(view, channel));
    let by_last = |a: &&Agent, b: &&Agent| {
        b.last_action_ms
            .unwrap_or(0)
            .cmp(&a.last_action_ms.unwrap_or(0))
            .then_with(|| collate(&a.name, &b.name))
    };
    let (mut working, mut rest): (Vec<&Agent>, Vec<&Agent>) =
        all.into_iter().partition(|a| is_working_now(a));
    working.sort_by(by_last);
    rest.sort_by(by_last);
    let ordered: Vec<&Agent> = working.iter().chain(rest.iter()).copied().collect();
    let stack = ordered.iter().take(3).copied().collect();
    Members {
        total: ordered.len(),
        now: working.len(),
        working,
        rest,
        ordered,
        stack,
    }
}

/// "43 agents have worked in #channel · 5 now": the button's tooltip, its
/// accessible name and the popover's title.
pub fn members_label(members: &Members, channel: &str, w: &impl Words) -> SharedString {
    let who = if members.total == 1 {
        w.text("workspace.membersWorkedOne", &[("channel", channel)])
    } else {
        w.text(
            "workspace.membersWorkedMany",
            &[("n", &members.total.to_string()), ("channel", channel)],
        )
    };
    let now = if members.now == 0 {
        w.text("workspace.membersNowNone", &[])
    } else {
        w.text(
            "workspace.membersNowCount",
            &[("n", &members.now.to_string())],
        )
    };
    format!("{who} · {now}").into()
}

/// "Working now (n)" and "Earlier (n)", the popover's group heads.
pub fn members_group_titles(members: &Members, w: &impl Words) -> (SharedString, SharedString) {
    (
        w.text(
            "workspace.membersWorkingNow",
            &[("n", &members.working.len().to_string())],
        ),
        w.text(
            "workspace.membersEarlier",
            &[("n", &members.rest.len().to_string())],
        ),
    )
}

/// A member row's second line (`memberRole`): its workshop line, else role
/// and team, else role, else serve's label.
pub fn member_role(a: &Agent, w: &impl Words) -> SharedString {
    if a.role_line.as_deref().is_some_and(|l| !l.is_empty()) {
        return agent_role_line(a, w);
    }
    let role: Option<String> = match a.role.as_deref() {
        Some("lead") => Some(w.text("workspace.leadRole", &[]).to_string()),
        Some(r) if !r.is_empty() => Some(local_role(r, w)),
        _ => None,
    };
    match (role, a.team.as_deref()) {
        (Some(role), Some(team)) if !team.is_empty() => format!("{role} · {team}").into(),
        (Some(role), _) => role.into(),
        (None, _) => a.label.clone().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::derive::workspace::test_words::en;

    const MIN: u64 = 60_000;
    const NOW: u64 = 1_794_078_000_000;

    fn agent(key: &str, presence: &str, last: u64) -> Agent {
        Agent {
            key: key.into(),
            name: key.into(),
            presence: presence.into(),
            last_action_ms: Some(NOW - last * MIN),
            label: format!("general-purpose · {key}"),
            ..Default::default()
        }
    }

    fn fixture() -> (WorkspaceView, Channel) {
        let mut rivet = agent("rivet", "working", 5);
        rivet.role = Some("constructor".into());
        rivet.team = Some("harbard".into());
        rivet.role_line = Some("constructor · harbard".into());
        let agents = vec![
            agent("brush", "finished", 90),
            rivet,
            agent("lima", "waiting", 2),
            agent("nut", "working", 1),
            agent("coil", "finished", 300),
        ];
        let channel = Channel {
            id: "harbard".into(),
            repo: "/r/harbard".into(),
            members: ["brush", "rivet", "lima", "nut", "coil", "gone"]
                .map(String::from)
                .to_vec(),
            ..Default::default()
        };
        let view = WorkspaceView {
            now_ms: NOW,
            agents,
            channels: vec![channel.clone()],
            ..Default::default()
        };
        (view, channel)
    }

    fn keys(list: &[&Agent]) -> Vec<String> {
        list.iter().map(|a| a.key.clone()).collect()
    }

    #[test]
    fn a_teammate_in_no_channel_of_its_own_counts_in_its_leads() {
        let (mut view, channel) = fixture();
        let mut lead = agent("session:lead", "waiting", 1);
        lead.channels = vec!["harbard".into()];
        let mut mate = agent("hammer", "working", 1);
        mate.lead_session = "lead".into();
        let mut elsewhere = agent("otra", "working", 1);
        elsewhere.lead_session = "nobody".into();
        view.agents.extend([lead, mate, elsewhere]);
        let mut room = channel.clone();
        room.members.push("session:lead".into());
        let m = channel_members(&view, &room);
        assert_eq!(m.now, 3);
        assert!(keys(&m.ordered).contains(&"hammer".to_string()));
        assert!(!keys(&m.ordered).contains(&"otra".to_string()));
    }

    #[test]
    fn counts_every_agent_that_worked_here_and_the_ones_working_now() {
        let (view, channel) = fixture();
        let m = channel_members(&view, &channel);
        assert_eq!((m.total, m.now), (5, 2));
        assert_eq!(
            members_label(&m, "harbard", &en()),
            "5 agents have worked in #harbard · 2 now"
        );
    }

    #[test]
    fn singular_and_nobody_working_now() {
        let (view, channel) = fixture();
        let one = Channel {
            members: vec!["brush".into()],
            ..channel
        };
        let m = channel_members(&view, &one);
        assert_eq!(
            members_label(&m, "harbard", &en()),
            "1 agent has worked in #harbard · none now"
        );
        assert_eq!(m.now, 0);
    }

    #[test]
    fn working_first_newest_first_then_the_rest_and_the_stack_is_the_first_three() {
        let (view, channel) = fixture();
        let m = channel_members(&view, &channel);
        assert_eq!(keys(&m.ordered), ["nut", "rivet", "lima", "brush", "coil"]);
        assert_eq!(keys(&m.stack), ["nut", "rivet", "lima"]);
    }

    #[test]
    fn a_lost_or_dormant_agent_is_not_working_whatever_presence_says() {
        let mut x = agent("x", "working", 1);
        x.lost = true;
        let mut y = agent("y", "working", 50);
        y.dormant = Some("amber".into());
        let view = WorkspaceView {
            now_ms: NOW,
            agents: vec![x, y],
            ..Default::default()
        };
        let ch = Channel {
            id: "harbard".into(),
            members: vec!["x".into(), "y".into()],
            ..Default::default()
        };
        let m = channel_members(&view, &ch);
        assert_eq!(m.now, 0);
        assert_eq!(
            members_label(&m, "harbard", &en()),
            "2 agents have worked in #harbard · none now"
        );
    }

    #[test]
    fn role_and_team_for_each_row_from_the_workshop_name_or_its_type() {
        let (view, _) = fixture();
        assert_eq!(member_role(&view.agents[1], &en()), "constructor · harbard");
        let mut t = agent("t", "working", 1);
        t.role = Some("revisora".into());
        t.team = Some("protocol".into());
        assert_eq!(member_role(&t, &en()), "revisora · protocol");
        assert_eq!(
            member_role(&view.agents[0], &en()),
            "general-purpose · brush"
        );
    }
}
