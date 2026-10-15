//! C ABI for the slices beyond the camera (see include/bodycam_core.h). Handles are opaque heap pointers owned by the host
//! (`*_create`/`*_load` ... `*_destroy`/`*_free`). Strings in are NUL-terminated UTF-8; strings out are copied into a host buffer
//! and the full length (without NUL) is returned, so a too-small buffer can be detected and retried.
use crate::ballistics;
use crate::cues::CueBank;
use crate::gunfire::{FireControl, GunStats, Rng, TriggerIn, shot_dirs};
use crate::lens::{LensMotion, LensParams, vertical_fov_deg};
use crate::recoil::Recoil;
use crate::sway::{Sway, SwayIn};
use crate::weapon_data::{Stats, WeaponData, controller_recoil, load_stats};
use std::ffi::{CStr, c_char};

unsafe fn s<'a>(p: *const c_char) -> Option<&'a str> {
    if p.is_null() { None } else { unsafe { CStr::from_ptr(p) }.to_str().ok() }
}
unsafe fn put(out: &str, buf: *mut c_char, cap: usize) -> i32 {
    if !buf.is_null() && cap > 0 {
        let n = out.len().min(cap - 1);
        unsafe {
            std::ptr::copy_nonoverlapping(out.as_ptr(), buf as *mut u8, n);
            *buf.add(n) = 0;
        }
    }
    out.len() as i32
}
fn boxed<T>(v: T) -> *mut T {
    Box::into_raw(Box::new(v))
}
unsafe fn free<T>(p: *mut T) {
    if !p.is_null() {
        drop(unsafe { Box::from_raw(p) });
    }
}

// ---- weapon data ----
pub struct BcStats(std::collections::HashMap<String, Stats>);

/// NULL on parse error. `weapon_stats.json`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_stats_load(json: *const c_char) -> *mut BcStats {
    unsafe { s(json) }.and_then(|j| load_stats(j).ok()).map_or(std::ptr::null_mut(), |m| boxed(BcStats(m)))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_stats_free(p: *mut BcStats) {
    unsafe { free(p) }
}
/// `assets/weapons/<Row>.json` + the stats table. NULL on error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_weapon_load(stats: *const BcStats, weapon_json: *const c_char) -> *mut WeaponData {
    let (Some(st), Some(j)) = (unsafe { stats.as_ref() }, unsafe { s(weapon_json) }) else { return std::ptr::null_mut() };
    WeaponData::from_json(j, &st.0).map_or(std::ptr::null_mut(), boxed)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_weapon_free(w: *mut WeaponData) {
    unsafe { free(w) }
}
#[repr(C)]
pub struct BcWeaponInfo {
    pub damage: f32,
    pub fire_rate: f32,
    pub pellets: u32,
    pub ammo_capacity: u32,
    pub mag_amount: u32,
    pub full_auto: u32,
    pub infinite_mags: u32,
    pub spread_hip: [f32; 2],
    pub spread_aim: [f32; 2],
    pub instability: f32,
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_weapon_info(w: *const WeaponData, out: *mut BcWeaponInfo) {
    let (Some(w), false) = (unsafe { w.as_ref() }, out.is_null()) else { return };
    let g = GunStats::from_stats(&w.stats);
    unsafe {
        *out = BcWeaponInfo {
            damage: g.damage, fire_rate: g.fire_rate, pellets: g.pellets, ammo_capacity: w.stats.ammo_capacity.unwrap_or(30.0) as u32,
            mag_amount: w.stats.mag_amount.unwrap_or(4.0) as u32, full_auto: g.full_auto as u32, infinite_mags: w.infinite_mags() as u32,
            spread_hip: [g.spread_hip.0, g.spread_hip.1], spread_aim: [g.spread_aim.0, g.spread_aim.1], instability: g.instability,
        }
    };
}
/// Controller recoil parameters for `bc_fire`: h_min,h_max,v_min,v_max,smoothing,damping,compensation.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_weapon_controller_recoil(w: *const WeaponData, out7: *mut f32) {
    let (Some(w), false) = (unsafe { w.as_ref() }, out7.is_null()) else { return };
    let c = controller_recoil(&w.anims);
    let v = [c.horizontal.0, c.horizontal.1, c.vertical.0, c.vertical.1, c.smoothing, c.damping, c.compensation];
    unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), out7, 7) };
}
/// Weapon cue path from the weapon JSON ("Fire", "FireSuppressed", "FireDry", "BulletCrack", "BulletWizz").
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_weapon_sound(w: *const WeaponData, kind: *const c_char, buf: *mut c_char, cap: usize) -> i32 {
    let (Some(w), Some(k)) = (unsafe { w.as_ref() }, unsafe { s(kind) }) else { return -1 };
    w.sound(k).map_or(-1, |p| unsafe { put(p, buf, cap) })
}

