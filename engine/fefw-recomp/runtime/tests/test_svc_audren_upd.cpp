// SPDX-License-Identifier: GPL-3.0-or-later
// Server/StateUpdater port (Ryujinx.Audio/Renderer/Server/StateUpdater.cs and the step order of AudioRenderSystem.Update).
//
// Every expected value below is derived by hand from the C# sources under refs/ryubing/src/Ryujinx.Audio/Renderer/Server
// (StateUpdater.cs, AudioRenderSystem.cs, Voice/VoiceInfo.cs, Effect/ReverbEffect.cs, Effect/BaseEffect.cs, Sink/DeviceSink.cs,
// Mix/MixInfo.cs). The derivation is in the comments next to each check. No value is copied from running the port.
//
// Test renderer ("REV10"): BehaviourInfo user revision = BaseRevisionMagic + Revision10 = 'R' 'E' 'V' '0'+10 = 0x3A564552.
//   * Revision10 < Revision15, so voices use VoiceInParameter1 (0x170 bytes) and UpdateVoices1 runs.
//   * Revision9 <= Revision10, so effects use version 2 (0xC0 in, 0x90 out).
//   * Revision7 <= Revision10, so the mix update is the dirty-only form (0x20-byte header, then mix_count MixParameters).
//   * Revision2 <= Revision10, so the splitter is supported (UpdateSplitter runs, size 0, no splitter data).
//   * Revision5 <= Revision10, so the elapsed frame count is supplied (UpdateRendererInfo runs).
//
// Input layout (C# call order, sizes from the C# structs; all offsets are relative to the start of the input):
//   0x000  UpdateDataHeader            0x40   (total_size declared = 0x18E0)
//   0x040  BehaviourParameter          0x10
//   0x050  MemoryPoolInParameter x2    0x40   (pool 0 request-attach, pool 1 invalid = no change)
//   0x090  VoiceChannelResourceInParameter x2  0xE0
//   0x170  VoiceInParameter1 x2        0x2E0  (voice 0 new + in use, voice 1 unused)
//   0x450  EffectInParameterVersion2 x1 0xC0  (reverb)
//   0x510  MixInParameterDirtyOnlyUpdate 0x20 (mix_count 2), then MixParameter x2  0x1260
//   0x1790 SinkInParameter x1          0x140  (device)
//   0x18D0 PerformanceInParameter x1   0x10
//   total  0x18E0 = 6368 bytes. No splitter data (size 0), no input for error info or renderer info (they only write output).
//
// Output layout (each step appends to the output cursor, the header is the first 0x40 bytes):
//   0x000 UpdateDataHeader 0x40 | 0x040 MemoryPoolOutStatus x2 0x20 | 0x060 VoiceOutStatus x2 0x20 | 0x080 EffectOutStatusVersion2 0x90
//   0x110 SinkOutStatus 0x20 | 0x130 PerformanceOutStatus 0x10 | 0x140 BehaviourErrorInfoOutStatus 0xB0 | 0x1F0 RendererInfoOutStatus 0x10
//   total 0x200. The header holds revision 0x3A564552, behaviour_size 0xB0, memory_pools_size 0x20, voices_size 0x20,
//   voice_resources_size 0 (not written by C#), effects_size 0x90, mixes_size 0 (not written by C#), sinks_size 0x20,
//   performance_buffer_size 0x10, splitter_size 0 (not written by C#), render_info_size 0x10, total_size 0x200.
#include <algorithm>
#include <cstring>
#include <vector>

#include "../src/services/audren/Server/BehaviourInfo.h"
#include "../src/services/audren/Server/Effect/EffectContext.h"
#include "../src/services/audren/Server/MemoryPool/PoolMapper.h"
#include "../src/services/audren/Server/Mix/MixContext.h"
#include "../src/services/audren/Server/Mix/MixInfo.h"
#include "../src/services/audren/Server/Sink/SinkContext.h"
#include "../src/services/audren/Server/Splitter/SplitterContext.h"
#include "../src/services/audren/Server/StateUpdater.h"
#include "../src/services/audren/Server/Voice/VoiceContext.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_upd)

