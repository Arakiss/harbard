//! `harbard lane`, `harbard heartbeat` and `harbard await lane`: the CLI over
//! [`harbard_core::lane`].
//!
//! Every verb that settles a lane turns the handovers it caused into notes —
//! to the old owner, to the new owner, and to the repository — so nobody has
//! to poll to learn that a turn passed. The caller of the verb is the author:
//! the session whose call noticed the handover, never a session it names.

use crate::cli::{Common, LaneCmd, LaneTakeArgs};
use crate::{Context, append_event, build_event_kind, fail, print_stdout, short};
use harbard_core::lane::{
    Handover, HandoverReason, HeartbeatOutcome, LaneRecord, LaneRequest, LaneStore, PrFacts,
    ReleaseOutcome, TakeOutcome,
};
use harbard_core::pr_watch::{GhSource, MIN_POLL_INTERVAL_SECS, PrState, PrWatch};
use harbard_core::{EventKind, NoteKind, Recipient, RecipientKind, Store, clock};
use serde_json::json;
use std::time::{Duration, Instant};

const UNSTABLE_SESSION: &str = "lanes need a stable session: run from Claude Code, Codex or \
Cursor, or pass --session <id>; a host-pid session cannot heartbeat what it took";

/// Waiting on a lane reads one small local file under a lock; once a second is
/// plenty for a turn measured in minutes and keeps the lock mostly free.
const MIN_LANE_POLL_MS: u64 = 1_000;
/// The book changes at most once per shared turn (minutes); reading it every
/// few seconds costs nothing and never reaches GitHub.
const MIN_PR_AWAIT_POLL_MS: u64 = 2_000;

/// `90s`, `60m`, `2h`. A bare number is refused: `--ttl 60` reads as minutes
/// to one agent and seconds to the next.
pub(crate) fn parse_ttl(raw: &str) -> Result<u64, String> {
    let raw = raw.trim();
    let unit_at = raw
        .find(|c: char| !c.is_ascii_digit())
        .filter(|index| *index > 0)
        .ok_or_else(|| format!("ttl \"{raw}\" needs a number and a unit: 90s, 60m or 2h"))?;
    let (digits, unit) = raw.split_at(unit_at);
    let value: u64 = digits
        .parse()
        .map_err(|_| format!("ttl \"{raw}\" is not a number of seconds, minutes or hours"))?;
    let scale = match unit {
        "s" => 1,
        "m" => 60,
        "h" => 60 * 60,
        _ => return Err(format!("ttl \"{raw}\" needs a unit: s, m or h")),
    };
    Ok(value.saturating_mul(scale))
}

/// `pr:2258,pr:2263` (clap already split on commas).
pub(crate) fn parse_release_on(items: &[String]) -> Result<Vec<u64>, String> {
    items
        .iter()
        .map(|item| {
            item.trim()
                .strip_prefix("pr:")
                .and_then(|number| number.parse::<u64>().ok())
                .filter(|number| *number > 0)
                .ok_or_else(|| format!("--release-on takes pr:<number>, got \"{item}\""))
        })
        .collect()
}

struct Caller {
    ctx: Context,
    repo: String,
}

fn caller(common: &Common) -> Result<Caller, String> {
    let ctx = Context::resolve(common);
    if !ctx.session_is_stable {
        return Err(UNSTABLE_SESSION.into());
    }
    let repo = repository(&ctx).ok_or_else(|| {
        "lanes belong to a repository: run inside a Git checkout or pass --repo".to_string()
    })?;
    Ok(Caller { ctx, repo })
}

/// The identity every worktree of a repository shares, so a lane taken in one
/// worktree is the same lane in all of them.
fn repository(ctx: &Context) -> Option<String> {
    ctx.repo_key.clone().or_else(|| ctx.repo.clone())
}

/// What the lane store consults to decide whether `release_on` fired: the
/// shared watcher's last answers, never a fresh call to GitHub.
pub(crate) fn pr_facts(store: &Store) -> impl PrFacts {
    PrWatch::at(store.root()).snapshot()
}

/// Seconds between shared PR turns. `HARBARD_PR_POLL_SECS` may lengthen it;
/// the watcher refuses anything under its floor.
fn pr_poll_secs() -> u64 {
    std::env::var("HARBARD_PR_POLL_SECS")
        .ok()
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(MIN_POLL_INTERVAL_SECS)
}

