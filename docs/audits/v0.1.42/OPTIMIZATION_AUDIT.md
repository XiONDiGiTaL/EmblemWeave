# EmblemWeave: Phase A - Comparative Architecture Research & Dependency Audit

> **Historical Phase A snapshot - not release evidence (reviewed 2026-11-24).**
> The authoritative status is `docs/audits/V0.1.42_RELEASE_READINESS.md`. This research predates Phase B and FCC's
> current engine refactor. The "EmblemWeave Current" columns are a 2026-11-20 baseline, not a release claim. No canonical
> engine build or engine test run exists.
>
> **Post-Phase-A corrections:**
> - `tools.lock`, structured build events and adaptive concurrency were added in Phase B. The "no lockfile" and
>   "fixed NX_JOBS" findings below describe the Phase A baseline, not current code.
> - The `write-watch ❌` row predates FCC's active platform split. FCC's uncommitted working tree adds
>   `platform/win32/write_watch_win32.cpp` and `platform/posix/write_watch_linux.cpp`; this is not integrated into
>   canonical `main` and is not release evidence.
> - These recommendations are research, not a build, test or release pass.

## Overview

This document presents the findings from Phase A of the Comprehensive Architecture, Dependency, Build Lifecycle and Launcher Optimization Initiative. It contains:

1. **Comparative Architecture Research** - Analysis of four reference projects (Recompia/RetroLauncher, Hookshot, LongshotLauncher, dkb-pc-recomp)
2. **Full Dependency Audit** - Systematic classification of every dependency in the EmblemWeave repository

---

## Part 1: Comparative Architecture Research

### Reference Projects Analyzed

| Project | Type | Key Architecture Decisions |
|---------|------|---------------------------|
| **Recompia / RetroLauncher** | Multi-title launcher (PSX) | vcpkg + FetchContent, portable toolchain packs, catalog caching, background installs |
| **Hookshot** | Single-title (OoT) with builder | Python-driven `make_game.py`, llvm-mingw/emsdk, hard-link RomFS, builder cache |
| **LongshotLauncher** | Single-title (OoT) fork with enhanced launcher | Signed builder releases, `tools.lock`, intelligent cache migration, portable/installed modes |
| **dkb-pc-recomp** | Single-title (DK Bananza) with installer | Vendored deps, MSYS2 UCRT64 Clang, .NET 10 NativeAOT shader translator, write-watch tracking |

---

### 1. Launcher Architecture Patterns

#### A. Build Orchestration

| Pattern | Recompia | Hookshot | LongshotLauncher | dkb-pc-recomp | **EmblemWeave Current** |
|---------|----------|----------|------------------|---------------|------------------------|
| **Build Driver** | CMake + Ninja | Python `make_game.py` | Python `make_game.py` via protocol | Python `build_game.py` | Rust `engine.rs` (8 stages) |
| **Parallelism** | CMake `-j` | `HOOKSHOT_JOBS` + memory limit | Per-build `LONGSHOT_JOBS` | Ninja parallel | Fixed `NX_JOBS` (capped 16; Phase A) |
| **Incremental** | Preserves intermediates | Deletes build artifacts by default | Object/lift stores (`LONGSHOT_OBJ_STORE`) | Object hashing pre-link | sccache only (no object store) |
| **Protocol** | CMake native | `@@` progress lines | `MakeGameProtocol` (structured) | Custom | Ad-hoc log parsing |

**Key Insight**: LongshotLauncher's structured protocol (`MakeGameCommand`/`MakeGameProtocol`/`MakeGameRun`) with `@@` progress parsing is superior to ad-hoc log parsing. EmblemWeave should adopt a structured build protocol.

#### B. Toolchain Management

