"""Measures a struck or rubbed bowl (or tingsha, bell, any modal ring) from a recording.

    python3 tools/analysis/bowl_partials.py FILE [FILE ...] [--start S] [--dur D] [--fmin HZ]
                                            [--fmax HZ] [--partials N] [--rubbed] [--json]

For each file:
* the strike: the loudest onset (or `--start`), the attack time (10 -> 90 % of the first
  peak of the 1 ms envelope), the spectral centroid of the first 100 ms and the share of energy
  at 2..6 kHz in the first 0.5 s (the band that pierces);
* the partials: the N strongest spectral peaks above `--fmin` over the ring (long Hann window,
  zero-padded, parabolic interpolation), as frequencies and ratios to the lowest; close pairs
  (< 2 %) are reported as one partial with its doublet split;
* per partial, through a narrow band-pass (+-max(6 Hz, 2.5 %)): the level at the strike re the
  fundamental, the decay time T60 (straight-line fit to the dB envelope while it is > 12 dB over
  the band's floor), and the beat: the strongest modulation of the de-trended dB envelope in
  0.04..12 Hz, with its depth (p95 - p5, dB);
* the tonality: energy in the partial bands vs all (100 Hz..16 kHz) -- low = hiss or noise.

`--rubbed` skips the strike and the decay (a sustained sound): partial levels, beats, tonality.
Pure numpy/scipy/soundfile; reads anything libsndfile reads (wav, flac, ogg, mp3).
"""
import argparse
import json
import sys

import numpy as np
import soundfile as sf
from scipy import signal


def load(path):
    x, sr = sf.read(path, always_2d=True)
    return x.mean(axis=1).astype(np.float64), float(sr)


def envelope_db(x, sr, ms):
    w = max(1, int(sr * ms / 1000))
    n = len(x) // w
    e = np.sqrt(np.mean(x[: n * w].reshape(n, w) ** 2, axis=1) + 1e-20)
    return 20 * np.log10(e), w


def find_onset(x, sr):
    """Start of the loudest strike: the frame where the 5 ms envelope first comes within 6 dB of
    its maximum, walked back to where it rose out of what was before."""
    e, w = envelope_db(x, sr, 5)
    k = int(np.argmax(e))
    j = k
    while j > 0 and e[j - 1] > e[k] - 30 and e[j - 1] < e[j] + 0.5 and k - j < 40:
        j -= 1
    return j * w / sr


def peaks(x, sr, fmin, fmax, n):
    """N strongest peaks (Hz, dB) of a long Hann-windowed spectrum, parabolic-interpolated."""
    N = 1 << int(np.ceil(np.log2(len(x) * 4)))
    X = np.abs(np.fft.rfft(x * np.hanning(len(x)), N))
    db = 20 * np.log10(X + 1e-12)
    f = np.arange(len(X)) * sr / N
    lo, hi = np.searchsorted(f, fmin), np.searchsorted(f, fmax)
    seg = db[lo:hi]
    floor = np.percentile(seg, 50)
    idx, _ = signal.find_peaks(seg, prominence=10, height=floor + 15)
    idx = idx + lo
    out = []
    for i in idx:
        a, b, c = db[i - 1], db[i], db[i + 1]
        p = 0.5 * (a - c) / (a - 2 * b + c) if (a - 2 * b + c) != 0 else 0
        out.append((float((i + p) * sr / N), float(b - 0.25 * (a - c) * p)))
    out.sort(key=lambda t: -t[1])
    return out[: n * 3]


def group_partials(pk, n, rel=0.02):
    """Strongest first; a peak within `rel` of a kept one is its doublet partner."""
    parts = []
    for f, d in pk:
        for p in parts:
            if abs(f - p["f"]) / p["f"] < rel:
                if "split" not in p:
                    p["split"] = abs(f - p["f"])
                    p["partner_db"] = d - p["db"]
                break
        else:
            if len(parts) < n:
                parts.append({"f": f, "db": d})
    parts.sort(key=lambda p: p["f"])
    return parts


