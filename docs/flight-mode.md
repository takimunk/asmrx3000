# Flight mode

The listener travels forward through an endless, procedurally generated landscape. The
landscape is rigid and the listener moves through it, so the spatial engine produces
everything that makes motion feel real: Doppler shift, parallax, sounds approaching and
receding, occlusion that changes as trunks pass, and a ground effect that changes with
altitude and terrain. With AirPods head tracking, the soundscape stays put in the world
while you turn your head, and streams past you at the flight speed.

Code: `core/src/flight/` (`mod.rs` runtime, `land.rs` landscape, `bridge.rs` UI hand-over,
`airflow.rs` wind noise of motion, `tests.rs`), plus `core/tests/flight_alloc.rs` and
`core/examples/flight.rs`. App: `Stage/FlightStage.swift`, plus small hooks in
`SceneRenderer`, `Stage`, `Popover` (the button) and `AudioEngine`.

## Using it

* **App**: the flight button in the transport row (the `flight` pictogram, next to play)
  toggles the mode, with a 1.5 s cross-fade. The **FLIGHT** group in the tracks has the speed
  macro and its children. The diorama shows the streamed landscape around the herm: trees
  rise from the rim and pass the herm, brooks and lakes cross the disc, the ground relief
  scrolls with the distance flown, and the ground sinks below the herm as it climbs.
* **C ABI** (`asmr.h`): `ASMR_PARAM_FLIGHT` (44; 0/1), controls `ASMR_CONTROL_FLIGHT_*` in
  `ASMR_GROUP_FLIGHT`, telemetry `ASMR_TELEMETRY_FLIGHT*`, object kinds `ASMR_OBJECT_LAKE`
  and `ASMR_OBJECT_ROCK`, and `asmr_get_object_path` for river, lake and ridge shapes.
* **Scenes**: a `flight` block in the scene JSON starts the scene in the air, at cruising
  speed and without a cross-fade:
  ```json
  "flight": { "speed": 12, "biome": "taiga", "seed": 4, "curviness": 0.5, "density": 1.2, "airflow": 1 }
  ```
  `"start": false` only presets the controls.
* **Offline**: `cargo run --release --manifest-path core/Cargo.toml --example flight -- [seconds] [out_dir] [speeds] [biome] [seed]`
  renders one binaural WAV per speed plus an accelerate-and-slow-down tour, and prints CPU,
  RMS and what is around every 10 s.

### Controls (group FLIGHT)

| id | key | range | default | notes |
|---|---|---|---|---|
| 1000 | `flight.speed` (macro) | 0..25 m/s | 6 | walk 1.5, run 5, glide 12, fast flight 25; the actual speed ramps at 2.5 m/s² |
| 1001 | `flight.altitude` | 1.2..40 m (log) | follows speed | ears above the ground. Follows the speed: 1.7 m walking, under the crowns at a run, about 20 m (above the canopy) in fast flight. Gentle swells of up to ±12 % above walking height |
| 1002 | `flight.curviness` | 0..1 | 0.35 | path curvature; 1 = turn radius 250 m. Applies to the path being laid down ~200 m ahead |
| 1003 | `flight.biome` | preset | mixed | mixed, broadleaf, taiga, river_valley, meadows, highlands |
| 1004 | `flight.density` | 0..2 | 1 | trees and stands; takes effect immediately (thins or fills the generated forest) |
| 1005 | `flight.seed` | 0..999 | 0 | another landscape |
| 1006 | `flight.airflow` | 0..3 | 1 | wind noise of motion at the ears (1 = physical, subtle) |

A new biome or seed fades every voice out (0.8 s), regenerates the landscape around the
listener and fades the new one in. The path, and so the direction you are flying, continues.

Weather and time of day (rain, wind, storm, temperature, season, hour) and the scene's
macros (wind speed, foliage, water flow, birds, insects, fire) apply to everything generated,
as in the scene.

## The landscape (`land.rs`)

Everything is generated in the **path frame** (s = distance along the flight path, u =
lateral offset, + left) and mapped once, at generation, into a flat world frame through the
path's centre line. The path is integrated from a smooth curvature κ(s), the sum of three
sines (530, 310 and 170 m), scaled by curviness at the time the path is laid down. Changing
curviness changes the turn rate, never the heading, so the path never kinks.

