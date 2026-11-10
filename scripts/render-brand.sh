#!/usr/bin/env bash
# Rasterise the brand set from assets/brand/*.svg.
#
# Every PNG and .icns in the repo is OUTPUT. Editing one by hand is how a
# product ends up with three logos at once — which is exactly what this replaced.
# Change the SVG, run this, commit both. Optional: --preview /path/to/preview.png.
#
# Needs: rsvg-convert (librsvg) and iconutil (macOS).
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
brand="$root/assets/brand"
icons="$root/apps/desktop/bundle"
web="$root/apps/console/src/app"
preview="${2:-}"
if [[ $# -gt 0 && ( "$1" != "--preview" || $# -ne 2 ) ]]; then
  echo "usage: $0 [--preview /path/to/preview.png]" >&2
  exit 1
fi

for bin in rsvg-convert iconutil; do
  command -v "$bin" >/dev/null || { echo "missing: $bin" >&2; exit 1; }
done

echo "==> Crossing geometry"
mark_path="$(sed -n 's/.* d="\([^"]*\)".*/\1/p' "$brand/mark.svg")"
[[ -n "$mark_path" ]] || { echo "missing master mark path" >&2; exit 1; }
for source in "$brand/app-icon.svg" "$brand/tray.svg" "$brand/og.svg"; do
  grep -Fq "d=\"$mark_path\"" "$source" || {
    echo "Crossing geometry differs: $source" >&2; exit 1;
  }
done

echo "==> Console mark (rail tile and chat Mark, drawn in currentColor)"
bash "$root/scripts/brand-geometry.sh"

png() { rsvg-convert -w "$2" -h "$2" "$1" -o "$3"; }

echo "==> desktop bundle"
png "$brand/app-icon.svg" 1024 "$icons/icon.png"
rsvg-convert -w 44 -h 44 "$brand/tray.svg" -o "$icons/tray.png"
cp "$brand/app-icon.svg" "$icons/icon.svg"
cp "$brand/tray.svg" "$icons/tray.svg"

echo "==> living Dock icon, tray variants and the macOS 26 layered icon"
python3 "$root/scripts/render-dock.py"

echo "==> icns"
set="$(mktemp -d)/harbard.iconset"
mkdir -p "$set"
for pair in "16 icon_16x16" "32 icon_16x16@2x" "32 icon_32x32" "64 icon_32x32@2x" \
            "128 icon_128x128" "256 icon_128x128@2x" "256 icon_256x256" \
            "512 icon_256x256@2x" "512 icon_512x512" "1024 icon_512x512@2x"; do
  png "$brand/app-icon.svg" "${pair% *}" "$set/${pair#* }.png"
done
iconutil -c icns "$set" -o "$icons/icon.icns"
rm -rf "$(dirname "$set")"

echo "==> web"
# Next.js file conventions: apple-icon must be a raster, opengraph-image is the
# social card. Distributed SVGs use the same source as their raster variants.
cp "$brand/app-icon.svg" "$web/icon.svg"
cp "$brand/app-icon.svg" "$web/apple-icon.svg"
png "$brand/app-icon.svg" 180 "$web/apple-icon.png"
rsvg-convert -w 1200 -h 630 "$brand/og.svg" -o "$web/opengraph-image.png"

if [[ -n "$preview" ]]; then
  echo "==> small-size preview"
  tmp="$(mktemp -d)"
  sheet="$tmp/preview.svg"
  printf '<svg xmlns="http://www.w3.org/2000/svg" width="1232" height="176" viewBox="0 0 1232 176"><rect width="1232" height="176" fill="#181b1f"/>' > "$sheet"
  preview_tile() {
    local x="$1" size="$2" source="$3" bg="$4" ink="$5" label="$6"
    local data
    data="$(base64 < "$source" | tr -d '\n')"
    printf '<rect x="%s" y="8" width="160" height="160" fill="%s"/><image x="%s" y="%s" width="%s" height="%s" href="data:image/png;base64,%s"/><text x="%s" y="157" text-anchor="middle" font-family="Helvetica,Arial,sans-serif" font-size="13" fill="%s">%s</text>' \
      "$x" "$bg" "$((x + 80 - size / 2))" "$((80 - size / 2))" "$size" "$size" "$data" "$((x + 80))" "$ink" "$label" >> "$sheet"
  }
  x=8
  for size in 16 22 32 64 128; do
    png "$brand/app-icon.svg" "$size" "$tmp/source.png"
    preview_tile "$x" "$size" "$tmp/source.png" '#181b1f' '#e2e4e7' "App ${size}px"
    x=$((x + 176))
  done
  for mode in light dark; do
    if [[ "$mode" == light ]]; then bg='#f3f4f5'; ink='#20242a'; else bg='#20242a'; ink='#e2e4e7'; fi
    sed "s/#000000/$ink/" "$brand/tray.svg" > "$tmp/tray.svg"
    rsvg-convert -w 22 -h 22 "$tmp/tray.svg" -o "$tmp/source.png"
    preview_tile "$x" 22 "$tmp/source.png" "$bg" "$ink" "Tray 22px $mode"
    x=$((x + 176))
  done
  printf '</svg>' >> "$sheet"
  rsvg-convert "$sheet" -o "$preview"
  rm -rf "$tmp"
fi

echo "done — $(cd "$root" && git status --short -- assets/brand apps/desktop/bundle "$web" apps/console/src/components/brand | wc -l | tr -d ' ') paths touched"
