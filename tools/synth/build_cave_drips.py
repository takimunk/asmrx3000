#!/usr/bin/env python3
"""Writes core/scenes/cave_drips.json: a dry ledge deep in a limestone cave (docs/scenes/cave_drips.md).

Engine frame: head at the origin, +x forward, +y left, +z up, metres. The ledge is at z -0.9, the
cave floor with its still pools and the stream at z -3.6 (`ground_z`), the dome rises to ~+9 m,
the passage to the mouth runs away behind (-x).

    python3 tools/synth/build_cave_drips.py
"""
import json
import math
import pathlib

ROOT = pathlib.Path(__file__).resolve().parents[2]
D = "generated/cave_drips/"


def files(stem, n, start=1):
    return [f"{D}{stem}_{i}.wav" for i in range(start, start + n)]


PLINK = files("cave_plink", 10)
SLOW = files("cave_slowdrip", 3)
DEEP = files("cave_deepdrip", 3)
ROCK_DRY = files("cave_rockdrip", 8)
ROCK_CAVE = files("cave_rockdrip", 3, 9)
PEBBLES = files("cave_pebbles", 7)


def echo(delay, feedback, mix, damp):
    return {"delay": delay, "feedback": feedback, "mix": mix, "damp_hz": damp}


def drip(id_, group, fs, pos, level, first, interval, rate, ec):
    return {"kind": "clip", "id": id_, "group": group, "class": "event", "files": fs, "level_db": level,
            "first": first, "interval": interval, "rate": rate, "fade": 0.01, "echo": ec,
            "motion": {"type": "fixed", "position": pos}}


ents = []
# The slow drip by your right knee: one stalactite into a small basin on the ledge, every ~6.5 s.
ents.append(drip(10, "slow", SLOW, [0.75, -0.85, -0.92], 52, 2, [6.1, 6.9], [0.985, 1.015],
                 echo(0.17, 0.55, 0.55, 3500)))

# Plinks into still pools on the cave floor below, all round (azimuth deg, distance m, z).
pools = [(35, 5.2, -3.5), (-40, 8.5, -3.5), (120, 6.5, -3.5), (-70, 4.0, -3.5), (10, 13.0, -3.5), (-145, 7.5, -3.5)]
for k, (az, r, z) in enumerate(pools):
    a = math.radians(az)
    pos = [round(r * math.cos(a), 2), round(r * math.sin(a), 2), z]
    d = [0.23, 0.29, 0.26, 0.21, 0.34, 0.27][k]
    ents.append(drip(20 + k, "pools", PLINK, pos, 58, 3 + 4 * k, [10, 30], [0.86, 1.16],
                     echo(d, 0.68, 0.75, 2600)))

# Drops on rock at every height: the ledge beside you, shelves and formations up in the dome.
rocks = [([1.0, 1.1, -0.9], ROCK_DRY), ([-1.5, 2.6, 2.8], ROCK_DRY + ROCK_CAVE), ([2.6, -1.8, 4.6], ROCK_DRY),
         ([-4.0, -3.0, 1.0], ROCK_DRY + ROCK_CAVE), ([5.5, 5.5, 1.6], ROCK_DRY), ([-2.2, 0.4, 5.8], ROCK_DRY + ROCK_CAVE)]
for k, (pos, fs) in enumerate(rocks):
    d = [0.19, 0.31, 0.37, 0.27, 0.33, 0.41][k]
    ents.append(drip(30 + k, "rock", fs, pos, 54, 5 + 5 * k, [16, 45], [0.8, 1.25], echo(d, 0.66, 0.8, 3200)))

# Far in: deep drips into a large pool and onto rock, with the long echo of the far halls.
for k, pos in enumerate([[16.0, -7.0, -3.0], [-11.0, 9.0, 1.5], [11.0, 12.0, -3.4]]):
    fs = DEEP if k != 1 else ROCK_CAVE + PLINK[:4]
    ents.append(drip(40 + k, "deep", fs, pos, 70, 9 + 11 * k, [20, 60], [0.8, 1.0],
                     echo([0.47, 0.55, 0.6][k], 0.75, 1.3, 1500)))

