# CPU Parallelism Audit: EmblemWeave Engine (fefw-recomp)

**Date:** 2026-11-21  
**Engine Version:** fefw-recomp (development)  
**Build:** clang-cl 23.1.5, Ninja, static MSVC runtime (/MT)  
**Target Hardware:** AMD Ryzen (tested on RX 7900 XT system)

---

## 1. Architecture Overview

### 1.1 Console Core Model (Switch 2 Emulation)

The engine emulates the Switch 2's 6 CPU cores mapped to host processors:

| Guest Core | Role | Host Mapping |
|------------|------|--------------|
| Core 0 | App/General | First available P-core |
| Core 1 | App/General | Next available P-core |
| Core 2 | App/General | Next available P-core |
| Core 3 | App/General | Next available P-core |
| Core 4 | System/Reserved | Soft affinity, lower priority |
| Core 5 | **Render Thread** | Reserved with **strict affinity** to a dedicated P-core |

**Key Implementation:** `choose_core()` and `reserve_render_core()` in `runtime/src/kernel/sched.cpp`.

The render thread (Core 5) gets **strict core affinity** and **highest priority** (Priority Band 5 = Time Critical on Windows). This prevents scheduler migration and ensures consistent frame pacing.

### 1.2 Guest Thread Model

- **One Win32 thread per guest thread**: no N:M threading, no fiber pooling
- **6 priority bands** (0=Idle … 5=Time Critical) mapped from Switch 2 priorities
- **Core slot mutexes** for multi-process isolation (prevents two game instances pinning the same host core)
- **TLS setup** per guest thread for fast access to kernel state

**Key Implementation:** `create_guest_thread()` and `host_thread_entry()` in `runtime/src/kernel/thread.cpp`.

### 1.3 Kernel Synchronization

- **Single SRWLOCK** protects all kernel wait queues (mutex, CV, address, sync)
- **Per-thread condition variables** for wake-up
- **Adaptive spinning:** `spin_us=30µs`, `lock_spin_us=30µs` before park
- **Wait statistics** tracked per wait type (mutex, CV, address, sync)

**Key Implementation:** `kernel_lock`, `wait_queue` and `adaptive_spin()` in `runtime/src/kernel/kernel_sync.cpp`.

---

## 2. Current Parallelism Architecture

### 2.1 Render Thread (Critical Path)

The render thread runs on **dedicated Core 5** with Time Critical priority. It performs:
- Command buffer recording (NVN → Vulkan translation)
- Descriptor set construction and binding
- Pipeline layout resolution
- Texture preparation and feedback checks
- Draw submission

**Bottleneck:** Single-threaded CPU-side render workload. All draw preparation happens serially on this thread.

### 2.2 Texture Preparation (Partially Parallel)

- `tex_prep_threads = 0` (auto = host cores - 2)
- Background texture transcoding (ASTC → BC7) on worker threads
- Staging ring buffer: 256 MiB (`staging_ring_mb`)
- Pool mirror copies: device-local duplicates of pool pages

**Configuration:** `controls.ini` `[nvn]` section
- `pool_mirror_mb = 0` (auto: quarter VRAM, max 2048 MiB, none on iGPU)
- `pool_mirror_pressure_percent = 85`
- `mirror_hot_copy = on` (copies hot pages without write tracking)

### 2.3 Shader Compilation (Parallel)

- Background pipeline compilation via Vulkan `VK_EXT_graphics_pipeline_library` (GPL)
- Prewarm system: 1671 pipelines built ahead (from log)
- Compile priority: "below normal" (`compile_priority = below`)
- Eco mode: enabled (`compile_eco = 1`)

### 2.4 Audio (Single-Threaded)

- One host sink per stream
- 1ms timer pacing
- Device-clock buffer release with latency compensation
- Opus decoding: native `nx_hle_zstd_multiframe` with thread-local ZSTD_DCtx

**Key Implementation:** `runtime/src/services/audio_out.cpp`

### 2.5 Adaptive Performance Controller (Monitoring Only)

