// SPDX-License-Identifier: GPL-3.0-or-later
// NVN2 DLSS on the host (src/gfx/nvn/api_dlss*.cpp, src/gfx/extras/taau.*). The entry points are called through their thunks with the
// arguments Fortune's Weave passes (main 0x1913B40 network set-up, 0xB70F00 per frame); the evaluation runs on a headless device and its
// output is read back: size and region, a constant picture, convergence of a jittered static scene, the history reset and reprojection.
#include <algorithm>
#include <cmath>
#include <cstring>
#include <functional>
#include <vector>

#include "api_dlss.h"
#include "nvn_app.h"
#include "nvn_cmd.h"
#include "nvn_impl.h"
NXTEST_GROUP(gfx_nvn_dlss)

using namespace nvnapp;
using nxrt::nvn::abi::Caller;
namespace be = nxrt::nvn::be;

namespace {

template <class Sig>
Caller<Sig> P(const char* name) {
    return Caller<Sig>(nxrt::nvn::proc_address(name));
}

// The DLSS procs, resolved after registration (before it, proc_address would hand out the logging stub).
struct Dlss {
    using V = void*;
    using CV = const void*;
    Caller<void(V)> b_defaults = P<void(V)>("nvnDlssNetworkBuilderSetDefaults");
    Caller<void(V, V)> b_set_device = P<void(V, V)>("nvnDlssNetworkBuilderSetDevice");
    Caller<V(CV)> b_get_device = P<V(CV)>("nvnDlssNetworkBuilderGetDevice");
    Caller<void(V, int)> b_set_flags = P<void(V, int)>("nvnDlssNetworkBuilderSetFlags");
    Caller<int(CV)> b_get_flags = P<int(CV)>("nvnDlssNetworkBuilderGetFlags");
    Caller<void(V, int)> b_set_method = P<void(V, int)>("nvnDlssNetworkBuilderSetMethod");
    Caller<int(CV)> b_get_method = P<int(CV)>("nvnDlssNetworkBuilderGetMethod");
    Caller<void(V, int)> b_set_preset = P<void(V, int)>("nvnDlssNetworkBuilderSetPreset");
    Caller<int(CV)> b_get_preset = P<int(CV)>("nvnDlssNetworkBuilderGetPreset");
    Caller<void(V, int, int)> b_set_out = P<void(V, int, int)>("nvnDlssNetworkBuilderSetOutputResolution");
    Caller<void(CV, int*, int*)> b_get_out = P<void(CV, int*, int*)>("nvnDlssNetworkBuilderGetOutputResolution");
    Caller<void(V, int)> b_set_entries = P<void(V, int)>("nvnDlssNetworkBuilderSetMaxEvaluationEntries");
    Caller<int(CV)> b_get_entries = P<int(CV)>("nvnDlssNetworkBuilderGetMaxEvaluationEntries");
    Caller<void(V, V, size_t)> b_set_storage = P<void(V, V, size_t)>("nvnDlssNetworkBuilderSetStorage");
    Caller<V(CV)> b_get_memory = P<V(CV)>("nvnDlssNetworkBuilderGetMemory");
    Caller<size_t(CV)> b_get_memory_size = P<size_t(CV)>("nvnDlssNetworkBuilderGetMemorySize");
    Caller<void(V, CV, const int*, int)> b_set_ids = P<void(V, CV, const int*, int)>("nvnDlssNetworkBuilderSetTextureIDs");
    Caller<void(CV, int*)> b_get_ids = P<void(CV, int*)>("nvnDlssNetworkBuilderGetTextureIDs");
    Caller<CV(CV)> b_get_pool = P<CV(CV)>("nvnDlssNetworkBuilderGetTexturePool");
    // called with the game's eighth argument (null)
    Caller<void(CV, int*, int*, int*, int*, int*, int*, V)> b_optimal = P<void(CV, int*, int*, int*, int*, int*, int*, V)>("nvnDlssNetworkBuilderGetOptimalSettings");
    Caller<size_t(CV)> b_storage_size = P<size_t(CV)>("nvnDlssNetworkBuilderGetStorageSize");
    Caller<uint8_t(V, CV)> n_init = P<uint8_t(V, CV)>("nvnDlssNetworkInitialize");
    Caller<void(V)> n_finalize = P<void(V)>("nvnDlssNetworkFinalize");
    Caller<uint8_t(V, int, int)> n_resize = P<uint8_t(V, int, int)>("nvnDlssNetworkResize");
    Caller<void(V)> p_defaults = P<void(V)>("nvnDlssParametersSetDefaults");
    Caller<void(V, CV, CV)> p_color = P<void(V, CV, CV)>("nvnDlssParametersSetInputColor");
    Caller<void(V, CV, CV)> p_output = P<void(V, CV, CV)>("nvnDlssParametersSetOutputColor");
    Caller<void(V, CV, CV)> p_motion = P<void(V, CV, CV)>("nvnDlssParametersSetMotionVectors");
    Caller<void(V, CV, CV)> p_depth = P<void(V, CV, CV)>("nvnDlssParametersSetDepth");
    Caller<void(V, CV, CV)> p_mask = P<void(V, CV, CV)>("nvnDlssParametersSetBiasCurrentColorMask");
    Caller<void(V, float, float)> p_jitter = P<void(V, float, float)>("nvnDlssParametersSetJitterOffset");
    Caller<void(CV, float*, float*)> p_get_jitter = P<void(CV, float*, float*)>("nvnDlssParametersGetJitterOffset");
    Caller<void(V, float, float)> p_mv_scale = P<void(V, float, float)>("nvnDlssParametersSetMotionVectorScale");
    Caller<void(CV, float*, float*)> p_get_mv_scale = P<void(CV, float*, float*)>("nvnDlssParametersGetMotionVectorScale");
    Caller<void(V, float)> p_pre_exposure = P<void(V, float)>("nvnDlssParametersSetPreExposure");
    Caller<float(CV)> p_get_pre_exposure = P<float(CV)>("nvnDlssParametersGetPreExposure");
    Caller<void(V, uint8_t)> p_reset = P<void(V, uint8_t)>("nvnDlssParametersSetHistoryReset");
    Caller<uint8_t(CV)> p_get_reset = P<uint8_t(CV)>("nvnDlssParametersGetHistoryReset");
    Caller<void(V, int)> p_index = P<void(V, int)>("nvnDlssParametersSetEvaluationIndex");
    Caller<int(CV)> p_get_index = P<int(CV)>("nvnDlssParametersGetEvaluationIndex");
    Caller<void(V, int, int)> p_in_size = P<void(V, int, int)>("nvnDlssParametersSetInputSubrectangleSize");
    Caller<void(CV, int*, int*)> p_get_in_size = P<void(CV, int*, int*)>("nvnDlssParametersGetInputSubrectangleSize");
    Caller<void(V, int, int)> p_color_base = P<void(V, int, int)>("nvnDlssParametersSetInputColorSubrectangleBase");
    Caller<void(CV, int*, int*)> p_get_color_base = P<void(CV, int*, int*)>("nvnDlssParametersGetInputColorSubrectangleBase");
    Caller<void(V, int, int)> p_depth_base = P<void(V, int, int)>("nvnDlssParametersSetDepthSubrectangleBase");
    Caller<void(V, int, int)> p_mv_base = P<void(V, int, int)>("nvnDlssParametersSetMotionVectorSubrectangleBase");
    Caller<void(V, int, int)> p_out_base = P<void(V, int, int)>("nvnDlssParametersSetOutputSubrectangleBase");
    Caller<void(CV, int*, int*)> p_get_out_base = P<void(CV, int*, int*)>("nvnDlssParametersGetOutputSubrectangleBase");
    Caller<void(V, int)> p_overlay_nans = P<void(V, int)>("nvnDlssParametersSetOverlayShowNans");
    Caller<void(V, CV, CV)> evaluate = P<void(V, CV, CV)>("nvnCommandBufferEvaluateDlss");
};

void setup_procs() {
    nxrt::nvn::install();
    nxrt::nvn::register_api_cmdbuf();
    nxrt::nvn::register_api_dlss();
}

// ---- half floats --------------------------------------------------------------------------------------------------------------------
uint16_t to_half(float f) {
    uint32_t x;
    std::memcpy(&x, &f, 4);
    const uint32_t sign = (x >> 16) & 0x8000u;
    int e = static_cast<int>((x >> 23) & 0xFF) - 127 + 15;
    uint32_t m = x & 0x7FFFFFu;
    if (e <= 0) return static_cast<uint16_t>(sign);
    if (e >= 31) return static_cast<uint16_t>(sign | 0x7C00u);
    m += 0x1000u;   // round to nearest
    if (m & 0x800000u) {
        m = 0;
        ++e;
        if (e >= 31) return static_cast<uint16_t>(sign | 0x7C00u);
    }
    return static_cast<uint16_t>(sign | (static_cast<uint32_t>(e) << 10) | (m >> 13));
}
float from_half(uint16_t h) {
    const uint32_t sign = (h & 0x8000u) << 16, e = (h >> 10) & 0x1F, m = h & 0x3FF;
    uint32_t x;
    if (e == 0) {
        if (!m) x = sign;
        else {
            float v = std::ldexp(static_cast<float>(m), -24);
            return sign ? -v : v;
        }
    } else if (e == 31) x = sign | 0x7F800000u | (m << 13);
    else x = sign | ((e - 15 + 127) << 23) | (m << 13);
    float f;
    std::memcpy(&f, &x, 4);
    return f;
}

// ---- a DLSS pass of the game, on the App's device ----------------------------------------------------------------------------------
struct Pass {
    App& app;
    Dlss d;
    int in_w, in_h, out_w, out_h;
    std::unique_ptr<Tex> color, depth, motion, out;
    std::unique_ptr<App::CmdBuf> cb;
    App::Buf readback;
    alignas(16) uint8_t object[0x858] = {};   // the game's DLSS object: the network at +0x28
    alignas(16) uint8_t builder[0xC0] = {};
    alignas(16) uint8_t params[0xC0] = {};
    int frames = 0;
    void* net() { return object + 0x28; }

