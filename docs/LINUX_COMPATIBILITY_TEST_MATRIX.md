# Linux compatibility test matrix

This page records **what was actually run**, on which host, with the output. A cell is only PASS when a command or a
session produced evidence; everything else is PENDING or NOT RUN, including things that are likely to work.

## 1. Environments used

| Environment | Detail |
| --- | --- |
| Host | Windows 11, project at `C:\Users\xiond\.config\freebuff-desktop\local-projects\413b2383-4be0-4b3c-b36e-5f485402a327` |
| Linux | WSL2 Ubuntu 26.04.1 LTS, kernel `6.6.87.2-microsoft-standard-WSL2`, x86_64 |
| GUI stack | WSLg: Weston compositor, Xwayland, `DISPLAY=:0`, `WAYLAND_DISPLAY=wayland-0`, `XDG_RUNTIME_DIR=/run/user/1000` |
| GUI packages | `libxkbcommon-dev`, `libwayland-dev`, `wayland-protocols`, `libx11-dev`, `libxcb*-dev`, `libxcursor-dev`, `libxi-dev`, `libxrandr-dev`, `libegl1-mesa-dev`, `libgl1-mesa-dev`, `libfontconfig1-dev`, `libfreetype-dev`, plus `x11-utils`, `xdotool`, `mesa-utils` |
| Compiler | Ubuntu clang 21.1.8 |
| Build tools | CMake 4.2.3, Ninja 1.13.2, pkg-config 2.5.1 |
| Vulkan | loader 1.4.341, Mesa ICDs present; device: `llvmpipe (LLVM 21.1.8, 256 bits)`, API 1.4.335, vendor `0x10005` |
| Display | WSLg: `DISPLAY=:0`, `WAYLAND_DISPLAY=wayland-0` |
| Rust | rustc 1.100.0, cargo 1.100.0 |
| Proton inspected | `X:\SteamLibrary\steamapps\common\Proton Hotfix`, build `1785167113 hotfix-20260727-d4`, Wine `wine-11.0`, 1.5 GiB |
| GPU | **None passed through.** lavapipe is a CPU Vulkan implementation; it is not a gameplay target. |
| Steam Deck | **Not available.** |
| Windows host | Visual Studio 18 Community, LLVM clang-cl 21.1.8, CMake + Ninja; used to check that the shared platform change still builds MSVC |

## 2. Results

| Environment | Build | Launcher | Vulkan | Engine | Gameplay | Saves | Audio | Video |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| WSL2 Ubuntu 26.04 (lavapipe) | PASS (platform layer) | PASS (linked + window, X11 and Wayland) | PASS (enumerate) | NOT RUN (blocked) | PENDING | PENDING | PENDING | PENDING |
| Windows 11 (clang-cl/MSVC) | PASS (platform layer) | N/A | N/A | N/A | N/A | N/A | N/A | N/A |
| Desktop Linux, AMD | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| Desktop Linux, NVIDIA | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| Desktop Linux, Intel | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| Steam Deck | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| Proton Hotfix | N/A | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN | NOT RUN |
| Wayland | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |
| X11 | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING | PENDING |

For the launcher, PASS now means the debug binary links and its first window opens; it does not mean the UI was
clicked through or that a release build was made.

## 3. Evidence

### 3.1 Platform layer build and test — PASS

```
$ cmake -S ew-linux/engine/fefw-recomp/runtime -B ~/ew-linux/build-platform -G Ninja \
        -DCMAKE_C_COMPILER=clang -DCMAKE_CXX_COMPILER=clang++ -DCMAKE_BUILD_TYPE=Release
-- nxrt: platform layer only (.../src/platform/posix); the full runtime is Windows-only, see docs/LINUX.md

$ cmake --build ~/ew-linux/build-platform
[4/5] Linking CXX executable nxrt_platform_tests
BUILD_EXIT=0

$ ctest --test-dir ~/ew-linux/build-platform --output-on-failure
1/1 Test #1: platform .........................   Passed    0.01 sec
100% tests passed, 0 tests failed out of 1
```

The test covers page size, reserve/commit/protect round trip, monotonic clock and sleep, thread + condition variable,
thread id, file create/write/read/seek/close, `file_mkdirs`, and a missing-file open.

### 3.2 Linux probe — PASS

```
$ ~/ew-linux/build-platform/nxrt_linux_probe
nxrt linux probe
kernel: Linux 6.6.87.2-microsoft-standard-WSL2 x86_64
page size: 4096
cores: 8 physical, 16 online
config root: /home/xiondigital/.config/EmblemWeave
cache root: /home/xiondigital/.cache/EmblemWeave
vulkan loader: libvulkan.so.1 opened
loader api version: 1.4.341
vulkan devices: 1
  [0] llvmpipe (LLVM 21.1.8, 256 bits) | cpu | api 1.4.335 | driver 0x06800008 | vendor 0x10005 device 0x0000
PROBE_EXIT=0
```

