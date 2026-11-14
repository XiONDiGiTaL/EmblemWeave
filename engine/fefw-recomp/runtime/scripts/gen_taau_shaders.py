# Compiles the compute shaders of the NVN2 DLSS host upscaler (src/gfx/extras/taau.h) with glslang and writes src/gfx/extras/taau_shaders.inc
# (uint32 arrays). The .inc is committed, so building the runtime does not need a shader compiler.
# Run: python -I scripts/gen_taau_shaders.py   (exit 1 and no .inc written if a shader fails to compile or validate)
# glslang: the GLSLANG environment variable, else glslang / glslangValidator on PATH (scoop's glslang works). spirv-val (SPIRV_VAL, the Vulkan
# SDK's or PATH) is run when it is found.
import os, shutil, subprocess, sys, tempfile

root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
out_path = os.path.join(root, "src", "gfx", "extras", "taau_shaders.inc")


def find_glslang():
    if os.environ.get("GLSLANG"):
        return os.environ["GLSLANG"]
    for name in ("glslang", "glslangValidator"):
        found = shutil.which(name)
        if found:
            return found
    sys.stderr.write("glslang not found (set GLSLANG)\n")
    sys.exit(1)


def find_spirv_val():
    if os.environ.get("SPIRV_VAL"):
        return os.environ["SPIRV_VAL"]
    sdk = os.path.join(r"C:\VulkanSDK\1.4.363.0\Bin", "spirv-val.exe")
    if os.path.exists(sdk):
        return sdk
    return shutil.which("spirv-val")


COMMON = r"""
vec3 safe_color(vec3 c) { return clamp(mix(c, vec3(0.0), isnan(c)), vec3(0.0), vec3(65000.0)); }
"""

