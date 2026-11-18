# Engine and launcher audit — 18 November 2026

This is a source-level audit of EmblemWeave at `755c29f` plus the local
changes described at the end. It is deliberately not a feature wish list: it
records what already exists, what was measured or observed, what was changed,
and what is blocked on data, hardware or a profile.

## A. Current architecture

| Layer | Implementation | State |
| --- | --- | --- |
| Launcher | Rust + egui (`apps/emblemweave`) | Builds, detects game data, writes `fefw.ini`, drives the engine build, owns saves/mods/diagnostics. |
| Recompiler | Python lifter under `engine/fefw-recomp/recomp` + `tools` | Lifts the six game modules to C++; the launcher runs it through the pinned `tools/.venv`. |
| Game executable | C++20, clang-cl, Ninja (`game-build/fefw`) | `fefw.exe` links the recompiled modules with the runtime. |
| Runtime | C++20 + Vulkan (`runtime`) | HLE services, NVN front end, Vulkan backend, audio, movies, diagnostics. |
| Rust research runtime | `crates/ew-boot`, `crates/ew-gfx` | Independent research path; draws are not rendered. It is not the primary engine and its DLC support is still stubbed. |
| Shader path | Prebuilt SPIR-V harness + per-game shader cache | Translates the game's shader archives; missing shaders are a hard build failure, not a silent black screen. |

The primary engine is `fefw-recomp`. Nothing here replaces it with the Rust
research runtime, a different renderer or an emulator.

### Already present (do not duplicate)

- Vulkan 1.3/1.4 device setup, dynamic rendering, synchronization2, timeline
  semaphores, descriptor indexing/buffer device address where the device
  reports them, pipeline caches, graphics pipeline libraries and background
  pipeline work.
- FSR 1, FSR 3.1, DLSS/DLAA, XeSS, NVIDIA Image Scaling, frame generation
  (DLSS-G / FSR 3), Reflex and AMD Anti-Lag 2.
- HDR output, wide gamut, tone mapping, grading, bloom, sharpen, motion blur,
  depth of field, vignette, dithering, and the merged HDR texture promotion.
- Frame limiter, VRR-aware caps, present modes, low-latency modes, frames in
  flight and the render-thread/scene split.
- Texture caches (SPIR-V, BC7), shader-compile thread/priority/eco controls,
  pool mirrors, shader prewarm, the F1 diagnostics overlay and the launcher's
  report bundle.
- DLC in the C++ engine: `[aoc] dir`, `aoc:u`, and
  `OpenDataStorageByDataId` serve `<title-id>/romfs` packs.

The audit found no justification for a second renderer, a mandatory Vulkan
layer, or a wholesale replacement of the allocator/loader.

## B. 1.1.1 and DLC findings

### 1.1.1

The checked tree presented as 1.1.1 has the **same supported executable** as
1.1.0: `main` build ID `db93b0125f4850601d61c3edde012f8eb2ecfe77`, and the same
seven ExeFS module hashes recorded in `game-build/release/dump_hashes.json`.
There is therefore no new build ID to map and no recompiler change to make for
that tree. What remains is data/save compatibility: a full build of that tree,
a 1.1.1 save load, and a 1.1.0 baseline. The status stays **unverified** until
that run exists; a successful boot alone is not enough.

No distinct 1.1.1 executable or save was present in the audited folders.

### DLC

The C++ engine already implements add-on content. The launcher did not:

- `GameData` had no DLC field.
- `fefw.ini` had no `[aoc]` section, so a dumped `dlc/` folder was never
  pointed at the engine.
- Setup and Overview did not show whether packs were found.

This pass adds folder DLC support: a `dlc/` folder beside the ExeFS, RomFS or
game root, or an explicit path, is scanned for 16-hex title-id folders holding
`romfs/` (or one numbered folder down). The ids are shown in Setup/Overview and
the folder is written as `[aoc] dir` for the build. An empty value is written
when there is no DLC, so the engine does not scan a default folder.

