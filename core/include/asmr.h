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

    /* Acoustic environment (34..40). Any thread; applied at the next block. Values are
     * clamped to the given ranges; defaults in brackets. See docs/spatial-engine.md.
     * ASMR_PARAM_TEMPERATURE (17) and ASMR_PARAM_HUMIDITY (18, 0..1) above also drive air absorption. */
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

/* Generic controls (docs/physical-models.md, "Controls ABI"): every multi-parameter model as a
   tree of sliders. Each group has one macro (bit0); its children are physical parameters. Moving
   a macro moves its children along a physical curve unless the user set a child explicitly
   (reported in bit3); asmr_set_control(child, NAN) hands the child back to its macro. Presets
   are discrete children (bit2): integer index, 0 = "as in the scene", names via
   asmr_get_control_option. Legacy params are the macros: WATER = water.flow, WIND = wind.speed
   (m/s here, slider units in the param), RAIN = rain.rate, STORM = rain.storm, BIRDS =
   birds.activity, ORBIT = birds.orbit, VOLUME = master.volume, TIME_OF_DAY = env.time_of_day,
   TEMPERATURE = env.temperature, HUMIDITY = env.humidity. Setting STORM/TEMPERATURE/HUMIDITY
   via asmr_set_param marks those children explicit. Lock-free; applied on the next render. */
enum {
    ASMR_GROUP_WATER = 0,
    ASMR_GROUP_WIND = 1,
    ASMR_GROUP_FOLIAGE = 2,
    ASMR_GROUP_RAIN = 3,
    ASMR_GROUP_BIRDS = 4,
    ASMR_GROUP_MASTER = 5,
    ASMR_GROUP_ENVIRONMENT = 6, /* time of day, temperature, humidity, season */
    ASMR_GROUP_INSECTS = 7,
    ASMR_GROUP_FIRE = 8,
};

enum {
    ASMR_CONTROL_FLAG_MACRO = 1u << 0,      /* top-level slider of its group */
    ASMR_CONTROL_FLAG_LOG = 1u << 1,        /* logarithmic slider (rain.rate: 0 at the far left) */
    ASMR_CONTROL_FLAG_DISCRETE = 1u << 2,   /* integer preset index */
    ASMR_CONTROL_FLAG_OVERRIDDEN = 1u << 3, /* child set by the user, not following its macro */
};

/* Control ids (stable). Children list their macro in `parent`. */
enum {
    ASMR_CONTROL_WATER_FLOW = 100,       /* macro 0..1, 0.6 = as built; discharge Q = (x/0.6)^2 */
    ASMR_CONTROL_WATER_REGIME = 101,     /* preset: scene, still_lake, lake_lapping, slow_river, brook_pebbles, brook_rocks, rapids, small_waterfall, drips, trickle */
    ASMR_CONTROL_WATER_SPEED = 102,      /* m/s, follows Q^0.34 */
    ASMR_CONTROL_WATER_DEPTH = 103,      /* m, follows Q^0.40 */
    ASMR_CONTROL_WATER_WIDTH = 104,      /* m, follows Q^0.26 */
    ASMR_CONTROL_WATER_ROCK = 105,       /* m, median bed rock size */
    ASMR_CONTROL_WATER_AERATION = 106,
    ASMR_CONTROL_WATER_TURBULENCE = 107,
    ASMR_CONTROL_WATER_GAIN = 108,
    ASMR_CONTROL_WIND_SPEED = 200,       /* macro, m/s */
    ASMR_CONTROL_WIND_GUSTINESS = 201,
    ASMR_CONTROL_WIND_EXPOSURE = 202,
    ASMR_CONTROL_WIND_CHARACTER = 203,   /* preset of the ambient wind sources */
    ASMR_CONTROL_WIND_WHISTLE = 204,
    ASMR_CONTROL_FOLIAGE_DENSITY = 300,  /* macro, x leaf density */
    ASMR_CONTROL_FOLIAGE_DRYNESS = 301,  /* -1 = season, 0 fresh .. 1 dry */
    ASMR_CONTROL_FOLIAGE_FLUTTER = 302,
    ASMR_CONTROL_FOLIAGE_CREAK = 303,
    ASMR_CONTROL_FOLIAGE_EXPOSURE = 304,
    ASMR_CONTROL_RAIN_RATE = 400,        /* macro, mm/h */
    ASMR_CONTROL_RAIN_STORM = 401,       /* follows heavy rain */
    ASMR_CONTROL_RAIN_SURFACE = 402,     /* preset of the open rain patches */
    ASMR_CONTROL_RAIN_DRYING = 403,      /* x speed of draining/drying (1 = real time) */
    ASMR_CONTROL_RAIN_GAIN = 404,
    ASMR_CONTROL_BIRDS_ACTIVITY = 500,   /* macro */
    ASMR_CONTROL_BIRDS_ORBIT = 501,
    ASMR_CONTROL_MASTER_VOLUME = 600,    /* macro */
    ASMR_CONTROL_ENV_TIME = 700,         /* macro, hours */
    ASMR_CONTROL_ENV_TEMPERATURE = 701,  /* deg C, follows a diurnal cycle */
    ASMR_CONTROL_ENV_HUMIDITY = 702,     /* follows a diurnal cycle */
    ASMR_CONTROL_ENV_SEASON = 703,       /* spring, summer, autumn, winter */
    ASMR_CONTROL_INSECTS_ACTIVITY = 800, /* macro */
    ASMR_CONTROL_INSECTS_WARMTH = 801,   /* deg C microclimate offset */
    ASMR_CONTROL_FIRE_HEAT = 900,        /* macro, kW */
    ASMR_CONTROL_FIRE_MOISTURE = 901,
    ASMR_CONTROL_FIRE_RESIN = 902,
};

