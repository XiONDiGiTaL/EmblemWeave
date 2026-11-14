// SPDX-License-Identifier: GPL-3.0-or-later
// NVN scene oracles, geometry: the ten primitive types, indexed draws with the three index sizes, base vertex, instancing,
// primitive restart, window origin, depth mode, viewport, scissor, culling, polygon mode, several draws and state changes in one
// command buffer, and state carried from one submitted command buffer to the next. Real translated shaders of the game (work/shader_cache):
//   vertex 2bf6629719b97303  gl_Position.xyz = attr0.xyz, gl_PointSize = 1, colour out = attr1 (vec4)
//   vertex a656883b0cdb8ebc  gl_Position.xyz = attr0.xyz, gl_PointSize = 1, no outputs
//   vertex 49afacac912112e1  gl_Position.xy = attr2.z * attr0.xy + attr2.xy, gl_Position.z = attr0.z (instance offset and scale in attr2)
//   fragment 062cfa5657f5388d  constant magenta (1, 0, 1, 1)
//   fragment 8b7d1022ca3c42ff  out = in_attr0 (passthrough). The guide's 0a77a28b0ad545f6 has the same body, but this machine's
//                              work/shader_cache/spirv has no 0a77a28b0ad545f6.spv, so the back end cannot load it.
#include "nvn_app.h"
#include <cstdint>
#include <cstdlib>
NXTEST_GROUP(gfx_nvn_scene_geometry)

using namespace nvnapp;

