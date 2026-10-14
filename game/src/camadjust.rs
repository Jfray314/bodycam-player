//! AC_BodycamCamera "camera adjust": the player now and then reaches up and re-seats the chest bodycam.
//! FootstepShake -> UpdateCameraAdjust(0.05 sprinting > 200 cm/s, else 0.01 above 20 cm/s) + MapRangeClamped(|Yaw+Pitch
//! (DeltaTime)|, 2..4 -> 0..1), clamped 0..1. At 1 with CanAdjust? (speed < 175 cm/s, no montage, not aiming) it resets and
//! PlayCameraAdjust(false): a DoOnce gate, delay RandomFloat(0, 3), then if not InDanger? (another character within 15 m) and
//! still CanAdjust? play A_FP_GoPro_Adjust01 (the Spawn variant is never called), and reopen the gate after the montage + 10..100 s.
use crate::player::{Footstep, Player};
use crate::weapon::{ActionKind, Inventory, Montage, MontageStarted};
use bevy::prelude::*;
use rand::Rng;

pub struct CamAdjustPlugin;
impl Plugin for CamAdjustPlugin {
    fn build(&self, app: &mut App) {
        // BODYCAM_ADJUST=<s>: dev hook, the gate check runs that many sim seconds after the viewmodel is ready.
        let dev = std::env::var("BODYCAM_ADJUST").ok().and_then(|v| v.parse().ok());
        app.insert_resource(CamAdjust { dev, track: load_track(), ..default() })
            .add_systems(Update, tick)
            .add_systems(PostUpdate, camera_anim.after(bevy::app::AnimationSystems).after(crate::viewmodel::FollowHead).before(bevy::transform::TransformSystems::Propagate));
    }
}

const DIR: &str = "/BodycamAnimationFramework/Animations/Gopro/Adjust/Character/";

#[derive(Resource, Default)]
pub struct CamAdjust {
    /// CameraAdjust (a Double in UE: 100 walking steps of 0.01 must reach exactly 1).
    pub value: f64,
    /// Seconds until the delayed PlayCameraAdjust check runs (gate closed).
    pending: Option<f32>,
    /// Gate closed until this many seconds pass (montage + 10..100 s).
    closed_for: f32,
    dev: Option<f32>,
    /// A_C_GoPro_Adjust01_Anim camera_armature rotation keys (UE quats relative to key 0) + sample rate.
    track: Vec<Quat>,
    /// Seconds since the adjust montage started (None = idle).
    since: Option<f32>,
}

/// Camera skeleton clip of the adjust montage (BafCameraAnimation plays it on the camera bone; Camera_Pitch/Yaw/Roll curves
/// are baked into these keys by tools/build_models.py -> camera_tracks.json).
const TRACK_HZ: f32 = 30.428572;
fn load_track() -> Vec<Quat> {
    let root = std::env::var("BODYCAM_ASSETS").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../assets-local").into());
    let Ok(txt) = std::fs::read_to_string(format!("{root}/models/camera_tracks.json")) else { return vec![] };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&txt) else { return vec![] };
    let keys: Vec<Quat> = v["A_C_GoPro_Adjust01_Anim"]["bones"][0]["quat"]
        .as_array()
        .map(|a| a.iter().map(|q| Quat::from_xyzw(q[0].as_f64().unwrap_or(0.0) as f32, q[1].as_f64().unwrap_or(0.0) as f32, q[2].as_f64().unwrap_or(0.0) as f32, q[3].as_f64().unwrap_or(1.0) as f32)).collect())
        .unwrap_or_default();
    let Some(&q0) = keys.first() else { return keys };
    // UE (X fwd, Y right, Z up, left-handed) -> Bevy (right X, up Y, fwd -Z): vector (x,y,z) -> (y,z,-x), handedness flips the angle.
    keys.iter().map(|&q| { let r = q0.inverse() * q; Quat::from_xyzw(-r.y, -r.z, r.x, r.w) }).collect()
}

