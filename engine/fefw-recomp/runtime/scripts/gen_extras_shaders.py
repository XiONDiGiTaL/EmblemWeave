# Compiles the output pass shaders (GLSL below) with glslc (Vulkan SDK), validates each with spirv-val, and writes
# src/gfx/extras/extras_shaders.inc (uint32 arrays). The .inc is committed, so building the runtime does not need the SDK.
# Run: python -I scripts/gen_extras_shaders.py   (exit 1 and no .inc written if any shader fails to compile or validate)
#
# The output pass encodes the game's final picture for the swapchain (sRGB, HDR10 or scRGB) and optionally upscales it with
# AMD FidelityFX FSR 1 (EASU then RCAS, MIT, headers in third_party/ffx_fsr1, the SDK v1.1.4 copy; see its README.txt).
# The layouts (push constants and descriptors) are documented in the header comment that this script writes into the .inc.
import os, shutil, struct, subprocess, sys, tempfile

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
glslc = r"C:\VulkanSDK\1.4.363.0\Bin\glslc.exe"
spirv_val = r"C:\VulkanSDK\1.4.363.0\Bin\spirv-val.exe"
fsr_inc = os.path.join(root, "third_party", "ffx_fsr1")
out_path = os.path.join(root, "src", "gfx", "extras", "extras_shaders.inc")

# Vertex shader: the same full-screen triangle as scripts/gen_present_shaders.py (VERT there).
OUT_VERT = r"""#version 450
// Full-screen triangle. v_dst is the destination position in [0,1]^2 (0,0 top left, as the image rows run).
layout(location = 0) out vec2 v_dst;
void main() {
    vec2 p = vec2(float((gl_VertexIndex << 1) & 2), float(gl_VertexIndex & 2));
    v_dst = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"""

