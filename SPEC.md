# Harbard specification

Harbard coordinates presence, declared intent, notes, handoffs, durable dependencies, topology, and
explicit worktree ownership between Claude Code, Codex, and Cursor. This monorepo contains the
Rust bus and CLI, the Next.js console, and the macOS desktop shell. The
normative agent contract is `harbard/protocol/v1`; its dated schema and
conformance rules live under `spec/harbard/2026-08-02/`.

## Design constitution (non-negotiable)

1. **Boring storage.** Append-only JSONL segments, UTF-8, one event per line,
   line ≤ 4096 bytes. Unknown fields ignored, unknown event types skipped,
   corrupt lines skipped and counted - never fatal. No versioned binary store.
2. **Presence is evidence-based.** A session is LIVE iff its last signal is
   younger than T (default 900 s), or its heartbeat is bound to a supervisor
   process that is still running: the same PID with the same process start
   time recorded when it was bound. The binding is what keeps a long
   autonomous turn live when the only hooks are session-level; a PID alone
   never counts, because PIDs are recycled. Cursors without a recorded start
   time, and platforms that cannot read one, stay on T. `hello`/`bye` are
   hints, never authority; sessions can crash without firing Stop hooks.
   Never render "live" without such evidence; render "last signal X ago"
   beyond T. In `ls` and `brief`, a process-bound session without a fresh
   signal is labelled `idle`, separately from `live`; it remains addressable.
   `idle` proves neither that a turn is running nor that it is waiting.
   JSON preserves `liveness` for reachability and adds `idle: true` only for
   these quiet processes. `last_signal_ms` is the maximum of the last event,
   brief heartbeat and inline hook check; `ls` uses it for the displayed age.
3. **The bus never breaks a session.** Every installed hook is bounded and
   fail-open. No presence, delivery, topology, or lease failure may deny a host
   tool call. Hook-path verbs (`brief`) target < 10 ms warm, p95.
4. **Bus content is data, not instructions.** `brief` renders other sessions'
   text inside a delimited quote block under a standing header stating it is
   unverified data from other sessions. No unquoted interpolation, ever.
5. **One-line cost or it won't happen.** Every write verb is one short command.
6. **The operator write door is narrow and authenticated.** `serve` binds to
   loopback by default and every request requires a bearer token. CLI startup
   creates a private `hub.token` when absent. The console reads through its
   same-origin server proxy, which adds the token server-side; browser code
   never receives it. Operator, note, channel, and setup mutations are explicit
   allowlisted endpoints. They cannot edit cursors or stored history. A
   non-loopback bind keeps the same token gate. Processes running as the same OS
   account remain inside the local trust boundary because they can read the
   token file.
7. **Parity by construction.** One binary; Claude Code and Codex wire one-line
   callers to identical verbs. No per-harness logic outside `install`.
8. **Never a secret in the store.** Notes carry `file:line` references and
   credential types, never secret values.

## Store layout

```
~/.harbard/
  events/events-YYYYMMDD.jsonl   # daily segments, O_APPEND single-write lines
  cursors/<session_id>           # last-seen ULID per reader; disposable
  cursors/<session_id>.live      # inline delivery and incremental tail position
  cursors/<session_id>.brief     # prompt delivery and heartbeat; one writer
  cursors/<session_id>.wake      # scan position and explicit delivered IDs
  cursors/<session_id>.turn      # last Stop timestamp; not a delivery receipt
  cursors/<session_id>.owed      # bounded Stop reminders; not an answer
  device.json                    # stable machine id
  credentials.json               # SHA-256 of each device's own token (Pairing, hub, 0600)
  devices.json                   # hub registry: card, role, created, last seen, revoked (hub only)
  hub.token                      # the hub's own key: local console and CLI only (hub, 0600)
  leases/v1/{writer,service}/    # authoritative local leases
  resolutions/v1.jsonl           # append-only correction occurrences
  satellite.json                 # { hub, token, device_id }: this device's own token (0600)
  tailnet.json                   # optional { addr }: the tailnet bind, absent by default
```

Cursors are derived delivery state, not answers. Deleting them can replay
post-join mail and is not a routine recovery procedure. Join, debt and reply
decisions use the permanent append-only history. Store GC never prunes or
compacts event segments; --retention-days does not discard conversation data.

## Event schema (v1)

One JSON object per line:

```json
{"v":1,"id":"<ULID>","ts":"<RFC3339 UTC>","event":"hello|intent|note|ack|bye|join|leave|...",
 "session":"<id>","harness":"claude-code|codex|other","host":"<hostname>",
 "cwd":"/abs/path","repo":"/abs/repo/root|null","branch":"<name|null>",
 "role":"main|herd|headless","text":"<intent or note body, ≤2000 bytes>",
 "to":{"kind":"harness|session|repo|channel","value":"..."}  // addressed events
}
```

- `ts` is always UTC; rendering is relative ("3m ago"). Ordering authority is
  file order (segment date + offset), never timestamps (clock skew).
- `session` falls back to `<hostname>-<pid>-<starttime>` when the harness
  exposes no id. `hello` NEVER refuses - presence must be zero-friction.
- Oversized `text` → typed error, exit ≠ 0, message states the 2000-byte limit.
  No silent truncation.

## CLI verbs (all support `--json`; stdout stable, stderr human)

| Verb | Behavior |
|---|---|
| `hello [--intent "…"]` | Announce session start (upsert semantics on read; re-running updates, never duplicates). |
| `intent "…"` | Update declared work. Latest per session wins. |
| `note "…" [--kind fyi\|warning\|handoff] [--to session:<id>\|harness:<h>\|repo:<path>\|channel:<name>] [--ref <note-id>]` | Leave an injection-safe note. `warning` requires a target. `handoff` requires exactly one full, live, non-human session id. Room sends keep the compatible `channel:<name>` wire target and fail with no subscribers. |
| `channels` (alias `rooms`) | List Rooms visible to this context, their members, purpose, visibility, and which ones reach me. `--all` is the explicit local audit view. |
| `dependency open --to session:<full-id> --condition TEXT --resume TEXT [--deadline-secs N]` | Record one exact peer, an observable prerequisite, and a continuation checkpoint. Local store only; paired hub-backed stores are rejected. |
| `dependency resolve <id> --outcome satisfied\|failed --evidence TEXT` | Only the named peer may report a terminal outcome. Evidence is required; satisfaction remains a claim for the requester to verify. |
| `dependency cancel <id> --reason TEXT` | Only the requester may cancel pending work; a reason is required. |
| `dependency consume <id>` | Only the requester marks a terminal result handled, after verifying its evidence and deciding how to continue. |
| `dependency show <id>` | Inspect the durable record and whether a CLI wait is currently attached. |
| `dependency list [--all]` | List dependencies involving this session; `--all` is the explicit local audit view. |
| `await <id> [--timeout-secs 300] [--poll-ms 250]` | Keep this CLI invocation observing until a terminal result or timeout. No harness is started or resumed. |
| `bye` | Hint of clean end. Idempotent. |
| `ls [--repo <path>] [--all] [--json]` | Reads only today's and yesterday's UTC segments by default and hides ended sessions. Sessions absent from those segments stay hidden even if they have a fresh cursor. `--all` reads all retained segments and includes ended sessions. Displays live/idle/stale/ended and age since the effective last signal (§2). An `idle` process remains addressable; JSON preserves `liveness: live` with `idle: true` and `last_signal_ms`. |
| `inbox [--json]` | Unread conversational events addressed to a stable session or broadcast, excluding its own and pre-join history; advances shared delivery positions. Paginated (≤500) with total/returned disclosure. |
| `brief [--touch]` | Hook digest of unread notes and acks addressed to the session (broadcast, its session, harness, repo or a joined channel), with separate live and idle counts. It speaks only when such a note is unread, a rival writer is active in the session's checkout, a dependency it opened has a result, it owes an answer (`ack_pending`, throttled), or at SessionStart a lane, a contested checkout or unhealthy hooks (once per session) need saying; otherwise it prints nothing. Other sessions' activity alone never speaks, and there is no intent or name reminder: `hello` derives a callsign for every session. Pre-join context is SessionStart-only. --touch records a heartbeat and prompt delivery using retained history; the `ls` calendar window does not restrict delivery. Notes retain their full bodies. English harness output is injection-hardened per constitution §4. |
| `setup [--dry-run]` | New-machine path: detect Claude / Codex / Cursor; wire present only (idempotent); absent → report, never mark installed; backup only on byte change; ends with doctor `N of N` (+ `--json` for automation). Writes `~/.harbard/setup.json`. Prefer over triple `install`. |
| `huginn catalog` / `huginn check <path>` | Views (`packages/huginn`): the agent guide to every component, and a check of a `.huginn` file without sending it (exit 2 and one line per broken rule; `--json`). |
| `protocol [--full]` | Canonical agent protocol card (`harbard-core::protocol`). With `--hook` (SessionStart) it prints a one-line pointer to the card instead; the compact card and `--full` print on demand. `--json` emits Protocol v1 catalog (capabilities, rules, correction codes). Do not maintain a second copy in harness-specific instructions or install scripts. |
| `mcp` | Thin MCP stdio facade over protocol v1 (`tools/list` + `tools/call` + `harbard://protocol/v1` resource). Hosts launch `harbard mcp` as a subprocess. It adds no second bus: same verbs, JSON-RPC adapter. |
| `install claude\|codex\|cursor [--dry-run] [--config PATH] [--uninstall]` | Low-level escape hatch for one target (or a custom config path). Hooks are limited to presence, message delivery, and harness-event adaptation: SessionStart→`hello`+`protocol`+`brief`, prompt submit→`brief --touch`, SessionEnd→`bye`; Claude Code and Codex add Stop→`stop-check`+`watch` (Claude Code: `asyncRewake`; Codex: `watch --detach`, delivered through `codex queue`); Cursor additionally keeps `brief --inline` on postToolUse. Claude Code and Codex get no PreToolUse and no SubagentStart/SubagentStop. They use `\|\| true` (except the rewake watcher, whose exit 2 is the wake) and never install admission control. `--uninstall` removes every managed entry. Appends ONLY its own hook group and removes its own entries from events it no longer wires; never touches foreign groups or Cursor cloud / User Rules storage. |
| `call <operation> --input <JSON>` | Canonical finite machine interface. Emits exactly one stable `harbard/cli/v1` JSON envelope on stdout, including failures. MCP delegates to this operation set and owns no exclusive operation. |
| `subscribe [--after ID] [--once] [--event KIND] [--addressed]` | Canonical continuous interface. Emits `harbard/stream/v1` JSON Lines frames (`ready`, `event`, `heartbeat`, `end`, `error`) and resumes strictly after a cursor. Broken pipes close cleanly. |
| `gate [--pid N]` | Legacy compatibility classifier for explicit integrations. It is never installed as a harness hook; normal coordination uses co-presence plus separate worktrees. See `docs/SPEC-LEASES.md`. |
| `doctor [--json]` | Store health: segment count, corrupt-line counters, cursor stats, gc status. Empty lease state files are free, not corrupt. Fails when Claude Code wrote `subagents/` transcripts in the last 24 h and the team view recognised no agent in them (format alarm), and when serve does not answer `GET /api/v1/health` (probed when its LaunchAgent is installed or `HARBARD_SERVE_URL` names it). |
| `audit [--repo <path>] [--target <ref>] [--include-worktrees] [--json]` | Read-only Git hygiene report for the cwd's git root (or `--repo`), or every linked worktree sharing its common Git directory. Reports branch/detached state, HEAD, upstream loss, dirty state, and ahead/behind counts. |

