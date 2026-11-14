// SPDX-License-Identifier: GPL-3.0-or-later
// Tests for nxrt::sync (src/os/sync.cpp). Rule used throughout: CHECK is per host thread, so worker threads never CHECK.
// They store results in variables and the test thread checks them after join_watched(), which also enforces the watchdog.
#include "test_framework.h"

#include "nxrt/sync.h"

#include <windows.h>

#include <atomic>
#include <chrono>
#include <cstdint>
#include <cstdlib>
#include <deque>
#include <random>
#include <thread>
#include <vector>

NXTEST_GROUP(sync)

namespace ns = nxrt::sync;

namespace {

using Threads = std::vector<std::thread>;
using Clock = std::chrono::steady_clock;

constexpr int64_t kMs = 1000000;
constexpr DWORD kWatchdogMs = 20000;

// Guest addresses are never dereferenced; each test takes its own slots.
uint64_t A(uint64_t slot) { return 0x71000000A0ull + slot * 0x100; }

double ms_since(Clock::time_point t0) {
    return std::chrono::duration<double, std::milli>(Clock::now() - t0).count();
}

void sleep_ms(int ms) { std::this_thread::sleep_for(std::chrono::milliseconds(ms)); }

// Joins every thread, but no longer than 20 s overall. On timeout the whole test process is ended with exit code 3, so a
// deadlock fails the run instead of hanging CTest.
void join_watched(const char* test, Threads& ts) {
    std::vector<HANDLE> handles;
    for (auto& t : ts) handles.push_back(static_cast<HANDLE>(t.native_handle()));
    if (!handles.empty() &&
        WaitForMultipleObjects(static_cast<DWORD>(handles.size()), handles.data(), TRUE, kWatchdogMs) == WAIT_TIMEOUT) {
        std::printf("    WATCHDOG: %s did not finish within 20 s; aborting the test run\n", test);
        std::fflush(stdout);
        std::_Exit(3);
    }
    for (auto& t : ts) t.join();
}

// Waits (on the test thread, bounded to 5 s) until pred() holds. Returns the final pred() value.
template <class F>
bool wait_for(F pred) {
    const auto limit = Clock::now() + std::chrono::seconds(5);
    while (!pred()) {
        if (Clock::now() > limit) return false;
        sleep_ms(1);
    }
    return true;
}

// Runs try_lock from another thread: true if it got the mutex (it is released again at once).
bool probe_try_lock(const char* test, uint64_t m) {
    bool got = false;
    Threads ts;
    ts.emplace_back([&] {
        got = ns::mutex_try_lock(m);
        if (got) ns::mutex_unlock(m);
    });
    join_watched(test, ts);
    return got;
}

}  // namespace

// ---- mutex --------------------------------------------------------------------------------------------------------------

NXTEST(mutex_mutual_exclusion) {
    const uint64_t m = A(1);
    ns::mutex_init(m, false);
    long counter = 0;  // plain, not atomic: only the guest mutex protects it
    Threads ts;
    for (int t = 0; t < 8; t++)
        ts.emplace_back([&] {
            for (int i = 0; i < 50000; i++) {
                ns::mutex_lock(m);
                counter++;
                ns::mutex_unlock(m);
            }
        });
    join_watched("mutex_mutual_exclusion", ts);
    CHECK_EQ(counter, 400000L);
    CHECK(!ns::mutex_is_locked(m));
}

NXTEST(mutex_try_lock) {
    const uint64_t m = A(2);
    ns::mutex_init(m, false);
    CHECK(ns::mutex_try_lock(m));               // free: succeeds
    CHECK(!ns::mutex_try_lock(m));              // owned by us, non-recursive: fails
    CHECK(!probe_try_lock("mutex_try_lock", m));  // held by another thread's view: fails
    CHECK(ns::mutex_is_locked_by_current(m));
    ns::mutex_unlock(m);
    CHECK(probe_try_lock("mutex_try_lock", m));  // free again: another thread succeeds
    CHECK(!ns::mutex_is_locked(m));
}

