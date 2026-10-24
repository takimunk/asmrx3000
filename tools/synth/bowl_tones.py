"""python3 tools/synth/bowl_tones.py assets/sounds/generated/body_scan

Clean sustained singing-bowl tones for the body-scan guide: seamless 24 s loops, no noise.

Partials of a struck/rubbed Himalayan bowl run near 1 : 2.71 : 5.15 : 8.3 (wine-glass-like
(2,0)...(5,0) modes); each mode is a doublet (the bowl is not perfectly round), which gives
the slow beating that makes a bowl "breathe". All frequencies are multiples of 1/24 Hz so a
24 s loop has no seam."""
import math, struct, sys, wave

SR, DUR = 48000, 24.0
N = int(SR * DUR)

def q(f):  # nearest multiple of 1/DUR
    return round(f * DUR) / DUR

def bowl(f0, amps, beats, breath=0.08):
    ratios = [1.0, 2.71, 5.15, 8.3]
    parts = []
    for r, a, b in zip(ratios, amps, beats):
        if a <= 0:
            continue
        f = q(f0 * r)
        parts.append((f, a * 0.5))
        parts.append((q(f0 * r + b), a * 0.5))  # doublet partner: beats at b Hz
    out = [0.0] * N
    w = 2 * math.pi / SR
    for f, a in parts:
        k = w * f
        for n in range(N):
            out[n] += a * math.sin(k * n)
    # A slow breath (one in 12 s), very shallow.
    kb = 2 * math.pi / (12.0 * SR)
    peak = 0.0
    for n in range(N):
        out[n] *= 1.0 + breath * math.sin(kb * n)
        peak = max(peak, abs(out[n]))
    g = 10 ** (-3 / 20) / peak
    return [v * g for v in out]

def save(path, x):
    with wave.open(path, "wb") as w:
        w.setnchannels(1); w.setsampwidth(2); w.setframerate(SR)
        w.writeframes(b"".join(struct.pack("<h", int(max(-1, min(1, v)) * 32767)) for v in x))

if __name__ == "__main__":
    out = sys.argv[1]
    voices = {
        # name: (f0, partial amplitudes, doublet beat Hz per partial)
        "guide_brush_el2_1": (220.0, [1.0, 0.42, 0.16, 0.05], [0.75, 1.25, 1.75, 2.25]),
        "guide_brush_el2_2": (220.0, [1.0, 0.38, 0.18, 0.04], [0.5, 1.0, 1.5, 2.0]),
        "guide_air_el2_1":   (330.0, [1.0, 0.22, 0.06, 0.0],  [0.5, 1.0, 1.5, 0.0]),
        "guide_grain_el2_1": (165.0, [1.0, 0.45, 0.2, 0.06],  [0.75, 1.0, 1.5, 2.0]),
        "guide_grain_el2_2": (165.0, [1.0, 0.4, 0.22, 0.05],  [0.5, 1.25, 1.75, 2.25]),
        "guide_warm_el2_1":  (110.0, [1.0, 0.3, 0.1, 0.0],    [0.25, 0.75, 1.25, 0.0]),
    }
    for name, (f0, a, b) in voices.items():
        save(f"{out}/{name}.wav", bowl(f0, a, b))
        print("wrote", name)
