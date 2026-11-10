//! `watch` and `stop-check`: the two Stop-hook verbs.
//!
//! `stop-check` runs synchronously when a turn ends and holds the stop only
//! while the session owes an answer ([`harbard_core::status::owed_notes`]).
//! `watch` runs in the background from the same Stop and waits, without a
//! turn, for something addressed to the session; when it lands, the watcher
//! wakes the session that is already running and delivers into it, never into
//! a second process.
//!
//! - Claude Code runs `watch` as an `asyncRewake` hook: exit 2 wakes the idle
//!   session and shows stderr as a system reminder; exit 0 says nothing.
//! - Codex has no rewake, so its Stop hook runs `watch --detach`, which arms a
//!   detached watcher and returns at once; that watcher delivers through
//!   `codex queue --thread <session_id>`. Codex fills the Stop hook's
//!   `session_id` with the root thread's id (codex-rs `core/src/hook_runtime.rs`
//!   builds Stop only for root turns, children get SubagentStop), which is the
//!   id `codex queue --thread` accepts.
//! - A one-shot `claude -p` turn ([`context::claude_print_turn`]) is never
//!   watched: nobody can be woken, and the CLI would wait for the watcher.
//!
//! Both verbs are silent when nothing is actionable: no stdout, exit 0. The
//! hooks they replace were retired for noise, and a hook that talks when it
//! has nothing to say would be retired for the same reason.

use crate::cli::Common;
use crate::context::{self, Context};
use harbard_core::cursor;
use harbard_core::oplog::OpEvent;
use harbard_core::wake::{self, Ledger, Wake};
use harbard_core::{EventKind, PidChecker, Reader, Store, SystemPidChecker, Window, clock};
use std::collections::HashSet;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Longest a watcher stays armed after a turn ends.
///
/// One day, including overnight peer/CI waits. This remains finite because
/// Claude Code enforces asyncRewake timeouts; exit/supersession/new prompts
/// stop the watcher sooner. After expiry, mail remains unread for the next prompt.
pub(crate) const LIFETIME_SECS: u64 = 24 * 60 * 60;

/// The Claude Code hook timeout for `watch`. Claude Code enforces `timeout`
/// on `asyncRewake` hooks, so it must exceed the watcher's own lifetime or the
/// harness kills the watcher first and the lifetime is fiction.
pub(crate) const HOOK_TIMEOUT_SECS: u64 = LIFETIME_SECS + 60;

/// How often the watcher looks at the tail of the log. A tail read parses only
/// the bytes appended since the last one, so this costs a `stat` when idle.
pub(crate) const POLL_MS: u64 = 500;

/// How long the watcher keeps collecting after the first actionable event, so
/// a burst — a lane handover writes two notes, an operator fires three asks —
/// becomes one wake instead of three turns. Poll plus window stays well inside
/// the five seconds the wake is promised in.
pub(crate) const COALESCE_MS: u64 = 1_500;

/// How often the watcher checks that the harness process above it is alive.
const SUPERVISOR_CHECK: Duration = Duration::from_secs(5);

pub(crate) struct WatchOptions {
    pub lifetime: Duration,
    pub poll: Duration,
    pub coalesce: Duration,
    pub supervisor: Option<u32>,
}

/// What a finished wait hands its caller: the events to deliver, and the mark
/// to commit once they are delivered. Committing only after delivery means a
/// failed `codex queue` leaves the notes unread for the next watcher.
struct Settled {
    wakes: Vec<Wake>,
    mark: Option<String>,
}

