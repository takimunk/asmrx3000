# /// script
# requires-python = ">=3.10,<3.14"
# dependencies = ["numpy>=1.26", "h5py>=3.10", "scipy>=1.11"]
# ///
"""Convert a SOFA (SimpleFreeFieldHRIR) file into the compact "AHRT" v1 binary
HRIR asset loaded by asmr-core.

Usage:
    uv run tools/hrtf/sofa_to_ahrt.py <in.sofa> <out.ahrt> [--dirs N] [--taps T]
                                     [--name "SADIE II KU100 48k"]

AHRT v1 layout (little endian, tightly packed):
    4s   magic "AHRT"
    u32  version = 1
    u32  sample_rate
    u32  num_dirs
    u32  taps
    f32  scale                 int16 * scale = float HRIR sample
    u32  name_len, name bytes (UTF-8), zero padding to a multiple of 4 bytes
    num_dirs x [f32 x, f32 y, f32 z]        unit vectors, +x fwd, +y left, +z up
    num_dirs x [i16 left[taps], i16 right[taps]]

The input file is treated as data only (read through h5py; nothing executed).
"""

from __future__ import annotations

import argparse
import math
import struct
import sys
from fractions import Fraction
from pathlib import Path

import h5py
import numpy as np
from scipy.signal import resample_poly

TARGET_FS = 48000
ONSET_TARGET = 12          # earliest onset lands on this sample
ONSET_THRESH_DB = -20.0    # onset = first sample above peak * 10^(dB/20)
FADE_LEN = 16              # half-Hann fade-out at the end
NORM_BAND = (200.0, 800.0)
MAGIC = b"AHRT"
VERSION = 1


# --------------------------------------------------------------------------- SOFA

def _attr_str(obj, key: str) -> str:
    v = obj.attrs.get(key)
    if v is None:
        return ""
    if isinstance(v, bytes):
        return v.decode("utf-8", "replace")
    if isinstance(v, np.ndarray) and v.dtype.kind in "SO":
        v = v.flat[0]
        return v.decode("utf-8", "replace") if isinstance(v, bytes) else str(v)
    return str(v)


def _positions_to_cartesian(pos: np.ndarray, kind: str) -> np.ndarray:
    """SOFA position array (M x 3) -> cartesian (M x 3) in the SOFA frame
    (+x front, +y left, +z up), which is also the engine frame."""
    kind = kind.lower()
    if kind.startswith("spherical"):
        az = np.deg2rad(pos[:, 0])
        el = np.deg2rad(pos[:, 1])
        r = pos[:, 2] if pos.shape[1] > 2 else np.ones(len(pos))
        return np.stack([r * np.cos(el) * np.cos(az),
                         r * np.cos(el) * np.sin(az),
                         r * np.sin(el)], axis=1)
    if kind.startswith("cartesian"):
        return pos.astype(np.float64)
    raise ValueError(f"unsupported position Type {kind!r}")


