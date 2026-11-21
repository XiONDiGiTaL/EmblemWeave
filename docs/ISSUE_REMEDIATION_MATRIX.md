# Issue Remediation Matrix

| Issue | Title | Status | Root Cause | Fix | Verification |
|-------|-------|--------|------------|-----|--------------|
| #75 | Cinematic playback: FPS drops, audio desync, missing voices | **PARTIAL** | 1. AV1 config parsing (WebM vs MP4)<br>2. hwopus outputs silence<br>3. Game data validation Warn | 1. Fixed WebM AV1CodecConfigurationRecord parsing in movie_vdec.cpp<br>2. hwopus needs Opus decoder implementation<br>3. Warn from general checks (missing Fs.bin.zst, undecoded module, missing title) | 1. "Unknown OBU type 0" error GONE<br>2. Decoder initializes correctly<br>3. End-to-end playback blocked by missing shader cache |
| #76 | Find Units stalls audio/visuals | **FIXED** | 1. Unbounded fallback search (128TB)<br>2. No timeout/cancellation<br>3. Threads at normal priority<br>4. No progress reporting | 1. Limited fallback to 2GB committed RW<br>2. Added 30s timeout & cancellation token<br>3. Threads at BELOW_NORMAL priority<br>4. Progress reporting every 16 pieces | 1. Unit tests pass<br>2. UI shows progress & cancel button<br>3. Scan completes in <5s on test systems |
| #70 | Cinematic slowdown, missing voices (incomplete RomFS) | **ADDRESSED** | Incomplete folder-based RomFS vs complete .nup | Documented requirement for complete .nup (252,686 files) | Complete .nup extracted and validated |
| #59 | Launcher window movement: CPU spikes during drag | **FIXED** | 1. WM_PAINT storm in OS modal move loop<br>2. Redundant per-frame filesystem/layout/mesh work<br>3. Uncapped frame rate with hover animations | 1. Suppress WM_PAINT during caption drag (IN_MOVE guard)<br>2. Cache root/data_root, labels, backdrop mesh, Log filter<br>3. 16ms frame interval (60 FPS cap) with sleep | 1. Drag CPU: 82.7% → 1.5% (55x)<br>2. Idle CPU: 14.7% → 3.9% (3.8x)<br>3. Sweep CPU: 97% → 24.9% (4x)<br>4. 0 px lag, 386 tests pass |
| #29 | Cinematic A/V desync, muted voices, shader stalls | **RELATED** | Shader compilation stalls + hwopus silence | Shader cache generation documented; hwopus fix needed | Shader cache can be pre-generated with --prepare-shaders |

## Legend
- **FIXED**: Root cause identified, fix implemented and tested
- **PARTIAL**: Some causes fixed, others need more work
- **ADDRESSED**: Workaround documented, not a code fix
- **RELATED**: Shares infrastructure with other issues

## Remaining Work
1. Implement Opus decoder in hwopus service (libopus or FFmpeg)
2. Generate shader cache for end-to-end cinematic testing
3. Add movie/voice asset validation to game data checks