#!/usr/bin/env python3
"""Measures underwater recordings (or our renders) for the procedural underwater models.

    python tools/analysis/underwater_measure.py KIND file.wav [file.wav ...] [--json out.json]

KIND selects what is measured on top of the common spectrum / level statistics:

* ``bed``      spectrum, band shares, level statistics only.
* ``whale``    tonal units 60–2000 Hz: dominant and f0 contour per unit, unit durations,
               gaps, contour smoothness (residual wobble in cents) and slope.
* ``dolphin``  tonal units 2–20 kHz (whistles) plus clicks above 8 kHz.
* ``reef``     impulses above 2 kHz (snapping shrimp): rate, inter-click intervals,
               amplitude spread; low pulses 50–500 Hz (fish).
* ``comfort``  for full-scene renders: sample-to-sample jumps vs local RMS, short-term
               loudness jumps over the running average, peak level, narrow tones > 3 kHz,
               2–6 kHz band share.

Only numpy / scipy / soundfile. Inputs are read-only; nothing is written next to them.
"""

from __future__ import annotations

import argparse
import json
import sys

import numpy as np
import soundfile as sf
from scipy import signal

BANDS = [(20, 200), (200, 1000), (1000, 3000), (3000, 6000), (6000, 12000), (12000, 24000)]


def load(path: str) -> tuple[np.ndarray, int]:
    x, sr = sf.read(path, always_2d=True)
    x = x.mean(axis=1).astype(np.float64)
    return x - x.mean(), sr


def db(v):
    return 10 * np.log10(np.maximum(v, 1e-20))


def spectrum(x, sr):
    f, p = signal.welch(x, sr, nperseg=8192, noverlap=4096)
    tot = p.sum()
    cen = float((f * p).sum() / tot)
    cum = np.cumsum(p) / tot
    bw99 = float(f[np.searchsorted(cum, 0.99)])
    med = float(f[np.searchsorted(cum, 0.5)])
    shares = {f"{lo}-{hi}": round(float(p[(f >= lo) & (f < hi)].sum() / tot), 4) for lo, hi in BANDS}
    # Effective bandwidth of the recording: highest frequency within 50 dB of the PSD peak
    # (after 1/3-octave smoothing) — NOAA clips are often band-limited.
    ps = signal.savgol_filter(db(p), 31, 2)
    top = ps.max()
    above = np.where(ps > top - 50)[0]
    eff = float(f[above[-1]]) if len(above) else 0.0
    return {"centroid_hz": round(cen), "median_hz": round(med), "bw99_hz": round(bw99), "effective_bw_hz": round(eff), "shares": shares}, (f, p)


def levels(x, sr):
    w = int(0.05 * sr)
    n = len(x) // w
    r = np.sqrt((x[: n * w].reshape(n, w) ** 2).mean(axis=1))
    d = db(r**2)
    d = d[d > d.max() - 80]
    peak = float(20 * np.log10(np.abs(x).max() + 1e-12))
    rms = float(20 * np.log10(np.sqrt((x**2).mean()) + 1e-12))
    return {
        "rms_dbfs": round(rms, 1),
        "peak_dbfs": round(peak, 1),
        "crest_db": round(peak - rms, 1),
        "frame50ms_p10_p50_p90": [round(float(np.percentile(d, q)), 1) for q in (10, 50, 90)],
        "spread_p90_p10_db": round(float(np.percentile(d, 90) - np.percentile(d, 10)), 1),
    }


def stft(x, sr, nper, hop):
    f, t, z = signal.stft(x, sr, nperseg=nper, noverlap=nper - hop, boundary=None, padded=False)
    return f, t, np.abs(z) ** 2


def reversals(cents, hyst=20.0, jump=120.0):
    """Turning points of a pitch track with `hyst` cents hysteresis; octave/harmonic jumps of
    the tracker (> `jump` cents between frames) restart the count instead of adding to it."""
    n, d = 0, 0
    hi = lo = prev = None
    for c in cents:
        if prev is not None and abs(c - prev) > jump:
            d, hi, lo = 0, c, c
        prev = c
        if hi is None:
            hi = lo = c
            continue
        if d >= 0:
            hi = max(hi, c)
            if hi - c > hyst:
                n += d == 1
                d, lo = -1, c
                continue
        if d <= 0:
            lo = min(lo, c)
            if c - lo > hyst:
                n += d == -1
                d, hi = 1, c
    return int(n)


