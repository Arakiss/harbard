//! The living logo's state, read from the app's own live slice: the console's
//! `logoState` (`lib/activity.ts`) and the one-shots `useLogo` played. The
//! Dock tile, the menu-bar mark and the Strip's mark all show this, and the
//! shell's living logo can call [`logo_state`] too.

use super::dock::{LogoState, OneShot};
use crate::desk::{Connection, Desk};
use crate::model::ActivityView;
use std::collections::HashMap;

/// Any agent active within this makes the logo "working".
const WORKING_MS: u64 = 60_000;

/// States in which an agent is at work even while its transcript is quiet.
pub const WORKING_STATES: [&str; 7] = [
    "thinking",
    "running",
    "testing",
    "editing",
    "reading",
    "web",
    "delegating",
];

/// The logo's lasting state, by precedence: no bus, still connecting, an
/// open BLOCKED, anything else in "Needs you", an agent active in the last
/// minute or in a working state now, else rest.
pub fn state_of(connection: Connection, activity: Option<&ActivityView>, now: u64) -> LogoState {
    match connection {
        Connection::Disconnected => return LogoState::Disconnected,
        Connection::Connecting => return LogoState::Connecting,
        Connection::Connected => {}
    }
    let Some(a) = activity else {
        return LogoState::Connecting;
    };
    if a.blocked > 0 {
        return LogoState::Blocked;
    }
    if a.needs > 0 {
        return LogoState::Needs;
    }
    let working = a.agents.iter().any(|ag| {
        WORKING_STATES.contains(&ag.activity.state.as_str())
            || ag.last_action_ms.is_some_and(|ms| now.saturating_sub(ms) <= WORKING_MS)
    });
    if working { LogoState::Working } else { LogoState::Idle }
}

/// The logo's state from the desk, as every surface shows it.
pub fn logo_state(desk: &Desk) -> LogoState {
    state_of(desk.connection(), desk.activity.as_deref(), desk.now_ms())
}

/// "Needs you" across the hub: this workspace's plus the other workspaces'
/// tiles (the Dock is the hub's, not one workspace's).
pub fn hub_needs(desk: &Desk) -> u64 {
    let here = desk.activity.as_ref().map_or(0, |a| a.needs);
    let elsewhere: u64 = desk
        .view
        .as_ref()
        .map_or(0, |v| v.workspaces.iter().filter(|t| !t.current).map(|t| t.needs).sum());
    here + elsewhere
}

/// Turns each new reaction (after the first reading) into one one-shot: a
/// `hop` is a verified delivery, a `glance` a message. Reactions the first
/// reading already carried are history and never play.
#[derive(Default)]
pub struct OneShots {
    baseline: Option<u64>,
    seen: HashMap<String, u64>,
}

impl OneShots {
    pub fn next(&mut self, activity: &ActivityView) -> Option<OneShot> {
        let Some(baseline) = self.baseline else {
            self.baseline = Some(activity.now_ms);
            return None;
        };
        let mut out = None;
        for agent in &activity.agents {
            for r in &agent.reactions {
                if r.at_ms <= baseline {
                    continue;
                }
                let id = format!("{}:{}:{}", agent.key, r.kind, r.at_ms);
                if self.seen.insert(id, r.at_ms).is_some() {
                    continue;
                }
                match r.kind.as_str() {
                    "hop" => out = Some(OneShot::Verified),
                    "glance" if out.is_none() => out = Some(OneShot::Message),
                    _ => {}
                }
            }
        }
        // Forget what is long past, so the set stays small.
        let horizon = activity.now_ms.saturating_sub(10 * 60_000);
        self.seen.retain(|_, at| *at >= horizon);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn activity(blocked: u64, needs: u64, agents: serde_json::Value, now: u64) -> ActivityView {
        serde_json::from_value(json!({
            "v": 1, "now_ms": now, "thresholds": { "amber_ms": 1, "red_ms": 2, "thinking_long_ms": 3, "waiting_sleepy_ms": 4, "running_long_ms": 5 },
            "agents": agents, "unread": { "directs": 0, "activity": 0 },
            "needs": needs, "directs": 0, "blocked": blocked,
        }))
        .unwrap()
    }

    fn agent(state: &str, last: Option<u64>, reactions: serde_json::Value) -> serde_json::Value {
        json!({ "key": "a", "presence": "working", "last_action_ms": last,
                "activity": { "state": state, "since_ms": 0, "source_ms": 0 }, "reactions": reactions })
    }

    #[test]
    fn the_state_follows_the_consoles_precedence() {
        let now = 1_000_000;
        assert_eq!(state_of(Connection::Disconnected, None, now), LogoState::Disconnected);
        assert_eq!(state_of(Connection::Connecting, None, now), LogoState::Connecting);
        let idle = activity(0, 0, json!([agent("normal", Some(now - 120_000), json!([]))]), now);
        assert_eq!(state_of(Connection::Connected, Some(&idle), now), LogoState::Idle);
        let recent = activity(0, 0, json!([agent("normal", Some(now - 30_000), json!([]))]), now);
        assert_eq!(state_of(Connection::Connected, Some(&recent), now), LogoState::Working);
        let testing = activity(0, 0, json!([agent("testing", None, json!([]))]), now);
        assert_eq!(state_of(Connection::Connected, Some(&testing), now), LogoState::Working);
        let needs = activity(0, 2, json!([agent("testing", None, json!([]))]), now);
        assert_eq!(state_of(Connection::Connected, Some(&needs), now), LogoState::Needs);
        let blocked = activity(1, 2, json!([]), now);
        assert_eq!(state_of(Connection::Connected, Some(&blocked), now), LogoState::Blocked);
    }

    #[test]
    fn only_reactions_after_the_first_reading_play_and_each_once() {
        let mut o = OneShots::default();
        let first = activity(
            0,
            0,
            json!([agent("normal", None, json!([{ "kind": "hop", "at_ms": 900 }]))]),
            1_000,
        );
        assert_eq!(o.next(&first), None, "history never plays");
        let glance = activity(
            0,
            0,
            json!([agent("normal", None, json!([{ "kind": "glance", "at_ms": 1_100 }]))]),
            1_200,
        );
        assert_eq!(o.next(&glance), Some(OneShot::Message));
        assert_eq!(o.next(&glance), None, "once");
        let both = activity(
            0,
            0,
            json!([agent(
                "normal",
                None,
                json!([{ "kind": "glance", "at_ms": 1_300 }, { "kind": "hop", "at_ms": 1_301 }])
            )]),
            1_400,
        );
        assert_eq!(o.next(&both), Some(OneShot::Verified), "a verified delivery wins");
    }
}
