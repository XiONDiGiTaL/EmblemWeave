# Cinematic Playback Architecture

## Overview

Fire Emblem: Fortune's Weave uses **WebM container with AV1 video** for cinematics. Audio is in separate **Opus files** decoded via the `hwopus` service.

## Container & Codec

| Aspect | Detail |
|--------|--------|
| Container | WebM (Matroska subset) |
| Video Codec | AV1 (AOMedia Video 1) |
| Audio Codec | Opus (separate files) |
| Video Decoder | dav1d (primary, CPU) / FFmpeg libavcodec (optional, D3D11VA hwaccel) |
| Audio Decoder | hwopus service (HLE of `nn::hwopus`) |

## Decoder Pipeline

```
┌─────────────────────────────────────────────────────────────────┐
│                      movie::Extractor (MF)                      │
│  • Parses WebM via Media Foundation (nn::movie::MediaExtractor) │
│  • Track selection: video (kind=0) + audio (kind=1/2)           │
│  • Preroll: waits for first decoded samples from ALL tracks     │
└──────────────────────────┬──────────────────────────────────────┘
                           │
          ┌────────────────┴────────────────┐
          ▼                                 ▼
┌───────────────────────┐         ┌───────────────────────┐
│   Video Decoder       │         │   Audio Decoder       │
│   (movie::VideoDecoder)│         │   (hwopus)           │
│                       │         │                       │
│ • dav1d (8 threads)   │         │ • Currently: SILENCE  │
│ • Frame-ahead: 30     │         │ • Needed: libopus/FFmpeg│
│ • Clock sync: skip    │         │ • 175 Opus files      │
│   late frames         │         │   (BGM/SE/VOICE)      │
│ • GPU fence wait      │         │                       │
└───────────┬───────────┘         └───────────┬───────────┘
            │                                 │
            ▼                                 ▼
    ┌───────────────┐                 ┌───────────────┐
    │ NV12 frames   │                 │ PCM (s16le)   │
    │ → Vulkan tex  │                 │ → AudioRenderer│
    └───────┬───────┘                 └───────────────┘
            │
            ▼
    ┌───────────────┐
    │ Present       │
    │ (Vulkan swap) │
    └───────────────┘
```

## Key Components

### movie_vdec.cpp (Video Decoder)

**Location**: `engine/fefw-recomp/runtime/src/media/movie_vdec.cpp`

**Features**:
- Dual backend: dav1d (default) + FFmpeg (optional)
- dav1d: 8 threads, `max_frame_delay=1`, 30 frames ahead (`g_ahead`)
- FFmpeg: dynamic DLL loading, D3D11VA hwaccel path
- Frame queue with clock synchronization (`movie_clock`)
- Late frame skipping based on presentation time
- GPU fence wait for frame release (`pending_free` + `serial_done`)
- Per-call logging via `video_decoder_log=1`

**AV1 Config Parsing (FIXED)**:
```cpp
// WebM AV1CodecConfigurationRecord (variable-length header)
// marker(1) | version(7) | seq_profile(3) | seq_level_idx_0(5) | seq_tier_0(1) 
// | high_bitdepth(1) | twelve_bit(1) | monochrome(1) | chroma_subsampling_x(1)
// | chroma_subsampling_y(1) | chroma_sample_position(2) | reserved(3)
// | initial_presentation_delay_present(1) | [initial_presentation_delay_minus_one(4)]
// | configOBUsLength(16) | [byte_align] | configOBUs...
```

### movie_mf.cpp (Extractor)

**Location**: `engine/fefw-recomp/runtime/src/media/movie_mf.cpp`

**Features**:
- HLE of `movie::Extractor` and `movie::MediaExtractor`
- Media Foundation topology for WebM parsing
- Track selection via `ext_select()` / `ext_select_locked()`
- Preroll synchronization: `ext_preroll()` waits for first samples from ALL selected tracks (2s timeout)
- Seek handling with loop restart optimization
- Per-track queues with timestamps

### hwopus (Audio Decoder) - IMPLEMENTED

**Location**: `engine/fefw-recomp/runtime/src/services/misc.cpp` (lines 36-400, 590-680)

**Implementation**: FFmpeg libavcodec (libopus) via dynamic DLL loading

