//! Fire control: fire-rate cooldown, trigger modes, dry fire, spread cone, regrip probability (port of game/src/weapon.rs `fire`).
//! The host supplies the muzzle origin/direction, does the raycasts (see `ballistics`) and plays the sounds.
use crate::weapon_data::Stats;
use crate::{Key, curve_eval};

/// Source of uniform [0,1) numbers. Implement for the host RNG; `Rng` is a ready xorshift.
pub trait Uniform {
    fn next01(&mut self) -> f32;
}
pub struct Rng(pub u64);
impl Uniform for Rng {
    fn next01(&mut self) -> f32 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        ((self.0.wrapping_mul(0x2545F4914F6CDD1D) >> 40) as f32) / 16777216.0
    }
}
impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed | 1)
    }
}

pub fn map_range_clamped(v: f32, a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
    if a0 == a1 {
        return b1;
    }
    (b0 + (v - a0) / (a1 - a0) * (b1 - b0)).clamp(b0.min(b1), b0.max(b1))
}

type V3 = [f32; 3];
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn norm(a: V3) -> Option<V3> {
    let l = dot(a, a).sqrt();
    (l > 1e-10 && l.is_finite()).then(|| [a[0] / l, a[1] / l, a[2] / l])
}
fn rot(v: V3, axis: V3, ang: f32) -> V3 {
    let (s, c) = ang.sin_cos();
    let (x, d) = (cross(axis, v), dot(axis, v) * (1.0 - c));
    [v[0] * c + x[0] * s + axis[0] * d, v[1] * c + x[1] * s + axis[1] * d, v[2] * c + x[2] * s + axis[2] * d]
}

/// UE VRandCone with an elliptical cone (half angles in degrees): `dir` must be unit length. "Up" is +Y; a mirrored host
/// axis only mirrors the (symmetric) distribution.
pub fn random_in_elliptical_cone(dir: V3, yaw_deg: f32, pitch_deg: f32, rng: &mut impl Uniform) -> V3 {
    let (h, v) = (yaw_deg.to_radians(), pitch_deg.to_radians());
    if h <= 0.0 && v <= 0.0 {
        return dir;
    }
    let theta = std::f32::consts::TAU * rng.next01();
    let mut phi = (2.0 * rng.next01() - 1.0).acos();
    let half = if h > 0.0 && v > 0.0 { (1.0 / ((theta.cos() / v).powi(2) + (theta.sin() / h).powi(2))).sqrt() } else { h.max(v) };
    phi %= half;
    let right = norm(cross(dir, [0.0, 1.0, 0.0])).unwrap_or([1.0, 0.0, 0.0]);
    norm(rot(rot(dir, right, phi), dir, theta)).unwrap_or(dir)
}

/// RegripProbabilityCurve (PRAS), value over held-time 0.1..0.8 mapped to 0..1.
const REGRIP_CURVE: [Key; 2] = [[0.2, 0.1, 2.0, 0.035_373_74, 0.035_373_74], [1.2, 1.0, 2.0, 0.000_488_48, 0.000_488_48]];

/// Chance that releasing the trigger plays a regrip animation. `held` = how long this pull lasted, `gap` = this pull's start
/// minus the previous pull's start.
pub fn regrip_weight(full_auto: bool, held: f32, gap: f32, fire_rate: f32, instability: f32) -> f32 {
    if !full_auto {
        map_range_clamped(gap, fire_rate + 0.1, fire_rate * 2.0, 0.02, 1.0) * instability
    } else {
        curve_eval(&REGRIP_CURVE, map_range_clamped(held, 0.1, 0.8, 0.0, 1.0)) * instability
    }
}

/// What the gun needs from the weapon row.
#[derive(Clone, Debug)]
pub struct GunStats {
    pub fire_rate: f32,
    pub full_auto: bool,
    pub instability: f32,
    pub pellets: u32,
    pub spread_hip: (f32, f32),
    pub spread_aim: (f32, f32),
    pub damage: f32,
}
impl GunStats {
    pub fn from_stats(s: &Stats) -> Self {
        GunStats {
            fire_rate: s.fire_rate.unwrap_or(0.1),
            full_auto: s.fire_modes.iter().any(|m| m == "FullAuto"),
            instability: s.instability.unwrap_or(0.0),
            pellets: s.pellet_count.unwrap_or(1.0) as u32,
            spread_hip: (s.spread_hip_x.unwrap_or(0.0), s.spread_hip_y.unwrap_or(0.0)),
            spread_aim: (s.spread_aim_x.unwrap_or(0.0), s.spread_aim_y.unwrap_or(0.0)),
            damage: s.damage.unwrap_or(30.0),
        }
    }
}

#[derive(Default, Clone, Copy, Debug)]
pub struct TriggerIn {
    /// Seconds this frame.
    pub dt: f32,
    /// Clock in seconds (any origin).
    pub now: f32,
    /// Fire button held / went down this frame.
    pub held: bool,
    pub just_pressed: bool,
    /// Reloading/drawing/etc, sprinting, dead.
    pub blocked: bool,
    pub aiming: bool,
}

#[derive(Default, Debug, Clone, PartialEq)]
pub struct FireOut {
    /// A round was fired this frame.
    pub fired: bool,
    /// Pulled the trigger on an empty gun (click).
    pub dry: bool,
    /// Trigger released: play a regrip after this many seconds.
    pub regrip_delay: Option<f32>,
}

#[derive(Default, Debug, Clone)]
pub struct FireControl {
    pub ammo: u32,
    pub cooldown: f32,
    fire_start: Option<f32>,
    last_fire_start: f32,
}