namespace {

using namespace nxrt::audren;

// Guest memory. Page aligned, because PoolMapper requires 0x1000-aligned cpu addresses for memory pools.
alignas(0x1000) uint8_t gGuest[0x3000];

const uint32_t kProcessHandle = 0x1234;
const int32_t kRevision = BehaviourInfo::BaseRevisionMagic + BehaviourInfo::Revision10;  // 0x3A564552

uint64_t GuestBase() { return static_cast<uint64_t>(reinterpret_cast<uintptr_t>(gGuest)); }

template <class T>
void Append(std::vector<uint8_t>& buffer, const T& value) {
    size_t offset = buffer.size();
    buffer.resize(offset + sizeof(T));
    std::memcpy(buffer.data() + offset, &value, sizeof(T));
}

// Read a struct at a byte offset of the output buffer (offsets used here are multiples of 0x10).
template <class T>
T* At(std::vector<uint8_t>& buffer, size_t offset) {
    return reinterpret_cast<T*>(buffer.data() + offset);
}

// The UpdateDataHeader the test sends. Section sizes are the C# sizes of the sections described in the header comment.
UpdateDataHeader DefaultHeader() {
    UpdateDataHeader header{};
    header.revision = kRevision;
    header.behaviour_size = static_cast<uint32_t>(sizeof(BehaviourParameter));                                          // 0x10
    header.memory_pools_size = static_cast<uint32_t>(2 * sizeof(MemoryPoolInParameter));                                 // 0x40
    header.voices_size = static_cast<uint32_t>(2 * sizeof(VoiceInParameter1));                                           // 0x2E0
    header.voice_resources_size = static_cast<uint32_t>(2 * sizeof(VoiceChannelResourceInParameter));                    // 0xE0
    header.effects_size = static_cast<uint32_t>(1 * sizeof(EffectInParameterVersion2));                                 // 0xC0
    header.splitter_size = 0;
    header.mixes_size = static_cast<uint32_t>(sizeof(MixInParameterDirtyOnlyUpdate) + 2 * sizeof(MixParameter));        // 0x1280
    header.sinks_size = static_cast<uint32_t>(1 * sizeof(SinkInParameter));                                              // 0x140
    header.performance_buffer_size = static_cast<uint32_t>(sizeof(PerformanceInParameter));                              // 0x10
    header.render_info_size = 0;
    header.total_size = 0x18E0;  // 0x40 + 0x10 + 0x40 + 0xE0 + 0x2E0 + 0xC0 + 0x1280 + 0x140 + 0x10
    return header;
}

// Build the full input buffer: the header given, then the sections in C# call order.
std::vector<uint8_t> BuildInput(const UpdateDataHeader& header, int32_t behaviourRevision) {
    std::vector<uint8_t> input;

    Append(input, header);

    BehaviourParameter behaviour{};
    behaviour.user_revision = behaviourRevision;
    behaviour.flags = 0;
    Append(input, behaviour);

    // Memory pools: pool 0 asks for an attach of 0x2000 bytes at the start of gGuest. Pool 1 is Invalid, so C# leaves it alone.
    MemoryPoolInParameter pool0{};
    pool0.cpu_address = GuestBase();
    pool0.size = 0x2000;
    pool0.state = MemoryPoolUserState::RequestAttach;
    Append(input, pool0);
    MemoryPoolInParameter pool1{};
    pool1.state = MemoryPoolUserState::Invalid;
    Append(input, pool1);

    // Voice channel resources: id 0 and id 1, both used. Mix[0] is 1.0 for id 0 and 0.5 for id 1.
    for (uint32_t id = 0; id < 2; id++) {
        VoiceChannelResourceInParameter resource{};
        resource.id = id;
        resource.mix[0] = (id == 0) ? 1.0f : 0.5f;
        resource.is_used = true;
        Append(input, resource);
    }

    // Voice 0: new, in use, Start, PCM16, one channel on resource 0, wave buffer 0 in pool 0 (0x1000 bytes at +0x1000),
    // samples 0..0x100 (0x200 bytes, inside the buffer). Wave buffers 1..3 are marked sent, so C# does not touch them.
    // Voice 1: not in use, all zero.
    VoiceInParameter1 voice0{};
    voice0.id = 0;
    voice0.node_id = 5;
    voice0.is_new = true;
    voice0.in_use = true;
    voice0.play_state = PlayState::Start;
    voice0.sample_format = SampleFormat::PcmInt16;
    voice0.sample_rate = 48000;
    voice0.priority = 8;
    voice0.sorting_order = 1;
    voice0.channel_count = 1;
    voice0.pitch = 1.0f;
    voice0.volume = 0.75f;
    voice0.wave_buffers_count = 1;
    voice0.wave_buffers_index = 0;
    voice0.data_source_state_address = 0;  // no data source state: ShouldUpdateParameters1 is false, so nothing is attached
    voice0.data_source_state_size = 0;
    voice0.mix_id = 0;
    voice0.splitter_id = Constants::UnusedSplitterId;
    voice0.wave_buffers[0].address = GuestBase() + 0x1000;
    voice0.wave_buffers[0].size = 0x1000;
    voice0.wave_buffers[0].start_sample_offset = 0;
    voice0.wave_buffers[0].end_sample_offset = 0x100;
    voice0.wave_buffers[0].sent_to_server = false;
    for (int k = 1; k < 4; k++) {
        voice0.wave_buffers[k].sent_to_server = true;
    }
    voice0.channel_resource_ids[0] = 0;
    voice0.decoding_behaviour_flags = DecodingBehaviour::Default;
    voice0.src_quality = SampleRateConversionQuality::Default;
    Append(input, voice0);
    VoiceInParameter1 voice1{};
    Append(input, voice1);

    // Effect 0: reverb, new, enabled, target mix 0, work buffer at the start of pool 0 (0x1000 bytes).
    // The reverb specific data is a ReverbParameter at offset 0 with 2 channels (valid) and sample rate 48000.
    EffectInParameterVersion2 effect{};
    effect.type = EffectType::Reverb;
    effect.is_new = true;
    effect.is_enabled = true;
    effect.mix_id = 0;
    effect.buffer_base = GuestBase();
    effect.buffer_size = 0x1000;
    effect.processing_order = 0;
    ReverbParameter reverb{};
    reverb.channel_count_max = 2;
    reverb.channel_count = 2;
    reverb.sample_rate = 48000;
    std::memcpy(effect.specific_data, &reverb, sizeof(reverb));
    Append(input, effect);

    // Mixes (dirty-only form): mix 0 is the final mix (no destination), mix 1 sends to mix 0. Each has 2 buffers.
    MixInParameterDirtyOnlyUpdate dirty{};
    dirty.magic = 0;
    dirty.mix_count = 2;
    Append(input, dirty);

    MixParameter mix0{};
    mix0.volume = 1.0f;
    mix0.sample_rate = 48000;
    mix0.buffer_count = 2;
    mix0.is_used = true;
    mix0.mix_id = 0;
    mix0.effect_count = 1;
    mix0.node_id = 0;
    mix0.mix_buffer_volume[0] = 1.0f;
    mix0.destination_mix_id = Constants::UnusedMixId;
    mix0.destination_splitter_id = Constants::UnusedSplitterId;
    Append(input, mix0);

    MixParameter mix1{};
    mix1.volume = 0.5f;
    mix1.sample_rate = 48000;
    mix1.buffer_count = 2;
    mix1.is_used = true;
    mix1.mix_id = 1;
    mix1.effect_count = 0;
    mix1.node_id = 1;
    mix1.destination_mix_id = 0;
    mix1.destination_splitter_id = Constants::UnusedSplitterId;
    Append(input, mix1);

    // Sink 0: device, used, node 7. Specific data is zero (no down-mix).
    SinkInParameter sink{};
    sink.type = SinkType::Device;
    sink.is_used = true;
    sink.magic = 0xCAFE;
    sink.node_id = 7;
    Append(input, sink);

    // Performance: no manager is attached, so the history is 0. The 0x10-byte section must still be present (C# checks its size).
    PerformanceInParameter performance{};
    performance.target_node_id = 0;
    Append(input, performance);

    return input;
}

// All the server objects of the test renderer, built as AudioRenderSystem builds them. Heap storage keeps the spans valid.
struct RendererState {
    BehaviourInfo behaviour;
    std::vector<MemoryPoolInfo> pools;
    std::vector<int32_t> sortedVoices;
    std::vector<VoiceInfo> voices;
    std::vector<VoiceChannelResource> resources;
    std::vector<VoiceState> voiceStatesCpu;
    std::vector<VoiceState> voiceStatesDsp;
    VoiceContext voiceContext;
    EffectContext effectContext;
    SinkContext sinkContext;
    SplitterContext splitterContext;
    std::vector<int32_t> sortedMixes;
    std::vector<MixInfo> mixes;
    std::vector<int32_t> effectOrder0;
    std::vector<int32_t> effectOrder1;
    std::vector<uint8_t> nodeStatesWork;
    std::vector<uint8_t> edgeMatrixWork;
    MixContext mixContext;

