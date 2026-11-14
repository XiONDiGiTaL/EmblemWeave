// SPDX-License-Identifier: GPL-3.0-or-later
// Tests for the S3c ports: ResamplerHelper, BiquadFilterHelper, the delay lines and ExponentialMovingAverage.
// Expected values are derived by hand from the C# arithmetic (the derivation is in each comment), never from the port.
#include <algorithm>
#include <cmath>
#include <vector>

#include "../src/services/audren/Dsp/BiquadFilterHelper.h"
#include "../src/services/audren/Dsp/Effect/DecayDelay.h"
#include "../src/services/audren/Dsp/Effect/DelayLine.h"
#include "../src/services/audren/Dsp/Effect/DelayLineReverb3d.h"
#include "../src/services/audren/Dsp/Effect/ExponentialMovingAverage.h"
#include "../src/services/audren/Dsp/Effect/IDelayLine.h"
#include "../src/services/audren/Dsp/ResamplerHelper.h"
#include "../src/services/audren/Dsp/UpsamplerHelper.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_res)

using namespace nxrt::audren;

namespace {

constexpr float kSentinel = -999.0f;

// Low quality picks raw samples: the output is the sample value itself (no /32768 scaling in the C#).
void RunResample(SampleRateConversionQuality q, const std::vector<int16_t>& in, float ratio, float& fraction, int32_t count,
                 std::vector<float>& out) {
    ResamplerHelper::Resample(Span<float>(out), Span<const int16_t>(in), ratio, fraction, count, q, false);
}

}  // namespace

// ---- ResamplerHelper ----

// Low quality, ratio 1.0: every step advances one input sample and reads the sample at the current index, so the output
// is the input, in raw int16 units.
NXTEST(low_quality_ratio_one_is_identity_in_raw_units) {
    std::vector<int16_t> in = {100, -200, 300, 32767, -32768, 5, 7, 9};
    std::vector<float> out(8, kSentinel);
    float fraction = 0.0f;
    RunResample(SampleRateConversionQuality::Low, in, 1.0f, fraction, 8, out);
    for (size_t i = 0; i < in.size(); i++) {
        CHECK_EQ(out[i], static_cast<float>(in[i]));
    }
    CHECK_EQ(fraction, 0.0f);
}

// Low quality picks input[floor(k * ratio)] for output k (the index advances by trunc of the carried sum).
// Ratio 0.75: floor(0, 0.75, 1.5, 2.25, 3, 3.75, 4.5, 5.25) = 0 0 1 2 3 3 4 5.
// Ratio 1.5:  floor(0, 1.5, 3, 4.5, 6, 7.5, 9, 10.5)       = 0 1 3 4 6 7 9 10.
NXTEST(low_quality_ramp_picks_floor_of_k_times_ratio) {
    std::vector<int16_t> ramp(32);
    for (size_t i = 0; i < ramp.size(); i++) ramp[i] = static_cast<int16_t>(i);

    const float ratio075[8] = {0, 0, 1, 2, 3, 3, 4, 5};
    const float ratio15[8] = {0, 1, 3, 4, 6, 7, 9, 10};

    std::vector<float> out(8, kSentinel);
    float fraction = 0.0f;
    RunResample(SampleRateConversionQuality::Low, ramp, 0.75f, fraction, 8, out);
    for (size_t k = 0; k < 8; k++) CHECK_EQ(out[k], ratio075[k]);

    out.assign(8, kSentinel);
    fraction = 0.0f;
    RunResample(SampleRateConversionQuality::Low, ramp, 1.5f, fraction, 8, out);
    for (size_t k = 0; k < 8; k++) CHECK_EQ(out[k], ratio15[k]);
}

// A DC input of 1000 stays exactly 1000 with low quality (every tap is one sample) at all five ratios.
NXTEST(low_quality_dc_stays_exact) {
    const float ratios[5] = {0.5f, 1.0f, 1.5f, 32000.0f / 48000.0f, 48000.0f / 32000.0f};
    std::vector<int16_t> dc(64, 1000);
    for (float ratio : ratios) {
        std::vector<float> out(16, kSentinel);
        float fraction = 0.0f;
        RunResample(SampleRateConversionQuality::Low, dc, ratio, fraction, 16, out);
        for (float v : out) CHECK_EQ(v, 1000.0f);
    }
}

