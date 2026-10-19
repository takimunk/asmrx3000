# Flight mode

The listener travels forward through an endless, procedurally generated landscape. The
landscape is rigid and the listener moves through it, so the spatial engine produces
everything that makes motion feel real: Doppler shift, parallax, sounds approaching and
receding, occlusion that changes as trunks pass, and a ground effect that changes with
altitude and terrain. With AirPods, the flight goes where you look: turn your head and the
course turns that way, while the soundscape stays put in the world around your head.

Code: `core/src/flight/` (`mod.rs` runtime, `land.rs` landscape, `bridge.rs` UI hand-over,
`airflow.rs` wind noise of motion, `tests.rs`), plus `core/tests/flight_alloc.rs` and
`core/examples/flight.rs`. App: `Stage/FlightStage.swift`, plus small hooks in
`SceneRenderer`, `Stage`, `Popover` (the button) and `AudioEngine`.

## Using it

* **App**: the flight button in the transport row (the `flight` pictogram, next to play)
  toggles the mode, with a 1.5 s cross-fade. The **FLIGHT** group in the tracks has the speed
  macro and its children. The view shows the streamed landscape around the herm in the
  travel frame (it always looks along the course): trees grow in at the edge of the haze and
  pass the herm, brooks and lakes cross the ground, the ground relief is anchored to the
  world (it slides and turns under the herm as the course does), and the ground sinks
  below the herm as it climbs.
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
  renders one binaural WAV per speed, an accelerate-and-slow-down tour, and a `steer` take
  at 10 m/s steered by a simulated head (left, right, then turning around), and prints CPU,
  RMS, heading and what is around every 10 s.

### Controls (group FLIGHT)

| id | key | range | default | notes |
|---|---|---|---|---|
| 1000 | `flight.speed` (macro) | 0..25 m/s | 6 | walk 1.5, run 5, glide 12, fast flight 25; the actual speed ramps at 2.5 m/s² |
| 1001 | `flight.altitude` | 1.2..40 m (log) | follows speed | ears above the ground. Follows the speed: 1.7 m walking, under the crowns at a run, about 20 m (above the canopy) in fast flight. Gentle swells of up to ±12 % above walking height |
| 1002 | `flight.curviness` | 0..1 | 0.35 | how much the course wanders when not steered by head; 1 = turn radius 250 m |
| 1003 | `flight.biome` | preset | mixed | mixed, broadleaf, taiga, river_valley, meadows, highlands, desert, snowy_mountains, lake_shore, sea_coast, big_river (docs/world.md) |
| 1004 | `flight.density` | 0..2 | 1 | trees and stands; takes effect immediately (thins or fills the generated forest) |
| 1005 | `flight.seed` | 0..999 | 0 | another landscape |
| 1006 | `flight.airflow` | 0..3 | 1 | wind noise of motion at the ears (1 = physical, subtle) |
| 1007 | `flight.steer_by_head` | preset | yaw | off, yaw (fly where you look), yaw_pitch (also climb and dive with the head); live only while AirPods poses arrive |
| 1008 | `flight.turn_rate` | 5..90 °/s | 30 | fastest turn when steering by head, reached with the head ~43° off course |
| 1009 | `flight.relief` | 0..2 | 1 | relief scale (0 flat .. 2 twice as steep); regenerates |
| 1010 | `flight.stops` | 0..1 | 0.5 | how often and how long the flight eases down and lingers at places (docs/world.md) |

A new biome or seed fades every voice out (0.8 s), regenerates the landscape around the
listener and fades the new one in. Your position and heading carry on.

Weather and time of day (rain, wind, storm, temperature, season, hour) and the scene's
macros (wind speed, foliage, water flow, birds, insects, fire) apply to everything generated,
as in the scene.

## The landscape (`land.rs`)

The landscape is a **pure function of (seed, biome, place)** over the plane. Hashes, not a
running RNG, decide every choice, so the trajectory and the order and timing of generation
never change what stands where. A test flies to the same point straight and by a 2 km loop
from the other side, and compares it tree by tree.

It is streamed **in 2D around the listener**, whichever way it flies:

* **Tiles** (40 m squares) exist within 240 m of the listener (the propagators carry
  ~235 m of delay) and are retired beyond 280 m. Each tile holds:
  * an **aggregate cell**: its crown area, integrated from the cover field and tabulated at
    eight density levels, with the dominant species;
  * **boulders** in rocky country, and a cricket or grasshopper **meadow** in the open.
