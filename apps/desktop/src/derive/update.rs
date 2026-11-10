//! "Update harbard" (`lib/update.ts`): the labels by id and the pure
//! decisions the banner, the menu, the dialog and Preferences share.

use crate::i18n::{t, tv};
use crate::model::prefs_update::{UpdateChange, UpdateCheck, UpdateError, UpdateRun, UpdateStep, UpdateView};
use crate::ui::format::{at_date, hace};
use gpui::{App, SharedString};

pub const STEP_ORDER: [&str; 6] = ["fetch", "cli", "hooks", "serve", "console", "desktop"];

/// How often the view is read while a run goes on, and otherwise.
pub const RUN_POLL_MS: u64 = 1_500;
pub const IDLE_POLL_MS: u64 = 10 * 60_000;
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

fn error_key(code: &str) -> Option<&'static str> {
    if let Some(k) = problem_key(code) {
        return Some(k);
    }
    Some(match code {
        "fetch_failed" => "update.errorFetchFailed",
        "step_failed" => "update.errorStepFailed",
        "interrupted" => "update.errorInterrupted",
        "busy" => "update.errorBusy",
        "forbidden_origin" | "loopback_only" => "update.errorLocalOnly",
        "role_forbids" => "update.errorRoleForbids",
        _ => return None,
    })
}

/// A refusal or failure in plain words; an unknown code keeps serve's message.
pub fn error_text(error: Option<&UpdateError>, cx: &App) -> SharedString {
    let Some(error) = error else {
        return SharedString::default();
    };
    match error_key(&error.code) {
        Some(k) => t(cx, k),
        None if !error.message.is_empty() => error.message.clone().into(),
        None => t(cx, "update.errorFallback"),
    }
}

pub fn short_sha(commit: Option<&str>) -> String {
    commit.unwrap_or_default().chars().take(7).collect()
}

/// "1 change", "11 changes".
pub fn changes_count(n: u64, cx: &App) -> SharedString {
    if n == 1 {
        t(cx, "update.changesOne")
    } else {
        tv(cx, "update.changesMany", &[("count", &n.to_string())])
    }
}

/// The banner's line: the headline, and how many changes when known.
pub fn banner_line(check: &UpdateCheck, cx: &App) -> SharedString {
    if check.behind > 0 {
        format!("{} · {}", t(cx, "update.bannerTitle"), changes_count(check.behind, cx)).into()
    } else {
        t(cx, "update.bannerTitle")
    }
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
            &[("when", &when), ("reason", &error_text(check.error.as_ref(), cx))],
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
pub enum BannerMode {
    Available,
    Running,
    Done,
}

/// The app was built before the run's target: it needs a restart to show it.
pub fn page_is_stale(run: &UpdateRun, page_commit: &str) -> bool {
    let Some(to) = run.to.as_deref() else { return false };
    if run.state != "done" || to.is_empty() || page_commit.is_empty() {
        return false;
    }
    let a = to.to_ascii_lowercase();
    let b = page_commit.to_ascii_lowercase();
    !(a.starts_with(&b) || b.starts_with(&a))
}

/// What the title bar's pill shows. A run in progress, or one that finished
/// while this app runs an older build, always shows; a new version shows
/// until "Later" for that target.
pub fn banner_mode(
    view: Option<&UpdateView>,
    dismissed: Option<&str>,
    page_commit: &str,
    restarting: bool,
) -> Option<BannerMode> {
    if restarting {
        return Some(BannerMode::Running);
    }
    let view = view?;
    if let Some(run) = &view.run {
        if run.state == "running" {
            return Some(BannerMode::Running);
        }
        if run.state == "done" && page_is_stale(run, page_commit) {
            return Some(BannerMode::Done);
        }
    }
    let check = view.check.as_ref()?;
    let target = check.target.as_ref()?;
    if check.status != "available" {
        return None;
    }
    (dismissed != Some(target.commit.as_str())).then_some(BannerMode::Available)
}

/// A finished run this app has not said "updated" for yet (within 10 min).
pub fn toast_for(run: Option<&UpdateRun>, now: u64, acked: Option<&str>) -> bool {
    let Some(run) = run else { return false };
    let Some(finished) = run.finished_ms else { return false };
    if run.state != "done" {
        return false;
    }
    if now.saturating_sub(finished) > 10 * 60_000 || now + 60_000 < finished {
        return false;
    }
    acked != Some(run.id.as_str())
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
        .map(|g| (g, changes.iter().filter(|c| group_of(&c.kind) == g).collect::<Vec<_>>()))
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
    Some(clock(step.finished_ms.unwrap_or(now).saturating_sub(started)))
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
    let notes = run.notes.as_deref().map(str::trim).filter(|n| !n.is_empty());
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
    use crate::model::prefs_update::UpdateTarget;

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
    fn the_banner_follows_run_then_check() {
        let mut view = UpdateView {
            check: Some(check("available", "abc1234")),
            ..Default::default()
        };
        assert_eq!(banner_mode(Some(&view), None, "", false), Some(BannerMode::Available));
        assert_eq!(banner_mode(Some(&view), Some("abc1234"), "", false), None);
        assert_eq!(banner_mode(None, None, "", true), Some(BannerMode::Running));
        view.run = Some(UpdateRun {
            state: "done".into(),
            to: Some("def5678".into()),
            ..Default::default()
        });
        assert_eq!(banner_mode(Some(&view), None, "abc", false), Some(BannerMode::Done));
        assert_eq!(banner_mode(Some(&view), Some("abc1234"), "def5678aa", false), None);
    }

    #[test]
    fn stale_compares_by_prefix() {
        let run = UpdateRun {
            state: "done".into(),
            to: Some("ABCDEF1".into()),
            ..Default::default()
        };
        assert!(!page_is_stale(&run, "abcdef1234"));
        assert!(page_is_stale(&run, "1234567"));
        assert!(!page_is_stale(&run, ""));
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
    fn toast_once_per_run_within_ten_minutes() {
        let run = UpdateRun {
            id: "r1".into(),
            state: "done".into(),
            finished_ms: Some(1_000_000),
            ..Default::default()
        };
        assert!(toast_for(Some(&run), 1_000_000 + 60_000, None));
        assert!(!toast_for(Some(&run), 1_000_000 + 60_000, Some("r1")));
        assert!(!toast_for(Some(&run), 1_000_000 + 11 * 60_000, None));
    }

    #[test]
    fn pending_steps_before_serve_answers() {
        let steps = run_steps(None);
        assert_eq!(steps.len(), 6);
        assert!(steps.iter().all(|s| s.state == "pending"));
    }
}
