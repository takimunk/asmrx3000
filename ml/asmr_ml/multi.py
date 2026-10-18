"""Shared multi-species DDSP voice + cheap per-species adapters.

    uv run python -m asmr_ml.multi base chaffinch robin song_thrush nightingale --steps 20000
    uv run python -m asmr_ml.multi adapt blackbird --steps 1500 [--max-recordings 20]

Conditioning is linear on purpose: the species embedding e enters `inp` and `out1` by
concatenation, and the adapter LoRA (B A, rank r) adds to `out1` / `out2`. So for a fixed species
everything folds into ordinary weights and biases
    inp([x, e]) = W_x x + (b + W_e e),    out (W + B A) ...
and the engine runs exactly the same tiny network as a per-species model (core/src/neural/
compose.rs does the folding when it loads base + adapter).

Adapter = embedding (D floats) + input normalisation (4) + LoRA on out1 / out2 (~1.6 k floats),
a few KB in f16. Base species get an embedding-only adapter.
"""

import argparse
import json
import math
import time

import numpy as np
import torch
import torch.nn.functional as F
from torch import nn

from .dsp import HOP, N_BANDS, N_HARM, SR, exp_sigmoid, harmonic, noise_bank, upsample
from .species import SPECIES
from .train_voice import RUNS, Data, device
from .voice import LEAK, NOISE_BIAS, mr_stft_loss

BASE = RUNS / "base"
D_EMB = 16


class LoRA(nn.Module):
    """Per-species low-rank deltas for one Linear: y += B_s (A_s x). B starts at zero."""

    def __init__(self, n_species: int, n_in: int, n_out: int, rank: int):
        super().__init__()
        self.A = nn.Parameter(torch.randn(n_species, rank, n_in) / math.sqrt(n_in))
        self.B = nn.Parameter(torch.zeros(n_species, n_out, rank))

    def forward(self, x, sp):  # x [B, T, in], sp [B]
        return torch.einsum("bor,bri,bti->bto", self.B[sp], self.A[sp], x)


class MultiVoice(nn.Module):
    def __init__(self, n_species: int, d_emb: int = D_EMB, embed: int = 64, hidden: int = 96, mlp: int = 128,
                 rank: int = 0):
        super().__init__()
        self.n_harm, self.n_bands, self.rank = N_HARM, N_BANDS, rank
        self.emb = nn.Embedding(n_species, d_emb)
        nn.init.normal_(self.emb.weight, std=0.3)
        self.inp = nn.Linear(3 + d_emb, embed)
        self.gru = nn.GRU(embed, hidden, batch_first=True)
        self.out1 = nn.Linear(hidden + 3 + d_emb, mlp)
        self.out2 = nn.Linear(mlp, 1 + N_HARM + N_BANDS)
        self.register_buffer("norm", torch.tensor([[12.0, 0.5, -30.0, 15.0]]).repeat(n_species, 1))
        if rank:
            self.lora1 = LoRA(n_species, hidden + 3 + d_emb, mlp, rank)
            self.lora2 = LoRA(n_species, mlp, 1 + N_HARM + N_BANDS, rank)

    def forward(self, f0, loud, effort, sp):
        n = self.norm[sp]  # [B, 4]
        x = torch.stack([(torch.log2(f0) - n[:, None, 0]) / n[:, None, 1],
                         (loud - n[:, None, 2]) / n[:, None, 3], effort], -1)
        e = self.emb(sp)[:, None, :].expand(-1, x.shape[1], -1)
        z = F.leaky_relu(self.inp(torch.cat([x, e], -1)), LEAK)
        g, _ = self.gru(z)
        c = torch.cat([g, x, e], -1)
        h = self.out1(c)
        if self.rank:
            h = h + self.lora1(c, sp)
        h = F.leaky_relu(h, LEAK)
        y = self.out2(h)
        if self.rank:
            y = y + self.lora2(h, sp)
        amp = exp_sigmoid(y[..., :1])
        k = torch.arange(1, self.n_harm + 1, device=f0.device, dtype=f0.dtype)
        logits = y[..., 1:1 + self.n_harm].masked_fill(~(f0[..., None] * k < 0.45 * SR), -1e4)
        harm = amp * torch.softmax(logits, -1)
        noise = exp_sigmoid(y[..., 1 + self.n_harm:] - NOISE_BIAS)
        return harm, noise

    def synth(self, f0, loud, effort, sp, bank, offsets):
        harm, noise = self(f0, loud, effort, sp)
        n = f0.shape[1] * HOP
        idx = offsets[:, None] + torch.arange(n, device=f0.device)[None]
        bands = bank[:, idx].permute(1, 2, 0)
        return harmonic(f0, harm) + (upsample(noise) * bands).sum(-1)


