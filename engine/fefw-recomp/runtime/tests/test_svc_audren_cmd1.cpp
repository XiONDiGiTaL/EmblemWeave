// SPDX-License-Identifier: GPL-3.0-or-later
// Tests for the basic DSP commands (src/services/audren/Dsp/Command, first batch). Every expected value is derived by hand from the C#
// arithmetic (Ryujinx.Audio/Renderer/Dsp/Command/*.cs and the helpers they call), not by running the port.
#include <chrono>
#include <cstdint>
#include <cstring>
#include <vector>

#include "../src/services/audren/Dsp/Command/AdpcmDataSourceCommandVersion1.h"
#include "../src/services/audren/Dsp/Command/CaptureBufferCommand.h"
#include "../src/services/audren/Dsp/Command/CircularBufferSinkCommand.h"
#include "../src/services/audren/Dsp/Command/ClearMixBufferCommand.h"
#include "../src/services/audren/Dsp/Command/CopyMixBufferCommand.h"
#include "../src/services/audren/Dsp/Command/DataSourceVersion2Command.h"
#include "../src/services/audren/Dsp/Command/DepopForMixBuffersCommand.h"
#include "../src/services/audren/Dsp/Command/DepopPrepareCommand.h"
#include "../src/services/audren/Dsp/Command/DeviceSinkCommand.h"
#include "../src/services/audren/Dsp/Command/DownMixSurroundToStereoCommand.h"
#include "../src/services/audren/Dsp/Command/FillBufferCommand.h"
#include "../src/services/audren/Dsp/Command/MixCommand.h"
#include "../src/services/audren/Dsp/Command/MixRampCommand.h"
#include "../src/services/audren/Dsp/Command/MixRampGroupedCommand.h"
#include "../src/services/audren/Dsp/Command/PcmFloatDataSourceCommandVersion1.h"
#include "../src/services/audren/Dsp/Command/PcmInt16DataSourceCommandVersion1.h"
#include "../src/services/audren/Dsp/Command/PerformanceCommand.h"
#include "../src/services/audren/Dsp/Command/UpsampleCommand.h"
#include "../src/services/audren/Dsp/Command/VolumeCommand.h"
#include "../src/services/audren/Dsp/Command/VolumeRampCommand.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_cmd1)

using namespace nxrt::audren;

namespace {

uint64_t NowNs() {
    return static_cast<uint64_t>(std::chrono::duration_cast<std::chrono::nanoseconds>(std::chrono::steady_clock::now().time_since_epoch()).count());
}

// Stands in for the server AddressInfo: GetReference(true) returns the guest address.
struct FakeAddressInfo {
    uint64_t Address = 0;
    uint64_t Size = 0;
    uint64_t GetReference(bool) const { return Address; }
};

// Stands in for the server Voice WaveBuffer: ToCommon builds the Dsp WaveBuffer as Server/Voice/WaveBuffer.cs does.
struct FakeServerWaveBuffer {
    FakeAddressInfo BufferAddressInfo;
    FakeAddressInfo ContextAddressInfo;
    uint32_t StartSampleOffset = 0;
    uint32_t EndSampleOffset = 0;
    bool ShouldLoop = false;
    bool IsEndOfStream = false;

    WaveBuffer ToCommon(int version) const {
        WaveBuffer wb{};
        wb.buffer = BufferAddressInfo.GetReference(true);
        wb.buffer_size = static_cast<uint32_t>(BufferAddressInfo.Size);
        wb.start_sample_offset = StartSampleOffset;
        wb.end_sample_offset = EndSampleOffset;
        wb.looping = ShouldLoop;
        wb.is_end_of_stream = IsEndOfStream;
        wb.loop_count = version == 2 ? 0 : -1;
        return wb;
    }
};

// Stands in for the server VoiceInfo (the members the data source commands read).
struct FakeVoiceInfo {
    uint32_t SampleRate = 48000;
    uint32_t ChannelsCount = 1;
    float Pitch = 1.0f;
    FakeServerWaveBuffer WaveBuffers[4];
    ::nxrt::audren::DecodingBehaviour DecodingBehaviour = ::nxrt::audren::DecodingBehaviour::Default;
    ::nxrt::audren::SampleFormat SampleFormat = ::nxrt::audren::SampleFormat::PcmInt16;
    ::nxrt::audren::SampleRateConversionQuality SrcQuality = ::nxrt::audren::SampleRateConversionQuality::Default;
    FakeAddressInfo DataSourceStateAddressInfo;
};

// Mock hardware device: records what the DeviceSink appends.
struct FakeDevice : IHardwareDevice {
    uint32_t SampleRateValue = 48000;
    uint32_t Channels = 2;
    int Appends = 0;
    uint32_t LastChannelCount = 0;
    std::vector<int16_t> Data;

