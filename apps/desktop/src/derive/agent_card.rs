//! The agent hover card in words (`lib/agent-card.ts`, `lib/progress.ts`)
//! and the status line's words it shows under "Now" (`lib/activity.ts`:
//! `microState`, `statusParts`, `elapsed`).

use crate::derive::{agent_name, dot_tone, presence_text};
use crate::i18n::{t, tv};
use crate::model::{Activity, Agent, Progress, StepRow, TestCounts, Thresholds};
use crate::ui::format::hhmm;
use gpui::{App, SharedString};

/// The card opens this long after the pointer arrives.
pub const OPEN_DELAY_MS: u64 = 300;
/// Leaving the trigger for the card is not leaving: this much grace.
pub const CLOSE_GRACE_MS: u64 = 150;
/// The card's width.
pub const CARD_WIDTH: f32 = 440.;

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
    if !m.is_multiple_of(60) {
        format!("{h} h {} min", m % 60)
    } else {
        format!("{h} h")
    }
}

/// "12 s", "1 min 12 s", "2 h 14 min": a running clock with its seconds.
pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    if s < 60 {
        return format!("{s} s");
    }
    let m = s / 60;
    if m < 60 {
        return if !s.is_multiple_of(60) {
            format!("{m} min {} s", s % 60)
        } else {
            format!("{m} min")
        };
    }
    let h = m / 60;
    if !m.is_multiple_of(60) {
        format!("{h} h {} min", m % 60)
    } else {
        format!("{h} h")
    }
}

/// How long ago, down to seconds (`relative`): (dictionary key, count).
pub fn relative_parts(ms: u64, now: u64) -> (&'static str, u64) {
    let s = now.saturating_sub(ms) / 1000;
    if s < 5 {
        return ("progress.relativeNow", 0);
    }
    if s < 60 {
        return ("progress.relativeSeconds", s);
    }
    let m = s / 60;
    if m < 60 {
        return ("progress.relativeMinutes", m);
    }
    ("progress.relativeHours", m / 60)
}

pub fn relative(ms: u64, now: u64, cx: &App) -> SharedString {
    let (key, n) = relative_parts(ms, now);
    tv(cx, key, &[("count", &n.to_string())])
}

/// A state with its microstate.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Micro {
    pub face: String,
    pub sleepy: bool,
    pub long: bool,
    /// The dictionary key of "Thinking a long while", or `None`.
    pub label: Option<&'static str>,
}

/// The microstate from the state's age and the desk's thresholds.
pub fn micro_state(activity: &Activity, now: u64, limits: &Thresholds) -> Micro {
    let age = now.saturating_sub(activity.since_ms);
    let plain = |face: &str| Micro {
        face: face.into(),
        ..Default::default()
    };
    match activity.state.as_str() {
        "thinking" if age >= limits.thinking_long_ms => Micro {
            face: "reading".into(),
            label: Some("activity.microThinkingLong"),
            ..Default::default()
        },
        "thinking" => plain("thinking"),
        "waiting" if age >= limits.amber_ms => Micro {
            face: "asleep".into(),
            label: Some("activity.microAsleepWaiting"),
            ..Default::default()
        },
        "waiting" if age >= limits.waiting_sleepy_ms => Micro {
            face: "waiting".into(),
            sleepy: true,
            label: Some("activity.microSleepyWaiting"),
            ..Default::default()
        },
        "waiting" => plain("waiting"),
        "running" | "testing" if age >= limits.running_long_ms => Micro {
            face: "running".into(),
            long: true,
            label: Some("activity.microRunningLong"),
            ..Default::default()
        },
        "running" | "testing" | "editing" => plain("running"),
        "asleep" if age >= limits.red_ms => Micro {
            face: "asleep".into(),
            long: true,
            label: Some("activity.microAsleepLong"),
            ..Default::default()
        },
        other => plain(if other.is_empty() { "normal" } else { other }),
    }
}

