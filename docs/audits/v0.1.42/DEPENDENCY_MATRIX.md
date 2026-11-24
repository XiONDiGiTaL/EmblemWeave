# EmblemWeave: Complete Dependency Matrix

> **Historical dependency inventory - not a build or release pass (reviewed 2026-11-24).**
> The authoritative status is `docs/audits/V0.1.42_RELEASE_READINESS.md`. This matrix is a Phase A/B snapshot; it is not
> a build, regression or performance validation. No canonical engine build or engine test run exists.
>
> **Status corrections:**
> - Phase B items in section 8 are code-level implementations. Regression tests and performance measurements remain
>   pending; do not read them as a release pass.
> - `No toolchain lockfile` and `No structured build protocol` in section 5.1 describe Phase A. Phase B added
>   `tools.lock`, BuildEvents and `ConcurrencyCalculator`.
> - `No write-watch tracking` in section 5.4 is superseded by FCC's uncommitted platform split; it is not integrated
>   into canonical `main`.

Generated from Phase A audit. Every dependency classified into 7 categories.

---

## Classification Legend

| Category | Symbol | Description |
|----------|--------|-------------|
| **Core** | 🔴 | Required for basic operation, no alternative |
| **Platform** | 🟠 | OS-specific, feature-gated by target |
| **Graphics** | 🟢 | Rendering pipeline, upscalers, video |
| **Recompiler** | 🔵 | AArch64→C pipeline, lifter, boot |
| **Build** | 🟡 | Compile-time only, not in final binary |
| **Optional** | 🟣 | Feature-gated, can be disabled at configure |
| **Dev** | ⚪ | Development/testing only |

| Status | Symbol | Description |
|--------|--------|-------------|
| **Mandatory** | ✅ | Always required |
| **Optional** | ⭕ | Can be disabled |
| **Feature-Gated** | 🔧 | Controlled by CMake option / Cargo feature |
| **Build-Only** | 🔨 | Not in final binary |
| **Dev-Only** | 🧪 | Development/testing only |

---

## 1. Rust Workspace Dependencies

### 1.1 Workspace Root (`Cargo.toml`)

| Dependency | Version | Category | Status | Used By | Feature-Gated | Purpose |
|------------|---------|----------|--------|---------|---------------|---------|
| `anyhow` | 1 | 🔴 Core | ✅ | All crates | No | Standard error handling |
| `serde` | 1 (derive) | 🔴 Core | ✅ | All crates | No | Serialization framework |
| `serde_json` | 1 | 🔴 Core | ✅ | All crates | No | JSON serialization |
| `lz4_flex` | 0.11 | 🔴 Core | ✅ | ew-format | No | LZ4 decompression (NSO) |
| `ruzstd` | 0.7 | 🔴 Core | ✅ | ew-format, emblemweave | No | Zstd decompression |
| `sha2` | 0.10 | 🔴 Core | ✅ | ew-format, ew-boot, emblemweave | No | SHA-256 hashing |

### 1.2 Crate-Level Dependencies

#### `ew-format` - Container Formats

| Dependency | Version | Category | Status | Feature-Gated | Purpose |
|------------|---------|----------|--------|---------------|---------|
| `anyhow` | workspace | 🔴 Core | ✅ | No | Error handling |
| `serde` | workspace | 🔴 Core | ✅ | No | Serialization |
| `serde_json` | workspace | 🔴 Core | ✅ | No | JSON |
| `lz4_flex` | workspace | 🔴 Core | ✅ | No | NSO decompression |
| `ruzstd` | workspace | 🔴 Core | ✅ | No | Zstd decompression |
| `sha2` | workspace | 🔴 Core | ✅ | No | Hashing |

**Bins**: `ew-inspect`, `ew-decode`, `ew-nup`

#### `ew-lift` - AArch64→C Recompilation Pipeline

