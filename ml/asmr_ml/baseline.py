"""MR-STFT of a no-network baseline on the voice's held-out windows, for scale:
a pure sine at the tracked f0 with amplitude from the tracked loudness (no harmonics, no noise).

    uv run python -m asmr_ml.baseline chaffinch
"""

import sys

import numpy as np
import torch

from .dsp import harmonic
from .species import SPECIES
from .train_voice import Data
from .voice import mr_stft_loss


def main() -> None:
    key = sys.argv[1]
    data = Data(key, 200)
    data.species = SPECIES[key]
    f0, loud, _, audio = data.batch(np.random.default_rng(1234), 96, "val")  # same windows as evaluate()
    amp = (10 ** (loud / 20) * np.sqrt(2)).clamp(max=1.0)[..., None]
    y = harmonic(f0, amp)
    print(f"{key}: sine baseline val MR-STFT {float(mr_stft_loss(y, audio)):.3f}")


if __name__ == "__main__":
    main()
