//! Procedural viewmodel layers from Bodycam's animation framework, applied to the arms root (camera space):
//! - ADS alpha at BAS AdsSettings.AdsSpeed,
//! - AnimGraphNode_SpringSway (native plugin node, ABP_Procedural_Sway) look/move sway with the BAS Look*/HipLook*/
//!   MovePositionSway tables about SwayPivotOffset,
//! - AC_BodycamAnimationComponent additive aim-in/out motions (RefreshAdditiveMotion).
//! The SpringSway node is compiled C++ with no source or docs, so the integrator is reconstructed from its pin names
//! (Stiffness, Damping, SmoothingSpeed, ExtraSmoothing, Scale, ClampMax, AdsScale, PivotOffset).
use crate::curve::{Key, eval};
use crate::player::Player;
use crate::status::finterp_to;
use crate::weapon::Inventory;
use bevy::input::mouse::AccumulatedMouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions};
use serde_json::Value;

pub struct SwayPlugin;
impl Plugin for SwayPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Ads>().init_resource::<Sway>().init_resource::<FakeLook>().init_resource::<AimOffset>().add_systems(Update, (tick_ads, tick_sway).chain());
    }
}

/// Dev: BODYCAM_AUTO "LookR"/"LookL"/"LookU" inject mouse counts per frame for the sway only.
#[derive(Resource, Default)]
pub struct FakeLook(pub Vec2);

/// ADS progress: `raw` rises/falls linearly at AdsSettings.AdsSpeed per second, `alpha` is the eased value.
#[derive(Resource, Default)]
pub struct Ads {
    pub raw: f32,
    pub alpha: f32,
    /// Seconds a full hip<->ADS transition takes for the current weapon.
    pub duration: f32,
}

fn tick_ads(time: Res<Time>, p: Single<&Player>, inv: Option<Single<&Inventory>>, mut ads: ResMut<Ads>) {
    let a = inv.as_ref().map(|i| &i.slots[i.current].anims["AdsSettings"]);
    let speed = a.and_then(|a| a["AdsSpeed"].as_f64()).filter(|s| *s > 0.0).unwrap_or(6.0) as f32;
    ads.duration = 1.0 / speed;
    let target = if p.aiming { 1.0 } else { 0.0 };
    let step = speed * time.delta_secs();
    ads.raw += (target - ads.raw).clamp(-step, step);
    // AdsBlend.BlendOption is Sinusoidal on the sampled weapons (BlendTime 0 = unused in the graph).
    // ponytail: ease shape fixed to sinusoidal; other BlendOptions not mapped.
    ads.alpha = 0.5 - 0.5 * (ads.raw * std::f32::consts::PI).cos();
}

