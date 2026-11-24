// SPDX-License-Identifier: GPL-3.0-or-later
// The host movie::VideoDecoder without the guest ABI (movie_vdec.cpp): its state and the frame handoff between the player's threads and the
// decode worker. The guest imports (vd_*) unpack their arguments and call these functions; tests/test_movie_vdec.cpp drives them directly.
// Only meaningful in a build with dav1d (NXRT_HAVE_DAV1D).
#pragma once

#include <condition_variable>
#include <cstddef>
#include <cstdint>
#include <deque>
#include <mutex>
#include <thread>
#include <utility>
#include <vector>

#include "nxrt/services.h"

#include "movie_ffmpeg.h"

struct Dav1dContext;

namespace nxrt::media::vdec {

// VideoFrame (SDK 22.2): +0x08 buffer, +0x28 / +0x30 bytes of luma / chroma written, +0x48 buffer size, +0x78 time, +0x88 end of stream (u8).
constexpr uint64_t kVfBuf = 0x08, kVfFilled = 0x28, kVfSize = 0x48, kVfTime = 0x78, kVfEos = 0x88;
constexpr int kMimeAv1 = 3;                          // nn::movie::MediaMimeType of an AV1 track
constexpr uint32_t kResultNoFrame = 0x7b;            // AcquireVideoFrame with no picture ready, as the library answers
constexpr uint32_t kResultDecoderStalled = 0x1000;   // nonzero decoder failure; 0x7a is the SDK's retry result and must not be used here

// The GPU serial a frame given back waits for before it takes a new picture: nvn's command serials. Tests put in their own.
struct GpuFence {
    uint64_t (*last_serial)();
    bool (*serial_done)(uint64_t serial);
};
extern GpuFence g_gpu;

struct Packet {
    std::vector<uint8_t> data;
    int64_t time = 0;
    bool eos = false;
};

struct Picture {   // a decoded picture in host memory (NV12 at its width), or the end of the stream
    std::vector<uint8_t> nv12;
    uint32_t w = 0, h = 0;
    int64_t time = 0;
    bool eos = false;
};

struct Av1 {   // the worker's decoder: FFmpeg's on a GPU, or dav1d
    Dav1dContext* dav1d = nullptr;
    ffmpeg::Decoder* ff = nullptr;
    std::vector<uint8_t> config;   // the track's av1C record: dav1d starts from it when the GPU refuses the stream
};

struct Decoder {
    std::mutex mu;
    std::condition_variable cv;              // the worker waits here for packets and for room in `decoded`
    std::thread worker;
    bool stop = false;
    uint64_t generation = 0;                 // stream/queue generation: Flush bumps it, invalidating queued and held packets
    int mime = 0;
    std::vector<uint8_t> config;             // the track's av1C record (Configure): its OBUs go to dav1d ahead of the first packet
    int w = 1920, h = 1080, color[3] = {0, 0, 0};
    std::vector<uint64_t> frames, free_;     // registered VideoFrames; free_ is used as a stack
    std::deque<uint64_t> ready;              // frames with a picture, not acquired yet
    std::deque<Packet> queue;                // packets not decoded yet
    size_t queued_bytes = 0;                 // payload bytes in `queue`
    std::deque<Picture> decoded;             // pictures waiting for a free frame
    // Frames the player gave back, with the GPU serial at that moment: free once the GPU passed it. Fortune's Weave's CPU-frame path
    // polls one fence (sync[0], main 0x1902870) for every buffer, so it released buffers whose NV12 copy to the textures had not run
    // yet; the next picture written into them showed 3 of 4 pictures early or torn (the choppy cutscenes).
    std::deque<std::pair<uint64_t, uint64_t>> pending_free;
    uint64_t packets = 0, pictures = 0;
    uint64_t progress = 0;                   // queue pops and pictures moved into frames: the no-progress watchdog's clock
    bool failed = false;                     // a stalled decoder was stopped; calls answer failure until Flush
    // the movie's clock (skip_late): picture timestamps against the host clock; unit = seconds per timestamp tick, learned from the spacing
    bool clk_on = false;
    int64_t clk_ts0 = 0, clk_last = 0, last_time = 0;
    double clk_t0 = 0, unit = 0;
    uint64_t skipped = 0;
    bool warned_size = false, warned_format = false, warned_mime = false, warned_error = false;
    services::ServiceEvent in_ready, out_ready;   // attached to in_ev / out_ev: any thread can signal them
    alignas(16) uint8_t in_ev[0x100]{}, out_ev[0x100]{}, fmt_ev[0x100]{};
    alignas(16) int32_t scratch[4]{};        // guest-visible out value for MediaProperty::GetInt32
};

void start_worker(Decoder& d);
void stop_worker(Decoder& d);
void register_frames(Decoder& d, const uint64_t* frames, size_t count);
// Queues a packet (waiting for room in the queue) and, when `eos`, the end of the stream. 0, or kResultDecoderStalled once stopped.
uint32_t decode_async(Decoder& d, const uint8_t* data, uint64_t size, int64_t time, bool eos);
uint32_t acquire(Decoder& d, uint64_t* frame, int* index);
void release(Decoder& d, uint64_t frame);
void flush(Decoder& d);

}  // namespace nxrt::media::vdec
