# Issue Remediation Results

## Issue #75: Cinematic Playback

### Root Causes Identified

1. **AV1 Configuration Parsing Failure (FIXED)**
   - **Error**: `Unknown OBU type 0 of size 12` followed by `movie decoder made no progress for 5 s`
   - **Cause**: Game uses WebM container with AV1 video. The AV1 codec configuration is stored in `AV1CodecConfigurationRecord` format (variable-length header with `configOBUsLength` field). The decoder incorrectly treated it as MP4 `av1C` format (fixed 4-byte header) and blindly skipped 4 bytes.
   - **Fix**: Added `extract_webm_av1_obus()` in `engine/fefw-recomp/runtime/src/media/movie_vdec.cpp` to properly parse the WebM AV1 configuration record. Added `extract_av1_obus()` with WebM-first, MP4-fallback logic.
   - **Verification**: Error completely gone from logs. Decoder initializes: `movie::VideoDecoder on the host: AV1 through dav1d 1.5.1 with x86-64 assembly (8 threads at above normal priority, 30 pictures ahead)`

2. **hwopus Outputs Silence (NOT FIXED)**
   - **Cause**: `make_hwopus_decoder()` in `engine/fefw-recomp/runtime/src/services/misc.cpp` decodes to zero-filled buffers (silence).
   - **Impact**: All Opus audio (voices, BGM, SFX during cinematics) is muted.
   - **Files**: 175 Opus files matching movie IDs (BGM_MV00_00.opus, SE_MV00_00.opus, etc.)
   - **Needed**: Implement actual Opus decoding using libopus or FFmpeg's Opus decoder.

3. **Game Data Validation "Warn" (EXPLAINED)**
   - **Source**: General validation checks, not movie-specific:
     - Missing `Fs.bin.zst` (RomFS content index)
     - Module not decoded yet
     - Missing title metadata (control.nacp)
     - Add-on content issues
   - **Not caused by**: Missing movie/voice assets (complete .nup has 252,686 files including all cinematic assets)

4. **Shader Cache Missing (BLOCKS TESTING)**
   - **Symptom**: Title screen renders black, audio plays
   - **Cause**: No shader cache (`shader_cache/programs.bin`)
   - **Workaround**: Run `fefw.exe --prepare-shaders` after first play session generates `shader_cache/programs.bin`

### Cinematic Pipeline Architecture

```
WebM (.nup) → movie::Extractor (MF) → Track Selection
                                    ├── Video Track → movie::VideoDecoder (dav1d/FFmpeg) → NV12 frames → Vulkan texture → Present
                                    └── Audio Track → hwopus (Opus) → PCM → AudioRenderer
```

**Key Components**:
- `movie_vdec.cpp`: dav1d (primary) + FFmpeg (optional hwaccel) AV1 decoder
- `movie_mf.cpp`: Media Foundation movie::Extractor HLE, track selection, preroll
- `misc.cpp`: hwopus service (currently silence)
- Frame-ahead queue: 30 frames default (`g_ahead`)
- Clock sync: `movie_clock` with late frame skipping
- GPU fence sync: `pending_free` with `nvn::be::gpu::serial_done`

### Test Results

| Test | Result |
|------|--------|
| AV1 config parsing (WebM) | ✅ Fixed - no more "Unknown OBU type 0" |
| AV1 config parsing (MP4 fallback) | ✅ Preserved |
| dav1d initialization | ✅ 8 threads, 30 frames ahead |
| Decoder stall | ✅ Fixed - no "no progress for 5s" |
| End-to-end cinematic playback | ❌ Blocked - missing shader cache |
| Audio playback | ❌ Silent - hwopus not implemented |

## Issue #76: Find Units Operation

### Root Causes Identified

1. **Unbounded Fallback Search**
   - Original: If guest range (0x60_0000_0000–0x7C_0000_0000) empty, searched entire 128TB process space (0x1_0000–0x7FFF_FFFF_FFFF)
   - Fixed: Limited to committed RW regions only, capped at 2GB total (`max_fallback_bytes`)

2. **No Timeout or Cancellation**
   - Original: Thread ran indefinitely, no way to cancel
   - Fixed: 30s timeout (`timeout_secs`), `ScanCancelToken` for user cancellation

3. **Thread Starvation**
   - Original: Up to 16 threads at normal priority
   - Fixed: Max 8 threads at `THREAD_PRIORITY_BELOW_NORMAL` (Windows)

4. **No Progress Reporting**
   - Original: UI showed "Searching…" indefinitely
   - Fixed: `ScanProgress` struct with pieces completed, units found, elapsed time. UI shows progress bar and Cancel button.

5. **Duplicate Concurrent Operations**
   - Original: Could spawn multiple searches (button disabled but race possible)
   - Fixed: Single `searching` slot, cancel token prevents new search until current finishes

### Implementation

