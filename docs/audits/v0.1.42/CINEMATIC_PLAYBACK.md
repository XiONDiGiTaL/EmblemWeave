# Cinematic Playback - AV1 Video + Opus Audio

## Overview
Implementation of cinematic playback using FFmpeg for AV1 video decoding and Opus audio decoding.

## Configuration
```ini
[media]
movie_hwaccel = 1
movie_decoder = ffmpeg
ffmpeg_dir = E:/EmblemWeave/upscalers/ffmpeg
```

## Implementation Details

### AV1 Decoder
- **Library**: dav1d (with x86-64 assembly, requires NASM at build)
- **Container**: WebM (AV1CodecConfigurationRecord parsing with variable-length OBU extraction)
- **Fallback**: MP4 av1C format support
- **Hardware Acceleration**: D3D11VA requested (8 threads at above normal priority, 30 pictures ahead)

### Opus Audio Decoder
- **Library**: hwopus via FFmpeg
- **Dynamic DLL Loading**: avcodec-62.dll, avutil-60.dll, swresample-6.dll
- **Integration**: nn::codec Opus decoder on host

### Codec Imports Routed
- AV1: 24 imports routed to host decoder
- VP9/VP8: 49 imports routed to host decoder (libvpx v1.16.0, 4 threads)

## Field Testing Checklist
- [ ] Trigger cinematic playback in-game
- [ ] Verify AV1 video decodes correctly (no artifacts, correct framerate)
- [ ] Verify Opus audio plays correctly (sync, no crackling)
- [ ] Test multiple cinematics
- [ ] Test seek/skip functionality
- [ ] Verify hardware acceleration is working (D3D11VA)
- [ ] Check CPU/GPU usage during playback

## Known Issues
- FFmpeg version: N-123619-g44ad73031d-20260325 (libavcodec 62, libavutil 60)
- FSR3 frame gen requested but vkCmdWriteBufferMarkerAMD not available on device

## Log Indicators of Success
```
movie: FFmpeg N-123619-g44ad73031d-20260325 loaded from E:/EmblemWeave/upscalers/ffmpeg
movie: movie::VideoDecoder on the host: AV1 through FFmpeg (D3D11VA requested, 8 threads at above normal priority, 30 pictures ahead), 24 imports routed
vcodec: nn::codec VP9/VP8 decoders on the host (libvpx v1.16.0, 4 threads): 49 imports routed
```

## Current Status (v0.1.42)
- **Infrastructure**: Complete - FFmpeg integration, dav1d AV1 decoder, hwopus Opus decoder all implemented
- **Runtime Stability**: Game now runs 120+ seconds without crashing (fixed read fault crash)
- **Field Test**: Pending - requires reaching cinematic in gameplay
- **Dependencies**: FFmpeg DLLs must be in `E:/EmblemWeave/upscalers/ffmpeg/`