# Body scan (`body_scan.json`)

Date: 2026-10-25. Status: procedural, no sound files. Every sound is the `bowl` model
(`core/src/sources/bowl.rs`): a rubbed singing bowl for the guide, struck bowls and a tingsha pair
for the accents. The route, the script and its labelled stages are those of the concept
([new-scenes-concepts.md](new-scenes-concepts.md), section 4); the guide's levels along the route are
measured, not hand-set (`tools/synth/level_guide.py`).

## What you hear

| Sound | Model | Where, when | Why |
|---|---|---|---|
| **The guide** | `bowl` preset `singing`: a rubbed bowl, no strike | One point travelling the route for 5 min: in from the front, around the head, over the crown, *inside the head*, throat and shoulders, down the left arm and across to the right, chest and heart, belly, both legs to the soles, back up and out | It names the body part by being there. Pure tone, no friction noise: earlier generated "brush"/"air" takes hissed as they moved ("like Parseltongue"), tones never did |
| guide timbres | 5 clips on the same path with their own gain columns, cross-fading | 220 Hz around the head, crown and the return; 110 Hz inside the head (warm, low, quiet); 330 Hz arms, chest; 165 Hz left arm and leg; 220 Hz the right-leg twin | A change of colour per region without any pitch glide: each tone is fixed, only the gains move. The guide tones are A2, E3, A3, E4 (fifths and octaves): any two that overlap in a cross-fade are consonant |
| **Opening bowl** | `large` (110 Hz) | 1 s, 1.5 m ahead, a little low | "The bowl is set before you": one soft felt strike, a minute-long ring |
| **Tingsha** | `tingsha` (2.10 / 2.14 kHz) | 140 s at the left fingertips, 160 s at the right | The turn of each hand: a small bright shimmer, tamed |
| **Feet bowl** | `large`, f0 146.8 Hz | 264 s, by the feet (1.2 m below the ears) | Grounding at the soles; between the guide's tones (no unison to beat against) |
| **Closing bowl** | `large` (110 Hz) | 292 s, ahead | The return |

Levers: **Guide** (−12..+6 dB on the guide group) and **Bowls** (−30..+4 dB on the strikes), as before.
The rest episode (afterglow, stillness) follows the 301 s script; the last bowl rings into it.

## The physics in the model

**Modes of a bowl.** A Himalayan bowl is a thick, squat shell that radiates from its (n, 0) flexural
modes. For a cylinder clamped at the base f(n,0) ∝ [(n²−1)²/(1+1/n²)]^½: 1 : 2.83 : 5.42 : 8.77 : 12.9
(Terwagne & Bush 2011). Real bowls run flatter: 2.7–2.9, 4.8–5.7, 7.5–9.1, 10.6–13.1 (Inácio,
Henrique & Antunes 2006, via Gołaś & Filipek 2016); the bowls measured below average
**1 : 2.79 : 5.22 : 8.2 : 11.7**, which the model uses.

**Doublets.** No bowl is round: every mode is a pair split by a fraction of a hertz to a few hertz
(Terwagne & Bush: "two peaks separated by several Hz ... a beating mode is heard"). A strike excites the
pair unequally, depending on where it lands; the sum beats at the split. Each strike here draws the
partner's share (preset −9 dB, ±3 dB), never closer than −3 dB: equal partners cancel to silence once a
beat. The main mode sits exactly on the pitch, the partner just above.

**Decay.** Each mode has its own T60, the fundamental longest; measured T60 falls with frequency as about
f^−0.75 (fitted on the 4.5" and SingingBowl1 recordings: slopes −0.67 and −0.91).

**Felt strike.** The mallet's force is a critically damped pulse t·e^(−t/τ) (contact ≈ 4 ms). Each mode
takes it in its own rotating frame, so its amplitude rises smoothly along the pulse's integral (10→90 %
in ≈ 3.4 τ) and ends exactly at its designed level. Driving the resonators with the bare pulse instead
gave a forced, off-resonant response: a low thump 15 dB over the ring in the first 100 ms (caught by the
`strike_has_no_thump` test). Mode levels follow a tilt of −6 dB/octave (soft felt; a hard mallet is near
0 dB) and a fourth-order roll-off above 2.5 kHz.

