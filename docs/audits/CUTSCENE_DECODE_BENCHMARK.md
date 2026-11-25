# Cutscene decode benchmark: NASM assembly vs C-only

**Date:** 2026-11-24
**Machine:** AMD Ryzen 7 5700X3D (8 cores / 16 threads), Radeon RX 7900 XT, Windows 11
**Tree:** `main` at `730fc46c`, plus working-tree movie-stats instrumentation (see *Instrumentation*)
**Compiler:** clang-cl 23.1.5 for both binaries
**NASM:** 3.02 at `%LOCALAPPDATA%\bin\NASM\nasm.exe`

This measures the local machine, not the reporter's Ryzen 9. The subject is the opening 1440p AV1 movie
(`/Movie/Product/MV00_00.webm`), which the game plays automatically at start-up. It is not an in-game story
cutscene and no gameplay save is used.

## Builds

| Build | Output | Configure | Runtime banner |
|---|---|---|---|
| Assembly | `game-build/fefw/build/release/fefw.exe` (169,846,272 B) | `NXRT_DAV1D_ASM=ON` (auto-detection) | `with x86-64 assembly` |
| C-only | `game-build/fefw/build/c-only/fefw.exe` (169,180,672 B) | `NXRT_DAV1D_ASM_AUTO=OFF`, `NXRT_DAV1D_ASM=OFF` | `C only: built without NASM` |

```powershell
build_fefw.ps1 -Config Release
build_fefw.ps1 -Config Release -BuildDir build/c-only `
  -CMakeArgs "-DNXRT_DAV1D_ASM_AUTO=OFF -DNXRT_DAV1D_ASM=OFF"
