# Compiles the NVIDIA Image Scaling (NIS) upscaler compute shader with glslc, validates it with spirv-val, and writes
# src/gfx/extras/nis_shaders.inc (uint32 array). The .inc is committed, so building the runtime does not need the SDK.
# Run: python -I scripts/gen_nis_shaders.py   (exit 1 and no .inc written if the shader fails to compile or validate)
# glslc / spirv-val: the GLSLC and SPIRV_VAL environment variables, else the Vulkan SDK's, else the ones on PATH.
#
# NIS v1.0.3 (MIT, third_party/nvidia_nis, unmodified) reads its two filter tables from textures. This script compiles a temporary copy of
# NIS_Scaler.h whose texture macros are rewritten: the tables become constant arrays (parsed from NIS_Config.h), every colour fetch goes
# through nis_in() and the store through nis_out(), which compress scene-linear HDR input to [0, 1) the way the FSR 1 passes do.
import os, re, shutil, struct, subprocess, sys, tempfile

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
nis_dir = os.path.join(root, "third_party", "nvidia_nis")
out_path = os.path.join(root, "src", "gfx", "extras", "nis_shaders.inc")


def tool(env, sdk_name):
    if os.environ.get(env):
        return os.environ[env]
    sdk = os.path.join(r"C:\VulkanSDK\1.4.363.0\Bin", sdk_name + ".exe")
    if os.path.exists(sdk):
        return sdk
    found = shutil.which(sdk_name)
    if not found:
        sys.stderr.write("%s not found (set %s)\n" % (sdk_name, env))
        sys.exit(1)
    return found


def coef_table(text, name):
    m = re.search(r"constexpr float %s\[kPhaseCount\]\[kFilterSize\] = \{(.*?)\};" % name, text, re.S)
    if not m:
        sys.stderr.write("NIS_Config.h: %s not found\n" % name)
        sys.exit(1)
    rows = re.findall(r"\{([^{}]*)\}", m.group(1))
    if len(rows) != 64:
        sys.stderr.write("NIS_Config.h: %s has %d rows, expected 64\n" % (name, len(rows)))
        sys.exit(1)
    vecs = []
    for r in rows:
        v = [x.strip().rstrip("f") for x in r.split(",") if x.strip()]
        if len(v) != 8:
            sys.stderr.write("NIS_Config.h: %s row has %d values\n" % (name, len(v)))
            sys.exit(1)
        vecs.append("vec4(%s)" % ", ".join(v[0:4]))   # texel (0, phase)
        vecs.append("vec4(%s)" % ", ".join(v[4:8]))   # texel (1, phase)
    return "const vec4 %s[128] = vec4[128](\n    %s);\n" % ("kNis_" + name, ",\n    ".join(vecs))


# The macro lines of NIS_Scaler.h's GLSL block and what the temporary copy uses instead.
MACROS = {
    "#define NVTEX_LOAD(x, pos) texelFetch(sampler2D(x, samplerLinearClamp), pos, 0)": "#define NVTEX_LOAD(x, pos) nis_load_##x(pos)",
    "#define NVTEX_SAMPLE(x, sampler, pos) textureLod(sampler2D(x, sampler), pos, 0)": "#define NVTEX_SAMPLE(x, sampler, pos) nis_in(textureLod(sampler2D(x, sampler), pos, 0))",
    "#define NVTEX_SAMPLE_RED(x, sampler, pos) textureGather(sampler2D(x, sampler), pos, 0)": "#define NVTEX_SAMPLE_RED(x, sampler, pos) nis_in(textureGather(sampler2D(x, sampler), pos, 0))",
    "#define NVTEX_SAMPLE_GREEN(x, sampler, pos) textureGather(sampler2D(x, sampler), pos, 1)": "#define NVTEX_SAMPLE_GREEN(x, sampler, pos) nis_in(textureGather(sampler2D(x, sampler), pos, 1))",
    "#define NVTEX_SAMPLE_BLUE(x, sampler, pos) textureGather(sampler2D(x, sampler), pos, 2)": "#define NVTEX_SAMPLE_BLUE(x, sampler, pos) nis_in(textureGather(sampler2D(x, sampler), pos, 2))",
    "#define NVTEX_STORE(x, pos, v) imageStore(x, NVI2(pos), v)": "#define NVTEX_STORE(x, pos, v) imageStore(x, NVI2(pos), nis_out(v))",
}

