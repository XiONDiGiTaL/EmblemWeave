// SPDX-License-Identifier: GPL-3.0-or-later
// Optional libavcodec backend for the Switch 2 movie decoder (src/media/movie_vdec.cpp).
//
// FFmpeg is not linked: a matching avcodec-*.dll / avutil-*.dll pair (plus the swresample-*.dll it imports) is
// loaded at run time, so a build without them still plays movies through dav1d. The public headers under
// third_party/ffmpeg/include pin the ABI this file compiles against; available() rejects a pair whose major version
// differs.
#pragma once

#include <cstddef>
#include <cstdint>
#include <vector>

namespace nxrt::media::ffmpeg {

// One decoded picture in host memory, NV12 at its width.
struct Frame {
    std::vector<uint8_t> nv12;
    uint32_t width = 0, height = 0;
    int64_t time = 0;
};

// True when a usable avcodec/avutil pair was found. Searches [media] ffmpeg_dir, then
// <fefw.exe>\upscalers\ffmpeg, <fefw.exe>\upscalers and <fefw.exe>, and checks the ABI major version.
bool available();

// The loaded FFmpeg version (for example "8.1.2"), or "" when unavailable.
const char* version();

struct Decoder;

// Opens an AV1 decoder with the track's av1C record. `hardware` asks for FFmpeg's native av1 decoder on D3D11VA and
// falls back to FFmpeg's libdav1d decoder if the device or decoder cannot be created; without it, libdav1d is used
// directly. The record's sequence header is fed as the first packet (as the engine's own dav1d path does). Null when
// the DLL pair or a decoder is unavailable.
Decoder* open(const std::vector<uint8_t>& config, int threads, bool hardware);

// True when this decoder is using D3D11VA rather than FFmpeg's software decoder.
bool hardware(const Decoder* decoder);

void close(Decoder* decoder);

// Decodes one AV1 packet. Pictures append to `out` in presentation order; the packet's timestamp is kept.
void decode(Decoder* decoder, const uint8_t* data, size_t size, int64_t time, std::vector<Frame>& out);

// Drains the pictures FFmpeg held back after the last packet, before the end-of-stream marker is delivered.
void drain(Decoder* decoder, std::vector<Frame>& out);

}  // namespace nxrt::media::ffmpeg
