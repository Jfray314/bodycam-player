//! BODYCAM_PERF=1: every 5 s log avg / 1%-low frame time (ms), entity count and GPU pass times. =2 also runs uncapped
//! (no vsync / fps limit). =img dumps loaded Image/Mesh bytes by source every 20 s (VRAM hunting).
use bevy::prelude::*;

pub struct PerfPlugin;
impl Plugin for PerfPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var("BODYCAM_PERF").is_err() { return; }
        app.add_plugins(bevy::render::diagnostic::RenderDiagnosticsPlugin);
        app.add_systems(Last, |imgs: Res<Assets<Image>>, meshes: Res<Assets<Mesh>>, srv: Res<AssetServer>, time: Res<Time<Real>>, mut next: Local<f32>| {
            if time.elapsed_secs() < *next || std::env::var("BODYCAM_PERF").is_ok_and(|v| v != "img") { return; }
            *next = time.elapsed_secs() + 20.0;
            let mut v: Vec<(usize, String)> = imgs.iter().map(|(id, i)| (i.data.as_ref().map_or(0, |d| d.len()), format!("{:?} {}x{}x{} mips={} {}", i.texture_descriptor.format, i.width(), i.height(), i.texture_descriptor.size.depth_or_array_layers, i.texture_descriptor.mip_level_count, srv.get_path(id).map(|p| p.to_string()).unwrap_or_default()))).collect();
            v.sort_by(|a, b| b.0.cmp(&a.0));
            let tot: usize = v.iter().map(|x| x.0).sum();
            let mesh_b: usize = meshes.iter().map(|(_, m)| m.get_vertex_buffer_size() + m.get_index_buffer_bytes().map_or(0, |b| b.len())).sum();
            println!("IMG n={} cpu_bytes={}MB meshes n={} {}MB", v.len(), tot >> 20, meshes.len(), mesh_b >> 20);
            let mut g: std::collections::BTreeMap<String, (usize, usize)> = default();
            for (b, n) in &v { let k = n.split(" mips=").nth(1).map(|r| r.split_once(' ').map_or("", |x| x.1)).unwrap_or(""); let k = k.split('#').next().unwrap_or("").to_string(); let e = g.entry(k).or_default(); e.0 += 1; e.1 += b; }
            let mut gv: Vec<_> = g.into_iter().collect(); gv.sort_by(|a, b| b.1.1.cmp(&a.1.1));
            for (k, (n, b)) in gv.iter().take(25) { println!("IMG  {}MB x{n} {k}", b >> 20); }
        });
        app.add_systems(Last, |diag: Res<bevy::diagnostic::DiagnosticsStore>, time: Res<Time<Real>>, q: Query<Entity>, mut v: Local<Vec<f32>>, mut acc: Local<f32>| {
            let dt = time.delta_secs() * 1000.0;
            v.push(dt);
            *acc += dt;
            if *acc >= 5000.0 {
                v.sort_by(|a, b| b.partial_cmp(a).unwrap());
                let n = v.len();
                let k = (n / 100).max(1);
                let avg = v.iter().sum::<f32>() / n as f32;
                let low: f32 = v[..k].iter().sum::<f32>() / k as f32;
                println!("PERF frames={n} avg={avg:.2}ms ({:.0}fps) 1%low={low:.2}ms ({:.0}fps) max={:.1} entities={}", 1000.0 / avg, 1000.0 / low, v[0], q.iter().count());
                let mut g: Vec<(f64, String)> = diag.iter().filter(|d| d.path().as_str().ends_with("elapsed_gpu")).filter_map(|d| Some((d.average()?, d.path().to_string()))).collect();
                g.sort_by(|a, b| b.0.total_cmp(&a.0));
                println!("GPU {}", g.iter().take(8).map(|(t, n)| format!("{n}={t:.2}ms")).collect::<Vec<_>>().join(" "));
                v.clear();
                *acc = 0.0;
            }
        });
    }
}
