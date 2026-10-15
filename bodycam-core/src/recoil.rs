//! Weapon (viewmodel) recoil animation: port of game/src/recoil.rs (PRAS RecoilAnimationComponent reconstruction).
//! Output: rigid transform for the arms root in camera space, `p' = M p + t` (m, x right, y up, z back).
use crate::gunfire::Uniform;
use crate::sway::{mul, yxz};
use crate::{Key, curve_eval, finterp_to};
use serde_json::Value;

/// Grip pivot (camera space, m) the curve rotation acts about.
const GRIP: [f32; 3] = [0.09, -0.22, -0.13];

type Curves = Vec<Vec<Key>>;
type V3 = [f32; 3];
type V2 = [f32; 2];

fn range(a: f32, b: f32, u: f32) -> f32 {
    if a == b { a } else { a.min(b) + (a.max(b) - a.min(b)) * u }
}
fn r2(v: &Value, rng: &mut impl Uniform) -> f32 {
    let (a, b) = (v["x"].as_f64().unwrap_or(0.0) as f32, v["y"].as_f64().unwrap_or(0.0) as f32);
    if a == b { a } else { range(a, b, rng.next01()) }
}
/// 4D range (x,y | z,w): one of the two 2D ranges at random.
fn r4(v: &Value, rng: &mut impl Uniform) -> f32 {
    let f = |k: &str| v[k].as_f64().unwrap_or(0.0) as f32;
    let (a, b) = if rng.next01() < 0.5 { (f("x"), f("y")) } else { (f("z"), f("w")) };
    if a == b { a } else { range(a, b, rng.next01()) }
}
fn f(v: &Value, k: &str) -> f32 {
    v[k].as_f64().unwrap_or(0.0) as f32
}
fn curves(d: &Value, key: &str) -> Curves {
    serde_json::from_value(d[key]["curves"]["FloatCurves"].clone()).unwrap_or_default()
}
fn apply(m: &[[f32; 3]; 3], v: V3) -> V3 {
    std::array::from_fn(|r| (0..3).map(|c| m[r][c] * v[c]).sum())
}

#[derive(Default)]
pub struct Recoil {
    /// Camera-space gun transform for this frame: p' = m p + t.
    pub m: [[f32; 3]; 3],
    pub t: V3,
    /// PRAS output as the probe records it (UE): OutRot (pitch, yaw, roll) deg, OutLoc (X fwd, Y right, Z up) cm.
    pub out_rot: V3,
    pub out_loc: V3,
    shots: u32,
    since_shot: f32,
    interval: f32,
    auto: bool,
    aiming: bool,
    roll_sign: f32,
    rot: V3,
    loc: V3,
    noise_t: V2,
    noise: V2,
    push_t: f32,
    push: f32,
    z_t: f32,
    z: f32,
    sway_t: V2,
    sway: V2,
    fire_sway: V2,
    layers: Vec<(f32, [Curves; 2], V3, V3)>,
    data: Value,
}

impl Recoil {
    /// One shot. `d` = the weapon's anim_set["RecoilData"], `interval` = fire rate, `full_auto` = fires while held.
    pub fn play(&mut self, d: &Value, aiming: bool, interval: f32, full_auto: bool, rng: &mut impl Uniform) {
        let bursting = self.since_shot < interval * 1.5 + 0.05;
        self.shots = if bursting { self.shots + 1 } else { 1 };
        self.auto = full_auto && self.shots > 1;
        self.aiming = aiming;
        self.interval = interval;
        self.since_shot = 0.0;
        self.data = d.clone();
        let pre = if aiming { "Aim" } else { "" };
        let kind = if self.auto { "Auto" } else { "Single" };
        let cs = [curves(d, &format!("{pre}{kind}Rot")), curves(d, &format!("{pre}{kind}Loc"))];
        let s = if aiming { "Aim" } else { "Hip" };
        let mut roll = r4(&d[format!("Roll{s}")], rng);
        if d["bSmoothRoll"].as_bool().unwrap_or(true) {
            self.roll_sign = if self.roll_sign > 0.0 { -1.0 } else { 1.0 };
            roll = roll.abs() * self.roll_sign;
        }
        let rot_t = [roll, r2(&d[format!("Pitch{s}")], rng), r4(&d[format!("Yaw{s}")], rng)];
        let loc_t = [r2(&d[format!("Kick{s}")], rng), 0.0, r2(&d[format!("Kick{s}Up")], rng)];
        // ponytail: MultiRot/MultiLoc and SmoothRot/SmoothLoc not applied (they fit the probe worse), as in the game.
        self.layers.clear(); // a new shot replaces the previous curve
        self.layers.push((0.0, cs, rot_t, loc_t));
        let ns = if aiming { f(d, "NoiseAimScalar") } else { 1.0 };
        let n = &d["Noise2DTarget"];
        let (u1, u2) = (rng.next01(), rng.next01());
        self.noise_t[0] += (-1.0 + 2.0 * u1) * f(n, "x") * ns;
        self.noise_t[1] += (-1.0 + 2.0 * u2) * f(n, "y") * ns;
        if full_auto && self.shots == 2 {
            self.push_t = f(d, "Pushback");
        }
        if self.auto {
            self.z_t += f(&d["ZProgress"], "Amount") * if aiming { f(d, "ProgressAimAlpha") } else { 1.0 };
        }
        let sw = &d["RecoilSway"];
        let ads = if aiming { f(sw, "AdsScale") } else { 1.0 };
        self.sway_t[0] += r2(&sw["PitchSway"], rng) * ads;
        self.sway_t[1] += r2(&sw["YawSway"], rng) * ads;
    }

