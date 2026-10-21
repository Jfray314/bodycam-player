// VHS background of W_ConnectionMenu / W_LoadingMenu: Image brush = MediaTexture Vhs_01_Tex playing Movies/Vhs/Vhs_01.bk2.
// The whole movie (5711 frames, 190 s, 29.97 fps) decoded with NihAV (tools: see REWRITE.md) into a 64x90 grid of 128x72 luma
// cells (8192x6480, under the 8192 texture limit), looped. BackgroundBlurNormal (BlurStrength 0..5, radius 165) over it: a clamped 9-tap blur inside the cell.
struct Vhs {
    // x: time s, y: brightness (Image ColorAndOpacity: 1 death screen, 0.094 loading screen), z: blur strength 0..5, w: unused
    p: vec4<f32>,
}
@group(1) @binding(0) var<uniform> vhs: Vhs;
@group(1) @binding(1) var frames: texture_2d<f32>;
@group(1) @binding(2) var frames_s: sampler;

struct In {
    @location(0) uv: vec2<f32>,
}

const COLS: f32 = 64.0;
const ROWS: f32 = 90.0;
const FRAMES: f32 = 5711.0;
const CELL: vec2<f32> = vec2<f32>(128.0, 72.0);

@fragment
fn fragment(in: In) -> @location(0) vec4<f32> {
    let f = floor(vhs.p.x * 29.97) % FRAMES;
    let cell = vec2<f32>(f % COLS, floor(f / COLS));
    let atlas = CELL * vec2<f32>(COLS, ROWS);
    let r = vhs.p.z * 0.2; // blur radius in source texels (0.5 at the old 320-wide cells)
    var v = 0.0;
    for (var j = -1; j <= 1; j++) {
        for (var i = -1; i <= 1; i++) {
            let px = clamp(in.uv * CELL + vec2<f32>(f32(i), f32(j)) * r, vec2<f32>(0.5), CELL - 0.5);
            v += textureSample(frames, frames_s, (cell * CELL + px) / atlas).r;
        }
    }
    return vec4<f32>(vec3<f32>(v / 9.0 * vhs.p.y), 1.0);
}
