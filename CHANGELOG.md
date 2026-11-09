# Changelog

Harbard is alpha. Interfaces and the on-disk store may still change between releases.

## Unreleased

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
