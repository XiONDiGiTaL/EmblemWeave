// SPDX-License-Identifier: GPL-3.0-or-later
// HDR and colour math (hdr_math.h): the transfer functions (sRGB, PQ), the BT.709 <-> BT.2020 primaries, the game's tone-map curve
// with its shoulder extension, and the HDR10 and scRGB encodes. The tone-map parameters are one real frame's values, given as the
// exact float bit patterns the game holds.
#include <cmath>
#include <cstdint>
#include <cstring>

#include "test_framework.h"

#include "extras/hdr_math.h"

NXTEST_GROUP(hdr_math)

namespace {

namespace hdr = nxrt::extras;

float from_bits(uint32_t bits) {
    float f;
    std::memcpy(&f, &bits, sizeof f);
    return f;
}

// One frame's eval params, v[0..19], as the game holds them.
hdr::TonemapEval frame_params() {
    static const uint32_t kBits[20] = {
        0x3dc202ffu, 0x4128e5a4u, 0x3dc202ffu, 0x4128e5a4u, 0xc1cdc2eau, 0x411425f0u, 0x3f0e38e2u, 0x00000000u,
        0x3fb1c71bu, 0x32800000u, 0x3e3851ecu, 0x3e85a1ccu, 0x3f233333u, 0x400b6efcu, 0xbf119192u, 0x3eb9999au,
        0x3c783e10u, 0x3f783e10u, 0x3c783e10u, 0x3f783e10u,
    };
    hdr::TonemapEval e{};
    for (int i = 0; i < 20; ++i) e.v[i] = from_bits(kBits[i]);
    return e;
}

float curve_at(const hdr::TonemapEval& e, double x) { return hdr::tonemap_curve(e, static_cast<float>(x)); }

}  // namespace

// ---- transfer functions ----------------------------------------------------------------------------------------------------------

NXTEST(pq_known_values) {
    CHECK(std::fabs(hdr::pq_oetf(0.0f)) < 1e-6f);                       // formula gives ~7e-7 at 0 nits
    CHECK(std::fabs(hdr::pq_oetf(10000.0f) - 1.0f) < 1e-5f);
    CHECK(std::fabs(hdr::pq_oetf(100.0f) - 0.5081f) < 1e-3f);
    CHECK(std::fabs(hdr::pq_oetf(1000.0f) - 0.7518f) < 1e-3f);
    const float nits[] = {0.01f, 1.0f, 10.0f, 100.0f, 203.0f, 1000.0f, 4000.0f};
    for (float n : nits) {
        const float back = hdr::pq_eotf(hdr::pq_oetf(n));
        CHECK(std::fabs(back - n) <= 1e-3f * n);                         // within 0.1 percent
    }
}

NXTEST(srgb_known_values) {
    CHECK(hdr::srgb_oetf(0.0f) == 0.0f);
    CHECK(std::fabs(hdr::srgb_oetf(1.0f) - 1.0f) < 1e-6f);
    CHECK(std::fabs(hdr::srgb_oetf(0.18f) - 0.4613f) < 1e-3f);
    const float values[] = {0.0f, 0.001f, 0.0031308f, 0.01f, 0.18f, 0.5f, 1.0f, 1.5f};
    for (float v : values) {
        CHECK(std::fabs(hdr::srgb_eotf(hdr::srgb_oetf(v)) - v) < 1e-5f);
    }
    CHECK(hdr::srgb_oetf(-0.5f) == 0.0f);                                 // input clamped to >= 0
}

// ---- primaries -------------------------------------------------------------------------------------------------------------------

NXTEST(primaries_white_is_preserved_both_ways) {
    const float white[3] = {1.0f, 1.0f, 1.0f};
    float to2020[3], back[3];
    hdr::bt709_to_bt2020(white, to2020);
    for (int c = 0; c < 3; ++c) CHECK(std::fabs(to2020[c] - 1.0f) < 1e-3f);
    hdr::bt2020_to_bt709(white, back);
    for (int c = 0; c < 3; ++c) CHECK(std::fabs(back[c] - 1.0f) < 1e-3f);
}

NXTEST(primaries_round_trip) {
    const float in[3] = {0.2f, 0.5f, 0.8f};
    float wide[3], narrow[3];
    hdr::bt709_to_bt2020(in, wide);
    hdr::bt2020_to_bt709(wide, narrow);
    for (int c = 0; c < 3; ++c) CHECK(std::fabs(narrow[c] - in[c]) < 1e-3f);
}

// ---- tone curve ------------------------------------------------------------------------------------------------------------------

NXTEST(tonemap_real_frame_parameters) {
    const hdr::TonemapEval e = frame_params();
    CHECK(std::fabs(e.v[12] - 0.6375f) < 1e-3f);                         // A
    CHECK(std::fabs(e.v[13] - 2.1788f) < 1e-3f);                         // p
    CHECK(std::fabs(e.v[14] + 0.5687f) < 1e-3f);                         // q
    CHECK(std::fabs(e.v[15] - 0.3625f) < 1e-3f);                         // B
    const float t1 = e.v[10], t2 = e.v[11];
    // continuous at both thresholds
    CHECK(std::fabs(curve_at(e, t1 - 1e-5) - curve_at(e, t1 + 1e-5)) < 2e-3f);
    CHECK(std::fabs(curve_at(e, t2 - 1e-5) - curve_at(e, t2 + 1e-5)) < 2e-3f);
    // monotonic over 0..50
    bool monotonic = true;
    float prev = curve_at(e, 0.0);
    for (int i = 1; i <= 2000; ++i) {
        const float y = curve_at(e, 50.0 * i / 2000.0);
        if (y < prev) monotonic = false;
        prev = y;
    }
    CHECK(monotonic);
    CHECK(std::fabs(curve_at(e, 1e6) - 1.0f) < 1e-3f);                   // the asymptote is A + B = 1
}

