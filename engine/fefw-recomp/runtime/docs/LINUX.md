# Native Linux and SteamOS

This is the port of the nxrt runtime (and through it EmblemWeave) from Windows to Linux, with SteamOS and the
Steam Deck as first-class targets. Windows remains the reference platform: every change here is expected to keep the
Windows build working, and the Windows build is the regression configuration.

The goal is a native Linux executable that uses Vulkan, SDL3 and the host's audio device directly. Wine and Proton may
remain a way to run the Windows build, but they are not this port.

## Status

What is verified today:

- **POSIX platform layer**: `src/platform/posix` (memory reservation, threads, mutexes/conditions, clocks, sleeps, files,
  page protection and fault handling, named sections, affinities) builds with GCC and Clang and passes
  `nxrt_platform_tests` on Linux (WSL2, Ubuntu).
- **Linux build configuration**: CMake no longer returns early on non-Windows hosts. The default Linux configuration
  builds the platform layer and its test; `-DNXRT_PLATFORM_ONLY=OFF` configures the rest of the runtime.
- **Windows**: the whole runtime still builds (`nxrt`, `nxrt_gfx` and the test executables).

What is ported to the platform boundary but not yet linked into a Linux executable:

- dispatch table allocation and the executable base anchor (`platform::module_base`);
- guest abort paths (`platform::process_exit`, Windows keeps `RaiseException`);
- log time base, line timestamps and thread names (`platform::now_ns`, `pthread_setname_np`);
- frame pacing (`platform::sleep_ns`, `platform::yield_cpu`);
- host device / Steam Deck detection (DMI files and `SteamDeck=1` instead of the registry);
- thread affinity parse (`std::strtoull`), the WAV writer's path (`fopen` with UTF-8), the video decoder's timing
  (`platform::now_ns`) and priority, and the audio renderer's thread boost (nice instead of MMCSS);
- the UI language from the process locale and the `controls.ini` lookup without `GetPrivateProfileStringA`.

What still blocks a full Linux link (the current frontier, all in `src/`):

| File | Windows dependency | Replacement |
| --- | --- | --- |
| `os/sync.cpp` | SRWLOCK / CONDITION_VARIABLE, `GetCurrentThreadId` | the platform mutex/condition/rwlock API |
| `services/fs_volume.cpp` | `CreateFileW`/`ReadFile`/`WriteFile`, `MoveFileExW`, `OpenProcess` | `platform::File` plus positioned I/O and truncate; `/proc` for liveness |
| `services/fs_romfs.cpp` | `CreateFileW`/`ReadFile`, UTF-16 conversion | `platform::File`, UTF-8 paths |
| `services/misc.cpp` | `LoadLibraryExW`/`GetProcAddress`, `GetModuleFileNameW` | `dlopen`/`dlsym`, `/proc/self/exe` |
| `kernel/host_place.cpp` | processor topology and ideal-processor APIs | `/sys/devices/system/cpu` + `thread_set_affinity` |
| `kernel/starvation_guard.cpp` | `NtQuerySystemInformation` thread states | POSIX scheduling equivalent, or a Linux guard |

Windows-only subsystems are left out of a Linux build, not stubbed: crash handler and minidump writer, symbol stack
walks, sampler, watchdog, memory guard, registry helpers, CPU/SMBIOS diagnostics, the gauge sampler, Media Foundation
movies, the XAudio2 sink, WASAPI device enumeration, the album (COM), mod DLL loading, the SEH guest unwinder and the
test tools. `CMakeLists.txt` lists them in `_nxrt_windows_only_sources`; `docs` and the list must stay in sync. A Linux
build of those features is real work with a Linux design (signals + `backtrace`/libunwind for crashes, `/proc` for
the sampler, PipeWire/SDL3 for audio, `libgcc`/custom unwinder for guest exceptions), not a shim.

Not started:

- the graphics shell on Linux (SDL3 window/input and Vulkan are portable, but `src/gfx/extras` carries DLSS/FFX/XeSS,
  the Win32 display-mode helper and Windows crash shortcuts);
- a native audio sink (SDL3 audio, so PipeWire/PulseAudio/ALSA are selected by SDL, matching the Steam Deck);
- the launcher's Linux build path (`apps/emblemweave/src/platform.rs` reports `build_supported = false` until the
  engine genuinely builds and runs natively, and `apps/emblemweave` is a Rust/egui application, which is portable);
- packaging (`.desktop`, icons, AppImage or tar, Steam library entry) and the Steam Runtime build.

## Architecture

Two processes make up a running installation:

- **EmblemWeave** (`apps/emblemweave`, Rust + egui) is the launcher and tool UI. Its OS questions go through
  `apps/emblemweave/src/platform.rs` (data folder, process list, dependency installers, engine build support, platform
  name). That module is already written per platform; the Linux branches are exercised by its unit tests, and
  `build_supported` stays false until the engine port is real.
- **The runtime** (`engine/fefw-recomp/runtime`, C++20) is the recompiled game: the NVN front end, the Horizon kernel
  and services, the SDL3/Vulkan graphics shell and the game module.

The runtime's OS boundary is `include/nxrt/platform.h`: memory (reserve/commit/protect/query), threads, mutexes and
conditions, read-write locks, clocks and sleeps, files, page-fault handlers, sections, affinity and priorities, and
process exit. `src/platform/win32` answers with VirtualAlloc/pthread-equivalents of Win32, `src/platform/posix` with
mmap/pthreads/clock_gettime/file descriptors. Nothing above that header includes `windows.h` by design; a Windows call
in a portable file is a bug.

