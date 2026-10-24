"""Builds the cave scene's WAVs (assets/sounds/generated/cave_drips/) from Freesound recordings.

    python3 tools/synth/cave_drips_sounds.py DL_DIR OUT_DIR      (needs numpy, scipy, soundfile)

DL_DIR holds the Freesound HQ previews as <id>.ogg (all CC0; ids, authors and cut times are in
docs/scenes/cave_drips-sources.md; a preview is linked from each sound's page as data-ogg="...-lq.ogg",
whose -hq.ogg is the one used).

One-shots: cut at the given time (12 ms before the onset), high-passed (2nd order Butterworth, zero
phase), a 2 ms fade-in and a cos² fade-out over the last 35 %, peak-normalised to -3 dBFS, 48 kHz mono
16-bit. Loops: cut, high-passed, the last `xf` seconds cross-faded (equal power) into the start so the
file loops without a seam, peak-normalised to -3 dBFS."""
import os
import sys
from math import gcd

import numpy as np
import soundfile as sf
from scipy.signal import butter, resample_poly, sosfiltfilt

DL, out = sys.argv[1], sys.argv[2]
os.makedirs(out, exist_ok=True)
cache = {}


def load(p):
    x, sr = sf.read(p, always_2d=True)
    return x.mean(axis=1).astype(np.float64), sr


def src(sid):
    if sid not in cache:
        x, sr = load(os.path.join(DL, f'{sid}.ogg'))
        if sr != 48000:
            g = gcd(int(sr), 48000)
            x = resample_poly(x, 48000 // g, int(sr) // g)
        cache[sid] = x
    return cache[sid]


def hp(x, f):
    return sosfiltfilt(butter(2, f, 'highpass', fs=48000, output='sos'), x)


def write(name, y):
    y = y - y.mean()
    y = y / (np.abs(y).max() + 1e-12) * 10 ** (-3 / 20)
    sf.write(os.path.join(out, name), y.astype(np.float32), 48000, subtype='PCM_16')
    print(f'{name:32} {len(y) / 48000:6.2f} s')


def shot(name, sid, t, length, hp_hz=60, fade=0.35):
    x = src(sid)
    s0 = int((t - 0.012) * 48000)
    pad = 9600
    seg = hp(x[max(0, s0 - pad): s0 + int(length * 48000) + pad], hp_hz)[pad: pad + int(length * 48000)]
    f = np.ones(len(seg))
    k = int(fade * len(seg))
    f[-k:] = np.cos(np.linspace(0, np.pi / 2, k)) ** 2
    f[:96] = np.linspace(0, 1, 96)
    write(name, seg * f)


def loop(name, sid, t0, t1, xf=2.0, hp_hz=40):
    x = hp(src(sid), hp_hz)
    seg = x[int(t0 * 48000): int(t1 * 48000)]
    n = int(xf * 48000)
    body = seg[:len(seg) - n].copy()
    a = np.linspace(0, np.pi / 2, n)
    body[:n] = seg[:n] * np.sin(a) + seg[len(seg) - n:] * np.cos(a)
    write(name, body)


# Plinks into still pools (tonal, with the cave's own tail).
PLINK = [('843936', 293.01), ('843936', 185.12), ('843936', 420.48), ('843936', 446.02), ('843936', 382.28),
         ('402929', 74.29), ('402929', 129.96), ('177958', 5.84), ('177958', 68.95), ('177958', 76.59)]
for i, (s, t) in enumerate(PLINK, 1):
    shot(f'cave_plink_{i}.wav', s, t, 1.4)
# The slow drip close by: one stalactite into one small pool (same ~860 Hz tone each time).
for i, t in enumerate([273.94, 356.74, 216.84], 1):
    shot(f'cave_slowdrip_{i}.wav', '843936', t, 1.4)
# Deep, low drips into a large pool far in.
for i, t in enumerate([7.80, 45.08, 78.98], 1):
    shot(f'cave_deepdrip_{i}.wav', '870748', t, 1.6)
# Drops on rock: dry close takes (the scene's echo puts them in the cave) and three cave takes.
ROCK = [('473802', 1.00), ('473802', 2.00), ('473802', 3.00), ('473802', 4.01), ('473802', 8.02), ('473802', 9.02),
        ('473802', 18.04), ('473802', 26.05)]
for i, (s, t) in enumerate(ROCK, 1):
    shot(f'cave_rockdrip_{i}.wav', s, t, 0.4, hp_hz=120, fade=0.5)
for i, t in enumerate([108.02, 137.94, 165.41], len(ROCK) + 1):
    shot(f'cave_rockdrip_{i}.wav', '402929', t, 1.2, hp_hz=120)
# Small stones trickling down a slope.
for i, (s, t, l) in enumerate([('179341', 0.43, 1.75), ('179341', 2.34, 1.9), ('179341', 5.53, 1.7),
                               ('179341', 7.30, 1.8), ('179341', 11.35, 1.4), ('521956', 0.0, 3.2),
                               ('133655', 0.0, 2.6)], 1):
    shot(f'cave_pebbles_{i}.wav', s, max(t, 0.012), l, hp_hz=80, fade=0.3)
# The underground stream: a low murmur (two stretches) and a brighter runnel.
loop('cave_stream_1.wav', '414167', 1.0, 21.0)
loop('cave_stream_2.wav', '414167', 24.0, 46.0)
loop('cave_runnel_1.wav', '462661', 0.5, 18.0, xf=2.5, hp_hz=60)