This proves the loader is discoverable and device enumeration works on Linux. It does **not** prove the engine's
renderer, because the engine was not built.

### 3.3 Launcher: type-check, link and window — PASS

Type-check, before the GUI packages were installed:

```
$ source ~/.cargo/env && cd ew-linux
$ CARGO_TARGET_DIR=~/ew-linux/cargo-target cargo check -p emblemweave --target x86_64-unknown-linux-gnu
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 49.50s
EXIT=0
```

0 errors, 139 warnings (the same warnings the Windows build reports; not introduced by this work).

After installing the Wayland/X11 development packages, the debug binary **links**:

```
$ CARGO_TARGET_DIR=~/ew-linux/cargo-target cargo build -p emblemweave --target x86_64-unknown-linux-gnu
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 58.31s
BUILD_EXIT=0

$ file ~/ew-linux/cargo-target/x86_64-unknown-linux-gnu/debug/EmblemWeave
ELF 64-bit LSB pie executable, x86-64, version 1 (SYSV), dynamically linked,
interpreter /lib64/ld-linux-x86-64.so.2, for GNU/Linux 3.2.0, with debug_info, not stripped

size: 376652016 bytes
```

This is a debug build; a release link was not attempted.

**X11 launch — PASS.** With `WAYLAND_DISPLAY` unset, `DISPLAY=:0`, `XDG_RUNTIME_DIR=/run/user/1000`:

```
PROCESS=RUNNING            (still alive after 12 s)
WINDOW_ID=6291460          (xdotool search --name EmblemWeave)
name: EmblemWeave
geometry: 1440x900
log: empty
```

The same result was obtained with `LIBGL_ALWAYS_SOFTWARE=1 MESA_LOADER_DRIVER_OVERRIDE=llvmpipe` and without it.
`xwininfo -root -tree` shows the window as a 1440x900 child of a 1504x964 frame, titled `EmblemWeave`.

**Wayland launch — PASS.** With `DISPLAY` unset, `WAYLAND_DISPLAY=wayland-0`, and `WAYLAND_DEBUG=1`:

```
PROCESS=RUNNING
the client creates wl_surface, xdg_surface and xdg_toplevel, sets the title "EmblemWeave"
and min size 1120x720, and completes 4 xdg_surface.configure / ack_configure cycles
no EGL, Mesa or protocol errors in the log
```

The same run also succeeds without `WAYLAND_DEBUG`; the trace is how the surface creation was observed, since an
X11 tool such as `xdotool` cannot see a native Wayland window.

**One failure mode was observed and is reproducible in this environment.** The first attempt exported both
`DISPLAY` and `WAYLAND_DISPLAY` without `MESA_LOADER_DRIVER_OVERRIDE`; the process stayed alive but printed:

```
libEGL warning: failed to get driver name for fd -1
libEGL warning: MESA-LOADER: failed to retrieve device information
MESA: error: ZINK: failed to choose pdev
libEGL warning: egl: failed to create dri2 screen
```

No X11 window existed, because winit had selected Wayland; that surface was not visible to `xdotool`. Isolated X11
and Wayland runs then completed as above. The launcher was not clicked through or exercised beyond opening its first
window; this is a launch test, not a UI test.

### 3.4 Proton Wine — INCONCLUSIVE

```
$ cd "/mnt/x/SteamLibrary/steamapps/common/Proton Hotfix"
$ ./files/bin/wine --version
wine-11.0

$ WINEPREFIX=$HOME/ew-linux/pfx WINEDEBUG=-all ./files/bin/wine cmd /c echo wine-exec-ok
<no output; wineboot.exe --init and winedevice.exe still alive after 180 s; killed>
```

The prefix was created (`drive_c`, `system.reg`, `user.reg`, `userdef.reg`, `dosdevices`), so Wine initialised part of
the way. The command did not complete. This is expected for a Proton build run outside the Steam Linux Runtime:
`proton` requires `STEAM_COMPAT_CLIENT_INSTALL_PATH`, `STEAM_COMPAT_DATA_PATH` and pressure-vessel. It is **not**
evidence that EmblemWeave fails under Proton, and it is **not** evidence that it works.

### 3.5 Engine build — NOT RUN

`NXRT_PLATFORM_ONLY` is forced when `NOT WIN32`, so CMake never reaches the rest of `nxrt`. The audit found 80 files
including `<windows.h>` and 700 Windows calls; `cmake/gfx.cmake` still passes MSVC linker flags. A full Linux build
would fail before linking, so it was not attempted. This is a known blocker, not a surprise.

