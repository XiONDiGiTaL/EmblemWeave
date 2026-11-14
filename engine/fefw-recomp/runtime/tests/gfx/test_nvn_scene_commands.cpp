// SPDX-License-Identifier: GPL-3.0-or-later
// NVN scene oracles, commands (S4d). Seven scenes that drive the command side of the NVN back end through the function
// pointers a game gets, with real translated shaders of the game read from work/shader_cache at run time (skipped when that data
// is not on this machine). Ordered by how likely they are to pass: window crop and acquire order, two queues, command handles,
// fences/events/reports, uniform buffers, many frames, compute. A failing check is a finding: the assertion states the correct result.
#include "nvn_app.h"

#include <algorithm>
#include <chrono>
#include <cmath>
#include <cstdlib>
NXTEST_GROUP(gfx_nvn_scene_commands)

using namespace nvnapp;

namespace {

constexpr int kW = 64, kH = 48;
// Position-only vertex shader a656883b0cdb8ebc (gl_Position.xyz = attribute 0) with the constant-magenta fragment shader 062cfa5657f5388d.
constexpr float kTri[9] = {-0.5f, -0.5f, 0.0f, 0.5f, -0.5f, 0.0f, 0.0f, 0.5f, 0.0f};
const float kBlack[4] = {0, 0, 0, 1}, kBlue[4] = {0, 0, 1, 1}, kYellow[4] = {1, 1, 0, 1}, kGreen[4] = {0, 1, 0, 1};

const uint8_t* px_at(const std::vector<uint8_t>& px, int x, int y) { return &px[(static_cast<size_t>(y) * kW + x) * 4]; }

bool near_color(const std::vector<uint8_t>& px, int x, int y, int r, int g, int b, int tol) {
    const uint8_t* p = px_at(px, x, y);
    return std::abs(p[0] - r) <= tol && std::abs(p[1] - g) <= tol && std::abs(p[2] - b) <= tol;
}

size_t count_color(const std::vector<uint8_t>& px, int r, int g, int b, int tol = 0) {
    size_t n = 0;
    for (size_t i = 0; i < px.size(); i += 4)
        if (std::abs(px[i] - r) <= tol && std::abs(px[i + 1] - g) <= tol && std::abs(px[i + 2] - b) <= tol) ++n;
    return n;
}

// Pixels of exactly magenta (255, 0, 255): their count and the mean x index (the triangle is centred on x = 31.5 in index space).
struct Census {
    size_t magenta = 0;
    double cx = 0;
};
Census magenta_census(const std::vector<uint8_t>& px) {
    Census c;
    double sum = 0;
    for (int y = 0; y < kH; ++y)
        for (int x = 0; x < kW; ++x) {
            const uint8_t* p = px_at(px, x, y);
            if (p[0] == 255 && p[1] == 0 && p[2] == 255) {
                ++c.magenta;
                sum += x;
            }
        }
    c.cx = c.magenta ? sum / static_cast<double>(c.magenta) : 0;
    return c;
}

void check_validation(App& app) {
    const auto v = app.validation();
    std::printf("    validation: %llu errors, %llu warnings %s\n", static_cast<unsigned long long>(v.errors), static_cast<unsigned long long>(v.warnings), v.first_error.c_str());
    CHECK_EQ(v.errors, 0ull);
}

// Writes the frame to work/nvn_refs/<scene>.png when NXRT_NVN_UPDATE_REFS=1, otherwise compares it with that file when it exists.
void check_reference(const char* scene, const std::vector<uint8_t>& px) {
    const std::string path = nxtest::project_path("work/nvn_refs/") + scene + ".png";
    const char* upd = std::getenv("NXRT_NVN_UPDATE_REFS");
    if (upd && std::string(upd) == "1") {
        CHECK(nxrt::gfx::save_png_rgba(path, px.data(), kW, kH));
        std::printf("    reference written: %s\n", path.c_str());
        return;
    }
    std::vector<uint8_t> ref;
    uint32_t rw = 0, rh = 0;
    if (!nxrt::gfx::load_png_rgba(path, &ref, &rw, &rh)) {
        std::printf("    no reference image %s (run once with NXRT_NVN_UPDATE_REFS=1)\n", path.c_str());
        return;
    }
    const nxrt::gfx::ImageDiff d = nxrt::gfx::compare_rgba(px.data(), kW, kH, ref.data(), rw, rh);
    std::printf("    vs reference: mean abs %.5f, max %d, fraction over 8 %.6f\n", d.mean_abs, d.max_abs, d.fraction_over_8);
    CHECK(d.same_size && d.mean_abs <= 0.05 && d.max_abs <= 4 && d.fraction_over_8 <= 0.0005);
}

// The triangle state every draw of these scenes uses: blend, colour, channel mask, polygon, depth, multisample, vertex attribute 0 (RGB32F).
void bind_states(App& app, NVNcommandBuffer* c) {
    auto& api = app.api;
    NVNblendState bs;
    api.nvnBlendStateSetDefaults(&bs);
    api.nvnCommandBufferBindBlendState(c, &bs);
    NVNcolorState cs;
    api.nvnColorStateSetDefaults(&cs);
    api.nvnCommandBufferBindColorState(c, &cs);
    NVNchannelMaskState ms;
    api.nvnChannelMaskStateSetDefaults(&ms);
    api.nvnCommandBufferBindChannelMaskState(c, &ms);
    NVNpolygonState ps;
    api.nvnPolygonStateSetDefaults(&ps);
    api.nvnCommandBufferBindPolygonState(c, &ps);
    NVNdepthStencilState ds;
    api.nvnDepthStencilStateSetDefaults(&ds);
    api.nvnCommandBufferBindDepthStencilState(c, &ds);
    NVNmultisampleState mss;
    api.nvnMultisampleStateSetDefaults(&mss);
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
}

void set_target(App& app, NVNcommandBuffer* c, NVNtexture* t) {
    NVNtexture* targets[] = {t};
    app.api.nvnCommandBufferSetRenderTargets(c, 1, targets, nullptr, nullptr, nullptr);
    app.api.nvnCommandBufferSetViewport(c, 0, 0, kW, kH);
    app.api.nvnCommandBufferSetScissor(c, 0, 0, kW, kH);
}

void clear_color(App& app, NVNcommandBuffer* c, const float rgba[4]) { app.api.nvnCommandBufferClearColor(c, 0, rgba, NVN_CLEAR_COLOR_MASK_RGBA); }

void bind_triangle(App& app, NVNcommandBuffer* c, NVNprogram* prog, const App::Buf& vb) {
    app.api.nvnCommandBufferBindVertexBuffer(c, 0, vb.address, vb.size);
    app.api.nvnCommandBufferBindProgram(c, prog, NVN_SHADER_STAGE_VERTEX_BIT | NVN_SHADER_STAGE_FRAGMENT_BIT);
}

// Target, black clear, state, triangle: one complete frame's worth of commands (no Begin/End around it).
void record_triangle_body(App& app, NVNcommandBuffer* c, NVNtexture* t, NVNprogram* prog, const App::Buf& vb) {
    set_target(app, c, t);
    clear_color(app, c, kBlack);
    bind_states(app, c);
    bind_triangle(app, c, prog, vb);
    app.api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
}

bool load_triangle_program(App::ShaderBlob* vs, App::ShaderBlob* fs) {
    return App::load_shader("a656883b0cdb8ebc", vs) && App::load_shader("062cfa5657f5388d", fs);
}

}  // namespace