namespace {

constexpr const char* kVsPos = "a656883b0cdb8ebc";
constexpr const char* kVsColour = "2bf6629719b97303";
constexpr const char* kVsInstance = "49afacac912112e1";
constexpr const char* kFsMagenta = "062cfa5657f5388d";
constexpr const char* kFsPass = "8b7d1022ca3c42ff";
constexpr int kW = 64, kH = 48;

// ---- pixels ------------------------------------------------------------------------------------------------------------
struct Census {
    size_t magenta = 0, green = 0, other = 0;   // exact magenta, exact green, and any other non-black pixel
};

// Counts pixels in [x0,x1) x [y0,y1). Default arguments: the whole frame.
Census census(const std::vector<uint8_t>& px, int x0 = 0, int y0 = 0, int x1 = kW, int y1 = kH) {
    Census c;
    for (int y = y0; y < y1; ++y)
        for (int x = x0; x < x1; ++x) {
            const uint8_t* p = &px[(static_cast<size_t>(y) * kW + x) * 4];
            if (p[0] == 255 && p[1] == 0 && p[2] == 255) ++c.magenta;
            else if (p[0] == 0 && p[1] == 255 && p[2] == 0) ++c.green;
            else if (p[0] || p[1] || p[2]) ++c.other;
        }
    return c;
}
bool is_magenta(const std::vector<uint8_t>& px, int x, int y) {
    const uint8_t* p = &px[(static_cast<size_t>(y) * kW + x) * 4];
    return p[0] == 255 && p[1] == 0 && p[2] == 255;
}
bool is_black(const std::vector<uint8_t>& px, int x, int y) {
    const uint8_t* p = &px[(static_cast<size_t>(y) * kW + x) * 4];
    return p[0] == 0 && p[1] == 0 && p[2] == 0;
}

// The full-size square of the position tests: x 16..47, y 12..35 (32 x 24 = 768 pixels), nothing else drawn.
void expect_square(const std::vector<uint8_t>& px, const char* what) {
    const Census c = census(px);
    std::printf("    %s: %zu magenta, %zu other\n", what, c.magenta, c.other);
    CHECK_EQ(c.magenta, size_t(768));
    CHECK_EQ(c.other, size_t(0));
    CHECK(is_magenta(px, 16, 12) && is_magenta(px, 47, 35) && is_black(px, 15, 12) && is_black(px, 48, 35));
}

// Reference frame work/nvn_refs/<name>.png: written with NXRT_NVN_UPDATE_REFS=1, compared on later runs when it exists.
void check_reference(const char* name, const std::vector<uint8_t>& px) {
    const std::string path = nxtest::project_path("work/nvn_refs/") + name + ".png";
    const char* upd = std::getenv("NXRT_NVN_UPDATE_REFS");
    if (upd && std::string(upd) == "1") {
        CHECK(nxrt::gfx::save_png_rgba(path, px.data(), kW, kH));
        std::printf("    reference written: %s\n", path.c_str());
        return;
    }
    std::vector<uint8_t> ref;
    uint32_t rw = 0, rh = 0;
    if (!nxrt::gfx::load_png_rgba(path, &ref, &rw, &rh)) {
        std::printf("    no reference yet: %s\n", path.c_str());
        return;
    }
    const auto d = nxrt::gfx::compare_rgba(px.data(), kW, kH, ref.data(), rw, rh);
    std::printf("    reference %s: mean %.4f max %d over8 %.6f\n", name, d.mean_abs, d.max_abs, d.fraction_over_8);
    CHECK(d.same_size && d.mean_abs <= 0.05 && d.max_abs <= 4 && d.fraction_over_8 <= 0.0005);
}

// ---- back end counters -------------------------------------------------------------------------------------------------
struct Stats {
    uint64_t draws = 0, skipped = 0, unresolved = 0, unsupported = 0, submits = 0;
};
Stats stats_now() {
    const auto s = nxrt::nvn::backend_stats();
    return {s.draws, s.draws_skipped, s.unresolved_addresses, s.unsupported_ops, s.submits};
}
// Healthy-case invariants of one case: the expected draws and submissions, nothing skipped, unresolved or unsupported, no
// validation error. `base` is the counter snapshot taken after the App was created.
void check_case(App& app, const Stats& base, uint64_t draws, uint64_t submits) {
    const Stats now = stats_now();
    const auto v = app.validation();
    std::printf("    backend: draws %llu (expect %llu), submits %llu (expect %llu), skipped %llu, unresolved %llu, unsupported %llu\n",
                static_cast<unsigned long long>(now.draws - base.draws), static_cast<unsigned long long>(draws),
                static_cast<unsigned long long>(now.submits - base.submits), static_cast<unsigned long long>(submits),
                static_cast<unsigned long long>(now.skipped - base.skipped), static_cast<unsigned long long>(now.unresolved - base.unresolved),
                static_cast<unsigned long long>(now.unsupported - base.unsupported));
    std::printf("    validation: %llu errors, %llu warnings %s\n", static_cast<unsigned long long>(v.errors),
                static_cast<unsigned long long>(v.warnings), v.first_error.c_str());
    CHECK_EQ(now.draws - base.draws, draws);
    CHECK_EQ(now.submits - base.submits, submits);
    CHECK_EQ(now.skipped - base.skipped, 0ull);
    CHECK_EQ(now.unresolved - base.unresolved, 0ull);
    CHECK_EQ(now.unsupported - base.unsupported, 0ull);
    CHECK_EQ(v.errors, 0ull);
}

bool have_data() {
    if (real_data_available()) return true;
    nxtest::skip("work/shader_cache not found");
    return false;
}

// ---- recording helpers -------------------------------------------------------------------------------------------------
// Records one frame: the window texture as the target, the full viewport and scissor, a black clear, then `record`. Submits,
// presents and reads the frame back. Every frame takes its own command buffer.
template <class Record>
bool frame(App& app, Record&& record, std::vector<uint8_t>* px) {
    const int idx = app.acquire();
    if (idx < 0) return false;
    auto cb = app.make_cmdbuf();
    if (!cb) return false;
    NVNcommandBuffer* c = cb->obj.get();
    NVNtexture* targets[] = {app.window_textures[idx]->obj.get()};
    app.api.nvnCommandBufferBeginRecording(c);
    app.api.nvnCommandBufferSetRenderTargets(c, 1, targets, nullptr, nullptr, nullptr);
    app.api.nvnCommandBufferSetViewport(c, 0, 0, kW, kH);
    app.api.nvnCommandBufferSetScissor(c, 0, 0, kW, kH);
    const float black[4] = {0, 0, 0, 1};
    app.api.nvnCommandBufferClearColor(c, 0, black, NVN_CLEAR_COLOR_MASK_RGBA);
    record(c);
    app.end_and_submit(*cb);
    return app.present_and_capture(idx, px);
}

std::unique_ptr<NVNprogram> make_prog(App& app, const char* vs_hash, const char* fs_hash, App::ShaderBlob* vs, App::ShaderBlob* fs) {
    if (!App::load_shader(vs_hash, vs) || !App::load_shader(fs_hash, fs)) return nullptr;
    return app.make_program({vs, fs});
}

NVNvertexAttribState attrib(App& app, NVNformat fmt, int stream, ptrdiff_t offset) {
    NVNvertexAttribState a;
    app.api.nvnVertexAttribStateSetDefaults(&a);
    app.api.nvnVertexAttribStateSetFormat(&a, fmt, offset);
    app.api.nvnVertexAttribStateSetStreamIndex(&a, stream);
    return a;
}
NVNvertexStreamState stream(App& app, ptrdiff_t stride, int divisor = 0) {
    NVNvertexStreamState s;
    app.api.nvnVertexStreamStateSetDefaults(&s);
    app.api.nvnVertexStreamStateSetStride(&s, stride);
    if (divisor) app.api.nvnVertexStreamStateSetDivisor(&s, divisor);
    return s;
}
// Attribute i of `attrs` is location i; stream i of `streams` is stream index i.
void bind_input(App& app, NVNcommandBuffer* c, const std::vector<NVNvertexAttribState>& attrs, const std::vector<NVNvertexStreamState>& streams) {
    app.api.nvnCommandBufferBindVertexAttribState(c, static_cast<int>(attrs.size()), attrs.data());
    app.api.nvnCommandBufferBindVertexStreamState(c, static_cast<int>(streams.size()), streams.data());
}

// The state a game binds before its draws. The polygon state is given by value (cull face, front face, polygon mode).
NVNpolygonState polygon(App& app, int cull, int front, int mode) {
    NVNpolygonState ps;
    app.api.nvnPolygonStateSetDefaults(&ps);
    app.api.nvnPolygonStateSetCullFace(&ps, static_cast<NVNface>(cull));
    app.api.nvnPolygonStateSetFrontFace(&ps, static_cast<NVNfrontFace>(front));
    app.api.nvnPolygonStateSetPolygonMode(&ps, static_cast<NVNpolygonMode>(mode));
    return ps;
}
void bind_states(App& app, NVNcommandBuffer* c, NVNpolygonState ps) {
    auto& api = app.api;
    NVNblendState bs;
    api.nvnBlendStateSetDefaults(&bs);
    NVNcolorState cs;
    api.nvnColorStateSetDefaults(&cs);
    NVNchannelMaskState ms;
    api.nvnChannelMaskStateSetDefaults(&ms);
    NVNdepthStencilState ds;
    api.nvnDepthStencilStateSetDefaults(&ds);
    NVNmultisampleState mss;
    api.nvnMultisampleStateSetDefaults(&mss);
    api.nvnCommandBufferBindBlendState(c, &bs);
    api.nvnCommandBufferBindColorState(c, &cs);
    api.nvnCommandBufferBindChannelMaskState(c, &ms);
    api.nvnCommandBufferBindPolygonState(c, &ps);
    api.nvnCommandBufferBindDepthStencilState(c, &ds);
    api.nvnCommandBufferBindMultisampleState(c, &mss);
}
void bind_program(App& app, NVNcommandBuffer* c, NVNprogram* prog) {
    app.api.nvnCommandBufferBindProgram(c, prog, NVN_SHADER_STAGE_VERTEX_BIT | NVN_SHADER_STAGE_FRAGMENT_BIT);
}
// Position only: attribute 0 (RGB32F) from stream 0, stride 12, from the buffer `vb`.
void bind_position(App& app, NVNcommandBuffer* c, const App::Buf& vb) {
    std::vector<NVNvertexAttribState> a = {attrib(app, NVN_FORMAT_RGB32F, 0, 0)};
    std::vector<NVNvertexStreamState> s = {stream(app, 12)};
    bind_input(app, c, a, s);
    app.api.nvnCommandBufferBindVertexBuffer(c, 0, vb.address, vb.size);
}
// Position and colour: attribute 0 (RGB32F at 0), attribute 1 (RGBA32F at 12), stream 0, stride 28.
void bind_colour(App& app, NVNcommandBuffer* c, const App::Buf& vb) {
    std::vector<NVNvertexAttribState> a = {attrib(app, NVN_FORMAT_RGB32F, 0, 0), attrib(app, NVN_FORMAT_RGBA32F, 0, 12)};
    std::vector<NVNvertexStreamState> s = {stream(app, 28)};
    bind_input(app, c, a, s);
    app.api.nvnCommandBufferBindVertexBuffer(c, 0, vb.address, vb.size);
}
// Instancing: locations 0, 1, 3 read vec4 vertices from stream 0 (stride 16); location 2 is the per-instance vec4 of stream 1 (divisor 1).
void bind_instanced(App& app, NVNcommandBuffer* c, const App::Buf& vb, const App::Buf& inst) {
    std::vector<NVNvertexAttribState> a = {
        attrib(app, NVN_FORMAT_RGB32F, 0, 0), attrib(app, NVN_FORMAT_RGBA32F, 0, 0), attrib(app, NVN_FORMAT_RGBA32F, 1, 0),
        attrib(app, NVN_FORMAT_RGBA32F, 0, 0)};
    std::vector<NVNvertexStreamState> s = {stream(app, 16), stream(app, 16, 1)};
    bind_input(app, c, a, s);
    app.api.nvnCommandBufferBindVertexBuffer(c, 0, vb.address, vb.size);
    app.api.nvnCommandBufferBindVertexBuffer(c, 1, inst.address, inst.size);
}

// One frame of the magenta program with position input: `count` vertices from `first` drawn as `mode`, back-face culling, fill.
bool draw_positions(App& app, NVNprogram* prog, const App::Buf& vb, int mode, int first, int count, std::vector<uint8_t>* px) {
    return frame(app, [&](NVNcommandBuffer* c) {
        bind_states(app, c, polygon(app, NVN_FACE_BACK, NVN_FRONT_FACE_CCW, NVN_POLYGON_MODE_FILL));
        bind_position(app, c, vb);
        bind_program(app, c, prog);
        app.api.nvnCommandBufferDrawArrays(c, static_cast<NVNdrawPrimitive>(mode), first, count);
    }, px);
}

// The window origin test. NDC +y is the top edge of a viewport rectangle in upper-left mode (image row 0 is the top of the picture:
// the apex of the triangle is near the top) and the GL convention in lower-left mode, where window y 0 is memory row 0 and NDC -y
// is the first row: the window texture is vertically mirrored in memory, and the present step flips it back (what the console's
// display does: the real game runs in lower-left mode and shows an upright picture, boot note 122), so the PRESENTED picture the test
// reads back is upright in both modes. Scissor and viewport y count memory rows in both modes, from row 0: in lower-left mode the
// scissor band 0..11 appears at the bottom of the presented picture.
void origin_case(bool upper_left, const char* ref) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    o.upper_left = upper_left;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs;
    auto prog = make_prog(app, kVsPos, kFsMagenta, &vs, &fs);
    REQUIRE(prog != nullptr);
    const float tri[] = {-0.5f, -0.5f, 0, 0.5f, -0.5f, 0, 0.0f, 0.5f, 0};
    const App::Buf vb = app.make_buffer(sizeof tri, tri);
    std::vector<uint8_t> px;
    REQUIRE(draw_positions(app, prog.get(), vb, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3, &px));
    const Census top = census(px, 0, 13, kW, 14), bottom = census(px, 0, 34, kW, 35);
    std::printf("    %s origin, triangle: row 13 (near the apex at y=+0.5) %zu magenta, row 34 (near the base) %zu magenta\n",
                upper_left ? "upper-left" : "lower-left", top.magenta, bottom.magenta);
    CHECK(top.magenta <= 4);        // the apex (NDC y = +0.5) is near the top of the presented picture in both modes
    CHECK(bottom.magenta >= 28);
    CHECK_EQ(census(px).other, size_t(0));
    check_reference(ref, px);

