# Engine and Renderer Validation

**Date:** 2026-11-24
**Source state:** the working tree that became this documentation's commit on `main` (base `6ac9791f`, plus the FCC
integration). The tree was dirty at build time; the commit hash of the integrated result is recorded in
`PROJECT_CONSOLIDATION_REPORT.md`.
**Scope:** a clean Windows build of the production runtime (`engine/fefw-recomp`) and its test suites, plus the Rust
launcher's tests. **No game content was available, so no gameplay, save, cinematic or audio behaviour was
exercised.**

---

## 1. Environment

| Item | Value |
|---|---|
| OS | Windows (x86-64) |
| CMake | 4.4.4 |
| Generator | Ninja 1.13.2 |
| C/C++ compiler | clang 23.1.5 (`engine/fefw-recomp/tools/bin/clang-cl.exe`, target `x86_64-pc-windows-msvc`) |
| Linker / manifest / RC | `lld-link.exe`, `llvm-mt.exe`, `llvm-rc.exe` (bundled in `tools/bin`) |
| Dependency scanner | `clang-scan-deps` 23.1.5 (system LLVM; the bundled `tools/bin` copy is absent) |
| NASM | 3.02 (`C:/Users/xiond/AppData/Local/bin/NASM/nasm.exe`), used by dav1d |
| Build type | `Release` (`/O2 /Ob2 /DNDEBUG`) |
| Build directory | `engine/fefw-recomp/runtime/build_int` (fresh; now in `.gitignore`) |
| Rust | `cargo test` for `apps/emblemweave` |

### Configure command

```bash
cd engine/fefw-recomp/runtime
TB="$PWD/../tools/bin"
cmake -S . -B build_int -G Ninja \
  -DCMAKE_BUILD_TYPE=Release \
  -DCMAKE_C_COMPILER="$TB/clang-cl.exe" -DCMAKE_CXX_COMPILER="$TB/clang-cl.exe" \
  -DCMAKE_LINKER="$TB/lld-link.exe" -DCMAKE_MT="$TB/llvm-mt.exe" -DCMAKE_RC_COMPILER="$TB/llvm-rc.exe" \
  -DCMAKE_C_COMPILER_CLANG_SCAN_DEPS="C:/Program Files/LLVM/bin/clang-scan-deps.exe" \
  -DCMAKE_CXX_COMPILER_CLANG_SCAN_DEPS="C:/Program Files/LLVM/bin/clang-scan-deps.exe" \
  -DNXRT_DEPS_DIR="C:/Users/xiond/AppData/Local/nxrt/deps" \
  -DNXRT_LIBVPX=ON \
  -DNXRT_LIBVPX_INCLUDE="C:/vcpkg/installed/x64-windows-static/include" \
  -DNXRT_LIBVPX_LIB="C:/vcpkg/installed/x64-windows-static/lib/vpx.lib"
```

### Build command

```bash
cmake --build build_int --parallel 8
```

**Result:** `1026` targets built, exit code `0`, no compiler errors. A second incremental build after the last
prewarm edit also exited `0`.

FFX frame-generation was not available (`third_party/amd_ffx_api_fg/ffx_api.lib` absent), so `framegen_ffx.cpp`
was excluded by the project's own feature gate — a configuration fact, not a failure of this build.

---

## 2. Artifacts

| Artifact | Size | SHA-256 |
|---|---|---|
| `mpr_launcher.exe` | 3,763,712 | `c9b0304c9f21980690a1dbeed723c01e7d206dd9e5556c38dbc627b634cf211b` |
| `nxrt_tests.exe` | 26,351,104 | `8a06cd04abf488c951f465984cd3680a47b1c26344ae8560a9a3854eaf4d76b6` |
| `nxrt_gfx_tests.exe` | — | `943cc446311c918a9e38bab4b3ef71cbf882da8ed45028eb4bb68f33139071e0` |
| `nxrt_gfx_demo.exe` | — | `6a9ff4027e4d8e1d28809a924d27b6d8b49bb75040cd395bbcb397ff4f5669c4` |

These describe **this** build. They are not release artifacts, not comparable to the hashes in
`engine/fefw-recomp/runtime/DEPLOYMENT_ISSUE37.md` (a different, unreproducible build), and no package was made.

---

## 3. Test results

### 3.1 Launcher (Rust)

```bash
cargo test --manifest-path apps/emblemweave/Cargo.toml
```

| Result | Value |
|---|---|
| Tests | 442 |
| Passed | 429 |
| Failed | 0 |
| Ignored | 13 |

Two tests more than the previous baseline (440 / 427 / 13): the new `process` module's
`hidden_applies_creation_flags_on_windows` and `new_hidden_creates_command`.

### 3.2 Engine (C++)

```bash
ctest --test-dir build_int --parallel 4 --output-on-failure --timeout 900
```

| Result | Value |
|---|---|
| Tests | 182 |
| Passed | 169 |
| Failed | 13 |

Every failure is classified below. **None was excluded, renamed, muted or relabelled to change the totals.**

