// SPDX-License-Identifier: GPL-3.0-or-later
// Tests for the Server/Voice ports (WaveBuffer, VoiceChannelResource, VoiceContext, VoiceInfo).
//
// Every expected value below is derived by hand from the C# control flow under refs/ryubing/src/Ryujinx.Audio/Renderer/Server/Voice
// (and Renderer/Parameter/VoiceInParameter1.cs for IsSampleOffsetValid). The derivation is noted next to each group. None of the
// expected values were taken from running the port.
//
// Setup shared by the groups: BehaviourInfo at REV5 (magic 0x35564552). The gates used here are IsSplitterSupported (min REV2),
// IsFlushVoiceWaveBuffersSupported (min REV5) and IsDecodingBehaviourFlagSupported (min REV5); all are true at REV5.
// The pool: one CPU pool [0x1000, 0x5000) attached through PoolMapper::Update(RequestAttach); with identity mapping its DspAddress is
// 0x1000 and Contains(addr, size) holds for any region inside [0x1000, 0x5000).
#include <cstring>
#include <vector>

#include "../src/services/audren/Server/Voice/VoiceContext.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_voice)

using namespace nxrt::audren;

namespace {

BehaviourInfo MakeRev5() {
    BehaviourInfo bi;
    bi.SetUserRevision(0x35564552);  // C# BehaviourInfo: BaseRevisionMagic + (5 << 24)
    return bi;
}

// Attach [0x1000, 0x5000) to the single pool (C# PoolMapper.Update with RequestAttach).
void AttachPool(PoolMapper& mapper, std::vector<MemoryPoolInfo>& pools) {
    MemoryPoolInParameter in{};
    in.cpu_address = 0x1000;
    in.size = 0x4000;
    in.state = MemoryPoolUserState::RequestAttach;
    MemoryPoolOutStatus out{};
    (void)mapper.Update(pools[0], in, out);
}

// A wavebuffer request: 0x100 bytes at `address`, last sample `end`, not yet sent to the server.
WaveBufferInternal Pending(uint64_t address, uint32_t end) {
    WaveBufferInternal w{};
    w.address = address;
    w.size = 0x100;
    w.start_sample_offset = 0;
    w.end_sample_offset = end;
    w.sent_to_server = false;
    return w;
}

// A VoiceInParameter1 for one PCM16 mono voice: id 3, node 7, Start, priority 10, sorting order 4, mix 5, 48 kHz.
// Two pending wave buffers (0x1000 and 0x2000, each 0x40 samples). Entries 2 and 3 are marked sent_to_server, so they do not go
// through the mapping branch; that matters because an unsent entry with a zero address would be reported as unmapped (see the
// quirk note in outside_pool_reports_error_and_does_not_mark_buffer_valid).
VoiceInParameter1 MakeParam1() {
    VoiceInParameter1 p{};
    p.id = 3;
    p.node_id = 7;
    p.is_new = true;
    p.in_use = true;
    p.play_state = PlayState::Start;
    p.sample_format = SampleFormat::PcmInt16;
    p.sample_rate = 48000;
    p.priority = 10;
    p.sorting_order = 4;
    p.channel_count = 1;
    p.pitch = 1.0f;
    p.volume = 0.5f;
    p.wave_buffers_count = 2;
    p.wave_buffers_index = 0;
    p.mix_id = 5;
    p.splitter_id = Constants::UnusedSplitterId;
    p.wave_buffers[0] = Pending(0x1000, 0x40);
    p.wave_buffers[1] = Pending(0x2000, 0x40);
    p.wave_buffers[2].sent_to_server = true;
    p.wave_buffers[3].sent_to_server = true;
    p.channel_resource_ids[0] = 0;
    p.src_quality = SampleRateConversionQuality::Default;
    p.decoding_behaviour_flags = DecodingBehaviour::Default;
    return p;
}

// One VoiceState per channel, viewed as the C# Memory<VoiceState>[] (one Span of length 1 per channel).
struct ChannelStates {
    std::vector<VoiceState> states;
    std::vector<Span<VoiceState>> views;

