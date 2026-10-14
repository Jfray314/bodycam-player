// Bodycam camera look for ReShade. Port of game/src/postfx.wesl (bodycam-player).
// Lens distortion, per-channel chromatic aberration, edge disk blur, sharpening, lens dirt, vignette,
// corner mask with colour fringe, grain, banding, and exposure pumping driven by frame brightness change.
// Not ported: rolling shutter, flash/damage/explosion hooks (they need game data ReShade cannot see).
#include "ReShade.fxh"

uniform float distortion <ui_type="slider"; ui_min=0.0; ui_max=1.0; ui_label="Lens distortion";> = 0.33;
uniform float distortion_density <ui_type="slider"; ui_min=0.1; ui_max=4.0; ui_label="Distortion density";> = 1.0;
uniform float ca <ui_type="slider"; ui_min=0.0; ui_max=0.02; ui_label="Chromatic aberration";> = 0.003;
uniform float aberration_fade_exponent <ui_type="slider"; ui_min=0.5; ui_max=6.0; ui_label="CA fade exponent";> = 2.0;
uniform float edge_blur <ui_type="slider"; ui_min=0.0; ui_max=40.0; ui_label="Edge blur (px at 1080p)";> = 20.0;
uniform float blur_fade_exponent <ui_type="slider"; ui_min=1.0; ui_max=16.0; ui_label="Edge blur fade exponent";> = 10.0;
uniform float sharpen <ui_type="slider"; ui_min=0.0; ui_max=2.0; ui_label="Centre sharpening";> = 0.0;
uniform float dirt <ui_type="slider"; ui_min=0.0; ui_max=1.0; ui_label="Lens dirt";> = 0.25;
uniform float dirt_threshold <ui_type="slider"; ui_min=0.0; ui_max=1.0; ui_label="Dirt brightness threshold";
    ui_tooltip="Game uses 4.0 in HDR. ReShade sees 0..1 display colour, so this is lower.";> = 0.8;
uniform float dirt_gain <ui_type="slider"; ui_min=0.0; ui_max=4.0; ui_label="Dirt gain";> = 1.5;
uniform float exposure_bias <ui_type="slider"; ui_min=-2.0; ui_max=2.0; ui_label="Exposure bias (EV)";> = 0.0;
uniform float vignette <ui_type="slider"; ui_min=0.0; ui_max=1.0; ui_label="Vignette";> = 0.4;
uniform float corner_mask <ui_type="slider"; ui_min=0.0; ui_max=1.0; ui_label="Corner mask strength";> = 1.0;
uniform float grain <ui_type="slider"; ui_min=0.0; ui_max=1.0; ui_label="Grain (luma)";> = 0.0;
uniform float temporal_noise <ui_type="slider"; ui_min=0.0; ui_max=1.0; ui_label="Chroma temporal noise";> = 0.0;
uniform float banding <ui_type="slider"; ui_min=0.0; ui_max=1.0; ui_label="Compression banding";> = 0.0;
uniform float pump_amount <ui_type="slider"; ui_min=0.0; ui_max=2.0; ui_label="Exposure pump strength";> = 1.0;
uniform float3 tint <ui_type="color"; ui_label="Colour tint";> = float3(1.0, 1.0, 1.0);
uniform int frame_count < source = "framecount"; >;
uniform float frametime < source = "frametime"; >;

// State texture (1x1): x = brightness, y = pump (EV), z = pump velocity, w = previous brightness.
texture BC_Lum { Width = 1; Height = 1; Format = RGBA16F; };
texture BC_Tmp { Width = 1; Height = 1; Format = RGBA16F; };
texture BC_State { Width = 1; Height = 1; Format = RGBA16F; };
sampler sLum { Texture = BC_Lum; MinFilter = POINT; MagFilter = POINT; };
sampler sTmp { Texture = BC_Tmp; MinFilter = POINT; MagFilter = POINT; };
sampler sState { Texture = BC_State; MinFilter = POINT; MagFilter = POINT; };

