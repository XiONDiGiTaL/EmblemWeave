// SPDX-License-Identifier: GPL-3.0-or-later
// Tests for the audio renderer decoders and state port (src/services/audren/Dsp: AdpcmHelper, PcmHelper, FixedPointHelper,
// FloatingPointHelper, DataSourceHelper, State/*). Expected values are NOT produced by running the port. They come from:
//   * hand-computed numbers (the comment next to each check shows the arithmetic),
//   * a reference DSP-ADPCM decoder written here from the format rule (RefAdpcm), independent of AdpcmHelper.cs,
//   * the C# control flow of DataSourceHelper.ProcessWaveBuffers, traced by hand (the trace is in the comment of each case).
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <vector>

#include "../src/services/audren/Dsp/AdpcmHelper.h"
#include "../src/services/audren/Dsp/DataSourceHelper.h"
#include "../src/services/audren/Dsp/FixedPointHelper.h"
#include "../src/services/audren/Dsp/FloatingPointHelper.h"
#include "../src/services/audren/Dsp/PcmHelper.h"
#include "../src/services/audren/Dsp/State/AuxiliaryBufferHeader.h"
#include "../src/services/audren/Dsp/State/CompressorState.h"
#include "../src/services/audren/Dsp/State/DelayState.h"
#include "../src/services/audren/Dsp/State/LimiterState.h"
#include "../src/services/audren/Dsp/State/Reverb3dState.h"
#include "../src/services/audren/Dsp/State/ReverbState.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_dec)

using namespace nxrt::audren;

namespace {

uint64_t Addr(const void* p) { return static_cast<uint64_t>(reinterpret_cast<uintptr_t>(p)); }

bool Near(double a, double b, double tol) { return std::fabs(a - b) <= tol; }

// Deterministic pseudo-random numbers (Numerical Recipes LCG).
struct Lcg {
    uint32_t s;
    uint32_t Next() {
        s = s * 1664525u + 1013904223u;
        return s >> 8;
    }
};

// Packs whole 14-sample frames into the DSP-ADPCM layout: 1 header byte (scale in the low nibble, coefficient index in the
// high nibble), then 14 nibbles, two per byte, first sample in the high nibble.
std::vector<uint8_t> PackAdpcm(const std::vector<int>& nibbles, const std::vector<int>& scales, const std::vector<int>& indices) {
    std::vector<uint8_t> out;
    size_t frames = nibbles.size() / 14;
    for (size_t f = 0; f < frames; f++) {
        out.push_back(static_cast<uint8_t>(scales[f] | (indices[f] << 4)));
        for (size_t k = 0; k < 14; k += 2) {
            out.push_back(static_cast<uint8_t>((nibbles[f * 14 + k] << 4) | nibbles[f * 14 + k + 1]));
        }
    }
    return out;
}

// Reference decoder written from the DSP-ADPCM rule (not from AdpcmHelper.cs):
//   n      = nibble as a signed 4-bit value (0x8..0xF are negative)
//   v      = clamp16(((n * 2^scale) * 2048 + c0 * h0 + c1 * h1 + 1024) >> 11)
//   h0 is the previous output, h1 the one before it; c0/c1 are the Q11 pair selected by the frame's coefficient index.
std::vector<int16_t> RefAdpcm(const std::vector<int>& nibbles, const std::vector<int>& scales, const std::vector<int>& indices, const std::vector<int16_t>& coef, int64_t h0 = 0, int64_t h1 = 0) {
    std::vector<int16_t> out;
    for (size_t s = 0; s < nibbles.size(); s++) {
        size_t f = s / 14;
        int64_t n = nibbles[s] >= 8 ? nibbles[s] - 16 : nibbles[s];
        int64_t c0 = coef[static_cast<size_t>(2 * indices[f])];
        int64_t c1 = coef[static_cast<size_t>(2 * indices[f] + 1)];
        int64_t v = ((n * (int64_t{1} << scales[f]) * 2048) + c0 * h0 + c1 * h1 + 1024) >> 11;
        if (v > 32767) v = 32767;
        if (v < -32768) v = -32768;
        out.push_back(static_cast<int16_t>(v));
        h1 = h0;
        h0 = v;
    }
    return out;
}

// Signed value of a nibble.
int Signed4(int nibble) { return nibble >= 8 ? nibble - 16 : nibble; }

// Random stream of `frames` full frames.
void RandomStream(Lcg& rng, int frames, std::vector<int>& nibbles, std::vector<int>& scales, std::vector<int>& indices, std::vector<int16_t>& coef) {
    nibbles.clear();
    scales.clear();
    indices.clear();
    coef.clear();
    for (int i = 0; i < frames * 14; i++) nibbles.push_back(static_cast<int>(rng.Next() & 0xF));
    for (int f = 0; f < frames; f++) {
        scales.push_back(static_cast<int>(rng.Next() % 16));
        indices.push_back(static_cast<int>(rng.Next() % 16));
    }
    for (int i = 0; i < 32; i++) coef.push_back(static_cast<int16_t>(static_cast<int>(rng.Next() % 4096) - 2048));
}

DecodingBehaviour Flags(DecodingBehaviour a, DecodingBehaviour b) {
    return static_cast<DecodingBehaviour>(static_cast<uint16_t>(a) | static_cast<uint16_t>(b));
}

DataSourceHelper::WaveBufferInformation MakeInfo(SampleFormat format, DecodingBehaviour behaviour) {
    DataSourceHelper::WaveBufferInformation info{};
    info.SourceSampleRate = 48000;
    info.Pitch = 1.0f;
    info.ChannelIndex = 0;
    info.ChannelCount = 1;
    info.DecodingBehaviour = behaviour;
    info.SrcQuality = SampleRateConversionQuality::Default;
    info.SampleFormat = format;
    return info;
}

}  // namespace