    void SetVolume(float) override {}
    float GetVolume() override { return 1.0f; }
    uint32_t GetSampleRate() override { return SampleRateValue; }
    uint32_t GetChannelCount() override { return Channels; }
    void AppendBuffer(Span<const int16_t> data, uint32_t channelCount) override {
        Appends++;
        LastChannelCount = channelCount;
        Data.assign(data.begin(), data.end());
    }
};

// Stands in for the server splitter destination: GetV1RefOrNull / GetV2RefOrNull return nullptr when the version is absent.
struct FakeSplitterVersion {
    std::vector<float> PreviousMixBufferVolume;
};
struct FakeSplitterDestination {
    FakeSplitterVersion* V1 = nullptr;
    FakeSplitterVersion* V2 = nullptr;
    FakeSplitterVersion* GetV1RefOrNull() const { return V1; }
    FakeSplitterVersion* GetV2RefOrNull() const { return V2; }
};

// Stands in for the server DeviceSink: Parameter is the binary DeviceParameter, UpsamplerInfo a pointer (nullptr: no upsampling).
struct FakeDeviceSink {
    ::nxrt::audren::DeviceParameter Parameter{};
    ::nxrt::audren::UpsamplerInfo* UpsamplerInfo = nullptr;
};

}  // namespace

// ---- ClearMixBuffer / CopyMixBuffer / FillBuffer on a small slab ----

NXTEST(clear_copy_fill_on_small_slab) {
    // 3 buffers of 4 samples (mixBufferCount 2 + voiceChannelCountMax 1).
    std::vector<float> mix(3 * 4, 7.0f);
    CommandList list(Span<float>(mix), 4, 48000, 2, 1);
    CHECK_EQ(list.BufferCount, 3u);
    for (int i = 0; i < 4; i++) mix[i] = 1.0f + static_cast<float>(i);  // buffer 0 = {1,2,3,4}

    CopyMixBufferCommand copy;
    copy.Initialize(0, 2, 1);
    copy.Process(list);  // buffer 2 = buffer 0
    CHECK_EQ(mix[8], 1.0f);
    CHECK_EQ(mix[11], 4.0f);
    CHECK_EQ(mix[4], 7.0f);  // buffer 1 untouched
    CHECK(copy.GetCommandType() == CommandType::CopyMixBuffer);
    CHECK_EQ(copy.NodeId, 1);

    ClearMixBufferCommand clear;
    clear.Initialize(2);
    clear.Process(list);  // clears the whole slab
    CHECK_EQ(mix[0], 0.0f);
    CHECK_EQ(mix[11], 0.0f);
    CHECK_EQ(mix[8], 0.0f);

    FakeSplitterVersion v1;
    v1.PreviousMixBufferVolume.assign(24, 9.0f);
    FakeSplitterDestination dest;
    dest.V1 = &v1;
    FillBufferCommand fill;
    fill.Initialize(dest, 4, 0.5f, 3);
    fill.Process(list);
    CHECK(!fill.IsV2);
    CHECK_EQ(v1.PreviousMixBufferVolume[0], 0.5f);
    CHECK_EQ(v1.PreviousMixBufferVolume[3], 0.5f);
    CHECK_EQ(v1.PreviousMixBufferVolume[4], 9.0f);  // only Length entries are written
}

