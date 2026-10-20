// Lens dirt drawn over the post-processed frame as a UI material (texture sampled directly, no scene taps):
// PostProcess/T_DirtMask_01 as a faint overlay (alpha = its luma). No lens blood: the real game shows none (see effects.rs).
struct Lens {
    // y: dirt strength (death-cut faded); x, z, w unused
    p: vec4<f32>,
}
@group(1) @binding(0) var<uniform> lens: Lens;
@group(1) @binding(1) var dirt: texture_2d<f32>;
@group(1) @binding(2) var dirt_s: sampler;

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
    let rgb = min(d / dl, vec3<f32>(1.0));
    let a = clamp(dl * 2.0, 0.0, 1.0) * lens.p.y;

    // UI blends in sRGB-encoded target space; colours above are authored as sRGB-ish already.
    return vec4<f32>(rgb, a * rim);
}