/// Camera-only rotation from the camera clip, blended with the montage envelope (BlendIn/BlendOut 0.25 s).
fn camera_anim(time: Res<Time>, mut st: ResMut<CamAdjust>, mut cam: Single<&mut Transform, With<crate::player::BodycamCamera>>) {
    let Some(t) = st.since.as_mut() else { return };
    *t += time.delta_secs();
    let t = *t;
    if t > 2.3333333 || st.track.is_empty() {
        st.since = None;
        return;
    }
    let f = (t * TRACK_HZ).min((st.track.len() - 1) as f32);
    let (i, a) = (f as usize, f.fract());
    let q = st.track[i].slerp(st.track[(i + 1).min(st.track.len() - 1)], a);
    let env = (t / 0.25).min(1.0) * ((2.3333333 - t) / 0.25).clamp(0.0, 1.0);
    cam.rotation *= Quat::IDENTITY.slerp(q, env);
}

/// UpdateCameraAdjust: returns true when the accumulator hits 1 and adjusting is allowed (it then resets).
pub fn update(value: &mut f64, step: f64, yaw_dt: f32, can_adjust: bool) -> bool {
    *value = (*value + step + ((yaw_dt.abs() as f64 - 2.0) / 2.0).clamp(0.0, 1.0)).clamp(0.0, 1.0);
    if *value == 1.0 && can_adjust {
        *value = 0.0;
        return true;
    }
    false
}

fn montage() -> Montage {
    let sound = serde_json::json!({"Sound": "/BodycamAnimationFramework/Animations/Gopro/Sound/CameraReadjustMuffle.CameraReadjustMuffle"});
    Montage {
        path: format!("{DIR}A_FP_GoPro_Adjust01_Montage.A_FP_GoPro_Adjust01_Montage"),
        length: 2.3333333,
        notifies: vec![(0.0001, "PlayWepSounds_C".into(), sound)],
        blend_in: 0.25,
        blend_out: 0.25, // montage BlendIn/BlendOut unset = UE defaults
        weapon: None,
    }
}

fn tick(
    time: Res<Time>,
    mut steps: MessageReader<Footstep>,
    p: Single<&Player>,
    inv: Option<Single<&Inventory>>,
    gunfight: Option<Res<crate::gunfight::Gunfight>>,
    ready: Res<crate::dev::Ready>,
    mut st: ResMut<CamAdjust>,
    mut out: MessageWriter<MontageStarted>,
) {
    let dt = time.delta_secs();
    if ready.0.is_some() && let Some(t) = st.dev.take() {
        st.pending = Some(t);
    }
    let speed_cm = p.vel.length() * 100.0;
    let can_adjust = speed_cm < 175.0 && !p.aiming && inv.as_ref().is_none_or(|i| i.action.is_none());
    // ponytail: Pitch(DeltaTime) isn't tracked; only the yaw term feeds the turn contribution.
    for _ in steps.read() {
        if speed_cm > 20.0 && update(&mut st.value, if speed_cm > 200.0 { 0.05 } else { 0.01 }, p.yaw_rate, can_adjust) && st.pending.is_none() && st.closed_for <= 0.0 {
            st.pending = Some(rand::rng().random_range(0.0..3.0));
        }
    }
    st.closed_for -= dt;
    let Some(left) = st.pending.as_mut() else { return };
    *left -= dt;
    if *left > 0.0 {
        return;
    }
    st.pending = None;
    // ponytail: InDanger? = the gunfight test bench is on (its shooter spawns inside 15 m); no other characters exist.
    if gunfight.is_some_and(|g| g.enabled) || !can_adjust {
        return;
    }
    let m = montage();
    st.since = Some(0.0);
    st.closed_for = m.length + rand::rng().random_range(10.0..100.0);
    let row = inv.as_ref().map(|i| i.slots[i.current].name.to_string()).unwrap_or_default();
    info!("camera adjust montage at {:.1} s (gate closed {:.0} s)", time.elapsed_secs(), st.closed_for);
    out.write(MontageStarted { row, kind: ActionKind::CameraAdjust, montage: m });
}

#[cfg(test)]
mod tests {
    #[test]
    fn accumulates_per_step_and_turning() {
        let mut v = 0.0;
        for _ in 0..99 {
            assert!(!super::update(&mut v, 0.01, 0.0, true));
        }
        assert!(super::update(&mut v, 0.01, 0.0, true) && v == 0.0); // 100 walking steps
        assert!(super::update(&mut v, 0.01, 4.0, true)); // one step during a fast turn
        assert!(!super::update(&mut v, 0.01, 4.0, false) && v == 1.0); // held at 1 until allowed
    }
}