// ---- Volume: output = round(input * volume), half to even (C# MathF.Round in the scalar path) ----

NXTEST(volume_multiplies_and_rounds_half_to_even) {
    std::vector<float> mix(3 * 8, 0.0f);
    CommandList list(Span<float>(mix), 8, 48000, 2, 1);
    for (int i = 0; i < 8; i++) mix[i] = 2.0f * static_cast<float>(i) + 1.0f;  // 1,3,5,...,15

    VolumeCommand volume;
    volume.Initialize(0.5f, 0, 4);
    volume.Process(list);
    // i + 0.5 rounded half to even: 0.5->0, 1.5->2, 2.5->2, 3.5->4, 4.5->4, 5.5->6, 6.5->6, 7.5->8
    const float expected[8] = {0, 2, 2, 4, 4, 6, 6, 8};
    for (int i = 0; i < 8; i++) CHECK_EQ(mix[i], expected[i]);
}

// ---- VolumeRamp: linear from Volume0 at sample 0; Volume1 is reached only after the last sample (C# semantics) ----

NXTEST(volume_ramp_linear_across_sample_count) {
    std::vector<float> mix(3 * 8, 0.0f);
    CommandList list(Span<float>(mix), 8, 48000, 2, 1);
    for (int i = 0; i < 8; i++) mix[i] = 800.0f;

    VolumeRampCommand ramp;
    ramp.Initialize(1.0f, 0.0f, 0, 5);  // ramp = (0 - 1) / 8 = -0.125; volume_i = 1 - 0.125 i
    ramp.Process(list);
    // output_i = 800 * (1 - 0.125 i) = 800 - 100 i
    CHECK_EQ(mix[0], 800.0f);  // first
    CHECK_EQ(mix[4], 400.0f);  // middle
    CHECK_EQ(mix[7], 100.0f);  // last (volume 0.125, not Volume1)
}

// ---- Mix: output += round(input * volume) ----

NXTEST(mix_accumulates_into_output_buffer) {
    std::vector<float> mix(3 * 8, 0.0f);
    CommandList list(Span<float>(mix), 8, 48000, 2, 1);
    for (int i = 0; i < 8; i++) {
        mix[i] = 2.0f * static_cast<float>(i);       // buffer 0 = 0,2,4,...,14
        mix[8 + i] = 10.0f;                           // buffer 1 = 10
    }

    MixCommand mixCmd;
    mixCmd.Initialize(0, 1, 6, 0.25f);  // C# order: input, output, nodeId, volume
    mixCmd.Process(list);
    // input*0.25 = 0, 0.5, 1, 1.5, 2, 2.5, 3, 3.5 -> round half even = 0, 0, 1, 2, 2, 2, 3, 4
    const float expected[8] = {10, 10, 11, 12, 12, 12, 13, 14};
    for (int i = 0; i < 8; i++) CHECK_EQ(mix[8 + i], expected[i]);
    CHECK_EQ(mix[0], 0.0f);  // input unchanged
}

// ---- MixRamp: output += ramped product, last mixed sample stored in the voice state ----

NXTEST(mix_ramp_accumulates_and_returns_last_sample) {
    std::vector<float> mix(3 * 8, 0.0f);
    CommandList list(Span<float>(mix), 8, 48000, 2, 1);
    for (int i = 0; i < 8; i++) {
        mix[i] = 800.0f;
        mix[8 + i] = 10.0f;
    }
    std::vector<VoiceState> voice(1);

    MixRampCommand mixRamp;
    mixRamp.Initialize(1.0f, 0.0f, 0, 1, 3, Span<VoiceState>(voice), 8);
    mixRamp.Process(list);
    // output = 10 + (800 - 100 i)
    CHECK_EQ(mix[8], 810.0f);
    CHECK_EQ(mix[8 + 4], 410.0f);
    CHECK_EQ(mix[8 + 7], 110.0f);
    // The returned state is the last mixed product (sample 7: 800 * 0.125 = 100).
    CHECK_EQ(voice[0].last_samples[3], 100.0f);
}

