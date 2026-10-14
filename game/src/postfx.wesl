// Bodycam camera look: one fullscreen pass, runs in linear HDR before tonemapping.
import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;

struct Fx {
    distortion: f32, distortion_density: f32, ca: f32, vignette: f32,
    aberration_fade_exponent: f32, edge_blur: f32, blur_fade_exponent: f32,
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
    // Off-viewport: the warp overshoots ~0.065 (height units) at mid-sides and ~0.13 in the corners; in-game the sides
    // stay (clamped, blurred) and only the corners go dark, so fade between the two.
    let out = max(max(-uv, uv - 1.0), vec2<f32>(0.0)) * vec2<f32>(dims.x / dims.y, 1.0);
    let fade = 1.0 - smoothstep(0.075, 0.15, length(out));
    return mix(mix(a, b, f.x), mix(c, d, f.x), f.y) * fade;
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

    // Lens distortion (M_PP_BodycamMaster): radial scale 1 + Intensity * ExponentialDensity(r^2). It also bends
    // the BodycamBorders lens-housing plane, which is rendered in the scene before this pass.
    // Material works in raw screen UV: r2 = |uv - 0.5|^2 (0.25 at mid-sides, 0.5 at corners).
    let dir = uv - 0.5;
    let r2 = dot(dir, dir);
    let k = 1.0 + fx.distortion * (1.0 - exp(-fx.distortion_density * r2));
    // Per-channel CA: Red/Cyan 3.5, Green/Magenta -1.6, Blue/Yellow -0.8, faded by r^AberrationFadeExponent.
    // Sign chosen so red survives past the frame edge (orange rim, as in-game).
    let caf = fx.ca * pow(2.0 * r2, fx.aberration_fade_exponent * 0.5);
    let uv_r = 0.5 + dir * k * (1.0 - 3.5 * caf);
    let uv_g = 0.5 + dir * k * (1.0 + 1.6 * caf);
    let uv_b = 0.5 + dir * k * (1.0 + 0.8 * caf);
    let r2_edge = dot((uv_g - 0.5) * 2.0, (uv_g - 0.5) * 2.0) * 0.5; // for later effects

    // MI_PP_DiskBlur: blur radius * r^BlurFadeExponent (r = 1 at the frame edge mid-sides).
    // r = 1 at mid-sides; capped at the full radius (uncapped x2 gave 40 px corner smears the user called tunnel vision).
    let redge = length((in.uv - 0.5) * 2.0);
    let blur_px = fx.edge_blur * (dims.y / 1080.0) * min(pow(redge, fx.blur_fade_exponent), 1.0)
        + fx.flash_amount * 22.0 + fx.explosion_shake * 10.0;
    let boff = blur_px / dims;
    var col = vec3<f32>(0.0);
    // Golden-angle disk, rotated per pixel: fixed taps ghosted into stair-steps at the 20 px corner radius.
    const N = 24;
    let rot = pcg3(vec3<u32>(vec2<u32>(in.position.xy), 9u)).x * 6.2832;
    for (var i = 0; i < N; i++) {
        let a = f32(i) * 2.39996 + rot;
        let o = vec2<f32>(cos(a), sin(a)) * sqrt((f32(i) + 0.5) / f32(N)) * boff;
        col += vec3<f32>(bil(uv_r + o).r, bil(uv_g + o).g, bil(uv_b + o).b); // UE clamps off-viewport samples
    }
    col /= f32(N);
    let ca = caf;

    // Centre sharpening (unsharp mask), faded toward the soft edges.
    let px = 1.3 / dims;
    let nb = (bil(uv_g + vec2<f32>(px.x, 0.0)) + bil(uv_g - vec2<f32>(px.x, 0.0))
            + bil(uv_g + vec2<f32>(0.0, px.y)) + bil(uv_g - vec2<f32>(0.0, px.y))) * 0.25;
    let sharp = fx.sharpen * (1.0 - clamp(blur_px * 0.3, 0.0, 1.0));
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

    // UE vignette (VignetteIntensity), natural cos^4-like falloff.
    let vv = dot((in.uv - 0.5) * vec2<f32>(aspect, 1.0), (in.uv - 0.5) * vec2<f32>(aspect, 1.0));
    col *= mix(1.0, 1.0 / ((1.0 + vv * 1.1) * (1.0 + vv * 1.1)), fx.vignette);

    // Damage: red edge darkening + blood splatter on the lens.
    if (fx.damage_amount > 0.001) {
        let e = smoothstep(0.15, 1.0, r2_edge);
        let splat = smoothstep(0.45, 0.75, fbm(in.uv * vec2<f32>(aspect, 1.0) * 4.0 + 3.0));
        let m = clamp(fx.damage_amount * (e * (0.6 + 0.8 * splat) + 0.15 * splat), 0.0, 1.0);
        let l = dot(col, vec3<f32>(0.3, 0.59, 0.11));
        col = mix(col, vec3<f32>(0.5, 0.02, 0.01) * (0.3 + l), m * 0.85);
    }

    // Lens housing: smooth circular rim centred on the view (|uv| in height units: 0.5 top/bottom, 0.89 mid-sides,
    // 1.02 corners), so only the corners go dark, curving around the centre. Replaces the BodycamBorders photo plane,
    // whose rough off-centre rim read as lumpy blobs.
    let hr = length((in.uv - 0.5) * vec2<f32>(aspect, 1.0));
    // Per-channel radius (blue goes dark first, red last) gives the warm orange fringe on the arc; fitted to the user's
    // in-game corner crop (R/G/B half-dark at |uv| ~0.94/0.925/0.905, black past ~0.98, long dim red tail).
    col *= vec3<f32>(1.0) - smoothstep(vec3<f32>(0.9, 0.87, 0.85), vec3<f32>(0.99, 0.96, 0.94), vec3<f32>(hr));

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
