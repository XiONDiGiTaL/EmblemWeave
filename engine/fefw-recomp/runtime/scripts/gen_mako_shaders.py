# Compiles the MAKO spatial scaler compute shader with glslang, validates it with spirv-val, and writes
# src/gfx/extras/mako_shaders.inc (uint32 array). The .inc is committed, so building the runtime does not need the SDK.
# Run: python -I scripts/gen_mako_shaders.py   (exit 1 and no .inc written if the shader fails to compile or validate)
# glslang / spirv-val: the GLSLANG and SPIRV_VAL environment variables, else the Vulkan SDK's, else PATH.
#
# The algorithm is adapted from MAKO's spatial_scaling.comp (GPL-3.0-or-later, Copyright (C) 2025-2026 Eugenio Segala and
# contributors): a clamped sharpened-cubic reconstruction with contrast-aware anti-ringing and bounded local sharpening in one
# output-resolution pass. The engine's version is FP32, takes the source as a combined image sampler and adds the same
# scene-linear HDR compression as FSR 1 and NIS.
import os, shutil, struct, subprocess, sys, tempfile

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
out_path = os.path.join(root, "src", "gfx", "extras", "mako_shaders.inc")


def tool(env, name):
    if os.environ.get(env):
        return os.environ[env]
    sdk = os.path.join(r"C:\VulkanSDK\1.4.363.0\Bin", name + ".exe")
    if os.path.exists(sdk):
        return sdk
    found = shutil.which(name)
    return found


