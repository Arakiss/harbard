#!/usr/bin/env bash
# Stage, sign and replace the app. Native quit/reopen belongs to the caller.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$ROOT"
if [[ -n "${HARBARD_APP_DIR:-}" ]]; then
  APP_DIR="$HARBARD_APP_DIR"
elif [[ -w /Applications ]]; then
  APP_DIR="/Applications"
else
  APP_DIR="$HOME/Applications"
fi
APP_NAME="harbard.app"
BUNDLE="$ROOT/src-tauri/target/release/bundle/macos/$APP_NAME"
LSREGISTER="/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister"
if [[ "${HARBARD_DESKTOP_SKIP_BUILD:-0}" != "1" ]]; then
  echo "harbard desktop — building $APP_NAME"
  bunx --bun @tauri-apps/cli build --bundles app
fi
[[ -f "$BUNDLE/Contents/MacOS/harbard-desktop" ]] || {
  echo "error: expected app executable in $BUNDLE" >&2; exit 1;
}
command -v codesign >/dev/null || { echo "error: codesign is required" >&2; exit 1; }
mkdir -p "$APP_DIR"
APP_DIR="$(cd "$APP_DIR" && pwd -P)"
DEST="$APP_DIR/$APP_NAME"
DIRS=("$APP_DIR")
MIRROR_DIR="${HARBARD_MIRROR_APP_DIR:-}"
if [[ -z "$MIRROR_DIR" && "$APP_DIR" == "/Applications" ]]; then
  MIRROR_DIR="$HOME/Applications"
fi
if [[ -n "$MIRROR_DIR" ]]; then
  mkdir -p "$MIRROR_DIR"
  MIRROR_DIR="$(cd "$MIRROR_DIR" && pwd -P)"
  [[ "$MIRROR_DIR" == "$APP_DIR" ]] || DIRS+=("$MIRROR_DIR")
fi

# Refuse a mapped installed copy; never kill a process by its basename.
RUNNING_PATHS="$(ps -axo comm= | sed 's/^[[:space:]]*//')"
for dir in "${DIRS[@]}"; do
  if printf '%s\n' "$RUNNING_PATHS" | grep -Fxq -- "$dir/$APP_NAME/Contents/MacOS/harbard-desktop"; then
    echo "error: $dir/$APP_NAME is running; quit that installed copy through native app control, then retry" >&2
    exit 1
  fi
  [[ ! -L "$dir/$APP_NAME" ]] || { echo "error: refusing symlink app destination: $dir/$APP_NAME" >&2; exit 1; }
