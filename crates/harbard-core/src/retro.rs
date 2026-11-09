//! Session retro: what a team's work says about how harbard's agents work,
//! and the rules it suggests.
//!
//! Computed from three sources, all already on this machine: the bus
//! (the operator's notes and their answers), the team projection
//! ([`crate::team`]: messages' first lines, labels, warnings and the stalls
//! the transcript fold keeps) and the operator's feedback ([`crate::feedback`]).
//! Nothing here reads a transcript body or a tool output: a sample carries
//! ids, safe labels, counts and timestamps only.
//!
//! A retro covers one finished team (every agent done or stopped) or a week.
//! Its lesson candidates are patterns that repeated, each with a short
//! title for the operator, the evidence (counts and dates) and the rule text
//! agents would receive if they approve it ([`crate::rules`]).
//!
//! The classifiers are keyword heuristics over first lines, written for the
//! way this fleet labels its messages (`FINDING · reviewer · …`, `DONE · …`).
//! They count; they do not judge.

use crate::coded::{Params, params};
use crate::event::{Event, EventKind, RecipientKind};
use crate::feedback::{Record, TargetKind, one_line};
use crate::team::{AgentRow, LeadPanel, TeamView};
use crate::workspace::WorkspaceView;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// A week, the default window.
pub const WEEK_MS: u64 = 7 * 24 * 3_600_000;
/// A note from the operator with no answer for this long is unanswered.
pub const UNANSWERED_MS: u64 = 15 * 60_000;
/// An idle agent quiet this long counts as idle time (no workspace view).
const IDLE_MS: u64 = 30 * 60_000;
/// Samples listed per metric.
const SAMPLES: usize = 20;

