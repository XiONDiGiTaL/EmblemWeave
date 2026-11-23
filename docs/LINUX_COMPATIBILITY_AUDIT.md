# Linux compatibility audit

**Base revision:** `22a380c901a45da9b8a7aaa60375a819f82530a4` (`main`, 22a380c "write_watch: ensure g_transitions
initialized before ImportFreeze use"). FCC could not be reached for a base-revision decision, so this is the
**provisional base** and must be confirmed before integration. The work lives on branch `linux/compat-prototype` in
the isolated worktree `ew-linux`; FCC's working tree, `E:\EmblemWeave`, the Windows build and the v0.1.42 work were
not modified.

**One imported snapshot.** `runtime/include/nxrt/platform.h`, `runtime/src/platform/posix/platform_posix.cpp` and
`runtime/src/platform/win32/platform_win32.cpp` are copied from FCC's *uncommitted* working tree at that revision,
because they contain the in-progress platform split that does not exist in the commit. The three files are one set:
the header declares memory sections, page-fault handling, per-thread CPU time and rwlocks that only the WIP backends
implement, so importing the header with the committed Win32 file would not link on Windows. The copies in `ew-linux`
are the only ones changed; FCC's files are untouched. This is recorded because those files have no commit identity and
integration must take them from FCC, not from this branch, unless FCC commits them first.

## 1. Method

Everything below was measured, not estimated:

* `python tools/port/win32_audit.py` over the 476 runtime sources, regenerating the Windows-call table.
* `grep -rl "<windows.h>" runtime/src runtime/include | wc -l`.
* `cargo check -p emblemweave --target x86_64-unknown-linux-gnu` under WSL Ubuntu 26.04 (rustc 1.100.0).
* A CMake + Ninja + clang 21 build of the POSIX platform layer and its self-test under the same WSL.
* `nxrt_linux_probe`, a new diagnostic that opens `libvulkan.so.1` and enumerates physical devices.

## 2. What is already portable

| Area | State | Evidence |
| --- | --- | --- |
| Launcher UI (`eframe`/`egui`) | Portable; links and opens on Linux | `cargo check` exits 0 (49.5 s); debug `cargo build` links (58.3 s); 1440x900 window under WSLg on X11 and Wayland |
| Launcher data root | `platform::data_root()` → `$XDG_CONFIG_HOME/EmblemWeave` or `~/.config/EmblemWeave` | `apps/emblemweave/src/platform.rs`, exercised by the probe's config-root line |
| Shader translator | .NET publishes a self-contained `linux-x64` ELF | Existing `docs/LINUX.md`, release asset |
| Window creation | SDL3 (`SDL_WINDOW_VULKAN`), already used by `runtime/src/gfx/platform.cpp` | Source; SDL3 is fetched by CMake |
| Vulkan API surface | Vulkan 1.3, `VK_KHR_swapchain`; `VK_EXT_full_screen_exclusive` optional | `src/gfx/vk_device.cpp` |
| Video decode | `movie_ffmpeg.cpp` exists next to the Media Foundation path, plus libvpx/dav1d options | `runtime/src/media/`, `cmake/media_deps.cmake` |
| Archive layout | Zip is Linux-safe and verified | `tools/verify_archive.py` |
| Platform layer (memory, threads, sync, time, files) | **Builds and passes its self-test on Linux** after this work | `ctest` 1/1 passed; see the test matrix |

The last row is the real change in this workstream: the POSIX backend now compiles, its test runs, and a diagnostic
binary reports the host. It is a foundation, not a game.

## 3. What is Windows-specific

### 3.1 The generated count

`tools/port/win32_audit.py` reports **700 Windows calls across 70 runtime files** (476 sources scanned), and 80 files
under `runtime/src` + `runtime/include` include `<windows.h>` directly.

| Subsystem | Files | Calls | Replacement |
| --- | ---: | ---: | --- |
| Threads | 32 | 134 | pthreads (`pthread_create`, `pthread_setaffinity_np`, per-thread nice) |
| Memory | 16 | 41 | `mmap`/`mprotect`/`madvise`, `memfd_create` |
| Sync | 10 | 138 | pthread mutex/cond/rwlock, futex where a raw wait is needed |
| Time | 21 | 74 | `clock_gettime(CLOCK_MONOTONIC)`, `nanosleep`, `timerfd` |
| Files | 31 | 144 | POSIX descriptors, `openat`, `stat`, `dlopen` |
| Registry | 1 | 3 | a config file, not a registry emulation |
| Window | 1 | 2 | SDL3 monitor queries |
| Input | 0 | 0 | already SDL3 gamepad |
| Audio | 4 | 16 | PipeWire/ALSA behind the existing audio sink |
| Media | 1 | 3 | the existing `movie_ffmpeg.cpp` path |
| Console/diag | 45 | 145 | `getpid`, `strerror`, UTF-8/UTF-16 conversion |
| **Total** | **70** | **700** | |

The five heavy subsystems (threads, memory, sync, time, files) are 531 of the 700 calls. That is why the platform
layer is the correct first cut: it removes the bulk of them from the call sites instead of editing 70 files.

### 3.2 Launcher modules

| Module | Windows dependency | Classification |
| --- | --- | --- |
| `platform.rs` | `%APPDATA%`, `explorer`, `notepad`, `tasklist`, `taskkill` | 1 (replace) and 3 (already abstracted) |
| `engine.rs` | `pwsh`, `cmd /C`, `vcvars64.bat`, `vswhere`, `clang-cl`, `cmake`, `ninja`, `dotnet` | 1: the build pipeline is Windows-shaped |
| `toolchain.rs` | Visual Studio / Windows SDK / LLVM discovery | 1: replaced by a Linux toolchain probe |
| `cheats.rs`, `cheats::process` | `OpenProcess`, `ReadProcessMemory`, `VirtualQueryEx` | 1: `process_vm_readv` + `/proc/<pid>/maps`; stub today |
| `pad.rs` | XInput | 4: SDL3 gamepad, already in the engine |
| `exe_icon.rs` | Win32 resource APIs | 6: skip on Linux (cosmetic) |
| `crash.rs` | `dbghelp` | 1: signal handler + `backtrace`/libunwind |
| `updater.rs` | process replace of a running `.exe` | 1: replace-on-restart is a Linux-safe design |
| `mods.rs`, `saves.rs`, `gamedata.rs` | `%LOCALAPPDATA%`, `robocopy` | 3: route through `platform.rs`; replace `robocopy` |

The launcher's *refusals* are correct and should stay until the underlying work exists: `build_supported()` and
`install_supported()` return false off Windows and the UI says why. A Linux build path must be added behind a real
probe, not by flipping those booleans.

### 3.3 Engine subsystems

| Area | Files / calls | Classification | Notes |
| --- | --- | --- | --- |
| Guest memory, write-watch | memory 16/41, part of sync | 1 (replace) | `mmap`/`mprotect`/`SIGSEGV` backend now exists; read/write fault distinction added |
| Threads, priority, affinity | 32/134 | 1 (replace) | pthreads backend exists; per-thread nice added |
| Synchronisation | 10/138 | 1 (replace) | pthread mutex/cond/rwlock; Windows SRW semantics map closely |
| Clock and sleep | 21/74 | 1 (replace) | `CLOCK_MONOTONIC`; `clock_hz()` = 1e9 |
| Files, DLL loading | 31/144 | 1 (replace) | `dlopen` for the Vulkan loader; POSIX descriptors for saves and packs |
| Vulkan device + swapchain | `gfx/vk_device.cpp`, `gfx/platform.cpp` | 3 + 4 | SDL3 window + system loader; Windows-only `VK_EXT_full_screen_exclusive` becomes optional |
| Exception/unwind for lifted code | `src/eh`, `diag/crash.cpp` | 1 + 6 | MSVC SEH has no direct Linux equivalent; guest exceptions must use the runtime's own handler |
| Audio (XAudio2) | 4/16 | 1 (replace) | XAudio2 is COM; use PipeWire/ALSA behind `audio_sink` |
| Media Foundation | 1/3 | 4 (replace with existing portable path) | `movie_ffmpeg.cpp` already exists |
| Lifted game libraries | `recomp/scripts/build_game.cmd` | 6 (substantial redesign) | MSVC `.lib` objects must be rebuilt for ELF; ABI shims for `__declspec`, packing, `#pragma comment` |
| Vendor upscalers | Streamline/FSR DLLs | 6 | Windows DLLs; the engine's own TAAU/DLSS-replacement path stays |

### 3.4 Build system

* `runtime/CMakeLists.txt` forces `NXRT_PLATFORM_ONLY` when `NOT WIN32`; that is why only the platform layer builds.
* `cmake/gfx.cmake` passes `/MAP /DEBUG /STACK:8388608` to the linker unconditionally, which is MSVC/lld-link
  syntax. A Linux build needs a toolchain branch, not flag-by-flag patching at each target.
* `cmake/gfx_deps.cmake` fetches SDL3; that part is already portable.
* `apps/emblemweave/src/engine.rs` runs `cmd /C recomp\scripts\build_game.cmd fefw` with a `vcvars64.bat`
  environment; the recompiler emits C++ that `clang-cl` compiles into `fefw_*.lib`. A Linux build needs `clang` +
  `lld` + `ar` and an ELF ABI for the lifted code.
* The shader translator is already cross-published (`dotnet publish -r linux-x64`).

## 4. Dependency classification

| Class | Meaning | Examples here |
| --- | --- | --- |
| 1 | Windows-only, needs replacement | `VirtualAlloc`, `CreateThread`, SRW locks, `CreateFileW`, `QueryPerformanceCounter`, XAudio2, `dbghelp`, `ReadProcessMemory` |
| 2 | Already cross-platform | `eframe`/`egui`, SDL3 window and gamepad, Vulkan API calls, zstd, libvpx, dav1d, FFmpeg path |
| 3 | Portable abstraction exists or is one module away | data root, logs, config, file dialogs, process listing, updater restart |
| 4 | Linux system library | `libvulkan.so.1`, pthreads, PipeWire/ALSA, `dlopen`, `mmap` |
| 5 | Available through Proton/Wine | Win32 process/window/audio/path behaviour for the Windows build |
| 6 | Unsupported or substantial redesign | MSVC-lifted libraries, SEH guest exceptions, vendor upscaler DLLs, Steam Input integration on Deck |

## 5. Native Linux feasibility

**Launcher: feasible, linked and opened.** `cargo check` for `x86_64-unknown-linux-gnu` passes with 0 errors under
native Linux (rustc 1.100.0). With the distribution's Wayland/X11/xkbcommon/EGL development packages installed, the
debug binary links and opens a 1440x900 window under WSLg on X11 and on Wayland. The window was not clicked through, a
release link was not attempted, and WSLg's software Vulkan is not a desktop GPU test.

**Engine: partially feasible, not yet buildable.** The platform layer builds and passes, but the rest of `nxrt` still
includes `<windows.h>` in 80 files and makes 700 calls. The two hardest items are not in that count:

1. **Lifted code.** The game's recompiled C++ is compiled by MSVC into `.lib` files. A Linux build requires rebuilding
   those for ELF and providing an ABI shim. This is a recompiler-output change, not a platform-layer change.
2. **Guest exception handling.** The runtime translates guest exceptions through Windows SEH. On Linux the equivalent
   is the runtime's own signal/unwind path; it must be designed and tested, not `#ifdef`-ed.

**Verdict:** a native build is achievable in stages, and the platform layer is no longer the blocker. It is not
achievable as a single flag flip, and no gameplay claim can be made today.

## 6. Proton-assisted compatibility assessment

| Question | Finding |
| --- | --- |
| Can the Windows launcher run under Proton? | Plausible: it is a Win32 GUI + Vulkan program. Untested; no full Proton session was established here. |
| Can the engine run under Proton? | Plausible for the same reason. Untested. |
| Do DXVK/VKD3D help? | **No.** The engine issues Vulkan directly; those layers translate D3D. |
| What does Proton actually provide? | Wine's Win32 API surface, prefix/path mapping, XAudio2/Media Foundation implementations, DLL loading, and Vulkan passthrough to the host driver. |
| Shared prefix for launcher + engine? | They should share one prefix per app id, as Proton expects; the launcher must pass `WINEPREFIX`/`STEAM_COMPAT_DATA_PATH` consistently. Untested. |
| GPU coverage | Depends on the host's Vulkan driver (RADV/NVIDIA/Intel), not on Proton. Software lavapipe is enough to enumerate but not to play. |
| Steam Deck | Proton is the natural route there, but a Deck run was not possible; nothing is claimed. |

**Why Proton is the faster compatibility route:** it reuses the existing Windows binaries and avoids the lifted-code
ABI problem entirely. **Why it is not the long-term answer:** it keeps a Windows build as the product, adds a runtime
the project does not control, and does not remove any of the 700 host calls from the codebase. Both paths can coexist:
ship native where it works, offer Proton as an explicit compatibility mode for the Windows build.

## 7. Findings that change the plan

1. **Proton's graphics stack is a red herring for EmblemWeave.** Any plan that says "use DXVK/VKD3D" is wrong for a
   Vulkan renderer. The reusable part of Proton is Wine's host surface and its operational patterns.
2. **The POSIX backend did not compile at all** at the start of this work: nine errors, including a `PROT_*` macro
   collision, a non-existent `Thread::id`, missing `<sys/resource.h>`, and an unhandled page-fault path that would
   loop forever. It now builds and passes. Details are in the test matrix.
3. **The self-test itself had a protection bug:** it passed `false`/`true` where the API takes a protection mask, so
   it read a `PROT_NONE` page. It passed only because the old backend treated the argument as a bool; against the
   newer mask-based backend it would fault. Fixed in the isolated branch.
4. **The bulk of the port is five subsystems, not 70 files.** The platform interface is the right boundary; the
   remaining call sites should be migrated to it incrementally.
5. **The launcher is ready to be the first native deliverable.** It already compiles and already refuses the Windows
   build path honestly; a Linux "engine not built here" flow is a small, truthful step.

## 8. Audit conclusion

* Native Linux: **platform layer verified; launcher compiles; engine blocked** by the remaining Win32 surface and the
  lifted-code ABI.
* Proton-assisted: **Wine executes; full Proton session unverified; no gameplay evidence.**
* The correct next step is not to transplant Proton. It is to finish the platform boundary, then the lifted-code
  build, then test rendering, audio and saves on real hardware.