Content is a **pure function of (seed, biome, chunk or bin index)**. Hashes, not a running
RNG, decide every choice, so the order and timing of generation never change what exists.
The tests stream the same landscape in uneven steps and compare it tree by tree.

* **Chunks** of 32 m along s exist from 70 m behind to 210 m ahead of the listener (the
  propagators carry ~235 m of delay). Each holds:
  * **trees** within |u| < 60 m on a jittered 2.5 m grid, accepted with the local forest
    cover. Cover is two octaves of value noise at the biome's stand scale (140..220 m),
    thresholded to the biome's forest share. Species come from the biome's weights, tilted
    by local groves and by water (willow and alder along banks). Each tree has a crown height
    and radius, a trunk radius and a rank. Trees are generated at twice the biome density,
    and the `density` control keeps those with `rank < density / 2`. A 3.5 m corridor along
    the path stays free of trunks.
  * **aggregate cells**: seven lateral bands (±140 m) with crown area tabulated at eight
    density levels. The inner bands are summed from the trees; the outer ones are
    integrated from the cover field.
  * **birds** in the trees (species by tree: conifers chaffinch, robin, sometimes a tawny owl;
    oaks wood pigeon and song thrush; willow and alder nightingales; cuckoos at edges),
    boulders, and cricket or grasshopper meadows in the open.
* **Features**, in bins along s:
  * **rivers and brooks** cross the path every `river_every` m (170..480 by biome), at up
    to ±37° with meanders. They come as brook over pebbles, rocky brook or slow river, with
    water sites every 10 m along the bed and riffle/pool variation.
  * **lakes**: beside the path, or across it (then you fly over water), with lapping sites
    along the shore.
  * **ridges**: walls 4..12 m high and 60..160 m long beside the path, sometimes with a
    **waterfall**.
  * **campfires**: in the most open spot nearby.

  Features carve the forest: no trees in water, on ridges or around a camp.
* The six **biomes** differ in forest share, stand density and scale, species, and how often
  water, rock, falls and camps appear (`land::BIOMES`).

Storage is fixed-capacity rings (6144 trees, 16 chunks, 128 features, 256 sites, 256 birds),
allocated when flight mode is built. Streaming generates at most one chunk per audio block
(mean 0.1 µs per block, worst 150 µs for a whole chunk) and never allocates.

## Motion

The listener moves along the path at the flight speed, rate-limited to 2.5 m/s². Altitude is
smoothed over 2.5 s and swells gently above walking height. Every emitter is placed
listener-relative each block, and the propagation gets its exact relative velocity:

  v_rel = (−v + ω·y, −ω·x, −climb) + R(−heading)·v_own

where ω = κ(s)·v is the turn rate. So Doppler, parallax and approach/recede come from the
propagation itself (retarded-time delay lines), not from an effect. Turning rotates the
whole field around the listener. Head tracking rotates the head within that frame.

The acoustic environment follows the ground under the listener:
* **ear height** = altitude, so the ground reflection (image source) changes as you climb.
  The `EAR_HEIGHT` clamp was widened from 3 m to 60 m for this.
* **ground type**: forest floor, grass in meadows, water over lakes and rivers.
* **stand density**: trunk scattering and the forest reverb tail.

These are quantised (5 % in height, 50 trunks/ha) so the reverb and ground model are only
rebuilt when they really change.

Every 4 blocks (~5 ms) the trunks within 30 m, plus boulders (spheres) and ridges (walls),
are fed to the propagation with `Acoustics::update_geometry`, a real-time-safe variant of
`set_trees`/`set_obstacles` added for this. It refills the reserved buffers, rebuilds the
diffraction obstacles and the 8 early-reflection trunks, and does not reset the environment.
Trunks and emitters are both rigid in the world, and each propagator refits its spectra when
its own path has moved enough (0.15 m when obstructed, 1 m otherwise), so a full refit is
forced only after a regeneration or every 4 m.

## Level of detail