/// The author of notes nobody's own call caused: handovers the background
/// watcher found by itself (an idle ttl ran out, a PR merged overnight).
fn watcher_context(repo: &str) -> Context {
    Context {
        session: "harbard-watch".into(),
        session_is_stable: true,
        harness: "other".into(),
        host: harbard_core::hostname(),
        cwd: repo.into(),
        repo: Some(repo.into()),
        repo_key: None,
        branch: None,
        role: harbard_core::Role::Main,
        parent: None,
        nickname: None,
    }
}

/// One pass of the shared watcher: offer to take the PR turn (a no-op unless
/// it is due), then settle every lane against what it knows. `serve` runs this
/// on its tick; `await pr` runs it while it waits, so PRs are still answered
/// on a machine where `serve` is not running. The on-disk turn keeps it to one
/// poll per interval however many callers there are.
pub(crate) fn watch_turn(store: &Store, author: Option<&Context>, now: u64) {
    let lanes = LaneStore::at(store.root());
    let watch = PrWatch::at(store.root());
    let records = lanes.list(None);
    let tied: Vec<(String, u64)> = records
        .iter()
        .filter_map(|record| record.owner.as_ref().map(|owner| (record, owner)))
        .flat_map(|(record, owner)| {
            owner
                .release_on
                .iter()
                .map(|number| (record.repo.clone(), *number))
        })
        .collect();
    if !tied.is_empty() || !watch.is_unused() {
        let _ = watch.tick(&GhSource::from_env(), &tied, pr_poll_secs(), now);
    }
    if records.is_empty() {
        return;
    }
    for handover in lanes.sweep(&watch.snapshot(), now) {
        let fallback;
        let author = match author {
            Some(author) => author,
            None => {
                fallback = watcher_context(&handover.repo);
                &fallback
            }
        };
        announce(store, author, std::slice::from_ref(&handover), now);
    }
}

/// `session` just showed a sign of life (a prompt, an event it wrote, a lane
/// verb): start the ttl of any lane promoted to it while it was away. Cheap
/// when there is nothing to claim, which is almost always — it runs on every
/// hook turn and every append.
pub(crate) fn claim_turn(store: &Store, session: &str, now: u64) {
    let lanes = LaneStore::at(store.root());
    let pending = lanes.list(None).iter().any(|record| {
        record
            .owner
            .as_ref()
            .is_some_and(|owner| owner.session == session && owner.claim_by_ms.is_some())
    });
    if !pending {
        return;
    }
    for handover in lanes.claim(session, &pr_facts(store), now) {
        let author = watcher_context(&handover.repo);
        announce(store, &author, std::slice::from_ref(&handover), now);
    }
}

/// Block until PR `number` of this repository is merged (`merged`) or no
/// longer open (`closed`). Reads the shared book; never asks GitHub itself
/// except by taking the shared turn when it is due.
pub(crate) fn cmd_await_pr(
    store: &Store,
    common: &Common,
    number: &str,
    condition: &str,
    timeout_secs: u64,
    poll_ms: u64,
) -> i32 {
    let Some(number) = number.parse::<u64>().ok().filter(|number| *number > 0) else {
        return fail(format!("await pr takes a PR number, got \"{number}\""));
    };
    let want_merged = match condition {
        "merged" => true,
        "closed" => false,
        _ => return fail("await pr <number> takes `merged` or `closed`"),
    };
    let ctx = Context::resolve(common);
    let Some(repo) = repository(&ctx) else {
        return fail("await pr needs the repository: run inside a Git checkout or pass --repo");
    };
    let watch = PrWatch::at(store.root());
    if let Err(error) = watch.want(&repo, number, clock::now_ms()) {
        return fail(error);
    }
    let poll = Duration::from_millis(poll_ms.max(MIN_PR_AWAIT_POLL_MS));
    let timeout = Duration::from_secs(timeout_secs);
    let start = Instant::now();
    loop {
        let now = clock::now_ms();
        watch_turn(store, Some(&ctx), now);
        let state = watch.snapshot().state(&repo, number);
        let (outcome, code) = match state {
            Some(PrState::Merged) => ("satisfied", 0),
            Some(PrState::Closed) if !want_merged => ("satisfied", 0),
            Some(PrState::Closed) => ("closed_unmerged", 4),
            _ if start.elapsed() >= timeout => ("timeout", 3),
            _ => {
                std::thread::sleep(poll.min(timeout.saturating_sub(start.elapsed())));
                continue;
            }
        };
        let state = state.map(PrState::as_str);
        emit(
            common,
            json!({
                "pr": number,
                "repo": repo,
                "state": state,
                "outcome": outcome,
                "elapsed_ms": start.elapsed().as_millis(),
            }),
            match outcome {
                "satisfied" => format!("PR #{number} is {}", state.unwrap_or("settled")),
                "closed_unmerged" => format!("PR #{number} was closed without merging"),
                _ => format!(
                    "PR #{number} is still {}; the shared watcher keeps following it",
                    state.unwrap_or("unanswered")
                ),
            },
        );
        return code;
    }
}

