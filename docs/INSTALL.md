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