// ---- DownMixSurroundToStereo: one frame, hand-computed with Constants::DefaultSurroundToStereoCoefficients {1, 0.707, 0.251, 0.707} ----

NXTEST(downmix_surround_to_stereo_one_frame) {
    // 12 buffers of 1 sample: inputs 0..5 (FL, FR, FC, LFE, BL, BR), stereo outputs 6 and 7, outputs 8..11 are cleared.
    std::vector<float> mix(12 * 1, 0.0f);
    CommandList list(Span<float>(mix), 1, 48000, 11, 1);
    mix[0] = 1000.0f;  // FL
    mix[1] = 2000.0f;  // FR
    mix[2] = 200.0f;   // FC
    mix[3] = 400.0f;   // LFE
    mix[4] = 600.0f;   // BL
    mix[5] = 200.0f;   // BR
    for (int i = 8; i < 12; i++) mix[i] = 9.0f;

    const uint8_t inputOffsets[6] = {0, 1, 2, 3, 4, 5};
    const uint8_t outputOffsets[6] = {6, 7, 8, 9, 10, 11};
    DownMixSurroundToStereoCommand downmix;
    downmix.Initialize(0, Span<const uint8_t>(inputOffsets, 6), Span<const uint8_t>(outputOffsets, 6),
                       Span<const float>(Constants::DefaultSurroundToStereoCoefficients), 2);
    downmix.Process(list);
    // L = 1.0*FL + 0.707*FC + 0.251*LFE + 0.707*BL = 1000 + 141.4 + 100.4 + 424.2 = 1666.0 -> 1666
    // R = 1.0*FR + 0.707*FC + 0.251*LFE + 0.707*BR = 2000 + 141.4 + 100.4 + 141.4 = 2383.2 -> 2383
    CHECK_EQ(mix[6], 1666.0f);
    CHECK_EQ(mix[7], 2383.0f);
    for (int i = 8; i < 12; i++) CHECK_EQ(mix[i], 0.0f);  // cleared
}

// ---- PCM int16 data source: one wave buffer of 240 samples, skip pitch/SRC so the samples pass through unchanged ----

NXTEST(pcm_int16_data_source_passes_samples_through_wave_buffer) {
    std::vector<float> mix(2 * 240, 0.0f);  // mixBufferCount 1 + voiceChannelCountMax 1
    CommandList list(Span<float>(mix), 240, 48000, 1, 1);

    int16_t pcm[240];
    for (int j = 0; j < 240; j++) pcm[j] = static_cast<int16_t>(j * 7 - 800);

    FakeVoiceInfo vi;
    vi.SampleRate = 48000;
    vi.ChannelsCount = 1;
    vi.Pitch = 1.0f;
    vi.DecodingBehaviour = ::nxrt::audren::DecodingBehaviour::SkipPitchAndSampleRateConversion;
    vi.WaveBuffers[0].BufferAddressInfo.Address = reinterpret_cast<uint64_t>(pcm);
    vi.WaveBuffers[0].BufferAddressInfo.Size = 480;  // 240 int16 samples
    vi.WaveBuffers[0].StartSampleOffset = 0;
    vi.WaveBuffers[0].EndSampleOffset = 240;

    std::vector<VoiceState> voice(1);
    voice[0].is_wave_buffer_valid[0] = true;

    PcmInt16DataSourceCommandVersion1 cmd;
    cmd.Initialize(vi, Span<VoiceState>(voice), 0, 0, 7);
    CHECK(cmd.Enabled);
    CHECK_EQ(cmd.NodeId, 7);
    CHECK(cmd.GetCommandType() == CommandType::PcmInt16DataSourceVersion1);
    cmd.Process(list);

    for (int j = 0; j < 240; j++) CHECK_EQ(mix[j], static_cast<float>(pcm[j]));  // raw int16 values, unscaled
    CHECK_EQ(voice[0].played_sample_count, 240ull);
    CHECK_EQ(voice[0].wave_buffer_consumed, 1u);
    CHECK(!voice[0].is_wave_buffer_valid[0]);
}

