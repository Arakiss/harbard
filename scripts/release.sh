#!/usr/bin/env bash
# Cut a release in one command: bump, changelog, gate, tag, push, GitHub
# release with the signed binaries, and install it on this machine.
#
#   scripts/release.sh <version> [title]     e.g. 0.3.0-alpha.8 "Rooms that answer"
#   scripts/release.sh <version> --dry-run   bump the files in place and print the notes;
#                                             no commit, tag or push (`git restore .` undoes it)
#
# The release notes are the changelog's Unreleased section, verbatim.
#
# HARBARD_RELEASE_INSTALL=0 skips the local install (`just update` in the
# primary checkout, which restarts the console but never the bus).
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd -P)"
cd "$ROOT"

die() { echo "error: $*" >&2; exit 1; }
say() { echo "harbard release — $*"; }

NEW="${1:-}"
DRY=0
TITLE=""
if [[ "${2:-}" == "--dry-run" ]]; then DRY=1; else TITLE="${2:-}"; fi
[[ "$NEW" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]] || die "usage: scripts/release.sh <version> [title|--dry-run]"
TAG="v$NEW"
OLD="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)"
[[ "$OLD" != "$NEW" ]] || die "already at $NEW"
git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && die "$TAG already exists"

if [[ "$DRY" -eq 0 ]]; then
  [[ -z "$(git status --porcelain --untracked-files=no)" ]] || die "tracked changes present; commit or set them aside first"
  git fetch -q origin main
  [[ "$(git rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] || die "HEAD is not origin/main; release from main as it is published"
fi

# The changelog's Unreleased section becomes the release; refuse an empty one.
NOTES="$(mktemp)"
trap 'rm -f "$NOTES"' EXIT
python3 - "$NEW" "$(date +%F)" "$NOTES" <<'PY'
import re, sys
new, day, notes_path = sys.argv[1:]
text = open("CHANGELOG.md").read()
marker = "## Unreleased\n"
start = text.index(marker) + len(marker)
end = text.find("\n## ", start)
body = text[start:end].strip("\n")
if not body.strip():
    sys.exit("error: the Unreleased section of CHANGELOG.md is empty")
open(notes_path, "w").write(body + "\n")
text = text[:start] + f"\n## {new} — {day}\n\n" + body + "\n" + text[end:]
open("CHANGELOG.md", "w").write(text)
PY

say "bumping $OLD -> $NEW"
OLD_RE="${OLD//./\\.}"
for file in Cargo.toml apps/desktop/package.json \
  apps/desktop/src-tauri/Cargo.toml apps/desktop/src-tauri/tauri.conf.json; do
  grep -q "\"$OLD_RE\"" "$file" || die "$file does not carry $OLD"
  sed -i '' "s/\"$OLD_RE\"/\"$NEW\"/" "$file"
done
sed -i '' "s/\"version\": \"$OLD_RE\"/\"version\": \"$NEW\"/" bun.lock
cargo update --workspace --offline -q
cargo update --workspace --offline -q --manifest-path apps/desktop/src-tauri/Cargo.toml

if [[ "$DRY" -eq 1 ]]; then
  say "dry run: files bumped in place, nothing committed. Release notes:"
  cat "$NOTES"
  exit 0
fi

say "running the gate"
just ci

git add CHANGELOG.md Cargo.toml Cargo.lock bun.lock \
  apps/desktop/package.json apps/desktop/src-tauri/Cargo.toml \
  apps/desktop/src-tauri/Cargo.lock apps/desktop/src-tauri/tauri.conf.json
git commit -q -m "chore: release $NEW"
git tag -a "$TAG" -m "$TAG"
say "pushing main and $TAG"
git push --atomic origin HEAD:main "$TAG"

# Binaries, checksums, signatures and the SBOM come from the release workflow.
say "building the signed binaries on GitHub"
RUN_URL="$(gh workflow run release.yml --ref "$TAG" -f "tag=$TAG" 2>&1 | grep -Eo 'https://[^ ]+/actions/runs/[0-9]+' | tail -1)"
[[ -n "$RUN_URL" ]] || die "could not read the release run id; see gh run list --workflow release.yml"
RUN_ID="${RUN_URL##*/}"
gh run watch "$RUN_ID" --exit-status --interval 20 >/dev/null || die "release run failed: $RUN_URL"

PRERELEASE="--prerelease=false"
[[ "$NEW" == *-* ]] && PRERELEASE="--prerelease"
gh release edit "$TAG" --title "Harbard $TAG${TITLE:+ — $TITLE}" --notes-file "$NOTES" "$PRERELEASE" >/dev/null
say "published $(gh release view "$TAG" --json url --jq .url)"

if [[ "${HARBARD_RELEASE_INSTALL:-1}" == "1" ]]; then
  # The production console runs from the primary checkout, so that is where
  # the update must happen; a release cut from a worktree moves it forward.
  PRIMARY="$(cd "$(git rev-parse --path-format=absolute --git-common-dir)/.." && pwd -P)"
  [[ "$(git -C "$PRIMARY" rev-parse --abbrev-ref HEAD)" == "main" ]] || die "primary checkout is not on main; install skipped"
  git -C "$PRIMARY" merge -q --ff-only "$TAG"
  WAS_OPEN=0
  if pgrep -xq harbard-desktop; then
    WAS_OPEN=1
    osascript -e 'quit app "harbard"' >/dev/null
    for _ in {1..20}; do pgrep -xq harbard-desktop || break; sleep 0.5; done
  fi
  (cd "$PRIMARY" && just update)
  [[ "$WAS_OPEN" -eq 1 ]] && open -a "$HOME/Applications/harbard.app"
fi
say "$TAG done"
