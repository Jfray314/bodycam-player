//! RC cars (perk slot): BP_RC_Car_Base with BP_RCCar_Trataxes / BP_RCCar_Kyoshin, driven by ChaosWheeledVehicleMovementComponent.
//! Input (IMC_Vehicle): W IA_Throttle, S IA_Brake (reverse when stopped), A/D IA_Steering, Space IA_Handbrake, Mouse2D IA_RightStick
//! (FPV camera look, whose yaw also steers; air control when no wheel touches); LMB GA_ExplodePerk, RMB GA_PerkJump, F leave, C light.
//! Deploy: Fire = A_FP_<Car>_Deploy; GA_LaunchPerk's A_FP_GoPro_PerkDeploy_Montage (rate 1.5) sends Deploy.Finished at 0.556 s ->
//! BP_PerkProxy.SpawnPerk at the held item's transform, possessed at once.
// ponytail: Chaos is approximated by a raycast car (4 suspension rays, plane-fit chassis, bicycle-model yaw, friction-limited grip);
// effective mass = the movement component's default Mass 1500 kg (the BeginPlay 4 kg Root override would make JumpForce 900000
// a 2 km/s launch). Leaving destroys the car (BP_PerkProxy re-possesses a live one in range). WBP_RC_Car_Interface is not ported:
// the drone HUD shows battery / speed / distance. Handbrake = rear grip x0.3; reverse ratio 4.0 (Chaos default).
use crate::audio::{wav_path, PlayCue};
use crate::items::{Detonate, DRONE_BLAST};
use crate::perks::{Perk, Phase};
use crate::player::{BodycamCamera, IgnoreTraces, Player};
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;

const MASS: f32 = 1500.0;
const G: f32 = 9.8;
const TORQUE: f32 = 100.0; // EngineSetup TorqueCurve (flat 100 Nm)
const MAX_RPM: f32 = 4000.0;
const GEARS: [f32; 4] = [2.4, 1.8, 1.4, 1.2]; // TransmissionSetup ForwardGearRatios
const FINAL: f32 = 3.6;
const CHANGE_UP: f32 = 3200.0;
const REVERSE: f32 = 4.0;
const ENGINE_BRAKE: f32 = 1.0; // EngineBrakeEffect 10000 (clamped to the full torque)
const BRAKE_TORQUE: f32 = 1500.0; // ChaosVehicleWheel MaxBrakeTorque default
const HANDBRAKE_TORQUE: f32 = 3000.0; // MaxHandBrakeTorque default (rear: bAffectedByHandbrake)
const FRONT_STEER: f32 = 50.0; // front wheels: MaxSteerAngle default (bAffectedBySteering)
const STEER_RISE: f32 = 2.5; // SteeringInputRate RiseRate / FallRate
const STEER_FALL: f32 = 5.0;
const JUMP_DV: f32 = 900_000.0 / MASS / 100.0; // GA_PerkJump AddImpulse(Up x JumpForce) kg cm/s -> m/s
const JUMP_COOLDOWN: f32 = 2.0; // GE_PerkJump_Cooldown
const ROTATION_POWER: f32 = 6000.0; // Rotation Power Rate (air control deg/s per unit input per frame-second)
const BIG_HIT: f32 = 5000.0; // Big Hit Sensibility (NormalImpulse kg cm/s)
const HIT_LOW: f32 = 10_000.0;
const HIT_MAX: f32 = 800_000.0;
const MAX_RANGE: f32 = 40.0; // MaxRange 4000 cm
const BATTERY_RATE: f32 = 2.0; // BatteryDepletionRate (% / s)
const DEPLOY_FINISHED: f32 = 0.556_318_8 / 1.5;
/// Kyoshin steering curve (MPH -> steering scale); Trataxes' is empty (constant 1).
const KYOSHIN_STEER: [(f32, f32); 7] = [(0.0, 1.0), (16.737_362, 0.816_195_7), (40.909_86, 0.728_027), (69.7609, 0.667_701), (97.832_184, 0.644_498_7), (122.004_684, 0.630_577_3), (150.855_71, 0.616_655_95)];

