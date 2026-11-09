//! `harbard trace` — the delivery pipeline, reconstructed for a human.
//!
//! The receipts the bus already keeps say how far each session's cursor got;
//! the ops log says WHY. This verb joins the two against the event store and
//! answers the only three questions that matter when a message goes
//! unanswered: did it land, did it go out, and if not — which stage said no.

use crate::cli::Common;
use harbard_core::id::ResolveIdError;
use harbard_core::note_trace::{self, NoteReport};
use harbard_core::oplog::{self, OpEvent};
use harbard_core::query::{self, DEFAULT_STALE_SECS};
use harbard_core::{Store, Window};

/// "14m ago"-style relative age, compact.
fn age(now: u64, then: u64) -> String {
    let s = now.saturating_sub(then) / 1000;
    if s < 60 {
        format!("{s}s ago")
    } else if s < 3600 {
        format!("{}m ago", s / 60)
    } else if s < 86_400 {
        format!("{}h ago", s / 3600)
    } else {
        format!("{}d ago", s / 86_400)
    }
}

fn render_note_report(report: &NoteReport, now: u64, alias_of: &dyn Fn(&str) -> String) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "note {} — {}{} · by {} · {} · {}\n",
        crate::short(&report.note_id),
        report.kind,
        report
            .to
            .as_ref()
            .map(|t| format!(" · to {t}"))
            .unwrap_or_else(|| " · broadcast".to_string()),
        report.author,
        report
            .via
            .as_deref()
            .map(|v| format!("via {v}"))
            .unwrap_or_else(|| "via unknown".to_string()),
        age(now, report.appended_ms),
    ));
    for line in report.text.lines() {
        out.push_str(&format!("  {line}\n"));
    }
    if report.deliveries.is_empty() {
        // "Nobody yet" is a promise, and for most undelivered notes it is a
        // false one. When no session in the store can still match the
        // recipient, the note is not waiting — it is finished, and the author
        // should be told to write it again to somebody reachable.
        if report.pending.is_empty() {
            out.push_str("\ndelivered: nobody, and nobody can — no session in the store matches this recipient\n");
        } else {
            out.push_str("\ndelivered: nobody yet\n");
        }
    } else {
        out.push_str("\ndelivered:\n");
        for d in &report.deliveries {
            let cont = d
                .continued_after_ms
                .map(|ms| format!(" · continued +{}s", ms.saturating_sub(d.ms) / 1000))
                .unwrap_or_else(|| " · no signal since".to_string());
            out.push_str(&format!(
                "  {} — injected {} · {} envelope · {}{}\n",
                alias_of(&d.session),
                d.verb,
                d.envelope,
                age(now, d.ms),
                cont,
            ));
        }
    }
    if !report.pending.is_empty() {
        out.push_str("pending:\n");
        for p in &report.pending {
            out.push_str(&format!("  {} — {}\n", alias_of(&p.session), p.verdict));
        }
    }
    out
}

/// One session's recent ops, as a timeline plus tallies.
fn render_session_report(
    session: &str,
    ops: &[OpEvent],
    now: u64,
    alias_of: &dyn Fn(&str) -> String,
) -> String {
    let mine: Vec<&OpEvent> = ops
        .iter()
        .filter(|o| o.session.as_deref() == Some(session))
        .collect();
    let mut out = String::new();
    out.push_str(&format!("session {}\n", alias_of(session)));
    if mine.is_empty() {
        out.push_str("no ops recorded — the ops log only exists since it shipped\n");
        return out;
    }
    let count = |op: &str, verb: Option<&str>| {
        mine.iter()
            .filter(|o| o.op == op && verb.is_none_or(|v| o.verb.as_deref() == Some(v)))
            .count()
    };
    let last_hook = mine.iter().rfind(|o| o.op == oplog::OP_HOOK);
    out.push_str(&format!(
        "hooks: inline {} · turn {} · hello {} · bye {}   throttled: {}   injected: {}   errors: {}\n",
        count(oplog::OP_HOOK, Some("inline")),
        count(oplog::OP_HOOK, Some("turn")),
        count(oplog::OP_HOOK, Some("hello")),
        count(oplog::OP_HOOK, Some("bye")),
        count(oplog::OP_THROTTLED, None),
        count(oplog::OP_INJECTED, None),
        count(oplog::OP_ERROR, None),
    ));
    if let Some(h) = last_hook {
        out.push_str(&format!(
            "last hook: {} {}\n",
            h.verb.as_deref().unwrap_or("?"),
            age(now, h.ms)
        ));
    }
    out.push('\n');
    for op in mine.iter().rev().take(15).rev() {
        out.push_str(&format!("  {}\n", render_op_line(op, now)));
    }
    out
}

