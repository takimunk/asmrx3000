# Spatial audio engine

How asmr-core turns a mono source signal into what two ears hear in a forest: the survey that
set the target, what is implemented and why, how each model is validated, and what it costs.
Code: `core/src/{propagation,acoustics,shaping,delay,nearfield,reverb,rotation,hrtf,fftconv}.rs`
and `core/src/physics/`.

## 1. Survey: what the reference engines do

Sources: public documentation and papers as of 2025. Product internals (Apple, Dolby, Meta) are
not public; the table lists documented behaviour only.

| Feature | Steam Audio (Valve, Apache-2.0) | Meta XR Audio SDK | Resonance Audio (Google, Apache-2.0) | Apple PHASE | Dolby Atmos binaural | **asmr-core (this)** |
|---|---|---|---|---|---|---|
| HRTF | default set + custom SOFA | Meta universal HRTF | SADIE KU100 in SH domain | system HRTF, personalised (TrueDepth scan) | proprietary, personalised option | SADIE II KU100 (measured), Brown–Duda analytic fallback |
| Ambisonics / decoder | up to order 3, virtual-speaker HRTF decode | order 1–3 playback | order 3 (SH-domain HRIRs) | n/a (object renderer) | n/a (object renderer) | order 3, **MagLS + diffuse-field covariance constraint**, partitioned FFT |
| Head rotation | per source | per source | SH-domain rotation | system | system | **exact SH-domain rotation of the whole bus** (direct + reflections + tail) |
| Near field (< 1 m) | no | near-field HRTF rendering | "near-field effect" ILD boost | not documented | distance modes (near/mid/far room) | **NFC-HOA range extrapolation** of the measured HRTF (physically exact for orders ≤ 3) |
| Distance / air | curves; 3-band air absorption | curves | log/linear rolloff | distance models | n/a | 1/r + **ISO 9613-1** (T, RH, p) as a continuous filter |
| Doppler | left to host engine | host | host | supported | n/a | **retarded-time fractional delay**: exact moving-source/moving-listener Doppler |
| Ground reflection | no | no | no | no | no | **image source + spherical-wave reflection coefficient Q** (Miki impedance: forest floor, grass, water, snow…) |
| Directivity | dipole weight/power | yes | alpha/sharpness pattern | cardioid / cone | n/a | point source on a rigid sphere (frequency-dependent, energy-normalised) |
| Occlusion / diffraction | ray / volumetric occlusion, 3-band transmission, baked pathing | geometry-based | low-pass occlusion | occluders with materials | n/a | **Fresnel–Kirchhoff (Babinet) diffraction** per obstacle, detour delay in shadow |
| Reflections | real-time or baked ray tracing → Ambisonic IRs | acoustic ray tracing (early reflections) | shoebox image sources | geometry-aware early reflections | room model | **geometric-optics trunk reflections** (convex cylinders), per trunk direction |
| Late reverb | convolution or parametric | ray traced / shared reverb | spectral reverb from room materials | presets / geometry | room model | **FDN whose T60, density and level come from forest geometry** (energy balance) |
| Outdoor propagation | no | no | no | no | no | forest **excess attenuation** (exact cylinder scattering, transport width) + ISO 9613-2 foliage |
| Wave-based low frequencies | no (Project Acoustics does: baked wave simulation) | no | no | no | no | later (see §4) |
| Cost model | per source + IR convolution, GPU option | per source | per source O(1) + fixed SH decode | system | system | per emitter O(1) + fixed SH decode; all spectral effects in one filter per path |

Research reviewed:
- Wave-based baking: Microsoft Project Acoustics/Triton (Raghuvanshi & Snyder 2014, 2018: parametric
  coding of baked wave simulations; ARD, Raghuvanshi et al. 2009). Best for indoor/occlusion at
  low frequencies; needs offline baking per scene and a GPU solver.
- Geometric acoustics: GSound and follow-ups (Schissler & Manocha), UTD edge diffraction
  (Tsingos et al. 2001), BTM. Best for many reflections in built geometry.
