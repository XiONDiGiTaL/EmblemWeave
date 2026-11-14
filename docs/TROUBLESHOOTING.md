# Troubleshooting

What EmblemWeave needs, and what the launcher means when it stops.

## What it needs

| Need | When | Where it comes from |
| --- | --- | --- |
| Your game files | Game Setup, always | Your own copy of *Fire Emblem: Fortune's Weave*: the base game and its update, a plaintext `.nup` or `.nsp` package, or a folder holding the extracted ExeFS and RomFS |
| A decoded `main` module | Recompilation, always | **Game Setup > Decode main module**: the launcher writes `%APPDATA%\EmblemWeave\work\decoded\main` with Python and the `zbic` package (`pip install zbic`) |
| A C compiler | Recompilation only, for the verification stage | `ziglang` (`pip install ziglang`, used as `zig cc`) or LLVM's `clang` on `PATH`. Without one the pipeline still lifts the game; it skips the native check |
| A Vulkan 1.3 driver | Play | Your graphics driver. The engine presents through Vulkan and has no other renderer |
| LLVM 23, CMake, Ninja, .NET 10, uv, Git, PowerShell 7, VS Build Tools with "Desktop development for C++" | Only when building `fefw.exe` from source: **Game Setup > Build the game** | winget (see the README's system requirements). The launcher names whichever are missing |
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
| `no C compiler found (install zig with pip install ziglang, or clang)` | Stage 6 was skipped, not failed: the lift is fine, the native check did not run | Optional: `pip install ziglang` |
| `` `…` cannot build C: … `` | A compiler was found but cannot compile anything (wrong build, missing SDK) | Install `ziglang`, or a working LLVM `clang`, and run again |
| `N of M shards were rejected by the C compiler: <first error>` | The compiler refused the emitted C. It is not a missing game file: every shard it was given came from the decoded module | The log has the whole listing. Send it: this is a launcher bug unless the first error names a missing toolchain file |
| `the compiler rejected N of M shards before finishing, so the cause is systemic (…)` | More than half the shards failed, so it is one cause (a broken compiler, a full disk, a bad include path), not N problems. Compiling stopped instead of repeating it 118 times | The parenthesised text is the compiler's first error. Free some disk, install `ziglang`, and run the pipeline again |
| `needs the native build` | Stage 7 needs stage 6 | Install a C compiler and recompile |
| `the verification program did not compile` | The C compiler built the shards but not the check harness | The `pipeline` log names the compiler and the error |

**Building the game** (Task: `engine`) and **Play**:

| Message | Meaning | Fix |
| --- | --- | --- |
| `the game build needs tools this PC does not have: …` | One or more build tools are absent. The list now includes Visual Studio's C++ tools, which the engine's scripts call through `vcvars64.bat` | Each tool is followed by its install command; install them and press Build the game again. All of them are in the README's prerequisites. If Visual Studio is the missing one, **Overview** has an **Install Visual Studio Build Tools** button that runs the winget line for you; restart the launcher when it finishes |
| `the engine folder engine/fefw-recomp is missing beside the launcher` | The launcher was moved out of the repository, or the folder was deleted | Unpack the repository whole, or set `EW_ENGINE` to a folder that holds `game-build/` |
| `the game is not built yet: run Build the game on Game Setup` (the button is on **Overview**) | `fefw.exe` is absent | Build the game |
| `pwsh … failed (exit code: 1): <last lines>` | A build step failed. The message now carries the last lines that step printed, so the reason is usually right there | The `engine` log holds the whole output. Send it together with the exported report, which now ends with the tail of the newest log |
| The window closes at once, or the picture stays black | Usually the Vulkan driver | Read `%APPDATA%\EmblemWeave\play\fefw.log`; update the graphics driver. A Vulkan 1.3 device is required |

## When none of this matches

Send the newest log from `%APPDATA%\EmblemWeave\logs`, together with the stage the recompilation page showed. **Diagnostics > Export report** writes the page's measurements to a file next to it.
