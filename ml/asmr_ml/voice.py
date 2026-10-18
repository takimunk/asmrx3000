"""DDSP voice: frame controls (f0, loudness, effort) -> harmonic amplitudes + noise band gains.

Mirrored layer by layer in core/src/neural/voice.rs. Keep the two in sync:
  x  = [ (log2 f0 - f0_mu) / f0_sd, (loud - l_mu) / l_sd, effort ]
  z  = leaky_relu(in(x))                      Linear 3 -> E
  g  = GRU(z)                                 PyTorch gate order r, z, n
  y  = out2(leaky_relu(out1([g, x])))         Linear H+3 -> M -> 1 + K + B
  amp   = exp_sigmoid(y0)
  harm  = amp * softmax(y1..K over harmonics below 0.45 sr)
  noise = exp_sigmoid(y(K+1).. - NOISE_BIAS)
"""

import torch
import torch.nn.functional as F
from torch import nn

from .dsp import HOP, N_BANDS, N_HARM, SR, exp_sigmoid, harmonic, upsample

NOISE_BIAS = 5.0
LEAK = 0.1


class Voice(nn.Module):
    def __init__(self, embed: int = 64, hidden: int = 96, mlp: int = 128,
                 n_harm: int = N_HARM, n_bands: int = N_BANDS):
        super().__init__()
        self.n_harm, self.n_bands = n_harm, n_bands
        self.inp = nn.Linear(3, embed)
        self.gru = nn.GRU(embed, hidden, batch_first=True)
        self.out1 = nn.Linear(hidden + 3, mlp)
        self.out2 = nn.Linear(mlp, 1 + n_harm + n_bands)
        # Normalisation of the inputs, set from the data before training.
        self.register_buffer("norm", torch.tensor([12.0, 0.5, -30.0, 15.0]))  # f0_mu, f0_sd, l_mu, l_sd

    def inputs(self, f0, loud, effort):
        f0_mu, f0_sd, l_mu, l_sd = self.norm
        return torch.stack([(torch.log2(f0) - f0_mu) / f0_sd, (loud - l_mu) / l_sd, effort], -1)

    def forward(self, f0, loud, effort, h=None):
        """f0, loud, effort: [B, T]. Returns harmonic amps [B, T, K], noise gains [B, T, B], h."""
        x = self.inputs(f0, loud, effort)
        z = F.leaky_relu(self.inp(x), LEAK)
        g, h = self.gru(z, h)
        y = self.out2(F.leaky_relu(self.out1(torch.cat([g, x], -1)), LEAK))
        amp = exp_sigmoid(y[..., :1])
        k = torch.arange(1, self.n_harm + 1, device=f0.device, dtype=f0.dtype)
        allowed = f0[..., None] * k < 0.45 * SR
        logits = y[..., 1 : 1 + self.n_harm].masked_fill(~allowed, -1e4)
        harm = amp * torch.softmax(logits, -1)
        noise = exp_sigmoid(y[..., 1 + self.n_harm :] - NOISE_BIAS)
        return harm, noise, h

    def synth(self, f0, loud, effort, bank: torch.Tensor, offsets: torch.Tensor):
        """Render audio [B, T * HOP]. `bank` [N_BANDS, L] unit-RMS band noise; one offset per item."""
        harm, noise, _ = self(f0, loud, effort)
        n = f0.shape[1] * HOP
        idx = offsets[:, None] + torch.arange(n, device=f0.device)[None]  # [B, n]
        bands = bank[:, idx].permute(1, 2, 0)  # [B, n, N_BANDS]
        noise_audio = (upsample(noise) * bands).sum(-1)
        return harmonic(f0, harm) + noise_audio, harm, noise


def mr_stft_loss(y: torch.Tensor, t: torch.Tensor, ffts=(2048, 1024, 512, 256, 128)) -> torch.Tensor:
    loss = y.new_zeros(())
    for n in ffts:
        w = torch.hann_window(n, device=y.device)
        Y = torch.stft(y, n, n // 4, window=w, return_complex=True).abs()
        T = torch.stft(t, n, n // 4, window=w, return_complex=True).abs()
        loss = loss + (Y - T).abs().mean() + (torch.log(Y + 1e-5) - torch.log(T + 1e-5)).abs().mean()
    return loss / len(ffts)