### 3.6 Windows regression check — PASS

The platform header, the self-test and CMake are shared with Windows, so the same platform-only target was built there
with Visual Studio 18's environment and LLVM's clang-cl:

```
$ cmake -S ew-linux/engine/fefw-recomp/runtime -B %TEMP%\ew-win-build -G Ninja \
        -DCMAKE_C_COMPILER=clang-cl.exe -DCMAKE_CXX_COMPILER=clang-cl.exe \
        -DCMAKE_LINKER=lld-link.exe -DNXRT_PLATFORM_ONLY=ON
$ cmake --build %TEMP%\ew-win-build
$ ctest --test-dir %TEMP%\ew-win-build --output-on-failure
1/1 Test #1: platform .........................   Passed    0.07 sec
100% tests passed out of 1
```

This check also found that the probe target needed a `NOT WIN32` guard (it includes `dlfcn.h`), which is now in the
CMake file. The Linux probe still builds and runs as shown in 3.2.

## 4. Defects found and fixed in this branch

All were found by the Linux compiler or by reading the POSIX backend against its contract.

| # | Defect | Effect | Fix |
| --- | --- | --- | --- |
| 1 | `platform.h` declared `PROT_*` constants that `<sys/mman.h>` defines as macros | 4 compile errors in every Linux TU that includes both | Declare only when the macro is absent |
| 2 | `thread_self()` assigned `self.id`, a member `Thread` does not have | Compile error | Removed; the id is `thread_id()` |
| 3 | `<sys/resource.h>` missing | `getrusage`, `RUSAGE_THREAD`, `setpriority`, `PRIO_PROCESS` undeclared | Include added |
| 4 | `syscall(SYS_getcpu, ...)` used as the CPU number | Always reports CPU 0 on success | `sched_getcpu()` |
| 5 | `setpriority(PRIO_PROCESS, 0, ...)` set the process, not the named thread | Thread priorities leaked to the launcher | `pthread_gettid_np` per-thread nice |
| 6 | Named mutex was a named semaphore, never unlinked | `/dev/shm` leak; not a mutex | `flock` on a file under `XDG_RUNTIME_DIR`/`/tmp` |
| 7 | `section_map_view` used `MAP_FIXED` | Silently replaced an existing guest mapping | `MAP_FIXED_NOREPLACE` |
| 8 | SIGSEGV handler always reported "write" and returned when unhandled | Write-watch saw false writes; an unhandled fault looped forever | Read `REG_ERR` bit 1; restore `SIG_DFL` and re-raise |
| 9 | Self-test passed `false`/`true` where the API takes a protection mask | Read a `PROT_NONE` page (faults on the mask-based backend) | Pass `PROT_READ` and `PROT_READ|PROT_WRITE`, then write |
| 10 | Platform-only build did not link pthreads | Fragile on distributions where libc does not re-export pthreads | `find_package(Threads)` + `Threads::Threads` |

## 5. Tests still pending

| Test | Why pending |
| --- | --- |
| Launcher interaction (menus, Game Setup, settings persistence) | Window opened, but no clicks were performed |
| Launcher release link | Only the debug profile was linked |
| Native engine build | 700 calls, MSVC-lifted libs, MSVC linker flags |
| First frame / title screen | Depends on the engine build |
| Actual gameplay | Depends on the first frame |
| Save load and persistence | Depends on gameplay |
| Audio output | Depends on gameplay; PipeWire/ALSA backend not written |
| Cinematic playback | Depends on gameplay; FFmpeg path not wired on Linux |
| Shader compilation and cache reuse | Depends on the renderer |
| Extended play / shutdown / repeated launches | Depends on gameplay |
| AMD desktop (RADV) | No hardware |
| NVIDIA desktop | No hardware |
| Intel desktop | No hardware |
| Steam Deck Game/Desktop Mode, controller, suspend | No hardware |
| Proton launcher + engine session | No Steam Runtime/pressure-vessel; Wine smoke test incomplete |
| Foreign-driver cache invalidation | Depends on the renderer |

## 6. What would change the classification

* **PROTON COMPATIBILITY VERIFIED** requires a real Steam installation where `EmblemWeave.exe` launches through
  Proton, reaches gameplay, loads a save, plays audio and survives an extended session. A launcher window is not
  enough.
* **NATIVE LINUX BUILD AND GAMEPLAY VERIFIED** requires P2–P4 above, on at least one AMD and one NVIDIA desktop,
  with saves, audio and cinematics.
* **ARCHITECTURE COMPLETE, IMPLEMENTATION BLOCKED** would require the platform work to be impossible without a
  redesign; it is not, because the layer now builds.
