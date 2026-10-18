"""Export a trained species (voice + song + contour bank) to an ASNW file for asmr-core.

    uv run python -m asmr_ml.export chaffinch            # -> core/assets/neural/chaffinch.asnw

Format (core/src/neural/weights.rs): "ASNW", u32 version, u32 count, then per tensor
name_len u16, name, dtype u8 (0 f32 / 1 f16), ndim u8, dims u32[], little-endian data.
Network weights are stored as f16 (halves the file, inaudible), configuration as f32.
"""

import argparse
import struct
from pathlib import Path

import numpy as np
import torch

from . import prep
from .dsp import HOP_S, N_HARM, band_centers, band_q
from .song import SongModel
from .train_voice import RUNS
from .voice import LEAK, NOISE_BIAS, Voice

VERSION = 1
ASSETS = Path(__file__).resolve().parents[2] / "core" / "assets" / "neural"


def write_asnw(path: Path, tensors: dict[str, tuple[np.ndarray, int]]) -> int:
    buf = bytearray(b"ASNW")
    buf += struct.pack("<II", VERSION, len(tensors))
    for name, (arr, dtype) in tensors.items():
        arr = np.ascontiguousarray(arr)
        nb = name.encode()
        buf += struct.pack("<H", len(nb)) + nb
        buf += struct.pack("<BB", dtype, arr.ndim)
        buf += struct.pack(f"<{arr.ndim}I", *arr.shape)
        buf += arr.astype("<f2" if dtype == 1 else "<f4").tobytes()
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(bytes(buf))
    return len(buf)


def data_stats(key: str) -> tuple[float, float, float]:
    """Loudness and effort between syllables (what the voice hears as 'silence') and median song interval."""
    quiet, quiet_eff, intervals = [], [], []
    for p in sorted((prep.CLEAN / key).glob("*.npz")):
        d = np.load(p)
        on = np.zeros(len(d["loud"]), bool)
        for a, b in d["syl"]:
            on[a:b] = True
        quiet.append(np.median(d["loud"][~on]))
        quiet_eff.append(np.median(d["effort"][~on]))
        syl, song = d["syl"], d["song"]
        for s in range(1, int(song.max(initial=0)) + 1):
            a = syl[song == s - 1][-1, 1]
            b = syl[song == s][0, 0]
            intervals.append((b - a) * HOP_S)
    return float(np.median(quiet)), float(np.median(quiet_eff)), float(np.clip(np.median(intervals), 2.0, 15.0))


def write_parity(key: str, voice: Voice) -> None:
    """Reference frame outputs for the Rust parity test, computed with f16-rounded weights."""
    q = Voice(hidden=voice.gru.hidden_size)
    q.load_state_dict({k: (v.half().float() if k != "norm" else v) for k, v in voice.state_dict().items()})
    d = np.load(sorted((prep.CLEAN / key).glob("*.npz"))[0])
    a = int(d["syl"][0, 0])
    sl = slice(a, a + 120)
    f0 = torch.from_numpy(d["f0"][sl])[None]
    loud = torch.from_numpy(np.maximum(d["loud"][sl], -100))[None]
    eff = torch.from_numpy(d["effort"][sl])[None]
    with torch.no_grad():
        harm, noise, _ = q(f0, loud, eff)
    write_asnw(ASSETS / "parity" / f"{key}.asnw", {
        "f0": (f0[0].numpy(), 0), "loud": (loud[0].numpy(), 0), "effort": (eff[0].numpy(), 0),
        "harm": (harm[0].numpy(), 0), "noise": (noise[0].numpy(), 0),
    })


def export(key: str, out_gain: float) -> Path:
    v = torch.load(RUNS / key / "voice.pt", map_location="cpu")
    s = torch.load(RUNS / key / "song.pt", map_location="cpu", weights_only=False)
    voice = Voice(hidden=v["hidden"])
    voice.load_state_dict(v["model"])
    song = SongModel(s["k"])
    song.load_state_dict(s["model"])
    silence_db, silence_effort, interval = data_stats(key)

    t: dict[str, tuple[np.ndarray, int]] = {}
    f32 = lambda x: (np.asarray(x, np.float32).reshape(np.shape(x) or (1,)), 0)  # noqa: E731
    t["cfg.voice_norm"] = f32(voice.norm.numpy())
    t["cfg.leak"] = f32(LEAK)
    t["cfg.noise_bias"] = f32(NOISE_BIAS)
    t["cfg.n_harm"] = f32(N_HARM)
    t["cfg.hop_s"] = f32(HOP_S)
    t["cfg.band_q"] = f32(band_q())
    t["cfg.band_centers"] = f32(band_centers())
    t["cfg.silence_db"] = f32(silence_db)
    t["cfg.silence_effort"] = f32(silence_effort)
    t["cfg.out_gain"] = f32(out_gain)
    t["cfg.song_interval_s"] = f32(interval)
    t["cfg.reuse"] = f32(s["reuse"])
    for name, p in voice.state_dict().items():
        if name != "norm":
            t[f"voice.{name}"] = (p.numpy(), 1)
    for name, p in song.state_dict().items():
        t[f"song.{name}"] = (p.numpy(), 1)
    b = s["bank"]
    for name in ("f0", "loud", "effort", "dur", "count", "modes", "coef_sd"):
        t[f"contour.{name}"] = (np.asarray(b[name], np.float32), 0)
    path = ASSETS / f"{key}.asnw"
    n = write_asnw(path, t)
    write_parity(key, voice)
    params = sum(p.numel() for p in voice.parameters()) + sum(p.numel() for p in song.parameters())
    print(f"{path}: {n / 1024:.0f} KiB, {params} parameters, silence {silence_db:.1f} dB, song interval {interval:.1f} s")
    return path


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("species", nargs="+")
    ap.add_argument("--out-gain", type=float, default=0.6)
    a = ap.parse_args()
    for k in a.species:
        export(k, a.out_gain)


if __name__ == "__main__":
    main()