* Within 90 m (dropped beyond 120 m), a tile also holds its **trees** and the **birds** in them:
  * Trees stand on a jittered 2.5 m grid, accepted with the local forest cover. Cover is
    two octaves of value noise at the biome's stand scale (140..220 m), thresholded to the
    biome's forest share.
  * Species come from the biome's weights, tilted by local groves and by water (willow and
    alder along banks).
  * Each tree has a crown height and radius, a trunk radius and a rank. Trees are generated
    at twice the biome density, and the `density` control keeps those with
    `rank < density / 2`.
  * Birds by tree: conifers chaffinch, robin, sometimes a tawny owl; oaks wood pigeon and song
    thrush; willow and alder nightingales; cuckoos at edges.
* **Macro cells** (360 m) exist within 800 m and hold the big features:
  * **brooks and rivers**: on average one per `river_every` m of travel (170..480 by biome),
    430 m long, any direction, meandering. They come as brook over pebbles, rocky brook or
    slow river, with water sites every 15 m and riffle/pool variation.
  * **lakes** (rotated ellipses) with lapping sites along the shore.
  * **ridges**: rocky crests in the relief, 6..26 m high, 80..180 m long, sometimes with a
    **waterfall** down a face.
  * **campfires**: in the most open spot of a few.

  Every feature that can reach a tile exists before the tile is made. Features carve the
  forest: no trees in water, on ridges or around a camp.
* Eleven **biomes** differ in relief, ground, forest share, stand density and scale,
  species, and how often water, rock, falls and camps appear (`land::BIOMES`). The relief,
  water bodies, surfaces, the sea, the big river and the new biomes are in **docs/world.md**.
* Tiles also carry the **terrain** on a 5 m grid, surf on the shoreline and wind over the most
  exposed crest.

Storage is toroidal: 16 × 16 tile slots with 320 trees and 16 birds each, and 8 × 8 macro
slots with 8 features and 128 sites each, indexed by coordinates modulo the ring. It is
allocated once when flight mode is built, so turning around, circling or flying anywhere
keeps the amount of content bounded and never allocates.

Streaming makes at most one tile and one tree set per audio block, nearest first: mean
≈1–2 µs, worst ≈220 µs per block at 25 m/s (a tile with its terrain, or a macro cell). After a reseed it makes six per block while the voices fade in.

## Motion and steering

The listener moves at the flight speed, rate-limited to 2.5 m/s², along its **heading**,
which integrates a turn rate ω:

* **Steering by head** (`flight.steer_by_head` = yaw, the default; live while AirPods poses
  arrive through `asmr_set_head_pose`). The input is the head yaw relative to the course,
  taken from the same predicted, smoothed head pose the audio is rendered with, so motion and
  sound agree.
  * The yaw is smoothed further (0.35 s), so jitter and quick glances do not wobble the flight.
  * Within ±8° of straight ahead the course holds.
  * Beyond that the turn rate grows linearly and reaches `flight.turn_rate` (30 °/s default)
    with the head ~43° off course.
  * ω is slew-limited to 0.8 rad/s², so turns build up and ease off like a glider's.

  It is rate control: a sustained look keeps turning you; look ahead again to hold the new
  course. With `yaw_pitch`, looking up climbs and looking down dives (up to 0.45·v m/s at
  ~43°), on top of the altitude control. The offset relaxes back over 6 s when pitch steering
  is off.
* **Otherwise** (off, or no head tracking): the course wanders by `flight.curviness` along a
  smooth curvature, three slow sines over the distance flown (530, 310, 170 m; turn radius
  250 m at curviness 1). The head only looks around.

Altitude is smoothed over 2.5 s and swells gently above walking height; the flight level
follows the relief (docs/world.md). With `flight.stops` the flight eases down and lingers at
places worth it. Every emitter is
placed listener-relative (travel frame, +x = where you fly) each block, and the propagation
gets its exact relative velocity:

  v_rel = (−v + ω·y, −ω·x, −climb) + R(−heading)·v_own

So Doppler, parallax, approach/recede, and the whole field swinging round in a turn come from
the propagation itself (retarded-time delay lines), not from an effect. Head tracking rotates
the head within the travel frame, and the predicted pose's neck parallax applies here too.

