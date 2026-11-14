// SPDX-License-Identifier: GPL-3.0-or-later
// Tests for the Server/CommandBuffer and Server/CommandGenerator ports of the audio renderer.
//
// Every expected command list below is derived by hand from the C# sources under refs/ryubing/src/Ryujinx.Audio/Renderer/Server:
//   CG  = CommandGenerator.cs (GenerateVoices, GenerateVoice, GenerateDataSource, GenerateVoiceMix, GenerateSubMixes, GenerateSubMix,
//         GenerateMix, GenerateFinalMix, GenerateSinks, GenerateSink, GenerateDevice)
//   CB  = CommandBuffer.cs (the Generate methods; each one adds exactly one command, except the effects with a validity guard)
//   VI  = Voice/VoiceInfo.cs (UpdateParametersForCommandGeneration: Started sets WasPlaying = false and returns true when a wave buffer
//         was just sent; Stopped/Paused return WasPlaying = (PreviousPlayState == Started))
//   PCM = Dsp/Command/PcmInt16DataSourceCommandVersion1.cs (Initialize copies SampleRate and Pitch from the voice)
// The generation order is the order of the C# AudioRenderSystem calls: GenerateVoices, GenerateSubMixes, GenerateFinalMixes, GenerateSinks.
//
// Shared setup: renderer sample rate 48000, 160 samples per buffer, 2 channels, one voice of one channel (PCM16, Started, wave buffer 0
// not yet sent, Volume 1.0, PreviousVolume 0.0, mix volumes 1.0 for channel 0 and channel 1, no biquad filters, no splitter).
// Node ids: final mix 1, sub mix 2, voice 3, sink 4. Constants::InvalidNodeId is the id of the clear and the sink commands.
#include <cstdint>
#include <cstdio>
#include <memory>
#include <string>
#include <vector>

#include "../src/services/audren/Server/CommandGenerator.h"
#include "../src/services/audren/Server/CommandProcessingTimeEstimatorVersion5.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_gen)

namespace {

using namespace nxrt::audren;

constexpr int32_t kFinalNode = 1;
constexpr int32_t kSubNode = 2;
constexpr int32_t kVoiceNode = 3;
constexpr int32_t kSinkNode = 4;
constexpr uint32_t kSampleCount = 160;
constexpr uint32_t kRendererSampleRate = 48000;

// PerformanceManager that accepts every entry request (every voice, mix and sink is a profiled target).
class FakePerformanceManager : public PerformanceManager {
  public:
    uint32_t CopyHistories(Span<uint8_t>) override { return 0; }
    void SetTargetNodeId(int32_t) override {}
    bool IsTargetNodeId(int32_t) override { return true; }
    bool GetNextEntry(PerformanceEntryAddresses&, PerformanceEntryType, int32_t) override { return true; }
    bool GetNextEntry(PerformanceEntryAddresses&, PerformanceDetailType, PerformanceEntryType, int32_t) override { return true; }
    void TapFrame(bool, uint32_t, uint64_t) override {}
};

// The server objects a CommandGenerator is built from. Not copyable or movable: the members refer to each other.
struct Rig {
    BehaviourInfo behaviour;
    RendererSystemContext renderer;
    std::vector<float> depopBuffer;
    std::vector<float> mixBuffer;
    std::unique_ptr<CommandProcessingTimeEstimatorVersion5> estimator;
    std::unique_ptr<CommandList> commandList;
    std::unique_ptr<CommandBuffer> commandBuffer;

    std::vector<int32_t> sortedVoices{0};
    std::vector<VoiceInfo> voices{1};
    std::vector<VoiceChannelResource> channelResources{1};
    std::vector<VoiceState> voiceStatesCpu{1};
    std::vector<VoiceState> voiceStatesDsp{1};
    VoiceContext voiceContext;

    std::vector<int32_t> sortedMixes;
    std::vector<std::vector<int32_t>> mixOrders;  // effect processing order storage, one per mix (a MixInfo keeps a pointer to it)
    std::vector<MixInfo> mixes;
    MixContext mixContext;