NXTEST(extend_shoulder_moves_the_asymptote_and_keeps_value_and_slope) {
    const hdr::TonemapEval before = frame_params();
    hdr::TonemapEval e = before;
    const double t2 = before.v[11];
    const double h = 1e-4;
    const float value_before = curve_at(before, t2);
    const float slope_before = (curve_at(before, t2 + h) - curve_at(before, t2)) / static_cast<float>(h);

    CHECK(hdr::extend_shoulder(&e, 5.0f));
    const float value_after = curve_at(e, t2);
    const float slope_after = (curve_at(e, t2 + h) - curve_at(e, t2)) / static_cast<float>(h);
    CHECK(std::fabs(value_after - value_before) < 1e-5f);
    CHECK(std::fabs(slope_after - slope_before) <= 1e-2f * std::fabs(slope_before));
    CHECK(std::fabs(curve_at(e, 1e7) - 5.0f) < 1e-2f);

    bool monotonic = true;
    float prev = curve_at(e, 0.0);
    for (int i = 1; i <= 2000; ++i) {
        const float y = curve_at(e, 200.0 * i / 2000.0);
        if (y < prev) monotonic = false;
        prev = y;
    }
    CHECK(monotonic);

    for (int i = 0; i < 20; ++i) {
        if (i >= 12 && i <= 15) continue;                                // the four shoulder terms change
        CHECK(e.v[i] == before.v[i]);
    }
}

NXTEST(extend_shoulder_refuses_a_peak_at_or_below_the_value_at_t2) {
    const hdr::TonemapEval before = frame_params();
    hdr::TonemapEval e = before;
    // the value at t2 is about 0.3625, so a 0.3 asymptote is below it
    CHECK(!hdr::extend_shoulder(&e, 0.3f));
    CHECK(std::memcmp(e.v, before.v, sizeof e.v) == 0);                  // bit-identical
    CHECK(!hdr::extend_shoulder(&e, 0.3625f));
    CHECK(std::memcmp(e.v, before.v, sizeof e.v) == 0);
}

NXTEST(extend_shoulder_peak_one_rebuilds_the_original_asymptote) {
    // The plan expected false for peak_ratio 1.0. The stated rule is "refuse when peak_ratio <= value at t2 + 1e-4",
    // and the value at t2 is about 0.3625, so 1.0 is accepted. Its asymptote is A + B = 1.0 again, the original curve.
    hdr::TonemapEval e = frame_params();
    CHECK(hdr::extend_shoulder(&e, 1.0f));
    CHECK(std::fabs(curve_at(e, 1e7) - 1.0f) < 1e-3f);
    CHECK(std::fabs(curve_at(e, 1e6) - 1.0f) < 1e-3f);
}

// ---- peak ratio ------------------------------------------------------------------------------------------------------------------

NXTEST(peak_ratio_values) {
    CHECK(std::fabs(hdr::peak_ratio(1000.0f, 203.0f, 1.0f) - 4.926f) < 1e-3f);
    CHECK(hdr::peak_ratio(1000.0f, 203.0f, 0.0f) == 1.0f);
    CHECK(hdr::peak_ratio(1000.0f, 0.0f, 1.0f) == hdr::peak_ratio(1000.0f, 203.0f, 1.0f));
    CHECK(hdr::peak_ratio(1000.0f, 203.0f, 5.0f) == hdr::peak_ratio(1000.0f, 203.0f, 2.0f));
    CHECK(hdr::peak_ratio(100.0f, 203.0f, 1.0f) == 1.0f);                // peak below paper white: never below 1
}

// ---- HDR encodes ----------------------------------------------------------------------------------------------------------------

NXTEST(encode_hdr10_and_scrgb_clamps_and_scales) {
    const hdr::HdrOut o{};                                               // paper 203, peak 1000
    const float one[3] = {1.0f, 1.0f, 1.0f};
    const float hundred[3] = {100.0f, 100.0f, 100.0f};
    float out[3];

    hdr::encode_hdr10(one, o, out);
    for (int c = 0; c < 3; ++c) CHECK(std::fabs(out[c] - hdr::pq_oetf(203.0f)) < 2e-3f);

    hdr::encode_hdr10(hundred, o, out);                                  // 20300 nits clamps to the 1000 peak
    for (int c = 0; c < 3; ++c) CHECK(std::fabs(out[c] - hdr::pq_oetf(1000.0f)) < 1e-4f);

    hdr::encode_scrgb(one, o, out);
    for (int c = 0; c < 3; ++c) CHECK(std::fabs(out[c] - 2.5375f) < 1e-4f);

    hdr::encode_scrgb(hundred, o, out);                                  // capped at 1000 / 80
    for (int c = 0; c < 3; ++c) CHECK(std::fabs(out[c] - 12.5f) < 1e-4f);
}