    Pass(App& a, int iw, int ih, int ow, int oh, int out_format = NVN_FORMAT_RGBA16F, int tex_w = 0, int tex_h = 0)
        : app(a), in_w(iw), in_h(ih), out_w(ow), out_h(oh) {
        color = app.make_texture_2d(NVN_FORMAT_RGBA16F, iw, ih, 0, 1, false);
        depth = app.make_texture_2d(NVN_FORMAT_DEPTH32F, iw, ih, 0, 1, false);
        motion = app.make_texture_2d(NVN_FORMAT_RG16F, iw, ih, 0, 1, false);
        out = app.make_texture_2d(out_format, tex_w ? tex_w : ow, tex_h ? tex_h : oh, 0, 1, false);
        cb = app.make_cmdbuf();
        readback = app.make_buffer(static_cast<size_t>(out->width) * out->height * 8);
    }
    bool ok() const { return color && depth && motion && out && cb; }

    // main 0x1913B40: the builder as the game fills it
    bool init(int method, int flags = 0x2000000A) {
        d.b_defaults(builder);
        d.b_set_device(builder, app.device.get());
        d.b_set_out(builder, out_w, out_h);
        d.b_set_method(builder, method);
        d.b_set_preset(builder, 5);
        int o[6] = {};
        d.b_optimal(builder, &o[0], &o[1], &o[2], &o[3], &o[4], &o[5], nullptr);
        int ids[32];
        for (int i = 0; i < 32; ++i) ids[i] = 200 + i;
        d.b_set_ids(builder, app.tex_pool.get(), ids, 32);
        d.b_set_flags(builder, flags);
        d.b_set_entries(builder, 2);
        storage.assign(d.b_storage_size(builder), 0);
        d.b_set_storage(builder, storage.data(), storage.size());
        return d.n_init(net(), builder) != 0;
    }
    std::vector<uint8_t> storage;

