# Troubleshooting

What EmblemWeave needs, and what the launcher means when it stops.

## What it needs

| Need | When | Where it comes from |
| --- | --- | --- |
| Your game files | Game Setup, always | Your own copy of *Fire Emblem: Fortune's Weave*: the base game and its update, or a folder holding the extracted ExeFS and RomFS |
| A decoded `main` module | Recompilation, always | **Game Setup > Decode main module**: the launcher writes `%APPDATA%\EmblemWeave\work\decoded\main` with Python and the `zbic` package (`pip install zbic`) |
| A C compiler | Recompilation only, for the verification stage | `ziglang` (`pip install ziglang`, used as `zig cc`) or LLVM's `clang` on `PATH`. Without one the pipeline still lifts the game; it skips the native check |
| A Vulkan 1.3 driver | Play | Your graphics driver. The engine presents through Vulkan and has no other renderer |
| LLVM 23, CMake, Ninja, .NET 10, uv, Git, PowerShell 7, VS Build Tools with "Desktop development for C++" | Only when building `fefw.exe` from source: **Game Setup > Build the game** | winget (see the README's system requirements). The launcher names whichever are missing |
| Disk space | Everywhere | About 30 GB for the decoded data, plus build output under `%APPDATA%\EmblemWeave\work` |

## Running straight from a `.nup`

**Game Setup > Choose .nup…** takes the package as it is: the ExeFS is copied out once (it is small) and the RomFS is read in place, so nothing has to be unpacked into a folder first. The engine serves reads and module loads straight from the package.

One condition comes with that: the package's files must be **stored**, not zstd-compressed, because the game reads them at offsets it chooses. A compressed package is refused with the message above and breaks at the first frame; a stored one boots like an extracted folder. The launcher now says which case a package is in at detection, before anything is built or run.

## Where the evidence is

Every task writes its own file to `%APPDATA%\EmblemWeave\logs`: the launcher's build stamp, the folders it ran from, each stage, and the whole output of every tool it called. **Diagnostics > Activity log > Log files** names the folder and opens it. The game keeps its own log at `%APPDATA%\EmblemWeave\play\fefw.log`.

A bug report with the failing task's log attached is worth more than any description of the window.

## Messages, and what to do

**Game Setup checks** (the list on the page and in Diagnostics):

| Message | Meaning | Fix |
| --- | --- | --- |
| `Game file: the .nup holds no base game or update` | The file picked is not this game, or only holds DLC | Pick the base game's `.nup` or its `.nsp` |
| `ExeFS: not found; choose the folder that contains your extracted exefs` | The game folder has no `exefs` beside it | Point **Game folder** at the folder that contains `exefs` and `romfs` |
| ``ExeFS: no `main` module in this ExeFS`` | The folder is an ExeFS but not this game's | Pick the update's ExeFS if the base one has no `main` |
| `RomFS: not found; the game cannot load its assets without it` | No `romfs` folder | Extract the RomFS beside the ExeFS |
| `RomFS: Fs.bin.zst (the content index the game loads first) is missing` | A partial extraction | Extract the RomFS again, complete |
| `N of M files in this .nup are zstd-compressed; …` | The package is served in place, so its files must be stored: a zstd frame has no offset to read into, and the game breaks before its first frame | Repack without compression (`nup pack` without the zstandard module writes stored files), or use an extracted `exefs` and `romfs` folder |
| `Decoded module: segment sizes do not match the module header; decode again` | The decoded copy is stale or truncated | **Game Setup > Decode again** |

**Decoding** (Task: `decode`):

| Message | Meaning | Fix |
| --- | --- | --- |
| `tools/zbic_extract.py was not found next to the launcher or in the working folder` | The launcher was moved away from its tools | Run it from the repository, or unpack the release whole |
| ``decoding failed; if the log mentions `zbic`, install it with `pip install zbic``` | The Python helper failed; the log has its output | `python -m pip install zbic`, then decode again |
| `decoded sizes do not match the module header` | The decode wrote less than the module declares | Decode again; if it repeats, send the `decode` log |

**Recompilation** (Task: `pipeline`):

| Message | Meaning | Fix |
| --- | --- | --- |
| `set the game folder on Game Setup first` | No ExeFS with a `main` module was found | Set up Game Setup |
| `the main module is not decoded` | The pipeline needs the decoded copy | Decode on Game Setup |
| `no C compiler found (install zig with pip install ziglang, or clang)` | Stage 6 was skipped, not failed: the lift is fine, the native check did not run | Optional: `pip install ziglang` |
| `` `…` cannot build C: … `` | A compiler was found but cannot compile anything (wrong build, missing SDK) | Install `ziglang`, or a working LLVM `clang`, and run again |
| `N of M shards were rejected by the C compiler: <first error>` | The compiler refused the emitted C. It is not a missing game file: every shard it was given came from the decoded module | The log has the whole listing. Send it: this is a launcher bug unless the first error names a missing toolchain file |
| `needs the native build` | Stage 7 needs stage 6 | Install a C compiler and recompile |
| `the verification program did not compile` | The C compiler built the shards but not the check harness | The `pipeline` log names the compiler and the error |

**Building the game** (Task: `engine`) and **Play**:

| Message | Meaning | Fix |
| --- | --- | --- |
| `build tools missing: LLVM 23 (clang-cl), CMake, …` | The named tools are not on `PATH` | Install them with winget; the list is the README's |
| `the engine (engine/fefw-recomp) was not found; run git submodule update --init` | Building from source without the engine tree | `git submodule update --init --recursive` |
| `the game is not built yet: run Build the game on Game Setup` (the button is on **Overview**) | `fefw.exe` is absent | Build the game |
| `> pwsh … failed (exit code: 1)` | A build step failed; the `engine` log holds its whole output | Send the log |
| The window closes at once, or the picture stays black | Usually the Vulkan driver | Read `%APPDATA%\EmblemWeave\play\fefw.log`; update the graphics driver. A Vulkan 1.3 device is required |

## When none of this matches

Send the newest log from `%APPDATA%\EmblemWeave\logs`, together with the stage the recompilation page showed. **Diagnostics > Export report** writes the page's measurements to a file next to it.
