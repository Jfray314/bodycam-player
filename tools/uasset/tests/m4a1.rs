use std::path::Path;
use uasset::{package::Package, props::Reader, usmap::Usmap};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../extracted");

#[test]
fn m4a1_modifiers() {
    let asset = Path::new(ROOT).join("Bodycam/Content/BodycamCore/AbilitySystem/GameplayEffects/Weapons/Stats/GE_WeaponStat_M4A1.uasset");
    let usmap = Path::new(ROOT).join("Bodycam.usmap");
    if !asset.exists() || !usmap.exists() {
        eprintln!("skipped: extracted assets not present");
        return;
    }
    let map = Usmap::load(usmap.to_str().unwrap()).unwrap();
    let pkg = Package::load(&asset).unwrap();
    let rd = Reader::new(&pkg, &map);
    let cdo = (0..pkg.exports.len()).map(|i| rd.export(i)).find(|e| e["cdo"] == true).unwrap();
    assert_eq!(cdo["status"], "ok");
    let got: Vec<(String, f64)> = cdo["properties"]["Modifiers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["Attribute"]["AttributeName"].as_str().unwrap().to_string(),
                m["ModifierMagnitude"]["ScalableFloatMagnitude"]["Value"].as_f64().unwrap(),
            )
        })
        .collect();
    for (name, v) in [("Damage", 35.0), ("FireRate", 0.09), ("Instability", 0.75), ("MagAmount", 4.0), ("RagdollForce", 2100.0)] {
        let g = got.iter().find(|(n, _)| n == name).unwrap_or_else(|| panic!("missing {name}")).1;
        assert!((g - v).abs() < 1e-6, "{name}: {g} != {v}");
    }
}
