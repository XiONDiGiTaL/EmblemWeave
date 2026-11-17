// EmblemWeave: Game Boy. Four shades of the original DMG screen, ordered dithering between them, and the green LCD tint.
// The picture is quantised in luma, not per channel, so the palette stays the four greens instead of drifting.
#include "ReShade.fxh"

uniform int Shades <
    ui_type = "combo"; ui_items = "Four (DMG)\0Eight (Pocket)\0";
    ui_label = "Palette";
> = 0;

uniform float Dither <
    ui_type = "slider"; ui_min = 0.0; ui_max = 1.0; ui_step = 0.01;
    ui_label = "Dither";
    ui_tooltip = "0 gives hard bands, 1 the full 4x4 ordered pattern.";
> = 0.75;

uniform float PixelSize <
    ui_type = "slider"; ui_min = 1.0; ui_max = 8.0; ui_step = 1.0;
    ui_label = "Pixel size";
> = 2.0;

// The DMG's four greens, darkest first.
static const float3 kDmg[4] = {
    float3(0.059, 0.220, 0.059),
    float3(0.188, 0.384, 0.188),
    float3(0.545, 0.675, 0.059),
    float3(0.608, 0.737, 0.059),
};

// The Pocket's eight-step grey-green ramp.
static const float3 kPocket[8] = {
    float3(0.06, 0.06, 0.06), float3(0.19, 0.20, 0.19), float3(0.33, 0.34, 0.32), float3(0.47, 0.48, 0.45),
    float3(0.60, 0.62, 0.58), float3(0.72, 0.74, 0.70), float3(0.84, 0.86, 0.82), float3(0.92, 0.94, 0.90),
};

// 4x4 Bayer matrix, normalised to -0.5..0.5.
static const float kBayer[16] = {
     0.0,  8.0,  2.0, 10.0,
    12.0,  4.0, 14.0,  6.0,
     3.0, 11.0,  1.0,  9.0,
    15.0,  7.0, 13.0,  5.0,
};

float3 PS_EW_GameBoy(float4 pos : SV_Position, float2 uv : TEXCOORD) : SV_Target
{
    // Quantise the sample position so the dither pattern sits on the emulated pixels, not on the screen.
    const float2 res = ReShade::ScreenSize / PixelSize;
    float2 snapped = floor(uv * res) / res + 0.5 / res;

    float3 c = tex2D(ReShade::BackBuffer, snapped).rgb;
    float luma = dot(c, float3(0.2126, 0.7152, 0.0722));

    const int levels = Shades == 0 ? 4 : 8;
    const float steps = float(levels - 1);

    int2 p = int2(fmod(floor(uv * res), 4.0));
    const float threshold = (kBayer[p.y * 4 + p.x] / 16.0 - 0.5) * Dither;
    luma = saturate(luma + threshold / steps);

    const int idx = clamp(int(luma * steps + 0.5), 0, levels - 1);
    return Shades == 0 ? kDmg[idx] : kPocket[idx];
}

technique EW_GameBoy <
    ui_label = "EmblemWeave Game Boy";
    ui_tooltip = "Four-shade DMG palette with ordered dithering.";
>
{
    pass
    {
        VertexShader = PostProcessVS;
        PixelShader = PS_EW_GameBoy;
    }
}
