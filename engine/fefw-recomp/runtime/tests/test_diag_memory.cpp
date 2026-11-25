// SPDX-License-Identifier: GPL-3.0-or-later
// diag_memory_snapshot() must set PROCESS_MEMORY_COUNTERS_EX.cb; a zero cb made the whole process-memory section report a failed query.
#include "test_framework.h"

#include "nxrt/diag.h"

#include <string>

NXTEST_GROUP(diag_memory)

NXTEST(snapshot_reports_process_memory) {
    const std::string text = nxrt::diag_memory_snapshot_text();
    CHECK(text.find("--- Process Memory ---") != std::string::npos);
    CHECK(text.find("failed to query process memory") == std::string::npos);
    CHECK(text.find("Working Set:") != std::string::npos);
    CHECK(text.find("Private Bytes:") != std::string::npos);
    CHECK(text.find("Pagefile Usage:") != std::string::npos);
}