- Outdoor auralization (aircraft, road traffic): Arntzen 2014; Rietdijk, Heutschi et al. 2017:
  Doppler via delay lines, ISO 9613-1, ground reflection via FIR from Q, turbulence. That's the closest
  prior art to our direct-path model.
- Ambisonic binaural: MagLS (Schörkhuber, Zaunschirm, Höldrich 2018), covariance constraint
  (Zaunschirm et al., JASA 2018), BiMagLS for head tracking (Lübeck et al. 2022); NFC-HOA
  (Daniel 2003); HRTF range extrapolation (Duraiswami et al. 2004, Pollow et al. 2012).
- Differentiable / neural acoustics: differentiable FDNs (Dal Santo et al. 2023), DiffRIR
  ("Hearing Anything Anywhere", Wang et al. 2024), neural acoustic fields (Luo et al. 2022).
  Useful later to fit our physical parameters to real forest recordings, not as the renderer.
- Forest acoustics: Embleton 1966; Price, Attenborough & Heap 1988; Swearingen & White 2007
  (multiple scattering by trunks); Tarrero et al. 2008 (forest reverberation).

Takeaway: the game engines are room-centric and leave the outdoor physics (Doppler, ground,
atmosphere, scattering media) to the host or ignore it. For a forest by a river, those are
what you actually hear. That's where asmr-core goes beyond them. For the binaural stage the
state of the art is MagLS-style SH decoding of a measured HRTF with SH-domain head rotation,
which we implement.

## 2. What is implemented (and why)

### Pipeline per block (64 samples)

```
emitter mono ─► delay line ─┬─ read at τ1 (retarded) ─► [path filter 1] ─► NFC (orders 1..3) ─┐
                            ├─ read at τ2 (image src) ─► [path filter 2] ───────────────────────┤ encode (world frame)
                            ├─ reads at trunk delays ─► trunk buses (8) ─► HP/LP ─► encode ──────┤
                            └─ direct read × send ───► FDN 16 lines ─► encode at 16 directions ──┤
                                                                                                ▼
                                                     Ambisonic bus (order 3, world) ─► SH rotation ─► MagLS decoder ─► L/R
```

### Direct path
- **Delay and Doppler.** τ solves |d − v τ| = c τ (retarded time with constant velocity), so
  a source approaching at v is heard at f·c/(c−v), not the moving-listener f·(1+v/c). Read
  with an 8-tap Kaiser-windowed sinc (256 phases, linear phase interpolation, 32.32 fixed-point
  positions): flat to ~18 kHz, no linear-interpolation modulation noise. On teleports the old
  and new positions are crossfaded instead of sweeping the delay.
- **Spreading.** 1/r (clamped at 0.15 m).
- **Air absorption.** ISO 9613-1 α(f; T, RH, p), exact formula at 40 log-spaced frequencies.
  Speed of sound c = 343.2·√(T/293.15).
- **Directivity.** Point source on a rigid sphere (Brown–Duda head-shadow form used
  reciprocally), normalised by the sphere-averaged power so it redistributes energy without
  adding any.
- **Obstacles.** Each obstacle is projected onto the plane perpendicular to the path as a
  rectangle; U/U0 = 1 − Fx·Fz with Fresnel integrals (Fresnel–Kirchhoff + Babinet). One formula covers:
  - the lit zone;
  - the shadow boundary (−6 dB);
  - the deep shadow (within 1 dB of the ITU-R P.526 knife edge);
  - low-frequency transparency of thin trunks.

  Power is averaged over a third of an octave (smooths two-edge Arago interference). In the shadow the
  shortest edge detour is added to the delay (continuous at the boundary). Trunks within
  15 m of the listener are explicit obstacles; boulders (spheres) and ridges/logs (walls)
  are supported.
- **Forest beyond 15 m.** Statistical excess attenuation exp(−n σ_tr r) from the exact
  partial-wave solution for a rigid cylinder. σ_tr is the transport width: the forward-diffracted
  half of the extinction keeps travelling with the direct sound. It is averaged over trunk radii
  of 8–28 cm. Canopy foliage uses ISO 9613-2 Annex A (dB/m per octave) for the part of the path
  inside the crown layer.
