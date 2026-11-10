#!/usr/bin/env bash
# scripts/install.sh end to end, in throwaway HOMEs, against a tarball packaged
# here from this checkout's binary. Never the network, never the real
# ~/.harbard, ~/.claude, ~/.codex, ~/.cursor, LaunchAgents or /Applications:
# every run is either --no-app, a dry run, or stopped before its first step,
# so nothing reaches launchctl or the app.
#
#   HARBARD_TEST_BIN  the harbard binary to package (default: a debug build)
# shellcheck disable=SC2016 # the single-quoted $ are meant: literal lines and snippets for sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
INSTALLER="$ROOT/scripts/install.sh"
TMP="$(mktemp -d)"
TMP="$(cd "$TMP" && pwd -P)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
ok() { echo "ok: $*"; }
inode() { stat -f %i "$1" 2>/dev/null || stat -c %i "$1"; }
sha() { if command -v sha256sum >/dev/null; then sha256sum "$1" | cut -d' ' -f1; else shasum -a 256 "$1" | cut -d' ' -f1; fi; }
has() { grep -Fq -- "$2" "$1" || { echo "--- $1" >&2; cat "$1" >&2; fail "$3: expected \"$2\""; }; }
hasnt() { if grep -Fq -- "$2" "$1"; then echo "--- $1" >&2; cat "$1" >&2; fail "$3: did not expect \"$2\""; fi; }

BIN_SRC="${HARBARD_TEST_BIN:-}"
if [ -z "$BIN_SRC" ]; then
  (cd "$ROOT" && cargo build --quiet --locked --bin harbard)
  BIN_SRC="${CARGO_TARGET_DIR:-$ROOT/target}/debug/harbard"
fi
[ -x "$BIN_SRC" ] || fail "no harbard binary at $BIN_SRC"
VERSION_LINE="$("$BIN_SRC" --version)"

# One archive under every target name, so the host's (and a pretend Linux's)
# is always there; packaged like release.yml packages it.
package() {
  local dist="$1" pkg="$TMP/pkg"
  rm -rf "$pkg"
  mkdir -p "$pkg" "$dist"
  cp "$BIN_SRC" "$pkg/harbard"
  cp "$ROOT/LICENSE" "$ROOT/README.md" "$pkg/"
  for t in x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu x86_64-apple-darwin aarch64-apple-darwin; do
    tar -C "$pkg" -czf "$dist/harbard-$t.tar.gz" .
    (cd "$dist" && printf '%s  %s\n' "$(sha "harbard-$t.tar.gz")" "harbard-$t.tar.gz" >"harbard-$t.tar.gz.sha256")
  done
}
package "$TMP/dist"

# run <home> <out-file> [flags...]: the installer with nothing of this
# machine's environment but the system tools (no cosign, no real harness).
run() {
  local home="$1" out="$2"
  shift 2
  mkdir -p "$home" "$TMP/t"
  env -i HOME="$home" PATH="${RUN_PATH:-/usr/bin:/bin:/usr/sbin:/sbin}" SHELL=/bin/zsh \
    LANG=en_US.UTF-8 TERM=xterm-256color TMPDIR="$TMP/t" \
    HARBARD_INSTALL_FROM="${RUN_FROM:-$TMP/dist}" HARBARD_VERSION=v0.0.0-test \
    ${RUN_OS:+HARBARD_INSTALL_OS="$RUN_OS"} ${RUN_NO_COLOR:+NO_COLOR=1} \
    "${RUN_SH:-sh}" "$INSTALLER" "$@" >"$out" 2>&1
}

harnesses() { mkdir -p "$1/.claude" "$1/.codex" "$1/.cursor"; }
snapshot() { (cd "$1" && find . -print | LC_ALL=C sort && find . -type f -exec cksum {} + | LC_ALL=C sort); }

# --- help and refusals -------------------------------------------------------
sh "$INSTALLER" --help >"$TMP/help" || fail "--help exited non-zero"
has "$TMP/help" "--uninstall" "--help lists the flags"
if sh "$INSTALLER" --bogus >"$TMP/bogus" 2>&1; then fail "an unknown flag was accepted"; fi
has "$TMP/bogus" "unknown option --bogus" "unknown flag"
if RUN_OS=FreeBSD run "$TMP/h-bsd" "$TMP/bsd" --no-app; then fail "an unsupported OS installed"; fi
has "$TMP/bsd" "does not support FreeBSD" "unsupported OS"
[ ! -e "$TMP/h-bsd/.local" ] || fail "an unsupported OS left files"
ok "help, unknown flags and unsupported systems"