// ---------------------------------------------------------------------------------------------------------------------
// ADPCM sizes and offsets. 14 samples are 8 bytes (1 header byte + 14 nibbles).
// Oracle: bytes(N) = ceil(N/14) headers + ceil(N/2) nibble bytes. Byte offset of sample s: 8 * (s / 14), plus for r = s % 14
// nonzero the header byte and the nibble byte of sample r: 1 + r / 2.
// ---------------------------------------------------------------------------------------------------------------------

NXTEST(adpcm_data_size_and_offsets) {
    for (int32_t n = 0; n <= 60; n++) {
        uint32_t expected = n == 0 ? 0u : static_cast<uint32_t>((n + 13) / 14 + (n + 1) / 2);
        CHECK_EQ(AdpcmHelper::GetAdpcmDataSize(n), expected);
    }
    // Spot values: 14 samples = 8 bytes, 15 = 10, 28 = 16, 13 = 8.
    CHECK_EQ(AdpcmHelper::GetAdpcmDataSize(14), 8u);
    CHECK_EQ(AdpcmHelper::GetAdpcmDataSize(15), 10u);
    CHECK_EQ(AdpcmHelper::GetAdpcmDataSize(28), 16u);
    CHECK_EQ(AdpcmHelper::GetAdpcmDataSize(13), 8u);

    for (int32_t s = 0; s <= 60; s++) {
        int32_t r = s % 14;
        int32_t expected = 8 * (s / 14) + (r == 0 ? 0 : 1 + r / 2);
        CHECK_EQ(AdpcmHelper::GetAdpcmOffsetFromSampleOffset(s), expected);
    }

    CHECK_EQ(AdpcmHelper::GetNibblesFromSampleCount(0), 0);
    CHECK_EQ(AdpcmHelper::GetNibblesFromSampleCount(1), 3);   // 2 header nibbles + 1
    CHECK_EQ(AdpcmHelper::GetNibblesFromSampleCount(14), 16);  // one whole frame, no partial header
    CHECK_EQ(AdpcmHelper::GetNibblesFromSampleCount(15), 19);

    // NibbleToSample: nibble 2 is sample 0 of frame 0, nibble 18 is sample 14 (the first sample of frame 1).
    for (int32_t k = 0; k < 14; k++) {
        CHECK_EQ(AdpcmHelper::NibbleToSample(2 + k), k);
        CHECK_EQ(AdpcmHelper::NibbleToSample(18 + k), 14 + k);
    }
}

// Pure pass-through: coefficient index 0 has c0 = c1 = 0, so the output is exactly nibble * 2^scale.
// Expected: sample k = Signed4(P[k]) << scale, with P = 0,1,7,8,15,14,9,10,11,12,13,2,3,4.
NXTEST(adpcm_pure_passthrough_scales) {
    const std::vector<int> pattern = {0, 1, 7, 8, 15, 14, 9, 10, 11, 12, 13, 2, 3, 4};
    std::vector<int16_t> coef(32, 0);
    for (int scale = 0; scale <= 3; scale++) {
        std::vector<uint8_t> stream = PackAdpcm(pattern, {scale}, {0});
        CHECK_EQ(stream.size(), 8u);

        int16_t out[14] = {};
        AdpcmLoopContext ctx{};
        int32_t n = AdpcmHelper::Decode(Span<int16_t>(out, 14), Span<const uint8_t>(stream.data(), stream.size()), 0, 14, 0, 14, Span<const int16_t>(coef.data(), coef.size()), ctx);
        CHECK_EQ(n, 14);
        for (int k = 0; k < 14; k++) {
            CHECK_EQ(out[k], Signed4(pattern[static_cast<size_t>(k)]) * (1 << scale));
        }
        // Loop context after the frame: header byte, and the last two outputs as history.
        CHECK_EQ(ctx.pred_scale, scale);
        CHECK_EQ(ctx.history0, out[13]);
        CHECK_EQ(ctx.history1, out[12]);
    }
}

// Scale 15 with nibble 7 gives 229376 and nibble 8 gives -262144: both must saturate to int16.
NXTEST(adpcm_saturation) {
    const std::vector<int> nibbles = {7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8, 7, 8};
    std::vector<int16_t> coef(32, 0);
    std::vector<uint8_t> stream = PackAdpcm(nibbles, {15}, {0});
    int16_t out[14] = {};
    AdpcmLoopContext ctx{};
    AdpcmHelper::Decode(Span<int16_t>(out, 14), Span<const uint8_t>(stream.data(), stream.size()), 0, 14, 0, 14, Span<const int16_t>(coef.data(), coef.size()), ctx);
    for (int k = 0; k < 14; k++) {
        CHECK_EQ(out[k], (k % 2 == 0) ? 32767 : -32768);
    }
}

