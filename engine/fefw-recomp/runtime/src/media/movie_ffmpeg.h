// SPDX-License-Identifier: GPL-3.0-or-later
// FFmpeg's hardware AV1 decoder for the Switch 2 movie decoder (src/media/movie_vdec.cpp): the movies decode on a GPU through FFmpeg's
// Vulkan video decoder (VideoToolbox on macOS) and come back to host memory as NV12. Where the GPU cannot, movie_vdec.cpp decodes with the
// engine's own dav1d.
//
// FFmpeg is not linked: a matching avcodec-*.dll / avutil-*.dll pair (plus the swresample-*.dll it imports) is loaded on first use,
// so a build without them still plays movies through dav1d. The public headers under third_party/ffmpeg/include pin the ABI this file
// compiles against; a pair whose major version differs is refused. FFmpeg's warnings and errors go to the log.
#pragma once

#include <cstddef>
#include <cstdint>
#include <string>
#include <vector>

namespace nxrt::media::ffmpeg {

// One decoded picture in host memory: NV12, rows packed at the picture's width, chroma interleaved (as movie_vdec.cpp's dav1d path).
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

// FFmpeg's hardware decoder on this platform: "Vulkan", or "VideoToolbox" on macOS.
const char* hardware_name();

struct Decoder;

// Loads FFmpeg and creates the device for `gpu` (as open() names it) ahead of the first movie: that takes about half a second, which the
// movie would otherwise spend without a picture. A device that cannot be created is remembered, so open() goes no further either.
void prepare(const std::string& gpu);

// Opens FFmpeg's AV1 decoder on a GPU with the track's av1C record as its extradata. `gpu` is the Vulkan device name; FFmpeg matches it as
// part of the name (case-sensitive), and an FFmpeg before 9.1 decodes on NVIDIA's GPUs only (so not on an unnamed GPU either). The device
// is created once and shared by every decoder on that GPU. Null with `why` set when the DLLs, the device or the decoder are unavailable.
Decoder* open(const std::vector<uint8_t>& config, const std::string& gpu, std::string& why);

void close(Decoder* decoder);

// Decodes one AV1 packet. Pictures append to `out` with the time of the packet that shows them. False when the decoder failed before its
// first picture (a GPU or driver without AV1 decoding finds out at the first frame): nothing came out, and the caller decodes the stream
// elsewhere, from this packet on. Errors after the first picture are logged and the stream goes on.
bool decode(Decoder* decoder, const uint8_t* data, size_t size, int64_t time, std::vector<Frame>& out);

// Drains the pictures FFmpeg held back after the last packet. The decoder takes no packets after this: close it.
void drain(Decoder* decoder, std::vector<Frame>& out);

}  // namespace nxrt::media::ffmpeg