    void clear(NVNcommandBuffer* c, Tex& t, float r, float g, float b, float a) {
        const float v[4] = {r, g, b, a};
        app.api.nvnCommandBufferClearTexture(c, t.obj.get(), nullptr, nullptr, v, NVN_CLEAR_COLOR_MASK_RGBA);
    }
    // RGBA floats of the input picture (in_w x in_h)
    void set_color(const std::vector<float>& rgba) {
        std::vector<uint16_t> h(rgba.size());
        for (size_t i = 0; i < rgba.size(); ++i) h[i] = to_half(rgba[i]);
        NVNcopyRegion r{0, 0, 0, in_w, in_h, 1};
        app.api.nvnTextureWriteTexels(color->obj.get(), nullptr, &r, h.data());
        app.api.nvnTextureFlushTexels(color->obj.get(), nullptr, &r);
    }
    // main 0xB70F00, then the copy of the output into the readback buffer
    void evaluate(float jx, float jy, bool reset, float mvx = 0.0f, float mvy = 0.0f, int out_bx = 0, int out_by = 0,
                  const std::function<void(NVNcommandBuffer*)>& before = nullptr) {
        NVNcommandBuffer* c = cb->obj.get();
        app.api.nvnCommandBufferBeginRecording(c);
        if (before) before(c);
        clear(c, *motion, mvx, mvy, 0, 0);
        const float zero[4] = {0.5f, 0, 0, 0};
        app.api.nvnCommandBufferClearTexture(c, depth->obj.get(), nullptr, nullptr, zero, NVN_CLEAR_COLOR_MASK_RGBA);
        d.p_defaults(params);
        d.p_color(params, color->obj.get(), nullptr);
        d.p_in_size(params, in_w, in_h);
        d.p_color_base(params, 0, 0);
        d.p_depth(params, depth->obj.get(), nullptr);
        d.p_depth_base(params, 0, 0);
        d.p_motion(params, motion->obj.get(), nullptr);
        d.p_mv_base(params, 0, 0);
        d.p_mv_scale(params, static_cast<float>(in_w), static_cast<float>(in_h));
        d.p_output(params, out->obj.get(), nullptr);
        d.p_out_base(params, out_bx, out_by);
        d.p_reset(params, reset ? 1 : 0);
        d.p_jitter(params, jx, jy);
        d.p_index(params, frames++ & 1);   // the game alternates 0 and 1 every frame (main 0x648DE0)
        d.evaluate(c, net(), params);
        app.api.nvnCommandBufferSetCopyRowStride(c, static_cast<ptrdiff_t>(out->width) * bpp());
        NVNcopyRegion r{0, 0, 0, out->width, out->height, 1};
        app.api.nvnCommandBufferCopyTextureToBuffer(c, out->obj.get(), nullptr, &r, readback.address, NVN_COPY_FLAGS_NONE);
        app.end_and_submit(*cb);
        app.finish();
    }
    int bpp() const { return out->format == NVN_FORMAT_RGBA8 ? 4 : 8; }
    // texel (x, y) of the output texture, RGBA
    void at(int x, int y, float px[4]) const {
        const uint8_t* p = readback.ptr + (static_cast<size_t>(y) * out->width + x) * bpp();
        for (int i = 0; i < 4; ++i) {
            if (bpp() == 4) px[i] = p[i] / 255.0f;
            else {
                uint16_t h;
                std::memcpy(&h, p + 2 * i, 2);
                px[i] = from_half(h);
            }
        }
    }
};

double radical_inverse(uint32_t i, uint32_t base) {
    double f = 1.0, r = 0.0;
    while (i) {
        f /= base;
        r += f * (i % base);
        i /= base;
    }
    return r;
}
// The game's jitter (main 0xAB0780): Halton(2, 3) minus 0.5, in render pixels.
void halton_jitter(uint32_t frame, float* jx, float* jy) {
    *jx = static_cast<float>(radical_inverse(frame % 8 + 1, 2) - 0.5);
    *jy = static_cast<float>(radical_inverse(frame % 8 + 1, 3) - 0.5);
}

}  // namespace