    const float quad[] = {-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, -1, 0, 1, 1, 0, -1, 1, 0};
    const App::Buf vq = app.make_buffer(sizeof quad, quad);
    std::vector<uint8_t> px2;
    REQUIRE(frame(app, [&](NVNcommandBuffer* c) {
        app.api.nvnCommandBufferSetScissor(c, 0, 0, kW, 12);   // rows 0..11 in both origin modes
        bind_states(app, c, polygon(app, NVN_FACE_BACK, NVN_FRONT_FACE_CCW, NVN_POLYGON_MODE_FILL));
        bind_position(app, c, vq);
        bind_program(app, c, prog.get());
        app.api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 6);
    }, &px2));
    const Census band = upper_left ? census(px2, 0, 0, kW, 12) : census(px2, 0, kH - 12, kW, kH);
    const Census all = census(px2);
    std::printf("    %s origin, scissor 0,0,64,12: %zu magenta in the origin band (%s), %zu in total, %zu other\n",
                upper_left ? "upper-left" : "lower-left", band.magenta, upper_left ? "presented rows 0..11" : "presented rows bottom 12", all.magenta, all.other);
    CHECK_EQ(band.magenta, size_t(768));
    CHECK_EQ(all.magenta, size_t(768));
    CHECK_EQ(all.other, size_t(0));
    check_case(app, base, 2, 2);
}

// Depth mode: a flat triangle at z = 0 must be drawn the same way in both modes.
void depth_case(bool depth_zero, const char* ref) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    o.depth_zero = depth_zero;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs;
    auto prog = make_prog(app, kVsPos, kFsMagenta, &vs, &fs);
    REQUIRE(prog != nullptr);
    const float tri[] = {-0.5f, -0.5f, 0, 0.5f, -0.5f, 0, 0.0f, 0.5f, 0};
    const App::Buf vb = app.make_buffer(sizeof tri, tri);
    std::vector<uint8_t> px;
    REQUIRE(draw_positions(app, prog.get(), vb, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3, &px));
    const Census c = census(px);
    std::printf("    depth mode %s: %zu magenta, %zu other, centre pixel %s\n", depth_zero ? "NEAR_IS_ZERO" : "NEAR_IS_MINUS_W", c.magenta, c.other,
                is_magenta(px, 32, 28) ? "magenta" : "not magenta");
    CHECK(c.magenta >= 360 && c.magenta <= 410);
    CHECK_EQ(c.other, size_t(0));
    CHECK(is_magenta(px, 32, 28));
    check_reference(ref, px);
    check_case(app, base, 1, 1);
}

}  // namespace

// ---- 1. primitive types: triangles, strip, fan -----------------------------------------------------------------------------
NXTEST(prim_triangles_strip_fan) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs;
    auto prog = make_prog(app, kVsPos, kFsMagenta, &vs, &fs);
    REQUIRE(prog != nullptr);
    // the same square (-0.5..0.5) as: two triangles, a strip of four vertices, and a fan around the centre (first corner repeated)
    const float tris[] = {-0.5f, -0.5f, 0, 0.5f, -0.5f, 0, 0.5f, 0.5f, 0, -0.5f, -0.5f, 0, 0.5f, 0.5f, 0, -0.5f, 0.5f, 0};
    const float strip[] = {-0.5f, -0.5f, 0, 0.5f, -0.5f, 0, -0.5f, 0.5f, 0, 0.5f, 0.5f, 0};
    const float fan[] = {0, 0, 0, -0.5f, -0.5f, 0, 0.5f, -0.5f, 0, 0.5f, 0.5f, 0, -0.5f, 0.5f, 0, -0.5f, -0.5f, 0};
    const App::Buf vt = app.make_buffer(sizeof tris, tris);
    const App::Buf vs4 = app.make_buffer(sizeof strip, strip);
    const App::Buf vf = app.make_buffer(sizeof fan, fan);
    std::vector<uint8_t> px_t, px_s, px_f;
    REQUIRE(draw_positions(app, prog.get(), vt, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 6, &px_t));
    REQUIRE(draw_positions(app, prog.get(), vs4, NVN_DRAW_PRIMITIVE_TRIANGLE_STRIP, 0, 4, &px_s));
    REQUIRE(draw_positions(app, prog.get(), vf, NVN_DRAW_PRIMITIVE_TRIANGLE_FAN, 0, 6, &px_f));
    expect_square(px_t, "TRIANGLES");
    expect_square(px_s, "TRIANGLE_STRIP");
    expect_square(px_f, "TRIANGLE_FAN");
    check_reference("geom_triangles", px_t);
    check_case(app, base, 3, 3);
}

