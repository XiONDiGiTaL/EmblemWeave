# System requirements

| Component | Requirement |
| --- | --- |
| Operating system | Windows 10 or 11, 64-bit |
| Graphics | A GPU with a Vulkan 1.3 driver. Upscalers are optional and need their vendor files (see [Upscaling](USAGE.md#upscaling)) |
| Processor | x86-64 with SSE4.2, POPCNT and CMPXCHG16B: Intel Nehalem (2008) or AMD Bulldozer (2011) and newer. FMA3 + AVX (Intel Haswell 2013, AMD Piledriver 2012) get the faster build |
| Memory | 8 GB minimum, 16 GB recommended |
| Storage | About 30 GB for the extracted game data, plus space for build output |
| Game | Your own legally obtained copy of *Fire Emblem: Fortune's Weave* |

The engine builds the game's lifted code for one of two processor targets. The launcher probes the CPU and chooses:
`fma` (FMA3 + AVX, fused multiply-add for the guest's FMADD/FMLA family) where the processor has it, otherwise `compat`
(SSE4.2 + POPCNT, with `fma`/`fmaf` as correctly rounded library calls). Sandy Bridge, Ivy Bridge and Xeon E5 v1/v2 fall in
the second group. A processor without SSE4.2, POPCNT or CMPXCHG16B (Core 2, Phenom II and older) cannot run the game: the
launcher refuses to build for it, and an existing build moved to such a PC stops with exit code 4 and a message naming the
missing features in `fefw_fatal.txt` instead of an illegal-instruction crash. The Diagnostics page shows the brand, the
detected instruction sets and the target the next build will use.

Building the game needs the tools below. The launcher checks every one with the same detection the build uses before it
starts, and the Requirements page names each missing one with the command that installs it.

<!-- requirements:start -->
| Dependency | Needed for | Version | Missing it |
| --- | --- | --- | --- |
| Visual Studio C++ tools | Build the game | 2022 (17) or newer | `winget install --id Microsoft.VisualStudio.2022.BuildTools --exact --override "--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended" --accept-package-agreements --accept-source-agreements` |
| Windows 10/11 SDK | Build the game | 10.0 | `Add Windows 11 SDK under Individual components in the Visual Studio Installer, or install the standalone SDK, then press Check again.` |
| LLVM (clang-cl and lld-link) | Build the game | 23 | `winget install --id LLVM.LLVM --exact --accept-package-agreements --accept-source-agreements` |
| CMake | Build the game | 3.25 | `winget install --id Kitware.CMake --exact --accept-package-agreements --accept-source-agreements` |
| Ninja | Build the game | any | `winget install --id Ninja-build.Ninja --exact --accept-package-agreements --accept-source-agreements` |
| NASM | Build the game | 2.14 | `winget install --id NASM.NASM --exact --accept-package-agreements --accept-source-agreements` |
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

The .NET 10 SDK is **not** needed: the release zip carries the shader translator published with its own runtime, so the
shader step runs on a PC with no .NET at all. The same sources also publish for Linux
(`EmblemWeave-vX.Y.Z-shader-translator-linux-x64.tar.gz`), and the build looks for the translator under
`tools/shaders/prebuilt/<platform>/`, so a Linux build picks the ELF binary on its own. For a Steam Deck, build on Windows
and run the result under Proton: [STEAMDECK.md](STEAMDECK.md) has the steps, the prefix paths and the `fefw.ini`
edits.

To build the launcher from source you also need the tools listed in [Building from source](BUILDING.md#building-from-source).