# Temporal upscale of one frame. The input is jittered: the centre of texel t of the colour region shows the scene at t + 0.5 - jitter
# (texels, +y down). Motion vectors give the previous position minus the current one; times mv_scale (plus mv_offset) they are in output UV
# units. The history keeps linear colour and, in alpha, the sample weight it has accumulated (at most max_weight).
TAAU = r"""
#version 450
layout(local_size_x = 8, local_size_y = 8) in;

layout(set = 0, binding = 0) uniform sampler2D u_color;
layout(set = 0, binding = 1) uniform sampler2D u_depth;
layout(set = 0, binding = 2) uniform sampler2D u_motion;
layout(set = 0, binding = 3) uniform sampler2D u_mask;
layout(set = 0, binding = 4) uniform sampler2D u_hist;
layout(set = 0, binding = 5, rgba16f) writeonly uniform image2D o_hist;

layout(push_constant) uniform Push {
    vec2 in_size;
    vec2 in_base;
    vec2 out_size;
    vec2 jitter;
    vec2 mv_scale;
    vec2 mv_offset;
    vec2 depth_base;
    vec2 depth_ratio;
    vec2 mv_base;
    vec2 mv_ratio;
    vec2 mask_base;
    vec2 mask_ratio;
    uint flags;
    float max_weight;
} pc;

const uint kReset = 1u, kDepthInverted = 2u, kMotionOut = 4u, kMask = 8u, kSpatial = 16u;
""" + COMMON + r"""
float maxc(vec3 c) { return max(max(c.r, c.g), c.b); }
// Reversible tone map around k, the neighbourhood's mean luma: the filter, the box and the blend do not depend on the scene's exposure
// (Fortune's Weave hands over pre-exposure HDR, mostly around 0.001 with highlights of 10 and more).
vec3 tonemap(vec3 c, float k) { return c / (k + maxc(c)); }
vec3 untonemap(vec3 c, float k) { return k * c / max(1.0 - maxc(c), 1e-6); }
vec3 ycocg(vec3 c) { return vec3(dot(c, vec3(0.25, 0.5, 0.25)), dot(c, vec3(0.5, 0.0, -0.5)), dot(c, vec3(-0.25, 0.5, -0.25))); }
vec3 rgb(vec3 y) { return vec3(y.x + y.y - y.z, y.x + y.z, y.x - y.y - y.z); }

ivec2 region(ivec2 t) { return clamp(t, ivec2(0), ivec2(pc.in_size) - 1); }
vec3 color_at(ivec2 t) { return safe_color(texelFetch(u_color, ivec2(pc.in_base) + region(t), 0).rgb); }
ivec2 aux(vec2 base, vec2 ratio, ivec2 t) { return ivec2(base + floor((vec2(t) + 0.5) * ratio)); }

// Catmull-Rom of the history in five bilinear taps.
vec3 history(vec2 uv) {
    vec2 pos = uv * pc.out_size;
    vec2 tc = floor(pos - 0.5) + 0.5;
    vec2 f = pos - tc;
    vec2 w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    vec2 w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    vec2 w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    vec2 w3 = f * f * (-0.5 + 0.5 * f);
    vec2 w12 = w1 + w2;
    vec2 t0 = (tc - 1.0) / pc.out_size, t3 = (tc + 2.0) / pc.out_size, t12 = (tc + w2 / w12) / pc.out_size;
    vec3 r = textureLod(u_hist, vec2(t12.x, t0.y), 0.0).rgb * (w12.x * w0.y);
    r += textureLod(u_hist, vec2(t0.x, t12.y), 0.0).rgb * (w0.x * w12.y);
    r += textureLod(u_hist, t12, 0.0).rgb * (w12.x * w12.y);
    r += textureLod(u_hist, vec2(t3.x, t12.y), 0.0).rgb * (w3.x * w12.y);
    r += textureLod(u_hist, vec2(t12.x, t3.y), 0.0).rgb * (w12.x * w3.y);
    float ws = w12.x * w0.y + w0.x * w12.y + w12.x * w12.y + w3.x * w12.y + w12.x * w3.y;
    return safe_color(r / ws);
}

vec3 clip_box(vec3 lo, vec3 hi, vec3 h) {
    vec3 c = 0.5 * (hi + lo), e = 0.5 * (hi - lo) + 1e-5;
    vec3 v = h - c;
    vec3 a = abs(v / e);
    float m = max(a.x, max(a.y, a.z));
    return m > 1.0 ? c + v / m : h;
}

void main() {
    ivec2 o = ivec2(gl_GlobalInvocationID.xy);
    if (o.x >= int(pc.out_size.x) || o.y >= int(pc.out_size.y)) return;
    vec2 uv = (vec2(o) + 0.5) / pc.out_size;
    vec2 q = uv * pc.in_size + pc.jitter;   // the pixel centre's scene point in the jittered input
    if ((pc.flags & kSpatial) != 0u) {
        vec2 b = q - 0.5;
        ivec2 i = ivec2(floor(b));
        vec2 f = b - vec2(i);
        vec3 c = mix(mix(color_at(i), color_at(i + ivec2(1, 0)), f.x), mix(color_at(i + ivec2(0, 1)), color_at(i + ivec2(1, 1)), f.x), f.y);
        imageStore(o_hist, o, vec4(c, 0.0));
        return;
    }
    ivec2 ic = ivec2(floor(q));
    vec2 kscale = pc.out_size / pc.in_size;
    bool inv = (pc.flags & kDepthInverted) != 0u;
    vec3 lin[9];
    float k = 0.0;
    for (int i = 0; i < 9; ++i) {
        lin[i] = color_at(ic + ivec2(i % 3 - 1, i / 3 - 1));
        k += dot(lin[i], vec3(0.25, 0.5, 0.25));
    }
    k = max(k / 9.0, 1e-8);
    vec3 sum = vec3(0.0), m1 = vec3(0.0), m2 = vec3(0.0), bmin = vec3(1e9), bmax = vec3(-1e9);
    ivec2 best = region(ic);   // the nearest-depth texel, the centre one on ties
    float best_z = texelFetch(u_depth, aux(pc.depth_base, pc.depth_ratio, best), 0).r;
    float wsum = 0.0, wmax = 0.0;
    for (int dy = -1; dy <= 1; ++dy) {
        for (int dx = -1; dx <= 1; ++dx) {
            ivec2 t = ic + ivec2(dx, dy);
            ivec2 tc = region(t);
            vec3 c = tonemap(lin[(dy + 1) * 3 + dx + 1], k);
            vec2 d = (vec2(t) + 0.5 - q) * kscale;   // sample minus pixel centre, output pixels
            float w = exp(-2.0 * dot(d, d));
            sum += c * w;
            wsum += w;
            wmax = max(wmax, w);
            vec3 y = ycocg(c);
            m1 += y;
            m2 += y * y;
            bmin = min(bmin, y);
            bmax = max(bmax, y);
            float z = texelFetch(u_depth, aux(pc.depth_base, pc.depth_ratio, tc), 0).r;
            if (inv ? z > best_z : z < best_z) {
                best_z = z;
                best = tc;
            }
        }
    }
    vec3 cur = sum / max(wsum, 1e-6);
    vec3 mean = m1 / 9.0;
    vec3 sd = sqrt(max(m2 / 9.0 - mean * mean, vec3(0.0)));
    vec3 lo = max(bmin, mean - 1.25 * sd), hi = min(bmax, mean + 1.25 * sd);

    ivec2 mt = (pc.flags & kMotionOut) != 0u ? ivec2(pc.mv_base + floor((vec2(o) + 0.5) * pc.mv_ratio)) : aux(pc.mv_base, pc.mv_ratio, best);
    vec2 puv = uv + texelFetch(u_motion, mt, 0).rg * pc.mv_scale + pc.mv_offset;
    float hw = 0.0;
    vec3 hist = cur;
    if ((pc.flags & kReset) == 0u && all(greaterThanEqual(puv, vec2(0.0))) && all(lessThanEqual(puv, vec2(1.0)))) {
        vec3 hy = ycocg(tonemap(history(puv), k));
        vec3 cy = clip_box(lo, hi, hy);
        hist = rgb(cy);
        hw = textureLod(u_hist, puv, 0.0).a * clamp(1.0 - 4.0 * length(cy - hy), 0.0, 1.0);
    }
    if ((pc.flags & kMask) != 0u) hw *= 1.0 - clamp(texelFetch(u_mask, aux(pc.mask_base, pc.mask_ratio, region(ic)), 0).r, 0.0, 1.0);
    float total = min(hw + wmax, pc.max_weight);
    float a = total > 1e-6 ? clamp(wmax / total, 0.0, 1.0) : 1.0;
    imageStore(o_hist, o, vec4(untonemap(mix(hist, cur, a), k), total));
}
"""

