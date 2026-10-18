"""Export the shared bird base and per-species adapters for asmr-core.

    uv run python -m asmr_ml.export_multi                 # base + all base species
    uv run python -m asmr_ml.export_multi --adapter blackbird

core/assets/neural/birds_base.asnw   shared weights (f16), configuration (f32), cfg.base_id
core/assets/neural/<species>.asna    adapter (f32): embeddings, normalisation, LoRA, constants
core/assets/neural/parity/<sp>.asnw  reference outputs for the Rust tests (composed == PyTorch)

Rust composes base + adapter by folding the embeddings into biases (core/src/neural/compose.rs).
"""

import argparse
import copy
import hashlib

import numpy as np
import torch

from . import multi, multisong
from .dsp import HOP_S, N_HARM, band_centers, band_q
from .export import ASSETS, data_stats, write_asnw
from .voice import LEAK, NOISE_BIAS

SKIP_VOICE = ("emb.weight", "norm", "lora1.A", "lora1.B", "lora2.A", "lora2.B")


def f32(x):
    return (np.asarray(x, np.float32).reshape(np.shape(x) or (1,)), 0)


def base_tensors():
    voice, species = multi.load_base()
    song, vae, ck = multisong.load_base()
    t = {
        "cfg.leak": f32(LEAK), "cfg.noise_bias": f32(NOISE_BIAS), "cfg.n_harm": f32(N_HARM), "cfg.hop_s": f32(HOP_S),
        "cfg.band_q": f32(band_q()), "cfg.band_centers": f32(band_centers()),
    }
    for n, p in voice.state_dict().items():
        if n not in SKIP_VOICE:
            t[f"voice.{n}"] = (p.numpy(), 1)
    for n, p in song.state_dict().items():
        if not n.startswith(("sp.", "bias")):
            t[f"song.{n}"] = (p.numpy(), 1)
    for n, p in vae.state_dict().items():
        if n.startswith(("tok.", "dec")):
            t[f"contour.{n}"] = (p.numpy(), 1)
    h = hashlib.sha256(b"".join(v[0].astype("<f2" if v[1] else "<f4").tobytes() for v in t.values())).digest()
    base_id = int.from_bytes(h[:3], "little")  # < 2^24: exact in f32
    t["cfg.base_id"] = f32(base_id)
    return t, base_id, voice, species, song, vae, ck


def reuse_of(key: str, cent) -> float:
    sets = multisong.collect([key], cent)[0]
    seen = [t[i] in t[:i] for t, _, _ in sets["train"] + sets["val"] for i in range(1, len(t))]
    return float(np.mean(seen))


def adapter_tensors(key, base_id, cent, voice_emb, norm, lora, song_emb, song_bias, contour_emb, out_gain):
    silence_db, silence_effort, interval = data_stats(key)
    t = {
        "cfg.base_id": f32(base_id), "cfg.voice_norm": f32(norm), "cfg.silence_db": f32(silence_db),
        "cfg.silence_effort": f32(silence_effort), "cfg.out_gain": f32(out_gain), "cfg.song_interval_s": f32(interval),
        "cfg.reuse": f32(reuse_of(key, cent)),
        "adapter.voice.emb": f32(voice_emb), "adapter.song.emb": f32(song_emb),
        "adapter.song.bias": f32(song_bias), "adapter.contour.emb": f32(contour_emb),
    }
    for n, p in lora.items():
        t[f"adapter.voice.{n}"] = f32(p)
    return t


def rounded(model):
    """Copy of a model with every weight rounded through f16 (what the engine loads)."""
    m = copy.deepcopy(model)
    m.load_state_dict({k: (v.half().float() if v.is_floating_point() else v) for k, v in model.state_dict().items()})
    return m