// Partial frames and end offsets. decodedCount = min(count, end - start - offset).
NXTEST(adpcm_partial_frame_and_end_offset) {
    const std::vector<int> pattern = {0, 1, 7, 8, 15, 14, 9, 10, 11, 12, 13, 2, 3, 4};
    std::vector<int16_t> coef(32, 0);
    std::vector<uint8_t> stream = PackAdpcm(pattern, {2}, {0});
    Span<const uint8_t> in(stream.data(), stream.size());
    Span<const int16_t> co(coef.data(), coef.size());

    // count 5 out of a 14-sample frame: sample-by-sample path, odd and even nibbles.
    int16_t out[14] = {};
    AdpcmLoopContext ctx{};
    CHECK_EQ(AdpcmHelper::Decode(Span<int16_t>(out, 14), in, 0, 14, 0, 5, co, ctx), 5);
    for (int k = 0; k < 5; k++) CHECK_EQ(out[k], Signed4(pattern[static_cast<size_t>(k)]) * 4);

    // end offset 10 limits the decode to 10 samples even when 14 are asked.
    int16_t out2[14] = {};
    AdpcmLoopContext ctx2{};
    CHECK_EQ(AdpcmHelper::Decode(Span<int16_t>(out2, 14), in, 0, 10, 0, 14, co, ctx2), 10);

    // Start in the middle of a frame: offset 3 with end 14 gives 11 samples, samples 3..13 of the frame.
    int16_t out3[14] = {};
    // The scale comes from the loop context, not from a header (no header is read mid-frame), so seed it with frame 0's header.
    AdpcmLoopContext ctx3{};
    ctx3.pred_scale = 2;
    CHECK_EQ(AdpcmHelper::Decode(Span<int16_t>(out3, 14), in, 0, 14, 3, 20, co, ctx3), 11);
    for (int k = 0; k < 11; k++) CHECK_EQ(out3[k], Signed4(pattern[static_cast<size_t>(3 + k)]) * 4);

    // Empty input and reversed offsets decode nothing.
    AdpcmLoopContext ctx4{};
    CHECK_EQ(AdpcmHelper::Decode(Span<int16_t>(out, 14), Span<const uint8_t>(), 0, 14, 0, 14, co, ctx4), 0);
    CHECK_EQ(AdpcmHelper::Decode(Span<int16_t>(out, 14), in, 8, 4, 0, 14, co, ctx4), 0);
}

// Coefficient index 1 = (c0 2048, c1 0): prediction = h0, so each output is nibble*2^scale plus the previous output.
// Hand check: nibbles 1,1,1 with scale 0 from zero history give 1, 2, 3. Then frame 2 (scale 0, nibble 1) carries 3 -> 4.
NXTEST(adpcm_history_carry_and_coefficient_index) {
    std::vector<int16_t> coef(32, 0);
    coef[2] = 2048;  // index 1, c0
    coef[3] = 0;     // index 1, c1
    std::vector<int> nibbles(28, 1);
    std::vector<int> scales = {0, 0};
    std::vector<int> indices = {1, 1};
    std::vector<uint8_t> stream = PackAdpcm(nibbles, scales, indices);
    Span<const uint8_t> in(stream.data(), stream.size());
    Span<const int16_t> co(coef.data(), coef.size());

    int16_t out[28] = {};
    AdpcmLoopContext ctx{};
    CHECK_EQ(AdpcmHelper::Decode(Span<int16_t>(out, 28), in, 0, 28, 0, 28, co, ctx), 28);
    for (int k = 0; k < 28; k++) CHECK_EQ(out[k], k + 1);
    CHECK_EQ(ctx.history0, 28);
    CHECK_EQ(ctx.history1, 27);

    // Same stream decoded in two calls: the second call starts at offset 14 and must continue from the carried history.
    int16_t out2[28] = {};
    AdpcmLoopContext ctx2{};
    CHECK_EQ(AdpcmHelper::Decode(Span<int16_t>(out2, 28), in, 0, 28, 0, 14, co, ctx2), 14);
    CHECK_EQ(AdpcmHelper::Decode(Span<int16_t>(out2 + 14, 14), in, 0, 28, 14, 14, co, ctx2), 14);
    for (int k = 0; k < 28; k++) CHECK_EQ(out2[k], k + 1);
}

// Random 3-frame streams with random scales (includes saturation), random coefficient indices and random Q11 coefficients.
// Decoded in one call and in 5-sample chunks; both must equal the reference.
NXTEST(adpcm_random_stream_matches_reference) {
    Lcg rng{12345u};
    for (int round = 0; round < 8; round++) {
        std::vector<int> nibbles, scales, indices;
        std::vector<int16_t> coef;
        RandomStream(rng, 3, nibbles, scales, indices, coef);
        std::vector<uint8_t> stream = PackAdpcm(nibbles, scales, indices);
        std::vector<int16_t> ref = RefAdpcm(nibbles, scales, indices, coef);
        Span<const uint8_t> in(stream.data(), stream.size());
        Span<const int16_t> co(coef.data(), coef.size());

        std::vector<int16_t> whole(42, 0);
        AdpcmLoopContext ctx{};
        CHECK_EQ(AdpcmHelper::Decode(Span<int16_t>(whole), in, 0, 42, 0, 42, co, ctx), 42);
        bool same = true;
        for (size_t k = 0; k < 42; k++) same = same && whole[k] == ref[k];
        CHECK_MSG(same, "one-call decode differs from the reference");

        std::vector<int16_t> chunked(42, 0);
        AdpcmLoopContext ctx2{};
        for (int pos = 0; pos < 42; pos += 5) {
            int32_t got = AdpcmHelper::Decode(Span<int16_t>(chunked.data() + pos, static_cast<size_t>(42 - pos)), in, 0, 42, pos, 5, co, ctx2);
            const int32_t expectedCount = std::min(5, 42 - pos);
            CHECK_EQ(got, expectedCount);
        }
        same = true;
        for (size_t k = 0; k < 42; k++) same = same && chunked[k] == ref[k];
        CHECK_MSG(same, "chunked decode differs from the reference");
    }
}

