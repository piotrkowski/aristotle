#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Convert a raw 8-bpp PW4 framebuffer dump into a PNG of the visible area.

Usage: fb2png.py RAW PNG [STRIDE WIDTH HEIGHT Y_OFFSET]
Defaults match the PW4: stride 1088, visible 1072 x 1448, no vertical offset.
"""
import sys

from PIL import Image


def main(raw, png, stride=1088, width=1072, height=1448, y_offset=0):
    data = open(raw, "rb").read()
    rows = len(data) // stride
    image = Image.frombytes("L", (stride, rows), data[: stride * rows])
    image.crop((0, y_offset, width, min(rows, y_offset + height))).save(png)


if __name__ == "__main__":
    if len(sys.argv) not in (3, 7):
        sys.exit(__doc__)
    main(sys.argv[1], sys.argv[2], *map(int, sys.argv[3:]))
