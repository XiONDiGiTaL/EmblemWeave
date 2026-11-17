// EmblemWeave: Vivid. A small, predictable contrast and colour lift for Fire Emblem: Fortune's Weave.
// Saturation above neutral, luminance-weighted so skin tones do not turn orange; a gentle S-curve for contrast; the darkest
// values pushed down so night scenes and the UI's dark panels keep their depth instead of washing out.
#include "ReShade.fxh"

uniform float Saturation <
    ui_type = "slider"; ui_min = 0.0; ui_max = 2.0; ui_step = 0.01;
    ui_label = "Saturation";
    ui_tooltip = "1.0 leaves the picture alone; 1.15-1.25 is the usual 'vivid' amount.";
> = 1.18;

uniform float Contrast <
    ui_type = "slider"; ui_min = 0.5; ui_max = 2.0; ui_step = 0.01;
    ui_label = "Contrast";
> = 1.06;

uniform float BlackPoint <
    ui_type = "slider"; ui_min = -0.1; ui_max = 0.1; ui_step = 0.005;
    ui_label = "Black point";
    ui_tooltip = "Negative deepens the darkest values; the game's UI panels sit there.";
> = -0.015;

uniform float Lift <
    ui_type = "slider"; ui_min = -0.1; ui_max = 0.1; ui_step = 0.005;
    ui_label = "Lift";
> = 0.0;

float3 PS_EW_Vivid(float4 pos : SV_Position, float2 uv : TEXCOORD) : SV_Target
{
    float3 c = tex2D(ReShade::BackBuffer, uv).rgb;

    // Contrast around a 0.5 pivot, then the black point, then the lift: the order keeps the S-curve centred.
    c = (c - 0.5) * Contrast + 0.5;
    c = c * (1.0 - BlackPoint) + BlackPoint;
    c += Lift;

    // Saturation against the Rec.709 luma, mixed by the slider.
    const float luma = dot(c, float3(0.2126, 0.7152, 0.0722));
    c = lerp(luma.xxx, c, Saturation);

    return saturate(c);
}

technique EW_Vivid <
    ui_label = "EmblemWeave Vivid";
    ui_tooltip = "Slightly more colour and contrast, deeper blacks.";
>
{
    pass
    {
        VertexShader = PostProcessVS;
        PixelShader = PS_EW_Vivid;
    }
}