- **Near field.** For r < R = 1.2 m (the HRTF measurement radius), order m is weighted by
  h_m(kr)/h_m(kR) (Bessel-polynomial IIR sections, bilinear). This is exact HRTF range
  extrapolation, valid down to 0.2 m. It reproduces the low-frequency ILD growth of close
  sources: 9.9 dB at 0.3 m vs 4.7 dB at 2 m, 300–900 Hz, KU100; KEMAR measurements (Brungart &
  Rabinowitz 1999) show the same trend.

### Ground-reflected path
Image source mirrored in the ground plane, with its own retarded delay, direction (from
below) and directivity angle. Its filter includes the spherical-wave reflection coefficient
Q = Rp + (1 − Rp) F(w): locally reacting ground, Miki (1990) impedance from the effective flow
resistivity, boundary-loss factor via the Faddeeva function (Weideman's rational
approximation). The ground-effect dip is not modelled explicitly; it emerges from the
interference of the two paths, as it does physically. Emitters can override the ground type
(the river segments reflect off water).

### One filter per path
All spectral effects of a path are multiplied into one complex target response on a
log-frequency grid and fitted by a **fixed-pole parallel filter** (Bank 2007/2011): 12 complex
poles, log-spaced from 30 Hz to 18 kHz. Magnitude-only effects get their minimum phase from a
precomputed discrete Hilbert operator; Q enters with its own phase. Because the poles are
fixed:
- the fit is one precomputed least-squares matrix–vector product, with 12 weighting profiles
  so steep stop bands keep relative accuracy;
- every coefficient set is stable;
- interpolating coefficients interpolates the response exactly. There is no zipper noise and
  no transient instability, however fast the source moves.

Fits run only when the predicted change exceeds about 0.4 dB (distance vs air absorption,
grazing angle, image distance, Fresnel-zone scale near obstacles, directivity angle). They
are staggered across emitters and ramped over two blocks. The filters of all emitters run
batched, 4 paths per SIMD group. This hides the IIR recursion latency (NEON intrinsics on
aarch64, portable fallback elsewhere).

### Forest reverberation
- **Early reflections:** the 8 trunks nearest the listener reflect each emitter specularly.
  The geometric-optics amplitude for a convex cylinder is A = √(a cos θi / 2) / √(r1 r2 (r1+r2)),
  with Doppler from the moving delays. A trunk's direction from the listener doesn't depend
  on the emitter, so each trunk is one mono bus, high-passed at ka = 1 (Rayleigh regime) with
  air absorption, and encoded once. Per emitter, the 4 strongest taps within 20 dB of the
  direct sound are rendered.
- **Late tail:** the forest is a slab between an absorbing floor (random-incidence absorption
  of the same ground impedance) and a leaky canopy, filled with scatterers:
  - decay rate G(f) = c[(α_floor + e_canopy)/(4H) + n σ_s (π/4) α_bark + 2 m_air], plus
    horizontal diffusion 1/(1 + t/t0);
  - T60 at 500 Hz and 4 kHz sets the per-line absorption of a 16-line Hadamard FDN (Jot,
    matched exactly at both frequencies);
  - line lengths scale with the scattering mean free path;
  - the level follows from energy balance: the critical distance r_c² = G L H / (4 n σ_tr c);
  - line outputs are encoded at 16 directions squashed towards the horizon (open sky above,
    absorbing floor below).

  Default forest (400 trunks/ha, forest floor): T60 ≈ 0.67 / 0.66 / 0.63 s at 0.5 / 1 / 4 kHz,
  scattering mean free path 55 m.

### Binaural stage
- **HRTF:** SADIE II KU100 (Apache-2.0), 598 directions, 256 taps, 16-bit, 620 KB, embedded
  in the library. Converter: `tools/hrtf/sofa_to_ahrt.py` (uv script, SOFA → "AHRT" v1, see
  `assets/hrtf/README.md`). The set is diffuse-field equalised, which is the only version SADIE
  publishes. That suits headphone playback.