The acoustic environment follows the ground under the listener:
* **ear height** = altitude above the ground under the listener, so the ground reflection
  (image source) changes as you climb. The `EAR_HEIGHT` clamp was widened from 3 m to 60 m.
* **ground type**: the surface under the listener (forest floor, grass, sand, snow, rock,
  water); every emitter also reflects off the ground at its own specular point.
* **terrain**: diffraction over hills per emitter, slope echoes, a reverb that follows the
  enclosure (docs/world.md).
* **stand density**: trunk scattering and the forest reverb tail.

These are quantised (5 % in height, 50 trunks/ha) so the reverb and ground model are only
rebuilt when they really change.

Every 4 blocks (~5 ms) the trunks within 30 m, plus boulders (spheres),
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
| birds | 14 | 8 | streamed birds within 150 m; slots have species (2 chaffinch, 2 robin, 2 song thrush, nightingale, wood pigeon, cuckoo, tawny owl, blackbird, skylark, gull, raven), and a bird takes the voice of the slot it gets | song activity / d, flying ×4 |
| water | 8 | 6 | river, lake, waterfall, surf and big-river sites within 220 m | weight / d |
| insects | 4 | 3 | meadow patches within 90 m | 1/d |
| fire | 2 | 1 | campfires within 150 m | 1/d |
| rain | 4 | 4 | four patches around the listener; surface follows the ground (under canopy, meadow, on water, sand, rock, snow); snowfall when it snows | — |
| air | 4 | 3 | wind over exposed ground within 160 m: dune sand, ridge howl, blowing snow, breeze on open crests | weight / d |

So **at most 41 emitters sound at once, out of 58**. The scene has 43, most of them asleep
(rain and thunder).

Area sources are extended: a crown of radius a heard from d gets the factor
d/√(d² + a²) on top of the propagation's 1/d (intensity ~1/(d² + a²)), so passing through
or next to a crown, a stand or a river segment never blows up like a point source.

Crowns are calibrated by **exposure to the wind**. Inside a closed stand the wind decays into
the canopy (u(z) = u_h·e^{a(z/h − 1)}, a ≈ 2..3; Inoue 1963, Cionco 1965), so a crown in
forest cover c gets exposure 1.1 − 0.45·c, and an aggregate stand 0.7. Lone and edge trees
get the full wind. With this, the near crowns in flight are as loud as the scene's trees
(within 1 dB), and the stands add the distant roar of the forest.

### The wind is fixed to the world

The wind has one direction for the whole world (`env.wind_direction`, where it blows from,
clockwise from north; the flight world's x is north). It does not turn with the course: the
gust field is sampled at each source's world offset from the listener (the listener-relative
position turned back by the heading, `WindField::at_heading`), so gusts sweep in from the
upwind compass side whatever the heading, and the propagation gets the wind vector turned into
the heading frame (`blow_to_heading`): sounds carried downwind reach further, sounds upwind of
the listener's path fall into the refraction shadow and darken, distant ones wander with the
turbulence (docs/spatial-engine.md). Flying a U-turn swaps which side the gusts arrive from.
Telemetry `WIND_FROM` gives the wind's side relative to the course (rad, +left). The biome's
wind factor scales the speed the propagation sees. Limitation: the listener's own motion
through the frozen gust pattern is not Doppler-shifted (the history is time-indexed around the
listener), so flying upwind does not make gusts pass faster.

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
  propagators, reverb and HRTF decoders, and the landscape within 240 m). It hands the pack
  over through an `AtomicPtr`; the audio thread takes it at the next block. The pack lives
  until the engine is dropped, so leaving and re-entering flight continues the journey. A
  scene with a `flight` block builds it in `Engine::with_scene`.
* The audio thread cross-fades scene and flight (equal power, 1.5 s). While fully in flight,
  the scene's emitters are not rendered; the director keeps stepping, so the weather is
  shared.
* With `Engine::render_bed` (7.1.4 for an external binaural renderer), the flight is
  decoded into the bed too.
