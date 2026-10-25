# Underwater drift — design note

The scene is fully procedural (no sound files): four models in `core/src/sources/`
(`whale.rs`, `dolphin.rs`, `reef.rs`, `sea.rs`, shared blocks in `marine.rs`) placed by
`core/scenes/underwater_drift.json`. It is built for focus and calm. An unpleasant sound
is the worst failure it can have, so comfort rules come before realism wherever the two
disagree. Every place where that happened is listed below.

## Why the old scene "floated"

The owner heard pitches drift and smear. There were three causes, and all three are gone:

| Cause | Old scene | Effect | Now |
|---|---|---|---|
| Doppler | `drift` motion at 2.0–3.2 m/s (current tunnel, whales, reef), `pass` dolphins | The engine propagates at the speed of sound in air (343 m/s), so each pass glided by ±0.6–0.9 % (≈ ±10–16 cents) | Moving sources drift at ≤ 0.4 m/s (≤ 0.12 %, ≈ 2 cents). Beds and reef stay fixed. Levers no longer drive `distance` (moving a 100 m source by 40 % over a lever ease is a Doppler glide too). The panel no longer offers `distance` or `pitch` |
| Rate jitter | `rate: [0.92, 1.04]` and similar on every clip, plus the group `pitch` control | Each event at a different playback speed (up to 8 %) | Models have no playback rate. Repeats vary by ≤ 1 % in pitch, as real song repeats do |
| Echo feedback | `echo` (0.3–0.6 s delays, feedback 0.4–0.6) on whale clips | Long, metallic smears of every call | A static 8-line FDN inside the whale/dolphin models: fixed delays, so it smears in time and never in pitch |