    EffectContext effectContext;
    SinkContext sinkContext;
    SplitterContext splitterContext;

    std::unique_ptr<FakePerformanceManager> performance;
    std::unique_ptr<CommandGenerator> generator;

    // revisionLevel: one of BehaviourInfo::Revision1..Revision15. mixCount includes the final mix (index 0).
    Rig(int32_t revisionLevel, uint32_t mixBufferCount, size_t mixCount, uint32_t sinkCount, uint32_t effectCount, bool withPerformance = false) {
        behaviour.SetUserRevision(BehaviourInfo::BaseRevisionMagic + revisionLevel);

        renderer.SessionId = 0;
        renderer.ChannelCount = 2;
        renderer.MixBufferCount = mixBufferCount;
        renderer.BehaviourInfo = &behaviour;
        renderer.UpsamplerManager = nullptr;
        depopBuffer.assign(kSampleCount * 2, 0.0f);
        renderer.DepopBuffer = Span<float>(depopBuffer);

        // One buffer per mix buffer plus one per voice channel (the CommandList layout of the C# AudioRenderSystem).
        mixBuffer.assign(static_cast<size_t>(kSampleCount) * (mixBufferCount + static_cast<uint32_t>(Constants::VoiceChannelCountMax)), 0.0f);
        estimator = std::make_unique<CommandProcessingTimeEstimatorVersion5>(kSampleCount, mixBufferCount + static_cast<uint32_t>(Constants::VoiceChannelCountMax));
        commandList = std::make_unique<CommandList>(Span<float>(mixBuffer), kSampleCount, kRendererSampleRate, mixBufferCount,
                                                    static_cast<uint32_t>(Constants::VoiceChannelCountMax));
        commandBuffer = std::make_unique<CommandBuffer>(*commandList, *estimator);

        voiceStatesCpu.assign(1, VoiceState{});
        voiceStatesDsp.assign(1, VoiceState{});
        voiceContext.Initialize(Span<int32_t>(sortedVoices), Span<VoiceInfo>(voices), Span<VoiceChannelResource>(channelResources),
                                Span<VoiceState>(voiceStatesCpu), Span<VoiceState>(voiceStatesDsp), 1);

        sortedMixes.resize(mixCount);
        mixOrders.assign(mixCount, std::vector<int32_t>(4, Constants::InvalidProcessingOrder));
        for (size_t i = 0; i < mixCount; i++) {
            mixes.emplace_back(Span<int32_t>(mixOrders[i]), behaviour);
        }
        mixContext.Initialize(Span<int32_t>(sortedMixes), Span<MixInfo>(mixes), Span<uint8_t>(), Span<uint8_t>());

        effectContext.Initialize(effectCount, 0);
        sinkContext.Initialize(sinkCount);

        if (withPerformance) {
            performance = std::make_unique<FakePerformanceManager>();
        }
        generator = std::make_unique<CommandGenerator>(*commandBuffer, renderer, voiceContext, mixContext, effectContext, sinkContext, splitterContext,
                                                       performance ? performance.get() : nullptr);
    }

    Rig(const Rig&) = delete;
    Rig& operator=(const Rig&) = delete;

    // The C# AudioRenderSystem order.
    void GenerateAll() {
        generator->GenerateVoices();
        generator->GenerateSubMixes();
        generator->GenerateFinalMixes();
        generator->GenerateSinks();
    }

    std::vector<CommandType> Types() const {
        std::vector<CommandType> types;
        for (const auto& command : commandList->Commands) {
            types.push_back(command->GetCommandType());
        }
        return types;
    }

