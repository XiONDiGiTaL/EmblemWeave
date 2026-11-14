// SPDX-License-Identifier: GPL-3.0-or-later
// State and bind commands recorded into a command list (api_cmd_state.cpp). Each test records through abi::proc, ends the
// recording and walks the list with RecordReader, checking op, payload fields, blob bytes and record size.
#include <cstring>

#include "nvn_cmd.h"
#include "nvn_impl.h"
#include "test_framework.h"
NXTEST_GROUP(gfx_nvn_cmd_state)

using namespace nxrt;
using namespace nxrt::nvn;
using namespace nxrt::nvn::be;

namespace {
void setup() {
    install();
    abi::reset_mismatches();
    register_api_cmdbuf();   // prerequisite (lifecycle, Begin/EndRecording); its own mismatch is counted and reset below
    abi::reset_mismatches();  // api_cmdbuf.cpp: nvnCommandBufferGetMemoryCallbackData returns void*, the table says void
    register_api_cmd_state();
    register_api_texture_builder();   // nvnTextureViewSet* (the render target views)
    CHECK_EQ(abi::mismatches(), 0u);
}

const char* const kNames[] = {
    "nvnCommandBufferBindBlendState",
    "nvnCommandBufferBindChannelMaskState",
    "nvnCommandBufferBindColorState",
    "nvnCommandBufferBindMultisampleState",
    "nvnCommandBufferBindPolygonState",
    "nvnCommandBufferBindDepthStencilState",
    "nvnCommandBufferBindVertexAttribState",
    "nvnCommandBufferBindVertexStreamState",
    "nvnCommandBufferBindProgram",
    "nvnCommandBufferBindVertexBuffer",
    "nvnCommandBufferBindVertexBuffers",
    "nvnCommandBufferBindUniformBuffer",
    "nvnCommandBufferBindUniformBuffers",
    "nvnCommandBufferBindStorageBuffer",
    "nvnCommandBufferBindStorageBuffers",
    "nvnCommandBufferBindTransformFeedbackBuffer",
    "nvnCommandBufferBindTransformFeedbackBuffers",
    "nvnCommandBufferBindTexture",
    "nvnCommandBufferBindTextures",
    "nvnCommandBufferBindImage",
    "nvnCommandBufferBindImages",
    "nvnCommandBufferSetTexturePool",
    "nvnCommandBufferSetSamplerPool",
    "nvnCommandBufferSetViewport",
    "nvnCommandBufferSetViewports",
    "nvnCommandBufferSetViewportSwizzles",
    "nvnCommandBufferSetScissor",
    "nvnCommandBufferSetScissors",
    "nvnCommandBufferSetDepthRange",
    "nvnCommandBufferSetDepthRanges",
    "nvnCommandBufferSetDepthBounds",
    "nvnCommandBufferSetStencilRef",
    "nvnCommandBufferSetStencilValueMask",
    "nvnCommandBufferSetStencilMask",
    "nvnCommandBufferSetBlendColor",
    "nvnCommandBufferSetPointSize",
    "nvnCommandBufferSetLineWidth",
    "nvnCommandBufferSetAlphaRef",
    "nvnCommandBufferSetPolygonOffsetClamp",
    "nvnCommandBufferSetSampleMask",
    "nvnCommandBufferSetRasterizerDiscard",
    "nvnCommandBufferSetDepthClamp",
    "nvnCommandBufferSetConservativeRasterEnable",
    "nvnCommandBufferSetConservativeRasterDilate",
    "nvnCommandBufferSetSubpixelPrecisionBias",
    "nvnCommandBufferSetPrimitiveRestart",
    "nvnCommandBufferSetPatchSize",
    "nvnCommandBufferSetInnerTessellationLevels",
    "nvnCommandBufferSetOuterTessellationLevels",
    "nvnCommandBufferSetRenderEnable",
    "nvnCommandBufferSetRenderEnableConditional",
    "nvnCommandBufferSetRenderTargets",
    "nvnCommandBufferBindCoverageModulationTable",
    "nvnCommandBufferSetShaderScratchMemory",
    "nvnCommandBufferSetProgramSubroutines",
    "nvnCommandBufferSetTiledCacheAction",
    "nvnCommandBufferSetTiledCacheTileSize",
    "nvnCommandBufferBindSeparateTexture",
    "nvnCommandBufferBindSeparateSampler",
    "nvnCommandBufferBindSeparateTextures",
    "nvnCommandBufferBindSeparateSamplers",
};

// A command buffer with a null device, recording from the start unless `begin` is false. Finalized at scope exit.
struct Recorder {
    NVNcommandBuffer cb;
    explicit Recorder(bool begin = true) {
        std::memset(&cb, 0, sizeof cb);
        const NVNboolean ok = abi::proc<PFNNVNCOMMANDBUFFERINITIALIZEPROC>("nvnCommandBufferInitialize")(&cb, nullptr);
        CHECK(ok);
        if (begin) abi::proc<PFNNVNCOMMANDBUFFERBEGINRECORDINGPROC>("nvnCommandBufferBeginRecording")(&cb);
    }
    ~Recorder() { abi::proc<PFNNVNCOMMANDBUFFERFINALIZEPROC>("nvnCommandBufferFinalize")(&cb); }
    Recorder(const Recorder&) = delete;
    Recorder& operator=(const Recorder&) = delete;
    // Ends the recording and returns its list (null when there was none).
    Ref<CmdList> end() {
        const NVNcommandHandle h = abi::proc<PFNNVNCOMMANDBUFFERENDRECORDINGPROC>("nvnCommandBufferEndRecording")(&cb);
        return handle_lookup(h);
    }
};

void expect_count(const Ref<CmdList>& list, uint32_t n) {
    CHECK(list);
    if (!list) return;
    CHECK_EQ(list->record_count, n);
}

// Advances to the next record and checks its op and total size. False when there is no such record.
bool expect_op(RecordReader& rd, Op op, uint32_t size) {
    if (!rd.next()) {
        CHECK(false);
        return false;
    }
    CHECK_EQ(static_cast<int>(rd.op()), static_cast<int>(op));
    CHECK_EQ(rd.size(), size);
    return true;
}

void expect_bytes(const uint8_t* got, const void* want, size_t n) { CHECK(std::memcmp(got, want, n) == 0); }
void expect_filled(const uint8_t* got, uint8_t v, size_t n) {
    for (size_t i = 0; i < n; ++i) CHECK_EQ(got[i], v);
}

void expect_u32(RecordReader& rd, Op op, uint32_t v) {
    if (!expect_op(rd, op, 16)) return;
    CHECK_EQ(rd.payload<P_U32>().v, v);
}
void expect_f32(RecordReader& rd, Op op, float v) {
    if (!expect_op(rd, op, 16)) return;
    CHECK_EQ(rd.payload<P_Float>().v, v);
}

// A state object with a pattern is recorded by value: the record keeps the bytes the object had at the call.
template <class S, class Bind>
void check_state_copy(Bind bind, Op op, uint8_t fill) {
    Recorder r;
    S s;
    std::memset(&s, fill, sizeof s);
    bind(&r.cb, &s);
    std::memset(&s, fill ^ 0xFF, sizeof s);   // the game reuses its object right after the call
    const Ref<CmdList> list = r.end();
    expect_count(list, 1);
    if (!list) return;
    RecordReader rd(*list);
    if (!expect_op(rd, op, 8 + 32)) return;
    const P_RawState& p = rd.payload<P_RawState>();
    CHECK_EQ(p.size, static_cast<uint32_t>(sizeof(S)));
    for (size_t i = 0; i < sizeof(S); ++i) CHECK_EQ(p.bytes[i], fill);
    for (size_t i = sizeof(S); i < sizeof p.bytes; ++i) CHECK_EQ(p.bytes[i], 0);
}

void expect_buffers(RecordReader& rd, uint32_t kind, uint32_t stage, uint32_t first, const NVNbufferRange* want, uint32_t count) {
    if (!expect_op(rd, Op::BindBuffers, 8 + 16 + 16 * count)) return;
    const P_BindBuffers& p = rd.payload<P_BindBuffers>();
    CHECK_EQ(p.kind, kind);
    CHECK_EQ(p.stage, stage);
    CHECK_EQ(p.first, first);
    CHECK_EQ(p.count, count);
    expect_bytes(rd.blob<P_BindBuffers>(), want, count * sizeof(NVNbufferRange));
}

constexpr uint32_t kRenderTargetsSize = 8 + static_cast<uint32_t>((sizeof(P_RenderTargets) + 7) & ~size_t{7});
}  // namespace