// ---- entry points --------------------------------------------------------------------------------------------------------------------
NXTEST(every_dlss_proc_is_implemented) {
    setup_procs();
    const std::vector<std::string> names = be::dlss_proc_names();
    std::printf("    %zu DLSS-family entry points\n", names.size());
    CHECK_EQ(names.size(), size_t(58));
    for (const std::string& n : names) CHECK_MSG(nxrt::nvn::proc_address(n.c_str()) != 0, n);
    // the 30 procs Fortune's Weave calls (main 0x1913B84..0x1913FA0 and 0xB70F78..0xB711C0) are among them
    const char* const used[] = {
        "nvnDlssNetworkFinalize", "nvnDlssNetworkBuilderSetDefaults", "nvnDlssNetworkBuilderSetDevice", "nvnDlssNetworkBuilderSetOutputResolution",
        "nvnDlssNetworkBuilderSetMethod", "nvnDlssNetworkBuilderSetPreset", "nvnDlssNetworkBuilderGetOptimalSettings", "nvnDlssNetworkBuilderSetTextureIDs",
        "nvnDlssNetworkBuilderSetFlags", "nvnDlssNetworkBuilderSetMaxEvaluationEntries", "nvnDlssNetworkBuilderGetStorageSize",
        "nvnDlssNetworkBuilderSetStorage", "nvnDlssNetworkInitialize", "nvnDlssParametersSetDefaults", "nvnDlssParametersSetInputColor",
        "nvnDlssParametersSetInputSubrectangleSize", "nvnDlssParametersSetInputColorSubrectangleBase", "nvnDlssParametersSetDepth",
        "nvnDlssParametersSetDepthSubrectangleBase", "nvnDlssParametersSetMotionVectors", "nvnDlssParametersSetMotionVectorSubrectangleBase",
        "nvnDlssParametersSetMotionVectorScale", "nvnDlssParametersSetOutputColor", "nvnDlssParametersSetOutputSubrectangleBase",
        "nvnDlssParametersSetHistoryReset", "nvnDlssParametersSetJitterOffset", "nvnDlssParametersSetBiasCurrentColorMask",
        "nvnDlssParametersSetBiasCurrentColorSubrectangleBase", "nvnDlssParametersSetEvaluationIndex", "nvnCommandBufferEvaluateDlss"};
    for (const char* u : used) CHECK_MSG(std::find(names.begin(), names.end(), std::string(u)) != names.end(), u);
}

NXTEST(builder_behaves_as_the_driver) {
    setup_procs();
    Dlss d;
    alignas(16) uint8_t b[0xC0];
    std::memset(b, 0xCD, sizeof b);
    void* const dev = reinterpret_cast<void*>(0x1234567800ull);
    d.b_set_device(b, dev);
    d.b_defaults(b);   // clears 0x08..0xC0, keeps the device, one evaluation entry
    CHECK_EQ(d.b_get_device(b), dev);
    CHECK_EQ(d.b_get_flags(b), 0);
    CHECK_EQ(d.b_get_method(b), 0);
    CHECK_EQ(d.b_get_preset(b), 0);
    CHECK_EQ(d.b_get_entries(b), 1);
    CHECK_EQ(d.b_get_memory(b), static_cast<void*>(nullptr));
    CHECK_EQ(d.b_get_memory_size(b), size_t(0));
    CHECK_EQ(d.b_get_pool(b), static_cast<const void*>(nullptr));
    size_t nonzero = 0;
    for (size_t i = 8; i < sizeof b; ++i) nonzero += b[i] != 0;
    CHECK_EQ(nonzero, size_t(1));   // only max entries (0x34) is set
    CHECK_EQ(b[0x34], uint8_t(1));

    d.b_set_flags(b, 0x2000004A);
    d.b_set_method(b, 2);
    d.b_set_preset(b, 9);
    d.b_set_out(b, 3840, 2160);
    d.b_set_entries(b, 2);
    int w = 0, h = 0;
    d.b_get_out(b, &w, &h);
    CHECK_EQ(w, 3840);
    CHECK_EQ(h, 2160);
    CHECK_EQ(d.b_get_flags(b), 0x2000004A);
    CHECK_EQ(d.b_get_method(b), 2);
    CHECK_EQ(d.b_get_preset(b), 9);
    CHECK_EQ(d.b_get_entries(b), 2);
    // the driver's offsets
    int32_t v;
    std::memcpy(&v, b + 0x20, 4);
    CHECK_EQ(v, 0x2000004A);
    std::memcpy(&v, b + 0x24, 4);
    CHECK_EQ(v, 2);
    std::memcpy(&v, b + 0x2C, 4);
    CHECK_EQ(v, 3840);
    std::memcpy(&v, b + 0x30, 4);
    CHECK_EQ(v, 2160);

    uint8_t mem[64];
    d.b_set_storage(b, mem, sizeof mem);
    CHECK_EQ(d.b_get_memory(b), static_cast<void*>(mem));
    CHECK_EQ(d.b_get_memory_size(b), sizeof mem);
    CHECK(d.b_storage_size(b) >= 0x1000);

    int ids[32], back[32];
    for (int i = 0; i < 32; ++i) ids[i] = 1000 + i;
    const void* pool = reinterpret_cast<const void*>(0xABC000ull);
    d.b_set_ids(b, pool, ids, 32);
    d.b_get_ids(b, back);
    CHECK(std::memcmp(ids, back, sizeof ids) == 0);
    CHECK_EQ(d.b_get_pool(b), pool);
    std::memcpy(&v, b + 0x38, 4);
    CHECK_EQ(v, 1000);
    d.b_set_ids(b, pool, ids, 8);   // a smaller reservation: the rest reads 0
    d.b_get_ids(b, back);
    CHECK_EQ(back[7], 1007);
    CHECK_EQ(back[8], 0);
    d.b_set_ids(b, pool, nullptr, 0);   // none reserved (device info 0x85 = 0)
    d.b_get_ids(b, back);
    CHECK_EQ(back[0], 0);
}

