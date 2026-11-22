# Linux, SteamOS and the Steam Deck

**Status: not supported.** There is no Linux build of the engine and none is advertised. This page is the plan,
with the parts that have already been built and run marked, so that "native Linux support" is a thing somebody
finishes rather than a sentence in a README.

> **Linux compatibility workstream (isolated branch `linux/compat-prototype`).** The audit, the Proton Hotfix
> reverse engineering, the architecture and the current test results are in
> [LINUX_COMPATIBILITY_AUDIT.md](LINUX_COMPATIBILITY_AUDIT.md), [PROTON_HOTFIX_REVERSE_ENGINEERING.md](PROTON_HOTFIX_REVERSE_ENGINEERING.md),
> [LINUX_ARCHITECTURE.md](LINUX_ARCHITECTURE.md), [LINUX_IMPLEMENTATION_PLAN.md](LINUX_IMPLEMENTATION_PLAN.md) and
> [LINUX_COMPATIBILITY_TEST_MATRIX.md](LINUX_COMPATIBILITY_TEST_MATRIX.md). Build commands are in
> [LINUX_BUILD.md](LINUX_BUILD.md). As of that work, the POSIX platform layer builds and passes its self-test on
> Linux and the launcher type-checks for Linux; the engine still does not build there.

## What already works on Linux

These are not plans: each was built or run on this machine while writing the page, and the command is here so it
can be repeated.

1. **The launcher compiles for Linux.** `cargo check -p emblemweave --target x86_64-unknown-linux-gnu` exits 0
   after adding the Linux target (`rustup target add x86_64-unknown-linux-gnu`). Everything the launcher draws is
   `eframe`/`egui`, which is portable; the Windows-only pieces are behind `cfg` and behind `src/platform.rs`
   ([PLATFORMS.md](PLATFORMS.md) is the audit).
2. **The shader translator builds and runs for Linux.**
   `dotnet publish engine/fefw-recomp/tools/shaders/harness/SpirvHarness.csproj -c Release -r linux-x64
   --self-contained true -p:PublishSingleFile=true` produces a 78 MB self-contained ELF binary; run under WSL
   Ubuntu 26.04 it prints its usage. The release already ships it as
   `EmblemWeave-v<version>-shader-translator-linux-x64.tar.gz`, with the executable bit set and verified by
   `tools/verify_archive.py`.
3. **The corpus extractor is pure Python** (`tools/shaders/ampere/extract_corpus.py`) and needs only `lz4` and
   `zstandard`, both of which build on Linux. It walks a RomFS folder, which is the same on both systems.
4. **The release archive is Linux-safe.** The zip has 1287 files in 131 folders, no backslashes and no absolute
   names, and extracts under Linux with `unzip` and with Python's `zipfile` with `docs/PACKAGE.md`,
   `docs/changelog/` and `engine/fefw-recomp/...` at the paths the launcher looks for. That check is automated:
   `tools/verify_archive.py`, run by `tools/package_release.ps1` and again by `.github/workflows/release.yml`.
5. **The window layer is SDL3.** `engine/fefw-recomp/runtime/src/gfx/platform.cpp` creates its window and its
   surface through SDL, which is why the graphics shell is the part of the engine closest to portable. The
   Vulkan side is ordinary Vulkan 1.3 with no Windows-specific extensions in the required set (`VK_KHR_swapchain`
   and `VK_EXT_full_screen_exclusive`; the latter is optional and Windows-only by nature, so a port must treat it
   as absent rather than required).

## What blocks it

