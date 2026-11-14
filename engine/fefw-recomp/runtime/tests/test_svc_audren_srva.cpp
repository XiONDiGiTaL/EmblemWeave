// SPDX-License-Identifier: GPL-3.0-or-later
// Server/ ports of the audio renderer: BehaviourInfo, MemoryPool (MemoryPoolInfo, AddressInfo, PoolMapper), Upsampler, Sink.
//
// Every expected value below is derived by hand from the C# sources under refs/ryubing/src/Ryujinx.Audio/Renderer/Server
// (and Constants.cs, Renderer/Common/BehaviourParameter), not from running the port. The derivation is noted next to each group.
#include <cstring>
#include <memory>
#include <string>
#include <vector>

#include "../src/services/audren/Server/BehaviourInfo.h"
#include "../src/services/audren/Server/MemoryPool/AddressInfo.h"
#include "../src/services/audren/Server/MemoryPool/PoolMapper.h"
#include "../src/services/audren/Server/Sink/CircularBufferSink.h"
#include "../src/services/audren/Server/Sink/DeviceSink.h"
#include "../src/services/audren/Server/Sink/SinkContext.h"
#include "../src/services/audren/Server/Upsampler/UpsamplerManager.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_srva)

using namespace nxrt::audren;

namespace {

// ---- helpers ----

// C# BehaviourInfo.BaseRevisionMagic = 'R' | 'E' << 8 | 'V' << 16 | '0' << 24 = 0x30564552.
// A user revision N is BaseRevisionMagic + (N << 24), so its low three bytes read "REV" and its top byte is '0' + N:
//   N=1 -> 0x31564552, N=5 -> 0x35564552, N=10 -> 0x3A564552, N=11 -> 0x3B564552, N=13 -> 0x3D564552.
using Predicate = bool (BehaviourInfo::*)() const;

// Predicates in the order of the C# file, with the minimum revision each one needs (C# CheckFeatureSupported argument):
//   0 IsAdpcmLoopContextBugFixed (2)            1 IsSplitterSupported (2)               2 IsLongSizePreDelaySupported (3)
//   3 IsAudioUsbDeviceOutputSupported (4)       4 IsFlushVoiceWaveBuffersSupported (5)  5 IsSplitterBugFixed (5)
//   6 IsElapsedFrameCountSupported (5)          7 IsDecodingBehaviourFlagSupported (5)  8 IsBiquadFilterEffectStateClearBugFixed (6)
//   9 IsMixInParameterDirtyOnlyUpdateSupported (7)  10 IsWaveBufferVersion2Supported (8)  11 IsEffectInfoVersion2Supported (9)
//   12 UseMultiTapBiquadFilterProcessing (10)   13 IsNewEffectChannelMappingSupported (11)
//   14 IsBiquadFilterParameterForSplitterEnabled (12)  15 IsSplitterPrevVolumeResetSupported (13)
//   16 IsSplitterDepopBugFixEnabled (14)        17 IsBiquadFilterParameterFloatSupported (15)
const Predicate kPredicates[18] = {
    &BehaviourInfo::IsAdpcmLoopContextBugFixed,
    &BehaviourInfo::IsSplitterSupported,
    &BehaviourInfo::IsLongSizePreDelaySupported,
    &BehaviourInfo::IsAudioUsbDeviceOutputSupported,
    &BehaviourInfo::IsFlushVoiceWaveBuffersSupported,
    &BehaviourInfo::IsSplitterBugFixed,
    &BehaviourInfo::IsElapsedFrameCountSupported,
    &BehaviourInfo::IsDecodingBehaviourFlagSupported,
    &BehaviourInfo::IsBiquadFilterEffectStateClearBugFixed,
    &BehaviourInfo::IsMixInParameterDirtyOnlyUpdateSupported,
    &BehaviourInfo::IsWaveBufferVersion2Supported,
    &BehaviourInfo::IsEffectInfoVersion2Supported,
    &BehaviourInfo::UseMultiTapBiquadFilterProcessing,
    &BehaviourInfo::IsNewEffectChannelMappingSupported,
    &BehaviourInfo::IsBiquadFilterParameterForSplitterEnabled,
    &BehaviourInfo::IsSplitterPrevVolumeResetSupported,
    &BehaviourInfo::IsSplitterDepopBugFixEnabled,
    &BehaviourInfo::IsBiquadFilterParameterFloatSupported,
};

// Expected answers per user revision, read off the minimum revisions above (true when user revision >= minimum).
struct RevisionCase {
    int32_t number;
    uint32_t magic;
    bool gate[18];
    float processingLimit;  // GetAudioRendererProcessingTimeLimit: 0.80 from REV5, 0.75 from REV4, else 0.70
    uint32_t perfFormat;    // GetPerformanceMetricsDataFormat: 2 from REV5, else 1
    int32_t estimator;      // GetCommandProcessingTimeEstimatorVersion: 5 from REV11, 4 from REV10, 3 from REV8, 2 from REV5, else 1
};

const RevisionCase kRevisionCases[] = {
    // REV1: every gate needs at least REV2, so all false.
    {1, 0x31564552, {false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false, false}, 0.70f, 1, 1},
    // REV5: gates 0..7 (minimum 2..5) true, gates 8..17 (minimum 6..15) false.
    {5, 0x35564552, {true, true, true, true, true, true, true, true, false, false, false, false, false, false, false, false, false, false}, 0.80f, 2, 2},
    // REV10: gates 0..12 (minimum 2..10) true, gates 13..17 (minimum 11..15) false.
    {10, 0x3A564552, {true, true, true, true, true, true, true, true, true, true, true, true, true, false, false, false, false, false}, 0.80f, 2, 4},
    // REV11: gates 0..13 (minimum 2..11) true, gates 14..17 (minimum 12..15) false.
    {11, 0x3B564552, {true, true, true, true, true, true, true, true, true, true, true, true, true, true, false, false, false, false}, 0.80f, 2, 5},
    // REV13: gates 0..15 (minimum 2..13) true, gates 16..17 (minimum 14..15) false.
    {13, 0x3D564552, {true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, true, false, false}, 0.80f, 2, 5},
};

BehaviourParameter::ErrorInfo MakeError(ResultCode code, uint64_t extra) {
    BehaviourParameter::ErrorInfo e{};
    e.error_code = code;
    e.extra_error_info = extra;
    return e;
}

// A SinkInParameter with the common fields set; the specific data is filled by SetSpecific.
SinkInParameter MakeSink(SinkType type, bool used, int32_t nodeId) {
    SinkInParameter p{};
    p.type = type;
    p.is_used = used;
    p.magic = 0xCAFE;
    p.node_id = nodeId;
    return p;
}

template <class T>
void SetSpecific(SinkInParameter& p, const T& data) {
    std::memcpy(p.specific_data, &data, sizeof(T));
}

// Attach [0x1000, 0x5000) to the single pool through MemoryPoolInParameter RequestAttach.
void AttachPool(PoolMapper& mapper, std::vector<MemoryPoolInfo>& pools) {
    MemoryPoolInParameter in{};
    in.cpu_address = 0x1000;
    in.size = 0x4000;
    in.state = MemoryPoolUserState::RequestAttach;
    MemoryPoolOutStatus out{};
    (void)mapper.Update(pools[0], in, out);
}

}  // namespace

