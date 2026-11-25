# Issue Remediation Status

**Date:** 2026-11-25
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
| #77 Return-to-title crash / damaged autosave | Implemented - 10-cycle gameplay run, native-zstd checksum regression | Fault-handler initialisation and fallbacks; a current working-tree build completed 10 gameplay -> title -> gameplay cycles (31,301 frames, 456.7 s, 0 error lines, 21 screenshots); a damaged `Auto` is moved to backups before Play when the slot checksums or native zstd reject it | The cause of the damaged `Auto` is unknown; the game's own path for damage found mid-session is unchanged; the run used a valid save and a working-tree build, and the reporter's damaged save was not available |
| #78 HDR left enabled after exit | Implemented - verified on one display | Full display snapshot (mode, HDR, SDR white, per monitor), sidecar snapshot for crash recovery, restore of only what the session changed, `SET_HDR_STATE`, 18 tests (13 fake, 5 live) | Live on Windows 11 26100 / RX 7900 XT, one 2560x1440@180 display: marked exclusive entered and exited normally, HDR-off stayed off, HDR-on stayed on, forcing HDR off during exclusive restored it, and a hard-killed marked session was repaired from the sidecar on the next launch. Remaining: SDR white is read-only (captured, not written), no second monitor, no live differing-mode restore, no live second restore of the same snapshot. |
| #79 Saves not read | Implemented - fixture matrix run, reporter case open | R1-R8 reconciliation, adoption, diagnostics, empty-save guard, fixture harness with refusal/manifest tests | Rows 16, 17, 19, 20 and 22 ran against the real engine: stale absolute paths honoured, nested save not adopted, missing `.meta` regenerated, foreign account ignored. Row 24's launcher import/backup/restore round-trip ran in a portable copy; its empty-profile UI flow and a reporter's own save remain (see `SAVE_VALIDATION_ISSUE79.md`) |
| #90 Long inherited PATH stops the build | Implemented - automation-verified | `engine.rs` sets `NX_ENV_READY`, so `nx_env.cmd` keeps the launcher's vcvars environment instead of running it again; a direct call runs `vcvars64.bat` with a short PATH and merges both afterwards; every long value is only named through delayed expansion. `check_cmd_scripts.ps1` covers a 7,906-character PATH in launcher and direct mode, and `package_release.ps1` runs that check before packing | A launcher build started from that PATH, to completion; nothing above cmd's own ceiling can be fixed in the script |
| #92 Graphics GPU label overlap | Verified | The GPU row stacks the combo and its note in a top-down block constrained to the control column, so the note wraps; "Detected", "No Vulkan 1.3 device" and "Looking for Vulkan devices..." all use it | None for the row. Viewed at 200 % scaling with a three-adapter hybrid list, with no Vulkan device, and with a long selected adapter name (2026-11-24 captures below); the 2026-11-25 pass re-inspected the captures and found no change to the row |
| #94 Package overlay markers counted as game files | Implemented - automation-verified | `make_layout()` drops `.source` and `.written` from the root of a package overlay only; existing overlays are not rewritten | The regression runs in `nxrt_tests`; no full game launch was needed or run |
| #97 Shader cache not reused | Implemented - automation-verified | `5e590bd4` removed the unused `ShaderCache`, put the translator build ID back in the journal header, and made `StartOver` delete the journal, every `.spv` and the preload pack | The 2026-11-24 harness smoke run covered header, resume and stale-file deletion; the full 3000-block corpus was not rerun in this pass |
| #101 Cutscenes fail without NASM | Implemented - automation-verified | NASM is `Need::Always` in `deps.rs`, `tools.lock` marks it required, and the setup, requirements and troubleshooting docs say so | No engine build or assembly-versus-C decode measurement was made in this pass; the launcher suite covers the dependency table |
| #104 GPU decoding | Deferred - enhancement | dav1d remains the default; FFmpeg/D3D11VA remains explicit; PR #105 is unmerged and needs a released FFmpeg 9.1 build | Engine-level evidence for a default change; isolated decoder benchmarks are not enough |
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
That is a build fact, but the opening movie has since been played on `cb2755cc` from the complete X: package (boot check
below): 1205-1655 pictures at the movie's own 30 frames/s behind a sustained 60 fps present loop, 0 errors. It was not
played to its end, and A/V sync, Opus output, a whole movie and a NASM-versus-C comparison are still unmeasured. The
FFmpeg/D3D11VA path is not part of that run.

