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
    ASMR_PARAM_NEURAL_BIRDS = 10, /* 1 = trained neural voices where a species has one (chaffinch, robin, song thrush, nightingale, blackbird) [1], 0 = procedural */

    /* Acoustic environment (34..40). Any thread; applied at the next block. Values are
     * clamped to the given ranges; defaults in brackets. See docs/spatial-engine.md.
     * ASMR_PARAM_TEMPERATURE (17) and ASMR_PARAM_HUMIDITY (18, 0..1) above also drive air absorption. */
    ASMR_PARAM_PRESSURE = 34,     /* ambient pressure, kPa, 50..110 [101.325] */
    ASMR_PARAM_GROUND = 35,       /* ground type, one of ASMR_GROUND_* [ASMR_GROUND_FOREST_FLOOR] */
    ASMR_PARAM_TREE_DENSITY = 36, /* trunks per hectare, 0..3000 [400]: scattering, occlusion, reverb */
    ASMR_PARAM_FOLIAGE = 37,      /* canopy foliage density, 0 (bare) .. 1 (dense summer) [0.6] */
    ASMR_PARAM_REVERB = 38,       /* late reverb level relative to the physical estimate, 0..4 [1] */
    ASMR_PARAM_EAR_HEIGHT = 39,   /* listener ear height above the ground, m, 0.1..60 [1.5] */
    ASMR_PARAM_HRTF = 40,         /* 0 = measured HRTF (SADIE II KU100), 1 = analytic spherical head [0] */

    /* Head tracking (41..43). See asmr_set_head_pose. */
    ASMR_PARAM_OUTPUT_LATENCY = 41,  /* render-to-ear delay the head pose is predicted over, s, 0..0.5 [0.15];
                                        host-measured (output presentation latency + IO buffer, incl. Bluetooth) */
    ASMR_PARAM_HEAD_PREDICTION = 42, /* 0 = last measured pose, 1 = full latency compensation [1] */
    ASMR_PARAM_HEAD_PARALLAX = 43,   /* 1 = head turns about the neck: the ears translate (motion parallax) [1] */

    /* Flight mode (docs/flight-mode.md): 1 = the listener travels forward through an endless,
     * procedurally generated landscape; 0 = back to the scene. Cross-fades over 1.5 s. The first
     * switch-on builds the landscape and its voices on the calling thread (tens of ms, allocates;
     * call it from the UI thread, never from the audio thread). Speed, altitude, path and landscape
     * are the ASMR_GROUP_FLIGHT controls. While flying, asmr_get_objects / asmr_get_sources report
     * the streamed world instead of the scene (see ASMR_OBJECT_LAKE / ROCK, asmr_get_object_path),
     * and ASMR_PARAM_EAR_HEIGHT is replaced by the flight altitude. */
    ASMR_PARAM_FLIGHT = 44,       /* 0 or 1 [0] */
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
    ASMR_TELEMETRY_HEAD_HORIZON_MS = 12, /* head-pose prediction horizon used, ms */
    ASMR_TELEMETRY_HEAD_RATE = 13,       /* head-pose samples per second arriving */
    /* Flight mode. */
    ASMR_TELEMETRY_FLIGHT = 14,          /* cross-fade 0 (scene) .. 1 (flying); objects switch at 0.5 */
    ASMR_TELEMETRY_FLIGHT_SPEED = 15,    /* current speed, m/s (ramps to flight.speed at 2.5 m/s^2) */
    ASMR_TELEMETRY_FLIGHT_ALTITUDE = 16, /* ears above the ground, m (follows flight.altitude, gentle swells) */
    ASMR_TELEMETRY_FLIGHT_ODOMETER = 17, /* distance flown along the path, m, wraps at 10 km (scroll the ground with it) */
    ASMR_TELEMETRY_FLIGHT_TURN = 18,     /* turn rate of the course, rad/s, + = turning left */
    ASMR_TELEMETRY_FLIGHT_HEADING = 19,  /* heading of the course in the landscape, rad 0..2pi (counter-clockwise) */
    ASMR_TELEMETRY_FLIGHT_X = 20,        /* position in the landscape, m, wrapping at 4096 m: anchor ground textures to it */
    ASMR_TELEMETRY_FLIGHT_Y = 21,        /* (the reported objects stay in the travel frame: +x = where you fly) */
    /* Sound director (docs/sound-director.md). Loudness of the mix after its dynamics, before
     * ASMR_PARAM_VOLUME, BS.1770: */
    ASMR_TELEMETRY_LOUDNESS_MOMENTARY = 22,  /* LUFS, 400 ms */
    ASMR_TELEMETRY_LOUDNESS_SHORT = 23,      /* LUFS, 3 s */
    ASMR_TELEMETRY_LOUDNESS_INTEGRATED = 24, /* LUFS, gated, since start */
    ASMR_TELEMETRY_MIX_GAIN_DB = 25,         /* leveller + comfort compressor + limiter, dB */
    ASMR_TELEMETRY_SOLOISTS = 26,            /* foreground sources right now */
    ASMR_TELEMETRY_MASKING = 27,             /* share of the soloists' loudness masked by the bed, 0..1 */
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
    ASMR_OBJECT_LAKE = 4,   /* flight mode: a lake; shape = shore outline (asmr_get_object_path) */
    ASMR_OBJECT_ROCK = 5,   /* flight mode: a boulder or a ridge (shape = ridge line); z = its top */
};