```

The C-only build remains practical as a diagnostic: it configures, links and runs. It is not a viable
shipping configuration for this title, as the results below show.

## Method

- 1080p output, 60 fps cap, dav1d 1.5.1, 8 decoder threads, 30 decoded pictures ahead, above-normal
  decoder priority, movie clock on.
- Same warm shader and BC7 caches; fresh save directory each run; no input; `[boot] max_seconds = 70`.
- Runs alternate assembly and C-only. Three instrumented pairs are reported here; an earlier five-pair
  series without the pacing histogram reproduced the decode and drop figures.
- Decode time is the runtime's own weighted `ms/packet` across the 5 s stats windows, excluding the first
  partial window. `Slowest packet` is the largest per-window slowest packet.
- Presented rate is the change in `frames acquired` over the measured movie window.
- Frame gaps are host-clock intervals between successful `AcquireVideoFrame` calls, i.e. the cadence at
  which the game player takes movie pictures. The histogram is cumulative. p95/p99 are upper bucket bounds;
  the 50 ms threshold is 1.5 frames at 30 fps.
- Audio: XAudio2 reported zero underruns in every run.

## Results (three instrumented pairs)

| Run | Decoder | Decode ms/pkt | Slowest packet | Shown fps | Late skips | Gap mean | p95 gap | p99 gap | Max gap | Gaps >50 ms |
|---|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| asm1 | assembly | 8.70 | 53.8 | 30.00 | 0 | 33.3 | 66.7 | 66.7 | 97.0 | 153/1655 (9.2%) |
| asm2 | assembly | 8.87 | 54.6 | 30.00 | 0 | 33.3 | 66.7 | 66.7 | 108.9 | 183/1653 (11.1%) |
| asm3 | assembly | 9.03 | 49.1 | 30.00 | 0 | 33.3 | 66.7 | 66.7 | 92.3 | 164/1655 (9.9%) |
| c1 | C-only | 30.43 | 142.8 | 29.21 | 36 | 34.2 | 150.0 | 150.0 | 210.9 | 453/1614 (28.1%) |
| c2 | C-only | 32.10 | 176.1 | 29.24 | 8 | 34.1 | 150.0 | 150.0 | 176.4 | 412/1477 (27.9%) |
| c3 | C-only | 30.88 | 157.2 | 26.49 | 20 | 37.5 | 150.0 | 150.0 | 5035.3 | 391/1412 (27.7%) |

Per 5 s window, assembly stayed at **29.9–30.1 fps** in all 30 windows. C-only windows ranged **11.4–43.2 fps**
with **15 of 28** windows below 29 fps.

### Aggregate

| | Assembly | C-only |
|---|---:|---:|
| Decode mean | **8.87 ms/packet** | **31.13 ms/packet** (3.51x) |
| Shown rate | **30.00 fps** in all runs | **28.31 fps** mean (26.49–29.24) |
| Late skips | **0** | **64** |
| Gaps >50 ms | 500/4963 (**10.1%**) | 1256/4503 (**27.9%**) |
| p95 / p99 gap | 66.7 / 66.7 ms | 150 / 150 ms |
| Max gap | 108.9 ms | 5,035.3 ms (stall); 176–211 ms otherwise |
| Movie frames not presented vs 30 fps | 0 | 242 over 143.5 s (5.6%) |

The earlier five-pair series measured **9.84 vs 36.79 ms/packet (3.74x)**, assembly at 30.00 fps with zero
skips, and C-only at 25.42 fps mean (20.84–29.63) with 31 skips and 1,117 missing frames over 244.6 s. One
C-only run hit the 5 s no-progress guard. That series is the wider-range observation; the instrumented series
is the lower-noise one. The decode gap is consistently about **3.5–3.7x**.

## Reading the pacing

Average fps understates the C-only problem. Run `c1` averaged 29.21 fps and run `c2` 29.24 fps, yet both
spent **28%** of frame intervals above 50 ms and had p95 gaps of 150 ms. Their 5 s windows swung from 24.0
to 35.1 fps and 21.8 to 43.2 fps. That is visible cadence trouble even when the one-minute average is close
to 30.

Assembly is not perfectly uniform at the acquire boundary: about 10% of intervals exceeded 50 ms, with a
worst gap of 109 ms. But it lost no pictures, held 30 fps in every window, and never approached the C-only
tail. The distinction is between occasional scheduling jitter and sustained decoder starvation.

## Effect on #101 / #70

- Without NASM, the 1440p AV1 decoder is roughly 3.5–3.7x slower per packet and cannot hold 30 fps on this
  8-core CPU. Even its best runs average about 29.2 fps with heavy pacing jitter; its worst instrumented run
  averaged 26.5 fps and 20 late skips, and one prior run reached 20.8 fps. This is the shortfall #101 fixed by
  making NASM required.
- With NASM, every measured run held 30.00 fps with zero late skips. The decode budget (8.9 ms/packet) leaves
  substantial headroom for scheduler and render jitter.
- This quantifies video decode and frame delivery. It does not test audio/video sync, in-game story cutscenes,
  gameplay, or the FFmpeg/GPU path proposed in #105.

## Limitations

- Opening movie only; no gameplay save or story cutscene.
- The 70 s deadline ends every run. In the instrumented series, two of the three C-only runs raised a host
  exception at about 71.2 s, after the end-of-run teardown and after all movie stats; the three assembly runs
  and the remaining C-only run exited normally with code 124. One assembly run in the earlier series raised a
  heap-corruption exception at the same point. The teardown crash is intermittent and is not attributed to
  NASM or the decoder; it needs separate investigation.
- C-only results drift with sustained load/thermals (the earlier series slowed to 45 ms/packet). The alternating
  order reduces, but does not remove, that effect.
- p95/p99 are histogram bucket upper bounds, not exact order statistics.
- The instrumented stats line and gap histogram are **uncommitted** in this tree.

## Artifacts

- Logs: `_recovery/bench/pasm1..3.log`, `_recovery/bench/pc1..3.log`; earlier series
  `_recovery/bench/asm1..5.log`, `_recovery/bench/c1..5.log`.
- Binaries: `_recovery/bench/asm/fefw.exe`, `_recovery/bench/c-only/fefw.exe`.
- Build logs: `_recovery/bench/asm-build2.log`, `_recovery/bench/c-only-build2.log`.
- Tests: `media_vdec` 11/11 after the instrumentation change.
