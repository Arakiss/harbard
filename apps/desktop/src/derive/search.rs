//! Search over what the view holds (`lib/workspace.ts` `search`): agents,
//! messages and reports, lowercased substring matches.

use crate::derive::direct::fold;
use crate::model::{Agent, Message, WorkspaceView};
use std::collections::HashSet;

#[derive(Default)]
pub struct Results<'a> {
    pub agents: Vec<&'a Agent>,
    pub messages: Vec<&'a Message>,
    /// A report path and the first message that named it.
    pub reports: Vec<(&'a str, &'a Message)>,
}

impl Results<'_> {
    pub fn total(&self) -> usize {
        self.agents.len() + self.messages.len() + self.reports.len()
    }
}

pub fn search<'a>(view: &'a WorkspaceView, query: &str) -> Results<'a> {
    let q = fold(query.trim());
    if q.is_empty() {
        return Results::default();
    }
    let has = |s: Option<&str>| s.is_some_and(|s| fold(s).contains(&q));
    let agents = view
        .agents
        .iter()
        .filter(|a| !a.temporary)
        .filter(|a| {
            has(Some(&a.name)) || has(a.handle.as_deref()) || has(a.team.as_deref()) || has(a.description.as_deref()) || has(a.brief.as_deref())
        })
        .collect();
    let mut messages: Vec<&Message> = view
        .messages
        .iter()
        .filter(|m| m.structured.is_none() && (has(Some(&m.text)) || has(Some(&m.author))))
        .collect();
    messages.sort_by(|a, b| b.ts_ms.cmp(&a.ts_ms));
    let mut seen = HashSet::new();
    let mut reports = Vec::new();
    for m in &view.messages {
        for f in &m.files {
            if has(Some(f)) && seen.insert(f.as_str()) {
                reports.push((f.as_str(), m));
            }
        }
    }
    Results { agents, messages, reports }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_agents_messages_and_reports_case_blind() {
        let view = WorkspaceView {
            agents: vec![
                Agent { key: "a".into(), name: "Adrian".into(), ..Default::default() },
                Agent { key: "t".into(), name: "Adrián temp".into(), temporary: true, ..Default::default() },
                Agent { key: "b".into(), name: "Barry".into(), brief: Some("fix the ADR parser".into()), ..Default::default() },
            ],
            messages: vec![
                Message { id: "1".into(), text: "old adr".into(), ts_ms: 1, files: vec!["/r/ADR.md".into()], ..Default::default() },
                Message { id: "2".into(), text: "new ADR".into(), ts_ms: 2, files: vec!["/r/ADR.md".into()], ..Default::default() },
                Message { id: "3".into(), text: "adr record".into(), ts_ms: 3, structured: Some("x".into()), ..Default::default() },
            ],
            ..Default::default()
        };
        let r = search(&view, "  adr ");
        assert_eq!(r.agents.iter().map(|a| a.key.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        assert_eq!(r.messages.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(), ["2", "1"]);
        assert_eq!(r.reports.len(), 1);
        assert_eq!(r.reports[0].1.id, "1");
        assert_eq!(search(&view, "  ").total(), 0);
    }
}
