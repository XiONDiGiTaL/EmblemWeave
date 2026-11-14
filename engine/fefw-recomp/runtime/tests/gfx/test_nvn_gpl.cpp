// SPDX-License-Identifier: GPL-3.0-or-later
// Pipeline libraries end to end (src/gfx/nvn/nvn_gpl.cpp): with [nvn] gpl on, the two shader parts of the triangle's program are built by
// the workers when its shaders are set, and the draw links them with the other two parts; with gpl = 0 no part is made and the draw builds
// the whole pipeline; both ways draw the same pixels. The scene is the triangle of test_nvn_scene_triangle (two real translated shaders,
// skipped when work/shader_cache is not on this machine).
#include <algorithm>
#include <chrono>
#include <cstdio>
#include <filesystem>
#include <string>
#include <system_error>
#include <thread>
#include <vector>

#include "nvn_app.h"
#include "nvn_pipeline.h"
#include "nxrt/config.h"
NXTEST_GROUP(gfx_nvn_gpl)

using namespace nvnapp;
namespace fs = std::filesystem;
namespace gpu = nxrt::nvn::be::gpu;
using namespace std::chrono_literals;

namespace {

struct Rgba { int r, g, b, a; };
Rgba at(const std::vector<uint8_t>& px, uint32_t w, uint32_t x, uint32_t y) {
    const uint8_t* p = &px[(static_cast<size_t>(y) * w + x) * 4];
    return {p[0], p[1], p[2], p[3]};
}

// A folder for the key and cache files, removed when the case ends (the App of the case is torn down first: declare it after).
struct TempDir {
    fs::path path;
    TempDir() {
        const auto stamp = std::chrono::steady_clock::now().time_since_epoch().count();
        path = fs::temp_directory_path() / ("nxrt_gfx_gpl_" + std::to_string(GetCurrentProcessId()) + "_" + std::to_string(stamp));
        std::error_code ec;
        fs::create_directories(path, ec);
    }
    ~TempDir() {
        std::error_code ec;
        fs::remove_all(path, ec);
    }
};

// Points the [nvn] prewarm settings at the given files for one case. Restores the defaults when the case ends, on any return.
struct PrewarmConfig {
    std::string keys, cache;
    PrewarmConfig(const fs::path& k, const fs::path& c)
        : keys(nxrt::config_get_str("nvn", "pipeline_keys", "pipeline_keys.bin")),
          cache(nxrt::config_get_str("nvn", "pipeline_cache", "pipeline_cache.bin")) {
        const std::string ks = k.string(), cs = c.string();
        nxrt::config_set("nvn", "pipeline_keys", ks.c_str());
        nxrt::config_set("nvn", "pipeline_cache", cs.c_str());
        nxrt::config_set("nvn", "prewarm", "1");
    }
    ~PrewarmConfig() {
        nxrt::config_set("nvn", "pipeline_keys", keys.c_str());
        nxrt::config_set("nvn", "pipeline_cache", cache.c_str());
        nxrt::config_set("nvn", "prewarm", "1");
    }
};

// Sets [nvn] gpl for one case (or one run of a case). Puts it back to "1" (the default) when that scope ends, on any return.
struct GplConfig {
    explicit GplConfig(const char* value) { nxrt::config_set("nvn", "gpl", value); }
    ~GplConfig() { nxrt::config_set("nvn", "gpl", "1"); }
};

// The pixel checks of test_nvn_scene_triangle.
void check_triangle(const std::vector<uint8_t>& px) {
    const Rgba inside = at(px, 64, 32, 28), above = at(px, 64, 32, 6), below = at(px, 64, 32, 42), left = at(px, 64, 8, 28);
    std::printf("    inside %d %d %d, above apex %d %d %d, below base %d %d %d, far left %d %d %d\n", inside.r, inside.g, inside.b, above.r, above.g, above.b,
                below.r, below.g, below.b, left.r, left.g, left.b);
    CHECK(inside.r == 255 && inside.g == 0 && inside.b == 255);
    CHECK(above.r == 0 && above.g == 0 && above.b == 0);
    CHECK(below.r == 0 && below.g == 0 && below.b == 0);
    CHECK(left.r == 0 && left.g == 0 && left.b == 0);
    size_t magenta = 0;
    for (size_t i = 0; i < px.size(); i += 4)
        if (px[i] == 255 && px[i + 1] == 0 && px[i + 2] == 255) ++magenta;
    std::printf("    magenta pixels %zu (base 32 px x height 24 px / 2 = 384)\n", magenta);
    CHECK(magenta > 360 && magenta < 410);
}

// Waits until the workers have finished every shader-part job queued so far, so the draw links finished parts instead of racing the workers
// for them. Counting finished parts is not enough: the program has one fragment part and one pre-rasterisation part for each raster variant
// (jobs 5 here), and the first two to finish may not be the two the draw needs. The queue and the running count are read together under the
// lock, so no job can be in between. 60 s is a safety stop for a stuck worker; the checks after the draw report it.
void wait_for_parts(uint64_t /*unused: kept so the call sites read the same*/) {
    const auto t0 = std::chrono::steady_clock::now();
    for (;;) {
        const gpu::GplStats s = gpu::gpl_stats();
        if (s.jobs > 0 && s.jobs_pending == 0) return;
        if (std::chrono::steady_clock::now() - t0 > 60s) return;
        std::this_thread::sleep_for(5ms);
    }
}

void print_gpl(const char* what, const gpu::GplStats& s) {
    std::printf("    %s: linked %llu (%.1f ms), link failures %llu, parts built ahead %llu (%.1f ms on workers), parts made at the draw %llu "
                "(%.1f ms), waited for %llu (%.1f ms), parts hit %llu, jobs %llu (%llu skipped)\n",
                what, static_cast<unsigned long long>(s.linked), s.link_ms, static_cast<unsigned long long>(s.link_failed),
                static_cast<unsigned long long>(s.parts_ahead), s.worker_ms, static_cast<unsigned long long>(s.parts_at_draw), s.draw_part_ms,
                static_cast<unsigned long long>(s.parts_waited), s.wait_ms, static_cast<unsigned long long>(s.parts_hit),
                static_cast<unsigned long long>(s.jobs), static_cast<unsigned long long>(s.jobs_skipped));
}

// The triangle of test_nvn_scene_triangle on `app`: program, vertex data, one draw, frame captured. When `parts_wait` is not 0 the draw
// waits for that many shader parts after the program is made. False when a setup step fails.
bool draw_triangle(App& app, std::vector<uint8_t>* px, uint64_t parts_wait) {
    App::ShaderBlob vs, fsh;
    if (!App::load_shader("a656883b0cdb8ebc", &vs)) return false;   // vertex: gl_Position.xyz = attribute 0
    if (!App::load_shader("062cfa5657f5388d", &fsh)) return false;  // fragment: colour (1, 0, 1, 1)
    auto prog = app.make_program({&vs, &fsh});
    if (!prog) return false;
    if (parts_wait != 0) wait_for_parts(parts_wait);

    const float verts[] = {-0.5f, -0.5f, 0.0f, 0.5f, -0.5f, 0.0f, 0.0f, 0.5f, 0.0f};
    App::Buf vb = app.make_buffer(sizeof verts, verts);

    auto cb = app.make_cmdbuf();
    if (!cb) return false;
    NVNcommandBuffer* c = cb->obj.get();
    auto& api = app.api;
    const int idx = app.acquire();
    NVNtexture* targets[] = {app.window_textures[idx]->obj.get()};
    api.nvnCommandBufferBeginRecording(c);
    api.nvnCommandBufferSetRenderTargets(c, 1, targets, nullptr, nullptr, nullptr);
    api.nvnCommandBufferSetViewport(c, 0, 0, 64, 48);
    api.nvnCommandBufferSetScissor(c, 0, 0, 64, 48);
    const float black[4] = {0, 0, 0, 1};
    api.nvnCommandBufferClearColor(c, 0, black, NVN_CLEAR_COLOR_MASK_RGBA);

    NVNblendState bs;       api.nvnBlendStateSetDefaults(&bs);
    NVNcolorState cs;       api.nvnColorStateSetDefaults(&cs);
    NVNchannelMaskState ms; api.nvnChannelMaskStateSetDefaults(&ms);
    NVNpolygonState ps;     api.nvnPolygonStateSetDefaults(&ps);
    NVNdepthStencilState ds; api.nvnDepthStencilStateSetDefaults(&ds);
    NVNmultisampleState mss; api.nvnMultisampleStateSetDefaults(&mss);
    api.nvnCommandBufferBindBlendState(c, &bs);
    api.nvnCommandBufferBindColorState(c, &cs);
    api.nvnCommandBufferBindChannelMaskState(c, &ms);
    api.nvnCommandBufferBindPolygonState(c, &ps);
    api.nvnCommandBufferBindDepthStencilState(c, &ds);
    api.nvnCommandBufferBindMultisampleState(c, &mss);

    NVNvertexAttribState attr;
    api.nvnVertexAttribStateSetDefaults(&attr);
    api.nvnVertexAttribStateSetFormat(&attr, NVN_FORMAT_RGB32F, 0);
    api.nvnVertexAttribStateSetStreamIndex(&attr, 0);
    NVNvertexStreamState stream;
    api.nvnVertexStreamStateSetDefaults(&stream);
    api.nvnVertexStreamStateSetStride(&stream, 12);
    api.nvnCommandBufferBindVertexAttribState(c, 1, &attr);
    api.nvnCommandBufferBindVertexStreamState(c, 1, &stream);
    api.nvnCommandBufferBindVertexBuffer(c, 0, vb.address, vb.size);
    api.nvnCommandBufferBindProgram(c, prog.get(), NVN_SHADER_STAGE_VERTEX_BIT | NVN_SHADER_STAGE_FRAGMENT_BIT);
    api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
    app.end_and_submit(*cb);

    return app.present_and_capture(idx, px);
}

// One run of the triangle with [nvn] gpl set to `gpl` and prewarm off, with its own key-file folder (so no run can read the files of
// another). `active` says whether the pipeline libraries were on for the run. False when a setup step fails.
bool render_once(const char* gpl, uint64_t parts_wait, std::vector<uint8_t>* px, bool* active) {
    TempDir td;
    const fs::path keys = td.path / "pipeline_keys.bin", cache = td.path / "pipeline_cache.bin";
    PrewarmConfig cfg(keys, cache);            // ends with prewarm = 1
    nxrt::config_set("nvn", "prewarm", "0");
    GplConfig g(gpl);                          // ends with gpl = 1
    App::Opts o;
    o.width = 64;
    o.height = 48;

    App app;
    if (!app.init(o)) return false;
    *active = gpu::gpl_active();
    const bool ok = draw_triangle(app, px, *active ? parts_wait : 0);
    CHECK_EQ(app.validation().errors, 0ull);
    app.shutdown();
    return ok;
}

}  // namespace

