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
        app.init_resource::<CamAdjust>().add_systems(Update, tick);
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
    mut st: ResMut<CamAdjust>,
    mut out: MessageWriter<MontageStarted>,
) {
    let dt = time.delta_secs();
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
