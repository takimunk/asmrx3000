"""Song syntax: syllables -> tokens (k-means on contours) -> GRU language model over tokens.

    uv run python -m asmr_ml.song chaffinch

Produces ml/runs/<species>/song.pt with
  - contour bank per token: up to EXEMPLARS real syllables (f0 / loudness / effort at P points,
    log duration) plus PCA modes of the token's f0 + loudness spread for small variations;
  - SongModel: GRU over the previous token -> next token (or end of song) + log gap before it.
The Rust side (core/src/neural/song.rs) samples tokens, then a contour per token, and feeds the
DDSP voice frame by frame.
"""

import argparse
import json
import math

import numpy as np
import torch
import torch.nn.functional as F
from torch import nn

from . import prep
from .dsp import HOP_S
from .train_voice import RUNS

P = 24  # contour points per syllable
MODES = 3
EXEMPLARS = 8  # real syllables kept per token


def resample(x: np.ndarray, n: int = P) -> np.ndarray:
    return np.interp(np.linspace(0, len(x) - 1, n), np.arange(len(x)), x)


def syllables(key: str, val_every: int = 6):
    """Per recording: list of songs, each a list of syllable dicts."""
    recs = []
    for i, p in enumerate(prep.prepared_files(prep.CLEAN / key)):
        d = np.load(p)
        songs: dict[int, list] = {}
        for (a, b), s in zip(d["syl"], d["song"]):
            n = b - a
            if n < 3 or n * HOP_S > 0.6 or d["conf"][a:b].mean() < 0.35:
                continue
            songs.setdefault(int(s), []).append({
                "start": int(a), "end": int(b),
                "f0": resample(np.log2(d["f0"][a:b])),
                "loud": resample(d["loud"][a:b]),
                "effort": resample(d["effort"][a:b]),
                "dur": n * HOP_S,
            })
        recs.append({"val": i % val_every == val_every - 1, "songs": [v for v in songs.values() if len(v) >= 2]})
    return recs


def features(s) -> np.ndarray:
    return np.concatenate([s["f0"] * 4.0, (s["loud"] - s["loud"].max()) / 10.0, [2.0 * math.log(s["dur"])]])


def kmeans(x: np.ndarray, k: int, iters: int = 60, seed: int = 0) -> tuple[np.ndarray, np.ndarray]:
    rng = np.random.default_rng(seed)
    # k-means++ init.
    c = [x[rng.integers(len(x))]]
    for _ in range(1, k):
        d = np.min(((x[:, None] - np.array(c)[None]) ** 2).sum(-1), 1)
        c.append(x[rng.choice(len(x), p=d / d.sum())])
    c = np.array(c)
    for _ in range(iters):
        lab = ((x[:, None] - c[None]) ** 2).sum(-1).argmin(1)
        for j in range(k):
            if (lab == j).any():
                c[j] = x[lab == j].mean(0)
    return c, lab


class SongModel(nn.Module):
    """Previous token (or BOS) -> next token (K + EOS) and log gap (mu, log sd) before it."""

    def __init__(self, k: int, embed: int = 16, hidden: int = 48, dropout: float = 0.25):
        super().__init__()
        self.k = k
        self.emb = nn.Embedding(k + 1, embed)  # index k = BOS
        self.gru = nn.GRU(embed, hidden, batch_first=True)
        self.drop = nn.Dropout(dropout)  # training only; nothing to mirror in Rust
        self.out = nn.Linear(hidden, k + 1 + 2)  # logits (k = EOS), gap mu, gap log sd

    def forward(self, prev):
        g, _ = self.gru(self.drop(self.emb(prev)))
        y = self.out(self.drop(g))
        return y[..., : self.k + 1], y[..., self.k + 1], y[..., self.k + 2]


def sequences(recs, lab_of, k):
    """(prev, target, log_gap, gap_mask) tensors, padded; target k = EOS."""
    seqs = []
    for r in recs:
        for song in r["songs"]:
            toks = [lab_of[id(s)] for s in song]
            gaps = [HOP_S] + [max(b["start"] - a["end"], 1) * HOP_S for a, b in zip(song[:-1], song[1:])]
            seqs.append((toks, gaps))
    L = max(len(t) for t, _ in seqs) + 1
    n = len(seqs)
    prev = torch.full((n, L), k, dtype=torch.long)
    tgt = torch.full((n, L), -100, dtype=torch.long)
    lg = torch.zeros(n, L)
    gm = torch.zeros(n, L)
    for i, (t, g) in enumerate(seqs):
        prev[i, 1 : len(t) + 1] = torch.tensor(t)
        tgt[i, : len(t)] = torch.tensor(t)
        tgt[i, len(t)] = k
        lg[i, : len(t)] = torch.log(torch.tensor(g))
        gm[i, 1 : len(t)] = 1
    return prev, tgt, lg, gm


def nll(model, batch):
    prev, tgt, lg, gm = batch
    logits, mu, lsd = model(prev)
    ce = F.cross_entropy(logits.reshape(-1, logits.shape[-1]), tgt.reshape(-1), ignore_index=-100)
    lsd = lsd.clamp(-4, 2)
    gap = (0.5 * ((lg - mu) / lsd.exp()) ** 2 + lsd) * gm
    return ce, gap.sum() / gm.sum().clamp(min=1)


