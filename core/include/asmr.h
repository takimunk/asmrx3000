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
    ASMR_PARAM_TIME_OF_DAY = 9, /* local time, hours 0..24; picks active species (dawn chorus, owls at night) */
    /* Weather (docs/physical-models.md). 10..15 are left free for other parameters. */
    ASMR_PARAM_RAIN = 16,        /* rain rate in the open, mm/h 0..200 (0.5 drizzle, 4 steady, 15 heavy, 50 cloudburst) */
    ASMR_PARAM_TEMPERATURE = 17, /* air temperature, deg C (insect song rates follow it) */
    ASMR_PARAM_HUMIDITY = 18,    /* relative humidity 0..1 (drying of leaves and ground) */
    ASMR_PARAM_STORM = 19,       /* thunderstorm activity 0..1 (lightning rate and closeness) */
};

/* Telemetry (asmr_get_telemetry). */
enum {
    ASMR_TELEMETRY_DSP_LOAD = 0,     /* fraction of the real-time budget */
    ASMR_TELEMETRY_GUST = 1,         /* current gust strength */
    ASMR_TELEMETRY_SAMPLE_RATE = 2,
    ASMR_TELEMETRY_BUBBLE_RATE = 3,  /* bubbles per second, whole river */
    ASMR_TELEMETRY_TIME = 4,         /* seconds of rendered audio */
    ASMR_TELEMETRY_OUTPUT_LEVEL = 5, /* RMS of the output */
    ASMR_TELEMETRY_RAIN_RATE = 8,    /* actual rain rate (smoothed, fluctuating), mm/h */
    ASMR_TELEMETRY_WETNESS = 9,      /* leaf wetness 0..1 */
    ASMR_TELEMETRY_LIGHTNING = 10,   /* flash 0..1: jumps to 1 on a strike, thunder arrives distance/343 s later */
    ASMR_TELEMETRY_DRIP = 11,        /* canopy drip rate, mm/h (keeps going after the rain) */
};

enum {
    ASMR_SOURCE_WATER = 0,
    ASMR_SOURCE_WIND = 1,
    ASMR_SOURCE_BIRD = 2,
    ASMR_SOURCE_RAIN = 3,    /* rain patch / drip zone under a tree */
    ASMR_SOURCE_THUNDER = 4, /* direction proxy ~40 m towards the latest strike */
    ASMR_SOURCE_FIRE = 5,
    ASMR_SOURCE_INSECT = 6,
};

/* World frame and head frame: +x forward, +y left, +z up, metres. */
typedef struct AsmrSourceInfo {
    uint32_t kind;
    float world_x, world_y, world_z; /* position in the scene */
    float rel_x, rel_y, rel_z;       /* position relative to the listener's head */
    float level;                     /* recent RMS of the source signal */
    float freq;                      /* dominant pitch if known (birds, insects, wind tones), else 0 */
} AsmrSourceInfo;

/* Scene objects (asmr_get_objects). */
enum {
    ASMR_OBJECT_TREE = 0,
    ASMR_OBJECT_RIVER = 1,
    ASMR_OBJECT_BIRD = 2,
    ASMR_OBJECT_SOURCE = 3, /* generic physical source: rain patch, campfire, crickets, waterfall, thunder */
};

enum {
    ASMR_OBJECT_FLAG_MOVABLE = 1u << 0, /* the user may drag it with asmr_set_object_position */
    ASMR_OBJECT_FLAG_MOVING = 1u << 1,  /* currently moving on its own (flying) */
    ASMR_OBJECT_FLAG_ABSENT = 1u << 2,  /* off stage: silent, do not draw */
};

/* Position: trees = trunk base, rivers = polyline centroid, birds = the bird. Same frame as above. */
typedef struct AsmrObjectInfo {
    uint32_t id;    /* stable for the lifetime of the scene */
    uint32_t kind;  /* ASMR_OBJECT_* */
    float x, y, z;
    uint32_t flags; /* ASMR_OBJECT_FLAG_* */
} AsmrObjectInfo;

AsmrEngine *asmr_engine_new(float sample_rate);
/* Engine running a JSON scene description (docs/director.md). NULL if it does not validate. */
AsmrEngine *asmr_engine_new_with_scene(float sample_rate, const char *scene_json);
void asmr_engine_free(AsmrEngine *engine);

/* Audio thread only. Non-interleaved stereo. */
void asmr_render(const AsmrEngine *engine, float *left, float *right, uint32_t frames);

/* Any thread. */
void asmr_set_param(const AsmrEngine *engine, uint32_t param, float value);
float asmr_get_param(const AsmrEngine *engine, uint32_t param);
float asmr_get_telemetry(const AsmrEngine *engine, uint32_t which);
uint32_t asmr_get_sources(const AsmrEngine *engine, AsmrSourceInfo *out, uint32_t max);
void asmr_get_spectrum(const AsmrEngine *engine, float *out, uint32_t bands); /* 40 Hz..16 kHz, log bands, 0..1 */
void asmr_get_wind_field(const AsmrEngine *engine, float *out, uint32_t points); /* left -> right */
uint32_t asmr_get_objects(const AsmrEngine *engine, AsmrObjectInfo *out, uint32_t max);
/* Moves a MOVABLE object (a tree carries its canopy and perched birds; a river translates its
   polyline). Lock-free; applied by the audio thread on the next asmr_render. Ignored otherwise. */
void asmr_set_object_position(const AsmrEngine *engine, uint32_t id, float x, float y, float z);

#ifdef __cplusplus
}
#endif

#endif /* ASMR_H */
