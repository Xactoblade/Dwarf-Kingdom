#!/usr/bin/env python3
"""Generate the default Dwarf Kingdom tileset (assets/tileset.png).

Pure-python PNG writer — no dependencies. 32x32 detailed pixel-art tiles,
procedurally drawn and deterministic.

Two kinds of glyphs:
- TERRAIN (wall/floor/stairs/ramp/gate/farm/block/boulder/seed/crop):
  drawn in shaded grayscale and TINTED by material/plant color at runtime,
  so depth fog, water tinting, and selection highlights keep working.
- SPRITES (dwarf/raider/meal/drink/artifact/still/kitchen/lever):
  full-color, rendered with a white tint.

This art is original to Dwarf Kingdom and released as CC0 / public domain.
To import other art (downloaded or AI-generated): replace assets/tileset.png
and update data/tileset.ron (tile_px, columns, glyph indices, tinted list).
"""
import struct, zlib, os

PX = 32
COLS = 6

def rng(seed):
    s = seed & 0xFFFFFFFF
    while True:
        s = (1103515245 * s + 12345) & 0x7FFFFFFF
        yield s / 0x7FFFFFFF

class Tile:
    def __init__(self, seed=1):
        self.p = [[(0, 0, 0, 0)] * PX for _ in range(PX)]
        self.r = rng(seed)

    def rand(self):
        return next(self.r)

    def set(self, x, y, c):
        if 0 <= x < PX and 0 <= y < PX:
            self.p[y][x] = c

    def fill(self, x0, y0, x1, y1, c):
        for y in range(y0, y1):
            for x in range(x0, x1):
                self.set(x, y, c)

    def speckle(self, x0, y0, x1, y1, c, density):
        for y in range(y0, y1):
            for x in range(x0, x1):
                if self.rand() < density:
                    self.set(x, y, c)

    def disc(self, cx, cy, rad, c):
        for y in range(PX):
            for x in range(PX):
                if (x - cx) ** 2 + (y - cy) ** 2 <= rad * rad:
                    self.set(x, y, c)

def gray(v, a=255):
    return (v, v, v, a)

# ------------------------------------------------------------- terrain tiles

def t_wall():
    t = Tile(11)
    # A rough natural rock face (this is dug stone, not masonry): irregular
    # blocky facets with lit tops and shadowed undersides, veined with cracks.
    t.fill(0, 0, PX, PX, gray(138))
    for _ in range(15):
        fx = int(t.rand() * (PX - 5))
        fy = int(t.rand() * (PX - 5))
        fw = 5 + int(t.rand() * 9)
        fh = 5 + int(t.rand() * 9)
        v = 118 + int(t.rand() * 66)
        x1, y1 = min(PX, fx + fw), min(PX, fy + fh)
        t.fill(fx, fy, x1, y1, gray(v))
        t.fill(fx, fy, x1, fy + 1, gray(min(v + 42, 236)))       # lit top edge
        t.fill(fx, y1 - 1, x1, y1, gray(max(v - 46, 44)))        # shadowed base
    for _ in range(4):                                            # cracks
        x = int(t.rand() * PX)
        y = 0
        while y < PX:
            t.set(x, y, gray(70))
            y += 1 + int(t.rand() * 2)
            x = max(0, min(PX - 1, x + (-1 if t.rand() < 0.5 else 1)))
    t.speckle(0, 0, PX, PX, gray(108), 0.10)
    t.speckle(0, 0, PX, PX, gray(178), 0.06)
    return t

def t_water():
    t = Tile(55)
    # Rippled water: banded ripple lines with the odd bright glint. Grayscale,
    # tinted to a depth-graded blue at runtime.
    t.fill(0, 0, PX, PX, gray(150))
    for y in range(0, PX, 4):
        t.fill(0, y, PX, y + 1, gray(120))
        t.fill(0, y + 2, PX, y + 3, gray(186))
    for _ in range(26):
        x = int(t.rand() * (PX - 4))
        y = int(t.rand() * PX)
        t.fill(x, y, x + 3 + int(t.rand() * 2), y + 1, gray(212))
    return t

def t_floor():
    t = Tile(22)
    t.fill(0, 0, PX, PX, gray(190))
    # Large irregular flagstones.
    for y in range(PX):
        for x in range(PX):
            v = 175 + int(20 * (((x * 7 + y * 13) % 23) / 23))
            t.set(x, y, gray(v))
    t.speckle(0, 0, PX, PX, gray(160), 0.10)
    t.speckle(0, 0, PX, PX, gray(210), 0.06)
    # Cracks
    x = 6
    for y in range(4, 28):
        x += -1 if t.rand() < 0.4 else (1 if t.rand() < 0.5 else 0)
        t.set(max(0, min(PX - 1, x)), y, gray(150))
    return t

def t_stairs():
    t = Tile(33)
    t.fill(0, 0, PX, PX, gray(60, 255))
    steps = 4
    h = PX // steps
    for s in range(steps):
        top = s * h
        inset = s * 3
        t.fill(inset, top, PX - inset, top + h, gray(120 + s * 22))
        t.fill(inset, top, PX - inset, top + 2, gray(min(150 + s * 25, 240)))
        t.fill(inset, top + h - 1, PX - inset, top + h, gray(70 + s * 15))
    return t

def t_ramp():
    t = Tile(44)
    # Textured ground base so the slope reads as terrain, not a black wedge.
    for y in range(PX):
        for x in range(PX):
            v = 95 + int(14 * (((x * 7 + y * 13) % 23) / 23))
            t.set(x, y, gray(v))
    for y in range(PX):
        for x in range(PX):
            if x + (PX - 1 - y) >= PX - 2:
                base = 130 + int((x - y + PX) * 60 / (2 * PX))
                t.set(x, y, gray(base))
    t.speckle(0, 0, PX, PX, gray(105), 0.05)
    for i in range(PX):
        t.set(i, PX - 1 - max(0, i - 2), gray(235))
    return t

