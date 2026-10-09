//! Port of Bodycam_Player (ALS_Base_CharacterBP child) movement + AC_BodycamCamera.
//! Movement constants are the live CharacterMovementComponent values (UE4SS dump, cm -> m).
//! Camera motion is played back from curves measured in-game (assets/camera_motion.json, tools/probe_curves.py).
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};
use crate::weapon::Fired;
use rand::Rng;
use serde::Deserialize;

const WALK: f32 = 1.25; // MaxWalkSpeed; measured walk 125 cm/s
const SPRINT: f32 = 3.25; // measured sprint 325 cm/s
const CROUCH: f32 = 1.00; // measured crouch-walk 100 cm/s
const ADS: f32 = 1.00; // measured aim-walk 100 cm/s
const MAX_ACCEL: f32 = 6.2089; // MaxAcceleration 620.89
const BRAKING: f32 = 10.0; // BrakingDecelerationWalking 1000
const GROUND_FRICTION: f32 = 4.0;
const JUMP_VEL: f32 = 3.0; // JumpZVelocity 300, AirControl 0
const GRAVITY: f32 = 9.8; // GravityScale 1
const HALF_HEIGHT: f32 = 0.90; // capsule; crouch-segment z drop (35 cm) + CrouchedHalfHeight
const CROUCH_HALF_HEIGHT: f32 = 0.55; // CrouchedHalfHeight 55
const SENS: f32 = 0.0022;
// ponytail: lean blend speed eyeballed; measure rise time from the lean sequence if it feels off.
const LEAN_SPEED: f32 = 6.0;

#[derive(Deserialize)]
struct Loop {
    period_s: f32,
    curve: Vec<[f32; 6]>,
}
#[derive(Deserialize)]
struct Event {
    pre_s: f32,
    series: Vec<[f32; 6]>,
}
#[derive(Deserialize)]
struct Seq {
    series: Vec<[f32; 6]>,
}
#[derive(Deserialize)]
struct Loops {
    walk: Loop,
    sprint: Loop,
    crouch_walk: Loop,
    ads_walk: Loop,
}
#[derive(Deserialize)]
struct Events {
    land: Event,
}
#[derive(Deserialize)]
struct Seqs {
    lean: Seq,
}
/// Camera view relative to (capsule centre, control rotation): x right, y up, z back (m); pitch, yaw, roll (deg).
#[derive(Deserialize, Resource)]
pub struct CameraMotion {
    hz: f32,
    idle: [f32; 6],
    crouch_idle: [f32; 6],
    loops: Loops,
    events: Events,
    sequences: Seqs,
}

impl CameraMotion {
    fn load() -> Self {
        serde_json::from_str(include_str!("../../assets/camera_motion.json")).expect("camera_motion.json")
    }
    /// Peak lean pose (one side) taken from the recorded lean sequence.
    fn lean_pose(&self) -> [f32; 6] {
        let s = &self.sequences.lean.series;
        *s.iter().max_by(|a, b| a[5].abs().total_cmp(&b[5].abs())).unwrap()
    }
}

fn sample_loop(l: &Loop, phase: f32) -> [f32; 6] {
    let n = l.curve.len();
    let f = phase.rem_euclid(1.0) * n as f32;
    let (i, a) = (f as usize % n, f.fract());
    let (p, q) = (l.curve[i], l.curve[(i + 1) % n]);
    std::array::from_fn(|k| p[k] + (q[k] - p[k]) * a)
}

fn add(a: &mut [f32; 6], b: [f32; 6], w: f32) {
    for k in 0..6 {
        a[k] += b[k] * w;
    }
}

/// Gait step / landing, replaces the animation footstep notifies. `kind as i32` is the cues' `FootstepType` switch value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum StepKind {
    Crouch = 1,
    Walk = 2,
    Run = 3,
    Land = 4,
}

#[derive(Message, Clone, Copy, Debug)]
pub struct Footstep {
    pub kind: StepKind,
    pub stride_start: bool,
}

pub struct PlayerPlugin;
impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CameraMotion::load())
            .add_message::<Footstep>()
            .add_systems(Startup, spawn)
            .add_systems(Update, (grab_cursor, on_fire, look, movement, bodycam_camera).chain());
    }
}

