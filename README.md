<p align="center">
  <img src="assets/banner.png" alt="harbard, run a fleet of coding agents on one repository" width="100%">
</p>

<p align="center">
  <a href="https://github.com/Arakiss/harbard/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/Arakiss/harbard/ci.yml?branch=main&label=ci&style=flat-square&labelColor=211e19" alt="CI status"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-e8b45a?style=flat-square&labelColor=211e19" alt="MIT license"></a>
  <img src="https://img.shields.io/badge/status-alpha-e8b45a?style=flat-square&labelColor=211e19" alt="Status alpha">
  <img src="https://img.shields.io/badge/platform-macOS%20%7C%20Linux-2bac76?style=flat-square&labelColor=211e19" alt="Platforms macOS and Linux">
  <img src="https://img.shields.io/badge/works%20with-Claude%20Code%20%C2%B7%20Codex%20%C2%B7%20Cursor-f2ecdd?style=flat-square&labelColor=211e19" alt="Works with Claude Code, Codex and Cursor">
</p>

# harbard

**Run several coding agents on one repository without them colliding.**

Harbard shows which Claude Code, Codex or Cursor session is working where, keeps each writer in its own git worktree, carries notes between sessions, and moves `main` only to a commit that passed your checks on the current tip. It runs on your machine: no account, no hosted service, one CLI and one event log for all three tools.

<p align="center">
  <img src="assets/screens/demo.webp" alt="The harbard console on a project channel: agents in the sidebar change what they are doing, a finding arrives, one agent asks whether it may lock a table, a verified delivery lands, and the operator answers the question from the channel" width="100%">
</p>
<p align="center"><sub>Agents post findings and deliveries to their project's channel. One asks a question, you answer from the console, and it goes back to work.</sub></p>

## Install

```sh
curl -fsSL https://raw.githubusercontent.com/Arakiss/harbard/main/scripts/install.sh | sh
```

On Linux, and on macOS with `--no-app`, this is quick: the newest CLI goes into `~/.local/bin` with its checksum checked (and its signature, when cosign is installed), harbard's hooks are wired into each of Claude Code, Codex and Cursor that it finds, and `harbard doctor` checks the result.

Without `--no-app`, on macOS it also compiles the desktop app on your Mac, starts the background services the app needs, and opens it. For that it installs Rust and bun if they are missing (asking once) and needs Apple's Command Line Tools (`xcode-select --install`). The first build takes several minutes; a prebuilt app is first on the [roadmap](#roadmap).

Running it again updates what it installed. Options go after `sh -s --`:

- `--dry-run` prints every action and changes nothing.
- `--yes` never asks.
- `--no-app` installs only the CLI and the hooks. The desktop app is macOS-only for now, so Linux always gets these two.
- `--version v0.3.0-alpha.10` installs that release instead of the newest.
- `--uninstall` removes what the installer added. Your data in `~/.harbard` stays unless you add `--purge`.

First run:

1. Start a Claude Code or Codex session in any repository.
2. It appears in that repository's channel in the app.
3. From another terminal in the same repository, `harbard ls --repo .` lists it.

On Linux there is no desktop app yet; to see the web console, build it from source as shown in [Console and desktop app](#console-and-desktop-app).

### From source

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

## What it answers

One coding agent on a repository is simple. With four, two sessions edit the same checkout, a handoff is written and never picked up, and a change reaches `main` after being tested against a tip that no longer exists.

| Question | What harbard gives you |
| --- | --- |
| Who is working, where, and on what? | Live presence, what each session says it is doing, and a board of every session |
| Is someone else already writing in this checkout? | A warning when two sessions share a checkout, a map of the repository's worktrees, and optional writer leases (a lock one session holds on a checkout) |
| What may move `main`, and on what evidence? | A delivery queue that rebuilds each candidate on the current tip, runs your checks and lands the commit that passed |

## What it looks like

<table>
  <tr>
    <td width="50%" valign="top">
      <img src="assets/screens/channel.webp" alt="A project channel in the console: each agent's start, findings, a blocked question and verified deliveries, with every agent's current activity in the sidebar">
      <br><sub>A channel per repository, and what every agent is doing right now.</sub>
    </td>
    <td width="50%" valign="top">
      <img src="assets/screens/needs-you.webp" alt="The Activity view with the Needs you tab: a blocked agent's question with a reply box, and an alert for an agent that has been idle mid-turn">
      <br><sub>Needs you: blocked agents and stalled sessions, answered in place.</sub>
    </td>
  </tr>
  <tr>
    <td width="50%" valign="top">
      <img src="assets/screens/deliveries.webp" alt="The Deliveries view: finished work with its commit and how far it has got, from delivered through review to integrated">
      <br><sub>Deliveries: each finished change and how far its commit has got.</sub>
    </td>
    <td width="50%" valign="top">
      <img src="assets/screens/direct-dark.webp" alt="Direct messages in dark mode: the list of agents with their current activity, and one agent's conversation with a verified delivery">
      <br><sub>One conversation per agent, here in dark mode.</sub>
    </td>
  </tr>
  <tr>
    <td colspan="2">
      <img src="assets/screens/strip.webp" alt="The Strip view for a wide 32:9 display: what needs you on the left, the agents of the last two hours as robots on a shelf, and the clock with the day's deliveries on the right">
      <br><sub>The Strip, for a wide second display: what needs you, who is working and the day's deliveries at a glance.</sub>
    </td>
  </tr>
</table>

The pictures come from the console running against its mock API with invented data. `just readme-media` regenerates them.

## Quick tour

Agents call these commands themselves once the hooks are in place. You can use them too.

```sh
harbard intent "reviewing the authentication flow"       # say what you are doing
harbard ls --repo .                                      # who is live in this repository
harbard audit                                            # checkout and worktree hygiene
harbard note "The API contract changed" --kind warning \
  --to "repo:$(pwd)"                                     # tell the other sessions here
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

Agents also have rooms, which keep what is said in them and wake nobody who has not joined; [the playbook](docs/PLAYBOOK.md) covers them. Run `harbard protocol --full` for the complete grammar.

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

| Coding agent | Presence and intent | Notes | When a note arrives |
| --- | --- | --- | --- |
| Claude Code | Yes | Yes | After its next tool call, at the end of its turn, or on an idle wake |
| Codex | Yes | Yes | Next prompt, or an idle wake |
| Cursor | Yes | Yes | Depends on the hook |

| Component | macOS | Linux | Windows |
| --- | --- | --- | --- |
| CLI and bus | Yes | Yes | No |
| Web console | Yes | Yes | No |
| Hub, self-update, background services | Yes | No | No |
| Desktop app | Yes | No | No |

[CI](.github/workflows/ci.yml) runs the Rust tests and the console checks on Linux and macOS for every pull request, and the desktop app's on macOS. The hub's background services and self-update are built on macOS LaunchAgents and the desktop app is a macOS app, so they do not run on Linux yet.

## Roadmap

In this order, each when it is done. There are no dates.

1. The desktop app and the console prebuilt in every release, so the installer downloads them instead of compiling on your Mac and no longer needs Rust or bun.
2. A demo workspace in the console with invented agents, so you can walk through every view before wiring a single session.
3. The console, the background services and the desktop app on Linux, installed the same way as on macOS.
4. A signed and notarized build of the prebuilt macOS app, so it opens without a Gatekeeper workaround.
5. How long each note took to reach its session, shown in the console.
6. Other coding agents that expose session, prompt and stop hooks. Open an issue naming yours.

Not planned: Windows. The hooks, paths and background services assume a POSIX system.

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
