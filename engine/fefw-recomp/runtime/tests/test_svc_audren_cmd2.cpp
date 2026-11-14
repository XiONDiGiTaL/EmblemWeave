// SPDX-License-Identifier: GPL-3.0-or-later
// Effect DSP commands (Dsp/Command: biquad, multi-tap biquad, auxiliary buffer, compressor, delay, limiter, reverb, reverb 3d).
// Expected values are derived by hand from the C# arithmetic (the derivation is in the comment of each case); none is taken from a run of the port.
// Common setup: one CommandList of N samples; the command reads buffer 0 (or the input index it names) and writes buffer 1.
#include <cmath>
#include <cstdint>
#include <cstring>
#include <vector>

#include "../src/services/audren/Dsp/Command/AuxiliaryBufferCommand.h"
#include "../src/services/audren/Dsp/Command/BiquadFilterAndMixCommand.h"
#include "../src/services/audren/Dsp/Command/BiquadFilterCommand.h"
#include "../src/services/audren/Dsp/Command/CompressorCommand.h"
#include "../src/services/audren/Dsp/Command/DelayCommand.h"
#include "../src/services/audren/Dsp/Command/LimiterCommandVersion1.h"
#include "../src/services/audren/Dsp/Command/LimiterCommandVersion2.h"
#include "../src/services/audren/Dsp/Command/MultiTapBiquadFilterAndMixCommand.h"
#include "../src/services/audren/Dsp/Command/MultiTapBiquadFilterCommand.h"
#include "../src/services/audren/Dsp/Command/Reverb3dCommand.h"
#include "../src/services/audren/Dsp/Command/ReverbCommand.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_cmd2)

using namespace nxrt::audren;

namespace {

// `buffers` mix buffers of `sampleCount` floats; buffer i is GetBufferPointer(i).
struct Mix {
    std::vector<float> data;
    CommandList list;
    Mix(uint32_t sampleCount, uint32_t buffers)
        : data(static_cast<size_t>(sampleCount) * buffers, 0.0f), list(Span<float>(data), sampleCount, 48000, buffers, 0) {}
    float* buf(int32_t index) { return list.GetBufferPointer(index); }
};

CpuAddress Addr(const void* p) { return static_cast<CpuAddress>(reinterpret_cast<uintptr_t>(p)); }

template <size_t N>
void ExpectBuf(const float* got, const float (&want)[N]) {
    for (size_t i = 0; i < N; i++) {
        CHECK_EQ(got[i], want[i]);
    }
}

// Q14 fixed point helper for the effect parameters (value * 16384).
constexpr uint32_t Q14(float v) { return static_cast<uint32_t>(static_cast<int32_t>(v * 16384.0f)); }

}  // namespace

// Pass-through: numerator {1,0,0}, denominator {0,0}. DF2: output = input*1 + state0 (state0 stays 0), so output == input exactly.
// One-pole low-pass: numerator {0.5,0,0}, denominator {0.5,0}. Recurrence y[n] = 0.5 x[n] + 0.5 y[n-1] (state0 = output*b1).
// Impulse x = {1,0,0,0} gives y = {0.5, 0.25, 0.125, 0.0625} (all exact in float).
NXTEST(biquad_pass_through_and_one_pole) {
    {
        Mix m(4, 2);
        const float in[4] = {1.0f, -2.0f, 0.5f, 3.0f};
        std::memcpy(m.buf(0), in, sizeof in);

        BiquadFilterParameter2 p{};
        p.enable = true;
        p.numerator[0] = 1.0f;
        BiquadFilterState st[1] = {};

        BiquadFilterCommand cmd;
        cmd.Initialize(0, p, Span<BiquadFilterState>(st, 1), 0, 1, true, 1);
        cmd.Process(m.list);
        ExpectBuf(m.buf(1), in);
    }
    {
        Mix m(4, 2);
        const float in[4] = {1.0f, 0.0f, 0.0f, 0.0f};
        std::memcpy(m.buf(0), in, sizeof in);

        BiquadFilterParameter2 p{};
        p.enable = true;
        p.numerator[0] = 0.5f;
        p.denominator[0] = 0.5f;
        BiquadFilterState st[1] = {};

        BiquadFilterCommand cmd;
        cmd.Initialize(0, p, Span<BiquadFilterState>(st, 1), 0, 1, true, 1);
        cmd.Process(m.list);
        const float want[4] = {0.5f, 0.25f, 0.125f, 0.0625f};
        ExpectBuf(m.buf(1), want);
    }
}

