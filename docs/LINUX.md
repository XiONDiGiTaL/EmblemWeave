# Linux, SteamOS and the Steam Deck

**Status: not supported.** There is no Linux build of the engine and none is advertised. This page is the plan,
with the parts that have already been built and run marked, so that "native Linux support" is a thing somebody
finishes rather than a sentence in a README.

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

## What a Linux build needs

Verified on the machine this page was written on, where marked:

| Piece | Version | Verified |
| --- | --- | --- |
| Rust | 1.98.1, target `x86_64-unknown-linux-gnu` | yes: launcher `cargo check` passes |
| .NET SDK | 10.0.404 (only to build the translator; the release ships it published) | yes: `-r linux-x64` publish produces a running ELF |
| C/C++ compiler | clang 18 or newer with `lld` | not yet: a stock Ubuntu 26.04 image has neither |
| CMake | 3.25 or newer | not yet (not installed in the image used) |
| Ninja | any recent | not yet |
| Vulkan loader | `libvulkan.so.1` (`libvulkan1` / `vulkan-loader`) | present in the image (9 `ldconfig` entries) |
| Vulkan driver | Mesa RADV or `nvidia` | present in the image; not exercised by the engine |
| SDL3 | fetched by the engine's CMake (`cmake/gfx_deps.cmake`) | not yet |
| Python | 3.13 or newer with `lz4`, `zstandard`; `tools/.venv` is created with `uv` | not yet on Linux |

The engine's CMake asks for the Windows SDK and MSVC-specific flags today; the port's first commit has to teach
`cmake/` a second toolchain file rather than patching the flags at each target.

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
2. **Engine platform layer.** `runtime/src/platform` gains a Linux implementation for memory, threads, files,
   the clock and exception handling; the 69 `<windows.h>` files include the interface instead. Done when
   `nxrt_tests` and `nxrt_gfx_tests` build and pass on Linux (they are the existing, game-independent tests).
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
