# Conflict Resolution Register

**Date:** 2026-11-24
**Scope:** the integration of FCC's working tree into canonical `main`
**Policy applied:** user data and working functionality first, then valid FCC engineering, then valid Freebuff work.
Ties were settled by correctness and by whether the result compiles and links from a clean configure — not by file
timestamps or by who wrote the code.

Each entry states the competing implementations, their behavioural difference, the decision, and how it was checked.

---

## CR-1 — Write-watch: one Windows file vs. a platform split

**Competing versions**
- `main`: `src/mem/write_watch.cpp`, a 1,127-line Windows implementation.
- FCC: the same code moved to `src/platform/win32/write_watch_win32.cpp`, plus a new
  `src/platform/posix/write_watch_linux.cpp` (`userfaultfd` + `mprotect`), with `src/mem/write_watch.cpp` deleted.

**Behavioural difference.** On Windows: none — the body is the same file. On Linux: FCC's version provides an
implementation where `main` had none.

**Dependencies.** `CMakeLists.txt` already globs `${NXRT_PLATFORM_DIR}/*.cpp` per host and excludes `/src/platform/`
from the recursive source glob, so the split needed no build-system change. Two CMake unit-test names
(`write_watch`, `svc_*`) and the `write_watch` CTest timeout are unaffected.

**Decision: FCC's split wins.** It is a pure relocation on Windows, it matches the build system's existing platform
layout, and it is the only path toward Linux. `mem/write_watch.cpp` is gone; nothing references the old path.

**Check.** Full engine build from a clean configure (see `ENGINE_AND_RENDERER_VALIDATION.md`); the Windows platform
glob compiled the moved file. The Linux file is **not** compiled on Windows and remains unverified (see CR-13).

---

## CR-2 — Where the renderer profiling implementation lives

**Competing versions**
- FCC: `src/diag/renderer_profiling.cpp`, i.e. the `nxrt` core library.
- The build reality: `nxrt` is built from `src/*.cpp` **excluding** `/src/gfx/`, links only `psapi`/`nxrt_zstd`, and
  never sees Volkan or the VMA. The file calls `vk_gauge_image_bytes()`, `vk_gauge_buffer_bytes()`,
  `vk_gauge_memory_bytes()` and `vk_gauge_memory_allocs()`, all defined in `src/gfx/vk_device.cpp`.

**Behavioural difference.** As written, any target that links `nxrt` without `nxrt_gfx` (notably `nxrt_tests`) has
undefined references, so the tree does not link. FCC worked around the symptom twice (a duplicate `g_ring_bytes`,
CR-3) instead of moving the file.

**Options considered**
1. Move the implementation into `nxrt_gfx` (chosen).
2. Keep it in core and add weak/`#pragma comment(linker, "/alternatename")` stubs.
3. Teach core to link `nxrt_gfx` — a circular dependency, rejected outright.

**Decision: (1).** The profiling data is produced and consumed entirely inside the graphics library
(`nvn_*.cpp`, `vk_renderer.cpp`); core has no producer or reader. Moving the file keeps one definition of every
symbol, removes the link inversion, and costs nothing: `nxrt_tests` never referenced the API.

**Check.** `src/diag/renderer_profiling.cpp` → `src/gfx/renderer_profiling.cpp`; the `src/gfx/*.cpp` glob picks it
up. Clean configure + full build.

---

## CR-3 — Duplicate `g_ring_bytes` symbol

**Competing versions**
- `nvn_gpu.cpp`: `std::atomic<uint64_t> g_ring_bytes{0}, g_ring_fallback_bytes{0};` — inside
  `nxrt::nvn::be::gpu`'s anonymous namespace.
- FCC's `renderer_profiling.cpp`: `namespace nxrt { extern std::atomic<uint64_t> g_ring_bytes; }` **plus** a strong
  definition `std::atomic<uint64_t> g_ring_bytes{0};` whose comment calls it a "weak fallback".

**Behavioural difference.** Two cases, both wrong: linked together they are two distinct objects, so the
`rp.upload_ring_mb` gauge reads a permanently-zero counter (silent bad data); and once both are in the same library,
the strong definition collides with the real one. The comment's "weak" claim is false — `std::atomic` at namespace
scope is not weak on MSVC/clang-cl.

**Decision: delete the duplicate and the gauge that read it.** The staging ring is already reported, always-on, by
the `staging_ring_kb` and `staging_own_kb` hitch counters registered beside the real variable.

**Check.** No remaining reference to `nxrt::g_ring_bytes`; `rp.upload_staging_mb` still reports upload traffic.

---

## CR-4 — Render-phase timers writing into the hitch ring

**Competing versions**
- `nxrt::HitchPhase`: `kHpRenderIdle = 0`, `kHpRenderReplay = 1`, `kHpRenderGpuWait = 2`, `kHpRenderSubmit = 3`,
  `kHpRenderUpload = 4`, `kHpRenderPipeline = 5`, … (24 values).