// Scene 6: the window crop presents a quarter of the window texture stretched to the window; three window textures are acquired 0,1,2,0.
NXTEST(window_crop_and_acquire_order) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    o.window_textures = 3;
    REQUIRE(app.init(o));
    auto cb = app.make_cmdbuf();
    REQUIRE(cb != nullptr);
    NVNcommandBuffer* c = cb->obj.get();
    std::vector<int> order;
    std::vector<uint8_t> px[4];
    for (int f = 0; f < 4; ++f) {
        const int idx = app.acquire();
        order.push_back(idx);
        if (f == 1) app.api.nvnWindowSetCrop(app.window.get(), 32, 24, 32, 24);   // from frame 1 on: the bottom-right quarter only
        app.api.nvnCommandBufferBeginRecording(c);
        set_target(app, c, app.window_textures[idx]->obj.get());
        clear_color(app, c, kBlue);                                               // the whole texture blue
        app.api.nvnCommandBufferSetScissor(c, 32, 24, 32, 24);
        clear_color(app, c, kYellow);                                             // its bottom-right quarter yellow
        app.end_and_submit(*cb);
        REQUIRE(app.present_and_capture(idx, &px[f]));
    }
    std::printf("    acquire order %d %d %d %d\n", order[0], order[1], order[2], order[3]);
    CHECK(order == (std::vector<int>{0, 1, 2, 0}));
    const size_t yellow0 = count_color(px[0], 255, 255, 0), blue0 = count_color(px[0], 0, 0, 255);
    std::printf("    frame 0, no crop: yellow %zu blue %zu, pixel (0,0) %d %d %d, (63,47) %d %d %d\n", yellow0, blue0, px_at(px[0], 0, 0)[0], px_at(px[0], 0, 0)[1],
                px_at(px[0], 0, 0)[2], px_at(px[0], 63, 47)[0], px_at(px[0], 63, 47)[1], px_at(px[0], 63, 47)[2]);
    CHECK(near_color(px[0], 0, 0, 0, 0, 255, 0));
    CHECK(near_color(px[0], 63, 47, 255, 255, 0, 0));
    CHECK_EQ(yellow0, size_t(32 * 24));
    CHECK_EQ(blue0, size_t(kW * kH - 32 * 24));
    const size_t yellow1 = count_color(px[1], 255, 255, 0);
    std::printf("    frame 1, crop 32,24,32,24: yellow %zu of %d pixels\n", yellow1, kW * kH);
    CHECK_EQ(yellow1, size_t(kW * kH));
    CHECK_EQ(count_color(px[3], 255, 255, 0), size_t(kW * kH));
    check_reference("window_crop", px[3]);
    check_validation(app);
}

