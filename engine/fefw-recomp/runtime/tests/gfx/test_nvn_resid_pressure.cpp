// SPDX-License-Identifier: GPL-3.0-or-later
// [gfx] residency: when memory use calls for demotions (src/gfx/nvn/nvn_resid_pressure.h).
#include "nvn_resid_pressure.h"
#include "test_framework.h"

NXTEST_GROUP(gfx_nvn_resid_pressure)

using nxrt::nvn::resid_pressure;

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
