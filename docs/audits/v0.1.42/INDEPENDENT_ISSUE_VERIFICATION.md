# Independent Issue Verification

> **Historical working notes, corrected 2026-11-24.** Statuses below are the 2026-11-22/23 session's claims. See
> `docs/audits/V0.1.42_RELEASE_READINESS.md` for the current assessment. Do not quote the old "1000+ tests" or
> "HDR restored" lines; they were not verified.

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
**Status**: **IMPLEMENTED - one 120-second session only; not verified.** The 10+ menu/title cycles below remain unrun.
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
**Status**: **PARTIAL.** Commit 35e2e79 forces windowed mode on stop; it does not capture or restore the original Windows HDR state, SDR white level or display mode. Field test pending, and the implementation is insufficient for the stated expectation.

## Test Environment
- **Launcher**: E:\EmblemWeave\EmblemWeave.exe (v0.1.42, local development binary `bf4a0021...`)
- **Runtime**: E:\EmblemWeave\data\play\fefw.exe (local development binary `8a2d0af5...`)
- **Config**: E:\EmblemWeave\data\play\fefw.ini
- **Save Data**: E:\EmblemWeave\data\save\Profile1\slot_000\
- **GPU**: AMD Radeon RX 7900 XT
- **OS**: Windows 11 Pro

## Regression Testing
Run all four test cases in sequence to verify no cross-issue regressions.

## Runtime Stability Baseline (v0.1.42)
- Game runs 120+ seconds (4000+ frames) without crash
- Mean FPS: 150+, 1% Low: 17+ (single-session claim)
- No "second exception inside crash handler" in that session
- Engine tests: no canonical run exists. The historical totals are irreconcilable (166/14/5 of 185; 931/14/29; "1000+"); a later 698-test run was incomplete. Do not claim all tests pass.