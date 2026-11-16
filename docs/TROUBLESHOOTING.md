# Troubleshooting

What EmblemWeave needs, and what the launcher means when it stops.

## What it needs

| Need | When | Where it comes from |
| --- | --- | --- |
| Your game files | Game Setup, always | Your own copy of *Fire Emblem: Fortune's Weave*: the base game and its update, a plaintext `.nup`, `.nx`, `.nsp` or `.xci` package, or a folder holding the extracted ExeFS and RomFS |
| A decoded `main` module | Recompilation, always | **Game Setup > Decode main module**: the launcher writes `%APPDATA%\EmblemWeave\work\decoded\main` with Python and the `zbic` package (`pip install zbic`) |
| A C compiler | Recompilation only, for the verification stage | `ziglang` (`pip install ziglang`, used as `zig cc`) or LLVM's `clang`, found on `PATH` or in its usual install folder (`C:\Program Files\LLVM\bin`, or the clang Visual Studio installs). Without one the pipeline still lifts the game; it skips the native check |
| A Vulkan 1.3 driver | Play | Your graphics driver. The engine presents through Vulkan and has no other renderer |
| Disk space | Everywhere | About 30 GB for the decoded data, plus build output under `%APPDATA%\EmblemWeave\work` |

## Tools for building the game

Only **Game Setup > Build the game** needs these. Each row is one entry of the launcher's dependency registry: the
Requirements page, this table and the pre-build check read the same definition, so they cannot disagree. A bundled
translator is not a dependency: the release carries it with its own runtime, and the .NET SDK row applies only to a build
that compiles the translator from source.

<!-- requirements:start -->
| Dependency | Needed for | Version | Missing it |
| --- | --- | --- | --- |
| Visual Studio C++ tools | Build the game | 2022 (17) or newer | `winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended" --accept-package-agreements --accept-source-agreements` |
| Windows 10/11 SDK | Build the game | 10.0 | `Add Windows 11 SDK under Individual components in the Visual Studio Installer, or install the standalone SDK, then press Check again.` |
| LLVM (clang-cl and lld-link) | Build the game | 23 | `winget install --id LLVM.LLVM --exact --accept-package-agreements --accept-source-agreements` |
| CMake | Build the game | 3.25 | `winget install --id Kitware.CMake --exact --accept-package-agreements --accept-source-agreements` |
| Ninja | Build the game | any | `winget install --id Ninja-build.Ninja --exact --accept-package-agreements --accept-source-agreements` |
| NASM | Build the game (optional) | 2.14 | `winget install --id NASM.NASM --exact --accept-package-agreements --accept-source-agreements` |
| Git | Build the game (optional) | any | `winget install --id Git.Git --exact --accept-package-agreements --accept-source-agreements` |
| PowerShell 7 | Build the game (optional) | 7 | `winget install --id Microsoft.PowerShell --exact --accept-package-agreements --accept-source-agreements` |
| uv | Build the game (optional) | any | `winget install --id astral-sh.uv --exact --accept-package-agreements --accept-source-agreements` |
| .NET 10 SDK | Build the game (only without the bundled translator) | 10 | `winget install --id Microsoft.DotNet.SDK.10 --exact --accept-package-agreements --accept-source-agreements` |
| .NET runtime | Shaders (optional) | 10 | Only runs a translator that was built here without being published as one executable. The bundled one carries its own runtime. |
| Shader translator | Shaders | any | Turns the game's shaders into SPIR-V. The release zip carries it, published with its own .NET runtime. |
| Python | Decode | 3.9 | `winget install --id Python.Python.3.12 --exact --accept-package-agreements --accept-source-agreements` |
| zbic (Python package) | Decode | any | `py -m pip install zbic` |
| C compiler | Recompilation | any | `py -m pip install ziglang` |
<!-- requirements:end -->

`EmblemWeave.exe --check-toolchain` runs the same checks in a console (exit 0 ready, 1 missing something, 2 the check
itself could not run), and `EmblemWeave.exe --print-requirements` prints the table above.

## Running straight from a `.nup`

**Game Setup > Choose .nup…** takes the package as it is: the ExeFS is copied out once (it is small) and the RomFS is read in place, so nothing has to be unpacked into a folder first. The engine serves reads and module loads straight from the package.

