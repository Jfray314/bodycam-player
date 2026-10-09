//! Weapon recoil animation: port of the PRAS RecoilAnimationComponent (Kinemation "Procedural Recoil Animation
//! System", native C++ in Bodycam - reconstructed from its RecoilData fields and docs). Every shot picks random
//! targets (Pitch/Yaw/Roll, Kick/KickUp; hip or aim ranges) that the Single/Auto Rot/Loc vector curves lerp to
//! (`lerp(0, target, curve(t))`), plus a 2D noise layer, pushback, Z progress, a recoil-sway spring around a
//! shoulder pivot and FireSway climb over the burst. Output is a camera-space offset applied to the arms root,
//! so the gun (and the hands holding it) kicks while the camera only gets the fire shake + controller recoil.
use crate::curve::{Key, eval};
use crate::status::finterp_to;
use crate::weapon::{Fired, Inventory};
use bevy::prelude::*;
use rand::Rng;
use serde_json::Value;

pub struct RecoilPlugin;

impl Plugin for RecoilPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Recoil>().add_systems(Update, tick);
    }
}

/// Grip pivot (camera space, m) the curve rotation acts about - same point the BAS PoseOffset uses.
const GRIP: Vec3 = Vec3::new(0.09, -0.22, -0.13);

#[derive(Resource, Default)]
pub struct Recoil {
    /// Camera-space (Bevy axes, metres) gun offset for this frame.
    pub offset: Transform,
    t: f32,
    shots: u32,
    since_shot: f32,
    interval: f32,
    auto: bool,
    aiming: bool,
    roll_sign: f32,
    /// Targets: (roll, pitch, yaw) degrees, (kick fwd, right, up) cm - UE axes.
    rot_t: Vec3,
    loc_t: Vec3,
    rot: Vec3,
    loc: Vec3,
    noise_t: Vec2,
    noise: Vec2,
    push_t: f32,
    push: f32,
    z_t: f32,
    z: f32,
    sway_t: Vec2,
    sway: Vec2,
    fire_sway: Vec2,
    curves: Option<[Vec<Vec<Key>>; 2]>,
    data: Value,
}

fn r2(v: &Value, rng: &mut impl Rng) -> f32 {
    let (a, b) = (v["x"].as_f64().unwrap_or(0.0) as f32, v["y"].as_f64().unwrap_or(0.0) as f32);
    if a == b { a } else { rng.random_range(a.min(b)..=a.max(b)) }
}

/// 4D range: FRandRange(FRandRange(x, y), FRandRange(z, w)) - avoids near-zero values.
fn r4(v: &Value, rng: &mut impl Rng) -> f32 {
    let f = |k: &str| v[k].as_f64().unwrap_or(0.0) as f32;
    let (a, b) = (rng.random_range(f("x").min(f("y"))..=f("x").max(f("y"))), rng.random_range(f("z").min(f("w"))..=f("z").max(f("w"))));
    if a == b { a } else { rng.random_range(a.min(b)..=a.max(b)) }
}

fn f(v: &Value, k: &str) -> f32 {
    v[k].as_f64().unwrap_or(0.0) as f32
}

fn curves(d: &Value, key: &str) -> Vec<Vec<Key>> {
    serde_json::from_value(d[key]["curves"]["FloatCurves"].clone()).unwrap_or_default()
}

