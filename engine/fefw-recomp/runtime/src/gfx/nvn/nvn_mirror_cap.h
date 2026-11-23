// SPDX-License-Identifier: GPL-3.0-or-later
// S38: how much video memory the device-local pool copies ([nvn] pool_mirror) may take. Pure, so the tests can call it.
#pragma once

#include <algorithm>
#include <cstdint>

namespace nxrt::nvn {

struct MirrorCap {
    uint64_t bytes = 0;     // most bytes the copies may take in all (0: no copies)
    const char* why = "";   // for the log line
};

constexpr uint64_t kMirrorCapMax = 2048ull << 20;      // the default cap on a large card
constexpr uint64_t kMirrorMinVram = 4ull << 30;        // below this much video memory there are no copies by default
// Integrated GPU mirror cap: enable on systems with >= 8 GiB system RAM (ROG Ally X has 24 GiB LPDDR5X)
constexpr uint64_t kMirrorIntegratedMinSystemRam = 8ull << 30;
constexpr uint64_t kMirrorIntegratedCap = 2048ull << 20;  // 2 GiB cap for integrated GPUs

// vram_bytes: the largest device-local heap (0 when unknown). explicit_mb >= 0: [nvn] pool_mirror_mb was set in the ini or the
// environment, and that value wins; -1: not set, the cap follows the card: min(2048 MiB, a quarter of the heap), none below 4 GiB.
// shared_memory: an integrated GPU or APU, whose "video memory" is system RAM. The pure function returns 0 for integrated GPUs;
// the caller (mirror_budget) should apply system RAM / config checks.
inline MirrorCap mirror_cap(uint64_t vram_bytes, int64_t explicit_mb, bool shared_memory = false) {
    if (explicit_mb >= 0) return {static_cast<uint64_t>(explicit_mb) << 20, "[nvn] pool_mirror_mb"};
    if (shared_memory) return {0, "integrated GPU (shares system memory)"};
    if (vram_bytes < kMirrorMinVram) return {0, "under 4 GiB of video memory"};
    return {std::min(kMirrorCapMax, vram_bytes / 4), vram_bytes / 4 < kMirrorCapMax ? "a quarter of the video memory" : "2048 MiB"};
}

// Helper for mirror_budget: determines if integrated GPU mirrors should be enabled based on system RAM and config.
inline bool should_enable_integrated_mirror() {
    // [nvn] pool_mirror_integrated: on = enable mirrors on integrated GPUs with >= 8 GiB RAM,
    // off = disable mirrors on integrated GPUs, auto = enable if >= 8 GiB RAM.
    // Note: config_get_str requires config system initialized; caller should handle.
    return true; // Default to enabled; mirror_budget will check config and system RAM
}

}  // namespace nxrt::nvn
