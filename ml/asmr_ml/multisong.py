"""Shared song model for all species: syllable tokens + GRU syntax + CVAE contour decoder.

    uv run python -m asmr_ml.multisong base chaffinch robin song_thrush nightingale
    uv run python -m asmr_ml.multisong adapt blackbird [--max-recordings 20]

- Tokens: one k-means vocabulary (K) over syllable contours pooled from the base species.
- Syntax: GRU over [token embedding, species embedding] -> next token / end + log gap; plus a
  per-species logit bias (which tokens a species uses at all).
- Contours: conditional VAE. The decoder maps [token embedding, species embedding, z] to a
  syllable (f0 / loudness / effort at P points + log duration); at run time z ~ N(0, variation).
  This replaces per-species exemplar banks, so a species is only a few hundred floats.

Adapter (per species) = song embedding (D_S) + logit bias (K + 1) + contour embedding (D_C).
Like the voice, all conditioning is linear and folds into biases in the engine.
"""

import argparse
import json
import math

import numpy as np
import torch
import torch.nn.functional as F
from torch import nn

from . import song as single
from .dsp import HOP_S
from .train_voice import RUNS

BASE = RUNS / "base"
K = 64
P = 16
D_S = 8  # species embedding for the syntax GRU
D_C = 8  # species embedding for the contour decoder
D_Z = 4
LEAK = 0.1
C_DIM = 3 * P + 1


def contour_vec(s) -> np.ndarray:
    r = lambda v: single.resample(v, P)  # noqa: E731
    return np.concatenate([r(s["f0"]) - 12.0, r(s["loud"]) / 20.0, r(s["effort"]), [math.log(s["dur"])]]).astype(np.float32)


def token_features(s) -> np.ndarray:
    f = single.resample(s["f0"], P)
    loud = single.resample(s["loud"], P)
    return np.concatenate([f * 4.0, (loud - loud.max()) / 10.0, [2.0 * math.log(s["dur"])]])


class SongNet(nn.Module):
    def __init__(self, n_species: int, k: int = K, embed: int = 16, hidden: int = 48):
        super().__init__()
        self.k = k
        self.emb = nn.Embedding(k + 1, embed)  # index k = BOS
        self.sp = nn.Embedding(n_species, D_S)
        self.bias = nn.Parameter(torch.zeros(n_species, k + 1))
        self.gru = nn.GRU(embed + D_S, hidden, batch_first=True)
        self.drop = nn.Dropout(0.25)
        self.out = nn.Linear(hidden, k + 3)

    def forward(self, prev, sp):
        e = torch.cat([self.drop(self.emb(prev)), self.sp(sp)[:, None].expand(-1, prev.shape[1], -1)], -1)
        g, _ = self.gru(e)
        y = self.out(self.drop(g))
        return y[..., : self.k + 1] + self.bias[sp][:, None], y[..., self.k + 1], y[..., self.k + 2]


class ContourVAE(nn.Module):
    def __init__(self, n_species: int, k: int = K, embed: int = 16, hidden: int = 128):
        super().__init__()
        self.tok = nn.Embedding(k, embed)
        self.sp = nn.Embedding(n_species, D_C)
        self.enc1 = nn.Linear(C_DIM + embed + D_C, hidden)
        self.enc2 = nn.Linear(hidden, 2 * D_Z)
        self.dec1 = nn.Linear(embed + D_C + D_Z, hidden)
        self.dec2 = nn.Linear(hidden, hidden)
        self.dec3 = nn.Linear(hidden, C_DIM)

    def decode(self, tok, sp, z):
        h = F.leaky_relu(self.dec1(torch.cat([self.tok(tok), self.sp(sp), z], -1)), LEAK)
        h = F.leaky_relu(self.dec2(h), LEAK)
        return self.dec3(h)

    def forward(self, c, tok, sp):
        h = F.leaky_relu(self.enc1(torch.cat([c, self.tok(tok), self.sp(sp)], -1)), LEAK)
        mu, logvar = self.enc2(h).chunk(2, -1)
        z = mu + torch.randn_like(mu) * (0.5 * logvar).exp()
        return self.decode(tok, sp, z), mu, logvar


# f0 errors matter most; duration next; loudness / effort less.
C_WEIGHT = torch.tensor([4.0] * P + [1.0] * P + [0.5] * P + [2.0])


