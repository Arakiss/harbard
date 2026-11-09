# Harbard protocol v1

This directory defines the public coordination contract implemented by
Harbard. The protocol is deliberately smaller than the product: it specifies
identity, addressing, delivery, correction, and writer admission across agent
harnesses. The CLI, HTTP API, MCP adapter, console, and desktop shell are
different transports and views over this contract.

The wire identifier is `harbard/protocol/v1`. The dated directory identifies
this published revision without changing the compatible major version.

## Authority

1. `schema.json` defines the catalog document shape.
2. `harbard-core::protocol::catalog()` emits the live catalog.
3. This document defines normative behavior.
4. Tests bind hook, CLI, HTTP, MCP, and lease behavior to the same invariants.

When prose and executable behavior disagree, the discrepancy is a protocol bug;
neither silently overrides the other.

## Protocol domains

### Identity and presence

- A session MUST have one stable session id and one normalized harness id.
- Liveness expresses reachability from signals or a confirmed supervisor.
  `ls` and `brief` MUST distinguish a confirmed process without a fresh signal
  as `idle`; JSON retains `liveness: live` and adds `idle: true`.
  `last_signal_ms` is the maximum event, heartbeat and hook-check timestamp;
  `ls` displays that signal's age. Neither age nor `idle` proves a hang or wait.
- `ls` MUST read only today's and yesterday's UTC segments unless `--all` is
  requested; delivery and debt queries retain their existing history windows.
- Child agents MAY share a writer domain with their verified parent or stable
  supervisor, but they remain distinct sessions for addressing and receipts.

### Addressing and delivery

- `fyi` MAY target a session, harness, repository, channel, or broadcast.
- `warning` MUST name a target and MAY interrupt matching live recipients.
- `handoff` MUST target exactly one live, non-human `session:<full-id>`.
- Delivery is not acknowledgement. Only the addressed session can close a
  handoff with `ack`.
- Channel membership is durable and checkout-scoped by default. A channel send
  with zero subscribers MUST fail before the note is appended.
- Note text is untrusted data. Renderers MUST keep it inside an injection-safe,
  bounded envelope and MUST NOT execute it as instruction text.

### Writer admission

Writer admission is opt-in. Installed hooks report presence and deliver
messages and never deny a tool call. The rules in this section bind an
integration that opts in through `claim`, `check`, `release`, or `gate`.

- The resource is one canonical Git worktree on one host. Sibling worktrees
  share a Git common directory but MUST have distinct writer resources.
- Proven reads and ambiguous orchestration MUST NOT acquire a writer lease.
- An integration that has opted in MUST acquire or renew the target worktree
  lease before a known checkout mutation runs. A verified foreign holder MUST
  deny the write.
- Target resolution MUST prefer structured file paths, explicit `workdir`/`cwd`,
  and Git `-C` or destructive `git worktree` targets over the caller checkout.
- Different spellings of the same filesystem path MUST canonicalize to one
  resource identity, including paths to files that do not exist yet.
- A single tool call that resolves to multiple worktrees MUST be denied with
  `write_scope_conflict`; v1 does not claim multiple resources atomically.
- Writer renewal and release MUST verify the fencing token, owner session, and
  live supervisor PID. Session end MUST release all writer leases owned by that
  exact execution pair, including target worktrees outside its starting cwd.
- Identity or storage uncertainty MAY fail open only with the explicit
  `lease_unprotected` correction. A known foreign holder never fails open.

### Resource lifecycle

Every protocol domain above hands an agent something it can open: a lease, a
channel, a lane, a session. This section states what happens to those things
afterwards. Agents forget to close what they open, so an implementation cannot
depend on them doing so.

- Closure MUST be derivable from evidence. An implementation MUST NOT require a
  closing verb for a resource to stop being counted as open.
- A writer lease MUST expire on its own clock and MUST verify that its holder
  still exists.