/// One part of a status line: plain words, or a label in code style.
#[derive(Clone, Debug, PartialEq)]
pub enum StatusPart {
    Text(SharedString),
    Code(SharedString),
}

/// The status line's words: "Running `cargo test`", "On the web · docs.rs".
pub fn status_parts(
    activity: &Activity,
    reports: usize,
    micro: Option<&Micro>,
    cx: &App,
) -> Vec<StatusPart> {
    use StatusPart::{Code, Text};
    let d = activity.detail.as_deref().filter(|d| !d.is_empty());
    let label = micro.and_then(|m| m.label).map(|k| t(cx, k));
    let tail = label.as_ref().map(|l| format!(" · {}", l.to_lowercase()));
    let code = |prefix: SharedString| -> Vec<StatusPart> {
        match d {
            Some(d) => vec![
                Text(format!("{prefix} ").into()),
                Code(d.to_string().into()),
            ],
            None => vec![Text(prefix)],
        }
    };
    let since = hhmm(activity.since_ms);
    match activity.state.as_str() {
        "thinking" => vec![Text(match &label {
            Some(l) => format!("{l}…").into(),
            None => t(cx, "activity.statusThinking"),
        })],
        "testing" => match d {
            Some(d) => {
                let mut v = vec![
                    Text(format!("{} · ", t(cx, "activity.statusRunningTests")).into()),
                    Code(d.to_string().into()),
                ];
                if let Some(tail) = tail {
                    v.push(Text(tail.into()));
                }
                v
            }
            None => vec![Text(
                format!(
                    "{}{}",
                    t(cx, "activity.statusRunningTests"),
                    tail.unwrap_or_default()
                )
                .into(),
            )],
        },
        "running" => {
            let mut v = if d.is_some() {
                code(t(cx, "activity.statusRunning"))
            } else {
                vec![Text(t(cx, "activity.statusRunningCommand"))]
            };
            if let Some(tail) = tail {
                v.push(Text(tail.into()));
            }
            v
        }
        "editing" => code(t(cx, "activity.statusEditing")),
        "reading" => code(t(cx, "activity.statusReading")),
        "web" => vec![Text(match d {
            Some(host) => tv(cx, "activity.statusOnWeb", &[("host", host)]),
            None => t(cx, "activity.statusSearchingWeb"),
        })],
        "delegating" => {
            let n = d
                .and_then(|d| d.parse::<u64>().ok())
                .filter(|n| *n > 0)
                .unwrap_or(1);
            let key = if n == 1 {
                "activity.statusDelegatingOne"
            } else {
                "activity.statusDelegatingMany"
            };
            vec![Text(tv(cx, key, &[("count", &n.to_string())]))]
        }
        "waiting" => {
            let key = if micro.is_some_and(|m| m.sleepy) {
                "activity.statusWaitingSleepy"
            } else {
                "activity.statusWaiting"
            };
            vec![Text(tv(cx, key, &[("time", &since)]))]
        }
        "asleep" => {
            let key = if micro.is_some_and(|m| m.long) {
                "activity.statusAsleepLong"
            } else {
                "activity.statusIdleSince"
            };
            vec![Text(tv(cx, key, &[("time", &since)]))]
        }
        "blocked" => vec![Text(match d {
            Some(d) => tv(cx, "activity.statusBlockedDetail", &[("detail", d)]),
            None => t(cx, "activity.statusBlocked"),
        })],
        "finished" if d == Some("closed") => vec![Text(t(cx, "activity.statusClosed"))],
        "finished" => vec![Text(match reports {
            0 => t(cx, "activity.statusFinished"),
            1 => tv(cx, "activity.statusFinishedReportsOne", &[("count", "1")]),
            n => tv(
                cx,
                "activity.statusFinishedReportsMany",
                &[("count", &n.to_string())],
            ),
        })],
        "verified" => vec![Text(t(cx, "activity.statusVerified"))],
        "disconnected" => vec![Text(t(cx, "activity.statusDisconnected"))],
        _ => vec![Text(t(cx, "activity.statusIdle"))],
    }
}