pub(crate) fn cmd_lane(store: &Store, common: &Common, action: &LaneCmd) -> i32 {
    let ctx = Context::resolve(common);
    if ctx.session_is_stable {
        claim_turn(store, &ctx.session, clock::now_ms());
    }
    match action {
        LaneCmd::Take(args) | LaneCmd::Queue(args) => cmd_take(store, common, args),
        LaneCmd::Show { lane } => cmd_show(store, common, lane.as_deref()),
        LaneCmd::Release { lane } => cmd_release(store, common, lane),
    }
}

fn cmd_take(store: &Store, common: &Common, args: &LaneTakeArgs) -> i32 {
    let caller = match caller(common) {
        Ok(caller) => caller,
        Err(error) => return fail(error),
    };
    let ttl_secs = match parse_ttl(&args.ttl) {
        Ok(ttl) => ttl,
        Err(error) => return fail(error),
    };
    let release_on = match parse_release_on(&args.release_on) {
        Ok(numbers) => numbers,
        Err(error) => return fail(error),
    };
    let lanes = LaneStore::at(store.root());
    let now = clock::now_ms();
    let request = LaneRequest {
        session: &caller.ctx.session,
        purpose: &args.purpose,
        ttl_secs,
        release_on: &release_on,
    };
    let take = match lanes.take(&caller.repo, &args.lane, &request, &pr_facts(store), now) {
        Ok(take) => take,
        Err(error) => return fail(error),
    };
    announce(store, &caller.ctx, &take.handovers, now);
    let aliases = harbard_core::alias::read(store);
    let (human, code) = match take.outcome {
        TakeOutcome::Acquired => (
            format!(
                "acquired {} for {} — keep it with `harbard heartbeat {}`, let go with `harbard lane release {}`",
                args.lane,
                minutes(ttl_secs),
                args.lane,
                args.lane
            ),
            0,
        ),
        TakeOutcome::Held => (
            format!(
                "held {} — ttl restarted at {}",
                args.lane,
                minutes(ttl_secs)
            ),
            0,
        ),
        // Non-zero on purpose: `harbard lane take x && migrate` must not run
        // the migration while the caller is only queued.
        TakeOutcome::Queued { position } => (
            format!(
                "queued for {} at position {position} behind {} — `harbard await lane {}` blocks until it is yours",
                args.lane,
                owner_label(&aliases, &take.record),
                args.lane
            ),
            2,
        ),
    };
    emit(
        common,
        json!({
            "outcome": take.outcome,
            "lane": record_json(&aliases, &take.record, now),
        }),
        human,
    );
    code
}

fn cmd_show(store: &Store, common: &Common, lane: Option<&str>) -> i32 {
    let ctx = Context::resolve(common);
    let Some(repo) = repository(&ctx) else {
        return fail("lanes belong to a repository: run inside a Git checkout or pass --repo");
    };
    let lanes = LaneStore::at(store.root());
    let now = clock::now_ms();
    let aliases = harbard_core::alias::read(store);
    let records = match lane {
        Some(lane) => match lanes.show(&repo, lane, &pr_facts(store), now) {
            Ok((record, handovers)) => {
                announce(store, &ctx, &handovers, now);
                record.into_iter().collect()
            }
            Err(error) => return fail(error),
        },
        None => lanes.list(Some(&repo)),
    };
    let human = if records.is_empty() {
        match lane {
            Some(lane) => format!("{lane} is free"),
            None => "no lanes held in this repository".to_string(),
        }
    } else {
        records
            .iter()
            .map(|record| lane_line(&aliases, record, now))
            .collect::<Vec<_>>()
            .join("\n")
    };
    emit(
        common,
        json!({
            "lanes": records
                .iter()
                .map(|record| record_json(&aliases, record, now))
                .collect::<Vec<_>>(),
        }),
        human,
    );
    0
}

