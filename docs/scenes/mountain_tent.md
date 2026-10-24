# Tent in the rain (`core/scenes/mountain_tent.json`)

Russian title: «Палатка в дождь». Sources and licences: [mountain_tent-sources.md](mountain_tent-sources.md).

## Premise

It is night, and you are lying in a small two-pole dome tent on an alpine meadow. Rain patters on the
nylon fly sheet just above and around your head. Heavier drops fall from the fly's hem on either side
into the wet grass. In gusts the fly flaps and the poles creak. A mountain stream runs past about 14 m
to the left. Thunder rolls across the valley ahead, several kilometres away. Close by, the sleeping
bag rustles when you shift, and now and then someone works the door zip.

The spatial idea is a small shell of sound right over you, with the big world outside it. The
canopy, the hem drips, the flapping panels, the poles and the bag all sit within 0.3–1.2 m of the
head. The meadow rain, the stream, the wind on the ridge and the thunder lie at 7 m, 14 m, about
30 m and 2–3 km.

## Spatial layout (engine frame: +x forward, +y left, +z up, head at the origin)

The tent's floor is at z −0.3, and the listener's head lies about 0.25 m above it. The tent's middle
is at [0.4, 0], the fly's apex at z +0.8, and its hem at about ±1.15 m to the sides. The door and
its vestibule are on the right (−y), and the stream is on the left (+y).

| Group | id | Source | Where | How |
|---|---|---|---|---|
| `rain` | 10, 11 | real: rain on a tent fly, light (tent_fly_light_1..2) | a ring of 3 overhead (centre [0.3, 0, 0.62], r 0.45 m) and a ring of 4 at the walls ([0.3, 0, 0.05], r 1.0 m) | loops, bed |
| `rain` | 12, 13 | real: steady rain on a small tent (tent_fly_steady_1..2) | the same two shells | loops, bed |
| `downpour` | 14, 15 | real: heavy rain on a tent (tent_fly_heavy_1..2) | the same two shells | loops, bed |
| `meadow` | 16 | model rain `on_soil`, area 80 m² | 6 emitters on a 7 m ring | bed |
| `drips` | 20, 21 | real: drops off foliage onto grass (tent_drips_1..2) | at the hem: [0.45, 1.15, −0.3] and [0.15, −1.15, −0.3] | loops, bed |
| `wind` | 30 | model wind `breeze`, exposure 0.6 | 3 emitters, 12 m spread around [−5, 9, 0.5] | bed |
| `gusts` | 31 | model wind `storm`, exposure 1.2 | 3 emitters, 15 m spread around [−20, 25, 4] (the ridge) | bed, faded in by the Wind lever |
| `fly` | 32, 33 | real: tent fabric flapping (tent_flap_1..4) | on the fly, left [0.45, 0.85, 0.3] and right [0.6, −0.85, 0.3] | events every 25–70 s and 30–80 s, ×0..2.2 by Wind |
| `fly` | 34, 35 | generated: tent pole creak (tent_pole_creak_el2_1..4) | at the poles' feet, ±0.95 m | events every 45–120 s |
| `storm` | 40, 41 | real: distant rolling thunder (tent_thunder_roll_1..3) | across the valley: [2600, 900, 600] and [1900, −2300, 500] | events every 60–150 s and 70–170 s, 116 dB at 1 m |
| `stream` | 50 | model water `brook_rocks`, gain 0.1 | 4 emitters, 5 m spread around [3, 14, −0.8] | bed |
| `stream` | 51 | real: alpine creeks (tent_creek_1..2) | 3 emitters, 6 m spread around [−3, 16, −0.8] | loops, bed |
| `tent` | 60 | real: sleeping bag and nylon rustle (tent_bag_rustle_1..4) | beside the left shoulder, [0.15, 0.28, −0.2] | events every 50–140 s |
| `tent` | 61 | real: tent zipper (tent_zipper_1..3) | the door, [0.6, −0.75, −0.05] | events every 300–600 s |

Rules from `docs/research/focus-and-sound.md` that the layout follows:
- **The bed is the rain on the fly.** It is stationary, broadband and close, and it holds most of
  the energy.
- **Close sounds stay soft.** The sleeping bag and the zipper are rare and sit under the rain.
- **Nothing moves through the head.** All sources are fixed. Motion comes from the listener's own
  head turns and from the gusts.
- **The sky is in front.** The thunder is far ahead, across the valley, and never close.

## Levers

