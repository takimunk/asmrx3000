# Cave (`cave_drips.json`)

Date: 2026-10-24. Status: built (scene, levers, direction, stage, Russian). Sounds: real CC0 field recordings,
see [cave_drips-sources.md](cave_drips-sources.md). The JSON is written by `tools/synth/build_cave_drips.py`
(edit there and rerun); the WAVs are cut by `tools/synth/cave_drips_sounds.py`.

## Premise

You sit on a dry ledge deep in a limestone cave, a few metres above the floor of a large hall. Water drips
everywhere, at every distance and height: plinks into still pools on the floor below, ticks on rock beside you
and up on shelves in the vault, and one slow drip by your right knee, falling every six and a half seconds
into a little basin in the ledge. Every drop rings out in the hall. An underground stream murmurs along the
floor below on the right. Far behind you the passage runs back to the entrance; a faint draught comes down it.
Now and then a few small stones trickle down a slope. No speech, no music.

## Spatial idea: a dome of drips

Engine frame: head at the origin, +x forward, +y left, +z up.

| What | Where | Why |
|---|---|---|
| ledge (you) | top at z -0.9, about 2.5 x 2.7 m | the near field: the slow drip and one rock drip are within 1.5 m |
| slow drip | [0.75, -0.85, -0.92], right knee | one fixed, rhythmic anchor (6.1-6.9 s), close and low |
| pools | six on the floor (z -3.5) at 35°/5.2 m, -40°/8.5, 120°/6.5, -70°/4.0, 10°/13, -145°/7.5 | plinks from **below**, all round, front and back |
| drips on rock | the ledge [1.0, 1.1, -0.9]; shelves [-1.5, 2.6, 2.8], [-4, -3, 1], [5.5, 5.5, 1.6]; the vault [2.6, -1.8, 4.6], [-2.2, 0.4, 5.8] | ticks from **above** and at ear height: elevation is part of the dome |
| far drips | [16, -7, -3], [11, 12, -3.4], and up the passage [-11, 9, 1.5] | the hall's far end, with the longest echoes |
| stream | murmur [1, -6, -3.4] (3 emitters, 4 m spread), runnel [9, -10, -3.3] | a broad bed below on the right |
| draught | [-26, 1, 1], 3 emitters | from far **behind**, the way to the entrance |
| pebbles | slide down from [5, 5.2, 2.6] to [5.5, 4.6, -1.8] (front-left), or [-3.6, -6, 3.4] to [-3.2, -6.4, -2] (behind right), 1.8 s | the only moving sounds: a short fall in elevation |

Broadband drip transients carry the pinna cues above 4 kHz (the dry rock drops have centroids of 1.4-5.6 kHz),
so the up/down placement works; head turns resolve front and back. Every drip gets the clip `echo` (two
damped feedback delays, d and 1.618·d), with its delay set by where it is: 0.17 s for the slow drip in its
alcove, 0.19-0.41 s for pools and rock, 0.47-0.6 s with more feedback and less brightness for the far drips.
So no two places ring alike, and the far ones sound far.

## Groups, levers, episodes

Groups (panel `controls`): `slow` Slow drip, `pools` Pools, `rock` Drips on rock, `deep` Far drips,
`stream` Stream, `draught` Draught, `pebbles` Pebbles.

| Lever | Range [default] | Moves |
|---|---|---|
| **Drips** (density) | 0..1 [0.5], lively 0.3 | events of pools and rock ×0.12..1..2.4, far drips ×0.2..2, the slow drip ×0.55..1.25 |
| **Pools** (plinks vs rock) | 0..1 [0.5] | pools events ×0.1..1.8 and level -4..+1 dB; rock events ×1.8..0.25 |
| **Stream** | 0..1 [0.4], lively 0.2 | stream level -30 / -12 / 0 / +8 dB |
| **Depth** (how far in) | 0..1 [0.6] | draught +6 → -4 → -30 dB; far drips level -14..+3 dB, events ×0.3..1.5 |
| **Pebbles** | 0..1 [0.3], lively 0.3 | pebble falls ×0..1..3 |

