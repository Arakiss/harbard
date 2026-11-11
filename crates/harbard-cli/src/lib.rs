//! harbard: the cross-harness session bus CLI (F0).
//!
//! All verbs support `--json`; stdout is stable, stderr is human. Hook-path
//! verbs (`brief`) stay bounded and fast, and no verb ever panics the caller.

pub mod agents;
pub mod archive;
pub mod attach;
pub mod audit;
pub mod clean;
pub mod cli;
pub mod context;
pub mod dependency;
pub mod friction;
pub mod gate;
pub mod hub;
pub mod hub_dx;
pub mod huginn;
pub mod install;
pub mod lane;
pub mod lifecycle;
pub mod machine;
pub mod mcp;
pub mod memory_sink;
pub mod render;
pub mod reply;
pub mod rooms;
pub mod serve;
pub mod setup;
pub mod sluice;
pub mod team_doctor;
pub mod theme;
pub mod trace;
pub mod update;
pub mod watch;
pub mod workspaces_cmd;

use clap::Parser;
use cli::{Cli, Cmd, Common, DeviceCmd, ExportWhat, HubCmd, LeaseTarget};
use context::Context;
use harbard_core::clock;
use harbard_core::query::{self, DEFAULT_STALE_SECS};
use harbard_core::{
    DEFAULT_RETENTION_DAYS, Event, EventKind, LeaseCheck, LeaseClaim, LeaseError, LeaseKind,
    LeaseMetadata, LeaseRecord, LeaseRelease, LeaseResource, LeaseStore, PidChecker,
    SCHEMA_VERSION, Store, SystemLeaseClock, SystemPidChecker, Window, same_writer_lineage,
};
use std::time::{Duration, UNIX_EPOCH};
use ulid::Ulid;

/// The `harbard` command line; `main.rs` only calls this.
pub fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    // A panic is the one failure no verb can catch and explain itself.
    let default_panic = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_panic(info);
        eprintln!("{}", report_line(None));
    }));
    let cli = Cli::try_parse().unwrap_or_else(|mut error| {
        use clap::error::{ContextKind, ContextValue, ErrorKind};

        if error.kind() == ErrorKind::InvalidSubcommand
            && matches!(error.get(ContextKind::InvalidSubcommand), Some(ContextValue::String(command)) if command == "lease")
        {
            error.remove(ContextKind::SuggestedSubcommand);
            error.insert(
                ContextKind::Suggested,
                ContextValue::StyledStrs(vec![
                    "Reservations use `harbard claim --writer` for the current Git worktree or `harbard claim --service NAME` for a service.".into(),
                    "Inspect with `harbard check --writer` or `harbard check --service NAME`; release with `harbard release --writer` or `harbard release --service NAME`.".into(),
                    "Use `harbard claim --help` for options. `lease` is not a command; no reservation was changed.".into(),
                ]),
            );
        }
        error.exit()
    });
    let common = &cli.common;
    let store = store_for(common);
    let now = clock::now_ms();

    // A hook that declares one harness while carrying another's payload is an
    // echo of a host that runs a foreign hook file (Cursor runs Claude Code's).
    // Answering it writes a second identity for a session that already spoke.
    //
    // Only the lifecycle verbs are dropped. `gate` in particular must ALWAYS
    // answer: a permission gate that stays silent is a gate that opened, and no
    // identity bookkeeping is worth turning off action safety for. Everything
    // else either reads or writes content the caller asked for by hand. The
    // Stop verbs are dropped too: an echo would hold or wake the Cursor
    // conversation under a Claude Code identity it never had.
    if matches!(
        &cli.cmd,
        Cmd::Hello { .. } | Cmd::Bye | Cmd::Brief { .. } | Cmd::Watch { .. } | Cmd::StopCheck
    ) && let Some(declared) = common.hook.as_deref()
        && context::cross_harness_hook(declared, context::hook_stdin())
    {
        let _ = harbard_core::oplog::append(
            &store,
            &harbard_core::oplog::OpEvent::cross_hook(now, declared, "cursor"),
        );
        return 0;
    }

    match &cli.cmd {
        Cmd::Hello { intent } => cmd_hello(&store, common, intent.as_deref(), now),
        Cmd::Name {
            name,
            reason,
            team,
            intro,
        } => {
            // Checked first: a name that took while its intro is refused
            // would be half of what was asked for.
            if let Some(text) = intro.as_deref()
                && let Err(why) = rooms::check_intro(text)
            {
                return fail(why);
            }
            let code = if harbard_core::workshop::listed(name).is_some() {
                // The console name first; the bus callsign is its ASCII
                // spelling (Hazel → Hazel), best effort.
                let code = pick_workshop_name(
                    &store,
                    common,
                    name,
                    reason.as_deref(),
                    team.as_deref(),
                    now,
                );
                if code == 0 {
                    let _ = cmd_name(&store, common, &ascii_fold(name), now);
                }
                code
            } else {
                if team.is_some() {
                    return fail(
                        "--team is for a name from the given-name list (Nora, Owen, Iris…): only those carry a team in the console; drop --team or pick one from the list",
                    );
                }
                if reason.is_some() {
                    eprintln!(
                        "harbard: --reason is for a name from the given-name list (Nora, Owen, Iris…)"
                    );
                }
                cmd_name(&store, common, name, now)
            };
            match intro.as_deref() {
                Some(text) if code == 0 => {
                    // `--json` already printed the name; one verb, one object.
                    let mut quiet = common.clone();
                    quiet.json = false;
                    rooms::cmd_intro(&store, &quiet, text, None, now)
                }
                _ => code,
            }
        }
        Cmd::Intro { text, agent } => rooms::cmd_intro(&store, common, text, agent.as_deref(), now),
        Cmd::Feedback {
            action,
            text,
            kind,
            verb,
        } => rooms::cmd_feedback(
            &store,
            common,
            action.as_ref(),
            text.as_deref(),
            kind.as_deref(),
            verb.as_deref(),
            now,
        ),
        Cmd::Channel { action } => rooms::cmd_channel(&store, common, action, now),
        Cmd::Intent { text } => cmd_write(&store, common, EventKind::Intent, text, None, now),
        Cmd::Note {
            kind,
            text,
            to,
            note_ref,
            fields,
            body_file,
            attach,
        } => {
            let extra = NoteExtra {
                body_file: body_file.as_deref(),
                attach,
                room: None,
            };
            cmd_note_with(
                &store,
                common,
                kind,
                text,
                to.as_deref(),
                note_ref.as_deref(),
                fields,
                &extra,
                now,
            )
        }
        Cmd::Reply {
            target,
            text,
            kind,
            fields,
            agent,
        } => reply::cmd_reply(
            &store,
            common,
            target,
            text,
            kind,
            fields,
            agent.as_deref(),
            now,
        ),
        Cmd::Relay { sent, limit } => {
            agents::cmd_relay(&store, common, sent.as_deref(), *limit, now)
        }
        Cmd::Bye => {
            let code = cmd_write(&store, common, EventKind::Bye, "", None, now);
            // SessionEnd/Stop: drop our writer lease so a crashed peer is not
            // the only reclaim path. Idempotent when we never held one.
            gate::release_writer_for_session(&store, common);
            lane::release_session_lanes(&store, common);
            code
        }
        Cmd::Gate { pid } => gate::cmd_gate(&store, common, *pid),
        Cmd::CheckoutPolicy { command } => lifecycle::cmd_checkout_policy(common, command),
        Cmd::Ls { all } => cmd_ls(&store, common, *all, now),
        Cmd::Agents {
            all,
            watch,
            interval_secs,
            frames,
        } => agents::cmd_agents(&store, common, *all, *watch, *interval_secs, *frames),
        Cmd::Owed { limit } => agents::cmd_owed(&store, common, *limit, now),
        Cmd::Fleet => cmd_fleet(&store, common, now),
        Cmd::Ack { target, note } => cmd_ack(&store, common, target, note.as_deref(), now),
        Cmd::Inbox => cmd_inbox(&store, common, now),
        Cmd::Dependency { action } => dependency::cmd_dependency(&store, common, action),
        Cmd::Await {
            target,
            purpose,
            ttl,
            timeout_secs,
            poll_ms,
        } => match target.iter().map(String::as_str).collect::<Vec<_>>()[..] {
            ["lane", lane] => lane::cmd_await_lane(
                &store,
                common,
                lane,
                purpose.as_deref(),
                ttl,
                *timeout_secs,
                *poll_ms,
            ),
            ["pr", number, condition] => {
                lane::cmd_await_pr(&store, common, number, condition, *timeout_secs, *poll_ms)
            }
            [id] if id != "lane" && id != "pr" => {
                dependency::cmd_await(&store, common, id, *timeout_secs, *poll_ms)
            }
            _ => fail(
                "await takes <dependency-id>, `lane <lane>`, or `pr <number> merged|closed`; for example `harbard await lane migrations --for \"apply 0042\"`",
            ),
        },
        Cmd::Lane { action } => lane::cmd_lane(&store, common, action),
        Cmd::Heartbeat { lane } => lane::cmd_heartbeat(&store, common, lane.as_deref()),
        Cmd::Attach { paths, label } => {
            attach::cmd_attach(&store, paths, label.as_deref(), common.json)
        }
        Cmd::Huginn { action } => huginn::cmd_huginn(action, common.json),
        Cmd::Protocol { subagent: true, .. } => cmd_subagent_brief(common),
        Cmd::Protocol { full, .. } => cmd_protocol(&store, common, *full, now),
        Cmd::Mcp => mcp::cmd_mcp(common),
        Cmd::Call { operation, input } => machine::cmd_call(common, operation, input),
        Cmd::Subscribe {
            after,
            once,
            poll_ms,
            heartbeat_ms,
            events,
            addressed,
        } => machine::cmd_subscribe(
            &store,
            common,
            after.as_deref(),
            *once,
            *poll_ms,
            *heartbeat_ms,
            events,
            *addressed,
        ),
        Cmd::Brief {
            touch,
            inline,
            throttle,
        } => {
            if *inline {
                cmd_brief_inline(&store, common, *throttle, now)
            } else {
                cmd_brief(&store, common, *touch, now)
            }
        }
        Cmd::Watch {
            lifetime_secs,
            poll_ms,
            coalesce_ms,
            detach,
            deliver,
            supervisor_pid,
        } => watch::cmd_watch(
            &store,
            common,
            watch::WatchOptions {
                lifetime: Duration::from_secs(*lifetime_secs),
                poll: Duration::from_millis(*poll_ms),
                coalesce: Duration::from_millis(*coalesce_ms),
                supervisor: *supervisor_pid,
            },
            *detach,
            deliver.as_deref(),
        ),
        Cmd::StopCheck => watch::cmd_stop_check(&store, common),
        Cmd::Deliver {
            sha,
            branch,
            checks,
            prepare,
            repo,
        } => match sluice_repo(repo) {
            Ok(repo) => {
                sluice::cmd_deliver(&store, common, repo, sha, branch, checks, prepare, now)
            }
            Err(code) => code,
        },
        Cmd::Integrate { repo } => match sluice_repo(repo) {
            Ok(repo) => sluice::cmd_integrate(&store, common, repo, now),
            Err(code) => code,
        },
        Cmd::Friction { limit } => friction::cmd_friction(&store, common, *limit),
        Cmd::Deliveries { repo } => match sluice_repo(repo) {
            Ok(repo) => sluice::cmd_deliveries(&store, common, repo),
            Err(code) => code,
        },
        Cmd::Join { channel, anchor } => cmd_membership(&store, common, channel, anchor, true, now),
        Cmd::Leave { channel, anchor } => {
            cmd_membership(&store, common, channel, anchor, false, now)
        }
        Cmd::Channels {
            create,
            archive,
            unarchive,
            delete,
            all,
        } => cmd_channels(
            &store,
            common,
            create.as_deref(),
            archive.as_deref(),
            unarchive.as_deref(),
            delete.as_deref(),
            *all,
        ),
        Cmd::Setup { dry_run } => cmd_setup(&store, common, *dry_run, now),
        Cmd::Install {
            target,
            dry_run,
            config,
            uninstall,
        } => cmd_install(target, *dry_run, config.as_deref(), *uninstall),
        Cmd::Thread { id } => cmd_thread(&store, common, id, now),
        Cmd::Search {
            query,
            limit,
            rebuild,
        } => cmd_search(&store, common, query, *limit, *rebuild),
        Cmd::Doctor => cmd_doctor(&store, common),
        Cmd::Archive { action } => archive::cmd_archive(&store, common, action),
        Cmd::Clean { finished, yes } => clean::cmd_clean(&store, common, *finished, *yes, now),
        Cmd::Trace { note, session } => {
            trace::cmd_trace(&store, common, note.as_deref(), session.as_deref(), now)
        }
        Cmd::Export { what } => match what {
            ExportWhat::Learning { since } => cmd_export_learning(&store, common, since, now),
        },
        Cmd::Theme { action } => theme::cmd_theme(&store, action),
        Cmd::Workspace { action } => workspaces_cmd::cmd_workspace(&store, common, action.as_ref()),
        Cmd::Update {
            check,
            yes,
            channel,
            source,
            install_dir,
            quiet,
            detach,
            run,
        } => update::cmd::cmd_update(
            &store,
            &update::cmd::Args {
                check: *check,
                yes: *yes,
                channel: channel.as_deref(),
                quiet: *quiet,
                detach: *detach,
                run: run.as_deref(),
                json: common.json,
                source: source.as_deref(),
                install_dir: install_dir.as_deref(),
            },
        ),
        Cmd::Audit {
            target,
            primary_only,
            include_worktrees: _,
        } => cmd_audit(common, target.as_deref(), !*primary_only),
        Cmd::Claim {
            target,
            ttl_secs,
            pid,
        } => cmd_claim(&store, common, target, *ttl_secs, *pid),
        Cmd::Check {
            target,
            claim_id,
            renew,
            pid,
        } => cmd_check(&store, common, target, claim_id.as_deref(), *renew, *pid),
        Cmd::Release {
            target,
            claim_id,
            pid,
        } => cmd_release(&store, common, target, claim_id.as_deref(), *pid),
        Cmd::Serve { port, bind, token } => {
            cmd_serve(store, common, port, bind, token.as_deref(), now)
        }
        Cmd::Hub { action } => cmd_hub(&store, common, action),
        Cmd::Pair {
            invite,
            name,
            no_install,
            no_daemon,
            forget,
        } => cmd_pair(
            &store,
            common,
            invite.as_deref(),
            name.as_deref(),
            *no_install,
            *no_daemon,
            *forget,
            now,
        ),
        Cmd::Device { action } => cmd_device(&store, common, action, now),
        Cmd::Satellite { interval } => cmd_satellite(&store, common, *interval, now),
    }
}

fn cmd_claim(
    store: &Store,
    common: &Common,
    target: &LeaseTarget,
    ttl_secs: u64,
    pid: Option<u32>,
) -> i32 {
    let invocation =
        match resolve_lease_invocation(common, target, pid, /*bind_writer_supervisor*/ true) {
            Ok(invocation) => invocation,
            Err(message) => return emit_lease_input_error(common, &message),
        };
    let leases = LeaseStore::at(store.root());
    match leases.claim(
        &invocation.resource,
        &invocation.metadata,
        ttl_secs,
        SystemLeaseClock,
        &SystemPidChecker,
    ) {
        Ok(LeaseClaim::Acquired(lease)) => {
            emit_lease_json_or_human(
                common,
                serde_json::json!({ "status": "acquired", "lease": lease }),
                format!(
                    "acquired {} {} until {}",
                    lease_kind(lease.resource.kind),
                    lease.claim_id,
                    lease.expires_at_ms
                ),
            );
            0
        }
        Ok(LeaseClaim::Renewed(lease)) => {
            emit_lease_json_or_human(
                common,
                serde_json::json!({ "status": "held", "lease": lease }),
                format!(
                    "held {} {} until {}",
                    lease_kind(lease.resource.kind),
                    lease.claim_id,
                    lease.expires_at_ms
                ),
            );
            0
        }
        Ok(LeaseClaim::Conflict(lease)) => {
            emit_lease_json_or_human(
                common,
                serde_json::json!({ "status": "conflict", "lease": lease_summary(&lease) }),
                format!(
                    "conflict {} until {}",
                    lease_kind(lease.resource.kind),
                    lease.expires_at_ms
                ),
            );
            2
        }
        Err(error) => emit_lease_error(common, &error),
    }
}

fn cmd_check(
    store: &Store,
    common: &Common,
    target: &LeaseTarget,
    claim_id: Option<&str>,
    renew: bool,
    pid: Option<u32>,
) -> i32 {
    let invocation = match resolve_lease_invocation(common, target, pid, renew && target.writer) {
        Ok(invocation) => invocation,
        Err(message) => return emit_lease_input_error(common, &message),
    };
    let leases = LeaseStore::at(store.root());
    let result = if renew {
        leases.renew_owned(
            &invocation.resource,
            claim_id.expect("clap requires claim id for renew"),
            &invocation.metadata,
            SystemLeaseClock,
            &SystemPidChecker,
        )
    } else {
        leases.check(
            &invocation.resource,
            claim_id,
            false,
            SystemLeaseClock,
            &SystemPidChecker,
        )
    };
    match result {
        Ok(LeaseCheck::Free) => {
            emit_lease_json_or_human(
                common,
                serde_json::json!({ "status": "free", "matches_claim": null }),
                format!("free {}", lease_kind(invocation.resource.kind)),
            );
            if claim_id.is_some() { 2 } else { 0 }
        }
        Ok(LeaseCheck::Active {
            lease,
            matches_claim,
        }) => {
            // Asked without a claim id, `check` used to answer `match=-` even
            // to the lease's own holder — the reading that sent agents looking
            // for a peer that was themselves. Identity does not need a token.
            let mine = matches_claim.is_none() && lease_is_mine(&lease, &invocation.metadata);
            let matches_claim = if mine { Some(true) } else { matches_claim };
            let authorized = matches_claim == Some(true);
            let visible_lease = if authorized {
                serde_json::to_value(&lease).unwrap_or_default()
            } else {
                lease_summary(&lease)
            };
            emit_lease_json_or_human(
                common,
                serde_json::json!({
                    "status": if renew && matches_claim == Some(true) { "renewed" } else { "active" },
                    "matches_claim": matches_claim,
                    "mine": mine,
                    "lease": visible_lease,
                }),
                if authorized {
                    format!(
                        "{} {} {} match={} until {}",
                        if renew && matches_claim == Some(true) {
                            "renewed"
                        } else {
                            "active"
                        },
                        lease_kind(lease.resource.kind),
                        lease.claim_id,
                        if mine {
                            "self"
                        } else {
                            match_label(matches_claim)
                        },
                        lease.expires_at_ms
                    )
                } else {
                    format!(
                        "active {} match={} until {}",
                        lease_kind(lease.resource.kind),
                        match_label(matches_claim),
                        lease.expires_at_ms
                    )
                },
            );
            if matches_claim == Some(false) { 2 } else { 0 }
        }
        Ok(LeaseCheck::Stale {
            lease,
            matches_claim,
        }) => {
            let authorized = matches_claim == Some(true);
            let visible_lease = if authorized {
                serde_json::to_value(&lease).unwrap_or_default()
            } else {
                lease_summary(&lease)
            };
            emit_lease_json_or_human(
                common,
                serde_json::json!({
                    "status": "stale",
                    "matches_claim": matches_claim,
                    "lease": visible_lease,
                }),
                if authorized {
                    format!(
                        "stale {} {} match={} expired {}",
                        lease_kind(lease.resource.kind),
                        lease.claim_id,
                        match_label(matches_claim),
                        lease.expires_at_ms
                    )
                } else {
                    format!(
                        "stale {} match={} expired {}",
                        lease_kind(lease.resource.kind),
                        match_label(matches_claim),
                        lease.expires_at_ms
                    )
                },
            );
            if claim_id.is_some() { 2 } else { 0 }
        }
        Err(error) => emit_lease_error(common, &error),
    }
}