- A channel MUST become dormant on silence alone. Dormancy MUST NOT require an
  empty subscriber list: an anchor is an address, not attendance, and a
  durable anchor would otherwise make the channel immortal.
- A lane's lifecycle receipt is a claim, not a fact. An implementation MUST
  check it against its supervisor's liveness and the lane's contents, and MUST
  distinguish a lane holding work from an empty one, because the safe action
  differs.
- Session liveness MUST be computed from signals and MUST NOT be taken from a
  session's own claim to be working.
- Where a cache and the append-only log disagree, the log is authoritative and
  the cache MUST be repaired rather than trusted.
- A standing correction MUST be withdrawn once evidence contradicts it.

Closure verbs remain in the protocol (`leave`, `release`, `bye`), and an agent
that calls them makes the state converge sooner. Nothing MAY depend on it doing
so.

### Corrections and resolution

- Correction codes are stable machine identifiers shared by hooks, CLI, HTTP,
  MCP, and views.
- Every occurrence has an immutable id. Repeating a previously resolved
  correction creates a new occurrence; it never reopens or overwrites history.
- The allowed lifecycle is `delivered → observed → applied|rejected|expired`.
  Terminal occurrences MUST NOT regress.
- Receipts report what Harbard observed. They MUST NOT claim delivery,
  acknowledgement, application, or liveness without corresponding evidence.

### Secrets and authority

- Notes, acks, intents, traces, and catalogs MUST NOT contain secret values.
- Hub credentials stay server-side and MUST NOT enter browser-prefixed
  environment variables.
- Every CLI-started HTTP server, including loopback, MUST require bearer
  authentication. Loopback origin checks are not operator identity proof.
- The web console MUST add the credential only in its server-side proxy; the
  browser bundle never receives it.
- Operator identity is not an agent role. Agent commands MUST NOT manufacture
  human-authored events.

## Compatibility policy

`harbard/protocol/v1` follows additive compatibility:

`schema.json` is the strict producer-conformance snapshot for this dated
revision. It rejects fields that this revision does not emit, catching drift in
Harbard itself. Consumers MUST NOT use an older dated snapshot to reject a
newer v1 catalog; the wire-version rules below govern consumer compatibility.

- New optional catalog fields, capability ids, rule ids, guarantee ids, and
  correction codes MAY be added within v1.
- A `rules` entry states what an agent MUST do; a `guarantees` entry states
  what the implementation MUST provide. Moving an obligation from the first to
  the second is additive; the reverse is not, and requires v2.
- Clients MUST ignore unknown fields and unknown correction codes while
  preserving them when relaying data.
- Removing or renaming a field/code, changing an existing field type, weakening
  an authorization rule, or changing acknowledgement cardinality requires
  `harbard/protocol/v2`.
- Deprecated aliases MUST remain explicit in `deprecated_brief_aliases`; silent
  semantic renames are forbidden.
- The append-only event schema version and the discoverability protocol version
  are separate. A catalog revision does not rewrite stored history.

## Conformance

A compatible implementation MUST prove at least the following. Items 1 to 3
apply only to an implementation that offers writer admission.

1. read, ambiguous, same-owner write, foreign-owner write, and infrastructure
   failure behavior on every supported harness envelope;
2. target-aware admission for absolute paths, explicit workdirs, `git -C`, and
   destructive worktree commands;
3. owner-bound renew/release and release-all-on-session-end behavior;
4. one-live-session handoff cardinality, channel zero-member refusal, and
   acknowledgement closure;
5. immutable correction occurrences and monotonic terminal resolution;
6. identical protocol version and correction codes through CLI, HTTP, and MCP;
7. that every published guarantee holds without any closing verb being called:
   a lease outlives its holder no longer than its ttl, a silent channel reaches
   dormant, a lane whose supervisor is gone stops reading active, and a
   contradicted correction is withdrawn.

The compact injected card is guidance, not a second specification. It MUST stay
bounded and aligned with this contract and the machine catalog.
