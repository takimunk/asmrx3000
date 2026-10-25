"""Comfort checks on a render (stereo or mono WAV): clicks, pitch wobble of tonal partials,
loudness jumps, peaks and the piercing 2..6 kHz band.

    python3 tools/analysis/render_check.py RENDER.wav [--tones 110,165,220,330] [--from S] [--to S]

* peak dBFS (sample peak) and integrated loudness (BS.1770 K-weighting, gated);
* loudness: the loudest momentary (400 ms) and short-term (3 s) level above the integrated
  level, and where;
* clicks: the peak above 7 kHz (5 ms) against the signal's 50 ms RMS: a mix of low partials
  stays far below -40 dB, a step or a dropout reaches -20 dB and more;
* pitch: for each tone, a +-3 % band-pass, instantaneous frequency from the analytic phase,
  smoothed over 50 ms, while the band is within 25 dB of its loudest: the spread (p1..p99) in
  cents and the slowest drift (1 s means) -- Doppler, near-field phase or vibrato show here;
* 2..6 kHz: the band's share of the energy per 3 s window (max) and over the whole take.
"""
import argparse

import numpy as np
import soundfile as sf
from scipy import signal


def k_weight(x, sr):
    # BS.1770 pre-filter (high shelf) and RLB high-pass, designed for any rate.
    f0, g, q = 1681.974450955533, 3.999843853973347, 0.7071752369554196
    k = np.tan(np.pi * f0 / sr)
    vh, vb = 10 ** (g / 20), 10 ** (g / 20) ** 0.4996667741545416
    a0 = 1 + k / q + k * k
    b = [(vh + vb * k / q + k * k) / a0, 2 * (k * k - vh) / a0, (vh - vb * k / q + k * k) / a0]
    a = [1, 2 * (k * k - 1) / a0, (1 - k / q + k * k) / a0]
    y = signal.lfilter(b, a, x, axis=0)
    f0, q = 38.13547087602444, 0.5003270373238773
    k = np.tan(np.pi * f0 / sr)
    a0 = 1 + k / q + k * k
    a = [1, 2 * (k * k - 1) / a0, (1 - k / q + k * k) / a0]
    return signal.lfilter([1, -2, 1], a, y, axis=0)


def loudness_series(x, sr, win, hop=0.1):
    y = k_weight(x, sr)
    p = (y**2).sum(axis=1) if y.ndim > 1 else y**2
    w, h = int(win * sr), int(hop * sr)
    c = np.concatenate([[0], np.cumsum(p)])
    starts = np.arange(0, len(p) - w, h)
    ms = (c[starts + w] - c[starts]) / w
    return starts / sr, -0.691 + 10 * np.log10(ms + 1e-20)


def integrated(x, sr):
    t, l = loudness_series(x, sr, 0.4, 0.1)
    l = l[l > -70]
    rel = 10 * np.log10(np.mean(10 ** (l / 10))) - 10
    l = l[l > rel]
    return 10 * np.log10(np.mean(10 ** (l / 10)))


def clicks(m, sr):
    """A click or a step is broadband: its energy above 7 kHz (where these scenes have none of
    their own) stands out against the signal around it. Returns the worst 5 ms peak above
    7 kHz relative to the 50 ms RMS of the whole signal there, dB, and where (only where the
    signal is above -60 dBFS: below that the 16-bit quantisation noise of a render reads as -30 dB)."""
    sos = signal.butter(8, 7000, btype="highpass", fs=sr, output="sos")
    h = np.abs(signal.sosfilt(sos, m))
    w, c = int(0.05 * sr), int(0.005 * sr)
    rms = np.sqrt(np.convolve(m**2, np.ones(w) / w, mode="same"))
    n = len(m) // c
    hp = h[: n * c].reshape(n, c).max(axis=1)
    rr = rms[: n * c].reshape(n, c).mean(axis=1)
    ok = rr > 10 ** (-60 / 20)
    r = np.where(ok, 20 * np.log10(hp / np.maximum(rr, 1e-12) + 1e-12), -200)
    i = int(np.argmax(r))
    return float(r[i]), i * c / sr