pub(crate) fn cmd_watch(
    store: &Store,
    common: &Common,
    opts: WatchOptions,
    detach: bool,
    deliver: Option<&str>,
) -> i32 {
    if detach {
        return spawn_detached(common, &opts);
    }
    // `claude -p` waits for its asyncRewake hooks before it exits, and a
    // one-shot turn has nobody to wake: watching would only hold the process
    // open for the watcher's whole lifetime. Leave at once, without a trace.
    if deliver.is_none() && context::claude_print_turn() {
        return 0;
    }
    let ctx = Context::resolve(common);
    if !ctx.session_is_stable {
        return 0;
    }
    crate::oplog(
        store,
        &OpEvent::hook(clock::now_ms(), "watch", &ctx.session, &ctx.harness),
    );
    let supervisor = opts
        .supervisor
        .or_else(|| context::supervised_harness_pid(&ctx.harness));
    let Some(settled) = wait(store, &ctx, &opts, supervisor) else {
        return 0;
    };
    let now = clock::now_ms();
    let text = wake::render(&settled.wakes, &harbard_core::alias::read(store), now);
    let ids: Vec<String> = settled.wakes.iter().map(|w| w.event.id.clone()).collect();
    let phases = phases(store, &ctx.session, &settled.wakes);
    if deliver == Some("codex") {
        if deliver_codex(store, &ctx, &text, now) {
            commit(store, &ctx.session, settled.mark, &ids);
            record(store, &ctx, &phases, "codex-queue", now);
        }
        return 0;
    }
    commit(store, &ctx.session, settled.mark, &ids);
    record(store, &ctx, &phases, "rewake", now);
    eprint!("{text}");
    2
}

/// Releases the session's watcher slot on every way out of [`wait`], unless a
/// newer watcher has taken it meanwhile.
struct OwnerGuard<'a> {
    store: &'a Store,
    session: &'a str,
    token: String,
}

impl Drop for OwnerGuard<'_> {
    fn drop(&mut self) {
        cursor::clear_watch_owner(self.store, self.session, &self.token);
    }
}

fn wait(
    store: &Store,
    ctx: &Context,
    opts: &WatchOptions,
    supervisor: Option<u32>,
) -> Option<Settled> {
    let session = ctx.session.as_str();
    let armed_ms = clock::now_ms();
    let guard = OwnerGuard {
        store,
        session,
        token: format!("{}:{}", std::process::id(), clock::now_ms()),
    };
    let _ = cursor::write_watch_owner(store, session, &guard.token);
    let started = Instant::now();

    // Position before snapshot, as `subscribe` does: an event racing in
    // between is read twice and deduplicated, never skipped.
    let (_, mut position) = store.read_since(None);
    // Recover original join, older questions and answers once at Stop.
    // Polling below still reads only appended tail bytes.
    let snapshot = store.read(Window::All);
    if wake::has_ended(&snapshot.events, session) {
        return None;
    }
    // Subscriptions as `brief` resolves them, read once per watcher: a room
    // joined mid-wait reaches the session from its next Stop on.
    let channels = crate::my_channels(store, ctx);
    let placement = harbard_core::workspaces::Placement::load(store);
    let gate = placement.gate(ctx.repo_key.as_deref().or(ctx.repo.as_deref()));
    let reader = reader_for(ctx, &channels, &gate);
    let brief_state = cursor::read_brief_state(store, session);
    let ended = cursor::read_turn_end(store, session);
    // A prompt after the previous Stop invalidates its idle interval. With no
    // proven Stop, only arrivals after this invocation may wake the reader.
    let mut idle_since = if ended > 0 && ended >= brief_state.hb {
        ended
    } else {
        armed_ms
    };
    // What landed while the turn ran reached nobody: the prompt's brief was
    // already printed, and nothing is injected mid-turn. Waiting for the next
    // human prompt left those notes unread for minutes or hours, so the floor
    // for what this Stop may deliver is the prompt that opened the turn. The
    // read marks still exclude everything a brief, an inbox or an earlier
    // wake already showed; `prompt` keeps detecting the next real prompt.
    let prompt = brief_state.hb;
    let floor = if prompt > 0 {
        prompt.min(idle_since)
    } else {
        idle_since
    };
    let brief = brief_state.seen;
    let inbox = cursor::read_inbox_cursor(store, session);
    let woke = cursor::read_wake_state(store, session).seen;
    let start = wake::start_index(
        &snapshot.events,
        &[brief.as_deref(), inbox.as_deref(), woke.as_deref()],
        session,
    );

    let mut ledger = Ledger::new(session);
    for event in &snapshot.events {
        ledger.observe(event);
    }
    let mut found: Vec<Wake> = snapshot.events[start.min(snapshot.events.len())..]
        .iter()
        .filter_map(|event| {
            if harbard_core::query::event_ms(event) <= floor
                || !ledger.actionable(event, clock::now_ms())
            {
                return None;
            }
            ledger.cause(event, &reader).map(|cause| Wake {
                event: event.clone(),
                cause,
            })
        })
        .collect();
    let mut last_scanned = snapshot.events.last().map(|e| e.id.clone()).or(woke);
    let mut scanned: HashSet<String> = snapshot.events.iter().map(|e| e.id.clone()).collect();
    let mut first_found = (!found.is_empty()).then(Instant::now);
    let mut supervisor_checked = Instant::now();

    loop {
        idle_since = idle_since.max(cursor::read_turn_end(store, session));
        if cursor::read_brief_state(store, session).hb > idle_since {
            return None;
        }
        if first_found.is_some_and(|at| at.elapsed() >= opts.coalesce) {
            match settle(
                store,
                session,
                &ledger,
                &mut found,
                last_scanned.clone(),
                floor,
            ) {
                Some(settled) => return Some(settled),
                None => first_found = None,
            }
        }
        if started.elapsed() >= opts.lifetime {
            return None;
        }
        if cursor::read_watch_owner(store, session).as_deref() != Some(guard.token.as_str()) {
            return None;
        }
        if supervisor_checked.elapsed() >= SUPERVISOR_CHECK {
            supervisor_checked = Instant::now();
            if supervisor.is_some_and(|pid| !SystemPidChecker.is_alive(pid)) {
                return None;
            }
        }
        std::thread::sleep(opts.poll);
        let (read, next) = store.read_since(position.as_ref());
        position = next;
        // A day boundary makes `read_since` hand back the whole window again;
        // the id set keeps that from replaying as new.
        for event in read.events {
            if !scanned.insert(event.id.clone()) {
                continue;
            }
            ledger.observe(&event);
            if event.session == session && event.event == EventKind::Bye {
                return None;
            }
            if harbard_core::query::event_ms(&event) > floor
                && ledger.actionable(&event, clock::now_ms())
                && let Some(cause) = ledger.cause(&event, &reader)
            {
                first_found.get_or_insert_with(Instant::now);
                last_scanned = Some(event.id.clone());
                found.push(Wake { event, cause });
            } else {
                last_scanned = Some(event.id.clone());
            }
        }
    }
}

