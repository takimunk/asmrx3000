"""Raw recordings -> cleaned 48 kHz audio + frame features + syllable segmentation.

    uv run python -m asmr_ml.prep chaffinch

Per recording (ml/data/clean/<species>/<n>.npz):
  audio   float32 [N]   high-passed, spectrally gated, peak-normalised
  f0      float32 [T]   Hz, dominant-frequency tracker; filled between syllables
  conf    float32 [T]   0..1 tonal confidence (peak energy / band energy)
  loud    float32 [T]   dB, frame power in the species band
  effort  float32 [T]   0..1, share of band energy away from f0 (0 whistle, 1 buzzy)
  syl     int32 [S, 2]  syllable [start, end) frames
  song    int32 [S]     song (phrase) index of each syllable
Recordings whose song-to-noise ratio is below --min-snr are skipped (listed in prep.json).

Bird song is 1.5-9 kHz, outside CREPE's range (~2 kHz max), so f0 comes from a
dominant-frequency tracker with sub-harmonic check, which suits near-tonal songbirds.
"""

import argparse
import json
import re
from pathlib import Path

import librosa
import numpy as np
import soundfile as sf
from scipy.ndimage import median_filter, uniform_filter
from scipy.signal import butter, find_peaks, sosfiltfilt

from .data import manifest
from .dsp import HOP, SR
from .species import SPECIES

CLEAN = manifest.DATA / "clean"
# Titles of mixed recordings (several species): the DDSP voice must hear one species.
SOUNDSCAPE = re.compile(r"\b(birds|soundscape|vogels|vogelzwitschern|dawn chorus)\b", re.I)
N_FFT = 2048  # zero-padded analysis FFT for f0 (window WIN)
WIN = 256


def load(path: Path) -> np.ndarray:
    x, sr = sf.read(path, dtype="float32", always_2d=True)
    x = x.mean(axis=1)
    if sr != SR:
        x = librosa.resample(x, orig_sr=sr, target_sr=SR, res_type="soxr_hq")
    return x


def highpass(x: np.ndarray, fc: float) -> np.ndarray:
    sos = butter(6, fc, "highpass", fs=SR, output="sos")
    return sosfiltfilt(sos, x).astype(np.float32)


def spectral_gate(x: np.ndarray, strength: float = 1.5) -> tuple[np.ndarray, float]:
    """Subtract a stationary noise profile (15th percentile per bin). Returns (clean, floor_db)."""
    X = librosa.stft(x, n_fft=1024, hop_length=256)
    mag = np.abs(X)
    noise = np.percentile(mag, 15, axis=1, keepdims=True)
    mask = np.clip(1.0 - strength * noise / np.maximum(mag, 1e-9), 0.0, 1.0)
    mask = uniform_filter(mask, size=(3, 3))
    y = librosa.istft(X * mask, hop_length=256, length=len(x))
    floor_db = 10 * np.log10(np.mean(noise**2) * 2 / 1024**1 + 1e-12)
    return y.astype(np.float32), float(floor_db)


def features(x: np.ndarray, f0_min: float, f0_max: float):
    """Frame i is centred on sample i * HOP."""
    S = np.abs(librosa.stft(x, n_fft=N_FFT, win_length=WIN, hop_length=HOP, window="hann")) ** 2
    freqs = librosa.fft_frequencies(sr=SR, n_fft=N_FFT)
    band = (freqs >= f0_min * 0.8) & (freqs <= 16_000)
    sb = S[band]
    fb = freqs[band]
    df = freqs[1]
    e_band = sb.sum(0) + 1e-12
    # Hann window of WIN samples: power sum -> mean-square of the frame.
    loud = 10 * np.log10(e_band * 2 / (WIN * 0.375 * WIN) + 1e-12)

    lo, hi = np.searchsorted(fb, [f0_min, f0_max])
    sub = sb[lo:hi]
    k = sub.argmax(0)
    # Parabolic interpolation on log power.
    kk = np.clip(k, 1, sub.shape[0] - 2)
    cols = np.arange(sub.shape[1])
    a, b, c = (np.log(sub[kk + d, cols] + 1e-20) for d in (-1, 0, 1))
    off = np.clip(0.5 * (a - c) / np.where(np.abs(a - 2 * b + c) > 1e-9, a - 2 * b + c, 1e-9), -1, 1)
    f0 = fb[lo + kk] + off * df
    # Sub-harmonic check: if there is strong energy at f0/2 inside the range, that is the f0.
    half = f0 / 2
    hk = np.round((half - freqs[0]) / df).astype(int)
    ok = half >= f0_min
    hp = S[np.clip(hk, 0, len(freqs) - 1), cols]
    pk = sub[k, cols]
    f0 = np.where(ok & (hp > 0.3 * pk), half, f0)

    # Tonal energy: within +-300 Hz of f0 and its harmonics (Hann-256 main lobe is +-375 Hz).
    tonal = np.zeros_like(e_band)
    for h in range(1, 5):
        near = np.abs(fb[:, None] - h * f0[None, :]) < 300
        tonal += (sb * near).sum(0)
    conf = np.clip(tonal / e_band, 0, 1)
    effort = 1.0 - conf
    return f0.astype(np.float32), conf.astype(np.float32), loud.astype(np.float32), effort.astype(np.float32)