enum {
    ASMR_OBJECT_FLAG_MOVABLE = 1u << 0, /* the user may drag it with asmr_set_object_position */
    ASMR_OBJECT_FLAG_MOVING = 1u << 1,  /* currently moving on its own (flying) */
    ASMR_OBJECT_FLAG_ABSENT = 1u << 2,  /* off stage: silent, do not draw */
};

/* Position: trees = trunk base, rivers = polyline centroid, birds = the bird. Same frame as above.
   Flight mode: everything is listener-relative and streams past (ground at z = -altitude); ids
   (>= 0x40000000) are stable while an object exists; trees are the nearest ones that fit in
   256 objects, after the rivers, lakes, rocks, sources and birds; a river's position is its middle. */
typedef struct AsmrObjectInfo {
    uint32_t id;    /* stable for the lifetime of the scene (flight: while the object exists) */
    uint32_t kind;  /* ASMR_OBJECT_* */
    float x, y, z;
    uint32_t flags; /* ASMR_OBJECT_FLAG_* */
} AsmrObjectInfo;

/* Birds of the scene (asmr_get_birds): what each one is doing, for drawing it fly its own way.
   docs/director.md "Life model". */
enum {
    ASMR_SPECIES_CHAFFINCH = 0, ASMR_SPECIES_ROBIN = 1, ASMR_SPECIES_SONG_THRUSH = 2, ASMR_SPECIES_NIGHTINGALE = 3,
    ASMR_SPECIES_TAWNY_OWL = 4, ASMR_SPECIES_CUCKOO = 5, ASMR_SPECIES_WOOD_PIGEON = 6, ASMR_SPECIES_SWALLOW = 7,
    ASMR_SPECIES_BLACKBIRD = 8, ASMR_SPECIES_SKYLARK = 9, ASMR_SPECIES_GULL = 10, ASMR_SPECIES_RAVEN = 11,
};
enum {
    ASMR_BIRD_RESTING = 0,     /* perched, quiet (or roosting) */
    ASMR_BIRD_SINGING = 1,     /* on a song post */
    ASMR_BIRD_ALERT = 2,       /* perched, alarm calls */
    ASMR_BIRD_FORAGING = 3,    /* on the ground, hopping */
    ASMR_BIRD_FLYING = 4,      /* flying from A to B */
    ASMR_BIRD_SOARING = 5,     /* feeding on the wing / soaring over an area */
    ASMR_BIRD_SONG_FLIGHT = 6, /* skylark: climbing, hovering, singing */
    ASMR_BIRD_CIRCLING = 7,
    ASMR_BIRD_ABSENT = 8,
};
enum {
    ASMR_FLIGHT_BOUNDING = 0,  /* finches: flap bursts and closed-wing glides, undulating */
    ASMR_FLIGHT_DIRECT = 1,    /* thrushes, robin, cuckoo: fast, low, straight */
    ASMR_FLIGHT_SWOOPING = 2,  /* swallows */
    ASMR_FLIGHT_CLATTER = 3,   /* wood pigeon: wing-clap take-off, glides */
    ASMR_FLIGHT_SILENT = 4,    /* owl: few beats, long glides */
    ASMR_FLIGHT_SOARING = 5,   /* gull, raven: slow beats, long glides */
    ASMR_FLIGHT_HOVERING = 6,  /* skylark */
};
enum {
    ASMR_BIRD_FLAG_VOCAL = 1u << 0,   /* voice sounding now */
    ASMR_BIRD_FLAG_CALLING = 1u << 1, /* short calls rather than song */
    ASMR_BIRD_FLAG_ALARM = 1u << 2,   /* alarm calls / fleeing */
};
typedef struct AsmrBirdInfo {
    uint32_t id;       /* = AsmrObjectInfo.id */
    uint32_t species;  /* ASMR_SPECIES_* */
    uint32_t state;    /* ASMR_BIRD_* */
    uint32_t flight;   /* ASMR_FLIGHT_* */
    float flap;        /* wingbeat effort now 0..1; 0 = perched, gliding, wings closed */
    float vx, vy, vz;  /* velocity, m/s */
    uint32_t flags;    /* ASMR_BIRD_FLAG_* */
} AsmrBirdInfo;

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
    ASMR_GROUP_FLIGHT = 9,      /* flight mode: speed, altitude, path, landscape */
    ASMR_GROUP_MIX = 10,        /* mix intent for the sound director: clarity, density, focus */
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
    /* Flight mode (ASMR_PARAM_FLIGHT). Applied while flying and kept for the next flight. */
    ASMR_CONTROL_FLIGHT_SPEED = 1000,     /* macro, m/s 0..25 [6]: walk 1.5, run 5, glide 12, fast flight 25 */
    ASMR_CONTROL_FLIGHT_ALTITUDE = 1001,  /* ears above the ground, m 1.2..40 (log); follows speed: 1.7 walking .. ~20 above the canopy */
    ASMR_CONTROL_FLIGHT_CURVINESS = 1002, /* wandering of the course when not steered by head: 0 straight .. 1 (turn radius 250 m) [0.35] */
    ASMR_CONTROL_FLIGHT_BIOME = 1003,     /* preset: mixed, broadleaf, taiga, river_valley, meadows, highlands [mixed]; option names via asmr_get_control_option */
    ASMR_CONTROL_FLIGHT_DENSITY = 1004,   /* trees and stands x 0..2 [1]; thins or fills the forest at once */
    ASMR_CONTROL_FLIGHT_SEED = 1005,      /* landscape 0..999 [0]; a new seed or biome fades out and regenerates around the listener */
    ASMR_CONTROL_FLIGHT_AIRFLOW = 1006,   /* wind noise of motion at the ears x 0..3 [1 = physical, subtle below ~10 m/s] */
    ASMR_CONTROL_FLIGHT_STEER = 1007,     /* "flight.steer_by_head", preset: off, yaw, yaw_pitch [yaw]. Fly where the head looks:
                                             the course turns towards the (predicted) head yaw beyond an 8 deg dead zone, smoothed
                                             over 0.35 s and slew-limited; yaw_pitch also climbs / dives with the head pitch.
                                             Live only while asmr_set_head_pose samples arrive; otherwise (and with off) the
                                             course wanders by flight.curviness and turning the head only looks around. */
    ASMR_CONTROL_FLIGHT_TURN_RATE = 1008, /* "flight.turn_rate", deg/s 5..90 [30]: fastest turn, reached with the head ~43 deg off course */
    /* Mix intent (docs/sound-director.md): art direction for the real-time sound director, never
     * per-block levels. Focus: -1 background .. 0 neutral .. +1 foreground. */
    ASMR_CONTROL_MIX_CLARITY = 1100,       /* macro 0..1 [0.6]: 0 = raw physical mix, 1 = strongest separation */
    ASMR_CONTROL_MIX_DENSITY = 1101,       /* overlapping bird phrases 0 (one at a time) .. 1 (no limit); follows clarity */
    ASMR_CONTROL_MIX_SOLOISTS = 1102,      /* foreground sources at once, 1..4; follows clarity [2] */
    ASMR_CONTROL_MIX_LOUDNESS = 1103,      /* target loudness, LUFS -36..-10 [-22] */
    ASMR_CONTROL_MIX_DYNAMICS = 1104,      /* leveller / comfort compressor amount 0..1 [0.7] */
    ASMR_CONTROL_MIX_FOCUS_WATER = 1105,
    ASMR_CONTROL_MIX_FOCUS_WIND = 1106,
    ASMR_CONTROL_MIX_FOCUS_BIRDS = 1107,
    ASMR_CONTROL_MIX_FOCUS_RAIN = 1108,    /* rain and thunder */
    ASMR_CONTROL_MIX_FOCUS_INSECTS = 1109,
    ASMR_CONTROL_MIX_FOCUS_FIRE = 1110,
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

