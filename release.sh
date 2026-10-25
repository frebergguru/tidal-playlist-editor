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
install -m 644 target/x86_64-pc-windows-gnu/release/tidal-editor.exe "$OUT/$NAME-windows-x86_64/$NAME.exe"
for d in "$OUT"/$NAME-*; do
    install -m 644 README.md LICENSE NOTICE "$d/"
done

# Windows: the .exe must not need MinGW runtime DLLs that we do not ship.
if objdump -p "$OUT/$NAME-windows-x86_64/$NAME.exe" | grep -iE "DLL Name: (libgcc|libstdc|libwinpthread)"; then
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

(cd "$OUT" && sha256sum ./*.tar.gz ./*.zip > SHA256SUMS)
echo "== done: $OUT"
ls -la "$OUT"
