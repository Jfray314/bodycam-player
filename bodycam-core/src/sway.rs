//! Viewmodel layer: ADS progress + the SpringSway look/move banks + LowReadySway (port of game/src/sway.rs, no Bevy).
//! Conventions as the rest of the core: metres (x right, y up, z back), degrees (pitch up+, yaw right+, roll cw+).
//! Output is a rigid transform for the arms/gun root in camera space: `p' = R(rot) * p + translation`
//! (the rotation already happens about the gun pivot, so the host just applies it to the root).
use crate::weapon_data::WeaponData;
use crate::{Key, curve_eval, finterp_to};

pub const ADS_SPEED_K: f32 = 1.2;
pub const LOOK_IN: f32 = 0.4777;
const GAIN_POS: f32 = 1.8;
const GAIN_ROT: f32 = 1.4;
const GAIN_MOVE: f32 = 1.0;
const ADS_K_POS: f32 = 0.5;
const ADS_K_ROT: f32 = 0.75;
const OMEGA: f32 = 10.0;
const ZETA_PER_DAMPING: f32 = 1.3;
const CLAMP_MAX: f32 = 10.0;
/// ik_hand_gun origin in the camera socket frame (UE cm, X fwd, Y right, Z up), hip / aiming.
// ponytail: M4A1 measured; host may override via `SwayIn::pivot_m`.
const PIVOT_HIP_UE: [f32; 3] = [10.6, 2.9, -28.5];
const PIVOT_ADS_UE: [f32; 3] = [12.4, 0.2, -14.2];

pub fn ads_advance(raw: f32, aiming: bool, speed: f32, dt: f32) -> f32 {
    let step = speed * ADS_SPEED_K * dt;
    raw + ((if aiming { 1.0 } else { 0.0 }) - raw).clamp(-step, step)
}
/// Sinusoidal ease of the linear ADS progress.
// ponytail: other AdsBlend options not mapped.
pub fn ads_ease(raw: f32) -> f32 {
    0.5 - 0.5 * (raw * std::f32::consts::PI).cos()
}

#[derive(Clone, Copy, Default, Debug)]
pub struct AxisParams {
    pub stiffness: f32,
    pub damping: f32,
    pub speed: f32,
    pub smooth: f32,
    pub extra: f32,
    pub scale: f32,
    pub gain: f32,
    pub first_order: bool,
}

#[derive(Clone, Copy, Default, Debug)]
pub struct AxisState {
    x: f32,
    v: f32,
    out: f32,
}

impl AxisState {
    pub fn step(&mut self, p: &AxisParams, input: f32, dt: f32) -> f32 {
        if p.scale == 0.0 {
            self.out = 0.0;
            return 0.0;
        }
        let target = (input * p.scale).clamp(-CLAMP_MAX, CLAMP_MAX) * p.gain;
        if p.first_order {
            self.x = finterp_to(self.x, target, dt, p.smooth);
            self.out = self.x;
            return self.out;
        }
        let (omega, zeta) = (OMEGA, ZETA_PER_DAMPING * p.damping);
        let n = (dt / 0.004).ceil().max(1.0) as i32;
        let h = dt / n as f32;
        for _ in 0..n {
            self.v += (omega * omega * (target - self.x) - 2.0 * zeta * omega * self.v) * h;
            self.x += self.v * h;
        }
        self.out = if p.extra > 0.0 { finterp_to(self.out, self.x, dt, p.extra) } else { self.x };
        self.out
    }
}

fn axis_params(v: &serde_json::Value, gain: f32, first_order: bool) -> [AxisParams; 3] {
    let g = |k: &str, i: usize| v[k][["x", "y", "z"][i]].as_f64().unwrap_or(0.0) as f32;
    std::array::from_fn(|i| AxisParams { stiffness: g("SpringStiffness", i), damping: g("SpringDamping", i), speed: g("SpringSpeed", i), smooth: g("SmoothDamping", i), extra: g("ExtraSmoothing", i), scale: g("Scale", i), gain, first_order })
}

/// LowReadySway (UE cm, negative = gun drops).
pub fn low_ready(cur: f32, ls: &serde_json::Value, yaw_in: f32, aiming: bool, dt: f32) -> f32 {
    let keys: Vec<Key> = serde_json::from_value(ls["Curve"]["curves"]["FloatCurve"].clone()).unwrap_or_default();
    let f = |k: &str| ls[k].as_f64().unwrap_or(0.0) as f32;
    finterp_to(cur, curve_eval(&keys, yaw_in.abs()) * f(if aiming { "AdsScale" } else { "Scale" }), dt, f("InterpSpeed"))
}

fn ue_vec(v: [f32; 3]) -> [f32; 3] {
    [v[1] * 0.01, v[2] * 0.01, -v[0] * 0.01]
}

/// Rotation matrix (row-major, public axes) of `Ry(-yaw) * Rx(pitch) * Rz(-roll)`, degrees: the game's `ue_rot`.
pub fn rot_matrix(pitch: f32, yaw: f32, roll: f32) -> [[f32; 3]; 3] {
    yxz(-yaw.to_radians(), pitch.to_radians(), -roll.to_radians())
}
/// Ry(a) * Rx(b) * Rz(c), radians (glam `EulerRot::YXZ`).
pub(crate) fn yxz(a: f32, b: f32, c: f32) -> [[f32; 3]; 3] {
    let (sy, cy) = a.sin_cos();
    let (sx, cx) = b.sin_cos();
    let (sz, cz) = c.sin_cos();
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rx = [[1.0, 0.0, 0.0], [0.0, cx, -sx], [0.0, sx, cx]];
    let rz = [[cz, -sz, 0.0], [sz, cz, 0.0], [0.0, 0.0, 1.0]];
    mul(mul(ry, rx), rz)
}
pub(crate) fn mul(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| a[i][k] * b[k][j]).sum()))
}