class MultiData:
    def __init__(self, species: list[str], frames: int, max_recordings: int | None = None):
        self.species = species
        self.sets = []
        for k in species:
            d = Data(k, frames)
            d.species = SPECIES[k]
            if max_recordings:
                d.train = d.train[:max_recordings]
            self.sets.append(d)

    def batch(self, rng, n, split="train"):
        out = [[], [], [], [], []]
        for _ in range(n):
            s = int(rng.integers(len(self.sets)))
            f0, loud, eff, a = self.sets[s].batch(rng, 1, split)
            for o, v in zip(out, (f0[0], loud[0], eff[0], a[0], torch.tensor(s))):
                o.append(v)
        return [torch.stack(o) for o in out]


@torch.no_grad()
def evaluate(model, data: MultiData, bank, dev, n_windows: int = 96) -> dict:
    """Per-species held-out MR-STFT on the same windows as train_voice.evaluate."""
    model.eval()
    res = {}
    for s, d in enumerate(data.sets):
        f0, loud, eff, audio = (t.to(dev) for t in d.batch(np.random.default_rng(1234), n_windows, "val"))
        sp = torch.full((n_windows,), s, dtype=torch.long, device=dev)
        offs = torch.zeros(n_windows, dtype=torch.long, device=dev)
        y = model.synth(f0, loud, eff, sp, bank, offs)
        res[data.species[s]] = round(float(mr_stft_loss(y, audio)), 4)
    model.train()
    return res


def train(model, data, params, steps, lr, dev, out_path, log_every=200, eval_every=2000, batch=32, eval_fn=None):
    eval_fn = eval_fn or (lambda m, b: evaluate(m, data, b, dev))
    bank = torch.from_numpy(noise_bank(20.0, seed=0)).to(dev)
    opt = torch.optim.Adam(params, lr=lr)
    sched = torch.optim.lr_scheduler.LambdaLR(opt, lambda s: 0.05 + 0.95 * 0.5 * (1 + math.cos(math.pi * min(s / steps, 1))))
    rng = np.random.default_rng(0)
    best, hist, t0 = float("inf"), [], time.time()
    for step in range(1, steps + 1):
        f0, loud, eff, audio, sp = (t.to(dev) for t in data.batch(rng, batch))
        offs = torch.from_numpy(rng.integers(0, bank.shape[1] - audio.shape[1], batch)).to(dev)
        loss = mr_stft_loss(model.synth(f0, loud, eff, sp, bank, offs), audio)
        opt.zero_grad()
        loss.backward()
        torch.nn.utils.clip_grad_norm_(params, 2.0)
        opt.step()
        sched.step()
        if step % log_every == 0:
            print(f"step {step:6d} loss {loss.item():.4f} ({(time.time() - t0) / step * 1000:.0f} ms/step)", flush=True)
        if step % eval_every == 0 or step == steps:
            m = eval_fn(model, bank)
            mean = float(np.mean(list(m.values())))
            hist.append({"step": step, "mean": mean, **m})
            print("  val", hist[-1], flush=True)
            if mean < best:
                best = mean
                torch.save(model.state_dict(), out_path)
    model.load_state_dict(torch.load(out_path))
    return hist, (time.time() - t0) / 60


def set_norms(model, data: MultiData, offset: int = 0):
    for s, d in enumerate(data.sets):
        model.norm[offset + s] = torch.tensor(d.stats())


def cmd_base(a):
    dev = device()
    torch.manual_seed(0)
    data = MultiData(a.species, 200)
    model = MultiVoice(len(a.species)).to(dev)
    set_norms(model, data)
    BASE.mkdir(parents=True, exist_ok=True)
    n_params = sum(p.numel() for n, p in model.named_parameters() if n != "emb.weight")
    print(f"base on {a.species}: {n_params} shared params, {[len(d.train) for d in data.sets]} train recordings")
    hist, minutes = train(model, data, list(model.parameters()), a.steps, a.lr, dev, BASE / "voice.pt")
    torch.save({"model": model.state_dict(), "species": a.species}, BASE / "voice.pt")
    (BASE / "voice_metrics.json").write_text(json.dumps(
        {"species": a.species, "shared_params": n_params, "steps": a.steps, "minutes": round(minutes, 1), "history": hist}, indent=1))