/// Drop what the session already has, then take what is left.
///
/// Between finding an event and delivering it the session may have answered
/// it, `stop-check` may have shown it as an owed note on the same Stop, or a
/// prompt may have carried it in the brief. Each of those is a second copy of
/// a message the reader already has, and a wake that repeats itself is the
/// noise this design exists to avoid.
fn settle(
    store: &Store,
    session: &str,
    ledger: &Ledger,
    found: &mut Vec<Wake>,
    last_scanned: Option<String>,
    floor: u64,
) -> Option<Settled> {
    let brief_seen = cursor::read_brief_state(store, session).seen;
    let inbox_seen = cursor::read_inbox_cursor(store, session);
    let wake_state = cursor::read_wake_state(store, session);
    let now = clock::now_ms();
    let held = cursor::read_owed_state(store, session).blocks;
    found.retain(|w| {
        let answered = w.event.event == EventKind::Note && ledger.answered(&w.event.id);
        let shown = held.contains_key(&w.event.id)
            || wake_state.delivered.contains(&w.event.id)
            || ledger.at_or_before(&w.event.id, brief_seen.as_deref())
            || ledger.at_or_before(&w.event.id, inbox_seen.as_deref())
            || ledger.at_or_before(&w.event.id, wake_state.seen.as_deref());
        !answered
            && !shown
            && harbard_core::query::event_ms(&w.event) > floor
            && ledger.actionable(&w.event, now)
    });
    if found.is_empty() {
        commit(store, session, last_scanned, &[]);
        return None;
    }
    let take = found.len().min(wake::MAX_WAKE_EVENTS);
    let wakes: Vec<Wake> = found.drain(..take).collect();
    // Anything left over is found again by the next watcher: the mark stops
    // at the last event this wake carried.
    let mark = if found.is_empty() {
        last_scanned
    } else {
        wakes.last().map(|w| w.event.id.clone())
    };
    Some(Settled { wakes, mark })
}