// ---- BehaviourInfo ----

// Derived from BehaviourInfo.cs: BaseRevisionMagic = 'R' | 'E'<<8 | 'V'<<16 | '0'<<24 = 0x30564552; Revision5 = 5<<24 = 0x05000000;
// LastRevision = Revision15 = 0x0F000000; ProcessRevision = magic + LastRevision = 0x3F564552.
NXTEST(behaviour_revision_constants_and_validity) {
    CHECK_EQ(BehaviourInfo::BaseRevisionMagic, 0x30564552);
    CHECK_EQ(BehaviourInfo::Revision5, 0x05000000);
    CHECK_EQ(BehaviourInfo::LastRevision, 0x0F000000);
    CHECK_EQ(BehaviourInfo::ProcessRevision, 0x3F564552);
    CHECK_EQ(BehaviourInfo::GetRevisionNumber(0x35564552), 5);
    // Revision 15 is the last supported; revision 16 (0x40564552) is not valid.
    CHECK(BehaviourInfo::CheckValidRevision(0x3F564552));
    CHECK(!BehaviourInfo::CheckValidRevision(0x40564552));
    // CheckFeatureSupported (C#): `revA > LastRevision` compares a revision NUMBER with LastRevision, which is the shifted
    // constant 15 << 24. So the "count as 1" branch never fires for real input and revision 16 is compared as 16: it passes REV5.
    CHECK(BehaviourInfo::CheckFeatureSupported(0x40564552, BehaviourInfo::BaseRevisionMagic + BehaviourInfo::Revision1));
    CHECK(BehaviourInfo::CheckFeatureSupported(0x40564552, BehaviourInfo::BaseRevisionMagic + BehaviourInfo::Revision5));
    CHECK(BehaviourInfo::CheckFeatureSupported(0x35564552, BehaviourInfo::BaseRevisionMagic + BehaviourInfo::Revision5));
}