| Pattern | Recompia | Hookshot | LongshotLauncher | dkb-pc-recomp | **EmblemWeave Current** |
|---------|----------|----------|------------------|---------------|------------------------|
| **Strategy** | Portable toolchain packs | llvm-mingw / emsdk | Signed builder releases | MSYS2 UCRT64 Clang | `env.cmd` + `nx_env.cmd` (VS + LLVM) |
| **Version Pinning** | Toolchain packs | Builder-provided tools | `tools.lock` (SHA-256) | Vendored in repo | `env.cmd` paths, no lockfile |
| **Compiler** | System / vcpkg | Clang + LLD | Bundled LLVM (llvm-mingw) | Clang + LLD | clang-cl + lld-link |
| **Verification** | Check on use | Implicit via builder | SHA-256 verification | Every object hashed | Basic `--version` check |
| **Windows SDK** | vcpkg / system | MSVC fallback | Via builder | MSYS2 | VS `vcvars64.bat` |

**Key Insight**: LongshotLauncher's `tools.lock` with SHA-256 verification of every toolchain component is the gold standard. EmblemWeave lacks a lockfile and relies on environment detection.

#### C. Cache Management Strategies

| Cache Type | Recompia | Hookshot | LongshotLauncher | dkb-pc-recomp | **EmblemWeave Current** |
|------------|----------|----------|------------------|---------------|------------------------|
| **Build Artifacts** | Preserved for incremental | Deleted unless `KEEP=1` | **Object/lift stores** shared | Object hashing pre-link | None (sccache only) |
| **Shader Translation** | Catalog cache | JIT on disk | **10 GB cache migrated on update** | 29K pre-translated + JIT | **Content-addressable (Phase 3)** |
| **Pipeline Cache** | Driver cache | NVIDIA ~2 GB first run | **Pipeline-only migration** (2¼ vs 83 min) | Keys saved, `VK_EXT_GPL` | `pipeline_cache.bin` (monolithic) |
| **Texture/Asset** | Overlay captures | Hard-linked RomFS | Build files folder | Write-watch dirty pages | Full copy |
| **Cache GC** | `retcomm cache gc` | Manual | **Free up space** UI | Not specified | Manual `work/` cleanup |

**Key Insight**: LongshotLauncher's intelligent cache migration (pipeline-only when translator/compiler changes, full when identical) with measured 37x speedup is exceptional. EmblemWeave's new ShaderCache is a good start but lacks pipeline cache intelligence.

#### D. Installation Workflow

| Aspect | Recompia | Hookshot | LongshotLauncher | dkb-pc-recomp | **EmblemWeave Current** |
|--------|----------|----------|------------------|---------------|------------------------|
| **Windows** | Installed + portable | Builder exe (500 MB) | **Setup exe OR portable** | Installer builds locally | Portable only (no installer) |
| **Linux** | AppImage / native | AppImage | AppImage + `.desktop` | Not primary | Not supported |
| **Updates** | Background queue | Rebuild in-place | **Signed, verified, rollback** | Rebuild | Manual rebuild |
| **Self-Update** | Hub manages | Launcher detects | `--self-check`, `--rollback` | N/A | Not implemented |
| **Data Preservation** | Saves preserved | Settings preserved | **Game, settings, mods, caches** | User provides keys | `engine.txt` remembers build |

**Key Insight**: LongshotLauncher's dual-mode (installed/portable) with signed updates, rollback, and comprehensive data preservation is the most complete. EmblemWeave is portable-only with no self-update mechanism.

#### E. Storage Management

| Aspect | Recompia | Hookshot | LongshotLauncher | dkb-pc-recomp | **EmblemWeave Current** |
|--------|----------|----------|------------------|---------------|------------------------|
| **Data Separation** | Config/State/Cache split | Game folder + builder cache | **XDG dirs (Linux), AppData (Win)** | Guest=host addresses | Single `Config::dir()` |
| **Saves** | RomM sync | `~/.local/share/hookshot/` | **Shared + per-instance** | Standard | `Config::save_dir()` |
| **Build Instances** | `releases/<tag>/` + `current` link | Single build folder | **Track/version folders** | Single build | Single `work/build/` |
| **Portable Mode** | `portable` file | AppImage.home | **`LONGSHOT_LAUNCHER_HOME`** | N/A | Portable by default |
| **Cleanup** | `retcomm cache gc` | Manual | **Free up space UI** | Not specified | Manual |

