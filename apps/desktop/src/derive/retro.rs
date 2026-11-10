//! Retro (`lib/retro.ts`, `lib/checks.ts`): the ten tiles, the sparkline,
//! lesson titles and evidence, the memory sink's answer, "Was it you?" lines.

use crate::derive::history::long_day;
use crate::i18n::{I18n, Locale, serve_text, t, tn, tv};
use crate::model::Params;
use crate::model::retro::{DayPoint, DeskCheck, Lesson, MemorySinkOutcome, Retro};
use gpui::{App, SharedString};
use serde_json::Value;

pub struct Tile {
    pub id: &'static str,
    pub label: SharedString,
    pub value: String,
    pub detail: Option<SharedString>,
    /// One value per day of the window, oldest first.
    pub series: Option<Vec<u64>>,
}

fn class_count(class: &str, count: u64, cx: &App) -> SharedString {
    let base = match class {
        "secret_leak" => "retro.classSecretLeak",
        "file_race" => "retro.classFileRace",
        "host_check" => "retro.classHostCheck",
        "wrong_state" => "retro.classWrongState",
        "ui" => "retro.classUi",
        "other" => "retro.classOther",
        _ => "retro.classFinding",
    };
    tn(cx, base, count, &[])
}

/// "12 min" under two hours, else "2.5 h" (one decimal, the locale's mark).
pub fn minutes(m: Option<u64>, cx: &App) -> String {
    let Some(m) = m else {
        return t(cx, "retro.noData").into();
    };
    if m < 120 {
        return format!("{m} min");
    }
    let tenths = (m as f64 / 6.0).round() as u64;
    let (whole, frac) = (tenths / 10, tenths % 10);
    let mut out = whole.to_string();
    if frac != 0 {
        let mark = if cx.global::<I18n>().locale == Locale::Es {
            ','
        } else {
            '.'
        };
        out.push(mark);
        out.push_str(&frac.to_string());
    }
    format!("{out} h")
}

pub fn tiles(r: &Retro, cx: &App) -> Vec<Tile> {
    let findings: u64 = r.delivery.findings.iter().map(|f| f.count).sum();
    let rounds: u64 = r.delivery.review_rounds.values().sum();
    let branches = r.delivery.review_rounds.len() as u64;
    let asked = r.messages.what_does_it_mean;
    let described = r.names.english_descriptions.len() as u64;
    let day = |f: fn(&DayPoint) -> u64| Some(r.daily.iter().map(f).collect::<Vec<_>>());
    let detail_findings: Vec<String> = r
        .delivery
        .findings
        .iter()
        .filter(|f| f.count > 0)
        .map(|f| class_count(&f.class, f.count, cx).to_string())
        .collect();
    vec![
        Tile {
            id: "findings",
            label: t(cx, "retro.tileFindings"),
            value: findings.to_string(),
            detail: (!detail_findings.is_empty()).then(|| detail_findings.join(", ").into()),
            series: day(|d| d.findings),
        },
        Tile {
            id: "repeated",
            label: t(cx, "retro.tileRepeated"),
            value: r
                .delivery
                .findings
                .iter()
                .filter(|f| f.repeats)
                .count()
                .to_string(),
            detail: None,
            series: None,
        },
        Tile {
            id: "rounds",
            label: t(cx, "retro.tileRounds"),
            value: rounds.to_string(),
            detail: (branches > 0).then(|| tn(cx, "retro.branches", branches, &[])),
            series: None,
        },
        Tile {
            id: "install",
            label: t(cx, "retro.tileInstall"),
            value: minutes(r.delivery.brief_to_install_p50_min, cx),
            detail: Some(t(cx, "retro.tileMedian")),
            series: None,
        },
        Tile {
            id: "unclear",
            label: t(cx, "retro.tileUnclear"),
            value: r.messages.not_understood.to_string(),
            detail: (asked > 0).then(|| tn(cx, "retro.whatDoesItMean", asked, &[])),
            series: day(|d| d.not_understood),
        },
        Tile {
            id: "unanswered",
            label: t(cx, "retro.tileUnanswered"),
            value: r.waiting.unanswered.len().to_string(),
            detail: r.waiting.answer_p50_min.map(|m| {
                tv(
                    cx,
                    "retro.tileUnansweredDetail",
                    &[("time", &minutes(Some(m), cx))],
                )
            }),
            series: day(|d| d.unanswered),
        },
        Tile {
            id: "idle",
            label: t(cx, "retro.tileIdle"),
            value: r.waiting.idle_min.to_string(),
            detail: (!r.waiting.stragglers.is_empty()).then(|| {
                tv(
                    cx,
                    "retro.tileIdleDetail",
                    &[("count", &r.waiting.stragglers.len().to_string())],
                )
            }),
            series: None,
        },
        Tile {
            id: "stalls",
            label: t(cx, "retro.tileStalls"),
            value: r.harness.stalls.len().to_string(),
            detail: (r.harness.stall_min > 0).then(|| {
                tv(
                    cx,
                    "retro.tileStallsDetail",
                    &[("minutes", &r.harness.stall_min.to_string())],
                )
            }),
            series: day(|d| d.stalls),
        },
        Tile {
            id: "two_writers",
            label: t(cx, "retro.tileTwoWriters"),
            value: r.harness.two_writers.to_string(),
            detail: None,
            series: None,
        },
        Tile {
            id: "names",
            label: t(cx, "retro.tileNames"),
            value: r.names.names_down.to_string(),
            detail: (described > 0).then(|| tn(cx, "retro.englishDescriptions", described, &[])),
            series: None,
        },
    ]
}