// ---- sway / ADS ----
#[repr(C)]
pub struct BcSwayIn {
    pub dt: f32,
    pub look_yaw_deg: f32,
    pub look_pitch_deg: f32,
    pub vel_fwd: f32,
    pub vel_right: f32,
    pub cam_pitch_rad: f32,
    pub aiming: u32,
    /// 1 = use `pivot_m` instead of the measured hip/ADS pivot.
    pub has_pivot: u32,
    pub pivot_m: [f32; 3],
}
#[repr(C)]
pub struct BcSwayOut {
    /// Apply to the arms root: p' = R(rot_deg) * p + translation.
    pub translation: [f32; 3],
    /// pitch, yaw, roll degrees.
    pub rot_deg: [f32; 3],
    /// Row-major rotation matrix of rot_deg.
    pub matrix: [f32; 9],
    pub ads_alpha: f32,
}
#[unsafe(no_mangle)]
pub extern "C" fn bc_sway_create() -> *mut Sway {
    boxed(Sway::default())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_sway_destroy(p: *mut Sway) {
    unsafe { free(p) }
}
/// Call again with a fresh `bc_sway_create` when the weapon changes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_sway_step(sw: *mut Sway, w: *const WeaponData, i: *const BcSwayIn, o: *mut BcSwayOut) {
    let (Some(sw), Some(w), Some(i), false) = (unsafe { sw.as_mut() }, unsafe { w.as_ref() }, unsafe { i.as_ref() }, o.is_null()) else { return };
    let r = sw.step(w, &SwayIn { dt: i.dt, look_yaw_deg: i.look_yaw_deg, look_pitch_deg: i.look_pitch_deg, vel_fwd: i.vel_fwd, vel_right: i.vel_right, aiming: i.aiming != 0, cam_pitch_rad: i.cam_pitch_rad, pivot_m: (i.has_pivot != 0).then_some(i.pivot_m) });
    let m = crate::sway::rot_matrix(r.rot_deg[0], r.rot_deg[1], r.rot_deg[2]);
    unsafe { *o = BcSwayOut { translation: r.translation, rot_deg: r.rot_deg, matrix: std::array::from_fn(|k| m[k / 3][k % 3]), ads_alpha: r.alpha } };
}

