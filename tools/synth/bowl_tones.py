"""python3 tools/synth/bowl_tones.py assets/sounds/generated/body_scan

Soft sustained tones for the body-scan guide: seamless 24 s loops, no noise.

Partials of a struck/rubbed Himalayan bowl run near 1 : 2.71 : 5.15 : 8.3 (wine-glass-like
(2,0)...(5,0) modes); each mode is a doublet (the bowl is not perfectly round), which gives
the slow beating that makes a bowl "breathe". All frequencies are multiples of 1/24 Hz so a
24 s loop has no seam."""
import math, struct, sys, wave

SR, DUR = 48000, 24.0
N = int(SR * DUR)

def q(f):  # nearest multiple of 1/DUR
    return round(f * DUR) / DUR

def bowl(f0, amps, beats, breath=0.08, ratios=(1.0, 2.71, 5.15, 8.3), breath_s=12.0):
    parts = []
    for r, a, b in zip(ratios, amps, beats):
        if a <= 0:
            continue
        f = q(f0 * r)
        # Doublet partner at 1/5 of the main mode: the beat is a gentle +-1.6 dB sway. (Equal
        # partners cancel completely once per beat: the tone vanished every 12 s.)
        parts.append((f, a * 0.83))
        parts.append((q(f0 * r + b), a * 0.17))
    out = [0.0] * N
    w = 2 * math.pi / SR
    for f, a in parts:
        k = w * f
        for n in range(N):
            out[n] += a * math.sin(k * n)
    # A slow, shallow breath.
    kb = 2 * math.pi / (breath_s * SR)
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
    # Soft: a near-sine with a faint octave, beats too slow to hear as pulsing (0.083 Hz, once in
    # 12 s) and a 12 s breath. The first version (bowl partials 2.71/5.15 beating at 0.75..2.25 Hz)
    # was too insistent for a meditation.
    soft = (1.0, 2.0)
    voices = {
        # name: (f0, partial amplitudes, doublet beat Hz per partial)
        "guide_brush_el2_1": (220.0, [1.0, 0.08], [0.083, 0.083]),
        "guide_brush_el2_2": (220.0, [1.0, 0.06], [0.083, 0.125]),
        "guide_air_el2_1":   (330.0, [1.0, 0.04], [0.083, 0.083]),
        "guide_grain_el2_1": (165.0, [1.0, 0.1],  [0.083, 0.125]),
        "guide_grain_el2_2": (165.0, [1.0, 0.08], [0.125, 0.083]),
        "guide_warm_el2_1":  (110.0, [1.0, 0.12], [0.083, 0.083]),
    }
    for name, (f0, a, b) in voices.items():
        save(f"{out}/{name}.wav", bowl(f0, a, b, breath=0.12, ratios=soft))
        print("wrote", name)