**Opus Packet Format**:
```
Nintendo 8-byte header: [u32_be payload_size][u32_be final_range]
Opus packet: [TOC][frame(s)...]
```

**Features**:
- Dynamic FFmpeg DLL loading (avcodec-62.dll, avutil-60.dll, swresample-6.dll) following movie_ffmpeg.cpp pattern
- OpusDecoder struct with AVCodecContext, AVFrame, SwrContext
- Nintendo 8-byte header parsing (u32 BE payload size, u32 BE final range)
- Planar float (FLTP) output converted to interleaved int16 PCM
- Fallback to silence on decode failure
- Logs decoder initialization and decode errors

**Opus Files** (175 total, matching movie IDs):
- `BGM_MV00_00.opus` - Background music
- `SE_MV00_00.opus` - Sound effects  
- `VOICE_MV00_00.opus` - Character voices
- etc.

### Synchronization

**Video Clock** (`movie_clock` in `movie_vdec.cpp`):
- Frame presentation time from dav1d `timestamp` field
- Late frame skipping: frames with `time < clock_now` are dropped
- Frame-ahead queue hides decode latency (30 frames default)

**Audio Clock** (AudioRenderer):
- Pushes PCM to audio endpoint
- No explicit sync with video clock currently

**Known Issue**: No cross-clock synchronization between video and audio pipelines. Audio plays at its own rate, video skips late frames. This causes A/V drift over time.

## Configuration

**nxrt.ini** (or `NXRT_*` env vars):
```ini
[media]
movies = host              # Enable host decoder
movie_decoder = dav1d      # dav1d | ffmpeg
movie_hwaccel = 1          # D3D11VA for FFmpeg

[log]
level = debug
video_decoder_log = 1      # Per-call movie decoder logging
```

## Game Data Requirements

**Complete .nup package** (update 131072):
- 252,686 files, 27.7 GB
- Includes all cinematic WebM videos and Opus audio
- RomFS content index: `Fs.bin.zst` (required for validation Ok)

**Folder-based RomFS** (NOT RECOMMENDED):
- Issue #70 showed incomplete folders cause cinematic problems
- Must have exact file count and structure matching .nup

## Shader Cache (Required for Testing)

**Problem**: Title screen renders black without shader cache.

**Solution**:
1. Run game once (generates `shader_cache/programs.bin`)
2. Run `fefw.exe --prepare-shaders` to pre-compile
3. Subsequent runs have no shader compilation stalls

**Files**:
- `shader_cache/programs.bin` - Program list
- `shader_cache/prepare.state` - Preparation status
- `pipeline_cache.bin` - Vulkan pipeline cache

## Debugging

**Enable verbose logging**:
```ini
[log]
level = debug
video_decoder_log = 1
```

**Key log messages**:
```
movie::VideoDecoder on the host: AV1 through dav1d 1.5.1 with x86-64 assembly (8 threads at above normal priority, 30 pictures ahead)
AV1 config: parsed WebM AV1CodecConfigurationRecord, 123 bytes of OBUs
movie: preroll: first samples decoded in 45.2 ms
movie: frame 123 at 41000000 us, clock 40998000 us, late=false
```

**Common failures**:
- `Unknown OBU type 0` → AV1 config parsing (FIXED)
- `movie decoder made no progress for 5 s` → Decoder stall (FIXED)
- `preroll: no decoded sample after 2 s` → Track selection or decoder init failure
- Black screen + audio → Missing shader cache

## Performance

**Typical decode** (1080p30 AV1, dav1d 8 threads):
- Decode: ~2-5 ms/frame
- Texture upload: ~1-3 ms/frame
- Present: ~16 ms (vsync)
- Total frame budget: 33.3 ms

**Bottlenecks**:
1. Shader compilation (first run only)
2. Texture upload bandwidth
3. dav1d CPU usage (8 threads)
4. A/V sync drift (no cross-clock sync)

## Future Improvements

1. **Add A/V clock synchronization** - Master clock with audio as reference
2. **Movie asset validation** - Check for cinematic files in game data validation
3. **Hardware decode** - Enable FFmpeg D3D11VA path for AV1
4. **Shader cache distribution** - Ship pre-compiled cache