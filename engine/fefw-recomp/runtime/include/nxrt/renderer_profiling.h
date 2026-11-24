// SPDX-License-Identifier: GPL-3.0-or-later
// Renderer performance instrumentation (Issue #37 Phase 1).
// Provides: frame-time percentiles (P50/P95/P99), shader translation duration, pipeline creation duration,
// cache hits/misses, texture creation/destruction, resource upload/writeback, render-thread CPU time,
// Vulkan submission time, GPU sync wait, present/fence wait, GPU memory usage.
//
// Uses existing nxrt::ft (frame trace), nxrt::gauge (live counters), nxrt::hitch (phase timers) infrastructure.
// All metrics are lock-free or use relaxed atomics for minimal overhead.
//
// Enable with: [prof] renderer_profiling = true (env NXRT_PROF_RENDERER_PROFILING)
// Persist per-session CSV: [prof] renderer_csv = "renderer_profile.csv" (env NXRT_PROF_RENDERER_CSV)
#pragma once
#include <atomic>
#include <cstdint>
#include <functional>
#include <string>
#include <vector>

#include "nxrt/ftrace.h"
#include "nxrt/gauge.h"

namespace nxrt::rp {

// ---- Frame-time percentiles ----------------------------------------------------------------
// Rolling window of presented frame samples (ms). Updated from the presenting thread.
// Access via frame_time_p50()/p95()/p99() from any thread (lock-free snapshot).

struct FrameTimeWindow {
    static constexpr size_t kCapacity = 8192;
    std::atomic<uint64_t> count{0};
    std::atomic<uint64_t> head{0};
    float samples[kCapacity]{};
    std::atomic<uint64_t> frame_number{0};

    void add(float ms) {
        const uint64_t idx = head.fetch_add(1, std::memory_order_relaxed) % kCapacity;
        samples[idx] = ms;
        count.fetch_add(1, std::memory_order_relaxed);
        frame_number.fetch_add(1, std::memory_order_relaxed);
    }

