# Reference recordings and model comparison

Ground truth for tuning the procedural sound models: real field recordings per sound category
and a tool that compares our renders with them using numbers. Recordings are in `ml/data/refs/<category>/`
(gitignored); only this attribution list and the code are in the repository.

## Reproduce

```sh
cd ml && ~/.local/bin/uv sync
uv run python -m asmr_ml.data.refs_fetch [category ...] --max 8   # Freesound (CC0 / CC BY only), key in ml/.env
uv run python -m asmr_ml.data.refs_xc                              # xeno-canto: frogs, woodpecker drumming
uv run python -m asmr_ml.data.refs_commons                         # Wikimedia Commons: Rana temporaria
./render_ours.sh          # renders the models mono, unspatialised -> ml/out/ours/<category>/*.wav
./compare_all.sh          # all comparisons, text output
uv run python -m asmr_ml.refcompare refs thunder_close                         # reference stats (median, IQR)
uv run python -m asmr_ml.refcompare compare brook out/ours/brook/brook_pebbles.wav  # ours vs references
uv run python -m asmr_ml.refcompare report > /tmp/tables.md                    # markdown sources + tables (this file's tables)
```

Results are also written as JSON to `ml/out/refs/<category>.json`.

One model, by hand:

```sh
cargo run --release --manifest-path core/Cargo.toml --example model_wav -- \
    <model> <preset|-> <seconds> <out.wav> [key=value ...]
# water brook_pebbles 60 /tmp/b.wav
# thunder close_strike 120 /tmp/t.wav distance=1500 interval=20
# wind forest_roar 90 /tmp/w.wav wind_ms=5 gust_amp=0.5 gust_period=6
# insects marsh_frogs 90 /tmp/f.wav hour=23 temperature=18
# wings:wood_pigeon - 60 /tmp/p.wav flight=7 pause=3      (flight_motion wingbeat pattern of the species)
```

Keys: environment (`wind_ms rain_mm_h temperature hour humidity leaf_wetness ground_wetness dryness leaf_cover storm`),
controls (`gust water activity flap gust_amp gust_period`), `seed`, thunder `distance`/`interval`, and any
other `key=value` is a `SoundDesc` model parameter (`flow_ms=1.2`). Output is 48 kHz mono 32-bit float.
Everything needed was public in `asmr_core`; nothing in `core/src` was touched.

## Features (all robust to microphone gain)

| feature | meaning |
|---|---|
| `band_<Hz>` | 1/3-octave level, dB re total power |
| `centroid_med/p10/p90`, `flatness_db` | spectral centroid distribution (Hz), spectral flatness (0 dB = white noise) |
| `am_0.1-1`, `am_1-4`, `am_4-16`, `am_16-64` | envelope modulation power (dB re mean level squared) in that rate band: slow swells vs babbling vs flutter/rattle |
| `ampeak_hz` | strongest envelope modulation rate 2-100 Hz (drumming rate, call pulse rate) |
| `crest_db`, `spread_db` | envelope p99 minus mean, p95 minus p10 of 50 ms RMS |
| `onset_rate`, `ioi_cv`, `event_ms` | onsets per s, regularity of intervals (0 = metronome), median event length |
| `tonal_db`, `dom_hz` | strongest narrow peak of the long-term spectrum over its 1/3-octave median; frequency of the strongest peak |
| `attack_s`, `decay20_s` | thunder only: rise from -20 dB to the peak, fall to -20 dB |
| `distance` | mean over groups (spectrum, modulation, dynamics, events, tonality, thunder) of the clipped robust z-score (about 1 = typical spread of the references; above 2 = clearly off) |

## How to read the numbers (caveats)

* Our renders are the isolated model with no room tone, so between events they are near digital silence.
  Recordings always carry a noise bed. This inflates `crest_db`, `spread_db` and the AM bands for sparse models
  (lapping, slow river, still lake, tree frogs, woodpecker). The envelope is floored 40 dB under its peak, but
  treat these as qualitative ("our model is events with gaps, the real thing is continuous").
* Most references have wind or handling rumble below about 150 Hz (`dom_hz` ~100 Hz for wings and wind). Trust bands
  above about 250 Hz and the centroid p50/p90 more than the lowest bands.
* References are Freesound HQ previews (ogg), selected by name/tags only (not auditioned). A few categories are small
  (`slow_river` 4, `still_lake_ambience` 3, `frogs_marsh` 3, `rain_forest` 3); `still_lake_ambience` references are
  wind/boat rumble dominated and are the least trustworthy. `frogs_common` has only 2 European Rana recordings
  from Freesound plus 3 from Wikimedia Commons (Rana temporaria).
* `gust_amp` in the wind renders is a synthetic gust process (0.3-0.9) chosen by hand, so the 0.1-1 Hz AM of wind is
  partly my setting, not the model.
* The `gull` and `raven` renders are identical (same wing style and seed in the engine).

## Biggest mismatches per model (current state)

Distance per model: brook_pebbles 0.80, rain on_leaves 0.67, forest_roar (wind_trees) 0.94 and gusty 0.38,
thunder 1.2-1.3, frogs: common 1.0, marsh 1.4, tree 2.2; worst: foliage oak 2.95 / birch 2.56 (vs wind in trees),
still_lake 2.5, woodpecker 2.35, lake_lapping 2.0, slow_river 2.1.

**Water**
* `brook_pebbles` is close overall. Too much very slow modulation (0.1-1 Hz: -12 dB vs -23 dB, a swell that
  real brooks do not have), a little too little 6 kHz, level spread p95-p10 10 dB vs 5 dB, events (bubbles) 90 ms vs
  170 ms and 5.4 vs 3.3 onsets/s: slightly too many, too short bubbles.
* `brook_rocks`: far too steady: 4-16 Hz modulation 12 dB and 16-64 Hz 9 dB below the references, no onsets at all
  (0 vs 3.3/s), `ampeak` 2.2 Hz vs 4.4 Hz; 250-500 Hz 5-7 dB too strong, 800-1000 Hz 3 dB too weak. "Rushing hiss"
  rather than babbling.
* `small_waterfall`: no low end (50-250 Hz 10-15 dB below), 2.5-6 kHz +4-5 dB, modulation at 1-64 Hz 6-10 dB too weak
  (steady hiss), modulation peak at 66 Hz vs 12 Hz in recordings.
* `slow_river`: essentially silent. Median 50 ms level -102 dBFS with a rare glug (-57 dBFS), against references
  that are continuous hiss. 3-8 kHz is 20-25 dB below the references (`band_4032` -35 vs -16 dB), centroid 960 Hz vs
  1600 Hz, AM 20 dB too strong (bursts instead of a steady flow). "A smooth river is mostly silent" is a design
  decision that real rivers contradict: add a broadband flow bed at 1-8 kHz.
* `lake_lapping` (wind_ms=3): too bright and too clicky. Centroid 2.9 kHz vs 1.1 kHz, 250-800 Hz 7-12 dB too low,
  8 kHz 14 dB too low and 1.2-1.6 kHz 5-7 dB too high, events 20 ms vs 110 ms (slaps are far shorter than real
  lapping), 1-64 Hz modulation 13-14 dB too strong. Real lapping is a low "sloshing" (dominant 300-600 Hz) with long
  soft events.
* `still_lake`: bursts with silence, not a steady ambience; centroid 2.2 kHz vs references (wind rumble, low
  confidence), 1-4 kHz 10 dB too strong, AM 16-23 dB too strong.

**Wind and foliage**
* `forest_roar` is the best match (0.94 vs wind in trees, 0.38 vs gusty wind): centroid 610 vs 490 Hz, a bit too
  much 2-4 kHz (+3 dB). Envelope rate of change: `ampeak` 2 Hz vs 12-25 Hz, so real tree wind has faster flutter; 1-16 Hz AM
  2-4 dB too deep.
* `breeze` (wind_trees): centroid 980 vs 490 Hz, 2-8 kHz 10-15 dB too strong. Not like wind in trees; it is
  closer to open-land wind (wind_open 1.0), but there too 500-1000 Hz is 8-15 dB too strong.
* `foliage` oak/birch (wind in trees): wrong spectrum. Centroid 3.6-4.5 kHz vs 0.5 kHz, 125-250 Hz 25-35 dB too
  weak, 2-8 kHz 15-35 dB too strong, a tonal peak at 540 Hz (10-12 dB above its neighbours vs 1.7 dB in recordings:
  audible "whistling pitch"), flatness -5 dB vs -23 dB (too hissy), 0.1-1 Hz AM 12-15 dB too deep. Foliage should
  carry rustle on top of a wind bed, not stand in for the wind.
* `foliage grass` / `dune_sand` as wind over open land: `grass` centroid 7.2 kHz vs 1.5 kHz and nothing below 500 Hz
  (-30 dB); `dune_sand` dominant peak 2.75 kHz.

**Rain**
* `on_leaves` 0.67 and `under_canopy` 0.85: good. `under_canopy` is too bright (centroid 3.6 kHz vs 1.8 kHz, 2.5-6 kHz
  +6-8 dB) and has `ampeak` 44 Hz vs 4 Hz; `on_leaves` lacks 1-1.3 kHz (-7 to -10 dB). Only 3 references.

