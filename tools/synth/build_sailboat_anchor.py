"""Builds core/scenes/sailboat_anchor.json: a small yacht at anchor in a quiet bay at night, heard
from the port berth of its cabin (docs/scenes/sailboat_anchor.md).

The scene is in the boat's frame (the listener lies in it): the hull, the mast and the chain stay
put, while the water at the hull and the bay outside move with the boat's motion. Two motions are
written out as looped `path` keyframes:
  * the roll in the swell, ~5.8 s a cycle with slowly breathing amplitude (swell groups): the
    water rises and slaps on the side that rolls down and falls away on the other, so the near
    lapping sways from one ear to the other;
  * the swing at anchor, a slow yaw of +-25 deg over minutes: the moored boats, the bell buoy
    and the gulls wheel slowly around the berth.
Both loops are periodic (an integer number of cycles), so `repeat` joins them without a seam.

python3 tools/synth/build_sailboat_anchor.py   (rewrites the scene file)"""
import json, math, pathlib

ROOT = pathlib.Path(__file__).resolve().parents[2]
OUT = ROOT / "core/scenes/sailboat_anchor.json"
D = "generated/sailboat_anchor/"

# The boat about the head (x forward to the bow, y to port = left, z up), metres.
CL = -0.9          # centreline: the berth is on the port side, the hull skin 0.55 m to the left
PORT, STBD = 0.62, -2.4
WL = -0.35         # waterline outside the hull, a little under the ear
MAST = (1.2, CL)   # keel-stepped mast through the saloon
BOW, STERN = 5.0, -4.6

ROLL_LOOP, ROLL_CYCLES = 300.0, 52          # 5.77 s a roll
YAW_LOOP = 720.0                              # the swing at anchor


def roll(t):
    """Roll phase (rad) and amplitude (0..1.2) at t, periodic in ROLL_LOOP."""
    u = t / ROLL_LOOP
    ph = 2 * math.pi * (ROLL_CYCLES * u) + 0.6 * math.sin(2 * math.pi * 2 * u)
    amp = 0.78 + 0.26 * math.sin(2 * math.pi * 3 * u + 0.5) + 0.12 * math.sin(2 * math.pi * 7 * u + 2.0)
    return ph, amp


def yaw(t):
    u = t / YAW_LOOP
    return math.radians(20 * math.sin(2 * math.pi * u) + 6 * math.sin(2 * math.pi * 3 * u + 1.0))


def r(v, n=3):
    return round(v, n)


def near_path(x, y, z, side, gain_swing, lift=0.07, sway=0.06, base_gain=1.0, phase=0.0):
    """Water at the hull: up and louder when its side rolls down. side +1 port, -1 starboard."""
    pts = []
    steps = ROLL_CYCLES * 6
    for i in range(steps + 1):
        t = ROLL_LOOP * i / steps
        ph, a = roll(t)
        s = math.sin(ph + phase) * a * side
        g = base_gain * (1 + gain_swing * s)
        pts.append([r(t, 2), r(x + 0.04 * math.cos(ph)), r(y - sway * s * side), r(z + lift * s), r(max(g, 0.05))])
    return {"type": "path", "points": pts, "repeat": True}


def far_path(x, y, z, step=8.0):
    """A thing out in the bay as the boat swings: the world turns the other way about the mast."""
    pts = []
    n = int(YAW_LOOP / step)
    for i in range(n + 1):
        t = YAW_LOOP * i / n
        a = -yaw(t)
        dx, dy = x - MAST[0], y - MAST[1]
        pts.append([r(t, 1), r(MAST[0] + dx * math.cos(a) - dy * math.sin(a), 1),
                    r(MAST[1] + dx * math.sin(a) + dy * math.cos(a), 1), r(z, 1)])
    return {"type": "path", "points": pts, "repeat": True}


def fixed(x, y, z):
    return {"type": "fixed", "position": [r(x, 2), r(y, 2), r(z, 2)]}


def clip(id, group, files, motion, **kw):
    e = {"kind": "clip", "id": id, "group": group}
    if files:
        e["files"] = [D + f + ".wav" for f in files]
    e.update(kw)
    e["motion"] = motion
    return e


