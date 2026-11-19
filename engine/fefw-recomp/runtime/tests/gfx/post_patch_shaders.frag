// SPDX-License-Identifier: GPL-3.0-or-later
// Authored stand-ins (no game data) for the post-effect SPIR-V patches, in the shape Ryujinx's translator gives the game's shaders: one
// std140 constant buffer of vec4s, read as AccessChain(cb, 0, vec, comp). generate_post_patch_shaders.py builds one module per define.
//   GT3 / GT2  the GT tone map's shape (spirv_hdr_tonemap_gt): the shoulder maximum P = c[2].x, three exp2, the BT.2020 -> BT.709 matrix,
//              then max(0, .) on all three output channels (GT3) or on two only (GT2, the shape keep_wide must refuse)
//   MB         a velocity scale c[0].y read once (spirv_push_scale_cb mode 0)
//   VIG        a falloff exp2(log2(x) * c[5].w) (mode 1)
//   PC         MB with a push constant of its own (refused)
//   TWO        two constant buffers (binding 22 and 23) both read at c[1].x (the binding filter)
#version 450
#ifdef TWO
layout(set = 0, binding = 22, std140) uniform fp_c3 { vec4 c[4096]; } cb3;
layout(set = 0, binding = 23, std140) uniform fp_c4 { vec4 c[4096]; } cb4;
layout(location = 0) in vec4 v_tc;
layout(location = 0) out vec4 o_color;
void main() { o_color = vec4(cb3.c[1].x, cb4.c[1].x, v_tc.z, 1.0); }
#else
layout(set = 0, binding = 22, std140) uniform fp_c3 { vec4 c[4096]; } cb;
layout(set = 2, binding = 128) uniform sampler2D u_scene;
layout(location = 0) in vec4 v_tc;
layout(location = 0) out vec4 o_color;
#ifdef PC
layout(push_constant) uniform Push { float k; } pc;
#endif

void main() {
#if defined(GT3) || defined(GT2)
    vec3 x = texture(u_scene, v_tc.xy).rgb;
    float P = cb.c[2].x;
    float r = P - exp2(-x.r), g = P - exp2(-x.g), b = P - exp2(-x.b);
    float o709r = 1.66049027 * r + -0.587639093 * g + -0.0728498 * b;
    float o709g = -0.124549903 * r + 1.13289988 * g + -0.0083494 * b;
    float o709b = -0.0181508 * r + -0.100578897 * g + 1.11872983 * b;
#ifdef GT3
    o_color = vec4(max(0.0, o709r), max(0.0, o709g), max(0.0, o709b), 1.0);
#else
    o_color = vec4(max(0.0, o709r), max(0.0, o709g), o709b, 1.0);
#endif
#elif defined(VIG)
    vec4 s = texture(u_scene, v_tc.xy);
    float v = exp2(log2(max(s.a, 1e-4)) * cb.c[5].w);
    o_color = vec4(s.rgb * v, s.a);
#else
    float s = cb.c[0].y;
    o_color = texture(u_scene, v_tc.xy + vec2(s, 0.0)) * s;
#ifdef PC
    o_color *= pc.k;
#endif
#endif
}
#endif