| Dependency | Version | Category | Status | Feature-Gated | Purpose |
|------------|---------|----------|--------|---------------|---------|
| `anyhow` | workspace | 🔴 Core | ✅ | No | Error handling |
| `serde` | workspace | 🔴 Core | ✅ | No | Serialization |
| `serde_json` | workspace | 🔴 Core | ✅ | No | JSON |
| `ew-format` | workspace | 🔴 Core | ✅ | No | Container format access |

**Bins**: `ew-lift`, `ew-tables`, `ew-path`

#### `ew-runtime` - Native Runtime

| Dependency | Version | Category | Status | Feature-Gated | Purpose |
|------------|---------|----------|--------|---------------|---------|
| `anyhow` | workspace | 🔴 Core | ✅ | No | Error handling |
| `serde` | workspace | 🔴 Core | ✅ | No | Serialization |
| `serde_json` | workspace | 🔴 Core | ✅ | No | JSON |
| `ew-format` | workspace | 🔴 Core | ✅ | No | Container format access |

#### `ew-boot` - Whole-Program Dispatcher

| Dependency | Version | Category | Status | Feature-Gated | Purpose |
|------------|---------|----------|--------|---------------|---------|
| `anyhow` | workspace | 🔴 Core | ✅ | No | Error handling |
| `serde` | workspace | 🔴 Core | ✅ | No | Serialization |
| `serde_json` | workspace | 🔴 Core | ✅ | No | JSON |
| `ew-lift` | workspace | 🔵 Recompiler | ✅ | No | Lifter integration |
| `ew-format` | workspace | 🔴 Core | ✅ | No | Container formats |
| `ew-runtime` | workspace | 🔴 Core | ✅ | No | Runtime services |
| `ew-gfx` | workspace | 🟢 Graphics | ✅ | No | Vulkan device layer |
| `libm` | 0.2.11 | 🔴 Core | ✅ | No | Math functions (no_std) |
| `windows-sys` | 0.59 | 🟠 Platform | ✅ | Partial (cfg) | Win32 APIs (15+ modules) |

#### `ew-gfx` - Vulkan Device Layer

| Dependency | Version | Category | Status | Feature-Gated | Purpose |
|------------|---------|----------|--------|---------------|---------|
| `ash` | 0.38 (loaded) | 🟢 Graphics | ✅ | Features: `loaded` | Vulkan bindings |
| `windows-sys` | 0.59 | 🟠 Platform | ✅ | Partial (cfg) | Win32 APIs (4 modules) |

**Bins**: `ew-present`

#### `emblemweave` (app) - Main Launcher

| Dependency | Version | Category | Status | Feature-Gated | Purpose |
|------------|---------|----------|--------|---------------|---------|
| `anyhow` | workspace | 🔴 Core | ✅ | No | Error handling |
| `serde` | workspace | 🔴 Core | ✅ | No | Serialization |
| `serde_json` | workspace | 🔴 Core | ✅ | No | JSON |
| `ew-format` | workspace | 🔴 Core | ✅ | No | Container formats |
| `ew-lift` | workspace | 🔵 Recompiler | ✅ | No | Lifter integration |
| `ew-runtime` | workspace | 🔴 Core | ✅ | No | Runtime services |
| `ew-boot` | workspace | 🔴 Core | ✅ | No | Dispatcher |
| `eframe` | 0.31 | 🔴 Core | ✅ | No | GUI framework (egui) |
| `rfd` | 0.15 | 🔴 Core | ✅ | No | File dialogs |
| `image` | 0.25 (png) | 🟢 Graphics | ✅ | Features: `png` | Icons/screenshots |
| `tar` | 0.4 | 🔴 Core | ✅ | No | Package extraction |
| `zstd` | 0.13 | 🔴 Core | ✅ | No | Package extraction |
| `sha2` | workspace | 🔴 Core | ✅ | No | Hashing |
| `roxmltree` | 0.20 | 🔴 Core | ✅ | No | controls.ini parsing |
| `zip` | 2.4 (deflate) | 🔴 Core | ✅ | No | Package extraction |
| `windows-sys` | 0.59 | 🟠 Platform | ✅ | Partial (cfg) | Win32 APIs (15 modules) |
| `winresource` | 0.1 | 🟡 Build | 🔨 | Build only | Windows resources |

