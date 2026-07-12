#!/usr/bin/env python3
"""Generate Dwarf Kingdom's event sounds (assets/sounds/*.wav).

Pure-python, deterministic, no dependencies. All audio is synthesized
from sines and filtered noise — original to Dwarf Kingdom, CC0 / public
domain. Replace any .wav with your own to reskin the soundscape.
"""
import math, os, struct, wave

RATE = 22050

def lcg(seed):
    s = seed & 0x7FFFFFFF
    while True:
        s = (1103515245 * s + 12345) & 0x7FFFFFFF
        yield (s / 0x3FFFFFFF) - 1.0  # -1..1

def env(i, n, attack=0.01, release=0.6):
    """Attack/decay envelope over n samples."""
    t = i / n
    a = min(1.0, t / max(attack, 1e-6))
    r = max(0.0, 1.0 - max(0.0, t - (1.0 - release)) / max(release, 1e-6))
    return a * r

def write_wav(name, samples):
    os.makedirs("assets/sounds", exist_ok=True)
    path = f"assets/sounds/{name}.wav"
    with wave.open(path, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(RATE)
        frames = b"".join(
            struct.pack("<h", max(-32767, min(32767, int(s * 32767)))) for s in samples
        )
        w.writeframes(frames)
    print(f"{path}: {len(samples) / RATE:.2f}s")

def sine(freq, i):
    return math.sin(2 * math.pi * freq * i / RATE)

def hit():
    """Combat: a sharp knock with a low body."""
    n = int(0.18 * RATE)
    noise = lcg(1)
    out = []
    for i in range(n):
        e = env(i, n, attack=0.002, release=0.9)
        body = 0.5 * sine(95, i) * e
        crack = 0.5 * next(noise) * env(i, n, attack=0.001, release=0.25)
        out.append((body + crack) * 0.8)
    return out

def horn():
    """Siege: a low, unfriendly two-note horn."""
    n = int(0.9 * RATE)
    out = []
    for i in range(n):
        f = 98.0 if i < n * 0.45 else 82.0
        e = env(i, n, attack=0.08, release=0.5)
        v = 0.5 * sine(f, i) + 0.25 * sine(f * 2, i) + 0.12 * sine(f * 3.01, i)
        out.append(v * e * 0.7)
    return out

def chime():
    """Artifact: a bright rising arpeggio."""
    notes = [523.25, 659.25, 783.99, 1046.5]
    seg = int(0.14 * RATE)
    out = []
    for k, f in enumerate(notes):
        for i in range(seg):
            e = env(i, seg, attack=0.01, release=0.85)
            v = (0.5 * sine(f, i) + 0.2 * sine(f * 2, i)) * e
            out.append(v * 0.55)
    # let the last note ring
    tail = int(0.3 * RATE)
    f = notes[-1]
    for i in range(tail):
        e = env(i + seg, seg + tail, attack=0.0, release=1.0)
        out.append(0.35 * sine(f, i + seg) * e)
    return out

def bell():
    """Caravan: a merchant's handbell, two quick strikes."""
    out = []
    for strike in range(2):
        n = int(0.28 * RATE)
        for i in range(n):
            e = env(i, n, attack=0.003, release=0.9)
            v = (
                0.4 * sine(1318.5, i)
                + 0.25 * sine(1975.5, i)
                + 0.12 * sine(2637.0, i)
            )
            out.append(v * e * 0.6)
    return out

def toll():
    """Burial: a single low bell, long decay."""
    n = int(1.2 * RATE)
    out = []
    for i in range(n):
        e = env(i, n, attack=0.004, release=0.97)
        v = 0.5 * sine(196.0, i) + 0.28 * sine(392.0, i) + 0.1 * sine(587.3, i)
        out.append(v * e * 0.7)
    return out

def hiss():
    """Obsidian: steam roaring off quenched magma."""
    n = int(0.7 * RATE)
    noise = lcg(7)
    out = []
    prev = 0.0
    for i in range(n):
        e = env(i, n, attack=0.05, release=0.7)
        # crude low-pass for a breathy hiss
        prev = prev * 0.6 + next(noise) * 0.4
        out.append(prev * e * 0.5)
    return out

def doom():
    """A death: one dark thud."""
    n = int(0.5 * RATE)
    out = []
    for i in range(n):
        e = env(i, n, attack=0.005, release=0.85)
        f = 65.0 * (1.0 - 0.3 * i / n)  # falling pitch
        out.append((0.6 * sine(f, i) + 0.2 * sine(f * 1.5, i)) * e * 0.8)
    return out

def fanfare():
    """The barony: a short proud figure."""
    notes = [392.0, 392.0, 523.25]
    lens = [0.12, 0.12, 0.35]
    out = []
    for f, d in zip(notes, lens):
        n = int(d * RATE)
        for i in range(n):
            e = env(i, n, attack=0.02, release=0.5)
            v = 0.45 * sine(f, i) + 0.2 * sine(f * 2, i)
            out.append(v * e * 0.6)
    return out

if __name__ == "__main__":
    write_wav("hit", hit())
    write_wav("horn", horn())
    write_wav("chime", chime())
    write_wav("bell", bell())
    write_wav("toll", toll())
    write_wav("hiss", hiss())
    write_wav("doom", doom())
    write_wav("fanfare", fanfare())