Expected drip rate at the defaults (from the intervals): slow 9/min, pools 18, rock 12, far 4.5: about 44
a minute, mostly soft and far. "The cave is raining": about 95 a minute. "Dry spell": about 10.

Direction (`climate: false`, `only: true`, starts with `settling_in`):

| Episode | wet / energy / drama | Stretches (levers) |
|---|---|---|
| `settling_in` (first only) | 0 / -0.2 / 0.2 | the cave settles around you [6-10 min]: drips 0.42, pools 0.55, stream 0.35 |
| `drip_by_drip` | 0 / -0.3 / 0 | drip by drip [15-30]: drips 0.4-0.6, pools 0.4-0.65 |
| `dry_spell` | -0.8 / -0.6 / 0.2 | the drips thin out 5 · dry spell [12-20]: drips 0.12, stream 0.12 · the first drips return 5 |
| `rain_above` | 0.9 / 0.5 / 0.6 | rain soaks in above [4-6] · the cave is raining [10-16]: drips 1, stream 0.9, pebbles 0.6 · the water drains away [6-8] |
| `deep_silence` | -0.3 / -0.9 / 0.1 | deep silence [12-20]: drips 0.22, stream 0.15, depth 1, pebbles 0 |
| `air_from_outside` (weight 0.6) | 0.2 / 0.2 / 0.3 | air from the entrance [6-10]: depth 0.12 · the air stills 4: depth 0.5 |
| `stones_settle` (weight 0.4) | 0.1 / 0.3 / 0.6 | stones settle 3: pebbles 1 · still again 3 |

The "Fair ↔ wet" steer favours the rain above, "Still ↔ lively" the dry spell and deep silence. A 6-hour plan
(`cargo run --release --example direction -- 6 core/scenes/cave_drips.json`) moves through settling in → drip by drip
→ deep silence → rain above → … with no episode back to back; the test
`the_cave_is_directed_through_all_its_spells` checks that 24 hours visit every episode and move every lever.

## Stage

`ClipSet.cave`: the floor of the hall and its pools (water discs with rims), the stream between its banks, the
ledge under the listener with its face falling to the floor in a few strokes, the basin of the slow drip, the
vault in contours and ribs (open behind, towards the camera and the passage), stalactites above every pool and
rock drip, a few boulders, the passage floor running back. Props: `pool` (rings spread on it while a drop
sounds), `stalactite` (the slow drip: its drop falls every 6.5 s), `stalagmite` (a shelf with a crown of
spray), `pebbles` (tumbling along their fall), `stream` (a reach of water, see-through); the draught is the
usual wind streamers. Labels: drip into a pool, slow drip, far drip, drip on rock, pebbles, underground stream,
runnel.

## Calibration (renders, `cargo run --release --example render -- 600 out.wav "" core/scenes/cave_drips.json`)

* Default levers, 600 s: peak -2.9 dBFS, RMS -31.3 dBFS, 1 s RMS p10/median/p90 -38.8/-35.3/-30.5 dBFS (the
  stream and draught keep it from ever going silent), DSP load 3.7 %.
* Against six real cave recordings (table in the sources file): 52 onsets/min (refs 4-71), drips +14.5 dB over
  the floor (refs +12..+26), tail T60 4.35 s (refs 3.4-6.3), early/late -6.6 dB (refs -6.1..+3.5), tail ripple
  1.7 dB (refs 1.1-2.0): the engine echo reads like the real halls, not like a flutter.
* `clip_groups` (40 s, every other group at -30 dB): every group's level, events, distance and pitch change the
  output (e.g. pools 0 dB -43.5, events ×0 -66, ×3 -40; stream 0 dB -38, -30 dB silent); motion only affects
  the pebbles' pass (the rest are fixed). The draught sits 12 dB under the stream at the default depth.
* Lever states, 300 s each: the cave is raining -29.5 dBFS RMS, dry spell -33.5, deep silence -32.1 (floor -51:
  the quietest bed), near the entrance -31.1.