# Output encode: present.frag of gen_present_shaders.py plus the fourth push constant p (the mode and the nit levels).
OUT_ENCODE_FRAG = r"""#version 450
layout(location = 0) in vec2 v_dst;
layout(location = 0) out vec4 o_color;
layout(set = 0, binding = 0) uniform sampler2D u_src;
layout(push_constant) uniform PushConstants {
    vec4 pic;    // the picture rectangle in destination UV: origin xy, size zw (0,0,1,1 when the picture fills the target)
    vec4 crop;   // the source crop in texels: origin xy, size zw
    vec4 src;    // xy: the whole source size in texels, z: 1 = flip the picture vertically
    vec4 p;      // x: mode (0 copy, 1 linear to sRGB, 2 linear to HDR10, 3 linear to scRGB), y: paper white nits, z: peak nits,
                 // w: 1 = apply the picture controls in adj
    vec4 adj;    // picture controls: x contrast, y saturation, z black level, w gamma (neutral 1, 1, 0, 1)
    vec4 g;      // x: the SDR display gamma for HDR output ([hdr] sdr_gamma; 0 = the sRGB curve, unchanged), y: the scene gain G of this
                 // frame (paper white / output white, 1 without [hdr] ui_nits), z: gamut expansion 0..1 ([hdr] gamut_expand; 0 = off),
                 // w: dither step = one code value of the target (1/1023 for 10-bit, 1/255 for 8-bit; 0 = no dither)
} pc;

// Picture controls on a display-encoded signal (0..1 in SDR; above 1 for HDR highlights): black level, contrast around mid grey, saturation
// around the Rec. 709 luma of the signal, then gamma. White (1.0) stays where it is for the black level and the gamma.
vec3 picture(vec3 c) {
    float b = pc.adj.z;
    c = b < 0.0 ? (c + b) / (1.0 + b) : b + c * (1.0 - b);
    c = (c - 0.5) * pc.adj.x + 0.5;
    float l = dot(c, vec3(0.2126, 0.7152, 0.0722));
    c = vec3(l) + (c - vec3(l)) * pc.adj.y;
    return pow(max(c, vec3(0.0)), vec3(1.0 / pc.adj.w));
}

// IEC 61966-2-1 sRGB OETF of clamp(x, 0, 1).
vec3 srgb_oetf(vec3 x) {
    x = clamp(x, 0.0, 1.0);
    vec3 lo = x * 12.92;
    vec3 hi = 1.055 * pow(x, vec3(1.0 / 2.4)) - 0.055;
    return vec3(x.r <= 0.0031308 ? lo.r : hi.r, x.g <= 0.0031308 ? lo.g : hi.g, x.b <= 0.0031308 ? lo.b : hi.b);
}

// SMPTE ST 2084 (PQ) inverse EOTF: nits in [0, 10000] to the 0..1 signal.
const float PQ_M1 = 2610.0 / 16384.0;
const float PQ_M2 = 2523.0 / 4096.0 * 128.0;
const float PQ_C1 = 3424.0 / 4096.0;
const float PQ_C2 = 2413.0 / 4096.0 * 32.0;
const float PQ_C3 = 2392.0 / 4096.0 * 32.0;
vec3 pq_encode(vec3 nits) {
    vec3 y = clamp(nits / 10000.0, 0.0, 1.0);
    vec3 ym = pow(y, vec3(PQ_M1));
    return pow((vec3(PQ_C1) + PQ_C2 * ym) / (vec3(1.0) + PQ_C3 * ym), vec3(PQ_M2));
}

// HDR output: the game's colour targets decode with the sRGB curve, but on the console its picture is shown by a TV, which displays the
// SDR signal on a power curve (about 2.2). Re-decode each channel's SDR range with that power so shadows sit where the console puts them
// (the sRGB curve lifts the darkest shades several times over). x / G is the game's own 0..1 signal; above 1 (the HDR highlights) it is
// left alone, and 1 maps to 1 so paper white does not move.
vec3 display_gamma(vec3 rgb) {
    float gam = pc.g.x, G = max(pc.g.y, 1e-4);
    if (gam <= 0.0) return rgb;
    vec3 x = abs(rgb) / G;   // a negative channel (a colour outside BT.709, [hdr] wide_gamut) is re-decoded by its size and keeps its sign
    vec3 d = sign(rgb) * G * pow(srgb_oetf(x), vec3(gam));
    return vec3(x.r <= 1.0 ? d.r : rgb.r, x.g <= 1.0 ? d.g : rgb.g, x.b <= 1.0 ? d.b : rgb.b);
}

// BT.709 -> BT.2020, optionally with gamut expansion: saturated colours are partly re-read as if their primaries were DCI-P3 (a wider
// gamut with the same D65 white), so they reach past BT.709; near-neutral colours (greys, skin, most UI) are left alone, and the
// BT.2020 luminance is kept so nothing gets brighter or darker. amount 0 is the exact conversion (the console's colours).
vec3 to_bt2020(vec3 rgb, float amount) {
    vec3 exact = vec3(dot(vec3(0.6274, 0.3293, 0.0433), rgb),
                      dot(vec3(0.0691, 0.9195, 0.0114), rgb),
                      dot(vec3(0.0164, 0.0880, 0.8956), rgb));
    if (amount <= 0.0) return exact;
    vec3 c = max(rgb, vec3(0.0));
    float mx = max(c.r, max(c.g, c.b)), mn = min(c.r, min(c.g, c.b));
    float sat = mx > 1e-6 ? (mx - mn) / mx : 0.0;
    float w = amount * smoothstep(0.2, 0.8, sat);
    if (w <= 0.0) return exact;
    vec3 wide = vec3(dot(vec3(0.753833, 0.198597, 0.047570), c),
                     dot(vec3(0.045744, 0.941777, 0.012479), c),
                     dot(vec3(-0.001210, 0.017601, 0.983608), c));
    const vec3 Y2020 = vec3(0.2627, 0.6780, 0.0593);
    float ye = dot(Y2020, exact), yw = dot(Y2020, wide);
    if (yw > 1e-6) wide *= ye / yw;
    return mix(exact, max(wide, vec3(0.0)), w);
}

// Triangular-PDF noise of +-1 code value from the pixel position (a fixed pattern, so a still picture does not shimmer): breaks the
// visible steps of a smooth gradient into fine grain once the signal is quantised to the target's 10 (or 8) bits.
vec3 dither(vec3 c, float step) {
    vec2 p = floor(gl_FragCoord.xy);
    float a = fract(sin(dot(p, vec2(12.9898, 78.233))) * 43758.5453);
    float b = fract(sin(dot(p, vec2(39.3468, 11.1353))) * 24634.6345);
    return c + (a + b - 1.0) * step;
}

void main() {
    vec2 local = (v_dst - pc.pic.xy) / pc.pic.zw;
    if (any(lessThan(local, vec2(0.0))) || any(greaterThan(local, vec2(1.0)))) {
        o_color = vec4(0.0, 0.0, 0.0, 1.0);   // letterbox bar (black in every mode)
        return;
    }
    if (pc.src.z > 0.5) local.y = 1.0 - local.y;   // flip: a window with the lower-left origin mode is presented upside down otherwise
    // texel-space position inside the crop; clamped to the crop's texel centres so a linear filter never reads outside it
    vec2 texel = pc.crop.xy + local * pc.crop.zw;
    texel = clamp(texel, pc.crop.xy + 0.5, pc.crop.xy + pc.crop.zw - 0.5);
    vec3 rgb = textureLod(u_src, texel / pc.src.xy, 0.0).rgb;

    int mode = int(pc.p.x + 0.5);
    bool adjust = pc.p.w > 0.5;
    if (mode >= 2) rgb = display_gamma(rgb);
    if (adjust && mode >= 2) rgb = pow(picture(pow(max(rgb, vec3(0.0)), vec3(1.0 / 2.2))), vec3(2.2));   // HDR: on a 2.2 power curve
    vec3 outc;
    if (mode == 1) {
        outc = srgb_oetf(rgb);
        if (adjust) outc = clamp(picture(outc), 0.0, 1.0);
    } else if (mode == 2) {
        // scene-linear BT.709 with 1.0 = paper white -> BT.2020 -> nits, limited to the peak, then PQ
        vec3 r2020 = to_bt2020(rgb, pc.g.z);
        vec3 nits = clamp(min(r2020 * pc.p.y, vec3(pc.p.z)), 0.0, 10000.0);
        outc = pq_encode(nits);
    } else if (mode == 3) {
        if (pc.g.z > 0.0) {   // expanded colours go past BT.709: back to scRGB's BT.709 primaries (negative components carry them)
            vec3 r2020 = to_bt2020(rgb, pc.g.z);
            rgb = vec3(dot(vec3(1.660491, -0.587641, -0.072850), r2020),
                       dot(vec3(-0.124550, 1.132900, -0.008349), r2020),
                       dot(vec3(-0.018151, -0.100579, 1.118730), r2020));
        }
        outc = min(rgb * pc.p.y, vec3(pc.p.z)) / 80.0;   // scRGB: 1.0 = 80 nits, limited to the peak like HDR10
    } else {
        outc = rgb;                  // mode 0 (and any other value): the texel unchanged
        if (adjust) outc = clamp(picture(outc), 0.0, 1.0);
    }
    if (pc.g.w > 0.0 && (mode == 1 || mode == 2)) outc = clamp(dither(outc, pc.g.w), 0.0, 1.0);
    o_color = vec4(outc, 1.0);
}
"""