fn cmd_release(
    store: &Store,
    common: &Common,
    target: &LeaseTarget,
    claim_id: Option<&str>,
    pid: Option<u32>,
) -> i32 {
    let invocation = match resolve_lease_invocation(common, target, pid, target.writer) {
        Ok(invocation) => invocation,
        Err(message) => return emit_lease_input_error(common, &message),
    };
    let leases = LeaseStore::at(store.root());
    // An owner should not have to dig its own token out of the store to let go.
    // Requiring it is why leases outlive the sessions that took them: the
    // session that could release knows who it is, and the id it was handed is
    // long gone from a shell that exited. `release_owned` still fences: an id
    // that belongs to somebody else is refused exactly as before.
    let claim_id = match claim_id {
        Some(id) => id.to_string(),
        None => {
            match leases.check(
                &invocation.resource,
                None,
                false,
                SystemLeaseClock,
                &SystemPidChecker,
            ) {
                Ok(LeaseCheck::Active { lease, .. }) | Ok(LeaseCheck::Stale { lease, .. })
                    if lease_is_mine(&lease, &invocation.metadata) =>
                {
                    lease.claim_id
                }
                Ok(LeaseCheck::Active { .. }) | Ok(LeaseCheck::Stale { .. }) => {
                    return emit_lease_input_error(
                        common,
                        "that lease belongs to another execution tree; pass --claim-id to fence it",
                    );
                }
                Ok(LeaseCheck::Free) => {
                    emit_lease_json_or_human(
                        common,
                        serde_json::json!({ "status": "free", "released": false }),
                        "nothing to release".to_string(),
                    );
                    return 0;
                }
                Err(error) => return emit_lease_error(common, &error),
            }
        }
    };
    let claim_id = claim_id.as_str();
    match leases.release_owned(
        &invocation.resource,
        claim_id,
        &invocation.metadata,
        SystemLeaseClock,
        &SystemPidChecker,
    ) {
        Ok(LeaseRelease::Released(lease)) => {
            emit_lease_json_or_human(
                common,
                serde_json::json!({ "status": "released", "released": true, "lease": lease }),
                format!(
                    "released {} {}",
                    lease_kind(lease.resource.kind),
                    lease.claim_id
                ),
            );
            0
        }
        Ok(LeaseRelease::Free) => {
            emit_lease_json_or_human(
                common,
                serde_json::json!({ "status": "free", "released": false }),
                format!("free {}", lease_kind(invocation.resource.kind)),
            );
            0
        }
        Ok(LeaseRelease::Fenced(lease)) => {
            emit_lease_json_or_human(
                common,
                serde_json::json!({ "status": "fenced", "released": false, "lease": lease_summary(&lease) }),
                format!("fenced {}", lease_kind(lease.resource.kind)),
            );
            2
        }
        Err(error) => emit_lease_error(common, &error),
    }
}

struct LeaseInvocation {
    resource: LeaseResource,
    metadata: LeaseMetadata,
}

const WRITER_SUPERVISOR_HELP: &str = "writer leases need a live harness supervisor; \
pass --pid <harness-pid> (not $$ from a tool shell) or run claim from Claude Code, Codex, or Cursor";

fn resolve_lease_invocation(
    common: &Common,
    target: &LeaseTarget,
    pid: Option<u32>,
    bind_writer_supervisor: bool,
) -> Result<LeaseInvocation, String> {
    let context = Context::resolve(common);
    let (resource, repository_id, branch, head_sha) = if target.writer {
        if !context.session_is_stable {
            return Err("writer leases require --session or a stable harness session id".into());
        }
        let worktree = context
            .repo
            .as_deref()
            .ok_or_else(|| "writer leases require a Git worktree".to_string())?;
        let repo = std::path::Path::new(worktree);
        let common_dir = context::git_common_dir(repo)
            .ok_or_else(|| "writer leases require a resolvable Git common directory".to_string())?
            .display()
            .to_string();
        (
            LeaseResource::writer(&context.host, &common_dir, worktree)
                .map_err(|_| "invalid writer resource".to_string())?,
            Some(common_dir),
            context.branch.clone(),
            context::git_head_sha(repo),
        )
    } else {
        let service = target
            .service
            .as_deref()
            .ok_or_else(|| "a service name is required".to_string())?;
        (
            LeaseResource::service(&context.host, service).map_err(|_| {
                "service name must use letters, digits, '.', '_', ':', '/', or '-'".to_string()
            })?,
            None,
            None,
            None,
        )
    };

    // Writer claims auto-bind a persistent harness ancestor. Service claims
    // never infer a pid — binding the ephemeral CLI would look valid and die.
    let pid = if target.writer && bind_writer_supervisor {
        match pid {
            Some(explicit) => Some(explicit),
            None => Some(
                context::supervised_harness_pid(&context.harness)
                    .ok_or_else(|| WRITER_SUPERVISOR_HELP.to_string())?,
            ),
        }
    } else {
        pid
    };

    Ok(LeaseInvocation {
        resource,
        metadata: LeaseMetadata {
            session: context.session,
            harness: context.harness,
            pid,
            parent: context.parent,
            repository_id,
            branch,
            head_sha,
        },
    })
}

fn emit_lease_json_or_human(common: &Common, value: serde_json::Value, human: String) {
    if common.json {
        print_stdout(&serde_json::to_string_pretty(&value).unwrap_or_else(|_| "{}".to_string()));
    } else {
        print_stdout(&human);
    }
}

fn emit_lease_input_error(common: &Common, message: &str) -> i32 {
    if common.json {
        emit_lease_json_or_human(
            common,
            serde_json::json!({ "status": "error", "error": "invalid_input" }),
            String::new(),
        );
        eprintln!("harbard: {message}");
    } else {
        eprintln!("harbard: {message}");
    }
    1
}

fn emit_lease_error(common: &Common, error: &LeaseError) -> i32 {
    let code = match error {
        LeaseError::CorruptState => "corrupt_state",
        LeaseError::Io(_) => "storage_error",
        LeaseError::InvalidResource(_)
        | LeaseError::InvalidMetadata(_)
        | LeaseError::InvalidTtl
        | LeaseError::DeadPid => "invalid_input",
    };
    if common.json {
        emit_lease_json_or_human(
            common,
            serde_json::json!({ "status": "error", "error": code }),
            String::new(),
        );
    }
    eprintln!("harbard: {error}");
    1
}

fn lease_kind(kind: LeaseKind) -> &'static str {
    match kind {
        LeaseKind::Writer => "writer",
        LeaseKind::Service => "service",
    }
}

fn match_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "yes",
        Some(false) => "no",
        None => "-",
    }
}

/// Is this writer lease already held by the caller's own execution tree?
///
/// `check` without a claim id has no token to compare, but it does know who is
/// asking. Answering `match=-` to the holder is what made a session read its
/// own lease as a peer's and stand down for a checkout it already owned.
fn lease_is_mine(lease: &LeaseRecord, requester: &LeaseMetadata) -> bool {
    lease.resource.kind == LeaseKind::Writer
        && same_writer_lineage(
            &lease.session,
            lease.parent.as_deref(),
            lease.pid,
            &requester.session,
            requester.parent.as_deref(),
            requester.pid,
            |pid| SystemPidChecker.is_alive(pid),
        )
}

fn lease_summary(lease: &harbard_core::LeaseRecord) -> serde_json::Value {
    serde_json::json!({
        "resource": lease.resource,
        "expires_at_ms": lease.expires_at_ms,
    })
}

/// The delivery queue's `--repo` (it shadows the global one, so it is a path
/// too), as the text the queue records; `.` when absent. A path that is not
/// UTF-8 is refused rather than recorded under a lossy spelling.
fn sluice_repo(repo: &Option<std::path::PathBuf>) -> Result<&str, i32> {
    match repo {
        None => Ok("."),
        Some(path) => path.to_str().ok_or_else(|| {
            fail(format!(
                "--repo {}: the delivery queue records repositories as UTF-8 text and this path is not",
                path.display()
            ))
        }),
    }
}

fn cmd_audit(common: &Common, target: Option<&str>, include_worktrees: bool) -> i32 {
    // Same default as every other verb: cwd's git root, or an explicit --repo.
    // An explicit path stays a path all the way down, so a checkout whose path
    // is not UTF-8 is audited where it is, not at a lossy spelling of it.
    let repo = match &common.repo {
        Some(path) => context::git_root(path).unwrap_or_else(|| path.clone()),
        None => {
            let ctx = Context::resolve(common);
            match ctx.repo.filter(|repo| !repo.trim().is_empty()) {
                Some(repo) => std::path::PathBuf::from(repo),
                None => {
                    return fail(
                        "audit needs a git checkout (run inside one, or pass --repo <path>)",
                    );
                }
            }
        }
    };

    let report = audit::inspect(&repo, target, include_worktrees);
    print_stdout(&audit::render(&report, common.json));

    if report.has_repository() { 0 } else { 1 }
}

fn cmd_serve(
    store: Store,
    common: &Common,
    port: &Option<u16>,
    bind: &str,
    token: Option<&str>,
    now: u64,
) -> i32 {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    // Serve never listens on a wildcard. `tailnet` adds the Mac's tailnet
    // address when `{home}/tailnet.json` names one; the retired bare LAN
    // bind (`lan`) now means the same.
    let tailnet_addr = match bind.trim().to_ascii_lowercase().as_str() {
        "loopback" | "localhost" | "127.0.0.1" => None,
        "tailnet" | "lan" | "all" | "0.0.0.0" => {
            let addr = serve::tailnet_addr(&store);
            if addr.is_none() {
                eprintln!(
                    "harbard serve: no tailnet address in {}/{}; listening on loopback only",
                    store.root().display(),
                    serve::TAILNET_CONFIG
                );
            }
            addr
        }
        other => {
            eprintln!("harbard serve: unknown --bind {other} (expected loopback|tailnet)");
            return 2;
        }
    };
    let bind_host = "127.0.0.1".to_string();
    let configured_token = token
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .or_else(|| {
            std::env::var("HARBARD_HUB_TOKEN")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
        })
        .or_else(|| hub_dx::read_hub_token(&store));
    let token = match configured_token {
        Some(token) => token,
        None => {
            let token = hub_dx::generate_hub_token();
            if let Err(error) = hub_dx::write_hub_token(&store, &token) {
                eprintln!("harbard serve: could not create the private API token ({error})");
                return 1;
            }
            token
        }
    };

    let cfg = serve::Config {
        port: port.unwrap_or(serve::DEFAULT_PORT),
        bind_host,
        token: Some(token),
        stale_secs: stale_secs(common),
        retention_days: common.retention_days.unwrap_or(DEFAULT_RETENTION_DAYS),
        started_ms: now,
        device_id: String::new(),
        tailnet_addr,
    };

    let shutdown = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&shutdown);
    if let Err(e) = ctrlc::set_handler(move || flag.store(true, Ordering::Relaxed)) {
        eprintln!("harbard serve: signal handler unavailable ({e}); Ctrl-C will hard-stop");
    }

    match serve::run(store, cfg, shutdown) {
        Ok(()) => 0,
        Err(e) => fail(e),
    }
}

