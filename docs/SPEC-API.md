# HTTP API

`harbard serve` exposes a local HTTP API over the store. The web console, the
desktop app, and paired devices use it. Other routes are specified in
[SPEC-FLEET-VIEW.md](SPEC-FLEET-VIEW.md), [SPEC-UPDATES.md](SPEC-UPDATES.md),
[SPEC-WORKSPACES.md](SPEC-WORKSPACES.md), and the workspace section of
[SPEC.md](../SPEC.md).

The constitution in `SPEC.md` applies, and three rules bind the API:

- **Writes are explicit.** Every mutation is an allowlisted `POST` (constitution
  §6). No route can edit cursors or stored history.
- **Clients never consume agent cursors.** Read models are computed per request
  or stream; `cursors/` belongs to agents only.
- **Bus content is data.** Every string from the store renders as inert text in
  the UI (React escaping, never `dangerouslySetInnerHTML`).

## Conventions

- Bind `127.0.0.1` by default. Default port `7419`, `--port` to override.
  Every request requires `Authorization: Bearer <token>` (or
  `X-Harbard-Token`); CLI startup generates `~/.harbard/hub.token` with mode
  `0600` when no token is configured. That key is the hub's own: loopback
  only, never on behalf of a device. Paired devices present their own token
  and reach what their pairing role allows (`SPEC.md`, "Pairing"). Serve never
  binds a wildcard: `--bind tailnet` adds the configured tailnet address.
  Prefer `harbard hub invite` and `harbard pair '…'` over hand-rolled
  environment variables.
- CORS: allow origins `http://localhost:*`, `http://127.0.0.1:*`, and portless
  named hosts (`http(s)://{name}.localhost[:port]`, `http(s)://{name}.local[:port]`,
  including worktree subdomains). With a hub token, also allow private IPv4
  origins (`10.*`, `172.16–31.*`, `192.168.*`) for paired consoles. Methods:
  `GET`/`HEAD`/`POST`.
- All responses are `application/json; charset=utf-8` unless noted.
- The versioned base path is `/api/v1`. Unknown paths return 404 JSON `{error}`.
- Lifecycle and failure logs are structured and timestamped; routine GETs are
  not logged. Query values and credentials are never logged.
- Graceful shutdown on SIGINT/SIGTERM. `--json` prints a startup line on stdout
  (`{port, pid, store, bind}`) so tools can discover it.

## Routes