# --- dry run changes nothing ---------------------------------------------------
H="$TMP/h-dry"
harnesses "$H"
before="$(snapshot "$H")"
run "$H" "$TMP/dry" --dry-run || { cat "$TMP/dry" >&2; fail "dry run exited non-zero"; }
[ "$before" = "$(snapshot "$H")" ] || fail "--dry-run changed the home"
has "$TMP/dry" "Dry run complete" "dry run"
has "$TMP/dry" "would install ~/.local/bin/harbard" "dry run names the binary"
has "$TMP/dry" "Claude Code  found" "dry run detects Claude Code"
has "$TMP/dry" "would wire ~/.cursor/hooks.json" "dry run plans Cursor"
if [ "$(uname -s)" = "Darwin" ]; then
  has "$TMP/dry" "would write and start 2 LaunchAgents" "a macOS dry run plans the services"
  hasnt "$TMP/dry" "com.harbard.console" "a macOS dry run plans no console"
  hasnt "$TMP/dry" "bun" "the app needs no bun"
  has "$TMP/dry" "Build and install the desktop app" "a macOS dry run plans the app"
fi
if command -v dash >/dev/null; then
  RUN_SH=dash run "$H" "$TMP/dry-dash" --dry-run --no-app || { cat "$TMP/dry-dash" >&2; fail "dash dry run"; }
  [ "$before" = "$(snapshot "$H")" ] || fail "--dry-run under dash changed the home"
fi
ok "--dry-run prints the plan and changes nothing"

# --- install, with every harness present ---------------------------------------
H="$TMP/h-main"
harnesses "$H"
printf '{"theme":"dark"}\n' >"$H/.claude/settings.json"
run "$H" "$TMP/install" --no-app || { cat "$TMP/install" >&2; fail "install exited non-zero"; }
BIN="$H/.local/bin/harbard"
[ -x "$BIN" ] || fail "the binary did not land at $BIN"
[ "$("$BIN" --version)" = "$VERSION_LINE" ] || fail "the installed binary does not run"
if [ "$(uname -s)" = "Darwin" ]; then
  codesign --verify --strict "$BIN" 2>/dev/null || fail "the installed binary is not signed"
