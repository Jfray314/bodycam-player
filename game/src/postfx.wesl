// Bodycam camera look: one fullscreen pass, runs in linear HDR before tonemapping.
import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct Fx {
    distortion: f32, edge_stretch: f32, ca: f32, vignette: f32,
    vignette_radius: f32, vignette_oval: f32, edge_blur: f32, edge_blur_start: f32,
    sharpen: f32, grain: f32, temporal_noise: f32, banding: f32,
    dirt: f32, dirt_threshold: f32, dirt_gain: f32, exposure_bias: f32,
    rolling_shutter: f32, flash_amount: f32, damage_amount: f32, explosion_shake: f32,
    dirt_boost: f32, time: f32, yaw_rate: f32, pitch_rate: f32,
    pump: f32, tint_r: f32, tint_g: f32, tint_b: f32,
}

@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var<uniform> fx: Fx;

fn pcg3(v0: vec3<u32>) -> vec3<f32> {
    var v = v0 * 1664525u + 1013904223u;
    v.x += v.y * v.z; v.y += v.z * v.x; v.z += v.x * v.y;
    v ^= v >> vec3<u32>(16u);
    v.x += v.y * v.z; v.y += v.z * v.x; v.z += v.x * v.y;
    return vec3<f32>(v >> vec3<u32>(8u)) / 16777216.0;
}

fn bil(uv: vec2<f32>) -> vec3<f32> {
    let dims = vec2<f32>(textureDimensions(tex));
    let p = uv * dims - 0.5;
    let i = vec2<i32>(floor(p));
    let f = fract(p);
    let mx = vec2<i32>(dims) - 1;
    let a = textureLoad(tex, clamp(i, vec2<i32>(0), mx), 0).rgb;
    let b = textureLoad(tex, clamp(i + vec2<i32>(1, 0), vec2<i32>(0), mx), 0).rgb;
    let c = textureLoad(tex, clamp(i + vec2<i32>(0, 1), vec2<i32>(0), mx), 0).rgb;
    let d = textureLoad(tex, clamp(i + vec2<i32>(1, 1), vec2<i32>(0), mx), 0).rgb;
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = vec2<u32>(vec2<i32>(floor(p)) + 4096);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = pcg3(vec3<u32>(i, 1u)).x;
    let b = pcg3(vec3<u32>(i + vec2<u32>(1u, 0u), 1u)).x;
    let c = pcg3(vec3<u32>(i + vec2<u32>(0u, 1u), 1u)).x;
    let d = pcg3(vec3<u32>(i + vec2<u32>(1u, 1u), 1u)).x;
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p0: vec2<f32>) -> f32 {
    var p = p0; var a = 0.5; var s = 0.0;
    for (var i = 0; i < 5; i++) { s += a * vnoise(p); p = p * 2.03 + 11.7; a *= 0.5; }
    return s;
}

