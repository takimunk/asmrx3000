# Night library (`night_library.json`)

Date: 2026-10-24. Status: built (scene, sounds, stage, panel, Russian). Sources and the checks against
real recordings: [night_library-sources.md](night_library-sources.md).

## Premise
An old reading room late at night. You sit in an armchair with a book on your lap. A wood fire burns
in the fireplace in the wall on your left, a long-case clock ticks against the far wall to the right
and strikes once (or twice) on the half hour, rain runs down the two tall windows behind you. Now and
then a page turns in your hands and a pen scratches on the writing table at your right hand; the old
house settles: a floorboard, a bookcase, the radiator under the window warming up, and very far away,
down the corridor beyond the doorway ahead, a door closes. No speech, no music.

## Spatial idea
Three distances, each with its own kind of sound:

| Layer | Distance | What | Character |
|---|---|---|---|
| Near field | 0.6-0.8 m, below the face | pages on the lap (ahead, 0.4 m down), the pen on the table (ahead-right) | crisp, close, slow; the only things "for you" |
| The room | 2-4.5 m, fixed places | fire left (2.9 m, low), clock ahead-right (4.5 m, at dial height), windows behind (2.2 m, both sides), radiator behind-right, floorboards, the bookcase ahead | stable anchors that stay put when the head turns |
| Outside and beyond | 3-50 m | rain on the street and a puddle under the window, the storm wind, thunder, all heard through the glass (`environment.medium` muffles every procedural model at 2.2 kHz while the clips stay crisp); the door 14 m down the corridor with a corridor echo | dull, far, behind the glass or the walls |

Turning the head brings the fire or the clock to the front; the windows are always behind.

## Groups (coordinates: +x ahead, +y left, +z up, head at the origin; floor -1.15)

| Group | Ids | Source | Position | Playback | level_db |
|---|---|---|---|---|---|
| fire | 10 | hearth_crackle (el2, 2 takes), 2 emitters 0.25 m | [1.0, 2.6, -0.8] | loop, bed | 54 |
| fire | 11 | log_settle (el2, 3) | [1.05, 2.56, -0.85] | events 70-180 s | 53 |
| embers | 12 | hearth_embers (el2, 2) | [1.0, 2.52, -0.95] | loop, texture | 44 |
| rain | 20, 23 | tallwindow_rain (el2, 2), one per window | [-1.75, ±1.2, 0.6] | loop, bed | 50 each |
| outside | 21, 22 | rain models `steady_rain` (4 emitters, 5 m) and `puddles` (gain 0.5) | [-6, 0, -0.6], [-2.6, 1.2, -1.2] | model, muffled | — |
| storm | 30, 34 | tallwindow_squall (real, PD), one per window | [-1.75, ±1.2, 0.9] | loop | 52 each |
| storm | 31, 32, 33 | wind `storm` (exposure 1.2, gain 0.13), wind `gap_howl` at the right sash (gain 0.18), rain `downpour` (gain 0.45) | outside / window frame | model, muffled | — |
| thunder | 40, 41 | thunder `distant_rumble` (70 s, gain 1.8), `rolling` (55 s, gain 0.8) | 47 and 52 m behind | model, muffled | — |
| pages | 50 | page_turn (real, PD, 6) | [0.42, 0.02, -0.42] | events 40-90 s | 45 |
| pen | 51 | pen_scratch (real, PD 2 + CC0 1) | [0.55, -0.5, -0.38] | events 50-130 s | 38 |
| clock | 60 | clock_tick (real, CC BY, 12 beat cycles) | [4.25, -1.4, 0.55] | events, interval = cycle 2.588583 s | 46 |
| chime | 61 | clock_strike (real, 1 and 2 strikes) | same | events, first 420 s exact, every 1800 s | 52 |
| house | 70-73 | floor_creak ×2 places, shelf_creak, radiator_tick (el2) | floor ahead-left, floor behind-right, bookcase ahead, radiator behind-right | events 90-480 s | 37-44 |
| corridor | 74 | door_far (el2), echo 0.11 s / 0.35 | [14, 1.8, -0.35] | events 300-900 s | 53 |

