# Report Action Register — FCC, Issue #37, Issue #70

**Status:** active engineering register  
**Repository:** `XiONDiGiTaL/EmblemWeave`  
**Reviewed:** 2026-11-25  
**Base inspected:** `main` @ `7d98b921` (the commit the reports described)  
**Implementation commit from this review:** `87524219`  
**Current test evidence:** `docs/audits/TEST_BASELINE.md`

The three source reports (`FCC_FINAL_REPORT.md`, `ISSUE_37_PHASE1_COMPLETION.md`,
`ISSUE_70_RESOLUTION.md`) were committed as a historical snapshot in
`3e10ad47dcce41ce27cfb9b71a56420f7035ec32`. They carry dated banners and are not status
or release documents. Their actionable content is transferred here; the files themselves
are removed in the separate cleanup commit that follows this register. Their original text
remains available from Git history at `3e10ad47` and from the consolidation records in
`docs/audits/`.

## Status key

| Code | Meaning |
|---|---|
| `V` | Complete and independently verified in this review |
| `I` | Implemented, but not adequately validated end to end |
| `P` | Partially implemented; a known part is missing |
| `O` | Outstanding implementation |
| `T` | Outstanding test, measurement or validation |
| `B` | Blocked by missing hardware, data, dependency or external evidence |
| `S` | Superseded or no longer applicable; historical claim retained for provenance |

## Current evidence baseline

- Core: `nxrt_tests --all` — **989 passed, 0 failed, 29 skipped**.
- Graphics: `nxrt_gfx_tests --all` — **616 passed, 41 failed, 118 skipped**.
  All 41 fail the missing `VK_LAYER_KHRONOS_validation` assertion; one also fails two
  image comparisons, tracked as issue #106.
- Launcher: `mpr_launcher_tests` — **66 passed, 1 failed, 2 skipped**; the one failure
  needs a physical/virtual gamepad.
- Profiling: `gfx_renderer_profiling` — **4/4** after the frame-window fix.
- Media: `media_ffmpeg` — **4/4**; dav1d is built with NASM/assembly.
- Former FCC failures (`write_watch`, `kernel_sync`, `soak`, `svc_*`, `tt_live`,
  `launcher_game_info`) pass at the current base. See `TEST_BASELINE.md` for commands and
  caveats.

## FCC_FINAL_REPORT.md

