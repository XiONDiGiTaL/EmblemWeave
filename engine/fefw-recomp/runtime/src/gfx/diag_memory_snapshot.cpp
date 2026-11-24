// SPDX-License-Identifier: GPL-3.0-or-later
// Memory and resource snapshot (declared in nxrt/diag.h) for long-session diagnostics (Issue #37) and the
// chapter-transition/shared-memory investigation (Issue #41).
//
// It lives in nxrt_gfx, not in the nxrt core: half of what it reports (the VMA video-memory budget, the live
// percentage, the deferred-destruction gauge) belongs to the graphics library, and the core library must not link
// against it. `diag_memory_snapshot_text()` is appended to the GPU device-lost report and can be called from any
// thread: every part takes its own lock briefly and allocates only its own text.
#include <windows.h>
#include <psapi.h>

#include <algorithm>
#include <chrono>
#include <sstream>
#include <string>
#include <vector>

#include "nxrt/diag.h"
#include "nxrt/gauge.h"
#include "nxrt/gfx.h"

namespace nxrt {
namespace {

std::string format_bytes(uint64_t bytes) {
    std::ostringstream o;
    if (bytes >= (1024ull * 1024 * 1024)) {
        o << static_cast<double>(bytes) / (1024.0 * 1024.0 * 1024.0) << " GiB";
    } else if (bytes >= (1024ull * 1024)) {
        o << static_cast<double>(bytes) / (1024.0 * 1024.0) << " MiB";
    } else if (bytes >= 1024) {
        o << static_cast<double>(bytes) / 1024.0 << " KiB";
    } else {
        o << bytes << " B";
    }
    return o.str();
}

}  // namespace

MemorySnapshot diag_memory_snapshot() {
    MemorySnapshot snap;
    std::ostringstream out;

    out << "=== MEMORY SNAPSHOT ===\n";
    out << "Timestamp: "
        << std::chrono::duration_cast<std::chrono::milliseconds>(std::chrono::steady_clock::now().time_since_epoch()).count()
        << " ms since process start\n\n";

    // 1. Process memory.
    out << "--- Process Memory ---\n";
    PROCESS_MEMORY_COUNTERS_EX pmc{};
    if (GetProcessMemoryInfo(GetCurrentProcess(), reinterpret_cast<PROCESS_MEMORY_COUNTERS*>(&pmc), sizeof(pmc))) {
        out << "  Working Set:          " << format_bytes(pmc.WorkingSetSize) << " (peak " << format_bytes(pmc.PeakWorkingSetSize) << ")\n";
        out << "  Private Bytes:        " << format_bytes(pmc.PrivateUsage) << "\n";
        out << "  Pagefile Usage:       " << format_bytes(pmc.PagefileUsage) << " (peak " << format_bytes(pmc.PeakPagefileUsage) << ")\n";
        out << "  Quota Paged Pool:     " << format_bytes(pmc.QuotaPagedPoolUsage) << " (peak " << format_bytes(pmc.QuotaPeakPagedPoolUsage) << ")\n";
        out << "  Quota Non-Paged Pool: " << format_bytes(pmc.QuotaNonPagedPoolUsage) << " (peak " << format_bytes(pmc.QuotaPeakNonPagedPoolUsage) << ")\n";
        out << "  Page Fault Count:     " << pmc.PageFaultCount << "\n";
    } else {
        out << "  (failed to query process memory)\n";
    }

    // 2. System memory.
    out << "\n--- System Memory ---\n";
    MEMORYSTATUSEX ms{};
    ms.dwLength = sizeof(ms);
    if (GlobalMemoryStatusEx(&ms)) {
        out << "  Total Physical:       " << format_bytes(ms.ullTotalPhys) << "\n";
        out << "  Available Physical:   " << format_bytes(ms.ullAvailPhys) << "\n";
        out << "  Total Pagefile:       " << format_bytes(ms.ullTotalPageFile) << "\n";
        out << "  Available Pagefile:   " << format_bytes(ms.ullAvailPageFile) << "\n";
        out << "  Total Virtual:        " << format_bytes(ms.ullTotalVirtual) << "\n";
        out << "  Available Virtual:    " << format_bytes(ms.ullAvailVirtual) << "\n";
        out << "  Memory Load:          " << ms.dwMemoryLoad << "%\n";
    } else {
        out << "  (failed to query system memory)\n";
    }

    // 3. Every gauge reading: the NVN pool, texture, pipeline and deferred-destruction counters are all here.
    out << "\n--- Gauges ---\n";
    const auto gauges = gauge_read_all();
    if (gauges.empty()) {
        out << "  (no gauges registered)\n";
    } else {
        for (const auto& [name, value] : gauges) out << "  " << name << ": " << value << "\n";
    }

    // 4. VMA video memory (sampled at most every 250 ms inside the getters).
    out << "\n--- Vulkan Video Memory (VMA) ---\n";
    const uint64_t budget = gfx::video_budget_bytes();
    const int usage_pct = gfx::video_usage_percent();
    const int live_pct = gfx::video_live_percent();
    out << "  Budget:               " << format_bytes(budget) << "\n";
    out << "  Usage:                " << (usage_pct >= 0 ? std::to_string(usage_pct) + "%" : "unknown") << "\n";
    out << "  Live Usage:           " << (live_pct >= 0 ? std::to_string(live_pct) + "%" : "unknown") << "\n";
    if (budget > 0 && usage_pct >= 0) {
        out << "  Used (est):           " << format_bytes(budget * static_cast<uint64_t>(usage_pct) / 100) << "\n";
    }

    // 5. Thread registry: what was running when the snapshot was taken.
    out << "\n--- Thread Registry ---\n";
    const auto threads = diag_threads();
    out << "  Registered Threads:   " << threads.size() << "\n";
    for (const auto& t : threads) {
        out << "    [" << t.host_tid << "] " << t.name;
        if (t.guest_tid) out << " (guest tid " << t.guest_tid << ")";
        if (t.ctx) out << " [ctx]";
        out << "\n";
    }

    // 6. Recent file ring.
    out << "\n--- Recent Files (oldest first) ---\n";
    const auto recent_files = diag_recent_files();
    if (recent_files.empty()) {
        out << "  (none)\n";
    } else {
        for (const auto& f : recent_files) out << "  " << f << "\n";
    }

    // 7. Last zstd error.
    out << "\n--- Last ZSTD Error ---\n";
    const std::string zstd_err = diag_last_zstd_error();
    out << "  " << (zstd_err.empty() ? "(none)" : zstd_err) << "\n";

    // 8. Frame timing over the last 30 s.
    out << "\n--- Frame Timing (last 30s) ---\n";
    out << frame_times_summary(30.0) << "\n";

    snap.text = out.str();
    return snap;
}

std::string diag_memory_snapshot_text() { return diag_memory_snapshot().text; }

}  // namespace nxrt
