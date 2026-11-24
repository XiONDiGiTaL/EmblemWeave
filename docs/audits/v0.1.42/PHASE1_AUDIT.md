# EmblemWeave: Phase 1 - Complete Codebase Audit

> **Historical Phase 1 snapshot - not release evidence (reviewed 2026-11-24).**
> The authoritative status is `docs/audits/V0.1.42_RELEASE_READINESS.md`. This dependency and pipeline inventory predates
> Phase B. The "no artifact caching" and "fixed job count" observations describe this snapshot; Phase B code now has
> `tools.lock`, structured build events and adaptive `ConcurrencyCalculator`, but those are not independently validated
> here. No canonical engine build or engine test run exists.

## Project Overview

EmblemWeave is a native PC recompilation launcher for Fire Emblem: Fortune's Weave. It consists of:

1. **Rust Launcher** (`apps/emblemweave/`) - eframe/egui GUI application that:
   - Validates game data
   - Decodes compressed modules (ZBIC)
   - Runs the recompilation pipeline (AArch64 → C)
   - Builds the C++ engine via CMake/Ninja
   - Translates shaders via .NET-based shader translator
   - Launches and monitors the game runtime

2. **Rust Crates** (`crates/`) - Shared libraries:
   - `ew-format` - Container formats (NSO0, ExeFS, romfs, NSP, XCI, NUP)
   - `ew-lift` - AArch64 → C static recompilation pipeline
   - `ew-runtime` - Native runtime for recompiled modules
   - `ew-boot` - Whole-program dispatcher
   - `ew-gfx` - Vulkan device layer (ash)

3. **C++ Engine** (`engine/fefw-recomp/`) - The game runtime:
   - `recomp/` - AArch64 → C recompiler (Python-based lifter)
   - `runtime/` - Game runtime with:
     - Platform abstraction (Win32/POSIX)
     - Vulkan rendering + NVN compatibility layer
     - SDL3 for window/input
     - ImGui for overlay
     - zstd for shader pack/RomFS decompression
     - dav1d/libvpx/FFmpeg for video decoding
     - Upscalers (FSR, XeSS, DLSS, NIS)

---

## Dependency Graph - Rust Workspace

### Workspace Root (`Cargo.toml`)

```toml
[workspace.dependencies]
anyhow = "1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
lz4_flex = "0.11"
ruzstd = "0.7"
sha2 = "0.10"
```

### Crate Dependencies

| Crate | Direct Dependencies | Purpose |
|-------|---------------------|---------|
| `ew-format` | anyhow, serde, serde_json, lz4_flex, ruzstd, sha2 | Container formats (NSO, ExeFS, romfs, NSP, XCI, NUP) |
| `ew-lift` | anyhow, serde, serde_json, ew-format | AArch64 → C recompilation pipeline |
| `ew-runtime` | anyhow, serde, serde_json, ew-format | Native runtime for recompiled modules |
| `ew-boot` | anyhow, serde, serde_json, ew-lift, ew-format, ew-runtime, ew-gfx, libm, windows-sys | Whole-program dispatcher |
| `ew-gfx` | ash (Vulkan), windows-sys | Vulkan device layer for NVN |
| `emblemweave` (app) | anyhow, serde, serde_json, ew-*, eframe, rfd, image, ruzstd, tar, zstd, sha2, roxmltree, zip, windows-sys, winresource (build) | Main launcher application |

### Rust Dependency Analysis

