// SPDX-License-Identifier: GPL-3.0-or-later
// nn::os::LightEvent on the kernel's WaitForAddress / SignalToAddress (S20, debug note 421). The game builds its thread hand-overs on it
// (CSynchronizationEvent: the graphics thread's command wake-up, CGraphicsFrameToken, the resource loader), and the sweep's stalls were first
// suspected to be a lost wake-up here. The SDK's protocol is ported below from the decompiled nn::os::detail::WaitLightEventImpl and
// SignalLightEventImpl (sdk.nso 0x33df00 and 0x33dd60): state 0 = clear and nobody waiting, 1 = clear with waiters, 2 = signaled.
//   auto-clear   wait: 2 -> 0 by compare-and-swap; 0 -> 1 then WaitForAddress(WaitIfEqual 1); signal: 0 -> 2, or with state 1
//                SignalToAddress(SignalAndModifyBasedOnWaitingThreadCountIfEqual, value 1, count 1)
//   manual-clear wait: sleeps while the state is 1; signal: 0 -> 2, or with state 1 SignalToAddress(SignalAndIncrementIfEqual, 1, count -1)
// Every round trip must wake exactly the threads the protocol promises; a lost wake-up ends in the 20 s guard below. Worker threads never
// CHECK: results go through atomics and the test thread checks after the join.
#include <windows.h>

#include <atomic>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <thread>
#include <vector>

#include "nxrt/kernel.h"
#include "test_framework.h"

NXTEST_GROUP(kernel_lightevent)

namespace ks = nxrt::kernel;

