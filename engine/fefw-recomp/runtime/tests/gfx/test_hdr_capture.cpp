// SPDX-License-Identifier: GPL-3.0-or-later
// HDR capture decoder (hdr_capture.h): the 10-bit PQ words in both channel orders, IEEE half decode, the 8-bit sRGB path, the
// brightness fractions, the PFM writer (read back from disk) and the argument checks.
#include <cmath>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>
#include <vector>

#include "test_framework.h"

#include "extras/hdr_capture.h"
#include "extras/hdr_math.h"

NXTEST_GROUP(hdr_capture)

namespace {

namespace hdr = nxrt::extras;
using hdr::CaptureFormat;

// 10-bit PQ code of an absolute luminance (BT.2020 channel value).
uint32_t pq_code(float nits) { return static_cast<uint32_t>(std::lround(hdr::pq_oetf(nits) * 1023.0f)); }

void put_u32(std::vector<uint8_t>& buf, size_t pixel, uint32_t word) {
    for (int b = 0; b < 4; ++b) buf[pixel * 4 + b] = static_cast<uint8_t>((word >> (8 * b)) & 0xFFu);
}

void put_u16(std::vector<uint8_t>& buf, size_t offset, uint16_t v) {
    buf[offset] = static_cast<uint8_t>(v & 0xFFu);
    buf[offset + 1] = static_cast<uint8_t>(v >> 8);
}

// Result of decoding a single 1x1 pixel.
struct One {
    bool ok = false;
    float lin[3] = {0, 0, 0};
    uint8_t sdr[4] = {0, 0, 0, 0};
    uint8_t map[4] = {0, 0, 0, 0};
};

One decode_one(CaptureFormat fmt, const std::vector<uint8_t>& px, float paper) {
    One o;
    std::vector<float> lin;
    std::vector<uint8_t> sdr, map;
    o.ok = hdr::decode_hdr_capture(fmt, px.data(), 1, 1, paper, &lin, &sdr, &map, nullptr);
    if (!o.ok) return o;
    for (int c = 0; c < 3; ++c) o.lin[c] = lin[c];
    for (int c = 0; c < 4; ++c) {
        o.sdr[c] = sdr[c];
        o.map[c] = map[c];
    }
    return o;
}

uint32_t float_bits_le(const std::vector<uint8_t>& b, size_t at) {
    return static_cast<uint32_t>(b[at]) | (static_cast<uint32_t>(b[at + 1]) << 8) | (static_cast<uint32_t>(b[at + 2]) << 16) |
           (static_cast<uint32_t>(b[at + 3]) << 24);
}

}  // namespace

// ---- A2B10G10R10 and A2R10G10B10 (PQ, BT.2020) ----------------------------------------------------------------------------------------

NXTEST(hdr_cap_a2b10g10r10_pq_203_and_1000_nits) {
    // 4x2 image: pixel 0 = PQ(203 nits) on all three channels, the rest black.
    std::vector<uint8_t> raw(4 * 2 * 4, 0);
    const uint32_t c203 = pq_code(203.0f);
    put_u32(raw, 0, c203 | (c203 << 10) | (c203 << 20));

    std::vector<float> lin;
    std::vector<uint8_t> sdr, map;
    hdr::HdrCaptureStats st{};
    CHECK(hdr::decode_hdr_capture(CaptureFormat::A2B10G10R10, raw.data(), 4, 2, 203.0f, &lin, &sdr, &map, &st));
    CHECK(lin.size() == 4 * 2 * 3);
    for (int c = 0; c < 3; ++c) CHECK(std::fabs(lin[c] - 1.0f) < 0.02f);
    CHECK(std::fabs(st.max_nits - 203.0) <= 0.02 * 203.0);
    CHECK(st.width == 4 && st.height == 2);
    CHECK(sdr.size() == 4 * 2 * 4 && map.size() == 4 * 2 * 4);
    CHECK(sdr[0] == 255 && sdr[1] == 255 && sdr[2] == 255 && sdr[3] == 255);   // white preview, alpha 255
    CHECK(map[0] == 255 && map[1] == 255 && map[2] == 255 && map[3] == 255);    // v = 1 is the top of the grey ramp

    // Same image with pixel 0 = PQ(1000 nits): 1000 / 203 = 4.926 paper-white units, magenta in the map.
    const uint32_t c1000 = pq_code(1000.0f);
    put_u32(raw, 0, c1000 | (c1000 << 10) | (c1000 << 20));
    CHECK(hdr::decode_hdr_capture(CaptureFormat::A2B10G10R10, raw.data(), 4, 2, 203.0f, &lin, &sdr, &map, &st));
    for (int c = 0; c < 3; ++c) CHECK(std::fabs(lin[c] - 4.926f) <= 0.02f * 4.926f);
    CHECK(std::fabs(st.max_nits - 1000.0) <= 0.02 * 1000.0);
    CHECK(map[0] == 255 && map[1] == 0 && map[2] == 255 && map[3] == 255);
}