/* Audio thread only, instead of asmr_render: a 7.1.4 loudspeaker bed for an external binaural
 * renderer (AUSpatialMixer with the listener's personalised HRTF). channels: count pointers to
 * frames floats, order L R C LFE Ls Rs Rls Rrs Vhl Vhr Ltr Rtr (kAudioChannelLayoutTag_Atmos_7_1_4);
 * extra channels are zeroed, LFE is silent. world_locked = 1: the engine does not apply its head
 * rotation (the renderer tracks the head itself). */
void asmr_render_bed(const AsmrEngine *engine, float *const *channels, uint32_t count, uint32_t frames,
                     uint32_t world_locked);

/* Any thread. Head orientation from a motion sensor: quaternion (w, x, y, z) rotating head-frame
 * vectors into the world frame, engine axes (+x forward, +y left, +z up), already recentred;
 * angular velocity wx, wy, wz in rad/s in the head frame (+z = turning left); age = seconds since
 * the sensor measured it. Replaces ASMR_PARAM_HEAD_YAW/PITCH/ROLL from the first call on. */
void asmr_set_head_pose(const AsmrEngine *engine, float qw, float qx, float qy, float qz,
                        float wx, float wy, float wz, float age);

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
/* Scene birds (AsmrBirdInfo above; match to asmr_get_objects by id); 0 in flight mode. Any thread. */
uint32_t asmr_get_birds(const AsmrEngine *engine, AsmrBirdInfo *out, uint32_t max);
/* Control tree (see AsmrControlInfo above). Any thread. */
uint32_t asmr_get_controls(const AsmrEngine *engine, AsmrControlInfo *out, uint32_t max);
void asmr_set_control(const AsmrEngine *engine, uint32_t id, float value);
/* Name of option `index` of a discrete control into `out` (NUL-terminated); returns its length, 0 if none. */
uint32_t asmr_get_control_option(const AsmrEngine *engine, uint32_t id, uint32_t index, char *out, uint32_t max);

