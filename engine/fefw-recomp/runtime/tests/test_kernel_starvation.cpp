// SPDX-License-Identifier: GPL-3.0-or-later
// The starvation guard (src/kernel/starvation_guard.cpp, debug note 422): a guest thread in the lowest priority band that the host leaves
// ready on its pinned processor, because ordinary programs keep that processor busy, gets a time slice at top priority now and then.
// Guest threads never run CHECK: results go through atomics and the test thread checks after the join.
#include <windows.h>

#include <atomic>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <thread>
#include <vector>

#include "nxrt/abi.h"
#include "nxrt/config.h"
#include "nxrt/dispatch.h"
#include "nxrt/kernel.h"
#include "nxrt/memory.h"
#include "nxrt/svc.h"
#include "test_framework.h"

NXTEST_GROUP(kernel_starvation)

namespace k = nxrt::kernel;

namespace {

std::atomic<bool> g_stop{false};
std::atomic<uint64_t> g_work{0};

void spinning_guest(::Ctx*) {
    while (!g_stop.load(std::memory_order_relaxed)) g_work.fetch_add(1, std::memory_order_relaxed);
}

uint64_t cpu_ms(HANDLE h) {
    FILETIME c, e, kt, ut;
    if (!GetThreadTimes(h, &c, &e, &kt, &ut)) return 0;
    auto to_ms = [](const FILETIME& f) { return ((static_cast<uint64_t>(f.dwHighDateTime) << 32) | f.dwLowDateTime) / 10000; };
    return to_ms(kt) + to_ms(ut);
}

void begin() {
    k::kernel_reset();
    nxrt::fn_clear();
    nxrt::svc_init();
    nxrt::mem::shutdown();
    nxrt::mem::init();
    nxrt::config_clear();
    g_stop = false;
    g_work = 0;
}

}  // namespace

NXTEST(boosts_a_lowest_band_thread_that_other_programs_starve) {
    begin();
    nxrt::config_set("sched", "affinity", "strict");   // the pinned placement of [sched] affinity = strict
    k::sched_reset_for_tests();
    const k::SchedInfo si = k::sched_info();
    if (!si.active) {
        nxtest::skip("the console core model is off on this machine");
        return;
    }
    const DWORD_PTR mask = static_cast<DWORD_PTR>(si.host_masks[0]);
    k::ThreadParams p;
    p.entry = 0x7100010000ull;
    nxrt::fn_register(p.entry, spinning_guest);
    p.stack_top = 0x7C00100000ull;
    p.prio = 59;                                   // nn::os 31: the lowest band
    p.core = 0;
    p.name = "starved";
    const uint32_t h = k::thread_create(p);
    REQUIRE(h != 0);
    // Six ordinary-priority spinners on the thread's processor, as a build or another program would be.
    std::vector<std::thread> hogs;
    std::atomic<bool> hogs_stop{false};
    for (int i = 0; i < 6; i++)
        hogs.emplace_back([&] {
            SetThreadAffinityMask(GetCurrentThread(), mask);
            volatile uint64_t x = 0;
            while (!hogs_stop.load(std::memory_order_relaxed)) x += 1;
        });
    Sleep(200);
    REQUIRE(k::thread_start(h) == k::kOk);
    auto t = k::handle_as<k::KThread>(h);
    HANDLE host = static_cast<HANDLE>(t->host_handle);
    Sleep(3000);
    const uint64_t without = cpu_ms(host);               // no guard yet: the hogs outrank the lowest band
    const uint64_t boosts0 = k::starvation_boosts();
    k::starvation_guard_start();
    Sleep(4000);
    const uint64_t with = cpu_ms(host) - without;
    const uint64_t boosts = k::starvation_boosts() - boosts0;
    g_stop = true;
    hogs_stop = true;
    for (auto& th : hogs) th.join();
    k::starvation_guard_stop();
    k::kernel_reset();
    std::printf("    the starved thread used %llu ms of CPU in 3 s without the guard and %llu ms in 4 s with it (%llu boosts)\n", static_cast<unsigned long long>(without),
                static_cast<unsigned long long>(with), static_cast<unsigned long long>(boosts));
    CHECK_MSG(without < 500, "the hogs did not starve the thread: the test setup does not show the problem");
    CHECK_MSG(boosts > 0, "the guard never boosted the starved thread");
    CHECK_MSG(with >= 800, "the starved thread got under 800 ms of CPU in 4 s with the guard: it does not keep the thread moving");
}
