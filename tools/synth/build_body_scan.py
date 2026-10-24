"""Builds core/scenes/body_scan.json. The path layers come from the generator in
docs/scenes/new-scenes-concepts.md (section 4), kept as written there; the rest of the scene
follows the same section.

usage: python3 build_body_scan.py OUT.json
"""
import json, math, os, sys

K = [  # t, x, y, z   (stage boundaries as in the table)
 (0,1.50,0,0.05),(5,1.20,0,0.04),(10,0.90,0.02,0.03),(15,0.62,0.03,0.02),(20,0.40,0.02,0.0),
 *[(20+3*(i+1), round(0.32*math.cos(math.radians(a)),3), round(0.32*math.sin(math.radians(a)),3), 0.0)
   for i,a in enumerate(range(45,361,45))],
 (46,0.30,0,0.06),(49,0.22,0,0.16),(52,0.08,0,0.22),(55,-0.01,0,0.23),(58,-0.02,0.01,0.235),(60,-0.01,0.10,0.21),
 (62,0,0.22,0.12),(64,0,0.28,0),(66,0,0.24,0.12),(68,0,0.12,0.21),(70,0,0,0.235),(72,0,-0.12,0.21),(74,0,-0.24,0.12),
 (76,0,-0.28,0),(80,0,-0.19,0.01),(84,0.01,-0.10,0.02),(88,0.02,-0.03,0.03),(91,0.02,0,0.03),(95,0.025,0,0.035),
 (99,0.02,0,0.025),(102,0.03,0,-0.03),(105,0.05,0,-0.10),(108,0.09,0,-0.19),(111,0.06,0.10,-0.22),(114,0.02,0.19,-0.23),
 (117,0,0.22,-0.29),(120,0,0.24,-0.36),(124,0,0.26,-0.45),(128,0.02,0.27,-0.52),(132,0.08,0.30,-0.57),(136,0.15,0.33,-0.60),
 (139,0.24,0.35,-0.62),(143,0.25,0.355,-0.625),(147,0.26,0.20,-0.64),(151,0.27,0,-0.65),(155,0.26,-0.20,-0.64),
 (159,0.24,-0.35,-0.62),(161,0.23,-0.355,-0.625),(164,0.15,-0.33,-0.60),(167,0.08,-0.30,-0.57),(170,0.02,-0.27,-0.52),
 (173,0,-0.25,-0.40),(176,0.01,-0.20,-0.24),(180,0.08,-0.10,-0.26),(184,0.12,0,-0.32),(188,0.13,0.05,-0.38),
 (192,0.135,0.055,-0.385),(196,0.14,0.02,-0.46),(199,0.16,0,-0.54),(203,0.19,0,-0.545),(209,0.16,0,-0.55),
 (213,0.19,0,-0.55),(219,0.16,0,-0.555),(223,0.19,0,-0.555),(229,0.16,0,-0.57),(231.5,0.14,0.03,-0.64),
 (233,0.12,0.06,-0.70),(237,0.10,0.11,-0.74),(241,0.22,0.12,-0.73),(245,0.36,0.12,-0.72),(249,0.46,0.12,-0.74),
 (253,0.45,0.12,-0.90),(257,0.43,0.12,-1.05),(260,0.42,0.12,-1.15),(263,0.50,0.13,-1.21),(267,0.505,0.13,-1.215),
 (271,0.45,0.08,-0.95),(275,0.36,0.04,-0.70),(279,0.24,0,-0.45),(282,0.20,0,-0.20),(285,0.25,0,0),(289,0.55,0,0),
 (294,0.80,0,-0.05),(300,0.90,0,-0.06)]
def lerp(pts, t):
    for (a,va),(b,vb) in zip(pts, pts[1:]):
        if a <= t <= b: return va + (vb-va)*(t-a)/(b-a)
    return pts[-1][1]
LAYER = {'brush':[(0,1),(76,1),(84,0),(282,0),(286,1),(300,1)],
         'air':  [(0,0),(102,0),(108,1),(116,1),(121,0),(176,0),(181,1),(229,1),(230,0),(267,0),(271,0.85),(275,1),(282,1),(286,0),(300,0)],
         'grain':[(0,0),(116,0),(121,1),(176,1),(181,0),(231.5,0),(233,1),(262,1),(266,0.5),(267,0.5),(273,0),(300,0)],
         'warm': [(0,0),(76,0),(84,1),(102,1),(108,0),(262,0),(266,0.7),(267,0.7),(273,0),(300,0)]}
