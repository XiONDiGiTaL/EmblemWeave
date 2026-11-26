// SPDX-License-Identifier: GPL-3.0-or-later
// [gfx] residency: what memory use means for demotions and restores (src/gfx/nvn/nvn_resid_pressure.h).
#include "nvn_resid_pressure.h"
#include "test_framework.h"

NXTEST_GROUP(gfx_nvn_resid_pressure)

using nxrt::nvn::resid_pressure;
using nxrt::nvn::resid_restore;

namespace {
constexpr uint64_t MiB = 1ull << 20;
}  // namespace

NXTEST(video_memory_under_the_percent_demotes_nothing) {
    CHECK(!resid_pressure(50, 60, -1, 80).demote);
    CHECK(!resid_pressure(79, 84, -1, 80).demote);
}

NXTEST(live_use_at_the_percent_demotes) {
    const auto p = resid_pressure(80, 82, -1, 80);
    CHECK(p.demote);
    CHECK(!p.sysram_only);
    CHECK_EQ(p.pct, 80);   // the room free inside the blocks does not count while the OS figure is under its trigger
}

NXTEST(os_use_demotes_at_85) {
    const auto p = resid_pressure(60, 85, -1, 80);
    CHECK(p.demote);
    CHECK_EQ(p.pct, 85);
    CHECK(resid_pressure(40, 85, -1, 48).demote);
}

// #114: a 16 GB Steam Deck sat at 85 to 89 percent of its RAM with video memory half empty.
NXTEST(sysram_under_92_demotes_nothing) {
    CHECK(!resid_pressure(46, 51, 85, 80).demote);
    CHECK(!resid_pressure(46, 51, 91, 80).demote);
    CHECK(!resid_pressure(40, 45, 91, 48).demote);   // a lower residency_percent does not lower it
}

NXTEST(sysram_at_92_demotes) {
    const auto p = resid_pressure(46, 51, 92, 80);
    CHECK(p.demote);
    CHECK(p.sysram_only);
    CHECK_EQ(p.pct, 92);
}

NXTEST(sysram_trigger_follows_a_percent_over_92) {
    CHECK(!resid_pressure(46, 51, 94, 95).demote);
    CHECK(resid_pressure(46, 51, 95, 95).demote);
}

NXTEST(sysram_under_its_trigger_does_not_raise_the_figure) {
    const auto p = resid_pressure(70, 86, 91, 80);
    CHECK(p.demote);
    CHECK(!p.sysram_only);
    CHECK_EQ(p.pct, 86);
}

NXTEST(unknown_figures) {
    CHECK(!resid_pressure(50, -1, -1, 80).demote);
    CHECK(resid_pressure(85, -1, -1, 80).demote);
}

NXTEST(restores_with_free_video_memory_get_the_usual_budget) {
    const auto r = resid_restore(51, 80, 8, 8, false);
    CHECK(!r.hold);
    CHECK(!r.catchup);
    CHECK_EQ(r.max_n, 8);
    CHECK_EQ(r.max_bytes, 8 * MiB);
}

NXTEST(restore_backlog_gets_four_times_under_85) {
    const auto r = resid_restore(84, 80, 8, 8, true);
    CHECK(r.catchup);
    CHECK_EQ(r.max_n, 32);
    CHECK_EQ(r.max_bytes, 32 * MiB);
}

NXTEST(restores_slow_from_85_and_again_from_92) {
    const auto warn = resid_restore(85, 80, 8, 8, true);
    CHECK(!warn.catchup);
    CHECK_EQ(warn.max_n, 2);
    CHECK_EQ(warn.max_bytes, 4 * MiB);
    const auto urgent = resid_restore(92, 80, 8, 8, true);
    CHECK_EQ(urgent.max_n, 1);
    CHECK_EQ(urgent.max_bytes, 2 * MiB);
}

NXTEST(restores_hold_from_96) {
    CHECK(!resid_restore(95, 80, 8, 8, false).hold);
    CHECK(resid_restore(96, 80, 8, 8, false).hold);
}

NXTEST(restores_with_video_use_unknown_are_not_slowed) {
    const auto r = resid_restore(-1, 80, 8, 8, false);
    CHECK(!r.hold);
    CHECK_EQ(r.max_n, 8);
}