NXTEST(mutex_recursive_nesting) {
    const uint64_t m = A(3);
    ns::mutex_init(m, true);
    ns::mutex_lock(m);
    ns::mutex_lock(m);
    ns::mutex_lock(m);
    CHECK(ns::mutex_is_locked_by_current(m));
    CHECK(!probe_try_lock("mutex_recursive_nesting", m));  // depth 3
    ns::mutex_unlock(m);
    CHECK(!probe_try_lock("mutex_recursive_nesting", m));  // depth 2
    ns::mutex_unlock(m);
    CHECK(!probe_try_lock("mutex_recursive_nesting", m));  // depth 1
    ns::mutex_unlock(m);
    CHECK(!ns::mutex_is_locked(m));
    CHECK(probe_try_lock("mutex_recursive_nesting", m));  // depth 0: free
}

NXTEST(mutex_nonrecursive_relock_returns) {
    const uint64_t m = A(4);
    ns::mutex_init(m, false);
    bool held_after_relock = false;
    bool free_after_one_unlock = false;
    Threads ts;
    ts.emplace_back([&] {
        ns::mutex_lock(m);
        ns::mutex_lock(m);  // logged as an error and ignored: must return, not block
        held_after_relock = ns::mutex_is_locked_by_current(m);
        ns::mutex_unlock(m);  // one unlock is enough, because the relock took nothing
        free_after_one_unlock = !ns::mutex_is_locked(m);
    });
    join_watched("mutex_nonrecursive_relock_returns", ts);
    CHECK(held_after_relock);
    CHECK(free_after_one_unlock);
}

NXTEST(mutex_unlock_by_non_owner_harmless) {
    const uint64_t m = A(5);
    ns::mutex_init(m, false);
    ns::mutex_lock(m);
    Threads ts;
    ts.emplace_back([&] { ns::mutex_unlock(m); });  // not the owner: logged, does nothing
    join_watched("mutex_unlock_by_non_owner_harmless", ts);
    CHECK(ns::mutex_is_locked_by_current(m));
    CHECK(ns::mutex_is_locked(m));
    ns::mutex_unlock(m);
    ns::mutex_unlock(m);  // unlock of a free mutex: logged, does nothing
    CHECK(!ns::mutex_is_locked(m));
}

// ---- condition variable -------------------------------------------------------------------------------------------------

NXTEST(cond_producer_consumer) {
    const uint64_t m = A(6);
    const uint64_t c = A(7);
    ns::mutex_init(m, false);
    ns::cond_init(c);
    constexpr int kItems = 20000;
    constexpr int kConsumers = 3;
    // Everything below is protected by the guest mutex m.
    std::deque<int> queue;
    std::vector<int> seen(kItems, 0);
    bool done = false;

    Threads ts;
    ts.emplace_back([&] {
        for (int i = 0; i < kItems; i++) {
            ns::mutex_lock(m);
            queue.push_back(i);
            ns::cond_signal(c);
            ns::mutex_unlock(m);
        }
        ns::mutex_lock(m);
        done = true;
        ns::cond_broadcast(c);
        ns::mutex_unlock(m);
    });
    for (int k = 0; k < kConsumers; k++)
        ts.emplace_back([&] {
            for (;;) {
                ns::mutex_lock(m);
                while (queue.empty() && !done) ns::cond_wait(c, m);
                if (queue.empty()) {
                    ns::mutex_unlock(m);
                    break;
                }
                const int v = queue.front();
                queue.pop_front();
                seen[v]++;
                ns::mutex_unlock(m);
            }
        });
    join_watched("cond_producer_consumer", ts);

    int bad = 0;
    for (int v : seen)
        if (v != 1) bad++;
    CHECK_EQ(bad, 0);  // every item consumed exactly once
    CHECK(queue.empty());
}