**Key Insight**: LongshotLauncher's XDG-compliant separation (config/state/cache) with track/version build instances and "Free up space" UI is the most sophisticated. EmblemWeave conflates all data in one folder.

#### F. Runtime/Build Separation

| Aspect | Recompia | Hookshot | LongshotLauncher | dkb-pc-recomp | **EmblemWeave Current** |
|--------|----------|----------|------------------|---------------|------------------------|
| **Build Artifacts** | Linked into work tree | Deleted post-build | **Stores in build files folder** | Object hashing | `work/build/` + `work/lift/` |
| **Runtime Data** | Installs link to ROMs | Game folder | Shared user folder | Standard | `run_dir()` (play/) |
| **Shader Translator** | Bundled in release | Built from source | **Bundled + self-contained** | .NET NativeAOT | Bundled OR built (.NET SDK) |
| **Platform Abstraction** | Retro-Runtime | Win32/Linux in `nx2` | Builder handles | `runtime/platform/` | ew-boot + nxrt platform |

**Key Insight**: dkb-pc-recomp's .NET 10 NativeAOT translator (`totk_xlate.exe`) eliminates .NET SDK dependency entirely. LongshotLauncher bundles translator with self-contained runtime. EmblemWeave still requires .NET SDK when unbundled.

---

### 2. Best Engineering Ideas to Adopt

#### From LongshotLauncher (Highest Priority)

1. **`tools.lock` with SHA-256 verification** - Immutable toolchain provenance
2. **Structured build protocol** (`MakeGameProtocol`) - Replace ad-hoc log parsing
3. **Intelligent cache migration** - Pipeline-only vs full based on toolchain/translator hash
4. **Dual-mode installation** (installed + portable) with signed updates + rollback
5. **XDG-compliant storage separation** - Config/State/Cache split
6. **Track/version build instances** - Multiple builds coexist, easy rollback
7. **"Free up space" UI** - User-controlled cache reclamation with size estimates
8. **Object/lift stores** (`LONGSHOT_OBJ_STORE`, `LONGSHOT_LIFT_STORE`) - True incremental builds

#### From dkb-pc-recomp

9. **.NET NativeAOT shader translator** - Eliminate .NET SDK requirement
10. **Write-watch memory tracking** (`MEM_WRITE_WATCH`) - Dirty page upload optimization
11. **Per-object hashing pre-link** - RAM fault mitigation
12. **`VK_EXT_graphics_pipeline_library`** - Fast pipeline linking
13. **Guest=host address space** (below 2^39) - IPC compatibility

#### From Hookshot

14. **Hard-link RomFS** - Avoid copying large assets
15. **Memory limit for compiler** (`HOOKSHOT_MEM_LIMIT_GB`) - Prevent OOM
16. **Mod resource table caching** - Rebuild only on change

#### From Recompia/RetroLauncher

17. **vcpkg + FetchContent hybrid** - Reproducible third-party deps
18. **Catalog caching with incremental hits** - Content-addressed DLL cache
19. **Background install queue** - Play while building
20. **Platform config merging** - Global per-platform settings

---

### 3. Architecture Comparison Matrix