// Start in the middle of a frame with the history restored from a whole decode: samples 5..41 must match the reference.
// The loop context carries the frame-0 header (scale and coefficient index) because sample 5 is in frame 0.
NXTEST(adpcm_mid_frame_start_with_restored_history) {
    Lcg rng{777u};
    std::vector<int> nibbles, scales, indices;
    std::vector<int16_t> coef;
    RandomStream(rng, 3, nibbles, scales, indices, coef);
    std::vector<uint8_t> stream = PackAdpcm(nibbles, scales, indices);
    std::vector<int16_t> ref = RefAdpcm(nibbles, scales, indices, coef);
    Span<const uint8_t> in(stream.data(), stream.size());
    Span<const int16_t> co(coef.data(), coef.size());

    AdpcmLoopContext ctx{};
    ctx.pred_scale = static_cast<int16_t>(scales[0] | (indices[0] << 4));
    ctx.history0 = ref[4];
    ctx.history1 = ref[3];

    int16_t out[37] = {};
    CHECK_EQ(AdpcmHelper::Decode(Span<int16_t>(out, 37), in, 5, 42, 0, 37, co, ctx), 37);
    bool same = true;
    for (int k = 0; k < 37; k++) same = same && out[k] == ref[static_cast<size_t>(5 + k)];
    CHECK_MSG(same, "mid-frame decode differs from the reference");
}

// ---------------------------------------------------------------------------------------------------------------------
// PCM helpers
// ---------------------------------------------------------------------------------------------------------------------

NXTEST(pcm_decode_interleaved_int16_selects_channel) {
    // Stereo, 4 frames: L = 10, 11, 12, 13; R = -10, -11, -12, -13.
    const int16_t interleaved[8] = {10, -10, 11, -11, 12, -12, 13, -13};
    int16_t out[4] = {};
    CHECK_EQ(PcmHelper::Decode(Span<int16_t>(out, 4), Span<const int16_t>(interleaved, 8), 0, 4, 1, 2), 4);
    CHECK_EQ(out[0], -10);
    CHECK_EQ(out[3], -13);
    CHECK_EQ(PcmHelper::Decode(Span<int16_t>(out, 4), Span<const int16_t>(interleaved, 8), 0, 4, 0, 2), 4);
    CHECK_EQ(out[0], 10);
    CHECK_EQ(out[3], 13);
    // Reversed offsets or empty input decode nothing.
    CHECK_EQ(PcmHelper::Decode(Span<int16_t>(out, 4), Span<const int16_t>(interleaved, 8), 4, 0, 0, 2), 0);
    CHECK_EQ(PcmHelper::Decode(Span<int16_t>(out, 4), Span<const int16_t>(), 0, 4, 0, 2), 0);
}

// Float input is scaled by 32767 and truncated toward zero, then saturated.
// 0.5 -> 16383.5 -> 16383; 1.0 -> 32767; 2.0 -> 65534 -> 32767; -0.5 -> -16383; -1.0 -> -32767; -2.0 -> -32768.
NXTEST(pcm_decode_float_input_converts_with_saturation) {
    const float interleaved[6] = {0.5f, -0.5f, 1.0f, -1.0f, 2.0f, -2.0f};
    int16_t ch0[3] = {};
    int16_t ch1[3] = {};
    CHECK_EQ(PcmHelper::Decode(Span<int16_t>(ch0, 3), Span<const float>(interleaved, 6), 0, 3, 0, 2), 3);
    CHECK_EQ(PcmHelper::Decode(Span<int16_t>(ch1, 3), Span<const float>(interleaved, 6), 0, 3, 1, 2), 3);
    CHECK_EQ(ch0[0], 16383);
    CHECK_EQ(ch0[1], 32767);
    CHECK_EQ(ch0[2], 32767);
    CHECK_EQ(ch1[0], -16383);
    CHECK_EQ(ch1[1], -32767);
    CHECK_EQ(ch1[2], -32768);
}

NXTEST(pcm_saturate_edges) {
    CHECK_EQ(PcmHelper::Saturate(32767.0f), 32767);
    CHECK_EQ(PcmHelper::Saturate(32767.9f), 32767);
    CHECK_EQ(PcmHelper::Saturate(32768.0f), 32767);
    CHECK_EQ(PcmHelper::Saturate(-32768.0f), -32768);
    CHECK_EQ(PcmHelper::Saturate(-32768.5f), -32768);
    CHECK_EQ(PcmHelper::Saturate(-40000.0f), -32768);
    CHECK_EQ(PcmHelper::Saturate(0.9f), 0);
    CHECK_EQ(PcmHelper::Saturate(-0.9f), 0);
    CHECK_EQ(PcmHelper::Saturate(-32767.5f), -32767);
}

// int16 -> float -> int16 must come back within one step for every sample value. Exact for 0 and 32767 (1.0 * 32767).
NXTEST(pcm_convert_round_trip) {
    CHECK_EQ(PcmHelper::ConvertSampleToPcmInt16(PcmHelper::ConvertSampleToPcmFloat(0)), 0);
    CHECK_EQ(PcmHelper::ConvertSampleToPcmInt16(PcmHelper::ConvertSampleToPcmFloat(32767)), 32767);
    CHECK_EQ(PcmHelper::ConvertSampleToPcmFloat(32767), 1.0f);
    int bad = 0;
    for (int s = -32768; s <= 32767; s++) {
        int back = PcmHelper::ConvertSampleToPcmInt16(PcmHelper::ConvertSampleToPcmFloat(static_cast<int16_t>(s)));
        if (std::abs(back - s) > 1) bad++;
    }
    CHECK_EQ(bad, 0);
}