// ---- PCM float data source: float * 32767, saturated, truncated to int16 (PcmHelper.ConvertSampleToPcmInt16 / Saturate) ----

NXTEST(pcm_float_data_source_converts_to_int16_scale) {
    std::vector<float> mix(2 * 240, 0.0f);
    CommandList list(Span<float>(mix), 240, 48000, 1, 1);

    float pcm[240] = {};
    pcm[0] = 0.5f;    // 16383.5 -> 16383
    pcm[1] = -1.0f;   // -32767
    pcm[2] = 2.0f;    // 65534 -> saturated 32767
    pcm[3] = -2.0f;   // -65534 -> saturated -32768
    pcm[4] = 0.25f;   // 8191.75 -> 8191

    FakeVoiceInfo vi;
    vi.SampleFormat = ::nxrt::audren::SampleFormat::PcmFloat;
    vi.DecodingBehaviour = ::nxrt::audren::DecodingBehaviour::SkipPitchAndSampleRateConversion;
    vi.WaveBuffers[0].BufferAddressInfo.Address = reinterpret_cast<uint64_t>(pcm);
    vi.WaveBuffers[0].BufferAddressInfo.Size = 960;
    vi.WaveBuffers[0].StartSampleOffset = 0;
    vi.WaveBuffers[0].EndSampleOffset = 240;

    std::vector<VoiceState> voice(1);
    voice[0].is_wave_buffer_valid[0] = true;

    PcmFloatDataSourceCommandVersion1 cmd;
    cmd.Initialize(vi, Span<VoiceState>(voice), 0, 0, 2);
    cmd.Process(list);
    CHECK_EQ(mix[0], 16383.0f);
    CHECK_EQ(mix[1], -32767.0f);
    CHECK_EQ(mix[2], 32767.0f);
    CHECK_EQ(mix[3], -32768.0f);
    CHECK_EQ(mix[4], 8191.0f);
    CHECK_EQ(mix[5], 0.0f);
}

// ---- ADPCM data source: one hand-built 14-sample frame, zero coefficients (no prediction), scale 0 ----

NXTEST(adpcm_data_source_decodes_one_frame_pass_through) {
    std::vector<float> mix(2 * 240, 99.0f);  // sentinel: samples past the decoded frame must stay untouched
    CommandList list(Span<float>(mix), 240, 48000, 1, 1);

    // Frame: header 0x00 (coefficient index 0, scale 0), then 7 bytes of nibble pairs (high nibble first), signed 4-bit values.
    // 0x12 -> 1, 2 | 0x34 -> 3, 4 | 0x56 -> 5, 6 | 0x78 -> 7, -8 | 0x9A -> -7, -6 | 0xBC -> -5, -4 | 0xDE -> -3, -2
    uint8_t frame[8] = {0x00, 0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE};
    int16_t coefficients[16] = {};  // all zero: prediction is 0 for every sample

    FakeVoiceInfo vi;
    vi.SampleFormat = ::nxrt::audren::SampleFormat::Adpcm;
    vi.DecodingBehaviour = ::nxrt::audren::DecodingBehaviour::SkipPitchAndSampleRateConversion;
    vi.WaveBuffers[0].BufferAddressInfo.Address = reinterpret_cast<uint64_t>(frame);
    vi.WaveBuffers[0].BufferAddressInfo.Size = 8;
    vi.WaveBuffers[0].StartSampleOffset = 0;
    vi.WaveBuffers[0].EndSampleOffset = 14;
    vi.DataSourceStateAddressInfo.Address = reinterpret_cast<uint64_t>(coefficients);
    vi.DataSourceStateAddressInfo.Size = sizeof(coefficients);

    std::vector<VoiceState> voice(1);
    voice[0].is_wave_buffer_valid[0] = true;

    AdpcmDataSourceCommandVersion1 cmd;
    cmd.Initialize(vi, Span<VoiceState>(voice), 0, 5);
    cmd.Process(list);

    const float expected[14] = {1, 2, 3, 4, 5, 6, 7, -8, -7, -6, -5, -4, -3, -2};
    for (int j = 0; j < 14; j++) CHECK_EQ(mix[j], expected[j]);
    CHECK_EQ(mix[14], 99.0f);
}