fn commit(store: &Store, session: &str, mark: Option<String>, ids: &[String]) {
    if let Some(mark) = mark {
        let mut state = cursor::read_wake_state(store, session);
        let inbox = cursor::read_inbox_cursor(store, session);
        let brief = cursor::read_brief_state(store, session).seen;
        state.delivered.retain(|id| {
            !wake::at_or_before(id, inbox.as_deref()) && !wake::at_or_before(id, brief.as_deref())
        });
        state.delivered.extend(ids.iter().cloned());
        state.seen = Some(mark);
        let _ = cursor::write_wake_state(store, session, &state);
    }
}

/// The delivered ids split by when they landed: while the turn that just
/// ended was running, or after it. Read when delivering, so a `stop-check`
/// that recorded the turn end after this watcher armed still counts.
struct Phases {
    mid_turn: Vec<String>,
    idle: Vec<String>,
}

fn phases(store: &Store, session: &str, wakes: &[Wake]) -> Phases {
    let prompt = cursor::read_brief_state(store, session).hb;
    let ended = cursor::read_turn_end(store, session);
    let (mid, idle): (Vec<&Wake>, Vec<&Wake>) = wakes.iter().partition(|w| {
        let at = harbard_core::query::event_ms(&w.event);
        at > prompt && at <= ended
    });
    let ids = |ws: Vec<&Wake>| ws.into_iter().map(|w| w.event.id.clone()).collect();
    Phases {
        mid_turn: ids(mid),
        idle: ids(idle),
    }
}

fn record(store: &Store, ctx: &Context, phases: &Phases, envelope: &str, now: u64) {
    let ids: Vec<String> = phases
        .mid_turn
        .iter()
        .chain(&phases.idle)
        .cloned()
        .collect();
    crate::oplog(
        store,
        &OpEvent::injected(now, "wake", ids, envelope, &ctx.session, &ctx.harness),
    );
    for (phase, ids) in [("mid_turn", &phases.mid_turn), ("idle", &phases.idle)] {
        harbard_core::learning::record_injections_in_phase(
            store,
            now,
            ids,
            envelope,
            "wake",
            &ctx.session,
            &ctx.harness,
            Some(phase),
        );
    }
}

/// Codex Stop hook: arm a watcher that outlives the hook and return at once.
///
/// Codex waits for a hook's stdout and stderr to close and kills its process
/// group on timeout (codex-rs `hooks/src/engine/command_runner.rs`), so the
/// watcher gets null stdio and a process group of its own. The harness pid is
/// read here, while Codex is still an ancestor, because a detached process has
/// no ancestry left to read it from.
fn spawn_detached(common: &Common, opts: &WatchOptions) -> i32 {
    let ctx = Context::resolve(common);
    if !ctx.session_is_stable {
        return 0;
    }
    let Ok(exe) = std::env::current_exe() else {
        return 0;
    };
    let supervisor = opts
        .supervisor
        .or_else(|| context::supervised_harness_pid(&ctx.harness));
    let mut cmd = Command::new(exe);
    cmd.args([
        "watch",
        "--session",
        &ctx.session,
        "--harness",
        &ctx.harness,
        "--cwd",
        &ctx.cwd,
        "--deliver",
        "codex",
        "--lifetime-secs",
        &opts.lifetime.as_secs().to_string(),
        "--poll-ms",
        &opts.poll.as_millis().to_string(),
        "--coalesce-ms",
        &opts.coalesce.as_millis().to_string(),
    ]);
    if let Some(home) = common.home.as_deref() {
        cmd.args(["--home", home]);
    }
    if let Some(pid) = supervisor {
        cmd.args(["--supervisor-pid", &pid.to_string()]);
    }
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    // Not waited on: the hook returns now and the watcher is reparented.
    let _ = cmd.spawn();
    0
}

