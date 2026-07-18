#!/usr/bin/env python3
"""Six distinct world-map site markers (city, fortress, hamlet, forest retreat,
dark fortress, ruins) as 32x32 grayscale tiles on a dark chip, so each site
kind reads as its own bold silhouette on the embark map (tinted by kind color
at runtime). Writes a 192x32 strip PNG to composite into tileset cells 66..71.

Full-opacity; dark base (~0.10) with a bright building silhouette, DF-style —
a settlement mark that pops against the lighter terrain around it."""
import struct, zlib

PX = 32
BASE = 0.10

def blank(v=BASE):
    return [[[v, v, v, 1.0] for _ in range(PX)] for _ in range(PX)]

def put(t, x, y, v):
    if 0 <= x < PX and 0 <= y < PX:
        t[y][x] = [v, v, v, 1.0]

def rect(t, x0, y0, x1, y1, v):
    for y in range(y0, y1):
        for x in range(x0, x1):
            put(t, x, y, v)

def vtri(t, cx, apex_y, base_y, half_at_base, v):
    """A vertical triangle (spire/roof), apex up."""
    span = base_y - apex_y
    if span <= 0:
        return
    for y in range(apex_y, base_y):
        half = int((y - apex_y) / span * half_at_base) + 1
        for x in range(cx - half, cx + half):
            put(t, x, y, v)

# ---- city: a skyline of towers ----
def city():
    t = blank()
    towers = [(3, 24, 7, 0.72), (8, 26, 8, 0.82), (14, 30, 7, 0.9),
              (21, 27, 7, 0.8), (26, 23, 6, 0.7)]
    for (x, top, w, v) in towers:
        rect(t, x, 32 - top, x + w, 30, v)
        # a few lit windows
        for wy in range(34 - top, 28, 3):
            for wx in range(x + 1, x + w - 1, 2):
                put(t, wx, wy, 0.35)
    return t

# ---- fortress: a battlemented keep with a gate ----
def fortress():
    t = blank()
    rect(t, 7, 12, 25, 29, 0.82)          # keep body
    for mx in range(7, 25, 4):            # crenellations
        rect(t, mx, 9, mx + 2, 12, 0.82)
    rect(t, 13, 19, 19, 29, 0.30)         # gate
    vtri(t, 16, 17, 22, 3, 0.5)           # arch top of gate
    return t

# ---- hamlet: one small cottage ----
def hamlet():
    t = blank()
    rect(t, 11, 17, 22, 27, 0.85)         # walls
    vtri(t, 16, 8, 17, 8, 0.65)           # roof
    rect(t, 14, 21, 18, 27, 0.30)         # door
    return t

# ---- forest retreat: conifers flanking a hut ----
def retreat():
    t = blank()
    vtri(t, 7, 5, 26, 6, 0.7)             # left conifer
    vtri(t, 25, 5, 26, 6, 0.7)            # right conifer
    rect(t, 6, 24, 8, 28, 0.4)            # trunks
    rect(t, 24, 24, 26, 28, 0.4)
    rect(t, 13, 18, 20, 27, 0.85)         # hut
    vtri(t, 16, 12, 18, 5, 0.6)           # hut roof
    return t

# ---- dark fortress: three jagged black spires ----
def darkfort():
    t = blank(0.06)
    for (cx, apex) in [(7, 6), (16, 2), (25, 7)]:
        vtri(t, cx, apex, 30, 5, 0.8)
    # sharpen: notch a dark line down each spire
    for cx in (7, 16, 25):
        for y in range(8, 30):
            put(t, cx, y, 0.24)
    return t

# ---- ruins: broken wall stubs ----
def ruins():
    t = blank()
    stubs = [(5, 15), (9, 12), (15, 20), (19, 10), (24, 17)]
    for (x, h) in stubs:
        rect(t, x, 30 - h, x + 3, 29, 0.5)
        put(t, x, 30 - h, 0.5)  # jagged top left
        put(t, x + 2, 30 - h + 1, 0.5)
    return t

def write_png(path, tiles):
    w, h = PX * len(tiles), PX
    raw = bytearray()
    for y in range(h):
        raw.append(0)
        for t in tiles:
            for x in range(PX):
                px = t[y][x]
                raw += bytes([int(px[0] * 255), int(px[1] * 255),
                              int(px[2] * 255), int(px[3] * 255)])
    def chunk(typ, data):
        c = struct.pack(">I", len(data)) + typ + data
        return c + struct.pack(">I", zlib.crc32(typ + data) & 0xffffffff)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n"
                + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(bytes(raw), 9))
                + chunk(b"IEND", b""))

import sys
out = sys.argv[1] if len(sys.argv) > 1 else "site_markers.png"
write_png(out, [city(), fortress(), hamlet(), retreat(), darkfort(), ruins()])
print("wrote", out)