| ID | Finding / required work | Status and evidence | Acceptance and dependencies | Owner / tracking | Release class |
|---|---|---|---|---|---|
| FCC-01 | Issue #70 cinematic playback is implemented but runtime verification is pending. Test AV1 decode, Opus audio, A/V sync and 1440p smoothness. | `B` — source and build verified; `media_ffmpeg` 4/4; no cutscene played end to end. | Play at least one AV1+Opus cutscene at 1440p; record frame times and 60 s A/V sync; attach log and CSV. Needs authorized game data and a runtime session. | engine/media; #70, #75 | Release blocker |
| FCC-02 | Document the 14 historical failures and their categories instead of suppressing them. | `V` — current baseline documented in `TEST_BASELINE.md`; failure count and causes differ from the report. | Keep the baseline updated for each release candidate; rerun after validation-layer installation. | docs/CI; this register | Follow-up |
| FCC-03 | Write-watch memory tracking was reported at 12/14; two fallback edge cases were open. | `S` — current `write_watch` is 17/17, including `import_freeze_holds_writes_until_it_ends`. | No code change required in this pass; retain the regression coverage. | platform; no new issue | Follow-up |
| FCC-04 | `kernel_sync`, `kernel_threads`, `soak`, `svc_fs_romfs`, `svc_info`, `svc_memory`, `tt_live` were reported failing. | `S` — targeted groups pass and full core is 0 failed. The report describes a different tree/test inventory. | No action beyond keeping the groups in CI. | runtime; no new issue | Follow-up |
| FCC-05 | `launcher_game_info` had two "Ready to play." string failures. | `S` — after rebuilding `mpr_launcher_tests`, both game-info tests pass; only the controller test fails. | Rebuild tests before citing launcher results. | launcher; no new issue | Follow-up |
| FCC-06 | Validation-layer tests cannot run without `VK_LAYER_KHRONOS_validation` (Vulkan SDK). | `B` — 41 graphics tests fail the validation assertion; log names the missing layer. | Install the Vulkan SDK or use a CI image with the layer; rerun `nxrt_gfx_tests --all`; require zero validation assertions. | CI/gfx; release checklist | Release prerequisite |
| FCC-07 | Physical controller tests need a DualSense/Xbox pad. | `B` — `launcher_pad_capture` has no press source here. | Run with a physical pad or a deterministic virtual HID; record the device and result. | CI/launcher | Follow-up |
| FCC-08 | AMD subgroup test hardcodes 32-wide expectations; the RX 7900 XT reports native 64. | `O`/`T` — required-32 produces 0/64 mismatches; default-64 produces 64/64 mismatches but is not asserted. The test fails only the validation oracle. | Make the test query `VkPhysicalDeviceSubgroupProperties`; assert the guest-warp path with required 32 and either cover native 64 explicitly or skip with a reason. Re-run with the validation layer. | gfx/tests; this register | Follow-up |
| FCC-09 | Branch consolidation and policy: report claimed only `main` and no local feature branches. | `S` — `origin` had only `main`; local branches, worktrees and the stash are intentionally preserved. | Do not delete branches/worktrees to make the historical statement true. | maintainers | Not applicable |
| FCC-10 | Release notes must distinguish verified, unverified and known-limitation work. | `V` — `docs/changelog/0.1.42.md` and the readiness audits carry that distinction. | Recheck the notes against this register before publishing. | docs/release | Follow-up |
| FCC-11 | Before publishing: game-data cinematics, manual gameplay (#75-#79), Vulkan SDK, updated notes, tag. | `O`/`B` — none of the runtime items is complete; no tag or release was created. | Complete `FCC-01`, `FCC-13` through `FCC-16`, `FCC-06`; then follow `docs/audits/V0.1.42_RELEASE_VALIDATION_CHECKLIST.md`. | release owner | Release blocker |
| FCC-12 | `TEST_KNOWN_ISSUES.md` should carry the current failure documentation. | `V` — new `TEST_BASELINE.md`; the historical file is retained and explicitly marked historical. | Update the baseline after each validation run; do not rewrite the historical table. | docs/CI | Follow-up |
| FCC-13 | Save hardening (#79) was called merged with launcher tests at 22/24; field saves were not validated. | `I`/`P` — launcher tests pass; #79 remains open; user save data still reports unreadable saves. | Reproduce with the reporter's save/fixture; verify load, migration and autosave; acceptance is in #79. | launcher/persistence; #79 | Release blocker |
| FCC-14 | Cinematic AV1+Opus (#75) was called code-complete. | `I` — #75 remains open; no end-to-end playback evidence. | Same evidence as `FCC-01`; include the reporter's cutscene case. | engine/media; #75 | Release blocker |
| FCC-15 | Find Units scan (#76) was called merged with a benchmark script. | `I` — #76 remains open; no benchmark rerun or stutter measurement. | Run the Find Units path on game data; record scan time and audio/video stutter; acceptance in #76. | engine/gameplay; #76 | Follow-up / release candidate |
| FCC-16 | 10+ transitions (#77) was called merged. | `I` — #77 remains open; 10-cycle run exists on a working-tree build, not a clean build, and the reporter's save is not tested. | Clean-build rerun with the damaged autosave; verify no return-to-title crash; acceptance in #77. | engine/gameplay; #77 | Release blocker |
| FCC-17 | HDR restoration (#78) was called merged. | `P` — original-state restoration is still outstanding; multi-monitor and live-mode restore are unverified. | Verify SDR/HDR exit restore on at least one HDR display and one multi-monitor setup; document any read-only state. | gfx/display; #78 record | Follow-up |
| FCC-18 | Feature inventory claims for NVN pool import/ImportFreeze, launcher folder check, `--compare-data`, and Rust launcher compilation. | `I` — code and unit/launcher coverage exist; no runtime game-session evidence in this review. | Exercise each path with real game data; report any gap against the owning issue. | engine/launcher | Follow-up |
| FCC-19 | Historical totals were 166 passed / 14 failed / 5 skipped. | `S` — different `ctest` inventory; current baseline is in `TEST_BASELINE.md`. | Do not use the old totals for release status. | docs/CI | Not applicable |

## ISSUE_37_PHASE1_COMPLETION.md

| ID | Finding / required work | Status and evidence | Acceptance and dependencies | Owner / tracking | Release class |
|---|---|---|---|---|---|
| I37-01 | Eleven profiling metric categories were delivered (frame time, shader/pipeline, cache, textures, uploads, render phases, submissions, sync, present, GPU memory). | `I` — code is present and builds; new unit coverage exists only for the frame window and CSV shape. | Produce a runtime CSV and JSON snapshot with non-zero counters for each category; verify units and gauges. Needs game content. | engine/gfx; #37 | Follow-up |
| I37-02 | Frame-time P50/P95/P99 rolling window. | `V` — found and fixed the `head - n` indexing bug in `87524219`; 4/4 tests; full gfx run includes them. | Keep the partial-window and wraparound tests; runtime percentiles still need a game session. | engine/gfx; #37 | Follow-up |
| I37-03 | Shader/pipeline/upload/submission/sync/render-phase timers. | `I` — code wired; no runtime counter validation. | Run a gameplay session and cross-check timer totals against frame time and phase coverage; confirm no double counting. | engine/gfx; #37 | Follow-up |
| I37-04 | CSV header, per-second row and final row. | `I` — header/row shape is tested; actual file creation and periodic flush were not exercised. | Enable `[prof] renderer_profiling=1` and `renderer_csv`; verify header, one row per interval, final row on shutdown, no truncated columns. | engine/gfx; #37 | Follow-up |
| I37-05 | Bounded prewarming: queue bound, priority, workers, draw-time feedback, grace, bounded drops. | `I` — `gfx_nvn_prewarm_record` 3/3; `gfx_nvn_prewarm` 4/4 skipped because `work/shader_cache` is absent; benefit unmeasured. | Provide a shader-cache fixture or game session; measure hit rate, wait time, drops and frame-time effect. | engine/gfx; #37 | Follow-up |
| I37-06 | Seven-scenario benchmark harness. | `O`/`B` — `scripts/benchmark_issue37.py` exists, but benchmark modes are placeholders and need game integration. | Implement/verify each mode with real ExeFS/RomFS/main.nso; record scenario, build hash and CSV. | engine/tooling; #37 | Follow-up |
| I37-07 | Build verification for the profiling/prewarm work. | `V` — current build passes; `NXRT_DAV1D_ASM=ON`, NASM present, profiling tests pass. | Keep a clean-configure build in CI. | build/CI | Follow-up |
| I37-08 | SHA-256 hashes and `X:\EmblemWeave` deployment were called verified. | `S` — local deployment record only; the files were untracked at `314f3b2e`, so it is not canonical provenance. No deployment was repeated. | Do not cite the hashes as release provenance; build from a committed tree if a deployment record is needed. | release/docs | Not applicable |
| I37-09 | Run the seven scenarios and analyze CSV baselines. | `B` — requires game content and the benchmark integration in `I37-06`. | Produce baseline tables per scenario and retain raw CSV/logs. | engine/perf; #37 | Follow-up |
| I37-10 | Persistent prewarm metadata across runs. | `O` — not implemented. | Define the cache key/version, write atomically, invalidate on schema change; test cold/warm startup. | engine/gfx; #37 | Follow-up |
| I37-11 | GPU timestamp queries for GPU-side frame time. | `O` — not implemented. | Add timestamp queries with a capability check and fallback; compare with CPU frame time. | engine/gfx; #37 | Follow-up |
| I37-12 | Automated regression detection in CI. | `O` — no profiling regression gate. | Define thresholds from a stable baseline; run on a fixed GPU/CI runner; fail with the CSV artifact attached. | CI/gfx | Follow-up |
| I37-13 | Visualization tooling for CSV data. | `O` — not implemented. | Produce a small report/plot from the CSV; document usage; no release dependency. | tooling/docs | Follow-up |
| I37-14 | Estimated overhead <0.1% when enabled, zero when disabled. | `T`/`B` — estimate only; no measurement. | A/B the same scene with profiling off/on; report frame-time deltas and CSV I/O cost. Needs game content. | engine/perf; #37 | Follow-up |
| I37-15 | Prewarm queue tests should run, not skip. | `B` — 4 skipped because `work/shader_cache` is absent. | Provide the fixture or make the test create a minimal cache; acceptance is 4/4 without a game install. | engine/gfx; #37 | Follow-up |
| I37-16 | Linux write-watch/portability. | `O` — POSIX layer builds, but the runtime still has Win32 dependencies. | Track under #7; complete the platform layer and run the Linux self-test. | platform; #7 | Follow-up |
| I37-17 | Static linking only, ~25 MB test executables. | `S` — accepted build choice/known limitation, not a defect. | No action unless distribution size becomes a requirement. | build | Not applicable |
| I37-18 | Report said four pre-existing memory tests fail. | `S` — current core suite has 0 failures; those tests pass or changed. | None; keep the current baseline. | runtime/CI | Not applicable |
| I37-19 | Issue #37's original 2-hour audio/RAM leak and 0-frame freeze. | `B` — no long session was run; the Opus decoder leak fix is present but `audio_out`/`audren` lifetimes need a 2+ hour session. | Run a 2-hour session with memory/voice counters and scene transitions; acceptance in #37. | engine/audio; #37 | Release blocker |
| I37-20 | Report called Phase 1 "complete" and "deployment verified". | `S` — build and code exist; runtime benefit unmeasured and deployment provenance not canonical. | Use this register and the readiness audits for status; do not close #37 on the report's verdict. | docs/engineering | Not applicable |

## ISSUE_70_RESOLUTION.md

| ID | Finding / required work | Status and evidence | Acceptance and dependencies | Owner / tracking | Release class |
|---|---|---|---|---|---|
| I70-01 | dav1d per-compiler flags were broken by mixed MSVC/GCC options. | `V` — per-compiler flags are present in `third_party/dav1d/CMakeLists.txt`; build succeeds with assembly; `media_ffmpeg` 4/4. | Keep a compile test for MSVC/clang-cl and one non-MSVC compiler. | engine/media/build | Follow-up |
| I70-02 | NASM auto-detection, cache behavior and CONFIG documentation. | `V` for the default build — `NXRT_DAV1D_ASM=ON`, `NXRT_DAV1D_ASM_AUTO=ON`, NASM found, assembly objects present, docs rows present. | Keep the cache/config rows covered by a configure test. | engine/build/docs | Follow-up |
| I70-03 | Six CMake-cache scenarios for NASM detection. | `T` — only the default ON/AUTO build was exercised in this review. | Configure and build with ASM ON, OFF and AUTO, with NASM present and absent; record the chosen path and flags. | engine/build | Follow-up |
| I70-04 | Cinematic playback pipeline (dav1d/FFmpeg backend). | `I` — decoders and unit tests exist; no cutscene rendered. | Same evidence as `FCC-01`; include audio and A/V sync. | engine/media; #70 | Release blocker |
| I70-05 | FFmpeg D3D11VA hardware decode. | `B` — requires FFmpeg 8.x DLLs and a GPU with AV1 decode; falls back to dav1d. | Document the runtime requirement; if claiming hardware decode, test with the DLLs and report the selected decoder. | engine/media; release docs | Follow-up |
| I70-06 | Game-data integrity checks: `.nup` vs extracted folders, `--compare-data`, folder check. | `I` — code and launcher tests exist; no complete/incomplete game-data runtime matrix. | Run the launcher against complete, partial and mismatched data; verify the reported status and no crash. | launcher; #70 | Follow-up |
| I70-07 | No explicit warning when game data is incomplete or missing. | `O` — the setup check is only offered when `app.game.romfs.is_some()`; no explicit incomplete-data warning. | Add a validation summary/warning with actionable paths; test complete, partial and missing cases. | launcher; #70 | Follow-up |
| I70-08 | NASM assembly is claimed several times faster than C-only (30-50 ms vs 10-15 ms at 1440p). | `T`/`B` — not measured; requires two builds and the same cutscene. | Build with ASM OFF and ON; decode the same AV1 samples; report p50/p95 decode time and dropped frames. | engine/media/perf; #70 | Follow-up |
| I70-09 | Report claimed 984-target build and 91% (166/182) tests. | `S` — current build/test inventory differs; core is 989/0/29 and graphics is 616/41/118. | Use `TEST_BASELINE.md`; do not reuse the old percentage. | docs/CI | Not applicable |
| I70-10 | Report concluded Issue #70 was resolved. | `S` — #70 remains open; only build-level evidence exists. | Keep #70 open until `FCC-01`/`I70-04` evidence exists. | engine/media; #70 | Release blocker |

## Cross-report findings

| ID | Finding / required work | Status and evidence | Acceptance and dependencies | Owner / tracking | Release class |
|---|---|---|---|---|---|
| X-01 | The FCC report classified all failures as hardware/environment. The current run has 41 validation-layer assertion failures plus two real image comparisons in `gfx_nvn_texgpu.compact_standin_upload_and_replacement`. | `O` — issue #106 created with the exact test, byte offset and acceptance criteria. | Fix the array stand-in/full-transcode readback mismatch; verify on AMD and one non-AMD GPU; no validation errors. | engine/gfx; #106 | Release blocker for texture-upload confidence |
| X-02 | The report said the repository was clean and all valid work consolidated. | `S` — local branches, worktrees and `stash@{0}` are intentionally preserved; `origin` had only `main`. | Do not delete or rewrite preserved work to satisfy the historical claim. | maintainers | Not applicable |
| X-03 | The reports treated test counts and failure causes as stable facts. | `S`/`P` — counts and causes changed with the tree; the current baseline is authoritative for today. | Rerun and update `TEST_BASELINE.md` at each release candidate. | docs/CI | Follow-up |
| X-04 | The reports named HEAD `28079a0a`/`314f3b2e` and a "verified deployment" that did not contain the profiling files. | `S` — `3e10ad47` is an ancestor of the current `main`; the reports are historical snapshots and their deployment hashes are local records. | Preserve the provenance note; do not cite the hashes as canonical. | docs/release | Not applicable |
| X-05 | The three reports should be removed only after their actionable content is durable. | `V` — this register and `TEST_BASELINE.md` exist; issue #106 tracks the new defect; the cleanup commit is separate. | Delete only the three requested files; keep all audit records and Git history. | docs | Cleanup |

## Execution order and ownership

1. **Correctness first:** fix or fully triage #106 (texture array readback), then rerun
   the focused test and the graphics suite with the validation layer.
2. **Data integrity:** #79 save loading, then #77/#41 crash paths.
3. **Cinematics:** #70/#75 with authorized game data; capture playback, audio and A/V sync.
4. **Long-session stability:** #37 two-hour run with memory/audio counters and profiling.
5. **Gameplay/performance:** #76 Find Units, prewarm hit rate, profiling overhead.
6. **Build/test infrastructure:** CMake NASM matrix, validation-layer CI, controller
   fixture, AMD subgroup capability test.
7. **Follow-up enhancements:** persistent prewarm metadata, GPU timestamps, CI
   regression detection, CSV visualization.

No individual assignee is invented here. Owners are the subsystem roles above; the open
GitHub issues are the user-visible tracking. #37, #70, #75, #76, #77, #79, #41, #29, #59
and #7 were already open. #106 is the only new issue created by this review. This register
is the durable task list for engineering-only follow-ups that do not warrant duplicate
issues.

## Blocked evidence required to unblock

| Blocked area | Required evidence |
|---|---|
| Cinematics (#70/#75) | Authorized game data; play one AV1+Opus cutscene at 1440p; renderer CSV, engine log, 60 s A/V sync observation, frame-time summary. |
| Long session (#37) | 2+ hour gameplay session; memory/voice counters before/after scene transitions; no 0-frame freeze; audio underrun log. |
| Validation layer | Vulkan SDK or CI image with `VK_LAYER_KHRONOS_validation`; rerun the 41 graphics tests; attach the zero-error result. |
| Controller | DualSense/Xbox pad or deterministic virtual HID; record device and press events. |
| AMD subgroup test | Query `VkPhysicalDeviceSubgroupProperties`; verify required-32 and native-64 paths separately. |
| NASM performance | ASM ON vs OFF builds; same AV1 sample; decode p50/p95 and dropped-frame counts. |
| CMake matrix | Configure/build logs for ON/OFF/AUTO with NASM present and absent. |
| Prewarm benefit | Shader-cache fixture or game session; hit rate, waits, bounded drops and frame-time comparison. |
| Texture #106 | Array stand-in and full-transcode readbacks matching CPU references on AMD and a non-AMD GPU. |

## Provenance

- Source reports: `FCC_FINAL_REPORT.md`, `ISSUE_37_PHASE1_COMPLETION.md`,
  `ISSUE_70_RESOLUTION.md`, committed at `3e10ad47`; removed in the separate cleanup
  commit after this register was written.
- Reconciliation and current status: `docs/audits/PROJECT_CONSOLIDATION_REPORT.md`,
  `docs/audits/FCC_WORK_INTEGRATION.md`, `docs/audits/CONFLICT_RESOLUTION_REGISTER.md`,
  `docs/audits/ISSUE_REMEDIATION_STATUS.md`,
  `docs/audits/V0.1.42_RELEASE_READINESS.md`, and `docs/audits/TEST_BASELINE.md`.
- The reports' original text remains in Git history at `3e10ad47`; do not rewrite that
  commit.