NXTEST(optimal_settings_are_the_consoles) {
    setup_procs();
    Dlss d;
    alignas(16) uint8_t b[0xC0];
    d.b_defaults(b);
    struct Case { int w, h, method; int want[6]; };
    // NGX: method 0 -> Balanced (0.58, range 0.5 .. 1), 1 -> Ultra Performance (1/3), 2 -> DLAA (1, range 0.99 .. 1); the networks the
    // console builds are 2048x1152 method 0 and 3840x2160 method 2
    const Case cases[] = {
        {2048, 1152, 0, {1188, 668, 1024, 576, 2048, 1152}},
        {3840, 2160, 2, {3840, 2160, 3802, 2138, 3840, 2160}},
        {3840, 2160, 1, {1280, 720, 1280, 720, 1280, 720}},
        {1920, 1080, 0, {1114, 626, 960, 540, 1920, 1080}},
        {1280, 720, 7, {742, 418, 640, 360, 1280, 720}},
    };
    for (const Case& c : cases) {
        d.b_set_out(b, c.w, c.h);
        d.b_set_method(b, c.method);
        int o[6] = {-1, -1, -1, -1, -1, -1};
        d.b_optimal(b, &o[0], &o[1], &o[2], &o[3], &o[4], &o[5], nullptr);
        std::printf("    %dx%d method %d: optimal %dx%d, min %dx%d, max %dx%d\n", c.w, c.h, c.method, o[0], o[1], o[2], o[3], o[4], o[5]);
        for (int i = 0; i < 6; ++i) CHECK_EQ(o[i], c.want[i]);
    }
}

NXTEST(parameters_behave_as_the_driver) {
    setup_procs();
    Dlss d;
    alignas(16) uint8_t p[0xC0];
    std::memset(p, 0xCD, sizeof p);
    d.p_defaults(p);
    float x = -1, y = -1;
    d.p_get_mv_scale(p, &x, &y);
    CHECK(x == 1.0f && y == 1.0f);
    d.p_get_jitter(p, &x, &y);
    CHECK(x == 0.0f && y == 0.0f);
    CHECK(d.p_get_pre_exposure(p) == 1.0f);
    CHECK_EQ(d.p_get_reset(p), uint8_t(0));
    CHECK_EQ(d.p_get_index(p), 0);
    int32_t ov[4];
    std::memcpy(ov, p + 0xB0, sizeof ov);
    CHECK(ov[0] == -1 && ov[1] == -1 && ov[2] == -1 && ov[3] == -1);
    uint64_t tex;
    std::memcpy(&tex, p, 8);
    CHECK_EQ(tex, 0ull);

    const void* t = reinterpret_cast<const void*>(0x5000ull);
    const void* v = reinterpret_cast<const void*>(0x6000ull);
    d.p_color(p, t, v);
    d.p_mask(p, t, nullptr);
    d.p_jitter(p, 0.25f, -0.375f);
    d.p_mv_scale(p, 1024.0f, 576.0f);
    d.p_pre_exposure(p, 2.5f);
    d.p_reset(p, 1);
    d.p_index(p, 1);
    d.p_in_size(p, 1024, 576);
    d.p_color_base(p, 3, 4);
    d.p_out_base(p, 5, 6);
    d.p_overlay_nans(p, 1);
    d.p_get_jitter(p, &x, &y);
    CHECK(x == 0.25f && y == -0.375f);
    d.p_get_mv_scale(p, &x, &y);
    CHECK(x == 1024.0f && y == 576.0f);
    CHECK(d.p_get_pre_exposure(p) == 2.5f);
    CHECK_EQ(d.p_get_reset(p), uint8_t(1));
    CHECK_EQ(d.p_get_index(p), 1);
    int a = 0, b2 = 0;
    d.p_get_in_size(p, &a, &b2);
    CHECK(a == 1024 && b2 == 576);
    d.p_get_color_base(p, &a, &b2);
    CHECK(a == 3 && b2 == 4);
    d.p_get_out_base(p, &a, &b2);
    CHECK(a == 5 && b2 == 6);
    // the driver's offsets: colour texture and view at 0, jitter at 0x50, reset at 0x78, overlay "show NaNs" at 0xB8
    std::memcpy(&tex, p, 8);
    CHECK_EQ(tex, 0x5000ull);
    std::memcpy(&tex, p + 8, 8);
    CHECK_EQ(tex, 0x6000ull);
    std::memcpy(&tex, p + 0x40, 8);
    CHECK_EQ(tex, 0x5000ull);
    float f;
    std::memcpy(&f, p + 0x50, 4);
    CHECK(f == 0.25f);
    CHECK_EQ(p[0x78], uint8_t(1));
    std::memcpy(ov, p + 0xB0, sizeof ov);
    CHECK_EQ(ov[2], 1);
}