/* Flight mode: shape of a streamed object (river bed, lake shore, ridge line) as listener-relative
   (x, y, z) triples, up to max_points (24 at most); returns the number of points, 0 for objects
   without a shape and outside flight mode. `out` holds 3 * max_points floats. Any thread. */
uint32_t asmr_get_object_path(const AsmrEngine *engine, uint32_t id, float *out, uint32_t max_points);

/* Prompt control (docs/prompt-control.md). Control thread. JSON strings, UTF-8. Read-only
   state/prompt calls write their NUL-terminated result into `out` only if `max` > its length,
   and return the length in bytes either way: call with out = NULL to size those buffers.
   Applying a patch is a mutation; use asmr_apply_patch_json_alloc to obtain its result once.
   - state: {"controls": [{key, group, macro | parent, value, target?, option?, options?, min, max, log?,
     unit?, default, overridden?}], "environment": {...}, "objects": [{id, kind, pos, movable?,
     moving?}], "frame", "gliding"}.
   - patch: {"glide_s": 4, "changes": [{"key": "water.flow", "value": 0.3},
     {"key": "water.regime", "value": "rapids"}, {"key": "water.depth", "value": "reset"}],
     "moves": [{"object": 3, "position": [x, y, z]}]}. Keys are AsmrControlInfo.key. Values are
     clamped; continuous controls glide (smoothstep, glide_s seconds, per change too), presets
     and resets of presets switch at once; "reset" glides a child back onto its macro's curve.
     asmr_set_control / asmr_set_param on a gliding control cancels its glide. Valid entries are
     applied even if others fail. Result: {"ok", "applied": [{key, value, clamped?}],
     "moved": [ids], "errors": ["unknown key `x`", ...], "undo": <patch restoring the touched
     controls and objects>}.
   - prompt: {"system": static system prompt, "context": current values as text,
     "tool": {"name": "set_scene", "description", "input_schema"}}; the model's tool input is a
     patch plus "explanation". */