def model(id, group, sound, motion, **kw):
    e = {"kind": "clip", "id": id, "group": group}
    e.update(kw)
    e["sound"] = sound
    e["motion"] = motion
    return e


LAP = ["yacht_cabin_lap_myloop_1"]
SLAP = ["yacht_hull_slap_kyles_1"]
CLUCK = ["yacht_hull_cluck_kyles_1", "yacht_hull_cluck_kyles_2", "yacht_hull_cluck_kyles_3", "yacht_hull_cluck_el2_1"]
CREAK = ["yacht_cabin_creak_el2_1", "yacht_cabin_creak_el2_2", "yacht_cabin_creak_el2_3"]
ROPE = ["yacht_rope_creak_rmutt_1", "yacht_rope_creak_rmutt_2", "yacht_rope_creak_rmutt_3"]
TAP = ["yacht_halyard_tap_el2_1", "yacht_halyard_tap_el2_2"]
CLINK = ["yacht_halyard_clink_auzet_1", "yacht_halyard_clink_auzet_2", "yacht_halyard_clink_auzet_3"]
RATTLE = ["yacht_halyard_rattle_el2_1", "yacht_halyard_rattle_el2_2"]
CHAIN = ["yacht_chain_grumble_el2_1", "yacht_chain_grumble_el2_3"]
BAYHAL = ["bay_halyards_auzet_1", "bay_halyards_auzet_2"]
JINGLE = ["bay_rigging_ceejay_1"]
BUOY = ["bell_buoy_craigsmith_1", "bell_buoy_craigsmith_2", "bell_buoy_craigsmith_3"]
GULL = ["yacht_gull_canardo_1", "yacht_gull_canardo_2", "yacht_gull_canardo_3", "yacht_gull_genghis_1"]

