//! Deliveries (`lib/deliveries.ts`): where each DONE's commit has got, as
//! serve derived it. This only words it and draws the steps.

use crate::i18n::{t, tv};
use crate::model::{DeliveryRow, ReviewStep, Shipment};
use crate::ui::format::{at_date, hhmm};
use gpui::{App, SharedString};

pub const ORDER: [&str; 5] = [
    "delivered",
    "in_review",
    "with_fixes",
    "integrated",
    "installed",
];

pub fn state_label(state: &str, cx: &App) -> SharedString {
    match state {
        "in_review" => t(cx, "deliveries.stateInReview"),
        "with_fixes" => t(cx, "deliveries.stateWithFixes"),
        "integrated" => t(cx, "deliveries.stateIntegrated"),
        "installed" => t(cx, "deliveries.stateInstalled"),
        _ => t(cx, "deliveries.stateDelivered"),
    }
}

/// The state colour a delivery state may wear (C3): amber waits on a
/// person, red came back with fixes, green is installed and verified.
/// Delivered and integrated are steps, not states, so they stay neutral.
pub fn state_tone(state: &str) -> &'static str {
    match state {
        "in_review" => "amber",
        "with_fixes" => "red",
        "installed" => "green",
        _ => "neutral",
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    pub id: &'static str,
    /// It went through it; a review it never had is skipped, not reached.
    pub reached: bool,
    /// Where it is now.
    pub current: bool,
}

fn index(state: &str) -> Option<usize> {
    ORDER.iter().position(|s| *s == state)
}

/// Four steps outside serve's repository, five in it.
pub fn steps(s: &Shipment) -> Vec<Step> {
    let up_to = index(&s.last_state).unwrap_or(3);
    let at = index(&s.state);
    let integrated = index("integrated").unwrap_or(3);
    ORDER[..=up_to]
        .iter()
        .map(|&id| {
            let reached = match id {
                "delivered" => true,
                "in_review" => !s.review.is_empty(),
                "with_fixes" => {
                    s.review.iter().any(|r| r.kind == "fixes") || s.state == "with_fixes"
                }
                "integrated" => at.is_some_and(|a| a >= integrated),
                _ => s.state == "installed",
            };
            Step {
                id,
                reached,
                current: id == s.state,
            }
        })
        .collect()
}

/// How a part of a delivery's outcome reads: `Muted` for what has not
/// happened yet, the state colours for what did (C3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Neutral,
    Muted,
    Green,
    Amber,
    Red,
}

impl Tone {
    /// The tone the stream's `state_chip` takes.
    pub fn chip(self) -> &'static str {
        match self {
            Tone::Green => "green",
            Tone::Amber => "amber",
            Tone::Red => "red",
            Tone::Neutral | Tone::Muted => "neutral",
        }
    }
}

/// One part of the outcome line: a dictionary key and how it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Part {
    pub key: &'static str,
    pub tone: Tone,
}

fn part(key: &'static str, tone: Tone) -> Part {
    Part { key, tone }
}

fn same_commit(a: &str, b: &str) -> bool {
    a.len() >= 7 && b.len() >= 7 && (a.starts_with(b) || b.starts_with(a))
}

/// The integration queue's last verdict on this delivery's commit
/// (`harbard deliver` / `integrate`: built on the tip and checked), or
/// `None` when the queue never saw it. Nothing else on the bus says whether
/// a delivery was verified.
pub fn check_verdict<'a>(s: &Shipment, queue: &'a [DeliveryRow]) -> Option<&'a str> {
    let names: Vec<&str> = [
        s.sha.as_deref(),
        s.commit.as_deref(),
        s.landed_as.as_deref(),
    ]
    .into_iter()
    .flatten()
    .collect();
    queue
        .iter()
        .filter(|d| names.iter().any(|n| same_commit(n, &d.sha)))
        .max_by_key(|d| d.decided_ms.unwrap_or(d.created_ms))
        .map(|d| d.verdict.as_str())
}