#[derive(Component, Default)]
pub struct Player {
    pub yaw: f32,
    pub pitch: f32,
    pub vel: Vec3,
    pub grounded: bool,
    pub crouched: bool,
    pub sprinting: bool,
    pub aiming: bool,
    /// -1 left .. 1 right, blended.
    pub lean: f32,
    /// Smoothed capsule centre height above the feet.
    center: f32,
    /// Stride phase (0..1) and how much of the current gait loop is applied.
    phase: f32,
    /// Seconds since landing (drives the measured land curve); large = inactive.
    since_land: f32,
    /// PRAS ControllerRecoil still to apply (deg; x = yaw right, y = pitch up) and its RecoilSmoothing.
    recoil_pending: Vec2,
    recoil_smoothing: f32,
    /// Active Shake_Rifle_Standard instances: (age s, pitch/yaw/roll amplitude deg), and the smoothed output.
    shakes: Vec<(f32, Vec3)>,
    shake: Vec3,
}

#[derive(Component)]
pub struct BodycamCamera;

fn spawn(mut cmd: Commands) {
    cmd.spawn((
        Player { grounded: true, center: HALF_HEIGHT, since_land: 99.0, ..default() },
        Transform::default(),
        Visibility::default(),
    ))
    .with_children(|p| {
        // FOV is owned by postfx::apply_fov (120 horizontal, from the live CameraComponent).
        p.spawn((BodycamCamera, Camera3d::default(), Projection::Perspective(PerspectiveProjection { near: 0.02, ..default() }), Transform::default()));
    });
}

fn grab_cursor(mouse: Res<ButtonInput<MouseButton>>, keys: Res<ButtonInput<KeyCode>>, mut cursor: Single<&mut CursorOptions>) {
    if mouse.just_pressed(MouseButton::Left) {
        cursor.grab_mode = CursorGrabMode::Locked;
        cursor.visible = false;
    }
    if keys.just_pressed(KeyCode::Escape) {
        cursor.grab_mode = CursorGrabMode::None;
        cursor.visible = true;
    }
}

fn look(time: Res<Time>, motion: Res<AccumulatedMouseMotion>, cursor: Single<&CursorOptions>, mut q: Single<&mut Player>) {
    let p = &mut **q;
    let d = if cursor.grab_mode == CursorGrabMode::Locked { motion.delta } else { Vec2::ZERO };
    // PRAS ControllerRecoil: the per-shot step reaches the view through RecoilSmoothing (interp speed).
    // ponytail: ControllerRecoil.Damping (recovery) not applied; Bodycam leaves recoil for the player to pull down.
    let step = p.recoil_pending * (p.recoil_smoothing * time.delta_secs()).min(1.0);
    p.recoil_pending -= step;
    p.yaw -= d.x * SENS + step.x.to_radians();
    p.pitch = (p.pitch - d.y * SENS + step.y.to_radians()).clamp(-89.9f32.to_radians(), 89.9f32.to_radians()); // ViewPitchMin/Max
}