| Dependency | Used By | Mandatory | Feature-Gatable | Lighter Alternative | Risk |
|------------|---------|-----------|-----------------|---------------------|------|
| `anyhow` | All crates | Yes | No | `eyre`, `thiserror` | Low - standard error handling |
| `serde` + `derive` | All crates | Yes | No | `serde` without derive (manual impl) | Medium - pervasive |
| `serde_json` | All crates | Yes | No | `simd-json`, `json` | Low - config/data only |
| `lz4_flex` | ew-format | Yes | No | `lz4` | Low - NSO decompression |
| `ruzstd` | ew-format, emblemweave | Yes | No | `zstd` crate | Low - Zstd decompression |
| `sha2` | ew-format, ew-boot, emblemweave | Yes | No | `sha2` is minimal | Low - hashing |
| `eframe` | emblemweave | Yes | No | `egui` + custom backend | High - major rewrite |
| `egui` | eframe | Yes | No | - | High - entire UI |
| `rfd` | emblemweave | Yes | No | `tinyfiledialogs` | Medium - file dialogs |
| `image` | emblemweave | Yes (png) | Features: `png` only | `png` crate directly | Medium - icons/screenshots |
| `tar` | emblemweave | Yes | No | `tar` is minimal | Low - package extraction |
| `zstd` | emblemweave | Yes | No | `ruzstd` already used | Low - package extraction |
| `roxmltree` | emblemweave | Yes | No | `xmltree`, `quick-xml` | Low - controls.ini |
| `zip` | emblemweave | Yes | No | `zip` is minimal | Low - package extraction |
| `windows-sys` | emblemweave, ew-boot, ew-gfx | Windows only | Yes (cfg) | Raw FFI | High - extensive Win32 |
| `ash` | ew-gfx | Yes | Features: `loaded` | `vulkanalia` | Medium - Vulkan bindings |
| `libm` | ew-boot | Yes | No | `std` math (no_std) | Low - math functions |
| `winresource` | emblemweave (build) | Build only | N/A | Manual .rc | Low - Windows resources |

### Duplicate/Transitive Analysis

- `anyhow`: 6 copies (workspace deduped)
- `serde` + `serde_json`: 6 copies (workspace deduped)
- `ruzstd`: 2 copies (workspace + emblemweave direct)
- `sha2`: 3 copies (workspace deduped)
- `windows-sys`: 3 copies (workspace + 2 crates direct)

**Optimization**: Move `ruzstd` to workspace dependencies only.

---

## Dependency Graph - C++ Engine

### Runtime CMake (`runtime/CMakeLists.txt`)

```cmake
# Core dependencies
- nxrt_zstd (vendored zstd 1.5.7)
- psapi (Windows system)
- SDL3-static (vendored 3.4.18)
- nxrt_volk (vendored Vulkan loader)
- nxrt_vma (vendored VMA 3.4.0)
- nxrt_imgui (vendored Dear ImGui 1.92.9b)
- nxrt_tiling (tools/tiling - tegra_tiling.cpp)
- nxrt_astc2bc7 (tools/textures - astc2bc7 + bc7e_ispc + astcdec DLLs)

# Optional
- libvpx (vcpkg, for VP9/VP8) - NXRT_HAVE_LIBVPX
- dav1d (vendored, for AV1) - NXRT_DAV1D
- FFmpeg headers (vendored, for libavcodec AV1) - runtime dlopen
```

### Graphics Dependencies (`gfx_deps.cmake`)

| Dependency | Version | License | Source | Purpose |
|------------|---------|---------|--------|---------|
| SDL3 | 3.4.18 | zlib | GitHub release | Window/input |
| Vulkan-Headers | 1.4.363.0 | Apache-2.0/MIT | GitHub | Vulkan API |
| volk | 1.4.363.0 | MIT | GitHub | Vulkan function loader |
| VMA | 3.4.0 | MIT | GitHub | Memory allocator |
| Dear ImGui | 1.92.9b | MIT | GitHub | Debug overlay |

### Third-Party Runtime (`third_party/`)

| Library | Purpose | Build Method |
|---------|---------|--------------|
| dav1d | AV1 decoder | CMake subdirectory (with NASM asm) |
| FFmpeg headers | libavcodec AV1 backend | Headers only, runtime DLLs |
| amd_ffx_api | FSR 3 frame gen | Header-only |
| amd_ffx_api_fg | FSR 3 FG API | Header-only |
| ffx_fsr1 | FSR 1 upscaler | Header-only |
| intel_xess | XeSS upscaler | Header-only |
| nvidia_nis | NIS upscaler | Header-only |
| nvidia_streamline | Streamline SDK | Header-only |
| renhdrum | HDR tonemapping | Header-only |