NXTEST(all_entry_points_registered) {
    setup();
    CHECK_EQ(sizeof(kNames) / sizeof(kNames[0]), 61u);
    for (const char* name : kNames) CHECK_MSG(is_implemented(name), name);
}

NXTEST(state_objects_recorded_by_value) {
    setup();
    check_state_copy<NVNblendState>(abi::proc<PFNNVNCOMMANDBUFFERBINDBLENDSTATEPROC>("nvnCommandBufferBindBlendState"), Op::BindBlend, 0x5A);
    check_state_copy<NVNchannelMaskState>(abi::proc<PFNNVNCOMMANDBUFFERBINDCHANNELMASKSTATEPROC>("nvnCommandBufferBindChannelMaskState"),
                                          Op::BindChannelMask, 0x3C);
    check_state_copy<NVNcolorState>(abi::proc<PFNNVNCOMMANDBUFFERBINDCOLORSTATEPROC>("nvnCommandBufferBindColorState"), Op::BindColor, 0x7E);
    check_state_copy<NVNmultisampleState>(abi::proc<PFNNVNCOMMANDBUFFERBINDMULTISAMPLESTATEPROC>("nvnCommandBufferBindMultisampleState"),
                                          Op::BindMultisample, 0x6B);
    check_state_copy<NVNpolygonState>(abi::proc<PFNNVNCOMMANDBUFFERBINDPOLYGONSTATEPROC>("nvnCommandBufferBindPolygonState"),
                                      Op::BindPolygon, 0x11);
    check_state_copy<NVNdepthStencilState>(abi::proc<PFNNVNCOMMANDBUFFERBINDDEPTHSTENCILSTATEPROC>("nvnCommandBufferBindDepthStencilState"),
                                           Op::BindDepthStencil, 0x2D);
}

NXTEST(vertex_attribs_and_streams_recorded) {
    setup();
    Recorder r;
    NVNvertexAttribState attr[2];
    std::memset(attr, 0x31, sizeof attr);
    NVNvertexStreamState streams[3];
    std::memset(streams, 0x42, sizeof streams);
    abi::proc<PFNNVNCOMMANDBUFFERBINDVERTEXATTRIBSTATEPROC>("nvnCommandBufferBindVertexAttribState")(&r.cb, 2, attr);
    abi::proc<PFNNVNCOMMANDBUFFERBINDVERTEXSTREAMSTATEPROC>("nvnCommandBufferBindVertexStreamState")(&r.cb, 3, streams);
    std::memset(attr, 0, sizeof attr);       // the game reuses its objects
    std::memset(streams, 0, sizeof streams);
    const Ref<CmdList> list = r.end();
    expect_count(list, 2);
    if (!list) return;
    RecordReader rd(*list);
    if (expect_op(rd, Op::BindVertexAttribs, 8 + 8 + 8)) {
        CHECK_EQ(rd.payload<P_BindVertexAttribs>().count, 2u);
        expect_filled(rd.blob<P_BindVertexAttribs>(), 0x31, sizeof attr);
    }
    if (expect_op(rd, Op::BindVertexStreams, 8 + 8 + 24)) {
        CHECK_EQ(rd.payload<P_BindVertexStreams>().count, 3u);
        expect_filled(rd.blob<P_BindVertexStreams>(), 0x42, sizeof streams);
    }
}