Voices are fixed pools of pre-built emitters. Generators are reconfigured in place with
`Generator::set_model`, never rebuilt. Each pool voices the best candidates with hysteresis
(a voice keeps its slot while it stays within 60 % of the cut-off priority), and spare slots
let outgoing voices fade (0.35 s) while new ones fade in.

| pool | slots | voiced | what | priority |
|---|---|---|---|---|
| near crowns | 14 | 10 | the nearest trees within 40 m, crown = foliage of its species, area = crown | 1/d |
| stands | 8 | 6 | aggregate cells within 220 m, area = the cell's crowns minus those voiced individually | √area / d |
| birds | 10 | 8 | streamed birds within 150 m; slots have species (2 chaffinch, 2 robin, 2 song thrush, nightingale, wood pigeon, cuckoo, tawny owl), and a bird takes the voice of the slot it gets | song activity / d, flying ×4 |
| water | 8 | 6 | river, lake and waterfall sites within 220 m | weight / d |
| insects | 4 | 3 | meadow patches within 90 m | 1/d |
| fire | 2 | 1 | campfires within 150 m | 1/d |
| rain | 4 | 4 | four patches around the listener; surface follows the ground (under canopy, meadow, on water) | — |

So **at most 38 emitters sound at once, out of 50**. The scene has 43, most of them asleep
(rain and thunder).

Area sources are extended: a crown of radius a heard from d gets the factor
d/√(d² + a²) on top of the propagation's 1/d (intensity ~1/(d² + a²)), so passing through
or next to a crown, a stand or a river segment never blows up like a point source.

Crowns are calibrated by **exposure to the wind**. Inside a closed stand the wind decays into
the canopy (u(z) = u_h·e^{a(z/h − 1)}, a ≈ 2..3; Inoue 1963, Cionco 1965), so a crown in
forest cover c gets exposure 1.1 − 0.45·c, and an aggregate stand 0.7. Lone and edge trees
get the full wind. With this, the near crowns in flight are as loud as the scene's trees
(within 1 dB), and the stands add the distant roar of the forest.

## Birds

Streamed birds perch in their tree's crown and sing by species and hour (the director's song
curves, rain suppression, the BIRDS slider). When the listener comes within 6 + 0.9·v m and
the bird is not far behind, it decides once whether to flush. The chance depends on the
species (wood pigeon 0.85, tawny owl 0.6, song thrush and cuckoo 0.5, chaffinch 0.4, robin
0.25, nightingale 0.2) and grows with speed. A flushed bird takes off with a wing clatter,
flies 25..55 m away from the path (a little forward or back, up then down) calling, and
perches there, quieter for a while (alarm).

## Wind noise of motion (`airflow.rs`)

Flow separation at the helix and the head sheds vortices at f ≈ St·U/d (St ≈ 0.2, d ≈ 1 cm,
so ~20·U Hz), with a broadband turbulent spectrum falling ~12 dB/octave above it. The
aerodynamic dipoles radiate pressure ∝ U³. The model is per-ear noise through a two-pole
low-pass at 20·U Hz plus a little high-band hiss, with turbulent gusts (0.15..0.6 s) and
level

  A10 · x³ / (1 + x³/8), x = U / 10 m/s, A10 = 0.006 (−44 dBFS at 10 m/s)

That is inaudible at a walk, a soft rush at 10 m/s and clearly present (−30 dBFS) at
25 m/s. U includes 40 % of the ambient wind. The ear facing the flow gets more (the flow
comes from azimuth −yaw in the head frame). It is head-locked and added after the binaural
decoder. `flight.airflow` scales it (0 = off).

## Threading and memory

* `Engine::set_param(FLIGHT, 1)` builds the flight pack once, on the calling thread
  (~100 ms: 50 generators including the neural voices, a second `Acoustics` with its own
  propagators, reverb and HRTF decoders, and the first 280 m of landscape). It hands the pack
  over through an `AtomicPtr`; the audio thread takes it at the next block. The pack lives
  until the engine is dropped, so leaving and re-entering flight continues the journey. A
  scene with a `flight` block builds it in `Engine::with_scene`.
* The audio thread cross-fades scene and flight (equal power, 1.5 s). While fully in flight,
  the scene's emitters are not rendered; the director keeps stepping, so the weather is
  shared.