    RendererState() {
        behaviour.SetUserRevision(kRevision);

        pools.assign(2, MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu));

        sortedVoices.assign(2, 0);
        voices.resize(2);
        resources.resize(2);
        voiceStatesCpu.resize(2);
        voiceStatesDsp.resize(2);
        voiceContext.Initialize(Span<int32_t>(sortedVoices), Span<VoiceInfo>(voices), Span<VoiceChannelResource>(resources),
                                Span<VoiceState>(voiceStatesCpu), Span<VoiceState>(voiceStatesDsp), 2);

        effectContext.Initialize(1, 1);
        sinkContext.Initialize(1);

        effectOrder0.assign(4, -1);
        effectOrder1.assign(4, -1);
        mixes.resize(2);
        mixes[0] = MixInfo(Span<int32_t>(effectOrder0), behaviour);
        mixes[1] = MixInfo(Span<int32_t>(effectOrder1), behaviour);
        sortedMixes.assign(2, 0);
        nodeStatesWork.assign(0x1000, 0);
        edgeMatrixWork.assign(0x1000, 0);
        mixContext.Initialize(Span<int32_t>(sortedMixes), Span<MixInfo>(mixes), Span<uint8_t>(nodeStatesWork),
                              Span<uint8_t>(edgeMatrixWork));
    }
};