**Thunder** (after the physical rework of the crack and the rolls, `docs/physical-models.md` thunder section; 3 seeds x 150 s)
* Close: no longer boomy. 0.7 km matches the close references in spectrum (63 Hz -19.8 vs -17.4, 400 Hz -11.2 vs -10.7,
  1 kHz -16.9 vs -16.0, 2.5 kHz -25.1 vs -27.9; dominant 223 vs 184 Hz; centroid 245 vs 294 Hz), distance 1.24 -> 1.13.
  At 1.5 km 1-2.5 kHz stay 5-6 dB under the references: ISO 9613-1 absorption over 1.5 km (the recordings are probably
  closer than ~1 km). Previously 50-100 Hz +8 dB, above 400 Hz up to -10 dB (energy-weighted N-waves twice too long).
* Distant (8 km): rolls 10-20 s (was ~5 s), decay to -20 dB 5.8 s (was 2.7; references 8.0 median, 3-10 s in the clean
  ones), attack 2.4 s (1.5; references 7.2 inflated by multi-flash files and the rain floor). Above ~500 Hz darker than
  the references by ISO 9613-1 physics; the distant references look like 3-5 km (3 km render matches 300-600 Hz) and have
  rain above 2 kHz. Distance 1.23 -> 1.15.
* The interval-regularity feature (`ioi_cv` 5-7 vs 0.8) is meaningless for thunder (our isolated strikes against
  rumble in recordings).

**Wings** (after the thump/low-body rewrite of `sources/wings.rs`)
* Was: every style far too bright (centroid 2.4-3.8 kHz vs 0.5-0.6 kHz, 2-8 kHz +10-20 dB, flatness -5 dB: hiss
  instead of the air thump). Now each beat is a low "thump" (pressure pulse of the downstroke, 100-190 Hz), a broad
  low-passed body (0.75-1.45 kHz corner) and a weak feather rustle (3-5 x the body corner, 20-30 % share). Clean
  recordings (e.g. fs543110, fs479605, fs717900) have their energy at 250 Hz-2 kHz and fall 5-8 dB per octave above.
* Distance (pause=3 harness): wood_pigeon 1.45 -> 1.01, chaffinch 1.58 -> 0.81, robin 1.62 -> 0.74, swallow 1.35 -> 0.72,
  gull/raven 1.51 -> 1.35. Centroid 2418/3251/3193/3824/1526 Hz -> 524/838/792/919/383 Hz (refs 465-622 Hz); flatness
  -6.6/-5.3/-5.4/-4.7/-9.0 dB -> -17.7/-14.7/-15.0/-14.0/-21.7 dB (refs -14.7 to -23); `event_ms` small birds 20 -> 40-50
  (the upstroke whisper now carries to the next beat), pigeon 60 -> 50.
* Remaining: `ioi_cv` 2.1-2.6 vs 0.84 comes from the harness (`pause=3` silences) and the flight patterns in
  `director/species.rs`, not from the beat model (with `pause=0.5`: chaffinch 1.0, robin 0.8, swallow 1.2, pigeon 1.6,
  gull 1.7). Gull/raven onsets 1.1/s vs 5.3/s (references are flocks / cormorant nests with many birds); the 126 Hz
  band is still high against most references (their <150 Hz is mic rumble, the cleanest ones are as high as ours).

**Frogs and woodpecker** (hour 23 / 8, 18 degrees C)
* `common_frogs` 1.0: onsets 18/s vs 4.7/s, pulses 10 ms vs 60 ms (clicks, not purrs), flatness -28 dB vs -18 dB
  (too tonal), band_1008-1600 +5 to 7 dB, 100-250 Hz +6 to 9 dB.
* `marsh_frogs` 1.4: onsets 10/s vs 4/s, `ioi_cv` 2.4 vs 1.0 (irregular), pulse 30 ms vs 50 ms; 2 kHz band 10 dB too
  weak (the loud fundamental of the real call is at 1.7-2 kHz with harmonics to 4 kHz; ours centroid p90 1.5 kHz vs
  2.5 kHz); 500-1300 Hz 6-8 dB too strong.
* `tree_frogs` 2.2: worst of the frogs. Call pulses 20 ms vs 80 ms and `ioi_cv` 3.5 vs 0.7, mid bands (250-800 Hz)
  15-25 dB too strong where real tree frogs are narrow at 2-3 kHz (dominant 2.5 kHz); chorus is bursty (0.1-1 Hz AM
  +16 dB; crest 27 dB vs 7.7 dB) while real recordings are a continuous chorus; flatness -13.5 dB vs -28 dB (too noisy:
  recordings are tonal).
* `woodpecker` 2.35: muffled. Centroid 830 Hz vs 1750 Hz, 2-8 kHz 6-15 dB too low (`band_4032` -29 vs -14 dB), dominant
  810 Hz vs 1090 Hz; roll rate 0.4/s vs 2.2/s onsets in recordings (our render has few, long rolls; not verified how the
  onset detector counts a roll), 16-64 Hz rattle modulation 10 dB deeper than recordings (strikes are too
  uniform and sharp), tonal flatness -21.5 dB vs -13.9 dB.

## Sources and licences

### brook

- Woodland Stream - naturenotesuk, CC BY 4.0, <https://freesound.org/people/naturenotesuk/sounds/733384/>
- A small stream. Water noise. - tosha73, CC BY 4.0, <https://freesound.org/people/tosha73/sounds/849564/>
- Mountain River 001.wav - Auxide_Audio, CC0 1.0, <https://freesound.org/people/Auxide_Audio/sounds/588179/>
- Creek Ambience - SduggySounds, CC0 1.0, <https://freesound.org/people/SduggySounds/sounds/721616/>
- Stream 1 - farandjoun, CC0 1.0, <https://freesound.org/people/farandjoun/sounds/826884/>
- brook fast-flowing.wav - Fenodyrie, CC0 1.0, <https://freesound.org/people/Fenodyrie/sounds/615244/>
- Stream Nov.wav - juskiddink, CC BY 4.0, <https://freesound.org/people/juskiddink/sounds/136403/>
- Kervijen babbling creek travelling.wav - bruno.auzet, CC0 1.0, <https://freesound.org/people/bruno.auzet/sounds/690326/>
- mountain stream high bitrate - Eelke, CC BY 4.0, <https://freesound.org/people/Eelke/sounds/462618/>

### frogs_common

- moor frogs Zandbergsvennen Kampina Netherlands 1208 pm 260302_0080 - klankbeeld, CC BY 4.0, <https://freesound.org/people/klankbeeld/sounds/847039/>
- Frog Croaking (UK Common Frog) - shaunhillyard, CC0 1.0, <https://freesound.org/people/shaunhillyard/sounds/532235/>
- Grasfrosch Paarungsrufe.OGG - Rabe19 (Diskussion), Public domain, <https://commons.wikimedia.org/wiki/File:Grasfrosch_Paarungsrufe.OGG>
- Rana temporaria 120329-015849.ogg - Gilles San Martin, CC BY-SA 3.0, <https://commons.wikimedia.org/wiki/File:Rana_temporaria_120329-015849.ogg>
- Rana temporaria croaking at Muhalnitsa, Botevgrad, Bulgaria.ogg - Dimitǎr Boevski, CC BY-SA 4.0, <https://commons.wikimedia.org/wiki/File:Rana_temporaria_croaking_at_Muhalnitsa,_Botevgrad,_Bulgaria.ogg>

### frogs_marsh

- Marsh Frog (Pelophylax ridibundus) territorial call - Tanguy Loïs, CC BY-SA 4.0, <https://xeno-canto.org/893221>
- Marsh Frog (Pelophylax ridibundus) territorial call - Tanguy Loïs, CC BY-SA 4.0, <https://xeno-canto.org/893215>
- Marsh Frog (Pelophylax ridibundus)  - Gobi, CC0 1.0, <https://xeno-canto.org/1005135>

### frogs_tree

- Common Tree Frog (Hyla arborea) territorial call - Noé Ferrari, CC BY 4.0, <https://xeno-canto.org/1095052>
- Common Tree Frog (Hyla arborea) territorial call - Noé Ferrari, CC BY 4.0, <https://xeno-canto.org/998076>
- Common Tree Frog (Hyla arborea) territorial call - Noé Ferrari, CC BY 4.0, <https://xeno-canto.org/998075>
- Common Tree Frog (Hyla arborea) territorial call - Tanguy Loïs, CC BY-SA 4.0, <https://xeno-canto.org/893216>
- Common Tree Frog (Hyla arborea)  - Marius Faiß, CC BY 4.0, <https://xeno-canto.org/900375>

### lake_lapping

- Lake Waves 2 converted to ORTF - Vogyik, CC BY 4.0, <https://freesound.org/people/Vogyik/sounds/872504/>
- Lake Waves 2.wav - Benboncan, CC BY 4.0, <https://freesound.org/people/Benboncan/sounds/67884/>
- Waves on Stones 20ft - Lake Ontario.wav - bertjerred, CC BY 4.0, <https://freesound.org/people/bertjerred/sounds/629946/>
- Lake with small waves on island with flat stone.wav - BonnyOrbit, CC0 1.0, <https://freesound.org/people/BonnyOrbit/sounds/442486/>
- LAKE - 1 - SamuelGremaud, CC0 1.0, <https://freesound.org/people/SamuelGremaud/sounds/460499/>
- Waves of a lake on the shore_96kHz - Nordliecht, CC0 1.0, <https://freesound.org/people/Nordliecht/sounds/816570/>