NXTEST(hdr_cap_a2_channel_order_and_bt2020_red) {
    // One logical pixel: pure BT.2020 red at paper white. R is PQ-coded, G and B are 0.
    const uint32_t cr = pq_code(203.0f);
    std::vector<uint8_t> a2r(4), a2b(4);
    put_u32(a2r, 0, cr << 20);      // A2R10G10B10: R in bits 20-29
    put_u32(a2b, 0, cr);            // A2B10G10R10: R in bits 0-9
    CHECK(a2r != a2b);

    const One x = decode_one(CaptureFormat::A2R10G10B10, a2r, 203.0f);
    const One y = decode_one(CaptureFormat::A2B10G10R10, a2b, 203.0f);
    CHECK(x.ok && y.ok);
    // BT.2020 red in BT.709 linear: R = 1.6605, G = -0.1246, B = -0.0182 (G and B negative, so the preview clamps them to 0).
    CHECK(x.lin[0] > 1.6f && x.lin[0] < 1.72f);
    CHECK(x.lin[1] < -0.1f && x.lin[1] > -0.15f);
    CHECK(x.lin[2] < 0.0f && x.lin[2] > -0.03f);
    CHECK(x.sdr[0] == 255 && x.sdr[1] == 0 && x.sdr[2] == 0 && x.sdr[3] == 255);
    for (int c = 0; c < 3; ++c) CHECK(std::fabs(x.lin[c] - y.lin[c]) < 1e-6f);

    // The same bits read with the other order: R's code lands in the blue channel (BT.2020 blue -> BT.709 gives R = -0.0729).
    const One z = decode_one(CaptureFormat::A2B10G10R10, a2r, 203.0f);
    CHECK(z.ok);
    CHECK(std::fabs(z.lin[0] - x.lin[0]) > 1.0f);
}

// ---- Rgba16F (scRGB) and the half-float decode --------------------------------------------------------------------------------------

NXTEST(hdr_cap_rgba16f_scrgb_and_half_values) {
    // scRGB 203 nits = 2.5375 (half 0x4113 = 2.537109375); at paper 203 it decodes to about 1.0.
    std::vector<uint8_t> px(8, 0);
    put_u16(px, 0, 0x4113);
    put_u16(px, 2, 0x4113);
    put_u16(px, 4, 0x4113);
    put_u16(px, 6, 0x3C00);
    const One a = decode_one(CaptureFormat::Rgba16F, px, 203.0f);
    CHECK(a.ok);
    for (int c = 0; c < 3; ++c) CHECK(std::fabs(a.lin[c] - 1.0f) < 1e-3f);
    CHECK(a.sdr[0] == 255);

    // At paper 80 the linear value equals the half value itself, so each half is checked directly.
    struct Case {
        uint16_t bits;
        float expect;
    };
    const Case cases[] = {
        {0x3C00, 1.0f},          // 1.0
        {0x3800, 0.5f},          // 0.5
        {0x7BFF, 65504.0f},      // largest finite half
        {0x0001, 5.9604645e-8f}, // smallest subnormal, 2^-24
        {0xBC00, -1.0f},         // negative: linear keeps the sign
        {0x7C00, 65504.0f},      // +inf is capped to 65504
        {0x7E00, 0.0f},          // NaN decodes to 0
    };
    for (const Case& c : cases) {
        put_u16(px, 0, c.bits);
        put_u16(px, 2, c.bits);
        put_u16(px, 4, c.bits);
        const One o = decode_one(CaptureFormat::Rgba16F, px, 80.0f);
        CHECK(o.ok);
        CHECK(std::fabs(o.lin[0] - c.expect) <= 1e-6f * std::fabs(c.expect) + 1e-12f);
    }

    // Negative values decode as 0 in the previews.
    put_u16(px, 0, 0xBC00);
    put_u16(px, 2, 0x3C00);
    put_u16(px, 4, 0x3C00);
    const One neg = decode_one(CaptureFormat::Rgba16F, px, 80.0f);
    CHECK(neg.lin[0] == -1.0f);
    CHECK(neg.sdr[0] == 0 && neg.sdr[1] == 255 && neg.sdr[2] == 255 && neg.sdr[3] == 255);
}

