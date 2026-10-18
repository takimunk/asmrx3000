"""Synthesis definitions shared by training and the Rust engine (core/src/neural/voice.rs).

Anything here that changes the sound must change in Rust too; the exported weight file
carries these constants so the engine never hard-codes them.

- frames: one control frame every HOP_S seconds; frame i sits at t = i * HOP_S, controls are
  linearly interpolated between frames on every sample.
- harmonic branch: phase accumulates f0 / sr; out = sum_k a_k sin(2 pi k phase), harmonics
  at or above 0.45 * sr are muted.
- noise branch: white noise through a fixed bank of RBJ band-pass biquads (constant 0 dB
  peak gain), each band normalised to unit RMS for white input, times a per-band gain.
"""

import math

import numpy as np
import torch

SR = 48_000
HOP = 240  # samples at SR
HOP_S = HOP / SR  # 5 ms, 200 frames per second
N_HARM = 8
N_BANDS = 20
BAND_LO, BAND_HI = 800.0, 16_000.0


def band_centers() -> np.ndarray:
    return np.geomspace(BAND_LO, BAND_HI, N_BANDS)


def band_q() -> float:
    r = (BAND_HI / BAND_LO) ** (1 / (N_BANDS - 1))
    return 1.0 / (math.sqrt(r) - 1.0 / math.sqrt(r))


def bandpass(fc: float, q: float, sr: float) -> tuple[np.ndarray, np.ndarray]:
    """RBJ cookbook band-pass, constant 0 dB peak gain. Returns (b, a) with a[0] = 1."""
    w = 2 * math.pi * fc / sr
    alpha = math.sin(w) / (2 * q)
    a0 = 1 + alpha
    b = np.array([alpha, 0.0, -alpha]) / a0
    a = np.array([1.0, -2 * math.cos(w) / a0, (1 - alpha) / a0])
    return b, a


def band_norm(fc: float, q: float, sr: float) -> float:
    """Gain that gives the band unit RMS for unit-variance white noise.

    Noise power through H is sum |h[n]|^2; for the RBJ band-pass this has the closed form
    alpha / (1 + alpha) (see Rust `voice.rs`), so the norm is its inverse square root.
    """
    w = 2 * math.pi * fc / sr
    alpha = math.sin(w) / (2 * q)
    return 1.0 / math.sqrt(alpha / (1 + alpha))


def noise_bank(seconds: float, seed: int = 0) -> np.ndarray:
    """[N_BANDS, n] float32: unit-RMS band-limited noise used as the training noise source."""
    from scipy.signal import lfilter

    rng = np.random.default_rng(seed)
    n = int(seconds * SR)
    white = rng.standard_normal(n)
    q = band_q()
    out = np.empty((N_BANDS, n), np.float32)
    for i, fc in enumerate(band_centers()):
        b, a = bandpass(fc, q, SR)
        out[i] = lfilter(b, a, white) * band_norm(fc, q, SR)
    return out


def upsample(x: torch.Tensor, hop: int = HOP) -> torch.Tensor:
    """[B, T, C] frame controls -> [B, T * hop, C], linear between frame i and i+1.

    Sample j of frame i gets x[i] + (x[i+1] - x[i]) * j / hop; the last frame is held.
    """
    nxt = torch.cat([x[:, 1:], x[:, -1:]], dim=1)
    w = torch.arange(hop, device=x.device, dtype=x.dtype) / hop  # [hop]
    y = x[:, :, None, :] + (nxt - x)[:, :, None, :] * w[None, None, :, None]
    return y.reshape(x.shape[0], -1, x.shape[2])


def harmonic(f0: torch.Tensor, amps: torch.Tensor, sr: int = SR) -> torch.Tensor:
    """f0 [B, T] Hz, amps [B, T, K] per frame -> [B, T * HOP] audio."""
    f0s = upsample(f0[..., None])[..., 0]
    a = upsample(amps)
    k = torch.arange(1, amps.shape[-1] + 1, device=f0.device, dtype=f0.dtype)
    # Mute harmonics at or above 0.45 sr (smoothly, at sample rate, like the Rust voice).
    a = a * (f0s[..., None] * k < 0.45 * sr)
    # Phase in cycles, wrapped to keep float32 precise over long excerpts.
    phase = torch.cumsum(f0s / sr, dim=1)
    phase = phase - torch.floor(phase)
    return (a * torch.sin(2 * math.pi * k * phase[..., None])).sum(-1)


def exp_sigmoid(x: torch.Tensor, max_value: float = 2.0, exponent: float = 10.0) -> torch.Tensor:
    """DDSP's scaled sigmoid: positive, log-shaped, bounded."""
    return max_value * torch.sigmoid(x) ** math.log(exponent) + 1e-7