1. **The engine's Win32 layer.** 69 files under `runtime/src` include `<windows.h>`. The heaviest use is the
   platform layer the lifted code calls into: guest memory (`VirtualAlloc`, `VirtualProtect`), threads
   (`CreateThread`, `SetThreadPriority`), files (`CreateFileW`, `ReadFile`, memory-mapped images), the clock
   (`QueryPerformanceCounter`, 28 uses), exception handling for the lifted code (`AddVectoredExceptionHandler`,
   `RaiseException`, SEH unwinding through `_except_handler`), and process/thread naming. This is the port: an
   interface with a Windows implementation (today's code) and a Linux one, not a `#ifdef` in 69 files.
2. **The lifted libraries are MSVC objects.** `recomp/scripts/build_game.cmd` produces `fefw_*.lib` with the same
   `clang-cl` that builds the runtime, and the game exe links them. A Linux build needs those libraries built by
   clang for ELF, which means the recompiler's generated C++ compiled a second time with a Linux target and an
   ABI shim for the few MSVC-isms in the generated code (`__declspec`, `#pragma comment`, struct packing).
3. **The build pipeline is Windows-shaped.** `engine::build` runs `pwsh`, `vcvars64.bat`, CMake with Ninja and
   `clang-cl`, then `dotnet build`/`publish` and `pwsh` scripts. On Linux the same steps exist with `clang`,
   `cmake`, `ninja` and `dotnet`, but the detection (Visual Studio, the Windows SDK, `vswhere`) is Windows-only
   and the launcher refuses the build there ([PLATFORMS.md](PLATFORMS.md)).
4. **NVIDIA's and AMD's extras are Windows DLLs.** DLSS through Streamline (`sl.interposer.dll`,
   `nvngx_dlss.dll`) and FSR 3 through the FidelityFX DLLs are Windows binaries. On Linux the FSR 1/NIS paths and
   the NVN-level DLSS replacement that ships in the engine (`[dlss] upscaler = taau`) still work; the vendor SDK
   upscalers do not, and the Graphics page has to say so.
5. **Cheats and the live save editor** read another process's memory. That is `ReadProcessMemory` today; the
   Linux equivalent is `process_vm_readv`/`/proc/<pid>/maps` and is a later phase, not part of "the game runs".
6. **Crash reporting** uses `dbghelp`. A Linux port uses a signal handler and `backtrace`/`libunwind`.
7. **The launcher's file dialogs and shell integration** (`explorer`, `notepad`, `taskkill`, `tasklist`) have
   direct equivalents (`xdg-open`, `$EDITOR`, `/proc`), and `platform.rs` is where they go.

## The Windows calls, counted

`python tools/port/win32_audit.py` reads `runtime/src` and `runtime/include`, counts what each subsystem calls, and
prints this table. It is generated rather than estimated, so a phase that lands can be re-measured with one command.
Counts are from the tree at this commit.

| Subsystem | Files | Calls | Replace with | Most used |
|---|---:|---:|---|---|
| Threads | 31 | 130 | pthreads | `GetCurrentThreadId`, `GetCurrentThread`, `SetThreadPriority`, `YieldProcessor` |
| Memory | 16 | 78 | mmap/mprotect | `VirtualFree`, `VirtualProtect`, `VirtualAlloc`, `VirtualQuery` |
| Sync | 11 | 147 | pthreads/futex | `ReleaseSRWLockExclusive`, `AcquireSRWLockExclusive`, `ReleaseSRWLockShared`, `WaitForSingleObject` |
| Time | 21 | 82 | clock_gettime/timerfd | `Sleep`, `QueryPerformanceCounter`, `QueryPerformanceFrequency`, `GetTickCount64` |
| Files | 28 | 122 | POSIX file API | `CloseHandle`, `GetProcAddress`, `GetFileAttributesW`, `CreateFileW` |
| Registry | 1 | 3 | config files instead | `RegOpenKeyExW`, `RegEnumValueW`, `RegCloseKey` |
| Window | 1 | 2 | SDL3 window | `MonitorFromWindow`, `GetMonitorInfoW` |
| Input | 0 | 0 | SDL3 gamepad | — |
| Audio | 4 | 16 | PipeWire/ALSA | `CoInitializeEx`, `CoUninitialize`, `CoCreateInstance` |
| Media | 1 | 3 | FFmpeg/libvpx | `MFCreateMediaType`, `MFStartup`, `MFCreateSourceReaderFromURL` |
| Console/diag | 43 | 150 | POSIX equivalent | `GetCurrentProcess`, `GetLastError`, `WideCharToMultiByte`, `TerminateProcess` |
| **Total** | **68** | **733** | | |

The shape of the port is in that table. **Threads, sync, memory, time and files are 559 of the 733 calls**, and they
are the five things the lifted code touches on almost every call it makes, so they are the interface phase 2 has to
get right. The window is not a project: it is already SDL3, and two monitor queries are the whole of it. Input does
not appear because SDL's gamepad API already does that job. Audio is four files and thirteen COM calls (XAudio2),
media is three Media Foundation calls, and one file reads three values from the registry. The 150 calls under
*Console/diag* are process ids, last-error text and UTF-16 conversion — a header and an `#if` each, not a subsystem.

The files that carry the most of it are the ones to open first: `src/diag/crash.cpp` (six subsystems),
`src/mem/write_watch.cpp`, `src/testtools/tt_core.cpp`, `src/services/audio_xaudio2.cpp` (five each),
`src/kernel/thread.cpp`, `src/kernel/host_place.cpp`, `src/kernel/starvation_guard.cpp`, `src/mem/memory.cpp` and
`src/gfx/nvn/nvn_gpu.cpp` (four each).

## What a Linux build needs

Verified on the machine this page was written on, where marked:

| Piece | Version | Verified |
| --- | --- | --- |
| Rust | 1.98.1, target `x86_64-unknown-linux-gnu` | yes: launcher `cargo check` passes |
| .NET SDK | 10.0.404 (only to build the translator; the release ships it published) | yes: `-r linux-x64` publish produces a running ELF |
| C/C++ compiler | clang 18 or newer with `lld` (`clang`, `lld`) | not yet: a stock Ubuntu 26.04 image has neither |
| CMake | 3.25 or newer (`cmake`) | not yet (not installed in the image used) |
| Ninja | any recent (`ninja-build`) | not yet |
| Vulkan loader | `libvulkan.so.1` (`libvulkan1` / `vulkan-loader`) | present in the image (9 `ldconfig` entries) |
| Vulkan driver | Mesa RADV or `nvidia` | present in the image; not exercised by the engine |
| SDL3 | fetched by the engine's CMake (`cmake/gfx_deps.cmake`) | not yet |
| Python | 3.13 or newer with `lz4`, `zstandard`; `tools/.venv` is created with `uv` | not yet on Linux |

The engine's CMake asks for the Windows SDK and MSVC-specific flags today; the port's first commit has to teach
`cmake/` a second toolchain file rather than patching the flags at each target.

What to install in a WSL Ubuntu (or any Ubuntu 26.04) to build and test the platform layer here:

```bash
sudo apt update
sudo apt install -y clang lld cmake ninja-build pkg-config \
                    libvulkan-dev libzstd-dev zlib1g-dev \
                    python3 python3-venv python3-pip
# only if the distribution has no SDL3 package yet; the engine's cmake/gfx_deps.cmake can also fetch it
sudo apt install -y libsdl3-dev || true
```

Then, from the repository root:

```bash
cmake -S engine/fefw-recomp/runtime -B build-linux -G Ninja \
      -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++
cmake --build build-linux
ctest --test-dir build-linux --output-on-failure     # one entry: platform
```

`runtime/CMakeLists.txt` builds only the platform layer there (`NXRT_PLATFORM_ONLY` is forced on when the host is not
Windows), because the rest of nxrt — the graphics shell, the game module, the lifted code — still asks for Windows
headers. Nothing else in the tree is needed for that build, and nothing in it writes to the Windows side of the tree.

## SteamOS and the Steam Deck

The Deck is Linux x86-64 with an immutable root and gamescope. Nothing above gets easier, and three things are
specific:

* **The build happens elsewhere.** A Deck player cannot run MSVC, so the release for a Deck would ship a prebuilt
  engine and the player would only run the launcher, the translator (Linux ELF, already verified) and the game
  data. That changes `Build the game` from "compile on the player's PC" to "verify and install the prebuilt",
  which is a different job flow and has to be designed, not bolted on.
* **gamescope is the compositor.** A fullscreen window goes through it; the Vulkan path only has to present to an
  `SDL_WINDOW_VULKAN` window, and `VK_EXT_full_screen_exclusive` must not be required. Frame pacing (the engine's
  limiter) and the Deck's 40/60 Hz profiles need testing there.
* **The controller is Steam Input's.** SDL's gamepad API and the engine's existing pad map cover it; XInput in the
  launcher's Controls page gets a Linux implementation (SDL or evdev) or the page says it cannot show a pad.
* **The filesystem is case-sensitive and read-only outside the prefix.** The save folder, the shader cache and
  the per-game data live under the Proton prefix or `$XDG_DATA_HOME`, and every path the launcher writes has to
  come from `platform.rs`, which is what that module is for.

## Phases

Each phase ends with something that was built and run, not something that was designed.

1. **Launcher on Linux.** A window opens, Game Setup reads a folder, Requirements explains that the build is
   Windows-only, Play is refused with the same sentence. Done when `cargo build --target
   x86_64-unknown-linux-gnu` links and the window runs on this machine. Most of it exists: the check passes.
2. **Engine platform layer.** *Started.* `runtime/include/nxrt/platform.h` now carries the host interface — memory,
   threads, synchronisation, time and files — with `src/platform/win32` (VirtualAlloc, `_beginthreadex`, SRW locks,
   QueryPerformanceCounter, the high-resolution waitable timer, CreateFileW) and `src/platform/posix` (mmap, pthreads,
   clock_gettime, nanosleep, file descriptors) behind it. The time subsystem (`kernel/time.cpp`) is the first call site
   moved: it no longer includes `<windows.h>`, and on Windows the tick and the sleep are the same calls as before.
   `tests/platform_selftest.cpp` exercises all five groups and builds and passes on Windows today; on Linux it builds
   from the same CMake file with `NXRT_PLATFORM_ONLY`.

   What is left for the phase: the other 20-odd files with host calls (threads and sync in `kernel/`, files in `svc/`),
   the exception and unwind path, `dlopen`-shaped loading, and `nxrt_gfx_tests`. Done when `nxrt_tests` and
   `nxrt_gfx_tests` build and pass on Linux (they are the existing, game-independent tests).
3. **Lifted code for Linux.** The recompiler emits C++ that clang links for ELF; `fefw_*.lib` becomes
   `libfefw_*.a`. Done when `fefw` boots to its first frame with the test dump.
4. **Play on Linux.** Input, audio, saves, mods, screenshots, the F1 menu. Done when a full play session happens
   on a Linux desktop, with the frame captured and compared as the Windows tests do.
5. **Deck.** Prebuilt engine path in the launcher, gamescope present, controller through SDL, a `.tar.gz` release
   asset checked by `tools/verify_archive.py`, and a run on real hardware.
6. **Only then**: a release note that says Linux is supported.

## What this page does not promise

No date, no branch, and no support claim. Issues asking for a Linux build get this page until phase 4 has been
run on real hardware; a launcher that compiles is not a game that plays.