// Default quality DC gain is the sum of the 4 taps of the phase used. Phases come from fraction * 128:
//   ratio 1.0 (lut1, phase 0 always):  row sum 32635 -> 1000 * 32635 / 32768 = 995.94 -> 996
//   ratio 0.5 (lut1, phases 0 and 64 alternate): row 64 sum 32984 -> 1006.59 -> 1007; row 0 gives 996 again.
//   ratio 1.5 (lut0, phases 0 and 64 alternate): row sums 32751 -> 999.42 and 32736 -> 999.02, both 999.
// Default lut1 has a per-phase DC gain of 0.995 to 1.007, so the 1e-3 tolerance cannot hold for ratios <= 1.
NXTEST(default_quality_dc_follows_table_row_sums) {
    std::vector<int16_t> dc(64, 1000);
    std::vector<float> out(8, kSentinel);
    float fraction = 0.0f;

    RunResample(SampleRateConversionQuality::Default, dc, 1.0f, fraction, 8, out);
    for (float v : out) CHECK_EQ(v, 996.0f);

    out.assign(8, kSentinel);
    fraction = 0.0f;
    RunResample(SampleRateConversionQuality::Default, dc, 0.5f, fraction, 8, out);
    for (size_t i = 0; i < 8; i++) CHECK_EQ(out[i], (i % 2 == 0) ? 996.0f : 1007.0f);

    out.assign(8, kSentinel);
    fraction = 0.0f;
    RunResample(SampleRateConversionQuality::Default, dc, 1.5f, fraction, 8, out);
    for (float v : out) CHECK_EQ(v, 999.0f);

    // The other two ratios use phases that are not hand-tabulated here: check the DC level within 1%.
    const float other[2] = {32000.0f / 48000.0f, 48000.0f / 32000.0f};
    for (float ratio : other) {
        out.assign(16, kSentinel);
        fraction = 0.0f;
        RunResample(SampleRateConversionQuality::Default, dc, ratio, fraction, 16, out);
        for (float v : out) CHECK(std::fabs(v - 1000.0f) <= 10.0f);
    }
}

// High quality DC gain, same method (8 taps):
//   ratio 1.0 (lut1 row 0 sum 32735 -> 998.99 -> 999); ratio 0.5 (row 0 -> 999, row 64 sum 32745 -> 999.30 -> 999).
//   ratio 1.5 (lut0 row 0 sum 32772 -> 1000.12 -> 1000; row 64 sum 32768 -> 1000).
NXTEST(high_quality_dc_follows_table_row_sums) {
    std::vector<int16_t> dc(64, 1000);
    std::vector<float> out(8, kSentinel);
    float fraction = 0.0f;

    RunResample(SampleRateConversionQuality::High, dc, 1.0f, fraction, 8, out);
    for (float v : out) CHECK_EQ(v, 999.0f);

    out.assign(8, kSentinel);
    fraction = 0.0f;
    RunResample(SampleRateConversionQuality::High, dc, 0.5f, fraction, 8, out);
    for (float v : out) CHECK_EQ(v, 999.0f);

    out.assign(8, kSentinel);
    fraction = 0.0f;
    RunResample(SampleRateConversionQuality::High, dc, 1.5f, fraction, 8, out);
    for (float v : out) CHECK_EQ(v, 1000.0f);

    const float other[2] = {32000.0f / 48000.0f, 48000.0f / 32000.0f};
    for (float ratio : other) {
        out.assign(16, kSentinel);
        fraction = 0.0f;
        RunResample(SampleRateConversionQuality::High, dc, ratio, fraction, 16, out);
        for (float v : out) CHECK(std::fabs(v - 1000.0f) <= 10.0f);
    }
}

// The fraction after N outputs is (f0 + N * ratio) minus its integer part, and the input index advances by the integer part.
//   ratio 1.5, f0 0,    N 3:  3 * 1.5 = 4.5  -> 0.5
//   ratio 1.5, f0 0,    N 10: 15.0           -> 0.0
//   ratio 0.5, f0 0,    N 5:  2.5            -> 0.5
//   ratio 0.75, f0 0,   N 6:  4.5            -> 0.5
//   ratio 0.5, f0 0.25, N 2:  1.25           -> 0.25
NXTEST(fraction_carry_after_n_outputs) {
    struct Case {
        float ratio;
        float f0;
        int32_t n;
        float expected;
    };
    const Case cases[5] = {
        {1.5f, 0.0f, 3, 0.5f}, {1.5f, 0.0f, 10, 0.0f}, {0.5f, 0.0f, 5, 0.5f}, {0.75f, 0.0f, 6, 0.5f}, {0.5f, 0.25f, 2, 0.25f},
    };
    const SampleRateConversionQuality qualities[3] = {
        SampleRateConversionQuality::Low, SampleRateConversionQuality::Default, SampleRateConversionQuality::High};
    std::vector<int16_t> in(64, 0);
    for (SampleRateConversionQuality q : qualities) {
        for (const Case& c : cases) {
            std::vector<float> out(static_cast<size_t>(c.n), kSentinel);
            float fraction = c.f0;
            RunResample(q, in, c.ratio, fraction, c.n, out);
            CHECK_EQ(fraction, c.expected);
        }
    }
}

