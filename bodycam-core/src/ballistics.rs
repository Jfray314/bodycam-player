//! Bullet damage model of GA_Fire_Base (port of game/src/weapon.rs): bone multipliers, surface absorbance, penetration loop.
//! The game has no distance falloff: damage only changes by penetration (a bullet that went through surfaces keeps
//! `mult`, the product of the absorbances so far) and by the bone hit. The host does the raycasts.
use std::collections::HashSet;
use std::hash::Hash;

/// GA_Fire_Base.FireRange (500000 cm).
pub const FIRE_RANGE_M: f32 = 5000.0;
const MIN_BULLET_MULT: f32 = 0.25;

/// GetBoneDamageMultiplier; 1.0 for anything that isn't a named bone.
pub fn bone_multiplier(bone: &str) -> f32 {
    match bone.to_ascii_lowercase().as_str() {
        "head" | "neck" => 10.0,
        "spine_01" => 2.5,
        "spine_02" | "spine_03" => 4.0,
        _ => 1.0,
    }
}

/// Absorbance by EPhysicalSurface index (FXMaterialDataTable_GenericBullets rows). 0 = stops the bullet.
pub fn surface_absorbance(surface: u8) -> f32 {
    match surface {
        3 => 0.1,
        6 | 15 | 36 | 51 => 0.6,
        12 => 1.0,
        13 => 0.3,
        _ => 0.0,
    }
}

/// Hits in trace order as (actor, absorbance); each new actor records the current multiplier, then absorbs; stops once the
/// multiplier is <= 0.25. Returns (hit index, multiplier at that hit).
pub fn penetrate<T: Eq + Hash + Copy>(hits: &[(T, f32)]) -> Vec<(usize, f32)> {
    let (mut mult, mut seen, mut out) = (1.0f32, HashSet::new(), vec![]);
    for (i, (actor, absorb)) in hits.iter().enumerate() {
        if mult <= MIN_BULLET_MULT {
            break;
        }
        if seen.insert(*actor) {
            out.push((i, mult));
            mult *= absorb;
        }
    }
    out
}

/// Damage applied by one penetration event.
pub fn hit_damage(base: f32, mult: f32, bone: &str) -> f32 {
    base * mult * bone_multiplier(bone)
}

/// Does the bullet end at this hit (so the trail stops)?
pub fn stops(absorb: f32, mult: f32) -> bool {
    absorb * mult <= MIN_BULLET_MULT
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn penetration_and_bone_multipliers() {
        // same cases as game/src/weapon.rs tests
        let m: Vec<f32> = penetrate(&[(1, 0.6), (1, 0.6), (2, 0.6), (3, 0.6), (4, 0.6)]).iter().map(|h| h.1).collect();
        assert_eq!(m.len(), 3);
        assert!((m[1] - 0.6).abs() < 1e-6 && (m[2] - 0.36).abs() < 1e-6);
        assert_eq!(penetrate(&[(1, 1.0), (2, 0.0)]).len(), 2);
        assert_eq!((bone_multiplier("head"), bone_multiplier("spine_02"), bone_multiplier("spine_01"), bone_multiplier("hand_r")), (10.0, 4.0, 2.5, 1.0));
        assert_eq!(hit_damage(35.0, 0.6, "Head"), 35.0 * 0.6 * 10.0);
        assert!(stops(0.0, 1.0) && !stops(0.6, 1.0));
    }
}