def tonal_units(x, sr, fmin, fmax, nper=4096, hop=512, ton_db=14.0, harmonic_f0=True):
    """Tracks the dominant tonal component; groups voiced frames into units."""
    f, t, P = stft(x, sr, nper, hop)
    band = (f >= fmin) & (f <= fmax)
    fb = f[band]
    Pb = P[band]
    pk = Pb.argmax(axis=0)
    peak = Pb[pk, np.arange(Pb.shape[1])]
    med = np.median(Pb, axis=0) + 1e-20
    tonality = db(peak / med)
    e = Pb.sum(axis=0)
    floor = np.percentile(e, 30)
    voiced = (tonality > ton_db) & (e > 3 * floor)
    # Parabolic interpolation of the peak bin.
    fr = np.empty(len(pk))
    df = f[1] - f[0]
    for i, k in enumerate(pk):
        if 0 < k < len(fb) - 1:
            a, b, c = db(Pb[k - 1, i]), db(Pb[k, i]), db(Pb[k + 1, i])
            den = a - 2 * b + c
            off = 0.5 * (a - c) / den if den != 0 else 0.0
            fr[i] = fb[k] + off * df
        else:
            fr[i] = fb[k]
    f0 = fr.copy()
    if harmonic_f0:
        # Sub-harmonic check: if a component at peak/2 or peak/3 is strong, prefer it.
        for i in np.where(voiced)[0]:
            for h in (3, 2):
                cand = fr[i] / h
                if cand < fmin:
                    continue
                k = np.argmin(np.abs(fb - cand))
                lo, hi = max(k - 2, 0), min(k + 3, len(fb))
                if Pb[lo:hi, i].max() > 0.1 * peak[i] and db(Pb[lo:hi, i].max() / med[i]) > ton_db:
                    f0[i] = fb[lo + Pb[lo:hi, i].argmax()]
                    break
    # Units: runs of voiced frames, gaps < 80 ms bridged, shorter than 120 ms dropped.
    dt = hop / sr
    v = voiced.copy()
    gap = int(0.08 / dt)
    i = 0
    n = len(v)
    while i < n:
        if not v[i]:
            j = i
            while j < n and not v[j]:
                j += 1
            if 0 < i and j < n and j - i <= gap:
                v[i:j] = True
            i = j
        else:
            i += 1
    units = []
    i = 0
    while i < n:
        if v[i]:
            j = i
            while j < n and v[j]:
                j += 1
            if (j - i) * dt >= 0.12:
                units.append((i, j))
            i = j
        else:
            i += 1
    out = []
    for a, b in units:
        seg = fr[a:b][voiced[a:b]]
        seg0 = f0[a:b][voiced[a:b]]
        if len(seg) < 4:
            continue
        cents = 1200 * np.log2(seg / np.median(seg))
        # Wobble: residual after a 150 ms moving average, cents RMS (contour smoothness),
        # computed on stretches where the tracked peak moves < 120 cents per frame (a jump
        # between harmonics or to an echo is the tracker's, not the voice's).
        k = max(3, int(0.15 / dt) | 1)
        cuts = np.where(np.abs(np.diff(cents)) > 120)[0] + 1
        res_all = []
        for part in np.split(cents, cuts):
            if len(part) > k:
                sm = np.convolve(part, np.ones(k) / k, mode="same")
                res_all.append((part - sm)[k // 2 : -(k // 2)])
        res = np.concatenate(res_all) if res_all else np.zeros(0)
        wob = float(np.sqrt((res**2).mean())) if len(res) else 0.0
        rev = reversals(cents)
        out.append(
            {
                "reversals": rev,
                "t0": round(float(t[a]), 2),
                "dur": round(float((b - a) * dt), 2),
                "dom_hz": [round(float(seg.min())), round(float(np.median(seg))), round(float(seg.max()))],
                "f0_med_hz": round(float(np.median(seg0))),
                "start_end_hz": [round(float(seg[0])), round(float(seg[-1]))],
                "range_oct": round(float(np.log2(seg.max() / seg.min())), 2),
                "wobble_cents": round(wob, 1),
            }
        )
    gaps = [round(out[i + 1]["t0"] - out[i]["t0"] - out[i]["dur"], 2) for i in range(len(out) - 1)]
    durs = [u["dur"] for u in out]
    summ = {
        "units": len(out),
        "voiced_share": round(float(voiced.mean()), 3),
        "dur_p10_p50_p90": [round(float(np.percentile(durs, q)), 2) for q in (10, 50, 90)] if durs else [],
        "gap_p10_p50_p90": [round(float(np.percentile(gaps, q)), 2) for q in (10, 50, 90)] if gaps else [],
        "f0_med_p10_p50_p90": [round(float(np.percentile([u["f0_med_hz"] for u in out], q))) for q in (10, 50, 90)] if out else [],
        "dom_med_p10_p50_p90": [round(float(np.percentile([u["dom_hz"][1] for u in out], q))) for q in (10, 50, 90)] if out else [],
        "range_oct_p50_p90": [round(float(np.percentile([u["range_oct"] for u in out], q)), 2) for q in (50, 90)] if out else [],
        "wobble_cents_p50_p90": [round(float(np.percentile([u["wobble_cents"] for u in out], q)), 1) for q in (50, 90)] if out else [],
        # Pitch direction changes per second of voiced unit time (vibrato / wobble shows up here).
        "reversals_per_s": round(sum(u["reversals"] for u in out) / max(1e-6, sum(u["dur"] for u in out)), 2) if out else 0.0,
    }
    return summ, out


def clicks(x, sr, f_lo, thresh=6.0, min_sep_ms=0.6):
    """Impulse detection on the high-passed envelope; returns rate and statistics."""
    sos = signal.butter(4, f_lo, "highpass", fs=sr, output="sos")
    y = signal.sosfiltfilt(sos, x)
    env = np.abs(signal.hilbert(y))
    k = max(1, int(0.0002 * sr))
    env = np.convolve(env, np.ones(k) / k, mode="same")
    # Local background: running median over 50 ms (computed on a decimated envelope).
    dec = max(1, int(0.001 * sr))
    e_d = env[: len(env) // dec * dec].reshape(-1, dec).max(axis=1)
    bg = signal.medfilt(e_d, 51)
    bg_full = np.repeat(bg, dec)
    bg_full = np.pad(bg_full, (0, len(env) - len(bg_full)), mode="edge")
    peaks, props = signal.find_peaks(env, height=thresh * bg_full + 1e-9, distance=int(min_sep_ms * 1e-3 * sr))
    dur = len(x) / sr
    if len(peaks) < 2:
        return {"clicks": int(len(peaks)), "rate_per_s": round(len(peaks) / dur, 2)}
    ici = np.diff(peaks) / sr * 1000
    amps = db(env[peaks] ** 2)
    # Spectral centroid of each click (1 ms window, the broadband signal).
    w = int(0.001 * sr)
    cents = []
    for p in peaks[:: max(1, len(peaks) // 300)]:
        seg = x[max(0, p - w // 2) : p + w // 2]
        if len(seg) < w:
            continue
        s = np.abs(np.fft.rfft(seg * np.hanning(len(seg)))) ** 2
        fr = np.fft.rfftfreq(len(seg), 1 / sr)
        cents.append((fr * s).sum() / s.sum())
    return {
        "clicks": int(len(peaks)),
        "rate_per_s": round(len(peaks) / dur, 1),
        "ici_ms_p10_p50_p90": [round(float(np.percentile(ici, q)), 1) for q in (10, 50, 90)],
        "amp_spread_p90_p10_db": round(float(np.percentile(amps, 90) - np.percentile(amps, 10)), 1),
        "click_centroid_hz_p50": round(float(np.median(cents))) if cents else 0,
    }


def low_pulses(x, sr):
    """Fish grunts / knocks: energy bursts 50–500 Hz above the running median."""
    sos = signal.butter(4, [50, 500], "bandpass", fs=sr, output="sos")
    y = signal.sosfiltfilt(sos, x)
    w = int(0.02 * sr)
    n = len(y) // w
    e = db((y[: n * w].reshape(n, w) ** 2).mean(axis=1))
    bg = signal.medfilt(e, 151)
    ev = e > bg + 8
    starts = np.where(np.diff(ev.astype(int)) == 1)[0]
    return {"low_events": int(len(starts)), "per_min": round(len(starts) / (len(x) / sr) * 60, 1)}


def comfort(x, sr):
    out = {}
    # Discontinuities: sample jump relative to the local RMS (5 ms) of the first difference.
    d = np.abs(np.diff(x))
    w = int(0.005 * sr)
    loc = np.sqrt(np.convolve(d**2, np.ones(w) / w, mode="same")) + 1e-9
    ratio = d / loc
    # Ignore near-silence (16-bit quantisation steps against a vanishing local level).
    ratio[loc < 10 ** (-80 / 20)] = 0.0
    out["max_jump_ratio"] = round(float(ratio.max()), 1)
    out["jumps_over_6x"] = int((ratio > 6).sum())
    out["peak_dbfs"] = round(float(20 * np.log10(np.abs(x).max() + 1e-12)), 2)
    # Short-term loudness (3 s, K-weighting approximated by a 2nd-order high-shelf + HPF).
    b, a = signal.iirfilter(2, 1500, btype="highpass", fs=sr, ftype="butter")
    hs = x + 0.58 * signal.lfilter(b, a, x)  # ~ +4 dB above 1.5 kHz
    sos = signal.butter(2, 60, "highpass", fs=sr, output="sos")
    k = signal.sosfilt(sos, hs)
    w3 = int(3 * sr)
    hop = int(0.1 * sr)
    st = np.array([np.mean(k[i : i + w3] ** 2) for i in range(0, len(k) - w3, hop)])
    st_db = db(st)
    avg = db(np.mean(k**2))
    out["short_term_max_over_avg_db"] = round(float(st_db.max() - avg), 1)
    out["short_term_p95_over_avg_db"] = round(float(np.percentile(st_db, 95) - avg), 1)
    # Momentary (400 ms) jumps over the preceding 10 s average.
    w4 = int(0.4 * sr)
    m = np.array([np.mean(k[i : i + w4] ** 2) for i in range(0, len(k) - w4, hop)])
    m_db = db(m)
    ctx = int(10 / 0.1)
    if len(m_db) > ctx:
        run = np.array([db(np.mean(m[max(0, i - ctx) : i])) for i in range(ctx, len(m))])
        out["momentary_max_over_prev10s_db"] = round(float((m_db[ctx:] - run).max()), 1)
    spec, (f, p) = spectrum(x, sr)
    out["spectrum"] = spec
    # Narrow tones above 3 kHz: max prominence of a PSD bin over the 1/3-octave median.
    fs, ts, P = stft(x, sr, 8192, 4096)
    hi = fs > 3000
    prom = []
    where = []
    fe = P.sum(axis=0)
    loud = fe > np.median(fe) * 10 ** (-30 / 10)
    fh = fs[hi]
    for j in np.where(loud)[0]:
        col = P[hi, j]
        med = signal.medfilt(col, 101) + 1e-20
        pr = db(col / med)
        k = int(pr.argmax())
        prom.append(pr[k])
        where.append(fh[k])
    prom = np.array(prom)
    out["tone_over_3k_prominence_db_p50_p99"] = [round(float(np.median(prom)), 1), round(float(np.percentile(prom, 99)), 1)]
    out["tone_over_3k_at_hz_p99"] = round(float(np.array(where)[prom >= np.percentile(prom, 99)].mean()))
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("kind", choices=["bed", "whale", "dolphin", "reef", "comfort"])
    ap.add_argument("files", nargs="+")
    ap.add_argument("--json")
    ap.add_argument("--units", action="store_true", help="print every tonal unit")
    a = ap.parse_args()
    res = {}
    for path in a.files:
        x, sr = load(path)
        r = {"seconds": round(len(x) / sr, 1)}
        r["spectrum"], _ = spectrum(x, sr)
        r["levels"] = levels(x, sr)
        if a.kind == "whale":
            r["tonal"], units = tonal_units(x, sr, 60, 2000)
            if a.units:
                r["units"] = units
        elif a.kind == "dolphin":
            r["tonal"], units = tonal_units(x, sr, 2500, min(20000, sr / 2 - 500), nper=1024, hop=128, ton_db=12.0, harmonic_f0=False)
            r["clicks"] = clicks(x, sr, 8000, thresh=8.0)
            if a.units:
                r["units"] = units
        elif a.kind == "reef":
            r["clicks"] = clicks(x, sr, 2000, thresh=4.0)
            r["low"] = low_pulses(x, sr)
        elif a.kind == "comfort":
            r["comfort"] = comfort(x, sr)
        res[path.split("/")[-1]] = r
        print(path.split("/")[-1])
        print(json.dumps(r, indent=1))
        sys.stdout.flush()
    if a.json:
        with open(a.json, "w") as fh:
            json.dump(res, fh, indent=1)


if __name__ == "__main__":
    main()