// ---- weapon recoil animation ----
pub struct BcRecoil(Recoil, Rng);
#[repr(C)]
pub struct BcRecoilOut {
    /// p' = matrix * p + translation (camera space metres, x right, y up, z back).
    pub matrix: [f32; 9],
    pub translation: [f32; 3],
    pub out_rot: [f32; 3],
    pub out_loc: [f32; 3],
}
#[unsafe(no_mangle)]
pub extern "C" fn bc_recoil_create(seed: u64) -> *mut BcRecoil {
    boxed(BcRecoil(Recoil::default(), Rng::new(seed)))
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_recoil_destroy(p: *mut BcRecoil) {
    unsafe { free(p) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_recoil_play(r: *mut BcRecoil, w: *const WeaponData, aiming: u32) {
    let (Some(r), Some(w)) = (unsafe { r.as_mut() }, unsafe { w.as_ref() }) else { return };
    r.0.play(&w.anims["RecoilData"], aiming != 0, w.stats.fire_rate.unwrap_or(0.1), w.full_auto(), &mut r.1);
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_recoil_tick(r: *mut BcRecoil, dt: f32, o: *mut BcRecoilOut) {
    let (Some(r), false) = (unsafe { r.as_mut() }, o.is_null()) else { return };
    r.0.tick(dt);
    unsafe { *o = BcRecoilOut { matrix: std::array::from_fn(|k| r.0.m[k / 3][k % 3]), translation: r.0.t, out_rot: r.0.out_rot, out_loc: r.0.out_loc } };
}

// ---- fire control / spread ----
pub struct BcGun {
    stats: GunStats,
    fc: FireControl,
    rng: Rng,
}
pub const BC_TRIG_HELD: u32 = 1;
pub const BC_TRIG_PRESSED: u32 = 2;
pub const BC_TRIG_BLOCKED: u32 = 4;
pub const BC_TRIG_AIMING: u32 = 8;
#[repr(C)]
pub struct BcFireOut {
    pub fired: u32,
    pub dry: u32,
    /// < 0: none; else seconds until the regrip animation.
    pub regrip_delay: f32,
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_gun_create(w: *const WeaponData, ammo: u32, seed: u64) -> *mut BcGun {
    let Some(w) = (unsafe { w.as_ref() }) else { return std::ptr::null_mut() };
    boxed(BcGun { stats: GunStats::from_stats(&w.stats), fc: FireControl::new(ammo), rng: Rng::new(seed) })
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_gun_destroy(g: *mut BcGun) {
    unsafe { free(g) }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_gun_set_ammo(g: *mut BcGun, ammo: u32) {
    if let Some(g) = unsafe { g.as_mut() } {
        g.fc.ammo = ammo;
    }
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_gun_ammo(g: *const BcGun) -> u32 {
    unsafe { g.as_ref() }.map_or(0, |g| g.fc.ammo)
}
/// One frame of trigger logic. `now` is any monotonic clock in seconds, `flags` BC_TRIG_*.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_gun_step(g: *mut BcGun, dt: f32, now: f32, flags: u32, o: *mut BcFireOut) {
    let (Some(g), false) = (unsafe { g.as_mut() }, o.is_null()) else { return };
    let i = TriggerIn { dt, now, held: flags & BC_TRIG_HELD != 0, just_pressed: flags & BC_TRIG_PRESSED != 0, blocked: flags & BC_TRIG_BLOCKED != 0, aiming: flags & BC_TRIG_AIMING != 0 };
    let r = g.fc.step(&g.stats, &i, &mut g.rng);
    unsafe { *o = BcFireOut { fired: r.fired as u32, dry: r.dry as u32, regrip_delay: r.regrip_delay.unwrap_or(-1.0) } };
}
/// After a shot: one spread direction per pellet around the unit `aim_dir`, written to `out` (3 floats each). Returns the pellet count.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_gun_dirs(g: *mut BcGun, aim_dir: *const f32, aiming: u32, out: *mut f32, max_pellets: u32) -> u32 {
    let (Some(g), false, false) = (unsafe { g.as_mut() }, aim_dir.is_null(), out.is_null()) else { return 0 };
    let a = unsafe { [*aim_dir, *aim_dir.add(1), *aim_dir.add(2)] };
    let dirs = shot_dirs(&g.stats, a, aiming != 0, &mut g.rng);
    let n = dirs.len().min(max_pellets as usize);
    for (k, d) in dirs.iter().take(n).enumerate() {
        unsafe { std::ptr::copy_nonoverlapping(d.as_ptr(), out.add(k * 3), 3) };
    }
    dirs.len() as u32
}

// ---- ballistics ----
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_bone_multiplier(bone: *const c_char) -> f32 {
    unsafe { s(bone) }.map_or(1.0, ballistics::bone_multiplier)
}
#[unsafe(no_mangle)]
pub extern "C" fn bc_surface_absorbance(surface: u8) -> f32 {
    ballistics::surface_absorbance(surface)
}
/// `n` hits in trace order (actor ids + absorbances). Writes up to `cap` (hit index, multiplier at the hit) pairs; returns the count.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_penetrate(actors: *const u64, absorb: *const f32, n: u32, out_idx: *mut u32, out_mult: *mut f32, cap: u32) -> u32 {
    if actors.is_null() || absorb.is_null() {
        return 0;
    }
    let hits: Vec<(u64, f32)> = (0..n as usize).map(|k| unsafe { (*actors.add(k), *absorb.add(k)) }).collect();
    let r = ballistics::penetrate(&hits);
    for (k, (i, m)) in r.iter().take(cap as usize).enumerate() {
        unsafe {
            if !out_idx.is_null() {
                *out_idx.add(k) = *i as u32;
            }
            if !out_mult.is_null() {
                *out_mult.add(k) = *m;
            }
        }
    }
    r.len().min(cap as usize) as u32
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_hit_damage(base: f32, mult: f32, bone: *const c_char) -> f32 {
    ballistics::hit_damage(base, mult, unsafe { s(bone) }.unwrap_or(""))
}

// ---- audio cues ----
/// `audio.json` (only its `footsteps` / `weapons` tables are read). NULL on parse error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_cues_load(json: *const c_char) -> *mut CueBank {
    unsafe { s(json) }.and_then(|j| CueBank::from_json(j).ok()).map_or(std::ptr::null_mut(), boxed)
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_cues_free(c: *mut CueBank) {
    unsafe { free(c) }
}
/// `kind` = BcOutput::footstep (1 crouch, 2 walk, 3 run, 4 land); `surface` = EPhysicalSurface index or -1. Cue names are
/// written separated by '\n'. Returns the full length, -1 on bad arguments.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_cues_footstep(c: *const CueBank, kind: i32, stride_start: u32, surface: i32, buf: *mut c_char, cap: usize) -> i32 {
    let Some(c) = (unsafe { c.as_ref() }) else { return -1 };
    let cues = c.footstep_cues(kind, stride_start != 0, u8::try_from(surface).ok());
    unsafe { put(&cues.join("\n"), buf, cap) }
}
/// Returns the length, or -1 if there is no such cue.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_cues_weapon(c: *const CueBank, weapon: *const c_char, kind: *const c_char, buf: *mut c_char, cap: usize) -> i32 {
    let (Some(c), Some(w), Some(k)) = (unsafe { c.as_ref() }, unsafe { s(weapon) }, unsafe { s(kind) }) else { return -1 };
    c.weapon_cue(w, k).map_or(-1, |p| unsafe { put(p, buf, cap) })
}

// ---- lens ----
/// `preset`: 0 = game, 1 = ReShade file defaults.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_lens_defaults(preset: u32, out: *mut LensParams) {
    if !out.is_null() {
        unsafe { *out = if preset == 1 { LensParams::reshade() } else { LensParams::game() } };
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn bc_lens_motion_create() -> *mut LensMotion {
    boxed(LensMotion::default())
}
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_lens_motion_destroy(m: *mut LensMotion) {
    unsafe { free(m) }
}
/// `w` = camera angular velocity, camera-local (pitch, yaw, roll) rad/s. Outputs the shader's yaw_rate, pitch_rate, pump.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bc_lens_motion_step(m: *mut LensMotion, dt: f32, w: *const f32, out3: *mut f32) {
    let (Some(m), false) = (unsafe { m.as_mut() }, w.is_null()) else { return };
    m.step(dt, unsafe { [*w, *w.add(1), *w.add(2)] });
    if !out3.is_null() {
        unsafe {
            *out3 = m.yaw_rate;
            *out3.add(1) = m.pitch_rate;
            *out3.add(2) = m.pump;
        }
    }
}
#[unsafe(no_mangle)]
pub extern "C" fn bc_vertical_fov_deg(h_fov_deg: f32, aspect: f32) -> f32 {
    vertical_fov_deg(h_fov_deg, aspect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn header_declares_every_export() {
        let h = include_str!("../include/bodycam_core.h");
        for src in [include_str!("ffi.rs"), include_str!("lib.rs")] {
            for l in src.lines().filter(|l| l.contains("extern \"C\" fn bc_")) {
                let name = l.split("fn ").nth(1).unwrap().split('(').next().unwrap();
                assert!(h.contains(&format!("{name}(")), "{name} missing from bodycam_core.h");
            }
        }
    }

    #[test]
    fn c_abi_round_trip() {
        let stats = CString::new(r#"{"Default":{"Damage":30.0,"FireRate":0.5,"fire_modes":["SemiAuto"]},"T":{"Damage":10.0,"PelletCount":3.0}}"#).unwrap();
        let weapon = CString::new(r#"{"row":"T","stats_effects":["/x/GE_WeaponStat_T.GE_WeaponStat_T_C"],"sounds":{"Cosmetic.Weapon.SFX.Fire":"cue/t"},"anim_set":{}}"#).unwrap();
        unsafe {
            let st = bc_stats_load(stats.as_ptr());
            let w = bc_weapon_load(st, weapon.as_ptr());
            assert!(!w.is_null());
            let mut info = std::mem::zeroed::<BcWeaponInfo>();
            bc_weapon_info(w, &mut info);
            assert_eq!((info.damage, info.fire_rate, info.pellets, info.full_auto), (10.0, 0.5, 3, 0));
            let mut buf = [0 as c_char; 16];
            let k = CString::new("Fire").unwrap();
            assert_eq!(bc_weapon_sound(w, k.as_ptr(), buf.as_mut_ptr(), 16), 5);
            let g = bc_gun_create(w, 2, 1);
            let mut o = std::mem::zeroed::<BcFireOut>();
            bc_gun_step(g, 0.016, 0.0, BC_TRIG_HELD | BC_TRIG_PRESSED, &mut o);
            assert_eq!((o.fired, bc_gun_ammo(g)), (1, 1));
            let mut dirs = [0.0f32; 9];
            assert_eq!(bc_gun_dirs(g, [0.0f32, 0.0, -1.0].as_ptr(), 0, dirs.as_mut_ptr(), 3), 3);
            let (a, ab) = ([1u64, 2, 2, 3], [0.6f32, 0.6, 0.6, 0.0]);
            let (mut idx, mut mult) = ([0u32; 4], [0.0f32; 4]);
            assert_eq!(bc_penetrate(a.as_ptr(), ab.as_ptr(), 4, idx.as_mut_ptr(), mult.as_mut_ptr(), 4), 3);
            assert!((mult[2] - 0.36).abs() < 1e-6);
            bc_gun_destroy(g);
            bc_weapon_free(w);
            bc_stats_free(st);
        }
    }
}
