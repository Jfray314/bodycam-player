//! Player reactions: hit camera shake, death + respawn, fall kill, explosion shake/ringing, flashbang.
//! Sources: Bodycam_Player.StartBloodOnCamera (ScreenDroplets 0.5 for 5 s), GA_Character_HandleFall (kill when falling faster than
//! SpeedRagdoll -1000 cm/s or > VacuumLimit 825 cm above the floor; GE_InstantKill = Health Override -500),
//! GA_Character_ExplosionNearby + ExplosionShake (pitch 10 deg @15 Hz, yaw 4 @5, roll 5 @2, 1 s + 1.5 s blend out),
//! WW_SurvivorStatus.HandleStartFlashbang (effect = MapRangeClamped(dist 250..1000 cm -> 1..0), stress 2..1, FInterpTo speed 0.25
//! until < 0.15, Flashbang_Ear loop + muffle mix). F8/F9/F11/F12 = explosion / flashbang / hit / kill (dev).
use crate::player::{BodycamCamera, Player};
use crate::postfx::BodycamPostFx;
use crate::status::{finterp_to, map_range_clamped, PlayerDamage, SurvivorStatus};
use bevy::audio::Volume;
use bevy::prelude::*;

/// An explosion at `at` (world, m). Grenade code sends it; F8 in dev.
#[derive(Message)]
pub struct Explosion {
    pub at: Vec3,
}
/// A flashbang burst at `at` (GameplayCue.Player.Flashed).
#[derive(Message)]
pub struct Flashbang {
    pub at: Vec3,
}

const DEATH_CUT_S: f32 = 1.0;
const DEATH_RESPAWN_S: f32 = 5.0; // ponytail: respawn flow is menu/server driven in the game; fixed delay here
// StartBloodOnCamera: Drops Intensity 0.5 while Timeline_0 runs (2.94 s, CurveFloat_1), Delay 5.0, then Timeline_1 (1.0 s, CurveFloat_3) and the PP
// is disabled. ponytail: UE pops off after that last second (its curve rises 0.03 -> 0.40); here it fades out over it.

#[derive(Resource, Default)]
struct Fx {
    shake: Option<f32>,
    shake_gain: f32,
    /// GCN_HitReaction camera shake: (time, scale, noise seed).
    hit: Option<(f32, f32, f32)>,
    flash: f32,
    /// Ringing without whiteout (explosions).
    deaf: f32,
    ring: Option<Entity>,
    dead: Option<f32>,
    spawn: Option<(Vec3, f32)>,
}

pub struct EffectsPlugin;
impl Plugin for EffectsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Fx>()
            .add_message::<Explosion>()
            .add_message::<Flashbang>()
            .add_systems(Update, (dev_keys, events, tick).chain().before(crate::player::bodycam_camera))
            .add_systems(Update, camera.after(crate::player::bodycam_camera));
    }
}

fn dev_keys(keys: Res<ButtonInput<KeyCode>>, cam: Single<&GlobalTransform, With<BodycamCamera>>, mut boom: MessageWriter<Explosion>, mut flash: MessageWriter<Flashbang>, mut dmg: MessageWriter<PlayerDamage>, mut s: ResMut<SurvivorStatus>) {
    let ahead = cam.translation() + cam.forward() * 4.0;
    if keys.just_pressed(KeyCode::F8) {
        boom.write(Explosion { at: ahead });
    }
    if keys.just_pressed(KeyCode::F9) {
        flash.write(Flashbang { at: ahead });
    }
    if keys.just_pressed(KeyCode::F11) {
        dmg.write(PlayerDamage(10.0));
    }
    if keys.just_pressed(KeyCode::F12) {
        s.health = 0.0;
    }
}