float3 pcg3(uint3 v)
{
    v = v * 1664525u + 1013904223u;
    v.x += v.y * v.z; v.y += v.z * v.x; v.z += v.x * v.y;
    v ^= v >> 16u;
    v.x += v.y * v.z; v.y += v.z * v.x; v.z += v.x * v.y;
    return float3(v >> 8u) / 16777216.0;
}

float3 bil(float2 uv)
{
    // Off-viewport: the sides stay (clamped, blurred), only the corners go dark.
    float2 o = max(max(-uv, uv - 1.0), 0.0) * float2(BUFFER_WIDTH / (float)BUFFER_HEIGHT, 1.0);
    float fade = 1.0 - smoothstep(0.075, 0.15, length(o));
    return tex2Dlod(ReShade::BackBuffer, float4(saturate(uv), 0, 0)).rgb * fade;
}

float vnoise(float2 p)
{
    int2 ii = (int2)floor(p) + 4096;
    uint2 i = (uint2)ii;
    float2 f = frac(p);
    float2 u = f * f * (3.0 - 2.0 * f);
    float a = pcg3(uint3(i, 1u)).x;
    float b = pcg3(uint3(i + uint2(1u, 0u), 1u)).x;
    float c = pcg3(uint3(i + uint2(0u, 1u), 1u)).x;
    float d = pcg3(uint3(i + uint2(1u, 1u), 1u)).x;
    return lerp(lerp(a, b, u.x), lerp(c, d, u.x), u.y);
}

float fbm(float2 p)
{
    float a = 0.5, s = 0.0;
    [unroll] for (int i = 0; i < 5; i++) { s += a * vnoise(p); p = p * 2.03 + 11.7; a *= 0.5; }
    return s;
}

float dirt_mask(float2 uv, float aspect)
{
    float2 p = uv * float2(aspect, 1.0);
    float smudge = smoothstep(0.52, 0.85, fbm(p * 3.0));
    float blotch = smoothstep(0.6, 0.9, fbm(p * 1.3 + 40.0));
    float2 cell = floor(p * 28.0);
    float3 h = pcg3(uint3((uint2)(int2)(cell + 512.0), 7u));
    float2 dp = frac(p * 28.0) - h.xy * 0.6 - 0.2;
    float speck = h.z > 0.92 ? smoothstep(0.12, 0.0, length(dp)) : 0.0;
    return saturate(smudge * 0.7 + blotch * 0.6 + speck);
}

// Pass 1: average brightness from a 16x16 grid of taps.
float4 PS_Lum(float4 vpos : SV_Position, float2 uv : TEXCOORD) : SV_Target
{
    float s = 0.0;
    [unroll] for (int y = 0; y < 16; y++)
    [unroll] for (int x = 0; x < 16; x++)
        s += dot(tex2Dlod(ReShade::BackBuffer, float4((x + 0.5) / 16.0, (y + 0.5) / 16.0, 0, 0)).rgb, float3(0.3, 0.59, 0.11));
    return float4(s / 256.0, 0, 0, 1);
}

// Pass 2: underdamped spring kicked by brightness changes (stand-in for the game's turn-rate kick).
float4 PS_Update(float4 vpos : SV_Position, float2 uv : TEXCOORD) : SV_Target
{
    float4 st = tex2Dfetch(sState, int2(0, 0));
    float cur = tex2Dfetch(sLum, int2(0, 0)).x;
    float dt = clamp(frametime * 0.001, 0.0001, 0.05);
    float kick = max(abs(cur - st.w) / dt - 1.0, 0.0) * 0.02 * pump_amount;
    float vel = st.z + (-18.0 * st.y - 3.0 * st.z - kick * 60.0) * dt;
    float pump = st.y + vel * dt;
    return float4(cur, clamp(pump, -1.0, 1.0), vel, cur);
}

float4 PS_Copy(float4 vpos : SV_Position, float2 uv : TEXCOORD) : SV_Target
{
    return tex2Dfetch(sTmp, int2(0, 0));
}

