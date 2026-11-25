// SPDX-License-Identifier: GPL-3.0-or-later
// Renderer performance instrumentation implementation (Issue #37 Phase 1).
#include "nxrt/renderer_profiling.h"

#include <algorithm>
#include <array>
#include <chrono>
#include <cstdio>
#include <cstring>
#include <fstream>
#include <iomanip>
#include <mutex>
#include <sstream>
#include <vector>

#include "nxrt/config.h"
#include "nxrt/ftrace.h"
#include "nxrt/gauge.h"
#include "nxrt/log.h"

// Forward declarations for vk_gauge functions (defined in vk_device.cpp, declared in vk_gauges.h).
// Avoids including vk_gauges.h which requires volk.h not in diag include path.
namespace nxrt {
nxrt::GaugeCounter& vk_gauge_image_bytes();
nxrt::GaugeCounter& vk_gauge_buffer_bytes();
nxrt::GaugeCounter& vk_gauge_memory_bytes();
nxrt::GaugeCounter& vk_gauge_memory_allocs();
}

namespace nxrt::rp {

namespace {
constexpr const char* kTag = "nvn.rprof";
}  // namespace

bool g_enabled = false;
std::string g_csv_path;
std::ofstream g_csv_file;
std::mutex g_csv_mu;
uint64_t g_csv_frame = 0;
bool g_gauges_registered = false;

// FrameTimeWindow implementation -----------------------------------------------------------

FrameTimeWindow::Percentiles FrameTimeWindow::snapshot() const {
    Percentiles p;
    const uint64_t n = std::min(count.load(std::memory_order_relaxed), kCapacity);
    if (n == 0) return p;

    std::vector<float> sorted;
    sorted.reserve(n);
    const uint64_t h = head.load(std::memory_order_relaxed);
    const uint64_t start = h - n;   // oldest sample in the window (head is the next write index)
    for (uint64_t i = 0; i < n; ++i) {
        const uint64_t idx = (start + i) % kCapacity;
        sorted.push_back(samples[idx]);
    }
    std::sort(sorted.begin(), sorted.end());

    p.frames = n;
    p.p50 = sorted[static_cast<size_t>(n * 0.50)];
    p.p95 = sorted[static_cast<size_t>(n * 0.95)];
    p.p99 = sorted[static_cast<size_t>(n * 0.99)];
    p.max = sorted.back();
    return p;
}

FrameTimeWindow& frame_time_window() {
    static FrameTimeWindow w;
    return w;
}

// Render-thread phase accounting. The phase ids of nxrt::rp::phase are this module's own (they do not line up with
// nxrt::HitchPhase), so a timer adds to the matching RenderThreadStats field instead of the hitch ring.
void render_phase_add(int phase, uint64_t ns) {
    auto& s = render_thread_stats();
    std::atomic<uint64_t>* acc = nullptr;
    switch (phase) {
        case phase::kReplay: acc = &s.replay_ns; break;
        case phase::kPipeline: acc = &s.pipeline_ns; break;
        case phase::kUpload: acc = &s.upload_ns; break;
        case phase::kPresent: acc = &s.present_ns; break;
        case phase::kAcquire: acc = &s.acquire_ns; break;
        case phase::kLimiter: acc = &s.limiter_ns; break;
        case phase::kIdle: acc = &s.idle_ns; break;
        case phase::kGpuWait: acc = &s.gpu_wait_ns; break;
        case phase::kSubmit: acc = &s.submit_ns; break;
        case phase::kLockWait: acc = &s.lock_wait_ns; break;
        case phase::kTexPrep: acc = &s.tex_prep_ns; break;
        case phase::kTexStaging: acc = &s.tex_staging_ns; break;
        case phase::kImageCreation: acc = &s.image_creation_ns; break;
        case phase::kSplitSubmit: acc = &s.split_submit_ns; break;
        default: return;
    }
    acc->fetch_add(ns, std::memory_order_relaxed);
}

float frame_time_p50() { return frame_time_window().snapshot().p50; }
float frame_time_p95() { return frame_time_window().snapshot().p95; }
float frame_time_p99() { return frame_time_window().snapshot().p99; }
float frame_time_max() { return frame_time_window().snapshot().max; }
uint64_t frame_time_frames() { return frame_time_window().frame_number.load(std::memory_order_relaxed); }


// ShaderTranslationStats --------------------------------------------------------------------

ShaderTranslationStats& shader_translation_stats() {
    static ShaderTranslationStats s;
    return s;
}


// PipelineCreationStats ---------------------------------------------------------------------

PipelineCreationStats& pipeline_creation_stats() {
    static PipelineCreationStats s;
    return s;
}


// TextureLifecycleStats ---------------------------------------------------------------------

TextureLifecycleStats& texture_lifecycle_stats() {
    static TextureLifecycleStats s;
    return s;
}


// ResourceUploadStats -----------------------------------------------------------------------

ResourceUploadStats& resource_upload_stats() {
    static ResourceUploadStats s;
    return s;
}


// RenderThreadStats -------------------------------------------------------------------------

RenderThreadStats& render_thread_stats() {
    static RenderThreadStats s;
    return s;
}


// SubmissionStats ---------------------------------------------------------------------------

SubmissionStats& submission_stats() {
    static SubmissionStats s;
    return s;
}


// GpuSyncStats ------------------------------------------------------------------------------

GpuSyncStats& gpu_sync_stats() {
    static GpuSyncStats s;
    return s;
}


// GpuMemoryStats ----------------------------------------------------------------------------

GpuMemoryStats& gpu_memory_stats() {
    static GpuMemoryStats s;
    return s;
}

void gpu_memory_stats_update() {
    auto& s = gpu_memory_stats();
    s.device_local_bytes.store(vk_gauge_image_bytes().get() + vk_gauge_buffer_bytes().get(), std::memory_order_relaxed);
    s.host_visible_bytes.store(vk_gauge_memory_bytes().get() - s.device_local_bytes.load(std::memory_order_relaxed), std::memory_order_relaxed);
    s.total_bytes.store(vk_gauge_memory_bytes().get(), std::memory_order_relaxed);
    const uint64_t cur = s.total_bytes.load(std::memory_order_relaxed);
    uint64_t peak = s.peak_bytes.load(std::memory_order_relaxed);
    while (cur > peak && !s.peak_bytes.compare_exchange_weak(peak, cur, std::memory_order_relaxed)) {}
    s.allocation_count.store(vk_gauge_memory_allocs().get(), std::memory_order_relaxed);
}


// PrewarmStats ------------------------------------------------------------------------------

PrewarmStats& prewarm_stats() {
    static PrewarmStats s;
    return s;
}


// CSV output --------------------------------------------------------------------------------

void write_csv_header_locked() {
    if (!g_csv_file.is_open()) return;
    g_csv_file << "frame,timestamp_s,";
    g_csv_file << "ft_p50_ms,ft_p95_ms,ft_p99_ms,ft_max_ms,ft_frames,";
    g_csv_file << "shader_translations,shader_total_ms,shader_max_ms,shader_patch_ms,shader_module_ms,shader_cache_hits,shader_cache_misses,";
    g_csv_file << "pipe_gfx_created,pipe_gfx_failed,pipe_gfx_total_ms,pipe_gfx_max_ms,pipe_comp_created,pipe_comp_failed,pipe_comp_total_ms,pipe_comp_max_ms,pipe_cache_hits,pipe_cache_misses,pipe_linked_parts,pipe_prewarm_taken,pipe_prewarm_waited,pipe_prewarm_wait_ms,";
    g_csv_file << "tex_created,tex_destroyed,tex_upload_mb,tex_upload_count,tex_upload_ms,tex_transcoded,tex_transcoded_ms,tex_residency_changes,tex_pool_hits,tex_pool_misses,tex_sparse_binds,tex_sparse_bind_ms,";
    g_csv_file << "upload_staging_allocs,upload_staging_mb,upload_staging_ms,upload_copy_ms,upload_writeback_ms,upload_writeback_mb,upload_ring_allocs,upload_ring_fallbacks,";
    g_csv_file << "rt_replay_ms,rt_pipeline_ms,rt_upload_ms,rt_present_ms,rt_acquire_ms,rt_limiter_ms,rt_idle_ms,rt_gpu_wait_ms,rt_submit_ms,rt_lock_wait_ms,rt_tex_prep_ms,rt_tex_staging_ms,rt_image_ms,rt_split_ms,rt_frame_cpu_ms,rt_frames,";
    g_csv_file << "sub_submits,sub_total_ms,sub_max_ms,sub_cmd_buffers,sub_wait_sems,sub_signal_sems,sub_batched,sub_batch_flush_ms,";
    g_csv_file << "sync_timeline_waits,sync_timeline_wait_ms,sync_fence_waits,sync_fence_wait_ms,sync_present_waits,sync_present_wait_ms,sync_acquire_waits,sync_acquire_wait_ms,";
    g_csv_file << "gpu_dev_local_mb,gpu_host_vis_mb,gpu_total_mb,gpu_peak_mb,gpu_allocs,gpu_alloc_failures,";
    g_csv_file << "pw_recorded,pw_queued,pw_built_ahead,pw_failed,pw_taken,pw_waited,pw_wait_ms,pw_linked_building,pw_claimed,pw_skipped,pw_promoted,pw_queue_depth,pw_queue_max,pw_workers,pw_meta_lookups,pw_meta_hits,pw_bounded_drops";
    g_csv_file << "\n";
}

void write_csv_row_locked() {
    if (!g_csv_file.is_open()) return;

    const auto ft = frame_time_window().snapshot();
    const auto& st = shader_translation_stats();
    const auto& ps = pipeline_creation_stats();
    const auto& ts = texture_lifecycle_stats();
    const auto& us = resource_upload_stats();
    const auto& rs = render_thread_stats();
    const auto& ss = submission_stats();
    const auto& gs = gpu_sync_stats();
    const auto& ms = gpu_memory_stats();
    const auto& pw = prewarm_stats();

    const double t = static_cast<double>(std::chrono::steady_clock::now().time_since_epoch().count()) * 1e-9;

    g_csv_file << g_csv_frame++ << "," << std::fixed << std::setprecision(3) << t << ",";
    g_csv_file << ft.p50 << "," << ft.p95 << "," << ft.p99 << "," << ft.max << "," << ft.frames << ",";
    g_csv_file << st.translations.load() << "," << st.total_ns.load() * 1e-6 << "," << st.max_ns.load() * 1e-6 << ","
               << st.patch_ns.load() * 1e-6 << "," << st.create_module_ns.load() * 1e-6 << ","
               << st.cache_hits.load() << "," << st.cache_misses.load() << ",";
    g_csv_file << ps.graphics_created.load() << "," << ps.graphics_failed.load() << ","
               << ps.graphics_total_ns.load() * 1e-6 << "," << ps.graphics_max_ns.load() * 1e-6 << ","
               << ps.compute_created.load() << "," << ps.compute_failed.load() << ","
               << ps.compute_total_ns.load() * 1e-6 << "," << ps.compute_max_ns.load() * 1e-6 << ","
               << ps.cache_hits.load() << "," << ps.cache_misses.load() << ","
               << ps.linked_from_parts.load() << "," << ps.prewarm_taken.load() << ","
               << ps.prewarm_waited.load() << "," << ps.prewarm_wait_ns.load() * 1e-6 << ",";
    g_csv_file << ts.created.load() << "," << ts.destroyed.load() << ","
               << ts.upload_bytes.load() * 1e-6 << "," << ts.upload_count.load() << ","
               << ts.upload_ns.load() * 1e-6 << "," << ts.transcoded.load() << ","
               << ts.transcoded_ns.load() * 1e-6 << "," << ts.residency_changes.load() << ","
               << ts.pool_hits.load() << "," << ts.pool_misses.load() << ","
               << ts.sparse_binds.load() << "," << ts.sparse_bind_ns.load() * 1e-6 << ",";
    g_csv_file << us.staging_allocs.load() << "," << us.staging_bytes.load() * 1e-6 << ","
               << us.staging_ns.load() * 1e-6 << "," << us.copy_ns.load() * 1e-6 << ","
               << us.writeback_ns.load() * 1e-6 << "," << us.writeback_bytes.load() * 1e-6 << ","
               << us.ring_allocs.load() << "," << us.ring_fallbacks.load() << ",";
    g_csv_file << rs.replay_ns.load() * 1e-6 << "," << rs.pipeline_ns.load() * 1e-6 << ","
               << rs.upload_ns.load() * 1e-6 << "," << rs.present_ns.load() * 1e-6 << ","
               << rs.acquire_ns.load() * 1e-6 << "," << rs.limiter_ns.load() * 1e-6 << ","
               << rs.idle_ns.load() * 1e-6 << "," << rs.gpu_wait_ns.load() * 1e-6 << ","
               << rs.submit_ns.load() * 1e-6 << "," << rs.lock_wait_ns.load() * 1e-6 << ","
               << rs.tex_prep_ns.load() * 1e-6 << "," << rs.tex_staging_ns.load() * 1e-6 << ","
               << rs.image_creation_ns.load() * 1e-6 << "," << rs.split_submit_ns.load() * 1e-6 << ","
               << rs.frame_cpu_ns.load() * 1e-6 << "," << rs.frames.load() << ",";
    g_csv_file << ss.submits.load() << "," << ss.total_ns.load() * 1e-6 << ","
               << ss.max_ns.load() * 1e-6 << "," << ss.cmd_buffers.load() << ","
               << ss.wait_semaphores.load() << "," << ss.signal_semaphores.load() << ","
               << ss.batched_submits.load() << "," << ss.batch_flush_ns.load() * 1e-6 << ",";
    g_csv_file << gs.timeline_waits.load() << "," << gs.timeline_wait_ns.load() * 1e-6 << ","
               << gs.fence_waits.load() << "," << gs.fence_wait_ns.load() * 1e-6 << ","
               << gs.present_waits.load() << "," << gs.present_wait_ns.load() * 1e-6 << ","
               << gs.acquire_waits.load() << "," << gs.acquire_wait_ns.load() * 1e-6 << ",";
    g_csv_file << ms.device_local_bytes.load() * 1e-6 << ","
               << ms.host_visible_bytes.load() * 1e-6 << ","
               << ms.total_bytes.load() * 1e-6 << ","
               << ms.peak_bytes.load() * 1e-6 << ","
               << ms.allocation_count.load() << ","
               << ms.allocation_failures.load() << ",";
    g_csv_file << pw.recorded_keys.load() << "," << pw.queued_keys.load() << ","
               << pw.built_ahead.load() << "," << pw.build_failures.load() << ","
               << pw.taken_by_draw.load() << "," << pw.waited_for.load() << ","
               << pw.wait_ns.load() * 1e-6 << "," << pw.linked_while_building.load() << ","
               << pw.claimed_by_draw.load() << "," << pw.skipped_busy.load() << ","
               << pw.promoted_keys.load() << "," << pw.queue_depth.load() << ","
               << pw.queue_depth_max.load() << "," << pw.workers_active.load() << ","
               << pw.metadata_lookups.load() << "," << pw.metadata_hits.load() << ","
               << pw.bounded_drops.load();
    g_csv_file << "\n";
    g_csv_file.flush();
}


// Initialization / shutdown -----------------------------------------------------------------

void renderer_profiling_init() {
    if (g_enabled) return;
    g_enabled = config_get_bool("prof", "renderer_profiling", false);
    if (!g_enabled) return;

    g_csv_path = config_get_str("prof", "renderer_csv", "");
    if (!g_csv_path.empty()) {
        g_csv_file.open(g_csv_path, std::ios::trunc);
        if (g_csv_file.is_open()) {
            write_csv_header_locked();
            NXLOG_I(kTag, "renderer profiling CSV: %s", g_csv_path.c_str());
        } else {
            NXLOG_W(kTag, "failed to open renderer profiling CSV: %s", g_csv_path.c_str());
        }
    }

    // Register gauges for soak sampler.
    renderer_profiling_register_gauges();

    NXLOG_I(kTag, "renderer profiling enabled");
}

void renderer_profiling_shutdown() {
    if (!g_enabled) return;
    // Write one final CSV row before closing.
    if (g_csv_file.is_open()) {
        std::lock_guard<std::mutex> lk(g_csv_mu);
        write_csv_row_locked();
        g_csv_file.close();
    }
    g_enabled = false;
    NXLOG_I(kTag, "renderer profiling shutdown");
}


// JSON snapshot -----------------------------------------------------------------------------

std::string renderer_profiling_snapshot_json() {
    const auto ft = frame_time_window().snapshot();
    const auto& st = shader_translation_stats();
    const auto& ps = pipeline_creation_stats();
    const auto& ts = texture_lifecycle_stats();
    const auto& us = resource_upload_stats();
    const auto& rs = render_thread_stats();
    const auto& ss = submission_stats();
    const auto& gs = gpu_sync_stats();
    const auto& ms = gpu_memory_stats();
    const auto& pw = prewarm_stats();

    std::ostringstream o;
    o << std::fixed << std::setprecision(3);
    o << "{";
    o << "\"frame_time\":{\"p50\":" << ft.p50 << ",\"p95\":" << ft.p95 << ",\"p99\":" << ft.p99 << ",\"max\":" << ft.max << ",\"frames\":" << ft.frames << "},";
    o << "\"shader\":{\"translations\":" << st.translations.load() << ",\"total_ms\":" << st.total_ns.load() * 1e-6
      << ",\"max_ms\":" << st.max_ns.load() * 1e-6 << ",\"patch_ms\":" << st.patch_ns.load() * 1e-6
      << ",\"module_ms\":" << st.create_module_ns.load() * 1e-6
      << ",\"cache_hits\":" << st.cache_hits.load() << ",\"cache_misses\":" << st.cache_misses.load() << "},";
    o << "\"pipeline\":{\"gfx_created\":" << ps.graphics_created.load() << ",\"gfx_failed\":" << ps.graphics_failed.load()
      << ",\"gfx_total_ms\":" << ps.graphics_total_ns.load() * 1e-6 << ",\"gfx_max_ms\":" << ps.graphics_max_ns.load() * 1e-6
      << ",\"comp_created\":" << ps.compute_created.load() << ",\"comp_failed\":" << ps.compute_failed.load()
      << ",\"comp_total_ms\":" << ps.compute_total_ns.load() * 1e-6 << ",\"comp_max_ms\":" << ps.compute_max_ns.load() * 1e-6
      << ",\"cache_hits\":" << ps.cache_hits.load() << ",\"cache_misses\":" << ps.cache_misses.load()
      << ",\"linked_parts\":" << ps.linked_from_parts.load() << ",\"prewarm_taken\":" << ps.prewarm_taken.load()
      << ",\"prewarm_waited\":" << ps.prewarm_waited.load() << ",\"prewarm_wait_ms\":" << ps.prewarm_wait_ns.load() * 1e-6 << "},";
    o << "\"texture\":{\"created\":" << ts.created.load() << ",\"destroyed\":" << ts.destroyed.load()
      << ",\"upload_mb\":" << ts.upload_bytes.load() * 1e-6 << ",\"upload_count\":" << ts.upload_count.load()
      << ",\"upload_ms\":" << ts.upload_ns.load() * 1e-6 << ",\"transcoded\":" << ts.transcoded.load()
      << ",\"transcoded_ms\":" << ts.transcoded_ns.load() * 1e-6 << ",\"residency_changes\":" << ts.residency_changes.load()
      << ",\"pool_hits\":" << ts.pool_hits.load() << ",\"pool_misses\":" << ts.pool_misses.load()
      << ",\"sparse_binds\":" << ts.sparse_binds.load() << ",\"sparse_bind_ms\":" << ts.sparse_bind_ns.load() * 1e-6 << "},";
    o << "\"upload\":{\"staging_allocs\":" << us.staging_allocs.load() << ",\"staging_mb\":" << us.staging_bytes.load() * 1e-6
      << ",\"staging_ms\":" << us.staging_ns.load() * 1e-6 << ",\"copy_ms\":" << us.copy_ns.load() * 1e-6
      << ",\"writeback_ms\":" << us.writeback_ns.load() * 1e-6 << ",\"writeback_mb\":" << us.writeback_bytes.load() * 1e-6
      << ",\"ring_allocs\":" << us.ring_allocs.load() << ",\"ring_fallbacks\":" << us.ring_fallbacks.load() << "},";
    o << "\"render_thread\":{\"replay_ms\":" << rs.replay_ns.load() * 1e-6 << ",\"pipeline_ms\":" << rs.pipeline_ns.load() * 1e-6
      << ",\"upload_ms\":" << rs.upload_ns.load() * 1e-6 << ",\"present_ms\":" << rs.present_ns.load() * 1e-6
      << ",\"acquire_ms\":" << rs.acquire_ns.load() * 1e-6 << ",\"limiter_ms\":" << rs.limiter_ns.load() * 1e-6
      << ",\"idle_ms\":" << rs.idle_ns.load() * 1e-6 << ",\"gpu_wait_ms\":" << rs.gpu_wait_ns.load() * 1e-6
      << ",\"submit_ms\":" << rs.submit_ns.load() * 1e-6 << ",\"lock_wait_ms\":" << rs.lock_wait_ns.load() * 1e-6
      << ",\"tex_prep_ms\":" << rs.tex_prep_ns.load() * 1e-6 << ",\"tex_staging_ms\":" << rs.tex_staging_ns.load() * 1e-6
      << ",\"image_ms\":" << rs.image_creation_ns.load() * 1e-6 << ",\"split_ms\":" << rs.split_submit_ns.load() * 1e-6
      << ",\"frame_cpu_ms\":" << rs.frame_cpu_ns.load() * 1e-6 << ",\"frames\":" << rs.frames.load() << "},";
    o << "\"submission\":{\"submits\":" << ss.submits.load() << ",\"total_ms\":" << ss.total_ns.load() * 1e-6
      << ",\"max_ms\":" << ss.max_ns.load() * 1e-6 << ",\"cmd_buffers\":" << ss.cmd_buffers.load()
      << ",\"wait_sems\":" << ss.wait_semaphores.load() << ",\"signal_sems\":" << ss.signal_semaphores.load()
      << ",\"batched\":" << ss.batched_submits.load() << ",\"batch_flush_ms\":" << ss.batch_flush_ns.load() * 1e-6 << "},";
    o << "\"gpu_sync\":{\"timeline_waits\":" << gs.timeline_waits.load() << ",\"timeline_wait_ms\":" << gs.timeline_wait_ns.load() * 1e-6
      << ",\"fence_waits\":" << gs.fence_waits.load() << ",\"fence_wait_ms\":" << gs.fence_wait_ns.load() * 1e-6
      << ",\"present_waits\":" << gs.present_waits.load() << ",\"present_wait_ms\":" << gs.present_wait_ns.load() * 1e-6
      << ",\"acquire_waits\":" << gs.acquire_waits.load() << ",\"acquire_wait_ms\":" << gs.acquire_wait_ns.load() * 1e-6 << "},";
    o << "\"gpu_memory\":{\"dev_local_mb\":" << ms.device_local_bytes.load() * 1e-6
      << ",\"host_vis_mb\":" << ms.host_visible_bytes.load() * 1e-6
      << ",\"total_mb\":" << ms.total_bytes.load() * 1e-6 << ",\"peak_mb\":" << ms.peak_bytes.load() * 1e-6
      << ",\"allocs\":" << ms.allocation_count.load() << ",\"alloc_failures\":" << ms.allocation_failures.load() << "},";
    o << "\"prewarm\":{\"recorded\":" << pw.recorded_keys.load() << ",\"queued\":" << pw.queued_keys.load()
      << ",\"built_ahead\":" << pw.built_ahead.load() << ",\"failed\":" << pw.build_failures.load()
      << ",\"taken\":" << pw.taken_by_draw.load() << ",\"waited\":" << pw.waited_for.load()
      << ",\"wait_ms\":" << pw.wait_ns.load() * 1e-6 << ",\"linked_building\":" << pw.linked_while_building.load()
      << ",\"claimed\":" << pw.claimed_by_draw.load() << ",\"skipped\":" << pw.skipped_busy.load()
      << ",\"promoted\":" << pw.promoted_keys.load() << ",\"queue_depth\":" << pw.queue_depth.load()
      << ",\"queue_max\":" << pw.queue_depth_max.load() << ",\"workers\":" << pw.workers_active.load()
      << ",\"meta_lookups\":" << pw.metadata_lookups.load() << ",\"meta_hits\":" << pw.metadata_hits.load()
      << ",\"bounded_drops\":" << pw.bounded_drops.load() << "}";
    return o.str();
}

std::string renderer_profiling_csv_header() {
    std::ostringstream o;
    o << "frame,timestamp_s,";
    o << "ft_p50_ms,ft_p95_ms,ft_p99_ms,ft_max_ms,ft_frames,";
    o << "shader_translations,shader_total_ms,shader_max_ms,shader_patch_ms,shader_module_ms,shader_cache_hits,shader_cache_misses,";
    o << "pipe_gfx_created,pipe_gfx_failed,pipe_gfx_total_ms,pipe_gfx_max_ms,pipe_comp_created,pipe_comp_failed,pipe_comp_total_ms,pipe_comp_max_ms,pipe_cache_hits,pipe_cache_misses,pipe_linked_parts,pipe_prewarm_taken,pipe_prewarm_waited,pipe_prewarm_wait_ms,";
    o << "tex_created,tex_destroyed,tex_upload_mb,tex_upload_count,tex_upload_ms,tex_transcoded,tex_transcoded_ms,tex_residency_changes,tex_pool_hits,tex_pool_misses,tex_sparse_binds,tex_sparse_bind_ms,";
    o << "upload_staging_allocs,upload_staging_mb,upload_staging_ms,upload_copy_ms,upload_writeback_ms,upload_writeback_mb,upload_ring_allocs,upload_ring_fallbacks,";
    o << "rt_replay_ms,rt_pipeline_ms,rt_upload_ms,rt_present_ms,rt_acquire_ms,rt_limiter_ms,rt_idle_ms,rt_gpu_wait_ms,rt_submit_ms,rt_lock_wait_ms,rt_tex_prep_ms,rt_tex_staging_ms,rt_image_ms,rt_split_ms,rt_frame_cpu_ms,rt_frames,";
    o << "sub_submits,sub_total_ms,sub_max_ms,sub_cmd_buffers,sub_wait_sems,sub_signal_sems,sub_batched,sub_batch_flush_ms,";
    o << "sync_timeline_waits,sync_timeline_wait_ms,sync_fence_waits,sync_fence_wait_ms,sync_present_waits,sync_present_wait_ms,sync_acquire_waits,sync_acquire_wait_ms,";
    o << "gpu_dev_local_mb,gpu_host_vis_mb,gpu_total_mb,gpu_peak_mb,gpu_allocs,gpu_alloc_failures,";
    o << "pw_recorded,pw_queued,pw_built_ahead,pw_failed,pw_taken,pw_waited,pw_wait_ms,pw_linked_building,pw_claimed,pw_skipped,pw_promoted,pw_queue_depth,pw_queue_max,pw_workers,pw_meta_lookups,pw_meta_hits,pw_bounded_drops";
    return o.str();
}

std::string renderer_profiling_csv_row() {
    std::lock_guard<std::mutex> lk(g_csv_mu);
    std::ostringstream o;
    // We need to replicate write_csv_row_locked logic here without file I/O
    const auto ft = frame_time_window().snapshot();
    const auto& st = shader_translation_stats();
    const auto& ps = pipeline_creation_stats();
    const auto& ts = texture_lifecycle_stats();
    const auto& us = resource_upload_stats();
    const auto& rs = render_thread_stats();
    const auto& ss = submission_stats();
    const auto& gs = gpu_sync_stats();
    const auto& ms = gpu_memory_stats();
    const auto& pw = prewarm_stats();

    const double t = static_cast<double>(std::chrono::steady_clock::now().time_since_epoch().count()) * 1e-9;

    o << std::fixed << std::setprecision(3);
    o << g_csv_frame++ << "," << t << ",";
    o << ft.p50 << "," << ft.p95 << "," << ft.p99 << "," << ft.max << "," << ft.frames << ",";
    o << st.translations.load() << "," << st.total_ns.load() * 1e-6 << "," << st.max_ns.load() * 1e-6 << ","
      << st.patch_ns.load() * 1e-6 << "," << st.create_module_ns.load() * 1e-6 << ","
      << st.cache_hits.load() << "," << st.cache_misses.load() << ",";
    o << ps.graphics_created.load() << "," << ps.graphics_failed.load() << ","
      << ps.graphics_total_ns.load() * 1e-6 << "," << ps.graphics_max_ns.load() * 1e-6 << ","
      << ps.compute_created.load() << "," << ps.compute_failed.load() << ","
      << ps.compute_total_ns.load() * 1e-6 << "," << ps.compute_max_ns.load() * 1e-6 << ","
      << ps.cache_hits.load() << "," << ps.cache_misses.load() << ","
      << ps.linked_from_parts.load() << "," << ps.prewarm_taken.load() << ","
      << ps.prewarm_waited.load() << "," << ps.prewarm_wait_ns.load() * 1e-6 << ",";
    o << ts.created.load() << "," << ts.destroyed.load() << ","
      << ts.upload_bytes.load() * 1e-6 << "," << ts.upload_count.load() << ","
      << ts.upload_ns.load() * 1e-6 << "," << ts.transcoded.load() << ","
      << ts.transcoded_ns.load() * 1e-6 << "," << ts.residency_changes.load() << ","
      << ts.pool_hits.load() << "," << ts.pool_misses.load() << ","
      << ts.sparse_binds.load() << "," << ts.sparse_bind_ns.load() * 1e-6 << ",";
    o << us.staging_allocs.load() << "," << us.staging_bytes.load() * 1e-6 << ","
      << us.staging_ns.load() * 1e-6 << "," << us.copy_ns.load() * 1e-6 << ","
      << us.writeback_ns.load() * 1e-6 << "," << us.writeback_bytes.load() * 1e-6 << ","
      << us.ring_allocs.load() << "," << us.ring_fallbacks.load() << ",";
    o << rs.replay_ns.load() * 1e-6 << "," << rs.pipeline_ns.load() * 1e-6 << ","
      << rs.upload_ns.load() * 1e-6 << "," << rs.present_ns.load() * 1e-6 << ","
      << rs.acquire_ns.load() * 1e-6 << "," << rs.limiter_ns.load() * 1e-6 << ","
      << rs.idle_ns.load() * 1e-6 << "," << rs.gpu_wait_ns.load() * 1e-6 << ","
      << rs.submit_ns.load() * 1e-6 << "," << rs.lock_wait_ns.load() * 1e-6 << ","
      << rs.tex_prep_ns.load() * 1e-6 << "," << rs.tex_staging_ns.load() * 1e-6 << ","
      << rs.image_creation_ns.load() * 1e-6 << "," << rs.split_submit_ns.load() * 1e-6 << ","
      << rs.frame_cpu_ns.load() * 1e-6 << "," << rs.frames.load() << ",";
    o << ss.submits.load() << "," << ss.total_ns.load() * 1e-6 << ","
      << ss.max_ns.load() * 1e-6 << "," << ss.cmd_buffers.load() << ","
      << ss.wait_semaphores.load() << "," << ss.signal_semaphores.load() << ","
      << ss.batched_submits.load() << "," << ss.batch_flush_ns.load() * 1e-6 << ",";
    o << gs.timeline_waits.load() << "," << gs.timeline_wait_ns.load() * 1e-6 << ","
      << gs.fence_waits.load() << "," << gs.fence_wait_ns.load() * 1e-6 << ","
      << gs.present_waits.load() << "," << gs.present_wait_ns.load() * 1e-6 << ","
      << gs.acquire_waits.load() << "," << gs.acquire_wait_ns.load() * 1e-6 << ",";
    o << ms.device_local_bytes.load() * 1e-6 << ","
      << ms.host_visible_bytes.load() * 1e-6 << ","
      << ms.total_bytes.load() * 1e-6 << ","
      << ms.peak_bytes.load() * 1e-6 << ","
      << ms.allocation_count.load() << ","
      << ms.allocation_failures.load() << ",";
    o << pw.recorded_keys.load() << "," << pw.queued_keys.load() << ","
      << pw.built_ahead.load() << "," << pw.build_failures.load() << ","
      << pw.taken_by_draw.load() << "," << pw.waited_for.load() << ","
      << pw.wait_ns.load() * 1e-6 << "," << pw.linked_while_building.load() << ","
      << pw.claimed_by_draw.load() << "," << pw.skipped_busy.load() << ","
      << pw.promoted_keys.load() << "," << pw.queue_depth.load() << ","
      << pw.queue_depth_max.load() << "," << pw.workers_active.load() << ","
      << pw.metadata_lookups.load() << "," << pw.metadata_hits.load() << ","
      << pw.bounded_drops.load();
    return o.str();
}


// Gauge registration for soak sampler -------------------------------------------------------

int64_t gauge_frame_time_p50() { return static_cast<int64_t>(frame_time_p50() * 1000); }  // µs
int64_t gauge_frame_time_p95() { return static_cast<int64_t>(frame_time_p95() * 1000); }
int64_t gauge_frame_time_p99() { return static_cast<int64_t>(frame_time_p99() * 1000); }
int64_t gauge_frame_time_max() { return static_cast<int64_t>(frame_time_max() * 1000); }

void renderer_profiling_register_gauges() {
    if (g_gauges_registered) return;
    gauge_register("rp.frame_time_p50_us", gauge_frame_time_p50);
    gauge_register("rp.frame_time_p95_us", gauge_frame_time_p95);
    gauge_register("rp.frame_time_p99_us", gauge_frame_time_p99);
    gauge_register("rp.frame_time_max_us", gauge_frame_time_max);

    // Shader
    gauge_register("rp.shader_translations", [] { return static_cast<int64_t>(shader_translation_stats().translations.load()); });
    gauge_register("rp.shader_cache_hits", [] { return static_cast<int64_t>(shader_translation_stats().cache_hits.load()); });
    gauge_register("rp.shader_cache_misses", [] { return static_cast<int64_t>(shader_translation_stats().cache_misses.load()); });

    // Pipeline
    gauge_register("rp.pipeline_gfx_created", [] { return static_cast<int64_t>(pipeline_creation_stats().graphics_created.load()); });
    gauge_register("rp.pipeline_gfx_failed", [] { return static_cast<int64_t>(pipeline_creation_stats().graphics_failed.load()); });
    gauge_register("rp.pipeline_comp_created", [] { return static_cast<int64_t>(pipeline_creation_stats().compute_created.load()); });
    gauge_register("rp.pipeline_cache_hits", [] { return static_cast<int64_t>(pipeline_creation_stats().cache_hits.load()); });
    gauge_register("rp.pipeline_cache_misses", [] { return static_cast<int64_t>(pipeline_creation_stats().cache_misses.load()); });
    gauge_register("rp.pipeline_linked_parts", [] { return static_cast<int64_t>(pipeline_creation_stats().linked_from_parts.load()); });
    gauge_register("rp.pipeline_prewarm_taken", [] { return static_cast<int64_t>(pipeline_creation_stats().prewarm_taken.load()); });

    // Texture
    gauge_register("rp.texture_created", [] { return static_cast<int64_t>(texture_lifecycle_stats().created.load()); });
    gauge_register("rp.texture_destroyed", [] { return static_cast<int64_t>(texture_lifecycle_stats().destroyed.load()); });
    gauge_register("rp.texture_pool_hits", [] { return static_cast<int64_t>(texture_lifecycle_stats().pool_hits.load()); });
    gauge_register("rp.texture_pool_misses", [] { return static_cast<int64_t>(texture_lifecycle_stats().pool_misses.load()); });

    // Upload
    gauge_register("rp.upload_staging_mb", [] { return static_cast<int64_t>(resource_upload_stats().staging_bytes.load() >> 20); });
    // The staging ring's own live bytes are reported by the always-on staging_ring_kb / staging_own_kb hitch counters in
    // nvn_gpu.cpp (the ring lives in that translation unit's anonymous namespace).

    // Render thread
    gauge_register("rp.rt_replay_ms", [] { return static_cast<int64_t>(render_thread_stats().replay_ns.load() * 1e-6); });
    gauge_register("rp.rt_pipeline_ms", [] { return static_cast<int64_t>(render_thread_stats().pipeline_ns.load() * 1e-6); });
    gauge_register("rp.rt_upload_ms", [] { return static_cast<int64_t>(render_thread_stats().upload_ns.load() * 1e-6); });
    gauge_register("rp.rt_present_ms", [] { return static_cast<int64_t>(render_thread_stats().present_ns.load() * 1e-6); });
    gauge_register("rp.rt_idle_ms", [] { return static_cast<int64_t>(render_thread_stats().idle_ns.load() * 1e-6); });

    // GPU sync
    gauge_register("rp.sync_timeline_wait_ms", [] { return static_cast<int64_t>(gpu_sync_stats().timeline_wait_ns.load() * 1e-6); });
    gauge_register("rp.sync_fence_wait_ms", [] { return static_cast<int64_t>(gpu_sync_stats().fence_wait_ns.load() * 1e-6); });

    // GPU memory
    gauge_register("rp.gpu_dev_local_mb", [] { return static_cast<int64_t>(gpu_memory_stats().device_local_bytes.load() >> 20); });
    gauge_register("rp.gpu_host_vis_mb", [] { return static_cast<int64_t>(gpu_memory_stats().host_visible_bytes.load() >> 20); });
    gauge_register("rp.gpu_total_mb", [] { return static_cast<int64_t>(gpu_memory_stats().total_bytes.load() >> 20); });
    gauge_register("rp.gpu_peak_mb", [] { return static_cast<int64_t>(gpu_memory_stats().peak_bytes.load() >> 20); });

    // Prewarm
    gauge_register("rp.prewarm_recorded", [] { return static_cast<int64_t>(prewarm_stats().recorded_keys.load()); });
    gauge_register("rp.prewarm_queued", [] { return static_cast<int64_t>(prewarm_stats().queued_keys.load()); });
    gauge_register("rp.prewarm_built_ahead", [] { return static_cast<int64_t>(prewarm_stats().built_ahead.load()); });
    gauge_register("rp.prewarm_queue_depth", [] { return static_cast<int64_t>(prewarm_stats().queue_depth.load()); });
    gauge_register("rp.prewarm_bounded_drops", [] { return static_cast<int64_t>(prewarm_stats().bounded_drops.load()); });

    g_gauges_registered = true;
}

// Periodic CSV write (called from soak sampler or render thread periodically).
void renderer_profiling_periodic_csv() {
    if (!g_enabled || !g_csv_file.is_open()) return;
    std::lock_guard<std::mutex> lk(g_csv_mu);
    write_csv_row_locked();
}

}  // namespace nxrt::rp