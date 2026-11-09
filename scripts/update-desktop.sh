#!/usr/bin/env bash
# Build before restart, and verify the console actually served by launchd.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
CONSOLE_DIR="$ROOT/apps/console"
DESKTOP_DIR="$ROOT/apps/desktop"
CONSOLE_LABEL="com.harbard.console"
CONSOLE_DOMAIN="gui/$(id -u)/$CONSOLE_LABEL"
CONSOLE_URL="${HARBARD_PRODUCTION_CONSOLE_URL:-http://127.0.0.1:3419}"
[[ "$(uname -s)" == "Darwin" ]] || { echo "error: desktop update requires macOS" >&2; exit 1; }
python3 "$ROOT/scripts/desktop-runtime.py" validate-url "$CONSOLE_URL"
JOB="$(launchctl print "$CONSOLE_DOMAIN")" || {
  echo "error: $CONSOLE_LABEL is not loaded" >&2; exit 1;
}
JOB_CWD="$(printf '%s\n' "$JOB" | sed -n 's/^[[:space:]]*working directory = //p' | head -1)"
[[ -n "$JOB_CWD" && -d "$JOB_CWD" ]] || { echo "error: cannot verify the console LaunchAgent working directory" >&2; exit 1; }
[[ "$(cd "$JOB_CWD" && pwd -P)" == "$(cd "$CONSOLE_DIR" && pwd -P)" ]] || {
  echo "error: this checkout is not the production console working directory; integrate into the persistent primary before updating" >&2; exit 1;
}
BEFORE_PID="$(printf '%s\n' "$JOB" | awk '/^[[:space:]]*pid = / {print $3; exit}')"
RECEIPT_DIR="$ROOT/target/desktop-installs"
mkdir -p "$RECEIPT_DIR"
RECEIPT="$RECEIPT_DIR/$(date -u +%Y%m%dT%H%M%SZ)-$$.json"
python3 "$ROOT/scripts/desktop-runtime.py" capture "$ROOT" "$RECEIPT"

echo "harbard update — building console and desktop before replacement"
(cd "$CONSOLE_DIR" && bun run build)
(cd "$DESKTOP_DIR" && bunx --bun @tauri-apps/cli build --bundles app)
python3 "$ROOT/scripts/desktop-runtime.py" built "$ROOT" "$RECEIPT"

# Installed apps must already be quit by the native caller. The installer
# refuses mapped copies and retains recoverable backups; it never quits UI.
# HARBARD_QUIT_APP=1 (what `harbard update` sets) makes this script that
# native caller: once both builds exist, it asks an open app to quit through
# Apple Events — the same as ⌘Q, never a signal — and waits for it to go.
if [[ "${HARBARD_QUIT_APP:-0}" == "1" ]]; then
  # Not grep -q: it stops at the first match, sed dies of SIGPIPE, and under
  # pipefail an open app read as closed, so the quit was silently skipped.
  app_open() { ps -axo comm= | sed 's/^[[:space:]]*//' | grep -E '/Applications/harbard\.app/Contents/MacOS/harbard-desktop$' >/dev/null; }
  if app_open; then
    echo "harbard update — quitting the open app so it can be replaced"
    osascript -e 'tell application id "com.harbard.desktop" to quit' >/dev/null 2>&1 || true
    QUIT_DEADLINE=$((SECONDS + 30))
    while app_open && (( SECONDS < QUIT_DEADLINE )); do sleep 0.25; done
    app_open && { echo "error: the app did not quit within 30 s; quit it and run the update again" >&2; exit 1; }
  fi
fi
HARBARD_DESKTOP_SKIP_BUILD=1 HARBARD_OPEN_APP=0 HARBARD_BUILD_RECEIPT="$RECEIPT" \
  bash "$DESKTOP_DIR/scripts/install.sh"

echo "harbard update — restarting the verified production console"
launchctl kickstart -k "$CONSOLE_DOMAIN"
# A fresh build usually answers within a second, but the first start after a
# rebuild can take far longer while macOS assesses the new files.
READY=0
DEADLINE=$((SECONDS + ${HARBARD_CONSOLE_READY_SECS:-180}))
NEXT_NOTE=$((SECONDS + 10))
while (( SECONDS < DEADLINE )); do
  if curl -fs --connect-timeout 1 --max-time 2 -o /dev/null "$CONSOLE_URL/" 2>/dev/null; then READY=1; break; fi
  if (( SECONDS >= NEXT_NOTE )); then
    echo "harbard update — console not answering yet ($((DEADLINE - SECONDS)) s left)"
    NEXT_NOTE=$((SECONDS + 15))
  fi
  sleep 0.25
done
[[ "$READY" == "1" ]] || { echo "error: production console did not become ready; app backups are retained" >&2; exit 1; }
AFTER_PID="$(launchctl print "$CONSOLE_DOMAIN" | awk '/^[[:space:]]*pid = / {print $3; exit}')"
[[ -n "$AFTER_PID" && "$AFTER_PID" != "$BEFORE_PID" ]] || {
  echo "error: console LaunchAgent did not acquire a new process" >&2; exit 1;
}
python3 "$ROOT/scripts/desktop-runtime.py" verify "$ROOT" "$RECEIPT" "$CONSOLE_URL" "$AFTER_PID"
echo "harbard update — verified console pid $AFTER_PID; receipt: $RECEIPT"
echo "harbard update — reopen the installed app through native app control and verify both views"