def band_env_db(x, sr, f, ms=20):
    bw = max(6.0, 0.025 * f)
    lo, hi = max(5.0, f - bw), min(0.49 * sr, f + bw)
    sos = signal.butter(2, [lo, hi], btype="bandpass", fs=sr, output="sos")
    y = signal.sosfiltfilt(sos, x)
    env = np.abs(signal.hilbert(y))
    w = max(1, int(sr * ms / 1000))
    n = len(env) // w
    e = env[: n * w].reshape(n, w).mean(axis=1)
    return 20 * np.log10(e + 1e-12), w / sr


def decay_and_beat(edb, dt, decay=True):
    t = np.arange(len(edb)) * dt
    floor = np.percentile(edb, 3)
    res = {}
    if decay:
        # Fit from 0.15 s after the strike while > floor + 12 dB.
        ok = (t > 0.15) & (edb > floor + 12)
        if ok.sum() > 20:
            last = np.max(np.nonzero(ok)[0])
            sel = np.zeros_like(ok)
            sel[int(0.15 / dt) : last + 1] = True
            a, b = np.polyfit(t[sel], edb[sel], 1)
            res["t60"] = float(-60 / a) if a < 0 else float("inf")
            resid = edb[sel] - (a * t[sel] + b)
        else:
            resid = edb - edb.mean()
    else:
        sel = edb > floor + 6
        a, b = np.polyfit(t[sel], edb[sel], 1) if sel.sum() > 20 else (0, edb.mean())
        resid = edb[sel] - (a * t[sel] + b)
    if len(resid) > 64:
        r = signal.detrend(resid)
        N = 1 << int(np.ceil(np.log2(len(r) * 8)))
        R = np.abs(np.fft.rfft(r * np.hanning(len(r)), N))
        fr = np.arange(len(R)) / (N * dt)
        band = (fr >= max(0.04, 1.5 / (len(r) * dt))) & (fr <= 12)
        if band.any():
            k = np.argmax(np.where(band, R, 0))
            res["beat_hz"] = float(fr[k])
            res["beat_depth_db"] = float(np.percentile(r, 95) - np.percentile(r, 5))
    return res


def band_share(x, sr, lo, hi):
    X = np.abs(np.fft.rfft(x * np.hanning(len(x)))) ** 2
    f = np.fft.rfftfreq(len(x), 1 / sr)
    tot = X[(f >= 40) & (f <= min(16000, sr / 2))].sum()
    return float(X[(f >= lo) & (f <= hi)].sum() / max(tot, 1e-30))


def centroid(x, sr):
    X = np.abs(np.fft.rfft(x * np.hanning(len(x)))) ** 2
    f = np.fft.rfftfreq(len(x), 1 / sr)
    m = (f >= 40) & (f <= 16000)
    return float((f[m] * X[m]).sum() / max(X[m].sum(), 1e-30))