E = []
# hull: the lapping right at the skin beside you (port), across the cabin (starboard) and under the
# bilge. One field recording from inside a sailboat cabin; loops start at random offsets, so the
# three are decorrelated.
E.append(clip(10, "hull", LAP, near_path(0.3, PORT, WL, +1, 0.4), **{"class": "bed", "playback": "loop", "level_db": 49, "first": 0, "fade": 6}))
E.append(clip(11, "hull", LAP, near_path(0.1, STBD, WL, -1, 0.4, phase=0.25), **{"class": "bed", "playback": "loop", "level_db": 58, "first": 0, "fade": 6}))
E.append(clip(12, "hull", LAP, fixed(0.6, CL, -1.25), **{"class": "bed", "playback": "loop", "level_db": 46, "emitters": 2, "spread": 1.6, "first": 0, "fade": 6}))
# slap: the swell's hollow slaps and clucks on the port side, swinging with the roll.
E.append(clip(20, "slap", SLAP, near_path(0.7, PORT + 0.05, WL + 0.05, +1, 0.7, lift=0.1), **{"class": "texture", "playback": "loop", "level_db": 47, "first": 0, "fade": 6}))
E.append(clip(21, "slap", CLUCK, near_path(-0.4, PORT, WL - 0.15, +1, 0.5, phase=0.6), **{"class": "event", "level_db": 42, "first": 6, "interval": [20, 60], "rate": [0.9, 1.08], "fade": 0.05}))
E.append(clip(22, "slap", CLUCK, near_path(-0.2, STBD + 0.1, WL - 0.15, -1, 0.5, phase=0.6), **{"class": "event", "level_db": 47, "first": 18, "interval": [30, 90], "rate": [0.85, 1.0], "fade": 0.05}))
# boat: the cabin's joinery and the snubber line on the bow cleat.
E.append(clip(30, "boat", CREAK, fixed(0.1, 0.35, 0.55), **{"class": "event", "level_db": 28, "first": 25, "interval": [60, 150], "rate": [0.9, 1.06], "fade": 0.1}))
E.append(clip(31, "boat", CREAK, fixed(2.2, -1.5, 0.2), **{"class": "event", "level_db": 34, "first": 50, "interval": [80, 200], "rate": [0.85, 1.0], "fade": 0.1}))
E.append(clip(32, "boat", ROPE, fixed(BOW - 0.3, CL + 0.2, 0.5), **{"class": "event", "level_db": 47, "first": 40, "interval": [90, 240], "rate": [0.85, 1.0], "fade": 0.1}))
# halyard: a loose halyard on the mast, ticks carried down the mast into the cabin and clinks
# up the mast through the hatch. One tap every few seconds, like a single boat in the port
# recording (not the takes' 0.2 s train).
E.append(clip(40, "halyard", TAP, fixed(MAST[0], MAST[1] + 0.12, 0.4), **{"class": "event", "level_db": 42, "first": 8, "interval": [20, 60], "rate": [0.9, 1.1], "fade": 0.02}))
E.append(clip(41, "halyard", CLINK, fixed(MAST[0], MAST[1], 4.5), **{"class": "event", "level_db": 51, "first": 25, "interval": [45, 120], "rate": [0.92, 1.06], "fade": 0.02}))
# gust: the breeze in the rig, the halyard frapping in the gusts, the moored boats' rigging
# jingling across the water in a fresh wind.
E.append(model(50, "rigging", {"model": "wind", "preset": "fence_wire", "params": {"exposure": 1.0, "gain": 0.13}}, fixed(MAST[0], MAST[1], 7.0), emitters=3, spread=1.2, first=0, fade=8))
E.append(model(51, "rigging", {"model": "wind", "preset": "breeze", "params": {"exposure": 0.6, "gain": 1.6}}, fixed(0.0, CL, 2.5), emitters=3, spread=5, first=0, fade=8))
E.append(clip(52, "gust", RATTLE, fixed(MAST[0], MAST[1], 3.0), **{"class": "event", "level_db": 47, "first": 30, "interval": [25, 70], "rate": [0.92, 1.06], "fade": 0.1}))
E.append(clip(53, "gust", JINGLE, far_path(-55.0, 95.0, 3.0), **{"class": "texture", "playback": "loop", "level_db": 86, "emitters": 2, "spread": 25, "first": 0, "fade": 8}))
# chain: the anchor chain grumbling over the bow roller and on the sand as the boat swings.
E.append(clip(60, "chain", CHAIN, fixed(BOW, CL, 0.25), **{"class": "event", "level_db": 47, "first": 45, "interval": [40, 120], "rate": [0.85, 1.0], "fade": 0.3}))
E.append(clip(61, "chain", CHAIN, fixed(BOW + 9.0, CL, -4.5), **{"class": "event", "level_db": 58, "first": 90, "interval": [60, 160], "rate": [0.7, 0.85], "fade": 0.5}))
# bay: the open water around the boat (procedural, muffled through the hull).
E.append(model(70, "bay", {"model": "water", "preset": "lake_lapping", "params": {"waves": 0.5, "fetch_m": 1500, "rock_d50_m": 0.01, "gain": 0.5}}, fixed(0.0, CL, WL - 0.2), emitters=4, spread=4.5, first=0, fade=8))
# harbour: other boats' halyards far across the bay, swinging round as the boat swings.
E.append(clip(80, "harbour", BAYHAL, far_path(-60.0, 110.0, 2.0), **{"class": "texture", "playback": "loop", "level_db": 85, "emitters": 2, "spread": 30, "first": 0, "fade": 8}))
E.append(clip(81, "harbour", BAYHAL, far_path(85.0, -150.0, 2.0), **{"class": "texture", "playback": "loop", "level_db": 85, "first": 0, "fade": 8}))
# buoy: a bell buoy at the harbour mouth.
E.append(clip(90, "buoy", BUOY, far_path(260.0, -190.0, 1.5, step=6.0), **{"class": "event", "level_db": 87, "first": 20, "interval": [15, 50], "rate": [0.97, 1.03], "fade": 3}))
# gulls: one gull at first light, wheeling over the bay.
E.append(clip(95, "gulls", GULL, {"type": "pass", "from": [60.0, 70.0, 25.0], "to": [-40.0, -80.0, 18.0], "duration": 30, "jitter": 25}, **{"class": "creature", "level_db": 84, "first": 30, "interval": [60, 160], "fade": 1}))
E.append(clip(96, "gulls", GULL, far_path(140.0, 60.0, 4.0), **{"class": "creature", "level_db": 86, "first": 70, "interval": [50, 140], "fade": 1}))