// gpl on, no key file yet: the program's two shader parts are built by the workers when its shaders are set, and the draw links them
// with the vertex input and fragment output parts it makes itself.
NXTEST(triangle_through_libraries) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    TempDir td;
    const fs::path keys = td.path / "pipeline_keys.bin", cache = td.path / "pipeline_cache.bin";
    PrewarmConfig cfg(keys, cache);
    GplConfig gpl("1");
    App::Opts o;
    o.width = 64;
    o.height = 48;

    App app;
    REQUIRE(app.init(o));
    if (!gpu::gpl_active()) {
        app.shutdown();
        nxtest::skip("pipeline libraries not supported on this GPU");
        return;
    }
    std::vector<uint8_t> px;
    REQUIRE(draw_triangle(app, &px, 2));
    check_triangle(px);
    const auto v = app.validation();
    CHECK_EQ(v.errors, 0ull);
    const gpu::GplStats s = gpu::gpl_stats();
    print_gpl("gpl on", s);
    CHECK(s.linked >= 1);
    CHECK(s.link_failed == 0);
    CHECK(s.parts_ahead >= 2);
    // The draw makes the vertex input and fragment output parts itself. It may also make the pre-rasterisation part of its raster flags:
    // the workers only know the default flags, the draw teaches them its own (learn_raster) and queues that part for the workers, so
    // the draw and a worker race for it (the draw waits for the worker, or makes it first). Never more than those three.
    CHECK(s.parts_at_draw <= 3);
    CHECK(s.parts_at_draw >= 2);
    app.shutdown();
}

