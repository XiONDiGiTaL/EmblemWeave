// SPDX-License-Identifier: GPL-3.0-or-later
// [gfx] residency: what memory use means for demoting idle textures and for restoring them. Pure, so the tests can call it.
#pragma once

#include <algorithm>
#include <cstdint>

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

struct ResidRestore {
    bool hold = false;      // none this present: try again later
    bool catchup = false;   // four times the usual budget
    int max_n = 0;          // textures this present at most
    uint64_t max_bytes = 0; // bytes of whole images this present at most
};

// How much a present may restore. video_pct: the OS's figure for video memory (-1: not known). Host RAM is no input: a draw wants these
// textures, and on a handheld host RAM stays high for reasons a held restore does not change (#114). restore_max, restore_mb: the ini
// keys; backlog: a texture has waited residency_restore_catchup_ms.
inline ResidRestore resid_restore(int video_pct, int residency_percent, int restore_max, int restore_mb, bool backlog) {
    ResidRestore r;
    if (video_pct >= 96) {   // near out of memory
        r.hold = true;
        return r;
    }
    const int warn = std::min(100, std::max(residency_percent + 5, 85));
    // no catch-up burst under pressure: bursts at 92%+ made a demote -> restore -> GPU ASTC transcode loop and WDDM paging spikes
    r.catchup = backlog && video_pct < warn;
    r.max_n = restore_max * (r.catchup ? 4 : 1);
    r.max_bytes = (static_cast<uint64_t>(restore_mb) << 20) * (r.catchup ? 4 : 1);
    if (video_pct >= std::max(residency_percent, 92)) {
        r.max_n = std::min(r.max_n, 1);
        r.max_bytes = std::min<uint64_t>(r.max_bytes, 2ull << 20);
    } else if (video_pct >= warn) {
        r.max_n = std::min(r.max_n, 2);
        r.max_bytes = std::min<uint64_t>(r.max_bytes, 4ull << 20);
    }
    return r;
}

}  // namespace nxrt::nvn