# Fire Emblem Fortune's Weave tone map (ImageProcessing.bfsha aa98bfc2796de9bf), rewritten: the game's GT curve per channel on its BT.2020
# scene, then BT.2020 -> BT.709. Interface identical to the game's translated shader (set 0 binding 22: its constant buffer c3; set 2 binding
# 128: the scene; location 0 in/out), plus a 48-byte fragment push constant (the alpha-test layout's range). With the neutral push values
# (R = the peak ratio, exposure 1, contrast/shadows/highlights/saturation 1, highlight saturation 0) it computes what the HDR variant of the
# game's shader computes (spirv_hdr_tonemap_gt): P = max(c2.x, R), the same toe, linear section and shoulder.
# [hdr] sharpen: Fortune's Weave's sharpen pass (filter.bfsha, fs e14b86c9fc0181d1), term for term: a 5-tap cross with the centre weight
# (1 + k/3) and the neighbours -k/12, k = c1.x. The strength multiplier, the [hdr] ui_nits scene gain (this draw is the scene's last pass
# before the UI, where hdr_gain_shader would put it) and the compare split come from the push constant.
FEFW_SHARPEN_FRAG = r"""#version 450
layout(set = 0, binding = 22, std140) uniform fp_c3 { vec4 c[4096]; } cb;
layout(set = 2, binding = 128) uniform sampler2D u_src;
layout(location = 0) in vec4 v_tc;
layout(location = 0) out vec4 o_color;
layout(push_constant) uniform PushConstants {
    vec4 a;   // x: scene gain (1 = none), y: strength multiplier (1 = the game's), z: compare: > 0 = half the target's width in pixels (the left
              // half keeps the game's strength), 0 = off
} pc;

void main() {
    vec4 c0 = cb.c[0];
    float maxv = fma(0.5, -c0.y, c0.w);
    float maxu = fma(0.5, -c0.x, c0.z);
    float u = v_tc.x * c0.z, v = v_tc.y * c0.w;
    float cu = min(maxu, max(0.0, u)), cv = min(maxv, max(0.0, v));
    vec4 l = texture(u_src, vec2(min(maxu, max(0.0, u - c0.x)), cv));
    vec4 r = texture(u_src, vec2(min(maxu, max(0.0, u + c0.x)), cv));
    vec4 d = texture(u_src, vec2(cu, min(maxv, max(0.0, v + c0.y))));
    vec4 e = texture(u_src, vec2(cu, min(maxv, max(0.0, v - c0.y))));
    vec4 m = texture(u_src, vec2(u, v));
    float k = cb.c[1].x;
    bool game_half = gl_FragCoord.x < pc.a.z;   // the target's pixels, not the source's (the scene may be upscaled)
    if (!game_half) k *= pc.a.y;
    float wn = (-k + 2.5) + fma(k, 0.16665999591350555419921875, -0.5);
    wn = wn + fma(k, 2.0, -4.0);
    wn = wn + fma(k, -1.33329999446868896484375, 2.0);
    float wc = fma(k, 0.33329999446868896484375, 1.0) * 2.0;
    vec3 rgb = fma(m.rgb, vec3(wc), fma(vec3(wn), e.rgb, fma(vec3(wn), d.rgb, fma(r.rgb, vec3(wn), vec3(wn) * l.rgb)))) * 0.5;
    float al = fma(m.w, wc, fma(wn, e.w, fma(wn, d.w, fma(wn, l.w, fma(r.w, wn, 1.0))))) * 0.5;
    o_color = vec4(rgb * pc.a.x, al);
}
"""