/// Per-car wheel / chassis data (BP_Front_Wheel(_Trataxes) / BP_Rear_Wheel(_Trataxes), WheelSetups AdditionalOffset z 5 cm).
#[derive(Clone, Copy)]
struct Spec {
    radius: f32,
    friction: f32,
    raise: f32,
    drop: f32,
}
fn spec(model: &str) -> Spec {
    match model {
        "Trataxes" => Spec { radius: 0.15, friction: 2.5, raise: 0.07, drop: 0.0 },
        _ => Spec { radius: 0.10, friction: 6.0, raise: 0.03, drop: 0.03 },
    }
}

pub fn is_car(model: &str) -> bool {
    matches!(model, "Trataxes" | "Kyoshin")
}

/// Fire montage length + the Deploy.Finished notify of the PerkDeploy montage.
pub fn finish_time(model: &str) -> f32 {
    (if model == "Trataxes" { 1.233_333_3 } else { 0.9 }) + DEPLOY_FINISHED
}

#[derive(Component)]
pub struct Car {
    model: &'static str,
    vel: Vec3,
    steer: f32,
    /// FPV camera (IA_RightStick on FPVCamera's relative rotation): yaw +-85, pitch -60..10 deg.
    cam: Vec2,
    /// NewVar: camera yaw mapped to a steering offset.
    cam_steer: f32,
    forward: bool,
    jump_cd: f32,
    hit_gate: f32,
    prev_vy: f32,
    grounded: bool,
    /// Wheel anchors in the car frame (front_l, front_r, rear_l, rear_r) once the glb is in.
    wheels: Option<[Vec3; 4]>,
}

#[derive(Component)]
struct CarModel;
/// One RC-Car_Driving_LOOP_Cue layer (looping wave + gain/pitch from the cue graph), or RC-Car_Steering_Outside_Cue.
#[derive(Component)]
struct Layer(usize);

pub struct RcCarPlugin;
impl Plugin for RcCarPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (frame, tag, drive, sound).chain()).add_systems(Update, camera.after(crate::player::bodycam_camera));
    }
}

/// BP_PerkProxy.SpawnPerk: the car at the held model's transform (it drops from the hand).
pub fn spawn(cmd: &mut Commands, assets: &AssetServer, model: &'static str, at: &GlobalTransform, cam: &GlobalTransform) {
    let (yaw, _, _) = cam.to_scale_rotation_translation().1.to_euler(EulerRot::YXZ);
    let pos = at.translation();
    cmd.spawn((
        Car { model, vel: Vec3::ZERO, steer: 0.0, cam: Vec2::ZERO, cam_steer: 0.0, forward: true, jump_cd: 0.0, hit_gate: 0.0, prev_vy: 0.0, grounded: false, wheels: None },
        Transform::from_translation(pos).with_rotation(Quat::from_rotation_y(yaw)),
        Visibility::default(),
        Name::new(model),
    ))
    .with_children(|c| {
        c.spawn((WorldAssetRoot(assets.load(GltfAssetLabel::Scene(0).from_asset(format!("models/guns/{model}.glb")))), Transform::default(), CarModel, IgnoreTraces));
    });
    // RC-Car_Driving_LOOP_Cue layers + RC-Car_Steering_Outside_Cue (ReceiveControllerChanged: Activate)
    for (i, w) in LAYERS.iter().enumerate() {
        cmd.spawn((Layer(i), AudioPlayer::<AudioSource>(assets.load(wav_path(w))), PlaybackSettings::LOOP.with_volume(bevy::audio::Volume::Linear(0.0))));
    }
}