controls = [
    {"group": "hull", "title": "Hull", "icon": "water"},
    {"group": "slap", "title": "Slaps", "icon": "water"},
    {"group": "boat", "title": "Creaks", "icon": "boat"},
    {"group": "halyard", "title": "Halyard", "icon": "mast"},
    {"group": "rigging", "title": "Rigging", "icon": "wind"},
    {"group": "gust", "title": "Gusts", "icon": "wind"},
    {"group": "chain", "title": "Anchor chain", "icon": "anchor"},
    {"group": "bay", "title": "Bay", "icon": "water"},
    {"group": "harbour", "title": "Moored boats", "icon": "mast"},
    {"group": "buoy", "title": "Bell buoy", "icon": "bell"},
    {"group": "gulls", "title": "Gulls", "icon": "gull"},
]

levers = [
    {"key": "swell", "title": "Swell", "icon": "water", "min": 0, "max": 1, "default": 0.3, "lively": 0.4, "drives": [
        {"group": "hull", "param": "level", "at": [[0, -5], [0.3, 0], [1, 4]]},
        {"group": "slap", "param": "level", "at": [[0, -30], [0.3, -10], [0.7, -3], [1, 0]]},
        {"group": "slap", "param": "events", "at": [[0, 0], [0.3, 0.6], [1, 2]]},
        {"group": "boat", "param": "events", "at": [[0, 0.3], [0.3, 1], [1, 2]]},
        {"group": "bay", "param": "level", "at": [[0, -12], [0.3, -4], [1, 3]]}]},
    {"key": "wind", "title": "Wind", "icon": "wind", "min": 0, "max": 1, "default": 0.15, "lively": 0.6, "drives": [
        {"group": "rigging", "param": "level", "at": [[0, -30], [0.15, -14], [0.6, -4], [1, 2]]},
        {"group": "gust", "param": "level", "at": [[0, -30], [0.3, -16], [0.7, -4], [1, 0]]},
        {"group": "gust", "param": "events", "at": [[0, 0], [0.3, 0.3], [0.7, 1], [1, 2]]},
        {"group": "halyard", "param": "events", "at": [[0, 0.8], [1, 1.6]]}]},
    {"key": "rigging", "title": "Rigging", "icon": "mast", "min": 0, "max": 1, "default": 0.35, "lively": 0.2, "drives": [
        {"group": "halyard", "param": "events", "at": [[0, 0], [0.35, 1], [1, 2.5]]},
        {"group": "halyard", "param": "level", "at": [[0, -8], [0.35, 0], [1, 3]]}]},
    {"key": "harbour", "title": "Harbour", "icon": "bell", "min": 0, "max": 1, "default": 0.35, "lively": 0.1, "drives": [
        {"group": "harbour", "param": "level", "at": [[0, -30], [0.35, -2], [1, 4]]},
        {"group": "buoy", "param": "events", "at": [[0, 0], [0.35, 0.8], [1, 1.8]]},
        {"group": "buoy", "param": "level", "at": [[0, -6], [1, 3]]}]},
    {"key": "tide", "title": "Tide", "icon": "anchor", "min": 0, "max": 1, "default": 0.15, "lively": 0.2, "drives": [
        {"group": "chain", "param": "events", "at": [[0, 0], [0.15, 0.5], [0.6, 1.4], [1, 2.6]]},
        {"group": "chain", "param": "level", "at": [[0, -6], [1, 3]]},
        {"group": "boat", "param": "level", "at": [[0, 0], [1, 2]]}]},
    {"key": "dawn", "title": "Dawn", "icon": "gull", "min": 0, "max": 1, "default": 0, "lively": 0.3, "drives": [
        {"group": "gulls", "param": "events", "at": [[0, 0], [0.3, 0.4], [1, 1.6]]},
        {"group": "gulls", "param": "level", "at": [[0, -8], [1, 0]]}]},
]