NXTEST(pcm_convert_to_8_24_32) {
    const int16_t in[4] = {0x1234, -256, 0x7FFF, -32768};
    int8_t o8[4] = {};
    PcmHelper::ConvertSampleToPcm8(Span<int8_t>(o8, 4), Span<const int16_t>(in, 4));
    CHECK_EQ(o8[0], 0x12);
    CHECK_EQ(o8[1], -1);
    CHECK_EQ(o8[2], 0x7F);
    CHECK_EQ(o8[3], -128);

    const int16_t in2[2] = {0x1234, -2};
    uint8_t o24[6] = {};
    PcmHelper::ConvertSampleToPcm24(Span<uint8_t>(o24, 6), Span<const int16_t>(in2, 2));
    // 0x1234 -> 00 34 12 (low byte in the middle), -2 = 0xFFFE -> 00 FE FF.
    CHECK_EQ(o24[0], 0x00);
    CHECK_EQ(o24[1], 0x34);
    CHECK_EQ(o24[2], 0x12);
    CHECK_EQ(o24[3], 0x00);
    CHECK_EQ(o24[4], 0xFE);
    CHECK_EQ(o24[5], 0xFF);

    int32_t o32[2] = {};
    PcmHelper::ConvertSampleToPcm32(Span<int32_t>(o32, 2), Span<const int16_t>(in2, 2));
    CHECK_EQ(o32[0], 0x12340000);
    CHECK_EQ(o32[1], -131072);
}

NXTEST(pcm_offsets_and_buffer_sizes) {
    CHECK_EQ(PcmHelper::GetBufferOffset<int16_t>(3, 2, 2), static_cast<uint64_t>(20));  // 2 bytes * 2 channels * (3 + 2)
    CHECK_EQ(PcmHelper::GetBufferOffset<float>(0, 4, 6), static_cast<uint64_t>(96));    // 4 * 6 * 4
    CHECK_EQ(PcmHelper::GetCountToDecode(0, 10, 4, 100), 6);
    CHECK_EQ(PcmHelper::GetCountToDecode(0, 10, 0, 3), 3);
    CHECK_EQ(PcmHelper::GetBufferSize<float>(0, 10, 4, 100), 24);
    CHECK_EQ(PcmHelper::GetBufferSize<int16_t>(5, 2, 0, 10), 0);  // end < start
}

// ---------------------------------------------------------------------------------------------------------------------
// Fixed point and floating point helpers
// ---------------------------------------------------------------------------------------------------------------------

NXTEST(fixed_point_conversions) {
    CHECK_EQ(FixedPointHelper::ToFixed(1.0f, 11), 2048);
    CHECK_EQ(FixedPointHelper::ToFixed(-0.5f, 15), -16384);
    CHECK(Near(FixedPointHelper::ToFloat(16384, 14), 1.0, 1e-9));
    CHECK(Near(FixedPointHelper::ConvertFloat(2048.0f, 11), 1.0, 1e-9));
    CHECK_EQ(FixedPointHelper::ToInt(-1, 14), -1);       // arithmetic shift rounds toward minus infinity
    CHECK_EQ(FixedPointHelper::ToInt(-16385, 14), -2);
    CHECK_EQ(FixedPointHelper::RoundUpAndToInt(1024, 11), 1);
    CHECK_EQ(FixedPointHelper::RoundUpAndToInt(1023, 11), 0);
    CHECK_EQ(FixedPointHelper::RoundUpAndToInt(-1024, 11), 0);
    CHECK_EQ(FixedPointHelper::RoundUpAndToInt(-1025, 11), -1);
}

NXTEST(floating_point_helpers) {
    CHECK_EQ(FloatingPointHelper::Pow10(0.5f), 1.0f);
    CHECK_EQ(FloatingPointHelper::Pow10(-5.3f), 0.0f);
    CHECK(Near(FloatingPointHelper::Pow10(-1.0f), 0.1, 1e-6));
    CHECK(Near(FloatingPointHelper::DecibelToLinear(0.0f), 1.0, 1e-7));
    CHECK(Near(FloatingPointHelper::DecibelToLinear(-20.0f), 0.1, 1e-6));
    CHECK(Near(FloatingPointHelper::Log10(0.0f), -10.0, 1e-6));  // clamped to 1e-10
    CHECK(Near(FloatingPointHelper::Log10(100.0f), 2.0, 1e-6));
    // MathF.Round rounds half to even.
    CHECK_EQ(FloatingPointHelper::RoundDown(2.5f), 2.0f);
    CHECK_EQ(FloatingPointHelper::RoundDown(3.5f), 4.0f);
    CHECK_EQ(FloatingPointHelper::RoundUp(2.5f), 2.0f);
    CHECK(Near(FloatingPointHelper::Cos(0.0f), 1.0, 1e-6));
    CHECK(Near(FloatingPointHelper::Sin(90.0f), 1.0, 1e-6));
    CHECK(Near(FloatingPointHelper::Cos(90.0f), 0.0, 1e-6));
    CHECK(Near(FloatingPointHelper::DegreesToRadians(180.0f), 3.14159265, 1e-6));
    // Mean square of normalised samples: {32768, 0} -> {1, 0} -> 0.5; {16384, -16384} -> {0.5, -0.5} -> 0.25.
    const float a[2] = {32768.0f, 0.0f};
    const float b[2] = {16384.0f, -16384.0f};
    CHECK(Near(FloatingPointHelper::MeanSquare(Span<const float>(a, 2)), 0.5, 1e-6));
    CHECK(Near(FloatingPointHelper::MeanSquare(Span<const float>(b, 2)), 0.25, 1e-6));
}

// ---------------------------------------------------------------------------------------------------------------------
// DataSourceHelper: conversions and channel remapping
// ---------------------------------------------------------------------------------------------------------------------