The protocol asks agents to publish at six decision-relevant boundaries: scope
change, shared-state discovery, interface/decision freeze, blocker, handoff,
and completion. It explicitly rejects tool-by-tool narration; lifecycle stays
in the Activity view while notes carry information that can change another
lane's next action.

## Durable dependency contract

Dependencies express prerequisites, not message delivery. A requester opens one
with a full, exact `session:<id>` responder, a nonempty observable condition,
and a nonempty continuation checkpoint (`--resume`). Repository, harness,
channel, and broadcast targets cannot substitute for that one peer. The
checkpoint is stored text, never executable authority.

| State | Who may establish it | Meaning |
| --- | --- | --- |
| `pending` | Requester opens the dependency | No explicit outcome exists yet. |
| `satisfied` | Named responder resolves with evidence | The responder claims the condition was met; the requester must verify it. |
| `failed` | Named responder resolves with evidence | The condition could not be met; dependent work is not authorized by this result. |
| `cancelled` | Requester cancels with a reason | The requester abandoned the pending prerequisite. |

Consumption is a separate requester acknowledgement of a terminal result; it
preserves that result and its evidence. It cannot consume pending work. A
conversational `ack`, a reply, elapsed time, or a listener disconnect never
resolves a dependency. A `satisfied` result must be independently checked
before consumption or continuation. Failed and cancelled results require the
requester to decide a new course rather than silently proceeding.