#[derive(Default, Clone, Copy)]
pub struct SwayIn {
    pub dt: f32,
    /// Look this frame in degrees (yaw right+, pitch up+).
    pub look_yaw_deg: f32,
    pub look_pitch_deg: f32,
    /// Character velocity in its own frame, m/s.
    pub vel_fwd: f32,
    pub vel_right: f32,
    pub aiming: bool,
    /// Camera pitch (radians, up+) for the LowReady drop direction.
    pub cam_pitch_rad: f32,
    /// Optional live gun origin (camera space, metres, public axes) overriding the measured hip/ADS pivot.
    pub pivot_m: Option<[f32; 3]>,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct SwayOut {
    pub translation: [f32; 3],
    /// pitch, yaw, roll (deg).
    pub rot_deg: [f32; 3],
    /// ADS alpha (eased).
    pub alpha: f32,
}

#[derive(Default)]
pub struct Sway {
    pos: [AxisState; 3],
    rot: [AxisState; 3],
    mov: [AxisState; 3],
    prev_look: [f32; 2],
    low_ready: f32,
    pub ads_raw: f32,
    pub alpha: f32,
}

impl Sway {
    /// Advance ADS progress only (weapon AdsSettings.AdsSpeed, default 6).
    pub fn tick_ads(&mut self, w: &WeaponData, aiming: bool, dt: f32) {
        let speed = w.anims["AdsSettings"]["AdsSpeed"].as_f64().filter(|s| *s > 0.0).unwrap_or(6.0) as f32;
        self.ads_raw = ads_advance(self.ads_raw, aiming, speed, dt);
        self.alpha = ads_ease(self.ads_raw);
    }

    /// Seconds a full hip<->ADS transition takes.
    pub fn ads_duration(w: &WeaponData) -> f32 {
        1.0 / (w.anims["AdsSettings"]["AdsSpeed"].as_f64().filter(|s| *s > 0.0).unwrap_or(6.0) as f32 * ADS_SPEED_K)
    }

    pub fn step(&mut self, w: &WeaponData, i: &SwayIn) -> SwayOut {
        let dt = i.dt.max(1e-4);
        self.tick_ads(w, i.aiming, dt);
        let look = [i.look_yaw_deg / (dt * 60.0) * LOOK_IN, i.look_pitch_deg / (dt * 60.0) * LOOK_IN];
        let mv = [(i.vel_fwd / 0.5).clamp(-1.0, 1.0), (i.vel_right / 0.5).clamp(-1.0, 1.0)];
        let ldi = self.prev_look; // one frame late like the ABP
        self.prev_look = look;
        let a = &w.anims;
        let (pos_t, rot_t) = if i.aiming { ("LookPositionSway", "LookRotationSway") } else { ("HipLookPositionSway", "HipLookRotationSway") };
        let ads_k = |k: f32| 1.0 + (k - 1.0) * self.alpha;
        let pp = axis_params(&a[pos_t], GAIN_POS * ads_k(ADS_K_POS), false);
        let rp = axis_params(&a[rot_t], GAIN_ROT * ads_k(ADS_K_ROT), false);
        let mp = axis_params(&a["MovePositionSway"], GAIN_MOVE, true);
        let (yaw_in, pitch_in) = (ldi[0], ldi[1]);
        let pos_in = [0.0, -yaw_in, pitch_in];
        let rot_in = [yaw_in, -pitch_in, yaw_in];
        let mv_in = [mv[0], -mv[1], 0.0];
        let step = |b: &mut [AxisState; 3], p: &[AxisParams; 3], inp: [f32; 3]| -> [f32; 3] { std::array::from_fn(|k| b[k].step(&p[k], inp[k], dt)) };
        let (p0, m0) = (step(&mut self.pos, &pp, pos_in), step(&mut self.mov, &mp, mv_in));
        let al = self.alpha;
        let pos: [f32; 3] = std::array::from_fn(|k| p0[k] + m0[k] * (1.0 - al));
        let rot = step(&mut self.rot, &rp, rot_in); // UE roll, pitch, yaw
        let pivot = i.pivot_m.unwrap_or_else(|| ue_vec(std::array::from_fn(|k| PIVOT_HIP_UE[k] + (PIVOT_ADS_UE[k] - PIVOT_HIP_UE[k]) * al)));
        let (pitch, yaw, roll) = (rot[1], rot[2], rot[0]);
        let m = rot_matrix(pitch, yaw, roll);
        let up = ue_vec(pos);
        let mut t: [f32; 3] = std::array::from_fn(|r| up[r] + pivot[r] - (0..3).map(|c| m[r][c] * pivot[c]).sum::<f32>());
        self.low_ready = low_ready(self.low_ready, &a["LowerSway"], ldi[0], i.aiming, dt);
        let lr = self.low_ready * 0.01;
        t[1] += i.cam_pitch_rad.cos() * lr;
        t[2] -= i.cam_pitch_rad.sin() * lr;
        SwayOut { translation: t, rot_deg: [pitch, yaw, roll], alpha: al }
    }
}
