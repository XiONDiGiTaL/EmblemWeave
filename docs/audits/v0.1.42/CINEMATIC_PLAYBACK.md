# Cinematic Playback - AV1 Video + Opus Audio

> **Historical working note - not release evidence (reviewed 2026-11-24).**
> The authoritative status is `docs/audits/V0.1.42_RELEASE_READINESS.md`. #70 and #75 are **Implemented - unverified**:
> the dav1d/CMake flag and the FFmpeg/Opus paths exist, but no canonical engine build or game-content playback was
> performed, and end-to-end A/V sync is unproven. The install-relative paths below are local development examples, not
> release artifacts.

## Overview
Implementation of cinematic playback using FFmpeg for AV1 video decoding and Opus audio decoding.

## Configuration
```ini
[media]
movie_hwaccel = 1
movie_decoder = ffmpeg
ffmpeg_dir = <install>/upscalers/ffmpeg
```

## Implementation Details

### AV1 Decoder
- **Library**: dav1d (with x86-64 assembly, requires NASM at build)
- **Container**: WebM (AV1CodecConfigurationRecord parsing with variable-length OBU extraction)
- **Fallback**: MP4 av1C format support
- **Hardware Acceleration**: D3D11VA requested (8 threads at above normal priority, 30 pictures ahead); not verified

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
movie: FFmpeg N-123619-g44ad73031d-20260325 loaded from <install>/upscalers/ffmpeg
movie: movie::VideoDecoder on the host: AV1 through FFmpeg (D3D11VA requested, 8 threads at above normal priority, 30 pictures ahead), 24 imports routed
vcodec: nn::codec VP9/VP8 decoders on the host (libvpx v1.16.0, 4 threads): 49 imports routed
```

## Current Status (v0.1.42)
- **Infrastructure**: **Implemented - unverified.** FFmpeg integration, the dav1d AV1 decoder and the hwopus Opus
  decoder exist, but no canonical engine build or game-content playback has been performed.
- **Runtime evidence**: one FCC 120-second session is the only runtime report. It is not the required 10+ menu/title
  cycles (#77) and does not establish cinematic, long-session or A/V-sync stability.
- **Field Test**: Pending - requires reaching cinematic in gameplay
- **Dependencies**: FFmpeg DLLs are loaded from the install's upscaler folder; the paths above are local examples.
- **Release status**: no canonical engine build, no game-content playback, and no end-to-end A/V sync verification.