fn cmd_release(store: &Store, common: &Common, lane: &str) -> i32 {
    let caller = match caller(common) {
        Ok(caller) => caller,
        Err(error) => return fail(error),
    };
    let lanes = LaneStore::at(store.root());
    let now = clock::now_ms();
    let release = match lanes.release(
        &caller.repo,
        lane,
        &caller.ctx.session,
        &pr_facts(store),
        now,
    ) {
        Ok(release) => release,
        Err(error) => return fail(error),
    };
    announce(store, &caller.ctx, &release.handovers, now);
    let aliases = harbard_core::alias::read(store);
    let human = match release.outcome {
        ReleaseOutcome::Released => match release.record.as_ref().and_then(|r| r.owner.as_ref()) {
            Some(next) => format!(
                "released {lane} — now {}'s",
                callsign(&aliases, &next.session)
            ),
            None => format!("released {lane} — it is free"),
        },
        ReleaseOutcome::LeftQueue => format!("left the queue for {lane}"),
        ReleaseOutcome::NotHeld => format!("{lane} was not yours; nothing released"),
    };
    emit(
        common,
        json!({
            "outcome": release.outcome,
            "lane": release.record.as_ref().map(|record| record_json(&aliases, record, now)),
        }),
        human,
    );
    0
}

pub(crate) fn cmd_heartbeat(store: &Store, common: &Common, lane: Option<&str>) -> i32 {
    let caller = match caller(common) {
        Ok(caller) => caller,
        Err(error) => return fail(error),
    };
    let lanes = LaneStore::at(store.root());
    let now = clock::now_ms();
    let session = caller.ctx.session.as_str();
    let targets: Vec<String> = match lane {
        Some(lane) => vec![lane.to_string()],
        None => lanes
            .list(Some(&caller.repo))
            .into_iter()
            .filter(|record| record.owned_by(session))
            .map(|record| record.lane)
            .collect(),
    };
    let facts = pr_facts(store);
    let mut renewed = Vec::new();
    let mut lost = Vec::new();
    for target in &targets {
        match lanes.heartbeat(&caller.repo, target, session, &facts, now) {
            Ok(beat) => {
                announce(store, &caller.ctx, &beat.handovers, now);
                match beat.outcome {
                    HeartbeatOutcome::Renewed => renewed.push(target.clone()),
                    HeartbeatOutcome::NotOwner => lost.push(target.clone()),
                }
            }
            Err(error) => return fail(error),
        }
    }
    let human = match (renewed.is_empty(), lost.is_empty()) {
        (true, true) => "no lanes held in this repository".to_string(),
        (false, true) => format!("renewed {}", renewed.join(", ")),
        (true, false) => format!(
            "not yours: {} — take it again or `harbard await lane`",
            lost.join(", ")
        ),
        (false, false) => format!(
            "renewed {}; not yours: {}",
            renewed.join(", "),
            lost.join(", ")
        ),
    };
    emit(
        common,
        json!({ "renewed": renewed, "not_owner": lost }),
        human,
    );
    if lost.is_empty() { 0 } else { 2 }
}

