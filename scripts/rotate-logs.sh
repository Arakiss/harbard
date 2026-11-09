#!/usr/bin/env bash
# Bound local Harbard logs. launchd keeps its descriptor open, so each log is
# copied and truncated in place: renaming it would leave launchd writing to the copy.
set -euo pipefail

MAX_BYTES="${HARBARD_LOG_MAX_BYTES:-5242880}"
KEEP="${HARBARD_LOG_GENERATIONS:-5}"

[ "$#" -gt 0 ] || exit 0

for path in "$@"; do
  [ -f "$path" ] || continue
  size="$(stat -f %z "$path" 2>/dev/null || stat -c %s "$path" 2>/dev/null || echo 0)"
  [ "$size" -ge "$MAX_BYTES" ] || continue
  rm -f -- "$path.$KEEP"
  index=$((KEEP - 1))
  while [ "$index" -ge 1 ]; do
    [ ! -f "$path.$index" ] || mv -f -- "$path.$index" "$path.$((index + 1))"
    index=$((index - 1))
  done
  cp -f -- "$path" "$path.1"
  : > "$path"
done
