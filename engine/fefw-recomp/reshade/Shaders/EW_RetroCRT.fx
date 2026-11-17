// EmblemWeave: Retro CRT. Scanlines, a slight aperture-grille tint, a soft vignette and a little horizontal bleed, sized from
// the frame height so a 1080p window shows the same number of scanlines a 720p one does. Curvature is deliberately left out:
// the game's UI sits at the edges, and bending text is a cost with no payoff.
#include "ReShade.fxh"

uniform float ScanlineStrength <
    ui_type = "slider"; ui_min = 0.0; ui_max = 1.0; ui_step = 0.01;
    ui_label = "Scanlines";
> = 0.35;

uniform float ScanlineCount <
    ui_type = "slider"; ui_min = 180.0; ui_max = 1200.0; ui_step = 1.0;
    ui_label = "Scanlines (per screen height)";
> = 480.0;

uniform float MaskStrength <
    ui_type = "slider"; ui_min = 0.0; ui_max = 1.0; ui_step = 0.01;
    ui_label = "Aperture grille";
> = 0.12;

uniform float Vignette <
    ui_type = "slider"; ui_min = 0.0; ui_max = 1.0; ui_step = 0.01;
    ui_label = "Vignette";
> = 0.22;

uniform float Bloom <
    ui_type = "slider"; ui_min = 0.0; ui_max = 1.0; ui_step = 0.01;
    ui_label = "Glow";
    ui_tooltip = "A cheap three-tap bleed: bright areas smear sideways the way a CRT's phosphor does.";
> = 0.12;

float3 PS_EW_RetroCRT(float4 pos : SV_Position, float2 uv : TEXCOORD) : SV_Target
{
    float3 c = tex2D(ReShade::BackBuffer, uv).rgb;

    // Horizontal bleed from the neighbouring pixels, weighted by their brightness.
    if (Bloom > 0.0)
    {
        const float2 px = ReShade::PixelSize;
        const float3 l = tex2D(ReShade::BackBuffer, uv - float2(px.x, 0.0)).rgb;
        const float3 r = tex2D(ReShade::BackBuffer, uv + float2(px.x, 0.0)).rgb;
        c += Bloom * 0.25 * (l + r) * (l + r) * 0.5;
    }

    // Scanlines on the frame's own rows.
    const float row = uv.y * ScanlineCount;
    const float line = 0.5 + 0.5 * cos(row * 6.2831853);
    c *= 1.0 - ScanlineStrength * (1.0 - line);

    // Aperture grille: every third column slightly dimmer, which is what makes a CRT look like it has colour fringes.
    const float col = fmod(floor(uv.x * ReShade::ScreenSize.x), 3.0);
    const float3 mask = col < 1.0 ? float3(1.0, 1.0 - MaskStrength * 0.5, 1.0 - MaskStrength)
                     : col < 2.0 ? float3(1.0 - MaskStrength, 1.0, 1.0 - MaskStrength)
                                 : float3(1.0 - MaskStrength, 1.0 - MaskStrength * 0.5, 1.0);
    c *= mask;

    // Vignette, squared so the centre stays untouched.
    const float2 d = uv - 0.5;
    c *= saturate(1.0 - Vignette * dot(d, d) * 2.0);

    return saturate(c);
}

technique EW_RetroCRT <
    ui_label = "EmblemWeave Retro CRT";
    ui_tooltip = "Scanlines, aperture grille, glow and a vignette.";
>
{
    pass
    {
        VertexShader = PostProcessVS;
        PixelShader = PS_EW_RetroCRT;
    }
}
