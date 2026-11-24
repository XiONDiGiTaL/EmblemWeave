# FCC Work Integration

**Date:** 2026-11-24
**Canonical branch:** `main`
**Base commit at the start of the integration:** `6ac9791f2b60b40a5b52b3bb6350ead4fae58dcb`
**Recovery point:** `refs/backup/fcc-wip-20261124` → `8c157698c07db3032aba9b381e5a02f7bc33acf8`

This file records what FCC left in the shared working tree, what was recovered, what was integrated, and what was
deliberately not integrated. It is the companion to `PROJECT_CONSOLIDATION_REPORT.md` (what happened) and
`CONFLICT_RESOLUTION_REGISTER.md` (why each competing implementation lost or won).

---

## 1. Recovery point

No file, branch, stash, or worktree was reset, cleaned, reverted, or overwritten. The recovery point was built
without touching the working tree: a temporary index was loaded from `HEAD`, `git add -A` captured every tracked and
untracked non-ignored file, `git write-tree` / `git commit-tree` produced a commit object, and
`git update-ref refs/backup/fcc-wip-20261124` stored it.

| Item | Value |
|---|---|
| Snapshot commit | `8c157698c07db3032aba9b381e5a02f7bc33acf8` |
| Snapshot parent | `6ac9791f` (canonical `main` at the time) |
| Files in snapshot | 47 changed, 6,064 insertions, 88 deletions |
| Tracked modifications | 32 modified, 1 deleted |
| Untracked paths | 15 |
| Stashes | `stash@{0}` "pre-PR WIP: audio EQ, MAKO, play UI, docs" — **preserved, not applied** |
| Worktrees | main checkout `6ac9791f`; `target/issue79/clean` detached at `b33564ec` (test fixture) |
| Ignored output | `build/`, `build_new/`, `runtime/build/`, `runtime/build_new/`, logs, caches — left in place |

Recovery is a single command:

```
git show refs/backup/fcc-wip-20261124        # the whole tree as FCC left it
git diff 6ac9791f refs/backup/fcc-wip-20261124
```

The reference is local to this clone. It is not pushed, and it must not be deleted until the stash and the
untracked reports have been reviewed by their author.

### Was FCC still running?

No evidence of a live FCC process was found, and no file in the tree changed while the inventory was taken
(`git status` was identical before and after). The upstream rate-limit error therefore did stop its local work.
The snapshot was still taken first, because "no evidence" is not proof.

---

## 2. Correcting FCC's own report

`FCC_FINAL_REPORT.md` describes HEAD `28079a0a` and an Issue #70 commit `7b59b0ad`. Both objects exist and
**both are ancestors of canonical `main`** (`git branch --contains 7b59b0ad` lists `main`, `origin/main`,
`gitgud/main`, `gitlab/main`). Its branch/test/"working tree clean" statements describe an older session, not the
tree that was handed over. The report is now committed with a dated banner; it is historical evidence, not a
status document.

Conversely, `ISSUE_37_PHASE1_COMPLETION.md` and `DEPLOYMENT_ISSUE37.md` name commit `314f3b2e` as the build they
deployed, but `renderer_profiling.h`, `renderer_profiling.cpp`, the prewarm changes and the profiling docs were
**untracked** in that commit's tree. The `X:\EmblemWeave` deployment was therefore built from a dirty tree and
cannot be reproduced from `314f3b2e`. Those files are now committed here, and the deployment hashes are labelled
as a local record.

This distinction matters for release work: a hash recorded against a commit that does not contain the code is not
provenance.

---

## 3. Recovered work, by area

Legend: **Integrated** = on `main` after this pass; **Integrated + fixed** = integrated, but a defect found during
review was corrected; **Preserved** = kept (stash/snapshot) but not applied; **Recorded** = documentation only.

### 3.1 Renderer profiling and diagnostics (Issue #37 Phase 1)

| Work | Files | Decision |
|---|---|---|
| Profiling API (11 metric families, RAII timers, macros, gauges) | `include/nxrt/renderer_profiling.h` | Integrated |
| Profiling implementation (percentiles, CSV, JSON, gauges) | `src/gfx/renderer_profiling.cpp` (was `src/diag/`) | Integrated + fixed (see CR-2, CR-3, CR-4, CR-6) |
| Memory/resource snapshot | `src/gfx/diag_memory_snapshot.cpp` (extracted from `registry.cpp`) | Integrated + fixed (CR-9) |
| Device-lost report enrichment (video memory, gauges, memory snapshot) | `src/gfx/vk_renderer.cpp` | Integrated |
| Shader/pipeline/texture/upload/sync/submission timers | `nvn_pipeline.cpp`, `nvn_gpu.cpp`, `nvn_gpu_texture.cpp`, `nvn_astc_gpu.cpp`, `nvn_replay.cpp`, `nvn_present.cpp` | Integrated |
| VMA pressure gauges | `src/gfx/vk_device.cpp` | Integrated |
| `[gfx] frame_wait_ms` (bounded present waits) | `src/gfx/vk_renderer.cpp`, `docs/CONFIG.md` | Integrated |
| Profiling and prewarming references | `docs/audits/ISSUE_37_*.md`, `DEPLOYMENT_ISSUE37.md` | Recorded (banners, corrected paths) |

### 3.2 Bounded pipeline prewarming