fn check_part(verdict: Option<&str>) -> Option<Part> {
    Some(match verdict? {
        "landed" => part("deliveries.outcomeVerified", Tone::Green),
        "rejected" => part("deliveries.outcomeVerifyFailed", Tone::Red),
        "conflict" => part("deliveries.outcomeConflict", Tone::Red),
        "failed" => part("deliveries.outcomeCheckBroke", Tone::Amber),
        "pending" => part("deliveries.outcomeVerifying", Tone::Neutral),
        _ => return None,
    })
}

/// Where git has the commit against where it can go: a delivery the agent
/// called done is not integrated until git says so.
fn landing_part(s: &Shipment) -> Option<Part> {
    if s.untracked.is_some() {
        return None;
    }
    Some(match s.state.as_str() {
        "installed" => part("deliveries.outcomeInstalled", Tone::Green),
        "integrated" if s.last_state == "integrated" => {
            part("deliveries.outcomeIntegrated", Tone::Green)
        }
        "integrated" => part("deliveries.outcomeNotInstalled", Tone::Muted),
        _ => part("deliveries.outcomeNotIntegrated", Tone::Muted),
    })
}

/// The outcome line (`Submitted · Verification failed · Not integrated`):
/// the agent's claim, the queue's check when there was one, and where git
/// has the commit. Built only from what serve reports.
pub fn outcome(s: &Shipment, verdict: Option<&str>) -> Vec<Part> {
    let mut parts = vec![part("deliveries.outcomeSubmitted", Tone::Neutral)];
    parts.extend(check_part(verdict));
    parts.extend(landing_part(s));
    parts
}

/// The one word a delivery's message wears in the stream: its furthest
/// fact, a failure before a wait, and never green before git or the queue
/// says so.
pub fn headline(s: &Shipment, verdict: Option<&str>) -> Part {
    let check = check_part(verdict);
    if let Some(p) = landing_part(s).filter(|p| p.tone == Tone::Green) {
        return p;
    }
    if let Some(p) = check.filter(|p| p.tone == Tone::Red) {
        return p;
    }
    match s.state.as_str() {
        "with_fixes" => return part("deliveries.stateWithFixes", Tone::Red),
        "in_review" => return part("deliveries.stateInReview", Tone::Amber),
        "integrated" => return part("deliveries.stateIntegrated", Tone::Neutral),
        _ => {}
    }
    match check {
        Some(p) if p.tone == Tone::Amber => p,
        _ => part("deliveries.outcomeSubmitted", Tone::Neutral),
    }
}

/// When it was delivered: the row's own timestamp, beside the name.
pub fn delivered_when(s: &Shipment, now: u64, cx: &App) -> SharedString {
    at_date(s.ts_ms, now, cx)
}

/// When each review step happened; `None` for a delivery nobody has
/// reviewed yet (its steps already say "Delivered").
pub fn review_times(s: &Shipment, cx: &App) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    let first = |kind: &str| s.review.iter().find(|r| r.kind == kind);
    if let Some(r) = first("reviewing") {
        parts.push(tv(cx, "deliveries.timesReview", &[("time", &hhmm(r.ts_ms))]).into());
    }
    let fixes: Option<&ReviewStep> = first("fixes");
    if let Some(r) = fixes {
        parts.push(tv(cx, "deliveries.timesFixes", &[("time", &hhmm(r.ts_ms))]).into());
    }
    if fixes.is_none()
        && let Some(r) = s.review.iter().rev().find(|r| r.kind == "reviewed")
    {
        parts.push(
            tv(
                cx,
                "deliveries.timesCleanReview",
                &[("time", &hhmm(r.ts_ms))],
            )
            .into(),
        );
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

pub fn untracked_text(why: Option<&str>, cx: &App) -> Option<SharedString> {
    match why? {
        "no_commit" => Some(t(cx, "deliveries.untrackedNoCommit")),
        "unknown_commit" => Some(t(cx, "deliveries.untrackedUnknownCommit")),
        "no_repository" => Some(t(cx, "deliveries.untrackedNoRepo")),
        _ => None,
    }
}

/// Which installed line to show: `None` when serve names no commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Installed {
    Unknown,
    Same(String),
    Mismatch { serve: String, app: String },
}

