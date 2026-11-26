// SPDX-License-Identifier: GPL-3.0-or-later
// Placement of the runtime's own busy host threads next to the console core model (notes/threads.md).
//
// The guest threads get ideal processors on the first hardware thread of six physical cores (sched.cpp: console core 0 = main on the
// highest, the job workers next, core 5 on the lowest), with the whole pool as affinity. The render thread (nvn-render, the busiest
// thread of the process) is a plain host thread: the host gives it an ideal processor of its own choosing, which can be the SMT sibling
// of main's. [sched] render_place moves it:
//   off (default)  leave it to the host;
//   core5          ideal processor = console core 5's processor (the system-side core: audio and other light threads), affinity the pool;
//   apart          as core5, and its affinity leaves out the physical core of console core 0 (main's), so the two busiest threads never
//                  share a core; main's own affinity is unchanged;
//   reserve        every other host thread of the process (the runtime's workers, the driver's, audio) is kept off that core too;
//   <hex mask>     affinity = that mask (ideal = its lowest processor).
// [sched] place_log = 1 logs every registered host thread's ideal processor once the game runs (diagnostic).
// A placer thread looks for the named threads in the diag registry every 2 s for the first 120 s (the render thread starts with the
// first NVN device), applies the setting once to each, and exits. Everything goes through nxrt/platform.h: thread ids from
// process_thread_ids(), affinity and ideal processors from host_thread_set_*(). POSIX has no ideal processor (host_thread_set_ideal_processor
// answers false), so there `core5` leaves the render thread to the scheduler and the log says so once; `apart` and `<hex mask>`, which
// are affinity, do apply.
#include <cctype>
#include <cstdlib>
#include <string>
#include <thread>
#include <unordered_set>
#include <vector>

#include "kernel_internal.h"
#include "nxrt/config.h"
#include "nxrt/diag.h"
#include "nxrt/kernel.h"
#include "nxrt/log.h"
#include "nxrt/platform.h"

namespace nxrt::kernel {
namespace {

int low_bit(uint64_t m) {
    for (int b = 0; b < 64; b++)
        if (m >> b & 1) return b;
    return 0;
}

// Says once that this host cannot be told where to run a thread (POSIX): the affinity settings still apply.
void note_no_ideal_processor() {
    static bool told = false;
    if (told) return;
    told = true;
    NXLOG_I("sched", "render_place: this host has no ideal processor: only affinity is applied ([sched] render_place = apart or a mask)");
}

void place(const ThreadRecord& r, const std::string& how, const SchedInfo& si) {
    const int before = nxrt::platform::host_thread_ideal_processor(r.host_tid);
    uint64_t aff = si.pool;
    int ideal = -1;
    if (how == "reserve" && si.render_core) {
        aff = si.render_core;
        ideal = low_bit(si.render_core);
    } else if (how == "core5" || how == "apart" || how == "reserve") {
        const int last = si.cores >= 6 ? 5 : si.cores - 1;
        ideal = low_bit(si.host_masks[last]);
        if (how == "apart" || how == "reserve") aff &= ~nxrt::platform::physical_core_mask(low_bit(si.host_masks[0]));
    } else {
        const uint64_t m = std::strtoull(how.c_str(), nullptr, 16);
        if (m) aff = m, ideal = low_bit(m);
    }
    if (!aff || ideal < 0) {
        NXLOG_W("sched", "render_place = %s: nothing to apply", how.c_str());
        return;
    }
    nxrt::platform::host_thread_set_affinity(r.host_tid, aff);
    if (!nxrt::platform::host_thread_set_ideal_processor(r.host_tid, ideal)) note_no_ideal_processor();
    NXLOG_I("sched", "render_place = %s: %s (tid %u) ideal processor %d -> %d, affinity %llx", how.c_str(), r.name.c_str(), r.host_tid, before,
            nxrt::platform::host_thread_ideal_processor(r.host_tid), (unsigned long long)aff);
}

// render_place = reserve: every host thread of the process that is not a guest thread and not the render thread (the runtime's workers, the
// driver's threads, audio) gets the guest pool as its affinity, so nothing but the render thread runs on its core and nothing shares main's.
// Threads come and go for the whole session (texture workers, the driver's), so this sweep runs every 2 s while the process lives.
void sweep_host_threads(const SchedInfo& si, const std::vector<ThreadRecord>& recs, std::unordered_set<uint32_t>& swept) {
    std::unordered_set<uint32_t> skip;
    for (const ThreadRecord& r : recs)
        if (r.guest_tid || r.name == "nvn-render") skip.insert(r.host_tid);
    std::vector<uint32_t> tids;
    nxrt::platform::process_thread_ids(&tids);
    const uint32_t self = nxrt::platform::thread_id();
    int moved = 0;
    for (const uint32_t tid : tids) {
        if (tid == self || skip.count(tid) || swept.count(tid)) continue;
        swept.insert(tid);
        if (nxrt::platform::host_thread_set_affinity(tid, si.pool)) ++moved;
    }
    static bool told = false;
    if (moved && !told) {
        told = true;
        NXLOG_I("sched", "render_place = reserve: %d host threads (workers, driver) moved to the guest pool %llx; new ones follow every 2 s", moved,
                (unsigned long long)si.pool);
    }
}

void placer(std::string how, bool log) {
    std::unordered_set<uint32_t> done, swept;
    bool logged = false;
    const bool reserve = how == "reserve";
    for (int i = 0; reserve || i < 60; i++) {
        nxrt::platform::sleep_ns(2000000000);   // 2 s
        const SchedInfo si = sched_info();
        if (!si.active) return;
        const std::vector<ThreadRecord> recs = diag_threads();
        if (reserve && si.render_core) sweep_host_threads(si, recs, swept);
        if (how != "off")
            for (const ThreadRecord& r : recs)
                if (r.name == "nvn-render" && done.insert(r.host_tid).second) place(r, how, si);
        if (log && !logged && i >= 14) {        // ~30 s in: the game's threads and the runtime's pools exist
            logged = true;
            std::string line;
            for (const ThreadRecord& r : recs) {
                const int ideal = nxrt::platform::host_thread_ideal_processor(r.host_tid);
                const int prio = nxrt::platform::host_thread_priority(r.host_tid);
                char b[96];
                std::snprintf(b, sizeof b, " %s=%d/%s", r.name.c_str(), ideal, prio == 0 ? "n" : std::to_string(prio).c_str());
                line += b;
            }
            NXLOG_I("sched", "host threads (name=ideal processor/priority):%s", line.c_str());
        }
        if (!reserve && (how == "off" || !done.empty()) && (!log || logged)) return;
    }
}

}  // namespace

void install_host_placement() {
    std::string how = config_get_str("sched", "render_place", "off");
    for (char& ch : how) ch = static_cast<char>(std::tolower(static_cast<unsigned char>(ch)));
    const bool log = config_get_bool("sched", "place_log", false);
    if (how == "off" && !log) return;
    std::thread(placer, how, log).detach();
}

}  // namespace nxrt::kernel