### #77 — return to title

The earlier 120-second FCC session is no longer the only runtime evidence. A preserved scripted run from
2026-11-24 05:30 (`_recovery/issue77_live/out_cycle10`) completed 10 gameplay -> title -> gameplay cycles:
**35,144 frames, 448.9 s, verdict pass, 21 screenshots, exit 0**. Its log records the paths and the script but not
a commit; it ran after `e9c0ccc9` and before the later read-fault change. The current working-tree engine binary
(`fefw.exe`, 169,846,272 bytes, built 2026-11-24 19:03, SHA-256
`8006801906240415d912055bbb8aaa3d5cb5b2f49d2e837dd75f057d68175fae`, built over `730fc46c` with the preserved
uncommitted movie changes) was run through the same 10-cycle script on 2026-11-24 21:30:
**31,301 frames, 456.7 s, verdict pass, 21 screenshots, exit 0, 0 error lines, 0 guest traps, 0 zstd errors**. The
screenshots show the title screen and the game scene at every cycle. This is real engine playback of the reported
workflow, not a synthetic unit test. The build is not a clean archive, and the reporter's damaged save was not
available, so the exact trigger is still not reproduced.

The 2026-11-25 pass added a separate, narrower guard for the damaged-autosave path in the report. The reporter's
`Auto` was damaged (`ZSTD_error_corruption_detected`, error -20) and the guest copied the decoder's error code
(`0xffffffffffffffec`) as a copy length. Before Play starts the game, `quarantine_damaged_autosave()` opens the
`Auto` slot the game will read. If parsing or verification fails, it moves the file byte for byte into
`save/backups/<stamp>-damaged-autosave-Auto`, writes a `.why.txt` naming the reason, and stops the launch with an
explanation. A second Play starts without the bad autosave. A valid autosave is untouched, and nothing is deleted.
`Global` is deliberately not checked: it has a different header and is not a slot.

The guard also decodes the frame with `zstd::bulk::decompress`, the same library and one-shot semantics as the
game's native hook. This closes a real gap: `ruzstd` accepts a frame whose frame checksum is corrupt while its
payload still decodes, but native zstd rejects it. The new regression test builds such a frame, shows that the
payload decodes and the slot's own FNV checksums pass, shows that native zstd rejects it, and shows that the guard
moves it to backups. The native output is required to match the launcher's payload, and a payload larger than the
game's 64 MiB save buffer is rejected before allocation.

This prevents the next launch from repeating the crash, but it does not repair the frame, identify how it was
damaged, or protect a session in which the corruption appears after Play. The reporter's save was not in either
bundle, so the guard is tested with synthetic truncated and checksum-damaged frames rather than their file.

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
| #77 | 10+ return-to-title cycles in one session with no fault; a reporter's damaged save through the new guard |
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

## 2026-11-24 dump-suspension safety, overlay markers and pending-contribution review

Baseline `cbfac50c` on `main` (equal to `origin/main`). The only working-tree change at the start was the pre-existing
`runtime/tests/test_write_watch.cpp` edit, which is preserved and not part of this pass's commit; `stash@{0}`, `_recovery/`
and the detached `target/issue79/clean` checkout are untouched.

### Two freeze dumps could deadlock each other (fixed)

`collect_threads()` suspends every registered thread before it reads any context. Nothing serialised dumpers, and
`diag_thread_dump()` has four independent callers: the freeze watchdog, the `[prof] stall_ms` sampler
(`src/diag/ftrace.cpp`), the Vulkan device-lost report (`src/gfx/vk_renderer.cpp`) and the fault report
(`src/services/logfault.cpp`). Two **registered** threads that dump at the same time each suspend the other, and each
resume loop lives in the thread the other one suspended, so neither ever runs again. The watchdog and the stall sampler
both react to the same "no frame tick" event, so this is a live race, not a theoretical one.