* Objects for the UI go through a seqlock bridge (`bridge.rs`, fixed arrays of atomics):
  * up to 256 objects: features and birds first, then the nearest trees that fit (the radius
    comes from a distance histogram);
  * up to 16 shapes of 24 points (rivers, lakes, ridges).

  Ids (≥ 0x40000000) are stable while an object exists. `asmr_get_objects` switches to the
  streamed objects at cross-fade 0.5.
* `core/tests/flight_alloc.rs` runs 40 s of flight (20 m/s, highlands, rain, birds, steering
  by head through two U-turns, a reseed and a speed change half way) under a counting global allocator: **zero allocations or
  frees** on the render thread.

## Measurements

Apple Silicon, release build, 48 kHz, 512-frame callbacks, single thread, 60 s each, biome
`mixed`, seed 0 (`--example flight`). CPU = render time / audio time.

| take | CPU % of one core | RMS dBFS | peak |
|---|---|---|---|
| 1.5 m/s | 7.1 | -26.9 | 0.47 |
| 6 m/s | 8.0 | -25.9 | 0.66 |
| 12 m/s | 8.9 | -27.0 | 0.72 |
| 25 m/s | 7.9 | -27.9 | 0.55 |
| tour (walk → 25 m/s → 6 m/s) | 8.4 | -28.8 | 0.42 |
| steer (10 m/s, head left / right / over the shoulder) | 9.9 | -26.2 | 0.53 |

For comparison, the scene alone (forest by the river, 30 s, `--example render`):
RMS −31.7 dBFS, peak −12.6 dBFS, DSP load 4.1 %. Flight is a few dB louder (it is a whole
forest, not eight trees in a clearing) and costs more because ~40 voices are awake instead
of ~25. Since the 2D world, there is content on every side (more birds, meadows and boulders
within reach), which costs about 1.5 % more than the earlier path-only landscape. Profile:
the propagation and its reverb and HRTF decoding and the foliage and water generators
dominate; streaming, level of detail and publishing are about 8 % of the flight's time. The speed-dependent part is the propagators refitting their spectra as their paths
change.

Demo takes are not in git: `--example flight` writes the WAVs to `build/flight/`
(`afconvert -f m4af -d aac -b 256000` makes small AAC copies for listening). The snapshot
`docs/flight/snapshot-flight.jpg` shows the app at 10 m/s steered by a head held 45° to
the left (`ASMRX3000 --snapshot out.png --flight 10 --steer 45 --select none --expand flight.speed`;
`--steer` feeds an AirPods-like head pose, `--seed n` picks another landscape).

## Tests

`cargo test --manifest-path core/Cargo.toml flight`:
* `landscape_is_a_function_of_place`: the same trees and features at a point reached straight
  or by a loop from the other side; another seed differs; nothing far away survives.
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
* `heading_follows_the_head`: a glance inside the dead zone keeps the course; a sustained
  40° look turns left near the full rate; looking ahead again holds the new course; right
  turns; a faster turn-rate setting turns faster.
* `steering_off_keeps_the_course`: with steering off the head only looks around; with
  `yaw_pitch` looking up climbs.
* `streaming_survives_turning_around`: a full U-turn at 20 m/s by head; objects and voices
  stay bounded and there is landscape ahead.
* `flight_reaches_the_bed_output`: the flight sounds in the 7.1.4 bed path.
* `wind_direction_is_world_fixed_in_flight`: a northerly stays north while the course turns
  by ~1.5 rad; relative to the course it turns by the opposite angle.
* `core/tests/flight_alloc.rs`: no allocation in the render path.
* `chunk_generation_cost` (tiles; ignored; `-- --ignored --nocapture`): per-block streaming cost.

## Limits and next steps

* The relief reaches the acoustics as per-emitter diffraction, a per-emitter ground plane,
  slope echoes and the reverb level (docs/world.md); a tilted ground plane is not modelled.
* There is no trunk avoidance: at walking height a course steered straight into a trunk
  passes through it (the trunk stops being an obstacle within 1 m).
* The relief, rivers, lakes, the sea and the big river are drawn from the engine's own
  heightfield (Stage/FlightTerrain.swift). Waterfalls, camps and meadows are reported but
  not drawn as models.
* Steering by head needs AirPods poses (`asmr_set_head_pose`); without them the course
  wanders by curviness. Keyboard steering would be a small addition (a turn-rate input).
* The first switch-on costs ~100 ms on the calling thread (the app calls it on the main
  thread when the button is pressed).