def t_gate():
    t = Tile(55)
    # Heavy portcullis: thick vertical bars, two crossbeams, dark backdrop.
    t.fill(0, 0, PX, PX, gray(45))
    for bx in range(2, PX, 6):
        t.fill(bx, 0, bx + 3, PX, gray(170))
        t.fill(bx, 0, bx + 1, PX, gray(215))
    for by in (6, 22):
        t.fill(0, by, PX, by + 4, gray(190))
        t.fill(0, by, PX, by + 1, gray(230))
        t.fill(0, by + 3, PX, by + 4, gray(120))
        for bx in range(4, PX, 8):
            t.fill(bx, by + 1, bx + 2, by + 3, gray(90))  # rivets
    return t

def t_farm():
    t = Tile(66)
    t.fill(0, 0, PX, PX, gray(120))
    for row in range(0, PX, 8):
        t.fill(0, row, PX, row + 3, gray(85))       # furrow shadow
        t.fill(0, row + 3, PX, row + 5, gray(150))  # ridge light
    t.speckle(0, 0, PX, PX, gray(100), 0.15)
    t.speckle(0, 0, PX, PX, gray(165), 0.05)
    return t

def t_block():
    t = Tile(77)
    for y in range(PX):
        for x in range(PX):
            v = 195 + int(18 * (((x * 5 + y * 3) % 17) / 17))
            t.set(x, y, gray(v))
    t.speckle(0, 0, PX, PX, gray(175), 0.08)
    return t

def t_boulder():
    t = Tile(88)
    t.disc(16, 18, 11, gray(140))
    t.disc(13, 15, 8, gray(170))
    t.disc(11, 13, 4, gray(200))
    t.disc(20, 22, 6, gray(110))
    t.speckle(6, 8, 26, 28, gray(120), 0.10)
    return t

def t_seed():
    t = Tile(99)
    for cx, cy in ((10, 12), (20, 10), (14, 21), (23, 20)):
        t.disc(cx, cy, 3, gray(150))
        t.disc(cx - 1, cy - 1, 1, gray(210))
    return t

def t_crop():
    t = Tile(111)
    # A leafy plant: stalk + drooping leaves, grayscale for plant tinting.
    for y in range(8, 28):
        t.fill(15, y, 17, y + 1, gray(150))
    for (sx, sy, d) in ((15, 12, -1), (16, 10, 1), (15, 16, -1), (16, 18, 1), (15, 21, -1)):
        x, y = sx, sy
        for i in range(8):
            x += d
            y += 1 if i % 3 == 2 else 0
            t.fill(x, y, x + 2, y + 2, gray(180))
            t.set(x, y, gray(215))
    t.fill(13, 26, 20, 28, gray(110))
    return t

# -------------------------------------------------------------- full sprites

SKIN = (232, 190, 148, 255)
SKIN_D = (198, 152, 110, 255)
BEARD = (168, 108, 48, 255)
BEARD_D = (128, 78, 30, 255)
TUNIC = (58, 96, 158, 255)
TUNIC_D = (40, 70, 122, 255)
BOOT = (86, 60, 38, 255)
HELM = (150, 155, 165, 255)
HELM_D = (110, 115, 128, 255)
OUT = (28, 24, 22, 255)

def outline(t):
    # 1px dark outline around any opaque pixel cluster.
    src = [row[:] for row in t.p]
    for y in range(PX):
        for x in range(PX):
            if src[y][x][3] == 0:
                near = False
                for dy in (-1, 0, 1):
                    for dx in (-1, 0, 1):
                        nx, ny = x + dx, y + dy
                        if 0 <= nx < PX and 0 <= ny < PX and src[ny][nx][3] > 0:
                            near = True
                if near:
                    t.set(x, y, OUT)

def t_dwarf():
    t = Tile(123)
    # Boots
    t.fill(10, 27, 15, 30, BOOT); t.fill(17, 27, 22, 30, BOOT)
    # Legs
    t.fill(11, 23, 15, 27, TUNIC_D); t.fill(17, 23, 21, 27, TUNIC_D)
    # Body (broad!)
    t.fill(8, 15, 24, 23, TUNIC)
    t.fill(8, 15, 24, 17, (78, 118, 182, 255))
    # Belt
    t.fill(8, 20, 24, 22, (120, 90, 40, 255))
    t.fill(14, 20, 18, 22, (208, 176, 60, 255))
    # Arms
    t.fill(5, 15, 8, 22, TUNIC_D); t.fill(24, 15, 27, 22, TUNIC_D)
    t.fill(5, 21, 8, 24, SKIN_D); t.fill(24, 21, 27, 24, SKIN_D)
    # Head
    t.fill(11, 6, 21, 13, SKIN)
    t.fill(11, 6, 21, 8, SKIN_D)
    # Eyes
    t.set(13, 9, OUT); t.set(18, 9, OUT)
    # Beard — the point of the whole exercise
    t.fill(10, 11, 22, 15, BEARD)
    t.fill(11, 15, 21, 18, BEARD)
    t.fill(13, 18, 19, 20, BEARD_D)
    t.fill(10, 11, 22, 12, BEARD_D)
    # Nose over beard
    t.fill(15, 10, 17, 12, SKIN_D)
    # Helmet
    t.fill(10, 3, 22, 7, HELM)
    t.fill(10, 3, 22, 4, HELM_D)
    t.fill(8, 6, 24, 7, HELM_D)
    outline(t)
    return t