// ---- 2. indexed drawing: the three index sizes give the same pixels --------------------------------------------------------
NXTEST(indexed_draw_index_types) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs;
    auto prog = make_prog(app, kVsPos, kFsMagenta, &vs, &fs);
    REQUIRE(prog != nullptr);
    const float quad[] = {-0.5f, -0.5f, 0, 0.5f, -0.5f, 0, 0.5f, 0.5f, 0, -0.5f, 0.5f, 0};
    const uint8_t i8[] = {0, 1, 2, 0, 2, 3};
    const uint16_t i16[] = {0, 1, 2, 0, 2, 3};
    const uint32_t i32[] = {0, 1, 2, 0, 2, 3};
    const App::Buf vb = app.make_buffer(sizeof quad, quad);
    const App::Buf ib[3] = {app.make_buffer(sizeof i8, i8), app.make_buffer(sizeof i16, i16), app.make_buffer(sizeof i32, i32)};
    const int types[3] = {NVN_INDEX_TYPE_UNSIGNED_BYTE, NVN_INDEX_TYPE_UNSIGNED_SHORT, NVN_INDEX_TYPE_UNSIGNED_INT};
    const char* names[3] = {"UNSIGNED_BYTE", "UNSIGNED_SHORT", "UNSIGNED_INT"};
    std::vector<uint8_t> px[3];
    for (int k = 0; k < 3; ++k) {
        const bool ok = frame(app, [&](NVNcommandBuffer* c) {
            bind_states(app, c, polygon(app, NVN_FACE_BACK, NVN_FRONT_FACE_CCW, NVN_POLYGON_MODE_FILL));
            bind_position(app, c, vb);
            bind_program(app, c, prog.get());
            app.api.nvnCommandBufferDrawElements(c, NVN_DRAW_PRIMITIVE_TRIANGLES, static_cast<NVNindexType>(types[k]), 6, ib[k].address);
        }, &px[k]);
        REQUIRE(ok);
        expect_square(px[k], names[k]);
    }
    CHECK(px[0] == px[1]);
    CHECK(px[1] == px[2]);
    check_reference("geom_indexed_u16", px[1]);
    check_case(app, base, 3, 3);
}

// ---- 3. window origin: the same triangle in both origin modes is mirrored --------------------------------------------------
NXTEST(origin_upper_left_apex) {
    origin_case(true, "geom_origin_upper_left");
}
NXTEST(origin_lower_left_apex) {
    origin_case(false, "geom_origin_lower_left");
}

// ---- 2b. QUADS, QUAD_STRIP, POLYGON (converted by the back end) -------------------------------------------------------------
NXTEST(prim_quads_quad_strip_polygon) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs;
    auto prog = make_prog(app, kVsPos, kFsMagenta, &vs, &fs);
    REQUIRE(prog != nullptr);
    const float quads[] = {-0.5f, -0.5f, 0, 0.5f, -0.5f, 0, 0.5f, 0.5f, 0, -0.5f, 0.5f, 0};
    // strip order (v0 bottom-left, v1 bottom-right, v2 top-left, v3 top-right): the quads (v0,v1,v3,v2) are counter-clockwise
    const float qstrip[] = {-0.5f, -0.5f, 0, 0.5f, -0.5f, 0, -0.5f, 0.5f, 0, 0.5f, 0.5f, 0};
    const float poly[] = {-0.5f, -0.5f, 0, 0.5f, -0.5f, 0, 0.5f, 0.5f, 0, -0.5f, 0.5f, 0};
    const App::Buf vq = app.make_buffer(sizeof quads, quads);
    const App::Buf vqs = app.make_buffer(sizeof qstrip, qstrip);
    const App::Buf vp = app.make_buffer(sizeof poly, poly);
    std::vector<uint8_t> px_q, px_qs, px_p;
    REQUIRE(draw_positions(app, prog.get(), vq, NVN_DRAW_PRIMITIVE_QUADS, 0, 4, &px_q));
    REQUIRE(draw_positions(app, prog.get(), vqs, NVN_DRAW_PRIMITIVE_QUAD_STRIP, 0, 4, &px_qs));
    REQUIRE(draw_positions(app, prog.get(), vp, NVN_DRAW_PRIMITIVE_POLYGON, 0, 4, &px_p));
    expect_square(px_q, "QUADS");
    expect_square(px_qs, "QUAD_STRIP");
    expect_square(px_p, "POLYGON");
    check_reference("geom_quads", px_q);
    check_case(app, base, 3, 3);
}

// ---- 2c. DrawElementsBaseVertex: indices 0..3 read vertices 4..7 ------------------------------------------------------------
NXTEST(draw_elements_base_vertex) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs;
    auto prog = make_prog(app, kVsPos, kFsMagenta, &vs, &fs);
    REQUIRE(prog != nullptr);
    // vertices 0..3: a decoy square in the top-left corner, never drawn; vertices 4..7: the square of the other tests
    const float verts[] = {-0.9f, 0.9f, 0, -0.6f, 0.9f, 0, -0.6f, 0.6f, 0, -0.9f, 0.6f, 0,
                           -0.5f, -0.5f, 0, 0.5f, -0.5f, 0, 0.5f, 0.5f, 0, -0.5f, 0.5f, 0};
    const uint16_t idx[] = {0, 1, 2, 0, 2, 3};
    const App::Buf vb = app.make_buffer(sizeof verts, verts);
    const App::Buf ib = app.make_buffer(sizeof idx, idx);
    std::vector<uint8_t> px;
    REQUIRE(frame(app, [&](NVNcommandBuffer* c) {
        bind_states(app, c, polygon(app, NVN_FACE_BACK, NVN_FRONT_FACE_CCW, NVN_POLYGON_MODE_FILL));
        bind_position(app, c, vb);
        bind_program(app, c, prog.get());
        app.api.nvnCommandBufferDrawElementsBaseVertex(c, NVN_DRAW_PRIMITIVE_TRIANGLES, NVN_INDEX_TYPE_UNSIGNED_SHORT, 6, ib.address, 4);
    }, &px));
    const Census decoy = census(px, 0, 0, 16, 12);
    std::printf("    decoy corner (x<16, y<12): %zu magenta\n", decoy.magenta);
    CHECK_EQ(decoy.magenta, size_t(0));
    expect_square(px, "DrawElementsBaseVertex");
    check_case(app, base, 1, 1);
}