- **Decoder:** MagLS:
  - LS below f_c = N c/(2π r_head) ≈ 1.9 kHz;
  - magnitude-only above, with phase carried over from the previous bin;
  - 2×2 diffuse-field covariance constraint, which matters for the reverberant field.

  It runs as uniformly partitioned FFT convolution (block 64, FFT 128), adding a fixed latency of
  64 samples.
- **Fallback:** Brown–Duda analytic HRIRs, decoded with the same MagLS code. M0's mode-matching
  + max-rE decoder lost 7–13 dB above 3 kHz at order 3; that is gone.
- **Head rotation:** exact SH-domain rotation of the whole bus (projection
  D = pinv(Y)·Y(R·d) over 36 directions, equivalent to Ivanic–Ruedenberg but convention-proof),
  interpolated per sample. Reflections and the diffuse tail turn with the head too.

### Not implemented yet (later), with reasons
| Item | Why later |
|---|---|
| Atmospheric turbulence (amplitude/phase fluctuation of distant sources) | Needs a wind-field-coupled stochastic model; small effect at < 100 m. Next on the outdoor list. |
| Refraction (wind/temperature gradients) | Matters beyond ~200 m, which our scene doesn't reach. |
| Direct HRIR rendering of the closest source(s), bypassing the bus | Above ~2 kHz order 3 can't resolve near-field parallax. MagLS keeps timbre, and NFC gets the low-frequency ILD. |
| BiMagLS / head-tracking-aware MagLS, HRTF personalisation | Decoder upgrade; needs listening tests. |
| Per-region ground types, heightfield terrain | Needs the scene graph (director session). The API already takes per-emitter ground and arbitrary obstacles. |
| Indoor: image sources/ray tracing against meshes, portals, wave-based low frequencies (FDTD/ARD bake) | Prompt-generated indoor scenes (M5). The fixed-pole filter, bus and rotation carry over unchanged. |
| Fitting physical parameters to recordings (differentiable FDN / DiffRIR-style) | Once we have field recordings of the target forest. |

## 3. Validation (unit tests, `cargo test`)

| Model | Test | Result |
|---|---|---|
| ISO 9613-1 air absorption | all 48 values of ISO 9613-2 Table 2 (6 climates × 8 octaves) | within 0.06 dB/km or 0.6 % |
| … end to end | source at 100 m, 125 Hz–12 kHz | within 0.4 dB of 1/r + ISO |
| Doppler | source approaching at 20 m/s, 1 kHz | heard 1062.46 Hz, theory c/(c−v) 1062.45 Hz (moving-listener formula would give 1058.78) |
| Ground effect, rigid | hs = hr = 1.5 m, d = 10 m | dip at 386.5 Hz, theory c/(2Δr) = 386.4 Hz, depth −33 dB |
| Ground effect, grass | hs 0.5, hr 1.5, d 20 m, vs \|1 + Q r1/r2 e^{−ikΔr}\|/r1 | within 0.7 dB, 100 Hz–4 kHz |
| Ground model | grassland 1 m/1.5 m/50 m first minimum | 200–900 Hz (known grass dip), < half the rigid value |
| Faddeeva function | w(0), w(i), w(1), Taylor series in both half planes | ≤ 1e-5 |
| Bessel J, Y | tabulated values | ≤ 1e-7 |
| Cylinder scattering | Rayleigh limit 3π²/4 k³a⁴; geometric limits 4a, 8a/3 | within 2 %, 5 %, 10 % |
| Diffraction | half-plane at the shadow boundary; ITU-R P.526 knife edge | −6.0 dB; within 1 dB |
| Trunk occlusion | 25 cm trunk 1 m in front, source at 10 m | 100 Hz −1.7 dB, 6 kHz −11.0 dB |
| Parallel filter fit | air absorption over 200 m, down to −80 dB at 16 kHz; complex Q | within 0.6 dB; complex error < 0.06 |
| Filter bank | batched lane vs single filter, across a coefficient ramp | identical to 1e-4 |
| Near field | NFC sections vs exact spherical Hankel ratios; free-field range extrapolation | exact; ILD within 0.4 dB of the 1/r-parallax value |
| Near field, measured HRTF | ILD 300–900 Hz, lateral source | 0.3 m 9.9 dB vs 2 m 4.7 dB |
| Head rotation | SH rotation vs rotated encoding; 90° turn moves a frontal source to the right ear | 1e-4; ILD < −8 dB |
| Reverb | FDN T60 (Schroeder, T20) | 1.196 s for a 1.2 s target |
| Decoders | measured vs analytic loudness | within 3 dB |
| Partitioned convolution | vs direct convolution, irregular block sizes | 1e-3 |