// Scene 5: two command buffers recorded alternately and submitted in one call, then a second queue on the same device.
NXTEST(two_queues_and_interleaved_command_buffers) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    App::ShaderBlob vs, fs;
    REQUIRE(load_triangle_program(&vs, &fs));
    auto prog = app.make_program({&vs, &fs});
    REQUIRE(prog != nullptr);
    App::Buf vb = app.make_buffer(sizeof kTri, kTri);

    auto q2 = std::make_unique<NVNqueue>();
    std::memset(q2.get(), 0, sizeof(NVNqueue));
    NVNqueueBuilder qb;
    app.api.nvnQueueBuilderSetDevice(&qb, app.device.get());
    app.api.nvnQueueBuilderSetDefaults(&qb);
    REQUIRE(app.api.nvnQueueInitialize(q2.get(), &qb));

    auto a = app.make_cmdbuf(), b = app.make_cmdbuf(), g = app.make_cmdbuf();
    REQUIRE(a != nullptr && b != nullptr && g != nullptr);
    const int idx = app.acquire();
    REQUIRE(idx >= 0);
    NVNtexture* tex = app.window_textures[idx]->obj.get();
    NVNcommandBuffer* ca = a->obj.get();
    NVNcommandBuffer* cb = b->obj.get();
    // alternately: both are open at the same time, their commands interleave in the recording calls
    app.api.nvnCommandBufferBeginRecording(ca);
    app.api.nvnCommandBufferBeginRecording(cb);
    set_target(app, ca, tex);
    set_target(app, cb, tex);
    clear_color(app, ca, kBlue);
    bind_states(app, cb);
    bind_triangle(app, cb, prog.get(), vb);
    app.api.nvnCommandBufferDrawArrays(cb, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
    const NVNcommandHandle ha = app.api.nvnCommandBufferEndRecording(ca);
    const NVNcommandHandle hb = app.api.nvnCommandBufferEndRecording(cb);
    const NVNcommandHandle hs[2] = {ha, hb};
    const auto s0 = nxrt::nvn::backend_stats();
    app.api.nvnQueueSubmitCommands(app.queue.get(), 2, hs);

    // second queue: a green 8x8 corner after the first queue's work
    auto gc = g->obj.get();
    app.api.nvnCommandBufferBeginRecording(gc);
    set_target(app, gc, tex);
    app.api.nvnCommandBufferSetScissor(gc, 0, 0, 8, 8);
    clear_color(app, gc, kGreen);
    const NVNcommandHandle hg = app.api.nvnCommandBufferEndRecording(gc);
    app.api.nvnQueueSubmitCommands(q2.get(), 1, &hg);

    std::vector<uint8_t> px;
    REQUIRE(app.present_and_capture(idx, &px));
    const auto s1 = nxrt::nvn::backend_stats();
    const Census cen = magenta_census(px);
    std::printf("    magenta %zu, corner (1,1) %d %d %d, (60,44) %d %d %d, centre (32,30) %d %d %d\n", cen.magenta, px_at(px, 1, 1)[0], px_at(px, 1, 1)[1],
                px_at(px, 1, 1)[2], px_at(px, 60, 44)[0], px_at(px, 60, 44)[1], px_at(px, 60, 44)[2], px_at(px, 32, 30)[0], px_at(px, 32, 30)[1], px_at(px, 32, 30)[2]);
    std::printf("    submits +%llu, draws +%llu, lists replayed +%llu\n", static_cast<unsigned long long>(s1.submits - s0.submits),
                static_cast<unsigned long long>(s1.draws - s0.draws), static_cast<unsigned long long>(s1.lists_replayed - s0.lists_replayed));
    CHECK(near_color(px, 1, 1, 0, 255, 0, 0));
    CHECK(near_color(px, 60, 44, 0, 0, 255, 0));
    CHECK(near_color(px, 32, 30, 255, 0, 255, 0));
    CHECK(cen.magenta >= 360 && cen.magenta <= 410);
    CHECK_EQ(s1.draws - s0.draws, 1ull);
    CHECK_EQ(s1.draws_skipped - s0.draws_skipped, 0ull);
    CHECK_EQ(s1.unresolved_addresses - s0.unresolved_addresses, 0ull);
    check_reference("two_queues", px);
    check_validation(app);
    app.api.nvnQueueFinish(q2.get());
}

