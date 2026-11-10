//! "While you were away", calm mode and the desktop's away suggestion
//! (`lib/away.ts`, `lib/calm.ts`, `lib/suggest.ts`): what waits for the
//! operator by urgency, what finished, who still works, the one thing calm
//! mode shows, and which suggestion makes sense in the state in force.

use crate::model::profile::DigestItem;
use crate::model::{Agent, Card, WorkspaceView};

const MIN: u64 = 60_000;

/// Needs you's order: blocked first, dormant last (`URGENCY`).
pub fn urgency(kind: &str) -> u8 {
    match kind {
        "blocked" => 0,
        "check" | "approval" => 1,
        "handoff" => 2,
        "warning" => 3,
        "mention" => 4,
        "ask" => 5,
        "unanswered" => 6,
        "two_writers" => 7,
        "dormant" => 8,
        _ => 9,
    }
}

/// The open cards (`openCards`).
pub fn open_cards(view: &WorkspaceView) -> impl Iterator<Item = &Card> {
    view.cards.iter().filter(|c| c.needs && c.resolved_ms.is_none())
}

/// Needs you: the open cards by urgency, the newest first within a kind.
pub fn needs_items(view: &WorkspaceView) -> Vec<&Card> {
    let mut out: Vec<&Card> = open_cards(view).collect();
    out.sort_by(|a, b| urgency(&a.kind).cmp(&urgency(&b.kind)).then(b.since_ms.cmp(&a.since_ms)));
    out
}

/// The dictionary key of a kind's reason (`home.reason*`).
pub fn reason_key(kind: &str) -> &'static str {
    match kind {
        "blocked" => "home.reasonBlocked",
        "approval" => "home.reasonApproval",
        "handoff" => "home.reasonHandoff",
        "warning" => "home.reasonWarning",
        "ask" => "home.reasonAsk",
        "mention" => "home.reasonMention",
        "unanswered" => "home.reasonUnanswered",
        "two_writers" => "home.reasonTwoWriters",
        "dormant" => "home.reasonDormant",
        _ => "home.reasonCheck",
    }
}

/// The chip's tone: red, amber or neutral.
pub fn tone(kind: &str) -> &'static str {
    match kind {
        "blocked" => "red",
        "approval" | "handoff" | "warning" | "mention" | "check" => "amber",
        _ => "neutral",
    }
}

/// Nobody reads this agent now: lost, or its lead is off the bus or ended.
pub fn session_gone(a: &Agent) -> bool {
    a.lost || a.delivery == "not_on_bus" || a.delivery == "ended"
}

#[derive(Debug, Clone, PartialEq)]
pub struct Waiting {
    /// `card:<id>` or `note:<id>`.
    pub id: String,
    pub kind: String,
    /// The agent's name, or a short session id.
    pub who: String,
    /// The agent's key when the workspace knows it.
    pub agent: Option<String>,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Done {
    pub id: String,
    pub who: String,
    /// `None` for a verified delivery: the text is `home.deliveryVerified` with `branch`.
    pub text: String,
    pub branch: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AwayModel {
    pub waiting: Vec<Waiting>,
    pub done: Vec<Done>,
    /// Agent keys at work.
    pub running: Vec<String>,
}

fn first_line(text: &str) -> String {
    text.split('\n').next().unwrap_or("").trim().to_string()
}

/// The agent a bus session speaks for.
fn agent_of<'a>(view: Option<&'a WorkspaceView>, session: &str) -> Option<&'a Agent> {
    view?.agents.iter().find(|a| a.lead_session == session || a.key == session)
}