A package may store its files as they are or pack each one as a zstd frame. Stored files are read at whatever offset the game asks for, straight from the package. A compressed file cannot be, so it is decompressed the first time the game reads it. The package is never rewritten, and there is no folder to delete: the cost is the decompression itself, and the Game Setup check says how much of the package is packed that way so the first start is not mistaken for a hang.

## Running straight from a `.nsp`

**Game Setup > Choose package…** takes a plaintext `.nsp` too: the unsigned program, control and meta NCAs that NSP Box writes for emulators. Nothing in one is encrypted, so the launcher unpacks it itself with no keys and no outside tool - the program NCA's ExeFS and RomFS, and `control.nacp` from the control NCA - into `%APPDATA%\EmblemWeave\work\nsp\<title>-<bytes>`. It records what it wrote in `dump.txt` and then treats that folder exactly like an extracted dump, so the game boots from it and the recompiler reads it. Game Setup shows the unpack while it runs: a bar, the bytes copied of the total, and `Unpacking the package: 8.3 GB of 27.1 GB copied` in the job's line. **Cancel** beside **Detect again** stops it at the next file and deletes the half-written copy, so a partial unpack can never be mistaken for a complete one; nothing else is touched. The unpack is repeated only when the package's size or modification time changes, and the folder can be deleted to reclaim the space.

A retail NSP stays encrypted: its NCAs are sealed with the console's own keys, which this project neither ships nor asks for. The Game Setup check names the first NCA it cannot read. Convert the package to a plaintext one (NSP Box's NSP → NUP and NUP → NSP do that without keys) or point EmblemWeave at an extracted folder instead.

## Running straight from a `.xci`

**Game Setup > Choose package…** also accepts a plaintext `.xci` game card. EmblemWeave reads the HFS0 root, opens its `secure` partition and unpacks the program and control NCAs into `work\xci\<title>-<bytes>`, then uses that folder like an extracted dump. The unpack, cancellation and `dump.txt` marker work exactly as they do for an `.nsp`.

A retail cartridge stays encrypted: its NCAs are sealed with console keys, which this project neither ships nor asks for. The check names the first NCA it cannot read. Use a decrypted XCI made from your own dump, or point EmblemWeave at an extracted folder.

## Where the evidence is

Every task writes its own file to `%APPDATA%\EmblemWeave\logs`: the launcher's build stamp, the folders it ran from, each stage, and the whole output of every tool it called. **Diagnostics > Activity log > Log files** names the folder and opens it. The game keeps its own log at `%APPDATA%\EmblemWeave\play\fefw.log`.

A bug report with the failing task's log attached is worth more than any description of the window.

## Messages, and what to do

**Game Setup checks** (the list on the page and in Diagnostics):

| Message | Meaning | Fix |
| --- | --- | --- |
| `Game file: the .nup holds no base game or update` | The file picked is not this game, or only holds DLC | Pick the base game's `.nup` or its `.nsp` |
| `Game file: … .nca is not a plaintext NCA: an encrypted retail package cannot be unpacked` | The `.nsp` or `.xci` still holds encrypted retail NCAs | Use a plaintext `.nup`/`.nsp`/`.xci`, or an extracted dump |
| `Game file: … is encrypted (NCA crypto type …)` | A retail cartridge NCA needs console keys | Use a decrypted `.xci`, a plaintext `.nsp`/`.nup`, or an extracted dump |
| `Game file: unpacked below …: N ExeFS files and M RomFS files (X GB)` | Expected: the package was unpacked once into the work folder | Nothing to fix. Delete that folder to reclaim the space; it is rebuilt on the next Detect |
| `ExeFS: not found; choose the folder that contains your extracted exefs` | The game folder has no `exefs` beside it | Point **Game folder** at the folder that contains `exefs` and `romfs` |
| ``ExeFS: no `main` module in this ExeFS`` | The folder is an ExeFS but not this game's | Pick the update's ExeFS if the base one has no `main` |
| `Game build: matches the dump the engine was tested with (update 131072 …)` | Your ExeFS is the build the engine was tested against | Nothing to fix |
| `Game build: a different build of the game (… differs)` | Another revision of the game | It still recompiles from your modules, but is not the tested build; the tested one is update 131072 |
| `RomFS: not found; the game cannot load its assets without it` | No `romfs` folder | Extract the RomFS beside the ExeFS |
| `RomFS: Fs.bin.zst (the content index the game loads first) is missing` | A partial extraction | Extract the RomFS again, complete |
| `N of M files are zstd-compressed: each is decompressed the first time the game reads it …` | The package keeps some files compressed; the engine decompresses each as it is read | Nothing to fix: the first start is slower, the package is never rewritten, and there is no cache folder to clear |
| `Decoded module: segment sizes do not match the module header; decode again` | The decoded copy is stale or truncated | **Game Setup > Decode again** |

