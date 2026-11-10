#!/usr/bin/env bash
# Retire the web console's LaunchAgent on a Mac that still has it.
#
# The desktop app used to show a Next.js console served by the LaunchAgent
# com.harbard.console on 127.0.0.1:3419. The app now draws everything itself
# and the console's code is gone, so that agent would only keep restarting a
# server that cannot start. This stops it and deletes its plist. Idempotent:
# on a Mac without it, it does nothing.
#
# Called by scripts/update-desktop.sh once the new app is installed, and by
# scripts/install-launchagents.sh.
set -euo pipefail

[[ "$(uname -s)" == "Darwin" ]] || exit 0
LABEL="com.harbard.console"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
SERVICE="gui/$(id -u)/$LABEL"

if launchctl print "$SERVICE" >/dev/null 2>&1; then
  if launchctl bootout "$SERVICE" >/dev/null 2>&1; then
    echo "stopped $LABEL (the web console is retired)"
  else
    echo "warning: could not stop $LABEL; it stops at the next login once its plist is gone" >&2
  fi
fi
if [[ -f "$PLIST" ]]; then
  rm -f "$PLIST"
  echo "deleted $PLIST"
fi