**New API in `cheats.rs`**:
```rust
pub struct ScanUnitsConfig { timeout_secs, max_fallback_bytes, thread_priority, max_threads }
pub struct ScanCancelToken { cancel(), is_cancelled() }
pub struct ScanProgress { total_pieces, completed_pieces, units_found, elapsed_secs, cancelled, timed_out }
pub fn scan_units(mem, config, cancel, progress) -> (Vec<u64>, ScanProgress)
```

**UI Updates in `pages/cheats.rs`**:
- Shows real-time progress: "Scanning… 45/120 pieces, 12 units found (3.2s)"
- Cancel button during scan
- Proper error messages on timeout/cancellation
- Success notification on completion

### Test Results

| Test | Result |
|------|--------|
| Unit tests (`finds_unit_records`, `unit_cheats_hold_and_revive`) | ✅ Pass |
| Compilation | ✅ Pass |
| All launcher tests (386) | ✅ Pass |
| Timeout behavior | ✅ 30s default, configurable |
| Cancellation | ✅ Immediate thread exit |
| Thread priority | ✅ BELOW_NORMAL on Windows |
| Progress reporting | ✅ Every 16 pieces |
| Memory limit | ✅ 2GB fallback cap |

## Deployment

### Files Deployed to `E:\EmblemWeave\data\play\`

| File | SHA256 | Source |
|------|--------|--------|
| fefw.exe | `96970418299d946a58acc83801906aaf4306e30899f1e827c7e7c28edcd3f1f8` | `engine/fefw-recomp/game-build/fefw/build/release/fefw.exe` |
| fefw.pdb | `51b2e892273bbc344ad1a4c2c0091cf6073ddea9cae0d9d703b312ffb90aba2b` | `engine/fefw-recomp/game-build/fefw/build/release/fefw.pdb` |

### Backup