NXTEST(buffer_bindings_recorded_with_their_ranges) {
    setup();
    Recorder r;
    const NVNbufferRange ranges[3] = {{0x1000, 16}, {0x2000, 32}, {0x3000, 48}};
    abi::proc<PFNNVNCOMMANDBUFFERBINDVERTEXBUFFERPROC>("nvnCommandBufferBindVertexBuffer")(&r.cb, 2, 0x1000, 256);
    abi::proc<PFNNVNCOMMANDBUFFERBINDVERTEXBUFFERSPROC>("nvnCommandBufferBindVertexBuffers")(&r.cb, 1, 3, ranges);
    abi::proc<PFNNVNCOMMANDBUFFERBINDUNIFORMBUFFERPROC>("nvnCommandBufferBindUniformBuffer")(&r.cb, NVN_SHADER_STAGE_COMPUTE, 4, 0x4000, 64);
    abi::proc<PFNNVNCOMMANDBUFFERBINDUNIFORMBUFFERSPROC>("nvnCommandBufferBindUniformBuffers")(&r.cb, NVN_SHADER_STAGE_VERTEX, 0, 2, ranges);
    abi::proc<PFNNVNCOMMANDBUFFERBINDSTORAGEBUFFERPROC>("nvnCommandBufferBindStorageBuffer")(&r.cb, NVN_SHADER_STAGE_FRAGMENT, 1, 0x5000, 128);
    abi::proc<PFNNVNCOMMANDBUFFERBINDSTORAGEBUFFERSPROC>("nvnCommandBufferBindStorageBuffers")(&r.cb, NVN_SHADER_STAGE_COMPUTE, 2, 1, ranges + 2);
    abi::proc<PFNNVNCOMMANDBUFFERBINDTRANSFORMFEEDBACKBUFFERPROC>("nvnCommandBufferBindTransformFeedbackBuffer")(&r.cb, 0, 0x6000, 8);
    abi::proc<PFNNVNCOMMANDBUFFERBINDTRANSFORMFEEDBACKBUFFERSPROC>("nvnCommandBufferBindTransformFeedbackBuffers")(&r.cb, 1, 2, ranges);
    const Ref<CmdList> list = r.end();
    expect_count(list, 8);
    if (!list) return;
    RecordReader rd(*list);
    const NVNbufferRange one_v[1] = {{0x1000, 256}};
    expect_buffers(rd, kBufVertex, 0, 2, one_v, 1);
    expect_buffers(rd, kBufVertex, 0, 1, ranges, 3);
    const NVNbufferRange one_u[1] = {{0x4000, 64}};
    expect_buffers(rd, kBufUniform, static_cast<uint32_t>(NVN_SHADER_STAGE_COMPUTE), 4, one_u, 1);
    expect_buffers(rd, kBufUniform, static_cast<uint32_t>(NVN_SHADER_STAGE_VERTEX), 0, ranges, 2);
    const NVNbufferRange one_s[1] = {{0x5000, 128}};
    expect_buffers(rd, kBufStorage, static_cast<uint32_t>(NVN_SHADER_STAGE_FRAGMENT), 1, one_s, 1);
    expect_buffers(rd, kBufStorage, static_cast<uint32_t>(NVN_SHADER_STAGE_COMPUTE), 2, ranges + 2, 1);
    const NVNbufferRange one_x[1] = {{0x6000, 8}};
    expect_buffers(rd, kBufXfb, 0, 0, one_x, 1);
    expect_buffers(rd, kBufXfb, 0, 1, ranges, 2);
}

NXTEST(textures_and_images_recorded) {
    setup();
    Recorder r;
    const uint64_t handles[3] = {0x11, 0x22, 0x33};
    abi::proc<PFNNVNCOMMANDBUFFERBINDTEXTUREPROC>("nvnCommandBufferBindTexture")(&r.cb, NVN_SHADER_STAGE_VERTEX, 3, 0xABCDEF01ull);
    abi::proc<PFNNVNCOMMANDBUFFERBINDTEXTURESPROC>("nvnCommandBufferBindTextures")(&r.cb, NVN_SHADER_STAGE_COMPUTE, 0, 3, handles);
    abi::proc<PFNNVNCOMMANDBUFFERBINDIMAGEPROC>("nvnCommandBufferBindImage")(&r.cb, NVN_SHADER_STAGE_FRAGMENT, 2, 0x1122334455ull);
    abi::proc<PFNNVNCOMMANDBUFFERBINDIMAGESPROC>("nvnCommandBufferBindImages")(&r.cb, NVN_SHADER_STAGE_VERTEX, 1, 2, handles);
    const Ref<CmdList> list = r.end();
    expect_count(list, 4);
    if (!list) return;
    RecordReader rd(*list);
    const uint64_t one = 0xABCDEF01ull, img = 0x1122334455ull;
    if (expect_op(rd, Op::BindTextures, 8 + 16 + 8)) {
        const P_BindTextures& p = rd.payload<P_BindTextures>();
        CHECK_EQ(p.stage, static_cast<uint32_t>(NVN_SHADER_STAGE_VERTEX));
        CHECK_EQ(p.first, 3u);
        CHECK_EQ(p.count, 1u);
        expect_bytes(rd.blob<P_BindTextures>(), &one, 8);
    }
    if (expect_op(rd, Op::BindTextures, 8 + 16 + 24)) {
        const P_BindTextures& p = rd.payload<P_BindTextures>();
        CHECK_EQ(p.stage, static_cast<uint32_t>(NVN_SHADER_STAGE_COMPUTE));
        CHECK_EQ(p.first, 0u);
        CHECK_EQ(p.count, 3u);
        expect_bytes(rd.blob<P_BindTextures>(), handles, 24);
    }
    if (expect_op(rd, Op::BindImages, 8 + 16 + 8)) {
        const P_BindTextures& p = rd.payload<P_BindTextures>();
        CHECK_EQ(p.stage, static_cast<uint32_t>(NVN_SHADER_STAGE_FRAGMENT));
        CHECK_EQ(p.first, 2u);
        CHECK_EQ(p.count, 1u);
        expect_bytes(rd.blob<P_BindTextures>(), &img, 8);
    }
    if (expect_op(rd, Op::BindImages, 8 + 16 + 16)) {
        const P_BindTextures& p = rd.payload<P_BindTextures>();
        CHECK_EQ(p.stage, static_cast<uint32_t>(NVN_SHADER_STAGE_VERTEX));
        CHECK_EQ(p.first, 1u);
        CHECK_EQ(p.count, 2u);
        expect_bytes(rd.blob<P_BindTextures>(), handles, 16);
    }
}