FEFW_GT_GRADE_FRAG = r"""#version 450
layout(set = 0, binding = 22, std140) uniform fp_c3 { vec4 c[4096]; } cb;
layout(set = 2, binding = 128) uniform sampler2D u_scene;
layout(location = 0) in vec4 v_tc;
layout(location = 0) out vec4 o_color;
layout(push_constant) uniform PushConstants {
    vec4 a;   // x: peak ratio R (P = max(game P, R); <= 1: the game's P), y: exposure (multiplier), z: contrast, w: scene saturation
    vec4 b;   // x: shadows, y: highlights, z: highlight saturation 0..1, w: 1 = keep colours outside BT.709 (no max(0, .))
    vec4 c;   // x: compare: > 0 = half the target's width in pixels; the left half as the game draws it (no grading, no highlight
              // saturation; testing aid), 0 = off,
              // y: hue preservation 0..1 (the curve on luminance, the scene colour's ratios kept)
} pc;

const vec3 Y2020 = vec3(0.2627, 0.6780, 0.0593);
const float MID = 0.18;

// Scene grading on the BT.2020 scene colour before the curve (scene-linear, mid grey 0.18): exposure, then contrast, shadows and highlights
// as powers of luminance around mid grey (shadows below it, highlights above it; the colour keeps its ratios), then saturation around the
// luminance. Neutral values leave the colour exactly as it is.
vec3 grade(vec3 x) {
    x *= pc.a.y;
    float y = dot(x, Y2020);
    if (y > 1e-8 && (pc.a.z != 1.0 || pc.b.x != 1.0 || pc.b.y != 1.0)) {
        float n = pow(y / MID, pc.a.z);
        n = n < 1.0 ? pow(n, 2.0 - pc.b.x) : pow(n, pc.b.y);
        x *= MID * n / y;
    }
    if (pc.a.w != 1.0) {
        float l = dot(x, Y2020);
        x = max(vec3(l) + (x - vec3(l)) * pc.a.w, vec3(0.0));
    }
    return x;
}

// The game's curve for one channel (the shader's own terms): power toe below m, linear from m, exponential shoulder towards P from S0.
float gt(float x, float a, float m, float l, float c, float P, float scale) {
    float l0 = ((P - m) * l) / a;
    float S0 = m + l0;
    float S1 = P - (a * l0 + m);
    float C2 = -(P * a / S1) / P;
    float t = clamp(x / m, 0.0, 1.0);
    float sm = t * t * (3.0 - 2.0 * t);
    float w2 = (x >= S0) ? 1.0 : 0.0;
    float w1 = sm - w2;
    float w0 = 1.0 - sm;
    float T = m * exp2(log2(abs(x / m)) * c);
    float L = m + a * (x - m);
    float S = P - S1 * exp2((x - S0) * C2 * 1.4426950408889634);
    return (w0 * T + w1 * L + w2 * S) * scale;
}

void main() {
    vec2 uv = v_tc.xy;
    vec4 s = texture(u_scene, uv);
    vec4 c0 = cb.c[0], c2 = cb.c[2];
    float P = pc.a.x > 1.001 ? max(c2.x, pc.a.x) : c2.x;
    bool game_half = gl_FragCoord.x < pc.c.x;   // the target's pixels, not the scene's (it may be smaller when upscaled)
    vec3 x = game_half ? s.rgb : grade(s.rgb);
    vec3 t = vec3(gt(x.r, c0.x, c0.y, c0.z, c0.w, P, c2.y), gt(x.g, c0.x, c0.y, c0.z, c0.w, P, c2.y), gt(x.b, c0.x, c0.y, c0.z, c0.w, P, c2.y));
    // Hue preservation: the game's curve applied per channel shifts hues as channels compress unevenly (bright orange toward yellow, saturated
    // blue toward cyan-white). Blend toward the curve applied to luminance with the scene colour's own ratios, over the whole range; where a
    // channel would pass the curve's maximum, back off toward the per-channel result just enough to fit.
    float hue = game_half ? 0.0 : pc.c.y;
    if (hue > 0.0) {
        float yi = dot(x, Y2020);
        if (yi > 1e-8) {
            vec3 hp = x * (gt(yi, c0.x, c0.y, c0.z, c0.w, P, c2.y) / yi);
            float lim = P * c2.y, f = 1.0;
            for (int i = 0; i < 3; ++i)
                if (hp[i] > lim && hp[i] > t[i]) f = min(f, max(lim - t[i], 0.0) / (hp[i] - t[i]));
            t = mix(t, mix(t, hp, f), hue);
        }
    }
    // Highlight saturation: the per-channel curve turns bright colours toward white. Blend toward the scene colour's own ratios at the
    // curve's luminance (hue and saturation of the scene, brightness of the curve), only in the highlights (from 0.8 to 2.5 x paper white),
    // limited so no channel passes the curve's maximum.
    float hs = pc.b.z;
    if (game_half) hs = 0.0;
    if (hs > 0.0) {
        float yt = dot(t, Y2020), yi = dot(x, Y2020);
        float w = hs * smoothstep(0.8, 2.5, yt);
        if (w > 0.0 && yi > 1e-8) {
            // hp has the curve's luminance and the scene colour's ratios. Where a channel would pass the curve's maximum, keep as much of the
            // colour as fits at that same luminance (mix toward the curve's own result, which fits): the hottest cores turn toward yellow-white
            // rather than dimming.
            vec3 hp = x * (yt / yi);
            float lim = P * c2.y, s = 1.0;
            for (int i = 0; i < 3; ++i)
                if (hp[i] > lim && hp[i] > t[i]) s = min(s, max(lim - t[i], 0.0) / (hp[i] - t[i]));
            hp = mix(t, hp, s);
            t = mix(t, hp, w);
        }
    }
    vec3 o = vec3(dot(vec3(1.6604902744293213, -0.5876390933990479, -0.0728515014052391), t),
                  dot(vec3(-0.1245499029755592, 1.1328999996185303, -0.0083478996530175), t),
                  dot(vec3(-0.0181511007249355, -0.1005787029862404, 1.1187298297882080), t));
    if (pc.b.w < 0.5) o = max(o, vec3(0.0));
    o_color = vec4(o, s.a);
}
"""

