//! Perk slot (Inventory.Slots.Perk.First): the Kamikaze FPV drone.
//! Flow (A_FP_Drone_Deploy_Montage notifies -> Bodycam_Player.DeployPerk / DetachDrone / GA_LaunchPerk): LMB plays Deploy; at 0 s the
//! goggles + RC controller appear on the arms (goggles/controller deploy clips at 1.5x), at 1.7955 s the held drone detaches and drops,
//! at 3.0354 s ("GameplayEvent.Perk.Drone.Deploy.Finished") the camera cuts to the drone (CameraTransition Disconnected) and the body is
//! locked. BP_FPV_Drone Angle-mode controls (IMC_Drone: WASD = LeftStick, mouse = look, Space/LMB up, Ctrl/RMB down, F leave, C light),
//! DroneMovementComponent MaxSpeed 700 / Acceleration 3500 / Deceleration 1000 cm/s, camera FOV 130. A hit above Crash Sensibility Angl
//! (350) fires GameplayEvent.Perk.Crash -> GA_DroneCrash -> GA_ExplodePerk (200 dmg, inner 200 / outer 1000 cm, exp 2). F = GA_LeavePerk
//! (Drone_Exit montage, back to Primary); battery drain GE_PerkStat_Default IdleBatteryConsumption 0.1 + 0.001 x speed(cm/s) per second,
//! empty = GA_OutOfBattery -> Perk.Leave; GadgetCooldown after use (GE_Perk_Cooldown).
// ponytail: not ported - goggles visual (virtual-bone socket), propeller/rotor spin, drone HUD widget (DroneUI), Acro/Horizon flight modes,
// FPV post-process/night vision, Spectator/Sentinel drones, RC car, bomb (GA_TriggerBomb, Special slot / game-mode item), GoPro PerkDeploy camera montage.
// GA_Flashlight is this drone light (GameplayCue.Perk.RcCar.Flashlight): on foot, C toggles the gun-mounted flashlight attachment (GA_ToggleAttachment_Flashlight), not a helmet light.
use crate::audio::PlayCue;
use crate::items::{Detonate, DRONE_BLAST, SLOT_PERK};
use crate::player::{BodycamCamera, IgnoreTraces, Player};
use crate::viewmodel::{ArmsRoot, GunRoot};
use crate::weapon::{Inventory, ItemKind, MontageStarted};
use bevy::animation::AnimationTargetId;
use bevy::gltf::Gltf;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::picking::mesh_picking::ray_cast::{MeshRayCast, MeshRayCastSettings};
use bevy::prelude::*;
use std::collections::HashMap;
use std::time::Duration;

const MAX_SPEED: f32 = 7.0; // DroneMovementComponent.MaxSpeed 700 cm/s
const ACCEL: f32 = 35.0; // Acceleration 3500
const DECEL: f32 = 10.0; // Deceleration 1000
const RAMP_TIME: f32 = 6.0; // AccelerationRampTime
const FWD_FORCE: f32 = 0.7; // Throttle Forward BackwardForce
const SIDE_FORCE: f32 = 0.5; // Throttle Left Right Force
const VERT_FORCE: f32 = 0.5; // IA_Jump / IA_Crouch AddMovementInput +-0.5
const DRONE_FOV: f32 = 130.0; // BP_FPV_Drone Camera FieldOfView
const CRASH_SPEED: f32 = 3.5; // Crash Sensibility Angl 350 (impulse, mass 1)
const DETACH_T: f32 = 1.7955;
const FINISH_T: f32 = 3.0354;
const COOLDOWN: f32 = 15.0; // ponytail: GadgetCooldown of Kamikaze is not in the dump (Sentinel GE_PerkStat sets 15)
const BATTERY_IDLE: f32 = 0.1;
const BATTERY_SPEED: f32 = 0.001;
const RADIUS: f32 = 0.12;
const G: f32 = 9.8;
/// CurveFloat_DroneAcceleration keys.
const ACC_CURVE: [crate::curve::Key; 2] = [[0.0, 0.0, 2.0, 13.502_299, 13.502_299], [1.0, 1.0, 2.0, 0.079_052_28, 0.079_052_28]];