/// Block until the caller owns `lane`. A caller not yet queued joins the queue
/// when it says what for; a timeout keeps its place.
pub(crate) fn cmd_await_lane(
    store: &Store,
    common: &Common,
    lane: &str,
    purpose: Option<&str>,
    ttl: &str,
    timeout_secs: u64,
    poll_ms: u64,
) -> i32 {
    let caller = match caller(common) {
        Ok(caller) => caller,
        Err(error) => return fail(error),
    };
    let lanes = LaneStore::at(store.root());
    let session = caller.ctx.session.as_str();
    let start = Instant::now();
    let now = clock::now_ms();
    claim_turn(store, session, now);
    let facts = pr_facts(store);
    let (current, handovers) = match lanes.show(&caller.repo, lane, &facts, now) {
        Ok(shown) => shown,
        Err(error) => return fail(error),
    };
    announce(store, &caller.ctx, &handovers, now);
    let involved = current
        .as_ref()
        .is_some_and(|record| record.owned_by(session) || record.position_of(session).is_some());
    if !involved {
        let Some(purpose) = purpose else {
            return fail(format!(
                "you are not queued for {lane}; pass --for \"…\" to join the queue while you wait"
            ));
        };
        let ttl_secs = match parse_ttl(ttl) {
            Ok(ttl) => ttl,
            Err(error) => return fail(error),
        };
        let request = LaneRequest {
            session,
            purpose,
            ttl_secs,
            release_on: &[],
        };
        match lanes.take(&caller.repo, lane, &request, &facts, now) {
            Ok(take) => announce(store, &caller.ctx, &take.handovers, now),
            Err(error) => return fail(error),
        }
    }
    let poll = Duration::from_millis(poll_ms.max(MIN_LANE_POLL_MS));
    let timeout = Duration::from_secs(timeout_secs);
    loop {
        let now = clock::now_ms();
        let (record, handovers) = match lanes.show(&caller.repo, lane, &facts, now) {
            Ok(shown) => shown,
            Err(error) => return fail(error),
        };
        announce(store, &caller.ctx, &handovers, now);
        let aliases = harbard_core::alias::read(store);
        let position = record.as_ref().and_then(|r| r.position_of(session));
        let (outcome, code, human) = match &record {
            Some(record) if record.owned_by(session) => ("acquired", 0, format!("{lane} is yours")),
            // Another call (`lane release`, `bye`) took the caller out of the
            // queue: waiting longer would wait for nothing.
            _ if position.is_none() => (
                "left",
                4,
                format!("no longer queued for {lane}; nothing to wait for"),
            ),
            Some(record) if start.elapsed() >= timeout => (
                "timeout",
                3,
                format!(
                    "still queued for {lane} at position {} behind {}; your place is kept",
                    position.unwrap_or_default(),
                    owner_label(&aliases, record)
                ),
            ),
            _ => {
                std::thread::sleep(poll.min(timeout.saturating_sub(start.elapsed())));
                continue;
            }
        };
        // Promoted while waiting here: the caller is awake by definition.
        if code == 0 {
            claim_turn(store, session, now);
        }
        emit(
            common,
            json!({
                "outcome": outcome,
                "elapsed_ms": start.elapsed().as_millis(),
                "lane": record.as_ref().map(|record| record_json(&aliases, record, now)),
            }),
            human,
        );
        return code;
    }
}

/// `bye`: release every lane the session holds and leave every queue, so the
/// next waiter does not sit behind a ttl nobody will renew. Best-effort: the
/// session is leaving and a failure here must not fail its departure.
pub(crate) fn release_session_lanes(store: &Store, common: &Common) {
    let ctx = Context::resolve(common);
    if !ctx.session_is_stable {
        return;
    }
    let now = clock::now_ms();
    let handovers =
        LaneStore::at(store.root()).release_session(&ctx.session, &pr_facts(store), now);
    announce(store, &ctx, &handovers, now);
}

/// One line per lane held in the caller's repository, for the SessionStart
/// brief. Read-only: the brief is on the hook path and never settles.
pub(crate) fn brief_lines(store: &Store, ctx: &Context, now: u64) -> Vec<String> {
    let Some(repo) = repository(ctx) else {
        return Vec::new();
    };
    let records = LaneStore::at(store.root()).list(Some(&repo));
    if records.is_empty() {
        return Vec::new();
    }
    let aliases = harbard_core::alias::read(store);
    records
        .iter()
        .map(|record| format!("harbard · {}", lane_line(&aliases, record, now)))
        .collect()
}

/// `lane · owner callsign · minutes held · queue length`.
fn lane_line(aliases: &harbard_core::alias::Aliases, record: &LaneRecord, now: u64) -> String {
    match &record.owner {
        Some(owner) => format!(
            "lane {} · {}{} · held {}m · {} waiting",
            record.lane,
            callsign(aliases, &owner.session),
            if owner.claim_by_ms.is_some() {
                " (promoted, not yet awake)"
            } else {
                ""
            },
            now.saturating_sub(owner.taken_at_ms) / 60_000,
            record.queue.len()
        ),
        None => format!(
            "lane {} · free · {} waiting",
            record.lane,
            record.queue.len()
        ),
    }
}