// Only sampleCount outputs are written; the rest of the buffer is untouched, and sampleCount 0 writes nothing.
NXTEST(sample_count_limits_writes) {
    std::vector<int16_t> dc(64, 1000);
    std::vector<float> out(10, kSentinel);
    float fraction = 0.25f;
    RunResample(SampleRateConversionQuality::Low, dc, 1.0f, fraction, 4, out);
    for (size_t i = 4; i < out.size(); i++) CHECK_EQ(out[i], kSentinel);
    CHECK_EQ(fraction, 0.25f);

    fraction = 0.25f;
    RunResample(SampleRateConversionQuality::Default, dc, 1.5f, fraction, 0, out);
    CHECK_EQ(fraction, 0.25f);
    for (size_t i = 0; i < 4; i++) CHECK_EQ(out[i], 1000.0f);
}

// 1 kHz sine at 48 kHz (48 samples per period, amplitude 10000), resampled by 1.5 (to 32 kHz):
//   amplitude is unchanged (gain about 1), and the half period is 24 / 1.5 = 16 output samples.
// Input sample n is round(10000 * sin(2 pi n / 48)). Output k reads input around 1.5 k.
NXTEST(sine_1khz_48k_resampled_by_1_5_keeps_amplitude_and_crossings) {
    constexpr double kPi = 3.14159265358979323846;
    std::vector<int16_t> sine(480);
    for (size_t n = 0; n < sine.size(); n++) {
        sine[n] = static_cast<int16_t>(std::lround(10000.0 * std::sin(2.0 * kPi * static_cast<double>(n) / 48.0)));
    }

    const SampleRateConversionQuality qualities[2] = {SampleRateConversionQuality::Default, SampleRateConversionQuality::High};
    for (SampleRateConversionQuality q : qualities) {
        std::vector<float> out(300, kSentinel);
        float fraction = 0.0f;
        RunResample(q, sine, 1.5f, fraction, 300, out);

        float peak = 0.0f;
        for (size_t k = 40; k < 260; k++) peak = std::max(peak, std::fabs(out[k]));
        CHECK(peak >= 9500.0f && peak <= 10500.0f);

        std::vector<size_t> crossings;
        for (size_t k = 41; k < 260; k++) {
            bool prevPos = out[k - 1] >= 0.0f;
            bool curPos = out[k] >= 0.0f;
            if (prevPos != curPos) crossings.push_back(k);
        }
        CHECK(crossings.size() >= 10);
        for (size_t i = 1; i < crossings.size(); i++) {
            long gap = static_cast<long>(crossings[i]) - static_cast<long>(crossings[i - 1]);
            CHECK(gap >= 15 && gap <= 17);
        }
    }
}

// ---- BiquadFilterHelper ----

// Pass-through (b0 = 1, the rest 0): the direct form 2 output is the input. The direct form 1 mix adds round(output * 1.0),
// which is exact for integer inputs.
NXTEST(biquad_pass_through_is_identity) {
    BiquadFilterParameter2 p{};
    p.enable = true;
    p.numerator[0] = 1.0f;

    const std::vector<float> in = {0.5f, -1.25f, 3.0f, 0.0f, 7.0f, -7.0f, 100.0f, 1.0f};
    std::vector<float> out(in.size(), 0.0f);
    BiquadFilterState s{};
    BiquadFilterHelper::ProcessBiquadFilter(p, s, Span<float>(out), Span<const float>(in), static_cast<uint32_t>(in.size()));
    for (size_t i = 0; i < in.size(); i++) CHECK_EQ(out[i], in[i]);

    const std::vector<float> ints = {3.0f, -4.0f, 5.0f, 6.0f, -7.0f, 8.0f, 9.0f, 10.0f};
    std::vector<float> mixed(ints.size(), 0.0f);
    BiquadFilterState s2{};
    BiquadFilterHelper::ProcessBiquadFilterAndMix(p, s2, Span<float>(mixed), Span<const float>(ints),
                                                  static_cast<uint32_t>(ints.size()), 1.0f);
    for (size_t i = 0; i < ints.size(); i++) CHECK_EQ(mixed[i], ints[i]);
}