/// Frame_FPV_Long_Range mesh axes (Blender X = UE X = model left, Blender Y = -UE Y = model back, Z up) -> world (forward -Z).
fn drone_basis() -> Quat {
    Quat::from_mat3(&Mat3::from_cols(Vec3::new(-1.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0), Vec3::new(0.0, 1.0, 0.0)))
}

#[derive(Default, PartialEq, Clone, Copy, Debug)]
pub enum Phase {
    #[default]
    Idle,
    /// Deploy montage running (goggles/controller up, drone dropped at DETACH_T).
    Deploy,
    Piloting,
    /// Drone_Exit montage running.
    Exit,
}

#[derive(Resource, Default)]
pub struct Perk {
    pub phase: Phase,
    t: f32,
    /// Seconds of GadgetCooldown left.
    pub cooldown: f32,
    pub battery: f32,
    light: bool,
    detached: bool,
    wants_exit: bool,
    visuals: Option<(Handle<Gltf>, Handle<Gltf>)>,
    fade: f32,
    fade_to: f32,
}

impl Perk {
    /// Inventory swaps and item keys are blocked while the perk runs.
    pub fn locked(&self) -> bool {
        self.phase != Phase::Idle
    }
}

#[derive(Component)]
struct Drone {
    vel: Vec3,
    yaw: f32,
    pitch: f32,
    flying: bool,
    /// Seconds a movement key has been held (AccelerationRampTime curve).
    hold: f32,
    tilt: Vec2,
    spawn: f32,
}

/// Controller / goggles on the arms, with their own clip graph.
#[derive(Component)]
struct PerkVisual {
    nodes: HashMap<String, AnimationNodeIndex>,
    want: Option<(&'static str, f32)>,
}
#[derive(Component)]
struct PerkAnim(Entity);
#[derive(Component)]
struct DroneLight;
/// Propeller_Drone_Kamikaze: Drone_Idle_Loop at volume 0.4; pitch/volume follow the speed (FC_FPVDrone_SFX curves not decoded).
#[derive(Component)]
struct Motor;
/// The drone's own mesh: hidden from its FPV camera (it sits inside the frame).
#[derive(Component)]
struct DroneBody;
/// Controller meshes get a dark plastic (the cooked skin material is not in the dump: slot High_Poly_Material_001 resolves to nothing).
#[derive(Component)]
struct DarkPlastic;
#[derive(Component)]
struct PerkText;
#[derive(Component)]
struct PerkFade;

pub struct PerksPlugin;
impl Plugin for PerksPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Perk>()
            .add_systems(Startup, (load, ui))
            .add_systems(Update, (deploy_flow, hook_anims, play_anims, tint, drone_fly, hud, fade).chain())
            .add_systems(Update, piloting_camera.after(crate::player::bodycam_camera));
    }
}

fn load(mut perk: ResMut<Perk>, assets: Res<AssetServer>) {
    perk.visuals = Some((assets.load("models/guns/DroneController.glb"), assets.load("models/guns/DroneGoggles.glb")));
}

fn ui(mut cmd: Commands, assets: Res<AssetServer>) {
    cmd.spawn((
        Text::new(""),
        TextFont { font: assets.load("fonts/Ki-Bold.otf").into(), font_size: 26.0.into(), ..default() },
        TextColor(Color::NONE),
        PerkText,
        Node { position_type: PositionType::Absolute, bottom: percent(8), left: percent(36), ..default() },
    ));
    cmd.spawn((Node { position_type: PositionType::Absolute, width: percent(100), height: percent(100), ..default() }, BackgroundColor(Color::NONE), GlobalZIndex(90), PerkFade, bevy::picking::Pickable::IGNORE));
}

fn root_of(mut e: Entity, parents: &Query<&ChildOf>, is: impl Fn(Entity) -> bool) -> Option<Entity> {
    loop {
        if is(e) {
            return Some(e);
        }
        e = parents.get(e).ok()?.parent();
    }
}