NXTEST(data_source_float_int_conversion) {
    const int32_t in[3] = {16384, -3, 0};
    float out[3] = {};
    DataSourceHelper::ToFloat(Span<float>(out, 3), Span<const int32_t>(in, 3), 3);
    CHECK_EQ(out[0], 16384.0f);
    CHECK_EQ(out[1], -3.0f);
    CHECK_EQ(out[2], 0.0f);

    const float fin[3] = {2.7f, -2.7f, 0.5f};
    int32_t iout[3] = {};
    DataSourceHelper::ToInt(Span<int32_t>(iout, 3), Span<const float>(fin, 3), 3);
    CHECK_EQ(iout[0], 2);   // truncation toward zero
    CHECK_EQ(iout[1], -2);
    CHECK_EQ(iout[2], 0);
}

NXTEST(data_source_channel_remap_round_trip) {
    uint16_t idx[6] = {0, 1, 2, 3, 4, 5};
    // Legacy order to channel-resource order for 5.1: back L/R move to 4/5, front centre and LFE to 2/3.
    DataSourceHelper::RemapLegacyChannelEffectMappingToChannelResourceMapping(false, Span<uint16_t>(idx, 6), 6);
    const uint16_t expected[6] = {0, 1, 4, 5, 2, 3};
    for (int k = 0; k < 6; k++) CHECK_EQ(idx[k], expected[k]);
    DataSourceHelper::RemapChannelResourceMappingToLegacy(true, Span<uint16_t>(idx, 6), 6);
    for (int k = 0; k < 6; k++) CHECK_EQ(idx[k], static_cast<uint16_t>(k));
    // Other channel counts and supported devices are untouched.
    DataSourceHelper::RemapLegacyChannelEffectMappingToChannelResourceMapping(false, Span<uint16_t>(idx, 6), 2);
    CHECK_EQ(idx[2], 2);
}

// ---------------------------------------------------------------------------------------------------------------------
// DataSourceHelper.ProcessWaveBuffers. Every case uses SkipPitchAndSampleRateConversion, so the resampler (not ported
// here) is not called. In that path the output is the raw int16 values as floats (C#: `outputBuffer[j] = tempBuffer[j]`),
// without the 1/32768 scale that the resampler applies. Target sample rate equals the source rate, pitch 1.
// ---------------------------------------------------------------------------------------------------------------------

// Two PCM16 mono buffers, A = 6 samples (end of buffer 0, not end of work), B = 4 samples (end of work).
// Trace (C# DataSourceHelper.cs 61-252): sampleCount 10, buffer 0 fully decoded (y 0..6), MarkEnd (consumed 1, index 1),
// buffer 1 decoded (y 6..10), MarkEnd (consumed 2, index 2), IsEndOfStream -> playedSampleCount reset to 0.
NXTEST(data_source_pcm16_sequence_skip_pitch) {
    const int16_t a[6] = {100, 200, 300, 400, 500, 600};
    const int16_t b[4] = {-1, -2, -3, -4};
    WaveBuffer wb[4] = {};
    wb[0].buffer = Addr(a);
    wb[0].buffer_size = sizeof(a);
    wb[0].end_sample_offset = 6;
    wb[1].buffer = Addr(b);
    wb[1].buffer_size = sizeof(b);
    wb[1].end_sample_offset = 4;
    wb[1].is_end_of_stream = true;
    Span<WaveBuffer> wbs(wb, 4);

    VoiceState voice{};
    voice.is_wave_buffer_valid[0] = true;
    voice.is_wave_buffer_valid[1] = true;

    float out[16] = {};
    auto info = MakeInfo(SampleFormat::PcmInt16, DecodingBehaviour::SkipPitchAndSampleRateConversion);
    DataSourceHelper::ProcessWaveBuffers(Span<float>(out, 16), info, wbs, voice, 48000, 10);

    for (int k = 0; k < 6; k++) CHECK_EQ(out[k], static_cast<float>(a[k]));
    for (int k = 0; k < 4; k++) CHECK_EQ(out[6 + k], static_cast<float>(b[k]));
    for (int k = 10; k < 16; k++) CHECK_EQ(out[k], 0.0f);

    CHECK_EQ(voice.wave_buffer_consumed, 2u);
    CHECK_EQ(voice.wave_buffer_index, 2u);
    CHECK_EQ(voice.is_wave_buffer_valid[0], false);
    CHECK_EQ(voice.is_wave_buffer_valid[1], false);
    CHECK_EQ(voice.played_sample_count, static_cast<uint64_t>(0));  // reset by the end-of-stream buffer
    CHECK_EQ(voice.offset, 0);
    CHECK_EQ(voice.loop_count, 0);
}

// Same buffers but buffer 1 is not end of stream: played_sample_count keeps the 10 samples.
NXTEST(data_source_played_sample_count_without_end_of_stream) {
    const int16_t a[6] = {100, 200, 300, 400, 500, 600};
    const int16_t b[4] = {-1, -2, -3, -4};
    WaveBuffer wb[4] = {};
    wb[0].buffer = Addr(a);
    wb[0].buffer_size = sizeof(a);
    wb[0].end_sample_offset = 6;
    wb[1].buffer = Addr(b);
    wb[1].buffer_size = sizeof(b);
    wb[1].end_sample_offset = 4;
    Span<WaveBuffer> wbs(wb, 4);

    VoiceState voice{};
    voice.is_wave_buffer_valid[0] = true;
    voice.is_wave_buffer_valid[1] = true;

    float out[16] = {};
    auto info = MakeInfo(SampleFormat::PcmInt16, DecodingBehaviour::SkipPitchAndSampleRateConversion);
    DataSourceHelper::ProcessWaveBuffers(Span<float>(out, 16), info, wbs, voice, 48000, 10);
    CHECK_EQ(voice.played_sample_count, static_cast<uint64_t>(10));
    CHECK_EQ(voice.wave_buffer_consumed, 2u);
}