    pub fn tick(&mut self, dt: f32) {
        self.since_shot += dt;
        let d = &self.data;
        let bursting = self.since_shot < self.interval * 1.5 + 0.05;
        if !bursting {
            self.shots = 0;
        }
        let (mut rot, mut loc) = ([0.0f32; 3], [0.0f32; 3]);
        self.layers.retain(|l| l.0 < 2.0);
        for (age, [rc, lc], rt, lt) in &mut self.layers {
            *age += dt;
            let c = |k: &Curves, i: usize| k.get(i).map(|k| curve_eval(k, *age)).unwrap_or(0.0);
            for i in 0..3 {
                rot[i] += c(rc, i) * rt[i];
                loc[i] += c(lc, i) * lt[i];
            }
        }
        (self.rot, self.loc) = (rot, loc);
        let (na, nz) = (&d["Noise2DAccel"], &d["Noise2DZero"]);
        self.noise[0] = finterp_to(self.noise[0], self.noise_t[0], dt, f(na, "x"));
        self.noise[1] = finterp_to(self.noise[1], self.noise_t[1], dt, f(na, "y"));
        self.noise_t[0] = finterp_to(self.noise_t[0], 0.0, dt, f(nz, "x"));
        self.noise_t[1] = finterp_to(self.noise_t[1], 0.0, dt, f(nz, "y"));
        self.push = finterp_to(self.push, self.push_t, dt, f(d, "PushbackAccel"));
        self.push_t = finterp_to(self.push_t, 0.0, dt, f(d, "PushbackZero"));
        let zp = &d["ZProgress"];
        if !bursting {
            self.z_t = finterp_to(self.z_t, 0.0, dt, f(zp, "Damping"));
        }
        self.z = finterp_to(self.z, self.z_t, dt, f(zp, "Accel"));
        let sw = &d["RecoilSway"];
        self.sway[0] = finterp_to(self.sway[0], self.sway_t[0], dt, f(sw, "Acceleration"));
        self.sway[1] = finterp_to(self.sway[1], self.sway_t[1], dt, f(sw, "Acceleration"));
        self.sway_t[0] = finterp_to(self.sway_t[0], 0.0, dt, f(sw, "Damping"));
        self.sway_t[1] = finterp_to(self.sway_t[1], 0.0, dt, f(sw, "Damping"));
        let fs = &d["FireSway"];
        let target: V2 = if bursting && self.shots > 0 {
            // ponytail: step/clamp fit and shot cap 6 are fitted constants, see game/src/recoil.rs.
            let n = (self.shots as f32).min(6.0);
            let (vs, ds) = (f(fs, "VerticalOffsetStep"), f(fs, "DepthOffsetStep"));
            let (vs, ds) = if vs > 0.0 { (vs, ds) } else { (4.0, 4.0) };
            let (cv, cd) = (f(fs, "ClampVerticalOffset"), f(fs, "ClampDepthOffset"));
            if self.aiming {
                [0.0, 0.0]
            } else if vs > 0.0 {
                [(vs * n).min(cv), (ds * n).min(cd)]
            } else {
                let keys: Vec<Key> = serde_json::from_value(fs["OffsetCurve"]["curves"]["FloatCurve"].clone()).unwrap_or_default();
                let v = curve_eval(&keys, n);
                [v.min(cv), v.min(cd)]
            }
        } else {
            [0.0, 0.0]
        };
        let speed = if bursting { f(fs, "InterpSpeed") } else { f(fs, "DampSmoothing") };
        self.fire_sway[0] = finterp_to(self.fire_sway[0], target[0], dt, speed);
        self.fire_sway[1] = finterp_to(self.fire_sway[1], target[1], dt, speed);

        let kick = self.loc[0] + self.push + self.fire_sway[1];
        let right = self.loc[1] + self.noise[0];
        let up = self.loc[2] + self.z + self.noise[1] + self.fire_sway[0];
        self.out_rot = [self.rot[1], self.rot[2], self.rot[0]];
        self.out_loc = [kick, right, up];
        let t = [-right * 0.01, up * 0.01, -kick * 0.01];
        let r = |deg: f32| deg.to_radians();
        let c = yxz(r(-self.rot[2]), r(-self.rot[1]), r(-self.rot[0]));
        let roll_sway = self.sway[1] * f(sw, "RollSwayMultiplier");
        let s = yxz(r(-self.sway[1]), r(-self.sway[0]), r(-roll_sway));
        let po = &sw["PivotOffset"];
        let pivot = [GRIP[0] + f(po, "y") * 0.01, GRIP[1] + f(po, "z") * 0.01, GRIP[2] - f(po, "x") * 0.01];
        // offset = T(t) * about(pivot, S) * about(GRIP, C)
        let ca = apply(&c, [-GRIP[0], -GRIP[1], -GRIP[2]]);
        let inner: V3 = std::array::from_fn(|k| ca[k] + GRIP[k] - pivot[k]);
        let sa = apply(&s, inner);
        self.m = mul(s, c);
        self.t = std::array::from_fn(|k| sa[k] + pivot[k] + t[k]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gunfire::Rng;

    fn data() -> Value {
        serde_json::json!({
            "PitchHip": {"x": -2.0, "y": -2.0}, "RollHip": {"x": 1.0, "y": 1.0, "z": 1.0, "w": 1.0}, "YawHip": {"x": 0.5, "y": 0.5, "z": 0.5, "w": 0.5},
            "KickHip": {"x": -3.0, "y": -3.0}, "KickHipUp": {"x": 1.0, "y": 1.0},
            "SingleRot": {"curves": {"FloatCurves": [[[0.0,0.0,0.0,0.0,0.0],[0.05,1.0,0.0,0.0,0.0],[0.3,0.0,0.0,0.0,0.0]],[[0.0,0.0,0.0,0.0,0.0],[0.05,1.0,0.0,0.0,0.0],[0.3,0.0,0.0,0.0,0.0]],[[0.0,0.0,0.0,0.0,0.0],[0.05,1.0,0.0,0.0,0.0],[0.3,0.0,0.0,0.0,0.0]]]}},
            "SingleLoc": {"curves": {"FloatCurves": [[[0.0,0.0,0.0,0.0,0.0],[0.05,1.0,0.0,0.0,0.0],[0.3,0.0,0.0,0.0,0.0]],[],[[0.0,0.0,0.0,0.0,0.0],[0.05,1.0,0.0,0.0,0.0],[0.3,0.0,0.0,0.0,0.0]]]}},
            "RecoilSway": {"Acceleration": 10.0, "Damping": 5.0}, "Noise2DAccel": {"x": 5.0, "y": 5.0}, "Noise2DZero": {"x": 5.0, "y": 5.0}
        })
    }

    #[test]
    fn single_shot_kicks_then_settles() {
        let (mut r, mut rng) = (Recoil::default(), Rng::new(9));
        r.play(&data(), false, 0.1, false, &mut rng);
        let mut peak = 0.0f32;
        for _ in 0..12 {
            r.tick(1.0 / 240.0);
            peak = peak.max(r.out_rot[0].abs());
        }
        assert!((peak - 2.0).abs() < 0.1, "{peak}"); // pitch target -2 deg at the curve peak
        assert!(r.out_loc[0] < 0.0 && r.out_loc[2] > 0.0);
        for _ in 0..240 {
            r.tick(1.0 / 240.0);
        }
        assert!(r.out_rot.iter().all(|v| v.abs() < 1e-3) && r.t.iter().all(|v| v.abs() < 1e-3), "{:?} {:?}", r.out_rot, r.t);
    }
}
