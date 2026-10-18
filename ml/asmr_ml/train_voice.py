"""Train the per-species DDSP voice (autoencoder: controls extracted from audio -> audio).

    uv run python -m asmr_ml.train_voice chaffinch --steps 20000

Writes ml/runs/<species>/voice.pt (best on held-out recordings) and voice_metrics.json.
"""

import argparse
import json
import math
import time
from pathlib import Path

import numpy as np
import torch

from . import prep
from .dsp import HOP, SR, noise_bank
from .voice import Voice, mr_stft_loss

RUNS = Path(__file__).resolve().parents[1] / "runs"


def device() -> torch.device:
    return torch.device("mps" if torch.backends.mps.is_available() else "cpu")


class Data:
    """All cleaned recordings of a species in memory, split by recording into train / val."""

    def __init__(self, key: str, frames: int, val_every: int = 6):
        self.frames = frames
        self.items = []
        for i, p in enumerate(sorted((prep.CLEAN / key).glob("*.npz"))):
            d = dict(np.load(p))
            active = np.zeros(len(d["loud"]), np.float32)
            for a, b in d["syl"]:
                active[a:b] = 1
            # Window starts whose window is at least 30 % song.
            c = np.concatenate([[0], np.cumsum(active)])
            n = len(active) - frames
            if n <= 0:
                continue
            frac = (c[frames:frames + n] - c[:n]) / frames
            starts = np.flatnonzero(frac >= 0.3)
            if len(starts) == 0:
                continue
            d["starts"] = starts
            d["val"] = i % val_every == val_every - 1
            self.items.append(d)
        self.train = [d for d in self.items if not d["val"]]
        self.val = [d for d in self.items if d["val"]]

    def stats(self) -> list[float]:
        voiced = np.concatenate([np.log2(d["f0"][d["voiced"]]) for d in self.train])
        loud = np.concatenate([d["loud"][d["voiced"]] for d in self.train])
        return [float(voiced.mean()), float(voiced.std()), float(loud.mean()), float(loud.std())]

    def window(self, d, s):
        T = self.frames
        a = d["audio"][s * HOP:(s + T) * HOP]
        a = np.pad(a, (0, T * HOP - len(a)))
        return d["f0"][s:s + T], np.maximum(d["loud"][s:s + T], -100), d["effort"][s:s + T], a

    def batch(self, rng, n, split="train"):
        pool = self.train if split == "train" else self.val
        out = [[], [], [], []]
        for _ in range(n):
            d = pool[rng.integers(len(pool))]
            s = d["starts"][rng.integers(len(d["starts"]))]
            for o, v in zip(out, self.window(d, s)):
                o.append(v)
        return [torch.from_numpy(np.stack(o)) for o in out]


@torch.no_grad()
def evaluate(model: Voice, data: Data, bank, dev, n_windows: int = 96) -> dict:
    """Held-out recordings: MR-STFT, f0 error (cents) and loudness error (dB) of the resynthesis."""
    model.eval()
    rng = np.random.default_rng(1234)
    f0, loud, eff, audio = data.batch(rng, n_windows, "val")
    f0, loud, eff, audio = (t.to(dev) for t in (f0, loud, eff, audio))
    offs = torch.from_numpy(rng.integers(0, bank.shape[1] - audio.shape[1], n_windows)).to(dev)
    y, _, _ = model.synth(f0, loud, eff, bank, offs)
    stft = float(mr_stft_loss(y, audio))
    y, f0, loud = y.cpu().numpy(), f0.cpu().numpy(), loud.cpu().numpy()
    cents, ldb = [], []
    sp = data.species
    for i in range(n_windows):
        f, c, lo, _ = prep.features(y[i], sp.f0_min, sp.f0_max)
        T = len(f0[i])
        f, c, lo = f[:T], c[:T], lo[:T]
        m = (c > 0.6) & (loud[i] > np.percentile(loud[i], 60))
        if m.any():
            cents.append(np.median(np.abs(1200 * np.log2(f[m] / f0[i][m]))))
            ldb.append(np.median(np.abs(lo[m] - loud[i][m])))
    model.train()
    return {"val_mr_stft": stft, "val_f0_err_cents": float(np.mean(cents)), "val_loud_err_db": float(np.mean(ldb))}


def main() -> None:
    from .species import SPECIES

    ap = argparse.ArgumentParser()
    ap.add_argument("species")
    ap.add_argument("--steps", type=int, default=20000)
    ap.add_argument("--batch", type=int, default=32)
    ap.add_argument("--frames", type=int, default=200)  # 1 s windows
    ap.add_argument("--lr", type=float, default=1e-3)
    ap.add_argument("--hidden", type=int, default=96)
    a = ap.parse_args()

    dev = device()
    torch.manual_seed(0)
    data = Data(a.species, a.frames)
    data.species = SPECIES[a.species]
    print(f"{a.species}: {len(data.train)} train / {len(data.val)} val recordings on {dev}")
    bank = torch.from_numpy(noise_bank(20.0, seed=0)).to(dev)
    model = Voice(hidden=a.hidden).to(dev)
    model.norm.copy_(torch.tensor(data.stats()))
    n_params = sum(p.numel() for p in model.parameters())
    print(f"params {n_params}, norm {model.norm.tolist()}")
    opt = torch.optim.Adam(model.parameters(), lr=a.lr)
    sched = torch.optim.lr_scheduler.LambdaLR(opt, lambda s: 0.5 * (1 + math.cos(math.pi * min(s / a.steps, 1))) * 0.95 + 0.05)
    out = RUNS / a.species
    out.mkdir(parents=True, exist_ok=True)
    rng = np.random.default_rng(0)
    best, hist, t0 = float("inf"), [], time.time()
    for step in range(1, a.steps + 1):
        f0, loud, eff, audio = (t.to(dev) for t in data.batch(rng, a.batch))
        offs = torch.from_numpy(rng.integers(0, bank.shape[1] - audio.shape[1], a.batch)).to(dev)
        y, _, _ = model.synth(f0, loud, eff, bank, offs)
        loss = mr_stft_loss(y, audio)
        opt.zero_grad()
        loss.backward()
        torch.nn.utils.clip_grad_norm_(model.parameters(), 2.0)
        opt.step()
        sched.step()
        if step % 200 == 0:
            print(f"step {step:6d} loss {loss.item():.4f} ({(time.time() - t0) / step * 1000:.0f} ms/step)", flush=True)
        if step % 2000 == 0 or step == a.steps:
            m = evaluate(model, data, bank, dev)
            m["step"] = step
            hist.append(m)
            print("  val", m, flush=True)
            if m["val_mr_stft"] < best:
                best = m["val_mr_stft"]
                torch.save({"model": model.state_dict(), "hidden": a.hidden, "species": a.species}, out / "voice.pt")
    final = {"species": a.species, "params": n_params, "steps": a.steps, "train_recordings": len(data.train),
             "val_recordings": len(data.val), "best_val_mr_stft": best, "history": hist,
             "minutes": round((time.time() - t0) / 60, 1)}
    (out / "voice_metrics.json").write_text(json.dumps(final, indent=1))


if __name__ == "__main__":
    main()
