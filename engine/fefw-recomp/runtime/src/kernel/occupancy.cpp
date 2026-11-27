// SPDX-License-Identifier: GPL-3.0-or-later
// [kernel] occupancy = <seconds> (diagnostic, off by default; notes/threads.md): would the game gain from more job workers?
// The game's pools are known by the names it gives its threads (nn::os::SetThreadName, wrapped by thread_diag.cpp): Basic_Worker*,
// GFX_Worker*, MOCull_*. Each kernel block (WaitForAddress, ArbitrateLock, WaitProcessWideKeyAtomic) marks its thread blocked; a sampler
// every 250 us records, while main is blocked and while it runs, how many threads of each pool are not blocked (running or ready).
// If main's waits mostly see every worker of a pool busy, the pool is saturated and more workers would shorten them; if they see one or
// two, main waits on a chain of jobs and more workers cannot help.
// The guest threads also take the game's names (Basic_Worker1, ...) in the log while this is on.
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
#include "nxrt/platform.h"

namespace nxrt::kernel {

bool g_occ_on = false;

namespace {

constexpr int kGroups = 5;   // 0 none, 1 main, 2 Basic_Worker, 3 GFX_Worker, 4 MOCull
const char* const kGroupName[kGroups] = {"-", "main", "Basic", "GFX", "MOCull"};
std::atomic<int> g_members[kGroups];
std::atomic<int> g_blocked[kGroups];
std::atomic<int> g_main_kind{-1};          // kind main is blocked in, -1 = running
int64_t g_main_t0 = 0;                     // when main blocked (platform clock ticks)
// Sharded kernel lock contention tracking (sync, mutex, cv, addr)
std::atomic<uint64_t> g_sync_contended{0}, g_sync_wait_ticks{0};
std::atomic<uint64_t> g_mutex_contended{0}, g_mutex_wait_ticks{0};
std::atomic<uint64_t> g_cv_contended{0}, g_cv_wait_ticks{0};
std::atomic<uint64_t> g_addr_contended{0}, g_addr_wait_ticks{0};
struct Waker { uint64_t n[3] = {}, ticks[3] = {}; };
std::unordered_map<std::string, Waker> g_wakers;   // who woke main, per kind, and how long main had been blocked

int64_t now_ticks() {
    return static_cast<int64_t>(platform::clock_ticks());
}

struct Hist {
    uint64_t n = 0;
    uint64_t run[kGroups][9] = {};           // running count 0..8
};

int64_t g_period_s = 0;

void sampler() {
    platform::thread_set_priority(platform::thread_self(), platform::ThreadPriority::Highest);
    Hist h[4];   // main blocked on an address, on a mutex, on a condition variable; main running
    uint64_t total = 0;
    const uint64_t freq = platform::clock_hz();
    int64_t t0 = now_ticks();
    for (;;) {
        platform::sleep_ns(250000);  // 250 us
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
        int64_t now = now_ticks();
        if (now - t0 < g_period_s * static_cast<int64_t>(freq)) continue;
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
            nxrt::platform::mutex_lock(detail::g_klocks.sync);
            w.swap(g_wakers);
            nxrt::platform::mutex_unlock(detail::g_klocks.sync);
            std::vector<std::pair<std::string, Waker>> v(w.begin(), w.end());
            auto tot = [](const Waker& x) { return x.ticks[0] + x.ticks[1] + x.ticks[2]; };
            std::sort(v.begin(), v.end(), [&](const auto& a, const auto& b) { return tot(a.second) > tot(b.second); });
            std::string line;
            for (size_t i = 0; i < v.size() && i < 10; i++) {
                const Waker& x = v[i].second;
                std::snprintf(b, sizeof b, "%s %s: address %llu %.1f ms, mutex %llu %.1f ms, cv %llu %.1f ms", i ? ";" : "", v[i].first.c_str(),
                              (unsigned long long)x.n[0], x.ticks[0] * 1000.0 / freq, (unsigned long long)x.n[1], x.ticks[1] * 1000.0 / freq,
                              (unsigned long long)x.n[2], x.ticks[2] * 1000.0 / freq);
                line += b;
            }
            NXLOG_I("kernel", "main woken in %llds by (blocked time ended by each waker):%s", static_cast<long long>(g_period_s), line.c_str());
            // Report contention for each sharded lock
            const uint64_t sync_c = g_sync_contended.exchange(0), sync_w = g_sync_wait_ticks.exchange(0);
            const uint64_t mutex_c = g_mutex_contended.exchange(0), mutex_w = g_mutex_wait_ticks.exchange(0);
            const uint64_t cv_c = g_cv_contended.exchange(0), cv_w = g_cv_wait_ticks.exchange(0);
            const uint64_t addr_c = g_addr_contended.exchange(0), addr_w = g_addr_wait_ticks.exchange(0);
            NXLOG_I("kernel", "kernel locks in %llds: sync %llu contended (%.1f ms, %.1f us each), mutex %llu contended (%.1f ms, %.1f us each), cv %llu contended (%.1f ms, %.1f us each), addr %llu contended (%.1f ms, %.1f us each)",
                    static_cast<long long>(g_period_s),
                    (unsigned long long)sync_c, sync_w * 1000.0 / freq, sync_c ? sync_w * 1e6 / freq / sync_c : 0.0,
                    (unsigned long long)mutex_c, mutex_w * 1000.0 / freq, mutex_c ? mutex_w * 1e6 / freq / mutex_c : 0.0,
                    (unsigned long long)cv_c, cv_w * 1000.0 / freq, cv_c ? cv_w * 1e6 / freq / cv_c : 0.0,
                    (unsigned long long)addr_c, addr_w * 1000.0 / freq, addr_c ? addr_w * 1e6 / freq / addr_c : 0.0);
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
    if (t.occ_group == 1) g_main_kind.store(kind, std::memory_order_relaxed), g_main_t0 = now_ticks();
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
    const int64_t t0 = now_ticks();
    nxrt::platform::mutex_lock(detail::g_klocks.sync);
    g_sync_contended.fetch_add(1, std::memory_order_relaxed);
    g_sync_wait_ticks.fetch_add(static_cast<uint64_t>(now_ticks() - t0), std::memory_order_relaxed);
    nxrt::platform::mutex_unlock(detail::g_klocks.sync);
}

void occ_main_woken() {
    const int k = g_main_kind.load(std::memory_order_relaxed);
    if (k < 0 || k > 2) return;
    KThread* w = current_thread();
    const char* name = w ? w->name.c_str() : log_thread_name();
    Waker& x = g_wakers[name && *name ? name : "host"];
    x.n[k]++;
    x.ticks[k] += static_cast<uint64_t>(now_ticks() - g_main_t0);
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
