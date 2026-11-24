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
| Verified | Observed working in a controlled run, with the run recorded. Only #92 is Verified, by the captures recorded below. |
| Implemented - automation-verified | The defect's mechanism is covered by an automated check that fails on the old code and passes on the new; the end-to-end user path was not re-run. |

---

## Summary

| Issue | Status after this pass | What exists | What is still missing |
|---|---|---|---|
| #37 Audio/RAM growth over long sessions | Partially implemented | Renderer counters (frame time, texture/upload/pipeline/submission/GPU-sync), memory+resource snapshot, deferred-destruction drain, audio stream-registry purge, concurrent large-decode cap | A multi-hour session; scene-transition before/after numbers; any evidence that memory growth is bounded |
| #41 Crashes past Chapter 12 | Outstanding | Memory snapshot now attached to the device-lost report; VMA pressure gauges; `[gfx] frame_wait_ms`; deferred-destroy drain; packed-cache decode cap | A reproduction, the reporter's full log, and a measured memory profile. No fix is claimed. |
| #59 Launcher drag responsiveness | Implemented - re-measured 2026-11-24 | `IN_MOVE` suppression, frame interval and a one-second Overview quick-action cache | Drag 1.9-4.9 % CPU with 0 px lag; steady idle 0 % after ~10 s; Overview sweep 39.8 % -> 21.1-26.8 %; reporter hardware still untested |
| #70 Cinematic AV1 (dav1d assembly) | Implemented - unverified | dav1d per-compiler flags build with NASM; assembly path enabled | Any cinematic played end to end; a NASM vs C-only comparison |
| #75 Cinematic AV1 + Opus sync | Implemented - unverified | Worker-thread decoder, decode-ahead, clock-based dropping, bundled FFmpeg option | End-to-end video/audio playback and sync measurement |
| #76 Find Units stutter | Implemented - unverified | Parallel scan, cancellation, progress | An in-battle frame-time measurement |
| #77 Return-to-title stability | Implemented - unverified | Fault-handler initialisation and fallbacks; write-watch platform split now builds | 10+ menu/title cycles; the write-watch race path exercised |
| #78 HDR left enabled after exit | Implemented - verified on one display | Full display snapshot (mode, HDR, SDR white, per monitor), sidecar snapshot for crash recovery, restore of only what the session changed, `SET_HDR_STATE`, 18 tests (13 fake, 5 live) | Live on Windows 11 26100 / RX 7900 XT, one 2560x1440@180 display: marked exclusive entered and exited normally, HDR-off stayed off, HDR-on stayed on, forcing HDR off during exclusive restored it, and a hard-killed marked session was repaired from the sidecar on the next launch. Remaining: SDR white is read-only (captured, not written), no second monitor, no live differing-mode restore, no live second restore of the same snapshot. |
| #79 Saves not read | Implemented - fixture matrix run, reporter case open | R1-R8 reconciliation, adoption, diagnostics, empty-save guard, fixture harness with refusal/manifest tests | Rows 16, 17, 19, 20 and 22 ran against the real engine: stale absolute paths honoured, nested save not adopted, missing `.meta` regenerated, foreign account ignored. Row 24's launcher import/backup/restore round-trip ran in a portable copy; its empty-profile UI flow and a reporter's own save remain (see `SAVE_VALIDATION_ISSUE79.md`) |
| #90 Long inherited PATH stops the build | Implemented - automation-verified | `engine.rs` sets `NX_ENV_READY`, so `nx_env.cmd` keeps the launcher's vcvars environment instead of running it again; a direct call runs `vcvars64.bat` with a short PATH and merges both afterwards; every long value is only named through delayed expansion. `check_cmd_scripts.ps1` covers a 7,906-character PATH in launcher and direct mode, and `package_release.ps1` runs that check before packing | A launcher build started from that PATH, to completion; nothing above cmd's own ceiling can be fixed in the script |
| #92 Graphics GPU label overlap | Verified | The GPU row stacks the combo and its note in a top-down block constrained to the control column, so the note wraps; "Detected", "No Vulkan 1.3 device" and "Looking for Vulkan devices..." all use it | None for the row. Viewed at 200 % scaling with a three-adapter hybrid list, with no Vulkan device, and with a long selected adapter name (2026-11-24 captures below) |
| #7 Linux / Steam Deck | Outstanding - platform layer builds | POSIX platform layer, `write_watch_linux.cpp` compiles and links, platform self-test passes on Ubuntu 24.04, Vulkan probe runs | The rest of the runtime: core still links `psapi`, `diag/registry.cpp` includes `<windows.h>`, the graphics shell is Win32. `docs/LINUX.md` is missing (see `LINUX_BUILD_STATUS.md`) |

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

