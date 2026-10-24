# Yacht at anchor: sound sources (for SOURCES.md)

Files in `assets/sounds/generated/sailboat_anchor/` (not in git). Date: 2026-10-24.

Post-processing (all files): mono (stereo summed), resampled to 48 kHz, high-pass (25–250 Hz by file, below),
for some a low-pass, events trimmed of leading/trailing silence with 10 ms / 60 ms fades, loops crossfaded
into themselves (2–3 s), peak-normalised to −3 dBFS with at most 8× gain, 16-bit. The Freesound files were
taken from their HQ previews (ogg).

## Field recordings (all CC0 1.0, checked on each Freesound page)

| Files | Source | Author | Licence | Cut / processing | Used as |
|---|---|---|---|---|---|
| yacht_cabin_lap_myloop_1 | https://freesound.org/people/myLoop/sounds/851577/ "Ammersee Sailboat Cabin ~ Gentle Lake Waves from Inside" | myLoop | CC0 | 0–134 s, HP 25 Hz, loop | `hull` bed (both sides, under the bilge) |
| yacht_hull_slap_kyles_1 | https://freesound.org/people/kyles/sounds/637206/ "boat knocks fiberglass water laps on hull" | kyles | CC0 | 0.5–41.5 s, HP 35 Hz, loop | `slap` texture |
| yacht_hull_cluck_kyles_1..3 | same | kyles | CC0 | 6.55–7.6, 13.3–14.4, 9.05–9.9 s, HP 35 Hz | `slap` clucks |
| yacht_rope_creak_rmutt_1..3 | https://freesound.org/people/Rmutt/sounds/145721/ "Mooring Rope" | Rmutt | CC0 | 0.7–6.3, 15.2–17.0, 21.4–22.8 s, HP 120, LP 9 kHz | `boat` snubber creaks |
| yacht_halyard_clink_auzet_1..3 | https://freesound.org/people/bruno.auzet/sounds/538010/ "halyards clattering port" | bruno.auzet | CC0 | 23.33–24.55, 25.08–27.1, 27.85–29.6 s, HP 150, LP 6 kHz | `halyard` clinks |
| bay_halyards_auzet_1..2 | same | bruno.auzet | CC0 | 2–132 s and 135–270 s, HP 60 Hz, loops | `harbour` |
| bay_rigging_ceejay_1 | https://freesound.org/people/ceejay/sounds/752726/ "Yacht rigging jingles in the wind" | ceejay | CC0 | 0.5–62.5 s, HP 60 Hz, loop | `gust` far rigging |
| bell_buoy_craigsmith_1..3 | https://freesound.org/people/craigsmith/sounds/675693/ "S29-19 Bell buoy" | craigsmith | CC0 | 0–30, 30–60, 60–90 s, HP 150 Hz | `buoy` |
| yacht_gull_canardo_1 | https://freesound.org/people/Canardo55/sounds/538016/ "Herring Gull 1" | Canardo55 | CC0 | 7.0–22.5 s, HP 250 Hz | `gulls` |
| yacht_gull_canardo_2..3 | https://freesound.org/people/Canardo55/sounds/538015/ "Herring Gull 2" | Canardo55 | CC0 | 0–4.2, 5.6–20.6 s, HP 250 Hz | `gulls` |
| yacht_gull_genghis_1 | https://freesound.org/people/genghisattenborough/sounds/744592/ "Herring Gull passing over roof" | genghisattenborough | CC0 | whole, HP 250 Hz | `gulls` |

## Generated (fal.ai, prompts in `tools/fal/prompts_sailboat_anchor.json`)

14 takes of ElevenLabs SFX v2 (`el2`), 97 s in all, **$0.194** (logged in `tools/fal/spend.log`).
Raw takes were checked against field recordings (below); the ones used:

| Files | From take | Processing | Used as |
|---|---|---|---|
| yacht_halyard_tap_el2_1, _2 | yacht_halyard_tick_el2_2 (2.25–2.57 s), _el2_1 (0.36–0.80 s) | single taps cut out, HP 120 Hz | `halyard` taps down the mast |
| yacht_halyard_rattle_el2_1, _2 | yacht_halyard_rattle_el2_1, _2 | HP 80 Hz | `gust` |
| yacht_chain_grumble_el2_1, _3 | yacht_chain_grumble_el2_1, _3 | HP 30 Hz, LP 4.5 kHz (through the hull) | `chain` |
| yacht_cabin_creak_el2_1..3 | yacht_cabin_creak_el2_1..3 | HP 40 Hz | `boat` joinery creaks |
| yacht_hull_cluck_el2_1 | yacht_hull_cluck_el2_1 | HP 35 Hz | `slap` clucks |

Rejected takes (not shipped): halyard_tick_el2_3 (82 % of its energy above 3 kHz, a hiss, not a tap),
chain_grumble_el2_2 (57 % above 3 kHz, a bright rattle), hull_cluck_el2_2/_3 (no energy under 300 Hz, a
plop, not a hull cluck), and the low thumps in the tick takes (72 % under 300 Hz).

## Checked against field recordings (references in a scratch folder, not in the repo)

Descriptors: energy split <300 / 300–3k / >3k Hz, median energy frequency, onsets per second, decay to −20 dB.

- **Hull clucks** vs 851577 (cabin at anchor: 72 % < 300 Hz, median 151 Hz) and 637206 (hull laps: 57 %,
  258 Hz): kept cluck_el2_1 (49 %, 325 Hz); _2/_3 at 7–8 % < 300 Hz and 700–1000 Hz were rejected. The real
  637206 knocks are the main cluck source.
- **Halyard**: in the calm-port recording 538010 a single boat's halyard taps about once every 1–5 s,
  clustered, each ringing ~300 ms; the generated takes tick in a regular 0.21 s train (4.8 per second).
  The takes were cut into single taps and the scene spaces them 20–60 s apart (house rule for calm, below
  the field rate), with real clinks from 538010 for the outside of the mast.
- **Anchor chain** vs 347837 (anchor chain, CC0, outdoors close): link clatter at ~10 onsets/s with strong
  4–16 kHz content when it runs. Generated takes match the onset rate (8–10/s); their high end is 10–20 dB
  lower, consistent with hearing it through the hull from the cabin, and they were low-passed at 4.5 kHz.
- **Creaks** vs 145721 (mooring rope) and 843241 (small boat creaking, Venice): stick-slip micro-onsets
  12.5/s in the rope, 12.4–13/s in the generated creaks; the generated joinery is lower (median 320 Hz vs
  0.9–1.2 kHz for rope and hull squeaks), kept for the cabin's wood, with the real rope for the snubber.
- **Scene statistics** vs 851577: see `docs/scenes/sailboat_anchor.md` (L10 − L50, event rate and bed
  share of a 15-minute render match the cabin recording within 1 dB / 1 event per minute / 1 %).

Not used: 760436 (sailboat cabin, but under way at 5 kn with a radio call), 252670 (sailing interior),
360633 (marina in a strong wind; the procedural wind covers it), 818829 (gulls with strong wind).