    explicit ChannelStates(size_t channels) : states(channels) {
        for (size_t i = 0; i < channels; i++) {
            views.push_back(Span<VoiceState>(&states[i], 1));
        }
    }

    Span<Span<VoiceState>> Spans() { return Span<Span<VoiceState>>(views.data(), views.size()); }
};

void AcquireWithParam1(VoiceInfo& voice, PoolMapper& mapper, BehaviourInfo& bi, ChannelStates& ch, const VoiceInParameter1& p) {
    BehaviourParameter::ErrorInfo err{};
    voice.UpdateParameters1(err, p, mapper, bi);
    std::vector<BehaviourParameter::ErrorInfo> errors;
    voice.UpdateWaveBuffers1(errors, p, ch.Spans(), mapper, bi);
}

}  // namespace

// Derivation (C# VoiceInfo.UpdateParameters1 + UpdateWaveBuffers1 with IsNew):
//   UpdatePlayState(Start): PreviousPlayState = Stopped (set by Initialize), PlayState = Started.
//   ShouldUpdateParameters1: DataSourceStateAddressInfo.CpuAddress (0) == 0 and Size (0) == 0, so no attach; outErrorInfo empty.
//   UpdateWaveBuffers1 with IsNew: InitializeWaveBuffers, then for each of the 4 entries:
//     entries 0 and 1 are not sent and map inside the pool, so IsSendToAudioProcessor = false and BufferAddressInfo is set;
//     entries 2 and 3 are sent and BufferInfoUnmapped is false, so they are skipped and keep IsSendToAudioProcessor = true.
//   The voice state valid flags are not touched by UpdateWaveBuffers (only command generation sets them).
//   WriteOutStatus1 with IsNew: the first branch, so voice_drop_flag = false and the counts are 0; IsNew becomes true.
NXTEST(acquire_from_voice_in_parameter1) {
    BehaviourInfo bi = MakeRev5();
    std::vector<MemoryPoolInfo> pools(1, MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu));
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);
    AttachPool(mapper, pools);
    CHECK(pools[0].IsMapped());
    CHECK(pools[0].DspAddress == 0x1000);

    VoiceInfo voice;
    voice.Initialize();
    ChannelStates ch(1);
    VoiceInParameter1 p = MakeParam1();

    BehaviourParameter::ErrorInfo err{};
    voice.UpdateParameters1(err, p, mapper, bi);
    CHECK(err.error_code == ResultCode::Success);

    CHECK(voice.InUse);
    CHECK(voice.Id == 3);
    CHECK(voice.NodeId == 7);
    CHECK(voice.PlayState == server::PlayState::Started);
    CHECK(voice.PreviousPlayState == server::PlayState::Stopped);
    CHECK(voice.SampleFormat == SampleFormat::PcmInt16);
    CHECK(voice.SampleRate == 48000u);
    CHECK(voice.ChannelsCount == 1u);
    CHECK(voice.Priority == 10u);
    CHECK(voice.SortingOrder == 4u);
    CHECK(voice.Pitch == 1.0f);
    CHECK(voice.Volume == 0.5f);
    CHECK(voice.MixId == 5);
    CHECK(voice.SplitterId == Constants::UnusedSplitterId);
    CHECK(voice.WaveBuffersCount == 2u);
    CHECK(voice.ChannelResourceIds[0] == 0);
    CHECK(voice.DecodingBehaviour == DecodingBehaviour::Default);
    CHECK(!voice.ShouldSkip());

    std::vector<BehaviourParameter::ErrorInfo> errors;
    voice.UpdateWaveBuffers1(errors, p, ch.Spans(), mapper, bi);
    CHECK(errors.empty());
    CHECK(!voice.BufferInfoUnmapped);

    CHECK(!voice.WaveBuffers[0].IsSendToAudioProcessor);
    CHECK(voice.WaveBuffers[0].BufferAddressInfo.CpuAddress == 0x1000u);
    CHECK(voice.WaveBuffers[0].BufferAddressInfo.Size == 0x100u);
    CHECK(voice.WaveBuffers[0].EndSampleOffset == 0x40u);
    CHECK(!voice.WaveBuffers[1].IsSendToAudioProcessor);
    CHECK(voice.WaveBuffers[1].BufferAddressInfo.CpuAddress == 0x2000u);
    CHECK(voice.WaveBuffers[2].IsSendToAudioProcessor);
    CHECK(voice.WaveBuffers[3].IsSendToAudioProcessor);
    CHECK(!ch.states[0].is_wave_buffer_valid[0]);
    CHECK(!ch.states[0].is_wave_buffer_valid[1]);

    VoiceOutStatus status{};
    voice.WriteOutStatus1(status, p, ch.Spans());
    CHECK(voice.IsNew);
    CHECK(!status.voice_drop_flag);
    CHECK(status.played_wave_buffers_count == 0u);
    CHECK(status.played_sample_count == 0u);
}

