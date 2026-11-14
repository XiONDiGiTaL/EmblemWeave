// SPDX-License-Identifier: GPL-3.0-or-later
// Pipeline prewarm end to end (src/gfx/nvn/nvn_prewarm.cpp): the key file is written by a first run and read back by a second one,
// whose draw takes the pipeline that a worker built ahead; a disabled prewarm writes nothing; a key file of another layout is
// ignored; the pipeline cache is written while the process is still running. The scene is the triangle of
// test_nvn_scene_triangle (two real translated shaders, skipped when work/shader_cache is not on this machine).
#include <chrono>
#include <cstdio>
#include <filesystem>
#include <fstream>
#include <string>
#include <system_error>
#include <thread>
#include <vector>

#include "nvn_app.h"
#include "nvn_pipeline.h"
#include "nxrt/config.h"
NXTEST_GROUP(gfx_nvn_prewarm)

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
        path = fs::temp_directory_path() / ("nxrt_gfx_prewarm_" + std::to_string(GetCurrentProcessId()) + "_" + std::to_string(stamp));
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

// The prewarm counters are process-wide and prewarm_shutdown does not reset them, so a case reads what changed while it ran.
// `loaded` is assigned by the key file load (not counted), so it is returned as it stands.
gpu::PrewarmStats since(const gpu::PrewarmStats& from) {
    gpu::PrewarmStats d = gpu::prewarm_stats();
    d.recorded -= from.recorded;
    d.queued -= from.queued;
    d.built -= from.built;
    d.taken -= from.taken;
    d.waited -= from.waited;
    d.claimed -= from.claimed;
    d.wait_ms -= from.wait_ms;
    return d;
}

void print_stats(const char* what, const gpu::PrewarmStats& s) {
    std::printf("    %s: recorded %llu, loaded %llu, queued %llu, built %llu, taken %llu (waited %llu, %.1f ms), claimed %llu\n", what,
                static_cast<unsigned long long>(s.recorded), static_cast<unsigned long long>(s.loaded), static_cast<unsigned long long>(s.queued),
                static_cast<unsigned long long>(s.built), static_cast<unsigned long long>(s.taken), static_cast<unsigned long long>(s.waited), s.wait_ms,
                static_cast<unsigned long long>(s.claimed));
}

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

// Waits (up to 6 s) until the prewarm workers have built every pipeline that the program's SetShaders queued (counted from `from`,
// the counters taken just before make_program). The draw then takes them instead of racing the workers for them.
void wait_for_queued_builds(const gpu::PrewarmStats& from) {
    const auto t0 = std::chrono::steady_clock::now();
    for (;;) {
        const gpu::PrewarmStats now = gpu::prewarm_stats();
        if (now.built - from.built >= now.queued - from.queued) return;
        if (std::chrono::steady_clock::now() - t0 > 6s) return;     // the checks after the draw say what happened
        std::this_thread::sleep_for(10ms);
    }
}

// The triangle of test_nvn_scene_triangle on `app`: program, vertex data, one draw, frame captured. False when a setup step fails.
bool draw_triangle(App& app, std::vector<uint8_t>* px) {
    App::ShaderBlob vs, fsh;
    if (!App::load_shader("a656883b0cdb8ebc", &vs)) return false;   // vertex: gl_Position.xyz = attribute 0
    if (!App::load_shader("062cfa5657f5388d", &fsh)) return false;  // fragment: colour (1, 0, 1, 1)
    const gpu::PrewarmStats before = gpu::prewarm_stats();
    auto prog = app.make_program({&vs, &fsh});
    if (!prog) return false;
    wait_for_queued_builds(before);

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

uintmax_t file_bytes(const fs::path& p, bool* exists) {
    std::error_code ec;
    const bool e = fs::exists(p, ec);
    if (exists) *exists = e;
    const uintmax_t n = e ? fs::file_size(p, ec) : 0;
    return ec ? 0 : n;
}

}  // namespace

// Run 1 records the triangle's pipeline into the key file; run 2 loads it, its SetShaders queues the pipeline on a worker and its draw
// takes the finished one.
NXTEST(key_file_written_and_reloaded) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    TempDir td;
    const fs::path keys = td.path / "pipeline_keys.bin", cache = td.path / "pipeline_cache.bin";
    PrewarmConfig cfg(keys, cache);
    App::Opts o;
    o.width = 64;
    o.height = 48;

    {
        App app;
        REQUIRE(app.init(o));
        std::vector<uint8_t> px;
        REQUIRE(draw_triangle(app, &px));
        check_triangle(px);
        const auto v = app.validation();
        CHECK_EQ(v.errors, 0ull);
        app.shutdown();
    }
    bool exists = false;
    const uintmax_t keys_bytes = file_bytes(keys, &exists);
    std::printf("    run 1: key file %llu bytes\n", static_cast<unsigned long long>(keys_bytes));
    CHECK(exists);
    CHECK(keys_bytes > 16);

    {
        App app;
        REQUIRE(app.init(o));
        const gpu::PrewarmStats before = gpu::prewarm_stats();
        std::vector<uint8_t> px;
        REQUIRE(draw_triangle(app, &px));
        check_triangle(px);
        const auto v = app.validation();
        CHECK_EQ(v.errors, 0ull);
        const gpu::PrewarmStats now = gpu::prewarm_stats();
        const gpu::PrewarmStats d = since(before);
        print_stats("run 2 (counters since its start)", d);
        CHECK(now.loaded >= 1);
        CHECK(d.queued >= 1);
        CHECK(d.built >= 1);
        CHECK(d.taken >= 1);
        CHECK(d.claimed + d.taken >= d.queued);
        CHECK_EQ(d.recorded, 0ull);
        app.shutdown();
    }
}

