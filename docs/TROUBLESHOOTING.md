# Troubleshooting

What EmblemWeave needs, and what the launcher means when it stops.

## What it needs

| Need | When | Where it comes from |
| --- | --- | --- |
| Your game files | Game Setup, always | Your own copy of *Fire Emblem: Fortune's Weave*: the base game and its update, a plaintext `.nup`, `.nx` or `.nsp` package, or a folder holding the extracted ExeFS and RomFS |
| A decoded `main` module | Recompilation, always | **Game Setup > Decode main module**: the launcher writes `%APPDATA%\EmblemWeave\work\decoded\main` with Python and the `zbic` package (`pip install zbic`) |
| A C compiler | Recompilation only, for the verification stage | `ziglang` (`pip install ziglang`, used as `zig cc`) or LLVM's `clang`, found on `PATH` or in its usual install folder (`C:\Program Files\LLVM\bin`, or the clang Visual Studio installs). Without one the pipeline still lifts the game; it skips the native check |
| A Vulkan 1.3 driver | Play | Your graphics driver. The engine presents through Vulkan and has no other renderer |
| LLVM 23, CMake, Ninja, .NET 10, uv, Git, PowerShell 7, VS 2022 Build Tools or newer with "Desktop development for C++" | Only when building `fefw.exe` from source: **Game Setup > Build the game** | winget (see the README's system requirements). The launcher names whichever are missing |
| Disk space | Everywhere | About 30 GB for the decoded data, plus build output under `%APPDATA%\EmblemWeave\work` |

## Running straight from a `.nup`

**Game Setup > Choose .nup…** takes the package as it is: the ExeFS is copied out once (it is small) and the RomFS is read in place, so nothing has to be unpacked into a folder first. The engine serves reads and module loads straight from the package.

A package may store its files as they are or pack each one as a zstd frame. Stored files are read at whatever offset the game asks for, straight from the package. A compressed file cannot be, so it is decompressed once, the first time the game reads it, and kept in the cache at `%LOCALAPPDATA%\EmblemWeave\\nup-cache`; later reads come from there. The cache holds as much as the compressed files add up to (the Game Setup check says how much), the first start is slower by the decompression, and the folder can be deleted when the game is not running.

## Running straight from a `.nsp`

**Game Setup > Choose package…** takes a plaintext `.nsp` too: the unsigned program, control and meta NCAs that NSP Box writes for emulators. Nothing in one is encrypted, so the launcher unpacks it itself with no keys and no outside tool — the program NCA's ExeFS and RomFS, and `control.nacp` from the control NCA — into `%APPDATA%\EmblemWeave\work\nsp\<title>-<bytes>`. It records what it wrote in `dump.txt` and then treats that folder exactly like an extracted dump, so the game boots from it and the recompiler reads it. Game Setup shows the unpack while it runs: a bar, the bytes copied of the total, and `Unpacking the package: 8.3 GB of 27.1 GB copied` in the job's line. **Cancel** beside **Detect again** stops it at the next file and deletes the half-written copy, so a partial unpack can never be mistaken for a complete one; nothing else is touched. The unpack is repeated only when the package's size or modification time changes, and the folder can be deleted to reclaim the space.

A retail NSP stays encrypted: its NCAs are sealed with the console's own keys, which this project neither ships nor asks for. The Game Setup check names the first NCA it cannot read. Convert the package to a plaintext one (NSP Box's NSP → NUP and NUP → NSP do that without keys) or point EmblemWeave at an extracted folder instead.

## Where the evidence is

Every task writes its own file to `%APPDATA%\EmblemWeave\logs`: the launcher's build stamp, the folders it ran from, each stage, and the whole output of every tool it called. **Diagnostics > Activity log > Log files** names the folder and opens it. The game keeps its own log at `%APPDATA%\EmblemWeave\play\fefw.log`.

A bug report with the failing task's log attached is worth more than any description of the window.

## Messages, and what to do

**Game Setup checks** (the list on the page and in Diagnostics):

| Message | Meaning | Fix |
| --- | --- | --- |
| `Game file: the .nup holds no base game or update` | The file picked is not this game, or only holds DLC | Pick the base game's `.nup` or its `.nsp` |
| `Game file: … .nca is not a plaintext NCA: an encrypted retail package cannot be unpacked` | The `.nsp` still holds encrypted retail NCAs | Use a plaintext `.nup`/`.nsp`, or an extracted dump |
| `Game file: unpacked below …: N ExeFS files and M RomFS files (X GB)` | Expected: the package was unpacked once into the work folder | Nothing to fix. Delete that folder to reclaim the space; it is rebuilt on the next Detect |
| `ExeFS: not found; choose the folder that contains your extracted exefs` | The game folder has no `exefs` beside it | Point **Game folder** at the folder that contains `exefs` and `romfs` |
| ``ExeFS: no `main` module in this ExeFS`` | The folder is an ExeFS but not this game's | Pick the update's ExeFS if the base one has no `main` |
| `Game build: matches the dump the engine was tested with (update 131072 …)` | Your ExeFS is the build the engine was tested against | Nothing to fix |
| `Game build: a different build of the game (… differs)` | Another revision of the game | It still recompiles from your modules, but is not the tested build; the tested one is update 131072 |
| `RomFS: not found; the game cannot load its assets without it` | No `romfs` folder | Extract the RomFS beside the ExeFS |
| `RomFS: Fs.bin.zst (the content index the game loads first) is missing` | A partial extraction | Extract the RomFS again, complete |
| `N of M files are zstd-compressed: each is decompressed once into …` | The package keeps some files compressed; the engine decompresses each into its cache the first time the game reads it | Nothing to fix: the first start is slower and the cache takes the space named. Delete `%LOCALAPPDATA%\EmblemWeave\\nup-cache` to reclaim it |
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
| `the build folder holds a cache CMake cannot reuse` (log line) | An earlier attempt left a CMake cache from another compiler or an older CMake | Nothing to do: the launcher clears that cache and configures once more by itself |
| The window closes at once, or the picture stays black | Usually the Vulkan driver | The launcher now watches the first minute of every launch: a run that exits early, or that has played sound for 90 s without presenting one frame, is reported in the window with the GPU it picked. Read `%APPDATA%\EmblemWeave\play\fefw.log`; update the graphics driver, try Windowed or Borderless, and a Vulkan 1.3 device is required |
| `…fetch_third_party.ps1 cannot be loaded because running scripts is disabled on this system` | PowerShell's execution policy refused the build's own script. Launcher 0.1.9 and older did not pass `-ExecutionPolicy Bypass` | Update the launcher: current builds run their scripts under a bypassed policy, and hand the same preference to any `pwsh` those scripts start |

## When none of this matches

Before sending it, ask the launcher what the log says. This needs no window, no build and no game files:

```powershell
.\EmblemWeave.exe --verify-log                       # the newest log
.\EmblemWeave.exe --verify-log "C:\path\to\some.log"  # a log from another machine
```

It prints the first error in the file with the lines that follow it, the last line, and which automatic repair the
launcher would try — `_deps` fetched half written, or a build folder whose CMake cache belongs to another toolchain. Exit
code 0 means an error was found, 3 means the log has no error line (the step was stopped before it printed one), 2 means
the file could not be read.

Send the newest log from `%APPDATA%\EmblemWeave\logs`, together with the stage the recompilation page showed. **Diagnostics > Export report** writes the page's measurements to a file next to it.