MAKO_SCALE = r"""#version 450
// SPDX-License-Identifier: GPL-3.0-or-later
// Adapted from MAKO's spatial_scaling.comp (Copyright (C) 2025-2026 Eugenio Segala and contributors, GPL-3.0-or-later).
// One output-resolution pass: a clamped sharpened-cubic reconstruction (Catmull-Rom tension 0.8) with a nine-sample
// separable filter, contrast-aware anti-ringing, and a bounded local sharpening term. Scene-linear HDR input is compressed
// to c = k*x / (1 + k*x) on every fetch and expanded again on store, as the FSR 1 and NIS passes do.
layout(local_size_x = 8, local_size_y = 8, local_size_z = 1) in;

layout(set = 0, binding = 0) uniform sampler2D u_src;
layout(set = 0, binding = 1, rgba16f) writeonly uniform image2D u_dst;

layout(push_constant) uniform PushConstants {
    vec4 sizes;   // xy: source view size in texels; zw: output size in texels
    vec4 tuning;  // x: sharpness 0..1; y: pre-scale k (0 = no HDR compression); zw: whole source size in texels
} pc;

vec4 compress4(vec4 c) {
    if (pc.tuning.y <= 0.0) return c;
    c.rgb = max(c.rgb, vec3(0.0)) * pc.tuning.y;
    return vec4(c.rgb / (vec3(1.0) + c.rgb), c.a);
}

vec3 expand3(vec3 c) {
    if (pc.tuning.y <= 0.0) return c;
    c = clamp(c, vec3(0.0), vec3(0.999));
    return c / (vec3(1.0) - c) / pc.tuning.y;
}

vec2 source_uv(vec2 texel) {
    const vec2 one = vec2(1.0);
    texel = clamp(texel, vec2(0.0), pc.sizes.xy - one);
    return (texel + vec2(0.5)) / pc.tuning.zw;
}

vec4 filtered(vec2 texel) { return compress4(textureLod(u_src, source_uv(texel), 0.0)); }

vec4 texel_at(ivec2 p) {
    const ivec2 zero = ivec2(0);
    p = clamp(p, zero, ivec2(pc.sizes.xy) - ivec2(1));
    return compress4(texelFetch(u_src, p, 0));
}

vec4 cubic_weights(float t) {
    float t2 = t * t;
    float t3 = t2 * t;
    // Catmull-Rom uses a tension of 0.5. A higher tension establishes a crisper reconstruction before the separately
    // configurable sharpening term, while the clamp and anti-ringing logic bound halos.
    const float tension = 0.8;
    return vec4(tension * (-t + 2.0 * t2 - t3),
                1.0 + (tension - 3.0) * t2 + (2.0 - tension) * t3,
                tension * t + (3.0 - 2.0 * tension) * t2 + (tension - 2.0) * t3,
                tension * (-t2 + t3));
}

float luma(vec3 c) { return dot(c, vec3(0.2126, 0.7152, 0.0722)); }

void main() {
    ivec2 out_coord = ivec2(gl_GlobalInvocationID.xy);
    if (any(greaterThanEqual(out_coord, ivec2(pc.sizes.zw)))) return;

    vec2 source_position = (vec2(out_coord) + vec2(0.5)) * pc.sizes.xy / pc.sizes.zw - vec2(0.5);
    ivec2 base = ivec2(floor(source_position));
    vec2 fraction = fract(source_position);
    vec4 hw = cubic_weights(fraction.x);
    vec4 vw = cubic_weights(fraction.y);

    // Fold the two positive inner lobes into one filtered sample: the same separable 4x4 reconstruction with nine
    // filtered samples, no private array and no dynamic indexing.
    vec3 hsample = vec3(hw.x, hw.y + hw.z, hw.w);
    vec3 vsample = vec3(vw.x, vw.y + vw.z, vw.w);
    vec3 hcoord = vec3(float(base.x - 1), float(base.x) + hw.z / hsample.y, float(base.x + 2));
    vec3 vcoord = vec3(float(base.y - 1), float(base.y) + vw.z / vsample.y, float(base.y + 2));

    vec4 reconstructed = filtered(vec2(hcoord.x, vcoord.x)) * (hsample.x * vsample.x);
    reconstructed += filtered(vec2(hcoord.y, vcoord.x)) * (hsample.y * vsample.x);
    reconstructed += filtered(vec2(hcoord.z, vcoord.x)) * (hsample.z * vsample.x);
    reconstructed += filtered(vec2(hcoord.x, vcoord.y)) * (hsample.x * vsample.y);
    reconstructed += filtered(vec2(hcoord.y, vcoord.y)) * (hsample.y * vsample.y);
    reconstructed += filtered(vec2(hcoord.z, vcoord.y)) * (hsample.z * vsample.y);
    reconstructed += filtered(vec2(hcoord.x, vcoord.z)) * (hsample.x * vsample.z);
    reconstructed += filtered(vec2(hcoord.y, vcoord.z)) * (hsample.y * vsample.z);
    reconstructed += filtered(vec2(hcoord.z, vcoord.z)) * (hsample.z * vsample.z);

    ivec2 center0 = ivec2(clamp(base.x, 0, int(pc.sizes.x) - 1), clamp(base.y, 0, int(pc.sizes.y) - 1));
    ivec2 center1 = ivec2(clamp(base.x + 1, 0, int(pc.sizes.x) - 1), clamp(base.y + 1, 0, int(pc.sizes.y) - 1));
    vec4 c00 = texel_at(ivec2(center0.x, center0.y));
    vec4 c10 = texel_at(ivec2(center1.x, center0.y));
    vec4 c01 = texel_at(ivec2(center0.x, center1.y));
    vec4 c11 = texel_at(ivec2(center1.x, center1.y));
    vec4 bilinear = mix(mix(c00, c10, fraction.x), mix(c01, c11, fraction.x), fraction.y);
    vec4 local_min = min(min(c00, c10), min(c01, c11));
    vec4 local_max = max(max(c00, c10), max(c01, c11));
    reconstructed = clamp(reconstructed, local_min, local_max);

    float min_luma = min(min(luma(c00.rgb), luma(c10.rgb)), min(luma(c01.rgb), luma(c11.rgb)));
    float max_luma = max(max(luma(c00.rgb), luma(c10.rgb)), max(luma(c01.rgb), luma(c11.rgb)));
    float contrast = max_luma - min_luma;
    float ringing_risk = smoothstep(0.06, 0.30, contrast);
    vec3 base_color = mix(reconstructed.rgb, bilinear.rgb, ringing_risk * 0.22);

    vec3 local_average = 0.25 * (c00.rgb + c10.rgb + c01.rgb + c11.rgb);
    float horizontal_edge = abs(luma(c10.rgb) - luma(c00.rgb)) + abs(luma(c11.rgb) - luma(c01.rgb));
    float vertical_edge = abs(luma(c01.rgb) - luma(c00.rgb)) + abs(luma(c11.rgb) - luma(c10.rgb));
    float edge_confidence = abs(horizontal_edge - vertical_edge) / (horizontal_edge + vertical_edge + 0.0001);
    const float sharpening_baseline = 3.0;
    float sharpening_gain = sharpening_baseline * clamp(pc.tuning.x, 0.0, 1.0) *
        mix(0.18, 0.32, edge_confidence) * mix(1.0, 0.55, ringing_risk);
    vec3 sharpened = base_color + (base_color - local_average) * sharpening_gain;

    vec3 envelope = vec3(sharpening_baseline * (0.02 + contrast * 0.08));
    sharpened = clamp(sharpened, local_min.rgb - envelope, local_max.rgb + envelope);
    imageStore(u_dst, out_coord, vec4(expand3(sharpened), clamp(reconstructed.a, 0.0, 1.0)));
}
"""