# Overlay composite: the ImGui overlay (rendered into an RGBA8 UNORM image, premultiplied alpha, 1.0 = UI white) drawn on top of an HDR
# swapchain image. UI white maps to paper white; the colours are encoded like kOutEncodeFrag modes 2 and 3.
OVERLAY_COMPOSITE_FRAG = r"""#version 450
layout(location = 0) in vec2 v_dst;
layout(location = 0) out vec4 o_color;
layout(set = 0, binding = 0) uniform sampler2D u_ui;
layout(push_constant) uniform PushConstants {
    vec4 p;      // x: mode (2 HDR10, 3 scRGB), y: paper white nits, z: peak nits
} pc;

// IEC 61966-2-1 sRGB EOTF (encoded to linear) of clamp(x, 0, 1).
vec3 srgb_eotf(vec3 x) {
    x = clamp(x, 0.0, 1.0);
    vec3 lo = x / 12.92;
    vec3 hi = pow((x + 0.055) / 1.055, vec3(2.4));
    return vec3(x.r <= 0.04045 ? lo.r : hi.r, x.g <= 0.04045 ? lo.g : hi.g, x.b <= 0.04045 ? lo.b : hi.b);
}

// SMPTE ST 2084 (PQ) inverse EOTF: nits in [0, 10000] to the 0..1 signal.
const float PQ_M1 = 2610.0 / 16384.0;
const float PQ_M2 = 2523.0 / 4096.0 * 128.0;
const float PQ_C1 = 3424.0 / 4096.0;
const float PQ_C2 = 2413.0 / 4096.0 * 32.0;
const float PQ_C3 = 2392.0 / 4096.0 * 32.0;
vec3 pq_encode(vec3 nits) {
    vec3 y = clamp(nits / 10000.0, 0.0, 1.0);
    vec3 ym = pow(y, vec3(PQ_M1));
    return pow((vec3(PQ_C1) + PQ_C2 * ym) / (vec3(1.0) + PQ_C3 * ym), vec3(PQ_M2));
}

void main() {
    vec4 texel = texture(u_ui, v_dst);
    float a = texel.a;
    if (a <= 0.0) {
        o_color = vec4(0.0);   // nothing drawn here: the blend leaves the swapchain image unchanged
        return;
    }
    vec3 straight = clamp(texel.rgb / a, 0.0, 1.0);
    vec3 lin = srgb_eotf(straight);   // scene-linear, 1.0 = paper white
    int mode = int(pc.p.x + 0.5);
    vec3 outc;
    if (mode == 2) {
        // BT.709 to BT.2020 -> nits, limited to the peak, then PQ
        vec3 r2020 = vec3(dot(vec3(0.6274, 0.3293, 0.0433), lin),
                          dot(vec3(0.0691, 0.9195, 0.0114), lin),
                          dot(vec3(0.0164, 0.0880, 0.8956), lin));
        vec3 nits = clamp(min(r2020 * pc.p.y, vec3(pc.p.z)), 0.0, 10000.0);
        outc = pq_encode(nits);
    } else if (mode == 3) {
        outc = lin * pc.p.y / 80.0;   // scRGB: 1.0 = 80 nits, not clamped
    } else {
        outc = lin;                   // any other value: scene-linear, unencoded
    }
    o_color = vec4(outc, a);
}
"""

# The SDK headers are read as GPU GLSL with 32-bit maths (FFX_HALF 0). FSR_RCAS_DENOISE is deliberately not defined.
FSR_COMMON_DEFINES = """#extension GL_GOOGLE_include_directive : require
#define FFX_GPU 1
#define FFX_GLSL 1
#define FFX_HALF 0
"""

