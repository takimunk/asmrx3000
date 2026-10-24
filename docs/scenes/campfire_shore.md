# Campfire by the sea (`campfire_shore.json`)

Date: 2026-10-24. Status: built, calibrated by render; not yet heard in the app with AirPods.

## Premise
Night on a shingle-and-sand beach. You sit on a driftwood log; a small driftwood fire burns 1 m in front of
you on the sand. Ahead, 15–35 m away, the sea fills a wide arc: the bay's surf as a continuous wash, and the
near waves that wash up the shingle and drain back, each running along the waterline from left to right as
the swell comes in obliquely. A light onshore breeze comes off the sea, the marram on the dunes rustles
behind you. Rarely a curlew calls far along the shore on the left, and a bell buoy clangs far out on the right.

## Spatial idea
Intimate near field against a wide horizontal field. The fire is a point at arm's length, just below ear
level (ears 0.85 m above the sand): crackle, pops, a log settling, damp wood hissing, all within 1.1 m, with
the near-field cues of the engine. Everything else is far and wide: the surf spans ±60° in front at 15–35 m,
the waves travel laterally across it (a slow, continuous left-to-right motion, the kind of movement that is
followed best), the breeze comes from the sea side, the grass is behind (front/back contrast), the curlew is
a far point on the left, the buoy a far point on the right.

| Group | Clips | Position | Notes |
|---|---|---|---|
| `fire` | real crackle loop (Glaneur de sons, CC BY 3.0); generated crackle loop | [1.0, ±0.1, −0.7] | the steady close fire |
| `flames` | generated flare loops; real bonfire loop (WWS, CC BY 4.0) | [1.0, 0, −0.4] | brought in by the Fire lever |
| `embers` | physical model `fire/embers` (gain 0.8) | [0.95, 0, −0.8] | coal ticks; louder relative to the rest when the fire is low |
| `wood` | log shifting (3 takes, 70–170 s), damp driftwood hiss (3 takes, 50–130 s) | beside the fire | one-shots |
| `sea` | far surf bed (2 loops, 5 emitters over 18 m); near waves (4 takes) on three `pass` lanes 15, 19, 24 m out | ahead | each wave travels +y → −y over its 11 s |
| `wind` | physical model `wind/breeze` (exposure 0.85), 3 emitters | 9 m ahead, 0.6 m up | onshore |
| `grass` | physical model `foliage/dry_grass`, 4 emitters | 4.2 m behind (±2.4 m) | dune marram |
| `night` | curlew (2 takes, 80–240 s) at 117 m left-front; bell buoy (2 takes, 40–110 s) at 450 m right | far | rare |

Environment: 01:00, summer, 14 °C, wind 5 m/s, `ground_z` −0.85 (seated). All sources sit above the ground
plane: sea sounds put under it (z −1.5…−2, where the real sea surface would be) were attenuated by the ground
effect by 10+ dB at 200 m and more beyond; they are now at z −0.75…−0.3.

## Levers
| Key | Title | Default | `lively` | Drives |
|---|---|---|---|---|
| `fire` | Fire | 0.55 | +0.5 | `fire.level` −30…+2 dB; `flames.level` −40…0 dB (in from 0.35); `embers.level` 0…−9 dB (the embers stand out as the fire falls); `wood.events` ×0.15…1.6 |
| `surf` | Surf | 0.4 | +0.4 | `sea.level` −8…+4 dB; `sea.events` ×0.45…1.7; `sea.distance` ×1.3…0.72 (the tide brings the waterline in) |
| `wind` | Wind | 0.25 | +0.6 | `wind.level` −30…+4 dB; `grass.level` −30…+4 dB; `flames.level` +0…3 dB (wind feeds the fire); `wood.events` ×1…1.4 |
| `night` | Night life | 0.4 | −0.2 | `night.events` ×0…2.2; `night.level` −6…+2 dB |