// ---- DataSourceVersion2: the command type follows the sample format ----

NXTEST(data_source_version2_command_type_follows_sample_format) {
    std::vector<float> mix(2 * 240, 0.0f);
    CommandList list(Span<float>(mix), 240, 48000, 1, 1);
    std::vector<VoiceState> voice(1);

    FakeVoiceInfo vi;
    vi.SampleFormat = ::nxrt::audren::SampleFormat::PcmInt16;
    DataSourceVersion2Command v2;
    v2.Initialize(vi, Span<VoiceState>(voice), 0, 0, 1);
    CHECK(v2.GetCommandType() == CommandType::PcmInt16DataSourceVersion2);

    vi.SampleFormat = ::nxrt::audren::SampleFormat::PcmFloat;
    v2.Initialize(vi, Span<VoiceState>(voice), 0, 0, 1);
    CHECK(v2.GetCommandType() == CommandType::PcmFloatDataSourceVersion2);

    vi.SampleFormat = ::nxrt::audren::SampleFormat::Adpcm;
    v2.Initialize(vi, Span<VoiceState>(voice), 0, 0, 1);
    CHECK(v2.GetCommandType() == CommandType::AdpcmDataSourceVersion2);
}

// ---- DeviceSink: PcmHelper.Saturate per sample (truncation, clamp), interleaved by device channel ----

NXTEST(device_sink_saturates_and_interleaves) {
    std::vector<float> mix(3 * 240, 0.0f);  // mixBufferCount 2 + 1
    CommandList list(Span<float>(mix), 240, 48000, 2, 1);

    // Buffer 0 (left) and buffer 1 (right).
    for (int j = 2; j < 240; j++) {
        mix[j] = static_cast<float>(j) + 0.75f;
        mix[240 + j] = -(static_cast<float>(j) + 0.75f);
    }
    mix[0] = 40000.0f;   // left, saturates
    mix[1] = 1.5f;       // left, truncates to 1
    mix[240] = -40000.0f;  // right, saturates
    mix[241] = -1.5f;      // right, truncates to -1

    FakeDeviceSink sink;
    sink.Parameter.input_count = 2;
    sink.Parameter.input[0] = 0;
    sink.Parameter.input[1] = 1;

    DeviceSinkCommand cmd;
    cmd.Initialize(0, sink, 0, list.Buffers, 9);
    CHECK_EQ(cmd.InputCount, 2u);

    FakeDevice dev;
    dev.Channels = 2;
    list.OutputDevice = &dev;
    cmd.Process(list);
    CHECK_EQ(dev.Appends, 1);
    CHECK_EQ(dev.LastChannelCount, 2u);
    CHECK_EQ(dev.Data.size(), 480u);
    CHECK_EQ(dev.Data[0], int16_t{32767});
    CHECK_EQ(dev.Data[1], int16_t{-32768});
    CHECK_EQ(dev.Data[2], int16_t{1});
    CHECK_EQ(dev.Data[3], int16_t{-1});
    // Sample j >= 2: left = trunc(j + 0.75) = j, right = trunc(-(j + 0.75)) = -j.
    for (int j : {2, 50, 239}) {
        CHECK_EQ(dev.Data[2 * j], static_cast<int16_t>(j));
        CHECK_EQ(dev.Data[2 * j + 1], static_cast<int16_t>(-j));
    }

    // Six-channel device: the output has 6 channels per frame; channels 2..5 stay zero.
    dev.Channels = 6;
    cmd.Process(list);
    CHECK_EQ(dev.Appends, 2);
    CHECK_EQ(dev.LastChannelCount, 6u);
    CHECK_EQ(dev.Data.size(), 1440u);
    CHECK_EQ(dev.Data[0], int16_t{32767});
    CHECK_EQ(dev.Data[1], int16_t{-32768});
    CHECK_EQ(dev.Data[6], int16_t{1});   // frame 1, left
    CHECK_EQ(dev.Data[7], int16_t{-1});  // frame 1, right
    CHECK_EQ(dev.Data[2], int16_t{0});
    CHECK_EQ(dev.Data[5], int16_t{0});
    CHECK_EQ(dev.Data[8], int16_t{0});
}