NXTEST(cond_broadcast_wakes_all) {
    const uint64_t m = A(8);
    const uint64_t c = A(9);
    ns::mutex_init(m, false);
    ns::cond_init(c);
    constexpr int kWaiters = 6;
    bool go = false;      // guest mutex m
    int arrived = 0;      // guest mutex m
    int woke = 0;         // guest mutex m

    Threads ts;
    for (int k = 0; k < kWaiters; k++)
        ts.emplace_back([&] {
            ns::mutex_lock(m);
            arrived++;
            while (!go) ns::cond_wait(c, m);
            woke++;
            ns::mutex_unlock(m);
        });

    // Once the test thread holds m and sees all waiters arrived, each of them has released m inside cond_wait.
    bool all_arrived = wait_for([&] {
        ns::mutex_lock(m);
        const bool ok = arrived == kWaiters;
        ns::mutex_unlock(m);
        return ok;
    });
    CHECK(all_arrived);

    ns::mutex_lock(m);
    go = true;
    ns::cond_broadcast(c);
    ns::mutex_unlock(m);
    join_watched("cond_broadcast_wakes_all", ts);
    CHECK_EQ(woke, kWaiters);
}

NXTEST(cond_timed_wait_timeout_keeps_mutex) {
    const uint64_t m = A(10);
    const uint64_t c = A(11);
    ns::mutex_init(m, false);
    ns::cond_init(c);
    bool result = true;
    bool held = false;
    double elapsed = 0;
    Threads ts;
    ts.emplace_back([&] {
        ns::mutex_lock(m);
        const auto t0 = Clock::now();
        result = ns::cond_timed_wait(c, m, 50 * kMs);
        elapsed = ms_since(t0);
        held = ns::mutex_is_locked_by_current(m);
        ns::mutex_unlock(m);
    });
    join_watched("cond_timed_wait_timeout_keeps_mutex", ts);
    CHECK(!result);
    CHECK(elapsed >= 50.0);
    CHECK(held);
    CHECK(!ns::mutex_is_locked(m));
}

NXTEST(cond_signal_without_waiters_not_remembered) {
    const uint64_t m = A(12);
    const uint64_t c = A(13);
    ns::mutex_init(m, false);
    ns::cond_init(c);
    bool first = true;
    double elapsed = 0;
    Threads ts;
    ts.emplace_back([&] {
        ns::mutex_lock(m);
        ns::cond_signal(c);  // nobody waiting: must not leave a token behind
        ns::cond_broadcast(c);
        const auto t0 = Clock::now();
        first = ns::cond_timed_wait(c, m, 50 * kMs);
        elapsed = ms_since(t0);
        ns::mutex_unlock(m);
    });
    join_watched("cond_signal_without_waiters_not_remembered", ts);
    CHECK(!first);
    CHECK(elapsed >= 50.0);
}

// ---- event --------------------------------------------------------------------------------------------------------------

NXTEST(event_auto_clear_releases_one) {
    const uint64_t e = A(14);
    ns::event_init(e, false, true);
    std::atomic<int> released{0};
    Threads ts;
    for (int k = 0; k < 4; k++)
        ts.emplace_back([&] {
            ns::event_wait(e);
            released++;
        });
    bool exact = true;
    for (int i = 1; i <= 4; i++) {
        ns::event_signal(e);
        exact = exact && wait_for([&] { return released.load() >= i; });
        sleep_ms(50);  // give any extra waiter time to wake wrongly
        exact = exact && released.load() == i;
    }
    join_watched("event_auto_clear_releases_one", ts);
    CHECK_MSG(exact, "each signal must release exactly one waiter");
    CHECK_EQ(released.load(), 4);
    CHECK(!ns::event_try_wait(e));
}

NXTEST(event_manual_clear_releases_all) {
    const uint64_t e = A(15);
    ns::event_init(e, false, false);
    std::atomic<int> released{0};
    Threads ts;
    for (int k = 0; k < 4; k++)
        ts.emplace_back([&] {
            ns::event_wait(e);
            released++;
        });
    ns::event_signal(e);
    join_watched("event_manual_clear_releases_all", ts);
    CHECK_EQ(released.load(), 4);

    // Stays signalled: try_wait succeeds repeatedly and a late waiter returns at once.
    CHECK(ns::event_try_wait(e));
    CHECK(ns::event_try_wait(e));
    bool late = false;
    Threads late_ts;
    late_ts.emplace_back([&] {
        ns::event_wait(e);
        late = true;
    });
    join_watched("event_manual_clear_releases_all", late_ts);
    CHECK(late);

    ns::event_clear(e);
    CHECK(!ns::event_try_wait(e));
}

