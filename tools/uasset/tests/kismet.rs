use std::path::Path;
use uasset::{kismet, package::Package, props::Reader, usmap::Usmap};

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../extracted");
const BP: &str = "Bodycam/Plugins/BodycamAnimationFramework/Content/Blueprints";

/// Every Function export decodes with no unknown opcodes, exact in-memory size, jump targets on statement starts.
fn check(rel: &str, min_fns: usize) {
    let (asset, usmap) = (Path::new(ROOT).join(rel), Path::new(ROOT).join("Bodycam.usmap"));
    if !asset.exists() || !usmap.exists() {
        eprintln!("skipped: extracted assets not present");
        return;
    }
    let map = Usmap::load(usmap.to_str().unwrap()).unwrap();
    let pkg = Package::load(&asset).unwrap();
    let rd = Reader::new(&pkg, &map);
    let mut n = 0;
    for (i, e) in pkg.exports.iter().enumerate() {
        if !matches!(pkg.obj_name(e.class), Some("Function" | "DelegateFunction")) {
            continue;
        }
        let d = kismet::read_struct(&rd, i).unwrap_or_else(|err| panic!("{}: {err}", e.name));
        let (st, err) = kismet::decode(&pkg, &d.script, d.mem_size);
        assert!(err.is_none(), "{}: {err:?}", e.name);
        assert_eq!(kismet::bad_targets(&st), Vec::<u32>::new(), "{}", e.name);
        assert_eq!(d.tail, 0, "{}: trailing bytes", e.name);
        n += 1;
    }
    assert!(n >= min_fns, "only {n} functions");
}

#[test]
fn simple_bp() {
    check(&format!("{BP}/BP_BodycamPlayerCameraManager.uasset"), 1);
}

#[test]
fn bodycam_player() {
    check(&format!("{BP}/Bodycam_Player.uasset"), 100);
}