// For each user revision in the table, every Is*Supported / Get* answer must match the C# comparison.
NXTEST(behaviour_revision_gating_per_revision) {
    for (const RevisionCase& c : kRevisionCases) {
        BehaviourInfo bi;
        bi.SetUserRevision(static_cast<int32_t>(c.magic));
        CHECK_EQ(bi.UserRevision, static_cast<int32_t>(c.magic));
        CHECK_EQ(BehaviourInfo::GetRevisionNumber(static_cast<int32_t>(c.magic)), c.number);

        for (int i = 0; i < 18; i++) {
            CHECK_MSG((bi.*kPredicates[i])() == c.gate[i], "predicate index " + std::to_string(i) + " at revision " + std::to_string(c.number));
        }
        CHECK_MSG(bi.GetAudioRendererProcessingTimeLimit() == c.processingLimit, "processing limit at revision " + std::to_string(c.number));
        CHECK_MSG(bi.GetPerformanceMetricsDataFormat() == c.perfFormat, "perf format at revision " + std::to_string(c.number));
        CHECK_MSG(bi.GetCommandProcessingTimeEstimatorVersion() == c.estimator, "estimator at revision " + std::to_string(c.number));
    }
}

// C#: IsMemoryPoolForceMappingEnabled tests bit 0 of the flags, whatever the revision. UpdateFlags replaces the whole value.
NXTEST(behaviour_flags_force_mapping_bit) {
    BehaviourInfo bi;
    CHECK(!bi.IsMemoryPoolForceMappingEnabled());
    bi.UpdateFlags(1);
    CHECK(bi.IsMemoryPoolForceMappingEnabled());
    bi.UpdateFlags(0xFFFE);
    CHECK(!bi.IsMemoryPoolForceMappingEnabled());
    bi.UpdateFlags(0x3);
    CHECK(bi.IsMemoryPoolForceMappingEnabled());
}