/// Wave per layer: Idling_10 (idle), Driving_Slow_Back (reverse drive), Idle_Loop (forward drive), WindCamera / WindMicSpeed (speed),
/// Idling_10 again for the steering servo cue.
const LAYERS: [&str; 6] = [
    "/Game/RCCar/SFX/wavs/RC-Car_Idling_10_wav",
    "/Game/RCCar/SFX/AlexAudio/Audio/Bodycam_Sfx_RCCar_Driving_Slow_Back_Loop_V3",
    "/Game/RCCar/SFX/wavs/RC-Car_Idle_Loop",
    "/Game/Audio/Player/Foley/Bodycam/Gopro/Bodycam_Wind/Bodycam_Foley_Player_Bodycam_WindCamera_Loop",
    "/Game/Audio/Player/Foley/Bodycam/Gopro/Bodycam_Wind/Bodycam_Foley_Player_Bodycam_WindMicSpeed_Loop",
    "/Game/RCCar/SFX/wavs/RC-Car_Idling_10_wav",
];

/// Car frame from the wheel joints (glb axes vary per import): forward = front - rear, left = left - right.
fn frame(
    mut cars: Query<(Entity, &mut Car, &GlobalTransform, &Children)>,
    mut models: Query<&mut Transform, With<CarModel>>,
    joints: Query<(&Name, &GlobalTransform)>,
    kids: Query<&Children>,
) {
    for (_, mut car, root, children) in &mut cars {
        if car.wheels.is_some() {
            continue;
        }
        let Some(m) = children.iter().find(|c| models.contains(*c)) else { continue };
        let names = ["wheel_front_l", "wheel_front_r", "wheel_rear_l", "wheel_rear_r"];
        let mut found = [None; 4];
        for d in kids.iter_descendants(m) {
            if let Ok((n, gt)) = joints.get(d)
                && let Some(i) = names.iter().position(|x| *x == n.as_str())
            {
                found[i] = Some(gt.translation());
            }
        }
        let Some(w) = found.iter().copied().collect::<Option<Vec<Vec3>>>() else { continue };
        if w[0].distance(w[3]) < 0.05 {
            continue; // transforms not propagated yet
        }
        let Ok(mut mt) = models.get_mut(m) else { continue };
        // positions relative to the model root, in its unrotated frame
        let inv = root.affine().inverse();
        let p: Vec<Vec3> = w.iter().map(|x| mt.rotation.inverse() * (inv.transform_point3(*x) - mt.translation)).collect();
        let fwd = ((p[0] + p[1]) - (p[2] + p[3])).normalize();
        let left = ((p[0] + p[2]) - (p[1] + p[3])).normalize();
        let up = fwd.cross(left).normalize();
        let left = up.cross(fwd);
        // model axes -> car frame (forward -Z, left -X, up +Y)
        let basis = Quat::from_mat3(&Mat3::from_cols(Vec3::NEG_Z, Vec3::NEG_X, Vec3::Y)) * Quat::from_mat3(&Mat3::from_cols(fwd, left, up)).inverse();
        mt.rotation = basis;
        let local: Vec<Vec3> = p.iter().map(|x| basis * *x + Vec3::Y * 0.05).collect();
        car.wheels = Some([local[0], local[1], local[2], local[3]]);
    }
}

/// The car's meshes never block its own rays.
fn tag(mut cmd: Commands, new: Query<Entity, Added<Mesh3d>>, parents: Query<&ChildOf>, cars: Query<(), With<Car>>) {
    for e in &new {
        let mut x = e;
        while let Ok(p) = parents.get(x) {
            x = p.parent();
            if cars.contains(x) {
                cmd.entity(e).insert(IgnoreTraces);
                break;
            }
        }
    }
}

fn steering_scale(model: &str, mph: f32) -> f32 {
    if model != "Kyoshin" {
        return 1.0;
    }
    let k = &KYOSHIN_STEER;
    let i = k.iter().position(|&(t, _)| t > mph).unwrap_or(k.len());
    if i == 0 {
        k[0].1
    } else if i == k.len() {
        k[k.len() - 1].1
    } else {
        let ((t0, v0), (t1, v1)) = (k[i - 1], k[i]);
        v0 + (v1 - v0) * (mph - t0) / (t1 - t0)
    }
}