fn short(c: &str) -> String {
    c.chars().take(7).collect()
}

/// The commit serve runs against the one this app was built from.
pub fn installed(running: Option<&str>, app_commit: &str) -> Installed {
    let Some(serve) = running.filter(|s| !s.is_empty()) else {
        return Installed::Unknown;
    };
    let same =
        !app_commit.is_empty() && (serve.starts_with(app_commit) || app_commit.starts_with(serve));
    if app_commit.is_empty() || same {
        return Installed::Same(short(serve));
    }
    Installed::Mismatch {
        serve: short(serve),
        app: short(app_commit),
    }
}

impl Installed {
    pub fn mismatch(&self) -> bool {
        !matches!(self, Installed::Same(_))
    }

    pub fn text(&self, cx: &App) -> SharedString {
        match self {
            Installed::Unknown => t(cx, "deliveries.installedUnknown"),
            Installed::Same(c) => tv(cx, "deliveries.installedSame", &[("commit", c)]),
            Installed::Mismatch { serve, app } => tv(
                cx,
                "deliveries.installedMismatch",
                &[("serve", serve), ("app", app)],
            ),
        }
    }
}

/// The commit this build came from, when the build stamped one
/// (`HARBARD_APP_COMMIT`, 7 to 40 hex digits).
pub fn app_commit() -> &'static str {
    let c = option_env!("HARBARD_APP_COMMIT").unwrap_or("");
    if (7..=40).contains(&c.len()) && c.chars().all(|ch| ch.is_ascii_hexdigit()) {
        c
    } else {
        ""
    }
}

