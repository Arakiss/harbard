//! Output rendering. Stdout is stable (human table or `--json`); callers write
//! human status to stderr. Rendering never touches the store.

use harbard_core::clock;
use harbard_core::{DoctorReport, InboxResult, SessionView};
use serde_json::json;

/// Render `ls`: one line per session with a liveness verdict, or JSON.
/// The fleet board as a person reads it: the counts first, then who and where.
pub fn fleet(view: &harbard_core::fleet::FleetView) -> String {
    let mut out = format!(
        "{} live · {} delegated · {} stale · {} pending · {} contested\n",
        view.counts.live,
        view.counts.delegated,
        view.counts.stale,
        view.counts.pending,
        view.counts.contested
    );
    for card in &view.sessions {
        let indent = "  ".repeat(card.depth.min(4) as usize);
        let intent = card
            .intent
            .as_deref()
            .map(|text| format!(" \"{text}\""))
            .unwrap_or_default();
        let ending = if card.ending { " (ending)" } else { "" };
        out.push_str(&format!(
            "{:<6} {indent}{} · {}{}{}\n",
            card.presence.as_str(),
            card.display_name,
            card.harness,
            ending,
            intent
        ));
    }
    for card in &view.checkouts {
        let contested = if card.contested {
            format!(" · CONTESTED by {}", card.rivals.len())
        } else {
            String::new()
        };
        out.push_str(&format!(
            "{} · {} live, {} delegated{}\n",
            card.id, card.live, card.delegated, contested
        ));
    }
    for item in &view.pending {
        out.push_str(&format!(
            "{} from {} · owed by {}: {}\n",
            item.kind,
            item.from_name,
            item.owed_by.len(),
            item.summary
        ));
    }
    out
}

pub fn ls(
    sessions: &[SessionView],
    aliases: &harbard_core::alias::Aliases,
    now_ms: u64,
    as_json: bool,
) -> String {
    if as_json {
        // `display_name` is added here, not on `SessionView`, whose key set is
        // the frozen `serve` contract. It comes from the board's resolver, so a
        // status line reading `ls --json` names a session exactly as the
        // console and the desktop app do instead of guessing from the id.
        let rows: Vec<serde_json::Value> = sessions
            .iter()
            .map(|s| {
                let mut row = serde_json::to_value(s).unwrap_or_default();
                if let Some(obj) = row.as_object_mut() {
                    let (name, _) = harbard_core::fleet::display_name(aliases, &s.session);
                    obj.insert("display_name".to_string(), json!(name));
                }
                row
            })
            .collect();
        return serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string());
    }
    if sessions.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for s in sessions {
        let where_ = s.repo.as_deref().unwrap_or(&s.cwd);
        let branch = s
            .branch
            .as_deref()
            .map(|b| format!(" ({b})"))
            .unwrap_or_default();
        let intent = s
            .intent
            .as_deref()
            .map(|i| format!(" \"{i}\""))
            .unwrap_or_default();
        let age = clock::relative(now_ms, s.last_signal_ms);
        out.push_str(&format!(
            "{:<6} {:<11} {} {}{}{}  · {}\n",
            s.activity_label(),
            s.harness,
            s.session,
            where_,
            branch,
            intent,
            age
        ));
    }
    out.trim_end().to_string()
}

/// Render `inbox`: the delivered notes plus a total/returned disclosure, or JSON.
pub fn inbox(result: &InboxResult, as_json: bool) -> String {
    if as_json {
        let v = json!({
            "returned": result.returned,
            "total": result.total,
            "events": result.events,
        });
        return serde_json::to_string_pretty(&v).unwrap_or_else(|_| "{}".to_string());
    }
    if result.events.is_empty() {
        return String::new();
    }
    let mut out = String::new();
    for e in &result.events {
        let target =
            e.to.as_ref()
                .map(|r| format!(" to {}:{}", recipient_kind(r), r.value))
                .unwrap_or_else(|| " (broadcast)".to_string());
        out.push_str(&format!(
            "[{} {}]{} {}\n",
            e.harness,
            short(&e.session),
            target,
            e.text
        ));
    }
    if result.total > result.returned {
        out.push_str(&format!(
            "… returned {} of {} (run inbox again for more)\n",
            result.returned, result.total
        ));
    }
    out.trim_end().to_string()
}