// Scene 2: one recorded list submitted twice in one call and in two frames; CallCommands and CopyCommands between lists (state of the
// first list visible in the second); a list whose command memory was recorded over is dropped and counted as missing.
NXTEST(command_handles_reuse_call_copy_and_drop) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    o.window_textures = 3;
    REQUIRE(app.init(o));
    App::ShaderBlob vs, fs;
    REQUIRE(load_triangle_program(&vs, &fs));
    auto prog = app.make_program({&vs, &fs});
    REQUIRE(prog != nullptr);
    App::Buf vb = app.make_buffer(sizeof kTri, kTri);
    auto setup = app.make_cmdbuf(), draw_cb = app.make_cmdbuf(), state_cb = app.make_cmdbuf(), only_cb = app.make_cmdbuf(),
         outer_call = app.make_cmdbuf(), outer_copy = app.make_cmdbuf(), drop_cb = app.make_cmdbuf();
    REQUIRE(setup != nullptr && draw_cb != nullptr && state_cb != nullptr && only_cb != nullptr && outer_call != nullptr && outer_copy != nullptr &&
            drop_cb != nullptr);
    const auto s0 = nxrt::nvn::backend_stats();

    // lists recorded once: the draw list (state + triangle, no target: the targets come from a setup list in each submission)
    NVNcommandBuffer* dc = draw_cb->obj.get();
    app.api.nvnCommandBufferBeginRecording(dc);
    bind_states(app, dc);
    bind_triangle(app, dc, prog.get(), vb);
    app.api.nvnCommandBufferDrawArrays(dc, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
    const NVNcommandHandle draw_h = app.api.nvnCommandBufferEndRecording(dc);
    // for CallCommands/CopyCommands: a state list and a draw-only list, the draw relies on the state the first one set
    NVNcommandBuffer* sc = state_cb->obj.get();
    app.api.nvnCommandBufferBeginRecording(sc);
    bind_states(app, sc);
    bind_triangle(app, sc, prog.get(), vb);
    const NVNcommandHandle state_h = app.api.nvnCommandBufferEndRecording(sc);
    NVNcommandBuffer* dr = only_cb->obj.get();
    app.api.nvnCommandBufferBeginRecording(dr);
    app.api.nvnCommandBufferDrawArrays(dr, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
    const NVNcommandHandle only_draw_h = app.api.nvnCommandBufferEndRecording(dr);

    auto record_setup = [&](NVNtexture* t) {
        NVNcommandBuffer* c = setup->obj.get();
        app.api.nvnCommandBufferBeginRecording(c);
        set_target(app, c, t);
        clear_color(app, c, kBlack);
        return app.api.nvnCommandBufferEndRecording(c);
    };
    std::vector<uint8_t> px[6];
    auto check_frame = [&](int f, const char* what) {
        const Census cen = magenta_census(px[f]);
        std::printf("    frame %d (%s): magenta %zu, mean x %.2f\n", f, what, cen.magenta, cen.cx);
        CHECK(cen.magenta >= 360 && cen.magenta <= 410);
    };

    // frame 0: the same list submitted twice in one call (two draws)
    int idx = app.acquire();
    NVNcommandHandle s_h = record_setup(app.window_textures[idx]->obj.get());
    NVNcommandHandle hs3[3] = {s_h, draw_h, draw_h};
    app.api.nvnQueueSubmitCommands(app.queue.get(), 3, hs3);
    REQUIRE(app.present_and_capture(idx, &px[0]));
    check_frame(0, "list submitted twice in one call");

    // frames 1 and 2: the same list submitted again, in two more frames
    for (int f = 1; f <= 2; ++f) {
        idx = app.acquire();
        s_h = record_setup(app.window_textures[idx]->obj.get());
        NVNcommandHandle hs2[2] = {s_h, draw_h};
        app.api.nvnQueueSubmitCommands(app.queue.get(), 2, hs2);
        REQUIRE(app.present_and_capture(idx, &px[f]));
        check_frame(f, "same list, a later frame");
    }

    // frame 3: an outer list calls the state list and then the draw list
    idx = app.acquire();
    {
        NVNcommandBuffer* oc = outer_call->obj.get();
        app.api.nvnCommandBufferBeginRecording(oc);
        set_target(app, oc, app.window_textures[idx]->obj.get());
        clear_color(app, oc, kBlack);
        const NVNcommandHandle inner[2] = {state_h, only_draw_h};
        app.api.nvnCommandBufferCallCommands(oc, 2, inner);
        const NVNcommandHandle h = app.api.nvnCommandBufferEndRecording(oc);
        app.api.nvnQueueSubmitCommands(app.queue.get(), 1, &h);
    }
    REQUIRE(app.present_and_capture(idx, &px[3]));
    check_frame(3, "CallCommands of two lists");

    // frame 4: the same two lists copied into an outer list
    idx = app.acquire();
    {
        NVNcommandBuffer* oc = outer_copy->obj.get();
        app.api.nvnCommandBufferBeginRecording(oc);
        set_target(app, oc, app.window_textures[idx]->obj.get());
        clear_color(app, oc, kBlack);
        const NVNcommandHandle inner[2] = {state_h, only_draw_h};
        app.api.nvnCommandBufferCopyCommands(oc, 2, inner);
        const NVNcommandHandle h = app.api.nvnCommandBufferEndRecording(oc);
        app.api.nvnQueueSubmitCommands(app.queue.get(), 1, &h);
    }
    REQUIRE(app.present_and_capture(idx, &px[4]));
    check_frame(4, "CopyCommands of two lists");

    // frame 5: record list X into command memory, then record Y over the same memory: X is dropped
    NVNcommandBuffer* xc = drop_cb->obj.get();
    app.api.nvnCommandBufferBeginRecording(xc);
    bind_states(app, xc);
    bind_triangle(app, xc, prog.get(), vb);
    app.api.nvnCommandBufferDrawArrays(xc, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
    const NVNcommandHandle x_h = app.api.nvnCommandBufferEndRecording(xc);
    app.api.nvnCommandBufferAddCommandMemory(xc, app.pool_cmd->obj.get(), static_cast<ptrdiff_t>(drop_cb->cmd_off), drop_cb->cmd_size);
    idx = app.acquire();
    app.api.nvnCommandBufferBeginRecording(xc);
    record_triangle_body(app, xc, app.window_textures[idx]->obj.get(), prog.get(), vb);
    const NVNcommandHandle y_h = app.api.nvnCommandBufferEndRecording(xc);
    const auto before_missing = nxrt::nvn::backend_stats().missing_handles;
    app.api.nvnQueueSubmitCommands(app.queue.get(), 1, &x_h);   // dropped: nothing is drawn, one missing handle
    const auto after_missing = nxrt::nvn::backend_stats().missing_handles;
    app.api.nvnQueueSubmitCommands(app.queue.get(), 1, &y_h);
    REQUIRE(app.present_and_capture(idx, &px[5]));
    check_frame(5, "list recorded over; the older handle submitted");
    std::printf("    missing handles for the dropped list: +%llu\n", static_cast<unsigned long long>(after_missing - before_missing));
    CHECK_EQ(after_missing - before_missing, 1ull);

    const auto s1 = nxrt::nvn::backend_stats();
    std::printf("    draws +%llu (expected 7: 2+1+1+1+1+1), submits +%llu, lists replayed +%llu, missing handles +%llu\n",
                static_cast<unsigned long long>(s1.draws - s0.draws), static_cast<unsigned long long>(s1.submits - s0.submits),
                static_cast<unsigned long long>(s1.lists_replayed - s0.lists_replayed), static_cast<unsigned long long>(s1.missing_handles - s0.missing_handles));
    CHECK_EQ(s1.draws - s0.draws, 7ull);
    CHECK_EQ(s1.draws_skipped - s0.draws_skipped, 0ull);
    CHECK_EQ(s1.unresolved_addresses - s0.unresolved_addresses, 0ull);
    CHECK_EQ(s1.unsupported_ops - s0.unsupported_ops, 0ull);
    check_reference("command_handles", px[5]);
    check_validation(app);
}

// Scene 3: fence after a submit, a queue finish, an event written by SignalEvent, ReportValue and ReportCounter (16 bytes at an address),
// ClearBuffer and CopyBufferToBuffer on pool memory read back after the finish.
NXTEST(fences_events_reports_and_buffer_commands) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    auto& api = app.api;
    const auto s0 = nxrt::nvn::backend_stats();

    // fence: a clear, then a queue fence; the wait succeeds, and after a finish it is already signalled
    auto work = app.make_cmdbuf();
    REQUIRE(work != nullptr);
    NVNcommandBuffer* wc = work->obj.get();
    api.nvnCommandBufferBeginRecording(wc);
    set_target(app, wc, app.window_textures[0]->obj.get());
    clear_color(app, wc, kBlue);
    app.end_and_submit(*work);
    api.nvnQueueFenceSync(app.queue.get(), app.sync.get(), NVN_SYNC_CONDITION_ALL_GPU_COMMANDS_COMPLETE, 0);
    const NVNsyncWaitResult w1 = api.nvnSyncWait(app.sync.get(), 2000000000ull);
    app.finish();
    const NVNsyncWaitResult w2 = api.nvnSyncWait(app.sync.get(), 2000000000ull);
    std::printf("    sync wait after submit %d, after finish %d (0 already, 1 satisfied, 2 timeout, 3 failed)\n", static_cast<int>(w1), static_cast<int>(w2));
    CHECK(w1 == NVN_SYNC_WAIT_RESULT_CONDITION_SATISFIED || w1 == NVN_SYNC_WAIT_RESULT_ALREADY_SIGNALED);
    CHECK(w2 == NVN_SYNC_WAIT_RESULT_ALREADY_SIGNALED || w2 == NVN_SYNC_WAIT_RESULT_CONDITION_SATISFIED);

    // event in pool memory: SignalEvent writes the value at the event's offset
    const size_t eoff = app.pool_main->take(64, 64);
    uint32_t* eword = reinterpret_cast<uint32_t*>(app.pool_main->base() + eoff);
    *eword = 0;
    NVNeventBuilder eb;
    api.nvnEventBuilderSetDefaults(&eb);
    api.nvnEventBuilderSetStorage(&eb, app.pool_main->obj.get(), static_cast<int64_t>(eoff));
    auto ev = std::make_unique<NVNevent>();
    std::memset(ev.get(), 0, sizeof(NVNevent));
    REQUIRE(api.nvnEventInitialize(ev.get(), &eb));
    auto ec = app.make_cmdbuf();
    REQUIRE(ec != nullptr);
    api.nvnCommandBufferBeginRecording(ec->obj.get());
    api.nvnCommandBufferSignalEvent(ec->obj.get(), ev.get(), NVN_EVENT_SIGNAL_MODE_WRITE, NVN_EVENT_SIGNAL_LOCATION_BOTTOM, 0, 0x1234);
    app.end_and_submit(*ec);
    app.finish();
    std::printf("    event after SET 0x1234: value 0x%x, pool word 0x%x\n", api.nvnEventGetValue(ev.get()), *eword);
    CHECK_EQ(api.nvnEventGetValue(ev.get()), 0x1234u);
    // ADD mode: the event must then read 0x1234 + 5; the back end writes SET (its note says so), which is a finding if it fails
    api.nvnCommandBufferBeginRecording(ec->obj.get());
    api.nvnCommandBufferSignalEvent(ec->obj.get(), ev.get(), NVN_EVENT_SIGNAL_MODE_ADD, NVN_EVENT_SIGNAL_LOCATION_BOTTOM, 0, 5);
    app.end_and_submit(*ec);
    app.finish();
    std::printf("    event after ADD 5: value 0x%x (expected 0x1239)\n", api.nvnEventGetValue(ev.get()));
    CHECK_EQ(api.nvnEventGetValue(ev.get()), 0x1239u);

    // ReportValue (value, timestamp) and ReportCounter(TIMESTAMP) (0, timestamp), 16 bytes each, read back after the finish
    App::Buf rep = app.make_buffer(32);
    std::memset(rep.ptr, 0, 32);
    auto rc = app.make_cmdbuf();
    REQUIRE(rc != nullptr);
    api.nvnCommandBufferBeginRecording(rc->obj.get());
    api.nvnCommandBufferReportValue(rc->obj.get(), 0xCAFEu, rep.address);
    api.nvnCommandBufferReportCounter(rc->obj.get(), NVN_COUNTER_TYPE_TIMESTAMP, rep.address + 16);
    app.end_and_submit(*rc);
    app.finish();
    uint64_t v0 = 0, t0 = 0, c1 = 1, t1 = 0;
    std::memcpy(&v0, rep.ptr, 8);
    std::memcpy(&t0, rep.ptr + 8, 8);
    std::memcpy(&c1, rep.ptr + 16, 8);
    std::memcpy(&t1, rep.ptr + 24, 8);
    std::printf("    ReportValue: value 0x%llx timestamp %llu; ReportCounter(TIMESTAMP): counter %llu timestamp %llu\n", static_cast<unsigned long long>(v0),
                static_cast<unsigned long long>(t0), static_cast<unsigned long long>(c1), static_cast<unsigned long long>(t1));
    CHECK_EQ(v0, 0xCAFEull);
    CHECK(t0 != 0);
    CHECK_EQ(c1, 0ull);
    CHECK(t1 != 0);

    // ClearBuffer fills 16 words with a value; CopyBufferToBuffer copies a pattern
    App::Buf cl = app.make_buffer(64), src = app.make_buffer(64), dst = app.make_buffer(64);
    std::memset(cl.ptr, 0xAB, 64);
    std::memset(dst.ptr, 0, 64);
    uint32_t* sw = reinterpret_cast<uint32_t*>(src.ptr);
    for (int i = 0; i < 16; ++i) sw[i] = 0x01000000u + static_cast<uint32_t>(i);
    auto bc = app.make_cmdbuf();
    REQUIRE(bc != nullptr);
    api.nvnCommandBufferBeginRecording(bc->obj.get());
    api.nvnCommandBufferClearBuffer(bc->obj.get(), cl.address, 64, 0x11223344u);
    api.nvnCommandBufferCopyBufferToBuffer(bc->obj.get(), src.address, dst.address, 64, 0);
    app.end_and_submit(*bc);
    app.finish();
    size_t cleared_bad = 0, copied_bad = 0;
    const uint32_t* clw = reinterpret_cast<const uint32_t*>(cl.ptr);
    const uint32_t* dstw = reinterpret_cast<const uint32_t*>(dst.ptr);
    for (int i = 0; i < 16; ++i) {
        cleared_bad += clw[i] != 0x11223344u ? 1 : 0;
        copied_bad += dstw[i] != sw[i] ? 1 : 0;
    }
    std::printf("    ClearBuffer: %zu of 16 words wrong; CopyBufferToBuffer: %zu of 16 words wrong\n", cleared_bad, copied_bad);
    CHECK_EQ(cleared_bad, 0u);
    CHECK_EQ(copied_bad, 0u);

    const auto s1 = nxrt::nvn::backend_stats();
    std::printf("    clears +%llu, copies +%llu, unsupported +%llu\n", static_cast<unsigned long long>(s1.clears - s0.clears),
                static_cast<unsigned long long>(s1.copies - s0.copies), static_cast<unsigned long long>(s1.unsupported_ops - s0.unsupported_ops));
    CHECK_EQ(s1.unresolved_addresses - s0.unresolved_addresses, 0ull);
    check_validation(app);
}

// Scene 1: a fragment shader whose output is a uniform block (490be1c3fd803f84: out = cb3[0]). The colour comes from a uniform buffer
// bound at fragment slot 0; an update between two draws in one command buffer is GPU-ordered; a CPU write between submissions shows
// in the next frame.
NXTEST(uniform_buffers_with_real_shaders) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    App::ShaderBlob vs, fs;
    REQUIRE(load_triangle_program(&vs, &fs));
    REQUIRE(App::load_shader("490be1c3fd803f84", &fs));   // fragment: out = fp_c3.data[0] (uniform block bound at fragment slot 0)
    auto prog = app.make_program({&vs, &fs});
    REQUIRE(prog != nullptr);
    App::Buf vb = app.make_buffer(sizeof kTri, kTri);
    App::Buf ub = app.make_buffer(65536);   // the block is declared as data[4096]; the shader reads data[0]
    std::memset(ub.ptr, 0, 65536);
    float* uf = reinterpret_cast<float*>(ub.ptr);
    const float colour_a[4] = {1.0f, 0.0f, 0.5f, 1.0f};    // (255, 0, 128)
    const float colour_b[4] = {0.0f, 1.0f, 0.25f, 1.0f};   // (0, 255, 64)
    const float colour_c[4] = {0.0f, 0.0f, 1.0f, 1.0f};    // (0, 0, 255)
    std::memcpy(uf, colour_a, 16);
    const auto s0 = nxrt::nvn::backend_stats();

    // frame 1: draw A (full target), update the buffer to B on the GPU, draw again with the left half as scissor
    int idx = app.acquire();
    auto cb = app.make_cmdbuf();
    REQUIRE(cb != nullptr);
    NVNcommandBuffer* c = cb->obj.get();
    app.api.nvnCommandBufferBeginRecording(c);
    set_target(app, c, app.window_textures[idx]->obj.get());
    clear_color(app, c, kBlack);
    bind_states(app, c);
    bind_triangle(app, c, prog.get(), vb);
    app.api.nvnCommandBufferBindUniformBuffer(c, NVN_SHADER_STAGE_FRAGMENT, 0, ub.address, 65536);
    app.api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
    app.api.nvnCommandBufferUpdateUniformBuffer(c, ub.address, 65536, 0, 16, colour_b);
    app.api.nvnCommandBufferSetScissor(c, 0, 0, 32, kH);
    app.api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
    app.end_and_submit(*cb);
    std::vector<uint8_t> px1;
    REQUIRE(app.present_and_capture(idx, &px1));
    std::printf("    frame 1: (40,30) %d %d %d [draw A only], (26,30) %d %d %d [draw B over A, left half]\n", px_at(px1, 40, 30)[0], px_at(px1, 40, 30)[1],
                px_at(px1, 40, 30)[2], px_at(px1, 26, 30)[0], px_at(px1, 26, 30)[1], px_at(px1, 26, 30)[2]);
    CHECK(near_color(px1, 40, 30, 255, 0, 128, 1));
    CHECK(near_color(px1, 26, 30, 0, 255, 64, 1));
    CHECK(near_color(px1, 2, 2, 0, 0, 0, 0));

    // frame 2: the CPU writes C between submissions (after a finish), the next frame draws C
    app.finish();
    std::memcpy(uf, colour_c, 16);
    idx = app.acquire();
    app.api.nvnCommandBufferBeginRecording(c);
    set_target(app, c, app.window_textures[idx]->obj.get());
    clear_color(app, c, kBlack);
    bind_states(app, c);
    bind_triangle(app, c, prog.get(), vb);
    app.api.nvnCommandBufferBindUniformBuffer(c, NVN_SHADER_STAGE_FRAGMENT, 0, ub.address, 65536);
    app.api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
    app.end_and_submit(*cb);
    std::vector<uint8_t> px2;
    REQUIRE(app.present_and_capture(idx, &px2));
    std::printf("    frame 2 (CPU write of 0,0,1,1): centre (32,30) %d %d %d\n", px_at(px2, 32, 30)[0], px_at(px2, 32, 30)[1], px_at(px2, 32, 30)[2]);
    CHECK(near_color(px2, 32, 30, 0, 0, 255, 1));
    CHECK(near_color(px2, 2, 2, 0, 0, 0, 0));

    const auto s1 = nxrt::nvn::backend_stats();
    std::printf("    draws +%llu (expected 3), unresolved +%llu, unsupported +%llu\n", static_cast<unsigned long long>(s1.draws - s0.draws),
                static_cast<unsigned long long>(s1.unresolved_addresses - s0.unresolved_addresses), static_cast<unsigned long long>(s1.unsupported_ops - s0.unsupported_ops));
    CHECK_EQ(s1.draws - s0.draws, 3ull);
    CHECK_EQ(s1.draws_skipped - s0.draws_skipped, 0ull);
    CHECK_EQ(s1.unresolved_addresses - s0.unresolved_addresses, 0ull);
    check_reference("uniform_buffers", px2);
    check_validation(app);
}