* Objects for the UI go through a seqlock bridge (`bridge.rs`, fixed arrays of atomics):
  * up to 256 objects: features and birds first, then the nearest trees that fit (the radius
    comes from a distance histogram);
  * up to 16 shapes of 24 points (rivers, lakes, ridges).

  Ids (≥ 0x40000000) are stable while an object exists. `asmr_get_objects` switches to the
  streamed objects at cross-fade 0.5.
* `core/tests/flight_alloc.rs` runs 40 s of flight (20 m/s, highlands, rain, birds, a reseed
  and a speed change half way) under a counting global allocator: **zero allocations or
  frees** on the render thread.

## Measurements

Apple Silicon, release build, 48 kHz, 512-frame callbacks, single thread, 60 s each, biome
`mixed`, seed 0 (`--example flight`). CPU = render time / audio time.

| take | CPU % of one core | RMS dBFS | peak |
|---|---|---|---|
| 1.5 m/s | 5.4 | -27.3 | 0.53 |
| 6 m/s | 6.8 | -27.2 | 0.62 |
| 12 m/s | 7.4 | -30.5 | 0.35 |
| 25 m/s | 6.5 | -29.3 | 0.48 |
| tour (walk → 25 m/s → 6 m/s) | 6.6 | -29.8 | 0.45 |
| river_valley, 10 m/s (before the merge with main) | 6.2 | -29.5 | 0.49 |

For comparison, the scene alone (forest by the river, 30 s, `--example render`):
RMS −31.7 dBFS, peak −12.6 dBFS, DSP load 4.1 %. Flight is a few dB louder (it is a whole
forest, not eight trees in a clearing) and costs more because ~40 voices are awake instead
of ~25. The speed-dependent part is the propagators refitting their spectra as their paths
change.

Demo takes are not in git: `--example flight` writes the WAVs to `build/flight/`
(`afconvert -f m4af -d aac -b 256000` makes small AAC copies for listening). The snapshot
`docs/flight/snapshot-flight.jpg` shows the app in flight at 6 m/s, just after a brook crossing
(`ASMRX3000 --snapshot out.png --flight 6 --select none --expand flight.speed`); with
`--flight 8 --seed 2` it flies over a lake.

## Tests

`cargo test --manifest-path core/Cargo.toml flight`:
* `landscape_is_deterministic`: same seed, same trees and features, independent of streaming
  steps; another seed differs; the window stays bounded.
* `landscapes_have_their_features`: every biome has forest, rivers, lakes or ridges, and
  sites within capacity over 5 km.
* `flight_renders_and_stays_bounded`: 25 m/s for 20 s; finite, audible; speed and altitude
  reached; sounding emitters ≤ 50; objects ≤ 256 and streamed.
* `objects_enter_and_leave`: ids appear and disappear; trees move 2.5 m per 0.25 s at 10 m/s,
  towards the listener.
* `audio_is_deterministic`: two engines render bit-identical audio.
* `controls_and_toggle`: altitude follows speed unless set; biome names; scene objects,
  then streamed ones, then scene objects again; shapes readable.
* `new_seed_new_landscape`: a reseed replaces every object.
* `birds_take_off_as_you_pass`: at least 3 flushed birds in 60 s at 12 m/s.
* `airflow_level_follows_speed`: level law, normalisation, windward ear louder.
* `core/tests/flight_alloc.rs`: no allocation in the render path.
* `chunk_generation_cost` (ignored; `-- --ignored --nocapture`): per-block streaming cost.

## Limits and next steps

* The ground is flat for the acoustics (one plane at −altitude). Ridges and boulders
  diffract but do not echo. Hills would need a terrain-aware ground model in the propagation.
* Curviness bends the path laid down at the horizon, so a change is heard after ~200 m of
  flight. Steering by head or keys would need the generator to follow a live heading.
* The diorama shows the 13 m disc around the herm: trees, birds, boulders, brooks and lakes.
  Ridges, waterfalls, camps and meadows are reported but not drawn. The UI session owns the
  look.
* The first switch-on costs ~100 ms on the calling thread (the app calls it on the main
  thread when the button is pressed).
