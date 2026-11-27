// SPDX-License-Identifier: GPL-3.0-or-later
// [gfx] residency: when memory use calls for demoting idle textures. Pure, so the tests can call it.
#pragma once

#include <algorithm>

namespace nxrt::nvn {

struct ResidPressure {
    bool demote = false;
    bool sysram_only = false;   // system RAM alone calls for it (for the log line)
    int pct = 0;                // the use that sets how much to give back and how long a texture must have been idle
};

// All in percent; -1: not known. live_pct: video memory in use without the room free inside the allocator's blocks; os_pct: the OS's
// figure with that room; sysram_pct: the host's RAM on a unified-memory GPU (-1 on a discrete one); residency_percent: the ini key.
inline ResidPressure resid_pressure(int live_pct, int os_pct, int sysram_pct, int residency_percent) {
    const int os_trigger = std::min(100, std::max(residency_percent + 5, 85));
    // Higher than the video memory triggers: the game with the rest of the system fills about 85 percent of a 16 GB handheld, and there
    // demotions gave back no RAM that showed, only restores to pay for (#114).
    const int sysram_trigger = std::min(100, std::max(residency_percent, 92));
    const bool os = os_pct >= os_trigger;
    const bool sysram = sysram_pct >= sysram_trigger;
    ResidPressure p;
    p.demote = live_pct >= residency_percent || os || sysram;
    p.sysram_only = sysram && !os && live_pct < residency_percent;
    p.pct = live_pct;
    if (os || sysram) p.pct = std::max(p.pct, os_pct);
    if (sysram) p.pct = std::max(p.pct, sysram_pct);
    return p;
}

}  // namespace nxrt::nvn
