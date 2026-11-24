// SPDX-License-Identifier: GPL-3.0-or-later
// Runtime lifecycle: shutdown must tear down workers started before init succeeded. mem::init can fail after runtime::init
// has already started the watchdog, memory guard, starvation guard and samplers; those must not outlive the failed attempt.
#include "test_framework.h"

#include <windows.h>
#include <tlhelp32.h>

#include <atomic>
#include <chrono>
#include <cstdio>
#include <fstream>
#include <functional>
#include <memory>
#include <string>
#include <thread>
#include <vector>

#include "nxrt/audio_sink.h"
#include "nxrt/config.h"
#include "nxrt/diag.h"
#include "nxrt/ftrace.h"
#include "nxrt/kernel.h"
#include "nxrt/memguard.h"
#include "nxrt/memory.h"
#include "nxrt/runtime.h"

NXTEST_GROUP(runtime_lifecycle)

namespace {

constexpr const char* kIni = "nxrt_runtime_lifecycle.ini";
constexpr const char* kSamples = "nxrt_runtime_lifecycle.folded";
constexpr const char* kTrace = "nxrt_runtime_lifecycle.trace";

void write_ini() {
    std::ofstream f(kIni, std::ios::binary | std::ios::trunc);
    f << "[diag]\n"
         "watchdog_seconds = 30\n"
         "hitch_log = 0\n"
         "[prof]\n"
         "sample_hz = 20\n"
         "sample_out = "
      << kSamples
      << "\n"
         "trace = "
      << kTrace
      << "\n"
         "trace_events = 4096\n"
         "[sched]\n"
         "starvation_guard = 1\n"
         "[memory]\n"
         "limit_gb = 8\n";
}

// Host threads that belong to this process (the test framework and every worker the runtime starts).
DWORD count_host_threads() {
    HANDLE snap = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
    if (snap == INVALID_HANDLE_VALUE) return 0;
    THREADENTRY32 te{};
    te.dwSize = sizeof te;
    DWORD n = 0;
    const DWORD pid = GetCurrentProcessId();
    if (Thread32First(snap, &te)) {
        do {
            if (te.th32OwnerProcessID == pid) n++;
        } while (Thread32Next(snap, &te));
    }
    CloseHandle(snap);
    return n;
}

bool wait_for(const std::function<bool()>& pred, int ms) {
    const auto end = std::chrono::steady_clock::now() + std::chrono::milliseconds(ms);
    while (!pred()) {
        if (std::chrono::steady_clock::now() >= end) return false;
        std::this_thread::sleep_for(std::chrono::milliseconds(10));
    }
    return true;
}

}  // namespace

NXTEST(reinitializes_after_shutdown) {
    std::remove(kIni);
    std::remove(kSamples);
    std::remove(kTrace);
    nxrt::config_clear();
    write_ini();

    nxrt::InitOptions io;
    io.ini_path = kIni;
    io.crash_handler = false;
    io.load_symbols = false;
    io.services = false;
    io.memory_init = false;  // this case is about worker lifecycle, not the fixed address layout
    const auto running = [] {
        return nxrt::watchdog_running() && nxrt::memguard_running() && nxrt::kernel::starvation_guard_running() &&
               nxrt::prof_sampler_running() && nxrt::ft::running();
    };
    const auto stopped = [] {
        return !nxrt::watchdog_running() && !nxrt::memguard_running() && !nxrt::kernel::starvation_guard_running() &&
               !nxrt::prof_sampler_running() && !nxrt::ft::running();
    };

    CHECK(nxrt::init(io));
    CHECK(nxrt::initialised());
    CHECK(running());
    nxrt::shutdown();
    CHECK(stopped());
    nxrt::shutdown();  // repeated shutdown is a no-op and must not touch the stopped thread handles
    CHECK(stopped());

    const DWORD baseline = count_host_threads();
    CHECK(nxrt::init(io));
    CHECK(nxrt::initialised());
    CHECK(running());
    nxrt::audio::set_sink(std::make_shared<nxrt::audio::NullSink>());
    nxrt::audio::start_periodic_stats();
    CHECK(nxrt::audio::periodic_stats_running());
    nxrt::shutdown();
    CHECK(stopped());
    CHECK(!nxrt::audio::periodic_stats_running());
    CHECK_EQ(count_host_threads(), baseline);

    // Repeated init/shutdown cycles must not leave a thread or a stale work handle behind.
    for (int cycle = 0; cycle < 2; cycle++) {
        CHECK(nxrt::init(io));
        CHECK(running());
        nxrt::shutdown();
        CHECK(stopped());
        CHECK_EQ(count_host_threads(), baseline);
    }
    nxrt::audio::set_sink(nullptr);

    nxrt::mem::shutdown();
    nxrt::config_clear();
    std::remove(kIni);
    std::remove(kSamples);
    std::remove(kTrace);
}

