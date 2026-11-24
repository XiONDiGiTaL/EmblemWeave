# Display State Capture and Restoration (Issue #78)

**Date:** 2026-11-24
**Commit:** `94c76883` — `gfx: capture and restore the desktop display state on exit (Issue #78)`
**Files:** `src/gfx/extras/display_state.{h,cpp}`, `src/gfx/extras/win_display_state.cpp`,
`tests/gfx/test_display_state.cpp`, `src/gfx/play/play.cpp`

## 1. What was wrong

`play::stop()` forced windowed mode and called that a restore. Nothing read the desktop's mode, its HDR state or its SDR
white level, so there was nothing to put back. Two consequences:

- leaving exclusive fullscreen returned the window, not necessarily the mode the desktop had before the session;
- a crash, kill or failed launch never reached `stop()` at all, so whatever the session had switched stayed in place.

The runtime never enabled Windows HDR itself, so "HDR remains enabled after exit" cannot be fixed by an unconditional
"turn HDR off": that would turn off the user's own HDR. The implementation therefore captures the state and restores only
what the session changed.

## 2. What a session does now

1. **Before the window mode is applied**, every attached monitor is captured: GDI device name (`EnumDisplayMonitors`),
   `DEVMODE` mode from `EnumDisplaySettingsExW` (width, height, bit depth, refresh, orientation, position), primary flag,
   and from the CCD database (`DisplayConfigGetDeviceInfo`) the HDR-supported and HDR-user-enabled flags plus the SDR
   white level.
2. The snapshot is written beside `fefw.ini` as `<ini>.display-state`, through a temp file and rename so a crash mid-write
   cannot leave a half snapshot.
3. Only when exclusive fullscreen is actually in effect is the snapshot marked `changed mode=1 hdr=1`. Borderless and
   windowed sessions never mark anything, so they can never move the desktop on exit.
4. `stop()` leaves fullscreen first, then restores. `exit_process()` restores too, because that path skips `stop()`.
5. A snapshot left behind by a crash is restored at the start of the next session, before the new snapshot is taken.

## 3. What restore writes

`restore_display_state()` walks the captured monitors and writes only the fields the session changed, and only when the
current value differs from the capture:

| Case | Behaviour |
|---|---|
| Session changed nothing | no write at all; the report says `unchanged` |
| Mode differs on a monitor the session switched | `ChangeDisplaySettingsExW` with the captured `DEVMODE` |
| HDR differs after that mode switch | `DISPLAYCONFIG_DEVICE_INFO_SET_HDR_STATE` to the captured value |
| Monitor no longer attached | counted `missing`, skipped, not a failure; other monitors still restored |
| OS refuses a write | counted `failed`; the remaining monitors are still attempted |
| Value already matches | never written (a second restore is a no-op) |
| SDR white level | captured, logged, **never written**: Windows exposes `DISPLAYCONFIG_SDR_WHITE_LEVEL` read-only |

Multi-monitor is handled per device name, so a secondary monitor that was not switched is never touched.

## 4. Tests

`tests/gfx/test_display_state.cpp`, 18 tests, all passing:

- 13 against a fake desktop: unchanged session writes nothing; a changed mode is restored and reported; HDR is restored
  when the mode change took it and left alone when it did not; a missing monitor is skipped; a refused mode or HDR write
  is reported and does not stop another monitor; a second restore writes nothing; snapshot round-trip, truncated monitor
  line, non-snapshot file, missing file, sidecar path.
- 5 live tests, run with `NXRT_DISPLAY_LIVE_TEST=1` (skipped otherwise, because they call the real display APIs):
  - capture read this desktop as `\.\DISPLAY1 2560x1440@180Hz 32-bit primary=1 hdr_supported=1 hdr_enabled=0 sdr_white=80.0`;
  - `ChangeDisplaySettingsExW` accepted the current mode;
  - `SET_HDR_STATE` accepted the current HDR state;
  - `NXRT_DISPLAY_SET_HDR=0|1` sets HDR for the harness and reads it back (the harness must restore it);
  - `NXRT_DISPLAY_TOGGLE_HDR=1` toggles HDR and restores it, reading back both states.