fn cmd_hub(store: &Store, common: &Common, action: &HubCmd) -> i32 {
    match action {
        HubCmd::Enable {
            port,
            rotate,
            no_reload,
        } => {
            let token = if *rotate {
                hub_dx::generate_hub_token()
            } else {
                hub_dx::read_hub_token(store).unwrap_or_else(hub_dx::generate_hub_token)
            };
            if let Err(e) = hub_dx::write_hub_token(store, &token) {
                return fail(e);
            }
            let plist = match hub_dx::write_serve_launchd(*port, true, Some(&token)) {
                Ok(p) => Some(p),
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("HARBARD_HOME") {
                        eprintln!("harbard hub enable: {msg}");
                        eprintln!(
                            "  token written; launchd skipped. For a real hub use default ~/.harbard."
                        );
                        None
                    } else {
                        return fail(e);
                    }
                }
            };
            if let Some(ref plist) = plist
                && !*no_reload
                && let Err(e) = hub_dx::kickstart_serve()
            {
                eprintln!("harbard hub enable: launchd reload failed ({e})");
                eprintln!("  plist written: {}", plist.display());
                eprintln!(
                    "  try: launchctl bootstrap gui/$(id -u) {}",
                    plist.display()
                );
            }
            let tailnet = serve::tailnet_addr(store);
            if common.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": true,
                        "port": port,
                        "token_path": hub_dx::hub_token_path(store).display().to_string(),
                        "plist": plist.as_ref().map(|p| p.display().to_string()),
                        "tailnet": tailnet.map(|a| a.to_string()),
                    })
                );
            } else {
                println!(
                    "hub enabled · :{port} · own key → {} (this Mac only)",
                    hub_dx::hub_token_path(store).display()
                );
                match tailnet {
                    Some(a) => println!("tailnet      {a}"),
                    None => println!(
                        "tailnet      not configured ({}/{}); loopback only",
                        store.root().display(),
                        serve::TAILNET_CONFIG
                    ),
                }
                if let Some(plist) = &plist {
                    println!("serve plist · {}", plist.display());
                }
                println!();
                println!(
                    "invite a device: harbard hub invite --name <device> [--role taller|pantalla]"
                );
            }
            0
        }
        HubCmd::Disable { port, no_reload } => {
            let plist = match hub_dx::write_serve_launchd(*port, false, None) {
                Ok(p) => Some(p),
                Err(e) => {
                    let msg = e.to_string();
                    if msg.contains("HARBARD_HOME") {
                        eprintln!("harbard hub disable: {msg}");
                        None
                    } else {
                        return fail(e);
                    }
                }
            };
            if let Some(ref plist) = plist
                && !*no_reload
                && let Err(e) = hub_dx::kickstart_serve()
            {
                eprintln!("harbard hub disable: launchd reload failed ({e})");
                eprintln!("  plist written: {}", plist.display());
            }
            if common.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "ok": true,
                        "bind": "loopback",
                        "plist": plist.as_ref().map(|p| p.display().to_string()),
                        "token_kept": hub_dx::read_hub_token(store).is_some(),
                    })
                );
            } else {
                println!("hub disabled · loopback only · token file kept");
                println!("re-enable: harbard hub enable");
            }
            0
        }
        HubCmd::Invite {
            port,
            name,
            role,
            workspaces,
        } => {
            let Some(access) = harbard_core::Access::parse(role) else {
                eprintln!("harbard hub invite: unknown --role {role} (expected taller|pantalla)");
                return 2;
            };
            if !workspaces.is_empty() {
                let set = match workspaces_cmd::ensure(store) {
                    Ok((set, _)) => set,
                    Err(e) => return fail(e),
                };
                if let Some(unknown) = workspaces.iter().find(|w| set.get(w).is_none()) {
                    eprintln!(
                        "harbard hub invite: no workspace has the id \"{unknown}\" (harbard workspace ls)"
                    );
                    return 2;
                }
            }
            let issued = harbard_core::pairing::issue_granting(
                store,
                name,
                access,
                workspaces,
                clock::now_ms(),
            );
            let token = match issued {
                Ok(Ok(token)) => token,
                Ok(Err(harbard_core::pairing::IssueError::OperatorByInvite)) => {
                    eprintln!("harbard hub invite: an invite never hands out Operador.");
                    eprintln!("  pair it as taller, then promote it on purpose:");
                    eprintln!(
                        "  harbard hub role <device> operador --yes  (or Preferencias › Equipos)"
                    );
                    return 2;
                }
                Ok(Err(harbard_core::pairing::IssueError::EmptyName)) => {
                    eprintln!("harbard hub invite: --name must not be empty");
                    return 2;
                }
                Err(e) => return fail(e),
            };
            let host = serve::tailnet_addr(store).map(|a| a.to_string());
            if common.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "invite": hub_dx::invite_line(host.as_deref(), *port, &token),
                        "name": name,
                        "role": access.as_str(),
                        "workspaces": workspaces,
                        "expires_in_hours": harbard_core::pairing::INVITE_TTL_MS / 3_600_000,
                        "tailnet": host,
                        "port": port,
                    })
                );
            } else {
                hub_dx::print_invite_block(host.as_deref(), *port, &token, name, access.as_str());
                if !workspaces.is_empty() {
                    println!("granted when it pairs: {}", workspaces.join(", "));
                }
            }
            0
        }
        HubCmd::Rotate => match hub_dx::rotate_hub_token(store) {
            Ok(r) => {
                if common.json {
                    println!("{}", serde_json::json!(r));
                } else {
                    println!("{}", hub_dx::rotated_text(&r));
                }
                0
            }
            Err(e) => fail(e),
        },
        HubCmd::Revoke { device, invite } => {
            let reg = harbard_core::DeviceRegistry::load(store);
            let found = if *invite {
                Err(0)
            } else {
                harbard_core::pairing::find_device(&reg, device).map(|card| card.id.clone())
            };
            // No device by that name: a pending invite (a leaked pair line).
            if let Err(0) = found {
                return match harbard_core::pairing::revoke_invite(store, device, clock::now_ms()) {
                    Ok(revoked) if !revoked.is_empty() => {
                        if common.json {
                            let refs: Vec<_> = revoked
                                .iter()
                                .map(|c| {
                                    serde_json::json!({
                                        "ref": harbard_core::pairing::invite_ref(c),
                                        "name": c.name,
                                    })
                                })
                                .collect();
                            println!("{}", serde_json::json!({ "ok": true, "invites": refs }));
                        } else {
                            for c in &revoked {
                                println!(
                                    "revoked · pending invite {} ({}) · it can no longer pair",
                                    c.name,
                                    harbard_core::pairing::invite_ref(c)
                                );
                            }
                        }
                        0
                    }
                    Ok(_) => {
                        eprintln!(
                            "harbard hub revoke: no device or pending invite matches \"{device}\" (harbard hub status)"
                        );
                        2
                    }
                    Err(e) => fail(e),
                };
            }
            let id = match found {
                Ok(id) => id,
                Err(n) => {
                    eprintln!(
                        "harbard hub revoke: {n} devices match \"{device}\"; use the full id (harbard hub status)"
                    );
                    return 2;
                }
            };
            match harbard_core::pairing::revoke(store, &id, clock::now_ms()) {
                Ok(Some(card)) => {
                    if common.json {
                        println!("{}", serde_json::json!({ "ok": true, "device": card }));
                    } else {
                        println!(
                            "revoked · {} ({}) · refused from its next request",
                            card.name, card.id
                        );
                    }
                    0
                }
                Ok(None) => {
                    eprintln!(
                        "harbard hub revoke: {device} is this hub itself, which is never revoked"
                    );
                    2
                }
                Err(e) => fail(e),
            }
        }
        HubCmd::Role { device, role, yes } => {
            let Some(access) = harbard_core::Access::parse(role) else {
                eprintln!(
                    "harbard hub role: unknown role {role} (expected operador|taller|pantalla)"
                );
                return 2;
            };
            if access == harbard_core::Access::Operator && !*yes {
                eprintln!(
                    "harbard hub role: Operador reads everything, the profile and every transcript included."
                );
                eprintln!("  promote on purpose: harbard hub role {device} operador --yes");
                return 2;
            }
            let reg = harbard_core::DeviceRegistry::load(store);
            let id = match harbard_core::pairing::find_device(&reg, device) {
                Ok(card) => card.id.clone(),
                Err(n) => {
                    eprintln!(
                        "harbard hub role: {n} devices match \"{device}\"; use the full id (harbard hub status)"
                    );
                    return 2;
                }
            };
            match harbard_core::pairing::set_access(store, &id, access) {
                Ok(Ok(card)) => {
                    if common.json {
                        println!("{}", serde_json::json!({ "ok": true, "device": card }));
                    } else {
                        println!("{} ({}) · {}", card.name, card.id, card.access.as_str());
                    }
                    0
                }
                Ok(Err(e)) => {
                    eprintln!("harbard hub role: refused ({e:?})");
                    2
                }
                Err(e) => fail(e),
            }
        }
        HubCmd::Grant { device, workspaces } => {
            let set = match workspaces_cmd::ensure(store) {
                Ok((set, _)) => set,
                Err(e) => return fail(e),
            };
            if let Some(unknown) = workspaces.iter().find(|w| set.get(w).is_none()) {
                eprintln!(
                    "harbard hub grant: no workspace has the id \"{unknown}\" (harbard workspace ls)"
                );
                return 2;
            }
            let reg = harbard_core::DeviceRegistry::load(store);
            let id = match harbard_core::pairing::find_device(&reg, device) {
                Ok(card) => card.id.clone(),
                Err(n) => {
                    eprintln!(
                        "harbard hub grant: {n} devices match \"{device}\"; use the full id (harbard hub status)"
                    );
                    return 2;
                }
            };
            let granted = harbard_core::pairing::update(store, |reg, _| {
                reg.get_mut(&id).map(|card| {
                    card.workspaces = workspaces.clone();
                    card.clone()
                })
            });
            match granted {
                Ok(Some(card)) => {
                    let sees = harbard_core::workspaces::device_grants(&card, &set);
                    if common.json {
                        println!(
                            "{}",
                            serde_json::json!({ "ok": true, "device": card.id, "sees": sees })
                        );
                    } else {
                        println!("{} ({}) sees: {}", card.name, card.id, sees.join(", "));
                    }
                    0
                }
                Ok(None) => fail(format!("no device {device}")),
                Err(e) => fail(e),
            }
        }
        HubCmd::Status => {
            let token = hub_dx::read_hub_token(store);
            let reg = harbard_core::DeviceRegistry::load(store);
            let lan = hub_dx::lan_ipv4();
            let serve_plist = hub_dx::file_exists(&hub_dx::launchd_serve_plist());
            // Probe local serve if up.
            let meta = ureq::AgentBuilder::new()
                .timeout(std::time::Duration::from_secs(2))
                .build()
                .get("http://127.0.0.1:7419/api/v1/meta")
                .call()
                .ok()
                .and_then(|r| r.into_json::<serde_json::Value>().ok());
            let creds = harbard_core::pairing::Credentials::load(store);
            let now_ms = clock::now_ms();
            let invites: Vec<serde_json::Value> =
                harbard_core::pairing::pending_invites(&creds, now_ms)
                    .into_iter()
                    .map(|c| {
                        serde_json::json!({
                            "ref": harbard_core::pairing::invite_ref(c),
                            "name": c.name,
                            "access": c.access,
                            "created_ms": c.created_ms,
                            "expires_ms": c.expires_ms,
                        })
                    })
                    .collect();
            if common.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "hub_token": token.is_some(),
                        "lan_ip": lan,
                        "devices": reg.devices,
                        "invites": invites,
                        "serve_plist": serve_plist,
                        "meta": meta,
                    })
                );
            } else {
                println!(
                    "token        {}",
                    if token.is_some() {
                        "present (~/.harbard/hub.token)"
                    } else {
                        "missing — harbard hub enable"
                    }
                );
                println!(
                    "lan          {}",
                    lan.as_deref().unwrap_or("unknown (offline?)")
                );
                println!(
                    "launchd      com.harbard.serve {}",
                    if serve_plist { "plist ok" } else { "no plist" }
                );
                if let Some(m) = &meta {
                    let hub = m.get("hub").and_then(|h| h.as_bool()).unwrap_or(false);
                    let bind = m.get("bind").and_then(|b| b.as_str()).unwrap_or("?");
                    println!("serve        up · bind={bind} · hub={hub}");
                } else {
                    println!("serve        not reachable on :7419");
                }
                println!("devices      {}", reg.devices.len());
                for d in &reg.devices {
                    let state = if d.revoked_ms.is_some() {
                        "revoked"
                    } else if d.role == harbard_core::DeviceRole::Hub || d.created_ms > 0 {
                        "paired"
                    } else {
                        "no credential"
                    };
                    println!(
                        "  {:<10} {:<9} {:<16} {:<14} {}",
                        d.role.as_str(),
                        d.access.as_str(),
                        d.name,
                        state,
                        d.id
                    );
                }
                if !invites.is_empty() {
                    println!("invites      {} pending", invites.len());
                    for i in &invites {
                        let left_h = i["expires_ms"]
                            .as_u64()
                            .map(|at| at.saturating_sub(now_ms) / 3_600_000)
                            .unwrap_or(0);
                        println!(
                            "  {:<10} {:<9} {:<16} {:<14} {}",
                            "invite",
                            i["access"].as_str().unwrap_or("?"),
                            i["name"].as_str().unwrap_or(""),
                            format!("{left_h} h left"),
                            i["ref"].as_str().unwrap_or("")
                        );
                    }
                    println!("  cancel one: harbard hub revoke --invite <name|ref>");
                }
                println!();
                println!("invite: harbard hub invite --name <device>");
            }
            0
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn cmd_pair(
    store: &Store,
    common: &Common,
    invite: Option<&str>,
    name: Option<&str>,
    no_install: bool,
    no_daemon: bool,
    forget: bool,
    now: u64,
) -> i32 {
    use harbard_core::{DeviceRole, LocalDevice};

    if forget {
        let _ = hub_dx::stop_satellite_launchd();
        if let Err(e) = hub_dx::SatelliteConfig::remove(store) {
            return fail(e);
        }
        if common.json {
            println!("{}", serde_json::json!({ "ok": true, "forgot": true }));
        } else {
            println!("unpaired · satellite.json removed · launchd stopped");
        }
        return 0;
    }

    let Some(raw) = invite.map(str::trim).filter(|s| !s.is_empty()) else {
        // Status / usage.
        if let Some(cfg) = hub_dx::SatelliteConfig::load(store) {
            if common.json {
                println!("{}", serde_json::to_string_pretty(&cfg).unwrap_or_default());
            } else {
                println!("paired · {} → {}", cfg.name, cfg.hub);
                println!("device · {}", cfg.device_id);
                println!("forget · harbard pair --forget");
            }
            return 0;
        }
        eprintln!("usage: harbard pair 'http://<hub-ip>:7419#<token>'");
        eprintln!("  get the invite from the mini: harbard hub invite");
        return 2;
    };

    let (hub_url, token) = match hub_dx::parse_pair_invite(raw) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("harbard pair: {e}");
            return 2;
        }
    };

    let mut local = match LocalDevice::load_or_create(store, DeviceRole::Satellite) {
        Ok(d) => d,
        Err(e) => return fail(e),
    };
    if let Some(n) = name {
        local.name = n.to_string();
        let _ = std::fs::write(
            LocalDevice::path(store),
            serde_json::to_string_pretty(&local).unwrap_or_default(),
        );
    }

    let cfg = hub_dx::SatelliteConfig {
        hub: hub_url.clone(),
        token: token.clone(),
        device_id: local.id.clone(),
        name: local.name.clone(),
    };
    if let Err(e) = cfg.save(store) {
        return fail(e);
    }

    // Make this process speak hub immediately (also for install children).
    unsafe {
        std::env::set_var("HARBARD_HUB", &hub_url);
        std::env::set_var("HARBARD_HUB_TOKEN", &token);
        std::env::set_var("HARBARD_DEVICE_ID", &local.id);
    }

    let client = hub::HubClient {
        base: hub_url.clone(),
        token: token.clone(),
        device_id: local.id.clone(),
    };
    let card = update::stamp(local.to_card(
        now,
        vec!["claude-code".into(), "codex".into(), "cursor".into()],
    ));
    let (card, joined) = match client.register_device(&card) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("harbard pair: could not reach hub ({e})");
            eprintln!("  check WiFi / firewall, and that the mini ran `harbard hub enable`");
            eprintln!("  probe: curl -H \"Authorization: Bearer …\" {hub_url}/api/v1/meta");
            return 1;
        }
    };

    // Same detect→wire path as `harbard setup` — only present harnesses.
    let mut installed = Vec::new();
    if !no_install {
        match setup::run(store, false, None, now) {
            Ok(report) => {
                if let Some(list) = report.get("harnesses").and_then(|v| v.as_array()) {
                    for h in list {
                        let present = h.get("present").and_then(|v| v.as_bool()).unwrap_or(false);
                        let action = h.get("action").and_then(|v| v.as_str()).unwrap_or("");
                        let target = h.get("target").and_then(|v| v.as_str()).unwrap_or("?");
                        if present && matches!(action, "installed" | "unchanged") {
                            installed.push(target.to_string());
                        }
                    }
                }
            }
            Err(e) => eprintln!("harbard pair: setup hooks skipped ({e})"),
        }
    }

    let mut daemon = false;
    if !no_daemon {
        match hub_dx::write_satellite_launchd(store) {
            Ok(plist) => match hub_dx::kickstart_satellite() {
                Ok(()) => daemon = true,
                Err(e) => {
                    eprintln!("harbard pair: launchd satellite failed ({e})");
                    eprintln!("  plist: {}", plist.display());
                    eprintln!("  fallback: harbard satellite");
                }
            },
            Err(e) => eprintln!("harbard pair: could not write satellite plist ({e})"),
        }
    }

    if common.json {
        println!(
            "{}",
            serde_json::json!({
                "ok": true,
                "joined": joined,
                "device": card,
                "hub": hub_url,
                "hooks": installed,
                "daemon": daemon,
            })
        );
    } else {
        let verb = if joined { "joined" } else { "re-paired" };
        println!("{verb} · {} ({}) → {hub_url}", card.name, card.id);
        if !installed.is_empty() {
            println!("hooks · {}", installed.join(", "));
        }
        if daemon {
            println!("satellite · launchd com.harbard.satellite (KeepAlive)");
        } else if no_daemon {
            println!("next · harbard satellite");
        }
        println!();
        println!("writes go to the hub; this device publishes heartbeats here.");
    }
    0
}

fn cmd_device(store: &Store, common: &Common, action: &DeviceCmd, now: u64) -> i32 {
    use harbard_core::{DeviceRole, LocalDevice};

    match action {
        DeviceCmd::Show => {
            let role = if hub::HubClient::resolve(store).is_some() {
                DeviceRole::Satellite
            } else {
                DeviceRole::Hub
            };
            match LocalDevice::load_or_create(store, role) {
                Ok(d) => {
                    if common.json {
                        println!("{}", serde_json::to_string(&d).unwrap_or_default());
                    } else {
                        println!("{}  {}  ({})", d.id, d.name, d.role.as_str());
                    }
                    0
                }
                Err(e) => fail(e),
            }
        }
        DeviceCmd::Join { name } => {
            let Some(mut client) = hub::HubClient::resolve(store) else {
                eprintln!("harbard device join: not paired");
                eprintln!("  one-shot: harbard pair 'http://<hub-ip>:7419#<token>'");
                eprintln!("  invite:   on the mini, run `harbard hub invite`");
                return 2;
            };
            let mut local = match LocalDevice::load_or_create(store, DeviceRole::Satellite) {
                Ok(d) => d,
                Err(e) => return fail(e),
            };
            if let Some(n) = name {
                local.name = n.clone();
                let _ = std::fs::write(
                    LocalDevice::path(store),
                    serde_json::to_string_pretty(&local).unwrap_or_default(),
                );
            }
            client.device_id = local.id.clone();
            // Export for subsequent CLI calls in this shell — operator should
            // also put HARBARD_DEVICE_ID in their env permanently.
            // SAFETY: single-threaded CLI process.
            unsafe {
                std::env::set_var("HARBARD_DEVICE_ID", &local.id);
            }
            let card = update::stamp(local.to_card(
                now,
                vec!["claude-code".into(), "codex".into(), "cursor".into()],
            ));
            match client.register_device(&card) {
                Ok((card, joined)) => {
                    if common.json {
                        println!(
                            "{}",
                            serde_json::json!({ "device": card, "joined": joined })
                        );
                    } else if joined {
                        println!("joined hub as {} ({})", card.name, card.id);
                    } else {
                        println!("heartbeat ok · {} ({})", card.name, card.id);
                    }
                    0
                }
                Err(e) => fail(e),
            }
        }
        DeviceCmd::Ls => {
            let Some(client) = hub::HubClient::resolve(store) else {
                // Local hub registry when running on the mini without remote env.
                let reg = harbard_core::DeviceRegistry::load(store);
                if common.json {
                    println!(
                        "{}",
                        serde_json::to_string(&reg.devices).unwrap_or_default()
                    );
                } else {
                    for d in &reg.devices {
                        println!(
                            "{:<8} {:<16} {:<24} {}",
                            d.role.as_str(),
                            d.name,
                            d.host,
                            d.id
                        );
                    }
                }
                return 0;
            };
            match client.list_devices() {
                Ok(devices) => {
                    if common.json {
                        println!("{}", serde_json::to_string(&devices).unwrap_or_default());
                    } else {
                        for d in &devices {
                            println!(
                                "{:<8} {:<16} {:<24} {}",
                                d.role.as_str(),
                                d.name,
                                d.host,
                                d.id
                            );
                        }
                    }
                    0
                }
                Err(e) => fail(e),
            }
        }
    }
}

fn cmd_satellite(store: &Store, _common: &Common, interval: u64, now: u64) -> i32 {
    use harbard_core::{DeviceRole, LocalDevice};
    use std::thread;
    use std::time::Duration;

    let Some(mut client) = hub::HubClient::resolve(store) else {
        eprintln!("harbard satellite: not paired");
        eprintln!("  one-shot: harbard pair 'http://<hub-ip>:7419#<token>'");
        eprintln!("  invite:   on the mini, run `harbard hub invite`");
        return 2;
    };
    let local = match LocalDevice::load_or_create(store, DeviceRole::Satellite) {
        Ok(d) => d,
        Err(e) => return fail(e),
    };
    client.device_id = local.id.clone();
    unsafe {
        std::env::set_var("HARBARD_DEVICE_ID", &local.id);
    }

    eprintln!(
        "harbard satellite: device {} → {} (every {interval}s)",
        local.id, client.base
    );

    let mut tick = now;
    loop {
        let card = update::stamp(local.to_card(
            tick,
            vec!["claude-code".into(), "codex".into(), "cursor".into()],
        ));
        if let Err(e) = client.register_device(&card) {
            eprintln!("harbard satellite: heartbeat failed: {e}");
        }
        thread::sleep(Duration::from_secs(interval.max(5)));
        tick = clock::now_ms();
    }
}

/// Append via hub when configured; otherwise local store.
fn append_event(store: &Store, event: &Event) -> Result<(), String> {
    if let Some(client) = hub::HubClient::resolve(store) {
        client
            .append_event(event)
            .map(|_| ())
            .map_err(|e| e.to_string())?;
    } else {
        store.append(event).map_err(|e| e.to_string())?;
    }
    // Writing anything is a sign of life: a lane promoted to this session
    // while it was away starts its ttl now.
    lane::claim_turn(store, &event.session, clock::now_ms());
    Ok(())
}

/// Append to this machine's store and nowhere else, even on a paired satellite.
fn append_local(store: &Store, event: &Event) -> Result<(), String> {
    store.append(event).map_err(|e| e.to_string())?;
    lane::claim_turn(store, &event.session, clock::now_ms());
    Ok(())
}

/// A self-healing hello: written locally, where it gates the next one, and
/// on a paired satellite also announced to the hub, best-effort and briefly.
/// The hub gives a device a session only through that session's hello, so a
/// session whose SessionStart never fired could not speak there otherwise.
fn self_heal_hello(store: &Store, hello: &Event) {
    let _ = store.append(hello);
    if let Some(client) = hub::HubClient::resolve(store) {
        let _ = client.announce(hello);
    }
}

fn store_for(common: &Common) -> Store {
    match &common.home {
        Some(h) if !h.trim().is_empty() => Store::at(h),
        _ => Store::resolve(),
    }
}

fn stale_secs(common: &Common) -> u64 {
    common.stale_secs.unwrap_or(DEFAULT_STALE_SECS)
}

fn new_id(now: u64) -> String {
    let when = UNIX_EPOCH + Duration::from_millis(now);
    Ulid::from_datetime(when).to_string()
}

/// Resolve `--ref` without ever guessing. Exact ids and displayed prefixes are
/// accepted only when they name exactly one note still present in the store.
fn resolve_note_ref(store: &Store, note_ref: Option<&str>) -> Result<Option<String>, String> {
    let Some(raw) = note_ref.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    if !harbard_core::id::is_event_prefix(raw) {
        return Err(format!(
            "ref \"{raw}\" is not an event id or leading ULID prefix; copy the id from `harbard inbox` or `harbard thread <id>`"
        ));
    }
    let read = store.read(Window::All);
    match harbard_core::id::event(&read.events, raw, EventKind::Note) {
        Ok(parent) => Ok(Some(parent.id.clone())),
        Err(harbard_core::id::ResolveIdError::Unknown) => Err(format!(
            "no note id starts with \"{raw}\" — copy the full 26-character ref from the bus"
        )),
        Err(harbard_core::id::ResolveIdError::Ambiguous(count)) => Err(format!(
            "ref \"{raw}\" matches {} notes — copy the full 26-character ref from the bus",
            count
        )),
    }
}

fn build_event(
    ctx: &Context,
    kind: EventKind,
    text: &str,
    to: Option<harbard_core::Recipient>,
    now: u64,
) -> Event {
    build_event_kind(ctx, kind, text, to, now, harbard_core::NoteKind::Fyi)
}

