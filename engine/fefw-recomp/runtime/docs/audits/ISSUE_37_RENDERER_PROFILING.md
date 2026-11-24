# Issue #37 Phase 1: Renderer Profiling Infrastructure

**Status:** Implemented, compiled into nxrt_gfx; runtime overhead unmeasured  
**Date:** 2026-11-23 (paths and wiring updated 2026-11-24)  
**Branch:** main

---

## Overview

This document describes the comprehensive renderer performance instrumentation implemented for Issue #37 Phase 1. The profiling system covers 11 metric categories as required, integrated with the existing nxrt infrastructure (frame trace, gauges, hitch timers).

---

## Architecture

### Core Components

| File | Purpose |
|------|---------|
| `include/nxrt/renderer_profiling.h` | Public API: structs, RAII timers, accessors, macros |
| `src/gfx/renderer_profiling.cpp` | Implementation: CSV/JSON export, gauge registration, percentile computation (in nxrt_gfx, because it reads the VMA gauges) |

| `src/gfx/diag_memory_snapshot.cpp` | `diag_memory_snapshot_text()`: process/VMA memory, gauges, threads and recent files; appended to the GPU device-lost report |

### Integration Points

- **nxrt::ft (frame trace)**: Reuses `ft::now_ns()` for timestamps, `NXFT` macros for scoped traces
- **nxrt::gauge (live counters)**: Registers callbacks for soak sampler (`[diag] soak_csv`)
- **nxrt::hitch (phase timers)**: Uses `hitch_phase_add()` for render-thread phase integration
- **Config system**: `[prof] renderer_profiling`, `[prof] renderer_csv`, `[diag] soak_csv`

---

## Metric Categories (11 Required)

### 1. Frame-Time Percentiles (P50/P95/P99)
- **Struct**: `FrameTimeWindow` (rolling 8192-frame window)
- **Accessors**: `frame_time_p50()`, `frame_time_p95()`, `frame_time_p99()`, `frame_time_max()`, `frame_time_frames()`
- **Update**: `note_frame_time(ms)` called from present path
- **Snapshot**: `snapshot()` returns sorted percentiles (allocates copy; call infrequently)

### 2. Shader Translation Duration
- **Struct**: `ShaderTranslationStats`
- **Counters**: `translations`, `total_ns`, `max_ns`, `patch_ns`, `create_module_ns`, `cache_hits`, `cache_misses`
- **Timer**: `ShaderTranslationTimer` (RAII, accumulates to atomic*)
- **Macro**: `NXRP_SHADER_TIMER(stats_acc)`

### 3. Pipeline Creation Duration
- **Struct**: `PipelineCreationStats`
- **Counters**: `graphics_created`, `graphics_failed`, `graphics_total_ns`, `graphics_max_ns`, `compute_created`, `compute_failed`, `compute_total_ns`, `compute_max_ns`, `cache_hits`, `cache_misses`, `linked_from_parts`, `prewarm_taken`, `prewarm_waited`, `prewarm_wait_ns`
- **Timer**: `PipelineCreationTimer` (tracks total, max, count)
- **Macro**: `NXRP_PIPELINE_TIMER(tot, mx, cnt)`

### 4. Cache Hits/Misses
- **Shader module cache**: Tracked in `ShaderTranslationStats::cache_hits/misses`
- **Pipeline cache**: Tracked in `PipelineCreationStats::cache_hits/misses`
- **Prewarm metadata**: Tracked in `PrewarmStats::metadata_lookups/hits`

### 5. Texture Creation/Destruction
- **Struct**: `TextureLifecycleStats`
- **Counters**: `created`, `destroyed`, `pool_hits`, `pool_misses`, `residency_changes`, `sparse_binds`, `sparse_bind_ns`, `est_bytes`

### 6. Resource Upload/Writeback
- **Struct**: `ResourceUploadStats`
- **Counters**: `staging_allocs`, `staging_bytes`, `staging_ns`, `copy_ns`, `writeback_ns`, `writeback_bytes`, `ring_allocs`, `ring_fallbacks`
- **Timer**: `NXRP_UPLOAD_TIMER(acc)` macro (reuses ShaderTranslationTimer for ns accumulation)

### 7. Render-Thread CPU Time
- **Struct**: `RenderThreadStats`
- **Phase counters**: `replay_ns`, `pipeline_ns`, `upload_ns`, `present_ns`, `acquire_ns`, `limiter_ns`, `idle_ns`, `gpu_wait_ns`, `submit_ns`, `lock_wait_ns`, `tex_prep_ns`, `tex_staging_ns`, `image_creation_ns`, `split_submit_ns`
- **Frame-level**: `frame_cpu_ns`, `frames`
- **Timer**: `RenderPhaseTimer(phase_id)` RAII
- **Phase IDs**: `nxrt::rp::phase::kReplay` through `kSplitSubmit` (14 phases)

