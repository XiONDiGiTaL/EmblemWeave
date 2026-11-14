// SPDX-License-Identifier: GPL-3.0-or-later
// Windows display HDR query (win_display.h) on this machine: the call does not crash and reports something, the numbers are in range
// when it is valid, and repeated calls leak no handles. Does not assert hdr_enabled either way, since the monitor's setting decides it.
// The full result is printed so the run output shows what the query found.
#include <windows.h>

#include <cstdio>

#include "test_framework.h"

#include "extras/win_display.h"

NXTEST_GROUP(win_display)

namespace {

namespace extras = nxrt::extras;

void print_info(const extras::DisplayHdrInfo& i) {
    std::printf(
        "win_display: valid=%d monitor=%s hdr_enabled=%d max_nits=%.1f min_nits=%.4f max_fullframe_nits=%.1f sdr_white_nits=%.1f\n"
        "win_display: how=%s\n",
        i.valid ? 1 : 0, i.monitor, i.hdr_enabled ? 1 : 0, i.max_nits, i.min_nits, i.max_fullframe_nits, i.sdr_white_nits, i.how);
}

}  // namespace

NXTEST(query_primary_does_not_crash_and_reports_text) {
    extras::DisplayHdrInfo i;
    const bool ret = extras::query_display_hdr(nullptr, &i);
    print_info(i);
    CHECK_EQ(ret, i.valid);
    CHECK_MSG(i.monitor[0] != '\0' || i.how[0] != '\0', "neither monitor nor how was filled");
}

NXTEST(valid_result_has_sane_numbers) {
    extras::DisplayHdrInfo i;
    extras::query_display_hdr(nullptr, &i);
    print_info(i);
    if (!i.valid) {
        ::nxtest::skip("query did not answer on this machine (see the how line above)");
        return;
    }
    CHECK(i.max_nits >= 0.0f);
    CHECK(i.sdr_white_nits >= 80.0f && i.sdr_white_nits <= 500.0f);
}

NXTEST(repeated_queries_leak_no_handles) {
    extras::DisplayHdrInfo i;
    DWORD before = 0, after = 0;
    CHECK(GetProcessHandleCount(GetCurrentProcess(), &before));
    for (int n = 0; n < 50; ++n) extras::query_display_hdr(nullptr, &i);
    CHECK(GetProcessHandleCount(GetCurrentProcess(), &after));
    const long long grown = static_cast<long long>(after) - static_cast<long long>(before);
    std::printf("win_display: handles before=%lu after=%lu (50 queries) grown=%lld\n", static_cast<unsigned long>(before),
                static_cast<unsigned long>(after), grown);
    CHECK_MSG(grown < 20, "handle count grew by 20 or more over 50 queries");
}
