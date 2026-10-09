use std::{fs, path::{Path, PathBuf}, sync::{atomic::{AtomicUsize, Ordering::Relaxed}, Mutex}};

fn walk(d: &Path, out: &mut Vec<PathBuf>) {
    for e in fs::read_dir(d).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() { walk(&p, out) } else if p.extension().is_some_and(|x| x == "uasset") { out.push(p) }
    }
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let (inp, outp) = (PathBuf::from(&a[1]), PathBuf::from(&a[2]));
    let mut files = vec![];
    walk(&inp, &mut files);
    let (ok, bad, skip) = (AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0));
    let (codecs, report) = (Mutex::new(std::collections::BTreeMap::<String, usize>::new()), Mutex::new(vec![]));
    let next = AtomicUsize::new(0);
    std::thread::scope(|s| for _ in 0..std::thread::available_parallelism().map_or(4, |n| n.get()) { s.spawn(|| loop {
        let i = next.fetch_add(1, Relaxed);
        let Some(p) = files.get(i) else { break };
        let Ok(ua) = fs::read(p) else { continue };
        let codec = binka::codec_name(&ua);
        let Ok(ue) = fs::read(p.with_extension("uexp")) else { continue };
        // only SoundWaves have a UEBA header; other waves are counted as unsupported, other assets ignored
        if !ue.windows(4).any(|w| w == b"ABEU") {
            if codec != "none" && ua.windows(9).any(|w| w == b"SoundWave") {
                *codecs.lock().unwrap().entry(codec.into()).or_default() += 1;
                skip.fetch_add(1, Relaxed);
            }
            continue;
        }
        *codecs.lock().unwrap().entry("BINKA".into()).or_default() += 1;
        let rel = p.strip_prefix(&inp).unwrap().with_extension("wav");
        let bulk = fs::read(p.with_extension("ubulk")).ok();
        match binka::decode(&ue, bulk.as_deref()) {
            Ok(d) => {
                let dst = outp.join(&rel);
                fs::create_dir_all(dst.parent().unwrap()).unwrap();
                let spec = hound::WavSpec { channels: d.channels as u16, sample_rate: d.rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
                let mut w = hound::WavWriter::create(&dst, spec).unwrap();
                let n = d.pcm[0].len();
                let (mut ss, mut pk) = (0f64, 0f32);
                for t in 0..n { for c in 0..d.channels {
                    let v = d.pcm[c][t];
                    ss += (v * v) as f64; pk = pk.max(v.abs());
                    w.write_sample((v * 32767.0).clamp(-32768.0, 32767.0) as i16).unwrap();
                } }
                w.finalize().unwrap();
                let rms = (ss / (n * d.channels).max(1) as f64).sqrt();
                report.lock().unwrap().push(format!("{}\t{:.2}s\trms={:.4}\tpeak={:.3}", rel.display(), n as f32 / d.rate as f32, rms, pk));
                ok.fetch_add(1, Relaxed);
            }
            Err(e) => { eprintln!("ERR {}: {e}", rel.display()); bad.fetch_add(1, Relaxed); }
        }
    }); });
    let mut r = report.into_inner().unwrap();
    r.sort();
    fs::create_dir_all(&outp).unwrap();
    fs::write(outp.join("report.tsv"), r.join("\n")).unwrap();
    println!("ok={} error={} unsupported={} codecs={:?}", ok.load(Relaxed), bad.load(Relaxed), skip.load(Relaxed), codecs.into_inner().unwrap());
}