// Only buffer 0 is valid and holds 6 samples; 10 are requested. Trace: after buffer 0, index 1 is not valid, so the voice
// starves (C# line 97-101): 6 samples written, the rest of the output stays untouched, index 1, consumed 1.
NXTEST(data_source_starvation_stops_at_invalid_buffer) {
    const int16_t a[6] = {100, 200, 300, 400, 500, 600};
    WaveBuffer wb[4] = {};
    wb[0].buffer = Addr(a);
    wb[0].buffer_size = sizeof(a);
    wb[0].end_sample_offset = 6;
    Span<WaveBuffer> wbs(wb, 4);

    VoiceState voice{};
    voice.is_wave_buffer_valid[0] = true;

    float out[10] = {};
    auto info = MakeInfo(SampleFormat::PcmInt16, DecodingBehaviour::SkipPitchAndSampleRateConversion);
    DataSourceHelper::ProcessWaveBuffers(Span<float>(out, 10), info, wbs, voice, 48000, 10);
    for (int k = 0; k < 6; k++) CHECK_EQ(out[k], static_cast<float>(a[k]));
    for (int k = 6; k < 10; k++) CHECK_EQ(out[k], 0.0f);
    CHECK_EQ(voice.wave_buffer_consumed, 1u);
    CHECK_EQ(voice.wave_buffer_index, 1u);
    CHECK_EQ(voice.played_sample_count, static_cast<uint64_t>(6));
    CHECK_EQ(voice.is_wave_buffer_valid[0], false);
}

// Looping buffer of 3 samples with loop_count 1 (plays 2 times in total). Trace: pass 1 decodes 3 samples and loops
// (voice.loop_count 1, not greater than 1, so no MarkEnd). Pass 2 decodes 3 samples, voice.loop_count becomes 2 > 1,
// MarkEnd (consumed 1, index 1, voice.loop_count reset to 0). Then buffer 1 is invalid: starve after 6 samples.
NXTEST(data_source_looping_buffer_with_loop_count) {
    const int16_t a[3] = {1, 2, 3};
    WaveBuffer wb[4] = {};
    wb[0].buffer = Addr(a);
    wb[0].buffer_size = sizeof(a);
    wb[0].end_sample_offset = 3;
    wb[0].looping = true;
    wb[0].loop_count = 1;
    Span<WaveBuffer> wbs(wb, 4);

    VoiceState voice{};
    voice.is_wave_buffer_valid[0] = true;

    float out[8] = {};
    auto info = MakeInfo(SampleFormat::PcmInt16, DecodingBehaviour::SkipPitchAndSampleRateConversion);
    DataSourceHelper::ProcessWaveBuffers(Span<float>(out, 8), info, wbs, voice, 48000, 8);
    const float expected[8] = {1, 2, 3, 1, 2, 3, 0, 0};
    for (int k = 0; k < 8; k++) CHECK_EQ(out[k], expected[k]);
    CHECK_EQ(voice.wave_buffer_consumed, 1u);
    CHECK_EQ(voice.wave_buffer_index, 1u);
    CHECK_EQ(voice.played_sample_count, static_cast<uint64_t>(6));
    CHECK_EQ(voice.loop_count, 0);
}

// Loop region [2, 5) inside a 6-sample buffer, loop_count 2 (3 passes after the first full pass), 12 samples requested.
// Trace: pass 1 plays 0..5 (voice.loop_count 0, full range). Passes 2 and 3 play the region 2..4 (3 samples each).
// Pass 3 raises voice.loop_count to 3 > 2 and marks the buffer consumed. Output: a0..a5, a2 a3 a4, a2 a3 a4.
// With PlayedSampleCountResetWhenLooping, playedSampleCount is reset after every loop pass, so the final value is 0.
NXTEST(data_source_loop_region_with_reset_flag) {
    const int16_t a[6] = {10, 20, 30, 40, 50, 60};
    WaveBuffer wb[4] = {};
    wb[0].buffer = Addr(a);
    wb[0].buffer_size = sizeof(a);
    wb[0].start_sample_offset = 0;
    wb[0].end_sample_offset = 6;
    wb[0].looping = true;
    wb[0].loop_count = 2;
    wb[0].loop_start_sample_offset = 2;
    wb[0].loop_end_sample_offset = 5;
    Span<WaveBuffer> wbs(wb, 4);

    const float expected[12] = {10, 20, 30, 40, 50, 60, 30, 40, 50, 30, 40, 50};

    {
        VoiceState voice{};
        voice.is_wave_buffer_valid[0] = true;
        float out[12] = {};
        auto info = MakeInfo(SampleFormat::PcmInt16, Flags(DecodingBehaviour::SkipPitchAndSampleRateConversion, DecodingBehaviour::PlayedSampleCountResetWhenLooping));
        DataSourceHelper::ProcessWaveBuffers(Span<float>(out, 12), info, wbs, voice, 48000, 12);
        for (int k = 0; k < 12; k++) CHECK_EQ(out[k], expected[k]);
        CHECK_EQ(voice.wave_buffer_consumed, 1u);
        CHECK_EQ(voice.played_sample_count, static_cast<uint64_t>(0));
        CHECK_EQ(voice.loop_count, 0);
    }
    {
        // Without the flag the count keeps growing: 6 + 3 + 3 = 12.
        VoiceState voice{};
        voice.is_wave_buffer_valid[0] = true;
        float out[12] = {};
        auto info = MakeInfo(SampleFormat::PcmInt16, DecodingBehaviour::SkipPitchAndSampleRateConversion);
        DataSourceHelper::ProcessWaveBuffers(Span<float>(out, 12), info, wbs, voice, 48000, 12);
        for (int k = 0; k < 12; k++) CHECK_EQ(out[k], expected[k]);
        CHECK_EQ(voice.played_sample_count, static_cast<uint64_t>(12));
    }
}