## Episodes (`only: true`, `start: "kindling"`, free run)
| Episode | wet / energy / drama | after / then | Stretches (minutes: levers) |
|---|---|---|---|
| `kindling` | −0.2 / 0.4 / 0.4, weight 0.6 | after before_dawn, low_embers | kindling catches 3 (ease 60 s): fire 0.3 · the fire takes 5: fire 0.7 · a good blaze 6–10: fire 0.85 |
| `steady_fire` | −0.4 / −0.2 / 0 | — | a steady fire 12–25: fire 0.6 |
| `low_embers` | −0.6 / −0.8 / 0 | after steady_fire, kindling, breeze, rising_tide | the fire burns down 6: fire 0.32 · low embers 12–22: fire 0.2, surf 0.3, night 0.3 |
| `rising_tide` | 0.3 / 0.5 / 0.4 | after steady_fire, low_embers, kindling, breeze | the tide turns 6: surf 0.58 · high water 10–18: surf 0.85, wind 0.35 · the tide eases 6: surf 0.55 |
| `breeze` | 0.7 / 0.6 / 0.6 | after steady_fire, rising_tide, kindling | a breeze off the sea 5: wind 0.6, fire 0.7 · breezy 8–14: wind 0.78, fire 0.75, surf 0.62 · the breeze drops 5 |
| `before_dawn` | −0.3 / −0.6 / 0.3, weight 0.5 | after low_embers, steady_fire; then kindling | the fire dies down 8: fire 0.22 · embers before dawn 12–20: fire 0.06, surf 0.3, night 0.8 |

Steer: weather toward wet favours the breeze and the tide; energy lifts fire, surf and wind through `lively`;
change favours the breeze and dawn. Planner (`--example direction`, 8 h, default seed): before_dawn 26 %, low_embers
24 %, steady_fire 18 %, kindling 18 %, rising_tide 14 %; with weather 0.6 / energy 0.6 the tide and the breeze
take most of the night.

## Calibration (renders, levers at default)
Each group's own contribution was measured by rendering the scene with and without it (same seeds, the group
muted in place) and subtracting the takes:

| Group | Level (dBFS, 150 s) | Re the surf |
|---|---|---|
| sea | −28.4 | 0 |
| fire (crackle) | −30.0 | −1.6 |
| wind | −37.5 | −9.1 |
| grass | −39.0 | −10.6 |
| embers | −39.5 | −11.1 |
| wood (one-shots, averaged) | −47.2 | rare events |
| night (averaged over 300 s) | −48.1 | rare; while sounding (2 s windows) 7–14 dB under the rest of the mix |

Findings that changed the design: crackle loops in class `texture` share the Wind class with the wind model,
so the mix's texture budget kept the fire and cut the wind by ~20 dB; the fire layers are class `event`
(SourceKind Fire) and the wind is heard. Far sounds at ground level lose 10–20 dB to the ground effect.

10-minute render with the direction and a slow head turn: peak −3.4 dBFS, RMS −29.6 dBFS, 30 s windows within
−28.4…−31.2 dBFS (no silence, no pumping), 10 ms RMS p99 only 9 dB over the median (no startling pops). Loop
seams: every loop is cyclically filtered (el2 loops) or cross-faded (real recordings), seam step within 3× the
median sample step. Event rate: ~8 waves/min (the swell rhythm, part of the bed), wood ~1.2/min, night ≤ 1/min.

Reference comparison of every generated or procedural sound: see `campfire_shore-sources.md`.

## Stage
`ClipSet.beach`: sand from the dunes to a waterline 14 m ahead curving away to both sides, the sea beyond to
the horizon with two lines of foam, the driftwood seat under the listener, pebbles, three dune crests behind
with marram on the nearest; stars 0.75. Props: a stone ring with crossed driftwood (`campfire`), see-through
flame tongues that grow with the level (`flames`), rising sparks (`sparks`), the coal bed (`embers`), a log
(`driftLog`), steam wisps (`steam`), wave crests (`sea`) and breaking waves travelling with their pass
(`surf`), marram tufts swaying (`duneGrass`), a curlew (`curlew`), the bell buoy (`buoy`), wind streamers.
