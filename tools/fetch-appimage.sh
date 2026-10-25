#!/bin/bash
# SPDX-License-Identifier: MIT
#
# fetch-appimage.sh — downloads what the AppImage needs into target/appimage/.
# None of it is kept in the repository. Every file is pinned by SHA-256.
#
#   runtime-x86_64   AppImage type2 runtime, tag 20251108 (MIT; statically links
#                    musl, libfuse (LGPL-2.1), squashfuse, zstd and zlib)
#   source/          the runtime's corresponding source, shipped next to every
#                    AppImage for the LGPL: the runtime at that tag (with its
#                    libfuse patch and build scripts) and libfuse 3.15.0, the
#                    exact tarball its build script pins
#   licences/        licence texts of the runtime's parts, for NOTICE

set -euo pipefail
cd "$(dirname "$0")/.."

DIR=target/appimage
mkdir -p "$DIR/source" "$DIR/licences"
UA="tidal-playlist-editor-build"

fetch() { # path sha256 url
    local path="$DIR/$1" sum="$2" url="$3"
    if [ -f "$path" ] && echo "$sum  $path" | sha256sum -c --status -; then
        return
    fi
    curl -fsSL -A "$UA" -o "$path.part" "$url"
    if ! echo "$sum  $path.part" | sha256sum -c --status -; then
        rm -f "$path.part"
        echo "FAIL: $1 does not match its pinned SHA-256 ($url)" >&2
        exit 1
    fi
    mv "$path.part" "$path"
}

fetch runtime-x86_64 2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d \
    https://github.com/AppImage/type2-runtime/releases/download/20251108/runtime-x86_64
fetch source/type2-runtime-20251108.tar.gz 4c4f6df4647c9f01f871d7edd3716d8aeeffda9d22ffbebe3fccd95a6ab52c95 \
    https://github.com/AppImage/type2-runtime/archive/refs/tags/20251108.tar.gz
fetch source/fuse-3.15.0.tar.xz 70589cfd5e1cff7ccd6ac91c86c01be340b227285c5e200baa284e401eea2ca0 \
    https://github.com/libfuse/libfuse/releases/download/fuse-3.15.0/fuse-3.15.0.tar.xz
fetch licences/musl-COPYRIGHT f9bc4423732350eb0b3f7ed7e91d530298476f8fec0c6c427a1c04ade22655af \
    "https://git.musl-libc.org/cgit/musl/plain/COPYRIGHT?h=v1.2.5"
fetch licences/squashfuse-LICENSE 9e909cc8a8ba27b1a649c964cf9eda37de911f3138c2b64e6d2f01703904ac13 \
    https://raw.githubusercontent.com/vasi/squashfuse/0.5.2/LICENSE
fetch licences/zstd-LICENSE 7055266497633c9025b777c78eb7235af13922117480ed5c674677adc381c9d8 \
    https://raw.githubusercontent.com/facebook/zstd/v1.5.6/LICENSE
fetch licences/zlib-LICENSE 845efc77857d485d91fb3e0b884aaa929368c717ae8186b66fe1ed2495753243 \
    https://raw.githubusercontent.com/madler/zlib/v1.3.1/LICENSE

# The runtime's own licence and libfuse's LGPL, taken from the pinned sources.
tar -xOzf "$DIR/source/type2-runtime-20251108.tar.gz" type2-runtime-20251108/LICENSE > "$DIR/licences/type2-runtime-LICENSE"
tar -xOJf "$DIR/source/fuse-3.15.0.tar.xz" fuse-3.15.0/LGPL2.txt > "$DIR/licences/libfuse-LGPL2.txt"
chmod 755 "$DIR/runtime-x86_64"
echo "AppImage runtime, sources and licences in $DIR"
