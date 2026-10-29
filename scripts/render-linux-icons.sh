#!/usr/bin/env bash
# Re-renders packaging/linux/icons/*.png from packaging/linux/miko-kagura.svg.
# The PNGs are committed so release builds need no SVG renderer; run this after
# editing the SVG. Requires rsvg-convert (librsvg2-bin / librsvg).
set -euo pipefail
. "$(dirname "$0")/release-common.sh"
command -v rsvg-convert >/dev/null || die 'Install rsvg-convert (librsvg2-bin).'
cd "$PROJECT_ROOT/packaging/linux"
mkdir -p icons
for size in 16 24 32 48 64 128 256 512; do
    rsvg-convert -w "$size" -h "$size" -o "icons/${size}x${size}.png" miko-kagura.svg
done
echo "Rendered $(ls icons | wc -l) icons into packaging/linux/icons"