// Fixed point, C# format: coefficient * 2^14, truncated by (int). 0.7f * 16384 = 11468.8 -> 11468; 0.3f * 16384 = 4915.2 -> 4915.
// Then the converted filter is y[n] = a x[n] + b y[n-1] with a = 11468 / 16384, b = 4915 / 16384 (exact in float).
// Step input of ones from zero state: y[n] = a (1 - b^(n+1)) / (1 - b), the closed form.
NXTEST(biquad_fixed_point_one_pole_step_response) {
    BiquadFilterParameter2 p{};
    p.numerator[0] = 0.7f;
    p.denominator[0] = 0.3f;

    BiquadFilterParameter1 q = BiquadFilterHelper::ToBiquadFilterParameter1(p);
    CHECK_EQ(q.numerator[0], static_cast<int16_t>(11468));
    CHECK_EQ(q.numerator[1], static_cast<int16_t>(0));
    CHECK_EQ(q.numerator[2], static_cast<int16_t>(0));
    CHECK_EQ(q.denominator[0], static_cast<int16_t>(4915));
    CHECK_EQ(q.denominator[1], static_cast<int16_t>(0));

    BiquadFilterParameter2 back = BiquadFilterHelper::ToBiquadFilterParameter2(q);
    CHECK_EQ(back.numerator[0], 11468.0f / 16384.0f);
    CHECK_EQ(back.denominator[0], 4915.0f / 16384.0f);

    const double a = 11468.0 / 16384.0;
    const double b = 4915.0 / 16384.0;
    const std::vector<float> ones(5, 1.0f);
    std::vector<float> out(5, 0.0f);
    BiquadFilterState s{};
    BiquadFilterHelper::ProcessBiquadFilter(back, s, Span<float>(out), Span<const float>(ones), 5);
    for (size_t n = 0; n < 5; n++) {
        double expected = a * (1.0 - std::pow(b, static_cast<double>(n + 1))) / (1.0 - b);
        CHECK(std::fabs(static_cast<double>(out[n]) - expected) < 1e-6);
    }
}

// Two calls with the state kept between them give the same samples as one long call (same operations per sample).
NXTEST(biquad_state_carries_between_calls) {
    BiquadFilterParameter2 p{};
    p.numerator[0] = 0.2f;
    p.numerator[1] = 0.3f;
    p.numerator[2] = 0.1f;
    p.denominator[0] = 0.5f;
    p.denominator[1] = -0.25f;

    std::vector<float> in(10);
    for (size_t n = 0; n < in.size(); n++) in[n] = static_cast<float>(static_cast<int>((n * 37) % 11) - 5);

    std::vector<float> whole(10, 0.0f);
    BiquadFilterState sWhole{};
    BiquadFilterHelper::ProcessBiquadFilter(p, sWhole, Span<float>(whole), Span<const float>(in), 10);

    std::vector<float> split(10, 0.0f);
    BiquadFilterState sSplit{};
    BiquadFilterHelper::ProcessBiquadFilter(p, sSplit, Span<float>(split).Slice(0, 4), Span<const float>(in).Slice(0, 4), 4);
    BiquadFilterHelper::ProcessBiquadFilter(p, sSplit, Span<float>(split).Slice(4, 6), Span<const float>(in).Slice(4, 6), 6);

    for (size_t n = 0; n < 10; n++) CHECK_EQ(split[n], whole[n]);
    CHECK_EQ(sSplit.state0, sWhole.state0);
    CHECK_EQ(sSplit.state1, sWhole.state1);
}

// ---- Delay lines ----