**Singing.** A rubbed bowl locks the (2,0) mode to the puja's stick-slip; the spectrum is that mode plus its
harmonics (2 f0 measured 29–31 dB under in the rubbed recordings) and a little (3,0). The model's singing
voice is pure oscillators: f0 with its doublet partner (0.083 Hz, −20 dB: a sway once in 12 s, ±0.9 dB),
2 f0 at −22 dB, (3,0) at −40 dB, and a slow breath (±6 %, 12 s and 19 s). No noise at all.

**Tingsha.** Two thick small cymbals, struck edge to edge. The pair measured rings at 2.86 and 2.93 kHz
(two cymbals 74 Hz apart: a roughness, not a beat), with upper modes at ×1.88 and a clash whose spectral
centroid is 6.2 kHz in the first 100 ms: 98 % of its energy is in 2–6 kHz, the band the ear is most
sensitive to. Ours is tamed: a lower pair (2.10 and 2.14 kHz), the second cymbal 1.6 Hz above and 4 dB
under the first (a 1.6 Hz shimmer of ~12 dB instead of a 74 Hz rasp), the ×1.88 mode 20 dB under, soft
contact (2 ms), T60 6 s, and a low level (39 dB at 1 m, ~0.75 m from the ears, at the fingertips).

**Cues.** Strikes are clip events: `first` + `exact` (and `interval` for repeats) bump the emitter's
`Controls::cue`; the model strikes on a new value (held while the clip's `activity` is 0). No engine
change was needed for this.

## Ground truth: recordings vs the model

Measured with `tools/analysis/bowl_partials.py` (partials by long-FFT peak picking, T60 by a straight-line
fit to each partial's band-passed envelope, beats from the de-trended envelope). Our renders are the
model alone, mono (`cargo run --release --example model_wav -- bowl <preset> 30 out.wav`).

### Struck bowls

| Recording | f1 (Hz) | ratios of partials 2..5 | T60 f1 / f2 / f3 (s) | doublet split f1 (partner) | partials 2..4 re f1 at the strike | attack, centroid 0–100 ms |
|---|---|---|---|---|---|---|
| Freesound 411486 (s-light) | 180.2 | 2.877, 5.364, 8.369 | 41 / 69 / 45 | 2.1 Hz (−4 dB), beat 17 dB | +2.4, −13.7, −20.6 dB | 1 ms, 489 Hz |
| Freesound 848027 (poiqwepoi) | 169.3 | 2.85, 5.39 | 118 / 86 / 65 | 0.94 Hz (−13 dB) | | 6 ms, 457 Hz |
| Commons SingingBowl1 | 105.2 | 2.828, 5.339, 8.427, 12.134 | – / 68 / 50 | 0.97 Hz (−27 dB); f3 1.72 Hz (−8 dB) | +5.5, −3.5, −3.9 dB | 32 ms, 430 Hz |
| Commons 11" bowl | 100.6 | 2.98, 5.74, 9.27 | 54 / 49 / 42 | 0.8 Hz (−32 dB) | | 8 ms (mains hum in the take) |
| Commons 4.5" bowl (hard hit) | 361.1 | 2.767, 5.171, 7.987, 11.337 | 48 / 41 / 24 | 7.0 Hz (−13 dB) | −0.8, +2.9, −1.6 dB | 1 ms, 2045 Hz |
| Freesound 697529 | 517.4 | 2.796, 5.131 | 30 / 20 / 10 | 2.35 Hz (−10 dB) | +3.2, +8.3 dB | 1 ms, 3657 Hz |
| Freesound 271370 | 698.7 | 2.691 | 10 / 7 | 1.24 Hz (−5 dB) | +6.8 dB | 4 ms, 1723 Hz |
| Gołaś & Filipek, bowls 1 / 2 | 472 / 334 | 2.77, 5.03, 7.66, 10.60 / 2.53, 4.61, 7.25, 10.44 | | | | |
| Old take `bowl_strike_el2_1..3` | 676–735 | 1.94 (one weak partial) | 10–14 | 1.0–3.4 Hz (0 dB) | −19 dB | 6–23 ms, 680–850 Hz |
| **Model `large`** | 110.0 | 2.79, 5.22, 8.2, 11.7 | 61 / 28 / 19 | 0.70 Hz (−9 dB), beat 6.2 dB | −11.4, −18.1 dB | 14 ms, 160 Hz |
| **Model `medium`** | 220.0 | same | 40 / 19 / – | 0.90 Hz (−9 dB), beat 6.2 dB | −11.7 dB | 11 ms, 307 Hz |
| **Model `small`** | 330.0 | same | 30 / 16 / – | 1.20 Hz (−9 dB) | −10.3 dB | 7 ms, 433 Hz |

The old generated strikes were near-pure 700 Hz tones without a bowl's partial series. The model has the
measured series and decays, doublets in the measured range (on the slow, shallow side: −9 dB partners
instead of −2..−5, beats 0.7–1.2 Hz on the fundamental), and a deliberately soft strike: upper partials
11–18 dB under the fundamental where hard mallets put them level with it, attack 7–14 ms instead of 1 ms,
no energy in 2–6 kHz.

### Tingsha

| | ring (Hz) | upper mode | T60 | beat | centroid 0–100 ms | 2–6 kHz share (first 0.5 s) |
|---|---|---|---|---|---|---|
| Freesound 435074 (steffcaffrey) | 2860 + 2934 (two cymbals) | ×1.88, −14 dB | 7.4 / 3.5 s | splits 13–16 Hz, cymbals 74 Hz apart | 6200 Hz | 98 % |
| Old take `tingsha_el2_1` | 6000 / 6123 | – | 2.9 s | 3.3 Hz | 6106 Hz | (above 6 kHz) |
| **Model `tingsha`** | 2100 + 2101.6 | ×1.88, −20 dB | 6.4 / 5.3 s | 1.6 Hz, 14.5 dB | 2120 Hz | 100 % (at 2.1 kHz, quiet) |

### Rubbed (singing) bowls

| | f0 (Hz) | 2 f0 | (3,0) | doublet |
|---|---|---|---|---|
| Commons Japanese rin, friction | 814.3 | −30.8 dB | ×2.668, −15 dB | 0.94 Hz (−4 dB) |
| Commons 4.5" bowl, rubbed | 523.7 | −28.8 dB | – | 1.27 Hz (−3 dB) |
| Freesound 417115 (dersinnsspace) | 269.6 | – | – | a 6.1 Hz pair at −3 dB |
| Old guide loop `guide_brush_el2_1` | 220.0 | −20.3 dB | – | 0.083 Hz |
| **Model `singing`** | 220.0 | −20.5 dB | −40 dB | 0.083 Hz (−18 dB measured) |

Rubbed recordings beat hard (−3..−4 dB partners: 15 dB swings at ~1 Hz); for a guide that has to stay
even while it travels, the model keeps the old guide's choice: a sway once in 12 s, under ±1 dB.

## Comfort measurements (the session as rendered)

`tools/analysis/render_check.py` on `render 310 ... turn` (the head sweeping ±90°) and `still`:

| Check | Result | Rule |
|---|---|---|
| Peak | −20.6 dBFS | < −1 dBFS |
| Integrated loudness | −30.6 LUFS (scene target −30) | |
| Loudest momentary / short-term over the integrated | +5.9 LU (105.6 s) / +5.4 LU (26 s) | ≤ ~8 dB |
| Guide pitch, every tone, p1..p99 of the 50 ms instantaneous frequency | 0.6–1.3 cents; 1 s means drift ≤ 1.2 cents | steady |
| Bowls' pitch excursion | 6–13 cents at the beat minima (doublet phase), tingsha 35 cents | physical, fixed sources |
| Clicks (peak above 7 kHz re the local RMS) | ≤ −36 dB, all at the −60 dBFS gate: 16-bit quantisation (−92..−96 dBFS) | none |
| 2–6 kHz share | 0.06 % of the session; 4 % in the loudest 3 s window (tingsha) | no piercing band |
| Hiss | tonal share of every model take ≥ 0.9998 | none |
| Guide evenness (guide alone, mix dynamics off, 3 s windows) | p5..p95 of the error to the target 0.6 dB, worst 1.7 dB | even; 3 dB under inside the head (designed) |

Doppler: the route moves at most ~0.1 m/s (0.03 %, 0.5 cent) and the measured guide pitch confirms it.
With the mix's dynamics on, the leveller lifts the "around the head" stage by up to ~5 LU at 20–37 s
after the quiet start (it did +3.5 LU on the previous version too): a slow swell, inside the rule, but
the one remaining unevenness. It is the engine's leveller, not the scene.

### What the measurements caught (and fixed)

* The guide swayed ±4 dB along the route with the model tone: the old path gains had been fitted, second
  by second, to the old loops' own beat phases. `level_guide.py` now measures with the guide's sway off,
  the mix dynamics off (`render ... still+nodyn`) and a 5 s smoothed error, on every keyframe (it used to
  skip odd seconds, which left the gains alternating 5× between neighbours).
* Twin and grain (both legs) at the same 165 Hz interfered as they moved; the twin is now 220 Hz.
* The opening strike at 220 Hz beat against the 220 Hz guide for a minute; the strikes now sit at
  110 / 146.8 Hz, where no guide tone is.
* Engine: a clip path entering the near-field zone (r < 1.2 m) started the NFC crossovers cold: a crackle
  ~15 dB over the signal's top end (every take, at 5 s). The crossovers now run warm outside the zone.
* Engine: a propagation path that went to sleep (silent input) kept old audio in its delay line; on wake
  it read the first delay's worth of it back (a burst when a guide layer came back in, 176 s). The line
  is cleared when the path sleeps.

## Listening renders (not in git)

`build/listen/`: `body_scan_full.wav` (whole session, head turning), `body_scan_still.wav`,
`solo_strike.wav` (the three bowls), `solo_tingsha.wav` (both hands), `solo_guide.wav` (the guide alone,
head turning). Make the solo scenes with any clip subset of `body_scan.json` and render with
`core/examples/render.rs`.

## Sources

Measured only (downloaded to `build/ref/`, never committed):

| Recording | Author | Licence | URL |
|---|---|---|---|
| singing bowl - single strike 6 | s-light | CC0 | https://freesound.org/s/411486/ |
| Tibetan singing bowl one strike. | poiqwepoi | CC0 | https://freesound.org/s/848027/ |
| Singing Bowl - Single Strike with Fade-Out | Fumbananana420 | CC0 | https://freesound.org/s/697529/ |
| singing bowl strike sound | inoshirodesign | CC0 | https://freesound.org/s/271370/ |
| Tingsha Cymbal | steffcaffrey | CC0 | https://freesound.org/s/435074/ |
| Tibetan bowl_rubbing rim | dersinnsspace | CC0 | https://freesound.org/s/417115/ |
| SingingBowl1.ogg, SingingBowl2.ogg | BambooBeast | public domain | https://commons.wikimedia.org/wiki/File:SingingBowl1.ogg |
| Tibetan Singing Bowl hit 4.5inch / hit 11inch / 4.5inch (rubbed) | Wikimedia Commons user | CC BY-SA 4.0 | https://commons.wikimedia.org/wiki/File:Tibetan_Singing_Bowl_hit_4.5inch.flac |
| Japanese rin played as friction idiophone | MichaelMaggs | CC BY-SA 4.0 | https://commons.wikimedia.org/wiki/File:Japanese_rin_played_as_friction_idiophone.ogg |
| Singing bowl.ogg | Wikimedia Commons user | CC BY-SA 3.0 | https://commons.wikimedia.org/wiki/File:Singing_bowl.ogg |
| The previous generated takes (`generated/body_scan/*`) | ElevenLabs via fal, and `bowl_tones.py` | ours | assets/sounds/SOURCES.md |

Literature:

* D. Terwagne, J. W. M. Bush, "Tibetan singing bowls", Nonlinearity 24 (2011) R51; arXiv:1106.6348.
  (n,0) modes, f ∝ a/R², doublet splitting and beating, rubbing locks the lowest mode with its harmonics.
* O. Inácio, L. Henrique, J. Antunes, "The dynamics of Tibetan singing bowls", Acta Acustica united
  with Acustica 92 (2006) 637–653. Modal identification of bowls, struck and rubbed simulations.
* A. Gołaś, R. Filipek, "Digital synthesis of sound generated by Tibetan bowls and bells", Archives of
  Acoustics 41 (2016) 139–150, doi:10.1515/aoa-2016-0014. Measured modal frequencies of two bowls; quotes
  Inácio et al.'s ratio ranges.
