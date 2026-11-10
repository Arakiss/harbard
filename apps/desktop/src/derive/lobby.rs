//! The Lobby (`lib/rooms.ts`, the Lobby half): groups, the filter box, the
//! lineup of robots and the words for harness and presence.

use crate::i18n::t;
use crate::model::{Channel, LobbyEntry, WorkspaceView};
use gpui::{App, SharedString};

pub const LOBBY: &str = "lobby";
pub const COMMONS: &str = "commons";

/// How many robots stand on the lineup shelf.
pub const LINEUP_MAX: usize = 14;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Here,
    Around,
    Gone,
}

impl Group {
    pub fn of(presence: &str) -> Group {
        match presence {
            "live" => Group::Here,
            "ended" => Group::Gone,
            _ => Group::Around,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Group::Here => "here",
            Group::Around => "around",
            Group::Gone => "gone",
        }
    }

    pub fn title(self, cx: &App) -> SharedString {
        match self {
            Group::Here => t(cx, "rooms.groupHere"),
            Group::Around => t(cx, "rooms.groupAround"),
            Group::Gone => t(cx, "rooms.groupGone"),
        }
    }
}

/// Here now, around, gone; serve's newest-first order holds inside each.
/// Empty groups are left out.
pub fn groups<'a>(entries: &[&'a LobbyEntry]) -> Vec<(Group, Vec<&'a LobbyEntry>)> {
    [Group::Here, Group::Around, Group::Gone]
        .into_iter()
        .map(|g| {
            (
                g,
                entries
                    .iter()
                    .copied()
                    .filter(|e| Group::of(&e.presence) == g)
                    .collect::<Vec<_>>(),
            )
        })
        .filter(|(_, list)| !list.is_empty())
        .collect()
}

/// How many entries fall in each group (here, around, gone), unfiltered.
pub fn counts(entries: &[LobbyEntry]) -> (usize, usize, usize) {
    let mut out = (0, 0, 0);
    for e in entries {
        match Group::of(&e.presence) {
            Group::Here => out.0 += 1,
            Group::Around => out.1 += 1,
            Group::Gone => out.2 += 1,
        }
    }
    out
}

/// "Claude Code", "Codex", "Cursor", or "Other harness".
pub fn harness_name(harness: &str, cx: &App) -> SharedString {
    match harness {
        "claude-code" => "Claude Code".into(),
        "codex" => "Codex".into(),
        "cursor" => "Cursor".into(),
        "other" => t(cx, "rooms.harnessOther"),
        other => other.to_string().into(),
    }
}

pub fn presence_word(presence: &str, cx: &App) -> SharedString {
    match presence {
        "live" => t(cx, "rooms.presenceLive"),
        "idle" => t(cx, "rooms.presenceIdle"),
        "stale" => t(cx, "rooms.presenceStale"),
        _ => t(cx, "rooms.presenceEnded"),
    }
}

/// The filter box: a case-insensitive substring of the name, the project,
/// the harness as shown or what it said. `harness` words a harness id.
pub fn filter<'a>(
    entries: &'a [LobbyEntry],
    query: &str,
    harness: impl Fn(&str) -> String,
) -> Vec<&'a LobbyEntry> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return entries.iter().collect();
    }
    let has = |s: Option<&str>| s.is_some_and(|s| s.to_lowercase().contains(&q));
    entries
        .iter()
        .filter(|e| {
            has(Some(&e.name))
                || has(e.project.as_deref())
                || has(Some(&harness(&e.harness)))
                || has(e.intro.as_deref())
        })
        .collect()
}

/// The robots on the shelf: who is here, or when nobody is, who is around.
/// Returns the ones shown and how many more there are.
pub fn lineup(entries: &[LobbyEntry]) -> (Vec<&LobbyEntry>, usize) {
    let here: Vec<&LobbyEntry> = entries.iter().filter(|e| e.presence == "live").collect();
    let pool = if here.is_empty() {
        entries.iter().filter(|e| e.presence != "ended").collect()
    } else {
        here
    };
    let more = pool.len().saturating_sub(LINEUP_MAX);
    (pool.into_iter().take(LINEUP_MAX).collect(), more)
}

/// `#lobby`, whatever serve listed (a placeholder when it did not).
pub fn lobby_channel(view: &WorkspaceView) -> Channel {
    view.channel(LOBBY).cloned().unwrap_or_else(|| Channel {
        id: LOBBY.into(),
        kind: Some("room".into()),
        built_in: true,
        ..Default::default()
    })
}