- 300-frame ring buffer (5 seconds @ 60 FPS)
- Bottleneck classification: CPU/GPU/VRAM/Shader/Streaming/Sync/Scheduling
- Scene classification: Loading/Crowded/Combat/Cinematic/Menu/Exploration
- Automatic mip bias adjustment (when `adaptive_mip = true`)

**Key Implementation:** `runtime/src/gfx/perf.cpp`

---

## 3. Identified Serialization Bottlenecks

### 3.1 Render Thread CPU Workload (Primary Bottleneck)

From `ENGINE_AUDIT.md` and runtime measurements:

| Work Item | Cost | Parallelizable? |
|-----------|------|-----------------|
| Descriptor-set construction/binding per draw | High | **Yes** (per-draw independent) |
| Uniform/storage range validation & copies | Medium | **Yes** (per-draw independent) |
| Per-draw texture preparation/feedback | Medium | **Yes** (per-draw independent) |
| Pipeline-layout lookup/invariants | Low | **Partially** (cached via `ProgLayout::writes_storage`) |
| Shader compilation/pipeline creation | High (first visit) | **Yes** (already background) |
| Storage-write tracking | Low | **Partially** (cached) |
| Alpha/HDR/layout selection | Low | **Cached** (64-entry TLS cache) |

**Evidence:** `perf_status.json` shows `bottleneck: "CPU-bound"`, `cpu_ms: 10.33`, `gpu_ms: 0.03`. The GPU is nearly idle while the CPU spends 10ms/frame.

### 3.2 Kernel Lock Contention

- **Single SRWLOCK** for all wait queues
- All guest threads contend on one lock for: mutex, CV, address waits, sync waits
- Adaptive spinning helps but doesn't eliminate contention under load

### 3.3 Texture Preparation Pipeline

- Staging ring buffer is single-producer (render thread) → single-consumer (upload thread)
- BC7 transcoding parallel but limited by `tex_prep_threads`
- Pool mirror copying happens on render thread during replay

### 3.4 Guest Thread Scheduling

- No work-stealing or task-based parallelism
- Each guest thread runs independently but synchronizes via kernel locks
- No parallel execution of independent game routines (all lifted code runs on guest threads)

---

## 4. Hardware Characteristics & Scheduling Gaps

### 4.1 Current CPU Detection

- `FEFW_CPU_VARIANT`: `fma` (FMA3 + AVX) or `compat` (SSE4.2 + POPCNT)
- Set via `FEFW_CPU_VARIANT` env var or CMake cache
- No detection of: P-core vs E-core (hybrid), SMT topology, cache hierarchy

### 4.2 Missing Hardware Awareness

| Feature | Current State | Needed |
|---------|---------------|--------|
| Hybrid CPU (P/E cores) | Not detected | Core type enumeration |
| SMT topology | Not detected | Sibling mapping |
| Cache hierarchy | Not detected | L2/L3 sharing maps |
| NUMA | Not applicable (consumer) | n/a |
| Thermal/power | Not monitored | RAPL/MSR reading |

### 4.3 Current Affinity Model

```cpp
// sched.cpp: choose_core()
- Core 5 (render) → strict affinity, highest priority
- Cores 0-3 → soft affinity, round-robin
- Core 4 → soft affinity, lower priority (system)
```

**Gap:** No topology-aware placement. On hybrid CPUs, render thread should pin to P-core; background workers to E-cores.

---

## 5. Parallelization Opportunities

### 5.1 High Impact (Tier 1)

| Opportunity | Description | Effort | Risk |
|-------------|-------------|--------|------|
| **Parallel descriptor set construction** | Build descriptor sets for multiple draws concurrently | Medium | Low (read-only NVN command stream) |
| **Parallel texture preparation** | Expand `tex_prep` to handle multiple draws' texture ops | Medium | Low (already threaded) |
| **Pipeline layout caching** | Extend `ProgLayout` cache to more invariants | Low | Very Low (already done for `writes_storage`) |
| **Multi-threaded command recording** | Split NVN command stream replay across threads | High | Medium (requires command buffer splitting) |