def read_sofa(path: Path):
    with h5py.File(path, "r") as f:
        conv = _attr_str(f, "SOFAConventions")
        if conv and conv != "SimpleFreeFieldHRIR":
            print(f"warning: SOFAConventions = {conv!r} (expected SimpleFreeFieldHRIR)",
                  file=sys.stderr)
        ir = np.asarray(f["Data.IR"][...], dtype=np.float64)          # M x R x N
        fs = float(np.asarray(f["Data.SamplingRate"][...]).flat[0])
        sp_ds = f["SourcePosition"]
        src = np.asarray(sp_ds[...], dtype=np.float64)
        src_type = _attr_str(sp_ds, "Type") or "spherical"
        if src.ndim == 1:
            src = src[None, :]
        if src.shape[0] == 1 and ir.shape[0] > 1:
            raise ValueError("SourcePosition is constant; cannot map directions")

        rp_ds = f["ReceiverPosition"]
        rp = np.asarray(rp_ds[...], dtype=np.float64)
        rp = rp.reshape(rp.shape[0], rp.shape[1], -1)[:, :, 0]       # R x C
        rp_cart = _positions_to_cartesian(rp, _attr_str(rp_ds, "Type") or "cartesian")

        delay = np.zeros((1, ir.shape[1]))
        if "Data.Delay" in f:
            delay = np.asarray(f["Data.Delay"][...], dtype=np.float64)
            delay = delay.reshape(-1, ir.shape[1])

        view = np.asarray(f["ListenerView"][...], dtype=np.float64).reshape(-1, 3)[0] \
            if "ListenerView" in f else np.array([1.0, 0.0, 0.0])
        view_type = _attr_str(f["ListenerView"], "Type") if "ListenerView" in f else "cartesian"
        view = _positions_to_cartesian(view[None, :], view_type or "cartesian")[0]

        meta = {k: _attr_str(f, k) for k in
                ("Title", "DatabaseName", "ListenerShortName", "History", "License",
                 "Organization", "References", "Comment", "DateModified")}

    if ir.shape[1] != 2:
        raise ValueError(f"expected 2 receivers, got {ir.shape[1]}")
    if np.linalg.norm(view - np.array([view[0], 0, 0])) > 1e-6 or view[0] <= 0:
        print(f"warning: ListenerView = {view}, expected +x; directions not rotated",
              file=sys.stderr)

    # Receiver order: left ear has +y.
    if rp_cart[0, 1] > rp_cart[1, 1]:
        order = "receiver 0 = left (+y), receiver 1 = right"
    elif rp_cart[0, 1] < rp_cart[1, 1]:
        order = "receiver 0 = RIGHT (-y) -> swapped"
        ir = ir[:, ::-1, :]
        delay = delay[:, ::-1]
    else:
        order = "ReceiverPosition ambiguous; assuming receiver 0 = left"

    # Apply Data.Delay (in samples) if present and non-zero.
    if np.any(delay != 0):
        d = np.broadcast_to(np.rint(delay).astype(int), (ir.shape[0], 2))
        pad = int(d.max())
        out = np.zeros((ir.shape[0], 2, ir.shape[2] + pad))
        for m in range(ir.shape[0]):
            for r in range(2):
                out[m, r, d[m, r]:d[m, r] + ir.shape[2]] = ir[m, r]
        ir = out

    dirs = _positions_to_cartesian(src, src_type)
    dirs /= np.linalg.norm(dirs, axis=1, keepdims=True)
    return ir, fs, dirs, src, src_type, rp_cart, order, meta


# --------------------------------------------------------------------------- processing

def fibonacci_sphere(n: int) -> np.ndarray:
    i = np.arange(n) + 0.5
    z = 1.0 - 2.0 * i / n
    r = np.sqrt(np.maximum(0.0, 1.0 - z * z))
    phi = i * math.pi * (3.0 - math.sqrt(5.0))
    return np.stack([r * np.cos(phi), r * np.sin(phi), z], axis=1)


def unique_directions(dirs: np.ndarray, tol_deg: float = 0.05) -> np.ndarray:
    """Indices of the first occurrence of each distinct measured direction."""
    keep: list[int] = []
    cos_tol = math.cos(math.radians(tol_deg))
    kept = np.empty((0, 3))
    for i, d in enumerate(dirs):
        if kept.shape[0] and np.max(kept @ d) >= cos_tol:
            continue
        keep.append(i)
        kept = np.vstack([kept, d])
    return np.asarray(keep, dtype=int)


def select_directions(dirs: np.ndarray, n: int) -> np.ndarray:
    uniq = unique_directions(dirs)
    if len(uniq) <= n:
        return uniq
    cand = dirs[uniq]
    fib = fibonacci_sphere(n)
    nearest = np.argmax(fib @ cand.T, axis=1)
    sel = np.unique(nearest)          # dedupe (sorted)
    return uniq[sel]


def onset_index(h: np.ndarray, thresh_db: float = ONSET_THRESH_DB) -> int:
    a = np.abs(h)
    pk = a.max()
    if pk <= 0:
        return len(h)
    return int(np.argmax(a >= pk * 10 ** (thresh_db / 20)))


def band_mean_mag(h: np.ndarray, fs: float, band=NORM_BAND, nfft: int = 8192) -> float:
    H = np.abs(np.fft.rfft(h, nfft))
    f = np.fft.rfftfreq(nfft, 1 / fs)
    m = (f >= band[0]) & (f <= band[1])
    return float(H[m].mean())


def nearest(dirs: np.ndarray, v) -> int:
    v = np.asarray(v, dtype=np.float64)
    return int(np.argmax(dirs @ (v / np.linalg.norm(v))))


# --------------------------------------------------------------------------- AHRT io

