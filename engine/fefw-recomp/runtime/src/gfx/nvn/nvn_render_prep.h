// SPDX-License-Identifier: GPL-3.0-or-later
// Background render preparation thread pool (S21+). Runs descriptor set construction and other
// render-thread preparatory work in parallel on worker threads.
#pragma once
#include "nvn_core.h"
#include "nxrt/gfx.h"

namespace nxrt::nvn::be::gpu {
struct ProgLayout;
}

namespace nxrt::nvn::be::renderprep {

// Initialize the render prep thread pool. Called once at startup.
void init();

// Shutdown the render prep thread pool.
void shutdown();

// Check if render prep is enabled.
bool enabled();

// A work item for building a descriptor set.
struct SetBuildWork {
    const gpu::ProgLayout* pl = nullptr;
    uint32_t set = 0;
    VkDescriptorSet result = VK_NULL_HANDLE;
    uint64_t content_key = 0;   // key for set cache lookup
    bool cache_hit = false;
};

// Submit a descriptor set build job. Returns true if queued, false if executed synchronously.
// The work item will be filled with the result when complete.
// If synchronous, the result is ready immediately on return.
bool build_set_async(SetBuildWork& work);

// Wait for all pending render prep jobs to complete.
void wait_idle();

// Statistics for monitoring.
struct Stats {
    uint64_t sets_built_async = 0;
    uint64_t sets_built_sync = 0;
    uint64_t cache_hits = 0;
    uint64_t cache_misses = 0;
};
Stats stats();

}  // namespace nxrt::nvn::be::renderprep