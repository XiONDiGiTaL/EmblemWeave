# What the release zip contains

Every release zip is built from this repository by `tools/package_release.ps1` on the Windows runner of
`.github/workflows/release.yml`. Nothing else goes in, and nothing is downloaded at packaging time except what the script
itself runs (`cargo build`, and `dotnet publish` for the shader translator). The same script still works locally for a
dry run.

Before it builds anything, the package script and the release workflow run `tools/check_release.py`. It fails unless
`Cargo.toml`, `docs/release.json`, the download button, the README's current-release line and the changelog index all
name the same version. The version bump and the site stamp are therefore checked together instead of being discovered
after the zip is published.

```
EmblemWeave-vX.Y.Z-windows-x64.zip
release-manifest.json    sidecar: SHA-256 of every file, plus source/build provenance
SHA256SUMS               combined checksums for the zip, translator and manifest
├── EmblemWeave.exe        the launcher (Rust, Release build) - this is the program you run
├── portable.txt           keeps settings, saves and build output in data/ beside the exe
├── LICENSE                EmblemWeave's own licence
├── README.md              the README from the tag the zip was built from
├── docs/                  every guide page the README links, the changelog and its screenshots
│   ├── PACKAGE.md         this file
│   ├── INSTALL.md, USAGE.md, BUILDING.md, SECURITY.md, TERMS.md, PRIVACY.md, LEGAL.md, DMCA.md
│   ├── RELEASE_VALIDATION.md, PORTABLE_INSTALLATION.md, UPGRADE_AND_RECOVERY.md
│   ├── changelog/         what changed in each release, newest first
│   └── screenshots/       the images the README shows
├── tools/                 offline archive, manifest and install verifiers (Python standard library)
├── upscalers/
│   └── ffmpeg/            the bundled FFmpeg 8.1 DLLs, GPLv3 text, hashes and source notes
└── engine/
    └── fefw-recomp/       the engine, in full source form (GPL-3.0-or-later, its own LICENSE and notices inside)
        ├── LEGAL.md, LICENSE, README.md, SECURITY.md
        ├── docs/          the engine's own documentation, including BUILDING.md and CONFIG.md
        ├── game-build/    the scripts and CMake projects that build fefw.exe and its shader step
        ├── recomp/        the AArch64 lifter that turns your dump into C++ sources, and its headers
        ├── runtime/       the runtime: Vulkan renderer, audio, HLE services, the F1 menu, the tests
        └── tools/
            ├── shaders/   the shader translator sources, the reference corpus, and
            │   └── prebuilt/win-x64/spirv_harness.exe   the translator, published with its own .NET runtime
            ├── textures/  the texture transcoders (astc-encoder, bc7enc_rdo, ispc are fetched, not shipped)
            ├── tiling/    the tiling/layout tools
            └── nso/       the NSO/module helpers
```

The engine tree is not a filesystem copy. `tools/package_release.ps1` asks `tools/verify_engine_tree.py` for the
`git ls-files` list, copies exactly those files, adds only the generated Windows shader translator, and then verifies
the copied tree has no untracked extras. The dump, `work`, `.github`, fetched texture/shader/download trees and the Linux
translator are not part of that list. A local object file or experimental source file therefore cannot enter a release
even when it sits in `engine/fefw-recomp`.

## Deliberately not in the zip

| Left out | Why |
| --- | --- |
| `game/`, `work/` | Your dump and everything generated from it. Never shipped, never uploaded. |
| `build/`, any `*.obj`, `*.pdb`, `*.lib`, `*.exe` under `engine/` | Build output. The only executable in the zip is `EmblemWeave.exe` and the translator above. |
| `tools/textures/third_party/`, `tools/shaders/third_party/`, `work/refs/` (not `runtime/third_party/`, which the build needs) | Fetched from their upstreams during **Build the game**, pinned by URL and hash in `runtime/cmake/gfx_deps.cmake` and `tools/textures/scripts/fetch_third_party.ps1`. |
| `node_modules/`, `dist/`, `.git/`, `.github/`, editor folders | Development leftovers, never part of a player package. `.github/` holds the engine's issue templates, which belong to the repository, not to a download. |
| `.venv/`, `__pycache__/`, `*.pyc`, `.pytest_cache/` | The engine's Python environment and byte-compiled caches, created on your PC by **Build the game**. |