fn build_event_kind(
    ctx: &Context,
    kind: EventKind,
    text: &str,
    to: Option<harbard_core::Recipient>,
    now: u64,
    note_kind: harbard_core::NoteKind,
) -> Event {
    Event {
        v: SCHEMA_VERSION,
        id: new_id(now),
        ts: clock::rfc3339_utc(now),
        event: kind,
        session: ctx.session.clone(),
        harness: ctx.harness.clone(),
        host: ctx.host.clone(),
        cwd: ctx.cwd.clone(),
        repo: ctx.repo.clone(),
        // Only when this session sits in a linked worktree. Equal to `repo` in
        // the common case, and every byte counts against MAX_LINE_BYTES.
        repo_key: ctx
            .repo_key
            .clone()
            .filter(|key| Some(key.as_str()) != ctx.repo.as_deref()),
        branch: ctx.branch.clone(),
        role: ctx.role,
        parent: ctx.parent.clone(),
        note_ref: None,
        text: text.to_string(),
        to,
        kind: note_kind,
    }
}

fn fail(msg: impl std::fmt::Display) -> i32 {
    eprintln!("harbard: {msg}");
    1
}

/// How an agent reports a failure of harbard itself.
fn report_line(verb: Option<&str>) -> String {
    let flag = verb
        .map(|verb| format!(" --verb \"{verb}\""))
        .unwrap_or_default();
    format!(
        "harbard: this is a fault in harbard, not in what you asked; to report it: harbard feedback --kind bug{flag} \"what you ran and what happened\""
    )
}

/// A failure of harbard itself: the store, the disk, a file it owns. Never a
/// refusal, which is an answer and goes through [`fail`]. Not offered when
/// `feedback` is what failed: it would point at the thing that just broke.
fn internal_failure(verb: &str, msg: impl std::fmt::Display) -> i32 {
    eprintln!("harbard: {msg}");
    if verb != "feedback" {
        eprintln!("{}", report_line(Some(verb)));
    }
    1
}

/// Bind a presence signal to the long-lived harness process that emitted it.
/// Best-effort: missing ancestry or a cursor write failure must
/// never break the session that Harbard is observing.
fn record_supervisor_signal(store: &Store, ctx: &Context, now: u64) {
    let Some((pid, start)) = supervisor_binding(&ctx.harness) else {
        return;
    };
    let mut state = harbard_core::cursor::read_live_state(store, &ctx.session);
    state.checked_ms = now;
    state.supervisor_pid = Some(pid);
    state.supervisor_start = start;
    let _ = harbard_core::cursor::write_live_state(store, &ctx.session, &state);
}

/// The long-lived harness process above this hook, paired with its start
/// marker. The two are always written together: a marker left over from a
/// previous PID would vouch for the wrong process. `None` start (unsupported
/// platform, unreadable process) keeps that session on time-based presence.
fn supervisor_binding(harness: &str) -> Option<(u32, Option<u64>)> {
    let pid = context::supervised_harness_pid(harness)?;
    Some((pid, SystemPidChecker.start_time(pid)))
}

fn cmd_hello(store: &Store, common: &Common, intent: Option<&str>, now: u64) -> i32 {
    let ctx = Context::resolve(common);
    let text = intent.unwrap_or("");
    let event = build_event(&ctx, EventKind::Hello, text, None, now);
    // Presence first; a full-disk write is the only refusal (E3).
    if let Err(e) = append_event(store, &event) {
        return internal_failure("hello", e);
    }
    record_supervisor_signal(store, &ctx, now);
    oplog(
        store,
        &harbard_core::oplog::OpEvent::hook(now, "hello", &ctx.session, &ctx.harness),
    );
    // A name, assigned once and remembered (F5). After presence, never before:
    // a session that appears on the board without a name is a smaller failure
    // than a session that does not appear at all.
    let mut aliases = harbard_core::alias::read_for_naming(store);
    let (alias, fresh) = if let Some(nick) = ctx.nickname.as_deref() {
        harbard_core::alias::claim(&mut aliases, &ctx.session, nick, now)
    } else {
        harbard_core::alias::resolve(&mut aliases, &ctx.session, now)
    };
    if fresh {
        let _ = harbard_core::alias::write(store, &aliases);
    }

    // Lazy GC — best-effort, never blocks presence.
    let _ = store.gc(common.retention_days.unwrap_or(DEFAULT_RETENTION_DAYS), now);
    let _ = harbard_core::oplog::gc(
        store,
        common.retention_days.unwrap_or(DEFAULT_RETENTION_DAYS),
        now,
    );
    // Dead sessions' cursors and delivery markers; at most once a day. It
    // judges age by file mtimes, so it reads the wall clock, never a
    // HARBARD_NOW_MS override.
    let pruned = harbard_core::prune::run_due(
        store,
        harbard_core::prune::wall_clock_ms(),
        &harbard_core::SystemPidChecker,
    );
    // The doctor's maintenance (resolution TTL, channel activity) used to ride
    // every prompt through the fleet health check. It rides this daily pass
    // now, so nothing depends on someone opening the console's doctor views.
    if pruned.is_some() {
        let _ = harbard_core::query::doctor(
            store,
            common.retention_days.unwrap_or(DEFAULT_RETENTION_DAYS),
        );
    }

    let receipt = harbard_core::protocol::receipt_hello(&event.id);
    if common.json {
        emit_write_with_receipt(common, &event, &receipt);
    } else {
        // Greet by name on the way in; it is how the operator learns which
        // one you are.
        eprintln!(
            "harbard: hello \u{2014} you are {} ({}) — {}",
            harbard_core::alias::display(&alias),
            short(&ctx.session),
            receipt.hint
        );
    }
    0
}

/// `harbard name Nora --reason "…"`: a name from the given-name list is
/// also this session's name in the console, with its reason in Profile.
/// Another name keeps only the bus callsign, as before.
fn pick_workshop_name(
    store: &Store,
    common: &Common,
    name: &str,
    reason: Option<&str>,
    team: Option<&str>,
    now: u64,
) -> i32 {
    use harbard_core::workshop::Registry;
    let ctx = Context::resolve(common);
    let read = store.read(Window::All);
    let live_sessions: Vec<String> = query::sessions(store, &read.events, now, stale_secs(common))
        .into_iter()
        .filter(|v| !matches!(v.liveness, harbard_core::query::Liveness::Ended))
        .map(|v| format!("session:{}", v.session))
        .collect();
    // In-process agents hold their names too: every one the console shows
    // (the team projection), and any named within that window, whose
    // transcript this machine may not see.
    let shown: Vec<String> =
        crate::serve::team_route::project(now, harbard_core::team::WINDOW_DEFAULT_MS)
            .leads
            .iter()
            .flat_map(|l| l.agents.iter().map(|a| a.id.clone()))
            .collect();
    let key = format!("session:{}", ctx.session);
    let picked = Registry::update(store.root(), |r| {
        let recent: Vec<String> = r
            .agents
            .iter()
            .filter(|(k, e)| {
                !k.starts_with("session:")
                    && now.saturating_sub(e.assigned_ms) <= harbard_core::team::WINDOW_DEFAULT_MS
            })
            .map(|(k, _)| k.clone())
            .collect();
        let live: std::collections::BTreeSet<&str> = live_sessions
            .iter()
            .chain(&shown)
            .chain(&recent)
            .map(String::as_str)
            .collect();
        let picked = r.pick(&key, name, reason, &live, now)?;
        if let Some(t) = team {
            r.set_team(&key, t);
        }
        Ok::<String, &'static str>(picked)
    });
    match picked {
        Ok(Ok(picked)) => {
            if !common.json {
                eprintln!("harbard: in the console you are {picked}");
            }
            0
        }
        Ok(Err(why)) => {
            eprintln!(
                "harbard: the console name {name} is not available ({why}); choose another from the list"
            );
            2
        }
        Err(error) => fail(error),
    }
}

/// `Hazel` → `Hazel`, `Victor` → `Pinon`.
fn ascii_fold(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            'á' | 'à' | 'ä' => 'a',
            'é' | 'è' | 'ë' => 'e',
            'í' | 'ì' | 'ï' => 'i',
            'ó' | 'ò' | 'ö' => 'o',
            'ú' | 'ù' | 'ü' => 'u',
            'ñ' => 'n',
            'Á' => 'A',
            'É' => 'E',
            'Í' => 'I',
            'Ó' => 'O',
            'Ú' => 'U',
            'Ñ' => 'N',
            other => other,
        })
        .collect()
}

fn cmd_name(store: &Store, common: &Common, name: &str, now: u64) -> i32 {
    let ctx = Context::resolve(common);
    let mut aliases = harbard_core::alias::read_for_naming(store);
    // Sweep the ghosts before answering. A name is refused only by somebody who
    // is still on the bus; a session that has ended does not keep holding one.
    let read = store.read(Window::All);
    let active: std::collections::HashSet<String> =
        query::sessions(store, &read.events, now, stale_secs(common))
            .into_iter()
            .filter(|v| !matches!(v.liveness, harbard_core::query::Liveness::Ended))
            .map(|v| v.session)
            .collect();
    let released = harbard_core::alias::release_inactive(&mut aliases, &active, now);
    let (alias, changed) =
        match harbard_core::alias::self_name(&mut aliases, &ctx.session, name, now) {
            Ok(result) => result,
            Err(message) => {
                if common.json {
                    println!("{}", serde_json::json!({ "error": message }));
                } else {
                    eprintln!("harbard: {message}");
                }
                return 2;
            }
        };
    if (changed || released > 0)
        && let Err(error) = harbard_core::alias::write(store, &aliases)
    {
        return internal_failure("name", error);
    }
    if changed {
        let event = build_event(&ctx, EventKind::Hello, "", None, now);
        if let Err(error) = append_event(store, &event) {
            return internal_failure("name", error);
        }
        record_supervisor_signal(store, &ctx, now);
        oplog(
            store,
            &harbard_core::oplog::OpEvent::hook(now, "name", &ctx.session, &ctx.harness),
        );
    }
    if common.json {
        println!(
            "{}",
            serde_json::json!({ "name": alias, "changed": changed })
        );
    } else if changed {
        eprintln!(
            "harbard: introduced as {} — this name is fixed for the session",
            harbard_core::alias::display(&alias)
        );
    } else {
        eprintln!(
            "harbard: already introduced as {}",
            harbard_core::alias::display(&alias)
        );
    }
    0
}

/// What `note` adds to a note besides its text: a body read from a file,
/// files to attach and cite, and whether it is a post in a room.
#[derive(Default)]
struct NoteExtra<'a> {
    body_file: Option<&'a std::path::Path>,
    attach: &'a [std::path::PathBuf],
    room: Option<RoomPost>,
}

/// A note written to a room by `channel post`, `intro` or `feedback`. A room
/// keeps what is said in it, so an empty room is no reason to refuse and the
/// receipt talks about the room rather than about who was reached.
#[derive(Clone, Copy)]
struct RoomPost {
    verb: &'static str,
    hint: &'static str,
    /// Written to this machine's store even on a paired satellite.
    local_only: bool,
}

#[allow(clippy::too_many_arguments)]
fn cmd_note(
    store: &Store,
    common: &Common,
    kind: &str,
    text: &str,
    to: Option<&str>,
    note_ref: Option<&str>,
    fields: &[String],
    now: u64,
) -> i32 {
    cmd_note_with(
        store,
        common,
        kind,
        text,
        to,
        note_ref,
        fields,
        &NoteExtra::default(),
        now,
    )
}

/// The bytes of a first line the console and every brief show whole.
const HUMAN_LINE_BYTES: usize = 140;

/// Whether the operator reads this note where it lands: addressed to them,
/// to a channel, or to everyone. A note between two agents is not theirs.
fn operator_reads(to: Option<&harbard_core::Recipient>) -> bool {
    match to {
        None => true,
        Some(t) => match t.kind {
            harbard_core::RecipientKind::Harness => t.value == harbard_core::HARNESS_HUMAN,
            harbard_core::RecipientKind::Channel => true,
            harbard_core::RecipientKind::Session => t.value.starts_with("operator@"),
            _ => false,
        },
    }
}

