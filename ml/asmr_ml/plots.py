"""Spectrogram sheets for listening examples (real vs resynthesis vs generated)."""

import sys

import librosa
import matplotlib
import numpy as np
import soundfile as sf

matplotlib.use("Agg")
import matplotlib.pyplot as plt  # noqa: E402


def sheet(paths: list[str], out: str, seconds: float = 8.0, fmax: float = 12000) -> None:
    fig, ax = plt.subplots(len(paths), 1, figsize=(14, 2.6 * len(paths)), squeeze=False)
    for a, p in zip(ax[:, 0], paths):
        y, sr = sf.read(p, dtype="float32", always_2d=True)
        y = y.mean(1)
        nz = np.flatnonzero(np.abs(y) > 0.01 * np.abs(y).max())
        start = max(0, (nz[0] if len(nz) else 0) - sr // 4)
        y = y[start:start + int(seconds * sr)]
        S = librosa.amplitude_to_db(np.abs(librosa.stft(y, n_fft=1024, hop_length=256)), ref=np.max)
        a.imshow(S, origin="lower", aspect="auto", cmap="magma", vmin=-70,
                 extent=[0, len(y) / sr, 0, sr / 2])
        a.set_ylim(0, fmax)
        a.set_title(p.split("/")[-1], fontsize=9)
    fig.tight_layout()
    fig.savefig(out, dpi=70)


if __name__ == "__main__":
    sheet(sys.argv[2:], sys.argv[1])