def t_raider():
    t = Tile(134)
    GSKIN = (128, 158, 96, 255)
    GSKIN_D = (98, 124, 70, 255)
    ARMOR = (78, 70, 66, 255)
    ARMOR_D = (54, 48, 45, 255)
    # Boots / legs
    t.fill(10, 27, 14, 30, ARMOR_D); t.fill(18, 27, 22, 30, ARMOR_D)
    t.fill(11, 22, 15, 27, ARMOR); t.fill(17, 22, 21, 27, ARMOR)
    # Lean body
    t.fill(9, 14, 23, 22, ARMOR)
    t.fill(9, 14, 23, 16, ARMOR_D)
    # Spiked pauldrons
    t.fill(6, 13, 10, 17, ARMOR_D); t.fill(22, 13, 26, 17, ARMOR_D)
    t.set(7, 12, ARMOR_D); t.set(24, 12, ARMOR_D)
    # Arms
    t.fill(6, 17, 9, 23, GSKIN_D); t.fill(23, 17, 26, 23, GSKIN_D)
    # Head
    t.fill(12, 5, 20, 13, GSKIN)
    t.fill(12, 5, 20, 7, GSKIN_D)
    # Ears
    t.fill(9, 7, 12, 10, GSKIN_D); t.fill(20, 7, 23, 10, GSKIN_D)
    # Red eyes, fangs
    t.set(14, 8, (200, 40, 40, 255)); t.set(18, 8, (200, 40, 40, 255))
    t.fill(14, 11, 15, 13, (240, 236, 220, 255))
    t.fill(17, 11, 18, 13, (240, 236, 220, 255))
    outline(t)
    return t

def t_meal():
    t = Tile(145)
    BOWL = (156, 96, 48, 255)
    BOWL_D = (118, 70, 34, 255)
    STEW = (198, 140, 60, 255)
    t.fill(6, 16, 26, 24, BOWL)
    t.fill(6, 22, 26, 24, BOWL_D)
    t.fill(8, 14, 24, 17, STEW)
    for cx, cy in ((11, 14), (17, 13), (21, 15)):
        t.disc(cx, cy, 1, (230, 190, 120, 255))
    # Steam
    for x, y0 in ((12, 6), (19, 4)):
        for i in range(6):
            t.set(x + (1 if i % 2 else 0), y0 + i, (235, 235, 235, 160))
    t.fill(10, 25, 22, 27, BOWL_D)
    outline(t)
    return t

def t_drink():
    t = Tile(156)
    WOOD = (140, 96, 50, 255)
    WOOD_D = (104, 70, 36, 255)
    FOAM = (244, 238, 210, 255)
    t.fill(8, 10, 22, 27, WOOD)
    for bx in (10, 14, 18):
        t.fill(bx, 10, bx + 1, 27, WOOD_D)
    t.fill(8, 24, 22, 27, WOOD_D)
    # Handle
    t.fill(22, 13, 26, 15, WOOD_D); t.fill(22, 20, 26, 22, WOOD_D)
    t.fill(24, 13, 26, 22, WOOD_D)
    # Foam overflowing
    t.fill(7, 7, 23, 11, FOAM)
    for cx in (8, 12, 17, 21):
        t.disc(cx, 7, 2, FOAM)
    outline(t)
    return t

def t_artifact():
    t = Tile(167)
    GOLD = (232, 190, 60, 255)
    GOLD_D = (176, 136, 30, 255)
    GEM = (86, 200, 220, 255)
    # A jeweled crown
    t.fill(7, 14, 25, 24, GOLD)
    t.fill(7, 22, 25, 24, GOLD_D)
    for px, ph in ((7, 8), (13, 11), (19, 8)):
        t.fill(px, 14 - ph, px + 6, 15, GOLD)
        t.set(px + 2, 14 - ph - 1, GOLD)
        t.set(px + 3, 14 - ph - 1, GOLD)
    t.disc(16, 18, 2, GEM)
    t.disc(10, 19, 1, (220, 80, 120, 255))
    t.disc(22, 19, 1, (120, 220, 120, 255))
    # Sparkles
    for sx, sy in ((5, 8), (27, 10), (25, 4)):
        t.set(sx, sy, (255, 255, 255, 255))
        t.set(sx - 1, sy, (255, 255, 255, 140)); t.set(sx + 1, sy, (255, 255, 255, 140))
        t.set(sx, sy - 1, (255, 255, 255, 140)); t.set(sx, sy + 1, (255, 255, 255, 140))
    outline(t)
    return t

def t_still():
    t = Tile(178)
    COPPER = (188, 116, 66, 255)
    COPPER_D = (142, 84, 46, 255)
    COPPER_L = (222, 152, 96, 255)
    # Pot belly
    t.disc(14, 20, 9, COPPER)
    t.disc(11, 17, 4, COPPER_L)
    t.fill(5, 26, 24, 29, COPPER_D)
    # Neck and coil
    t.fill(12, 6, 17, 12, COPPER)
    t.fill(17, 7, 26, 9, COPPER_D)
    t.fill(24, 9, 26, 18, COPPER_D)
    t.disc(25, 20, 2, (120, 170, 220, 255))  # drip
    outline(t)
    return t

def t_kitchen():
    t = Tile(189)
    IRON = (88, 88, 96, 255)
    IRON_D = (60, 60, 68, 255)
    FIRE = (238, 140, 40, 255)
    # Cauldron
    t.fill(7, 12, 25, 24, IRON)
    t.fill(7, 21, 25, 24, IRON_D)
    t.fill(5, 12, 27, 15, IRON_D)
    t.fill(9, 10, 23, 13, (120, 170, 90, 255))  # bubbling stew
    # Fire below
    for fx, fh in ((10, 4), (14, 6), (18, 5), (22, 3)):
        t.fill(fx, 30 - fh, fx + 2, 30, FIRE)
        t.set(fx, 30 - fh - 1, (250, 210, 80, 255))
    t.fill(6, 29, 26, 31, (90, 60, 34, 255))  # logs
    outline(t)
    return t

def t_lever():
    t = Tile(199)
    STONE = (130, 130, 138, 255)
    WOOD = (140, 96, 50, 255)
    KNOB = (200, 60, 50, 255)
    t.fill(8, 22, 24, 29, STONE)
    t.fill(8, 27, 24, 29, gray(95))
    # Diagonal arm
    x, y = 15, 22
    for i in range(11):
        t.fill(x, y, x + 3, y + 2, WOOD)
        x += 1; y -= 2
    t.disc(26, 3, 3, KNOB)
    t.disc(25, 2, 1, (240, 140, 130, 255))
    outline(t)
    return t

