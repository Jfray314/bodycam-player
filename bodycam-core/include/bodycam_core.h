/* bodycam-core C ABI. Hand-written, keep in sync with src/lib.rs (#[repr(C)] structs). */
#ifndef BODYCAM_CORE_H
#define BODYCAM_CORE_H
#include <stdint.h>
#include <stddef.h>
#ifdef __cplusplus
extern "C" {
#endif

/* BcInput.flags */
#define BC_GROUNDED     (1u << 0)
#define BC_CROUCHED     (1u << 1)
#define BC_SPRINTING    (1u << 2)
#define BC_AIMING       (1u << 3)
#define BC_TRIGGER_HELD (1u << 4)
#define BC_LEAN_PRESSED (1u << 5)
#define BC_LANDED       (1u << 6) /* set on the one frame the host character touched the ground */

typedef struct BcCore BcCore;

typedef struct BcInput {
    float dt;             /* seconds */
    float look_yaw_deg;   /* player look this frame, right = + */
    float look_pitch_deg; /* up = + */
    float move_x;         /* strafe input -1 (left) .. 1 (right) */
    float speed;          /* horizontal speed, metres/s */
    float lean_target;    /* -1 (left) .. 1 (right); host does wall checks */
    uint32_t flags;
} BcInput;

/* Unreal convention: metres, x right, y up, z back; degrees, pitch up+, yaw right+, roll clockwise+. */
typedef struct BcOutput {
    float pos[3];       /* camera offset from the feet (y includes capsule centre height) */
    float rot[3];       /* pitch, yaw, roll offset relative to the control rotation, fire shake included */
    float base_pos[3];  /* same without fire shake (viewmodel attach pose) */
    float base_rot[3];
    float ctl_yaw_deg;  /* control rotation after look + recoil */
    float ctl_pitch_deg;
    float ctl_delta_yaw_deg;   /* change this frame (look + recoil) */
    float ctl_delta_pitch_deg;
    float lean;         /* current lean -1..1 */
    int32_t footstep;   /* 0 none, 1 crouch, 2 walk, 3 run, 4 land */
    int32_t stride_start; /* 1 if this footstep starts a stride */
} BcOutput;

BcCore* bc_create(uint64_t seed);                              /* NULL on failure */
void bc_destroy(BcCore*);
void bc_set_control(BcCore*, float yaw_deg, float pitch_deg);  /* sync from the host (spawn, teleport) */
/* One shot: controller recoil steps (deg), RecoilSmoothing, Damping, Compensation. Call before bc_step. */
void bc_fire(BcCore*, float h_min, float h_max, float v_min, float v_max, float smoothing, float damping, float compensation);
void bc_step(BcCore*, const BcInput*, BcOutput*);

/* ---------------- weapon data (host reads the JSON files itself and passes the text) ----------------
 * stats  = assets/weapon_stats.json, weapon = assets/weapons/<Row>.json. All loaders return NULL on error. */
typedef struct BcStats BcStats;
typedef struct BcWeapon BcWeapon;
BcStats*  bc_stats_load(const char* weapon_stats_json);
void      bc_stats_free(BcStats*);
BcWeapon* bc_weapon_load(const BcStats*, const char* weapon_json);
void      bc_weapon_free(BcWeapon*);

typedef struct BcWeaponInfo {
    float damage, fire_rate;       /* fire_rate = seconds between shots */
    uint32_t pellets, ammo_capacity, mag_amount, full_auto, infinite_mags;
    float spread_hip[2], spread_aim[2]; /* max yaw, max pitch in degrees */
    float instability;
} BcWeaponInfo;
void bc_weapon_info(const BcWeapon*, BcWeaponInfo*);
void bc_weapon_controller_recoil(const BcWeapon*, float out7[7]); /* the 7 args of bc_fire, in order */
/* Cue asset path from the weapon JSON ("Fire", "FireSuppressed", "FireDry", "BulletCrack", "BulletWizz").
 * Copies into buf (NUL terminated, truncated); returns the full length, -1 if absent. */
int bc_weapon_sound(const BcWeapon*, const char* kind, char* buf, size_t cap);

/* ---------------- viewmodel sway + ADS ---------------- */
typedef struct BcSway BcSway;
typedef struct BcSwayIn {
    float dt, look_yaw_deg, look_pitch_deg; /* look this frame; yaw right+, pitch up+ */
    float vel_fwd, vel_right;               /* character velocity in its own frame, m/s */
    float cam_pitch_rad;
    uint32_t aiming, has_pivot;
    float pivot_m[3];                       /* used when has_pivot */
} BcSwayIn;
typedef struct BcSwayOut {
    float translation[3];  /* arms/gun root in camera space: p' = R * p + translation (metres; x right, y up, z back) */
    float rot_deg[3];      /* pitch, yaw, roll */
    float matrix[9];       /* R, row major */
    float ads_alpha;       /* 0 hip .. 1 aimed (eased); blend your ADS pose with it */
} BcSwayOut;
BcSway* bc_sway_create(void);            /* make a new one when the weapon changes */
void bc_sway_destroy(BcSway*);
void bc_sway_step(BcSway*, const BcWeapon*, const BcSwayIn*, BcSwayOut*);

/* ---------------- weapon recoil animation (gun kick) ---------------- */
typedef struct BcRecoil BcRecoil;
typedef struct BcRecoilOut {
    float matrix[9], translation[3]; /* p' = M * p + t, camera space metres */
    float out_rot[3], out_loc[3];    /* PRAS-style outputs: rot pitch,yaw,roll deg; loc fwd,right,up cm */
} BcRecoilOut;
BcRecoil* bc_recoil_create(uint64_t seed);
void bc_recoil_destroy(BcRecoil*);
void bc_recoil_play(BcRecoil*, const BcWeapon*, uint32_t aiming); /* call once per shot */
void bc_recoil_tick(BcRecoil*, float dt, BcRecoilOut*);           /* call every frame */

/* ---------------- fire control + spread ---------------- */
typedef struct BcGun BcGun;
#define BC_TRIG_HELD    1u  /* fire button down */
#define BC_TRIG_PRESSED 2u  /* went down this frame */
#define BC_TRIG_BLOCKED 4u  /* reloading / drawing / sprinting / dead */
#define BC_TRIG_AIMING  8u
typedef struct BcFireOut {
    uint32_t fired;       /* a round left the gun: apply recoil, play the shot, call bc_gun_dirs */
    uint32_t dry;         /* empty click */
    float regrip_delay;   /* < 0 none, else start a regrip animation after this many seconds */
} BcFireOut;
BcGun* bc_gun_create(const BcWeapon*, uint32_t ammo, uint64_t seed);
void   bc_gun_destroy(BcGun*);
void   bc_gun_set_ammo(BcGun*, uint32_t ammo); /* after reload */
uint32_t bc_gun_ammo(const BcGun*);
void   bc_gun_step(BcGun*, float dt, float now_s, uint32_t trig_flags, BcFireOut*);
/* Spread: one direction per pellet around the unit aim_dir, 3 floats each, up to max_pellets. Returns the pellet count. */
uint32_t bc_gun_dirs(BcGun*, const float aim_dir[3], uint32_t aiming, float* out_dirs, uint32_t max_pellets);

/* ---------------- ballistics (host raycasts, core decides what the hits do) ---------------- */
float bc_bone_multiplier(const char* bone);   /* head/neck 10, spine_01 2.5, spine_02/03 4, else 1 */
float bc_surface_absorbance(uint8_t physical_surface);
/* Hits in trace order. Writes (hit index, damage multiplier at that hit) for every hit the bullet reaches; returns the count. */
uint32_t bc_penetrate(const uint64_t* actor_ids, const float* absorbance, uint32_t n, uint32_t* out_idx, float* out_mult, uint32_t cap);
float bc_hit_damage(float base_damage, float mult, const char* bone); /* base * mult * bone multiplier */

/* ---------------- audio cue selection ---------------- */
typedef struct BcCues BcCues;
BcCues* bc_cues_load(const char* audio_json);  /* reads the "footsteps" and "weapons" tables */
void    bc_cues_free(BcCues*);
/* kind = BcOutput.footstep (1 crouch 2 walk 3 run 4 land); surface = physical surface index or -1.
 * Cue names are '\n' separated: surface cue first, then foley layers. Returns the full length. */
int bc_cues_footstep(const BcCues*, int kind, uint32_t stride_start, int surface, char* buf, size_t cap);
int bc_cues_weapon(const BcCues*, const char* weapon, const char* kind, char* buf, size_t cap); /* -1 if none */

/* ---------------- lens (shared with mods/reshade/Bodycam.fx) ---------------- */
typedef struct BcLensParams {
    float distortion, distortion_density, ca, vignette, aberration_fade_exponent, edge_blur, blur_fade_exponent, sharpen,
          grain, temporal_noise, banding, dirt, dirt_threshold, dirt_gain, exposure_bias, rolling_shutter;
} BcLensParams;
void bc_lens_defaults(uint32_t preset, BcLensParams*);  /* 0 game (HDR), 1 ReShade file defaults */
typedef struct BcLensMotion BcLensMotion;
BcLensMotion* bc_lens_motion_create(void);
void bc_lens_motion_destroy(BcLensMotion*);
/* w = camera angular velocity in camera-local (pitch, yaw, roll) rad/s; out3 = yaw_rate, pitch_rate, pump (shader inputs). */
void bc_lens_motion_step(BcLensMotion*, float dt, const float w[3], float out3[3]);
float bc_vertical_fov_deg(float horizontal_fov_deg, float aspect);

#ifdef __cplusplus
}
#endif
#endif
