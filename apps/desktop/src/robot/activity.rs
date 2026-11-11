//! What a robot shows for what an agent is doing: the console's
//! `lib/activity.ts` (microstate, face, status words, timer, reactions).
//! Nothing here invents a state serve did not send.

use crate::model::{Activity, Reaction, Thresholds};
use std::collections::HashMap;

/// The faces the robots draw: twelve, one per observable state.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Face {
    Normal,
    Thinking,
    Running,
    Reading,
    Web,
    Delegating,
    Waiting,
    Asleep,
    Blocked,
    Finished,
    Verified,
    Disconnected,
}

impl Face {
    pub const ALL: [Face; 12] = [
        Face::Normal,
        Face::Thinking,
        Face::Running,
        Face::Reading,
        Face::Web,
        Face::Delegating,
        Face::Waiting,
        Face::Asleep,
        Face::Blocked,
        Face::Finished,
        Face::Verified,
        Face::Disconnected,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Face::Normal => "normal",
            Face::Thinking => "thinking",
            Face::Running => "running",
            Face::Reading => "reading",
            Face::Web => "web",
            Face::Delegating => "delegating",
            Face::Waiting => "waiting",
            Face::Asleep => "asleep",
            Face::Blocked => "blocked",
            Face::Finished => "finished",
            Face::Verified => "verified",
            Face::Disconnected => "disconnected",
        }
    }

    pub fn parse(s: &str) -> Option<Face> {
        Face::ALL.into_iter().find(|f| f.id() == s)
    }

    /// Blocked and waiting faces turn the light amber.
    pub fn amber(self) -> bool {
        matches!(self, Face::Blocked | Face::Waiting)
    }
}

/// A state with its microstate: the face to draw and the words to say.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Micro {
    pub face: Face,
    /// Waiting past the sleepy threshold: eyes half closed.
    pub sleepy: bool,
    /// A microstate that asks for attention (long run, too long asleep).
    pub long: bool,
    /// The dictionary key of "Thinking for a long time" and the like.
    pub label: Option<&'static str>,
}

/// The microstate: thinking past `thinking_long_ms` looks from side to side;
/// waiting dozes past `waiting_sleepy_ms` and sleeps past `amber_ms`; a run
/// past `running_long_ms` is flagged; asleep past `red_ms` is flagged.
pub fn micro_state(state: &str, since_ms: u64, now: u64, t: &Thresholds) -> Micro {
    let age = now.saturating_sub(since_ms);
    let plain = |face| Micro {
        face,
        sleepy: false,
        long: false,
        label: None,
    };
    match state {
        "thinking" if age >= t.thinking_long_ms => Micro {
            face: Face::Reading,
            sleepy: false,
            long: false,
            label: Some("activity.microThinkingLong"),
        },
        "thinking" => plain(Face::Thinking),
        "waiting" if age >= t.amber_ms => Micro {
            face: Face::Asleep,
            sleepy: false,
            long: false,
            label: Some("activity.microAsleepWaiting"),
        },
        "waiting" if age >= t.waiting_sleepy_ms => Micro {
            face: Face::Waiting,
            sleepy: true,
            long: false,
            label: Some("activity.microSleepyWaiting"),
        },
        "waiting" => plain(Face::Waiting),
        "running" | "testing" if age >= t.running_long_ms => Micro {
            face: Face::Running,
            sleepy: false,
            long: true,
            label: Some("activity.microRunningLong"),
        },
        "running" | "testing" | "editing" => plain(Face::Running),
        "asleep" if age >= t.red_ms => Micro {
            face: Face::Asleep,
            sleepy: false,
            long: true,
            label: Some("activity.microAsleepLong"),
        },
        other => plain(Face::parse(other).unwrap_or(Face::Normal)),
    }
}

/// When the microstate of `state` next changes, on serve's clock.
pub fn micro_next_change(state: &str, since_ms: u64, now: u64, t: &Thresholds) -> Option<u64> {
    let marks: &[u64] = match state {
        "thinking" => &[t.thinking_long_ms],
        "waiting" => &[t.waiting_sleepy_ms, t.amber_ms],
        "running" | "testing" => &[t.running_long_ms],
        "asleep" => &[t.red_ms],
        _ => &[],
    };
    marks
        .iter()
        .map(|m| since_ms + m)
        .filter(|&at| at > now)
        .min()
}