- FCC's `nxrt::rp::phase`: `kReplay = 0`, `kPipeline = 1`, `kUpload = 2`, `kPresent = 3`, `kAcquire = 4`, …
  (14 values).

**Behavioural difference.** `RenderPhaseTimer` called `hitch_phase_add(phase, …)` with the **rp** id. Every timer
therefore landed in an unrelated hitch bucket: replay was booked as idle, pipeline as replay, upload as GPU wait,
present as submit, acquire as upload, limiter as pipeline. The hitch report and the 30-second hitch line — existing,
user-visible diagnostics — would have become quietly wrong. The ids stayed below `kHpCount`, so nothing crashed;
the data just lied.

**Decision: give the profiling module its own accumulators.** `render_phase_add(int phase, uint64_t ns)` maps each
`rp::phase` to the matching `RenderThreadStats` field. The hitch ring keeps only its own `HitchTimer` call sites.
Both sets of numbers are then correct and independent.

**Check.** Compiled; the mapping is total over `rp::phase` (a `default:` case drops unknown ids rather than writing
out of range).

---

## CR-5 — `drain_pending_destroy()` wrapper resolving to itself

**Competing versions**
- Internal helper (anonymous namespace): `void drain_pending_destroy()` — moves queued destructions onto the
  deferred list.
- FCC's new public wrapper in the enclosing namespace: `void drain_pending_destroy() { if (g) ::nxrt::nvn::be::gpu::drain_pending_destroy(); }`.

**Behavioural difference.** Members of an unnamed namespace are visible to qualified lookup in the enclosing
namespace, so the qualified call names both functions: the wrapper is either ambiguous (compile error) or resolves
to itself (unbounded recursion on every `backend_drain_deferred_destruction()` call, which is exactly the
scene-transition path this work exists to make safer).

**Decision: rename the internal helper to `absorb_pending_destroy()`** and have the wrapper (and `tear_down()`)
call it directly. No behaviour change beyond removing the self-reference.

**Check.** Compiled; single call chain `backend_drain_deferred_destruction → drain_pending_destroy → absorb_pending_destroy`.

---

## CR-6 — Frame-time metrics with no producer

**Competing versions**
- The API: `note_frame_time(float ms)` and a `FrameTimeWindow` of 8,192 samples, advertised as the source of
  P50/P95/P99.
- The wiring: **no caller anywhere in the tree.**

**Behavioural difference.** Every percentile stayed `0.000`, and `renderer_profiling_periodic_csv()` was defined
but never declared or called, so `[prof] renderer_csv` would have held a header line and (at most) one shutdown row.
This is visible in the committed Rekarina results, which report P50/P95/P99/max all zero as a "baseline".

**Decision: wire it in the present path.** On each presented (non-repeated) frame, the render thread adds the
frame's start-to-end time to the window; about once a second it also refreshes the GPU-memory gauges and appends a
CSV row. Both are no-ops when profiling is off. `renderer_profiling_periodic_csv()` is declared in the header.

**Limitation, stated plainly:** no measurement has been taken yet. What changed is that a future run produces real
numbers instead of zeros.

---

## CR-7 — `prewarm_recency_decay` read but ignored

**Competing versions**
- Config: `[nvn] prewarm_recency_decay` (default `0.99`) is read into `g_queue_config`.
- The priority function: hard-coded `1.0f + age * 0.01f`.

**Behavioural difference.** Changing the documented key did nothing; the effective decay was always 0.99.

**Decision: use the configured value.** `KeyMetadata::update_priority(frame, decay)` takes the decay and both call
sites pass `g_queue_config.recency_decay`.

**Check.** Compiled; documented in `docs/CONFIG.md`.

---

## CR-8 — Memory snapshot in the core diagnostics library

**Competing versions**
- FCC: `diag_memory_snapshot()` added to `src/diag/registry.cpp`, calling `nxrt::gfx::video_budget_bytes()`,
  `video_usage_percent()` and `video_live_percent()` (defined in `src/gfx/vk_device.cpp`).
- The build reality: `registry.cpp` is in `nxrt` core, which does not link the graphics library.

**Behavioural difference.** Same class of failure as CR-2: undefined references for every target that links core
without gfx, including `nxrt_tests`.

**Decision: keep the API in `nxrt/diag.h`, move the implementation to `src/gfx/diag_memory_snapshot.cpp`, and
revert `registry.cpp` to its previous state.** The snapshot only makes sense with a device; the declaration stays
in core so callers do not need Vulkan headers.

**Addition:** the snapshot is appended to the GPU device-lost report, which is where Issue #37/#41 evidence is
actually collected from a player.

**Check.** Clean build; `registry.cpp` is byte-identical to `HEAD`.

---

## CR-9 — Launcher subprocess consoles

**Competing versions**
- `main`: `std::process::Command` used directly for build probes, `git rev-parse`, python probes and ReShade setup;
  launching them from a GUI process flashes console windows.
