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
/// Bodycam_Player "Update Leaning Values": Leaning Alpha = FInterpTo(Alpha, Target, dt, 12).
const LEAN_SPEED: f32 = 12.0;
/// Measured lean cascade after Leaning Alpha (assets/sway_reference.json): camera offset / NormalizedLean 8, camera roll 2 on top of that.
const LEAN_POS_SPEED: f32 = 8.0;
const LEAN_ROLL_SPEED: f32 = 2.0;
/// AC_BodycamCamera.Yaw(DeltaTime) = yaw rate (deg/s) * this (measured, corr 0.998, no lag).
const YAW_DELTA_PER_DEG_S: f32 = 0.00796;
/// Camera roll (cam_roll in the recordings) follows FInterp(1.77) of strafe + look terms; the BP's FInterp 8 alone is 2-3x too fast.
// ponytail: single empirical stage (look gain 2.59 covers the missing arm/camera lag); the BP structure behind it isn't decoded.
const ROLL_SPEED: f32 = 1.77;
const ROLL_LOOK_GAIN: f32 = 2.59;

/// Yaw rate (deg/s, right +) -> AC_BodycamCamera Yaw(DeltaTime).
pub fn yaw_delta_time(deg_per_s: f32) -> f32 {
    deg_per_s * YAW_DELTA_PER_DEG_S
}

/// Spring Camera roll target (UE deg, cw +): Movement.X (-1..1) -> -5..5 plus Yaw(DeltaTime) (-3..3) -> 8..-8.
pub fn spring_roll_target(move_x: f32, yaw_delta: f32) -> f32 {
    let map = |v: f32, a: f32, b: f32, c: f32, d: f32| c + (d - c) * ((v - a) / (b - a)).clamp(0.0, 1.0);
    map(move_x, -1.0, 1.0, -5.0, 5.0) + ROLL_LOOK_GAIN * map(yaw_delta, -3.0, 3.0, 8.0, -8.0)
}

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
            .add_systems(Update, (grab_cursor, on_fire, look, movement, lean, bodycam_camera).chain());
    }
}

#[derive(Component, Default)]
pub struct Player {
    pub yaw: f32,
    pub pitch: f32,
    pub vel: Vec3,
    pub grounded: bool,
    /// effects.rs death: input is ignored.
    pub dead: bool,
    /// perks.rs: the camera is on the drone, the body ignores input.
    pub locked: bool,
    pub crouched: bool,
    pub sprinting: bool,
    pub aiming: bool,
    /// -1 left .. 1 right, blended.
    pub lean: f32,
    /// Lean as the camera shows it (measured in-game, camera_motion.json sequences.lean): the offset follows
    /// Leaning Alpha through FInterp 8 (LEAN_POS_SPEED), the roll trails that at 2 (29.5 deg reached after ~1.5 s).
    lean_pos: f32,
    lean_roll: f32,
    /// Smoothed capsule centre height above the feet.
    center: f32,
    /// Stride phase (0..1) and how much of the current gait loop is applied.
    phase: f32,
    /// Seconds since landing (drives the measured land curve); large = inactive.
    since_land: f32,
    /// PRAS ControllerRecoil still to apply (deg; x = yaw right, y = pitch up) and its RecoilSmoothing.
    ctl_recoil: CtlRecoil,
    /// Active Shake_Rifle_Standard instances: (age s, pitch/yaw/roll amplitude deg), and the smoothed output.
    shakes: Vec<(f32, Vec3)>,
    shake: Vec3,
    /// Bodycam_Player "Smooth Control Rotation": RInterpTo(control rotation, 25) - the arms + camera mount follow this.
    smooth_yaw: f32,
    smooth_pitch: f32,
    /// UE Yaw(DeltaTime) = Look.X / dt / 50 (Look.X taken as degrees of yaw input this frame).
    pub yaw_rate: f32,
    /// IA_Move X (strafe, right +), Q/E held, and the Spring Camera roll (UE deg, cw +), see ROLL_SPEED.
    move_x: f32,
    lean_pressed: bool,
    roll: f32,
    /// Camera pose in player space from body motion only (no bob/shake); the viewmodel attaches here.
    pub view_base: Transform,
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
        p.spawn((BodycamCamera, bevy::camera::visibility::RenderLayers::from_layers(&[0, crate::attach::VM_LAYER]), Camera3d::default(), Projection::Perspective(PerspectiveProjection { near: 0.02, ..default() }), Transform::default()));
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

/// UE APlayerController Add{Pitch,Yaw}Input multiplies by the default Input{Pitch,Yaw}Scale of 2.5; measured
/// ctl_pitch per M4A1 shot is 0.65 deg for a 0.2..0.3 step (probe, rec_1791597946).
const CTL_INPUT_SCALE: f32 = 2.5;

/// PRAS ControllerRecoil: the per-shot step reaches the view through RecoilSmoothing (interp speed). The applied
/// offset returns to zero at Damping (rate/s) only once the trigger is released (measured: pitch climbs linearly
/// through a held burst, then decays e^-15t; a semi click decays ~0.05 s after the shot). Player input against it
/// (Compensation 0..1) is credited to the offset so the recovery doesn't pull past where the player corrected.
/// No Bodycam gun sets Compensation; 1 (full credit) is measured: rec_1791908511, M4A1 ADS burst of ~30 shots (~19 deg of offset)
/// pulled down by hand ends 3 deg below the start, and releasing the trigger moves the view < 1 deg (0 credit would drop ~19 deg).
#[derive(Default)]
pub struct CtlRecoil {
    pending: Vec2,
    offset: Vec2,
    smoothing: f32,
    damping: f32,
    comp: f32,
}

impl CtlRecoil {
    pub fn fire(&mut self, c: &crate::weapon::ControllerRecoil, rng: &mut impl Rng) {
        let r = |(a, b): (f32, f32), rng: &mut dyn rand::RngCore| if a == b { a } else { rng.random_range(a..=b) };
        self.pending += Vec2::new(r(c.horizontal, rng), r(c.vertical, rng)) * CTL_INPUT_SCALE;
        self.smoothing = c.smoothing;
        (self.damping, self.comp) = (c.damping, c.compensation);
    }