def vae_loss(model, c, tok, sp, beta=0.05):
    rec, mu, logvar = model(c, tok, sp)
    r = (((rec - c) ** 2) * C_WEIGHT).mean()
    kl = (-0.5 * (1 + logvar - mu**2 - logvar.exp())).sum(-1).mean()
    return r + beta * kl, r


def collect(species: list[str], cent: np.ndarray, max_recordings=None):
    """Per species: train / val songs as (tokens, gaps, contour vectors)."""
    out = []
    for key in species:
        recs = single.syllables(key)
        if max_recordings:
            tr = [r for r in recs if not r["val"]][:max_recordings]
            recs = tr + [r for r in recs if r["val"]]
        sets = {"train": [], "val": []}
        for r in recs:
            for song in r["songs"]:
                feats = np.stack([token_features(s) for s in song])
                toks = ((feats[:, None] - cent[None]) ** 2).sum(-1).argmin(1).tolist()
                gaps = [HOP_S] + [max(b["start"] - a["end"], 1) * HOP_S for a, b in zip(song[:-1], song[1:])]
                sets["val" if r["val"] else "train"].append((toks, gaps, np.stack([contour_vec(s) for s in song])))
        out.append(sets)
    return out


def pad(songs, sp_ids, k):
    L = max(len(t) for t, _, _ in songs) + 1
    n = len(songs)
    prev = torch.full((n, L), k, dtype=torch.long)
    tgt = torch.full((n, L), -100, dtype=torch.long)
    lg, gm = torch.zeros(n, L), torch.zeros(n, L)
    for i, (t, g, _) in enumerate(songs):
        prev[i, 1:len(t) + 1] = torch.tensor(t)
        tgt[i, :len(t)] = torch.tensor(t)
        tgt[i, len(t)] = k
        lg[i, :len(t)] = torch.log(torch.tensor(g))
        gm[i, 1:len(t)] = 1
    return prev, tgt, lg, gm, torch.tensor(sp_ids)


def song_nll(model, b):
    prev, tgt, lg, gm, sp = b
    logits, mu, lsd = model(prev, sp)
    ce = F.cross_entropy(logits.reshape(-1, logits.shape[-1]), tgt.reshape(-1), ignore_index=-100)
    lsd = lsd.clamp(-4, 2)
    gap = ((0.5 * ((lg - mu) / lsd.exp()) ** 2 + lsd) * gm).sum() / gm.sum().clamp(min=1)
    return ce, gap


def flat(sets, split, sp_offset=0):
    c, t, s = [], [], []
    for i, d in enumerate(sets):
        for toks, _, cv in d[split]:
            c.append(cv)
            t += toks
            s += [i + sp_offset] * len(toks)
    return torch.from_numpy(np.concatenate(c)), torch.tensor(t), torch.tensor(s)


def fit(song, vae, params, sets, steps, sp_offset=0, lr=3e-3, log=True):
    tr_songs = [x for d in sets for x in d["train"]]
    tr_ids = [i + sp_offset for i, d in enumerate(sets) for _ in d["train"]]
    va_songs = [x for d in sets for x in d["val"]]
    va_ids = [i + sp_offset for i, d in enumerate(sets) for _ in d["val"]]
    tr, va = pad(tr_songs, tr_ids, K), pad(va_songs, va_ids, K)
    ctr, cva = flat(sets, "train", sp_offset), flat(sets, "val", sp_offset)
    opt = torch.optim.AdamW(params, lr=lr, weight_decay=1e-2)
    best, best_state = float("inf"), None
    for step in range(1, steps + 1):
        idx = torch.randint(0, tr[0].shape[0], (64,))
        ce, gap = song_nll(song, [t[idx] for t in tr])
        j = torch.randint(0, ctr[0].shape[0], (256,))
        vl, _ = vae_loss(vae, ctr[0][j], ctr[1][j], ctr[2][j])
        loss = ce + 0.2 * gap + vl
        opt.zero_grad()
        loss.backward()
        opt.step()
        if step % 100 == 0:
            song.eval()
            vae.eval()
            with torch.no_grad():
                vce, _ = song_nll(song, va)
                _, vrec = vae_loss(vae, *cva)
            song.train()
            vae.train()
            score = vce.item() + vrec.item()
            if score < best:
                best = score
                best_state = ({k: v.clone() for k, v in song.state_dict().items()},
                              {k: v.clone() for k, v in vae.state_dict().items()}, vce.item(), vrec.item())
            if log and step % 500 == 0:
                print(f"step {step}: val ce {vce.item():.3f} (ppl {math.exp(vce.item()):.2f}) contour mse {vrec.item():.4f}", flush=True)
    song.load_state_dict(best_state[0])
    vae.load_state_dict(best_state[1])
    return best_state[2], best_state[3], va