def write_ahrt(path: Path, fs: int, dirs: np.ndarray, q: np.ndarray, scale: float, name: str):
    nd, _, taps = q.shape
    nb = name.encode("utf-8")
    pad = (-len(nb)) % 4
    with open(path, "wb") as f:
        f.write(MAGIC)
        f.write(struct.pack("<IIIIf", VERSION, fs, nd, taps, scale))
        f.write(struct.pack("<I", len(nb)))
        f.write(nb + b"\0" * pad)
        f.write(dirs.astype("<f4").tobytes())
        f.write(q.astype("<i2").tobytes())


def read_ahrt(path: Path):
    b = Path(path).read_bytes()
    if b[:4] != MAGIC:
        raise ValueError("bad magic")
    ver, fs, nd, taps, scale = struct.unpack_from("<IIIIf", b, 4)
    if ver != VERSION:
        raise ValueError(f"unsupported version {ver}")
    (nlen,) = struct.unpack_from("<I", b, 24)
    off = 28
    name = b[off:off + nlen].decode("utf-8")
    off += nlen + ((-nlen) % 4)
    dirs = np.frombuffer(b, "<f4", nd * 3, off).reshape(nd, 3)
    off += nd * 12
    q = np.frombuffer(b, "<i2", nd * 2 * taps, off).reshape(nd, 2, taps)
    off += nd * 2 * taps * 2
    if off != len(b):
        raise ValueError(f"trailing/missing bytes: parsed {off}, file {len(b)}")
    return fs, dirs, q.astype(np.float64) * scale, name, len(b)


# --------------------------------------------------------------------------- report

def itd_us(l: np.ndarray, r: np.ndarray, fs: float, lowpass_hz: float | None = None,
           up: int = 8) -> float:
    """Lag of right relative to left (positive = left ear leads), in microseconds."""
    if lowpass_hz:
        from scipy.signal import butter, sosfiltfilt
        sos = butter(4, lowpass_hz, fs=fs, output="sos")
        l, r = sosfiltfilt(sos, l), sosfiltfilt(sos, r)
    lu, ru = resample_poly(l, up, 1), resample_poly(r, up, 1)
    xc = np.correlate(ru, lu, mode="full")
    lag = int(np.argmax(xc)) - (len(lu) - 1)
    return lag / (fs * up) * 1e6


def sanity_report(path: Path):
    fs, dirs, h, name, size = read_ahrt(path)
    nd, _, taps = h.shape
    print(f"--- sanity report: {path}")
    print(f"name        : {name}")
    print(f"sample rate : {fs} Hz")
    print(f"num_dirs    : {nd}")
    print(f"taps        : {taps}")
    print(f"file size   : {size} bytes ({size / 1024:.1f} KiB)")
    norms = np.linalg.norm(dirs, axis=1)
    print(f"|dir| range : {norms.min():.6f} .. {norms.max():.6f}")
    print(f"elev range  : {np.degrees(np.arcsin(dirs[:, 2].clip(-1, 1))).min():.1f} .. "
          f"{np.degrees(np.arcsin(dirs[:, 2].clip(-1, 1))).max():.1f} deg")
    for label, v in (("left  (+y)", (0, 1, 0)), ("right (-y)", (0, -1, 0)),
                     ("front (+x)", (1, 0, 0)), ("up    (+z)", (0, 0, 1))):
        i = nearest(dirs, v)
        el, er = np.sum(h[i, 0] ** 2), np.sum(h[i, 1] ** 2)
        print(f"ILD {label}: {10 * np.log10(el / er):+6.2f} dB   "
              f"(dir {np.round(dirs[i], 3)}, L-R energy)")
    i = nearest(dirs, (0, 1, 0))
    print(f"ITD left dir, broadband xcorr : {itd_us(h[i, 0], h[i, 1], fs):+7.1f} us "
          f"(positive = left ear leads)")
    print(f"ITD left dir, <1.5 kHz xcorr  : {itd_us(h[i, 0], h[i, 1], fs, 1500.0):+7.1f} us")
    on_l, on_r = onset_index(h[i, 0]), onset_index(h[i, 1])
    print(f"onsets left dir (-20 dB)      : L {on_l}  R {on_r}  "
          f"-> {(on_r - on_l) / fs * 1e6:+.1f} us")
    j = nearest(dirs, (1, 0, 0))
    print(f"front 200-800 Hz mean |H|     : L {band_mean_mag(h[j, 0], fs):.4f}  "
          f"R {band_mean_mag(h[j, 1], fs):.4f}")
    print(f"global peak |h|               : {np.abs(h).max():.4f}")
    print(f"earliest onset (all dirs)     : "
          f"{min(onset_index(x) for x in h.reshape(-1, taps))}")