/// `awayModel`: waiting by urgency, finished since `since`, still running.
pub fn away_model(view: Option<&WorkspaceView>, held: &[DigestItem], since: u64, now: u64) -> AwayModel {
    let mut waiting = Vec::new();
    if let Some(v) = view {
        for c in needs_items(v) {
            let first = c.agents.first();
            let a = first.and_then(|k| v.agent(k));
            let text = c.message.as_deref().and_then(|id| v.message(id)).map(|m| first_line(&m.text)).unwrap_or_default();
            waiting.push(Waiting {
                id: format!("card:{}", c.id),
                kind: c.kind.clone(),
                who: a.map(|a| a.name.clone()).or_else(|| first.cloned()).unwrap_or_else(|| "harbard".into()),
                agent: a.map(|a| a.key.clone()),
                text,
            });
        }
    }
    let mut done = Vec::new();
    for n in held {
        let a = agent_of(view, &n.session);
        let who = a.map(|a| a.name.clone()).unwrap_or_else(|| n.session.chars().take(8).collect());
        if matches!(n.kind.as_str(), "handoff" | "warning" | "ask") {
            waiting.push(Waiting {
                id: format!("note:{}", n.id),
                kind: n.kind.clone(),
                who,
                agent: a.map(|a| a.key.clone()),
                text: n.text.clone(),
            });
        } else {
            done.push(Done {
                id: format!("note:{}", n.id),
                who,
                text: n.text.clone(),
                branch: None,
            });
        }
    }
    // Stable: Needs you's own order holds within a kind.
    waiting.sort_by_key(|w| urgency(&w.kind));
    if let Some(v) = view {
        for m in &v.messages {
            if m.label.as_deref() != Some("DONE") || m.ts_ms < since || m.ts_ms > now {
                continue;
            }
            done.push(Done {
                id: format!("msg:{}", m.id),
                who: v.agent(&m.agent).map(|a| a.name.clone()).unwrap_or_else(|| m.agent.clone()),
                text: first_line(&m.text),
                branch: None,
            });
        }
        for d in &v.deliveries {
            if d.verdict != "landed" || d.decided_ms.is_none_or(|ms| ms < since) {
                continue;
            }
            done.push(Done {
                id: format!("delivery:{}", d.id),
                who: d.channel.as_ref().map(|c| format!("#{c}")).unwrap_or_default(),
                text: String::new(),
                branch: Some(d.branch.clone()),
            });
        }
    }
    let running = view
        .map(|v| v.agents.iter().filter(|a| a.presence == "working" && !a.lost).map(|a| a.key.clone()).collect())
        .unwrap_or_default();
    AwayModel { waiting, done, running }
}

/// "1 h 12 min", or `None` under a minute.
pub fn away_span(since: u64, now: u64) -> Option<String> {
    let m = (now.saturating_sub(since) as f64 / MIN as f64).round() as u64;
    if m < 1 {
        return None;
    }
    if m < 60 {
        return Some(format!("{m} min"));
    }
    if m < 24 * 60 {
        return Some(if m.is_multiple_of(60) { format!("{} h", m / 60) } else { format!("{} h {} min", m / 60, m % 60) });
    }
    Some(format!("{} h", (m as f64 / 60.0).round() as u64))
}

/// What calm mode's card says about the top item.
#[derive(Debug, Clone, PartialEq)]
pub enum CalmTitle {
    /// The operator's own message nobody answered: quoted as theirs.
    Own { quote: String, ts_ms: u64 },
    /// The first line of what it asks (empty = the reason), and the rest.
    /// `gone`: a blocked card whose agent's session is gone.
    Ask { title: String, detail: String, gone: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub struct CalmModel {
    pub top: Option<(Waiting, CalmTitle)>,
    /// Open items, the top one included.
    pub waiting: usize,
    /// Agents at work.
    pub working: usize,
}

/// `calmModel`: the first waiting item not put off with "Later".
pub fn calm_model(view: Option<&WorkspaceView>, later: &[String], now: u64) -> CalmModel {
    let m = away_model(view, &[], now, now);
    let next = m.waiting.iter().find(|w| !later.contains(&w.id)).cloned();
    let top = next.map(|w| {
        let id = w.id.trim_start_matches("card:");
        let card = view.and_then(|v| v.cards.iter().find(|c| c.id == id));
        let msg = card.and_then(|c| c.message.as_deref()).and_then(|mid| view.and_then(|v| v.message(mid)));
        let text = if card.is_some_and(|c| c.message.is_some()) {
            msg.map(|m| m.text.clone()).unwrap_or_default()
        } else {
            w.text.clone()
        };
        let title = match msg {
            Some(m) if m.from == "operator" => CalmTitle::Own {
                quote: text.trim().to_string(),
                ts_ms: m.ts_ms,
            },
            _ => {
                let mut lines = text.split('\n');
                let first = lines.next().unwrap_or("").trim().to_string();
                let rest = lines.collect::<Vec<_>>().join(" ").trim().to_string();
                let agent = w.agent.as_deref().and_then(|k| view.and_then(|v| v.agent(k)));
                let gone = card.is_some_and(|c| c.kind == "blocked") && agent.is_some_and(session_gone);
                CalmTitle::Ask {
                    title: first,
                    detail: if gone { String::new() } else { rest },
                    gone,
                }
            }
        };
        (w, title)
    });
    CalmModel {
        top,
        waiting: m.waiting.len(),
        working: m.running.len(),
    }
}

/// `suggestionFor`: the state to offer, or `None` when it makes no sense.
/// `target` is "away" or "available"; returns the target back when it fits.
pub fn suggestion_for(target: &str, current: &str, enabled: bool) -> Option<&'static str> {
    if !enabled {
        return None;
    }
    match target {
        "away" if current != "away" && current != "done" => Some("away"),
        "available" if current == "away" => Some("available"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DeliveryRow, Message};

    fn card(id: &str, kind: &str, since: u64) -> Card {
        Card {
            id: id.into(),
            kind: kind.into(),
            needs: true,
            since_ms: since,
            agents: vec!["a1".into()],
            ..Default::default()
        }
    }

    fn agent(key: &str, name: &str) -> Agent {
        Agent {
            key: key.into(),
            name: name.into(),
            ..Default::default()
        }
    }

    #[test]
    fn needs_go_by_urgency_then_newest() {
        let mut resolved = card("r", "blocked", 9);
        resolved.resolved_ms = Some(10);
        let v = WorkspaceView {
            cards: vec![card("m", "mention", 5), card("b1", "blocked", 1), card("b2", "blocked", 3), resolved, card("d", "dormant", 9)],
            ..Default::default()
        };
        let ids: Vec<&str> = needs_items(&v).iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["b2", "b1", "m", "d"]);
    }

    #[test]
    fn held_notes_slot_in_by_kind_and_the_rest_is_done() {
        let mut a = agent("a1", "Ada");
        a.lead_session = "sess-123456789".into();
        a.presence = "working".into();
        let mut lost = agent("a2", "Lost");
        lost.presence = "working".into();
        lost.lost = true;
        let v = WorkspaceView {
            agents: vec![a, lost],
            cards: vec![card("c", "unanswered", 1)],
            messages: vec![
                Message { id: "m1".into(), agent: "a1".into(), label: Some("DONE".into()), ts_ms: 50, text: "Shipped\nmore".into(), ..Default::default() },
                Message { id: "m2".into(), agent: "a1".into(), label: Some("DONE".into()), ts_ms: 5, ..Default::default() },
            ],
            deliveries: vec![DeliveryRow { id: "d".into(), verdict: "landed".into(), decided_ms: Some(60), branch: "feat/x".into(), channel: Some("harbard".into()), ..Default::default() }],
            ..Default::default()
        };
        let held = [
            DigestItem { id: "n1".into(), session: "sess-123456789".into(), kind: "handoff".into(), text: "take it".into(), ..Default::default() },
            DigestItem { id: "n2".into(), session: "abcdefghijk".into(), kind: "fyi".into(), text: "fyi".into(), ..Default::default() },
        ];
        let m = away_model(Some(&v), &held, 10, 100);
        let ids: Vec<&str> = m.waiting.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, ["note:n1", "card:c"]);
        assert_eq!(m.waiting[0].who, "Ada");
        let done: Vec<(&str, &str)> = m.done.iter().map(|d| (d.id.as_str(), d.who.as_str())).collect();
        assert_eq!(done, [("note:n2", "abcdefgh"), ("msg:m1", "Ada"), ("delivery:d", "#harbard")]);
        assert_eq!(m.done[1].text, "Shipped");
        assert_eq!(m.running, ["a1"]);
    }