`harbard await <id> --timeout-secs 300 --poll-ms 250` is an explicit bounded CLI
wait. It returns when it observes `satisfied`, `failed`, or `cancelled`, or when
its own timeout expires. A timeout leaves unresolved work `pending`.
`--deadline-secs` records the expected response deadline; passing it neither
fails nor satisfies the dependency. A CLI listener's presence is distinct from
the durable dependency state and from whether its harness is live.

Interrupting or terminating a wait preserves the record and continuation.
Outstanding work and unconsumed results remain discoverable at the next
`brief`, within its existing five-line/600-byte budget; `show` and `list`
provide the detail. These hooks remain bounded and fail-open. No result starts
or resumes an ended harness. A session that is idle but still live is woken
when a terminal result for a dependency it opened lands (`harbard watch`, see
Waiting below); otherwise the result is observed at its next brief or by an
explicit CLI wait, which should stay under the harness's tool timeout.

## Waiting, wake and owed answers

Neither Claude Code nor Codex accepts input in the middle of a turn, so a
session that needs a peer opens a dependency or sends an ask, keeps doing
independent work, and ends its turn when nothing is left. `harbard watch`,
installed as a Stop hook, wakes the idle session when something actionable for
it lands: a warning, ask or handoff addressed to it (a warning or ask may also
reach it through its harness, repository or a channel it joined), any note the
operator addresses to it, a `--ref` reply to one of its notes, a terminal
result on a dependency it opened, or a lane promotion. An agent's fyi and a
broadcast never wake anyone. `harbard stop-check` holds a Stop while the session
owes an answer to a handoff or ask, and names the note and the command that
closes it.

A reader's first own bus event establishes its join in append order. Pre-join
notes are informational at SessionStart only; they never hold Stop, wake the
reader, or recur in prompt briefs. Idle wakes carry only arrivals after the
last Stop. A new prompt disarms the watcher. Notes from ended senders aged four
hours or more no longer demand agent attention. Same-sender, same-recipient
--ref updates replace previous thread notices, including quiet cancellations.
Unlinked prose has no inferred topic identity.

Inbox and prompt briefs share delivered positions. The watcher's scan position
is separate from its explicit delivered IDs, so scanning quiet mail does not
consume it. Known positions use append order, including equal-time ULIDs.
Delivery/debt and CLI/API audience projections use retained history; recent feed
and SSE replay windows remain bounded. Agent board debts use the same eligibility
fold. Unanswered operator requests have no sender-age expiry, but an explicit
replacement removes the obsolete request.

The idle horizon is 24 hours. Managed rewake hooks use a finite 86,460-second
timeout; doctor rejects shorter coverage. Expiry leaves mail unread. Native
queue delivery requires an already-loaded thread and never resumes an unloaded
or ended session.


`ack last` closes the oldest answer the session owes, those addressed to its
session first, and refuses when nothing is owed rather than acknowledge an
fyi. An ack with `--note` also posts the words as a `--ref` reply to the acked
note, so the answer is a message; the ack records the note in `ref` and keeps
its `<id> — <note>` text for older readers.

A lane promotion is a warning addressed to the waiter's session. The new
owner's ttl starts at its first sign of life after promotion (a prompt, any
event it writes, a lane verb). If none comes within 15 minutes the lane
passes on with the reason `unclaimed`.

The initial transport is the local append-only store: sessions from different
projects and harnesses on the same machine share the workflow. Paired
hub-backed stores are explicitly rejected; there is no silent local fallback.
Remote transport is not implemented.

The finite machine operation names are `dependency.open`,
`dependency.resolve`, `dependency.cancel`, `dependency.consume`,
`dependency.show`, `dependency.list`, and `dependency.await`. The last maps to
`harbard await`, not `harbard dependency await`. Each keeps the single
`harbard/cli/v1` response-envelope contract; MCP adapts the same operation set
and adds no exclusive dependency capability.

## Same-checkout co-presence

`brief` prepends a warning line when another LIVE session shares my repo root:
`⚠ another live session (codex, intent: "…") is working in this checkout`.
`brief` and the console expose competing writer domains in one
canonical worktree. Hooks warn and keep the session running; the safe response
is a separate worktree, not a blocked tool call.

