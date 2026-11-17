// EmblemWeave: Vivid. A small, predictable contrast and colour lift for Fire Emblem: Fortune's Weave.
// Self-contained on purpose: no ReShade.fxh, because that file ships with the shader repository, and a copy of the game with
// only these effects beside it must still compile. ReShade supplies BUFFER_* and PostProcessVS through the header, so the
// back buffer, the full-screen triangle and the screen size are declared here instead.
texture2D BackBufferTex : COLOR;
sampler2D BackBuffer { Texture = BackBufferTex; };
static const float2 kScreenSize = float2(BUFFER_WIDTH, BUFFER_HEIGHT);

void EW_PostVS(in uint id : SV_VertexID, out float4 pos : SV_Position, out float2 uv : TEXCOORD0)
{
    uv = float2((id << 1) & 2, id & 2);
    pos = float4(uv * float2(2.0, -2.0) + float2(-1.0, 1.0), 0.0, 1.0);
}

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
    ui_type = "slider"; ui_min = -0.05; ui_max = 0.05; ui_step = 0.005;
    ui_label = "Black point";
    ui_tooltip = "Negative deepens the darkest values; the game's UI panels sit there. The same control lightens the picture when positive, which is what the Bright preset uses.";
> = -0.015;

float3 PS_EW_Vivid(float4 pos : SV_Position, float2 uv : TEXCOORD0) : SV_Target
{
    float3 c = tex2D(BackBuffer, uv).rgb;

    c = (c - 0.5) * Contrast + 0.5;
    c = c * (1.0 - BlackPoint) + BlackPoint;

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
        VertexShader = EW_PostVS;
        PixelShader = PS_EW_Vivid;
    }
}
