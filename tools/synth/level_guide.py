"""Evens the body-scan guide out by ear-level measurement: renders the guide alone, compares each
second with the target (one level everywhere, 3 dB under it inside the head, the designed
silences left alone) and scales every path layer's gains there, a few rounds.

ASMR_SOUNDS_DIR=assets/sounds python3 tools/synth/level_guide.py core/scenes/body_scan.json AUDIT_BIN [rounds]
(AUDIT_BIN: a renderer printing `<t>s <dB> dB <segment>` per second for `SCENE 300 solo=guide
bypass every=1`.)"""
import json, math, re, subprocess, sys

scene, audit = sys.argv[1], sys.argv[2]
rounds = int(sys.argv[3]) if len(sys.argv) > 3 else 3

def lerp(pts, t):
    for (a, va), (b, vb) in zip(pts, pts[1:]):
        if a <= t <= b:
            return va + (vb - va) * (t - a) / (b - a)
    return pts[-1][1]

DIP = [(0, 0), (78, 0), (86, 3), (100, 3), (106, 0), (300, 0)]

def measure():
    out = subprocess.run([audit, scene, "300", "solo=guide", "bypass", "every=1"], capture_output=True, text=True).stdout
    lv = {}
    for l in out.splitlines():
        m = re.match(r"\s*(\d+)s\s+(-?[0-9.]+) dB", l)
        if m:
            lv[int(m[1])] = float(m[2])
    return lv

for k in range(rounds):
    lv = measure()
    ref = sorted(v for t, v in lv.items() if 20 <= t <= 75)[len([t for t in lv if 20 <= t <= 75]) // 2]
    err = {}
    for t, v in lv.items():
        if t < 6 or v < -100 or 229 <= t <= 234 or t > 294:
            continue  # fades and the designed silence
        err[t] = max(-12.0, min(12.0, (ref - lerp(DIP, t)) - v))
    worst = max(abs(e) for e in err.values())
    print(f"round {k}: ref {ref:.1f} dB, worst error {worst:.1f} dB")
    d = json.load(open(scene))
    for e in d["entities"]:
        mo = e["motion"]
        if mo["type"] != "path":
            continue
        for p in mo["points"]:
            if len(p) < 5 or p[4] <= 0:
                continue
            ts = [t for t in (math.floor(p[0]) - 1, math.floor(p[0]), math.floor(p[0]) + 1) if t in err]
            if ts:
                p[4] *= 10 ** ((sum(err[t] for t in ts) / len(ts)) / 20 * 0.8)
        top = max(p[4] for p in mo["points"] if len(p) > 4)
        shift = math.ceil(2 * 20 * math.log10(top / 2.0)) / 2 if top > 0 else 0
        e["level_db"] = e["level_db"] + shift
        for p in mo["points"]:
            if len(p) > 4:
                p[4] = round(p[4] * 10 ** (-shift / 20), 3)
    json.dump(d, open(scene, "w"), indent=2)
