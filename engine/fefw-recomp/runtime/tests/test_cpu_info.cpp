// SPDX-License-Identifier: GPL-3.0-or-later
// cpu_info::sample() must set PROCESS_MEMORY_COUNTERS_EX.cb; without it the query fails and the F1 menu and logs show 0 bytes.
#include "test_framework.h"

#include "nxrt/cpu_info.h"

#include <thread>

NXTEST_GROUP(cpu_info)

NXTEST(sample_reports_process_memory) {
    const nxrt::cpu::Load first = nxrt::cpu::sample();   // starts the CPU window
    (void)first;
    std::this_thread::sleep_for(std::chrono::milliseconds(20));
    const nxrt::cpu::Load l = nxrt::cpu::sample();
    CHECK(l.valid);
    CHECK(l.working_set > 0);
    CHECK(l.committed > 0);
    CHECK(l.handles > 0);
    CHECK(l.ram_avail > 0);
}