# The input pass of the SDK upscalers (FSR 3, DLSS through Streamline): the game's colour, depth and motion vectors copied into the images
# of the TemporalFrame (temporal.h): colour rgba16f, depth r32f, motion rg16f in render UV units (previous minus current).
PREP = r"""
#version 450
layout(local_size_x = 8, local_size_y = 8) in;

layout(set = 0, binding = 0) uniform sampler2D u_color;
layout(set = 0, binding = 1) uniform sampler2D u_depth;
layout(set = 0, binding = 2) uniform sampler2D u_motion;
layout(set = 0, binding = 3, rgba16f) writeonly uniform image2D o_color;
layout(set = 0, binding = 4, r32f) writeonly uniform image2D o_depth;
layout(set = 0, binding = 5, rg16f) writeonly uniform image2D o_motion;

layout(push_constant) uniform Push {
    vec2 in_size;
    vec2 in_base;
    vec2 mv_scale;
    vec2 mv_offset;
    vec2 depth_base;
    vec2 depth_ratio;
    vec2 mv_base;
    vec2 mv_ratio;
} pc;
""" + COMMON + r"""
void main() {
    ivec2 t = ivec2(gl_GlobalInvocationID.xy);
    if (t.x >= int(pc.in_size.x) || t.y >= int(pc.in_size.y)) return;
    imageStore(o_color, t, vec4(safe_color(texelFetch(u_color, ivec2(pc.in_base) + t, 0).rgb), 1.0));
    imageStore(o_depth, t, vec4(texelFetch(u_depth, ivec2(pc.depth_base + floor((vec2(t) + 0.5) * pc.depth_ratio)), 0).r, 0.0, 0.0, 0.0));
    vec2 mv = texelFetch(u_motion, ivec2(pc.mv_base + floor((vec2(t) + 0.5) * pc.mv_ratio)), 0).rg * pc.mv_scale + pc.mv_offset;
    imageStore(o_motion, t, vec4(mv, 0.0, 0.0));
}
"""