// gpl = 0: no part is made, the draw builds the whole pipeline as it did before the libraries existed.
NXTEST(whole_pipeline_when_off) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    TempDir td;
    const fs::path keys = td.path / "pipeline_keys.bin", cache = td.path / "pipeline_cache.bin";
    PrewarmConfig cfg(keys, cache);
    GplConfig gpl("0");                        // ends with gpl = 1
    App::Opts o;
    o.width = 64;
    o.height = 48;

    App app;
    REQUIRE(app.init(o));
    std::vector<uint8_t> px;
    REQUIRE(draw_triangle(app, &px, 0));
    check_triangle(px);
    const auto v = app.validation();
    CHECK_EQ(v.errors, 0ull);
    const gpu::GplStats s = gpu::gpl_stats();
    print_gpl("gpl off", s);
    CHECK(s.linked == 0);
    CHECK(s.parts_ahead == 0);
    app.shutdown();
}

// The same triangle with the libraries on and off, prewarm off in both, so only the gpl setting differs: the two frames are byte-identical.
NXTEST(same_picture_both_ways) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    std::vector<uint8_t> on, off;
    bool on_active = false, off_active = true;
    REQUIRE(render_once("1", 2, &on, &on_active));
    if (!on_active) {
        nxtest::skip("pipeline libraries not supported on this GPU");
        return;
    }
    REQUIRE(render_once("0", 0, &off, &off_active));
    CHECK(!off_active);
    std::printf("    gpl on: pixels %zu bytes; gpl off: pixels %zu bytes\n", on.size(), off.size());
    check_triangle(on);
    check_triangle(off);
    const bool same = on.size() == off.size() && std::equal(on.begin(), on.end(), off.begin());
    if (!same) {
        size_t differ = 0;
        const size_t n = std::min(on.size(), off.size());
        for (size_t i = 0; i < n; ++i)
            if (on[i] != off[i]) ++differ;
        std::printf("    the two frames differ in %zu bytes\n", differ);
    }
    CHECK(same);
}