    uint32_t SumOfEstimates() const {
        uint32_t sum = 0;
        for (const auto& command : commandList->Commands) {
            sum += command->GetEstimatedProcessingTime();
        }
        return sum;
    }
};

// The final mix (C# MixInfo with MixId FinalMixId): buffers [0, bufferCount), volume 1.0, no destination.
void SetFinalMix(Rig& rig, uint32_t bufferCount) {
    MixInfo& mix = rig.mixes[0];
    mix.MixId = Constants::FinalMixId;
    mix.NodeId = kFinalNode;
    mix.IsUsed = true;
    mix.BufferOffset = 0;
    mix.BufferCount = bufferCount;
    mix.SampleRate = kRendererSampleRate;
    mix.Volume = 1.0f;
    mix.DestinationMixId = Constants::UnusedMixId;
    mix.DestinationSplitterId = Constants::UnusedSplitterId;
}

// The single voice: PCM16, Started, one channel, routed to the mix with id mixId.
void SetVoice(Rig& rig, uint32_t sampleRate, float pitch, int32_t mixId) {
    VoiceInfo& voice = rig.voices[0];
    voice.InUse = true;
    voice.IsNew = false;
    voice.WasPlaying = false;
    voice.SampleFormat = SampleFormat::PcmInt16;
    voice.SampleRate = sampleRate;
    voice.ChannelsCount = 1;
    voice.Id = 0;
    voice.NodeId = kVoiceNode;
    voice.MixId = mixId;
    voice.SplitterId = Constants::UnusedSplitterId;
    voice.PlayState = server::PlayState::Started;
    voice.PreviousPlayState = server::PlayState::Started;
    voice.Pitch = pitch;
    voice.Volume = 1.0f;
    voice.PreviousVolume = 0.0f;
    voice.WaveBuffersCount = 1;
    voice.ChannelResourceIds[0] = 0;
    voice.DataSourceStateUnmapped = false;
    voice.BufferInfoUnmapped = false;
    voice.VoiceDropFlag = false;

    VoiceChannelResource& resource = rig.channelResources[0];
    resource.Id = 0;
    resource.IsUsed = true;
    resource.Mix[0] = 1.0f;
    resource.Mix[1] = 1.0f;
}

// A device sink on the final mix (C# DeviceSink with SinkType.Device and two inputs).
void SetDeviceSink(Rig& rig) {
    auto sink = std::make_unique<DeviceSink>();
    sink->Type = SinkType::Device;
    sink->IsUsed = true;
    sink->NodeId = kSinkNode;
    sink->Parameter.input_count = 2;
    sink->Parameter.input[0] = 0;
    sink->Parameter.input[1] = 1;
    rig.sinkContext.GetSinkSlot(0) = std::move(sink);
}

std::string Describe(const std::vector<CommandType>& types) {
    std::string text;
    for (CommandType type : types) {
        text += std::to_string(static_cast<int>(type)) + " ";
    }
    return text;
}

// Expected command list of a PCM16 voice on the final mix with a device sink: CG GenerateVoices, GenerateFinalMixes, GenerateSinks.
std::vector<CommandType> FinalMixVoiceSequence(CommandType dataSource) {
    return {
        CommandType::ClearMixBuffer,        // CB GenerateClearMixBuffer, from the CommandGenerator constructor
        CommandType::DepopPrepare,          // CG GenerateDataSource, mix branch (MixId != UnusedMixId)
        dataSource,                         // CG GenerateDataSource, !WasPlaying
        CommandType::VolumeRamp,            // CG GenerateVoice, volume 0.0 -> 1.0 on buffer 2 (MixBufferCount 2 + channel 0)
        CommandType::MixRamp,               // CG GenerateVoiceMix: channel 0 to mix buffer 0 (bufferCount 2 <= 6, mixVolume 1.0)
        CommandType::MixRamp,               // CG GenerateVoiceMix: channel 0 to mix buffer 1
        CommandType::DepopForMixBuffers,    // CG GenerateFinalMix
        CommandType::DeviceSink,            // CG GenerateDevice, no upsampler (48000 Hz), no downmix (2 inputs, not 6)
    };
}

}  // namespace

