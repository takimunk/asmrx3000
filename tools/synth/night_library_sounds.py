#!/usr/bin/env python3
"""Builds the night library's sound files (assets/sounds/generated/night_library/) from the fal
takes and the real recordings listed in docs/scenes/night_library-sources.md.

    uv run --no-project --with numpy --with scipy --with soundfile --with av \
        python tools/synth/night_library_sounds.py RAW REF OUT

RAW  the el2 takes as tools/fal/gen.py writes them (RAW/night_library/<name>_el2_<n>.wav)
REF  the downloaded originals: src_pendulum_klankbeeld.wav, orig_rain_window_cori.ogg,
     orig_book_pages_stephan.ogg, orig_inkpen_stephan.ogg, src_pen_marks_cc0.wav
OUT  where the finished files go (48 kHz mono 16-bit)

Every file: high-passed (25 Hz at least, more where a take had a drone), events trimmed with
short fades, peak-normalised to -3 dBFS with at most 8x gain. Loops are cross-faded end into
start. The clock is cut on its own beat so that its file repeats seamlessly as an event whose
interval is the file's length (printed, and written into the scene)."""
import sys
from math import gcd
from pathlib import Path

import numpy as np
import soundfile as sf
from scipy.signal import butter, resample_poly, sosfiltfilt

SR = 48000
RAW, REF, OUT = (Path(a) for a in sys.argv[1:4])
OUT.mkdir(parents=True, exist_ok=True)


