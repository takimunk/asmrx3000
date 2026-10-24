# Yacht at anchor (`sailboat_anchor.json`)

Date: 2026-10-24. Status: built. Scene id `sailboat_anchor`, title "Yacht at anchor" ("Яхта на якоре").
Sounds and licences: [sailboat_anchor-sources.md](sailboat_anchor-sources.md). The JSON is written by
`tools/synth/build_sailboat_anchor.py` (the roll and swing paths are computed there): edit the builder,
not the JSON.

## Premise
You lie in the port berth of a small sailing yacht (about 10 m) anchored in a quiet bay at night. The hull
skin is a hand's breadth from your left ear and the water laps and clucks against it, a little below ear
level; across the cabin the starboard side answers more softly. The boat rolls slowly in a low swell. Above
you a loose halyard ticks on the mast, which goes down through the cabin to the keel just ahead of you. At
the bow the snubber line creaks on its cleat, and now and then the anchor chain grumbles over the roller and
along the sand as the boat swings. Out in the bay: other boats' halyards far off across the water, a bell
buoy at the harbour mouth, at first light a gull. No speech, no music, no engines.

## Spatial idea
The listener lies in the boat, so the scene is in the boat's frame: hull, mast, chain and cleat stay put,
and the water and the bay move.

- **Roll, near field.** The lapping on each side follows a looped `path` (300 s, 52 rolls of ~5.8 s,
  amplitude breathing in swell groups). The side that rolls down rises by a few centimetres and gets up to
  ~3 dB louder, the other side falls back, so the wash sways from ear to ear at the roll period. Measured on
  a render: the left–right level difference peaks at 0.174 Hz (5.7 s).
- **Swing, far field.** The moored boats, the bell buoy and one gull sit on a second looped path (720 s): the
  boat's yaw at anchor of about ±25°, so the bay wheels slowly around the berth over minutes.
- **Vertical.** Water just below the ear and under the bilge (−1.25 m), cabin joinery beside and above, the
  halyard on the mast from 0.4 m (ticks carried down the mast) up to 4.5 m (clinks through the hatch), the
  rig hum at 7 m, the chain forward and down to the seabed at −4.5 m.
- **Through the hull.** `environment.medium: "water"` with `muffle_hz: 2600` only low-passes the procedural
  models (wind in the rig, the open water), as in the lantern room: the outdoors is heard through the deck
  while the recorded clips stay close and crisp.

Frame: head at the origin, +x to the bow, +y to port, +z up. Centreline y = −0.9, port skin y = 0.62,
starboard skin y = −2.4, waterline z = −0.35, deck 0.55, keel −1.3, mast at (1.2, −0.9), bow x = 5.0.

## Groups

| Group | Clips | Where | Kind |
|---|---|---|---|
| `hull` | cabin lapping (field recording), port path, starboard path, under the bilge (2 emitters) | 0.7 m, 2.4 m, 1.6 m | loop, bed |
| `slap` | slap loop (port path), clucks port and starboard (paths) | port skin, starboard skin | texture + events |
| `boat` | cabin joinery creaks (2 places), snubber rope creaks at the bow cleat | 0.65 m, 2.7 m, 4.7 m | events |
| `halyard` | taps carried down the mast, clinks up the mast | mast 0.4 m and 4.5 m up | events |
| `rigging` | wind models: `fence_wire` in the shrouds (7 m up), `breeze` over the deck | above | models |
| `gust` | halyard rattling in a gust; moored boats' rigging jingling in a fresh wind (swing path) | mast 3 m; 110 m | events + texture |
| `chain` | chain grumble over the roller; the same further out and down, slower | bow; 14 m ahead, 4.5 m down | events |
| `bay` | water model `lake_lapping` (sand bottom, small waves) | 4.5 m ring at the waterline | model |
| `harbour` | other boats' halyards in a calm port (two loops, swing paths) | 125 m port-aft, 175 m starboard | texture |
| `buoy` | bell buoy (field recording, 3 takes, swing path) | 320 m, harbour mouth | events |
| `gulls` | gull calls: one passing over, one on the swing path | 30–150 m | creature |

## Levers