def load_base(rank: int = 0, extra: int = 0):
    ck = torch.load(BASE / "voice.pt", map_location="cpu")
    state, species = ck["model"], ck["species"]
    model = MultiVoice(len(species) + extra, rank=rank)
    own = model.state_dict()
    for k, v in state.items():
        if k in ("emb.weight", "norm"):
            own[k][: len(species)] = v
        else:
            own[k] = v
    model.load_state_dict(own)
    return model, species


def cmd_adapt(a):
    """New species: frozen base, train embedding (+ LoRA) only."""
    dev = device()
    torch.manual_seed(0)
    model, base_species = load_base(rank=a.rank, extra=1)
    new = len(base_species)
    with torch.no_grad():
        model.emb.weight[new] = model.emb.weight[:new].mean(0)
    data = MultiData([a.species], 200, a.max_recordings)
    set_norms(model, data, offset=new)
    model = model.to(dev)
    for p in model.parameters():
        p.requires_grad_(False)
    params = [model.emb.weight]
    if a.rank:
        params += [model.lora1.A, model.lora1.B, model.lora2.A, model.lora2.B]
    for p in params:
        p.requires_grad_(True)

    # The data batch uses species index 0 (its only species); remap to the new slot.
    class Remap:
        sets, species = data.sets, data.species

        def batch(self, rng, n, split="train"):
            b = data.batch(rng, n, split)
            b[4] = torch.full_like(b[4], new)
            return b

    out = RUNS / a.species
    out.mkdir(parents=True, exist_ok=True)
    tag = f"adapter_r{a.rank}" + (f"_n{a.max_recordings}" if a.max_recordings else "")
    hist, minutes = train(model, Remap(), params, a.steps, a.lr, dev, out / f"{tag}.pt", eval_every=500,
                          eval_fn=lambda m, bank: evaluate(_Shift(m, new), data, bank, dev))
    n_adapter = D_EMB + 4 + (sum(p[new].numel() for p in params[1:]) if a.rank else 0)
    torch.save({"species": a.species, "slot": new, "rank": a.rank, "emb": model.emb.weight[new].detach().cpu(),
                "norm": model.norm[new].cpu(),
                "lora": {n: p[new].detach().cpu() for n, p in
                         [("lora1.A", model.lora1.A), ("lora1.B", model.lora1.B), ("lora2.A", model.lora2.A),
                          ("lora2.B", model.lora2.B)]} if a.rank else {}}, out / f"{tag}.pt")
    m = {"species": a.species, "rank": a.rank, "adapter_floats": n_adapter, "train_recordings": len(data.sets[0].train),
         "steps": a.steps, "minutes": round(minutes, 1), "best_val_mr_stft": min(h["mean"] for h in hist), "history": hist}
    (out / f"{tag}_metrics.json").write_text(json.dumps(m, indent=1))
    print(json.dumps({k: v for k, v in m.items() if k != "history"}))


class _Shift(nn.Module):
    """View of a MultiVoice that maps species 0 to slot `new` (for evaluating an adapter)."""

    def __init__(self, model, new):
        super().__init__()
        self.m, self.new = model, new

    def synth(self, f0, loud, eff, sp, bank, offs):
        return self.m.synth(f0, loud, eff, torch.full_like(sp, self.new), bank, offs)

    def train(self, mode=True):
        self.m.train(mode)
        return self


def main() -> None:
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    b = sub.add_parser("base")
    b.add_argument("species", nargs="+")
    b.add_argument("--steps", type=int, default=20000)
    b.add_argument("--lr", type=float, default=1e-3)
    d = sub.add_parser("adapt")
    d.add_argument("species")
    d.add_argument("--steps", type=int, default=1500)
    d.add_argument("--lr", type=float, default=3e-3)
    d.add_argument("--rank", type=int, default=4)
    d.add_argument("--max-recordings", type=int, default=None)
    a = ap.parse_args()
    {"base": cmd_base, "adapt": cmd_adapt}[a.cmd](a)


if __name__ == "__main__":
    main()