# FSR 1 EASU, driven the way the SDK's ffx_fsr1_easu.h drives it (64 invocations, 16x16 tile, four pixels each).
FSR_EASU_COMP = "#version 450\n" + FSR_COMMON_DEFINES + r"""#include "ffx_core.h"
#define FFX_FSR_EASU_FLOAT 1

layout(local_size_x = 64, local_size_y = 1, local_size_z = 1) in;
layout(set = 0, binding = 0) uniform sampler2D u_src;                       // linear filter, clamp to edge
layout(set = 0, binding = 1, rgba16f) writeonly uniform image2D u_dst;
layout(push_constant) uniform PushConstants {
    uvec4 con0;   // fsr1_easu_consts() output, in this order
    uvec4 con1;
    uvec4 con2;
    uvec4 con3;
    vec4 opt;     // x: 1 = scene-linear HDR input (compress/expand), 0 = [0,1] input; y, z: output width, height; w: pre-scale k of the compression (0 = 1)
} pc;

bool linear_in() { return pc.opt.x > 0.5; }

// Perceptual compression for scene-linear input: c = k*x / (1 + k*x) with k = opt.w (0 means 1), so FSR sees values in [0, 1).
float prescale() { return pc.opt.w > 0.0 ? pc.opt.w : 1.0; }

vec4 compress4(vec4 x) {
    x = max(x, vec4(0.0)) * prescale();
    return x / (vec4(1.0) + x);
}

// Undo the compression after EASU: x = c / (1 - c) / k, with c clamped to [0, 0.999].
vec3 expand3(vec3 c) {
    c = clamp(c, vec3(0.0), vec3(0.999));
    return c / (vec3(1.0) - c) / prescale();
}

// The fetch callbacks of the SDK's ffx_fsr1_callbacks_glsl.h, on this pass's bindings.
FfxFloat32x4 FsrEasuRF(FfxFloat32x2 p) { vec4 r = textureGather(u_src, p, 0); return linear_in() ? compress4(r) : r; }
FfxFloat32x4 FsrEasuGF(FfxFloat32x2 p) { vec4 r = textureGather(u_src, p, 1); return linear_in() ? compress4(r) : r; }
FfxFloat32x4 FsrEasuBF(FfxFloat32x2 p) { vec4 r = textureGather(u_src, p, 2); return linear_in() ? compress4(r) : r; }

#include "fsr1/ffx_fsr1.h"

void CurrFilter(uvec2 pos) {
    if (any(greaterThanEqual(pos, uvec2(pc.opt.yz)))) return;   // outside the output
    vec3 c;
    ffxFsrEasuFloat(c, pos, pc.con0, pc.con1, pc.con2, pc.con3);
    if (linear_in()) c = expand3(c);
    imageStore(u_dst, ivec2(pos), vec4(c, 1.0));
}

void main() {
    // Remap the 64 invocations to a PS-like 8x8 swizzle, then four pixels per invocation: a 16x16 tile.
    uvec2 gxy = ffxRemapForQuad(gl_LocalInvocationID.x) + uvec2(gl_WorkGroupID.x << 4u, gl_WorkGroupID.y << 4u);
    CurrFilter(gxy);
    gxy.x += 8u;
    CurrFilter(gxy);
    gxy.y += 8u;
    CurrFilter(gxy);
    gxy.x -= 8u;
    CurrFilter(gxy);
}
"""

# FSR 1 RCAS on the EASU output, no denoise. The input is read with texelFetch from a sampled image (no sampler).
FSR_RCAS_COMP = "#version 450\n" + "#extension GL_EXT_samplerless_texture_functions : require\n" + FSR_COMMON_DEFINES + r"""#include "ffx_core.h"
#define FSR_RCAS_F 1

layout(local_size_x = 64, local_size_y = 1, local_size_z = 1) in;
layout(set = 0, binding = 0) uniform texture2D u_src;                       // the EASU output, read with texelFetch
layout(set = 0, binding = 1, rgba16f) writeonly uniform image2D u_dst;
layout(push_constant) uniform PushConstants {
    uvec4 con;    // fsr1_rcas_consts() output
    vec4 opt;     // x: 1 = scene-linear HDR (compress/expand), 0 = [0,1]; y, z: output width, height; w: pre-scale k (0 = 1)
} pc;

bool linear_in() { return pc.opt.x > 0.5; }

float prescale() { return pc.opt.w > 0.0 ? pc.opt.w : 1.0; }

vec4 compress4(vec4 x) {
    x = max(x, vec4(0.0)) * prescale();
    return x / (vec4(1.0) + x);
}

vec3 expand3(vec3 c) {
    c = clamp(c, vec3(0.0), vec3(0.999));
    return c / (vec3(1.0) - c) / prescale();
}

// Input callbacks of the SDK's RCAS: reads are clamped to the output, so the 3x3 neighbourhood stays inside the image.
FfxFloat32x4 FsrRcasLoadF(FfxInt32x2 p) {
    vec4 t = texelFetch(u_src, clamp(p, ivec2(0), ivec2(pc.opt.yz) - 1), 0);
    return linear_in() ? compress4(t) : t;
}
void FsrRcasInputF(inout FfxFloat32 r, inout FfxFloat32 g, inout FfxFloat32 b) {}

#include "fsr1/ffx_fsr1.h"

void CurrFilter(uvec2 pos) {
    if (any(greaterThanEqual(pos, uvec2(pc.opt.yz)))) return;   // outside the output
    vec3 c;
    FsrRcasF(c.r, c.g, c.b, pos, pc.con);
    if (linear_in()) c = expand3(c);
    imageStore(u_dst, ivec2(pos), vec4(c, 1.0));
}

void main() {
    uvec2 gxy = ffxRemapForQuad(gl_LocalInvocationID.x) + uvec2(gl_WorkGroupID.x << 4u, gl_WorkGroupID.y << 4u);
    CurrFilter(gxy);
    gxy.x += 8u;
    CurrFilter(gxy);
    gxy.y += 8u;
    CurrFilter(gxy);
    gxy.x -= 8u;
    CurrFilter(gxy);
}
"""