fn record_json(
    aliases: &harbard_core::alias::Aliases,
    record: &LaneRecord,
    now: u64,
) -> serde_json::Value {
    json!({
        "repo": record.repo,
        "lane": record.lane,
        "owner": record.owner.as_ref().map(|owner| json!({
            "session": owner.session,
            "callsign": callsign(aliases, &owner.session),
            "purpose": owner.purpose,
            "taken_at_ms": owner.taken_at_ms,
            "expires_at_ms": owner.expires_at_ms,
            "ttl_left_secs": owner.expires_at_ms.saturating_sub(now) / 1000,
            "claim_by_ms": owner.claim_by_ms,
            "release_on": owner.release_on,
        })),
        "queue": record.queue.iter().map(|waiter| json!({
            "session": waiter.session,
            "callsign": callsign(aliases, &waiter.session),
            "purpose": waiter.purpose,
            "joined_at_ms": waiter.joined_at_ms,
        })).collect::<Vec<_>>(),
    })
}

fn callsign(aliases: &harbard_core::alias::Aliases, session: &str) -> String {
    harbard_core::alias::get(aliases, session)
        .map(|alias| harbard_core::alias::display(&alias))
        .unwrap_or_else(|| short(session))
}

fn owner_label(aliases: &harbard_core::alias::Aliases, record: &LaneRecord) -> String {
    record
        .owner
        .as_ref()
        .map(|owner| format!("{} ({})", callsign(aliases, &owner.session), owner.purpose))
        .unwrap_or_else(|| "nobody".to_string())
}

fn minutes(secs: u64) -> String {
    if secs.is_multiple_of(60) {
        format!("{}m", secs / 60)
    } else {
        format!("{secs}s")
    }
}

