# What the release zip contains

Every release zip is built from this repository by `tools/package_release.ps1`. Nothing else goes in, and nothing is
downloaded at packaging time except what the script itself runs (`cargo build`, and `dotnet publish` for the shader
translator).

```
EmblemWeave-vX.Y.Z-windows-x64.zip
├── EmblemWeave.exe        the launcher (Rust, Release build) — this is the program you run
├── LICENSE                EmblemWeave's own licence
├── README.md              the README from the tag the zip was built from
├── PACKAGE.md             this file
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

## Deliberately not in the zip

| Left out | Why |
| --- | --- |
| `game/`, `work/` | Your dump and everything generated from it. Never shipped, never uploaded. |
| `build/`, any `*.obj`, `*.pdb`, `*.lib`, `*.exe` under `engine/` | Build output. The only executable in the zip is `EmblemWeave.exe` and the translator above. |
| `tools/textures/third_party/`, `tools/shaders/third_party/`, `work/refs/` | Fetched from their upstreams during **Build the game**, pinned by URL and hash in `runtime/cmake/gfx_deps.cmake` and `tools/textures/scripts/fetch_third_party.ps1`. |
| `launcher/` | The engine's own Tauri launcher. EmblemWeave has its own launcher and never runs this one; it was only carrying a second updater and 1 MB of dead weight. It remains in the repository. |
| `node_modules/`, `dist/`, `.git/`, `.github/`, editor folders | Development leftovers, never part of a player package. `.github/` holds the engine's issue templates, which belong to the repository, not to a download. |
| `.venv/`, `__pycache__/`, `*.pyc`, `.pytest_cache/` | The engine's Python environment and byte-compiled caches, created on your PC by **Build the game**. |

`engine/fefw-recomp/game-build/release/BUNDLED_TOOLS.txt` describes the tool bundle the engine's own build ships (Zig, an
embedded Python, a prebuilt lifter). EmblemWeave does not use that bundle and does not ship those tools; the file is kept
because it travels with the engine sources and states, correctly, that none of them contains game code.

Nothing else is filtered: the zip is the source tree as it is in the repository, so a file that is in the repository
but not in the zip is one of the rows above.

## Nothing personal, nothing from the dump

Generated documentation and examples use generic drive paths (`C:\Program Files\LLVM\bin`, `D:\dumps\fefw\…`), never
a contributor's home folder, and the package carries no credentials, no `.env`, no keys and no logs. The two test
`.elf` fixtures and `recomp/oracle/difftest/corpus.json` are synthetic — hand-built instruction fixtures and AArch64
instruction encodings — and `tools/shaders/corpus/sascha_dksh/` is Sascha Willems' MIT-licensed Vulkan sample shaders
rewritten into the Maxwell dialect, shipped with its licence. `runtime/assets/prompts/prompt_icons.pak` is built from
Kenney's CC0 Input Prompts by `runtime/scripts/build_prompt_pack.py`, again with its licence beside it. No file in the
zip is derived from a game dump: those live in `game/` and `work/` and never leave your PC.

## The Linux translator

`EmblemWeave-vX.Y.Z-shader-translator-linux-x64.tar.gz` (about 41 MB) holds `linux-x64/spirv_harness`, the same sources
published for `linux-x64`. It is not inside the Windows zip because a Windows player cannot run it: 41 MB of ELF would
double the download for nothing. The launcher looks for the translator under `tools/shaders/prebuilt/<platform>/` —
`win-x64` on Windows (including Wine and Proton, which run the Windows binary) and `linux-x64` elsewhere — so untarring
the asset into `engine/fefw-recomp/tools/shaders/prebuilt/` is all a Linux build needs. The archive keeps the executable
bit when it is made on Linux; if it arrives without one, `chmod +x linux-x64/spirv_harness`.

## Size

About 46 MB, of which about 38 MB is `spirv_harness.exe`: the shader translator published as a single self-contained
executable so the build needs no .NET SDK and no .NET runtime. The launcher itself is about 13 MB, the engine sources a
few MB.

## Extracting on Linux or macOS

The archive is a plain zip: every entry is a relative path with forward slashes, and everything sits inside one
`EmblemWeave-vX.Y.Z/` folder. `tools/package_release.ps1` refuses to write a zip whose entries use backslashes or
absolute paths, and every release is checked with a second, non-Windows reader:

```sh
unzip -q EmblemWeave-vX.Y.Z-windows-x64.zip -d EmblemWeave
python3 -c "import zipfile;print(zipfile.ZipFile('EmblemWeave-vX.Y.Z-windows-x64.zip').namelist()[:3])"
```

Both print `EmblemWeave-vX.Y.Z/…` with `/`. The symptom of files landing in one flat pile with `\` in their names comes
from re-packing the folder on Windows with a tool that writes backslashes (Windows PowerShell 5.1's `Compress-Archive`
does; `pwsh` 7 and Explorer do not). If you see it, check the archive's `sha256` against the release notes — if it differs,
you are extracting a re-zipped copy, not the download.

The `.exe` inside is a Windows program: it runs under Wine or Proton, not natively, and the launcher's build step installs
its tools with `winget` and runs PowerShell scripts, so it expects Windows. [STEAMDECK.md](STEAMDECK.md) is the Proton
route step by step: add the launcher as a non-Steam game, find the prefix, and re-point `fefw.ini`.

## Verifying a download

Each release note carries the `sha256` of its zip. On Windows:

```powershell
Get-FileHash EmblemWeave-vX.Y.Z-windows-x64.zip -Algorithm SHA256
```

Compare it with the value in the release notes. The source for the tag is public, so the strongest check is to build
the launcher yourself (`cargo build --release -p emblemweave`) and compare behaviour.

## Rebuilding the package

```powershell
pwsh -File tools/package_release.ps1 -Version 0.1.12
```

The script reads the version from `Cargo.toml` when `-Version` is omitted, builds the launcher and the translator if
they are missing, copies the tree above, writes `target/pkg/EmblemWeave-vX.Y.Z-windows-x64.zip`, and prints its
`sha256`.