NXTEST(program_and_pool_objects_are_held) {
    setup();
    Recorder r;
    auto* prog = new Program();   // the guest program owns one reference
    NVNprogram gprog;
    std::memset(&gprog, 0, sizeof gprog);
    bind_guest(&gprog, prog);
    CHECK_EQ(prog->refs.load(), 1);
    abi::proc<PFNNVNCOMMANDBUFFERBINDPROGRAMPROC>("nvnCommandBufferBindProgram")(&r.cb, &gprog, 3);
    CHECK_EQ(prog->refs.load(), 2);   // the list holds it too
    unbind_guest(&gprog);             // the game finalizes the program: the list keeps it alive
    CHECK_EQ(prog->refs.load(), 1);

    auto* tp = new TexturePool();
    NVNtexturePool gtp;
    std::memset(&gtp, 0, sizeof gtp);
    bind_guest(&gtp, tp);
    auto* sp = new SamplerPool();
    NVNsamplerPool gsp;
    std::memset(&gsp, 0, sizeof gsp);
    bind_guest(&gsp, sp);
    NVNtexturePool not_ours;   // a guest object without a host object
    std::memset(&not_ours, 0, sizeof not_ours);
    abi::proc<PFNNVNCOMMANDBUFFERSETTEXTUREPOOLPROC>("nvnCommandBufferSetTexturePool")(&r.cb, &gtp);
    abi::proc<PFNNVNCOMMANDBUFFERSETSAMPLERPOOLPROC>("nvnCommandBufferSetSamplerPool")(&r.cb, &gsp);
    abi::proc<PFNNVNCOMMANDBUFFERSETTEXTUREPOOLPROC>("nvnCommandBufferSetTexturePool")(&r.cb, &not_ours);
    CHECK_EQ(tp->refs.load(), 2);
    CHECK_EQ(sp->refs.load(), 2);
    unbind_guest(&gtp);
    unbind_guest(&gsp);
    CHECK_EQ(tp->refs.load(), 1);
    CHECK_EQ(sp->refs.load(), 1);

    const Ref<CmdList> list = r.end();
    expect_count(list, 4);
    if (!list) return;
    CHECK_EQ(list->held.size(), 3u);   // program, texture pool, sampler pool (the foreign pool holds nothing)
    CHECK(list->held[0].get() == static_cast<Obj*>(prog));
    CHECK(list->held[1].get() == static_cast<Obj*>(tp));
    CHECK(list->held[2].get() == static_cast<Obj*>(sp));
    RecordReader rd(*list);
    if (expect_op(rd, Op::BindProgram, 8 + 16)) {
        const P_BindProgram& p = rd.payload<P_BindProgram>();
        CHECK(p.program == prog);
        CHECK_EQ(p.stages, 3u);
    }
    if (expect_op(rd, Op::SetTexturePool, 8 + 8)) CHECK(rd.payload<P_Pool>().pool == tp);
    if (expect_op(rd, Op::SetSamplerPool, 8 + 8)) CHECK(rd.payload<P_Pool>().pool == sp);
    if (expect_op(rd, Op::SetTexturePool, 8 + 8)) CHECK(rd.payload<P_Pool>().pool == nullptr);
}