def analyse(path, start=None, dur=None, fmin=60.0, fmax=12000.0, n=6, rubbed=False):
    x, sr = load(path)
    out = {"file": path.split("/")[-1], "sr": sr, "seconds": round(len(x) / sr, 2)}
    t0 = start if start is not None else (0.0 if rubbed else find_onset(x, sr))
    i0 = int(t0 * sr)
    seg = x[i0 : i0 + int((dur or len(x) / sr) * sr)]
    out["onset_s"] = round(t0, 3)
    peak = np.max(np.abs(x))
    out["peak_dbfs"] = round(20 * np.log10(peak + 1e-12), 1)
    if not rubbed:
        e1, w1 = envelope_db(seg[: int(0.3 * sr)], sr, 1)
        lin = 10 ** (e1 / 20)
        k = int(np.argmax(lin[:100])) if len(lin) else 0
        pk = lin[k] if len(lin) else 1
        a10 = np.argmax(lin >= 0.1 * pk)
        a90 = np.argmax(lin >= 0.9 * pk)
        out["attack_ms"] = round((a90 - a10) * w1 / sr * 1000, 1)
        out["centroid_0_100ms"] = round(centroid(seg[: int(0.1 * sr)], sr))
        out["share_2_6k_0_500ms"] = round(band_share(seg[: int(0.5 * sr)], sr, 2000, 6000), 4)
    out["share_2_6k_all"] = round(band_share(seg, sr, 2000, 6000), 4)
    ring = seg[int(0.05 * sr) :] if not rubbed else seg
    pk = peaks(ring, sr, fmin, min(fmax, 0.45 * sr), n)
    parts = group_partials(pk, n)
    if not parts:
        return out
    # The fundamental is the lowest partial that matters: within 40 dB of the strongest
    # (a faint low peak is hum or a neighbour, not the bowl).
    top = max(p["db"] for p in parts)
    parts = [p for p in parts if p["db"] > top - 40]
    f1 = parts[0]["f"]
    early = seg[: int(0.5 * sr)] if not rubbed else seg
    E = np.abs(np.fft.rfft(early * np.hanning(len(early)), 1 << 18))
    fe = np.fft.rfftfreq(1 << 18, 1 / sr)

    def level_at(f):
        m = (fe > f * 0.985) & (fe < f * 1.015)
        return 20 * np.log10(E[m].max() + 1e-12)

    ref = level_at(f1)
    tonal = np.zeros(len(seg), dtype=bool)
    for p in parts:
        p["ratio"] = round(p["f"] / f1, 3)
        p["level_db"] = round(level_at(p["f"]) - ref, 1)
        edb, dt = band_env_db(seg, sr, p["f"])
        p.update(decay_and_beat(edb, dt, decay=not rubbed))
        p["f"] = round(p["f"], 1)
        p["db"] = round(p["db"], 1)
        for k in ("split", "partner_db", "t60", "beat_hz", "beat_depth_db"):
            if k in p:
                p[k] = round(p[k], 2)
    # Tonality: power inside the partial bands over all power, 100 Hz..16 kHz.
    X = np.abs(np.fft.rfft(seg * np.hanning(len(seg)))) ** 2
    f = np.fft.rfftfreq(len(seg), 1 / sr)
    allm = (f >= 100) & (f <= min(16000, sr / 2))
    inb = np.zeros_like(allm)
    for p in parts:
        bw = max(6.0, 0.025 * p["f"])
        inb |= (f > p["f"] - bw) & (f < p["f"] + bw)
    out["tonal_share"] = round(float(X[allm & inb].sum() / max(X[allm].sum(), 1e-30)), 4)
    out["f1"] = round(f1, 1)
    out["partials"] = parts
    return out


def table(r):
    lines = [f"{r['file']}  ({r['seconds']} s, onset {r['onset_s']} s, peak {r['peak_dbfs']} dBFS)"]
    for k in ("attack_ms", "centroid_0_100ms", "share_2_6k_0_500ms", "share_2_6k_all", "tonal_share"):
        if k in r:
            lines.append(f"  {k}: {r[k]}")
    for p in r.get("partials", []):
        extra = f" split {p['split']} Hz ({p['partner_db']:+.0f} dB)" if "split" in p else ""
        lines.append(
            f"  {p['f']:8.1f} Hz  x{p['ratio']:<6} {p['level_db']:+6.1f} dB  T60 {p.get('t60', float('nan')):6.1f} s"
            f"  beat {p.get('beat_hz', float('nan')):5.2f} Hz / {p.get('beat_depth_db', float('nan')):4.1f} dB{extra}"
        )
    return "\n".join(lines)


if __name__ == "__main__":
    ap = argparse.ArgumentParser()
    ap.add_argument("files", nargs="+")
    ap.add_argument("--start", type=float)
    ap.add_argument("--dur", type=float)
    ap.add_argument("--fmin", type=float, default=60.0)
    ap.add_argument("--fmax", type=float, default=12000.0)
    ap.add_argument("--partials", type=int, default=6)
    ap.add_argument("--rubbed", action="store_true")
    ap.add_argument("--json", action="store_true")
    a = ap.parse_args()
    res = [analyse(f, a.start, a.dur, a.fmin, a.fmax, a.partials, a.rubbed) for f in a.files]
    if a.json:
        json.dump(res, sys.stdout, indent=1)
    else:
        print("\n\n".join(table(r) for r in res))