HEADER = """// GENERATED by scripts/gen_taau_shaders.py: do not edit
// SPIR-V of the NVN2 DLSS host upscaler (taau.h), compiled with glslang -V --target-env vulkan1.3 -Os.
//
// kTaauComp: compute, local size 8 x 8 x 1, dispatch (ceil(out w / 8), ceil(out h / 8), 1).
//   Set 0: binding 0..4 COMBINED_IMAGE_SAMPLER (colour, depth, motion vectors, bias mask, history; texelFetch except the history, which is
//   filtered), binding 5 STORAGE_IMAGE rgba16f (the new history). Push constants: 104 bytes (TaauPush in taau.cpp).
// kTaauPrepComp: compute, local size 8 x 8 x 1, dispatch (ceil(in w / 8), ceil(in h / 8), 1).
//   Set 0: binding 0..2 COMBINED_IMAGE_SAMPLER (colour, depth, motion vectors), binding 3..5 STORAGE_IMAGE rgba16f, r32f, rg16f.
//   Push constants: 64 bytes (PrepPush in taau.cpp).
"""


def compile_one(glslang, spirv_val, tmp, name, src):
    comp = os.path.join(tmp, name + ".comp")
    spv = os.path.join(tmp, name + ".spv")
    with open(comp, "w", newline="\n") as f:
        f.write(src)
    res = subprocess.run([glslang, "-V", "--target-env", "vulkan1.3", "-Os", "-o", spv, comp], capture_output=True, text=True)
    if res.returncode != 0:
        sys.stderr.write("glslang failed on %s:\n%s%s" % (name, res.stdout, res.stderr))
        sys.exit(1)
    data = open(spv, "rb").read()
    if len(data) % 4 or len(data) == 0:
        sys.stderr.write("%s: bad SPIR-V size\n" % name)
        sys.exit(1)
    if spirv_val:
        v = subprocess.run([spirv_val, "--target-env", "vulkan1.3", spv], capture_output=True, text=True)
        if v.returncode != 0:
            sys.stderr.write("spirv-val failed on %s:\n%s\n" % (name, v.stdout + v.stderr))
            sys.exit(1)
    return [int.from_bytes(data[i:i + 4], "little") for i in range(0, len(data), 4)]


def main():
    glslang = find_glslang()
    spirv_val = find_spirv_val()
    with tempfile.TemporaryDirectory() as tmp:
        arrays = [("kTaauComp", compile_one(glslang, spirv_val, tmp, "taau", TAAU)),
                  ("kTaauPrepComp", compile_one(glslang, spirv_val, tmp, "taau_prep", PREP))]
    lines = [HEADER]
    for name, words in arrays:
        lines.append("static const uint32_t %s[] = {" % name)
        for i in range(0, len(words), 8):
            lines.append("    " + ", ".join("0x%08x" % w for w in words[i:i + 8]) + ",")
        lines.append("};")
    with open(out_path, "w", newline="\n") as f:
        f.write("\n".join(lines) + "\n")
    print("wrote %s (%s)%s" % (out_path, ", ".join("%s %d words" % (n, len(w)) for n, w in arrays),
                                "" if spirv_val else "; spirv-val not found, not validated"))


if __name__ == "__main__":
    main()