NXTEST(viewports_scissors_and_depth_ranges_recorded) {
    setup();
    Recorder r;
    abi::proc<PFNNVNCOMMANDBUFFERSETVIEWPORTPROC>("nvnCommandBufferSetViewport")(&r.cb, 1, 2, 300, 400);
    const float vps[8] = {0.f, 1.f, 2.f, 3.f, 4.f, 5.f, 6.f, 7.f};
    abi::proc<PFNNVNCOMMANDBUFFERSETVIEWPORTSPROC>("nvnCommandBufferSetViewports")(&r.cb, 2, 2, vps);
    abi::proc<PFNNVNCOMMANDBUFFERSETSCISSORPROC>("nvnCommandBufferSetScissor")(&r.cb, -5, 6, 7, 8);
    const int sc[8] = {1, 2, 3, 4, 5, 6, 7, 8};
    abi::proc<PFNNVNCOMMANDBUFFERSETSCISSORSPROC>("nvnCommandBufferSetScissors")(&r.cb, 1, 2, sc);
    abi::proc<PFNNVNCOMMANDBUFFERSETDEPTHRANGEPROC>("nvnCommandBufferSetDepthRange")(&r.cb, 0.25f, 0.75f);
    const float dr[4] = {0.1f, 0.2f, 0.3f, 0.4f};
    abi::proc<PFNNVNCOMMANDBUFFERSETDEPTHRANGESPROC>("nvnCommandBufferSetDepthRanges")(&r.cb, 1, 2, dr);
    abi::proc<PFNNVNCOMMANDBUFFERSETDEPTHBOUNDSPROC>("nvnCommandBufferSetDepthBounds")(&r.cb, 1, 0.5f, 0.75f);
    const Ref<CmdList> list = r.end();
    expect_count(list, 7);
    if (!list) return;
    RecordReader rd(*list);
    const float one_vp[4] = {1.f, 2.f, 300.f, 400.f};
    if (expect_op(rd, Op::SetViewports, 8 + 8 + 16)) {
        CHECK_EQ(rd.payload<P_Rects>().first, 0u);
        CHECK_EQ(rd.payload<P_Rects>().count, 1u);
        expect_bytes(rd.blob<P_Rects>(), one_vp, sizeof one_vp);
    }
    if (expect_op(rd, Op::SetViewports, 8 + 8 + 32)) {
        CHECK_EQ(rd.payload<P_Rects>().first, 2u);
        CHECK_EQ(rd.payload<P_Rects>().count, 2u);
        expect_bytes(rd.blob<P_Rects>(), vps, sizeof vps);
    }
    const int one_sc[4] = {-5, 6, 7, 8};
    if (expect_op(rd, Op::SetScissors, 8 + 8 + 16)) {
        CHECK_EQ(rd.payload<P_Rects>().first, 0u);
        CHECK_EQ(rd.payload<P_Rects>().count, 1u);
        expect_bytes(rd.blob<P_Rects>(), one_sc, sizeof one_sc);
    }
    if (expect_op(rd, Op::SetScissors, 8 + 8 + 32)) {
        CHECK_EQ(rd.payload<P_Rects>().first, 1u);
        CHECK_EQ(rd.payload<P_Rects>().count, 2u);
        expect_bytes(rd.blob<P_Rects>(), sc, sizeof sc);
    }
    const float one_dr[2] = {0.25f, 0.75f};
    if (expect_op(rd, Op::SetDepthRanges, 8 + 8 + 8)) {
        CHECK_EQ(rd.payload<P_DepthRanges>().first, 0u);
        CHECK_EQ(rd.payload<P_DepthRanges>().count, 1u);
        expect_bytes(rd.blob<P_DepthRanges>(), one_dr, sizeof one_dr);
    }
    if (expect_op(rd, Op::SetDepthRanges, 8 + 8 + 16)) {
        CHECK_EQ(rd.payload<P_DepthRanges>().first, 1u);
        CHECK_EQ(rd.payload<P_DepthRanges>().count, 2u);
        expect_bytes(rd.blob<P_DepthRanges>(), dr, sizeof dr);
    }
    if (expect_op(rd, Op::SetDepthBounds, 8 + 16)) {
        const P_DepthBounds& p = rd.payload<P_DepthBounds>();
        CHECK_EQ(p.enable, 1u);
        CHECK_EQ(p.near_, 0.5f);
        CHECK_EQ(p.far_, 0.75f);
    }
}

NXTEST(stencil_and_fixed_function_values_recorded) {
    setup();
    Recorder r;
    abi::proc<PFNNVNCOMMANDBUFFERSETSTENCILREFPROC>("nvnCommandBufferSetStencilRef")(&r.cb, NVN_FACE_FRONT, -1);
    abi::proc<PFNNVNCOMMANDBUFFERSETSTENCILVALUEMASKPROC>("nvnCommandBufferSetStencilValueMask")(&r.cb, NVN_FACE_BACK, 0x0F);
    abi::proc<PFNNVNCOMMANDBUFFERSETSTENCILMASKPROC>("nvnCommandBufferSetStencilMask")(&r.cb, NVN_FACE_BACK, 0x80);
    const float rgba[4] = {0.1f, 0.2f, 0.3f, 0.4f};
    abi::proc<PFNNVNCOMMANDBUFFERSETBLENDCOLORPROC>("nvnCommandBufferSetBlendColor")(&r.cb, rgba);
    abi::proc<PFNNVNCOMMANDBUFFERSETPOINTSIZEPROC>("nvnCommandBufferSetPointSize")(&r.cb, 3.5f);
    abi::proc<PFNNVNCOMMANDBUFFERSETLINEWIDTHPROC>("nvnCommandBufferSetLineWidth")(&r.cb, 2.25f);
    abi::proc<PFNNVNCOMMANDBUFFERSETALPHAREFPROC>("nvnCommandBufferSetAlphaRef")(&r.cb, 0.5f);
    abi::proc<PFNNVNCOMMANDBUFFERSETPOLYGONOFFSETCLAMPPROC>("nvnCommandBufferSetPolygonOffsetClamp")(&r.cb, 1.5f, -2.0f, 0.25f);
    abi::proc<PFNNVNCOMMANDBUFFERSETSAMPLEMASKPROC>("nvnCommandBufferSetSampleMask")(&r.cb, 0x0F);
    const Ref<CmdList> list = r.end();
    expect_count(list, 9);
    if (!list) return;
    RecordReader rd(*list);
    if (expect_op(rd, Op::SetStencil, 8 + 16)) {
        const P_Stencil& p = rd.payload<P_Stencil>();
        CHECK_EQ(p.what, 0u);
        CHECK_EQ(p.face, static_cast<uint32_t>(NVN_FACE_FRONT));
        CHECK_EQ(static_cast<int>(p.value), -1);
    }
    if (expect_op(rd, Op::SetStencil, 8 + 16)) {
        const P_Stencil& p = rd.payload<P_Stencil>();
        CHECK_EQ(p.what, 1u);
        CHECK_EQ(p.face, static_cast<uint32_t>(NVN_FACE_BACK));
        CHECK_EQ(p.value, 0x0Fu);
    }
    if (expect_op(rd, Op::SetStencil, 8 + 16)) {
        const P_Stencil& p = rd.payload<P_Stencil>();
        CHECK_EQ(p.what, 2u);
        CHECK_EQ(p.face, static_cast<uint32_t>(NVN_FACE_BACK));
        CHECK_EQ(p.value, 0x80u);
    }
    if (expect_op(rd, Op::SetBlendColor, 8 + 16)) expect_bytes(reinterpret_cast<const uint8_t*>(&rd.payload<P_BlendColor>()), rgba, sizeof rgba);
    expect_f32(rd, Op::SetPointSize, 3.5f);
    expect_f32(rd, Op::SetLineWidth, 2.25f);
    expect_f32(rd, Op::SetAlphaRef, 0.5f);
    if (expect_op(rd, Op::SetPolygonOffsetClamp, 8 + 16)) {
        const P_PolygonOffsetClamp& p = rd.payload<P_PolygonOffsetClamp>();
        CHECK_EQ(p.factor, 1.5f);
        CHECK_EQ(p.units, -2.0f);
        CHECK_EQ(p.clamp, 0.25f);
    }
    expect_u32(rd, Op::SetSampleMask, 0x0F);
}