// ADPCM through ProcessWaveBuffers: coefficients come from guest memory (ExtraParameter), the loop context from the wave
// buffer's Context address (read only when offset == 0). Two frames with coefficient index 1 (c0 = 2048, so the output
// accumulates). The context sets history0 = 1000, so the expected output is the reference decode started from h0 = 1000.
NXTEST(data_source_adpcm_reads_coefficients_and_context) {
    Lcg rng{4242u};
    std::vector<int> nibbles, scales, indices;
    std::vector<int16_t> unusedCoef;
    RandomStream(rng, 2, nibbles, scales, indices, unusedCoef);
    indices = {1, 1};
    std::vector<int16_t> coef(32, 0);
    coef[2] = 2048;
    std::vector<uint8_t> stream = PackAdpcm(nibbles, scales, indices);

    AdpcmLoopContext context{};
    context.pred_scale = static_cast<int16_t>(scales[0] | (indices[0] << 4));
    context.history0 = 1000;
    context.history1 = 0;

    WaveBuffer wb[4] = {};
    wb[0].buffer = Addr(stream.data());
    wb[0].buffer_size = static_cast<uint32_t>(stream.size());
    wb[0].end_sample_offset = 28;
    wb[0].context = Addr(&context);
    wb[0].context_size = sizeof(context);
    Span<WaveBuffer> wbs(wb, 4);

    VoiceState voice{};
    voice.is_wave_buffer_valid[0] = true;

    auto info = MakeInfo(SampleFormat::Adpcm, DecodingBehaviour::SkipPitchAndSampleRateConversion);
    info.ExtraParameter = Addr(coef.data());
    info.ExtraParameterSize = coef.size() * sizeof(int16_t);

    float out[28] = {};
    DataSourceHelper::ProcessWaveBuffers(Span<float>(out, 28), info, wbs, voice, 48000, 28);

    std::vector<int16_t> ref = RefAdpcm(nibbles, scales, indices, coef, 1000, 0);
    for (int k = 0; k < 28; k++) CHECK_EQ(out[k], static_cast<float>(ref[static_cast<size_t>(k)]));
    CHECK_EQ(voice.wave_buffer_consumed, 1u);
    CHECK_EQ(voice.loop_context.history0, ref[27]);
    CHECK_EQ(voice.loop_context.history1, ref[26]);
}

// ---------------------------------------------------------------------------------------------------------------------
// State classes
// ---------------------------------------------------------------------------------------------------------------------

// AuxiliaryBufferInfo is read and written at guest addresses. Reset writes 8 bytes at offset 0 (ReadOffset and WriteOffset)
// and 4 bytes at offset 0xC; LostSampleCount (offset 8) is never reset (C# comment: "never reset, since REV10").
NXTEST(state_aux_buffer_info_reset_keeps_lost_count) {
    uint32_t words[16] = {};
    CpuAddress buf = Addr(words);
    using Info = AuxiliaryBufferHeader::AuxiliaryBufferInfo;
    Info::SetReadOffset(buf, 5);
    Info::SetWriteOffset(buf, 7);
    Info::SetLostSampleCount(buf, 9);
    Info::SetTotalSampleCount(buf, 11);
    CHECK_EQ(words[0], 5u);
    CHECK_EQ(words[1], 7u);
    CHECK_EQ(words[2], 9u);
    CHECK_EQ(words[3], 11u);
    CHECK_EQ(Info::GetReadOffset(buf), 5u);
    CHECK_EQ(Info::GetWriteOffset(buf), 7u);
    CHECK_EQ(Info::GetLostSampleCount(buf), 9u);
    CHECK_EQ(Info::GetTotalSampleCount(buf), 11u);

    Info::Reset(buf);
    CHECK_EQ(words[0], 0u);
    CHECK_EQ(words[1], 0u);
    CHECK_EQ(words[2], 9u);
    CHECK_EQ(words[3], 0u);
    CHECK_EQ(sizeof(AuxiliaryBufferHeader), static_cast<size_t>(0x80));
    CHECK_EQ(sizeof(Info), static_cast<size_t>(0x40));
}

// Compressor: threshold -10, ratio 2 (inverse 0.5), attack 0.5, makeup disabled, output gain 0.
// Expected: gain reduction (1 - 0.5) / 6, Unknown10 = -11.5, Unknown14 = -8.5, alpha 0.5, makeup 0, output 10^0 = 1.
NXTEST(state_compressor_from_parameter) {
    CompressorParameter p{};
    p.threshold = -10.0f;
    p.ratio = 2.0f;
    p.attack_coefficient = 0.5f;
    p.output_gain = 0.0f;
    p.makeup_gain_enabled = false;
    CompressorState s(p);
    CHECK(Near(s.CompressorGainReduction, 0.5 / 6.0, 1e-7));
    CHECK(Near(s.Unknown10, -11.5, 1e-6));
    CHECK(Near(s.Unknown14, -8.5, 1e-6));
    CHECK(Near(s.PreviousCompressionEmaAlpha, 0.5, 1e-7));
    CHECK(Near(s.MakeupGain, 0.0, 1e-7));
    CHECK(Near(s.OutputGain, 1.0, 1e-6));
}