/// The Lucide icon of the card's "Now" line (`nowIcon`).
pub fn now_icon(state: &str) -> &'static str {
    match state {
        "running" | "testing" => "square-terminal",
        "editing" => "code-xml",
        "reading" => "file-text",
        "web" => "globe",
        "delegating" => "users",
        "waiting" => "hourglass",
        "asleep" => "moon",
        "blocked" => "ban",
        "finished" | "verified" => "circle-check",
        "disconnected" => "wifi-off",
        _ => "coffee",
    }
}

/// What a step looks like: the icon family it is drawn with.
pub fn step_kind(step: &StepRow) -> &str {
    if step.commit {
        return "commit";
    }
    let tool = step.tool.as_deref();
    match step.state.as_str() {
        "reading" if matches!(tool, Some("Grep" | "Glob")) => "searching",
        "thinking" if tool == Some("SendMessage") => "message",
        "thinking" => "thinking",
        other => other,
    }
}

/// The Lucide icon per step kind (`STEP_ICON`).
pub fn step_icon(kind: &str) -> &'static str {
    match kind {
        "editing" => "code-xml",
        "reading" => "file-text",
        "searching" => "search",
        "running" => "settings",
        "testing" => "square-terminal",
        "commit" => "git-branch",
        "web" => "globe",
        "delegating" => "users",
        "message" => "send",
        _ => "sparkles",
    }
}

fn other_tool(tool: &str) -> Option<&'static str> {
    Some(match tool {
        "SendMessage" => "toolSendMessage",
        "TodoWrite" => "toolTodoWrite",
        "TaskCreate" => "toolTaskCreate",
        "TaskUpdate" => "toolTaskUpdate",
        "TaskList" => "toolTaskList",
        "Skill" => "toolSkill",
        "ToolSearch" => "toolToolSearch",
        "BashOutput" => "toolBashOutput",
        "TaskOutput" => "toolTaskOutput",
        "KillShell" => "toolKillShell",
        "TaskStop" => "toolTaskStop",
        "Monitor" => "toolMonitor",
        "AskUserQuestion" => "toolAskUserQuestion",
        "ExitPlanMode" => "toolExitPlanMode",
        "EnterWorktree" => "toolEnterWorktree",
        "LSP" => "toolLsp",
        _ => return None,
    })
}

/// The dictionary key and detail of a step in words (`stepText`).
pub fn step_words(step: &StepRow) -> (String, Option<&str>) {
    let d = step.detail.as_deref().filter(|d| !d.is_empty());
    let with = |detail_key: &str, bare: &str| match d {
        Some(_) => (detail_key.to_string(), d),
        None => (bare.to_string(), None),
    };
    let key_and = match step_kind(step) {
        "commit" => ("stepCommit".to_string(), None),
        "running" => with("stepRunningDetail", "stepRunning"),
        "testing" => with("stepTestsDetail", "stepTests"),
        "editing" => with("stepEditingDetail", "stepEditing"),
        "searching" => with("stepSearchingDetail", "stepSearching"),
        "reading" => with("stepReadingDetail", "stepReading"),
        "web" if step.tool.as_deref() == Some("WebSearch") => ("stepWebSearch".to_string(), None),
        "web" => with("stepOpeningDetail", "stepOpening"),
        "delegating" => ("stepDelegating".to_string(), None),
        "message" => ("toolSendMessage".to_string(), None),
        _ => (
            step.tool
                .as_deref()
                .and_then(other_tool)
                .unwrap_or("stepTool")
                .to_string(),
            None,
        ),
    };
    (format!("progress.{}", key_and.0), key_and.1)
}

pub fn step_text(step: &StepRow, cx: &App) -> SharedString {
    let (key, detail) = step_words(step);
    match detail {
        Some(d) => tv(cx, &key, &[("detail", d)]),
        None => t(cx, &key),
    }
}