- FCC: a new `process.rs` with `hidden()` (`CREATE_NO_WINDOW`) and `new_hidden()`, applied at those call sites.

**Behavioural difference.** FCC's version suppresses console flashes; it does not change the game runtime, which
keeps its own `show_debug_console` setting.

**Decision: keep FCC's version.** It is additive, cfg-gated to Windows, and has two unit tests.

**Check.** `cargo test` for `apps/emblemweave`: 442 tests, 429 passed, 0 failed, 13 ignored — two more tests than
the previous 440/427/13 baseline, matching the two new `process` tests.

---

## CR-10 — Documentation contradicting the code

| Claim | Reality | Resolution |
|---|---|---|
| `docs/CONFIG.md`: `prewarm_threads` default `max(1, min(6, cpus / 4))` | code: `clamp(cpus / 8, 1, 2)`, capped by `prewarm_max_workers` (2) | doc corrected to the code |
| FCC reports: `prewarm_workers` | config key is `prewarm_max_workers`; `prewarm_threads` is the worker count | doc rows added for the real keys |
| `ISSUE_37_PHASE1_COMPLETION.md`: "commit 314f3b2e", "COMPLETE - DEPLOYMENT VERIFIED" | the files were untracked at that commit; runtime effect unmeasured | dated banner; status qualified |
| `REKARINA_PERFORMANCE_RESULTS.md`: baseline P50/P95/P99 = 0.000 | metric had no producer (CR-6) | dated banner explaining the zeros |

---

## CR-11 — The preserved stash vs. the integrated tree

**Competing versions**
- `stash@{0}` "pre-PR WIP: audio EQ, MAKO, play UI, docs": 45 files, 1,412 insertions — audio sink EQ, MAKO
  upscaler plumbing, a new overview page, play UI changes, docs.
- The integrated tree: builds and passes its launcher tests; the engine builds from a clean configure.

**Behavioural difference.** Unknown. The stash predates the current `main`, overlaps live files (`engine.rs`,
`main.rs`, `CONFIG.md`, `output_pass.*`, `play_*`, `nvn_present.cpp`), and has never been built against this tree.

**Decision: preserve, do not apply in this pass.** Applying 1,400 lines of unbuilt draft work on top of an
already-unverified change set would make any failure unattributable and could regress working functionality — the
first priority in the policy. The stash is retained in full, with its base commit, for its author or a dedicated
follow-up branch built on today's `main`.

**Recorded, not lost:** `git stash list` still shows it; the snapshot commit also contains the tree as handed over.

---

## CR-12 — Historical reports vs. release documentation

**Competing versions.** FCC's reports claim completed, verified, deployed work. The reconciled readiness audit
records #70/#75/#76 as implemented-unverified and records no release candidate.

**Decision.** Do not delete or rewrite the historical reports; add dated banners and keep the audit authoritative.
The reports remain useful evidence of what was attempted, and the banners prevent them from being quoted as a
release verdict. Files: `FCC_FINAL_REPORT.md`, `ISSUE_70_RESOLUTION.md`, `ISSUE_37_PHASE1_COMPLETION.md`,
`DEPLOYMENT_ISSUE37.md`, `REKARINA_PERFORMANCE_*.md`, plus the two `runtime/docs/audits/ISSUE_37_*.md` references
whose paths and status were corrected.

---

## CR-13 — Linux portability claims

**Competing versions**
- FCC shipped `write_watch_linux.cpp` and described Linux work as in progress.
- The tree cannot build for Linux: `nxrt` links `psapi` unconditionally, `src/diag/registry.cpp` includes
  `<windows.h>`, and the configured toolchain is clang-cl/MSVC.

**Decision: keep the file, claim nothing.** It is a genuine step, but it is not compiled by any target here, so it
is recorded as unverified source rather than as a port. A native Linux build needs the platform layer (`psapi`,
`registry.cpp`, thread affinity, the Vulkan loader path) guarded first; that is the next Linux milestone.

---

## Summary

| ID | Area | Outcome |
|---|---|---|
| CR-1 | write-watch platform split | FCC wins (relocation + Linux source) |
| CR-2 | profiling implementation location | moved to `nxrt_gfx` |
| CR-3 | duplicate `g_ring_bytes` | removed; bogus gauge dropped |
| CR-4 | phase-timer id mismatch | own accumulators |
| CR-5 | self-recursive drain wrapper | internal helper renamed |
| CR-6 | unfed frame-time metrics | wired into present |
| CR-7 | ignored recency decay | wired |
| CR-8 | memory snapshot link inversion | moved to `nxrt_gfx`, hooked to device-lost report |
| CR-9 | launcher console flashes | FCC wins |
| CR-10 | docs vs. code | docs corrected |
| CR-11 | stash WIP | preserved, not applied |
| CR-12 | historical claims | bannered, audit authoritative |
| CR-13 | Linux claims | source kept, no claim made |