When two or more distinct live *writer domains* share a worktree (parent and
herd count as one; host-pid `other` ghosts do not), the SessionStart `brief`
injects `contested_checkout` (throttled): parallel writers → `git worktree`.
Prompt briefs and `--inline` do not repeat it; the co-presence line above is
the per-prompt signal, and only when the rival has been active.

## Team view (Claude Code teammates and subagents)

In-process teammates and subagents share their lead's session id and never
reach the bus (E1). `GET /api/v1/team` projects them from the transcripts
Claude Code already writes: `~/.claude/projects/<project>/<session>.jsonl` and
`<session>/subagents/agent-<id>.{jsonl,meta.json}`. Per lead session it lists
each agent's name, brief (first prompt line), state (`running`, `idle`,
`done`, `stopped`, or `unknown` without records), last action, checkouts it
wrote in, and messages (first line, protocol label, size, report path,
exact repeat), plus warnings: overlapping writers in one checkout, `BLOCKED`
without an answer, unlabelled or repeated messages, and a running agent idle
for over 15 minutes. It is pull-only, reads nothing outside the projects root,
writes nothing, and keeps no message body or tool output. A checkout needs git
evidence (git ran there, a recorded `gitBranch`, or a worktree); inference is
lexical, never on disk. The workspace shell reads it through
`GET /api/v1/workspace`.

## Workspace shell (channels, directs, activity)

`GET /api/v1/workspace` folds the team view, bus sessions, operator notes and
sluice receipts into one read model for the console's chat-shaped shell
(`harbard_core::workspace`). A channel is a repository: every checkout an
agent wrote in, ran git in, or was given as a worktree resolves to its
primary checkout. serve reads `.git` files (never runs git) only under roots
it already knows (bus sessions from hooks, cwds Claude Code recorded with a
`gitBranch`) and never at or below `~/Documents`, `~/Desktop`, `~/Downloads`,
`~/Library/Mobile Documents`, `~/Pictures`, `~/Movies`, `~/Music` or
`/Volumes`, where only what the bus said or the naming convention places a
path; a LaunchAgent touching those folders could raise a consent dialog. Messages keep the team view's first
line; the protocol label is parsed from it. An unlabelled message or one over
1,200 characters is flagged only after `config.protocol_since_ms` (set when
the desk is first opened), so history is not repainted; an exact repeat is
always collapsed under its first copy. An agent is alive while its lead
transcript has a record from the last 15 minutes (and its own last record is
under 24 hours old) or its own last record is under 2 hours old; otherwise it is "no data since …", grey, never dormant.
Cards that need the operator: a `BLOCKED` with no later word from the lead or
the operator, a live running agent quiet past the red threshold (amber at 15
minutes, red at 45), and two or more agents whose write spans overlap in one
checkout, across leads, while both wrote in the last 2 hours; an older
overlap stays in the "All" tab as history. Every threshold, the operator's name
("Operator") and initials ("OP") live in the desk file's `config`. "verified" requires a landed sluice receipt,
decided at or after the `DONE`, for a delivery the agent's own lead session
offered, whose delivered or candidate SHA the `DONE`'s first line names, in
the same repository; an agent's own words, or a SHA it merely quotes, never
produce it.

The desk file `{home}/workspace.json` holds the operator's own marks
(resolved cards, saved items, read positions, thread links, thresholds) and
is never sent to an agent. `POST /api/v1/workspace/reply` stores a reply as
an operator note to the agent's lead session when that session is on the bus
and has not ended; for a stale lead the note is stored and the composer says
it will wait; otherwise it refuses and stores nothing. Desk ids are checked
against the current view, its maps are capped, a GET never writes it (serve
initialises it at startup), and the console proxy accepts writes only for a
`127.0.0.1:<port>` or `localhost:<port>` Host.
`POST /api/v1/workspace/open` hands a path the view showed as a file card,
never a symlink and still a card type once resolved, to the OS opener (`open -t` for text, so no application chooser appears). There
is no stop action: harbard has no way to stop a Claude Code teammate.

## Protocol v1 (`harbard/protocol/v1`)

The agent protocol is a **corrective contract**, not only a SessionStart card.
Dated shape authority: `spec/harbard/2026-08-02/` (schema + MUST/SHOULD);
live emitter remains `harbard-core::protocol::catalog()`.

