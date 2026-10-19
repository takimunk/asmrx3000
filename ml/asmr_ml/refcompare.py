"""Compare rendered sound models with real field recordings using gain-robust features.

    uv run python -m asmr_ml.refcompare refs <category>
    uv run python -m asmr_ml.refcompare compare <category> <our.wav> [...]
    uv run python -m asmr_ml.refcompare report                      # markdown sources + tables
    uv run python -m asmr_ml.refcompare features <file> [...]     # one file, raw features

References live in ml/data/refs/<category>/ (fetch: asmr_ml.data.refs_fetch / refs_xc).
Everything is relative (band levels re total, modulation depth re mean level, level
differences in dB), so the microphone gain of a recording does not matter.

Features
  band_<Hz>       1/3-octave level, dB re total power (50 Hz .. 12.5 kHz)
  centroid_*      spectral centroid (Hz): median, p10, p90 over 64 ms frames
  flatness_db     spectral flatness, median over frames, dB (0 = white noise, very negative = tonal)
  am_*            envelope modulation power (dB re mean^2) in 0.1-1, 1-4, 4-16, 16-64 Hz:
                  babbling vs steady hiss, machine-gun vs natural
  ampeak_hz       strongest envelope modulation rate 2-100 Hz (drumming / call pulse rate)
  crest_db        p99 of 50 ms RMS minus its mean (dB); spread_db: p95 - p10 of 50 ms RMS
  onset_rate      onsets per second (rises > 6 dB in 30 ms on the 10 ms envelope)
  ioi_cv          coefficient of variation of inter-onset intervals (0 = metronome)
  tonal_db/_hz    strongest narrow peak of the long-term spectrum over its 1/3-octave median
  dom_hz          frequency of the strongest long-term spectral peak (100 Hz .. 10 kHz)
  event_ms        median event length (onset until 10 dB below the event peak)
  attack_s/decay20_s  (thunder) rise from -20 dB to the peak, fall from peak to -20 dB
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

import numpy as np
import scipy.signal as sg
import soundfile as sf

ML = Path(__file__).resolve().parents[1]
REFS = ML / "data" / "refs"
OUT = ML / "out" / "refs"
SR = 32000
MAX_S = 150.0
CENTERS = [50 * 2 ** (i / 3) for i in range(0, 25)]  # 50 Hz .. ~12.7 kHz
CENTERS = [c for c in CENTERS if c * 2 ** (1 / 6) < SR / 2]
AM_BANDS = [(0.1, 1), (1, 4), (4, 16), (16, 64)]


def load(path: Path) -> np.ndarray:
    try:
        x, sr = sf.read(str(path), always_2d=True, dtype="float32")
    except Exception:
        import librosa

        y, sr = librosa.load(str(path), sr=None, mono=False)
        x = np.atleast_2d(y).T
    x = x.mean(axis=1)
    if sr != SR:
        g = np.gcd(int(sr), SR)
        x = sg.resample_poly(x, SR // g, int(sr) // g).astype(np.float32)
    x = x[: int(MAX_S * SR)]
    x = x - x.mean()
    return x


def db(p, floor=1e-20):
    return 10 * np.log10(np.maximum(p, floor))


def band_profile(x):
    f, p = sg.welch(x, SR, nperseg=8192, noverlap=4096)
    df = f[1] - f[0]
    total = p.sum() * df
    out = {}
    for c in CENTERS:
        m = (f >= c / 2 ** (1 / 6)) & (f < c * 2 ** (1 / 6))
        out[f"band_{int(round(c))}"] = float(db(p[m].sum() * df / total))
    return out, f, p


def envelope(x, hop):
    n = len(x) // hop
    e = np.sqrt((x[: n * hop].reshape(n, hop) ** 2).mean(axis=1) + 1e-14)
    # 40 dB floor below the peak: digital silence between events must not dominate ratios
    return np.maximum(e, e.max() * 1e-2)


def am_spectrum(x):
    hop = 128  # 4 ms -> envelope at 250 Hz
    env = envelope(x, hop)
    rate = SR / hop
    m = env.mean()
    d = env / m - 1
    n = len(d)
    w = np.hanning(n)
    spec = np.abs(np.fft.rfft(d * w)) ** 2 / (w ** 2).sum()
    fr = np.fft.rfftfreq(n, 1 / rate)
    df = fr[1]
    out = {}
    for lo, hi in AM_BANDS:
        sel = (fr >= lo) & (fr < hi)
        # power spectral density integrated over the band = modulation variance share
        out[f"am_{lo:g}-{hi:g}"] = float(db(spec[sel].sum() * 2 / n)) if sel.any() and n * df > 0 else float("nan")
    sel = (fr >= 2) & (fr <= 100)
    sm = np.convolve(spec[sel], np.ones(5) / 5, mode="same")
    out["ampeak_hz"] = float(fr[sel][int(np.argmax(sm))])
    return out


def spectral_stats(x):
    f, _, Z = sg.stft(x, SR, nperseg=2048, noverlap=1536)
    P = np.abs(Z) ** 2 + 1e-18
    band = (f >= 50) & (f <= 12000)
    Pb, fb = P[band], f[band]
    tot = Pb.sum(axis=0)
    keep = tot > tot.max() * 1e-6
    cen = (fb[:, None] * Pb).sum(axis=0) / tot
    flat = np.exp(np.log(Pb).mean(axis=0)) / Pb.mean(axis=0)
    cen, flat = cen[keep], flat[keep]
    return {
        "centroid_med": float(np.median(cen)),
        "centroid_p10": float(np.percentile(cen, 10)),
        "centroid_p90": float(np.percentile(cen, 90)),
        "flatness_db": float(10 * np.log10(np.median(flat))),
    }


def dynamics(x):
    env = envelope(x, int(0.05 * SR))
    e = db(env ** 2)
    return {
        "crest_db": float(np.percentile(e, 99) - e.mean()),
        "spread_db": float(np.percentile(e, 95) - np.percentile(e, 10)),
    }


def onsets_and_events(x):
    hop = int(0.01 * SR)
    e = db(envelope(x, hop) ** 2)
    e = np.maximum(e, e.max() - 70)
    dt = hop / SR
    rise = np.zeros_like(e)
    k = 3
    rise[k:] = e[k:] - e[:-k]
    pk, _ = sg.find_peaks(rise, height=6.0, distance=int(0.04 / dt))
    dur = len(x) / SR
    res = {"onset_rate": float(len(pk) / dur)}
    if len(pk) > 3:
        ioi = np.diff(pk) * dt
        res["ioi_cv"] = float(ioi.std() / ioi.mean())
    else:
        res["ioi_cv"] = float("nan")
    lens = []
    for i, p in enumerate(pk):
        end = pk[i + 1] if i + 1 < len(pk) else min(len(e), p + int(2 / dt))
        seg = e[p:end]
        if len(seg) < 3:
            continue
        j = int(np.argmax(seg))
        below = np.where(seg[j:] < seg[j] - 10)[0]
        lens.append((j + (below[0] if len(below) else len(seg) - j)) * dt)
    res["event_ms"] = float(np.median(lens) * 1000) if lens else float("nan")
    return res


def tonal(f, p):
    sel = (f >= 100) & (f <= 10000)
    fs, ps = f[sel], db(p[sel])
    # 1/3-octave running median on a log axis
    sm = np.empty_like(ps)
    for i, fc in enumerate(fs):
        m = (fs >= fc / 2 ** (1 / 6)) & (fs <= fc * 2 ** (1 / 6))
        sm[i] = np.median(ps[m])
    ex = ps - sm
    i = int(np.argmax(ex))
    return {"tonal_db": float(ex[i]), "tonal_hz": float(fs[i]), "dom_hz": float(fs[int(np.argmax(ps))])}


def thunder_events(x):
    hop = int(0.05 * SR)
    e = db(envelope(x, hop) ** 2)
    e = sg.medfilt(e, 3)
    dt = hop / SR
    floor = np.percentile(e, 10)
    pk, pr = sg.find_peaks(e, prominence=15, distance=int(3 / dt))
    if len(pk) == 0:
        pk = np.array([int(np.argmax(e))])
    att, dec = [], []
    for i, p in enumerate(pk):
        lim = pk[i + 1] if i + 1 < len(pk) else len(e)
        before = np.where(e[:p] < e[p] - 20)[0]
        a = (p - before[-1]) * dt if len(before) else p * dt
        after = np.where(e[p:lim] < e[p] - 20)[0]
        d = (after[0] if len(after) else lim - p) * dt
        att.append(a)
        dec.append(d)
    return {"attack_s": float(np.median(att)), "decay20_s": float(np.median(dec)), "n_thunder": float(len(pk))}


def features(path: Path, category: str = "") -> dict:
    x = load(path)
    if len(x) < SR * 2:
        raise ValueError(f"{path}: too short")
    f = {}
    bands, fr, p = band_profile(x)
    f.update(bands)
    f.update(spectral_stats(x))
    f.update(am_spectrum(x))
    f.update(dynamics(x))
    f.update(onsets_and_events(x))
    f.update(tonal(fr, p))
    if category.startswith("thunder"):
        f.update(thunder_events(x))
    f["duration_s"] = len(x) / SR
    return f


# --- statistics -------------------------------------------------------------------------

# Noise floors for the robust z-score (differences smaller than this never count as large).
FLOOR = {"band": 3.0, "centroid": 0.2, "flatness": 3.0, "am": 3.0, "crest": 3.0, "spread": 3.0,
         "onset": 0.3, "ioi": 0.15, "event": 0.3, "ampeak": 0.2, "tonal": 3.0, "dom": 0.2, "attack": 0.4, "decay20": 0.4}
GROUPS = {"band": "spectrum", "centroid": "spectrum", "flatness": "spectrum", "am": "modulation", "ampeak": "modulation",
          "crest": "dynamics", "spread": "dynamics", "onset": "events", "ioi": "events", "event": "events",
          "tonal": "tonality", "dom": "tonality", "attack": "thunder", "decay20": "thunder"}
RELATIVE = {"ampeak", "centroid", "event", "dom", "onset", "attack", "decay20"}  # scale-type features: log ratio
SKIP_Z = {"duration_s", "n_thunder", "tonal_hz"}


def prefix(k):
    return k.split("_")[0]


def summarize(rows: list[dict]) -> dict:
    keys = sorted({k for r in rows for k in r}, key=lambda k: list(rows[0]).index(k) if k in rows[0] else 999)
    out = {}
    for k in keys:
        v = np.array([r[k] for r in rows if k in r and np.isfinite(r[k])])
        if len(v):
            out[k] = {"median": float(np.median(v)), "q1": float(np.percentile(v, 25)), "q3": float(np.percentile(v, 75)), "n": int(len(v))}
    return out


def zscore(k, ours, s):
    p = prefix(k)
    fl = FLOOR.get(p, 1.0)
    if p in RELATIVE:
        a, b = np.log(max(ours, 1e-6)), np.log(max(s["median"], 1e-6))
        spread = max((np.log(max(s["q3"], 1e-6)) - np.log(max(s["q1"], 1e-6))) / 1.349, fl)
        return (a - b) / spread
    spread = max((s["q3"] - s["q1"]) / 1.349, fl)
    return (ours - s["median"]) / spread


def distance(ours: dict, ref: dict) -> tuple[float, dict]:
    g: dict[str, list[float]] = {}
    for k, s in ref.items():
        if k in SKIP_Z or k not in ours or not np.isfinite(ours[k]) or prefix(k) not in GROUPS:
            continue
        g.setdefault(GROUPS[prefix(k)], []).append(min(abs(zscore(k, ours[k], s)), 4.0))
    gs = {k: float(np.mean(v)) for k, v in g.items()}
    return float(np.mean(list(gs.values()))) if gs else float("nan"), gs


# --- commands ---------------------------------------------------------------------------

AUDIO = (".wav", ".ogg", ".mp3", ".flac")


def ref_files(cat):
    d = REFS / cat
    if not d.is_dir():
        raise SystemExit(f"no references in {d} (run asmr_ml.data.refs_fetch)")
    return sorted(p for p in d.iterdir() if p.suffix.lower() in AUDIO)


def ref_features(cat, refresh=False) -> tuple[dict, dict]:
    jf = OUT / f"{cat}.json"
    cached = json.loads(jf.read_text()) if jf.exists() else {}
    files = cached.get("files", {})
    names = [p.name for p in ref_files(cat)]
    if refresh or set(files) != set(names):
        files = {}
        for p in ref_files(cat):
            try:
                files[p.name] = features(p, cat)
            except Exception as e:
                print(f"skip {p.name}: {e}", file=sys.stderr)
    return files, summarize(list(files.values()))


def save(cat, **kw):
    OUT.mkdir(parents=True, exist_ok=True)
    jf = OUT / f"{cat}.json"
    d = json.loads(jf.read_text()) if jf.exists() else {}
    d.update(kw)
    jf.write_text(json.dumps(d, indent=1))


def fmt(k, v):
    return f"{v:9.2f}" if abs(v) < 1000 else f"{v:9.0f}"


def cmd_refs(a):
    files, summ = ref_features(a.category, refresh=True)
    save(a.category, files=files, reference=summ)
    print(f"{a.category}: {len(files)} references")
    print(f"{'feature':16s} {'median':>9s} {'q1':>9s} {'q3':>9s}")
    for k, s in summ.items():
        print(f"{k:16s} {fmt(k, s['median'])} {fmt(k, s['q1'])} {fmt(k, s['q3'])}")


def cmd_compare(a):
    files, summ = ref_features(a.category)
    if not files:
        raise SystemExit("no usable references")
    res = {}
    for path in a.files:
        ours = features(Path(path), a.category)
        score, gs = distance(ours, summ)
        res[Path(path).name] = {"features": ours, "distance": score, "groups": gs, "flags": {}}
        print(f"\n== {path}  vs {a.category} ({len(files)} refs)   distance {score:.2f}  "
              + "  ".join(f"{k} {v:.1f}" for k, v in gs.items()))
        print(f"{'feature':16s} {'ours':>9s} {'ref med':>9s} {'q1':>9s} {'q3':>9s}  flag")
        for k, s in summ.items():
            if k not in ours or not np.isfinite(ours[k]) or k in ("duration_s", "n_thunder"):
                continue
            v = ours[k]
            flag = ""
            if v < s["q1"] or v > s["q3"]:
                z = zscore(k, v, s) if k not in SKIP_Z else 0.0
                flag = ("LOW " if v < s["q1"] else "HIGH") + (f" z={z:+.1f}" if k not in SKIP_Z else "")
                res[Path(path).name]["flags"][k] = flag
            print(f"{k:16s} {fmt(k, v)} {fmt(k, s['median'])} {fmt(k, s['q1'])} {fmt(k, s['q3'])}  {flag}")
    cmp = json.loads((OUT / f"{a.category}.json").read_text()).get("compare", {}) if (OUT / f"{a.category}.json").exists() else {}
    cmp.update(res)
    save(a.category, files=files, reference=summ, compare=cmp)


TABLE_KEYS = ["band_79", "band_252", "band_504", "band_1008", "band_2016", "band_4032", "band_8063", "centroid_med",
              "flatness_db", "am_0.1-1", "am_1-4", "am_4-16", "am_16-64", "ampeak_hz", "crest_db", "spread_db",
              "onset_rate", "ioi_cv", "event_ms", "tonal_db", "dom_hz", "attack_s", "decay20_s"]


def cmd_report(a):
    """Markdown: sources (attribution) and feature tables for every category with a cached summary."""
    mf = REFS / "manifest.jsonl"
    rows = [json.loads(l) for l in mf.read_text().splitlines() if l.strip()]
    cats = sorted({r["category"] for r in rows})
    print("## Sources and licences\n")
    for c in cats:
        print(f"### {c}\n")
        for r in rows:
            if r["category"] == c and (REFS / r["path"]).exists():
                print(f"- {r['name'][:80]} - {r['author']}, {r['licence']}, <{r['url']}>")
        print()
    print("## Feature tables\n")
    print("Reference: median [q1-q3] over the recordings. Ours: one render per row.\n")
    for c in cats:
        jf = OUT / f"{c}.json"
        if not jf.exists():
            continue
        d = json.loads(jf.read_text())
        ref, cmp = d.get("reference", {}), d.get("compare", {})
        keys = [k for k in TABLE_KEYS if k in ref]
        print(f"### {c} ({len(d.get('files', {}))} references)\n")
        print("| feature | reference | " + " | ".join(Path(n).stem for n in cmp) + " |")
        print("|---|---|" + "---|" * len(cmp))
        for k in keys:
            s = ref[k]
            cells = []
            for n, r in cmp.items():
                v = r["features"].get(k)
                if v is None or not np.isfinite(v):
                    cells.append("-")
                else:
                    cells.append(f"{v:.3g}" + ("*" if k in r["flags"] else ""))
            print(f"| {k} | {s['median']:.3g} [{s['q1']:.3g} .. {s['q3']:.3g}] | " + " | ".join(cells) + " |")
        print("| distance | | " + " | ".join(f"{r['distance']:.2f}" for r in cmp.values()) + " |\n")
    print("`*` = outside the reference interquartile range.")


def cmd_features(a):
    for p in a.files:
        print(json.dumps(features(Path(p), a.category), indent=1))


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)
    r = sub.add_parser("refs")
    r.add_argument("category")
    r.set_defaults(fn=cmd_refs)
    c = sub.add_parser("compare")
    c.add_argument("category")
    c.add_argument("files", nargs="+")
    c.set_defaults(fn=cmd_compare)
    rp = sub.add_parser("report")
    rp.set_defaults(fn=cmd_report)
    f = sub.add_parser("features")
    f.add_argument("files", nargs="+")
    f.add_argument("--category", default="")
    f.set_defaults(fn=cmd_features)
    a = ap.parse_args()
    a.fn(a)


if __name__ == "__main__":
    main()
