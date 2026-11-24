# Project Consolidation Report

**Date:** 2026-11-24
**Branch:** `main` (no feature branch, no pull request, no force push, no history rewrite)
**Base at the start:** `6ac9791f2b60b40a5b52b3bb6350ead4fae58dcb`
**Recovery point for FCC's tree:** `refs/backup/fcc-wip-20261124` = `8c157698c07db3032aba9b381e5a02f7bc33acf8`

---

## 1. What was handed over

FCC stopped on an upstream rate-limit error with a mixed working tree on top of canonical `main`:

- **32 modified files**, among them the whole NVN renderer front end, the Vulkan device/renderer layer, audio and
  RomFS services, the diagnostics registry and six launcher source files.
- **1 deletion:** `src/mem/write_watch.cpp`.
- **15 untracked paths:** the renderer-profiling header and implementation, the new platform write-watch pair, the
  launcher's `process.rs`, benchmark and diagnostic scripts, two engine audit documents, three top-level reports,
  two Rekarina reports, and a deployment record.
- **1 stash:** "pre-PR WIP: audio EQ, MAKO, play UI, docs" (45 files, 1,412 insertions).
- Two worktrees (the main checkout and an Issue #79 test fixture).
- Build output and logs under ignored paths.

Full inventory and the per-file decision table are in `FCC_WORK_INTEGRATION.md`.

### Recovery

A commit object was created from a temporary index (`git read-tree HEAD`, `git add -A`, `git write-tree`,
`git commit-tree`) so that every tracked and untracked file was captured **without touching the working tree**, and
stored under `refs/backup/fcc-wip-20261124`. Nothing was reset, cleaned, reverted or overwritten. The stash and both
worktrees are untouched.

### One correction to the handover

FCC's `FCC_FINAL_REPORT.md` describes HEAD `28079a0a`; that commit and its Issue #70 commit `7b59b0ad` are
**ancestors of `main`**. The report is a stale snapshot. Its claims are now bannered rather than treated as status.
Conversely, the Issue #37 Phase 1 work it described as "deployed and verified" was **untracked** at `314f3b2e`, so
that deployment cannot be reproduced from the commit it names.

---

## 2. What was integrated

| Area | Integrated | Notes |
|---|---|---|
| Renderer profiling (11 metric families, CSV/JSON, gauges) | Yes | Moved into `nxrt_gfx`; frame-time sampling, GPU-memory refresh and the periodic CSV row wired into the present path |
| Memory/resource snapshot | Yes | New `src/gfx/diag_memory_snapshot.cpp`; appended to the GPU device-lost report |
| Bounded pipeline prewarm + metadata priority | Yes | `prewarm_recency_decay` now honoured; queue bound enforced |
| Deferred-destruction drain | Yes | Self-recursive wrapper fixed |
| Platform write-watch split (Win32 + Linux source) | Yes | Byte-identical move on Windows; Linux file kept but not compiled |
| VMA pressure gauges, `[gfx] frame_wait_ms` | Yes | Documented in `docs/CONFIG.md` |
| Audio registry purge, packed-cache decode cap | Yes | #37/#41 mitigations, unverified |
| Launcher console suppression (`process.rs`) | Yes | Two new unit tests |
| Elapsed-time label fix | Yes | — |
| Audio EQ / MAKO / overview / play UI WIP | **No — preserved in `stash@{0}`** | Unbuilt draft overlapping live files; see CR-11 |

Freebuff's earlier work (save names and localisation, background save-name loading, bundled FFmpeg install,
release/provenance docs, save discovery and empty-save guards, launcher reliability) is untouched.

### Defects found and corrected during integration

Nine review findings, all recorded in `CONFLICT_RESOLUTION_REGISTER.md`: two link-level problems (profiling in the
core library, duplicate `g_ring_bytes`), one link inversion for the memory snapshot, a self-recursive wrapper, a
phase-timer mismatch that would have corrupted existing hitch diagnostics, two metrics with no producer, an ignored
config key, and stale test expectations in two suites.