// ---- 2d. instancing: DrawElementsInstanced with a divisor-1 stream -----------------------------------------------------------
// Unit square with per-instance offsets (-0.5, 0) and (+0.5, 0), scale 0.25: two 16 x 12 pixel squares, at x 8..24 and 40..56.
NXTEST(instanced_elements_divisor) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs;
    auto prog = make_prog(app, kVsInstance, kFsMagenta, &vs, &fs);
    REQUIRE(prog != nullptr);
    const float unit[] = {-1, -1, 0, 1, 1, -1, 0, 1, 1, 1, 0, 1, -1, -1, 0, 1, 1, 1, 0, 1, -1, 1, 0, 1};
    const uint16_t idx[] = {0, 1, 2, 0, 2, 5};
    const float inst[] = {-0.5f, 0, 0.25f, 0, 0.5f, 0, 0.25f, 0};
    const App::Buf vb = app.make_buffer(sizeof unit, unit);
    const App::Buf ib = app.make_buffer(sizeof idx, idx);
    const App::Buf ibuf = app.make_buffer(sizeof inst, inst);
    std::vector<uint8_t> px;
    REQUIRE(frame(app, [&](NVNcommandBuffer* c) {
        bind_states(app, c, polygon(app, NVN_FACE_BACK, NVN_FRONT_FACE_CCW, NVN_POLYGON_MODE_FILL));
        bind_instanced(app, c, vb, ibuf);
        bind_program(app, c, prog.get());
        app.api.nvnCommandBufferDrawElementsInstanced(c, NVN_DRAW_PRIMITIVE_TRIANGLES, NVN_INDEX_TYPE_UNSIGNED_SHORT, 6, ib.address, 0, 0, 2);
    }, &px));
    const Census left = census(px, 0, 0, 32, kH), right = census(px, 32, 0, kW, kH);
    const Census all = census(px);
    std::printf("    instances: left half %zu magenta, right half %zu magenta, total %zu, other %zu\n", left.magenta, right.magenta, all.magenta,
                all.other);
    CHECK_EQ(left.magenta, size_t(192));
    CHECK_EQ(right.magenta, size_t(192));
    CHECK_EQ(all.other, size_t(0));
    CHECK(is_magenta(px, 16, 24) && is_magenta(px, 48, 24) && is_black(px, 32, 24));
    check_reference("geom_instanced", px);
    check_case(app, base, 1, 1);
}

// ---- 2e. DrawArraysInstanced with a draw-time base instance: only instance 1 is drawn -------------------------------------
NXTEST(instanced_arrays_base_instance) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs;
    auto prog = make_prog(app, kVsInstance, kFsMagenta, &vs, &fs);
    REQUIRE(prog != nullptr);
    const float unit[] = {-1, -1, 0, 1, 1, -1, 0, 1, 1, 1, 0, 1, -1, -1, 0, 1, 1, 1, 0, 1, -1, 1, 0, 1};
    const float inst[] = {-0.5f, 0, 0.25f, 0, 0.5f, 0, 0.25f, 0};
    const App::Buf vb = app.make_buffer(sizeof unit, unit);
    const App::Buf ibuf = app.make_buffer(sizeof inst, inst);
    std::vector<uint8_t> px;
    REQUIRE(frame(app, [&](NVNcommandBuffer* c) {
        bind_states(app, c, polygon(app, NVN_FACE_BACK, NVN_FRONT_FACE_CCW, NVN_POLYGON_MODE_FILL));
        bind_instanced(app, c, vb, ibuf);
        bind_program(app, c, prog.get());
        app.api.nvnCommandBufferDrawArraysInstanced(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 6, 1, 1);
    }, &px));
    const Census left = census(px, 0, 0, 32, kH), right = census(px, 32, 0, kW, kH);
    std::printf("    base instance 1, count 1: left half %zu magenta, right half %zu magenta\n", left.magenta, right.magenta);
    CHECK_EQ(left.magenta, size_t(0));
    CHECK_EQ(right.magenta, size_t(192));
    CHECK_EQ(census(px).other, size_t(0));
    check_case(app, base, 1, 1);
}

// ---- 2f. primitive restart: a strip split into two triangles, U16 (0xFFFF) and U32 (0xFFFFFFFF) ---------------------------
NXTEST(primitive_restart_u16_u32) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs;
    auto prog = make_prog(app, kVsPos, kFsMagenta, &vs, &fs);
    REQUIRE(prog != nullptr);
    // A = (v0,v1,v2) bottom-left, B = (v3,v4,v5) top-right. Without the restart, (v1,v2,v3) would join them: the centre is the probe.
    const float verts[] = {-0.9f, -0.9f, 0, -0.1f, -0.9f, 0, -0.9f, -0.1f, 0, 0.1f, 0.9f, 0, 0.9f, 0.9f, 0, 0.9f, 0.1f, 0};
    const App::Buf vb = app.make_buffer(sizeof verts, verts);
    const uint16_t i16[] = {0, 1, 2, 0xFFFF, 3, 4, 5};
    const uint32_t i32[] = {0, 1, 2, 0xFFFFFFFFu, 3, 4, 5};
    const App::Buf ib16 = app.make_buffer(sizeof i16, i16);
    const App::Buf ib32 = app.make_buffer(sizeof i32, i32);
    std::vector<uint8_t> px16, px32;
    REQUIRE(frame(app, [&](NVNcommandBuffer* c) {
        bind_states(app, c, polygon(app, NVN_FACE_NONE, NVN_FRONT_FACE_CCW, NVN_POLYGON_MODE_FILL));   // no culling: the triangles differ in winding
        bind_position(app, c, vb);
        bind_program(app, c, prog.get());
        app.api.nvnCommandBufferSetPrimitiveRestart(c, 1, 0xFFFF);
        app.api.nvnCommandBufferDrawElements(c, NVN_DRAW_PRIMITIVE_TRIANGLE_STRIP, NVN_INDEX_TYPE_UNSIGNED_SHORT, 7, ib16.address);
    }, &px16));
    REQUIRE(frame(app, [&](NVNcommandBuffer* c) {
        bind_states(app, c, polygon(app, NVN_FACE_NONE, NVN_FRONT_FACE_CCW, NVN_POLYGON_MODE_FILL));
        bind_position(app, c, vb);
        bind_program(app, c, prog.get());
        app.api.nvnCommandBufferSetPrimitiveRestart(c, 1, static_cast<int>(0xFFFFFFFFu));
        app.api.nvnCommandBufferDrawElements(c, NVN_DRAW_PRIMITIVE_TRIANGLE_STRIP, NVN_INDEX_TYPE_UNSIGNED_INT, 7, ib32.address);
    }, &px32));
    for (int k = 0; k < 2; ++k) {
        const std::vector<uint8_t>& px = k ? px32 : px16;
        const Census c = census(px);
        std::printf("    %s restart: %zu magenta, %zu other; A probe (12,38) %s, B probe (51,9) %s, bridge probe (22,24) %s\n", k ? "U32" : "U16",
                    c.magenta, c.other, is_magenta(px, 12, 38) ? "magenta" : "black", is_magenta(px, 51, 9) ? "magenta" : "black",
                    is_black(px, 22, 24) ? "black" : "magenta");
        // two right triangles of 245.76 px each, plus edge pixels; with the bridge drawn the count would pass 1100
        CHECK(c.magenta >= 480 && c.magenta <= 540);
        CHECK_EQ(c.other, size_t(0));
        CHECK(is_magenta(px, 12, 38) && is_magenta(px, 51, 9));
        CHECK(is_black(px, 22, 24));
    }
    CHECK(px16 == px32);
    check_reference("geom_restart", px16);
    check_case(app, base, 2, 2);
}