// Scenario 1: one started PCM16 mono voice at 48000 Hz, one wave buffer, routed to the final mix (2 channels), device sink.
// BehaviourInfo revision 7 is below Revision8, so IsWaveBufferVersion2Supported is false and the data source is PcmInt16DataSourceVersion1.
// Command fields (CB and the command Initialize methods): output buffer = MixBufferCount + channelIndex = 2 + 0; volume ramp uses
// buffer 2 for input and output; mix ramp k uses input 2, output k, lastSampleIndex k; depop uses the final mix (offset 0, count 2, 48000 Hz).
NXTEST(voice_pcm16_final_mix_version1) {
    Rig rig(BehaviourInfo::Revision7, 2, 1, 1, 0);
    SetFinalMix(rig, 2);
    SetVoice(rig, 48000, 1.0f, Constants::FinalMixId);
    SetDeviceSink(rig);

    rig.GenerateAll();

    std::vector<CommandType> expected = FinalMixVoiceSequence(CommandType::PcmInt16DataSourceVersion1);
    auto actual = rig.Types();
    if (actual != expected) {
        std::fprintf(stderr, "scenario 1 types: %s\n", Describe(actual).c_str());
    }
    CHECK(actual == expected);
    if (actual.size() != expected.size()) {
        return;
    }

    const auto& commands = rig.commandList->Commands;
    auto* depop = dynamic_cast<const DepopPrepareCommand*>(commands[1].get());
    CHECK(depop != nullptr);
    if (depop) {
        CHECK_EQ(depop->NodeId, kVoiceNode);
        CHECK_EQ(depop->MixBufferCount, 2u);
    }
    auto* pcm = dynamic_cast<const PcmInt16DataSourceCommandVersion1*>(commands[2].get());
    CHECK(pcm != nullptr);
    if (pcm) {
        CHECK_EQ(pcm->OutputBufferIndex, 2);
        CHECK_EQ(pcm->SampleRate, 48000u);
        CHECK_EQ(pcm->Pitch, 1.0f);
    }
    auto* ramp = dynamic_cast<const VolumeRampCommand*>(commands[3].get());
    CHECK(ramp != nullptr);
    if (ramp) {
        CHECK_EQ(ramp->InputBufferIndex, 2);
        CHECK_EQ(ramp->OutputBufferIndex, 2);
        CHECK_EQ(ramp->Volume1, 1.0f);
    }
    auto* mix0 = dynamic_cast<const MixRampCommand*>(commands[4].get());
    auto* mix1 = dynamic_cast<const MixRampCommand*>(commands[5].get());
    CHECK(mix0 != nullptr && mix1 != nullptr);
    if (mix0 && mix1) {
        CHECK_EQ(mix0->InputBufferIndex, 2);
        CHECK_EQ(mix0->OutputBufferIndex, 0);
        CHECK_EQ(mix0->LastSampleIndex, 0);
        CHECK_EQ(mix1->InputBufferIndex, 2);
        CHECK_EQ(mix1->OutputBufferIndex, 1);
        CHECK_EQ(mix1->LastSampleIndex, 1);
    }
    auto* depopFinal = dynamic_cast<const DepopForMixBuffersCommand*>(commands[6].get());
    CHECK(depopFinal != nullptr);
    if (depopFinal) {
        CHECK_EQ(depopFinal->MixBufferOffset, 0u);
        CHECK_EQ(depopFinal->MixBufferCount, 2u);
    }
    auto* sink = dynamic_cast<const DeviceSinkCommand*>(commands[7].get());
    CHECK(sink != nullptr);
    if (sink) {
        CHECK_EQ(sink->NodeId, Constants::InvalidNodeId);
    }

    // CB AddCommand: the total is the sum of the estimates of the commands added.
    CHECK_EQ(rig.commandBuffer->EstimatedProcessingTime, rig.SumOfEstimates());
}