# The silence before the legs: 230..232 s (the spec's 230..231.5, widened so that 1 s keyframes
# hold it at zero).
FADE   = [(0,0),(6,1),(229,1),(230,0),(232,0),(233,1),(289,1),(300,0)]
INSIDE = [(0,1),(78,1),(88,0.45),(100,0.45),(106,1),(300,1)]
BREATH = [(0,1),(196,1),(199,0.85),(203,1.25),(209,0.85),(213,1.25),(219,0.85),(223,1.25),(229,1),(300,1)]

# --- Resampling -------------------------------------------------------------------------
# The engine runs a uniform Catmull-Rom through the keyframes, ignoring their times. With the
# route's uneven spacing (2..6 s) that makes the speed jump at every knot, and the near-still
# holds (fingertips, crown, heart, soles) overshoot into a small back-and-forth loop. So the
# route is first made a smooth function of time here (cubic Hermite with time-scaled tangents,
# zero tangents at the ends of a hold), then sampled every STEP s: on evenly spaced keyframes
# the engine's spline follows it closely.
STEP = 1.0
HOLD_MS = 0.006   # a segment slower than this (m/s) is a hold

def _sub(a, b): return [x - y for x, y in zip(a, b)]
def _norm(a): return math.sqrt(sum(x * x for x in a))

def route_at(t):
    ts = [k[0] for k in K]; ps = [list(k[1:4]) for k in K]
    n = len(K)
    t = min(max(t, ts[0]), ts[-1])
    i = next(j for j in range(n - 1) if t <= ts[j + 1])
    def vel(j):  # m/s on segment j..j+1
        return [d / (ts[j + 1] - ts[j]) for d in _sub(ps[j + 1], ps[j])]
    def hold(j): return 0 <= j < n - 1 and _norm(vel(j)) < HOLD_MS
    def tangent(k):
        if k == 0 or k == n - 1 or hold(k - 1) or hold(k):
            return [0.0, 0.0, 0.0]
        a, b = vel(k - 1), vel(k)
        return [(x + y) / 2 for x, y in zip(a, b)]
    h = ts[i + 1] - ts[i]
    u = (t - ts[i]) / h
    m0, m1 = tangent(i), tangent(i + 1)
    h00, h10, h01, h11 = 2*u**3 - 3*u**2 + 1, u**3 - 2*u**2 + u, -2*u**3 + 3*u**2, u**3 - u**2
    return [h00 * p0 + h10 * h * a + h01 * p1 + h11 * h * b for p0, p1, a, b in zip(ps[i], ps[i + 1], m0, m1)]

SAMPLES = [round(i * STEP, 3) for i in range(int(300 / STEP) + 1)]

# --- Level along the route ----------------------------------------------------------------
# The spec's g_dist only undoes distance. Measured in the engine (white-ish noise, mix
# bypassed), the body below the ears costs much more: -4 dB at -45 deg elevation, -7 dB at
# -70 deg, -4.6 dB over the crown, -7.6 dB straight behind. So each sample's gain comes from
# the engine itself: a loop of the layer's file (or a stand-in while the takes do not exist)
# is held at the sample's position and measured; the gain brings it to the 0.4 m front level,
# minus a small distance cue (the spec's exponent 0.8 = 20 % of the 1/r fall kept). Low
# frequencies are lifted far more than 1/r close to the head by the near-field filters (a 110 Hz
# hum: +16 dB from 0.4 to 0.2 m), so the cut may go down to -30 dB.
CALIB = os.environ.get("CALIB_BIN", os.path.join(os.path.dirname(os.path.abspath(__file__)), "sc/target/release/calib"))
def measure(positions, wav):
    import subprocess, tempfile
    with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
        json.dump(positions, f)
    out = subprocess.run([CALIB, f.name, wav, "-1.23"], check=True, capture_output=True, text=True).stdout
    return json.loads(out)

