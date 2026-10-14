/* bodycam-core C ABI. Hand-written, keep in sync with src/lib.rs (#[repr(C)] structs). */
#ifndef BODYCAM_CORE_H
#define BODYCAM_CORE_H
#include <stdint.h>
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

#ifdef __cplusplus
}
#endif
#endif
