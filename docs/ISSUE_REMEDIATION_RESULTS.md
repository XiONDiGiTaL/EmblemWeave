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

## Remaining Limitations

1. **hwopus silence**: Voices/BGM/SFX during cinematics remain muted until Opus decoder implemented
2. **Shader cache**: End-to-end cinematic testing requires manual `--prepare-shaders` run
3. **Movie asset validation**: No specific check for cinematic movie/voice files in game data validation
4. **Audio/video sync**: Not tested end-to-end due to shader cache blocker