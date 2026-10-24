"""Cuts, converts and post-processes the mountain_tent sounds (docs/scenes/mountain_tent-sources.md).

  uv run --with numpy --with scipy --with soundfile python tools/synth/mountain_tent_sounds.py SRC_DIR GEN_DIR OUT_DIR

SRC_DIR holds the Freesound HQ previews named <id>.ogg; GEN_DIR the raw el2 takes from tools/fal/gen.py
(tent_pole_creak_el2_N.wav); OUT_DIR is assets/sounds/generated/mountain_tent.
Mono (channel mean), 48 kHz, 16-bit; high-pass (25 Hz, 2nd order; 40 Hz for the fabric and Foley);
event cuts get 10 ms in / 150-400 ms out fades; loops get an equal-power 2 s cross-fade of the tail
into the head (seamless at the wrap); peak normalised to -3 dBFS (gain capped at 8x, +18 dB)."""
import sys, os, numpy as np, soundfile as sf
from scipy.signal import butter, sosfiltfilt, resample_poly
from math import gcd
SRC, GEN, OUT = sys.argv[1:4]
os.makedirs(OUT, exist_ok=True)
SR = 48000

def load(path, a=None, b=None):
    x, sr = sf.read(path, always_2d=True)
    x = x.mean(1)
    if a is not None:
        x = x[int(a * sr):int(b * sr) if b else None]
    if sr != SR:
        g = gcd(SR, sr); x = resample_poly(x, SR // g, sr // g)
    return x

def hp(x, hz): return sosfiltfilt(butter(2, hz, 'hp', fs=SR, output='sos'), x)

def fades(x, fin=0.01, fout=0.15):
    n1, n2 = int(fin * SR), int(fout * SR)
    x = x.copy(); x[:n1] *= np.linspace(0, 1, n1); x[-n2:] *= np.cos(np.linspace(0, np.pi / 2, n2))**2
    return x

def loop(x, xf=2.0):
    n = int(xf * SR)
    head, tail = x[:n], x[-n:]
    t = np.linspace(0, np.pi / 2, n)
    body = x[:-n].copy()
    body[:n] = head * np.sin(t) + tail * np.cos(t)   # wrap: the tail fades out under the head
    return body

def norm(x):
    p = np.abs(x).max(); g = min(10 ** (-3 / 20) / p, 8.0)
    return x * g, 20 * np.log10(g)

def write(name, x):
    y, g = norm(x)
    sf.write(os.path.join(OUT, name), y, SR, subtype='PCM_16')
    r = 20 * np.log10(np.sqrt((y**2).mean()))
    print(f"{name:32s} {len(y)/SR:6.2f}s gain {g:+5.1f} dB rms {r:6.1f} dBFS")

fs = lambda i: os.path.join(SRC, f"{i}.ogg")
# Rain on the fly (loops).
for name, (i, a, b) in {
    "tent_fly_light_1.wav": (397916, 0, None), "tent_fly_light_2.wav": (816110, 1.5, None),
    "tent_fly_steady_1.wav": (592997, 160, 222), "tent_fly_steady_2.wav": (592997, 470, 532),
    "tent_fly_heavy_1.wav": (484724, 0, None), "tent_fly_heavy_2.wav": (427879, 34, None),
}.items():
    write(name, loop(hp(load(fs(i), a, b), 40)))
# Drops off the fly's hem onto the grass and ferns (loops).
for name, (i, a, b) in {"tent_drips_1.wav": (865323, 10, 72), "tent_drips_2.wav": (865323, 92, 154)}.items():
    write(name, loop(hp(load(fs(i), a, b), 60)))
# Creek (loops).
for name, (i, a, b) in {"tent_creek_1.wav": (423623, 8, 70), "tent_creek_2.wav": (869104, 245, 307)}.items():
    write(name, loop(hp(load(fs(i), a, b), 25)))
# The fly flapping in gusts.
for name, (i, a, b) in {"tent_flap_1.wav": (569539, 28.6, 33.0), "tent_flap_2.wav": (569539, 38.9, 44.2),
                        "tent_flap_3.wav": (569539, 15.8, 20.6), "tent_flap_4.wav": (569538, 25.2, 27.6)}.items():
    write(name, fades(hp(load(fs(i), a, b), 40), 0.25, 0.4))
# Thunder rolls (with their own rain under them, faded).
for name, (i, a, b) in {"tent_thunder_roll_1.wav": (577315, 0.5, 31.0), "tent_thunder_roll_2.wav": (717890, 126.5, 151.0),
                        "tent_thunder_roll_3.wav": (572443, 0.8, 25.0)}.items():
    write(name, fades(hp(load(fs(i), a, b), 25), 0.5, 3.0))
# Sleeping bag and zipper.
for name, (i, a, b) in {"tent_bag_rustle_1.wav": (776275, 3.45, 5.0), "tent_bag_rustle_2.wav": (776275, 8.75, 10.4),
                        "tent_bag_rustle_3.wav": (658410, 0.0, 2.6), "tent_bag_rustle_4.wav": (658410, 5.35, 8.0),
                        "tent_zipper_1.wav": (438985, 3.4, 6.9), "tent_zipper_2.wav": (438985, 12.0, 14.7),
                        "tent_zipper_3.wav": (569540, 0.0, None)}.items():
    write(name, fades(hp(load(fs(i), a, b), 40), 0.01, 0.15))
# Generated pole creaks: the main 1.4 s around each take's loudest moment (the reference frame
# creaks last 0.4-1.5 s).
for k in range(1, 5):
    x = hp(load(os.path.join(GEN, f"tent_pole_creak_el2_{k}.wav")), 40)
    e = np.convolve(x**2, np.ones(2400) / 2400, 'same'); c = int(np.argmax(e))
    a = max(0, c - int(0.45 * SR)); b = min(len(x), a + int(1.4 * SR))
    write(f"tent_pole_creak_el2_{k}.wav", fades(x[a:b], 0.02, 0.3))