NXTEST(flags_and_tessellation_recorded) {
    setup();
    Recorder r;
    abi::proc<PFNNVNCOMMANDBUFFERSETRASTERIZERDISCARDPROC>("nvnCommandBufferSetRasterizerDiscard")(&r.cb, 1);
    abi::proc<PFNNVNCOMMANDBUFFERSETDEPTHCLAMPPROC>("nvnCommandBufferSetDepthClamp")(&r.cb, 0);
    abi::proc<PFNNVNCOMMANDBUFFERSETCONSERVATIVERASTERENABLEPROC>("nvnCommandBufferSetConservativeRasterEnable")(&r.cb, 1);
    abi::proc<PFNNVNCOMMANDBUFFERSETPRIMITIVERESTARTPROC>("nvnCommandBufferSetPrimitiveRestart")(&r.cb, 1, 0xFFFF);
    abi::proc<PFNNVNCOMMANDBUFFERSETPATCHSIZEPROC>("nvnCommandBufferSetPatchSize")(&r.cb, 3);
    const float inner[2] = {1.5f, 2.5f};
    abi::proc<PFNNVNCOMMANDBUFFERSETINNERTESSELLATIONLEVELSPROC>("nvnCommandBufferSetInnerTessellationLevels")(&r.cb, inner);
    const float outer[4] = {1.f, 2.f, 3.f, 4.f};
    abi::proc<PFNNVNCOMMANDBUFFERSETOUTERTESSELLATIONLEVELSPROC>("nvnCommandBufferSetOuterTessellationLevels")(&r.cb, outer);
    abi::proc<PFNNVNCOMMANDBUFFERSETRENDERENABLEPROC>("nvnCommandBufferSetRenderEnable")(&r.cb, 0);
    abi::proc<PFNNVNCOMMANDBUFFERSETRENDERENABLECONDITIONALPROC>("nvnCommandBufferSetRenderEnableConditional")(
        &r.cb, static_cast<NVNconditionalRenderMode>(2), 0x7000);
    const Ref<CmdList> list = r.end();
    expect_count(list, 9);
    if (!list) return;
    RecordReader rd(*list);
    expect_u32(rd, Op::SetRasterizerDiscard, 1);
    expect_u32(rd, Op::SetDepthClamp, 0);
    expect_u32(rd, Op::SetConservativeRaster, 1);
    if (expect_op(rd, Op::SetPrimitiveRestart, 8 + 8)) {
        CHECK_EQ(rd.payload<P_PrimitiveRestart>().enable, 1u);
        CHECK_EQ(rd.payload<P_PrimitiveRestart>().index, 0xFFFFu);
    }
    expect_u32(rd, Op::SetPatchSize, 3);
    if (expect_op(rd, Op::SetTessLevels, 8 + 24)) {
        CHECK_EQ(rd.payload<P_TessLevels>().outer, 0u);
        CHECK_EQ(rd.payload<P_TessLevels>().v[0], 1.5f);
        CHECK_EQ(rd.payload<P_TessLevels>().v[1], 2.5f);
    }
    if (expect_op(rd, Op::SetTessLevels, 8 + 24)) {
        CHECK_EQ(rd.payload<P_TessLevels>().outer, 1u);
        expect_bytes(reinterpret_cast<const uint8_t*>(rd.payload<P_TessLevels>().v), outer, sizeof outer);
    }
    expect_u32(rd, Op::SetRenderEnable, 0);
    if (expect_op(rd, Op::SetRenderEnable, 8 + 16)) {   // the conditional form: 16-byte payload
        CHECK_EQ(rd.payload<P_RenderEnableCond>().mode, 2u);
        CHECK_EQ(rd.payload<P_RenderEnableCond>().address, 0x7000ull);
    }
}