/// The face a robot draws: a forced face, else the microstate's, else
/// testing and editing type, else the state itself, else at rest.
pub fn face_for(forced: Option<Face>, state: Option<&str>, micro: Option<&Micro>) -> Face {
    if let Some(f) = forced {
        return f;
    }
    if let Some(m) = micro {
        return m.face;
    }
    match state {
        Some("testing" | "editing") => Face::Running,
        Some(s) => Face::parse(s).unwrap_or(Face::Normal),
        None => Face::Normal,
    }
}

/// "8 s", "2 min", "1 h", "1 h 5 min": the status line's timer.
pub fn elapsed(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        return format!("{s} s");
    }
    let m = s / 60;
    if m < 60 {
        return format!("{m} min");
    }
    let h = m / 60;
    if m.is_multiple_of(60) {
        format!("{h} h")
    } else {
        format!("{h} h {} min", m % 60)
    }
}

/// How long until `elapsed(age)` reads differently.
pub fn elapsed_next_change(age_ms: u64) -> u64 {
    let unit = if age_ms < 60_000 { 1000 } else { 60_000 };
    unit - age_ms % unit
}

/// The dictionary: a key and its `{holes}`, worded.
pub type Translate<'a> = &'a dyn Fn(&str, &[(&str, &str)]) -> String;

/// One part of a status line: plain words, or a label in code style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StatusPart {
    Text(String),
    Code(String),
}

/// The status line's words: "Running `cargo test`", "On the web · docs.rs".
/// `tr(key, vars)` is the dictionary; `hhmm` words a time.
pub fn status_parts(
    activity: &Activity,
    reports: usize,
    micro: Option<&Micro>,
    tr: Translate,
    hhmm: &dyn Fn(u64) -> String,
) -> Vec<StatusPart> {
    use StatusPart::{Code, Text};
    let d = activity.detail.as_deref().filter(|d| !d.is_empty());
    let label = micro.and_then(|m| m.label).map(|k| tr(k, &[]));
    let suffix = label.as_ref().map(|l| format!(" · {}", l.to_lowercase()));
    let code = |prefix: String| match d {
        Some(d) => vec![Text(format!("{prefix} ")), Code(d.to_string())],
        None => vec![Text(prefix)],
    };
    match activity.state.as_str() {
        "thinking" => vec![Text(match &label {
            Some(l) => format!("{l}…"),
            None => tr("activity.statusThinking", &[]),
        })],
        "testing" => match d {
            Some(d) => {
                let mut out = vec![
                    Text(format!("{} · ", tr("activity.statusRunningTests", &[]))),
                    Code(d.to_string()),
                ];
                out.extend(suffix.map(Text));
                out
            }
            None => vec![Text(format!(
                "{}{}",
                tr("activity.statusRunningTests", &[]),
                suffix.unwrap_or_default()
            ))],
        },
        "running" => {
            let mut out = if d.is_some() {
                code(tr("activity.statusRunning", &[]))
            } else {
                vec![Text(tr("activity.statusRunningCommand", &[]))]
            };
            out.extend(suffix.map(Text));
            out
        }
        "editing" => code(tr("activity.statusEditing", &[])),
        "reading" => code(tr("activity.statusReading", &[])),
        "web" => vec![Text(match d {
            Some(host) => tr("activity.statusOnWeb", &[("host", host)]),
            None => tr("activity.statusSearchingWeb", &[]),
        })],
        "delegating" => {
            let n = d
                .and_then(|d| d.parse::<u64>().ok())
                .filter(|&n| n > 0)
                .unwrap_or(1);
            let count = n.to_string();
            let key = if n == 1 {
                "activity.statusDelegatingOne"
            } else {
                "activity.statusDelegatingMany"
            };
            vec![Text(tr(key, &[("count", &count)]))]
        }
        "waiting" => {
            let time = hhmm(activity.since_ms);
            let key = if micro.is_some_and(|m| m.sleepy) {
                "activity.statusWaitingSleepy"
            } else {
                "activity.statusWaiting"
            };
            vec![Text(tr(key, &[("time", &time)]))]
        }
        "asleep" => {
            let time = hhmm(activity.since_ms);
            let key = if micro.is_some_and(|m| m.long) {
                "activity.statusAsleepLong"
            } else {
                "activity.statusIdleSince"
            };
            vec![Text(tr(key, &[("time", &time)]))]
        }
        "blocked" => vec![Text(match d {
            Some(detail) => tr("activity.statusBlockedDetail", &[("detail", detail)]),
            None => tr("activity.statusBlocked", &[]),
        })],
        "finished" => {
            // Its lead stopped it: closed, never "No task".
            if d == Some("closed") {
                return vec![Text(tr("activity.statusClosed", &[]))];
            }
            if d == Some("stopped") {
                return vec![Text(tr("activity.statusStopped", &[]))];
            }
            let count = reports.to_string();
            vec![Text(match reports {
                0 => tr("activity.statusFinished", &[]),
                1 => tr("activity.statusFinishedReportsOne", &[("count", &count)]),
                _ => tr("activity.statusFinishedReportsMany", &[("count", &count)]),
            })]
        }
        "verified" => vec![Text(tr("activity.statusVerified", &[]))],
        "disconnected" => vec![Text(tr("activity.statusDisconnected", &[]))],
        _ => vec![Text(tr("activity.statusIdle", &[]))],
    }
}

