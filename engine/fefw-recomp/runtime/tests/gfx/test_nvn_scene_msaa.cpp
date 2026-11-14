// SPDX-License-Identifier: GPL-3.0-or-later
// NVN scene oracle: a 4x multisampled colour target, a triangle drawn into it with real shaders, resolved with
// nvnCommandBufferDownsample into the window texture. The game uses multisample targets (nvnTextureBuilderSetSamples) and Downsample.
// Edge pixels must come out with partial coverage (a value between black and magenta) which a single-sample render cannot produce.
#include "nvn_app.h"
NXTEST_GROUP(gfx_nvn_scene_msaa)

using namespace nvnapp;

NXTEST(msaa_triangle_resolves_with_partial_coverage) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    App app;
    App::Opts o;
    o.width = 64;
    o.height = 48;
    REQUIRE(app.init(o));
    auto& api = app.api;
    App::ShaderBlob vs, fs;
    REQUIRE(App::load_shader("a656883b0cdb8ebc", &vs));
    REQUIRE(App::load_shader("062cfa5657f5388d", &fs));
    auto prog = app.make_program({&vs, &fs});
    REQUIRE(prog != nullptr);
    // the 4x target
    auto ms = std::make_unique<Tex>();
    ms->obj = std::make_unique<NVNtexture>();
    std::memset(ms->obj.get(), 0, sizeof(NVNtexture));
    {
        NVNtextureBuilder b;
        api.nvnTextureBuilderSetDevice(&b, app.device.get());
        api.nvnTextureBuilderSetDefaults(&b);
        api.nvnTextureBuilderSetFlags(&b, NVN_TEXTURE_FLAGS_COMPRESSIBLE);
        api.nvnTextureBuilderSetTarget(&b, NVN_TEXTURE_TARGET_2D_MULTISAMPLE);
        api.nvnTextureBuilderSetSize2D(&b, 64, 48);
        api.nvnTextureBuilderSetFormat(&b, NVN_FORMAT_RGBA8);
        api.nvnTextureBuilderSetSamples(&b, 4);
        const size_t size = api.nvnTextureBuilderGetStorageSize(&b), align = api.nvnTextureBuilderGetStorageAlignment(&b);
        ms->pool = app.pool_main.get();
        ms->offset = ms->pool->take(size, std::max<size_t>(align, 256));
        api.nvnTextureBuilderSetStorage(&b, ms->pool->obj.get(), static_cast<ptrdiff_t>(ms->offset));
        REQUIRE(api.nvnTextureInitialize(ms->obj.get(), &b));
    }
    const float verts[] = {-0.5f, -0.5f, 0.0f, 0.5f, -0.5f, 0.0f, 0.0f, 0.5f, 0.0f};
    App::Buf vb = app.make_buffer(sizeof verts, verts);
    auto cb = app.make_cmdbuf();
    NVNcommandBuffer* c = cb->obj.get();
    const int idx = app.acquire();
    NVNtexture* targets[] = {ms->obj.get()};
    api.nvnCommandBufferBeginRecording(c);
    api.nvnCommandBufferSetRenderTargets(c, 1, targets, nullptr, nullptr, nullptr);
    api.nvnCommandBufferSetViewport(c, 0, 0, 64, 48);
    api.nvnCommandBufferSetScissor(c, 0, 0, 64, 48);
    const float black[4] = {0, 0, 0, 1};
    api.nvnCommandBufferClearColor(c, 0, black, NVN_CLEAR_COLOR_MASK_RGBA);
    NVNblendState bs; api.nvnBlendStateSetDefaults(&bs);
    NVNcolorState cs; api.nvnColorStateSetDefaults(&cs);
    NVNchannelMaskState cm; api.nvnChannelMaskStateSetDefaults(&cm);
    NVNpolygonState ps; api.nvnPolygonStateSetDefaults(&ps);
    NVNdepthStencilState ds; api.nvnDepthStencilStateSetDefaults(&ds);
    NVNmultisampleState mss; api.nvnMultisampleStateSetDefaults(&mss);
    api.nvnMultisampleStateSetMultisampleEnable(&mss, 1);
    api.nvnMultisampleStateSetSamples(&mss, 4);
    api.nvnCommandBufferBindBlendState(c, &bs);
    api.nvnCommandBufferBindColorState(c, &cs);
    api.nvnCommandBufferBindChannelMaskState(c, &cm);
    api.nvnCommandBufferBindPolygonState(c, &ps);
    api.nvnCommandBufferBindDepthStencilState(c, &ds);
    api.nvnCommandBufferBindMultisampleState(c, &mss);
    NVNvertexAttribState attr; api.nvnVertexAttribStateSetDefaults(&attr);
    api.nvnVertexAttribStateSetFormat(&attr, NVN_FORMAT_RGB32F, 0);
    NVNvertexStreamState stream; api.nvnVertexStreamStateSetDefaults(&stream);
    api.nvnVertexStreamStateSetStride(&stream, 12);
    api.nvnCommandBufferBindVertexAttribState(c, 1, &attr);
    api.nvnCommandBufferBindVertexStreamState(c, 1, &stream);
    api.nvnCommandBufferBindVertexBuffer(c, 0, vb.address, vb.size);
    api.nvnCommandBufferBindProgram(c, prog.get(), NVN_SHADER_STAGE_VERTEX_BIT | NVN_SHADER_STAGE_FRAGMENT_BIT);
    api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
    api.nvnCommandBufferDownsample(c, ms->obj.get(), app.window_textures[idx]->obj.get());
    app.end_and_submit(*cb);
    std::vector<uint8_t> px;
    REQUIRE(app.present_and_capture(idx, &px));
    size_t full = 0, partial = 0, black_px = 0;
    for (size_t i = 0; i < px.size(); i += 4) {
        if (px[i] == 255 && px[i + 1] == 0 && px[i + 2] == 255) ++full;
        else if (px[i] == 0 && px[i + 1] == 0 && px[i + 2] == 0) ++black_px;
        else if (px[i] > 0 && px[i] < 255 && px[i] == px[i + 2] && px[i + 1] == 0) ++partial;
    }
    std::printf("    full magenta %zu, partial coverage %zu, black %zu of %zu\n", full, partial, black_px, px.size() / 4);
    CHECK(full > 300 && full < 400);
    CHECK(partial >= 20);                       // the two slanted edges (32 measured)
    CHECK_EQ(full + partial + black_px, px.size() / 4);
    const auto v = app.validation();
    std::printf("    validation: %llu errors %s\n", static_cast<unsigned long long>(v.errors), v.first_error.c_str());
    CHECK_EQ(v.errors, 0ull);
    app.shutdown();
}