// BiquadFilterAndMix (no ramp): out += nearbyint(filtered * Volume1). The output buffer starts at 10 (pass-through case) and at 0 (one-pole).
// Pass-through, input {128,0,0,0}, volume 0.5: out = {10 + nearbyint(64), 10, 10, 10} = {74, 10, 10, 10}.
// One-pole (a0 = 0.5, b1 = 0.5), input {128,0,0,0}, volume 1.0: filtered = {64, 32, 16, 8}, out = {64, 32, 16, 8}.
NXTEST(biquad_and_mix_adds_scaled_output) {
    {
        Mix m(4, 2);
        const float in[4] = {128.0f, 0.0f, 0.0f, 0.0f};
        std::memcpy(m.buf(0), in, sizeof in);
        const float start[4] = {10.0f, 10.0f, 10.0f, 10.0f};
        std::memcpy(m.buf(1), start, sizeof start);

        BiquadFilterParameter2 p{};
        p.enable = true;
        p.numerator[0] = 1.0f;
        BiquadFilterState st[1] = {};
        BiquadFilterState prev[1] = {};
        VoiceState vs[1] = {};

        BiquadFilterAndMixCommand cmd;
        cmd.Initialize(0.5f, 0.5f, 0, 1, 0, Span<VoiceState>(vs, 1), p, Span<BiquadFilterState>(st, 1), Span<BiquadFilterState>(prev, 1), true,
                       false, false, 1);
        cmd.Process(m.list);
        const float want[4] = {74.0f, 10.0f, 10.0f, 10.0f};
        ExpectBuf(m.buf(1), want);
    }
    {
        Mix m(4, 2);
        const float in[4] = {128.0f, 0.0f, 0.0f, 0.0f};
        std::memcpy(m.buf(0), in, sizeof in);

        BiquadFilterParameter2 p{};
        p.enable = true;
        p.numerator[0] = 0.5f;
        p.denominator[0] = 0.5f;
        BiquadFilterState st[1] = {};
        BiquadFilterState prev[1] = {};
        VoiceState vs[1] = {};

        BiquadFilterAndMixCommand cmd;
        cmd.Initialize(1.0f, 1.0f, 0, 1, 0, Span<VoiceState>(vs, 1), p, Span<BiquadFilterState>(st, 1), Span<BiquadFilterState>(prev, 1), true,
                       false, false, 1);
        cmd.Process(m.list);
        const float want[4] = {64.0f, 32.0f, 16.0f, 8.0f};
        ExpectBuf(m.buf(1), want);
    }
}

// Multi-tap, two stages: stage 0 = one-pole (0.5/0.5), stage 1 = one-pole (0.5/0.5) fed by stage 0.
// Impulse {1,0,0,0}: stage 0 = {0.5, 0.25, 0.125, 0.0625}; stage 1: z0 = 0.5*0.5 = 0.25, z1 = 0.5*0.25 + 0.5*0.25 = 0.25,
// z2 = 0.5*0.125 + 0.5*0.25 = 0.1875, z3 = 0.5*0.0625 + 0.5*0.1875 = 0.125. Single tap (stage 0 only): {0.5, 0.25, 0.125, 0.0625}.
NXTEST(multitap_biquad_two_taps_and_single_tap) {
    const float in[4] = {1.0f, 0.0f, 0.0f, 0.0f};
    BiquadFilterParameter2 taps[2] = {};
    for (auto& t : taps) {
        t.enable = true;
        t.numerator[0] = 0.5f;
        t.denominator[0] = 0.5f;
    }
    const bool init[2] = {false, false};
    {
        Mix m(4, 2);
        std::memcpy(m.buf(0), in, sizeof in);
        BiquadFilterState states[2] = {};

        MultiTapBiquadFilterCommand cmd;
        cmd.Initialize(0, Span<const BiquadFilterParameter2>(taps, 2), Span<BiquadFilterState>(states, 2), 0, 1, Span<const bool>(init, 2), 1);
        cmd.Process(m.list);
        const float want[4] = {0.25f, 0.25f, 0.1875f, 0.125f};
        ExpectBuf(m.buf(1), want);
    }
    {
        Mix m(4, 2);
        std::memcpy(m.buf(0), in, sizeof in);
        BiquadFilterState states[1] = {};

        MultiTapBiquadFilterCommand cmd;
        cmd.Initialize(0, Span<const BiquadFilterParameter2>(taps, 1), Span<BiquadFilterState>(states, 1), 0, 1, Span<const bool>(init, 1), 1);
        cmd.Process(m.list);
        const float want[4] = {0.5f, 0.25f, 0.125f, 0.0625f};
        ExpectBuf(m.buf(1), want);
    }
}