// Procedural stand-in for T_DirtMask_01: smudges, blotches, specks.
fn dirt_mask(uv: vec2<f32>, aspect: f32) -> f32 {
    let p = uv * vec2<f32>(aspect, 1.0);
    let smudge = smoothstep(0.52, 0.85, fbm(p * 3.0));
    let blotch = smoothstep(0.6, 0.9, fbm(p * 1.3 + 40.0));
    let cell = floor(p * 28.0);
    let h = pcg3(vec3<u32>(vec2<u32>(vec2<i32>(cell + 512.0)), 7u));
    let dp = fract(p * 28.0) - h.xy * 0.6 - 0.2;
    let speck = select(0.0, smoothstep(0.12, 0.0, length(dp)), h.z > 0.92);
    return clamp(smudge * 0.7 + blotch * 0.6 + speck, 0.0, 1.0);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let dims = vec2<f32>(textureDimensions(tex));
    let aspect = dims.x / dims.y;
    let frame = u32(fx.time * 60.0);
    var uv = in.uv;
    // Rolling shutter: later rows see the scene displaced by the turn that happened during readout.
    let row = uv.y - 0.5;
    uv.x -= row * fx.yaw_rate * fx.rolling_shutter / 1.9;
    uv.y -= row * fx.pitch_rate * fx.rolling_shutter / 1.1;
    uv += (pcg3(vec3<u32>(frame, 3u, 1u)).xy - 0.5) * fx.explosion_shake * 0.012;

    // Fisheye: corner-normalised so the output never samples outside the overscanned render.
    let d = (uv - 0.5) * 2.0;
    let r2 = dot(d, d) * 0.5;
    let norm = 1.0 + fx.distortion + fx.edge_stretch;
    let sd = d * (1.0 + fx.distortion * r2 + fx.edge_stretch * r2 * r2) / norm;
    let ca = fx.ca * r2;
    let uv_r = (sd * (1.0 + ca)) * 0.5 + 0.5;
    let uv_g = sd * 0.5 + 0.5;
    let uv_b = (sd * (1.0 - ca)) * 0.5 + 0.5;

    // DiskBlur: vogel-disk blur, zero in the centre, plus flash ringing and explosion smear.
    let edge = smoothstep(fx.edge_blur_start, 1.0, r2);
    let blur_px = fx.edge_blur * edge + fx.flash_amount * 22.0 + fx.explosion_shake * 10.0;
    let boff = blur_px / dims;
    var col = vec3<f32>(0.0);
    const N = 11;
    for (var i = 0; i < N; i++) {
        let a = f32(i) * 2.39996;
        let o = vec2<f32>(cos(a), sin(a)) * sqrt((f32(i) + 0.5) / f32(N)) * boff;
        col += vec3<f32>(bil(uv_r + o).r, bil(uv_g + o).g, bil(uv_b + o).b);
    }
    col /= f32(N);

    // Centre sharpening (unsharp mask), faded toward the soft edges.
    let px = 1.3 / dims;
    let nb = (bil(uv_g + vec2<f32>(px.x, 0.0)) + bil(uv_g - vec2<f32>(px.x, 0.0))
            + bil(uv_g + vec2<f32>(0.0, px.y)) + bil(uv_g - vec2<f32>(0.0, px.y))) * 0.25;
    let sharp = fx.sharpen * (1.0 - smoothstep(0.0, 0.7, r2)) * (1.0 - clamp(blur_px * 0.3, 0.0, 1.0));
    col = max(col + (col - nb) * sharp, vec3<f32>(0.0));

    // Lens dirt: scatters light above the threshold (sun, lamps, muzzle flash).
    var scatter = vec3<f32>(0.0);
    let mx = vec2<i32>(dims) - 1;
    for (var k = 0; k < 8; k++) {
        let a = f32(k) * 0.7854;
        let dir = vec2<f32>(cos(a), sin(a) / aspect);
        scatter += max(textureLoad(tex, clamp(vec2<i32>((uv_g + dir * 0.10) * dims), vec2<i32>(0), mx), 0).rgb - fx.dirt_threshold, vec3<f32>(0.0));
        scatter += max(textureLoad(tex, clamp(vec2<i32>((uv_g + dir * 0.28) * dims), vec2<i32>(0), mx), 0).rgb - fx.dirt_threshold, vec3<f32>(0.0));
    }
    scatter = scatter / 16.0 * fx.dirt_gain * (1.0 + fx.dirt_boost * 6.0);
    col += scatter * dirt_mask(in.uv, aspect) * fx.dirt;

    // Flashbang whiteout.
    col = mix(col, vec3<f32>(6.0), smoothstep(0.0, 1.0, fx.flash_amount));

    // Exposure bias + sensor pumping (EV), then cool tint.
    col *= exp2(fx.exposure_bias + fx.pump) * vec3<f32>(fx.tint_r, fx.tint_g, fx.tint_b);

    // Vignette: dark, oval (wider than tall).
    let vd = vec2<f32>(d.x * fx.vignette_oval, d.y);
    col *= 1.0 - fx.vignette * smoothstep(fx.vignette_radius, 1.25, dot(vd, vd) * 0.5);

    // Damage: red edge darkening + blood splatter on the lens.
    if (fx.damage_amount > 0.001) {
        let e = smoothstep(0.15, 1.0, r2);
        let splat = smoothstep(0.45, 0.75, fbm(in.uv * vec2<f32>(aspect, 1.0) * 4.0 + 3.0));
        let m = clamp(fx.damage_amount * (e * (0.6 + 0.8 * splat) + 0.15 * splat), 0.0, 1.0);
        let l = dot(col, vec3<f32>(0.3, 0.59, 0.11));
        col = mix(col, vec3<f32>(0.5, 0.02, 0.01) * (0.3 + l), m * 0.85);
    }

    // Sensor grain (luma + chroma), animated per frame, heavier in shadows.
    let n = pcg3(vec3<u32>(vec2<u32>(in.position.xy), frame)) - 0.5;
    let l = dot(col, vec3<f32>(0.3, 0.59, 0.11));
    let w = (0.3 + 1.0 / (1.0 + l * 2.0)) * max(l, 0.02) * 1.5;
    col = max(col + (vec3<f32>(n.x) * fx.grain + n * fx.temporal_noise) * w, vec3<f32>(0.0));

    // Compression-ish banding in a gamma-ish space.
    let g = sqrt(col);
    let q = floor(g * 40.0 + 0.5) / 40.0;
    col = mix(col, q * q, fx.banding);
    return vec4<f32>(col, 1.0);
}
