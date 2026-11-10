#!/usr/bin/env bash
# Build the native app in release and assemble harbard.app, ad-hoc signed.
# Installs nothing: apps/desktop/scripts/install.sh does that.
#
#   apps/desktop/scripts/bundle.sh     progress on stderr, the bundle's path on stdout
#
# The bundle lands in <cargo target dir>/release/bundle/macos/harbard.app:
#
#   Contents/MacOS/harbard-desktop     the release binary harbard-app, under the
#                                      executable name every script, the update
#                                      run and the uninstaller match the app by
#   Contents/Info.plist                bundle/Info.plist, version from Cargo.toml
#   Contents/Resources/icon.icns       the classic icon
#   Contents/Resources/Assets.car      the macOS 26 layered icon (CFBundleIconName)
#   Contents/Resources/harbard-build.json  which source it was built from
#
# Environment:
#   HARBARD_BUILD_RECEIPT   a receipt from `scripts/desktop-runtime.py capture`,
#                           taken before the build: checked against the source
#                           once the build is done, then embedded. Without it
#                           the script captures its own.
set -euo pipefail
APP_ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
REPO="$(cd "$APP_ROOT/../.." && pwd -P)"
cd "$APP_ROOT"
log() { echo "harbard bundle — $*" >&2; }
die() { echo "error: $*" >&2; exit 1; }

[[ "$(uname -s)" == "Darwin" ]] || die "the app bundle is macOS-only"
# rustup's cargo honours apps/desktop/rust-toolchain.toml; a cargo from
# another source first on PATH would build with whatever compiler it has.
if [[ -x "$HOME/.cargo/bin/cargo" ]]; then PATH="$HOME/.cargo/bin:$PATH"; fi
command -v cargo >/dev/null || die "cargo is required; install Rust with rustup (https://rustup.rs)"
command -v codesign >/dev/null || die "codesign is required"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/harbard-bundle.XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

# One read of cargo's own answer, so a target dir set by CARGO_TARGET_DIR or
# a cargo config is the one the binary is found in.
cargo metadata --format-version 1 --no-deps --offline >"$WORK/metadata.json"
# One line each: the default target dir of the installer's checkout sits
# under "Application Support", so a space must not split them.
meta() {
  python3 - "$WORK/metadata.json" "$1" <<'PY'
import json, sys
meta = json.load(open(sys.argv[1]))
package = next(p for p in meta["packages"] if p["name"] == "harbard-app")
print(meta["target_directory"] if sys.argv[2] == "target" else package["version"])
PY
}
TARGET="$(meta target)"
VERSION="$(meta version)"
[[ -n "$TARGET" ]] || die "cargo metadata named no target directory"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || die "unexpected version in Cargo.toml: $VERSION"
BINARY="$TARGET/release/harbard-app"
OUT="$TARGET/release/bundle/macos"
APP="$OUT/harbard.app"

RECEIPT="${HARBARD_BUILD_RECEIPT:-}"
if [[ -z "$RECEIPT" ]]; then
  RECEIPT="$WORK/harbard-build.json"
  python3 "$REPO/scripts/desktop-runtime.py" capture "$REPO" "$RECEIPT"
fi

log "building harbard-app $VERSION (release, $(cargo --version))"
cargo build --release --locked --bin harbard-app >&2
[[ -x "$BINARY" ]] || die "cargo built no binary at $BINARY"
python3 "$REPO/scripts/desktop-runtime.py" built "$REPO" "$RECEIPT" "$VERSION"

log "assembling $APP"
STAGE="$WORK/harbard.app"
mkdir -p "$STAGE/Contents/MacOS" "$STAGE/Contents/Resources"
cp "$BINARY" "$STAGE/Contents/MacOS/harbard-desktop"
sed "s/@VERSION@/$VERSION/g" "$APP_ROOT/bundle/Info.plist" >"$STAGE/Contents/Info.plist"
plutil -lint -s "$STAGE/Contents/Info.plist" >&2 || die "Info.plist does not parse"
printf 'APPL????' >"$STAGE/Contents/PkgInfo"
cp "$APP_ROOT/bundle/icon.icns" "$APP_ROOT/bundle/Assets.car" "$STAGE/Contents/Resources/"
cp "$RECEIPT" "$STAGE/Contents/Resources/harbard-build.json"
codesign --force --deep --sign - "$STAGE" >&2
codesign --verify --deep --strict "$STAGE" >&2

mkdir -p "$OUT"
rm -rf "$APP"
mv "$STAGE" "$APP"
log "built $APP"
printf '%s\n' "$APP"
