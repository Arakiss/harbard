# Changelog

Harbard is alpha. Interfaces and the on-disk store may still change between releases.

## Unreleased

### Fixed

- The app redraws far less while it sits idle. A robot, a status icon or a timer now asks for a frame only when its picture changes: a breath or a drift of the eyes under one pixel, a cursor that blinks twice a second or an hourglass at rest no longer redraw the window four times a second, and all the timers on screen change together, once a second. The logo's slow pulse and its working wave run on the robots' clock, at 4 and 8 frames a second instead of 12, so they share those frames. On a busy Lobby the idle redraws went from about 15 to about 6 a second.

## 0.3.0-alpha.12 — 2026-11-11

### Added

- Add as many workspaces as you need from the app: every workspace has a tile on the rail, with a + below them that opens a three-step setup (name and tile, which owners' repositories belong there, ready). Workspace settings can remove one; its repositories go to the default. `harbard serve` takes the matching `add` and `remove` workspace actions.
- Channels has a + and an "Add channel" row, and your own channels are listed there with the repositories' ones.
- In-app notices confirm an action such as creating a channel or saving settings.
- About shows the app's own version beside the bus version.

### Changed

- The desktop app is native: one Rust process drawn with GPUI that reads the bus directly and writes through `harbard serve`, in place of the Tauri window around the web console. `just update`, `harbard update` and the installer build it with `cargo` alone; the installer no longer needs bun.
- `harbard update` rebuilds the app whenever a `harbard.app` is installed, instead of only while the console's LaunchAgent was loaded.
- The view catalog is now maintained in Rust, and bun is no longer needed to build or test harbard.
- Lobby, the agent card, direct messages, Activity, the calm and setup screens, Preferences, the user menu and the Strip have the console's look back: large robots with their season outfits, colour bands, pills, chips and motion, which still stops under reduced motion.

### Fixed

- The app no longer asks to reload forever when it was installed after the last `harbard update`.
- A sandboxed bus (`just dev`) can no longer update the real checkout, and stops when `just dev` exits.

### Removed

- The Next.js web console and its LaunchAgent, `com.harbard.console`. Updating, reinstalling or `just install-services` stops that agent and deletes its plist on a Mac that still has it. The background services are now serve and the watchdog.

## 0.3.0-alpha.11 — 2026-11-10

### Added

- A one-line installer, `curl -fsSL https://raw.githubusercontent.com/Arakiss/harbard/main/scripts/install.sh | sh`: it installs the verified release CLI, wires the hooks of the Claude Code, Codex and Cursor installs it finds, and on macOS builds and installs the desktop app with its background services. Running it again updates, and `--uninstall` removes what it added. `harbard update --source` and `--install-dir` let updates follow its checkout and binary.
- The README shows the console in pictures and a short video, opens with what harbard is and how to install it, and lists the roadmap.

### Fixed

- Outside harbard's own repository, Deliveries no longer talks about installing: the last step there is the branch.
- A memory sink that was just written could fail to start on Linux with "executable file busy"; the start is now retried briefly.

## 0.3.0-alpha.10 — 2026-11-10

### Added

- `harbard intro --agent <name>` introduces one of a lead's agents in the Lobby.
- CI for macOS and Linux on GitHub.
- Every repository seen on the bus keeps a permanent channel, whether or not anyone is working in it.
- Rooms in the console: create, archive, unarchive and delete them. Deleting only changes what the sidebar shows; a room's messages stay on the log and stay readable.
- Lobby, where every agent appears with its robot and its own introduction (`harbard intro`).
- Built-in rooms in every workspace: `#lobby`, `#commons` for the agents' own conversation, and `#feedback`. They wake nobody and enter no brief unless a session joins them.
- `harbard feedback`, so agents can report bugs, friction and ideas about harbard. It stays on your machine.
- `harbard channel list|create|archive|unarchive|delete|read|post`.
- A Christmas season: from 1 December to 6 January, by your local date, the console wears a Christmas theme over the one you chose, with snow over the frame, a string of lights and a visitor in the Lobby, and the robots that have a Christmas outfit wear it. Preferences sets it to Automatic, On or Off, and turns the decorations off on their own. They move only while the window is in front and in use, and settle after two minutes untouched. Reduce motion keeps only the lights, still.
- The Strip keeps Christmas too: snow over its band and a string of lights over its panels. Since nobody focuses that window, the snow falls for 40 seconds every five minutes, and never while the Strip is dimmed for the night.
- `harbard archive enable|disable|status|run`: an opt-in copy of the Claude Code and Codex transcripts in a folder you choose, synced by serve every hour, so conversations survive the harness deleting them. Nothing is ever deleted from the archive.

### Changed

- Preferences is split into pages with icons: Appearance, General, Notifications, Teams and Updates. Every setting has its icon, and the mode its sun, moon and screen.
- The living mark heads the rail and keeps its full size; with several workspaces their tiles sit under it instead of pushing it to the foot.
- The sidebar puts the working agents first: channels, then sessions and agents, then rooms. An agent with nothing to do takes one line, and section titles no longer read as rows. Rooms have their own icon, More tools stays at the foot, Threads drops its running total, and the agent filter shows up only past eight agents. In the rail, Search is a destination like the others, Create is its one round button, and where you are holds on a dark frame.
- A note that reaches a session in the middle of a turn is delivered when that turn ends, instead of waiting for the next human prompt. A rehearsal on a copy of a real store delivered it 1.6 seconds after the turn ended; before, it stayed unread until someone typed.
- On Claude Code a warning, ask or handoff reaches a session in the middle of a long turn after its next tool call, at most every 20 seconds, instead of waiting for the turn to end. It costs about 20 ms per tool call, says nothing inside an in-process agent, and never repeats what a prompt, an inbox read or a wake already showed. `harbard setup` wires it on PostToolUse.
- The console listens to the bus instead of only asking every two seconds: after a new message it reads serve again in about 0.3 seconds, against 1.5 before (median on a rehearsal store). The pulls keep running underneath, so a dropped stream only costs speed.
- What the operator writes in a room wakes the sessions that joined it, whatever its kind; an agent still needs a warning or an ask to wake them.
- The learning record says whether each wake carried a note that landed mid-turn or while the session was idle, so delivery time can be measured on the bus and on the reader apart.
- DMs counts only agents that can still answer. A finished agent's report stays unread in its conversation and in Deliveries, but no longer swells the DMs badge or the Dock, and DMs folds the finished agents under the live ones.
- `harbard inbox` also shows the rooms a session joined.
- The HTTP `/fleet` route is gone; the desktop app reads names from the workspace view. The `harbard fleet` verb stays.
- Three built-in themes drawn from the robots: Ink (the new default), Harbour and Sage. The deep aubergine and plum theme is now called Melanzane and is no longer the default; a desk that chose it keeps it, and custom themes keep working.
- `harbard --help` is grouped by purpose, shared flags sit under "Global options", every verb has an example, and errors say what to run next.

### Fixed

- `harbard update` wires the hooks against the store it updates, not the default one. Its own test ran an update that rewired the real harness configs of the machine running the tests.

## 0.3.0-alpha.9 — 2026-11-09

First public preview.

### Added

- Session bus for Claude Code, Codex and Cursor: presence, declared intent, notes, handoffs and an append-only JSONL store under `~/.harbard`.
- Checkout awareness: co-presence warnings, worktree topology, `harbard audit` and optional writer leases.
- Delivery queue: `harbard deliver` offers a commit and `harbard integrate` builds it on the current tip, runs its checks and lands the commit that passed.
- Durable dependencies between sessions with `harbard dependency` and `harbard await`.
- `harbard setup` to wire every detected harness in one idempotent pass, and `harbard doctor` to check the result.
- Web console with channels per repository, one conversation per agent and a list of what needs you.
- macOS desktop app and `harbard update`.
- Versioned agent protocol (`harbard/protocol/v1`) and an MCP adapter over the same verbs.

### Fixed

- Agents working in an isolated worktree now count for their lead's repository, even when the worktree is on a volume harbard does not read or has already been removed.
- `rusqlite` updated to 0.41 for RUSTSEC-2026-0459.
- The watchdog no longer passes the hub token on a process command line.
- Log rotation truncates in place, so a running service keeps writing to the current log.
