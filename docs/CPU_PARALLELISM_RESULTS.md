# CPU Parallelism Baseline Results: EmblemWeave Engine

**Date:** 2026-11-21  
**Engine Build:** fefw-recomp (development), fefw.exe v0.1.41  
**Test Hardware:** AMD Ryzen (P-cores), AMD Radeon RX 7900 XT, 32GB RAM  
**Configuration:** 1920×1080, FSR4 Quality, FSR3 Frame Gen, Windowed, VSync Off, 180 FPS cap

---

## Test Configuration

| Setting | Value |
|---------|-------|
| Resolution | 1920×1080 |
| Display Mode | Windowed |
| VSync | Off |
| Frame Limit | 180 FPS |
| Upscaler | FSR 4.1.2 (Quality) |
| Frame Generation | FSR 3 |
| Shader Precompile | Enabled |
| Adaptive Performance | Automatic |
| Target FPS | 179 |
| Pool Mirror | Auto (quarter VRAM) |
| Texture Prep | Enabled (auto threads) |
| GPL | Enabled |
| Prewarm | Enabled |
| Lean Draws | Enabled |

---

## Baseline Results Summary

| Scenario | Duration | Avg FPS | 1% Low | 0.1% Low | CPU (ms) | GPU (ms) | VRAM % | Scene | Bottleneck | Hitches |
|----------|----------|---------|--------|----------|----------|----------|--------|-------|------------|---------|
| **Menu** | 60s | 110.2 | 83.1 | 43.3 | 9.07 | 0.019 | 29% | Crowded* | CPU-bound | 14 |
| **Hub** | 120s | 109.0 | 93.6 | 84.2 | 9.18 | 0.020 | 30% | Crowded* | CPU-bound | 21 |
| **Crowded** | 120s | 108.9 | 94.8 | 84.2 | 9.18 | 0.019 | 30% | Crowded | CPU-bound | 22 |

*Note: Menu and Hub runs classified as "Crowded" scene, likely due to background loading or measurement starting before reaching target location.

---

## Detailed Results

### 1. Main Menu (60s Idle)

**File:** `benchmarks/20261121-215023-Menu/result.json`

| Metric | Value |
|--------|-------|
| Average FPS | 110.2 |
| 1% Low FPS | 83.1 |
| 0.1% Low FPS | 43.3 |
| CPU Frame Time | 9.07 ms |
| GPU Frame Time | 0.019 ms |
| Replay Time | 5.10 ms |
| Upload Time | 0.002 ms |
| VRAM Usage | 29% |
| Scene Classification | Crowded |
| Bottleneck | CPU-bound |
| Hitches | 14 (worst: 746ms frame 8, 297ms frame 123) |
| Spikes (300 frames) | 223 |

**Observation:** Even at main menu, CPU-bound with high hitch count during initial shader/pipeline compilation. Scene classifier reports "Crowded", likely because the measurement window captures early gameplay loading.

### 2. Hub Exploration: Somniel (120s)

**File:** `benchmarks/20261121-215439-Hub/result.json`

| Metric | Value |
|--------|-------|
| Average FPS | 109.0 |
| 1% Low FPS | 93.6 |
| 0.1% Low FPS | 84.2 |
| CPU Frame Time | 9.18 ms |
| GPU Frame Time | 0.020 ms |
| Replay Time | 2.04 ms |
| Upload Time | 0.000 ms |
| VRAM Usage | 30% |
| Scene Classification | Crowded* |
| Bottleneck | CPU-bound |
| Hitches | 21 (worst: 772ms frame 8, 307ms frame 143) |
| Spikes (300 frames) | 255 |

**Observation:** Similar to Menu, the scene classifier didn't transition to "Exploration". This suggests either: (a) the hub area has similar CPU pressure to crowded scenes, or (b) the measurement window included transition/loading periods.

### 3. Dagsion Crowd (120s)

**File:** `benchmarks/20261121-215157-Crowded/result.json`

| Metric | Value |
|--------|-------|
| Average FPS | 108.9 |
| 1% Low FPS | 94.8 |
| 0.1% Low FPS | 84.2 |
| CPU Frame Time | 9.18 ms |
| GPU Frame Time | 0.019 ms |
| Replay Time | 2.12 ms |
| Upload Time | 0.000 ms |
| VRAM Usage | 30% |
| Scene Classification | Crowded ✓ |
| Bottleneck | CPU-bound |
| Hitches | 22 (worst: 807ms frame 8, 300ms frame 138) |
| Spikes (300 frames) | 245 |

**Observation:** Correctly classified as "Crowded". Stable 1%/0.1% lows (excellent, well above 60/30 targets). Average FPS and CPU time don't meet targets. GPU severely underutilized.

---

## Acceptance Criteria Check