## 4. CPU budget

`cargo run --release --example bench_spatial -- 64`: thread CPU time / audio time on one M5 Max
performance core (user-interactive QoS, like the CoreAudio thread), 48 kHz, 64-sample blocks,
best of three, pre-generated noise as input (source generators excluded). "Moving": half the
emitters circle at 3–15 m/s (Doppler, refits, changing occlusion). Measured on 2026-10-18
while the machine ran other heavy jobs (Blender, ML training; load average 5–12), which
inflates and scatters the numbers; the range over 6 runs is given.

| Configuration | CPU, one core |
|---|---|
| 64 emitters, full pipeline, half moving | 6.0–9.6 % |
| 64 emitters, full pipeline, all static | 4.5–7.5 % |
| 64 emitters, direct path only (no ground, reverb, ER, occlusion, forest, NFC) | 3.6–6.8 % |
| fixed cost: forest reverb + SH rotation + MagLS decoder | 1.3–2.1 % |

Budget: ≤ 10 % of one core at 64 emitters. Met in every run. The engine itself (13 sources
including generators, `examples/render`) runs at ~15× real time.

How it gets there:
- one fused filter per path;
- path filters batched 4 per SIMD group (4 independent accumulators: one chain would
  serialise 24 FMAs per sample);
- vectorised fixed-FIR reads when a delay is constant;
- refits only on a predicted change above ~0.4 dB;
- early reflections limited to each emitter's 4 strongest taps within 20 dB of the direct sound;
- partitioned FFT decoder.

## 5. A/B against M0 (`examples/render`, 30 s, same seeds)

| Render | RMS | Octave levels 63 Hz … 16 kHz (dB) |
|---|---|---|
| M0, static head | −25.4 dBFS | −53.1 −50.0 −44.3 −26.9 −31.9 −39.7 −47.6 −63.9 −71.0 |
| new, static head | −21.4 dBFS | −46.4 −43.4 −38.3 −22.8 −30.6 −32.7 −38.1 −49.4 −62.9 |
| M0, head turning | −24.9 dBFS | −53.1 −50.0 −44.2 −26.6 −31.2 −37.9 −44.5 −61.6 −71.5 |
| new, head turning | −21.0 dBFS | −47.0 −44.0 −38.2 −22.4 −29.8 −32.0 −39.1 −49.5 −63.3 |

- 2–16 kHz is 7–14 dB brighter. M0's mode-matching decoder rolled off the highs; MagLS
  restores the HRTF magnitude.
- 63–250 Hz is ~6.5 dB fuller: the coherent ground reflection (rigid under the river) doubles
  low-frequency pressure for the low sources.
- Distant HF is lower than in a free field: air absorption, forest scattering and foliage.
- Overall +4 dB; peaks reach −2.5 dBFS at volume 0.8, so the soft clipper (knee −3 dBFS) can
  touch occasionally. Shells may want ~−3 dB of default volume.

## 6. AirPods: head tracking and spatial impression

### 6.1 Audit of the M0/M1 shell (before this change)

