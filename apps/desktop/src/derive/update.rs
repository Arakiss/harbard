//! "Update harbard" (`lib/update.ts`): the labels by id and the pure
//! decisions the dialog and Preferences add to the shell's update store.

use crate::i18n::{t, tv};
use crate::model::update::{
    UpdateChange, UpdateCheck, UpdateError, UpdateRun, UpdateStep, UpdateView,
};
use crate::prefs::shell_update::{banner_line, error_text};
use crate::ui::format::{at_date, hace};
use gpui::{App, SharedString};

pub const STEP_ORDER: [&str; 6] = ["fetch", "cli", "hooks", "serve", "console", "desktop"];

/// The tray's "Update…" re-checks when the last look is older than this.
pub const FRESH_MS: u64 = 5 * 60_000;

pub fn step_label(id: &str, cx: &App) -> SharedString {
    let key = match id {
        "fetch" => "update.stepFetch",
        "cli" => "update.stepCli",
        "hooks" => "update.stepHooks",
        "serve" => "update.stepServe",
        "console" => "update.stepConsole",
        "desktop" => "update.stepDesktop",
        other => return other.to_string().into(),
    };
    t(cx, key)
}

fn problem_key(problem: &str) -> Option<&'static str> {
    Some(match problem {
        "no_checkout" => "update.problemNoCheckout",
        "dirty" => "update.problemDirty",
        "not_on_main" => "update.problemNotOnMain",
        "diverged" => "update.problemDiverged",
        _ => return None,
    })
}

/// What a source problem means for the person, and what to do about it.
pub fn source_problem_text(problem: &str, cx: &App) -> SharedString {
    problem_key(problem)
        .map(|k| t(cx, k))
        .unwrap_or_else(|| problem.to_string().into())
}

/// A check's or a run's error in plain words (the shell's `error_text`).
pub fn update_error_text(error: Option<&UpdateError>, cx: &App) -> SharedString {
    match error {
        Some(e) => error_text(Some(&e.code), &e.message, cx),
        None => SharedString::default(),
    }
}

pub fn short_sha(commit: Option<&str>) -> String {
    commit.unwrap_or_default().chars().take(7).collect()
}

/// Preferences and About: where the installed version stands.
pub fn status_sentence(view: Option<&UpdateView>, now: u64, cx: &App) -> SharedString {
    let Some(view) = view else {
        return t(cx, "update.statusNoServe");
    };
    let Some(check) = &view.check else {
        return t(cx, "update.statusNeverChecked");
    };
    let when = hace(check.checked_ms, now, cx);
    match check.status.as_str() {
        "error" => tv(
            cx,
            "update.statusCheckFailed",
            &[
                ("when", &when),
                ("reason", &update_error_text(check.error.as_ref(), cx)),
            ],
        ),
        "available" => tv(
            cx,
            "update.statusAvailableLine",
            &[("line", &banner_line(check, cx)), ("when", &when)],
        ),
        _ => tv(cx, "update.statusUpToDate", &[("when", &when)]),
    }
}

