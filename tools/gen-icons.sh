#!/bin/bash
# SPDX-License-Identifier: MIT
#
# gen-icons.sh — renders the app icon (packaging/icon.svg, our own design) to
# packaging/icon.png (256 px: window icon and Linux package) and
# packaging/icon.ico (16–256 px: the Windows .exe). Run after changing the SVG
# and commit the results; builds use the committed files.
#
# Needs rsvg-convert (librsvg) and ImageMagick 7 (magick).

set -euo pipefail
cd "$(dirname "$0")/.."

tmp=$(mktemp -d scratchpad/icons.XXXXXX 2>/dev/null || mktemp -d)
trap 'rm -rf "$tmp"' EXIT
sizes=(16 24 32 48 64 128 256)
for s in "${sizes[@]}"; do
    rsvg-convert -w "$s" -h "$s" packaging/icon.svg -o "$tmp/$s.png"
done
cp "$tmp/256.png" packaging/icon.png
magick "${sizes[@]/#/$tmp/}" packaging/icon.ico 2>/dev/null || \
    magick $(for s in "${sizes[@]}"; do echo "$tmp/$s.png"; done) packaging/icon.ico
echo "wrote packaging/icon.png and packaging/icon.ico"
