#!/usr/bin/env python3
"""Build 4 bold, map-scale terrain feature sprites (peak, hills, dune, forest)
as full 32x32 grayscale tiles (tinted by biome color at runtime), and write a
128x32 strip PNG so it can be composited into tileset.png cells 62..65.

Grayscale + full opacity: no transparency, so nothing bleeds to black on the
world map. Shading is baked in so tinting by the biome color reads as relief."""
import struct, zlib, math

PX = 32

def blank(v=0.5):
    return [[[v, v, v, 1.0] for _ in range(PX)] for _ in range(PX)]

def putpx(t, x, y, v, a=1.0):
    if 0 <= x < PX and 0 <= y < PX:
        t[y][x] = [v, v, v, a]

# ---- peak: a shaded mountain, bright snow cap, dark right face -------------
def peak():
    t = blank(0.50)
    apex_x, apex_y = 16, 5
    base_y = 29
    for y in range(PX):
        for x in range(PX):
            # triangle half-width grows with depth below the apex
            depth = y - apex_y
            if depth < 0:
                continue
            half = int(depth * 0.62)
            if abs(x - apex_x) <= half and y <= base_y:
                # left face lit, right face shadowed
                side = (x - apex_x) / (half + 1)
                v = 0.78 - 0.42 * side  # 0.78 left -> 0.36 right
                # snow cap near the apex
                if depth < 9:
                    v = min(1.0, v + 0.22)
                putpx(t, x, y, max(0.2, min(1.0, v)))
    return t

# ---- hills: two soft rounded humps ----------------------------------------
def hills():
    t = blank(0.60)
    for (cx, cy, rx, ry) in [(10, 22, 10, 9), (22, 24, 11, 8)]:
        for y in range(PX):
            for x in range(PX):
                dx = (x - cx) / rx
                dy = (y - cy) / ry
                if dx * dx + dy * dy <= 1.0 and y >= cy - ry:
                    # top lit, underside shaded
                    lit = (cy - y) / ry
                    v = 0.66 + 0.20 * lit - 0.10 * (x - cx) / rx
                    putpx(t, x, y, max(0.45, min(0.9, v)))
    return t

# ---- dune: wavy horizontal sand ridges ------------------------------------
def dune():
    t = blank(0.72)
    for y in range(PX):
        for x in range(PX):
            ridge = math.sin((x / PX) * math.pi * 2 + (y / 6.0)) * 3.0
            band = (y + ridge) % 8
            v = 0.82 if band < 3 else 0.60
            putpx(t, x, y, v)
    return t

# ---- forest: a clump of rounded canopies ----------------------------------
def forest():
    t = blank(0.42)
    canopies = [(7, 12, 6), (18, 9, 6), (25, 15, 5),
                (12, 21, 6), (23, 24, 6), (6, 26, 5)]
    for (cx, cy, r) in canopies:
        for y in range(PX):
            for x in range(PX):
                d = math.hypot(x - cx, y - cy)
                if d <= r:
                    # domed canopy: lit crown, shaded skirt
                    lit = (r - d) / r
                    v = 0.34 + 0.30 * lit
                    if (cy - y) > r * 0.4:  # sunlit top
                        v += 0.08
                    putpx(t, x, y, max(0.22, min(0.72, v)))
    return t

def write_png(path, tiles):
    """Horizontal strip of the given tiles -> one PNG."""
    w = PX * len(tiles)
    h = PX
    raw = bytearray()
    for y in range(h):
        raw.append(0)
        for ti, t in enumerate(tiles):
            for x in range(PX):
                px = t[y][x]
                raw += bytes([int(px[0] * 255), int(px[1] * 255),
                              int(px[2] * 255), int(px[3] * 255)])
    def chunk(typ, data):
        c = struct.pack(">I", len(data)) + typ + data
        return c + struct.pack(">I", zlib.crc32(typ + data) & 0xffffffff)
    sig = b"\x89PNG\r\n\x1a\n"
    ihdr = struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)
    idat = zlib.compress(bytes(raw), 9)
    with open(path, "wb") as f:
        f.write(sig + chunk(b"IHDR", ihdr) + chunk(b"IDAT", idat) + chunk(b"IEND", b""))

out = "/private/tmp/claude-501/-Users-brianadducci-Developer-Dwarf-Kingdom/6d8433b4-9ea3-4a90-917f-d2907432c612/scratchpad/map_features.png"
write_png(out, [peak(), hills(), dune(), forest()])
print("wrote", out)