// Derivation (C# VoiceInfo.UpdateParameters2 + UpdateWaveBuffers2): the same flow as VoiceInParameter1, except that the biquad
// filters are copied (no conversion) and the flush count is added (REV5 gate on). Here the biquad filter 0 numerator[0] = 0.5 must be
// copied unchanged. The mix volumes are not part of the voice parameter; they live in VoiceChannelResource.Mix (see the
// update_for_command_generation_resets_resources test).
NXTEST(acquire_from_voice_in_parameter2) {
    BehaviourInfo bi = MakeRev5();
    std::vector<MemoryPoolInfo> pools(1, MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu));
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);
    AttachPool(mapper, pools);

    VoiceInParameter2 p{};
    p.id = 4;
    p.node_id = 9;
    p.is_new = true;
    p.in_use = true;
    p.play_state = PlayState::Start;
    p.sample_format = SampleFormat::PcmInt16;
    p.sample_rate = 48000;
    p.priority = 20;
    p.sorting_order = 1;
    p.channel_count = 1;
    p.pitch = 1.0f;
    p.volume = 0.25f;
    p.biquad_filters[0].enable = true;
    p.biquad_filters[0].numerator[0] = 0.5f;
    p.wave_buffers_count = 1;
    p.wave_buffers_index = 0;
    p.mix_id = 6;
    p.splitter_id = Constants::UnusedSplitterId;
    p.wave_buffers[0].address = 0x3000;
    p.wave_buffers[0].size = 0x100;
    p.wave_buffers[0].end_sample_offset = 0x40;
    p.wave_buffers[0].sent_to_server = false;
    p.wave_buffers[1].sent_to_server = true;
    p.wave_buffers[2].sent_to_server = true;
    p.wave_buffers[3].sent_to_server = true;
    p.channel_resource_ids[0] = 1;
    p.src_quality = SampleRateConversionQuality::Default;
    p.decoding_behaviour_flags = DecodingBehaviour::Default;

    VoiceInfo voice;
    voice.Initialize();
    ChannelStates ch(1);

    BehaviourParameter::ErrorInfo err{};
    voice.UpdateParameters2(err, p, mapper, bi);
    CHECK(err.error_code == ResultCode::Success);
    CHECK(voice.PlayState == server::PlayState::Started);
    CHECK(voice.Id == 4);
    CHECK(voice.NodeId == 9);
    CHECK(voice.Priority == 20u);
    CHECK(voice.SortingOrder == 1u);
    CHECK(voice.MixId == 6);
    CHECK(voice.BiquadFilters[0].enable);
    CHECK(voice.BiquadFilters[0].numerator[0] == 0.5f);
    CHECK(voice.ChannelResourceIds[0] == 1);
    CHECK(voice.WaveBuffersCount == 1u);

    std::vector<BehaviourParameter::ErrorInfo> errors;
    voice.UpdateWaveBuffers2(errors, p, ch.Spans(), mapper, bi);
    CHECK(errors.empty());
    CHECK(!voice.WaveBuffers[0].IsSendToAudioProcessor);
    CHECK(voice.WaveBuffers[0].BufferAddressInfo.CpuAddress == 0x3000u);
    CHECK(voice.WaveBuffers[0].EndSampleOffset == 0x40u);
    CHECK(voice.WaveBuffers[1].IsSendToAudioProcessor);
    CHECK(!voice.ShouldSkip());
}