// Scenario 2: the same voice at 32000 Hz. CG and PCM: the data source copies the voice SampleRate (32000) and Pitch (0.5); the sample
// rate conversion itself runs in the DSP, so the command list is the same as scenario 1 apart from those two fields.
NXTEST(voice_pcm16_32000_carries_pitch) {
    Rig rig(BehaviourInfo::Revision7, 2, 1, 1, 0);
    SetFinalMix(rig, 2);
    SetVoice(rig, 32000, 0.5f, Constants::FinalMixId);
    SetDeviceSink(rig);

    rig.GenerateAll();

    auto actual = rig.Types();
    CHECK(actual == FinalMixVoiceSequence(CommandType::PcmInt16DataSourceVersion1));
    if (rig.commandList->Commands.size() > 2) {
        auto* pcm = dynamic_cast<const PcmInt16DataSourceCommandVersion1*>(rig.commandList->Commands[2].get());
        CHECK(pcm != nullptr);
        if (pcm) {
            CHECK_EQ(pcm->SampleRate, 32000u);
            CHECK_EQ(pcm->Pitch, 0.5f);
        }
    }
}

// Scenario 3: the voice goes to a sub mix (id 1, buffers 2..3) that has a reverb effect and goes to the final mix (buffers 0..1).
// CG GenerateVoices: depop and data source use the sub mix (count 2, offset 2); the channel writes to buffer 4 (MixBufferCount 4 + 0),
// and mixes to buffers 2 and 3 (mixVolume 1.0). CG GenerateSubMix: depop for the sub mix, then the reverb effect (BufferOffset 2),
// then GenerateMix: the sub mix has DestinationMixId 0, the identity volume matrix gives one Mix per buffer, 2 -> 0 and 3 -> 1.
// CG GenerateFinalMix: depop for the final mix. Then GenerateDevice.
NXTEST(sub_mix_with_reverb_effect) {
    Rig rig(BehaviourInfo::Revision7, 4, 2, 1, 1);
    SetFinalMix(rig, 2);

    MixInfo& sub = rig.mixes[1];
    sub.MixId = 1;
    sub.NodeId = kSubNode;
    sub.IsUsed = true;
    sub.BufferOffset = 2;
    sub.BufferCount = 2;
    sub.SampleRate = kRendererSampleRate;
    sub.Volume = 1.0f;
    sub.DestinationMixId = Constants::FinalMixId;
    sub.DestinationSplitterId = Constants::UnusedSplitterId;
    sub.MixBufferVolume()[0 * Constants::MixBufferCountMax + 0] = 1.0f;
    sub.MixBufferVolume()[1 * Constants::MixBufferCountMax + 1] = 1.0f;
    rig.mixOrders[1][0] = 0;  // effect 0 processes this mix

    auto reverb = std::make_unique<ReverbEffect>();
    reverb->MixId = 1;
    reverb->IsEnabled = true;
    reverb->Parameter.channel_count = 2;
    reverb->State = Span<ReverbState>();  // Initialize only stores the span; ReverbState has no default constructor
    rig.effectContext.GetEffect(0) = std::move(reverb);

    SetVoice(rig, 48000, 1.0f, 1);
    SetDeviceSink(rig);

    rig.GenerateAll();

    std::vector<CommandType> expected = {
        CommandType::ClearMixBuffer,      // CG constructor
        CommandType::DepopPrepare,        // voice depop on the sub mix (count 2, offset 2)
        CommandType::PcmInt16DataSourceVersion1,
        CommandType::VolumeRamp,          // buffer 4 (MixBufferCount 4 + channel 0)
        CommandType::MixRamp,             // 4 -> 2
        CommandType::MixRamp,             // 4 -> 3
        CommandType::DepopForMixBuffers,  // sub mix, GenerateSubMix
        CommandType::Reverb,              // sub mix effect 0, GenerateReverbEffect (BufferOffset 2)
        CommandType::Mix,                 // 2 -> 0
        CommandType::Mix,                 // 3 -> 1
        CommandType::DepopForMixBuffers,  // final mix
        CommandType::DeviceSink,
    };
    auto actual = rig.Types();
    if (actual != expected) {
        std::fprintf(stderr, "scenario 3 types: %s\n", Describe(actual).c_str());
    }
    CHECK(actual == expected);
    if (actual.size() != expected.size()) {
        return;
    }

    const auto& commands = rig.commandList->Commands;
    auto* reverbCommand = dynamic_cast<const ReverbCommand*>(commands[7].get());
    CHECK(reverbCommand != nullptr);
    if (reverbCommand) {
        CHECK_EQ(reverbCommand->NodeId, kSubNode);
    }
    auto* mix0 = dynamic_cast<const MixCommand*>(commands[8].get());
    auto* mix1 = dynamic_cast<const MixCommand*>(commands[9].get());
    CHECK(mix0 != nullptr && mix1 != nullptr);
    if (mix0 && mix1) {
        CHECK_EQ(mix0->InputBufferIndex, 2);
        CHECK_EQ(mix0->OutputBufferIndex, 0);
        CHECK_EQ(mix0->Volume, 1.0f);
        CHECK_EQ(mix1->InputBufferIndex, 3);
        CHECK_EQ(mix1->OutputBufferIndex, 1);
        CHECK_EQ(mix1->Volume, 1.0f);
    }
    CHECK_EQ(rig.commandBuffer->EstimatedProcessingTime, rig.SumOfEstimates());
}

