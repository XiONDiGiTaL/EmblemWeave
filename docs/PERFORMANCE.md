# Performance

What the build does to make the game cheap to run, what was measured rather than assumed, and what was read from
other recompilation projects and deliberately not copied.

## Measured

| Change | Before | After | How it was measured |
| --- | ---: | ---: | --- |
| `/OPT:REF /OPT:ICF` on the `fefw.exe` link (`game-build/fefw/CMakeLists.txt`) | 173,405,696 bytes | 168,420,352 bytes | Relink only (`cmake --build . --target fefw`, 22 s), then a 50 s play run that presented frames and logged 55 fps lines. |
| SSE4.2 CRC-32C in the `.nx` readers (instead of the scalar table) | 0.48 GB/s | 9.91 GB/s | CRC microbenchmark, 64 MiB buffer, 32 passes per path on the development PC; both paths produced the same checksum. Single-threaded integrity-check rate, not a whole-game frame rate. |

`/DEBUG` leaves `/OPT:REF` and `/OPT:ICF` off unless they are named, and the recompiled code repeats the same small
helpers in every translation unit. Naming them folds those copies: 4.9 MB less to page in, and a smaller
instruction-cache footprint for the same code. It is a small win, measured rather than claimed.

The build also removes the `RCX*.tmp` scratch files lld leaves behind when a link is interrupted. One build folder had
eight of them, 1.2 GB in total, none of them ever read again.

## This pass: the launcher's caption and the frame counters

| Change | Before | After | How it was measured |
| --- | --- | --- | --- |
| Maximized launcher caption drag (`apps/emblemweave/src/platform.rs`) | 0 px of window movement through a 1.9 s drag on the caption: winit starts a move only for a restored borderless window, so a maximized launcher sat still until it was restored by hand | restores under the pointer 40 ms after the press, then tracks the cursor with 0 px lag | A scripted Win32 probe (`SetCursorPos` + `mouse_event`, `GetWindowRect` at every step) against the installed launcher; the same probe on the restored path measures 0 px lag and a first move after 36 ms |
| `Frames::stats()` (`apps/emblemweave/src/perf.rs`) | three copies of the last 900 frame times, each sorted, on every call the Diagnostics page and overview graph made | one copy, one sort, every number read from that order | `perf::tests`; the sort count is structural, not an FPS claim |
| `FpsStats::snapshot()` (`engine/fefw-recomp/runtime/src/gfx/fps_stats.cpp`) | a fresh `std::vector` for up to 5 s of frames, allocated and filled every 250 ms while the counter is shown | the same buffer, kept per thread and refilled | `gfx_fps_stats` tests cover every number it reports; the allocation removal is not a gameplay FPS claim |
| F1 counter (`engine/fefw-recomp/runtime/src/gfx/overlay.cpp`) | the most detailed view filled five lines into a four-float array | the array is sized for all five lines | code inspection; the fifth line only appears with frame generation on |

No gameplay FPS number is claimed for this pass: none of these changes was measured on the game with a capture.

## This pass: borderless edges and the last caption slide

The previous caption fix stopped a maximized launcher sitting still, but the restore itself was still a window-manager
transition. `ShowWindow(SW_RESTORE)`, and later `SetWindowPlacement`, both animate the restore; the move loop
anchors on an intermediate frame, so the first pointer movement jumped the window (14 px with the intermediate
`SetWindowPlacement` build). The launcher now clears `WS_MAXIMIZE` and sets the rect with `SetWindowPos`
directly, which has no transition, and asks DWM to disable its transitions for the window. The borderless edges
start in `WM_NCHITTEST`, the same pass as the caption, instead of waiting for `ViewportCommand::BeginResize`
after the frame; Windows owns the sizing loop and the resize cursors.

| Change | Before | After | How it was measured |
| --- | --- | --- | --- |
| Maximized caption press | 182,182 -> 307,0 -> 321,0 -> 328,0 over ~50 ms with no pointer movement (the OS restore animation) | 301,0 1440x900 as the drag starts, with no transition frame | scripted Win32 probe, `GetWindowRect` with the launcher forced to the foreground |
| Dragging on from that same press | 14 px jump when the move loop anchored on an intermediate `SetWindowPlacement` frame | 0 px settled lag over 80 one-pixel steps | the same probe, 1 px steps with a 20 ms settle |
| Restored caption drag | an injected event of catch-up in the old probe | 0 px settled lag over 80 one-pixel steps | the same probe, 1 px steps with a 20 ms settle |
| Left and right borderless edges | `BeginResize` was sent after the frame that saw the press | 0-1 px settled lag; the left edge 300->200 grew the window 1440->1540 and stayed there; a second run 340->280 grew it 1440->1500; the right edge tracked to 1620 | the same probe, 1 px steps with a 20 ms settle |
| Resize cursors | egui set its own icon | Windows' hit-test cursors: WE on left/right, NS on top/bottom, NWSE/NESW on the corners | `GetCursorInfo` after moving the real pointer to each band |
| Hit-test codes | caption only | caption 2; left 10, right 11, top 12, bottom 15, top-left 13, bottom-right 17 | `WM_NCHITTEST` sent at each band |

