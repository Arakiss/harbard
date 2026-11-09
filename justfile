default: gates

# Install the CLI into ~/.cargo/bin and ad-hoc sign (required on macOS so
# launchd / Cursor failClosed hooks do not SIGKILL an unsigned binary). The
# binary is swapped in by rename, never rewritten in place: see the script.
install-cli:
    bash scripts/install-cli.sh

# Install the LaunchAgents the desktop app depends on (API, console, watchdog).
# Run this once, before the first `just update`.
install-services:
    #!/usr/bin/env bash
    set -euo pipefail
    (cd apps/console && bun run build)
    bash scripts/install-launchagents.sh

# The whole gate. `just install-hooks` runs it before every push.
ci: oss-guard gates desktop-gates console-gates watchdog-gate desktop-install-gate git-hooks-gate install-cli-gate

gates: fmt clippy test doc deny

# Secrets, personal paths and mail addresses in anything HEAD reaches, plus any
# name in the private rules `git config harbard.privateGitleaks` points to.
# Examples use placeholder folders such as /Users/somebody.
oss-guard:
    #!/usr/bin/env bash
    set -euo pipefail
    if printf '/Users/%s/work\n' zorblat | gitleaks stdin --no-banner --log-level error --config .gitleaks.toml; then
      echo "oss-guard: the path rule stopped matching" >&2; exit 1
    fi
    gitleaks git --no-banner --log-level warn --redact --log-opts=HEAD --config .gitleaks.toml
    private="$(git config --type=path --get harbard.privateGitleaks || true)"
    if [ -n "$private" ]; then
      gitleaks git --no-banner --log-level warn --log-opts=HEAD --config "$private"
      git log --format=%B HEAD | gitleaks stdin --no-banner --log-level warn --config "$private"
    fi

fmt:
    cargo fmt --all --check

clippy:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

doc:
    cargo doc --workspace --no-deps

deny:
    cargo deny check

# The Tauri crate is not a workspace member, so the root gates do not cover it.
desktop-gates:
    cd apps/desktop/src-tauri && cargo fmt --check
    cd apps/desktop/src-tauri && cargo clippy --all-targets -- -D warnings
    cd apps/desktop/src-tauri && cargo test

# `bun test` is scoped to src because the Playwright e2e specs are not bun tests.
console-gates:
    bun install --frozen-lockfile
    bun audit --audit-level=high
    cd packages/huginn && bun run typecheck && bun test src
    cd apps/console && bun run typecheck
    cd apps/console && bun test src
    cd apps/console && bun run build

watchdog-gate:
    bash -n scripts/rotate-logs.sh
    bash -n scripts/harbard-watchdog.sh
    bash -n scripts/install-launchagents.sh
    bash scripts/test-watchdog.sh

desktop-install-gate:
    bash -n scripts/update-desktop.sh apps/desktop/scripts/install.sh
    PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-desktop-runtime.py

git-hooks-gate:
    PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-git-hooks.py

install-cli-gate:
    bash -n scripts/install-cli.sh
    bash scripts/test-install-cli.sh

# Install the pre-push hook that runs `just ci`. Idempotent.
install-hooks:
    #!/usr/bin/env bash
    set -euo pipefail
    root="$(git rev-parse --show-toplevel)"
    gitdir="$(git rev-parse --absolute-git-dir)"
    install -d "$gitdir/hooks"
    install -m 0755 "$root/scripts/pre-push" "$gitdir/hooks/pre-push"
    echo "installed: $gitdir/hooks/pre-push"

# Bring the whole product up locally: bus API + fleet console.
up:
    #!/usr/bin/env bash
    harbard serve --port 7419 & S=$!
    (cd apps/console && bun run start) & C=$!
    trap "kill $S $C 2>/dev/null" EXIT
    echo "harbard up — API :7419 · console http://localhost:3419"
    wait

# Console dev loop against the mock bus: one command, both processes, HMR on.
# The console is fronted by portless — remember http://harbard.local:1355
# (or https://harbard.localhost), not a port number.
dev:
    #!/usr/bin/env bash
    (cd apps/console && PORT=7420 bun run mock/serve.ts) & M=$!
    (cd apps/console && HARBARD_API=http://127.0.0.1:7420 bun run dev) & C=$!
    trap "kill $M $C 2>/dev/null" EXIT
    # Give portless a beat to register, then print the stable URL.
    sleep 1
    URL=$(cd apps/console && bunx portless get harbard 2>/dev/null || true)
    echo "harbard dev — mock bus :7420 · console ${URL:-http://harbard.local:1355} (HMR)"
    wait

# Rebuild and install the production console and the desktop app, verified.
update:
    bash scripts/update-desktop.sh

desktop-dev:
    cd apps/desktop && bun run dev

# A throwaway bundle for watching the living Dock icon beside the installed
# app. Its own bundle id gives it its own single-instance socket, so it can
# never hand off to (or show the window of) the installed copy; its window
# is a local page, so it reaches neither serve nor the console. Not installed:
# run the printed binary with HARBARD_DOCK_DEMO=1 and quit it from the tray.
desktop-dock-demo:
    cd apps/desktop && bunx --bun @tauri-apps/cli build --bundles app --config src-tauri/tauri.dockdemo.conf.json
    @echo "HARBARD_DOCK_DEMO=1 HARBARD_CONSOLE_URL=http://127.0.0.1:1 ${CARGO_TARGET_DIR:-apps/desktop/src-tauri/target}/release/bundle/macos/harbard-dockdemo.app/Contents/MacOS/harbard-desktop"

# Rebuild only the Tauri shell. Prefer `just update` unless the console is
# deliberately managed by another development loop.
desktop-shell:
    cd apps/desktop && bun run install:app

# Cut a release from published main: bump, changelog, gate, tag, push, the
# signed GitHub release (notes = the Unreleased section) and the local install.
release version title="":
    bash scripts/release.sh "{{version}}" "{{title}}"
