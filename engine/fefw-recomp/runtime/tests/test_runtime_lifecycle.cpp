// SPDX-License-Identifier: GPL-3.0-or-later
// Runtime lifecycle: shutdown must tear down workers started before init succeeded. mem::init can fail after runtime::init
// has already started the watchdog, memory guard, starvation guard and samplers; those must not outlive the failed attempt.
#include "test_framework.h"

#include <windows.h>

#include <cstdio>
#include <fstream>

#include "nxrt/config.h"
#include "nxrt/diag.h"
#include "nxrt/kernel.h"
#include "nxrt/memguard.h"
#include "nxrt/memory.h"
#include "nxrt/runtime.h"

NXTEST_GROUP(runtime_lifecycle)

namespace {

constexpr const char* kIni = "nxrt_runtime_lifecycle.ini";
constexpr const char* kSamples = "nxrt_runtime_lifecycle.folded";

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
         "[sched]\n"
         "starvation_guard = 1\n"
         "[memory]\n"
         "limit_gb = 8\n";
}

}  // namespace

NXTEST(init_failure_tears_down_started_workers) {
    std::remove(kIni);
    std::remove(kSamples);
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

    // All four started even though the memory layout failed.
    CHECK(nxrt::watchdog_running());
    CHECK(nxrt::memguard_running());
    CHECK(nxrt::kernel::starvation_guard_running());
    CHECK(nxrt::prof_sampler_running());

    // shutdown() must not return early on !g_up: every worker above must be stopped.
    nxrt::shutdown();
    CHECK(!nxrt::initialised());
    CHECK(!nxrt::watchdog_running());
    CHECK(!nxrt::memguard_running());
    CHECK(!nxrt::kernel::starvation_guard_running());
    CHECK(!nxrt::prof_sampler_running());

    nxrt::mem::release_fixed(nxrt::mem::kStackBase);
    nxrt::mem::shutdown();
    nxrt::config_clear();
    std::remove(kIni);
    std::remove(kSamples);
}