# (C name, source file name, GLSL text, stage note for the header comment)
SHADERS = [
    ("kOutVert", "out_encode.vert", OUT_VERT),
    ("kOutEncodeFrag", "out_encode.frag", OUT_ENCODE_FRAG),
    ("kFefwGtGradeFrag", "fefw_gt_grade.frag", FEFW_GT_GRADE_FRAG),
    ("kFefwSharpenFrag", "fefw_sharpen.frag", FEFW_SHARPEN_FRAG),
    ("kOverlayCompositeFrag", "overlay_composite.frag", OVERLAY_COMPOSITE_FRAG),
    ("kFsrEasuComp", "fsr_easu.comp", FSR_EASU_COMP),
    ("kFsrRcasComp", "fsr_rcas.comp", FSR_RCAS_COMP),
]

HEADER = """// GENERATED by scripts/gen_extras_shaders.py: do not edit
// SPIR-V for the output pass, compiled with glslc -O --target-env=vulkan1.3 from the GLSL in scripts/gen_extras_shaders.py and
// checked with spirv-val --target-env vulkan1.3. FSR 1 headers: third_party/ffx_fsr1 (AMD FidelityFX SDK v1.1.4, MIT).
// All shaders use descriptor set 0. Push constants are std430 (vec4/uvec4 blocks, 16-byte aligned; offsets are in bytes).
//
// kOutVert: vertex shader, full-screen triangle, drawn with 3 vertices and no vertex input. No descriptors, no push
//   constants. Output location 0: vec2 v_dst (destination position in [0,1]^2, row 0 at the top). Used with kOutEncodeFrag and
//   kOverlayCompositeFrag.
//
// kOutEncodeFrag: fragment shader for a graphics pipeline that writes the swapchain (or the frame image).
//   Descriptors: set 0, binding 0, COMBINED_IMAGE_SAMPLER (sampler2D u_src), fragment stage. The source is a linear-filtered
//     game picture; the same sampler setup as the present pass.
//   Push constants: 80 bytes, VK_SHADER_STAGE_FRAGMENT_BIT.
//     offset 0   vec4 pic   picture rectangle in destination UV: x, y, w, h (0, 0, 1, 1 when the picture fills the target)
//     offset 16  vec4 crop  source crop in texels: x, y, w, h
//     offset 32  vec4 src   x, y: whole source size in texels; z: 1 = flip the picture vertically, 0 = no flip
//     offset 48  vec4 p     x: mode, read as int(x + 0.5):
//                             0 = copy, the texel unchanged (as present.frag)
//                             1 = scene-linear to sRGB (IEC 61966-2-1 OETF of clamp(rgb, 0, 1)), for an 8-bit UNORM swapchain
//                             2 = scene-linear to HDR10: 1.0 = paper white, BT.709 to BT.2020, nits = min(rgb * y, z) per
//                                 channel (negatives to 0), ST 2084 PQ signal, for an A2B10G10R10 swapchain
//                             3 = scene-linear to scRGB: min(rgb * y, z) / 80 (z = peak nits), for a float16 swapchain with
//                                 VK_COLOR_SPACE_EXTENDED_SRGB_LINEAR_EXT
//                             any other value = copy
//                           y: paper white in nits (used by modes 2 and 3); z: peak luminance in nits (mode 2 limit);
//                           w: 1 = apply the picture controls of adj, 0 = the plain encode above
//     offset 64  vec4 adj   picture controls, x contrast, y saturation, z black level b, w gamma g, on the display-encoded signal c
//                           (modes 0 and 1: the output signal, clamped to [0,1] after; modes 2 and 3: rgb^(1/2.2) before the encode,
//                           ^2.2 after): c = b < 0 ? (c + b) / (1 + b) : b + c (1 - b); c = (c - 0.5) x + 0.5;
//                           c = l + (c - l) y with l the Rec. 709 luma of c; c = max(c, 0)^(1/g)
//   Output: location 0, vec4, alpha always 1. Letterbox bars are (0, 0, 0, 1) in every mode.
//
// kOverlayCompositeFrag: fragment shader that draws the ImGui overlay on an HDR swapchain image. Used with kOutVert.
//   The overlay was rendered into an RGBA8 UNORM image (cleared to 0), so its texels are premultiplied: (rgb * a, a).
//   Descriptors: set 0, binding 0, COMBINED_IMAGE_SAMPLER (sampler2D u_ui), fragment stage. The image has the swapchain's size
//     and is read at v_dst with a linear sampler, clamp to edge (the texel centres, so no scaling).
//   Push constants: 16 bytes, VK_SHADER_STAGE_FRAGMENT_BIT.
//     offset 0   vec4 p     x: mode, read as int(x + 0.5): 2 = HDR10 (BT.709 to BT.2020, nits = min(rgb * y, z), ST 2084 PQ),
//                             3 = scRGB (rgb * y / 80, no clamp), any other value = scene-linear, unencoded.
//                           y: paper white nits (UI white, the sRGB value 1.0, maps to this); z: peak nits (mode 2 limit); w: unused.
//   Output: location 0, vec4 (encoded rgb of the un-premultiplied colour, a = the overlay's alpha). Texels with alpha 0 output
//     (0, 0, 0, 0). Blended with SRC_ALPHA / ONE_MINUS_SRC_ALPHA on the swapchain.
//
// kFsrEasuComp: compute shader, FSR 1 EASU (FP32 path). Local size 64 x 1 x 1. Each 16 x 16 output tile is one workgroup:
//   the 64 invocations each write four output pixels. Dispatch: ceil(out_w / 16) x ceil(out_h / 16) x 1. Pixels at or past
//   out_w / out_h are skipped inside the shader, so the dispatch may overshoot.
//   Descriptors: set 0, binding 0, COMBINED_IMAGE_SAMPLER (sampler2D u_src), compute stage. Use clamp-to-edge addressing.
//     textureGather returns the four texels around each point, so the filter setting does not affect the result.
//   set 0, binding 1, STORAGE_IMAGE (image2D u_dst, rgba16f, writeonly), compute stage. The output, out_w x out_h.
//   Push constants: 80 bytes, VK_SHADER_STAGE_COMPUTE_BIT.
//     offset 0   uvec4 con0   } the four words of fsr1_easu_consts() (src/gfx/extras/fsr1_consts.h), in this order
//     offset 16  uvec4 con1   }
//     offset 32  uvec4 con2   }
//     offset 48  uvec4 con3   }
//     offset 64  vec4 opt     x: perceptual flag. 0 = input is an sRGB-style [0,1] picture, used as is.
//                               1 = input is scene-linear HDR (1.0 = paper white, may exceed 1): each fetched texel is
//                               compressed c = x / (1 + x) (x clamped to >= 0) before EASU, and each result is expanded
//                               x = c / (1 - c) (c clamped to [0, 0.999]) after it. The output image then holds linear HDR.
//                             y: output width in pixels; z: output height in pixels; w: pre-scale k of the compression (0 = 1).
//
// kFsrRcasComp: compute shader, FSR 1 RCAS (FP32, sharpening without denoise). Local size 64 x 1 x 1, same tile and dispatch
//   as EASU. It reads the EASU output (the intermediate image, out_w x out_h) and writes the final image.
//   Descriptors: set 0, binding 0, SAMPLED_IMAGE (texture2D u_src), compute stage. Read with texelFetch, so no sampler is
//     bound for it. Reads outside the image are clamped to the edge.
//   set 0, binding 1, STORAGE_IMAGE (image2D u_dst, rgba16f, writeonly), compute stage.
//   Push constants: 32 bytes, VK_SHADER_STAGE_COMPUTE_BIT.
//     offset 0   uvec4 con  the four words of fsr1_rcas_consts() (src/gfx/extras/fsr1_consts.h)
//     offset 16  vec4 opt   x: perceptual flag, same meaning as EASU's (1 = scene-linear: compress before RCAS, expand after);
//                           y: output width; z: output height; w: pre-scale k (0 = 1).
"""


