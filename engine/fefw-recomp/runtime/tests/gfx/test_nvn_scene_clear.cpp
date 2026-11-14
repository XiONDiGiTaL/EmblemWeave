// SPDX-License-Identifier: GPL-3.0-or-later
// NVN scene oracle 1: clears through the NVN API (device, queue, pools, window texture, command buffer), presents and reads the
// frame back. The first end-to-end test of the back end: the pixels must be exactly what the game asked for.
#include "nvn_app.h"
NXTEST_GROUP(gfx_nvn_scene_clear)

using namespace nvnapp;

NXTEST(clear_color_reaches_the_screen) {
    App app;
    App::Opts o;
    o.width = 64;
    o.height = 48;
    REQUIRE(app.init(o));
    auto cb = app.make_cmdbuf();
    REQUIRE(cb != nullptr);
    const int idx = app.acquire();
    REQUIRE(idx >= 0);
    NVNtexture* targets[] = {app.window_textures[idx]->obj.get()};
    app.api.nvnCommandBufferBeginRecording(cb->obj.get());
    app.api.nvnCommandBufferSetRenderTargets(cb->obj.get(), 1, targets, nullptr, nullptr, nullptr);
    app.api.nvnCommandBufferSetViewport(cb->obj.get(), 0, 0, 64, 48);
    app.api.nvnCommandBufferSetScissor(cb->obj.get(), 0, 0, 64, 48);
    const float red[4] = {1.0f, 0.0f, 0.25f, 1.0f};
    app.api.nvnCommandBufferClearColor(cb->obj.get(), 0, red, NVN_CLEAR_COLOR_MASK_RGBA);
    app.end_and_submit(*cb);
    std::vector<uint8_t> px;
    REQUIRE(app.present_and_capture(idx, &px));
    size_t wrong = 0;
    for (size_t i = 0; i < px.size(); i += 4)
        if (px[i] != 255 || px[i + 1] != 0 || std::abs(px[i + 2] - 64) > 1) ++wrong;
    std::printf("    frame %ux%u first pixel %d %d %d %d, %zu of %zu pixels wrong\n", app.width, app.height, px[0], px[1], px[2], px[3], wrong, px.size() / 4);
    CHECK_EQ(wrong, 0u);
    const auto v = app.validation();
    std::printf("    validation: %llu errors, %llu warnings %s\n", static_cast<unsigned long long>(v.errors), static_cast<unsigned long long>(v.warnings), v.first_error.c_str());
    CHECK_EQ(v.errors, 0ull);
    const auto st = nxrt::nvn::backend_stats();
    std::printf("    submits %llu passes %llu clears %llu presents %llu pools %llu\n", static_cast<unsigned long long>(st.submits), static_cast<unsigned long long>(st.passes),
                static_cast<unsigned long long>(st.clears), static_cast<unsigned long long>(st.presents), static_cast<unsigned long long>(st.pools_imported));
    CHECK(st.clears >= 1);
    app.shutdown();
}