/// One axis of the spring node.
#[derive(Clone, Copy, Default, Debug)]
pub struct AxisParams {
    pub stiffness: f32,
    pub damping: f32,
    pub speed: f32,
    pub smooth: f32,
    pub extra: f32,
    pub scale: f32,
    pub ads_scale: f32,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct AxisState {
    smoothed: f32,
    x: f32,
    v: f32,
    out: f32,
}

/// Node tuning: Scale is the input gain in the node's units; the BAS values assume UE's per-frame mouse input.
// ponytail: GAIN is a guess so a 1-degree-per-frame flick gives ~1 cm / ~1 deg; tune against the real game.
const GAIN: f32 = 0.2;
const INPUT_CLAMP: f32 = 10.0; // node ClampMax default

impl AxisState {
    /// `input` in degrees-per-60Hz-frame units (or normalized speed for move), `ads` 0..1, `hip_only` fades with ADS.
    pub fn step(&mut self, p: &AxisParams, input: f32, ads: f32, dt: f32) -> f32 {
        if p.scale == 0.0 && p.ads_scale == 0.0 {
            self.out = 0.0;
            return 0.0;
        }
        // 1. smooth the raw input (SmoothDamping = FInterp speed)
        self.smoothed = finterp_to(self.smoothed, input.clamp(-INPUT_CLAMP, INPUT_CLAMP), dt, p.smooth);
        let gain = (p.scale + (p.ads_scale - p.scale) * ads) * GAIN;
        let target = self.smoothed * gain;
        // 2. damped spring toward it: omega = speed*sqrt(stiffness), zeta = damping (1 = critically damped).
        // ponytail: guessed mapping of SpringStiffness/Damping/Speed onto omega/zeta; semi-implicit Euler, substepped.
        let omega = p.speed * p.stiffness.max(0.0).sqrt();
        let zeta = p.damping;
        let n = (dt / 0.004).ceil().max(1.0) as i32;
        let h = dt / n as f32;
        for _ in 0..n {
            self.v += (omega * omega * (target - self.x) - 2.0 * zeta * omega * self.v) * h;
            self.x += self.v * h;
        }
        // 3. ExtraSmoothing = extra FInterp on the output (0 = off).
        self.out = if p.extra > 0.0 { finterp_to(self.out, self.x, dt, p.extra) } else { self.x };
        self.out
    }
}

fn axis_params(v: &Value, ads_only_hip: bool) -> [AxisParams; 3] {
    let _ = ads_only_hip;
    let g = |k: &str, i: usize| v[k][["x", "y", "z"][i]].as_f64().unwrap_or(0.0) as f32;
    std::array::from_fn(|i| AxisParams {
        stiffness: g("SpringStiffness", i),
        damping: g("SpringDamping", i),
        speed: g("SpringSpeed", i),
        smooth: g("SmoothDamping", i),
        extra: g("ExtraSmoothing", i),
        scale: g("Scale", i),
        ads_scale: if v.get("AdsScale").is_some() { g("AdsScale", i) } else { 0.0 },
    })
}

/// Spring bank for one BAS table (3 axes).
#[derive(Default)]
struct Bank {
    s: [AxisState; 3],
}

#[derive(Resource, Default)]
pub struct Sway {
    /// Camera-space (Bevy axes, metres) offset the arms root gets, already pivoted about SwayPivotOffset.
    pub offset: Transform,
    look_pos: Bank,
    look_rot: Bank,
    hip_pos: Bank,
    hip_rot: Bank,
    move_pos: Bank,
    weapon: String,
}

/// UE (X fwd, Y right, Z up; cm) -> Bevy camera space (m).
pub fn ue_vec(v: Vec3) -> Vec3 {
    Vec3::new(v.y, v.z, -v.x) * 0.01
}
/// UE rotator (roll about X fwd, pitch about Y right, yaw about Z up; degrees) -> Bevy quaternion.
pub fn ue_rot(roll: f32, pitch: f32, yaw: f32) -> Quat {
    Quat::from_rotation_y(-yaw.to_radians()) * Quat::from_rotation_x(pitch.to_radians()) * Quat::from_rotation_z(-roll.to_radians())
}

const SENS_DEG: f32 = 0.0022 * 57.29578; // player.rs SENS (rad per count) in degrees
const GRIP: Vec3 = Vec3::new(0.09, -0.22, -0.13);

/// Pure spring-bank step used by the system and the tests. Inputs: look (yaw, pitch deg/frame) and move (fwd, right, 0..1).
fn run(sw: &mut Sway, anims: &Value, look: Vec2, mv: Vec2, ads: f32, dt: f32) {
    let get = |k: &str| axis_params(&anims[k], k.starts_with("Hip"));
    let (lp, lr, hp, hr, mp) = (get("LookPositionSway"), get("LookRotationSway"), get("HipLookPositionSway"), get("HipLookRotationSway"), get("MovePositionSway"));
    // Gun lags the view: turning right (+yaw input) pushes it left/down. Look input in degrees per 60 Hz frame.
    let (yaw_in, pitch_in) = (look.x, look.y);
    let hip = 1.0 - ads;
    // Position vec (x fwd, y right, z up); ponytail: which look axis feeds which table axis is guessed from the BAS values.
    let pos_in = [pitch_in, -yaw_in, -pitch_in];
    // Rotation vec (x roll, y pitch, z yaw).
    let rot_in = [-yaw_in, -pitch_in, -yaw_in];
    let mv_in = [mv.x, -mv.y, 0.0];
    let step = |b: &mut Bank, p: &[AxisParams; 3], inp: [f32; 3], a: f32, w: f32| -> Vec3 {
        Vec3::from_array(std::array::from_fn(|i| b.s[i].step(&p[i], inp[i], a, dt) * w))
    };
    let pos = step(&mut sw.look_pos, &lp, pos_in, ads, 1.0) + step(&mut sw.hip_pos, &hp, pos_in, 0.0, hip) + step(&mut sw.move_pos, &mp, mv_in, ads, 1.0);
    let rot = step(&mut sw.look_rot, &lr, rot_in, ads, 1.0) + step(&mut sw.hip_rot, &hr, rot_in, 0.0, hip);
    let pivot_ue = Vec3::new(anims["SwayPivotOffset"]["x"].as_f64().unwrap_or(0.0) as f32, anims["SwayPivotOffset"]["y"].as_f64().unwrap_or(0.0) as f32, anims["SwayPivotOffset"]["z"].as_f64().unwrap_or(0.0) as f32);
    let pivot = GRIP + ue_vec(pivot_ue);
    sw.offset = Transform::from_translation(ue_vec(pos) + pivot) * Transform::from_rotation(ue_rot(rot.x, rot.y, rot.z)) * Transform::from_translation(-pivot);
}

fn tick_sway(
    time: Res<Time>,
    motion: Res<AccumulatedMouseMotion>,
    fake: Res<FakeLook>,
    cursor: Single<&CursorOptions>,
    p: Single<&Player>,
    inv: Option<Single<&Inventory>>,
    ads: Res<Ads>,
    mut sw: ResMut<Sway>,
) {
    let Some(inv) = inv else { return };
    let dt = time.delta_secs().max(1e-4);
    let d = (if cursor.grab_mode == CursorGrabMode::Locked { motion.delta } else { Vec2::ZERO }) + fake.0;
    // counts/frame -> degrees per 60 Hz frame
    let look = d * SENS_DEG / (dt * 60.0);
    // Body-local velocity (Bevy x right, z back) normalized by sprint speed.
    let local = Quat::from_rotation_y(p.yaw).inverse() * p.vel;
    let mv = Vec2::new(-local.z, local.x) / 3.25;
    let anims = inv.slots[inv.current].anims.clone();
    if sw.weapon != inv.slots[inv.current].name {
        *sw = Sway { weapon: inv.slots[inv.current].name.to_string(), ..default() };
    }
    run(&mut sw, &anims, look, mv, ads.alpha, dt);
}

// --- additive aim-in/out motions (AC_BodycamAnimationComponent.PlayAdditiveMotion / RefreshAdditiveMotion) ---

struct Curves {
    pos: [Vec<Key>; 3],
    rot: [Vec<Key>; 3],
}

fn curves(m: &Value) -> Option<Curves> {
    let get = |k: &str| -> Option<[Vec<Key>; 3]> {
        let f = m[k]["curves"]["FloatCurves"].as_array()?;
        let keys = |c: &Value| -> Vec<Key> { c.as_array().map(|a| a.iter().filter_map(|k| serde_json::from_value::<Key>(k.clone()).ok()).collect()).unwrap_or_default() };
        (f.len() >= 3).then(|| std::array::from_fn(|i| keys(&f[i])))
    };
    Some(Curves { pos: get("PositionCurve")?, rot: get("RotationCurve")? })
}

fn sample(c: &[Vec<Key>; 3], t: f32) -> Vec3 {
    Vec3::new(eval(&c[0], t), eval(&c[1], t), eval(&c[2], t))
}

fn end_time(c: &[Vec<Key>; 3]) -> f32 {
    c.iter().filter_map(|k| k.last()).map(|k| k[0]).fold(0.0, f32::max)
}

#[derive(Default)]
pub struct AimMotion {
    /// Camera-space offset for the arms root.
    pub offset: Transform,
    active: Option<Active>,
    pos: Vec3,
    rot: Quat,
    was_aiming: bool,
    init: bool,
}

struct Active {
    c: Curves,
    t: f32,
    playback: f32,
    blend: f32,
    cached_pos: Vec3,
    cached_rot: Quat,
    rot_scale: Vec3,
    acc: Vec3,
    total: Vec3,
}

// Not scheduled: AC.ActiveAdditiveMotion stays identity through every aim in/out in the UE4SS recordings
// (extracted/probe/rec_*.csv), so BAS AimIn/AimOut motions aren't played on aiming; holding the curve end
// (M4 ~9 cm) shoved the gun off the sights. Kept for whatever does call PlayAdditiveMotion.
#[allow(dead_code)]
fn tick_motion(
    time: Res<Time>,
    p: Single<&Player>,
    inv: Option<Single<&Inventory>>,
    mut m: Local<AimMotion>,
    mut out: ResMut<AimOffset>,
) {
    let Some(inv) = inv else { return };
    if !m.init {
        m.init = true;
        m.was_aiming = p.aiming;
        m.rot = Quat::IDENTITY;
    }
    let anims = &inv.slots[inv.current].anims;
    if p.aiming != m.was_aiming {
        m.was_aiming = p.aiming;
        let d = &anims[if p.aiming { "AimIn" } else { "AimOut" }];
        if let Some(c) = curves(d) {
            let v3 = |k: &str, def: Vec3| d[k].as_object().map(|o| Vec3::new(o["x"].as_f64().unwrap_or(1.0) as f32, o["y"].as_f64().unwrap_or(1.0) as f32, o["z"].as_f64().unwrap_or(1.0) as f32)).unwrap_or(def);
            let (lo, hi) = (v3("RotationScaleMin", Vec3::ONE), v3("RotationScaleMax", Vec3::ZERO));
            let hi = if d["RotationScaleMax"].is_object() { hi } else { lo };
            let r = (time.elapsed_secs() * 7.31).fract(); // ponytail: RandomFloatInRange stand-in, deterministic
            let rot_scale = lo.lerp(hi, r);
            // Total travel of the position curve (the graph's loop over the curve), per axis.
            let end = end_time(&c.pos);
            let mut total = Vec3::ZERO;
            let (mut prev, n) = (sample(&c.pos, 0.0), 120);
            for i in 1..=n {
                let cur = sample(&c.pos, end * i as f32 / n as f32);
                total += (cur - prev).abs();
                prev = cur;
            }
            let (cp, cr) = (m.pos, m.rot);
            m.active = Some(Active {
                c,
                t: 0.0,
                playback: d["Playback"].as_f64().unwrap_or(1.0) as f32,
                blend: d["BlendTime"].as_f64().unwrap_or(0.0) as f32,
                cached_pos: cp,
                cached_rot: cr,
                rot_scale,
                acc: Vec3::ZERO,
                total,
            });
        }
    }
    let dt = time.delta_secs();
    let mut done = false;
    let (mut pos, mut rot) = (m.pos, m.rot);
    if let Some(a) = &mut m.active {
        let prev = a.t;
        a.t += dt * a.playback; // ponytail: WeaponAttributeSet ADSSpeed attribute treated as 1.0
        a.acc += (sample(&a.c.pos, a.t) - sample(&a.c.pos, prev)).abs();
        let end_v = sample(&a.c.pos, end_time(&a.c.pos));
        let prog = Vec3::new(ratio(a.acc.x, a.total.x), ratio(a.acc.y, a.total.y), ratio(a.acc.z, a.total.z)).clamp(Vec3::ZERO, Vec3::ONE);
        pos = a.cached_pos * (Vec3::ONE - prog) + end_v * prog;
        let rv = sample(&a.c.rot, a.t) * a.rot_scale;
        let k = if a.blend == 0.0 { 1.0 } else { (a.t / a.blend).clamp(0.0, 1.0) };
        rot = a.cached_rot.slerp(ue_rot(rv.x, rv.y, rv.z), k);
        done = a.t >= end_time(&a.c.pos).max(end_time(&a.c.rot));
    }
    if done {
        m.active = None;
    }
    m.pos = pos;
    m.rot = rot;
    m.offset = Transform::from_translation(ue_vec(pos)) * Transform::from_rotation(rot);
    out.0 = m.offset;
}

fn ratio(a: f32, b: f32) -> f32 {
    if b > 1e-6 { a / b } else { 1.0 }
}

#[derive(Resource, Default)]
pub struct AimOffset(pub Transform);

#[cfg(test)]
mod tests {
    use super::*;