// ---- Rgba8 (sRGB) -----------------------------------------------------------------------------------------------------------------------

NXTEST(hdr_cap_rgba8_srgb_byte_188) {
    const std::vector<uint8_t> px = {188, 0, 255, 255};
    const One o = decode_one(CaptureFormat::Rgba8, px, 203.0f);
    CHECK(o.ok);
    CHECK(std::fabs(o.lin[0] - hdr::srgb_eotf(188.0f / 255.0f)) < 1e-6f);
    CHECK(std::fabs(o.lin[0] - 0.5f) < 0.01f);                  // about half of paper white
    CHECK(o.lin[1] == 0.0f && std::fabs(o.lin[2] - 1.0f) < 1e-6f);
    CHECK(o.sdr[0] == 188 && o.sdr[1] == 0 && o.sdr[2] == 255 && o.sdr[3] == 255);   // the preview round-trips the byte
}

NXTEST(hdr_cap_false_colour_ramps) {
    // Rgba16F at paper 80: the half value is the paper-white value v.
    struct Case {
        uint16_t bits;
        uint8_t r, g, b;
    };
    const Case cases[] = {
        {0x3400, 64, 64, 64},     // v = 0.25, top of the dark ramp
        {0x3800, 128, 128, 128},  // v = 0.5, grey ramp
        {0x3D00, 255, 231, 0},    // v = 1.25, yellow ramp
        {0x4200, 255, 80, 0},     // v = 3, orange -> red ramp
        {0x4600, 255, 0, 255},    // v = 6, magenta
    };
    for (const Case& c : cases) {
        std::vector<uint8_t> px(8, 0);
        put_u16(px, 0, c.bits);
        put_u16(px, 2, c.bits);
        put_u16(px, 4, c.bits);
        put_u16(px, 6, 0x3C00);
        const One o = decode_one(CaptureFormat::Rgba16F, px, 80.0f);
        CHECK(o.ok);
        CHECK(o.map[0] == c.r && o.map[1] == c.g && o.map[2] == c.b && o.map[3] == 255);
    }
}

// ---- statistics over a 10x10 image ---------------------------------------------------------------------------------------------------

NXTEST(hdr_cap_fractions_10x10) {
    // Rgba16F at paper 80 (so the half value is the paper-white value v): 30 at 1.5x, 10 at 3x, 5 at 5x, 55 at 0.5x.
    std::vector<uint8_t> raw(10 * 10 * 8, 0);
    for (size_t i = 0; i < 100; ++i) {
        uint16_t h = 0x3800;                         // 0.5
        if (i < 30) h = 0x3E00;                      // 1.5
        else if (i < 40) h = 0x4200;                 // 3
        else if (i < 45) h = 0x4500;                 // 5
        for (int c = 0; c < 3; ++c) put_u16(raw, i * 8 + 2 * c, h);
        put_u16(raw, i * 8 + 6, 0x3C00);
    }
    hdr::HdrCaptureStats st{};
    CHECK(hdr::decode_hdr_capture(CaptureFormat::Rgba16F, raw.data(), 10, 10, 80.0f, nullptr, nullptr, nullptr, &st));
    CHECK(std::fabs(st.frac_above_paper - 0.45) < 1e-9);
    CHECK(std::fabs(st.frac_above_2x - 0.15) < 1e-9);
    CHECK(std::fabs(st.frac_above_4x - 0.05) < 1e-9);
    CHECK(std::fabs(st.max_nits - 400.0) < 1e-6);                                  // 5 x 80
    // mean of v = (30*1.5 + 10*3 + 5*5 + 55*0.5) / 100 = 127.5 / 100 = 1.275 paper-white units = 102 nits
    CHECK(std::fabs(st.mean_nits - 102.0) < 1e-4);
    CHECK(st.width == 10 && st.height == 10);
}