| Criterion | EmblemWeave | LongshotLauncher | Hookshot | dkb-pc-recomp | Recompia |
|-----------|-------------|------------------|----------|---------------|----------|
| **Launcher Language** | Rust (eframe) | C# (Tauri?) | Python + GUI | Python + GUI | C++ (ImGui) |
| **Build Driver** | Rust orchestrates 8 stages | Python protocol | Python script | Python script | CMake |
| **Toolchain Lockfile** | ❌ (Phase A) | ✅ `tools.lock` | ❌ | ❌ | ❌ (packs) |
| **Incremental Build** | sccache only | Object/lift stores | ❌ | Object hashing | Preserves intermediates |
| **Shader Cache** | Content-addressable (NEW) | 10 GB smart migration | JIT disk | 29K pre + JIT | Catalog |
| **Pipeline Cache** | Monolithic file | Smart migration | Driver | Keys + GPL | Driver |
| **Installer** | ❌ Portable only | ✅ Dual mode + signed | Builder exe | Local build | Hub managed |
| **Self-Update** | ❌ | ✅ Signed + rollback | Rebuild | N/A | Background queue |
| **Storage Separation** | Single folder | XDG Config/State/Cache | Game + builder | Standard | Config/State/Cache |
| **Portable Mode** | Default | `LONGSHOT_LAUNCHER_HOME` | AppImage.home | N/A | `portable` file |
| **Multi-Build** | Single `work/` | Track/version folders | Single | Single | Releases + current link |
| **Cache GC UI** | ❌ | ✅ "Free up space" | ❌ | ❌ | `retcomm cache gc` |
| **Shader Translator** | .NET (bundled or SDK) | Bundled self-contained | Built from source | .NET NativeAOT | N/A |
| **Write-Watch** | ❌ (Phase A) | ❌ | ❌ | ✅ `MEM_WRITE_WATCH` | ❌ |
| **Hard-Link Assets** | ❌ | ✅ RomFS | ✅ | ❌ | Links to ROMs |

**2026-11-24 correction:** the EmblemWeave write-watch ❌ above is stale. FCC's uncommitted tree replaces
`mem/write_watch.cpp` with platform implementations; they are not integrated into canonical `main`.

---

## Part 2: Full Dependency Audit

### Classification Categories

Every dependency is classified into one of 7 categories:

| Category | Description | Examples |
|----------|-------------|----------|
| **Core** | Required for basic operation, no alternative | `anyhow`, `serde`, `SDL3`, `Vulkan` |
| **Platform** | OS-specific, feature-gated by target | `windows-sys`, `winresource` |
| **Graphics** | Rendering pipeline, upscalers, video | `ash`, `imgui`, `vma`, `dav1d`, `libvpx` |
| **Recompiler** | AArch64→C pipeline, lifter, boot | `unicorn`, `capstone`, `zbic`, `pyelftools` |
| **Build** | Compile-time only, not in final binary | `cmake`, `ninja`, `clang-cl`, `sccache`, `uv` |
| **Optional** | Feature-gated, can be disabled at configure | `NXRT_UPSCALER_*`, `NXRT_DAV1D`, `NXRT_LIBVPX` |
| **Dev** | Development/testing only | `cargo-tarpaulin`, test fixtures |

---

### 2.1 Rust Workspace Dependencies

#### Workspace Root (`Cargo.toml`)

| Dependency | Version | Category | Used By | Feature-Gated | Notes |
|------------|---------|----------|---------|---------------|-------|
| `anyhow` | 1 | Core | All crates | No | Standard error handling |
| `serde` | 1 (derive) | Core | All crates | No | Pervasive serialization |
| `serde_json` | 1 | Core | All crates | No | Config/data only |
| `lz4_flex` | 0.11 | Core | ew-format | No | NSO decompression |
| `ruzstd` | 0.7 | Core | ew-format, emblemweave | No | Zstd decompression |
| `sha2` | 0.10 | Core | ew-format, ew-boot, emblemweave | No | Hashing |

#### Crate-Level Dependencies

