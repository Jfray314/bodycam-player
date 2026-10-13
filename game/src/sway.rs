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
    ads.duration = 1.0 / (speed * ADS_SPEED_K);
    ads.raw = ads_advance(ads.raw, p.aiming, speed, time.delta_secs());
    ads.alpha = ads_ease(ads.raw);
}

/// Recorded gun origin reaches the ADS pose 1.2x faster than 1/AdsSpeed (tools/probe_sway.py gun.ads: sinusoidal fit speed 2.75
/// vs the Ads node pin 2.3, both M4A1 recordings).
// ponytail: factor measured on the M4A1 only (pin 2.3); a per-weapon attribute may be behind it.
pub const ADS_SPEED_K: f32 = 1.2;

pub fn ads_advance(raw: f32, aiming: bool, speed: f32, dt: f32) -> f32 {
    let step = speed * ADS_SPEED_K * dt;
    raw + ((if aiming { 1.0 } else { 0.0 }) - raw).clamp(-step, step)
}

/// AdsBlend.BlendOption is Sinusoidal on the sampled weapons (BlendTime 0 = unused in the graph); smoothstep fits as well.
// ponytail: ease shape fixed to sinusoidal; other BlendOptions not mapped.
pub fn ads_ease(raw: f32) -> f32 {
    0.5 - 0.5 * (raw * std::f32::consts::PI).cos()
}