### #90 - long inherited PATH

`toolchain::resolve()` (PR #91) stopped giving the build the inherited PATH twice. This pass removes the other
half of the failure: the launcher already runs `vcvars64.bat` once and passes its INCLUDE, LIB and PATH, so it now
sets `NX_ENV_READY` and `nx_env.cmd` keeps that environment instead of running `vswhere`/`vcvars64.bat` again
(about 2 s per call, and the PATH it appended was what crossed the 8191-character line). A direct `nx_env.cmd`
still runs `vcvars64.bat`, but with a short PATH; the caller's PATH is put back and both are merged through
delayed expansion, so no line expands past the limit.

The check drives a 7,906-character PATH through both modes (`cmd /c exit 0`, so nothing else from the toolchain
is needed), and the same check run against the previous `nx_env.cmd` fails both calls with `The input line is too`
`long.` - the regression is covered, not assumed. A full launcher build from a PATH that long has not been re-run
in this pass. Above roughly 8,191 characters cmd stops delivering PATH to a child at all; that ceiling is
Windows', not the script's.

### #92 - Graphics page GPU label

`widgets::setting_row` lays its control out right to left, so the GPU row's "Detected" label was placed beside the
280 px combo in whatever width was left, sometimes none, and ran left over the description. The row now puts the
combo (still right-aligned) and the note in a top-down block constrained to the control column, where the note
wraps; the empty-list and waiting lines use the same block.

Verified on 2026-11-24 with the launcher's own capture path at `--size 1280x720 --scale 2` (a 2560x1440 frame,
the logical layout of a 200 % display) and `--scroll 2050`:

- the three-adapter list `AMD Radeon(TM) 890M Graphics` / `NVIDIA GeForce RTX 4090 Laptop GPU` / `MuMuPlayer
  Virtual Display Adapter` wrapped "Detected: ..." to two lines under the combo, inside the control column, with
  no overlap on the description and no clipping at the right or bottom edge;
- `--gpu-list none` wrapped the no-Vulkan-device line to two lines in the same place, also without overlap or
  clipping;
- with `graphics.gpu` set to the long adapter name, the combo widened to show the whole name and the note stayed
  below it.

The same rows were captured at `--size 1120x720 --scale 2` (the minimum logical width) with the same result. The
`--scale` option sets egui's pixels-per-point for the capture; it does not change the Win32 DPI the custom title
bar's hit test reads, which the GPU row does not use. The full images are in the ignored `target/capture/`
directory and are not committed.

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
| #79 | A reporter's own save, listed and loaded (the fixture matrix is done; row 24's UI flow is still open) |
| #90 | A launcher build started from a PATH near 8,191 characters, to completion |
| #92 | **Met 2026-11-24**: Graphics page at 200 % scaling with a three-adapter hybrid list, with no Vulkan device, and with a long selected adapter; note wrapped without overlap or clipping |
| #7 | A native Linux build of `nxrt` + `nxrt_gfx`, then a runtime test; Proton and Gamescope as separate targets |

---

## 2026-11-24 integration record

Landed on `main` after the FCC pass, in addition to the review and merge of PRs #81, #83, #85, #87, #89 and #91:

| Commit | Change | Validation |
|---|---|---|
| `99034aee` | `NX_ENV_READY` handshake, short-PATH vcvars, long-PATH cases in the script check (issue #90) | `cargo test -p emblemweave`: 432 passed, 13 ignored; `check_cmd_scripts.ps1`: 5 PASS; the same check against the previous script fails as expected |
| `fb0c9f25` | GPU row note wraps inside the control column (issue #92) | `cargo check -p emblemweave`; not viewed in the running launcher |
| `c5d2bbf7` | Credits for mmkzer0 (PRs #81, #83, #85, #87, #89, #91) | README and About page read back |
| `8a4e5d9d` | `package_release.ps1` runs the cmd script check before building or packing | PowerShell parser check; the added command run directly (exit 0) |

## 2026-11-24 GPU-row capture pass

The launcher's `--screenshot` path now takes three capture-only options, so the #92 conditions can be reproduced
without a second machine or a real 200 % display:

- `--scale FACTOR` sets egui's zoom for the run and sizes the window by the same factor, which is the logical
  layout a display at that scale produces (`--size 1280x720 --scale 2` is a 2560x1440 frame at 200 %).
- `--scroll POINTS` opens the page at that offset, so a row below the first screenful is captured without a
  window taller than the display.
- `--gpu-list "NAME|KIND|VRAM_MB;..."` supplies the device list the row draws (`none` for an empty list). It is
  set before the first `cached()` call and never changes the real Vulkan scan or the saved settings.

Validation: `cargo build -p emblemweave --release` succeeded; `cargo test -p emblemweave` -> 432 passed, 0
failed, 13 ignored. The captures are `target/capture/gpu200-hybrid-2050.png`, `gpu200-none-2050.png` and
`gpu200-hybrid-selected.png` (2560x1440). Not checked: a physical display switched to 200 %, and the
"Looking for Vulkan devices..." frame (the waiting line shares the same block).

## 2026-11-24 engine pass: unclaimed read faults and the freeze watchdog

Found by running the whole engine suite (`nxrt_tests.exe --all`) instead of one group at a time: 7 of 987 tests
failed, and six of them only when an earlier group had created a write-watch region.

- **Read faults were still being rescued.** `e9c0ccc9` removed the four write-fault fallbacks that made foreign
  or decommitted pages writable, but the read path kept two of the same: a read fault inside a live write-watch
  region, or anywhere in the `[kHeapBase, kHeapBase + kHeapMax)` window, turned the page read-write and retried
  the read. A process that had created one NVN pool region therefore never saw a guest read of decommitted or
  guest-no-access memory fault: host protection silently disagreed with the page table (`heap_decommit`,
  `svcSetMemoryPermission`, stack guards) and invalid reads were hidden. The unbounded rescues are gone; an
  active `ImportFreeze` still restores the page it froze, the only case bounded by an owner.
- **A scripted run's freeze watchdog outlived the run.** `tt::init` starts a watchdog with `on_fire` set when
  `[test]` asks a freeze to end the run (exit code 5); nothing stopped it at `end_run` or in `tt::shutdown`.
  In a long-lived process (the test binary, a launcher that keeps the runtime loaded) the leftover watchdog then
  fired on the idle process. `tt` now records that it started the watchdog and stops it in `tt::shutdown`, and
  `watchdog_stop` detaches instead of joining when it runs on the watchdog thread itself, so the freeze path
  (`on_fire` -> `end_run` -> shutdown) cannot deadlock.
- **The shared-font empty case depended on the working directory.** `svc_pl.pl_no_font_found_still_answers`
  asserts no DejaVu is found, but the lookup also searches `assets/fonts` under the current directory, so
  running the suite from `runtime/` found the repository font; it passed only from `build/release`. The case now
  runs from its empty temporary folder (restoring the old one afterwards), so it means the same thing everywhere.

Validation: `powershell -File scripts/t.ps1 -All` -> `SUMMARY --all: 958 passed, 0 failed, 29 skipped (of 987)`
(before: 951 passed, 7 failed). `svc_pl` 10/10 from `runtime/` and from a neutral directory; `memory` 27/27,
`svc_memory` 21/21, `crash` 15/15, `write_watch` 17/17, `watchdog` 12/12 standalone; `tt_live,watchdog` 14/14
(was 13/14). Not verified: the read-fault change under real gameplay (no game dump or long session was
available). It is covered by the memory and svc_memory groups, which fault on decommitted, unmapped and
guest-no-access pages with a live write-watch region in the same process.

## 2026-11-24 stash review: audio EQ, play UI and docs

`stash@{0}` (`8de48bfa`, base `d1964c64`, 45 tracked files at +1,412/−107 plus 11 untracked) was
compared with `main` at `4a76488c`, not applied. Its base is 168 commits old; most of it has since
landed in a newer form. The stash remains at `stash@{0}`.

Already on `main`, newer than the stash:

- **Audio EQ.** `audio_eq.h/cpp` and `test_audio_eq.cpp` are present with ten bands, preamp,
  automatic headroom, a limiter, eight presets (including Headphones Boost) and a live F1 panel;
  the launcher writes `[audio] eq*`. `svc_audio_eq`: 7 passed, 0 failed.
- **Play UI / master boost.** The Audio page and F1 panel apply 0–200% immediately and describe the boost as limited.
  The stash's warning gate is not used: `Confirm::AudioBoost` is never constructed, and the modal
  in `play_audio_ui.cpp` cannot open because `boost_warning` is never set. The dead modal is a
  separate cleanup, not an integration point.
- **Drag handling.** `platform.rs` suppresses `WM_PAINT` during `WM_MOVING` and handles native
  drag; the stash's `raw-window-handle` subclass adds nothing. Issue #59 still needs the reporter's
  machine.
- **MAKO, build jobs, release tooling.** MAKO is byte-identical; build concurrency goes through
  `ToolchainManager::calculate_jobs` (the stash's `build_jobs` helper is an unused duplicate);
  `check_release.py` and the workflow on `main` have the published/development split.
- **Docs.** The MAKO, EQ, build-job and release text is present. Two stale audio statements were
  corrected here: the above-100% boost applies immediately, and the preset list now includes
  Headphones Boost (`STATUS.md`, `USAGE.md`, runtime `CONFIG.md`).

Left out:

| Piece | Why |
|---|---|
| Overview "At a glance" / `overview_details` card | From the retired 1,864-line landing page. Current `overview.rs` is a 495-line redesign with Next steps and Recent activity; the card duplicates both and was not visually verified. |
| Above-100% warning gate and `Confirm::AudioBoost(u32)` | Superseded by immediate limited boost; the current variant is dead and the docs now say so. |
| `throttle_drag_paints` and the `raw-window-handle` dependency | Superseded by `platform.rs`'s `WM_MOVING` paint suppression. |
| Older 7-preset EQ tables and old `play_audio_ui` | Superseded by the eight-preset/preamp implementation and shared UI on `main`. |
| `docs/RELEASE-LEDGER.md` 0.1.40 note | Stale: the ledger now points at 0.1.41 published and 0.1.42 in development. |
| `build-runtime-111.log` | Build artifact. |

Validation: `cargo test -p emblemweave` -> 432 passed, 0 failed, 13 ignored;
`powershell -File scripts/t.ps1 svc_audio_eq` -> 7 passed, 0 failed. Not verified: in-game audio
(no game dump or long session), the retired Overview card (not integrated), and the 200% boost on
physical speakers or headphones.

## 2026-11-24 issue #59 re-measurement

The reporter's caption drag did not reproduce on this host: a 3.25 s synthetic drag moved the
window 220 px with 0 px sampled lag, 1.92 % of one core, and 0 % CPU after it settled. The native
`WM_PAINT` guard is unchanged. The first window appeared in 392-969 ms; steady idle was 0 % once
the first ~10 s of startup work and toasts finished.

What did reproduce was per-frame work on the Overview. The page re-read save profiles, every mod
tree (including each mod's RomFS/ExeFS) and the cheats JSON on every frame. A pointer sweep cost
39.8 % of one core on Overview versus 23.5 % on the static Appearance page.

`quick_actions` now caches that display state for one second. Three post-fix sweeps measured
26.8 %, 25.7 % and 21.1 % (mean 24.5 %); startup CPU through the first nine seconds fell from
1.92 s to 1.00 s; the same drag stayed at 0 px lag (4.9 % CPU in the post-fix run, inside the
prior 1.5-6.8 % range). `cargo test -p emblemweave`: 432 passed, 0 failed, 13 ignored.

Not verified: the reporter's Windows 10/11 hardware, physical mouse versus synthetic input, and
multi-monitor/high-DPI drag. The fix targets frames that do run, not idle repaint.

## 2026-11-24 runtime lifecycle, partial initialization and freeze-dump audit

This pass traced `runtime::init()` / `runtime::shutdown()` and the workers they start. The summary is
code-level and test-level evidence; no game dump or long gameplay session was available on this host.

### What was wrong

- `shutdown()` only ran the worker teardown when initialization had succeeded. A failed address-space
  reservation (`mem::init()`) still started the watchdog, memory guard, starvation guard, sampler and frame
  trace; `shutdown()` returned before stopping them, and a later init could not restart the trace.
- The frame-trace writer and stall sampler were never stopped by `runtime::shutdown()`. Their threads and the
  event buffer survived the runtime, and a restarted trace could reuse stale per-thread trace indices.
- The audio stats thread was process-static and had no stop path; the audio renderer/audout pacing threads,
  HID sampler and VI vsync thread are service-owned and were not reset by runtime shutdown.
- `watchdog_stop()` held the watchdog control lock while joining. A freeze callback that called
  `watchdog_stop()` (directly or through shutdown) could deadlock against a caller doing the same.
- A freeze dump suspended, read and resumed one thread at a time. `SuspendThread` is asynchronous, so each
  `GetThreadContext` waited for its own thread to reach a suspendable point; those waits serialized.

### What changed

- `g_up` still means "initialization succeeded", not "anything started". `g_init_started` means init was
  attempted, and `g_services_registered` records whether the service layer was installed. Shutdown always
  runs after an attempt, stops every runtime worker, resets the service-owned workers, and resets the IPC
  registry so a later init registers services from a clean state. The summary is still printed only when
  `g_up` is true.
- `ft::stop()` joins the writer and sampler, waits for `event()` writers already inside the buffer, flushes
  and frees the buffer, resets counters and thread-name state, and permits `start_from_config()` again.
  `ft::running()` makes the state observable to tests.
- `watchdog_stop()` moves the thread handle out under the control lock and joins outside it. `watchdog_start()`
  waits for a detached generation without holding the control lock, re-checks after the wait, and refuses to
  restart from the dying generation. The existing self-detach path remains for callback-driven shutdown.
- `runtime::shutdown()` calls `services::reset_all()` (audren DSP, audout pacing, HID sampler, VI vsync, WAV
  tee, service state) and `ipc::registry_reset()`, then stops the audio stats thread with
  `audio::stop_periodic_stats()`.
- Freeze dumps suspend every registered thread before reading any context, then read contexts/stacks, then
  resume every thread. Formatting still runs only after all threads are running again. Storage and registry
  snapshots are allocated before the first suspension, and no allocation or logging occurs while suspended.

### Regression tests

`runtime_lifecycle` now covers: failed-memory init teardown, repeated init/shutdown, reinit with services
enabled, service reset, frame-trace stop/restart with active `event()` writers, six concurrent shutdown
callers, a watchdog callback that shuts the runtime down and a later reinit, host thread-count return to the
pre-init count, and the audio stats thread stopping. `watchdog` adds a child-process test where a caller and
the freeze callback both call `watchdog_stop()`; the child fails by timeout if the old lock inversion returns.

### Freeze-dump baseline

Measured with `nxrt_tests watchdog` on this Windows host, three dumps per thread count, registered busy-spin
threads (a worst case, not a game):

| Registered threads | Before batching | After batching | After: collect | After: format |
|---:|---:|---:|---:|---:|
| 1 | 0.07-0.11 ms | 0.07-0.10 ms | 0.05-0.08 ms | 0.02 ms |
| 8 | 0.31-0.56 ms | 0.25-0.31 ms | 0.19-0.25 ms | 0.05-0.06 ms |
| 32 | 194-204 ms | 18.6-33.7 ms | 18.4-33.5 ms | 0.12-0.19 ms |

The 32-thread before figure was almost entirely `GetThreadContext` (194-204 ms); after batching it is 0.6-0.7 ms,
and the remaining interruption is `ResumeThread` wake-up (17.5-32.7 ms, host-load dependent). Dump content and symbolization are
unchanged; the existing `thread_dump_lists_registered_threads_not_caller`, `suspended_threads_keep_running`,
`dump_names_symbolised_host_function` and request-file tests pass.

### Validation and limitations

- Full suite: `SUMMARY --all: 967 passed, 0 failed, 29 skipped (of 996)`. Standalone: `runtime_lifecycle`
  6/6, `watchdog` 14/14, `tt_core.watchdog_shutdown_from_on_fire_restarts` 1/1, `runtime_e2e` 1/1, `memguard`
  8/8, `soak` 9/9, `svc_audio_out` 10/10, `write_watch` 17/17.
- Only `mem::init()` has a failure return in the init sequence; the other stages are best-effort starts with
  no failure state to inject. The actual memory-failure path is covered, and each worker's own stop/restart
  path is covered separately. No artificial stage-failure hook was added.
- `services::reset_all()` assumes guest execution has stopped. Callback-driven runtime shutdown with active
  guest threads is not exercised; the watchdog callback test uses the core runtime without services.
- The graphics backend and movie decoders are not owned by `nxrt`: the app calls `nvn::shutdown_backend()`
  (as the graphics test app does), and decoder threads live on guest objects. `fefw_main` relies on process
  exit after the entry point returns. Real return-to-title, scene recreation and extended gameplay remain
  unverified on this host.
- The uncommitted `test_write_watch.cpp` additions were preserved and their 17-case group passed; they are not
  part of this audit's commit.
