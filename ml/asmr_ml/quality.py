"""Perch-based quality report for one species (real vs resynthesis vs generated vs other species).

    uv run python -m asmr_ml.quality chaffinch [--generated-seconds 120]

Writes WAV sets to out/quality/<species>/ and runs perch_eval (separate TensorFlow env):
  real        held-out cleaned recordings (never trained on)
  resynth     shared base voice + species adapter driven by controls extracted from `real`
  generated   Rust engine, built-in species (song model + contour decoder + voice)
  other       held-out real recordings of the other species (how far "a different bird" is)
"""

import argparse
import json
import subprocess
from pathlib import Path

import numpy as np
import soundfile as sf
import torch

from . import multi, prep
from .dsp import HOP, SR, noise_bank
from .species import SPECIES

ML = Path(__file__).resolve().parents[1]
OUT = ML / "out" / "quality"


def val_files(key: str, val_every: int = 6) -> list[Path]:
    files = sorted((prep.CLEAN / key).glob("*.npz"))
    return [p for i, p in enumerate(files) if i % val_every == val_every - 1]


def voice_for(key: str):
    """(model, slot) for a base species or an adapted one (runs/<key>/adapter_r4.pt)."""
    model, species = multi.load_base(rank=4, extra=1)
    if key in species:
        return model.eval(), species.index(key)
    ad = torch.load(multi.RUNS / key / "adapter_r4.pt", map_location="cpu")
    slot = len(species)
    with torch.no_grad():
        model.emb.weight[slot] = ad["emb"]
        model.norm[slot] = ad["norm"]
        for n, v in ad["lora"].items():
            mod, par = n.split(".")
            getattr(getattr(model, mod), par)[slot] = v
    return model.eval(), slot


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("species")
    ap.add_argument("--generated-seconds", type=int, default=120)
    ap.add_argument("--max-seconds", type=float, default=30.0, help="per held-out recording")
    a = ap.parse_args()
    out = OUT / a.species
    for d in ("real", "resynth", "generated", "other"):
        (out / d).mkdir(parents=True, exist_ok=True)
        for f in (out / d).glob("*.wav"):
            f.unlink()

    model, slot = voice_for(a.species)
    bank = torch.from_numpy(noise_bank(a.max_seconds + 1, seed=9))
    T = int(a.max_seconds * SR / HOP)
    for p in val_files(a.species):
        d = np.load(p)
        s = max(0, int(d["syl"][0, 0]) - 40)
        sl = slice(s, s + T)
        sf.write(out / "real" / f"{p.stem}.wav", d["audio"][s * HOP:(s + T) * HOP], SR)
        f0 = torch.from_numpy(d["f0"][sl])[None]
        loud = torch.from_numpy(np.maximum(d["loud"][sl], -100))[None]
        eff = torch.from_numpy(d["effort"][sl])[None]
        with torch.no_grad():
            y = model.synth(f0, loud, eff, torch.tensor([slot]), bank, torch.zeros(1, dtype=torch.long))
        sf.write(out / "resynth" / f"{p.stem}.wav", y[0].numpy(), SR)
    for other in SPECIES:
        if other != a.species and (prep.CLEAN / other).exists():
            for p in val_files(other)[:2]:
                d = np.load(p)
                s = max(0, int(d["syl"][0, 0]) - 40)
                sf.write(out / "other" / f"{other}_{p.stem}.wav", d["audio"][s * HOP:(s + T) * HOP], SR)
    for i in range(a.generated_seconds // 30):
        subprocess.run(["cargo", "run", "--release", "-q", "--manifest-path", str(ML.parent / "core" / "Cargo.toml"),
                        "--example", "neural_bird", "--", a.species, "30", str(out / "generated" / f"g{i}.wav"), "1", str(100 + i)],
                       check=True, capture_output=True)
    r = subprocess.run(["uv", "run", "--project", str(ML / "perch_eval"), "python", str(ML / "perch_eval" / "perch_eval.py"),
                        "--latin", SPECIES[a.species].latin, "--out", str(out / "perch.json")]
                       + [f"--set={n}={out / n}" for n in ("real", "resynth", "generated", "other")],
                       capture_output=True, text=True)
    if r.returncode:
        raise SystemExit(r.stderr[-2000:])
    print(json.dumps(json.loads((out / "perch.json").read_text()), indent=1))


if __name__ == "__main__":
    main()