MAIN = r"""#version 450
#extension GL_GOOGLE_include_directive : enable
// NVIDIA Image Scaling v1.0.3 scaler (upscale + sharpen), FP32, SDR path. Block 32 x 24, 256 threads (the SDK's defaults for NVIDIA).
#define NIS_GLSL 1
#define NIS_SCALER 1
#define NIS_HDR_MODE 0
#define NIS_BLOCK_WIDTH 32
#define NIS_BLOCK_HEIGHT 24
#define NIS_THREAD_GROUP_SIZE 256
#define NIS_USE_HALF_PRECISION 0
#define NIS_VIEWPORT_SUPPORT 0

layout(push_constant) uniform NisConst {
    float kDetectRatio;
    float kDetectThres;
    float kMinContrastRatio;
    float kRatioNorm;
    float kContrastBoost;
    float kEps;
    float kSharpStartY;
    float kSharpScaleY;
    float kSharpStrengthMin;
    float kSharpStrengthScale;
    float kSharpLimitMin;
    float kSharpLimitScale;
    float kScaleX;
    float kScaleY;
    float kDstNormX;
    float kDstNormY;
    float kSrcNormX;
    float kSrcNormY;
    uint kInputViewportOriginX;
    uint kInputViewportOriginY;
    uint kInputViewportWidth;
    uint kInputViewportHeight;
    uint kOutputViewportOriginX;
    uint kOutputViewportOriginY;
    uint kOutputViewportWidth;
    uint kOutputViewportHeight;
    float kPrescale;   // NISConfig::reserved0: > 0 = scene-linear HDR input, compressed c = k*x / (1 + k*x) on fetch and expanded on store
    float reserved1;
};

layout(set = 0, binding = 0) uniform sampler samplerLinearClamp;
layout(set = 0, binding = 1) uniform texture2D in_texture;
layout(set = 0, binding = 2, rgba16f) writeonly uniform image2D out_texture;

vec4 nis_in(vec4 x) {
    if (kPrescale <= 0.0) return x;
    x = max(x, vec4(0.0)) * kPrescale;
    return x / (vec4(1.0) + x);
}
vec4 nis_out(vec4 c) {
    if (kPrescale <= 0.0) return c;
    vec3 v = clamp(c.rgb, vec3(0.0), vec3(0.999));
    return vec4(v / (vec3(1.0) - v) / kPrescale, 1.0);
}

@COEF@
vec4 nis_load_coef_scaler(ivec2 p) { return kNis_coef_scale[p.y * 2 + p.x]; }
vec4 nis_load_coef_usm(ivec2 p) { return kNis_coef_usm[p.y * 2 + p.x]; }

#include "NIS_Scaler_rw.h"

layout(local_size_x = NIS_THREAD_GROUP_SIZE) in;
void main() {
    NVScaler(gl_WorkGroupID.xy, gl_LocalInvocationID.x);
}
"""

HEADER = """// GENERATED by scripts/gen_nis_shaders.py: do not edit
// SPIR-V of the NVIDIA Image Scaling v1.0.3 scaler (third_party/nvidia_nis, MIT), compiled with glslc -O --target-env=vulkan1.3 and checked
// with spirv-val --target-env vulkan1.3.
//
// kNisScalerComp: compute shader, local size 256 x 1 x 1. One workgroup per 32 x 24 output block: dispatch (ceil(out_w / 32), ceil(out_h / 24), 1).
//   Descriptors (set 0, compute stage): binding 0 SAMPLER (linear, clamp to edge), binding 1 SAMPLED_IMAGE (texture2D in_texture, the source),
//     binding 2 STORAGE_IMAGE (image2D out_texture, rgba16f, writeonly). The filter tables are constants in the shader.
//   Push constants: 112 bytes, VK_SHADER_STAGE_COMPUTE_BIT: the first 112 bytes of NISConfig (third_party/nvidia_nis/NIS_Config.h, filled by
//     NVScalerUpdateConfig), with reserved0 used as the pre-scale k: > 0 means scene-linear HDR input, compressed c = k*x / (1 + k*x) on every fetch
//     and expanded x = c / (1 - c) / k (c clamped to [0, 0.999]) on the store; 0 means a [0, 1] picture used as is.
//   NIS scales by 1x to 2x per side only (NVScalerUpdateConfig fails outside that range).
"""


def main():
    glslc = tool("GLSLC", "glslc")
    spirv_val = tool("SPIRV_VAL", "spirv-val")
    with open(os.path.join(nis_dir, "NIS_Config.h"), encoding="utf-8") as f:
        cfg = f.read()
    with open(os.path.join(nis_dir, "NIS_Scaler.h"), encoding="utf-8") as f:
        scaler = f.read()
    for old, new in MACROS.items():
        if scaler.count(old) != 1:
            sys.stderr.write("NIS_Scaler.h: macro line not found once: %s\n" % old)
            sys.exit(1)
        scaler = scaler.replace(old, new)
    src = MAIN.replace("@COEF@", coef_table(cfg, "coef_scale") + coef_table(cfg, "coef_usm"))
    with tempfile.TemporaryDirectory() as tmp:
        with open(os.path.join(tmp, "NIS_Scaler_rw.h"), "w", newline="\n") as f:
            f.write(scaler)
        comp = os.path.join(tmp, "nis_scaler.comp")
        with open(comp, "w", newline="\n") as f:
            f.write(src)
        res = subprocess.run([glslc, "-O", "--target-env=vulkan1.3", "-I", tmp, "-o", "-", comp], capture_output=True)
        if res.returncode != 0:
            sys.stderr.write("glslc failed:\n%s" % res.stderr.decode(errors="replace"))
            sys.exit(1)
        spv = res.stdout
        if len(spv) % 4:
            sys.stderr.write("SPIR-V size is not a multiple of 4\n")
            sys.exit(1)
        path = os.path.join(tmp, "nis.spv")
        with open(path, "wb") as f:
            f.write(spv)
        v = subprocess.run([spirv_val, "--target-env", "vulkan1.3", path], capture_output=True, text=True)
        if v.returncode != 0:
            sys.stderr.write("spirv-val failed:\n%s\nFAILED: nis_shaders.inc not written\n" % (v.stdout + v.stderr))
            sys.exit(1)
    words = struct.unpack("<%dI" % (len(spv) // 4), spv)
    lines = [HEADER, "\nstatic const uint32_t kNisScalerComp[] = {\n"]
    for i in range(0, len(words), 8):
        lines.append("    " + ", ".join("0x%08x" % w for w in words[i:i + 8]) + ",\n")
    lines.append("};\n")
    with open(out_path, "w", newline="\n") as f:
        f.write("".join(lines))
    print("kNisScalerComp %d words, spirv-val PASS\nwrote %s" % (len(words), out_path))


if __name__ == "__main__":
    main()