    // Snapshot percentiles (allocates a sorted copy; call infrequently, e.g. once per second).
    struct Percentiles { float p50 = 0, p95 = 0, p99 = 0, max = 0; uint64_t frames = 0; };
    Percentiles snapshot() const;
};

// Global frame-time window (defined in renderer_profiling.cpp).
FrameTimeWindow& frame_time_window();

// Convenience accessors (lock-free, read current window state).
float frame_time_p50();
float frame_time_p95();
float frame_time_p99();
float frame_time_max();
uint64_t frame_time_frames();

// Call once per presented frame (from present path).
inline void note_frame_time(float ms) { frame_time_window().add(ms); }


// ---- Shader translation timing -------------------------------------------------------------
// Records duration of SPIR-V translation, patching, and module creation.

struct ShaderTranslationStats {
    std::atomic<uint64_t> translations{0};
    std::atomic<uint64_t> total_ns{0};
    std::atomic<uint64_t> max_ns{0};
    std::atomic<uint64_t> patch_ns{0};       // SPIR-V patching (alpha test, dual source, XFB, HDR variants)
    std::atomic<uint64_t> create_module_ns{0}; // vkCreateShaderModule
    std::atomic<uint64_t> cache_hits{0};      // module found in g_modules
    std::atomic<uint64_t> cache_misses{0};    // module created
};

ShaderTranslationStats& shader_translation_stats();

// RAII timer for shader translation phases.
struct ShaderTranslationTimer {
    uint64_t t0;
    explicit ShaderTranslationTimer(std::atomic<uint64_t>* acc = nullptr) : t0(nxrt::ft::now_ns()), acc_(acc) {}
    ~ShaderTranslationTimer() {
        const uint64_t dt = nxrt::ft::now_ns() - t0;
        if (acc_) acc_->fetch_add(dt, std::memory_order_relaxed);
    }
    std::atomic<uint64_t>* acc_ = nullptr;
};

inline uint64_t now_ns() { return nxrt::ft::now_ns(); }


// ---- Pipeline creation timing --------------------------------------------------------------
// Records duration of graphics/compute pipeline creation (vkCreateGraphicsPipelines, vkCreateComputePipelines).

struct PipelineCreationStats {
    std::atomic<uint64_t> graphics_created{0};
    std::atomic<uint64_t> graphics_failed{0};
    std::atomic<uint64_t> graphics_total_ns{0};
    std::atomic<uint64_t> graphics_max_ns{0};
    std::atomic<uint64_t> compute_created{0};
    std::atomic<uint64_t> compute_failed{0};
    std::atomic<uint64_t> compute_total_ns{0};
    std::atomic<uint64_t> compute_max_ns{0};
    std::atomic<uint64_t> cache_hits{0};      // pipeline found in g_gfx / g_compute
    std::atomic<uint64_t> cache_misses{0};    // pipeline created
    std::atomic<uint64_t> linked_from_parts{0}; // fast-linked from prebuilt parts
    std::atomic<uint64_t> prewarm_taken{0};   // taken from prewarm queue
    std::atomic<uint64_t> prewarm_waited{0};  // waited for prewarm build
    std::atomic<uint64_t> prewarm_wait_ns{0};
};

PipelineCreationStats& pipeline_creation_stats();

struct PipelineCreationTimer {
    uint64_t t0;
    std::atomic<uint64_t>* total_acc;
    std::atomic<uint64_t>* max_acc;
    std::atomic<uint64_t>* count_acc;
    PipelineCreationTimer(std::atomic<uint64_t>* tot, std::atomic<uint64_t>* mx, std::atomic<uint64_t>* cnt)
        : t0(now_ns()), total_acc(tot), max_acc(mx), count_acc(cnt) {}
    ~PipelineCreationTimer() {
        const uint64_t dt = now_ns() - t0;
        total_acc->fetch_add(dt, std::memory_order_relaxed);
        count_acc->fetch_add(1, std::memory_order_relaxed);
        uint64_t cur = max_acc->load(std::memory_order_relaxed);
        while (dt > cur && !max_acc->compare_exchange_weak(cur, dt, std::memory_order_relaxed)) {}
    }
};


// ---- Texture lifecycle tracking ------------------------------------------------------------
// Texture creation, destruction, and residency changes.

struct TextureLifecycleStats {
    std::atomic<uint64_t> created{0};
    std::atomic<uint64_t> destroyed{0};
    std::atomic<uint64_t> upload_bytes{0};
    std::atomic<uint64_t> upload_count{0};
    std::atomic<uint64_t> upload_ns{0};
    std::atomic<uint64_t> transcoded{0};      // ASTC->BC7 transcodes
    std::atomic<uint64_t> transcoded_ns{0};
    std::atomic<uint64_t> residency_changes{0};
    std::atomic<uint64_t> pool_hits{0};       // reused from pool
    std::atomic<uint64_t> pool_misses{0};     // new allocation
    std::atomic<uint64_t> sparse_binds{0};
    std::atomic<uint64_t> sparse_bind_ns{0};
    std::atomic<uint64_t> est_bytes{0};       // estimated GPU bytes for created textures
};

TextureLifecycleStats& texture_lifecycle_stats();


// ---- Resource upload / writeback -----------------------------------------------------------
// Staging buffer allocations, copies, writeback operations.

struct ResourceUploadStats {
    std::atomic<uint64_t> staging_allocs{0};
    std::atomic<uint64_t> staging_bytes{0};
    std::atomic<uint64_t> staging_ns{0};
    std::atomic<uint64_t> copy_ns{0};
    std::atomic<uint64_t> writeback_ns{0};    // async writeback (if enabled)
    std::atomic<uint64_t> writeback_bytes{0};
    std::atomic<uint64_t> ring_allocs{0};     // staging ring allocations
    std::atomic<uint64_t> ring_fallbacks{0};  // ring full -> private buffer
};

ResourceUploadStats& resource_upload_stats();


// ---- Render thread CPU time ----------------------------------------------------------------
// Time spent in render thread phases (replay, pipeline creation, uploads, present, idle).

struct RenderThreadStats {
    // Phase timers (nanoseconds, cumulative). Updated via HitchTimer (hitch_phase_add).
    // Phases match nxrt::HitchPhase but are tracked separately for renderer profiling.
    std::atomic<uint64_t> replay_ns{0};
    std::atomic<uint64_t> pipeline_ns{0};
    std::atomic<uint64_t> upload_ns{0};
    std::atomic<uint64_t> present_ns{0};
    std::atomic<uint64_t> acquire_ns{0};
    std::atomic<uint64_t> limiter_ns{0};
    std::atomic<uint64_t> idle_ns{0};
    std::atomic<uint64_t> gpu_wait_ns{0};
    std::atomic<uint64_t> submit_ns{0};
    std::atomic<uint64_t> lock_wait_ns{0};
    std::atomic<uint64_t> tex_prep_ns{0};
    std::atomic<uint64_t> tex_staging_ns{0};
    std::atomic<uint64_t> image_creation_ns{0};
    std::atomic<uint64_t> split_submit_ns{0};

