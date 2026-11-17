# Installation, portable storage and game data

The release zip is self-contained apart from the game data and the Windows build tools. Start with the quick steps on the main README; this page has the full layouts and the decode helper.

## Install the release

Download `EmblemWeave-<version>-windows-x64.zip` from the [releases page](https://github.com/XiONDiGiTaL/EmblemWeave/releases),
unzip it anywhere and run `EmblemWeave.exe`. Choose your game on **Game Setup**, then press **Build the game** on the
Overview page (once; it lists any build tools you still need) and **Play**. To build the launcher yourself, see
[docs/BUILDING.md](BUILDING.md).

A release zip contains `portable.txt`, so its settings, decoded modules, build output and saves start in `data` beside
`EmblemWeave.exe`; your game folder is never written to. A copy made before portable-by-default, or one whose marker was
removed, uses `%APPDATA%\EmblemWeave` (Windows) or `$XDG_CONFIG_HOME/EmblemWeave` (Linux) instead. The launcher's
**About > Storage** page shows which folder is in use.

The zip and what it deliberately leaves out are listed in [PACKAGE.md](PACKAGE.md), and
`tools/package_release.ps1` builds it reproducibly.

## The build tools, and making them the release's job

The one part of the install that is still the player's is the build toolchain. Today the launcher finds, versions and
(where it can) `winget`-installs seven things: Visual Studio's C++ tools, the Windows 10/11 SDK, LLVM's `clang-cl` and
`lld-link`, CMake, Ninja, Python with `zbic`, and optionally NASM, PowerShell 7, `uv` and the .NET SDK. That is more
than the other recompilations ask for, and the difference is not the engine: it is what each project ships.

`FireEmblemEngage-EngagePCRecomp` is the clearest comparison, because it is installed here and can be measured. Its
`START-HERE.txt` lists four requirements — Windows 10/11 x64, an AVX2 CPU, a Vulkan 1.3 driver, 40 GiB free — and no
build tools at all. Its folder carries `llvm/` (316 MB: clang and the MinGW-w64 target), `python/` (101 MB, a relocatable
interpreter with its packages), `bin/ninja.exe` (0.6 MB), its extractor and its translator. The player runs the setup,
points it at their dump, and waits. NX2navi does the same thing in the other direction: its launcher "downloads the build
tools" into its own folder and the setup .exe is the only file the player handles.

So the way to make EmblemWeave's setup one step is not to remove steps from the engine; it is to stop asking the
player to install what the zip can carry:

| Piece | Size | Can the release carry it? | Effect |
| --- | ---: | --- | --- |
| LLVM `clang-cl` + `lld-link` | about 250 MB unpacked | yes, LLVM's own licence allows redistribution | removes the LLVM install |
| CMake | about 60 MB unpacked | yes, BSD-3-Clause | removes the CMake install |
| Ninja | under 1 MB | yes, Apache-2.0 | removes the Ninja install |
| Python + `zbic`, `capstone`, `lz4`, `zstandard` | about 120 MB | yes, PSF licence plus wheel licences | removes the Python install and the pip step |
| NASM, PowerShell 7, `uv`, Git, .NET SDK | 0 to 300 MB | already optional or replaced by the bundled translator | nothing to remove |
| Visual Studio C++ tools and the Windows SDK | 2 to 6 GB | the headers and import libraries are Microsoft's, and redistributing them is not ours to do | stays, but the launcher's one-button `winget` install already runs it unattended |

The launcher is ready for that layout today: `toolchain::tool_path` puts `engine/fefw-recomp/tools\bin` first on the
PATH every probe and every build step uses, so a zip that carries the four portable pieces there is found and used with
no other change. The remaining work is packaging (put the extracted tools in the zip and check their hashes, as
`tools/package_release.ps1` already does for everything else), not code in the build.

Removing Visual Studio as well is the bigger prize and the bigger job: it means building the runtime and every static
dependency with clang against MinGW-w64 or libc++, which is the same ABI change a Linux build needs ([LINUX.md](LINUX.md)
phase 3). It should be done once, for both platforms, rather than twice.

## Portable mode

Every current release is portable by default: `portable.txt` ships beside `EmblemWeave.exe`, so the launcher keeps
`play` (the run folder and its `fefw.ini`), `work` (decoded modules and build output), `save` and `logs` in `data`
beside the executable. Unzip it anywhere, carry it on a stick, and the whole thing moves as one piece with its saves and
build cache. A copy that already has a `%APPDATA%\EmblemWeave` profile and no marker keeps using that profile until you
switch it.

The switch is **About > Storage > Keep everything in this folder**: it writes the marker, creates `data` and copies
your saves across, and the game is built into the new folder on the next build. Delete `portable.txt` to go back to the
profile folder. The game's own dump is read where it is, in either mode.

## Where the folder can live

Spaces are fine, including in a folder named after the game: a build was run from a path with spaces to check it. An
apostrophe and a non-ASCII character each used to be a trap for the shader translator, which MSBuild builds with item
transforms written as `@(Item->'…')`; when the path has either, the launcher builds the translator from a copy under its
own work folder and copies the result back, so the folder can be named anything without being renamed. Long paths, past
about 120 characters, still make CMake warn about object file paths; a shorter folder avoids the question.

## Preparing your game data

The simplest way is a **`.nup` dump** of your own copy of the game. On the
**Game Setup** page, choose **Choose package…** and select the file. EmblemWeave
then does the rest:

- it checks the container's table of contents and every file it reads against
  their SHA-256 hashes, so a damaged dump is reported rather than used;
- it copies the small ExeFS (about 70 MB) once into its work folder;
- it reads the RomFS (26 GB) **in place** from the `.nup`, so nothing has to be
  extracted, and it takes the title and version from the dump's own metadata.

The `.nup` is only ever read.

A **`.nx` archive** (NXFormat) works the same way. Its catalog, manifest and
block index are checked against the SHA-256 values in the archive's own header,
the ExeFS is copied out once, and the engine serves the RomFS straight from the
archive: each block is decompressed and its 16 KiB slices checked with CRC-32C
as the game reads them. Point **Choose package…** at the `.nx`; nothing is
extracted and the file is only ever read.

**Which build your copy is.** Game Setup also hashes your ExeFS modules against
the manifest the engine was tested with, taken from the preserved fefw-recomp
0.2.0 package: update 131072, `main` build `db93b0125f48…`. If all seven hashes
agree the check says so; a different build is reported as such, and still
recompiles, since the pipeline works from your own modules.

An **`.nsp` package** works too, when it is a plaintext repack: NSP Box writes
those without keys, as unsigned program, control and meta NCAs. Pick it with
**Choose package…** as well. EmblemWeave unpacks it itself, once, into
the launcher's `work\nsp` folder, then uses that folder like an extracted dump,
so the game boots from it and the recompiler reads it. The unpack is repeated
only when the package's size or modification time changes, and the folder can be
deleted to reclaim the space.

A plaintext **`.xci` game card** is accepted the same way. EmblemWeave reads
the card's HFS0 root, opens its `secure` partition, and unpacks the program and
control NCAs into `work\xci`; the game and the recompiler then use that folder
like an extracted dump. Retail cartridge images keep their NCAs encrypted, so
they are refused with the name of the first NCA that needs a console key. A
decrypted XCI created by your own dump tool is what this path reads.

You can also use an extracted copy. EmblemWeave then needs:

| Item | What it is | Where it usually is |
| --- | --- | --- |
| ExeFS | The game's executable modules (`main`, `sdk`, `main.npdm`, ...) | `exefs\` |
| RomFS | The game's assets (`Fs.bin.zst` and the asset folders) | `romfs\` |
| `control.nacp` | Title metadata, including the version | `data\control\control.nacp` in an NSP extraction |

Choose the containing folder with **Folder…** on Game Setup. Plain dumps and NSP
extractions are both detected automatically, and every location can be set
individually. A retail NSP stays encrypted with the console's own keys, which
EmblemWeave neither has nor asks for, so only plaintext repacks are unpacked.

The game's `main` module is compressed with ZBIC, a Zstandard variant. Decode it
once with **Decode main module** on Game Setup. This currently uses a small
helper script and the Python `zbic` package:

```powershell
python -m pip install zbic
```
