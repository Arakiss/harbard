#!/usr/bin/env bash
# install-cli.sh must replace the binary with a new inode, never rewrite the
# running one in place. Runs against a temp directory with fake binaries; never
# touches ~/.cargo/bin.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail() { echo "FAIL: $*" >&2; exit 1; }
inode() { stat -f %i "$1" 2>/dev/null || stat -c %i "$1"; }

printf '#!/bin/sh\necho old\n' >"$TMP/old"
printf '#!/bin/sh\necho new\n' >"$TMP/new"
DEST="$TMP/bin/harbard"

HARBARD_INSTALL_DIR="$TMP/bin" HARBARD_INSTALL_SRC="$TMP/old" bash "$ROOT/scripts/install-cli.sh" >/dev/null
[ "$("$DEST")" = "old" ] || fail "first install did not land"
before="$(inode "$DEST")"

# Hold the old file open, the way a running process holds its executable.
exec 3<"$DEST"
HARBARD_INSTALL_DIR="$TMP/bin" HARBARD_INSTALL_SRC="$TMP/new" bash "$ROOT/scripts/install-cli.sh" >/dev/null
after="$(inode "$DEST")"

[ "$before" != "$after" ] || fail "binary was rewritten on the same inode ($before)"
[ "$("$DEST")" = "new" ] || fail "second install did not land"
grep -q "echo old" <&3 || fail "the open old inode was modified"
exec 3<&-
[ -x "$DEST" ] || fail "installed binary is not executable"
leftover="$(find "$TMP/bin" -name '.harbard-install.*' | wc -l | tr -d ' ')"
[ "$leftover" = "0" ] || fail "staging directory left behind"

echo "ok: install-cli replaces the binary with a new inode"
