"""Evens the body-scan guide out by measurement: renders the guide alone (the scene's `guide`
clips, head still, mix dynamics off), measures the loudness every second (BS.1770 K-weighting, both ears, 3 s
window), compares it with the target (one level everywhere, 3 dB under it inside the head, the
designed silences and fades left alone) and scales every path layer's keyframe gains there.
A few rounds.

    python3 tools/synth/level_guide.py core/scenes/body_scan.json [rounds]

Needs numpy, scipy, soundfile, and the render example built:
    cargo build --release --manifest-path core/Cargo.toml --example render

The guide's own slow sway (doublet beat, breath) is switched off while measuring, and the
error is smoothed over 5 s, so the gains follow the route (distance, direction, near field,
the body below the ears), never one take's beat phases (the earlier version fitted those, and
a different tone then swayed by +-4 dB).
"""
import json
import math
import os
import subprocess
import sys
import tempfile

import numpy as np
import soundfile as sf

sys.path.insert(0, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "analysis"))
import render_check as rc  # noqa: E402

ROOT = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "..")
RENDER = os.path.join(ROOT, "core/target/release/examples/render")
SECONDS = 300


def lerp(pts, t):
    for (a, va), (b, vb) in zip(pts, pts[1:]):
        if a <= t <= b:
            return va + (vb - va) * (t - a) / (b - a)
    return pts[-1][1]


# The design: 3 dB under the rest inside the head, eased in and out.
DIP = [(0, 0), (78, 0), (86, 3), (100, 3), (106, 0), (300, 0)]
# Left alone: the opening fade, the silence before the legs, the closing fade.
SKIP = lambda t: t < 6 or 229 <= t <= 234 or t > 294  # noqa: E731


def measure(scene):
    d = json.loads(json.dumps(scene))
    d["entities"] = [e for e in d["entities"] if e.get("group") == "guide"]
    for e in d["entities"]:
        e["sound"]["params"].update({"split_hz": 0.0, "breath": 0.0})
    d["levers"] = []
    d["controls"] = [c for c in d["controls"] if c["group"] == "guide"]
    with tempfile.TemporaryDirectory() as tmp:
        sj, wav = os.path.join(tmp, "s.json"), os.path.join(tmp, "g.wav")
        json.dump(d, open(sj, "w"))
        subprocess.run([RENDER, str(SECONDS + 2), wav, "still+nodyn", sj], check=True, capture_output=True)
        x, sr = sf.read(wav, always_2d=True)
    t, l = rc.loudness_series(x, sr, 3.0, 1.0)
    # loudness_series windows start at t: centre them.
    return {int(ti) + 1: li for ti, li in zip(t, l)}


def main():
    path = sys.argv[1]
    rounds = int(sys.argv[2]) if len(sys.argv) > 2 else 3
    for k in range(rounds + 1):
        scene = json.load(open(path))
        lv = measure(scene)
        ref = float(np.median([v for t, v in lv.items() if 20 <= t <= 75]))
        err = {t: max(-12.0, min(12.0, (ref - lerp(DIP, t)) - v)) for t, v in lv.items() if not SKIP(t) and v > -90}
        ts = sorted(err)
        smooth = {t: float(np.mean([err[u] for u in ts if abs(u - t) <= 2])) for t in ts}
        worst = max(abs(e) for e in err.values())
        spread = np.percentile(list(err.values()), 95) - np.percentile(list(err.values()), 5)
        print(f"round {k}: ref {ref:.1f} LUFS, worst error {worst:.1f} dB, p5..p95 {spread:.1f} dB")
        if k == rounds:
            print("error (target - measured) per second:")
            for row in range(0, SECONDS, 10):
                print("  ".join(f"{t:3d}:{err[t]:+5.1f}" if t in err else f"{t:3d}:  -  " for t in range(row, row + 10)))
            break
        for e in scene["entities"]:
            mo = e["motion"]
            if e.get("group") != "guide" or mo["type"] != "path":
                continue
            for p in mo["points"]:
                if len(p) < 5 or p[4] <= 0:
                    continue
                t = int(round(p[0]))
                if t in smooth:
                    p[4] *= 10 ** (smooth[t] / 20 * 0.8)
            # Keep the largest gain at 2 (the clip envelope is clamped at 4; levers add up
            # to +6 dB); the difference goes into the model's level.
            top = max(p[4] for p in mo["points"] if len(p) > 4)
            shift = math.ceil(2 * 20 * math.log10(top / 2.0)) / 2 if top > 0 else 0
            prm = e["sound"]["params"]
            prm["level_db"] = round(prm["level_db"] + shift, 1)
            for p in mo["points"]:
                if len(p) > 4:
                    p[4] = round(p[4] * 10 ** (-shift / 20), 3)
        write(scene, path)


def write(scene, path):
    import re

    txt = json.dumps(scene, indent=1, ensure_ascii=False)
    txt = re.sub(r"\[\s*(-?[\d.e+-]+(?:,\s*-?[\d.e+-]+)*)\s*\]", lambda m: "[" + ", ".join(x.strip() for x in m.group(1).split(",")) + "]", txt)
    open(path, "w").write(txt + "\n")


if __name__ == "__main__":
    main()