// C#: AppendError stores while _errorIndex <= MaxErrorInfos - 1 (i.e. at most 10 entries), CopyErrorInfo reports
// min(_errorIndex, 10) and copies the whole array, ClearError resets the index. Release builds compile the C# Debug.Assert away.
NXTEST(behaviour_error_list_append_limit_clear) {
    BehaviourInfo bi;
    BehaviourParameter::ErrorInfo out[10]{};
    uint32_t count = 99;

    bi.CopyErrorInfo(Span<BehaviourParameter::ErrorInfo>(out, 10), count);
    CHECK_EQ(count, 0u);

    BehaviourParameter::ErrorInfo e1 = MakeError(ResultCode::DeviceNotFound, 1);
    BehaviourParameter::ErrorInfo e2 = MakeError(ResultCode::OperationFailed, 2);
    BehaviourParameter::ErrorInfo e3 = MakeError(ResultCode::InvalidAddressInfo, 3);
    bi.AppendError(e1);
    bi.AppendError(e2);
    bi.AppendError(e3);
    bi.CopyErrorInfo(Span<BehaviourParameter::ErrorInfo>(out, 10), count);
    CHECK_EQ(count, 3u);
    CHECK(out[0].error_code == ResultCode::DeviceNotFound);
    CHECK_EQ(out[0].extra_error_info, 1u);
    CHECK(out[2].error_code == ResultCode::InvalidAddressInfo);
    CHECK_EQ(out[2].extra_error_info, 3u);
    CHECK_EQ(out[3].extra_error_info, 0u);

    // Seven more fill slots 3..9 (extra 100..106); the next two are dropped.
    for (uint64_t i = 0; i < 9; i++) {
        BehaviourParameter::ErrorInfo e = MakeError(ResultCode::UnsupportedOperation, 100 + i);
        bi.AppendError(e);
    }
    bi.CopyErrorInfo(Span<BehaviourParameter::ErrorInfo>(out, 10), count);
    CHECK_EQ(count, 10u);
    CHECK_EQ(out[9].extra_error_info, 106u);

    bi.ClearError();
    bi.CopyErrorInfo(Span<BehaviourParameter::ErrorInfo>(out, 10), count);
    CHECK_EQ(count, 0u);
}

// ---- MemoryPool ----

// C#: Contains is [cpu, cpu+size) inclusive-start; Translate = DspAddress + offset when contained and mapped, else 0.
NXTEST(memory_pool_info_contains_and_translate) {
    MemoryPoolInfo created = MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Dsp);
    CHECK(created.Location == MemoryPoolInfo::LocationType::Dsp);
    CHECK_EQ(created.CpuAddress, 0u);
    CHECK(!created.IsMapped());

    MemoryPoolInfo p = MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu);
    p.SetCpuAddress(0x1000, 0x4000);
    p.DspAddress = 0x1000;
    CHECK(p.Contains(0x1000, 0x4000));
    CHECK(p.Contains(0x4FFF, 1));
    CHECK(!p.Contains(0x4F00, 0x200));  // ends at 0x5100 > 0x5000
    CHECK(!p.Contains(0xFFF, 1));
    CHECK_EQ(p.Translate(0x2000, 0x100), 0x2000u);  // 0x1000 + (0x2000 - 0x1000)
    CHECK_EQ(p.Translate(0x6000, 1), 0u);
    p.DspAddress = 0;
    CHECK_EQ(p.Translate(0x2000, 0x100), 0u);  // not mapped
}

// C#: Update with RequestAttach then RequestDetach moves Detached -> Attached -> Detached; any other state is ignored.
NXTEST(pool_mapper_update_state_transitions) {
    std::vector<MemoryPoolInfo> pools(1, MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu));
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);

    MemoryPoolOutStatus out{};
    out.state = MemoryPoolUserState::Detached;
    MemoryPoolInParameter in{};
    in.cpu_address = 0x1000;
    in.size = 0x4000;

    in.state = MemoryPoolUserState::RequestAttach;
    CHECK(mapper.Update(pools[0], in, out) == PoolMapper::UpdateResult::Success);
    CHECK(out.state == MemoryPoolUserState::Attached);
    CHECK_EQ(pools[0].CpuAddress, 0x1000u);
    CHECK_EQ(pools[0].Size, 0x4000u);
    CHECK_EQ(pools[0].DspAddress, 0x1000u);  // identity mapping

    in.state = MemoryPoolUserState::RequestDetach;
    CHECK(mapper.Update(pools[0], in, out) == PoolMapper::UpdateResult::Success);
    CHECK(out.state == MemoryPoolUserState::Detached);
    CHECK_EQ(pools[0].CpuAddress, 0u);
    CHECK_EQ(pools[0].Size, 0u);
    CHECK_EQ(pools[0].DspAddress, 0u);

    // Not a request: no change to the pool or the output status.
    in.state = MemoryPoolUserState::New;
    out.state = MemoryPoolUserState::Detached;
    CHECK(mapper.Update(pools[0], in, out) == PoolMapper::UpdateResult::Success);
    CHECK(out.state == MemoryPoolUserState::Detached);
    CHECK_EQ(pools[0].CpuAddress, 0u);
}

