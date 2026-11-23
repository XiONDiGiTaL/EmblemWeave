# Engine Performance Roadmap: EmblemWeave (fefw-recomp)

**Version:** 1.0  
**Date:** 2026-11-21  
**Status:** Draft, based on CPU_PARALLELISM_AUDIT.md findings

---

## Executive Summary

The engine is **CPU-bound on the render thread** with GPU utilization < 1%. The primary bottleneck is serial CPU-side work in the render thread: descriptor set construction, pipeline layout resolution, texture preparation, and draw submission, all single-threaded.

**Goal:** Reduce render thread CPU time from ~10ms to ≤6ms/frame (target 144+ FPS avg, 60+ FPS 1% low) by parallelizing independent work and optimizing hot paths.

---

## Phase 1: Foundation & Measurement (Week 1-2)

### 1.1 Complete Baseline Across All Scenarios
| Scenario | Duration | Target Metrics |
|----------|----------|----------------|
| Main Menu (idle) | 60s | Avg FPS, 1%/0.1% lows, CPU/GPU ms |
| Hub Exploration | 120s | Same + scene classification |
| Dagsion Crowd | 120s | Same (current: 96.8 avg, 17.7 1% low) |
| Character-Heavy Combat | 120s | Same |
| Cinematic/Cutscene | Full clip | Same |
| Area Transition + Shader Compile | First visit | Shader backlog, compile stalls |
| 30-min Soak | 1800s | Thermal, memory leaks, stability |

**Deliverable:** `CPU_PARALLELISM_RESULTS.md` with full dataset.

### 1.2 Render Thread Profiling
- **Tool:** Tracy + RenderDoc captures for each scenario
- **Focus:** Attribute CPU time per work item in `VkRenderer::replay()`
- **Output:** Flame graphs, per-function timing, memory allocation profiles

### 1.3 Hardware Topology Detection
- Implement `CpuTopology` class in `runtime/src/core/`
- Detect: P-core/E-core, SMT siblings, cache sharing, core frequencies
- Expose via `nxrt::get_cpu_topology()` for scheduler

**Files to create/modify:**
- `runtime/src/core/cpu_topology.cpp/h` (new)
- `runtime/src/kernel/sched.cpp`: integrate topology
- `runtime/CMakeLists.txt`: add new source

---

## Phase 2: Render Thread Parallelization (Week 2-4)

### 2.1 Parallel Descriptor Set Construction (Highest ROI)
**Target:** Reduce descriptor/binding work from ~3-4ms to ~1ms

**Approach:**
1. Split `VkRenderer::build_descriptor_sets()` into per-draw work items
2. Use thread pool (existing `tex_prep` pool or new `render_prep` pool)
3. Each worker builds descriptor set for a batch of draws
4. Main thread only merges/binds

**Files:**
- `runtime/src/gfx/vk_renderer.cpp`: `replay()`, `build_descriptor_sets()`
- `runtime/src/gfx/nvn/nvn_command_buffer.cpp`: descriptor logic
- New: `runtime/src/gfx/render_prep_pool.cpp/h`

**Acceptance:** Descriptor construction time ≤ 1ms/frame in Crowded scene.

### 2.2 Pipeline Layout & Invariant Caching
**Target:** Eliminate redundant pipeline layout lookups per draw

**Current:** `ProgLayout::writes_storage` cached once; alpha/HDR layout 64-entry TLS cache

**Extend to:**
- Push constant layout per program
- Descriptor set layout per program
- Vertex input state hash
- Dynamic state configuration

**Files:**
- `runtime/src/gfx/nvn/nvn_pipeline.cpp`: `ProgLayout`, `PipelineLayoutCache`
- `runtime/src/gfx/vk_renderer.cpp`: layout resolution

**Acceptance:** Pipeline layout resolution < 0.1ms/frame.

### 2.3 Parallel Texture Preparation Expansion
**Target:** Hide texture upload latency behind render thread

**Current:** `tex_prep_threads = 0` (auto), staging ring 256 MiB