// ---- 4. viewport: a sub-rectangle receives the quad, everything outside stays black ------------------------------------------
NXTEST(viewport_sub_rectangle) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs;
    auto prog = make_prog(app, kVsPos, kFsMagenta, &vs, &fs);
    REQUIRE(prog != nullptr);
    const float quad[] = {-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, -1, 0, 1, 1, 0, -1, 1, 0};
    const App::Buf vb = app.make_buffer(sizeof quad, quad);
    std::vector<uint8_t> px;
    REQUIRE(frame(app, [&](NVNcommandBuffer* c) {
        app.api.nvnCommandBufferSetViewport(c, 0, 0, 32, 24);   // the top-left quarter
        bind_states(app, c, polygon(app, NVN_FACE_BACK, NVN_FRONT_FACE_CCW, NVN_POLYGON_MODE_FILL));
        bind_position(app, c, vb);
        bind_program(app, c, prog.get());
        app.api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 6);
    }, &px));
    const Census all = census(px), quarter = census(px, 0, 0, 32, 24);
    std::printf("    viewport 0,0,32,24: %zu magenta in total, %zu inside the viewport, %zu other\n", all.magenta, quarter.magenta, all.other);
    CHECK_EQ(all.magenta, size_t(768));
    CHECK_EQ(quarter.magenta, size_t(768));
    CHECK_EQ(all.other, size_t(0));
    check_reference("geom_viewport", px);
    check_case(app, base, 1, 1);
}

// ---- 4b. scissor: the full-screen quad is cut to the scissor rectangle x 16..47, y 12..35 --------------------------------
NXTEST(scissor_cuts_quad) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs;
    auto prog = make_prog(app, kVsPos, kFsMagenta, &vs, &fs);
    REQUIRE(prog != nullptr);
    const float quad[] = {-1, -1, 0, 1, -1, 0, 1, 1, 0, -1, -1, 0, 1, 1, 0, -1, 1, 0};
    const App::Buf vb = app.make_buffer(sizeof quad, quad);
    std::vector<uint8_t> px;
    REQUIRE(frame(app, [&](NVNcommandBuffer* c) {
        app.api.nvnCommandBufferSetScissor(c, 16, 12, 32, 24);
        bind_states(app, c, polygon(app, NVN_FACE_BACK, NVN_FRONT_FACE_CCW, NVN_POLYGON_MODE_FILL));
        bind_position(app, c, vb);
        bind_program(app, c, prog.get());
        app.api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 6);
    }, &px));
    const Census all = census(px), box = census(px, 16, 12, 48, 36);
    std::printf("    scissor 16,12,32,24: %zu magenta in total, %zu inside the scissor, %zu other\n", all.magenta, box.magenta, all.other);
    CHECK_EQ(all.magenta, size_t(768));
    CHECK_EQ(box.magenta, size_t(768));
    CHECK_EQ(all.other, size_t(0));
    CHECK(is_magenta(px, 16, 12) && is_black(px, 15, 12) && is_black(px, 48, 35));
    check_reference("geom_scissor", px);
    check_case(app, base, 1, 1);
}

// ---- 3b. depth mode: NEAR_IS_ZERO and NEAR_IS_MINUS_W draw a flat triangle the same ----------------------------------------
NXTEST(depth_mode_near_is_zero) {
    depth_case(true, "geom_depth_near_is_zero");
}
NXTEST(depth_mode_near_is_minus_w) {
    depth_case(false, "geom_depth_near_is_minus_w");
}

// ---- 1b. POINTS: three points at pixel centres, one pixel each ------------------------------------------------------------
NXTEST(prim_points) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs;
    auto prog = make_prog(app, kVsPos, kFsMagenta, &vs, &fs);
    REQUIRE(prog != nullptr);
    // the centres of pixels (16,12), (32,24) and (47,35)
    const float pts[] = {-0.484375f, 0.4791667f, 0, 0.015625f, -0.0208333f, 0, 0.484375f, -0.4791667f, 0};
    const App::Buf vb = app.make_buffer(sizeof pts, pts);
    std::vector<uint8_t> px;
    REQUIRE(frame(app, [&](NVNcommandBuffer* c) {
        app.api.nvnCommandBufferSetPointSize(c, 1.0f);
        bind_states(app, c, polygon(app, NVN_FACE_BACK, NVN_FRONT_FACE_CCW, NVN_POLYGON_MODE_FILL));
        bind_position(app, c, vb);
        bind_program(app, c, prog.get());
        app.api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_POINTS, 0, 3);
    }, &px));
    const Census c = census(px);
    std::printf("    POINTS: %zu magenta, %zu other; (16,12) %s, (32,24) %s, (47,35) %s\n", c.magenta, c.other,
                is_magenta(px, 16, 12) ? "magenta" : "black", is_magenta(px, 32, 24) ? "magenta" : "black",
                is_magenta(px, 47, 35) ? "magenta" : "black");
    CHECK_EQ(c.magenta, size_t(3));
    CHECK_EQ(c.other, size_t(0));
    CHECK(is_magenta(px, 16, 12) && is_magenta(px, 32, 24) && is_magenta(px, 47, 35));
    check_reference("geom_points", px);
    check_case(app, base, 1, 1);
}

// ---- 1c. LINES, LINE_STRIP, LINE_LOOP (the last converted by the back end) ------------------------------------------------
NXTEST(prim_lines_strip_loop) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs;
    auto prog = make_prog(app, kVsPos, kFsMagenta, &vs, &fs);
    REQUIRE(prog != nullptr);
    const float lines[] = {-0.5f, 0, 0, 0.5f, 0, 0, 0, -0.5f, 0, 0, 0.5f, 0};   // a horizontal and a vertical segment
    const float strip[] = {-0.5f, -0.5f, 0, 0.5f, -0.5f, 0, 0.5f, 0.5f, 0};
    const float loop[] = {-0.5f, -0.5f, 0, 0.5f, -0.5f, 0, 0.5f, 0.5f, 0, -0.5f, 0.5f, 0};
    const App::Buf vl = app.make_buffer(sizeof lines, lines);
    const App::Buf vs3 = app.make_buffer(sizeof strip, strip);
    const App::Buf vlo = app.make_buffer(sizeof loop, loop);
    std::vector<uint8_t> px_l, px_s, px_o;
    REQUIRE(draw_positions(app, prog.get(), vl, NVN_DRAW_PRIMITIVE_LINES, 0, 4, &px_l));
    REQUIRE(draw_positions(app, prog.get(), vs3, NVN_DRAW_PRIMITIVE_LINE_STRIP, 0, 3, &px_s));
    REQUIRE(draw_positions(app, prog.get(), vlo, NVN_DRAW_PRIMITIVE_LINE_LOOP, 0, 4, &px_o));

    const Census l = census(px_l), lh = census(px_l, 16, 22, 48, 26), lv = census(px_l, 30, 12, 34, 36);
    std::printf("    LINES: %zu magenta in total, horizontal band %zu, vertical band %zu, other %zu\n", l.magenta, lh.magenta, lv.magenta, l.other);
    CHECK(l.magenta >= 48 && l.magenta <= 64);
    CHECK(lh.magenta >= 28);
    CHECK(lv.magenta >= 20);
    CHECK_EQ(census(px_l, 0, 0, 8, 8).magenta, size_t(0));
    CHECK_EQ(census(px_l, 56, 40, kW, kH).magenta, size_t(0));
    CHECK_EQ(l.other, size_t(0));

    const Census s = census(px_s), sb = census(px_s, 16, 34, 48, 38), sr = census(px_s, 46, 12, 50, 36), si = census(px_s, 20, 16, 44, 32);
    std::printf("    LINE_STRIP: %zu magenta in total, bottom edge %zu, right edge %zu, interior %zu, other %zu\n", s.magenta, sb.magenta, sr.magenta,
                si.magenta, s.other);
    CHECK(s.magenta >= 45 && s.magenta <= 70);
    CHECK(sb.magenta >= 28);
    CHECK(sr.magenta >= 20);
    CHECK_EQ(si.magenta, size_t(0));
    CHECK_EQ(s.other, size_t(0));

    const Census lo = census(px_o), inside = census(px_o, 20, 16, 44, 32), left = census(px_o, 14, 14, 18, 34);
    std::printf("    LINE_LOOP: %zu magenta in total, interior %zu, left edge %zu, other %zu\n", lo.magenta, inside.magenta, left.magenta, lo.other);
    CHECK(lo.magenta >= 100 && lo.magenta <= 120);
    CHECK_EQ(inside.magenta, size_t(0));
    CHECK(left.magenta >= 20);
    CHECK_EQ(lo.other, size_t(0));
    check_reference("geom_lines_loop", px_o);
    check_case(app, base, 3, 3);
}