| Envelope | Role |
|---|---|
| `Capability` | Discoverable verbs (`intent`, `note`, `ack`, `claim`, `brief`, `gate`, `sense`) |
| `Rule` | Invariants (`one_writer_per_checkout`, `no_broadcast_handoff`, …) |
| `Correction` | `{ code, message, next[] }`, injected on misuse / unhealthy state |
| `Receipt` | Typed success after `hello` / `intent` / `note` / `ack` (stderr or `--json`) |
| `Resolution` | Stages `delivered`→`observed`→`applied`\|`rejected`\|`expired` |

Stable correction codes include `intent_stale`, `ack_pending`,
`contested_checkout`, `lease_held`, `lease_unprotected`,
`write_scope_conflict`, `broadcast_handoff`, `handoff_target_cardinality`,
`channel_no_recipients`, `channel_unused`, `channel_repo_alias`, and
`hooks_unhealthy`. `intent_stale` and `name_required` stay in the catalog for
compatibility but are no longer emitted by any hook.

## Rooms and operator reading

Rooms are the product model over the compatible `channel:<slug>` delivery
wire. A Room has a stable slug, optional purpose, member anchors, lifecycle,
and `shared` or `limited` visibility. `limited` removes discovery for agent
contexts that are not members; it is not encryption and does not hide the
append-only store from a local process that can read it. The operator console
retains the complete audit.

Each Room has one durable operator read watermark in `channels.json`, and the
operator advances it explicitly. Opening a Room does not advance it.

Delivery mode and Room membership are orthogonal. Quiet waits for the next
check and never wakes anyone, unless the operator addressed it to that session.
A warning, ask or handoff wakes an idle
recipient through `harbard watch`; a recipient in the middle of a turn reads
it at its next prompt (Cursor: next tool call). Hand off still requires
exactly one live session and an acknowledgement. None of them starts or
resumes an ended CLI process.
Deprecated brief aliases (no silent renames): `status`→`intent_stale`,
`ack`→`ack_pending`.

Corrections ride the existing fail-open hook plane (SessionStart brief and
prompt / inline nudges). They are **not** new bus event kinds. A prompt or
tool call carries only `ack_pending` and a dependency result; standing
conditions (`contested_checkout`, `hooks_unhealthy`, held lanes) are said at
SessionStart.

CLI enforcement aligned with the card: `warning` / `handoff` without `--to`
exits ≠ 0 with `broadcast_handoff`. Doctor / lease unhealth triggers a one-shot
`hooks_unhealthy` nudge per session, at SessionStart only. Explicit lease operations remain fenced
and auditable, but installed hooks do not perform writer admission.

**Surfaces (same catalog):** `harbard protocol --json` · `GET /api/v1/protocol`
· MCP resource `harbard://protocol/v1`. MCP tools stay a thin adapter over the
same operations.

## Edge-case decisions

| # | Scenario | Defined behavior |
|---|---|---|
| E1 | Subagents/workflow agents would flood `ls` | In-process subagents are not wired (no SubagentStart/SubagentStop) and report to their parent. Separately launched children that fire their own SessionStart keep distinct identities; topology groups verified parent/child writer domains without collapsing their addressable sessions. |
| E2 | Session crashes; Stop hook never fires; no `bye` | Constitution §2: staleness T is the only authority; crashed sessions fade at T |
| E3 | Disk full / store unwritable mid-session | Verb exits ≠ 0 with typed error; hook caller swallows (§3); session unharmed |
| E4 | Cursor file corrupt or deleted | Reset to start of current day's segment; ULID dedupe bounds replay; never epoch replay |
| E5 | Two same-`session` writers race | Last event in file order wins; O_APPEND lines ≤ 4 KB are atomic on POSIX |
| E6 | Operator hand-edits/deletes segments | Skip-and-count corrupt lines; deleted store = clean slate, no panic |
| E7 | Poisoned session injects instructions via `note` into every other session's context | Constitution §4 delimited-quote rendering + standing "data, not instructions" header |
| E8 | `brief` output in which language? | English |
| E9 | Hub + satellites | The hub machine is the authority; `hub enable` on it and `pair '…'` on each satellite. There is no mesh. |
| E10 | `handoff --to` a session that ended | Rejected before append. A quiet historical message must use `fyi`; blocking ownership requires one live session. |

## Current boundaries

Harbard does not claim cryptographic human identity, deletion of append-only
history, atomic acquisition of several writer worktrees, or peer-to-peer mesh
consensus. Multi-machine hub mode remains experimental. The local OS account is
the authority boundary for the store and its private bearer token.