### Recompiler Dependencies (`recomp/`)

- Python 3.9+ with `zbic` package
- LLVM/Clang 23+ (clang-cl + lld-link)
- CMake 3.25+, Ninja
- Visual Studio 2022+ (MSVC toolset + Windows SDK)
- NASM 2.14+ (optional, for dav1d asm)
- .NET 10 SDK (only when shader translator not bundled)

---

## Build Pipeline Stages

### Launcher Pipeline (8 stages in `engine.rs`)

1. **Python environment** - Create uv venv, install requirements
2. **Prepare the ExeFS** - Extract main.nso uncompressed
3. **Recompile the game code** - Run `recomp/scripts/build_game.cmd` (Python lifter → C → clang-cl)
4. **Texture transcoders** - Build tools/textures (ISPC for astc2bc7)
5. **Build fefw.exe** - CMake configure + Ninja build (SDL3, Vulkan, NVN, etc.)
6. **Shader translator** - Clone Ryujinx repo, build SpirvHarness (.NET 10)
7. **Translate the shaders** - Run translator on game shaders → SPIR-V
8. **Write fefw.ini** - Generate config

### C++ Build (`runtime/scripts/build.ps1`)

- Environment setup via `env.cmd` (MSVC vcvars + LLVM + CMake + Ninja)
- CMake configure (Ninja generator, clang-cl)
- Ninja build with sccache (if available)
- Optional ctest

### Recompilation (`recomp/cmake/NxRecomp.cmake`)

- Python lifter (`nxrecomp lift`) generates C shards (default 16)
- clang-cl compiles shards with `/O2`, `-fno-strict-aliasing`, `-ffp-contract=off`, `-mcx16`
- Optional `nx_cc_launcher.py` for compile time limits

---

## Key Observations

### Rust
1. **Workspace deduping works well** - Most deps shared via workspace
2. **`ruzstd` duplicated** - In workspace AND emblemweave direct
3. **`eframe`/`egui` heavy** - ~50 transitive deps, but entire UI depends on it
4. **`ash` minimal** - Only `loaded` feature, no Vulkan headers duplication
5. **Windows-sys feature set large** - 15+ Win32 modules, could be trimmed per-crate

### C++
1. **Vendored dependencies** - All graphics deps fetched via FetchContent (good for reproducibility)
2. **sccache integration** - Already in build.ps1, cache dir configurable
3. **Unity build not used** - Each .cpp compiled separately
4. **Precompiled headers not used** - Could help with ImGui/SDL/Vulkan headers
5. **Fixed job count** - Default 10, overridden by NX_JOBS env (Phase 1 snapshot; Phase B adds adaptive
   `ConcurrencyCalculator`, not validated in this audit)
6. **Shader translator** - Heavy .NET build, only needed when not bundled

### Build Pipeline
1. **Repeated work** - Python venv created each build, shader translator cloned each CI
2. **No artifact caching** - Pipeline stages don't track input hashes (Phase 1 snapshot; Phase B adds `tools.lock`
   and structured events, but this audit did not validate an artifact cache)
3. **Blocking operations** - Launcher spawns processes synchronously on background threads
4. **Log file per stage** - Good for debugging, but many small files

---

## Phase 2 Optimization Opportunities

### Rust
1. Move `ruzstd` to workspace only
2. Feature-gate `windows-sys` per crate (already partially done)
3. Consider `zstd` feature in `image` crate (already minimal)
4. Audit `ew-boot` - only used by launcher, could be feature-gated

### C++
1. Enable unity builds for graphics sources (nxrt_gfx)
2. Add precompiled headers for ImGui/SDL/Vulkan
3. Make job count hardware-aware (detect cores + memory)
4. Feature-gate optional upscalers (FSR/XeSS/DLSS/NIS)
5. Cache shader translator binary in release

### Build Pipeline
1. Content-addressable cache for pipeline stages
2. Parallelize independent stages (texture transcoders + shader translator)
3. sccache for Rust (already in CI for C++)
4. Incremental shader translation (only changed shaders)