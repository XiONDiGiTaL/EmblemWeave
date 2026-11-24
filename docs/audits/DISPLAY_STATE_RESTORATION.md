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

`tests/gfx/test_display_state.cpp`, 16 tests, all passing:

- 13 against a fake desktop: unchanged session writes nothing; a changed mode is restored and reported; HDR is restored
  when the mode change took it and left alone when it did not; a missing monitor is skipped; a refused mode or HDR write
  is reported and does not stop another monitor; a second restore writes nothing; snapshot round-trip, truncated monitor
  line, non-snapshot file, missing file, sidecar path.
- 3 live tests, run with `NXRT_DISPLAY_LIVE_TEST=1` (skipped otherwise, because they call the real display APIs):
  - capture read this desktop as `\\.\DISPLAY1 2560x1440@180Hz 32-bit primary=1 hdr_supported=1 hdr_enabled=0 sdr_white=80.0`;
  - `ChangeDisplaySettingsExW` accepted the current mode (written back unchanged: no visible change);
  - `SET_HDR_STATE` accepted the current HDR state (0).

```
SUMMARY display_state: 16 passed, 0 failed, 0 skipped (of 16)
```

## 5. What is not verified

- **An actual exclusive-fullscreen round trip.** Restoring a mode that was really switched was not run, because it
  changes the user's screen; the setter was proven only by writing the current mode back. This is the main gap.
- **HDR on → play → exit → HDR on.** This desktop has HDR off, so the "restore HDR after a mode change" path ran only
  against the fake desktop.
- **Crash recovery in a real crash.** The stale-snapshot path is covered by the fake desktop and by the file format, but
  no process was killed mid-session with a snapshot present.
- **Multi-monitor.** Only one monitor is attached here; the per-device logic is tested with a fake second monitor.

Until those are run, the issue should be described as "implemented with unit and live-API coverage", not "verified".