### rain_forest

- Forest cabin summer rain 2 - giddster, CC0 1.0, <https://freesound.org/people/giddster/sounds/719056/>
- Rain Forest Steady - mikaelacampbell18, CC BY 3.0, <https://freesound.org/people/mikaelacampbell18/sounds/617078/>
- Forest cabin summer rain 6 - giddster, CC0 1.0, <https://freesound.org/people/giddster/sounds/719060/>

### slow_river

- River very close prespective.WAV - marsal.oido, CC0 1.0, <https://freesound.org/people/marsal.oido/sounds/251655/>
- Soft flowing water very close to the river - Mixigodo, CC0 1.0, <https://freesound.org/people/Mixigodo/sounds/553606/>
- Flowing river. - Tom_Kaszuba, CC0 1.0, <https://freesound.org/people/Tom_Kaszuba/sounds/660252/>
- river or stream nearby good water movement flow detail.flac - kyles, CC0 1.0, <https://freesound.org/people/kyles/sounds/637790/>

### still_lake_ambience

- lake Zandmeren Hoenzadriel Gelderland Netherlands 1127 am 260225_0078 - klankbeeld, CC BY 4.0, <https://freesound.org/people/klankbeeld/sounds/846578/>
- lake Zandmeren Hoenzadriel Gelderland Netherlands 1137 am 260225_0078 - klankbeeld, CC BY 4.0, <https://freesound.org/people/klankbeeld/sounds/846492/>
- lakeside ambiance Sweden - Eelke, CC BY 4.0, <https://freesound.org/people/Eelke/sounds/462830/>

### thunder_close

- Thunderclap - Fission9, CC0 1.0, <https://freesound.org/people/Fission9/sounds/534023/>
- Huge Thunder Crack [MS] with declipper - -CASK-, CC BY 4.0, <https://freesound.org/people/-CASK-/sounds/732989/>
- G46-07-Loud Muffled Thunder Crack.wav - craigsmith, CC0 1.0, <https://freesound.org/people/craigsmith/sounds/438744/>
- Closeup Thunder Strike 01 - loganzsound, CC0 1.0, <https://freesound.org/people/loganzsound/sounds/840628/>
- 20260410 Amazing thunder - DeqstersLab, CC BY 4.0, <https://freesound.org/people/DeqstersLab/sounds/850941/>
- Thunder and Beginning of Rainfall - Fission9, CC0 1.0, <https://freesound.org/people/Fission9/sounds/505294/>
- Thunder Re-edit - Fission9, CC0 1.0, <https://freesound.org/people/Fission9/sounds/476228/>

### thunder_distant

- Rolling thunder closer rain drops ms 77mel 190903.flac - TRP, CC0 1.0, <https://freesound.org/people/TRP/sounds/574386/>
- 200930 Thunder, distant low rolling, after storm drips, roof 5pm.flac - TRP, CC0 1.0, <https://freesound.org/people/TRP/sounds/568969/>
- 120726 Thunder, rolling very distant dispersed, starts to rain at end, Toronto.w - TRP, CC0 1.0, <https://freesound.org/people/TRP/sounds/616607/>
- Rolling_Distant_Thunder_In_Heavy_Rain_04 - BlondPanda, CC0 1.0, <https://freesound.org/people/BlondPanda/sounds/778372/>
- Distant Thunder 1 - Fission9, CC0 1.0, <https://freesound.org/people/Fission9/sounds/581122/>
- 230405 Thunder DRY rolling distant low rumbles, roof, EM272s, Toronto 7am - TRP, CC0 1.0, <https://freesound.org/people/TRP/sounds/717845/>
- Distant Thunder 2 - Fission9, CC0 1.0, <https://freesound.org/people/Fission9/sounds/581123/>
- Thunder, distant rolling, med, noisy, 01.wav - TRP, CC0 1.0, <https://freesound.org/people/TRP/sounds/573669/>

### waterfall_small

- Small waterfall, creek running over rocks - Waiheke Island, Aotearoa / New Zeala - brentsimpson, CC0 1.0, <https://freesound.org/people/brentsimpson/sounds/727804/>
- petite chute d'eau a cap-bon-ami - MarleneAyni, CC0 1.0, <https://freesound.org/people/MarleneAyni/sounds/731703/>
- Waterfall sound. - Tom_Kaszuba, CC0 1.0, <https://freesound.org/people/Tom_Kaszuba/sounds/660255/>
- small waterfall near.wav - GiovanniProvenzale, CC BY 4.0, <https://freesound.org/people/GiovanniProvenzale/sounds/237157/>
- Brook falls into water - AugustSandberg, CC0 1.0, <https://freesound.org/people/AugustSandberg/sounds/677582/>
- Small Waterfall in a park - WhisperingEarth, CC0 1.0, <https://freesound.org/people/WhisperingEarth/sounds/800738/>

### wind_gusty

- Wind on bushes with muffled gust of wind, twig branch, close to desert ground - felix.blume, CC0 1.0, <https://freesound.org/people/felix.blume/sounds/711106/>
- storm wind in trees 1707 160221_0857.wav - klankbeeld, CC BY 4.0, <https://freesound.org/people/klankbeeld/sounds/677800/>
- wind bushes coast 903 PM 240311_0680 - klankbeeld, CC BY 4.0, <https://freesound.org/people/klankbeeld/sounds/735332/>
- wind trees 130914_00.wav - klankbeeld, CC BY 4.0, <https://freesound.org/people/klankbeeld/sounds/207295/>
- gusts on coniferous tree 1 - bruno.auzet, CC0 1.0, <https://freesound.org/people/bruno.auzet/sounds/829003/>

### wind_open

- Short Grass Prairie Environment in Wind Cave NP (USA) - lakewoodsound, CC BY 4.0, <https://freesound.org/people/lakewoodsound/sounds/696108/>
- Heavy Wind, Through Grass & Bushes - Kinoton, CC0 1.0, <https://freesound.org/people/Kinoton/sounds/656680/>
- reed-grass in wind 04 151011_0818.wav - klankbeeld, CC BY 4.0, <https://freesound.org/people/klankbeeld/sounds/328020/>
- reed-grass in wind 03 151011_0818.wav - klankbeeld, CC BY 4.0, <https://freesound.org/people/klankbeeld/sounds/326309/>
- Dry Grasses in Wind - flumpytripod, CC0 1.0, <https://freesound.org/people/flumpytripod/sounds/845415/>
- Ranch_Ambiance_Wind_Field_Grass 01.wav - n_audioman, CC BY 4.0, <https://freesound.org/people/n_audioman/sounds/321962/>
- Field ambience 01.wav - szelestamas, CC0 1.0, <https://freesound.org/people/szelestamas/sounds/620099/>

### wind_trees

- oak tree wind contact microphones 150111_0490.wav - klankbeeld, CC BY 4.0, <https://freesound.org/people/klankbeeld/sounds/261005/>
- storm wind in trees 1638 160221_0857.wav - klankbeeld, CC BY 4.0, <https://freesound.org/people/klankbeeld/sounds/665174/>
- wind tree silver birch 03 140215.wav - klankbeeld, CC BY 4.0, <https://freesound.org/people/klankbeeld/sounds/233987/>
- Windy atmosphere in the forest. - felix.blume, CC0 1.0, <https://freesound.org/people/felix.blume/sounds/414094/>

### wings_large

- animals cormorants inside nest loud wings mono DPA4061.wav - Soundholder, CC BY 3.0, <https://freesound.org/people/Soundholder/sounds/425375/>
- ravenflyby.aif - JamesOC, CC0 1.0, <https://freesound.org/people/JamesOC/sounds/86883/>
- Laulujoutsen, joutsen, toitotus, lentoonlähtö vedestä, ohilento / A whooper, who - YleArkisto, CC BY 4.0, <https://freesound.org/people/YleArkisto/sounds/264371/>
- Laulujoutsen, joutsen, parvi, lentoonlähtö vedestä / A whooper, whooper swan, a  - YleArkisto, CC BY 4.0, <https://freesound.org/people/YleArkisto/sounds/264374/>
- swans fly by 191218_0099.wav - klankbeeld, CC BY 4.0, <https://freesound.org/people/klankbeeld/sounds/501847/>
- swan1.wav - molkko, CC BY 4.0, <https://freesound.org/people/molkko/sounds/474253/>
- Birds (Geese) Flapping into Pond.wav - lolamadeus, CC0 1.0, <https://freesound.org/people/lolamadeus/sounds/179340/>
- Geese fly away - tripjazz, CC0 1.0, <https://freesound.org/people/tripjazz/sounds/512325/>

### wings_pigeon