**Decoding** (Task: `decode`):

| Message | Meaning | Fix |
| --- | --- | --- |
| ``the decoder needs Python's `zbic` package: run `pip install zbic` …`` | The bridge is built into the launcher and ran, but the Python that ran it has no `zbic` package | `pip install zbic` (or `py -m pip install zbic`), then decode again. The bridge needs no `tools` folder beside the launcher |
| `decoding failed; the log has the bridge's own message` | The bridge failed for another reason (a read error, a missing module) | The `decode` log has Python's traceback |
| `decoded sizes do not match the module header` | The decode wrote less than the module declares | Decode again; if it repeats, send the `decode` log |

**Recompilation** (Task: `pipeline`):

| Message | Meaning | Fix |
| --- | --- | --- |
| `set the game folder on Game Setup first` | No ExeFS with a `main` module was found | Set up Game Setup |
| `the main module is not decoded` | The pipeline needs the decoded copy | Decode on Game Setup |
| `no C compiler found: install Visual Studio Build Tools …` | Stage 6 was skipped, not failed: the lift is fine, the native check did not run. The launcher looks in LLVM's and Visual Studio's own folders as well as `PATH`, so this means there really is no compiler | Install Visual Studio Build Tools with the C++ workload (the same one the game build needs), `winget install LLVM.LLVM`, or `uv pip install --python tools\.venv ziglang`, then run the pipeline again |
| `` `…` cannot compile the lifted code (app_lift_000.c): <error> `` | The compiler ran and built the generated headers, but refused the first real shard, so the rest were never started. One message with the compiler's own first line replaces the old 117 rejections | The same error line is in the `pipeline` log with everything under it. A missing header means the C++ workload or Windows SDK is absent; anything else is ours: send the log |
| `cxx.h(7,10): fatal error: 'bit' file not found` | The C++ library of the Visual Studio in use predates C++20 and has no `<bit>` header. The build now checks the Visual Studio version before it starts and says this instead of failing on the header | Install 2022 or newer: `winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"` |
| `dotnet build failed … sdk-not-found` / exit code `0x8000809b` | The .NET *runtime* is installed, so `dotnet.exe` answers, but there is no 10.x SDK to build the shader translator with. From 0.1.11 the release zip carries the translator published with its own runtime, so this step needs no .NET at all; the message only appears on a copy that has to build it, or for an older zip | Update to the current release, or `winget install Microsoft.DotNet.SDK.10` |
| `` `…` cannot build C: … `` | A compiler was found but cannot compile anything (wrong build, missing SDK) | Install `ziglang`, or a working LLVM `clang`, and run again |
| `N of M shards were rejected by the C compiler: <first error>` | Later shards failed after the first one compiled, so the cause is in the code or the machine, not in the compiler as a whole. It is not a missing game file: every shard came from the decoded module | The log has the whole listing. Send it: this is a launcher bug unless the first error names a missing toolchain file |
| `the compiler rejected N of M shards before finishing, so the cause is systemic (…)` | More than half the shards failed, so it is one cause (a broken compiler, a full disk, a bad include path), not N problems. Compiling stopped instead of repeating it 118 times | The parenthesised text is the compiler's first error. Free some disk, install `ziglang`, and run the pipeline again |
| `needs the native build` | Stage 7 needs stage 6 | Install a C compiler and recompile |
| `the verification program did not compile` | The C compiler built the shards but not the check harness | The `pipeline` log names the compiler and the error |

**Building the game** (Task: `engine`) and **Play**:

| Message | Meaning | Fix |
| --- | --- | --- |
| `the game build needs tools this PC does not have: …` | One or more build tools are absent. The list now includes Visual Studio's C++ tools, which the engine's scripts call through `vcvars64.bat` | Each tool is followed by its install command; install them and press Build the game again. All of them are in the README's prerequisites. **Overview** has an **Install the missing build tools** button that runs those winget lines for you, Visual Studio included; restart the launcher when it finishes |
| `the engine folder engine/fefw-recomp is missing beside the launcher` | The launcher was moved out of the repository, or the folder was deleted | Unpack the repository whole, or set `EW_ENGINE` to a folder that holds `game-build/` |
| `the game is not built yet: run Build the game on Game Setup` (the button is on **Overview**) | `fefw.exe` is absent | Build the game |
| `pwsh … failed (exit code: 1): <error line>` | A build step failed. The message carries the line that explains it, picked out of the run rather than the summary at the end, so `CMake Generate step failed. Build files cannot be regenerated correctly.` arrives together with the `CMake Error` that caused it | The `engine` log holds the whole output. Send it together with the exported report, which now ends with the tail of the newest log |
| `CMake 3.x is too old: the engine's projects need 3.25 or newer` | CMake is installed but older than the engine's projects ask for. CMake reports that as a generate failure, not as a version problem | `winget upgrade Kitware.CMake` (or `winget install Kitware.CMake`), restart the launcher, press Build the game again |
| `Python environment: … failed the probe: …` / `tools\.venv was created but does not pass the Python probe` | A `tools\.venv` left by an older release or another Python install still has a `python.exe`, but that interpreter cannot start (for example `Module use of python311.dll conflicts with this version of Python`), is not the interpreter the launcher started, or its packages are incomplete. The log names the interpreter path, version, the probe result and the recovery | Nothing: the launcher removes **only** the generated engine `tools\.venv` and creates it again before the next step. Do not uninstall other Python versions; they are not used by that environment. If recreation itself fails, install Python 3.12+ and press Build the game again |
| `Files found` shows `missing` for DLSS, FSR 3 or XeSS (the game's F1 menu, Graphics tab) | The vendor DLLs AMD, NVIDIA and Intel do not let us ship are not in a folder the engine looks in, so it falls back to its own temporal upscaler (#26) | **Graphics > Upscaler files > Look for the files** copies what it finds in Downloads, the Desktop, the launcher folder and the engine folder into `%APPDATA%\EmblemWeave\play\upscalers`; or put them there by hand in `fsr`, `streamline` and `xess`. The runtime also searches `upscalers` or `upscaler` folders beside `fefw.exe` and in every folder above it, up through the engine checkout, so a copy left in `engine/fefw-recomp/upscaler/fsr` is found too. Restart the game after adding them |
| `This game folder was built by an earlier release; press Rebuild to update its engine to …` | The engine is compiled from the sources in your copy, so a new launcher next to an old build keeps the old engine | Press **Rebuild** once. The line disappears when the build records the version it was made with |
| `clang++: error: unknown argument: '-clang:-fmacro-prefix-map=…'` or `clang++: error: no such file or directory: '/W3'` | The build folder was configured by an older release for the clang++ driver, and the engine handed it the flags it writes for clang-cl. Its own check for which driver is running was true for both on Windows, which is why this reached clang++ (#25) | Update to 0.1.23 and press Build the game again: the script starts such a folder over, and the flag now goes by the frontend variant. If it repeats, delete `engine\fefw-recomp\game-build\fefw\build` |
| `the build folder holds a cache CMake cannot reuse` (log line) | An earlier attempt left a CMake cache from another compiler or an older CMake | Nothing to do: the launcher clears that cache and configures once more by itself |
| `no complete Windows SDK was found. Visual Studio's own environment names none, and …\Windows Kits\10 has no version with both …` | Visual Studio's `vcvars64.bat` selected no SDK, and the usual folder has no version with both `um\windows.h` and `um\x64\kernel32.lib`. An SDK on another drive is found through Visual Studio, so this means the workload really is missing | Add **Windows 11 SDK** in Visual Studio Installer (Individual components), or install one from Microsoft's download page |
| `Visual Studio 16.x … is too old` | The newest C++ tools are 2019 (16). Its C++ library has no `<bit>`, which is the `'bit' file not found` failure reports used to show | Install Visual Studio 2022 Build Tools with the C++ workload; the message carries the `winget` line |
| `Visual Studio at … has no MSVC toolset` | The install is there but `VC\Tools\MSVC` is empty: the C++ workload's headers and libraries were never installed | Modify that install in Visual Studio Installer and add "Desktop development with C++" |
| `clang-cl 22.x is older than 23` | LLVM is installed but too old for the code the recompiler generates | `winget upgrade LLVM.LLVM` |
| `lld-link.exe is missing beside …` | LLVM's `bin` folder is incomplete, so every link step would fail | Reinstall LLVM |
| `the toolchain cannot build the smallest test file: compile with … failed` | The pieces were all found but disagree: a header from one install with libraries from another, or an SDK that is only half installed. It is reported before the recompile, not twenty minutes into it | The message carries the compiler's own error line and the `INCLUDE` list it used; adding "Desktop development with C++" or updating LLVM fixes the usual cases |
| `another EmblemWeave build is already running (process N)` | Two builds would compile into the same folders and unpack the same sources | Wait for the other one. If no build is open, end that process, or delete `engine\fefw-recomp\work\build\.build.lock` |
| `could not download … after 4 attempts` or `… is incomplete (no CMakeLists.txt); discarding it and fetching again` | A dependency fetch failed or was interrupted; the fetcher never keeps anything it could not verify | Nothing: press Build the game again. The message names the URL and the last error, and the half-written folder was already removed |
| The window closes at once | Usually the Vulkan driver | The launcher now watches the first minute of every launch: a run that exits early, or that has played sound for 90 s without presenting one frame, is reported in the window with the GPU it picked. Read `%APPDATA%\EmblemWeave\play\fefw.log`; update the graphics driver, try Windowed or Borderless, and a Vulkan 1.3 device is required |
| The picture stays black while the sound plays | The shader cache is incomplete, so no draw is recorded and the present pass copies an untouched (black) texture. It ran at 60 fps and the audio worked, which is why this read as a driver problem (issues #12 and #15). Since 0.1.16 the log says so instead of leaving it to be guessed: `nvnProgramSetShaders: no translated shader for block …` for the first few blocks, then one `more programs than this log lists have no translated shader` naming the shader folder | Press **Build the game**: it reports the blocks the corpus expects and the cache holds (`shader_report`), translates only the missing ones (`--only-missing`), and stops with the counts and the first hashes if it still cannot. A cache holding a few files and no `results.tsv` is a translation that was interrupted, not one that finished |
| `N of M shaders still have no SPIR-V (first: …)` | The build checked the corpus (`corpus/unique.tsv`) against the translation report (`spirv/results.tsv`), translated what was missing, and N blocks still have none: every program that uses one fails to link and its draws are skipped | The line names the first five hashes and `spirv/results.tsv`; the `engine` log has the translator's own output. Send both with a diagnostics report |
| ``extract_corpus: no shader archive under …`` | The shader corpus was about to be built from a path that holds no `*.bfsha`/`*.ptcl`/`*.arc`/`*.sarc`/`*.bnsh` archive, which is what a `.nup`/`.nsp` file looks like to a folder walk | Nothing: **Build the game** copies the archives out of the package and builds the corpus from that folder. The message only appears if that copy found none |
| `…fetch_third_party.ps1 cannot be loaded because running scripts is disabled on this system` | PowerShell's execution policy refused the build's own script. Launcher 0.1.9 and older did not pass `-ExecutionPolicy Bypass` | Update the launcher: current builds run their scripts under a bypassed policy, and hand the same preference to any `pwsh` those scripts start |

**Building the game from a folder the tools dislike** (issues #17, #18 and #20):

| Message | Meaning | Fix |
| --- | --- | --- |
| The build fails on a path containing `&`, or on spaces, parentheses or a long folder name | Ninja put an object path on the command line, and `cmd.exe` split an unescaped `&` (issue #38, for example `G:\\Games\\PC Ports & Recomps\\emblemweave`) | Update the engine to a release whose `build_game.cmd` and `build_fefw.ps1` configure with `CMAKE_NINJA_FORCE_RESPONSE_FILE=ON`. You do not need to move the installation; if you build the engine yourself, add that flag rather than renaming the folder |
| `this engine folder has no translator at …\\prebuilt\\win-x64\\spirv_harness.exe; building one from source` | Every release zip carries that file. An engine folder without it is not from this release, usually because a new `EmblemWeave.exe` was dropped into an older unzipped folder | Unzip the release into a folder of its own and point the launcher at that. Mixing a launcher from one release with another release's engine folder is what produced the `dav1d` and `control.nacp` messages in #17 |
| `this folder's path cannot be handed to MSBuild (an apostrophe or non-ASCII character in it), so the shader translator is built from … instead` | The path has a `'` (as in "Fortune's Weave") or a non-ASCII character, and MSBuild ends its `@(Item->'…')` quoting there, printing the transform as a file name (`MSB3191`, `MSB3021`) | Nothing to fix: the translator is built from a copy under the launcher's work folder and copied back. The message is there so a slow first build is not mistaken for a hang |
| `apply_patch: source file not found` / the patch step fails | The translator's patch step could not find its input, or a patch did not apply | The input is under `tools\shaders\third_party\ryubing`; the build stages it with `setup_ryujinx.ps1`. Delete `tools\shaders\build` and build again |
| `add_subdirectory given source "…/third_party/dav1d" which is not an existing directory` | The engine folder is from 0.1.15 or older, which did not ship that tree (#17) | Use 0.1.16 or newer. Every release since carries `runtime/third_party/dav1d` |
| `You have changed variables that require your cache to be deleted`, then `clang++: error: unknown argument: '-clang:-fmacro-prefix-map=…'` | 0.1.17 set the resource compiler after CMake had configured, so CMake wiped the cache and configured again with its defaults: Debug, and the `clang++` driver, which does not take a `-clang:` argument | Fixed in the next release. Until then, delete `engine\fefw-recomp\game-build\fefw\build` and build again |
| `Title metadata: control.nacp not found` | The dump has no `control.nacp`, so the title name and display version are unknown. The build and the game do not need it | Nothing to fix unless you want the version line: extracted `.nsp` dumps keep it at `data\control\control.nacp` |

| `main.npdm is compressed in the .nup`, `... is compressed in the .nup; the game reads RomFS files in place`, or the game exits two seconds after Play | 0.1.19 and older refused any `.nup` that packs some of its files with zstd (#24) | 0.1.20 and newer write those files into `engine\fefw-recomp\work\romfs-overlay` and the engine reads them from there: press Play, which writes them first and then starts the game, or **Check my game folder** on Game Setup if you would rather not start it yet. Neither compiles anything. A compressed ExeFS no longer matters at all |

## Console windows during the recompilation pipeline

The pipeline compiles the C it emits, one shard per process, using zig's clang (`python -m ziglang cc`), `zig`, or LLVM's
`clang`. The launcher is a GUI program, so a compiler started without `CREATE_NO_WINDOW` gets a console window of its own,
and a large module is hundreds of shards: that is the wall of blank windows in issue #23, and it was a bug in how the
stages started their children, not anything else. Every spawn in `jobs.rs` now sets the flag, so the next release opens
none. They are compiler processes: the launcher's log names the compiler, its version and one line per shard, and the
`Recompilation` page shows the same count.

There is no PHP in this project (no `.php` file, no PHP runtime, no PHP install step), no shell is started to run
anything, nothing in the pipeline touches the network, and the launcher uploads nothing: the only connections it makes are
the ones listed in the README's network table, for an update check, mods you ask for, a `git` clone of the shader
sources and `uv` installing the pinned packages. `%APPDATA%\EmblemWeave\logs` holds every command each stage ran, with
its whole output, which is what to read instead of the process list.

## Can this PC build the game at all

One command answers that without opening the window and without starting a build:

```powershell
.\EmblemWeave.exe --check-toolchain
```

It prints the Visual Studio install, the MSVC toolset and Windows SDK that `vcvars64.bat` selects, `clang-cl` and
`lld-link`, CMake and Ninja, every `CC`, `CXX`, `INCLUDE`, `LIB` or `VCINSTALLDIR` it is ignoring, and then compiles and
links one small file with that exact environment. Exit code 0 means the build can start, 1 means a piece is missing or
too old (the message says which and what to install), 2 means the pieces are there but cannot build a file, which is
usually one install's headers with another's libraries.

## When none of this matches

Before sending it, ask the launcher what the log says. This needs no window, no build and no game files:

```powershell
.\EmblemWeave.exe --verify-log                       # the newest log
.\EmblemWeave.exe --verify-log "C:\path\to\some.log"  # a log from another machine
```

It prints the first error in the file with the lines that follow it, the last line, and which automatic repair the
launcher would try - `_deps` fetched half written, or a build folder whose CMake cache belongs to another toolchain. Exit
code 0 means an error was found, 3 means the log has no error line (the step was stopped before it printed one), 2 means
the file could not be read.

Send the newest log from `%APPDATA%\EmblemWeave\logs`, together with the stage the recompilation page showed. **Diagnostics > Export report** writes the page's measurements to a file next to it.