This pass also unpacks add-on packages into a managed `dlc/` folder beside the
copied ExeFS. `.nup` add-on packages and the data NCAs of `.nsp`/`.xci` are
filtered to this title's add-on title-id block and written as
`<title id>/romfs`. A cleanup guard removes the managed folder on cancellation,
a bad package or a write error, so a later detection cannot serve a partial or
stale pack; a completion marker reuses a finished unpack. The `.nup` path skips
an add-on whose RomFS index is invalid instead of failing the whole extraction,
and both paths reject NCA paths with a drive prefix or any non-normal component.
Synthetic `.nup`, `.nsp` and `.xci` detection tests cover the real
`detect_with` wiring, the title filter, progress, cancel, reuse and the
stale-folder failure case; the `.xci` test proves the secure-partition NCAs and
the managed add-on marker through detection. A focused failure test also starts
with a complete old pack and an encrypted add-on NCA, and proves the managed
folder is removed when the add-on scan fails. The `ew-format` tests cover the
`.nsp`/`.xci` NCA RomFS copy, including the secure partition of an XCI.
The program and control lookups scan for their own NCA kind instead of
parsing every NCA first, so an encrypted add-on leaves the base game and its
RomFS detectable; the add-on scan still fails, which is what clears the old
managed folder.

No real DLC payload was present in the audited data, so no DLC build/run was
possible. The feature is covered by scanner and synthetic-package tests and an
ini test, not by a real add-on boot.

### RomFS audio (`.elf.zst`) buffer lifetime

The `.elf.zst` extension is not special to the loader. The game opens it through
the same RomFS path as any asset: `services/fs_romfs.cpp` `RomfsImage::read` →
`Impl::read_host`. That function selects one of three sources:

- a packed `.nup` entry: `Impl::packed_content` → `nup::read_file`
  (`loader/nup.cpp`), which allocates a `src` vector, decompresses into `out`,
  frees `src` at return, and returns `out` to the caller;
- an `.nx` file: `nx::Reader::read` → `Reader::Impl::block_bytes`, which keeps a
  256-block LRU of decoded archive blocks (`kCacheBlocks`, about 4 MiB at
  16 KiB per block);
- an extracted `.nsp`/`.xci`/folder file: `Impl::acquire` (16 open handles,
  `kOpenFiles`) and a positioned `ReadFile`; no file-content cache.

A decoded **outer** `.nup` entry is retained rather than freed per read when it
fits the cache. `packed_content` moves it into a `shared_ptr` and stores it in
`PackedCache`, keyed by file index, under `kPackedBudget = 1536 MiB`. The cache evicts the least recently used entry when
adding the next one would exceed the budget, so one large file no longer clears
every cached entry. Entries larger than `kPackedEntryCap = 256 MiB` are served
for the current read but not retained; this keeps a packed `.elf.zst` from
pinning the whole budget. Before decoding an entry that will not fit, the cache
drops its retained references, so old buffers are not held alongside its output.
For such an entry the game still gets the decoded `.zst` file, not its PCM/ELF,
and a later read of a differently positioned range will decode the frame again
rather than find it in memory. For an entry within the cap, the retained total
stays at or below 1.5 GiB; during the read that adds it, the transient total is
at most the retained total plus one 256 MiB entry.
`RomfsImage::Stats` reports the retained and peak bytes, entry count, evictions
and skips. `packed_cache_evicts_lru_and_skips_oversize_entries` tests the policy
with tiny limits, and `romfs_packed_elf_zst_cache_peak` reads a packed
`audio.elf.zst` through `RomfsImage` and checks the peak accounting. These are
cache-accounting measurements; no real `.elf.zst` package was available for a
process-RSS measurement.

The game then decompresses the inner `.elf.zst` frame through
`nx_hle_zstd_multiframe` (`game-build/fefw/src/fefw_hle.cpp`). It calls
`ZSTD_decompressDCtx` directly into the guest destination (`c->x[1]`, capacity
`c->x[2]`); there is no host-side output allocation, cache or reference. The
`ZSTD_DCtx` is `thread_local` and persists for the thread, retaining zstd's
internal window/tables (bounded by the checked `2^27 + 1` window) but not the
previous output. A native error falls back to the lifted original. Whether the
guest frees its decoded audio buffer is title-side behaviour; this runtime does
not retain it.