    fn params() -> AxisParams {
        AxisParams { stiffness: 0.6, damping: 0.6, speed: 10.0, smooth: 7.0, extra: 0.0, scale: 5.0, ads_scale: 0.0 }
    }

    #[test]
    fn spring_follows_input_sign_and_settles() {
        let (mut s, p) = (AxisState::default(), params());
        // steady input -> output converges to input * scale * GAIN, same sign
        let mut out = 0.0;
        for _ in 0..600 {
            out = s.step(&p, 1.0, 0.0, 1.0 / 60.0);
        }
        assert!((out - 5.0 * GAIN).abs() < 1e-2, "{out}");
        // input released -> returns to zero
        for _ in 0..600 {
            out = s.step(&p, 0.0, 0.0, 1.0 / 60.0);
        }
        assert!(out.abs() < 1e-3, "{out}");
        // negative input -> negative output
        let mut s = AxisState::default();
        for _ in 0..30 {
            out = s.step(&p, -1.0, 0.0, 1.0 / 60.0);
        }
        assert!(out < 0.0);
    }

    #[test]
    fn ads_scale_replaces_hip_scale() {
        let mut p = params();
        p.ads_scale = 0.0;
        let mut s = AxisState::default();
        let mut out = 1.0;
        for _ in 0..600 {
            out = s.step(&p, 1.0, 1.0, 1.0 / 60.0);
        }
        assert!(out.abs() < 1e-3);
    }

    #[test]
    fn underdamped_overshoots() {
        let (mut s, p) = (AxisState::default(), params());
        let peak = (0..300).map(|_| s.step(&p, 1.0, 0.0, 1.0 / 60.0)).fold(0.0f32, f32::max);
        assert!(peak > 5.0 * GAIN * 1.01); // damping 0.6 -> overshoots
    }
}
