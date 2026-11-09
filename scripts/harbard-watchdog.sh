#!/usr/bin/env bash
# Recover serve only after three consecutive authenticated health failures.
set -euo pipefail

PORT="${HARBARD_API_PORT:-7419}"
FAILURES_REQUIRED="${HARBARD_WATCHDOG_FAILURES:-3}"
STATE_FILE="${HARBARD_WATCHDOG_STATE:-$HOME/.harbard/watchdog.state}"
LOG_DIR="${HARBARD_LOG_DIR:-$HOME/.harbard/logs}"
LABEL="com.harbard.serve"
DOMAIN="gui/$(id -u)/$LABEL"
URL="http://127.0.0.1:$PORT/api/v1/health"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CURL_BIN="${HARBARD_WATCHDOG_CURL:-curl}"
LAUNCHCTL_BIN="${HARBARD_WATCHDOG_LAUNCHCTL:-launchctl}"
SLEEP_BIN="${HARBARD_WATCHDOG_SLEEP:-sleep}"

mkdir -p "$(dirname "$STATE_FILE")" "$LOG_DIR"
"$ROOT/scripts/rotate-logs.sh" \
  "$LOG_DIR/com.harbard.serve.log" \
  "$LOG_DIR/com.harbard.serve.err.log" \
  "$LOG_DIR/com.harbard.watchdog.log" \
  "$LOG_DIR/com.harbard.watchdog.err.log"

probe() {
  if [ -s "$HOME/.harbard/hub.token" ]; then
    "$CURL_BIN" --connect-timeout 1 --max-time 2 -fsS -o /dev/null \
      -H @<(printf 'Authorization: Bearer %s\n' "$(<"$HOME/.harbard/hub.token")") "$URL"
  else
    "$CURL_BIN" --connect-timeout 1 --max-time 2 -fsS -o /dev/null "$URL"
  fi
}

serve_pid() {
  "$LAUNCHCTL_BIN" print "$DOMAIN" 2>/dev/null | awk '/^[[:space:]]*pid = / { print $3; exit }'
}

log_event() {
  printf '{"ts":"%s","event":"%s","failures":%s,"old_pid":"%s","new_pid":"%s"}\n' \
    "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$1" "${2:-0}" "${3:-}" "${4:-}"
}

if probe; then
  printf '0\n' > "$STATE_FILE"
  exit 0
fi

failures="$(cat "$STATE_FILE" 2>/dev/null || echo 0)"
case "$failures" in (*[!0-9]*|'') failures=0 ;; esac
failures=$((failures + 1))
printf '%s\n' "$failures" > "$STATE_FILE"
log_event "probe_failed" "$failures"
[ "$failures" -ge "$FAILURES_REQUIRED" ] || exit 0

old_pid="$(serve_pid || true)"
"$LAUNCHCTL_BIN" kickstart -k "$DOMAIN"

for _attempt in $(seq 1 20); do
  "$SLEEP_BIN" 0.25
  new_pid="$(serve_pid || true)"
  if [ -n "$new_pid" ] && [ "$new_pid" != "$old_pid" ] && probe; then
    printf '0\n' > "$STATE_FILE"
    log_event "recovered" 0 "$old_pid" "$new_pid"
    exit 0
  fi
done

new_pid="$(serve_pid || true)"
log_event "recovery_failed" "$failures" "$old_pid" "$new_pid"
exit 1