// ---- 5. culling and winding: CULL BACK / FRONT / NONE with CCW and CW triangles -------------------------------------------
NXTEST(cull_face_and_winding) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs;
    auto prog = make_prog(app, kVsPos, kFsMagenta, &vs, &fs);
    REQUIRE(prog != nullptr);
    // vertices 0..2: counter-clockwise in NDC (x right, y up); vertices 3..5: the same triangle clockwise
    const float verts[] = {-0.5f, -0.5f, 0, 0.5f, -0.5f, 0, 0.0f, 0.5f, 0, -0.5f, -0.5f, 0, 0.0f, 0.5f, 0, 0.5f, -0.5f, 0};
    const App::Buf vb = app.make_buffer(sizeof verts, verts);
    struct Case {
        const char* name;
        int cull, front, first;
        bool drawn;
    };
    const Case cases[] = {
        {"BACK, CCW front, CCW triangle", NVN_FACE_BACK, NVN_FRONT_FACE_CCW, 0, true},
        {"BACK, CCW front, CW triangle", NVN_FACE_BACK, NVN_FRONT_FACE_CCW, 3, false},
        {"FRONT, CCW front, CCW triangle", NVN_FACE_FRONT, NVN_FRONT_FACE_CCW, 0, false},
        {"FRONT, CCW front, CW triangle", NVN_FACE_FRONT, NVN_FRONT_FACE_CCW, 3, true},
        {"NONE, CCW front, CW triangle", NVN_FACE_NONE, NVN_FRONT_FACE_CCW, 3, true},
        {"BACK, CW front, CW triangle", NVN_FACE_BACK, NVN_FRONT_FACE_CW, 3, true},
        {"BACK, CW front, CCW triangle", NVN_FACE_BACK, NVN_FRONT_FACE_CW, 0, false},
    };
    for (const Case& k : cases) {
        std::vector<uint8_t> px;
        const bool ok = frame(app, [&](NVNcommandBuffer* c) {
            bind_states(app, c, polygon(app, k.cull, k.front, NVN_POLYGON_MODE_FILL));
            bind_position(app, c, vb);
            bind_program(app, c, prog.get());
            app.api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_TRIANGLES, k.first, 3);
        }, &px);
        REQUIRE(ok);
        const Census c = census(px);
        std::printf("    %-32s %s: %zu magenta, %zu other\n", k.name, k.drawn ? "drawn" : "culled", c.magenta, c.other);
        CHECK_EQ(c.other, size_t(0));
        if (k.drawn) CHECK(c.magenta >= 360 && c.magenta <= 410);
        else CHECK_EQ(c.magenta, size_t(0));
    }
    check_case(app, base, 7, 7);
}

// ---- 5b. polygon mode LINE and POINT --------------------------------------------------------------------------------------
NXTEST(polygon_mode_line_and_point) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs;
    auto prog = make_prog(app, kVsPos, kFsMagenta, &vs, &fs);
    REQUIRE(prog != nullptr);
    const float tri[] = {-0.5f, -0.5f, 0, 0.5f, -0.5f, 0, 0.0f, 0.5f, 0};
    const App::Buf vb = app.make_buffer(sizeof tri, tri);
    std::vector<uint8_t> px_line, px_point;
    REQUIRE(frame(app, [&](NVNcommandBuffer* c) {
        bind_states(app, c, polygon(app, NVN_FACE_NONE, NVN_FRONT_FACE_CCW, NVN_POLYGON_MODE_LINE));
        bind_position(app, c, vb);
        bind_program(app, c, prog.get());
        app.api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
    }, &px_line));
    REQUIRE(frame(app, [&](NVNcommandBuffer* c) {
        bind_states(app, c, polygon(app, NVN_FACE_NONE, NVN_FRONT_FACE_CCW, NVN_POLYGON_MODE_POINT));
        bind_position(app, c, vb);
        bind_program(app, c, prog.get());
        app.api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
    }, &px_point));

    const Census l = census(px_line), lc = census(px_line, 26, 26, 38, 30);
    std::printf("    LINE mode: %zu magenta in total, centre box %zu, other %zu\n", l.magenta, lc.magenta, l.other);
    CHECK(l.magenta >= 70 && l.magenta <= 100);
    CHECK_EQ(lc.magenta, size_t(0));
    CHECK_EQ(l.other, size_t(0));

    const Census p = census(px_point), pc = census(px_point, 26, 26, 38, 30);
    const Census n1 = census(px_point, 15, 35, 18, 38), n2 = census(px_point, 47, 35, 50, 38), n3 = census(px_point, 31, 11, 34, 14);
    std::printf("    POINT mode: %zu magenta in total, centre box %zu, around vertices %zu %zu %zu, other %zu\n", p.magenta, pc.magenta, n1.magenta,
                n2.magenta, n3.magenta, p.other);
    CHECK(p.magenta >= 3 && p.magenta <= 12);
    CHECK_EQ(pc.magenta, size_t(0));
    CHECK(n1.magenta >= 1 && n2.magenta >= 1 && n3.magenta >= 1);
    CHECK_EQ(p.other, size_t(0));
    check_reference("geom_polygon_line", px_line);
    check_case(app, base, 2, 2);
}