---

## 2. C++ Engine Dependencies

### 2.1 Core Runtime (`runtime/CMakeLists.txt`)

| Dependency | Version | Category | Status | Feature Gate | Source | Purpose |
|------------|---------|----------|--------|--------------|--------|---------|
| `nxrt_zstd` | 1.5.7 | 🔴 Core | ✅ | Always | Vendored (CMake) | Shader pack/RomFS decompression |
| `psapi` | System | 🟠 Platform | ✅ | Windows only | Windows SDK | Process memory info |
| `SDL3` | 3.4.18 | 🔴 Core | ✅ | Always | FetchContent | Window/input |
| `nxrt_volk` | 1.4.363.0 | 🟢 Graphics | ✅ | Always | FetchContent | Vulkan function loader |
| `vulkan_headers` | 1.4.363.0 | 🟢 Graphics | ✅ | Always | FetchContent | Vulkan API |
| `nxrt_vma` | 3.4.0 | 🟢 Graphics | ✅ | Always | FetchContent | Memory allocator |
| `nxrt_imgui` | 1.92.9b | 🟢 Graphics | ✅ | Always | FetchContent | Debug overlay |
| `nxrt_tiling` | - | 🟢 Graphics | ✅ | Always | tools/tiling | Tegra tiling |
| `nxrt_astc2bc7` | - | 🟢 Graphics | 🔧 | `NXRT_ASTC2BC7` | tools/textures | ASTC→BC7 transcoder |

### 2.2 Optional Graphics Dependencies (`runtime/CMakeLists.txt`)

| Dependency | Version | Category | Status | Feature Gate | Source | Purpose |
|------------|---------|----------|--------|--------------|--------|---------|
| `dav1d` | Latest | 🟣 Optional | ⭕ | `NXRT_DAV1D` | Vendored (CMake) | AV1 video decoder |
| `libvpx` | Latest | 🟣 Optional | ⭕ | `NXRT_LIBVPX` | vcpkg | VP9/VP8 video decoder |
| `FFmpeg headers` | Latest | 🟣 Optional | ⭕ | Runtime dlopen | Vendored | libavcodec AV1 backend |
| `amd_ffx_api` | Latest | 🟣 Optional | ⭕ | `NXRT_UPSCALER_FSR` | Header-only | FSR 3 frame gen |
| `amd_ffx_api_fg` | Latest | 🟣 Optional | ⭕ | `NXRT_UPSCALER_FSR` | Header-only | FSR 3 FG API |
| `ffx_fsr1` | Latest | 🟣 Optional | ⭕ | `NXRT_UPSCALER_FSR` | Header-only | FSR 1 upscaler |
| `intel_xess` | Latest | 🟣 Optional | ⭕ | `NXRT_UPSCALER_XESS` | Header-only | XeSS upscaler |
| `nvidia_nis` | Latest | 🟣 Optional | ⭕ | `NXRT_UPSCALER_NIS` | Header-only | NIS upscaler |
| `nvidia_streamline` | Latest | 🟣 Optional | ⭕ | `NXRT_UPSCALER_DLSS` | Header-only | Streamline SDK |
| `renhdrum` | Latest | 🟢 Graphics | ✅ | Always | Header-only | HDR tonemapping |

### 2.3 Recompiler Dependencies (`recomp/`)