// Run every step of AudioRenderSystem.Update in its order, stopping at the first failure. Returns the first non-success result.
ResultCode RunAll(RendererState& state, const std::vector<uint8_t>& input, std::vector<uint8_t>& output) {
    std::fill(output.begin(), output.end(), 0);  // AudioRenderSystem.Update clears the output first.

    StateUpdater updater(Span<const uint8_t>(input), Span<uint8_t>(output), kProcessHandle, &state.behaviour);

    ResultCode result = updater.UpdateBehaviourInfo();
    if (result != ResultCode::Success) return result;

    result = updater.UpdateMemoryPools(Span<MemoryPoolInfo>(state.pools));
    if (result != ResultCode::Success) return result;

    result = updater.UpdateVoiceChannelResources(state.voiceContext);
    if (result != ResultCode::Success) return result;

    PoolMapper mapper(kProcessHandle, Span<MemoryPoolInfo>(state.pools), state.behaviour.IsMemoryPoolForceMappingEnabled());

    result = updater.UpdateVoices1(state.voiceContext, mapper);
    if (result != ResultCode::Success) return result;

    result = updater.UpdateEffects(state.effectContext, true, mapper);
    if (result != ResultCode::Success) return result;

    if (state.behaviour.IsSplitterSupported()) {
        result = updater.UpdateSplitter(state.splitterContext);
        if (result != ResultCode::Success) return result;
    }

    result = updater.UpdateMixes(state.mixContext, 6, state.effectContext, state.splitterContext);
    if (result != ResultCode::Success) return result;

    result = updater.UpdateSinks(state.sinkContext, mapper);
    if (result != ResultCode::Success) return result;

    result = updater.UpdatePerformanceBuffer(nullptr, Span<uint8_t>());
    if (result != ResultCode::Success) return result;

    result = updater.UpdateErrorInfo();
    if (result != ResultCode::Success) return result;

    if (state.behaviour.IsElapsedFrameCountSupported()) {
        result = updater.UpdateRendererInfo(42);
        if (result != ResultCode::Success) return result;
    }

    return updater.CheckConsumedSize();
}

}  // namespace