// Derivation:
//   Data source: the parameter gives DataSourceStateAddress 0x8000 while the voice has 0. ShouldUpdateParameters1 is true, and
//   TryAttachBuffer(0x8000, 0x100): no pool contains it, so AssignDspAddress is false, the error is InvalidAddressInfo with
//   extra = 0x8000, and the return is _isForceMapEnabled = false. So DataSourceStateUnmapped = true, outErrorInfo = (0x8000), and
//   ShouldSkip is true.
//   Wave buffer 0 at 0x8000 (sent = false): IsSampleOffsetValid(PCM16, end 0x40, size 0x100) is true, IsSendToAudioProcessor
//   becomes false, TryAttachBuffer fails and appends one error (0x8000), BufferInfoUnmapped = true.
//   Entry 1 (valid, pending) is then entered because BufferInfoUnmapped is set: it maps, so BufferInfoUnmapped = false again.
//   Quirk: the final BufferInfoUnmapped reflects the LAST entered buffer, not the first failure. The asserts below follow this.
//   The valid flag of voice state 0 is never set by UpdateWaveBuffers, so it stays false.
NXTEST(outside_pool_reports_error_and_does_not_mark_buffer_valid) {
    BehaviourInfo bi = MakeRev5();
    std::vector<MemoryPoolInfo> pools(1, MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu));
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);
    AttachPool(mapper, pools);

    {
        VoiceInfo voice;
        voice.Initialize();
        VoiceInParameter1 p = MakeParam1();
        p.data_source_state_address = 0x8000;
        p.data_source_state_size = 0x100;

        BehaviourParameter::ErrorInfo err{};
        voice.UpdateParameters1(err, p, mapper, bi);
        CHECK(err.error_code == ResultCode::InvalidAddressInfo);
        CHECK(err.extra_error_info == 0x8000u);
        CHECK(voice.DataSourceStateUnmapped);
        CHECK(voice.ShouldSkip());
    }

    {
        VoiceInfo voice;
        voice.Initialize();
        ChannelStates ch(1);
        VoiceInParameter1 p = MakeParam1();
        p.wave_buffers[0].address = 0x8000;  // outside the pool
        p.wave_buffers[0].end_sample_offset = 0x40;

        BehaviourParameter::ErrorInfo err{};
        voice.UpdateParameters1(err, p, mapper, bi);
        CHECK(err.error_code == ResultCode::Success);

        std::vector<BehaviourParameter::ErrorInfo> errors;
        voice.UpdateWaveBuffers1(errors, p, ch.Spans(), mapper, bi);
        CHECK(errors.size() == 1u);
        CHECK(errors[0].error_code == ResultCode::InvalidAddressInfo);
        CHECK(errors[0].extra_error_info == 0x8000u);
        CHECK(!voice.WaveBuffers[0].IsSendToAudioProcessor);
        CHECK(!voice.WaveBuffers[0].BufferAddressInfo.HasMappedMemoryPool());
        CHECK(!ch.states[0].is_wave_buffer_valid[0]);
        CHECK(!voice.BufferInfoUnmapped);
        CHECK(voice.WaveBuffers[1].BufferAddressInfo.CpuAddress == 0x2000u);
    }
}

