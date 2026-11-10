# Updates

Harbard tells whoever runs it that a newer version exists, shows what changed,
and installs it in one step: from the console, from the desktop app, or with
`harbard update`.

## Channels

| Channel    | "Newer" means                                                     | Notes shown                                   |
|------------|-------------------------------------------------------------------|-----------------------------------------------|
| `main`     | `origin/main` has commits the running serve was not built from    | `CHANGELOG.md` "Unreleased" on `origin/main`  |
| `releases` | a `v*` tag whose version is greater than the running version      | every `CHANGELOG.md` section newer than ours  |

The default is `main`, which follows every commit; `releases` follows tagged
versions only. Both channels need only `git` and the credentials the checkout
already uses; no GitHub token, no `gh`.

## Where the answer comes from

- **The source checkout.** The primary checkout of the repository the binary
  was built from (`build.rs` embeds the source root; a linked worktree resolves
  to its primary). `~/.harbard/update.json` may name another one
  (`harbard update --source <dir>`); the one-line installer names the checkout
  it clones, because a release binary was built somewhere else.
- **serve** checks in the background: two minutes after it starts, then every
  six hours, when `auto_check` is on. The result is cached in
  `~/.harbard/update/check.json`.
- **Hooks never touch the network.** SessionStart reads the cache and, when an
  update is available, says so in one line. When the cache is older than a day
  and serve is not running, it starts a detached `harbard update --check
  --quiet`, at most once an hour.
- **Satellites** report their version and commit to the hub with their card;
  Pairing shows who runs what and who is behind the hub.

## The run

`harbard update` (or the console's "Update") runs, in order:

1. `fetch`: fetch `origin`, fast-forward the checkout's `main` to the target.
   Refused when the checkout has uncommitted tracked changes, is not on `main`,
   or `main` has commits `origin` does not.
2. `cli`: `scripts/install-cli.sh`, into `update.json`'s `install_dir` when
   set (`harbard update --install-dir <dir>`), so an update replaces the copy
   the installer put in `~/.local/bin` instead of adding a second one.
3. `hooks`: `harbard setup` with the new binary, so a version that adds a
   hook reaches every machine. Never fatal; doctor reports a stale wiring.
4. `serve`: `launchctl kickstart -k gui/<uid>/com.harbard.serve` (skipped
   when that job is not loaded).
5. `console`: `scripts/update-desktop.sh`, which builds the console and the
   app, swaps the app and restarts the console (skipped when
   `com.harbard.console` is not loaded). An open app is quit natively before
   the swap and reopened after it.
6. `desktop`: the reopen, reported apart so the UI can say it.

The `serve`, `console` and `desktop` steps use macOS LaunchAgents.

The run is a detached process (its own process group), so restarting serve
and the console does not stop it. It writes `~/.harbard/update/run.json`
after every step and its output to `~/.harbard/update/run.log`.

## API

All under `/api/v1`. Reads for Operator; writes loopback-only, Operator, never
from a satellite (`X-Harbard-Device`), same-origin.

### `GET /update`

```json
{
  "installed": { "version": "0.3.0-alpha.9", "commit": "a955a72…" },
  "prefs": { "channel": "main", "auto_check": true },
  "source": { "path": "/Users/…/harbard", "problem": null },
  "check": {
    "checked_ms": 1762600000000,
    "channel": "main",
    "status": "available",
    "error": null,
    "target": {
      "version": "0.3.0-alpha.9",
      "commit": "…",
      "tag": null,
      "title": null,
      "date_ms": 1762600000000
    },
    "behind": 11,
    "notes": "### Hooks that speak only when spoken to\n\n- …",
    "changes": [
      { "commit": "…", "kind": "feat", "scope": "console", "subject": "a rail tile per workspace", "date_ms": 0 }
    ],
    "changes_truncated": false
  },
  "run": {
    "id": "01…",
    "state": "running",
    "started_ms": 0,
    "finished_ms": null,
    "from": "…",
    "to": "…",
    "error": null,
    "steps": [
      { "id": "fetch", "state": "done", "started_ms": 0, "finished_ms": 0, "detail": null }
    ],
    "log_tail": ["…"],
    "notes": "### …",
    "changes": [],
    "changes_truncated": false
  }
}
```

- `check` is `null` until the first check; `run` is `null` until the first run.
- `check.status`: `current` | `available` | `error`.
- `source.problem`: `null` | `no_checkout` | `dirty` | `not_on_main` | `diverged`.
- `change.kind`: `feat` | `fix` | `perf` | `refactor` | `docs` | `test` | `build` | `chore` | `other`.
- `run.state`: `running` | `done` | `failed`; `step.state`: `pending` | `running` | `done` | `failed` | `skipped`.
- `step.id`: `fetch` | `cli` | `hooks` | `serve` | `console` | `desktop`.
- `step.detail`: for a skipped step a code, `not_installed` (no such
  LaunchAgent on this machine) or `not_open` (the app was not open, so it is
  not reopened); for a failed step an English message.
- `run.notes`, `run.changes`: what that run brings, copied from the check it
  started from, so "what's new" survives the new build's `current` check.
- `run.error` and `check.error` are `{ "code": "...", "message": "..." }` when set.
  Codes: `fetch_failed`, `dirty`, `not_on_main`, `diverged`, `no_checkout`,
  `step_failed`, `interrupted`.

### `POST /update/check`

Checks now (synchronously, up to about 30 s) and answers the `GET` body.

### `POST /update/install`

Starts a run. `202` with the `GET` body. `409 { "error": "busy", "message": … }`
while one runs; `409` with the source problem's code as `error` when the
checkout cannot fast-forward. Refusals use serve's usual shape,
`{ "error": "<code>", "message": "<text>" }`.

### `POST /update/prefs`

`{ "channel"?: "main" | "releases", "auto_check"?: boolean }`. Answers the
`GET` body; a channel change re-checks.

## CLI

```sh
harbard update               # check, show what changed, ask, install
harbard update --check       # check only (exit 0 current, 10 available)
harbard update --yes         # install without asking
harbard update --channel releases
harbard update --json        # the GET body
```
