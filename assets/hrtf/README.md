# Measured HRTF asset

`sadie2_ku100_48k.ahrt` is a measured head-related impulse response set for the
asmr-core binaural renderer, in the compact "AHRT" v1 format (see
`tools/hrtf/sofa_to_ahrt.py` for the byte layout).

## Source

- Database: SADIE II Database, version 2-2 (18-06-2024), University of York
  (The Audio Lab, Department of Electronic Engineering; Cal Armstrong, Lewis
  Thresh, Gavin Kearney).
- Subject: D1, Neumann KU100 dummy head. Free field, Genelec 8010 loudspeakers
  at 1.2 m, 8802 measured directions covering the full sphere (-90 to +90 deg
  elevation).
- Project page: https://www.york.ac.uk/sadie-project/database.html
- Archive: https://zenodo.org/records/12092466/files/D1_HRIR_SOFA.zip?download=1
  (DOI 10.5281/zenodo.12092466),
  sha256 `366321fa78f211bacc0ec6bea96701625b196a5db54ec16748b0eab0b9705f75`
- File used: `D1_HRIR_SOFA/D1_48K_24bit_256tap_FIR_SOFA.sofa`,
  sha256 `9af7cb19531e52fb7ae8ec92621e6ab62b1d5fe584b3742be36699a0ddb0ccd4`

### Equalisation

SADIE II publishes one processed version of each HRIR set. The SOFA `History`
attribute says: "Measurement, Low Frequency Extension, Diffuse Field
Equalization, Windowing, Diffuse Field Equalization". So this asset is
**diffuse-field equalised**. It is not minimum phase, and it is not
time-aligned per direction: interaural time differences are intact. The same
processing is used at 44.1, 48 and 96 kHz and in the WAV copies. No raw or
free-field-only version is published.

## Licence

Apache License 2.0, Copyright 2018 University of York. The full text and the
original notice are in [`LICENSE-SADIE-II.txt`](LICENSE-SADIE-II.txt). The
licence allows redistributing modified derivatives. Conditions: include the
licence, keep the copyright notice, and state the changes (listed below).

### Attribution

> HRTF data: SADIE II Database, subject D1 (Neumann KU100), Copyright 2018
> University of York, licensed under the Apache License 2.0.
> C. Armstrong, L. Thresh, D. Murphy, G. Kearney, "A Perceptual Evaluation of
> Individual and Non-Individual HRTFs: A Case Study of the SADIE II Database",
> Applied Sciences 8(11):2029, 2018. https://doi.org/10.3390/app8112029
> Modified: direction subset, delay trimming, truncation, gain normalisation
> and 16-bit quantisation (see assets/hrtf/README.md).

## Changes made by the conversion

1. Picked 598 of the 8703 distinct measured directions. Each point of a
   600-point Fibonacci sphere maps to its nearest measured direction, and
   duplicates are removed.
2. Removed the common leading delay: every IR is shifted by the same 52 samples,
   so the earliest -20 dB onset over the whole set falls at sample 12. ITDs are
   unchanged.
3. Zero-padded or truncated to 256 taps, with a 16-sample half-Hann fade-out at
   the end.
4. Gain-normalised so that the mean magnitude over 200-800 Hz, averaged over
   both ears, is 1.0 for the frontal direction.
5. Quantised to int16 with one global float scale (SNR about 75 dB).

Directions are unit vectors in the engine frame: +x forward, +y left, +z up.
This is the same as SOFA's frame, where azimuth runs counterclockwise from the
front. Receiver 0 is the left ear (ReceiverPosition y = +0.09 m).

## Reproduce

```sh
mkdir -p hrtf-src && cd hrtf-src
curl -L -o D1_HRIR_SOFA.zip "https://zenodo.org/records/12092466/files/D1_HRIR_SOFA.zip?download=1"
unzip D1_HRIR_SOFA.zip D1_HRIR_SOFA/D1_48K_24bit_256tap_FIR_SOFA.sofa
cd ..
uv run tools/hrtf/sofa_to_ahrt.py \
  hrtf-src/D1_HRIR_SOFA/D1_48K_24bit_256tap_FIR_SOFA.sofa \
  assets/hrtf/sadie2_ku100_48k.ahrt \
  --dirs 600 --taps 256 --name "SADIE II KU100 (D1) 48k DFE"
```

Sanity check of the result: 598 directions, 256 taps, 619584 bytes. Energy
ILD at the left direction is +15.8 dB, at the right -16.3 dB, at the front
-0.5 dB. The ITD at the left direction is +646 us from the -20 dB onsets and
+797 us from cross-correlation below 1.5 kHz, with the left ear leading.