---

## 3. Verification performed

See `ENGINE_AND_RENDERER_VALIDATION.md` for the full record, commands, hashes and failure classification.

| Check | Result |
|---|---|
| Clean configure (fresh `build_int`) | Success |
| Full engine build (1,026 targets) | Success, exit 0, no errors |
| Engine test suite (182 tests) | **169 passed, 13 failed**, all classified: 4 pre-existing crashes/failures reproduced on the pre-integration binary, 6 need the Vulkan validation layer or a specific GPU, 1 needs a gamepad, 1 is load-sensitive and passes alone |
| Launcher test suite (442 tests) | **429 passed, 0 failed, 13 ignored** |
| Two stale test suites corrected | `kernel_threads` 27/29 → 29/29; `launcher_game_info` 22/24 → 25/25 |

**Not performed:** any game launch, save load, cinematic, audio session, return-to-title cycle, HDR restore,
long-session soak, packaging, or Linux build. No game data was available.

---

## 4. Release position

No release tag, GitHub release, deployment or package was produced. `main` stays on 0.1.42 development metadata with
0.1.41 as the public release. `NEXT_RELEASE_READINESS.md` records the blockers; the short version is that no
user-visible fix in this cycle has been demonstrated in a running game, and Issue #79 is still the critical path.

---

## 5. Commits

The integration was committed in logical groups directly on `main`; the hashes are recorded in the final commit of
this sequence and summarised here:

| Commit | Subject | Contents |
|---|---|---|
| `3b4d9531e380a067dd6359a6d1862145f0b9411b` | engine: integrate FCC's renderer profiling, bounded prewarm and write-watch split | renderer profiling (moved to `nxrt_gfx`), memory snapshot, prewarm bounds, platform write-watch split, services, `[gfx] frame_wait_ms`, CONFIG rows, `kernel_threads` expectations |
| `1a827515c357bd7851453c3f2c6e463ce9799b4e` | launcher: hide build subprocess consoles and cover the cinematic warning | `process.rs` and its call sites, elapsed-time label, `launcher_game_info` fixtures |
| `9cf691802b5c5e1b618c64074ce8dcd41287693f` | tools: add the read-only #79 save-fixture harness | `tools/issue79_fixture.py` |
| `3e10ad47dcce41ce27cfb9b71a56420f7035ec32` | docs: reconcile the v0.1.42 notes with the integrated tree | the six reports, FCC report banners and path corrections, five-note sweep, readiness-audit entries |
| this commit | docs: record the integration commits and the validated build | this commit table, manifest source/build/tests, `.gitignore` for build output |

`Refs`-style references are used in commit messages; no `Fixes`/`Closes`/`Resolves` trailer was written, because
nothing in this cycle was verified against a running game.

---

## 6. Remaining work

1. **Issue #79 run.** Execute `tools/issue79_fixture.py` for rows 16, 17, 19, 20 and 22, then launch the game against
   each fixture and load the save. This is the release-critical item.
2. **Cinematics.** One AV1 cutscene with Opus audio to close #70/#75, with frame times and decoder counters recorded.
3. **A soak.** Two hours with `[diag] soak_csv` and `[prof] renderer_profiling`, ten scene transitions, comparing
   working set, GPU memory and deferred-destruction depth (#37/#41).
4. **A clean baseline build at the pre-integration commit** to convert the "reproduces on the pre-integration
   binary" evidence for `soak`, `svc_info`, `svc_memory` and `write_watch` into a formal bisect, and to triage the
   three segfaulting tests properly.
5. **Linux platform layer.** Guard `psapi`, `<windows.h>` in `registry.cpp` and the toolchain selection, then compile
   and test `write_watch_linux.cpp` natively.
6. **The stash.** Evaluate the audio EQ / MAKO / overview work on top of today's `main`, in its own change set.
7. **HDR restoration (#78).** Capture and restore the original Windows HDR state, SDR white level and display mode.
8. **A real trace of the profiling overhead** against a build with `[prof] renderer_profiling` off.
