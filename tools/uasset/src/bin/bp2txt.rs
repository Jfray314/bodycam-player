//! Blueprint -> text: class, variables (CDO defaults), components, and pseudo-code for every function.
use serde_json::Value;
use uasset::{kismet, package::Package, props::Reader, usmap::Usmap};

fn cut(s: &str) -> String {
    if s.len() > 240 { format!("{}...", s.chars().take(240).collect::<String>()) } else { s.into() }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        eprintln!("usage: bp2txt <usmap> <file.uasset>");
        std::process::exit(2);
    }
    let map = Usmap::load(&a[1]).unwrap_or_else(|e| panic!("usmap: {e}"));
    let pkg = Package::load(std::path::Path::new(&a[2])).unwrap_or_else(|e| panic!("{e}"));
    let rd = Reader::new(&pkg, &map);
    let cname = |i: usize| pkg.obj_name(pkg.exports[i].class).unwrap_or("None");
    let (mut ok, mut bad) = (0, 0);
    let classes: Vec<usize> = (0..pkg.exports.len()).filter(|&i| cname(i).ends_with("GeneratedClass")).collect();
    for &c in &classes {
        let d = match kismet::read_struct(&rd, c) {
            Ok(d) => d,
            Err(e) => {
                println!("// class {}: struct decode failed: {e}", pkg.exports[c].name);
                continue;
            }
        };
        println!("class {} : {}", pkg.exports[c].name, pkg.path(d.sup).unwrap_or_default());
        let cdo = (0..pkg.exports.len()).find(|&i| pkg.exports[i].flags & 0x10 != 0).map(|i| rd.export(i));
        let props = cdo.as_ref().and_then(|c| c["properties"].as_object());
        println!("\n// variables");
        for f in &d.fields {
            let def = props.and_then(|p| p.get(&f.name)).map(|v| format!(" = {}", cut(&v.to_string()))).unwrap_or_default();
            println!("var {}: {}{def}", f.name, f.ty);
        }
        println!("\n// CDO properties not matching a class variable (inherited defaults)");
        for (k, v) in props.into_iter().flatten().filter(|(k, _)| !d.fields.iter().any(|f| &f.name == *k)) {
            println!("default {k} = {v}");
        }
        println!("\n// components and construction script nodes");
        for i in 0..pkg.exports.len() {
            let (n, k) = (&pkg.exports[i].name, cname(i));
            if n.ends_with("_GEN_VARIABLE") || k == "SCS_Node" {
                let e = rd.export(i);
                let p: Vec<String> = e["properties"].as_object().into_iter().flatten().filter(|(_, v)| !matches!(v, Value::Null)).map(|(k, v)| format!("{k}={v}")).collect();
                println!("{k} {n}: {}", p.join(", "));
            }
        }
    }
    let entries = kismet::ubergraph_entries(&rd);
    println!("\n// functions");
    for i in 0..pkg.exports.len() {
        if !matches!(cname(i), "Function" | "DelegateFunction") {
            continue;
        }
        let name = &pkg.exports[i].name;
        match kismet::read_struct(&rd, i) {
            Ok(d) => {
                let (st, err) = kismet::decode(&pkg, &d.script, d.mem_size);
                let bt = kismet::bad_targets(&st);
                if err.is_some() || !bt.is_empty() {
                    bad += 1
                } else {
                    ok += 1
                }
                let e = if name.starts_with("ExecuteUbergraph") { entries.clone() } else { Default::default() };
                print!("\n{}", kismet::pseudo(&pkg, &d, name, &st, &e));
                if let Some(err) = err {
                    println!("  // DECODE ERROR: {err}");
                }
                if !bt.is_empty() {
                    println!("  // BAD JUMP TARGETS: {bt:?}");
                }
                if d.tail > 0 {
                    println!("  // tail bytes: {}", d.tail);
                }
            }
            Err(e) => {
                bad += 1;
                println!("\nfn {name}  // STRUCT DECODE ERROR: {e}");
            }
        }
    }
    eprintln!("{}: functions ok={ok} bad={bad}", a[2]);
}