```
SUMMARY display_state: 18 passed, 0 failed, 0 skipped (of 18)
```

## 5. Live round trip (2026-11-24)

**Machine:** Windows 11 Pro 10.0.26100, AMD Radeon RX 7900 XT (driver 32.0.32029.2010), one monitor
`\.\DISPLAY1` at 2560x1440@180Hz, HDR supported, peak 456 nits; SDR white reads 80 nits with HDR off and 240 with HDR
on. The RomFS was `X:\FEFW\Fire Emblem꞉ Fortune's Weave + Update 1.1.1.nup` (28,946,251,968 bytes, SSD), read-only.
`fefw.exe` was rebuilt from the current tree first: the existing binary was from 23:31, before commit `94c76883`, and
contained neither snapshot string. `window_mode=exclusive` came from a scratch `controls.ini` selected with
`[play] controls`, because play settings are read from `controls.ini`, not `fefw.ini`.

| Case | Observed |
|---|---|
| exclusive fullscreen, HDR off | while live, sidecar `changed 1 1` and the monitor line ended `... 80000 1 1 1`; normal window close (`display: ending the process (pass): window closed`, exit 0) logged `display restore: mode 0, HDR 0, unchanged 2, missing 0, failed 0`; sidecar removed; HDR stayed off (80 nits) |
| HDR-off no-change | covered by the same run: restore wrote nothing and the final live capture was `hdr_enabled=0 sdr_white=80.0` |
| HDR on, normal exclusive run | HDR set to 1 through the production backend (`hdr_enabled=1 sdr_white=240.0`); sidecar captured the monitor with `... 1 1 240000 1 1 1` and `changed 1 1`; normal close logged `mode 0, HDR 0, unchanged 2`; final HDR was still on (240 nits). This desktop did not drop HDR when exclusive fullscreen was applied, so no HDR write was needed. |
| HDR on, forced off during exclusive | HDR set off while the game was in exclusive fullscreen; the next capture read `hdr_enabled=0`; closing the window logged `display restore: mode 0, HDR 1, unchanged 1, missing 0, failed 0: \.\DISPLAY1: HDR on restored`; the final capture read `hdr_enabled=1 sdr_white=240.0` |
| crash recovery | a marked exclusive session (`changed 1 1`, HDR on) was hard-killed (`taskkill /F`); the sidecar survived. HDR was then set off while the game was down (80 nits). The next launch logged `recovered a display snapshot from an earlier session: mode 0, HDR 1, unchanged 1, missing 0, failed 0: \.\DISPLAY1: HDR on restored`, wrote a fresh snapshot, and normal close ended at HDR 1 (240 nits). |

`changed 1 1` is set only after `platform::fullscreen_mode()` reports `Exclusive`, so the sidecar proves the game really
entered exclusive fullscreen. `mode 0` on restore is expected: exclusive used the same 2560x1440@180 desktop mode, so
there was no mode value to write; entry and exit are still proven by the mark, the normal `stop()`, and the removed
sidecar. After the runs HDR was set back off, `sdr_white` reads 80 nits, no sidecar or game process remained.

## 6. What the hardware and API did not provide

- **Nothing was refused.** `SDL_SetWindowFullscreen` accepted exclusive, `ChangeDisplaySettingsExW` accepted the
  captured/current mode, and `DisplayConfigSetDeviceInfo(SET_HDR_STATE)` accepted both HDR on and HDR off.
- **SDR white level cannot be restored.** Windows exposes `DISPLAYCONFIG_SDR_WHITE_LEVEL` read-only; the value is
  captured and logged, never written. This is an API limitation, not a test gap.
- **Only one monitor is attached**, so a live unplug/missing-monitor or second-monitor restore was not run. The fake
  desktop covers `missing`, independent per-monitor writes, and a refused write.
- **A different mode was not switched.** Exclusive used the desktop mode, so the live mode write count is 0; the fake
  desktop covers restoring a mode that actually differs. HDR did not drop on its own either; the HDR restore write was
  exercised by turning HDR off during the exclusive session and during the crash window.
- **Second live restore is not repeated.** The fake test `a_second_restore_writes_nothing` covers idempotence; the game
  removes the sidecar after its one restore, so there is no live second pass over the same snapshot.