`upscalers\ffmpeg` is the only vendor-DLL folder in the zip. It carries `avcodec-62.dll`, `avutil-60.dll` and
`swresample-6.dll` so the optional movie decoder works without a separate download; the AMD, NVIDIA and Intel upscaler
DLLs are still the player's to obtain. The launcher finds the shipped folder beside `EmblemWeave.exe` and writes its path
into `[media] ffmpeg_dir`; `upscalers\ffmpeg\README.md` records the version, hashes, licence and source links, and
`SHA256SUMS` is checked by the packaging script before it writes the zip.

`engine/fefw-recomp/game-build/release/BUNDLED_TOOLS.txt` describes the tool bundle the engine's own build ships (Zig, an
embedded Python, a prebuilt lifter). EmblemWeave does not use that bundle and does not ship those tools; the file is kept
because it travels with the engine sources and states, correctly, that none of them contains game code.

Nothing else is filtered: the zip is the source tree as it is in the repository, so a file that is in the repository
but not in the zip is one of the rows above.

## Nothing personal, nothing from the dump

Generated documentation and examples use generic drive paths (`C:\Program Files\LLVM\bin`, `D:\dumps\fefw\…`), never
a contributor's home folder, and the package carries no credentials, no `.env`, no keys and no logs. The two test
`.elf` fixtures and `recomp/oracle/difftest/corpus.json` are synthetic (hand-built instruction fixtures and AArch64 instruction
encodings), and `tools/shaders/corpus/sascha_dksh/` is Sascha Willems' MIT-licensed Vulkan sample shaders
rewritten into the Maxwell dialect, shipped with its licence. `runtime/assets/prompts/prompt_icons.pak` is built from
Kenney's CC0 Input Prompts by `runtime/scripts/build_prompt_pack.py`, again with its licence beside it. No file in the
zip is derived from a game dump: those live in `game/` and `work/` and never leave your PC.

## The Linux translator

`EmblemWeave-vX.Y.Z-shader-translator-linux-x64.tar.gz` (about 41 MB) holds `linux-x64/spirv_harness`, the same sources
published for `linux-x64`. It is not inside the Windows zip because a Windows player cannot run it: 41 MB of ELF would
double the download for nothing. The launcher looks for the translator under `tools/shaders/prebuilt/<platform>/` -
`win-x64` on Windows (including Wine and Proton, which run the Windows binary) and `linux-x64` elsewhere - so untarring
the asset into `engine/fefw-recomp/tools/shaders/prebuilt/` is all a Linux build needs. It is built and archived on an
Ubuntu runner by `.github/workflows/release.yml`, so the executable bit is recorded and checked by
`tools/verify_archive.py`; if a copy somehow loses it, `chmod +x linux-x64/spirv_harness`.

## Size

The engine sources and the shader translator are most of the download; the translator is published as a single
self-contained executable so the build needs no .NET SDK and no .NET runtime. The launcher itself is about 14 MB. The three
FFmpeg DLLs add about 35 MB compressed; the package built while adding them measured 112.5 MB.

## Extracting on Linux or macOS

The archive is a plain zip: every entry is a relative path with forward slashes, and everything sits inside one
`EmblemWeave-vX.Y.Z/` folder, as every release has since 0.1.5. Extract it anywhere; the folder becomes the install
folder, with `EmblemWeave.exe` beside the `docs/` and `engine/` folders it expects.

Two things keep it that way, and both run on every release:

* `tools/package_release.ps1` writes the zip entry by entry instead of handing the folder to a compressor. Each
  name is built with `/`, each folder gets its own entry, and the tree's executable bits are set on the entries
  that need them. It then runs the verifier below against the file it just wrote and fails the release if the
  verifier does.