// MultiTapBiquadFilterAndMix with two one-pole stages, input {128,0,0,0}, volume 1.0, no ramp.
// Stage 0: {64, 32, 16, 8}. Stage 1: z0 = 32, z1 = 0.5*32 + 0.5*32 = 32, z2 = 0.5*16 + 0.5*32 = 24, z3 = 0.5*8 + 0.5*24 = 16.
// Output (starts at 0, += nearbyint(z)) = {32, 32, 24, 16}.
NXTEST(multitap_biquad_and_mix_two_taps) {
    Mix m(4, 2);
    const float in[4] = {128.0f, 0.0f, 0.0f, 0.0f};
    std::memcpy(m.buf(0), in, sizeof in);

    BiquadFilterParameter2 p0{};
    BiquadFilterParameter2 p1{};
    p0.enable = p1.enable = true;
    p0.numerator[0] = p1.numerator[0] = 0.5f;
    p0.denominator[0] = p1.denominator[0] = 0.5f;
    BiquadFilterState s0[1] = {};
    BiquadFilterState s1[1] = {};
    BiquadFilterState q0[1] = {};
    BiquadFilterState q1[1] = {};
    VoiceState vs[1] = {};

    MultiTapBiquadFilterAndMixCommand cmd;
    cmd.Initialize(1.0f, 1.0f, 0, 1, 0, Span<VoiceState>(vs, 1), p0, p1, Span<BiquadFilterState>(s0, 1), Span<BiquadFilterState>(s1, 1),
                   Span<BiquadFilterState>(q0, 1), Span<BiquadFilterState>(q1, 1), true, true, false, false, 1);
    cmd.Process(m.list);
    const float want[4] = {32.0f, 32.0f, 24.0f, 16.0f};
    ExpectBuf(m.buf(1), want);
}