_cal = {}
def g_cal(proxy, mirror):
    key = (proxy, mirror)
    if key not in _cal:
        pts = [route_at(t) for t in SAMPLES]
        if mirror: pts = [[x, -y, z] for x, y, z in pts]
        # One reference for every layer: white-ish noise at 0.4 m in front. Each layer's own
        # file is measured against it, so a hum (whose lows the near field lifts by ~10 dB at
        # 0.4 m) is not left 10 dB hotter than the brush; level_db offsets (grain -2, warm +2)
        # stay as designed.
        ref = measure([[0.4, 0.0, 0.0]], WHITE)[0]
        got = measure(pts, proxy)
        g = []
        for t, (x, y, z), m in zip(SAMPLES, pts, got):
            r = math.sqrt(x*x + y*y + z*z)
            target = ref - 0.2 * 20 * math.log10(max(r, 0.15) / 0.4)
            # Inside the head the guide is quieter (the design's -4 dB under the lap around it):
            # 8 dB at the bottom, eased in over 78..86 s and out over 100..106 s (-4 dB on the stage mean).
            target -= 8.0 * lerp([(0, 0), (78, 0), (86, 1), (100, 1), (106, 0), (300, 0)], t)
            g.append(10 ** (min(max(target - m, -30.0), 14.0) / 20))
        _cal[key] = g
    return _cal[key]

def layer(name, proxy, mirror=False):
    w = LAYER[name]
    gc = g_cal(proxy, mirror)
    out = []
    for t, g0 in zip(SAMPLES, gc):
        x, y, z = route_at(t)
        g = g0 * lerp(FADE, t) * lerp(INSIDE, t) * lerp(BREATH, t) * lerp(w, t)
        out.append([t, round(x, 4), round(-y if mirror else y, 4), round(z, 4), g])
    return out

def fit(level, pts, gmax=2.0):
    """Moves the layer's level so its largest path gain is gmax (the clip envelope is clamped at
    4, and levers add up to +6 dB): same sound, headroom kept."""
    top = max(p[4] for p in pts)
    shift = math.ceil(2 * 20 * math.log10(top / gmax)) / 2 if top > 0 else 0
    k = 10 ** (-shift / 20)
    return level + shift, [p[:4] + [round(p[4] * k, 3)] for p in pts]

# The twin (right leg): the left leg's grain mirrored (y -> -y), from the hips to where the two
# merge again on the way up the front; silent before and after.
def twin(proxy):
    pts = [p for p in layer('grain', proxy, mirror=True) if 228 <= p[0] <= 283]
    return [[0, pts[0][1], pts[0][2], pts[0][3], 0.0]] + pts

# The brush, air and hum takes breathe: strokes dip 15..30 dB for a fraction of a second.
# Two emitters at the same point play different stretches of the loop (decorrelated), so their
# dips rarely meet and the guide stays continuous; -3 dB keeps the level.
# Not the hum: two stretches of one steady tone beat against each other (measured: 10..15 dB
# flutter inside the head).
SMOOTH = set()  # the guide is clean tones now: two stretches of a tone would beat
def path_clip(cid, name, files, level, pts):
    level, pts = fit(level, pts)
    d = {"kind": "clip", "id": cid, "group": "guide", "class": "texture", "level_db": level, "playback": "loop",
         "fade": 0.5, "first": 0, "files": files, "motion": {"type": "path", "points": pts}}
    if name in SMOOTH:
        d["level_db"] = level - 3.0
        d["emitters"] = 2
    return d

G = "generated/body_scan/"
REPO = os.environ.get("REPO", "/Users/diipushk/projects/asmrx3000/.claude/worktrees/scene-direction-module-fe106b")
HERE = os.path.dirname(os.path.abspath(__file__))
WHITE = os.path.join(HERE, "ph2/generated/body_scan/white.wav")
def proxy(name, warm=False):
    """The layer's own take for the calibration when it exists, else a stand-in spectrum."""
    real = os.path.join(REPO, "assets/sounds", G, name)
    if os.path.exists(real):
        return real
    return os.path.join(HERE, "ph/generated/body_scan/guide_warm_el2_1.wav" if warm else "ph2/generated/body_scan/white.wav")

def bowl(cid, at, pos, level, rate, files):
    return {"kind": "clip", "id": cid, "group": "bowls", "class": "event", "level_db": level, "playback": "events",
            "first": at, "exact": True, "interval": [99999, 99999], "rate": rate, "fade": 0.05,
            "files": files, "motion": {"type": "fixed", "position": pos}}

