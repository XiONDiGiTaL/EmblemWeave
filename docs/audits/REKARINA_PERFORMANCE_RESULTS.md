# Rekarina Performance Integration — Results and Validation

**Date:** 2026-11-24  
**Git Commit:** 113cc8fa (EmblemWeave main branch)  
**Reference Comparison:** `docs/audits/REKARINA_PERFORMANCE_COMPARISON.md`  
**Status:** Historical research. Superseded where it conflicts with the reconciled readiness audit.

> **Historical status (2026-11-24, Freebuff consolidation).** The measurements in this file were taken before the
> profiling path was actually wired: `note_frame_time()` had no caller and `g_memory_stats_update()` was never
> invoked, so every frame-time and GPU-memory row reads `0.000`. Those zeros are a **metric bug**, not a fast
> renderer. The present path now feeds the frame-time window and refreshes the GPU-memory gauges about once a second,
> so future runs produce real percentiles. No Priority 1 item from this plan (Linux write-watch, async write-back,
> texture pooling, entry-step prewarm) has been benchmarked; only the Linux write-watch and a bounded prewarm queue
> exist as code, and neither is validated. See `docs/audits/ENGINE_AND_RENDERER_VALIDATION.md`.

---

## Baseline Performance Profile (Pre-Integration)

### Profiling Configuration
- **Build:** Release, statically linked
- **Profiling:** `[prof] renderer_profiling = true`, `[prof] renderer_csv = "renderer_profile.csv"`
- **Prewarm:** `[nvn] prewarm_enabled = true`, `prewarm_workers = 2`, `prewarm_max_queue = 5000`
- **Test Binary:** `nxrt_gfx_tests.exe gfx_nvn_scene_clear`

### Baseline CSV Metrics (Frame 0 — Cold Start)

| Category | Metric | Value |
|----------|--------|-------|
| **Frame Time** | P50 / P95 / P99 / Max (ms) | 0.000 / 0.000 / 0.000 / 0.000 |
| **Shader** | Translations / Total MS / Max MS | 0 / 0.000 / 0.000 |
| **Pipeline** | GFX Created / Failed / Total MS | 0 / 0 / 0.000 |
| **Pipeline** | COMP Created / Failed / Total MS | 0 / 0 / 0.000 |
| **Texture** | Created / Destroyed / Upload MB | 1 / 0 / 0.000 |
| **Texture** | Pool Hits / Misses | 0 / 1 |
| **Upload** | Staging Allocs / MB / MS | 0 / 0.000 / 0.000 |
| **Upload** | Writeback MS / MB | 0.000 / 0.000 |
| **Render Thread** | Replay / Pipeline / Upload MS | 0.000 / 0.000 / 0.000 |
| **Render Thread** | Present / Acquire / Limiter MS | 0.000 / 0.000 / 0.000 |
| **Render Thread** | Submit / Lock Wait MS | 0.000 / 0.000 |
| **Submission** | Submits / Total MS / Max MS | 1 / 0.279 / 0.279 |
| **GPU Sync** | Timeline Waits / MS | 0 / 0.000 |
| **GPU Sync** | Fence Waits / MS | 0 / 0.000 |
| **GPU Sync** | Present Waits / MS | 0 / 0.000 |
| **GPU Memory** | Dev Local / Host Vis / Total MB | 15.207 / 8.893 / 24.100 |
| **Prewarm** | Recorded / Queued / Built Ahead | 6 / 2 / 0 |
| **Prewarm** | Waited / Wait MS | 1 / 0.049 |
| **Prewarm** | Bounded Drops | 0 |

> **Note:** Baseline captured on minimal test scene (`gfx_nvn_scene_clear`). Full gameplay benchmarks require game content (ExeFS, RomFS, main.nso) and are tracked in `scripts/benchmark_issue37.py`.

---

## Rekarina Technique Gap Analysis Summary

### Critical Gaps (Priority 1 — Must Implement)