| Dependency | Version | Category | Status | Source | Purpose |
|------------|---------|----------|--------|--------|---------|
| Python | 3.9+ | 🟡 Build | ✅ | System | Host for lifter |
| `unicorn` | 2.1.4 | 🔵 Recompiler | ✅ | PyPI | Reference CPU for oracles |
| `capstone` | 5.0.9 | 🔵 Recompiler | ✅ | PyPI | Instruction decode |
| `lz4` | Latest | 🔵 Recompiler | ✅ | PyPI | NSO decompression |
| `pyelftools` | Latest | 🔵 Recompiler | ✅ | PyPI | ELF reading for tests |
| `zbic` | Latest | 🔵 Recompiler | ✅ | PyPI (GitHub) | ZBIC-compressed NSOs |
| `zstandard` | Latest | 🔵 Recompiler | ✅ | PyPI | .zst archives in romfs |
| LLVM/Clang | 23+ | 🟡 Build | ✅ | System | clang-cl + lld-link |
| CMake | 3.25+ | 🟡 Build | ✅ | System | Build configuration |
| Ninja | Latest | 🟡 Build | ✅ | System | Build execution |
| Visual Studio | 2022+ | 🟡 Build | ✅ | System | MSVC toolset + Windows SDK |
| NASM | 2.14+ | 🟡 Build | ⭕ | System | dav1d asm (optional) |
| .NET SDK | 10 | 🟡 Build | 🔧 | System | Shader translator build |

### 2.4 Shader Translator Dependencies (`tools/shaders/`)

| Dependency | Version | Category | Status | Source | Purpose |
|------------|---------|----------|--------|--------|---------|
| Ryujinx.Common | a82350b | 🔵 Recompiler | ✅ | Git (staged) | Common utilities |
| Spv.Generator | a82350b | 🔵 Recompiler | ✅ | Git (staged) | SPIR-V generation |
| Ryujinx.Graphics.Shader | a82350b | 🔵 Recompiler | ✅ | Git (staged) | Shader translation |
| .NET SDK | 10 | 🟡 Build | 🔧 | System | Build translator |
| NuGet packages | Pinned | 🔵 Recompiler | ✅ | nuget.config | .NET dependencies |

---

## 3. Tool Dependencies (Build Scripts)

### 3.1 Runtime Build (`runtime/scripts/build.ps1`)

| Tool | Version | Category | Status | Purpose |
|------|---------|----------|--------|---------|
| `clang-cl` | 23+ | 🟡 Build | ✅ | C/C++ compiler |
| `lld-link` | 23+ | 🟡 Build | ✅ | Linker |
| `cmake` | 3.25+ | 🟡 Build | ✅ | Build configuration |
| `ninja` | Latest | 🟡 Build | ✅ | Build execution |
| `sccache` | Latest | 🟡 Build | ⭕ | Compiler cache (optional) |
| `vcvars64.bat` | VS 2022+ | 🟡 Build | ✅ | MSVC environment |
| `ctest` | CMake | 🟡 Build | ⭕ | Test execution |

### 3.2 Texture Transcoder (`tools/textures/scripts/build.ps1`)

| Tool | Version | Category | Status | Purpose |
|------|---------|----------|--------|---------|
| `clang-cl` | 23+ | 🟡 Build | ✅ | C/C++ compiler |
| `lld-link` | 23+ | 🟡 Build | ✅ | Linker |
| `cmake` | 3.25+ | 🟡 Build | ✅ | Build configuration |
| `ninja` | Latest | 🟡 Build | ✅ | Build execution |
| `ispc` | Latest | 🟡 Build | 🔧 | ISPC compiler (optional) |
| `sccache` | Latest | 🟡 Build | ⭕ | Compiler cache (optional) |

### 3.3 Tiling Library (`tools/tiling/scripts/build.ps1`)

| Tool | Version | Category | Status | Purpose |
|------|---------|----------|--------|---------|
| `clang-cl` | 23+ | 🟡 Build | ✅ | C/C++ compiler |
| `lld-link` | 23+ | 🟡 Build | ✅ | Linker |
| `cmake` | 3.25+ | 🟡 Build | ✅ | Build configuration |
| `ninja` | Latest | 🟡 Build | ✅ | Build execution |
| `sccache` | Latest | 🟡 Build | ⭕ | Compiler cache (optional) |

### 3.4 Shader Translator Setup (`tools/shaders/scripts/setup_ryujinx.ps1`)

| Tool | Version | Category | Status | Purpose |
|------|---------|----------|--------|---------|
| `robocopy` | Windows | 🟡 Build | ✅ | Stage Ryujinx sources |
| `dotnet` | 10 | 🟡 Build | ✅ | Build translator |