// C#: RequestAttach needs a non-zero, page-aligned (0x1000) address and size; a detach needs the same address and size,
// and a pool still marked IsUsed cannot be unmapped (UnmapError).
NXTEST(pool_mapper_update_errors) {
    std::vector<MemoryPoolInfo> pools(1, MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu));
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);
    MemoryPoolOutStatus out{};
    MemoryPoolInParameter in{};
    in.state = MemoryPoolUserState::RequestAttach;

    in.cpu_address = 0;
    in.size = 0x4000;
    CHECK(mapper.Update(pools[0], in, out) == PoolMapper::UpdateResult::InvalidParameter);
    in.cpu_address = 0x1800;  // not page aligned
    CHECK(mapper.Update(pools[0], in, out) == PoolMapper::UpdateResult::InvalidParameter);
    in.cpu_address = 0x1000;
    in.size = 0x123;  // not page aligned
    CHECK(mapper.Update(pools[0], in, out) == PoolMapper::UpdateResult::InvalidParameter);
    in.size = 0;
    CHECK(mapper.Update(pools[0], in, out) == PoolMapper::UpdateResult::InvalidParameter);
    CHECK_EQ(pools[0].CpuAddress, 0u);

    in.size = 0x4000;
    CHECK(mapper.Update(pools[0], in, out) == PoolMapper::UpdateResult::Success);

    // Detach with a different address: rejected, pool stays attached.
    in.state = MemoryPoolUserState::RequestDetach;
    in.cpu_address = 0x2000;
    CHECK(mapper.Update(pools[0], in, out) == PoolMapper::UpdateResult::InvalidParameter);
    CHECK_EQ(pools[0].CpuAddress, 0x1000u);

    // Detach while in use: UnmapError, pool stays attached.
    in.cpu_address = 0x1000;
    pools[0].IsUsed = true;
    CHECK(mapper.Update(pools[0], in, out) == PoolMapper::UpdateResult::UnmapError);
    CHECK_EQ(pools[0].CpuAddress, 0x1000u);

    // Clearing the usage state lets the detach through.
    PoolMapper::ClearUsageState(Span<MemoryPoolInfo>(pools));
    CHECK(!pools[0].IsUsed);
    CHECK(mapper.Update(pools[0], in, out) == PoolMapper::UpdateResult::Success);
    CHECK(out.state == MemoryPoolUserState::Detached);
}

// C#: TryAttachBuffer finds the pool containing the region, sets the AddressInfo to it, and GetReference translates the address.
// A region that is not inside any pool records InvalidAddressInfo with the CPU address as extra info, and returns the force-map flag.
NXTEST(pool_mapper_attach_buffer_translate_and_not_found) {
    std::vector<MemoryPoolInfo> pools(1, MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu));
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);
    AttachPool(mapper, pools);
    REQUIRE(pools[0].IsMapped());

    // Inside [0x1000, 0x5000): found, DSP address = 0x1000 + (0x2000 - 0x1000) = 0x2000.
    AddressInfo inside;
    BehaviourParameter::ErrorInfo err{};
    CHECK(mapper.TryAttachBuffer(err, inside, 0x2000, 0x100));
    CHECK(err.error_code == ResultCode::Success);
    CHECK(inside.HasMappedMemoryPool());
    CHECK_EQ(inside.GetReference(false), 0x2000u);
    CHECK(!pools[0].IsUsed);
    CHECK_EQ(inside.GetReference(true), 0x2000u);
    CHECK(pools[0].IsUsed);  // GetReference(markUsed) marks the pool

    // Not inside (ends past 0x5000): not found, error recorded, returns false (no force mapping), reference 0.
    AddressInfo outside;
    CHECK(!mapper.TryAttachBuffer(err, outside, 0x4F00, 0x200));
    CHECK(err.error_code == ResultCode::InvalidAddressInfo);
    CHECK_EQ(err.extra_error_info, 0x4F00u);
    CHECK(!outside.HasMappedMemoryPool());
    CHECK_EQ(outside.GetReference(false), 0u);

    // With force mapping enabled the not-found region is still reported as an error, but the mapping is performed and returns true.
    PoolMapper forced(0, Span<MemoryPoolInfo>(pools), true);
    AddressInfo forcedInfo;
    CHECK(forced.TryAttachBuffer(err, forcedInfo, 0x6000, 0x100));
    CHECK(err.error_code == ResultCode::InvalidAddressInfo);
    CHECK_EQ(forcedInfo.GetReference(false), 0x6000u);
}