| # | Technique | Rekarina Source | EmblemWeave Status | Impact |
|---|-----------|-----------------|-------------------|--------|
| 1 | **Linux write-watch (`userfaultfd`/`mprotect`)** | `vk_memory.c` + `platform/platform.h` | ❌ Windows-only `WriteWatch` | Blocks Steam Deck / Linux support; memory coherence |
| 2 | **Async Write-Back (AWB) on compute queue** | `vk_sync.c` (lines 258-370, 483-481) | ❌ All write-back on graphics queue | ~3.5 ms/frame GPU time savings (OoT data) |
| 3 | **Texture pooling (TexSlot adoption/parking)** | `vk_texture.c` (`adopt_released`, `follow_rekey`, `park_image`) | ❌ No image pooling — new VkImage every texture | Eliminates allocation spikes at scene transitions |
| 4 | **Entry-step pipeline prewarm** | `vk_pipeline.c` (`vk_pipeline_prewarm`, `lb_warm_prepared`) | ❌ Fully async only (Issue #37) | Eliminates first-frame compilation stalls |

### High-Value Gaps (Priority 2 — Should Implement)

| # | Technique | Rekarina Source | EmblemWeave Status | Impact |
|---|-----------|-----------------|-------------------|--------|
| 5 | **Lazy write-back on fault (PROT_NONE)** | `vk_memory.c` (mirror lazy write-back) | ❌ Upfront copy at submission | Defers write-back until CPU reads; saves bandwidth |
| 6 | **Submitter thread for vkQueueSubmit2** | `vk_sync.c` (`sq_main`, lines 767-884) | ❌ Render thread submits directly | ~1.5 ms/frame off render thread |
| 7 | **Calibrated timestamps (GPU idle analysis)** | `vk_sync.c` (`cal_refresh`, `ts_read_cb`) | ❌ No GPU idle/stalled breakdown | Detailed GPU profiling; identifies starvation |

### Future Work (Priority 3)

| # | Technique | Rekarina Source | Effort | Impact |
|---|-----------|-----------------|--------|--------|
| 8 | **Graphics Pipeline Library (GPL) fast-linking** | `vk_pipeline.c` (gpl path) | Very High | ~50% faster pipeline creation |
| 9 | **Async compute on compute queue** | `vk_sync.c` (ac_mode, NVIDIA only) | High | True async compute |
| 10 | **Snapshot ring for zero-stall readback** | `vk_memory.c` (snapshot ring) | Medium | Zero-stall CPU access to GPU data |

---

## Implementation Specifications (From Comparison Document)

### 1. Linux Write-Watch (`userfaultfd`/`mprotect`)

**Target:** `engine/fefw-recomp/runtime/src/nxrt/write_watch.cpp` (new `write_watch_linux.cpp`)

```cpp
// Platform abstraction required:
#if defined(_WIN32)
    // Existing WriteWatch via mem::ww
#else
    // Linux: userfaultfd + mprotect
    // 1. Create userfaultfd per region (UFFD_USER_MODE_ONLY | UFFD_NONBLOCK)
    // 2. mprotect(PROT_NONE) on pages to track
    // 3. Background thread: UFFDIO_PAGEFAULT handler
    //    - mprotect(PROT_READ|PROT_WRITE) on faulted page
    //    - Record write in bitmap
    // 4. arm(): mprotect(PROT_NONE) on range
    // 5. take_written(): return pages faulted since last call
#endif
```

**Integration:** `nvn_gpu_pool.cpp` `Mirror::absorb_writes` ↔ `mem::ww::take_written`

**Validation:** Steam Deck / Linux CI; compare memory coherence Windows vs Linux

---

### 2. Async Write-Back (AWB) on Compute Queue

**Target:** New `nvn_async_wb.cpp` + `nvn_gpu.cpp` integration

```cpp
// Architecture (from Rekarina vk_sync.c):
// Graphics Queue                    Compute Queue (AWB)
// Submit fence=N               ←    Wait on timeline=N
//    │                               │
//    ▼                               ▼
// Signal timeline=N              Record copy mirror→host (vkCmdCopyBuffer)
//                                   │
//                                   ▼
//                              Signal awb_sem=N
// Completion thread:
//   wait timeline=N
//   wait awb_sem=N (if AWB pending)
//   → mark fence done
```

**Config:** `[nvn] async_writeback = true` (default off)

**Key Integration Points:**
- Detect compute-only queue family at device init
- In `pool_copy_end`: if large mirror dirty, record to AWB CB instead of graphics
- `wait_serial_blocking`: also wait `awb_sem` for that serial
- Must coordinate with `defer_destroy` / `retire()`

**Expected Gain:** ~3.5 ms/frame off graphics queue (measured on OoT queue 3)

---

### 3. Texture Pooling (TexSlot Adoption)

**Target:** New `nvn_texture_pool.cpp` + modifications to `nvn_gpu_texture.cpp`

```cpp
// Rekarina's TexSlot system:
struct TexSlot {
    VkImage image;
    VkDeviceMemory memory;
    VkImageView view;
    uint32_t refcount;
    enum { LIVE, PARKED, ADOPTED } state;
    // Dual-key lookup:
    //   by_key: creation params (format, size, usage, etc.)
    //   by_addr: guest GPU address
};

// Operations:
// adopt_released: take image/memory from parked slot
// follow_rekey: rebind existing image to new guest address
// park_image: keep alive for reuse (timer-based eviction)
```

**Current EmblemWeave Flow:**
```
Texture created → new VkImage + VkDeviceMemory
Texture released → defer_destroy(tex_destroy) → vkDestroyImage + vkFreeMemory
```

**New Flow:**
```
Texture created → by_key lookup → by_addr lookup → adopt_released → new alloc
Texture released → move to PARKED, start TTL timer
Parked eviction → vkDestroyImage + vkFreeMemory (on VRAM pressure or TTL)
```

**Config:** `[nvn] texture_pool_size_mb = 256`, `texture_park_ttl_frames = 300`

---

### 4. Entry-Step Pipeline Prewarm

**Target:** New `nvn_prewarm_entry.cpp` + `nvn_pipeline.cpp` hook

```cpp
// Called from install_backend() after device init, before first frame
void prewarm_entry_step() {
    // 1. Load persistent metadata (shader/pipeline keys from previous runs)
    // 2. Walk preparation ledger (all encountered during gameplay recording)
    // 3. Sort by priority: frequency * 0.99^frames_since_last
    // 4. Spawn N-1 builder threads (hardware_concurrency - 1)
    // 5. Build ALL graphics/mesh/compute pipelines SYNCHRONOUSLY
    // 6. lb_warm_prepared: pre-build shader libraries (if GPL later)
    // 7. Block until complete
}
```

**Config:** `[nvn] prewarm_entry_step = true` (default on)

**Startup Cost:** ~2-3s on OoT for 1000s of pipelines (acceptable for gameplay sessions)

---

## Validation Plan

### Unit Tests (Add to `nxrt_gfx_tests.exe`)

| Test | Target |
|------|--------|
| `gfx_nvn_pool_write_watch` | Linux write-watch correctness |
| `gfx_nvn_async_wb` | AWB synchronization with defer_destroy |
| `gfx_nvn_texture_pool` | Adoption, parking, eviction, no use-after-free |
| `gfx_nvn_prewarm_entry` | Entry-step completes, no first-use stalls |

### Soak Test (2-Hour Gameplay)

**Configuration:**
```ini
[prof]
renderer_profiling = true
renderer_csv = "soak_profile.csv"

[diag]
soak_csv = "soak.csv"

[nvn]
async_writeback = true
texture_pool_size_mb = 256
prewarm_entry_step = true
```

**Metrics to Compare (Before/After):**

| Metric | Baseline Target | Success Criteria |
|--------|-----------------|------------------|
| Frame Time P50 | < 16.67 ms | ≤ baseline |
| Frame Time P95 | < 20 ms | ≤ baseline - 10% |
| Frame Time P99 | < 33 ms | ≤ baseline - 15% |
| `nvn.cnt.gpu_us` (GPU busy) | — | ↓ 3.5 ms/frame (AWB) |
| `nvn.pool_import_failures` | — | ↓ (async write-back) |
| `nvn.mirror.copied_mb` | — | Stabilized (pooling) |
| `rt_submit_ms` | — | ↓ 1.5 ms (submitter thread) |
| `rt_lock_wait_ms` | — | ↓ (less contention) |
| `pw_wait_ms` / `pw_bounded_drops` | — | 0 waited on entry step |
| Startup time | — | +2-3s acceptable |

### Steam Deck / Linux Validation

- Run `nxrt_gfx_tests.exe` on Steam Deck (Linux)
- Verify write-watch parity with Windows
- Confirm no memory coherence issues
- Profile frame times on Deck GPU (RDNA 2 / Van Gogh)

---

## Files to Create / Modify

### New Implementation Files
```
engine/fefw-recomp/runtime/src/nxrt/write_watch_linux.cpp          # Linux write-watch
engine/fefw-recomp/runtime/src/gfx/nvn/nvn_async_wb.cpp            # Async write-back
engine/fefw-recomp/runtime/src/gfx/nvn/nvn_texture_pool.cpp        # Texture pooling
engine/fefw-recomp/runtime/src/gfx/nvn/nvn_prewarm_entry.cpp       # Entry-step prewarm
engine/fefw-recomp/runtime/src/gfx/nvn/nvn_gpu_timing.cpp          # Calibrated timestamps
engine/fefw-recomp/runtime/src/gfx/nvn/nvn_submitter.cpp           # Submitter thread
```

### Modified Files
```
engine/fefw-recomp/runtime/src/nxrt/write_watch.cpp                # Platform abstraction
engine/fefw-recomp/runtime/src/gfx/nvn/nvn_gpu_pool.cpp            # Mirror + AWB + pooling
engine/fefw-recomp/runtime/src/gfx/nvn/nvn_gpu_texture.cpp         # Texture pool integration
engine/fefw-recomp/runtime/src/gfx/nvn/nvn_pipeline.cpp            # Entry-step hook
engine/fefw-recomp/runtime/src/gfx/nvn/nvn_prewarm.cpp             # Priority queue reuse
engine/fefw-recomp/runtime/src/gfx/nvn/nvn_gpu.cpp                 # Submitter, AWB, timing
engine/fefw-recomp/runtime/src/gfx/nvn/nvn_gpu_internal.h          # New declarations
docs/CONFIG.md                                                      # New config knobs
```

### New Config Knobs (docs/CONFIG.md)
```ini
[nvn]
# Async write-back on compute queue (offloads mirror copies from graphics queue)
async_writeback = false

# Texture pooling (reduces allocation spikes)
texture_pool_size_mb = 256
texture_park_ttl_frames = 300

# Entry-step pipeline prewarm (eliminates first-use stalls)
prewarm_entry_step = true

# Submitter thread (moves vkQueueSubmit2 off render thread)
submit_thread = false

# Calibrated timestamps for GPU idle analysis
gpu_timing = false
```

---

## Risk Assessment & Mitigation

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| AWB sync bugs with defer_destroy | Medium | High (device loss) | Validation mode; small mirrors first; extensive soak |
| Texture pool use-after-free | Medium | High | Refcount + validation mode (`texture_pool_validate = true`) |
| Entry-step startup regression | Low | Medium (user-perceived) | Configurable; measure; only for gameplay sessions |
| Linux write-watch correctness | Medium | High (correctness) | Steam Deck testing; stress test coherence |
| Submitter thread reordering | Low | High | Strict queue order; `submit_drain()` before any wait |

---

## Next Steps

1. **Implement Priority 1 items** in order: Linux write-watch → AWB → Texture pooling → Entry-step prewarm
2. **Run unit tests** after each implementation
3. **Run soak test** after all Priority 1 complete
4. **Validate on Steam Deck** (Linux write-watch)
5. **Document results** in this file (update with actual measurements)
6. **Implement Priority 2** items based on Priority 1 validation

---

## Sign-Off

**Investigation Complete:** ✅  
**Comparison Document:** ✅ `REKARINA_PERFORMANCE_COMPARISON.md`  
**Results Document:** ✅ `REKARINA_PERFORMANCE_RESULTS.md` (this file)  
**Implementation:** ⏳ Pending

**Next Action:** Begin Priority 1 implementation with Linux write-watch (`write_watch_linux.cpp`)

---

*End of REKARINA_PERFORMANCE_RESULTS.md*