- One `std::recursive_mutex` now serialises the collection phase (recursive so an `on_fire` callback can dump again from
  the watchdog thread). The suspended window needs nothing another thread owns - `GetThreadContext`,
  `RtlVirtualUnwind`, `ResumeThread` and `CloseHandle` take no user lock and allocate nothing - so serialising cannot
  deadlock against the threads it suspends.
- `ResumeSuspended` releases every suspension on scope exit, including an early return or an exception. Previously the
  resume loop was straight-line code with no structural guarantee.
- `ThreadDumpTiming` gained `suspended` and `resumed` counters, so a dump can be checked for "every suspension undone"
  instead of inferred from thread behaviour.

Evidence: `watchdog.concurrent_dumps_do_not_deadlock` runs a child process with 6 registered busy workers and 4
registered dumper threads, 25 dumps each, and fails by timeout if the child does not exit. With the lock line removed
the child never exits (`finished concurrent thread dumps deadlocked (child did not exit)`); with the lock it passes. The
defect was reproduced, not inferred.

New coverage in `watchdog` (19 cases, all passing): `every_suspension_is_released_and_accounted` (5 dumps over 4 busy
workers, `resumed == suspended`), `dump_preserves_a_threads_own_suspend_count` (a thread that suspended itself once must
still be suspended exactly once after a dump; `ResumeThread` must return 1), `dump_survives_threads_leaving_the_registry_mid_dump`
(25 dumps while threads register and unregister continuously), `concurrent_dumps_do_not_deadlock`, and
`a_thread_that_cannot_be_suspended_is_reported` (a registered thread that exited without unregistering: its duplicated
handle stays valid, `SuspendThread` fails, the dump reports `(could not suspend)` and resumes nothing).

The 32-busy-thread synthetic benchmark (a worst case, not a game) measured 27.95 ms and 35.00 ms total in two full-suite
runs, of which `ResumeThread` wake-up was 26.6 ms and 33.8 ms; context retrieval stayed at 0.74-0.83 ms and formatting at
0.19 ms. The earlier 18.6-33.7 ms range still holds; the lock adds no measurable cost.

Not verified: a real freeze on game hardware. Threads that hold locks the dump path needs, allocation while suspended, and
suspend-count behaviour under a debugger remain reasoned about (the window allocates nothing and takes no user lock) but
are exercised only by these synthetic cases.

### Package overlay markers are no longer game files (#94, fixed)

The launcher writes `.source` and `.written` into the root of the package overlay (`OVERLAY_SOURCE` / `OVERLAY_WRITTEN`
in `apps/emblemweave/src/gamedata.rs`), and the runtime served the overlay whole, so a complete install counted two files
more than `[fs] romfs_files` and warned about incomplete game data on every start.

`make_layout()` now drops those two names from the root of a **package** overlay only (an installed mod may ship a file of
that name on purpose). Nothing is rewritten on disk, so existing 2 GB overlays are not rebuilt and user data is untouched.
New case `svc_fs_romfs.romfs_package_overlay_markers_are_not_game_files` checks the file count, the image contents, that
neither marker is resolvable in the image, and that a mod's `.source` is kept.

### Partial-initialization audit

| Stage | Can fail? | Cleanup on shutdown | Finding |
|---|---|---|---|
| `config_load_file` | no (missing file is fine) | none needed (static map) | repeated init reloads over the same map |
| `log_init` | no; falls back to stderr | `log_flush` only | `apply()` closes the previous file first, so cycles do not leak a handle; `log_shutdown()` is not called and the file stays open by design |
| `symbols_load_map` | no | none | static map, refilled per init |
| `crash_install` | no | none | installed once per process |
| `mem::init` | **yes** | `mem::shutdown()` is *not* called by `runtime::shutdown()` | the failed path is covered; the reservation deliberately survives so a later init reuses it |
| `svc_init`, kernel locks, spin hint, starvation guard | no | guard stopped | static state |
| `services::register_all` | only on `bad_alloc` | `reset_all()` + `ipc::registry_reset()` behind `g_services_registered` | see residual 1 |
| media/HLE installs, trace sites | no | not undone | hooks are process-wide; re-registering overwrites by name |
| `memguard_start`, `watchdog_start` | no | stopped | both refuse a second start while running |
| `hitch_start`, `soak_start`, `prof_sampler_start`, `ft::start_from_config` | no (no-op unless configured) | stopped | `ft::stop()` is restartable and waits for active `event()` writers |
| `tt::init`, `timestep::init` | no | shut down | |

