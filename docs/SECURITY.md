# Security and what EmblemWeave runs

This page is the detailed version of the security summary on the main README. It lists every program the launcher starts, why it starts it, and the only network destinations it uses.

## The short version

EmblemWeave is one Rust executable plus the engine folder it drives. It installs no service, driver or browser
extension, ships no PHP, Python or AI runtime of its own, has no accounts, and uploads nothing. Saves, settings, mods,
logs and the decoded copy of your game all stay on your PC, in `data` beside `EmblemWeave.exe` in a portable install or
under `%APPDATA%\EmblemWeave` in a profile install.

Everything the launcher starts, and why:

| Program | When | Why |
| --- | --- | --- |
| `vswhere.exe` | start-up, build | finds Visual Studio's C++ tools |
| `dotnet --list-sdks` | start-up, build | reads the installed SDK version |
| `spirv_harness.exe` | Build the game | the shader translator bundled in the zip; it carries its own .NET runtime and opens no network connection |
| `dotnet build` | Build the game, only when the zip has no translator | builds the translator from the staged sources |
| `git clone`, `git checkout` | Build the game | fetches the pinned Ryubing revision the shader translator is ported from |
| `pwsh` | Build the game | runs the engine's build scripts (`-NoProfile -ExecutionPolicy Bypass`) |
| `cmd /C recomp\scripts\build_game.cmd` | Build the game | drives the engine's own build |
| `cmake`, `ninja`, `clang-cl` | Build the game | configures and compiles the engine, one compiler per CPU core |
| `uv` | Build the game | makes the engine's Python environment and installs its pinned requirements |
| `python` | Decode | runs the zbic bridge compiled into the launcher; it reads your dump and writes the decoded copy |
| `clang` or `zig` | Recompilation | compiles the lifted C, and its verification program |
| `fefw.exe` | Play | the game itself |
| `curl.exe` | update check, mod browsing | see the network table below |
| `tar.exe` | unpacking a mod archive or an update | the archive you chose |
| `winget` | Install the missing build tools | only when you press that button |
| `notepad`, `explorer` | Open log, Open folder | opens the file or folder you asked for |

Every console program is created with `CREATE_NO_WINDOW`, so the launcher never opens a console window, and every
command it runs is written to the task's log in `%APPDATA%\EmblemWeave\logs`. The scripts it runs start their own
tools in turn (Visual Studio's `vcvars64.bat`, `robocopy`, `cmake`, `ninja`, `clang-cl`, `python`); those appear in the
same log, line by line, for every build. If you see terminal windows, shell
scripts or "press any key to continue" prompts while EmblemWeave is open, they are not from EmblemWeave.

Before a build starts, the launcher asks `vswhere` for Visual Studio, reads the MSVC toolset and the Windows SDK out of
the environment `vcvars64.bat` itself sets, checks `clang-cl` and `lld-link` beside each other, and then compiles and
links one small file with exactly that environment. A missing header, an install without the C++ workload or an `INCLUDE`
left by another shell stops the build in a second, with the piece named. `EmblemWeave.exe --check-toolchain` prints the
same report without a window (0 ready, 1 a piece is missing, 2 it cannot compile), and only one build can use the folder
at a time.

Network access, only for the action you asked for. The launcher's own HTTP goes through Windows' `curl.exe`; `git`
and `uv` make their own connections during a build:

| Destination | When | What is sent |
| --- | --- | --- |
| `api.github.com`, `github.com` and GitHub's release CDN | update check and update download | a plain `GET` for the release list or the zip; nothing about you or your game |
| `gamebanana.com` | browsing or installing a mod you chose | the page or file id you asked for |
| `git.ryujinx.app` | Build the game, shader step | a filtered clone of the Ryubing fork, checked out at the revision pinned in `engine.rs`; no credentials |
| `pypi.org` (through `uv`) | Build the game, Python step | installs the packages pinned in `recomp/requirements.txt` |

There is no telemetry, no analytics and no crash upload: Diagnostics exports a text file, and Troubleshooter writes a
local zipped bundle that you review, attach and send yourself.

**Build the game** downloads the engine's pinned third-party sources once, because the engine compiles them from
source: SDL3, Vulkan-Headers, volk, VulkanMemoryAllocator, Dear ImGui and zstd through CMake
(`runtime/cmake/gfx_deps.cmake`), and astc-encoder, bc7enc_rdo and ispc through the texture script
(`tools/textures/scripts/fetch_third_party.ps1`). The shader step additionally clones the pinned Ryubing revision named
above, and stages three of its projects for the translator. Every one of those URLs is written out in those files, and
each CMake download is checked against the `SHA256` pinned beside it.

The release zip is not code-signed; instead its `sha256` is published in every release note. The source is public,
so the strongest check is to build the same binary yourself (`cargo build --release -p emblemweave`) and compare.
Security reports are welcome through the repository's **Security** tab as a private advisory.