/// The status line as plain text (tooltips, tests).
pub fn status_text(parts: &[StatusPart]) -> String {
    parts
        .iter()
        .map(|p| match p {
            StatusPart::Text(t) | StatusPart::Code(t) => t.as_str(),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Reactions: each plays once, as it happens, in every view that shows it.
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReactionKind {
    Glance,
    Spark,
    Hop,
    Shake,
    Boot,
}

impl ReactionKind {
    pub fn parse(s: &str) -> Option<ReactionKind> {
        Some(match s {
            "glance" => ReactionKind::Glance,
            "spark" => ReactionKind::Spark,
            "hop" => ReactionKind::Hop,
            "shake" => ReactionKind::Shake,
            "boot" => ReactionKind::Boot,
            _ => return None,
        })
    }

    /// How long one run lasts, in ms.
    pub fn ms(self) -> u64 {
        match self {
            ReactionKind::Hop => 700,
            ReactionKind::Glance => 900,
            ReactionKind::Spark => 800,
            ReactionKind::Shake => 550,
            ReactionKind::Boot => 1600,
        }
    }

    fn rank(self) -> u8 {
        match self {
            ReactionKind::Boot => 5,
            ReactionKind::Shake => 4,
            ReactionKind::Hop => 3,
            ReactionKind::Spark => 2,
            ReactionKind::Glance => 1,
        }
    }
}

/// Views showing the same agent within this of the first play join it.
pub const REACTION_JOIN_MS: u64 = 1_500;

/// `key:kind:at` → when it first played (wall ms).
pub type Played = HashMap<String, i64>;

/// The reaction to play now, if any: newer than `baseline`, not already
/// played (or played under `REACTION_JOIN_MS` ago), the highest ranked of
/// what is fresh. Returns it with when it first played.
pub fn next_reaction(
    key: &str,
    reactions: &[Reaction],
    baseline: u64,
    played: &mut Played,
    wall_now: u64,
) -> Option<(ReactionKind, u64)> {
    let mut fresh: Vec<(String, ReactionKind)> = reactions
        .iter()
        .filter(|r| r.at_ms > baseline)
        .filter_map(|r| {
            ReactionKind::parse(&r.kind).map(|k| (format!("{key}:{}:{}", r.kind, r.at_ms), k))
        })
        .filter(|(id, _)| {
            played
                .get(id)
                .is_none_or(|&at| wall_now as i64 - at < REACTION_JOIN_MS as i64)
        })
        .collect();
    fresh.sort_by_key(|(_, k)| std::cmp::Reverse(k.rank()));
    let (id, kind) = fresh.first().cloned()?;
    let started = (*played.entry(id).or_insert(wall_now as i64)).max(0) as u64;
    // Lower-ranked reactions of the same moment are spent with it.
    for (other, _) in fresh.iter().skip(1) {
        played
            .entry(other.clone())
            .or_insert(wall_now as i64 - REACTION_JOIN_MS as i64);
    }
    Some((kind, started))
}

/// Mark every fresh reaction as played without playing it (Reduce motion):
/// turning motion back on must not replay them.
pub fn spend_reactions(
    key: &str,
    reactions: &[Reaction],
    baseline: u64,
    played: &mut Played,
    wall_now: u64,
) {
    for r in reactions.iter().filter(|r| r.at_ms > baseline) {
        played
            .entry(format!("{key}:{}:{}", r.kind, r.at_ms))
            .or_insert(wall_now as i64 - REACTION_JOIN_MS as i64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: u64 = 60_000;
    const NOW: u64 = 1_794_000_000_000;

    fn limits() -> Thresholds {
        Thresholds {
            amber_ms: 15 * MIN,
            red_ms: 45 * MIN,
            thinking_long_ms: 2 * MIN,
            waiting_sleepy_ms: 5 * MIN,
            running_long_ms: 10 * MIN,
        }
    }

    fn at(state: &str, minutes: f64) -> Micro {
        micro_state(state, NOW, NOW + (minutes * MIN as f64) as u64, &limits())
    }

    #[test]
    fn microstates_follow_the_desks_thresholds() {
        assert_eq!(
            (at("thinking", 1.9).face, at("thinking", 1.9).label),
            (Face::Thinking, None)
        );
        assert_eq!(
            (at("thinking", 2.0).face, at("thinking", 2.0).label),
            (Face::Reading, Some("activity.microThinkingLong"))
        );
        assert!(!at("waiting", 4.9).sleepy);
        assert_eq!(at("waiting", 4.9).face, Face::Waiting);
        assert!(at("waiting", 5.0).sleepy);
        assert_eq!(at("waiting", 15.0).face, Face::Asleep);
        assert!(!at("running", 9.9).long);
        assert!(at("running", 10.0).long);
        assert_eq!(
            (at("testing", 10.0).face, at("testing", 10.0).long),
            (Face::Running, true)
        );
        assert!(!at("asleep", 44.0).long);
        assert_eq!(at("asleep", 45.0).label, Some("activity.microAsleepLong"));
        let quick = Thresholds {
            thinking_long_ms: MIN / 4,
            running_long_ms: MIN / 2,
            ..limits()
        };
        assert_eq!(
            micro_state("thinking", NOW, NOW + MIN / 2, &quick).face,
            Face::Reading
        );
        assert!(micro_state("running", NOW, NOW + MIN, &quick).long);
    }

    #[test]
    fn editing_draws_the_typing_face_and_every_other_state_is_itself() {
        assert_eq!(at("editing", 30.0).face, Face::Running);
        for s in [
            "normal",
            "reading",
            "web",
            "delegating",
            "blocked",
            "finished",
            "verified",
            "disconnected",
        ] {
            assert_eq!(at(s, 90.0).face.id(), s);
        }
    }

    #[test]
    fn the_next_microstate_change_is_the_next_threshold() {
        assert_eq!(
            micro_next_change("waiting", NOW, NOW + MIN, &limits()),
            Some(NOW + 5 * MIN)
        );
        assert_eq!(
            micro_next_change("waiting", NOW, NOW + 6 * MIN, &limits()),
            Some(NOW + 15 * MIN)
        );
        assert_eq!(
            micro_next_change("waiting", NOW, NOW + 16 * MIN, &limits()),
            None
        );
        assert_eq!(micro_next_change("finished", NOW, NOW, &limits()), None);
    }

    #[test]
    fn the_face_is_forced_then_micro_then_typing_then_the_state() {
        let m = at("thinking", 3.0);
        assert_eq!(
            face_for(Some(Face::Asleep), Some("thinking"), Some(&m)),
            Face::Asleep
        );
        assert_eq!(face_for(None, Some("thinking"), Some(&m)), Face::Reading);
        assert_eq!(face_for(None, Some("testing"), None), Face::Running);
        assert_eq!(face_for(None, Some("editing"), None), Face::Running);
        assert_eq!(face_for(None, Some("finished"), None), Face::Finished);
        // The verified face follows serve's state only.
        assert_eq!(face_for(None, Some("verified"), None), Face::Verified);
        assert_eq!(face_for(None, Some("whatever"), None), Face::Normal);
        assert_eq!(face_for(None, None, None), Face::Normal);
        assert!(Face::Blocked.amber() && Face::Waiting.amber() && !Face::Thinking.amber());
    }

    #[test]
    fn the_timer() {
        let got: Vec<String> = [8_000, 104_000, 21 * MIN, 60 * MIN, 65 * MIN]
            .into_iter()
            .map(elapsed)
            .collect();
        assert_eq!(got, ["8 s", "1 min", "21 min", "1 h", "1 h 5 min"]);
        assert_eq!(elapsed_next_change(8_200), 800);
        assert_eq!(elapsed_next_change(61_000), 59_000);
    }

    fn en(key: &str, vars: &[(&str, &str)]) -> String {
        let (ns, k) = key.split_once('.').unwrap();
        let raw = crate::assets::Assets::local(&format!("locales/en/{ns}.json")).unwrap();
        let dict: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        crate::i18n::fill(dict[k].as_str().unwrap(), vars)
    }

    fn line(state: &str, detail: Option<&str>, reports: usize) -> String {
        let a = Activity {
            state: state.into(),
            since_ms: 0,
            detail: detail.map(str::to_string),
            source_ms: 0,
        };
        status_text(&status_parts(&a, reports, None, &en, &|_| "00:12".into()))
    }

    #[test]
    fn the_status_line_says_what_the_console_says() {
        assert_eq!(line("thinking", None, 0), "Thinking…");
        assert_eq!(
            line("running", Some("cargo build"), 0),
            "Running cargo build"
        );
        assert_eq!(line("running", None, 0), "Running a command");
        assert_eq!(
            line("testing", Some("cargo test"), 0),
            "Running tests · cargo test"
        );
        assert_eq!(
            line("editing", Some("workspace.css"), 0),
            "Editing workspace.css"
        );
        assert_eq!(line("reading", Some("team.rs"), 0), "Reading team.rs");
        assert_eq!(line("web", Some("docs.rs"), 0), "On the web · docs.rs");
        assert_eq!(line("delegating", Some("2"), 0), "Delegating to 2 agents");
        assert_eq!(line("delegating", Some("1"), 0), "Delegating to 1 agent");
        assert_eq!(
            line("waiting", None, 0),
            "Waiting in the background since 00:12"
        );
        assert_eq!(line("asleep", None, 0), "No activity since 00:12");
        assert_eq!(
            line("blocked", Some("Can it ask for the password twice?"), 0),
            "Blocked · “Can it ask for the password twice?”"
        );
        assert_eq!(line("finished", None, 2), "Finished · 2 reports");
        assert_eq!(line("finished", Some("closed"), 2), "Closed");
        assert_eq!(line("verified", None, 0), "Verified by harbard");
        assert_eq!(line("disconnected", None, 0), "No connection to harbard");
        assert_eq!(line("normal", None, 0), "No task right now");
        let a = Activity {
            state: "running".into(),
            since_ms: 0,
            detail: Some("seq".into()),
            source_ms: 0,
        };
        let m = micro_state("running", 0, 11 * MIN, &limits());
        let parts = status_parts(&a, 0, Some(&m), &en, &|_| String::new());
        assert_eq!(
            parts,
            [
                StatusPart::Text("Running ".into()),
                StatusPart::Code("seq".into()),
                StatusPart::Text(" · running for a long time".into())
            ]
        );
    }

    fn r(kind: &str, at_ms: u64) -> Reaction {
        Reaction {
            kind: kind.into(),
            at_ms,
        }
    }

    #[test]
    fn reactions_play_once_and_views_mounted_together_join() {
        let mut played = Played::new();
        assert_eq!(
            next_reaction("a", &[r("glance", NOW - 1)], NOW, &mut played, 0),
            None
        );
        assert_eq!(
            next_reaction("a", &[r("glance", NOW + 5)], NOW, &mut played, 1_000),
            Some((ReactionKind::Glance, 1_000))
        );
        assert_eq!(
            next_reaction(
                "a",
                &[r("glance", NOW + 5)],
                NOW,
                &mut played,
                1_000 + REACTION_JOIN_MS
            ),
            None
        );
        let mut played = Played::new();
        assert!(next_reaction("a", &[r("hop", NOW + 5)], NOW, &mut played, 1_000).is_some());
        assert_eq!(
            next_reaction("a", &[r("hop", NOW + 5)], NOW, &mut played, 1_200),
            Some((ReactionKind::Hop, 1_000))
        );
    }

    #[test]
    fn of_reactions_arriving_together_the_strongest_plays_and_the_rest_are_spent() {
        let mut played = Played::new();
        let both = [r("glance", NOW + 5), r("shake", NOW + 5)];
        assert_eq!(
            next_reaction("a", &both, NOW, &mut played, 1_000).map(|x| x.0),
            Some(ReactionKind::Shake)
        );
        assert_eq!(
            next_reaction("a", &both, NOW, &mut played, 1_000 + REACTION_JOIN_MS),
            None
        );
    }

    #[test]
    fn reactions_skipped_under_reduce_motion_are_spent() {
        let mut played = Played::new();
        let rs = [r("boot", NOW + 5)];
        spend_reactions("a", &rs, NOW, &mut played, 1_000);
        assert_eq!(next_reaction("a", &rs, NOW, &mut played, 1_000), None);
        assert_eq!(next_reaction("a", &rs, NOW, &mut played, 60_000), None);
    }
}
