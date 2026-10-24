# Campfire by the sea: sound sources (to merge into assets/sounds/SOURCES.md)

Files in `assets/sounds/generated/campfire_shore/` (48 kHz mono 16-bit WAV, not in git). Post-processing as
SOURCES.md: high-pass (2nd order; run cyclically on loops so the seam stays clean), trimming of leading and
trailing silence and a short fade (one-shots), peak-normalised to −3 dBFS with at most 8× gain. Extra steps
are listed per file.

## Real recordings

| File | Original | Author | Licence | Attribution / changes |
|---|---|---|---|---|
| campfire_crackle_ccby.wav | https://commons.wikimedia.org/wiki/File:Campfire_sound_ambience.ogg (Freesound https://freesound.org/people/Glaneur%20de%20sons/sounds/29727/) | Glaneur de sons | CC BY 3.0 (https://creativecommons.org/licenses/by/3.0) | "Campfire sound ambience" by Glaneur de sons, CC BY 3.0. Changes: 9.5–47 s cut, 3 s cross-faded loop, 25 Hz high-pass, the biggest pops soft-limited (tanh, 10 dB under the peak), normalised. |
| campfire_bonfire_ccby.wav | https://commons.wikimedia.org/wiki/File:WWS_Bonfireburning.ogg (http://www.workwithsounds.eu/sound/bonfire-burning/) | Work With Sounds / Werstas, recorded by Kimmo Kestinen | CC BY 4.0 (https://creativecommons.org/licenses/by/4.0) | "WWS Bonfireburning" by Work With Sounds / Werstas (Kimmo Kestinen), CC BY 4.0. Changes: stereo summed to mono, 1–51 s cut, 4 s cross-faded loop, 60 Hz high-pass, normalised. |

Rejected: `Ocean_Waves_on_a_Tropical_Beach.ogg` (Commons, marked CC0 "own work") was used for the surf bed at
first and then dropped: its embedded metadata names a label's YouTube "Topic" channel as the author, so the
CC0 claim cannot be trusted. `On_a_pebble_beach.ogg` (Commons, public domain, earthcalling) is a 128 kbps mp3
with wind rumble 15–25 dB over the sea below 150 Hz: kept as a reference only.

## Generated (fal.ai, prompts in `tools/fal/prompts_campfire_shore.json`)

| Files | Model | Used in | Extra processing |
|---|---|---|---|
| campfire_crackle_el2_2 | el2 (loop) | `fire` | 40 Hz HP, pops soft-limited 6 dB |
| campfire_flare_el2_1..2 | el2 (loop) | `flames` | 90 Hz HP (a sub-bass boom 8 dB over the bonfire reference) |
| campfire_log_shift_el2_1..3 | el2 | `wood` | 40 Hz HP (take 2 had 26 % of its energy below 25 Hz) |
| campfire_damp_hiss_el2_1..3 | el2 | `wood` | 100 Hz HP, 0.5 s fade-out |
| shore_breakers_far_el2_1..2 | el2 (loop) | `sea` bed | 40 Hz HP |
| shore_wave_el2_1..4 | el2 | `sea` waves | 40 Hz HP, 0.8 s fade-out |
| shore_curlew_el2_2..3 | el2 | `night` | 300 Hz HP, pitched up ×1.6 (see below) |
| shore_bell_buoy_el2_1..2 | el2 | `night` | 60 Hz HP, 1 s fade-out |

Rejected takes: campfire_crackle_el2_1 (hollow: 250 Hz–2 kHz 14–20 dB under the reference, a 13.5 dB tone at
398 Hz, centroid 8.3 kHz), shore_curlew_el2_1 (no clear call contour), shore_breakers_far_sa25_1 (Stable Audio
2.5, 90 s: half its energy below 25 Hz, 63 dB level range, sputtering 4–16 Hz modulation 14 dB over the
references).