- 230501_2409_2-1_FR_DoveFlying.wav - kevp888, CC BY 4.0, <https://freesound.org/people/kevp888/sounds/687350/>
- Pigeons_Marseille_1 - nicotep, CC0 1.0, <https://freesound.org/people/nicotep/sounds/741562/>
- Pigeon, Flies Away, Flapping Wings - Kinoton, CC0 1.0, <https://freesound.org/people/Kinoton/sounds/689998/>
- antwerpen duiven vleugels.wav - NickPeeters, CC0 1.0, <https://freesound.org/people/NickPeeters/sounds/415019/>
- Lipetskie Golubi - himura.kazuto, CC0 1.0, <https://freesound.org/people/himura.kazuto/sounds/466480/>
- R21-40b-Bird Wings Flapping.wav - craigsmith, CC0 1.0, <https://freesound.org/people/craigsmith/sounds/479605/>
- Pigeons flying - mcmikai, CC0 1.0, <https://freesound.org/people/mcmikai/sounds/532180/>
- porto_pigeons_flapping_32.wav - mhusinsky, CC BY 3.0, <https://freesound.org/people/mhusinsky/sounds/76447/>

### wings_small

- BirdFlap3_Deeper.wav - AgentDD, CC BY 3.0, <https://freesound.org/people/AgentDD/sounds/246223/>
- 230930 Wing flaps, light quiet sparrows, EM272s para, Toronto 9am - TRP, CC0 1.0, <https://freesound.org/people/TRP/sounds/717900/>
- Eurasian Blue Tit Call and Wing Sounds - naturenotesuk, CC BY 4.0, <https://freesound.org/people/naturenotesuk/sounds/830034/>
- Wing flaps flying many bird feeder NOTL 181224.flac - TRP, CC0 1.0, <https://freesound.org/people/TRP/sounds/574655/>
- FlapWings.aif - framixo, CC0 1.0, <https://freesound.org/people/framixo/sounds/463381/>
- Birds flapping wingsEQed.wav - juskiddink, CC BY 4.0, <https://freesound.org/people/juskiddink/sounds/65880/>
- S29-05 Bird wings flapping quickly; medium size.wav - craigsmith, CC0 1.0, <https://freesound.org/people/craigsmith/sounds/675959/>
- Birds_flapping 3.wav - deleted_user_2104797, CC0 1.0, <https://freesound.org/people/deleted_user_2104797/sounds/164475/>
- Birds_flapping 4.wav - deleted_user_2104797, CC0 1.0, <https://freesound.org/people/deleted_user_2104797/sounds/164474/>
- Birds_flapping 1.wav - deleted_user_2104797, CC0 1.0, <https://freesound.org/people/deleted_user_2104797/sounds/164477/>
- bird_flapping_1.wav - Clusman, CC0 1.0, <https://freesound.org/people/Clusman/sounds/543110/>
- 1. Wings Flapping - Lsoundaccount, CC0 1.0, <https://freesound.org/people/Lsoundaccount/sounds/753219/>

### woodpecker_drum

- Great Spotted Woodpecker (Dendrocopos major) drumming - Sonothèque ADVL, CC0 1.0, <https://xeno-canto.org/918609>
- Great Spotted Woodpecker (Dendrocopos major) drumming - Latal, CC BY 4.0, <https://xeno-canto.org/879162>
- Great Spotted Woodpecker (Dendrocopos major) drumming - Benoît Van Hecke, CC BY-SA 4.0, <https://xeno-canto.org/704068>
- Great Spotted Woodpecker (Dendrocopos major) drumming - Olivier Grimm, CC BY-SA 4.0, <https://xeno-canto.org/560950>
- Great Spotted Woodpecker (Dendrocopos major) drumming - Sonothèque ADVL, CC0 1.0, <https://xeno-canto.org/1042763>
- Great Spotted Woodpecker (Dendrocopos major) call, drumming, excitement call - Tanguy Loïs, CC BY-SA 4.0, <https://xeno-canto.org/894019>

## Feature tables

Reference: median [q1-q3] over the recordings. Ours: one render per row.

### brook (9 references)

| feature | reference | brook_pebbles | brook_rocks |
|---|---|---|---|
| band_79 | -32.2 [-40.7 .. -30] | -33.7 | -34.2 |
| band_252 | -24.5 [-26.4 .. -18.2] | -24.6 | -17.4* |
| band_504 | -13.6 [-20.3 .. -11.8] | -13 | -10.5* |
| band_1008 | -9.62 [-10.9 .. -8.56] | -8.15* | -12.1* |
| band_2016 | -11.7 [-12.5 .. -10.2] | -10.6 | -10* |
| band_4032 | -13.9 [-16.5 .. -11.2] | -15.9 | -14.3 |
| band_8063 | -14.2 [-21.4 .. -13.6] | -21.1 | -17.7 |
| centroid_med | 2.38e+03 [1.58e+03 .. 2.71e+03] | 1.78e+03 | 1.88e+03 |
| flatness_db | -7.19 [-11 .. -6.45] | -9.47 | -7.74 |
| am_0.1-1 | -23.2 [-24.4 .. -22.1] | -12.4* | -16.3* |
| am_1-4 | -20.2 [-21.2 .. -18.5] | -18.3* | -25.8* |
| am_4-16 | -14.9 [-16.3 .. -14] | -13.5* | -27* |
| am_16-64 | -12.3 [-13.5 .. -11.7] | -10.5* | -21.7* |
| ampeak_hz | 4.43 [3.64 .. 5.37] | 5.72* | 2.2* |
| crest_db | 5.77 [4.73 .. 6.21] | 6.98* | 4.65* |
| spread_db | 5.23 [4.56 .. 7.55] | 10.1* | 6.33 |
| onset_rate | 3.25 [2.17 .. 3.61] | 5.42* | 0* |
| ioi_cv | 0.84 [0.721 .. 0.841] | 0.867* | - |
| event_ms | 170 [120 .. 220] | 90* | - |
| tonal_db | 1.78 [1.11 .. 3.8] | 0.771* | 0.777* |
| dom_hz | 816 [586 .. 1.04e+03] | 781 | 430* |
| distance | | 0.80 | 1.72 |

### frogs_common (5 references)

| feature | reference | common_frogs |
|---|---|---|
| band_79 | -21.6 [-32.6 .. -16.3] | -21.3 |
| band_252 | -18.6 [-25.4 .. -18.5] | -13.7* |
| band_504 | -4.25 [-8.04 .. -3.35] | -3.7 |
| band_1008 | -21.4 [-22.7 .. -16.5] | -15.7* |
| band_2016 | -30.6 [-31.8 .. -25.4] | -25.7 |
| band_4032 | -36 [-37.9 .. -28.6] | -35.8 |
| band_8063 | -37.4 [-39.4 .. -33.7] | -44.5* |
| centroid_med | 528 [526 .. 537] | 487* |
| flatness_db | -18.1 [-18.6 .. -17.4] | -27.7* |
| am_0.1-1 | -10.3 [-12.8 .. 0.985] | -5.88 |
| am_1-4 | -10.4 [-14.6 .. -6.37] | -7.1 |
| am_4-16 | -10.3 [-16.9 .. -5.69] | -16.8 |
| am_16-64 | -6.47 [-8.65 .. 3.22] | 1.86 |
| ampeak_hz | 30.9 [14.1 .. 31] | 24.6 |
| crest_db | 10.6 [8 .. 22.5] | 10.7 |
| spread_db | 14.7 [11.6 .. 30.2] | 26.4 |
| onset_rate | 4.68 [4.05 .. 5.9] | 18.4* |
| ioi_cv | 1.19 [0.971 .. 1.83] | 1.04 |
| event_ms | 60 [40 .. 70] | 10* |
| tonal_db | 6.14 [5.8 .. 6.22] | 5.56* |
| dom_hz | 453 [430 .. 480] | 473 |
| distance | | 1.02 |

### frogs_marsh (3 references)

| feature | reference | marsh_frogs |
|---|---|---|
| band_79 | -33.5 [-37 .. -29.8] | -40.2* |
| band_252 | -29.7 [-39.1 .. -25.6] | -21.6* |
| band_504 | -21.5 [-28.8 .. -21.3] | -17.8* |
| band_1008 | -19.9 [-20.1 .. -18.6] | -11.1* |
| band_2016 | -4.18 [-4.8 .. -3.26] | -14.1* |
| band_4032 | -16.5 [-24.5 .. -12.9] | -18.2 |
| band_8063 | -41.5 [-44.1 .. -35.5] | -30.3* |
| centroid_med | 1.55e+03 [1e+03 .. 1.62e+03] | 1.47e+03 |
| flatness_db | -16.8 [-19.7 .. -14.4] | -15.7 |
| am_0.1-1 | -2.38 [-3.07 .. -1.84] | -3.38* |
| am_1-4 | -6.09 [-6.11 .. -4.74] | -12.6* |
| am_4-16 | 1.79 [0.566 .. 1.8] | -2.07* |
| am_16-64 | -4.44 [-6.14 .. -3.03] | 2.44* |
| ampeak_hz | 8.43 [7.57 .. 9.37] | 6.54* |
| crest_db | 17.7 [17.3 .. 23.2] | 18 |
| spread_db | 25.8 [22.9 .. 28.8] | 33.5* |
| onset_rate | 4.01 [3.78 .. 4.68] | 9.96* |
| ioi_cv | 1.01 [0.898 .. 1.61] | 2.4* |
| event_ms | 50 [40 .. 62.5] | 30* |
| tonal_db | 8.18 [8.11 .. 9.99] | 5.88* |
| dom_hz | 1.73e+03 [1.67e+03 .. 1.91e+03] | 1.46e+03* |
| distance | | 1.37 |

