# Five new scenes: concepts ready to build

Date: 2026-10-23. Status: concept. Nothing here is built yet.

Each scene is described so that a builder can write the scene JSON, the `prompts_v2.json` entries and the
direction block without guessing. Coordinates use the engine frame: head at the origin, **+x forward**
(the screen), **+y left**, **+z up**, metres. Every source is world-fixed: when the head turns, the
AirPods tracking keeps it in place.

| # | Scene (file) | Premise in one line | Spatial idea | Session |
|---|---|---|---|---|
| 1 | **Body scan** (`body_scan.json`) | One soft point of sound travels around and through your body for 5 minutes | Body-scale path: head, inside the head, down the arms and legs, a twin on the second leg | defined, 5:00 + rest |
| 2 | **Lantern room** (`lighthouse_gale.json`) | Night watch at the top of a lighthouse in a gale | Vertical: sea 32 m below, copper dome and vent overhead, the lens turning beside you, weather from one side | long listening |
| 3 | **Night train** (`night_train.json`) | Lower berth of a sleeper car, 100 km/h through the night | Linear flow: crossings Doppler past, bogies under the floor front and back, tunnels close a ring around you | long listening |
| 4 | **Tea house** (`tea_house.json`) | A tea ceremony in a small tea room while it rains in the garden | Near-field hands in front of you, garden on one side, a water jar singing underground, rain on the roof above | scripted 19.5 min, loops |
| 5 | **Singing ice** (`singing_ice.json`) | A windless night on a frozen lake whose ice sings | A vast horizontal plane: chirps and cracks from every azimuth up to 300 m, a crack that runs underneath you | long listening, sleep |