/// UE socket (cm, (pitch, yaw, roll)) on a bone -> child transform (Y mirrored like the PSK import: tools/blender/ueblender.attach_parts).
fn socket(loc: [f32; 3], rot: [f32; 3]) -> Transform {
    let (p, y, r) = (rot[0].to_radians(), rot[1].to_radians(), rot[2].to_radians());
    let ue = Quat::from_rotation_z(y) * Quat::from_rotation_y(p) * Quat::from_rotation_x(-r);
    let f = Quat::from_mat3(&Mat3::from_diagonal(Vec3::new(1.0, -1.0, 1.0))); // reflection: conjugate through the mirror
    let m = Mat3::from_diagonal(Vec3::new(1.0, -1.0, 1.0)) * Mat3::from_quat(ue) * Mat3::from_diagonal(Vec3::new(1.0, -1.0, 1.0));
    let _ = f;
    Transform { translation: Vec3::new(loc[0], -loc[1], loc[2]) * 0.01, rotation: Quat::from_mat3(&m), ..default() }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn deploy_flow(
    mut cmd: Commands,
    (time, mouse, keys): (Res<Time>, Res<ButtonInput<MouseButton>>, Res<ButtonInput<KeyCode>>),
    mut perk: ResMut<Perk>,
    mut inv: Single<&mut Inventory>,
    mut p: Single<&mut Player>,
    (gltfs, mut graphs, assets): (Res<Assets<Gltf>>, ResMut<Assets<AnimationGraph>>, Res<AssetServer>),
    (joints, parents, arms): (Query<(Entity, &Name), With<AnimationTargetId>>, Query<&ChildOf>, Query<(), With<ArmsRoot>>),
    (mut guns, mut arms_vis): (Query<(Entity, &GunRoot, &GlobalTransform, &mut Visibility)>, Query<&mut Visibility, (With<ArmsRoot>, Without<GunRoot>)>),
    (drones, visuals, mut visual_cmd): (Query<(Entity, &Transform), With<Drone>>, Query<Entity, With<PerkVisual>>, Query<&mut PerkVisual>),
    cam: Single<&GlobalTransform, With<BodycamCamera>>,
    (mut fx, settings, mut out, mut snd): (ResMut<crate::postfx::PostFxState>, Res<crate::ui::settings::Settings>, MessageWriter<MontageStarted>, MessageWriter<PlayCue>),
    motors: Query<Entity, With<Motor>>,
) {
    let dt = time.delta_secs();
    perk.cooldown = (perk.cooldown - dt).max(0.0);
    let cur = inv.current;
    let on_drone = cur == SLOT_PERK && inv.slots.get(cur).is_some_and(|w| w.kind == ItemKind::Drone);
    match perk.phase {
        Phase::Idle => {
            if on_drone && mouse.just_pressed(MouseButton::Left) && !inv.busy() && !p.dead && perk.cooldown <= 0.0 && inv.slots[cur].ammo > 0 {
                let m = inv.slots[cur].montage("Fire");
                inv.use_item(m, &mut out);
                *perk = Perk { phase: Phase::Deploy, visuals: perk.visuals.take(), battery: 100.0, ..default() };
            }
        }
        Phase::Deploy => {
            let t0 = perk.t;
            perk.t += dt;
            let t1 = perk.t;
            let crossed = |at: f32| t0 < at && t1 >= at;
            if t0 == 0.0 {
                // DeployDrone_C (bIsDeploy) at 0.0001: DeployPerk -> goggles + controller visible, Montage_Play rate 1.5.
                // ponytail: goggles (socket on the arms skeleton's virtual bone 'VB goggles_root', not exported) are not shown
                let Some((hc, _hg)) = perk.visuals.clone() else { return };
                for (h, name, bone, sock, clip) in [
                    (hc, "DroneController", "hand_l", socket([13.695_484, -12.322_571, -1.742_179], [-7.828_202, 95.434_975, -151.7156]), "A_W_Controller_Deploy"),
                ] {
                    let Some(g) = gltfs.get(&h) else { continue };
                    let Some(joint) = joints.iter().find(|(e, n)| n.as_str() == bone && root_of(*e, &parents, |x| arms.contains(x)).is_some()).map(|j| j.0) else { continue };
                    let mut graph = AnimationGraph::new();
                    let nodes: HashMap<String, AnimationNodeIndex> = g.named_animations.iter().map(|(n, c)| (n.to_string(), graph.add_clip(c.clone(), 1.0, graph.root))).collect();
                    let root = cmd
                        .spawn((
                            PerkVisual { nodes, want: Some((clip, 1.5)) },
                            WorldAssetRoot(g.scenes[0].clone()),
                            AnimationGraphHandle(graphs.add(graph)),
                            sock,
                            Name::new(name),
                            IgnoreTraces,
                        ))
                        .id();
                    cmd.entity(joint).add_child(root);
                    if name == "DroneController" {
                        cmd.entity(root).insert(DarkPlastic);
                    }
                }
                let _ = &assets;
            }
            if crossed(DETACH_T) && !perk.detached {
                // DetachDrone_C: the held drone leaves the hand (K2_DetachFromComponent keep world) and drops.
                perk.detached = true;
                if let Some((_, _, gt, mut vis)) = guns.iter_mut().find(|(_, g, _, _)| g.0 == "Kamikaze")
                    && let Some(g) = gltfs.get(&assets.load::<Gltf>("models/guns/Kamikaze.glb"))
                {
                    *vis = Visibility::Hidden;
                    let (_, hand_rot, hand_pos) = gt.to_scale_rotation_translation();
                    let (yaw, _, _) = cam.to_scale_rotation_translation().1.to_euler(EulerRot::YXZ);
                    cmd.spawn((
                        Drone { vel: Vec3::ZERO, yaw, pitch: 0.0, flying: false, hold: 0.0, tilt: Vec2::ZERO, spawn: 0.0 },
                        Transform::from_translation(hand_pos).with_rotation(hand_rot * drone_basis().inverse()),
                        Visibility::default(),
                        Name::new("KamikazeDrone"),
                    ))
                    .with_children(|c| {
                        c.spawn((WorldAssetRoot(g.scenes[0].clone()), Transform::from_rotation(drone_basis()), IgnoreTraces, DroneBody));
                    });
                }
            }
            if crossed(FINISH_T) {
                // Deploy.Finished -> GA_LaunchPerk: camera transition Disconnected, drone possessed, body locked.
                perk.phase = Phase::Piloting;
                perk.fade = 1.0;
                perk.fade_to = 0.0;
                p.locked = true;
                snd.write(PlayCue { cue: "/Game/FPV_Drone/Sfx/Boot/ECS_Boot_Bip_Sound_Cue".into(), dist_cm: 0.0 });
                cmd.spawn((Motor, AudioPlayer::<AudioSource>(assets.load("audio/FPV_Drone/Sfx/Drone/Drone_Idle_Loop.wav")), PlaybackSettings::LOOP.with_volume(bevy::audio::Volume::Linear(0.4))));
                fx.base_fov_deg = DRONE_FOV;
                for mut v in &mut arms_vis {
                    *v = Visibility::Hidden;
                }
                for (e, t) in &drones {
                    let _ = (e, t);
                }
            }
        }
        Phase::Piloting => {
            let leave = keys.just_pressed(KeyCode::KeyF) || perk.battery <= 0.0 || perk.wants_exit;
            if leave {
                // GA_LeavePerk / GA_OutOfBattery: control returns to the body, Drone_Exit montage.
                perk.phase = Phase::Exit;
                perk.t = 0.0;
                for m in &motors {
                    cmd.entity(m).despawn();
                }
                perk.fade = 1.0;
                perk.fade_to = 0.0;
                p.locked = false;
                fx.base_fov_deg = settings.fov;
                for mut v in &mut arms_vis {
                    *v = Visibility::Inherited;
                }
                if perk.wants_exit {
                    // crash -> GA_PerkDestruction ... drone already exploded
                } else {
                    for (e, _) in &drones {
                        cmd.entity(e).despawn();
                    }
                }
                let m = inv.slots[SLOT_PERK].anims["Extra"].get("Exit").and_then(crate::weapon::Montage::from_value);
                inv.use_item(m, &mut out);
                for mut v in &mut visual_cmd {
                    v.want = Some((if v.nodes.contains_key("A_W_Controller_Exit") { "A_W_Controller_Exit" } else { "A_W_Goggles_Exit" }, 1.0));
                }
            }
        }
        Phase::Exit => {
            perk.t += dt;
            if perk.t >= 1.7 && !inv.busy() {
                for e in &visuals {
                    cmd.entity(e).despawn();
                }
                perk.phase = Phase::Idle;
                perk.cooldown = COOLDOWN;
                perk.wants_exit = false;
                perk.detached = false;
                perk.t = 0.0;
                inv.swap_to(0, &mut out);
            }
        }
    }
    // swapping away while idle resets nothing: the perk only runs from its own slot
    let _ = (cur, on_drone);
}

/// glTF AnimationPlayers under a PerkVisual root get that root's graph.
fn hook_anims(mut cmd: Commands, new: Query<Entity, Added<AnimationPlayer>>, parents: Query<&ChildOf>, roots: Query<&AnimationGraphHandle, With<PerkVisual>>, vis: Query<(), With<PerkVisual>>) {
    for e in &new {
        if let Some(r) = root_of(e, &parents, |x| vis.contains(x))
            && let Ok(g) = roots.get(r)
        {
            cmd.entity(e).insert((g.clone(), AnimationTransitions::new(), PerkAnim(r)));
        }
    }
}

fn play_anims(mut roots: Query<&mut PerkVisual>, mut players: Query<(&mut AnimationPlayer, &mut AnimationTransitions, &PerkAnim)>) {
    for (mut player, mut tr, link) in &mut players {
        let Ok(mut v) = roots.get_mut(link.0) else { continue };
        let Some((clip, speed)) = v.want else { continue };
        let Some(&node) = v.nodes.get(clip) else { continue };
        tr.play(&mut player, node, Duration::ZERO).set_speed(speed);
        v.want = None;
    }
}

#[allow(clippy::too_many_arguments)]
fn drone_fly(
    mut cmd: Commands,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    mut perk: ResMut<Perk>,
    mut q: Query<(Entity, &mut Transform, &mut Drone)>,
    mut ray: MeshRayCast,
    ignore: Query<(), With<IgnoreTraces>>,
    lights: Query<Entity, With<DroneLight>>,
    mut boom: MessageWriter<Detonate>,
    mut snd: MessageWriter<PlayCue>,
    mut motor: Query<&mut AudioSink, With<Motor>>,
) {
    let dt = time.delta_secs().min(0.05);
    let Ok((e, mut tf, mut d)) = q.single_mut() else { return };
    let filter = |x: Entity| !ignore.contains(x);
    let mut cast = |from: Vec3, dir: Vec3, max: f32| -> Option<(f32, Vec3)> {
        let dir = Dir3::new(dir).ok()?;
        ray.cast_ray(Ray3d::new(from, dir), &MeshRayCastSettings::default().with_filter(&filter)).first().filter(|(_, h)| h.distance <= max).map(|(_, h)| (h.distance, h.normal.normalize_or_zero()))
    };
    if !d.flying {
        // the dropped drone: gravity, rests on the first surface below (physics body in UE)
        d.spawn += dt;
        d.vel.y -= G * dt;
        let step = d.vel * dt;
        if let Some((dist, n)) = cast(tf.translation, step, step.length() + 0.05) {
            tf.translation += step.normalize_or_zero() * (dist - 0.05).max(0.0);
            d.vel = Vec3::ZERO;
            let _ = n;
            // lay level
            let target = Quat::from_rotation_y(d.yaw);
            tf.rotation = tf.rotation.slerp(target, (6.0 * dt).min(1.0));
        } else {
            tf.translation += step;
        }
        if perk.phase == Phase::Piloting {
            d.flying = true;
            if let Some(y) = std::env::var("BODYCAM_DRONE_YAW").ok().and_then(|v| v.parse::<f32>().ok()) {
                d.yaw = y.to_radians(); // dev: start the flight looking this way (180 = back at the player)
            }
            tf.rotation = Quat::from_rotation_y(d.yaw);
            perk.battery = 100.0;
        }
        return;
    }
    if perk.phase != Phase::Piloting {
        return;
    }
    // Look: Mouse2D -> control rotation.
    let sens = 0.0022 * crate::ui::settings::sens_mult();
    d.yaw -= motion.delta.x * sens;
    d.pitch = (d.pitch - motion.delta.y * sens).clamp(-1.5, 1.5);
    let key = |k: KeyCode| keys.pressed(k) as i8 as f32;
    let (fwd_in, side_in) = (key(KeyCode::KeyW) - key(KeyCode::KeyS), key(KeyCode::KeyD) - key(KeyCode::KeyA));
    let up_in = (key(KeyCode::Space) + mouse.pressed(MouseButton::Left) as i8 as f32) - (key(KeyCode::ControlLeft) + mouse.pressed(MouseButton::Right) as i8 as f32);
    let yaw_q = Quat::from_rotation_y(d.yaw);
    let cam_fwd = yaw_q * Quat::from_rotation_x(d.pitch) * Vec3::NEG_Z;
    let flat_fwd = yaw_q * Vec3::NEG_Z;
    let right = yaw_q * Vec3::X;
    // Angle - Movement: forward uses the camera's 3D forward, backward the flat one.
    let input = if fwd_in > 0.0 { cam_fwd } else { flat_fwd } * fwd_in * FWD_FORCE + right * side_in * SIDE_FORCE + Vec3::Y * up_in.clamp(-1.0, 1.0) * VERT_FORCE;
    let input = input.clamp_length_max(1.0);
    if input.length() > 0.01 {
        d.hold += dt;
        let ramp = crate::curve::eval(&ACC_CURVE, d.hold / RAMP_TIME);
        d.vel += input * ACCEL * ramp.max(0.05) * dt;
    } else {
        d.hold = 0.0;
        let s = d.vel.length();
        d.vel = d.vel.normalize_or_zero() * (s - DECEL * dt).max(0.0);
    }
    d.vel = d.vel.clamp_length_max(MAX_SPEED);
    // collision sweep (ray of the move + RADIUS)
    let step = d.vel * dt;
    if let Some((dist, n)) = cast(tf.translation, step, step.length() + RADIUS) {
        let n = if n.dot(d.vel) > 0.0 { -n } else { n }; // face the surface toward the drone (mesh normals are not reliably outward)
        let impact = -d.vel.dot(n);
        if impact > CRASH_SPEED {
            // GameplayEvent.Perk.Crash -> GA_DroneCrash -> GA_ExplodePerk
            boom.write(Detonate { at: tf.translation, spec: DRONE_BLAST });
            snd.write(PlayCue { cue: "/Game/FPV_Drone/Sfx/DroneCrash/DroneCrashBipAndPropellerCut_Cue".into(), dist_cm: 0.0 });
            cmd.entity(e).despawn();
            for l in &lights {
                cmd.entity(l).despawn();
            }
            perk.wants_exit = true;
            return;
        }
        tf.translation += step.normalize_or_zero() * (dist - RADIUS).max(0.0);
        let vn = d.vel.dot(n).min(0.0);
        d.vel -= n * vn;
    } else {
        tf.translation += step;
    }
    // visual tilt toward the movement (Angle - Drone Visual Rotation)
    let local = Quat::from_rotation_y(-d.yaw) * d.vel;
    let target = Vec2::new((local.z / MAX_SPEED).clamp(-1.0, 1.0) * 0.45, (-local.x / MAX_SPEED).clamp(-1.0, 1.0) * 0.45);
    let tilt = d.tilt;
    d.tilt += (target - tilt) * (6.0 * dt).min(1.0);
    tf.rotation = yaw_q * Quat::from_rotation_x(-d.tilt.x) * Quat::from_rotation_z(d.tilt.y);
    for mut m in &mut motor {
        let k = d.vel.length() / MAX_SPEED;
        m.set_speed(0.85 + 0.5 * k);
        m.set_volume(bevy::audio::Volume::Linear(0.4 + 0.3 * k));
    }
    // battery: IdleBatteryConsumption + SpeedBatteryConsumptionFactor x speed (cm/s) per second
    perk.battery -= (BATTERY_IDLE + BATTERY_SPEED * d.vel.length() * 100.0) * dt;
    // C: GA_Flashlight toggle
    if keys.just_pressed(KeyCode::KeyC) {
        perk.light = !perk.light;
        snd.write(PlayCue { cue: "/Game/BodycamWeapons/LowPolyShooterPack/Audio/WAV/Weapons/S_WEP_Flashlight_Click".into(), dist_cm: 0.0 });
        for l in &lights {
            cmd.entity(l).despawn();
        }
        if perk.light {
            // FlashLight SpotLightComponent: OuterConeAngle 75, AttenuationRadius 3000 cm
            cmd.spawn((SpotLight { intensity: 120_000.0, range: 30.0, outer_angle: 75f32.to_radians().min(1.35), inner_angle: 35f32.to_radians(), color: Color::srgb(1.0, 0.97, 0.9), shadow_maps_enabled: false, ..default() }, Transform::default(), DroneLight));
        }
    }
}

/// While piloting the BodycamCamera sits on the drone (FPVCamera socket ~ +10.5 cm forward, +2.3 cm up), looking along the control rotation.
fn piloting_camera(
    perk: Res<Perk>,
    drone: Query<(&Transform, &Drone), Without<BodycamCamera>>,
    player: Single<&GlobalTransform, With<Player>>,
    mut cam: Single<&mut Transform, With<BodycamCamera>>,
    mut lights: Query<&mut Transform, (With<DroneLight>, Without<BodycamCamera>, Without<Drone>)>,
    time: Res<Time>,
    mut body: Query<&mut Visibility, With<DroneBody>>,
) {
    let hide = perk.phase == Phase::Piloting;
    for mut v in &mut body {
        *v = if hide { Visibility::Hidden } else { Visibility::Inherited };
    }
    if perk.phase != Phase::Piloting {
        return;
    }
    let Ok((tf, d)) = drone.single() else { return };
    let rot = Quat::from_rotation_y(d.yaw) * Quat::from_rotation_x(d.pitch);
    let mut pos = tf.translation + rot * Vec3::new(0.0, 0.0226, -0.105);
    // FPV shake (CS_FpvDroneFlyShake): speed-scaled micro jitter
    let k = (d.vel.length() / MAX_SPEED) * 0.004;
    let t = time.elapsed_secs();
    pos += Vec3::new((t * 37.0).sin(), (t * 41.0).cos(), (t * 29.0).sin()) * k;
    let world = Transform::from_translation(pos).with_rotation(rot);
    let inv = player.affine().inverse();
    let local = Mat4::from(inv) * world.to_matrix();
    let (_, r, tr) = local.to_scale_rotation_translation();
    cam.translation = tr;
    cam.rotation = r;
    for mut l in &mut lights {
        *l = world;
    }
}

fn hud(
    perk: Res<Perk>,
    inv: Single<&Inventory>,
    mut text: Single<(&mut Text, &mut TextColor), With<PerkText>>,
    drone: Query<&Drone>,
) {
    let (t, c) = &mut *text;
    let held = inv.current == SLOT_PERK && inv.slots.get(SLOT_PERK).is_some_and(|w| w.kind == ItemKind::Drone);
    match perk.phase {
        Phase::Piloting => {
            let speed = drone.single().map(|d| d.vel.length()).unwrap_or(0.0);
            t.0 = format!("BAT {:3.0}%   {:4.1} m/s   [F] LEAVE   [C] LIGHT", perk.battery.max(0.0), speed);
            c.0 = if perk.battery < 15.0 { Color::srgb(1.0, 0.25, 0.1) } else { Color::srgb(0.7, 1.0, 0.7) };
        }
        Phase::Idle if held && perk.cooldown > 0.0 => {
            t.0 = format!("KAMIKAZE COOLDOWN {:.0}s", perk.cooldown.ceil());
            c.0 = Color::srgb(1.0, 0.7, 0.2);
        }
        Phase::Idle if held => {
            t.0 = "[LMB] DEPLOY DRONE".into();
            c.0 = Color::srgba(1.0, 1.0, 1.0, 0.7);
        }
        _ => c.0 = Color::NONE,
    }
}

/// CameraTransition Connected/Disconnected (FastTransition): a short dip through black.
fn fade(time: Res<Time>, mut perk: ResMut<Perk>, mut q: Single<&mut BackgroundColor, With<PerkFade>>) {
    let dt = time.delta_secs();
    perk.fade += (perk.fade_to - perk.fade).signum() * dt * 3.0;
    perk.fade = perk.fade.clamp(0.0, 1.0);
    q.0 = Color::srgba(0.0, 0.0, 0.0, perk.fade);
}

/// DarkPlastic roots: every mesh below gets a plain dark material.
fn tint(mut cmd: Commands, new: Query<Entity, Added<Mesh3d>>, parents: Query<&ChildOf>, roots: Query<(), With<DarkPlastic>>, mut mats: ResMut<Assets<StandardMaterial>>, mut cache: Local<Option<Handle<StandardMaterial>>>) {
    for e in &new {
        if root_of(e, &parents, |x| roots.contains(x)).is_some() {
            let m = cache.get_or_insert_with(|| mats.add(StandardMaterial { base_color: Color::srgb(0.06, 0.06, 0.065), perceptual_roughness: 0.55, ..default() })).clone();
            cmd.entity(e).insert(MeshMaterial3d(m));
        }
    }
}