## Building

Windows (the reference configuration, unchanged):

```bat
cmake --build build\release --target nxrt nxrt_gfx_tests -j
ctest --test-dir build\release --output-on-failure
```

Linux (Ubuntu 24.04 or a SteamOS desktop session; WSL2 works for compilation and the platform test):

```sh
sudo apt install build-essential cmake ninja-build git
cd engine/fefw-recomp/runtime
cmake -S . -B build/linux -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build/linux -j
ctest --test-dir build/linux --output-on-failure
```

The runtime base needs Vulkan and a display only for the graphics shell. `NXRT_BUILD_GFX` defaults to ON on Windows
and OFF elsewhere until the shell port lands; the same goes for `NXRT_BUILD_TESTS` (the runtime tests link Windows-only
diagnostics today). A full-runtime Linux attempt:

```sh
cmake -S . -B build/linux-full -G Ninja -DCMAKE_BUILD_TYPE=Release \
      -DNXRT_PLATFORM_ONLY=OFF -DNXRT_BUILD_GFX=OFF -DNXRT_BUILD_TESTS=OFF
```

Third-party sources (zstd, and SDL3/Vulkan/volk/VMA/ImGui when the shell is on) are fetched at configure time into
`$XDG_CACHE_HOME/nxrt/deps` (or `~/.cache/nxrt/deps`), `%LOCALAPPDATA%\nxrt\deps` on Windows, and pinned by URL hash.

## Steam Deck and SteamOS

- **Architecture**: x86-64, the same build as a desktop; no cross-compilation. Guest code is AArch64 and is lifted.
- **Filesystem**: case-sensitive. Paths that the Windows build resolved case-insensitively must be matched exactly;
  archive extraction and the RomFS reader must not rely on case folding. This is a porting bug to fix, not a Wine
  workaround.
- **Steam Runtime**: link only what the runtime provides (SDL3 and Vulkan are loaded through the runtime's own
  copies/loader; libc must be built against the oldest supported runtime, which is the packaging step).
- **Vulkan**: the Steam Deck's RADV driver is the primary target. The device layer already selects a queue family and
  surface from SDL3's extensions; presentation goes through `SDL_Vulkan_CreateSurface`, so X11/Wayland/Game Mode are
  handled by SDL.
- **Controllers and Steam Input**: SDL3's gamepad layer, already used by `src/gfx/input.cpp` and
  `platform::gamepad_poll`; Steam Input presents an Xbox-layout pad, which is the layout the runtime maps.
- **Game Mode**: the window is SDL3 + borderless/fullscreen; no exclusive-mode or desktop-resolution assumptions in the
  portable paths. Display scaling uses `pixel_size` (drawable pixels), not window coordinates.
- **Audio**: a native sink is required; the plan is SDL3 audio (PipeWire on SteamOS, PulseAudio/ALSA elsewhere), behind
  the existing `AudioSink` interface so the audio services and tests do not change.
- **Memory**: the guest address space lives at fixed low addresses; Linux allows the same `mmap(MAP_FIXED)` layout.
  `vm.max_map_count` and overcommit settings must be documented for Deck users if the heap aliasing needs them.
- **Handheld detection**: `host_device.cpp` reads `/sys/class/dmi/id/product_name` and `SteamDeck=1`, which is what
  SteamOS sets in Game Mode, so Deck tuning works without the registry.

## Verification

| Check | Windows | Linux |
| --- | --- | --- |
| Platform layer builds | yes | yes (GCC 13, Clang 18, WSL2) |
| Platform self-test | yes | yes |
| Core runtime builds | yes | 6 source files left (frontier above) |
| Runtime tests | yes | not yet configured |
| Window + Vulkan | yes | no |
| Audio device | yes (XAudio2) | no |
| Guest execution | yes | no |

"Compiles" is not "works": a subsystem is called working here only after it has run. The Steam Deck itself has not run
this build; WSL2 proves compilation and the platform contract, not RADV, PipeWire, Game Mode or Steam Input.

## Roadmap

1. **Core link on Linux**: port the frontier table above (`sync` first, then files, then the module loader), so `nxrt`
   links natively. Verify with a Linux build of the runtime tests that do not need the graphics shell.
2. **Graphics shell**: port `src/gfx` with the vendor upscalers off, the Win32 display helper behind the platform, and
   SDL3 window/input/Vulkan surface as-is. Add the Linux equivalents of the Windows-only paths (crash shortcut,
   display modes).
3. **Audio**: SDL3 `AudioSink` implementation, device selection, `audio_devices` enumeration through SDL, and the same
   underrun/queue behaviour as XAudio2.
4. **Launcher**: build `apps/emblemweave` on Linux, point it at the native engine build, flip `build_supported` and the
   dependency-install messages only after steps 1-3 run. Data folder is already `$XDG_CONFIG_HOME/EmblemWeave`.
5. **SteamOS/Deck**: Game Mode launch, controller-only navigation, 1280x800 and 1920x1080 scaling, performance
   defaults, and a Deck smoke test on hardware.
6. **Packaging**: `.desktop` file, icon, a self-contained directory or AppImage built in the Steam Runtime, and a
   Steam shortcut manifest.

Each step lands on `main` with the Windows build kept green.
