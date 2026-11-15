# Where the engine comes from

`engine/fefw-recomp` is not EmblemWeave's own code. It is **fefw-recomp**, the
static recompilation of *Fire Emblem: Fortune's Weave* by
[Goldoire](https://github.com/Goldoire/fefw-recomp) and its contributors, GPL-3.0,
used here as the baseline engine that the launcher sets up, builds and runs.
EmblemWeave is the launcher around it: game data handling, the recompilation
pipeline, mods, saves, cheats, the in-game menu and the interface.

## The handover

In November 2026 the author of fefw-recomp said they would not continue the
project and removed the repository from GitHub. **Obersteinn** had saved the last
release, `fefw-recomp-0.2.0-win64.zip`, with its build tools and documentation,
and shared it so the work would not be lost
([archive](https://drive.google.com/file/d/1U8_QkcL7yxbW8eUTwGywHsynLCOCshHu/view)).
That archive is the only known copy of the 0.2.0 package.

## What this repository keeps

- **The engine** in `engine/fefw-recomp`: the last upstream commit taken in
  (`501f7c0`, 9 November 2026) plus EmblemWeave's own changes, such as serving
  zstd-packed `.nup` entries from a decompressed cache. It is committed here as
  ordinary files rather than as a submodule, since a submodule would point at a
  repository that no longer exists: a fresh clone is complete and needs no fetch
  step, and the vendored tree keeps its own `.gitignore` so game data and build
  output stay out. The copy carries the files, not the upstream commit history,
  which went with the repository; `git log` of EmblemWeave records when and why
  it was brought in.
- **`apps/emblemweave/data/dump_hashes.json`**, copied unchanged from the 0.2.0
  package's `tools/build/dump_hashes.json`. Game Setup hashes the user's ExeFS
  modules against it and reports whether they are the build the engine was tested
  with - update 131072, `main` build `db93b0125f48…` - or a different one.
- **Its build notes.** The 0.2.0 first-run build (verify the dump against the
  manifest, write an uncompressed copy of the ZBIC `main`, lift and compile with
  a resumable stamp) is the reference the launcher's own pipeline and its
  Build the game checks follow.

## What it does not ship

The packaged `fefw-launcher.exe`, `fefw_rt.dll`, `renhdrum.dll`, the bundled
Python and Zig runtimes and the translation files stay out of this repository.
EmblemWeave has its own launcher and its own runtime; nothing from the package
is redistributed here, and the archive itself contains no game data.

## Credit

- **Goldoire** and everyone who contributed to fefw-recomp: the engine, its
  runtime, its build tooling, and the research behind them. The baseline this
  project stands on.
- **Obersteinn**: preserving the final 0.2.0 release and passing it on, which
  kept the reference build, its build scripts and its tested-dump manifest
  available after the original repository was removed.

Both are credited in the [README](../README.md#credits) and on the launcher's
About page.

## The 0.2.0 package, audited

The archive was compared against this tree file by file. Every build-tool file it carries is present here already: its
`tools/build/recomp` (41 files) is a subset of this repository's `recomp/` (135), its `sdk/` pieces (`fefw.ico`,
`fefw.manifest`, `fefw.rc`, `fefw_rt_api.h`, `fefw_stub.cpp`) live in `game-build/fefw`, `dump_hashes.json` is in
`apps/emblemweave/data`, and the runtime font it ships (`fonts/DejaVuSans.ttf`) is already committed at
`runtime/assets/fonts/`. `fefw_fma.c` and `fefw_rt.lib` are products of its release-only `FEFW_RT_DLL` build, which this
project does not use. Nothing from the package is missing here.

One delta is worth recording: the package's `recomp/games/fefw/main.sites` is the only file whose contents differ from
this repository's, and its three extra lines point at two features the runtime does not implement yet, a real-time map
cursor step (`cursor_real_time`) and a stat radar chart. Copying the sites without the hooks would do nothing, so they
wait for whoever ports those two features.

## The FE3H Recomp, inspected

A built install of *FE3H Recomp* v0.1.6 was inspected: an MIT-licensed static recompilation of
*Fire Emblem: Three Houses* built on [LAS Recomp](https://github.com/peiikai/LASRecomp) by Peiikai (also MIT), with the
Three Houses changes by onerag90. It is not a source tree: the folder holds
`fe3hrecomp.exe`, its DLLs, `cache/`, `extracted/`, a bundled Zig, a PyInstaller `lasgen`, and the only sources are six
files in `gamelib-src/` (`cpu.h`, `gamelib.h`, `simd.h`, `simd_fp.h`, `gamelib_entry.c`, an empty
`hle_symbols.txt`). Nothing from it is redistributed here.

What was taken: its Sharpening setting, four steps off / low / default / high. Our engine has had `[upscale] sharpness`
all along, applied live, but only the game's F1 menu could reach it; **Graphics > Sharpening** now writes it before
every start and adopts a value changed in the menu, the same way the upscaler is handled.

What was checked and already matches: `runtime/src/services/audren` carries the reverb commands, edge matrix and 3D
delay-line reverb their port had to add; shader stages are built on workers, linked from ready parts and only waited
for when nothing is ready, with the on-disk cache the play folder keeps; upscaling (FSR 1.0, NIS, FSR 3.1, DLSS/DLAA),
render scale, DLC discovery, movies and Atmosphere cheat codes are all present in the engine.

Two of its notes are worth keeping, and neither is a file to copy:

- `simd.h` lowers AArch64 FP/AdvSIMD to portable C with hardware-exact semantics (first signalling NaN quietened then
  the first quiet NaN, `FPCR.DN` default NaN, `FPCR.FZ` flushing, `FPCR.RMode` for arithmetic and conversions, the ARM
  estimate tables), and says plainly what it does not model (FPSR cumulative bits and QC, FP16 arithmetic). It prompted
  the same work here: `ew-lift` now applies `FPCR.RMode`, `FPCR.FZ` and `FPCR.DN` to every scalar operation in both the
  interpreter and the emitted C, so a function that changes a mode no longer diverges from the console. Vector
  operations (`simd.rs`) and FPSR's cumulative flags are still outside that, and the engine's own C++ recompiler keeps
  the equivalent gap recorded in `recomp/docs/GAPS.md`.
- Cheats that patch the main module's *code*. Their generator compiles the patched instructions beside the originals
  and picks at run time, because a recompiled game no longer runs the bytes a cheat rewrites. Our `cheats.rs` states
  the limitation instead. It is the one feature in their port that the cheats page cannot do.

## The Engage PC Recomp, inspected

A second install was inspected, *Engage PC Recomp* 0.1.0-alpha.6 at `5e0ce98f120bd669cb79d3b365d0da1d6d54145b`, MIT
(Valiantsin Dzerakh), built on TotK Recomp (also MIT). Unlike the FE3H install this one ships its sources: a Python
recompiler (`recomp/`, 4,951 lines), a C runtime (`runtime/`, 152 files), the setup and launcher GUIs, the tools, and a
bundled Python, LLVM and Ninja. Nothing from it is redistributed here; it
is MIT, which the GPL recompiler may take from the way it already takes from smo-recomp and SuperMarioWonderRecomp.

Three mechanisms are worth knowing about:

- **Patches with a run-time switch.** `recomp/patches.py` reads a patch table whose lines are `set <name> <env>
  <default>`, `build <module> <hash>`, `site <set> <module> <addr> <orig> <new>...` and `spin ...`. A patched site is
  lifted twice and the generated C runs `if (rt_patch_on[set]) { replacement } else { original }`, so a code patch is
  switched from the menu instead of being fixed at generation time. Runtime-owned values live in a reserved 4 KiB page
  after the module image (`@page:x<n>` encodes `adrp` to it). Our `load_sites` has `patch <addr> <orig> <new>`, which is
  compile-time only, and is the only feature in it this project cannot express.
- **Spin-wait detection.** `tools/spin_waits.py` (adapted from nxnative, MIT, Copyright (c) 2026 seedds) proves a loop
  cannot leave by itself - only loads, register and flag arithmetic, direct branches and barriers on every path back,
  nothing live at the head written - and names its back-edges. Those are lifted as `rt_spin_gate(c, head); goto head`,
  which slows the poll and hands the virtual core to a waiting thread. Our kernel already blocks in
  `kernel/busy_mutex.cpp`, but there is no static detector to find loops that burn a core without one.
- **Static jump-table resolution.** `recomp/jumptables.py` (1,345 lines) walks back from the `br` over its block and
  predecessor blocks, evaluates paths symbolically, and iterates tables and function-wide facts to a common fixpoint; it
  handles a dispatch block placed elsewhere (`b.ls`), `ccmp` chains and table bases hoisted out of loops. Our
  `nxrecomp/jumptables.py` (287 lines, adapted from SuperMarioWonderRecomp) covers clang's compressed tables and
  table-relative `ldrsw` tables, and falls back to a dispatch over the function otherwise. That fallback is why
  [STATUS.md](STATUS.md) still calls jump tables partial.

What the engine already has that they also needed: the same site kinds minus the run-time switch (`patch`, `inline`,
`midhook`, `entryhook`, `dynimm`, `noinline`, `jtable`), C++ exception lifting, a config file, an audio mixer with
reverb, ASTC decoding, FSR shaders and an extractor that unpacks `.nsp`.

One experiment did not transfer: their `tools/spin_waits.py` run against this title's ExeFS stops in `recomp/nso.py` with an
LZ4 block error on the first module it opens, so their readers cover their module set rather than this title's. The tools are
Engage-specific; the three designs above are not.