// Derivation of the PlayState transitions (C# UpdatePlayState and UpdateParametersForCommandGeneration):
//   Stop from Stopped: PreviousPlayState is still set to Stopped, then return; PlayState stays Stopped.
//   Pause: PlayState = Paused, PreviousPlayState = old state. The Paused branch returns WasPlaying = (Previous == Started).
//   Paused and Stopped only mark the buffers as used; they do not touch the voice state valid flags.
//   Stopping (from Started): every buffer is released in order (see below), PlayState = Stopped, WasPlaying = (Previous == Started).
//   ShouldSkip ignores PlayState; it depends only on InUse, WaveBuffersCount, the unmapped flags and VoiceDropFlag.
NXTEST(play_state_transitions_and_was_playing) {
    BehaviourInfo bi = MakeRev5();
    std::vector<MemoryPoolInfo> pools(1, MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu));
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);
    AttachPool(mapper, pools);

    VoiceInfo voice;
    voice.Initialize();
    ChannelStates ch(1);
    AcquireWithParam1(voice, mapper, bi, ch, MakeParam1());

    // Started: buffers 0 and 1 become valid for channel 0; returns true; WasPlaying false.
    CHECK(voice.UpdateParametersForCommandGeneration(ch.Spans()));
    CHECK(!voice.WasPlaying);
    CHECK(ch.states[0].is_wave_buffer_valid[0]);
    CHECK(ch.states[0].is_wave_buffer_valid[1]);
    CHECK(voice.WaveBuffers[0].IsSendToAudioProcessor);
    CHECK(voice.WaveBuffers[1].IsSendToAudioProcessor);

    // Pause from Started: PreviousPlayState = Started, so WasPlaying = true; valid flags are kept.
    voice.UpdatePlayState(::nxrt::audren::PlayState::Pause);
    CHECK(voice.PlayState == server::PlayState::Paused);
    CHECK(voice.PreviousPlayState == server::PlayState::Started);
    CHECK(voice.UpdateParametersForCommandGeneration(ch.Spans()));
    CHECK(voice.WasPlaying);
    CHECK(ch.states[0].is_wave_buffer_valid[0]);
    CHECK(!voice.ShouldSkip());

    // Start from Paused: Started again, WasPlaying false (the Started branch sets it to false).
    voice.UpdatePlayState(::nxrt::audren::PlayState::Start);
    CHECK(voice.PlayState == server::PlayState::Started);
    CHECK(voice.PreviousPlayState == server::PlayState::Paused);
    CHECK(voice.UpdateParametersForCommandGeneration(ch.Spans()));
    CHECK(!voice.WasPlaying);

    // Stop from Started: PlayState = Stopping. Command generation releases both valid buffers in order: index 0 -> 1 -> 2,
    // consumed 0 -> 1 -> 2. Then the offset, played count, pitch and fraction are reset, PlayState = Stopped, WasPlaying = true.
    voice.UpdatePlayState(::nxrt::audren::PlayState::Stop);
    CHECK(voice.PlayState == server::PlayState::Stopping);
    CHECK(voice.PreviousPlayState == server::PlayState::Started);
    CHECK(voice.UpdateParametersForCommandGeneration(ch.Spans()));
    CHECK(voice.PlayState == server::PlayState::Stopped);
    CHECK(voice.WasPlaying);
    CHECK(!ch.states[0].is_wave_buffer_valid[0]);
    CHECK(!ch.states[0].is_wave_buffer_valid[1]);
    CHECK(ch.states[0].wave_buffer_index == 2u);
    CHECK(ch.states[0].wave_buffer_consumed == 2u);
    CHECK(ch.states[0].played_sample_count == 0u);
    CHECK(ch.states[0].offset == 0);

    // Stop from Stopped: returns early, PreviousPlayState is recorded as Stopped, PlayState stays Stopped.
    voice.UpdatePlayState(::nxrt::audren::PlayState::Stop);
    CHECK(voice.PlayState == server::PlayState::Stopped);
    CHECK(voice.PreviousPlayState == server::PlayState::Stopped);
    CHECK(!voice.UpdateParametersForCommandGeneration(ch.Spans()));
    CHECK(!voice.WasPlaying);

    // Pause from Stopped: WasPlaying = (Previous Stopped == Started) = false.
    voice.UpdatePlayState(::nxrt::audren::PlayState::Pause);
    CHECK(voice.PlayState == server::PlayState::Paused);
    CHECK(!voice.UpdateParametersForCommandGeneration(ch.Spans()));
    CHECK(!voice.WasPlaying);
}