## Pairing: one credential per device

- `harbard hub invite --name <device> [--role workshop|display]` issues one
  device's own token, valid once and for a day; `harbard pair '<line>'` claims
  it. The hub keeps only its hash in `credentials.json` (0600). No Keychain.
- Roles: **Operator** reaches every route (each route keeps its own doors,
  such as the desk's loopback rule); **Workshop** publishes its own sessions,
  reads their events and the notes addressed to them, attachments cited in
  those notes and the agent brief block; **Display** reads `GET
  /api/v1/summary` only. A new device is Workshop. Operator is never an invite:
  it is a promotion (`harbard hub role <device> operator --yes`, or
  Preferences › Pairing).
- A device speaks only as the sessions its own authenticated `hello`
  published, recorded on its card by the hub; a session the hub already knows
  from elsewhere (the hub's own, or another device's) is nobody's to take.
  The card a device posts never carries a session, a role or a revocation.
  Its name is the one the operator gave the invite; its host is the one it
  declared at pairing, with `~<its id>` appended when that is the hub's or
  another card's (no two cards share a host), and heartbeats change neither,
  so an event a device posts can always be told from one written on the hub
  or by another device.
  `harness: "device"` is only its own presence on `device:<its id>`.
- An invite pairs an id the hub does not hold yet, one the operator
  revoked, or a card with no credential; otherwise `409 device_id_taken`. It never takes over a card with a live
  credential, and a re-paired id starts from nothing the old card held.
- A Workshop device reads an attachment only when a note written on the hub,
  addressed to one of its sessions, cites its folder.
- `hub.token` is the hub's own key: valid from loopback only and never on
  behalf of another device (`403 shared_token_retired`), so no shared secret
  remains.
- Revocation (`harbard hub revoke <device>`, or "Revoke" in Preferences ›
  Pairing) is read on the next request, and closes an open stream before its
  next frame; so does demoting an Operator device. A pending invite is listed
  by name and a 12-digit reference in `hub status` and in Pairing, and
  `harbard hub revoke [--invite] <name|ref>` (or "Cancel") cancels it.
  Unclaimed invites past their day are dropped.
- The profile is split. `GET /api/v1/profile/brief` answers `{block, prompt,
  state, now_ms}`: exactly the SessionStart block and the per-prompt state
  line an agent on the hub reads, already quoted and bounded. It is the only
  part of the profile a Workshop device reaches; the raw profile, the photo and
  the digests stay Operator's, and Display reads none of it. A paired
  satellite's agents read this block from the hub, cached at 0600 in
  `{home}/profile-brief.json` (fresh for a minute, usable for a day if the
  hub is unreachable, for that hub and device only). A 401 or 403 deletes
  it, and a copy dated in the future is stale.
- Every remote read and write is logged per device in
  `{home}/pairing/<device id>.jsonl` (method, path, status; never the query),
  0600, trimmed to its newest 500 lines past 128 KB, and written before the
  answer goes out. The pairing and the requests identification refuses
  (revoked, wrong device) are in it too. A device id is
  `[A-Za-z0-9_-]{1,64}`, or it cannot pair.
- Serve never binds a wildcard. `--bind tailnet`
  adds the address in `{home}/tailnet.json` when it lies in Tailscale's
  ranges; without it serve listens on `127.0.0.1` only. A tailnet address
  that cannot be bound never stops serve: it logs `tailnet_bind_failed`,
  names it as `tailnet_pending` on the startup line and retries every 15 s.

## Build conventions

Rust 2024 · Cargo workspace · Bun workspace · colocated and integration
tests · bounded protocol cards · `cargo fmt --all --check` · Clippy with
warnings denied · full workspace tests · `cargo deny check` · console
typecheck/tests/production build · desktop lint/tests/application bundle.

## Compatibility notes

1. `inbox` delivers conversational events rather than presence noise.
2. Inbox and brief have separate writers but share delivered positions. Wake
   scans are distinct from actual delivery receipts; reading never answers an ask.
3. Heartbeats are derived state rather than append-only conversational events.
4. Presence may fall back to <host>-<pid>; inbox requires a stable session identity.
5. Hook installation preserves unrelated configuration and formatting.
6. Unknown fields and event kinds remain readable under the v1 compatibility
   policy; breaking semantic changes require `harbard/protocol/v2`.