#[allow(clippy::too_many_arguments)]
fn drive(
    mut cmd: Commands,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    mut perk: ResMut<Perk>,
    mut q: Query<(Entity, &mut Transform, &mut Car)>,
    mut ray: MeshRayCast,
    ignore: Query<(), With<IgnoreTraces>>,
    player: Single<&GlobalTransform, With<Player>>,
    mut boom: MessageWriter<Detonate>,
    mut crashed: MessageWriter<crate::items::DroneCrashed>,
    mut snd: MessageWriter<PlayCue>,
) {
    let dt = time.delta_secs().min(0.05);
    let Ok((e, mut tf, mut c)) = q.single_mut() else { return };
    let Some(wheels) = c.wheels else { return };
    let s = spec(c.model);
    let driving = perk.phase == Phase::Piloting;
    let filter = |x: Entity| !ignore.contains(x);
    let mut cast = |from: Vec3, dir: Vec3, max: f32| -> Option<(f32, Vec3)> {
        let d = Dir3::new(dir).ok()?;
        ray.cast_ray(Ray3d::new(from, d), &MeshRayCastSettings::default().with_filter(&filter)).first().filter(|(_, h)| h.distance <= max).map(|(_, h)| (h.distance, h.normal.normalize_or_zero()))
    };
    let key = |k: KeyCode| keys.pressed(k) as i8 as f32;
    let (throttle, brake, handbrake) = if driving { (key(KeyCode::KeyW), key(KeyCode::KeyS), keys.pressed(KeyCode::Space)) } else { (0.0, 0.0, false) };
    // IA_RightStick: FPV camera relative rotation; its yaw also steers (NewVar, sign by Forward)
    if driving && motion.delta != Vec2::ZERO {
        let sens = 0.11 * crate::ui::settings::sens_mult();
        c.cam.x = (c.cam.x + motion.delta.x * sens).clamp(-85.0, 85.0);
        c.cam.y = (c.cam.y - motion.delta.y * sens).clamp(-60.0, 10.0);
        let m = c.cam.x / 85.0;
        c.cam_steer = if c.forward { m } else { -m };
    }
    if brake > 0.0 {
        c.forward = false;
    }
    if throttle > 0.0 {
        c.forward = true;
    }
    let raw = if driving { ((key(KeyCode::KeyD) - key(KeyCode::KeyA)) + c.cam_steer).clamp(-1.0, 1.0) } else { 0.0 };
    let rate = if raw.abs() > c.steer.abs() { STEER_RISE } else { STEER_FALL };
    c.steer += (raw - c.steer).clamp(-rate * dt, rate * dt);

    // suspension rays (down the chassis) from each wheel anchor
    let up = tf.rotation * Vec3::Y;
    let len = s.raise + s.drop + s.radius;
    let hits: Vec<Option<(Vec3, Vec3)>> = wheels
        .iter()
        .map(|w| {
            let a = tf.transform_point(*w) + up * s.raise;
            cast(a, -up, len + 0.05).map(|(d, n)| (a - up * d, n))
        })
        .collect();
    let contacts = hits.iter().flatten().count();
    let was = c.grounded;
    c.grounded = contacts > 0;
    c.vel.y -= G * dt;
    if c.grounded {
        // ride height: wheel centres at radius above the hits; chassis tilts to the hit plane
        let pts: Vec<Vec3> = hits.iter().zip(wheels.iter()).map(|(h, w)| h.map(|h| h.0).unwrap_or(tf.transform_point(*w) - up * (s.radius + s.drop))).collect();
        let fwd_v = ((pts[0] + pts[1]) - (pts[2] + pts[3])) * 0.5;
        let left_v = ((pts[0] + pts[2]) - (pts[1] + pts[3])) * 0.5;
        let n = fwd_v.cross(left_v).normalize_or(Vec3::Y);
        let n = if n.y < 0.0 { -n } else { n };
        let target = Quat::from_rotation_arc(up, n) * tf.rotation;
        tf.rotation = tf.rotation.slerp(target, (12.0 * dt).min(1.0));
        let mean_err = hits.iter().zip(wheels.iter()).filter_map(|(h, w)| h.map(|h| (tf.transform_point(*w) - h.0).dot(up) - s.radius)).sum::<f32>() / contacts as f32;
        tf.translation -= up * mean_err * (20.0 * dt).min(1.0);
        let vn = c.vel.dot(n);
        if vn < 0.0 {
            c.vel -= n * vn;
        }
        // drive / brake along the chassis forward
        let fwd = (tf.rotation * Vec3::NEG_Z).reject_from(n).normalize_or_zero();
        let right = fwd.cross(n).normalize_or_zero();
        let u = c.vel.dot(fwd);
        let lat = c.vel.dot(right);
        let wheel_rpm = u.abs() / s.radius * 60.0 / std::f32::consts::TAU;
        let mut force = 0.0;
        if throttle > 0.0 || (brake > 0.0 && u < 0.5) {
            // FrontWheelDrive: first gear under ChangeUpRPM, no torque past MaxRPM (reverse = brake held while stopped)
            let rev = throttle == 0.0;
            let ratio = if rev { REVERSE } else { GEARS.iter().copied().find(|g| wheel_rpm * g * FINAL < CHANGE_UP).unwrap_or(GEARS[3]) };
            if wheel_rpm * ratio * FINAL < MAX_RPM {
                force = TORQUE * ratio * FINAL / s.radius * if rev { -brake } else { throttle };
            }
        } else if u.abs() > 0.05 {
            force = -u.signum() * ENGINE_BRAKE * TORQUE * GEARS[0] * FINAL / s.radius;
        }
        if brake > 0.0 && u > 0.5 {
            force -= 4.0 * BRAKE_TORQUE / s.radius;
        }
        if handbrake && u.abs() > 0.05 {
            force -= u.signum() * 2.0 * HANDBRAKE_TORQUE / s.radius;
        }
        // friction circle: driven front axle carries half the load
        let grip = s.friction * G;
        let a = (force / MASS).clamp(-grip, grip * 0.5);
        let mut u2 = u + a * dt;
        let coasting_or_braking = throttle == 0.0 && (brake == 0.0 || u > 0.5) || handbrake;
        if coasting_or_braking && u2 * u < 0.0 {
            u2 = 0.0; // brakes / engine brake stop the car, they do not reverse it
        }
        // lateral grip kills sideways slide (handbrake: rear grip drop -> drift)
        let lat_grip = grip * if handbrake { 0.3 } else { 1.0 };
        let lat2 = lat - lat.signum() * (lat_grip * dt).min(lat.abs());
        c.vel = fwd * u2 + right * lat2 + n * c.vel.dot(n);
        // bicycle-model yaw, limited by lateral grip
        let wheelbase = (wheels[0] - wheels[2]).length().max(0.1);
        let delta = (c.steer * FRONT_STEER * steering_scale(c.model, u.abs() * 2.237)).to_radians();
        let mut yaw_rate = u2 * delta.tan() / wheelbase;
        if u2.abs() > 0.1 {
            yaw_rate = yaw_rate.clamp(-lat_grip / u2.abs(), lat_grip / u2.abs());
        }
        tf.rotate_axis(Dir3::new(n).unwrap_or(Dir3::Y), -yaw_rate * dt);
    } else if driving {
        // airborne: IA_RightStick -> SetPhysicsAngularVelocityInDegrees(Mesh(dt x -Y x rate, 0, dt x X x rate))
        let inp = motion.delta * 0.02;
        let w = Vec3::new(-inp.y, 0.0, inp.x) * dt * ROTATION_POWER;
        tf.rotation = tf.rotation * Quat::from_euler(EulerRot::XYZ, (w.x * dt).to_radians(), (-w.z * dt).to_radians(), 0.0);
    }
    // chassis box sweep along the motion (Box extent ~45 cm long): walls stop the car; big hits play RC-Car_Crashing_Short_Cue
    c.hit_gate -= dt;
    let step = c.vel * dt;
    let reach = (wheels[0] - wheels[2]).length() * 0.5 + s.radius;
    let centre = tf.translation + up * (s.radius + 0.03);
    if let Some((d, n)) = cast(centre, step, step.length() + reach) {
        let n = if n.dot(c.vel) > 0.0 { -n } else { n };
        let into = -c.vel.dot(n);
        if into > 0.0 {
            let impulse = into * MASS * 100.0;
            if impulse > BIG_HIT && c.hit_gate <= 0.0 {
                c.hit_gate = 0.4;
                let vol = ((impulse - HIT_LOW) / (HIT_MAX - HIT_LOW)).clamp(0.0, 1.0) * 0.25 + 0.05;
                let dist = player.translation().distance(tf.translation) * 100.0;
                snd.write(PlayCue { bools: vec![], cue: "/Game/RCCar/SFX/cues/RC-Car_Crashing_Short_Cue".into(), dist_cm: dist / vol.sqrt() });
            }
            c.vel += n * into;
            tf.translation += step.normalize_or_zero() * (d - reach).max(0.0);
        } else {
            tf.translation += step;
        }
    } else {
        tf.translation += step;
    }
    // landing: vertical speed crosses from < -40 cm/s into +-40
    let vy = c.vel.y;
    if c.prev_vy < -0.4 && (-0.4..0.4).contains(&vy) && !was && c.grounded {
        snd.write(PlayCue { bools: vec![], cue: "/Game/RCCar/SFX/AlexAudio/Cue/RCCar_Land_Cue".into(), dist_cm: 0.0 });
    }
    c.prev_vy = vy;
    perk.speed = c.vel.length();
    perk.dist = tf.translation.distance(player.translation());
    if !driving {
        return;
    }
    c.jump_cd -= dt;
    if mouse.just_pressed(MouseButton::Right) && c.jump_cd <= 0.0 {
        // GA_PerkJump: impulse along the chassis up; GameplayCue.Perk.RcCar.Jump
        c.jump_cd = JUMP_COOLDOWN;
        c.vel += up * JUMP_DV;
        snd.write(PlayCue { bools: vec![], cue: "/Game/RCCar/SFX/AlexAudio/Cue/RCCar_Jump_Cue".into(), dist_cm: 0.0 });
    }
    perk.battery -= BATTERY_RATE * dt;
    if mouse.just_pressed(MouseButton::Left) {
        // GA_ExplodePerk (AbilitySet_RcCar PrimaryAttack)
        boom.write(Detonate { at: tf.translation, spec: DRONE_BLAST });
        crashed.write(crate::items::DroneCrashed { at: tf.translation });
        cmd.entity(e).despawn();
        perk.wants_exit = true;
    } else if perk.dist > MAX_RANGE {
        perk.wants_exit = true; // GetDistanceFromPlayer IsBeyondMaxRange
    }
}

