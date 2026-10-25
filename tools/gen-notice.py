#!/usr/bin/env python3
"""Writes NOTICE: every third-party crate compiled into the release binaries
(Linux and Windows), with its licence texts, plus the bundled fonts.

Usage: python3 -I tools/gen-notice.py > NOTICE   (run from the repo root)
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