// DelayLine(1000 Hz, 0.010 s): SampleCountMax = round(10) = 10. C# ConfigureDelay sets the last index to N - 1 = 9 and Update wraps
// as soon as the write index reaches it, so the cycle is 9 slots: a sample comes back 9 Update calls later.
// With inputs x_k = k + 1: output k is x_(k-9) for k >= 9, and 0 before (zeroed buffer).
NXTEST(delay_line_returns_input_nine_updates_later_for_count_ten) {
    DelayLine dl(1000, 0.010f);
    CHECK_EQ(dl.SampleCountMax(), 10u);
    CHECK_EQ(dl.CurrentSampleCount(), 10u);

    const float expected[15] = {0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6};
    for (int k = 0; k < 15; k++) {
        CHECK_EQ(dl.Update(static_cast<float>(k + 1)), expected[k]);
    }
}

// SetDelay(0.005 s) gives N = 5, last index 4; the same rule gives a delay of 4: output k is x_(k-4) for k >= 4.
NXTEST(delay_line_set_delay_shortens_the_loop) {
    DelayLine dl(1000, 0.010f);
    dl.SetDelay(0.005f);
    CHECK_EQ(dl.CurrentSampleCount(), 5u);

    const float expected[9] = {0, 0, 0, 0, 1, 2, 3, 4, 5};
    for (int k = 0; k < 9; k++) {
        CHECK_EQ(dl.Update(static_cast<float>(k + 1)), expected[k]);
    }
}

// DelayLine3d(1000 Hz, 0.010 s): SampleCountMax = 10, CurrentSampleCount = 10. Its first write goes to slot 10 (outside the
// read cycle 0..9), so x_0 is never read; the read index runs one ahead of the write index, so output k is x_(k-9) for k >= 10.
NXTEST(delay_line_3d_drops_first_sample_and_delays_by_nine) {
    DelayLine3d dl(1000, 0.010f);
    CHECK_EQ(dl.SampleCountMax(), 10u);
    CHECK_EQ(dl.CurrentSampleCount(), 10u);

    const float expected[15] = {0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 3, 4, 5, 6};
    for (int k = 0; k < 15; k++) {
        CHECK_EQ(dl.Update(static_cast<float>(k + 1)), expected[k]);
    }
}

// ExponentialMovingAverage: mean += alpha (value - mean). alpha 0.5, value 1: 0 -> 0.5 -> 0.75.
NXTEST(exponential_moving_average_steps) {
    ExponentialMovingAverage m;
    CHECK_EQ(m.Update(1.0f, 0.5f), 0.5f);
    CHECK_EQ(m.Update(1.0f, 0.5f), 0.75f);
    CHECK_EQ(m.Read(), 0.75f);
}

// DecayDelay first output: the delay line holds zeros, so processed = value = 1, the wrapped line returns 0 and the result is
// 0 + 1 * 0.5 = 0.5.
NXTEST(decay_delay_first_output_is_value_times_decay) {
    DelayLine dl(1000, 0.010f);
    DecayDelay dd(dl);
    dd.SetDecayRate(0.5f);
    CHECK_EQ(dd.Update(1.0f), 0.5f);
}

// ---- UpsamplerHelper (scale 6: 40 input samples -> 240 outputs; the phase 0 output is the input 10 samples back) ----

// Phase 0 outputs history[9]. After n NextInput calls history[19] is the newest input and history[9] is the input 10 back,
// i.e. x(n-11). With x_j = j + 1 the phase 0 output of the m-th input (m from 0) is 0 for m < 10 and m - 9 from m = 10.
NXTEST(upsampler_phase_zero_is_input_ten_back) {
    std::vector<float> in(40);
    for (size_t j = 0; j < in.size(); j++) in[j] = static_cast<float>(j + 1);
    std::vector<float> out(240, kSentinel);
    UpsamplerBufferState state{};
    UpsamplerHelper::Upsample(Span<float>(out), Span<const float>(in), 240, 40, state);
    CHECK(state.Initialized);
    CHECK_EQ(state.Scale, 6.0f);
    for (size_t m = 0; m < 40; m++) {
        float expected = m >= 10 ? static_cast<float>(m - 9) : 0.0f;
        CHECK_EQ(out[6 * m], expected);
    }
}

// Once the 20-sample history is full of a DC input of 1, every phase's windowed-sinc bank sums to about 1 (DC gain).
NXTEST(upsampler_dc_gain_is_about_one) {
    std::vector<float> in(40, 1.0f);
    std::vector<float> out(240, kSentinel);
    UpsamplerBufferState state{};
    UpsamplerHelper::Upsample(Span<float>(out), Span<const float>(in), 240, 40, state);
    for (size_t i = 130; i < out.size(); i++) CHECK(std::fabs(out[i] - 1.0f) <= 0.01f);
}