**The clock's beat.** The recording is out of beat (1.09 s, then 1.50 s). Each tick file is one
cycle, all exactly 124252 samples, played as events whose interval is that length, so a random cycle
follows the last one and the beat stays even (rendered 5 min through the engine: gaps 1.096 ± 0.019 s
and 1.492 ± 0.021 s, the original's spread). One long tick file was tried first and drifted 87 ms per
98 s: the engine counts the interval down in f32 (64-sample blocks). The strike files have the
escapement clicks removed from their hum so the strike never doubles the beat.

## Levers

| Key | Title (ru) | Range [default] | lively | Drives |
|---|---|---|---|---|
| `fire` | Fire (Огонь) | 0..1 [0.6] | 0.3 | fire.level [[0,-30],[0.15,-18],[0.4,-6],[0.6,0],[1,3]]; fire.events [[0,0.2],[0.3,0.6],[0.6,1],[1,1.6]]; embers.level [[0,-2],[0.3,0],[0.6,-5],[1,-12]] |
| `rain` | Rain (Дождь) | 0..1 [0.3] | 0.2 | rain.level [[0,-30],[0.1,-16],[0.3,-4],[0.6,0],[1,4]]; outside.level [[0,-30],[0.2,-16],[0.5,-5],[1,1]] |
| `storm` | Storm (Буря) | 0..1 [0] | 0.2 | storm.level [[0,-30],[0.25,-14],[0.6,-5],[1,0]]; thunder.level [[0,-40],[0.4,-30],[0.6,-12],[1,0]]; thunder.distance [[0,2],[1,1]] |
| `reading` | Reading (Чтение) | 0..1 [0.6] | 0.3 | pages.events [[0,0],[0.3,0.5],[0.6,1],[1,1.8]]; pen.events [[0,0],[0.3,0],[0.5,0.6],[1,1.6]] |
| `clock` | Clock (Часы) | 0..1 [0.5] | 0 | clock.level [[0,-30],[0.1,-18],[0.5,0],[1,5]]; chime.events [[0,0],[0.2,1],[1,1]]; chime.level [[0,-10],[0.5,0],[1,3]] |
| `house` | House (Дом) | 0..1 [0.5] | 0.2 | house.events [[0,0],[0.5,1],[1,2]]; corridor.events [[0,0],[0.5,1],[1,1.8]] |

Low fire brings the embers up as the crackle goes; the storm adds the lashing rain on the panes, the
wind, a whistle at the sash, a downpour and thunder on top of whatever the rain lever does.

## Direction (`climate: false`, `only: true`, `start: quiet_reading`, change 0.3, free run)

| Episode | wet / energy / drama | after / then | Segments: label, minutes, levers (others at default) |
|---|---|---|---|
| `quiet_reading` | -0.3 / -0.2 / 0 | — | reading by the fire 12-25: rain 0.25, reading 0.65 |
| `rain_sets_in` | 0.5 / 0.1 / 0.3 | after quiet_reading, late_stillness, fire_burns_low | rain setting in 5: rain 0.45 · steady rain 8-16: rain 0.7, reading 0.5 |
| `fire_burns_low` | 0 / -0.5 / 0.2 | after quiet_reading, rain_sets_in, late_stillness | the fire burns low 8: fire 0.3 · glowing embers 6-12: fire 0.1, reading 0.35, clock 0.6 · a fresh log 4 (ease 25): fire 0.75 |
| `storm_outside` | 1 / 0.8 / 1 | after rain_sets_in, then late_stillness | the wind rises 4: rain 0.75, storm 0.35 · storm over the roofs 6-10: rain 1, storm 0.9, fire 0.7, reading 0.2 · storm moving away 6: rain 0.6, storm 0.3 |
| `late_stillness` | -0.5 / -0.8 / 0.1 | after all but itself | late-night stillness 12-25: rain 0.12, fire 0.35, reading 0.15, house 0.3, clock 0.65 |

Planned 8 h at neutral steer (`--example direction`): late stillness 28 %, rain sets in 27 %, fire
burning low 21 %, quiet reading 21 %, the storm 3 % (one in 8 h; the weather steer brings it more often).

## Calibration (leveller off, other groups at -30 dB, default levers, 120 s)

| Group | RMS dBFS | max 1 s | Relative to the fire |
|---|---|---|---|
| fire (bed) | -35.4 | -29.5 | 0 |
| rain on the windows (rain 0.3) | -39.7 | -37.6 | -4 |
| clock ticks | -47.2 | -45.9 | -12 (crest ~25 dB: each tick clear) |
| outside (rain 0.3) | -48.5 | -47.9 | -13 |
| embers (fire 0.6) | -54.6 | -48.1 | -19; at fire 0.1 they are the bed |
| pages / pen (events) | — | -37.2 / -38.2 | 2-3 dB under the bed's 1 s level: the focus, like the tea house's host |
| house creaks (events) | — | -45.5 | 10 dB under |
| storm at 1: squall, wind, sash, downpour | -33/-33/-39/-36 per source | | together ~6 dB over the quiet room |
| thunder at storm 1 | | -28 | under the storm bed |
| clock strike | | ~-32 | 2 dB over the bed for a moment, twice an hour |

Every lever moves its own groups by 20-30 dB between its ends (`leversolo` check below). Defaults give
about 2.5 noticeable events a minute, most of them the pages and the pen; the house adds one every
1-2 minutes.

## Stage (`ClipSet.library`)
The room in the set's pencil lines, cut open on the camera's side (right wall left out, the tall
windows behind cut like a plan section above the sill): floorboards, a rug, the armchair, the writing
table, the fireplace with its chimney breast, mantel, opening and hearth stone in the left wall, the
front wall with the doorway into a long corridor and the door at its end. Props, built in metres at
the room's scale: andirons and logs, a bed of embers, a slumping log, the open book whose leaf lifts
with each page sound, paper, inkwell and pen, the long-case clock with its swinging pendulum (the
strike shares it), a bookcase, a column radiator, floorboards, a door ajar. Rain on the panes uses the
lighthouse's glass prop; the outside models get the usual rain, wind and bolt props. Camera from
behind the right shoulder.

## Checks run
- `cargo test`, `cargo clippy --all-targets`, `clip_groups` on the scene, a 25 min render with head
  turns (no clipping: peak -2.9 dBFS at volume 0.8; no silence: 10 s levels -33..-38 dBFS through the
  direction's episodes), a 5 min clock-only render for the beat, `direction` for 8 h.
- `scripts/check-macos.sh`, `scripts/check-l10n.py`, `swift build -c release`, a `--snapshot` of the
  stage.
- Not checked: listening with AirPods head tracking (the turn render only checks that it runs).
