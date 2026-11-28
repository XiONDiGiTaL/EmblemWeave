# Linux implementation plan

**Branch:** `linux/compat-prototype` in worktree `ew-linux`, based provisionally on `22a380c`.
**Current classification:** **PARTIAL IMPLEMENTATION, GAMEPLAY TESTING PENDING.**
Nothing here is merged, pushed or offered as a release.

## 1. What this branch actually changed

| File | Change | Why |
| --- | --- | --- |
| `runtime/include/nxrt/platform.h` | `PROT_NONE/READ/WRITE/EXEC` are declared only when `<sys/mman.h>` has not already defined them | The header's constants collided with the POSIX macros in every Linux translation unit; nine compile errors started here |
| `runtime/src/platform/posix/platform_posix.cpp` | Added `<sys/resource.h>`, `<cstdio>`, `<sys/file.h>`, `<ucontext.h>`; removed a non-existent `Thread::id`; per-thread `setpriority`; `sched_getcpu`; flock-based named mutex; `MAP_FIXED_NOREPLACE`; SIGSEGV handler distinguishes read/write and re-raises when unhandled | The backend did not compile, and four of its functions had contract bugs that would surface at runtime |
| `runtime/src/platform/win32/platform_win32.cpp` | Imported FCC's WIP snapshot; added `<vector>` | The imported header declares functions the committed Win32 file does not implement; without this file the Windows platform build does not link. The MSVC build then found the missing include. |
| `runtime/tests/platform_selftest.cpp` | Protection test passes `PROT_READ` / `PROT_READ|PROT_WRITE` and writes after restoring write access | The old test passed `false`/`true` as a mask; against the mask-based backend it would fault |
| `runtime/CMakeLists.txt` | Platform-only builds link `Threads::Threads`; new `nxrt_linux_probe` target | pthreads must be explicit; the probe is the diagnostic entry point |
| `runtime/tools/linux_probe.cpp` | New: kernel, page size, cores, XDG roots, Vulkan loader and physical devices | A Linux bug report needs these facts; it also exercises the platform layer |
| New Linux documentation pages | Added alongside the workstream | The audit, architecture, plan and build guide |

The three imported files (`platform.h`, `platform_posix.cpp`, `platform_win32.cpp`) come from FCC's uncommitted
working tree; see the audit's provenance note. Every edit above is in `ew-linux` only.

## 2. Phase plan

Each phase ends with something that was built and run. A phase is not done because it compiles.

### P0 — Foundation (done in this branch)

* POSIX platform layer compiles with clang 21 on Ubuntu 26.04.
* `platform_selftest` passes (1/1 CTest).
* `nxrt_linux_probe` builds and reports a real Vulkan device (lavapipe) and the XDG roots.
* Launcher `cargo check -p emblemweave --target x86_64-unknown-linux-gnu` passes natively (0 errors).
* After the GUI packages were installed, the debug launcher **links** and opens a 1440x900 window under WSLg on both
  X11 and Wayland (Wayland surface configure/ack observed; no errors).
* The same platform layer still builds and passes its self-test with MSVC/clang-cl on Windows, so the shared header and
test change did not regress the Windows side.

**Acceptance met.** This is the baseline every later phase builds on.

### P1 — Launcher runs on a Linux desktop (started)

1. ~~Install the GUI development packages and build the launcher.~~ Done: debug link verified (release not attempted).
2. ~~Open a window on X11 and Wayland.~~ Done under WSLg: 1440x900 X11 window, and a Wayland `xdg_toplevel` with
   configure/ack on both backends.
3. Add a Linux `build_supported()` reason: "the engine build is not available on Linux yet", with the existing
   Requirements page naming the missing pieces.
4. Route every write through `platform.rs`; verify no `%APPDATA%` string remains on the Linux path.
5. Replace `explorer`/`notepad` with `xdg-open`/`$EDITOR`; `tasklist`/`taskkill` with `/proc`.
6. Open Game Setup, read a folder, confirm the build refusal is the only blocked action and that settings persist.

**Exit:** a window opens, settings persist under `$XDG_CONFIG_HOME`, and Play is refused with an accurate sentence.
**Remaining:** steps 3–6, a release link, and a real desktop (WSLg is a software-GL launch, not a desktop GPU test).

### P2 — Native engine platform layer

1. Move the remaining thread/sync/file/clock call sites onto `nxrt::platform` (start with `kernel/thread.cpp`,
   `kernel/starvation_guard.cpp`, `svc/`, `diag/log.cpp`).
