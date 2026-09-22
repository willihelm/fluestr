#!/usr/bin/env python3
"""Generates the source icon (1024x1024 PNG) without external dependencies.

Motif: three stacked bars of differing length — chat lines and a level meter
at the same time, and, with the bottom one cut to a stub, the F of Fluestr.

    python3 docs/generate-icon.py crates/app/icons/source.png
    pnpm --dir ui tauri icon ../crates/app/icons/source.png \
        --output ../crates/app/icons
"""

import struct
import sys
import zlib

SIZE = 1024

# Apple's icon grid: on a 1024 canvas the body is 824 wide, centred, leaving a
# 100 px margin all round. Filling the canvas instead makes the icon sit larger
# in the Dock than every app that follows the grid.
MARGIN = 100
TILE = SIZE - 2 * MARGIN
TILE_X0 = TILE_Y0 = MARGIN

# macOS corners are a continuous curve, not a circular arc. A superellipse of
# this exponent is the usual approximation; a circular radius reads visibly
# rounder at the corner's midpoint.
SUPERELLIPSE_N = 5.0

# Vertical supersampling. Horizontal coverage is computed exactly from each
# shape's x-extent per sub-row, so only y needs sampling.
SUBROWS = 8

BG = (0x12, 0x14, 0x1A)
SURFACE = (0x1B, 0x1E, 0x27)
ACCENT = (0x6E, 0xA8, 0xFE)
DIM = (0x4A, 0x5A, 0x7E)

# Bar geometry, as fractions of the tile — not of the canvas, so the motif
# keeps its proportions inside the body rather than against the margin.
BAR_H = 0.10 * TILE
GAP = 0.07 * TILE
TOP = 0.28 * TILE
LEFT = 0.22 * TILE
# The third bar is a stub rather than a third full line: long, shorter, barely
# there is what turns the stack into an F without drawing one.
BARS = [
    (TOP, LEFT, 0.78 * TILE, ACCENT),
    (TOP + BAR_H + GAP, LEFT, 0.62 * TILE, DIM),
    (TOP + 2 * (BAR_H + GAP), LEFT, 0.34 * TILE, ACCENT),
]
BAR_RADIUS = BAR_H / 2


def tile_half_width(y):
    """Half-width of the superellipse at row centre `y`, in tile coordinates."""
    ty = 2.0 * (y - TILE / 2.0) / TILE
    if abs(ty) >= 1.0:
        return 0.0
    return (TILE / 2.0) * (1.0 - abs(ty) ** SUPERELLIPSE_N) ** (1.0 / SUPERELLIPSE_N)


def bar_span(y, y0, x0, x1):
    """x-extent of a bar with semicircular ends at row centre `y`, or None."""
    if not (y0 <= y < y0 + BAR_H):
        return None
    dy = abs(y - (y0 + BAR_H / 2.0))
    if dy >= BAR_RADIUS:
        return None
    # The ends are circles of BAR_RADIUS centred BAR_RADIUS in from each edge.
    inset = BAR_RADIUS - (BAR_RADIUS**2 - dy**2) ** 0.5
    return x0 + inset, x1 - inset


def add_span(coverage, xa, xb, weight):
    """Adds the horizontal coverage of [xa, xb) to a per-pixel accumulator."""
    xa = max(xa, 0.0)
    xb = min(xb, float(SIZE))
    if xb <= xa:
        return
    for x in range(int(xa), min(int(xb) + 1, SIZE)):
        overlap = min(xb, x + 1.0) - max(xa, float(x))
        if overlap > 0.0:
            coverage[x] += overlap * weight


def build_rows():
    weight = 1.0 / SUBROWS
    for y in range(SIZE):
        alpha = [0.0] * SIZE
        # Premultiplied colour, so a partly covered edge pixel does not pick up
        # the black it is being composited against.
        premul = [[0.0] * SIZE for _ in range(3)]

        for sub in range(SUBROWS):
            sy = y + (sub + 0.5) * weight - TILE_Y0
            half = tile_half_width(sy)
            if half <= 0.0:
                continue

            tile_cov = [0.0] * SIZE
            centre = TILE_X0 + TILE / 2.0
            add_span(tile_cov, centre - half, centre + half, weight)

            # Subtle vertical gradient so the surface does not look dead.
            t = sy / TILE
            surface = tuple(int(BG[i] + (SURFACE[i] - BG[i]) * t) for i in range(3))

            # The bars are separated by gaps, so at most one covers a sub-row.
            bar_cov = None
            bar_colour = None
            for y0, x0, x1, colour in BARS:
                span = bar_span(sy, y0, x0, x1)
                if span is not None:
                    bar_cov = [0.0] * SIZE
                    add_span(bar_cov, TILE_X0 + span[0], TILE_X0 + span[1], weight)
                    bar_colour = colour
                    break

            for x in range(SIZE):
                a = tile_cov[x]
                if a <= 0.0:
                    continue
                alpha[x] += a
                if bar_cov is None:
                    for i in range(3):
                        premul[i][x] += surface[i] * a
                    continue
                b = min(bar_cov[x], a)
                for i in range(3):
                    premul[i][x] += bar_colour[i] * b + surface[i] * (a - b)

        row = bytearray()
        for x in range(SIZE):
            a = alpha[x]
            if a <= 0.0:
                row += bytes((0, 0, 0, 0))
                continue
            row += bytes(
                (
                    *(min(255, max(0, round(premul[i][x] / a))) for i in range(3)),
                    min(255, max(0, round(a * 255.0))),
                )
            )
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
