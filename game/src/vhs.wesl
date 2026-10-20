// VHS background of W_ConnectionMenu / W_LoadingMenu (Image brush = MediaTexture Vhs_01_Tex playing Movies/Vhs/Vhs_01.bk2).
// ponytail: the movie is Bink 2 (no decoder here), so this is procedural analog static standing in for it; swap in the decoded
// frames when available. BackgroundBlur (BlurStrength 0..5, radius 165) is folded in by softening the noise.
struct Vhs {
    // x: time s, y: brightness (Image ColorAndOpacity: 1 death screen, 0.094 loading screen), z: blur strength 0..5, w: unused
    p: vec4<f32>,
}
@group(1) @binding(0) var<uniform> vhs: Vhs;

struct In {
    @location(0) uv: vec2<f32>,
    @location(4) @interpolate(flat) size: vec2<f32>,
}

fn hash(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2<f32>(127.1, 311.7));
    return fract(sin(h) * 43758.5453);
}

fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2<f32>(1.0, 0.0)), u.x), mix(hash(i + vec2<f32>(0.0, 1.0)), hash(i + vec2<f32>(1.0, 1.0)), u.x), u.y);
}

@fragment
fn fragment(in: In) -> @location(0) vec4<f32> {
    let t = vhs.p.x;
    let frame = floor(t * 29.97); // the movie's frame rate
    let lines = 480.0;
    let y = floor(in.uv.y * lines);
    // per-line horizontal jitter (tracking) and a slow rolling bright band
    let jit = (hash(vec2<f32>(y, frame)) - 0.5) * 0.004;
    let x = (in.uv.x + jit) * in.size.x / in.size.y * lines;
    let fine = hash(vec2<f32>(floor(x), y) + frame * 17.0);
    let coarse = noise(vec2<f32>(x * 0.12, y * 0.12) + frame * 3.1);
    let b = clamp(vhs.p.z / 5.0, 0.0, 1.0);
    var v = mix(fine, coarse, 0.35 + 0.5 * b);
    v = mix(v, 0.5, 0.35 * b); // blurred static loses contrast
    let band = smoothstep(0.0, 0.08, abs(fract(in.uv.y * 0.6 - t * 0.12) - 0.5) - 0.38);
    v = v * (0.55 + 0.25 * band);
    let line = 0.85 + 0.15 * sin(in.uv.y * lines * 3.14159);
    let c = vec3<f32>(v * line) * vhs.p.y;
    return vec4<f32>(c, 1.0);
}