    // Frame-level: set by render thread at end of frame.
    std::atomic<uint64_t> frame_cpu_ns{0};
    std::atomic<uint32_t> frames{0};
};

RenderThreadStats& render_thread_stats();

// Adds a render-thread phase's duration to its RenderThreadStats counter (see nxrt::rp::phase for the ids).
void render_phase_add(int phase, uint64_t ns);

// RAII phase timer for the render thread's phases.
struct RenderPhaseTimer {
    int phase;
    uint64_t t0;
    explicit RenderPhaseTimer(int p) : phase(p), t0(now_ns()) {}
    ~RenderPhaseTimer() { render_phase_add(phase, now_ns() - t0); }
};


// ---- Vulkan submission timing --------------------------------------------------------------
// vkQueueSubmit2 duration, command buffer count, wait/signal semaphore counts.

struct SubmissionStats {
    std::atomic<uint64_t> submits{0};
    std::atomic<uint64_t> total_ns{0};
    std::atomic<uint64_t> max_ns{0};
    std::atomic<uint64_t> cmd_buffers{0};
    std::atomic<uint64_t> wait_semaphores{0};
    std::atomic<uint64_t> signal_semaphores{0};
    std::atomic<uint64_t> batched_submits{0};  // submissions merged in batch
    std::atomic<uint64_t> batch_flush_ns{0};
};

SubmissionStats& submission_stats();

struct SubmissionTimer {
    uint64_t t0;
    explicit SubmissionTimer() : t0(now_ns()) {}
    ~SubmissionTimer() {
        const uint64_t dt = now_ns() - t0;
        submission_stats().total_ns.fetch_add(dt, std::memory_order_relaxed);
        submission_stats().submits.fetch_add(1, std::memory_order_relaxed);
        uint64_t cur = submission_stats().max_ns.load(std::memory_order_relaxed);
        while (dt > cur && !submission_stats().max_ns.compare_exchange_weak(cur, dt, std::memory_order_relaxed)) {}
    }
};


// ---- GPU sync wait --------------------------------------------------------------
// Timeline semaphore waits, fence waits, vkWaitForFences.

struct GpuSyncStats {
    std::atomic<uint64_t> timeline_waits{0};
    std::atomic<uint64_t> timeline_wait_ns{0};
    std::atomic<uint64_t> fence_waits{0};
    std::atomic<uint64_t> fence_wait_ns{0};
    std::atomic<uint64_t> present_waits{0};
    std::atomic<uint64_t> present_wait_ns{0};
    std::atomic<uint64_t> acquire_waits{0};
    std::atomic<uint64_t> acquire_wait_ns{0};
};

GpuSyncStats& gpu_sync_stats();

struct GpuSyncTimer {
    uint64_t t0;
    std::atomic<uint64_t>* wait_acc;
    std::atomic<uint64_t>* count_acc;
    GpuSyncTimer(std::atomic<uint64_t>* w, std::atomic<uint64_t>* c) : t0(now_ns()), wait_acc(w), count_acc(c) {}
    ~GpuSyncTimer() {
        const uint64_t dt = now_ns() - t0;
        wait_acc->fetch_add(dt, std::memory_order_relaxed);
        count_acc->fetch_add(1, std::memory_order_relaxed);
    }
};


// ---- GPU memory usage --------------------------------------------------------------
// Device-local, host-visible, and total allocation tracking via vk_gauges.

struct GpuMemoryStats {
    std::atomic<uint64_t> device_local_bytes{0};
    std::atomic<uint64_t> host_visible_bytes{0};
    std::atomic<uint64_t> total_bytes{0};
    std::atomic<uint64_t> peak_bytes{0};
    std::atomic<uint64_t> allocation_count{0};
    std::atomic<uint64_t> allocation_failures{0};
};

GpuMemoryStats& gpu_memory_stats();

// Update from vk_gauges (called periodically from soak sampler or render thread).
void gpu_memory_stats_update();


// ---- Pipeline prewarm stats --------------------------------------------------------------
// Bounded prewarm: metadata-driven, limited queue, priority by recency/frequency.

struct PrewarmStats {
    std::atomic<uint64_t> recorded_keys{0};
    std::atomic<uint64_t> queued_keys{0};
    std::atomic<uint64_t> built_ahead{0};
    std::atomic<uint64_t> build_failures{0};
    std::atomic<uint64_t> taken_by_draw{0};
    std::atomic<uint64_t> waited_for{0};
    std::atomic<uint64_t> wait_ns{0};
    std::atomic<uint64_t> linked_while_building{0};
    std::atomic<uint64_t> claimed_by_draw{0};
    std::atomic<uint64_t> skipped_busy{0};
    std::atomic<uint64_t> promoted_keys{0};
    std::atomic<uint64_t> queue_depth{0};      // current
    std::atomic<uint64_t> queue_depth_max{0};  // peak
    std::atomic<uint64_t> workers_active{0};
    std::atomic<uint64_t> metadata_lookups{0};
    std::atomic<uint64_t> metadata_hits{0};
    std::atomic<uint64_t> bounded_drops{0};    // dropped due to queue bound
};

PrewarmStats& prewarm_stats();


// ---- Initialization / shutdown -----------------------------------------------------------

void renderer_profiling_init();
void renderer_profiling_shutdown();

// Per-frame snapshot (called from present path or soak sampler).
// Returns JSON string with all current metrics.
std::string renderer_profiling_snapshot_json();

// CSV header for [prof] renderer_csv.
std::string renderer_profiling_csv_header();
// CSV row for current metrics.
std::string renderer_profiling_csv_row();
// Appends one row to the open [prof] renderer_csv file (no-op when profiling or the file is off).
// The present path calls it about once a second.
void renderer_profiling_periodic_csv();

// Register gauges for soak sampler ([diag] soak_csv).
void renderer_profiling_register_gauges();

// Frame-time percentile gauges (for soak sampler).
int64_t gauge_frame_time_p50();
int64_t gauge_frame_time_p95();
int64_t gauge_frame_time_p99();
int64_t gauge_frame_time_max();

}  // namespace nxrt::rp