// With [nvn] prewarm = 0 the scene renders as usual, nothing is recorded and no key file is made.
NXTEST(disabled_by_config) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    TempDir td;
    const fs::path keys = td.path / "pipeline_keys.bin", cache = td.path / "pipeline_cache.bin";
    PrewarmConfig cfg(keys, cache);            // ends with prewarm = 1
    nxrt::config_set("nvn", "prewarm", "0");
    App::Opts o;
    o.width = 64;
    o.height = 48;

    App app;
    REQUIRE(app.init(o));
    const gpu::PrewarmStats before = gpu::prewarm_stats();
    std::vector<uint8_t> px;
    REQUIRE(draw_triangle(app, &px));
    check_triangle(px);
    const auto v = app.validation();
    CHECK_EQ(v.errors, 0ull);
    const gpu::PrewarmStats now = gpu::prewarm_stats();
    const gpu::PrewarmStats d = since(before);
    print_stats("disabled (counters since its start)", d);
    CHECK_EQ(d.recorded, 0ull);
    CHECK_EQ(d.queued, 0ull);
    CHECK_EQ(d.built, 0ull);
    CHECK_EQ(d.taken, 0ull);
    CHECK_EQ(d.waited, 0ull);
    CHECK_EQ(d.claimed, 0ull);
    CHECK(d.wait_ms == 0.0);
    CHECK(now.loaded == before.loaded);       // no key file is read either
    bool exists = true;
    file_bytes(keys, &exists);
    CHECK(!exists);
    app.shutdown();
}

// A key file of another record layout (the magic, then size 7 and 0) is ignored: the scene renders and nothing is loaded from it.
NXTEST(key_file_with_wrong_layout_is_ignored) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    TempDir td;
    const fs::path keys = td.path / "pipeline_keys.bin", cache = td.path / "pipeline_cache.bin";
    PrewarmConfig cfg(keys, cache);
    {
        std::ofstream f(keys, std::ios::binary);
        const char magic[8] = {'N', 'X', 'P', 'K', 'E', 'Y', 'S', '1'};
        const uint32_t size = 7, pad = 0;
        f.write(magic, 8);
        f.write(reinterpret_cast<const char*>(&size), 4);
        f.write(reinterpret_cast<const char*>(&pad), 4);
    }
    App::Opts o;
    o.width = 64;
    o.height = 48;

    App app;
    REQUIRE(app.init(o));
    const gpu::PrewarmStats before = gpu::prewarm_stats();
    std::vector<uint8_t> px;
    REQUIRE(draw_triangle(app, &px));
    check_triangle(px);
    const auto v = app.validation();
    CHECK_EQ(v.errors, 0ull);
    const gpu::PrewarmStats now = gpu::prewarm_stats();
    print_stats("wrong layout (counters since its start)", since(before));
    CHECK(now.loaded == before.loaded);       // the file was not read: nothing loaded from it
    app.shutdown();
}

// The pipeline cache is written by the saver thread while the process runs (it wakes every 2 s and saves when a pipeline is new).
NXTEST(cache_saved_while_running) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    TempDir td;
    const fs::path keys = td.path / "pipeline_keys.bin", cache = td.path / "pipeline_cache.bin";
    PrewarmConfig cfg(keys, cache);
    App::Opts o;
    o.width = 64;
    o.height = 48;

    App app;
    REQUIRE(app.init(o));
    std::vector<uint8_t> px;
    REQUIRE(draw_triangle(app, &px));
    check_triangle(px);

    // Still running: the app is not shut down while the file is awaited.
    const auto t0 = std::chrono::steady_clock::now();
    bool appeared = false;
    uintmax_t size = 0;
    for (int i = 0; i <= 60 && !appeared; ++i) {
        std::error_code ec;
        const uintmax_t n = fs::file_size(cache, ec);
        if (!ec && n > 0) {
            appeared = true;
            size = n;
            break;
        }
        std::this_thread::sleep_for(100ms);
    }
    const double ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t0).count();
    std::printf("    pipeline cache %s after %.0f ms, %llu bytes\n", appeared ? "appeared" : "did not appear", ms, static_cast<unsigned long long>(size));
    CHECK(appeared);
    CHECK(size > 0);
    app.shutdown();
}