Resource accounting: `reinitializes_after_shutdown` now also compares `GetProcessHandleCount` across repeated
init/shutdown cycles, not just the host thread count. Both return to the pre-cycle value, so no thread handle, log file or
profile handle survives a cycle.

Residual findings, deliberately not changed:

1. `g_services_registered` is set *after* `services::register_all()`. A throw from the registration loop (`bad_alloc`
   only) would skip the service reset. Moving the flag before the call would make cleanup unconditional; it was left as
   is because a failed allocation leaves the process unable to continue anyway, and because every `*_reset()` would then
   run against services that were never registered.
2. `services::reset_all()` assumes guest execution has stopped. That holds at the only production call site:
   `fefw_main` calls `nxrt::shutdown()` after the lifted entry function returns, and the process exits immediately after.
   It is **not** enforced for threads the game created, which can outlive the entry point. Do not add joins or termination
   there: guest threads may hold the heap, log or GPU locks, and the process is about to exit. `reset_all()` is really a
   re-init facility, and the callback-driven `on_fire` path (`tt_core`) is only used by scripted runs.
3. `init()` retried after a failed `init()` without an intervening `shutdown()` is not supported or tested. The contract is
   that `shutdown()` follows every attempt; `register_service` overwrites by name, so a retry would not duplicate
   factories, but the best-effort starts would run a second time.

### Contributions merged (2026-11-24)

- **#96** merged as `b4a0d20d`: `ComputeKeyHash` uses the static `SHA256.HashData` instead of one shared `SHA256`, which
  threw when the worker's `Parallel.ForEach` hashed concurrently and took the worker down. This is the fix for #95.
- **#103** merged as `28403a60`: the frame handoff moved into `vdec::start_worker/stop_worker/register_frames/
  decode_async/acquire/release/flush` and `movie_vdec_internal.h`; `release()` wakes the worker when a frame waits for
  its GPU serial while pictures wait. Two tests drive the handoff against a 120-picture 64x64 AV1 clip:
  `released_frame_takes_the_waiting_picture_once_the_gpu_passed_it` and `player_threads_get_every_picture_once_and_in_order`.
  `media_vdec` 9/9.
- **#99** merged as `3f316bde` after resolving its conflict onto #103's refactor: the 30 s game-stall window (10 presented
  frames per 5 s marks the game as running), `eos_owed` for a stop that found no free frame, per-player timestamps in the
  stall log, and the widened stall message now live in `vdec::release`, `vdec::flush`, `abort_stalled` and
  `wait_for_room`. `release()` keeps both wake reasons (pending GPU plus decoded, or `eos_owed`), and `flush` clears it.
  Two handoff tests were added after the merge for the owed-EOS path (`an_owed_eos_rides_the_next_released_frame`,
  `flush_drops_an_owed_eos`). **Limitation:** the 5 s/30 s hitch window itself still has no test, and nothing here
  demonstrates a real game hitch.
- **#93** not merged: main already carries the #92 fix (`fb0c9f25`, captured at 200 % by `4a76488c`) with a top-down
  block that constrains the combo and its note to the control column; #93 patches the older layout and conflicts.
- **#105** remains open and is not merged. It is stacked on #103 and changes the default to `movie_decoder = auto`; AMD and
  Intel are gated on FFmpeg >= 9.1 (the shipped 8.1-dev build fails Vulkan decode on AMD with
  `VK_ERROR_OUT_OF_DEVICE_MEMORY`), and it removes `movie_hwaccel` / FFmpeg's libdav1d path. Wait for a released 9.1 build
  and in-game evidence before changing the default.
- **#97** fixed in `5e590bd4`: the content-addressable `ShaderCache` is removed rather than repaired with `IncludeFields`
  (a working index would carry every SPIR-V byte and be rewritten after every block); the translator build ID is back in
  the journal header and `StartOver` deletes the journal, every `.spv` and the preload pack when it changes. The harness
  builds with 0 warnings/errors; a smoke run wrote `# spirv_harness journal  2  <build-id>`, resumed without reset, and
  deleted a stale `.spv` when given a header naming another build. A full 3000-block corpus run was not repeated here.
