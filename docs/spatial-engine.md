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