impl Recoil {
    /// One shot. `interval` = seconds between shots (fire rate), `full_auto` = weapon fires while held.
    pub fn play(&mut self, d: &Value, aiming: bool, interval: f32, full_auto: bool, rng: &mut impl Rng) {
        let bursting = self.since_shot < interval * 1.5 + 0.05;
        self.shots = if bursting { self.shots + 1 } else { 1 };
        // The first shot is always a single shot; later shots of a held trigger use the auto curves.
        self.auto = full_auto && self.shots > 1;
        self.aiming = aiming;
        self.interval = interval;
        self.since_shot = 0.0;
        self.t = 0.0;
        self.data = d.clone();
        let pre = if aiming { "Aim" } else { "" };
        let kind = if self.auto { "Auto" } else { "Single" };
        self.curves = Some([curves(d, &format!("{pre}{kind}Rot")), curves(d, &format!("{pre}{kind}Loc"))]);
        let s = if aiming { "Aim" } else { "Hip" };
        let mut roll = r4(&d[format!("Roll{s}")], rng);
        if d["bSmoothRoll"].as_bool().unwrap_or(true) {
            // Roll sign alternates each shot.
            self.roll_sign = if self.roll_sign > 0.0 { -1.0 } else { 1.0 };
            roll = roll.abs() * self.roll_sign;
        }
        self.rot_t = Vec3::new(roll, r2(&d[format!("Pitch{s}")], rng), r4(&d[format!("Yaw{s}")], rng));
        self.loc_t = Vec3::new(r2(&d[format!("Kick{s}")], rng), 0.0, r2(&d[format!("Kick{s}Up")], rng));
        if self.auto {
            let (mr, ml) = (&d["MultiRot"], &d["MultiLoc"]);
            self.rot_t *= Vec3::new(f(mr, "roll"), f(mr, "pitch"), f(mr, "yaw"));
            self.loc_t *= Vec3::new(f(ml, "x"), f(ml, "y"), f(ml, "z"));
        }
        let ns = if aiming { f(d, "NoiseAimScalar") } else { 1.0 };
        let n = &d["Noise2DTarget"];
        self.noise_t += Vec2::new(rng.random_range(-1.0..=1.0) * f(n, "x"), rng.random_range(-1.0..=1.0) * f(n, "y")) * ns;
        if full_auto && self.shots == 2 {
            self.push_t = f(d, "Pushback");
        }
        if self.auto {
            self.z_t += f(&d["ZProgress"], "Amount") * if aiming { f(d, "ProgressAimAlpha") } else { 1.0 };
        }
        let sw = &d["RecoilSway"];
        let ads = if aiming { f(sw, "AdsScale") } else { 1.0 };
        self.sway_t += Vec2::new(r2(&sw["PitchSway"], rng), r2(&sw["YawSway"], rng)) * ads;
    }