NXTEST(render_targets_recorded) {
    setup();
    Recorder r;
    NVNtexture c0, c1, cd;   // guest textures that are not bound to a host object: recorded as null
    std::memset(&c0, 0, sizeof c0);
    std::memset(&c1, 0, sizeof c1);
    std::memset(&cd, 0, sizeof cd);
    NVNtextureView v0, vd;
    abi::proc<PFNNVNTEXTUREVIEWSETDEFAULTSPROC>("nvnTextureViewSetDefaults")(&v0);
    abi::proc<PFNNVNTEXTUREVIEWSETLEVELSPROC>("nvnTextureViewSetLevels")(&v0, 1, 2);
    abi::proc<PFNNVNTEXTUREVIEWSETDEFAULTSPROC>("nvnTextureViewSetDefaults")(&vd);
    abi::proc<PFNNVNTEXTUREVIEWSETLEVELSPROC>("nvnTextureViewSetLevels")(&vd, 0, 3);
    auto set_targets = abi::proc<PFNNVNCOMMANDBUFFERSETRENDERTARGETSPROC>("nvnCommandBufferSetRenderTargets");
    const NVNtexture* colors[2] = {&c0, &c1};
    const NVNtextureView* views[2] = {&v0, nullptr};
    set_targets(&r.cb, 2, colors, views, &cd, &vd);
    const NVNtexture* many[9] = {&c0, &c0, &c0, &c0, &c0, &c0, &c0, &c0, &c0};
    set_targets(&r.cb, 9, many, nullptr, nullptr, nullptr);    // clamped to 8
    set_targets(&r.cb, -3, many, nullptr, nullptr, nullptr);   // clamped to 0
    const Ref<CmdList> list = r.end();
    expect_count(list, 3);
    if (!list) return;
    RecordReader rd(*list);
    if (expect_op(rd, Op::SetRenderTargets, kRenderTargetsSize)) {
        const P_RenderTargets& p = rd.payload<P_RenderTargets>();
        CHECK_EQ(p.count, 2u);
        CHECK(p.color[0].tex == nullptr);
        CHECK_EQ(p.color[0].has_view, 1u);
        CHECK(p.color[0].view.has_levels);
        CHECK_EQ(p.color[0].view.min_level, 1);
        CHECK_EQ(p.color[0].view.num_levels, 2);
        CHECK_EQ(p.color[1].has_view, 0u);
        CHECK(p.depth.tex == nullptr);
        CHECK_EQ(p.depth.has_view, 1u);
        CHECK_EQ(p.depth.view.num_levels, 3);
        CHECK_EQ(p.color[2].has_view, 0u);   // entries past the count stay empty
    }
    if (expect_op(rd, Op::SetRenderTargets, kRenderTargetsSize)) CHECK_EQ(rd.payload<P_RenderTargets>().count, 8u);
    if (expect_op(rd, Op::SetRenderTargets, kRenderTargetsSize)) CHECK_EQ(rd.payload<P_RenderTargets>().count, 0u);
}

NXTEST(coverage_table_recorded) {
    setup();
    Recorder r;
    float table[16];
    for (int i = 0; i < 16; ++i) table[i] = static_cast<float>(i) * 0.5f;
    abi::proc<PFNNVNCOMMANDBUFFERBINDCOVERAGEMODULATIONTABLEPROC>("nvnCommandBufferBindCoverageModulationTable")(&r.cb, table);
    const Ref<CmdList> list = r.end();
    expect_count(list, 1);
    if (!list) return;
    RecordReader rd(*list);
    if (expect_op(rd, Op::SetCoverageModulationTable, 8 + 8 + 64)) expect_bytes(rd.blob<P_U32>(), table, sizeof table);
}

NXTEST(no_op_functions_record_nothing) {
    setup();
    Recorder r;
    const NVNviewportSwizzle swizzles[4] = {};
    abi::proc<PFNNVNCOMMANDBUFFERSETVIEWPORTSWIZZLESPROC>("nvnCommandBufferSetViewportSwizzles")(&r.cb, 0, 1, swizzles);
    NVNmemoryPool pool;
    std::memset(&pool, 0, sizeof pool);
    abi::proc<PFNNVNCOMMANDBUFFERSETSHADERSCRATCHMEMORYPROC>("nvnCommandBufferSetShaderScratchMemory")(&r.cb, &pool, 0, 4096);
    NVNprogram prog;
    std::memset(&prog, 0, sizeof prog);
    const int subroutines[1] = {0};
    abi::proc<PFNNVNCOMMANDBUFFERSETPROGRAMSUBROUTINESPROC>("nvnCommandBufferSetProgramSubroutines")(&r.cb, &prog, NVN_SHADER_STAGE_VERTEX, 0, 1, subroutines);
    abi::proc<PFNNVNCOMMANDBUFFERSETTILEDCACHEACTIONPROC>("nvnCommandBufferSetTiledCacheAction")(&r.cb, static_cast<NVNtiledCacheAction>(0));
    abi::proc<PFNNVNCOMMANDBUFFERSETTILEDCACHETILESIZEPROC>("nvnCommandBufferSetTiledCacheTileSize")(&r.cb, 32, 32);
    abi::proc<PFNNVNCOMMANDBUFFERSETCONSERVATIVERASTERDILATEPROC>("nvnCommandBufferSetConservativeRasterDilate")(&r.cb, 0.5f);
    abi::proc<PFNNVNCOMMANDBUFFERSETSUBPIXELPRECISIONBIASPROC>("nvnCommandBufferSetSubpixelPrecisionBias")(&r.cb, 8, 8);
    const NVNseparateTextureHandle tex{0x12};
    const NVNseparateSamplerHandle smp{0x34};
    abi::proc<PFNNVNCOMMANDBUFFERBINDSEPARATETEXTUREPROC>("nvnCommandBufferBindSeparateTexture")(&r.cb, NVN_SHADER_STAGE_VERTEX, 0, tex);
    abi::proc<PFNNVNCOMMANDBUFFERBINDSEPARATESAMPLERPROC>("nvnCommandBufferBindSeparateSampler")(&r.cb, NVN_SHADER_STAGE_VERTEX, 0, smp);
    abi::proc<PFNNVNCOMMANDBUFFERBINDSEPARATETEXTURESPROC>("nvnCommandBufferBindSeparateTextures")(&r.cb, NVN_SHADER_STAGE_VERTEX, 0, 1, &tex);
    abi::proc<PFNNVNCOMMANDBUFFERBINDSEPARATESAMPLERSPROC>("nvnCommandBufferBindSeparateSamplers")(&r.cb, NVN_SHADER_STAGE_VERTEX, 0, 1, &smp);
    const Ref<CmdList> list = r.end();
    expect_count(list, 0);
}