// Derivation (C# FlushWaveBuffers, called from UpdateParametersForCommandGeneration, and the second UpdateWaveBuffers call):
//   Start 1: acquire (buffers 0 and 1 pending). Command generation makes valid[0] and valid[1] true.
//   Update 2 (not new): buffer 0 is re-queued with address 0x3000 (sent = false). FlushWaveBufferCount becomes 0 + 1 = 1.
//     UpdateWaveBuffers: isValid(0) is true, so no force unmap. The branch is entered, IsSendToAudioProcessor = false, the new
//     address 0x3000 maps inside the pool.
//   Command generation 2: FlushWaveBuffers(1) from WaveBuffersIndex 0 marks entry 0 as played (valid[0] was true): index 0 -> 1,
//     consumed 0 -> 1, valid[0] = false, IsSendToAudioProcessor = true again. Flush clears FlushWaveBufferCount. The Started loop sees
//     every buffer already sent, so nothing new is validated. Primary valid[1] is still true, so it returns true.
//   WriteOutStatus with the not-new parameter and IsNew = false: played_wave_buffers_count = 1, played_sample_count = 0.
NXTEST(wave_buffer_consumption_across_update_wave_buffers_calls) {
    BehaviourInfo bi = MakeRev5();
    std::vector<MemoryPoolInfo> pools(1, MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu));
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);
    AttachPool(mapper, pools);

    VoiceInfo voice;
    voice.Initialize();
    ChannelStates ch(1);
    AcquireWithParam1(voice, mapper, bi, ch, MakeParam1());
    CHECK(voice.UpdateParametersForCommandGeneration(ch.Spans()));
    CHECK(ch.states[0].is_wave_buffer_valid[0]);
    CHECK(ch.states[0].is_wave_buffer_valid[1]);

    VoiceInParameter1 p2 = MakeParam1();
    p2.is_new = false;
    p2.flush_wave_buffer_count = 1;
    p2.wave_buffers[0] = Pending(0x3000, 0x40);
    p2.wave_buffers[1].sent_to_server = true;

    BehaviourParameter::ErrorInfo err{};
    voice.UpdateParameters1(err, p2, mapper, bi);
    CHECK(voice.FlushWaveBufferCount == 1);

    std::vector<BehaviourParameter::ErrorInfo> errors;
    voice.UpdateWaveBuffers1(errors, p2, ch.Spans(), mapper, bi);
    CHECK(errors.empty());
    CHECK(!voice.WaveBuffers[0].IsSendToAudioProcessor);
    CHECK(voice.WaveBuffers[0].BufferAddressInfo.CpuAddress == 0x3000u);

    CHECK(voice.UpdateParametersForCommandGeneration(ch.Spans()));
    CHECK(voice.FlushWaveBufferCount == 0);
    CHECK(ch.states[0].wave_buffer_index == 1u);
    CHECK(ch.states[0].wave_buffer_consumed == 1u);
    CHECK(!ch.states[0].is_wave_buffer_valid[0]);
    CHECK(ch.states[0].is_wave_buffer_valid[1]);
    CHECK(!voice.WasPlaying);

    VoiceOutStatus status{};
    voice.WriteOutStatus1(status, p2, ch.Spans());
    CHECK(!status.voice_drop_flag);
    CHECK(status.played_wave_buffers_count == 1u);
    CHECK(status.played_sample_count == 0u);
}

// Derivation (C# VoiceInfo.ShouldSkip and WriteOutStatus):
//   ShouldSkip = !InUse || WaveBuffersCount == 0 || DataSourceStateUnmapped || BufferInfoUnmapped || VoiceDropFlag.
//   The VoiceDropFlag is read back by WriteOutStatus (not new) as voice_drop_flag.
//   ResetVoiceDropFlag in the parameter clears it.
NXTEST(voice_drop_flag_and_skip_conditions) {
    BehaviourInfo bi = MakeRev5();
    std::vector<MemoryPoolInfo> pools(1, MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu));
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);
    AttachPool(mapper, pools);

    VoiceInfo voice;
    voice.Initialize();
    ChannelStates ch(1);
    VoiceInParameter1 p = MakeParam1();
    p.is_new = false;
    AcquireWithParam1(voice, mapper, bi, ch, p);
    CHECK(!voice.ShouldSkip());

    voice.VoiceDropFlag = true;
    CHECK(voice.ShouldSkip());
    VoiceOutStatus status{};
    voice.WriteOutStatus1(status, p, ch.Spans());
    CHECK(status.voice_drop_flag);

    BehaviourParameter::ErrorInfo err{};
    p.reset_voice_drop_flag = true;
    voice.UpdateParameters1(err, p, mapper, bi);
    CHECK(!voice.VoiceDropFlag);
    CHECK(!voice.ShouldSkip());

    p.in_use = false;
    voice.UpdateParameters1(err, p, mapper, bi);
    CHECK(voice.ShouldSkip());
}

