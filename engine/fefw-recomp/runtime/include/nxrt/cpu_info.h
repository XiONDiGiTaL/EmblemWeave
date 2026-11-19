// SPDX-License-Identifier: GPL-3.0-or-later
// The host processor for the F1 menu's Debug page and the log: what it is (read once) and how busy the game keeps it (sampled).
// src/diag/cpu_info.cpp. The GPU's counterpart is the device's own (gfx::Device::gpu()).
#pragma once

#include <cstdint>
#include <string>
#include <vector>

namespace nxrt::cpu {

struct Static {
    std::string brand;              // "13th Gen Intel(R) Core(TM) i7-13700K" (CPUID 0x80000002..4, trimmed)
    std::string vendor;             // "GenuineIntel", "AuthenticAMD"
    int logical = 0;                // hardware threads
    int cores = 0;                  // physical cores
    int perf_cores = 0, eff_cores = 0;     // hybrid processors (Intel 12th gen and newer): cores by efficiency class; 0 / 0 when not hybrid
    int perf_threads = 0, eff_threads = 0;
    int l2_kb = 0;                  // all L2 caches added up
    int l3_kb = 0;                  // all L3 caches added up
    int base_mhz = 0;               // the processor's rated clock (Windows' ~MHz), 0 if unknown
    bool sse42 = false, popcnt = false, cx16 = false, avx = false, avx2 = false, fma = false, bmi2 = false, avx512f = false;
    uint64_t ram_total = 0;         // bytes of physical memory
};
const Static& info();               // read on the first call
std::string features_text(const Static& s);   // "SSE4.2, AVX, AVX2, FMA3, BMI2" (AVX-512 when present)
std::string summary_line();         // one line for the log: brand, cores, threads, caches, clock, features

// Which build of the lifted game code runs ("FMA3", "compatible (no FMA3)"): the game's start-up code says (a string that lives for the
// whole run); null when it did not.
void set_game_build(const char* text);
const char* game_build();

// Load since the previous sample (the first call starts the window and reports zeros). Call it a few times a second at most.
struct Load {
    bool valid = false;
    double process_pct = 0;         // of the whole processor (all hardware threads)
    double process_threads = 0;     // the same as hardware threads kept busy (1.0 = one thread's worth)
    double system_pct = 0;          // every process
    uint64_t working_set = 0;       // this process's physical memory
    uint64_t committed = 0;         // this process's private committed memory
    uint64_t ram_avail = 0;         // physical memory free for use
    uint32_t handles = 0;
};
Load sample();

// The busiest registered threads (the diag registry: game threads, the render thread, the runtime's workers) since the previous call:
// percent of one hardware thread, the busiest first, at most `max`.
struct ThreadLoad {
    std::string name;
    uint32_t tid = 0;
    double pct = 0;
    int ideal = -1;                 // its ideal processor (the hardware thread Windows prefers for it; -1 if unknown)
};
std::vector<ThreadLoad> thread_loads(size_t max);

}  // namespace nxrt::cpu