direction = {"climate": False, "only": True, "start": "still_night", "change": 0.35, "episodes": [
    {"name": "still_night", "wet": -0.5, "energy": -0.7, "drama": 0, "segments": [
        {"label": "a still night at anchor", "minutes": [15, 30], "levers": {"swell": 0.25, "wind": 0.1, "rigging": 0.3}}]},
    {"name": "flat_calm", "wet": -0.8, "energy": -1, "drama": 0, "after": ["still_night", "tide_turns", "breeze"], "segments": [
        {"label": "flat calm", "minutes": [12, 25], "levers": {"swell": 0.1, "wind": 0, "rigging": 0.12, "harbour": 0.25, "tide": 0.05}}]},
    {"name": "breeze", "wet": 0.5, "energy": 0.6, "drama": 0.4, "after": ["still_night", "flat_calm", "tide_turns"], "segments": [
        {"label": "a breeze picks up", "minutes": 5, "levers": {"wind": 0.45, "swell": 0.4, "rigging": 0.55}},
        {"label": "a fresh night breeze", "minutes": [8, 15], "levers": {"wind": 0.8, "swell": 0.65, "rigging": 0.75, "harbour": 0.5, "tide": 0.4}},
        {"label": "the breeze dies away", "minutes": 6, "levers": {"wind": 0.2, "swell": 0.45}}]},
    {"name": "tide_turns", "wet": 0, "energy": 0.1, "drama": 0.7, "after": ["still_night", "flat_calm", "breeze"], "segments": [
        {"label": "slack water", "minutes": 4, "levers": {"swell": 0.15, "wind": 0.05, "tide": 0.3}},
        {"label": "the boat swings to the tide", "minutes": [5, 9], "levers": {"tide": 1, "swell": 0.35}, "ease": 25},
        {"label": "lying to the new tide", "minutes": [6, 10], "levers": {"tide": 0.3, "swell": 0.3}}]},
    {"name": "dawn", "wet": -0.2, "energy": 0.3, "drama": 0.4, "weight": 0.4, "after": ["flat_calm", "tide_turns", "still_night"], "then": "still_night", "segments": [
        {"label": "first light", "minutes": 6, "levers": {"dawn": 0.35, "swell": 0.3, "harbour": 0.45}},
        {"label": "dawn over the bay", "minutes": [8, 14], "levers": {"dawn": 1, "wind": 0.25, "harbour": 0.55}},
        {"label": "morning settles", "minutes": 6, "levers": {"dawn": 0.3, "wind": 0.15}}]},
]}

scene = {
    "name": "Yacht at anchor",
    "seed": 6400,
    # Night at anchor, calm; procedural sounds (wind in the rig, the open water) are heard
    # through the deck and hull (`medium: water` only low-passes the models, see lighthouse).
    "environment": {"time_of_day": 2.0, "wind_ms": 3.0, "rain_mm_h": 0.0, "storm": 0.0, "temperature_c": 14.0,
                    "humidity": 0.85, "ground_z": WL, "medium": "water", "muffle_hz": 2600},
    "entities": E,
    "controls": controls,
    "levers": levers,
    "direction": direction,
}


def compact(o, ind=0):
    """JSON with one keyframe (or one short list) per line: readable and diffable."""
    pad = " " * ind
    if isinstance(o, dict):
        items = [f'{pad} {json.dumps(k)}: {compact(v, ind + 1).lstrip()}' for k, v in o.items()]
        return pad + "{\n" + ",\n".join(items) + "\n" + pad + "}"
    if isinstance(o, list):
        if all(not isinstance(v, (dict, list)) for v in o) or (o and all(isinstance(v, list) and all(not isinstance(w, (dict, list)) for w in v) for v in o) and len(o) <= 6):
            return pad + json.dumps(o)
        if o and all(isinstance(v, list) for v in o):
            return pad + "[\n" + ",\n".join(pad + " " + json.dumps(v) for v in o) + "\n" + pad + "]"
        return pad + "[\n" + ",\n".join(compact(v, ind + 1) for v in o) + "\n" + pad + "]"
    return pad + json.dumps(o)


OUT.write_text(compact(scene) + "\n")
print(f"wrote {OUT.relative_to(ROOT)}: {len(E)} clips, {sum(len(e['motion'].get('points', [])) for e in E)} path keyframes")