    /// View delta (deg; x yaw right, y pitch up) this frame. `input` = player look this frame in the same convention.
    pub fn tick(&mut self, dt: f32, input: Vec2, trigger_held: bool) -> Vec2 {
        let step = self.pending * (self.smoothing * dt).min(1.0);
        self.pending -= step;
        self.offset += step;
        for i in 0..2 {
            if input[i] * self.offset[i] < 0.0 {
                let c = (input[i].abs() * self.comp).min(self.offset[i].abs());
                self.offset[i] -= c * self.offset[i].signum();
            }
        }
        let ret = if trigger_held { Vec2::ZERO } else { self.offset * (self.damping * dt).min(1.0) };
        self.offset -= ret;
        step - ret
    }
}

fn look(time: Res<Time>, motion: Res<AccumulatedMouseMotion>, mouse: Res<ButtonInput<MouseButton>>, cursor: Single<&CursorOptions>, fake: Res<crate::sway::FakeLook>, mut q: Single<&mut Player>) {
    let p = &mut **q;
    if p.dead || p.locked {
        return;
    }
    let sens = SENS * crate::ui::settings::sens_mult(); // Settings > Game > Mouse Sensitivity
    let d = if cursor.grab_mode == CursorGrabMode::Locked { motion.delta } else { Vec2::ZERO } + fake.0; // + scripted look (dev.rs)
    let input = Vec2::new(d.x, -d.y) * sens.to_degrees(); // same sign convention as the recoil step
    let step = p.ctl_recoil.tick(time.delta_secs(), input, mouse.pressed(MouseButton::Left));
    p.yaw -= d.x * sens + step.x.to_radians();
    p.yaw_rate = yaw_delta_time((d.x * sens).to_degrees() / time.delta_secs().max(1e-4));
    p.pitch = (p.pitch - d.y * sens + step.y.to_radians()).clamp(-89.9f32.to_radians(), 89.9f32.to_radians()); // ViewPitchMin/Max
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
        q.ctl_recoil.fire(c, &mut rng);
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
fn movement(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut q: Single<(&mut Player, &mut Transform)>,
    inv: Option<Single<&crate::weapon::Inventory>>,
    mut steps: MessageWriter<Footstep>,
    map: Option<Res<crate::map::MapState>>,
    geo: Query<(), With<crate::map::MapGeometry>>,
    mut ray: bevy::picking::mesh_picking::ray_cast::MeshRayCast,
    grid: Option<Res<crate::grid::Colliders>>,
) {
    let (p, tf) = &mut *q;
    let none = ButtonInput::<KeyCode>::default();
    let keys = if p.dead || p.locked { &none } else { &*keys }; // dead / piloting a perk: no input
    if map.as_ref().is_some_and(|m| !m.ready()) {
        return; // level still spawning: hold the player so they do not fall through unloaded geometry
    }
    let dt = time.delta_secs();
    let input = Vec2::new(
        keys.pressed(KeyCode::KeyD) as i8 as f32 - keys.pressed(KeyCode::KeyA) as i8 as f32,
        keys.pressed(KeyCode::KeyW) as i8 as f32 - keys.pressed(KeyCode::KeyS) as i8 as f32,
    )
    .normalize_or_zero();
    p.move_x = input.x;
    p.crouched = keys.pressed(KeyCode::ControlLeft); // IA_Crouch: LeftControl, InputTriggerDown (hold)
    // GA_Character_Sprint: forward only, blocked by aim/crouch (GE_BlockSprint).
    // ALS CanSprint (LookingDirection mode): input amount > 0.5 and move direction within 100 deg of the view;
    // not while aiming (Aiming rotation mode) or crouched (AllowedGait). GA_Character_Sprint needs acceleration.
    let within_100 = input.length() > 0.5 && input.x.atan2(input.y).to_degrees().abs() < 100.0;
    // GA_Reload_Base.TryBlockingSprint: cancels sprint and applies GE_BlockSprint for the whole reload.
    let reloading = inv.is_some_and(|i| i.action.as_ref().is_some_and(|a| a.kind == crate::weapon::ActionKind::Reload));
    p.sprinting = keys.pressed(KeyCode::ShiftLeft) && within_100 && !p.crouched && !p.aiming && !reloading;
    let max = if p.crouched { CROUCH } else if p.sprinting { SPRINT } else if p.aiming { ADS } else { WALK };

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
    let before = tf.translation;
    tf.translation += p.vel * dt;
    // Map level: ray-cast collision. Test grid: AABB list (walls, steps, ramps). Grey-box fallback: flat ground at y=0.
    let on_ground = if let Some(g) = &grid {
        let mut pos = tf.translation;
        let was = p.grounded;
        let hit = g.collide(before.y, &mut pos, &mut p.vel, was);
        tf.translation = pos;
        hit
    } else if map.is_some() {
        let mut pos = tf.translation;
        let was = p.grounded;
        let hit = crate::map::collide(&mut ray, &geo, before, &mut pos, &mut p.vel, was);
        tf.translation = pos;
        hit
    } else {
        tf.translation.y <= 0.0
    };
    if on_ground {
        if !p.grounded {
            p.since_land = 0.0;
            steps.write(Footstep { kind: StepKind::Land, stride_start: false });
        }
        if map.is_none() && grid.is_none() {
            tf.translation.y = 0.0;
        }
        p.vel.y = 0.0;
        p.grounded = true;
    } else {
        p.grounded = false;
    }
    p.since_land += dt;
    tf.rotation = rot;
}

/// Entities the player's own traces ignore (lens housing plane, tracers, viewmodel).
#[derive(Component)]
pub struct IgnoreTraces;

/// Bodycam_Player "Check Automatic Leaning" + "Update Leaning Values", run every tick.
fn lean(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mut q: Single<(&mut Player, &Transform)>,
    cam: Single<&GlobalTransform, With<BodycamCamera>>,
    ignore: Query<(), With<IgnoreTraces>>,
    mut ray: bevy::picking::mesh_picking::ray_cast::MeshRayCast,
) {
    use bevy::picking::mesh_picking::ray_cast::MeshRayCastSettings;
    let (p, tf) = &mut *q;
    let filter = |e: Entity| !ignore.contains(e);
    let settings = MeshRayCastSettings::default().with_filter(&filter);
    let mut cast = |from: Vec3, dir: Vec3, max: f32| -> Option<f32> {
        let d = Dir3::new(dir).ok()?;
        ray.cast_ray(Ray3d::new(from, d), &settings).first().map(|(_, h)| h.distance).filter(|&dist| dist <= max)
    };
    let center = tf.translation + Vec3::Y * p.center;
    let right = tf.rotation * Vec3::X;
    let forward = tf.rotation * Vec3::NEG_Z; // "Get Forward Rotation": control yaw forward
    let head = cam.translation(); // ponytail: camera stands in for the 'head' socket until the body mesh exists
    // BlockLeft/RightLean: SphereTraceSingle radius 20 from head, 25 cm sideways. ponytail: ray of 25 + 20 cm.
    let block_right = cast(head, right, 0.45).is_some();
    let block_left = cast(head, -right, 0.45).is_some();

    let pressed = keys.pressed(KeyCode::KeyE) as i8 as f32 - keys.pressed(KeyCode::KeyQ) as i8 as f32; // IA_Lean
    let target = if pressed != 0.0 {
        pressed
    } else {
        // Five forward traces each side, offset i * Corner Peek Division (10 cm), from (actor xy, camera z).
        // Length by gait: walking 300 cm (SelectFloat always picks 300), sprinting 100 cm. Hits right count +1, left -1; keep the nearest wall.
        let len = if p.sprinting { 1.0 } else { 3.0 };
        let start = Vec3::new(center.x, cam.translation().y, center.z);
        let (mut hits, mut wall) = (0i32, 3.0f32);
        for side in [1.0f32, -1.0] {
            for i in 1..=5 {
                if let Some(d) = cast(start + right * side * 0.10 * i as f32, forward, len)
                    && d < wall
                {
                    hits += side as i32;
                    wall = d;
                }
            }
        }
        let value = 15.0 / 5.0 * hits as f32; // |Leaning Camera Offset Rotation.X| / 5 * TraceIndexHit
        let map = |v: f32, a: f32, b: f32, c: f32, d: f32| c + (d - c) * ((v - a) / (b - a)).clamp(0.0, 1.0);
        if p.sprinting {
            map(value, -15.0, 15.0, 1.0, -1.0)
        } else if p.aiming {
            0.0 // no automatic leaning while aiming
        } else {
            map(value * map(wall * 100.0, 100.0, 300.0, 1.0, 0.0), -15.0, 15.0, 1.0, -1.0)
        }
    };
    p.lean_pressed = pressed != 0.0;
    let blocked = (target < 0.0 && block_left) || (target > 0.0 && block_right);
    p.lean = crate::status::finterp_to(p.lean, if blocked { 0.0 } else { target }, time.delta_secs(), LEAN_SPEED);
}

/// AC_BodycamCamera replacement: measured idle pose + gait loops + land event + lean, on the chest mount.
pub(crate) fn bodycam_camera(time: Res<Time>, cm: Res<CameraMotion>, mut player: Single<&mut Player>, mut cam: Single<&mut Transform, With<BodycamCamera>>, mut steps: MessageWriter<Footstep>) {
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
    // Measured (tools/probe_sway.py): alpha FInterp 12 -> FInterp 8 = NormalizedLean and camera offset (0.16 cm rms), roll one more FInterp 2.
    p.lean_pos = crate::status::finterp_to(p.lean_pos, p.lean, dt, LEAN_POS_SPEED);
    p.lean_roll = crate::status::finterp_to(p.lean_roll, p.lean_pos, dt, LEAN_ROLL_SPEED);
    add(&mut c, [lp[0], lp[1], lp[2], 0.0, 0.0, 0.0].map(|v| v * side), p.lean_pos);
    add(&mut c, [0.0, 0.0, 0.0, lp[3], lp[4], lp[5]].map(|v| v * side), p.lean_roll);
    // UE hangs the camera off the arms' FP_Camera socket, so bob/land/lean move arms and camera together;
    // only camera shakes (fire) are camera-only. The arms follow this pose.
    // Smooth Control Rotation (Bodycam_Player tick): RInterpTo 25 drives SpringArms (arms + camera mount); the body keeps
    // the raw yaw, so the camera gets the lag as a local yaw offset. Snap on large jumps (spawn/teleport).
    if (p.yaw - p.smooth_yaw).abs() > 1.0 {
        p.smooth_yaw = p.yaw;
    }
    p.smooth_yaw = crate::status::finterp_to(p.smooth_yaw, p.yaw, dt, 25.0);
    p.smooth_pitch = crate::status::finterp_to(p.smooth_pitch, p.pitch, dt, 25.0);
    let lag_yaw = p.smooth_yaw - p.yaw;
    // while aiming / holding lean the BP uses the lean rotation instead, which our measured lean roll already covers.
    let roll_target = if p.aiming || p.lean_pressed { 0.0 } else { spring_roll_target(p.move_x, p.yaw_rate) };
    p.roll = crate::status::finterp_to(p.roll, roll_target, dt, ROLL_SPEED);
    c[5] -= p.roll; // UE roll cw+ -> Bevy ccw+
    p.view_base = Transform::from_xyz(c[0], p.center + c[1], c[2])
        .with_rotation(Quat::from_euler(EulerRot::YXZ, lag_yaw + c[4].to_radians(), p.smooth_pitch + c[3].to_radians(), c[5].to_radians()));

    // Fire shake is in UE convention: pitch up+, yaw right+ (-> Bevy left+), roll cw+ (-> Bevy ccw+).
    let fs = fire_shake(p, dt);
    add(&mut c, [0.0, 0.0, 0.0, fs.x, -fs.y, -fs.z], 1.0);

    cam.translation = Vec3::new(c[0], p.center + c[1], c[2]);
    cam.rotation = Quat::from_euler(EulerRot::YXZ, lag_yaw + c[4].to_radians(), p.smooth_pitch + c[3].to_radians(), c[5].to_radians());
}

#[cfg(test)]
mod tests {
    /// Full compensation: pulling down against a held burst is credited, so releasing the trigger doesn't drop the view.
    #[test]
    fn compensated_burst_does_not_recover_past_the_pull() {
        let c = crate::weapon::ControllerRecoil { horizontal: (0.0, 0.0), vertical: (0.25, 0.25), smoothing: 50.0, damping: 15.0, compensation: 1.0 };
        let (mut r, mut rng, dt) = (super::CtlRecoil::default(), rand::rng(), 1.0 / 60.0);
        let mut view = 0.0;
        for f in 0..180 {
            if f % 5 == 0 && f < 150 {
                r.fire(&c, &mut rng);
            }
            let pull = if f < 150 { -0.25 * super::CTL_INPUT_SCALE / 5.0 } else { 0.0 }; // the player's hand matches the climb
            view += r.tick(dt, glam_vec(0.0, pull), f < 150).y + pull;
        }
        let at_release = view;
        for _ in 0..60 {
            view += r.tick(dt, glam_vec(0.0, 0.0), false).y;
        }
        assert!((view - at_release).abs() < 0.5, "release moved the view {} deg", view - at_release);
        // Without credit the release pulls back the whole ~19 deg climb.
        let mut r0 = super::CtlRecoil::default();
        let c0 = crate::weapon::ControllerRecoil { compensation: 0.0, ..c };
        let (mut v0, mut at0) = (0.0, 0.0);
        for f in 0..240 {
            if f % 5 == 0 && f < 150 {
                r0.fire(&c0, &mut rng);
            }
            let pull = if f < 150 { -0.25 * super::CTL_INPUT_SCALE / 5.0 } else { 0.0 };
            v0 += r0.tick(dt, glam_vec(0.0, pull), f < 150).y + pull;
            if f == 149 {
                at0 = v0;
            }
        }
        assert!(v0 - at0 < -10.0, "{}", v0 - at0);
    }
    fn glam_vec(x: f32, y: f32) -> bevy::math::Vec2 {
        bevy::math::Vec2::new(x, y)
    }

    #[test]
    fn camera_motion_loads() {
        let cm = super::CameraMotion::load();
        assert!(cm.loops.walk.curve.len() == 64 && cm.loops.walk.period_s > 0.5);
        assert!(cm.idle[1] > 0.6 && cm.idle[1] < 0.8); // chest mount ~72 cm above capsule centre
        assert!(cm.lean_pose()[5].abs() > 20.0); // measured lean roll ~29 deg
    }

    /// Lean step response: alpha FInterp 12 -> offset FInterp 8 -> roll FInterp 2 against the in-game recording
    /// (assets/sway_reference.json: NormalizedLean, camera right offset cm, camera roll deg after a Q/E press).
    #[test]
    fn lean_matches_recording() {
        let r: serde_json::Value = serde_json::from_str(include_str!("../../assets/sway_reference.json")).unwrap();
        let s = &r["lean"]["series_60hz"];
        let col = |k: &str| -> Vec<f32> { s[k].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect() };
        let (nl, off, roll) = (col("normalized_lean"), col("camera_right_cm"), col("cam_roll"));
        let (off0, roll0) = (off[0], roll[0]);
        let press = (s["press_offset_s"].as_f64().unwrap() * 60.0).round() as usize;
        let dt = 1.0 / 60.0;
        let (mut alpha, mut pos, mut rl) = (0.0f32, 0.0f32, 0.0f32);
        let (mut e_nl, mut e_off, mut e_roll, mut n) = (0.0f32, 0.0f32, 0.0f32, 0);
        for i in 0..nl.len() {
            if i >= press {
                alpha = crate::status::finterp_to(alpha, -1.0, dt, super::LEAN_SPEED);
                pos = crate::status::finterp_to(pos, alpha, dt, super::LEAN_POS_SPEED);
                rl = crate::status::finterp_to(rl, pos, dt, super::LEAN_ROLL_SPEED);
            }
            // peaks: NormalizedLean 15 deg-units/15, offset 24 cm, roll 30.2 deg (recorded, measured peak sign -)
            e_nl += (pos - nl[i]).powi(2);
            e_off += (24.0 * pos - (off[i] - off0)).powi(2);
            e_roll += (30.2 * rl - (roll[i] - roll0)).powi(2);
            n += 1;
        }
        let rms = |e: f32| (e / n as f32).sqrt();
        eprintln!("lean rms: normalized {:.4}, offset {:.3} cm, roll {:.3} deg", rms(e_nl), rms(e_off), rms(e_roll));
        assert!(rms(e_nl) < 0.02 && rms(e_off) < 0.6 && rms(e_roll) < 0.8);
    }

    /// Spring Camera roll against cam_roll of the strafe / look recordings. `before` is the previous model (Yaw(DeltaTime) =
    /// rate / 50 and FInterp 8), `after` the fitted one.
    #[test]
    fn camera_roll_matches_recording() {
        let r: serde_json::Value = serde_json::from_str(include_str!("../../assets/sway_reference.json")).unwrap();
        let map = |v: f32, a: f32, b: f32, c: f32, d: f32| c + (d - c) * ((v - a) / (b - a)).clamp(0.0, 1.0);
        let dt = 1.0 / 60.0;
        let (mut before, mut after, mut n, mut sd) = (0.0f32, 0.0f32, 0, 0.0f32);
        for (_file, wins) in r["roll"]["series_60hz"].as_object().unwrap() {
            for w in wins.as_array().unwrap() {
                let col = |k: &str| -> Vec<f32> { w[k].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect() };
                let (yaw, mx, roll) = (col("ctl_yaw"), col("move_x"), col("cam_roll"));
                let (mut old, mut new) = (roll[0], roll[0]);
                for i in 1..yaw.len() {
                    let rate = (yaw[i] - yaw[i - 1]) / dt;
                    // old: yaw_rate = rate / 50, target map(+-5) + map(8..-8), FInterp 8
                    let t_old = map(mx[i], -1.0, 1.0, -5.0, 5.0) + map(rate / 50.0, -3.0, 3.0, 8.0, -8.0);
                    old = crate::status::finterp_to(old, t_old, dt, 8.0);
                    new = crate::status::finterp_to(new, super::spring_roll_target(mx[i], super::yaw_delta_time(rate)), dt, super::ROLL_SPEED);
                    if i > 30 {
                        before += (old - roll[i]).powi(2);
                        after += (new - roll[i]).powi(2);
                        sd += roll[i].powi(2);
                        n += 1;
                    }
                }
            }
        }
        let rms = |e: f32| (e / n as f32).sqrt();
        eprintln!("camera roll rms deg: before {:.3}, after {:.3} (signal rms {:.3})", rms(before), rms(after), rms(sd));
        assert!(rms(after) < 0.75 * rms(before) && rms(after) < 0.75 * rms(sd));
    }

    /// Headless: step `movement` in a minimal App (no window/render); walking reaches WALK, sprinting SPRINT, then stops.
    #[test]
    fn movement_speeds() {
        use bevy::prelude::*;
        use bevy::time::TimeUpdateStrategy;
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(TimeUpdateStrategy::ManualDuration(std::time::Duration::from_millis(16)))
            .init_resource::<ButtonInput<KeyCode>>()
            .init_resource::<Assets<Mesh>>() // movement ray-casts the map (none here: flat ground)
            .add_message::<super::Footstep>()
            .add_systems(Update, super::movement);
        let e = app.world_mut().spawn((super::Player { grounded: true, ..default() }, Transform::default())).id();
        let run = |app: &mut App, keys: &[KeyCode], secs: f32| {
            let mut k = app.world_mut().resource_mut::<ButtonInput<KeyCode>>();
            k.reset_all();
            keys.iter().for_each(|&c| k.press(c));
            for _ in 0..(secs / 0.016) as i32 {
                app.update();
            }
            app.world().get::<super::Player>(e).unwrap().vel.length()
        };
        app.update(); // first frame has dt 0
        assert!((run(&mut app, &[KeyCode::KeyW], 3.0) - super::WALK).abs() < 0.01);
        assert!((run(&mut app, &[KeyCode::KeyW, KeyCode::ShiftLeft], 3.0) - super::SPRINT).abs() < 0.01);
        assert!(run(&mut app, &[], 1.0) < 0.01);
        assert!(app.world().get::<Transform>(e).unwrap().translation.z < -5.0); // moved forward (-Z)
    }

    #[test]
    fn fire_shake_loads() {
        let s: &super::FireShake = &super::FIRE_SHAKE;
        assert_eq!(s.rotation_curve.curves.float_curves.len(), 3);
        assert_eq!(s.play_rate, 2.0);
    }

    /// RMS of simulated mean ctl_pitch vs the probe's mean (mouse untouched bursts) for M4A1 hip/aim bursts.
    /// Run: cargo test -p game ctl_recoil_fit -- --nocapture
    #[test]
    fn ctl_recoil_fit() {
        use super::CtlRecoil;
        use bevy::prelude::Vec2;
        use rand::SeedableRng;
        let reference: serde_json::Value = serde_json::from_str(include_str!("../../assets/recoil_reference.json")).unwrap();
        let w: serde_json::Value = serde_json::from_str(include_str!("../../assets/weapons/M4A1.json")).unwrap();
        let cr = crate::weapon::ControllerRecoil::from(&w["anim_set"]);
        for key in ["M4A1/hip/1", "M4A1/aim/1", "M4A1/hip/30"] {
            let b = &reference["bursts"][key];
            let data: Vec<f32> = b["ctl_pitch"]["mean"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
            let times: Vec<f32> = b["times"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect();
            let release = times.last().unwrap() + 0.04; // trigger release a little after the last shot
            let (dt, seeds) = (0.0125f32, 200);
            let mut mean = vec![0.0f32; data.len()];
            for seed in 0..seeds {
                let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
                let (mut c, mut y) = (CtlRecoil::default(), 0.0f32);
                let mut next = 0;
                for j in 0..data.len() {
                    let t = (j as f32 - 2.0) * dt;
                    while next < times.len() && times[next] <= t + 1e-4 {
                        c.fire(&cr, &mut rng);
                        next += 1;
                    }
                    y += c.tick(dt, Vec2::ZERO, t < release).y;
                    mean[j] += y / seeds as f32;
                }
            }
            let n = data.len() - 2;
            let rms = (data[2..].iter().zip(&mean[2..]).map(|(a, b)| (a - b).powi(2)).sum::<f32>() / n as f32).sqrt();
            let sig = (data[2..].iter().map(|a| a * a).sum::<f32>() / n as f32).sqrt();
            println!("ctl_pitch {key}: rms {rms:.3} deg (signal rms {sig:.3})");
            if key.ends_with("/1") {
                assert!(rms < 0.15, "{key}: {rms}");
            }
        }
    }
}
