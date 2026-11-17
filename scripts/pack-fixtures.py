#!/usr/bin/env python3
"""Pack QLC+'s fixture library into one compressed file that Lumen embeds.

    git clone --depth 1 --filter=blob:none --sparse https://github.com/mcallegari/qlcplus
    git -C qlcplus sparse-checkout set resources/fixtures
    python3 scripts/pack-fixtures.py qlcplus

Format (zlib-compressed): u32 little-endian index length, JSON index
[[path, offset, length], ...], then the concatenated .qxf files.
"""
import json, os, struct, sys, zlib, shutil

src = os.path.join(sys.argv[1], "resources", "fixtures")
out_dir = os.path.join(os.path.dirname(__file__), "..", "crates", "lumen-fixtures", "assets")
files, blob = [], bytearray()
for root, _, names in sorted(os.walk(src)):
    for n in sorted(names):
        if n.endswith(".qxf"):
            data = open(os.path.join(root, n), "rb").read()
            rel = os.path.relpath(os.path.join(root, n), src).replace(os.sep, "/")
            files.append([rel, len(blob), len(data)])
            blob += data
index = json.dumps(files, separators=(",", ":")).encode()
raw = struct.pack("<I", len(index)) + index + bytes(blob)
packed = zlib.compress(raw, 9)
open(os.path.join(out_dir, "qlcplus-fixtures.bin"), "wb").write(packed)
shutil.copy(os.path.join(sys.argv[1], "COPYING"), os.path.join(out_dir, "QLCPLUS-LICENSE.txt"))
print(f"{len(files)} fixtures, {len(raw)/1e6:.1f} MB -> {len(packed)/1e6:.1f} MB")
