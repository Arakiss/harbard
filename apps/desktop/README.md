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