A minimize/restore after a dragged restore came back at the same rect with the placement at normal (`showCmd` 1),
so clearing `WS_MAXIMIZE` did not leave a maximized placement behind.

The probe uses synthetic Win32 input, so it measures the window manager's response, not a physical mouse.
Snapped and half-screen window states were not exercised.

No gameplay FPS number is claimed: this pass changes the launcher frame, not the engine.

## What the engine already does

These are in `engine/fefw-recomp/runtime` and are on by default; `runtime/docs/CONFIG.md` has every key.

- **Pipeline prewarm.** Pipelines met at a draw are recorded in `pipeline_keys.bin`; the next start builds them on
  worker threads before the draw asks (`prewarm_threads`).
- **Graphics pipeline library.** A draw whose pipeline is still building gets a fast-linked one from separately
  compiled parts; a background thread replaces it with the whole pipeline and the next lookup swaps it in
  (`gpl`, `gpl_upgrade`).
- **Pipeline cache.** `pipeline_cache.bin`, written while new pipelines appear and at shutdown, with the save and the
  compiles taking turns rather than blocking the render thread (`pipeline_cache`, `pipeline_cache_lock`).
- **Frame limiter on a high-resolution waitable timer**, not `timeBeginPeriod` plus `Sleep(1)`.
- **Compile threads below idle and in Windows efficiency mode**, so a burst of shader work does not take the fast
  cores from the game (`compile_priority`, `compile_eco`).
- **Pool mirrors** for buffers on a discrete GPU: the log's `nvn.pool: pool mirrors` line reports how much is mirrored
  and how many binds were served from the copy.
- **Per-program derived state.** The stage-set to pipeline-layout lookup has a direct-mapped cache, and each layout's
  alpha-test/HDR variant and whether its shaders write storage are worked out once, not on every draw (`layout_cache`,
  `alpha_cache`, `ProgLayout::writes_storage`).

## Read from NX2navi, and what happened to it

[NX2navi](https://github.com/tangerineturtle1/NX2navi) is another Switch→PC static recompilation, for *Ocarina of
Time*. Its source is public to read, but its additions are **all rights reserved**: `LICENSE-ADDITIONS` says no part of
them may be copied or published without written permission, and it is not compatible with this project's GPL-3.0. So
nothing was copied. What follows is what its own log and documentation show it doing, compared with this engine, and
whether the idea is worth pursuing here independently.

| Technique | NX2navi | This engine | Verdict |
| --- | --- | --- | --- |
| Pipeline library with a fast-linked stand-in, replaced in the background | yes | yes: `gpl`, `gpl_upgrade` (`whole` or `lto`) | Already here, with a measurement switch (`gpl_ab`) to compare. |
| Prewarm from a recorded key file | yes | yes: `pipeline_keys.bin`, `prewarm_threads` | Already here. |
| Pipeline cache that survives a translator change | carries translations from the previous translator | shader cache is keyed per translator identity | Worth checking: a translator update currently costs a fresh compile of everything. This is the most useful idea on the list. |
| A new VMA block allocated ahead of time on a background thread (`VRAM_HEADROOM`) | yes | no | Candidate, but it must be measured: an extra block is memory held away from the game. The engine's own `tools/perf` scripts can compare a run with it on and off. |
| `SignedZeroInfNanPreserve` on every stage | yes | no | Candidate, and a correctness question before a speed one: if the translated SPIR-V assumes IEEE denormals, preserving them can be *slower* and more correct. Test against the shader corpus, not by feel. |
| GPU breadcrumbs (`VK_AMD_buffer_marker`) | yes, on by default | no | Diagnostics, not speed. Useful for the "device lost" reports; cheap to add behind an option. |
| Mesh shaders, per-draw fragment shading rate | yes | no | Not applicable: those are properties of NX2navi's own renderer, not of the NVN draws this engine replays. |
| 1 ms timer kept while the window is not in front | yes | high-resolution waitable timer, no `timeBeginPeriod` | Equivalent result, better mechanism. Nothing to do. |

## Reproducing a number

1. Build the game, then run it for a fixed time and capture the engine log (`NXRT_GFX_LOG_FPS=1` gives one line per
   second with the presented rate).
2. For a link-only comparison, change the link flags and rebuild the `fefw` target; only the link reruns.
3. `runtime/scripts/prof/prof_run.py` runs the executable with a frame cap, a duration and a tag, and writes what the
   run reported. `tools/perf` holds the report scripts.

A change is only recorded above once a run has produced the number.
