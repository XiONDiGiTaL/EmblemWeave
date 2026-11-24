# Issue #37 Phase 1: Bounded Pipeline Prewarming System

**Status:** Implemented; bounded queue and metadata priority compiled, effect unmeasured  
**Date:** 2026-11-23 (status updated 2026-11-24)  
**Branch:** main

> **Note (2026-11-24, Freebuff consolidation).** The bounded queue (`[nvn] prewarm_max_queue`), metadata priority
> (`prewarm_metadata_priority`) and `prewarm_update_frame()` are code-complete and on `main`. No benchmark shows that
> the priority order improves first-visit stalls; that requires game content and a frame-time comparison. The old
> `prewarm_workers` key in earlier drafts is `prewarm_max_workers` in the code.

---

## Overview

This document describes the bounded pipeline prewarming system implemented for Issue #37 Phase 1. The system uses available pipeline metadata and previously encountered variants to pre-build pipelines ahead of draw-time demand, eliminating first-use compilation stalls.

---

## Architecture

### Core Components

| File | Purpose |
|------|---------|
| `src/gfx/nvn/nvn_prewarm.cpp` | Prewarm queue, workers, priority scheduling, metadata tracking |
| `src/gfx/nvn/nvn_pipeline.h` | API declarations: `prewarm_note_key_request`, `prewarm_update_frame` |
| `src/gfx/nvn/nvn_pipeline.cpp` | Integration: cache hit/miss tracking, prewarm counters, key requests |
| `src/gfx/nvn/nvn_present.cpp` | Frame counter update: `prewarm_update_frame()` called per frame |
| `include/nxrt/renderer_profiling.h` | `PrewarmStats` struct for telemetry |

### Design Principles

1. **Bounded Queue**: Fixed maximum size (default 5000 entries) prevents unbounded memory growth
2. **Metadata-Driven Priority**: Keys ranked by recency × frequency score, re-sorted each frame
3. **Worker Pool**: Configurable worker threads (default 2) build pipelines off render thread
4. **Draw-Time Feedback**: `prewarm_note_key_request(key)` called at draw time updates metadata
5. **Frame-Based Decay**: Priority scores decay each frame (`recency_decay=0.99`)
6. **Graceful Degradation**: If queue full, lowest-priority entries dropped (`bounded_drops` counter)

---

## Data Structures

### KeyMetadata
```cpp
struct KeyMetadata {
    uint64_t frequency = 0;           // How many frames this key was requested
    uint64_t last_request_frame = 0;  // Last frame this key was requested
    uint64_t first_seen_frame = 0;    // First frame this key was seen
    double priority_score = 0.0;      // Computed: frequency * recency_decay^(current_frame - last_request)
};
```

### PrewarmQueueConfig
```cpp
struct PrewarmQueueConfig {
    size_t max_queue_size = 5000;     // Hard bound on queue entries
    int max_workers = 2;              // Worker thread count
    bool metadata_priority = true;    // Use metadata for priority (vs FIFO)
    double recency_decay = 0.99;      // Per-frame priority decay factor
};
```

### PrewarmQueueEntry
```cpp
struct PrewarmQueueEntry {
    PipelineKey key;                  // The pipeline key to build
    KeyMetadata metadata;             // Priority metadata
    std::atomic<bool> building{false}; // Currently being built by worker
    std::atomic<bool> ready{false};   // Build complete, ready for draw
    VkPipeline pipeline = VK_NULL_HANDLE; // Result pipeline (if successful)
    VkResult result = VK_NOT_READY;   // Build result
};
```

---

## Algorithm

### Priority Computation
```
priority_score = frequency * (recency_decay ^ (current_frame - last_request_frame))
```

- **Frequency**: Count of frames where key was requested by draw calls
- **Recency**: Exponential decay based on frames since last request
- **Result**: Recently used, frequently requested keys bubble to top

### Frame Update (`prewarm_update_frame`)
Called once per presented frame from `nvn_present.cpp`:

1. Increment global frame counter
2. Recompute `priority_score` for all queued entries
3. Sort queue by `priority_score` descending
4. Enforce `max_queue_size`: drop lowest-priority entries, increment `bounded_drops`
5. Signal workers to process top entries

### Draw-Time Feedback (`prewarm_note_key_request`)
Called from `nvn_pipeline.cpp` when a draw needs a pipeline:

1. Look up or create `KeyMetadata` for the key
2. Increment `frequency`
3. Update `last_request_frame = current_frame`
4. If first time seen, set `first_seen_frame = current_frame`
5. Increment `metadata_lookups`; if entry existed, increment `metadata_hits`
6. If key not in prewarm queue and queue not full, add to queue with initial priority

