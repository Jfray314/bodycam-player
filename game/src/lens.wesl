// Screen-space lens layers drawn over the post-processed frame as a UI material (textures sampled directly, no scene taps):
//  - blood droplets: UE Bodycam_Player.ScreenDroplets (UltraDynamicSky Screen_Droplets, WaterDroplets_Small: R height, A cell id), 'Drops Intensity'
//  - lens dirt: PostProcess/T_DirtMask_01 as a faint overlay (alpha = its luma)
struct Lens {
    // x: blood coverage 0..1, y: dirt strength, z: droplet tiling, w: unused
    p: vec4<f32>,
}
@group(1) @binding(0) var<uniform> lens: Lens;
@group(1) @binding(1) var drops: texture_2d<f32>;
@group(1) @binding(2) var drops_s: sampler;
@group(1) @binding(3) var dirt: texture_2d<f32>;
@group(1) @binding(4) var dirt_s: sampler;

struct In {
    @location(0) uv: vec2<f32>,
    @location(4) @interpolate(flat) size: vec2<f32>,
}

@fragment
fn fragment(in: In) -> @location(0) vec4<f32> {
    let aspect = in.size.x / in.size.y;
    let c = (in.uv - 0.5) * vec2<f32>(aspect, 1.0);
    let hr = length(c);
    // Same circular housing as postfx.wesl: nothing past the dark corners.
    let rim = 1.0 - smoothstep(0.915, 0.985, hr);

    // Dirt: scratches/dust of the real mask, source-over with colour = mask / luma, alpha = luma.
    let d = textureSample(dirt, dirt_s, in.uv).rgb;
    let dl = max(dot(d, vec3<f32>(0.3, 0.59, 0.11)), 1e-3);
    var rgb = min(d / dl, vec3<f32>(1.0));
    var a = clamp(dl * 2.0, 0.0, 1.0) * lens.p.y;

    // Blood droplets: tile the droplet texture over the screen; bigger drops (high R) show first, more of them as coverage grows.
    if (lens.p.x > 0.001) {
        let t = textureSample(drops, drops_s, in.uv * vec2<f32>(aspect, 1.0) * lens.p.z);
        let edge = mix(0.75, 1.0, smoothstep(0.1, 0.9, hr)); // Screen Center / Edge Strength
        let th = 1.0 - lens.p.x * 0.6 * edge + (t.a - 0.5) * 0.25;
        let m = smoothstep(th, th + 0.12, t.r);
        let n = normalize(vec3<f32>((t.g - 0.5) * 2.0, (t.b - 0.5) * 2.0, 0.6));
        let spec = pow(max(dot(n, normalize(vec3<f32>(-0.4, 0.5, 0.75))), 0.0), 24.0);
        let blood = vec3<f32>(0.32, 0.015, 0.012) + vec3<f32>(0.55, 0.3, 0.3) * spec * m;
        let ba = m * 0.9;
        let oa = ba + a * (1.0 - ba);
        rgb = (blood * ba + rgb * a * (1.0 - ba)) / max(oa, 1e-4);
        a = oa;
    }
    // UI blends in sRGB-encoded target space; colours above are authored as sRGB-ish already.
    return vec4<f32>(rgb, a * rim);
}