None of the five uses speech or music. All of them rely on what the engine already has, plus the `path`
motion and scene levers that are being added now. The few small engine needs are listed in
[section 9](#9-engine-needs).

---

## 1. What the designs rely on in the engine

Read from `core/src/clips.rs`, `scene.rs`, `direction/catalogue.rs`, `propagation.rs`,
`nearfield.rs`, `engine.rs`, the working-tree `asmr.h`, and `docs/direction.md` and
`docs/spatial-engine.md`.

| Fact | Consequence for design |
|---|---|
| `path` motion: `points` `[t, x, y, z]` or `[t, x, y, z, gain]`, times in seconds **from the scene start**, Catmull-Rom between points, gain linear between points, held at the last point or looped with `repeat`. Continuous, so it is present from the start. Its clock advances at `dt × motion` (group `motion` param). | The body scan is one path. Timbre changes are made by **several clips on the same path**, each with its own gain column, so they cross-fade (no per-keyframe filter is needed). Never put `motion` of a path group on a lever, or the path would drift away from the script labels. |
| Catmull-Rom uses uniform parameters per segment, not time. | Keep keyframes roughly evenly spaced in time (2–4 s) inside a stage. For a hold, move the point by 0.5–1 cm instead of repeating it exactly: a repeated point with non-zero neighbouring tangents makes a small overshoot loop. |
| `clip_env = env × level` is clamped to 0..4 in `ClipPlayer`. | Path gains stay ≤ 2.2, so a +10 dB group or lever trim still fits. |
| Level ∝ 1/max(r, 0.15 m) (`propagation::R_MIN`). Near-field NFC is exact down to 0.2 m (`R_MIN_NFC`), clamped below. | Getting closer than 0.15 m does not get louder. "Inside the head" (r ≈ 0.04 m) is quiet by design. Its direction is nearly median (front-up), so both ears get almost the same signal, which is the in-head cue. |
| `first` is jittered by +0..20 % (`c.first + rng.range(0, 0.2·first)`). | A one-shot meant for second 140 can fire up to 28 s late. Timed accents late in a session need an exact cue time (section 9, need 2). A one-shot at t ≈ 1 s is fine as it is. |
| Group params: `level` dB −30..+10, `events` ×0..3 (0 stops new appearances and cues), `motion` ×0..3, `distance` ×0.3..3, `pitch` ×0.5..2 (files only), `a`/`b` for water and plasma models. | Levers switch event groups on and off with `events` 0. Continuous model clips cannot be stopped by `events`, so they are faded with `level` −30 dB plus `distance` ×3. |
| Levers (`LeverDesc`): `key, title, icon, min, max, default, log, lively (−1..1), drives:[{group, param, at:[[lever, effect],…]}]`, at most 16. `level` effects are dB added, `b` added, the rest multiplied. | All levers below are written in this schema. |
| Direction in clip scenes: `"climate": false`, only the levers move. Segments carry `"levers": { "<key>": value or [lo, hi] }`; **levers a segment leaves out go back to their default**; the transition takes ~40 s unless the segment sets `ease`. | Every scene sets `"climate": false, "only": true`, so the built-in forest weather does not appear in the cue list. Lever defaults are chosen as the "resting" state, so segments only name what they change. |
| `environment.medium: "water"` low-passes **only the physical models**, not clips (`engine.rs`, `em.base.is_some()`). | Lantern room: all procedural outdoor sounds (wind, sea, rain on the dome, thunder) can be heard "through glass" with `muffle_hz: 2800`, while the interior clips stay crisp. It is a semantic hack; renaming it is a nicety (need 5). |
| Wind/rain models in a clip scene read the static environment (`wind_ms`, `rain_mm_h`). Wind `exposure` scales the local speed (0..2.5). | A rising gale is two wind clips with different `exposure` cross-faded by a lever. This gives a real change of spectrum, not only of level. |
| No room reverb for clip scenes (the reverb is the forest's trunks and the slab). Clips have `echo` (two damped feedback delays). | Indoor rooms are recorded into the clips ("small wooden room", "close-mic") or get a short `echo`. A shoebox reverb would be nice to have (need 4), but no design depends on it. |

**Levels.** `level_db` is SPL at 1 m. At the listener, L ≈ level_db − 20·log10(r) (plus air absorption).
Each scene gives `level_db` and the intended level at the listener relative to its bed. The comfort mix
normalises the total, so the **ratios** matter. Tune by ear after the first render.

---

## 2. Research digest

Confidence tags follow `docs/research/focus-and-sound.md`: **[V]** checked in the source or its abstract
during this review, **[L]** known from the literature but not rechecked now, **[I]** my inference.

### 2.1 ASMR: what triggers it and why
- Barratt & Davis 2015 (PeerJ 3:e851, n = 475) is the first survey. The most common triggers are
  **whispering, personal attention, crisp sounds and slow movements**. Users report a calm, flow-like
  state and temporary relief of low mood. [V, abstract]
- Barratt, Spence & Davis 2017 (PeerJ 5:e3846, n = 130): **lower-pitched, complex** sounds work better,
  as do slow, detail-focused actions. Intimacy and closeness matter. [V, abstract]
- Poerio et al. 2018 (PLOS ONE 13:e0196645): in the lab, heart rate fell about 3.1 bpm more in ASMR
  experiencers than in controls, while skin conductance rose: relaxation and engagement at the same time.
  [V]
- Smith, Fredborg & Kornelsen 2017 (Soc Neurosci 12:361) found weaker default-mode connectivity in ASMR
  experiencers (n = 11 + 11). Fredborg, Clark & Smith 2018 (PeerJ) relate ASMR to trait mindfulness.
  [V, citation; findings L]
- **Design reading [I]:** speech and whisper are out for this app, because they are semantic and they
  distract (focus-and-sound §1). What remains are crisp, close, slow, low and complex sounds, **personal
  space** (a sound 0.3–1 m away whose source is "for you"), and **slow binaural movement**. All five scenes
  build on these.

### 2.2 Following a moving sound
- Minimum audible movement angle: the best resolution is at **slow speeds, about 2°/s horizontally**, and
  thresholds grow to about 10° at 320°/s. Vertical motion is resolved worse, best at 7–11°/s (Saberi &
  Perrott 1990, JASA). A source that **sounds throughout its travel** is tracked better than one marked
  only at its end points (0.9° vs 1.6°; Perrott & Marlborough 1989). [V, abstracts]
- Elevation relies on pinna spectral cues above ~4–5 kHz, so a **broadband** source is needed. Pure tones
  are hard to place up and down, and front–back confusion is common for narrowband sounds (Middlebrooks &
  Green 1991, Annu Rev Psychol). [L]
- Head movements remove most front–back confusions (Wightman & Kistler 1999, JASA; their 1994 ASA
  abstract shows listeners with many reversals losing nearly all of them once head motion was allowed).
  Externalisation improves when the source moves **relative to the head** because of the listener's
  motion; head-locked sources collapse inward (Brimijoin, Boyd & Akeroyd 2013, PLOS ONE). [V, abstract and
  secondary]
- Near field (< 1 m): the low-frequency ILD grows steeply as a lateral source approaches (Brungart &
  Rabinowitz 1999, JASA). The engine reproduces it (9.9 dB at 0.3 m vs 4.7 dB at 2 m). [V, engine doc]
- **Design rules [I]:** the moving guide must be continuous and broadband with energy up to 8–10 kHz.
  It travels at 10–15°/s around the head and slower when it moves vertically along the limbs (2–4 cm/s).
  Head turns are invited ("look at your hand with your ears"); with tracking, the body-fixed path stays
  where the body is.

### 2.3 Sound, body and peripersonal space
- A sound approaching a hand speeds up tactile processing **only once it is within a limited distance of
  the body**, and more so when it approaches than when it recedes (Canzoneri, Magosso & Serino 2012, PLOS
  ONE 7:e44306). [V, abstract]
- Audio-tactile interaction is strongest for sounds close to the head and behind it, within about 20–30
  cm (Kitagawa, Zampini & Spence 2005, Exp Brain Res). Action sounds change perceived body size and
  distance (Tajadura-Jiménez et al. 2012, Curr Biol). [L]
- **Design reading [I]:** a sound passing a few centimetres from a body part plausibly draws attention to
  that body part, which is the job of a body scan. No study tested a moving sound as a body-scan guide,
  so this is a hypothesis for the n = 1 protocol.

### 2.4 Body scan
- The body scan is the first formal MBSR practice (Kabat-Zinn 1990). The full version lasts ~45 min,
  usually from the feet upward; short 3–15 min versions are common. [L] Eight weeks of body scan improved
  interoceptive accuracy measures (Fischer et al. 2017, Front Hum Neurosci, PMC5601051). [V, abstract] A brief
  body scan reduced false tactile reports (Mirams et al. 2013, Conscious Cogn). [L]
- Typical script structure: settle, then move attention region by region (head and face, neck and
  shoulders, arms and hands, chest and breath, belly, hips and legs, feet), then widen to the whole body,
  and return. Breath anchors the torso section. Slow breathing at ~6 per minute is the usual "resonance"
  pace (Lehrer & Gevirtz 2014). [L]
- **Design reading [I]:** 5 minutes is enough for 11 regions at 15–40 s each. Novelty at each region
  boundary keeps tracking alive: a new timbre, a short stillness, a single soft accent, a split into two
  sources.

### 2.5 Calm for long listening
- Attention Restoration Theory: "soft fascination" restores directed attention (Kaplan 1995). Natural
  sounds, birdsong and water above all, are mostly rated restorative (Ratcliffe 2021, Front Psychol,
  narrative review). A meta-analysis found improved health and mood and lower stress, with water strongest
  for health and birdsong for stress (Buxton et al. 2021, PNAS). [V, review abstract; PNAS via coverage]
- The house rules for focus are in `docs/scenes/forest.md` and `docs/research/focus-and-sound.md` §
  "Правила дизайна сцен": a stationary broadband bed (~80–90 % of the energy), at most 1–2 noticeable
  events per minute, events 6–12 dB under the bed, lulls every few minutes, slow tangential motion, at
  most one gentle approach per 5–10 min, no recognisable loop shorter than ~10 min, no rotating fields.
- The long-listening scenes (2, 3, 5) follow these rules. The body scan and the tea ceremony are defined
  sessions: attention to the moving or close source **is** the task there, so they may be denser, but they
  keep the soft levels and slow motion.

---

## 3. Common to all five

- Every scene has `"direction": { "climate": false, "only": true, … }`, scene episodes, and a lever block. In clip scenes the
  direction moves only the levers.
- The steer axes map like this: **weather** (−1 fair .. +1 wet) favours episodes with `wet` > 0, which is
  each scene's "rough" axis (rain, gale, snow drift). **Energy** favours `energy`. **Change** favours
  `drama`.
- Generation: fal ElevenLabs SFX v2 (`el2`, $0.002/s, ≤ 22 s, `loop: true` for seamless beds) and Stable
  Audio 2.5 (`sa25`, $0.20 flat per take, ≤ 190 s) for long rooms. Prompt entries are written in the
  `tools/fal/prompts_v2.json` format. Every prompt ends with the house phrase "no music, no voices, dry,
  single source". Listen to each take: el2 sometimes adds music or extra sources.
- Files go to `assets/sounds/generated/<scene>/<name>_<model>_<n>.wav` (what `gen.py` writes). The
  procedural models replace clips wherever they can.

---

## 4. Scene 1 — Body scan (`body_scan.json`)

### Premise
You sit upright on a chair, hands resting on your thighs, eyes closed. A soft point of sound appears in
front of you, comes close, and travels: around your head, over the crown, into the centre of your head,
down your throat, out along the left arm to the fingertips, across your lap to the right hand, up the right
arm, to the chest and heart, to the belly where it breathes three slow breaths, then splits into two and
walks down **both** legs at once to the feet. It rises up the front of the body and leaves forward. Five
minutes. The panel shows the stage ("crown", "left arm", …).

### Why it holds attention
- One continuous broadband source, which is the easiest kind to follow (Perrott & Marlborough), moving at
  speeds close to the best resolution: 15°/s around the head, ~2.5 cm/s along the limbs.
- Each region has its own texture (brush at the head, a warm hum inside it, breath at the torso, a slow
  rainstick trickle along the limbs). The change of timbre tells you, without a word, that a new region
  has begun.
- Small surprises: a 4 s stillness at the crown, the hush of the inside, a hold at each set of fingertips,
  three breaths at the belly, **2.5 s of silence** after the belly, then the guide reappears **as two**,
  one on each hip, and walks down both legs.
- Head tracking: the path is fixed to the body. When you tilt your head down "to look at" your left hand,
  the sound stays on the hand, which convinces you it is there.

### Body geometry (seated adult, ~1.75 m, head centre = midpoint between the ear canals)
| Point | Position [x, y, z] m | Basis |
|---|---|---|
| Ear canals | [0, ±0.075, 0] | head breadth ~15 cm |
| Crown (vertex), 3–4 cm off the skin | [−0.01, 0, 0.23] | vertex ~12–13 cm above the ear axis |
| Throat | [0.05, 0, −0.10] | |
| Suprasternal notch | [0.09, 0, −0.19] | |
| Shoulder (acromion) | [0.02, ±0.19, −0.23] | biacromial breadth ~0.39 m |
| Elbow | [0.02, ±0.27, −0.52] | upper arm ~0.30 m, hanging |
| Wrist | [0.15, ±0.33, −0.60] | forearm forward and down to the thighs |
| Fingertips (resting on the outer thighs) | [0.24, ±0.35, −0.62] | brief: hands ±0.35, z −0.6, 0.2 forward |
| Sternum / heart | [0.12, 0, −0.32] / [0.13, +0.05, −0.38] | heart left of the midline |
| Navel | [0.16, 0, −0.55] | seated ear-to-navel ~0.55 m |
| Hip joints | [0.10, ±0.11, −0.74] | seat ~0.78 m below the ears |
| Knees | [0.46, ±0.12, −0.74] | thigh ~0.45 m, horizontal |
| Ankles | [0.42, ±0.12, −1.15] | shin ~0.42 m |
| Toes / soles | [0.50, ±0.13, −1.21] | floor at −1.23 (seat height 0.45) |

### Sound groups

| Group / clip | What | Motion | Playback, class | `level_db` | At listener |
|---|---|---|---|---|---|
| `guide` id 10 `brush` | soft continuous brush strokes | path, gain column "brush" | loop, texture, fade 0.5 | 46 | 54 dB at 0.4 m (reference) |
| `guide` id 11 `air` | breath-like airy whoosh | same path, "air" | loop, texture | 46 | same |
| `guide` id 12 `grain` | slow rainstick trickle | same path, "grain" | loop, texture | 44 | same |
| `guide` id 13 `warm` | warm low hum with a faint bowl partial | same path, "warm" | loop, texture | 48 | about −4 dB inside |
| `guide` id 14 `twin` | the grain texture, second file | twin path (right leg) | loop, texture | 44 | as `grain` |
| `room` id 20 | quiet room tone | fixed [−0.4, 0, 0.6], emitters 4, spread 2.5 | loop, bed, fade 6 | 34 | ~32 dB (−20 dB re guide) |
| `room` id 21 | very distant air (procedural) | `sound: {model: wind, preset: breeze, params: {distance_m: 400, exposure: 0.3, gain: 0.25}}` fixed [−30, 40, 6], emitters 3, spread 30 | bed, fade 10 | — | barely audible, below the room tone |
| `bowls` id 30 | opening bowl | fixed [1.5, 0, −0.25], `first` 1 | events, event, `interval` [99999, 99999] | 60 | ~56 dB |
| `bowls` id 31, 32 | tiny tingsha "touch" on each fingertip | fixed [0.25, 0.36, −0.62] at t 140; [0.23, −0.36, −0.62] at t 160 | events, rate [1.5, 1.6] | 46 | soft, ~ −8 dB re guide |
| `bowls` id 33 | low bowl at the feet | fixed [0.5, 0, −1.22] at t 264 | events, rate [0.8, 0.8] | 56 | |
| `bowls` id 34 | closing bowl | fixed [1.4, 0, −0.3] at t 292 | events | 58 | |

Ids 31–34 need an exact cue time (need 2). Until it exists, leave them out: the timbre changes already mark
the regions. Environment: `time_of_day` 21, `wind_ms` 2, `rain_mm_h` 0, `ground_z` −1.23 (the floor
reflection gives the feet a real floor under them).

**Gain rule.** Per-point gain = g_dist × g_layer × g_fade × g_inside × g_breath, where
g_dist = clamp((max(r, 0.15)/0.4)^0.8, 0.5, 2.2). This keeps the level within about ±3 dB of the 0.4 m
reference from the crown (r 0.23) to the soles (r 1.32), and still leaves a little distance cue. g_inside
falls to 0.45 from 78 to 88 s and returns by 106 s. g_breath is 1.25 at each inhale peak and 0.85 at each
exhale. Linear cross-fades between decorrelated textures dip about 3 dB at the midpoint, which is wanted
here (a soft "breath" at each boundary).

### The path

Stages (segment labels): **arrival** 0–20 s · **around the head** 20–46 · **crown** 46–76 · **inside**
76–102 · **throat & shoulders** 102–117 · **left arm** 117–143 · **across to the right arm** 143–176 ·
**chest & heart** 176–196 · **belly** 196–229 · **legs & feet** 229–267 · **return** 267–289 · **rest**
289–300.

Notes on the route:
- Arrival: 1.5 m → 0.4 m in front in 20 s (5.5 cm/s). This is the scene's only approach, slow (> 5 s) and
  invited.
- Around the head: radius 0.32 m at ear height, counter-clockwise seen from above (left ear first), 45°
  every 3 s = 15°/s, one lap in 24 s.
- Crown: up the forehead to the vertex, 4 s of near-stillness, then a halo arc over the top: left ear → crown
  → right ear (15°/s).
- Inside: from the right ear inward to the centre [0.02, 0, 0.03] over 15 s, 11 s there, then it sinks.
  The texture turns into the warm hum and the level falls by about 7 dB.
- Arms: 2.5 cm/s down the left arm (vertical motion is resolved worse, so it is slow), a 4 s hold at the
  fingertips, 16 s across the lap at thigh height, a 2 s hold at the right fingertips, faster (15 s) up the
  right arm.
- Belly: three breaths of 10 s each (inhale 4 s, exhale 6 s). The point moves 3 cm forward on each inhale
  and its gain swells by +3.4 dB: 6 breaths/min.
- Legs: 2.5 s of silence (229–231.5), then the guide reappears as two mirrored sources at the hips (id 12
  on the left, id 14 on the right), walks down the thighs, the knees, the shins, and rests 4 s on the
  soles, where the warm hum joins (grounding). The two sources together are +3 dB: the body widens.
- Return: up the front of the body; the twin fades out by 275 s while the air texture rises, then the brush
  again at the face. It leaves forward to 0.9 m and fades out at 300 s.

Master table. Positions are shared by ids 10–13; each id uses its own gain column.

| t s | x | y | z | r m | stage starts | gain brush (10) | air (11) | grain (12) | warm (13) |
|---|---|---|---|---|---|---|---|---|---|
| 0 | 1.5 | 0 | 0.05 | 1.5 | arrival | 0.0 | 0.0 | 0.0 | 0.0 |
| 5 | 1.2 | 0 | 0.04 | 1.2 |  | 1.83 | 0.0 | 0.0 | 0.0 |
| 10 | 0.9 | 0.02 | 0.03 | 0.9 |  | 1.91 | 0.0 | 0.0 | 0.0 |
| 15 | 0.62 | 0.03 | 0.02 | 0.62 |  | 1.42 | 0.0 | 0.0 | 0.0 |
| 20 | 0.4 | 0.02 | 0.0 | 0.4 |  | 1.0 | 0.0 | 0.0 | 0.0 |
| 23 | 0.226 | 0.226 | 0.0 | 0.32 | around | 0.84 | 0.0 | 0.0 | 0.0 |
| 26 | 0.0 | 0.32 | 0.0 | 0.32 |  | 0.84 | 0.0 | 0.0 | 0.0 |
| 29 | -0.226 | 0.226 | 0.0 | 0.32 |  | 0.84 | 0.0 | 0.0 | 0.0 |
| 32 | -0.32 | 0.0 | 0.0 | 0.32 |  | 0.84 | 0.0 | 0.0 | 0.0 |
| 35 | -0.226 | -0.226 | 0.0 | 0.32 |  | 0.84 | 0.0 | 0.0 | 0.0 |
| 38 | -0.0 | -0.32 | 0.0 | 0.32 |  | 0.84 | 0.0 | 0.0 | 0.0 |
| 41 | 0.226 | -0.226 | 0.0 | 0.32 |  | 0.84 | 0.0 | 0.0 | 0.0 |
| 44 | 0.32 | -0.0 | 0.0 | 0.32 |  | 0.84 | 0.0 | 0.0 | 0.0 |
| 46 | 0.3 | 0 | 0.06 | 0.31 | crown | 0.81 | 0.0 | 0.0 | 0.0 |
| 49 | 0.22 | 0 | 0.16 | 0.27 |  | 0.73 | 0.0 | 0.0 | 0.0 |
| 52 | 0.08 | 0 | 0.22 | 0.23 |  | 0.65 | 0.0 | 0.0 | 0.0 |
| 55 | -0.01 | 0 | 0.23 | 0.23 |  | 0.64 | 0.0 | 0.0 | 0.0 |
| 58 | -0.02 | 0.01 | 0.235 | 0.24 |  | 0.66 | 0.0 | 0.0 | 0.0 |
| 60 | -0.01 | 0.1 | 0.21 | 0.23 |  | 0.65 | 0.0 | 0.0 | 0.0 |
| 62 | 0 | 0.22 | 0.12 | 0.25 |  | 0.69 | 0.0 | 0.0 | 0.0 |
| 64 | 0 | 0.28 | 0 | 0.28 |  | 0.75 | 0.0 | 0.0 | 0.0 |
| 66 | 0 | 0.24 | 0.12 | 0.27 |  | 0.73 | 0.0 | 0.0 | 0.0 |
| 68 | 0 | 0.12 | 0.21 | 0.24 |  | 0.67 | 0.0 | 0.0 | 0.0 |
| 70 | 0 | 0 | 0.235 | 0.23 |  | 0.65 | 0.0 | 0.0 | 0.0 |
| 72 | 0 | -0.12 | 0.21 | 0.24 |  | 0.67 | 0.0 | 0.0 | 0.0 |
| 74 | 0 | -0.24 | 0.12 | 0.27 |  | 0.73 | 0.0 | 0.0 | 0.0 |
| 76 | 0 | -0.28 | 0 | 0.28 | inside | 0.75 | 0.0 | 0.0 | 0.0 |
| 80 | 0 | -0.19 | 0.01 | 0.19 |  | 0.25 | 0.0 | 0.0 | 0.25 |
| 84 | 0.01 | -0.1 | 0.02 | 0.1 |  | 0.0 | 0.0 | 0.0 | 0.33 |
| 88 | 0.02 | -0.03 | 0.03 | 0.05 |  | 0.0 | 0.0 | 0.0 | 0.22 |
| 91 | 0.02 | 0 | 0.03 | 0.04 |  | 0.0 | 0.0 | 0.0 | 0.23 |
| 95 | 0.025 | 0 | 0.035 | 0.04 |  | 0.0 | 0.0 | 0.0 | 0.23 |
| 99 | 0.02 | 0 | 0.025 | 0.03 |  | 0.0 | 0.0 | 0.0 | 0.23 |
| 102 | 0.03 | 0 | -0.03 | 0.04 | throat | 0.0 | 0.0 | 0.0 | 0.32 |
| 105 | 0.05 | 0 | -0.1 | 0.11 |  | 0.0 | 0.23 | 0.0 | 0.23 |
| 108 | 0.09 | 0 | -0.19 | 0.21 |  | 0.0 | 0.6 | 0.0 | 0.0 |
| 111 | 0.06 | 0.1 | -0.22 | 0.25 |  | 0.0 | 0.68 | 0.0 | 0.0 |
| 114 | 0.02 | 0.19 | -0.23 | 0.3 |  | 0.0 | 0.79 | 0.0 | 0.0 |
| 117 | 0 | 0.22 | -0.29 | 0.36 | left arm | 0.0 | 0.74 | 0.19 | 0.0 |
| 120 | 0 | 0.24 | -0.36 | 0.43 |  | 0.0 | 0.21 | 0.85 | 0.0 |
| 124 | 0 | 0.26 | -0.45 | 0.52 |  | 0.0 | 0.0 | 1.23 | 0.0 |
| 128 | 0.02 | 0.27 | -0.52 | 0.59 |  | 0.0 | 0.0 | 1.36 | 0.0 |
| 132 | 0.08 | 0.3 | -0.57 | 0.65 |  | 0.0 | 0.0 | 1.47 | 0.0 |
| 136 | 0.15 | 0.33 | -0.6 | 0.7 |  | 0.0 | 0.0 | 1.57 | 0.0 |
| 139 | 0.24 | 0.35 | -0.62 | 0.75 |  | 0.0 | 0.0 | 1.66 | 0.0 |
| 143 | 0.25 | 0.355 | -0.625 | 0.76 | right arm | 0.0 | 0.0 | 1.67 | 0.0 |
| 147 | 0.26 | 0.2 | -0.64 | 0.72 |  | 0.0 | 0.0 | 1.6 | 0.0 |
| 151 | 0.27 | 0 | -0.65 | 0.7 |  | 0.0 | 0.0 | 1.57 | 0.0 |
| 155 | 0.26 | -0.2 | -0.64 | 0.72 |  | 0.0 | 0.0 | 1.6 | 0.0 |
| 159 | 0.24 | -0.35 | -0.62 | 0.75 |  | 0.0 | 0.0 | 1.66 | 0.0 |
| 161 | 0.23 | -0.355 | -0.625 | 0.75 |  | 0.0 | 0.0 | 1.66 | 0.0 |
| 164 | 0.15 | -0.33 | -0.6 | 0.7 |  | 0.0 | 0.0 | 1.57 | 0.0 |
| 167 | 0.08 | -0.3 | -0.57 | 0.65 |  | 0.0 | 0.0 | 1.47 | 0.0 |
| 170 | 0.02 | -0.27 | -0.52 | 0.59 |  | 0.0 | 0.0 | 1.36 | 0.0 |
| 173 | 0 | -0.25 | -0.4 | 0.47 |  | 0.0 | 0.0 | 1.14 | 0.0 |
| 176 | 0.01 | -0.2 | -0.24 | 0.31 | chest | 0.0 | 0.0 | 0.82 | 0.0 |
| 180 | 0.08 | -0.1 | -0.26 | 0.29 |  | 0.0 | 0.62 | 0.15 | 0.0 |
| 184 | 0.12 | 0 | -0.32 | 0.34 |  | 0.0 | 0.88 | 0.0 | 0.0 |
| 188 | 0.13 | 0.05 | -0.38 | 0.4 |  | 0.0 | 1.01 | 0.0 | 0.0 |
| 192 | 0.135 | 0.055 | -0.385 | 0.41 |  | 0.0 | 1.02 | 0.0 | 0.0 |
| 196 | 0.14 | 0.02 | -0.46 | 0.48 | belly | 0.0 | 1.16 | 0.0 | 0.0 |
| 199 | 0.16 | 0 | -0.54 | 0.56 |  | 0.0 | 1.12 | 0.0 | 0.0 |
| 203 | 0.19 | 0 | -0.545 | 0.58 |  | 0.0 | 1.68 | 0.0 | 0.0 |
| 209 | 0.16 | 0 | -0.55 | 0.57 |  | 0.0 | 1.13 | 0.0 | 0.0 |
| 213 | 0.19 | 0 | -0.55 | 0.58 |  | 0.0 | 1.69 | 0.0 | 0.0 |
| 219 | 0.16 | 0 | -0.555 | 0.58 |  | 0.0 | 1.14 | 0.0 | 0.0 |
| 223 | 0.19 | 0 | -0.555 | 0.59 |  | 0.0 | 1.7 | 0.0 | 0.0 |
| 229 | 0.16 | 0 | -0.57 | 0.59 | legs | 0.0 | 1.37 | 0.0 | 0.0 |
| 231.5 | 0.14 | 0.03 | -0.64 | 0.66 |  | 0.0 | 0.0 | 0.0 | 0.0 |
| 233 | 0.12 | 0.06 | -0.7 | 0.71 |  | 0.0 | 0.0 | 1.59 | 0.0 |
| 237 | 0.1 | 0.11 | -0.74 | 0.75 |  | 0.0 | 0.0 | 1.66 | 0.0 |
| 241 | 0.22 | 0.12 | -0.73 | 0.77 |  | 0.0 | 0.0 | 1.69 | 0.0 |
| 245 | 0.36 | 0.12 | -0.72 | 0.81 |  | 0.0 | 0.0 | 1.77 | 0.0 |
| 249 | 0.46 | 0.12 | -0.74 | 0.88 |  | 0.0 | 0.0 | 1.88 | 0.0 |
| 253 | 0.45 | 0.12 | -0.9 | 1.01 |  | 0.0 | 0.0 | 2.1 | 0.0 |
| 257 | 0.43 | 0.12 | -1.05 | 1.14 |  | 0.0 | 0.0 | 2.2 | 0.0 |
| 260 | 0.42 | 0.12 | -1.15 | 1.23 |  | 0.0 | 0.0 | 2.2 | 0.0 |
| 263 | 0.5 | 0.13 | -1.21 | 1.32 |  | 0.0 | 0.0 | 1.93 | 0.39 |
| 267 | 0.505 | 0.13 | -1.215 | 1.32 | return | 0.0 | 0.0 | 1.1 | 1.54 |
| 271 | 0.45 | 0.08 | -0.95 | 1.05 |  | 0.0 | 1.09 | 0.36 | 0.51 |
| 275 | 0.36 | 0.04 | -0.7 | 0.79 |  | 0.0 | 1.72 | 0.0 | 0.0 |
| 279 | 0.24 | 0 | -0.45 | 0.51 |  | 0.0 | 1.21 | 0.0 | 0.0 |
| 282 | 0.2 | 0 | -0.2 | 0.28 |  | 0.0 | 0.76 | 0.0 | 0.0 |
| 285 | 0.25 | 0 | 0 | 0.25 |  | 0.51 | 0.17 | 0.0 | 0.0 |
| 289 | 0.55 | 0 | 0 | 0.55 | rest | 1.29 | 0.0 | 0.0 | 0.0 |
| 294 | 0.8 | 0 | -0.05 | 0.8 |  | 0.95 | 0.0 | 0.0 | 0.0 |
| 300 | 0.9 | 0 | -0.06 | 0.9 |  | 0.0 | 0.0 | 0.0 | 0.0 |

twin: [[0, 0.16, 0, -0.57, 0], [229, 0.16, 0, -0.57, 0], [231.5, 0.14, -0.03, -0.64, 0], [233, 0.12, -0.06, -0.7, 1.59], [237, 0.1, -0.11, -0.74, 1.66], [241, 0.22, -0.12, -0.73, 1.69], [245, 0.36, -0.12, -0.72, 1.77], [249, 0.46, -0.12, -0.74, 1.88], [253, 0.45, -0.12, -0.9, 2.1], [257, 0.43, -0.12, -1.05, 2.2], [260, 0.42, -0.12, -1.15, 2.2], [263, 0.5, -0.13, -1.21, 2.2], [267, 0.505, -0.13, -1.215, 2.2], [271, 0.45, -0.08, -0.95, 1.96], [275, 0.36, -0.04, -0.7, 1.72], [279, 0.24, 0, -0.45, 0], [282, 0.2, 0, -0.2, 0]]
brush [[0, 1.5, 0, 0.05, 0.0], [5, 1.2, 0, 0.04, 1.83], [10, 0.9, 0.02, 0.03, 1.91], [15, 0.62, 0.03, 0.02, 1.42], [20, 0.4, 0.02, 0.0, 1.0], [23, 0.226, 0.226, 0.0, 0.84], [26, 0.0, 0.32, 0.0, 0.84], [29, -0.226, 0.226, 0.0, 0.84], [32, -0.32, 0.0, 0.0, 0.84], [35, -0.226, -0.226, 0.0, 0.84], [38, -0.0, -0.32, 0.0, 0.84], [41, 0.226, -0.226, 0.0, 0.84], [44, 0.32, -0.0, 0.0, 0.84], [46, 0.3, 0, 0.06, 0.81], [49, 0.22, 0, 0.16, 0.73], [52, 0.08, 0, 0.22, 0.65], [55, -0.01, 0, 0.23, 0.64], [58, -0.02, 0.01, 0.235, 0.66], [60, -0.01, 0.1, 0.21, 0.65], [62, 0, 0.22, 0.12, 0.69], [64, 0, 0.28, 0, 0.75], [66, 0, 0.24, 0.12, 0.73], [68, 0, 0.12, 0.21, 0.67], [70, 0, 0, 0.235, 0.65], [72, 0, -0.12, 0.21, 0.67], [74, 0, -0.24, 0.12, 0.73], [76, 0, -0.28, 0, 0.75], [80, 0, -0.19, 0.01, 0.25], [84, 0.01, -0.1, 0.02, 0.0], [88, 0.02, -0.03, 0.03, 0.0], [91, 0.02, 0, 0.03, 0.0], [95, 0.025, 0, 0.035, 0.0], [99, 0.02, 0, 0.025, 0.0], [102, 0.03, 0, -0.03, 0.0], [105, 0.05, 0, -0.1, 0.0], [108, 0.09, 0, -0.19, 0.0], [111, 0.06, 0.1, -0.22, 0.0], [114, 0.02, 0.19, -0.23, 0.0], [117, 0, 0.22, -0.29, 0.0], [120, 0, 0.24, -0.36, 0.0], [124, 0, 0.26, -0.45, 0.0], [128, 0.02, 0.27, -0.52, 0.0], [132, 0.08, 0.3, -0.57, 0.0], [136, 0.15, 0.33, -0.6, 0.0], [139, 0.24, 0.35, -0.62, 0.0], [143, 0.25, 0.355, -0.625, 0.0], [147, 0.26, 0.2, -0.64, 0.0], [151, 0.27, 0, -0.65, 0.0], [155, 0.26, -0.2, -0.64, 0.0], [159, 0.24, -0.35, -0.62, 0.0], [161, 0.23, -0.355, -0.625, 0.0], [164, 0.15, -0.33, -0.6, 0.0], [167, 0.08, -0.3, -0.57, 0.0], [170, 0.02, -0.27, -0.52, 0.0], [173, 0, -0.25, -0.4, 0.0], [176, 0.01, -0.2, -0.24, 0.0], [180, 0.08, -0.1, -0.26, 0.0], [184, 0.12, 0, -0.32, 0.0], [188, 0.13, 0.05, -0.38, 0.0], [192, 0.135, 0.055, -0.385, 0.0], [196, 0.14, 0.02, -0.46, 0.0], [199, 0.16, 0, -0.54, 0.0], [203, 0.19, 0, -0.545, 0.0], [209, 0.16, 0, -0.55, 0.0], [213, 0.19, 0, -0.55, 0.0], [219, 0.16, 0, -0.555, 0.0], [223, 0.19, 0, -0.555, 0.0], [229, 0.16, 0, -0.57, 0.0], [231.5, 0.14, 0.03, -0.64, 0.0], [233, 0.12, 0.06, -0.7, 0.0], [237, 0.1, 0.11, -0.74, 0.0], [241, 0.22, 0.12, -0.73, 0.0], [245, 0.36, 0.12, -0.72, 0.0], [249, 0.46, 0.12, -0.74, 0.0], [253, 0.45, 0.12, -0.9, 0.0], [257, 0.43, 0.12, -1.05, 0.0], [260, 0.42, 0.12, -1.15, 0.0], [263, 0.5, 0.13, -1.21, 0.0], [267, 0.505, 0.13, -1.215, 0.0], [271, 0.45, 0.08, -0.95, 0.0], [275, 0.36, 0.04, -0.7, 0.0], [279, 0.24, 0, -0.45, 0.0], [282, 0.2, 0, -0.2, 0.0], [285, 0.25, 0, 0, 0.51], [289, 0.55, 0, 0, 1.29], [294, 0.8, 0, -0.05, 0.95], [300, 0.9, 0, -0.06, 0.0]]
air [[0, 1.5, 0, 0.05, 0.0], [5, 1.2, 0, 0.04, 0.0], [10, 0.9, 0.02, 0.03, 0.0], [15, 0.62, 0.03, 0.02, 0.0], [20, 0.4, 0.02, 0.0, 0.0], [23, 0.226, 0.226, 0.0, 0.0], [26, 0.0, 0.32, 0.0, 0.0], [29, -0.226, 0.226, 0.0, 0.0], [32, -0.32, 0.0, 0.0, 0.0], [35, -0.226, -0.226, 0.0, 0.0], [38, -0.0, -0.32, 0.0, 0.0], [41, 0.226, -0.226, 0.0, 0.0], [44, 0.32, -0.0, 0.0, 0.0], [46, 0.3, 0, 0.06, 0.0], [49, 0.22, 0, 0.16, 0.0], [52, 0.08, 0, 0.22, 0.0], [55, -0.01, 0, 0.23, 0.0], [58, -0.02, 0.01, 0.235, 0.0], [60, -0.01, 0.1, 0.21, 0.0], [62, 0, 0.22, 0.12, 0.0], [64, 0, 0.28, 0, 0.0], [66, 0, 0.24, 0.12, 0.0], [68, 0, 0.12, 0.21, 0.0], [70, 0, 0, 0.235, 0.0], [72, 0, -0.12, 0.21, 0.0], [74, 0, -0.24, 0.12, 0.0], [76, 0, -0.28, 0, 0.0], [80, 0, -0.19, 0.01, 0.0], [84, 0.01, -0.1, 0.02, 0.0], [88, 0.02, -0.03, 0.03, 0.0], [91, 0.02, 0, 0.03, 0.0], [95, 0.025, 0, 0.035, 0.0], [99, 0.02, 0, 0.025, 0.0], [102, 0.03, 0, -0.03, 0.0], [105, 0.05, 0, -0.1, 0.23], [108, 0.09, 0, -0.19, 0.6], [111, 0.06, 0.1, -0.22, 0.68], [114, 0.02, 0.19, -0.23, 0.79], [117, 0, 0.22, -0.29, 0.74], [120, 0, 0.24, -0.36, 0.21], [124, 0, 0.26, -0.45, 0.0], [128, 0.02, 0.27, -0.52, 0.0], [132, 0.08, 0.3, -0.57, 0.0], [136, 0.15, 0.33, -0.6, 0.0], [139, 0.24, 0.35, -0.62, 0.0], [143, 0.25, 0.355, -0.625, 0.0], [147, 0.26, 0.2, -0.64, 0.0], [151, 0.27, 0, -0.65, 0.0], [155, 0.26, -0.2, -0.64, 0.0], [159, 0.24, -0.35, -0.62, 0.0], [161, 0.23, -0.355, -0.625, 0.0], [164, 0.15, -0.33, -0.6, 0.0], [167, 0.08, -0.3, -0.57, 0.0], [170, 0.02, -0.27, -0.52, 0.0], [173, 0, -0.25, -0.4, 0.0], [176, 0.01, -0.2, -0.24, 0.0], [180, 0.08, -0.1, -0.26, 0.62], [184, 0.12, 0, -0.32, 0.88], [188, 0.13, 0.05, -0.38, 1.01], [192, 0.135, 0.055, -0.385, 1.02], [196, 0.14, 0.02, -0.46, 1.16], [199, 0.16, 0, -0.54, 1.12], [203, 0.19, 0, -0.545, 1.68], [209, 0.16, 0, -0.55, 1.13], [213, 0.19, 0, -0.55, 1.69], [219, 0.16, 0, -0.555, 1.14], [223, 0.19, 0, -0.555, 1.7], [229, 0.16, 0, -0.57, 1.37], [231.5, 0.14, 0.03, -0.64, 0.0], [233, 0.12, 0.06, -0.7, 0.0], [237, 0.1, 0.11, -0.74, 0.0], [241, 0.22, 0.12, -0.73, 0.0], [245, 0.36, 0.12, -0.72, 0.0], [249, 0.46, 0.12, -0.74, 0.0], [253, 0.45, 0.12, -0.9, 0.0], [257, 0.43, 0.12, -1.05, 0.0], [260, 0.42, 0.12, -1.15, 0.0], [263, 0.5, 0.13, -1.21, 0.0], [267, 0.505, 0.13, -1.215, 0.0], [271, 0.45, 0.08, -0.95, 1.09], [275, 0.36, 0.04, -0.7, 1.72], [279, 0.24, 0, -0.45, 1.21], [282, 0.2, 0, -0.2, 0.76], [285, 0.25, 0, 0, 0.17], [289, 0.55, 0, 0, 0.0], [294, 0.8, 0, -0.05, 0.0], [300, 0.9, 0, -0.06, 0.0]]
grain [[0, 1.5, 0, 0.05, 0.0], [5, 1.2, 0, 0.04, 0.0], [10, 0.9, 0.02, 0.03, 0.0], [15, 0.62, 0.03, 0.02, 0.0], [20, 0.4, 0.02, 0.0, 0.0], [23, 0.226, 0.226, 0.0, 0.0], [26, 0.0, 0.32, 0.0, 0.0], [29, -0.226, 0.226, 0.0, 0.0], [32, -0.32, 0.0, 0.0, 0.0], [35, -0.226, -0.226, 0.0, 0.0], [38, -0.0, -0.32, 0.0, 0.0], [41, 0.226, -0.226, 0.0, 0.0], [44, 0.32, -0.0, 0.0, 0.0], [46, 0.3, 0, 0.06, 0.0], [49, 0.22, 0, 0.16, 0.0], [52, 0.08, 0, 0.22, 0.0], [55, -0.01, 0, 0.23, 0.0], [58, -0.02, 0.01, 0.235, 0.0], [60, -0.01, 0.1, 0.21, 0.0], [62, 0, 0.22, 0.12, 0.0], [64, 0, 0.28, 0, 0.0], [66, 0, 0.24, 0.12, 0.0], [68, 0, 0.12, 0.21, 0.0], [70, 0, 0, 0.235, 0.0], [72, 0, -0.12, 0.21, 0.0], [74, 0, -0.24, 0.12, 0.0], [76, 0, -0.28, 0, 0.0], [80, 0, -0.19, 0.01, 0.0], [84, 0.01, -0.1, 0.02, 0.0], [88, 0.02, -0.03, 0.03, 0.0], [91, 0.02, 0, 0.03, 0.0], [95, 0.025, 0, 0.035, 0.0], [99, 0.02, 0, 0.025, 0.0], [102, 0.03, 0, -0.03, 0.0], [105, 0.05, 0, -0.1, 0.0], [108, 0.09, 0, -0.19, 0.0], [111, 0.06, 0.1, -0.22, 0.0], [114, 0.02, 0.19, -0.23, 0.0], [117, 0, 0.22, -0.29, 0.19], [120, 0, 0.24, -0.36, 0.85], [124, 0, 0.26, -0.45, 1.23], [128, 0.02, 0.27, -0.52, 1.36], [132, 0.08, 0.3, -0.57, 1.47], [136, 0.15, 0.33, -0.6, 1.57], [139, 0.24, 0.35, -0.62, 1.66], [143, 0.25, 0.355, -0.625, 1.67], [147, 0.26, 0.2, -0.64, 1.6], [151, 0.27, 0, -0.65, 1.57], [155, 0.26, -0.2, -0.64, 1.6], [159, 0.24, -0.35, -0.62, 1.66], [161, 0.23, -0.355, -0.625, 1.66], [164, 0.15, -0.33, -0.6, 1.57], [167, 0.08, -0.3, -0.57, 1.47], [170, 0.02, -0.27, -0.52, 1.36], [173, 0, -0.25, -0.4, 1.14], [176, 0.01, -0.2, -0.24, 0.82], [180, 0.08, -0.1, -0.26, 0.15], [184, 0.12, 0, -0.32, 0.0], [188, 0.13, 0.05, -0.38, 0.0], [192, 0.135, 0.055, -0.385, 0.0], [196, 0.14, 0.02, -0.46, 0.0], [199, 0.16, 0, -0.54, 0.0], [203, 0.19, 0, -0.545, 0.0], [209, 0.16, 0, -0.55, 0.0], [213, 0.19, 0, -0.55, 0.0], [219, 0.16, 0, -0.555, 0.0], [223, 0.19, 0, -0.555, 0.0], [229, 0.16, 0, -0.57, 0.0], [231.5, 0.14, 0.03, -0.64, 0.0], [233, 0.12, 0.06, -0.7, 1.59], [237, 0.1, 0.11, -0.74, 1.66], [241, 0.22, 0.12, -0.73, 1.69], [245, 0.36, 0.12, -0.72, 1.77], [249, 0.46, 0.12, -0.74, 1.88], [253, 0.45, 0.12, -0.9, 2.1], [257, 0.43, 0.12, -1.05, 2.2], [260, 0.42, 0.12, -1.15, 2.2], [263, 0.5, 0.13, -1.21, 1.93], [267, 0.505, 0.13, -1.215, 1.1], [271, 0.45, 0.08, -0.95, 0.36], [275, 0.36, 0.04, -0.7, 0.0], [279, 0.24, 0, -0.45, 0.0], [282, 0.2, 0, -0.2, 0.0], [285, 0.25, 0, 0, 0.0], [289, 0.55, 0, 0, 0.0], [294, 0.8, 0, -0.05, 0.0], [300, 0.9, 0, -0.06, 0.0]]
warm [[0, 1.5, 0, 0.05, 0.0], [5, 1.2, 0, 0.04, 0.0], [10, 0.9, 0.02, 0.03, 0.0], [15, 0.62, 0.03, 0.02, 0.0], [20, 0.4, 0.02, 0.0, 0.0], [23, 0.226, 0.226, 0.0, 0.0], [26, 0.0, 0.32, 0.0, 0.0], [29, -0.226, 0.226, 0.0, 0.0], [32, -0.32, 0.0, 0.0, 0.0], [35, -0.226, -0.226, 0.0, 0.0], [38, -0.0, -0.32, 0.0, 0.0], [41, 0.226, -0.226, 0.0, 0.0], [44, 0.32, -0.0, 0.0, 0.0], [46, 0.3, 0, 0.06, 0.0], [49, 0.22, 0, 0.16, 0.0], [52, 0.08, 0, 0.22, 0.0], [55, -0.01, 0, 0.23, 0.0], [58, -0.02, 0.01, 0.235, 0.0], [60, -0.01, 0.1, 0.21, 0.0], [62, 0, 0.22, 0.12, 0.0], [64, 0, 0.28, 0, 0.0], [66, 0, 0.24, 0.12, 0.0], [68, 0, 0.12, 0.21, 0.0], [70, 0, 0, 0.235, 0.0], [72, 0, -0.12, 0.21, 0.0], [74, 0, -0.24, 0.12, 0.0], [76, 0, -0.28, 0, 0.0], [80, 0, -0.19, 0.01, 0.25], [84, 0.01, -0.1, 0.02, 0.33], [88, 0.02, -0.03, 0.03, 0.22], [91, 0.02, 0, 0.03, 0.23], [95, 0.025, 0, 0.035, 0.23], [99, 0.02, 0, 0.025, 0.23], [102, 0.03, 0, -0.03, 0.32], [105, 0.05, 0, -0.1, 0.23], [108, 0.09, 0, -0.19, 0.0], [111, 0.06, 0.1, -0.22, 0.0], [114, 0.02, 0.19, -0.23, 0.0], [117, 0, 0.22, -0.29, 0.0], [120, 0, 0.24, -0.36, 0.0], [124, 0, 0.26, -0.45, 0.0], [128, 0.02, 0.27, -0.52, 0.0], [132, 0.08, 0.3, -0.57, 0.0], [136, 0.15, 0.33, -0.6, 0.0], [139, 0.24, 0.35, -0.62, 0.0], [143, 0.25, 0.355, -0.625, 0.0], [147, 0.26, 0.2, -0.64, 0.0], [151, 0.27, 0, -0.65, 0.0], [155, 0.26, -0.2, -0.64, 0.0], [159, 0.24, -0.35, -0.62, 0.0], [161, 0.23, -0.355, -0.625, 0.0], [164, 0.15, -0.33, -0.6, 0.0], [167, 0.08, -0.3, -0.57, 0.0], [170, 0.02, -0.27, -0.52, 0.0], [173, 0, -0.25, -0.4, 0.0], [176, 0.01, -0.2, -0.24, 0.0], [180, 0.08, -0.1, -0.26, 0.0], [184, 0.12, 0, -0.32, 0.0], [188, 0.13, 0.05, -0.38, 0.0], [192, 0.135, 0.055, -0.385, 0.0], [196, 0.14, 0.02, -0.46, 0.0], [199, 0.16, 0, -0.54, 0.0], [203, 0.19, 0, -0.545, 0.0], [209, 0.16, 0, -0.55, 0.0], [213, 0.19, 0, -0.55, 0.0], [219, 0.16, 0, -0.555, 0.0], [223, 0.19, 0, -0.555, 0.0], [229, 0.16, 0, -0.57, 0.0], [231.5, 0.14, 0.03, -0.64, 0.0], [233, 0.12, 0.06, -0.7, 0.0], [237, 0.1, 0.11, -0.74, 0.0], [241, 0.22, 0.12, -0.73, 0.0], [245, 0.36, 0.12, -0.72, 0.0], [249, 0.46, 0.12, -0.74, 0.0], [253, 0.45, 0.12, -0.9, 0.0], [257, 0.43, 0.12, -1.05, 0.0], [260, 0.42, 0.12, -1.15, 0.0], [263, 0.5, 0.13, -1.21, 0.39], [267, 0.505, 0.13, -1.215, 1.54], [271, 0.45, 0.08, -0.95, 0.51], [275, 0.36, 0.04, -0.7, 0.0], [279, 0.24, 0, -0.45, 0.0], [282, 0.2, 0, -0.2, 0.0], [285, 0.25, 0, 0, 0.0], [289, 0.55, 0, 0, 0.0], [294, 0.8, 0, -0.05, 0.0], [300, 0.9, 0, -0.06, 0.0]]

**Twin path (id 14, right leg),** `[t, x, y, z, gain]`. Its gain equals the main grain gain in 233–271:

```json
[[0,0.16,0,-0.57,0],[229,0.16,0,-0.57,0],[231.5,0.14,-0.03,-0.64,0],[233,0.12,-0.06,-0.70,1.59],
 [237,0.10,-0.11,-0.74,1.66],[241,0.22,-0.12,-0.73,1.69],[245,0.36,-0.12,-0.72,1.77],[249,0.46,-0.12,-0.74,1.88],
 [253,0.45,-0.12,-0.90,2.10],[257,0.43,-0.12,-1.05,2.20],[260,0.42,-0.12,-1.15,2.20],[263,0.50,-0.13,-1.21,1.93],
 [267,0.505,-0.13,-1.215,1.10],[271,0.45,-0.08,-0.95,0.36],[275,0.36,-0.04,-0.70,0],[282,0.20,0,-0.20,0]]
```

The table was generated by the script below (keep it next to the scene, e.g.
`tools/scenes/body_scan_path.py`, to retune). It prints the four `points` arrays for ids 10–13:

```python
import math, json
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
         'air':  [(0,0),(102,0),(108,1),(116,1),(121,0),(176,0),(181,1),(229,1),(230,0),(267,0),(275,1),(282,1),(286,0),(300,0)],
         'grain':[(0,0),(116,0),(121,1),(176,1),(181,0),(231.5,0),(233,1),(262,1),(266,0.5),(267,0.5),(273,0),(300,0)],
         'warm': [(0,0),(76,0),(84,1),(102,1),(108,0),(262,0),(266,0.7),(267,0.7),(273,0),(300,0)]}
FADE   = [(0,0),(6,1),(229,1),(230,0),(231.5,0),(233,1),(289,1),(300,0)]
INSIDE = [(0,1),(78,1),(88,0.45),(100,0.45),(106,1),(300,1)]
BREATH = {199:0.85,203:1.25,209:0.85,213:1.25,219:0.85,223:1.25}
for name, w in LAYER.items():
    pts = []
    for t,x,y,z in K:
        r = math.sqrt(x*x+y*y+z*z); gd = min(max((max(r,0.15)/0.4)**0.8, 0.5), 2.2)
        pts.append([t,x,y,z, round(gd*lerp(FADE,t)*lerp(INSIDE,t)*BREATH.get(t,1)*lerp(w,t), 2)])
    print(name, json.dumps(pts))
```

### What to generate (≈ $0.62)

```json
[
 {"scene":"bodyscan","name":"guide_brush","model":"el2","duration":22,"loop":true,"prompt_influence":0.65,"variations":2,
  "prompt":"Close-up of a soft wide makeup brush slowly stroking a foam microphone cover, continuous gentle brushing, even and smooth, broadband soft swishing with airy high detail, no taps. Only the brush, no music, no voices, dry, single source."},
 {"scene":"bodyscan","name":"guide_air","model":"el2","duration":22,"loop":true,"prompt_influence":0.6,"variations":2,
  "prompt":"A very soft steady airy whoosh like slow calm breath passing over a microphone, continuous, smooth, no breathing rhythm, no mouth sounds, gentle broadband air. Only the air, no music, no voices, dry, single source."},
 {"scene":"bodyscan","name":"guide_grain","model":"el2","duration":22,"loop":true,"prompt_influence":0.6,"variations":2,
  "prompt":"A rainstick tilted very slowly, a continuous fine trickle of small seeds falling inside a bamboo tube, soft and even, close-up, no shaking. Only the rainstick, no music, no voices, dry, single source."},
 {"scene":"bodyscan","name":"guide_warm","model":"el2","duration":22,"loop":true,"prompt_influence":0.6,"variations":2,
  "prompt":"A warm soft low hum of a large singing bowl rubbed slowly with a suede mallet, steady sustained drone around 110 Hz with a faint shimmering overtone, no strikes, smooth. Only the bowl, no music, no voices, dry, single source."},
 {"scene":"bodyscan","name":"bowl_strike","model":"el2","duration":12,"prompt_influence":0.65,"variations":3,
  "prompt":"One gentle strike of a medium Tibetan singing bowl with a soft mallet, a single clear warm tone ringing out and decaying slowly to silence. Single sound, no music, no voices, dry."},
 {"scene":"bodyscan","name":"room_tone","model":"sa25","duration":90,"variations":1,
  "prompt":"Field recording, room tone of a quiet wooden room at night, very soft steady air, faint distant hum, nothing happening, constant, realistic, no music, no voices, no clicks."}
]
```

`grain` variation 1 goes to id 12 and variation 2 to id 14, so the twins are decorrelated.

### Levers

| Key | Title | Range [default] | Drives |
|---|---|---|---|
| `guide` | Guide | −12..+6 dB [0] | `guide.level` [[−12, −12], [6, 6]] |
| `room` | Room | 0..1 [0.5] | `room.level` [[0, −30], [0.5, 0], [1, 6]] |
| `bowls` | Bowls | 0..1 [0.7] | `bowls.level` [[0, −30], [0.7, 0], [1, 4]] |

All with `lively` 0. None moves `guide.motion` (path clock).

### Direction: the script

```jsonc
"direction": {
  "climate": false, "only": true, "start": "body_scan", "script": [ { "episode": "body_scan" } ], "repeat": false,
  "energy": -0.6, "change": 0.1,
  "episodes": [
    { "name": "body_scan", "weight": 0, "energy": -0.6, "drama": 0.2, "then": "rest",
      "segments": [
        { "label": "arrival",             "minutes": 0.3333, "levers": { "room": 0.5 } },
        { "label": "around the head",     "minutes": 0.4333 },
        { "label": "crown",               "minutes": 0.5 },
        { "label": "inside",              "minutes": 0.4333, "levers": { "room": 0.35 } },
        { "label": "throat & shoulders",  "minutes": 0.25,   "levers": { "room": 0.5 } },
        { "label": "left arm",            "minutes": 0.4333 },
        { "label": "across to the right arm", "minutes": 0.55 },
        { "label": "chest & heart",       "minutes": 0.3333 },
        { "label": "belly",               "minutes": 0.55,   "levers": { "room": 0.42 } },
        { "label": "legs & feet",         "minutes": 0.6333, "levers": { "room": 0.5 } },
        { "label": "return",              "minutes": 0.3667 },
        { "label": "rest",                "minutes": 0.1833, "levers": { "room": 0.6 } } ] },
    { "name": "rest", "weight": 1, "energy": -1,
      "segments": [ { "label": "afterglow", "minutes": 2, "levers": { "room": 0.6 } },
                    { "label": "stillness", "minutes": [3, 6], "levers": { "room": 0.45 } } ] }
  ]
}
```

The 12 segments add up to exactly 300 s, aligned with the path stages. Keep the default ease: room-lever
moves are small and slow. `cue` should be hidden in this scene (or should restart it): cueing "next" would
move the label but not the path (need 3).

### Out-of-the-box
No app does a body scan without a voice. Here the "voice" is a sound that is at the body part it names.
Inside the head, both twins on the legs, and the breathing belly are things a recorded binaural track
cannot do with head tracking: here the path stays on the body when you turn your head.

---

## 5. Scene 2 — Lantern room (`lighthouse_gale.json`)

### Premise
Night watch in the lantern room of a 30 m lighthouse on a rock. A 2.2 m-radius glass room. You sit 1.2 m
from the great lens, which turns on its rollers. The clockwork works under the floor. Outside: wind in the
gallery railing, rain on the windward panes and on the copper dome above, the ventilator ball whistling
in the cap, and the sea booming on the rocks 32 m below. Far out, a bell buoy rocks, and in fog a distant
station sounds its horn. Over hours the weather comes and goes: a calm watch, the wind freshens, a full
gale, a squall with thunder, fog, clearing.

### Why it is spatially strong
- **Vertical:** the sea is below you at −32 m (elevation about −50° to −70° for the nearest breakers), the
  dome rain and the vent are overhead (+2.4, +3.2 m), and the room is at ear height. No other scene puts
  the main bed below the listener.
- **Weather from one side:** windward is left-behind (+y, −x). Turning your head toward it brings the
  lashing glass in front. The lee side is quiet.
- **One slow orbit:** the lens carriage circles its own centre (radius 0.7 m, period 30 s). From where you
  sit it swings across ±35° in front of you, so it is never a rotating field around the head.
- **Glass:** `medium: "water"` with `muffle_hz: 2800` low-passes all procedural outdoor sound, as if heard
  through the panes. The interior clips stay crisp.

### Environment
`time_of_day` 1.0, `wind_ms` 12, `rain_mm_h` 6, `temperature_c` 9, `humidity` 0.95, `ground_z` −33 (the sea
and rock: there is no meaningful floor reflection for the outdoor sources), `medium` "water", `muffle_hz`
2800.

### Sound groups

| Group | id | Source | Position / motion | Playback / interval | Level | Notes |
|---|---|---|---|---|---|---|
| `lamp` | 10 | clip `optic_rollers` (loop) | orbit center [1.2, 0, −0.4], radius 0.7, period 30 | loop, texture | 50 dB | the turning lens |
| `lamp` | 11 | clip `clockwork` (loop) | fixed [1.2, 0, −1.4] | loop, texture | 50 dB | governor whirr under the floor |
| `wind` | 20 | model wind `breeze`, `exposure` 0.5, `gain` 1.0 | fixed [−8, 10, 0], emitters 3, spread 8 | bed | model | moderate wind (≈ 6 m/s) |
| `gale` | 21 | model wind `storm`, `exposure` 1.6 | fixed [−8, 10, 0], emitters 3, spread 8 | bed | model | gale body (≈ 19 m/s) |
| `gale` | 22 | model wind `fence_wire`, `exposure` 1.4 | fixed [1.2, 0, −0.6], emitters 6, spread 2.8 | bed | model | the gallery railing ring |
| `gale` | 23 | model wind `gap_howl`, `exposure` 1.2 | fixed [1.2, 0, 3.2] | bed | model | ventilator ball overhead |
| `rain` | 30 | clip `rain_on_glass` (loop) | fixed [−0.4, 1.9, 0.3], emitters 3, spread 1.2 | loop, bed | 54 dB | windward panes; crisp, inside |
| `rain` | 31 | model rain `tin_roof`, params `{gain: 0.6}` | fixed [1.0, 0, 2.4], emitters 4, spread 1.6 | bed | model | copper dome overhead |
| `sea` | 40 | model water `lake_lapping`, `{waves: 2, fetch_m: 20000, gain: 1.4}` | fixed [0, 0, −32], emitters 6, spread 25 | bed | model | the swell around the rock |
| `sea` | 41–43 | clip `surf_crash` (4 takes) | fixed [14, −10, −31], [−6, 18, −31], [10, 12, −31] | events [9, 16], [11, 20], [14, 26]; rate [0.9, 1.05]; fade 1 | 100 dB | breakers with a real 8–16 s swell period; −6 dB re bed |
| `buoy` | 50 | clip `bell_buoy` | fixed [260, 160, −31] | events [6, 18] | 100 dB | ~50 dB at 305 m, a far anchor |
| `fog` | 60 | clip `foghorn` | fixed [−600, −1300, −25] | events [45, 75], default events 0 | 122 dB | only in fog |
| `storm` | 70 | model thunder `distant_rumble` | fixed [−3000, 2500, 800] | model | model | faded out except in a squall |
| `tower` | 80 | clip `pane_rattle` | fixed [−0.2, 2.1, 0.2] | events [40, 120] | 44 dB | the windward glass in gusts |
| `tower` | 81 | clip `stair_ping` | fixed [0.4, −0.8, −1.6] | events [60, 180] | 46 dB | iron stair below flexing |

Focus check: in a full gale about 4–6 noticeable events/min (breakers, rattle) under a loud stationary
bed. Breakers are broadband swells without sharp attacks, so they read as bed. In "calm watch" only the
lamp, the swell, the buoy and 2–3 breakers/min remain.

### What to generate (≈ $0.72)

```json
[
 {"scene":"lighthouse","name":"optic_rollers","model":"el2","duration":22,"loop":true,"prompt_influence":0.6,"variations":2,
  "prompt":"A heavy brass lighthouse lens slowly rotating on steel rollers, a soft continuous low rolling rumble with a faint metallic purr, steady and smooth, heard close indoors. Only the mechanism, no music, no voices, no beeps, dry, single source."},
 {"scene":"lighthouse","name":"clockwork","model":"el2","duration":22,"loop":true,"prompt_influence":0.6,"variations":2,
  "prompt":"An old clockwork machine with a spinning fan governor under a wooden floor, a soft steady whirr with a slow faint ticking of gears, constant, muffled. Only the clockwork, no music, no voices, no chimes, dry, single source."},
 {"scene":"lighthouse","name":"rain_on_glass","model":"el2","duration":22,"loop":true,"prompt_influence":0.6,"variations":2,
  "prompt":"Wind-driven rain lashing against a large glass window, heard from inside close to the pane, dense pattering and streaming water on glass, rising and falling slightly with gusts. Only the rain on glass, no thunder, no music, no voices, dry, single source."},
 {"scene":"lighthouse","name":"surf_crash","model":"el2","duration":12,"prompt_influence":0.6,"variations":4,
  "prompt":"One large ocean wave breaking on rocks far below a tall stone tower, a deep soft boom and a long rushing wash of foam fading away, heard from high above through closed windows. Single wave, no seagulls, no music, no voices, dry."},
 {"scene":"lighthouse","name":"bell_buoy","model":"el2","duration":20,"prompt_influence":0.6,"variations":2,
  "prompt":"A bell buoy rocking in the swell far out at sea at night, irregular slow clangs of a heavy bronze bell, some soft and some louder, with silence between them, distant. Only the bell, no music, no voices, dry, single source."},
 {"scene":"lighthouse","name":"foghorn","model":"el2","duration":10,"prompt_influence":0.65,"variations":2,
  "prompt":"One long low two-tone diaphone foghorn blast from a distant lighthouse across the water at night, a deep groan rising and ending with a grunt, then silence. Single sound, no ship engine, no music, no voices, dry."},
 {"scene":"lighthouse","name":"pane_rattle","model":"el2","duration":6,"prompt_influence":0.65,"variations":3,
  "prompt":"A large window pane in an iron frame rattling briefly in a gust of wind, a short soft vibrating buzz of glass, then still. Single sound, no music, no voices, dry."},
 {"scene":"lighthouse","name":"stair_ping","model":"el2","duration":6,"prompt_influence":0.65,"variations":3,
  "prompt":"A cast iron spiral staircase in a tower creaking and giving one faint metallic ping as the structure flexes in the wind, heard from just above. Single sound, no footsteps, no music, no voices, dry."}
]
```

Total ≈ 44+44+44+48+40+20+18+18 = 276 s of el2 ≈ $0.55. Re-roll margin to ~$1.

### Levers

| Key | Title | Range [def] | `lively` | Drives (`group.param: at`) |
|---|---|---|---|---|
| `gale` | Gale | 0..1 [0.3] | +0.6 | `wind.level` [[0, 0], [0.6, −4], [1, −14]]; `gale.level` [[0, −30], [0.3, −16], [0.7, −5], [1, 0]]; `rain.level` [[0, −24], [0.4, −8], [1, 2]]; `tower.events` [[0, 0], [0.5, 0.6], [1, 1.8]] |
| `sea` | Sea | 0..1 [0.4] | +0.4 | `sea.level` [[0, −8], [1, 3]]; `sea.events` [[0, 0.4], [0.5, 1], [1, 1.6]]; `buoy.events` [[0, 0.5], [1, 1.6]] |
| `thunder` | Thunder | 0..1 [0] | +0.2 | `storm.level` [[0, −30], [0.3, −10], [1, 0]]; `storm.distance` [[0, 3], [1, 1]] |
| `fog` | Fog | 0..1 [0] | −0.3 | `fog.events` [[0, 0], [0.3, 0.6], [1, 1.2]]; `buoy.level` [[0, 0], [1, 3]] (sound carries in fog) |
| `lamp` | Lamp | 0..1 [0.6] | 0 | `lamp.level` [[0, −24], [0.6, 0], [1, 4]]; `lamp.motion` [[0, 0.6], [1, 1.4]] (rotation period 50 → 21 s) |

### Episodes (`only: true`, `start: "calm_watch"`, free run)

| Episode | wet / energy / drama | after / then | Segments: label, minutes, lever targets |
|---|---|---|---|
| `calm_watch` | −0.6 / −0.7 / 0 | — | "a calm watch" [12, 25]: gale 0.12, sea 0.3, thunder 0, fog 0 |
| `freshening` | 0.2 / 0.3 / 0.2 | after calm_watch, clearing | "the wind freshens" 5: gale 0.4, sea 0.45 · "a steady blow" [8, 15]: gale 0.55, sea 0.6 |
| `gale` | 0.6 / 0.8 / 0.6 | after freshening | "the gale builds" 4: gale 0.75, sea 0.8 · "full gale" [8, 14]: gale 0.95, sea 1 · "easing" 5: gale 0.5, sea 0.7 |
| `squall` | 1 / 0.7 / 1 | after freshening, gale; then clearing | "squall line" 3: gale 0.6, thunder 0.3 · "the squall" 2 (ease 20): gale 1, thunder 0.9, sea 0.9 · "thunder moving off" 6: gale 0.55, thunder 0.35 |
| `fog` | 0.3 / −0.5 / 0.2 | after calm_watch, clearing | "fog rolls in" 4: gale 0.08, fog 0.6, sea 0.25 · "fog bank" [10, 20]: fog 1, gale 0.04, sea 0.2 · "fog lifting" 4: fog 0.25 |
| `clearing` | −0.4 / −0.2 / 0.1 | after gale, squall, fog | "the sky clears" [6, 10]: gale 0.3, thunder 0, fog 0, sea 0.6 (the swell outlasts the wind) |

Steer: **weather** toward wet favours freshening, gale and squall. **Energy** lifts the gale, sea and
thunder levers through `lively`. **Change** favours the squall and fog.

### Out-of-the-box
A storm you are safe inside, built mostly from the app's own physics: real wind spectra for railing,
vent and gale, real rain on a resonant dome, a sea below you. The vertical layout (sea under you, dome above,
and the lens turning at your side) exists in no other ambient app.

---

## 6. Scene 3 — Night train (`night_train.json`)

### Premise
Lower berth of a sleeper compartment, sitting up facing the direction of travel (+x), window on your left
(+y, 0.45 m). About 100 km/h (27 m/s) through the night. Under the floor the front bogie clicks over the
rail joints, then the rear one. A glass of tea rattles in its metal holder on the window table. Level
crossings come up ahead, ring past with their Doppler drop and are gone behind. Bridges boom hollow under
you, tunnels close in, rain streaks the glass. Once in a long while a train on the other track thunders
by. Sometimes the train slows through a sleeping station.

### Why it is spatially strong
- **Linear flow:** everything outside enters ahead, passes and leaves behind with a physically correct
  Doppler shift (drift motion at the train's speed). Unlike underwater drift it has hard, recognisable
  landmarks (bells, bridges) and an interior that rides with you.
- **Below and fore-aft:** the bogies are fixed 7.5 m ahead and 9.5 m behind at −1.6 m. The "da-dum… da-dum"
  alternates front and back under you.
- **Envelopment:** in a tunnel a 6-emitter `tunnel` ring roars around the forward axis, then opens again.
- **Near field:** the tea glass at 0.7 m front-left and the rain on the window at 0.5 m give crisp close
  detail against the low rumble.

### Environment
`time_of_day` 2.5, `wind_ms` 0, `rain_mm_h` 0, `ground_z` −1.23 (the compartment floor), no medium.

### Sound groups

| Group | id | Clip / model | Position / motion | Playback, interval | `level_db` | Notes |
|---|---|---|---|---|---|---|
| `rumble` | 10 | `car_interior` (sa25, 120 s) | fixed [0, 0, −0.5], emitters 2, spread 1.5 | loop, bed | 58 | the main bed |
| `rumble` | 11 | `rolling_noise` (el2 loop) | fixed [0, 0, −1.4], emitters 4, spread 2 | loop, bed | 60 | wheel/rail roar under the floor |
| `bogies` | 20 | `bogie_joints` loop | fixed [7.5, 0, −1.6] | loop, texture, rate [0.97, 1.03] | 70 | front bogie |
| `bogies` | 21 | `bogie_joints` loop (other take) | fixed [−9.5, 0, −1.6] | loop, texture | 68 | rear bogie |
| `crossings` | 30 | `crossing_bell` (2 takes, 14 s) | drift speed 27, ahead 160, behind 160, lateral [6, 14], height [−1.5, 0] | events [120, 360], fade 3 | 96 | ~12 s pass, Doppler |
| `passing` | 40 | `passing_train` (2 takes, 8 s) | pass from [180, −3.6, −0.6] to [−180, −3.6, −0.6], duration 6.5 | events [600, 1200], event | 86 | rare; tangential at 3.6 m |
| `bridges` | 50 | `bridge_rumble` (2 takes, 12 s) | fixed [0, 0, −2.5], emitters 3, spread 6 | events [300, 720], fade 2 | 76 | |
| `tunnels` | 60 | `tunnel_roar` (2 takes, 20 s) | fixed [0, 0, 0], emitters 6, spread 2.4, `tunnel: true` | events [420, 900], fade 3 | 74 | |
| `rain` | 70 | `rain_window` loop | fixed [0.15, 0.5, 0.05], emitters 2, spread 0.5 | loop, bed | 44 | crisp, close |
| `cabin` | 80 | `tea_glass` (3 takes) | fixed [0.55, 0.38, −0.42] | events [25, 70], rate [0.9, 1.1] | 40 | the classic glass holder |
| `cabin` | 81 | `panel_creak` (3 takes) | fixed [−0.3, −0.9, 0.6] | events [30, 90] | 44 | upper berth and panels |
| `cabin` | 82 | `vent_hum` loop | fixed [0, 0, 0.95] | loop, bed | 36 | |
| `horn` | 90 | `loco_horn` (2 takes) | fixed [260, 0, 2] | events [240, 600] | 104 | ~55 dB, far ahead |
| `station` | 95 | `station_pass` (2 takes, 20 s) | drift speed 8, ahead 80, behind 80, lateral [4, 6], height [−0.5, 1] | events [60, 60]; events lever 0 except while slowing | 70 | |

Physics note: the bogie loops are what you hear because the wheels ride with you. Joints are fixed in
the world, but the sound is emitted at the wheel, so it stays fixed relative to the car. The `pitch`
param of the loops changes their tempo together with the speed (joint rate ∝ speed). The `motion` of the
drift groups scales their drift speed, so landmarks slow down with the train.

### What to generate (≈ $0.92)

```json
[
 {"scene":"nighttrain","name":"car_interior","model":"sa25","duration":120,"variations":1,
  "prompt":"Field recording inside a sleeper train compartment at night at steady speed, deep low rolling rumble, soft rhythmic sway, faint rattle of fittings, constant, realistic, no music, no voices, no announcements."},
 {"scene":"nighttrain","name":"rolling_noise","model":"el2","duration":22,"loop":true,"prompt_influence":0.6,"variations":2,
  "prompt":"Steady low roar of steel train wheels rolling on continuous rails, heard through the floor of a passenger car, smooth broadband rumble, no clicks. Only the rolling noise, no music, no voices, dry, single source."},
 {"scene":"nighttrain","name":"bogie_joints","model":"el2","duration":22,"loop":true,"prompt_influence":0.65,"variations":2,
  "prompt":"Train wheels passing over rail joints at steady speed, the regular soft double knock da-dum, da-dum about once a second, heard from inside the carriage through the floor, low and muffled. Only the wheels, no horn, no music, no voices, dry, single source."},
 {"scene":"nighttrain","name":"crossing_bell","model":"el2","duration":14,"prompt_influence":0.6,"variations":2,
  "prompt":"A railway level crossing warning bell ringing steadily at night, fast regular metallic dings, heard outdoors from a few metres away, constant. Only the bell, no train, no cars, no music, no voices, dry, single source."},
 {"scene":"nighttrain","name":"passing_train","model":"el2","duration":8,"prompt_influence":0.6,"variations":2,
  "prompt":"A train on the next track passing close by in the opposite direction, heard from inside a closed train compartment, a soft pressure thump then a rushing rumble rising and falling away. Single pass, no horn, no music, no voices, dry."},
 {"scene":"nighttrain","name":"bridge_rumble","model":"el2","duration":12,"prompt_influence":0.6,"variations":2,
  "prompt":"A train crossing a steel truss bridge, heard from inside the carriage, the rumble turns hollow and booming with metallic resonance for a few seconds, then returns to normal. Only the bridge rumble, no horn, no music, no voices, dry."},
 {"scene":"nighttrain","name":"tunnel_roar","model":"el2","duration":20,"prompt_influence":0.6,"variations":2,
  "prompt":"A train entering a tunnel, heard from inside a compartment: the sound closes in with a pressure change, an enclosed roaring rumble for many seconds, then opens out again as it leaves the tunnel. No horn, no music, no voices, dry."},
 {"scene":"nighttrain","name":"rain_window","model":"el2","duration":22,"loop":true,"prompt_influence":0.6,"variations":2,
  "prompt":"Raindrops hitting and streaking along a train window at speed, heard from inside right next to the glass, soft fine tapping and trickling, steady. Only the rain on the window, no thunder, no music, no voices, dry, single source."},
 {"scene":"nighttrain","name":"tea_glass","model":"el2","duration":6,"prompt_influence":0.65,"variations":3,
  "prompt":"A teaspoon rattling softly in a glass of tea in a metal glass holder on a train table, a few quiet tinkles as the carriage sways, then still. Single sound, no music, no voices, dry."},
 {"scene":"nighttrain","name":"panel_creak","model":"el2","duration":8,"prompt_influence":0.65,"variations":3,
  "prompt":"Soft creaks of wooden and plastic panels in a swaying train compartment, a couple of gentle groans and ticks, then quiet. Single sound, no music, no voices, dry."},
 {"scene":"nighttrain","name":"vent_hum","model":"el2","duration":22,"loop":true,"prompt_influence":0.6,"variations":1,
  "prompt":"A quiet air conditioning vent in a train compartment ceiling, soft steady airflow hiss with a faint fan hum, constant. Only the vent, no music, no voices, dry, single source."},
 {"scene":"nighttrain","name":"loco_horn","model":"el2","duration":8,"prompt_influence":0.65,"variations":2,
  "prompt":"One long mellow diesel locomotive horn far ahead at night, heard from a carriage at the back of the train, soft and distant. Single sound, no music, no voices, dry."},
 {"scene":"nighttrain","name":"station_pass","model":"el2","duration":20,"prompt_influence":0.6,"variations":2,
  "prompt":"Passing slowly through an empty railway station at night, heard from inside a train: faint electrical buzz of platform lights, a soft signal bell, the echo of the train off the platform walls. No voices, no announcements, no music, dry."}
]
```

≈ 360 s el2 ($0.72) + 1 sa25 ($0.20).

### Levers

| Key | Title | Range [def] | `lively` | Drives |
|---|---|---|---|---|
| `speed` | Speed | 0..1 [1] | +0.3 | `rumble.level` [[0, −14], [1, 0]]; `rumble.pitch` [[0, 0.8], [1, 1]]; `bogies.level` [[0, −10], [1, 0]]; `bogies.pitch` [[0, 0.6], [1, 1]]; `crossings.motion` [[0, 0.3], [1, 1]]; `passing.events` [[0, 0.3], [1, 1]]; `station.events` [[0, 1], [0.4, 0.4], [0.55, 0]] |
| `track` | Old rails | 0..1 [0.5] | 0 | `bogies.level` [[0, −30], [0.5, −6], [1, 2]] |
| `rain` | Rain | 0..1 [0] | 0 | `rain.level` [[0, −30], [0.3, −8], [1, 3]] |
| `land` | Countryside | 0..1 [0.4] | +0.6 | `crossings.events` [[0, 0], [0.5, 1], [1, 2]]; `bridges.events` [[0, 0], [0.5, 0.6], [1, 1.6]]; `tunnels.events` [[0, 0], [0.5, 0.5], [1, 2]]; `horn.events` [[0, 0.2], [1, 1.5]] |
| `cabin` | Compartment | 0..1 [0.5] | +0.2 | `cabin.events` [[0, 0], [0.5, 1], [1, 2]]; `cabin.level` [[0, −12], [0.5, 0], [1, 3]] |

### Episodes (`only: true`, `start: "departure"`, free run)

| Episode | wet / energy / drama | after | Segments (minutes: lever targets) |
|---|---|---|---|
| `departure` | 0 / 0.2 / 0.4 | — | "pulling out" 2: speed 0.15, track 0.7 · "gathering speed" 3: speed 0.6 · "line speed" 2: speed 1, land 0.3 |
| `plains` | 0 / −0.4 / 0.1 | any | "open plains" [10, 20]: speed 1, track 0.2, land 0.3, cabin 0.4 |
| `old_line` | 0 / 0.2 / 0.3 | any | "old jointed line" [8, 15]: track 1, speed 0.8, land 0.6 |
| `hills` | 0 / 0.4 / 0.7 | plains, old_line | "into the hills" 3: land 0.8, speed 0.8 · "tunnels and bridges" [6, 10]: land 1, speed 0.7 · "out of the hills" 3: land 0.5 |
| `rain_belt` | 0.9 / 0 / 0.3 | any | "first drops" 3: rain 0.3 · "rain on the glass" [8, 15]: rain 0.8 · "rain eases" 4: rain 0.15 |
| `station` | 0 / 0 / 0.5 | plains, old_line, deep_night | "slowing" 2: speed 0.4 · "a sleeping station" 1.5: speed 0.25 · "away again" 3: speed 0.85 |
| `deep_night` | −0.3 / −0.9 / 0 | any | "deep night" [15, 30]: speed 0.9, land 0.1, cabin 0.2, track 0.3, rain 0 |

Focus check: in `plains` and `deep_night`, crossings come ~1 per 4–12 min, the tea glass ~1/min, and there
is no tunnel. The passing train is at most one per 10–20 min, tangential, and 8–10 dB under the bed at
its peak. `hills` is the lively spell (energy +).

### Out-of-the-box
The motion is the scene's own: Doppler crossings, bogies alternating under you, tunnels closing around
you. Plus one intimate, culturally specific near sound: the spoon in the tea-glass holder. A sleep and
focus classic ("train sounds" are a huge genre), here physically spatial instead of a stereo recording.

---

## 7. Scene 4 — Tea house (`tea_house.json`)

### Premise
A small tea room (4.5 tatami, ≈ 2.7 × 2.7 m) on a rainy afternoon. You kneel as the guest, ears 0.85 m
above the tatami. The host (never heard speaking) kneels in front of you, a little to the left. The iron
kettle simmers on the sunken hearth with the sound the tea masters call *matsukaze*, "wind in the pines",
over a few crackling coals. The shōji on your right is open to the garden: rain on maple leaves, moss and a
stepping stone, drops falling from the eaves, the soft trickle of a rain chain. A *suikinkutsu*, a jar
buried in the ground, turns drips into clear bell-like tones from below. Far away, a bamboo *shishi-odoshi*
knocks and a temple bell sounds. The ceremony is scripted: the utensils are purified, the tea is whisked,
the bowl is set before you, you contemplate, the host clears, and the eaves drip after the rain. Then it
begins again.

### Why it is spatially strong
- **Personal attention without a voice:** every host action happens 0.8–1.2 m in front of you at hand
  height (z −0.5..−0.8), with the slow, detail-focused, crisp sounds that ASMR research rates highest
  (Barratt et al. 2015, 2017). The bowl slides toward you and stops 0.45 m away: the scene's only approach,
  slow (4 s), and meant for you.
- **Three layers in height:** rain on the shingle roof above you (+1.4 m), the room at ear and hand height,
  the suikinkutsu **underground** behind-right (−1.5 m, below the floor level of the garden).
- **Open side:** the garden is on the right only. Turn your head and the garden comes to the front; the
  tea room keeps its quiet left side.

### Environment
`time_of_day` 15, `rain_mm_h` 4, `wind_ms` 0.5, `temperature_c` 17, `humidity` 0.95, `wetness` 1,
`ground_z` −0.85 (tatami).

### Sound groups

| Group | id | Source | Position / motion | Playback, interval | Level |
|---|---|---|---|---|---|
| `kettle` | 10 | clip `kettle_simmer` loop | fixed [1.0, 0.65, −0.75] | loop, texture | 46 dB |
| `kettle` | 11 | model fire `embers`, `{heat_kw: 1, gain: 0.3}` | fixed [1.0, 0.65, −0.85] | model | soft crackle |
| `cloth` | 20 | clip `fukusa` (3 takes) | fixed [0.95, 0.25, −0.55] | events [10, 20] | 42 dB |
| `ladle` | 21 | clip `ladle_pour` (3 takes) | fixed [1.1, 0.0, −0.6] | events [14, 28] | 44 dB |
| `whisk` | 22 | clip `whisk` (3 takes, 15 s) | fixed [0.85, 0.2, −0.7] | events [25, 40] | 44 dB |
| `host` | 23 | clip `tatami_shift` (3 takes) | pass from [1.3, 0.5, −0.75] to [0.95, 0.05, −0.75], duration 6, jitter 0.1 | events [30, 60] | 40 dB |
| `serve` | 24 | clip `bowl_slide` (3 takes) | pass from [0.85, 0.15, −0.8] to [0.45, 0, −0.82], duration 4 | events [70, 90] | 42 dB |
| `rain` | 30 | model rain `on_leaves` | fixed [1.5, −4.0, −0.3], emitters 4, spread 3 | model | garden |
| `rain` | 31 | model rain `on_soil` (moss) | fixed [−1.0, −3.0, −0.85], emitters 2, spread 2 | model | |
| `rain` | 32 | model rain `on_rock` | fixed [0.6, −1.8, −0.85] | model | the stepping stone |
| `rain` | 33 | model rain `wood_deck` (shingles) | fixed [0.3, 0, 1.4], emitters 4, spread 1.4 | model | overhead |
| `eaves` | 40 | model water `drips` | fixed [0.5, −1.5, −0.6], emitters 3, spread 1.5 | model | drops along the eave line |
| `eaves` | 41 | model water `trickle` | fixed [−1.2, −1.6, −0.4] | model | rain chain |
| `garden` | 50 | clip `suikinkutsu` (3 takes), `echo {delay 0.12, feedback 0.4, mix 0.5, damp_hz 3000}` | fixed [−1.8, −3.2, −1.5] | events [5, 14], rate [0.94, 1.06] | 50 dB |
| `garden` | 51 | clip `shishi_odoshi` (2 takes) | fixed [4, −9, −0.7] | events [45, 100] | 64 dB |
| `garden` | 52 | clip `furin` (2 takes) | fixed [0.3, −1.5, 0.9] | events [20, 60], rate [0.97, 1.03] | 46 dB |
| `temple` | 60 | clip `temple_bell` | fixed [−350, −420, 25] | events [480, 900] | 112 dB |

Levels at the listener, relative to the rain bed: host actions −2..−6 dB (they are the focus), kettle
−8 dB, garden events −8..−14 dB, temple bell −12 dB.

### What to generate (≈ $0.49)

```json
[
 {"scene":"teahouse","name":"kettle_simmer","model":"el2","duration":22,"loop":true,"prompt_influence":0.6,"variations":2,
  "prompt":"A cast iron Japanese tea kettle simmering just below the boil over charcoal, a soft steady rushing hiss like wind in pine trees with faint bubbling, close-up in a quiet room. Only the kettle, no whistle, no music, no voices, dry, single source."},
 {"scene":"teahouse","name":"fukusa","model":"el2","duration":8,"prompt_influence":0.65,"variations":3,
  "prompt":"A silk cloth being folded slowly and precisely and then wiping a small lacquered wooden tea caddy, soft crisp silk rustles and a faint snap of the cloth, close-up. Single action, no music, no voices, dry."},
 {"scene":"teahouse","name":"ladle_pour","model":"el2","duration":8,"prompt_influence":0.65,"variations":3,
  "prompt":"A bamboo ladle dipped into hot water and slowly poured into a ceramic tea bowl, a gentle splash and a thin clear pouring stream, then the ladle rested on the kettle, close-up. Single action, no music, no voices, dry."},
 {"scene":"teahouse","name":"whisk","model":"el2","duration":15,"prompt_influence":0.65,"variations":3,
  "prompt":"A bamboo whisk whisking matcha tea in a ceramic bowl, fast light rhythmic swishing and frothing, slowing down at the end with a few gentle strokes, close-up. Only the whisk and bowl, no music, no voices, dry."},
 {"scene":"teahouse","name":"tatami_shift","model":"el2","duration":8,"prompt_influence":0.65,"variations":3,
  "prompt":"A person in a silk kimono shifting position while kneeling on tatami mats, soft fabric rustle and a gentle slide of fabric on woven straw. Single movement, no footsteps, no music, no voices, dry."},
 {"scene":"teahouse","name":"bowl_slide","model":"el2","duration":5,"prompt_influence":0.65,"variations":3,
  "prompt":"A heavy ceramic tea bowl set down softly on a tatami mat and slid slowly forward a little, a muted knock and a gentle scrape on woven straw, close-up. Single action, no music, no voices, dry."},
 {"scene":"teahouse","name":"suikinkutsu","model":"el2","duration":10,"prompt_influence":0.65,"variations":3,
  "prompt":"Suikinkutsu water koto: single water drops falling into a buried clay jar, each producing a clear resonant bell-like tone with a hollow echo from underground, sparse with silence between. Only the drips, no music, no voices, dry."},
 {"scene":"teahouse","name":"shishi_odoshi","model":"el2","duration":4,"prompt_influence":0.65,"variations":2,
  "prompt":"A bamboo shishi-odoshi in a garden tipping and striking a stone once, a single hollow wooden knock with a little water splash, then silence. Single sound, no music, no voices, dry."},
 {"scene":"teahouse","name":"furin","model":"el2","duration":8,"prompt_influence":0.65,"variations":2,
  "prompt":"A small glass Japanese wind chime stirred by a faint breeze, two or three delicate high glass tinkles, then still. Single sound, no music, no melody, no voices, dry."},
 {"scene":"teahouse","name":"temple_bell","model":"el2","duration":15,"prompt_influence":0.65,"variations":1,
  "prompt":"One strike of a large Buddhist temple bell heard from far away across hills in the rain, a deep soft boom with a long slowly decaying hum. Single sound, no music, no voices, dry."}
]
```

≈ 245 s el2 ≈ $0.49.

### Levers

| Key | Title | Range [def] | `lively` | Drives |
|---|---|---|---|---|
| `rite` | Ceremony | 0..4 [0] (0 garden, 1 preparing, 2 whisking, 3 serving, 4 clearing) | 0 | `cloth.events` [[0.9, 0], [1, 1], [1.9, 1], [2, 0], [3.9, 0], [4, 1]]; `ladle.events` [[0.9, 0], [1, 0.8], [2, 0.8], [2.1, 0], [3.9, 0], [4, 1]]; `whisk.events` [[1.9, 0], [2, 1.2], [2.9, 1.2], [3, 0]]; `serve.events` [[2.9, 0], [3, 1], [3.9, 1], [4, 0]]; `host.events` [[0.9, 0], [1, 1], [4, 1]]; `kettle.level` [[0, −8], [1, 0], [2, 0], [4, −5]] |
| `rain` | Rain | 0..1 [0.6] | 0 | `rain.level` [[0, −30], [0.5, −2], [1, 5]]; `eaves.level` [[0, −6], [0.5, 0], [1, 3]] |
| `garden` | Garden | 0..1 [0.5] | +0.3 | `garden.events` [[0, 0], [0.5, 1], [1, 2]]; `temple.events` [[0, 0], [0.5, 1], [1, 1.6]] |
| `kettle` | Kettle | −12..+6 dB [0] | 0 | `kettle.level` [[−12, −12], [6, 6]] |

The `rite` lever is a stage selector. Its curves are steps, so a stage switches the right host groups on
(`events` > 0) and the others off. Because the script lands exactly on integers, stage boundaries are
clean. A user can also move it by hand to "ask for whisking".

### Direction: scripted, repeating (`only: true`, `start: "arrival"`, `repeat: true`)

| # | Episode (= script step) | Segments (label, minutes, lever targets) |
|---|---|---|
| 1 | `arrival` | "rain in the garden" 3: rite 0, rain 0.6, garden 0.5 |
| 2 | `preparation` | "purifying the utensils" 3: rite 1 |
| 3 | `whisking` | "whisking the tea" 2: rite 2 |
| 4 | `serving` | "the bowl is set before you" 1.5: rite 3 |
| 5 | `contemplation` | "rain and the water jar" 4: rite 0, garden 0.8, rain 0.7 |
| 6 | `clearing` | "clearing the utensils" 2: rite 4 |
| 7 | `after_rain` | "the eaves drip" 4: rite 0, rain 0.12, garden 0.6 |

Cycle 19.5 min. Every segment sets `"ease": 3`, so a stage switches within seconds instead of the default
~40 s. Segments that leave out `rain`, `garden` or `kettle` return them to their defaults (0.6, 0.5, 0 dB).
While the script runs, the steer axes only act through `lively` (the energy axis lifts `garden`). Give the
episodes `wet` (after_rain −0.5, contemplation +0.4) and `energy` anyway: a free variant without a script
(`repeat: false`) then picks among `contemplation` and `after_rain` by the weather steer, for a tea room
with no ceremony.

### Out-of-the-box
A ritual as a sound scene: a defined, beautiful 20-minute arc with real cultural sounds (matsukaze, the
underground water koto, shishi-odoshi) that most listeners have never heard spatially. The host is present
only through the hands.

---

## 8. Scene 5 — Singing ice (`singing_ice.json`)

### Premise
A clear, windless winter night on a large lake covered with new black ice, 5–10 cm thick. You sit on a
pad on the ice, ears 0.8 m above it. As the ice cools and contracts it cracks. In thin ice the bending
waves are **dispersive**: high frequencies run faster, so a crack far away arrives as a descending
whistle-chirp ("pew"), the famous singing ice. Near cracks are a sharp tock with a ringing tail. Sometimes
a crack **runs** across the lake: you hear it travel. Rarely one runs right **beneath** you. Deep booms roll
from far away. The shore forest murmurs 500 m off, fine snow hisses past, an owl calls from the shore, and
late at night a lone skater glides by in the distance.

### Why it is spatially strong
- **A plane of events around you:** every event lies on the ice plane (z −0.8), at all azimuths, from 8 m to
  400 m. With head tracking you turn toward each chirp and it stays there. This is open-field azimuth
  practice, the opposite of the body scan's near field.
- **Movement through the plane:** crack runs cross the front at 30–40 m. The rare "under" crack runs
  0.85 m below the ears from left-behind to right-front in 3 s: you feel it pass under you.
- **Distance as content:** the `distance` lever scales the singing field (×0.5..2), from an intimate pond to
  a vast lake. Air absorption at −14 °C and 70 % RH shapes the far chirps physically.
- **Silence:** the bed is very quiet. The scene is about the space between events.

### Environment
`time_of_day` 23, `temperature_c` −14, `humidity` 0.7, `wind_ms` 1.5, `rain_mm_h` 0, `season` "winter",
`ground_z` −0.8 (the ice: the ground reflection gives every event its image in the ice).

### Sound groups

Singing positions (azimuth°, distance m → [x, y, z]): 20°/90 → [84.6, 30.8, −0.8]; 75°/220 → [56.9, 212.5,
−0.8]; 140°/60 → [−46.0, 38.6, −0.8]; 200°/300 → [−281.9, −102.6, −0.8]; 260°/140 → [−24.3, −137.9, −0.8];
320°/180 → [137.9, −115.7, −0.8].

| Group | id | Source | Position / motion | Playback, interval | `level_db` |
|---|---|---|---|---|---|
| `shore` | 10 | model wind `forest_roar`, `{distance_m: 600, exposure: 0.4, gain: 0.5}` | fixed [−150, 500, 10], emitters 4, spread 300 | bed | model |
| `air` | 11 | model wind `breeze`, `{exposure: 0.3, gain: 0.4}` | fixed [0, 0, −0.5], emitters 3, spread 4 | bed | model |
| `air` | 12 | clip `snow_hiss` loop | fixed [0, 0, −0.75], emitters 4, spread 5 | loop, bed | 38 |
| `singing` | 20–25 | clip `ice_sing` (6 takes, each clip picks among all) | the six positions above | events [50, 160] each, rate [0.85, 1.15] | 98 |
| `cracks` | 30–32 | clip `ice_crack_near` (4 takes) | [12, −6, −0.8], [−9, 14, −0.8], [5, 18, −0.8] | events [120, 400] | 76 |
| `runs` | 40 | clip `ice_run` (3 takes, 5 s) | pass from [35, 70, −0.8] to [30, −70, −0.8], duration 3.5 | events [300, 700] | 92 |
| `runs` | 41 | clip `ice_run` | pass from [−60, −50, −0.8] to [−40, 60, −0.8], duration 4 | events [400, 900] | 92 |
| `depth` | 50 | clip `ice_run` (lowest take) | pass from [−40, 30, −0.85] to [40, −30, −0.85], duration 3 | events [600, 1200], first 400 | 64 |
| `depth` | 51–52 | clip `ice_boom` (3 takes) | fixed [−400, 250, −1], [350, −300, −1] | events [200, 600], rate [0.8, 1.0] | 110 |
| `life` | 60 | clip `owl` (2 takes) | fixed [−180, 420, 14] | events [90, 240] | 90 |
| `life` | 61 | clip `skater` (2 takes, 22 s) | pass from [28, 45, −0.8] to [18, −45, −0.8], duration 22 | events [240, 600] | 62 |

Use `jitter` 0 on the passes. The current jitter adds 0.5·σ to z, which would lift the ice events off the
plane. Rate check at defaults: 6 singing clips at ~105 s mean → ~3.4 chirps/min, mostly far and soft
(−10..−18 dB re the loudest), plus ~0.7 near cracks/min. That fits "≤ 1–2 noticeable events/min" because
the far chirps are soft and spectrally smooth. `frost` raises the rate in the cooling spells.

### What to generate (≈ $0.38)

First check Freesound and Wikimedia Commons for CC0 / CC BY recordings of singing ice ("singing ice",
"ice sounds lake", "black ice skating sound"). Real takes exist, and the house rule is to match field
recordings (`sound-ground-truth`). Otherwise generate:

```json
[
 {"scene":"ice","name":"ice_sing","model":"el2","duration":6,"prompt_influence":0.7,"variations":6,
  "prompt":"Singing ice on a frozen lake at night: a distant crack in thin black ice arriving as a descending whistling chirp, a laser-like pew sliding from high to low pitch with a short ringing tail, then silence. Single sound, no music, no voices, dry."},
 {"scene":"ice","name":"ice_crack_near","model":"el2","duration":4,"prompt_influence":0.7,"variations":4,
  "prompt":"A sharp crack in thin lake ice a few metres away, a clean high tock followed by a short metallic ringing ping that travels away across the ice. Single sound, no music, no voices, dry."},
 {"scene":"ice","name":"ice_run","model":"el2","duration":5,"prompt_influence":0.7,"variations":3,
  "prompt":"A long crack running fast across a frozen lake, a zipping series of ice pings and whistles sweeping past for a few seconds and fading. Single event, no music, no voices, dry."},
 {"scene":"ice","name":"ice_boom","model":"el2","duration":8,"prompt_influence":0.7,"variations":3,
  "prompt":"A deep distant boom rolling across a large frozen lake at night, a low thump of thick ice shifting with a long rumbling whoomp and a faint singing tail. Single sound, no thunder, no music, no voices, dry."},
 {"scene":"ice","name":"snow_hiss","model":"el2","duration":22,"loop":true,"prompt_influence":0.6,"variations":2,
  "prompt":"Fine dry powder snow drifting across smooth ice in a light breeze, a very soft continuous hiss, constant and gentle. Only the snow, no wind howl, no music, no voices, dry, single source."},
 {"scene":"ice","name":"owl","model":"el2","duration":6,"prompt_influence":0.65,"variations":2,
  "prompt":"A tawny owl hooting from a distant forest edge across a frozen lake at night, one long wavering hoot, then silence. Single call, no other birds, no music, no voices, dry."},
 {"scene":"ice","name":"skater","model":"el2","duration":22,"prompt_influence":0.6,"variations":2,
  "prompt":"A lone ice skater gliding past on natural lake ice at a distance, long smooth blade strokes with a soft scraping hiss and ringing of the ice under the blades, passing from one side to the other. Only the skater, no music, no voices, dry."}
]
```

≈ 191 s el2 ≈ $0.38.

### Levers

| Key | Title | Range [def] | `lively` | Drives |
|---|---|---|---|---|
| `frost` | Frost | 0..1 [0.5] | +0.4 | `singing.events` [[0, 0.2], [0.5, 1], [1, 2.2]]; `cracks.events` [[0, 0.1], [0.5, 1], [1, 2]]; `runs.events` [[0, 0], [0.5, 1], [1, 2]] |
| `depth` | Deep ice | 0..1 [0.3] | +0.2 | `depth.events` [[0, 0], [0.3, 1], [1, 2.5]]; `depth.level` [[0, −10], [1, 2]] |
| `drift` | Snow drift | 0..1 [0.2] | +0.3 | `air.level` [[0, −20], [0.2, −6], [1, 6]]; `shore.level` [[0, −10], [1, 4]] |
| `life` | Shore life | 0..1 [0.3] | +0.3 | `life.events` [[0, 0], [0.5, 1], [1, 2]] |
| `vast` | Vastness | 0..1 [0.5] | 0 | `singing.distance` [[0, 0.5], [0.5, 1], [1, 2]]; `runs.distance` [[0, 0.6], [1, 1.6]] |

### Episodes (`only: true`, `start: "dusk_cooling"`, free run)

| Episode | wet / energy / drama | after | Segments |
|---|---|---|---|
| `dusk_cooling` | −0.3 / 0.4 / 0.5 | — | "the ice tightens" [8, 14]: frost 0.85, depth 0.4, drift 0.15 |
| `still_night` | −0.5 / −0.8 / 0 | any | "deep still night" [15, 30]: frost 0.35, depth 0.2, drift 0.05, life 0.15 |
| `booming` | 0 / 0.3 / 0.7 | still_night, dusk_cooling | "the lake booms" [4, 7]: depth 1, frost 0.6 · "settling" 4: depth 0.3 |
| `snow_drift` | 0.7 / 0.2 / 0.3 | still_night, skater | "snow begins to drift" 4: drift 0.6 · "drifting snow" [6, 12]: drift 0.85, frost 0.3 · "calm again" 4: drift 0.15 |
| `skater` | −0.2 / 0.3 / 0.4 | still_night | "a late skater" 5: life 1 · "alone again" 3: life 0.2 |
| `coldest_hour` | −0.6 / 0 / 0.6 | still_night, booming | "the coldest hour" [8, 12]: frost 1, depth 0.5, drift 0 |

### Out-of-the-box
A real natural phenomenon that sounds like science fiction, played as it physically is: a dispersive
field on a plane around and under the listener, at true distances, in cold air. It is perfect for head
tracking (turn toward each chirp) and for sleep (long silences).

---

## 9. Engine needs

Ordered by importance. All are small. Everything else works with what exists plus the `path` motion.

1. **Lever targets in direction segments**: now present in the working tree (`Segment.levers`, `climate:
   false`, see `docs/direction.md` "Клип-сцены"). Nothing more is needed. One check: `ease` must also apply to
   lever transitions, because the tea house's `rite` stages want ~3 s, not the default 40 s.
2. **Exact cue times for event clips.** Either an `at: [t1, t2, …]` list (seconds from the scene start, no
   jitter), or `first` without the +0..20 % jitter when `interval` is `[∞, ∞]`. Needed only by the body
   scan's timed accents (ids 31–34). The scene works without them.
3. **Path clock tied to the script** (nice to have). Today a path runs on scene time. If the user cues
   `next` or the script repeats, the path does not follow. Option: `"clock": "script"` on a path, so its t is
   the direction's elapsed script time and restarts with it. Until then the body scan hides `cue` and does
   not repeat (reload to start again).
4. **Room reverb for clip scenes** (nice to have, no design depends on it): a small shoebox FDN (T60
   0.3–0.6 s) for the tea room and the compartment. Today the room is baked into the clips.
5. **A neutral name for model muffling** (cosmetic): `medium: "water"` is used in the lantern room only for
   its low-pass on physical models. An alias such as `"behind_glass"` with the same effect, or a per-group
   `lowpass_hz`, would make it honest.
6. **Optional "in-head" blend on path keyframes** (not required): a 6th column 0..1 that cross-fades the
   source to a diotic, HRTF-bypassed feed. It would make "inside the head" unmistakable. The current
   design gets close with the near-median position and the warm low timbre.

Not needed, explicitly: per-keyframe low-pass (timbre is done by cross-fading layers on one path), a
head-locked clip (the body path is world-fixed, which is right for a still torso), and new physical models.

## 10. Budget

| Scene | el2 seconds | sa25 takes | Estimate | With one re-roll |
|---|---|---|---|---|
| Body scan | 212 | 1 | $0.62 | ~$1.1 |
| Lantern room | 276 | 0 | $0.55 | ~$1.1 |
| Night train | 360 | 1 | $0.92 | ~$1.6 |
| Tea house | 245 | 0 | $0.49 | ~$1.0 |
| Singing ice | 191 | 0 | $0.38 | ~$0.8 |
| **Total** | 1284 | 2 | **$2.97** | ~$5.6 |

Everything else is procedural: wind (breeze, storm, fence_wire, gap_howl, forest_roar), rain (tin_roof,
on_leaves, on_soil, on_rock, wood_deck), water (lake_lapping, drips, trickle), fire (embers), thunder
(distant_rumble).

## 11. How to check each scene
- Render 10 min (`cargo run --release --example render`) and check that the bed holds 80–90 % of the
  energy and that the noticeable events per minute match the numbers above.
- Body scan: listen with head tracking. At each stage, turn the head ±30° and check that the point stays
  on the body part. Check that the crown and the feet are within ±3 dB of the 0.4 m level and that no
  boundary clicks or overshoots (Catmull-Rom loops show as a short back-and-forth).
- Lantern room: check that windward/lee is obvious and that the sea reads as **below** (tilt the head down).
- Night train: check Doppler on the crossings and that the bogie alternation reads front and back, not
  left and right.
- Singing ice: check that the far chirps are placed by azimuth within ~15° after a head turn, and that the
  under-crack is felt below rather than inside the head.
- Run the planner (`--example direction`) on each scene JSON for a few hours and check the episode chronicle
  and the lever traces.

## 12. Sources
- Barratt, E. L., & Davis, N. J. (2015). Autonomous Sensory Meridian Response (ASMR): a flow-like mental state. *PeerJ* 3:e851. https://peerj.com/articles/851/
- Barratt, E. L., Spence, C., & Davis, N. J. (2017). Sensory determinants of the ASMR. *PeerJ* 5:e3846. https://peerj.com/articles/3846/
- Poerio, G. L., Blakey, E., Hostler, T. J., & Veltri, T. (2018). More than a feeling: ASMR is characterized by reliable changes in affect and physiology. *PLOS ONE* 13(6):e0196645. https://doi.org/10.1371/journal.pone.0196645
- Smith, S. D., Fredborg, B. K., & Kornelsen, J. (2017). An examination of the default mode network in individuals with ASMR. *Social Neuroscience* 12(4):361–365.
- Fredborg, B. K., Clark, J. M., & Smith, S. D. (2018). Mindfulness and ASMR. *PeerJ* 6:e5414.
- Saberi, K., & Perrott, D. R. (1990). Minimum audible movement angles as a function of sound source trajectory. *JASA* 88:2639. https://sites.socsci.uci.edu/~saberi/saberiandperrott1990a.pdf
- Perrott, D. R., & Marlborough, K. (1989). Minimum audible movement angle: marking the end points of the path traveled by a moving sound source. *JASA* 85:1773.
- Middlebrooks, J. C., & Green, D. M. (1991). Sound localization by human listeners. *Annu. Rev. Psychol.* 42:135–159.
- Wightman, F. L., & Kistler, D. J. (1999). Resolution of front–back ambiguity in spatial hearing by listener and source movement. *JASA* 105:2841. 1994 abstract: https://www.auditory.org/asamtgs/asa94mit/5pPP/5pPP3.html
- Brimijoin, W. O., Boyd, A. W., & Akeroyd, M. A. (2013). The contribution of head movement to the externalization and internalization of sounds. *PLOS ONE* 8(12):e83068. https://pmc.ncbi.nlm.nih.gov/articles/PMC3846779/
- Brungart, D. S., & Rabinowitz, W. M. (1999). Auditory localization of nearby sources. Head-related transfer functions. *JASA* 106:1465.
- Canzoneri, E., Magosso, E., & Serino, A. (2012). Dynamic sounds capture the boundaries of peripersonal space representation in humans. *PLOS ONE* 7(9):e44306. https://www.ncbi.nlm.nih.gov/pmc/articles/PMC3460958/
- Kitagawa, N., Zampini, M., & Spence, C. (2005). Audiotactile interactions in near and far space. *Exp. Brain Res.* 166:528–537.
- Tajadura-Jiménez, A., et al. (2012). Action sounds recalibrate perceived tactile distance. *Current Biology* 22(13):R516.
- Kabat-Zinn, J. (1990). *Full Catastrophe Living.* Delacorte.
- Fischer, D., Messner, M., & Pollatos, O. (2017). Improvement of interoceptive processes after an 8-week body scan intervention. *Front. Hum. Neurosci.* 11:452. https://www.ncbi.nlm.nih.gov/pmc/articles/PMC5601051/
- Mirams, L., Poliakoff, E., Brown, R. J., & Lloyd, D. M. (2013). Brief body-scan meditation practice improves somatosensory perceptual decision making. *Consciousness and Cognition* 22(1):348–359.
- Lehrer, P. M., & Gevirtz, R. (2014). Heart rate variability biofeedback: how and why does it work? *Front. Psychol.* 5:756.
- Kaplan, S. (1995). The restorative benefits of nature: toward an integrative framework. *J. Environ. Psychol.* 15:169–182.
- Ratcliffe, E. (2021). Sound and soundscape in restorative natural environments: a narrative literature review. *Front. Psychol.* 12:570563. https://www.ncbi.nlm.nih.gov/pmc/articles/PMC8107214/
- Buxton, R. T., Pearson, A. L., Allou, C., Fristrup, K., & Wittemyer, G. (2021). A synthesis of health benefits of natural sounds and their distribution in national parks. *PNAS* 118(14):e2013097118.
- House rules and earlier reviews: `docs/scenes/forest.md`, `docs/research/focus-and-sound.md`, `docs/spatial-engine.md`.
