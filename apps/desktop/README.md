# harbard desktop

A Tauri shell around the web console. It adds what a browser tab cannot:

- **Menubar tray** with the fleet's live count (the product mark as a template
  icon, so macOS inverts it for the light and dark menubar). Every icon in
  `src-tauri/icons/` is generated: edit `assets/brand/*.svg` at the repo root
  and run `scripts/render-brand.sh`, never the PNGs.
- **Native notifications** when the fleet answers the operator: an ack to your
  handoffs, or a `warning`/`handoff` from an agent
- **Global shortcut** `⌘⇧Space` to summon/hide the window; closing the window
  hides it (quit lives in the tray menu)
- **A living Dock icon** (`src/dock.rs`): the console reports the living
  logo's state through `desk_set_status` (`apps/console/src/lib/desktop-status.ts`).
  At rest the Dock shows the bundle icon. Every other state swaps a tile that
  `scripts/render-dock.py` pre-renders: green with a halo while working,
  amber when you are needed, red for a block, a hollow grey ring with no bus.
  A new message and a verified delivery each play one animation of under a
  second, at most one every 3 s and none in Low Power Mode. The red badge
  counts Needs you plus unread Direct; it hides at 0 and reads 99+ above
  99. A new Needs you item bounces the Dock once when harbard is not
  frontmost, unless Preferences › Notify in the Dock is off. The tray icon
  carries the state as a shape: a dot, a ring, or an exclamation. The
  shell's own bus poll shows the outage itself. Closing the window only
  hides it: in the desktop shell the console keeps pulling the faces'
  slice (`?only=activity`, which carries needs and directs) every 10 s
  while hidden (2 s when visible; a browser tab stops), so the badge and
  the bounce stay live. A live console
  repeats its state every 20 s; once it has reported, 60 s of silence (its
  page gone, the console down) shows the outage ring with the last badge,
  until the next report. `just desktop-dock-demo` builds
  a separate bundle (`com.harbard.desktop.dockdemo`, its own
  single-instance socket, a local page) in which `HARBARD_DOCK_DEMO=1`
  cycles every state and event once. Any other build refuses the demo.
- **macOS 26 layered icon**: `assets/brand/harbard.icon` (tile, bubble and
  dot layers; `icon.json` is hand-written) is compiled by actool into
  `src-tauri/icons/Assets.car`. `tauri build` copies it into
  `Contents/Resources`, and `Info.plist` adds `CFBundleIconName`. Older macOS
  keeps the `.icns`.

The console stays a web app on loopback (see `docs/SPEC-API.md`). This shell loads it, holds
no bus logic, and obeys the console's no-egress rule: its only network is
`127.0.0.1:7419` (API) plus the local console URL.

## Install (Spotlight / Launchpad)

The production console LaunchAgent must be installed first. On a machine that
has never run harbard, install both agents, the bus API and the console,
once:

```sh
just install-services
```

That builds the console, writes `com.harbard.serve` and `com.harbard.console`
to `~/Library/LaunchAgents`, loads them, and verifies both answer on loopback.
Quit every running installed copy through the native app menu first. Then run
the update from the persistent checkout used by the console LaunchAgent:

```sh
just update
```

The update builds both targets before replacement, signs and stages both app
copies, retains the previous bundles, restarts the verified console LaunchAgent
and compares its served assets with the current build. A private receipt under
`target/desktop-installs/` records the source fingerprint and BUILD_ID; a
copy is embedded in each signed app. Finally reopen `/Applications/harbard.app`
(or the reported installed path) through native app control and inspect both
views. The installer never kills or opens apps by shell commands.

For the uncommon case where a development loop owns the console and only the
Tauri shell should move:

```sh
just desktop-shell
```

That builds a release `harbard.app`, installs it to `/Applications/harbard.app`
when writable (otherwise `~/Applications`), mirrors into `~/Applications`,
registers it with Launch Services, and reports the installed path to reopen. When both `/Applications` and
`~/Applications` are present, the signed app is mirrored only after signing and
their executable digests must match. Spotlight can therefore choose either copy
without reopening a stale build.

Force a destination:

```sh
HARBARD_APP_DIR=/Applications just update
```

## Dev loop

For rapid tray / notification / shortcut iteration without rebundling:

```sh
just desktop-dev
```

## Console URL

The shell picks the console URL automatically:

1. `HARBARD_CONSOLE_URL` if set
2. `http://127.0.0.1:3419` when launchd / `next start` answers
3. `http://harbard.local:1355` when the portless dev console answers
4. otherwise falls back to `:3419`

```sh
HARBARD_CONSOLE_URL=http://harbard.local:1355 just desktop-dev
```

## Desk discipline

The shell is meant to stay out of the way:

- **No focus theft on doorbell.** Native notify, plus the Strip view when a strip display is present;
  the monitor-clamped fallback opens only on explicit request, and `set_focus` is never called when the fleet rings. Focus is
  only for explicit summon (`⌘⇧Space` / tray Open).
- **Tray mute.** Notification mute/unmute lives in the tray menu and
  `~/.harbard/desk.json` (`desk_get_prefs` / `desk_set_mute` for the console).
- **Notification clicks.** macOS notify-rust has no click callback; use tray
  **Open harbard** or the summon shortcut.
- **Lean poll.** Loopback `127.0.0.1:7419` every 4s; alias cache TTL 60s; one
  OS toast per event batch; one `summon_pulse` per batch.
- **Not always-on-top.** Close hides to the tray; Quit is an explicit menu act.
- Webview capabilities omit `set_focus` so loopback console JS cannot yank the
  keyboard. Rust still focuses on user-initiated toggle.

## Boundaries

- Outside the root Cargo workspace on purpose: the tauri/wry tree must not
  slow or fail the bus's gates (`just gates`). Its own lockfile under
  `src-tauri/`.
- Local install is an `.app` bundle (`bundle.targets = ["app"]`).
- UI freshness comes from the running console. `just update` owns that complete
  dependency chain and is the default after any product change. Use
  `just desktop-shell` only when a separate development loop intentionally owns
  the console.
- Loopback console URLs are declared under capability `remote.urls` so the
  Overlay titlebar's `data-tauri-drag-region` can IPC `toggleMaximize` on
  double-click (macOS zoom; use the green button or ⌃⌘F for Spaces fullscreen).