Spend: $0.51 (19 el2 takes) + $0.29 (the far-surf re-roll: 1 sa25 + 2 el2) = **$0.80** of the $3 budget
(`tools/fal/spend.log`).

## Checked against field recordings
Features from `ml/asmr_ml/refcompare.py` (gain-robust: 1/3-octave levels re total, centroid, envelope
modulation, onsets). References were downloaded to a scratch folder, not into the repo: the two real fire
recordings above, `On_a_pebble_beach.ogg` (PD, high-passed at 150 Hz to remove its wind rumble),
`NausetBeach.ogg` (CC BY-SA 4.0, surf after sunset, reference only), two Eurasian curlew recordings
(`XN_Numenius_arquata.ogg`, CC BY-SA 3.0; `Eurasian_Curlew_(Numenius_arquata)_(W1CDR0001389_BD26).ogg`, CC BY-SA
4.0; reference only).

| Sound | Reference | Result |
|---|---|---|
| Generated crackle (take 2) | Glaneur campfire 10–47 s | centroid 274 vs 477 Hz, onsets 9.7 vs 8.8 /s, inter-onset CV 0.54 vs 0.57, crest 15 vs 19 dB, bands within 5–7 dB (a little more below 100 Hz). Kept. |
| Generated flare | WWS bonfire | dense crackle 9–10 onsets/s vs 2.3, 500 Hz–1 kHz 15–19 dB thinner, 63–126 Hz 8 dB stronger: high-passed at 90 Hz and paired with the real bonfire for the body. |
| Procedural `fire/campfire` (and a damp variant) | both fire recordings | crest 5–7 dB vs 19–22, onsets 1.0–1.4 /s vs 6.8–8.8, centroid 1.5–2.0 kHz vs 0.5 kHz: a steady mid-band roar, not a close campfire. **Not used.** |
| Procedural `fire/embers` | Glaneur campfire | crest 23 vs 19 dB, onsets 3 /s, 4 kHz band 9 dB brighter (glassy coal ticks). Used only as the quiet coal bed (−11 dB re the surf). |
| Procedural `water/sea_surf`, `sea_surf` + shingle 0.8, `rock_surf` | pebble beach, Nauset | 8 kHz band −13 dB re total vs −26…−31 dB in the recordings (much too bright), no shingle rattle onsets (0 vs 2 /s). **Not used.** |
| Generated far surf (take 1) | Nauset beach | centroid 1.42 vs 1.86 kHz (darker, as at 30 m), bands within 3–5 dB from 126 Hz to 4 kHz, slow (0.1–1 Hz) swell modulation present, 1–4 Hz modulation −20 vs −24 dB. Take 2 darker still (centroid 0.83 kHz), used as its alternate. |
| Generated waves (4 takes) | pebble beach (HP 150 Hz) | centroid 1.73–1.83 vs 1.70 kHz, bands within 2–4 dB from 250 Hz to 8 kHz; two surges ~5 s apart per take vs a 6–7 s rhythm in the recording; fewer distinct pebble clicks (0.1–0.3 vs 2 onsets/s: the reference is a close microphone in the shingle). Kept. |
| Generated curlew | two curlew recordings | call fundamental 0.7–1.45 kHz vs 1.5–2.4 kHz in both recordings (an octave-ish low): resampled ×1.6, now 1.1–2.3 kHz. |
| Damp-wood hiss | Glaneur campfire high band | centroids 5.6–9.4 kHz, in the 6–10 kHz region where the close campfire has its strongest band (8 kHz −8 dB re total); one take whistles at 2.8 kHz (steam). No dedicated recording found. |
| Log shift | — | no free recording of a settling log found; checked only for level, sub-bass and tonal artefacts. |
| Bell buoy | — | no free recording found (Commons has none); a tonal bell at 504 Hz, irregular clangs. Unverified against a recording. |
| Wind `breeze`, foliage `dry_grass` | `wind_open` set | the models' earlier calibration against open-land wind recordings (docs/references.md, docs/physical-models.md). |
