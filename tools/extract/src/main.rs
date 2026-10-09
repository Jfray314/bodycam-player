//! Read-only extractor: copies player-related files out of Bodycam's paks into ./extracted.
//! Usage: extract <Paks dir> <out dir> <regex-ish substring>...
use std::{fs, io::BufReader, path::Path};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (paks, out, filters) = (&args[0], Path::new(&args[1]), &args[2..]);
    for e in fs::read_dir(paks).unwrap().flatten() {
        let p = e.path();
        if p.extension().is_none_or(|x| x != "pak") { continue; }
        let mut r = BufReader::new(fs::File::open(&p).unwrap()); // read-only handle
        let pak = match repak::PakBuilder::new().reader(&mut r) {
            Ok(pak) => pak,
            Err(err) => { eprintln!("skip {}: {err}", p.display()); continue; }
        };
        let mount = pak.mount_point().trim_start_matches("../../../").to_string();
        for f in pak.files() {
            let full = format!("{mount}{f}");
            let low = full.to_lowercase();
            if !filters.iter().any(|k| low.contains(&k.to_lowercase())) { continue; }
            let dst = out.join(&full);
            if dst.exists() { continue; }
            fs::create_dir_all(dst.parent().unwrap()).unwrap();
            match pak.get(&f, &mut r) {
                Ok(bytes) => fs::write(&dst, bytes).unwrap(),
                Err(err) => eprintln!("fail {full}: {err}"),
            }
        }
    }
}
