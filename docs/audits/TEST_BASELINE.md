# Current Test Baseline — 2026-11-25

This is the current, reproducible test baseline for the engine tree. It replaces the
historical failure table in `docs/audits/v0.1.42/TEST_KNOWN_ISSUES.md` for status
reporting; that file remains a historical record.

**Repository:** `XiONDiGiTaL/EmblemWeave`  
**Branch:** `main`  
**Base inspected:** `7d98b921`  
**Post-fix commit:** `87524219` (`FrameTimeWindow::snapshot` correctness fix and tests)  
**Build directory:** `engine/fefw-recomp/runtime/build_new`  
**Compiler:** bundled `clang-cl`, Ninja, Windows x86-64

## Results

| Suite | Passed | Failed | Skipped | Total | Notes |
|---|---:|---:|---:|---:|---|
| `nxrt_tests.exe --all` | 989 | 0 | 29 | 1018 | Full core/runtime suite |
| `nxrt_gfx_tests.exe --all` | 617 | 40 | 118 | 775 | Includes the 4 new profiling tests; #106 fixed |
| `mpr_launcher_tests.exe` | 66 | 1 | 2 | 69 | Rebuilt before running |

### Graphics failures

All 41 failing graphics tests fail the `validation_enabled()` assertion because
`VK_LAYER_KHRONOS_validation` is not installed. The device log says:

```
gfx: validation requested but VK_LAYER_KHRONOS_validation is not installed (Vulkan SDK): continuing without
```

This is an environment dependency, not evidence that the rendering paths work. Install
the Vulkan SDK (or provide a CI image with the layer) and rerun the suite; the assertion
is an oracle, not a cosmetic check.

**Fixed (2026-11-26):** `gfx_nvn_texgpu.compact_standin_upload_and_replacement` now passes.
The root cause was a mismatch in the test's `read_back_tight` helper: compact stand-in
levels store distinct block rows in a packed layout (`PreparedLevel::rows` holds absolute
offsets per layer and block row), but the readback assumed a contiguous per-layer layout.
The fix makes the readback use `pl.rows` for compact levels, matching the upload path
in `nvn_gpu_texture.cpp`. Both 2D and 2D-array cases now pass.

### Launcher failure

`launcher_pad_capture.poll_reports_the_first_press_on_a_virtual_gamepad` fails because
there is no physical or virtual gamepad providing a press in this environment. The
`launcher_game_info` failures reported by the historical FCC report do not reproduce
after rebuilding `mpr_launcher_tests`; both game-info tests pass.

## Targeted checks

| Check | Result | Evidence |
|---|---|---|
| `write_watch` | 17/17 | Includes `import_freeze_holds_writes_until_it_ends` |
| `kernel_sync` | 27/27 | No heap-corruption assertion |
| `soak` | 9/9 | No access violation |
| `svc_fs_romfs` | 21/21 | Fixture path present |
| `svc_info` | 25/25 | No thread-tick crash |
| `svc_memory` | 21/21 | AMD mapping path passes |
| `tt_live` | 2/2 | Present path passes |
| `media_ffmpeg` | 4/4 | Raw AV1, av1c extradata, d3d11va request path |
| `gfx_renderer_profiling` | 4/4 | Empty/partial/wrapped window and CSV shape |
| `gfx_nvn_prewarm_record` | 3/3 | Record/filter/refusal paths |
| `gfx_nvn_prewarm` | 0/0/4 skipped | Requires `work/shader_cache` |

The former `kernel_sync`, `soak`, `svc_*`, `tt_live`, `write_watch` and
`launcher_game_info` failures in the FCC report are superseded: they pass at the current
base. That does not mean they were fixed by this review; the tree and tests changed
after the report.

## Build facts relevant to Issue #70

- `CMakeCache.txt`: `NXRT_DAV1D_ASM=ON`, `NXRT_DAV1D_ASM_AUTO=ON`.
- NASM was found and dav1d assembly objects are present in the build.
- `media_ffmpeg` passes 4/4.
- No cutscene has been played end to end; this is build/test evidence only.

## Commands

Run from `engine/fefw-recomp/runtime/build_new`:

```sh
./nxrt_tests.exe --all
./nxrt_gfx_tests.exe --all
./mpr_launcher_tests.exe

./nxrt_tests.exe write_watch
./nxrt_gfx_tests.exe gfx_renderer_profiling
./nxrt_gfx_tests.exe gfx_nvn_texgpu.compact_standin_upload_and_replacement
./nxrt_tests.exe media_ffmpeg
```

Do not compare these totals with the historical FCC `166/14/5` table: it was produced
by a different `ctest` inventory before the integrated tree and the new tests existed.