// Scenario 4: the final mix goes to a circular buffer sink instead of a device sink. CG GenerateSink: CircularBuffer ->
// GenerateCircularBuffer(finalMix.BufferOffset, sink, InvalidNodeId). The CommandBuffer command carries the sink's current write offset.
NXTEST(circular_buffer_sink) {
    Rig rig(BehaviourInfo::Revision7, 2, 1, 1, 0);
    SetFinalMix(rig, 2);
    SetVoice(rig, 48000, 1.0f, Constants::FinalMixId);

    auto sink = std::make_unique<CircularBufferSink>();
    sink->Type = SinkType::CircularBuffer;
    sink->IsUsed = true;
    sink->NodeId = kSinkNode;
    sink->CurrentWriteOffset = 0x40;
    rig.sinkContext.GetSinkSlot(0) = std::move(sink);

    rig.GenerateAll();

    std::vector<CommandType> expected = FinalMixVoiceSequence(CommandType::PcmInt16DataSourceVersion1);
    expected.back() = CommandType::CircularBufferSink;
    auto actual = rig.Types();
    if (actual != expected) {
        std::fprintf(stderr, "scenario 4 types: %s\n", Describe(actual).c_str());
    }
    CHECK(actual == expected);
    if (actual.size() == expected.size()) {
        auto* circular = dynamic_cast<const CircularBufferSinkCommand*>(rig.commandList->Commands.back().get());
        CHECK(circular != nullptr);
        if (circular) {
            CHECK_EQ(circular->NodeId, Constants::InvalidNodeId);
            CHECK_EQ(circular->CurrentOffset, 0x40u);
        }
    }
}

// Scenario 5: a stopped voice generates nothing. VI: PlayState Stopped with PreviousPlayState Stopped gives WasPlaying = false and
// UpdateForCommandGeneration returns false, so GenerateVoice never runs for it. Only the final mix and the device sink remain.
NXTEST(stopped_voice_generates_nothing) {
    Rig rig(BehaviourInfo::Revision7, 2, 1, 1, 0);
    SetFinalMix(rig, 2);
    SetVoice(rig, 48000, 1.0f, Constants::FinalMixId);
    rig.voices[0].PlayState = server::PlayState::Stopped;
    rig.voices[0].PreviousPlayState = server::PlayState::Stopped;
    SetDeviceSink(rig);

    rig.GenerateAll();

    std::vector<CommandType> expected = {
        CommandType::ClearMixBuffer,
        CommandType::DepopForMixBuffers,
        CommandType::DeviceSink,
    };
    auto actual = rig.Types();
    if (actual != expected) {
        std::fprintf(stderr, "scenario 5 types: %s\n", Describe(actual).c_str());
    }
    CHECK(actual == expected);
}

