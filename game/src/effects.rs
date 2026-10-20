//! Player reactions: blood on the lens, death + respawn, fall kill, explosion shake/ringing, flashbang.
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
const BLOOD_HOLD_S: f32 = 2.94 + 5.0;
const BLOOD_FADE_S: f32 = 1.0;

#[derive(Resource, Default)]
struct Fx {
    blood: Option<f32>,
    shake: Option<f32>,
    shake_gain: f32,
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
    for _ in dmg.read() {
        if fx.blood.is_none() && fx.dead.is_none() {
            fx.blood = Some(0.0); // DoOnce until the 5 s delay ran out
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
    // blood
    post.damage_amount = match fx.blood {
        Some(t) => {
            let t = t + dt;
            fx.blood = (t < BLOOD_HOLD_S + BLOOD_FADE_S).then_some(t);
            1.0 * (1.0 - ((t - BLOOD_HOLD_S) / BLOOD_FADE_S).clamp(0.0, 1.0)) // BloodDropletsIntensity 0.5 -> coverage 1.0 (lens.wesl)
        }
        None => 0.0,
    };
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
            info!("effects: player died");
        }
        Some(t) => {
            let t = t + dt;
            fx.dead = Some(t);
            // Camera.Transition.Disconnected (Bodycam_Player.BroadcastCameraTransitionOnDeath): the feed rides the falling body,
            // then cuts hard to black (user, from the game). ponytail: CameraTransitionSubsystem is C++; DEATH_CUT_S is by eye,
            // and the InstantKill (headshot) immediate cut isn't ported (no headshot info reaches here).
            let f = if t < DEATH_CUT_S { 1.0 } else { 0.0 };
            post.tint_r = f;
            post.tint_g = f;
            post.tint_b = f;
            post.damage_amount = post.damage_amount.max(0.8 * (t / 0.6).min(1.0) * f);
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
                fx.blood = None;
                post.tint_r = 1.0;
                post.tint_g = 1.0;
                post.tint_b = 1.0;
            }
        }
        None => {}
    }
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
    fn shake_envelope() {
        assert_eq!(super::shake_env(0.5), 1.0);
        assert!((super::shake_env(1.75) - 0.5).abs() < 1e-6);
        assert_eq!(super::shake_env(3.0), 0.0);
    }
}