| Item | Finding |
|---|---|
| Sensor | `CMHeadphoneMotionManager` delivering to the **main** queue: the pose waited behind UI work (the ASCII console redraws at 30–60 fps). |
| Update rate | Apple doesn't specify one. Measured by others: **~25 Hz** for AirPods ("fixed by the OS"). CoreMotion's general 100 Hz device-motion limit doesn't apply to headphones. |
| Representation | Euler angles `attitude.yaw/pitch/roll`, each multiplied by a sign "guess" and sent as three separate params. Euler round-trips are fine near upright but couple axes at large pitch. |
| Axes | Apple documents the headphone axes only in a picture. Under the reading x → right ear, y → forward, z → up, `CMAttitude`'s Euler angles mean what their names say (pitch + = look up, roll + = right ear down, yaw + = turn left). Under the other plausible reading (x → left, y → back) **pitch and roll flip, yaw does not**. Open-source trackers disagree (AirTracker ships with pitch inverted). Can't be settled without a device: see the listening check 6.5-1. |
| Recentre | Yaw only: correct, gravity must stay absolute. |
| Drift | Not handled. AirPods have no magnetometer, so yaw drifts a few degrees per minute. |
| Latency | Not compensated. The engine rendered the last pose as is, in 25 Hz steps (40 ms staircase). |
| Rendering | Per-block SH rotation from Euler params, no smoothing: audible stepping on fast turns. |

**Latency chain** (head moves → sound at the ear), typical, macOS + AirPods Pro over AAC:

| Stage | Delay |
|---|---|
| IMU fusion in the buds + Bluetooth uplink + CoreMotion | ~15–40 ms (sample age, measurable from `CMLogItem.timestamp`) |
| Sampling at 25 Hz (mean wait) | ~20 ms |
| Main-queue dispatch (removed) | 0–16 ms |
| Next render block, then IO buffer (512 frames at 48 kHz) | ~11–22 ms |
| CoreAudio device latency + safety offset | a few ms |
| AAC encode, Bluetooth A2DP, decode and buffer in the buds | ~100–150 ms (macOS under-reports it in `presentationLatency`) |
| **Total** | **~150–250 ms**. Psychoacoustic thresholds for head-tracked audio are ~30–80 ms (Brungart, Simpson & Kordik 2005; Lindau 2009), so the lag matters a lot. |

### 6.2 What Apple offers third parties (macOS 14 … 26)