| Key | Title | Default | Drives |
|---|---|---|---|
| `swell` | Swell | 0.3 | hull level −5..+4 dB, slap level −30..0 dB and events 0..×2, creaks ×0.3..×2, bay −12..+3 dB |
| `wind` | Wind | 0.15 | rigging −30..+2 dB, gust level −30..0 dB and events 0..×2, halyard events ×0.8..×1.6 |
| `rigging` | Rigging | 0.35 | halyard events 0..×2.5, level −8..+3 dB |
| `harbour` | Harbour | 0.35 | moored boats −30..+4 dB, buoy events 0..×1.8 and level −6..+3 dB |
| `tide` | Tide | 0.15 | chain events 0..×2.6 and level −6..+3 dB, creaks +0..2 dB |
| `dawn` | Dawn | 0 | gull events 0..×1.6, level −8..0 dB |

The roll and swing are written into the paths and are not on a lever (a `motion` lever would drift the
path clock).

## Episodes (`only: true`, `start: "still_night"`, free run)

| Episode | wet / energy / drama | Stretches (lever targets; the rest go to their defaults) |
|---|---|---|
| `still_night` | −0.5 / −0.7 / 0 | "a still night at anchor" 15–30 min: swell 0.25, wind 0.1, rigging 0.3 |
| `flat_calm` | −0.8 / −1 / 0 | "flat calm" 12–25: swell 0.1, wind 0, rigging 0.12, harbour 0.25, tide 0.05 |
| `breeze` | 0.5 / 0.6 / 0.4 | "a breeze picks up" 5 · "a fresh night breeze" 8–15 (wind 0.8, swell 0.65, rigging 0.75, harbour 0.5, tide 0.4) · "the breeze dies away" 6 |
| `tide_turns` | 0 / 0.1 / 0.7 | "slack water" 4 · "the boat swings to the tide" 5–9 (tide 1, ease 25 s) · "lying to the new tide" 6–10 |
| `dawn` | −0.2 / 0.3 / 0.4, weight 0.4 | "first light" 6 · "dawn over the bay" 8–14 (dawn 1, wind 0.25, harbour 0.55) · "morning settles" 6; then still_night |

`--example direction -- 12 core/scenes/sailboat_anchor.json`: still night 31 %, flat calm 25 %, tide turns
19 %, breeze 12 %, dawn 12 % (about once in two hours; clip scenes have no clock, so dawn is a spell).

## Calibration (renders, `cargo run --release --example render`)
- **Against the field recording.** The bed is built on a real recording made in a sailboat cabin at anchor
  (Freesound 851577). The same short-term statistics on that recording and on a 15-minute render of the
  scene at its defaults: L10 − L50 3.4 dB vs 3.3 dB; noticeable rises (100 ms level 6 dB over the running
  10 s median) 5.8/min vs 4.9/min; energy within 3 dB of the running median 76 % vs 75 %. The first
  renders had 8–13 rises a minute and 60–67 % bed share (halyard every 3–11 s, clucks every 4–18 s, roll
  swing ±60 %); event intervals and the swing were cut to these numbers.
- **Events against the bed** (5 s mix levels, each family alone over the beds, cued every 12 s):
  clucks −9 dB, creaks −9, snubber −11, mast taps −13, clinks −10, gust rattle −6 (only in wind), chain −9,
  far chain −12, bell buoy −13, gulls −10. The bell buoy first measured +9 dB over the bed at the
  distance-law level (its file is mostly ringing gaps, normalised by RMS), now 87 dB at 1 m.
- **Wind lever**: the rig and gust layers sit 25 dB under the bed at the default 0.15, −17 dB at 0.5, −9 dB
  at 1. Harbour: −18 dB at 0.35, −13 dB at 1.
- 15 min render: peak −2.9 dBFS, no silence after the 0.4 s fade-in, DSP load ~7 %.
- Loops: every loop file is crossfaded into itself (seam step ≤ 0.26× the file's 99th-percentile sample
  step); the engine also starts loops at random offsets.
- `--example clip_groups`: every group answers its level and distance; event groups answer `events` when
  their events fall in the 40 s window (rare ones were checked with the family renders above).

## Stage
`ClipSet.yacht`: the hull in pale lines (sheer, waterline, bilge, transom, keel and fin), the berth under the
listener and the cabin sole, the coachroof with its portlights, the mast through the cabin with spreaders,
shrouds, forestay, backstay and boom, the chain from the bow down to the anchor on the sand, the bay's water
with low swell lines, a low shore round three sides and the open harbour mouth where the buoy is; a few stars.
Camera over the starboard quarter. Props: wavelets at the hull (lapping, slaps), a splash ring (clucks), a
joinery knee (creaks), a horn cleat with its line (snubber), a stretch of mast with its swinging halyard
(taps, clinks, rattles), chain links over the roller, moored yachts rocking (harbour), a gull with beating
wings, the existing bell buoy, wind streamers in the rig, wave crests for the open water.
