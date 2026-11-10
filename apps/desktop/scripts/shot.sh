#!/usr/bin/env bash
# Capture the largest window of a running app process to a PNG.
#   scripts/shot.sh <pid> <out.png> [x y w h]   (optional crop, in window points)
set -euo pipefail
pid=${1:?pid}; out=${2:?out.png}
here=$(cd "$(dirname "$0")" && pwd)
bin=${TMPDIR:-/tmp}/harbard-winid
[[ -x "$bin" ]] || swiftc -O "$here/winid.swift" -o "$bin" >/dev/null
win=$("$bin" "$pid" | awk '{print $4*$5, $1}' | sort -n -r | head -1 | cut -d' ' -f2)
[[ -n "$win" ]] || { echo "no window for pid $pid" >&2; exit 1; }
screencapture -x -o -l"$win" "$out"
if [[ $# -ge 6 ]]; then
  python3 - "$out" "$3" "$4" "$5" "$6" <<'PY'
import sys
from PIL import Image
p, x, y, w, h = sys.argv[1], *map(int, sys.argv[2:])
im = Image.open(p)
s = im.width / 1440 if im.width > 1500 else 1
im.crop((int(x*s), int(y*s), int((x+w)*s), int((y+h)*s))).save(p)
PY
fi
echo "$out"