/// "3 pass, 1 fails[, 2 skipped]".
pub fn test_summary(run: &TestCounts, cx: &App) -> SharedString {
    let pick = |n: u64, one: &str, many: &str| {
        tv(
            cx,
            if n == 1 { one } else { many },
            &[("count", &n.to_string())],
        )
    };
    let mut parts = vec![
        pick(
            run.passed,
            "progress.testsPassOne",
            "progress.testsPassMany",
        ),
        pick(
            run.failed,
            "progress.testsFailOne",
            "progress.testsFailMany",
        ),
    ];
    if run.skipped > 0 {
        parts.push(pick(
            run.skipped,
            "progress.testsSkippedOne",
            "progress.testsSkippedMany",
        ));
    }
    parts
        .iter()
        .map(|s| s.as_ref())
        .collect::<Vec<_>>()
        .join(", ")
        .into()
}

/// One piece of the "Progress" line: words, then numbers in bold with theirs.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgressPart {
    /// running, steps, commits, testing
    pub key: &'static str,
    pub lead: Option<SharedString>,
    pub values: Vec<(String, SharedString)>,
    /// ok, fail
    pub tone: Option<&'static str>,
}

/// The card's "Progress" parts: only real signals.
pub fn progress_parts(
    agent: &Agent,
    progress: Option<&Progress>,
    now: u64,
    cx: &App,
) -> Vec<ProgressPart> {
    let mut parts = Vec::new();
    if let Some(first) = agent.first_ms.filter(|ms| *ms > 0) {
        parts.push(ProgressPart {
            key: "running",
            lead: Some(t(cx, "agents.progressRunning")),
            values: vec![(elapsed(now.saturating_sub(first)), SharedString::default())],
            tone: None,
        });
    }
    if let Some(p) = progress {
        let words = |n: u64, one: &str, many: &str| t(cx, if n == 1 { one } else { many });
        parts.push(ProgressPart {
            key: "steps",
            lead: None,
            values: vec![(
                p.tool_calls.to_string(),
                words(p.tool_calls, "agents.stepOne", "agents.stepMany"),
            )],
            tone: None,
        });
        parts.push(ProgressPart {
            key: "commits",
            lead: None,
            values: vec![(
                p.commits.to_string(),
                words(p.commits, "agents.commitOne", "agents.commitMany"),
            )],
            tone: None,
        });
        if let Some(run) = &p.test {
            let mut values = vec![
                (
                    run.passed.to_string(),
                    words(run.passed, "agents.passOne", "agents.passMany"),
                ),
                (
                    run.failed.to_string(),
                    words(run.failed, "agents.failOne", "agents.failMany"),
                ),
            ];
            if run.skipped > 0 {
                values.push((
                    run.skipped.to_string(),
                    words(run.skipped, "agents.skipOne", "agents.skipMany"),
                ));
            }
            parts.push(ProgressPart {
                key: "testing",
                lead: Some(t(cx, "agents.testsLead")),
                values,
                tone: Some(if run.ok { "ok" } else { "fail" }),
            });
        }
    }
    parts
}

/// The token behind the "Now" block (`.hb-card-now`): green while work runs,
/// amber while it waits or runs long, red when blocked, a plain surface
/// when there is nothing going on.
pub fn now_wash(state: &str, long: bool) -> &'static str {
    match state {
        "blocked" => "red-bg",
        "waiting" | "asleep" => "warn-bg",
        _ if now_quiet(state) => "surface-2",
        _ if long => "warn-bg",
        _ => "ok-bg",
    }
}

/// A state with nothing running: its tile goes quiet (`control`, `ink-3`)
/// instead of the terminal's dark one.
pub fn now_quiet(state: &str) -> bool {
    matches!(state, "normal" | "finished" | "disconnected" | "")
}

/// The ink of a step's icon by its kind (`.hb-step-ic[data-kind]`).
pub fn step_ink(kind: &str) -> &'static str {
    match kind {
        "reading" | "searching" | "web" => "focus",
        "running" | "testing" => "green-ink",
        "commit" => "st-orbit",
        _ => "accent-text",
    }
}