### frogs_tree (5 references)

| feature | reference | tree_frogs |
|---|---|---|
| band_79 | -42.7 [-44.1 .. -36.4] | -31.2* |
| band_252 | -41.8 [-43.4 .. -38.7] | -27.2* |
| band_504 | -40.5 [-42.2 .. -36.7] | -21.4* |
| band_1008 | -19.1 [-22 .. -17.9] | -16.9* |
| band_2016 | -6.95 [-19.6 .. -2.92] | -2.58* |
| band_4032 | -25.1 [-30.6 .. -23.9] | -17.3* |
| band_8063 | -37.6 [-40.9 .. -37.1] | -25.8* |
| centroid_med | 2.37e+03 [2.14e+03 .. 2.41e+03] | 2.15e+03 |
| flatness_db | -27.8 [-29.1 .. -21.9] | -13.5* |
| am_0.1-1 | -15.2 [-18.6 .. -14.6] | 0.652* |
| am_1-4 | -16.3 [-18.9 .. -9.58] | -13.9 |
| am_4-16 | -9.72 [-12.3 .. -6.02] | 1.2* |
| am_16-64 | -14.5 [-14.6 .. -13.6] | -0.246* |
| ampeak_hz | 4.22 [3.74 .. 6.89] | 13.9* |
| crest_db | 7.73 [6.61 .. 15.7] | 27.1* |
| spread_db | 12.2 [9.42 .. 22] | 37.4* |
| onset_rate | 5.55 [2.79 .. 6.24] | 5.03 |
| ioi_cv | 0.715 [0.556 .. 0.856] | 3.54* |
| event_ms | 80 [70 .. 90] | 20* |
| tonal_db | 9.86 [8.65 .. 10.8] | 5.51* |
| dom_hz | 2.47e+03 [2.14e+03 .. 2.7e+03] | 2.21e+03 |
| distance | | 2.21 |

### lake_lapping (6 references)

| feature | reference | lake_lapping |
|---|---|---|
| band_79 | -27.9 [-38.9 .. -23] | -34.5 |
| band_252 | -15.7 [-19.2 .. -14] | -28.1* |
| band_504 | -9.88 [-10.6 .. -8.94] | -17.3* |
| band_1008 | -9.92 [-10.8 .. -9.28] | -10.2 |
| band_2016 | -12.3 [-13 .. -10.8] | -9.7* |
| band_4032 | -19.1 [-19.5 .. -16.7] | -22.4* |
| band_8063 | -21.6 [-23.9 .. -18.8] | -36* |
| centroid_med | 1.1e+03 [1.06e+03 .. 1.36e+03] | 2.93e+03* |
| flatness_db | -14.5 [-15.8 .. -9.94] | -5.18* |
| am_0.1-1 | -9.02 [-9.98 .. -6.19] | -7.91 |
| am_1-4 | -12.6 [-15.9 .. -11] | 1.42* |
| am_4-16 | -11.3 [-16.3 .. -8] | 1.39* |
| am_16-64 | -11.4 [-14.7 .. -8.07] | 2.99* |
| ampeak_hz | 2.54 [2.1 .. 3.36] | 2.55 |
| crest_db | 11.2 [9.34 .. 11.8] | 28.5* |
| spread_db | 14.4 [13.8 .. 14.6] | 32.4* |
| onset_rate | 2.46 [1.07 .. 5.04] | 4.52 |
| ioi_cv | 0.894 [0.763 .. 1.04] | 1.07* |
| event_ms | 110 [66.2 .. 278] | 20* |
| tonal_db | 3.46 [2.45 .. 4.18] | 2.15* |
| dom_hz | 410 [289 .. 572] | 1.18e+03* |
| distance | | 2.01 |

### rain_forest (3 references)

| feature | reference | on_leaves | under_canopy |
|---|---|---|---|
| band_79 | -27.9 [-39.7 .. -23.8] | -30.3 | -35.7 |
| band_252 | -26.3 [-26.7 .. -21.7] | -22.7 | -29* |
| band_504 | -14.5 [-21.8 .. -14.5] | -8.46* | -21.9* |
| band_1008 | -11.2 [-17 .. -11] | -17.5* | -11.4 |
| band_2016 | -13.5 [-20.1 .. -12.6] | -16.1 | -11.5* |
| band_4032 | -15.1 [-22.6 .. -13.6] | -11.3* | -8.8* |
| band_8063 | -19.7 [-26.5 .. -15.5] | -16.5 | -13.1* |
| centroid_med | 1.78e+03 [1.52e+03 .. 2.57e+03] | 2.11e+03 | 3.59e+03* |
| flatness_db | -7.57 [-8.59 .. -5.99] | -7.27 | -4.75* |
| am_0.1-1 | -18.5 [-23.8 .. -16.6] | -26.7* | -29.3* |
| am_1-4 | -18.9 [-22.8 .. -16.3] | -21 | -24.2* |
| am_4-16 | -18.2 [-20.5 .. -13.5] | -14.8 | -17.9 |
| am_16-64 | -15 [-17 .. -10.7] | -10.8 | -12.6 |
| ampeak_hz | 3.71 [2.92 .. 21.6] | 14.3 | 43.8* |
| crest_db | 6.67 [4.53 .. 6.82] | 3.52* | 2.93* |
| spread_db | 4.97 [3.69 .. 7.06] | 4.3 | 3.29* |
| onset_rate | 0.402 [0.277 .. 5.66] | 4.73 | 1.7 |
| ioi_cv | 0.777 [0.643 .. 1.22] | 0.774 | 0.902 |
| event_ms | 280 [165 .. 2.41e+03] | 100* | 370 |
| tonal_db | 2.56 [2.05 .. 2.58] | 0.977* | 0.841* |
| dom_hz | 398 [281 .. 646] | 680* | 988* |
| distance | | 0.67 | 0.85 |

### slow_river (4 references)

| feature | reference | slow_river |
|---|---|---|
| band_79 | -37.4 [-41.4 .. -31.8] | -29.5* |
| band_252 | -19.2 [-21.3 .. -16.9] | -21.3* |
| band_504 | -11.2 [-11.8 .. -10.9] | -5.81* |
| band_1008 | -9.23 [-9.57 .. -9.2] | -10.3* |
| band_2016 | -11.7 [-12.5 .. -10.7] | -13.2* |
| band_4032 | -15.8 [-16.7 .. -14.7] | -34.9* |
| band_8063 | -18.7 [-19.4 .. -16.9] | -43* |
| centroid_med | 1.6e+03 [1.42e+03 .. 1.95e+03] | 958* |
| flatness_db | -9.12 [-9.74 .. -8.01] | -11.3* |
| am_0.1-1 | -26.4 [-27.6 .. -26.1] | -7.94* |
| am_1-4 | -22.9 [-24.7 .. -22.1] | -3.08* |
| am_4-16 | -18.4 [-19.4 .. -18] | 0.593* |
| am_16-64 | -15.8 [-16.4 .. -15.4] | 0.564* |
| ampeak_hz | 7.71 [4.97 .. 12.3] | 2.65* |
| crest_db | 3.16 [2.78 .. 3.29] | 26.8* |
| spread_db | 3.26 [2.83 .. 3.45] | 0* |
| onset_rate | 0.49 [0.368 .. 0.682] | 0.15* |
| ioi_cv | 0.915 [0.876 .. 0.936] | 0.911 |
| event_ms | 535 [495 .. 1.05e+03] | 20* |
| tonal_db | 1.82 [1.38 .. 2.48] | 3.69* |
| dom_hz | 506 [388 .. 613] | 547 |
| distance | | 2.07 |

### still_lake_ambience (3 references)

| feature | reference | still_lake |
|---|---|---|
| band_79 | -5.11 [-21.9 .. -3.72] | -35.9* |
| band_252 | -21.5 [-23.4 .. -21] | -29.7* |
| band_504 | -25.1 [-26.2 .. -18.6] | -25.4 |
| band_1008 | -18.3 [-20.5 .. -14] | -11.1* |
| band_2016 | -28.2 [-29.2 .. -19.9] | -9.71* |
| band_4032 | -27.2 [-29.6 .. -21.2] | -23.2 |
| band_8063 | -32.1 [-35.5 .. -23.5] | -37.2* |
| centroid_med | 154 [132 .. 923] | 2.24e+03* |
| flatness_db | -24.8 [-26.9 .. -16.8] | -7.52* |
| am_0.1-1 | -17.1 [-18.7 .. -12.1] | -7.39* |
| am_1-4 | -16.8 [-17.3 .. -8.83] | -0.586* |
| am_4-16 | -13.9 [-14 .. -7.6] | 3.03* |
| am_16-64 | -9.1 [-10.1 .. -5.77] | 4.41* |
| ampeak_hz | 99.5 [51.1 .. 99.6] | 2.4* |
| crest_db | 6.58 [5.42 .. 12.4] | 27.1* |
| spread_db | 7.06 [6.41 .. 13.1] | 32.8* |
| onset_rate | 7.57 [7.22 .. 7.62] | 5.37* |
| ioi_cv | 0.757 [0.726 .. 0.767] | 0.803* |
| event_ms | 80 [65 .. 85] | 20* |
| tonal_db | 4.53 [4 .. 5.91] | 1.48* |
| dom_hz | 113 [107 .. 455] | 1.18e+03* |
| distance | | 2.48 |

