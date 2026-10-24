# Tent in the rain: sound sources (`assets/sounds/generated/mountain_tent/`)

For the integrator to merge into `assets/sounds/SOURCES.md`. Collected on 2026-10-24.

The scene runs on real recordings wherever a free licence allowed it, plus the engine's own physical
models. Only the tent-pole creaks are generated (fal.ai ElevenLabs SFX v2, $0.032).

The recordings are Freesound **HQ previews** (Ogg Vorbis, about 192 kbit/s), not the original uploads,
because Freesound downloads need a login. Every sound used is **CC0 1.0**: no attribution is required,
but the authors are credited below as a courtesy. The licence was read from each sound's page.

Post-processing (`tools/synth/mountain_tent_sounds.py`):
- stereo mixed to mono, resampled to 48 kHz, 16-bit WAV;
- a zero-phase 2nd-order high-pass: 40 Hz for the fabric, Foley and rain, 25 Hz for the creek and thunder;
- loops: a 2 s equal-power cross-fade of the tail into the head;
- events: 10–250 ms fade-in, 150 ms–3 s fade-out;
- peak normalised to −3 dBFS, with the gain capped at 8× (+18 dB).

The engine normalises every file to unit loudness on load, so the scene's `level_db` sets the level.

## Recordings (CC0 1.0)

| File(s) | Source (Freesound id, author, title) | Cut |
|---|---|---|
| tent_fly_light_1.wav | [397916](https://freesound.org/s/397916/) Bronxio, "Inside a tent with rain (good for loop)" | 0–51 s, loop |
| tent_fly_light_2.wav | [816110](https://freesound.org/s/816110/) Nordliecht, "RainInTent" (Les Bioux, Vaud, Switzerland; sparse big drops) | 1.5–28 s, loop |
| tent_fly_steady_1.wav, tent_fly_steady_2.wav | [592997](https://freesound.org/s/592997/) Petrosilia, "heavy rain on a small tent" (17 min) | 160–222 s and 470–532 s (windows with no level or low-band outliers), loops |
| tent_fly_heavy_1.wav | [484724](https://freesound.org/s/484724/) Breviceps, "Heavy Rain on a tent" | 0–49.6 s, loop |
| tent_fly_heavy_2.wav | [427879](https://freesound.org/s/427879/) Sassaby, "Heavy Rain In Tent Catherdral Ranges" | 34–52.8 s (the even, heaviest part), loop |
| tent_flap_1..3.wav | [569539](https://freesound.org/s/569539/) bmacphail, "Tent Fabric Blow in Heavy Wind" | 28.6–33.0, 38.9–44.2, 15.8–20.6 s |
| tent_flap_4.wav | [569538](https://freesound.org/s/569538/) bmacphail, "Tent Fabric Blow in Light Wind" | 25.2–27.6 s |
| tent_thunder_roll_1.wav | [577315](https://freesound.org/s/577315/) TRP, "Thunder, single long rolling, distant, light rain, TORONTO, 2011" | 0.5–31 s |
| tent_thunder_roll_2.wav | [717890](https://freesound.org/s/717890/) TRP, "230823 Thunder, dry distant rolling, R-07 EM272s, Stratford 12pm" | 126.5–151 s |
| tent_thunder_roll_3.wav | [572443](https://freesound.org/s/572443/) TRP, "180806 Thunder, distant rolling boomy, rain drops TORONTO" | 0.8–25 s |
| tent_creek_1.wav | [423623](https://freesound.org/s/423623/) vollkornbrot, "Creek in the Alps" | 8–70 s, loop |
| tent_creek_2.wav | [869104](https://freesound.org/s/869104/) bassimat, "Mountain Stream Flowing Past from the Riverbank, Boscoverde, Dolomites - Torrente Pettorina" | 245–307 s, loop |
| tent_bag_rustle_1..2.wav | [776275](https://freesound.org/s/776275/) Joedozer, "Sleeping Bag Struggle" | 3.45–5.0 and 8.75–10.4 s |
| tent_bag_rustle_3..4.wav | [658410](https://freesound.org/s/658410/) IENBA, "Nylon Jacket or Raincoat" | 0–2.6 and 5.35–8.0 s |
| tent_zipper_1..2.wav | [438985](https://freesound.org/s/438985/) vintage2005, "Zipper for a tent or large bag" | 3.4–6.9 and 12.0–14.7 s |
| tent_zipper_3.wav | [569540](https://freesound.org/s/569540/) bmacphail, "Zipper Zipping Slow" | whole |

## Generated (fal.ai, `tools/fal/prompts_mountain_tent.json`)

| Files | Model | Prompt name | Cost |
|---|---|---|---|
| tent_pole_creak_el2_1..4.wav | el2 (ElevenLabs SFX v2), 4 s, prompt_influence 0.65 | tent_pole_creak | 4 × $0.008 = $0.032 |

Each 4 s take was cut down to the 1.4 s around its loudest moment. The reason is in the design note.

## Physical models (no files)

| Group | Model and preset |
|---|---|
| `meadow` | rain `meadow` (rain on grass all around, 6 emitters on a 7 m ring) |
| `drips` | water `drips`, `drips_hz` 0.9, `drop_height_m` 0.8, at the fly's hem on both sides |
| `wind`, `gusts` | wind `breeze` (exposure 0.6) and `storm` (exposure 1.2) |
| `storm` | thunder `distant_rumble` and `rolling` |
| `stream` | water `brook_rocks`, together with the creek recordings |

## Considered, not used

- "Regen in de tent" by luc de bruijn (Wikimedia Commons, CC BY 3.0, 8 min). It is mono and dark:
  about 8 dB under the other tent recordings at 8 kHz, which costs the canopy its elevation cue. Its
  second half has tonal bursts at 1–2 kHz.
- 717863 (TRP, rain on a fabric tent): the drops sound like static.
- 174874 (venaportae, Altay mountains): clips at 0 dBFS.
- 776275's louder struggle passages.
- Kranhytt 672691 (CC BY 4.0): used only as a reference for the pole creaks.
- Also used only as references, not shipped: 637774, 452296 and 530069 (drips), 517995 and 167684
  (wind), 196125 (thunder over a meadow), 572464 (a soft mountain stream).
