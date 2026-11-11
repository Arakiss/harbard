default: gates

# Install the CLI into ~/.cargo/bin and ad-hoc sign (required on macOS so
# launchd / Cursor failClosed hooks do not SIGKILL an unsigned binary). The
# binary is swapped in by rename, never rewritten in place: see the script.
install-cli:
    bash scripts/install-cli.sh

# Install the LaunchAgents the desktop app depends on (API and watchdog), and
# retire the web console's agent on a Mac that still has it. Run this once,
# before the first `just update`.
install-services:
    bash scripts/install-launchagents.sh

# The whole gate. `just install-hooks` runs it before every push.
ci: oss-guard gates desktop-gates desktop-deny watchdog-gate desktop-install-gate git-hooks-gate install-cli-gate install-gate

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

# rustup's cargo goes first: only it honours apps/desktop/rust-toolchain.toml.
# The app is its own workspace, so the root gates do not cover it.
desktop-gates:
    cd apps/desktop && PATH="$HOME/.cargo/bin:$PATH" cargo fmt --check
    cd apps/desktop && PATH="$HOME/.cargo/bin:$PATH" cargo clippy --all-targets --locked -- -D warnings
    cd apps/desktop && PATH="$HOME/.cargo/bin:$PATH" cargo test --locked

# Licences, advisories and sources of the app's tree, with its own exceptions.
desktop-deny:
    cargo deny --manifest-path apps/desktop/Cargo.toml check --config apps/desktop/deny.toml

# Rewrite the agents' guide to views after a change to the catalog
# (crates/harbard-cli/src/huginn/catalog.json). `just test` fails until it is.
huginn-guide:
    HUGINN_BLESS=1 cargo test -p harbard-cli --lib huginn::guide

watchdog-gate:
    bash -n scripts/rotate-logs.sh
    bash -n scripts/harbard-watchdog.sh
    bash -n scripts/install-launchagents.sh scripts/retire-console.sh
    bash scripts/test-watchdog.sh
    bash scripts/test-launchagents.sh

desktop-install-gate:
    bash -n scripts/update-desktop.sh apps/desktop/scripts/install.sh apps/desktop/scripts/bundle.sh
    PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-desktop-runtime.py

git-hooks-gate:
    PYTHONDONTWRITEBYTECODE=1 python3 scripts/test-git-hooks.py

install-cli-gate:
    bash -n scripts/install-cli.sh
    bash scripts/test-install-cli.sh

# The one-line installer, in throwaway HOMEs against a locally packaged
# tarball: never the network, the LaunchAgents or the installed app.
install-gate:
    sh -n scripts/install.sh
    bash scripts/test-install.sh

# Install the pre-push hook that runs `just ci`. Idempotent.
install-hooks:
    #!/usr/bin/env bash
    set -euo pipefail
    root="$(git rev-parse --show-toplevel)"
    gitdir="$(git rev-parse --absolute-git-dir)"
    install -d "$gitdir/hooks"
    install -m 0755 "$root/scripts/pre-push" "$gitdir/hooks/pre-push"
    echo "installed: $gitdir/hooks/pre-push"

# Bring the whole product up locally: the bus API, and the app reading it.
up:
    #!/usr/bin/env bash
    harbard serve --port 7419 & S=$!
    trap "kill $S 2>/dev/null" EXIT
    echo "harbard up — API :7419 · the app opens on top"
    (cd apps/desktop && PATH="$HOME/.cargo/bin:$PATH" cargo run --release)

# The app against a throwaway copy of the bus (apps/desktop/scripts/sandbox.sh):
# its writes land in the copy, never in ~/.harbard. Stops the copy's serve on exit.
dev dir="/tmp/harbard-dev" port="7519":
    #!/usr/bin/env bash
    set -euo pipefail
    bash apps/desktop/scripts/sandbox.sh start "{{dir}}" "{{port}}"
    trap 'bash apps/desktop/scripts/sandbox.sh stop "{{dir}}"' EXIT
    cd apps/desktop
    HARBARD_HOME="{{dir}}" HARBARD_API_PORT="{{port}}" PATH="$HOME/.cargo/bin:$PATH" cargo run

# Rebuild and install the desktop app, verified against its source.
update:
    bash scripts/update-desktop.sh

# The app against the live bus (~/.harbard, serve on :7419).
desktop-dev:
    cd apps/desktop && PATH="$HOME/.cargo/bin:$PATH" cargo run

# Build harbard.app without installing it; prints the bundle's path.
desktop-bundle:
    bash apps/desktop/scripts/bundle.sh

# Cut a release from published main: bump, changelog, gate, tag, push, the
# signed GitHub release (notes = the Unreleased section) and the local install.
release version title="":
    bash scripts/release.sh "{{version}}" "{{title}}"