// ---- 6. several draws and state changes in one command buffer ------------------------------------------------------------
NXTEST(multi_draw_state_changes) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs, cvs, cfs;
    auto prog = make_prog(app, kVsPos, kFsMagenta, &vs, &fs);
    auto prog_c = make_prog(app, kVsColour, kFsPass, &cvs, &cfs);
    REQUIRE(prog != nullptr);
    REQUIRE(prog_c != nullptr);
    // buffer A: left square (x -0.875..-0.125) then right square (x 0.125..0.875), both y -0.5..0.5
    const float sq[] = {-0.875f, -0.5f, 0, -0.125f, -0.5f, 0, -0.125f, 0.5f, 0, -0.875f, -0.5f, 0, -0.125f, 0.5f, 0, -0.875f, 0.5f, 0,
                        0.125f, -0.5f, 0, 0.875f, -0.5f, 0, 0.875f, 0.5f, 0, 0.125f, -0.5f, 0, 0.875f, 0.5f, 0, 0.125f, 0.5f, 0};
    // buffer B: green square x -0.25..0.25, y 0.5..1.0, position (xyz) and colour (rgba) per vertex
    const float gr[] = {-0.25f, 0.5f, 0, 0, 1, 0, 1, 0.25f, 0.5f, 0, 0, 1, 0, 1, 0.25f, 1.0f, 0, 0, 1, 0, 1,
                        -0.25f, 0.5f, 0, 0, 1, 0, 1, 0.25f, 1.0f, 0, 0, 1, 0, 1, -0.25f, 1.0f, 0, 0, 1, 0, 1};
    const uint8_t right_idx[] = {6, 7, 8, 9, 10, 11};
    const App::Buf va = app.make_buffer(sizeof sq, sq);
    const App::Buf vbg = app.make_buffer(sizeof gr, gr);
    const App::Buf ib = app.make_buffer(sizeof right_idx, right_idx);
    std::vector<uint8_t> px;
    REQUIRE(frame(app, [&](NVNcommandBuffer* c) {
        bind_states(app, c, polygon(app, NVN_FACE_NONE, NVN_FRONT_FACE_CCW, NVN_POLYGON_MODE_FILL));
        // 1: magenta program, DrawArrays, the left square
        bind_program(app, c, prog.get());
        bind_position(app, c, va);
        app.api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 6);
        // 2: switch to the colour program, its own vertex layout and buffer: the green square at the top
        bind_program(app, c, prog_c.get());
        bind_colour(app, c, vbg);
        app.api.nvnCommandBufferDrawArrays(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 6);
        // 3: back to the magenta program and buffer A, DrawElements over the right square
        bind_program(app, c, prog.get());
        bind_position(app, c, va);
        app.api.nvnCommandBufferDrawElements(c, NVN_DRAW_PRIMITIVE_TRIANGLES, NVN_INDEX_TYPE_UNSIGNED_BYTE, 6, ib.address);
    }, &px));
    const Census all = census(px), g = census(px, 0, 0, kW, 12);
    std::printf("    magenta %zu (expect 1152), green %zu (expect 192), green rows only %zu, other %zu\n", all.magenta, all.green, g.green, all.other);
    CHECK_EQ(all.magenta, size_t(1152));
    CHECK_EQ(all.green, size_t(192));
    CHECK_EQ(g.green, size_t(192));
    CHECK_EQ(all.other, size_t(0));
    CHECK(is_magenta(px, 16, 24) && is_magenta(px, 48, 24) && is_black(px, 32, 24) && is_black(px, 32, 20));
    CHECK(px[(static_cast<size_t>(4) * kW + 32) * 4 + 1] == 255);   // green at (32, 4)
    check_reference("geom_multi_draw", px);
    check_case(app, base, 3, 1);
}

// ---- 6b. two command buffers submitted one after the other: the second draws with the state the first one bound ---------
NXTEST(two_command_buffers_state_persists) {
    if (!have_data()) return;
    App app;
    App::Opts o;
    o.width = kW;
    o.height = kH;
    REQUIRE(app.init(o));
    const Stats base = stats_now();
    App::ShaderBlob vs, fs;
    auto prog = make_prog(app, kVsPos, kFsMagenta, &vs, &fs);
    REQUIRE(prog != nullptr);
    // vertices 0..2: triangle A (lower left), vertices 3..5: triangle B (lower right, mirrored)
    const float verts[] = {-0.875f, -0.5f, 0, -0.125f, -0.5f, 0, -0.875f, 0.5f, 0, 0.125f, -0.5f, 0, 0.875f, -0.5f, 0, 0.875f, 0.5f, 0};
    const App::Buf vb = app.make_buffer(sizeof verts, verts);
    const int idx = app.acquire();
    REQUIRE(idx >= 0);
    NVNtexture* targets[] = {app.window_textures[idx]->obj.get()};
    // command buffer A: targets, viewport, scissor, clear, all the state, draws triangle A
    auto a = app.make_cmdbuf();
    REQUIRE(a != nullptr);
    NVNcommandBuffer* ca = a->obj.get();
    app.api.nvnCommandBufferBeginRecording(ca);
    app.api.nvnCommandBufferSetRenderTargets(ca, 1, targets, nullptr, nullptr, nullptr);
    app.api.nvnCommandBufferSetViewport(ca, 0, 0, kW, kH);
    app.api.nvnCommandBufferSetScissor(ca, 0, 0, kW, kH);
    const float black[4] = {0, 0, 0, 1};
    app.api.nvnCommandBufferClearColor(ca, 0, black, NVN_CLEAR_COLOR_MASK_RGBA);
    bind_states(app, ca, polygon(app, NVN_FACE_NONE, NVN_FRONT_FACE_CCW, NVN_POLYGON_MODE_FILL));
    bind_program(app, ca, prog.get());
    bind_position(app, ca, vb);
    app.api.nvnCommandBufferDrawArrays(ca, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
    app.end_and_submit(*a);
    // command buffer B: only a draw. Program, vertex input and buffer, polygon state, targets and viewport are the ones A bound.
    auto b = app.make_cmdbuf();
    REQUIRE(b != nullptr);
    NVNcommandBuffer* cb = b->obj.get();
    app.api.nvnCommandBufferBeginRecording(cb);
    app.api.nvnCommandBufferDrawArrays(cb, NVN_DRAW_PRIMITIVE_TRIANGLES, 3, 3);
    app.end_and_submit(*b);
    std::vector<uint8_t> px;
    REQUIRE(app.present_and_capture(idx, &px));
    const Census all = census(px), left = census(px, 0, 0, 32, kH), right = census(px, 32, 0, kW, kH);
    std::printf("    magenta %zu (left %zu, right %zu), other %zu; A probe (8,30) %s, B probe (56,30) %s, centre (32,24) %s\n", all.magenta, left.magenta,
                right.magenta, all.other, is_magenta(px, 8, 30) ? "magenta" : "black", is_magenta(px, 56, 30) ? "magenta" : "black",
                is_black(px, 32, 24) ? "black" : "magenta");
    CHECK(left.magenta >= 265 && left.magenta <= 310);
    CHECK(right.magenta >= 265 && right.magenta <= 310);
    CHECK_EQ(all.other, size_t(0));
    CHECK(is_magenta(px, 8, 30) && is_magenta(px, 56, 30) && is_black(px, 32, 24));
    check_reference("geom_two_command_buffers", px);
    check_case(app, base, 2, 2);
}