// ---- CircularBufferSink: samples written as int16 into the ring, wrap at CircularBufferSize ----

NXTEST(circular_buffer_sink_writes_and_wraps) {
    std::vector<float> mix(3 * 4, 0.0f);  // 4 samples per buffer
    CommandList list(Span<float>(mix), 4, 48000, 2, 1);
    const float in0[4] = {1.5f, -2.5f, 3.75f, 40000.0f};
    const float in1[4] = {-1.0f, 0.5f, 100.9f, -40000.0f};
    for (int j = 0; j < 4; j++) {
        mix[j] = in0[j];
        mix[4 + j] = in1[j];
    }

    alignas(8) uint8_t ring[32] = {};
    CircularBufferParameter parameter{};
    parameter.buffer_size = 24;
    parameter.input_count = 2;
    parameter.input[0] = 0;
    parameter.input[1] = 1;
    FakeAddressInfo ringInfo;
    ringInfo.Address = reinterpret_cast<uint64_t>(ring);
    ringInfo.Size = sizeof(ring);

    // Start at offset 16: input 0 goes to bytes 16..23 (offset then reaches 24 >= 24, wraps to 0); input 1 goes to bytes 0..7.
    CircularBufferSinkCommand cmd;
    cmd.Initialize(0, parameter, ringInfo, 16, 3);
    CHECK_EQ(cmd.CircularBufferSize, 24ull);
    cmd.Process(list);

    int16_t r[12];
    std::memcpy(r, ring, sizeof(r));
    // input 1 (at bytes 0..7): trunc(-1.0), trunc(0.5), trunc(100.9), saturate(-40000)
    CHECK_EQ(r[0], int16_t{-1});
    CHECK_EQ(r[1], int16_t{0});
    CHECK_EQ(r[2], int16_t{100});
    CHECK_EQ(r[3], int16_t{-32768});
    // input 0 (at bytes 16..23): trunc(1.5), trunc(-2.5), trunc(3.75), saturate(40000)
    CHECK_EQ(r[8], int16_t{1});
    CHECK_EQ(r[9], int16_t{-2});
    CHECK_EQ(r[10], int16_t{3});
    CHECK_EQ(r[11], int16_t{32767});
    // Nothing else was written.
    CHECK_EQ(r[4], int16_t{0});
    CHECK_EQ(r[7], int16_t{0});
}

// ---- Performance: start/end fields through PerformanceEntryAddresses ----

NXTEST(performance_command_writes_entry_fields) {
    std::vector<float> mix(2 * 4, 0.0f);
    CommandList list(Span<float>(mix), 4, 48000, 1, 1);

    std::vector<int32_t> entry(4, 0);
    PerformanceEntryAddresses addresses;
    addresses.BaseMemory = Span<int32_t>(entry);
    addresses.StartTimeOffset = 0;
    addresses.EntryCountOffset = 4;
    addresses.ProcessingTimeOffset = 8;

    // The DSP started 2 ms ago: start time = elapsed microseconds, at least 2000.
    list.StartTime = NowNs() - 2000000;
    PerformanceCommand start;
    start.Initialize(addresses, PerformanceCommand::Type::Start, 1);
    start.Process(list);
    CHECK(entry[0] >= 2000);
    CHECK(entry[0] < 2000 + 1000000);

    PerformanceCommand end;
    end.Initialize(addresses, PerformanceCommand::Type::End, 1);
    end.Process(list);
    CHECK_EQ(entry[1], 1);  // one entry counted
    CHECK(entry[2] >= 0);   // processing time = end elapsed - start elapsed
    CHECK(entry[2] < 1000000);
}