// Derivation (C# VoiceContext.Sort): indices 0..3 are sorted by Priority, then by SortingOrder.
//   voice 0: priority 5, sorting 0; voice 1: priority 2, sorting 3; voice 2: priority 2, sorting 1; voice 3: priority 255, sorting 0.
//   Priority 2 first (voice 2 before voice 1 because 1 < 3), then priority 5 (voice 0), then 255 (voice 3): sorted = [2, 1, 0, 3].
NXTEST(voice_context_sorts_by_priority_then_sorting_order) {
    std::vector<int32_t> sortedStorage(4, -1);
    std::vector<VoiceInfo> voices(4);
    std::vector<VoiceChannelResource> resources(4);
    std::vector<VoiceState> cpu(4);
    std::vector<VoiceState> dsp(4);

    voices[0].Priority = 5;
    voices[0].SortingOrder = 0;
    voices[1].Priority = 2;
    voices[1].SortingOrder = 3;
    voices[2].Priority = 2;
    voices[2].SortingOrder = 1;
    voices[3].Priority = 255;
    voices[3].SortingOrder = 0;

    VoiceContext ctx;
    ctx.Initialize(Span<int32_t>(sortedStorage), Span<VoiceInfo>(voices), Span<VoiceChannelResource>(resources), Span<VoiceState>(cpu),
                   Span<VoiceState>(dsp), 4);
    CHECK(ctx.GetCount() == 4u);

    ctx.Sort();
    CHECK(sortedStorage[0] == 2);
    CHECK(sortedStorage[1] == 1);
    CHECK(sortedStorage[2] == 0);
    CHECK(sortedStorage[3] == 3);
    CHECK(&ctx.GetSortedState(0) == &voices[2]);
    CHECK(&ctx.GetSortedState(3) == &voices[3]);
}

// Derivation (C# VoiceChannelResource): a default instance is all zero (Mix, PreviousMix, Id, IsUsed) and has Size 0xD0 (static_assert).
// UpdateState copies Mix into PreviousMix (C# Mix.AsSpan().CopyTo(PreviousMix.AsSpan())).
NXTEST(channel_resource_defaults_and_update_state) {
    VoiceChannelResource r;
    CHECK(r.Id == 0u);
    CHECK(!r.IsUsed);
    CHECK(r.Mix[0] == 0.0f);
    CHECK(r.Mix[23] == 0.0f);
    CHECK(r.PreviousMix[0] == 0.0f);
    CHECK(sizeof(VoiceChannelResource) == 0xD0);

    r.Mix[0] = 1.5f;
    r.Mix[23] = 2.0f;
    r.UpdateState();
    CHECK(r.PreviousMix[0] == 1.5f);
    CHECK(r.PreviousMix[23] == 2.0f);
    CHECK(r.Mix[0] == 1.5f);
}