uint32_t asmr_get_state_json(const AsmrEngine *engine, char *out, uint32_t max);
/* Every call applies the patch, even with out=NULL. Use the allocated variant below to
   obtain the full result and correct undo without repeating the operation. */
uint32_t asmr_apply_patch_json(const AsmrEngine *engine, const char *patch, char *out, uint32_t max);
/* Applies once; returns NULL on NULL inputs. Free the returned JSON with asmr_string_free.
   These allocation APIs belong on the control thread. */
char *asmr_apply_patch_json_alloc(const AsmrEngine *engine, const char *patch);
void asmr_string_free(char *text);
uint32_t asmr_get_prompt_json(const AsmrEngine *engine, char *out, uint32_t max);

/* Sound director telemetry (asmr_get_mix). Groups index the arrays below. */
enum {
    ASMR_MIX_GROUP_WATER = 0,
    ASMR_MIX_GROUP_WIND = 1,    /* wind and foliage */
    ASMR_MIX_GROUP_BIRDS = 2,
    ASMR_MIX_GROUP_RAIN = 3,
    ASMR_MIX_GROUP_THUNDER = 4,
    ASMR_MIX_GROUP_INSECTS = 5,
    ASMR_MIX_GROUP_FIRE = 6,
    ASMR_MIX_GROUPS = 8,        /* array size (one spare) */
};

typedef struct AsmrMixInfo {
    float loudness_momentary;   /* LUFS after the mix dynamics, before volume */
    float loudness_short;
    float loudness_integrated;
    float gain_db;              /* mix dynamics gain */
    float group_lufs[8];        /* each group's share of the output (bed + its soloists), LUFS; -70 = silent */
    float group_sone[8];        /* loudness of the group alone, sone (Zwicker) */
    float group_masking[8];     /* 0..1: part of the group's loudness hidden by everything else */
    float group_duck_db[8];     /* masking-aware cut on the group's bed in the bands it occupies, dB <= 0 */
    uint32_t soloists;          /* foreground sources now */
    uint32_t soloist_source[4]; /* their asmr_get_sources indices (flight: 64 + slot), UINT32_MAX = none */
    float soloist_salience[4];  /* 0..1 */
    float soloist_masking[4];   /* 0..1 */
    uint32_t phrases;           /* bird phrases sounding now */
    uint32_t max_phrases;       /* their limit (0 = none) */
    float masking_before;       /* soloists' masked share without / with ducking, 0..1 */
    float masking_after;
    float smr_before_db;        /* soloists' signal-to-mask ratio in their bands without / with ducking, dB */
    float smr_after_db;
} AsmrMixInfo;

/* Any thread. */
void asmr_get_mix(const AsmrEngine *engine, AsmrMixInfo *out);

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
