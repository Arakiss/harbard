# Authoritative local lease contract

> **Default hooks do not admit writers.** `setup` and `install` install only
> fail-open presence and delivery hooks. `claim`, `check`, and `release` are
> authoritative opt-in lease operations. `gate` is a compatibility verb for
> explicit callers. The gate material below applies to those callers and does
> not describe default hook behavior.

## Scope and platform

Harbard leases prevent concurrent writers for one checkout and concurrent owners
of one host-global service. They are authoritative state, separate from the
fail-open event bus. Leases support macOS and Linux only: they depend
on Unix advisory locks, process liveness, `O_NOFOLLOW`, ownership and mode checks,
atomic same-directory rename, and directory `fsync`. Unsupported platforms fail
at compile time rather than receiving weaker semantics.

No lease record stores command arguments, environment variables, intent text, or
other free-form payloads. State lives under `~/.harbard/leases/v1/{writer,service}`;
directories use mode `0700` and state, lock, and temporary files use `0600`.

## Commands

```text
harbard gate    [--pid PID]          # explicit callers: classify + ensure writer lease
harbard claim   (--writer | --service NAME) [--ttl-secs N] [--pid PID]
harbard check   (--writer | --service NAME) [--claim-id ULID] [--renew] [--pid PID]
harbard release (--writer | --service NAME) --claim-id ULID [--pid PID]
```

`harbard gate` is the explicit enforcement path for integrations.
It reads the harness hook JSON from stdin, classifies the pending tool call,
and either allows a read with no lease or atomically `ensure`s a writer lease
bound to the stable harness supervisor PID. A rival holder is denied before
the tool runs. `bye` / SessionEnd releases every writer lease owned by the
exact session/supervisor pair, including target worktrees outside its cwd.

Classification follows checkout effects, not binary names alone. Read-only
`sed`, descriptor duplication/discard redirection, opening a local artifact,
and `git worktree list` do not need the checkout lease. `git worktree add` is
also admitted as the prescribed escape from a contested checkout; the new
worktree receives its own writer resource before source edits. Nearby mutating
forms (`sed -i`, redirection to a real file, `git worktree remove`, `git push`)
remain protected.

Admission resolves the worktree actually targeted by a structured file path,
explicit tool `workdir`/`cwd`, Git `-C`, or destructive `git worktree` target.
Paths are canonicalized even when the destination file does not exist yet. A
single call spanning more than one worktree is denied with
`write_scope_conflict`: v1 has no atomic multi-resource claim.

`--writer` identifies a resource by host, Git common directory, and exact
worktree root. It requires a stable `--session` or harness session id.
`--service NAME` identifies a resource by host and the exact global service name;
it does not persist unrelated checkout metadata. `claim` returns a ULID
`claim_id`, which is the fencing token for later renewal and release. For a
writer lease the token is necessary but not sufficient: renewal and release
also require the owner session and live supervisor PID. Service leases retain
capability-token semantics.
`ensure` (used by `gate`) renews in place when the same session already holds
the active lease.

The CLI uses a non-overridable system clock sampled only after the resource lock
has been acquired. `HARBARD_NOW_MS` is a test and demo facility and has no
authority over leases. Writer claims without `--pid` walk the process ancestry
and supervise the first stable Claude Code, Codex, or Cursor process for the
calling context, skipping the one-shot shell that invoked Harbard. If no live
harness ancestor can be resolved safely, claim fails closed rather than binding
an ephemeral tool-shell pid (`$$`). `--pid` remains an explicit numeric
override; service leases never infer one. A supplied or inferred PID must
already be alive. A lease becomes reclaimable when its TTL expires or its PID
is no longer alive.

## Exit and JSON contract

| Command result | Exit | JSON `status` | Token visibility |
|---|---:|---|---|
| Claim acquired | `0` | `acquired` | Full lease, including new `claim_id` |
| Claim held | `2` | `conflict` | Resource and expiry only |
| Check without token | `0` | `free`, `active`, or `stale` | Never reveals a held token |
| Check with matching active token and owner proof | `0` | `active` or `renewed` | Full lease |
| Check with missing, stale, or different token | `2` | `free`, `stale`, or `active` | Never reveals the current token |
| Release matching token and owner proof | `0` | `released` | Released lease |
| Release already-free resource | `0` | `free` | No lease |
| Release different token | `2` | `fenced` | Resource and expiry only |
| Invalid input or storage/corruption failure | `1` | `error` | Error code only |
| Argument-shape rejection by Clap | `2` | Not emitted | Parser diagnostic on stderr |

With `--json`, stdout is a stable JSON object. A gate allow caused by a writer
lease includes both its `claim_id` and canonical `worktree`; read and non-Git
allows emit `null` for both. Every admission audit row carries the same
worktree identity, and the field is mandatory because a row that names no
checkout cannot be attributed. A call denied for spanning several
checkouts records one row per contested worktree. A row blocked by a verified
holder also carries `holder_expires_in_ms`, the holder's remaining lease time
measured against that row's own timestamp, so the log answers "how long was
this checkout busy" without a join against state that has since changed. Human output is one concise line on stdout; diagnostics go to stderr.
Corrupt, mismatched, symlinked, non-regular,
wrong-owner, or incorrectly permissioned authoritative state fails closed and is
never overwritten. `harbard doctor` reports `leases.healthy = false` and exits `1`
when lease state is corrupt or inaccessible.

