# Linux architecture

This page describes the architecture this workstream recommends, given the audit. It separates what runs natively
from what runs through Proton, and it keeps the Windows build as the reference implementation.

## 1. Two execution paths, one codebase

```
                      +-------------------------------+
                      |        EmblemWeave repo       |
                      |  apps/emblemweave (Rust/egui) |
                      |  engine/fefw-recomp (C++/CMake)|
                      +---------------+---------------+
                                      |
              +-----------------------+-----------------------+
              |                                               |
   NATIVE LINUX PATH                                  PROTON PATH (optional)
   launcher: cargo build --target linux               launcher: EmblemWeave.exe
   engine:   clang + CMake + Ninja                    engine:   fefw.exe
   host:     nxrt::platform POSIX backend             host:     Wine 11 + Win32 + Vulkan
   graphics: libvulkan.so.1 + SDL3                    graphics: libvulkan.so.1 (passthrough)
   audio:    PipeWire/ALSA                            audio:    Wine XAudio2
   saves:    $XDG_* / game data root                  saves:    prefix drive_c
```

The two paths share the launcher's settings format, the game-data layout, the shader translator (already a Linux
ELF) and the engine's Vulkan renderer. They do **not** share a process, a prefix, or a build.

### Native path

* **Launcher**: the existing Rust binary, built for `x86_64-unknown-linux-gnu`. It keeps `platform.rs` as the only
  place that answers OS questions. On Linux `build_supported()` stays false until the engine build exists; the UI
  explains that instead of failing later.
* **Engine**: the production `fefw-recomp` runtime, compiled by clang against the POSIX platform backend. The
  graphics shell stays SDL3 + Vulkan; no second engine, no Rust runtime substitution.
* **Build**: CMake selects the platform directory by host, as it already does. A Linux toolchain branch replaces the
  MSVC link options; the lifted game code is rebuilt for ELF by the recompiler.

### Proton path

* Runs the **existing Windows binaries** under the Proton that Steam installed. No fork, no bundled runtime, no
  copied Wine.
* The launcher and engine share one prefix per app id. Steam passes `STEAM_COMPAT_DATA_PATH`,
  `STEAM_COMPAT_CLIENT_INSTALL_PATH` and the app id; a manual launch must set them explicitly.
* Graphics are Vulkan passthrough. DXVK/VKD3D are present in Proton but are not used by EmblemWeave and must not be
  made a dependency.

## 2. Why both can coexist

| Concern | Native | Proton |
| --- | --- | --- |
| Launcher | same Rust code, Linux target | same `.exe`, unchanged |
| Engine | same C++ code, Linux build | same `.exe`, unchanged |
| Saves/config | XDG roots, same file formats | prefix `drive_c`, same file formats |
| Graphics | system Vulkan + SDL3 | Wine + system Vulkan |
| Audio | PipeWire/ALSA sink | Wine XAudio2 |
| Build owner | project | Valve |
| Failure mode | a missing port | a Wine/Proton regression outside the project |

The launcher can detect which path it is on (`platform::name()` plus an explicit "compatibility mode" setting) and
point the user at the right data root. It must not silently write a native config into a Proton prefix or the
reverse.

## 3. Platform boundary

`runtime/include/nxrt/platform.h` is the contract: memory, threads, synchronisation, time, files, page protection,
sections, named mutexes, fault handling. Two implementations exist:

* `src/platform/win32/platform_win32.cpp` — today's calls, unchanged behaviour.
* `src/platform/posix/platform_posix.cpp` — mmap/pthreads/clock_gettime/descriptors; the version in this branch
  builds and passes `platform_selftest`.

Rules for finishing the port:

1. **Move call sites to the interface, not `#ifdef`s.** A file that needs a thread, a file handle or a clock asks
   `nxrt::platform`; it does not include `<windows.h>`.
2. **Keep the Windows implementation the reference.** The POSIX side must match its semantics (reserve/commit,
   relative waits, SRW shared/exclusive, `CreateFileW` sharing), not merely compile.
3. **No platform type in a public header.** The interface passes integers and opaque pointers; that is why it works
   without Vulkan or SDL includes.
4. **Test the contract once, on both hosts.** `tests/platform_selftest.cpp` is the shared test; a new function is not
   done until both backends pass the same assertions.

## 4. Graphics

* **Loader**: open `libvulkan.so.1` at runtime (the engine already has a volk-style loader). Do not link the loader
  into the executable; a missing loader must produce a diagnostic, not a load-time failure.