- **Location**: `E:\EmblemWeave\backups\pre-av1-fix-20261121-153710\`
- **Contents**: Previous fefw.exe (169,523,712 bytes) and fefw.pdb (36,089,856 bytes)

### Source Commit

- **Repository**: `C:\Users\xiond\.config\freebuff-desktop\local-projects\413b2383-4be0-4b3c-b36e-5f485402a327\EmblemWeave`
- **Branch**: `main`
- **Commit**: `a3b84ee` (Phase D Production Hardening + PR #74 zbic error message fix)
- **Changes**: 
  - `engine/fefw-recomp/runtime/src/media/movie_vdec.cpp` - AV1 WebM config parsing fix
  - `apps/emblemweave/src/cheats.rs` - Find Units scan_units API rewrite
  - `apps/emblemweave/src/pages/cheats.rs` - UI updates for progress/cancel

## Issue #59: Launcher Window Movement Performance

### Root Causes Identified

1. **WM_PAINT Storm in Modal Move Loop (FIXED)**
   - **Cause**: On Windows, dragging a caption runs a modal move loop inside `DefWindowProc`. This loop dispatches `WM_PAINT` for every pointer movement step. winit answers each paint with `RedrawRequested`; eframe draws a frame, and if egui asks for another frame, winit posts another `WM_PAINT`. The loop follows the pointer only as fast as the launcher can draw. [winit#4708](https://github.com/rust-windowing/winit/issues/4708)
   - **Fix**: `platform.rs` subclasses launcher window, records `HTCAPTION` press, arms `IN_MOVE` guard on `WM_ENTERSIZEMOVE`, sends `WM_PAINT` to `DefWindowProcW` during move. Windows validates paint region, DWM carries last frame; no frame drawn, no next paint posted.
   - **Verification**: Drag CPU 82.7% → 5.2% (16x), deployed fix 1.5% (55x), 0 px lag.

2. **Redundant Per-Frame Work (FIXED)**
   - **Cause**: `engine::root()`/`data_root()` called filesystem ops per frame; `widgets::tracked` laid out every character separately; `widgets::backdrop` rebuilt mesh every frame; Log page rebuilt filtered list (50k lines) every frame; pages asked for frames unnecessarily; no frame cap ran at monitor rate (151 FPS).
   - **Fix**: Cache root/data_root once per process; `LayoutJob` with `extra_letter_spacing` cached per label; backdrop mesh cached per size/colourway; Log filter rebuilt only on change; Controls 4 FPS, Cheats on attach, About 20 FPS; global 16ms frame interval sleep.
   - **Verification**: Frame build time 20.6ms → 2.7ms (7.6x); sweep CPU 97% → 24.9% (4x); idle CPU 0% maintained.

3. **DWM Transition Animation (FIXED)**
   - **Cause**: `ShowWindow(SW_RESTORE)` and `SetWindowPlacement` animate restore; move loop anchors on intermediate frame causing jump.
   - **Fix**: Clear `WS_MAXIMIZE`, set rect with `SetWindowPos` (no transition), `DwmSetWindowAttribute(DWMWA_TRANSITIONS_FORCEDISABLED)`.
   - **Verification**: Maximized caption press restores under pointer in 40ms, then tracks 1:1.

4. **Windhawk Slick Window Arrangement Mod (MITIGATED)**
   - **Cause**: Injected `slick-window-arrangement_1.0.2_229520.dll` computes release velocity, starts 10ms timer calling `SetWindowPos` for 50 frames → post-release glide.
   - **Fix**: Register `Windhawk_Unsubclass_slick-window-arrangement` message, send to launcher HWND on `WM_ENTERSIZEMOVE` to remove mod's subclass for that drag only.
   - **Verification**: Post-release drift 82px over 800ms → 0px.

### Performance Measurements

| Metric | Pre-Fix (Deployed) | Post-Fix (Current Release) | Improvement |
|--------|-------------------|---------------------------|-------------|
| Deployed SHA256 | `a3c59cec` | `748e3a99` | — |
| Idle CPU (2s) | 14.7% | 3.9% | **3.8x** |
| Caption Drag CPU (3s) | 15.6% | 1.5% | **10.4x** |
| Pointer Sweep CPU (6s) | 43.6% | 45.3% | Similar (exercises full UI) |
| Frame Build Time | ~20ms | 2.7ms | **7.6x** |
| Window Tracking Lag | N/A | 0 px | Perfect |
| Launcher Tests | N/A | 386 passed | ✅ |

### Test Results

| Test | Result |
|------|--------|
| WM_PAINT suppression during caption drag | ✅ Verified via move_cpu_probe.ps1 |
| DWM transitions disabled | ✅ hr=0x00000000 in launcher.log |
| Root/data_root caching | ✅ Invalidated on checkout/storage change |
| Label caching (LayoutJob) | ✅ One job per label |
| Backdrop mesh caching | ✅ Per size/colourway |
| Log page filter caching | ✅ Rebuild on lines/filter change |
| Page frame pacing | ✅ Controls 4 FPS, Cheats on attach, About 20 FPS |
| Global 60 FPS cap | ✅ 16ms interval sleep in App::update |
| Caption hit-test band | ✅ Excludes chips/actions (44-240, 572-896 at 1440w) |
| Maximized restore without transition | ✅ SetWindowPos + DWMWA_TRANSITIONS_FORCEDISABLED |
| Borderless resize edges | ✅ WM_NCHITTEST returns 10/11/12/15/13/17 |
| Windhawk mitigation | ✅ Unsubclass message sent on ENTERTISIZEMOVE |
| All launcher tests (386) | ✅ Pass |
| Deployed launcher SHA256 verified | ✅ 748e3a9938d86523abb82d29e14b56e8697383e0fc6155f01e55f6cad3b4f378 |

### Deployment

**Deployed Launcher Updated**:
- **Location**: `E:\EmblemWeave\EmblemWeave.exe`
- **SHA256**: `748e3a9938d86523abb82d29e14b56e8697383e0fc6155f01e55f6cad3b4f378`
- **Backup**: `E:\EmblemWeave\backups\pre-issue59-fix-20261121-174206\` (previous `a3c59cec` build)
- **Source Commit**: `1581790` (Add issue remediation documentation) + fixes from `d9f18c0`, `4d7edcf`, and subsequent

### Key Files Modified

| File | Change |
|------|--------|
| `apps/emblemweave/src/platform.rs` | Native drag: IN_MOVE guard, WM_PAINT suppression, DWM transitions, hit-test, Windhawk mitigation |
| `apps/emblemweave/src/main.rs` | Frame pacing: FRAME_INTERVAL=16ms, last_frame tracking, sleep |
| `apps/emblemweave/src/perf.rs` | Frames stats: single sort optimization |
| `apps/emblemweave/src/pages/performance.rs` | Runtime status display |
| `apps/emblemweave/src/pages/diagnostics.rs` | Frame time graph, diagnostics export |
| `apps/emblemweave/src/theme.rs` | Baloo 2, Nunito font config |
| `apps/emblemweave/src/shell.rs` | Top bar, nav, status bar, activity hub (cached labels) |
| `apps/emblemweave/src/widgets/tracked.rs` | LayoutJob caching |
| `apps/emblemweave/src/widgets/backdrop.rs` | Mesh caching |
| `apps/emblemweave/src/pages/log.rs` | Filter caching |

## Remaining Limitations

1. **hwopus silence**: Voices/BGM/SFX during cinematics remain muted until Opus decoder implemented
2. **Shader cache**: End-to-end cinematic testing requires manual `--prepare-shaders` run
3. **Movie asset validation**: No specific check for cinematic movie/voice files in game data validation
4. **Audio/video sync**: Not tested end-to-end due to shader cache blocker
5. **Sweep settled CPU**: 21.1% after sweep due to hover tooltip decay (~2s); could optimize further