- **#101** fixed in `5e590bd4`: NASM is `Need::Always` in the launcher, so a setup without it waits for installation like
  CMake and Ninja; the note, `docs/REQUIREMENTS.md`, `docs/TROUBLESHOOTING.md`, `docs/INSTALL.md` and `tools.lock` now say
  required. `cargo test -p emblemweave --locked`: 432 passed, 0 failed, 13 ignored. No assembly-versus-C decode
  measurement was repeated on this host (NASM is not installed here) and no engine build was re-run.
- **Launcher CI**: run 45492974436 failed `skip-baseline` because `EXPECTED_CASES` was still 68 while the suite had gained
  `launcher_game_info.summary_warns_when_cinematic_assets_are_missing` (#70/#75). Fixed in `c9cb7c9b` (69); this was a
  stale pinned count, not a failing case. The other red job, `Launcher Rust tests`, failed in `test_prepare_build_env`:
  it calls `toolchain::init()`, so a runner whose clang is 20.1.8 fails the launcher's 23 minimum before the assertion.
  Fixed in `65686213` by splitting the variables the manager owns into `build_env_extras()` and testing those; the host
  probe keeps its own path. `cargo test`: 432 passed; the workflow is green on `65686213`.

### Build dependencies (#101) and GPU decoding (#104)

- **#101**: the launcher now requires NASM (`Need::Always`, fixed in `5e590bd4`). The issue's measurements (19-54 ms per
  1440p packet C-only versus 5.5-14 ms with assembly, against a 33 ms frame budget) agree with the existing #70/#75
  records. The runtime still builds C-only when configured outside the launcher and logs that, but a normal setup can no
  longer reach that state.
- **#104**: the production path today is the built-in dav1d by default, with `[media] movie_decoder = ffmpeg` +
  `movie_hwaccel` as opt-in D3D11VA on whichever adapter Windows picks, falling back to FFmpeg's libdav1d outside the
  engine's thread-priority policy. PR #105's design (Vulkan decode on the renderer's device, version gate, dav1d fallback,
  parity evidence over all 35,511 frames) addresses device selection, fallback and compatibility, but not distribution
  (the 9.1 ABI change) or the untested macOS path. Defaults stay with software here.

### Validation

- `powershell -NoProfile -ExecutionPolicy Bypass -File scripts/t.ps1 -All` from `engine/fefw-recomp/runtime`, Release,
  clang-cl 20.1.8: `SUMMARY --all: 980 passed, 0 failed, 29 skipped (of 1009)` after #103/#99 and the two owed-EOS tests.
- Targeted: `watchdog` 19/19, `runtime_lifecycle` 6/6, `svc_fs_romfs` 21/21.
- Synthetic measurements only, apart from the `cb2755cc` boot check recorded at the end of this document (a real
  package and opening movie; no gameplay, no clean shutdown). Return-to-title, scene recreation, extended-session
  behaviour, the combined #99/#103 semantics and the GPU decode path in game remain unverified.

## 2026-11-24 boot check: cb2755cc built and launched on the X: package