def segment(loud: np.ndarray, f0: np.ndarray, conf: np.ndarray, floor_db: float, song_gap_s: float = 0.5):
    """Active regions by threshold, then split into syllables at loudness dips and f0 jumps.

    Reverb and fast trills fill the gaps between syllables, so a plain threshold finds whole
    phrases; a chaffinch syllable is a fast down-sweep, so the jump back up in f0 is the cue.
    """
    peak = np.percentile(loud, 99.5)
    thr = max(floor_db + 12, peak - 35)
    on = median_filter((loud > thr).astype(np.uint8), size=3).astype(bool)
    edges = np.flatnonzero(np.diff(np.concatenate([[0], on.astype(np.int8), [0]])))
    regions = edges.reshape(-1, 2)
    regions = regions[(regions[:, 1] - regions[:, 0]) >= 3]

    smooth = uniform_filter(loud, 3)
    dips, _ = find_peaks(-smooth, prominence=5.0)
    lf = np.log2(np.maximum(f0, 1.0))
    tonal = conf > 0.4
    jumps = np.flatnonzero((np.abs(np.diff(lf)) > 0.35) & tonal[1:] & tonal[:-1]) + 1
    cuts = np.union1d(dips, jumps)

    syl, song_of_region = [], []
    gap = int(song_gap_s / (HOP / SR))
    song = 0
    for i, (a, b) in enumerate(regions):
        if i and a - regions[i - 1, 1] > gap:
            song += 1
        inner = cuts[(cuts > a + 2) & (cuts < b - 2)]
        bounds = [a, *inner.tolist(), b]
        pieces = []
        for x, y in zip(bounds[:-1], bounds[1:]):
            if pieces and y - x < 3:
                pieces[-1][1] = y  # too short to stand alone: merge into the previous piece
            else:
                pieces.append([x, y])
        for x, y in pieces:
            if y - x >= 3:
                syl.append((x, y))
                song_of_region.append(song)
    return np.array(syl, np.int32).reshape(-1, 2), np.array(song_of_region, np.int32), thr


def fill_f0(f0: np.ndarray, voiced: np.ndarray) -> np.ndarray:
    """Interpolate log f0 across unvoiced frames so the network input stays smooth."""
    idx = np.flatnonzero(voiced)
    if len(idx) == 0:
        return f0
    lf = np.interp(np.arange(len(f0)), idx, np.log2(f0[idx]))
    return (2**lf).astype(np.float32)


def process(key: str, min_snr: float, kinds: tuple[str, ...]) -> None:
    sp = SPECIES[key]
    out = CLEAN / key
    out.mkdir(parents=True, exist_ok=True)
    report = []
    recs = sorted(manifest.usable(key), key=lambda r: r.id)
    for n, r in enumerate(recs):
        if r.extra.get("species_in_file", 1) > 1 or SOUNDSCAPE.search(r.id):
            report.append({"id": r.id, "skip": "multi-species soundscape"})
            continue
        if r.kind and not any(k in r.kind.lower() for k in kinds):
            report.append({"id": r.id, "skip": f"kind '{r.kind}'"})
            continue
        try:
            x = load(manifest.RAW / r.path)
        except Exception as e:  # noqa: BLE001 - corrupt or unsupported file
            report.append({"id": r.id, "skip": f"decode: {e}"})
            continue
        if len(x) < SR:
            report.append({"id": r.id, "skip": "too short"})
            continue
        x = highpass(x, sp.f0_min * 0.7)
        y, floor_db = spectral_gate(x)
        f0, conf, loud, effort = features(y, sp.f0_min, sp.f0_max)
        _, _, loud_raw, _ = features(x, sp.f0_min, sp.f0_max)
        floor_raw = float(np.percentile(loud_raw, 20))
        syl, song, thr = segment(loud, f0, conf, float(np.percentile(loud, 20)))
        if len(syl) == 0:
            report.append({"id": r.id, "skip": "no syllables"})
            continue
        on = np.zeros(len(loud), bool)
        for a, b in syl:
            on[a:b] = True
        snr = float(np.percentile(loud_raw[on], 75) - floor_raw)
        if snr < min_snr:
            report.append({"id": r.id, "skip": f"snr {snr:.1f} dB"})
            continue
        voiced = on & (conf > 0.4)
        f0 = fill_f0(f0, voiced)
        # Silence outside syllables so the model is not trained on residual noise.
        gate = np.repeat(median_filter(on.astype(np.float32), 5), HOP)[: len(y)]
        gate = np.pad(gate, (0, len(y) - len(gate)), mode="edge")
        gate = np.convolve(gate, np.hanning(481) / np.hanning(481).sum(), mode="same")
        y = y * np.maximum(gate, 0.03)
        peak = np.abs(y).max() + 1e-9
        y = (y / peak * 0.5).astype(np.float32)
        loud = loud + 20 * np.log10(0.5 / peak)
        np.savez(out / f"{n:04d}.npz", audio=y, f0=f0, conf=conf, loud=loud.astype(np.float32),
                 effort=effort, syl=syl, song=song, voiced=voiced, id=r.id)
        report.append({"id": r.id, "file": f"{n:04d}.npz", "snr": round(snr, 1), "syllables": len(syl),
                       "songs": int(song[-1]) + 1, "seconds": round(len(y) / SR, 1)})
        print(f"{key} {n:04d}: snr {snr:5.1f} dB, {len(syl):4d} syllables, {int(song[-1]) + 1:3d} songs, {r.id}")
    (out / "prep.json").write_text(json.dumps(report, indent=1, ensure_ascii=False))
    kept = [x for x in report if "file" in x]
    print(f"{key}: kept {len(kept)}/{len(report)}, {sum(x['seconds'] for x in kept) / 60:.1f} min, "
          f"{sum(x['syllables'] for x in kept)} syllables")


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("species", nargs="*", default=list(SPECIES))
    ap.add_argument("--min-snr", type=float, default=15.0)
    ap.add_argument("--kinds", default="song", help="comma list; recordings with another type are skipped")
    a = ap.parse_args()
    for k in a.species:
        process(k, a.min_snr, tuple(a.kinds.split(",")))


if __name__ == "__main__":
    main()