| Crate | Dependency | Version | Category | Feature-Gated | Purpose |
|-------|------------|---------|----------|---------------|---------|
| `ew-format` | `anyhow`, `serde`, `serde_json`, `lz4_flex`, `ruzstd`, `sha2` | workspace | Core | No | Container formats |
| `ew-lift` | `anyhow`, `serde`, `serde_json`, `ew-format` | workspace | Recompiler | No | AArch64→C pipeline |
| `ew-runtime` | `anyhow`, `serde`, `serde_json`, `ew-format` | workspace | Core | No | Native runtime |
| `ew-boot` | `anyhow`, `serde`, `serde_json`, `ew-lift`, `ew-format`, `ew-runtime`, `ew-gfx`, `libm`, `windows-sys` | workspace + direct | Core + Platform | Partial (windows-sys) | Whole-program dispatcher |
| `ew-gfx` | `ash` (loaded), `windows-sys` | direct | Graphics + Platform | Partial (windows-sys) | Vulkan device layer |
| `emblemweave` (app) | `eframe` (0.31), `rfd`, `image` (png), `tar`, `zstd`, `roxmltree`, `zip`, `windows-sys`, `winresource` (build) | direct | Core + Platform + Graphics + Build | Partial | Main launcher |

#### Rust Dependency Issues Found

1. **`ruzstd` duplicated** - In workspace deps AND emblemweave direct deps
2. **`windows-sys` feature sets not minimized per-crate** - 15+ modules in app, subsets in ew-boot/ew-gfx
3. **`eframe`/`egui` heavy** - ~50 transitive deps, entire UI depends on it
4. **`winresource` build dep only** - Correctly separated
5. **No optional feature gating for graphics crates** - `ash` only uses `loaded` feature (good)

---

### 2.2 C++ Engine Dependencies (`engine/fefw-recomp/runtime`)

#### Core Runtime (via CMake)

| Dependency | Version | Category | Source | Feature Gate | Purpose |
|------------|---------|----------|--------|--------------|---------|
| `nxrt_zstd` | 1.5.7 | Core | Vendored (CMake) | Always | Shader pack/RomFS decompression |
| `psapi` | System | Platform | Windows SDK | Windows only | Process memory info |
| `SDL3` | 3.4.18 | Core | FetchContent | Always | Window/input |
| `nxrt_volk` | 1.4.363.0 | Graphics | FetchContent | Always | Vulkan function loader |
| `vulkan_headers` | 1.4.363.0 | Graphics | FetchContent | Always | Vulkan API |
| `nxrt_vma` | 3.4.0 | Graphics | FetchContent | Always | Memory allocator |
| `nxrt_imgui` | 1.92.9b | Graphics | FetchContent | Always | Debug overlay |
| `nxrt_tiling` | - | Graphics | tools/tiling | Always | Tegra tiling |
| `nxrt_astc2bc7` | - | Graphics | tools/textures | `NXRT_ASTC2BC7` | ASTC→BC7 transcoder |

#### Optional Graphics Dependencies

| Dependency | Version | Category | Source | Feature Gate | Purpose |
|------------|---------|----------|--------|--------------|---------|
| `dav1d` | Latest | Optional | Vendored (CMake) | `NXRT_DAV1D` | AV1 video decoder |
| `libvpx` | Latest | Optional | vcpkg | `NXRT_LIBVPX` | VP9/VP8 video decoder |
| `FFmpeg headers` | Latest | Optional | Vendored | Runtime dlopen | libavcodec AV1 backend |
| `amd_ffx_api` | Latest | Optional | Header-only | `NXRT_UPSCALER_FSR` | FSR 3 frame gen |
| `amd_ffx_api_fg` | Latest | Optional | Header-only | `NXRT_UPSCALER_FSR` | FSR 3 FG API |
| `ffx_fsr1` | Latest | Optional | Header-only | `NXRT_UPSCALER_FSR` | FSR 1 upscaler |
| `intel_xess` | Latest | Optional | Header-only | `NXRT_UPSCALER_XESS` | XeSS upscaler |
| `nvidia_nis` | Latest | Optional | Header-only | `NXRT_UPSCALER_NIS` | NIS upscaler |
| `nvidia_streamline` | Latest | Optional | Header-only | `NXRT_UPSCALER_DLSS` | Streamline SDK |
| `renhdrum` | Latest | Optional | Header-only | Always | HDR tonemapping |

