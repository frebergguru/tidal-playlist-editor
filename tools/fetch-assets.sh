#!/bin/bash
# SPDX-License-Identifier: MIT
#
# fetch-assets.sh — puts the third-party fonts the app embeds into assets/fonts/.
# They are not kept in the repository. Uses the system's copies when present
# (Arch/Manjaro: noto-fonts, ttf-dejavu), otherwise downloads them from upstream.
#
#   Noto Sans Regular  — SIL Open Font License 1.1 (notofonts)
#   DejaVu Sans        — Bitstream Vera licence / public domain (dejavu-fonts 2.37)

set -euo pipefail
cd "$(dirname "$0")/.."
DEST=assets/fonts
mkdir -p "$DEST"
UA="Mozilla/5.0 (X11; Linux x86_64)"
get() { curl -sSfL -A "$UA" -o "$2" "$1"; }

if [ ! -s "$DEST/NotoSans-Regular.ttf" ]; then
    if [ -s /usr/share/fonts/noto/NotoSans-Regular.ttf ]; then
        cp /usr/share/fonts/noto/NotoSans-Regular.ttf "$DEST/"
    else
        get "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSans/hinted/ttf/NotoSans-Regular.ttf" \
            "$DEST/NotoSans-Regular.ttf"
    fi
fi
[ -s "$DEST/NotoSans-LICENSE.txt" ] ||
    get "https://raw.githubusercontent.com/notofonts/latin-greek-cyrillic/main/OFL.txt" "$DEST/NotoSans-LICENSE.txt"

if [ ! -s "$DEST/DejaVuSans.ttf" ] || [ ! -s "$DEST/DejaVu-LICENSE.txt" ]; then
    if [ -s /usr/share/fonts/TTF/DejaVuSans.ttf ] && [ -s /usr/share/licenses/ttf-dejavu/LICENSE ]; then
        cp /usr/share/fonts/TTF/DejaVuSans.ttf "$DEST/"
        cp /usr/share/licenses/ttf-dejavu/LICENSE "$DEST/DejaVu-LICENSE.txt"
    else
        work=$(mktemp -d "${PWD}/target/fetch-XXXXXX" 2>/dev/null || { mkdir -p target && mktemp -d "${PWD}/target/fetch-XXXXXX"; })
        get "https://github.com/dejavu-fonts/dejavu-fonts/releases/download/version_2_37/dejavu-fonts-ttf-2.37.tar.bz2" \
            "$work/dejavu.tar.bz2"
        tar -xjf "$work/dejavu.tar.bz2" -C "$work" dejavu-fonts-ttf-2.37/ttf/DejaVuSans.ttf dejavu-fonts-ttf-2.37/LICENSE
        cp "$work/dejavu-fonts-ttf-2.37/ttf/DejaVuSans.ttf" "$DEST/"
        cp "$work/dejavu-fonts-ttf-2.37/LICENSE" "$DEST/DejaVu-LICENSE.txt"
        rm -rf "$work"
    fi
fi
ls -l "$DEST"