    #[test]
    fn spans_read_like_the_console() {
        assert_eq!(away_span(0, 20_000), None);
        assert_eq!(away_span(0, 5 * MIN), Some("5 min".into()));
        assert_eq!(away_span(0, 120 * MIN), Some("2 h".into()));
        assert_eq!(away_span(0, 72 * MIN), Some("1 h 12 min".into()));
        assert_eq!(away_span(0, 30 * 60 * MIN), Some("30 h".into()));
    }

    #[test]
    fn calm_quotes_the_operators_own_message_and_skips_later() {
        let mut c1 = card("c1", "blocked", 2);
        c1.message = Some("m1".into());
        let mut c2 = card("c2", "unanswered", 1);
        c2.message = Some("m2".into());
        let mut a = agent("a1", "Ada");
        a.delivery = "ended".into();
        let v = WorkspaceView {
            agents: vec![a],
            cards: vec![c1, c2],
            messages: vec![
                Message { id: "m1".into(), from: "agent".into(), text: "Can I push?\nIt is red".into(), ..Default::default() },
                Message { id: "m2".into(), from: "operator".into(), text: " ship it ".into(), ts_ms: 7, ..Default::default() },
            ],
            ..Default::default()
        };
        let m = calm_model(Some(&v), &[], 100);
        let (w, title) = m.top.clone().unwrap();
        assert_eq!(w.id, "card:c1");
        assert_eq!(title, CalmTitle::Ask { title: "Can I push?".into(), detail: String::new(), gone: true });
        assert_eq!(m.waiting, 2);
        let m = calm_model(Some(&v), &["card:c1".into()], 100);
        assert_eq!(m.top.unwrap().1, CalmTitle::Own { quote: "ship it".into(), ts_ms: 7 });
    }

    #[test]
    fn suggestions_only_where_they_make_sense() {
        assert_eq!(suggestion_for("away", "available", true), Some("away"));
        assert_eq!(suggestion_for("away", "away", true), None);
        assert_eq!(suggestion_for("away", "done", true), None);
        assert_eq!(suggestion_for("available", "away", true), Some("available"));
        assert_eq!(suggestion_for("available", "focused", true), None);
        assert_eq!(suggestion_for("away", "available", false), None);
    }
}