/// One axis of the spring node.
#[derive(Clone, Copy, Default, Debug)]
pub struct AxisParams {
    pub stiffness: f32,
    pub damping: f32,
    pub speed: f32,
    /// Table SmoothDamping = node DampingSpeed: FInterp rate of the move (first-order) bank.
    pub smooth: f32,
    pub extra: f32,
    /// Final gain in output units (cm / deg): the table Scale, times the node's AdsScale pin while aiming, times the fitted gain.
    pub scale: f32,
    /// Move bank: output = FInterp(smooth) of the input (no spring).
    pub first_order: bool,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct AxisState {
    x: f32,
    v: f32,
    out: f32,
}

/// Fitted to the recorded camera-space gun pose (tools/probe_sway.py gun.look / gun.move, M4A1 hip): output (cm / deg) per
/// unit of Scale * input, spring natural frequency, and damping ratio per table SpringDamping.
// ponytail: SpringStiffness / SpringSpeed / SmoothDamping had no measurable effect on the hip look springs (omega 9-10.5 for
// stiffness .7-1, speed 7-10), and there is no look input while aiming in the recordings, so the ADS tables use the same constants.
const GAIN_POS: f32 = 0.55;
const GAIN_ROT: f32 = 0.41;
const GAIN_MOVE: f32 = 1.0;
const OMEGA: f32 = 10.0;
const ZETA_PER_DAMPING: f32 = 1.3;
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
        let target = input.clamp(-INPUT_CLAMP, INPUT_CLAMP) * p.scale;
        if p.first_order {
            self.x = finterp_to(self.x, target, dt, p.smooth);
            self.out = self.x;
            return self.out;
        }
        // damped spring toward the target (semi-implicit Euler, substepped)
        let (omega, zeta) = (OMEGA, ZETA_PER_DAMPING * p.damping);
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

fn axis_params(v: &Value, aiming: bool, pins: [f32; 3], gain: f32, first_order: bool) -> [AxisParams; 3] {
    let g = |k: &str, i: usize| v[k][["x", "y", "z"][i]].as_f64().unwrap_or(0.0) as f32;
    std::array::from_fn(|i| AxisParams {
        stiffness: g("SpringStiffness", i),
        damping: g("SpringDamping", i),
        speed: g("SpringSpeed", i),
        smooth: g("SmoothDamping", i),
        extra: g("ExtraSmoothing", i),
        scale: g("Scale", i) * if aiming { pins[i] } else { 1.0 } * gain,
        first_order,
    })
}

/// Spring bank for one output (3 axes).
#[derive(Default)]
struct Bank {
    s: [AxisState; 3],
}

#[derive(Resource, Default)]
pub struct Sway {
    /// Camera-space (Bevy axes, metres) offset the arms root gets, already pivoted about the gun origin.
    pub offset: Transform,
    pos: Bank,
    rot: Bank,
    mov: Bank,
    /// LookDeltaInput lags the controller by one frame (the ABP updates before the controller).
    prev_look: Vec2,
    /// ADS alpha, only used to move the rotation pivot from the hip to the ADS gun origin.
    pub alpha: f32,
    /// LowReadySway (UE cm), see `low_ready`.
    low_ready: f32,
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
/// ik_hand_gun origin in the FP_Camera socket frame (UE cm, X fwd, Y right, Z up) hip / aiming, measured in the M4A1 recordings
/// (tools/probe_sway.py): the recorded sway rotates the gun about this point (the position response shows no lever arm).
// ponytail: ADS origin is the M4A1 iron-sight pose; other weapons/optics sit elsewhere. The BAS SwayPivotOffset table value is not used.
const PIVOT_HIP_UE: Vec3 = Vec3::new(10.6, 2.9, -28.5);
const PIVOT_ADS_UE: Vec3 = Vec3::new(12.4, 0.2, -14.2);

fn g3(v: &Value, table: &str, field: &str, i: usize) -> f32 {
    v[table][field][["x", "y", "z"][i]].as_f64().unwrap_or(0.0) as f32
}

/// Pure spring-bank step used by the system and the tests. Inputs: LookDeltaInput (yaw + = turn right, pitch + = look up, UE
/// signs; already in node units, this frame's value) and the move axes (fwd, right; +-1). The node parameters switch instantly with IsAiming: aiming uses the
/// Look* tables, hip the HipLook* tables, on the same springs (recording: node params flip on the key frame).
fn run(sw: &mut Sway, anims: &Value, ldi: Vec2, mv: Vec2, aiming: bool, dt: f32) {
    let (pos_t, rot_t) = if aiming { ("LookPositionSway", "LookRotationSway") } else { ("HipLookPositionSway", "HipLookRotationSway") };
    let pos_pins = [1.0, g3(anims, pos_t, "AdsScale", 1) * MAGNIFIED[1], g3(anims, pos_t, "AdsScale", 2) * MAGNIFIED[2]];
    let pp = axis_params(&anims[pos_t], aiming, pos_pins, GAIN_POS, false);
    let rp = axis_params(&anims[rot_t], aiming, ROT_ADS_PIN, GAIN_ROT, false);
    let mp = axis_params(&anims["MovePositionSway"], aiming, [1.0; 3], GAIN_MOVE, true);
    let (yaw_in, pitch_in) = (ldi.x, ldi.y);
    // Fitted to the recorded gun pose (signs per +1 input): position y -yaw, position z +pitch, rotation yaw +yaw, rotation pitch -pitch.
    // Position x has no look node. ponytail: the roll (rotation x) sign is unobserved (ADS-only node, no look input while aiming).
    let pos_in = [0.0, -yaw_in, pitch_in];
    let rot_in = [-yaw_in, -pitch_in, yaw_in];
    let mv_in = [mv.x, -mv.y, 0.0];
    let step = |b: &mut Bank, p: &[AxisParams; 3], inp: [f32; 3]| -> Vec3 { Vec3::from_array(std::array::from_fn(|i| b.s[i].step(&p[i], inp[i], dt))) };
    let pos = step(&mut sw.pos, &pp, pos_in) + step(&mut sw.mov, &mp, mv_in);
    let rot = step(&mut sw.rot, &rp, rot_in);
    let pivot = ue_vec(PIVOT_HIP_UE.lerp(PIVOT_ADS_UE, sw.alpha));
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
    // counts/frame -> LookDeltaInput (degrees per 60 Hz frame * LOOK_IN), taken one frame late like the ABP
    let look = Vec2::new(d.x, -d.y) * SENS_DEG / (dt * 60.0) * LOOK_IN; // UE signs: mouse down = look down
    // Body-local velocity (Bevy x right, z back) -> move axes: the node inputs are ~+-1 whenever moving (saturate by 0.5 m/s).
    let local = Quat::from_rotation_y(p.yaw).inverse() * p.vel;
    let mv = (Vec2::new(-local.z, local.x) / 0.5).clamp(Vec2::splat(-1.0), Vec2::ONE);
    let anims = inv.slots[inv.current].anims.clone();
    if sw.weapon != inv.slots[inv.current].name {
        *sw = Sway { weapon: inv.slots[inv.current].name.to_string(), ..default() };
    }
    let ldi = sw.prev_look;
    sw.prev_look = look;
    sw.alpha = ads.alpha;
    run(&mut sw, &anims, ldi, mv, p.aiming, dt);
    let lr = low_ready(sw.low_ready, &anims["LowerSway"], ldi.x, p.aiming, dt);
    sw.low_ready = lr;
    // ModifyBone VB ik_hand_gun, translation Z in component space (ALS mesh: yaw only -> world up) -> camera space.
    sw.offset.translation += Vec3::new(0.0, p.pitch.cos(), -p.pitch.sin()) * lr * 0.01;
}

/// ABP_Procedural_Sway LowReadySway (UE cm, negative = gun drops): FInterpTo(Curve(|LookDeltaInput.X|) * (aiming ? AdsScale : Scale),
/// InterpSpeed) from the BAS LowerSway settings. Decoded from the ABP bytecode; matches the recorded value to 0.07 cm rms.
fn low_ready(cur: f32, ls: &Value, yaw_in: f32, aiming: bool, dt: f32) -> f32 {
    let keys: Vec<Key> = serde_json::from_value(ls["Curve"]["curves"]["FloatCurve"].clone()).unwrap_or_default();
    let f = |k: &str| ls[k].as_f64().unwrap_or(0.0) as f32;
    finterp_to(cur, eval(&keys, yaw_in.abs()) * f(if aiming { "AdsScale" } else { "Scale" }), dt, f("InterpSpeed"))
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
        AxisParams { stiffness: 0.6, damping: 0.6, speed: 10.0, smooth: 7.0, extra: 0.0, scale: 5.0, first_order: false }
    }