def t_tomb():
    t = Tile(210)
    STONE = (168, 168, 176, 255)
    STONE_D = (120, 120, 130, 255)
    # A headstone on a low mound.
    t.fill(4, 24, 28, 29, (110, 90, 60, 255))
    t.fill(6, 23, 26, 25, (90, 120, 60, 255))
    t.fill(11, 8, 21, 24, STONE)
    t.disc(16, 9, 5, STONE)
    t.fill(11, 8, 13, 24, STONE_D)
    t.fill(11, 22, 21, 24, STONE_D)
    # An inscription
    for iy in (12, 15, 18):
        t.fill(14, iy, 19, iy + 1, STONE_D)
    outline(t)
    return t

def t_cow():
    t = Tile(221)
    HIDE = (196, 168, 132, 255)
    HIDE_D = (150, 124, 92, 255)
    SPOT = (78, 60, 44, 255)
    # A stocky quadruped, side view.
    t.fill(6, 24, 9, 30, HIDE_D)   # legs
    t.fill(12, 24, 15, 30, HIDE_D)
    t.fill(18, 24, 21, 30, HIDE_D)
    t.fill(23, 24, 26, 30, HIDE_D)
    t.fill(5, 12, 27, 25, HIDE)    # broad body
    t.fill(5, 12, 27, 14, HIDE_D)  # back shadow
    # Head at the right
    t.fill(24, 14, 30, 22, HIDE)
    t.fill(28, 16, 30, 20, (60, 48, 36, 255))  # muzzle
    t.fill(25, 11, 27, 14, HIDE_D)  # horn/ear
    # Spots
    t.disc(11, 18, 3, SPOT)
    t.disc(18, 20, 2, SPOT)
    t.set(29, 30, HIDE_D)  # tail hint
    t.fill(4, 15, 6, 25, HIDE_D)   # rump/tail
    outline(t)
    return t

def t_sheep():
    t = Tile(232)
    WOOL = (232, 228, 220, 255)
    WOOL_D = (188, 184, 176, 255)
    FACE = (72, 66, 60, 255)
    t.fill(8, 25, 11, 30, FACE)    # legs
    t.fill(13, 25, 16, 30, FACE)
    t.fill(19, 25, 22, 30, FACE)
    # Fluffy body: overlapping discs
    for cx, cy in ((11, 18), (16, 16), (21, 18), (14, 21), (19, 21)):
        t.disc(cx, cy, 5, WOOL)
    for cx, cy in ((12, 21), (18, 22)):
        t.disc(cx, cy, 3, WOOL_D)
    # Head
    t.fill(22, 16, 28, 23, FACE)
    t.set(24, 18, WOOL); t.set(26, 18, WOOL)  # eyes
    t.fill(23, 14, 25, 16, FACE)  # ear
    outline(t)
    return t

def t_dog():
    t = Tile(94)
    FUR = (150, 116, 80, 255)
    FUR_D = (110, 82, 54, 255)
    FUR_L = (186, 152, 112, 255)
    NOSE = (40, 32, 26, 255)
    # A lean side-view hound: four legs, a low body, a raised tail.
    t.fill(7, 23, 9, 29, FUR_D)    # legs
    t.fill(11, 23, 13, 29, FUR_D)
    t.fill(18, 23, 20, 29, FUR_D)
    t.fill(22, 23, 24, 29, FUR_D)
    t.fill(6, 15, 25, 24, FUR)     # body
    t.fill(6, 15, 25, 17, FUR_L)   # back highlight
    # Chest and head at the right
    t.fill(23, 12, 29, 21, FUR)    # head
    t.fill(27, 15, 30, 19, FUR_D)  # muzzle
    t.set(29, 16, NOSE); t.set(29, 17, NOSE)
    t.set(25, 14, NOSE)            # eye
    t.fill(22, 9, 25, 13, FUR_D)   # pointed ear
    # Tail sweeping up from the left rump
    t.fill(3, 12, 6, 15, FUR)
    t.fill(2, 10, 4, 13, FUR_L)
    outline(t)
    return t

def t_weapon():
    t = Tile(151)
    STEEL = (198, 204, 214, 255)
    STEEL_L = (232, 236, 244, 255)
    STEEL_D = (140, 146, 158, 255)
    GUARD = (150, 120, 60, 255)
    GRIP = (96, 66, 40, 255)
    # A sword laid diagonally: hilt at lower-left, point at upper-right.
    for i in range(20):
        x = 7 + i
        y = 24 - i
        t.set(x, y, STEEL)
        t.set(x, y - 1, STEEL_L)   # bright edge
        t.set(x + 1, y, STEEL_D)   # shaded edge
    t.set(27, 4, STEEL_L); t.set(26, 5, STEEL)  # point
    for j in range(-3, 4):
        t.set(9 + j, 22 + j, GUARD)  # crossguard
    t.fill(5, 24, 8, 28, GRIP)       # grip
    t.set(4, 28, GUARD); t.set(5, 28, GUARD)  # pommel
    outline(t)
    return t

def t_tree():
    t = Tile(77)
    # A tree seen from above: a rounded, lumpy mass of foliage with a lit
    # upper-left and a shaded underside, plus a hint of trunk at the base.
    # Grayscale, so it tints to the wood's (and the season's) color at runtime.
    cx, cy = 16, 15
    t.disc(cx, cy + 1, 12, gray(90))       # underside shadow
    t.disc(cx, cy, 12, gray(140))          # canopy body
    t.disc(cx - 2, cy - 2, 9, gray(172))   # lit upper-left mass
    for (lx, ly, r, v) in ((10, 10, 4, 188), (21, 12, 4, 162), (14, 8, 3, 198),
                           (21, 19, 4, 150), (11, 20, 3, 150), (16, 14, 3, 178)):
        t.disc(lx, ly, r, gray(v))
    t.speckle(4, 3, 28, 27, gray(205), 0.06)  # leaf glints
    t.speckle(4, 3, 28, 27, gray(105), 0.06)  # leaf shadows
    t.fill(15, 25, 18, 31, gray(78))          # trunk hint
    return t