### thunder_close (7 references)

| feature | reference | close_1500m | storm_1500m |
|---|---|---|---|
| band_79 | -16.4 [-18.1 .. -15.3] | -9.29* | -9.29* |
| band_252 | -9.69 [-9.82 .. -8.48] | -12.9* | -12.9* |
| band_504 | -11.3 [-13.7 .. -10.1] | -17.5* | -18* |
| band_1008 | -16 [-18.9 .. -15.8] | -25.2* | -25.9* |
| band_2016 | -24.1 [-27.9 .. -23.5] | -35.1* | -36.2* |
| band_4032 | -37.6 [-38.4 .. -32.5] | -45.8* | -46.5* |
| band_8063 | -51.7 [-58.4 .. -45.1] | -51.7 | -51.7 |
| centroid_med | 294 [251 .. 359] | 152* | 144* |
| flatness_db | -29.9 [-37.5 .. -25.9] | -38.6* | -38.8* |
| am_0.1-1 | -10.5 [-12.3 .. -5.42] | -1.99* | -0.352* |
| am_1-4 | -12.4 [-14.9 .. -8.4] | -7.32* | -3.39* |
| am_4-16 | -11.5 [-14.7 .. -10.4] | -7.54* | -4.63* |
| am_16-64 | -10.7 [-14.5 .. -8.38] | -6.83* | -4.92* |
| ampeak_hz | 2.53 [2.18 .. 3.62] | 2.21 | 2.02* |
| crest_db | 23.4 [21.9 .. 28.1] | 25.2 | 22.6 |
| spread_db | 26.3 [23.7 .. 27.4] | 25 | 26.3 |
| onset_rate | 3.35 [2.93 .. 3.79] | 2.52* | 3.58 |
| ioi_cv | 0.951 [0.901 .. 1.74] | 5.57* | 4.28* |
| event_ms | 70 [60 .. 90] | 60 | 70 |
| tonal_db | 6.21 [5.69 .. 6.54] | 2.74* | 3.65* |
| dom_hz | 184 [160 .. 219] | 102* | 109* |
| attack_s | 0.25 [0.175 .. 0.9] | 0.75 | 0.775 |
| decay20_s | 1.6 [1.33 .. 4.03] | 2.35 | 2.1 |
| distance | | 1.28 | 1.21 |

### thunder_distant (8 references)

| feature | reference | distant_8000m | storm_8000m |
|---|---|---|---|
| band_79 | -10.4 [-11.5 .. -9.39] | -7.45* | -7.71* |
| band_252 | -14.4 [-16.9 .. -9.96] | -16.1 | -15.3 |
| band_504 | -20.2 [-22.2 .. -18.7] | -26.1* | -25.9* |
| band_1008 | -31.3 [-35 .. -28.4] | -37.9* | -38* |
| band_2016 | -45.6 [-55 .. -41.8] | -45.8 | -45.2 |
| band_4032 | -55.3 [-66 .. -41.3] | -49.4 | -48.7 |
| band_8063 | -60 [-74.8 .. -46.4] | -52.2 | -51.6 |
| centroid_med | 178 [114 .. 221] | 113* | 119 |
| flatness_db | -38.6 [-51.9 .. -26.6] | -41.1 | -40.8 |
| am_0.1-1 | -5.36 [-7.98 .. -3.89] | -2.14* | -0.171* |
| am_1-4 | -8.53 [-11.5 .. -7.26] | -9.73 | -10.8 |
| am_4-16 | -7.55 [-9.75 .. -7.17] | -7.88 | -8.54 |
| am_16-64 | -6.67 [-7.2 .. -5.75] | -5.38* | -5.03* |
| ampeak_hz | 2.64 [2.41 .. 2.98] | 2.83 | 3.28* |
| crest_db | 17.4 [15.2 .. 18.9] | 28.2* | 27.4* |
| spread_db | 19.6 [17.2 .. 25.7] | 28.7* | 27.8* |
| onset_rate | 4.95 [3.92 .. 8.09] | 2.7* | 2.25* |
| ioi_cv | 0.778 [0.692 .. 0.946] | 6.45* | 6.24* |
| event_ms | 80 [70 .. 92.5] | 50* | 60* |
| tonal_db | 3.23 [2.41 .. 3.71] | 1.97* | 2.97 |
| dom_hz | 109 [104 .. 137] | 109 | 102* |
| attack_s | 7.21 [2.09 .. 12.7] | 2.7 | 1.78* |
| decay20_s | 8.03 [2.81 .. 9.94] | 2.3* | 2.25* |
| distance | | 1.19 | 1.23 |

### waterfall_small (6 references)

| feature | reference | small_waterfall |
|---|---|---|
| band_79 | -27 [-29.4 .. -20.7] | -42.7* |
| band_252 | -23.1 [-25.5 .. -15.9] | -28.7* |
| band_504 | -16.5 [-18.9 .. -13.9] | -19.1* |
| band_1008 | -11.6 [-14.6 .. -10] | -10.2 |
| band_2016 | -13.7 [-14 .. -13.2] | -13.5 |
| band_4032 | -13.3 [-16.3 .. -10.3] | -9.04* |
| band_8063 | -14.7 [-25.2 .. -12.9] | -13.1 |
| centroid_med | 2.67e+03 [1.47e+03 .. 3.51e+03] | 3.42e+03 |
| flatness_db | -5.91 [-12.2 .. -5.43] | -4.79* |
| am_0.1-1 | -27.6 [-29.8 .. -25.2] | -20.6* |
| am_1-4 | -24.5 [-27.2 .. -22.3] | -30.6* |
| am_4-16 | -19.2 [-22 .. -16.9] | -30.5* |
| am_16-64 | -16.7 [-17.9 .. -13.2] | -25.1* |
| ampeak_hz | 11.7 [9.25 .. 15.5] | 66.3* |
| crest_db | 3.21 [2.41 .. 4.13] | 3.27 |
| spread_db | 3.61 [2.27 .. 4.48] | 4.65* |
| onset_rate | 0.341 [0.165 .. 2.15] | 0* |
| ioi_cv | 0.936 [0.861 .. 1.03] | - |
| event_ms | 920 [274 .. 1.49e+03] | - |
| tonal_db | 2.26 [2.02 .. 2.45] | 0.748* |
| dom_hz | 650 [216 .. 1.2e+03] | 957 |
| distance | | 1.61 |

### wind_gusty (5 references)

| feature | reference | breeze_6ms_gusty | forest_roar_7ms_gusty |
|---|---|---|---|
| band_79 | -16 [-19.9 .. -13] | -21.4* | -12.4* |
| band_252 | -13.2 [-14.5 .. -10.7] | -16.8* | -12.7 |
| band_504 | -13.4 [-14.7 .. -12.2] | -14.6 | -12.9 |
| band_1008 | -14.8 [-15.6 .. -13.1] | -12.5* | -13.9 |
| band_2016 | -23.8 [-25.7 .. -17.1] | -10.5* | -16.2* |
| band_4032 | -28.6 [-31.6 .. -18.7] | -10.9* | -21.8 |
| band_8063 | -33.4 [-34.6 .. -29.3] | -15.6* | -32.7 |
| centroid_med | 452 [385 .. 825] | 1.59e+03* | 625 |
| flatness_db | -17.9 [-22.3 .. -16.4] | -9.17* | -18.6 |
| am_0.1-1 | -13.8 [-18.4 .. -8.81] | -0.52* | -13.8 |
| am_1-4 | -23.7 [-24.2 .. -14.8] | -17.8 | -20.7 |
| am_4-16 | -18.4 [-20.4 .. -14.6] | -23.3* | -17.9 |
| am_16-64 | -12.8 [-14.6 .. -11.4] | -19.6* | -13.6 |
| ampeak_hz | 18 [3.38 .. 25] | 2.35* | 2.81* |
| crest_db | 8.18 [4.51 .. 10.7] | 17.5* | 9.77 |
| spread_db | 11.1 [5.93 .. 14.4] | 38.8* | 13.6 |
| onset_rate | 1.85 [0.238 .. 1.98] | 0.117* | 0.85 |
| ioi_cv | 1.25 [1.1 .. 1.51] | 1.06* | 1.3 |
| event_ms | 270 [130 .. 1.66e+03] | 2.16e+03* | 360 |
| tonal_db | 1.79 [1.75 .. 3.74] | 1.22* | 0.907* |
| dom_hz | 105 [102 .. 133] | 113 | 109 |
| distance | | 1.56 | 0.38 |

### wind_open (7 references)