| Key | Title | Default | `lively` | Drives |
|---|---|---|---|---|
| `rain` | Rain | 0.35 | +0.5 | `rain.level` [0 −40, 0.08 −18, 0.35 0, 0.7 +2]; `downpour.level` [0.55 −40, 0.75 −10, 1 0]; `meadow.level` [0 −40, 0.1 −16, 0.5 −4, 1 +2] |
| `drips` | Drips | 0.4 | +0.2 | `drips.level` [0 −40, 0.15 −14, 0.5 −4, 1 0] |
| `wind` | Wind | 0.25 | +0.6 | `wind.level` [0 −30, 0.25 −6, 0.6 0, 1 +2]; `gusts.level` [0.35 −30, 0.7 −10, 1 −2]; `fly.events` [0 0, 0.2 0.25, 0.5 0.9, 1 2.2]; `fly.level` [−6..+3] |
| `thunder` | Thunder | 0 | +0.2 | `storm.level` [0 −40, 0.3 −10, 1 0]; `storm.events` [0 0, 0.3 0.5, 1 1.4]; `storm.distance` [0 ×2, 1 ×1] |
| `stream` | Stream | 0.5 | +0.1 | `stream.level` [0 −30, 0.5 0, 1 +4]; `stream.distance` [×1.6 .. ×0.8]; `stream.a` (the brook model's flow) [×0.6 .. ×1.35] |
| `tent` | Tent | 0.4 | −0.2 | `tent.events` [0 0, 0.4 1, 1 2.2]; `tent.level` [−4 .. +2] |

Drips have a lever of their own because they outlast the rain: in the clearing the Rain lever is at
0 while the hem still drips.

## Episodes (`"climate": false, "only": true, "start": "drizzle"`, free run)

| Episode | wet / energy / drama | after | Segments (minutes: levers) |
|---|---|---|---|
| `drizzle` | −0.2 / −0.6 / 0 | — | "drizzle on the fly" [10, 20]: rain 0.18, drips 0.3, wind 0.15, stream 0.45 |
| `steady_rain` | 0.4 / −0.1 / 0.1 | drizzle, clearing, night_wind | "the rain sets in" 5: rain 0.4, drips 0.5 · "steady rain" [12, 25]: rain 0.55, drips 0.7, stream 0.6 |
| `passing_storm` | 1 / 0.7 / 1 → clearing | steady_rain, night_wind | "thunder over the valley" 4: rain 0.6, wind 0.45, thunder 0.3 · "the storm passes over" [4, 7] (ease 25 s): rain 0.95, wind 0.8, thunder 0.85 · "thunder moving off" 6: rain 0.5, thunder 0.35 |
| `clearing` | −0.6 / −0.7 / 0.1 | passing_storm, steady_rain, drizzle | "the rain stops" 4: rain 0.05, drips 0.75 · "only the drips" [10, 18]: rain 0, drips 0.55, stream 0.5 |
| `night_wind` (weight 0.6) | 0.1 / 0.4 / 0.4 | drizzle, steady_rain, clearing | "wind off the ridge" 4: wind 0.55 · "gusts on the fly" [6, 12]: wind 0.75 · "the wind drops" 4: wind 0.3 |

The planner (`cargo run --release --example direction -- 6 core/scenes/mountain_tent.json`) runs
through every episode in 6 hours. Share of time: steady rain 39 %, clearing 25 %, drizzle 18 %,
night wind 9 %, passing storm 9 %. Every storm is followed by a clearing.

## Checked against field recordings

Every recorded sound in the scene is a real field recording. The procedural and generated parts were
compared with CC0 recordings of the same things, using the features of `ml/asmr_ml/refcompare.py`
(1/3-octave bands re total, centroid, envelope modulation, onsets, crest).

- **Meadow rain** (outside the tent). The engine's `meadow` preset was much brighter than real rain
  on the ground (freesound 530069, 865323): its centroid was 4.4 kHz against 2.9–3.4 kHz, and its
  500 Hz band was 13–15 dB low. The `on_soil` preset matches within 3 dB from 500 Hz to 8 kHz
  (centroid 2.6 kHz), so the scene uses `on_soil` with an 80 m² patch. Both presets are smoother
  than the recordings: crest 1.5 dB against 3.2–3.6 dB, with no resolved onsets. At 7 m and −10 to
  −16 dB under the fly, that does not matter.
- **Drips at the hem.** The water model `drips` was dropped. Its drops into a pool are pure plinks:
  the 1 kHz band stands 15–20 dB over its neighbours, flatness is −30 dB and events last 20 ms. The
  references (637774, 452296, drips onto roofs, wood and ground) are flat from 250 Hz to 8 kHz.
  freesound 865323 (drops on grass and ferns, onset rate 0.7/s, flat 500 Hz–8 kHz) is used instead.
- **Stream.** The model `brook_rocks` against three alpine creeks (423623, 869104, 572464): its bands
  from 250 Hz to 4 kHz are within 1–3 dB. It is more eventful than the recordings, with 4.2
  onsets/s against 0.05–0.5 and 4–16 Hz modulation 4–10 dB stronger. The real creek recordings
  therefore carry the stream, and the model stays 10 dB under them (gain 0.1) for its width.
- **Thunder.** The models `distant_rumble` and `rolling` against TRP's three distant rolls (577315,
  717890, 572443): centroid 212–222 Hz against 114–122 Hz, and the 250 Hz band 6–12 dB higher. For
  thunder 2–3 km away across a valley that is too bright, so the scene plays the real rolls instead.