# The underground stream: a low murmur running along the floor below on the right, and where it
# breaks over a lip further ahead, a brighter runnel.
ents.append({"kind": "clip", "id": 50, "group": "stream", "class": "bed", "playback": "loop",
             "files": [f"{D}cave_stream_1.wav", f"{D}cave_stream_2.wav"], "level_db": 51, "first": 0, "fade": 6,
             "emitters": 3, "spread": 4, "motion": {"type": "fixed", "position": [1.0, -6.0, -3.4]}})
ents.append({"kind": "clip", "id": 51, "group": "stream", "class": "bed", "playback": "loop",
             "files": [f"{D}cave_runnel_1.wav"], "level_db": 50, "first": 0, "fade": 6,
             "emitters": 2, "spread": 1.5, "motion": {"type": "fixed", "position": [9.0, -10.0, -3.3]}})

# The draught from the cave mouth far behind: slow air in the passage with its low hollow note.
ents.append({"kind": "clip", "id": 60, "group": "draught", "class": "texture", "emitters": 3, "spread": 3,
             "first": 0, "fade": 8,
             "sound": {"model": "wind", "preset": "breeze",
                       "params": {"exposure": 1.2, "body": 1.0, "tilt_db": 3.0, "gap_hz": 70.0, "gap_length_m": 1.5,
                                  "gap_q": 8.0, "gap": 0.35, "distance_m": 40.0, "gust_response": 0.5, "gain": 0.45}},
             "motion": {"type": "fixed", "position": [-26.0, 1.0, 1.0]}})

# Small stones trickling down a slope: from a shelf down to the floor.
for k, (a, b, first) in enumerate([([5.0, 5.2, 2.6], [5.5, 4.6, -1.8], 140), ([-3.6, -6.0, 3.4], [-3.2, -6.4, -2.0], 330)]):
    ents.append({"kind": "clip", "id": 70 + k, "group": "pebbles", "class": "event", "files": PEBBLES, "level_db": 60,
                 "first": first, "interval": [150, 420], "rate": [0.85, 1.1], "fade": 0.05,
                 "echo": echo(0.29, 0.6, 0.8, 2500),
                 "motion": {"type": "pass", "from": a, "to": b, "duration": 1.8}})