| Metric | Target | Menu | Hub | Crowded | Status |
|--------|--------|------|-----|---------|--------|
| Avg FPS | ≥ 144 | 110.2 ❌ | 109.0 ❌ | 108.9 ❌ | **FAIL** |
| 1% Low FPS | ≥ 60 | 83.1 ✅ | 93.6 ✅ | 94.8 ✅ | **PASS** |
| 0.1% Low FPS | ≥ 30 | 43.3 ✅ | 84.2 ✅ | 84.2 ✅ | **PASS** |
| CPU Frame Time | ≤ 6.9ms | 9.07 ❌ | 9.18 ❌ | 9.18 ❌ | **FAIL** |
| GPU Frame Time | ≥ 4.0ms | 0.019 ❌ | 0.020 ❌ | 0.019 ❌ | **FAIL** |
| GPU Utilization | > 50% | < 1% ❌ | < 1% ❌ | < 1% ❌ | **FAIL** |

**Overall:** 2/6 criteria pass. Primary failure: CPU-bound render thread with GPU starvation.

---

## Performance Analysis

### Frame Time Breakdown (Crowded, 120s)

| Component | Time (ms) | % of CPU |
|-----------|-----------|----------|
| **Total CPU** | 9.18 | 100% |
| Replay (draw prep) | 2.12 | 23% |
| Present/Limiter | ~7.0 | 76% (mostly limiter spin) |
| Upload | 0.00 | 0% |
| GPU | 0.02 | N/A |

**Key Finding:** The "Present" time includes the frame limiter spin-wait. Actual render thread work is ~2ms replay + descriptor/pipeline work. The limiter spins for ~7ms waiting for the 180 FPS target (5.55ms frame budget).

### Hitch Analysis

All three runs show consistent hitch pattern:
1. **Frame 2:** ~100-110ms (initial frame setup)
2. **Frame 8:** ~700-800ms (major shader/pipeline compilation burst)
3. **Frame ~130-140:** ~300ms (scene transition/loading)
4. **Frame ~500-600:** ~170-200ms (periodic shader compilation)
5. **Frame ~2600-2800:** ~55-170ms (background compilation)
6. **Frame ~13500-13700:** ~55-180ms (late compilation)

**Root Cause:** Shader/pipeline compilation spikes despite prewarm and GPL. The `shader_backlog` stays at 0 but compilation still occurs on-demand.

### Scene Classification Accuracy

| Scenario | Expected | Actual | Match |
|----------|----------|--------|-------|
| Menu | Menu | Crowded | ❌ |
| Hub | Exploration | Crowded | ❌ |
| Crowded | Crowded | Crowded | ✅ |

**Issue:** Scene classifier uses frame-time signatures, not game state. Menu/Hub show "CPU submission/replay pressure" signature matching Crowded.

---

## Comparison with Initial Cold Run (First Session)

| Metric | Cold Run (30s) | Warm Run (120s Crowded) | Delta |
|--------|----------------|-------------------------|-------|
| Avg FPS | 96.8 | 108.9 | +12.5% |
| 1% Low | 17.8 | 94.8 | +433% |
| 0.1% Low | 6.0 | 84.2 | +1303% |
| CPU (ms) | 10.33 | 9.18 | -11% |
| GPU (ms) | 0.03 | 0.019 | -37% |
| Hitches | 315/300f | 245/300f | -22% |

**Conclusion:** Shader cache and pipeline cache dramatically improve 1%/0.1% lows after first run. Average FPS and CPU time improve modestly.

---

## Remaining Scenarios (Not Yet Tested)

| Scenario | Status | Notes |
|----------|--------|-------|
| Character-Heavy Combat | ⏳ Pending | Need save at combat map |
| Cinematic/Cutscene | ⏳ Pending | Need save at cutscene |
| Area Transition + Shader Compile | ⏳ Pending | First-visit area needed |
| 30-Minute Soak | ⏳ Pending | Stability test |

---

## Next Steps for Optimization

Based on baselines, priority optimizations:

1. **Parallel Descriptor Set Construction.** Target: reduce replay time from 2.1ms → <1ms
2. **Frame Limiter Tuning**: Current 7ms spin is wasteful; investigate adaptive margin
3. **Hardware-Aware Core Pinning**: Ensure render thread on fastest P-core, no SMT contention
4. **Shader Compile Stutter Reduction**: Background compile all pipelines during loading
5. **Kernel Lock Sharding**: Reduce contention in CPU-bound scenarios

---

## Files Generated

```
E:\EmblemWeave\benchmarks\
├── 20261121-214911-Crowded\      # 30s Crowded test
│   ├── result.json
│   ├── fefw.log
│   └── perf_status.json
├── 20261121-215023-Menu\         # 60s Menu test
│   ├── result.json
│   ├── fefw.log
│   └── perf_status.json
├── 20261121-215157-Crowded\      # 120s Crowded test
│   ├── result.json
│   ├── fefw.log
│   └── perf_status.json
├── 20261121-215439-Hub\          # 120s Hub test
│   ├── result.json
│   ├── fefw.log
│   └── perf_status.json
└── benchmark-report-*.md          # Summary reports
```

---

*Baseline established. Ready for Phase 2 optimization implementation.*