/// The status chip's tone (`amber`, `red`, `green`, `""`) and word.
pub fn status_chip(check: Option<&UpdateCheck>, cx: &App) -> (&'static str, SharedString) {
    match check.map(|c| c.status.as_str()) {
        None => ("", t(cx, "update.chipNotChecked")),
        Some("available") => ("amber", t(cx, "update.statusAvailable")),
        Some("error") => ("red", t(cx, "update.chipFailed")),
        Some(_) => ("green", t(cx, "update.statusCurrent")),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    News,
    Fixes,
    Improvements,
    Other,
}

impl Group {
    pub fn label(self, cx: &App) -> SharedString {
        t(
            cx,
            match self {
                Group::News => "update.groupNew",
                Group::Fixes => "update.groupFixes",
                Group::Improvements => "update.groupImprovements",
                Group::Other => "update.groupOther",
            },
        )
    }
}

fn group_of(kind: &str) -> Group {
    match kind {
        "feat" => Group::News,
        "fix" => Group::Fixes,
        "perf" | "refactor" => Group::Improvements,
        _ => Group::Other,
    }
}

/// The commits by group, in a fixed order, empty groups left out.
pub fn group_changes(changes: &[UpdateChange]) -> Vec<(Group, Vec<&UpdateChange>)> {
    [Group::News, Group::Fixes, Group::Improvements, Group::Other]
        .into_iter()
        .map(|g| {
            (
                g,
                changes
                    .iter()
                    .filter(|c| group_of(&c.kind) == g)
                    .collect::<Vec<_>>(),
            )
        })
        .filter(|(_, v)| !v.is_empty())
        .collect()
}

/// A subject starts with a capital in the list, the way a changelog reads.
pub fn sentence_case(subject: &str) -> String {
    let s = subject.trim();
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// "0.3.0-alpha.10", or "main" when the number did not move.
pub fn target_name(check: &UpdateCheck, installed_version: &str) -> String {
    let Some(target) = &check.target else {
        return installed_version.to_string();
    };
    if let Some(tag) = &target.tag {
        return tag.strip_prefix('v').unwrap_or(tag).to_string();
    }
    if target.version != installed_version {
        target.version.clone()
    } else {
        "main".into()
    }
}

pub fn target_when(check: &UpdateCheck, now: u64, cx: &App) -> Option<SharedString> {
    check
        .target
        .as_ref()?
        .date_ms
        .filter(|ms| *ms > 0)
        .map(|ms| at_date(ms, now, cx))
}

/// The steps to draw: serve's, or every step pending before the first answer.
pub fn run_steps(run: Option<&UpdateRun>) -> Vec<UpdateStep> {
    match run {
        Some(r) if !r.steps.is_empty() => r.steps.clone(),
        _ => STEP_ORDER
            .iter()
            .map(|id| UpdateStep {
                id: (*id).into(),
                state: "pending".into(),
                ..Default::default()
            })
            .collect(),
    }
}

/// "12 s", "1 min 12 s", "2 h 14 min".
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

/// The running clock for a step in progress, its length once done.
pub fn step_elapsed(step: &UpdateStep, now: u64) -> Option<String> {
    let started = step.started_ms?;
    if step.state == "skipped" || step.state == "pending" {
        return None;
    }
    Some(clock(
        step.finished_ms.unwrap_or(now).saturating_sub(started),
    ))
}

pub fn step_state_text(state: &str, cx: &App) -> SharedString {
    t(
        cx,
        match state {
            "running" => "update.stepRunning",
            "done" => "update.stepDone",
            "failed" => "update.stepFailed",
            "skipped" => "update.stepSkipped",
            _ => "update.stepPending",
        },
    )
}

/// Where a run is, for the pill: the step in hand, counted among the steps
/// that run on this Mac, and how long that step has gone on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Progress {
    /// 1-based, among the steps not skipped.
    pub index: usize,
    pub total: usize,
    pub step: String,
    /// Since the step started; `None` between steps.
    pub elapsed_ms: Option<u64>,
}

/// The step in hand: the one running, else the next one pending (serve is
/// between steps). Skipped steps count in neither the place nor the total.
pub fn run_progress(run: &UpdateRun, now: u64) -> Option<Progress> {
    if run.state != "running" {
        return None;
    }
    let steps: Vec<&UpdateStep> = run.steps.iter().filter(|s| s.state != "skipped").collect();
    let at = steps
        .iter()
        .position(|s| s.state == "running")
        .or_else(|| steps.iter().position(|s| s.state == "pending"))?;
    let step = steps[at];
    Some(Progress {
        index: at + 1,
        total: steps.len(),
        step: step.id.clone(),
        elapsed_ms: step
            .started_ms
            .filter(|_| step.state == "running")
            .map(|t| now.saturating_sub(t)),
    })
}

/// "0:07", "3:12", "1:02:09": a clock that ticks in place.
pub fn mmss(ms: u64) -> String {
    let s = ms / 1000;
    let (h, m, s) = (s / 3600, s / 60 % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// A failed run that no longer describes this Mac: what is installed now is
/// not the commit it started from, so a newer install happened since (by
/// `just update`, a release or another run) and the failure is history.
pub fn run_superseded(run: &UpdateRun, installed: Option<&str>) -> bool {
    if run.state != "failed" {
        return false;
    }
    let Some(installed) = installed.filter(|c| !c.is_empty()) else {
        return false;
    };
    let installed = installed.to_lowercase();
    !run.from.as_deref().is_some_and(|from| {
        let from = from.to_lowercase();
        from.starts_with(&installed) || installed.starts_with(&from)
    })
}

/// Where a step's detail goes: beside it (skipped) or under it (failed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailAt {
    Side,
    Below,
}

pub fn step_detail(step: &UpdateStep, cx: &App) -> Option<(SharedString, DetailAt)> {
    if step.state == "skipped" {
        let text = match step.detail.as_deref() {
            Some("not_installed") => t(cx, "update.skipNotInstalled"),
            Some("not_open") => t(cx, "update.skipNotOpen"),
            _ => step_state_text("skipped", cx),
        };
        return Some((text, DetailAt::Side));
    }
    if step.state == "failed"
        && let Some(d) = &step.detail
    {
        return Some((d.clone().into(), DetailAt::Below));
    }
    None
}

/// What a finished run brought, when serve kept it.
pub struct Brought<'a> {
    pub notes: Option<&'a str>,
    pub changes: &'a [UpdateChange],
    pub truncated: bool,
}

pub fn brought_by(run: Option<&UpdateRun>) -> Option<Brought<'_>> {
    let run = run?;
    if run.state != "done" {
        return None;
    }
    let notes = run
        .notes
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    let changes = run.changes.as_deref().unwrap_or(&[]);
    if notes.is_none() && changes.is_empty() {
        return None;
    }
    Some(Brought {
        notes,
        changes,
        truncated: run.changes_truncated.unwrap_or(false),
    })
}

