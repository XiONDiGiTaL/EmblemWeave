// SPDX-License-Identifier: GPL-3.0-or-later
// nn::os::LockBusyMutex as host code (src/kernel/busy_mutex.cpp, debug note 240): it takes the lock the way the SDK does (lock word at +0x10,
// disable count in TLS+0x100) and gives the processor away while it waits, so a waiter that outranks the holder on the same host
// processor cannot starve it. Guest threads never run CHECK: results go through atomics and the test thread checks after the join.
#include <windows.h>

#include <atomic>
#include <chrono>
#include <cstdint>
#include <cstring>
#include <thread>

#include "nxrt/abi.h"
#include "nxrt/dispatch.h"
#include "nxrt/kernel.h"
#include "test_framework.h"
NXTEST_GROUP(kernel_busy_mutex)

namespace {

constexpr const char* kLock = "_ZN2nn2os13LockBusyMutexEPNS0_13BusyMutexTypeE";

struct Fake {                                   // a BusyMutexType (lock word at +0x10) and a TLS block (disable count at +0x100)
    alignas(64) uint8_t mutex[64] = {};
    alignas(64) uint8_t tls[0x200] = {};
    std::atomic<uint32_t>* word() { return reinterpret_cast<std::atomic<uint32_t>*>(mutex + 0x10); }
    uint16_t* disable() { return reinterpret_cast<uint16_t*>(tls + 0x100); }
};

void call_lock(Fake& f) {
    ::Ctx c;
    std::memset(&c, 0, sizeof c);
    c.x[0] = reinterpret_cast<uint64_t>(f.mutex);
    c.tpidrro = reinterpret_cast<uint64_t>(f.tls);
    nxrt::HleFn fn = nxrt::hle_lookup(kLock);
    fn(&c);
}

}  // namespace

NXTEST(uncontended_lock_sets_word_and_disable_count) {
    nxrt::kernel::install_busy_mutex_hle();
    REQUIRE(nxrt::hle_lookup(kLock) != nullptr);
    Fake f;
    *f.disable() = 2;                                         // already inside two busy sections
    call_lock(f);
    CHECK_EQ(f.word()->load(), (uint32_t)1);
    CHECK_EQ((int)*f.disable(), 3);
}

NXTEST(waiter_that_outranks_the_holder_does_not_starve_it) {
    nxrt::kernel::install_busy_mutex_hle();
    Fake f;
    f.word()->store(1);                                       // held by the other thread
    std::atomic<bool> holder_ready{false}, acquired{false}, holder_done{false};
    const DWORD_PTR cpu = DWORD_PTR(1) << 1;                  // both threads on host processor 1
    std::thread holder([&] {
        SetThreadAffinityMask(GetCurrentThread(), cpu);
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
        holder_ready = true;
        volatile uint64_t sink = 0;
        const auto until = std::chrono::steady_clock::now() + std::chrono::milliseconds(300);
        while (std::chrono::steady_clock::now() < until) sink += 1;     // 300 ms of work while holding the lock (it needs the processor)
        f.word()->store(0, std::memory_order_release);
        holder_done = true;
    });
    while (!holder_ready) Sleep(1);
    std::thread waiter([&] {
        SetThreadAffinityMask(GetCurrentThread(), cpu);
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
        SetThreadPriorityBoost(GetCurrentThread(), TRUE);
        call_lock(f);
        acquired = true;
    });
    const auto t0 = std::chrono::steady_clock::now();
    while (!acquired && std::chrono::steady_clock::now() - t0 < std::chrono::seconds(10)) Sleep(5);
    const bool got = acquired.load();
    if (!got) f.word()->store(0);                             // let the threads end whatever happens
    holder.join();
    waiter.join();
    CHECK_MSG(got, "the waiter did not get the lock within 10 s: the holder was starved");
    CHECK(holder_done.load());
    CHECK_EQ(f.word()->load(), (uint32_t)1);
    CHECK_EQ((int)*f.disable(), 1);
}

// ---- WFE / YIELD of the lifted code (nx_spin_hint in recomp ctx.h; src/kernel/busy_mutex.cpp, debug note 420) ---------------------------

NXTEST(spin_hint_without_a_hook_is_only_a_pause) {
    nx_spin_hook = nullptr;
    nx_spin_hint(0);
    nx_spin_hint(1);                                          // no hook: nothing to call, nothing to crash
}

NXTEST(installed_hook_receives_both_kinds) {
    nxrt::kernel::install_spin_hint();
    REQUIRE(nx_spin_hook != nullptr);
    static std::atomic<int> seen[2];
    seen[0] = seen[1] = 0;
    void (*saved)(int) = nx_spin_hook;
    nx_spin_hook = [](int kind) { seen[kind & 1]++; };
    nx_spin_hint(0);
    nx_spin_hint(1);
    nx_spin_hint(1);
    nx_spin_hook = saved;
    CHECK_EQ(seen[0].load(), 1);
    CHECK_EQ(seen[1].load(), 2);
}

NXTEST(wfe_spin_loop_does_not_starve_the_thread_it_waits_for) {
    nxrt::kernel::install_spin_hint();
    // A guest spin loop (plain load, WFE) of a thread that outranks the one that will release it, both on one host processor.
    alignas(64) static std::atomic<uint32_t> flag;
    flag.store(0);
    std::atomic<bool> done{false}, spinner_started{false};
    const DWORD_PTR cpu = DWORD_PTR(1) << 1;
    std::thread releaser([&] {
        SetThreadAffinityMask(GetCurrentThread(), cpu);
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST);
        while (!spinner_started) Sleep(1);
        volatile uint64_t sink = 0;
        const auto until = std::chrono::steady_clock::now() + std::chrono::milliseconds(300);
        while (std::chrono::steady_clock::now() < until) sink += 1;     // 300 ms of work before it can release the spinner
        flag.store(1, std::memory_order_release);
    });
    std::thread spinner([&] {
        SetThreadAffinityMask(GetCurrentThread(), cpu);
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_HIGHEST);
        SetThreadPriorityBoost(GetCurrentThread(), TRUE);
        spinner_started = true;
        while (flag.load(std::memory_order_relaxed) == 0) nx_spin_hint(1);
        done = true;
    });
    const auto t0 = std::chrono::steady_clock::now();
    while (!done && std::chrono::steady_clock::now() - t0 < std::chrono::seconds(10)) Sleep(5);
    const bool got = done.load();
    if (!got) flag.store(1);
    releaser.join();
    spinner.join();
    CHECK_MSG(got, "the spinner was not released within 10 s: the WFE hint did not give the processor away");
}