NXTEST(init_failure_tears_down_started_workers) {
    std::remove(kIni);
    std::remove(kSamples);
    std::remove(kTrace);
    nxrt::mem::shutdown();
    nxrt::config_clear();

    // Occupy the fixed stack reservation so runtime::init's mem::init fails. init does not return at that point: it still
    // starts the worker threads below, then reports failure with g_up false.
    REQUIRE(nxrt::mem::reserve_fixed(nxrt::mem::kStackBase, nxrt::mem::kStackSize, "runtime-lifecycle-test"));
    write_ini();

    nxrt::InitOptions io;
    io.ini_path = kIni;
    io.crash_handler = false;
    io.load_symbols = false;
    io.services = false;
    CHECK(!nxrt::init(io));
    CHECK(!nxrt::initialised());

    // All five started even though the memory layout failed.
    CHECK(nxrt::watchdog_running());
    CHECK(nxrt::memguard_running());
    CHECK(nxrt::kernel::starvation_guard_running());
    CHECK(nxrt::prof_sampler_running());
    CHECK(nxrt::ft::running());

    // shutdown() must not return early on !g_up: every worker above must be stopped.
    nxrt::shutdown();
    CHECK(!nxrt::initialised());
    CHECK(!nxrt::watchdog_running());
    CHECK(!nxrt::memguard_running());
    CHECK(!nxrt::kernel::starvation_guard_running());
    CHECK(!nxrt::prof_sampler_running());
    CHECK(!nxrt::ft::running());

    nxrt::mem::release_fixed(nxrt::mem::kStackBase);
    nxrt::mem::shutdown();
    nxrt::config_clear();
    std::remove(kIni);
    std::remove(kSamples);
    std::remove(kTrace);
}

NXTEST(service_workers_are_reset_on_shutdown) {
    std::remove(kIni);
    std::remove(kSamples);
    std::remove(kTrace);
    nxrt::config_clear();
    write_ini();

    nxrt::InitOptions io;
    io.ini_path = kIni;
    io.crash_handler = false;
    io.load_symbols = false;
    io.services = true;
    io.memory_init = false;   // services still register; the fixed layout is unnecessary for this lifecycle check
    io.watchdog = false;
    io.memguard = false;

    CHECK(nxrt::init(io));
    CHECK(nxrt::initialised());
    nxrt::audio::set_sink(std::make_shared<nxrt::audio::NullSink>());
    nxrt::audio::start_periodic_stats();
    CHECK(nxrt::audio::periodic_stats_running());
    nxrt::shutdown();
    CHECK(!nxrt::audio::periodic_stats_running());
    CHECK(!nxrt::initialised());

    // Media Foundation or the audio backend may keep a process-wide helper alive after the first use. From here on, a second
    // service init/shutdown must return to the same host thread count: the service resets themselves must be complete.
    const DWORD baseline = count_host_threads();
    CHECK(nxrt::init(io));
    nxrt::shutdown();
    CHECK(wait_for([&] { return count_host_threads() == baseline; }, 1000));

    nxrt::audio::set_sink(nullptr);
    nxrt::mem::shutdown();
    nxrt::config_clear();
    std::remove(kIni);
    std::remove(kSamples);
    std::remove(kTrace);
}

