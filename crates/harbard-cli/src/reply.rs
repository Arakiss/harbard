//! `harbard reply <note-id>|last "…"`: answer a note in one short command.
//!
//! Before it, every answer was `harbard note --session <id> --harness <h>
//! --ref <id> --to harness:human "…"`, and the address was the part people got
//! wrong. The note already says everything the answer needs: its id is the
//! thread, its author is the recipient, and the session it was addressed to
//! is the one answering when the caller's shell does not say who it is.

use crate::cli::Common;
use crate::context::Context;
use harbard_core::{Event, EventKind, RecipientKind, Store, Window, query};

#[allow(clippy::too_many_arguments)]
pub fn cmd_reply(
    store: &Store,
    common: &Common,
    target: &str,
    text: &str,
    kind: &str,
    fields: &[String],
    agent: Option<&str>,
    now: u64,
) -> i32 {
    let ctx = Context::resolve(common);
    let read = store.read(Window::All);
    let note = match resolve_target(store, &read.events, &ctx, target, now) {
        Ok(note) => note,
        Err(why) => return crate::fail(why),
    };
    let answering = match answering_as(store, common, &ctx, &read.events, note, now) {
        Ok(c) => c,
        Err(why) => return crate::fail(why),
    };
    let me = Context::resolve(&answering);
    if note.session == me.session {
        return crate::fail(format!(
            "note {} is your own note — a reply goes to its author; to add to your own thread, harbard note --ref {} \"…\"",
            note.id, note.id
        ));
    }
    let to = harbard_core::wake::reply_target(note);
    // A note relayed to an in-process agent, answered with `--agent`: the
    // answer is that agent's, and goes back under its name, so the console
    // threads it in its conversation and the operator knows who answered.
    // Without `--agent` the answer is the caller's own, unsigned: a name is
    // never put on words by guessing whose they are.
    let keys = crate::agents::to_agents(&note.text);
    let relayed;
    let text = if agent.is_none() {
        text
    } else if keys.is_empty() {
        return crate::fail(format!(
            "note {} was not relayed to an agent — --agent is only for answering a relayed note; drop --agent to answer as yourself",
            note.id
        ));
    } else {
        let (view, _) = crate::agents::workspace(store, common, now);
        let names: Vec<(String, String, String)> = keys
            .iter()
            .map(|k| {
                let (name, address) = crate::agents::agent_names(&view, &me.session, k);
                (k.clone(), name, address)
            })
            .collect();
        let want = agent.unwrap_or_default();
        let chosen = names
            .iter()
            .find(|(k, n, a)| [k, n, a].iter().any(|v| v.eq_ignore_ascii_case(want)));
        let Some((_, name, _)) = chosen else {
            let listed: Vec<&str> = names.iter().map(|(_, _, a)| a.as_str()).collect();
            return crate::fail(format!(
                "note {} was relayed to {} — --agent {want} is none of them; pass one of those names",
                note.id,
                listed.join(", ")
            ));
        };
        relayed = if text.starts_with(&format!("{name}:")) {
            text.to_string()
        } else {
            format!("{name}: {text}")
        };
        relayed.as_str()
    };
    if !common.json {
        eprintln!(
            "harbard: replying to {} {} from {}",
            note.kind.as_str(),
            crate::short(&note.id),
            author(store, note, now)
        );
    }
    crate::cmd_note(
        store,
        &answering,
        kind,
        text,
        Some(&to),
        Some(&note.id),
        fields,
        now,
    )
}

/// The note being answered: an exact id or prefix, or `last` — the first
/// answer `harbard owed` lists ([`crate::agents::answer_order`]: the
/// operator's first, then oldest first), else the newest note another
/// session addressed to it. A note relayed to an agent (`to_agent:`) is
/// never `last`: the agent's answer goes by its id, with `--agent`.
fn resolve_target<'e>(
    store: &Store,
    events: &'e [Event],
    ctx: &Context,
    target: &str,
    now: u64,
) -> Result<&'e Event, String> {
    if target != "last" {
        return match harbard_core::id::event(events, target, EventKind::Note) {
            Ok(event) => Ok(event),
            Err(harbard_core::id::ResolveIdError::Unknown) => Err(format!(
                "no note id starts with \"{target}\" — copy the id from the message, or use `harbard reply last`"
            )),
            Err(harbard_core::id::ResolveIdError::Ambiguous(count)) => Err(format!(
                "note id \"{target}\" matches {count} notes — use the full 26-character id"
            )),
        };
    }
    if !ctx.session_is_stable {
        return Err(
            "reply last needs to know who you are — pass --session <id>, or reply to the note by its id"
                .to_string(),
        );
    }
    let channels = crate::my_channels(store, ctx);
    let placement = harbard_core::workspaces::Placement::load(store);
    let gate = placement.gate(ctx.repo_key.as_deref().or(ctx.repo.as_deref()));
    let reader = harbard_core::Reader {
        session: &ctx.session,
        harness: &ctx.harness,
        repo: ctx.repo.as_deref(),
        repo_key: ctx.repo_key.as_deref().or(ctx.repo.as_deref()),
        channels: &channels,
        workspace: &gate,
    };
    let relayed = |e: &Event| !crate::agents::to_agents(&e.text).is_empty();
    let mut owed: Vec<&Event> = harbard_core::status::owed_notes(events, &reader, now)
        .into_iter()
        .filter(|e| !relayed(e))
        .collect();
    owed.sort_by_key(|e| crate::agents::answer_order(e));
    if let Some(owed) = owed.first() {
        return Ok(owed);
    }
    events
        .iter()
        .rev()
        .find(|e| {
            e.event == EventKind::Note
                && e.session != ctx.session
                && e.is_addressed_to_reader(&reader)
                && !relayed(e)
        })
        .ok_or_else(|| {
            "nothing to reply to: no note addressed to you is in the store — reply to a note by its id, or start one with `harbard note \"…\"`"
                .to_string()
        })
}

/// The shared options the answer is written with. A shell that carries a
/// session marker answers as that session. One that does not answers as the
/// session the note was addressed to, with that session's harness: it is the
/// only session the note asked.
fn answering_as(
    store: &Store,
    common: &Common,
    ctx: &Context,
    events: &[Event],
    note: &Event,
    now: u64,
) -> Result<Common, String> {
    if ctx.session_is_stable {
        return Ok(common.clone());
    }
    let Some(to) = note
        .to
        .as_ref()
        .filter(|t| t.kind == RecipientKind::Session)
    else {
        return Err(format!(
            "cannot tell which session is answering note {} — it was not addressed to one session; pass --session <id>",
            note.id
        ));
    };
    let mut answering = common.clone();
    answering.session = Some(to.value.clone());
    if answering.harness.is_none() {
        answering.harness = query::sessions(store, events, now, crate::stale_secs(common))
            .into_iter()
            .find(|v| v.session == to.value)
            .map(|v| v.harness);
    }
    Ok(answering)
}

fn author(store: &Store, note: &Event, now: u64) -> String {
    if note.is_operator() || note.harness == harbard_core::HARNESS_HUMAN {
        return "the operator".to_string();
    }
    let mut aliases = harbard_core::alias::read_for_naming(store);
    let (name, _) = harbard_core::alias::resolve(&mut aliases, &note.session, now);
    format!(
        "{} ({})",
        harbard_core::alias::display(&name),
        crate::short(&note.session)
    )
}