/// Render `doctor`: store health, or JSON.
pub fn doctor(report: &DoctorReport, as_json: bool) -> String {
    if as_json {
        return serde_json::to_string_pretty(report).unwrap_or_else(|_| "{}".to_string());
    }
    let mut out = format!(
        "store        {}\nexists       {}\nsegments     {}\nlines        {} ({} events, {} corrupt, {} unknown)\ncursors      {}\nleases       {} ({} active, {} stale, {} corrupt, {} inaccessible, {} temp) [{}]\nresolutions  {} open ({} expired this pass)\nretention    {} days (gc runs on hello)\nsegments     {} .. {}",
        report.store_root,
        report.store_exists,
        report.segments,
        report.total_lines,
        report.events,
        report.corrupt_lines,
        report.unknown_events,
        report.cursors,
        report.leases.total,
        report.leases.active,
        report.leases.stale,
        report.leases.corrupt,
        report.leases.inaccessible,
        report.leases.temp_files,
        if report.leases.healthy {
            "healthy"
        } else {
            "unhealthy"
        },
        report.open_resolutions,
        report.expired_resolutions,
        report.retention_days,
        report.oldest_segment.as_deref().unwrap_or("-"),
        report.newest_segment.as_deref().unwrap_or("-"),
    );
    let fts = &report.fts;
    out.push_str(&format!(
        "\nfts          {} notes indexed{}",
        fts.indexed_notes,
        if fts.present {
            String::new()
        } else {
            " (missing — run doctor/search to rebuild)".into()
        }
    ));
    let ch = &report.channels;
    if !ch.orphan.is_empty()
        || !ch.armed.is_empty()
        || !ch.dormant.is_empty()
        || !ch.legacy_unknown_age.is_empty()
        || !ch.repo_alias.is_empty()
        || !report.open_channel_unused.is_empty()
    {
        out.push_str("\n\nchannel hygiene (diagnose only — never auto-archive)");
        if !ch.orphan.is_empty() {
            out.push_str(&format!(
                "\n  orphan              {} — join <name> or archive when empty · {}",
                ch.orphan.len(),
                ch.orphan
                    .iter()
                    .map(|n| format!("#{n}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        if !ch.armed.is_empty() {
            out.push_str(&format!(
                "\n  armed               {} — members, zero notes · {}",
                ch.armed.len(),
                ch.armed
                    .iter()
                    .map(|n| format!("#{n}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        if !ch.dormant.is_empty() {
            out.push_str(&format!(
                "\n  dormant             {} — empty after activity; consider archive · {}",
                ch.dormant.len(),
                ch.dormant
                    .iter()
                    .map(|n| format!("#{n}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        if !ch.legacy_unknown_age.is_empty() {
            out.push_str(&format!(
                "\n  legacy_unknown_age  {} — no timestamps; excluded from age suggestions · {}",
                ch.legacy_unknown_age.len(),
                ch.legacy_unknown_age
                    .iter()
                    .map(|n| format!("#{n}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        if !ch.repo_alias.is_empty() {
            // Listed with the checkout, not just the name: the answer to
            // "what do I do with it" is the address on the right of the arrow.
            out.push_str(&format!(
                "\n  repo alias          {} — same audience as --to repo: · {}",
                ch.repo_alias.len(),
                ch.repo_alias
                    .iter()
                    .map(|(name, repo)| format!("#{name}→{repo}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        if !report.open_channel_unused.is_empty() {
            out.push_str(&format!(
                "\n  channel_unused open {} — note / leave / archive-when-empty closes · {}",
                report.open_channel_unused.len(),
                report
                    .open_channel_unused
                    .iter()
                    .map(|n| format!("#{n}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        if report.armed_channel_unused > 0 {
            out.push_str(&format!(
                "\n  armed this pass     {}",
                report.armed_channel_unused
            ));
        }
        if report.retired_channel_unused > 0 {
            out.push_str(&format!(
                "\n  retired this pass   {} — the log disagreed with the correction",
                report.retired_channel_unused
            ));
        }
    }
    out
}

fn recipient_kind(r: &harbard_core::Recipient) -> &'static str {
    match r.kind {
        harbard_core::RecipientKind::Harness => "harness",
        harbard_core::RecipientKind::Session => "session",
        harbard_core::RecipientKind::Repo => "repo",
        harbard_core::RecipientKind::Channel => "channel",
        harbard_core::RecipientKind::Unknown => "unknown",
    }
}

fn short(session: &str) -> String {
    let n = session.len().min(12);
    let mut end = 0;
    for (i, c) in session.char_indices() {
        if i + c.len_utf8() > n {
            break;
        }
        end = i + c.len_utf8();
    }
    session[..end].to_string()
}