/// Hand the message to the running Codex thread. `codex queue` starts a turn
/// on an idle thread and holds the message while one is active. Without the
/// app-server daemon it writes the queue through an embedded server and the
/// open session picks it up on its own poll; with no `codex` on PATH it fails
/// and the notes stay unread for the next watcher.
fn deliver_codex(store: &Store, ctx: &Context, text: &str, now: u64) -> bool {
    let output = Command::new("codex")
        .args(["queue", "--thread", &ctx.session, "--message", text])
        .stdin(Stdio::null())
        .output();
    let error = match output {
        Ok(out) if out.status.success() => return true,
        Ok(out) => String::from_utf8_lossy(&out.stderr).trim().to_string(),
        Err(e) => e.to_string(),
    };
    crate::oplog(
        store,
        &OpEvent::error(
            now,
            "watch",
            &format!("codex queue: {error}"),
            &ctx.session,
            &ctx.harness,
        ),
    );
    false
}

fn reader_for<'a>(
    ctx: &'a Context,
    channels: &'a [String],
    workspace: &'a harbard_core::workspaces::Gate<'a>,
) -> Reader<'a> {
    Reader {
        session: &ctx.session,
        harness: &ctx.harness,
        repo: ctx.repo.as_deref(),
        repo_key: ctx.repo_key.as_deref().or(ctx.repo.as_deref()),
        channels,
        workspace,
    }
}

/// Stop hook: hold the stop while the session owes an answer.
///
/// Owed answers come from [`harbard_core::status::owed_notes`], the same list
/// that ack last closes, using [`wake::calls_for`] as the watcher's routing
/// rule. The board can also show broadcast asks; those never hold an agent's
/// Stop because they did not call for that particular session.
///
/// Both harnesses read `{"decision":"block","reason":…}` on stdout with exit
/// 0 and turn the reason into the next prompt. `stop_hook_active` means this
/// stop already follows a held one, so it is never held again, and no note
/// holds more than [`wake::MAX_BLOCKS_PER_NOTE`] stops: a reminder repeated
/// past that point teaches the agent to answer the hook, not the person.
pub(crate) fn cmd_stop_check(store: &Store, common: &Common) -> i32 {
    let ctx = Context::resolve(common);
    if !ctx.session_is_stable {
        return 0;
    }
    let now = clock::now_ms();
    // Both Stop hooks run concurrently. The watcher re-reads this boundary
    // before delivering, so active-turn traffic cannot masquerade as idle mail.
    let _ = cursor::write_turn_end(store, &ctx.session, now);
    crate::oplog(
        store,
        &OpEvent::hook(now, "stop", &ctx.session, &ctx.harness),
    );
    let continued = context::hook_stdin()
        .and_then(|payload| payload.get("stop_hook_active"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if continued {
        return 0;
    }
    let read = store.read(Window::All);
    let channels = crate::my_channels(store, &ctx);
    let placement = harbard_core::workspaces::Placement::load(store);
    let gate = placement.gate(ctx.repo_key.as_deref().or(ctx.repo.as_deref()));
    let reader = reader_for(&ctx, &channels, &gate);
    let mut state = cursor::read_owed_state(store, &ctx.session);
    // The same list `ack last` closes, in the same order.
    let due: Vec<&harbard_core::Event> =
        harbard_core::status::owed_notes(&read.events, &reader, now)
            .into_iter()
            .filter(|note| {
                state.blocks.get(&note.id).copied().unwrap_or(0) < wake::MAX_BLOCKS_PER_NOTE
            })
            .take(wake::MAX_OWED_SHOWN)
            .collect();
    if due.is_empty() {
        return 0;
    }
    for note in &due {
        *state.blocks.entry(note.id.clone()).or_default() += 1;
    }
    let _ = cursor::write_owed_state(store, &ctx.session, &state);
    let reason = wake::render_owed(&due, &harbard_core::alias::read(store), now);
    let ids: Vec<String> = due.iter().map(|note| note.id.clone()).collect();
    crate::oplog(
        store,
        &OpEvent::injected(now, "stop", ids, "block", &ctx.session, &ctx.harness),
    );
    let decision = serde_json::json!({ "decision": "block", "reason": reason });
    crate::print_stdout(&decision.to_string());
    0
}