done
STAGES=()
BACKUPS=()
INSTALLED=()
rollback() {
  local index destination backup stage
  for ((index=${#STAGES[@]}-1; index>=0; index--)); do
    destination="${DIRS[$index]}/$APP_NAME"
    backup="${BACKUPS[$index]:-}"
    stage="${STAGES[$index]}"
    if [[ "${INSTALLED[$index]:-0}" == "1" && -d "$destination" ]]; then
      mv "$destination" "$stage/failed-$APP_NAME" || return 1
    fi
    if [[ -n "$backup" && -d "$backup/$APP_NAME" ]]; then
      mv "$backup/$APP_NAME" "$destination" || return 1
      rmdir "$backup" || true
    fi
  done
}
finish() {
  local status=$? stage
  set +u
  trap - EXIT
  if [[ "$status" -ne 0 ]]; then
    if ! rollback; then
      echo "error: rollback needs operator recovery; retained backup/stage paths follow" >&2
    fi
    for stage in "${STAGES[@]}"; do echo "retained stage: $stage" >&2; done
    for stage in "${BACKUPS[@]}"; do [[ -z "$stage" ]] || echo "backup: $stage" >&2; done
  fi
  exit "$status"
}
trap finish EXIT

# Prepare every replacement before moving either incumbent bundle.
for dir in "${DIRS[@]}"; do
  stage="$(mktemp -d "$dir/.harbard-stage.XXXXXX")"
  STAGES+=("$stage")
  BACKUPS+=("")
  INSTALLED+=("0")
  if [[ "${#STAGES[@]}" == "1" ]]; then
    ditto "$BUNDLE" "$stage/$APP_NAME"
    if [[ -n "${HARBARD_BUILD_RECEIPT:-}" ]]; then
      mkdir -p "$stage/$APP_NAME/Contents/Resources"
      cp "$HARBARD_BUILD_RECEIPT" "$stage/$APP_NAME/Contents/Resources/harbard-build.json"
    fi
    codesign --force --deep --sign - "$stage/$APP_NAME"
  else
    ditto "${STAGES[0]}/$APP_NAME" "$stage/$APP_NAME"
  fi
  codesign --verify --deep --strict "$stage/$APP_NAME"
done
EXPECTED_HASH="$(shasum -a 256 "${STAGES[0]}/$APP_NAME/Contents/MacOS/harbard-desktop" | awk '{print $1}')"
for stage in "${STAGES[@]}"; do
  actual="$(shasum -a 256 "$stage/$APP_NAME/Contents/MacOS/harbard-desktop" | awk '{print $1}')"
  [[ "$actual" == "$EXPECTED_HASH" ]] || { echo "error: staged desktop copies diverged" >&2; exit 1; }
done
# Recheck after signing/copying: native launch during preparation must not
# turn a previously stopped bundle into a live replacement target.
RUNNING_PATHS="$(ps -axo comm= | sed 's/^[[:space:]]*//')"
for dir in "${DIRS[@]}"; do
  if printf '%s\n' "$RUNNING_PATHS" | grep -Fxq -- "$dir/$APP_NAME/Contents/MacOS/harbard-desktop"; then
    echo "error: installed app started during preparation; quit it through native app control and retry" >&2
    exit 1
  fi
done
for ((index=0; index<${#DIRS[@]}; index++)); do
  destination="${DIRS[$index]}/$APP_NAME"
  if [[ -e "$destination" ]]; then
    backup="$(mktemp -d "${DIRS[$index]}/.harbard-backup.XXXXXX")"
    BACKUPS[$index]="$backup"
    mv "$destination" "$backup/$APP_NAME"
  fi
  mv "${STAGES[$index]}/$APP_NAME" "$destination"
  INSTALLED[$index]="1"
  codesign --verify --deep --strict "$destination"
done
for ((index=0; index<${#DIRS[@]}; index++)); do
  destination="${DIRS[$index]}/$APP_NAME"
  actual="$(shasum -a 256 "$destination/Contents/MacOS/harbard-desktop" | awk '{print $1}')"
  [[ "$actual" == "$EXPECTED_HASH" ]] || { echo "error: installed desktop copies diverged" >&2; exit 1; }
done
trap - EXIT
for stage in "${STAGES[@]}"; do rmdir "$stage" || true; done
for ((index=0; index<${#DIRS[@]}; index++)); do
  destination="${DIRS[$index]}/$APP_NAME"
  if [[ "${HARBARD_REGISTER_APP:-1}" == "1" && -x "$LSREGISTER" ]]; then
    "$LSREGISTER" -f "$destination" >/dev/null 2>&1 || true
  fi
  echo "installed: $destination (sha256 $EXPECTED_HASH)"
  [[ -z "${BACKUPS[$index]}" ]] || echo "previous bundle retained: ${BACKUPS[$index]}/$APP_NAME"
  # Only the two newest backups stay: one per update had piled up to 45, each
  # still registered with macOS under this app's bundle id.
  while IFS= read -r old; do
    [[ -n "$old" ]] || continue
    if [[ "${HARBARD_REGISTER_APP:-1}" == "1" && -x "$LSREGISTER" ]]; then
      "$LSREGISTER" -u "$old/$APP_NAME" >/dev/null 2>&1 || true
    fi
    rm -rf -- "$old"
  done < <(ls -1dt "${DIRS[$index]}"/.harbard-backup.* 2>/dev/null | tail -n +3)
done
if [[ "${HARBARD_OPEN_APP:-0}" != "0" ]]; then
  echo "harbard desktop — native reopening is caller-owned; open $DEST through native app control"
fi
