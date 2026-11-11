//! Retro (`lib/retro.ts`, `lib/checks.ts`): lesson titles and evidence, the memory sink's answer, "Was it you?" lines.

use crate::derive::history::long_day;
use crate::i18n::{serve_text, t, tn, tv};
use crate::model::Params;
use crate::model::retro::{DeskCheck, Lesson, MemorySinkOutcome};
use gpui::{App, SharedString};
use serde_json::Value;

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
    fn short_text_collapses_and_cuts() {
        assert_eq!(short_text("  a \n b  "), "a b");
        let long = "x".repeat(81);
        let cut = short_text(&long);
        assert_eq!(cut.chars().count(), 80);
        assert!(cut.ends_with('…'));
        assert_eq!(short_text(&"y".repeat(80)).chars().count(), 80);
    }
}
