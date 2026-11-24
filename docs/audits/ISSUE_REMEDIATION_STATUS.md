# Issue Remediation Status

**Date:** 2026-11-24
**Basis:** the code on canonical `main` after the FCC integration, the build and test run recorded in
`ENGINE_AND_RENDERER_VALIDATION.md`, and the reconciled audit `V0.1.42_RELEASE_READINESS.md`.
**No GitHub issue was closed or commented on.** Every status below is a statement about the code in this
repository, not about the issue tracker.

Status vocabulary is the one the readiness audit uses:

| Status | Meaning |
|---|---|
| Implemented - unverified | Code exists and compiles or has unit coverage; the user-visible behaviour has not been demonstrated. |
| Partially implemented | Some required behaviour exists; a named part is missing or contradicted by the code. |
| Outstanding | No adequate implementation or no adequate evidence. |
| Verified | Observed working in a controlled run, with the run recorded. **Nothing in this table is Verified.** |

---

## Summary

| Issue | Status after this pass | What exists | What is still missing |
|---|---|---|---|
| #37 Audio/RAM growth over long sessions | Partially implemented | Renderer counters (frame time, texture/upload/pipeline/submission/GPU-sync), memory+resource snapshot, deferred-destruction drain, audio stream-registry purge, concurrent large-decode cap | A multi-hour session; scene-transition before/after numbers; any evidence that memory growth is bounded |
| #41 Crashes past Chapter 12 | Outstanding | Memory snapshot now attached to the device-lost report; VMA pressure gauges; `[gfx] frame_wait_ms`; deferred-destroy drain; packed-cache decode cap | A reproduction, the reporter's full log, and a measured memory profile. No fix is claimed. |
| #59 Launcher drag responsiveness | Implemented - previously measured | `IN_MOVE` suppression and frame interval (already on `main`) | Not re-measured in this pass; DWM behaviour on the reporter's machine untested |
| #70 Cinematic AV1 (dav1d assembly) | Implemented - unverified | dav1d per-compiler flags build with NASM; assembly path enabled | Any cinematic played end to end; a NASM vs C-only comparison |
| #75 Cinematic AV1 + Opus sync | Implemented - unverified | Worker-thread decoder, decode-ahead, clock-based dropping, bundled FFmpeg option | End-to-end video/audio playback and sync measurement |
| #76 Find Units stutter | Implemented - unverified | Parallel scan, cancellation, progress | An in-battle frame-time measurement |
| #77 Return-to-title stability | Implemented - unverified | Fault-handler initialisation and fallbacks; write-watch platform split now builds | 10+ menu/title cycles; the write-watch race path exercised |
| #78 HDR left enabled after exit | Partially implemented | `play::stop()` forces windowed mode; HDR output path | Capturing and restoring the original Windows HDR state, SDR white level and display mode. The runtime still never reads them. |
| #79 Saves not read | Implemented - unverified | R1-R8 reconciliation, adoption, diagnostics, empty-save guard, read-only fixture harness | A controlled run of `tools/issue79_fixture.py` plus game launches, and a reporter confirming a save loads |
| #7 Linux / Steam Deck | Outstanding | `write_watch_linux.cpp` source (uncompiled) | A Linux build target at all: core still links `psapi` and includes `<windows.h>` |

---

## Per-issue notes

### #37 — audio and resource lifecycle

This pass adds instrumentation, not a fix. What is now available on a real run:

- `[prof] renderer_profiling = true` with `[prof] renderer_csv` gives one row a second of frame-time percentiles,
  texture create/destroy counts and bytes, upload/transcode time, pipeline creation and cache behaviour, render
  thread phase time, submission counts, GPU-sync waits, GPU memory, and the prewarm counters.
- `[diag] soak_csv` already samples process working set, private bytes, handles and threads; the profiling gauges
  are registered into it, so a soak line and a profiling line can be read side by side.