def spike(t, cx, cy, tx, ty, w, c):
    # A tapering spoke from (cx, cy) out to (tx, ty): width w at the base,
    # narrowing to a point at the tip.
    n = max(abs(tx - cx), abs(ty - cy), 1)
    for i in range(n + 1):
        f = i / n
        x = round(cx + (tx - cx) * f)
        y = round(cy + (ty - cy) * f)
        ww = max(0, int(round(w * (1 - f))))
        t.fill(x - ww, y - ww, x + ww + 1, y + ww + 1, c)

def t_conifer():
    t = Tile(88)
    # A conifer from above: a tight, spiky radial crown of dark needles.
    cx, cy = 16, 16
    for (tx, ty) in ((16, 1), (16, 31), (1, 16), (31, 16),
                     (5, 5), (27, 5), (5, 27), (27, 27)):
        spike(t, cx, cy, tx, ty, 3, gray(112))
    t.disc(cx, cy + 1, 7, gray(80))       # shaded core
    t.disc(cx, cy, 7, gray(138))          # dense crown centre
    t.disc(cx - 1, cy - 1, 4, gray(168))  # lit peak
    t.speckle(4, 4, 28, 28, gray(96), 0.10)
    t.fill(15, 29, 18, 31, gray(70))      # trunk hint
    return t

def t_willow():
    t = Tile(99)
    # A weeping willow: a broad canopy with long drooping tendrils.
    cx, cy = 16, 12
    t.disc(cx, cy + 1, 11, gray(85))
    t.disc(cx, cy, 11, gray(140))
    t.disc(cx - 2, cy - 2, 7, gray(172))
    for x0 in (7, 11, 16, 21, 25):
        x = x0
        for i in range(9):
            t.set(x, cy + 8 + i, gray(max(70, 130 - i * 6)))
            if i % 3 == 2:
                x += 1 if x0 > 16 else -1
    t.speckle(4, 2, 28, 22, gray(182), 0.05)
    t.speckle(4, 2, 28, 22, gray(104), 0.05)
    return t

def t_birch():
    t = Tile(120)
    # A slender birch: a small high canopy on a thin, pale, dappled trunk.
    cx, cy = 16, 11
    t.fill(15, 12, 18, 31, gray(198))
    for yy in range(15, 30, 4):
        t.set(15, yy, gray(88)); t.set(17, yy + 1, gray(88))  # bark marks
    t.disc(cx, cy + 1, 8, gray(118))
    t.disc(cx, cy, 8, gray(158))
    t.disc(cx - 1, cy - 1, 5, gray(190))
    for (lx, ly, r, v) in ((10, 8, 3, 176), (21, 9, 3, 150), (15, 5, 3, 196), (19, 15, 3, 150)):
        t.disc(lx, ly, r, gray(v))
    t.speckle(6, 2, 26, 19, gray(202), 0.06)
    return t

# --- Textured ground tiles (tinted grayscale). Several variants of each, so
# the field is broken up per-tile instead of reading as a flat colored grid.

def _grass(seed, pebble=False):
    t = Tile(seed)
    t.fill(0, 0, PX, PX, gray(158))              # bright turf base
    t.speckle(0, 0, PX, PX, gray(132), 0.14)     # a touch of soil
    t.speckle(0, 0, PX, PX, gray(190), 0.16)
    for _ in range(95):                           # blades of grass
        bx = int(t.rand() * PX)
        by = 3 + int(t.rand() * (PX - 5))
        h = 2 + int(t.rand() * 4)
        shade = gray(185 + int(t.rand() * 70))    # bright blade tips
        lean = -1 if t.rand() < 0.5 else 1
        x = bx
        for i in range(h):
            t.set(x, by - i, shade)
            if i % 2 == 1:
                x += lean
    if pebble:                                    # an odd stone in the turf
        t.disc(23, 24, 3, gray(120))
        t.disc(22, 23, 2, gray(175))
    return t

def t_grass_a(): return _grass(201)
def t_grass_b(): return _grass(202)
def t_grass_c(): return _grass(203, pebble=True)

def _dirt(seed):
    t = Tile(seed)
    t.fill(0, 0, PX, PX, gray(128))
    t.speckle(0, 0, PX, PX, gray(104), 0.28)      # grain
    t.speckle(0, 0, PX, PX, gray(150), 0.12)
    for _ in range(9):                            # pebbles
        px_ = 2 + int(t.rand() * (PX - 4))
        py_ = 2 + int(t.rand() * (PX - 4))
        r = 1 + int(t.rand() * 2)
        t.disc(px_, py_, r, gray(94))
        t.disc(px_ - 1, py_ - 1, max(1, r - 1), gray(152))
    return t

def t_dirt_a(): return _dirt(211)
def t_dirt_b(): return _dirt(212)

def _rock(seed):
    t = Tile(seed)
    t.fill(0, 0, PX, PX, gray(140))
    t.speckle(0, 0, PX, PX, gray(118), 0.20)      # grit
    t.speckle(0, 0, PX, PX, gray(168), 0.10)
    for _ in range(3):                            # jagged cracks
        x = int(t.rand() * PX)
        y = 0
        while y < PX:
            t.set(x, y, gray(78))
            t.set(min(PX - 1, x + 1), y, gray(96))
            y += 1 + int(t.rand() * 2)
            x = max(0, min(PX - 1, x + (-1 if t.rand() < 0.5 else 1)))
    for _ in range(4):                            # lit facets
        fx = 3 + int(t.rand() * (PX - 8))
        fy = 3 + int(t.rand() * (PX - 8))
        t.fill(fx, fy, fx + 3, fy + 2, gray(175))
    return t

def t_rock_a(): return _rock(221)
def t_rock_b(): return _rock(222)

def t_ws_forge():
    t = Tile(301)
    iron = (78, 80, 90, 255); iron_d = (52, 54, 62, 255); iron_l = (122, 124, 134, 255)
    fire = (240, 130, 36, 255); ember = (250, 200, 70, 255)
    t.fill(11, 24, 21, 30, iron_d)          # stump
    t.fill(7, 22, 25, 26, iron_d)           # base
    t.fill(6, 17, 20, 22, iron)             # body
    t.fill(6, 17, 20, 18, iron_l)           # top highlight
    t.fill(19, 18, 25, 21, iron)            # horn
    t.disc(26, 24, 4, fire); t.disc(26, 24, 2, ember)  # forge glow
    outline(t)
    return t