### 3.5 Game Build (`recomp/scripts/build_game.cmd`)

| Tool | Version | Category | Status | Purpose |
|------|---------|----------|--------|---------|
| `clang-cl` | 23+ | 🟡 Build | ✅ | C compiler for lifted code |
| `lld-link` | 23+ | 🟡 Build | ✅ | Linker |
| `cmake` | 3.25+ | 🟡 Build | ✅ | Build configuration |
| `ninja` | Latest | 🟡 Build | ✅ | Build execution |
| `sccache` | Latest | 🟡 Build | ⭕ | Compiler cache |
| `vcvars64.bat` | VS 2022+ | 🟡 Build | ✅ | MSVC environment |

### 3.6 Third-Party Fetch (`tools/textures/scripts/fetch_third_party.ps1`)

| Dependency | Version | Category | Status | Source | Purpose |
|------------|---------|----------|--------|--------|---------|
| `bc7enc_rdo` | Latest | 🟢 Graphics | ✅ | GitHub | BC1 encoder (rgbcx) |
| `astcenc` | Latest | 🟢 Graphics | ✅ | GitHub | ASTC decoder DLLs |
| `bc7e` (ISPC) | Latest | 🟢 Graphics | 🔧 | GitHub | BC7 encoder (ISPC) |

---

## 4. Dependency Summary by Category

### 4.1 Count by Category

| Category | Mandatory | Optional | Feature-Gated | Build-Only | Dev-Only | Total |
|----------|-----------|----------|---------------|------------|----------|-------|
| 🔴 **Core** | 12 | 0 | 0 | 0 | 0 | 12 |
| 🟠 **Platform** | 2 | 1 | 3 | 0 | 0 | 3 |
| 🟢 **Graphics** | 6 | 12 | 12 | 0 | 0 | 18 |
| 🔵 **Recompiler** | 7 | 0 | 0 | 0 | 0 | 7 |
| 🟡 **Build** | 8 | 2 | 0 | 10 | 0 | 10 |
| 🟣 **Optional** | 0 | 12 | 12 | 0 | 0 | 12 |
| ⚪ **Dev** | 0 | 0 | 0 | 0 | 2 | 2 |
| **Total** | **35** | **27** | **27** | **10** | **2** | **64** |

### 4.2 Count by Ecosystem

| Ecosystem | Dependencies | Count |
|-----------|--------------|-------|
| **Rust (crates.io)** | anyhow, serde, serde_json, lz4_flex, ruzstd, sha2, eframe, rfd, image, tar, zstd, roxmltree, zip, windows-sys, ash, libm, winresource | 17 |
| **C++ (FetchContent)** | SDL3, Vulkan-Headers, volk, VMA, imgui, dav1d, zstd | 7 |
| **C++ (Header-only)** | amd_ffx_api, amd_ffx_api_fg, ffx_fsr1, intel_xess, nvidia_nis, nvidia_streamline, renhdrum | 7 |
| **C++ (vcpkg)** | libvpx | 1 |
| **C++ (Tools)** | tegra_tiling, astc2bc7, bc7e_ispc, rgbcx | 4 |
| **Python (PyPI)** | unicorn, capstone, lz4, pyelftools, zbic, zstandard | 6 |
| **System Tools** | LLVM/Clang, CMake, Ninja, VS, NASM, .NET SDK | 7 |
| **.NET (NuGet)** | Ryujinx.Common, Spv.Generator, Ryujinx.Graphics.Shader | 3 |

---

## 5. Issues & Recommendations

### 5.1 Critical Issues (Block Phase B)

| Issue | Location | Impact | Recommendation |
|-------|----------|--------|----------------|
| No toolchain lockfile (Phase A; superseded in Phase B) | All builds | No reproducibility, no cache invalidation | Implement `tools.lock` with SHA-256 (LongshotLauncher pattern) |
| .NET SDK required when unbundled | Shader translator | 200+ MB install, version conflicts | Bundle .NET NativeAOT translator (dkb-pc-recomp) |
| No structured build protocol (Phase A; superseded in Phase B) | `engine.rs` | Fragile log parsing, no progress API | Adopt `MakeGameProtocol` (LongshotLauncher) |
| No compiler memory limit | `build.ps1`, `build_game.cmd` | OOM on low-RAM machines | Add `HOOKSHOT_MEM_LIMIT_GB` equivalent |