- `diag_memory_snapshot_text()` produces a full process/system/VMA/gauge/thread/recent-file snapshot, now included
  in the GPU device-lost report.
- `backend_drain_deferred_destruction()` lets a scene transition release the previous scene's GPU objects instead
  of letting them pile up until the next natural retire.
- The packed RomFS cache now limits concurrent decodes of entries too large to cache (two at a time), which is the
  path that spikes system memory when several large voice banks load together.
- Audio streams remove their dead weak references from the registry on shutdown, so a session that opens and closes
  many streams does not accumulate registry entries.

**None of this demonstrates bounded memory.** Closing #37 needs a 2-hour session with `soak_csv` and
`renderer_csv` on, and a before/after comparison across ten scene transitions. That run has not happened.

### #41 — chapter-transition failures

The audit's key distinction stands: a Vulkan device-loss report does **not** independently establish a system
memory leak. This pass deliberately adds evidence collection and bounded-memory mitigations without claiming a
cause:

- video-memory pressure (`vma.pressure_percent`, `vma.live_pressure_percent`) and the deferred-destruction queue
  depth are now visible in the device-lost report, next to the process and system memory figures;
- `[gfx] frame_wait_ms` turns an unresponsive GPU or compositor into skipped frames (and a swapchain rebuild after
  three in a row) instead of a permanently frozen window;
- the scene-transition drain and the decode cap reduce peak pressure.

**No fix is claimed.** The report is from a 16 GB shared-memory device with device loss after roughly 400 seconds,
and the full log is still not in the repository. Without it, any "fix" would be speculative.

### #70 / #75 — cinematics

The dav1d flag fix is on `main` and the canonical build produces dav1d with x86-64 assembly (NASM found and used).
That is a build fact, not a playback fact: no cinematic has been played, so AV1 decode, Opus audio, A/V sync,
1440p smoothness and frame drops are all unmeasured. The FFmpeg/D3D11VA path is likewise unexercised.

### #77 — return to title

The 120-second FCC session is the only runtime evidence and it is not 10+ cycles. The write-watch refactor it
depended on now builds (CR-1), which removes the "did the fix survive the refactor" question at the source level,
but the race itself has not been exercised. The `[nvn] write_watch` unit test is part of the engine suite.

### #78 — HDR and display restoration

Unchanged in substance. Forcing windowed mode is not restoring the user's display state: the runtime still does
not capture HDR enablement, SDR white level or the display mode before it starts, so it cannot put them back.
This remains a named, missing piece of behaviour rather than a bug with an unknown cause.

### #79 — save discovery

The read-only fixture harness (`tools/issue79_fixture.py`) is the controlled path: it copies disposable fixtures,
writes a dedicated `fefw.ini`/`fefw.log`, records source and fixture manifests, and refuses to run against the
user's real installation, `E:`, `X:`, `%APPDATA%` saves or the repository's `data` directory. It deliberately does
not launch the game. The launcher side of the matrix still has to be run by hand, and no reporter has confirmed a
save loading, so the issue stays implemented-unverified.

---

## What would move each item

| Issue | Minimum evidence to change status |
|---|---|
| #37 | 2-hour soak with `soak_csv` + `renderer_csv`; flat working set and bounded GPU memory across 10 transitions |
| #41 | The reporter's full `fefw.log` and device-lost report; a reproduction on a 16 GB shared-memory device |
| #59 | Drag measurement on the reporter's Windows build, with and without DWM composition |
| #70 / #75 | One cutscene played to completion with audio; decoder statistics and frame times recorded |
| #76 | In-battle frame-time percentiles with the scan open and closed |
| #77 | 10+ return-to-title cycles in one session with no fault |
| #78 | Start with HDR on, exit, and show Windows still has HDR on with the original SDR white level and mode |
| #79 | A fixture run plus a game launch that lists and loads the fixture save |
| #7 | A native Linux build of `nxrt` + `nxrt_gfx`, then a runtime test; Proton and Gamescope as separate targets |