def ngram_ppl(train, val, k, order):
    """Perplexity of add-0.5 unigram / bigram baselines on the same token stream."""
    from collections import Counter

    def stream(b):
        prev, tgt, _, _ = b
        for p, t in zip(prev.tolist(), tgt.tolist()):
            for a, c in zip(p, t):
                if c != -100:
                    yield a, c

    cnt, ctx = Counter(), Counter()
    for a, c in stream(train):
        key = (a, c) if order == 2 else c
        cnt[key] += 1
        ctx[a if order == 2 else 0] += 1
    v = k + 1
    lp, n = 0.0, 0
    for a, c in stream(val):
        key = (a, c) if order == 2 else c
        lp += math.log((cnt[key] + 0.5) / (ctx[a if order == 2 else 0] + 0.5 * v))
        n += 1
    return math.exp(-lp / n)


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("species")
    ap.add_argument("--tokens", type=int, default=32)
    ap.add_argument("--steps", type=int, default=3000)
    a = ap.parse_args()
    torch.manual_seed(0)

    recs = syllables(a.species)
    train_syl = [s for r in recs if not r["val"] for song in r["songs"] for s in song]
    all_syl = [s for r in recs for song in r["songs"] for s in song]
    x = np.stack([features(s) for s in train_syl])
    cent, _ = kmeans(x, a.tokens)
    lab_of = {}
    for s in all_syl:
        lab_of[id(s)] = int(((features(s)[None] - cent) ** 2).sum(-1).argmin())
    k = a.tokens

    # Contour bank: real syllables as exemplars (averaging would blur fast sweeps), plus PCA
    # modes of the within-token spread for small variations around an exemplar.
    rng = np.random.default_rng(0)
    bank = {"f0": np.zeros((k, EXEMPLARS, P)), "loud": np.zeros((k, EXEMPLARS, P)),
            "effort": np.zeros((k, EXEMPLARS, P)), "dur": np.zeros((k, EXEMPLARS)), "count": np.zeros(k),
            "modes": np.zeros((k, MODES, 2 * P)), "coef_sd": np.zeros((k, MODES))}
    for j in range(k):
        mem = [s for s in train_syl if lab_of[id(s)] == j] or [min(all_syl, key=lambda s: ((features(s) - cent[j]) ** 2).sum())]
        dist = np.array([((features(s) - cent[j]) ** 2).sum() for s in mem])
        # Exemplars must be typical and free of f0 tracking glitches (octave jumps).
        smooth = [np.abs(np.diff(m["f0"])).max() < 0.4 for m in mem]
        typical = [m for m, d, ok in zip(mem, dist, smooth) if d <= np.percentile(dist, 80) and ok] or mem
        pick = rng.choice(len(typical), min(EXEMPLARS, len(typical)), replace=False)
        for e, i in enumerate(pick):
            ex = typical[i]
            bank["f0"][j, e], bank["loud"][j, e], bank["effort"][j, e] = ex["f0"], ex["loud"], ex["effort"]
            bank["dur"][j, e] = math.log(ex["dur"])
        bank["count"][j] = len(pick)
        if len(mem) > MODES:
            f0 = np.stack([s["f0"] for s in mem])
            loud = np.stack([s["loud"] for s in mem])
            res = np.concatenate([f0 - f0.mean(0), (loud - loud.mean(0)) / 10.0], 1)
            _, sv, vt = np.linalg.svd(res, full_matrices=False)
            bank["modes"][j] = vt[:MODES]
            bank["coef_sd"][j] = sv[:MODES] / math.sqrt(len(mem))

    tr = sequences([r for r in recs if not r["val"]], lab_of, k)
    va = sequences([r for r in recs if r["val"]], lab_of, k)
    model = SongModel(k)
    opt = torch.optim.AdamW(model.parameters(), lr=3e-3, weight_decay=1e-2)
    best, best_state = float("inf"), None
    for step in range(1, a.steps + 1):
        idx = torch.randint(0, tr[0].shape[0], (64,))
        ce, gap = nll(model, [t[idx] for t in tr])
        loss = ce + 0.2 * gap
        opt.zero_grad()
        loss.backward()
        opt.step()
        if step % 100 == 0:
            model.eval()
            with torch.no_grad():
                vce, vgap = nll(model, va)
            if step % 500 == 0:
                print(f"step {step}: train ce {ce.item():.3f}  val ce {vce.item():.3f} (ppl {math.exp(vce.item()):.2f}) gap nll {vgap.item():.3f}")
            model.train()
            if vce.item() < best:
                best, best_state = vce.item(), {n: v.clone() for n, v in model.state_dict().items()}
    model.load_state_dict(best_state)
    # How repetitive the species is: share of syllables whose token already occurred in the same
    # song. The engine re-sings the earlier rendition with this probability.
    seen = [len(set(song_toks[:i])) == len(set(song_toks[:i + 1]))
            for song_toks in ([lab_of[id(x)] for x in song] for r in recs for song in r["songs"])
            for i in range(1, len(song_toks))]
    reuse = float(np.mean(seen))
    metrics = {
        "reuse": reuse,
        "tokens": k,
        "train_syllables": len(train_syl),
        "train_songs": int(tr[0].shape[0]),
        "val_songs": int(va[0].shape[0]),
        "val_ppl_gru": math.exp(best),
        "val_ppl_unigram": ngram_ppl(tr, va, k, 1),
        "val_ppl_bigram": ngram_ppl(tr, va, k, 2),
        "params": sum(p.numel() for p in model.parameters()),
        "song_len_mean": float(np.mean([len(s) for r in recs for s in r["songs"]])),
    }
    print(json.dumps(metrics, indent=1))
    out = RUNS / a.species
    out.mkdir(parents=True, exist_ok=True)
    torch.save({"model": model.state_dict(), "k": k, "bank": bank, "centroids": cent, "reuse": reuse}, out / "song.pt")
    (out / "song_metrics.json").write_text(json.dumps(metrics, indent=1))


if __name__ == "__main__":
    main()
