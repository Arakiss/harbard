# harbard desktop

The native fleet console: one Rust process drawn with GPUI
([gpui-fast](https://github.com/longbridge/gpui-fast) + [GPUI Kit](https://github.com/longbridge/gpui-kit)).

- **Reads** come from the bus log directly. A background thread (`src/hub.rs`)
  keeps the hub's window of events in memory, wakes the moment the store
  changes (kqueue, `harbard_core::log_watch`) and recomputes the workspace read
  with serve's own code. No HTTP, no polling on the hot path.
- **Writes** go to `harbard serve` on loopback (`src/api.rs`). serve validates
  and appends; the hub sees the append like any other.

## Layout

| Path | What |
|---|---|
| `src/hub.rs` | the reader thread |
| `src/desk.rs` | `Desk`: the live state screens read, and the write door |
| `src/model/` | the read's types (serve's JSON shapes) |
| `src/theme.rs` | the `--hb-*` colour tokens, from serve's theme read |
| `src/i18n.rs` | the English and Spanish dictionaries in `assets/locales` |
| `src/nav.rs` | routes and history |
| `src/shell/` | the window frame: title band, rail, panel |
| `src/screens/` | one screen per route |
| `src/ui/` | shared pieces: icons, type scale, badges, formatting |

## Run

```sh
cargo run                      # against the live bus (~/.harbard, serve on :7419)
scripts/sandbox.sh start /tmp/hb-sandbox 7519
HARBARD_HOME=/tmp/hb-sandbox HARBARD_API_PORT=7519 cargo run   # against a copy
```

The toolchain is pinned in `rust-toolchain.toml`; put `~/.cargo/bin` first on
`PATH` so cargo uses rustup's compiler.

## Build and install

```sh
scripts/bundle.sh              # release build → <target>/release/bundle/macos/harbard.app
just update                    # from the repository root: build, swap the installed copies, verify
```

`scripts/bundle.sh` assembles `harbard.app` by hand: the release binary as
`Contents/MacOS/harbard-desktop` (the name every script finds the running app
by), `bundle/Info.plist` with the version from `Cargo.toml`, the icons in
`bundle/`, and `harbard-build.json`, the source the build came from. The bundle
is signed ad hoc. `scripts/install.sh` puts it in `/Applications` and
`~/Applications`, keeps the previous copy as a backup and refuses to replace a
copy that is running. `just update` (`scripts/update-desktop.sh`) stops there
when the app is open; `harbard update` quits it first and reopens it after.

`build.rs` embeds the short commit as `HARBARD_APP_COMMIT` (set it to override
git) and refuses to build when `bundle/Assets.car` is older than its sources in
`assets/brand/harbard.icon`; `scripts/render-brand.sh` recompiles it.

Gates: `just desktop-gates` (fmt, clippy with warnings denied, tests) and
`just desktop-deny` (`deny.toml`: this tree's licences and advisories).