### Worker Processing
Each worker thread:
1. Pops highest-priority non-building entry from queue
2. Marks `building = true`
3. Builds pipeline (graphics or compute) via Vulkan API
4. On success: stores `pipeline`, sets `ready = true`, increments `built_ahead`
5. On failure: sets `result`, increments `build_failures`
6. Clears `building`

### Draw-Time Consumption
When draw thread needs a pipeline:
1. Check main pipeline cache first (fast path)
2. If miss, check prewarm queue for `ready` entry
3. If found: take pipeline, increment `prewarm_taken`, remove from queue
4. If building: wait (with timeout), increment `prewarm_waited` and `prewarm_wait_ns`
5. If not in queue: build synchronously (fallback), increment `cache_misses`

---

## Configuration

### Enable Prewarming
```ini
[nvn]
prewarm_enabled = true
prewarm_max_queue = 5000
prewarm_workers = 2
prewarm_recency_decay = 0.99
```

### Environment Variables
- `NXRT_NVN_PREWARM_ENABLED=1`
- `NXRT_NVN_PREWARM_MAX_QUEUE=5000`
- `NXRT_NVN_PREWARM_WORKERS=2`
- `NXRT_NVN_PREWARM_RECENCY_DECAY=0.99`

---

## Telemetry (PrewarmStats)

All counters are `std::atomic<uint64_t>` with relaxed ordering:

| Counter | Description |
|---------|-------------|
| `recorded_keys` | Total unique keys ever requested |
| `queued_keys` | Keys currently in prewarm queue |
| `built_ahead` | Pipelines successfully built by workers |
| `build_failures` | Pipeline build failures in workers |
| `taken_by_draw` | Prewarmed pipelines consumed by draws |
| `waited_for` | Draws that waited for in-progress build |
| `wait_ns` | Total nanoseconds draws spent waiting |
| `linked_while_building` | Fast-links from prebuilt parts during build |
| `claimed_by_draw` | Draw claimed a building entry |
| `skipped_busy` | Draw skipped prewarm (all workers busy) |
| `promoted_keys` | Keys promoted in priority |
| `queue_depth` | Current queue size |
| `queue_depth_max` | Peak queue size |
| `workers_active` | Workers currently building |
| `metadata_lookups` | `prewarm_note_key_request` calls |
| `metadata_hits` | Key already had metadata |
| `bounded_drops` | Entries dropped due to queue bound |

---

## Integration Points

### nvn_pipeline.cpp
```cpp
// In shader_module(): track cache hits/misses
if (module_found) {
    stats.cache_hits++;
} else {
    stats.cache_misses++;
    NXRP_SHADER_TIMER(stats.create_module_ns);
    // ... create module ...
}

// In build_graphics_pipeline():
NXRP_PIPELINE_TIMER(&stats.graphics_total_ns, &stats.graphics_max_ns, &stats.graphics_created);
if (pipeline_found) {
    stats.cache_hits++;
} else {
    stats.cache_misses++;
    // ... create pipeline ...
    if (from_prewarm) stats.prewarm_taken++;
    if (waited_for_prewarm) { stats.prewarm_waited++; stats.prewarm_wait_ns += wait_time; }
}
if (fast_linked) stats.linked_from_parts++;

// Request prewarm for this key at draw time
prewarm_note_key_request(key);
```

### nvn_present.cpp
```cpp
// In present_on_render_thread(), once per frame:
prewarm_update_frame(frame_number);
```

### nvn_prewarm.cpp
```cpp
// Worker thread function:
void prewarm_worker() {
    while (!shutdown) {
        auto entry = pop_highest_priority();
        if (!entry) { wait_for_work(); continue; }
        
        entry->building = true;
        workers_active++;
        
        VkResult res = build_pipeline(entry->key, &entry->pipeline);
        entry->result = res;
        entry->ready = true;
        entry->building = false;
        workers_active--;
        
        if (res == VK_SUCCESS) built_ahead++;
        else build_failures++;
    }
}
```

---

## Bounded Queue Enforcement

The queue bound is enforced in `prewarm_update_frame()`:

```cpp
void enforce_queue_bound() {
    if (queue.size() > config.max_queue_size) {
        // Sort by priority (already sorted)
        size_t to_drop = queue.size() - config.max_queue_size;
        for (size_t i = 0; i < to_drop; ++i) {
            queue.pop_back();  // Lowest priority at back
            stats.bounded_drops++;
        }
        stats.queue_depth = queue.size();
    }
}
```

This ensures:
- Memory usage is predictable: `max_queue_size * sizeof(PrewarmQueueEntry)`
- Old/unused keys are evicted automatically
- `bounded_drops` telemetry alerts if bound is too aggressive

---

## Benchmark Scenarios (7 Required)

