// SPDX-License-Identifier: GPL-3.0-or-later
// Aging for guest threads that the host scheduler leaves ready but never runs (S20, debug note 422).
//
// The console core model pins every guest thread to one host processor and maps the HOS priorities onto six host bands (Windows
// THREAD_PRIORITY_*, POSIX nice levels) with the wake-up boost off (kernel/sched.cpp, debug notes 180 and 221). That keeps the console's
// strict priority order, and it has a price on a shared PC: the lowest bands rank at or below ordinary programs, so while a build or
// another program keeps the pinned processor busy, a thread in those bands stays ready and gets no time. The resource loader sits in the
// lowest band: with a normal-priority load on its processor the game's load never finishes (the frames keep coming, the loader sits in its
// decompress loop) and a scripted run reports a "hang".
//
// This guard asks the platform which of our threads are in the ready state and how often each was switched in (platform::host_snapshot:
// NtQuerySystemInformation on Windows, /proc/self/task on Linux). A guest thread that was ready at several polls in a row without a single
// context switch in between gets the host's top priority for 450 ms (and its yields stop handing the processor over, which cost it most of
// a boost: a yield gives a time slice to any ready thread, a lower priority one included), then goes back to its band. Healthy runs never
// starve for that long, so nothing changes for them: it is an escape from starvation, not a second scheduler. Under [sched] affinity =
// strict the boost also widens the thread's affinity to the whole processor pool for its length: a higher priority alone does not help a
// thread pinned to a processor that another thread at the top band (the highest is 15 in the high priority class, the same as the boost)
// or DPC work holds; any idle processor does. On POSIX the top band is nice -20, which an unprivileged process may be refused: the affinity
// widening still applies and the log says the priority change was refused. Config: [sched] starvation_guard (default on), [sched]
// starvation_ms (default 100, time a thread must have been ready and unscheduled before it is boosted), [sched] starvation_poll_ms
// (default 50).
#include <algorithm>
#include <atomic>
#include <chrono>
#include <condition_variable>
#include <cstdio>
#include <cstdint>
#include <deque>
#include <memory>
#include <mutex>
#include <string>
#include <thread>
#include <unordered_map>
#include <vector>

#include "nxrt/config.h"
#include "nxrt/diag.h"
#include "nxrt/gauge.h"
#include "nxrt/kernel.h"
#include "nxrt/log.h"
#include "nxrt/platform.h"