// C#: Map / InitializeSystemPool. A CPU-located pool cannot be a system pool; a DSP-located one maps to its CPU address.
NXTEST(pool_mapper_map_and_system_pool) {
    std::vector<MemoryPoolInfo> pools(1, MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu));
    PoolMapper mapper(0x1234, Span<MemoryPoolInfo>(pools), false);

    MemoryPoolInfo cpuPool = MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu);
    CHECK(!mapper.InitializeSystemPool(cpuPool, 0x9000, 0x1000));
    CHECK_EQ(cpuPool.CpuAddress, 0u);

    MemoryPoolInfo dspPool = MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Dsp);
    CHECK(mapper.InitializeSystemPool(dspPool, 0x9000, 0x1000));
    CHECK_EQ(dspPool.DspAddress, 0x9000u);
    CHECK_EQ(mapper.GetProcessHandle(dspPool), 0x1234u);
    CHECK_EQ(mapper.GetProcessHandle(cpuPool), PoolMapper::CurrentProcessPseudoHandle);

    MemoryPoolInfo q = MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu);
    q.SetCpuAddress(0x8000, 0x1000);
    CHECK_EQ(mapper.Map(q), 0x8000u);
    CHECK(mapper.Unmap(q));
    CHECK_EQ(q.DspAddress, 0u);
    CHECK_EQ(q.CpuAddress, 0u);
}

NXTEST(address_info_defaults_and_setup) {
    AddressInfo a = AddressInfo::Create();
    CHECK_EQ(a.CpuAddress, 0u);
    CHECK(!a.HasMemoryPoolState());
    CHECK_EQ(a.GetReference(false), 0u);

    AddressInfo b = AddressInfo::Create(0x1000, 0x10);
    CHECK_EQ(b.CpuAddress, 0x1000u);
    CHECK_EQ(b.Size, 0x10u);
    CHECK(!b.HasMappedMemoryPool());
}

// ---- Upsampler ----

// C#: UpSampleEntrySize = TargetSampleCount (240) * VoiceChannelCountMax (6) = 1440 floats per slot.
// Allocate hands out the first free slot, Free makes it available again, and Allocate returns null when all slots are used.
NXTEST(upsampler_manager_allocate_and_free) {
    std::vector<float> work(2 * 1440, 0.0f);
    UpsamplerManager mgr(Span<float>(work), 2);

    UpsamplerInfo* a = mgr.Allocate();
    REQUIRE(a != nullptr);
    CHECK_EQ(a->OutputBuffer.Length, 1440u);
    CHECK(a->OutputBuffer.Data == work.data());
    CHECK_EQ(a->SampleCount, 240u);

    UpsamplerInfo* b = mgr.Allocate();
    REQUIRE(b != nullptr);
    CHECK(b->OutputBuffer.Data == work.data() + 1440);
    CHECK(mgr.Allocate() == nullptr);

    a->Release();  // frees slot 0
    UpsamplerInfo* c = mgr.Allocate();
    REQUIRE(c != nullptr);
    CHECK(c->OutputBuffer.Data == work.data());
    CHECK(mgr.Allocate() == nullptr);
}