| feature | reference | breeze_4ms | dune_sand_6ms | foliage_grass_4ms |
|---|---|---|---|---|
| band_79 | -17.3 [-23.1 .. -13.3] | -19.6 | -29.1* | -62.1* |
| band_252 | -20.5 [-25.9 .. -18] | -15.1* | -22 | -47.7* |
| band_504 | -23.9 [-27.1 .. -23.7] | -12.4* | -18.8* | -38.6* |
| band_1008 | -27.3 [-29.2 .. -19.4] | -10.2* | -15.7* | -29.7* |
| band_2016 | -24.1 [-32.2 .. -18.6] | -11.2* | -12.8* | -20.5 |
| band_4032 | -23.9 [-29.1 .. -16.8] | -15.4* | -9.69* | -11.8* |
| band_8063 | -20.8 [-28.5 .. -8.52] | -20.6 | -9.65 | -6.71* |
| centroid_med | 1.54e+03 [889 .. 3.9e+03] | 1.15e+03 | 4.25e+03* | 7.24e+03* |
| flatness_db | -8.52 [-13 .. -5.95] | -11.1 | -3.46* | -4.42* |
| am_0.1-1 | -11.8 [-15.8 .. -11] | -5.16* | -7.27* | -5.55* |
| am_1-4 | -15.4 [-23.7 .. -11.8] | -18.6 | -21.7 | -14.1 |
| am_4-16 | -13.3 [-21 .. -10.9] | -20.8 | -22.1* | -19.5 |
| am_16-64 | -10.3 [-16.9 .. -7.24] | -18.2* | -21.1* | -19.4* |
| ampeak_hz | 2.89 [2.68 .. 4.45] | 2.06* | 2.52* | 2.22* |
| crest_db | 11.5 [6.93 .. 12.6] | 11.9 | 8.87 | 12.1 |
| spread_db | 12.2 [11.3 .. 14.2] | 18.6* | 16.1* | 16.7* |
| onset_rate | 1.65 [0.872 .. 9.22] | 0.0444* | 0* | 0.411* |
| ioi_cv | 0.905 [0.723 .. 1.24] | 0.718* | - | 2.86* |
| event_ms | 200 [60 .. 875] | 4.6e+03* | - | 120 |
| tonal_db | 3.24 [1.71 .. 4.31] | 1.14* | 0.881* | 1.02* |
| dom_hz | 102 [102 .. 113] | 105 | 2.75e+03* | 7.16e+03* |
| distance | | 1.01 | 1.91 | 1.59 |

### wind_trees (4 references)

| feature | reference | before_mix_breeze_oak | before_foliage_oak | before_foliage_spruce | before_mix_roar_spruce | before_wind_breeze | before_wind_forest_roar | before_wind_storm | a5_foliage_oak | a5_foliage_spruce | a5_mix_breeze_oak | a5_mix_roar_spruce | a5_wind_breeze | a5_wind_forest_roar | a5_wind_storm | t_spruce_nocreak | breeze_3ms | foliage_birch_4ms | foliage_oak_5ms | forest_roar_5ms |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| band_79 | -15.6 [-15.8 .. -14.8] | -36.2* | -48.2* | -51.7* | -17.9 | -21* | -14.1* | -17.7 | -50.1* | -45.3* | -32.7* | -16.9 | -19.6 | -13.5* | -16.8 | -44.2* | -19.3* | -51.8* | -48.1* | -12.9* |
| band_252 | -12.9 [-14.3 .. -12.1] | -29.7* | -33* | -35.4* | -17.3 | -17 | -12.3 | -16.3 | -34.6* | -28.6* | -26.7* | -15.7 | -14.4 | -11.9 | -14.2 | -27.5* | -15.1* | -36.6* | -32.6* | -12.3 |
| band_504 | -10.6 [-16.2 .. -10.3] | -21.7 | -7.31* | -7.04* | -18.8 | -14.8 | -13.4 | -15.7 | -4.45* | -6.9* | -18.4 | -15.2 | -12 | -13 | -13.8 | -15.5 | -12.2 | -17* | -14.6 | -12.6 |
| band_1008 | -12.2 [-17.6 .. -11.4] | -14.3 | -15.1 | -15.1 | -15.4 | -12.7 | -14.8 | -14.4 | -17.3 | -8.96* | -15.1 | -11.6* | -10.3* | -14.3 | -13 | -8.22* | -10.1* | -18.5* | -14.9 | -14 |
| band_2016 | -20.6 [-26.8 .. -18.9] | -8.82* | -9.54* | -9.44* | -10.8* | -10.8* | -15.8* | -15.1* | -11.4* | -10.8* | -9.7* | -10.1* | -11.5* | -16.3* | -12.8* | -9.24* | -11.3* | -11.4* | -9.46* | -16* |
| band_4032 | -26.3 [-33.8 .. -24.3] | -9.88* | -10.6* | -9.69* | -10.1* | -10.4* | -22 | -14.6 | -11.1* | -17.2 | -9.51* | -16.3 | -15.6 | -23.3 | -12.9 | -16.1 | -15.5* | -7.98* | -9.35* | -23* |
| band_8063 | -36.3 [-45.3 .. -32.7] | -13.9* | -14.7* | -16.7* | -14.5* | -16* | -33 | -16.8* | -15.1* | -21.6* | -13.5* | -21.8* | -20.8* | -34.3 | -16.6* | -20.5* | -20.7* | -11.4* | -13.4* | -33.9 |
| centroid_med | 491 [384 .. 583] | 3.25e+03* | 3.38e+03* | 1.06e+03* | 822* | 1.13e+03* | 541 | 1.43e+03* | 3.63e+03* | 1.47e+03* | 3.44e+03* | 1.26e+03* | 1.27e+03* | 579 | 1.78e+03* | 1.53e+03* | 977* | 4.46e+03* | 3.59e+03* | 607* |
| flatness_db | -23.2 [-29.9 .. -21.1] | -5.24* | -5.33* | -12.4* | -14.1* | -11.3* | -20 | -8.93* | -4.98* | -10.8* | -4.83* | -11.3* | -10.5* | -19.4 | -7.74* | -10.6* | -12.1* | -4.34* | -4.99* | -19.4* |
| am_0.1-1 | -16.6 [-19.9 .. -14.1] | 0.404* | -0.49* | -1.18* | -4.94* | 1.2* | -2.9* | -4.75* | -7.72* | -13.5 | -11 | -17.8 | -16.6 | -15.6 | -13.4 | -14.2 | -1.48* | -4.45* | -1.81* | -14.7 |
| am_1-4 | -25.4 [-26.9 .. -23.5] | -14.4* | -10.1* | -11.3* | -15.8* | -8.75* | -7.32* | -11.3* | -14.4* | -18.5 | -15.9* | -23.8 | -20.9 | -19.3 | -21.5 | -21.7 | -17.7* | -17.3* | -13.2* | -19.2* |
| am_4-16 | -21 [-21.4 .. -19.2] | -21.6* | -11.9* | -11.8* | -19 | -13.4* | -9.14* | -12* | -15.7 | -19.7 | -19.8 | -23.4* | -23.4* | -18.7 | -23.4* | -23.6* | -19.5 | -20.9 | -17.7* | -17* |
| am_16-64 | -14.9 [-15.7 .. -13.7] | -16.3* | -15.3 | -14.4 | -15.5 | -14.2 | -8.34* | -13 | -20.1* | -19.2* | -22.1* | -18.4* | -19.9* | -14.3 | -18.9* | -19.2* | -16.7* | -22.2* | -19.4* | -13.3* |
| ampeak_hz | 18.7 [11.9 .. 24.6] | 2.05* | 2.03* | 2.08* | 2.55* | 2.1* | 2.08* | 2.75* | 2.52* | 2.05* | 2.2* | 2.3* | 2.13* | 2.58* | 2.08* | 2.13* | 2.06* | 2.04* | 2.16* | 2.18* |
| crest_db | 7.25 [6.18 .. 8.57] | 27.4* | 26.1* | 25.1* | 20* | 27* | 21.2* | 24.1* | 14.6* | 10.2 | 9.1 | 5.09* | 6.43* | 4.96* | 5.8* | 7.88 | 18* | 14.8* | 18.2* | 8.36 |
| spread_db | 8.83 [7.54 .. 11.6] | 38.1* | 35.7* | 38* | 38.4* | 38.7* | 34.8* | 36.1* | 14.3 | 12.1 | 13.2 | 6.93* | 8.04 | 7.01* | 10.9 | 8.88 | 33.4* | 26.9* | 36.3* | 12.8* |
| onset_rate | 0.33 [0.188 .. 2.33] | 0.05* | 0.167* | 0.3* | 0.383 | 0.183* | 1.55 | 0.0833* | 0.217* | 0.283* | 0.0333* | 0.05* | 0* | 0.833 | 0* | 0.0333* | 0.244 | 0.189 | 0.478 | 0.6 |
| ioi_cv | 1.2 [1.05 .. 1.32] | - | 2.11* | 2.14* | 2.5* | 1.34 | 1.39 | 0.914* | 1.33 | 2.38* | - | - | - | 1.32 | - | - | 1.52* | 1.25 | 1.71* | 1.43* |
| event_ms | 800 [539 .. 1.16e+03] | 2e+03* | 155 | 145 | 240 | 720 | 270 | 1.41e+03* | 160 | 120 | 3.83e+03* | 2.91e+03* | - | 555 | - | 1.86e+04* | 925 | 1.63e+03* | 360* | 530* |
| tonal_db | 1.7 [1.01 .. 2.38] | 10.1* | 19.6* | 21.1* | 11* | 2.08 | 1.95 | 13.5* | 20.2* | 16.8* | 14.2* | 7.94* | 1.09* | 1.01* | 6.87* | 1.37* | 1.53 | 10.5* | 12.3* | 0.878* |
| dom_hz | 137 [122 .. 150] | 598* | 469* | 465* | 598* | 102* | 168 | 180* | 469* | 469* | 598* | 598* | 102* | 102* | 180* | 895* | 105* | 539* | 539* | 102* |
| distance | | 2.48 | 2.74 | 2.66 | 2.20 | 1.79 | 1.38 | 2.01 | 1.82 | 1.75 | 1.85 | 1.50 | 1.69 | 0.83 | 1.51 | 1.57 | 1.75 | 2.56 | 2.95 | 0.94 |

