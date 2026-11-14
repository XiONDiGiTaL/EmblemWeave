// SPDX-License-Identifier: GPL-3.0-or-later
// The nvnDeviceGetInteger value table (src/gfx/nvn/nvn_device_info.inc) against the NVNdeviceInfo enum (third_party/nvn/nvn.h).
#include <map>

#include <nvn/nvn.h>
#include "test_framework.h"
NXTEST_GROUP(gfx_nvn_device_info)

namespace {

// NVNdeviceInfo has 98 enumerators without NVN_DEVICE_INFO_LARGE (0x00..0x61, nvn.h lines 235-333).
constexpr int kEnumeratorCount = 98;
static_assert(static_cast<int>(NVN_DEVICE_INFO_EVENTS_SUPPORT_REDUCTION_OPERATIONS) == kEnumeratorCount - 1,
              "the last enumerator must be 0x61 (kEnumeratorCount is out of date)");

// Pass 1: count the lines of the table.
int count_lines() {
    int n = 0;
#define NVN_DEVICE_INFO_VALUE(name, value) ++n;
#include "nvn_device_info.inc"
#undef NVN_DEVICE_INFO_VALUE
    return n;
}

// Pass 2: name -> value, counting any name that appears twice.
std::map<int, int> fill_table(int& duplicates) {
    std::map<int, int> t;
    duplicates = 0;
#define NVN_DEVICE_INFO_VALUE(name, value) \
    if (!t.emplace(static_cast<int>(name), static_cast<int>(value)).second) ++duplicates;
#include "nvn_device_info.inc"
#undef NVN_DEVICE_INFO_VALUE
    return t;
}

int value_of(const std::map<int, int>& t, NVNdeviceInfo key) {
    auto it = t.find(static_cast<int>(key));
    return it == t.end() ? -1 : it->second;
}

}  // namespace

NXTEST(line_count_matches_enumerators) {
    CHECK_EQ(count_lines(), kEnumeratorCount);
}

NXTEST(no_duplicates_and_every_enumerator_once) {
    int dups = 0;
    const auto t = fill_table(dups);
    CHECK_EQ(dups, 0);
    CHECK_EQ(static_cast<int>(t.size()), kEnumeratorCount);
    for (int i = 0; i < kEnumeratorCount; ++i) CHECK(t.count(i) == 1);
}

NXTEST(known_switch_values) {
    int dups = 0;
    const auto t = fill_table(dups);
    CHECK_EQ(value_of(t, NVN_DEVICE_INFO_API_MAJOR_VERSION), 55);
    CHECK_EQ(value_of(t, NVN_DEVICE_INFO_API_MINOR_VERSION), 15);
    CHECK_EQ(value_of(t, NVN_DEVICE_INFO_UNIFORM_BUFFER_BINDINGS_PER_STAGE), 14);
    CHECK_EQ(value_of(t, NVN_DEVICE_INFO_MAX_UNIFORM_BUFFER_SIZE), 65536);
    CHECK_EQ(value_of(t, NVN_DEVICE_INFO_UNIFORM_BUFFER_ALIGNMENT), 256);
    CHECK_EQ(value_of(t, NVN_DEVICE_INFO_COLOR_BUFFER_BINDINGS), 8);
    CHECK_EQ(value_of(t, NVN_DEVICE_INFO_VERTEX_ATTRIBUTES), 16);
    CHECK_EQ(value_of(t, NVN_DEVICE_INFO_TEXTURE_DESCRIPTOR_SIZE), 32);
    CHECK_EQ(value_of(t, NVN_DEVICE_INFO_SUPPORTS_ASTC_FORMATS), 1);
    CHECK_EQ(value_of(t, NVN_DEVICE_INFO_MAX_TEXTURE_SIZE), 16384);
    CHECK_EQ(value_of(t, NVN_DEVICE_INFO_MEMORY_POOL_PAGE_SIZE), 65536);
}