**Enhancements:**
- Increase `tex_prep_threads` default to `max(4, cores - 2)`
- Add priority queue: frame-critical textures first
- Implement texture residency prediction (prefetch next frame's working set)
- Parallel BC7 transcoding for multiple textures simultaneously

**Files:**
- `runtime/src/gfx/nvn/nvn_texture_prep.cpp` (new or extend `nvn_gpu_pool.cpp`)
- `controls.ini`: new `tex_prep_priority` setting

**Acceptance:** Texture upload stalls = 0 in steady state.

### 2.4 Command Buffer Pre-recording (Research)
**Target:** Reuse command buffers across frames for static draws

**Approach:**
- Identify draws with stable descriptors/pipelines (UI, static geometry)
- Pre-record secondary command buffers
- Replay via `vkCmdExecuteCommands` with minimal patching

**Risk:** High. The NVN command stream may not have stable draw IDs.

**Decision point:** After Phase 2.1-2.3 results.

---

## Phase 3: Kernel & Scheduler Optimization (Week 3-4)

### 3.1 Kernel Lock Sharding
**Target:** Reduce contention on single SRWLOCK

**Current:** One `kernel_lock` protects mutex, CV, address, sync wait queues

**Split into:**
- `mutex_lock`: mutex wait queue
- `cv_lock`: condition variable wait queue
- `addr_lock`: address wait queue (atomic waits)
- `sync_lock`: sync object wait queue

**Files:**
- `runtime/src/kernel/kernel_sync.cpp`: `kernel_lock`, `wait_queue`

**Acceptance:** Lock contention < 1% of CPU time in Tracy profile.

### 3.2 Hardware-Aware Core Assignment
**Target:** Optimal thread placement on hybrid/SMT CPUs

**Implementation:**
```cpp
// New policy in sched.cpp
struct CoreAssignment {
    int render_core;           // Best P-core, no SMT sibling
    std::vector<int> worker_cores;  // Remaining P-cores, then E-cores
    std::vector<int> background_cores; // E-cores, SMT siblings
};
```

**Integration:**
- `choose_core()` uses topology
- `reserve_render_core()` pins to best P-core
- Guest threads (0-3) → P-cores
- System thread (4) → E-core or SMT sibling
- Worker pools → E-cores + SMT siblings

**Files:**
- `runtime/src/core/cpu_topology.cpp` (Phase 1.3)
- `runtime/src/kernel/sched.cpp`

**Acceptance:** No render thread migration; worker threads don't contend with render on P-cores.

### 3.3 Adaptive Spin Tuning
**Target:** Reduce latency for short waits without burning CPU

**Current:** `spin_us=30µs`, `lock_spin_us=30µs` (hardcoded)

**Make adaptive:**
- Track average wait duration per wait type
- Adjust spin time: short waits → spin longer; long waits → park immediately
- Expose via `controls.ini` `[kernel]` section

**Files:**
- `runtime/src/kernel/kernel_sync.cpp`: `adaptive_spin()`
- `controls.ini`: new `[kernel]` section

---

## Phase 4: Compiler Optimization (Week 4-5)

### 4.1 PGO (Profile-Guided Optimization)
**Target:** 5-15% speedup on hot paths

**Process:**
1. Build with `-fprofile-instr-generate -fcoverage-mapping`
2. Run benchmark suite (all 7 scenarios)
3. Merge profiles: `llvm-profdata merge`
4. Rebuild with `-fprofile-instr-use=merged.profdata`

**Scope:** `nxrt_gfx`, `nxrt_game_fefw`, `fefw.exe` (not lifted libraries; they're AOT)

**Files:**
- `game-build/fefw/CMakeLists.txt`: PGO flags
- `runtime/CMakeLists.txt`: PGO flags for nxrt_gfx

**Risk:** Profile must represent real gameplay; synthetic benchmarks may misoptimize.

### 4.2 LTO (Link-Time Optimization)
**Target:** Cross-TU inlining, dead code elimination

**Current:** `/OPT:REF /OPT:ICF` at link time only

**Enable:** `-flto=thin` (clang-cl) for full LTO
- ThinLTO scales better than full LTO
- Combine with PGO for best results

**Files:**
- `game-build/fefw/CMakeLists.txt`: `-flto=thin`
- `runtime/CMakeLists.txt`: `-flto=thin` for nxrt targets

**Risk:** Increased link time; debug info quality.

### 4.3 Unity Builds for Graphics
**Target:** Faster compile, better optimization context

**Current:** `NXRT_UNITY_BUILD=ON` with batch size 4 (optional)

**Enable by default** for `nxrt_gfx`: the graphics shell has heavy headers (Vulkan, SDL, ImGui)

**Files:**
- `runtime/cmake/gfx.cmake`: `NXRT_UNITY_BUILD ON` default

---

## Phase 5: Advanced Parallel Execution Research (Week 5-8)

### 5.1 Selective Parallel Game Routines
**Concept:** Identify pure, side-effect-free functions in lifted game code for parallel execution

**Approach:**
1. Static analysis of lifted C++ for `[[gnu::pure]]` / `[[nodiscard]]` candidates
2. Runtime profiling to find hot pure functions
3. Task-based execution with dependency tracking

**Risk:** Very High; the game code is not designed for parallelism; it may break determinism.

**Decision:** Only after Phases 1-4 show diminishing returns.

### 5.2 GPU-Driven Visibility (Experimental)
**Concept:** Move frustum/occlusion culling to GPU compute

**Prerequisites:**
- NVN command stream preserves draw bounding volumes
- Game uses standard culling patterns

**Approach:**
- Compute shader reads instance data, writes indirect draw args
- `VK_EXT_draw_indirect_count` for variable draw counts

**Risk:** High; this requires NVN command stream analysis.

---

## Phase 6: Audio & Resource Processing (Week 3-4, parallel)

### 6.1 Parallel Audio Decoding
**Target:** Decode multiple Opus streams concurrently

**Current:** Single `nx_hle_zstd_multiframe` call per stream, thread-local ZSTD_DCtx

**Enhancement:**
- Thread pool for Opus/Zstd decoding
- Lock-free ring buffer for decoded PCM
- Prioritize active voices

**Files:**
- `runtime/src/services/audio_out.cpp`
- `runtime/src/games/fefw/fefw_hle.cpp`: `nx_hle_zstd_multiframe`

### 6.2 Asset Loading Parallelism
**Target:** Reduce loading screen times

**Current:** Single-threaded RomFS reads with 256-block LRU (4 MiB) + packed cache (1.5 GiB)

**Enhancements:**
- Async file reads with `IOCP` / `io_uring` (Windows: `ReadFileEx` + completion ports)
- Parallel decompression of multiple assets
- Priority: visible assets first

---

## Acceptance Criteria Summary

| Phase | Metric | Baseline | Target | Validation |
|-------|--------|----------|--------|------------|
| 1 | Baseline complete | 1/7 scenarios | 7/7 scenarios | `CPU_PARALLELISM_RESULTS.md` |
| 2.1 | Descriptor build time | ~3-4ms | ≤1ms | Tracy profile |
| 2.2 | Pipeline layout time | ~0.5ms | ≤0.1ms | Tracy profile |
| 2.3 | Texture upload stalls | Occasional | 0 steady-state | Hitch log |
| 3.1 | Kernel lock contention | Unknown | <1% CPU | Tracy profile |
| 3.2 | Render thread migration | Possible | Never | ETW/Perf |
| 4.1 | PGO speedup | 0% | 5-15% | Before/after benchmark |
| 4.2 | LTO binary size | ~170MB | -10-20% | `size` / link map |
| All | Avg FPS (Crowded) | 96.8 | ≥144 | `perf_status.json` |
| All | 1% Low FPS | 17.75 | ≥60 | `perf_status.json` |
| All | 0.1% Low FPS | 5.99 | ≥30 | `perf_status.json` |
| All | GPU utilization | <1% | >50% | `perf_status.json` gpu_ms |

---

## Deployment Checklist (Mandatory)

Before any optimization lands in `E:\EmblemWeave`:

- [ ] All 7 baseline scenarios pass with new build
- [ ] No regression in any scenario (>5% delta = fail)
- [ ] 1% lows improve in ≥5/7 scenarios
- [ ] No new hitches introduced (hitch count ≤ baseline)
- [ ] Memory usage stable over 30-min soak
- [ ] Save/load compatibility verified (1.1.0 + 1.1.1)
- [ ] DLC detection still works
- [ ] Backup of `E:\EmblemWeave\data\play\fefw.exe` and `.pdb` created
- [ ] Rollback procedure documented

---

## Documentation Deliverables

| Document | Status | Owner |
|----------|--------|-------|
| `CPU_PARALLELISM_AUDIT.md` | ✅ Done | This session |
| `CPU_PARALLELISM_RESULTS.md` | ⏳ Pending | Phase 1 |
| `ENGINE_PERFORMANCE_ROADMAP.md` | ✅ Done | This session |
| `PERFORMANCE_BASELINE.md` | ⏳ Pending | Phase 1 |
| `OPTIMIZATION_REPORT.md` | ⏳ Pending | Post-Phase 4 |

---

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Parallel descriptor sets break rendering | Medium | High | Extensive RenderDoc validation; feature flag |
| PGO profile doesn't represent real play | Medium | Medium | Use real gameplay captures; multiple profiles |
| LTO increases link time excessively | High | Low | ThinLTO; parallel link (`lld-link /threads:N`) |
| Hardware topology detection fails on some CPUs | Low | Medium | Fallback to conservative assignment |
| Kernel lock sharding introduces races | Low | Critical | Stress test with `NXRT_STRESS_KERNEL=1` |
| Optimizations break 1.1.1 save compatibility | Low | Critical | Test both saves before deploy |

---

*Roadmap is living; update priorities based on Phase 1 measurement results.*