NXTEST(network_initialize_resize_finalize) {
    setup_procs();
    Dlss d;
    alignas(16) uint8_t b[0xC0], net[0x800];
    std::memset(net, 0xEE, sizeof net);
    d.b_defaults(b);
    CHECK_EQ(d.n_init(net, b), uint8_t(0));   // no output resolution
    d.b_set_out(b, 2048, 1152);
    CHECK_EQ(d.n_init(net, b), uint8_t(1));
    size_t dirty = 0;
    for (size_t i = 16; i < be::kDlssNetworkSize; ++i) dirty += net[i] != 0;
    CHECK_EQ(dirty, size_t(0));                 // cleared as the driver clears it (0x258 bytes), the header aside
    CHECK_EQ(net[be::kDlssNetworkSize], uint8_t(0xEE));
    CHECK_EQ(d.n_resize(net, 4096, 1152), uint8_t(0));   // above the size it was made with
    CHECK_EQ(d.n_resize(net, 1920, 1080), uint8_t(1));
    CHECK_EQ(d.n_init(net, b), uint8_t(1));     // again, without Finalize
    d.n_finalize(net);
    CHECK_EQ(d.n_resize(net, 1024, 576), uint8_t(0));
    d.n_finalize(net);                          // twice: nothing happens
}

NXTEST(evaluate_records_one_command) {
    setup_procs();
    Dlss d;
    using namespace nxrt::nvn::be;
    NVNtexture color{}, output{}, depth{};
    Texture* ch = new Texture();
    Texture* oh = new Texture();
    Texture* dh = new Texture();
    bind_guest(&color, ch);
    bind_guest(&output, oh);
    bind_guest(&depth, dh);
    alignas(16) uint8_t b[0xC0], net[0x800] = {}, p[0xC0];
    d.b_defaults(b);
    d.b_set_out(b, 2048, 1152);
    d.b_set_flags(b, 0x2000000A);
    REQUIRE(d.n_init(net, b));
    d.p_defaults(p);
    d.p_color(p, &color, nullptr);
    d.p_output(p, &output, nullptr);
    d.p_depth(p, &depth, nullptr);
    d.p_jitter(p, 0.125f, -0.25f);
    d.p_reset(p, 1);
    d.p_index(p, 1);
    NVNcommandBuffer cmd;
    std::memset(&cmd, 0, sizeof cmd);
    auto init = nxrt::nvn::abi::proc<PFNNVNCOMMANDBUFFERINITIALIZEPROC>("nvnCommandBufferInitialize");
    REQUIRE(init(&cmd, nullptr));
    auto begin = nxrt::nvn::abi::proc<PFNNVNCOMMANDBUFFERBEGINRECORDINGPROC>("nvnCommandBufferBeginRecording");
    auto end = nxrt::nvn::abi::proc<PFNNVNCOMMANDBUFFERENDRECORDINGPROC>("nvnCommandBufferEndRecording");
    begin(&cmd);
    d.evaluate(&cmd, net, p);
    d.p_output(p, nullptr, nullptr);
    d.evaluate(&cmd, net, p);                   // no output texture: nothing recorded
    const uint64_t h = end(&cmd);
    Ref<CmdList> list = handle_lookup(h);
    REQUIRE(list);
    RecordReader r(*list);
    REQUIRE(r.next());
    CHECK(r.op() == Op::EvaluateDlss);
    const auto* e = static_cast<const DlssEval*>(r.payload<P_EvaluateDlss>().eval);
    REQUIRE(e != nullptr);
    CHECK(e->color.get() == ch);
    CHECK(e->output.get() == oh);
    CHECK(e->depth.get() == dh);
    CHECK(!e->motion);
    CHECK(e->p.jitter[0] == 0.125f && e->p.jitter[1] == -0.25f);
    CHECK_EQ(e->p.reset, uint8_t(1));
    CHECK_EQ(e->p.eval_index, 1);
    CHECK_EQ(e->out_w, 2048);
    CHECK_EQ(e->out_h, 1152);
    CHECK_EQ(e->net->flags, 0x2000000A);
    CHECK(!r.next());
    d.n_finalize(net);
    d.evaluate(&cmd, net, p);                   // finalized: nothing
}

// ---- the evaluation on the GPU ---------------------------------------------------------------------------------------------------------
NXTEST(constant_picture_fills_the_output) {
    App app;
    App::Opts o;
    o.width = 64;
    o.height = 64;
    REQUIRE(app.init(o));
    for (int fmt : {static_cast<int>(NVN_FORMAT_RGBA16F), static_cast<int>(NVN_FORMAT_RGBA8)}) {
        Pass ps(app, 32, 18, 64, 36, fmt);
        REQUIRE(ps.ok());
        REQUIRE(ps.init(0));
        std::vector<float> c(32 * 18 * 4);
        for (size_t i = 0; i < c.size(); i += 4) {
            c[i] = 0.5f;
            c[i + 1] = 0.25f;
            c[i + 2] = 0.125f;
            c[i + 3] = 1.0f;
        }
        ps.set_color(c);
        for (int frame = 0; frame < 3; ++frame) {
            float jx, jy;
            halton_jitter(static_cast<uint32_t>(frame), &jx, &jy);
            ps.evaluate(jx, jy, frame == 0);
            float worst = 0;
            for (int y = 0; y < 36; ++y)
                for (int x = 0; x < 64; ++x) {
                    float px[4];
                    ps.at(x, y, px);
                    worst = std::max({worst, std::fabs(px[0] - 0.5f), std::fabs(px[1] - 0.25f), std::fabs(px[2] - 0.125f)});
                }
            std::printf("    output %s, frame %d: worst channel error %.5f\n", fmt == NVN_FORMAT_RGBA8 ? "RGBA8" : "RGBA16F", frame, worst);
            CHECK(worst < (fmt == NVN_FORMAT_RGBA8 ? 2.5f / 255.0f : 2e-3f));
        }
        ps.d.n_finalize(ps.net());
    }
    const auto v = app.validation();
    std::printf("    validation: %llu errors %s\n", static_cast<unsigned long long>(v.errors), v.first_error.c_str());
    CHECK_EQ(v.errors, 0ull);
}

