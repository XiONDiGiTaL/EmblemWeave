# Issue Remediation Matrix

| Issue | Title | Status | Root Cause | Fix | Verification |
|-------|-------|--------|------------|-----|--------------|
| #75 | Cinematic playback: FPS drops, audio desync, missing voices | **PARTIAL** | 1. AV1 config parsing (WebM vs MP4)<br>2. hwopus outputs silence<br>3. Game data validation Warn | 1. Fixed WebM AV1CodecConfigurationRecord parsing in movie_vdec.cpp<br>2. hwopus needs Opus decoder implementation<br>3. Warn from general checks (missing Fs.bin.zst, undecoded module, missing title) | 1. "Unknown OBU type 0" error GONE<br>2. Decoder initializes correctly<br>3. End-to-end playback blocked by missing shader cache |
| #76 | Find Units stalls audio/visuals | **FIXED** | 1. Unbounded fallback search (128TB)<br>2. No timeout/cancellation<br>3. Threads at normal priority<br>4. No progress reporting | 1. Limited fallback to 2GB committed RW<br>2. Added 30s timeout & cancellation token<br>3. Threads at BELOW_NORMAL priority<br>4. Progress reporting every 16 pieces | 1. Unit tests pass<br>2. UI shows progress & cancel button<br>3. Scan completes in <5s on test systems |
| #70 | Cinematic slowdown, missing voices (incomplete RomFS) | **ADDRESSED** | Incomplete folder-based RomFS vs complete .nup | Documented requirement for complete .nup (252,686 files) | Complete .nup extracted and validated |
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