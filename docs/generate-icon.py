#!/usr/bin/env python3
"""Generates the source icon (1024x1024 PNG) without external dependencies.

A placeholder until the design pass in P13. Motif: three stacked bars of
differing length — chat lines and a level meter at the same time.

    python3 docs/generate-icon.py crates/app/icons/source.png
    pnpm --dir ui tauri icon ../crates/app/icons/source.png \
        --output ../crates/app/icons
"""

import struct
import sys
import zlib

SIZE = 1024
BG = (0x12, 0x14, 0x1A)
SURFACE = (0x1B, 0x1E, 0x27)
ACCENT = (0x6E, 0xA8, 0xFE)
DIM = (0x4A, 0x5A, 0x7E)

# Outer corner radius of the rounded tile, as is customary for macOS icons.
CORNER = int(SIZE * 0.22)

# (y_top, y_bottom, x_left, x_right, colour) per bar
BAR_H = int(SIZE * 0.10)
GAP = int(SIZE * 0.07)
TOP = int(SIZE * 0.28)
LEFT = int(SIZE * 0.22)
BARS = [
    (TOP, TOP + BAR_H, LEFT, int(SIZE * 0.78), ACCENT),
    (TOP + BAR_H + GAP, TOP + 2 * BAR_H + GAP, LEFT, int(SIZE * 0.62), DIM),
    (TOP + 2 * (BAR_H + GAP), TOP + 3 * BAR_H + 2 * GAP, LEFT, int(SIZE * 0.70), ACCENT),
]
BAR_RADIUS = BAR_H // 2


def inside_rounded_rect(x, y, x0, y0, x1, y1, radius):
    """True when (x, y) lies inside the rectangle with rounded corners."""
    if not (x0 <= x < x1 and y0 <= y < y1):
        return False
    # Nearest corner centre
    cx = x0 + radius if x < x0 + radius else (x1 - 1 - radius if x > x1 - 1 - radius else x)
    cy = y0 + radius if y < y0 + radius else (y1 - 1 - radius if y > y1 - 1 - radius else y)
    if cx == x and cy == y:
        return True
    return (x - cx) ** 2 + (y - cy) ** 2 <= radius**2


def build_rows():
    for y in range(SIZE):
        row = bytearray()
        for x in range(SIZE):
            if not inside_rounded_rect(x, y, 0, 0, SIZE, SIZE, CORNER):
                row += bytes((0, 0, 0, 0))  # transparent outside the tile
                continue

            # Subtle vertical gradient so the surface does not look dead.
            t = y / SIZE
            colour = tuple(int(BG[i] + (SURFACE[i] - BG[i]) * t) for i in range(3))

            for y0, y1, x0, x1, bar_colour in BARS:
                if inside_rounded_rect(x, y, x0, y0, x1, y1, BAR_RADIUS):
                    colour = bar_colour
                    break

            row += bytes((*colour, 255))
        yield row


def write_png(path):
    raw = bytearray()
    for row in build_rows():
        raw.append(0)  # filter type 0 (none)
        raw += row

    def chunk(tag, data):
        out = struct.pack(">I", len(data)) + tag + data
        return out + struct.pack(">I", zlib.crc32(tag + data) & 0xFFFFFFFF)

    header = struct.pack(">IIBBBBB", SIZE, SIZE, 8, 6, 0, 0, 0)  # 8-bit RGBA
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", header)
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
        + chunk(b"IEND", b"")
    )
    with open(path, "wb") as handle:
        handle.write(png)
    print(f"{path}: {SIZE}x{SIZE}, {len(png) / 1024:.1f} KiB")


if __name__ == "__main__":
    write_png(sys.argv[1] if len(sys.argv) > 1 else "icon.png")
