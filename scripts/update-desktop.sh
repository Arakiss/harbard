#!/usr/bin/env bash
# Build the desktop app, then replace the installed copies, verified.
#
# Nothing is replaced until the new bundle exists and its source receipt
# holds. The receipt ties the source fingerprint to the bundle and to every
# installed copy; it stays in target/desktop-installs/.
#
#   HARBARD_QUIT_APP=1   quit an open installed app first (what `harbard update` sets)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
DESKTOP_DIR="$ROOT/apps/desktop"
[[ "$(uname -s)" == "Darwin" ]] || { echo "error: desktop update requires macOS" >&2; exit 1; }

RECEIPT_DIR="$ROOT/target/desktop-installs"
mkdir -p "$RECEIPT_DIR"
RECEIPT="$RECEIPT_DIR/$(date -u +%Y%m%dT%H%M%SZ)-$$.json"
python3 "$ROOT/scripts/desktop-runtime.py" capture "$ROOT" "$RECEIPT"

echo "harbard update — building the desktop app before replacement"
BUNDLE="$(HARBARD_BUILD_RECEIPT="$RECEIPT" bash "$DESKTOP_DIR/scripts/bundle.sh")"
[[ -d "$BUNDLE" ]] || { echo "error: the build named no bundle" >&2; exit 1; }

# Installed apps must already be quit by the native caller. The installer
# refuses mapped copies and retains recoverable backups; it never quits UI.
# HARBARD_QUIT_APP=1 (what `harbard update` sets) makes this script that
# native caller: once the build exists, it asks an open app to quit through
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
HARBARD_DESKTOP_SKIP_BUILD=1 HARBARD_OPEN_APP=0 HARBARD_APP_BUNDLE="$BUNDLE" HARBARD_BUILD_RECEIPT="$RECEIPT" \
  bash "$DESKTOP_DIR/scripts/install.sh"
python3 "$ROOT/scripts/desktop-runtime.py" verify "$ROOT" "$RECEIPT"

# The app no longer needs the web console; a Mac that ran it still has its
# LaunchAgent. Only now, with the new app in place, is it safe to stop.
bash "$ROOT/scripts/retire-console.sh"
echo "harbard update — installed and verified; receipt: $RECEIPT"
echo "harbard update — reopen the installed app through native app control"
