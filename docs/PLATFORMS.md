# Platforms

EmblemWeave runs on Windows. This page is the audit of what ties it there, what has been separated out so a
build for another system cannot do the Windows thing by accident, and what is still Windows-only. The plan for
Linux, including what has been built and run already and what has not, is [LINUX.md](LINUX.md).

The launcher now type-checks for Linux: `cargo check -p emblemweave --target x86_64-unknown-linux-gnu` exits 0
(rustc 1.98.1, GPL crates and `eframe` included). That is a compile, not a running program, and it is not a claim
that the game plays there. What it does prove is that the Windows-only pieces are behind `cfg` and behind the one
module that decides.

## What the launcher asks the operating system for

| Question | Windows answer | Where it lives now | Off Windows |
| --- | --- | --- | --- |
| Is the game running? | `CreateToolhelp32Snapshot` over the process list, `OpenProcess` for memory | `cheats::process` | no process list: `platform::game_running()` is always false, and the Cheats page finds no game and writes nothing (`cheats::process` has a stub `Process`) |
| May the launcher install build tools? | `winget` lines run from an install `.cmd` | `engine::start_install_lines` guards with `platform::install_supported()` | refused, with a sentence naming the platform and pointing at the distribution's packages |
| Can this platform build the game? | Visual Studio's `clang-cl` through `vcvars64.bat`, CMake, Ninja | `engine::build` guards with `platform::build_supported()` | refused before the job starts, not ninety steps later |
| Where does the launcher keep its data? | `%APPDATA%\EmblemWeave` | `platform::data_root()`, used by `Config::dir()` | `$XDG_CONFIG_HOME/EmblemWeave`, else `$HOME/.config/EmblemWeave` |
| Which Vulkan loader? | `vulkan-1.dll`, installed by the graphics driver and found through the system search path; the engine's own loader opens it, and a missing one shows in its log as "no Vulkan loader" | nothing in the launcher: the name is only prose, because the launcher never loads Vulkan itself | `libvulkan.so.1` from the distribution's loader package (macOS would be `libvulkan.1.dylib`), in `ld.so`'s path |
| What is this platform called? | "Windows" | `platform::name()` | "Linux", "macOS" |

`docs/LINUX.md` says what each of these still needs before a native build exists. Nothing in the table is a
feature: three of the six are refusals with an explanation, which is the point of doing the audit before the port.

## The audit

Counts are from the tree at this commit, so they can be checked with the command beside them.

### Process creation

`Command::new` appears 28 times in `apps/emblemweave/src` (`grep -rc Command::new apps/emblemweave/src/*.rs`):
`jobs.rs` 9, `engine.rs` 6, `toolchain.rs` 6, `updater.rs` 4, `mods.rs` 2, `deps.rs` 1. The programs are the
build's own tools (`pwsh`, `cmake`, `ninja`, `clang-cl`, `dotnet`, `git`, `robocopy`) plus desktop conveniences
also reachable from a cross-platform build (`explorer` 7, `notepad` 2), and two Windows-only queries (`tasklist`,
`taskkill`). `jobs.rs` is where a step's command line becomes a child process, and it is also where
`CREATE_NO_WINDOW` and the log redirection live.

What changed: the two entry points that *act* on the system without the player pressing anything else, the
dependency installer and the game build, refuse off Windows. What did not change: the command lines themselves
are still written in Windows terms (`pwsh -File ...`, `cmake --build ...`). A Linux port replaces the shell and
the compiler in `jobs.rs`, not the steps; that is phase 2 in [LINUX.md](LINUX.md).

### Windows-specific paths

`%APPDATA%` and `%LOCALAPPDATA%` appear in `config.rs` (4), `engine.rs` (5), `jobs.rs` (4), `saves.rs` (3),
`mods.rs` (2), `main.rs` (2), `gamedata.rs` (1), `snapshots.rs` (1), `toolchain.rs` (1). They are three kinds:

* the launcher's own folder, which is now `platform::data_root()` in one place (`Config::dir()`), the same
  `%APPDATA%\EmblemWeave` on Windows as before;
* the engine's own caches (shader SPIR-V, the BC7/ASTC texture cache), which are *the engine's* folders and are
  passed to it as paths in `fefw.ini`, so a port gives it the port's cache folders;
* prose in messages, which now goes through `platform::NO_DATA_DIR` where it is an error and is written out in full
  where it is a sentence.

### Registry and Visual Studio discovery