| Scenario | Expected Prewarm Behavior | Key Metrics |
|----------|---------------------------|-------------|
| **Cold Startup** | Empty queue, metadata cold | `recorded_keys` ramp, `queued_keys` growth, `build_failures`=0 |
| **First Gameplay Entry** | Keys from startup reused | `metadata_hits` high, `prewarm_taken` > 0, `queue_depth` stable |
| **Shader-Heavy Scenes** | Many new variants | `recorded_keys` spike, `built_ahead` ramps, `bounded_drops` if queue small |
| **Repeated Visits** | High metadata hit rate | `metadata_hits` ≈ `metadata_lookups`, `prewarm_taken` high, `waited_for` low |
| **Combat Scenes** | Burst of new keys | `queued_keys` spike, `workers_active` = max_workers, `skipped_busy` if saturated |
| **Scene Transitions** | Old keys decay, new keys rise | `promoted_keys`, `queue_depth` stable, `bounded_drops` for stale keys |
| **Warm Startup** | Metadata persists across runs | `recorded_keys` high immediately, `prewarm_taken` from frame 1 |

---

## Performance Characteristics

### Memory Overhead
- `KeyMetadata`: ~32 bytes per unique key
- `PrewarmQueueEntry`: ~64 bytes + pipeline handle
- At `max_queue_size=5000`: ~500 KB base + pipeline handles
- Pipeline objects: GPU driver memory (not counted in queue bound)

### CPU Overhead
- **Render thread**: `prewarm_note_key_request()` = hashmap lookup + atomic increments (~50ns)
- **Present thread**: `prewarm_update_frame()` = O(N log N) sort once/frame (N ≤ 5000, ~0.1ms)
- **Worker threads**: Full pipeline creation cost (moved off render thread)

### Latency Hiding
- Ideal: Pipeline ready before draw needs it (`prewarm_taken` path)
- Acceptable: Draw waits briefly for in-progress build (`prewarm_waited` < 1ms)
- Fallback: Synchronous build (`cache_misses` path, same as no prewarm)

---

## Tuning Guidelines

### Queue Size
- **Too small** (`<1000`): High `bounded_drops`, thrashing, low `prewarm_taken`
- **Too large** (`>10000`): Memory pressure, longer sort times, diminishing returns
- **Sweet spot**: 3000-5000 for typical Switch titles

### Worker Count
- **1**: Minimal thread overhead, may not keep up with burst scenes
- **2** (default): Good balance, matches typical CPU core allocation
- **4+**: Diminishing returns, Vulkan driver may serialize internally

### Recency Decay
- **0.95**: Fast decay, adapts quickly to scene changes, forgets old keys fast
- **0.99** (default): Balanced, retains keys for ~100 frames (1.6s at 60fps)
- **0.999**: Slow decay, keeps keys for ~1000 frames (16s), may crowd out new keys

---

## Verification

### Build Verification
```bash
cmake --build . --target nxrt_gfx --config Release
cmake --build . --target mpr_launcher --config Release
```

### Functional Test
1. Enable prewarm: `[nvn] prewarm_enabled = true`
2. Run gameplay session with scene transitions
3. Verify `prewarm_stats().built_ahead > 0`
4. Verify `prewarm_stats().taken_by_draw > 0`
5. Verify `prewarm_stats().bounded_drops` reasonable (<5% of `queued_keys`)

### CSV Output
Prewarm metrics included in `renderer_profile.csv` (see ISSUE_37_RENDERER_PROFILING.md).

---

## Deployment

### Artifacts
- `nxrt_gfx.dll` / `nxrt_gfx.lib` (contains prewarm implementation)
- `mpr_launcher.exe` (links nxrt_gfx)

### Rollback
Backup `X:\EmblemWeave` before deployment.

### SHA-256 Hashes (Record After Build)
```
mpr_launcher.exe: 7e2e4b65e74458c9db3dd29434b65490f931444eeb928d45a17d868effb12c3e
nxrt_gfx_demo.exe: 42b175fc5f7cc88fadcc40c001dff63122893974b56d3b4c642d84c3146ded78
nxrt_gfx_tests.exe: 780ba8111fabc6ea922f65baa6d11860feb6469f7bca854a7686afbfa0d36e7f
```
Note: Libraries are statically linked into executables (nxrt.lib, nxrt_gfx.lib).

---

## Future Enhancements (Phase 2+)

1. **Persistent Metadata**: Save/load `KeyMetadata` to disk across runs
2. **Shader-Level Prewarm**: Pre-translate SPIR-V for predicted keys
3. **Adaptive Worker Count**: Scale workers based on queue depth
4. **Priority Inheritance**: Boost priority of keys needed by in-flight frames
5. **Clustered Builds**: Batch similar keys (same shader, different state) for driver efficiency
6. **Telemetry Dashboard**: Real-time prewarm queue visualization