### 5.2 High Priority Issues

| Issue | Location | Impact | Recommendation |
|-------|----------|--------|----------------|
| No object/lift stores | `engine.rs`, `jobs.rs` | Full recompile on any change | Implement `LONGSHOT_OBJ_STORE`/`LONGSHOT_LIFT_STORE` |
| Single build directory | `engine.rs` | No rollback, no parallel builds | Track/version build instances |
| Monolithic pipeline cache | `engine.rs`, `nxrt` | Full rebuild on translator change | Intelligent cache migration (LongshotLauncher) |
| No hard-link optimization | `jobs.rs`, `gamedata.rs` | Slow asset copying | Hard-link RomFS where possible (Hookshot) |

### 5.3 Medium Priority Issues

| Issue | Location | Impact | Recommendation |
|-------|----------|--------|----------------|
| `ruzstd` duplicated | `Cargo.toml`, `emblemweave/Cargo.toml` | Slightly larger lockfile | Move to workspace only |
| `windows-sys` features not minimized | `ew-boot`, `ew-gfx`, `emblemweave` | Larger compile times | Per-crate feature subsets |
| NASM not feature-gated in CMake | `runtime/CMakeLists.txt` | Forced dependency | Add `NXRT_NASM` option |
| Git listed but not needed | `deps.rs` | Confusion | Remove or mark as dev-only |

### 5.4 Low Priority Issues

| Issue | Location | Impact | Recommendation |
|-------|----------|--------|----------------|
| uv not used in CI | `.github/workflows/` | Slower Rust builds | Add uv to CI |
| No `VK_EXT_graphics_pipeline_library` | `runtime/src/gfx/` | Slower pipeline linking | Adopt extension (dkb-pc-recomp) |
| Write-watch not integrated in canonical main | `runtime/src/gfx/` | Full texture uploads | FCC's uncommitted platform split has replacements; integrate after handoff |

---

## 6. Feature Gate Mapping

### 6.1 CMake Options (`runtime/CMakeLists.txt`)

| Option | Default | Dependencies Affected | Category |
|--------|---------|----------------------|----------|
| `NXRT_UPSCALER_FSR` | ON | amd_ffx_api, amd_ffx_api_fg, ffx_fsr1 | 🟣 Optional |
| `NXRT_UPSCALER_DLSS` | ON | nvidia_streamline | 🟣 Optional |
| `NXRT_UPSCALER_XESS` | ON | intel_xess | 🟣 Optional |
| `NXRT_UPSCALER_NIS` | ON | nvidia_nis | 🟣 Optional |
| `NXRT_UPSCALER_MAKO` | ON | mako_spatial.cpp | 🟣 Optional |
| `NXRT_DAV1D` | ON | dav1d | 🟣 Optional |
| `NXRT_LIBVPX` | ON | libvpx (vcpkg) | 🟣 Optional |
| `NXRT_ASTC2BC7` | ON | astc2bc7, bc7e_ispc | 🟢 Graphics |
| `NXRT_UNITY_BUILD` | OFF | All targets | 🟡 Build |
| `NXRT_PCH` | ON | nxrt, nxrt_gfx | 🟡 Build |
| `NXRT_BUILD_TESTS` | ON | Test executables | 🧪 Dev |
| `NXRT_BUILD_LIFT_TESTS` | ON | Lift integration tests | 🧪 Dev |
| `NXRT_BUILD_GFX` | ON | nxrt_gfx | 🟢 Graphics |
| `NXRT_BUILD_GAMES` | ON | games/fefw | 🔴 Core |
| `NXRT_PLATFORM_ONLY` | OFF (WIN32) | Platform layer only | 🟠 Platform |

### 6.2 Cargo Features

