#!/usr/bin/env python3
"""Writes NOTICE: every third-party crate compiled into the release binaries
(Linux and Windows), with its licence texts, plus the bundled fonts.

Usage: python3 -I tools/gen-notice.py > NOTICE   (run from the repo root)
       python3 -I tools/gen-notice.py --appimage target/appimage > NOTICE-appimage
With --appimage, also the AppImage runtime and the libraries it links in
(licence texts fetched by tools/fetch-appimage.sh).
"""
import json, os, subprocess, sys

TARGETS = ["x86_64-unknown-linux-gnu", "x86_64-pc-windows-gnu"]
LICENSE_PREFIXES = ("license", "licence", "copying", "notice", "copyright")


def resolve(target):
    meta = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--format-version", "1", "--locked", "--filter-platform", target]))
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    pkgs = {p["id"]: p for p in meta["packages"]}
    root = meta["resolve"]["root"]
    seen, stack = set(), [root]
    while stack:
        cur = stack.pop()
        if cur in seen:
            continue
        seen.add(cur)
        for dep in nodes[cur]["deps"]:
            # Only normal (and build-time code that ends up linked, e.g. cc-built C) dependencies.
            if any(k["kind"] in (None, "build") for k in dep["dep_kinds"]):
                stack.append(dep["pkg"])
    seen.discard(root)
    return {pkgs[i]["name"] + " " + pkgs[i]["version"]: pkgs[i] for i in seen}


def licence_files(pkg):
    d = os.path.dirname(pkg["manifest_path"])
    out = []
    for f in sorted(os.listdir(d)):
        if f.lower().startswith(LICENSE_PREFIXES) and os.path.isfile(os.path.join(d, f)):
            with open(os.path.join(d, f), encoding="utf-8", errors="replace") as fh:
                out.append((f, fh.read().strip()))
    # Crates that embed fonts (epaint_default_fonts) keep each font's licence
    # next to it in fonts/, as .txt files.
    fonts = os.path.join(d, "fonts")
    if os.path.isdir(fonts):
        for f in sorted(os.listdir(fonts)):
            if f.lower().endswith(".txt"):
                with open(os.path.join(fonts, f), encoding="utf-8", errors="replace") as fh:
                    out.append(("fonts/" + f, fh.read().strip()))
    return out


all_pkgs = {}
for t in TARGETS:
    all_pkgs.update(resolve(t))

w = sys.stdout.write
w("TIDAL Playlist Editor — third-party notices\n")
w("=" * 72 + "\n\n")
w("This program includes the following open-source components.\n\n")
w("Fonts\n-----\n")
for font, lic in [("Noto Sans (SIL Open Font License 1.1)", "assets/fonts/NotoSans-LICENSE.txt"),
                  ("DejaVu Sans (Bitstream Vera / public domain)", "assets/fonts/DejaVu-LICENSE.txt")]:
    w(f"\n{font}\n\n")
    with open(lic, encoding="utf-8") as fh:
        w(fh.read().strip() + "\n")
w("\n\nRust crates\n-----------\n")
w("Identical licence texts are printed once, at the end, and referred to as [T1], [T2], ….\n")
texts = {}
for key in sorted(all_pkgs, key=str.lower):
    p = all_pkgs[key]
    w("\n" + "-" * 72 + "\n")
    w(f"{key}  —  {p.get('license') or 'see licence file'}\n")
    if p.get("repository"):
        w(f"{p['repository']}\n")
    files = licence_files(p)
    if not files:
        w(f"(No licence file in the published crate; licence: {p.get('license')}.)\n")
    for name, text in files:
        ref = texts.setdefault(text, f"T{len(texts) + 1}")
        w(f"{name}: [{ref}]\n")
w("\n\n" + "=" * 72 + "\nLicence texts\n" + "=" * 72 + "\n")
for text, ref in texts.items():
    w(f"\n[{ref}]\n{text}\n")

w("\n\n" + "=" * 72 + "\n")
w("Source code of MPL-2.0 components\n")
w("-" * 33 + "\n")
w("The MPL-2.0 licensed crates listed above are used without modification. Their\n"
  "source code is available from https://crates.io/crates/<name>/<version>, for\n"
  "example https://crates.io/crates/symphonia-core.\n")
w("\n" + "=" * 72 + "\n")
w("Standard licence texts (for crates that name a licence but ship no file)\n")
w("=" * 72 + "\n")
w("\n[MIT]\n" + """Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
""")
for spdx in ["Apache-2.0", "BSL-1.0", "MPL-2.0"]:
    with open(f"/usr/share/licenses/spdx/{spdx}.txt", encoding="utf-8") as fh:
        w(f"\n[{spdx}]\n" + fh.read().strip() + "\n")

w("\n\n" + "=" * 72 + "\n")
w("Rust standard library and MinGW-w64 runtime\n")
w("=" * 72 + "\n")
w("\nThe Rust standard library is built into every binary. It is licensed under\n"
  "MIT OR Apache-2.0 (texts [MIT] and [Apache-2.0] above); the copyright notices\n"
  "of it and its dependencies are in the Rust toolchain's\n"
  "share/doc/rust/COPYRIGHT-library.html and at https://github.com/rust-lang/rust.\n")
# The Windows build links the MinGW-w64 runtime statically; its licence asks
# for this file in every binary distribution.
with open("/usr/share/licenses/mingw-w64-crt/COPYING.MinGW-w64-runtime.txt", encoding="utf-8") as fh:
    w("\nMinGW-w64 runtime (Windows build)\n\n" + fh.read().strip() + "\n")

if "--appimage" in sys.argv:
    d = os.path.join(sys.argv[sys.argv.index("--appimage") + 1], "licences")
    w("\n\n" + "=" * 72 + "\n")
    w("AppImage runtime\n")
    w("=" * 72 + "\n")
    w("The AppImage starts with the AppImage type2 runtime (tag 20251108,\n"
      "https://github.com/AppImage/type2-runtime), which statically links musl libc,\n"
      "libfuse 3.15.0 (LGPL-2.1, with the runtime's patch), squashfuse, zstd and zlib.\n"
      "Their complete corresponding source is published next to every AppImage as\n"
      "appimage-runtime-source-20251108.tar.gz: the runtime's source at that tag (its\n"
      "libfuse patch and build scripts included, which rebuild and relink it) and the\n"
      "libfuse 3.15.0 release tarball. The runtime is used without modification.\n")
    for name, f in [("AppImage type2 runtime (MIT)", "type2-runtime-LICENSE"),
                    ("libfuse (LGPL-2.1)", "libfuse-LGPL2.txt"),
                    ("musl libc (MIT)", "musl-COPYRIGHT"),
                    ("squashfuse (BSD-2-Clause)", "squashfuse-LICENSE"),
                    ("zstd (BSD-3-Clause)", "zstd-LICENSE"),
                    ("zlib (Zlib)", "zlib-LICENSE")]:
        with open(os.path.join(d, f), encoding="utf-8", errors="replace") as fh:
            w(f"\n{name}\n\n" + fh.read().strip() + "\n")