namespace nxrt::kernel {
namespace {

using HostSnapshot = nxrt::platform::HostSnapshot;

struct Guard {
    std::mutex m;
    std::condition_variable cv;
    std::thread th;
    bool stop = false;
    std::atomic<uint64_t> boosts{0};
};

Guard& guard() {
    static Guard* g = new Guard;     // never destroyed: a joinable thread must not die with the static
    return *g;
}

struct Seen {
    uint64_t switches = 0;
    int ready_polls = 0;
    int boost_polls = 0;
    bool boosted = false;
};
constexpr int kBoostMs = 450;      // a boost lasts this long (rounded up to whole polls)

const char* band_name(nxrt::platform::ThreadPriority host_prio) {
    switch (host_prio) {
        case nxrt::platform::ThreadPriority::TimeCritical: return "time critical";
        case nxrt::platform::ThreadPriority::Highest: return "highest";
        case nxrt::platform::ThreadPriority::AboveNormal: return "above normal";
        case nxrt::platform::ThreadPriority::Normal: return "normal";
        case nxrt::platform::ThreadPriority::BelowNormal: return "below normal";
        case nxrt::platform::ThreadPriority::Lowest: return "lowest";
        default: return "?";
    }
}

// One line: the starved thread, its band and console cores, and the busiest threads of this process and the busiest other programs between
// the two snapshots (ms of CPU). A starver of ours shows as a thread with about the whole window; none: another program or the system.
// The thread's host placement when it was found starved (before the boost changed it): affinity, ideal processor, the processor of its last SVC.
std::string placement(const KThread& t) {
    char ideal[24] = "?";
    const int p = nxrt::platform::host_thread_ideal_processor(t.host_tid);
    if (p >= 0) std::snprintf(ideal, sizeof ideal, "%d", p);
    char buf[128];
    std::snprintf(buf, sizeof buf, "host affinity 0x%llx, ideal processor %s, last ran on processor %d (at its last SVC)",
                  static_cast<unsigned long long>(t.host_affinity.load(std::memory_order_relaxed)), ideal, t.last_proc.load(std::memory_order_relaxed));
    return buf;
}

void log_starvation(const KThread& t, const std::string& where, int ready_ms, const HostSnapshot& a, const HostSnapshot& b, uint64_t boost_no,
                    bool boost_refused) {
    std::unordered_map<uint32_t, std::string> names;
    for (const ThreadRecord& r : diag_threads()) names[r.host_tid] = r.name;
    std::vector<std::pair<uint64_t, uint32_t>> mine;
    for (const auto& th : b.threads) {
        auto it = std::find_if(a.threads.begin(), a.threads.end(), [&](const HostSnapshot::Thread& x) { return x.id == th.id; });
        const uint64_t d = (it != a.threads.end() && it->cpu_100ns <= th.cpu_100ns) ? th.cpu_100ns - it->cpu_100ns : 0;
        if (d) mine.emplace_back(d, th.id);
    }
    std::sort(mine.begin(), mine.end(), [](const auto& x, const auto& y) { return x.first > y.first; });
    std::vector<std::pair<uint64_t, uint32_t>> procs;
    for (const auto& pr : b.processes) {
        auto it = std::find_if(a.processes.begin(), a.processes.end(), [&](const HostSnapshot::Proc& x) { return x.id == pr.id; });
        if (it == a.processes.end() || it->cpu_100ns > pr.cpu_100ns) continue;
        if (pr.cpu_100ns - it->cpu_100ns) procs.emplace_back(pr.cpu_100ns - it->cpu_100ns, pr.id);
    }
    std::sort(procs.begin(), procs.end(), [](const auto& x, const auto& y) { return x.first > y.first; });
    std::string busy, other;
    char buf[160];
    for (size_t i = 0; i < mine.size() && i < 5; i++) {
        auto it = names.find(mine[i].second);
        const std::string n = it != names.end() ? it->second : "host thread " + std::to_string(mine[i].second);
        std::snprintf(buf, sizeof buf, "%s%s %.0f", busy.empty() ? "" : ", ", n.c_str(), static_cast<double>(mine[i].first) / 10000.0);
        busy += buf;
    }
    for (size_t i = 0; i < procs.size() && i < 3; i++) {
        std::string n;
        nxrt::platform::process_name(procs[i].second, &n);
        if (n.empty()) n = procs[i].second == 0 ? "idle" : "pid " + std::to_string(procs[i].second);
        std::snprintf(buf, sizeof buf, "%s%s %.0f", other.empty() ? "" : ", ", n.c_str(), static_cast<double>(procs[i].first) / 10000.0);
        other += buf;
    }
    NXLOG_W("sched", "starved: %s (console cores 0x%llx, band %s; %s) was ready for %d ms without running: boosted for %d ms (boost %llu%s). CPU ms in that window: "
            "busiest threads of this process: %s; busiest other processes: %s",
            t.name.c_str(), static_cast<unsigned long long>(t.core_mask), band_name(sched_host_priority(t.prio)), where.c_str(), ready_ms, kBoostMs,
            static_cast<unsigned long long>(boost_no), boost_refused ? ", the host refused the priority raise" : "", busy.empty() ? "none" : busy.c_str(),
            other.empty() ? "none" : other.c_str());
}

// [sched] starvation_lean (default true): the guard costs 1.4-1.6 ms of CPU per frame at the top band in a clean run (a snapshot of every
// process on the PC every 50 ms) and boosted nothing in any tester run. Lean: while no guest thread was ready-and-unscheduled at the last
// poll the next one comes after [sched] starvation_idle_poll_ms (default 200) instead of starvation_poll_ms; once one is seen, polls are
// starvation_poll_ms apart again. A starvation is then caught after up to idle_poll + starvation_ms instead of poll + starvation_ms.
void loop(Guard* g, int poll_ms, int need_polls, bool lean, int idle_poll_ms) {
    log_set_thread_name("starve-guard");
    nxrt::platform::host_thread_set_priority(nxrt::platform::thread_id(),
                                             static_cast<int>(nxrt::platform::ThreadPriority::Highest));   // it must run when everything else is starved
    std::unordered_map<uint32_t, Seen> seen;       // by host thread id
    const int boost_polls = std::max(1, (kBoostMs + poll_ms - 1) / poll_ms);
    std::deque<HostSnapshot> hist;                 // the last need_polls snapshots, oldest first
    uint64_t last_line = 0;
    uint64_t quiet = 0;                            // boosts not logged since the last line (at most one line per second)
    bool watching = true;                          // lean: some guest thread was ready without a switch at the last poll (or a boost runs)
    bool priority_refused = false;
    for (;;) {
        {
            std::unique_lock<std::mutex> lk(g->m);
            const int wait_ms = lean && !watching ? idle_poll_ms : poll_ms;
            if (g->cv.wait_for(lk, std::chrono::milliseconds(wait_ms), [g] { return g->stop; })) return;
        }
        watching = false;
        HostSnapshot snap;
        if (!nxrt::platform::host_snapshot(&snap)) continue;
        std::unordered_map<uint32_t, const HostSnapshot::Thread*> by_tid;
        for (const HostSnapshot::Thread& th : snap.threads) by_tid[th.id] = &th;
        hist.push_back(snap);
        while (hist.size() > static_cast<size_t>(need_polls)) hist.pop_front();
        for (const std::shared_ptr<KThread>& t : threads()) {
            if (!t->started || t->exited || !t->host_thread || !t->host_tid) continue;
            auto it = by_tid.find(t->host_tid);
            Seen& s = seen[t->host_tid];
            if (it == by_tid.end()) {
                s = Seen{};
                continue;
            }
            const HostSnapshot::Thread& r = *it->second;
            if (r.ready && r.switches == s.switches) s.ready_polls++;
            else s.ready_polls = r.ready ? 1 : 0;
            if (s.ready_polls >= 1 || s.boosted) watching = true;   // ready now: poll again soon
            s.switches = r.switches;
            if (s.boosted) {                                    // boosted: keep it for boost_polls polls, then back to its band
                if (++s.boost_polls >= boost_polls) {
                    sched_apply(*t);                            // its band, and under strict its own processor again
                    t->aged = false;
                    s.boosted = false;
                    s.ready_polls = 0;
                }
            } else if (s.ready_polls >= need_polls) {
                const std::string where = placement(*t);
                const SchedInfo si = sched_info();
                if (si.active && si.pool) {                     // strict: any idle processor of the pool; soft: already the pool
                    if (nxrt::platform::host_thread_set_affinity(t->host_tid, si.pool))
                        t->host_affinity.store(si.pool, std::memory_order_relaxed);
                }
                const bool raised = nxrt::platform::host_thread_set_priority(t->host_tid, static_cast<int>(nxrt::platform::ThreadPriority::TimeCritical));
                if (!raised && !priority_refused) {
                    priority_refused = true;
                    NXLOG_W("sched", "the host refuses a starving thread the top priority (POSIX needs privilege for nice -20): only its affinity is widened");
                }
                t->aged = true;                                 // its yields keep the processor while it is aged
                s.boosted = true;
                s.boost_polls = 0;
                const uint64_t n = ++g->boosts;
                const uint64_t now = nxrt::platform::tick_count_ms();
                if (now - last_line >= 1000) {          // every boost names its starver, at most one line per second
                    last_line = now;
                    if (quiet) NXLOG_W("sched", "(%llu more starvation boosts since the last line)", static_cast<unsigned long long>(quiet));
                    quiet = 0;
                    log_starvation(*t, where, (s.ready_polls - 1) * poll_ms, hist.front(), hist.back(), n, !raised);
                } else {
                    ++quiet;
                }
            }
        }
    }
}

}  // namespace

void starvation_guard_start() {
    if (!config_get_bool("sched", "starvation_guard", true)) {
        NXLOG_I("sched", "[sched] starvation_guard = 0: guest threads the host leaves ready are not aged");
        return;
    }
    Guard& g = guard();
    std::lock_guard<std::mutex> lk(g.m);
    if (g.th.joinable()) return;
    // A thread counts as starved when it was ready at `need` polls in a row with no context switch in between: proven ready for (need - 1)
    // polls. The old 150 ms poll with two polls let a starved game thread sit 150..300 ms (the ~230 ms hitches of the p1 soak line up with
    // boosts of main and guest-89); a 50 ms poll and 100 ms catch it after 100..150 ms.
    const int ms = static_cast<int>(std::max<int64_t>(1, config_get_int("sched", "starvation_ms", 100)));
    const int poll = static_cast<int>(std::clamp<int64_t>(config_get_int("sched", "starvation_poll_ms", 50), 10, 1000));
    const int need = (ms + poll - 1) / poll + 1;
    g.stop = false;
    const bool lean = config_get_bool("sched", "starvation_lean", true);
    const int idle = static_cast<int>(std::clamp<int64_t>(config_get_int("sched", "starvation_idle_poll_ms", 200), poll, 1000));
    g.th = std::thread(loop, &g, poll, need, lean, idle);
    NXLOG_I("sched", "starvation guard on: a guest thread ready for %d ms without running (polled every %d ms%s) is boosted to the top band on the whole processor pool for %d ms (debug note 422)",
            (need - 1) * poll, poll, lean ? ", every starvation_idle_poll_ms while nothing is ready: [sched] starvation_lean" : "", kBoostMs);
}

void starvation_guard_stop() {
    Guard& g = guard();
    {
        std::lock_guard<std::mutex> lk(g.m);
        if (!g.th.joinable()) return;
        g.stop = true;
    }
    g.cv.notify_all();
    g.th.join();
}

bool starvation_guard_running() {
    Guard& g = guard();
    std::lock_guard<std::mutex> lk(g.m);
    return g.th.joinable();
}

uint64_t starvation_boosts() { return guard().boosts.load(); }

namespace {
// Gauge (include/nxrt/gauge.h). The guard object is never destroyed (see guard()), so the callback stays valid through exit.
const bool g_gauge_boosts_registered = [] {
    nxrt::gauge_register("kernel.starvation_boosts", [] { return static_cast<int64_t>(starvation_boosts()); });
    nxrt::hitch_add_counter("starvation_boosts", []() -> uint64_t { return starvation_boosts(); });   // the hitch line: a slow frame that ended with a boost
    return true;
}();
}  // namespace

}  // namespace nxrt::kernel