NXTEST(nothing_recorded_outside_recording_or_for_null_arguments) {
    setup();
    Recorder r(false);   // not recording yet
    NVNblendState blend;
    std::memset(&blend, 1, sizeof blend);
    abi::proc<PFNNVNCOMMANDBUFFERBINDBLENDSTATEPROC>("nvnCommandBufferBindBlendState")(&r.cb, &blend);
    abi::proc<PFNNVNCOMMANDBUFFERSETPOINTSIZEPROC>("nvnCommandBufferSetPointSize")(&r.cb, 2.0f);
    abi::proc<PFNNVNCOMMANDBUFFERSETRENDERTARGETSPROC>("nvnCommandBufferSetRenderTargets")(&r.cb, 0, nullptr, nullptr, nullptr, nullptr);
    abi::proc<PFNNVNCOMMANDBUFFERBEGINRECORDINGPROC>("nvnCommandBufferBeginRecording")(&r.cb);
    // recording now, but with null or empty arguments
    abi::proc<PFNNVNCOMMANDBUFFERBINDBLENDSTATEPROC>("nvnCommandBufferBindBlendState")(&r.cb, nullptr);
    NVNvertexAttribState attr[1];
    std::memset(attr, 0, sizeof attr);
    abi::proc<PFNNVNCOMMANDBUFFERBINDVERTEXATTRIBSTATEPROC>("nvnCommandBufferBindVertexAttribState")(&r.cb, 0, attr);
    abi::proc<PFNNVNCOMMANDBUFFERBINDVERTEXBUFFERSPROC>("nvnCommandBufferBindVertexBuffers")(&r.cb, 0, 0, nullptr);
    abi::proc<PFNNVNCOMMANDBUFFERBINDTEXTURESPROC>("nvnCommandBufferBindTextures")(&r.cb, NVN_SHADER_STAGE_VERTEX, 0, 2, nullptr);
    abi::proc<PFNNVNCOMMANDBUFFERBINDCOVERAGEMODULATIONTABLEPROC>("nvnCommandBufferBindCoverageModulationTable")(&r.cb, nullptr);
    const Ref<CmdList> list = r.end();
    expect_count(list, 0);
    // The list is handed over at EndRecording: a later call does not record into it.
    abi::proc<PFNNVNCOMMANDBUFFERSETPOINTSIZEPROC>("nvnCommandBufferSetPointSize")(&r.cb, 2.0f);
    abi::proc<PFNNVNCOMMANDBUFFERBEGINRECORDINGPROC>("nvnCommandBufferBeginRecording")(&r.cb);
    const Ref<CmdList> second = r.end();
    expect_count(list, 0);
    expect_count(second, 0);
}

// A list recorded into command memory outlives its command buffer: the game may finalize the buffer and call the handle later
// (Animal Crossing's worker jobs do it every frame). A newer list recorded over the same memory still drops it, and a list without
// command memory goes with its buffer.
NXTEST(handles_outlive_their_finalized_command_buffer) {
    setup();
    auto add_mem = abi::proc<PFNNVNCOMMANDBUFFERADDCOMMANDMEMORYPROC>("nvnCommandBufferAddCommandMemory");
    NVNcommandHandle backed = 0, unbacked = 0;
    {
        Recorder r(false);
        add_mem(&r.cb, nullptr, 0x10000, 0x1000);
        abi::proc<PFNNVNCOMMANDBUFFERBEGINRECORDINGPROC>("nvnCommandBufferBeginRecording")(&r.cb);
        abi::proc<PFNNVNCOMMANDBUFFERSETLINEWIDTHPROC>("nvnCommandBufferSetLineWidth")(&r.cb, 2.0f);
        backed = abi::proc<PFNNVNCOMMANDBUFFERENDRECORDINGPROC>("nvnCommandBufferEndRecording")(&r.cb);
    }
    {
        Recorder r;
        abi::proc<PFNNVNCOMMANDBUFFERSETLINEWIDTHPROC>("nvnCommandBufferSetLineWidth")(&r.cb, 2.0f);
        unbacked = abi::proc<PFNNVNCOMMANDBUFFERENDRECORDINGPROC>("nvnCommandBufferEndRecording")(&r.cb);
    }
    CHECK(backed != 0);
    CHECK(unbacked != 0);
    const Ref<CmdList> kept = handle_lookup(backed);
    CHECK(kept);
    if (kept) expect_count(kept, 1);
    CHECK(!handle_lookup(unbacked));
    {
        Recorder r(false);
        add_mem(&r.cb, nullptr, 0x10000, 0x1000);   // the same memory again
        abi::proc<PFNNVNCOMMANDBUFFERBEGINRECORDINGPROC>("nvnCommandBufferBeginRecording")(&r.cb);
        abi::proc<PFNNVNCOMMANDBUFFERSETLINEWIDTHPROC>("nvnCommandBufferSetLineWidth")(&r.cb, 3.0f);
        const NVNcommandHandle over = abi::proc<PFNNVNCOMMANDBUFFERENDRECORDINGPROC>("nvnCommandBufferEndRecording")(&r.cb);
        CHECK(handle_lookup(over));
        CHECK(!handle_lookup(backed));
    }
}