/// The wash behind a step row (`.hb-step[data-status]`).
pub fn step_wash(status: &str) -> &'static str {
    match status {
        "running" => "ok-bg",
        "error" => "red-bg",
        _ => "tint",
    }
}

/// The wash and ink of the presence pill (`.hb-card-pill[data-kind=presence]`).
pub fn presence_pill(tone: &str) -> (&'static str, &'static str) {
    match tone {
        "green" => ("green-bg", "green-ink"),
        "amber" => ("warn-bg", "warn-ink"),
        "red" => ("red-bg", "red-ink"),
        _ => ("surface-2", "ink-2"),
    }
}

// ---------------------------------------------------------------------------
// Where the card goes
// ---------------------------------------------------------------------------

/// Between the trigger and the card (`placeCard(…, 14)`).
pub const CARD_GAP: f32 = 14.;
/// The arrow keeps this far from the card's corners.
pub const ARROW_INSET: f32 = 22.;

/// A rectangle in window points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

/// The side of the trigger the card opens on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Right,
    Left,
    Below,
    Above,
}

/// Where the card goes and where its arrow points, measured along the
/// edge that faces the trigger from the card's top (right and left) or its
/// left (below and above).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    pub left: f32,
    pub top: f32,
    pub side: Side,
    pub arrow: f32,
}

/// Place a `width` × `height` card beside `trigger`, inside `area` (the
/// panel under its header): to the right when it fits, else to the left,
/// else below, else above (`placeCard`). It never leaves `area`, so it can
/// cover neither a screen's header nor the title band; a card taller than
/// `area` starts at its top.
pub fn place_card(trigger: Rect, width: f32, height: f32, area: Rect, gap: f32) -> Placement {
    let clamp_y = |y: f32| y.min(area.bottom - height).max(area.top);
    let clamp_x = |x: f32| x.min(area.right - width).max(area.left);
    let along = |mid: f32, start: f32, length: f32| {
        (mid - start)
            .min(length - ARROW_INSET)
            .max(ARROW_INSET)
            .round()
    };
    let mid_y = (trigger.top + trigger.bottom) / 2.;
    let mid_x = (trigger.left + trigger.right) / 2.;
    let beside = |left: f32, side: Side| {
        let top = clamp_y(trigger.top - 8.);
        Placement {
            left,
            top,
            side,
            arrow: along(mid_y, top, height),
        }
    };
    let stacked = |top: f32, side: Side| {
        let left = clamp_x(trigger.left);
        Placement {
            left,
            top,
            side,
            arrow: along(mid_x, left, width),
        }
    };
    if trigger.right + gap + width <= area.right {
        beside(trigger.right + gap, Side::Right)
    } else if trigger.left - gap - width >= area.left {
        beside(trigger.left - gap - width, Side::Left)
    } else if trigger.bottom + gap + height <= area.bottom {
        stacked(trigger.bottom + gap, Side::Below)
    } else if trigger.top - gap - height >= area.top {
        stacked(trigger.top - gap - height, Side::Above)
    } else {
        // Nowhere it fits whole: over the trigger's left, inside the area.
        beside(clamp_x(trigger.left - gap - width), Side::Left)
    }
}

