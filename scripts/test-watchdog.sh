#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d "${TMPDIR:-/tmp}/harbard-watchdog-test.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/bin" "$TMP/home/.harbard" "$TMP/logs"
printf '111\n' > "$TMP/pid"

cat > "$TMP/bin/curl" <<'EOF'
#!/usr/bin/env bash
[ -f "$HARBARD_TEST_HEALTH" ]
EOF

cat > "$TMP/bin/launchctl" <<'EOF'
#!/usr/bin/env bash
if [ "$1" = "print" ]; then
  printf '    pid = %s\n' "$(cat "$HARBARD_TEST_PID")"
  exit 0
fi
if [ "$1" = "kickstart" ]; then
  printf '222\n' > "$HARBARD_TEST_PID"
  touch "$HARBARD_TEST_HEALTH"
  count="$(cat "$HARBARD_TEST_KICKS" 2>/dev/null || echo 0)"
  printf '%s\n' "$((count + 1))" > "$HARBARD_TEST_KICKS"
  exit 0
fi
exit 1
EOF

cat > "$TMP/bin/sleep" <<'EOF'
#!/usr/bin/env bash
exit 0
EOF
chmod +x "$TMP/bin/curl" "$TMP/bin/launchctl" "$TMP/bin/sleep"

run_watchdog() {
  HOME="$TMP/home" \
  HARBARD_LOG_DIR="$TMP/logs" \
  HARBARD_WATCHDOG_STATE="$TMP/state" \
  HARBARD_WATCHDOG_CURL="$TMP/bin/curl" \
  HARBARD_WATCHDOG_LAUNCHCTL="$TMP/bin/launchctl" \
  HARBARD_WATCHDOG_SLEEP="$TMP/bin/sleep" \
  HARBARD_TEST_HEALTH="$TMP/healthy" \
  HARBARD_TEST_PID="$TMP/pid" \
  HARBARD_TEST_KICKS="$TMP/kicks" \
  "$ROOT/scripts/harbard-watchdog.sh"
}

run_watchdog >/dev/null
run_watchdog >/dev/null
[ ! -f "$TMP/kicks" ] || { echo "watchdog restarted before threshold" >&2; exit 1; }
run_watchdog > "$TMP/recovery.log"
[ "$(cat "$TMP/kicks")" = "1" ]
[ "$(cat "$TMP/state")" = "0" ]
grep -q '"event":"recovered"' "$TMP/recovery.log"
echo "watchdog: threshold and verified recovery passed"