// Auxiliary buffer (Ryujinx AuxiliaryBufferCommand). countMax = 8 ints. Input buffer 0 = {1,2,3,4}; the return ring (read side) holds {10,20,30,40,...}.
// Send side writes the input into the send ring at the send header's WriteOffset (+ the Initialize writeOffset, 0 here) and advances it by updateCount.
// Read side reads from the return ring at the return header's ReadOffset (+ the Initialize writeOffset: the C# passes WriteOffset as the read base),
// and advances the return ReadOffset by updateCount.
NXTEST(auxiliary_buffer_write_read_and_wrap) {
    using Info = AuxiliaryBufferHeader::AuxiliaryBufferInfo;
    alignas(8) int32_t sendRing[8] = {};
    alignas(8) int32_t returnRing[8] = {10, 20, 30, 40, 0, 0, 0, 0};

    // Case 1: no wrap. Send writes {1,2,3,4} at 0 -> WriteOffset 4. Read returns {10,20,30,40} -> ReadOffset 4.
    {
        Info sendInfo{};
        Info returnInfo{};
        AuxiliaryBufferAddresses addrs{};
        addrs.send_buffer_info = Addr(&sendInfo);
        addrs.return_buffer_info = Addr(&returnInfo);

        Mix m(4, 2);
        const float in[4] = {1.0f, 2.0f, 3.0f, 4.0f};
        std::memcpy(m.buf(0), in, sizeof in);

        AuxiliaryBufferCommand cmd;
        cmd.Initialize(0, 0, 1, addrs, true, 8, Addr(sendRing), Addr(returnRing), 4, 0, 1);
        cmd.Process(m.list);

        CHECK_EQ(sendRing[0], 1);
        CHECK_EQ(sendRing[3], 4);
        CHECK_EQ(sendInfo.WriteOffset, 4u);
        CHECK_EQ(returnInfo.ReadOffset, 4u);
        CHECK_EQ(sendInfo.TotalSampleCount, 0u);
        const float got[4] = {10.0f, 20.0f, 30.0f, 40.0f};
        ExpectBuf(m.buf(1), got);
        ExpectBuf(m.buf(0), in);   // float -> int -> float round trip of integer samples
    }
    // Case 2: send wrap. WriteOffset 6 -> writes go to ring[6], ring[7], ring[0], ring[1]; WriteOffset becomes (6 + 4) % 8 = 2.
    {
        for (auto& v : sendRing) v = 0;
        Info sendInfo{};
        sendInfo.WriteOffset = 6;
        Info returnInfo{};
        AuxiliaryBufferAddresses addrs{};
        addrs.send_buffer_info = Addr(&sendInfo);
        addrs.return_buffer_info = Addr(&returnInfo);

        Mix m(4, 2);
        const float in[4] = {1.0f, 2.0f, 3.0f, 4.0f};
        std::memcpy(m.buf(0), in, sizeof in);

        AuxiliaryBufferCommand cmd;
        cmd.Initialize(0, 0, 1, addrs, true, 8, Addr(sendRing), Addr(returnRing), 4, 0, 1);
        cmd.Process(m.list);

        const int32_t wantRing[8] = {3, 4, 0, 0, 0, 0, 1, 2};
        for (size_t i = 0; i < 8; i++) {
            CHECK_EQ(sendRing[i], wantRing[i]);
        }
        CHECK_EQ(sendInfo.WriteOffset, 2u);
    }
    // Case 3: read wrap. Return ReadOffset 6 over ring {10..80}: reads ring[6], ring[7], ring[0], ring[1] = {70,80,10,20}; ReadOffset -> 2.
    {
        alignas(8) const int32_t ring[8] = {10, 20, 30, 40, 50, 60, 70, 80};
        Info sendInfo{};
        Info returnInfo{};
        returnInfo.ReadOffset = 6;
        AuxiliaryBufferAddresses addrs{};
        addrs.send_buffer_info = Addr(&sendInfo);
        addrs.return_buffer_info = Addr(&returnInfo);

        Mix m(4, 2);
        std::memset(m.buf(0), 0, 4 * sizeof(float));

        AuxiliaryBufferCommand cmd;
        cmd.Initialize(0, 0, 1, addrs, true, 8, Addr(sendRing), Addr(ring), 4, 0, 1);
        cmd.Process(m.list);

        const float got[4] = {70.0f, 80.0f, 10.0f, 20.0f};
        ExpectBuf(m.buf(1), got);
        CHECK_EQ(returnInfo.ReadOffset, 2u);
    }
    // Case 4: disabled. Both headers' offsets and TotalSampleCount are reset (LostSampleCount is kept), and input is copied to output.
    {
        Info sendInfo{};
        sendInfo.ReadOffset = 3;
        sendInfo.WriteOffset = 5;
        sendInfo.LostSampleCount = 3;
        sendInfo.TotalSampleCount = 9;
        Info returnInfo{};
        returnInfo.ReadOffset = 2;
        returnInfo.WriteOffset = 7;
        returnInfo.LostSampleCount = 1;
        returnInfo.TotalSampleCount = 4;
        AuxiliaryBufferAddresses addrs{};
        addrs.send_buffer_info = Addr(&sendInfo);
        addrs.return_buffer_info = Addr(&returnInfo);

        Mix m(4, 2);
        const float in[4] = {1.0f, 2.0f, 3.0f, 4.0f};
        std::memcpy(m.buf(0), in, sizeof in);

        AuxiliaryBufferCommand cmd;
        cmd.Initialize(0, 0, 1, addrs, false, 8, Addr(sendRing), Addr(returnRing), 4, 0, 1);
        cmd.Process(m.list);

        CHECK_EQ(sendInfo.ReadOffset, 0u);
        CHECK_EQ(sendInfo.WriteOffset, 0u);
        CHECK_EQ(sendInfo.TotalSampleCount, 0u);
        CHECK_EQ(sendInfo.LostSampleCount, 3u);
        CHECK_EQ(returnInfo.ReadOffset, 0u);
        CHECK_EQ(returnInfo.WriteOffset, 0u);
        CHECK_EQ(returnInfo.TotalSampleCount, 0u);
        CHECK_EQ(returnInfo.LostSampleCount, 1u);
        ExpectBuf(m.buf(1), in);
    }
}

