// EmblemWeave: Retro CRT. Scanlines, an aperture-grille tint, a soft vignette and a little horizontal bleed, sized from the
// frame height so a 1080p window shows the same number of scanlines a 720p one does. Curvature is deliberately left out: the
// game's UI sits at the edges and bending text is a cost with no payoff. Self-contained: no ReShade.fxh.
texture2D BackBufferTex : COLOR;
sampler2D BackBuffer { Texture = BackBufferTex; };
static const float2 kScreenSize = float2(BUFFER_WIDTH, BUFFER_HEIGHT);
static const float2 kTexelSize = float2(BUFFER_RCP_WIDTH, BUFFER_RCP_HEIGHT);

void EW_PostVS(in uint id : SV_VertexID, out float4 pos : SV_Position, out float2 uv : TEXCOORD0)
{
    uv = float2((id << 1) & 2, id & 2);
    pos = float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

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

uniform float Glow <
    ui_type = "slider"; ui_min = 0.0; ui_max = 1.0; ui_step = 0.01;
    ui_label = "Glow";
    ui_tooltip = "A cheap three-tap bleed: bright areas smear sideways the way a CRT's phosphor does.";
> = 0.12;

float3 PS_EW_RetroCRT(float4 pos : SV_Position, float2 uv : TEXCOORD0) : SV_Target
{
    float3 c = tex2D(BackBuffer, uv).rgb;

    if (Glow > 0.0)
    {
        const float3 l = tex2D(BackBuffer, uv - float2(kTexelSize.x, 0.0)).rgb;
        const float3 r = tex2D(BackBuffer, uv + float2(kTexelSize.x, 0.0)).rgb;
        c += Glow * 0.25 * (l + r) * (l + r) * 0.5;
    }

    const float row = uv.y * ScanlineCount;
    const float line = 0.5 + 0.5 * cos(row * 6.2831853);
    c *= 1.0 - ScanlineStrength * (1.0 - line);

    const float gx = uv.x * kScreenSize.x;
    const float col = gx - floor(gx / 3.0) * 3.0;
    const float3 mask = col < 1.0 ? float3(1.0, 1.0 - MaskStrength * 0.5, 1.0 - MaskStrength)
                     : col < 2.0 ? float3(1.0 - MaskStrength, 1.0, 1.0 - MaskStrength)
                                 : float3(1.0 - MaskStrength, 1.0 - MaskStrength * 0.5, 1.0);
    c *= mask;

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
        VertexShader = EW_PostVS;
        PixelShader = PS_EW_RetroCRT;
    }
}
