// SPDX-License-Identifier: GPL-3.0-or-later
// movie::VideoDecoder (the Switch 2 movie library; Fortune's Weave's subsdk2) on the host, decoding AV1 with dav1d or, optionally, FFmpeg's libavcodec. include/nxrt/movie_vdec.h.
//
// Ported from nxnative-penis6 dm/host/hle_nx2_movie.c (MIT; commits 9d1a455e17 and 3f9113b10e). The library's own decoder drives nn::codec,
// the NVDEC of the skipped GPU boot. Its nn::movie::MediaExtractor runs as it is, so the game demuxes and this decodes. The player (main
// 0x1900000-0x1907xxx) registers VideoFrames with its own buffers, hands packets to DecodeAsync on the input event, takes pictures with
// AcquireVideoFrame on the output event and gives them back with ReleaseVideoFrame. The frames are the game's: this writes the fields their
// accessors read. Pictures are NV12 at the picture's width: the player's CPU-frame path. The console's player uses native images, which the
// host cannot fill, so the game module selects the other path (games/fefw/fefw_play.cpp).
//
// Unlike the fork, a host thread per decoder decodes ahead (a 1440p picture takes tens of milliseconds, which stalled the game's main thread
// when it was decoded inside ReleaseVideoFrame). Up to g_ahead pictures ([media] video_decode_ahead) wait in host memory; a guest call or the
// worker copies them into free frames. The game's player waits for a late picture and its sound drops out meanwhile; once the pictures catch
// up with the clock the sound stays behind (Fortune's Weave: 0.9 s late pictures at 43-51 s of a 1440p movie, two dropouts, then the sound out
// of step to the end). The cushion of decoded pictures rides over a stretch the decoder takes longer for than the movie's frame rate allows;
// dav1d's x86-64 assembly (NXRT_DAV1D_ASM, needs NASM at build time) makes every picture faster. The input and output events are host
// kernel events attached to the guest's SystemEventTypes (as vi's vsync), so the worker can wake the player.
//
// Layouts (SDK 22.2): VideoFrame +0x08 buffer, +0x28 / +0x30 bytes of luma / chroma written, +0x48 buffer size, +0x78 time, +0x88 end of
// stream (u8); nn::movie::Buffer +0x00 data, +0x10 bytes, +0x18 time, +0x20 flags (8: end of stream). The host state hangs off the guest
// object's first word (where the library keeps its own), with a marker in the second.
//   [media] movies = host | skip          host (default for a title with game::Info::movie_video_decoder); skip: every movie fails to open
//                                         and the game moves on as after a skip (core/switch2.cpp)
//   [media] movie_decoder = dav1d|ffmpeg  dav1d (default) is built in. ffmpeg loads the player's FFmpeg 8.x shared DLLs
//                                         (avcodec-62.dll, avutil-60.dll, swresample-6.dll) from [media] ffmpeg_dir or upscalers\ffmpeg.
//   [media] movie_hwaccel = 1             FFmpeg only: 1 asks for D3D11VA and copies NV12 frames back; 0 uses its libdav1d decoder.
//   [media] video_decoder_threads = 0     dav1d worker threads per decoder; 0 (default): half the logical processors, 4 to 8
//   [media] video_decode_ahead = 30       decoded pictures held ahead of the player (4 to 120; 30 is about 1 s, 165 MiB at 2560x1440)
//   [media] video_decoder_priority = above  Windows priority of the decode worker and dav1d's task threads: above (default), highest or
//                                         normal (as before). The process runs in the high class and the guest threads sit at above normal
//                                         and higher (kernel/sched.cpp), so at normal dav1d got only the CPU the game left: on a CPU with
//                                         few cores the 1440p movies fell behind (#40). Power throttling is off for them unless normal.
//   [media] video_decoder_log = 1         one log line per decoder call (verbose)
#include <algorithm>
#include <chrono>
#include <cmath>
#include <atomic>
#include <cctype>
#include <cerrno>
#include <condition_variable>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <deque>
#include <mutex>
#include <string>
#include <thread>
#include <vector>

#include <windows.h>

#include "nxrt/abi.h"
#include "nxrt/config.h"
#include "nxrt/diag.h"
#include "nxrt/dispatch.h"
#include "nxrt/game.h"
#include "nxrt/log.h"
#include "nxrt/module.h"
#include "nxrt/movie_vdec.h"
#include "nxrt/services.h"

#include "movie_ffmpeg.h"

#if NXRT_HAVE_DAV1D
#include <dav1d/dav1d.h>
extern "C" void (*dav1d_nxrt_thread_hook)(int begin);   // third_party/dav1d/src/win32/thread.c
#endif

namespace nxrt::nvn::be::gpu {   // gfx/nvn/nvn_gpu.h (its header needs the NVN include path)
uint64_t last_serial();
bool serial_done(uint64_t serial);
}  // namespace nxrt::nvn::be::gpu