/// The sparkline's points in a `w`×`h` box with a 1px inset; zero sits on
/// the bottom, the largest value on the top (`sparkPath`, rounded).
pub fn spark_points(values: &[u64], w: f32, h: f32) -> Vec<(f32, f32)> {
    if values.is_empty() {
        return Vec::new();
    }
    let max = values.iter().copied().max().unwrap_or(0) as f32;
    let n = values.len();
    values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = if n == 1 {
                w / 2.
            } else {
                1. + (i as f32 * (w - 2.)) / (n - 1) as f32
            };
            let y = if max == 0. {
                h - 1.
            } else {
                h - 1. - (v as f32 / max) * (h - 2.)
            };
            (x.round(), y.round())
        })
        .collect()
}

pub fn feedback_label(kind: &str, cx: &App) -> SharedString {
    match kind {
        "message" => t(cx, "retro.feedbackMessage"),
        "delivery" => t(cx, "retro.feedbackDelivery"),
        "needs_you" => t(cx, "retro.feedbackNeedsYou"),
        "robot_state" => t(cx, "retro.feedbackRobotState"),
        "dock" => t(cx, "retro.feedbackDock"),
        other => other.to_string().into(),
    }
}

pub fn feedback_value_label(value: &str, cx: &App) -> SharedString {
    let key = match value {
        "up" => "retro.valueUp",
        "down" => "retro.valueDown",
        "unclear" => "retro.valueUnclear",
        "yes" => "retro.valueYes",
        "no" => "retro.valueNo",
        "missing" => "retro.valueMissing",
        "not_needed" => "retro.valueNotNeeded",
        "wrong" => "retro.valueWrong",
        "annoying" => "retro.valueAnnoying",
        other => return other.to_string().into(),
    };
    t(cx, key)
}

/// serve titles each lesson in English; the title is looked up by key.
pub fn lesson_title(key: &str, fallback: &str, cx: &App) -> SharedString {
    let k = match key {
        "shared_security_primitive" => "retro.lessonSharedSecurity",
        "review_repeat" => "retro.lessonReviewRepeat",
        "permission_stalls" => "retro.lessonPermissionStalls",
        "rm_variable" => "retro.lessonRmVariable",
        "shell_wrapper" => "retro.lessonShellWrapper",
        "unanswered_operator" => "retro.lessonUnanswered",
        "plain_messages" => "retro.lessonPlainMessages",
        "descriptions_in_spanish" => "retro.lessonDescriptions",
        "readable_names" => "retro.lessonReadableNames",
        "background_wait" => "retro.lessonBackgroundWait",
        "needs_you_noise" => "retro.lessonNeedsYouNoise",
        "delivery_quality" => "retro.lessonDeliveryQuality",
        _ => return fallback.to_string().into(),
    };
    t(cx, k)
}

fn evidence_noun(name: &str) -> Option<&'static str> {
    Some(match name {
        "n_stall" => "retro.nounStall",
        "n_note" | "n_asked" => "retro.nounNote",
        "n_unclear" | "n_jargon" => "retro.nounMessage",
        "n_down" | "n_generated" => "retro.nounName",
        "n_renamed" => "retro.nounRenamed",
        "n_alert" => "retro.nounAlert",
        "n_delivery" => "retro.nounDelivery",
        _ => return None,
    })
}