The Doppler effect was measured directly. A steady-carrier probe (cricket model, carrier
4.37 kHz) was rendered through the engine 70 m to the side, `fixed` vs `drift`. Carrier
range over the take: fixed ±22.5 cents (the model's own variation), drift 0.25 m/s ±22.6
cents (unchanged), drift 2.5 m/s −31…+42 cents (the old scene's speeds).

## The models

### Whale (`whale`, presets `humpback`, `humpback_distant`, `humpback_low`)

Humpback song is hierarchical (Payne & McVay 1971). *Units*, the single sounds, form a
*phrase*. A phrase repeated 2–4 times is a *theme*. The themes in a fixed order make the
*song*, which cycles with breathing pauses. Each seed builds its own song: 4–5 themes from
seven phrase templates. Each template copies units measured in the references:

* a series of short cries ~0.5 s apart, each held at one pitch (650–800 Hz, 0.23–0.35 s;
  Stimpert S1 0–3 s)
* a long, slowly falling moan, then a whoop rising from ~110 Hz
* 4–6 upsweep chirps ~0.9 s apart (370 → 650 Hz, 0.17–0.24 s), then a falling moan
  (Garland S2)
* two falling grunts (135 → 75 Hz), then a long wail rising by 20 % (S1 22–26 s)
* low moans (~85–120 Hz)
* a held wail, then a downsweep (S1 37–46 s)
* downsweep cries (1100–1250 → 650–750 Hz)

Held units (moans, wails) glide at a constant rate in cents per second, or not at all. See
"Floating tones" below for why.

A unit is a band-limited additive voice: up to 20 harmonics via the Chebyshev
recurrence. Harmonics fade out between 0.36 and 0.45 × sample rate, so nothing aliases even
mid-glide. The spectral tilt sets brightness, and a Gaussian formant bump (≈ 520 Hz for
moans, 1.3 kHz for cries) stands in for the resonance of the whale's air spaces. Pitch
moves only along a log-frequency contour: a straight glide for held units, and an eased
sweep or a monotone cubic (Fritsch–Carlson) through 3 knots for the short sweeps. There is
no vibrato and no jitter. Attack is 30–160 ms and release 60–350 ms (raised cosine). Grunts and moans get
a little band-passed breath noise. A 2-pole low-pass (`tone_hz`, distance) and the static
FDN (t60 1.8 s, wet 0.08) finish the voice. The second whale (`humpback_distant`) is
transposed down by 20 % (`pitch` 0.8), so two singers never hold nearly the same note.

### Dolphins (`dolphins`, presets `bottlenose_pod`, `distant_pod`, `whistles_only`)

Each of 1–4 animals has a *signature whistle*: an upsweep, downsweep, arch, U, sine or
flat-then-rise contour, 0.25–0.8 s, often in 1–2 loops. Two thirds of whistles are the
signature and the rest are variants. Another animal answers within 0.3–1.2 s (30 %).
Whistles have a fundamental plus harmonics at −18 and −30 dB. Echolocation clicks are
Gabor pulses (σ = 70 µs, 8.5–12.5 kHz) in trains of 12–42 clicks. Their intervals glide
smoothly from 45–90 ms by ±25 %, and the level follows the sin² of the beam sweeping past.
A *buzz* (intervals of 6–9 ms) is rare (10 % of trains) and 9 dB quieter.

Comfort: the default `pitch` 0.68 transposes the measured 7–17 kHz whistles to ≈ 4.8–12 kHz,
still inside the bottlenose range. A one-pole low-pass at 7 kHz and the engine's air
absorption over the 20–30 m distance soften the top. Whistles are the only tonal sound above
3 kHz in the scene, so they are short, sparse (8 per minute while the pod is near) and quiet.

### Reef (`reef`, presets `coral_reef`, `distant_reef`, `quiet_reef`)

Snapping shrimp (Alpheidae) produce cavitation snaps, which are broadband impulses. Their
timing is Poisson (900 snaps/s by default). With shrimp spread evenly over the bottom, the
amplitude follows P(A > a) ∝ a⁻², capped at 6× the faintest, about 12 dB over the median.
Louder (nearer) snaps take a brighter low-pass path (5 kHz, 3 kHz, 1.8 kHz). A 1 kHz
high-pass follows, then three fixed allpasses (1.3/2.9/4.7 ms) that model multipath over
the reef. Colony activity wanders slowly (OU, τ 40 s, σ 2 dB).

Fish calls use soft-onset damped pulses (the difference of two decaying phasors, rise
1–4 ms):

* damselfish chirps: 4–10 pulses, 31–41 ms apart, 380–950 Hz (measured on the Samoa clip:
  36 ms pulse period, 0.06–0.27 s trains)
* grunts: 5–12 pulses, 45–75 ms apart, 100–200 Hz, quiet
* knocks: 1–2 pulses, 180–450 Hz

### Sea (`sea`, presets `open_water`, `deep_water`, `diver_bubbles`, `vent_bubbles`, `rain_above`)

* **Bed.** This layer follows the Wenz/Knudsen shape:
  * flow pseudo-noise below 160 Hz
  * a broad wash band around 450 Hz
  * a faint tail above 1 kHz (measured slope −3/−6.5/−9 dB per octave at 1–2/2–4/4–8 kHz)
  
  The swell (period 10 s, jittered per cycle) moves the *level* of flow and wash by 4 dB
  peak to peak, with the wash lagging.
* **Bubbles.** Each bubble is a Minnaert resonator, f = 3.26/r, with van den Doel damping
  and a 0.4 ms soft rise.
  * A diver breathes out every ~5.5 s. Each exhalation is a 1.1–1.8 s burst whose rate peaks
    at 35–55 bubbles/s, with radii 1.5–6 mm (0.55–2.2 kHz) and a few large ones.
  * A vent releases a 6/s trickle at 1.1–2.2 kHz.
* **Rain above.** Each drop's impact pulse lands in a soft 1–4.5 kHz band, with an
  occasional larger-drop bubble (1.6–4 kHz). The real small-drop hump at 13–25 kHz
  (Medwin & Nystuen) is left out on purpose, because it is accurate but hissy and tiring.

## Ground truth

The reference recordings are read-only and not in the repo. Their licences are in
`assets/sounds/SOURCES.md` of the `new-game-modes-locations` tree:

* humpback: Stimpert et al. 2012 S1/S2, Garland et al. 2013 S2 (PLoS ONE, CC BY 2.5)
* dolphins: NOAA Fisheries PAG clips (common, striped, bottlenose, humpback dolphin)
* reef: the NOAA PMEL American Samoa hydrophone clip

The NOAA reef clip is band-limited to 2.2 kHz, and the PAG clips are 64 kbps mp3s.

Measured with `tools/analysis/underwater_measure.py` (numpy/scipy). Our renders were
measured the same way: model solos via `examples/model_wav`, and the scene through the
engine via `examples/clip_stems` and `examples/render`.

### Whale

| | units (s) p10/p50/p90 | gaps (s) p10/p50/p90 | f0 per unit (Hz) p10/p50/p90 | energy <200 / 200–1k / 1–3k Hz | contour residual (cents) p50/p90 | pitch reversals /s |
|---|---|---|---|---|---|---|
| Stimpert S1 | 0.23 / 0.53 / 2.65 | 0.16 / 0.62 / 1.82 | 75 / 540 / 726 | 12 / 66 / 14 % | 0 / 36 | 2.2 |
| Stimpert S2 | 0.26 / 0.42 / 2.96 | 0.09 / 0.29 / 2.44 | 84 / 515 / 659 | 26 / 66 / 6 % | 8 / 40 | 5.6 |
| Garland S2 | 0.17 / 0.39 / 1.02 | 0.67 / 1.57 / 2.91 | 99 / 337 / 526 | (noisy, 10 kHz band-limit) | 0 / 25 | 2.2 |
| ours, dry | 0.29 / 0.70 / 1.86 | 0.53 / 0.84 / 4.19 | 92 / 312 / 465 | 32 / 64 / 4 % | 16 / 34 | 0.6 |
| ours, in the scene | 0.44 / 0.90 / 2.67 | 0.42 / 0.65 / 5.12 | 93 / 355 / 622 | 16 / 75 / 9 % | 14 / 45 | 5.5 |

* **Residual:** the RMS of the tracked pitch around its 150 ms moving average, on stretches
  free of tracker jumps.
* **Reversals:** turning points of the tracked pitch with 20-cent hysteresis.
* **Pitch tracker confusion:** the tracker is easily misled by echoes and harmonics. One
  static 37 ms echo added to our dry render raises the residual from 16 to 207 cents
  before the jump filter.

The scene figures include two whales, the seabed reflection and the reverberation. The
cross-check of Doppler is the probe above. Compared with the recordings, our units are
slightly longer, and the high cry cluster sits a little lower (by choice, cries are the
brightest units). Our contours are steadier than the real voices (fewer reversals).

### Dolphins

| | whistle (s) p10/p50/p90 | dominant (Hz) p10/p50/p90 | range (oct) p50/p90 | click centroid (Hz) | click interval (ms) p50 |
|---|---|---|---|---|---|
| NOAA common | 0.17 / 0.38 / 0.75 | 9990 / 11921 / 12944 | 0.76 / 1.76 | 11249 | — |
| NOAA striped | 0.17 / 0.26 / 0.70 | 9164 / 10157 / 18008 | 1.70 / 2.24 | — | — |
| NOAA bottlenose | 0.30 / 0.49 / 0.61 | 9487 / 12058 / 14916 | 2.06 / 2.27 | 14104 | 65 |
| NOAA humpback dolphin | 0.16 / 0.28 / 0.56 | 5295 / 8314 / 8979 | 2.25 / 2.54 | 9546 | 73 |
| ours, `pitch` 1, dry | 0.35 / 0.39 / 1.21 | 7144 / 7738 / 8992 | 0.92 / 1.18 | 9019 | 55 |

The default `pitch` is 0.68, so the scene's whistles sit at ≈ 4.9–6.1 kHz dominant. Our
whistles sweep less (≈ 1 octave against 1–2.3) and lower, and the clicks are duller. Both
are deliberate: the reference clips are close-range recordings of animals working, while
the scene's pod is a polite 20–30 m away.

### Reef

| | centroid | 50 ms level spread p90−p10 | crest | standout snaps /s (4× local median) | fish pulse events /min |
|---|---|---|---|---|---|
| NOAA Samoa (≤ 2.2 kHz) | 849 Hz | 13.1 dB | 30.6 dB | 6.5 | 321 |
| ours, low-passed to 2.2 kHz | 1028 Hz | 6.2 dB | 27.4 dB | ≈ 0 | 19 |
| ours, full band | 2985 Hz | 6.3 dB | 23.7 dB | 0 | 19 |

The reference is a busy reef with a hydrophone on it. Ours is the same process heard calmer:
standout snaps are capped and smeared, so the crackle stays a texture, and fish call ~10×
less often. Full band, the crackle's energy is 27 % at 1–3 kHz, 38 % at 3–6 kHz and 19 % at
6–12 kHz. Its spectrum is smooth, with no resonance.

### Sea

There is no licensed open-ocean bed recording to compare with (see `SOURCES.md`), so the
bed follows the Wenz curves. Its peak is ≈ 90 Hz, and the slopes are −3, −6.5 and −9 dB per
octave above 1, 2 and 4 kHz. The swell is 4 dB peak to peak at 10 s.

## Comfort checks

The 10-minute scene render (`examples/render 600 … turn`) was measured with
`underwater_measure.py comfort`. The scene test (`core/tests/underwater_drift.rs`) asserts
the same limits on 40 s every run.

| Check | Limit | 10 min take | 3 min with rain above at 0.8 |
|---|---|---|---|
| Largest sample step / local (5 ms) RMS step | no clicks: < 6× | 5.0×, none over 6× | 6.2×, 2 samples over 6× (rain impacts, genuinely tiny clicks, soft) |
| Peak | < −1 dBFS | −15.6 dBFS | −11.0 dBFS |
| Short-term (3 s) loudness over the take's average | < 8 dB | max 3.8 dB, p95 1.8 dB | max 3.2 dB |
| Momentary (400 ms) over the preceding 10 s | small | max 5.7 dB | max 4.8 dB |
| Narrow tones above 3 kHz | none sustained | only dolphin whistles (0.25–0.8 s) | same |
| Energy 3–6 kHz | low | 2.8 % | 5.7 % |

Stem balance (600 s, leveller off, `examples/clip_stems`, RMS over the whole take including
the time a group is absent): current −38.0 dBFS, whales −44.5, reef −53.8 (the
`calm_drift` episode holds the reef lever at 0.2–0.5), bubbles −50.8, dolphins −60 (before
the final +3 dB), rain above −67.6 (lever at 0).

## Floating tones (comfort analyzer follow-up)

`examples/comfort` (src/comfort.rs) tracks spectral peaks. A tone held ≥ 0.8 s whose
straight-line-detrended pitch swings ≥ 2 cycles at 0.5–10 Hz counts as a floating tone.
Over 30 minutes (`… comfort -- 30 core/scenes/underwater_drift.json turn worst=12`) the first
version of this scene reached 20 floating tones per hour, the failure threshold. All of them
were whale units at 200–1050 Hz, 1–1.5 s long, wavering by 9–20 cents at 1.4–2.9 Hz.

The causes, one by one:

| Cause | Fix |
|---|---|
| Contour knots joined by a per-segment smoothstep. The pitch stopped at every inner knot, so a falling moan fell in a staircase. A line fitted through that leaves a 2–3 Hz ripple | Three or more knots use a monotone cubic (Fritsch–Carlson): no stall, no overshoot |
| Templates with a deliberate swell. The wail went up–down–up, and each cry in a 0.5 s series made a small arch. Bridged by the reverberation, the series became one tone wavering at 2 Hz | Series cries are held at one pitch. The swell is gone |
| Two whales on almost the same note. Two tones a few Hz apart beat, and the analyzer sees one tone wavering (found by removing the second whale: its events vanished) | The second whale is transposed down by 20 % |
| The reverberation of a glide. The tail holds the pitch of 60–150 ms ago, and against the direct sound it beats at (glide rate × delay), 1–6 Hz | Model reverb lowered (wet 0.25 → 0.08, distant 0.5 → 0.15). Held units glide gently, so the beat stays slow and shallow |
| Curved glides on held units (arches, eased falls) in the scene's bed noise. One hump plus noise reads as several swings | Held units glide at a constant rate in cents per second (`Ease::Linear`). The detrend leaves nothing to waver |

Each whale probe alone, through the engine (fixed or drifting at 0.25 m/s), scores 0 floating
tones. So the engine's propagation is not the cause; the mix and the contours were.
`examples/comfort` gained a `clarity=` option. It was used to rule out the mix's
masking-aware ducking (clarity 0 changed nothing).

| 30 min, `turn` | before | after |
|---|---|---|
| default seed | 20.0 /h (WARN) | 0 /h |
| seed 1 | 8.0 /h and loudness surprise 11.1 LU (WARN) | 2.0 /h (one tone, 6.6 cents), no warnings |
| seed 7 | 4.0 /h | 0 /h |

The "before" column for seeds 1 and 7 was measured after the first contour fix.
After the fixes, all three runs pass every check with no warnings.

## Known weaknesses

* **Tracker figures.** The whale's spectrum still has more energy below 200 Hz than the
  references (the recordings' noise floor sits higher up). Units in the scene are a little
  longer than real ones.
* **Repetitive voices.** The song library has seven phrase templates. Over hours a
  listener may recognise them, though each seed orders and tunes them differently.
* **Reef realism.** The reef is deliberately calmer and softer than a real one. Someone
  who knows reef recordings will miss the sharp, close snaps.
* **No sea-water absorption.** The engine still uses air absorption and c = 343 m/s
  underwater. That is why motion is kept slow; a faster source would glide audibly.
  Seawater absorbs far less at these frequencies; the air absorption acts as a gentle
  distance low-pass instead.
