# Night library: sound sources

For the integrator to merge into `assets/sounds/SOURCES.md`. All files are in
`assets/sounds/generated/night_library/` (48 kHz mono 16-bit), built by
`tools/synth/night_library_sounds.py` from the fal takes and the originals below (the script's header
says how to run it). Processing for every file: high-pass (25 Hz at least; more where noted), events
trimmed with short fades, peak-normalised to -3 dBFS with at most 8x gain, loops cross-faded end into
start.

## Real recordings (shipped)

| Files | Original | Author | Licence | Attribution / notes |
|---|---|---|---|---|
| clock_tick_kb_1..12, clock_strike_kb_1..2 | https://commons.wikimedia.org/wiki/File:223959_klankbeeld_pendulum-slow-ten-o-clock.wav (Freesound 223959) | klankbeeld | CC BY 4.0 | "Pendulum clock: klankbeeld (Freesound 223959), CC BY 4.0, cut and processed." 44.1 kHz WAV, 269 s: a slow pendulum clock, out of beat (1.09 s / 1.50 s), striking ten. Ticks: twelve single beat cycles cut at the tick after the long gap, all 2.5886 s long (the mean cycle; 124252 samples), 30 Hz high-pass. Strikes: the last strike (from 145.05 s) and the last two (from 142.47 s) to 165.6 s with their hum; the escapement clicks under the hum are patched with the hum from 320 ms earlier (280 ms windows, 10 ms fades), so they do not double the tick files' beat; 3 s fade-out. |
| tallwindow_squall_pd_1 | https://commons.wikimedia.org/wiki/File:Rain_against_the_window.ogg | cori | Public domain | None required. 82 s of wind-driven rain against a window from inside; 40 Hz high-pass, 2 s loop cross-fade. Decoded from the original Ogg Vorbis. |
| page_turn_pd_1..6 | https://commons.wikimedia.org/wiki/File:Book_paper_pages_assorted.ogg | stephan | Public domain | None required. Six single page turns cut at 6.6, 10.7, 17.6, 22.8, 31.0, 39.3 s; 120 Hz high-pass. |
| pen_scratch_pd_1..2 | https://commons.wikimedia.org/wiki/File:Writing_with_inkpen.ogg | stephan | Public domain | None required. Two phrases of handwriting (3.9-10.6 s, 10.4-17.5 s); 300 Hz 4th-order high-pass (the original's handling rumble). |
| pen_scratch_cc0_1 | https://commons.wikimedia.org/wiki/File:292934-152_Writing-pen-paper-various_marks_and_lines.wav | soundsnap | CC0 | None required. 0-12.2 s of short pen marks and lines; 150 Hz high-pass. |

## Generated (fal, ElevenLabs SFX v2 `el2`), 2026-10-24

Prompts: `tools/fal/prompts_night_library.json` (file name = `<prompt name>_el2_<n>.wav`). 31 takes,
**$0.68** in `tools/fal/spend.log`.

| Files | Prompt name | Used as | Processing beyond the defaults |
|---|---|---|---|
| hearth_crackle_el2_1..2 | hearth_crackle | `fire` bed (loop) | 170 Hz 4th-order high-pass (a 140-160 Hz roar was the loudest octave), a slow random gain of ±3.5 dB (sines of 7.3, 12.9, 23.1 s) so the fire flares and settles, 1.5 s loop cross-fade |
| hearth_embers_el2_1..2 | hearth_embers | `embers` texture (loop) | 220 Hz 4th-order high-pass (a 140 Hz drone under the takes) |
| log_settle_el2_1..3 | log_settle | `fire` events | 60 Hz high-pass, 0.4 s fade-out |
| tallwindow_rain_el2_1..2 | tallwindow_rain | `rain` beds, one per window (loop) | 120 Hz high-pass |
| floor_creak_el2_1..3, shelf_creak_el2_1..2, radiator_tick_el2_1..3, door_far_el2_1..2 | same | `house` and `corridor` events | 40 Hz high-pass |

Generated and not used (the real recordings above matched the references better):
`clock_tick_el2_1..2`, `clock_strike_el2_1..2`, `page_turn_el2_1..4`, `pen_scratch_el2_1..3`, `room_tone_el2_1`.

## Checked against real recordings

Reference files downloaded to a scratch folder (not in the repo), compared with the takes after an 80 Hz
high-pass: octave-band shape relative to the loudest band, transients per second in the 2-8 kHz band
(+6 and +12 dB over a 300 ms median), the 10-90 % spread of 250 ms levels, and a narrow-peak (hum) test.

| Sound | Reference | What the numbers said | Decision |
|---|---|---|---|
| Fire crackle | Campfire sound ambience (Glaneur de sons, CC BY 3.0, Commons); Bones breaking wood fire ice crackling (stephan, PD) | Crackle rates match (takes 13.3-13.9/s at +6 dB, 7.8-8.0/s at +12 dB; campfire 13.9 / 7.5). The takes' 125 Hz octave was the loudest band (campfire -5 dB, stephan -17 dB) and the level spread 7-11 dB against 20-28 dB | 170 Hz high-pass (125 Hz now -8/-10 dB), slow gain swell (spread now 13-17 dB) |
| Embers | (no clean real reference found) | 7-8 dB narrow peak at 138-141 Hz, the 125 Hz octave 11 dB over the next band: a drone | 220 Hz high-pass; drone gone (peak test 3.7 dB, as the real files) |
| Clock | Grandfather Clock Ticking (Jud McCranie, CC BY-SA 4.0, reference only, not shipped); klankbeeld above | Real cases peak at 500 Hz with a wooden body down to 250 Hz; el2 tick 1 had nothing below 1 kHz (-32..-48 dB), tick 2 was bright and hissy | Real klankbeeld clock used |
| Clock strike | klankbeeld above | el2 strikes have the right shape (500 Hz peak) but are faded out at 8 s; the real gong rod hums for ~20 s | Real strike used |
| Pages | Book paper pages assorted (stephan, PD), Turning a page (planish, PD) | Real paper peaks at 8 kHz; the el2 pages peak at 2 kHz with 8 kHz 3-10 dB down: duller | Real pages used |
| Pen | Writing with inkpen (stephan, PD), pen marks (soundsnap, CC0) | Real writing comes in words with pauses (250 ms level spread 12-26 dB); el2 takes scratch without a break (3-5 dB) and carry extra 8-16 kHz hiss | Real pen used |
| Rain on glass | Rain against the window (cori, PD) | The real file is wind-driven rain with low buffeting (63-1000 Hz flat, 2 transients/s at +12 dB); the el2 takes are a close, steady patter (1 kHz peak, 16-18/s) | el2 kept for light rain on the panes, the real file used for the storm layer |
| Floor creaks | Creaking floors (Work With Sounds, CC BY 4.0, reference only) | The real file is a walk (steady 3 steps/s); the el2 creaks are single, dark (250-500 Hz, nothing above 4 kHz), like a creak heard across a room | el2 kept |
| Room tone | — | el2 take has a 13.6 dB tonal peak at 398 Hz | Dropped; the fire, rain and clock carry the bed |
