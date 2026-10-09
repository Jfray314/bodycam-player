//! Bakes the SoundCue graphs, attenuation presets and sound tables the game needs into one JSON.
//! Usage: audio-bake <extracted/json dir> <assets/weapons dir> <assets-local dir>
//! Writes <assets-local>/audio-data/audio.json and lists referenced SoundWaves with no decoded wav.
use serde_json::{json, Map, Value};
use std::{collections::{BTreeMap, BTreeSet}, fs, path::{Path, PathBuf}};

fn pkg_file(root: &Path, pkg: &str) -> PathBuf {
    let pkg = pkg.split('.').next().unwrap();
    match pkg.strip_prefix("/Game/") {
        Some(r) => root.join("Content").join(format!("{r}.json")),
        None => {
            let (plug, rest) = pkg[1..].split_once('/').unwrap_or((&pkg[1..], ""));
            root.join("Plugins").join(plug).join("Content").join(format!("{rest}.json"))
        }
    }
}

fn load(root: &Path, pkg: &str) -> Option<Value> {
    serde_json::from_str(&fs::read_to_string(pkg_file(root, pkg)).ok()?).ok()
}

fn walk(d: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(d).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() { walk(&p, out) } else if p.extension().is_some_and(|x| x == "json") { out.push(p) }
    }
}

/// Node references inside a cue become bare node names; the cue's own path means "no child".
fn norm(v: &Value, pkg: &str) -> Value {
    match v {
        Value::String(s) if s == pkg => Value::Null,
        Value::String(s) if s.starts_with(&format!("{pkg}.")) => Value::String(s.rsplit('.').next().unwrap().into()),
        Value::Array(a) => Value::Array(a.iter().map(|x| norm(x, pkg)).collect()),
        Value::Object(o) => Value::Object(o.iter().map(|(k, x)| (k.clone(), norm(x, pkg))).collect()),
        _ => v.clone(),
    }
}

fn short(s: &str) -> String { s.split('.').next().unwrap().to_string() }

struct Bake { root: PathBuf, cues: Map<String, Value>, att: Map<String, Value>, waves: BTreeSet<String>, chains: Map<String, Value>, todo: Vec<String> }

impl Bake {
    fn att(&mut self, p: &str) {
        let p = short(p);
        if self.att.contains_key(&p) { return }
        let v = load(&self.root, &p).map(|d| d["exports"][0]["properties"]["Attenuation"].clone()).unwrap_or(Value::Null);
        if v.is_null() { eprintln!("missing attenuation {p}") }
        self.att.insert(p, v);
    }
    fn chain(&mut self, p: &str) {
        let p = short(p);
        if self.chains.contains_key(&p) { return }
        let Some(d) = load(&self.root, &p) else { eprintln!("missing chain {p}"); self.chains.insert(p, Value::Null); return };
        let mut list = vec![];
        for l in d["exports"][0]["properties"]["Chain"].as_array().into_iter().flatten() {
            let Some(pr) = l["Preset"].as_str() else { continue };
            let preset = load(&self.root, pr).map(|d| d["exports"][0].clone()).unwrap_or(Value::Null);
            list.push(json!({"bypass": l["bBypass"].as_bool().unwrap_or(false), "class": preset["class"], "settings": preset["properties"]}));
        }
        self.chains.insert(p, Value::Array(list));
    }
    fn cue(&mut self, pkg: &str) {
        let pkg = short(pkg);
        if self.cues.contains_key(&pkg) { return }
        let Some(d) = load(&self.root, &pkg) else { eprintln!("missing cue {pkg}"); self.cues.insert(pkg, Value::Null); return };
        let mut nodes = Map::new();
        let mut head = Value::Null;
        for e in d["exports"].as_array().unwrap() {
            let class = e["class"].as_str().unwrap_or("").rsplit('.').next().unwrap().to_string();
            let p = norm(&e["properties"], &pkg);
            if class == "SoundCue" { head = p; continue }
            for (k, v) in p.as_object().into_iter().flatten() {
                match k.as_str() {
                    "AttenuationSettings" => self.att(v.as_str().unwrap()),
                    "SoundWaveAssetPtr" => { if let Some(s) = v.as_str() { self.waves.insert(short(s)); } }
                    "ChildNodes" => for c in v.as_array().unwrap() { if let Some(s) = c.as_str() && s.starts_with('/') { self.todo.push(s.into()) } },
                    _ => {}
                }
            }
            nodes.insert(e["name"].as_str().unwrap().into(), json!({"class": class, "p": p}));
        }
        if let Some(a) = head["AttenuationSettings"].as_str() { self.att(a) }
        if let Some(c) = head["SourceEffectChain"].as_str() { self.chain(c) }
        let conc = &head["ConcurrencyOverrides"];
        let cue = json!({
            "first": head["FirstNode"], "volume": head.get("VolumeMultiplier").unwrap_or(&json!(1.0)), "pitch": head.get("PitchMultiplier").unwrap_or(&json!(1.0)),
            "att": head["AttenuationSettings"], "chain": head["SourceEffectChain"],
            "max_count": conc["MaxCount"], "stop_oldest": conc["ResolutionRule"].as_str().is_some_and(|r| r.ends_with("StopOldest")),
            "nodes": nodes,
        });
        self.cues.insert(pkg, cue);
    }
    fn drain(&mut self) { while let Some(c) = self.todo.pop() { self.cue(&c) } }
}

