// SPDX-License-Identifier: GPL-3.0-or-later
// S38: the cap of the device-local pool copies (src/gfx/nvn/nvn_mirror_cap.h).
#include <cstdint>

#include "nvn_mirror_cap.h"
#include "test_framework.h"

NXTEST_GROUP(gfx_nvn_mirror_cap)

using nxrt::nvn::mirror_cap;

namespace {
constexpr uint64_t MiB = 1ull << 20;
constexpr uint64_t GiB = 1ull << 30;
}  // namespace

NXTEST(large_card_caps_at_2048_mib) {
    CHECK_EQ(mirror_cap(10 * GiB, -1).bytes, 2048 * MiB);
    CHECK_EQ(mirror_cap(24 * GiB, -1).bytes, 2048 * MiB);
    CHECK_EQ(mirror_cap(8 * GiB, -1).bytes, 2048 * MiB);      // exactly a quarter
}

NXTEST(mid_card_gets_a_quarter) {
    CHECK_EQ(mirror_cap(6 * GiB, -1).bytes, 1536 * MiB);
    CHECK_EQ(mirror_cap(4 * GiB, -1).bytes, 1024 * MiB);      // 4 GiB is enough
}

NXTEST(small_or_unknown_card_gets_no_copies) {
    CHECK_EQ(mirror_cap(4 * GiB - 1, -1).bytes, 0ull);
    CHECK_EQ(mirror_cap(2 * GiB, -1).bytes, 0ull);
    CHECK_EQ(mirror_cap(0, -1).bytes, 0ull);
}

NXTEST(explicit_value_wins) {
    CHECK_EQ(mirror_cap(2 * GiB, 512).bytes, 512 * MiB);      // even under 4 GiB
    CHECK_EQ(mirror_cap(24 * GiB, 4096).bytes, 4096 * MiB);   // even over 2048
    CHECK_EQ(mirror_cap(24 * GiB, 0).bytes, 0ull);            // 0 turns the copies off
    CHECK_EQ(mirror_cap(16 * GiB, -1, true).bytes, 0ull);     // integrated GPU: none by default
    CHECK_EQ(mirror_cap(16 * GiB, 1024, true).bytes, 1024 * MiB);   // but an explicit value still wins
}