@torch.no_grad()
def per_species_ppl(song, va, n_species, sp_offset=0):
    out = []
    for s in range(sp_offset, sp_offset + n_species):
        m = va[4] == s
        if m.any():
            ce, _ = song_nll(song.eval(), [t[m] for t in va])
            out.append(math.exp(ce.item()))
    return out


def cmd_base(a):
    torch.manual_seed(0)
    rng = np.random.default_rng(0)
    pooled = []
    for key in a.species:
        syl = [s for r in single.syllables(key) if not r["val"] for song in r["songs"] for s in song]
        pick = rng.choice(len(syl), min(6000, len(syl)), replace=False)
        pooled += [token_features(syl[i]) for i in pick]
    cent, _ = single.kmeans(np.stack(pooled), K)
    sets = collect(a.species, cent)
    song, vae = SongNet(len(a.species)), ContourVAE(len(a.species))
    vce, vrec, va = fit(song, vae, list(song.parameters()) + list(vae.parameters()), sets, a.steps)
    ppl = per_species_ppl(song, va, len(a.species))
    BASE.mkdir(parents=True, exist_ok=True)
    torch.save({"song": song.state_dict(), "vae": vae.state_dict(), "centroids": cent, "species": a.species}, BASE / "song.pt")
    m = {"species": a.species, "tokens": K, "val_ppl": dict(zip(a.species, [round(p, 2) for p in ppl])),
         "val_contour_mse": round(vrec, 4),
         "shared_params": sum(p.numel() for n, p in list(song.named_parameters()) + list(vae.named_parameters())
                              if not n.startswith(("sp.", "bias")))}
    (BASE / "song_metrics.json").write_text(json.dumps(m, indent=1))
    print(json.dumps(m))


def load_base(extra: int = 0):
    ck = torch.load(BASE / "song.pt", map_location="cpu", weights_only=False)
    n = len(ck["species"])
    song, vae = SongNet(n + extra), ContourVAE(n + extra)
    for model, state in ((song, ck["song"]), (vae, ck["vae"])):
        own = model.state_dict()
        for k_, v in state.items():
            if k_ in ("sp.weight", "bias"):
                own[k_][:n] = v
            else:
                own[k_] = v
        model.load_state_dict(own)
    return song, vae, ck


def cmd_adapt(a):
    torch.manual_seed(0)
    song, vae, ck = load_base(extra=1)
    new = len(ck["species"])
    with torch.no_grad():
        song.sp.weight[new] = song.sp.weight[:new].mean(0)
        song.bias[new] = song.bias[:new].mean(0)
        vae.sp.weight[new] = vae.sp.weight[:new].mean(0)
    for p in list(song.parameters()) + list(vae.parameters()):
        p.requires_grad_(False)
    params = [song.sp.weight, song.bias, vae.sp.weight]
    for p in params:
        p.requires_grad_(True)
    sets = collect([a.species], ck["centroids"], a.max_recordings)
    vce, vrec, va = fit(song, vae, params, sets, a.steps, sp_offset=new, lr=1e-2)
    out = RUNS / a.species
    out.mkdir(parents=True, exist_ok=True)
    tag = "song_adapter" + (f"_n{a.max_recordings}" if a.max_recordings else "")
    torch.save({"species": a.species, "song_emb": song.sp.weight[new].detach(), "bias": song.bias[new].detach(),
                "contour_emb": vae.sp.weight[new].detach()}, out / f"{tag}.pt")
    m = {"species": a.species, "val_ppl": round(math.exp(vce), 2), "val_contour_mse": round(vrec, 4),
         "adapter_floats": D_S + K + 1 + D_C, "train_songs": len(sets[0]["train"])}
    (out / f"{tag}_metrics.json").write_text(json.dumps(m, indent=1))
    print(json.dumps(m))


def main() -> None:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("base")
    b.add_argument("species", nargs="+")
    b.add_argument("--steps", type=int, default=6000)
    d = sub.add_parser("adapt")
    d.add_argument("species")
    d.add_argument("--steps", type=int, default=1500)
    d.add_argument("--max-recordings", type=int, default=None)
    a = ap.parse_args()
    {"base": cmd_base, "adapt": cmd_adapt}[a.cmd](a)


if __name__ == "__main__":
    main()
