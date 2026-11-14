// SPDX-License-Identifier: GPL-3.0-or-later
#include "test_framework.h"
#include "nxrt/abi.h"
#include "nxrt/runtime.h"
NXTEST_GROUP(smoke)

NXTEST(abi_header_resolves) {
    std::printf("    ABI: %s\n", nxrt::abi_source());
    CHECK(nxrt::abi_source() != nullptr);
    Ctx c{};
    CHECK_EQ(sizeof(c.x) / sizeof(c.x[0]), 32u);
}
