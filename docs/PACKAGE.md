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
| `node_modules/`, `dist/`, `.git/` | Development leftovers, never part of a player package. |
| `.venv/` | The engine's Python environment, created on your PC by **Build the game**. |

## Size

About 46 MB, of which about 38 MB is `spirv_harness.exe`: the shader translator published as a single self-contained
executable so the build needs no .NET SDK and no .NET runtime. The launcher itself is about 13 MB, the engine sources a
few MB.

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