// Scene 4: 120 frames of a moving triangle; vertex data written by the CPU into a ring of three buffers each frame; the window cycles its
// three textures. Every frame must be presented and show the triangle at its position; timing is printed and bounded.
NXTEST(many_frames_moving_triangle) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    o.window_textures = 3;
    REQUIRE(app.init(o));
    App::ShaderBlob vs, fs;
    REQUIRE(load_triangle_program(&vs, &fs));
    auto prog = app.make_program({&vs, &fs});
    REQUIRE(prog != nullptr);
    App::Buf ring[3];
    for (auto& r : ring) r = app.make_buffer(sizeof kTri);
    auto cb = app.make_cmdbuf();
    REQUIRE(cb != nullptr);
    const auto s0 = nxrt::nvn::backend_stats();
    constexpr int kFrames = 120;
    size_t bad = 0;
    double worst = 0;
    std::vector<uint8_t> px;
    const auto t0 = std::chrono::steady_clock::now();
    for (int f = 0; f < kFrames; ++f) {
        const double phase = 2.0 * 3.14159265358979323846 * f / kFrames;
        const float dx = static_cast<float>(0.4 * std::sin(phase));
        const float verts[9] = {dx - 0.5f, -0.5f, 0.0f, dx + 0.5f, -0.5f, 0.0f, dx, 0.5f, 0.0f};
        const App::Buf& vb = ring[f % 3];
        std::memcpy(vb.ptr, verts, sizeof verts);
        const int idx = app.acquire();
        REQUIRE(idx >= 0);
        app.api.nvnCommandBufferBeginRecording(cb->obj.get());
        record_triangle_body(app, cb->obj.get(), app.window_textures[idx]->obj.get(), prog.get(), vb);
        app.end_and_submit(*cb);
        REQUIRE(app.present_and_capture(idx, &px));
        const Census cen = magenta_census(px);
        const double want_cx = 31.5 + dx * 32.0;
        const double err = std::fabs(cen.cx - want_cx);
        worst = std::max(worst, err);
        if (cen.magenta < 360 || cen.magenta > 410 || err > 1.0) {
            ++bad;
            if (bad <= 3) std::printf("    frame %d: magenta %zu, mean x %.2f, wanted %.2f\n", f, cen.magenta, cen.cx, want_cx);
        }
    }
    const auto t1 = std::chrono::steady_clock::now();
    const double ms = std::chrono::duration<double, std::milli>(t1 - t0).count() / kFrames;
    const auto s1 = nxrt::nvn::backend_stats();
    std::printf("    %d frames, %.3f ms per frame (record, submit, present, readback), %zu frames off, worst centroid error %.3f px\n", kFrames, ms, bad, worst);
    std::printf("    presents +%llu, draws +%llu, skipped +%llu, unresolved +%llu, unsupported +%llu\n", static_cast<unsigned long long>(s1.presents - s0.presents),
                static_cast<unsigned long long>(s1.draws - s0.draws), static_cast<unsigned long long>(s1.draws_skipped - s0.draws_skipped),
                static_cast<unsigned long long>(s1.unresolved_addresses - s0.unresolved_addresses), static_cast<unsigned long long>(s1.unsupported_ops - s0.unsupported_ops));
    CHECK_EQ(bad, 0u);
    CHECK_EQ(s1.presents - s0.presents, static_cast<unsigned long long>(kFrames));
    CHECK_EQ(s1.draws - s0.draws, static_cast<unsigned long long>(kFrames));
    CHECK_EQ(s1.draws_skipped - s0.draws_skipped, 0ull);
    CHECK_EQ(s1.unresolved_addresses - s0.unresolved_addresses, 0ull);
    CHECK_EQ(s1.unsupported_ops - s0.unsupported_ops, 0ull);
    CHECK(ms < 50.0);
    check_reference("many_frames", px);
    check_validation(app);
}