fi
grep -q harbard "$H/.claude/settings.json" || fail "Claude Code was not wired"
grep -q theme "$H/.claude/settings.json" || fail "Claude Code's own settings were lost"
grep -q harbard "$H/.codex/hooks.json" || fail "Codex was not wired"
grep -q harbard "$H/.cursor/hooks.json" || fail "Cursor was not wired"
LOG="$(ls "$H"/.harbard/logs/install-*.log)"
sed -n -e 's/^  wrote //p' -e 's/^  backed up to //p' "$LOG" >"$TMP/written"
[ "$(wc -l <"$TMP/written")" -ge 4 ] || fail "setup's log does not list what it wrote"
while IFS= read -r written; do
  case "$written" in "$H"/*) ;; *) fail "setup wrote outside the test home: $written" ;; esac
done <"$TMP/written"
has "$TMP/install" "Claude Code  wired" "the summary of wiring"
has "$TMP/install" "Cursor       wired" "the summary of wiring"
has "$TMP/install" "is installed." "the closing panel"
has "$TMP/install" "--uninstall" "the closing panel says how to uninstall"
has "$TMP/install" "[1/4] Download and verify" "plain numbered steps"
if grep -q $'\033' "$TMP/install"; then fail "escape codes in output that is not a terminal"; fi
[ "$(grep -cFx '# Added by the harbard installer' "$H/.zshrc")" = 1 ] || fail "PATH not added to .zshrc once"
has "$H/.zshrc" 'export PATH="$HOME/.local/bin:$PATH"' "the PATH line"
has "$H/.harbard/install-receipt" "bin=$BIN" "the receipt"
hasnt "$H/.harbard/install-receipt" "agent=" "a --no-app install started no service"
hasnt "$H/.harbard/install-receipt" "app=" "a --no-app install installed no app"
[ -z "$(find "$H/.local/bin" -name '.harbard-install.*')" ] || fail "a staging directory was left behind"
ok "install lands a runnable binary, wires the harnesses in the test home, plain output"

# --- re-run is idempotent ------------------------------------------------------
ino="$(inode "$BIN")"
cfg="$(sha "$H/.claude/settings.json") $(sha "$H/.codex/hooks.json") $(sha "$H/.cursor/hooks.json")"
run "$H" "$TMP/again" --no-app || { cat "$TMP/again" >&2; fail "re-run exited non-zero"; }
[ "$(inode "$BIN")" = "$ino" ] || fail "a re-run replaced an up-to-date binary"
[ "$cfg" = "$(sha "$H/.claude/settings.json") $(sha "$H/.codex/hooks.json") $(sha "$H/.cursor/hooks.json")" ] ||
  fail "a re-run rewrote the harness configs"
[ "$(grep -cFx '# Added by the harbard installer' "$H/.zshrc")" = 1 ] || fail "a re-run added PATH again"
has "$TMP/again" "is already installed" "a re-run says the CLI is current"
has "$TMP/again" "already wired" "a re-run says the hooks are wired"
ok "a re-run changes nothing that is current"

# --- a checksum mismatch is refused -------------------------------------------
cp -R "$TMP/dist" "$TMP/dist-bad"
for f in "$TMP"/dist-bad/*.sha256; do
  printf '%064d  %s\n' 0 "$(basename "$f" .sha256)" >"$f"
done
H="$TMP/h-bad"
if RUN_FROM="$TMP/dist-bad" run "$H" "$TMP/bad" --no-app; then fail "a checksum mismatch installed"; fi
has "$TMP/bad" "checksum mismatch" "the refusal says why"
has "$TMP/bad" "Step 1 of 4 failed" "the failure names its step"
has "$TMP/bad" "Full log" "the failure gives the log"
[ ! -e "$H/.local/bin/harbard" ] || fail "a refused archive was installed"
ok "a checksum mismatch is refused before anything is installed"

# --- Linux: no app, and a PATH that already has the directory -------------------
H="$TMP/h-linux"
harnesses "$H"
RUN_OS=Linux RUN_PATH="$H/.local/bin:/usr/bin:/bin:/usr/sbin:/sbin" run "$H" "$TMP/linux" ||
  { cat "$TMP/linux" >&2; fail "Linux install exited non-zero"; }
has "$TMP/linux" "macOS-only" "Linux says the app is macOS-only"
hasnt "$TMP/linux" "LaunchAgents" "Linux plans no services"
[ -x "$H/.local/bin/harbard" ] || fail "Linux install did not land"
[ ! -e "$H/.zshrc" ] || fail "PATH was edited although the directory was already on it"
has "$TMP/linux" "already on your PATH" "PATH left alone"
ok "Linux installs the CLI and hooks and says the app is macOS-only"

# --- services owned by another checkout are left alone (macOS) -------------------
if [ "$(uname -s)" = "Darwin" ]; then
  H="$TMP/h-dev"
  mkdir -p "$H/Library/LaunchAgents"
  printf '<plist><dict><key>Label</key><string>com.harbard.serve</string><key>WorkingDirectory</key><string>/somewhere/harbard</string></dict></plist>\n' \
    >"$H/Library/LaunchAgents/com.harbard.serve.plist"
  if run "$H" "$TMP/dev"; then fail "the installer took over another checkout's services"; fi
  has "$TMP/dev" "already run from /somewhere/harbard" "the refusal names the checkout"
  [ ! -e "$H/.local" ] || fail "the refusal installed something"
  ok "services that run from another checkout are not replaced"

  # A Mac from the console era: the plan says the console's agent goes.
  H="$TMP/h-console"
  mkdir -p "$H/Library/LaunchAgents"
  printf '<plist><dict><key>Label</key><string>com.harbard.console</string><key>WorkingDirectory</key><string>/somewhere/harbard/apps/console</string></dict></plist>\n' \
    >"$H/Library/LaunchAgents/com.harbard.console.plist"
  run "$H" "$TMP/console" --dry-run || { cat "$TMP/console" >&2; fail "a dry run with the console's agent"; }
  has "$TMP/console" "would stop and delete com.harbard.console" "the plan retires the console's agent"
  [ -f "$H/Library/LaunchAgents/com.harbard.console.plist" ] || fail "a dry run deleted the console's plist"
  ok "a dry run on a console-era Mac plans the console's retirement and changes nothing"
fi

# --- a terminal gets the live UI; NO_COLOR keeps it but drops colour ------------
if [ "$(uname -s)" = "Darwin" ]; then
  tty_run() { script -q /dev/null "$@"; }
else
  tty_run() { script -qec "$(printf '%q ' "$@")" /dev/null; }
fi
if command -v script >/dev/null; then
  H="$TMP/h-tty"
  harnesses "$H"
  tty_run env -i HOME="$H" PATH=/usr/bin:/bin:/usr/sbin:/sbin SHELL=/bin/zsh LANG=en_US.UTF-8 \
    TERM=xterm-256color TMPDIR="$TMP/t" HARBARD_INSTALL_FROM="$TMP/dist" HARBARD_VERSION=v0.0.0-test \
    sh "$INSTALLER" --dry-run --no-app >"$TMP/tty" 2>&1 </dev/null || true
  grep -q $'\033\\[3[0-9]m' "$TMP/tty" || { cat "$TMP/tty" >&2; fail "no colour on a terminal"; }
  has "$TMP/tty" "○" "step marks on a terminal"
  hasnt "$TMP/tty" "[1/4]" "plain step lines on a terminal"
  tty_run env -i HOME="$H" PATH=/usr/bin:/bin:/usr/sbin:/sbin SHELL=/bin/zsh LANG=en_US.UTF-8 NO_COLOR=1 \
    TERM=xterm-256color TMPDIR="$TMP/t" HARBARD_INSTALL_FROM="$TMP/dist" HARBARD_VERSION=v0.0.0-test \
    sh "$INSTALLER" --dry-run --no-app >"$TMP/tty-nc" 2>&1 </dev/null || true
  has "$TMP/tty-nc" "○" "step marks without colour"
  hasnt "$TMP/tty-nc" $'\033[3' "NO_COLOR"
  ok "a terminal gets the live UI, and NO_COLOR drops the colour"
fi

# --- uninstall removes what the install added; --purge removes the data ---------
H="$TMP/h-main"
run "$H" "$TMP/un-dry" --uninstall --dry-run || { cat "$TMP/un-dry" >&2; fail "uninstall dry run"; }
[ -x "$BIN" ] || fail "an uninstall dry run removed the binary"
has "$TMP/un-dry" 'would delete ~/.local/bin/harbard' "the uninstall plan"
run "$H" "$TMP/un" --uninstall || { cat "$TMP/un" >&2; fail "uninstall exited non-zero"; }
[ ! -e "$BIN" ] || fail "the binary is still there"
for f in "$H/.claude/settings.json" "$H/.codex/hooks.json" "$H/.cursor/hooks.json"; do
  if grep -q harbard "$f"; then fail "hooks left in $f"; fi
done
grep -q theme "$H/.claude/settings.json" || fail "uninstall lost Claude Code's own settings"
hasnt "$H/.zshrc" "harbard installer" "the PATH line is gone"
[ -d "$H/.harbard" ] || fail "uninstall deleted the data without --purge"
[ ! -e "$H/.harbard/install-receipt" ] || fail "the receipt outlived the uninstall"
has "$TMP/un" "harbard is uninstalled" "the closing line"
run "$H" "$TMP/purge" --uninstall --purge --yes || { cat "$TMP/purge" >&2; fail "purge exited non-zero"; }
[ ! -e "$H/.harbard" ] || fail "--purge kept the data"
ok "--uninstall removes the binary, hooks and PATH line; --purge removes the data"

# --- unit: the functions behind the steps a test cannot run --------------------
# lib <home> <snippet>: the installer's functions, loaded by sh in a throwaway
# HOME, then the snippet. Nothing here loads a LaunchAgent or installs the app.
lib() {
  mkdir -p "$1" "$TMP/t"
  env -i HOME="$1" PATH="/usr/bin:/bin:/usr/sbin:/sbin" SHELL="${LIB_SHELL:-/bin/zsh}" LANG=C \
    TMPDIR="$TMP/t" HARBARD_INSTALL_SOURCE_ONLY=1 INSTALLER="$INSTALLER" TEST_TMP="$TMP" sh -c '
      . "$INSTALLER"
      exec 3>&1
      ui_init
      DRY_RUN=0; YES=1; MODIFY_PATH=1; OS=macos; STEP_DETAILS=""; STEP_STATUS=ok; STEP_FAIL=""; STEP_HINT=""
      resolve_paths
      WORK="$(mktemp -d)"; LOG="$WORK/log"; : >"$LOG"
      '"$2"
}

# PATH, per shell, once, and removed to the byte.
H="$TMP/h-unit-path"
mkdir -p "$H"
printf "alias ll='ls -l'\n" >"$H/.bashrc"
cp "$H/.bashrc" "$TMP/bashrc.before"
LIB_SHELL=/bin/bash lib "$H" 'OS=linux; path_setup; path_setup' >/dev/null
[ "$(grep -cFx '# Added by the harbard installer' "$H/.bashrc")" = 1 ] || fail "bash on Linux: PATH not in .bashrc once"
LIB_SHELL=/bin/bash lib "$H" 'path_setup' >/dev/null
has "$H/.bash_profile" 'export PATH="$HOME/.local/bin:$PATH"' "bash on macOS writes .bash_profile"
LIB_SHELL=/opt/homebrew/bin/fish lib "$H" 'path_setup' >/dev/null
has "$H/.config/fish/conf.d/harbard.fish" "fish_add_path \"$H/.local/bin\"" "fish gets fish_add_path"
lib "$H" 'step_uncli' >/dev/null
cmp -s "$H/.bashrc" "$TMP/bashrc.before" || fail "uninstall did not restore .bashrc to the byte"
hasnt "$H/.bash_profile" "harbard" "uninstall cleans .bash_profile"
[ ! -e "$H/.config/fish/conf.d/harbard.fish" ] || fail "uninstall left the fish file"
ok "PATH goes once into the right file for zsh, bash and fish, and comes out cleanly"

# Services that belong to this checkout are ours to update; others are not.
H="$TMP/h-unit-owner"
mkdir -p "$H/Library/LaunchAgents"
plist() {
  printf '<?xml version="1.0" encoding="UTF-8"?>\n<plist version="1.0">\n<dict>\n  <key>Label</key>\n  <string>%s</string>\n  <key>WorkingDirectory</key>\n  <string>%s</string>\n</dict>\n</plist>\n' "$1" "$2" \
    >"$H/Library/LaunchAgents/$1.plist"
}
plist com.harbard.serve "$H/Library/Application Support/harbard/source"
[ "$(lib "$H" 'check_service_owner; printf "[%s]" "$FOREIGN_SERVICES"')" = "[]" ] ||
  fail "the installer's own services read as another checkout's"
plist com.harbard.console "/work/harbard/apps/console"
[ "$(lib "$H" 'check_service_owner; printf "[%s]" "$FOREIGN_SERVICES"')" = "[]" ] ||
  fail "a retired console's agent made the services read as another checkout's"
plist com.harbard.serve "/work/harbard"
[ "$(lib "$H" 'check_service_owner; printf "[%s]" "$FOREIGN_SERVICES"')" = "[/work/harbard]" ] ||
  fail "another checkout's services were not recognised"
ok "the installer tells its own services from a source checkout's"

# The source step: clone at the release, fast-forward to the next, refuse local
# changes, and point `harbard update` at the checkout (path with a space).
O="$TMP/upstream"
g() { git -c user.name=t -c user.email=t@t -C "$O" "$@"; }
mkdir -p "$O"
g init -q -b main
printf '[workspace]\n' >"$O/Cargo.toml"
g add -A && g commit -q -m "chore: one" && g tag v0.0.1
printf 'x\n' >"$O/a.txt"
g add -A && g commit -q -m "feat: two" && g tag v0.0.2
git clone -q --bare "$O" "$TMP/origin.git"
H="$TMP/h-unit-src"
mkdir -p "$H/.local/bin"
cp "$BIN_SRC" "$H/.local/bin/harbard"
SRC="$H/Library/Application Support/harbard/source"
src_step() { lib "$H" "GIT_URL=\"file://\$TEST_TMP/origin.git\"; VERSION=$1; step_source; echo \"rc=\$? changed=\$SOURCE_CHANGED fail=\$STEP_FAIL\""; }
src_step v0.0.1 >"$TMP/src1"
has "$TMP/src1" "rc=0 changed=1" "the first source step clones"
[ "$(git -C "$SRC" rev-parse HEAD)" = "$(g rev-parse v0.0.1)" ] || fail "the clone is not at the release"
[ "$(git -C "$SRC" symbolic-ref --short HEAD)" = main ] || fail "the clone is not on main"
[ "$(git -C "$SRC" rev-parse --abbrev-ref 'main@{upstream}')" = origin/main ] || fail "main does not track origin/main"
has "$H/.harbard/update.json" '"channel": "releases"' "update follows releases"
has "$H/.harbard/update.json" "\"install_dir\": \"$H/.local/bin\"" "update installs where the installer did"
has "$H/.harbard/update.json" "Application Support/harbard/source" "update builds from the checkout"
src_step v0.0.2 >"$TMP/src2"
has "$TMP/src2" "rc=0 changed=1" "a newer release fast-forwards"
[ "$(git -C "$SRC" rev-parse HEAD)" = "$(g rev-parse v0.0.2)" ] || fail "the checkout did not move to the new release"
src_step v0.0.2 >"$TMP/src3"
has "$TMP/src3" "rc=0 changed=0" "the same release changes nothing"
printf 'edited\n' >"$SRC/a.txt"
src_step v0.0.2 >"$TMP/src4" || true
has "$TMP/src4" "rc=1" "local changes stop the step"
has "$TMP/src4" "has local changes" "and say why"
ok "the source step clones, updates, and refuses local changes"

echo "ok: install.sh"