typedef struct AsmrControlInfo {
    uint32_t id;           /* stable across runs */
    uint32_t group;        /* sound group: ASMR_GROUP_WATER = 0, ASMR_GROUP_WIND = 1, ASMR_GROUP_FOLIAGE = 2, ASMR_GROUP_RAIN = 3, ASMR_GROUP_BIRDS = 4, ASMR_GROUP_MASTER = 5, then more */
    uint32_t parent;       /* id of the macro control this one branches from; UINT32_MAX for top-level */
    uint32_t flags;        /* bit0 = macro (top-level slider), bit1 = logarithmic, bit2 = discrete/preset index */
    float min, max, value, default_value;
    char key[24];          /* machine name, e.g. "water.flow", "water.depth" */
    char unit[8];          /* "m/s", "m", "mm/h", "" */
} AsmrControlInfo;

AsmrEngine *asmr_engine_new(float sample_rate);
/* Engine running a JSON scene description (docs/director.md). NULL if it does not validate. */
AsmrEngine *asmr_engine_new_with_scene(float sample_rate, const char *scene_json);
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
uint32_t asmr_get_objects(const AsmrEngine *engine, AsmrObjectInfo *out, uint32_t max);
/* Moves a MOVABLE object (a tree carries its canopy and perched birds; a river translates its
   polyline). Lock-free; applied by the audio thread on the next asmr_render. Ignored otherwise. */
void asmr_set_object_position(const AsmrEngine *engine, uint32_t id, float x, float y, float z);
/* Control tree (see AsmrControlInfo above). Any thread. */
uint32_t asmr_get_controls(const AsmrEngine *engine, AsmrControlInfo *out, uint32_t max);
void asmr_set_control(const AsmrEngine *engine, uint32_t id, float value);
/* Name of option `index` of a discrete control into `out` (NUL-terminated); returns its length, 0 if none. */
uint32_t asmr_get_control_option(const AsmrEngine *engine, uint32_t id, uint32_t index, char *out, uint32_t max);

/* Live per-track peak waveform (asmr_get_scope): one point per 64-sample block, oldest first. */
enum {
    ASMR_SCOPE_WATER = 0,
    ASMR_SCOPE_WIND = 1,
    ASMR_SCOPE_BIRDS = 2,
    ASMR_SCOPE_FLIGHT = 3,  /* birds in flight */
    ASMR_SCOPE_OUTPUT = 4,
    ASMR_SCOPE_RAIN = 5,
    ASMR_SCOPE_INSECTS = 6,
    ASMR_SCOPE_FIRE = 7,
    ASMR_SCOPE_THUNDER = 8,
};
uint32_t asmr_get_scope(const AsmrEngine *engine, uint32_t track, float *out, uint32_t points);

#ifdef __cplusplus
}
#endif

#endif /* ASMR_H */
