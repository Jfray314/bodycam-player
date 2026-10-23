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
    /// PRAS output as the probe records it (UE): OutRot (pitch, yaw, roll) deg, OutLoc (X fwd, Y right, Z up) cm.
    pub out_rot: Vec3,
    pub out_loc: Vec3,
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
    /// Active shots: (age s, rot curves, loc curves, rot target, loc target); their curve responses add up.
    layers: Vec<(f32, [Vec<Vec<Key>>; 2], Vec3, Vec3)>,
    data: Value,
}

fn r2(v: &Value, rng: &mut impl Rng) -> f32 {
    let (a, b) = (v["x"].as_f64().unwrap_or(0.0) as f32, v["y"].as_f64().unwrap_or(0.0) as f32);
    if a == b { a } else { rng.random_range(a.min(b)..=a.max(b)) }
}

/// 4D range (x,y | z,w): one of the two 2D ranges at random - so the value is never near zero.
/// ponytail: inferred from amplitude fit (a uniform pick between the two draws halves the measured yaw/roll amplitude).
fn r4(v: &Value, rng: &mut impl Rng) -> f32 {
    let f = |k: &str| v[k].as_f64().unwrap_or(0.0) as f32;
    let (a, b) = if rng.random_bool(0.5) { (f("x"), f("y")) } else { (f("z"), f("w")) };
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
        if false {
            // ponytail: MultiRot/MultiLoc made auto pitch 2.2x too big vs the probe (RMS 2.9 vs 1.0 deg), so they are not applied.
            let (mr, ml) = (&d["MultiRot"], &d["MultiLoc"]);
            self.rot_t *= Vec3::new(f(mr, "roll"), f(mr, "pitch"), f(mr, "yaw"));
            self.loc_t *= Vec3::new(f(ml, "x"), f(ml, "y"), f(ml, "z"));
        }
        if let Some(c) = &self.curves {
            self.layers.clear(); // a new shot replaces the previous curve (measured; summing them overshoots)
            self.layers.push((0.0, c.clone(), self.rot_t, self.loc_t));
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
        self.layers.retain(|l| l.0 < 2.0);
        for (age, [rc, lc], rt, lt) in &mut self.layers {
            *age += dt;
            let c = |k: &Vec<Vec<Key>>, i: usize| k.get(i).map(|k| eval(k, *age)).unwrap_or(0.0);
            rot += Vec3::new(c(rc, 0), c(rc, 1), c(rc, 2)) * *rt;
            loc += Vec3::new(c(lc, 0), c(lc, 1), c(lc, 2)) * *lt;
        }
        if false {
            // ponytail: SmoothRot/SmoothLoc low-pass flattened the per-shot pitch swings that the probe shows unsmoothed; not applied.
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
            // Measured (probe, M4A1 aim bursts): no climb and no forward drift while aiming, so FireSway is hip only.
            // Pistols have per-shot steps (Vertical/DepthOffsetStep); rifles use OffsetCurve(shots).
            // ponytail: the exact native mapping (curve x axis, step vs curve) is fitted, see recoil_fit.
            // ponytail: shot count capped at 6, fitted (Glock Z saturates ~14 cm after ~6 shots).
            let n = (self.shots as f32).min(6.0);
            let (vs, ds) = (f(fs, "VerticalOffsetStep"), f(fs, "DepthOffsetStep"));
            let (vs, ds) = if vs > 0.0 { (vs, ds) } else { (4.0, 4.0) }; // ponytail: rifle asset omits the steps (native default unknown); 4 fits M4A1 hip Z/X
            let (cv, cd) = (f(fs, "ClampVerticalOffset"), f(fs, "ClampDepthOffset"));
            if self.aiming {
                Vec2::ZERO
            } else if vs > 0.0 {
                Vec2::new((vs * n).min(cv), (ds * n).min(cd))
            } else {
                let keys: Vec<Key> = serde_json::from_value(fs["OffsetCurve"]["curves"]["FloatCurve"].clone()).unwrap_or_default();
                let v = eval(&keys, n);
                Vec2::new(v.min(cv), v.min(cd))
            }
        } else {
            Vec2::ZERO
        };
        let speed = if bursting { f(fs, "InterpSpeed") } else { f(fs, "DampSmoothing") };
        self.fire_sway.x = finterp_to(self.fire_sway.x, target.x, dt, speed);
        self.fire_sway.y = finterp_to(self.fire_sway.y, target.y, dt, speed);

        // UE (X fwd, Y right, Z up) cm -> Bevy (-Z fwd, X right, Y up) m. Negative pitch = muzzle up, kick < 0 = back.
        let mut kick = self.loc.x + self.push + self.fire_sway.y; // depth sway drifts forward (measured)
        // RecoilData.MaxKickHip caps how far back the gun may travel, hip and aim alike: every probe burst (M4A1 hip/aim, BC5, AK aim)
        // bottoms out at OutLoc.X = -2.8..-3.0 = -MaxKickHip, though the AK's KickAim (-6..-7) + Pushback (-4) would reach -11.
        // Without the cap the optic is thrown into the camera on sustained ADS fire.
        if let Some(max) = d["MaxKickHip"].as_f64() {
            kick = kick.max(-max as f32);
        }
        let right = self.loc.y + self.noise.x; // PRAS OutLoc.Y; the camera-space gun moves -0.96x of it (probe sockets, rec_1791602920), so Bevy x = -right below
        let up = self.loc.z + self.noise.y + self.z + self.fire_sway.x;
        self.out_rot = Vec3::new(self.rot.y, self.rot.z, self.rot.x);
        self.out_loc = Vec3::new(kick, right, up);
        let t = Vec3::new(-right, up, -kick) * 0.01;
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
            // attachment VerticalRecoil / HorizontalRecoil / KickHip / KickUp multipliers
            let m = &w.mods;
            (rec.rot_t.y, rec.rot_t.z, rec.loc_t.x, rec.loc_t.z) = (rec.rot_t.y * m.pitch, rec.rot_t.z * m.yaw, rec.loc_t.x * m.kick, rec.loc_t.z * m.kick_up);
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

    use rand::SeedableRng;

    const CH: [&str; 6] = ["OutRot.Pitch", "OutRot.Yaw", "OutRot.Roll", "OutLoc.X", "OutLoc.Y", "OutLoc.Z"];

    /// Frames the real shot precedes the frame where the probe first sees it (onset detection threshold lag).
    fn lead() -> usize {
        1
    }

    fn series(v: &Value) -> Vec<f32> {
        v.as_array().unwrap().iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()
    }

    /// Replays a recorded burst class (shot frames on the probe's 12.5 ms grid) through `Recoil` with `seeds` random
    /// seeds; returns the mean OutRot/OutLoc trajectories (same layout as the reference) per channel.
    fn sim_burst(gun: &str, aiming: bool, b: &Value, seeds: u64) -> (Vec<Vec<f32>>, [f32; 6]) {
        let w: Value = serde_json::from_str(&std::fs::read_to_string(format!("../assets/weapons/{gun}.json")).unwrap()).unwrap();
        let d = &w["anim_set"]["RecoilData"];
        let stats: Value = serde_json::from_str(include_str!("../../assets/weapon_stats.json")).unwrap();
        let interval = stats[gun]["FireRate"].as_f64().unwrap_or(0.1) as f32;
        let full_auto = stats[gun]["fire_modes"].as_array().is_some_and(|m| m.iter().any(|x| x == "FullAuto"));
        let dt = 0.0125f32;
        let pre = 2usize;
        let len = b["len"].as_u64().unwrap() as usize;
        let frames: Vec<usize> = series(&b["times"]).iter().map(|t| (pre + (t / dt).round() as usize).saturating_sub(lead())).collect();
        let mut mean = vec![vec![0.0f32; pre + len]; 6];
        let mut amp = [0.0f32; 6];
        for seed in 0..seeds {
            let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
            let mut r = Recoil::default();
            r.roll_sign = if seed % 2 == 0 { 1.0 } else { -1.0 }; // real sign state is carried over from earlier shots
            for j in 0..pre + len {
                for _ in frames.iter().filter(|&&f| f == j) {
                    r.play(d, aiming, interval, full_auto, &mut rng);
                }
                r.tick(dt);
                for (c, v) in [r.out_rot.x, r.out_rot.y, r.out_rot.z, r.out_loc.x, r.out_loc.y, r.out_loc.z].into_iter().enumerate() {
                    mean[c][j] += v / seeds as f32;
                    amp[c] += v * v / (seeds as usize * (pre + len)) as f32; // mean over seeds of per-seed mean square
                }
            }
        }
        (mean, amp.map(f32::sqrt))
    }

    /// RMS error of the simulated mean vs the recorded mean, per channel, over every recorded burst class.
    /// `FIT=<substring>` filters classes, `DUMP=<channel idx>` prints the curves.
    /// Run: cargo test -p game recoil_fit -- --nocapture
    #[test]
    fn recoil_fit() {
        let reference: Value = serde_json::from_str(include_str!("../../assets/recoil_reference.json")).unwrap();
        let (mut sq, mut sq0, mut n) = ([0.0f32; 6], [0.0f32; 6], 0usize);
        let only = std::env::var("FIT").unwrap_or_default();
        let dump: Option<usize> = std::env::var("DUMP").ok().and_then(|v| v.parse().ok());
        for (key, b) in reference["bursts"].as_object().unwrap() {
            if !only.is_empty() && !key.contains(&only) {
                continue;
            }
            let mut p = key.split('/');
            if key.starts_with("Rifle") {
                continue; // unidentified gun, see tools/probe_recoil.py
            }
            let (gun, aiming) = (p.next().unwrap(), p.next().unwrap() == "aim");
            let (sim, amp) = sim_burst(gun, aiming, b, 200);
            let mut line = format!("{key:16} n={:<2}", b["n"]);
            for (c, name) in CH.iter().enumerate() {
                let data = series(&b[name]["mean"]);
                let (mut e, mut e0) = (0.0f32, 0.0f32);
                for j in 2..data.len() {
                    e += (sim[c][j] - data[j]).powi(2);
                    e0 += data[j].powi(2);
                    if dump == Some(c) {
                        println!("  j={j} data={:.2} sim={:.2}", data[j], sim[c][j]);
                    }
                }
                let k = (data.len() - 2) as f32;
                line += &format!("  {}:{:.2}/{:.2}[amp {:.1}]", &name[3..], (e / k).sqrt(), (e0 / k).sqrt(), amp[c]);
                sq[c] += e;
                sq0[c] += e0;
                n += data.len() - 2;
            }
            println!("{line}");
        }
        let n = (n / 6).max(1) as f32;
        let err = sq.map(|e| (e / n).sqrt());
        println!("TOTAL rms error per channel {:?}; signal rms {:?}", err, sq0.map(|e| (e / n).sqrt()));
        if only.is_empty() {
            // pitch deg, X/Z cm (yaw/roll/Y are random walks: only their amplitude is comparable)
            assert!(err[0] < 2.2 && err[3] < 1.3 && err[5] < 4.0, "recoil fit regressed: {err:?}");
        }
    }

    /// AK ADS full-auto: the camera-space gun offset must stay well clear of the camera (optic ~10 cm in front).
    #[test]
    fn ak_ads_burst_stays_off_camera() {
        let w: Value = serde_json::from_str(include_str!("../../assets/weapons/AK47.json")).unwrap();
        let d = &w["anim_set"]["RecoilData"];
        let mut r = Recoil::default();
        let mut rng = rand::rng();
        let (mut mn, mut mx, mut ang) = (Vec3::splat(9.0), Vec3::splat(-9.0), 0.0f32);
        for i in 0..300 {
            if i % 10 == 0 && i < 250 {
                r.play(d, true, 0.1, true, &mut rng);
            }
            let o = r.tick(0.01);
            (mn, mx) = (mn.min(o.translation), mx.max(o.translation));
            ang = ang.max(o.rotation.angle_between(Quat::IDENTITY).to_degrees());
        }
        // -MaxKickHip (3 cm) plus a little pivot-rotation lever arm; uncapped it reached ~11 cm.
        assert!(mx.z < 0.05, "gun pulled > 5 cm back: {mx:?} (min {mn:?}, max angle {ang})");
    }
}
