use serde_json::json;
use std::path::{Path, PathBuf};
use uasset::{package::Package, props::Reader, usmap::Usmap};

fn walk(p: &Path, out: &mut Vec<PathBuf>) {
    if p.is_dir() {
        let mut v: Vec<_> = std::fs::read_dir(p).into_iter().flatten().flatten().map(|e| e.path()).collect();
        v.sort();
        v.iter().for_each(|c| walk(c, out));
    } else if p.extension().is_some_and(|e| e == "uasset") {
        out.push(p.to_path_buf());
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    if a.len() < 3 {
        eprintln!("usage: dump <usmap> <file.uasset|dir> [out_dir]");
        std::process::exit(2);
    }
    let map = Usmap::load(&a[1]).unwrap_or_else(|e| {
        eprintln!("usmap: {e}");
        std::process::exit(1)
    });
    let root = PathBuf::from(&a[2]);
    let mut files = vec![];
    walk(&root, &mut files);
    let base = if root.is_dir() { root.clone() } else { root.parent().map(Path::to_path_buf).unwrap_or_default() };
    let (mut ok, mut partial, mut fail) = (0, 0, 0);
    let mut unsupported = std::collections::BTreeSet::new();
    for f in files {
        let rel = f.strip_prefix(&base).unwrap_or(&f).to_path_buf();
        let res = Package::load(&f).map(|pkg| {
            let rd = Reader::new(&pkg, &map);
            let exports: Vec<_> = (0..pkg.exports.len()).map(|i| rd.export(i)).collect();
            let bad: Vec<_> = exports.iter().filter(|e| e["status"] == "partial").collect();
            let msg = bad.first().map(|e| format!("{}: {}", e["name"].as_str().unwrap_or(""), e["error"].as_str().unwrap_or("")));
            unsupported.extend(rd.unsupported.borrow().iter().cloned());
            (json!({"package": pkg.pkg_name, "exports": exports}), msg, bad.len(), pkg.exports.len())
        });
        match res {
            Ok((v, msg, nbad, n)) => {
                if let (Some(out), true) = (a.get(3), true) {
                    let dest = Path::new(out).join(&rel).with_extension("json");
                    std::fs::create_dir_all(dest.parent().unwrap()).ok();
                    std::fs::write(dest, serde_json::to_string_pretty(&v).unwrap()).ok();
                }
                if nbad == 0 {
                    ok += 1;
                    println!("ok      {}", rel.display());
                } else {
                    partial += 1;
                    println!("partial {} ({nbad}/{n} exports) {}", rel.display(), msg.unwrap_or_default());
                }
            }
            Err(e) => {
                fail += 1;
                println!("fail    {} {e}", rel.display());
            }
        }
    }
    println!("SUMMARY ok={ok} partial={partial} fail={fail}");
    if !unsupported.is_empty() {
        println!("UNSUPPORTED STRUCTS: {unsupported:?}");
    }
}