/// Finding classes, in the order they are shown.
pub const CLASSES: [&str; 6] = [
    "secret_leak",
    "file_race",
    "host_check",
    "wrong_state",
    "ui",
    "other",
];
/// Classes that are a security primitive: one owner, everyone reuses it.
const SECURITY: [&str; 3] = ["secret_leak", "file_race", "host_check"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope {
    /// Every lead with work in the window.
    Week,
    /// One lead session's team.
    Team(String),
}

pub struct RetroInput<'a> {
    pub now_ms: u64,
    pub since_ms: u64,
    pub scope: Scope,
    pub events: &'a [Event],
    pub team: &'a TeamView,
    /// The workspace read, for dormancy and message chips when available.
    pub view: Option<&'a WorkspaceView>,
    pub feedback: &'a [Record],
    /// The workshop's votes on names (`names.json`): the one source of
    /// name feedback.
    pub name_votes: &'a [crate::workshop::Vote],
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Retro {
    pub v: u32,
    pub now_ms: u64,
    pub since_ms: u64,
    /// `week` or `team`.
    pub scope: &'static str,
    pub lead: Option<String>,
    pub agents: usize,
    pub delivery: DeliveryMetrics,
    pub messages: MessageMetrics,
    pub waiting: WaitingMetrics,
    pub harness: HarnessMetrics,
    pub names: NameMetrics,
    /// Standing taps per target kind and value.
    pub feedback: BTreeMap<String, BTreeMap<String, u64>>,
    /// One point per UTC day of the window, oldest first: the sparklines.
    pub daily: Vec<DayPoint>,
    pub lessons: Vec<Lesson>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DeliveryMetrics {
    /// Minutes from an agent's brief to its first message about an install.
    pub brief_to_install_min: Vec<AgentMinutes>,
    pub brief_to_install_p50_min: Option<u64>,
    /// Review passes (a reviewer's DONE) per branch.
    pub review_rounds: BTreeMap<String, u64>,
    pub findings: Vec<FindingClass>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgentMinutes {
    pub agent: String,
    pub minutes: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FindingClass {
    pub class: &'static str,
    pub count: u64,
    pub branches: Vec<String>,
    /// Seen on two or more branches.
    pub repeats: bool,
    pub days: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct MessageMetrics {
    /// Messages the operator marked "I do not understand it".
    pub not_understood: u64,
    /// Message chips flagging jargon, when the view carries them.
    pub jargon_chips: u64,
    /// The operator's notes asking what something means.
    pub what_does_it_mean: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct WaitingMetrics {
    pub unanswered: Vec<Unanswered>,
    pub answered: u64,
    pub answer_p50_min: Option<u64>,
    /// Minutes agents spent idle or dormant, summed.
    pub idle_min: u64,
    /// Agents that ended a turn waiting on a background job.
    pub stragglers: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Unanswered {
    pub note: String,
    pub at_ms: u64,
    pub open_min: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct HarnessMetrics {
    pub stalls: Vec<StallSample>,
    pub stall_min: u64,
    /// Two agents writing one checkout.
    pub two_writers: u64,
    /// Messages repeated byte for byte.
    pub repeats: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StallSample {
    pub agent: String,
    pub tool: &'static str,
    pub pattern: &'static str,
    pub at_ms: u64,
    pub minutes: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct NameMetrics {
    /// 👎 on a name (workshop votes).
    pub names_down: u64,
    /// Names the operator renamed (workshop votes).
    pub renamed: u64,
    /// Names with numbers, hashes or `-2` suffixes, or no name at all.
    pub generated: Vec<String>,
    /// Agents whose description, shown to the operator, reads as English.
    pub english_descriptions: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct DayPoint {
    pub day: String,
    pub findings: u64,
    pub not_understood: u64,
    pub stalls: u64,
    pub unanswered: u64,
}

/// A rule the retro suggests. `key` is stable: approving, discarding or
/// retiring it is remembered by key, so it is not proposed again.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Lesson {
    pub key: &'static str,
    pub title: &'static str,
    pub evidence: String,
    /// The console's code for `evidence`, filled from `evidence_params`.
    pub evidence_code: &'static str,
    #[serde(skip_serializing_if = "Params::is_empty")]
    pub evidence_params: Params,
    pub rule: String,
    pub count: u64,
    pub dates: Vec<String>,
}

// ---------------------------------------------------------------------------
// Classifiers
// ---------------------------------------------------------------------------

fn fold(s: &str) -> String {
    s.to_lowercase()
        .chars()
        .map(|c| match c {
            'á' | 'à' => 'a',
            'é' | 'è' => 'e',
            'í' => 'i',
            'ó' | 'ò' => 'o',
            'ú' | 'ü' => 'u',
            other => other,
        })
        .collect()
}

/// The class of a finding, from its first line.
pub fn finding_class(first_line: &str) -> &'static str {
    let t = fold(first_line);
    let any = |words: &[&str]| words.iter().any(|w| t.contains(w));
    if any(&[
        "leak",
        "fuga",
        "filtra",
        "secret",
        "token",
        "contrasena",
        "password",
        "redact",
        "enmascar",
        "credencial",
        "credential",
    ]) {
        "secret_leak"
    } else if any(&["race", "carrera", "toctou", "symlink", "enlace simbolico"]) {
        "file_race"
    } else if any(&["host", "rebinding", "origin", "origen"]) {
        "host_check"
    } else if any(&[
        "wrong state",
        "estado incorrecto",
        "estado erroneo",
        "estado equivocado",
        "mislabel",
    ]) {
        "wrong_state"
    } else if any(&[
        " ui",
        "ui ",
        "css",
        "layout",
        "foco",
        "focus",
        "contraste",
        "accesib",
        "pantalla",
    ]) {
        "ui"
    } else {
        "other"
    }
}

/// The branch a reviewer's or builder's message is about: the agent's name
/// without its review suffix and `harbard-` prefix
/// (`direct-review-2` → `direct`, `harbard-profile` → `profile`).
pub fn branch_of(agent: &str) -> String {
    let mut name = agent.trim().to_lowercase();
    if let Some((head, tail)) = name.rsplit_once('-')
        && !tail.is_empty()
        && tail.bytes().all(|b| b.is_ascii_digit())
    {
        name = head.to_string();
    }
    for suffix in ["-review", "-reviewer", "-revision"] {
        if let Some(head) = name.strip_suffix(suffix) {
            name = head.to_string();
        }
    }
    if let Some(rest) = name.strip_prefix("review-") {
        name = rest.to_string();
    }
    if let Some(rest) = name.strip_prefix("harbard-") {
        name = rest.to_string();
    }
    one_line(&name, 48).unwrap_or_else(|| "no-branch".into())
}

fn is_reviewer(agent: &str) -> bool {
    let n = agent.to_lowercase();
    n.contains("review") || n.contains("revis")
}

/// A name nobody would choose: none at all, a numeric suffix, or a run of
/// hex or digits.
pub fn looks_generated(name: Option<&str>) -> bool {
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
        return true;
    };
    if let Some((_, tail)) = name.rsplit_once('-')
        && !tail.is_empty()
        && tail.bytes().all(|b| b.is_ascii_digit())
    {
        return true;
    }
    let mut digits = 0;
    let mut hexrun = 0;
    for c in name.chars() {
        if c.is_ascii_digit() {
            digits += 1;
        }
        if c.is_ascii_hexdigit() {
            hexrun += 1;
            if hexrun >= 6 && digits > 0 {
                return true;
            }
        } else {
            hexrun = 0;
        }
    }
    digits >= 3
}

/// Whether a text reads as English, by its function words.
pub fn reads_english(text: &str) -> bool {
    const EN: [&str; 16] = [
        "the", "and", "of", "for", "with", "to", "build", "review", "fix", "add", "write", "check",
        "branch", "test", "make", "run",
    ];
    // Spanish function words too, so a line written in Spanish is not counted
    // as all content words.
    const ES: [&str; 10] = [
        "el", "la", "los", "las", "de", "del", "y", "para", "con", "en",
    ];
    let words: Vec<String> = fold(text)
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_string)
        .collect();
    let en = words.iter().filter(|w| EN.contains(&w.as_str())).count();
    let es = words.iter().filter(|w| ES.contains(&w.as_str())).count();
    en > es
}

/// «¿Qué quiere decir…?», «¿qué significa…?», «no entiendo».
pub fn asks_what_it_means(text: &str) -> bool {
    let t = fold(text);
    [
        "que quiere decir",
        "que significa",
        "no entiendo",
        "no lo entiendo",
        "what does",
        "what is meant",
    ]
    .iter()
    .any(|p| t.contains(p))
}

/// Leads whose every agent is done or stopped.
pub fn finished_teams(team: &TeamView) -> Vec<String> {
    team.leads
        .iter()
        .filter(|l| {
            !l.agents.is_empty()
                && l.agents
                    .iter()
                    .all(|a| matches!(a.state, "done" | "stopped"))
        })
        .map(|l| l.session.clone())
        .collect()
}

fn day(ms: u64) -> String {
    crate::clock::rfc3339_utc(ms).chars().take(10).collect()
}

fn p50(mut v: Vec<u64>) -> Option<u64> {
    if v.is_empty() {
        return None;
    }
    v.sort_unstable();
    Some(v[v.len() / 2])
}

fn name_of(a: &AgentRow) -> String {
    a.name
        .as_deref()
        .and_then(|n| one_line(n, 48))
        .unwrap_or_else(|| format!("agent {}", one_line(&a.id, 12).unwrap_or_default()))
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

// ---------------------------------------------------------------------------
// The retro
// ---------------------------------------------------------------------------

pub fn compute(input: &RetroInput<'_>) -> Retro {
    let since = input.since_ms;
    let now = input.now_ms;
    let in_window = |ms: u64| ms >= since && ms <= now;
    let leads: Vec<&LeadPanel> = input
        .team
        .leads
        .iter()
        .filter(|l| match &input.scope {
            Scope::Week => true,
            Scope::Team(s) => &l.session == s,
        })
        .collect();
    let agents: Vec<&AgentRow> = leads
        .iter()
        .flat_map(|l| l.agents.iter())
        .filter(|a| input.scope != Scope::Week || a.seen_ms >= since)
        .collect();
    let lead_sessions: BTreeSet<&str> = leads.iter().map(|l| l.session.as_str()).collect();

    // --- delivery -------------------------------------------------------
    let mut delivery = DeliveryMetrics::default();
    let mut classes: BTreeMap<&'static str, (u64, BTreeSet<String>, BTreeSet<String>)> =
        BTreeMap::new();
    let mut finding_days: BTreeMap<String, u64> = BTreeMap::new();
    for a in &agents {
        let name = name_of(a);
        let outs = a
            .messages
            .iter()
            .filter(|m| m.dir == "out" && in_window(m.ts_ms));
        let mut installed: Option<u64> = None;
        for m in outs {
            let label = m.label.as_deref();
            if label == Some("FINDING") {
                let class = finding_class(&m.first_line);
                let e = classes.entry(class).or_default();
                e.0 += 1;
                e.1.insert(branch_of(&name));
                e.2.insert(day(m.ts_ms));
                *finding_days.entry(day(m.ts_ms)).or_default() += 1;
            }
            if label == Some("DONE") && is_reviewer(&name) {
                *delivery.review_rounds.entry(branch_of(&name)).or_default() += 1;
            }
            if installed.is_none() && !is_reviewer(&name) && fold(&m.first_line).contains("instal")
            {
                installed = Some(m.ts_ms);
            }
        }
        if let (Some(start), Some(at)) = (a.first_ms, installed) {
            delivery.brief_to_install_min.push(AgentMinutes {
                agent: name.clone(),
                minutes: at.saturating_sub(start) / 60_000,
            });
        }
    }
    delivery.brief_to_install_p50_min = p50(delivery
        .brief_to_install_min
        .iter()
        .map(|a| a.minutes)
        .collect());
    delivery.findings = CLASSES
        .iter()
        .filter_map(|c| {
            let (count, branches, days) = classes.remove(c)?;
            Some(FindingClass {
                class: c,
                count,
                repeats: branches.len() >= 2,
                branches: branches.into_iter().collect(),
                days: days.into_iter().collect(),
            })
        })
        .collect();

    // --- feedback, messages ----------------------------------------------
    let taps: Vec<&Record> = input
        .feedback
        .iter()
        .filter(|r| !r.undone && in_window(r.ms))
        .collect();
    let mut feedback: BTreeMap<String, BTreeMap<String, u64>> = BTreeMap::new();
    for r in &taps {
        *feedback
            .entry(r.target_kind.as_str().to_string())
            .or_default()
            .entry(r.value.clone())
            .or_default() += 1;
    }
    let count = |kind: TargetKind, value: &str| {
        taps.iter()
            .filter(|r| r.target_kind == kind && r.value == value)
            .count() as u64
    };
    let operator_notes: Vec<&Event> = input
        .events
        .iter()
        .filter(|e| e.event == EventKind::Note && e.is_operator())
        .filter(|e| in_window(crate::query::event_ms(e)))
        .collect();
    let messages = MessageMetrics {
        not_understood: count(TargetKind::Message, "unclear"),
        jargon_chips: input.view.map_or(0, |v| {
            v.messages
                .iter()
                .filter(|m| in_window(m.ts_ms))
                .filter(|m| m.flags.iter().any(|f| matches!(*f, "jargon" | "jerga")))
                .count() as u64
        }),
        what_does_it_mean: operator_notes
            .iter()
            .filter(|e| asks_what_it_means(&e.text))
            .count() as u64,
    };

    // --- waiting -----------------------------------------------------------
    let mut waiting = WaitingMetrics::default();
    let mut answer_min = Vec::new();
    let mut unanswered_days: BTreeMap<String, u64> = BTreeMap::new();
    for note in operator_notes.iter().filter(|e| e.to.is_some()) {
        let to = note.to.as_ref();
        let addressed_to_scope = input.scope == Scope::Week
            || to.is_some_and(|t| {
                t.kind == RecipientKind::Session && lead_sessions.contains(t.value.as_str())
            });
        if !addressed_to_scope {
            continue;
        }
        let at = crate::query::event_ms(note);
        let answer = input.events.iter().find(|e| {
            e.id > note.id
                && !e.is_operator()
                && (e.note_ref.as_deref() == Some(note.id.as_str())
                    || (e.event == EventKind::Ack
                        && e.acknowledged_note_id() == Some(note.id.as_str()))
                    || (e.event == EventKind::Note
                        && to.is_some_and(|t| {
                            t.kind == RecipientKind::Session && t.value == e.session
                        })))
        });
        match answer {
            Some(a) => {
                waiting.answered += 1;
                answer_min.push(crate::query::event_ms(a).saturating_sub(at) / 60_000);
            }
            None if now.saturating_sub(at) >= UNANSWERED_MS => {
                *unanswered_days.entry(day(at)).or_default() += 1;
                if waiting.unanswered.len() < SAMPLES {
                    waiting.unanswered.push(Unanswered {
                        note: note.id.clone(),
                        at_ms: at,
                        open_min: now.saturating_sub(at) / 60_000,
                    });
                }
            }
            None => {}
        }
    }
    waiting.answer_p50_min = p50(answer_min);
    waiting.idle_min = match input.view {
        Some(v) => v
            .agents
            .iter()
            .filter(|a| a.dormant.is_some())
            .filter_map(|a| a.last_action_ms)
            .map(|ms| now.saturating_sub(ms) / 60_000)
            .sum(),
        None => agents
            .iter()
            .filter(|a| a.state == "idle")
            .filter_map(|a| a.last_action_ms)
            .filter(|ms| now.saturating_sub(*ms) >= IDLE_MS)
            .map(|ms| now.saturating_sub(ms) / 60_000)
            .sum(),
    };
    waiting.stragglers = agents
        .iter()
        .filter(|a| a.activity.state == "waiting")
        .map(|a| name_of(a))
        .collect();

    // --- harness -------------------------------------------------------------
    let mut harness = HarnessMetrics::default();
    let mut stall_days: BTreeMap<String, u64> = BTreeMap::new();
    for a in &agents {
        for s in a.progress.stalls.iter().filter(|s| in_window(s.started_ms)) {
            *stall_days.entry(day(s.started_ms)).or_default() += 1;
            harness.stall_min += s.gap_ms / 60_000;
            if harness.stalls.len() < SAMPLES {
                harness.stalls.push(StallSample {
                    agent: name_of(a),
                    tool: s.tool,
                    pattern: s.pattern,
                    at_ms: s.started_ms,
                    minutes: s.gap_ms / 60_000,
                });
            }
        }
    }
    for l in &leads {
        for w in l.warnings.iter().filter(|w| in_window(w.since_ms)) {
            match w.kind {
                "two_writers" => harness.two_writers += 1,
                "repeat" => harness.repeats += w.count.max(1) as u64,
                _ => {}
            }
        }
    }

    // --- names -------------------------------------------------------------------
    let name_votes: Vec<&crate::workshop::Vote> = input
        .name_votes
        .iter()
        .filter(|v| in_window(v.ts_ms))
        .collect();
    let names = NameMetrics {
        names_down: name_votes.iter().filter(|v| v.vote == "down").count() as u64,
        renamed: name_votes.iter().filter(|v| v.vote == "rename").count() as u64,
        generated: agents
            .iter()
            .filter(|a| looks_generated(a.name.as_deref()))
            .map(|a| name_of(a))
            .collect(),
        english_descriptions: agents
            .iter()
            .filter(|a| a.description.as_deref().is_some_and(reads_english))
            .map(|a| name_of(a))
            .collect(),
    };

    // --- daily series ----------------------------------------------------------------
    let unclear_days: BTreeMap<String, u64> = taps
        .iter()
        .filter(|r| r.target_kind == TargetKind::Message && r.value == "unclear")
        .fold(BTreeMap::new(), |mut m, r| {
            *m.entry(day(r.ms)).or_default() += 1;
            m
        });
    let days = (now.saturating_sub(since) / 86_400_000).clamp(1, 31);
    let daily = (0..days)
        .rev()
        .map(|i| {
            let d = day(now.saturating_sub(i * 86_400_000));
            DayPoint {
                findings: finding_days.get(&d).copied().unwrap_or(0),
                not_understood: unclear_days.get(&d).copied().unwrap_or(0),
                stalls: stall_days.get(&d).copied().unwrap_or(0),
                unanswered: unanswered_days.get(&d).copied().unwrap_or(0),
                day: d,
            }
        })
        .collect();

    let mut retro = Retro {
        v: 1,
        now_ms: now,
        since_ms: since,
        scope: match input.scope {
            Scope::Week => "week",
            Scope::Team(_) => "team",
        },
        lead: match &input.scope {
            Scope::Team(s) => Some(s.clone()),
            Scope::Week => None,
        },
        agents: agents.len(),
        delivery,
        messages,
        waiting,
        harness,
        names,
        feedback,
        daily,
        lessons: Vec::new(),
    };
    let vote_days: Vec<String> = dates_of(
        name_votes
            .iter()
            .filter(|v| v.vote != "up")
            .map(|v| day(v.ts_ms)),
    );
    retro.lessons = lessons(&retro, &taps, &vote_days);
    retro
}

const CLASS_WORDS: [(&str, &str, &str); 6] = [
    ("secret_leak", "secret leak", "secret leaks"),
    ("file_race", "file-read race", "file-read races"),
    ("host_check", "Host check failure", "Host check failures"),
    ("wrong_state", "wrong state", "wrong states"),
    ("ui", "interface failure", "interface failures"),
    ("other", "other finding", "other findings"),
];

fn class_words(class: &str, n: u64) -> String {
    let (_, one, many) = CLASS_WORDS
        .iter()
        .find(|(c, _, _)| *c == class)
        .copied()
        .unwrap_or(("other", "finding", "findings"));
    plural(n, one, many)
}

/// The findings of a repeated-class lesson as params; the console holds the
/// class words for each language.
fn repeat_params(findings: &[&FindingClass], named: bool) -> Params {
    let mut out = params([("items", findings.len().into())]);
    for (i, f) in findings.iter().enumerate() {
        out.insert(format!("class_{i}").into(), f.class.into());
        out.insert(format!("count_{i}").into(), f.count.into());
        out.insert(format!("branches_{i}").into(), f.branches.len().into());
        if named {
            out.insert(format!("names_{i}").into(), f.branches.join(", ").into());
        }
    }
    out
}

fn dates_of(days: impl IntoIterator<Item = String>) -> Vec<String> {
    days.into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn lessons(r: &Retro, taps: &[&Record], vote_days: &[String]) -> Vec<Lesson> {
    let mut out = Vec::new();
    let tap_dates = |kind: TargetKind, value: &str| {
        dates_of(
            taps.iter()
                .filter(|t| t.target_kind == kind && t.value == value)
                .map(|t| day(t.ms)),
        )
    };

    let repeated = |security: bool| -> Vec<&FindingClass> {
        r.delivery
            .findings
            .iter()
            .filter(|f| f.repeats && SECURITY.contains(&f.class) == security)
            .collect()
    };
    let sec = repeated(true);
    if !sec.is_empty() {
        let parts: Vec<String> = sec
            .iter()
            .map(|f| {
                format!(
                    "{} on {} ({})",
                    class_words(f.class, f.count),
                    plural(f.branches.len() as u64, "branch", "branches"),
                    f.branches.join(", ")
                )
            })
            .collect();
        out.push(Lesson {
            key: "shared_security_primitive",
            title: "One security piece, one owner",
            evidence: format!(
                "{}. The same piece was written on each branch.",
                parts.join("; ")
            ),
            evidence_code: "retro.evidence.shared_security",
            evidence_params: repeat_params(&sec, true),
            rule: "When two or more builders need the same security primitive, one owns it and the others reuse it".into(),
            count: sec.iter().map(|f| f.count).sum(),
            dates: dates_of(sec.iter().flat_map(|f| f.days.clone())),
        });
    }
    let other = repeated(false);
    if !other.is_empty() {
        let parts: Vec<String> = other
            .iter()
            .map(|f| {
                format!(
                    "{} on {}",
                    class_words(f.class, f.count),
                    plural(f.branches.len() as u64, "branch", "branches")
                )
            })
            .collect();
        out.push(Lesson {
            key: "review_repeat",
            title: "The same failure across several branches",
            evidence: format!("{}.", parts.join("; ")),
            evidence_code: "retro.evidence.review_repeat",
            evidence_params: repeat_params(&other, false),
            rule: "Before asking for review, check your branch for the findings other branches already got".into(),
            count: other.iter().map(|f| f.count).sum(),
            dates: dates_of(other.iter().flat_map(|f| f.days.clone())),
        });
    }

    let stalls_of = |pattern: &str| -> Vec<&StallSample> {
        r.harness
            .stalls
            .iter()
            .filter(|s| s.pattern == pattern)
            .collect()
    };
    let stall_evidence = |v: &[&StallSample]| {
        let longest = v.iter().map(|s| s.minutes).max().unwrap_or(0);
        let dates = dates_of(v.iter().map(|s| day(s.at_ms))).join(", ");
        (
            format!(
                "{} waiting for a permission, the longest {longest} min ({dates}).",
                plural(v.len() as u64, "stall", "stalls"),
            ),
            params([
                ("n_stall", v.len().into()),
                ("longest", longest.into()),
                ("dates", dates.into()),
            ]),
        )
    };
    for (pattern, key, title, rule) in [
        (
            "rm_variable",
            "rm_variable",
            "An rm on a variable stops the agent",
            "A rm on a variable stalls on a prompt; use \"${VAR:?}\"",
        ),
        (
            "shell_wrapper",
            "shell_wrapper",
            "bash -c asks for permission and nobody answers",
            "Write multi-step shell to a script file and run the file; never wrap commands in bash -c",
        ),
    ] {
        let v = stalls_of(pattern);
        if !v.is_empty() {
            let (evidence, evidence_params) = stall_evidence(&v);
            out.push(Lesson {
                key,
                title,
                evidence,
                evidence_code: "retro.evidence.stalls",
                evidence_params,
                rule: rule.into(),
                count: v.len() as u64,
                dates: dates_of(v.iter().map(|s| day(s.at_ms))),
            });
        }
    }
    let others = stalls_of("other");
    if others.len() >= 2 {
        let (evidence, evidence_params) = stall_evidence(&others);
        out.push(Lesson {
            key: "permission_stalls",
            title: "Permissions that stop agents",
            evidence,
            evidence_code: "retro.evidence.stalls",
            evidence_params,
            rule: "Before a long unattended run, use only commands that run without a prompt; a prompt nobody answers stops you".into(),
            count: others.len() as u64,
            dates: dates_of(others.iter().map(|s| day(s.at_ms))),
        });
    }

    if !r.waiting.unanswered.is_empty() {
        let longest = r
            .waiting
            .unanswered
            .iter()
            .map(|u| u.open_min)
            .max()
            .unwrap_or(0);
        out.push(Lesson {
            key: "unanswered_operator",
            title: "Your notes without a reply",
            evidence: format!(
                "{} unanswered, the oldest {longest} min ago.",
                plural(r.waiting.unanswered.len() as u64, "note", "notes")
            ),
            evidence_code: "retro.evidence.unanswered",
            evidence_params: params([
                ("n_note", r.waiting.unanswered.len().into()),
                ("longest", longest.into()),
            ]),
            rule: "Answer every note from the operator in your next turn, even if only to say when you will have the answer".into(),
            count: r.waiting.unanswered.len() as u64,
            dates: dates_of(r.waiting.unanswered.iter().map(|u| day(u.at_ms))),
        });
    }

    let unclear =
        r.messages.not_understood + r.messages.what_does_it_mean + r.messages.jargon_chips;
    if unclear >= 2 {
        out.push(Lesson {
            key: "plain_messages",
            title: "Messages you don't understand",
            evidence: format!(
                "{} marked \"I don't understand\", {} asking what something means, {} with jargon.",
                plural(r.messages.not_understood, "message", "messages"),
                plural(r.messages.what_does_it_mean, "note", "notes"),
                plural(r.messages.jargon_chips, "message", "messages")
            ),
            evidence_code: "retro.evidence.plain_messages",
            evidence_params: params([
                ("n_unclear", r.messages.not_understood.into()),
                ("n_asked", r.messages.what_does_it_mean.into()),
                ("n_jargon", r.messages.jargon_chips.into()),
            ]),
            rule: "Write to the operator in plain language: no SHAs, flags or internal names in the first line".into(),
            count: unclear,
            dates: tap_dates(TargetKind::Message, "unclear"),
        });
    }

    let generated = r.names.generated.len() as u64;
    if r.names.names_down + r.names.renamed >= 1 || generated >= 2 {
        out.push(Lesson {
            key: "readable_names",
            title: "Names that can be read",
            evidence: format!(
                "{} with 👎, {} and {} with numbers, suffixes or no name.",
                plural(r.names.names_down, "name", "names"),
                plural(r.names.renamed, "renamed", "renamed"),
                plural(generated, "name", "names")
            ),
            evidence_code: "retro.evidence.readable_names",
            evidence_params: params([
                ("n_down", r.names.names_down.into()),
                ("n_renamed", r.names.renamed.into()),
                ("n_generated", generated.into()),
            ]),
            rule:
                "Give each agent a short name for its job, with no numbers, hashes or -2 suffixes"
                    .into(),
            count: r.names.names_down + r.names.renamed + generated,
            dates: vote_days.to_vec(),
        });
    }

    if !r.waiting.stragglers.is_empty() {
        out.push(Lesson {
            key: "background_wait",
            title: "Agents waiting in the background",
            evidence: match r.waiting.stragglers.len() {
                1 => "1 agent ended its turn waiting for a background task.".to_string(),
                n => format!("{n} agents ended their turn waiting for a background task."),
            },
            evidence_code: if r.waiting.stragglers.len() == 1 {
                "retro.evidence.background_wait.one"
            } else {
                "retro.evidence.background_wait.many"
            },
            evidence_params: params([("count", r.waiting.stragglers.len().into())]),
            rule:
                "Work in the foreground; never end a turn waiting on a background job or a wake-up"
                    .into(),
            count: r.waiting.stragglers.len() as u64,
            dates: vec![day(r.now_ms)],
        });
    }

    let not_needed = r
        .feedback
        .get("needs_you")
        .and_then(|m| m.get("not_needed"))
        .copied()
        .unwrap_or(0);
    if not_needed >= 2 {
        out.push(Lesson {
            key: "needs_you_noise",
            title: "Alerts that didn't need you",
            evidence: format!(
                "{} marked \"This didn't need me\".",
                plural(not_needed, "alert", "alerts")
            ),
            evidence_code: "retro.evidence.needs_you_noise",
            evidence_params: params([("n_alert", not_needed.into())]),
            rule: "Mark a message BLOCKED only for a question only the operator can answer; decide the rest yourself".into(),
            count: not_needed,
            dates: tap_dates(TargetKind::NeedsYou, "not_needed"),
        });
    }

    let missing = r
        .feedback
        .get("delivery")
        .map(|m| m.get("no").copied().unwrap_or(0) + m.get("missing").copied().unwrap_or(0))
        .unwrap_or(0);
    if missing >= 2 {
        out.push(Lesson {
            key: "delivery_quality",
            title: "Deliveries with something missing",
            evidence: format!(
                "{} that didn't help or were missing something.",
                plural(missing, "delivery", "deliveries")
            ),
            evidence_code: "retro.evidence.delivery_quality",
            evidence_params: params([("n_delivery", missing.into())]),
            rule: "Before DONE, check each acceptance criterion of the brief and name the evidence for it".into(),
            count: missing,
            dates: dates_of(
                taps.iter()
                    .filter(|t| t.target_kind == TargetKind::Delivery && t.value != "yes")
                    .map(|t| day(t.ms)),
            ),
        });
    }

    out.sort_by(|a, b| b.count.cmp(&a.count).then(a.key.cmp(b.key)));
    out
}

/// The lessons, without those whose key the operator already decided.
pub fn candidates(retro: &Retro, decided: &BTreeSet<String>) -> Vec<Lesson> {
    retro
        .lessons
        .iter()
        .filter(|l| !decided.contains(l.key))
        .cloned()
        .collect()
}

#[cfg(test)]
mod tests;