#### Recompiler Dependencies (`engine/fefw-recomp/recomp`)

| Dependency | Version | Category | Source | Purpose |
|------------|---------|----------|--------|---------|
| Python | 3.9+ | Build | System | Host for lifter |
| `unicorn` | 2.1.4 | Recompiler | PyPI | Reference CPU for oracles |
| `capstone` | 5.0.9 | Recompiler | PyPI | Instruction decode |
| `lz4` | Latest | Recompiler | PyPI | NSO decompression |
| `pyelftools` | Latest | Recompiler | PyPI | ELF reading for tests |
| `zbic` | Latest | Recompiler | PyPI (GitHub) | ZBIC-compressed NSOs |
| `zstandard` | Latest | Recompiler | PyPI | .zst archives in romfs |
| LLVM/Clang | 23+ | Build | System | clang-cl + lld-link |
| CMake | 3.25+ | Build | System | Build configuration |
| Ninja | Latest | Build | System | Build execution |
| Visual Studio | 2022+ | Build | System | MSVC toolset + Windows SDK |
| NASM | 2.14+ | Build | Optional | dav1d asm |
| .NET SDK | 10 | Build | Conditional | Shader translator build |

#### Shader Translator Dependencies (`engine/fefw-recomp/tools/shaders`)

| Dependency | Version | Category | Source | Purpose |
|------------|---------|----------|--------|---------|
| Ryujinx.Common | a82350b | Translator | Git (staged) | Common utilities |
| Spv.Generator | a82350b | Translator | Git (staged) | SPIR-V generation |
| Ryujinx.Graphics.Shader | a82350b | Translator | Git (staged) | Shader translation |
| .NET SDK | 10 | Build | System | Build translator |
| NuGet packages | Pinned | Translator | nuget.config | .NET dependencies |

---

### 2.3 Dependency Matrix Summary

#### By Category

| Category | Count | Mandatory | Optional | Feature-Gated | Dev/Build Only |
|----------|-------|-----------|----------|---------------|----------------|
| **Core** | 12 | 12 | 0 | 0 | 0 |
| **Platform** | 3 | 2 | 1 | 3 | 0 |
| **Graphics** | 18 | 6 | 12 | 12 | 0 |
| **Recompiler** | 7 | 7 | 0 | 0 | 0 |
| **Build** | 10 | 8 | 2 | 0 | 10 |
| **Optional** | 12 | 0 | 12 | 12 | 0 |
| **Dev** | 2 | 0 | 0 | 0 | 2 |
| **Total** | 64 | 35 | 27 | 27 | 12 |

#### By Language/Ecosystem

| Ecosystem | Dependencies | Count |
|-----------|--------------|-------|
| Rust (crates.io) | anyhow, serde, serde_json, lz4_flex, ruzstd, sha2, eframe, rfd, image, tar, zstd, roxmltree, zip, windows-sys, ash, libm, winresource | 17 |
| C++ (FetchContent) | SDL3, Vulkan-Headers, volk, VMA, imgui, dav1d, zstd | 7 |
| C++ (Header-only) | amd_ffx_api, amd_ffx_api_fg, ffx_fsr1, intel_xess, nvidia_nis, nvidia_streamline, renhdrum | 7 |
| C++ (vcpkg) | libvpx | 1 |
| C++ (Tools) | tegra_tiling, astc2bc7, bc7e_ispc, rgbcx | 4 |
| Python (PyPI) | unicorn, capstone, lz4, pyelftools, zbic, zstandard | 6 |
| System Tools | LLVM/Clang, CMake, Ninja, VS, NASM, .NET SDK | 7 |
| .NET (NuGet) | Ryujinx.* | 3 |