// Positive path, step by step, checking the return code of each step and the state each step leaves behind.
NXTEST(full_update_sequence_matches_csharp) {
    RendererState state;
    std::vector<uint8_t> input = BuildInput(DefaultHeader(), kRevision);
    std::vector<uint8_t> output(0x400, 0);

    std::fill(output.begin(), output.end(), 0);
    StateUpdater updater(Span<const uint8_t>(input), Span<uint8_t>(output), kProcessHandle, &state.behaviour);

    // Constructor: the input header is read (0x40), and the output header gets revision and total size (C# UpdateDataHeader.Initialize).
    {
        UpdateDataHeader* header = At<UpdateDataHeader>(output, 0);
        // C#: Revision = _behaviourInfo.UserRevision; TotalSize = sizeof(UpdateDataHeader) = 0x40.
        CHECK_EQ(header->revision, kRevision);
        CHECK_EQ(header->total_size, 0x40u);
    }

    // C# UpdateBehaviourInfo: revision valid and equal to the user revision, BehaviourSize 0x10. Returns Success.
    CHECK_EQ(updater.UpdateBehaviourInfo(), ResultCode::Success);

    // C# UpdateMemoryPools: 2 * 0x20 = 0x40 input bytes match the header. Pool 0: RequestAttach, PoolMapper.Update gives Attached
    // (cpu address page aligned, size page aligned, identity mapping, so DspAddress = CpuAddress). Pool 1: Invalid, Success with no
    // change, so its output status stays zero. Output: 2 * 0x10 = 0x20 bytes at 0x40. OutputHeader.MemoryPoolsSize = 0x20.
    CHECK_EQ(updater.UpdateMemoryPools(Span<MemoryPoolInfo>(state.pools)), ResultCode::Success);
    CHECK_EQ(state.pools[0].CpuAddress, GuestBase());
    CHECK_EQ(state.pools[0].Size, 0x2000u);
    CHECK_EQ(state.pools[0].DspAddress, GuestBase());
    CHECK_EQ(At<MemoryPoolOutStatus>(output, 0x40)->state, MemoryPoolUserState::Attached);
    CHECK_EQ(At<MemoryPoolOutStatus>(output, 0x50)->state, MemoryPoolUserState::Invalid);
    CHECK_EQ(At<UpdateDataHeader>(output, 0)->memory_pools_size, 0x20u);
    CHECK_EQ(At<UpdateDataHeader>(output, 0)->total_size, 0x40u + 0x20u);

    // C# UpdateVoiceChannelResources: 2 * 0x70 = 0xE0 matches. Each resource takes Id, the 24 Mix values and IsUsed.
    CHECK_EQ(updater.UpdateVoiceChannelResources(state.voiceContext), ResultCode::Success);
    CHECK_EQ(state.resources[0].Id, 0u);
    CHECK_EQ(state.resources[1].Id, 1u);
    CHECK_EQ(state.resources[0].Mix[0], 1.0f);
    CHECK_EQ(state.resources[1].Mix[0], 0.5f);
    CHECK(state.resources[0].IsUsed);
    CHECK(state.resources[1].IsUsed);

    // C# UpdateVoices1 (Revision10 < Revision15). Input 2 * 0x170 = 0x2E0 matches. Every voice is first set to InUse = false.
    // Voice 0 (in use): UpdateParameters1 copies the fields (Start -> Started, Stopped before because Initialize set it),
    //   ShouldUpdateParameters1 is false (address 0 == 0 and size 0 == 0), so no attach and no error.
    //   UpdateWaveBuffers1 (IsNew): wave buffer 0 is not sent, so its fields are copied and it is attached in pool 0
    //   (0x1000 .. 0x2000 of gGuest, the attach range is inside pool 0): no error. Buffers 1..3 are sent and skipped.
    //   WriteOutStatus1 (IsNew): IsNew = true and the out status is zero (played counts 0, drop flag false).
    // Voice 1 (not in use): nothing is written, its out status stays zero.
    // Output: 2 * 0x10 = 0x20 bytes at 0x60. OutputHeader.VoicesSize = 0x20.
    PoolMapper mapper(kProcessHandle, Span<MemoryPoolInfo>(state.pools), false);
    CHECK_EQ(updater.UpdateVoices1(state.voiceContext, mapper), ResultCode::Success);
    {
        VoiceInfo& voice0 = state.voices[0];
        VoiceInfo& voice1 = state.voices[1];
        CHECK(voice0.InUse);
        CHECK_EQ(voice0.Id, 0);
        CHECK_EQ(voice0.NodeId, 5);
        CHECK_EQ(voice0.MixId, 0);
        CHECK_EQ(voice0.Priority, 8u);
        CHECK_EQ(voice0.SortingOrder, 1u);
        CHECK_EQ(voice0.SampleRate, 48000u);
        CHECK_EQ(voice0.ChannelsCount, 1u);
        CHECK_EQ(voice0.WaveBuffersCount, 1u);
        CHECK_EQ(voice0.ChannelResourceIds[0], 0);
        CHECK(voice0.PlayState == nxrt::audren::server::PlayState::Started);
        CHECK(voice0.IsNew);
        CHECK(!voice0.BufferInfoUnmapped);
        CHECK_EQ(voice0.WaveBuffers[0].StartSampleOffset, 0u);
        CHECK_EQ(voice0.WaveBuffers[0].EndSampleOffset, 0x100u);
        CHECK(!voice0.WaveBuffers[0].IsSendToAudioProcessor);
        CHECK_EQ(voice0.WaveBuffers[0].BufferAddressInfo.CpuAddress, GuestBase() + 0x1000);
        CHECK(!voice1.InUse);
    }
    CHECK_EQ(At<VoiceOutStatus>(output, 0x60)->played_sample_count, 0u);
    CHECK_EQ(At<VoiceOutStatus>(output, 0x60)->played_wave_buffers_count, 0u);
    CHECK(!At<VoiceOutStatus>(output, 0x60)->voice_drop_flag);
    CHECK_EQ(At<VoiceOutStatus>(output, 0x70)->played_sample_count, 0u);
    CHECK_EQ(At<UpdateDataHeader>(output, 0)->voices_size, 0x20u);

    // C# UpdateEffects -> UpdateEffectsVersion2 (Revision9 <= Revision10). Input 1 * 0xC0 matches.
    // Effect 0 slot is a BaseEffect (Invalid), so ResetEffect makes a ReverbEffect. Reverb Update: the parameter has valid channel
    // counts, IsNew, so UsageState = New and the work buffer attaches in pool 0 (no error). StoreStatus with the renderer active:
    // UsageState is not Disabled, so the state is Enabled (3). Output: 0x90 bytes at 0x80. OutputHeader.EffectsSize = 0x90.
    CHECK_EQ(updater.UpdateEffects(state.effectContext, true, mapper), ResultCode::Success);
    {
        BaseEffect& effect = *state.effectContext.GetEffect(0);
        CHECK(effect.Type == EffectType::Reverb);
        CHECK_EQ(effect.MixId, 0);
        CHECK_EQ(effect.ProcessingOrder, 0u);
        CHECK(effect.IsEnabled);
        CHECK(effect.UsageState == nxrt::audren::UsageState::New);
        CHECK(!effect.BufferUnmapped);
    }
    CHECK(At<EffectOutStatusVersion2>(output, 0x80)->state == EffectState::Enabled);
    CHECK_EQ(At<UpdateDataHeader>(output, 0)->effects_size, 0x90u);

    // C# UpdateSplitter (Revision2 <= Revision10): no splitter in use, so SplitterContext.Update returns true without reading.
    // Input consumed moves to start + SplitterSize (0). Nothing is written to the output.
    CHECK_EQ(updater.UpdateSplitter(state.splitterContext), ResultCode::Success);

    // C# UpdateMixes, dirty-only path (Revision7 <= Revision10). Input 0x20 + 2 * 0x930 = 0x1280 matches.
    // Mix 0 (final): is_used differs from the default, so it is set and the processing order is cleared. Its destination is unused.
    // Mix 1: is_used set, destination mix 0. Connection: splitter supported, so UpdateConnection connects 1 -> 0 in the edge matrix.
    // Sorting: the context is dirty and no splitter is in use, so Sort() runs. Output: nothing written; OutputHeader.MixesSize stays 0.
    CHECK_EQ(updater.UpdateMixes(state.mixContext, 6, state.effectContext, state.splitterContext), ResultCode::Success);
    {
        MixInfo& finalMix = state.mixContext.GetState(0);
        MixInfo& sub = state.mixContext.GetState(1);
        CHECK(finalMix.IsUsed);
        CHECK_EQ(finalMix.MixId, 0);
        CHECK_EQ(finalMix.Volume, 1.0f);
        CHECK_EQ(finalMix.BufferCount, 2u);
        CHECK_EQ(finalMix.DestinationMixId, Constants::UnusedMixId);
        CHECK(sub.IsUsed);
        CHECK_EQ(sub.MixId, 1);
        CHECK_EQ(sub.Volume, 0.5f);
        CHECK_EQ(sub.BufferCount, 2u);
        CHECK_EQ(sub.DestinationMixId, 0);
    }
    CHECK_EQ(At<UpdateDataHeader>(output, 0)->mixes_size, 0u);

    // C# UpdateSinks. Input 1 * 0x140 matches. The slot is a BaseSink (Invalid), so ResetSink makes a DeviceSink.
    // DeviceSink.Update: IsUsed differs from the default, so IsUsed = true and NodeId = 7. Errors: none. Output status is zeroed.
    // Output: 0x20 bytes at 0x110. OutputHeader.SinksSize = 0x20.
    CHECK_EQ(updater.UpdateSinks(state.sinkContext, mapper), ResultCode::Success);
    {
        BaseSink& sink = state.sinkContext.GetSink(0);
        CHECK(sink.Type == SinkType::Device);
        CHECK(sink.IsUsed);
        CHECK_EQ(sink.NodeId, 7);
    }
    CHECK_EQ(At<SinkOutStatus>(output, 0x110)->last_written_offset, 0u);
    CHECK_EQ(At<UpdateDataHeader>(output, 0)->sinks_size, 0x20u);

    // C# UpdatePerformanceBuffer with no manager: input 0x10 matches. HistorySize = 0. Output 0x10 bytes at 0x130.
    CHECK_EQ(updater.UpdatePerformanceBuffer(nullptr, Span<uint8_t>()), ResultCode::Success);
    CHECK_EQ(At<PerformanceOutStatus>(output, 0x130)->history_size, 0u);
    CHECK_EQ(At<UpdateDataHeader>(output, 0)->performance_buffer_size, 0x10u);

    // C# UpdateErrorInfo: no error was appended, so the count is 0. Output 0xB0 bytes at 0x140. BehaviourSize = 0xB0.
    CHECK_EQ(updater.UpdateErrorInfo(), ResultCode::Success);
    CHECK_EQ(At<BehaviourErrorInfoOutStatus>(output, 0x140)->error_infos_count, 0u);
    CHECK_EQ(At<UpdateDataHeader>(output, 0)->behaviour_size, 0xB0u);

    // C# UpdateRendererInfo (Revision5 <= Revision10): ElapsedFrameCount = 42. Output 0x10 bytes at 0x1F0.
    CHECK_EQ(updater.UpdateRendererInfo(42), ResultCode::Success);
    CHECK_EQ(At<RendererInfoOutStatus>(output, 0x1F0)->elapsed_frame_count, 42u);
    CHECK_EQ(At<UpdateDataHeader>(output, 0)->render_info_size, 0x10u);

    // C# CheckConsumedSize: input consumed = 0x18E0 = header total_size. Output consumed = 0x200 = output header total_size.
    CHECK_EQ(updater.CheckConsumedSize(), ResultCode::Success);
    CHECK_EQ(At<UpdateDataHeader>(output, 0)->total_size, 0x200u);
    CHECK_EQ(At<UpdateDataHeader>(output, 0)->revision, kRevision);
}