NXTEST(event_timed_wait_paths) {
    const uint64_t e = A(16);
    ns::event_init(e, false, true);

    // Timeout path: nothing signals, so it must report false after at least 50 ms.
    bool timed_out_result = true;
    double timed_out_elapsed = 0;
    Threads t1;
    t1.emplace_back([&] {
        const auto t0 = Clock::now();
        timed_out_result = ns::event_timed_wait(e, 50 * kMs);
        timed_out_elapsed = ms_since(t0);
    });
    join_watched("event_timed_wait_paths", t1);
    CHECK(!timed_out_result);
    CHECK(timed_out_elapsed >= 50.0);

    // Success path: a 5 s wait is released by a signal after 100 ms, well before its deadline.
    bool signalled_result = false;
    double signalled_elapsed = 0;
    Threads t2;
    t2.emplace_back([&] {
        const auto t0 = Clock::now();
        signalled_result = ns::event_timed_wait(e, 5000 * kMs);
        signalled_elapsed = ms_since(t0);
    });
    sleep_ms(100);
    ns::event_signal(e);
    join_watched("event_timed_wait_paths", t2);
    CHECK(signalled_result);
    CHECK(signalled_elapsed < 4000.0);

    // Auto-clear consumed by that waiter; a zero-timeout poll on a set event succeeds once, then fails.
    ns::event_signal(e);
    CHECK(ns::event_timed_wait(e, 0));
    CHECK(!ns::event_timed_wait(e, 0));
}

// ---- semaphore ----------------------------------------------------------------------------------------------------------

NXTEST(sem_counting) {
    const uint64_t s = A(17);
    ns::sem_init(s, 0, INT32_MAX);
    CHECK_EQ(ns::sem_count(s), 0);
    ns::sem_release(s, 5);
    CHECK_EQ(ns::sem_count(s), 5);
    for (int i = 0; i < 5; i++) CHECK(ns::sem_try_acquire(s));
    CHECK(!ns::sem_try_acquire(s));  // 6th fails: count is 0
    CHECK_EQ(ns::sem_count(s), 0);

    // Blocking acquire on an empty semaphore waits for a release.
    const uint64_t s2 = A(18);
    ns::sem_init(s2, 0, INT32_MAX);
    std::atomic<bool> got{false};
    Threads ts;
    ts.emplace_back([&] {
        ns::sem_acquire(s2);
        got = true;
    });
    sleep_ms(100);
    CHECK(!got.load());
    ns::sem_release(s2, 1);
    join_watched("sem_counting", ts);
    CHECK(got.load());
}

NXTEST(sem_clamps_at_max) {
    const uint64_t s = A(19);
    ns::sem_init(s, 0, 3);
    ns::sem_release(s, 10);
    CHECK_EQ(ns::sem_count(s), 3);
    ns::sem_release(s);  // default n = 1, still clamped
    CHECK_EQ(ns::sem_count(s), 3);
    for (int i = 0; i < 3; i++) CHECK(ns::sem_try_acquire(s));
    CHECK(!ns::sem_try_acquire(s));

    ns::sem_init(s, 2, 3);  // initial count 2, release 5 -> clamped to 3
    ns::sem_release(s, 5);
    CHECK_EQ(ns::sem_count(s), 3);
}