def pitch(m, sr, f):
    sos = signal.butter(2, [f * 0.97, f * 1.03], btype="bandpass", fs=sr, output="sos")
    y = signal.sosfiltfilt(sos, m)
    z = signal.hilbert(y)
    ph = np.unwrap(np.angle(z))
    inst = np.diff(ph) * sr / (2 * np.pi)
    env = np.abs(z)[1:]
    w = int(0.05 * sr)
    k = np.ones(w) / w
    # Amplitude-weighted mean frequency over 50 ms (robust where partners beat).
    fi = np.convolve(inst * env**2, k, "same") / np.maximum(np.convolve(env**2, k, "same"), 1e-30)
    lvl = 20 * np.log10(np.convolve(env, k, "same") + 1e-20)
    on = lvl > lvl.max() - 25
    if on.sum() < sr:
        return None
    cents = 1200 * np.log2(np.maximum(fi[on], 1) / f)
    sec = int(sr)
    means = [np.mean(1200 * np.log2(np.maximum(fi[s : s + sec][on[s : s + sec]], 1) / f)) for s in range(0, len(fi) - sec, sec) if on[s : s + sec].mean() > 0.9]
    return {
        "f": f,
        "seconds": round(on.sum() / sr, 1),
        "spread_c": round(float(np.percentile(cents, 99) - np.percentile(cents, 1)), 2),
        "p999_c": round(float(np.percentile(np.abs(cents - np.median(cents)), 99.9)), 2),
        "drift_c": round(float(max(means) - min(means)), 2) if means else None,
        "median_c": round(float(np.median(cents)), 2),
    }


def band(m, sr, lo=2000, hi=6000, win=3.0):
    w = int(win * sr)
    shares = []
    tot_b = tot = 0.0
    for s in range(0, len(m) - w, w):
        X = np.abs(np.fft.rfft(m[s : s + w] * np.hanning(w))) ** 2
        f = np.fft.rfftfreq(w, 1 / sr)
        a = X[(f >= 40) & (f <= 16000)].sum()
        b = X[(f >= lo) & (f <= hi)].sum()
        tot += a
        tot_b += b
        if a > 0:
            shares.append((b / a, s / sr))
    mx = max(shares) if shares else (0, 0)
    return mx, tot_b / max(tot, 1e-30)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("wav")
    ap.add_argument("--tones", default="110,165,220,330")
    ap.add_argument("--from", dest="t0", type=float, default=0.0)
    ap.add_argument("--to", dest="t1", type=float)
    a = ap.parse_args()
    x, sr = sf.read(a.wav, always_2d=True)
    i0 = int(a.t0 * sr)
    i1 = int(a.t1 * sr) if a.t1 else len(x)
    x = x[i0:i1]
    m = x.mean(axis=1)
    print(f"{a.wav}: {len(x) / sr:.1f} s from {a.t0:.0f} s")
    print(f"  peak {20 * np.log10(np.abs(x).max() + 1e-20):.1f} dBFS")
    li = integrated(x, sr)
    print(f"  integrated {li:.1f} LUFS")
    for name, win in (("momentary", 0.4), ("short-term", 3.0)):
        t, l = loudness_series(x, sr, win)
        k = int(np.argmax(l))
        print(f"  max {name} {l[k] - li:+.1f} LU at {t[k] + a.t0:.1f} s")
    r, at = clicks(m, sr)
    print(f"  clicks: worst >7 kHz peak {r:+.1f} dB re local RMS at {at + a.t0:.2f} s (clean tones < -40)")
    for f in [float(v) for v in a.tones.split(",") if v]:
        p = pitch(m, sr, f)
        if p:
            print(f"  pitch {f:.0f} Hz over {p['seconds']} s: median {p['median_c']:+.2f} c, p1..p99 spread {p['spread_c']} c, p99.9 dev {p['p999_c']} c, drift of 1 s means {p['drift_c']} c")
    (mx, at), whole = band(m, sr)
    print(f"  2-6 kHz share: whole {whole:.4f}, max 3 s window {mx:.4f} at {at + a.t0:.0f} s")


if __name__ == "__main__":
    main()