scene = {
    "name": "Cave",
    "seed": 7300,
    "environment": {"time_of_day": 12.0, "season": "autumn", "temperature_c": 9.0, "humidity": 0.98,
                    "wind_ms": 3.0, "rain_mm_h": 0.0, "storm": 0.0, "ground_z": -3.6},
    "entities": ents,
    "controls": [
        {"group": "slow", "title": "Slow drip", "icon": "drip"},
        {"group": "pools", "title": "Pools", "icon": "ripple"},
        {"group": "rock", "title": "Drips on rock", "icon": "drip"},
        {"group": "deep", "title": "Far drips", "icon": "cave"},
        {"group": "stream", "title": "Stream", "icon": "water"},
        {"group": "draught", "title": "Draught", "icon": "wind"},
        {"group": "pebbles", "title": "Pebbles", "icon": "pebbles"},
    ],
    "levers": [
        {"key": "drips", "title": "Drips", "icon": "drip", "min": 0, "max": 1, "default": 0.5, "lively": 0.3,
         "drives": [{"group": "pools", "param": "events", "at": [[0, 0.12], [0.5, 1], [1, 2.4]]},
                    {"group": "rock", "param": "events", "at": [[0, 0.12], [0.5, 1], [1, 2.4]]},
                    {"group": "deep", "param": "events", "at": [[0, 0.2], [0.5, 1], [1, 2]]},
                    {"group": "slow", "param": "events", "at": [[0, 0.55], [0.5, 1], [1, 1.25]]}]},
        {"key": "pools", "title": "Pools", "icon": "ripple", "min": 0, "max": 1, "default": 0.5, "lively": 0,
         "drives": [{"group": "pools", "param": "events", "at": [[0, 0.1], [0.5, 1], [1, 1.8]]},
                    {"group": "pools", "param": "level", "at": [[0, -4], [0.5, 0], [1, 1]]},
                    {"group": "rock", "param": "events", "at": [[0, 1.8], [0.5, 1], [1, 0.25]]}]},
        {"key": "stream", "title": "Stream", "icon": "water", "min": 0, "max": 1, "default": 0.4, "lively": 0.2,
         "drives": [{"group": "stream", "param": "level", "at": [[0, -30], [0.15, -12], [0.4, 0], [1, 8]]}]},
        {"key": "depth", "title": "Depth", "icon": "cave", "min": 0, "max": 1, "default": 0.6, "lively": 0,
         "drives": [{"group": "draught", "param": "level", "at": [[0, 6], [0.6, -4], [1, -30]]},
                    {"group": "deep", "param": "level", "at": [[0, -14], [0.6, 0], [1, 3]]},
                    {"group": "deep", "param": "events", "at": [[0, 0.3], [0.6, 1], [1, 1.5]]}]},
        {"key": "pebbles", "title": "Pebbles", "icon": "pebbles", "min": 0, "max": 1, "default": 0.3, "lively": 0.3,
         "drives": [{"group": "pebbles", "param": "events", "at": [[0, 0], [0.3, 1], [1, 3]]}]},
    ],
    "direction": {
        "climate": False, "only": True, "start": "settling_in", "change": 0.35,
        "episodes": [
            {"name": "settling_in", "weight": 0, "wet": 0, "energy": -0.2, "drama": 0.2,
             "segments": [{"label": "the cave settles around you", "minutes": [6, 10],
                           "levers": {"drips": 0.42, "pools": 0.55, "stream": 0.35}}]},
            {"name": "drip_by_drip", "wet": 0, "energy": -0.3, "drama": 0,
             "segments": [{"label": "drip by drip", "minutes": [15, 30], "levers": {"drips": [0.4, 0.6], "pools": [0.4, 0.65]}}]},
            {"name": "dry_spell", "wet": -0.8, "energy": -0.6, "drama": 0.2, "after": ["drip_by_drip", "deep_silence"],
             "segments": [{"label": "the drips thin out", "minutes": 5, "levers": {"drips": 0.25, "stream": 0.25}},
                          {"label": "dry spell", "minutes": [12, 20], "levers": {"drips": 0.12, "stream": 0.12, "pebbles": 0.1}},
                          {"label": "the first drips return", "minutes": 5, "levers": {"drips": 0.35, "stream": 0.3}}]},
            {"name": "rain_above", "wet": 0.9, "energy": 0.5, "drama": 0.6, "after": ["drip_by_drip", "deep_silence", "dry_spell"],
             "segments": [{"label": "rain soaks in above", "minutes": [4, 6], "levers": {"drips": 0.68, "stream": 0.55}},
                          {"label": "the cave is raining", "minutes": [10, 16],
                           "levers": {"drips": 1, "pools": 0.65, "stream": 0.9, "pebbles": 0.6}},
                          {"label": "the water drains away", "minutes": [6, 8], "levers": {"drips": 0.6, "stream": 0.6}}]},
            {"name": "deep_silence", "wet": -0.3, "energy": -0.9, "drama": 0.1, "after": ["drip_by_drip", "dry_spell"],
             "segments": [{"label": "deep silence", "minutes": [12, 20],
                           "levers": {"drips": 0.22, "stream": 0.15, "depth": 1, "pebbles": 0}}]},
            {"name": "air_from_outside", "weight": 0.6, "wet": 0.2, "energy": 0.2, "drama": 0.3, "after": ["drip_by_drip", "rain_above"],
             "segments": [{"label": "air from the entrance", "minutes": [6, 10], "levers": {"depth": 0.12, "drips": 0.45}},
                          {"label": "the air stills", "minutes": 4, "levers": {"depth": 0.5}}]},
            {"name": "stones_settle", "weight": 0.4, "wet": 0.1, "energy": 0.3, "drama": 0.6, "after": ["drip_by_drip", "rain_above"],
             "segments": [{"label": "stones settle", "minutes": 3, "levers": {"pebbles": 1}},
                          {"label": "still again", "minutes": 3, "levers": {"pebbles": 0.15}}]},
        ],
    },
}

out = ROOT / "core/scenes/cave_drips.json"
out.write_text(json.dumps(scene, indent=1, ensure_ascii=False) + "\n")
print(f"wrote {out.relative_to(ROOT)}: {len(ents)} clips")