fn render_op_line(op: &OpEvent, now: u64) -> String {
    let at = age(now, op.ms);
    match op.op.as_str() {
        oplog::OP_HOOK => format!("{at:>12}  hook {}", op.verb.as_deref().unwrap_or("?")),
        oplog::OP_THROTTLED => {
            format!(
                "{at:>12}  throttled (last check {}s before)",
                op.ago_ms.unwrap_or(0) / 1000
            )
        }
        oplog::OP_INJECTED => format!(
            "{at:>12}  injected {} — {} note(s) · {} envelope",
            op.verb.as_deref().unwrap_or("?"),
            op.delivered.as_ref().map_or(0, Vec::len),
            op.envelope.as_deref().unwrap_or("?"),
        ),
        oplog::OP_HELD_QUIET => format!(
            "{at:>12}  held quiet — {} fyi note(s) wait for the turn",
            op.quiet.unwrap_or(0),
        ),
        oplog::OP_NOTE_APPENDED => format!(
            "{at:>12}  note appended — {} {}{}",
            op.kind.as_deref().unwrap_or("?"),
            op.to
                .as_deref()
                .map(|t| format!("to {t} "))
                .unwrap_or_default(),
            op.note.as_deref().map(crate::short).unwrap_or_default(),
        ),
        oplog::OP_ERROR => format!(
            "{at:>12}  ERROR in {} — {}",
            op.verb.as_deref().unwrap_or("?"),
            op.msg.as_deref().unwrap_or("?"),
        ),
        other => format!("{at:>12}  {other}"),
    }
}

fn render_today_report(ops: &[OpEvent], now: u64, alias_of: &dyn Fn(&str) -> String) -> String {
    let day_ago = now.saturating_sub(86_400_000);
    let recent: Vec<&OpEvent> = ops.iter().filter(|o| o.ms >= day_ago).collect();
    let errors = recent.iter().filter(|o| o.op == oplog::OP_ERROR).count();
    let mut out = format!("last 24h: {} ops · {} errors\n", recent.len(), errors);
    for op_name in [
        oplog::OP_HOOK,
        oplog::OP_THROTTLED,
        oplog::OP_INJECTED,
        oplog::OP_HELD_QUIET,
        oplog::OP_NOTE_APPENDED,
    ] {
        let n = recent.iter().filter(|o| o.op == op_name).count();
        if n > 0 {
            out.push_str(&format!("  {op_name} ×{n}\n"));
        }
    }
    // Last hook per session — the "who is actually awake" board.
    let mut last: std::collections::BTreeMap<&str, u64> = std::collections::BTreeMap::new();
    for op in recent.iter().filter(|o| o.op == oplog::OP_HOOK) {
        if let Some(s) = op.session.as_deref() {
            last.insert(s, op.ms);
        }
    }
    if !last.is_empty() {
        out.push_str("last hook by session:\n");
        for (s, ms) in last {
            out.push_str(&format!("  {} — {}\n", alias_of(s), age(now, ms)));
        }
    }
    out
}

pub fn cmd_trace(
    store: &Store,
    common: &Common,
    note: Option<&str>,
    session: Option<&str>,
    now: u64,
) -> i32 {
    let stale = common.stale_secs.unwrap_or(DEFAULT_STALE_SECS);
    let aliases = harbard_core::alias::read(store);
    let alias_of = |session: &str| -> String {
        harbard_core::alias::get(&aliases, session)
            .map(|a| harbard_core::alias::display(&a))
            .unwrap_or_else(|| crate::short(session))
    };

    let ops = oplog::read(store, Window::All);

    if let Some(note_id) = note {
        let report = match note_trace::report_for_id(store, note_id, now, stale) {
            Ok(report) => report,
            Err(ResolveIdError::Unknown) => {
                eprintln!("harbard: no note \"{note_id}\" in the store");
                return 1;
            }
            Err(ResolveIdError::Ambiguous(count)) => {
                eprintln!(
                    "harbard: note prefix \"{note_id}\" matches {count} notes; use the full 26-character id"
                );
                return 1;
            }
        };
        if common.json {
            println!(
                "{}",
                serde_json::to_string_pretty(&report).unwrap_or_default()
            );
        } else {
            print!("{}", render_note_report(&report, now, &alias_of));
        }
        return 0;
    }

    if let Some(session_id) = session {
        // Accept an alias or a unique prefix, but never inherit BTreeMap order
        // as a delivery verdict when two sessions share what was typed.
        let read = store.read(Window::All);
        let views = query::sessions(store, &read.events, now, stale);
        let mut matches: Vec<&str> = views
            .iter()
            .filter(|view| {
                view.session.starts_with(session_id)
                    || aliases.by_session.get(&view.session).is_some_and(|alias| {
                        alias.eq_ignore_ascii_case(session_id)
                            || harbard_core::alias::display(alias).eq_ignore_ascii_case(session_id)
                    })
            })
            .map(|view| view.session.as_str())
            .collect();
        matches.sort_unstable();
        matches.dedup();
        let resolved = match matches.as_slice() {
            [only] => (*only).to_string(),
            [] => {
                eprintln!("harbard: no session or alias \"{session_id}\" in the store");
                return 1;
            }
            many => {
                eprintln!(
                    "harbard: session \"{session_id}\" matches {} sessions; use the full id",
                    many.len()
                );
                return 1;
            }
        };
        if common.json {
            let mine: Vec<&OpEvent> = ops
                .ops
                .iter()
                .filter(|o| o.session.as_deref() == Some(resolved.as_str()))
                .collect();
            println!(
                "{}",
                serde_json::to_string_pretty(
                    &serde_json::json!({ "session": resolved, "ops": mine })
                )
                .unwrap_or_default()
            );
        } else {
            print!(
                "{}",
                render_session_report(&resolved, &ops.ops, now, &alias_of)
            );
        }
        return 0;
    }

    if common.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({ "ops_total": ops.ops.len(), "corrupt_lines": ops.corrupt_lines }))
                .unwrap_or_default()
        );
    } else {
        print!("{}", render_today_report(&ops.ops, now, &alias_of));
    }
    0
}