    pub fn tick(&mut self, dt: f32) -> Transform {
        self.since_shot += dt;
        self.t += dt;
        let d = &self.data;
        let bursting = self.since_shot < self.interval * 1.5 + 0.05;
        if !bursting {
            self.shots = 0;
        }
        // Curve layer.
        let (mut rot, mut loc) = (Vec3::ZERO, Vec3::ZERO);
        if let Some([rc, lc]) = &self.curves {
            let c = |k: &Vec<Vec<Key>>, i: usize| k.get(i).map(|k| eval(k, self.t)).unwrap_or(0.0);
            rot = Vec3::new(c(rc, 0), c(rc, 1), c(rc, 2)) * self.rot_t;
            loc = Vec3::new(c(lc, 0), c(lc, 1), c(lc, 2)) * self.loc_t;
        }
        if self.auto {
            let (sr, sl) = (&d["SmoothRot"], &d["SmoothLoc"]);
            let sr = [f(sr, "roll"), f(sr, "pitch"), f(sr, "yaw")];
            let sl = [f(sl, "x"), f(sl, "y"), f(sl, "z")];
            for i in 0..3 {
                self.rot[i] = finterp_to(self.rot[i], rot[i], dt, sr[i]);
                self.loc[i] = finterp_to(self.loc[i], loc[i], dt, sl[i]);
            }
        } else {
            (self.rot, self.loc) = (rot, loc);
        }
        // Noise (right, up), pushback (forward), Z progress (up).
        let (na, nz) = (&d["Noise2DAccel"], &d["Noise2DZero"]);
        self.noise.x = finterp_to(self.noise.x, self.noise_t.x, dt, f(na, "x"));
        self.noise.y = finterp_to(self.noise.y, self.noise_t.y, dt, f(na, "y"));
        self.noise_t.x = finterp_to(self.noise_t.x, 0.0, dt, f(nz, "x"));
        self.noise_t.y = finterp_to(self.noise_t.y, 0.0, dt, f(nz, "y"));
        self.push = finterp_to(self.push, self.push_t, dt, f(d, "PushbackAccel"));
        self.push_t = finterp_to(self.push_t, 0.0, dt, f(d, "PushbackZero"));
        let zp = &d["ZProgress"];
        if !bursting {
            self.z_t = finterp_to(self.z_t, 0.0, dt, f(zp, "Damping"));
        }
        self.z = finterp_to(self.z, self.z_t, dt, f(zp, "Accel"));
        // Recoil sway spring (pitch, yaw) about the shoulder pivot.
        let sw = &d["RecoilSway"];
        self.sway.x = finterp_to(self.sway.x, self.sway_t.x, dt, f(sw, "Acceleration"));
        self.sway.y = finterp_to(self.sway.y, self.sway_t.y, dt, f(sw, "Acceleration"));
        self.sway_t.x = finterp_to(self.sway_t.x, 0.0, dt, f(sw, "Damping"));
        self.sway_t.y = finterp_to(self.sway_t.y, 0.0, dt, f(sw, "Damping"));
        // FireSway: OffsetCurve(shots in burst) cm climb (up) and push (back), clamped; settles when the burst ends.
        let fs = &d["FireSway"];
        let target = if bursting && self.shots > 0 {
            let keys: Vec<Key> = serde_json::from_value(fs["OffsetCurve"]["curves"]["FloatCurve"].clone()).unwrap_or_default();
            let v = eval(&keys, self.shots as f32);
            // ponytail: FireSway while aiming is unknown (native); scaled like RecoilSway.AdsScale until compared with ADS footage.
            let ads = if self.aiming { f(&d["RecoilSway"], "AdsScale") } else { 1.0 };
            Vec2::new(v.min(f(fs, "ClampVerticalOffset")), v.min(f(fs, "ClampDepthOffset"))) * ads
        } else {
            Vec2::ZERO
        };
        let speed = if bursting { f(fs, "InterpSpeed") } else { f(fs, "DampSmoothing") };
        self.fire_sway.x = finterp_to(self.fire_sway.x, target.x, dt, speed);
        self.fire_sway.y = finterp_to(self.fire_sway.y, target.y, dt, speed);

        // UE (X fwd, Y right, Z up) cm -> Bevy (-Z fwd, X right, Y up) m. Negative pitch = muzzle up, kick < 0 = back.
        let kick = self.loc.x + self.push - self.fire_sway.y;
        let right = self.loc.y + self.noise.x;
        let up = self.loc.z + self.noise.y + self.z + self.fire_sway.x;
        let t = Vec3::new(right, up, -kick) * 0.01;
        let r = |deg: f32| deg.to_radians();
        let curve_rot = Quat::from_euler(EulerRot::YXZ, r(-self.rot.z), r(-self.rot.y), r(-self.rot.x));
        let roll_sway = self.sway.y * f(sw, "RollSwayMultiplier");
        let sway_rot = Quat::from_euler(EulerRot::YXZ, r(-self.sway.y), r(-self.sway.x), r(-roll_sway));
        let po = &sw["PivotOffset"];
        let pivot = GRIP + Vec3::new(f(po, "y"), f(po, "z"), -f(po, "x")) * 0.01;
        let about = |p: Vec3, q: Quat| Transform::from_translation(p) * Transform::from_rotation(q) * Transform::from_translation(-p);
        self.offset = Transform::from_translation(t) * about(pivot, sway_rot) * about(GRIP, curve_rot);
        self.offset
    }
}

fn tick(time: Res<Time>, mut rec: ResMut<Recoil>, mut fired: MessageReader<Fired>, inv: Option<Single<&Inventory>>) {
    let mut rng = rand::rng();
    if let Some(inv) = inv {
        let w = &inv.slots[inv.current];
        for e in fired.read() {
            rec.play(&w.anims["RecoilData"], e.aiming, w.stats.fire_rate.unwrap_or(0.1), w.full_auto, &mut rng);
        }
    }
    rec.tick(time.delta_secs());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hip_shot_kicks_and_settles() {
        let w: Value = serde_json::from_str(include_str!("../../assets/weapons/M4A1.json")).unwrap();
        let d = &w["anim_set"]["RecoilData"];
        let mut r = Recoil::default();
        let mut rng = rand::rng();
        r.play(d, false, 0.08, true, &mut rng);
        let mut peak = 0.0f32;
        for _ in 0..10 {
            let o = r.tick(0.01);
            peak = peak.max(o.translation.length());
        }
        assert!(peak > 0.02, "hip shot should move the gun > 2 cm, got {peak}");
        for _ in 0..300 {
            r.tick(0.01);
        }
        let o = r.tick(0.01);
        assert!(o.translation.length() < 0.003 && o.rotation.angle_between(Quat::IDENTITY) < 0.01, "settles: {o:?}");
    }
}