## Required behaviour

| ID | Priority | Behavior | Evidence |
|---|---|---|---|
| L1 | P0 | Exactly one of 100 concurrent processes acquires one resource. | Shared-home CLI stress test. |
| L2 | P0 | Writer keys distinguish sibling worktrees while sharing repository identity. | Core and CLI tests. |
| L3 | P0 | A conflict is returned before any supervised process can be started. | `claim` exit contract. |
| L4 | P0 | Expired or dead-PID leases can be reclaimed; dead PIDs cannot be newly claimed. | Injected clock/PID tests. |
| L5 | P0 | An old token cannot release a replacement claim. | Core and CLI fencing tests. |
| L6 | P0 | Corrupt or unsafe state fails closed without overwrite or blocking. | Corrupt, symlink, FIFO, ownership/mode tests. |
| L7 | P0 | Resource and Git-head metadata are frozen for future head-change supervision. | Writer lease JSON contract. |
| L8 | P0 | Matching release is idempotent only after the resource is free. | Core and CLI release tests. |
| L14 | P1 | TTL bounds PID reuse and time is injectable below the authoritative CLI boundary. | Clock/PID tests. |
| L15 | P1 | Doctor reports active, stale, corrupt, temporary, and inaccessible lease state. | Core and CLI doctor tests. |
| L16 | P1 | Old same-resource temporary files are collected under the stable lock. | Deterministic temporary-file test. |
| G1 | P0 | First mutating tool acquires; same session renews; rival is denied with no checkout change. | CLI gate integration test. |
| G2 | P0 | Read-only tools and git status/diff/log / search / non-mutating tests never acquire. | `write_class` unit tests + gate CLI. |
| G3 | P0 | Only **Write** acquires the writer lease. Ambiguous never acquires (free or foreign) so Task/opaque scripts cannot brick the worktree; Write still denies rivals. Infra failure on the Write path soft-allows. | `write_class` + gate CLI. |
| G4 | P0 | SessionEnd/`bye` releases the session's writer lease idempotently; dead PID allows reclaim. | Gate CLI + core ensure tests. |
| G5 | P0 | `setup`/`install` removes managed gate entries from Claude Code, Codex, and Cursor; every installed bus hook stays fail-open. | Install fixtures + temp-HOME integration. |
| G6 | P0 | TaskOutput/Monitor/Await are Read; Task/Agent/Workflow are Ambiguous (no lease); Write/Edit/Shell-mutators acquire and deny rivals. | `write_class` + gate CLI. |
| G7 | P0 | Same live supervisor PID shares the writer lease across session ids (parent + herd), so a Task child cannot brick its parent. | Core `ensure` + CLI gate (parent Write → child `git add` Held). |
| G9 | P0 | Cursor allow path always prints `{"permission":"allow"}`; install wrapper fail-opens with the same JSON when `harbard` is missing. | CLI gate + dry-run wrapper simulation. |
| G10 | P0 | Compound/wrapper bypasses fail closed: `echo && git push`, `bash -c 'git push'`, `eval git push` are Write. | `write_class` adversarial + gate CLI. |
| G11 | P0 | `git` / `gh` / `harbard` classify by subcommand (`git log`/`gh run list`/`harbard ls` Read; push/merge/claim Write). | `write_class` unit tests. |
| G12 | P0 | Absolute paths classify by basename: `/usr/bin/git push` is Write (deny under foreign lease); `/usr/bin/git log` is Read. | `write_class` + gate CLI. |
| G13 | P0 | Command substitution cannot hide writes: `echo $(git push)` / `` echo `git push` `` are Write. | `write_class` unit tests. |
| G15 | P0 | Harbard bus verbs (`intent`, `note`, `ack`, `hello`, `bye`, `join`, `leave`) are Read: they touch only `~/.harbard`. `claim`/`release`/`setup`/`install` remain Write. | `write_class` unit tests. |
| G16 | P0 | Under a foreign writer, read-only `sed`, `>/dev/null`, `2>&1`, `open`, `git worktree list`, and the `git worktree add` escape remain available; their mutating neighbors still deny. | Classifier table tests + foreign-lease gate integration test. |
| G17 | P0 | Absolute structured paths, explicit `workdir`, and `git -C` acquire/check the target worktree lease rather than the caller cwd lease. | Gate unit + two-worktree CLI integration tests. |
| G18 | P0 | Nonexistent target files canonicalize through their existing Git root; symlink spelling cannot create a second lease identity. | Context canonicalization regression test. |
| G19 | P0 | A tool call spanning multiple worktrees is denied before mutation with `write_scope_conflict`. | Multi-path apply-patch regression test. |
| G20 | P0 | A leaked writer claim id cannot renew or release without the exact owner session and live supervisor PID; SessionEnd releases every writer lease for that pair. | Core negative ownership tests + CLI stolen-token test. |

## Harness coverage

| Harness | Installed coordination surface | Blocking admission |
|---|---|---|
| Claude Code | Session lifecycle, prompt delivery, and the Stop watcher | None installed |
| Codex | Session lifecycle, prompt delivery, and the Stop watcher | None installed |
| Cursor | Session lifecycle and hook-dependent delivery | No `preToolUse` entry installed |

When the `harbard` binary is absent, SIGKILLed, or the store errors, installed
hook wrappers exit successfully so the host session keeps running.
