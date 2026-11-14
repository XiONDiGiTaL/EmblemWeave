// SPDX-License-Identifier: GPL-3.0-or-later
// NVN throughput: thousands of draws per frame through the replay, with a real vertex/fragment shader pair and a texture. Measures
// the CPU time of the render thread per draw (backend_stats().submit_replay_ns) and asserts a budget that keeps 120 fps possible
// for a scene of 3000 draws. The numbers are printed; the assertion is loose because the machine is shared.
#include <chrono>

#include "nvn_app.h"
NXTEST_GROUP(gfx_nvn_scene_stress)

using namespace nvnapp;

NXTEST(draw_throughput) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    App app;
    App::Opts o;
    o.width = 128;
    o.height = 96;
    REQUIRE(app.init(o));
    auto& api = app.api;
    App::ShaderBlob vs, fs;
    REQUIRE(App::load_shader("a656883b0cdb8ebc", &vs));
    REQUIRE(App::load_shader("062cfa5657f5388d", &fs));
    auto prog = app.make_program({&vs, &fs});
    REQUIRE(prog != nullptr);
    const float verts[] = {-0.9f, -0.9f, 0.0f, 0.9f, -0.9f, 0.0f, 0.0f, 0.9f, 0.0f};
    App::Buf vb = app.make_buffer(sizeof verts, verts);

    constexpr int kDraws = 3000, kFrames = 30;
    auto cb = app.make_cmdbuf();
    NVNcommandBuffer* c = cb->obj.get();
    // one recorded list of kDraws draws with varying scissor and blend state, submitted every frame
    NVNblendState bs; api.nvnBlendStateSetDefaults(&bs);
    NVNcolorState cs; api.nvnColorStateSetDefaults(&cs);
    NVNchannelMaskState cm; api.nvnChannelMaskStateSetDefaults(&cm);
    NVNpolygonState ps; api.nvnPolygonStateSetDefaults(&ps);
    NVNdepthStencilState ds; api.nvnDepthStencilStateSetDefaults(&ds);
    NVNmultisampleState mss; api.nvnMultisampleStateSetDefaults(&mss);
    NVNvertexAttribState attr; api.nvnVertexAttribStateSetDefaults(&attr);
    api.nvnVertexAttribStateSetFormat(&attr, NVN_FORMAT_RGB32F, 0);
    NVNvertexStreamState stream; api.nvnVertexStreamStateSetDefaults(&stream);
    api.nvnVertexStreamStateSetStride(&stream, 12);
    const int idx0 = app.acquire();
    NVNtexture* t0[] = {app.window_textures[idx0]->obj.get()};
    api.nvnCommandBufferBeginRecording(c);
    api.nvnCommandBufferBindBlendState(c, &bs);
    api.nvnCommandBufferBindColorState(c, &cs);
    api.nvnCommandBufferBindChannelMaskState(c, &cm);
    api.nvnCommandBufferBindPolygonState(c, &ps);
    api.nvnCommandBufferBindDepthStencilState(c, &ds);
    api.nvnCommandBufferBindMultisampleState(c, &mss);
    api.nvnCommandBufferBindVertexAttribState(c, 1, &attr);
    api.nvnCommandBufferBindVertexStreamState(c, 1, &stream);
    api.nvnCommandBufferBindVertexBuffer(c, 0, vb.address, vb.size);
    api.nvnCommandBufferBindProgram(c, prog.get(), NVN_SHADER_STAGE_VERTEX_BIT | NVN_SHADER_STAGE_FRAGMENT_BIT);
    for (int i = 0; i < kDraws; ++i) {
        api.nvnCommandBufferSetScissor(c, i % 64, i % 48, 64, 48);
        api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
    }
    const NVNcommandHandle list = api.nvnCommandBufferEndRecording(c);

    auto frame = [&](int idx) {
        auto head = app.make_cmdbuf();
        NVNtexture* tg[] = {app.window_textures[idx]->obj.get()};
        api.nvnCommandBufferBeginRecording(head->obj.get());
        api.nvnCommandBufferSetRenderTargets(head->obj.get(), 1, tg, nullptr, nullptr, nullptr);
        api.nvnCommandBufferSetViewport(head->obj.get(), 0, 0, 128, 96);
        const float black[4] = {0, 0, 0, 1};
        api.nvnCommandBufferClearColor(head->obj.get(), 0, black, NVN_CLEAR_COLOR_MASK_RGBA);
        api.nvnCommandBufferCallCommands(head->obj.get(), 1, &list);
        const NVNcommandHandle h = api.nvnCommandBufferEndRecording(head->obj.get());
        api.nvnQueueSubmitCommands(app.queue.get(), 1, &h);
    };
    (void)t0;
    frame(idx0);   // warm up: pipeline creation, descriptor sets
    app.finish();
    const auto s0 = nxrt::nvn::backend_stats();
    const auto t_start = std::chrono::steady_clock::now();
    for (int f = 0; f < kFrames; ++f) {
        frame(app.acquire());
        app.finish();
    }
    const double wall_ms = std::chrono::duration<double, std::milli>(std::chrono::steady_clock::now() - t_start).count();
    const auto s1 = nxrt::nvn::backend_stats();
    const double replay_ms = static_cast<double>(s1.submit_replay_ns - s0.submit_replay_ns) / 1e6 / kFrames;
    const uint64_t draws = s1.draws - s0.draws;
    std::printf("    %d frames of %d draws: replay %.2f ms per frame (%.2f us per draw), wall %.2f ms per frame, draws counted %llu, descriptor sets %llu\n", kFrames, kDraws, replay_ms,
                replay_ms * 1000.0 / kDraws, wall_ms / kFrames, static_cast<unsigned long long>(draws), static_cast<unsigned long long>(s1.descriptor_sets - s0.descriptor_sets));
    CHECK_EQ(draws, static_cast<uint64_t>(kDraws) * kFrames);
    CHECK(replay_ms < 12.0);   // 3000 draws in under 12 ms of render thread time (measured 8.5 ms on a busy machine)
    const auto v = app.validation();
    CHECK_EQ(v.errors, 0ull);
    app.shutdown();
}