// C# UpdateBehaviourInfo: the revision in BehaviourParameter must be valid and equal to the user revision.
NXTEST(wrong_revision_is_rejected) {
    {
        // Valid revision, but not the one the renderer runs with (REV9 instead of REV10).
        RendererState state;
        std::vector<uint8_t> input = BuildInput(DefaultHeader(), BehaviourInfo::BaseRevisionMagic + BehaviourInfo::Revision9);
        std::vector<uint8_t> output(0x400, 0);
        StateUpdater updater(Span<const uint8_t>(input), Span<uint8_t>(output), kProcessHandle, &state.behaviour);
        CHECK_EQ(updater.UpdateBehaviourInfo(), ResultCode::InvalidUpdateInfo);
    }
    {
        // Revision number 16 is above the last supported revision (15), so CheckValidRevision is false.
        RendererState state;
        std::vector<uint8_t> input = BuildInput(DefaultHeader(), BehaviourInfo::BaseRevisionMagic + (16 << 24));
        std::vector<uint8_t> output(0x400, 0);
        StateUpdater updater(Span<const uint8_t>(input), Span<uint8_t>(output), kProcessHandle, &state.behaviour);
        CHECK_EQ(updater.UpdateBehaviourInfo(), ResultCode::InvalidUpdateInfo);
    }
}

