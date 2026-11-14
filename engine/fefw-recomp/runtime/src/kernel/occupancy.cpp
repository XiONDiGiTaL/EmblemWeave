// SPDX-License-Identifier: GPL-3.0-or-later
// [kernel] occupancy = <seconds> (diagnostic, off by default; notes/threads.md): would the game gain from more job workers?
// The game's pools are known by the names it gives its threads (nn::os::SetThreadName, wrapped by thread_diag.cpp): Basic_Worker*,
// GFX_Worker*, MOCull_*. Each kernel block (WaitForAddress, ArbitrateLock, WaitProcessWideKeyAtomic) marks its thread blocked; a sampler
// every 250 us records, while main is blocked and while it runs, how many threads of each pool are not blocked (running or ready).
// If main's waits mostly see every worker of a pool busy, the pool is saturated and more workers would shorten them; if they see one or
// two, main waits on a chain of jobs and more workers cannot help.
// The guest threads also take the game's names (Basic_Worker1, ...) in the log while this is on.
#include <windows.h>

#include <atomic>
#include <cstdio>
#include <cstring>
#include <string>
#include <thread>
#include <unordered_map>
#include <vector>
#include <algorithm>

#include "kernel_internal.h"
#include "nxrt/config.h"
#include "nxrt/kernel.h"
#include "nxrt/log.h"
#include "nxrt/diag.h"