This records a binary built from `main` and launched: commit `cb2755cc4f1c557bd6e4e1c854a5d602e2cbdc59` ("fix(diag):
serialise thread dumps and keep launcher markers out of RomFS", 2026-11-24 16:37 -05:00), `HEAD` equal to `origin/main`.
The earlier statements that no `fefw.exe` was built from `main` and that no game was available no longer apply. The run
is the opening movie, not gameplay; its limits are listed at the end.

### Build

```
cd engine/fefw-recomp
powershell -NoProfile -ExecutionPolicy Bypass -File game-build/scripts/build_fefw.ps1 -Config Release
```

Release, clang-cl 20.1.8. Output `game-build/fefw/build/release/fefw.exe`, 169,840,640 bytes, whose `NXRT_BUILD_COMMIT`
string read `cb2755cc4f`. The only working-tree change was the pre-existing `runtime/tests/test_write_watch.cpp` edit,
which is not part of the executable. The first attempt stopped at 210/220 and was resumed to a clean link; the binary
checked is from that completed link. The exe has been rebuilt since (the uncommitted av1C parse fix), so the size and
build string describe the artifact as it was run.

### Run

From the gitignored `_recovery/` directory, with the ini below and a copy of the launcher's `controls.ini`:

```
cd EmblemWeave/_recovery/boot-check
../../engine/fefw-recomp/game-build/fefw/build/release/fefw.exe > run.out 2>&1
```

| Setting | Value |
|---|---|
| `[fs] exefs` | `engine/fefw-recomp/game/dump/exefs` |
| `[fs] romfs` | `X:/FEFW/Fire Emblem꞉ Fortune's Weave + Update 1.1.1.nup` (preferred source: faster drive, file with update) |
| `[fs] save_dir`, `[fs] mods` | `_recovery/boot-check/save`, `_recovery/boot-check/mods` |
| `[nvn] shader_cache`, `bc7_cache` | `engine/fefw-recomp/work/shaders/spirv`, `work/bc7_cache` (warm) |
| `[fefw]` | `output = 1080p`, `fps60 = 1` |
| `[upscale] mode`, `[dlss] upscaler` | `off`, `auto` |
| `[diag] watchdog_seconds` | `60` |
| `[boot] max_seconds` | `75` |
| `[log]` | `path = fefw.log`, `level = info` |

`controls.ini`: windowed, 1280x720, vsync off, 60 fps limit, render scale 100 %, stereo audio, flat EQ. The package
mounted as `RomFS image from X:/FEFW/Fire Emblem꞉ Fortune's Weave + Update 1.1.1.nup: 252686 files, 21466 directories,
29037945728 bytes of data`.

Two runs, no input; the game walked to the opening movie on its own. Run 1 had no `mods/` directory; run 2 (the
cleaner one) ran after creating it. Both live in `_recovery/boot-check/`: `fefw_run1.log`, `fefw.log`, stderr in
`run.out`.

### Log counts

| Run | I | W | E | Opening movie |
|---|---:|---:|---:|---|
| 1 (no `mods/`) | 719 | 11 | **0** | player at 20.7 s; 1752 packets, 1655 pictures released by 75.8 s; decode 7.42 ms/packet |
| 2 (`mods/` present) | 717 | 8 | **0** | player at 34.5 s; 1302 packets, 1205 pictures released by 74.7 s; decode 12.28 ms/packet |

No error line in either log. Run 1's eleven warnings: three `mods: mods folder ...` lines for the missing directory,
one slow DSP frame (88.4 ms) and its follow-up, the `nvn.gpu` statistics-counter stub, four hitches (frame 2 at
89.8 ms, frame 8 at 1065.0 ms, frame 251 at 188.7 ms, frame 615 at 317.5 ms) and the `max_seconds` notice. Run 2's
eight: one recovered display snapshot, one slow DSP frame (79.0 ms) and its follow-up, the same `nvn.gpu` stub, three
hitches (frame 2 at 61.4 ms, frame 8 at 696.9 ms, frame 143 at 101.2 ms) and `max_seconds`. Creating `mods/` removed the
three folder warnings.

Run 1's stderr also carried one vendored-dav1d `Unknown OBU type 0 of size 12` as the decoder read the codec
configuration; playback was unaffected. The parse defect it exposed is fixed on main in `b5c7d4b7` (the av1C/OBU
change).

### Movie and frame evidence

- Run 2: `movie: stats` at 74.686 s reads packets 1302, pictures 1237, frames acquired 1208, released 1205, queue 64
  (2.8 MiB, max 64), feed waits 1237 (39988.6 ms). 1205 pictures over the 40.2 s the player ran is 30.0 pictures/s, the
  movie's own rate; `pictures - released` stays at the 32-picture pipeline (30 ahead plus two in flight), so nothing was
  dropped.
- Run 1: 1655 pictures over the 55.2 s from the player at 20.664 s to the last stat at 75.849 s, also 30.0/s; 1752
  packets, decode 7.42 ms/packet.
- `gfx: fps` (71 one-second samples) reported 60.0 on every steady second: min 60.0, mean 60.9, max 102.8 (the start-up
  burst before the limiter settles). Present pass GPU 0.032-0.033 ms.
- `xaudio2` ended at `underruns +0 (0), dropped +0, written 48023 frames/s (rate 48000)`. That is the audio sink's
  health; movie A/V sync was not measured (no audio capture, no reference playback).
- 1440p AV1 (`movie::VideoDecoder: 2560x1440`) on the production default path, the built-in dav1d: `AV1 through dav1d
  1.5.1 with x86-64 assembly (8 threads at above normal priority, 30 pictures ahead)`, with no `[media] movie_decoder`
  override in the ini.

### Limitation: both runs were killed at the deadline

Both runs ended at `[boot] max_seconds = 75`:

```
boot: max_seconds reached: the guest is still running; ending the process
```

exit code 124. The movie was still playing (queue full, packets still arriving) when the deadline hit, so this is a
start-up and playback record only:

- `runtime::shutdown()`, display/HDR restore and any post-entry teardown were not exercised;
- the movie was not played to its end, so #70/#75's end-to-end requirement and A/V sync remain open;
- no input was given and the game never returned to the title, so #77 is untouched;
- no scene after the opening movie ran, and there is no gameplay memory profile (#37/#41/#76/#79).

What it does establish: `main` builds, mounts the complete X: package (252,686 files), reaches the game's opening movie
without input, decodes 1440p AV1 at the movie's 30 frames per second behind a sustained 60 fps present loop with zero log
errors and zero audio underruns, for the ~55 s the window allowed.

### Follow-up: the deadline now runs the end-of-run teardown

That run ended with `TerminateProcess(124)` on the deadline thread, so `nxrt::shutdown()` was never reached. `deadline_thread`
now starts the same teardown the returned-entry path runs (`nxrt::shutdown()`, which logs the end-of-run summary and stops
the runtime's workers) on its own thread, waits up to 15 s for it, and only then ends the process with 124. A teardown that
cannot finish logs `the end-of-run teardown did not return within 15 s` instead of wedging the run.

Re-run on the same base (`cb2755cc` plus the uncommitted av1C and deadline changes), same build command and ini as above,
with a fresh `_recovery/teardown-check2/` save and `mods/`:

| | |
|---|---|
| deadline | `boot: max_seconds reached: the guest is still running; ending the run through the end-of-run teardown` at 76.512 s |
| summary | `runtime: end-of-run summary: svc: 448745 calls, 30 distinct, 0 unknown ...` at 76.581 s |
| stages | `timestep: end of run`, then `services: language en-US (from Windows)` |
| workers | `boot: end-of-run teardown returned: runtime down, host threads 97 -> 92` at 76.653 s |
| exit | 124, with no 15 s fallback line |
| log | 708 I, 7 W, **0 E**; opening movie at 11.6 s, 1805 pictures released by 71.8 s (30.0/s), `gfx: fps` mean 61.0 |

The five threads that go away are runtime-owned workers (the watchdog, guards and samplers `nxrt::shutdown()` stops); the
game's own threads keep running - a guest `hid: SendVibrationValues2` line at 76.586 s falls inside the teardown window,
which is the documented assumption that `services::reset_all()` runs only once guest execution has stopped. The process now
ends through the teardown instead of bypassing it, and the deadline path shares the code the normal tail runs.

Not covered: the entry function returning (the game never returns), shutdown while a guest thread holds a runtime lock (the
15 s bound only prevents a wedge), and display/HDR restore, which `nxrt::shutdown()` does not perform. The change is in
`game-build/fefw/src/fefw_main.cpp`, committed in `b5c7d4b7`.

## 2026-11-25 stability campaign validation

Base `730fc46c` on `main`; no GitHub issue was commented on, closed or reopened. The source changes in this pass are
the #77 damaged-autosave guard in `apps/emblemweave/src/saves.rs`, its call in `apps/emblemweave/src/engine.rs`, and
the native-zstd validation in `apps/emblemweave/src/savefile.rs`.

### #77 damaged-autosave guard

`quarantine_damaged_autosave()` runs after `sync_save_dir()` and before the game starts. It opens the exact
`account/<id>/000/Auto` slot the runtime reads at boot. A file that `Slot::open()`, `verify()` or the native zstd
check rejects is moved unchanged into `save/backups/<stamp>-damaged-autosave-Auto`, a `.why.txt` names the reason,
and Play stops with a message naming both. The next Play starts without that file. A valid autosave is left in place. Nothing is deleted or
overwritten, and the archive keeps the original bytes.

Regression tests added: `a_damaged_autosave_is_moved_to_backups_before_play`,
`a_checksum_corrupt_frame_that_still_decompresses_is_rejected_before_play` and
`a_valid_autosave_is_not_moved_by_the_play_guard`. The first truncates a valid slot and checks that the bytes in
backups are identical, that the `.why.txt` exists and that a second call finds nothing. The checksum case builds a
valid checksummed frame, confirms native zstd accepts it, corrupts only the frame checksum, confirms the payload still
decodes and the slot's FNV checksums still pass, confirms native zstd rejects it, and confirms the guard preserves
the bytes in backups. The valid case checks that the file is not moved and no backups folder is created. These are
synthetic frames, not the reporter's save (their save was not in the report bundle).

Limitations: the guard is a launch-time quarantine, not a repair. It does not establish why the frame was damaged and
does not protect a session that damages the autosave after Play has started. The game's own zstd hook still logs the
file and falls back; its guest-side copy path was not changed.

### Test results

- Launcher: `cargo test -p emblemweave --locked` from the repository root. **439 passed, 0 failed, 13 ignored**
  (452 total). This includes the three #77 autosave tests, the existing #79 save/profile cases and the four D3D11VA
  manifest/checker/CLI-capture tests.
- Runtime: `engine/fefw-recomp/runtime/build/release/nxrt_tests.exe --all`. **980 passed, 0 failed, 29 skipped**
  (1009 total), including `write_watch` 17/17 and `watchdog` 19/19. The `svc_fs_romfs` group alone passed 21/21 and
  `svc_fs_volume` 13/13; the #94 case `romfs_package_overlay_markers_are_not_game_files` is in the former.
- #92: the current GPU row was re-read in `pages/media.rs`; the 2026-11-24 captures in `target/capture/` were inspected
  again. The three-adapter list wraps "Detected: ..." below the combo inside the control column, and the no-device line
  wraps in the same place, with no overlap or clipping. The captures are not committed.
- #97: `Batch.cs` still puts `TranslatorBuildId` in the journal header; a different header calls `StartOver()`, which
  deletes the journal, every `.spv` and `<out>.preload.pack*`. No `ShaderCache` remains. The prior smoke run covered a
  resumed journal and deletion of a stale `.spv`; the full corpus was not rerun here.
- #101: `deps.rs` has NASM as `Need::Always`; `tools.lock` has it in the required set; setup, requirements and
  troubleshooting docs name it. No engine build or C-only-versus-assembly decode measurement was made in this pass.
- #104: no code change. dav1d remains the default, FFmpeg/D3D11VA remains explicit, and PR #105 is still unmerged.
  The existing engine-level evidence does not justify changing the default for 0.1.42.

### What remains unverified

One game session was run in this pass: the current working-tree engine binary (SHA-256
`8006801906240415d912055bbb8aaa3d5cb5b2f49d2e837dd75f057d68175fae`) completed the preserved 10-cycle
return-to-title script on 2026-11-24 21:30. It loaded the existing save, returned to the title screen and re-entered
the game ten times: 31,301 frames, 456.7 s, verdict pass, 21 screenshots, exit 0, **0 error lines, 0 guest traps,
0 zstd errors**. The log is `_recovery/issue77_live/run_cycle10_current.log`, the result is
`out_cycle10_current/result.json`, and the screenshots are `shots_cycle10_current/`. It used a valid save and a
working-tree build, not a clean archive or the reporter's damaged file. No cutscene, long session, or other
hardware was tested.

#77 now has its 10 cycles, but still needs a clean-build rerun and the reporter's damaged save through the guard.
#79 still needs a reporter's own save; #92's captures are from 2026-11-24, not a new run; #97's full corpus and
#101's decode measurement were not repeated; #104 needs engine-level evidence on the affected hardware. The
runtime suite result is a clean full-suite run on this host, separate from the 10-cycle gameplay run.