### wings_large (8 references)

| feature | reference | gull | raven |
|---|---|---|---|
| band_79 | -20 [-27.4 .. -12.2] | -34.2* | -34.2* |
| band_252 | -19.2 [-25.7 .. -14.8] | -19.1 | -19.1 |
| band_504 | -19.4 [-22.2 .. -12.3] | -10.8* | -10.8* |
| band_1008 | -11.5 [-13.8 .. -9.78] | -9.4* | -9.4* |
| band_2016 | -19.9 [-22.4 .. -17.6] | -11.4* | -11.4* |
| band_4032 | -23.6 [-25.3 .. -21.9] | -14.8* | -14.8* |
| band_8063 | -36.4 [-42.4 .. -31.2] | -18.5* | -18.5* |
| centroid_med | 601 [434 .. 993] | 1.53e+03* | 1.53e+03* |
| flatness_db | -23.2 [-26.5 .. -14.6] | -9.05* | -9.05* |
| am_0.1-1 | -12 [-15.2 .. -4.27] | 3.95* | 3.95* |
| am_1-4 | -11.1 [-14.5 .. -5.9] | 1.51* | 1.51* |
| am_4-16 | -8.56 [-11.8 .. -3] | -2.66* | -2.66* |
| am_16-64 | -10.4 [-11.3 .. -8.45] | -13* | -13* |
| ampeak_hz | 4.12 [3.58 .. 5.88] | 2.95* | 2.95* |
| crest_db | 15.8 [12.7 .. 20.6] | 29.9* | 29.9* |
| spread_db | 18.8 [13.8 .. 21.8] | 30.8* | 30.8* |
| onset_rate | 5.3 [2.11 .. 6.04] | 0.833* | 0.833* |
| ioi_cv | 0.875 [0.826 .. 1.62] | 1.31 | 1.31 |
| event_ms | 75 [57.5 .. 102] | 140* | 140* |
| tonal_db | 7.9 [5.16 .. 9.9] | 2.71* | 2.71* |
| dom_hz | 363 [102 .. 722] | 781* | 781* |
| distance | | 1.51 | 1.51 |

### wings_pigeon (8 references)

| feature | reference | wood_pigeon |
|---|---|---|
| band_79 | -18.2 [-23 .. -16.3] | -41.5* |
| band_252 | -20.8 [-34.8 .. -13.2] | -26.5 |
| band_504 | -18.6 [-29 .. -13.5] | -17.2 |
| band_1008 | -18.5 [-30.9 .. -16.7] | -10.2* |
| band_2016 | -23.2 [-33.7 .. -19.7] | -9.44* |
| band_4032 | -26.3 [-32.7 .. -18.8] | -11.5* |
| band_8063 | -31.1 [-36.3 .. -20.2] | -15.5* |
| centroid_med | 466 [354 .. 997] | 2.42e+03* |
| flatness_db | -18.2 [-20.9 .. -13.5] | -6.64* |
| am_0.1-1 | -2.2 [-9.18 .. 1.58] | -1.93 |
| am_1-4 | -9.76 [-10.9 .. -6.15] | -3.66* |
| am_4-16 | -6.94 [-12.9 .. -4.23] | 3.2* |
| am_16-64 | -5.68 [-8.86 .. -3.31] | -10.8* |
| ampeak_hz | 2.63 [2.17 .. 7.37] | 6.78 |
| crest_db | 15.9 [12.1 .. 23.7] | 26.5* |
| spread_db | 15 [12 .. 29.6] | 33.6* |
| onset_rate | 5.06 [3.96 .. 6.88] | 5.42 |
| ioi_cv | 0.84 [0.453 .. 0.99] | 2.39* |
| event_ms | 55 [47.5 .. 75] | 60 |
| tonal_db | 6.37 [5.72 .. 8.1] | 1.51* |
| dom_hz | 219 [102 .. 324] | 1.28e+03* |
| distance | | 1.45 |

### wings_small (12 references)

| feature | reference | chaffinch | robin | swallow |
|---|---|---|---|---|
| band_79 | -14.8 [-25.4 .. -13.5] | -48.6* | -47.6* | -51.4* |
| band_252 | -22.6 [-29 .. -16.4] | -32.2* | -32* | -35.2* |
| band_504 | -27.3 [-32.1 .. -12.6] | -22.9 | -22.9 | -26.5 |
| band_1008 | -25.7 [-31.5 .. -16.5] | -14.4* | -14.1* | -17.3 |
| band_2016 | -25.7 [-28.1 .. -18.6] | -9.09* | -8.98* | -9.94* |
| band_4032 | -24.8 [-31.9 .. -17.2] | -9.59* | -9.85* | -8.96* |
| band_8063 | -26.8 [-30.4 .. -19] | -12.7* | -12.9* | -11.5* |
| centroid_med | 622 [266 .. 1.03e+03] | 3.25e+03* | 3.19e+03* | 3.82e+03* |
| flatness_db | -14.7 [-17.8 .. -11.4] | -5.28* | -5.38* | -4.74* |
| am_0.1-1 | -4.96 [-8.17 .. 1.1] | -2.39 | -4.06 | -3.01 |
| am_1-4 | -7.89 [-13.4 .. -4.21] | -1.82* | -6.35 | -0.906* |
| am_4-16 | -3.84 [-11.5 .. 1.27] | 2.26* | 0.856 | 3.46* |
| am_16-64 | -5.67 [-9.57 .. -2.7] | 0.685* | -1.46* | -4.91 |
| ampeak_hz | 4.6 [2.55 .. 7.63] | 15.1* | 15.1* | 7.83* |
| crest_db | 21.4 [16.7 .. 27.8] | 25.2 | 18.5 | 26.1 |
| spread_db | 29.7 [19.6 .. 34] | 32.2 | 31.7 | 31.4 |
| onset_rate | 3.36 [2.8 .. 6.63] | 5.67 | 9.18* | 5.58 |
| ioi_cv | 0.842 [0.42 .. 1.17] | 2.3* | 2.84* | 2.15* |
| event_ms | 50 [48.8 .. 66.2] | 20* | 20* | 40* |
| tonal_db | 6.52 [5.64 .. 8.36] | 1.5* | 1.32* | 1.75* |
| dom_hz | 104 [102 .. 132] | 2.02e+03* | 1.82e+03* | 2.14e+03* |
| distance | | 1.58 | 1.62 | 1.35 |

### woodpecker_drum (6 references)

| feature | reference | woodpecker |
|---|---|---|
| band_79 | -51.9 [-68.6 .. -33.7] | -30.5* |
| band_252 | -27.1 [-44.9 .. -17.3] | -24.3 |
| band_504 | -17.9 [-23.2 .. -16.4] | -18 |
| band_1008 | -11.5 [-12.6 .. -9.49] | -14* |
| band_2016 | -13.4 [-15.6 .. -10.3] | -20.4* |
| band_4032 | -14.2 [-15.2 .. -12.5] | -29.2* |
| band_8063 | -19.9 [-22.7 .. -15] | -32.6* |
| centroid_med | 1.75e+03 [1.65e+03 .. 2.13e+03] | 830* |
| flatness_db | -13.8 [-14.3 .. -12] | -21.5* |
| am_0.1-1 | -6.17 [-7.65 .. -3.5] | 1.78* |
| am_1-4 | -11.3 [-12.6 .. -9.85] | -8.83* |
| am_4-16 | -13.5 [-14.1 .. -10.8] | -7.53* |
| am_16-64 | -5.75 [-7.54 .. -4.36] | 6.38* |
| ampeak_hz | 17.9 [8.22 .. 19.9] | 18.5 |
| crest_db | 14.8 [12.3 .. 16] | 35.9* |
| spread_db | 15.9 [13.7 .. 17.6] | 0* |
| onset_rate | 2.23 [1.96 .. 2.4] | 0.408* |
| ioi_cv | 1.74 [1.61 .. 2.08] | 4.63* |
| event_ms | 57.5 [40 .. 78.8] | 20* |
| tonal_db | 8.68 [7.15 .. 11.6] | 8.86 |
| dom_hz | 1.09e+03 [941 .. 1.26e+03] | 809* |
| distance | | 2.35 |

`*` = outside the reference interquartile range.
