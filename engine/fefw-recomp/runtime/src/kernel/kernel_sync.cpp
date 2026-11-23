// SPDX-License-Identifier: GPL-3.0-or-later
// Kernel-level synchronisation with the semantics of the Horizon SVCs: ArbitrateLock/Unlock (guest mutex words),
// WaitProcessWideKeyAtomic/SignalProcessWideKey (condition variables), WaitForAddress/SignalToAddress, WaitSynchronization and
// CancelSynchronization. The recompiled SDK's nn::os mutexes and condition variables sit on these.
// Logic ported from SuperMarioWonderRecomp runtime/src/kernel.cpp (MIT, Copyright (c) 2026 BelmanteGu; see THIRD_PARTY_NOTICES.md),
// reworked: one kernel lock, a condition variable per thread, waiters woken in priority order, a pending
// CancelSynchronization is remembered until the next wait.

#include <algorithm>
#include <cctype>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <functional>
#include <string>
#include <atomic>
#include <thread>
#include <unordered_map>
#include <unordered_set>
#include <vector>

#include "kernel_internal.h"
#include "nxrt/config.h"
#include "nxrt/gauge.h"
#include "nxrt/kernel.h"
#include "nxrt/log.h"

namespace nxrt::kernel {
namespace detail {

// One kernel lock for every wait queue (thread.cpp and occupancy.cpp take the same lock).
detail::KernelLock g_k;

namespace {
std::vector<KThread*> g_sync_waiters;    // in WaitSynchronization: woken by any change of waitable state
std::vector<KThread*> g_mutex_waiters;   // waiting for a guest mutex word to be handed over
std::unordered_set<uint64_t> g_broadcast_mutexes; // mutexes that had a condvar broadcast
std::vector<KThread*> g_cv_waiters;      // waiting on a process-wide key
std::vector<KThread*> g_addr_waiters;    // WaitForAddress

// Per-lock Guard classes
// All wait queues share the one kernel lock: the sharded queues left a window between handing a
// mutex over and queueing its waiter, and an unlock in that window lost the wakeup. thread.cpp and
// occupancy.cpp already take g_k, so the queues and the rest of the kernel stay consistent.
struct GuardMutex {
    GuardMutex() { g_k.lock(); }
    ~GuardMutex() { g_k.unlock(); }
};

struct GuardCV {
    GuardCV() { g_k.lock(); }
    ~GuardCV() { g_k.unlock(); }
};

struct GuardAddr {
    GuardAddr() { g_k.lock(); }
    ~GuardAddr() { g_k.unlock(); }
};

struct GuardSync {
    GuardSync() { g_k.lock(); }
    ~GuardSync() { g_k.unlock(); }
};

// Legacy Guard for compatibility (stats, sync_reset, etc.)
struct Guard {
    Guard() { g_k.lock(); }
    ~Guard() { g_k.unlock(); }
};

void remove_from(std::vector<KThread*>& v, KThread* t) { v.erase(std::remove(v.begin(), v.end(), t), v.end()); }

uint32_t load32(uint64_t a) { return __atomic_load_n(reinterpret_cast<uint32_t*>(a), __ATOMIC_SEQ_CST); }
void store32(uint64_t a, uint32_t v) { __atomic_store_n(reinterpret_cast<uint32_t*>(a), v, __ATOMIC_SEQ_CST); }
bool cas32(uint64_t a, uint32_t expected, uint32_t v) {
    return __atomic_compare_exchange_n(reinterpret_cast<uint32_t*>(a), &expected, v, false, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
}

// The calling guest thread, or a per-host-thread stand-in (tests and host threads that call kernel functions directly).
KThread& self() {
    if (KThread* t = current_thread()) return *t;
    static thread_local KThread host;
    return host;
}

// Highest priority (lowest number) first, FIFO among equals: the order in which Horizon hands a mutex over or signals a key.
KThread* pick_best(std::vector<KThread*>& v, uint64_t KThread::*field, uint64_t value) {
    KThread* best = nullptr;
    for (KThread* t : v)
        if (t->*field == value && (!best || t->prio < best->prio)) best = t;
    return best;
}

// Hands the mutex word at addr to its best waiter (or clears it). g_k held.
// If this mutex had a condvar broadcast, wake all waiters.
void unlock_mutex(uint64_t addr) {
    // Collect all waiters for this mutex
    std::vector<KThread*> waiters;
    for (KThread* t : g_mutex_waiters) {
        if (t->wait_mutex == addr) {
            waiters.push_back(t);
        }
    }
    if (waiters.empty()) {
        store32(addr, 0);
        g_broadcast_mutexes.erase(addr);
        return;
    }
    // Sort by priority (lower value = higher priority)
    std::sort(waiters.begin(), waiters.end(), [](KThread* a, KThread* b) { return a->prio < b->prio; });

    // Check if this is a broadcast mutex
    const bool is_broadcast = g_broadcast_mutexes.erase(addr) > 0;

    if (is_broadcast) {
        // For broadcast: wake all waiters, transferring mutex to the highest priority one
        // The mutex word will be set to the highest priority waiter's tag with waiters bit
        // since there are more waiters (they'll be woken and contend for the mutex)
        uint32_t new_val = waiters[0]->wait_tag | kHandleWaitMask;
        uint32_t cur = load32(addr);
        while (!cas32(addr, cur, new_val)) {
            cur = load32(addr);
            if (!(cur & kHandleWaitMask)) {
                new_val = waiters[0]->wait_tag;
            }
        }
        // Remove all waiters from the global list
        for (KThread* t : waiters) {
            remove_from(g_mutex_waiters, t);
        }
        // Wake all waiters
        for (KThread* t : waiters) {
            wake_thread(*t);
        }
    } else {
        // Normal: wake only the best waiter
        KThread* next = waiters[0];
        int more = static_cast<int>(waiters.size()) - 1;
        uint32_t new_val = next->wait_tag | (more ? kHandleWaitMask : 0);
        uint32_t cur = load32(addr);
        while (!cas32(addr, cur, new_val)) {
            cur = load32(addr);
            if (!(cur & kHandleWaitMask)) {
                new_val = next->wait_tag;
            }
        }
        remove_from(g_mutex_waiters, next);
        wake_thread(*next);
    }
}

// Monotonic nanoseconds from the registered console's tick frequency, split so the multiplication cannot overflow.
int64_t now_ns() {
    const uint64_t t = system_tick();
    const uint64_t hz = tick_hz();
    return static_cast<int64_t>((t / hz) * 1000000000ull + (t % hz) * 1000000000ull / hz);
}

// set_blocked_hook: the host thread that runs a hook while it is blocked in a kernel wait, and the hook.
std::atomic<uint32_t> g_blocked_tid{0};
std::atomic<void (*)()> g_blocked_fn{nullptr};

int64_t qpc_now() {
    return static_cast<int64_t>(nxrt::platform::clock_ticks());
}

// [kernel] wait_stats = <seconds> (env NXRT_KERNEL_WAIT_STATS, default 0 = off): per guest thread, how often it blocked in WaitForAddress,
// ArbitrateLock and WaitProcessWideKeyAtomic, for how long, and the wake latency (wake_thread to the thread running again with the kernel
// lock: the host's wake-up plus the lock), logged every <seconds>. Counted under g_k.
struct WaitStats {
    std::string name;
    int prio = 0, ideal = 0;
    uint64_t mask = 0;
    uint64_t waits[3] = {}, blocked[3] = {};           // per kind: blocks, QPC ticks blocked
    uint64_t woken = 0, spun = 0, lat_sum = 0, lat_max = 0, hist[8] = {};
};
constexpr int64_t kLatBoundsUs[7] = {5, 10, 20, 50, 100, 200, 500};
std::atomic<std::unordered_map<const KThread*, WaitStats>*> g_stats{nullptr};
std::unordered_map<const KThread*, int64_t> g_woken_at;   // g_k held; only while wait_stats is on

// [kernel] spin_us (env NXRT_KERNEL_SPIN_US, default 0 = off): a thread about to sleep in one of those waits first polls its wake flag
// this long with g_k released, so a signal that comes within it reaches the thread without the host's sleep and wake-up.
// [kernel] spin_threads / lock_spin_threads (comma lists of game thread name prefixes, `main` = the main thread; empty = every thread): only
// those threads spin (spin_us) or poll a held mutex (lock_spin_us). spin_us for every thread was a loss in Dagsion (cpu-1 H: the job workers
// and the GFX workers spin each time they go idle, on the processors and SMT siblings the game and render threads need); the waits that the
// frame waits for are main's and the job workers' it waits on.
struct WaitTuning {
    int64_t hz = 1, spin = 0, stats_s = 0, lock_spin = 0;
    std::vector<std::string> spin_names, lock_names;      // empty: every thread
};
std::vector<std::string> name_list(const std::string& csv) {
    std::vector<std::string> v;
    size_t i = 0;
    while (i <= csv.size()) {
        size_t j = csv.find(',', i);
        if (j == std::string::npos) j = csv.size();
        std::string w = csv.substr(i, j - i);
        while (!w.empty() && w.front() == ' ') w.erase(w.begin());
        while (!w.empty() && w.back() == ' ') w.pop_back();
        if (!w.empty()) v.push_back(w);
        i = j + 1;
    }
    return v;
}
bool name_listed(const std::vector<std::string>& v, const char* name) {
    for (const std::string& w : v)
        if (!std::strncmp(name, w.c_str(), w.size())) return true;
    return false;
}
void stats_report_loop(int64_t seconds);
// Physical cores of processor group 0 (SMT siblings counted once). 0 when the call fails.
int physical_core_count() {
    return nxrt::platform::physical_core_count();
}
// [kernel] wait_spin (default auto): the measured spin set (PERF_NOTES, waits track: 0.4-0.8 ms off main's frame work in the street,
// 10-35 % fewer frames over 20 ms on a Ryzen 3700X) as the defaults of spin_us = 30, spin_threads = main,Basic_Worker, lock_spin_us = 30
// and lock_spin_threads = main; each of those keys set in the ini or the environment still wins. A spinning thread keeps its core busy
// instead of sleeping, so auto turns it on only with 8 or more physical cores (the six guest cores, the render thread and the driver keep
// a smaller CPU busy already; a Steam Deck has 4). true / false force it.
bool wait_spin_on() {
    std::string v = config_get_str("kernel", "wait_spin", "auto");
    for (char& c : v) c = static_cast<char>(std::tolower(static_cast<unsigned char>(c)));
    if (v.empty() || v == "auto") {
        const int cores = physical_core_count();
        const bool r = cores >= 8;
        NXLOG_I("kernel", "[kernel] wait_spin = auto: %s (%d physical cores)", r ? "on" : "off", cores);
        return r;
    }
    return !(v == "false" || v == "0" || v == "off" || v == "no");
}
const WaitTuning& wait_tuning() {      // first called before g_k is taken (config reads log)
    static const WaitTuning w = [] {
        WaitTuning v;
        v.hz = nxrt::platform::clock_hz();
        const bool spin_set = wait_spin_on();
        const int64_t us = config_get_int("kernel", "spin_us", spin_set ? 30 : 0);
        v.spin = us > 0 ? us * v.hz / 1000000 : 0;
        v.stats_s = config_get_int("kernel", "wait_stats", 0);
        // [kernel] lock_spin_us (experiment, notes/threads.md): ArbitrateLock first polls the mutex word this long without the kernel lock and
        // returns (the guest retries its compare-and-swap) when the owner has released it, before it queues as a waiter. A queued waiter is
        // handed the mutex and owns it until the host wakes it (5-10 us), so contended locks convoy; a spinner never makes that hand-over.
        const int64_t lus = config_get_int("kernel", "lock_spin_us", spin_set ? 30 : 0);
        v.lock_spin = lus > 0 ? lus * v.hz / 1000000 : 0;
        const std::string sl = config_get_str("kernel", "spin_threads", spin_set ? "main,Basic_Worker" : ""),
                          ll = config_get_str("kernel", "lock_spin_threads", spin_set ? "main" : "");
        v.spin_names = name_list(sl);
        v.lock_names = name_list(ll);
        if (v.lock_spin) NXLOG_I("kernel", "ArbitrateLock polls the mutex word %lld us before queueing ([kernel] lock_spin_us), threads: %s", static_cast<long long>(lus),
                                 ll.empty() ? "all" : ll.c_str());
        if (v.spin) NXLOG_I("kernel", "waits spin %lld us before sleeping ([kernel] spin_us), threads: %s", static_cast<long long>(us), sl.empty() ? "all" : sl.c_str());
        if (v.stats_s > 0) {
            g_stats.store(new std::unordered_map<const KThread*, WaitStats>, std::memory_order_release);
            std::thread(stats_report_loop, v.stats_s).detach();
        }
        return v;
    }();
    return w;
}

void stats_report_loop(int64_t seconds) {
    const double us_per_tick = 1e6 / static_cast<double>(wait_tuning().hz);
    for (;;) {
        nxrt::platform::sleep_ns(seconds * 1000000000);
        std::unordered_map<const KThread*, WaitStats> snap;
        g_k.lock();
        snap.swap(*g_stats.load(std::memory_order_acquire));
        g_k.unlock();
        std::vector<const WaitStats*> rows;
        for (auto& kv : snap) rows.push_back(&kv.second);
        std::sort(rows.begin(), rows.end(), [](const WaitStats* a, const WaitStats* b) {
            return a->blocked[0] + a->blocked[1] + a->blocked[2] > b->blocked[0] + b->blocked[1] + b->blocked[2];
        });
        for (const WaitStats* s : rows) {
            const uint64_t n = s->waits[0] + s->waits[1] + s->waits[2];
            if (!n) continue;
            char hist[160];
            int len = 0;
            for (int i = 0; i < 8 && s->woken; i++)
                len += std::snprintf(hist + len, sizeof hist - len, "%s%s%lld %.0f%%", i ? ", " : "", i < 7 ? "<" : ">=",
                                     static_cast<long long>(kLatBoundsUs[i < 7 ? i : 6]), 100.0 * s->hist[i] / s->woken);
            NXLOG_I("kernel", "waits in %llds: %s (prio %d, core %d of %llx) blocked %llu times %.1f ms (address %llu %.1f ms, mutex %llu %.1f ms, cv %llu %.1f ms), "
                    "spin caught %llu; woken %llu, wake latency mean %.1f us max %.0f us (us: %s)", static_cast<long long>(seconds), s->name.c_str(), s->prio,
                    s->ideal, static_cast<unsigned long long>(s->mask),
                    static_cast<unsigned long long>(n), (s->blocked[0] + s->blocked[1] + s->blocked[2]) * us_per_tick / 1000.0,
                    static_cast<unsigned long long>(s->waits[0]), s->blocked[0] * us_per_tick / 1000.0, static_cast<unsigned long long>(s->waits[1]),
                    s->blocked[1] * us_per_tick / 1000.0, static_cast<unsigned long long>(s->waits[2]), s->blocked[2] * us_per_tick / 1000.0,
                    static_cast<unsigned long long>(s->spun), static_cast<unsigned long long>(s->woken),
                    s->woken ? s->lat_sum * us_per_tick / static_cast<double>(s->woken) : 0.0, s->lat_max * us_per_tick, s->woken ? hist : "-");
        }
    }
}

}  // namespace

// One sleep on t's condition variable (lock held, released while sleeping). On the thread set_blocked_hook names it sleeps 10 ms at most and,
// when that runs out, calls the hook with lock released. Every caller loops and re-checks its condition, so a wake that comes while the hook
// runs is not lost.
void sleep_locked(KThread& t, uint32_t ms, platform::Mutex* lock) {
    void (*fn)() = g_blocked_fn.load(std::memory_order_acquire);
    if (fn && platform::thread_id() == g_blocked_tid.load(std::memory_order_relaxed)) {
        // Run the hook between 10 ms slices. The wait takes the lock again before it returns.
        uint32_t left = ms;
        while (left) {
            const uint32_t slice = std::min<uint32_t>(left, 10);
            if (platform::cond_wait_ms(thread_cv(t), lock, slice)) return;
            left -= slice;
            platform::mutex_unlock(lock);
            fn();
            platform::mutex_lock(lock);
        }
        return;
    }
    // A synchronization wait is woken by notify_waitable_changed(), not by KThread::wake.
    if (ms == UINT32_MAX) {
        platform::cond_wait(thread_cv(t), lock);
    } else {
        platform::cond_wait_ms(thread_cv(t), lock, ms);
    }
}

// Waits (g_k held, released while sleeping) until t.wake or the timeout; false on timeout.
bool block_on_wake(KThread& t, int64_t timeout_ns, int kind, platform::Mutex* lock) {
    const int64_t until = timeout_ns < 0 ? 0 : now_ns() + timeout_ns;
    if (t.wake) return true;
    const WaitTuning& wt = wait_tuning();
    auto* stats = g_stats.load(std::memory_order_relaxed);
    const bool spin_me = wt.spin && (wt.spin_names.empty() || t.spin_ok);
    const int64_t t0 = stats || spin_me ? qpc_now() : 0;
    bool spun = false, slept = false;
    if (g_occ_on) occ_block(t, kind);
    if (spin_me && timeout_ns != 0) {
        int64_t end = t0 + wt.spin;
        if (timeout_ns > 0 && timeout_ns < 1000000000) end = std::min(end, t0 + timeout_ns * wt.hz / 1000000000);
        nxrt::platform::mutex_unlock(lock);
        while (!__atomic_load_n(&t.wake, __ATOMIC_ACQUIRE) && qpc_now() < end) nxrt::platform::yield_cpu();
        nxrt::platform::mutex_lock(lock);
        spun = t.wake;
    }
    auto done = [&](bool ok) {
        if (g_occ_on) occ_unblock(t);
        if (stats) {
            const int64_t now = qpc_now();
            WaitStats& s = (*stats)[&t];
            if (s.name.empty() || (t.game_name[0] && s.name == t.name)) s.name = t.game_name[0] ? std::string(t.game_name) + " (" + t.name + ")" : t.name;
            s.prio = t.prio, s.ideal = t.ideal_core, s.mask = t.core_mask;
            s.waits[kind]++;
            s.blocked[kind] += static_cast<uint64_t>(now - t0);
            s.spun += spun;
            auto w = g_woken_at.find(&t);
            if (ok && slept && w != g_woken_at.end()) {
                const uint64_t lat = static_cast<uint64_t>(std::max<int64_t>(0, now - w->second));
                const double us = static_cast<double>(lat) * 1e6 / static_cast<double>(wt.hz);
                int b = 0;
                while (b < 7 && us >= static_cast<double>(kLatBoundsUs[b])) b++;
                s.woken++, s.lat_sum += lat, s.lat_max = std::max(s.lat_max, lat), s.hist[b]++;
            }
            if (w != g_woken_at.end()) g_woken_at.erase(w);
        }
        return ok;
    };
    while (!t.wake) {
        int64_t timeout = -1;
        if (timeout_ns >= 0) {
            int64_t rem = until - now_ns();
            if (rem <= 0) return done(false);
            timeout = rem;
        }
        // Use platform::cond_wait with timeout - for now use a simple approach
        // TODO: Add timed cond_wait to platform layer
        if (timeout >= 0) {
            // Convert to milliseconds for a simple sleep-based approach
            int64_t ms = (timeout + 999999) / 1000000;
            nxrt::platform::mutex_unlock(lock);
            nxrt::platform::sleep_ns(std::min<int64_t>(timeout, ms * 1000000));
            nxrt::platform::mutex_lock(lock);
        } else {
            nxrt::platform::cond_wait(thread_cv(t), lock);
        }
        slept = true;
    }
    return done(true);
}

void wake_thread(KThread& t, uint32_t result) {
    t.wait_result = result;                                // before the flag: a spinner reads the result once it sees the flag
    __atomic_store_n(&t.wake, true, __ATOMIC_RELEASE);     // read without g_k by a thread spinning in block_on_wake
    if (g_stats.load(std::memory_order_relaxed)) g_woken_at[&t] = qpc_now();
    if (g_occ_on && t.occ_group == 1) occ_main_woken();
    nxrt::platform::cond_signal(thread_cv(t));
}

void thread_mark_exited(KThread& t) {
    GuardSync g;
    t.exited = true;
    for (KThread* w : g_sync_waiters) nxrt::platform::cond_signal(thread_cv(*w));
}

void sync_reset() {
    Guard g;
    g_sync_waiters.clear();
    g_mutex_waiters.clear();
    g_broadcast_mutexes.clear();
    g_cv_waiters.clear();
    g_addr_waiters.clear();
}

}  // namespace detail

using namespace detail;

// The kernel lock is a process-wide member of g_k; kept for callers that initialize at startup.
void init_kernel_locks() {}

std::string sync_wait_report() {
    // Called from the watchdog thread while the process may be frozen: never block on the kernel lock.
    if (!nxrt::platform::mutex_trylock(g_k.m)) return "kernel sync report: the kernel lock is held (a thread is inside a sync call)\n";
    struct Release { ~Release() { nxrt::platform::mutex_unlock(g_k.m); } } release;
    std::string out = "kernel sync waiters:\n";
    char b[256];
    auto list = [&](const char* kind, const std::vector<KThread*>& v, uint64_t KThread::*field) {
        for (KThread* t : v) {
            const uint64_t a = t->*field;
            std::snprintf(b, sizeof b, "  %-18s %s (guest %llu) address 0x%llx value 0x%08x tag 0x%x wake %d\n", kind, t->name.c_str(),
                          static_cast<unsigned long long>(t->id), static_cast<unsigned long long>(a), a ? load32(a) : 0u, t->wait_tag, t->wake ? 1 : 0);
            out += b;
        }
    };
    list("WaitForAddress", g_addr_waiters, &KThread::wait_addr);
    list("process-wide key", g_cv_waiters, &KThread::wait_key);
    list("mutex hand-over", g_mutex_waiters, &KThread::wait_mutex);
    for (KThread* t : g_cv_waiters) {
        const uint32_t mv = t->wait_mutex ? load32(t->wait_mutex) : 0;
        std::snprintf(b, sizeof b, "  cv mutex            %s (guest %llu) mutex 0x%llx value 0x%08x tag 0x%x\n", t->name.c_str(),
                      static_cast<unsigned long long>(t->id), static_cast<unsigned long long>(t->wait_mutex), mv, t->wait_tag);
        out += b;
    }
    std::snprintf(b, sizeof b, "  WaitSynchronization waiters: %zu\n", g_sync_waiters.size());
    out += b;
    return out;
}

uint32_t arbitrate_lock(uint32_t owner, uint64_t addr, uint32_t tag) {
    if (addr & 3) return kInvalidAddress;
    const WaitTuning& wt = wait_tuning();
    if (wt.lock_spin && (wt.lock_names.empty() || self().lock_spin_ok)) {
        const int64_t end = qpc_now() + wt.lock_spin;
        for (uint32_t i = 0; load32(addr) == (owner | kHandleWaitMask); i++) {
            if ((i & 15) == 15 && qpc_now() >= end) break;
            nxrt::platform::yield_cpu();
        }
    }
    GuardMutex g;
    KThread& me = self();
    if (load32(addr) != (owner | kHandleWaitMask)) return kOk;      // the owner already released it: the caller retries in guest code
    me.wake = false;
    me.wait_mutex = addr;
    me.wait_tag = tag;
    g_mutex_waiters.push_back(&me);
    block_on_wake(me, -1, 1, g_k.m);
    return me.wait_result;
}

uint32_t arbitrate_unlock(uint64_t addr) {
    if (addr & 3) return kInvalidAddress;
    GuardMutex g;
    unlock_mutex(addr);
    return kOk;
}

uint32_t wait_process_wide_key(uint64_t mutex, uint64_t key, uint32_t tag, int64_t timeout_ns) {
    if (mutex & 3 || key & 3) return kInvalidAddress;
    wait_tuning();
    const int64_t until = timeout_ns <= 0 ? 0 : now_ns() + timeout_ns;
    {
        Guard g;
        KThread& me = self();
        // Like Horizon: the condition variable's "has waiters" word is written BEFORE the mutex is released. The SDK's Signal/Broadcast
        // read that word in guest code and skip the SVC when it is 0; the other order lost wake-ups (Wonder: a thread took the freed mutex,
        // posted, saw 0, and the waiter then slept forever).
        me.wake = false;
        me.wait_mutex = mutex;
        me.wait_key = key;
        me.wait_tag = tag;
        me.wait_result = 0;
        g_cv_waiters.push_back(&me);
        store32(key, 1);
        unlock_mutex(mutex);
        if (timeout_ns == 0 || !block_on_wake(me, timeout_ns, 2, g_k.m)) {
            // Timed out (not handed the mutex): leave the condition variable or the mutex queue a signal moved us to, return without
            // the mutex (the SDK re-locks it itself: TimedWait -> InternalCriticalSection::Enter).
            remove_from(g_cv_waiters, &me);
            remove_from(g_mutex_waiters, &me);
            bool others = std::any_of(g_cv_waiters.begin(), g_cv_waiters.end(), [&](KThread* t) { return t->wait_key == key; });
            if (!others) store32(key, 0);
            return kTimedOut;
        }
        // Woken up. Check if we own the mutex (mutex word has our tag).
        // If not, we need to acquire it via arbitrate_lock.
        // The kernel lock (g) is released here when the scope ends.
        uint32_t cur = load32(mutex);
        if ((cur & ~kHandleWaitMask) == tag) {
            // We own the mutex directly.
            remove_from(g_cv_waiters, &me);
            bool others = std::any_of(g_cv_waiters.begin(), g_cv_waiters.end(), [&](KThread* t) { return t->wait_key == key; });
            if (!others) store32(key, 0);
            return kOk;
        }
        // We don't own the mutex yet. We'll acquire it below after releasing the kernel lock.
    }
    // Kernel lock released. Now acquire the mutex via arbitrate_lock.
    // Loop until we own the mutex or timeout.
    for (;;) {
        int64_t remaining_ns = timeout_ns <= 0 ? -1 : until - now_ns();
        if (remaining_ns < 0) remaining_ns = 0;
        // The owner is the current mutex holder.
        uint32_t cur = load32(mutex);
        uint32_t owner = cur & ~kHandleWaitMask;
        uint32_t rc = arbitrate_lock(owner, mutex, tag);
        if (rc != kOk) {
            // Timeout or error - clean up (need to re-acquire kernel lock for cleanup)
            Guard g;
            KThread& me = self();
            remove_from(g_cv_waiters, &me);
            remove_from(g_mutex_waiters, &me);
            bool others = std::any_of(g_cv_waiters.begin(), g_cv_waiters.end(), [&](KThread* t) { return t->wait_key == key; });
            if (!others) store32(key, 0);
            return rc;
        }
        // arbitrate_lock returned kOk. Check if we actually own the mutex now.
        // arbitrate_lock returns kOk if the mutex was free (owner released it), but we might not have acquired it yet.
        cur = load32(mutex);
        if ((cur & ~kHandleWaitMask) == tag) {
            // We own the mutex.
            break;
        }
        // Mutex is free (0) or held by someone else. Loop and try again.
        // If mutex is free, the next arbitrate_lock call will return kOk immediately,
        // but we'll check again and try to CAS it in the test's mutex_lock.
        // Actually, we should try to acquire it ourselves here.
        if (cur == 0) {
            // Mutex is free. Try to acquire it.
            if (cas32(mutex, 0, tag)) {
                break;  // Got it!
            }
            // Lost the race, loop again.
            continue;
        }
        // Mutex is held by someone else. Loop and call arbitrate_lock again.
    }
    // We own the mutex.
    {
        Guard g;
        KThread& me = self();
        remove_from(g_cv_waiters, &me);
        bool others = std::any_of(g_cv_waiters.begin(), g_cv_waiters.end(), [&](KThread* t) { return t->wait_key == key; });
        if (!others) store32(key, 0);
    }
    return kOk;
}

uint32_t signal_process_wide_key(uint64_t key, int32_t count) {
    if (key & 3) return kInvalidAddress;
    int n = 0;
    const bool is_broadcast = count < 0;
    std::vector<KThread*> to_queue;
    {
        Guard g;
        while (count <= 0 || n < count) {
            KThread* t = pick_best(g_cv_waiters, &KThread::wait_key, key);
            if (!t) break;
            remove_from(g_cv_waiters, t);
            n++;
            // Try to give it the mutex right away.
            for (;;) {
                uint32_t v = load32(t->wait_mutex);
                if (v == 0) {
                    if (!cas32(t->wait_mutex, 0, t->wait_tag)) continue;
                    wake_thread(*t);
                    break;
                }
                if (!(v & kHandleWaitMask) && !cas32(t->wait_mutex, v, v | kHandleWaitMask)) continue;
                // For broadcast, queue ALL waiters on the mutex and mark it for broadcast wakeup.
                // For signal, queue only this one waiter.
                to_queue.push_back(t);
                if (is_broadcast) {
                    g_broadcast_mutexes.insert(t->wait_mutex);
                }
                break;
            }
        }
        bool others = std::any_of(g_cv_waiters.begin(), g_cv_waiters.end(), [&](KThread* t) { return t->wait_key == key; });
        if (!others) store32(key, 0);
        // Queue the handed-off waiters before the kernel lock is released: an unlock in the gap
        // could find no waiter, clear the mutex word, and leave the waiter queued for ever.
        if (!to_queue.empty()) {
            for (KThread* t : to_queue) {
                g_mutex_waiters.push_back(t);
            }
        }
    }
    return kOk;
}

uint32_t wait_for_address(uint64_t addr, uint32_t type, int32_t value, int64_t timeout_ns) {
    if (addr & 3) return kInvalidAddress;
    if (type > 2) return kInvalidEnum;
    wait_tuning();
    GuardAddr g;
    KThread& me = self();
    int32_t cur = static_cast<int32_t>(load32(addr));
    switch (type) {
        case 0:  // WaitIfLessThan
            if (!(cur < value)) return kInvalidState;
            break;
        case 1:  // DecrementAndWaitIfLessThan: one atomic step, as the kernel's exclusive load/store loop (guest threads change the word
                 // with their own atomics without the kernel lock; a plain load and store lost their updates)
            for (;;) {
                if (!(cur < value)) return kInvalidState;
                if (cas32(addr, static_cast<uint32_t>(cur), static_cast<uint32_t>(cur) - 1u)) break;
                cur = static_cast<int32_t>(load32(addr));
            }
            break;
        case 2:  // WaitIfEqual
            if (cur != value) return kInvalidState;
            break;
    }
    if (timeout_ns == 0) return kTimedOut;
    me.wake = false;
    me.wait_addr = addr;
    me.wait_result = 0;
    g_addr_waiters.push_back(&me);
    if (!block_on_wake(me, timeout_ns, 0, g_k.m)) {
        remove_from(g_addr_waiters, &me);
        return kTimedOut;
    }
    return me.wait_result;
}

uint32_t signal_to_address(uint64_t addr, uint32_t type, int32_t value, int32_t count) {
    if (addr & 3) return kInvalidAddress;
    if (type > 2) return kInvalidEnum;
    GuardAddr g;
    int waiters = static_cast<int>(std::count_if(g_addr_waiters.begin(), g_addr_waiters.end(), [&](KThread* t) { return t->wait_addr == addr; }));
    switch (type) {
        case 0:  // Signal
            break;
        case 1:  // SignalAndIncrementIfEqual
            if (!cas32(addr, static_cast<uint32_t>(value), static_cast<uint32_t>(value + 1))) return kInvalidState;
            break;
        case 2: {  // SignalAndModifyByWaitingCountIfEqual
            int32_t updated;
            if (count <= 0)
                updated = waiters ? value - 2 : value + 1;
            else if (waiters)
                updated = waiters <= count ? value - 1 : value;
            else
                updated = value + 1;
            if (!cas32(addr, static_cast<uint32_t>(value), static_cast<uint32_t>(updated))) return kInvalidState;
            break;
        }
    }
    int n = 0;
    while (count <= 0 || n < count) {
        KThread* t = pick_best(g_addr_waiters, &KThread::wait_addr, addr);
        if (!t) break;
        remove_from(g_addr_waiters, t);
        wake_thread(*t);
        n++;
    }
    return kOk;
}

uint32_t wait_synchronization(const uint32_t* handles, int32_t count, int64_t timeout_ns, int32_t* index_out) {
    if (count < 0 || count > 0x40) return kOutOfRange;
    std::vector<std::shared_ptr<KObject>> objs;
    for (int i = 0; i < count; i++) {
        auto o = handle_get(handles[i]);
        if (!o) {
            if (index_out) *index_out = -1;
            return kInvalidHandle;
        }
        objs.push_back(std::move(o));
    }
    auto ready = [&]() -> int {
        for (int i = 0; i < count; i++)
            if (objs[i]->signaled()) return i;
        return -1;
    };
    const int64_t until = timeout_ns <= 0 ? 0 : now_ns() + timeout_ns;
    KThread& me = self();
    {
        GuardSync g;
        g_sync_waiters.push_back(&me);
    }
    struct Leave {
        KThread* t;
        ~Leave() {
            GuardSync g;
            remove_from(g_sync_waiters, t);
        }
    } leave{&me};
    for (;;) {
        int i;
        {
            GuardSync g;
            if (me.cancel_sync) {
                me.cancel_sync = false;
                return kCancelled;
            }
            i = ready();
        }
        if (i >= 0) {
            if (index_out) *index_out = i;
            objs[i]->consume();
            return kOk;
        }
        if (timeout_ns == 0) return kTimedOut;
        uint32_t ms = UINT32_MAX;
        if (timeout_ns > 0) {
            int64_t rem = until - now_ns();
            if (rem <= 0) return kTimedOut;
            int64_t m = (rem + 999999) / 1000000;
            ms = static_cast<uint32_t>(m > 0x7FFFFFFE ? 0x7FFFFFFE : m);
        }
        GuardSync g;
        if (me.cancel_sync) {
            me.cancel_sync = false;
            return kCancelled;
        }
        i = ready();
        if (i >= 0) {
            if (index_out) *index_out = i;
            objs[i]->consume();
            return kOk;
        }
        sleep_locked(me, ms, g_k.m);
    }
}

namespace detail {
void (*blocked_hook_here())() {
    void (*fn)() = g_blocked_fn.load(std::memory_order_acquire);
    return fn && platform::thread_id() == g_blocked_tid.load(std::memory_order_relaxed) ? fn : nullptr;
}
}  // namespace detail

void set_blocked_hook(uint32_t host_tid, void (*fn)()) {
    g_blocked_tid.store(host_tid, std::memory_order_relaxed);
    g_blocked_fn.store(fn, std::memory_order_release);
}

uint32_t cancel_synchronization(uint32_t thread_handle) {
    auto t = handle_as<KThread>(thread_handle);
    if (!t) return kInvalidHandle;
    GuardSync g;
    t->cancel_sync = true;
    if (std::find(g_sync_waiters.begin(), g_sync_waiters.end(), t.get()) != g_sync_waiters.end()) nxrt::platform::cond_signal(thread_cv(*t));
    return kOk;
}

void notify_waitable_changed() {
    GuardSync g;
    for (KThread* t : g_sync_waiters) nxrt::platform::cond_signal(thread_cv(*t));
}

bool kernel_names_wanted() {
    const WaitTuning& wt = wait_tuning();
    return wt.stats_s > 0 || !wt.spin_names.empty() || !wt.lock_names.empty();
}

void kernel_thread_named(KThread& t, const char* game_name) {
    if (!game_name) return;
    const WaitTuning& wt = wait_tuning();
    Guard g;
    std::snprintf(t.game_name, sizeof t.game_name, "%s", game_name);
    t.spin_ok = wt.spin && name_listed(wt.spin_names, game_name);
    t.lock_spin_ok = wt.lock_spin && name_listed(wt.lock_names, game_name);
}

// ---- gauges (include/nxrt/gauge.h): wait-queue lengths ----
namespace {
// Removes its gauge when the file's statics are torn down (declared after the wait queues, so it goes first).
struct GaugeHook {
    const char* name;
    GaugeHook(const char* n, std::function<int64_t()> f) : name(n) { nxrt::gauge_register(n, std::move(f)); }
    ~GaugeHook() { nxrt::gauge_unregister(name); }
};

// Length of one wait queue under the kernel lock. The sampler never waits for the kernel lock: -1 while a thread holds it.
int64_t queue_len_mutex(const std::vector<KThread*>& v) {
    if (!nxrt::platform::mutex_trylock(g_k.m)) return -1;
    const int64_t n = static_cast<int64_t>(v.size());
    nxrt::platform::mutex_unlock(g_k.m);
    return n;
}
int64_t queue_len_cv(const std::vector<KThread*>& v) {
    if (!nxrt::platform::mutex_trylock(g_k.m)) return -1;
    const int64_t n = static_cast<int64_t>(v.size());
    nxrt::platform::mutex_unlock(g_k.m);
    return n;
}
int64_t queue_len_addr(const std::vector<KThread*>& v) {
    if (!nxrt::platform::mutex_trylock(g_k.m)) return -1;
    const int64_t n = static_cast<int64_t>(v.size());
    nxrt::platform::mutex_unlock(g_k.m);
    return n;
}
int64_t queue_len_sync(const std::vector<KThread*>& v) {
    if (!nxrt::platform::mutex_trylock(g_k.m)) return -1;
    const int64_t n = static_cast<int64_t>(v.size());
    nxrt::platform::mutex_unlock(g_k.m);
    return n;
}

GaugeHook gauge_wait_sync("kernel.waiters_sync", [] { return queue_len_sync(g_sync_waiters); });
GaugeHook gauge_wait_mutex("kernel.waiters_mutex", [] { return queue_len_mutex(g_mutex_waiters); });
GaugeHook gauge_wait_cv("kernel.waiters_cv", [] { return queue_len_cv(g_cv_waiters); });
GaugeHook gauge_wait_addr("kernel.waiters_addr", [] { return queue_len_addr(g_addr_waiters); });
}  // namespace

}  // namespace nxrt::kernel