* `tools/verify_archive.py` reads an archive back and refuses it when an entry is absolute (`/x`, `C:/x`), carries
  a drive letter, climbs out with `..`, starts with `./` or `.\`, uses a backslash, contains a control character,
  or collides with another entry's name or case (which is how a Windows build produces a zip that drops a file on
  Linux). With `--require` it also checks that the files a player needs are at the paths the launcher reads:
  `docs/PACKAGE.md`, `docs/changelog/`, `engine/fefw-recomp/`, `tools/` and the translator. With `--extract` it
  unpacks the archive into a temporary folder, checks nothing landed outside it, and checks the executable bit.
  It runs on Windows and on Linux with only the Python standard library, so a Linux user can check a download
  before extracting it:

```sh
python3 tools/verify_archive.py EmblemWeave-vX.Y.Z-windows-x64.zip --require EmblemWeave-vX.Y.Z/docs/PACKAGE.md
```

```sh
unzip -q EmblemWeave-vX.Y.Z-windows-x64.zip -d EmblemWeave
python3 -c "import zipfile;print(zipfile.ZipFile('EmblemWeave-vX.Y.Z-windows-x64.zip').namelist()[:3])"
```

Before it publishes, `tools/package_release.ps1` also extracts the zip and runs the engine's own CMake configure
from the extracted tree: the first step a missing source folder kills, and the one that caught the release without
`runtime/third_party`. It needs the same Visual Studio C++ tools and LLVM the game build needs, and fetches the
graphics dependencies a build fetches (into `%LOCALAPPDATA%\nxrt\deps`, so only the first run needs
the network). It skips, with that reason, when those tools are absent, and `-SkipConfigureCheck` turns it off.

Both print names like `EmblemWeave-vX.Y.Z/docs/PACKAGE.md` with `/`.

The symptom of files landing in one flat pile with `\` in their names comes
from re-packing the folder on Windows with a tool that writes backslashes (Windows PowerShell 5.1's `Compress-Archive`
does; `pwsh` 7 and Explorer do not). If you see it, check the archive's `sha256` against the release notes - if it differs,
you are extracting a re-zipped copy, not the download.

The release workflow builds both archives, then verifies the zip's paths and required engine files and the tarball's
executable bit before it publishes anything. It also configures the extracted engine on the Windows runner, which is the
step a missing source folder kills. The `--shader-translator-linux-x64.tar.gz` asset is built on Ubuntu, so its mode
comes from Linux rather than a Windows zip writer.

The `.exe` inside is a Windows program: it runs under Wine or Proton, not natively, and the launcher's build step installs
its tools with `winget` and runs PowerShell scripts, so it expects Windows. [STEAMDECK.md](STEAMDECK.md) is the Proton
route step by step: add the launcher as a non-Steam game, find the prefix, and re-point `fefw.ini`.

## Verifying a download

Each release note carries the `sha256` of its zip. On Windows:

```powershell
Get-FileHash EmblemWeave-vX.Y.Z-windows-x64.zip -Algorithm SHA256
```

Compare it with the value in the release notes. The release also publishes `release-manifest.json`, which lists every
file and its own SHA-256, plus a combined `SHA256SUMS`. The zip carries the same verifiers under `tools/`:

```powershell
python tools\verify_archive.py EmblemWeave-vX.Y.Z-windows-x64.zip
python tools\release_manifest.py verify --archive EmblemWeave-vX.Y.Z-windows-x64.zip --manifest release-manifest.json
python tools\verify_install.py --root . --manifest ..\release-manifest.json
```

The last command checks an unpacked install against the manifest; user data and generated build output are ignored. See
[RELEASE_VALIDATION.md](RELEASE_VALIDATION.md) for the full checklist. The source for the tag is public, so the strongest
check is to build the launcher yourself (`cargo build --release -p emblemweave`) and compare behaviour.

## Rebuilding the package

```powershell
pwsh -File tools/package_release.ps1 -Version 0.1.12
```

The script reads the version from `Cargo.toml` when `-Version` is omitted, builds the launcher and the translator if
they are missing, copies the tree above, writes `target/pkg/EmblemWeave-vX.Y.Z-windows-x64.zip`, and prints its
`sha256`.
