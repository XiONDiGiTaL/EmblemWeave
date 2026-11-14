// SPDX-License-Identifier: GPL-3.0-or-later
// Recorded work commands (api_cmd_work.cpp) through the typed thunks. Every entry point is called through abi::proc, the
// command buffer is initialised through nvnCommandBufferInitialize, and the list it records is walked back with
// RecordReader: op, payload fields, blob and held objects are checked for each record.
#include <cstring>
#include <vector>

#include "nvn_cmd.h"
#include "nvn_impl.h"
#include "test_framework.h"
NXTEST_GROUP(gfx_nvn_cmd_work)

using namespace nxrt;
using namespace nxrt::nvn;
using namespace nxrt::nvn::be;


#define CB_PROC(UP, name) abi::proc<PFNNVNCOMMANDBUFFER##UP##PROC>(#name)
#define NEXT_OR_RETURN(w, OP) \
    if (!(w).next(OP)) return

namespace {

// Every name registered by this family (plus the cmdbuf family that the tests need). 55 entries.
const char* const kNames[] = {
    "nvnCommandBufferDrawArrays", "nvnCommandBufferDrawArraysInstanced", "nvnCommandBufferDrawElements",
    "nvnCommandBufferDrawElementsBaseVertex", "nvnCommandBufferDrawElementsInstanced", "nvnCommandBufferDrawArraysIndirect",
    "nvnCommandBufferDrawElementsIndirect", "nvnCommandBufferMultiDrawArraysIndirectCount",
    "nvnCommandBufferMultiDrawElementsIndirectCount", "nvnCommandBufferDrawTransformFeedback",
    "nvnCommandBufferBeginTransformFeedback", "nvnCommandBufferEndTransformFeedback", "nvnCommandBufferPauseTransformFeedback",
    "nvnCommandBufferResumeTransformFeedback", "nvnCommandBufferClearColor", "nvnCommandBufferClearColori",
    "nvnCommandBufferClearColorui", "nvnCommandBufferClearDepthStencil", "nvnCommandBufferClearTexture",
    "nvnCommandBufferClearTexturei", "nvnCommandBufferClearTextureui", "nvnCommandBufferClearBuffer",
    "nvnCommandBufferCopyBufferToBuffer", "nvnCommandBufferCopyBufferToTexture", "nvnCommandBufferCopyTextureToBuffer",
    "nvnCommandBufferCopyTextureToTexture", "nvnCommandBufferDownsample", "nvnCommandBufferTiledDownsample",
    "nvnCommandBufferDownsampleTextureView", "nvnCommandBufferTiledDownsampleTextureView", "nvnCommandBufferUpdateUniformBuffer",
    "nvnCommandBufferReportCounter", "nvnCommandBufferResetCounter", "nvnCommandBufferReportValue", "nvnCommandBufferBarrier",
    "nvnCommandBufferFenceSync", "nvnCommandBufferWaitSync", "nvnCommandBufferSignalEvent", "nvnCommandBufferWaitEvent",
    "nvnCommandBufferDispatchCompute", "nvnCommandBufferDispatchComputeIndirect", "nvnCommandBufferDrawTexture",
    "nvnCommandBufferDiscardColor", "nvnCommandBufferDiscardDepthStencil", "nvnCommandBufferResolveDepthBuffer",
    "nvnCommandBufferSaveZCullData", "nvnCommandBufferRestoreZCullData", "nvnCommandBufferPushDebugGroupStatic",
    "nvnCommandBufferPushDebugGroupDynamic", "nvnCommandBufferPushDebugGroup", "nvnCommandBufferPopDebugGroup",
    "nvnCommandBufferPopDebugGroupId", "nvnCommandBufferInsertDebugMarkerStatic", "nvnCommandBufferInsertDebugMarkerDynamic",
    "nvnCommandBufferInsertDebugMarker",
};

void setup() {
    install();
    abi::reset_mismatches();
    register_api_cmdbuf();
    // register_api_cmdbuf reports a mismatch of its own (nvnCommandBufferGetMemoryCallbackData: table says void, impl returns
    // void*). It is not this family's, so the count is reset and only this family's registrations are checked.
    abi::reset_mismatches();
    register_api_cmd_work();
    CHECK_EQ(abi::mismatches(), 0u);
}

// Initialises a zeroed command buffer through the proc table (null device).
void init_cb(NVNcommandBuffer* c) {
    std::memset(c, 0, sizeof *c);
    auto init = abi::proc<PFNNVNCOMMANDBUFFERINITIALIZEPROC>("nvnCommandBufferInitialize");
    CHECK(init(c, nullptr));
}

// Guest objects with host objects behind them, plus the host pointers for comparisons.
struct Guests {
    NVNtexture tex{}, tex2{};
    NVNtextureView view{};
    NVNcopyRegion region{};
    NVNsync sync{};
    NVNevent event{};
    Texture* th;
    Texture* th2;
    Sync* sh;
    Event* eh;
    Guests() {
        th = new Texture();
        bind_guest(&tex, th);
        th2 = new Texture();
        bind_guest(&tex2, th2);
        sh = new Sync();
        bind_guest(&sync, sh);
        eh = new Event();
        bind_guest(&event, eh);
        region = NVNcopyRegion{1, 2, 3, 4, 5, 6};
    }
};

CmdList& nonnull(const Ref<CmdList>& l) {
    static CmdList empty;
    return l ? *l : empty;
}

// Walks the records of a finished list. next(op) reads the next record and checks its op.
struct Walk {
    Ref<CmdList> list;
    RecordReader r;
    explicit Walk(uint64_t handle) : list(handle_lookup(handle)), r(nonnull(list)) {}
    bool next(Op op) {
        const bool ok = r.next() && r.op() == op;
        CHECK(ok);
        return ok;
    }
    bool at_end() { return !r.next(); }
};

uint32_t bits_of(float f) {
    uint32_t u;
    std::memcpy(&u, &f, 4);
    return u;
}

// Calls every entry point that records a command, with valid arguments. Used where nothing may be recorded.
void fire_all(NVNcommandBuffer* c, Guests& g) {
    static const float fcol[4] = {1, 2, 3, 4};
    static const int icol[4] = {1, 2, 3, 4};
    static const uint32_t ucol[4] = {1, 2, 3, 4};
    static const uint8_t data[16] = {};
    CB_PROC(DRAWARRAYS, nvnCommandBufferDrawArrays)(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
    CB_PROC(DRAWARRAYSINSTANCED, nvnCommandBufferDrawArraysInstanced)(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3, 0, 1);
    CB_PROC(DRAWELEMENTS, nvnCommandBufferDrawElements)(c, NVN_DRAW_PRIMITIVE_TRIANGLES, NVN_INDEX_TYPE_UNSIGNED_INT, 3, 0x1000);
    CB_PROC(DRAWELEMENTSBASEVERTEX, nvnCommandBufferDrawElementsBaseVertex)(c, NVN_DRAW_PRIMITIVE_TRIANGLES, NVN_INDEX_TYPE_UNSIGNED_INT, 3, 0x1000, 0);
    CB_PROC(DRAWELEMENTSINSTANCED, nvnCommandBufferDrawElementsInstanced)(c, NVN_DRAW_PRIMITIVE_TRIANGLES, NVN_INDEX_TYPE_UNSIGNED_INT, 3, 0x1000, 0, 0, 1);
    CB_PROC(DRAWARRAYSINDIRECT, nvnCommandBufferDrawArraysIndirect)(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0x4000);
    CB_PROC(DRAWELEMENTSINDIRECT, nvnCommandBufferDrawElementsIndirect)(c, NVN_DRAW_PRIMITIVE_TRIANGLES, NVN_INDEX_TYPE_UNSIGNED_INT, 0x5000, 0x6000);
    CB_PROC(MULTIDRAWARRAYSINDIRECTCOUNT, nvnCommandBufferMultiDrawArraysIndirectCount)(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0x7000, 0x8000, 5, 32);
    CB_PROC(MULTIDRAWELEMENTSINDIRECTCOUNT, nvnCommandBufferMultiDrawElementsIndirectCount)(c, NVN_DRAW_PRIMITIVE_TRIANGLES, NVN_INDEX_TYPE_UNSIGNED_INT, 0x9000, 0xA000, 0xB000, 7, 16);
    CB_PROC(CLEARCOLOR, nvnCommandBufferClearColor)(c, 0, fcol, 15);
    CB_PROC(CLEARCOLORI, nvnCommandBufferClearColori)(c, 0, icol, 15);
    CB_PROC(CLEARCOLORUI, nvnCommandBufferClearColorui)(c, 0, ucol, 15);
    CB_PROC(CLEARDEPTHSTENCIL, nvnCommandBufferClearDepthStencil)(c, 0.5f, 1, 0, 0xFF);
    CB_PROC(CLEARTEXTURE, nvnCommandBufferClearTexture)(c, &g.tex, &g.view, &g.region, fcol, 15);
    CB_PROC(CLEARTEXTUREI, nvnCommandBufferClearTexturei)(c, &g.tex, nullptr, nullptr, icol, 15);
    CB_PROC(CLEARTEXTUREUI, nvnCommandBufferClearTextureui)(c, &g.tex, nullptr, nullptr, ucol, 15);
    CB_PROC(CLEARBUFFER, nvnCommandBufferClearBuffer)(c, 0x9000, 256, 0xDEADBEEFu);
    CB_PROC(COPYBUFFERTOBUFFER, nvnCommandBufferCopyBufferToBuffer)(c, 0x1000, 0x2000, 512, 0);
    CB_PROC(COPYBUFFERTOTEXTURE, nvnCommandBufferCopyBufferToTexture)(c, 0x3000, &g.tex, nullptr, nullptr, 0);
    CB_PROC(COPYTEXTURETOBUFFER, nvnCommandBufferCopyTextureToBuffer)(c, &g.tex, nullptr, nullptr, 0x4000, 0);
    CB_PROC(COPYTEXTURETOTEXTURE, nvnCommandBufferCopyTextureToTexture)(c, &g.tex, nullptr, nullptr, &g.tex2, nullptr, nullptr, 0);
    CB_PROC(DOWNSAMPLE, nvnCommandBufferDownsample)(c, &g.tex, &g.tex2);
    CB_PROC(TILEDDOWNSAMPLE, nvnCommandBufferTiledDownsample)(c, &g.tex, &g.tex2);
    CB_PROC(DOWNSAMPLETEXTUREVIEW, nvnCommandBufferDownsampleTextureView)(c, &g.tex, &g.view, &g.tex2, &g.view);
    CB_PROC(TILEDDOWNSAMPLETEXTUREVIEW, nvnCommandBufferTiledDownsampleTextureView)(c, &g.tex, &g.view, &g.tex2, &g.view);
    CB_PROC(UPDATEUNIFORMBUFFER, nvnCommandBufferUpdateUniformBuffer)(c, 0x5000, 4096, 0, 16, data);
    CB_PROC(REPORTCOUNTER, nvnCommandBufferReportCounter)(c, NVN_COUNTER_TYPE_SAMPLES_PASSED, 0xA000);
    CB_PROC(RESETCOUNTER, nvnCommandBufferResetCounter)(c, NVN_COUNTER_TYPE_SAMPLES_PASSED);
    CB_PROC(REPORTVALUE, nvnCommandBufferReportValue)(c, 7u, 0xB000);
    CB_PROC(BARRIER, nvnCommandBufferBarrier)(c, 0x11);
    CB_PROC(FENCESYNC, nvnCommandBufferFenceSync)(c, &g.sync, NVN_SYNC_CONDITION_ALL_GPU_COMMANDS_COMPLETE, 0);
    CB_PROC(WAITSYNC, nvnCommandBufferWaitSync)(c, &g.sync);
    CB_PROC(SIGNALEVENT, nvnCommandBufferSignalEvent)(c, &g.event, NVN_EVENT_SIGNAL_MODE_ADD, NVN_EVENT_SIGNAL_LOCATION_BOTTOM, 3, 0x55);
    CB_PROC(WAITEVENT, nvnCommandBufferWaitEvent)(c, &g.event, NVN_EVENT_WAIT_MODE_GEQUAL_WRAP, 0x77);
    CB_PROC(DISPATCHCOMPUTE, nvnCommandBufferDispatchCompute)(c, 4, 5, 6);
    CB_PROC(DISPATCHCOMPUTEINDIRECT, nvnCommandBufferDispatchComputeIndirect)(c, 0xC000);
    CB_PROC(DRAWTEXTURE, nvnCommandBufferDrawTexture)(c, 0x123, nullptr, nullptr);
    CB_PROC(DISCARDCOLOR, nvnCommandBufferDiscardColor)(c, 1);
    CB_PROC(DISCARDDEPTHSTENCIL, nvnCommandBufferDiscardDepthStencil)(c);
    CB_PROC(RESOLVEDEPTHBUFFER, nvnCommandBufferResolveDepthBuffer)(c);
    CB_PROC(DRAWTRANSFORMFEEDBACK, nvnCommandBufferDrawTransformFeedback)(c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0x100);
    CB_PROC(BEGINTRANSFORMFEEDBACK, nvnCommandBufferBeginTransformFeedback)(c, 0x100);
    CB_PROC(ENDTRANSFORMFEEDBACK, nvnCommandBufferEndTransformFeedback)(c, 0x100);
    CB_PROC(PAUSETRANSFORMFEEDBACK, nvnCommandBufferPauseTransformFeedback)(c, 0x100);
    CB_PROC(RESUMETRANSFORMFEEDBACK, nvnCommandBufferResumeTransformFeedback)(c, 0x100);
}

}  // namespace

NXTEST(all_names_registered_and_implemented) {
    setup();
    for (const char* n : kNames) CHECK(is_implemented(n));
    CHECK_EQ(sizeof kNames / sizeof kNames[0], 55u);
}

NXTEST(nothing_recorded_outside_recording_or_without_object) {
    setup();
    Guests g;
    NVNcommandBuffer c;
    init_cb(&c);
    // before BeginRecording
    fire_all(&c, g);
    CB_PROC(BEGINRECORDING, nvnCommandBufferBeginRecording)(&c);
    NVNcommandHandle h0 = CB_PROC(ENDRECORDING, nvnCommandBufferEndRecording)(&c);
    {
        Walk w(h0);
        CHECK(w.list);
        CHECK_EQ(w.list->record_count, 0u);
        CHECK(w.list->bytes.empty());
        CHECK_EQ(w.list->draw_count, 0u);
    }
    // one real record, then calls after EndRecording must not touch that list
    CB_PROC(BEGINRECORDING, nvnCommandBufferBeginRecording)(&c);
    CB_PROC(DRAWARRAYS, nvnCommandBufferDrawArrays)(&c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0, 3);
    NVNcommandHandle h1 = CB_PROC(ENDRECORDING, nvnCommandBufferEndRecording)(&c);
    Walk w1(h1);
    CHECK(w1.list);
    const uint32_t before_count = w1.list->record_count;
    const size_t before_bytes = w1.list->bytes.size();
    CHECK_EQ(before_count, 1u);
    fire_all(&c, g);
    CHECK_EQ(w1.list->record_count, before_count);
    CHECK_EQ(w1.list->bytes.size(), before_bytes);
    // null command buffer and null host objects: no crash, nothing recorded
    fire_all(nullptr, g);
    NVNtexture unbound;
    std::memset(&unbound, 0, sizeof unbound);
    NVNsync unbound_sync;
    std::memset(&unbound_sync, 0, sizeof unbound_sync);
    CB_PROC(BEGINRECORDING, nvnCommandBufferBeginRecording)(&c);
    CB_PROC(CLEARTEXTURE, nvnCommandBufferClearTexture)(&c, &unbound, nullptr, nullptr, nullptr, 15);
    CB_PROC(CLEARTEXTURE, nvnCommandBufferClearTexture)(&c, &g.tex, nullptr, nullptr, nullptr, 15);   // null colour
    CB_PROC(FENCESYNC, nvnCommandBufferFenceSync)(&c, &unbound_sync, NVN_SYNC_CONDITION_ALL_GPU_COMMANDS_COMPLETE, 0);
    CB_PROC(WAITSYNC, nvnCommandBufferWaitSync)(&c, &unbound_sync);
    CB_PROC(COPYBUFFERTOTEXTURE, nvnCommandBufferCopyBufferToTexture)(&c, 0x3000, &unbound, nullptr, nullptr, 0);
    CB_PROC(DOWNSAMPLE, nvnCommandBufferDownsample)(&c, &unbound, &g.tex2);
    CB_PROC(UPDATEUNIFORMBUFFER, nvnCommandBufferUpdateUniformBuffer)(&c, 0x5000, 16, 0, 16, nullptr);   // null data
    NVNcommandHandle h2 = CB_PROC(ENDRECORDING, nvnCommandBufferEndRecording)(&c);
    Walk w2(h2);
    CHECK(w2.list);
    CHECK_EQ(w2.list->record_count, 0u);
    CHECK_EQ(w2.list->held.size(), 0u);
}

NXTEST(draws_record_their_payloads) {
    setup();
    NVNcommandBuffer c;
    init_cb(&c);
    CB_PROC(BEGINRECORDING, nvnCommandBufferBeginRecording)(&c);
    CB_PROC(DRAWARRAYS, nvnCommandBufferDrawArrays)(&c, NVN_DRAW_PRIMITIVE_TRIANGLES, 7, 9);
    CB_PROC(DRAWARRAYSINSTANCED, nvnCommandBufferDrawArraysInstanced)(&c, NVN_DRAW_PRIMITIVE_TRIANGLES, 1, 2, 3, 4);
    CB_PROC(DRAWELEMENTS, nvnCommandBufferDrawElements)(&c, NVN_DRAW_PRIMITIVE_TRIANGLES, NVN_INDEX_TYPE_UNSIGNED_INT, 36, 0x1000);
    CB_PROC(DRAWELEMENTSBASEVERTEX, nvnCommandBufferDrawElementsBaseVertex)(&c, NVN_DRAW_PRIMITIVE_TRIANGLES, NVN_INDEX_TYPE_UNSIGNED_INT, 12, 0x2000, -5);
    CB_PROC(DRAWELEMENTSINSTANCED, nvnCommandBufferDrawElementsInstanced)(&c, NVN_DRAW_PRIMITIVE_TRIANGLES, NVN_INDEX_TYPE_UNSIGNED_INT, 6, 0x3000, -2, 8, 3);
    CB_PROC(DRAWARRAYSINDIRECT, nvnCommandBufferDrawArraysIndirect)(&c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0x4000);
    CB_PROC(DRAWELEMENTSINDIRECT, nvnCommandBufferDrawElementsIndirect)(&c, NVN_DRAW_PRIMITIVE_TRIANGLES, NVN_INDEX_TYPE_UNSIGNED_INT, 0x5000, 0x6000);
    CB_PROC(MULTIDRAWARRAYSINDIRECTCOUNT, nvnCommandBufferMultiDrawArraysIndirectCount)(&c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0x7000, 0x8000, 5, 32);
    CB_PROC(MULTIDRAWELEMENTSINDIRECTCOUNT, nvnCommandBufferMultiDrawElementsIndirectCount)(&c, NVN_DRAW_PRIMITIVE_TRIANGLES, NVN_INDEX_TYPE_UNSIGNED_INT, 0x9000, 0xA000, 0xB000, 7, 16);
    NVNcommandHandle h = CB_PROC(ENDRECORDING, nvnCommandBufferEndRecording)(&c);
    Walk w(h);
    CHECK(w.list);
    CHECK_EQ(w.list->draw_count, 9u);
    CHECK_EQ(w.list->record_count, 9u);

    {
        NEXT_OR_RETURN(w, Op::DrawArrays);
        const auto& p = w.r.payload<P_DrawArrays>();
        CHECK_EQ(p.mode, static_cast<uint32_t>(NVN_DRAW_PRIMITIVE_TRIANGLES));
        CHECK_EQ(p.first, 7u);
        CHECK_EQ(p.count, 9u);
        CHECK_EQ(p.base_instance, 0u);
        CHECK_EQ(p.instance_count, 1u);
    }
    {
        NEXT_OR_RETURN(w, Op::DrawArrays);
        const auto& p = w.r.payload<P_DrawArrays>();
        CHECK_EQ(p.first, 1u);
        CHECK_EQ(p.count, 2u);
        CHECK_EQ(p.base_instance, 3u);
        CHECK_EQ(p.instance_count, 4u);
    }
    {
        NEXT_OR_RETURN(w, Op::DrawElements);
        const auto& p = w.r.payload<P_DrawElements>();
        CHECK_EQ(p.type, static_cast<uint32_t>(NVN_INDEX_TYPE_UNSIGNED_INT));
        CHECK_EQ(p.count, 36u);
        CHECK_EQ(p.instance_count, 1u);
        CHECK_EQ(p.base_vertex, 0);
        CHECK_EQ(p.base_instance, 0u);
        CHECK_EQ(static_cast<unsigned long long>(p.index_address), 0x1000ull);
    }
    {
        NEXT_OR_RETURN(w, Op::DrawElements);
        const auto& p = w.r.payload<P_DrawElements>();
        CHECK_EQ(p.count, 12u);
        CHECK_EQ(p.base_vertex, -5);
        CHECK_EQ(p.instance_count, 1u);
        CHECK_EQ(static_cast<unsigned long long>(p.index_address), 0x2000ull);
    }
    {
        NEXT_OR_RETURN(w, Op::DrawElements);
        const auto& p = w.r.payload<P_DrawElements>();
        CHECK_EQ(p.count, 6u);
        CHECK_EQ(p.base_vertex, -2);
        CHECK_EQ(p.base_instance, 8u);
        CHECK_EQ(p.instance_count, 3u);
        CHECK_EQ(static_cast<unsigned long long>(p.index_address), 0x3000ull);
    }
    {
        NEXT_OR_RETURN(w, Op::DrawIndirect);
        const auto& p = w.r.payload<P_DrawIndirect>();
        CHECK_EQ(p.type, 0xFFFFFFFFu);
        CHECK_EQ(p.multi, 0u);
        CHECK_EQ(p.max_count, 1u);
        CHECK_EQ(static_cast<long long>(p.stride), 0ll);
        CHECK_EQ(static_cast<unsigned long long>(p.index_address), 0ull);
        CHECK_EQ(static_cast<unsigned long long>(p.indirect_address), 0x4000ull);
        CHECK_EQ(static_cast<unsigned long long>(p.count_address), 0ull);
    }
    {
        NEXT_OR_RETURN(w, Op::DrawIndirect);
        const auto& p = w.r.payload<P_DrawIndirect>();
        CHECK_EQ(p.type, static_cast<uint32_t>(NVN_INDEX_TYPE_UNSIGNED_INT));
        CHECK_EQ(p.multi, 0u);
        CHECK_EQ(p.max_count, 1u);
        CHECK_EQ(static_cast<unsigned long long>(p.index_address), 0x5000ull);
        CHECK_EQ(static_cast<unsigned long long>(p.indirect_address), 0x6000ull);
        CHECK_EQ(static_cast<unsigned long long>(p.count_address), 0ull);
    }
    {
        NEXT_OR_RETURN(w, Op::DrawIndirect);
        const auto& p = w.r.payload<P_DrawIndirect>();
        CHECK_EQ(p.type, 0xFFFFFFFFu);
        CHECK_EQ(p.multi, 1u);
        CHECK_EQ(p.max_count, 5u);
        CHECK_EQ(static_cast<long long>(p.stride), 32ll);
        CHECK_EQ(static_cast<unsigned long long>(p.index_address), 0ull);
        CHECK_EQ(static_cast<unsigned long long>(p.indirect_address), 0x7000ull);
        CHECK_EQ(static_cast<unsigned long long>(p.count_address), 0x8000ull);
    }
    {
        NEXT_OR_RETURN(w, Op::DrawIndirect);
        const auto& p = w.r.payload<P_DrawIndirect>();
        CHECK_EQ(p.type, static_cast<uint32_t>(NVN_INDEX_TYPE_UNSIGNED_INT));
        CHECK_EQ(p.multi, 1u);
        CHECK_EQ(p.max_count, 7u);
        CHECK_EQ(static_cast<long long>(p.stride), 16ll);
        CHECK_EQ(static_cast<unsigned long long>(p.index_address), 0x9000ull);
        CHECK_EQ(static_cast<unsigned long long>(p.indirect_address), 0xA000ull);
        CHECK_EQ(static_cast<unsigned long long>(p.count_address), 0xB000ull);
    }
    CHECK(w.at_end());
}

NXTEST(draw_texture_regions_and_transform_feedback_noops) {
    setup();
    NVNcommandBuffer c;
    init_cb(&c);
    const float dst[4] = {1.0f, 2.0f, 3.0f, 4.0f};
    const float src[4] = {5.0f, 6.0f, 7.0f, 8.0f};
    CB_PROC(BEGINRECORDING, nvnCommandBufferBeginRecording)(&c);
    CB_PROC(DRAWTEXTURE, nvnCommandBufferDrawTexture)(&c, 0x123456789ull, reinterpret_cast<const NVNdrawTextureRegion*>(dst),
                                                      reinterpret_cast<const NVNdrawTextureRegion*>(src));
    CB_PROC(DRAWTEXTURE, nvnCommandBufferDrawTexture)(&c, 0x42ull, nullptr, nullptr);
    // transform feedback: DrawTransformFeedback (a draw) and Begin / Pause / Resume / End are recorded with the control buffer address
    CB_PROC(DRAWTRANSFORMFEEDBACK, nvnCommandBufferDrawTransformFeedback)(&c, NVN_DRAW_PRIMITIVE_TRIANGLES, 0x100);
    CB_PROC(BEGINTRANSFORMFEEDBACK, nvnCommandBufferBeginTransformFeedback)(&c, 0x100);
    CB_PROC(PAUSETRANSFORMFEEDBACK, nvnCommandBufferPauseTransformFeedback)(&c, 0x100);
    CB_PROC(RESUMETRANSFORMFEEDBACK, nvnCommandBufferResumeTransformFeedback)(&c, 0x100);
    CB_PROC(ENDTRANSFORMFEEDBACK, nvnCommandBufferEndTransformFeedback)(&c, 0x100);
    NVNcommandHandle h = CB_PROC(ENDRECORDING, nvnCommandBufferEndRecording)(&c);
    Walk w(h);
    CHECK(w.list);
    CHECK_EQ(w.list->draw_count, 3u);
    {
        NEXT_OR_RETURN(w, Op::DrawTexture);
        const auto& p = w.r.payload<P_DrawTexture>();
        CHECK_EQ(static_cast<unsigned long long>(p.handle), 0x123456789ull);
        CHECK_EQ(p.dst[0], 1.0f);
        CHECK_EQ(p.dst[3], 4.0f);
        CHECK_EQ(p.src[0], 5.0f);
        CHECK_EQ(p.src[3], 8.0f);
    }
    {
        NEXT_OR_RETURN(w, Op::DrawTexture);
        const auto& p = w.r.payload<P_DrawTexture>();
        CHECK_EQ(static_cast<unsigned long long>(p.handle), 0x42ull);
        for (int i = 0; i < 4; ++i) {
            CHECK_EQ(p.dst[i], 0.0f);
            CHECK_EQ(p.src[i], 0.0f);
        }
    }
    {
        NEXT_OR_RETURN(w, Op::DrawXfb);
        const auto& p = w.r.payload<P_DrawXfb>();
        CHECK_EQ(p.mode, static_cast<uint32_t>(NVN_DRAW_PRIMITIVE_TRIANGLES));
        CHECK_EQ(static_cast<unsigned long long>(p.control), 0x100ull);
    }
    for (Op op : {Op::BeginXfb, Op::PauseXfb, Op::ResumeXfb, Op::EndXfb}) {
        NEXT_OR_RETURN(w, op);
        CHECK_EQ(static_cast<unsigned long long>(w.r.payload<P_Counter>().address), 0x100ull);
    }
    CHECK(w.at_end());
}

NXTEST(clears_record_colour_bits_and_targets) {
    setup();
    Guests g;
    NVNcommandBuffer c;
    init_cb(&c);
    const float fcol[4] = {1.5f, -2.0f, 0.25f, 3.0f};
    const int icol[4] = {-1, 2, -3, 4};
    const uint32_t ucol[4] = {0xFFFFFFFFu, 1, 2, 3};
    CB_PROC(BEGINRECORDING, nvnCommandBufferBeginRecording)(&c);
    CB_PROC(CLEARCOLOR, nvnCommandBufferClearColor)(&c, 2, fcol, 0xF);
    CB_PROC(CLEARCOLORI, nvnCommandBufferClearColori)(&c, 1, icol, 0x3);
    CB_PROC(CLEARCOLORUI, nvnCommandBufferClearColorui)(&c, 0, ucol, 0x7);
    CB_PROC(CLEARDEPTHSTENCIL, nvnCommandBufferClearDepthStencil)(&c, 0.5f, 1, -7, 0xFF);
    CB_PROC(CLEARTEXTURE, nvnCommandBufferClearTexture)(&c, &g.tex, &g.view, &g.region, fcol, 0xF);
    CB_PROC(CLEARTEXTUREI, nvnCommandBufferClearTexturei)(&c, &g.tex2, nullptr, nullptr, icol, 0x2);
    CB_PROC(CLEARTEXTUREUI, nvnCommandBufferClearTextureui)(&c, &g.tex, nullptr, &g.region, ucol, 0x9);
    CB_PROC(CLEARBUFFER, nvnCommandBufferClearBuffer)(&c, 0x9000, 256, 0xDEADBEEFu);
    NVNcommandHandle h = CB_PROC(ENDRECORDING, nvnCommandBufferEndRecording)(&c);
    Walk w(h);
    CHECK(w.list);
    CHECK_EQ(w.list->draw_count, 0u);
    {
        NEXT_OR_RETURN(w, Op::ClearColor);
        const auto& p = w.r.payload<P_ClearColor>();
        CHECK_EQ(p.target, 2u);
        CHECK_EQ(p.kind, 0u);
        CHECK_EQ(p.mask, 0xFu);
        for (int i = 0; i < 4; ++i) CHECK_EQ(p.value[i], bits_of(fcol[i]));
    }
    {
        NEXT_OR_RETURN(w, Op::ClearColor);
        const auto& p = w.r.payload<P_ClearColor>();
        CHECK_EQ(p.target, 1u);
        CHECK_EQ(p.kind, 1u);
        CHECK_EQ(p.mask, 0x3u);
        for (int i = 0; i < 4; ++i) CHECK_EQ(p.value[i], static_cast<uint32_t>(icol[i]));
    }
    {
        NEXT_OR_RETURN(w, Op::ClearColor);
        const auto& p = w.r.payload<P_ClearColor>();
        CHECK_EQ(p.target, 0u);
        CHECK_EQ(p.kind, 2u);
        CHECK_EQ(p.mask, 0x7u);
        CHECK_EQ(p.value[0], 0xFFFFFFFFu);
        CHECK_EQ(p.value[3], 3u);
    }
    {
        NEXT_OR_RETURN(w, Op::ClearDepthStencil);
        const auto& p = w.r.payload<P_ClearDepthStencil>();
        CHECK_EQ(p.depth, 0.5f);
        CHECK_EQ(p.depth_mask, 1u);
        CHECK_EQ(p.stencil, -7);
        CHECK_EQ(p.stencil_mask, 0xFFu);
    }
    {
        NEXT_OR_RETURN(w, Op::ClearTexture);
        const auto& p = w.r.payload<P_ClearTexture>();
        CHECK(p.tex == g.th);
        CHECK_EQ(p.has_view, 1u);
        CHECK_EQ(p.kind, 0u);
        CHECK_EQ(p.has_region, 1u);
        CHECK_EQ(p.region.x, 1);
        CHECK_EQ(p.region.y, 2);
        CHECK_EQ(p.region.z, 3);
        CHECK_EQ(p.region.w, 4);
        CHECK_EQ(p.region.h, 5);
        CHECK_EQ(p.region.d, 6);
        CHECK_EQ(p.mask, 0xFu);
        for (int i = 0; i < 4; ++i) CHECK_EQ(p.value[i], bits_of(fcol[i]));
    }
    {
        NEXT_OR_RETURN(w, Op::ClearTexture);
        const auto& p = w.r.payload<P_ClearTexture>();
        CHECK(p.tex == g.th2);
        CHECK_EQ(p.has_view, 0u);
        CHECK_EQ(p.has_region, 0u);
        CHECK_EQ(p.kind, 1u);
        CHECK_EQ(p.mask, 0x2u);
        CHECK_EQ(p.value[0], static_cast<uint32_t>(-1));
    }
    {
        NEXT_OR_RETURN(w, Op::ClearTexture);
        const auto& p = w.r.payload<P_ClearTexture>();
        CHECK(p.tex == g.th);
        CHECK_EQ(p.has_view, 0u);
        CHECK_EQ(p.has_region, 1u);
        CHECK_EQ(p.kind, 2u);
        CHECK_EQ(p.mask, 0x9u);
        CHECK_EQ(p.value[0], 0xFFFFFFFFu);
    }
    {
        NEXT_OR_RETURN(w, Op::ClearBuffer);
        const auto& p = w.r.payload<P_ClearBuffer>();
        CHECK_EQ(static_cast<unsigned long long>(p.address), 0x9000ull);
        CHECK_EQ(static_cast<unsigned long long>(p.size), 256ull);
        CHECK_EQ(p.value, 0xDEADBEEFu);
    }
    CHECK(w.at_end());
    // the three texture clears hold their textures
    CHECK_EQ(w.list->held.size(), 3u);
    CHECK(w.list->held[0].get() == static_cast<Obj*>(g.th));
    CHECK(w.list->held[1].get() == static_cast<Obj*>(g.th2));
    CHECK(w.list->held[2].get() == static_cast<Obj*>(g.th));
}

NXTEST(copies_record_regions_strides_and_holds) {
    setup();
    Guests g;
    NVNcommandBuffer c;
    init_cb(&c);
    auto set_row = abi::proc<PFNNVNCOMMANDBUFFERSETCOPYROWSTRIDEPROC>("nvnCommandBufferSetCopyRowStride");
    auto set_img = abi::proc<PFNNVNCOMMANDBUFFERSETCOPYIMAGESTRIDEPROC>("nvnCommandBufferSetCopyImageStride");
    set_row(&c, 256);
    set_img(&c, 4096);
    CB_PROC(BEGINRECORDING, nvnCommandBufferBeginRecording)(&c);
    CB_PROC(COPYBUFFERTOBUFFER, nvnCommandBufferCopyBufferToBuffer)(&c, 0x1000, 0x2000, 512, 3);
    CB_PROC(COPYBUFFERTOTEXTURE, nvnCommandBufferCopyBufferToTexture)(&c, 0x3000, &g.tex, &g.view, &g.region, 1);
    CB_PROC(COPYTEXTURETOBUFFER, nvnCommandBufferCopyTextureToBuffer)(&c, &g.tex2, nullptr, nullptr, 0x4000, 2);
    CB_PROC(COPYTEXTURETOTEXTURE, nvnCommandBufferCopyTextureToTexture)(&c, &g.tex, &g.view, &g.region, &g.tex2, nullptr, &g.region, 5);
    CB_PROC(DOWNSAMPLE, nvnCommandBufferDownsample)(&c, &g.tex, &g.tex2);
    CB_PROC(TILEDDOWNSAMPLE, nvnCommandBufferTiledDownsample)(&c, &g.tex2, &g.tex);
    CB_PROC(DOWNSAMPLETEXTUREVIEW, nvnCommandBufferDownsampleTextureView)(&c, &g.tex, &g.view, &g.tex2, &g.view);
    CB_PROC(TILEDDOWNSAMPLETEXTUREVIEW, nvnCommandBufferTiledDownsampleTextureView)(&c, &g.tex2, &g.view, &g.tex, nullptr);
    NVNcommandHandle h = CB_PROC(ENDRECORDING, nvnCommandBufferEndRecording)(&c);
    Walk w(h);
    CHECK(w.list);
    {
        NEXT_OR_RETURN(w, Op::CopyBufferToBuffer);
        const auto& p = w.r.payload<P_CopyBufferToBuffer>();
        CHECK_EQ(static_cast<unsigned long long>(p.src), 0x1000ull);
        CHECK_EQ(static_cast<unsigned long long>(p.dst), 0x2000ull);
        CHECK_EQ(static_cast<unsigned long long>(p.size), 512ull);
        CHECK_EQ(p.flags, 3u);
    }
    {
        NEXT_OR_RETURN(w, Op::CopyBufferToTexture);
        const auto& p = w.r.payload<P_CopyBufTex>();
        CHECK_EQ(static_cast<unsigned long long>(p.address), 0x3000ull);
        CHECK(p.tex == g.th);
        CHECK_EQ(p.has_view, 1u);
        CHECK_EQ(p.flags, 1u);
        CHECK_EQ(p.has_region, 1u);
        CHECK_EQ(p.region.w, 4);
        CHECK_EQ(static_cast<long long>(p.row_stride), 256ll);
        CHECK_EQ(static_cast<long long>(p.image_stride), 4096ll);
    }
    {
        NEXT_OR_RETURN(w, Op::CopyTextureToBuffer);
        const auto& p = w.r.payload<P_CopyBufTex>();
        CHECK_EQ(static_cast<unsigned long long>(p.address), 0x4000ull);
        CHECK(p.tex == g.th2);
        CHECK_EQ(p.has_view, 0u);
        CHECK_EQ(p.has_region, 0u);
        CHECK_EQ(p.flags, 2u);
        CHECK_EQ(static_cast<long long>(p.row_stride), 256ll);
        CHECK_EQ(static_cast<long long>(p.image_stride), 4096ll);
    }
    {
        NEXT_OR_RETURN(w, Op::CopyTextureToTexture);
        const auto& p = w.r.payload<P_CopyTexTex>();
        CHECK(p.src == g.th);
        CHECK(p.dst == g.th2);
        CHECK_EQ(p.has_src_view, 1u);
        CHECK_EQ(p.has_src_region, 1u);
        CHECK_EQ(p.has_dst_view, 0u);
        CHECK_EQ(p.has_dst_region, 1u);
        CHECK_EQ(p.flags, 5u);
        CHECK_EQ(p.dst_region.d, 6);
    }
    {
        NEXT_OR_RETURN(w, Op::Downsample);
        const auto& p = w.r.payload<P_Downsample>();
        CHECK(p.src == g.th);
        CHECK(p.dst == g.th2);
        CHECK_EQ(p.has_views, 0u);
    }
    {
        NEXT_OR_RETURN(w, Op::Downsample);   // TiledDownsample records the same way
        const auto& p = w.r.payload<P_Downsample>();
        CHECK(p.src == g.th2);
        CHECK(p.dst == g.th);
        CHECK_EQ(p.has_views, 0u);
    }
    {
        NEXT_OR_RETURN(w, Op::Downsample);
        const auto& p = w.r.payload<P_Downsample>();
        CHECK(p.src == g.th);
        CHECK(p.dst == g.th2);
        CHECK_EQ(p.has_views, 1u);
    }
    {
        NEXT_OR_RETURN(w, Op::Downsample);
        const auto& p = w.r.payload<P_Downsample>();
        CHECK(p.src == g.th2);
        CHECK(p.dst == g.th);
        CHECK_EQ(p.has_views, 1u);
    }
    CHECK(w.at_end());
    CHECK_EQ(w.list->record_count, 8u);
    // holds: buffer-to-texture 1, texture-to-buffer 1, texture-to-texture 2, four downsamples 2 each
    CHECK_EQ(w.list->held.size(), 12u);
}

NXTEST(uniform_update_copies_its_data_and_clamps_size) {
    setup();
    NVNcommandBuffer c;
    init_cb(&c);
    uint8_t src[16];
    for (int i = 0; i < 16; ++i) src[i] = static_cast<uint8_t>(0x10 + i);
    std::vector<uint8_t> big(70000, 0x7e);
    CB_PROC(BEGINRECORDING, nvnCommandBufferBeginRecording)(&c);
    CB_PROC(UPDATEUNIFORMBUFFER, nvnCommandBufferUpdateUniformBuffer)(&c, 0x5000, 4096, 32, 16, src);
    std::memset(src, 0, sizeof src);   // the game reuses its source right away
    CB_PROC(UPDATEUNIFORMBUFFER, nvnCommandBufferUpdateUniformBuffer)(&c, 0x5100, 16, 0, 16, nullptr);   // refused
    CB_PROC(UPDATEUNIFORMBUFFER, nvnCommandBufferUpdateUniformBuffer)(&c, 0x6000, 1u << 20, 0, 70000, big.data());
    NVNcommandHandle h = CB_PROC(ENDRECORDING, nvnCommandBufferEndRecording)(&c);
    Walk w(h);
    CHECK(w.list);
    {
        NEXT_OR_RETURN(w, Op::UpdateUniformBuffer);
        const auto& p = w.r.payload<P_UpdateUniformBuffer>();
        CHECK_EQ(static_cast<unsigned long long>(p.address), 0x5000ull);
        CHECK_EQ(static_cast<unsigned long long>(p.buffer_size), 4096ull);
        CHECK_EQ(static_cast<long long>(p.offset), 32ll);
        CHECK_EQ(static_cast<unsigned long long>(p.size), 16ull);
        const uint8_t* blob = w.r.blob<P_UpdateUniformBuffer>();
        for (int i = 0; i < 16; ++i) CHECK_EQ(blob[i], static_cast<uint8_t>(0x10 + i));
    }
    {
        NEXT_OR_RETURN(w, Op::UpdateUniformBuffer);
        const auto& p = w.r.payload<P_UpdateUniformBuffer>();
        CHECK_EQ(static_cast<unsigned long long>(p.address), 0x6000ull);
        CHECK_EQ(static_cast<unsigned long long>(p.size), 65536ull);
        const uint8_t* blob = w.r.blob<P_UpdateUniformBuffer>();
        bool all = true;
        for (int i = 0; i < 65536; ++i) all = all && blob[i] == 0x7e;
        CHECK(all);
    }
    CHECK(w.at_end());
}

NXTEST(counters_barrier_sync_and_events) {
    setup();
    Guests g;
    NVNcommandBuffer c;
    init_cb(&c);
    CB_PROC(BEGINRECORDING, nvnCommandBufferBeginRecording)(&c);
    CB_PROC(REPORTCOUNTER, nvnCommandBufferReportCounter)(&c, NVN_COUNTER_TYPE_SAMPLES_PASSED, 0xA000);
    CB_PROC(RESETCOUNTER, nvnCommandBufferResetCounter)(&c, NVN_COUNTER_TYPE_SAMPLES_PASSED);
    CB_PROC(REPORTVALUE, nvnCommandBufferReportValue)(&c, 0xCAFEBABEu, 0xB000);
    CB_PROC(BARRIER, nvnCommandBufferBarrier)(&c, 0x11);
    CB_PROC(FENCESYNC, nvnCommandBufferFenceSync)(&c, &g.sync, NVN_SYNC_CONDITION_ALL_GPU_COMMANDS_COMPLETE, 0x5);
    CB_PROC(WAITSYNC, nvnCommandBufferWaitSync)(&c, &g.sync);
    CB_PROC(SIGNALEVENT, nvnCommandBufferSignalEvent)(&c, &g.event, NVN_EVENT_SIGNAL_MODE_ADD, NVN_EVENT_SIGNAL_LOCATION_BOTTOM, 3, 0x55);
    CB_PROC(WAITEVENT, nvnCommandBufferWaitEvent)(&c, &g.event, NVN_EVENT_WAIT_MODE_GEQUAL_WRAP, 0x77);
    NVNcommandHandle h = CB_PROC(ENDRECORDING, nvnCommandBufferEndRecording)(&c);
    Walk w(h);
    CHECK(w.list);
    {
        NEXT_OR_RETURN(w, Op::ReportCounter);
        const auto& p = w.r.payload<P_Counter>();
        CHECK_EQ(p.type, static_cast<uint32_t>(NVN_COUNTER_TYPE_SAMPLES_PASSED));
        CHECK_EQ(static_cast<unsigned long long>(p.address), 0xA000ull);
    }
    {
        NEXT_OR_RETURN(w, Op::ResetCounter);
        const auto& p = w.r.payload<P_Counter>();
        CHECK_EQ(p.type, static_cast<uint32_t>(NVN_COUNTER_TYPE_SAMPLES_PASSED));
        CHECK_EQ(static_cast<unsigned long long>(p.address), 0ull);
    }
    {
        NEXT_OR_RETURN(w, Op::ReportValue);
        const auto& p = w.r.payload<P_ReportValue>();
        CHECK_EQ(p.value, 0xCAFEBABEu);
        CHECK_EQ(static_cast<unsigned long long>(p.address), 0xB000ull);
    }
    {
        NEXT_OR_RETURN(w, Op::Barrier);
        CHECK_EQ(w.r.payload<P_Barrier>().bits, 0x11u);
    }
    {
        NEXT_OR_RETURN(w, Op::FenceSync);
        const auto& p = w.r.payload<P_Sync>();
        CHECK(p.sync == g.sh);
        CHECK_EQ(p.condition, static_cast<uint32_t>(NVN_SYNC_CONDITION_ALL_GPU_COMMANDS_COMPLETE));
        CHECK_EQ(p.flags, 0x5u);
    }
    {
        NEXT_OR_RETURN(w, Op::WaitSync);
        CHECK(w.r.payload<P_Sync>().sync == g.sh);
    }
    {
        NEXT_OR_RETURN(w, Op::SignalEvent);
        const auto& p = w.r.payload<P_Event>();
        CHECK(p.event == g.eh);
        CHECK_EQ(p.mode, static_cast<uint32_t>(NVN_EVENT_SIGNAL_MODE_ADD));
        CHECK_EQ(p.location, static_cast<uint32_t>(NVN_EVENT_SIGNAL_LOCATION_BOTTOM));
        CHECK_EQ(p.index, 3u);
        CHECK_EQ(p.value, 0x55u);
    }
    {
        NEXT_OR_RETURN(w, Op::WaitEvent);
        const auto& p = w.r.payload<P_Event>();
        CHECK(p.event == g.eh);
        CHECK_EQ(p.mode, static_cast<uint32_t>(NVN_EVENT_WAIT_MODE_GEQUAL_WRAP));
        CHECK_EQ(p.value, 0x77u);
    }
    CHECK(w.at_end());
    CHECK_EQ(w.list->held.size(), 4u);   // fence, wait on the sync, signal on the event, wait on the event: one hold each
}

NXTEST(compute_discards_and_nop_debug_zcull) {
    setup();
    NVNcommandBuffer c;
    init_cb(&c);
    CB_PROC(BEGINRECORDING, nvnCommandBufferBeginRecording)(&c);
    CB_PROC(DISPATCHCOMPUTE, nvnCommandBufferDispatchCompute)(&c, 4, 5, 6);
    CB_PROC(DISPATCHCOMPUTEINDIRECT, nvnCommandBufferDispatchComputeIndirect)(&c, 0xC000);
    CB_PROC(DISCARDCOLOR, nvnCommandBufferDiscardColor)(&c, 3);
    CB_PROC(DISCARDDEPTHSTENCIL, nvnCommandBufferDiscardDepthStencil)(&c);
    CB_PROC(RESOLVEDEPTHBUFFER, nvnCommandBufferResolveDepthBuffer)(&c);
    // debug and ZCull: nothing recorded
    CB_PROC(SAVEZCULLDATA, nvnCommandBufferSaveZCullData)(&c, 0xD000, 64);
    CB_PROC(RESTOREZCULLDATA, nvnCommandBufferRestoreZCullData)(&c, 0xD000, 64);
    CB_PROC(PUSHDEBUGGROUPSTATIC, nvnCommandBufferPushDebugGroupStatic)(&c, 1, "group");
    CB_PROC(PUSHDEBUGGROUPDYNAMIC, nvnCommandBufferPushDebugGroupDynamic)(&c, 2, "group");
    CB_PROC(PUSHDEBUGGROUP, nvnCommandBufferPushDebugGroup)(&c, 3, "group");
    CB_PROC(POPDEBUGGROUP, nvnCommandBufferPopDebugGroup)(&c);
    CB_PROC(POPDEBUGGROUPID, nvnCommandBufferPopDebugGroupId)(&c, 3);
    CB_PROC(INSERTDEBUGMARKERSTATIC, nvnCommandBufferInsertDebugMarkerStatic)(&c, 4, "mark");
    CB_PROC(INSERTDEBUGMARKERDYNAMIC, nvnCommandBufferInsertDebugMarkerDynamic)(&c, 5, "mark");
    CB_PROC(INSERTDEBUGMARKER, nvnCommandBufferInsertDebugMarker)(&c, "mark");
    NVNcommandHandle h = CB_PROC(ENDRECORDING, nvnCommandBufferEndRecording)(&c);
    Walk w(h);
    CHECK(w.list);
    CHECK_EQ(w.list->draw_count, 0u);
    {
        NEXT_OR_RETURN(w, Op::Dispatch);
        const auto& p = w.r.payload<P_Dispatch>();
        CHECK_EQ(p.x, 4u);
        CHECK_EQ(p.y, 5u);
        CHECK_EQ(p.z, 6u);
    }
    {
        NEXT_OR_RETURN(w, Op::DispatchIndirect);
        CHECK_EQ(static_cast<unsigned long long>(w.r.payload<P_DispatchIndirect>().address), 0xC000ull);
    }
    {
        NEXT_OR_RETURN(w, Op::DiscardColor);
        CHECK_EQ(w.r.payload<P_U32>().v, 3u);
    }
    {
        NEXT_OR_RETURN(w, Op::DiscardDepthStencil);
        CHECK_EQ(w.r.size(), static_cast<uint32_t>(sizeof(RecHeader)));
    }
    {
        NEXT_OR_RETURN(w, Op::ResolveDepth);
        CHECK_EQ(w.r.size(), static_cast<uint32_t>(sizeof(RecHeader)));
    }
    CHECK(w.at_end());
}
