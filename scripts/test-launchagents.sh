#!/usr/bin/env bash
# scripts/install-launchagents.sh in a throwaway HOME, with launchctl, curl
# and harbard simulated: never the Mac's own LaunchAgents or the live serve.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/harbard-agents-test.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
fail() { echo "FAIL: $*" >&2; exit 1; }
mkdir -p "$TMP/bin" "$TMP/home/Library/LaunchAgents"

shim() { printf '#!/usr/bin/env bash\n%s\n' "$2" >"$TMP/bin/$1"; chmod +x "$TMP/bin/$1"; }
shim uname 'echo Darwin'
shim harbard 'exit 0'
shim curl 'exit 0'
shim bun 'touch "$TEST_TMP/bun-called"; exit 99'
# shellcheck disable=SC2016 # expanded by the shim, not here
shim launchctl 'printf "%s\n" "$*" >>"$TEST_TMP/launchctl.log"
case "$1" in
  print) [[ "$2" == */com.harbard.console && -f "$TEST_TMP/console-loaded" ]] ;;
  bootout|bootstrap|kickstart) exit 0 ;;
  *) exit 99 ;;
esac'

agents() {
  env -i HOME="$TMP/home" PATH="$TMP/bin:/usr/bin:/bin:/usr/sbin:/sbin" TEST_TMP="$TMP" \
    HARBARD_API_PORT=1 bash "$ROOT/scripts/install-launchagents.sh" >"$TMP/out" 2>&1 ||
    { cat "$TMP/out" >&2; fail "install-launchagents.sh exited non-zero"; }
}

# A Mac upgraded from the console era: its agent is stopped and deleted.
AGENTS="$TMP/home/Library/LaunchAgents"
printf '<plist/>\n' >"$AGENTS/com.harbard.console.plist"
touch "$TMP/console-loaded"
agents
[ ! -e "$AGENTS/com.harbard.console.plist" ] || fail "the console's plist outlived the install"
grep -Fqx "bootout gui/$(id -u)/com.harbard.console" "$TMP/launchctl.log" || fail "the console's agent was not stopped"
for label in com.harbard.serve com.harbard.watchdog; do
  plutil -lint -s "$AGENTS/$label.plist" || fail "$label.plist does not parse"
  grep -Fq "bootstrap gui/$(id -u) $AGENTS/$label.plist" "$TMP/launchctl.log" || fail "$label was not loaded"
done
[ "$(find "$AGENTS" -name '*.plist' | wc -l | tr -d ' ')" = 2 ] || fail "expected exactly serve and watchdog"
if grep -q console "$AGENTS"/*.plist; then fail "a plist still names the console"; fi
[ ! -e "$TMP/bun-called" ] || fail "bun was run"
grep -q "api     ready" "$TMP/out" || fail "the API probe did not report"
echo "ok: an upgraded Mac loses the console's agent and gets serve and the watchdog"

# A fresh Mac: no console to retire, nothing stopped that was not there.
rm -f "$TMP/console-loaded" "$TMP/launchctl.log"
agents
if grep -q '^bootout gui/[0-9]*/com.harbard.console' "$TMP/launchctl.log"; then fail "a fresh Mac had a console stopped"; fi
echo "ok: a fresh Mac gets serve and the watchdog, and nothing else"
