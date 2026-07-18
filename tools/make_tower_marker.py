#!/usr/bin/env python3
"""One 32x32 world-map marker for a necromancer's tower: a tall lone spire with
battlements and a lit window, on a dark chip. Grayscale, full opacity (tinted an
eerie color at runtime). Writes a single-tile PNG to composite into cell 72."""
import struct, zlib, sys

PX = 32
def blank(v=0.10):
    return [[[v, v, v, 1.0] for _ in range(PX)] for _ in range(PX)]
def rect(t, x0, y0, x1, y1, v):
    for y in range(y0, y1):
        for x in range(x0, x1):
            if 0 <= x < PX and 0 <= y < PX:
                t[y][x] = [v, v, v, 1.0]

def tower():
    t = blank()
    rect(t, 12, 6, 20, 30, 0.74)          # tall shaft
    for mx in range(12, 20, 3):           # battlements
        rect(t, mx, 3, mx + 2, 6, 0.74)
    rect(t, 14, 11, 18, 15, 0.95)         # lit window (bright — the glow)
    rect(t, 15, 24, 17, 30, 0.30)         # door
    return t

def write_png(path, t):
    raw = bytearray()
    for y in range(PX):
        raw.append(0)
        for x in range(PX):
            p = t[y][x]
            raw += bytes([int(p[0]*255), int(p[1]*255), int(p[2]*255), int(p[3]*255)])
    def chunk(typ, data):
        return struct.pack(">I", len(data)) + typ + data + struct.pack(">I", zlib.crc32(typ+data)&0xffffffff)
    open(path, "wb").write(b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", PX, PX, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(bytes(raw), 9)) + chunk(b"IEND", b""))

write_png(sys.argv[1] if len(sys.argv) > 1 else "tower.png", tower())
print("wrote tower")
