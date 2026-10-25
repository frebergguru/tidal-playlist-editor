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

# Known-good SHA-256 sums. A file is only put in place when it matches, so a
# moved upstream branch, a different distro build or a broken download stops
# here instead of ending up in the app. Update the pins deliberately.
NOTO_TTF=478c558ea716033cd60c03438f628dfa75694dcf6b5f6d505a2f05fd2b4f3823
NOTO_OFL=cee9892f9f0cc8fe882c9e9537ee6a89621d86ee7ceaf70b02e2b2b1c25c061a
DEJAVU_TAR=fa9ca4d13871dd122f61258a80d01751d603b4d3ee14095d65453b4e846e17d7
# DejaVu Sans 2.37: upstream release file, and Arch's ttf-dejavu build.
DEJAVU_TTF="7da195a74c55bef988d0d48f9508bd5d849425c1770dba5d7bfc6ce9ed848954 6038a160b491e121c1f12c7bccb4a9c8730296e3adc1086a059404ed84b7451c"
DEJAVU_LIC=7a083b136e64d064794c3419751e5c7dd10d2f64c108fe5ba161eae5e5958a93

work=$(mkdir -p target && mktemp -d "${PWD}/target/fetch-XXXXXX")
trap 'rm -rf "$work"' EXIT

# matches FILE "SUM [SUM…]"
matches() {
    [ -s "$1" ] || return 1
    local have
    have=$(sha256sum "$1" | cut -d' ' -f1)
    for want in $2; do [ "$have" = "$want" ] && return 0; done
    return 1
}
# get URL FILE: downloads to a temp file first, so a failed download never
# leaves a partial file behind.
get() {
    local tmp="$work/dl.$$"
    curl -sSfL -A "$UA" -o "$tmp" "$1"
    mv "$tmp" "$2"
}
# install_checked SRC DEST "SUMS": copies only a matching file into place.
install_checked() {
    if ! matches "$1" "$3"; then
        echo "FAIL: $1 does not match the pinned SHA-256 (got $(sha256sum "$1" | cut -d' ' -f1))" >&2
        exit 1
    fi
    cp "$1" "$work/put.$$"
    mv "$work/put.$$" "$2"
}

if ! matches "$DEST/NotoSans-Regular.ttf" "$NOTO_TTF"; then
    src=/usr/share/fonts/noto/NotoSans-Regular.ttf
    if ! matches "$src" "$NOTO_TTF"; then
        src="$work/noto.ttf"
        get "https://github.com/notofonts/notofonts.github.io/raw/main/fonts/NotoSans/hinted/ttf/NotoSans-Regular.ttf" "$src"
    fi
    install_checked "$src" "$DEST/NotoSans-Regular.ttf" "$NOTO_TTF"
fi
if ! matches "$DEST/NotoSans-LICENSE.txt" "$NOTO_OFL"; then
    get "https://raw.githubusercontent.com/notofonts/latin-greek-cyrillic/main/OFL.txt" "$work/ofl.txt"
    install_checked "$work/ofl.txt" "$DEST/NotoSans-LICENSE.txt" "$NOTO_OFL"
fi

if ! matches "$DEST/DejaVuSans.ttf" "$DEJAVU_TTF" || ! matches "$DEST/DejaVu-LICENSE.txt" "$DEJAVU_LIC"; then
    ttf=/usr/share/fonts/TTF/DejaVuSans.ttf
    lic=/usr/share/licenses/ttf-dejavu/LICENSE
    if ! matches "$ttf" "$DEJAVU_TTF" || ! matches "$lic" "$DEJAVU_LIC"; then
        get "https://github.com/dejavu-fonts/dejavu-fonts/releases/download/version_2_37/dejavu-fonts-ttf-2.37.tar.bz2" \
            "$work/dejavu.tar.bz2"
        install_checked "$work/dejavu.tar.bz2" "$work/dejavu-checked.tar.bz2" "$DEJAVU_TAR"
        tar -xjf "$work/dejavu-checked.tar.bz2" -C "$work" dejavu-fonts-ttf-2.37/ttf/DejaVuSans.ttf dejavu-fonts-ttf-2.37/LICENSE
        ttf="$work/dejavu-fonts-ttf-2.37/ttf/DejaVuSans.ttf"
        lic="$work/dejavu-fonts-ttf-2.37/LICENSE"
    fi
    install_checked "$ttf" "$DEST/DejaVuSans.ttf" "$DEJAVU_TTF"
    install_checked "$lic" "$DEST/DejaVu-LICENSE.txt" "$DEJAVU_LIC"
fi
ls -l "$DEST"
