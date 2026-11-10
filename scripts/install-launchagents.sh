#!/usr/bin/env bash
# Install the macOS LaunchAgents the desktop app depends on.
#
#   com.harbard.serve    the bus API on 127.0.0.1:7419 (harbard serve)
#   com.harbard.watchdog verifies serve and recovers it after three failures
#
# The app reads the bus itself and writes through serve; it needs no server
# of its own. A Mac that still has the retired web console's agent
# (com.harbard.console) has it stopped and deleted here.
#
# Idempotent: re-running rewrites both plists and kickstarts the jobs.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
AGENT_DIR="$HOME/Library/LaunchAgents"
LOG_DIR="$HOME/.harbard/logs"
UID_NUM="$(id -u)"

SERVE_LABEL="com.harbard.serve"
WATCHDOG_LABEL="com.harbard.watchdog"
SERVE_PORT="${HARBARD_API_PORT:-7419}"

[ "$(uname -s)" = "Darwin" ] || { echo "error: macOS only" >&2; exit 1; }

HARBARD_BIN="$(command -v harbard || true)"
[ -n "$HARBARD_BIN" ] || { echo "error: harbard is not on PATH; run 'just install-cli' first" >&2; exit 1; }

mkdir -p "$AGENT_DIR" "$LOG_DIR"

# A LaunchAgent gets almost no environment: serve runs git for "Deliveries"
# and the watchdog runs curl, so PATH is spelled out.
AGENT_PATH="/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin"

write_plist() {
  local label="$1" workdir="$2" out="$AGENT_DIR/$1.plist"
  shift 2
  {
    printf '<?xml version="1.0" encoding="UTF-8"?>\n'
    printf '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n'
    printf '<plist version="1.0">\n<dict>\n'
    printf '  <key>Label</key><string>%s</string>\n' "$label"
    printf '  <key>ProgramArguments</key>\n  <array>\n'
    for arg in "$@"; do printf '    <string>%s</string>\n' "$arg"; done
    printf '  </array>\n'
    printf '  <key>WorkingDirectory</key><string>%s</string>\n' "$workdir"
    printf '  <key>EnvironmentVariables</key>\n  <dict>\n'
    printf '    <key>PATH</key><string>%s</string>\n' "$AGENT_PATH"
    printf '    <key>HOME</key><string>%s</string>\n' "$HOME"
    printf '  </dict>\n'
    printf '  <key>RunAtLoad</key><true/>\n'
    printf '  <key>KeepAlive</key><true/>\n'
    printf '  <key>StandardOutPath</key><string>%s/%s.log</string>\n' "$LOG_DIR" "$label"
    printf '  <key>StandardErrorPath</key><string>%s/%s.err.log</string>\n' "$LOG_DIR" "$label"
    printf '</dict>\n</plist>\n'
  } > "$out"
  echo "wrote $out"
}

write_watchdog_plist() {
  local out="$AGENT_DIR/$WATCHDOG_LABEL.plist"
  {
    printf '<?xml version="1.0" encoding="UTF-8"?>\n'
    printf '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n'
    printf '<plist version="1.0">\n<dict>\n'
    printf '  <key>Label</key><string>%s</string>\n' "$WATCHDOG_LABEL"
    printf '  <key>ProgramArguments</key><array><string>%s/scripts/harbard-watchdog.sh</string></array>\n' "$ROOT"
    printf '  <key>WorkingDirectory</key><string>%s</string>\n' "$ROOT"
    printf '  <key>EnvironmentVariables</key><dict><key>PATH</key><string>%s</string><key>HOME</key><string>%s</string></dict>\n' "$AGENT_PATH" "$HOME"
    printf '  <key>RunAtLoad</key><true/>\n'
    printf '  <key>StartInterval</key><integer>20</integer>\n'
    printf '  <key>ProcessType</key><string>Background</string>\n'
    printf '  <key>StandardOutPath</key><string>%s/%s.log</string>\n' "$LOG_DIR" "$WATCHDOG_LABEL"
    printf '  <key>StandardErrorPath</key><string>%s/%s.err.log</string>\n' "$LOG_DIR" "$WATCHDOG_LABEL"
    printf '</dict>\n</plist>\n'
  } > "$out"
  echo "wrote $out"
}

load_agent() {
  local label="$1" domain="gui/$UID_NUM/$1"
  launchctl bootout "gui/$UID_NUM" "$AGENT_DIR/$label.plist" >/dev/null 2>&1 || true
  launchctl bootstrap "gui/$UID_NUM" "$AGENT_DIR/$label.plist"
  launchctl kickstart -k "$domain" >/dev/null 2>&1 || true
  echo "loaded $label"
}

bash "$ROOT/scripts/retire-console.sh"
write_plist "$SERVE_LABEL" "$ROOT" "$HARBARD_BIN" serve --port "$SERVE_PORT"
write_watchdog_plist
"$ROOT/scripts/rotate-logs.sh" "$LOG_DIR"/*.log
load_agent "$SERVE_LABEL"
load_agent "$WATCHDOG_LABEL"

# Once ~/.harbard/hub.token exists, `serve` authenticates every route and an
# unauthenticated probe reads 401 — a live API looks dead. (`harbard hub
# status` has this same blind spot: it reports "serve not reachable" against a
# server that is answering fine.)
ready_api() {
  local url="$1" i token=""
  [ -f "$HOME/.harbard/hub.token" ] && token="$(cat "$HOME/.harbard/hub.token")"
  for i in $(seq 1 40); do
    if [ -n "$token" ]; then
      curl -fs -o /dev/null -H @<(printf 'Authorization: Bearer %s\n' "$token") "$url" 2>/dev/null && return 0
    else
      curl -fs -o /dev/null "$url" 2>/dev/null && return 0
    fi
    sleep 0.25
  done
  return 1
}

if ready_api "http://127.0.0.1:$SERVE_PORT/api/v1/health"; then
  echo "api     ready on 127.0.0.1:$SERVE_PORT"
else
  echo "api     NOT answering on 127.0.0.1:$SERVE_PORT — see $LOG_DIR/$SERVE_LABEL.err.log" >&2
  exit 1
fi