### 5.2 Medium Impact (Tier 2)

| Opportunity | Description | Effort | Risk |
|-------------|-------------|--------|------|
| **Kernel lock sharding** | Separate locks per wait queue type | Medium | Low (internal kernel change) |
| **Task-based guest scheduling** | Convert guest threads to task graph for parallel regions | High | High (requires game code analysis) |
| **Hardware-aware core assignment** | Detect P/E cores, SMT; pin render to best P-core | Medium | Low |
| **Parallel audio decoding** | Decode multiple Opus streams concurrently | Low | Low (streams are independent) |

### 5.3 Research/Experimental (Tier 3)

| Opportunity | Description | Effort | Risk |
|-------------|-------------|--------|------|
| **Selective parallel game routines** | Identify pure functions in lifted code for parallel execution | Very High | Very High |
| **GPU-driven culling** | Move frustum/occlusion to GPU compute | Very High | High |
| **Mesh shaders / task shaders** | Replace draw submission with mesh pipeline | Very High | High |

---

## 6. Baseline Performance Metrics (Initial Run)

**Configuration:** 1920×1080, FSR4 Quality, FSR3 Frame Gen, Windowed, VSync Off, 180 FPS cap

| Metric | Value | Notes |
|--------|-------|-------|
| Average FPS | 96.8 | From `perf_status.json` |
| 1% Low FPS | 17.75 | `low1_fps` |
| 0.1% Low FPS | 5.99 | `low01_fps` |
| CPU Frame Time | 10.33 ms | `cpu_ms` |
| GPU Frame Time | 0.03 ms | `gpu_ms` shows **GPU severely underutilized** |
| Upload Time | 0.31 ms | `upload_ms` |
| Replay Time | 2.43 ms | `replay_ms` |
| VRAM Usage | 27% | `vram_percent` |
| Bottleneck | CPU-bound | `bottleneck` classification |
| Scene | Crowded | `scene` classification |
| Spikes (300 frames) | 315 | `spikes` count |

**Key Finding:** GPU runs at ~0.03ms/frame while CPU takes 10.33ms. **GPU utilization < 1%**. The render thread is the hard bottleneck.

---

## 7. Acceptance Criteria for Optimizations

| Metric | Current | Target | Measurement |
|--------|---------|--------|-------------|
| Avg FPS (Crowded) | 96.8 | ≥ 144 (+50%) | `perf_status.json` avg_fps |
| 1% Low FPS | 17.75 | ≥ 60 (+240%) | `perf_status.json` low1_fps |
| 0.1% Low FPS | 5.99 | ≥ 30 (+400%) | `perf_status.json` low01_fps |
| CPU Frame Time | 10.33 ms | ≤ 6.9 ms (-33%) | `perf_status.json` cpu_ms |
| GPU Frame Time | 0.03 ms | ≥ 4.0 ms (utilization) | `perf_status.json` gpu_ms |
| Render Thread Stalls | Frequent | Rare | Hitch log count |
| Shader Compile Stalls | First-visit | Background only | `shader_backlog` = 0 |

---

## 8. Next Steps

1. **Establish full baseline** across all 10 gameplay scenarios (Section 6 only covers Crowded)
2. **Profile render thread** with Tracy/RenderDoc to attribute CPU time per work item
3. **Implement parallel descriptor set construction** (Tier 1, highest ROI)
4. **Add hardware topology detection** for P/E core awareness
5. **Shard kernel lock** by wait queue type
6. **Expand texture preparation parallelism**
7. **Evaluate PGO/LTO** for critical render thread hot paths

---

## 9. Related Documents

- [[ENGINE_AUDIT]]: Full engine audit with roadmap
- [[PERFORMANCE]]: Benchmark methodology and results (to be created)
- [[ENGINE_PERFORMANCE_ROADMAP]]: Prioritized optimization roadmap (to be created)

---

*This audit is a living document. Update as optimizations are implemented and measured.*