| Endpoint | Returns |
|---|---|
| `GET /api/v1/sessions` | `{sessions: SessionView[], now_ms}`: every known session in the window (today+yesterday segments), newest signal first |
| `GET /api/v1/events?after=<ulid>&limit=<n≤500>` | `{events: Event[], total, returned, newest_id}`: file order, Recent window, with disclosure |
| `GET /api/v1/snapshot` | One coherent live projection: sessions, latest events, channels, aliases, delivery cursors, and active leases, all derived from one in-memory event cache |
| `GET /api/v1/history?before=&snapshot=&limit≤500&kinds=&harness=&channel=` | Archive page, newest-first; bounded reverse segment walk; `{events, oldest_id, newest_id, has_more, returned, snapshot_newest_id, retention_days}`. Unknown `before`/`snapshot` returns **409** `cursor_expired`. |
| `GET /api/v1/history/sessions` | Sessions over all segments (`window=all`). Rewrites `ended` to `stale`. |
| `GET /api/v1/history/checkouts` | Ledger per tree (`window=all`): occupants, epitaph on bye, last marks, last sluice receipt. Keeps real `ended`. |
| `GET /api/v1/notes/{id}/trace` | Same reconstruction as `harbard trace --note --json`. 404 unknown, 409 ambiguous prefix. |
| `GET /api/v1/leases` | `{leases: WriterLeaseView[], now_ms}`: the active writer leases (who owns which checkout), most recently renewed first. Read-only. |
| `GET /api/v1/channels?inactive_days=` | Membership snapshot + `candidates[]` (archive eligibility) + `repo_alias[]` (`[name, repo]` rooms that deliver what `--to repo:` already delivers) + `meta` |
| `POST /api/v1/channels` `archive_bulk` | `{names: string[]}` → per-channel `archived\|skipped\|conflict`; one membership write |
| `GET /api/v1/stream?after=<ulid>` | SSE (`text/event-stream`): one shared incremental store tailer fans out bounded per-client queues; reconnect resumes after an event id; heartbeat comment every 15 s |
| `GET /api/v1/health` | Process and stream health: uptime, RSS, threads, file descriptors, incremental-read latency, active streams, slow-client drops, retries, and old handoffs |
| `GET /api/v1/doctor` | Store health: segment count, corrupt/unknown counters, gc state, store path; the CLI also rolls up hooks |
| `GET /api/v1/setup` | First run: `needs_setup`, harness detect/health, `setup.json` state |
| `POST /api/v1/setup/install` | Loopback only: wire selected targets |
| `POST /api/v1/setup/complete` | Loopback only: persist wizard completion (`setup.json`) |
| `GET /api/v1/meta` | `{name, version, host, started_ms, hub: bool}` |
| `GET /api/v1/devices` | `{devices: DeviceCard[], now_ms}`: hub device registry |
| `POST /api/v1/devices` | Pair with an invite, or heartbeat a device's own card |
| `GET /api/v1/pairing` | Preferences › Pairing: every device with role, created, last seen, revoked, credential and its latest actions (Operator only) |
| `GET /api/v1/pairing/<id>/log?limit=` | That device's remote reads and writes, newest first (method, path, status; never the query) |
| `POST /api/v1/pairing/<id>/role` | `{role, confirm?}`; Operator needs `confirm` = the device's name. Desk door: loopback, never a device |
| `POST /api/v1/pairing/<id>/revoke` | Refused from its next request; the hub itself answers 409 |
| `GET /api/v1/profile/brief` | `{block, prompt, state, now_ms}`: the agent brief block of the profile, the only part a Workshop device reads |
| `GET /api/v1/summary` | `{agents: [{key, presence, state, since_ms}], needs, blocked, directs}`: the Display read, no bodies |
| `POST /api/v1/events` | Satellite append of a pre-built event (token; serve validates the line) |

`GET /api/v1/protocol` serves the same catalog document as
`harbard protocol --json` and the MCP resource `harbard://protocol/v1`. Query:
`depth=compact|full`, optional `version=` (unsupported returns 406 with
`supported`).

`GET|POST /api/v1/operator` reads and sets the hub's `operator.json` display
name for the human. Authorship of notes stays `operator@{host}` / `human` /
`operator`; the display name is a label for brief and UI only (`POST` body:
`{ "display_name" }`).

## Types

`SessionView` serializes exactly the core struct: `session`, `harness`,
`host`, `cwd`, `repo`, `branch`, `role`, `parent`, `intent`,
`root`, `depth`, `descendants`, `lifecycle`, `event_count`,
`liveness` (`live|stale|ended`), `last_ms`, `last_signal_ms`, `first_ms`,
`identity` (`observed|inferred|unknown`). Linked worktrees also carry
`repo_key`. A quiet process-bound session keeps `liveness: live` for delivery
and adds `idle: true`; `idle` is omitted otherwise. `last_ms` is the last bus
event; `last_signal_ms` also includes brief heartbeats and inline hook checks.
Human `ls` and `brief` label quiet confirmed processes `idle` and distinguish
them from recent signals. `Event` is the stored schema verbatim.

`WriterLeaseView` serializes exactly the core struct: `host`,
`worktree`, `git_common_dir`, `repo` (label derived from `git_common_dir`),
`session`, `harness`, `branch`, `claimed_at_ms`, `renewed_at_ms`,
`expires_at_ms`. The holder's `pid` is never disclosed, and expired or
dead-supervisor records are absent. Degraded lease files are skipped and are
never fatal; `GET /api/v1/doctor` remains the lease-health surface.