- **Wind.** The model `breeze` against wind in grass (517995). From 250 to 500 Hz they agree within
  1.5 dB. Above 1 kHz the model is 6–10 dB brighter than this rather dark recording. A strong
  plains wind (167684) is far darker than both. The model is quiet in the mix (−10 dB at the default
  Wind) and sits at 10–30 m.
- **Pole creak** (el2). The only generated sound. No free recording of a creaking tent pole was
  found. Against the closest one, Kranhytt 672691 (a tarp tent on a metal frame in wind, CC BY 4.0,
  used only as a reference): the reference's frame sounds last 0.4–1.5 s, with a centroid of
  1.0–4.0 kHz. The el2 takes ran 3–3.7 s with a centroid of 1.7–2.6 kHz, so each take was cut to
  1.4 s around its loudest moment.
- **Rain on the fly.** The six loops come from five different tent recordings, all of them real.
  Their centroids are 1.5–2.3 kHz, and band shapes agree within 5–8 dB from 250 Hz to 4 kHz. The
  light takes have 6–10 onsets/s and the heavy ones are a dense roar. "Regen in de tent" (CC BY)
  was rejected: it is about 8 dB darker at 8 kHz.

## Calibration (offline renders, `scratch` probe; group levels are solo RMS at the ears, dB re full scale)

| State (levers) | Rain on the fly | Drips | Stream | Meadow | Wind + gusts | Fly | Thunder | Tent |
|---|---|---|---|---|---|---|---|---|
| drizzle (0.18 / 0.3 / 0.15 / 0 / 0.45) | −31.7 | −36.3 (−4.6) | −39.7 (−8.0) | −47.8 (−16) | −47.7 | — | — | −43.4 (−11.7) |
| steady (0.55 / 0.7 / 0.25 / 0 / 0.6) | −27.0 | −30.7 (−3.7) | −35.0 (−8.0) | −37.5 (−10.5) | −40.2 | — | — | −39.0 (−12) |
| storm (0.95 / 0.9 / 0.8 / 0.85 / 0.75) | −26.9 (+ downpour −27.0) | −30.2 | −33.5 | −34.6 | −34.7 / −32.8 | −32.2 (−5) | see below | −38.5 |
| only the drips (0 / 0.55 / 0.08 / 0 / 0.5) | −49.9 | −31.9 | −38.1 (−6.2) | −65 | −55 | — | — | −50 |

The figures in brackets are relative to the rain on the fly (in the clearing, to the drips). The fly,
tent and thunder are event groups, so their values are averages over 90 s that include silence.
During a flap the fly is at about the rain's level. A bag rustle is a few dB under it.

After these runs, the drips lever was set 1 dB lower in steady rain and the thunder rolls 4 dB
higher, so that a roll stands at about −2 dB under the rain during the storm. Peak output was
−2.9 dBFS (the output limiter) in every state, with no clipping and no silence: the 1 s RMS never
fell below −35 dBFS.

Lever sweeps. Each row is the solo RMS of the groups the lever drives (the others at −30 dB), 80 s
renders, after the trims above. The floor of everything at −30 dB is about −47.

| Lever | 0 | 0.25 | 0.5 | 0.75 | 1 |
|---|---|---|---|---|---|
| Rain (rain + downpour + meadow) | −48.2 | −28.9 | −26.9 | −26.9 | −27.0 |
| Drips | −47.2 | −37.3 | −32.8 | −31.5 | −30.2 |
| Wind (wind + gusts + fly) | −47.1 | −43.6 | −35.8 | −31.0 | −28.8 |
| Thunder (storm) | −47.2 | −47.2 | −46.0 | −41.3 | −36.1 |
| Stream | −47.3 | −46.8 | −37.7 | −33.7 | −31.6 |
| Tent (bag + zipper) | −47.2 | −42.8 | −41.7 | −40.3 | −40.8 |

Above 0.5, Rain changes the timbre more than the level. The steady loops cross-fade into the
downpour, and the comfort mix holds the total. Thunder below 0.3 is silence by design. Tent changes
how often the bag sounds, not how loud it is.

A 10-minute render with the direction running (`--example render -- 600 out.wav levels
core/scenes/mountain_tent.json`; it opens on drizzle, then the clearing) peaked at −2.9 dBFS, with
an RMS of −31 dBFS. Every minute stayed between −29 and −34.5 dBFS, and L/R correlation was 0.23.

Loops: every loop wraps with a sample jump no larger than the file's median sample step
(0.4–2.3×), and the head and tail levels are within 2.3 dB of each other.