// ---- Convenience macros ----------------------------------------------------------------

#define NXRP_FT(name) NXFT(name)  // reuse frame trace
#define NXRP_FT_MIN(name, min_ns) NXFT_MIN(name, min_ns)

#define NXRP_SHADER_TIMER(stats_acc) nxrt::rp::ShaderTranslationTimer _timer(&stats_acc)
#define NXRP_PIPELINE_TIMER(tot, mx, cnt) nxrt::rp::PipelineCreationTimer _timer(tot, mx, cnt)
#define NXRP_SUBMISSION_TIMER() nxrt::rp::SubmissionTimer _timer
#define NXRP_GPU_SYNC_TIMER(wait_acc, count_acc) nxrt::rp::GpuSyncTimer _timer(wait_acc, count_acc)
#define NXRP_RENDER_PHASE(phase) nxrt::rp::RenderPhaseTimer _timer(phase)
#define NXRP_UPLOAD_TIMER(acc) nxrt::rp::ShaderTranslationTimer _timer(&(acc))  // reuse for ns accumulation

// Phase IDs for render thread (match nxrt::HitchPhase where applicable).
namespace nxrt::rp::phase {
enum {
    kReplay = 0,
    kPipeline = 1,
    kUpload = 2,
    kPresent = 3,
    kAcquire = 4,
    kLimiter = 5,
    kIdle = 6,
    kGpuWait = 7,
    kSubmit = 8,
    kLockWait = 9,
    kTexPrep = 10,
    kTexStaging = 11,
    kImageCreation = 12,
    kSplitSubmit = 13,
    kCount
};
}  // namespace nxrt::rp::phase