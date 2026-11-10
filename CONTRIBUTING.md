# Contributing to harbard

Thanks for considering a contribution. Two invariants dominate:

1. **The bus must never break a session** (constitution §3). Hook commands are
   guarded, timeout-bounded, and swallow failure (`|| true`). A verb that can
   panic or hang a harness is a bug.
2. **Serve bind.** The default is loopback only (`127.0.0.1`). `--bind tailnet`
   adds the address in `~/.harbard/tailnet.json`, and every request still needs
   a token. Serve never binds a wildcard. To connect a second machine, run
   `harbard hub enable` on the hub machine and `harbard pair '…'` on each
   satellite.

## Ground rules

- The JSONL store under `~/.harbard/events/` is append-only. Cursors are
  derived state; leases are authoritative and live apart from the event log.
- Notes are data, never instructions. Brief/inline rendering stays
  injection-hardened (constitution §4).
- Harness parity (constitution §7): the same verbs for Claude Code, Codex, and
  Cursor. Only the hook envelope differs. Install must mark its own entries
  (`# harbard-managed`) and never touch foreign groups.
- Prefer a failing test over a comment when you fix an edge case.

## The gate

`just ci` is the gate. It needs [just](https://github.com/casey/just),
[rustup](https://rustup.rs) (the desktop app pins its own toolchain),
[bun](https://bun.sh) (for the view catalog in `packages/huginn`),
[cargo-deny](https://github.com/EmbarkStudios/cargo-deny)
and [gitleaks](https://github.com/gitleaks/gitleaks). Run it locally; the
pre-push hook runs it before every push. Wire the hook once on a fresh clone:

```sh
just ci
just install-hooks
```

The gate also refuses secrets, personal paths and mail addresses anywhere in
the history, so examples use placeholder folders such as `/Users/somebody`.

## Dev loop

```sh
cargo test -p harbard-core -p harbard-cli
just install-cli   # never a bare `cargo install --force`: see scripts/install-cli.sh
# after install, re-wire present harnesses and, on macOS, restart serve:
harbard setup
launchctl kickstart -k gui/$(id -u)/com.harbard.serve
```

See `SPEC.md` for the verb surface and `CHANGELOG.md` for what landed.