def load(path):
    path = Path(path)
    if path.suffix == ".ogg":
        import av
        c = av.open(str(path))
        s = c.streams.audio[0]
        frames = [f.to_ndarray() for f in c.decode(s)]
        x = np.concatenate(frames, axis=1).astype(np.float64)
        x = x.mean(axis=0) if x.ndim > 1 else x
        sr = s.rate
    else:
        x, sr = sf.read(str(path), always_2d=True)
        x = x.mean(axis=1)
    if sr != SR:
        g = gcd(sr, SR)
        x = resample_poly(x, SR // g, sr // g)
    return x


def hp(x, hz, order=2):
    return sosfiltfilt(butter(order, hz, "hp", fs=SR, output="sos"), x)


def lp(x, hz, order=2):
    return sosfiltfilt(butter(order, hz, "lp", fs=SR, output="sos"), x)


def fades(x, a=0.01, b=0.05):
    x = x.copy()
    na, nb = int(a * SR), int(b * SR)
    if na: x[:na] *= np.sin(np.linspace(0, np.pi / 2, na)) ** 2
    if nb: x[-nb:] *= np.cos(np.linspace(0, np.pi / 2, nb)) ** 2
    return x


def trim(x, margin_db=40, pre=0.03, post=0.25):
    """Cuts leading and trailing stretches more than `margin_db` under the peak (20 ms frames)."""
    w = int(0.02 * SR)
    r = np.array([np.sqrt(np.mean(x[i:i + w] ** 2)) for i in range(0, len(x) - w, w)])
    rd = 20 * np.log10(r + 1e-12)
    on = np.where(rd > rd.max() - margin_db)[0]
    a = max(0, on[0] * w - int(pre * SR))
    b = min(len(x), (on[-1] + 1) * w + int(post * SR))
    return x[a:b]


def norm(x, peak_db=-3.0, max_gain=8.0):
    g = min(10 ** (peak_db / 20) / (np.abs(x).max() + 1e-12), max_gain)
    return x * g


def loop_xfade(x, secs=1.5):
    """A seamless loop: the last `secs` are cross-faded (equal power) into the start."""
    n = int(secs * SR)
    t = np.linspace(0, np.pi / 2, n)
    head = x[:n] * np.sin(t) + x[-n:] * np.cos(t)
    return np.concatenate([head, x[n:-n]])


def write(name, x):
    x = np.clip(x, -1, 1)
    sf.write(str(OUT / name), x.astype(np.float32), SR, subtype="PCM_16")
    print(f"  {name:32s} {len(x) / SR:7.2f} s  peak {20 * np.log10(np.abs(x).max() + 1e-12):5.1f} dBFS")


def slow_swell(n, seed, depth_db=3.0):
    """A slow random gain (three incommensurate sines, 7..23 s): a fire flaring and settling."""
    rng = np.random.default_rng(seed)
    t = np.arange(n) / SR
    g = sum(np.sin(2 * np.pi * t / p + rng.uniform(0, 2 * np.pi)) for p in (7.3, 12.9, 23.1)) / 3
    return 10 ** (depth_db * g / 20)


def raw(name):
    return load(RAW / "night_library" / name)


# --- Fire -------------------------------------------------------------------------------------
print("fire")
for n in (1, 2):
    x = hp(raw(f"hearth_crackle_el2_{n}.wav"), 170, 4)      # a 140-160 Hz roar, the loudest octave
    x = x * slow_swell(len(x), 10 + n, 3.5)
    write(f"hearth_crackle_el2_{n}.wav", norm(loop_xfade(x)))
for n in (1, 2):
    x = hp(raw(f"hearth_embers_el2_{n}.wav"), 220, 4)       # a 140 Hz drone under the embers
    write(f"hearth_embers_el2_{n}.wav", norm(loop_xfade(x)))
for n in (1, 2, 3):
    x = hp(raw(f"log_settle_el2_{n}.wav"), 60)
    write(f"log_settle_el2_{n}.wav", norm(fades(trim(x), 0.005, 0.4)))

# --- Rain on the windows ----------------------------------------------------------------------
print("rain")
for n in (1, 2):
    x = hp(raw(f"tallwindow_rain_el2_{n}.wav"), 120)
    write(f"tallwindow_rain_el2_{n}.wav", norm(loop_xfade(x)))
x = hp(load(REF / "orig_rain_window_cori.ogg"), 40)
write("tallwindow_squall_pd_1.wav", norm(loop_xfade(x, 2.0)))

# --- The clock (klankbeeld, CC BY 4.0) --------------------------------------------------------
print("clock")
clock = hp(load(REF / "src_pendulum_klankbeeld.wav"), 30)


def clicks(x, a, b, min_gap=0.6):
    """Times of the escapement's clicks (1.5 kHz high-passed peaks) between a and b seconds."""
    h = sosfiltfilt(butter(4, 1500, "hp", fs=SR, output="sos"), x[int(a * SR):int(b * SR)])
    hop = int(0.001 * SR)
    e = np.array([np.max(np.abs(h[i:i + hop])) for i in range(0, len(h) - hop, hop)])
    ed = 20 * np.log10(e + 1e-12)
    floor = np.percentile(ed, 30)
    picked = []
    for i in np.argsort(-ed):
        if ed[i] < floor + 12: break
        if all(abs(i - j) > min_gap * 1000 for j in picked): picked.append(i)
    return sorted(a + i * 0.001 for i in picked)


ticks = clicks(clock, 0, 119)
# One beat cycle per file (tick after the long gap, then the tock 1.09 s later), all files the
# same length: the mean cycle. Played as events whose interval is that length, a random cycle
# follows the last one and the beat stays even. (One long file with a 98 s interval drifted:
# the engine counts the interval down in f32, 87 ms early per 98 s at 64-sample blocks.)
cycles = [ticks[i + 2] - ticks[i] for i in range(2, 40, 2)]
period = float(np.mean(cycles))
n = int(round(period * SR))
beats = [fades(clock[int(round((ticks[i] - 0.3) * SR)):][:n], 0.004, 0.004) for i in range(2, 26, 2)]
g = norm(np.concatenate(beats)).max() / np.concatenate(beats).max()   # one gain for all cycles
for j, x in enumerate(beats, 1):
    write(f"clock_tick_kb_{j}.wav", x * g)
print(f"    beat cycle {period:.4f} s (= the events interval), {n} samples; cycles {min(cycles):.3f}..{max(cycles):.3f}")

# The strikes: the last of the ten (from 145.1 s) and the last two (from 142.5 s), each with its
# long hum. The escapement keeps clicking under the hum and would double the tick loop's beat,
# so every click after the strike (280 ms with its wooden after-knock) is patched with the hum
# from 320 ms earlier (10 ms fades).
# Found in 4 s windows: the hum is 30 dB louder at the start than at the end, so one threshold
# for the whole tail misses the later clicks.
strike_clicks = sorted({round(t, 3) for w in np.arange(142.0, 168.0, 4.0) for t in clicks(clock, w, w + 4.0, 0.5)
                        if all(abs(t - s) > 0.25 for s in (142.56, 145.14))})


def patched(x0, t0):
    x = x0.copy()
    f = int(0.01 * SR)
    for t in strike_clicks:
        if t < t0 + 0.3: continue
        i0, i1 = int((t - 0.03 - t0) * SR), int((t + 0.25 - t0) * SR)
        if i1 >= len(x): continue
        src = x0[i0 - int(0.32 * SR):i1 - int(0.32 * SR)]
        w = np.ones(i1 - i0)
        w[:f] = np.sin(np.linspace(0, np.pi / 2, f)) ** 2
        w[-f:] = np.cos(np.linspace(0, np.pi / 2, f)) ** 2
        x[i0:i1] = x[i0:i1] * (1 - w) + src * w
    return x


for name, t0 in (("clock_strike_kb_1.wav", 145.05), ("clock_strike_kb_2.wav", 142.47)):
    seg = clock[int(t0 * SR):int(165.6 * SR)]
    write(name, norm(fades(patched(seg, t0), 0.02, 3.0)))

# --- Reading: pages and a pen (stephan PD, soundsnap CC0) --------------------------------------
print("reading")
pages = hp(load(REF / "orig_book_pages_stephan.ogg"), 120)
for i, (a, b) in enumerate([(6.62, 9.85), (10.68, 12.3), (17.6, 18.8), (22.82, 24.35), (31.0, 32.6), (39.34, 40.55)], 1):
    seg = pages[int(a * SR):int(b * SR)]
    write(f"page_turn_pd_{i}.wav", norm(fades(trim(seg, 45, 0.04, 0.2), 0.01, 0.15)))
pen = hp(load(REF / "orig_inkpen_stephan.ogg"), 300, 4)        # handling rumble below 300 Hz
for i, (a, b) in enumerate([(3.85, 10.6), (10.4, 17.5)], 1):
    write(f"pen_scratch_pd_{i}.wav", norm(fades(pen[int(a * SR):int(b * SR)], 0.05, 0.3)))
marks = hp(load(REF / "src_pen_marks_cc0.wav"), 150)
write("pen_scratch_cc0_1.wav", norm(fades(marks[int(0.0 * SR):int(12.2 * SR)], 0.02, 0.3)))

# --- The house -------------------------------------------------------------------------------
print("house")
for stem, k in (("floor_creak", 3), ("shelf_creak", 2), ("radiator_tick", 3), ("door_far", 2)):
    for n in range(1, k + 1):
        x = hp(raw(f"{stem}_el2_{n}.wav"), 40)
        write(f"{stem}_el2_{n}.wav", norm(fades(trim(x), 0.005, 0.3)))