| # | Test | Symptom | Classification | Evidence |
|---|---|---|---|---|
| 29 | `soak` | SEGFAULT at `gauge_callback_and_counter` | **Pre-existing crash** | The pre-integration binary (`build_new/nxrt_tests.exe`, built before this integration) segfaults identically (exit 139) |
| 57 | `svc_info` | SEGFAULT at `info_thread_tick_count_pseudo_handle_counts_cpu` | **Pre-existing crash** | Same pre-integration binary, exit 139 |
| 59 | `svc_memory` | SEGFAULT after `MapPhysicalMemory` returns `0xD401` | **Pre-existing crash** | Same pre-integration binary, exit 139 |
| 76 | `tt_live` | Fails without a full GPU context | **Pre-existing** | Pre-integration binary exits 4 on the same test |
| 79 | `write_watch` | 2 of 14 checks: a foreign fault expected `PAGE_READONLY` sees `PAGE_READWRITE`; one guard-boundary write path | **Pre-existing assertion drift** | `src/platform/win32/write_watch_win32.cpp` is **content-identical** to `HEAD:src/mem/write_watch.cpp` (`diff --strip-trailing-cr` = 0 lines); the pre-integration binary reports the same 12 passed / 2 failed |
| 80 | `extras_output` | 7 of 7 require `r.dev->validation_enabled()` | **Environment: Vulkan validation layer not installed** | Assertions name `validation_enabled()`; the same tests report `validation: 0 errors, 0 warnings` from their own checks |
| 84 | `gfx_device` | 1 of 12, validation layer | **Environment** | as above |
| 112 | `gfx_nvn_mask_clear` | 3 of 3, validation layer | **Environment** | as above |
| 116 | `gfx_nvn_present_pass` | 5 of 5, validation layer | **Environment** | as above |
| 133 | `gfx_nvn_subgroup` | 3 of 7; device subgroup size 64 where the test expects 32 | **Hardware dependency** (AMD GPU) | The test states the expectation; this machine reports 64 |
| 137 | `gfx_nvn_texgpu` | 22 of 23, validation layer | **Environment** | as above |
| 155 | `launcher_pad_capture` | 1 of 4: `poll_reports_the_first_press_on_a_virtual_gamepad` | **Environment: no gamepad / virtual pad driver** | The other three capture tests pass |
| 15 | `kernel_starvation` | Failed under `ctest --parallel 4`, passed alone | **Load-sensitive test** | Isolated run: 1 passed / 0 failed; the test needs a thread to actually be starved for 3 s, which four parallel test processes change |

### 3.3 Tests corrected in this pass

Two failures were not environmental and were fixed at the source of the mismatch, not by relaxing the assertion:

1. **`kernel_threads` (was 27/29).** After the platform refactor, `sched_host_priority()` returns
   `nxrt::platform::ThreadPriority` (a band enum), while the test's expected tables still returned Win32
   `THREAD_PRIORITY_*` constants. The observed values (5, 4, 3, 0) were exactly the enum's band order, so the
   product mapping was right and the test was stale. The helpers now express the same six-band model in the enum,
   and a new `win32_of_band()` helper checks the Win32 priority that `thread_set_priority` actually sets. **29/29
   pass.**
2. **`launcher_game_info` (was 22/24).** `GameStatus::summary()` appends "Warning: cinematic movie and voice assets
   not found." when the RomFS has no movie/voice assets — an intentional #70/#75 aid. The test's fixture had no such
   assets and expected the bare "Ready to play.". The fixture now provides them, a new test
   `summary_warns_when_cinematic_assets_are_missing` covers all three warning variants, and
   `game_dir_finished_setup_is_ready_to_play` asserts `movies`/`voices` are detected. **25/25 pass.**

The two fixes add one test and correct three expectations; they do not remove coverage.

---

## 4. What the renderer work actually changed

Reviewing FCC's change set turned up five defects that would have made the new instrumentation wrong or unbuildable.
They are recorded in `CONFLICT_RESOLUTION_REGISTER.md` (CR-2 … CR-8); in short:

- the profiling implementation moved into `nxrt_gfx` (it reads VMA gauges; core does not link them);
- a duplicate `g_ring_bytes` and its permanently-zero gauge were removed;
- render-phase timers now write to their own counters instead of unrelated hitch buckets;
- `drain_pending_destroy()` no longer resolves to itself;
- frame-time sampling, GPU-memory refresh and the periodic CSV row are now actually called from the present path,
  where before nothing produced them.

**No performance claim is made.** No benchmark was run: the frame-time window, the CSV and the soak sampler are
now fed, which is what makes a before/after measurement possible; the measurement itself needs game content.
"Compiles and is wired" is not "faster".

---

## 5. Functional testing that was not possible

No game data was available and no game session was started. Therefore none of the following was observed:

- launcher startup → engine discovery → build → launch;
- game-data validation and portable install;
- save creation, save reload, profile handling (#79);
- mod discovery;
- shader/pipeline cache reuse across runs;
- audio playback, cinematic AV1+Opus playback or A/V sync (#70, #75);
- return-to-title transitions (#77);
- fullscreen/HDR restore (#78);
- long-session memory behaviour (#37, #41);
- diagnostic report generation from a real crash.

The unit tests exercise the pieces around these paths (cache logic, save/profile parsing, HDR maths, prewarm
bookkeeping), but **a passing unit test is not the behaviour**. Nothing above may be described as working on the
strength of this build.

---

## 6. Linux

Not built, not tested, and not claimed:

- `nxrt` links `psapi` unconditionally (`CMakeLists.txt`) and `src/diag/registry.cpp` includes `<windows.h>`;
- the configured toolchain is clang-cl/MSVC and the platform glob selects `src/platform/win32`;
- no Linux machine or container was available in this environment.

`src/platform/posix/write_watch_linux.cpp` was added by FCC and is kept, but no target compiles it. Proton,
Gamescope and Steam Deck were not tested in any form.

---

## 7. Reproducing this record

```bash
# engine
cd engine/fefw-recomp/runtime
cmake --build build_int --parallel 8
ctest --test-dir build_int --parallel 4 --output-on-failure --timeout 900

# launcher
cd ../../..        # repository root
cargo test --manifest-path apps/emblemweave/Cargo.toml
```

Expected: build exit 0; 169/182 engine tests pass with the 13 classified failures above; 429/442 launcher tests
pass with 13 ignored. The `kernel_starvation` result depends on machine load; run it alone if it fails in the
parallel run.