/// What the composer says above the box in a room (`roomComposerNotice`).
pub fn room_notice(channel: &Channel, cx: &App) -> Option<SharedString> {
    if channel.id == LOBBY || channel.id == COMMONS {
        return Some(crate::i18n::tv(
            cx,
            "rooms.noticeReachesNoWork",
            &[("room", &channel.id)],
        ));
    }
    if channel.reach.as_deref() == Some("ready") {
        return None;
    }
    Some(crate::i18n::tv(
        cx,
        "rooms.noticeNoReaders",
        &[("room", &channel.id)],
    ))
}

/// FNV-1a over code points: the robot identity hash (`robotFor`).
fn name_hash(name: &str) -> u32 {
    let mut x: u32 = 2_166_136_261;
    for c in name.chars() {
        x ^= c as u32;
        x = x.wrapping_mul(16_777_619);
    }
    x
}

/// The robot's tile, 1..4 (`--hb-robot-tile-<n>`): the card band wears it.
pub fn robot_tile(name: &str) -> u32 {
    ((name_hash(name) >> 9) % 4) + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(key: &str, presence: &str) -> LobbyEntry {
        LobbyEntry {
            key: key.into(),
            name: key.into(),
            harness: "codex".into(),
            presence: presence.into(),
            ..Default::default()
        }
    }

    #[test]
    fn groups_keep_serve_order_and_drop_empty_ones() {
        let all = [
            entry("a", "ended"),
            entry("b", "live"),
            entry("c", "stale"),
            entry("d", "idle"),
            entry("e", "live"),
        ];
        let refs: Vec<&LobbyEntry> = all.iter().collect();
        let g = groups(&refs);
        let keys: Vec<(Group, Vec<&str>)> = g
            .iter()
            .map(|(g, l)| (*g, l.iter().map(|e| e.key.as_str()).collect()))
            .collect();
        assert_eq!(
            keys,
            vec![
                (Group::Here, vec!["b", "e"]),
                (Group::Around, vec!["c", "d"]),
                (Group::Gone, vec!["a"])
            ]
        );
        let only_gone = [entry("x", "ended")];
        let refs: Vec<&LobbyEntry> = only_gone.iter().collect();
        assert_eq!(groups(&refs).len(), 1);
        assert_eq!(counts(&all), (2, 2, 1));
    }

    #[test]
    fn lineup_prefers_who_is_here_then_who_is_around() {
        let all = [entry("a", "ended"), entry("b", "idle"), entry("c", "stale")];
        let (shown, more) = lineup(&all);
        assert_eq!(
            shown.iter().map(|e| e.key.as_str()).collect::<Vec<_>>(),
            vec!["b", "c"]
        );
        assert_eq!(more, 0);
        let many: Vec<LobbyEntry> = (0..20).map(|i| entry(&format!("k{i}"), "live")).collect();
        let (shown, more) = lineup(&many);
        assert_eq!(shown.len(), LINEUP_MAX);
        assert_eq!(more, 6);
        assert!(lineup(&[entry("z", "ended")]).0.is_empty());
    }

    #[test]
    fn filter_matches_name_project_harness_and_intro() {
        let mut a = entry("Adrian", "live");
        a.project = Some("harbard".into());
        let mut b = entry("Barry", "idle");
        b.intro = Some("I fix the Retro".into());
        b.harness = "claude-code".into();
        let all = [a, b];
        let word = |h: &str| {
            if h == "claude-code" {
                "Claude Code".to_string()
            } else {
                "Codex".to_string()
            }
        };
        let names = |q: &str| {
            filter(&all, q, word)
                .iter()
                .map(|e| e.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names("  "), vec!["Adrian", "Barry"]);
        assert_eq!(names("HARB"), vec!["Adrian"]);
        assert_eq!(names("claude code"), vec!["Barry"]);
        assert_eq!(names("retro"), vec!["Barry"]);
        assert!(names("nobody").is_empty());
    }

    #[test]
    fn robot_tile_is_stable_and_in_range() {
        for name in ["Adrian", "Damian", "Claude · lead", "ñandú"] {
            let t = robot_tile(name);
            assert!((1..=4).contains(&t));
            assert_eq!(t, robot_tile(name));
        }
        // FNV-1a of "a" is 0xe40c292c.
        assert_eq!(name_hash("a"), 0xe40c_292c);
    }
}
