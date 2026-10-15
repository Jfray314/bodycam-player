//! Weapon data loading. The host hands over JSON strings (assets/weapon_stats.json and assets/weapons/<Row>.json, extracted
//! from the user's own install); nothing is embedded here.
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

/// GE_WeaponStat_* values for one row (missing ones inherit the table's `Default` row).
#[derive(Deserialize, Clone, Debug, Default)]
#[serde(rename_all = "PascalCase")]
pub struct Stats {
    pub damage: Option<f32>,
    pub fire_rate: Option<f32>,
    pub instability: Option<f32>,
    pub mag_amount: Option<f32>,
    pub ammo_capacity: Option<f32>,
    pub pellet_count: Option<f32>,
    pub ragdoll_force: Option<f32>,
    pub spread_hip_x: Option<f32>,
    pub spread_hip_y: Option<f32>,
    pub spread_aim_x: Option<f32>,
    pub spread_aim_y: Option<f32>,
    #[serde(default, rename = "fire_modes")]
    pub fire_modes: Vec<String>,
    #[serde(default, rename = "tags")]
    pub tags: Vec<String>,
}

impl Stats {
    fn over(self, d: &Stats) -> Stats {
        Stats {
            damage: self.damage.or(d.damage),
            fire_rate: self.fire_rate.or(d.fire_rate),
            instability: self.instability.or(d.instability),
            mag_amount: self.mag_amount.or(d.mag_amount),
            ammo_capacity: self.ammo_capacity.or(d.ammo_capacity),
            pellet_count: self.pellet_count.or(d.pellet_count),
            ragdoll_force: self.ragdoll_force.or(d.ragdoll_force),
            spread_hip_x: self.spread_hip_x.or(d.spread_hip_x),
            spread_hip_y: self.spread_hip_y.or(d.spread_hip_y),
            spread_aim_x: self.spread_aim_x.or(d.spread_aim_x),
            spread_aim_y: self.spread_aim_y.or(d.spread_aim_y),
            fire_modes: if self.fire_modes.is_empty() { d.fire_modes.clone() } else { self.fire_modes },
            tags: if self.tags.is_empty() { d.tags.clone() } else { self.tags },
        }
    }
}

/// Parse weapon_stats.json: rows with the `Default` row folded in (and removed).
pub fn load_stats(json: &str) -> Result<HashMap<String, Stats>, String> {
    let mut all: HashMap<String, Stats> = serde_json::from_str(json).map_err(|e| e.to_string())?;
    let d = all.remove("Default").unwrap_or_default();
    Ok(all.into_iter().map(|(k, s)| (k, s.over(&d))).collect())
}

/// PRAS RecoilData.ControllerRecoil of a weapon's anim set (same parse as the game).
pub fn controller_recoil(anims: &Value) -> crate::ControllerRecoil {
    let c = &anims["RecoilData"]["ControllerRecoil"];
    let r = |k: &str| (c[k]["x"].as_f64().unwrap_or(0.0) as f32, c[k]["y"].as_f64().unwrap_or(0.0) as f32);
    let (lo, hi) = r("HorizontalRecoilStep");
    let (vlo, vhi) = r("VerticalRecoilStep");
    crate::ControllerRecoil { horizontal: (lo.min(hi), lo.max(hi)), vertical: (vlo.min(vhi), vlo.max(vhi)), smoothing: r("RecoilSmoothing").1.max(1.0), damping: c["Damping"].as_f64().unwrap_or(0.0) as f32, compensation: c["Compensation"].as_f64().unwrap_or(1.0) as f32 }
}

/// One weapon: resolved stats + its baked animation-set JSON (sway tables, recoil data, sounds ...).
pub struct WeaponData {
    pub row: String,
    pub stats: Stats,
    pub anims: Value,
}

impl WeaponData {
    /// `weapon_json` = assets/weapons/<Row>.json; `stats` from `load_stats` (row missing = all defaults).
    pub fn from_json(weapon_json: &str, stats: &HashMap<String, Stats>) -> Result<Self, String> {
        let v: Value = serde_json::from_str(weapon_json).map_err(|e| e.to_string())?;
        let row = v["row"].as_str().ok_or("no row")?.to_string();
        let anims = v.get("anim_set").cloned().unwrap_or(Value::Null);
        // stats row = the GE_WeaponStat_<Key> the weapon grants (not always the item row name)
        let ge = v["stats_effects"].get(0).and_then(Value::as_str).unwrap_or("");
        let key = ge.rsplit("GE_WeaponStat_").next().unwrap_or("").trim_end_matches("_C").trim_start_matches('_');
        let stats = stats.get(key).cloned().unwrap_or_default();
        Ok(WeaponData { row, stats, anims: anims.clone() }.with_sounds(v))
    }
    fn with_sounds(mut self, v: Value) -> Self {
        // keep the whole row reachable for `sound`: stash it under a key anim sets never use
        if let Some(s) = v.get("sounds") {
            self.anims["_sounds"] = s.clone();
        }
        self
    }
    /// Cue asset path for a `Cosmetic.Weapon.SFX.<kind>` entry (Fire, FireSuppressed, FireDry, BulletCrack, BulletWizz).
    pub fn sound(&self, kind: &str) -> Option<&str> {
        self.anims["_sounds"][format!("Cosmetic.Weapon.SFX.{kind}")].as_str()
    }
    pub fn full_auto(&self) -> bool {
        self.stats.fire_modes.iter().any(|m| m == "FullAuto")
    }
    pub fn infinite_mags(&self) -> bool {
        self.stats.tags.iter().any(|t| t == "Gameplay.Weapons.InfiniteMag")
    }
}
