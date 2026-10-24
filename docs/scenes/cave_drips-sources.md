# Cave (`cave_drips.json`): sound sources

For the integrator to merge into `assets/sounds/SOURCES.md` (section "Cave (assets/sounds/generated/cave_drips/)").

All 37 files are **real field recordings from Freesound, all CC0 1.0** (no attribution required; courtesy
credits below). Nothing was generated: **fal spend $0**. The draught is the procedural wind model (no file).

Taken from the Freesound HQ previews (ogg, ~190 kbps; the originals need an API key), decoded, mixed to mono,
resampled to 48 kHz and cut by `tools/synth/cave_drips_sounds.py` (the cut times are in the script):

* one-shots: cut 12 ms before the onset, high-passed (2nd order Butterworth, zero phase: 60 Hz for drips,
  120 Hz for drops on rock, 80 Hz for pebbles), 2 ms fade-in, cos² fade-out over the last 35 % (50 % for the
  dry rock drops), peak-normalised to -3 dBFS, 16-bit. Plinks and the slow drip keep 1.4 s of their own
  cave tail, the deep drips 1.6 s; the dry rock drops are 0.4 s (the scene's `echo` gives them the cave).
* loops (stream, runnel): high-passed at 40 Hz (runnel 60 Hz), the last 2 s (2.5 s) cross-faded with equal
  power into the start, so the file loops without a seam (measured seam jump 0.02-0.14 σ), -3 dBFS.

| Files | Freesound id, title | Author | Licence | Cut from (s) |
|---|---|---|---|---|
| cave_plink_1..5, cave_slowdrip_1..3 | [843936](https://freesound.org/people/TSP-Talk/sounds/843936/) "Teufelsfelsen Cave Dripping Water – Distant Aircraft Overflight" | TSP-Talk | CC0 1.0 | plinks 293.01, 185.12, 420.48, 446.02, 382.28; slow drip 273.94, 356.74, 216.84 (one pool, ~860 Hz) |
| cave_plink_6..7, cave_rockdrip_9..11 | [402929](https://freesound.org/people/Selector/sounds/402929/) "Drops in the small cave" | Selector | CC0 1.0 | plinks 74.29, 129.96; rock 108.02, 137.94, 165.41 |
| cave_plink_8..10 | [177958](https://freesound.org/people/Sclolex/sounds/177958/) "Water Dripping in Cave.wav" | Sclolex | CC0 1.0 | 5.84, 68.95, 76.59 |
| cave_deepdrip_1..3 | [870748](https://freesound.org/people/CHallSmith/sounds/870748/) "Drips, Giant Cave Drips, Slow" | CHallSmith | CC0 1.0 | 7.80, 45.08, 78.98 |
| cave_rockdrip_1..8 | [473802](https://freesound.org/people/MPierluissi/sounds/473802/) "ROCKMisc_RAIN DROP ON ROCK_MP.wav" | MPierluissi | CC0 1.0 | 1.00, 2.00, 3.00, 4.01, 8.02, 9.02, 18.04, 26.05 |
| cave_pebbles_1..5 | [179341](https://freesound.org/people/lolamadeus/sounds/179341/) "Gravel Impacts and Falls.wav" | lolamadeus | CC0 1.0 | 0.43, 2.34, 5.53, 7.30, 11.35 |
| cave_pebbles_6 | [521956](https://freesound.org/people/Kastenfrosch/sounds/521956/) "Stone Crumble 2" | Kastenfrosch | CC0 1.0 | 0-3.2 |
| cave_pebbles_7 | [133655](https://freesound.org/people/samuelburt/sounds/133655/) "scree.flop.03.aif" | samuelburt | CC0 1.0 | 0-2.6 |
| cave_stream_1, cave_stream_2 | [414167](https://freesound.org/people/felix.blume/sounds/414167/) "Stream running underland." | felix.blume | CC0 1.0 | 1-21, 24-46 (a bump at ~22 s and the louder end left out) |
| cave_runnel_1 | [462661](https://freesound.org/people/Duasun/sounds/462661/) "Small cave river flow" | Duasun | CC0 1.0 | 0.5-18 |

Notes:
* 843936 has a distant aircraft in places and a -52 dBFS noise floor: the cuts are 35-40 dB over it, so each
  plink carries a little hiss in its tail. 870748 is a sound designer's take (near-pure 690-700 Hz tones,
  79 dB over the spectral median, centroid ~180 Hz, probably slowed down); it is used only for the three far,
  deep drips.
* Recordings checked as references only (not cut): [818888](https://freesound.org/people/mentos987/sounds/818888/)
  mentos987 "cave dripping water", [337618](https://freesound.org/people/borralbi/sounds/337618/) borralbi
  "Dripping ceiling in an old limekiln", [627789](https://freesound.org/people/poots/sounds/627789/) poots
  "underground mine dripping", [260588](https://freesound.org/people/felix.blume/sounds/260588/) felix.blume
  "Drops falling into some buckets inside an old house", all CC0. Downloaded to a scratch folder, not the repo.

## Checked against the field recordings

The rule: generated or assembled sounds are matched to real recordings of the same thing. Here the drips
are themselves cave recordings; what had to be checked is the **scene** (the engine's `echo`, the event
rates and the levels), so a 10-minute render at the default levers was measured with the same scripts as
six real cave-drip recordings (onsets 9 dB over a running floor; Schroeder decay of isolated drips, T60 from
the -5..-25 dB slope; early/late = energy in the first 50 ms vs the rest of 1.8 s; roughness = ripple of
the 10 ms envelope about its straight decay, which is high for discrete flutter echoes):

| | onsets /min | drip peak over floor | T60 of the tails | early/late | roughness |
|---|---|---|---|---|---|
| 843936 Teufelsfelsen cave | 60 | +14 dB | 4.2 s | -6.1 dB | 1.9 dB |
| 402929 small cave | 52 | +18 dB | 5.5 s | -1.7 dB | 1.3 dB |
| 177958 cave | 27 | +26 dB | 6.3 s | +3.5 dB | 2.0 dB |
| 337618 limekiln | 6.5 | +13 dB | 4.1 s | -5.6 dB | - |
| 627789 mine | 3.6 | +12 dB | 3.4 s | -5.8 dB | 1.1 dB |
| 870748 "giant drips" (designed) | 71 | +36 dB | 1.6 s | +3.8 dB | 3.2 dB |
| **render, default levers (600 s)** | **52** | **+14.5 dB** | **4.35 s** (p25 3.25, p75 5.6) | **-6.6 dB** | **1.7 dB** |

The plinks' pitch (530-1700 Hz, 40 dB tonal prominence in the first 40 ms) is the pools' own; it sits where a
drop's entrained bubble rings (Minnaert, f₀ = 3.26/r: bubbles of 2-6 mm radius), as in the core's water model.