// Compressor, below threshold. Threshold -10 dB, ratio 2, output gain 0 dB, makeup off, input gain 1. Input 1000 on every sample.
// mean = (1000/32768)^2 ~ 0.00093; newMean = mean (alpha 1 from an EMA at 0); y = 10*log10(newMean) ~ -30 dB < Unknown10 (-11.5),
// so the gain branch is not taken: z = 1. Gain average stays 1 and OutputGain = 10^(0/20) = 1, so output = input exactly.
NXTEST(compressor_below_threshold_unchanged) {
    Mix m(4, 2);
    const float in[4] = {1000.0f, 1000.0f, 1000.0f, 1000.0f};
    std::memcpy(m.buf(0), in, sizeof in);

    CompressorParameter cp{};
    cp.input[0] = 0;
    cp.output[0] = 1;
    cp.channel_count_max = 6;
    cp.channel_count = 1;
    cp.sample_rate = 48000;
    cp.threshold = -10.0f;
    cp.ratio = 2.0f;
    cp.input_gain = 1.0f;
    cp.attack_coefficient = 0.5f;
    cp.release_coefficient = 0.5f;
    cp.output_gain = 0.0f;
    cp.status = UsageState::Invalid;

    CompressorParameter init = cp;
    CompressorState st(init);

    CompressorCommand cmd;
    cmd.Initialize(0, cp, Span<CompressorState>(&st, 1), Span<EffectResultState>(), true, 1);
    cmd.Process(m.list);
    ExpectBuf(m.buf(1), in);
}

// Limiter v1, delay line of 3 samples (delay_buffer_sample_count_min = 3). Attack = release = 1 makes the detector and gain averages
// follow their input exactly. Input at full scale (32767 -> 1.0 after /32767) on every sample. The output at sample n is the sample
// written 3 samples earlier (0 for the first 3), times 32767.
// Threshold 2.0 (never exceeded): gain 1 -> output {0, 0, 0, 32767, 32767, 32767}.
// Threshold 0.5: attenuation 0.5 from sample 0, compression gain average 1 -> 0.5 -> {0, 0, 0, 16383.5, 16383.5, 16383.5}.
namespace {
LimiterParameter LimiterParams(float threshold) {
    LimiterParameter lp{};
    lp.input[0] = 0;
    lp.output[0] = 1;
    lp.channel_count_max = 6;
    lp.channel_count = 1;
    lp.sample_rate = 48000;
    lp.attack_coefficient = 1.0f;
    lp.release_coefficient = 1.0f;
    lp.threshold = threshold;
    lp.input_gain = 1.0f;
    lp.output_gain = 1.0f;
    lp.delay_buffer_sample_count_min = 3;
    lp.delay_buffer_sample_count_max = 4;
    lp.status = UsageState::Invalid;
    return lp;
}
}  // namespace

NXTEST(limiter_v1_below_and_above_threshold) {
    const float full[6] = {32767.0f, 32767.0f, 32767.0f, 32767.0f, 32767.0f, 32767.0f};
    {
        Mix m(6, 2);
        std::memcpy(m.buf(0), full, sizeof full);
        LimiterParameter lp = LimiterParams(2.0f);
        LimiterState st(lp, 0);

        LimiterCommandVersion1 cmd;
        cmd.Initialize(0, lp, Span<LimiterState>(&st, 1), true, 0, 1);
        cmd.Process(m.list);
        const float want[6] = {0.0f, 0.0f, 0.0f, 32767.0f, 32767.0f, 32767.0f};
        ExpectBuf(m.buf(1), want);
    }
    {
        Mix m(6, 2);
        std::memcpy(m.buf(0), full, sizeof full);
        LimiterParameter lp = LimiterParams(0.5f);
        LimiterState st(lp, 0);

        LimiterCommandVersion1 cmd;
        cmd.Initialize(0, lp, Span<LimiterState>(&st, 1), true, 0, 1);
        cmd.Process(m.list);
        const float want[6] = {0.0f, 0.0f, 0.0f, 16383.5f, 16383.5f, 16383.5f};
        ExpectBuf(m.buf(1), want);
    }
}