impl FireControl {
    pub fn new(ammo: u32) -> Self {
        FireControl { ammo, ..Default::default() }
    }

    /// One frame. Call `shot_dirs` after a `fired` result to get the pellet directions.
    pub fn step(&mut self, g: &GunStats, i: &TriggerIn, rng: &mut impl Uniform) -> FireOut {
        let mut out = FireOut::default();
        self.cooldown -= i.dt;
        // GA_Fire_Base ability end (trigger released): regrip roll, then LastFireTime = FireStartTime.
        if let Some(start) = self.fire_start
            && !i.held
        {
            let weight = regrip_weight(g.full_auto, i.now - start, start - self.last_fire_start, g.fire_rate, g.instability);
            if rng.next01() < weight {
                out.regrip_delay = Some(0.1 + rng.next01() * 0.4);
            }
            self.last_fire_start = start;
            self.fire_start = None;
        }
        let trigger = if g.full_auto { i.held } else { i.just_pressed };
        if trigger && self.ammo == 0 && i.just_pressed && self.cooldown <= 0.0 && !i.blocked {
            out.dry = true;
        }
        if !trigger || self.cooldown > 0.0 || i.blocked || self.ammo == 0 {
            return out;
        }
        self.ammo -= 1;
        self.fire_start.get_or_insert(i.now);
        self.cooldown = g.fire_rate;
        out.fired = true;
        out
    }
}

/// One direction per pellet around `aim_dir` (unit), using the hip or aim spread.
pub fn shot_dirs(g: &GunStats, aim_dir: V3, aiming: bool, rng: &mut impl Uniform) -> Vec<V3> {
    let (sx, sy) = if aiming { g.spread_aim } else { g.spread_hip };
    (0..g.pellets).map(|_| random_in_elliptical_cone(aim_dir, sx, sy, rng)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    const M4: GunStats = GunStats { fire_rate: 0.1, full_auto: true, instability: 0.5, pellets: 1, spread_hip: (1.0, 1.0), spread_aim: (0.0, 0.0), damage: 35.0 };

    #[test]
    fn elliptical_spread_stays_in_ellipse_and_is_wider_in_yaw() {
        let mut rng = Rng::new(7);
        let (mut sh, mut sv) = (0.0f32, 0.0f32);
        for _ in 0..20000 {
            let d = random_in_elliptical_cone([0.0, 0.0, -1.0], 6.0, 2.0, &mut rng);
            let (h, v) = (d[0].asin().to_degrees(), d[1].asin().to_degrees());
            assert!((h / 6.0).powi(2) + (v / 2.0).powi(2) < 1.05, "{h} {v}");
            sh += h * h;
            sv += v * v;
        }
        assert!(sh > sv * 2.0, "yaw spread {sh} vs pitch {sv}");
        assert_eq!(random_in_elliptical_cone([0.0, 0.0, -1.0], 0.0, 0.0, &mut rng), [0.0, 0.0, -1.0]);
    }

    #[test]
    fn full_auto_cadence_and_dry_fire() {
        let mut f = FireControl::new(3);
        let mut rng = Rng::new(1);
        let mut shots = vec![];
        for n in 0..60 {
            let i = TriggerIn { dt: 0.01, now: n as f32 * 0.01, held: true, just_pressed: n == 0, ..Default::default() };
            let o = f.step(&M4, &i, &mut rng);
            if o.fired {
                shots.push(n);
            }
            if o.dry {
                shots.push(1000 + n);
            }
        }
        // 0.1 s cooldown at 100 Hz = every 10-11 frames; out of ammo after 3; dry click only on the press frame (n==0 is a shot)
        assert_eq!(shots.len(), 3, "{shots:?}");
        assert!(shots[1] - shots[0] >= 10 && shots[1] - shots[0] <= 11);
        // fresh press on the empty gun clicks once
        let o = f.step(&M4, &TriggerIn { dt: 0.2, now: 1.0, held: true, just_pressed: true, ..Default::default() }, &mut rng);
        assert!(o.dry && !o.fired);
        // blocked = nothing
        let mut f = FireControl::new(3);
        assert_eq!(f.step(&M4, &TriggerIn { dt: 0.01, held: true, just_pressed: true, blocked: true, ..Default::default() }, &mut rng), FireOut::default());
    }

    #[test]
    fn semi_auto_needs_a_fresh_press() {
        let g = GunStats { full_auto: false, ..M4 };
        let mut f = FireControl::new(10);
        let mut rng = Rng::new(3);
        let n: usize = (0..50).filter(|&n| f.step(&g, &TriggerIn { dt: 0.01, now: n as f32 * 0.01, held: true, just_pressed: n == 0, ..Default::default() }, &mut rng).fired).count();
        assert_eq!(n, 1);
    }

    #[test]
    fn regrip_weights() {
        // semi: gap at/under FireRate+0.1 -> 0.02*instability, at 2*FireRate... clamps to 1.0*instability (range reversed)
        assert!((regrip_weight(false, 0.0, 0.0, 0.3, 0.5) - 0.02 * 0.5).abs() < 1e-6);
        assert!((regrip_weight(true, 0.0, 0.0, 0.1, 1.0) - 0.1).abs() < 1e-6); // curve value at t=0 is key 0 value
        assert!(regrip_weight(true, 10.0, 0.0, 0.1, 1.0) > 0.9); // curve keys end at 1.2: x=1 is just under 1
    }

    #[test]
    fn pellets_per_shot() {
        let g = GunStats { pellets: 8, ..M4 };
        assert_eq!(shot_dirs(&g, [0.0, 0.0, -1.0], false, &mut Rng::new(5)).len(), 8);
    }
}