def write_parity(key, voice, slot, song, vae, sp_slot):
    from . import prep

    d = np.load(sorted((prep.CLEAN / key).glob("*.npz"))[0])
    a = int(d["syl"][0, 0])
    sl = slice(a, a + 120)
    f0 = torch.from_numpy(d["f0"][sl])[None]
    loud = torch.from_numpy(np.maximum(d["loud"][sl], -100))[None]
    eff = torch.from_numpy(d["effort"][sl])[None]
    sp = torch.tensor([slot])
    toks = torch.tensor([0, 5, 17, 33])
    z = torch.tensor([[0.0, 0.0, 0.0, 0.0], [1.0, -0.5, 0.3, 2.0], [-1.2, 0.7, 0.0, -0.4], [0.5, 0.5, -1.5, 1.0]])
    with torch.no_grad():
        harm, noise = voice(f0, loud, eff, sp)
        cont = vae.decode(toks, torch.full_like(toks, sp_slot), z)
    write_asnw(ASSETS / "parity" / f"{key}.asnw", {
        "f0": (f0[0].numpy(), 0), "loud": (loud[0].numpy(), 0), "effort": (eff[0].numpy(), 0),
        "harm": (harm[0].numpy(), 0), "noise": (noise[0].numpy(), 0),
        "contour.tok": (toks.float().numpy(), 0), "contour.z": (z.numpy(), 0), "contour.out": (cont.numpy(), 0),
    })


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--adapter", nargs="*", default=[], help="adapted (non-base) species to export")
    ap.add_argument("--voice-tag", default="adapter_r4")
    ap.add_argument("--out-gain", type=float, default=0.6)
    a = ap.parse_args()

    t, base_id, voice, species, song, vae, ck = base_tensors()
    n = write_asnw(ASSETS / "birds_base.asnw", t)
    shared = sum(v[0].size for k, v in t.items() if not k.startswith("cfg."))
    print(f"birds_base.asnw: {n / 1024:.0f} KiB, {shared} shared parameters, base_id {base_id}")
    cent = ck["centroids"]
    for i, key in enumerate(species):
        at = adapter_tensors(key, base_id, cent, voice.emb.weight[i].detach(), voice.norm[i], {},
                             song.sp.weight[i].detach(), song.bias[i].detach(), vae.sp.weight[i].detach(), a.out_gain)
        n = write_asnw(ASSETS / f"{key}.asna", at)
        print(f"{key}.asna: {n} bytes")
    rv, rs, rvae = rounded(voice), rounded(song), rounded(vae)
    write_parity(species[0], rv, 0, rs, rvae, 0)

    for key in a.adapter:
        va = torch.load(multi.RUNS / key / f"{a.voice_tag}.pt", map_location="cpu")
        sa = torch.load(multi.RUNS / key / "song_adapter.pt", map_location="cpu")
        lora = {n: v.numpy() for n, v in va["lora"].items()}
        at = adapter_tensors(key, base_id, cent, va["emb"], va["norm"], lora, sa["song_emb"], sa["bias"],
                             sa["contour_emb"], a.out_gain)
        n = write_asnw(ASSETS / f"{key}.asna", at)
        print(f"{key}.asna: {n} bytes (rank {va['rank']})")
        # Parity for an adapter with LoRA: put it in an extra slot of rounded copies.
        pv, _ = multi.load_base(rank=max(va["rank"], 1), extra=1)
        pv = rounded(pv)
        slot = len(species)
        with torch.no_grad():
            pv.emb.weight[slot] = va["emb"]
            pv.norm[slot] = va["norm"]
            for n_, v in va["lora"].items():
                mod, par = n_.split(".")
                getattr(getattr(pv, mod), par)[slot] = v
        ps, pvae, _ = multisong.load_base(extra=1)
        pvae = rounded(pvae)
        with torch.no_grad():
            pvae.sp.weight[slot] = sa["contour_emb"]
        write_parity(key, pv.eval(), slot, ps, pvae.eval(), slot)


if __name__ == "__main__":
    main()
