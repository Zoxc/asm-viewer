#!/bin/sh
# Renders the app's icon, `app.svg`, into the two rasters the build takes:
# `app.png` for the window and `app.ico` for the Windows executable. Both are
# committed, so a build needs neither tool; run this again after changing the SVG.
# Needs ImageMagick's `magick` with its librsvg delegate, and `python3`.
set -eu
cd "$(dirname "$0")"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
sizes="16 20 24 32 40 48 64 256"
for size in $sizes; do
    magick -background none -density 1200 app.svg -resize ${size}x$size \
        PNG32:"$work/$size.png"
done
cp "$work/256.png" app.png
# Each entry is stored as a PNG. ImageMagick writes them as bitmaps, which makes the
# 256 pixel one alone a quarter of a megabyte.
python3 - "$work" $sizes <<'END'
import struct, sys
work, sizes = sys.argv[1], [int(s) for s in sys.argv[2:]]
images = [open(f"{work}/{s}.png", "rb").read() for s in sizes]
out = struct.pack("<HHH", 0, 1, len(images))
offset = 6 + 16 * len(images)
for size, data in zip(sizes, images):
    side = size % 256
    out += struct.pack("<BBBBHHII", side, side, 0, 0, 1, 32, len(data), offset)
    offset += len(data)
open("app.ico", "wb").write(out + b"".join(images))
END