// Limiter v2 (same arithmetic as v1, threshold 0.5) also fills the statistics: InputMax = max |input| = 1.0,
// CompressionGainMin = min gain = 0.5 (reset values: 0 and 1 before the first sample).
NXTEST(limiter_v2_statistics) {
    const float full[6] = {32767.0f, 32767.0f, 32767.0f, 32767.0f, 32767.0f, 32767.0f};
    Mix m(6, 2);
    std::memcpy(m.buf(0), full, sizeof full);
    LimiterParameter lp = LimiterParams(0.5f);
    lp.statistics_enabled = true;
    lp.statistics_reset = true;
    LimiterState st(lp, 0);
    EffectResultState res[1] = {};

    LimiterCommandVersion2 cmd;
    cmd.Initialize(0, lp, Span<LimiterState>(&st, 1), Span<EffectResultState>(res, 1), true, 0, 1);
    cmd.Process(m.list);

    const float want[6] = {0.0f, 0.0f, 0.0f, 16383.5f, 16383.5f, 16383.5f};
    ExpectBuf(m.buf(1), want);
    const LimiterStatistics* stats = reinterpret_cast<const LimiterStatistics*>(res[0].specific_data);
    CHECK_EQ(stats->input_max[0], 1.0f);
    CHECK_EQ(stats->compression_gain_min[0], 0.5f);
}

// Delay, mono. Sample rate 48000 (Q14 encoded) -> the DelayState uses 48 samples per ms. delay_time = 1 ms = 48 samples.
// DelayLine semantics: the cycle covers indices 0..lastSampleIndex-1 (lastSampleIndex = 47), so an input written at sample 0 comes back
// at sample 47 (= N-1 with N = 48). Command: out = (in*dry + delayed*out)/64 with dry 0, out 1, in gain 1, so the echo appears as-is.
// Input impulse 1.0 at sample 0 -> output 1.0 at sample 47 and 0 elsewhere (N = 96 samples).
// Feedback 0.5: the echo is written back with gain 0.5 (x64 scale), so a second echo of 0.5 appears at sample 94.
NXTEST(delay_impulse_emerges_after_n_minus_one_samples) {
    auto makeParams = [](uint32_t feedback) {
        DelayParameter dp{};
        dp.input[0] = 0;
        dp.output[0] = 1;
        dp.channel_count_max = 6;
        dp.channel_count = 1;
        dp.delay_time_max = 1;
        dp.delay_time = 1;
        dp.sample_rate = 48000u << 14;
        dp.in_gain = Q14(1.0f);
        dp.feedback_gain = feedback;
        dp.out_gain = Q14(1.0f);
        dp.dry_gain = 0;
        dp.channel_spread = 0;
        dp.low_pass_amount = 0;
        dp.status = UsageState::Invalid;
        return dp;
    };
    {
        Mix m(96, 2);
        float* in = m.buf(0);
        in[0] = 1.0f;
        DelayParameter dp = makeParams(0);
        DelayState st(dp, 0);

        DelayCommand cmd;
        cmd.Initialize(0, dp, Span<DelayState>(&st, 1), true, 0, 1, false);
        cmd.Process(m.list);

        const float* out = m.buf(1);
        for (uint32_t i = 0; i < 96; i++) {
            CHECK_EQ(out[i], i == 47 ? 1.0f : 0.0f);
        }
    }
    {
        Mix m(96, 2);
        float* in = m.buf(0);
        in[0] = 1.0f;
        DelayParameter dp = makeParams(Q14(0.5f));
        DelayState st(dp, 0);

        DelayCommand cmd;
        cmd.Initialize(0, dp, Span<DelayState>(&st, 1), true, 0, 1, false);
        cmd.Process(m.list);

        const float* out = m.buf(1);
        CHECK_EQ(out[47], 1.0f);
        CHECK_EQ(out[94], 0.5f);
        CHECK_EQ(out[0], 0.0f);
        CHECK_EQ(out[95], 0.0f);
    }
}