fn events(
    mut cmd: Commands,
    mut fx: ResMut<Fx>,
    assets: Res<AssetServer>,
    mut dmg: MessageReader<PlayerDamage>,
    mut boom: MessageReader<Explosion>,
    mut flash: MessageReader<Flashbang>,
    mut st: ResMut<SurvivorStatus>,
    cam: Single<&GlobalTransform, With<BodycamCamera>>,
) {
    for d in dmg.read() {
        // GCN_HitReaction (locally controlled): Sprint_CameraShake, Scale = MapRangeClamped(RawMagnitude, 0, 100, 0, 1); bSingleInstance
        if fx.dead.is_none() {
            fx.hit = Some((0.0, map_range_clamped(d.0, 0.0, 100.0, 0.0, 1.0), rand::random::<f32>() * 1000.0));
        }
    }
    let ring = |cmd: &mut Commands, fx: &mut Fx| {
        if fx.ring.is_none() {
            // Flashbang_Ear_Cue: looping ear whine, FadeIn 1 s
            fx.ring = Some(cmd.spawn((AudioPlayer::<AudioSource>(assets.load("audio/BallisticsVFX/SFX/FlashbangEar/Flashbang_Ear.wav")), PlaybackSettings::LOOP.with_volume(Volume::Linear(0.0)))).id());
        }
    };
    for e in boom.read() {
        let d_cm = cam.translation().distance(e.at) * 100.0;
        let near = map_range_clamped(d_cm, 250.0, 2000.0, 1.0, 0.0);
        fx.shake = Some(0.0);
        fx.shake_gain = fx.shake_gain.max(near.max(0.15));
        // ponytail: ExplosionNearby itself only plays the flashed montage; ears ring when the blast is close (guess: 6 m).
        let rg = map_range_clamped(d_cm, 250.0, 600.0, 1.0, 0.0) * 0.6;
        if rg > fx.deaf.max(0.15) {
            fx.deaf = rg;
            ring(&mut cmd, &mut fx);
        }
        st.increase_stress(near * 0.5);
    }
    for e in flash.read() {
        let d_cm = cam.translation().distance(e.at) * 100.0;
        if fx.dead.is_none() {
            fx.flash = map_range_clamped(d_cm, 250.0, 1000.0, 1.0, 0.0);
            ring(&mut cmd, &mut fx);
        }
        st.stress = map_range_clamped(d_cm, 250.0, 1000.0, 2.0, 1.0);
    }
}