So the decoded audio **file** is not necessarily freed per read: a small packed
`.nup` entry is cached within the 1.5 GiB budget, a large one is decoded for the
current read and dropped, and `.nx` blocks are cached within 4 MiB. The only
growth path found beyond the previously audited `.nx` block cache is
`packed_cache`; `nx::Reader::Impl::spans` also keeps one small span vector per
file (metadata, not decoded audio) without a byte budget. No unbounded per-read
decoded-PCM growth was found.

## C. External projects

| Project | License | Relevant work | Decision |
| --- | --- | --- | --- |
| [MAKO](https://github.com/eugeniosegala/MAKO) | GPL-3.0-or-later | Linux Vulkan layer, LSFG frame generation (user-supplied Lossless Scaling), LS1/MAKO scaling, vkBasalt effects, adaptive/display-aware pacing, profiles, experimental HDR. | **Study only.** Windows/Linux layer architecture does not fit this native Vulkan runtime; its pacing ideas are already represented by the frame limiter, VRR cap and `framegen_pace`. Do not ship a second layer. |
| [Vulkan Memory Allocator](https://github.com/GPUOpen-LibrariesAndSDKs/VulkanMemoryAllocator) | MIT | Suballocation, budgets, defragmentation. | Evaluate only with a memory profile; the runtime already has pool mirrors and a budget path. |
| [Volk](https://github.com/zeux/volk) | MIT | Meta-loader for Vulkan entry points. | Optional loader cleanup; no measured frame-time benefit. |
| [SPIRV-Tools](https://github.com/KhronosGroup/SPIRV-Tools) / glslang | Apache-2.0 / BSD | SPIR-V validation and optimization. | Useful in the build/validation path, not the frame loop. |
| [RenderDoc](https://github.com/baldurk/renderdoc) / [Tracy](https://github.com/wolfpld/tracy) | MIT / BSD | Frame and CPU capture. | Developer tools, not runtime dependencies. The engine already exports logs and timings. |
| [FidelityFX SDK](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK) | MIT | FSR, frame generation, denoisers. | FSR paths already integrated; upgrade only with a measured comparison. |
| [XeSS](https://github.com/intel/xess) / [Streamline](https://github.com/NVIDIA-RTX/Streamline) | MIT / NVIDIA | XeSS and DLSS/Reflex/frame generation. | Already integrated as optional SDK paths. |
| [ReShade](https://github.com/crosire/reshade) | BSD-3-Clause | Optional post-processing layer. | Already integrated as a user install. |
| DXVK / VKD3D-Proton / Wine / Proton | Zlib / LGPL / LGPL | D3D translation and compatibility. | Not applicable: the engine is native Vulkan, not D3D. |
| Filament / Granite / The Forge / Godot / vkQuake | Apache-2.0 / MIT / proprietary / MIT / GPL | Scene/resource/pipeline architecture. | Reference only; no source copied. |

MAKO's GPL-3.0-or-later licence would permit code reuse in this GPL-3.0
engine, but its layer targets Linux swapchains and a different frame-generation
model. Copying it would add a dependency and a second owner of presentation,
not remove a measured bottleneck.

## D. Bottleneck report

The repository's own render-track measurements (`docs/PERFORMANCE.md`,
`PERF_NOTES.md`) put the cost in the CPU replay thread before the GPU is the
limit, especially in the Dagsion hub/crowds:

- descriptor-set construction and binding resolution per draw;
- uniform/storage range validation and copies;
- per-draw texture preparation and feedback checks;
- pipeline-layout lookup and per-program invariants;
- shader compilation and pipeline creation during first visits;
- storage-write tracking and alpha/HDR layout selection.

The earlier local changes addressed the last two invariants: `writes_storage`
is computed once per `ProgLayout`, and alpha/HDR/post layout variants use a
64-entry thread-local cache. These remove work from every draw; they do not
change the rendered result. No in-game FPS number is claimed because this
workspace has no runnable save/scene capture for a before/after measurement.

Dagsion was not re-profiled in this pass. Attributing its drops to the GPU,
resolution or a specific game function would be speculation without a capture.
The safe next step is a RenderDoc/engine-stats capture of the same route before
changing culling, streaming or draw batching.

## E. Prioritized roadmap

### Tier 1 — low risk, evidence already exists

1. **DLC folder wiring** (done here): scan, `[aoc] dir`, Setup/Overview.
2. **Per-program derived state** (done in the local engine changes):
   `ProgLayout::writes_storage`, `alpha_layout_cached`.
3. **Build correctness**: host `/EHsc` so `ipc.cpp`/`crash.cpp` compile; the
   full runtime build and EH/lift tests pass.

### Tier 2 — needs a capture or a real asset

4. **Add-on extraction** (done here): unpack filtered `.nup` packages and
   `.nsp`/`.xci` data NCAs with progress/cancel, a cleanup guard and a reuse
   marker. A real add-on boot still needs a DLC package.
5. **1.1.1 data/save validation**: full build of the 1.1.1 tree, one 1.1.1
   save, one 1.1.0 baseline. No executable change expected.
6. **Dagsion capture and attribution**: CPU render-thread time vs GPU busy,
   descriptor/build_set share, texture streaming stalls, shader/pipeline
   events. Only then choose culling, batching or streaming work.
7. **Texture residency/streaming budget**: use the existing cache and memory
   lines; do not force BC or lower quality by default.

### Tier 3 / experimental

8. **GPU-driven visibility or instancing**: only if the capture shows
   draw-submission or geometry cost that the NVN command stream can preserve.
9. **MAKO-style adaptive pacing on Windows**: the engine already has the
   limiter and framegen pacing; a new implementation needs a measured latency
   benefit and a display matrix.
10. **Native Linux build**: separate port (window/input/audio/presentation),
    not a packaging change and not replaced by Proton.

### Not recommended

- Replacing fefw-recomp with the Rust research runtime.
- A mandatory MAKO/vkBasalt/OptiScaler layer on Windows.
- Mesh shaders, ray tracing or forced BC7 without a measured quality/perf
  case.
- Skipping objects, animations or effects to make a benchmark look better.

## F. Benchmark plan

Baseline and each candidate must use the same dump, save, route, driver,
resolution, upscaler and shader-cache state.

1. Main menu, 60 s idle.
2. Hub walk, 120 s.
3. Dagsion crowd, 120 s, same camera path.
4. Character-heavy combat map, 120 s.
5. Cutscene, full clip.
6. Area transition and first-visit shader compilation.
7. 30-minute soak.

Record average FPS, 1% and 0.1% lows, frame-time histogram, CPU/render-thread
time, GPU busy, VRAM/system working set, pipeline/shader events, queue waits and
texture uploads. Use RenderDoc for a single-frame capture and the engine's
`replay_stats`/F1 page for per-frame counts. A change is kept only if the
before/after distributions separate beyond run-to-run noise; one average FPS
number is not evidence.

## G. Tooling

- **REA 6.7.0** at `X:/REA` can drive Ghidra 12.1.x. Its provider check rejects
  the installed Ghidra 12.2 (`unsupported_version`) and Hopper/IDA/Kuna are not
  configured here. A REA inspection of the 161 MB `fefw.exe` was started with
  Ghidra 12.1; it is not fast enough to treat as an interactive step.
- **Ghidra 12.2** is at `D:/ghidra_12.2_PUBLIC`; direct headless analysis of the
  same 161 MB binary is a batch job, not a frame-profiling substitute.
- Binary analysis can answer “what does this function/import do”; it cannot
  replace the runtime capture needed to rank Dagsion costs. No optimization in
  this pass claims a Ghidra-derived speedup.

## H. Changes in this pass

- Launcher DLC discovery and ini/UI wiring.
- Audit and roadmap (this file).
- No renderer, shader, timing or gameplay change.
- The earlier engine changes (per-layout caches and `/EHsc`) remain
  uncommitted; the launcher builds and tests with them present.