def t_ws_furnace():
    t = Tile(302)
    stone = (128, 122, 112, 255); stone_d = (92, 86, 78, 255); stone_l = (162, 156, 146, 255)
    fire = (245, 140, 40, 255); glow = (255, 210, 90, 255)
    t.fill(6, 5, 26, 30, stone)
    t.fill(6, 5, 26, 7, stone_l); t.fill(6, 28, 26, 30, stone_d)
    for yy in range(9, 30, 4):
        t.fill(6, yy, 26, yy + 1, stone_d)
    t.fill(11, 17, 21, 27, (28, 18, 14, 255))  # mouth
    t.disc(16, 23, 4, fire); t.disc(16, 23, 2, glow)
    outline(t)
    return t

def t_ws_loom():
    t = Tile(303)
    wood = (146, 100, 54, 255); wood_d = (104, 70, 36, 255); thread = (222, 214, 196, 255)
    t.fill(6, 5, 9, 29, wood); t.fill(23, 5, 26, 29, wood)   # posts
    t.fill(6, 5, 26, 8, wood_d); t.fill(6, 26, 26, 29, wood_d)
    for tx in range(11, 22, 2):
        t.fill(tx, 8, tx + 1, 26, thread)   # warp threads
    t.fill(9, 16, 23, 18, wood_d)           # shuttle bar
    outline(t)
    return t

def t_ws_mason():
    t = Tile(304)
    wood = (140, 96, 50, 255); wood_d = (100, 68, 34, 255)
    stone = (150, 148, 154, 255); stone_l = (186, 184, 190, 255); stone_d = (110, 108, 114, 255)
    steel = (200, 204, 212, 255)
    t.fill(4, 22, 28, 25, wood); t.fill(4, 25, 28, 27, wood_d)
    t.fill(6, 25, 8, 30, wood_d); t.fill(24, 25, 26, 30, wood_d)
    t.fill(11, 12, 22, 22, stone); t.fill(11, 12, 22, 14, stone_l); t.fill(11, 20, 22, 22, stone_d)
    t.fill(6, 10, 8, 18, steel); t.fill(5, 8, 9, 11, wood_d)  # chisel
    outline(t)
    return t