NXTEST(hdr_cap_stats_line_and_bytes_per_pixel) {
    hdr::HdrCaptureStats s{};
    s.max_nits = 1000.0;
    s.mean_nits = 12.34;
    s.frac_above_paper = 0.45;
    s.frac_above_2x = 0.15;
    s.frac_above_4x = 0.05;
    CHECK(hdr::capture_stats_line(s) == "max_nits=1000.0 mean_nits=12.3 above_paper=45.00% above_2x=15.00% above_4x=5.00%");

    CHECK(hdr::capture_bytes_per_pixel(CaptureFormat::A2B10G10R10) == 4);
    CHECK(hdr::capture_bytes_per_pixel(CaptureFormat::A2R10G10B10) == 4);
    CHECK(hdr::capture_bytes_per_pixel(CaptureFormat::Rgba16F) == 8);
    CHECK(hdr::capture_bytes_per_pixel(CaptureFormat::Rgba8) == 4);
}

// ---- PFM writer, read back from disk -------------------------------------------------------------------------------------------------

NXTEST(hdr_cap_write_pfm_round_trip) {
    // 2x2 image, top row first in memory: (1,2,3) (4,5,6) / bottom row (7,8,9) (10,11,12). Scale 0.5.
    const std::vector<float> lin = {1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12};
    const char* tmp = std::getenv("TEMP");
    const std::string path = std::string(tmp ? tmp : ".") + "/nxrt_hdr_capture_test.pfm";
    CHECK(hdr::write_pfm(path, lin, 2, 2, 0.5f));

    std::vector<uint8_t> bytes;
    if (std::FILE* f = std::fopen(path.c_str(), "rb")) {
        uint8_t buf[256];
        size_t n;
        while ((n = std::fread(buf, 1, sizeof buf, f)) > 0) bytes.insert(bytes.end(), buf, buf + n);
        std::fclose(f);
    }
    std::remove(path.c_str());

    const std::string header = "PF\n2 2\n-1.0\n";
    REQUIRE(bytes.size() == header.size() + 2 * 2 * 3 * 4);
    CHECK(std::memcmp(bytes.data(), header.data(), header.size()) == 0);

    // Rows bottom to top: the bottom image row (7..12) comes first, then the top row (1..6), each scaled by 0.5.
    const float expect[12] = {3.5f, 4.0f, 4.5f, 5.0f, 5.5f, 6.0f, 0.5f, 1.0f, 1.5f, 2.0f, 2.5f, 3.0f};
    for (size_t i = 0; i < 12; ++i) {
        float v;
        const uint32_t bits = float_bits_le(bytes, header.size() + 4 * i);
        std::memcpy(&v, &bits, sizeof v);
        CHECK(v == expect[i]);
    }
}

// ---- argument checks -----------------------------------------------------------------------------------------------------------------

NXTEST(hdr_cap_invalid_arguments_return_false) {
    std::vector<uint8_t> raw(8, 0);
    std::vector<float> lin;
    std::vector<uint8_t> sdr;
    CHECK(!hdr::decode_hdr_capture(CaptureFormat::Rgba8, nullptr, 1, 1, 203.0f, &lin, &sdr, nullptr, nullptr));
    CHECK(!hdr::decode_hdr_capture(CaptureFormat::Rgba8, raw.data(), 0, 1, 203.0f, &lin, &sdr, nullptr, nullptr));
    CHECK(!hdr::decode_hdr_capture(CaptureFormat::Rgba8, raw.data(), 1, 0, 203.0f, &lin, &sdr, nullptr, nullptr));
    CHECK(!hdr::decode_hdr_capture(CaptureFormat::Rgba8, raw.data(), 1, 1, 0.0f, &lin, &sdr, nullptr, nullptr));
    CHECK(!hdr::decode_hdr_capture(CaptureFormat::Rgba8, raw.data(), 1, 1, -203.0f, &lin, &sdr, nullptr, nullptr));
    CHECK(hdr::decode_hdr_capture(CaptureFormat::Rgba8, raw.data(), 1, 1, 203.0f, nullptr, nullptr, nullptr, nullptr));  // all outputs optional

    // PFM: a path in a missing folder is an I/O error; a buffer that is not w*h*3 floats is refused.
    CHECK(!hdr::write_pfm("nxrt_no_such_dir_hdr_capture/x.pfm", std::vector<float>(12, 0.0f), 2, 2, 1.0f));
    CHECK(!hdr::write_pfm("nxrt_no_such_dir_hdr_capture/x.pfm", std::vector<float>(5, 0.0f), 2, 2, 1.0f));
}