`toolchain.rs` is the Windows toolchain's map: `vswhere.exe`, `VC\Tools\MSVC`, `Windows Kits\10`, the SDK's
`um\windows.h` and `x64\kernel32.lib`, LLVM's `clang-cl.exe` and `lld-link.exe`, and the probes that decide
whether a version is new enough (`28` references in `grep -c "Visual Studio\|Windows Kits\|vswhere"`). None of it
reads the registry directly: it asks `vswhere`, which is the documented way, and falls back to the usual folders.

What changed: no code moved, because nothing off Windows calls it now. `engine::build` and the Requirements page
are its only callers in a path that starts a job, and the build refuses first. The Requirements page still shows
the probes on another platform, which is honest: it lists what the Windows build needs, and the page's heading
says the build is Windows-only.

### `winget`

38 references in `apps/emblemweave/src`, all from one registry: `deps.rs` gives each dependency an `Install`
(`Winget` line, `Manual` text, or a pip command), and `docs/TROUBLESHOOTING.md` and `README.md` print the same
table (`deps::markdown()` has a test that fails if the docs and the registry disagree). Two things changed:

* `engine::start_install_lines`, the only place that runs them, refuses when `platform::install_supported()` is
  false. That covers the Overview button, `--install` style entry points and anything else that calls it.
* The launcher never runs an install line implicitly: only that one function does, and only when the button is
  pressed.

### Windows-only APIs

| API | Used for | File | Off Windows |
| --- | --- | --- | --- |
| `ReadProcessMemory`, `WriteProcessMemory`, `VirtualQueryEx`, `OpenProcess`, the process snapshot | cheats and the save editor's live values | `cheats.rs` | stub `process` module: `find` returns none, so nothing is written |
| `XInputGetState` | the Controls page's live pad and its button test | `pad.rs` | `poll()` returns none; the page says no pad is connected |
| `LoadLibraryExW`, `FindResourceW`, `SHChangeNotify` | the icon `Build the game` writes into `fefe.exe` | `exe_icon.rs` | the module is `#[cfg(windows)]`; the build's icon step is skipped |
| `GetKeyState` | cheat codes that test a button | `cheats::held_keys` | returns 0, so a button reads as up |
| dbghelp and the process list | the crash report | `crash.rs` | the report is skipped; the log still has the error |

The engine is a separate story and the bigger one: 69 files under `engine/fefw-recomp/runtime/src` include
`<windows.h>`, with `VirtualAlloc` (14), `CreateThread` (15), `CreateFileW` (7), `GetModuleHandleW` (3) and
`QueryPerformanceCounter` (28) at the top. Its window is SDL3 (`gfx/platform.cpp`), which is already portable;
its memory, threads, files and clock are not.

### Vulkan loader and driver discovery

The engine loads Vulkan itself (the `volk`-style loader in `src/gfx/vk_device.cpp`): `vulkan-1.dll` from the
system search path, then the instance and physical-device enumeration, then the log lines this project's
diagnostics read (`GPU 0: ... vendor 0x10de, device ...`, `device ready: ...`). The launcher does not load
Vulkan; it reads the engine's log (`engine::live()`) and lists `upscalers\` DLLs next to the game. Off Windows
the loader file name is `libvulkan.so.1` (`platform::vulkan_loader_file()`), which a distribution's
`libvulkan1`/`vulkan-loader` package provides; whether the engine's loader can be built against it is phase 2.

### External commands

The build's steps call `pwsh`, `cmake`, `ninja`, `clang-cl`, `lld-link`, `dotnet`, `git`, `robocopy` and `tar`
(`tools/package_release.ps1`). Release packaging is PowerShell as well, because it is the release machine's
shell. A Linux release step would be a script of its own that calls the same `tools/verify_archive.py`, which is
the part that has to agree between the two.

## What is verified, and what is not

* Verified here: `cargo test -p emblemweave` on Windows (112 tests), and `cargo check -p emblemweave --target
  x86_64-unknown-linux-gnu` exiting 0. The Linux check compiles the launcher including `eframe`, so the
  separation is real code, not a comment.
* Verified here: the packaged translator builds for Linux (`dotnet publish -r linux-x64`) as an ELF binary and
  runs under WSL Ubuntu 26.04 (`spirv_harness --help`prints its usage), and the release zip extracts under Linux
  with both `unzip` and Python's `zipfile` with every nested path intact (see [PACKAGE.md](PACKAGE.md)).
* Not verified: the launcher running on Linux (no window toolkit test yet, and Game Setup's build path is
  refused by design), the engine on Linux (it does not build), SteamOS and Steam Deck, and any Vulkan
  presentation outside Windows.
* Not claimed: native Linux support. [LINUX.md](LINUX.md) is a plan with verified pieces in it, not a release.