// Derivation (C# VoiceInfo.UpdateForCommandGeneration with IsNew): ResetResources clears the DSP-side VoiceState of each channel
// resource (MemoryMarshal.Cast + Clear), then calls VoiceChannelResource.UpdateState (Mix -> PreviousMix). Then PreviousVolume = Volume
// and IsNew = false. Expected: dsp[2] cleared to 0, PreviousMix[0] = 0.25, PreviousVolume = 0.75, IsNew = false.
NXTEST(update_for_command_generation_resets_resources) {
    std::vector<int32_t> sortedStorage(3, 0);
    std::vector<VoiceInfo> voices(3);
    std::vector<VoiceChannelResource> resources(3);
    std::vector<VoiceState> cpu(3);
    std::vector<VoiceState> dsp(3);

    VoiceContext ctx;
    ctx.Initialize(Span<int32_t>(sortedStorage), Span<VoiceInfo>(voices), Span<VoiceChannelResource>(resources), Span<VoiceState>(cpu),
                   Span<VoiceState>(dsp), 3);

    VoiceInfo& voice = voices[0];
    voice.Initialize();
    voice.IsNew = true;
    voice.ChannelsCount = 1;
    voice.ChannelResourceIds[0] = 2;
    voice.Volume = 0.75f;
    voice.PreviousVolume = 0.0f;
    voice.InUse = true;

    resources[2].IsUsed = true;
    resources[2].Mix[0] = 0.25f;
    dsp[2].played_sample_count = 99;

    voice.UpdateForCommandGeneration(ctx);

    CHECK(dsp[2].played_sample_count == 0u);
    CHECK(resources[2].PreviousMix[0] == 0.25f);
    CHECK(voice.PreviousVolume == 0.75f);
    CHECK(!voice.IsNew);
}

// Derivation (C# server WaveBuffer.ToCommon): with the buffer attached through the pool, the Common buffer address is
// Translate(0x2000, 0x100) = DspAddress 0x1000 + (0x2000 - 0x1000) = 0x2000. Version 2 copies the loop fields; any other version sets
// loop_count = -1 and leaves the loop offsets at 0. No context address, so the context fields stay 0.
NXTEST(wave_buffer_to_common) {
    std::vector<MemoryPoolInfo> pools(1, MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu));
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);
    AttachPool(mapper, pools);

    server::WaveBuffer wb;
    BehaviourParameter::ErrorInfo err{};
    CHECK(mapper.TryAttachBuffer(err, wb.BufferAddressInfo, 0x2000, 0x100));
    wb.StartSampleOffset = 0x10;
    wb.EndSampleOffset = 0x40;
    wb.ShouldLoop = true;
    wb.IsEndOfStream = false;
    wb.LoopStartSampleOffset = 0x10;
    wb.LoopEndSampleOffset = 0x20;
    wb.LoopCount = 3;

    ::nxrt::audren::WaveBuffer c2 = wb.ToCommon(2);
    CHECK(c2.buffer == 0x2000u);
    CHECK(c2.buffer_size == 0x100u);
    CHECK(c2.context == 0u);
    CHECK(c2.context_size == 0u);
    CHECK(c2.start_sample_offset == 0x10u);
    CHECK(c2.end_sample_offset == 0x40u);
    CHECK(c2.looping);
    CHECK(!c2.is_end_of_stream);
    CHECK(c2.loop_count == 3);
    CHECK(c2.loop_start_sample_offset == 0x10u);
    CHECK(c2.loop_end_sample_offset == 0x20u);

    ::nxrt::audren::WaveBuffer c1 = wb.ToCommon(1);
    CHECK(c1.buffer == 0x2000u);
    CHECK(c1.loop_count == -1);
    CHECK(c1.loop_start_sample_offset == 0u);
    CHECK(c1.loop_end_sample_offset == 0u);
}

// Derivation (C# WaveBufferInternal.IsSampleOffsetInRangeForPcm<T>): start * size_of(T) <= Size and end * size_of(T) <= Size.
//   PCM16 (2 bytes), Size 0x100: end 0x80 gives 0x100 <= 0x100 (true); end 0x81 gives 0x102 (false).
//   PcmFloat (4 bytes), Size 0x100: end 0x40 gives 0x100 (true); end 0x41 gives 0x104 (false).
NXTEST(is_sample_offset_valid_pcm16_and_float) {
    WaveBufferInternal w{};
    w.size = 0x100;
    w.start_sample_offset = 0;

    w.end_sample_offset = 0x80;
    CHECK(IsSampleOffsetValid(w, SampleFormat::PcmInt16));
    w.end_sample_offset = 0x81;
    CHECK(!IsSampleOffsetValid(w, SampleFormat::PcmInt16));

    w.end_sample_offset = 0x40;
    CHECK(IsSampleOffsetValid(w, SampleFormat::PcmFloat));
    w.end_sample_offset = 0x41;
    CHECK(!IsSampleOffsetValid(w, SampleFormat::PcmFloat));
}