// Scenario 6: with a PerformanceManager that accepts every entry, the performance commands bracket the voice, mix and sink work.
// CG GenerateVoices: Start(Voice); GenerateVoice: Start(PcmInt16) End(PcmInt16) around the data source; Start(BiquadFilter) End around the
// (empty) biquad step, because the voice has a destination; Start(VolumeRamp) End; Start(Mix) End around the mix ramps; End(Voice).
// CG GenerateFinalMixes: Start(FinalMix); GenerateFinalMix: depop, Start(Mix) End(Mix) (the volume is 1.0, so no VolumeRamp pair),
// End(FinalMix). CG GenerateSinks: Start(Sink) device sink End(Sink).
NXTEST(performance_commands_bracket_work) {
    Rig rig(BehaviourInfo::Revision7, 2, 1, 1, 0, true);
    SetFinalMix(rig, 2);
    SetVoice(rig, 48000, 1.0f, Constants::FinalMixId);
    SetDeviceSink(rig);

    rig.GenerateAll();

    using T = PerformanceCommand::Type;
    std::vector<CommandType> expected = {
        CommandType::ClearMixBuffer,
        CommandType::Performance,          // Start(Voice)
        CommandType::Performance,          // Start(PcmInt16)
        CommandType::DepopPrepare,
        CommandType::PcmInt16DataSourceVersion1,
        CommandType::Performance,          // End(PcmInt16)
        CommandType::Performance,          // Start(BiquadFilter)
        CommandType::Performance,          // End(BiquadFilter)
        CommandType::Performance,          // Start(VolumeRamp)
        CommandType::VolumeRamp,
        CommandType::Performance,          // End(VolumeRamp)
        CommandType::Performance,          // Start(Mix)
        CommandType::MixRamp,
        CommandType::MixRamp,
        CommandType::Performance,          // End(Mix)
        CommandType::Performance,          // End(Voice)
        CommandType::Performance,          // Start(FinalMix)
        CommandType::DepopForMixBuffers,
        CommandType::Performance,          // Start(Mix, final)
        CommandType::Performance,          // End(Mix, final)
        CommandType::Performance,          // End(FinalMix)
        CommandType::Performance,          // Start(Sink)
        CommandType::DeviceSink,
        CommandType::Performance,          // End(Sink)
    };
    auto actual = rig.Types();
    if (actual != expected) {
        std::fprintf(stderr, "scenario 6 types: %s\n", Describe(actual).c_str());
    }
    CHECK(actual == expected);
    if (actual.size() != expected.size()) {
        return;
    }

    // The Start/End sequence of the performance commands, in order (S = Start, E = End).
    std::vector<T> perfTypes = {T::Start, T::Start, T::End, T::Start, T::End, T::Start, T::End, T::Start, T::End, T::End, T::Start, T::Start,
                                T::End,   T::End,   T::Start, T::End};
    std::vector<T> actualPerf;
    for (const auto& command : rig.commandList->Commands) {
        if (auto* perf = dynamic_cast<const PerformanceCommand*>(command.get())) {
            actualPerf.push_back(perf->PerformanceType);
        }
    }
    CHECK(actualPerf == perfTypes);
    CHECK_EQ(rig.commandBuffer->EstimatedProcessingTime, rig.SumOfEstimates());
}

// Scenario 7: revision 11 (>= Revision8) selects the wave buffer version 2 data source. CB GenerateDataSourceVersion2 creates a
// DataSourceVersion2Command whose type for PCM16 is PcmInt16DataSourceVersion2 (DataSourceVersion2Command.cs GetCommandTypeBySampleFormat).
NXTEST(voice_pcm16_data_source_version2) {
    Rig rig(BehaviourInfo::Revision11, 2, 1, 1, 0);
    SetFinalMix(rig, 2);
    SetVoice(rig, 48000, 1.0f, Constants::FinalMixId);
    SetDeviceSink(rig);

    rig.GenerateAll();

    auto actual = rig.Types();
    if (actual != FinalMixVoiceSequence(CommandType::PcmInt16DataSourceVersion2)) {
        std::fprintf(stderr, "scenario 7 types: %s\n", Describe(actual).c_str());
    }
    CHECK(actual == FinalMixVoiceSequence(CommandType::PcmInt16DataSourceVersion2));
}