NXTEST(sem_ping_pong) {
    const uint64_t a = A(20);
    const uint64_t b = A(21);
    ns::sem_init(a, 0, INT32_MAX);
    ns::sem_init(b, 0, INT32_MAX);
    std::atomic<int> ping{0};
    std::atomic<int> pong{0};
    Threads ts;
    ts.emplace_back([&] {
        for (int i = 0; i < 10000; i++) {
            ns::sem_release(a, 1);
            ns::sem_acquire(b);
            ping++;
        }
    });
    ts.emplace_back([&] {
        for (int i = 0; i < 10000; i++) {
            ns::sem_acquire(a);
            ns::sem_release(b, 1);
            pong++;
        }
    });
    join_watched("sem_ping_pong", ts);
    CHECK_EQ(ping.load(), 10000);
    CHECK_EQ(pong.load(), 10000);
    CHECK_EQ(ns::sem_count(a), 0);
    CHECK_EQ(ns::sem_count(b), 0);
}

// ---- housekeeping -------------------------------------------------------------------------------------------------------

NXTEST(destroy_object_count_reset_all) {
    const size_t c0 = ns::object_count();
    ns::mutex_init(A(22), false);
    ns::cond_init(A(23));
    ns::event_init(A(24), false, true);
    ns::sem_init(A(25), 3, 10);
    CHECK_EQ(ns::object_count(), c0 + 4);

    ns::destroy(A(22));
    CHECK_EQ(ns::object_count(), c0 + 3);
    ns::destroy(A(22));  // unknown address: no-op
    CHECK_EQ(ns::object_count(), c0 + 3);

    // A new object at a destroyed address starts clean: the old semaphore had count 3, the new one has count 0.
    ns::destroy(A(25));
    CHECK(!ns::sem_try_acquire(A(25)));
    CHECK_EQ(ns::object_count(), c0 + 3);  // the lookup above created a fresh one

    // Kind change at the same address replaces the object: cond at A(23) becomes a semaphore with count 2.
    ns::sem_release(A(23), 2);
    CHECK_EQ(ns::sem_count(A(23)), 2);
    CHECK_EQ(ns::object_count(), c0 + 3);

    ns::reset_all();
    CHECK_EQ(ns::object_count(), static_cast<size_t>(0));
}

// ---- stress -------------------------------------------------------------------------------------------------------------

NXTEST(stress_random_ops) {
    const uint64_t m = A(30);
    const uint64_t c = A(31);
    const uint64_t e = A(32);
    const uint64_t s = A(33);
    ns::mutex_init(m, false);
    ns::cond_init(c);
    ns::event_init(e, false, true);
    ns::sem_init(s, 0, INT32_MAX);

    long long plain = 0;  // guest mutex m protects it
    std::atomic<long long> expected{0};
    const auto end = Clock::now() + std::chrono::seconds(2);

    Threads ts;
    for (int t = 0; t < 6; t++)
        ts.emplace_back([&, t] {
            std::mt19937 rng(static_cast<unsigned>(1234 + t));
            long long pairs = 0;
            while (Clock::now() < end) {
                const int64_t rnd_ns = static_cast<int64_t>(rng() % 2000000u);  // 0..2 ms
                switch (rng() % 6) {
                    case 0:  // lock / unlock pair
                        ns::mutex_lock(m);
                        plain++;
                        ns::mutex_unlock(m);
                        pairs++;
                        break;
                    case 1:  // timed cond wait
                        ns::mutex_lock(m);
                        (void)ns::cond_timed_wait(c, m, rnd_ns);
                        ns::mutex_unlock(m);
                        break;
                    case 2:  // cond signal
                        ns::mutex_lock(m);
                        ns::cond_signal(c);
                        ns::mutex_unlock(m);
                        break;
                    case 3:  // cond broadcast
                        ns::mutex_lock(m);
                        ns::cond_broadcast(c);
                        ns::mutex_unlock(m);
                        break;
                    case 4:  // event signal then timed wait
                        ns::event_signal(e);
                        (void)ns::event_timed_wait(e, rnd_ns);
                        break;
                    default:  // semaphore release then timed acquire
                        ns::sem_release(s, 1);
                        (void)ns::sem_timed_acquire(s, rnd_ns);
                        break;
                }
            }
            expected += pairs;
        });
    join_watched("stress_random_ops", ts);
    CHECK(expected.load() > 0);
    CHECK_EQ(plain, expected.load());
    CHECK(!ns::mutex_is_locked(m));
}
