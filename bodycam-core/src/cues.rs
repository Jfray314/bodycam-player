//! Audio cue selection (port of game/src/audio.rs footsteps / weapon sound lookup). The core only picks cue names; the host
//! plays them from audio it extracted itself. Table: the `footsteps` and `weapons` maps of the baked audio.json (extra keys ignored).
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize, Default)]
pub struct CueBank {
    #[serde(default)]
    weapons: HashMap<String, HashMap<String, String>>,
    #[serde(default)]
    footsteps: HashMap<String, Option<String>>,
}

/// Foley layers per footstep: (cue prefix, played on "step" = every step or "stride" = stride start only).
const FOLEY: [(&str, &str); 4] = [
    ("/Game/Audio/Player/Foley/Armor/Foley_Cloth_Move_Gear_", "step"),
    ("/Game/Audio/Player/Foley/Legs/Foley_Cloth_Move_Leg_", "step"),
    ("/Game/Audio/Player/Foley/Arms/Foley_Cloth_Move_Arms_", "stride"),
    ("/Game/Audio/Player/Foley/Bodycam/Foley_Bodycam_", "stride"),
];

/// Core `BcOutput::footstep` values.
pub const STEP_CROUCH: i32 = 1;
pub const STEP_WALK: i32 = 2;
pub const STEP_RUN: i32 = 3;
pub const STEP_LAND: i32 = 4;

pub fn gait_name(kind: i32) -> &'static str {
    match kind {
        STEP_CROUCH => "Crouch",
        STEP_WALK => "Walk",
        _ => "Run",
    }
}

/// DT_Footstep_Player row for an EPhysicalSurface index (`None` = no physical material: Default).
pub fn footstep_key(surface: Option<u8>) -> String {
    surface.map_or("SurfaceType_Default".to_string(), |n| format!("SurfaceType{n}"))
}

impl CueBank {
    pub fn from_json(s: &str) -> Result<Self, String> {
        serde_json::from_str(s).map_err(|e| e.to_string())
    }

    /// Footstep cue for a surface row, falling back to the Default row.
    pub fn footstep_cue(&self, surface: &str) -> Option<&str> {
        self.footsteps.get(surface).or(self.footsteps.get("SurfaceType_Default"))?.as_deref()
    }

    /// `kind`: Fire, FireSuppressed, FireDry, BulletCrack, BulletWizz. Weapon names match ignoring case and punctuation.
    pub fn weapon_cue(&self, weapon: &str, kind: &str) -> Option<&str> {
        let key = |s: &str| s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase();
        let w = key(weapon);
        self.weapons.iter().find(|(r, _)| key(r) == w)?.1.get(kind).map(String::as_str)
    }

    /// Everything to play for one footstep event: the surface cue first, then foley layers (not for landings).
    pub fn footstep_cues(&self, kind: i32, stride_start: bool, surface: Option<u8>) -> Vec<String> {
        let mut out: Vec<String> = self.footstep_cue(&footstep_key(surface)).map(str::to_string).into_iter().collect();
        if kind != STEP_LAND {
            let gait = gait_name(kind);
            out.extend(FOLEY.iter().filter(|(_, when)| *when == "step" || stride_start).map(|(p, _)| format!("{p}{gait}")));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn bank() -> CueBank {
        CueBank::from_json(r#"{"footsteps":{"SurfaceType_Default":"cue/default","SurfaceType3":"cue/metal","SurfaceType9":null},"weapons":{"CZ-P-09":{"Fire":"cue/cz"}},"cues":{}}"#).unwrap()
    }
    #[test]
    fn surface_and_fallback() {
        let b = bank();
        assert_eq!(b.footstep_cue(&footstep_key(Some(3))), Some("cue/metal"));
        assert_eq!(b.footstep_cue(&footstep_key(Some(7))), Some("cue/default")); // unknown row -> Default
        assert_eq!(b.footstep_cue(&footstep_key(None)), Some("cue/default"));
        assert_eq!(b.footstep_cue("SurfaceType9"), None); // explicitly silent row
    }
    #[test]
    fn foley_layers_by_event() {
        let b = bank();
        assert_eq!(b.footstep_cues(STEP_WALK, false, None).len(), 1 + 2); // surface + step layers
        let s = b.footstep_cues(STEP_RUN, true, Some(3));
        assert_eq!(s.len(), 1 + 4);
        assert!(s[1].ends_with("Gear_Run") && s[4].ends_with("Foley_Bodycam_Run"));
        assert_eq!(b.footstep_cues(STEP_LAND, true, None), vec!["cue/default"]);
    }
    #[test]
    fn weapon_name_matching() {
        assert_eq!(bank().weapon_cue("cz p 09", "Fire"), Some("cue/cz"));
        assert_eq!(bank().weapon_cue("AK47", "Fire"), None);
    }
}