NXTEST(output_region_follows_the_network_size_and_base) {
    App app;
    App::Opts o;
    o.width = 64;
    o.height = 64;
    REQUIRE(app.init(o));
    Pass ps(app, 32, 18, 64, 36, NVN_FORMAT_RGBA16F, 80, 48);   // the output texture is larger than the network's output
    REQUIRE(ps.ok());
    REQUIRE(ps.init(0));
    ps.set_color(std::vector<float>(32 * 18 * 4, 1.0f));
    auto marker = [&](NVNcommandBuffer* c) { ps.clear(c, *ps.out, 0.25f, 0.25f, 0.25f, 0.25f); };
    auto count = [&](int bx, int by, int w, int h, size_t* in_ok, size_t* out_ok) {
        *in_ok = *out_ok = 0;
        for (int y = 0; y < 48; ++y)
            for (int x = 0; x < 80; ++x) {
                float px[4];
                ps.at(x, y, px);
                const bool inside = x >= bx && x < bx + w && y >= by && y < by + h;
                if (inside) *in_ok += std::fabs(px[0] - 1.0f) < 1e-3f;
                else *out_ok += std::fabs(px[0] - 0.25f) < 1e-3f;
            }
    };
    size_t in_ok, out_ok;
    ps.evaluate(0, 0, true, 0, 0, 8, 6, marker);
    count(8, 6, 64, 36, &in_ok, &out_ok);
    std::printf("    64x36 at 8,6: %zu of %d texels written, %zu of %d left alone\n", in_ok, 64 * 36, out_ok, 80 * 48 - 64 * 36);
    CHECK_EQ(in_ok, size_t(64 * 36));
    CHECK_EQ(out_ok, size_t(80 * 48 - 64 * 36));
    REQUIRE(ps.d.n_resize(ps.net(), 48, 27));
    ps.evaluate(0, 0, true, 0, 0, 0, 0, marker);
    count(0, 0, 48, 27, &in_ok, &out_ok);
    std::printf("    resized to 48x27 at 0,0: %zu of %d texels written, %zu of %d left alone\n", in_ok, 48 * 27, out_ok, 80 * 48 - 48 * 27);
    CHECK_EQ(in_ok, size_t(48 * 27));
    CHECK_EQ(out_ok, size_t(80 * 48 - 48 * 27));
    ps.d.n_finalize(ps.net());
}

namespace {
// The scene (output pixel coordinates): smooth bands plus a hard diagonal edge, between 0.1 and 0.9.
float scene(float x, float y) {
    const float bands = 0.5f + 0.3f * std::sin(x * 0.9f) * std::cos(y * 0.7f);
    return (x + 0.6f * y > 40.0f) ? bands * 0.4f + 0.5f : bands * 0.6f;
}
// A frame of the 2x scene as a jittered renderer draws it: texel t shows the scene at t + 0.5 - sign * jitter (render pixels).
std::vector<float> render(int w, int h, float jx, float jy, float sign, float shift = 0.0f, float exposure = 1.0f) {
    std::vector<float> c(static_cast<size_t>(w) * h * 4);
    for (int y = 0; y < h; ++y)
        for (int x = 0; x < w; ++x) {
            const float v = exposure * scene(2.0f * (x + 0.5f - sign * jx) - shift, 2.0f * (y + 0.5f - sign * jy));
            float* p = &c[(static_cast<size_t>(y) * w + x) * 4];
            p[0] = p[1] = p[2] = v;
            p[3] = 1.0f;
        }
    return c;
}
double error_vs_scene(const Pass& ps, float shift = 0.0f, int margin = 3, float exposure = 1.0f) {
    double e = 0;
    int n = 0;
    for (int y = margin; y < ps.out_h - margin; ++y)
        for (int x = margin; x < ps.out_w - margin; ++x) {
            float px[4];
            ps.at(x, y, px);
            e += std::fabs(px[0] / exposure - scene(x + 0.5f - shift, y + 0.5f));
            ++n;
        }
    return e / n;
}
}  // namespace

static std::vector<float> output_red(const Pass& ps) {
    std::vector<float> v(static_cast<size_t>(ps.out_w) * ps.out_h);
    for (int y = 0; y < ps.out_h; ++y)
        for (int x = 0; x < ps.out_w; ++x) {
            float px[4];
            ps.at(x, y, px);
            v[static_cast<size_t>(y) * ps.out_w + x] = px[0];
        }
    return v;
}
static double mean_change(const std::vector<float>& a, const std::vector<float>& b) {
    double d = 0;
    for (size_t i = 0; i < a.size(); ++i) d += std::fabs(a[i] - b[i]);
    return d / static_cast<double>(a.size());
}

