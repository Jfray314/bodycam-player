//! bodycam-core must behave exactly like the game's own logic: same inputs, same outputs.
use bevy::prelude::*;
use bodycam_core as core;
use rand::RngCore;
use serde_json::Value;

fn weapon_json(row: &str) -> String {
    std::fs::read_to_string(format!("{}/../assets/weapons/{row}.json", env!("CARGO_MANIFEST_DIR"))).unwrap()
}
fn stats_json() -> &'static str {
    include_str!("../../assets/weapon_stats.json")
}

#[test]
fn stats_and_controller_recoil_match_for_every_weapon() {
    let gs = crate::weapon::load_stats();
    let cs = core::weapon_data::load_stats(stats_json()).unwrap();
    assert_eq!(gs.len(), cs.len());
    for e in std::fs::read_dir(concat!(env!("CARGO_MANIFEST_DIR"), "/../assets/weapons")).unwrap().flatten() {
        let txt = std::fs::read_to_string(e.path()).unwrap();
        let v: Value = serde_json::from_str(&txt).unwrap();
        let ge = v["stats_effects"].get(0).and_then(Value::as_str).unwrap_or("");
        let key = ge.rsplit("GE_WeaponStat_").next().unwrap_or("").trim_end_matches("_C").trim_start_matches('_');
        let Some(g) = gs.get(key) else { continue };
        let w = core::weapon_data::WeaponData::from_json(&txt, &cs).unwrap();
        let c = &w.stats;
        assert_eq!((g.damage, g.fire_rate, g.instability, g.pellet_count), (c.damage, c.fire_rate, c.instability, c.pellet_count), "{key}");
        assert_eq!((g.spread_hip_x, g.spread_hip_y, g.spread_aim_x, g.spread_aim_y), (c.spread_hip_x, c.spread_hip_y, c.spread_aim_x, c.spread_aim_y), "{key}");
        assert_eq!((&g.fire_modes, &g.tags, g.ammo_capacity, g.mag_amount), (&c.fire_modes, &c.tags, c.ammo_capacity, c.mag_amount), "{key}");
        let (a, b) = (crate::weapon::ControllerRecoil::from(&v["anim_set"]), core::weapon_data::controller_recoil(&w.anims));
        assert_eq!((a.horizontal, a.vertical, a.smoothing, a.damping, a.compensation), (b.horizontal, b.vertical, b.smoothing, b.damping, b.compensation), "{key}");
    }
}

#[test]
fn ballistics_match() {
    for s in 0..=255u8 {
        assert_eq!(crate::weapon::surface_absorbance(s), core::ballistics::surface_absorbance(s));
    }
    for b in ["head", "Neck", "spine_01", "spine_02", "spine_03", "hand_r", ""] {
        assert_eq!(crate::weapon::bone_multiplier(b), core::ballistics::bone_multiplier(b));
    }
    let cases: [&[(u32, f32)]; 4] = [&[(1, 0.6), (1, 0.6), (2, 0.6), (3, 0.6), (4, 0.6)], &[(1, 1.0), (2, 0.0)], &[], &[(1, 0.3), (2, 0.3), (3, 1.0), (4, 1.0)]];
    for h in cases {
        assert_eq!(crate::weapon::penetrate(h), core::ballistics::penetrate(h));
    }
}

/// Feeds both RNG APIs from the same 24-bit-grid sequence.
struct Seq(Vec<u32>, usize);
impl Seq {
    fn new(n: usize, seed: u32) -> Self {
        let mut x = seed.max(1);
        Seq((0..n).map(|_| { x ^= x << 13; x ^= x >> 17; x ^= x << 5; x >> 8 }).collect(), 0)
    }
    fn k(&mut self) -> u32 {
        let k = self.0[self.1 % self.0.len()];
        self.1 += 1;
        k
    }
}
impl RngCore for Seq {
    fn next_u32(&mut self) -> u32 {
        self.k() << 8
    }
    fn next_u64(&mut self) -> u64 {
        ((self.next_u32() as u64) << 32) | self.next_u32() as u64
    }
    fn fill_bytes(&mut self, d: &mut [u8]) {
        d.iter_mut().for_each(|b| *b = self.next_u32() as u8);
    }
}
impl core::gunfire::Uniform for Seq {
    fn next01(&mut self) -> f32 {
        self.k() as f32 / 16777216.0
    }
}

#[test]
fn spread_cone_matches() {
    let mut a = Seq::new(2000, 7);
    let mut b = Seq::new(2000, 7);
    for (dir, h, v) in [(Vec3::NEG_Z, 6.0, 2.0), (Vec3::new(0.3, 0.2, -0.9).normalize(), 5.0, 6.0), (Vec3::X, 1.0, 0.0), (Vec3::Y, 3.0, 3.0), (Vec3::NEG_Z, 0.0, 0.0)] {
        for _ in 0..100 {
            let g = crate::weapon::random_in_elliptical_cone(dir, h, v, &mut a);
            let c = core::gunfire::random_in_elliptical_cone(dir.to_array(), h, v, &mut b);
            assert!((g - Vec3::from_array(c)).length() < 2e-4, "{g} vs {c:?}");
        }
    }
}