// Scene 7: compute shader 444294be0c21533c (one workgroup, group 0 only, writes ten words to storage slot 0). Dispatched with two
// groups: group 1 returns, so the words after the ten written stay as they were.
NXTEST(compute_dispatch_writes_storage) {
    if (!real_data_available()) { nxtest::skip("work/shader_cache not found"); return; }
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    App::ShaderBlob cs;
    REQUIRE(App::load_shader("444294be0c21533c", &cs));
    auto prog = app.make_program({&cs});
    REQUIRE(prog != nullptr);
    App::Buf sb = app.make_buffer(4096);   // larger than the 64 bytes checked below: a scan shows where any write lands
    uint32_t* w = reinterpret_cast<uint32_t*>(sb.ptr);
    for (int i = 0; i < 1024; ++i) w[i] = 0xFFFFFFFFu;
    auto cb = app.make_cmdbuf();
    REQUIRE(cb != nullptr);
    const auto s0 = nxrt::nvn::backend_stats();
    NVNcommandBuffer* c = cb->obj.get();
    app.api.nvnCommandBufferBeginRecording(c);
    app.api.nvnCommandBufferBindProgram(c, prog.get(), NVN_SHADER_STAGE_COMPUTE_BIT);
    app.api.nvnCommandBufferBindStorageBuffer(c, NVN_SHADER_STAGE_COMPUTE, 0, sb.address, 4096);
    app.api.nvnCommandBufferDispatchCompute(c, 2, 1, 1);
    app.end_and_submit(*cb);
    app.finish();
    const uint32_t want[10] = {3, 1, 0, 0, 0, 3, 1, 3, 0, 0};
    size_t bad = 0;
    for (int i = 0; i < 10; ++i) bad += w[i] != want[i] ? 1 : 0;
    size_t untouched = 0;
    for (int i = 10; i < 16; ++i) untouched += w[i] == 0xFFFFFFFFu ? 1 : 0;
    size_t changed_total = 0;
    int first_changed = -1;
    for (int i = 0; i < 1024; ++i)
        if (w[i] != 0xFFFFFFFFu) {
            ++changed_total;
            if (first_changed < 0) first_changed = i;
        }
    std::printf("    diagnostic: %zu of 1024 words changed anywhere in the 4 KiB buffer, first at word %d\n", changed_total, first_changed);
    std::printf("    words 0..9: %zu wrong; words 10..15 untouched: %zu of 6; words 0..9 = %u %u %u %u %u %u %u %u %u %u\n", bad, untouched, w[0], w[1], w[2], w[3],
                w[4], w[5], w[6], w[7], w[8], w[9]);
    CHECK_EQ(bad, 0u);
    CHECK_EQ(untouched, 6u);
    const auto s1 = nxrt::nvn::backend_stats();
    std::printf("    dispatches +%llu, unsupported +%llu\n", static_cast<unsigned long long>(s1.dispatches - s0.dispatches),
                static_cast<unsigned long long>(s1.unsupported_ops - s0.unsupported_ops));
    CHECK_EQ(s1.dispatches - s0.dispatches, 1ull);
    CHECK_EQ(s1.unsupported_ops - s0.unsupported_ops, 0ull);
    check_validation(app);
}