HEADER = """// GENERATED by scripts/gen_mako_shaders.py: do not edit
// SPIR-V of the MAKO-derived spatial scaler (mako_spatial.cpp), compiled with glslang -V --target-env vulkan1.3 -Os and
// checked with spirv-val --target-env vulkan1.3.
//
// kMakoScaleComp: compute, local size 8 x 8 x 1, dispatch (ceil(out w / 8), ceil(out h / 8), 1).
//   Set 0: binding 0 COMBINED_IMAGE_SAMPLER (linear, clamp to edge; the source), binding 1 STORAGE_IMAGE rgba16f
//   (writeonly; the output). Push constants: 32 bytes, VK_SHADER_STAGE_COMPUTE_BIT:
//     offset 0  vec4 sizes   xy: source view size in texels; zw: output size in texels
//     offset 16 vec4 tuning  x: sharpness 0..1; y: pre-scale k (0 = no HDR compression); zw: whole source size in texels
//
// The algorithm is adapted from MAKO's spatial_scaling.comp (GPL-3.0-or-later, Copyright (C) 2025-2026 Eugenio Segala and
// contributors): one output-resolution pass with a clamped sharpened-cubic reconstruction, contrast-aware anti-ringing and
// bounded local sharpening. Scene-linear input is compressed on fetch and expanded on store.
"""


def compile_one(glslang, spirv_val, tmp):
    comp = os.path.join(tmp, "mako.comp")
    spv = os.path.join(tmp, "mako.spv")
    with open(comp, "w", newline="\n") as f:
        f.write(MAKO_SCALE)
    res = subprocess.run([glslang, "-V", "--target-env", "vulkan1.3", "-Os", "-o", spv, comp], capture_output=True, text=True)
    if res.returncode != 0:
        sys.stderr.write("glslang failed:\n%s%s" % (res.stdout, res.stderr))
        sys.exit(1)
    data = open(spv, "rb").read()
    if len(data) == 0 or len(data) % 4:
        sys.stderr.write("mako: bad SPIR-V size\n")
        sys.exit(1)
    if spirv_val:
        v = subprocess.run([spirv_val, "--target-env", "vulkan1.3", spv], capture_output=True, text=True)
        if v.returncode != 0:
            sys.stderr.write("spirv-val failed:\n%s\n" % (v.stdout + v.stderr))
            sys.exit(1)
    return [int.from_bytes(data[i:i + 4], "little") for i in range(0, len(data), 4)]


def main():
    glslang = tool("GLSLANG", "glslang")
    spirv_val = tool("SPIRV_VAL", "spirv-val")
    if not glslang:
        sys.stderr.write("glslang not found (set GLSLANG)\n")
        sys.exit(1)
    with tempfile.TemporaryDirectory() as tmp:
        words = compile_one(glslang, spirv_val, tmp)
    lines = [HEADER, "static const uint32_t kMakoScaleComp[] = {"]
    for i in range(0, len(words), 8):
        lines.append("    " + ", ".join("0x%08x" % w for w in words[i:i + 8]) + ",")
    lines.append("};")
    with open(out_path, "w", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
    print("wrote %s (%d words)%s" % (out_path, len(words), "" if spirv_val else "; spirv-val not found, not validated"))


if __name__ == "__main__":
    main()