| Work | Files | Decision |
|---|---|---|
| Queue bound, metadata priority, per-frame re-prioritisation | `nvn_prewarm.cpp`, `nvn_pipeline.h`, `nvn_present.cpp` | Integrated + fixed (CR-7) |
| Draw-time key notes (`prewarm_note_key_request`) | `nvn_pipeline.cpp` | Integrated |
| Deferred-destruction drain for scene transitions | `nvn_gpu.cpp`, `nvn_gpu_internal.h`, `nvn_backend.h` | Integrated + fixed (CR-5) |

### 3.3 Platform write-watch split and Linux

| Work | Files | Decision |
|---|---|---|
| Windows write-watch moved out of `src/mem/` | `src/platform/win32/write_watch_win32.cpp` | Integrated (CR-1) |
| Linux write-watch (`userfaultfd` + `mprotect`) | `src/platform/posix/write_watch_linux.cpp` | Integrated as source, **not compiled or tested** (see §5) |

### 3.4 Long-session stability (Issues #37/#41)

| Work | Files | Decision |
|---|---|---|
| Audio stream registry purge on shutdown | `src/services/audio_out.cpp` | Integrated |
| Concurrent large-entry decode limit for the packed RomFS cache | `src/services/fs_romfs.cpp`, `src/services/packed_cache.h` | Integrated |
| Memory snapshot for transition comparisons | `src/gfx/diag_memory_snapshot.cpp` | Integrated |

### 3.5 Launcher reliability

| Work | Files | Decision |
|---|---|---|
| `CREATE_NO_WINDOW` for build/probe subprocesses | `apps/emblemweave/src/process.rs` (+ `main.rs`, `bin/ew_storage.rs`, `engine.rs`, `build_versioning.rs`, `toolchain_manager.rs`) | Integrated (CR-10) |
| Elapsed-time label without a double "left" suffix | `apps/emblemweave/src/shell.rs` | Integrated |

### 3.6 Preserved, deliberately not integrated

| Work | Where | Reason |
|---|---|---|
| Audio EQ, MAKO upscaler, overview page, play UI/docs WIP (45 files, 1,412 insertions) | `stash@{0}` | Unverified draft that overlaps live files (`engine.rs`, `main.rs`, `CONFIG.md`, `output_pass.*`, `play_*`). Applying it on top of an unbuilt tree would have mixed two unverified change sets and made any build failure unattributable. It stays in the stash for its author or a dedicated follow-up. |
| Issue #70 dav1d flags | already on `main` (`7b59b0ad`) | Nothing to integrate; status corrected to implemented-unverified. |
| `FCC_FINAL_REPORT.md`, `ISSUE_70_RESOLUTION.md` | repository root | Recorded with banners as historical evidence. |
| `REKARINA_PERFORMANCE_COMPARISON.md`, `REKARINA_PERFORMANCE_RESULTS.md` | `docs/audits/` | Recorded with banners; no performance claim is adopted. |

---

## 4. Freebuff work retained

All earlier Freebuff work is untouched and remains on `main`:

- Save-editor name and localisation improvements, background save-name loading (`bb2056f7`, `a40583ac`, `827f09f4`).
- Bundled FFmpeg runtime install (`113cc8fa`).
- Release documentation and provenance corrections (`6646b5f7` … `6ac9791f`).
- Save discovery/recovery and empty-save guarding (`42d9a612`, `b33564ec`, `314f3b2e`, `7df8f93e`).
- Launcher and toolchain reliability fixes already on `main`.
- The read-only Issue #79 fixture harness (`tools/issue79_fixture.py`) and its documentation.

No Freebuff change was reverted to make room for FCC's. The only files both sides touched were
`apps/emblemweave/src/engine.rs` and `main.rs`, where FCC's edits are additive (`mod process;` and `hidden()`
call sites) and do not overlap Freebuff's save-name work.

---

## 5. Known gaps in the recovered work

1. **Linux is not a build target yet.** `nxrt` still links `psapi` unconditionally, `src/diag/registry.cpp`
   includes `<windows.h>`, and the toolchain resolves MSVC/clang-cl. `write_watch_linux.cpp` is therefore
   source-only: it has never been compiled on Linux. Its logic may be sound, but until the core builds there it is
   unverified code. Proton/Gamescope and Steam Deck are untouched.
2. **Profiling overhead is not measured.** FCC estimated "<0.1% frame time"; no measurement exists. The timers are
   behind a config flag and disabled by default, so the release path is unaffected.
3. **The frame-time window now has a producer.** Before this pass `note_frame_time()` had no caller, which is why
   the committed Rekarina results show `P50/P95/P99 = 0.000`. Anyone citing those zeros as "fast" was reading a
   metric bug.
4. **Prewarm priority is unvalidated.** It compiles and is bounded, but no run shows it reduces first-visit stalls.
5. **The `X:\EmblemWeave` deployment is local-only.** It was not rebuilt in this pass and is not canonical.

---

## 6. Attribution and licensing

FCC's code keeps its GPL-3.0-or-later SPDX headers. The profiling and prewarm work is original to this repository.
No MAKO frame-generation weights or other restricted assets were imported: the MAKO references in the stash are
upscaler plumbing only, and the stash is not merged. The Rekarina documents are analysis of a GPL project, not
copied code; no Rekarina source was added.