bowls = [G + f"bowl_strike_el2_{i}.wav" for i in (1, 2, 3)]
tingsha = [G + f"tingsha_el2_{i}.wav" for i in (1, 2)]
entities = [
    path_clip(10, "brush", [G + "guide_brush_el2_1.wav", G + "guide_brush_el2_2.wav"], 38, layer('brush', proxy("guide_brush_el2_1.wav"))),
    path_clip(11, "air", [G + "guide_air_el2_1.wav"], 38, layer('air', proxy("guide_air_el2_1.wav"))),
    path_clip(12, "grain", [G + "guide_grain_el2_1.wav"], 36, layer('grain', proxy("guide_grain_el2_1.wav"))),
    path_clip(13, "warm", [G + "guide_warm_el2_1.wav"], 40, layer('warm', proxy("guide_warm_el2_1.wav", warm=True))),
    path_clip(14, "twin", [G + "guide_grain_el2_2.wav"], 36, twin(proxy("guide_grain_el2_2.wav"))),
    bowl(30, 1, [1.5, 0, -0.25], 49, [1, 1], bowls),
    bowl(31, 140, [0.25, 0.36, -0.62], 42, [0.97, 1.03], tingsha),
    bowl(32, 160, [0.23, -0.36, -0.62], 42, [0.97, 1.03], tingsha),
    bowl(33, 264, [0.5, 0, -1.22], 46, [0.8, 0.8], bowls),
    bowl(34, 292, [1.4, 0, -0.3], 47, [1, 1], bowls),
]

# The direction starts somewhere inside its first segment (30..100 % of it left). A 3 s lead-in
# (the shortest segment allowed) takes that jitter: it lasts 0.9..3 s, so with 18 s after it
# every later stage starts within -1.1..+1.0 s of the path's own boundary.
segs = [("arrival", 3, None), ("arrival", 18, None), ("around the head", 26, None), ("crown", 30, None),
        ("inside", 26, None), ("throat & shoulders", 15, None), ("left arm", 26, None),
        ("across to the right arm", 33, None), ("chest & heart", 20, None), ("belly", 33, None),
        ("legs & feet", 38, None), ("return", 22, None), ("rest", 11, None)]
assert abs(sum(s for _, s, _ in segs) - 301) < 1e-9
def seg(label, secs, levers):
    d = {"label": label, "minutes": round(secs / 60, 6)}
    if levers: d["levers"] = levers
    return d

scene = {
    "name": "Body scan",
    "seed": 6100,
    "environment": {"time_of_day": 21.0, "rain_mm_h": 0.0, "wind_ms": 2.0, "storm": 0.0, "temperature_c": 20.0,
                    "humidity": 0.5, "ground_z": -1.23, "loudness_lufs": -30.0},
    "controls": [
        {"group": "guide", "title": "Guide", "icon": "tone", "params": ["level"]},
        {"group": "bowls", "title": "Bowls", "icon": "tone", "params": ["level"]},
    ],
    "levers": [
        {"key": "guide", "title": "Guide", "icon": "tone", "min": -12, "max": 6, "default": 0, "lively": 0,
         "drives": [{"group": "guide", "param": "level", "at": [[-12, -12], [6, 6]]}]},
        {"key": "bowls", "title": "Bowls", "icon": "tone", "min": 0, "max": 1, "default": 0.7, "lively": 0,
         "drives": [{"group": "bowls", "param": "level", "at": [[0, -30], [0.7, 0], [1, 4]]}]},
    ],
    "direction": {
        "climate": False, "only": True, "start": "body_scan", "script": [{"episode": "body_scan"}], "repeat": False,
        "energy": -0.6, "change": 0.1,
        "episodes": [
            {"name": "body_scan", "weight": 0, "energy": -0.6, "drama": 0.4, "then": "rest",
             "segments": [seg(*s) for s in segs]},
            {"name": "rest", "weight": 1, "energy": -1,
             "segments": [{"label": "afterglow", "minutes": 2},
                          {"label": "stillness", "minutes": [3, 6]}]},
        ],
    },
    "entities": entities,
}

def dump(o, ind=0):
    """Readable JSON: one entity per block, each path keyframe on its own line."""
    return json.dumps(o, indent=1)

out = json.dumps(scene, indent=1)
# keyframes one per line instead of one number per line
import re
out = re.sub(r"\[\n\s+(-?[\d.e+-]+),\n\s+(-?[\d.e+-]+),\n\s+(-?[\d.e+-]+)(?:,\n\s+(-?[\d.e+-]+))?(?:,\n\s+(-?[\d.e+-]+))?\n\s+\]",
             lambda m: "[" + ", ".join(g for g in m.groups() if g is not None) + "]", out)
open(sys.argv[1], "w").write(out + "\n")
