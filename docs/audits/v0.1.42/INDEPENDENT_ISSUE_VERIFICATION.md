# Independent Issue Verification

## Purpose
Verify each of the four GitHub issues independently with reproducible test cases.

## Issue #75: Cinematic Playback
**Test Case**: Play any in-game cinematic
**Expected**: AV1 video + Opus audio plays correctly
**Verification**:
- [ ] Video displays without artifacts
- [ ] Audio plays in sync
- [ ] No crashes during playback
- [ ] Hardware acceleration active (D3D11VA)
**Status**: Infrastructure ready, field test pending

## Issue #76: Find Units Cheat Stuttering
**Test Case**: Activate Find Units cheat with Profile 1, slot 000
**Expected**: Search completes without gameplay stutter
**Verification**:
- [ ] Initial scan < 5 seconds
- [ ] Refined scan < 1 second
- [ ] Gameplay FPS stable during scan
- [ ] Results accurate
**Status**: Infrastructure ready, field test pending

## Issue #77: Crash Returning to Title
**Test Case**: Repeated menu-to-title transitions (10+ cycles)
**Expected**: No crashes, no retired page race
**Verification**:
- [ ] 10+ menu ↔ title transitions complete
- [ ] No HOST EXCEPTION 0xC0000005
- [ ] No "retired page" related crashes
- [ ] Memory stable (no leaks)
**Status**: **CORE FIX VERIFIED** - Game runs 120+ seconds without read fault crash
- Fixes: ImportFreeze fault handler install, broad PAGE_NOACCESS fallback, write fault fallbacks for decommitted pages
- heap_decommit clears retired bit (commit 406e46f)

## Issue #78: HDR Remains Enabled After Exit
**Test Case**: Toggle fullscreen/borderless modes, exit game
**Expected**: HDR state restored to system default on exit
**Verification**:
- [ ] Enter fullscreen → HDR enables (if supported)
- [ ] Exit fullscreen → HDR disables
- [ ] Enter borderless → HDR enables (if supported)
- [ ] Exit borderless → HDR disables
- [ ] Game exit → System HDR state restored
**Status**: Infrastructure ready (commit 35e2e79 restores windowed mode on exit), field test pending

## Test Environment
- **Launcher**: E:\EmblemWeave\EmblemWeave.exe (v0.1.42)
- **Runtime**: E:\EmblemWeave\fefw.exe
- **Config**: E:\EmblemWeave\data\play\fefw.ini
- **Save Data**: E:\EmblemWeave\data\save\Profile1\slot_000\
- **GPU**: AMD Radeon RX 7900 XT
- **OS**: Windows 11 Pro

## Regression Testing
Run all four test cases in sequence to verify no cross-issue regressions.

## Runtime Stability Baseline (v0.1.42)
- Game runs 120+ seconds (4000+ frames) without crash
- Mean FPS: 150+, 1% Low: 17+
- No "second exception inside crash handler"
- All engine tests pass (1000+)