namespace nxrt::media {

namespace {
std::atomic<bool> g_enabled{false};
}

#if NXRT_HAVE_DAV1D
namespace {

#define A(i) (c->x[i])
#define RET(v) (c->x[0] = static_cast<uint64_t>(v))

constexpr const char* kTag = "movie";
constexpr uint64_t kMarker = 0x4e5852434d564443ull;   // "NXRCMVDC" at +8 of the guest object
constexpr uint64_t kVfBuf = 0x08, kVfFilled = 0x28, kVfSize = 0x48, kVfTime = 0x78, kVfEos = 0x88;
constexpr uint64_t kBufData = 0x00, kBufFilled = 0x10, kBufTime = 0x18, kBufFlags = 0x20;
constexpr uint32_t kBufFlagEos = 8;
constexpr int kMimeAv1 = 3;                // nn::movie::MediaMimeType of an AV1 track
constexpr uint32_t kResultNoFrame = 0x7b;  // AcquireVideoFrame with no picture ready, as the library answers
constexpr uint32_t kAutoClear = 1;         // nn::os::EventClearMode_AutoClear
constexpr size_t kMaxFrames = 64;
// The player must not hand the decoder an unbounded stream: the old queue took every packet as fast as the demuxer produced it (3301 packets
// in 11 s of a 60 fps movie on a 1440p opening) while only ~330 pictures decoded. The feed thread then blocked here until the worker had
// room, so the decoder ran on the current picture instead of a queue's worth of stale ones and memory stayed bounded. The byte cap keeps a
// few large 4K packets from taking the same room as hundreds of small ones.
//
// Normal backpressure never drops a packet: dropping one AV1 packet loses the decoder's reference state until the next keyframe, so every
// frame after it can be wrong. This wait cannot deadlock. DecodeAsync runs on the guest's MovieVideoHandler thread; the player releases frames
// on a different thread (the log shows guest-118 feeding and main releasing), and that release calls deliver(), which wakes this thread when a
// frame frees a slot. The worker wakes this thread after every queue pop.
//
// A player can still stop releasing frames altogether. A wait is therefore abandoned after kNoProgressLimit with no queue pop and no
// deliver() moving a picture, as long as the game kept presenting frames meanwhile. A game that presents (almost) nothing is stalled as a
// whole, by a load or a hitch on its main thread, and that thread is the one that releases the frames: that wait goes on, up to
// kGameStallLimit, because stopping the movie there cut it short for nothing. abort_stalled() then stops the movie: it marks the decoder
// failed, clears the stalled input/decoded state (the only place packets are discarded), returns any safe ready/free frame as an EOS frame,
// and wakes both guest events. When the player holds every frame, the EOS is owed instead and deliver() queues it on the first frame the
// player gives back; without it the player waited for an end that never came and the movie froze on its last picture. A subsequent
// DecodeAsync returns kResultDecoderStalled instead of waiting. vd_flush() resets the failure for the next stream, and stop_worker() still
// joins the worker on finalize.
constexpr auto kNoProgressLimit = std::chrono::seconds(5);
constexpr auto kGameStallLimit = std::chrono::seconds(30);
constexpr uint64_t kGameRunningFrames = 10;   // presented within kNoProgressLimit (2 a second): the game runs, so the stall is the player's
constexpr uint32_t kResultDecoderStalled = 0x1000;   // nonzero decoder failure; 0x7a is the SDK's retry result and must not be used here

bool g_log = false;
int g_threads = 4;
size_t g_ahead = 30;                        // decoded pictures held in host memory while no frame is free ([media] video_decode_ahead)
int g_priority = THREAD_PRIORITY_ABOVE_NORMAL;   // of the decode worker and dav1d's task threads ([media] video_decoder_priority)
bool g_hwaccel = true;                      // FFmpeg's D3D11VA path ([media] movie_hwaccel)
enum class Av1Backend { Dav1d, Ffmpeg };
Av1Backend g_backend = Av1Backend::Dav1d;   // [media] movie_decoder
std::atomic<int> g_dav1d_threads{0};        // dav1d task threads alive (numbers their names)

// The calling thread as a movie decoder thread: its priority, no power throttling (EcoQoS would clock it down on a hybrid or a laptop CPU),
// and a diag registration, so place_log, the sampling profiler and the freeze watchdog see it.
void decoder_thread_begin(const char* name) {
    log_set_thread_name(name);
    diag_register_thread(name, nullptr);
    if (g_priority == THREAD_PRIORITY_NORMAL) return;
    SetThreadPriority(GetCurrentThread(), g_priority);
    THREAD_POWER_THROTTLING_STATE st = {};
    st.Version = THREAD_POWER_THROTTLING_CURRENT_VERSION;
    st.ControlMask = THREAD_POWER_THROTTLING_EXECUTION_SPEED;
    st.StateMask = 0;   // explicitly not throttled
    SetThreadInformation(GetCurrentThread(), ThreadPowerThrottling, &st, sizeof st);
}

void dav1d_thread_hook(int begin) {
    if (!begin) {
        diag_unregister_thread();
        g_dav1d_threads.fetch_sub(1, std::memory_order_relaxed);
        return;
    }
    char name[32];
    snprintf(name, sizeof name, "movie-av1-%d", g_dav1d_threads.fetch_add(1, std::memory_order_relaxed));
    decoder_thread_begin(name);
}
std::atomic<uint64_t> s_packets{0}, s_pictures{0}, s_acquired{0}, s_released{0};
// Decode time of the packets since the last stats line (worker threads): total and the slowest, in microseconds.
std::atomic<uint64_t> s_dec_us{0}, s_dec_n{0}, s_dec_max_us{0};
// The input queue as the player sees it (packets and bytes waiting for the worker), and the times the feed thread waited for room. Read
// without the decoder lock by stats_tick.
std::atomic<uint64_t> s_queue_packets{0}, s_queue_bytes{0}, s_max_queue_packets{0}, s_blocked{0}, s_blocked_us{0};

// A stats line at most every 5 s while a movie plays: packets in, pictures decoded, frames acquired and released by the player, and the
// input queue. The queue line is what shows backpressure working: it stays at kMovieMaxQueuedPackets instead of growing without bound.
void stats_tick() {
    static std::atomic<int64_t> last_ns{0};
    const int64_t now = static_cast<int64_t>(GetTickCount64()) * 1000000;
    int64_t prev = last_ns.load(std::memory_order_relaxed);
    if (prev && now - prev < 5'000'000'000ll) return;
    if (!last_ns.compare_exchange_strong(prev, now)) return;
    const uint64_t n = s_dec_n.exchange(0), us = s_dec_us.exchange(0), mx = s_dec_max_us.exchange(0);
    NXLOG_I(kTag, "stats: packets %llu, pictures %llu, frames acquired %llu, released %llu, queue %llu packets (%.1f MiB, max %llu), feed waits %llu (%.1f ms); decode %.2f ms per packet (slowest %.1f) over %llu",
            static_cast<unsigned long long>(s_packets.load()), static_cast<unsigned long long>(s_pictures.load()),
            static_cast<unsigned long long>(s_acquired.load()), static_cast<unsigned long long>(s_released.load()),
            static_cast<unsigned long long>(s_queue_packets.load()), s_queue_bytes.load() / 1048576.0,
            static_cast<unsigned long long>(s_max_queue_packets.load()), static_cast<unsigned long long>(s_blocked.load()),
            s_blocked_us.load() / 1000.0, n ? us / 1000.0 / n : 0.0, mx / 1000.0, static_cast<unsigned long long>(n));
}

inline uint64_t rd64(uint64_t a) { uint64_t v; std::memcpy(&v, reinterpret_cast<const void*>(a), 8); return v; }
inline uint32_t rd32(uint64_t a) { uint32_t v; std::memcpy(&v, reinterpret_cast<const void*>(a), 4); return v; }
inline void wr64(uint64_t a, uint64_t v) { std::memcpy(reinterpret_cast<void*>(a), &v, 8); }
inline void wr8(uint64_t a, uint8_t v) { std::memcpy(reinterpret_cast<void*>(a), &v, 1); }

// A guest function called from a handler, on the guest thread that called the import; returns x0.
uint64_t gcall(::Ctx* c, uint64_t fn, uint64_t a0, uint64_t a1 = 0, uint64_t a2 = 0, uint64_t a3 = 0) {
    if (!fn) return 0;
    uint64_t save[9];
    std::memcpy(save, c->x, sizeof save);
    const uint64_t lr = c->x[30], f = c->fn;
    c->x[0] = a0; c->x[1] = a1; c->x[2] = a2; c->x[3] = a3;
    dispatch_call(c, fn);
    const uint64_t r = c->x[0];
    std::memcpy(c->x, save, sizeof save);
    c->x[30] = lr;
    c->fn = f;
    return r;
}

struct Fns {
    uint64_t create_event, attach_event, get_int32, set_int32, set_size;
};
const Fns& fns() {
    static const Fns f = [] {
        Fns o{};
        o.create_event = module_find_symbol("_ZN2nn2os17CreateSystemEventEPNS0_15SystemEventTypeENS0_14EventClearModeEb");
        o.attach_event = module_find_symbol("_ZN2nn2os33AttachReadableHandleToSystemEventEPNS0_15SystemEventTypeEjbNS0_14EventClearModeE");
        o.get_int32 = module_find_symbol("_ZNK2nn5movie13MediaProperty8GetInt32EPiPKc");
        o.set_int32 = module_find_symbol("_ZN2nn5movie13MediaProperty8SetInt32EPKci");
        o.set_size = module_find_symbol("_ZN2nn5movie13MediaProperty7SetSizeEPKcm");
        if (!o.create_event || !o.attach_event || !o.get_int32 || !o.set_int32 || !o.set_size)
            NXLOG_E(kTag, "guest nn::os / nn::movie::MediaProperty functions missing: movies will not play");
        return o;
    }();
    return f;
}

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

struct Av1 {   // the worker's decoder: dav1d, or FFmpeg's libavcodec when selected and available
    Dav1dContext* dav1d = nullptr;
    ffmpeg::Decoder* ff = nullptr;
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
    bool eos_owed = false;                   // the stop found no frame for its EOS: deliver() queues it on the first frame given back
    // When the player last asked for a frame (with or without getting one), last got one and last gave one back: the stall line uses these
    // to tell a player that stopped calling from one that asks while nothing is ready.
    std::chrono::steady_clock::time_point last_ask{}, last_acquire{}, last_release{};
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

Decoder* dec_of(uint64_t self) {
    if (!self || rd64(self + 8) != kMarker) return nullptr;
    return reinterpret_cast<Decoder*>(rd64(self));
}

// I420 to NV12, rows packed at the picture's width, chroma interleaved.
void to_nv12(uint8_t* o, uint32_t w, uint32_t h, const uint8_t* py, ptrdiff_t sy, const uint8_t* pu, const uint8_t* pv, ptrdiff_t suv) {
    const uint32_t cw = (w + 1) / 2, ch = (h + 1) / 2;
    for (uint32_t r = 0; r < h; ++r) std::memcpy(o + static_cast<uint64_t>(r) * w, py + static_cast<ptrdiff_t>(r) * sy, w);
    uint8_t* q = o + static_cast<uint64_t>(w) * h;
    for (uint32_t r = 0; r < ch; ++r) {
        const uint8_t* u = pu + static_cast<ptrdiff_t>(r) * suv;
        const uint8_t* v = pv + static_cast<ptrdiff_t>(r) * suv;
        for (uint32_t x = 0; x < cw; ++x) { *q++ = u[x]; *q++ = v[x]; }
    }
}

uint64_t nv12_size(uint32_t w, uint32_t h) { return static_cast<uint64_t>(w) * h + static_cast<uint64_t>((w + 1) / 2) * 2 * ((h + 1) / 2); }

// A player that shows one picture a frame plays a 60 fps movie at the game's frame rate: Ocarina of Time's AV1 movies ran far below real time
// with their sound ahead while the game drew 10 to 30 frames a second. So pictures go by the clock, as a video player does (nxnative oot2
// 3b588ca6): the timestamps from the first picture against the host clock, and of the waiting pictures the newest one that is due, the late
// ones before it dropped. A picture is late once the next is 50 ms overdue (no slack would make a game one frame behind skip every other
// picture). A stall over a second (a load, a pause) or a timestamp going back (a loop, a new stream) starts the clock again; Flush too.
// A game that keeps up drops nothing. [media] movie_clock = 0 turns it off. The timestamp unit (us or ns) is learned from the spacing.
double host_seconds() { return std::chrono::duration<double>(std::chrono::steady_clock::now().time_since_epoch()).count(); }

void learn_unit(Decoder& d, int64_t t) {
    if (d.unit == 0 && d.last_time && t > d.last_time) {
        const int64_t dt = t - d.last_time;   // one picture: ~16,667 at 60 fps in us, ~16,666,667 in ns, ~17 in ms
        if (dt >= 2000 && dt <= 200000) d.unit = 1e-6;
        else if (dt >= 2000000 && dt <= 200000000) d.unit = 1e-9;
        else if (dt >= 2 && dt <= 200) d.unit = 1e-3;
        if (d.unit != 0) NXLOG_I(kTag, "movie clock: timestamps in %s (picture spacing %lld)", d.unit == 1e-6 ? "us" : d.unit == 1e-9 ? "ns" : "ms",
                                 static_cast<long long>(dt));
    }
    d.last_time = t;
}

void skip_late(Decoder& d) {   // lock held, at least one decoded picture
    static const bool on = config_get_bool("media", "movie_clock", true);
    if (!on || d.unit == 0 || d.decoded.front().eos) return;
    const double now = host_seconds();
    const int64_t ts = d.decoded.front().time;
    if (!d.clk_on || ts < d.clk_last || now - (d.clk_t0 + static_cast<double>(ts - d.clk_ts0) * d.unit) > 1.0) {
        d.clk_on = true;
        d.clk_ts0 = ts;
        d.clk_t0 = now;
        return;
    }
    while (d.decoded.size() > 1 && !d.decoded[1].eos && d.clk_t0 + static_cast<double>(d.decoded[1].time - d.clk_ts0) * d.unit <= now - 0.05) {
        d.decoded.pop_front();
        if (!d.skipped++) NXLOG_I(kTag, "movie clock: late pictures skipped to keep the movie in time (the game shows fewer pictures a second than the movie has)");
    }
}

bool queue_eos(Decoder& d);

// Moves decoded pictures into free frames (caller holds the lock) and wakes the player when one is ready.
void deliver(Decoder& d) {
    bool any = false;
    while (!d.pending_free.empty() && nvn::be::gpu::serial_done(d.pending_free.front().second)) {
        d.free_.push_back(d.pending_free.front().first);
        d.pending_free.pop_front();
    }
    if (d.eos_owed && queue_eos(d)) d.eos_owed = false;
    while (!d.decoded.empty() && !d.free_.empty()) {
        skip_late(d);
        Picture& p = d.decoded.front();
        if (!p.eos) {
            learn_unit(d, p.time);
            d.clk_last = p.time;
        }
        const uint64_t f = d.free_.back();
        d.free_.pop_back();
        uint64_t y = 0, uv = 0;
        if (!p.eos) {
            y = static_cast<uint64_t>(p.w) * p.h;
            uv = nv12_size(p.w, p.h) - y;
            const uint64_t buf = rd64(f + kVfBuf), cap = rd64(f + kVfSize);
            if (buf && y + uv <= cap) std::memcpy(reinterpret_cast<void*>(buf), p.nv12.data(), y + uv);
            else {
                if (!d.warned_size) NXLOG_W(kTag, "a %ux%u picture (%llu bytes) does not fit its frame's %llu-byte buffer: frames left empty", p.w, p.h,
                                            static_cast<unsigned long long>(y + uv), static_cast<unsigned long long>(cap));
                d.warned_size = true;
                y = uv = 0;
            }
        }
        wr64(f + kVfFilled, y);
        wr64(f + kVfFilled + 8, uv);
        wr64(f + kVfTime, static_cast<uint64_t>(p.time));
        wr8(f + kVfEos, p.eos ? 1 : 0);
        d.ready.push_back(f);
        d.decoded.pop_front();
        any = true;
    }
    if (any) {
        ++d.progress;
        d.out_ready.signal();
        d.cv.notify_all();   // room in `decoded` again
    }
}

// Gives the player an end-of-stream frame without overwriting a frame it still owns: a ready frame not acquired yet is preferred, then a free
// one. Leaves the remaining ready frames for the last pictures. Caller holds the lock; true when a frame was queued.
bool queue_eos(Decoder& d) {
    uint64_t f = 0;
    if (!d.ready.empty()) {
        f = d.ready.back();
        d.ready.pop_back();
    } else if (!d.free_.empty()) {
        f = d.free_.back();
        d.free_.pop_back();
    } else {
        return false;
    }
    wr64(f + kVfFilled, 0);
    wr64(f + kVfFilled + 8, 0);
    wr64(f + kVfTime, static_cast<uint64_t>(d.last_time));
    wr8(f + kVfEos, 1);
    d.ready.push_back(f);
    d.out_ready.signal();
    return true;
}

// Milliseconds since `t`, or -1 when it never happened.
long long ms_since(std::chrono::steady_clock::time_point t, std::chrono::steady_clock::time_point now) {
    if (t == std::chrono::steady_clock::time_point{}) return -1;
    return static_cast<long long>(std::chrono::duration_cast<std::chrono::milliseconds>(now - t).count());
}

// Stops a decoder that has made no progress for `stalled` while the game presented `presented` frames. Called with d.mu held. The feed thread
// is waiting on cv; marking failed and notifying makes it return without queueing the held packet. The worker sees failed after its current
// decode and drops that result; Flush can revive the decoder, Finalize still joins the worker. True when an EOS frame could be made.
bool abort_stalled(Decoder& d, std::chrono::steady_clock::duration stalled, uint64_t presented) {
    if (d.failed) return false;
    d.failed = true;
    // The state as the stall found it, before deliver() and the clears below change it: which side stopped is in these numbers. Frames
    // ready but not asked for: the player stopped. Asked for while none were ready and frames wait in pending_free: the GPU serial they
    // wait for did not complete. None ready, free or pending: the player holds every frame.
    const auto now = std::chrono::steady_clock::now();
    const uint64_t oldest = d.pending_free.empty() ? 0 : d.pending_free.front().second;
    char gpu[96];
    if (oldest)
        std::snprintf(gpu, sizeof gpu, "oldest serial %llu %s, last reserved %llu", static_cast<unsigned long long>(oldest),
                      nvn::be::gpu::serial_done(oldest) ? "done" : "not done", static_cast<unsigned long long>(nvn::be::gpu::last_serial()));
    else
        std::snprintf(gpu, sizeof gpu, "last reserved serial %llu", static_cast<unsigned long long>(nvn::be::gpu::last_serial()));
    NXLOG_E(kTag, "movie stall state: queue %zu packets (%zu bytes), decoded %zu, ready %zu, free %zu, pending GPU %zu of %zu frames (%s); "
                  "player asked %lld ms ago, acquired %lld ms ago, released %lld ms ago",
            d.queue.size(), d.queued_bytes, d.decoded.size(), d.ready.size(), d.free_.size(), d.pending_free.size(), d.frames.size(), gpu,
            ms_since(d.last_ask, now), ms_since(d.last_acquire, now), ms_since(d.last_release, now));
    deliver(d);   // harvest any frame whose GPU copy already finished, so EOS has a safe frame when possible
    d.queue.clear();
    d.queued_bytes = 0;
    d.decoded.clear();
    s_queue_packets.store(0);
    s_queue_bytes.store(0);
    const bool eos = queue_eos(d);
    d.eos_owed = !eos;
    d.cv.notify_all();
    d.in_ready.signal();
    if (!eos) d.out_ready.signal();   // wake a player waiting for output even when no frame can carry EOS yet
    NXLOG_E(kTag, "movie decoder made no progress for %.1f s while the game presented %llu frame%s; stopping the movie (stalled queue and "
                  "decoded pictures discarded; EOS %s)",
            std::chrono::duration<double>(stalled).count(), static_cast<unsigned long long>(presented), presented == 1 ? "" : "s",
            eos ? "queued" : "follows on the first frame the player gives back");
    return eos;
}

// Waits for room in the input queue. Normal backpressure continues as long as the worker/decode/frame path makes progress. The wait ends
// early when the held packet's generation is no longer current (Flush), the decoder is stopped, or it has been marked failed. Five seconds with
// no queue pop and no deliver() while the game presents frames is a stall (with the game presenting none, kGameStallLimit): abort_stalled
// stops the movie and this returns Stalled (or StalledEos when an EOS frame could be made).
enum class FeedWait { Keep, Stopped, Flushed, Stalled, StalledEos };
FeedWait wait_for_room(Decoder& d, std::unique_lock<std::mutex>& lk, uint64_t packet_bytes, uint64_t packet_generation) {
    const auto invalidated = [&] {
        return !movie_held_packet_is_valid(packet_generation, d.generation, d.stop);
    };
    const auto has_room = [&] {
        return invalidated() || d.failed || movie_packet_queue_has_room(d.queue.size(), d.queued_bytes, packet_bytes);
    };
    if (invalidated()) return d.stop ? FeedWait::Stopped : FeedWait::Flushed;
    if (has_room()) return d.failed ? FeedWait::Stalled : FeedWait::Keep;
    ++s_blocked;
    const auto t0 = std::chrono::steady_clock::now();
    uint64_t seen = d.progress;
    auto moved = t0;                                     // the last queue pop or deliver()
    uint64_t frames_moved = diag_frame_count();          // frames the game had presented then
    uint64_t frames_window = frames_moved;               // and when the current window began
    auto deadline = t0 + kNoProgressLimit;
    const auto progressed = [&](std::chrono::steady_clock::time_point now) {
        seen = d.progress;
        moved = now;
        frames_moved = frames_window = diag_frame_count();
        deadline = now + kNoProgressLimit;
    };
    while (!has_room()) {
        if (d.cv.wait_until(lk, deadline) == std::cv_status::timeout) {
            const auto now = std::chrono::steady_clock::now();
            if (d.progress != seen) {
                progressed(now);
                continue;
            }
            // Next to no frame presented in this window either: the game itself stands still, and its main thread is the one that releases
            // the frames. Wait on for it, up to kGameStallLimit after the last progress.
            const uint64_t frames = diag_frame_count();
            if (frames - frames_window < kGameRunningFrames && now - moved < kGameStallLimit) {
                frames_window = frames;
                deadline = std::min(now + kNoProgressLimit, moved + kGameStallLimit);
                continue;
            }
            const bool eos = abort_stalled(d, now - moved, frames - frames_moved);
            s_blocked_us += static_cast<uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(now - t0).count());
            return eos ? FeedWait::StalledEos : FeedWait::Stalled;
        }
        if (d.progress != seen) progressed(std::chrono::steady_clock::now());
    }
    s_blocked_us += static_cast<uint64_t>(std::chrono::duration_cast<std::chrono::microseconds>(
        std::chrono::steady_clock::now() - t0).count());
    if (invalidated()) return d.stop ? FeedWait::Stopped : FeedWait::Flushed;
    return d.failed ? FeedWait::Stalled : FeedWait::Keep;
}

// Decodes one packet into pictures (worker thread, without the lock). `decoder` is the worker's own context.
void decode_packet(Decoder& d, Av1& decoder, const Packet& k, std::vector<Picture>& out) {
    if (decoder.ff) {
        std::vector<ffmpeg::Frame> frames;
        ffmpeg::decode(decoder.ff, k.data.data(), k.data.size(), k.time, frames);
        for (ffmpeg::Frame& f : frames) out.push_back(Picture{std::move(f.nv12), f.width, f.height, f.time, false});
        return;
    }
    Dav1dContext* av1 = decoder.dav1d;
    if (!av1) return;
    Dav1dData in{};
    uint8_t* b = dav1d_data_create(&in, k.data.size());
    if (!b) {
        NXLOG_E(kTag, "out of memory for a %zu-byte packet", k.data.size());
        return;
    }
    std::memcpy(b, k.data.data(), k.data.size());
    in.m.timestamp = k.time;
    for (int guard = 0; in.sz && guard < 64; ++guard) {
        const int r = dav1d_send_data(av1, &in);
        if (r < 0 && r != DAV1D_ERR(EAGAIN)) {
            if (!d.warned_error) NXLOG_E(kTag, "AV1 decode error %d (packet at %lld): its data is dropped", r, static_cast<long long>(k.time));
            d.warned_error = true;
            break;
        }
        for (;;) {
            Dav1dPicture p{};
            if (dav1d_get_picture(av1, &p) < 0) break;
            if (p.p.bpc == 8 && p.p.layout == DAV1D_PIXEL_LAYOUT_I420) {
                Picture pic;
                pic.w = static_cast<uint32_t>(p.p.w);
                pic.h = static_cast<uint32_t>(p.p.h);
                pic.time = p.m.timestamp;
                pic.nv12.resize(nv12_size(pic.w, pic.h));
                to_nv12(pic.nv12.data(), pic.w, pic.h, static_cast<const uint8_t*>(p.data[0]), p.stride[0], static_cast<const uint8_t*>(p.data[1]),
                        static_cast<const uint8_t*>(p.data[2]), p.stride[1]);
                out.push_back(std::move(pic));
            } else {
                if (!d.warned_format) NXLOG_W(kTag, "a picture dropped: %d bits, layout %d (only 4:2:0 8-bit is converted)", p.p.bpc, static_cast<int>(p.p.layout));
                d.warned_format = true;
            }
            dav1d_picture_unref(&p);
        }
    }
    if (in.sz) dav1d_data_unref(&in);
}

// Parse WebM AV1CodecConfigurationRecord to extract the config OBUs.
// Returns empty vector if parsing fails or config is not in WebM format.
static std::vector<uint8_t> extract_webm_av1_obus(const std::vector<uint8_t>& config) {
    if (config.size() < 7) return {};   // minimum header size

    // Bit reader for the variable-length header
    const uint8_t* data = config.data();
    size_t bits = config.size() * 8;
    size_t bit_pos = 0;

    auto get_bits = [&](size_t n) -> uint32_t {
        if (bit_pos + n > bits) return 0;
        uint32_t val = 0;
        for (size_t i = 0; i < n; ++i) {
            size_t byte_idx = bit_pos / 8;
            size_t bit_idx = 7 - (bit_pos % 8);
            val = (val << 1) | ((data[byte_idx] >> bit_idx) & 1);
            ++bit_pos;
        }
        return val;
    };

    // AV1CodecConfigurationRecord fields
    const uint32_t marker = get_bits(1);
    const uint32_t version = get_bits(7);
    if (marker != 1 || version != 1) return {};   // not WebM AV1CodecConfigurationRecord

    get_bits(3);   // seq_profile
    get_bits(5);   // seq_level_idx_0
    get_bits(1);   // seq_tier_0
    get_bits(1);   // high_bitdepth
    get_bits(1);   // twelve_bit
    get_bits(1);   // monochrome
    get_bits(1);   // chroma_subsampling_x
    get_bits(1);   // chroma_subsampling_y
    get_bits(2);   // chroma_sample_position
    get_bits(3);   // reserved

    const uint32_t initial_presentation_delay_present = get_bits(1);
    if (initial_presentation_delay_present) {
        get_bits(4);   // initial_presentation_delay_minus_one
    }

    // configOBUsLength (16 bits)
    if (bit_pos + 16 > bits) return {};
    const uint32_t config_obus_length = get_bits(16);

    // Align to byte boundary
    bit_pos = (bit_pos + 7) / 8 * 8;
    size_t byte_pos = bit_pos / 8;

    if (byte_pos + config_obus_length > config.size()) return {};

    std::vector<uint8_t> obus;
    obus.assign(config.begin() + byte_pos, config.begin() + byte_pos + config_obus_length);
    return obus;
}

// Try to extract OBUs from config, handling both WebM AV1CodecConfigurationRecord and MP4 av1C formats.
static std::vector<uint8_t> extract_av1_obus(const std::vector<uint8_t>& config) {
    // First try WebM format (AV1CodecConfigurationRecord)
    auto obus = extract_webm_av1_obus(config);
    if (!obus.empty()) {
        NXLOG_I(kTag, "AV1 config: parsed WebM AV1CodecConfigurationRecord, %zu bytes of OBUs", obus.size());
        return obus;
    }
    // Fall back to MP4 av1C format: 4-byte header (version, profile, level, tier/flags) then OBUs
    if (config.size() > 4) {
        NXLOG_I(kTag, "AV1 config: using MP4 av1C fallback (skip 4 bytes), %zu bytes of OBUs", config.size() - 4);
        return std::vector<uint8_t>(config.begin() + 4, config.end());
    }
    return {};
}

Av1 open_av1(Decoder& d, const std::vector<uint8_t>& config) {
    Av1 out;
    if (g_backend == Av1Backend::Ffmpeg) {
        out.ff = ffmpeg::open(config, g_threads, g_hwaccel);
        if (out.ff) return out;
        static std::atomic<bool> warned{false};
        if (!warned.exchange(true))
            NXLOG_W(kTag, "FFmpeg is selected but its AV1 decoder could not start: using dav1d (put matching avcodec-62.dll, avutil-60.dll and swresample-6.dll in upscalers\\ffmpeg)");
    }
    Dav1dSettings s;
    dav1d_default_settings(&s);
    s.n_threads = g_threads;
    s.max_frame_delay = 1;   // a picture per packet: the player's output event wants no decoder delay
    Dav1dContext* av1 = nullptr;
    if (dav1d_open(&av1, &s) < 0) {
        NXLOG_E(kTag, "dav1d failed to start");
        return out;
    }
    out.dav1d = av1;
    auto obus = extract_av1_obus(config);
    if (!obus.empty()) {
        Packet k;
        k.data = std::move(obus);
        std::vector<Picture> none;
        decode_packet(d, out, k, none);
    }
    return out;
}

void worker_main(Decoder* d) {
    decoder_thread_begin("movie-vdec");
    Av1 av1;
    uint64_t gen = d->generation;
    std::unique_lock<std::mutex> lk(d->mu);
    for (;;) {
        const auto can_work = [&] { return d->stop || (!d->queue.empty() && d->decoded.size() < g_ahead); };
        while (!can_work()) {
            if (d->pending_free.empty()) d->cv.wait(lk);
            else {   // frames wait for the GPU: look again shortly (deliver frees them and wakes the player)
                d->cv.wait_for(lk, std::chrono::milliseconds(1));
                if (!d->decoded.empty() || d->eos_owed) deliver(*d);
            }
        }
        if (d->stop) break;
        if (gen != d->generation) {
            if (av1.dav1d) dav1d_close(&av1.dav1d);
            if (av1.ff) ffmpeg::close(av1.ff);
            av1 = {};
            gen = d->generation;
        }
        Packet k = std::move(d->queue.front());
        d->queue.pop_front();
        d->queued_bytes -= std::min(d->queued_bytes, k.data.size());
        ++d->progress;   // room for the feed thread: reset its no-progress deadline
        s_queue_packets.store(d->queue.size());
        s_queue_bytes.store(d->queued_bytes);
        d->cv.notify_all();   // room for the feed thread, which may be waiting in DecodeAsync
        const bool reopen = !av1.dav1d && !av1.ff && !k.eos && d->mime == kMimeAv1;
        const std::vector<uint8_t> config = reopen ? d->config : std::vector<uint8_t>{};
        lk.unlock();
        std::vector<Picture> out;
        if (reopen) av1 = open_av1(*d, config);
        if (k.eos) {
            if (av1.ff) {
                std::vector<ffmpeg::Frame> frames;
                ffmpeg::drain(av1.ff, frames);
                for (ffmpeg::Frame& f : frames) out.push_back(Picture{std::move(f.nv12), f.width, f.height, f.time, false});
            }
            out.emplace_back();
            out.back().eos = true;
        } else if (av1.dav1d || av1.ff) {
            LARGE_INTEGER t0, t1, hz;
            QueryPerformanceCounter(&t0);
            decode_packet(*d, av1, k, out);
            QueryPerformanceCounter(&t1);
            QueryPerformanceFrequency(&hz);
            const uint64_t us = static_cast<uint64_t>((t1.QuadPart - t0.QuadPart) * 1000000 / hz.QuadPart);
            s_dec_us += us;
            s_dec_n++;
            for (uint64_t m = s_dec_max_us.load(); us > m && !s_dec_max_us.compare_exchange_weak(m, us);) {}
        } else if (d->mime != kMimeAv1 && !d->warned_mime) {
            NXLOG_W(kTag, "video of MIME type %d is not decoded on the host (AV1 only): pictures left out", d->mime);
            d->warned_mime = true;
        }
        lk.lock();
        if (!d->failed && gen == d->generation) {
            for (Picture& p : out) {
                if (!p.eos) {
                    ++d->pictures;
                    s_pictures++;
                }
                d->decoded.push_back(std::move(p));
            }
            deliver(*d);
            s_queue_packets.store(d->queue.size());
            s_queue_bytes.store(d->queued_bytes);
            if (d->queue.empty()) d->in_ready.signal();
        }
    }
    if (av1.dav1d) dav1d_close(&av1.dav1d);
    if (av1.ff) ffmpeg::close(av1.ff);
    diag_unregister_thread();
}

void stop_worker(Decoder& d) {
    {
        std::lock_guard<std::mutex> lk(d.mu);
        d.stop = true;
    }
    d.cv.notify_all();
    if (d.worker.joinable()) d.worker.join();
}

// Attaches a host event to the guest's SystemEventType (managed handle, auto clear: each wait takes one signal), as vi's vsync event.
void attach_event(::Ctx* c, uint8_t* guest_ev, services::ServiceEvent& host) {
    const uint32_t h = host.new_handle();
    if (!h) {
        NXLOG_E(kTag, "no handle for a decoder event");
        return;
    }
    gcall(c, fns().attach_event, reinterpret_cast<uint64_t>(guest_ev), h, 1, kAutoClear);
}

int prop_int(::Ctx* c, Decoder& d, uint64_t prop, const char* name, int fallback) {
    if (!prop) return fallback;
    d.scratch[0] = 0;
    const uint64_t ok = gcall(c, fns().get_int32, prop, reinterpret_cast<uint64_t>(&d.scratch[0]), reinterpret_cast<uint64_t>(name));
    return (ok & 1) && d.scratch[0] > 0 ? d.scratch[0] : fallback;
}

// What the library's own decoder writes (subsdk2 +0x3400, the names checked in its strings), so the player sizes its frames: NV12.
void write_props(::Ctx* c, const Decoder& d, uint64_t prop) {
    if (!prop) return;
    const Fns& f = fns();
    gcall(c, f.set_int32, prop, reinterpret_cast<uint64_t>("width"), static_cast<uint64_t>(d.w));
    gcall(c, f.set_int32, prop, reinterpret_cast<uint64_t>("height"), static_cast<uint64_t>(d.h));
    gcall(c, f.set_size, prop, reinterpret_cast<uint64_t>("output-buf-size"), static_cast<uint64_t>(d.w) * d.h * 3 / 2);
    gcall(c, f.set_size, prop, reinterpret_cast<uint64_t>("input-buf-size"), 0x400000);
    gcall(c, f.set_int32, prop, reinterpret_cast<uint64_t>("color-standard"), static_cast<uint64_t>(d.color[0]));
    gcall(c, f.set_int32, prop, reinterpret_cast<uint64_t>("color-range"), static_cast<uint64_t>(d.color[1]));
    gcall(c, f.set_int32, prop, reinterpret_cast<uint64_t>("color-transfer"), static_cast<uint64_t>(d.color[2]));
}

// ---- movie::VideoDecoder ---------------------------------------------------------------------------------------------------------------

void vd_ctor(::Ctx* c) {   // (self)
    if (A(0)) { wr64(A(0), 0); wr64(A(0) + 8, 0); }
}

// Finalize and the destructor stop the worker but leave the Decoder (and the guest events in it) allocated: the player may still hold the
// events, and a game makes few decoders.
void retire(uint64_t self, Decoder* d) {
    if (d) {
        stop_worker(*d);
        std::lock_guard<std::mutex> lk(d->mu);
        d->queue.clear();
        d->decoded.clear();
        d->ready.clear();
        d->pending_free.clear();
        d->free_.clear();
        d->frames.clear();
    }
    if (self) { wr64(self, 0); wr64(self + 8, 0); }
}

void vd_dtor(::Ctx* c) {   // (self)
    retire(A(0), dec_of(A(0)));
}

void vd_initialize(::Ctx* c) {   // (self, void* mem, size_t size, MediaMimeType, VideoDecoderOutputFormat, VideoCodecMode)
    const uint64_t self = A(0);
    const int mime = static_cast<int>(A(3)), fmt = static_cast<int>(A(4)), mode = static_cast<int>(A(5));
    if (!self) { RET(0); return; }
    auto* d = new Decoder();
    d->mime = mime;
    wr64(self, reinterpret_cast<uint64_t>(d));
    wr64(self + 8, kMarker);
    attach_event(c, d->in_ev, d->in_ready);
    attach_event(c, d->out_ev, d->out_ready);
    gcall(c, fns().create_event, reinterpret_cast<uint64_t>(d->fmt_ev), kAutoClear, 0);   // the format never changes
    d->worker = std::thread(worker_main, d);
    NXLOG_I(kTag, "VideoDecoder: MIME type %d%s, output format %d, mode %d", mime, mime == kMimeAv1 ? " (AV1)" : "", fmt, mode);
    RET(0);
}

void vd_finalize(::Ctx* c) {   // (self)
    retire(A(0), dec_of(A(0)));
    RET(0);
}

void vd_configure(::Ctx* c) {   // (self, const MediaProperty*, const void* config, size_t)
    Decoder* d = dec_of(A(0));
    if (!d) { RET(0); return; }
    const uint8_t* cfg = reinterpret_cast<const uint8_t*>(A(2));
    const size_t n = A(3);
    const int w = prop_int(c, *d, A(1), "width", 1920), h = prop_int(c, *d, A(1), "height", 1080);
    const int cs = prop_int(c, *d, A(1), "color-standard", 0), cr = prop_int(c, *d, A(1), "color-range", 0), ct = prop_int(c, *d, A(1), "color-transfer", 0);
    {
        std::lock_guard<std::mutex> lk(d->mu);
        d->config.assign(cfg && n ? cfg : nullptr, cfg && n ? cfg + n : nullptr);
        d->w = w;
        d->h = h;
        d->color[0] = cs;
        d->color[1] = cr;
        d->color[2] = ct;
    }
    NXLOG_I(kTag, "VideoDecoder: %dx%d, colour standard %d range %d transfer %d, %zu bytes of codec configuration", w, h, cs, cr, ct, n);
    RET(0);
}

void vd_frame_requirements(::Ctx* c) {   // (self, MediaProperty*): also GetOutputFrameProperties
    if (Decoder* d = dec_of(A(0))) write_props(c, *d, A(1));
    RET(0);
}

void vd_register_frames(::Ctx* c) {   // (self, VideoFrame** frames, int n)
    Decoder* d = dec_of(A(0));
    const uint64_t list = A(1);
    int n = static_cast<int>(A(2));
    if (!d || !list || n <= 0) { RET(0); return; }
    if (static_cast<size_t>(n) > kMaxFrames) {
        NXLOG_W(kTag, "%d frames registered, %zu used", n, kMaxFrames);
        n = static_cast<int>(kMaxFrames);
    }
    std::lock_guard<std::mutex> lk(d->mu);
    d->frames.clear();
    d->free_.clear();
    d->pending_free.clear();
    d->ready.clear();
    for (int i = 0; i < n; ++i) d->frames.push_back(rd64(list + 8ull * i));
    for (int i = n - 1; i >= 0; --i) d->free_.push_back(d->frames[i]);
    deliver(*d);
    if (g_log) NXLOG_I(kTag, "VideoDecoder: %d frames registered", n);
    RET(0);
}

void vd_input_event(::Ctx* c) {   // (self, SystemEventType** out)
    Decoder* d = dec_of(A(0));
    if (A(1)) wr64(A(1), d ? reinterpret_cast<uint64_t>(d->in_ev) : 0);
    RET(0);
}
void vd_output_event(::Ctx* c) {
    Decoder* d = dec_of(A(0));
    if (A(1)) wr64(A(1), d ? reinterpret_cast<uint64_t>(d->out_ev) : 0);
    RET(0);
}
void vd_format_event(::Ctx* c) {   // the format never changes
    Decoder* d = dec_of(A(0));
    if (A(1)) wr64(A(1), d ? reinterpret_cast<uint64_t>(d->fmt_ev) : 0);
    RET(0);
}

void vd_start(::Ctx* c) {   // (self)
    if (Decoder* d = dec_of(A(0))) d->in_ready.signal();
    RET(0);
}

void vd_stop(::Ctx* c) { RET(0); }

void vd_decode_async(::Ctx* c) {   // (self, const nn::movie::Buffer*)
    Decoder* d = dec_of(A(0));
    const uint64_t buf = A(1);
    if (!d || !buf) { RET(0); return; }
    const uint64_t data = rd64(buf + kBufData), n = rd64(buf + kBufFilled);
    const int64_t time = static_cast<int64_t>(rd64(buf + kBufTime));
    const uint32_t flags = rd32(buf + kBufFlags);
    FeedWait wait = FeedWait::Keep;
    {
        std::unique_lock<std::mutex> lk(d->mu);
        if (d->failed) wait = FeedWait::Stalled;
        else if (n && data) {
            // Keep the packet: see the queue comment above. This is the guest's feed thread, not the thread that releases frames, so waiting
            // for the worker cannot close a cycle. Normal room waits reset on every queue pop or delivered picture in wait_for_room().
            const uint64_t generation = d->generation;
            wait = wait_for_room(*d, lk, n, generation);
            if (wait == FeedWait::Flushed && g_log)
                NXLOG_I(kTag, "DecodeAsync: stream flush invalidated the held packet at %lld", static_cast<long long>(time));
            if (wait == FeedWait::Keep) {
                Packet k;
                k.data.assign(reinterpret_cast<const uint8_t*>(data), reinterpret_cast<const uint8_t*>(data) + n);
                k.time = time;
                d->queued_bytes += k.data.size();
                d->queue.push_back(std::move(k));
                ++d->packets;
                ++s_packets;
            }
        }
        if (wait == FeedWait::Keep && !d->stop && (flags & kBufFlagEos)) {
            Packet k;
            k.eos = true;
            d->queue.push_back(std::move(k));
        }
        uint64_t maxq = s_max_queue_packets.load();
        if (d->queue.size() > maxq && !s_max_queue_packets.compare_exchange_weak(maxq, d->queue.size())) {}
        s_queue_packets.store(d->queue.size());
        s_queue_bytes.store(d->queued_bytes);
        if (g_log && wait == FeedWait::Keep)
            NXLOG_I(kTag, "DecodeAsync: %llu bytes at %lld%s; queued %zu, decoded %zu, ready %zu, free %zu", static_cast<unsigned long long>(n),
                    static_cast<long long>(time), (flags & kBufFlagEos) ? " (end of stream)" : "", d->queue.size(), d->decoded.size(),
                    d->ready.size(), d->free_.size());
    }
    d->cv.notify_all();
    if (wait == FeedWait::Stalled) { RET(kResultDecoderStalled); return; }
    RET(0);
}

uint32_t acquire(Decoder* d, uint64_t* out, int* idx) {
    std::lock_guard<std::mutex> lk(d->mu);
    const auto now = std::chrono::steady_clock::now();
    d->last_ask = now;
    if (d->ready.empty()) {
        *out = 0;
        return kResultNoFrame;
    }
    d->last_acquire = now;
    *out = d->ready.front();
    d->ready.pop_front();
    if (!d->ready.empty()) d->out_ready.signal();   // one signal per wait: the next ready frame needs its own
    if (idx) {
        *idx = -1;
        for (size_t i = 0; i < d->frames.size(); ++i)
            if (d->frames[i] == *out) *idx = static_cast<int>(i);
    }
    return 0;
}

void vd_acquire(::Ctx* c) {   // (self, VideoFrame** out)
    Decoder* d = dec_of(A(0));
    uint64_t f = 0;
    const uint32_t r = d ? acquire(d, &f, nullptr) : kResultNoFrame;
    if (A(1)) wr64(A(1), f);
    if (f) {
        s_acquired++;
        if (g_log) NXLOG_I(kTag, "AcquireVideoFrame: %llx at %lld (luma %llu, chroma %llu bytes%s)", static_cast<unsigned long long>(f),
                           static_cast<long long>(rd64(f + kVfTime)), static_cast<unsigned long long>(rd64(f + kVfFilled)),
                           static_cast<unsigned long long>(rd64(f + kVfFilled + 8)), (rd32(f + kVfEos) & 0xff) ? ", end of stream" : "");
    }
    stats_tick();
    RET(r);
}

void vd_acquire_index(::Ctx* c) {   // (self, VideoFrame** out, int* index)
    Decoder* d = dec_of(A(0));
    uint64_t f = 0;
    int idx = -1;
    const uint32_t r = d ? acquire(d, &f, &idx) : kResultNoFrame;
    if (A(1)) wr64(A(1), f);
    if (A(2)) std::memcpy(reinterpret_cast<void*>(A(2)), &idx, 4);
    if (f) s_acquired++;
    RET(r);
}

void vd_release(::Ctx* c) {   // (self, VideoFrame*)
    Decoder* d = dec_of(A(0));
    const uint64_t f = A(1);
    if (!d) { RET(0); return; }
    {
        std::lock_guard<std::mutex> lk(d->mu);
        const bool known = std::find(d->frames.begin(), d->frames.end(), f) != d->frames.end();
        const bool already = std::find(d->free_.begin(), d->free_.end(), f) != d->free_.end();
        const bool pending = std::any_of(d->pending_free.begin(), d->pending_free.end(), [f](const auto& p) { return p.first == f; });
        if (f && known && !already && !pending && d->free_.size() + d->pending_free.size() < kMaxFrames)
            d->pending_free.emplace_back(f, nvn::be::gpu::last_serial());
        s_released++;
        d->last_release = std::chrono::steady_clock::now();
        if (g_log) NXLOG_I(kTag, "ReleaseVideoFrame: %llx%s; queued %zu, decoded %zu, ready %zu, free %zu", static_cast<unsigned long long>(f),
                           known ? (already ? " (already free)" : "") : " (not registered)", d->queue.size(), d->decoded.size(), d->ready.size(),
                           d->free_.size());
        deliver(*d);
        // A frame still waiting for the GPU: the worker polls pending frames, so the owed EOS follows once the GPU passed it.
        if (d->eos_owed) d->cv.notify_all();
    }
    RET(0);
}

void vd_flush(::Ctx* c) {   // (self)
    Decoder* d = dec_of(A(0));
    if (!d) { RET(0); return; }
    {
        std::lock_guard<std::mutex> lk(d->mu);
        ++d->generation;
        d->failed = false;   // a flush starts a new stream; the worker was left alive by abort_stalled()
        d->eos_owed = false;
        ++d->progress;
        d->clk_on = false;
        d->last_time = 0;
        d->queue.clear();
        d->queued_bytes = 0;
        s_queue_packets.store(0);
        s_queue_bytes.store(0);
        d->decoded.clear();
        d->ready.clear();
        d->pending_free.clear();
        d->free_.clear();
        for (auto it = d->frames.rbegin(); it != d->frames.rend(); ++it) d->free_.push_back(*it);
    }
    d->cv.notify_all();
    if (g_log) NXLOG_I(kTag, "VideoDecoder flushed");
    d->in_ready.signal();
    RET(0);
}

void vd_set_core_mask(::Ctx* c) { (void)c; }
void vd_get_core_mask(::Ctx* c) { RET(1); }
void vd_set_priority(::Ctx* c) { (void)c; }
void vd_get_priority(::Ctx* c) { RET(16); }

struct Import {
    const char* name;
    HleFn fn;
};
const Import kImports[] = {
    {"_ZN5movie12VideoDecoderC1Ev", vd_ctor},
    {"_ZN5movie12VideoDecoderC2Ev", vd_ctor},
    {"_ZN5movie12VideoDecoderD1Ev", vd_dtor},
    {"_ZN5movie12VideoDecoderD2Ev", vd_dtor},
    {"_ZN5movie12VideoDecoder10InitializeEPvmN2nn5movie13MediaMimeTypeENS_24VideoDecoderOutputFormatENS_14VideoCodecModeE", vd_initialize},
    {"_ZN5movie12VideoDecoder8FinalizeEv", vd_finalize},
    {"_ZN5movie12VideoDecoder9ConfigureEPKN2nn5movie13MediaPropertyEPKvm", vd_configure},
    {"_ZN5movie12VideoDecoder25GetVideoFrameRequirementsEPN2nn5movie13MediaPropertyE", vd_frame_requirements},
    {"_ZN5movie12VideoDecoder24GetOutputFramePropertiesEPN2nn5movie13MediaPropertyE", vd_frame_requirements},
    {"_ZN5movie12VideoDecoder18RegisterVideoFrameEPPNS_10VideoFrameEi", vd_register_frames},
    {"_ZN5movie12VideoDecoder18GetInputReadyEventEPPN2nn2os15SystemEventTypeE", vd_input_event},
    {"_ZN5movie12VideoDecoder19GetOutputReadyEventEPPN2nn2os15SystemEventTypeE", vd_output_event},
    {"_ZN5movie12VideoDecoder21GetFormatChangedEventEPPN2nn2os15SystemEventTypeE", vd_format_event},
    {"_ZN5movie12VideoDecoder5StartEv", vd_start},
    {"_ZN5movie12VideoDecoder4StopEv", vd_stop},
    {"_ZN5movie12VideoDecoder11DecodeAsyncEPKN2nn5movie6BufferE", vd_decode_async},
    {"_ZN5movie12VideoDecoder17AcquireVideoFrameEPPNS_10VideoFrameE", vd_acquire},
    {"_ZN5movie12VideoDecoder17AcquireVideoFrameEPPNS_10VideoFrameEPi", vd_acquire_index},
    {"_ZN5movie12VideoDecoder17ReleaseVideoFrameEPNS_10VideoFrameE", vd_release},
    {"_ZN5movie12VideoDecoder5FlushEv", vd_flush},
    {"_ZN5movie12VideoDecoder17SetThreadCoreMaskEm", vd_set_core_mask},
    {"_ZN5movie12VideoDecoder17GetThreadCoreMaskEv", vd_get_core_mask},
    {"_ZN5movie12VideoDecoder17SetThreadPriorityEi", vd_set_priority},
    {"_ZN5movie12VideoDecoder17GetThreadPriorityEv", vd_get_priority},
};

}  // namespace

bool install_movie_decoder_hle() {
    static std::once_flag once;
    std::call_once(once, [] {
        const game::Info& gi = game::info();
        if (gi.platform != game::Platform::Switch2 || !gi.movie_video_decoder) return;
        if (config_get_str("media", "movies", "host") == "skip") {
            NXLOG_I(kTag, "[media] movies = skip: every movie fails to open and is skipped");
            return;
        }
        std::string decoder = config_get_str("media", "movie_decoder", "dav1d");
        for (char& ch : decoder) ch = static_cast<char>(std::tolower(static_cast<unsigned char>(ch)));
        if (decoder == "ffmpeg") {
            g_hwaccel = config_get_bool("media", "movie_hwaccel", true);
            if (ffmpeg::available()) g_backend = Av1Backend::Ffmpeg;
            else             NXLOG_W(kTag, "[media] movie_decoder = ffmpeg, but the FFmpeg DLLs are missing or the wrong version: using dav1d (avcodec-62.dll, avutil-60.dll and swresample-6.dll in upscalers\\ffmpeg)");
        } else if (decoder != "dav1d" && decoder != "cpu") {
            NXLOG_W(kTag, "[media] movie_decoder = %s is not dav1d or ffmpeg: using dav1d", decoder.c_str());
        }
        g_log = config_get_bool("media", "video_decoder_log", false);
        const int64_t auto_threads = std::clamp<int64_t>(static_cast<int64_t>(std::thread::hardware_concurrency()) / 2, 4, 8);
        const int64_t threads = config_get_int("media", "video_decoder_threads", 0);
        g_threads = static_cast<int>(threads <= 0 ? auto_threads : std::clamp<int64_t>(threads, 1, 16));
        g_ahead = static_cast<size_t>(std::clamp<int64_t>(config_get_int("media", "video_decode_ahead", 30), 4, 120));
        {
            std::string p = config_get_str("media", "video_decoder_priority", "above");
            for (char& ch : p) ch = static_cast<char>(std::tolower(static_cast<unsigned char>(ch)));
            if (p == "normal") g_priority = THREAD_PRIORITY_NORMAL;
            else if (p == "highest") g_priority = THREAD_PRIORITY_HIGHEST;
            else {
                if (p != "above" && p != "above_normal") NXLOG_W(kTag, "[media] video_decoder_priority = %s is not above, highest or normal: above", p.c_str());
                g_priority = THREAD_PRIORITY_ABOVE_NORMAL;
            }
        }
        dav1d_nxrt_thread_hook = dav1d_thread_hook;
        for (const Import& im : kImports) {
            hle_register(im.name, im.fn);
            module_override_symbol(im.name);
        }
        g_enabled = true;
        const char* prio_name = g_priority == THREAD_PRIORITY_NORMAL ? "normal" : g_priority == THREAD_PRIORITY_HIGHEST ? "highest" : "above normal";
        if (g_backend == Av1Backend::Ffmpeg) {
            NXLOG_I(kTag, "movie::VideoDecoder on the host: AV1 through FFmpeg %s (%s, %d threads at %s priority, %zu pictures ahead), %zu imports routed",
                    ffmpeg::version(), g_hwaccel ? "D3D11VA requested" : "software", g_threads, prio_name, g_ahead,
                    sizeof kImports / sizeof kImports[0]);
        } else {
#if NXRT_DAV1D_ASM
            NXLOG_I(kTag, "movie::VideoDecoder on the host: AV1 through dav1d %s with x86-64 assembly (%d threads at %s priority, %zu pictures ahead), %zu imports routed",
                    dav1d_version(), g_threads, prio_name, g_ahead, sizeof kImports / sizeof kImports[0]);
#else
            // A runtime built without NASM: 1440p pictures take 30-50 ms instead of 10-15, so the movies play below 30 fps (choppy cutscenes).
            NXLOG_W(kTag, "movie::VideoDecoder on the host: AV1 through dav1d %s, C only: built without NASM, movies decode several times slower "
                    "and may play below their frame rate (%d threads at %s priority, %zu pictures ahead), %zu imports routed", dav1d_version(), g_threads, prio_name, g_ahead, sizeof kImports / sizeof kImports[0]);
#endif
        }
    });
    return g_enabled;
}

uint64_t movie_frames_acquired() { return s_acquired.load(std::memory_order_relaxed); }

#else   // built without dav1d

bool install_movie_decoder_hle() {
    static bool logged = false;
    if (!logged && game::info().movie_video_decoder) {
        logged = true;
        NXLOG_W("movie", "built without dav1d (NXRT_HAVE_DAV1D): movies are skipped");
    }
    return false;
}

uint64_t movie_frames_acquired() { return 0; }

#endif

bool movie_decoder_enabled() { return g_enabled; }

}  // namespace nxrt::media