NXTEST(ftrace_stop_waits_for_active_events_and_restarts) {
    std::remove(kIni);
    std::remove(kSamples);
    std::remove(kTrace);
    nxrt::mem::shutdown();
    nxrt::config_clear();
    write_ini();

    nxrt::InitOptions io;
    io.ini_path = kIni;
    io.crash_handler = false;
    io.load_symbols = false;
    io.services = false;
    io.memory_init = false;
    io.watchdog = false;
    io.memguard = false;

    const auto run_writers = [](std::atomic<bool>& stop) {
        const uint16_t zone = nxrt::ft::zone_id("lifecycle-bench");
        std::vector<std::thread> writers;
        for (int i = 0; i < 4; i++) {
            writers.emplace_back([&stop, zone] {
                while (!stop.load(std::memory_order_relaxed)) {
                    const uint64_t t = nxrt::ft::now_ns();
                    nxrt::ft::event(zone, t, t + 1);
                }
            });
        }
        return writers;
    };

    CHECK(nxrt::init(io));
    CHECK(nxrt::ft::running());
    std::atomic<bool> stop{false};
    std::vector<std::thread> writers = run_writers(stop);
    std::this_thread::sleep_for(std::chrono::milliseconds(30));
    nxrt::shutdown();   // ft::stop must wait for event() calls already inside the buffer
    stop.store(true);
    for (auto& t : writers) t.join();
    CHECK(!nxrt::ft::running());

    CHECK(nxrt::init(io));
    stop.store(false);
    writers = run_writers(stop);
    std::this_thread::sleep_for(std::chrono::milliseconds(30));
    nxrt::shutdown();
    stop.store(true);
    for (auto& t : writers) t.join();
    CHECK(!nxrt::ft::running());

    nxrt::mem::shutdown();
    nxrt::config_clear();
    std::remove(kIni);
    std::remove(kSamples);
    std::remove(kTrace);
}

NXTEST(concurrent_shutdown_is_serialised) {
    std::remove(kIni);
    std::remove(kSamples);
    std::remove(kTrace);
    nxrt::mem::shutdown();
    nxrt::config_clear();
    write_ini();

    nxrt::InitOptions io;
    io.ini_path = kIni;
    io.crash_handler = false;
    io.load_symbols = false;
    io.services = false;
    io.memory_init = false;

    const DWORD baseline = count_host_threads();
    CHECK(nxrt::init(io));
    CHECK(nxrt::watchdog_running());
    std::vector<std::thread> callers;
    for (int i = 0; i < 6; i++) callers.emplace_back([] { nxrt::shutdown(); });
    for (auto& t : callers) t.join();
    CHECK(!nxrt::initialised());
    CHECK(!nxrt::watchdog_running());
    CHECK(!nxrt::memguard_running());
    CHECK(!nxrt::kernel::starvation_guard_running());
    CHECK(!nxrt::prof_sampler_running());
    CHECK(!nxrt::ft::running());
    CHECK_EQ(count_host_threads(), baseline);

    nxrt::mem::shutdown();
    nxrt::config_clear();
    std::remove(kIni);
    std::remove(kSamples);
    std::remove(kTrace);
}

NXTEST(watchdog_callback_can_shutdown_then_runtime_restarts) {
    std::remove(kIni);
    std::remove(kSamples);
    std::remove(kTrace);
    nxrt::mem::shutdown();
    nxrt::config_clear();
    write_ini();

    nxrt::InitOptions io;
    io.ini_path = kIni;
    io.crash_handler = false;
    io.load_symbols = false;
    io.services = false;
    io.memory_init = false;
    io.watchdog = false;   // the callback test installs its own short-threshold watchdog

    const DWORD baseline = count_host_threads();
    CHECK(nxrt::init(io));
    static std::atomic<bool> fired{false};
    fired.store(false);
    nxrt::WatchdogOptions wo;
    wo.seconds = 0.05;
    wo.poll_ms = 1;
    wo.report_path = "";
    wo.on_fire = [](const char*, void*) {
        nxrt::shutdown();   // the callback runs on the watchdog thread: this stops and detaches its own generation
        fired.store(true);
    };
    nxrt::watchdog_start(wo);
    CHECK(wait_for([] { return fired.load(); }, 3000));
    CHECK(!nxrt::initialised());
    CHECK(!nxrt::watchdog_running());

    // The detached generation must have left run_loop before the next start reuses its Control fields.
    wo.on_fire = nullptr;   // the restart only checks lifecycle; do not run the callback again
    nxrt::watchdog_start(wo);
    CHECK(nxrt::watchdog_running());
    nxrt::watchdog_stop();
    CHECK(!nxrt::watchdog_running());

    // And the whole runtime must come up again after the callback-driven teardown.
    CHECK(nxrt::init(io));
    CHECK(nxrt::initialised());
    nxrt::shutdown();
    CHECK(wait_for([&] { return count_host_threads() == baseline; }, 1000));

    nxrt::mem::shutdown();
    nxrt::config_clear();
    std::remove(kIni);
    std::remove(kSamples);
    std::remove(kTrace);
}
