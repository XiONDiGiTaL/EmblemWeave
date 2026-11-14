// SPDX-License-Identifier: GPL-3.0-or-later
// FSR 1 CPU constants (extras/fsr1_consts.h): the EASU and RCAS push-constant words, checked against the arithmetic of
// ffxFsrPopulateEasuConstants and FsrRcasCon in the FidelityFX SDK v1.1.4 header (third_party/ffx_fsr1/fsr1/ffx_fsr1.h).
#include <cmath>
#include <cstdint>
#include <cstring>

#include "test_framework.h"

#include "extras/fsr1_consts.h"

NXTEST_GROUP(fsr1_consts)

namespace {

float as_float(uint32_t u) {
    float f;
    std::memcpy(&f, &u, sizeof f);
    return f;
}

uint32_t as_uint(float f) {
    uint32_t u;
    std::memcpy(&u, &f, sizeof u);
    return u;
}

}  // namespace

// 1280x720 -> 1920x1080, source texture the same size as the viewport.
NXTEST(easu_720p_to_1080p) {
    uint32_t c0[4], c1[4], c2[4], c3[4];
    nxrt::extras::fsr1_easu_consts(c0, c1, c2, c3, 1280.0f, 720.0f, 1280.0f, 720.0f, 1920.0f, 1080.0f);

    // con0: viewport/output scale and the half-pixel offset of the first tap, as in the header (left to right).
    CHECK(std::fabs(as_float(c0[0]) - 0.66666666f) < 1e-7f);
    CHECK_EQ(c0[0], as_uint(1280.0f * (1.0f / 1920.0f)));
    CHECK_EQ(c0[1], as_uint(720.0f * (1.0f / 1080.0f)));
    CHECK_EQ(c0[2], as_uint(0.5f * 1280.0f * (1.0f / 1920.0f) - 0.5f));
    CHECK_EQ(c0[3], as_uint(0.5f * 720.0f * (1.0f / 1080.0f) - 0.5f));
    CHECK(std::fabs(as_float(c0[2]) - (0.5f * 0.66666666f - 0.5f)) < 1e-6f);

    // con1: (1/tex_w, 1/tex_h, 1/tex_w, -1/tex_h)
    CHECK_EQ(c1[0], as_uint(1.0f / 1280.0f));
    CHECK_EQ(c1[1], as_uint(1.0f / 720.0f));
    CHECK_EQ(c1[2], as_uint(1.0f / 1280.0f));
    CHECK_EQ(c1[3], as_uint(-1.0f / 720.0f));

    // con2 and con3: the neighbour offsets used by the 12-tap kernel
    CHECK_EQ(c2[0], as_uint(-1.0f / 1280.0f));
    CHECK_EQ(c2[1], as_uint(2.0f / 720.0f));
    CHECK_EQ(c2[2], as_uint(1.0f / 1280.0f));
    CHECK_EQ(c2[3], as_uint(2.0f / 720.0f));
    CHECK_EQ(c3[0], as_uint(0.0f));
    CHECK_EQ(c3[1], as_uint(4.0f / 720.0f));
    CHECK_EQ(c3[2], 0u);
    CHECK_EQ(c3[3], 0u);
}

// The viewport is smaller than the texture (960x540 inside a 1024x576 texture): each input must land in its own slot.
NXTEST(easu_viewport_inside_larger_texture) {
    uint32_t c0[4], c1[4], c2[4], c3[4];
    nxrt::extras::fsr1_easu_consts(c0, c1, c2, c3, 960.0f, 540.0f, 1024.0f, 576.0f, 1920.0f, 1080.0f);
    CHECK_EQ(as_float(c0[0]), 0.5f);
    CHECK_EQ(as_float(c0[1]), 0.5f);
    CHECK_EQ(c0[2], as_uint(0.5f * 960.0f * (1.0f / 1920.0f) - 0.5f));
    CHECK_EQ(as_float(c1[0]), 1.0f / 1024.0f);
    CHECK_EQ(c1[1], as_uint(1.0f / 576.0f));
    CHECK_EQ(c1[3], as_uint(-1.0f / 576.0f));
    CHECK_EQ(c3[1], as_uint(4.0f / 576.0f));
}

// RCAS: con[0] = bits of exp2(-stops); con[1] = the same value as two binary16 halves (low half x, high half y).
// Half values were taken from Python's struct '<e' packing (0x3c00 = 1.0, 0x3800 = 0.5, 0x3400 = 0.25, 0x39a8 = 2^-0.5).
NXTEST(rcas_sharpness_constants) {
    uint32_t con[4];

    nxrt::extras::fsr1_rcas_consts(con, 0.0f);
    CHECK_EQ(con[0], 0x3f800000u);
    CHECK_EQ(con[1], 0x3c003c00u);
    CHECK_EQ(con[2], 0u);
    CHECK_EQ(con[3], 0u);

    nxrt::extras::fsr1_rcas_consts(con, 1.0f);
    CHECK_EQ(con[0], as_uint(std::exp2(-1.0f)));
    CHECK_EQ(con[1], 0x38003800u);

    nxrt::extras::fsr1_rcas_consts(con, 2.0f);
    CHECK_EQ(con[0], as_uint(0.25f));
    CHECK_EQ(con[1], 0x34003400u);

    nxrt::extras::fsr1_rcas_consts(con, 0.5f);
    CHECK_EQ(con[0], as_uint(std::exp2(-0.5f)));
    CHECK_EQ(con[0], 0x3f3504f3u);
    CHECK_EQ(con[1], 0x39a839a8u);
}