fn tick(
    mut cmd: Commands,
    time: Res<Time>,
    mut fx: ResMut<Fx>,
    mut st: ResMut<SurvivorStatus>,
    mut p: Single<(&mut Player, &mut Transform)>,
    mut post: Single<&mut BodycamPostFx>,
    mut vol: ResMut<GlobalVolume>,
    mut sinks: Query<&mut AudioSink>,
    mut cam_tr: MessageWriter<crate::transition::CamTransition>,
) {
    let dt = time.delta_secs();
    let (pl, tf) = &mut *p;
    let fx = &mut *fx;
    if fx.spawn.is_none() && tf.translation != Vec3::ZERO {
        fx.spawn = Some((tf.translation, pl.yaw));
    }
    // GA_Character_HandleFall: ends uncancelled (-> GE_InstantKill) once falling faster than SpeedRagdoll.
    if !pl.grounded && pl.vel.y < -10.0 && fx.dead.is_none() {
        st.health = 0.0;
    }
    // No lens blood: Bodycam_Player.StartBloodOnCamera has no caller, its MID is never created and the ScreenDroplets PP has no
    // blendables, so the real game shows nothing on screen when hit (GCN_HitReaction = camera shake only).
    if let Some((t, k, seed)) = fx.hit {
        fx.hit = (t + dt < HIT_SHAKE_S).then_some((t + dt, k, seed));
    }
    // explosion shake
    if let Some(t) = fx.shake {
        fx.shake = (t + dt < 2.5).then_some(t + dt);
        post.explosion_shake = fx.shake.map_or(0.0, |t| shake_env(t) * fx.shake_gain * 0.6);
    } else {
        fx.shake_gain = 0.0;
        post.explosion_shake = 0.0;
    }
    // flashbang / ringing: HandleFlashbangTick
    if fx.flash > 0.0 {
        fx.flash = finterp_to(fx.flash, 0.0, dt, 0.25);
        if fx.flash < 0.15 {
            fx.flash = 0.0;
        }
    }
    if fx.deaf > 0.0 {
        fx.deaf = finterp_to(fx.deaf, 0.0, dt, 0.25);
        if fx.deaf < 0.15 {
            fx.deaf = 0.0;
        }
    }
    post.flash_amount = fx.flash;
    let ear = fx.flash.max(fx.deaf);
    let ringing = ear > 0.0;
    if let Some(e) = fx.ring
        && let Ok(mut s) = sinks.get_mut(e)
    {
        let v = s.volume().to_linear();
        let target = if ringing { 0.8 * ear.clamp(0.4, 1.0) } else { 0.0 };
        s.set_volume(Volume::Linear(v + (target - v) * (dt * 1.0).min(1.0))); // FadeIn/FadeOut 1 s
        if !ringing && v < 0.01 {
            cmd.entity(e).despawn();
            fx.ring = None;
        }
    }
    // SM_Muffle stand-in: duck everything while the ears ring.
    let duck = 1.0 - 0.65 * ear.min(1.0);
    let cur = vol.volume.to_linear();
    vol.volume = Volume::Linear(cur + (duck - cur) * (dt * 4.0).min(1.0));
    // death + respawn
    match fx.dead {
        None if st.health <= 0.0 => {
            fx.dead = Some(0.0);
            fx.flash = 0.0;
            fx.deaf = 0.0;
            pl.dead = true;
            pl.vel = Vec3::ZERO;
            // Bodycam_Player.BroadcastCameraTransitionOnDeath -> W_ConnectionMenu DeactivatePanel (transition.rs)
            cam_tr.write(crate::transition::CamTransition { connected: false, instant: false });
            info!("effects: player died");
        }
        Some(t) => {
            let t = t + dt;
            fx.dead = Some(t);
            // The feed rides the falling body for DeactivatePanel's first 1.0 s, then the VHS "CAM DISCONNECTED" panel covers it
            // (transition.rs); black underneath. ponytail: the InstantKill (headshot) instant panel isn't sent (no headshot info here).
            let f = if t < DEATH_CUT_S { 1.0 } else { 0.0 };
            post.tint_r = f;
            post.tint_g = f;
            post.tint_b = f;
            if t > DEATH_RESPAWN_S {
                st.health = st.health_max;
                st.stress = 0.1;
                st.stamina = 0.0;
                if let Some((pos, yaw)) = fx.spawn {
                    tf.translation = pos;
                    pl.yaw = yaw;
                }
                pl.pitch = 0.0;
                pl.dead = false;
                fx.dead = None;
                cam_tr.write(crate::transition::CamTransition { connected: true, instant: false });
                post.tint_r = 1.0;
                post.tint_g = 1.0;
                post.tint_b = 1.0;
            }
        }
        None => {}
    }
}

/// Sprint_CameraShake (LegacyCameraShake): OscillationDuration 0.3 s, default BlendIn 0.1 / BlendOut 0.2.
const HIT_SHAKE_S: f32 = 0.3;

/// FOscillator::UpdateOffset PerlinNoise: 1D gradient noise (-1..1) at t * frequency.
fn perlin(x: f32) -> f32 {
    let grad = |i: f32| {
        let h = ((i * 127.1).sin() * 43_758.547).fract();
        h * 2.0 - 1.0
    };
    let (i, f) = (x.floor(), x.fract());
    let u = f * f * (3.0 - 2.0 * f);
    (grad(i) * f * (1.0 - u) + grad(i + 1.0) * (f - 1.0) * u) * 2.0
}