// The section sizes in the header must match the counts of the renderer (memory pools, voices). Each check is a separate run.
NXTEST(section_size_mismatch_is_rejected) {
    {
        // memory_pools_size 0x40 + 0x10 while the renderer has 2 pools (0x40 expected): C# returns InvalidUpdateInfo before reading.
        RendererState state;
        UpdateDataHeader header = DefaultHeader();
        header.memory_pools_size += 0x10;
        std::vector<uint8_t> input = BuildInput(header, kRevision);
        std::vector<uint8_t> output(0x400, 0);
        StateUpdater updater(Span<const uint8_t>(input), Span<uint8_t>(output), kProcessHandle, &state.behaviour);
        CHECK_EQ(updater.UpdateBehaviourInfo(), ResultCode::Success);
        CHECK_EQ(updater.UpdateMemoryPools(Span<MemoryPoolInfo>(state.pools)), ResultCode::InvalidUpdateInfo);
    }
    {
        // voices_size 0x2E0 - 0x10 (two voices of 0x170 expected): C# UpdateVoices1 returns InvalidUpdateInfo before reading.
        RendererState state;
        UpdateDataHeader header = DefaultHeader();
        header.voices_size -= 0x10;
        std::vector<uint8_t> input = BuildInput(header, kRevision);
        std::vector<uint8_t> output(0x400, 0);
        StateUpdater updater(Span<const uint8_t>(input), Span<uint8_t>(output), kProcessHandle, &state.behaviour);
        PoolMapper mapper(kProcessHandle, Span<MemoryPoolInfo>(state.pools), false);
        CHECK_EQ(updater.UpdateVoices1(state.voiceContext, mapper), ResultCode::InvalidUpdateInfo);
        CHECK(!state.voices[0].InUse);
    }
}

// Truncated input. A read that runs past the end of the buffer is NXAUDREN_FATAL (abort), as C# throws, and cannot be tested in
// process. What can be tested: the input carries fewer bytes than the header declares in total. Every section is present and
// parses, so only the consumed-size check sees the difference: the header declares 16 bytes more than the sections deliver.
NXTEST(declared_total_mismatch_fails_consumed_check) {
    RendererState state;
    UpdateDataHeader header = DefaultHeader();
    header.total_size = 0x18E0 + 0x10;
    std::vector<uint8_t> input = BuildInput(header, kRevision);
    std::vector<uint8_t> output(0x400, 0);
    CHECK_EQ(RunAll(state, input, output), ResultCode::InvalidUpdateInfo);
}