* **Instance**: request Vulkan 1.3, `VK_KHR_surface` plus the SDL-provided platform extension. Treat
  `VK_EXT_full_screen_exclusive` as optional; on Linux it is Windows-only.
* **Devices**: enumerate at runtime, never hardcode a vendor or a GPU index. Prefer a discrete GPU, fall back to
  integrated, and allow an explicit override for multi-GPU systems.
* **Presentation**: SDL3 window with `SDL_WINDOW_VULKAN`; borderless fullscreen is a window mode, not an exclusive
  mode switch. Wayland and X11 both go through SDL; do not talk to either protocol directly.
* **Caches**: shader/pipeline caches live under `$XDG_CACHE_HOME/EmblemWeave` (native) or the prefix's cache
  directory (Proton), keyed by driver and device. A cache from another driver must be discarded, not reused.
* **Diagnostics**: `nxrt_linux_probe` reports kernel, page size, cores, config/cache roots, loader version and every
  physical device with vendor/device/API/driver. It is the first thing a Linux bug report should include.

## 5. Audio, video, input

* **Audio**: keep the existing `audio_sink` abstraction. The Windows backend is XAudio2; the Linux backend is
  PipeWire with an ALSA fallback. Do not emulate XAudio2 natively.
* **Video**: `movie_ffmpeg.cpp` already exists next to `movie_mf.cpp`. On Linux the FFmpeg path is the implementation;
  Media Foundation is not.
* **Input**: SDL3 gamepad already normalises controllers. Steam Input on Deck presents an Xbox-style pad, which the
  existing map handles; the launcher's XInput page uses SDL on Linux (or says no pad is connected).

## 6. Files, saves, profiles

* One root per purpose, resolved in `platform.rs` and passed to the engine explicitly:
  `$XDG_CONFIG_HOME/EmblemWeave` (settings), `$XDG_DATA_HOME/EmblemWeave` (profiles, saves),
  `$XDG_CACHE_HOME/EmblemWeave` (shader/pipeline caches).
* Never write inside the installation directory; SteamOS mounts it read-only outside the prefix.
* Case sensitivity: match game-data paths case-insensitively where the Windows build did, or resolve the real name
  once and cache it. Do not assume `RomFS` and `romfs` are the same directory.
* Saves stay byte-compatible. The format is the contract, not the path.

## 7. Build and toolchain

| Piece | Windows (today) | Linux (target) |
| --- | --- | --- |
| Compiler | `clang-cl` via `vcvars64.bat` | `clang`/`clang++` |
| Linker | `lld-link` | `lld` |
| Build files | CMake + Ninja | CMake + Ninja |
| Static libs | `.lib` | `.a` (`llvm-ar`) |
| Lifted code | `recomp/scripts/build_game.cmd` | same recompiler, ELF target |
| Translator | `dotnet publish -r win-x64` | `dotnet publish -r linux-x64` (already verified) |
| Detection | `vswhere`, Windows SDK | `clang --version`, CMake, Ninja, `pkg-config` |

CMake should choose flags by toolchain (`CMAKE_CXX_COMPILER_ID`/`WIN32`), not by patching `/MAP` at each target.
`NXRT_PLATFORM_ONLY` stays the honest Linux default until the rest of `nxrt` compiles.

## 8. Steam Deck

Deck is Linux x86-64 with an immutable root, gamescope and Steam Input:

* **Build elsewhere, ship prebuilt.** The Deck cannot run MSVC; the launcher must verify/install a prebuilt engine
  instead of compiling on device.
* **Gamescope**: present to an SDL Vulkan window; do not require exclusive fullscreen.
* **Steam Input**: the pad arrives as an Xbox-style controller; SDL handles it.
* **Storage**: caches and saves under the prefix/XDG roots; the install tree is read-only.
* **Power**: the frame limiter and suspend/resume need testing on hardware; no claim is made from a desktop run.
* **Launch**: a Steam library entry with the launcher as the target, so no terminal is needed. Proton mode uses the
  same entry with the Windows launcher as the target and Proton forced in the game's properties.

## 9. What this architecture deliberately does not do

* It does not fork, rebuild or bundle Proton.
* It does not use DXVK/VKD3D (the renderer is Vulkan).
* It does not introduce a second engine or redirect Build/Play to the experimental Rust runtime.
* It does not put platform `#ifdef`s in the 70 call-site files; it moves them to the platform interface.
* It does not claim Deck support without a Deck test.