/// RC-Car_Driving_LOOP_Cue: ParamCrossFade on RPM (= |MPH|) and Forward (MPH clamped -1..1), Modulator / ModulatorContinuous
/// (pitch 0.8..1.5, volume 0.7..1 over RPM 0..9), cue volume 0.7 pitch 1.3; RC-Car_Steering_Outside_Cue volume / pitch = |steering|.
fn sound(mut cmd: Commands, cars: Query<&Car>, perk: Res<Perk>, mut layers: Query<(Entity, &Layer, &mut AudioSink)>) {
    let Ok(c) = cars.single() else {
        for (e, ..) in &layers {
            cmd.entity(e).despawn();
        }
        return;
    };
    let _ = perk;
    let mph = c.vel.length() * 2.237 * if c.forward { 1.0 } else { -1.0 };
    let rpm = mph.abs();
    let fwd = mph.clamp(-1.0, 1.0);
    let lin = |x: f32, a: f32, b: f32| ((x - a) / (b - a)).clamp(0.0, 1.0);
    // PCF_3 Forward: reverse branch (fades out 0..1) at pitch 0.9, forward branch (fades in 0..1)
    let rev_w = 1.0 - lin(fwd, 0.0, 1.0);
    let pitch_dir = 0.9 * rev_w + (1.0 - rev_w);
    // PCF_0 RPM: idle (full at 0, out by 5), drive (in 0..1), wind (in 3..6)
    let idle = 1.0 - lin(rpm, 0.0, 5.0);
    let drive = lin(rpm, 0.0, 1.0);
    let wind = lin(rpm, 3.0, 6.0);
    let m = lin(rpm, 0.0, 9.0);
    let (mod_pitch, mod_vol) = (0.8 + 0.7 * m, 0.7 + 0.3 * m);
    let back = 1.0 - lin(fwd, 0.0, 1.0);
    for (_, l, mut s) in &mut layers {
        let (v, p) = match l.0 {
            0 => (idle * 0.5, 1.0),
            1 => (drive * mod_vol * back, mod_pitch * 0.8),
            2 => (drive * mod_vol * (1.0 - back), mod_pitch),
            3 => (wind, 1.0),
            4 => (wind * 2.0, 1.0),
            _ => (0.5 * c.steer.abs(), 1.4 * c.steer.abs().max(0.05)),
        };
        let (v, p) = if l.0 < 5 { (v * 0.7, p * pitch_dir * 1.3) } else { (v, p) };
        s.set_volume(bevy::audio::Volume::Linear(v));
        s.set_speed(p.max(0.05));
    }
}