/// Shake_Rifle_Standard (BafCameraShake): rotation curve * random amplitude per shot, PlayRate 2, Smoothing 35.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct FireShake {
    #[serde(rename = "pitch")]
    pitch: Range2,
    yaw: Range2,
    roll: Range2,
    smoothing: f32,
    play_rate: f32,
    rotation_curve: ShakeCurve,
}
#[derive(Deserialize)]
struct Range2 {
    x: f32,
    y: f32,
}
#[derive(Deserialize)]
struct ShakeCurve {
    curves: ShakeCurves,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct ShakeCurves {
    float_curves: Vec<Vec<crate::curve::Key>>,
}

fn on_fire(mut ev: MessageReader<Fired>, mut q: Single<&mut Player>) {
    let mut rng = rand::rng();
    let shake: &FireShake = &FIRE_SHAKE;
    for f in ev.read() {
        let c = &f.controller;
        q.recoil_pending += Vec2::new(rng.random_range(c.horizontal.0..=c.horizontal.1), rng.random_range(c.vertical.0..=c.vertical.1));
        q.recoil_smoothing = c.smoothing;
        let r = |r: &Range2, rng: &mut rand::rngs::ThreadRng| rng.random_range(r.x.min(r.y)..=r.x.max(r.y));
        let amp = Vec3::new(r(&shake.pitch, &mut rng), r(&shake.yaw, &mut rng), r(&shake.roll, &mut rng));
        q.shakes.push((0.0, amp));
    }
}

static FIRE_SHAKE: std::sync::LazyLock<FireShake> =
    std::sync::LazyLock::new(|| serde_json::from_str(include_str!("../../assets/fire_shake.json")).expect("fire_shake.json"));

/// Sum of active fire shakes in UE convention (pitch up, yaw right, roll cw), smoothed.
fn fire_shake(p: &mut Player, dt: f32) -> Vec3 {
    let s: &FireShake = &FIRE_SHAKE;
    let c = &s.rotation_curve.curves.float_curves;
    let mut target = Vec3::ZERO;
    for (age, amp) in &mut p.shakes {
        *age += dt;
        let t = *age * s.play_rate;
        target += Vec3::new(crate::curve::eval(&c[0], t), crate::curve::eval(&c[1], t), crate::curve::eval(&c[2], t)) * *amp;
    }
    let end = c.iter().filter_map(|k| k.last()).map(|k| k[0]).fold(0.0, f32::max) / s.play_rate;
    p.shakes.retain(|(age, _)| *age < end);
    p.shake += (target - p.shake) * (s.smoothing * dt).min(1.0);
    p.shake
}

/// UCharacterMovementComponent::CalcVelocity (walking) with Bodycam's values, plus jump/fall.
fn movement(time: Res<Time>, keys: Res<ButtonInput<KeyCode>>, mut q: Single<(&mut Player, &mut Transform)>, mut steps: MessageWriter<Footstep>) {
    let (p, tf) = &mut *q;
    let dt = time.delta_secs();
    let input = Vec2::new(
        keys.pressed(KeyCode::KeyD) as i8 as f32 - keys.pressed(KeyCode::KeyA) as i8 as f32,
        keys.pressed(KeyCode::KeyW) as i8 as f32 - keys.pressed(KeyCode::KeyS) as i8 as f32,
    )
    .normalize_or_zero();
    p.crouched = keys.pressed(KeyCode::ControlLeft); // IA_Crouch: LeftControl, InputTriggerDown (hold)
    // GA_Character_Sprint: forward only, blocked by aim/crouch (GE_BlockSprint).
    p.sprinting = keys.pressed(KeyCode::ShiftLeft) && input.y > 0.0 && !p.crouched && !p.aiming;
    let max = if p.crouched { CROUCH } else if p.sprinting { SPRINT } else if p.aiming { ADS } else { WALK };
    let lean_target = keys.pressed(KeyCode::KeyE) as i8 as f32 - keys.pressed(KeyCode::KeyQ) as i8 as f32;
    p.lean += (lean_target - p.lean) * (LEAN_SPEED * dt).min(1.0);

    let rot = Quat::from_rotation_y(p.yaw);
    let mut v = Vec3::new(p.vel.x, 0.0, p.vel.z);
    if p.grounded {
        let accel = rot * Vec3::new(input.x, 0.0, -input.y) * MAX_ACCEL;
        if accel.length_squared() > 0.0 {
            // Friction steers velocity toward the input direction, then accelerate, clamp to max speed.
            let dir = accel.normalize();
            v -= (v - dir * v.length()) * (dt * GROUND_FRICTION).min(1.0);
            v = (v + accel * dt).clamp_length_max(max);
        } else {
            // BrakingFrictionFactor 0 -> only BrakingDecelerationWalking.
            let speed = v.length();
            v = if speed > BRAKING * dt { v * (1.0 - BRAKING * dt / speed) } else { Vec3::ZERO };
        }
        if keys.just_pressed(KeyCode::Space) && !p.crouched {
            p.vel.y = JUMP_VEL;
            p.grounded = false;
        }
    }
    p.vel.x = v.x;
    p.vel.z = v.z;
    p.vel.y -= GRAVITY * dt;
    tf.translation += p.vel * dt;
    // ponytail: flat ground at y=0 only; add a collision crate when real geometry exists.
    if tf.translation.y <= 0.0 {
        if !p.grounded {
            p.since_land = 0.0;
            steps.write(Footstep { kind: StepKind::Land, stride_start: false });
        }
        tf.translation.y = 0.0;
        p.vel.y = 0.0;
        p.grounded = true;
    }
    p.since_land += dt;
    tf.rotation = rot;
}

/// AC_BodycamCamera replacement: measured idle pose + gait loops + land event + lean, on the chest mount.
fn bodycam_camera(time: Res<Time>, cm: Res<CameraMotion>, mut player: Single<&mut Player>, mut cam: Single<&mut Transform, With<BodycamCamera>>, mut steps: MessageWriter<Footstep>) {
    let p = &mut **player;
    let dt = time.delta_secs();
    let target_center = if p.crouched { CROUCH_HALF_HEIGHT } else { HALF_HEIGHT };
    p.center += (target_center - p.center) * (12.0 * dt).min(1.0);

    let speed = Vec2::new(p.vel.x, p.vel.z).length();
    let (gait, ref_speed) = if p.crouched {
        (&cm.loops.crouch_walk, CROUCH)
    } else if p.sprinting {
        (&cm.loops.sprint, SPRINT)
    } else if p.aiming {
        (&cm.loops.ads_walk, ADS)
    } else {
        (&cm.loops.walk, WALK)
    };
    let w = if p.grounded { (speed / ref_speed).min(1.0) } else { 0.0 };
    let before = p.phase;
    p.phase += dt / gait.period_s * w;
    // One measured loop = one stride = two steps (phase crosses 0.0 and 0.5); stands in for the animation notifies.
    if w > 0.3 && (p.phase * 2.0).floor() > (before * 2.0).floor() {
        let kind = if p.crouched { StepKind::Crouch } else if p.sprinting { StepKind::Run } else { StepKind::Walk };
        steps.write(Footstep { kind, stride_start: p.phase.floor() > before.floor() });
    }

    let crouch_a = ((HALF_HEIGHT - p.center) / (HALF_HEIGHT - CROUCH_HALF_HEIGHT)).clamp(0.0, 1.0);
    let mut c: [f32; 6] = std::array::from_fn(|k| cm.idle[k] + (cm.crouch_idle[k] - cm.idle[k]) * crouch_a);
    add(&mut c, sample_loop(gait, p.phase), w);
    let land = &cm.events.land;
    let i = ((p.since_land + land.pre_s) * cm.hz) as usize;
    if p.since_land < 5.0 && i < land.series.len() {
        add(&mut c, land.series[i], 1.0);
    }
    // Recorded lean peaks are signed; mirror for the other side.
    let lp = cm.lean_pose();
    let side = if lp[5] < 0.0 { 1.0 } else { -1.0 }; // positive lean = right = negative (cw) roll
    add(&mut c, lp.map(|v| v * side), p.lean);

    // Fire shake is in UE convention: pitch up+, yaw right+ (-> Bevy left+), roll cw+ (-> Bevy ccw+).
    let fs = fire_shake(p, dt);
    add(&mut c, [0.0, 0.0, 0.0, fs.x, -fs.y, -fs.z], 1.0);

    cam.translation = Vec3::new(c[0], p.center + c[1], c[2]);
    cam.rotation = Quat::from_euler(EulerRot::YXZ, c[4].to_radians(), p.pitch + c[3].to_radians(), c[5].to_radians());
}

#[cfg(test)]
mod tests {
    #[test]
    fn camera_motion_loads() {
        let cm = super::CameraMotion::load();
        assert!(cm.loops.walk.curve.len() == 64 && cm.loops.walk.period_s > 0.5);
        assert!(cm.idle[1] > 0.6 && cm.idle[1] < 0.8); // chest mount ~72 cm above capsule centre
        assert!(cm.lean_pose()[5].abs() > 20.0); // measured lean roll ~29 deg
    }

    #[test]
    fn fire_shake_loads() {
        let s: &super::FireShake = &super::FIRE_SHAKE;
        assert_eq!(s.rotation_curve.curves.float_curves.len(), 3);
        assert_eq!(s.play_rate, 2.0);
    }
}
