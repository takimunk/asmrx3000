"""Listening examples for one species (ml/out/listen/<species>/):

  1_real.wav        held-out recording (cleaned: high-pass + spectral gate), never trained on
  2_resynth.wav     DDSP voice driven by f0 / loudness / effort extracted from 1_real
                    (same controls, the network supplies the timbre)
  3_generated.wav   rendered by the Rust engine: song model + contour bank + voice, new phrases
  sheet.png         spectrograms of the three

    uv run python -m asmr_ml.listen chaffinch
"""

import argparse
import subprocess
from pathlib import Path

import numpy as np
import soundfile as sf
import torch

from . import prep
from .dsp import HOP, SR, noise_bank
from .plots import sheet
from .train_voice import RUNS
from .voice import Voice

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "ml" / "out" / "listen"


def held_out(key: str, val_every: int = 6) -> Path:
    files = sorted((prep.CLEAN / key).glob("*.npz"))
    val = [p for i, p in enumerate(files) if i % val_every == val_every - 1]
    # The held-out recording with the most syllables per second (densest song).
    return max(val, key=lambda p: len(np.load(p)["syl"]) / len(np.load(p)["loud"]))


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("species")
    ap.add_argument("--seconds", type=float, default=20.0)
    a = ap.parse_args()
    out = OUT / a.species
    out.mkdir(parents=True, exist_ok=True)

    d = np.load(held_out(a.species))
    start = max(0, int(d["syl"][0, 0]) - 40)
    T = int(a.seconds * SR / HOP)
    sl = slice(start, start + T)
    real = d["audio"][start * HOP:(start + T) * HOP]
    sf.write(out / "1_real.wav", real, SR)

    ck = torch.load(RUNS / a.species / "voice.pt", map_location="cpu")
    voice = Voice(hidden=ck["hidden"])
    voice.load_state_dict(ck["model"])
    voice.eval()
    bank = torch.from_numpy(noise_bank(a.seconds + 1, seed=5))
    f0 = torch.from_numpy(d["f0"][sl])[None]
    loud = torch.from_numpy(np.maximum(d["loud"][sl], -100))[None]
    eff = torch.from_numpy(d["effort"][sl])[None]
    with torch.no_grad():
        y, _, _ = voice.synth(f0, loud, eff, bank, torch.zeros(1, dtype=torch.long))
    sf.write(out / "2_resynth.wav", y[0].numpy(), SR)

    gen = out / "3_generated.wav"
    subprocess.run(["cargo", "run", "--release", "-q", "--manifest-path", str(ROOT / "core" / "Cargo.toml"),
                    "--example", "neural_bird", "--", a.species, str(a.seconds), str(gen)], check=True)
    sheet([str(out / "1_real.wav"), str(out / "2_resynth.wav"), str(gen)], str(out / "sheet.png"))
    print(f"{out}: real from {d['id']}")


if __name__ == "__main__":
    main()