// ---- Sink ----

NXTEST(sink_context_slots_and_replacement) {
    SinkContext ctx;
    ctx.Initialize(3);
    CHECK_EQ(ctx.GetCount(), 3u);
    CHECK(ctx.GetSink(2).Type == SinkType::Invalid);
    CHECK(!ctx.GetSink(0).IsUsed);

    // The StateUpdater.ResetSink step: the slot is replaced by a derived sink.
    ctx.GetSinkSlot(1) = std::make_unique<DeviceSink>();
    CHECK(ctx.GetSink(1).TargetSinkType() == SinkType::Device);
    CHECK(ctx.GetSink(1).Type == SinkType::Device);
}

// C#: the constructor runs CleanUp, which sets Type to TargetSinkType; IsTypeValid compares the parameter type with it.
NXTEST(sink_types_and_type_validation) {
    CircularBufferSink cb;
    DeviceSink dev;
    CHECK(cb.Type == SinkType::CircularBuffer);
    CHECK(dev.Type == SinkType::Device);

    SinkInParameter deviceParam = MakeSink(SinkType::Device, true, 3);
    CHECK(dev.IsTypeValid(deviceParam));
    CHECK(!cb.IsTypeValid(deviceParam));
}

// C#: attach at the first used update (AddressInfo to the pool), three command generations of
// frame = TargetSampleSize(2) * SampleCount(240) * InputCount(2) = 960 bytes on a 2048-byte buffer, then detach.
//   gen 1: last 0,   old 0,    cur 960
//   gen 2: last 0,   old 960,  cur 1920
//   gen 3: last 960, old 1920, cur (2880 % 2048) = 832
NXTEST(circular_buffer_sink_attach_write_detach) {
    std::vector<MemoryPoolInfo> pools(1, MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu));
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);
    AttachPool(mapper, pools);

    CircularBufferParameter cbp{};
    cbp.buffer_address = 0x2000;
    cbp.buffer_size = 0x800;
    cbp.input_count = 2;
    cbp.sample_count = 240;

    CircularBufferSink s;
    BehaviourParameter::ErrorInfo err{};
    SinkOutStatus out{};
    SinkInParameter attach = MakeSink(SinkType::CircularBuffer, true, 7);
    SetSpecific(attach, cbp);

    s.Update(err, attach, out, mapper);
    CHECK(err.error_code == ResultCode::Success);
    CHECK(s.IsUsed);
    CHECK_EQ(s.NodeId, 7);
    CHECK(!s.ShouldSkip);
    CHECK_EQ(out.last_written_offset, 0u);
    CHECK_EQ(s.Parameter.buffer_size, 0x800u);
    CHECK_EQ(s.CircularBufferAddressInfo.GetReference(false), 0x2000u);

    s.UpdateForCommandGeneration();
    s.UpdateForCommandGeneration();
    s.UpdateForCommandGeneration();
    CHECK_EQ(s.CurrentWriteOffset, 832u);

    s.Update(err, attach, out, mapper);
    CHECK_EQ(out.last_written_offset, 960u);

    // Detach: IsUsed goes false and the node id is taken from the new parameter; command generation no longer moves the offset.
    SinkInParameter detach = MakeSink(SinkType::CircularBuffer, false, 9);
    SetSpecific(detach, cbp);
    s.Update(err, detach, out, mapper);
    CHECK(err.error_code == ResultCode::Success);
    CHECK(!s.IsUsed);
    CHECK_EQ(s.NodeId, 9);
    CHECK_EQ(out.last_written_offset, 960u);
    s.UpdateForCommandGeneration();
    CHECK_EQ(s.CurrentWriteOffset, 832u);
}

