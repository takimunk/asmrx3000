"""Perch 2.0 (Google, Apache 2.0 weights) as an automatic quality metric for synthesised birds.

Separate uv project because Perch needs TensorFlow, which does not co-install with the
training environment. Offline tool only; nothing here ships in the engine.

    uv run --project perch_eval python perch_eval/perch_eval.py --latin "Fringilla coelebs" \
        --set real=out/quality/chaffinch/real --set generated=out/quality/chaffinch/generated

For each set: the loudest non-overlapping 5 s windows (32 kHz) of its WAVs ->
  target_top1   share of windows where the target species is Perch's top bird class
  target_rank   median rank of the target species among all 14 795 classes (1 = best)
  target_prob   mean softmax probability of the target species
  cos_to_real   cosine similarity of the set's mean embedding to the "real" set's mean
  knn_to_real   mean cosine similarity of each window to its nearest "real" window
"""

import argparse
import glob
import json
import os

import numpy as np
import soundfile as sf
from scipy.signal import resample_poly

MODEL = os.path.expanduser("~/.cache/kagglehub/models/google/bird-vocalization-classifier/tensorFlow2/perch_v2_cpu/1")
SR = 32_000
WIN = 5 * SR


def windows(path: str, max_windows: int) -> list[np.ndarray]:
    x, sr = sf.read(path, dtype="float32", always_2d=True)
    x = x.mean(1)
    if sr != SR:
        g = np.gcd(sr, SR)
        x = resample_poly(x, SR // g, sr // g).astype(np.float32)
    if len(x) < WIN:
        x = np.pad(x, (0, WIN - len(x)))
    hop = SR // 2
    starts = np.arange(0, len(x) - WIN + 1, hop)
    rms = np.array([np.sqrt(np.mean(x[s:s + WIN] ** 2)) for s in starts])
    out, taken = [], []
    for i in np.argsort(-rms):
        s = starts[i]
        if rms[i] <= 1e-5 or any(abs(s - t) < WIN for t in taken):
            continue
        taken.append(s)
        w = x[s:s + WIN]
        out.append(w / (np.abs(w).max() + 1e-9) * 0.5)
        if len(out) >= max_windows:
            break
    return out


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--latin", required=True)
    ap.add_argument("--set", action="append", required=True, help="name=dir of wavs")
    ap.add_argument("--max-windows", type=int, default=4, help="per file")
    ap.add_argument("--out")
    a = ap.parse_args()

    import tensorflow as tf

    model = tf.saved_model.load(MODEL).signatures["serving_default"]
    labels = [l.strip() for l in open(os.path.join(MODEL, "assets", "labels.csv"))][1:]
    target = labels.index(a.latin)

    res, embs = {}, {}
    for spec in a.set:
        name, d = spec.split("=", 1)
        ws = [w for f in sorted(glob.glob(os.path.join(d, "*.wav"))) for w in windows(f, a.max_windows)]
        if not ws:
            continue
        e, lg = [], []
        for i in range(0, len(ws), 16):
            o = model(inputs=tf.constant(np.stack(ws[i:i + 16])))
            e.append(o["embedding"].numpy())
            lg.append(o["label"].numpy())
        e, lg = np.concatenate(e), np.concatenate(lg)
        p = np.exp(lg - lg.max(1, keepdims=True))
        p /= p.sum(1, keepdims=True)
        rank = (lg > lg[:, target:target + 1]).sum(1) + 1
        res[name] = {"windows": len(ws), "target_top1": float(np.mean(rank == 1)),
                     "target_rank": float(np.median(rank)), "target_prob": float(p[:, target].mean()),
                     "top_class": labels[int(np.bincount(lg.argmax(1)).argmax())]}
        embs[name] = e / np.linalg.norm(e, axis=1, keepdims=True)
    if "real" in embs:
        ref = embs["real"]
        mref = ref.mean(0) / np.linalg.norm(ref.mean(0))
        for name, e in embs.items():
            m = e.mean(0) / np.linalg.norm(e.mean(0))
            sims = e @ ref.T
            if name == "real":  # leave-one-out for the reference itself
                np.fill_diagonal(sims, -1)
            res[name]["cos_to_real"] = float(m @ mref)
            res[name]["knn_to_real"] = float(sims.max(1).mean())
    print(json.dumps(res, indent=1))
    if a.out:
        with open(a.out, "w") as f:
            json.dump(res, f, indent=1)


if __name__ == "__main__":
    main()