// Reverb, mono, sample rate 1000 Hz (Q14 encoded), so 1 ms = 1000 samples in the delay lines.
// Early tap 0 has time 0 ms -> EarlyDelayTime[0] = 1 sample. Impulse 1.0 (x64 = 64) written to the pre-delay line at sample 0.
// At sample 1 tap 0 reads it: out = 64 * EarlyGain[0] / 64 = 0.70 (earlyGainBase Room[0] = 0.70, early gain 1). Output at sample 0 = 0.
// The late path reads the pre-delay line 12.5 ms later (12500 samples), so output becomes non-zero at ~sample 12501 and stays bounded.
// Silence in gives silence out. The bound |out| < 10 is a loose bound: the early taps sum to <= 0.7 per sample and the feedback loop decays.
NXTEST(reverb_silence_and_impulse_tail) {
    ReverbParameter rp{};
    rp.input[0] = 0;
    rp.output[0] = 1;
    rp.channel_count_max = 6;
    rp.channel_count = 1;
    rp.sample_rate = static_cast<int32_t>(1000u << 14);
    rp.early_mode = ReverbEarlyMode::Room;
    rp.early_gain = static_cast<int32_t>(Q14(1.0f));
    rp.pre_delay_time = 0;
    rp.late_mode = ReverbLateMode::Room;
    rp.late_gain = static_cast<int32_t>(Q14(1.0f));
    rp.decay_time = static_cast<int32_t>(Q14(1.0f));
    rp.high_frequency_decay_ratio = static_cast<int32_t>(Q14(1.0f));
    rp.coloration = 0;
    rp.reverb_gain = static_cast<int32_t>(Q14(1.0f));
    rp.out_gain = static_cast<int32_t>(Q14(1.0f));
    rp.dry_gain = 0;
    rp.status = UsageState::Invalid;

    const uint32_t N = 40000;
    {
        Mix m(N, 2);
        ReverbParameter init = rp;
        ReverbState st(init, 0, false);

        ReverbCommand cmd;
        cmd.Initialize(0, rp, Span<ReverbState>(&st, 1), true, 0, 1, false, false);
        cmd.Process(m.list);

        const float* out = m.buf(1);
        for (uint32_t i = 0; i < N; i++) {
            CHECK_EQ(out[i], 0.0f);
        }
    }
    {
        Mix m(N, 2);
        m.buf(0)[0] = 1.0f;
        ReverbParameter init = rp;
        ReverbState st(init, 0, false);

        ReverbCommand cmd;
        cmd.Initialize(0, rp, Span<ReverbState>(&st, 1), true, 0, 1, false, false);
        cmd.Process(m.list);

        const float* out = m.buf(1);
        CHECK_EQ(out[0], 0.0f);
        CHECK_EQ(out[1], 0.70f);
        bool tailNonZero = false;
        bool bounded = true;
        for (uint32_t i = 0; i < N; i++) {
            if (!std::isfinite(out[i]) || std::fabs(out[i]) >= 10.0f) bounded = false;
            if (i >= 12500 && out[i] != 0.0f) tailNonZero = true;
        }
        CHECK(tailNonZero);
        CHECK(bounded);
    }
}

