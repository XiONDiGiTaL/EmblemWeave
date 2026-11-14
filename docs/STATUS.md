# EmblemWeave status

**Summary: the game is playable through the engine.** EmblemWeave builds and
runs the game with [fefw-recomp](https://github.com/Goldoire/fefw-recomp)
(GPL-3.0, at `engine/fefw-recomp`): on this PC (AMD Radeon RX 7900 XT) the
retail game, update 131072, runs at about 60 fps with sound, from the
launcher's **Build the game** and **Play**. The rest of this page covers
EmblemWeave's own Rust recompiler and runtime, a research path that now
runs the game's frame loop past the autosave notice and the opening movie
into its first map load. Its command buffers now carry out the clears and
texel copies that write render targets and swap textures, and the launcher's
runtime presents the presented swap texture in a window of its own; the
game's draws are still counted rather than rendered, so the window does not
yet show the game's picture.

A 25-minute headless run reaches 27.9 billion instructions and then stops
where the game aborts: its pool of sound handles fills up, because the host
audio renderer never reports a voice's wave buffers as played, so no handle is
ever released. That is the next blocker.

Last verified: 9 November 2026, retail `main` module of version 1.1.0, release build. The
state of the code itself (native dispatch behind `--native`, the window that presents
frames) was re-read on 13 November 2026 without a run; every measurement here is still
the 9-10 November one.

This page is the summary: it records what was measured on the game data.
[docs/ENGINE.md](ENGINE.md) is the engine's own plan and is kept current with the code.
Where the two disagree, this page carries the measurement and ENGINE.md carries the
detail. The row about native execution below had gone stale and is corrected here:
native code runs, as an option, and has since `ew-native` was added.

## How to read the states

| State | Meaning |
| --- | --- |
| **Verified** | Implemented, tested, and checked against an independent implementation or the real data |
| **Implemented** | Implemented and unit tested; exercised by the boot path |
| **Partial** | Works for the cases listed; the rest is reported, never guessed |
| **Saved only** | The launcher stores the settings; nothing consumes them yet |
| **Blocked** | Not implemented; a run that reaches it stops with its name |

## Compatibility matrix

### Recompiler

| Area | State | Evidence |
| --- | --- | --- |
| NSO0 parsing, LZ4 segments | Verified | segment boundaries land on end of file for every LZ4 module |
| `.nup` game containers | Verified | table of contents and every file checked against SHA-256; the ExeFS copied out byte-identical to an extracted dump; the RomFS served in place to `nn::fs` (the same 122 opens and stop as from a folder) |
| `.nsp` game packages | Verified | plaintext NCAs read and unpacked by the recompiler itself: the real 29.2 GB package yields the same 7 ExeFS files and the same 252,686 RomFS files as the `.nup` of the same build, and `main` is the same size in both |
| ZBIC segments (`main`) | Implemented (bridge) | `tools/zbic_extract.py`; decoded sizes match the header |
| Conditional select, `fmov` immediates | Verified | two decoder bugs fixed in this milestone: `csinv`/`cinv`/`csetm` ran as `csneg`, and `fmov` with an immediate of 0x30-0x3f decoded as a register move; both found from wrong values in the game's own code (a CRC-32 one too high, a clobbered `this`) and pinned by decode tests |
| Barriers and cache maintenance | Implemented | `dsb`, `dmb`, `isb`, `clrex`, `dc cvac/civac/cvau`, `ic ivau` are no-ops on one coherent host memory; `dc zva` still stops the run |
| AArch64 decode coverage | Verified | **99.97%** of 6,714,569 real words; 2,165 unsupported, 172,019 padding |
| System registers, `svc`, `ldpsw` | Implemented | `mrs`/`msr` (thread pointers, counter, FPCR/FPSR, cache geometry) and supervisor calls in the decoder, interpreter and C emitter |
| Integer, bitfield, multiply-high, `extr`, CRC-32 | Verified | interpreter, emitter and oracle agree |
| Scalar floating point | Verified | |
| Advanced SIMD | Verified | copies, three-same, two-reg, shifts, long/narrow, by-element, permutes, `ext`, `tbl`/`tbx`, reductions, immediates, structure loads/stores; 400 encodings × 6 edge-case states, every register identical (`verify_simd.sh`); 5,000 encodings × 6 states passed before the logic fixes, which do not touch the vector unit |
| Atomics, exclusives, `cas`, acquire/release | Implemented | LSE read-modify-write, `ldxr`/`stxr` with a monitor, `cas`/`casp`, `ldar`/`stlr` |
| Control-flow recovery | Verified | 304,909 blocks, 3,620 complete functions in a 4,000-function pass |
| Jump tables | Partial | 151 tables, 2,005 targets; 380 indirect edges are resolved at run time |
| Emitted C | Verified | 400 functions identical to the interpreter (`verify_cfg.sh`) |
| Interpreter against an independent CPU | Verified | `oracle_lockstep.py` against Unicorn: 30,000 consecutive instructions of the game's zstd decoder identical, register for register, after the fixes listed below |
| Native execution of emitted C in the runtime | Implemented (opt-in) | `ew-native` lifts the whole module, emits it as C, compiles it in parallel and links `ew_native.dll`; `ew-run --native <dll>` dispatches into it at every function the library has, and `EW_NATIVE_CHECK=1` runs each native call beside the interpreter and reports the first difference. Measured on 10 November: the whole main module builds in about 16 minutes, and a billion boot instructions take 12 s native against 22 s interpreted. A run without `--native` still interprets |

### Runtime and host services

| Area | State | Evidence |
| --- | --- | --- |
| Module image, imports, exports | Implemented | 836 jump slots named; every export available to host services |
| Library modules (`sdk`) | Implemented | `sdk` loaded after `main` with its DT_RELR relocations at its base; `main`'s C++ standard library, `nn::util` and audio-configuration imports bind to `sdk`'s own code, and data pointers bind across modules by name |
| Supervisor calls | Partial | lock and condition-variable arbitration, system tick, thread id, sleep, debug output, break; each thread has a TLS block with its handle and `ThreadType` |
| SDK startup sequence | Implemented | `nninitStartup` → module entry → `nnMain`, on one guest state |
| Guest thread scheduler | Implemented | deterministic round robin, per-thread registers, TLS and monitor; deadlock detection |
| `nn::os` mutexes, condition variables, events, semaphores, reader/writer locks | Implemented | real blocking with owner tracking; `EventClearMode` as the SDK defines it; mutex state is kept on the host by address and condition variables use only their own 8 bytes, because this SDK's `pthread_mutex_t` is 8 bytes with its condition variable directly after it |
| `nn::os` threads, sleep, timers, TLS, memory info | Implemented | create/start/wait/destroy; timers on the host clock |
| `nn::os` message queues and system events | Implemented | rings in the game's buffer, blocking when full or empty |
| `nn::os` multi-wait | Implemented | holders for events and system events, kept on the host; `WaitAny` reports without consuming the signal |
| `nn::os` fibers | Blocked | not reached yet |
| `nn::mem::StandardAllocator`, `NumberLineAllocator` | Implemented | one heap per allocator object with a coalescing free list (the title creates three); number lines for pool offsets |
| `nn::lmem` frame heaps | Implemented | head and tail allocation, `ZeroClear` |
| `nn::fs` | Implemented | RomFS read-only, save data in `%APPDATA%\EmblemWeave\save`, real result codes |
| `nn::fs` add-on content | Blocked | no DLC mount is offered |
| `nn::oe`, `nn::account`, `nn::time`, `nn::aoc` | Implemented | language and profile from the launcher, version from `control.nacp`, time in UTC |
| musl C runtime | Implemented | printf/scanf family, `strto*`, stdio, pthreads, `rand`, `errno`, fenv (round to nearest, with the flush-to-zero mode bits the game asks for) |
| Math library | Verified | musl's libm through the `libm` crate; arguments now read from `s`/`d` registers |
| libc++ runtime pieces | Implemented | `random_device`, guards, `__hash_memory`, `__next_prime`, `pmr`, nothrow `new` |
| `qsort`, `__dynamic_cast`, `std::exception_ptr` | Implemented | the title's own `sdk` code |
| `std::thread` | Implemented | `pthread_create` makes a host thread; `detach` and the destructor run on the host, since the SDK's versions search a thread list host threads are not in |
| C++ exception throwing | Blocked | not reached yet |
| `nv::`, `nn::vi`, NVN bootstrap | Implemented | driver region recorded, one SDR display and layer, every NVN procedure resolved to a named host function |
| NVN device | Implemented | `nvnDeviceInitialize` succeeds only when the host Vulkan device exists (`ew-gfx`); measured on an AMD Radeon RX 7900 XT, Vulkan 1.4.359 |
| NVN device properties | Partial | answered where the game's own use establishes them (alignments, queue memory, feature flags); any other id stops the run naming it |
| NVN memory pools, buffers, syncs, events | Implemented | storage-backed pools use the guest address as the GPU address |
| NVN textures, pools, samplers, views | Implemented | block-linear storage sizes by the GOB rules; formats confirmed by the game's own BNTX-to-NVN table, including ASTC |
| NVN render-state objects | Implemented | state packed into the structure's own bytes, so copies keep it |
| NVN command buffers and queues | Partial | recorded on the host with copies of their pointed-at memory; memory effects (copies, clears, uniform updates, reports, fences, events) and the texel commands (texture clears, texture-to-texture copies) execute on the game's tiled storage; **draws are counted, not rendered** |
| NVN window | Implemented | two 3840x2160 swap textures, acquire and present; presents are counted and logged every 300 |
| NVN programs | Partial | shader stages recorded; not translated |
| DLSS upscaler | Partial | answered as a host upscaler at half the output resolution; not evaluated yet |
| Input (`nn::hid`) | Implemented | player 1 is a Pro Controller from XInput pad 0, mapped by button position; `ew-run --press` scripts buttons by frame for headless runs; motion, vibration and the Joy-Con mouse report no devices |
| Audio (`nn::audio`) | Partial | configuration built by `sdk`'s own code; the renderer opens and its event fires every 5 ms; memory pools attach; Opus decodes to silence of the right length; voices are not mixed to the speakers |
| Sound data | Implemented | the game links its own relocatable sound-table ELFs (`Sound/Data/*.elf.zst`); effect and bus lookups resolve |
| Play reports (`nn::prepo`) | Implemented | accepted and never sent |
| Movies (`nn::movie`) | Partial | opening a movie reports unsupported media, and the game skips it; the opening movie is `Movie/Product/MV00_00.webm` (VP9) |
| Frame loop | Implemented | thousands of frames presented headless; the autosave notice and the first battle-map load are reached |
| **Playable frame** | **Not reached** | draws are not rendered |

### Launcher

| Area | State |
| --- | --- |
| Liquid-glass controls; schemes: XiØN's own palette exactly (xiondigital.net), a retro arcade palette with a neon grid (Arcade), gold, silver and blue (FE); the logo render in a colourway per scheme | Implemented |
| Navigation, theme system ([XiØN](https://github.com/XiONDiGiTaL), Emblem, Arcade), wordmark and crest | Implemented |
| Persistent settings (atomic writes, recovery from a damaged file) | Implemented |
| Game data detection and validation (plain dumps and NSP extractions) | Implemented, checked on the retail data |
| Main module decoding through the bridge | Implemented |
| Recompilation pipeline with staged progress and cancellation | Implemented, run on the retail data |
| Boot test with live progress, cancellation and a full report | Implemented, run on the retail data |
| Diagnostics with search, demangled symbols and export | Implemented |
| Language and profile settings | Implemented; consumed by the runtime |
| Upscaling | Implemented: FSR 1.0, NVIDIA Image Scaling, FSR 3.1 and DLSS/DLAA are chosen on **Graphics** and written to the engine's `fefw.ini` (with the DLL folder when one is found) before every start, so the game itself runs them |
| Display settings | Implemented: resolution, render scale, window mode, vsync, aspect ratio and frame-rate limit are written to play mode's `controls.ini` (`[display] window_mode`, `vsync`, `aspect`, `fps_limit`, `width`/`height`, `render_width`/`render_height`, `render_scale`) and the picture size to `[fefw] output` in `fefw.ini` before every start |
| Audio master volume | Implemented: written to `[audio] volume` in the same file; music, effects and voice wait for the mixer |
| Settings only when changed | Implemented: a start writes only values that differ from the ones it wrote last (kept in `%APPDATA%\EmblemWeave\applied.json`), so settings changed in the game's F1 menu are not reset by the next launch |
| Settings read back | Implemented: **Graphics** and **Audio** re-read `fefw.ini` and `controls.ini` (once a second, `engine::live`), show what the game will start with, name the settings still waiting for the next start, and take back the values the game's F1 menu changed (`engine::adopt`) so the pages agree with it, the render size too (pixels in the file, a ratio on the page) |
| Quality and shader precompiling | Saved only; the page says so |
| Taskbar icon | Implemented: the launcher puts its own icon into `fefw.exe` at startup, after a build and before a start (`exe_icon::brand` via `engine::brand_game`); a copy already running keeps the engine's icon until it closes |
| Log files | Implemented: every job writes `%APPDATA%\EmblemWeave\logs\<task>-<date>.log` with the build stamp, paths, arguments, each stage and the full output of the tools it ran (the C compiler's whole message, not a summary); **Diagnostics > Log** shows the folder and opens it |
| Compiler preflight | Implemented: one short file is built before the shards, so a compiler that cannot run says so once instead of failing every shard |
| Command line: `--page`, `--boot-test` | Implemented |

## Measured boot path

```
target\release\ew-run --module work\decoded\main --lib work\decoded\sdk --romfs <romfs> --max-steps 60000000000 --max-seconds 450 --press A@2500 --press A@3000 --press A@3500
```

This is the interpreted run. Adding `--native build\native\ew_native.dll`
runs the same path through the compiled code, and `--window` shows the frames
that run presents.

Over 15 billion guest instructions and more than 46,000 functions in 450
seconds, with these host events in order (abridged):

```
heap: StandardAllocator given 0xffe00000 bytes, then two sub-heaps
fs: mounted Contents: read-only; open Contents:/Fs.bin.zst
gfx: NVN device on AMD Radeon RX 7900 XT (Vulkan 1.4.359)
gfx: NVN queue created; memory pool 0x1f000000 bytes
gfx: window: 2 textures of 3840x2160, format 0x3d, present interval 1
gfx: upscaler for output width 3840
fs: open Contents:/Shaders/*.bfsha.zst (16 shader archives)
fs: open Contents:/Graphics/Textures/common_default.bntx.zst, character_default.bntx.zst
gfx: memory pools for the texture archives
audio: renderer 1 opened; its event fires every 5 ms
fs: open Contents:/Sound/Data/*.elf.zst, Font/*, Layout/Common.arc.zst, Hub.arc.zst
gfx: first frame presented; 300, 600, ... frames presented
fs: open Contents:/Layout/Textures/Tutorial/USen/TUT_Hub_AutoSave_001.bntx.zst
movie: Contents:/Movie/Product/MV00_00.webm cannot be played yet; reported as unsupported media
fs: open Contents:/Map/Btl/..., Map/Com/Obj/..., Map/Tex/... (battle-map data)
```

## What changed in this milestone

1. **Into the frame loop.** Two decoder bugs (`csinv` and `fmov` immediates)
   were silently corrupting the game's arithmetic. Fixing them, keeping mutex
   and condition-variable state from overwriting neighbouring objects, and
   adding multi-wait, play reports, `std::thread` and a movie layer took the
   boot from 1.4 billion instructions to the frame loop, the autosave notice
   and the first map load.
2. **Debugging tools.** `ew-run --watch ADDR` logs a function's arguments and
   return value (with `EW_PEEK` memory dumps), `--press` scripts input,
   `--all-calls` lists every import reached, and `ew_unzst` unpacks a RomFS
   `.zst` file.

### Earlier in this milestone

1. **The NVN layer.** Device, queues, memory, textures, samplers, render state,
   programs, window and command buffers are served on the host. Device
   properties come from the game's own use; texture formats come from the
   game's BNTX-to-NVN table, which confirmed the numbering.
2. **The title's `sdk` module.** Loaded beside `main` with RELR relocations;
   the C++ standard library and other library code now run as the title's own
   code. This alone took the run from 3.6 million to 17 million instructions.
3. **System instructions and supervisor calls.** `mrs`/`msr`, `svc` and
   `ldpsw`, with per-thread TLS and kernel lock arbitration.
4. **A real heap.** Three allocator objects previously shared one heap and
   could hand out overlapping memory; each now has its own free list.
5. **Input and audio start-up**, message queues and system events.
6. **Launcher**: the exact xiondigital.net palette, Arcade with its grid, the
   FE scheme, a new wordmark and crest, liquid-glass controls, and command-line
   options that make the screenshots reproducible.

## Known limitations

1. **Native code is opt-in, and host calls still leave it.** Without
   `--native` the runtime executes the reference interpreter, about 15 million
   guest instructions per second in a release build. With a library from
   `ew-native` it dispatches into compiled code instead, but every host call a
   native function makes returns through the full run loop, so hot loops that
   call into the host (a shuffle calling the C library's random number generator
   each iteration, say) still pay for it. Serving simple host calls from inside
   native code is the remaining speed step.
2. **Nothing is drawn.** Command buffers are recorded and their memory effects
   and texel effects (clears, texture copies) carried out, so a render target
   holds what the game asked for, but draws are not executed and the game's
   shaders (compiled for the console's GPU) are not translated yet. The window
   presents the swap texture; until the draws run, it is not the game's picture.
3. **ZBIC needs an external decoder.** The bridge runs the GPL-2.0 `zbic`
   package out of process. A clean-room Rust decoder is planned.
4. **Floating point environment.** Only round to nearest is modelled; the
   flush-to-zero and default-NaN mode bits are accepted but not applied.
5. **Time zone.** Calendar conversion is UTC.
6. **Optimised native builds.** The launcher compiles the translation at `-O0`
   to prove it builds; the largest lifted functions take many minutes at `-O1`.
7. **Emitted C size.** 4,000 functions produce about 213 MB of C. The launcher
   splits it into shards balanced by instruction count and compiles them in
   parallel, but the largest functions still take minutes at `-O1`; the emitter
   needs to produce tighter code before the whole game is compiled.
8. **Function boundaries are inferred**, not proven, because the module has no
   symbol table for its internal functions.

## Next actions, in order

1. **Audio renderer updates.** Serve the renderer's update protocol so voices
   report played samples and consumed wave buffers. The game's sound handles
   are released only then, and today their pool runs out during the first map
   load. Exit criterion: the long run passes the map load without the abort.
2. **Rendering.** Execute the recorded draws so the frame in the window is the
   game's own. In place already: the window, its swap chain, the upload of the
   presented swap texture (`ew-run --window`, and the launcher's own presenter in
   `ew-gfx::present`), and the buffer and texture clears and copies that write
   render targets. What is missing is the draws themselves, which is the exit
   criterion: the game's first frame on screen. Then shader translation from the
   console GPU's binaries, and texture decoding (block-linear, BC, ASTC).
3. **Host calls from native code**, so a native function that calls the host
   does not return through the whole run loop (see limitation 1).
4. **Audio mixing** to WASAPI, and voice state updates.
5. **Movies**: VP9 decoding for `Movie/Product/*.webm` and the cut scenes.
6. **Gameplay validation** from the first battle map on.

## Reproducing these results

```powershell
# decode the main module once
python tools/zbic_extract.py <exefs> work/decoded main

# workspace tests
cargo test --workspace

# decode the sdk module (LZ4) once
target\release\ew-decode <exefs>\sdk work\decoded\sdk

# run the game on the interpreter and print the stop report
target\release\ew-run --module work\decoded\main --lib work\decoded\sdk --romfs <romfs> --nacp <control.nacp> --max-steps 2500000000

# build the game's code into one native library (about 16 minutes for the whole module)
target\release\ew-native --module work\decoded\main --out build\native

# run that instead of the interpreter, checking every native call against it
$env:EW_NATIVE_CHECK=1
target\release\ew-run --module work\decoded\main --lib work\decoded\sdk --romfs <romfs> --nacp <control.nacp> --native build\native\ew_native.dll --max-steps 60000000000

# show the frames the game presents in a window (they are not drawn yet)
target\release\ew-run --module work\decoded\main --lib work\decoded\sdk --romfs <romfs> --window "EmblemWeave"

# differential verification on your data
bash tools/verify_cfg.sh
bash tools/verify_simd.sh work/decoded/main/text.bin 400

# lockstep against the independent CPU from a snapshot
target\release\ew-run --module work\decoded\main --romfs <romfs> --snapshot-at 0x6c500 --snapshot-steps 30000
python tools/oracle_lockstep.py build/snapshot
```
