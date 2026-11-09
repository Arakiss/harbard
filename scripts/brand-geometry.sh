#!/usr/bin/env bash
# Write the console's mark geometry from assets/brand/mark.svg.
#
# The rail's logo tile and the chat Mark draw it through BrandMark: shapes
# filled with currentColor follow the theme, any other fill is the presence
# dot (--hb-green), and <mask> gaps are kept. Called by render-brand.sh; runs
# on its own too (needs bun). A console test fails when the generated file
# and the SVG disagree.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
command -v bun >/dev/null || { echo "missing: bun" >&2; exit 1; }
bun "$root/apps/console/scripts/brand-geometry.ts" "$root/assets/brand/mark.svg" \
  "$root/apps/console/src/components/brand/mark-geometry.ts"