/// While driving the BodycamCamera sits on the FPV camera part (FPVCameraHDMona + its Viewpoint socket), with the IA_RightStick
/// look and CS_RCDriveShake (Perlin rot pitch/roll 0.3, yaw 0.1 deg at 5 Hz, scale MapRangeClamped(|speed| 0..500 cm/s -> 0..2)).
fn camera(
    perk: Res<Perk>,
    cars: Query<(Entity, &Transform, &Car), Without<BodycamCamera>>,
    nodes: Query<(&Name, &GlobalTransform)>,
    kids: Query<&Children>,
    player: Single<&GlobalTransform, With<Player>>,
    mut cam: Single<&mut Transform, With<BodycamCamera>>,
    time: Res<Time>,
) {
    if perk.phase != Phase::Piloting {
        return;
    }
    let Ok((e, tf, c)) = cars.single() else { return };
    let up = tf.rotation * Vec3::Y;
    let at = kids.iter_descendants(e).filter_map(|d| nodes.get(d).ok()).find(|(n, _)| n.as_str() == "FPVCameraHDMona").map(|(_, g)| g.translation()).unwrap_or(tf.translation + up * 0.15);
    let k = (c.vel.length() / 5.0).clamp(0.0, 1.0) * 2.0;
    let t = time.elapsed_secs() * 5.0 * std::f32::consts::TAU;
    let shake = Quat::from_euler(EulerRot::YXZ, (0.1 * k * (t * 0.83).sin()).to_radians(), (0.3 * k * t.sin()).to_radians(), (0.3 * k * (t * 1.13).cos()).to_radians());
    let rot = tf.rotation * Quat::from_euler(EulerRot::YXZ, -c.cam.x.to_radians(), c.cam.y.to_radians(), 0.0) * shake;
    let world = Transform::from_translation(at + rot * Vec3::new(0.0, 0.0, -0.018)).with_rotation(rot);
    let local = Mat4::from(player.affine().inverse()) * world.to_matrix();
    let (_, r, tr) = local.to_scale_rotation_translation();
    cam.translation = tr;
    cam.rotation = r;
}

#[cfg(test)]
mod tests {
    #[test]
    fn steering_curve() {
        assert_eq!(super::steering_scale("Kyoshin", 0.0), 1.0);
        assert!((super::steering_scale("Kyoshin", 200.0) - 0.616_655_95).abs() < 1e-6);
        assert!((super::steering_scale("Kyoshin", 8.368_681) - 0.908_097_85).abs() < 1e-4);
        assert_eq!(super::steering_scale("Trataxes", 50.0), 1.0);
        // top speed: MaxRPM in 4th gear (Kyoshin r 10 cm -> 9.7 m/s)
        let v = super::MAX_RPM / (super::GEARS[3] * super::FINAL) / 60.0 * std::f32::consts::TAU * 0.10;
        assert!((v - 9.696).abs() < 0.01);
    }
}
