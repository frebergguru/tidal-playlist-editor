#!/bin/bash
# SPDX-License-Identifier: MIT
#
# release.sh — build, test and package TIDAL Playlist Editor for Linux and Windows.
# Refuses the release if any shipped binary still carries symbols or debug info
# (check-stripped.sh), both before and after packaging.
#
# Usage: ./release.sh            output in dist/<version>/

set -euo pipefail
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:$PATH"

VERSION=$(grep -m1 '^version' Cargo.toml | cut -d'"' -f2)
OUT="dist/$VERSION"
NAME=tidal-playlist-editor
JOBS=${JOBS:-4}

echo "== assets"
tools/fetch-assets.sh

echo "== tests"
cargo test --locked -j "$JOBS"
cargo clippy --locked -j "$JOBS" --all-targets -- -D warnings

echo "== build"
cargo build --release --locked -j "$JOBS"
cargo build --release --locked -j "$JOBS" --target x86_64-pc-windows-gnu

echo "== NOTICE"
python3 -I tools/gen-notice.py > NOTICE

rm -rf "$OUT"
mkdir -p "$OUT/$NAME-linux-x86_64" "$OUT/$NAME-windows-x86_64"
install -m 755 target/release/tidal-editor "$OUT/$NAME-linux-x86_64/$NAME"
install -m 644 packaging/$NAME.desktop "$OUT/$NAME-linux-x86_64/"
install -m 644 packaging/icon.png "$OUT/$NAME-linux-x86_64/$NAME.png"
install -m 644 target/x86_64-pc-windows-gnu/release/tidal-editor.exe "$OUT/$NAME-windows-x86_64/$NAME.exe"
for d in "$OUT"/$NAME-*; do
    install -m 644 README.md LICENSE NOTICE "$d/"
done

# Windows: the .exe must not need MinGW runtime DLLs that we do not ship.
# objdump runs on its own first, so a missing or failing objdump stops the
# release instead of looking like "no bad DLLs".
IMPORTS=$(objdump -p "$OUT/$NAME-windows-x86_64/$NAME.exe")
if ! grep -q "DLL Name:" <<<"$IMPORTS"; then
    echo "FAIL: could not read the Windows import table" >&2
    exit 1
fi
if grep -iE "DLL Name: (libgcc|libstdc|libwinpthread)" <<<"$IMPORTS"; then
    echo "FAIL: Windows build depends on MinGW runtime DLLs" >&2
    exit 1
fi

echo "== strip check (files)"
./check-stripped.sh "$OUT"

echo "== package"
tar -C "$OUT" -czf "$OUT/$NAME-$VERSION-linux-x86_64.tar.gz" "$NAME-linux-x86_64"
python3 -I -c '
import os, sys, zipfile
src, dst = sys.argv[1], sys.argv[2]
with zipfile.ZipFile(dst, "w", zipfile.ZIP_DEFLATED) as z:
    for root, _, files in os.walk(src):
        for f in files:
            p = os.path.join(root, f)
            z.write(p, os.path.relpath(p, os.path.dirname(src)))
' "$OUT/$NAME-windows-x86_64" "$OUT/$NAME-$VERSION-windows-x86_64.zip"

echo "== strip check (archives)"
./check-stripped.sh "$OUT/$NAME-$VERSION-linux-x86_64.tar.gz" "$OUT/$NAME-$VERSION-windows-x86_64.zip"

echo "== AppImage"
# The runtime is third-party (MIT, with LGPL-2.1 libfuse linked in): fetched
# pinned, checked for symbols like our own binaries, and its corresponding
# source is published next to the AppImage.
tools/fetch-appimage.sh
python3 -I tools/gen-notice.py --appimage target/appimage > NOTICE-appimage
APPDIR=target/appimage/AppDir
rm -rf "$APPDIR"
mkdir -p "$APPDIR/usr/bin" "$APPDIR/usr/share/applications" "$APPDIR/usr/share/icons/hicolor/256x256/apps" \
    "$APPDIR/usr/share/doc/$NAME"
install -m 755 target/release/tidal-editor "$APPDIR/usr/bin/$NAME"
install -m 644 packaging/$NAME.desktop "$APPDIR/$NAME.desktop"
install -m 644 packaging/$NAME.desktop "$APPDIR/usr/share/applications/"
install -m 644 packaging/icon.png "$APPDIR/$NAME.png"
install -m 644 packaging/icon.png "$APPDIR/usr/share/icons/hicolor/256x256/apps/$NAME.png"
ln -s "$NAME.png" "$APPDIR/.DirIcon"
ln -s "usr/bin/$NAME" "$APPDIR/AppRun"
install -m 644 README.md LICENSE "$APPDIR/usr/share/doc/$NAME/"
install -m 644 NOTICE-appimage "$APPDIR/usr/share/doc/$NAME/NOTICE"
echo "== strip check (AppImage contents and runtime)"
./check-stripped.sh "$APPDIR" target/appimage/runtime-x86_64
APPIMAGE="$OUT/$NAME-$VERSION-x86_64.AppImage"
ARCH=x86_64 appimagetool --no-appstream --runtime-file target/appimage/runtime-x86_64 "$APPDIR" "$APPIMAGE"
./check-stripped.sh "$APPIMAGE"
# The runtime's complete corresponding source (LGPL-2.1 for libfuse).
cat > target/appimage/source/README.txt <<'TXT'
Source code of the AppImage runtime used by TIDAL Playlist Editor's AppImage.

type2-runtime-20251108.tar.gz  The AppImage type2 runtime at tag 20251108
                               (https://github.com/AppImage/type2-runtime), with
                               the libfuse patch and the build scripts that build
                               and link the runtime (see BUILD.md inside).
fuse-3.15.0.tar.xz             libfuse 3.15.0 (LGPL-2.1), the exact release the
                               runtime's build script uses (same SHA-256).

The runtime is used unmodified. Licences: see NOTICE inside the AppImage
(usr/share/doc/tidal-playlist-editor/NOTICE).
TXT
tar -C target/appimage -czf "$OUT/appimage-runtime-source-20251108.tar.gz" source

(cd "$OUT" && sha256sum ./*.tar.gz ./*.zip ./*.AppImage > SHA256SUMS)
echo "== done: $OUT"
ls -la "$OUT"
