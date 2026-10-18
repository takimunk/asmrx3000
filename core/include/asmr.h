/* asmr-core C ABI. Keep in sync with core/src/lib.rs and core/src/engine.rs. */
#ifndef ASMR_H
#define ASMR_H

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef struct AsmrEngine AsmrEngine;

/* Parameters (asmr_set_param / asmr_get_param). Angles in radians. */
enum {
    ASMR_PARAM_VOLUME = 0,     /* 0..1.5 */
    ASMR_PARAM_WIND = 1,       /* 0..1 mean wind */
    ASMR_PARAM_BIRDS = 2,      /* 0..1 song activity */
    ASMR_PARAM_WATER = 3,      /* 0..1 river flow */
    ASMR_PARAM_ORBIT = 4,      /* 0..1 speed of the bird circling the listener, 0 = perched */
    ASMR_PARAM_PLAYING = 5,    /* 0 or 1, fades in/out */
    ASMR_PARAM_HEAD_YAW = 6,   /* + = turned left */
    ASMR_PARAM_HEAD_PITCH = 7, /* + = looking up */
    ASMR_PARAM_HEAD_ROLL = 8,  /* + = right ear down */

    /* Acoustic environment (32..40). Any thread; applied at the next block. Values are
     * clamped to the given ranges; defaults in brackets. See docs/spatial-engine.md. */
    ASMR_PARAM_TEMPERATURE = 32,  /* air temperature, deg C, -30..45 [15]: speed of sound, ISO 9613-1 absorption */
    ASMR_PARAM_HUMIDITY = 33,     /* relative humidity, %, 1..100 [70] */
    ASMR_PARAM_PRESSURE = 34,     /* ambient pressure, kPa, 50..110 [101.325] */
    ASMR_PARAM_GROUND = 35,       /* ground type, one of ASMR_GROUND_* [ASMR_GROUND_FOREST_FLOOR] */
    ASMR_PARAM_TREE_DENSITY = 36, /* trunks per hectare, 0..3000 [400]: scattering, occlusion, reverb */
    ASMR_PARAM_FOLIAGE = 37,      /* canopy foliage density, 0 (bare) .. 1 (dense summer) [0.6] */
    ASMR_PARAM_REVERB = 38,       /* late reverb level relative to the physical estimate, 0..4 [1] */
    ASMR_PARAM_EAR_HEIGHT = 39,   /* listener ear height above the ground, m, 0.1..3 [1.5] */
    ASMR_PARAM_HRTF = 40,         /* 0 = measured HRTF (SADIE II KU100), 1 = analytic spherical head [0] */
};

/* Values of ASMR_PARAM_GROUND (effective flow resistivity in brackets). */
enum {
    ASMR_GROUND_FOREST_FLOOR = 0, /* litter / moss, 40 kPa s/m^2 */
    ASMR_GROUND_GRASS = 1,        /* 200 kPa s/m^2 */
    ASMR_GROUND_WATER = 2,        /* acoustically rigid */
    ASMR_GROUND_COMPACT_SOIL = 3, /* 2000 kPa s/m^2 */
    ASMR_GROUND_SNOW = 4,         /* 20 kPa s/m^2 */
    ASMR_GROUND_ASPHALT = 5,      /* acoustically rigid */
};

/* Telemetry (asmr_get_telemetry). */
enum {
    ASMR_TELEMETRY_DSP_LOAD = 0,     /* fraction of the real-time budget */
    ASMR_TELEMETRY_GUST = 1,         /* current gust strength */
    ASMR_TELEMETRY_SAMPLE_RATE = 2,
    ASMR_TELEMETRY_BUBBLE_RATE = 3,  /* bubbles per second, whole river */
    ASMR_TELEMETRY_TIME = 4,         /* seconds of rendered audio */
    ASMR_TELEMETRY_OUTPUT_LEVEL = 5, /* RMS of the output */
};

enum {
    ASMR_SOURCE_WATER = 0,
    ASMR_SOURCE_WIND = 1,
    ASMR_SOURCE_BIRD = 2,
};

/* World frame and head frame: +x forward, +y left, +z up, metres. */
typedef struct AsmrSourceInfo {
    uint32_t kind;
    float world_x, world_y, world_z; /* position in the scene */
    float rel_x, rel_y, rel_z;       /* position relative to the listener's head */
    float level;                     /* recent RMS of the source signal */
    float freq;                      /* dominant pitch if known (birds), else 0 */
} AsmrSourceInfo;

AsmrEngine *asmr_engine_new(float sample_rate);
void asmr_engine_free(AsmrEngine *engine);

/* Audio thread only. Non-interleaved stereo. Any frame count; the binaural decoder adds a
 * fixed latency of 64 samples. */
void asmr_render(const AsmrEngine *engine, float *left, float *right, uint32_t frames);

/* Any thread. */
void asmr_set_param(const AsmrEngine *engine, uint32_t param, float value);
float asmr_get_param(const AsmrEngine *engine, uint32_t param);
float asmr_get_telemetry(const AsmrEngine *engine, uint32_t which);
uint32_t asmr_get_sources(const AsmrEngine *engine, AsmrSourceInfo *out, uint32_t max);
void asmr_get_spectrum(const AsmrEngine *engine, float *out, uint32_t bands); /* 40 Hz..16 kHz, log bands, 0..1 */
void asmr_get_wind_field(const AsmrEngine *engine, float *out, uint32_t points); /* left -> right */

#ifdef __cplusplus
}
#endif

#endif /* ASMR_H */