namespace {

// Sets [nvn] gpl_upgrade for one case; the default ("whole") comes back when the case ends.
struct UpgradeConfig {
    explicit UpgradeConfig(const char* value) { nxrt::config_set("nvn", "gpl_upgrade", value); }
    ~UpgradeConfig() { nxrt::config_set("nvn", "gpl_upgrade", "whole"); }
};

// The triangle through the libraries, then again after the optimised replacement was built: the second draw finds it, uses it and draws the same picture.
void upgrade_case(const char* mode) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    TempDir td;
    const fs::path keys = td.path / "pipeline_keys.bin", cache = td.path / "pipeline_cache.bin";
    PrewarmConfig cfg(keys, cache);
    GplConfig gpl("1");
    UpgradeConfig up(mode);
    App::Opts o;
    o.width = 64;
    o.height = 48;

    App app;
    REQUIRE(app.init(o));
    if (!gpu::gpl_active()) {
        app.shutdown();
        nxtest::skip("pipeline libraries not supported on this GPU");
        return;
    }
    std::vector<uint8_t> px;
    REQUIRE(draw_triangle(app, &px, 2));
    check_triangle(px);
    const auto t0 = std::chrono::steady_clock::now();
    while (gpu::gpl_stats().upgrades_done + gpu::gpl_stats().upgrades_failed < 1 && std::chrono::steady_clock::now() - t0 < 20s) std::this_thread::sleep_for(10ms);
    gpu::GplStats s = gpu::gpl_stats();
    print_gpl("after the first draw", s);
    CHECK(s.upgrades_queued >= 1);
    CHECK(s.upgrades_done >= 1);
    CHECK(s.upgrades_failed == 0);
    std::vector<uint8_t> px2;
    REQUIRE(draw_triangle(app, &px2, 0));
    check_triangle(px2);
    s = gpu::gpl_stats();
    std::printf("    upgrades queued %llu, built %llu (%.1f ms), taken %llu\n", static_cast<unsigned long long>(s.upgrades_queued), static_cast<unsigned long long>(s.upgrades_done),
                s.upgrade_ms, static_cast<unsigned long long>(s.upgrades_taken));
    CHECK(s.upgrades_taken >= 1);
    CHECK_EQ(app.validation().errors, 0ull);
    CHECK(std::equal(px.begin(), px.end(), px2.begin()));
    app.shutdown();
}

}  // namespace

// gpl_upgrade = whole: a background thread builds the whole pipeline of a linked one and the next lookup takes it.
NXTEST(linked_pipeline_is_replaced_by_whole) { upgrade_case("whole"); }

// gpl_upgrade = lto: the same, with the parts linked again with link-time optimisation.
NXTEST(linked_pipeline_is_replaced_by_lto_link) { upgrade_case("lto"); }

// gpl_upgrade = off: the linked pipeline stays.
NXTEST(no_upgrade_when_off) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    TempDir td;
    const fs::path keys = td.path / "pipeline_keys.bin", cache = td.path / "pipeline_cache.bin";
    PrewarmConfig cfg(keys, cache);
    GplConfig gpl("1");
    UpgradeConfig up("off");
    App::Opts o;
    o.width = 64;
    o.height = 48;
    App app;
    REQUIRE(app.init(o));
    if (!gpu::gpl_active()) {
        app.shutdown();
        nxtest::skip("pipeline libraries not supported on this GPU");
        return;
    }
    std::vector<uint8_t> px;
    REQUIRE(draw_triangle(app, &px, 2));
    check_triangle(px);
    const gpu::GplStats s = gpu::gpl_stats();
    CHECK(s.linked >= 1);
    CHECK_EQ(s.upgrades_queued, 0ull);
    app.shutdown();
}