float4 PS_Bodycam(float4 vpos : SV_Position, float2 in_uv : TEXCOORD) : SV_Target
{
    float2 dims = float2(BUFFER_WIDTH, BUFFER_HEIGHT);
    float aspect = dims.x / dims.y;
    uint frame = (uint)frame_count;
    float pump = tex2Dfetch(sState, int2(0, 0)).y;

    float2 dir = in_uv - 0.5;
    float r2 = dot(dir, dir);
    float k = 1.0 + distortion * (1.0 - exp(-distortion_density * r2));
    float caf = ca * pow(2.0 * r2, aberration_fade_exponent * 0.5);
    float2 uv_r = 0.5 + dir * k * (1.0 - 3.5 * caf);
    float2 uv_g = 0.5 + dir * k * (1.0 + 1.6 * caf);
    float2 uv_b = 0.5 + dir * k * (1.0 + 0.8 * caf);

    float redge = length((in_uv - 0.5) * 2.0);
    float blur_px = edge_blur * (dims.y / 1080.0) * min(pow(redge, blur_fade_exponent), 1.0);
    float2 boff = blur_px / dims;
    float3 col = 0.0;
    float rot = pcg3(uint3((uint2)vpos.xy, 9u)).x * 6.2832;
    [unroll] for (int i = 0; i < 24; i++)
    {
        float a = (float)i * 2.39996 + rot;
        float2 o = float2(cos(a), sin(a)) * sqrt(((float)i + 0.5) / 24.0) * boff;
        col += float3(bil(uv_r + o).r, bil(uv_g + o).g, bil(uv_b + o).b);
    }
    col /= 24.0;

    float2 px = 1.3 / dims;
    float3 nb = (bil(uv_g + float2(px.x, 0)) + bil(uv_g - float2(px.x, 0))
               + bil(uv_g + float2(0, px.y)) + bil(uv_g - float2(0, px.y))) * 0.25;
    float sharp = sharpen * (1.0 - saturate(blur_px * 0.3));
    col = max(col + (col - nb) * sharp, 0.0);

    float3 scatter = 0.0;
    [unroll] for (int j = 0; j < 8; j++)
    {
        float a = (float)j * 0.7854;
        float2 d = float2(cos(a), sin(a) / aspect);
        scatter += max(tex2Dlod(ReShade::BackBuffer, float4(saturate(uv_g + d * 0.10), 0, 0)).rgb - dirt_threshold, 0.0);
        scatter += max(tex2Dlod(ReShade::BackBuffer, float4(saturate(uv_g + d * 0.28), 0, 0)).rgb - dirt_threshold, 0.0);
    }
    scatter = scatter / 16.0 * dirt_gain;
    col += scatter * dirt_mask(in_uv, aspect) * dirt;

    col *= exp2(exposure_bias + pump) * tint;

    float2 vd = (in_uv - 0.5) * float2(aspect, 1.0);
    float vv = dot(vd, vd);
    float vt = 1.0 + vv * 1.1;
    col *= lerp(1.0, 1.0 / (vt * vt), vignette);

    float hr = length(vd);
    float3 cm = smoothstep(float3(0.935, 0.915, 0.9), float3(1.0, 0.985, 0.97), hr.xxx);
    col *= 1.0 - cm * corner_mask;

    float3 n = pcg3(uint3((uint2)vpos.xy, frame)) - 0.5;
    float l = dot(col, float3(0.3, 0.59, 0.11));
    float w = (0.3 + 1.0 / (1.0 + l * 2.0)) * max(l, 0.02) * 1.5;
    col = max(col + (n.xxx * grain + n * temporal_noise) * w, 0.0);

    float3 g = sqrt(col);
    float3 q = floor(g * 40.0 + 0.5) / 40.0;
    col = lerp(col, q * q, banding);
    return float4(col, 1.0);
}

technique Bodycam
{
    pass Lum { VertexShader = PostProcessVS; PixelShader = PS_Lum; RenderTarget = BC_Lum; }
    pass Update { VertexShader = PostProcessVS; PixelShader = PS_Update; RenderTarget = BC_Tmp; }
    pass Copy { VertexShader = PostProcessVS; PixelShader = PS_Copy; RenderTarget = BC_State; }
    pass Look { VertexShader = PostProcessVS; PixelShader = PS_Bodycam; }
}