fn wav_rel(pkg: &str) -> String {
    let p = pkg.strip_prefix("/Game/").map(|r| r.strip_prefix("Audio/").unwrap_or(r)).unwrap_or(&pkg[1..]);
    format!("audio/{p}.wav")
}

fn is_cue(root: &Path, p: &str) -> bool {
    load(root, p).is_some_and(|d| d["exports"].as_array().is_some_and(|e| e.iter().any(|x| x["class"].as_str().is_some_and(|c| c.ends_with("SoundCue")))))
}

fn collect_sounds(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(o) => for (k, x) in o { if k == "Sound" && x.is_string() { out.insert(short(x.as_str().unwrap())); } else { collect_sounds(x, out) } },
        Value::Array(a) => a.iter().for_each(|x| collect_sounds(x, out)),
        _ => {}
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (root, weapons, local) = (PathBuf::from(&a[1]), PathBuf::from(&a[2]), PathBuf::from(&a[3]));
    let mut b = Bake { root: root.clone(), cues: Map::new(), att: Map::new(), waves: BTreeSet::new(), chains: Map::new(), todo: vec![] };

    // Roots: every SoundCue under Content/Audio and the Ultimate_SFX gun/aim cues, plus whatever the table rows name.
    let mut files = vec![];
    for d in ["Content/Audio/Guns/Cue", "Content/Audio/Player", "Content/Ultimate_SFX/GunShot/Cue", "Content/Ultimate_SFX/ClothesSounds", "Content/Ultimate_SFX/Repiration"] { walk(&root.join(d), &mut files) }
    for f in files {
        let rel = f.strip_prefix(root.join("Content")).unwrap().with_extension("");
        let pkg = format!("/Game/{}", rel.to_string_lossy().replace('\\', "/"));
        if is_cue(&root, &pkg) { b.cue(&pkg) }
    }

    // Weapon cosmetic sounds (DT_WeaponItems) by row name.
    let dt = load(&root, "/Game/BodycamCore/ItemsDefinition/Gameplay/DT_WeaponItems").expect("DT_WeaponItems");
    let mut wsounds = Map::new();
    for (row, v) in dt["exports"][0]["properties"]["_rows"].as_object().unwrap() {
        if let Some(ce) = v["GameplayDataDefinition"]["CosmeticEffects"].as_object() {
            let m: Map<String, Value> = ce.iter().filter(|(k, _)| k.starts_with("Cosmetic.Weapon.SFX")).map(|(k, x)| (k.rsplit('.').next().unwrap().into(), json!(short(x.as_str().unwrap())))).collect();
            if !m.is_empty() {
                for x in m.values() { b.todo.push(x.as_str().unwrap().into()) }
                wsounds.insert(row.clone(), Value::Object(m));
            }
        }
    }
    // Footstep surfaces.
    let ft = load(&root, "/Game/AdvancedLocomotionV4/Audio/Footsteps/Blueprint/DT/DT_Footstep_Player").expect("DT_Footstep_Player");
    let mut surfaces = Map::new();
    for (_, v) in ft["exports"][0]["properties"]["_rows"].as_object().unwrap() {
        let s = v.as_object().unwrap().values().filter_map(|x| x.as_str()).collect::<Vec<_>>();
        if s.len() == 2 {
            let (sf, cue) = if s[0].starts_with("EPhysical") { (s[0], s[1]) } else { (s[1], s[0]) };
            surfaces.insert(sf.rsplit("::").next().unwrap().into(), json!(short(cue)));
            b.todo.push(cue.into())
        }
    }
    // Montage sound notifies from the baked per-weapon data.
    let mut notif = BTreeSet::new();
    let mut wf = vec![];
    walk(&weapons, &mut wf);
    for f in wf { if let Ok(v) = serde_json::from_str::<Value>(&fs::read_to_string(f).unwrap()) { collect_sounds(&v["anim_set"], &mut notif) } }
    for s in &notif { if is_cue(&root, s) { b.todo.push(s.clone()) } else { b.waves.insert(s.clone()); } }
    b.drain();

    let missing: Vec<&String> = b.waves.iter().filter(|w| !local.join(wav_rel(w)).exists()).collect();
    for m in &missing { println!("MISSING {m}") }
    eprintln!("cues {} att {} waves {} (missing {}) chains {}", b.cues.len(), b.att.len(), b.waves.len(), missing.len(), b.chains.len());
    let active: BTreeMap<_, _> = b.chains.iter().map(|(k, v)| (k.clone(), v.as_array().map_or(0, |l| l.iter().filter(|x| x["bypass"] == false).count()))).collect();
    eprintln!("active chain entries: {active:?}");
    let out = json!({"cues": b.cues, "att": b.att, "chains": b.chains, "weapons": wsounds, "footsteps": surfaces});
    fs::create_dir_all(local.join("audio-data")).unwrap();
    fs::write(local.join("audio-data/audio.json"), serde_json::to_string(&out).unwrap()).unwrap();
}