fn emit(common: &Common, value: serde_json::Value, human: String) {
    if common.json {
        print_stdout(&serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".into()));
    } else {
        print_stdout(&human);
    }
}

/// Turn handovers into notes: a warning to the new owner (its turn started
/// and its ttl is running), a warning to an old owner who did not let go
/// themselves, and an fyi to the repository. The author never notes itself.
pub(crate) fn announce(store: &Store, author: &Context, handovers: &[Handover], now: u64) {
    if handovers.is_empty() {
        return;
    }
    let aliases = harbard_core::alias::read(store);
    for handover in handovers {
        for (recipient, kind, text) in handover_notes(&aliases, handover, &author.session) {
            let event =
                build_event_kind(author, EventKind::Note, &text, Some(recipient), now, kind);
            let _ = append_event(store, &event);
        }
    }
}

pub(crate) fn handover_notes(
    aliases: &harbard_core::alias::Aliases,
    handover: &Handover,
    author: &str,
) -> Vec<(Recipient, NoteKind, String)> {
    let lane = &handover.lane;
    let why = match handover.reason {
        HandoverReason::Released => "released".to_string(),
        HandoverReason::Expired => "its ttl expired without a heartbeat".to_string(),
        HandoverReason::SessionEnded => "its owner's session ended".to_string(),
        HandoverReason::Unclaimed => format!(
            "it was promoted to a session that showed no sign of life within {}",
            minutes(harbard_core::lane::PROMOTION_GRACE_SECS)
        ),
        HandoverReason::PrsSettled => format!(
            "PR {} settled",
            handover
                .prs
                .iter()
                .map(|number| format!("#{number}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    };
    let from = handover
        .from
        .as_deref()
        .map_or_else(|| "nobody".to_string(), |s| callsign(aliases, s));
    let to = handover
        .to
        .as_deref()
        .map_or_else(|| "free".to_string(), |s| callsign(aliases, s));
    let session = |value: &str| Recipient {
        kind: RecipientKind::Session,
        value: value.to_string(),
    };
    let mut notes = Vec::new();
    // Addressed to the waiter's session, never to its repo: that is what the
    // wake watcher listens for, and a waiter asleep since it queued is exactly
    // who must be woken. Its ttl only starts when it next shows a sign of life.
    if let Some(next) = handover.to.as_deref().filter(|next| *next != author) {
        notes.push((
            session(next),
            NoteKind::Warning,
            format!(
                "lane {lane} is yours now ({why}). Your ttl starts at your next prompt or harbard command; if neither comes within {}, the lane passes on. Keep it with `harbard heartbeat {lane}`; let go with `harbard lane release {lane}`.",
                minutes(harbard_core::lane::PROMOTION_GRACE_SECS)
            ),
        ));
    }
    let involuntary = !matches!(
        handover.reason,
        HandoverReason::Released | HandoverReason::SessionEnded
    );
    if let Some(previous) = handover
        .from
        .as_deref()
        .filter(|previous| involuntary && *previous != author)
    {
        notes.push((
            session(previous),
            NoteKind::Warning,
            format!("lane {lane} is no longer yours ({why}); now {to}. Stop work that needs it."),
        ));
    }
    // The next holder has been told above. A lane handed on in the normal way
    // asks nothing of the rest of the repository; one taken from its holder may.
    if involuntary {
        notes.push((
            Recipient {
                kind: RecipientKind::Repo,
                value: handover.repo.clone(),
            },
            NoteKind::Fyi,
            format!("lane {lane}: {from} → {to} ({why})"),
        ));
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ttl_requires_a_unit() {
        assert_eq!(parse_ttl("90s"), Ok(90));
        assert_eq!(parse_ttl("60m"), Ok(3600));
        assert_eq!(parse_ttl("2h"), Ok(7200));
        for bad in ["60", "m", "", "10d", "1.5h", "-5m"] {
            assert!(parse_ttl(bad).is_err(), "{bad} must be refused");
        }
    }

    #[test]
    fn release_on_takes_pr_prefixed_numbers_only() {
        let items = ["pr:2258".to_string(), " pr:2263".to_string()];
        assert_eq!(parse_release_on(&items), Ok(vec![2258, 2263]));
        for bad in ["2258", "pr:", "pr:0", "issue:4"] {
            assert!(parse_release_on(&[bad.to_string()]).is_err(), "{bad}");
        }
    }

    fn handover(reason: HandoverReason, from: &str, to: Option<&str>) -> Handover {
        Handover {
            repo: "/work/northwind".into(),
            lane: "migrations".into(),
            reason,
            from: Some(from.into()),
            to: to.map(str::to_string),
            prs: vec![],
        }
    }

    fn recipients(notes: &[(Recipient, NoteKind, String)]) -> Vec<(RecipientKind, String)> {
        notes
            .iter()
            .map(|(recipient, _, _)| (recipient.kind, recipient.value.clone()))
            .collect()
    }

    #[test]
    fn an_expiry_noticed_by_a_third_session_notes_both_owners_and_the_repo() {
        let aliases = harbard_core::alias::Aliases::default();
        let notes = handover_notes(
            &aliases,
            &handover(HandoverReason::Expired, "old", Some("new")),
            "observer",
        );
        assert_eq!(
            recipients(&notes),
            vec![
                (RecipientKind::Session, "new".to_string()),
                (RecipientKind::Session, "old".to_string()),
                (RecipientKind::Repo, "/work/northwind".to_string()),
            ]
        );
        assert!(
            notes[..2]
                .iter()
                .all(|(_, kind, _)| *kind == NoteKind::Warning)
        );
    }

    #[test]
    fn a_voluntary_release_tells_only_the_next_holder() {
        let aliases = harbard_core::alias::Aliases::default();
        let notes = handover_notes(
            &aliases,
            &handover(HandoverReason::Released, "old", Some("new")),
            "old",
        );
        assert_eq!(
            recipients(&notes),
            vec![(RecipientKind::Session, "new".to_string())]
        );
        let freed = handover_notes(
            &aliases,
            &handover(HandoverReason::Released, "old", None),
            "old",
        );
        assert!(freed.is_empty(), "a lane left free asks nothing of anyone");
    }

    #[test]
    fn a_waiter_whose_own_call_promoted_it_is_not_noted() {
        let aliases = harbard_core::alias::Aliases::default();
        let notes = handover_notes(
            &aliases,
            &handover(HandoverReason::Expired, "old", Some("new")),
            "new",
        );
        assert_eq!(
            recipients(&notes),
            vec![
                (RecipientKind::Session, "old".to_string()),
                (RecipientKind::Repo, "/work/northwind".to_string()),
            ]
        );
    }
}
