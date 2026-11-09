<p align="center">
  <img src="assets/banner.png" alt="harbard, run a fleet of coding agents on one repository" width="100%">
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-e8b45a?style=flat-square&labelColor=211e19" alt="MIT license"></a>
  <img src="https://img.shields.io/badge/status-alpha-e8b45a?style=flat-square&labelColor=211e19" alt="Status alpha">
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux-2bac76?style=flat-square&labelColor=211e19" alt="Platforms macOS and Linux">
  <img src="https://img.shields.io/badge/built%20with-Rust-f2ecdd?style=flat-square&labelColor=211e19" alt="Built with Rust">
  <img src="https://img.shields.io/badge/works%20with-Claude%20Code%20%C2%B7%20Codex%20%C2%B7%20Cursor-f2ecdd?style=flat-square&labelColor=211e19" alt="Works with Claude Code, Codex and Cursor">
</p>

# harbard

Harbard coordinates parallel coding agents working on the same repository. It shows which session is working where, keeps writers in separate git worktrees, passes messages between agents, and lands a commit on your branch only after that exact commit has passed your checks.

It runs on your machine. There is no account and no hosted service. Claude Code, Codex and Cursor all use the same CLI and the same event log.

## The problem

One coding agent on a repository is simple. With four, two sessions edit the same checkout, a handoff is written and never picked up, and a change reaches `main` after being tested against a tip that no longer exists.

Harbard answers the three questions that come up:

| Question | What harbard gives you |
| --- | --- |
| Who is working, where, and on what? | Live presence, declared intent and a board of every session |
| Is someone else already writing in this checkout? | Co-presence warnings, worktree topology and optional writer leases |
| What may move `main`, and on what evidence? | A delivery queue that rebuilds each candidate on the current tip, runs your checks and lands the commit that passed |

## Install

You need a stable Rust toolchain, git and [just](https://github.com/casey/just).

```sh
git clone https://github.com/Arakiss/harbard.git
cd harbard
just install-cli
```

Connect the agents you have installed:

```sh
harbard setup --dry-run   # show every file it would change
harbard setup             # wire Claude Code, Codex and Cursor
harbard doctor            # check the hooks and the store
```

`setup` is safe to run again and leaves your other hook configuration alone.

## Quick tour

Agents call these commands themselves once the hooks are in place. You can use them too.

```sh
harbard intent "reviewing the authentication flow"       # say what you are doing
harbard ls --repo .                                      # who is live in this repository
harbard audit                                            # checkout and worktree hygiene
harbard note "The API contract changed" --kind warning \
  --to repo:/absolute/path                               # tell the other sessions here
harbard inbox                                            # read what was sent to you
```

To land work through the queue, an agent offers a commit and `integrate` decides:

```sh
harbard deliver --sha "$(git rev-parse HEAD)" --branch main
harbard integrate
harbard deliveries
```

`integrate` builds each candidate on the current tip, runs its checks against that build, and moves the branch to exactly the commit that passed.

When one session depends on another, record it so the wait survives a restart:

```sh
harbard dependency open --to 'session:<full-session-id>' \
  --condition "Review the proposed commit and report the findings" \
  --resume "Verify the findings before continuing"
harbard await <dependency-id> --timeout-secs 300
```

Run `harbard protocol --full` for the complete grammar.

## How it works

```text
agent session ─┐
agent session ─┼─ hooks and CLI ─ append-only log ─┬─ CLI
agent session ─┘                                   ├─ web console
                                                   └─ desktop app
```

Everything starts as one line in a JSONL log under `~/.harbard`. Presence, notes, topology and deliveries are projections of that log, so there is no separate database to keep in step and a corrupt line is skipped without stopping the bus.

Hooks are bounded by a timeout and fail open. A failure in harbard does not block your agent's tool call. Notes from other sessions arrive as quoted data, and agents are told to treat them as unverified text.

## Compatibility

| Harness | Presence and intent | Notes | Delivery |
| --- | --- | --- | --- |
| Claude Code | Yes | Yes | Next prompt, or an idle wake |
| Codex | Yes | Yes | Next prompt, or an idle wake |
| Cursor | Yes | Yes | Depends on the hook |

| Component | macOS | Linux | Windows |
| --- | --- | --- | --- |
| CLI and bus | Yes | Yes | No |
| Web console | Yes | Yes | No |
| Hub, self-update, background services | Yes | No | No |
| Desktop app | Yes | No | No |

## Console and desktop app

The web console and the macOS app read the same projection as the CLI. They show channels per repository, one conversation per agent, and a list of what needs you, such as blocked agents or two writers in one checkout.

```sh
bun install --frozen-lockfile
(cd apps/console && bun run build)
just up        # API and console, on this machine
```

The API binds to `127.0.0.1` and every request needs a private bearer token. The browser talks to a same-origin proxy, so the token stays on the server.

## Security

Anyone with access to your OS account can read the store, and notes or paths may be sensitive. Harbard does not redact them, so keep credentials out of notes and intents. See [SECURITY.md](SECURITY.md) before binding the hub to a tailnet, and report vulnerabilities through GitHub's private reporting.

## Documentation

- [Architecture and invariants](SPEC.md)
- [Day-to-day operation](docs/PLAYBOOK.md)
- [The delivery queue](docs/SPEC-SLUICE.md)
- [Writer leases](docs/SPEC-LEASES.md)
- [Connecting an external memory](docs/memory-sink.md)
- [The versioned agent protocol](spec/harbard/)

## Contributing

Harbard is alpha and its interfaces still change. [CONTRIBUTING.md](CONTRIBUTING.md) covers the setup. `just ci` runs the whole gate, and `just install-hooks` makes it run before every push.

## License

[MIT](LICENSE)