pub fn verdict_text(verdict: &str, cx: &App) -> SharedString {
    match verdict {
        "landed" => t(cx, "deliveries.verdictLanded"),
        "conflict" => t(cx, "deliveries.verdictConflict"),
        "rejected" => t(cx, "deliveries.verdictRejected"),
        "failed" => t(cx, "deliveries.verdictFailed"),
        "pending" => t(cx, "deliveries.verdictPending"),
        other => other.to_string().into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ship(state: &str, last: &str, review: &[&str]) -> Shipment {
        Shipment {
            state: state.into(),
            last_state: last.into(),
            review: review
                .iter()
                .map(|k| ReviewStep {
                    kind: (*k).into(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        }
    }

    fn marks(s: &Shipment) -> Vec<(&'static str, bool, bool)> {
        steps(s)
            .into_iter()
            .map(|s| (s.id, s.reached, s.current))
            .collect()
    }

    fn queued(sha: &str, verdict: &str, at: u64) -> DeliveryRow {
        DeliveryRow {
            sha: sha.into(),
            verdict: verdict.into(),
            created_ms: at,
            ..Default::default()
        }
    }

    fn keys(parts: &[Part]) -> Vec<&'static str> {
        parts.iter().map(|p| p.key).collect()
    }

    #[test]
    fn a_failed_check_reads_as_failed_and_not_integrated() {
        let mut s = ship("delivered", "integrated", &[]);
        s.sha = Some("2ba4982".into());
        let queue = [
            queued("2ba49820aa", "pending", 1),
            queued("2ba49820aa", "rejected", 2),
            queued("ffffffffff", "landed", 3),
        ];
        let verdict = check_verdict(&s, &queue);
        assert_eq!(verdict, Some("rejected"));
        assert_eq!(
            keys(&outcome(&s, verdict)),
            vec![
                "deliveries.outcomeSubmitted",
                "deliveries.outcomeVerifyFailed",
                "deliveries.outcomeNotIntegrated",
            ]
        );
        let h = headline(&s, verdict);
        assert_eq!(
            (h.key, h.tone),
            ("deliveries.outcomeVerifyFailed", Tone::Red)
        );
    }

    #[test]
    fn without_a_check_or_main_a_delivery_is_only_submitted() {
        let s = ship("delivered", "installed", &[]);
        assert_eq!(check_verdict(&s, &[]), None);
        assert_eq!(
            keys(&outcome(&s, None)),
            vec![
                "deliveries.outcomeSubmitted",
                "deliveries.outcomeNotIntegrated"
            ]
        );
        assert_eq!(headline(&s, None).tone, Tone::Neutral);
    }

    #[test]
    fn green_only_once_git_has_it_where_it_can_go() {
        let done = ship("integrated", "integrated", &[]);
        assert_eq!(headline(&done, None).tone, Tone::Green);
        let not_yet = ship("integrated", "installed", &[]);
        assert_eq!(headline(&not_yet, None).tone, Tone::Neutral);
        assert_eq!(
            keys(&outcome(&not_yet, None))[1],
            "deliveries.outcomeNotInstalled"
        );
        let there = ship("installed", "installed", &[]);
        assert_eq!(
            headline(&there, Some("landed")).key,
            "deliveries.outcomeInstalled"
        );
        let fixes = ship("with_fixes", "integrated", &["fixes"]);
        assert_eq!(headline(&fixes, None).tone, Tone::Red);
        let mut untracked = ship("delivered", "integrated", &[]);
        untracked.untracked = Some("no_commit".into());
        assert_eq!(
            keys(&outcome(&untracked, None)),
            vec!["deliveries.outcomeSubmitted"]
        );
    }

    #[test]
    fn only_states_that_ask_for_someone_take_a_colour() {
        assert_eq!(state_tone("delivered"), "neutral");
        assert_eq!(state_tone("integrated"), "neutral");
        assert_eq!(state_tone("in_review"), "amber");
        assert_eq!(state_tone("with_fixes"), "red");
        assert_eq!(state_tone("installed"), "green");
    }

    #[test]
    fn steps_stop_at_the_last_state() {
        assert_eq!(steps(&ship("delivered", "integrated", &[])).len(), 4);
        assert_eq!(steps(&ship("delivered", "installed", &[])).len(), 5);
    }

    #[test]
    fn a_review_never_had_is_skipped_not_reached() {
        assert_eq!(
            marks(&ship("integrated", "installed", &[])),
            vec![
                ("delivered", true, false),
                ("in_review", false, false),
                ("with_fixes", false, false),
                ("integrated", true, true),
                ("installed", false, false),
            ]
        );
        assert_eq!(
            marks(&ship("installed", "installed", &["reviewing", "fixes"])),
            vec![
                ("delivered", true, false),
                ("in_review", true, false),
                ("with_fixes", true, false),
                ("integrated", true, false),
                ("installed", true, true),
            ]
        );
        let s = marks(&ship("with_fixes", "integrated", &["reviewing"]));
        assert_eq!(s[2], ("with_fixes", true, true));
    }

    #[test]
    fn installed_compares_by_prefix_either_way() {
        assert_eq!(installed(None, "abc1234"), Installed::Unknown);
        assert_eq!(
            installed(Some("c76268f0aa"), ""),
            Installed::Same("c76268f".into())
        );
        assert_eq!(
            installed(Some("c76268f0aa"), "c76268f"),
            Installed::Same("c76268f".into())
        );
        assert_eq!(
            installed(Some("c76268f"), "c76268f0aa"),
            Installed::Same("c76268f".into())
        );
        assert_eq!(
            installed(Some("c76268f0aa"), "1234567"),
            Installed::Mismatch {
                serve: "c76268f".into(),
                app: "1234567".into()
            }
        );
        assert!(Installed::Unknown.mismatch());
    }
}
