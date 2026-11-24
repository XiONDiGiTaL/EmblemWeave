# Native Linux Build Status

**Date:** 2026-11-24
**Commit:** `9418e7ad` — `linux: make the write-watch implementation compile`
**Host:** WSL2 Ubuntu 24.04, Linux 6.6.87.2-microsoft-standard-WSL2, x86-64
**Toolchain:** clang 21.1.8, CMake, Ninja (distribution packages)

## 1. What builds

```bash
cd engine/fefw-recomp/runtime
cmake -S . -B build_wsl -G Ninja -DCMAKE_BUILD_TYPE=Release
cmake --build build_wsl -j 8
ctest --test-dir build_wsl --output-on-failure
```

Result:

```
[4/5] Linking CXX executable nxrt_linux_probe
1/1 Test #1: platform ... Passed   0.01 sec
100% tests passed, 0 tests failed out of 1
```

The produced targets are `libnxrt_platform.a`, `nxrt_platform_tests` and `nxrt_linux_probe`. This is the POSIX platform
layer only: `CMakeLists.txt` sets `NXRT_PLATFORM_ONLY` on non-Windows and returns before the full runtime, because the
graphics shell, the game module and the launcher are still Windows-only.

`nxrt_linux_probe` on this host:

```
vulkan loader: libvulkan.so.1 opened
loader api version: 1.4.341
vulkan devices: 1
  [0] llvmpipe (LLVM 21.1.8, 256 bits) | cpu | api 1.4.335 | driver 0x06800008
```

## 2. What did not compile, and why

`src/platform/posix/write_watch_linux.cpp` had **never been compiled**: the Windows build never sees it, and the POSIX
build had never been run. It failed with 20 errors:

- missing `<memory>` and `<thread>` (`std::unique_ptr`, `std::make_unique`, `std::this_thread::yield`);
- `nxrt::platform::PROT_READ` / `PROT_WRITE` / `PROT_NONE`: these are `<sys/mman.h>` macros, so the qualified name
  expanded to `nxrt::platform::0x1` and could not parse. Replaced with the numeric values the file already uses;
- `UFFD_NONBLOCK`, which `linux/userfaultfd.h` does not define; the open flag is `O_NONBLOCK`.

After those fixes it compiles and links. That is a **build** result: the userfaultfd path is not exercised by the platform
self-test, and no Linux game run, Vulkan render or Proton comparison was made.

## 3. Remaining Windows dependencies

The full runtime still cannot be configured on Linux: `nxrt` links `psapi` unconditionally, `src/diag/registry.cpp`
includes `<windows.h>`, and the graphics shell uses Win32 APIs directly. `docs/LINUX.md`, which `CMakeLists.txt`
references, does not exist in the tree. Phase 2 of that document (the platform layer) is what builds today; phase 3 (the
rest of nxrt) has not been started.

No claim of Linux or Steam Deck support follows from this report, and nothing here is evidence about Proton compatibility:
that is a separate path in which a Windows build runs under a compatibility layer.