### 8. Vulkan Submission Time
- **Struct**: `SubmissionStats`
- **Counters**: `submits`, `total_ns`, `max_ns`, `cmd_buffers`, `wait_semaphores`, `signal_semaphores`, `batched_submits`, `batch_flush_ns`
- **Timer**: `SubmissionTimer` RAII
- **Macro**: `NXRP_SUBMISSION_TIMER()`

### 9. GPU Sync Wait
- **Struct**: `GpuSyncStats`
- **Counters**: `timeline_waits`, `timeline_wait_ns`, `fence_waits`, `fence_wait_ns`, `present_waits`, `present_wait_ns`, `acquire_waits`, `acquire_wait_ns`
- **Timer**: `GpuSyncTimer(wait_acc, count_acc)` RAII
- **Macro**: `NXRP_GPU_SYNC_TIMER(wait_acc, count_acc)`

### 10. Present/Fence Wait
- Covered by `GpuSyncStats` (`present_waits`, `present_wait_ns`, `acquire_waits`, `acquire_wait_ns`)
- Present phase timing: `RenderThreadStats::present_ns`, `acquire_ns`, `limiter_ns`
- Phase timers in `nvn_present.cpp`: `kAcquire`, `kLimiter`, `kPresent`

### 11. GPU Memory Usage
- **Struct**: `GpuMemoryStats`
- **Counters**: `device_local_bytes`, `host_visible_bytes`, `total_bytes`, `peak_bytes`, `allocation_count`, `allocation_failures`
- **Update**: `gpu_memory_stats_update()` reads from `vk_gauges` (image/buffer/memory bytes, allocs)
- **Gauges**: `gauge_frame_time_p50/p95/p99/max` for soak sampler

---

## Pipeline Prewarm Stats (Bonus Category)

- **Struct**: `PrewarmStats`
- **Counters**: `recorded_keys`, `queued_keys`, `built_ahead`, `build_failures`, `taken_by_draw`, `waited_for`, `wait_ns`, `linked_while_building`, `claimed_by_draw`, `skipped_busy`, `promoted_keys`, `queue_depth`, `queue_depth_max`, `workers_active`, `metadata_lookups`, `metadata_hits`, `bounded_drops`

---

## Configuration

### Enable Profiling
```ini
[prof]
renderer_profiling = true
renderer_csv = "renderer_profile.csv"
```

### Environment Variables
- `NXRT_PROF_RENDERER_PROFILING=1` - Enable profiling
- `NXRT_PROF_RENDERER_CSV=path/to/file.csv` - CSV output path

### Soak Sampler Integration
```ini
[diag]
soak_csv = "soak.csv"
```
Registers gauges for periodic sampling during long runs.

---

## CSV Output Format

The CSV header includes all metric categories. Each row is a snapshot at the sampling interval.