pub fn channel_label(id: &str, cx: &App) -> SharedString {
    match id {
        "releases" => t(cx, "update.channelReleasesLabel"),
        _ => t(cx, "update.channelMainLabel"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::update::UpdateTarget;

    fn check(status: &str, commit: &str) -> UpdateCheck {
        UpdateCheck {
            status: status.into(),
            target: Some(UpdateTarget {
                commit: commit.into(),
                version: "0.3.0".into(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    #[test]
    fn groups_keep_their_order() {
        let c = |kind: &str, s: &str| UpdateChange {
            kind: kind.into(),
            subject: s.into(),
            ..Default::default()
        };
        let all = [
            c("docs", "d"),
            c("fix", "f1"),
            c("feat", "n"),
            c("perf", "p"),
            c("fix", "f2"),
        ];
        let groups = group_changes(&all);
        let ids: Vec<_> = groups.iter().map(|(g, v)| (*g, v.len())).collect();
        assert_eq!(
            ids,
            [
                (Group::News, 1),
                (Group::Fixes, 2),
                (Group::Improvements, 1),
                (Group::Other, 1)
            ]
        );
        assert_eq!(groups[1].1[1].subject, "f2");
    }

    #[test]
    fn clock_reads_like_the_console() {
        assert_eq!(clock(12_000), "12 s");
        assert_eq!(clock(72_000), "1 min 12 s");
        assert_eq!(clock(120_000), "2 min");
        assert_eq!(clock((2 * 60 + 14) * 60_000), "2 h 14 min");
        assert_eq!(clock(3 * 3_600_000), "3 h");
    }

    #[test]
    fn target_names() {
        let mut c = check("available", "abc");
        assert_eq!(target_name(&c, "0.3.0"), "main");
        assert_eq!(target_name(&c, "0.2.9"), "0.3.0");
        c.target.as_mut().unwrap().tag = Some("v0.3.1".into());
        assert_eq!(target_name(&c, "0.3.0"), "0.3.1");
        assert_eq!(sentence_case("  add a thing"), "Add a thing");
    }

    #[test]
    fn pending_steps_before_serve_answers() {
        let steps = run_steps(None);
        assert_eq!(steps.len(), 6);
        assert!(steps.iter().all(|s| s.state == "pending"));
    }

    fn step(id: &str, state: &str, started: Option<u64>) -> UpdateStep {
        UpdateStep {
            id: id.into(),
            state: state.into(),
            started_ms: started,
            ..Default::default()
        }
    }

    #[test]
    fn progress_counts_the_steps_that_run_here_and_times_the_one_in_hand() {
        let mut run = UpdateRun {
            state: "running".into(),
            steps: vec![
                step("fetch", "done", Some(0)),
                step("cli", "done", Some(10)),
                step("hooks", "skipped", None),
                step("serve", "done", Some(20)),
                step("console", "running", Some(1_000)),
                step("desktop", "pending", None),
            ],
            ..Default::default()
        };
        let p = run_progress(&run, 193_000).unwrap();
        assert_eq!((p.index, p.total, p.step.as_str()), (4, 5, "console"));
        assert_eq!(p.elapsed_ms, Some(192_000));
        // Between steps: the next one, with no clock yet.
        run.steps[4].state = "done".into();
        let p = run_progress(&run, 200_000).unwrap();
        assert_eq!(
            (p.index, p.step.as_str(), p.elapsed_ms),
            (5, "desktop", None)
        );
        run.state = "failed".into();
        assert_eq!(run_progress(&run, 0), None);
    }

    #[test]
    fn the_clock_ticks_in_minutes_and_seconds() {
        assert_eq!(mmss(7_400), "0:07");
        assert_eq!(mmss(192_000), "3:12");
        assert_eq!(mmss(3_729_000), "1:02:09");
    }

    #[test]
    fn a_failure_is_history_once_something_newer_is_installed() {
        let run = UpdateRun {
            state: "failed".into(),
            from: Some("f33a51e4ea".into()),
            to: Some("ef6f5e618b".into()),
            ..Default::default()
        };
        // 11 Nov 2026: the run to ef6f5e6 failed, then 0faaa43 was installed.
        assert!(run_superseded(&run, Some("0faaa4381e")));
        // Nothing installed since: the failure still stands.
        assert!(!run_superseded(&run, Some("f33a51e")));
        assert!(!run_superseded(&run, None));
        let done = UpdateRun {
            state: "done".into(),
            ..run
        };
        assert!(!run_superseded(&done, Some("0faaa43")));
    }
}