def hex_words(words, name):
    lines = ["static const uint32_t %s[] = {\n" % name]
    for i in range(0, len(words), 8):
        lines.append("    " + ", ".join("0x%08x" % w for w in words[i:i + 8]) + ",\n")
    lines.append("};\n")
    return "".join(lines)


def compile_glsl(src_name, text, tmp):
    src = os.path.join(tmp, src_name)
    with open(src, "w", newline="\n") as f:
        f.write(text)
    res = subprocess.run([glslc, "-O", "--target-env=vulkan1.3", "-I", fsr_inc, "-o", "-", src], capture_output=True)
    if res.returncode != 0:
        sys.stderr.write("glslc failed on %s:\n%s" % (src_name, res.stderr.decode(errors="replace")))
        sys.exit(1)
    if res.stderr:
        sys.stderr.write("glslc warnings on %s:\n%s" % (src_name, res.stderr.decode(errors="replace")))
    return res.stdout


def validate(spv, name, tmp):
    path = os.path.join(tmp, name + ".spv")
    with open(path, "wb") as f:
        f.write(spv)
    res = subprocess.run([spirv_val, "--target-env", "vulkan1.3", path], capture_output=True, text=True)
    return res.returncode == 0, (res.stdout + res.stderr).strip()


def main():
    body = [HEADER]
    rows = []
    failed = False
    with tempfile.TemporaryDirectory() as tmp:
        for cname, src_name, text in SHADERS:
            spv = compile_glsl(src_name, text, tmp)
            if len(spv) % 4:
                sys.stderr.write("%s: SPIR-V size is not a multiple of 4\n" % cname)
                sys.exit(1)
            ok, msg = validate(spv, cname, tmp)
            words = to_words(spv)
            rows.append((cname, len(words), "PASS" if ok else "FAIL"))
            if not ok:
                failed = True
                sys.stderr.write("spirv-val failed on %s:\n%s\n" % (cname, msg))
            body.append("\n" + hex_words(words, cname))
    print("%-16s %10s  %s" % ("name", "words", "spirv-val"))
    for name, n, verdict in rows:
        print("%-16s %10d  %s" % (name, n, verdict))
    if failed:
        print("FAILED: extras_shaders.inc not written")
        sys.exit(1)
    with open(out_path, "w", newline="\n") as f:
        f.write("".join(body))
    print("wrote %s" % out_path)


def to_words(spv):
    return struct.unpack("<%dI" % (len(spv) // 4), spv)


if __name__ == "__main__":
    main()