#[test]
fn sway_chain_matches() {
    let txt = weapon_json("M4A1");
    let cs = core::weapon_data::load_stats(stats_json()).unwrap();
    let mut wd = core::weapon_data::WeaponData::from_json(&txt, &cs).unwrap();
    // LowReady is added outside `run` in the game; compare the springs alone.
    wd.anims["LowerSway"] = Value::Null;
    let anims = wd.anims.clone();
    let speed = anims["AdsSettings"]["AdsSpeed"].as_f64().unwrap() as f32;
    let (mut gs, mut cs_) = (crate::sway::Sway::default(), core::sway::Sway::default());
    let (mut raw, dt) = (0.0f32, 1.0f32 / 60.0);
    let mut prev = [0.0f32; 2];
    for n in 0..480 {
        let aiming = (120..300).contains(&n);
        let look = [if n < 400 { (n as f32 * 0.37).sin() * 3.0 } else { 0.0 }, (n as f32 * 0.11).cos() * 1.5];
        let mv = [if (60..200).contains(&n) { 1.0 } else { 0.0 }, if n > 250 { -1.0 } else { 0.0 }];
        // core sees degrees/frame and applies the one-frame lag itself; the game's input is already lagged node units
        let conv = |d: f32| d / (dt * 60.0) * core::sway::LOOK_IN;
        let ldi = Vec2::new(conv(prev[0]), conv(prev[1]));
        prev = look;
        raw = crate::sway::ads_advance(raw, aiming, speed, dt);
        gs.alpha = crate::sway::ads_ease(raw);
        crate::sway::run(&mut gs, &anims, ldi, Vec2::new(mv[0], mv[1]), aiming, dt);
        let o = cs_.step(&wd, &core::sway::SwayIn { dt, look_yaw_deg: look[0], look_pitch_deg: look[1], vel_fwd: mv[0] * 0.5, vel_right: mv[1] * 0.5, aiming, ..Default::default() });
        assert!((gs.alpha - o.alpha).abs() < 1e-6);
        assert!((gs.offset.translation - Vec3::from_array(o.translation)).length() < 1e-5, "frame {n}: {} vs {:?}", gs.offset.translation, o.translation);
        let m = Mat3::from_quat(gs.offset.rotation);
        let cm = core::sway::rot_matrix(o.rot_deg[0], o.rot_deg[1], o.rot_deg[2]);
        for r in 0..3 {
            for c in 0..3 {
                assert!((m.col(c)[r] - cm[r][c]).abs() < 1e-5, "frame {n} rot [{r}][{c}]");
            }
        }
    }
}

#[test]
fn weapon_recoil_animation_matches() {
    let txt = weapon_json("M4A1");
    let v: Value = serde_json::from_str(&txt).unwrap();
    let mut d = v["anim_set"]["RecoilData"].clone();
    // collapse the random ranges so both implementations are deterministic
    for k in ["PitchHip", "PitchAim", "KickHip", "KickHipUp", "KickAim", "KickAimUp"] {
        if let Some(o) = d.get_mut(k) {
            o["y"] = o["x"].clone();
        }
    }
    for k in ["RollHip", "RollAim", "YawHip", "YawAim"] {
        if let Some(o) = d.get_mut(k) {
            for z in ["y", "z", "w"] {
                o[z] = o["x"].clone();
            }
        }
    }
    for k in ["PitchSway", "YawSway"] {
        if let Some(o) = d["RecoilSway"].get_mut(k) {
            o["y"] = o["x"].clone();
        }
    }
    d["Noise2DTarget"] = serde_json::json!({"x": 0.0, "y": 0.0});
    let (mut g, mut c) = (crate::recoil::Recoil::default(), core::recoil::Recoil::default());
    let (mut r1, mut r2) = (Seq::new(64, 3), Seq::new(64, 3));
    let dt = 1.0f32 / 120.0;
    for n in 0..400 {
        if n % 8 == 0 && n < 160 {
            let aiming = n >= 80;
            g.play(&d, aiming, 0.07, true, &mut r1);
            c.play(&d, aiming, 0.07, true, &mut r2);
        }
        let t = g.tick(dt);
        c.tick(dt);
        assert!((t.translation - Vec3::from_array(c.t)).length() < 1e-5, "frame {n}: {} vs {:?}", t.translation, c.t);
        assert!((g.out_rot - Vec3::from_array(c.out_rot)).length() < 1e-4 && (g.out_loc - Vec3::from_array(c.out_loc)).length() < 1e-4, "frame {n}");
        let m = Mat3::from_quat(t.rotation);
        for r in 0..3 {
            for k in 0..3 {
                assert!((m.col(k)[r] - c.m[r][k]).abs() < 1e-5, "frame {n} [{r}][{k}]");
            }
        }
    }
}

#[test]
fn lens_defaults_match() {
    let (g, c) = (crate::postfx::BodycamPostFx::default(), core::lens::LensParams::game());
    assert_eq!(
        [g.distortion, g.distortion_density, g.ca, g.vignette, g.aberration_fade_exponent, g.edge_blur, g.blur_fade_exponent, g.sharpen, g.grain, g.temporal_noise, g.banding, g.dirt, g.dirt_threshold, g.dirt_gain, g.exposure_bias, g.rolling_shutter],
        [c.distortion, c.distortion_density, c.ca, c.vignette, c.aberration_fade_exponent, c.edge_blur, c.blur_fade_exponent, c.sharpen, c.grain, c.temporal_noise, c.banding, c.dirt, c.dirt_threshold, c.dirt_gain, c.exposure_bias, c.rolling_shutter]
    );
}
