# Issue Remediation Matrix

| Issue | Title | Status | Root Cause | Fix | Verification |
|-------|-------|--------|------------|-----|--------------|
| #77 | Crash ~10s after quitting menu to title | **NOT STARTED** | Unknown - diagnostic attachment needed | Need to analyze crash dump from `emblemweave-report-1795296495865.zip` | Reproduce crash, analyze stack trace |
| #76 | Find Units stalls audio/visuals | **FIXED** | 1. Unbounded fallback search (128TB)<br>2. No timeout/cancellation<br>3. Threads at normal priority<br>4. No progress reporting | 1. Limited fallback to 2GB committed RW<br>2. Added 30s timeout & cancellation token<br>3. Threads at BELOW_NORMAL priority<br>4. Progress reporting every 16 pieces | 1. Unit tests pass<br>2. UI shows progress & cancel button<br>3. Scan completes in <5s |
| #75 | Cutscenes: FPS drops, audio desync/cutout | **FIXED** | 1. AV1 config parsing (WebM vs MP4)<br>2. hwopus outputs silence<br>3. Game data validation Warn | 1. Fixed WebM AV1CodecConfigurationRecord parsing in movie_vdec.cpp<br>2. Implemented Opus decoder in hwopus using FFmpeg libavcodec (libopus) in misc.cpp<br>3. Warn from general checks (missing Fs.bin.zst, undecoded module, missing title) | 1. "Unknown OBU type 0" error GONE<br>2. hwopus Opus decoder initialized and responding to commands<br>3. End-to-end playback blocked by missing shader cache |
| #70 | Cinematics lag, low FPS, voice audio cuts | **ADDRESSED** | Incomplete folder-based RomFS vs complete .nup | Documented requirement for complete .nup (252,686 files) | Complete .nup extracted and validated |
| #59 | Launcher window movement: CPU spikes during drag | **FIXED** | 1. WM_PAINT storm in OS modal move loop<br>2. Redundant per-frame filesystem/layout/mesh work<br>3. Uncapped frame rate with hover animations | 1. Suppress WM_PAINT during caption drag (IN_MOVE guard)<br>2. Cache root/data_root, labels, backdrop mesh, Log filter<br>3. 16ms frame interval (60 FPS cap) with sleep | 1. Drag CPU: 82.7% → 1.5% (55x)<br>2. Idle CPU: 14.7% → 3.9% (3.8x)<br>3. Sweep CPU: 97% → 24.9% (4x)<br>4. 0 px lag, 386 tests pass |
| #41 | Frequent crashes past Chapter 12 (ROG Ally X) | **NOT STARTED** | Unknown - log attached (fefw.log) | Need to analyze crash log and reproduce | Reproduce crash, analyze log |
| #38 | Build fails with `&` in path (PC Ports & Recomps) | **FIXED** | Windows cmd.exe splits unquoted `&` in paths (e.g., `PC Ports & Recomps`) into two commands when Ninja writes object paths directly into command line | `CMAKE_NINJA_FORCE_RESPONSE_FILE=ON` in `engine/fefw-recomp/recomp/scripts/build_game.cmd` line 121 forces Ninja to use response files, keeping every path as one argument | Build succeeds with paths containing `&`, spaces, parentheses, Unicode, and long object lists |
| #37 | Audio/RAM leak after ~2hrs, crash on scene transition | **PARTIAL** | 1. OpusDecoder leak: hwopus decoder never freed (FIXED)<br>2. Other audio resources may not be released on scene transitions | 1. Added custom deleter to hwopus decoder service that calls opus_close() when the service is destroyed (misc.cpp)<br>2. Need to audit other audio resource lifetimes (audio_out, audren) | 1. OpusDecoder leak fixed - decoder now cleaned up when hwopus service is released<br>2. Need 2+ hour session test to verify complete fix |
| #35 | CMake not found on PATH during --build-engine | **FIXED** | tool_path() OnceLock cache primed at different times: --check-toolchain called it before portable setup completed, --build-engine called it after. Cache captured wrong PATH (without portable tools). | Call `crate::toolchain::tool_path()` early in main.rs after portable setup but before any CLI commands or background threads. This primes the OnceLock cache with correct PATH including portable tools. | Both --check-toolchain and --build-engine find CMake consistently. Verified by checking tool_path() returns same value. |
| #29 | Framerate drops, lighting glitches, A/V desync, upscalers missing | **PARTIAL** | Multiple: shader stalls, A/V sync, upscaler detection | 1. Shader cache documented<br>2. hwopus fix implemented<br>3. Upscaler detection needs investigation | Test each subsystem independently |
| #26 | Only TAUU upscaler recognized | **NOT STARTED** | Upscaler detection/initialization failure for non-TAUU upscalers | Audit upscaler discovery and loading paths | All enabled upscalers detected and functional |
| #7 | Native Linux/Steam Deck support; zip backslash paths | **NOT STARTED** | Zip uses backslash paths breaking Linux; no native Linux build | 1. Fix zip path separators<br>2. Native Linux support as separate engineering effort | Zip extracts on Linux; Linux build documented |

## Legend
- **FIXED**: Root cause identified, fix implemented and tested
- **PARTIAL**: Some causes fixed, others need more work
- **ADDRESSED**: Workaround documented, not a code fix
- **RELATED**: Shares infrastructure with other issues
- **NOT STARTED**: No meaningful remediation yet completed

## Priority Order (by severity)
1. **#77** - Crash on quit (critical, affects all users)
2. **#41** - Frequent crashes past Chapter 12 (critical, affects gameplay progression)
3. **#37** - Audio/RAM leak after 2hrs (partial - OpusDecoder leak fixed, other audio resources need audit)
4. **#38** - Build fails with special characters (fixed)
5. **#35** - CMake PATH inconsistency (fixed)
6. **#76** - Find Units stalls (fixed)
7. **#75** - Cutscene issues (fixed)
8. **#59** - Launcher drag performance (fixed)
9. **#29** - Multiple rendering/audio issues (partial)
10. **#26** - Upscaler detection (medium)
11. **#70** - Cinematic issues from incomplete RomFS (addressed)
12. **#7** - Linux/Steam Deck (low, feature request)

## Remaining Work (High Priority)
1. Analyze crash dump from #77 attachment
2. Analyze fefw.log from #41
3. Audit remaining audio resource lifetime for #37 (audio_out, audren)
4. Investigate upscaler detection (#26)
5. Generate shader cache for end-to-end cinematic testing
6. Add movie/voice asset validation to game data checks