// Reverb 3d, mono, sample rate 48000 (raw Hz, the C# divides by 1000). Silence in gives silence out.
// Impulse 1.0 at sample 0: the output at sample 0 is 0 (every tap reads the empty line, the dry gain is 0), and the early taps and
// the feedback network bring non-zero samples later. Same loose bound check as the reverb case.
NXTEST(reverb3d_silence_and_impulse_bounded) {
    Reverb3dParameter r3{};
    r3.input[0] = 0;
    r3.output[0] = 1;
    r3.channel_count_max = 6;
    r3.channel_count = 1;
    r3.sample_rate = 48000;
    r3.room_hf = 0.0f;
    r3.hf_reference = 1000.0f;
    r3.decay_time = 1.0f;
    r3.hf_decay_ratio = 1.0f;
    r3.room_gain = 0.0f;
    r3.reflections_gain = 0.0f;
    r3.reverb_gain = 0.0f;
    r3.diffusion = 0.0f;
    r3.reflection_delay = 0.0f;
    r3.reverb_delay_time = 0.0f;
    r3.density = 100.0f;
    r3.dry_gain = 0.0f;
    r3.parameter_status = UsageState::Invalid;

    const uint32_t N = 4000;
    {
        Mix m(N, 2);
        Reverb3dParameter init = r3;
        Reverb3dState st(init, 0);

        Reverb3dCommand cmd;
        cmd.Initialize(0, r3, Span<Reverb3dState>(&st, 1), true, 0, 1, false);
        cmd.Process(m.list);

        const float* out = m.buf(1);
        for (uint32_t i = 0; i < N; i++) {
            CHECK_EQ(out[i], 0.0f);
        }
    }
    {
        Mix m(N, 2);
        m.buf(0)[0] = 1.0f;
        Reverb3dParameter init = r3;
        Reverb3dState st(init, 0);

        Reverb3dCommand cmd;
        cmd.Initialize(0, r3, Span<Reverb3dState>(&st, 1), true, 0, 1, false);
        cmd.Process(m.list);

        const float* out = m.buf(1);
        CHECK_EQ(out[0], 0.0f);
        bool anyNonZero = false;
        bool bounded = true;
        for (uint32_t i = 0; i < N; i++) {
            if (!std::isfinite(out[i]) || std::fabs(out[i]) >= 10.0f) bounded = false;
            if (out[i] != 0.0f) anyNonZero = true;
        }
        CHECK(anyNonZero);
        CHECK(bounded);
    }
}

// Disabled effects copy the input to the output (channel 0 index 0 -> output index 1), as the C# else branches do.
NXTEST(disabled_effects_copy_input) {
    const float in[4] = {3.0f, 4.0f, 5.0f, 6.0f};
    {
        Mix m(4, 2);
        std::memcpy(m.buf(0), in, sizeof in);
        CompressorParameter cp{};
        cp.input[0] = 0;
        cp.output[0] = 1;
        cp.channel_count_max = 6;
        cp.channel_count = 1;
        cp.status = UsageState::Invalid;
        CompressorParameter init = cp;
        CompressorState st(init);

        CompressorCommand cmd;
        cmd.Initialize(0, cp, Span<CompressorState>(&st, 1), Span<EffectResultState>(), false, 1);
        cmd.Process(m.list);
        ExpectBuf(m.buf(1), in);
    }
    {
        Mix m(4, 2);
        std::memcpy(m.buf(0), in, sizeof in);
        LimiterParameter lp = LimiterParams(1.0f);
        LimiterState st(lp, 0);

        LimiterCommandVersion1 cmd;
        cmd.Initialize(0, lp, Span<LimiterState>(&st, 1), false, 0, 1);
        cmd.Process(m.list);
        ExpectBuf(m.buf(1), in);
    }
    {
        Mix m(4, 2);
        std::memcpy(m.buf(0), in, sizeof in);
        DelayParameter dp{};
        dp.input[0] = 0;
        dp.output[0] = 1;
        dp.channel_count_max = 6;
        dp.channel_count = 1;
        dp.delay_time_max = 1;
        dp.delay_time = 1;
        dp.sample_rate = 48000u << 14;
        dp.status = UsageState::Invalid;
        DelayState st(dp, 0);

        DelayCommand cmd;
        cmd.Initialize(0, dp, Span<DelayState>(&st, 1), false, 0, 1, false);
        cmd.Process(m.list);
        ExpectBuf(m.buf(1), in);
    }
    {
        Mix m(4, 2);
        std::memcpy(m.buf(0), in, sizeof in);
        ReverbParameter rp{};
        rp.input[0] = 0;
        rp.output[0] = 1;
        rp.channel_count_max = 6;
        rp.channel_count = 1;
        rp.sample_rate = static_cast<int32_t>(1000u << 14);
        rp.status = UsageState::Invalid;
        ReverbParameter init = rp;
        ReverbState st(init, 0, false);

        ReverbCommand cmd;
        cmd.Initialize(0, rp, Span<ReverbState>(&st, 1), false, 0, 1, false, false);
        cmd.Process(m.list);
        ExpectBuf(m.buf(1), in);
    }
}