namespace nxrt::kernel {

bool g_occ_on = false;

namespace {

constexpr int kGroups = 5;   // 0 none, 1 main, 2 Basic_Worker, 3 GFX_Worker, 4 MOCull
const char* const kGroupName[kGroups] = {"-", "main", "Basic", "GFX", "MOCull"};
std::atomic<int> g_members[kGroups];
std::atomic<int> g_blocked[kGroups];
std::atomic<int> g_main_kind{-1};          // kind main is blocked in, -1 = running
int64_t g_main_t0 = 0;                     // g_k: when main blocked (QPC)
std::atomic<uint64_t> g_k_contended{0}, g_k_wait_ticks{0};   // g_k acquisitions that found it held, and the time they waited
struct Waker { uint64_t n[3] = {}, ticks[3] = {}; };
std::unordered_map<std::string, Waker> g_wakers;   // g_k: who woke main, per kind, and how long main had been blocked

int64_t qpc() {
    LARGE_INTEGER c;
    QueryPerformanceCounter(&c);
    return c.QuadPart;
}

struct Hist {
    uint64_t n = 0;
    uint64_t run[kGroups][9] = {};           // running count 0..8
};

int64_t g_period_s = 0;

void sampler() {
    HANDLE timer = CreateWaitableTimerExW(nullptr, nullptr, CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, TIMER_ALL_ACCESS);
    SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
    Hist h[4];   // main blocked on an address, on a mutex, on a condition variable; main running
    uint64_t total = 0;
    LARGE_INTEGER f, t0, now;
    QueryPerformanceFrequency(&f);
    QueryPerformanceCounter(&t0);
    for (;;) {
        if (timer) {
            LARGE_INTEGER due;
            due.QuadPart = -2500;   // 250 us
            SetWaitableTimer(timer, &due, 0, nullptr, nullptr, FALSE);
            WaitForSingleObject(timer, INFINITE);
        } else {
            Sleep(1);
        }
        if (g_members[1].load(std::memory_order_relaxed) == 0) continue;   // main not tagged yet
        const int k = g_main_kind.load(std::memory_order_relaxed);
        Hist& x = h[k < 0 ? 3 : (k > 2 ? 0 : k)];
        x.n++;
        total++;
        for (int g = 2; g < kGroups; g++) {
            int r = g_members[g].load(std::memory_order_relaxed) - g_blocked[g].load(std::memory_order_relaxed);
            r = r < 0 ? 0 : (r > 8 ? 8 : r);
            x.run[g][r]++;
        }
        QueryPerformanceCounter(&now);
        if (now.QuadPart - t0.QuadPart < g_period_s * f.QuadPart) continue;
        t0 = now;
        static const char* const kWhat[4] = {"main waits on an address", "main waits on a mutex", "main waits on a cv", "main runs"};
        std::string out;
        char b[256];
        std::snprintf(b, sizeof b, "occupancy in %llds (%llu samples): main blocked %.0f%% (address %.0f%%, mutex %.0f%%, cv %.0f%%)",
                      static_cast<long long>(g_period_s), static_cast<unsigned long long>(total), 100.0 * (h[0].n + h[1].n + h[2].n) / total,
                      100.0 * h[0].n / total, 100.0 * h[1].n / total, 100.0 * h[2].n / total);
        out += b;
        for (int w = 0; w < 4; w++) {
            if (!h[w].n || w == 2) continue;
            std::snprintf(b, sizeof b, "; %s:", kWhat[w]);
            out += b;
            for (int g = 2; g < kGroups; g++) {
                const int m = g_members[g].load(std::memory_order_relaxed);
                if (!m) continue;
                double mean = 0;
                for (int r = 0; r <= 8; r++) mean += r * static_cast<double>(h[w].run[g][r]);
                mean /= static_cast<double>(h[w].n);
                std::snprintf(b, sizeof b, " %s %.2f of %d [", kGroupName[g], mean, m);
                out += b;
                for (int r = 0; r <= m && r <= 8; r++) {
                    std::snprintf(b, sizeof b, "%s%d:%.0f%%", r ? " " : "", r, 100.0 * h[w].run[g][r] / h[w].n);
                    out += b;
                }
                out += "]";
            }
        }
        NXLOG_I("kernel", "%s", out.c_str());
        {   // who main waited for
            std::unordered_map<std::string, Waker> w;
            AcquireSRWLockExclusive(&detail::g_k);
            w.swap(g_wakers);
            ReleaseSRWLockExclusive(&detail::g_k);
            std::vector<std::pair<std::string, Waker>> v(w.begin(), w.end());
            auto tot = [](const Waker& x) { return x.ticks[0] + x.ticks[1] + x.ticks[2]; };
            std::sort(v.begin(), v.end(), [&](const auto& a, const auto& b) { return tot(a.second) > tot(b.second); });
            std::string line;
            for (size_t i = 0; i < v.size() && i < 10; i++) {
                const Waker& x = v[i].second;
                std::snprintf(b, sizeof b, "%s %s: address %llu %.1f ms, mutex %llu %.1f ms, cv %llu %.1f ms", i ? ";" : "", v[i].first.c_str(),
                              (unsigned long long)x.n[0], x.ticks[0] * 1000.0 / f.QuadPart, (unsigned long long)x.n[1], x.ticks[1] * 1000.0 / f.QuadPart,
                              (unsigned long long)x.n[2], x.ticks[2] * 1000.0 / f.QuadPart);
                line += b;
            }
            NXLOG_I("kernel", "main woken in %llds by (blocked time ended by each waker):%s", static_cast<long long>(g_period_s), line.c_str());
            const uint64_t nc = g_k_contended.exchange(0), wt = g_k_wait_ticks.exchange(0);
            NXLOG_I("kernel", "kernel lock (g_k) in %llds: %llu contended acquisitions, %.1f ms waiting in all (%.1f us each)", static_cast<long long>(g_period_s),
                    (unsigned long long)nc, wt * 1000.0 / f.QuadPart, nc ? wt * 1e6 / f.QuadPart / nc : 0.0);
        }
        for (Hist& y : h) y = Hist{};
        total = 0;
    }
}

}  // namespace

bool occupancy_wanted() {
    static const bool on = [] {
        g_period_s = config_get_int("kernel", "occupancy", 0);
        return g_period_s > 0;
    }();
    return on;
}

void occ_block(const KThread& t, int kind) {
    if (t.occ_group <= 0) return;
    if (t.occ_group == 1) g_main_kind.store(kind, std::memory_order_relaxed), g_main_t0 = qpc();
    else g_blocked[t.occ_group].fetch_add(1, std::memory_order_relaxed);
}

void occ_unblock(const KThread& t) {
    if (t.occ_group <= 0) return;
    if (t.occ_group == 1) g_main_kind.store(-1, std::memory_order_relaxed);
    else g_blocked[t.occ_group].fetch_sub(1, std::memory_order_relaxed);
}

void occ_name(KThread& t, const char* game_name) {
    if (!occupancy_wanted()) return;
    int g = 0;
    if (!std::strncmp(game_name, "Basic_Worker", 12)) g = 2;
    else if (!std::strncmp(game_name, "GFX_Worker", 10)) g = 3;
    else if (!std::strncmp(game_name, "MOCull_", 7)) g = 4;
    if (!t.started) t.name = game_name;   // not started yet: nothing else reads it
    if (!g || t.occ_group) return;
    t.occ_group = g;
    g_members[g].fetch_add(1, std::memory_order_relaxed);
}

void occ_lock_contended() {
    const int64_t t0 = qpc();
    AcquireSRWLockExclusive(&detail::g_k);
    g_k_contended.fetch_add(1, std::memory_order_relaxed);
    g_k_wait_ticks.fetch_add(static_cast<uint64_t>(qpc() - t0), std::memory_order_relaxed);
}

void occ_main_woken() {
    const int k = g_main_kind.load(std::memory_order_relaxed);
    if (k < 0 || k > 2) return;
    KThread* w = current_thread();
    const char* name = w ? w->name.c_str() : log_thread_name();
    Waker& x = g_wakers[name && *name ? name : "host"];
    x.n[k]++;
    x.ticks[k] += static_cast<uint64_t>(qpc() - g_main_t0);
}

void install_occupancy() {
    if (!occupancy_wanted()) return;
    g_occ_on = true;
    std::thread(sampler).detach();
    NXLOG_I("kernel", "[kernel] occupancy: pool occupancy while main is blocked, logged every %lld s", static_cast<long long>(g_period_s));
}

void occ_tag_main(KThread& t) {
    if (!g_occ_on || t.occ_group) return;
    t.occ_group = 1;
    g_members[1].fetch_add(1, std::memory_order_relaxed);
}

}  // namespace nxrt::kernel