def t_ws_carpenter():
    t = Tile(305)
    log = (150, 108, 62, 255); log_d = (110, 78, 44, 255); ring = (178, 140, 92, 255)
    steel = (200, 204, 212, 255)
    for (cx, cy) in ((10, 23), (19, 23), (14, 16)):
        t.disc(cx, cy, 5, log); t.disc(cx, cy, 3, ring); t.disc(cx, cy, 1, log_d)
    for i in range(15):
        t.set(8 + i, 8 + i // 2, steel)
        if i % 2 == 0:
            t.set(8 + i, 9 + i // 2, (140, 146, 158, 255))  # saw teeth
    t.fill(5, 5, 9, 9, log_d)  # saw handle
    outline(t)
    return t

def t_ws_bench():
    t = Tile(306)
    wood = (150, 110, 60, 255); wood_d = (108, 78, 42, 255)
    steel = (198, 204, 214, 255); gold = (228, 190, 70, 255); red = (190, 70, 60, 255)
    t.fill(4, 20, 28, 24, wood); t.fill(4, 24, 28, 26, wood_d)
    t.fill(6, 24, 8, 30, wood_d); t.fill(24, 24, 26, 30, wood_d)
    t.fill(8, 12, 10, 20, steel)      # a tool
    t.disc(15, 16, 3, gold)           # a trinket
    t.fill(20, 13, 24, 20, red)       # a craft in progress
    outline(t)
    return t

def t_ws_cloth():
    t = Tile(307)
    cloth = (198, 176, 214, 255); cloth_d = (150, 130, 168, 255)
    steel = (210, 214, 222, 255); thread = (230, 220, 200, 255)
    t.fill(6, 16, 26, 20, cloth); t.fill(6, 20, 26, 24, cloth_d); t.fill(6, 24, 26, 28, cloth)
    for yy in (18, 22, 26):
        t.fill(6, yy, 26, yy + 1, cloth_d)
    t.fill(20, 6, 22, 16, steel)      # needle
    for i in range(8):
        t.set(21 + (i % 3) - 1, 6 + i, thread)
    outline(t)
    return t

def t_ws_hides():
    t = Tile(308)
    wood = (130, 92, 50, 255)
    hide = (176, 138, 96, 255); hide_d = (140, 106, 70, 255); hide_l = (202, 166, 124, 255)
    t.fill(5, 5, 8, 29, wood); t.fill(24, 5, 27, 29, wood); t.fill(5, 5, 27, 8, wood)
    t.fill(9, 9, 23, 26, hide)
    t.fill(9, 9, 23, 11, hide_l); t.fill(9, 24, 23, 26, hide_d)
    t.disc(12, 15, 2, hide_d); t.disc(19, 19, 2, hide_l)
    outline(t)
    return t

def t_ws_well():
    t = Tile(309)
    stone = (140, 136, 128, 255); stone_d = (100, 96, 90, 255); stone_l = (172, 168, 160, 255)
    wood = (140, 96, 50, 255); rope = (200, 186, 150, 255); water = (60, 130, 200, 255)
    t.fill(8, 18, 24, 29, stone); t.fill(8, 18, 24, 20, stone_l); t.fill(8, 27, 24, 29, stone_d)
    for yy in range(21, 29, 3):
        t.fill(8, yy, 24, yy + 1, stone_d)
    t.fill(11, 15, 21, 19, water)     # water in the shaft
    t.fill(8, 6, 10, 18, wood); t.fill(22, 6, 24, 18, wood)  # posts
    t.fill(6, 3, 26, 7, wood)         # roof
    t.fill(15, 7, 16, 15, rope)
    t.fill(13, 14, 18, 18, wood)      # bucket
    outline(t)
    return t

# --- Item sprites: grayscale so each takes its material/item colour at runtime
# (an oak log tints oak-brown, a steel bar steel-grey, a granite statue grey).

def i_log():
    t = Tile(401)
    t.fill(5, 13, 27, 21, gray(150))
    t.fill(5, 13, 27, 15, gray(190))
    t.fill(5, 19, 27, 21, gray(106))
    t.disc(8, 17, 4, gray(140)); t.disc(8, 17, 2, gray(186)); t.set(8, 17, gray(118))
    outline(t); return t

def i_bar():
    t = Tile(402)
    t.fill(7, 14, 25, 22, gray(165))
    t.fill(9, 12, 23, 14, gray(202))
    t.fill(7, 14, 25, 15, gray(214))
    t.fill(7, 21, 25, 22, gray(112))
    outline(t); return t

def i_barrel():
    t = Tile(403)
    for yy in range(6, 28):
        bulge = 2 if 11 < yy < 23 else 0
        t.fill(9 - bulge, yy, 23 + bulge, yy + 1, gray(150))
    t.fill(9, 6, 23, 8, gray(188))
    t.fill(7, 11, 25, 13, gray(100)); t.fill(7, 21, 25, 23, gray(100))
    for xx in range(11, 22, 4):
        for yy in range(8, 27):
            t.set(xx, yy, gray(128))
    outline(t); return t

def i_bin():
    # A squared crate — flat lid, slatted sides, banded corners. Deliberately
    # boxy so it never reads as the barrel's round cask at a glance.
    t = Tile(419)
    t.fill(6, 7, 26, 27, gray(150))
    # Lid, catching the light.
    t.fill(6, 7, 26, 12, gray(190))
    t.fill(8, 9, 24, 11, gray(206))
    # Slats down the body.
    for yy in range(14, 26, 4):
        t.fill(7, yy, 25, yy + 2, gray(122))
    # Corner bands.
    t.fill(6, 12, 9, 27, gray(104)); t.fill(23, 12, 26, 27, gray(104))
    # A shadow where it meets the floor.
    t.fill(6, 25, 26, 27, gray(96))
    outline(t); return t

def i_bed():
    t = Tile(404)
    t.fill(5, 16, 27, 24, gray(150))
    t.fill(5, 22, 27, 25, gray(108))
    t.fill(5, 12, 11, 24, gray(172))
    t.fill(7, 14, 24, 18, gray(202))
    t.fill(11, 18, 24, 22, gray(160))
    outline(t); return t

def i_clothes():
    t = Tile(405)
    t.fill(10, 8, 22, 26, gray(172))
    t.fill(6, 10, 12, 16, gray(158)); t.fill(20, 10, 26, 16, gray(158))
    t.fill(14, 8, 18, 12, gray(138))
    t.fill(10, 8, 22, 10, gray(202))
    outline(t); return t

def i_statue():
    t = Tile(406)
    t.fill(11, 24, 21, 29, gray(140)); t.fill(9, 27, 23, 29, gray(104))
    t.disc(16, 8, 3, gray(182))
    t.fill(13, 11, 19, 24, gray(166))
    t.fill(10, 13, 13, 20, gray(160)); t.fill(19, 13, 22, 20, gray(160))
    t.fill(13, 11, 19, 12, gray(200))
    outline(t); return t

def i_instrument():
    t = Tile(407)
    t.disc(13, 21, 7, gray(160)); t.disc(11, 19, 3, gray(196))
    t.fill(18, 4, 21, 20, gray(150))
    for i in range(4):
        t.set(19, 6 + i * 3, gray(118))
    for sx in (15, 17):
        for yy in range(9, 24):
            t.set(sx, yy, gray(212))
    outline(t); return t

def i_armor():
    t = Tile(408)
    t.fill(9, 8, 23, 22, gray(176))
    t.fill(9, 8, 23, 10, gray(216))
    t.disc(16, 21, 7, gray(176)); t.fill(9, 20, 23, 26, gray(156))
    t.fill(15, 10, 17, 24, gray(150))
    t.set(13, 14, gray(128)); t.set(19, 14, gray(128))
    outline(t); return t

def i_gem():
    t = Tile(409)
    for i in range(8):
        t.fill(16 - i, 8 + i, 16 + i, 9 + i, gray(182))
    for i in range(9):
        w = 8 - i
        t.fill(16 - w, 16 + i, 16 + w, 17 + i, gray(150))
    t.fill(9, 15, 23, 17, gray(212))
    for i in range(8):
        t.set(16, 16 + i, gray(120))
    t.set(11, 15, gray(232)); t.set(20, 15, gray(232))
    outline(t); return t

def i_cloth():
    t = Tile(410)
    t.fill(6, 12, 26, 24, gray(170))
    t.disc(6, 18, 6, gray(158)); t.disc(6, 18, 3, gray(192))
    for yy in range(13, 24, 3):
        t.fill(10, yy, 26, yy + 1, gray(148))
    t.fill(6, 12, 26, 14, gray(202))
    outline(t); return t

def i_leather():
    t = Tile(411)
    t.fill(7, 14, 25, 22, gray(160))
    t.disc(7, 18, 5, gray(150)); t.disc(7, 18, 2, gray(186)); t.disc(25, 18, 5, gray(150))
    t.fill(9, 15, 24, 16, gray(192))
    outline(t); return t

def i_wool():
    t = Tile(412)
    for (cx, cy, r, v) in ((13, 18, 6, 200), (19, 17, 5, 190), (16, 13, 5, 212),
                           (11, 21, 4, 186), (21, 21, 4, 186), (16, 19, 4, 200)):
        t.disc(cx, cy, r, gray(v))
    t.speckle(6, 8, 26, 26, gray(160), 0.08)
    outline(t); return t

def i_glass():
    t = Tile(413)
    t.fill(10, 6, 22, 8, gray(200))
    for i in range(7):
        t.fill(10 + i, 8 + i, 22 - i, 9 + i, gray(186))
    t.fill(15, 15, 17, 24, gray(170)); t.fill(11, 24, 21, 26, gray(182))
    t.fill(12, 8, 14, 14, gray(226))
    outline(t); return t

def i_craft():
    t = Tile(414)
    t.disc(16, 12, 4, gray(186))
    t.fill(13, 15, 19, 26, gray(166))
    t.fill(11, 24, 21, 27, gray(140))
    t.set(14, 12, gray(230)); t.set(18, 18, gray(210))
    outline(t); return t

def t_reed():
    t = Tile(160)
    # Tall reeds and cattails at the water's edge — thin stalks with the odd
    # dark seed-head. Grayscale, tinted green at runtime.
    base = PX - 2
    for _ in range(15):
        bx = 3 + int(t.rand() * (PX - 6))
        hh = 13 + int(t.rand() * 15)
        v = 130 + int(t.rand() * 95)
        x = bx
        for i in range(hh):
            t.set(x, base - i, gray(v))
            t.set(x + 1, base - i, gray(min(255, v + 22)))
            if i % 5 == 4:
                x += -1 if t.rand() < 0.5 else 1
        if t.rand() < 0.45:  # a cattail head
            for k in range(4):
                t.fill(x - 1, base - hh - k, x + 2, base - hh - k + 1, gray(96))
    return t

def t_bush():
    t = Tile(140)
    # A low leafy shrub — a rounded clump of foliage on tiny stems.
    for (lx, ly, r, v) in ((16, 18, 7, 122), (12, 15, 5, 148), (20, 14, 5, 140),
                           (16, 11, 5, 162), (10, 20, 4, 132), (22, 20, 4, 132)):
        t.disc(lx, ly, r, gray(v))
    t.speckle(6, 5, 26, 26, gray(185), 0.09)
    t.speckle(6, 5, 26, 26, gray(96), 0.08)
    t.fill(15, 24, 18, 29, gray(82))
    return t

def t_stone():
    t = Tile(150)
    # A couple of rocks resting on the ground, lit from the upper-left.
    for (cx, cy, r) in ((13, 18, 5), (21, 20, 4), (18, 12, 3)):
        t.disc(cx, cy + 1, r, gray(84))       # ground shadow
        t.disc(cx, cy, r, gray(120))
        t.disc(cx - 1, cy - 1, max(1, r - 2), gray(178))
    return t

ORDER = [
    ("wall", t_wall), ("floor", t_floor), ("stairs", t_stairs), ("ramp", t_ramp),
    ("gate", t_gate), ("farm", t_farm), ("block", t_block), ("boulder", t_boulder),
    ("seed", t_seed), ("crop", t_crop), ("dwarf", t_dwarf), ("raider", t_raider),
    ("meal", t_meal), ("drink", t_drink), ("artifact", t_artifact),
    ("still", t_still), ("kitchen", t_kitchen), ("lever", t_lever),
    ("tomb", t_tomb), ("cow", t_cow), ("sheep", t_sheep), ("dog", t_dog),
    ("weapon", t_weapon), ("tree", t_tree), ("tree_conifer", t_conifer),
    ("tree_willow", t_willow), ("tree_birch", t_birch),
    ("grass_a", t_grass_a), ("grass_b", t_grass_b), ("grass_c", t_grass_c),
    ("dirt_a", t_dirt_a), ("dirt_b", t_dirt_b), ("rock_a", t_rock_a), ("rock_b", t_rock_b),
    ("water", t_water), ("bush", t_bush), ("stone", t_stone), ("reed", t_reed),
    ("ws_forge", t_ws_forge), ("ws_furnace", t_ws_furnace), ("ws_loom", t_ws_loom),
    ("ws_mason", t_ws_mason), ("ws_carpenter", t_ws_carpenter), ("ws_bench", t_ws_bench),
    ("ws_cloth", t_ws_cloth), ("ws_hides", t_ws_hides), ("ws_well", t_ws_well),
    ("i_log", i_log), ("i_bar", i_bar), ("i_barrel", i_barrel), ("i_bin", i_bin), ("i_bed", i_bed),
    ("i_clothes", i_clothes), ("i_statue", i_statue), ("i_instrument", i_instrument),
    ("i_armor", i_armor), ("i_gem", i_gem), ("i_cloth", i_cloth), ("i_leather", i_leather),
    ("i_wool", i_wool), ("i_glass", i_glass), ("i_craft", i_craft),
]
TINTED = ["wall", "floor", "stairs", "ramp", "gate", "farm", "block", "boulder",
          "seed", "crop", "tree", "tree_conifer", "tree_willow", "tree_birch",
          "grass_a", "grass_b", "grass_c", "dirt_a", "dirt_b", "rock_a", "rock_b",
          "water", "bush", "stone", "reed",
          "i_log", "i_bar", "i_barrel", "i_bin", "i_bed", "i_clothes", "i_statue",
          "i_instrument", "i_armor", "i_gem", "i_cloth", "i_leather", "i_wool",
          "i_glass", "i_craft"]

def main():
    rows = (len(ORDER) + COLS - 1) // COLS
    W, H = COLS * PX, rows * PX
    pix = [[(0, 0, 0, 0)] * W for _ in range(H)]
    for n, (name, fn) in enumerate(ORDER):
        tile = fn()
        gx, gy = (n % COLS) * PX, (n // COLS) * PX
        for y in range(PX):
            for x in range(PX):
                pix[gy + y][gx + x] = tile.p[y][x]

    raw = b"".join(
        b"\x00" + b"".join(struct.pack("4B", *p) for p in row) for row in pix
    )
    def chunk(tag, data):
        c = tag + data
        return struct.pack(">I", len(data)) + c + struct.pack(">I", zlib.crc32(c))
    png = (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", W, H, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw, 9))
        + chunk(b"IEND", b"")
    )
    os.makedirs("assets", exist_ok=True)
    with open("assets/tileset.png", "wb") as f:
        f.write(png)
    print(f"assets/tileset.png written: {W}x{H}, {len(ORDER)} tiles, {PX}px")
    for n, (name, _) in enumerate(ORDER):
        tinted = " (tinted)" if name in TINTED else ""
        print(f"  {name} = {n}{tinted}")

if __name__ == "__main__":
    main()
