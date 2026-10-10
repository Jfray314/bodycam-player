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
        app.init_resource::<Ads>().init_resource::<Sway>().init_resource::<FakeLook>().init_resource::<AimOffset>().add_systems(Update, (tick_ads, tick_sway, tick_motion).chain());
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
    /// Final gain: the table Scale, times the node's AdsScale pin while aiming.
    pub scale: f32,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct AxisState {
    smoothed: f32,
    x: f32,
    v: f32,
    out: f32,
}

/// Node tuning: the spring output unit per unit of (scaled) input.
// ponytail: GAIN is a guess (the SpringSway output isn't in the recordings, only its inputs); 0.2 / LOOK_IN keeps the
// previous look feel now that the input is the real LookDeltaInput (0.4777 of the old degrees-per-60Hz-frame value).
const GAIN: f32 = 0.2 / LOOK_IN;
/// SpringSway ClampMax pin (ABP_Procedural_Sway json default 10); applied to the input.
const INPUT_CLAMP: f32 = 10.0;
/// ABP_Procedural_Sway LookDeltaInput = control rotation rate (deg/s) / 60 * this, one frame late (tools/probe_sway.py,
/// corr 0.998 against the recordings).
pub const LOOK_IN: f32 = 0.4777;

impl AxisState {
    /// `input` is the node InputValue (LookDeltaInput or a +-1 move axis).
    pub fn step(&mut self, p: &AxisParams, input: f32, dt: f32) -> f32 {
        if p.scale == 0.0 {
            self.out = 0.0;
            return 0.0;
        }
        // 1. smooth the raw input (SmoothDamping = FInterp speed)
        self.smoothed = finterp_to(self.smoothed, input.clamp(-INPUT_CLAMP, INPUT_CLAMP), dt, p.smooth);
        let target = self.smoothed * p.scale * GAIN;
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

/// MagnifiedADSScale (Y, Z) of the sampled M4A1 setup: the aim-pin multiplier of the position nodes.
// ponytail: MagnifiedADSScale varies with the optic (and wobbles with the camera); the M4A1 iron-sight values are used for all.
const MAGNIFIED: [f32; 3] = [1.0, 0.2923, 0.3951];
/// AdsScale pins of the rotation nodes while aiming (x, y, z), constant in the recording.
// ponytail: literals from the M4A1 recording; no table field matches them (tables hold -2/3/3).
const ROT_ADS_PIN: [f32; 3] = [1.0, 0.5, 1.0];

fn axis_params(v: &Value, aiming: bool, pins: [f32; 3]) -> [AxisParams; 3] {
    let g = |k: &str, i: usize| v[k][["x", "y", "z"][i]].as_f64().unwrap_or(0.0) as f32;
    std::array::from_fn(|i| AxisParams {
        stiffness: g("SpringStiffness", i),
        damping: g("SpringDamping", i),
        speed: g("SpringSpeed", i),
        smooth: g("SmoothDamping", i),
        extra: g("ExtraSmoothing", i),
        scale: g("Scale", i) * if aiming { pins[i] } else { 1.0 },
    })
}

/// Spring bank for one output (3 axes).
#[derive(Default)]
struct Bank {
    s: [AxisState; 3],
}

#[derive(Resource, Default)]
pub struct Sway {
    /// Camera-space (Bevy axes, metres) offset the arms root gets, already pivoted about SwayPivotOffset.
    pub offset: Transform,
    pos: Bank,
    rot: Bank,
    mov: Bank,
    /// LookDeltaInput lags the controller by one frame (the ABP updates before the controller).
    prev_look: Vec2,
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

fn g3(v: &Value, table: &str, field: &str, i: usize) -> f32 {
    v[table][field][["x", "y", "z"][i]].as_f64().unwrap_or(0.0) as f32
}

/// Pure spring-bank step used by the system and the tests. Inputs: LookDeltaInput (yaw, pitch; already in node units, this
/// frame's value) and the move axes (fwd, right; +-1). The node parameters switch instantly with IsAiming: aiming uses the
/// Look* tables, hip the HipLook* tables, on the same springs (recording: node params flip on the key frame).
fn run(sw: &mut Sway, anims: &Value, ldi: Vec2, mv: Vec2, aiming: bool, dt: f32) {
    let (pos_t, rot_t) = if aiming { ("LookPositionSway", "LookRotationSway") } else { ("HipLookPositionSway", "HipLookRotationSway") };
    let pos_pins = [1.0, g3(anims, pos_t, "AdsScale", 1) * MAGNIFIED[1], g3(anims, pos_t, "AdsScale", 2) * MAGNIFIED[2]];
    let pp = axis_params(&anims[pos_t], aiming, pos_pins);
    let rp = axis_params(&anims[rot_t], aiming, ROT_ADS_PIN);
    let mp = axis_params(&anims["MovePositionSway"], aiming, [1.0; 3]);
    let (yaw_in, pitch_in) = (ldi.x, ldi.y);
    // Recorded node inputs: position y and rotation x/z take LookDeltaInput.X (yaw), position z and rotation y take .Y (pitch);
    // position x has no look node. ponytail: output signs below are guessed (the node output isn't recorded).
    let pos_in = [0.0, -yaw_in, -pitch_in];
    let rot_in = [-yaw_in, -pitch_in, -yaw_in];
    let mv_in = [mv.x, -mv.y, 0.0];
    let step = |b: &mut Bank, p: &[AxisParams; 3], inp: [f32; 3]| -> Vec3 { Vec3::from_array(std::array::from_fn(|i| b.s[i].step(&p[i], inp[i], dt))) };
    let pos = step(&mut sw.pos, &pp, pos_in) + step(&mut sw.mov, &mp, mv_in);
    let rot = step(&mut sw.rot, &rp, rot_in);
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
    mut sw: ResMut<Sway>,
) {
    let Some(inv) = inv else { return };
    let dt = time.delta_secs().max(1e-4);
    let d = (if cursor.grab_mode == CursorGrabMode::Locked { motion.delta } else { Vec2::ZERO }) + fake.0;
    // counts/frame -> LookDeltaInput (degrees per 60 Hz frame * LOOK_IN), taken one frame late like the ABP
    let look = d * SENS_DEG / (dt * 60.0) * LOOK_IN;
    // Body-local velocity (Bevy x right, z back) -> move axes: the node inputs are ~+-1 whenever moving (saturate by 0.5 m/s).
    let local = Quat::from_rotation_y(p.yaw).inverse() * p.vel;
    let mv = (Vec2::new(-local.z, local.x) / 0.5).clamp(Vec2::splat(-1.0), Vec2::ONE);
    let anims = inv.slots[inv.current].anims.clone();
    if sw.weapon != inv.slots[inv.current].name {
        *sw = Sway { weapon: inv.slots[inv.current].name.to_string(), ..default() };
    }
    let ldi = sw.prev_look;
    sw.prev_look = look;
    run(&mut sw, &anims, ldi, mv, p.aiming, dt);
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
        AxisParams { stiffness: 0.6, damping: 0.6, speed: 10.0, smooth: 7.0, extra: 0.0, scale: 5.0 }
    }

    #[test]
    fn spring_follows_input_sign_and_settles() {
        let (mut s, p) = (AxisState::default(), params());
        // steady input -> output converges to input * scale * GAIN, same sign
        let mut out = 0.0;
        for _ in 0..600 {
            out = s.step(&p, 1.0, 1.0 / 60.0);
        }
        assert!((out - 5.0 * GAIN).abs() < 1e-2 * GAIN, "{out}");
        // input released -> returns to zero
        for _ in 0..600 {
            out = s.step(&p, 0.0, 1.0 / 60.0);
        }
        assert!(out.abs() < 1e-3, "{out}");
        // negative input -> negative output
        let mut s = AxisState::default();
        for _ in 0..30 {
            out = s.step(&p, -1.0, 1.0 / 60.0);
        }
        assert!(out < 0.0);
    }

    #[test]
    fn zero_scale_is_off() {
        let mut p = params();
        p.scale = 0.0;
        let mut s = AxisState::default();
        assert_eq!(s.step(&p, 1.0, 1.0 / 60.0), 0.0);
    }

    #[test]
    fn underdamped_overshoots() {
        let (mut s, p) = (AxisState::default(), params());
        let peak = (0..300).map(|_| s.step(&p, 1.0, 1.0 / 60.0)).fold(0.0f32, f32::max);
        assert!(peak > 5.0 * GAIN * 1.01); // damping 0.6 -> overshoots
    }

    /// The node input, stepped with the control-rotation rate from the recordings, reproduces the recorded LookDeltaInput.X.
    /// before = the previous model (degrees per 60 Hz frame, no lag), after = LOOK_IN with the one-frame lag.
    #[test]
    fn look_delta_input_matches_recording() {
        let r: serde_json::Value = serde_json::from_str(include_str!("../../assets/sway_reference.json")).unwrap();
        let (mut before, mut after, mut sig, mut n) = (0.0f32, 0.0f32, 0.0f32, 0);
        for w in r["look_input"]["series_60hz"].as_array().unwrap() {
            let col = |k: &str| -> Vec<f32> { w[k].as_array().unwrap().iter().map(|v| v.as_f64().unwrap() as f32).collect() };
            let (yaw, ldi) = (col("ctl_yaw"), col("ldi_x"));
            let mut prev = 0.0;
            for i in 1..yaw.len() {
                let per_frame = yaw[i] - yaw[i - 1]; // degrees per 60 Hz frame
                before += (per_frame - ldi[i]).powi(2);
                after += (prev * LOOK_IN - ldi[i]).powi(2);
                sig += ldi[i].powi(2);
                prev = per_frame;
                n += 1;
            }
        }
        let rms = |e: f32| (e / n as f32).sqrt();
        eprintln!("LookDeltaInput.X rms: before {:.3}, after {:.3} (signal rms {:.3})", rms(before), rms(after), rms(sig));
        assert!(rms(after) < 0.3 * rms(before) && rms(after) < 0.35 * rms(sig));
    }

    /// Tables switch with IsAiming on the same springs: toggling mid-flick keeps the state continuous.
    #[test]
    fn aiming_switches_params_not_state() {
        let anims: Value = serde_json::from_str::<Value>(include_str!("../../assets/weapons/M4A1.json")).map(|v| v["anim_set"].clone()).unwrap();
        let mut sw = Sway::default();
        for _ in 0..30 {
            run(&mut sw, &anims, Vec2::new(8.0, 0.0), Vec2::ZERO, false, 1.0 / 60.0);
        }
        let before = sw.offset.translation;
        run(&mut sw, &anims, Vec2::new(8.0, 0.0), Vec2::ZERO, true, 1.0 / 60.0);
        assert!(before.distance(sw.offset.translation) < 0.05, "{before} -> {}", sw.offset.translation);
    }
}