/// The pills and words at the top of the card.
pub struct CardModel {
    pub name: SharedString,
    pub brief: Option<String>,
    pub teams: Vec<String>,
    pub role: Option<String>,
    pub presence: (SharedString, &'static str),
}

/// "general-purpose · aa18a5f8" is the label; the type is its first part.
pub fn agent_type(label: &str) -> Option<&str> {
    let ty = label.split(" · ").next().unwrap_or("").trim();
    (!ty.is_empty()).then_some(ty)
}

pub fn card_model(agent: &Agent, cx: &App) -> CardModel {
    let role = agent_type(&agent.label)
        .filter(|ty| *ty != "general-purpose" && *ty != agent.name)
        .map(str::to_string);
    let brief = agent
        .description
        .as_deref()
        .or(agent.brief.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    let presence = if agent.lost {
        (t(cx, "agents.noData"), "grey")
    } else {
        (
            presence_text(&agent.presence, cx),
            dot_tone(&agent.presence, agent.dormant.as_deref()),
        )
    };
    CardModel {
        name: agent_name(agent, cx),
        brief,
        teams: agent.channels.iter().take(2).cloned().collect(),
        role,
        presence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    /// The main window at 1440 × 900: the panel under a 56 px header.
    const AREA: Rect = Rect {
        left: 86.,
        top: 105.,
        right: 1426.,
        bottom: 886.,
    };

    #[test]
    fn a_sidebar_row_opens_the_card_to_its_right_with_the_arrow_at_its_middle() {
        let row = rect(88., 520., 368., 564.);
        let p = place_card(row, 440., 650., AREA, CARD_GAP);
        assert_eq!(p.side, Side::Right);
        assert_eq!(p.left, 382.);
        // 520 - 8 would run past the bottom: it rises to fit.
        assert_eq!(p.top, 886. - 650.);
        assert_eq!(p.arrow, 542. - 236.);
    }

    #[test]
    fn a_row_under_the_header_never_lifts_the_card_into_it() {
        // The first DM row sits right under the header; the card is tall.
        let row = rect(79., 98., 498., 226.);
        let p = place_card(row, 440., 520., AREA, CARD_GAP);
        assert_eq!(p.side, Side::Right);
        assert_eq!(p.top, AREA.top);
        assert_eq!(p.arrow, 162. - 105.);
    }

    #[test]
    fn the_arrow_keeps_clear_of_the_corners() {
        let row = rect(88., 100., 368., 110.);
        let p = place_card(row, 440., 500., AREA, CARD_GAP);
        assert_eq!(p.arrow, ARROW_INSET);
        let low = rect(88., 880., 368., 886.);
        let p = place_card(low, 440., 500., AREA, CARD_GAP);
        assert_eq!(p.arrow, 500. - ARROW_INSET);
    }

    #[test]
    fn it_flips_left_then_below_then_above_when_the_right_is_full() {
        let right_edge = rect(1100., 400., 1300., 430.);
        let p = place_card(right_edge, 440., 300., AREA, CARD_GAP);
        assert_eq!((p.side, p.left), (Side::Left, 1100. - CARD_GAP - 440.));

        let narrow = Rect {
            left: 86.,
            top: 105.,
            right: 600.,
            bottom: 886.,
        };
        let wide = rect(100., 200., 560., 230.);
        let p = place_card(wide, 440., 300., narrow, CARD_GAP);
        assert_eq!((p.side, p.top), (Side::Below, 230. + CARD_GAP));
        assert_eq!(p.left, 100.);
        assert_eq!(p.arrow, 230.);

        let low = rect(100., 700., 560., 730.);
        let p = place_card(low, 440., 300., narrow, CARD_GAP);
        assert_eq!((p.side, p.top), (Side::Above, 700. - CARD_GAP - 300.));
    }

    #[test]
    fn a_card_taller_than_the_panel_starts_at_its_top() {
        let row = rect(88., 300., 368., 330.);
        let p = place_card(row, 440., 2000., AREA, CARD_GAP);
        assert_eq!(p.top, AREA.top);
    }

    #[test]
    fn washes_follow_the_console() {
        assert_eq!(now_wash("running", false), "ok-bg");
        assert_eq!(now_wash("running", true), "warn-bg");
        assert_eq!(now_wash("waiting", false), "warn-bg");
        assert_eq!(now_wash("blocked", false), "red-bg");
        assert_eq!(now_wash("finished", true), "surface-2");
        assert!(now_quiet("normal") && !now_quiet("editing"));
        assert_eq!(step_ink("searching"), "focus");
        assert_eq!(step_ink("testing"), "green-ink");
        assert_eq!(step_ink("commit"), "st-orbit");
        assert_eq!(step_ink("editing"), "accent-text");
        assert_eq!(step_wash("running"), "ok-bg");
        assert_eq!(step_wash("error"), "red-bg");
        assert_eq!(step_wash("done"), "tint");
        assert_eq!(presence_pill("green"), ("green-bg", "green-ink"));
        assert_eq!(presence_pill("grey"), ("surface-2", "ink-2"));
    }

    fn step(state: &str, tool: Option<&str>) -> StepRow {
        StepRow {
            state: state.into(),
            tool: tool.map(Into::into),
            ..Default::default()
        }
    }

    #[test]
    fn step_kinds_follow_the_console() {
        assert_eq!(step_kind(&step("reading", Some("Grep"))), "searching");
        assert_eq!(step_kind(&step("reading", Some("Read"))), "reading");
        assert_eq!(step_kind(&step("thinking", Some("SendMessage"))), "message");
        assert_eq!(step_kind(&step("thinking", Some("TodoWrite"))), "thinking");
        let mut c = step("running", Some("Bash"));
        c.commit = true;
        assert_eq!(step_kind(&c), "commit");
    }

    #[test]
    fn step_words_carry_the_safe_label_only() {
        let mut s = step("running", Some("Bash"));
        s.detail = Some("cargo test".into());
        assert_eq!(
            step_words(&s),
            ("progress.stepRunningDetail".into(), Some("cargo test"))
        );
        assert_eq!(
            step_words(&step("web", Some("WebSearch"))),
            ("progress.stepWebSearch".into(), None)
        );
        assert_eq!(
            step_words(&step("thinking", Some("TodoWrite"))),
            ("progress.toolTodoWrite".into(), None)
        );
        assert_eq!(
            step_words(&step("thinking", Some("mcp__x"))),
            ("progress.stepTool".into(), None)
        );
    }

    #[test]
    fn counts_times_and_the_running_clock() {
        assert_eq!(clock(12_000), "12 s");
        assert_eq!(clock(72_000), "1 min 12 s");
        assert_eq!(clock(120_000), "2 min");
        assert_eq!(clock(2 * 3_600_000 + 14 * 60_000), "2 h 14 min");
        assert_eq!(elapsed(8_000), "8 s");
        assert_eq!(elapsed(125_000), "2 min");
        assert_eq!(elapsed(3_900_000), "1 h 5 min");
        assert_eq!(relative_parts(1_000, 3_000), ("progress.relativeNow", 0));
        assert_eq!(relative_parts(0, 30_000), ("progress.relativeSeconds", 30));
        assert_eq!(
            relative_parts(0, 5 * 60_000),
            ("progress.relativeMinutes", 5)
        );
        assert_eq!(
            relative_parts(0, 3 * 3_600_000),
            ("progress.relativeHours", 3)
        );
    }

    #[test]
    fn microstates_from_the_thresholds() {
        let limits = Thresholds {
            amber_ms: 1_000,
            red_ms: 2_000,
            thinking_long_ms: 100,
            waiting_sleepy_ms: 500,
            running_long_ms: 300,
        };
        let a = |state: &str| Activity {
            state: state.into(),
            since_ms: 0,
            ..Default::default()
        };
        assert_eq!(micro_state(&a("thinking"), 50, &limits).label, None);
        assert_eq!(micro_state(&a("thinking"), 150, &limits).face, "reading");
        assert!(micro_state(&a("waiting"), 600, &limits).sleepy);
        assert_eq!(micro_state(&a("waiting"), 1_200, &limits).face, "asleep");
        assert!(micro_state(&a("testing"), 400, &limits).long);
        assert!(micro_state(&a("asleep"), 2_500, &limits).long);
        assert_eq!(micro_state(&a("editing"), 0, &limits).face, "running");
    }

    #[test]
    fn the_role_pill_skips_general_purpose_and_the_own_name() {
        assert_eq!(
            agent_type("general-purpose · aa18"),
            Some("general-purpose")
        );
        assert_eq!(agent_type("Explore · aa18"), Some("Explore"));
        assert_eq!(agent_type(""), None);
    }
}