fn num(v: Option<&Value>) -> u64 {
    match v {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(s)) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

/// The findings of a repeated-class lesson, one clause per class.
fn evidence_items(params: &Params, count: u64, cx: &App) -> String {
    (0..count)
        .map(|i| {
            let classes = class_count(
                params
                    .get(&format!("class_{i}"))
                    .and_then(Value::as_str)
                    .unwrap_or(""),
                num(params.get(&format!("count_{i}"))),
                cx,
            );
            let branches = tn(
                cx,
                "retro.branches",
                num(params.get(&format!("branches_{i}"))),
                &[],
            );
            match params.get(&format!("names_{i}")) {
                None => tv(
                    cx,
                    "retro.evidenceItem",
                    &[("classes", &classes), ("branches", &branches)],
                ),
                Some(names) => {
                    let names = names
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| names.to_string());
                    tv(
                        cx,
                        "retro.evidenceItemNamed",
                        &[
                            ("classes", &classes),
                            ("branches", &branches),
                            ("names", &names),
                        ],
                    )
                }
            }
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// A lesson's evidence: serve's counts, in this language's words.
pub fn lesson_evidence(lesson: &Lesson, cx: &App) -> SharedString {
    let vars = lesson.evidence_params.as_ref().map(|params| {
        let mut vars = params.clone();
        for (name, value) in params {
            if let (Some(noun), Value::Number(n)) = (evidence_noun(name), value) {
                let phrase = tn(cx, noun, n.as_u64().unwrap_or(0), &[]);
                vars.insert(name[2..].to_string(), Value::String(phrase.to_string()));
            }
        }
        if let Some(Value::Number(n)) = params.get("items") {
            vars.insert(
                "items".into(),
                Value::String(evidence_items(params, n.as_u64().unwrap_or(0), cx)),
            );
        }
        vars
    });
    serve_text(
        cx,
        lesson.evidence_code.as_deref(),
        vars.as_ref(),
        &lesson.evidence,
    )
}

/// The memory sink's answer in the operator's language; an unknown code
/// keeps serve's English note.
pub fn memory_sink_note(o: &MemorySinkOutcome, cx: &App) -> Option<SharedString> {
    let param = |k: &str| {
        o.params
            .as_ref()
            .and_then(|p| p.get(k))
            .cloned()
            .unwrap_or_default()
    };
    Some(match o.code.as_deref() {
        Some("memory_sink_disabled") => t(cx, "retro.memorySinkNoteDisabled"),
        Some("memory_sink_not_configured") => t(cx, "retro.memorySinkNoteNotConfigured"),
        Some("memory_sink_locked") => t(cx, "retro.memorySinkNoteLocked"),
        Some("memory_sink_timed_out") => t(cx, "retro.memorySinkNoteTimedOut"),
        Some("memory_sink_refused") => tv(
            cx,
            "retro.memorySinkNoteRefused",
            &[("reason", &param("reason"))],
        ),
        Some("memory_sink_failed") => tv(
            cx,
            "retro.memorySinkNoteFailed",
            &[("detail", &param("detail"))],
        ),
        Some("memory_sink_bad_answer") => t(cx, "retro.memorySinkNoteBadAnswer"),
        _ => return o.note.clone().map(Into::into),
    })
}

/// serve's refusal of characters nobody sees, in the operator's language.
pub fn invisible_refusal(message: &str, cx: &App) -> Option<SharedString> {
    if !message.contains("characters you cannot see") {
        return None;
    }
    let found: Vec<&str> = message
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '+'))
        .filter(|w| {
            w.strip_prefix("U+").is_some_and(|h| {
                (4..=6).contains(&h.len()) && h.chars().all(|c| c.is_ascii_hexdigit())
            })
        })
        .collect();
    let list = if found.is_empty() {
        String::new()
    } else {
        format!(" ({})", found.join(", "))
    };
    Some(tv(cx, "messages.invisibleRefusal", &[("list", &list)]))
}

/// Whitespace collapsed, cut to 79 characters and "…" past 80.
pub fn short_text(text: &str) -> String {
    let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.chars().count() > 80 {
        format!("{}…", t.chars().take(79).collect::<String>())
    } else {
        t
    }
}

/// "Rule approved: “…” · 5 November · kept on 6 November".
pub fn check_line(c: &DeskCheck, cx: &App) -> String {
    let text = c.text.as_deref().unwrap_or("");
    let what = match c.what.as_str() {
        "rule" => tv(
            cx,
            "workspace.checkLineRule",
            &[("text", &short_text(text))],
        ),
        "context" if !text.trim().is_empty() => tv(
            cx,
            "workspace.checkLineContext",
            &[("text", &short_text(text))],
        ),
        "context" => t(cx, "workspace.checkLineContextCleared"),
        "memory_sink" => t(cx, "workspace.checkLineMemorySink"),
        _ => t(cx, "workspace.checkLineOther"),
    };
    let answered = c.answered_ms.unwrap_or(c.since_ms);
    let answer = match c.outcome.as_deref() {
        Some("kept") => tv(
            cx,
            "workspace.checkLineKept",
            &[("day", &long_day(answered, cx))],
        ),
        Some("revoked") => tv(
            cx,
            "workspace.checkLineRevoked",
            &[("day", &long_day(answered, cx))],
        ),
        _ => t(cx, "workspace.checkLineUnanswered"),
    };
    format!("{what} · {} · {answer}", long_day(c.since_ms, cx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spark_points_scale_to_the_max_with_zero_at_the_bottom() {
        assert!(spark_points(&[], 112., 28.).is_empty());
        assert_eq!(spark_points(&[3], 112., 28.), vec![(56., 1.)]);
        let p = spark_points(&[0, 2, 1], 112., 28.);
        assert_eq!(p, vec![(1., 27.), (56., 1.), (111., 14.)]);
        assert_eq!(
            spark_points(&[0, 0], 112., 28.),
            vec![(1., 27.), (111., 27.)]
        );
    }

    #[test]
    fn short_text_collapses_and_cuts() {
        assert_eq!(short_text("  a \n b  "), "a b");
        let long = "x".repeat(81);
        let cut = short_text(&long);
        assert_eq!(cut.chars().count(), 80);
        assert!(cut.ends_with('…'));
        assert_eq!(short_text(&"y".repeat(80)).chars().count(), 80);
    }
}