    #[test]
    fn spring_follows_input_sign_and_settles() {
        let (mut s, p) = (AxisState::default(), params());
        // steady input -> output converges to input * scale, same sign
        let mut out = 0.0;
        for _ in 0..600 {
            out = s.step(&p, 1.0, 1.0 / 60.0);
        }
        assert!((out - 5.0).abs() < 1e-2, "{out}");
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
        assert!(peak > 5.0 * 1.01); // damping 0.6 -> overshoots
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

    fn col(v: &Value) -> Vec<f32> {
        v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect()
    }
    /// Gun-origin delta (UE cm: X fwd, Y right, Z up) and rotation (UE roll, pitch, yaw deg) implied by the sway transform.
    fn gun_delta(sw: &Sway, origin_ue: Vec3) -> ([f32; 3], [f32; 3]) {
        let g = ue_vec(origin_ue);
        let d = sw.offset.transform_point(g) - g;
        let rv = sw.offset.rotation.to_scaled_axis().map(f32::to_degrees);
        ([-d.z * 100.0, d.x * 100.0, d.y * 100.0], [-rv.z, rv.x, -rv.y])
    }
    fn m4a1() -> Value {
        serde_json::from_str::<Value>(include_str!("../../assets/weapons/M4A1.json")).map(|v| v["anim_set"].clone()).unwrap()
    }

    fn step_look(sw: &mut Sway, anims: &Value, ldi: Vec2, dt: f32) {
        run(sw, anims, ldi, Vec2::ZERO, false, dt);
    }

    /// ADS alpha stepped with the recorded frame times vs the recorded gun-origin progress hip -> ADS (first 0.3 s after IsAiming flips).
    #[test]
    fn ads_alpha_matches_recorded_transition() {
        let r: Value = serde_json::from_str(include_str!("../../assets/sway_reference.json")).unwrap();
        let rms = |k: f32| {
            let (mut e, mut n) = (0f32, 0f32);
            for (dir, aim) in [("in", true), ("out", false)] {
                for w in r["gun"]["ads"]["series"][dir].as_array().unwrap() {
                    let (t, p) = (col(&w["t"]), col(&w["progress"]));
                    let mut raw: f32 = if aim { 0.0 } else { 1.0 };
                    for i in 0..t.len() {
                        let dt = if i == 0 { 0.015 } else { t[i] - t[i - 1] };
                        let step = 2.3 * k * dt;
                        raw += ((if aim { 1.0 } else { 0.0 }) - raw).clamp(-step, step);
                        e += (ads_ease(raw) - p[i]).powi(2);
                        n += 1.0;
                    }
                }
            }
            (e / n).sqrt()
        };
        let (before, after) = (rms(1.0), rms(ADS_SPEED_K));
        eprintln!("ADS alpha vs recorded progress rms: before {before:.3}, after {after:.3}");
        assert!(after < 0.8 * before);
        // the production stepper agrees with the test's
        assert!((ads_advance(0.0, true, 2.3, 0.1) - 0.1 * 2.3 * ADS_SPEED_K).abs() < 1e-6);
    }

    /// Strafe move spring (hip MovePositionSway) vs the recorded gun-origin Y in the walking windows.
    #[test]
    fn move_sway_matches_recorded_gun_pose() {
        let r: Value = serde_json::from_str(include_str!("../../assets/sway_reference.json")).unwrap();
        let m = &r["gun"]["move"];
        let origin = Vec3::from_array(col(&r["gun"]["look"]["gun_origin_hip_cm"]).try_into().unwrap());
        let anims = m4a1();
        let (mut e, mut sig, mut n) = (0f32, 0f32, 0f32);
        for w in m["windows"].as_array().unwrap() {
            let (t, strafe, y) = (col(&w["t"]), col(&w["strafe"]), col(&w["pos_y_cm"]));
            let mut sw = Sway::default();
            let pred: Vec<f32> = (0..t.len())
                .map(|i| {
                    run(&mut sw, &anims, Vec2::ZERO, Vec2::new(0.0, strafe[i]), false, if i == 0 { 0.0169 } else { t[i] - t[i - 1] });
                    gun_delta(&sw, origin).0[1]
                })
                .collect();
            let (mp, my) = (pred.iter().sum::<f32>() / t.len() as f32, y.iter().sum::<f32>() / t.len() as f32);
            for i in 0..t.len() {
                e += (pred[i] - mp - (y[i] - my)).powi(2);
                sig += (y[i] - my).powi(2);
                n += 1.0;
            }
        }
        eprintln!("move sway vs recorded gun Y rms: {:.3} cm (signal {:.3} cm)", (e / n).sqrt(), (sig / n).sqrt());
        assert!(e < 0.5 * sig);
    }

    /// Hip look sway stepped with the recorded node inputs reproduces the recorded camera-space gun pose
    /// (assets/sway_reference.json gun.look; recording minus its pitch-angle baseline). Prints RMS (cm, deg).
    #[test]
    fn look_sway_matches_recorded_gun_pose() {
        let r: Value = serde_json::from_str(include_str!("../../assets/sway_reference.json")).unwrap();
        let l = &r["gun"]["look"];
        let s = &l["series"];
        let (t, ldx, ldy) = (col(&s["t"]), col(&s["ldi_x"]), col(&s["ldi_y"]));
        let g = &s["gun_ref"];
        let origin = Vec3::from_array(col(&l["gun_origin_hip_cm"]).try_into().unwrap());
        let anims = m4a1();
        let mut sw = Sway::default();
        let (mut pos, mut rot, mut psig, mut rsig, mut n) = ([0f32; 3], [0f32; 3], 0f32, 0f32, 0f32);
        let refs: Vec<Vec<f32>> = ["pos_x", "pos_y", "pos_z", "rot_roll", "rot_pitch", "rot_yaw"].iter().map(|k| col(&g[*k])).collect();
        for i in 0..t.len() {
            let dt = if i == 0 { 0.0138 } else { t[i] - t[i - 1] };
            step_look(&mut sw, &anims, Vec2::new(ldx[i], ldy[i]), dt);
            let (p, q) = gun_delta(&sw, origin);
            for c in 0..3 {
                pos[c] += (p[c] - refs[c][i]).powi(2);
                rot[c] += (q[c] - refs[3 + c][i]).powi(2);
                psig += refs[c][i].powi(2);
                rsig += refs[3 + c][i].powi(2);
            }
            n += 1.0;
        }
        let (pc, rd) = ((pos.iter().sum::<f32>() / (3.0 * n)).sqrt(), (rot.iter().sum::<f32>() / (3.0 * n)).sqrt());
        eprintln!("look sway vs recorded gun pose rms: {pc:.3} cm ({:.3} cm signal), {rd:.3} deg ({:.3} deg signal); per axis pos {:?} rot {:?}",
            (psig / (3.0 * n)).sqrt(), (rsig / (3.0 * n)).sqrt(), pos.map(|v| (v / n).sqrt()), rot.map(|v| (v / n).sqrt()));
        assert!(pc < 0.9 * (psig / (3.0 * n)).sqrt() && rd < 0.9 * (rsig / (3.0 * n)).sqrt());
    }

    /// Fast yaw lowers the gun (M4A1 LowerSway: curve -1 at |input| 4.56, AdsScale 25, Scale 8, InterpSpeed 5) and it recovers at rest.
    #[test]
    fn low_ready_drops_on_fast_yaw() {
        let ls = m4a1()["LowerSway"].clone();
        let run_for = |mut v: f32, inp: f32, aim: bool, secs: f32| { for _ in 0..(secs * 60.0) as i32 { v = low_ready(v, &ls, inp, aim, 1.0 / 60.0); } v };
        let ads = run_for(0.0, -6.0, true, 2.0);
        assert!((ads + 25.0).abs() < 0.5, "{ads}");
        assert!((run_for(0.0, 6.0, false, 2.0) + 8.0).abs() < 0.5);
        assert!(run_for(ads, 0.0, true, 2.0).abs() < 0.5);
        assert!(run_for(0.0, 1.0, true, 2.0).abs() < 1.0); // slow turns barely move it
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