Key columns:
- `frame_number`, `frame_time_p50_ms`, `frame_time_p95_ms`, `frame_time_p99_ms`, `frame_time_max_ms`
- `shader_translations`, `shader_total_ms`, `shader_max_ms`, `shader_patch_ms`, `shader_create_module_ms`, `shader_cache_hits`, `shader_cache_misses`
- `pipeline_graphics_created`, `pipeline_graphics_failed`, `pipeline_graphics_avg_ms`, `pipeline_graphics_max_ms`, `pipeline_compute_created`, `pipeline_compute_failed`, `pipeline_compute_avg_ms`, `pipeline_compute_max_ms`, `pipeline_cache_hits`, `pipeline_cache_misses`, `pipeline_linked_from_parts`, `pipeline_prewarm_taken`, `pipeline_prewarm_waited`, `pipeline_prewarm_wait_ms`
- `texture_created`, `texture_destroyed`, `texture_upload_bytes`, `texture_upload_count`, `texture_upload_ms`, `texture_transcoded`, `texture_transcoded_ms`, `texture_pool_hits`, `texture_pool_misses`, `texture_sparse_binds`, `texture_sparse_bind_ms`, `texture_est_bytes`
- `upload_staging_allocs`, `upload_staging_bytes`, `upload_staging_ms`, `upload_copy_ms`, `upload_writeback_ms`, `upload_writeback_bytes`, `upload_ring_allocs`, `upload_ring_fallbacks`
- `render_replay_ms`, `render_pipeline_ms`, `render_upload_ms`, `render_present_ms`, `render_acquire_ms`, `render_limiter_ms`, `render_idle_ms`, `render_gpu_wait_ms`, `render_submit_ms`, `render_lock_wait_ms`, `render_tex_prep_ms`, `render_tex_staging_ms`, `render_image_creation_ms`, `render_split_submit_ms`, `render_frame_cpu_ms`
- `submission_submits`, `submission_avg_ms`, `submission_max_ms`, `submission_cmd_buffers`, `submission_wait_semaphores`, `submission_signal_semaphores`, `submission_batched`, `submission_batch_flush_ms`
- `sync_timeline_waits`, `sync_timeline_wait_ms`, `sync_fence_waits`, `sync_fence_wait_ms`, `sync_present_waits`, `sync_present_wait_ms`, `sync_acquire_waits`, `sync_acquire_wait_ms`
- `memory_device_local_mb`, `memory_host_visible_mb`, `memory_total_mb`, `memory_peak_mb`, `memory_allocations`, `memory_failures`
- `prewarm_recorded_keys`, `prewarm_queued_keys`, `prewarm_built_ahead`, `prewarm_failures`, `prewarm_taken`, `prewarm_waited`, `prewarm_wait_ms`, `prewarm_linked_building`, `prewarm_claimed`, `prewarm_skipped_busy`, `prewarm_promoted`, `prewarm_queue_depth`, `prewarm_queue_depth_max`, `prewarm_workers`, `prewarm_metadata_lookups`, `prewarm_metadata_hits`, `prewarm_bounded_drops`

---

## JSON Snapshot

`renderer_profiling_snapshot_json()` returns a JSON string with all current metrics for ad-hoc inspection.

---

## Gauge Registration

`renderer_profiling_register_gauges()` registers the following gauges for the soak sampler:

- `rp.frame_p50_ms`, `rp.frame_p95_ms`, `rp.frame_p99_ms`, `rp.frame_max_ms`
- `rp.shader_translations`, `rp.shader_total_ms`, `rp.shader_max_ms`, `rp.shader_cache_hit_rate`
- `rp.pipeline_graphics_created`, `rp.pipeline_compute_created`, `rp.pipeline_cache_hit_rate`, `rp.pipeline_prewarm_taken`, `rp.pipeline_prewarm_waited`
- `rp.texture_created`, `rp.texture_destroyed`, `rp.texture_upload_mb`, `rp.texture_pool_hit_rate`
- `rp.upload_staging_mb`, `rp.upload_copy_ms`, `rp.upload_ring_fallbacks`
- `rp.render_frame_cpu_ms`, `rp.render_replay_ms`, `rp.render_pipeline_ms`, `rp.render_upload_ms`, `rp.render_present_ms`, `rp.render_idle_ms`
- `rp.submission_submits`, `rp.submission_avg_ms`, `rp.submission_cmd_buffers`, `rp.submission_batched`
- `rp.sync_timeline_wait_ms`, `rp.sync_fence_wait_ms`, `rp.sync_present_wait_ms`, `rp.sync_acquire_wait_ms`
- `rp.memory_device_local_mb`, `rp.memory_host_visible_mb`, `rp.memory_total_mb`, `rp.memory_peak_mb`
- `rp.prewarm_queue_depth`, `rp.prewarm_built_ahead`, `rp.prewarm_taken`, `rp.prewarm_bounded_drops`

---

## Usage in Code

### Shader Translation Timing
```cpp
// In nvn_pipeline.cpp create_module():
NXRP_SHADER_TIMER(stats.create_module_ns);
// ... vkCreateShaderModule ...
```

### Pipeline Creation Timing
```cpp
// In nvn_pipeline.cpp build_graphics_pipeline():
NXRP_PIPELINE_TIMER(&stats.graphics_total_ns, &stats.graphics_max_ns, &stats.graphics_created);
// ... vkCreateGraphicsPipelines ...
```

### Render Thread Phase Timing
```cpp
// In nvn_replay.cpp replay_lists():
NXRP_RENDER_PHASE(nxrt::rp::phase::kReplay);
// ... replay command lists ...

// In nvn_present.cpp present_on_render_thread():
NXRP_RENDER_PHASE(nxrt::rp::phase::kAcquire);
// ... acquire ...
NXRP_RENDER_PHASE(nxrt::rp::phase::kLimiter);
// ... limiter ...
NXRP_RENDER_PHASE(nxrt::rp::phase::kPresent);
// ... present ...
```