/// Sprint_CameraShake pose offset (rotation deg pitch/yaw/roll, location cm Y/Z) at `t`, scaled and blended.
fn hit_shake(t: f32, scale: f32, seed: f32) -> (Vec3, Vec2) {
    let w = (t / 0.1).min(1.0) * ((HIT_SHAKE_S - t) / 0.2).min(1.0) * scale;
    let n = |amp: f32, hz: f32, k: f32| amp * perlin(seed + k * 31.7 + t * hz);
    let rot = Vec3::new(n(10.0, 5.0, 0.0), n(5.0, 0.1, 1.0), n(10.0, 5.0, 2.0)) * w;
    // LocOscillation Y: 0.2 cm, default 1 Hz sine, random initial offset; Z: 5 cm, 0.3 Hz, offset zero
    let tau = std::f32::consts::TAU;
    let loc = Vec2::new(0.2 * (tau * (t + seed.fract())).sin(), 5.0 * (tau * 0.3 * t).sin()) * w;
    (rot, loc)
}

/// ExplosionShake envelope: full for OscillationDuration 1 s, then BlendOut 1.5 s.
fn shake_env(t: f32) -> f32 {
    if t < 1.0 { 1.0 } else { (1.0 - (t - 1.0) / 1.5).max(0.0) }
}

/// After the camera pose is built: explosion oscillation; dead, the camera rides the collapsing body (EventRagdollStart attaches it
/// to the Mesh's fp_camera socket, a head socket). The body's neck_01 is followed (the head bone is scaled away, see body::DeadCam)
/// at the offset the camera had when the player died, so there is no snap. Bone/player transforms are last frame's (1 frame lag).
fn camera(
    fx: Res<Fx>,
    dead: Res<crate::body::DeadCam>,
    roots: Query<&crate::body::BodyRoot>,
    bones: Query<&GlobalTransform, Without<BodycamCamera>>,
    pl: Single<&GlobalTransform, (With<Player>, Without<BodycamCamera>)>,
    mut off: Local<Option<bevy::math::Affine3A>>,
    mut cam: Single<(&mut Transform, &GlobalTransform), With<BodycamCamera>>,
) {
    let (cam, cam_g) = &mut *cam;
    if let Some(t) = fx.shake {
        let a = shake_env(t) * fx.shake_gain;
        let o = |amp: f32, hz: f32| amp * (std::f32::consts::TAU * hz * t).sin() * a;
        cam.rotation *= Quat::from_euler(EulerRot::YXZ, o(4.0, 5.0).to_radians(), o(10.0, 15.0).to_radians(), o(5.0, 2.0).to_radians());
    }
    if let Some((t, k, seed)) = fx.hit {
        let (r, l) = hit_shake(t, k, seed);
        cam.rotation *= Quat::from_euler(EulerRot::YXZ, r.y.to_radians(), r.x.to_radians(), r.z.to_radians());
        let d = cam.rotation * Vec3::new(l.x, l.y, 0.0) * 0.01; // UE Y right, Z up (cm)
        cam.translation += d;
    }
    let neck = roots.iter().find(|r| r.local && r.world).and_then(|r| r.bones.get("neck_01")).and_then(|&e| bones.get(e).ok());
    let (Some(neck), true) = (neck, dead.0) else {
        *off = None;
        return;
    };
    let neck = neck.affine();
    let o = *off.get_or_insert_with(|| neck.inverse() * cam_g.affine());
    **cam = Transform::from_matrix(Mat4::from(pl.affine().inverse() * neck * o));
}

#[cfg(test)]
mod tests {
    #[test]
    fn hit_shake_bounds() {
        for i in 0..300 {
            let t = i as f32 * 0.001;
            let (r, l) = super::hit_shake(t, 1.0, 3.3);
            assert!(r.abs().max_element() <= 10.0 + 1e-3 && l.y.abs() <= 5.0);
        }
        assert_eq!(super::hit_shake(0.0, 1.0, 3.3).0, bevy::math::Vec3::ZERO);
        assert!(super::perlin(2.0).abs() < 1e-5); // zero at lattice points
    }

    #[test]
    fn shake_envelope() {
        assert_eq!(super::shake_env(0.5), 1.0);
        assert!((super::shake_env(1.75) - 0.5).abs() < 1e-6);
        assert_eq!(super::shake_env(3.0), 0.0);
    }
}
