# Installation, portable storage and game data

The release zip is self-contained apart from the game data and the Windows build tools. Start with the quick steps below; this page has the full layouts and the decode helper.

## Quick start

1. Download `EmblemWeave-<version>-windows-x64.zip` from the [releases page](https://github.com/XiONDiGiTaL/EmblemWeave/releases) and unzip it anywhere.
2. Run `EmblemWeave.exe`. A fresh release is **portable by default**: settings, saves, builds, mods and logs stay in `data` beside the executable, so the whole folder can be moved as one piece.
3. On **Game Setup**, choose your own dump: a `.nup`, `.nx`, plaintext `.nsp`, plaintext `.xci`, or an extracted folder.
4. Press **Build the game** once on **Overview**, then **Play**.

The build needs Visual Studio 2022 Build Tools with the C++ workload, LLVM, CMake and Ninja. The launcher checks each one and offers the install command. The shader translator ships in the zip with its own runtime, so the .NET SDK is not needed.

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

## Updating

**Use the launcher's updater.** When a newer release is published, Overview shows **Update available** with an **Update now**
button; **About > Updates** has the same action. The launcher downloads the release, writes the new launcher, docs and engine
sources over this install, and restarts once. `data\` (saves, settings, decoded modules and logs), the built game in
`engine\fefw-recomp\game-build`, the fetched build tools and the third-party sources are kept. It never adds or removes
`portable.txt`, so a portable install stays portable and a profile install stays a profile install. An update started by a
launcher older than this one leaves the rest of the release in the download cache; the new launcher writes it on first
start, so those engine sources catch up without another download.

The launcher checks at startup; **About > Updates** can turn that off or check by hand. If a release has no Windows download,
the update card links to its release page instead of trying to install it. A failed executable swap starts the old launcher
and writes the reason to `logs\update.log`.

The launcher is the small part; the engine build and the fetched build tools are the expensive part, and a new release zip is
mostly a new launcher and engine sources. There is no need to delete the old folder and start again.

**Updating by hand.** This is the fallback when the launcher cannot update itself, for example on a read-only folder:

- **Unzip the new release into the same folder.** It replaces the launcher, docs and engine sources, and leaves `data\` and
  the built game alone — your saves, settings, decoded modules, build output and the game itself keep working.
- **If you would rather unpack into a fresh folder**, keep these two from the old one and move them across:
  - `data\` — in a portable install this is where your saves (`data\save`), settings and build output live;
  - `engine\` — the engine checkout and the built `fefw.exe`, plus the tools fetched into `engine\fefw-recomp\tools`.

  Copy them in before the first launch and the new copy is the old one with a newer launcher.
- **Or keep the engine anywhere at all.** A fresh copy that cannot find an engine asks for one on **Game Setup > Engine
  folder**: press **Use another engine folder…** and choose the old copy's `engine\fefw-recomp` (or the old copy itself). The
  launcher remembers it and plays the existing build. This is the one to use when the old build lives on another drive, or when
  the old folder is kept as a backup.
- **On first run it looks for you.** A copy with no build of its own checks `engine.txt` in its settings, in the profile
  settings folder and in the `data` folder of any EmblemWeave copy beside it, and looks for a built engine in those siblings.
  When it finds one, the Overview page opens with **A build is already on this PC** and a **Use this build** button; **Not now**
  declines that one until you say otherwise, and nothing is compiled before you answer.

Saves are not touched by any of this: in a portable install they are in `data\save`, and in a profile install in
`%APPDATA%\EmblemWeave\save` (Windows) or `$XDG_CONFIG_HOME/EmblemWeave/save` (Linux). **About > Storage** says which one
this copy uses.

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

The launcher now does that itself (`apps/emblemweave/src/portable.rs`). `toolchain::tool_path` puts
`engine/fefw-recomp/tools\bin` first on the PATH every probe and every build step uses, so a piece that lands there is
found before anything installed on the PC, and the Requirements page reports it as **Carried** rather than installed.
On first run a copy without that folder fetches the four pieces in the background (`tools\fetch.log` records the
progress); `EmblemWeave.exe --fetch-tools` does the same in the foreground, which is what a build machine should run.

| Piece | Archive | Pinned SHA-256 |
| --- | --- | --- |
| LLVM `clang-cl` + `lld-link` 23.1.5 | `clang+llvm-23.1.5-x86_64-pc-windows-msvc.tar.xz` (901 MB) | `62fdada7…11c6d4` |
| CMake 3.31.6 | `cmake-3.31.6-windows-x86_64.zip` (46 MB) | `d163cd3a…e4f836` |
| Ninja 1.13.1 | `ninja-win.zip` (0.3 MB) | `26a40fa8…74ae25bf` |
| Python 3.12.8 (embeddable) | `python-3.12.8-embed-amd64.zip` (11 MB) | `8d3f33be…e3ced2fb` |

Each archive is checked against the hash pinned in the table before it is unpacked, so a truncated or substituted
download stops with a message instead of filling `tools\bin` with something that half works. Anything that fails the
check is deleted and fetched again on the next run. The executables are linked into `tools\bin` (a hard link, which is
instant and costs no extra space) with LLVM's `lib` and CMake's `share` beside it, because both look for their files next
to their own `bin`.

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
recompiles, since the pipeline works from your own modules. The label on the
folder does not decide this: a tree labelled 1.1.1 that carries these same
seven modules is the supported build, while a folder renamed to look newer but
holding different bytes is reported as the different build it is.

An **`.nsp` package** works too, when it is a plaintext repack: one written
without keys, as unsigned program, control and meta NCAs. Pick it with
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
decrypted XCI created by your own dump tool is what this path reads. EmblemWeave never asks for keys and cannot decrypt a retail image.

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