| Crate | Features | Status |
|-------|----------|--------|
| `ash` | `loaded` | Minimal - good |
| `image` | `png` | Minimal - good |
| `zip` | `deflate-flate2`, `flate2` | Minimal - good |
| `windows-sys` | 15+ modules (app), 4 (ew-gfx), 5 (ew-boot) | **Needs minimization** |
| `ew-format` | None (bins only) | Good |
| `ew-lift` | None (bins only) | Good |
| `ew-boot` | None | Good |
| `ew-gfx` | None (bins only) | Good |
| `emblemweave` | None | Good |

---

## 7. Version Pinning Status

| Dependency | Pinned? | Mechanism | Location |
|------------|---------|-----------|----------|
| Rust workspace deps | ✅ | `Cargo.lock` | Root |
| SDL3 | ✅ | FetchContent URL_HASH | `gfx_deps.cmake` |
| Vulkan-Headers | ✅ | FetchContent URL_HASH | `gfx_deps.cmake` |
| volk | ✅ | FetchContent URL_HASH | `gfx_deps.cmake` |
| VMA | ✅ | FetchContent URL_HASH | `gfx_deps.cmake` |
| imgui | ✅ | FetchContent URL_HASH | `gfx_deps.cmake` |
| dav1d | ✅ | Vendored CMake | `cmake/media_deps.cmake` |
| Ryujinx translator | ✅ | Git commit | `engine.rs:29` |
| Python packages | ✅ | `requirements.txt` with `==` | `recomp/requirements.txt` |
| Toolchain (clang, cmake, ninja) | ❌ | Environment detection | `nx_env.cmd` |
| .NET SDK | ❌ | Version check only | `engine.rs:720` |
| NASM | ❌ | Version check only | `deps.rs` |
| vcpkg packages | ❌ | Not used (libvpx only) | - |

---

## 8. Action Items for Phase B

### Immediate (Week 1)
1. [x] Create `tools.lock` schema with SHA-256 for all toolchain components
2. [x] Implement structured build protocol in `engine.rs` (replace log parsing)
3. [x] Add `NX_JOBS` memory-aware calculation to `build_game.cmd` (port from `build.ps1`)
4. [ ] Move `ruzstd` to workspace-only dependency (still pending)

### Short-term (Week 2-3)
5. [ ] Investigate .NET NativeAOT for shader translator
6. [ ] Implement object/lift stores for incremental recompilation
7. [ ] Add track/version build directory structure
8. [ ] Implement intelligent pipeline cache migration

### Medium-term (Week 4-6)
9. [ ] XDG-compliant storage separation
10. [ ] Portable mode via `EMBLEMWEAVE_HOME` env var
11. [ ] "Free up space" UI implementation
12. [ ] Hard-link RomFS optimization

### Phase B Implementation Status (code-level; not release validation)

**Implemented in the working tree (not independently validated here):**
- ✅ `tools.lock` manifest with 12 tool specifications (engine/fefw-recomp/tools.lock)
- ✅ Unified ToolchainManager in Rust (apps/emblemweave/src/toolchain_manager.rs)
- ✅ Structured build protocol (JSON Lines) via BuildEvents.psm1 and build.ps1
- ✅ Adaptive memory-aware concurrency (ConcurrencyCalculator in toolchain_manager.rs, Get-AdaptiveJobCount in build.ps1)
- ✅ sccache configuration preserved and integrated
- ✅ Dependency inventory reconciled with Phase A usage; this is not a build or test verification
- ✅ nx_env.cmd and build_game.cmd updated for structured events
- ✅ engine.rs fully integrated with ToolchainManager and event emission

**Validation still pending:**
- [ ] Regression tests (clean/incremental/fresh/partial toolchain scenarios)
- [ ] Performance measurements (discovery, provisioning, startup, clean/incremental build, memory, cache hit rate)
- [ ] Independent review of the Phase B documentation and implementation

---

*Generated: 2026-11-20 | Phase A snapshot | Phase B implementation present; validation pending (reviewed 2026-11-24)*