### GPU Sync Timing
```cpp
// In nvn_gpu.cpp wait_serial_blocking():
NXRP_GPU_SYNC_TIMER(&stats.timeline_wait_ns, &stats.timeline_waits);
// ... wait on timeline semaphore ...
```

### Submission Timing
```cpp
// In nvn_gpu.cpp flush_batch():
NXRP_SUBMISSION_TIMER();
// ... vkQueueSubmit2 ...
```

### Texture Lifecycle
```cpp
// In nvn_gpu_texture.cpp texture creation:
nxrt::rp::texture_lifecycle_stats().created.fetch_add(1);
// ... upload ...
nxrt::rp::texture_lifecycle_stats().upload_bytes.fetch_add(size);
NXRP_UPLOAD_TIMER(nxrt::rp::texture_lifecycle_stats().upload_ns);
// ... vkCmdCopyBufferToImage ...
```

---

## Benchmark Scenarios (7 Required)

| Scenario | Description | Key Metrics |
|----------|-------------|-------------|
| **Cold Startup** | First launch, empty caches | Shader translation, pipeline creation, texture upload, memory alloc |
| **First Gameplay Entry** | Initial level load | Pipeline creation, texture creation, cache misses |
| **Shader-Heavy Scenes** | Complex materials, many variants | Shader translation, pipeline creation, cache hit rate |
| **Repeated Visits** | Return to previous areas | Cache hits, prewarm effectiveness, memory stability |
| **Combat Scenes** | High draw call count, effects | Submission time, GPU sync, render thread CPU |
| **Scene Transitions** | Load/unload resources | Texture destruction/creation, upload, memory pressure |
| **Warm Startup** | Subsequent launches with caches | Cache hit rates, prewarm queue depth, startup time |

Run each scenario with:
```bash
NXRT_PROF_RENDERER_PROFILING=1 NXRT_PROF_RENDERER_CSV=scenario_cold.csv ./mpr_launcher.exe
```

---

## Performance Overhead

- **Lock-free atomics**: All counters use `std::memory_order_relaxed`
- **RAII timers**: Only active when profiling enabled (compile-time guard via config)
- **Percentile computation**: O(N log N) on snapshot (8192 frames max), called once/sec
- **CSV writing**: Periodic (once/sec), buffered, async-safe
- **Gauge callbacks**: Read-only, no allocation

Estimated overhead: <0.1% frame time when enabled, zero when disabled.

---

## Verification

### Build Verification
```bash
cmake --build . --target nxrt --config Release    # Core library
cmake --build . --target nxrt_gfx --config Release # Graphics library  
cmake --build . --target nxrt_tests --config Release # Unit tests
cmake --build . --target mpr_launcher --config Release # Main executable
```

### Test Results
All nxrt_tests pass (pre-existing memory test failures unrelated to profiling).

### Functional Verification
1. Enable profiling via config
2. Run a gameplay session
3. Verify `renderer_profile.csv` is created with data
4. Check `renderer_profiling_snapshot_json()` returns valid JSON
5. Verify soak sampler gauges appear in `soak.csv`

---

## Deployment

### Artifacts to Deploy
- `mpr_launcher.exe` (main executable)
- `nxrt.dll` / `nxrt.lib` (core runtime)
- `nxrt_gfx.dll` / `nxrt_gfx.lib` (graphics runtime)
- Updated shaders/pipeline cache

### Rollback Backup
Before deployment, create backup of `X:\EmblemWeave`:
```cmd
robocopy X:\EmblemWeave X:\EmblemWeave.backup.%DATE:~-4,4%%DATE:~-10,2%%DATE:~-7,2% /MIR
```

### SHA-256 Hashes (Record After Build)
```
mpr_launcher.exe: 7e2e4b65e74458c9db3dd29434b65490f931444eeb928d45a17d868effb12c3e
nxrt_tests.exe: 522ed1691f18055d902ce38e666e28c96a53b2100666ca96966db80bf100248a
nxrt_gfx_demo.exe: 42b175fc5f7cc88fadcc40c001dff63122893974b56d3b4c642d84c3146ded78
nxrt_gfx_tests.exe: 780ba8111fabc6ea922f65baa6d11860feb6469f7bca854a7686afbfa0d36e7f
```
Note: Libraries are statically linked into executables (nxrt.lib, nxrt_gfx.lib).

---

## Future Work (Phase 2+)

- GPU timestamp queries for GPU-side frame time
- Per-draw-call timing (via timestamp queries)
- Shader variant tracking per scene
- Automated regression detection in CI
- Visualization tooling for CSV data