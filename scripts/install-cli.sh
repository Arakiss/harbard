#!/usr/bin/env bash
# Install the harbard CLI so a running copy is never overwritten in place.
#
# The new binary is built into a staging directory beside the destination,
# ad-hoc signed there, and renamed over the old path: a new inode, swapped
# atomically, so a hook never runs a half-written or unsigned file and running
# processes keep the old one until they exit.
#
# Signing. macOS ties what the operator allowed (a removable volume, Full
# Disk Access, Background Task Management's launch constraint) to the
# binary's designated requirement. An ad-hoc signature's requirement is its
# own hash, so a grant does not carry over to the next build. A certificate identity gives a
# requirement of "this identifier, signed by this certificate", the same for
# every build, so one answer holds. The identity is read from
# HARBARD_CODESIGN_IDENTITY, else from {harbard home}/codesign-identity (one
# line, a name `security find-identity -v -p codesigning` lists). Without
# one, or if it no longer signs, the build is signed ad hoc as before and the
# script says so.
#
# --locked builds the versions in Cargo.lock, the ones CI tested.
#
#   HARBARD_INSTALL_DIR  destination directory (default: $CARGO_HOME/bin)
#   HARBARD_INSTALL_SRC  install this prebuilt binary instead of building
#                        (used by scripts/test-install-cli.sh)
#   HARBARD_CODESIGN_IDENTITY  code-signing identity (default: the file above)
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST_DIR="${HARBARD_INSTALL_DIR:-${CARGO_HOME:-$HOME/.cargo}/bin}"
DEST="$DEST_DIR/harbard"

mkdir -p "$DEST_DIR"
# Same directory, same filesystem: the final mv is a rename(2), never a copy.
STAGE="$(mktemp -d "$DEST_DIR/.harbard-install.XXXXXX")"
trap 'rm -rf "$STAGE"' EXIT

if [ -n "${HARBARD_INSTALL_SRC:-}" ]; then
  mkdir -p "$STAGE/bin"
  cp "$HARBARD_INSTALL_SRC" "$STAGE/bin/harbard"
else
  cargo install --locked --path "$ROOT/crates/harbard-cli" --force --root "$STAGE"
fi
NEW="$STAGE/bin/harbard"
chmod 0755 "$NEW"

# Sign before the rename so the destination path never holds an unsigned file.
if command -v codesign >/dev/null 2>&1; then
  IDENTITY="${HARBARD_CODESIGN_IDENTITY:-}"
  ID_FILE="${HARBARD_HOME:-$HOME/.harbard}/codesign-identity"
  if [ -z "$IDENTITY" ] && [ -r "$ID_FILE" ]; then
    IDENTITY="$(head -n 1 "$ID_FILE" | tr -d '\r')"
  fi
  if [ -n "$IDENTITY" ] && codesign --force --identifier com.harbard.cli --sign "$IDENTITY" "$NEW" 2>/dev/null; then
    echo "signed: $IDENTITY (stable across builds)"
  else
    [ -n "$IDENTITY" ] && echo "warning: identity \"$IDENTITY\" did not sign; signing ad hoc" >&2
    codesign --force --sign - "$NEW"
    echo "signed: ad hoc (macOS asks again for each build's permissions)"
  fi
fi

mv -f "$NEW" "$DEST"
echo "installed: $DEST"
