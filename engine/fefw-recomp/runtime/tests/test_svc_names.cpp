// SPDX-License-Identifier: GPL-3.0-or-later
#include "test_framework.h"

#include <cstddef>
#include <cstdint>
#include <string>

NXTEST_GROUP(svc_names)

namespace {

struct SvcName {
    uint32_t num;
    const char* name;
};

#define NXRT_SVC(n, name) {n, name},
static const SvcName kSvc[] = {
#include "../src/kernel/svc_names.inc"
};
#undef NXRT_SVC

constexpr std::size_t kCount = sizeof(kSvc) / sizeof(kSvc[0]);

std::string name_of(uint32_t num) {
    for (std::size_t i = 0; i < kCount; i++)
        if (kSvc[i].num == num) return kSvc[i].name;
    return "";
}

}  // namespace

NXTEST(numbers_strictly_ascending) {
    for (std::size_t i = 1; i < kCount; i++) {
        CHECK_MSG(kSvc[i].num > kSvc[i - 1].num,
                  "entry " + std::to_string(i) + " (0x" + std::to_string(kSvc[i].num) + ") not above previous");
    }
}

NXTEST(has_at_least_100_entries) {
    CHECK(kCount >= 100);
}

NXTEST(svc_01_set_heap_size) {
    CHECK_EQ(name_of(0x01), std::string("SetHeapSize"));
}

NXTEST(svc_18_and_21) {
    CHECK_EQ(name_of(0x18), std::string("WaitSynchronization"));
    CHECK_EQ(name_of(0x21), std::string("SendSyncRequest"));
}

NXTEST(svc_34_45_91) {
    CHECK_EQ(name_of(0x34), std::string("WaitForAddress"));
    CHECK_EQ(name_of(0x45), std::string("CreateEvent"));
    CHECK_EQ(name_of(0x91), std::string("UnmapInsecurePhysicalMemory"));
}