// A static scene rendered at half size with the game's jitter: the history gathers the sub-pixel samples, so the result gets closer to the
// full-size scene than one frame (the reset frame) can; rendered with the opposite jitter sign it does not. Once converged the picture holds
// still from frame to frame (no jitter shimmer), unlike frames upscaled one by one (a reset every frame). The same holds for the scene at the
// brightness Fortune's Weave hands over (pre-exposure HDR, a thousandth of this one).
NXTEST(jittered_static_scene_converges) {
    App app;
    App::Opts o;
    o.width = 64;
    o.height = 64;
    REQUIRE(app.init(o));
    double first[2] = {}, last[4] = {}, shimmer[3] = {};
    for (int s = 0; s < 4; ++s) {
        const float sign = s == 1 ? -1.0f : 1.0f, exposure = s == 3 ? 0.001f : 1.0f;
        Pass ps(app, 32, 32, 64, 64);
        REQUIRE(ps.ok());
        REQUIRE(ps.init(0));
        std::vector<float> prev;
        for (uint32_t frame = 0; frame < 40; ++frame) {
            float jx, jy;
            halton_jitter(frame, &jx, &jy);
            ps.set_color(render(32, 32, jx, jy, sign, 0.0f, exposure));
            ps.evaluate(jx, jy, frame == 0 || s == 2);
            if (frame == 0 && s < 2) first[s] = error_vs_scene(ps);
            std::vector<float> cur = output_red(ps);
            if (frame >= 32 && s < 3) shimmer[s] += mean_change(cur, prev) / 8.0;
            prev = std::move(cur);
        }
        last[s] = error_vs_scene(ps, 0.0f, 3, exposure);
        ps.d.n_finalize(ps.net());
    }
    std::printf("    at a thousandth of the brightness: error after 40 frames %.4f (relative)\n", last[3]);
    CHECK(last[3] < 1.3 * last[0]);
    std::printf("    mean change between the last 8 frames: %.5f converged, %.5f frame by frame (a reset every frame)\n", shimmer[0], shimmer[2]);
    CHECK(shimmer[0] < 0.25 * shimmer[2]);
    std::printf("    mean error vs the full-size scene: first frame %.4f, after 40 frames %.4f (jitter as declared) / %.4f (opposite sign)\n", first[0],
                last[0], last[1]);
    CHECK(last[0] < 0.75 * first[0]);
    CHECK(last[0] < 0.8 * last[1]);
}

// The history follows the motion vectors: a scene panning right by one render pixel a frame is tracked with the game's vectors (previous
// minus current position, in render UV units with the motion vector scale of the render size) better than with none.
NXTEST(history_follows_the_motion_vectors) {
    App app;
    App::Opts o;
    o.width = 64;
    o.height = 64;
    REQUIRE(app.init(o));
    double err[2] = {};
    for (int s = 0; s < 2; ++s) {
        Pass ps(app, 32, 32, 64, 64);
        REQUIRE(ps.ok());
        REQUIRE(ps.init(0));
        float shift = 0;
        for (uint32_t frame = 0; frame < 24; ++frame) {
            float jx, jy;
            halton_jitter(frame, &jx, &jy);
            shift = 2.0f * frame;   // output pixels
            ps.set_color(render(32, 32, jx, jy, 1.0f, shift));
            const float mv = s == 0 ? -1.0f / 32.0f : 0.0f;   // the previous position is one render pixel to the left
            ps.evaluate(jx, jy, frame == 0, mv, 0.0f);
        }
        err[s] = error_vs_scene(ps, shift, 4);
        ps.d.n_finalize(ps.net());
    }
    std::printf("    panning scene, mean error after 24 frames: %.4f with the motion vectors, %.4f without\n", err[0], err[1]);
    CHECK(err[0] < 0.8 * err[1]);
}

// A reset drops the history: after frames of flat grey, a checkerboard of single texels (whose neighbourhood box still holds the grey, so
// clipping keeps the history) shows at once with the reset and only faintly without.
NXTEST(history_reset_is_honoured) {
    App app;
    App::Opts o;
    o.width = 64;
    o.height = 64;
    REQUIRE(app.init(o));
    double dev[2] = {};
    for (int s = 0; s < 2; ++s) {
        Pass ps(app, 32, 32, 64, 64);
        REQUIRE(ps.ok());
        REQUIRE(ps.init(0));
        ps.set_color(std::vector<float>(32 * 32 * 4, 0.5f));
        for (uint32_t frame = 0; frame < 12; ++frame) {
            float jx, jy;
            halton_jitter(frame, &jx, &jy);
            ps.evaluate(jx, jy, frame == 0);
        }
        std::vector<float> checker(32 * 32 * 4);
        for (int y = 0; y < 32; ++y)
            for (int x = 0; x < 32; ++x)
                for (int i = 0; i < 4; ++i) checker[(static_cast<size_t>(y) * 32 + x) * 4 + i] = ((x + y) & 1) ? 0.75f : 0.25f;
        ps.set_color(checker);
        ps.evaluate(0, 0, s == 0);
        double d = 0;
        for (int y = 0; y < 64; ++y)
            for (int x = 0; x < 64; ++x) {
                float px[4];
                ps.at(x, y, px);
                d += std::fabs(px[0] - 0.5f);
            }
        dev[s] = d / (64 * 64);
        ps.d.n_finalize(ps.net());
    }
    std::printf("    mean distance from grey after the checkerboard appears: %.4f with the reset, %.4f without\n", dev[0], dev[1]);
    CHECK(dev[0] > 0.2);
    CHECK(dev[1] < 0.5 * dev[0]);
}