/// The human-line checks an agent's message gets in the console, said to
/// the author before the note is written: a first line the console cuts,
/// and a line that reads as jargon to the operator.
fn human_line_warnings(text: &str) -> Vec<String> {
    let first = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let sentence = harbard_core::team::parse_label(first).map_or(first.to_string(), |l| l.sentence);
    let mut out = Vec::new();
    if first.len() > HUMAN_LINE_BYTES {
        out.push(format!(
            "harbard: the first line the operator reads is {} bytes; the console and the briefs show about {HUMAN_LINE_BYTES} — keep one sentence there and put the detail in --body-file <path>",
            first.len()
        ));
    }
    let why = harbard_core::human::jargon(&sentence);
    if !why.is_empty() {
        out.push(format!(
            "harbard: the operator reads this first line as jargon ({}) — write it as one sentence in plain language; the detail goes after a blank line or in --body-file",
            why.join(", ")
        ));
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn cmd_note_with(
    store: &Store,
    common: &Common,
    kind: &str,
    text: &str,
    to: Option<&str>,
    note_ref: Option<&str>,
    fields: &[String],
    extra: &NoteExtra<'_>,
    now: u64,
) -> i32 {
    // Composed before anything is written: a half-applied structure would be a
    // note whose shape says one thing and whose body says another.
    let mut body = if fields.is_empty() {
        text.to_string()
    } else {
        match harbard_core::report::compose(text, fields) {
            Ok(body) => body,
            Err(e) => return fail(e),
        }
    };
    if let Some(path) = extra.body_file {
        let detail = match std::fs::read_to_string(path) {
            Ok(d) => d,
            Err(e) => {
                return fail(format!(
                    "--body-file {}: {e}; check the path, or put the detail in the note's text",
                    path.display()
                ));
            }
        };
        let detail = detail.trim_end();
        if !detail.is_empty() {
            body = format!("{}\n\n{detail}", body.trim_end());
        }
    }
    let text = body.as_str();
    let recipient = match to {
        Some(s) => match context::parse_recipient(s) {
            Ok(r) => Some(r),
            Err(e) => return fail(e),
        },
        None => None,
    };
    // A session id is the one recipient kind that was never checked against
    // anything: `harness:` validates, `channel:` normalizes and counts members,
    // and `session:` took whatever it was handed. Delivery matches on exact
    // equality, so one wrong character produced a note addressed to nobody with
    // a completely normal receipt — and `trace` then read "delivered: nobody
    // yet", which sounds like a peer who has not looked yet rather than a peer
    // who does not exist. Observed in this very repo: a warning about an
    // imminent write went to `…-4e04-8a55-…` while the session was `…-4d0d-aa63-…`.
    let recipient = match resolve_session_recipient(store, common, recipient, now) {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    let Some(note_kind) = harbard_core::NoteKind::parse(kind) else {
        return fail(format!("unknown note kind \"{kind}\""));
    };
    // warning → needs a named target; handoff → exactly one live session.
    let to_human = recipient.as_ref().is_some_and(|t| {
        t.kind == harbard_core::RecipientKind::Harness && t.value == harbard_core::HARNESS_HUMAN
    });
    if note_kind == harbard_core::NoteKind::Handoff {
        // Handing work to the operator: one person, who closes it from the
        // console (a "Handover" card), not a session that acks.
        if !to_human
            && let Err(line) =
                ensure_handoff_one_live_session(store, common, recipient.as_ref(), now)
        {
            return fail(line);
        }
    } else if matches!(
        note_kind,
        harbard_core::NoteKind::Warning | harbard_core::NoteKind::Ask
    ) && recipient.is_none()
    {
        // An ask may address a crowd — that is the whole point of it — but not
        // NOBODY: an obligation with no one who owes it is not an obligation,
        // and the author would wait on an answer that was never assigned.
        let c = harbard_core::protocol::correction_broadcast_handoff();
        return fail(c.line());
    }
    // Addressing yourself. Refused at the door rather than recorded and lamented
    // afterwards: Tizne did exactly this at 17:50 answering the operator, saw
    // the reach line say nobody could read it, and re-sent eight seconds later —
    // so the thread carries two near-identical answers to one question. There is
    // no case where a note to your own session is what you meant.
    if let Some(t) = recipient.as_ref()
        && t.kind == harbard_core::event::RecipientKind::Session
    {
        let ctx = Context::resolve(common);
        if t.value == ctx.session {
            return fail(
                "that is your own session — a note to yourself reaches nobody; name the peer you meant, or drop --to to reach everyone",
            );
        }
    }

    // Writing to a session that has ended. Said here, at the moment of writing,
    // because this is the only point where the author can still choose — by the
    // time `trace` reports "delivered: nobody yet" the decision is hours old.
    let ended_notice = recipient
        .as_ref()
        .filter(|t| t.kind == harbard_core::event::RecipientKind::Session)
        .and_then(|t| session_ended_notice(store, common, &t.value, now));

    let built_in_room = recipient.as_ref().is_some_and(|t| {
        t.kind == harbard_core::event::RecipientKind::Channel
            && harbard_core::membership::is_built_in(&t.value)
    });
    if built_in_room && note_kind != harbard_core::NoteKind::Fyi {
        return fail(format!(
            "the built-in rooms are quiet and take only fyi notes, so `--kind {kind}` here would be owed by nobody; address someone instead: harbard note --kind {kind} --to session:<id> \"…\""
        ));
    }
    if let Some(t) = recipient.as_ref()
        && t.kind == harbard_core::event::RecipientKind::Channel
        && !built_in_room
        && extra.room.is_none()
    {
        let m = harbard_core::membership::read(store);
        if !harbard_core::membership::has_members(&m, &t.value) {
            let c = harbard_core::protocol::correction_channel_no_recipients(&t.value);
            return fail(c.line());
        }
    }
    let resolved_ref = match resolve_note_ref(store, note_ref) {
        Ok(r) => r,
        Err(e) => return fail(e),
    };
    // Last, once every refusal above has had its say: copy the files and
    // cite their paths in the body, so the human line stays the first line.
    let mut body = text.to_string();
    if !extra.attach.is_empty() {
        let copied = match attach::attach(store, extra.attach, None) {
            Ok(c) => c,
            Err(e) => return fail(format!("--attach: {e}")),
        };
        let paths: Vec<String> = copied
            .iter()
            .map(|a| a.path.display().to_string())
            .collect();
        let sep = if harbard_core::human::technical_body(&body).is_some() {
            "\n"
        } else {
            "\n\n"
        };
        body = format!("{}{sep}{}", body.trim_end(), paths.join("\n"));
    }
    let text = body.as_str();
    // On stderr even with --json: stdout stays the event.
    // The built-in rooms are the agents' own: nothing there is a report to
    // the operator, so the plain-language checks would only be noise.
    if operator_reads(recipient.as_ref()) && !built_in_room {
        for line in human_line_warnings(text) {
            eprintln!("{line}");
        }
    }
    let ctx = Context::resolve(common);
    if let Err(e) = refuse_across_workspaces(store, common, &ctx, recipient.as_ref(), now) {
        return fail(e);
    }
    let mut event = build_event_kind(&ctx, EventKind::Note, text, recipient, now, note_kind);
    event.note_ref = resolved_ref;
    if let Err(e) = event.to_line() {
        return fail(format!(
            "{e}; shorten it and put the rest in a file with --attach <path>"
        ));
    }
    let verb = extra.room.map_or("note", |room| room.verb);
    let appended = if extra.room.is_some_and(|room| room.local_only) {
        append_local(store, &event)
    } else {
        append_event(store, &event)
    };
    if let Err(e) = appended {
        oplog(
            store,
            &harbard_core::oplog::OpEvent::error(
                now,
                "note",
                &e.to_string(),
                &ctx.session,
                &ctx.harness,
            ),
        );
        return internal_failure(verb, e);
    }
    // Built-in rooms stay out of the snapshot until someone joins one, or
    // they would surface as empty rooms to archive.
    if let Some(t) = event.to.as_ref()
        && t.kind == harbard_core::event::RecipientKind::Channel
        && !built_in_room
    {
        let mut m = harbard_core::membership::read(store);
        harbard_core::membership::touch_note_activity(&mut m, &t.value, now);
        let _ = harbard_core::membership::write(store, &m);
        close_channel_unused(
            store,
            &t.value,
            harbard_core::ReceiptStage::Applied,
            now,
            Some("first note"),
        );
    }
    oplog(
        store,
        &harbard_core::oplog::OpEvent::note_appended(
            now,
            &event.id,
            kind,
            to.map(str::to_string),
            "cli",
            &ctx.session,
            &ctx.harness,
        ),
    );
    learn(
        store,
        &harbard_core::learning::note(
            now,
            &event.id,
            kind,
            to.map(str::to_string),
            "cli",
            &ctx.session,
            &ctx.harness,
        ),
    );
    let delivery = if note_kind.interrupts() {
        "interrupt"
    } else {
        "turn"
    };
    let to_label = event
        .to
        .as_ref()
        .map(|r| format!("{}:{}", kind_label(r), r.value));
    let mut receipt =
        harbard_core::protocol::receipt_note(&event.id, note_kind.as_str(), delivery, to_label);
    if let Some(t) = event.to.as_ref()
        && t.kind == harbard_core::event::RecipientKind::Channel
    {
        let m = harbard_core::membership::read(store);
        let members = m.channels.get(&t.value).map(|v| v.len()).unwrap_or(0);
        let live = count_live_channel_recipients(store, common, &t.value, now);
        if members > 0 && live == 0 {
            receipt.hint =
                "recorded; members>0 but live_recipients=0 — delivery waits for a live subscriber";
        }
    }
    if let Some(room) = extra.room {
        receipt.verb = room.verb;
        receipt.hint = room.hint;
    } else if built_in_room {
        receipt.hint = "kept in the room; harbard channel read shows it";
    }
    emit_write_with_receipt(common, &event, &receipt);
    if let Some(c) = ended_notice {
        eprintln!("{}", c.line());
    }
    // How many readers this note can actually reach, said while the author can
    // still do something about it. Every receipt used to read identically
    // whether the note landed on six sessions or on none, and two thirds of all
    // quiet notes reached nobody — the author never found out, because the only
    // other place that knows is `trace`, hours later, on a note whose recipient
    // is already wrong.
    // A note to the person at the console while their state holds it: the
    // author learns now that it waits for their digest (`harbard_core::hold`).
    if !common.json {
        let state = harbard_core::profile::Profile::load(store.root());
        if let Some(why) = harbard_core::hold::held_receipt(state.state_at(now), &event) {
            eprintln!("harbard: {why}");
        }
    }
    let to_operator = event.to.as_ref().is_some_and(|t| {
        t.kind == harbard_core::RecipientKind::Harness && t.value == harbard_core::HARNESS_HUMAN
    });
    // A view is drawn in the operator's app and nowhere else: a session
    // receives the note's text and a path.
    let view_nobody_sees = !to_operator
        && event.to.is_some()
        && extra.room.is_none()
        && !built_in_room
        && extra
            .attach
            .iter()
            .any(|p| p.extension().is_some_and(|e| e == "huginn"));
    if !common.json && view_nobody_sees {
        eprintln!(
            "harbard: a view is drawn only in the operator's app — this recipient gets the text and a file path; to show it, send it with --to operator"
        );
    }
    if !common.json && to_operator {
        eprintln!("harbard: the operator reads it in the console");
    } else if !common.json && (extra.room.is_some() || built_in_room) {
        // A room keeps its messages, so who is live right now says nothing
        // about whether this will be read.
    } else if !common.json {
        let reach = live_reach(store, common, &event, now);
        if reach == 0 {
            eprintln!(
                "harbard: no live session matches this recipient — nobody will read it unless one starts"
            );
        } else {
            eprintln!("harbard: {reach} live session(s) can read this");
        }
    }
    0
}

/// Live sessions, other than the author, that this event is addressed to.
fn live_reach(store: &Store, common: &Common, event: &Event, now: u64) -> usize {
    let read = store.read(harbard_core::Window::All);
    let views = query::sessions(store, &read.events, now, stale_secs(common));
    let memberships = harbard_core::membership::read(store);
    let placement = harbard_core::workspaces::Placement::load(store);
    views
        .iter()
        .filter(|v| {
            v.session != event.session
                && matches!(v.liveness, harbard_core::query::Liveness::Live)
                && v.harness != harbard_core::HARNESS_HUMAN
        })
        .filter(|v| {
            let channels = harbard_core::membership::channels_for(
                &memberships,
                &v.session,
                &v.harness,
                v.repo.as_deref(),
            );
            let gate = placement.gate_of_session(v);
            event.is_addressed_to_reader(&harbard_core::Reader {
                session: &v.session,
                harness: &v.harness,
                repo: v.repo.as_deref(),
                repo_key: v.repo_key.as_deref().or(v.repo.as_deref()),
                channels: &channels,
                workspace: &gate,
            })
        })
        .count()
}

/// Handoff must name exactly one live `session:<id>` (not repo/harness/channel/
/// broadcast). Quiet or ended recipients can receive FYIs, but nobody can own
/// and close a blocking handoff while they are not live.
fn ensure_handoff_one_live_session(
    store: &Store,
    common: &Common,
    recipient: Option<&harbard_core::Recipient>,
    now: u64,
) -> Result<(), String> {
    use harbard_core::event::RecipientKind;
    let c = harbard_core::protocol::correction_handoff_target_cardinality();
    let Some(to) = recipient else {
        return Err(c.line());
    };
    if to.kind != RecipientKind::Session {
        return Err(c.line());
    }
    let read = store.read(harbard_core::Window::All);
    let views = query::sessions(store, &read.events, now, stale_secs(common));
    let live = views.iter().any(|v| {
        v.session == to.value
            && v.harness != harbard_core::HARNESS_HUMAN
            && v.liveness == harbard_core::Liveness::Live
    });
    if !live {
        return Err(c.line());
    }
    Ok(())
}

fn close_channel_unused(
    store: &Store,
    channel: &str,
    stage: harbard_core::ReceiptStage,
    now: u64,
    evidence: Option<&str>,
) {
    let session = format!("channel:{}", harbard_core::membership::normalize(channel));
    let _ = harbard_core::ResolutionStore::at(store.root()).advance_latest(
        &session,
        harbard_core::protocol::codes::CHANNEL_UNUSED,
        stage,
        now,
        evidence,
    );
}

/// Accept a session recipient only if it names a session the bus has seen.
///
/// Exact id wins. A unique prefix expands to the full id — the same courtesy
/// `trace` already extends to note ids, and the reason it matters is that every
/// surface an agent reads (the brief, `ls`, the console) prints a TRUNCATED id,
/// so the natural copy-paste is a prefix. Zero matches or an ambiguous prefix
/// is a refusal naming the candidates, because silently addressing nobody is
/// the failure this exists to remove.
/// A `recipient_ended` correction when the addressed session is not live, plus
/// the live sessions sharing its checkout so the author has somewhere to go.
///
/// `None` when the session is live, unknown, or is the author's own.
fn session_ended_notice(
    store: &Store,
    common: &Common,
    session: &str,
    now: u64,
) -> Option<harbard_core::protocol::Correction> {
    let read = store.read(harbard_core::Window::All);
    let views = query::sessions(store, &read.events, now, stale_secs(common));
    let target = views.iter().find(|v| v.session == session)?;
    if target.liveness == query::Liveness::Live {
        return None;
    }
    let mut aliases = harbard_core::alias::read_for_naming(store);
    let (name, _) = harbard_core::alias::resolve(&mut aliases, session, now);
    // Worktrees count as the same project: exact path equality would miss a
    // requester in a worktree of the reviewer's checkout, one path segment away.
    // The trailing slash is load-bearing — `/northwind-ci` must not read as a
    // worktree of `/northwind`.
    let same_project = |a: &str, b: &str| {
        a == b || a.starts_with(&format!("{b}/")) || b.starts_with(&format!("{a}/"))
    };
    let target_repo = target.repo.clone();
    let peers: Vec<String> = views
        .iter()
        .filter(|v| {
            v.liveness == query::Liveness::Live
                && v.session != session
                && match (v.repo.as_deref(), target_repo.as_deref()) {
                    (Some(a), Some(b)) => same_project(a, b),
                    _ => false,
                }
        })
        .map(|v| {
            let (peer, _) = harbard_core::alias::resolve(
                &mut harbard_core::alias::read(store),
                &v.session,
                now,
            );
            format!(
                "{} ({})",
                harbard_core::alias::display(&peer),
                &v.session[..v.session.len().min(13)]
            )
        })
        .collect();
    Some(harbard_core::protocol::correction_recipient_ended(
        &harbard_core::alias::display(&name),
        &peers,
    ))
}

/// Refuse a note to a session or repository in another workspace: delivery
/// would stop it at the boundary, and a note nobody can read must not get a
/// normal receipt. The operator's notes are never refused.
fn refuse_across_workspaces(
    store: &Store,
    common: &Common,
    ctx: &Context,
    to: Option<&harbard_core::Recipient>,
    now: u64,
) -> Result<(), String> {
    use harbard_core::RecipientKind;
    let Some(to) = to.filter(|t| matches!(t.kind, RecipientKind::Session | RecipientKind::Repo))
    else {
        return Ok(());
    };
    if ctx.harness == harbard_core::HARNESS_HUMAN {
        return Ok(());
    }
    let placement = harbard_core::workspaces::Placement::load(store);
    if placement.is_single() {
        return Ok(());
    }
    let mine = placement.of_repo(ctx.repo_key.as_deref().or(ctx.repo.as_deref()));
    let theirs = match to.kind {
        RecipientKind::Session => {
            let read = store.read(harbard_core::Window::All);
            let views = query::sessions(store, &read.events, now, stale_secs(common));
            let Some(view) = views.iter().find(|v| v.session == to.value) else {
                return Ok(());
            };
            match placement.gate_of_session(view) {
                harbard_core::workspaces::Gate::Within { workspace, .. } => workspace,
                harbard_core::workspaces::Gate::Open => return Ok(()),
            }
        }
        _ => placement.of_repo(Some(&to.value)),
    };
    if mine == theirs {
        return Ok(());
    }
    let name = |id: &str| {
        placement
            .registry()
            .and_then(|r| r.get(id))
            .map(|w| w.name.clone())
            .unwrap_or_else(|| id.to_string())
    };
    Err(format!(
        "{}:{} works in the workspace \"{}\" and this session in \"{}\"; notes do not cross workspaces, so it would never read this — `harbard workspace where` explains the placement",
        to.kind.as_str(),
        to.value,
        name(&theirs),
        name(&mine)
    ))
}

fn resolve_session_recipient(
    store: &Store,
    common: &Common,
    recipient: Option<harbard_core::Recipient>,
    now: u64,
) -> Result<Option<harbard_core::Recipient>, String> {
    let Some(to) = recipient else {
        return Ok(None);
    };
    if to.kind != harbard_core::event::RecipientKind::Session {
        return Ok(Some(to));
    }
    let read = store.read(harbard_core::Window::All);
    let views = query::sessions(store, &read.events, now, stale_secs(common));
    if views.iter().any(|v| v.session == to.value) {
        return Ok(Some(to));
    }
    let matches: Vec<&str> = views
        .iter()
        .map(|v| v.session.as_str())
        .filter(|s| s.starts_with(&to.value))
        .collect();
    match matches.as_slice() {
        [only] => Ok(Some(harbard_core::Recipient {
            kind: harbard_core::event::RecipientKind::Session,
            value: (*only).to_string(),
        })),
        [] => {
            // Callsigns are current ledger entries, not fuzzy ID substitutes.
            // Count inactive/orphaned entries too: a reused name must never
            // silently choose the live session over another recorded owner.
            let aliases = harbard_core::alias::read(store);
            let owners: Vec<&String> = aliases
                .by_session
                .iter()
                .filter(|(_, alias)| alias.eq_ignore_ascii_case(&to.value))
                .map(|(session, _)| session)
                .collect();
            match owners.as_slice() {
                [session] => {
                    let Some(view) = views.iter().find(|view| view.session == **session) else {
                        return Err(format!(
                            "callsign \"{}\" has no current session on this bus — check `harbard ls`; use a full session id",
                            to.value
                        ));
                    };
                    if view.liveness != harbard_core::Liveness::Live || view.ending {
                        return Err(format!(
                            "callsign \"{}\" is not live or is ending — check `harbard ls`; use the full session id for a historical FYI",
                            to.value
                        ));
                    }
                    Ok(Some(harbard_core::Recipient {
                        kind: harbard_core::event::RecipientKind::Session,
                        value: view.session.clone(),
                    }))
                }
                [] => Err(format!(
                    "no session on this bus starts with \"{}\" and no exact current callsign matches — check `harbard ls`; \
                     a note to an unknown id reaches nobody",
                    to.value
                )),
                many => Err(format!(
                    "ambiguous callsign \"{}\": {} recorded sessions — check `harbard ls` and use the full session id",
                    to.value,
                    many.len()
                )),
            }
        }
        many => Err(format!(
            "\"{}\" matches {} sessions ({}) — use more of the id",
            to.value,
            many.len(),
            many.iter()
                .take(4)
                .map(|s| &s[..s.len().min(13)])
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

fn count_live_channel_recipients(store: &Store, common: &Common, channel: &str, now: u64) -> usize {
    let m = harbard_core::membership::read(store);
    let name = harbard_core::membership::normalize(channel);
    let read = store.read(harbard_core::Window::All);
    let views = query::sessions(store, &read.events, now, stale_secs(common));
    views
        .iter()
        .filter(|v| {
            v.liveness == harbard_core::Liveness::Live
                && v.harness != harbard_core::HARNESS_HUMAN
                && harbard_core::membership::channels_for(
                    &m,
                    &v.session,
                    &v.harness,
                    v.repo.as_deref(),
                )
                .iter()
                .any(|c| c == &name)
        })
        .count()
}

fn kind_label(to: &harbard_core::Recipient) -> &'static str {
    match to.kind {
        harbard_core::RecipientKind::Harness => "harness",
        harbard_core::RecipientKind::Session => "session",
        harbard_core::RecipientKind::Repo => "repo",
        harbard_core::RecipientKind::Channel => "channel",
        _ => "unknown",
    }
}

/// `ack` (F5): the reader tells the author they have it.
///
/// The store already knows an event REACHED a session — that is the delivery
/// cursor. This is the other half, and only the reader can supply it: reaching
/// somebody is not the same as them taking it on.
fn cmd_ack(store: &Store, common: &Common, target: &str, note: Option<&str>, now: u64) -> i32 {
    let ctx = Context::resolve(common);
    let read = store.read(Window::All);
    let channels = my_channels(store, &ctx);
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

    // `last` resolves to the oldest answer this session still owes (session-
    // addressed first). It used to take the newest note addressed here of any
    // kind, so an fyi that landed after a handoff swallowed the ack and the
    // handoff stayed open; with nothing owed it now refuses instead.
    let resolved = if target == "last" {
        match harbard_core::status::owed_notes(&read.events, &reader, now).first() {
            Some(owed) => Some((*owed).clone()),
            None => return fail(nothing_owed(store, &read.events, &ctx, &reader)),
        }
    } else {
        match harbard_core::id::event(&read.events, target, EventKind::Note) {
            Ok(event) => Some(event.clone()),
            Err(harbard_core::id::ResolveIdError::Unknown) => None,
            Err(harbard_core::id::ResolveIdError::Ambiguous(count)) => {
                return fail(format!(
                    "note id \"{target}\" matches {count} notes — use the full 26-character id"
                ));
            }
        }
    };

    let Some(acked) = resolved else {
        return fail(format!(
            "no event with id \"{target}\" in the current window; `harbard inbox` shows what reached you and `harbard owed` what you still owe"
        ));
    };

    if acked.session == ctx.session || !acked.is_addressed_to_reader(&reader) {
        return fail(format!(
            "note {} was not addressed to this session and cannot be acknowledged here; `harbard thread {}` reads it, and `harbard owed` lists what is yours",
            acked.id, acked.id
        ));
    }

    // The acknowledgement goes back to the author, not to the whole bus.
    let to = Some(harbard_core::Recipient {
        kind: harbard_core::RecipientKind::Session,
        value: acked.session.clone(),
    });
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    // An ack with words is an answer, and answers are messages: 73% of acks
    // carried the real reply, which the console rendered as a receipt under
    // the note and nothing else could thread. So the words also land as a
    // `--ref` reply to the acked note. The reply goes first: if it cannot be
    // written, nothing is, and the obligation stays visibly open.
    let reply = match note {
        Some(n) => {
            let mut reply = build_event_kind(
                &ctx,
                EventKind::Note,
                n,
                to.clone(),
                now,
                harbard_core::NoteKind::Fyi,
            );
            reply.note_ref = Some(acked.id.clone());
            if let Err(e) = reply.to_line() {
                return fail(format!("{e}; shorten --note and try again"));
            }
            if let Err(e) = append_event(store, &reply) {
                return internal_failure("ack", e);
            }
            Some(reply)
        }
        None => None,
    };
    // `<id> — <note>` stays the stored text: older readers find the acked note
    // from the text alone. `ref` names it explicitly for everything newer.
    let text = match note {
        Some(n) => format!("{} — {n}", acked.id),
        None => acked.id.clone(),
    };
    let mut event = build_event(&ctx, EventKind::Ack, &text, to, now);
    event.note_ref = Some(acked.id.clone());
    if let Err(e) = event.to_line() {
        return fail(format!("{e}; shorten --note and try again"));
    }
    if let Err(e) = append_event(store, &event) {
        return internal_failure("ack", e);
    }
    learn(
        store,
        &harbard_core::learning::ack(now, &acked.id, &ctx.session, &ctx.harness),
    );
    record_resolution(
        store,
        &ctx.session,
        harbard_core::protocol::codes::ACK_PENDING,
        harbard_core::ReceiptStage::Applied,
        now,
        Some("ack written"),
    );

    let receipt = harbard_core::protocol::receipt_ack(&event.id, &acked.id);
    if common.json {
        let v = serde_json::json!({
            "acked": acked.id,
            "kind": acked.kind.as_str(),
            "to": acked.session,
            "reply": reply.as_ref().map(|r| &r.id),
            "receipt": receipt,
        });
        print_stdout(&serde_json::to_string_pretty(&v).unwrap_or_default());
    } else {
        eprintln!(
            "harbard: acked {} {} to {}{} — {}",
            acked.kind.as_str(),
            short(&acked.id),
            short(&acked.session),
            if reply.is_some() {
                " with a --ref reply"
            } else {
                ""
            },
            receipt.hint
        );
    }
    0
}

/// Why `ack last` refused: nothing is owed. Names the note it would have taken
/// under the old rule, so an agent that meant to answer it learns where the
/// answer goes instead of being left with a bare "no".
fn nothing_owed(
    store: &Store,
    events: &[Event],
    ctx: &Context,
    reader: &harbard_core::Reader<'_>,
) -> String {
    let newest = events.iter().rev().find(|e| {
        e.event == EventKind::Note && e.session != ctx.session && e.is_addressed_to_reader(reader)
    });
    let Some(newest) = newest else {
        return "nothing owed: no handoff or ask addressed to you is waiting for an answer, and no note reached you in the current window".to_string();
    };
    let aliases = harbard_core::alias::read(store);
    let author = harbard_core::alias::get(&aliases, &newest.session)
        .map(|a| harbard_core::alias::display(&a))
        .unwrap_or_else(|| short(&newest.session));
    let clip: String = newest.text.chars().take(80).collect();
    format!(
        "nothing owed: no handoff or ask addressed to you is waiting for an answer, so ack last closes nothing. The newest note that reached you is {} ({} from {author}: \"{clip}\"); it asks nothing of you. To answer it anyway: harbard reply {} \"…\"",
        newest.id,
        newest.kind.as_str(),
        newest.id
    )
}

/// Record a pipeline fact, best-effort: telemetry never breaks the verb it
/// watches.
fn oplog(store: &Store, ev: &harbard_core::oplog::OpEvent) {
    let _ = harbard_core::oplog::append(store, ev);
}

fn learn(store: &Store, ev: &harbard_core::learning::LearningEvent) {
    harbard_core::learning::record(store, ev);
}

fn cmd_export_learning(store: &Store, common: &Common, since: &str, now: u64) -> i32 {
    let secs = match harbard_core::learning::parse_since(since) {
        Ok(s) => s,
        Err(e) => return fail(e),
    };
    let since_ms = now.saturating_sub(secs.saturating_mul(1000));
    let summary = harbard_core::learning::summarize(store, since_ms);
    if common.json {
        print_stdout(&serde_json::to_string_pretty(&summary).unwrap_or_default());
    } else {
        let c = &summary.counters;
        eprintln!(
            "harbard learning · last {since} · {} notes · injected {}{} · tti p50 {} · unacked handoffs {}",
            c.notes,
            c.injected,
            c.injected_pct
                .map(|p| format!(" ({p:.0}%)"))
                .unwrap_or_default(),
            c.time_to_inject_p50_ms
                .map(|ms| format!("{ms}ms"))
                .unwrap_or_else(|| "—".into()),
            c.unacked_handoffs,
        );
        for ev in &summary.events {
            let line = serde_json::to_string(ev).unwrap_or_default();
            println!("{line}");
        }
    }
    0
}

fn cmd_write(
    store: &Store,
    common: &Common,
    kind: EventKind,
    text: &str,
    to: Option<harbard_core::Recipient>,
    now: u64,
) -> i32 {
    cmd_write_with(store, common, kind, text, to, now)
}

fn cmd_write_with(
    store: &Store,
    common: &Common,
    kind: EventKind,
    text: &str,
    to: Option<harbard_core::Recipient>,
    now: u64,
) -> i32 {
    let ctx = Context::resolve(common);
    // Bye is idempotent: Claude/Codex Stop (and double sessionEnd) can fire
    // many times per conversation. A second bye with no intervening write is a
    // no-op so the census is not 10× bye noise.
    if kind == EventKind::Bye {
        let read = store.read(Window::Recent);
        if read
            .events
            .iter()
            .rev()
            .find(|e| e.session == ctx.session)
            .is_some_and(|e| e.event == EventKind::Bye)
        {
            if common.json {
                let v = serde_json::json!({ "bye": "already", "session": ctx.session });
                print_stdout(&serde_json::to_string_pretty(&v).unwrap_or_default());
            }
            return 0;
        }
    }
    let event = build_event(&ctx, kind, text, to, now);
    // Enforce the text/line bound with a typed error before writing.
    if let Err(e) = event.to_line() {
        return fail(format!("{e}; say it in one line"));
    }
    if let Err(e) = append_event(store, &event) {
        return internal_failure(kind.as_str(), e);
    }
    if kind == EventKind::Bye {
        oplog(
            store,
            &harbard_core::oplog::OpEvent::hook(now, "bye", &ctx.session, &ctx.harness),
        );
    }
    match event.event {
        EventKind::Intent => {
            record_resolution(
                store,
                &ctx.session,
                harbard_core::protocol::codes::INTENT_STALE,
                harbard_core::ReceiptStage::Applied,
                now,
                Some("intent written"),
            );
            let receipt = harbard_core::protocol::receipt_intent(&event.id);
            emit_write_with_receipt(common, &event, &receipt);
        }
        EventKind::Hello => {
            let receipt = harbard_core::protocol::receipt_hello(&event.id);
            emit_write_with_receipt(common, &event, &receipt);
        }
        _ => emit_write(common, &event, &ctx),
    }
    0
}

/// M5d — record correction delivery / application (best-effort; never fails verbs).
fn record_resolution(
    store: &Store,
    session: &str,
    code: &str,
    stage: harbard_core::ReceiptStage,
    now_ms: u64,
    evidence: Option<&str>,
) {
    let resolutions = harbard_core::ResolutionStore::at(store.root());
    if stage == harbard_core::ReceiptStage::Delivered {
        let _ = resolutions.deliver(session, code, now_ms, evidence);
    } else {
        let _ = resolutions.advance_latest(session, code, stage, now_ms, evidence);
    }
}

/// Confirm a write: JSON event on stdout with `--json`, else a human line on
/// stderr so hook stdout stays clean for `brief` to own.
fn emit_write(common: &Common, event: &Event, _ctx: &Context) {
    if common.json {
        if let Ok(s) = serde_json::to_string(event) {
            println!("{s}");
        }
    } else {
        let what = match event.event {
            EventKind::Hello => "hello",
            EventKind::Intent => "intent",
            EventKind::Note => "note",
            EventKind::Bye => "bye",
            EventKind::Join => "join",
            EventKind::Leave => "leave",
            EventKind::Ack => "ack",
            EventKind::DeviceJoin => "device_join",
            EventKind::DeviceLeave => "device_leave",
            EventKind::Unknown => "event",
            EventKind::Dependency => "dependency",
            EventKind::Feedback => "feedback",
            EventKind::Rule => "rule",
            EventKind::ChannelLifecycle => "channel_lifecycle",
        };
        eprintln!("harbard: {what} recorded ({})", short(&event.session));
    }
}

/// Event JSON (compat) plus a protocol receipt for teaching agents.
fn emit_write_with_receipt(
    common: &Common,
    event: &Event,
    receipt: &harbard_core::protocol::Receipt,
) {
    if common.json {
        let mut body = serde_json::to_value(event).unwrap_or_else(|_| serde_json::json!({}));
        if let Some(obj) = body.as_object_mut() {
            obj.insert(
                "receipt".into(),
                serde_json::to_value(receipt).unwrap_or_default(),
            );
        }
        print_stdout(&serde_json::to_string(&body).unwrap_or_default());
    } else {
        eprintln!("{}", harbard_core::protocol::receipt_line(receipt));
    }
}

/// `harbard fleet` — the projection, for a reader that is not a browser.
///
/// Retained history, like the served projection: active sessions can span days.
fn cmd_fleet(store: &Store, common: &Common, now: u64) -> i32 {
    let read = store.read(Window::All);
    let leases = harbard_core::LeaseStore::at(store.root()).active_writers(
        harbard_core::SystemLeaseClock,
        &harbard_core::SystemPidChecker,
    );
    let view = harbard_core::fleet::build(&harbard_core::fleet::FleetInputs {
        store,
        events: &read.events,
        now_ms: now,
        stale_secs: stale_secs(common),
        leases: &leases,
        hub_enabled: false,
        devices: harbard_core::DeviceRegistry::load(store).devices.len() as u64,
    });
    if common.json {
        print_stdout(&serde_json::to_string(&view).unwrap_or_else(|_| "{}".into()));
        return 0;
    }
    print_stdout(&render::fleet(&view));
    0
}

fn cmd_ls(store: &Store, common: &Common, all: bool, now: u64) -> i32 {
    let read = store.read(if all { Window::All } else { Window::Recent });
    let mut sessions = query::sessions(store, &read.events, now, stale_secs(common));
    if let Some(filter) = &common.repo {
        // Same two-sided rule as delivery: naming any worktree names the
        // repository, so a filter cannot silently hide the sibling trees.
        let checkout = context::canonical_repo(filter);
        let repository = context::canonical_repo_recipient(filter);
        sessions.retain(|s| {
            let mine =
                |v: Option<&str>| v == Some(checkout.as_str()) || v == Some(repository.as_str());
            mine(s.repo.as_deref()) || mine(s.repo_key.as_deref())
        });
    }
    // Ended sessions are history, not an audience. Left in by default they
    // buried the answer to the only question `ls` is asked — who can I talk to
    // right now — under 76 dead rows carrying their last intent as if current.
    let ended = if all {
        0
    } else {
        let before = sessions.len();
        sessions.retain(|s| !matches!(s.liveness, harbard_core::query::Liveness::Ended));
        before - sessions.len()
    };
    let aliases = harbard_core::alias::read(store);
    let out = render::ls(&sessions, &aliases, now, common.json);
    print_stdout(&out);
    if ended > 0 && !common.json {
        eprintln!("harbard: {ended} ended session(s) hidden — `harbard ls --all` shows them");
    }
    if !common.json {
        for line in lane::brief_lines(store, &Context::resolve(common), now) {
            eprintln!("{line}");
        }
    }
    0
}

fn cmd_inbox(store: &Store, common: &Common, _now: u64) -> i32 {
    let ctx = Context::resolve(common);
    if !ctx.session_is_stable {
        return fail(
            "inbox requires a stable session id: use --session <id> or export HARBARD_SESSION, CLAUDE_CODE_SESSION_ID, CODEX_THREAD_ID, or CURSOR_CONVERSATION_ID from your harness session",
        );
    }
    let read = store.read(Window::All);
    let cursor = harbard_core::cursor::read_inbox_cursor(store, &ctx.session);
    let brief = harbard_core::cursor::read_brief_state(store, &ctx.session).seen;
    let woke = harbard_core::cursor::read_wake_state(store, &ctx.session);
    // Inbox/brief positions mean all addressed notes were delivered; wake.seen
    // is a scan position, so only its explicit delivery receipts may dedupe.
    let start = harbard_core::wake::start_index(
        &read.events,
        &[cursor.as_deref(), brief.as_deref()],
        &ctx.session,
    );
    let placement = harbard_core::workspaces::Placement::load(store);
    let gate = placement.gate(ctx.repo_key.as_deref().or(ctx.repo.as_deref()));
    // The Rooms this session joined, exactly as `brief` resolves them: a
    // built-in Room reaches it only when it joined that Room.
    let channels = my_channels(store, &ctx);
    let reader = harbard_core::Reader {
        session: &ctx.session,
        harness: &ctx.harness,
        repo: ctx.repo.as_deref(),
        repo_key: ctx.repo_key.as_deref().or(ctx.repo.as_deref()),
        channels: &channels,
        workspace: &gate,
    };
    let result = query::inbox(
        &read.events[start.min(read.events.len())..],
        &reader,
        None,
        &woke.delivered,
    );
    if let Some(nc) = &result.new_cursor {
        let _ = harbard_core::cursor::write_inbox_cursor(store, &ctx.session, nc);
    }
    // What an agent reads: no hidden code point in any body.
    let mut result = result;
    for e in &mut result.events {
        e.text = harbard_core::brief::visible_text(&e.text);
    }
    let out = render::inbox(&result, common.json);
    print_stdout(&out);
    0
}

/// The "Context for …" block from `profile.json`, or `None` when the
/// operator typed nothing an agent needs (`harbard_core::profile`).
fn operator_context(store: &Store, now: u64) -> Option<String> {
    // A paired satellite has no profile of its own: its agents read the
    // operator's brief block from the hub (Pairing, Workshop).
    if let Some(remote) = hub::remote_brief(store, now) {
        return remote.block;
    }
    let desk = harbard_core::workspace::DeskState::load(store);
    harbard_core::profile::Profile::load(store.root()).brief_block(&desk.config.operator_name, now)
}

/// Who the rules block says approved them: the profile's name, the desk's,
/// else "the operator".
fn rules_by(store: &Store) -> String {
    let desk = harbard_core::workspace::DeskState::load(store);
    harbard_core::profile::Profile::load(store.root()).shown_name(&desk.config.operator_name)
}

/// Every prompt: the operator's coordination state and its rules, while it
/// is not Available.
fn operator_state(store: &Store, now: u64) -> Option<String> {
    if let Some(remote) = hub::remote_brief(store, now) {
        return remote.prompt;
    }
    let desk = harbard_core::workspace::DeskState::load(store);
    harbard_core::profile::Profile::load(store.root()).prompt_block(&desk.config.operator_name, now)
}

/// What a session reads when the operator's state ends.
fn operator_back_line(store: &Store) -> String {
    let desk = harbard_core::workspace::DeskState::load(store);
    harbard_core::profile::Profile::load(store.root()).back_line(&desk.config.operator_name)
}

/// Rival writers a session remembers having been told about.
const MAX_RIVALS_WARNED: usize = 32;

/// Canonical agent protocol — compact for SessionStart, `--full` on demand.
/// Cursor sessionStart needs the `additional_context` envelope; Claude/Codex
/// SessionStart inject plain stdout.
/// `harbard protocol --subagent`: the rule on who reads a subagent's reports.
/// In the hook, in the envelope SubagentStart injects.
fn cmd_subagent_brief(common: &Common) -> i32 {
    use harbard_core::protocol::SUBAGENT_BRIEF;
    if common.hook.is_some() {
        println!(
            "{}",
            serde_json::json!({
                "hookSpecificOutput": {
                    "hookEventName": "SubagentStart",
                    "additionalContext": SUBAGENT_BRIEF,
                }
            })
        );
    } else {
        println!("{SUBAGENT_BRIEF}");
    }
    0
}

fn cmd_protocol(store: &Store, common: &Common, full: bool, now: u64) -> i32 {
    use harbard_core::protocol::{ProtocolDepth, catalog, render};
    let depth = if full {
        ProtocolDepth::Full
    } else {
        ProtocolDepth::Compact
    };
    // At SessionStart the card is a pointer, not a sheet: the hook pays for
    // one line in every session's context and the grammar stays on demand.
    // Changing the verb rather than the wiring quiets every config that was
    // installed before, with no reinstall.
    // The card carries what the operator typed in "Profile" and the rules
    // the operator approved, bounded and quoted; the pointer carries neither,
    // because the SessionStart brief right after it already does.
    let pointer = common.hook.is_some() && !full;
    let context = (!pointer).then(|| operator_context(store, now)).flatten();
    let rules = harbard_core::rules::load(store).active;
    let text = if pointer {
        harbard_core::protocol::HOOK_POINTER.to_string()
    } else {
        let card = format!(
            "{}\n\n{}",
            render(depth).trim_end(),
            harbard_core::profile::state_card()
        );
        let card = match &context {
            Some(block) => format!("{card}\n\n{block}"),
            None => card,
        };
        match harbard_core::rules::card_block(&rules, &rules_by(store)) {
            Some(block) => format!("{card}\n\n{block}"),
            None => card,
        }
    };
    if common.json {
        let mut card = catalog(depth);
        if let Some(map) = card.as_object_mut() {
            let states: Vec<serde_json::Value> = harbard_core::profile::STATE_CARD
                .iter()
                .map(|(name, rule)| serde_json::json!({ "state": name, "rule": rule }))
                .collect();
            map.insert("operator_states".into(), serde_json::Value::Array(states));
            if let Some(block) = context {
                map.insert("operator_context".into(), serde_json::Value::String(block));
            }
            let texts: Vec<&str> = rules
                .iter()
                .take(harbard_core::rules::MAX_RULES)
                .map(|r| r.text.as_str())
                .collect();
            map.insert("approved_rules".into(), serde_json::json!(texts));
        }
        print_stdout(&serde_json::to_string_pretty(&card).unwrap_or_default());
    } else {
        match common.hook.as_deref() {
            Some("cursor") => print_stdout(&hook_envelope("cursor", &text, "sessionStart")),
            _ => print_stdout(&text),
        }
    }
    0
}

/// `join` / `leave` (F3).
///
/// Two writes, in this order: the durable snapshot (the authority for delivery,
/// because it must outlive the log's retention window) and then a bus event, so
/// the change also reads as part of the conversation. A snapshot write that
/// fails aborts before the event — announcing a subscription that does not
/// exist would be worse than a silent no-op.
fn cmd_membership(
    store: &Store,
    common: &Common,
    channel: &str,
    anchor: &str,
    joining: bool,
    now: u64,
) -> i32 {
    use harbard_core::membership::{self, AnchorKind, Member};

    let name = membership::normalize(channel);
    if !membership::is_valid_name(&name) {
        return fail(format!(
            "invalid channel name \"{channel}\"; use letters, digits, - _ . (max 48)"
        ));
    }
    let ctx = Context::resolve(common);
    let Some(kind) = AnchorKind::parse(anchor) else {
        return fail(format!("unknown anchor \"{anchor}\""));
    };
    let value = match kind {
        AnchorKind::Repo => match ctx.repo.clone() {
            Some(r) => r,
            None => {
                return fail(
                    "no repository here to anchor to; run inside a checkout or pass --as session",
                );
            }
        },
        AnchorKind::Session => ctx.session.clone(),
        AnchorKind::Harness => ctx.harness.clone(),
    };
    let member = Member {
        kind,
        value: value.clone(),
    };

    let mut m = membership::read(store);
    let changed = if joining {
        membership::join(&mut m, &name, member)
    } else {
        membership::leave(&mut m, &name, &member)
    };

    if !changed {
        // Idempotent and quiet: re-joining is not an error, and it must not
        // append a second event that reads as a second person arriving.
        if common.json {
            let v = serde_json::json!({ "channel": name, "changed": false });
            print_stdout(&serde_json::to_string_pretty(&v).unwrap_or_default());
        } else {
            eprintln!(
                "harbard: {} already {} #{name}",
                value,
                if joining { "in" } else { "out of" }
            );
        }
        return 0;
    }

    if let Err(e) = membership::write(store, &m) {
        return internal_failure(if joining { "join" } else { "leave" }, e);
    }
    if !joining {
        let emptied = m
            .channels
            .get(&name)
            .is_none_or(|members| members.is_empty());
        if emptied {
            // Conscious leave that empties the room closes channel_unused.
            close_channel_unused(
                store,
                &name,
                harbard_core::ReceiptStage::Rejected,
                now,
                Some("left membership"),
            );
        }
    }

    let kind_ev = if joining {
        EventKind::Join
    } else {
        EventKind::Leave
    };
    let to = Some(harbard_core::Recipient {
        kind: harbard_core::RecipientKind::Channel,
        value: name.clone(),
    });
    let event = build_event(&ctx, kind_ev, &format!("as {}:{value}", anchor), to, now);
    let _ = store.append(&event); // narrative only; the snapshot already holds

    // A room whose whole membership is one checkout is that checkout. Said
    // here, at the join, because this is the only moment the agent is still
    // deciding: by the first note it has already spent the subscription.
    let alias = if joining {
        membership::repo_alias(&m, &name)
    } else {
        None
    };

    if common.json {
        let v = serde_json::json!({
            "channel": name,
            "changed": true,
            "anchor": anchor,
            "repo_alias": alias,
        });
        print_stdout(&serde_json::to_string_pretty(&v).unwrap_or_default());
    } else {
        eprintln!(
            "harbard: {} #{name} (as {anchor}:{value})",
            if joining { "joined" } else { "left" }
        );
    }
    if let Some(repo) = alias.as_deref() {
        eprintln!(
            "{}",
            harbard_core::protocol::correction_channel_repo_alias(&name, repo).line()
        );
    }
    0
}

/// `channels` / `rooms`: what is visible here, who is in it, and which reach me.
fn cmd_channels(
    store: &Store,
    common: &Common,
    create: Option<&str>,
    archive: Option<&str>,
    unarchive: Option<&str>,
    delete: Option<&str>,
    show_all: bool,
) -> i32 {
    use harbard_core::membership;

    let ctx = Context::resolve(common);
    let mut m = membership::read(store);

    if let Some(name) = create {
        // Orphan create is gone: join creates + subscribes atomically.
        let name = membership::normalize(name);
        return fail(format!(
            "channels --create is retired (orphan rooms are not a happy path); \
use `harbard join {name}` to create and subscribe"
        ));
    }

    if let Some(name) = archive.or(delete)
        && membership::is_built_in(name)
    {
        return fail(format!(
            "#{} is built in and is never archived or deleted; `harbard leave {}` stops it reaching you",
            membership::normalize(name),
            membership::normalize(name)
        ));
    }

    if let Some(name) = archive {
        if let Err(e) = membership::archive(&mut m, name) {
            return fail(e);
        }
        if let Err(e) = membership::write(store, &m) {
            return internal_failure("channels", e);
        }
        let name = membership::normalize(name);
        close_channel_unused(
            store,
            &name,
            harbard_core::ReceiptStage::Applied,
            clock::now_ms(),
            Some("archived when empty"),
        );
        rooms::record_channel_lifecycle(
            store,
            common,
            harbard_core::rooms::LifecycleAction::Archive,
            &name,
            clock::now_ms(),
        );
        eprintln!("harbard: archived #{name} — hidden from the rail; events stay");
        if unarchive.is_none() && delete.is_none() && !common.json {
            return 0;
        }
    }

    if let Some(name) = unarchive {
        if let Err(e) = membership::unarchive(&mut m, name) {
            return fail(e);
        }
        if let Err(e) = membership::write(store, &m) {
            return internal_failure("channels", e);
        }
        let name = membership::normalize(name);
        rooms::record_channel_lifecycle(
            store,
            common,
            harbard_core::rooms::LifecycleAction::Unarchive,
            &name,
            clock::now_ms(),
        );
        eprintln!("harbard: unarchived #{name}");
        if archive.is_none() && delete.is_none() && !common.json {
            return 0;
        }
    }

    if let Some(name) = delete {
        if let Err(e) = membership::delete_archived(&mut m, name) {
            return fail(e);
        }
        if let Err(e) = membership::write(store, &m) {
            return internal_failure("channels", e);
        }
        let name = membership::normalize(name);
        rooms::record_channel_lifecycle(
            store,
            common,
            harbard_core::rooms::LifecycleAction::Delete,
            &name,
            clock::now_ms(),
        );
        eprintln!("harbard: deleted #{name} — historical events stay");
        if archive.is_none() && unarchive.is_none() && !common.json {
            return 0;
        }
    }

    let mine = membership::channels_for(&m, &ctx.session, &ctx.harness, ctx.repo.as_deref());
    let visible =
        membership::visible_rooms_for(&m, &ctx.session, &ctx.harness, ctx.repo.as_deref());
    let visible_names: std::collections::BTreeSet<_> = if show_all {
        m.channels.keys().cloned().collect()
    } else {
        visible.into_iter().collect()
    };

    if common.json {
        let channels: std::collections::BTreeMap<_, _> = m
            .channels
            .iter()
            .filter(|(name, _)| visible_names.contains(*name))
            .map(|(name, members)| (name.clone(), members.clone()))
            .collect();
        let meta: std::collections::BTreeMap<_, _> = m
            .meta
            .iter()
            .filter(|(name, _)| visible_names.contains(*name))
            .map(|(name, meta)| (name.clone(), meta.clone()))
            .collect();
        let v = serde_json::json!({
            "channels": channels,
            "archived": m.archived,
            "meta": meta,
            "mine": mine,
        });
        print_stdout(&serde_json::to_string_pretty(&v).unwrap_or_default());
        return 0;
    }

    if m.channels.is_empty() {
        eprintln!("harbard: no channels yet — `harbard join <name>` creates one");
        return 0;
    }
    for (name, members) in &m.channels {
        if !visible_names.contains(name) {
            continue;
        }
        let archived = membership::is_archived(&m, name);
        if archived && !show_all {
            continue;
        }
        let here = if mine.contains(name) { "*" } else { " " };
        let visibility = m
            .meta
            .get(name)
            .map(|meta| meta.visibility)
            .unwrap_or_default();
        let flag = match (archived, visibility) {
            (true, harbard_core::RoomVisibility::Limited) => " [archived, limited]",
            (true, _) => " [archived]",
            (false, harbard_core::RoomVisibility::Limited) => " [limited]",
            (false, _) => "",
        };
        let who = if members.is_empty() {
            "(empty)".to_string()
        } else {
            members
                .iter()
                .map(|x| format!("{}:{}", x.kind.as_str(), x.value))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let purpose = m
            .meta
            .get(name)
            .and_then(|meta| meta.purpose.as_deref())
            .map(|value| format!(" — {value}"))
            .unwrap_or_default();
        println!("{here} #{name}{flag}  {who}{purpose}");
    }
    if !show_all && !m.archived.is_empty() {
        eprintln!(
            "harbard: {} archived — `harbard channels --all` to list, `--unarchive <name>` to restore",
            m.archived.len()
        );
    }
    0
}

/// `brief --inline`: the hot-path channel (F2).
///
/// Wired on PreToolUse, so it runs before EVERY tool call of every session.
/// Two properties make that affordable: the throttle stamp is read from a
/// tiny per-session file and short-circuits before the store read, and the
/// render is bounded harder than the turn-boundary brief.
///
/// It always advances its own mark on delivery. Re-delivering the same message
/// on every subsequent tool call would be worse than not delivering it: the
/// agent would see the operator repeat themselves forever.
fn cmd_brief_inline(store: &Store, common: &Common, throttle: u64, now: u64) -> i32 {
    let ctx = Context::resolve(common);
    // An in-process Claude Code agent shares its lead's hooks and carries its
    // own `agent_id`. The lead's mail is not its mail, and it reports to the
    // lead natively, so between its tool calls the bus says nothing.
    if ctx.harness == "claude-code" && ctx.parent.is_some() {
        return 0;
    }
    oplog(
        store,
        &harbard_core::oplog::OpEvent::hook(now, "inline", &ctx.session, &ctx.harness),
    );
    let mut state = harbard_core::cursor::read_live_state(store, &ctx.session);
    let supervisor = supervisor_binding(&ctx.harness);

    // A resumed session may keep its id but run under a new process. Replace
    // the old fence before applying the throttle or the new process would look
    // dead until the previous 20-second inline window expired. A recycled PID
    // under a different start marker is a new process too.
    if let Some((pid, start)) = supervisor
        && (state.supervisor_pid != Some(pid) || state.supervisor_start != start)
    {
        state.supervisor_pid = Some(pid);
        state.supervisor_start = start;
        state.checked_ms = now;
        let _ = harbard_core::cursor::write_live_state(store, &ctx.session, &state);
    }

    // Throttle first: this is the whole point of the hot path being cheap.
    if throttle > 0 && now.saturating_sub(state.checked_ms) < throttle.saturating_mul(1000) {
        oplog(
            store,
            &harbard_core::oplog::OpEvent::throttled(
                now,
                now.saturating_sub(state.checked_ms),
                &ctx.session,
                &ctx.harness,
            ),
        );
        if common.json {
            let v = serde_json::json!({ "brief": null, "throttled": true });
            print_stdout(&serde_json::to_string_pretty(&v).unwrap_or_default());
        }
        return 0;
    }
    if ctx.session_is_stable {
        lane::claim_turn(store, &ctx.session, now);
    }

    // Self-heal herd children that only hit PreToolUse (Claude subagents
    // before SubagentStart was wired, or a missed start): announce once when
    // we know the parent and the store has never seen this session.
    if ctx.parent.is_some() {
        let recent = store.read(Window::Recent);
        let unknown = !recent.events.iter().any(|e| e.session == ctx.session);
        if unknown {
            let hello = build_event(&ctx, EventKind::Hello, "", None, now);
            self_heal_hello(store, &hello);
            let mut aliases = harbard_core::alias::read_for_naming(store);
            let (_, fresh) = if let Some(nick) = ctx.nickname.as_deref() {
                harbard_core::alias::claim(&mut aliases, &ctx.session, nick, now)
            } else {
                harbard_core::alias::resolve(&mut aliases, &ctx.session, now)
            };
            if fresh {
                let _ = harbard_core::alias::write(store, &aliases);
            }
        }
    }

    // Resolve incorporation once, then keep the hot path proportional to new
    // bytes. Capture the tail first so an append racing the snapshot is retried.
    let initializing = state.pos.is_none();
    let (read, mut next_pos) = if initializing {
        let (_, boundary) = store.read_since(None);
        (store.read(Window::All), boundary)
    } else {
        store.read_since(state.pos.as_ref())
    };
    let start = if initializing {
        match read
            .events
            .iter()
            .position(|event| event.session == ctx.session)
        {
            Some(joined) => (joined + 1).max(harbard_core::wake::read_index(
                &read.events,
                &[state.seen.as_deref()],
            )),
            None => {
                next_pos = None;
                read.events.len()
            }
        }
    } else {
        0
    };

    // What a prompt's brief, an inbox read or a wake already showed is not
    // news between tool calls: the inline cursor alone does not know about
    // them, and a session that just started using this path would otherwise
    // hear every warning since it joined.
    let brief_seen = harbard_core::cursor::read_brief_state(store, &ctx.session).seen;
    let inbox_seen = harbard_core::cursor::read_inbox_cursor(store, &ctx.session);
    let mut woke = harbard_core::cursor::read_wake_state(store, &ctx.session);
    let shown = [
        brief_seen.as_deref(),
        inbox_seen.as_deref(),
        woke.seen.as_deref(),
    ];
    let fresh: Vec<Event> = read.events[start..]
        .iter()
        .filter(|event| {
            event.event != EventKind::Note
                || !(shown
                    .iter()
                    .any(|mark| harbard_core::wake::at_or_before(&event.id, *mark))
                    || woke.delivered.contains(&event.id))
        })
        .cloned()
        .collect();

    let channels = my_channels(store, &ctx);
    let operator = harbard_core::OperatorProfile::load(store);
    let operator_label = (operator.updated_ms > 0).then_some(operator.display_name.as_str());
    let placement = harbard_core::workspaces::Placement::load(store);
    let gate = placement.gate(ctx.repo_key.as_deref().or(ctx.repo.as_deref()));
    let rendered = harbard_core::brief::render_inline(
        &fresh,
        &ctx.session,
        &ctx.harness,
        ctx.repo.as_deref(),
        ctx.repo_key.as_deref(),
        &channels,
        state.seen.as_deref(),
        operator_label,
        now,
        &gate,
    );

    let mut next = state;
    next.checked_ms = now;
    if let Some((pid, start)) = supervisor {
        next.supervisor_pid = Some(pid);
        next.supervisor_start = start;
    }
    next.pos = next_pos;
    if let Some(seen) = rendered.new_seen.clone() {
        next.seen = Some(seen);
    }
    // Best-effort: a cursor write must never break the tool call it precedes.
    let _ = harbard_core::cursor::write_live_state(store, &ctx.session, &next);

    // Between tool calls the only correction is an owed answer, at most once
    // per STATUS_INTERVAL. Standing conditions (contested checkout, unhealthy
    // hooks) are said once at SessionStart, never per tool call.
    let mut brief_st = harbard_core::cursor::read_brief_state(store, &ctx.session);
    let (out_text, ack_nudged) = {
        let want_ack = harbard_core::status::nudge_due(brief_st.ack_nudge_ms, now) && {
            let reader = harbard_core::Reader {
                session: &ctx.session,
                harness: &ctx.harness,
                repo: ctx.repo.as_deref(),
                repo_key: ctx.repo_key.as_deref().or(ctx.repo.as_deref()),
                channels: &channels,
                workspace: &gate,
            };
            harbard_core::status::should_ack_nudge(
                &store.read(Window::Recent).events,
                &reader,
                brief_st.ack_nudge_ms,
                now,
            )
        };
        let dependency_hint = dependency::brief_hint(store, &ctx.session);
        let mut lines: Vec<&str> = Vec::new();
        if let Some(hint) = dependency_hint.as_deref() {
            lines.push(hint);
        }
        if want_ack {
            lines.push(harbard_core::status::ACK_NUDGE);
        }
        let merged = harbard_core::status::merge_lines(
            rendered.text.clone(),
            &lines,
            harbard_core::brief::MAX_INLINE_BYTES,
        );
        let ack_shipped = want_ack && merged.as_ref().is_some_and(|t| t.contains("ack_pending"));
        (merged, ack_shipped)
    };
    if ack_nudged {
        brief_st.ack_nudge_ms = now;
        record_resolution(
            store,
            &ctx.session,
            harbard_core::protocol::codes::ACK_PENDING,
            harbard_core::ReceiptStage::Delivered,
            now,
            Some("brief/inline"),
        );
        let _ = harbard_core::cursor::write_brief_state(store, &ctx.session, &brief_st);
    }

    // The delivery fact, and the answer to "why hasn't my message landed":
    // either these ids went out through this envelope, or quiet notes are
    // sitting here waiting for the reader's next turn boundary.
    if !rendered.delivered.is_empty() {
        // On the hot path plain stdout is NOT delivery (the harness writes it
        // to a debug log), so a `plain` envelope here marks the black hole.
        let envelope = match common.hook.as_deref() {
            Some(_) => "hook",
            None => "plain",
        };
        oplog(
            store,
            &harbard_core::oplog::OpEvent::injected(
                now,
                "inline",
                rendered.delivered.clone(),
                envelope,
                &ctx.session,
                &ctx.harness,
            ),
        );
        harbard_core::learning::record_injections(
            store,
            now,
            &rendered.delivered,
            envelope,
            "inline",
            &ctx.session,
            &ctx.harness,
        );
        // The watcher armed at this turn's Stop and the next prompt's brief
        // both skip what the wake state lists as delivered.
        if envelope == "hook" {
            woke.delivered.extend(rendered.delivered.iter().cloned());
            let _ = harbard_core::cursor::write_wake_state(store, &ctx.session, &woke);
        }
    } else if rendered.held_quiet > 0 {
        oplog(
            store,
            &harbard_core::oplog::OpEvent::held_quiet(
                now,
                rendered.held_quiet,
                &ctx.session,
                &ctx.harness,
            ),
        );
    }

    // A name on every hook path (F5), not just the turn boundary: a session
    // that only ever crosses the hot path — long autonomous runs, or anything
    // started before aliases existed — must not stay "Codex 225ee6be" to the
    // operator. Best-effort, and after the throttle check so a throttled call
    // costs nothing. Prefer a herd nickname (Codex nick / Claude agent_type)
    // when the session is still unnamed.
    let mut aliases = harbard_core::alias::read_for_naming(store);
    let (_, fresh) = if let Some(nick) = ctx.nickname.as_deref() {
        harbard_core::alias::claim(&mut aliases, &ctx.session, nick, now)
    } else {
        harbard_core::alias::resolve(&mut aliases, &ctx.session, now)
    };
    if fresh {
        let _ = harbard_core::alias::write(store, &aliases);
    }

    if common.json {
        let v = serde_json::json!({ "brief": out_text, "throttled": false });
        print_stdout(&serde_json::to_string_pretty(&v).unwrap_or_default());
    } else if let Some(text) = out_text {
        // On the hot path, plain stdout is NOT delivery: for PreToolUse a
        // harness writes stdout to its debug log and the agent never sees it.
        // Context injection needs the hook envelope, so `--hook` decides the
        // shape. (The turn-boundary `brief` keeps plain stdout, which
        // SessionStart/UserPromptSubmit do inject.)
        match common.hook.as_deref() {
            Some(harness) => print_stdout(&hook_envelope(harness, &text, &hook_event())),
            None => print_stdout(&text),
        }
    }
    0
}

/// Wrap an inline digest in the harness's context-injection envelope. Claude
/// Code and Codex share the `hookSpecificOutput` protocol; Cursor speaks a
/// flat `additional_context` shape instead (the field its postToolUse and
/// sessionStart hooks inject). An unknown harness gets the claude shape rather
/// than silence: a harness that ignores the envelope loses nothing and one
/// that understands it gains delivery.
fn hook_envelope(harness: &str, text: &str, event: &str) -> String {
    let v = if harness == "cursor" {
        serde_json::json!({ "additional_context": text })
    } else {
        serde_json::json!({
            "hookSpecificOutput": {
                "hookEventName": event,
                "additionalContext": text,
            }
        })
    };
    serde_json::to_string(&v).unwrap_or_default()
}

/// The hook event this process answers, as the harness named it on stdin.
/// Claude Code drops `additionalContext` whose `hookEventName` is not the
/// event that ran, so the envelope echoes it. `PreToolUse` when absent, the
/// event the inline brief was first wired to.
fn hook_event() -> String {
    context::hook_stdin()
        .and_then(|payload| payload.get("hook_event_name"))
        .and_then(|v| v.as_str())
        .filter(|v| !v.trim().is_empty())
        .unwrap_or("PreToolUse")
        .to_string()
}

/// The channels this session receives. Read on every hook invocation because
/// a subscription made in another session must reach this one without a
/// restart; the file is small and the read is a single `stat`+parse.
fn my_channels(store: &Store, ctx: &Context) -> Vec<String> {
    harbard_core::membership::channels_for(
        &harbard_core::membership::read(store),
        &ctx.session,
        &ctx.harness,
        ctx.repo.as_deref(),
    )
}

fn cmd_brief(store: &Store, common: &Common, touch: bool, now: u64) -> i32 {
    let ctx = Context::resolve(common);
    oplog(
        store,
        &harbard_core::oplog::OpEvent::hook(now, "turn", &ctx.session, &ctx.harness),
    );
    // A turn starting is the surest sign of life a session gives: a lane
    // promoted to it while it slept starts its ttl here, not at promotion.
    if ctx.session_is_stable {
        lane::claim_turn(store, &ctx.session, now);
    }
    // A long session must retain its original join, questions and answers;
    // the calendar's recent window is presentation, not delivery authority.
    let read = store.read(if touch { Window::All } else { Window::Recent });
    let state = harbard_core::cursor::read_brief_state(store, &ctx.session);
    let woke = harbard_core::cursor::read_wake_state(store, &ctx.session);
    // Only SessionStart may carry historical context. Prompt delivery shares
    // inbox/watch marks and never paginates through pre-join traffic.
    let seen = if touch {
        let inbox = harbard_core::cursor::read_inbox_cursor(store, &ctx.session);
        let start = harbard_core::wake::start_index(
            &read.events,
            &[state.seen.as_deref(), inbox.as_deref()],
            &ctx.session,
        );
        start
            .checked_sub(1)
            .and_then(|i| read.events.get(i))
            .map(|e| e.id.as_str())
    } else {
        state.seen.as_deref()
    };
    let channels = my_channels(store, &ctx);
    let views = query::sessions(store, &read.events, now, stale_secs(common));
    let placement = harbard_core::workspaces::Placement::load(store);
    let gate = placement.gate(ctx.repo_key.as_deref().or(ctx.repo.as_deref()));
    let rendered = harbard_core::brief::render_digest(
        store,
        &read.events,
        &views,
        &ctx.session,
        &ctx.harness,
        ctx.repo.as_deref(),
        ctx.repo_key.as_deref(),
        &channels,
        seen,
        now,
        &woke.delivered,
        &gate,
        &state.rivals_warned,
    );

    // Protocol corrections. Every prompt: an owed answer (throttled) and a
    // dependency result. SessionStart only: standing conditions — held lanes,
    // a contested checkout, unhealthy hooks (one-shot) — said once on the way
    // in; repeating them on every prompt buries the notes that need reading.
    // There is no intent or name reminder: a session that never names itself
    // already carries a derived callsign, and a stale intent asks nothing of
    // the reader.
    let session_start = !touch;
    let reader = harbard_core::Reader {
        session: &ctx.session,
        harness: &ctx.harness,
        repo: ctx.repo.as_deref(),
        repo_key: ctx.repo_key.as_deref().or(ctx.repo.as_deref()),
        channels: &channels,
        workspace: &gate,
    };
    let want_ack =
        harbard_core::status::should_ack_nudge(&read.events, &reader, state.ack_nudge_ms, now);
    let want_checkout = session_start
        && harbard_core::status::should_checkout_nudge(
            &views,
            &ctx.session,
            ctx.repo.as_deref(),
            state.checkout_nudge_ms,
            now,
        );
    // One-shot: once a session has been nudged, never pay for the check again.
    let want_doctor = session_start
        && state.doctor_nudge_ms == 0
        && harbard_core::status::should_doctor_nudge(fleet_unhealthy(store), state.doctor_nudge_ms);
    let dependency_hint = dependency::brief_hint(store, &ctx.session);
    let lane_lines = if session_start {
        lane::brief_lines(store, &ctx, now)
    } else {
        Vec::new()
    };
    // SessionStart only: a newer harbard, from the cached check. A stale
    // cache starts a detached check for the next session; the hook never
    // waits on the network.
    let update_line = if session_start {
        update::nudge_background_check(store, now);
        update::brief_line(store)
    } else {
        None
    };
    let mut nudge_lines: Vec<&str> = Vec::new();
    if let Some(hint) = dependency_hint.as_deref() {
        nudge_lines.push(hint);
    }
    nudge_lines.extend(lane_lines.iter().map(String::as_str));
    if want_checkout {
        nudge_lines.push(harbard_core::status::CHECKOUT_NUDGE);
    }
    if want_ack {
        nudge_lines.push(harbard_core::status::ACK_NUDGE);
    }
    if want_doctor {
        nudge_lines.push(harbard_core::status::DOCTOR_NUDGE);
    }
    if let Some(line) = update_line.as_deref() {
        nudge_lines.push(line);
    }
    let out_text = harbard_core::status::merge_lines(
        rendered.text.clone(),
        &nudge_lines,
        harbard_core::brief::MAX_BYTES,
    );
    let ack_nudged = want_ack && out_text.as_ref().is_some_and(|t| t.contains("ack_pending"));
    // SessionStart: the rules the operator approved, short and in their own bounded
    // block after the digest, so they never crowd out a note.
    let out_text = if session_start {
        let rules = harbard_core::rules::load(store).active;
        match (
            out_text,
            harbard_core::rules::brief_block(&rules, &rules_by(store)),
        ) {
            (Some(t), Some(block)) => Some(format!("{t}\n{block}")),
            (None, Some(block)) => Some(block),
            (t, None) => t,
        }
    } else {
        out_text
    };
    let checkout_nudged = want_checkout
        && out_text
            .as_ref()
            .is_some_and(|t| t.contains("contested_checkout"));
    let doctor_nudged = want_doctor
        && out_text
            .as_ref()
            .is_some_and(|t| t.contains("hooks_unhealthy"));

    // Every other prompt: the operator's state, only when it changed.
    let prompt_state = if touch {
        operator_state(store, now)
    } else {
        None
    };
    let state_news = if touch {
        harbard_core::profile::state_news(
            state.operator_state.as_deref(),
            prompt_state.as_deref(),
            operator_back_line(store),
        )
    } else {
        None
    };

    if touch || ack_nudged || checkout_nudged || doctor_nudged {
        // Self-heal (M1.1b): a session that never announced itself — started
        // before harbard existed, or a `codex exec` headless run whose
        // SessionStart hook never fired — gets a minimal hello on its FIRST
        // touch so it appears on the board. Gated on no prior heartbeat AND no
        // event of ours in the window, so a normal hello+touch never duplicates.
        if touch {
            let unknown = !read.events.iter().any(|e| e.session == ctx.session);
            if unknown && state.hb == 0 {
                let hello = build_event(&ctx, EventKind::Hello, "", None, now);
                self_heal_hello(store, &hello); // best-effort; never breaks a session
            }
            // Name any session that predates F5, on the same self-heal principle
            // as the missing hello above: a session already running when aliases
            // landed should not have to restart to be called something.
            let mut aliases = harbard_core::alias::read_for_naming(store);
            let (_, fresh) = if let Some(nick) = ctx.nickname.as_deref() {
                harbard_core::alias::claim(&mut aliases, &ctx.session, nick, now)
            } else {
                harbard_core::alias::resolve(&mut aliases, &ctx.session, now)
            };
            if fresh {
                let _ = harbard_core::alias::write(store, &aliases);
            }
        }

        let mut next = state;
        if touch {
            if let Some(seen) = rendered.new_seen.clone() {
                next.seen = Some(seen);
            }
            next.hb = now;
            if let Some((pid, start)) = supervisor_binding(&ctx.harness) {
                next.supervisor_pid = Some(pid);
                next.supervisor_start = start;
            }
            if let Some(rival) = rendered.rival.clone() {
                next.rivals_warned.push(rival);
                let excess = next.rivals_warned.len().saturating_sub(MAX_RIVALS_WARNED);
                next.rivals_warned.drain(..excess);
            }
            next.operator_state = prompt_state.clone();
        }
        if ack_nudged {
            next.ack_nudge_ms = now;
            record_resolution(
                store,
                &ctx.session,
                harbard_core::protocol::codes::ACK_PENDING,
                harbard_core::ReceiptStage::Delivered,
                now,
                Some("brief"),
            );
        }
        if checkout_nudged {
            next.checkout_nudge_ms = now;
            record_resolution(
                store,
                &ctx.session,
                harbard_core::protocol::codes::CONTESTED_CHECKOUT,
                harbard_core::ReceiptStage::Delivered,
                now,
                Some("brief"),
            );
        }
        if doctor_nudged {
            next.doctor_nudge_ms = now;
            record_resolution(
                store,
                &ctx.session,
                harbard_core::protocol::codes::HOOKS_UNHEALTHY,
                harbard_core::ReceiptStage::Delivered,
                now,
                Some("brief"),
            );
        }
        let _ = harbard_core::cursor::write_brief_state(store, &ctx.session, &next);
    }
    if !rendered.delivered.is_empty() {
        // At a turn boundary plain stdout IS injected by the harness (Claude,
        // Codex); only Cursor needs the envelope.
        let envelope = if matches!(common.hook.as_deref(), Some("cursor")) {
            "hook"
        } else {
            "stdout"
        };
        oplog(
            store,
            &harbard_core::oplog::OpEvent::injected(
                now,
                "turn",
                rendered.delivered.clone(),
                envelope,
                &ctx.session,
                &ctx.harness,
            ),
        );
        harbard_core::learning::record_injections(
            store,
            now,
            &rendered.delivered,
            envelope,
            "turn",
            &ctx.session,
            &ctx.harness,
        );
    }
    // SessionStart only: what the operator typed about themself in "Profile",
    // after the brief and outside its byte budget, which it never displaces.
    // Every other prompt: the state and its rules, when they changed.
    let block = if session_start {
        operator_context(store, now)
    } else {
        state_news
    };
    let out_text = match (out_text, block) {
        (Some(brief), Some(block)) => Some(format!("{brief}\n{block}")),
        (brief, block) => brief.or(block),
    };
    if common.json {
        let v = serde_json::json!({ "brief": out_text });
        print_stdout(&serde_json::to_string_pretty(&v).unwrap_or_default());
    } else if let Some(text) = out_text {
        // Claude injects hook stdout at the boundary; Cursor does not — its
        // sessionStart reads an `additional_context` field instead. Plain
        // stdout under Cursor would be delivery into a debug log.
        match common.hook.as_deref() {
            Some("cursor") => print_stdout(&hook_envelope("cursor", &text, "sessionStart")),
            _ => print_stdout(&text),
        }
    }
    0
}

fn cmd_setup(store: &Store, common: &Common, dry_run: bool, now: u64) -> i32 {
    // `--harness` is the shared identity flag; here it also names the one
    // harness to wire. It used to be ignored, and setup wired all three.
    let only = match common.harness.as_deref().map(setup::target_of) {
        Some(Ok(target)) => Some(target),
        Some(Err(why)) => return fail(why),
        None => None,
    };
    match setup::run(store, dry_run, only, now) {
        Ok(report) => {
            if common.json {
                print_stdout(&serde_json::to_string_pretty(&report).unwrap_or_default());
            } else {
                print_stdout(&setup::render_text(&report));
            }
            // Absent harnesses are fine; failing a *present* wire is not.
            if report.get("ok").and_then(|v| v.as_bool()).unwrap_or(false) {
                0
            } else {
                1
            }
        }
        Err(e) => fail(e),
    }
}

fn cmd_install(target: &str, dry_run: bool, config: Option<&str>, uninstall: bool) -> i32 {
    if dry_run {
        let v = install::dry_run_json(target);
        print_stdout(&serde_json::to_string_pretty(&v).unwrap_or_default());
        return 0;
    }
    let path = config
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| install::default_config_path(target));
    if uninstall {
        return match install::remove(&path, target) {
            Ok(applied) => {
                if applied.changed {
                    eprintln!(
                        "harbard: removed harbard hooks from {} ({})",
                        applied.path.display(),
                        applied.events.join(", ")
                    );
                    if let Some(bak) = applied.backup {
                        eprintln!("harbard: backup {}", bak.display());
                    }
                } else {
                    eprintln!(
                        "harbard: {} has no harbard hooks (unchanged)",
                        applied.path.display()
                    );
                }
                0
            }
            Err(e) => fail(e),
        };
    }
    match install::apply(&path, target) {
        Ok(applied) => {
            if applied.changed {
                eprintln!(
                    "harbard: wired {} hooks in {}",
                    applied.events.join(", "),
                    applied.path.display()
                );
                if let Some(bak) = applied.backup {
                    eprintln!("harbard: backup {}", bak.display());
                }
            } else {
                eprintln!(
                    "harbard: {} already wired (unchanged)",
                    applied.path.display()
                );
            }
            0
        }
        Err(e) => fail(e),
    }
}

/// Hooks or writer leases look unhealthy — used for the one-shot doctor nudge.
///
/// Asks only the two things the verdict reads. It used to run the whole
/// `query::doctor` — a second full-store read plus channel reconciliation —
/// on every prompt, to look at one field of its report. That maintenance
/// still runs where a doctor report is asked for: `harbard doctor` and the
/// console's `/api/v1/doctor`.
fn fleet_unhealthy(store: &Store) -> bool {
    let leases = harbard_core::LeaseStore::at(store.root()).stats(
        harbard_core::SystemLeaseClock,
        &harbard_core::SystemPidChecker,
    );
    let hooks_ok = install::health()
        .get("healthy")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    !(leases.healthy && hooks_ok)
}

/// Read a conversation back, whole.
///
/// The text form is built to be PASTED — into another model's session, into an
/// issue, into a message. So it carries the whole body of every message, never
/// a preview: the reason to copy a thread is that the 140-byte fragments are
/// what failed you.
fn cmd_thread(store: &Store, common: &Common, id: &str, now: u64) -> i32 {
    let read = store.read(Window::All);
    // Any member id opens the thread, and any event kind: the message in front
    // of the operator is as likely to be an ack or a reply as the question.
    let anchor = match harbard_core::id::event(&read.events, id, EventKind::Note) {
        Ok(e) => e.id.clone(),
        Err(_) => match harbard_core::id::event(&read.events, id, EventKind::Ack) {
            Ok(e) => e.id.clone(),
            Err(harbard_core::id::ResolveIdError::Ambiguous(n)) => {
                return fail(format!(
                    "id \"{id}\" matches {n} events; use the full 26-character id"
                ));
            }
            Err(_) => return fail(format!("no note \"{id}\" in the store")),
        },
    };
    let Some(thread) = harbard_core::thread::thread_for(&read.events, &anchor) else {
        return fail(format!("no note \"{id}\" in the store"));
    };
    let aliases = harbard_core::alias::read(store);
    let name = |session: &str| -> String {
        if session.starts_with("operator@") {
            return "operator".to_string();
        }
        harbard_core::alias::get(&aliases, session)
            .map(|a| harbard_core::alias::display(&a))
            .unwrap_or_else(|| short(session))
    };

    if common.json {
        let messages: Vec<serde_json::Value> = thread
            .events()
            .map(|e| {
                serde_json::json!({
                    "id": e.id,
                    "ts": e.ts,
                    "event": e.event.as_str(),
                    "kind": e.kind.as_str(),
                    "session": e.session,
                    "name": name(&e.session),
                    "harness": e.harness,
                    "to": e.to.as_ref().map(|r| format!("{}:{}", r.kind.as_str(), r.value)),
                    "ref": e.note_ref,
                    // What an agent reads: no hidden code point.
                    "text": harbard_core::brief::visible_text(&e.text),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::json!({
                "thread": thread.root.id,
                "count": thread.len(),
                "messages": messages,
            })
        );
        return 0;
    }

    let mut out = String::new();
    out.push_str(&format!(
        "thread {} — {} message(s)\n",
        short(&thread.root.id),
        thread.len()
    ));
    for e in thread.events() {
        let to =
            e.to.as_ref()
                .map(|r| format!(" → {}:{}", r.kind.as_str(), r.value))
                .unwrap_or_else(|| " → everyone".to_string());
        let verb = if e.event == EventKind::Ack {
            "ack".to_string()
        } else {
            e.kind.as_str().to_string()
        };
        out.push_str(&format!(
            "\n{} · {} · {}{}\n",
            name(&e.session),
            verb,
            clock::relative(now, harbard_core::query::event_ms(e)),
            to
        ));
        // An ack whose words are already above as a reply is only a receipt.
        if e.event == EventKind::Ack && harbard_core::thread::ack_is_echoed(e, &thread.replies) {
            out.push_str("  received\n");
            continue;
        }
        // Whole body, every line indented so the author's own structure
        // survives the paste; hidden code points out.
        for line in harbard_core::brief::visible_text(&e.text).lines() {
            out.push_str(&format!("  {line}\n"));
        }
    }
    print_stdout(&out);
    0
}

fn cmd_search(store: &Store, common: &Common, query: &str, limit: usize, rebuild: bool) -> i32 {
    if rebuild && let Err(e) = harbard_core::fts::rebuild(store) {
        eprintln!("harbard: fts rebuild failed: {e}");
        return 1;
    }
    match harbard_core::fts::search(store, query, limit) {
        Ok(mut hits) => {
            // What an agent reads: no hidden code point, no control.
            for h in &mut hits {
                h.text = harbard_core::brief::visible_text(&h.text);
            }
            if common.json {
                print_stdout(
                    &serde_json::to_string_pretty(&serde_json::json!({
                        "query": query,
                        "hits": hits,
                        "returned": hits.len(),
                    }))
                    .unwrap_or_else(|_| "{}".into()),
                );
            } else if hits.is_empty() {
                print_stdout("no matches");
            } else {
                let mut out = String::new();
                for h in &hits {
                    let who = h
                        .alias
                        .as_deref()
                        .map(|a| a.to_string())
                        .unwrap_or_else(|| short(&h.session));
                    let to = match (&h.to_kind, &h.to_value) {
                        (Some(k), Some(v)) => format!(" → {k}:{v}"),
                        _ => String::new(),
                    };
                    out.push_str(&format!(
                        "{id}  {who} ({harness}){to}\n  {text}\n",
                        id = h.id,
                        harness = h.harness,
                        text = h.text.replace('\n', " "),
                    ));
                }
                print_stdout(out.trim_end());
            }
            0
        }
        Err(harbard_core::fts::FtsError::EmptyQuery) => {
            eprintln!("harbard: search needs at least one 2+ char token");
            2
        }
        Err(e) => {
            eprintln!("harbard: search failed: {e}");
            1
        }
    }
}

fn cmd_doctor(store: &Store, common: &Common) -> i32 {
    let report = query::doctor(
        store,
        common.retention_days.unwrap_or(DEFAULT_RETENTION_DAYS),
    );
    let hooks = install::health();
    let hooks_ok = hooks
        .get("healthy")
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    let fleet = hub_dx::doctor_fleet_json(store);
    let team = team_doctor::format_check();
    let serve = team_doctor::serve_check(store);
    // Advisory: the guard lives outside harbard, so it never fails doctor.
    let rm_guard = team_doctor::rm_guard_check();
    let out = if common.json {
        let mut body = serde_json::to_value(&report).unwrap_or_else(|_| serde_json::json!({}));
        if let Some(obj) = body.as_object_mut() {
            obj.insert("hooks".into(), hooks);
            obj.insert("fleet".into(), fleet);
            obj.insert("team".into(), team.json);
            obj.insert("serve".into(), serve.json);
            obj.insert("rm_guard".into(), rm_guard.json);
            obj.insert("update".into(), update::doctor_json(store));
            obj.insert("archive".into(), archive::doctor_json(store));
        }
        serde_json::to_string_pretty(&body).unwrap_or_else(|_| "{}".to_string())
    } else {
        let mut text = render::doctor(&report, false);
        text.push('\n');
        text.push_str(&hub_dx::doctor_fleet_text(store));
        text.push('\n');
        text.push_str(&serve.text);
        text.push('\n');
        text.push_str(&team.text);
        text.push('\n');
        text.push_str(&rm_guard.text);
        text.push('\n');
        text.push_str(&update::doctor_text(store));
        text.push('\n');
        text.push_str(&archive::doctor_text(store));
        text.push('\n');
        text.push_str(&install::health_text(&hooks));
        text
    };
    print_stdout(&out);
    if report.leases.healthy && hooks_ok && team.ok && serve.ok {
        0
    } else {
        1
    }
}

fn print_stdout(s: &str) {
    if !s.is_empty() {
        println!("{s}");
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
