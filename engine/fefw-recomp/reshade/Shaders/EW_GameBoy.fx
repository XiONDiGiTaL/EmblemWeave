// EmblemWeave: Game Boy. Four shades of the original DMG screen (or the Pocket's eight), ordered dithering between them.
// Self-contained: no ReShade.fxh, so it compiles on a bare ReShade install.
texture2D BackBufferTex : COLOR;
sampler2D BackBuffer { Texture = BackBufferTex; };
static const float2 kScreenSize = float2(BUFFER_WIDTH, BUFFER_HEIGHT);

void EW_PostVS(in uint id : SV_VertexID, out float4 pos : SV_Position, out float2 uv : TEXCOORD0)
{
    uv = float2((id << 1) & 2, id & 2);
    pos = float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

uniform int Palette <
    ui_type = "combo"; ui_items = "Four (DMG)\0Eight (Pocket)\0";
    ui_label = "Palette";
> = 0;

uniform float Dither <
    ui_type = "slider"; ui_min = 0.0; ui_max = 1.0; ui_step = 0.01;
    ui_label = "Dither";
    ui_tooltip = "0 gives hard bands, 1 the full 4x4 ordered pattern.";
> = 0.75;

uniform float BlockSize <
    ui_type = "slider"; ui_min = 1.0; ui_max = 8.0; ui_step = 1.0;
    ui_label = "Pixel size";
> = 2.0;

static const float3 kDmg[4] = {
    float3(0.059, 0.220, 0.059),
    float3(0.188, 0.384, 0.188),
    float3(0.545, 0.675, 0.059),
    float3(0.608, 0.737, 0.059),
};

static const float3 kPocket[8] = {
    float3(0.06, 0.06, 0.06), float3(0.19, 0.20, 0.19), float3(0.33, 0.34, 0.32), float3(0.47, 0.48, 0.45),
    float3(0.60, 0.62, 0.58), float3(0.72, 0.74, 0.70), float3(0.84, 0.86, 0.82), float3(0.92, 0.94, 0.90),
};

// 4x4 Bayer matrix, 0..15.
static const float kBayer[16] = {
     0.0,  8.0,  2.0, 10.0,
    12.0,  4.0, 14.0,  6.0,
     3.0, 11.0,  1.0,  9.0,
    15.0,  7.0, 13.0,  5.0,
};

float3 PS_EW_GameBoy(float4 pos : SV_Position, float2 uv : TEXCOORD0) : SV_Target
{
    // The emulated pixel grid, so the dither pattern sits on game pixels rather than on screen pixels.
    const float2 res = kScreenSize / BlockSize;
    const float2 px = floor(uv * res);
    const float2 snapped = (px + 0.5) / res;

    float luma = dot(tex2D(BackBuffer, snapped).rgb, float3(0.2126, 0.7152, 0.0722));

    const int levels = Palette == 0 ? 4 : 8;
    const float steps = float(levels - 1);

    // floor() twice rather than fmod: the effect compiler's fmod does not accept this overload here.
    const float2 cell = px - floor(px / 4.0) * 4.0;
    const int cell_i = clamp(int(cell.y) * 4 + int(cell.x), 0, 15);
    const float threshold = (kBayer[cell_i] / 16.0 - 0.5) * Dither;
    luma = saturate(luma + threshold / steps);

    const int idx = clamp(int(luma * steps + 0.5), 0, levels - 1);
    return Palette == 0 ? kDmg[idx] : kPocket[idx];
}

technique EW_GameBoy <
    ui_label = "EmblemWeave Game Boy";
    ui_tooltip = "Four-shade DMG palette with ordered dithering.";
>
{
    pass
    {
        VertexShader = EW_PostVS;
        PixelShader = PS_EW_GameBoy;
    }
}