---

### 2.4 Critical Dependency Issues

#### High Priority

1. **No toolchain lockfile (Phase A; superseded in Phase B)** - Unlike LongshotLauncher's `tools.lock`, the Phase A
   baseline had no SHA-256 verification of toolchain components
2. **`.NET SDK required when translator not bundled`** - Unlike dkb-pc-recomp's NativeAOT or LongshotLauncher's self-contained bundle
3. **sccache only for C++** - Rust builds don't use sccache (though `.cargo/config.toml` configures it)
4. **No object/lift stores** - Unlike LongshotLauncher's `LONGSHOT_OBJ_STORE`/`LONGSHOT_LIFT_STORE` for true incremental builds
5. **Single build directory** - No track/version support for multiple builds or easy rollback

#### Medium Priority

6. **`ruzstd` duplicated** - Workspace + direct dependency
7. **`windows-sys` feature sets not minimized** - Large feature sets in app and crates
8. **No memory limit for compiler** - Unlike Hookshot's `HOOKSHOT_MEM_LIMIT_GB`
9. **No hard-link asset optimization** - Unlike Hookshot's RomFS hard-linking
10. **Monolithic pipeline cache** - Unlike LongshotLauncher's intelligent migration

#### Low Priority

11. **NASM optional but not feature-gated in CMake** - Could be `NXRT_NASM`
12. **Git not needed (FetchContent used)** - Listed as optional in deps.rs but not in CMake
13. **uv optional but not used in CI** - Only for local dev speed

---

## Part 3: Recommendations for Phase B-G

Based on the comparative research and dependency audit, the following priorities emerge:

### Phase B: Launcher Toolchain Management (Highest Impact)
- Implement `tools.lock` with SHA-256 verification (LongshotLauncher pattern)
- Add structured build protocol to replace log parsing
- Bundle .NET NativeAOT shader translator (dkb-pc-recomp pattern)
- Add compiler memory limit (Hookshot pattern)

### Phase C: Build Lifecycle Overhaul
- Implement object/lift stores for true incremental builds
- Add track/version build instances with rollback
- Intelligent cache migration (pipeline-only vs full)
- Hard-link asset optimization

### Phase D: Storage Management Redesign
- XDG-compliant Config/State/Cache separation
- Portable mode via environment variable
- "Free up space" UI with size estimates
- Shared saves with per-instance caches

### Phase E: Runtime/Build Separation
- Strict separation of build artifacts vs runtime data
- Content-addressable stores for all cache types
- Write-watch memory tracking for texture uploads
- `VK_EXT_graphics_pipeline_library` adoption

### Phase F: Installation Experience
- Dual-mode installer (installed + portable)
- Signed updates with rollback
- Self-update mechanism
- Background build queue

### Phase G: Validation & Benchmarks
- Benchmark suite for each optimization
- Regression tests for cache migration
- Memory/CPU profiling of build pipeline

---

## Sources

- [RetroPortingToolKit/Retro-Launcher](https://github.com/RetroPortingToolKit/Retro-Launcher)
- [Recompia](https://recompia.com/)
- [Hookshot - How It Works](https://hookshotbuilder.top/how-it-works)
- [Hookshot GitHub](https://github.com/zbray/hookshot)
- [LongshotLauncher](https://git.magicbean.dev/MagicBeanDev/LongshotLauncher)
- [Longshot Launcher Website](https://longshotlauncher.com/)
- [dkb-pc-recomp](https://github.com/Lucas-blip-png/dkb-pc-recomp)
- [dkb-pc-recomp Architecture](https://github.com/Lucas-blip-png/dkb-pc-recomp/blob/bananza/docs/architecture.md)
- [Recomps.wtf - Static Recompilation Pipeline](https://recomps.wtf/learn/static-recompilation-pipeline)