2. Replace the exception path: the runtime's own handler for guest faults, `backtrace`/libunwind for host crashes.
3. Port `dlopen`-shaped library loading (Vulkan loader, upscaler probing).
4. Make `cmake/gfx.cmake` toolchain-aware: no `/MAP`, `/DEBUG`, `/STACK` on Linux; link `dl`, `pthread`, `vulkan`.
5. Build `nxrt` and `nxrt_tests` (not only the platform layer) with clang; run the game-independent test groups.

**Exit:** `nxrt_tests` and `nxrt_gfx_tests` build and pass on Linux, with SDL3 + Vulkan present.
**Blocked by:** the 700-call surface; this is the largest single phase.

### P3 — Lifted code and a first frame

1. Teach the recompiler's `build_game.cmd` equivalent to emit ELF objects with clang and `llvm-ar`.
2. Add the ABI shim for generated code (`__declspec`, struct packing, `#pragma comment`, symbol decoration).
3. Link `fefw` for Linux and boot the test dump to the first frame.
4. Capture the frame and compare it with the Windows reference.

**Exit:** the game reaches its first rendered frame with the test data.
**Blocked by:** P2 and the recompiler output format; this is the highest-risk phase.

### P4 — Playable Linux build

1. Input (SDL gamepad + keyboard/mouse), audio (PipeWire/ALSA), saves/profiles, shader caches.
2. Video playback through `movie_ffmpeg.cpp`; Opus audio through the existing decoder.
3. Diagnostics: crash handler, log roots, `nxrt_linux_probe` attached to bug reports.
4. A full session on desktop Linux with save/reload, audio, cinematic, extended play and clean shutdown.

**Exit:** the Linux validation list passes on at least one AMD and one NVIDIA desktop.
**Blocked by:** hardware access (none in this environment).

### P5 — Steam Deck

1. Prebuilt-engine install/verify flow in the launcher (no compiler on device).
2. Gamescope-compatible fullscreen, Steam Input mapping, suspend/resume, shader-cache location.
3. A Steam library entry that launches without a terminal.
4. Real-device test in Game Mode and Desktop Mode.

**Exit:** a Deck session, not a desktop session on a Deck-shaped VM.
**Blocked by:** no Deck hardware.

## 3. Priorities

1. Finish the platform boundary (P2) — it removes 531 of 700 calls and is testable without a GPU.
2. Make the launcher a truthful native artifact (P1) — small, visible, low risk.
3. Lift the game for ELF (P3) — the real gate to gameplay.
4. Audio/media/saves (P4) — needed for "playable", not for "renders".
5. Deck packaging (P5) — last, because it depends on everything else.

Proton mode is orthogonal: it can be offered earlier as an explicit compatibility option, but it must be tested on a
real Steam installation before it is advertised.

## 4. Known blockers

| Blocker | Impact | Workaround |
| --- | --- | --- |
| FCC base revision unconfirmed | Integration may need a rebase | Provisional `22a380c`; document the imported WIP snapshot |
| Imported POSIX files are uncommitted in FCC's tree | No commit identity | FCC should commit them; this branch's copies are fixes, not the source of truth |
| 700 Windows calls / 80 `<windows.h>` files | Native engine build impossible today | P2, subsystem by subsystem |
| MSVC-lifted `.lib` objects | No ELF link | P3 ABI shim + recompiler work |
| No GPU passthrough in WSL (lavapipe only) | No rendering/performance test | Real desktop or Deck required |
| No Steam Runtime / pressure-vessel in WSL | Full Proton session not established | Test on a Steam installation |
| No Steam Deck | Deck claims impossible | Mark pending |
| WSLg has no passed-through GPU | The launcher opens, but this is not a desktop GPU or render test | Use a real desktop or Deck for performance and gameplay |

## 5. Testing requirements

The minimum bar for "supported" is:

* clean install and first launch on AMD and NVIDIA desktops;
* game-data discovery and compilation;
* startup, title screen, actual gameplay, save/load, audio, cinematic;
* shader compilation and cache reuse across launches;
* extended play and clean shutdown;
* X11 and Wayland;
* Proton mode on the same titles, if it is advertised;
* Steam Deck in Game Mode.

A CMake build, a launcher window, or a title-screen render is not sufficient on its own.

## 6. Integration with FCC

Do not merge or push this branch. When FCC is ready:

1. FCC confirms the base revision and commits (or discards) its WIP POSIX backend.
2. Re-apply this branch's fixes on top of that commit; drop the imported copies if FCC's version already carries them.
3. Review the platform-contract changes first (header, selftest, CMake), then the probe and docs.
4. Run the Windows build and the Windows platform self-test before merging: the header and CMake changes are shared.
5. Only then decide whether Proton mode ships as an option.

## 7. Reproduction

`docs/LINUX_BUILD.md` has the exact package list, commands, expected output and the Proton launch configuration.