| API | Availability | What it gives / limits |
|---|---|---|
| `CMHeadphoneMotionManager` | macOS 14+ | Attitude (quaternion), rotation rate, gravity, user acceleration, connection status (macOS 14). Requires `NSMotionUsageDescription`. **The only head-pose source; no lower-latency API exists.** |
| `AUSpatialMixer` (AudioToolbox) | macOS 12.3+ head tracking, 13+ personalised HRTF, 14+ status query | `kAudioUnitProperty_SpatialMixerEnableHeadTracking`, `…PersonalizedHRTFMode` (off/on/**auto** = follow the system setting), `…AnyInputIsUsingPersonalizedHRTF` (read back). Inputs can be point sources or an **ambience bed** (e.g. 7.1.4). **The only way to use the listener's personalised HRTF**: Apple never exposes the HRTF data itself. |
| Entitlement `com.apple.developer.spatial-audio.profile-access` | macOS 15+ | Applies the personalised profile to AVAudioEngine / AUSpatialMixer / PHASE output. A capability, so it needs a properly signed build (our ad-hoc `codesign -` build can't carry it). Without it the mixer may fall back to the generic HRTF; we read back `…AnyInputIsUsingPersonalizedHRTF` and show it. |
| `AVAudioEnvironmentNode.isListenerHeadTrackingEnabled` | macOS 15+, entitlement `com.apple.developer.coremotion.head-pose` | Apple's head tracking applied inside its own 3D mixer. Spatialises mono inputs only. |
| `AVGameBypassSystemSpatialAudio` | iOS/tvOS 18 only | Opt out of automatic game spatialisation. No macOS equivalent. |
| `HeadTrackedSpatialAudio`, `intendedSpatialExperience` | visionOS 26 only | not on macOS |
| PHASE | macOS 12+ | Geometry-aware engine with the same personalised-HRTF entitlement. We don't use it: our acoustics are our own. |
| System "Spatialize Stereo" (Control Center: Off / Fixed / Head Tracked) | user setting | **No public API to read or disable it.** It virtualises plain stereo from apps such as Music and video players. Cog's developers report that macOS doesn't spatialise an AUHAL client's own output; AVAudioEngine output is an AUHAL client. Verify with check 6.5-4 that toggling it changes nothing for us. |
| Output format with AirPods | — | 48 kHz stereo (AAC A2DP) normally. **If any app opens the AirPods microphone, the route drops to HFP** (16/24 kHz, narrowband), and the engine is rebuilt for the new rate. |

### 6.3 What changed

1. **Pose path** (`HeadTracker.swift`): full attitude quaternion + gyroscope rate on a dedicated
   user-interactive queue, passed straight to the engine (`asmr_set_head_pose`, lock-free seqlock).
   Additional details:
   - The device→head axis mapping is one explicit rotation, switchable between the two readings
     (`UserDefaults headAxes = rightForwardUp | leftBackUp`).
   - Recentring is about the vertical only.
   - Slow drift correction: 0.5°/s, only while the head is still and within ±20° of forward
     (`headDriftCorrection`).
   - The sample age comes from the sensor timestamp.
2. **Prediction** (`core/src/headpose.rs`): the pose is extrapolated to the moment each 64-sample
   block is *heard*. The horizon is h = sensor age + time since arrival (audio clock) + output
   latency + smoothing lag. The model is constant angular velocity plus half the estimated
   angular acceleration, with the angle linear up to 25° and saturating at 45°.
   `AudioEngine` measures the output latency: device presentation latency + IO buffer + decoder
   block + mixer latency, at least 160 ms on Bluetooth because macOS under-reports AAC; override
   with `headLatencyMs`. `ASMR_PARAM_HEAD_PREDICTION` scales the horizon for A/B.
3. **Smoothing:** 12 ms slerp per block towards the predicted pose, then per-sample interpolation
   of the SH rotation matrix. No 40 ms staircase, and the prediction error at each new sample is
   absorbed in ~12 ms.
4. **Head translation / motion parallax:** the head turns about the neck pivot (head centre 3 cm
   in front of and 9 cm above it). The ears translate by a few cm, so close sources (the stream at
   your feet, a bird at 1 m) shift against far ones, as in reality. This cue is absent from
   rotation-only trackers, Apple's included. It feeds the propagation directly: delays,
   directions, near-field filters.
5. **Apple renderer option** (`AudioEngine.renderer = .apple | .appleTracked`):
   - The core decodes its bus to a 7.1.4 bed (AllRAD-style, `core/src/bed.rs`,
     `asmr_render_bed`), and AUSpatialMixer renders it with `PersonalizedHRTFMode = auto`.
   - `.apple` keeps our predicted head rotation and feeds a head-locked bed.
   - `.appleTracked` lets Apple track the head and feeds a world-locked bed.
   - The UI can show `personalizedHRTF` (read back from the mixer) and `outputDescription`.
   - Default stays `.core`: MagLS at order 3 with SADIE II is sharper than a 12-channel bed. The
     personalised option is for listeners whose ears the KU100 doesn't fit.
6. **Externalisation cues already in the engine**, and how they now interact with head motion:
   - early trunk reflections, ground reflection and the diffuse tail are encoded in the world
     frame and rotate with the head (SH rotation), so they stay put while you turn;
   - near-field ILD below 1.2 m;
   - air absorption and DRR as distance cues;
   - interaural coherence of the diffuse field matched by the MagLS covariance constraint.

### 6.4 Measurements

Tracking error = angle between the rendered pose and the true head pose at the moment the sound
is heard (simulated 25 Hz sensor, 30 ms sample age, gyro noise 0.03 rad/s;
`cargo test --release prediction_error_table -- --nocapture`):

| Motion | Latency | No prediction (rms / max) | Prediction (rms / max) |
|---|---|---|---|
| natural sway (±50° + ±15° yaw, ±10° nod) | 100 ms | 11.5° / 22.6° | 1.9° / 4.2° |
| | 160 ms | 15.6° / 30.2° | 3.6° / 7.7° |
| | 250 ms | 21.4° / 41.0° | 6.9° / 14.3° |
| 90° turn in 300 ms, then hold | 160 ms | 24.1° / 86.7° | 18.8° / 81.6° |

- Prediction can't anticipate the *start* of a sudden turn; that is the floor set by the
  Bluetooth latency. It catches up fast and doesn't overshoot by more than the 45° cap at a stop.
- Smoothness: with the raw 25 Hz pose stepping 0.04 rad, the largest per-block step of the
  rendered pose is < 0.006 rad (test `rendered_pose_is_smooth`).

Offline renders (`cargo run --release --example headtrack_ab`, files in `build/ab/`):
- `head_lagged.wav` vs `head_predicted.wav`: the engine is fed only the tracking error during the
  natural-sway motion, which is what a moving listener hears. Lagged: the forest swings ±30°
  (rms 15.3°). Predicted: it nearly stands still (rms 3.4°, max 6.5°).
- `ext_anechoic.wav` vs `ext_full.wav` (static head): direct + ground only vs the full forest.
  Interaural cross-correlation (IACC, 100 ms frames) at 250/500/1k/2k/4k Hz:
  0.79/0.52/0.42/0.31/0.30 vs 0.75/0.46/0.32/0.22/0.21. The full forest is more decorrelated, a
  standard correlate of externalisation and spaciousness.

CPU: head pose ≈ 0 (one prediction + slerp per block); 64 emitters still 3.9–5.1 % of one core.

### 6.5 Listening checks with AirPods (Pro / Max / 3rd gen)

1. **Axes.** Recentre facing the screen, then:
   - Turn your head left: the river (in front) must move to your **right**.
   - Look up: sounds in front move **down**.
   - Tilt your right ear down: a front source should lean **towards your left ear**.

   If up/down or tilt are inverted (but left/right is right), run
   `defaults write com.asmrx3000.app headAxes leftBackUp` and relaunch.
2. **Lag.** Turn quickly to the left by ~45° and stop. The scene should land and stay. If it
   keeps swinging after you stop (overshoot), lower the latency estimate:
   `defaults write com.asmrx3000.app headLatencyMs 120`. If it trails behind, raise it to 200.
   `defaults delete com.asmrx3000.app headLatencyMs` returns to the measured value.
3. **Prediction A/B.** Compare HEAD_PREDICTION 1 vs 0 while looking around slowly. Until the UI
   gets a toggle: `defaults write com.asmrx3000.app headPrediction -float 0` (1 = on), relaunch. With prediction the forest should feel nailed to the room.
4. **System spatial audio.** Control Center → Sound → AirPods → Spatialize Stereo: Off / Fixed /
   Head Tracked. Our output must not change. If it changes, macOS is double-spatialising: keep it
   Off and report.
5. **Parallax.** With HEAD_PARALLAX on (`defaults write com.asmrx3000.app headParallax -float 0` turns it off), turn your head while a close source plays (orbit bird at
   0.5 m, or the stream). The close source should shift relative to the distant forest. Compare
   with HEAD_PARALLAX = 0.
6. **Personalised HRTF.** If you scanned your ears (iPhone: Settings → AirPods → Personalised
   Spatial Audio), switch `renderer` to `apple`:
   - `defaults write com.asmrx3000.app renderer apple` and relaunch; the console log line
     "personalised HRTF: yes/no" tells whether the mixer uses it;
   - compare front/back and elevation (birds above) with `core`;
   - also try `renderer appleTracked`; `defaults delete com.asmrx3000.app renderer` goes back.
7. **Drift.** Sit still facing the screen for 5 minutes. Forward should stay forward (±5°). If it
   creeps while you hold a turned head, disable it with
   `defaults write com.asmrx3000.app headDriftCorrection -bool false`.
8. **Mic route.** Start a call or dictation using the AirPods mic. Sound quality drops (HFP), and
   that is expected: note it, then end the call; the engine should come back at 48 kHz.

### 6.6 Next
- Torso/shoulder reflection: the KU100 has no torso, and the torso does not turn with the head,
  which is a further dynamic externalisation cue. Needs a direction-dependent delay operator in the
  SH domain.
- BiMagLS (head-tracking-aware MagLS), and ITD individualisation from head size.
- A signed build with the profile-access capability, so the personalised HRTF is guaranteed.