namespace {

using Clock = std::chrono::steady_clock;

struct LightEvent {
    alignas(4) uint32_t state = 0;
    uint8_t auto_clear = 1;
};

std::atomic<int> g_protocol_errors{0};
std::atomic<int> g_kernel_wakes{0};      // waits that really slept in the kernel and were woken (the path that can lose a wake-up)

uint64_t addr_of(const void* p) { return reinterpret_cast<uint64_t>(p); }
uint32_t load_state(const LightEvent& e) { return __atomic_load_n(&e.state, __ATOMIC_SEQ_CST); }
bool cas_state(LightEvent& e, uint32_t from, uint32_t to) {
    return __atomic_compare_exchange_n(&e.state, &from, to, false, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
}

void wait_auto(LightEvent& e) {
    const uint64_t a = addr_of(&e.state);
    for (;;) {
        const uint32_t s = load_state(e);
        if (s == 2) {
            if (cas_state(e, 2, 0)) return;
            continue;
        }
        if (s == 0 && !cas_state(e, 0, 1)) continue;
        const uint32_t r = ks::wait_for_address(a, 2, 1, -1);       // WaitIfEqual 1, no timeout
        if (r == ks::kOk) { g_kernel_wakes++; return; }              // the signaler consumed the event for us
        if (r != ks::kInvalidState) g_protocol_errors++;
    }
}

void signal_auto(LightEvent& e) {
    const uint64_t a = addr_of(&e.state);
    for (;;) {
        const uint32_t s = load_state(e);
        if (s == 2) return;
        if (s == 0) {
            if (cas_state(e, 0, 2)) return;
            continue;
        }
        const uint32_t r = ks::signal_to_address(a, 2, 1, 1);
        if (r == ks::kOk) return;
        if (r != ks::kInvalidState) g_protocol_errors++;
    }
}

void wait_manual(LightEvent& e) {
    const uint64_t a = addr_of(&e.state);
    for (;;) {
        const uint32_t s = load_state(e);
        if (s == 2) return;
        if (s == 0 && !cas_state(e, 0, 1)) continue;
        const uint32_t r = ks::wait_for_address(a, 2, 1, -1);
        if (r == ks::kOk || r == ks::kInvalidState) return;          // woken, or the state had already left 1
        g_protocol_errors++;
    }
}

void signal_manual(LightEvent& e) {
    const uint64_t a = addr_of(&e.state);
    for (;;) {
        const uint32_t s = load_state(e);
        if (s == 2) return;
        if (s == 0) {
            if (cas_state(e, 0, 2)) return;
            continue;
        }
        const uint32_t r = ks::signal_to_address(a, 1, 1, -1);
        if (r == ks::kOk) return;
        if (r != ks::kInvalidState) g_protocol_errors++;
    }
}

// Waits for pred() on the test thread; a lost wake-up shows as the guard expiring.
template <class F>
bool wait_for(F pred, int ms = 20000) {
    const auto limit = Clock::now() + std::chrono::milliseconds(ms);
    while (!pred()) {
        if (Clock::now() > limit) return false;
        Sleep(0);
    }
    return true;
}

[[noreturn]] void give_up(const char* test) {
    std::printf("    WATCHDOG: %s did not finish within 20 s; aborting the test run\n", test);
    std::fflush(stdout);
    std::_Exit(3);
}

// K waiters, one signaler: every signal must release exactly one waiter, in a handshake (signal, wait until it was consumed).
void auto_round_trips(const char* name, int waiters, int rounds) {
    ks::kernel_reset();
    LightEvent e;
    e.auto_clear = 1;
    std::atomic<int> consumed{0};
    std::atomic<bool> stop{false};
    g_protocol_errors = 0;
    g_kernel_wakes = 0;
    std::vector<std::thread> ts;
    for (int i = 0; i < waiters; i++)
        ts.emplace_back([&] {
            for (;;) {
                wait_auto(e);
                if (stop.load()) return;
                consumed++;
            }
        });
    for (int r = 1; r <= rounds; r++) {
        signal_auto(e);
        if (!wait_for([&] { return consumed.load() >= r; })) give_up(name);
        if (r % 7 == 0) Sleep(0);                                       // vary the interleaving: sometimes the waiters get to sleep first
    }
    stop = true;
    for (int i = 0; i < waiters; i++) signal_auto(e);                    // release every waiter for the exit check
    std::vector<HANDLE> hs;
    for (auto& t : ts) hs.push_back(static_cast<HANDLE>(t.native_handle()));
    bool joined = true;
    for (int tries = 0; tries < 2000; tries++) {                         // the exit wake-ups coalesce (an auto-clear signal while signaled is lost by design): keep signaling
        if (WaitForMultipleObjects(static_cast<DWORD>(hs.size()), hs.data(), TRUE, 10) != WAIT_TIMEOUT) break;
        signal_auto(e);
        if (tries == 1999) joined = false;
    }
    if (!joined) give_up(name);
    for (auto& t : ts) t.join();
    CHECK_EQ(consumed.load(), rounds);
    CHECK_EQ(g_protocol_errors.load(), 0);
    CHECK_MSG(g_kernel_wakes.load() > 0, "no wait ever slept in the kernel: the test did not exercise the wake-up path");
}

}  // namespace

NXTEST(auto_clear_one_waiter_round_trips) { auto_round_trips("auto_clear_one_waiter_round_trips", 1, 20000); }
NXTEST(auto_clear_four_waiters_one_signal_wakes_one) { auto_round_trips("auto_clear_four_waiters_one_signal_wakes_one", 4, 20000); }

NXTEST(auto_clear_signal_before_wait_is_remembered) {
    ks::kernel_reset();
    LightEvent e;
    for (int i = 0; i < 1000; i++) {
        signal_auto(e);
        signal_auto(e);                       // a second signal while signaled changes nothing
        CHECK_EQ(load_state(e), 2u);
        wait_auto(e);                         // consumed without sleeping
        CHECK_EQ(load_state(e), 0u);
    }
}

NXTEST(manual_clear_signal_releases_every_waiter) {
    ks::kernel_reset();
    constexpr int kWaiters = 4, kRounds = 5000;
    LightEvent e;
    e.auto_clear = 0;
    std::atomic<int> arrived{0}, passed{0}, generation{0};
    std::atomic<bool> stop{false};
    g_protocol_errors = 0;
    std::vector<std::thread> ts;
    for (int i = 0; i < kWaiters; i++)
        ts.emplace_back([&] {
            int seen = 0;
            while (!stop.load()) {
                while (generation.load() == seen && !stop.load()) Sleep(0);   // wait for the next round to be opened
                if (stop.load()) return;
                seen = generation.load();
                arrived++;
                wait_manual(e);
                passed++;
            }
        });
    for (int r = 1; r <= kRounds; r++) {
        generation = r;                                                     // open the round: the waiters go to wait on the (cleared) event
        if (r % 3 == 0) Sleep(0);
        if (r % 2 == 0) {                                                   // signal before every waiter is in (the late ones must pass at once)
            signal_manual(e);
        } else {
            if (!wait_for([&] { return arrived.load() >= r * kWaiters; })) give_up("manual_clear_signal_releases_every_waiter");
            signal_manual(e);
        }
        if (!wait_for([&] { return passed.load() >= r * kWaiters; })) give_up("manual_clear_signal_releases_every_waiter");
        __atomic_store_n(&e.state, 0u, __ATOMIC_SEQ_CST);                  // ClearLightEvent: every waiter of this round has left
    }
    stop = true;
    for (auto& t : ts) t.join();
    CHECK_EQ(passed.load(), kRounds * kWaiters);
    CHECK_EQ(g_protocol_errors.load(), 0);
}
