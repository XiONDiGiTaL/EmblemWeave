// SPDX-License-Identifier: GPL-3.0-or-later
// Placement of the runtime's own busy host threads next to the console core model (notes/threads.md).
//
// The guest threads get ideal processors on the first hardware thread of six physical cores (sched.cpp: console core 0 = main on the
// highest, the job workers next, core 5 on the lowest), with the whole pool as affinity. The render thread (nvn-render, the busiest
// thread of the process) is a plain host thread: Windows gives it an ideal processor of its own choosing, which can be the SMT sibling
// of main's. [sched] render_place moves it:
//   off (default)  leave it to Windows;
//   core5          ideal processor = console core 5's processor (the system-side core: audio and other light threads), affinity the pool;
//   apart          as core5, and its affinity leaves out the physical core of console core 0 (main's), so the two busiest threads never
//                  share a core; main's own affinity is unchanged;
//   <hex mask>     affinity = that mask (ideal = its lowest processor).
// [sched] place_log = 1 logs every registered host thread's ideal processor once the game runs (diagnostic).
// A placer thread looks for the named threads in the diag registry every 2 s for the first 120 s (the render thread starts with the
// first NVN device), applies the setting once to each, and exits.
#include <windows.h>

#include <cctype>
#include <cstdlib>
#include <string>
#include <thread>
#include <unordered_set>

#include "kernel_internal.h"
#include "nxrt/config.h"
#include "nxrt/diag.h"
#include "nxrt/kernel.h"
#include "nxrt/log.h"

namespace nxrt::kernel {
namespace {

// The physical-core mask (all SMT siblings) holding processor `p` of group 0; just that bit when the call fails.
uint64_t phys_core_of(int p) {
    DWORD len = 0;
    GetLogicalProcessorInformationEx(RelationProcessorCore, nullptr, &len);
    if (len) {
        std::string buf(len, '\0');
        auto* base = reinterpret_cast<PSYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>(buf.data());
        if (GetLogicalProcessorInformationEx(RelationProcessorCore, base, &len))
            for (DWORD off = 0; off < len;) {
                auto* e = reinterpret_cast<PSYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>(buf.data() + off);
                if (e->Relationship == RelationProcessorCore && e->Processor.GroupMask[0].Group == 0 && (e->Processor.GroupMask[0].Mask >> p & 1))
                    return static_cast<uint64_t>(e->Processor.GroupMask[0].Mask);
                off += e->Size;
            }
    }
    return 1ull << p;
}

int low_bit(uint64_t m) {
    for (int b = 0; b < 64; b++)
        if (m >> b & 1) return b;
    return 0;
}

int ideal_of(HANDLE h) {
    PROCESSOR_NUMBER pn{};
    return GetThreadIdealProcessorEx(h, &pn) ? pn.Number : -1;
}

void place(const ThreadRecord& r, const std::string& how, const SchedInfo& si) {
    HANDLE h = OpenThread(THREAD_SET_INFORMATION | THREAD_QUERY_INFORMATION, FALSE, r.host_tid);
    if (!h) {
        NXLOG_W("sched", "render_place: cannot open %s (tid %u): error %lu", r.name.c_str(), r.host_tid, GetLastError());
        return;
    }
    const int before = ideal_of(h);
    uint64_t aff = si.pool;
    int ideal = -1;
    if (how == "core5" || how == "apart") {
        const int last = si.cores >= 6 ? 5 : si.cores - 1;
        ideal = low_bit(si.host_masks[last]);
        if (how == "apart") aff &= ~phys_core_of(low_bit(si.host_masks[0]));
    } else {
        const uint64_t m = _strtoui64(how.c_str(), nullptr, 16);
        if (m) aff = m, ideal = low_bit(m);
    }
    if (!aff || ideal < 0) {
        NXLOG_W("sched", "render_place = %s: nothing to apply", how.c_str());
        CloseHandle(h);
        return;
    }
    SetThreadAffinityMask(h, static_cast<DWORD_PTR>(aff));
    PROCESSOR_NUMBER pn{};
    pn.Number = static_cast<BYTE>(ideal);
    SetThreadIdealProcessorEx(h, &pn, nullptr);
    NXLOG_I("sched", "render_place = %s: %s (tid %u) ideal processor %d -> %d, affinity %llx", how.c_str(), r.name.c_str(), r.host_tid, before, ideal_of(h),
            (unsigned long long)aff);
    CloseHandle(h);
}

void placer(std::string how, bool log) {
    std::unordered_set<uint32_t> done;
    bool logged = false;
    for (int i = 0; i < 60; i++) {
        Sleep(2000);
        const SchedInfo si = sched_info();
        if (!si.active) return;
        const std::vector<ThreadRecord> recs = diag_threads();
        if (how != "off")
            for (const ThreadRecord& r : recs)
                if (r.name == "nvn-render" && done.insert(r.host_tid).second) place(r, how, si);
        if (log && !logged && i >= 14) {        // ~30 s in: the game's threads and the runtime's pools exist
            logged = true;
            std::string line;
            for (const ThreadRecord& r : recs) {
                HANDLE h = OpenThread(THREAD_QUERY_INFORMATION, FALSE, r.host_tid);
                if (!h) continue;
                char b[96];
                snprintf(b, sizeof b, " %s=%d/%s", r.name.c_str(), ideal_of(h), GetThreadPriority(h) == THREAD_PRIORITY_NORMAL ? "n" : std::to_string(GetThreadPriority(h)).c_str());
                line += b;
                CloseHandle(h);
            }
            NXLOG_I("sched", "host threads (name=ideal processor/priority):%s", line.c_str());
        }
        if ((how == "off" || !done.empty()) && (!log || logged)) return;
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