// C#: a region outside every pool gives InvalidAddressInfo (extra = CPU address) and ShouldSkip = !TryAttachBuffer (true).
NXTEST(circular_buffer_sink_unmapped_buffer_is_skipped) {
    std::vector<MemoryPoolInfo> pools(1, MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu));
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);
    AttachPool(mapper, pools);

    CircularBufferParameter cbp{};
    cbp.buffer_address = 0x6000;
    cbp.buffer_size = 0x800;
    cbp.input_count = 2;
    cbp.sample_count = 240;

    CircularBufferSink s;
    BehaviourParameter::ErrorInfo err{};
    SinkOutStatus out{};
    SinkInParameter attach = MakeSink(SinkType::CircularBuffer, true, 4);
    SetSpecific(attach, cbp);
    s.Update(err, attach, out, mapper);
    CHECK(err.error_code == ResultCode::InvalidAddressInfo);
    CHECK_EQ(err.extra_error_info, 0x6000u);
    CHECK(s.ShouldSkip);
}

// C#: the first update copies the common parameters (IsUsed, NodeId) and the device parameter. A later update with the same
// IsUsed only refreshes the enable flag and the coefficients, and keeps the node id. The coefficients mirror the device parameter.
NXTEST(device_sink_update_and_downmix_coefficients) {
    PoolMapper mapper(0, false);
    DeviceSink d;

    DeviceParameter dp{};
    dp.down_mix_parameter_enabled = true;
    const float first[4] = {0.5f, 0.25f, 0.125f, 1.0f};
    std::memcpy(dp.down_mix_parameter, first, sizeof first);

    BehaviourParameter::ErrorInfo err{};
    SinkOutStatus out{};
    SinkInParameter p = MakeSink(SinkType::Device, true, 3);
    SetSpecific(p, dp);
    d.Update(err, p, out, mapper);
    CHECK(err.error_code == ResultCode::Success);
    CHECK(d.IsUsed);
    CHECK_EQ(d.NodeId, 3);
    CHECK(d.Parameter.down_mix_parameter_enabled);
    CHECK_EQ(d.DownMixCoefficients.size(), 4u);
    CHECK(d.DownMixCoefficients[0] == 0.5f);
    CHECK(d.DownMixCoefficients[1] == 0.25f);
    CHECK(d.DownMixCoefficients[2] == 0.125f);
    CHECK(d.DownMixCoefficients[3] == 1.0f);

    // Same IsUsed: node id kept (9 ignored), enable flag and coefficients replaced.
    DeviceParameter dp2{};
    dp2.down_mix_parameter_enabled = false;
    const float second[4] = {0.1f, 0.2f, 0.3f, 0.4f};
    std::memcpy(dp2.down_mix_parameter, second, sizeof second);
    SinkInParameter p2 = MakeSink(SinkType::Device, true, 9);
    SetSpecific(p2, dp2);
    d.Update(err, p2, out, mapper);
    CHECK_EQ(d.NodeId, 3);
    CHECK(!d.Parameter.down_mix_parameter_enabled);
    CHECK(d.DownMixCoefficients[0] == 0.1f);
    CHECK(d.DownMixCoefficients[3] == 0.4f);

    // Detach: IsUsed false, node id taken from this update.
    SinkInParameter p3 = MakeSink(SinkType::Device, false, 5);
    SetSpecific(p3, dp2);
    d.Update(err, p3, out, mapper);
    CHECK(!d.IsUsed);
    CHECK_EQ(d.NodeId, 5);
}

// C#: DeviceSink.CleanUp releases the upsampler it holds and clears the reference, so the slot can be allocated again.
NXTEST(device_sink_cleanup_releases_upsampler) {
    std::vector<float> work(1440, 0.0f);
    UpsamplerManager mgr(Span<float>(work), 1);
    DeviceSink d;
    d.UpsamplerInfo = mgr.Allocate();
    REQUIRE(d.UpsamplerInfo != nullptr);
    CHECK(mgr.Allocate() == nullptr);

    d.CleanUp();
    CHECK(d.UpsamplerInfo == nullptr);
    CHECK(d.Type == SinkType::Device);
    CHECK(!d.IsUsed);
    CHECK(mgr.Allocate() != nullptr);
}