# --------------------------------------------------------------------------- main

def main(argv=None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("input", type=Path)
    ap.add_argument("output", type=Path)
    ap.add_argument("--dirs", type=int, default=600, help="target direction count")
    ap.add_argument("--taps", type=int, default=256, help="HRIR length in samples")
    ap.add_argument("--name", type=str, default=None, help="description stored in file")
    a = ap.parse_args(argv)
    if a.taps <= FADE_LEN + ONSET_TARGET:
        ap.error("--taps too small")

    ir, fs, dirs, src, src_type, rp, order, meta = read_sofa(a.input)
    print(f"input       : {a.input}")
    print(f"database    : {meta['DatabaseName']} / {meta['Title']} / {meta['ListenerShortName']}")
    print(f"history     : {meta['History']}")
    print(f"IR          : {ir.shape[0]} x {ir.shape[1]} x {ir.shape[2]} @ {fs:g} Hz")
    print(f"source pos  : {src_type}, e.g. {src[0]}")
    print(f"receivers   : {order}  (ReceiverPosition {rp.tolist()})")

    # Resample.
    if int(round(fs)) != TARGET_FS:
        fr = Fraction(TARGET_FS, int(round(fs))).limit_denominator(1000)
        ir = resample_poly(ir, fr.numerator, fr.denominator, axis=-1)
        print(f"resampled   : {fs:g} -> {TARGET_FS} Hz ({fr.numerator}/{fr.denominator})")
    fs = TARGET_FS

    # Direction subset.
    sel = select_directions(dirs, a.dirs)
    print(f"directions  : {len(unique_directions(dirs))} unique measured, "
          f"{len(sel)} kept (Fibonacci target {a.dirs})")
    h = ir[sel]
    d = dirs[sel]

    # Common leading delay removal (same shift for every IR -> ITDs preserved).
    onsets = np.array([[onset_index(h[m, r]) for r in range(2)] for m in range(len(h))])
    first = int(onsets.min())
    shift = first - ONSET_TARGET
    if shift > 0:
        h = h[:, :, shift:]
    elif shift < 0:
        h = np.concatenate([np.zeros(h.shape[:2] + (-shift,)), h], axis=2)
    print(f"onset       : earliest {first} samples -> shifted by {shift} to {ONSET_TARGET}")

    # Truncate / pad + half-Hann fade-out.
    n_in = h.shape[2]
    if n_in >= a.taps:
        tail = np.abs(h[:, :, a.taps:]) if n_in > a.taps else np.zeros(1)
        h = h[:, :, :a.taps].copy()
        e_lost = float(np.sum(tail ** 2) / max(np.sum(ir[sel] ** 2), 1e-30))
        print(f"truncate    : {n_in} -> {a.taps} taps (energy beyond cut {10*np.log10(e_lost+1e-30):.1f} dB)")
    else:
        h = np.concatenate([h, np.zeros(h.shape[:2] + (a.taps - n_in,))], axis=2)
        print(f"pad         : {n_in} -> {a.taps} taps")
    fade = 0.5 * (1 + np.cos(np.pi * np.arange(1, FADE_LEN + 1) / FADE_LEN))  # ->0
    h[:, :, -FADE_LEN:] *= fade

    # Normalise: frontal 200-800 Hz mean magnitude (avg of both ears) = 1.
    fi = nearest(d, (1, 0, 0))
    g = 0.5 * (band_mean_mag(h[fi, 0], fs) + band_mean_mag(h[fi, 1], fs))
    h /= g
    print(f"normalise   : front dir {np.round(d[fi], 4)}, 200-800 Hz gain {g:.4f} -> 1.0")

    # Quantise.
    scale = float(np.abs(h).max() / 32767.0)
    q = np.clip(np.rint(h / scale), -32768, 32767).astype(np.int16)
    err = h - q * scale
    print(f"quantise    : scale {scale:.6e}, SNR {10*np.log10(np.sum(h**2)/np.sum(err**2)):.1f} dB")

    name = a.name or " ".join(x for x in (meta["DatabaseName"], meta["ListenerShortName"],
                                          f"{fs // 1000}k") if x)
    a.output.parent.mkdir(parents=True, exist_ok=True)
    write_ahrt(a.output, fs, d.astype(np.float32), q, np.float32(scale).item(), name)
    print(f"wrote       : {a.output}")
    sanity_report(a.output)
    return 0


if __name__ == "__main__":
    sys.exit(main())
