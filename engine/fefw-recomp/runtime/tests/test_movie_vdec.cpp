// SPDX-License-Identifier: GPL-3.0-or-later
// media/movie_vdec.cpp: the bounds of its packet queue; the dav1d thread hook (third_party/dav1d/src/win32/thread.c) it uses to give the AV1
// task threads their priority and register them for the diagnostics (#40), which runs on each task thread once as it starts and once before
// it ends; and the frame handoff between the player's threads and the decode worker.
#include <windows.h>

#include <atomic>
#include <mutex>
#include <set>

#include "nxrt/movie_vdec.h"
#include "test_framework.h"

NXTEST_GROUP(media_vdec)   // outside the #if: CMake registers the group from this line whether or not dav1d is built

// A packet held while DecodeAsync waits for room belongs to the stream generation captured at entry. Backpressure alone must keep it; a Flush
// (new generation) or Stop must discard it, or a seek would feed an old non-keyframe into the new stream.
NXTEST(held_packet_is_invalidated_only_by_flush_or_stop) {
    using namespace nxrt::media;
    CHECK(movie_held_packet_is_valid(7, 7, false));    // normal backpressure: generation unchanged
    CHECK(!movie_held_packet_is_valid(7, 8, false));   // Flush: new queue generation
    CHECK(!movie_held_packet_is_valid(7, 7, true));    // Stop
    CHECK(!movie_held_packet_is_valid(7, 8, true));
}

// The input queue bound that stopped the opening movie's 3301-packet backlog (issue #29) and the byte cap that keeps large packets from
// taking the same room as hundreds of small ones. The real DecodeAsync waits on exactly this predicate.
NXTEST(movie_packet_queue_is_bounded) {
    using namespace nxrt::media;
    CHECK(movie_packet_queue_has_room(0, 0, 1));
    CHECK(movie_packet_queue_has_room(kMovieMaxQueuedPackets - 1, 0, 1));
    CHECK(!movie_packet_queue_has_room(kMovieMaxQueuedPackets, 0, 1));
    CHECK(movie_packet_queue_has_room(1, kMovieMaxQueuedBytes - 10, 10));
    CHECK(!movie_packet_queue_has_room(1, kMovieMaxQueuedBytes - 9, 10));
    // One packet larger than the byte cap is let in when the queue is empty, rather than making the movie unable to start.
    CHECK(movie_packet_queue_has_room(0, 0, kMovieMaxQueuedBytes + 1));
}

#if __has_include(<dav1d/dav1d.h>)
#include <dav1d/dav1d.h>

#include <algorithm>
#include <chrono>
#include <condition_variable>
#include <cstring>
#include <deque>
#include <fstream>
#include <iterator>
#include <thread>
#include <vector>

#include "../src/media/movie_vdec_internal.h"

extern "C" void (*dav1d_nxrt_thread_hook)(int begin);

namespace {
std::atomic<int> g_begins{0}, g_ends{0}, g_prio_ok{0};
std::mutex g_mu;
std::set<DWORD> g_tids;

void hook(int begin) {
    if (begin) {
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_ABOVE_NORMAL);
        if (GetThreadPriority(GetCurrentThread()) == THREAD_PRIORITY_ABOVE_NORMAL) g_prio_ok++;
        std::lock_guard<std::mutex> lk(g_mu);
        g_tids.insert(GetCurrentThreadId());
        g_begins++;
    } else {
        g_ends++;
    }
}
}  // namespace

NXTEST(dav1d_thread_hook_runs_on_each_task_thread) {
    g_begins = g_ends = g_prio_ok = 0;
    g_tids.clear();
    dav1d_nxrt_thread_hook = hook;
    Dav1dSettings s;
    dav1d_default_settings(&s);
    s.n_threads = 4;
    s.max_frame_delay = 1;
    Dav1dContext* c = nullptr;
    REQUIRE(dav1d_open(&c, &s) == 0);
    // dav1d_open creates its task threads (n_threads of them) but does not wait for them to run: wait here, 5 s at most
    for (int i = 0; i < 500 && g_begins.load() < 4; ++i) Sleep(10);
    const int begun = g_begins.load();
    CHECK_EQ(begun, 4);
    CHECK_EQ(g_prio_ok.load(), begun);
    {
        std::lock_guard<std::mutex> lk(g_mu);
        CHECK_EQ(static_cast<int>(g_tids.size()), begun);   // a thread of its own each time
        CHECK(g_tids.count(GetCurrentThreadId()) == 0);     // never the caller's thread
    }
    dav1d_close(&c);
    CHECK_EQ(g_ends.load(), begun);                      // dav1d_close joins them: every end ran
    dav1d_nxrt_thread_hook = nullptr;
}

NXTEST(dav1d_without_hook_still_opens) {
    dav1d_nxrt_thread_hook = nullptr;
    Dav1dSettings s;
    dav1d_default_settings(&s);
    s.n_threads = 2;
    Dav1dContext* c = nullptr;
    REQUIRE(dav1d_open(&c, &s) == 0);
    dav1d_close(&c);
    CHECK(c == nullptr);
}

// ---- the frame handoff (src/media/movie_vdec_internal.h) -------------------------------------------------------------------------------
// The test movie: 120 pictures of 64x64 AV1 in IVF, made with
//   ffmpeg -f lavfi -i testsrc2=size=64x64:rate=30 -frames:v 120 -c:v libaom-av1 -cpu-used 8 -crf 63 -g 60 -pix_fmt yuv420p -f ivf movie_64x64.ivf
// As in the game's movies, every other packet only shows a hidden frame decoded earlier (5 bytes).

namespace {
namespace vd = nxrt::media::vdec;
using namespace std::chrono_literals;

constexpr const char* kMovie = NXRT_TEST_DATA_DIR "/movie_64x64.ivf";

std::vector<std::vector<uint8_t>> read_ivf(const char* path) {
    std::ifstream in(path, std::ios::binary);
    const std::vector<uint8_t> file((std::istreambuf_iterator<char>(in)), std::istreambuf_iterator<char>());
    std::vector<std::vector<uint8_t>> packets;
    size_t at = 32;   // the file header; each packet follows a 12-byte header: its size, then a 64-bit timestamp
    while (at + 12 <= file.size()) {
        uint32_t size = 0;
        std::memcpy(&size, file.data() + at, 4);
        at += 12;
        if (size > file.size() - at) break;
        packets.emplace_back(file.begin() + at, file.begin() + at + size);
        at += size;
    }
    return packets;
}

// The GPU as the decoder sees it: the serial of the player's last copy, and how far the GPU got.
std::atomic<uint64_t> g_issued{0}, g_completed{0};
uint64_t fake_last_serial() { return g_issued; }
bool fake_serial_done(uint64_t serial) { return serial <= g_completed; }

struct FakeGpu {   // puts the fake fence in for one test
    vd::GpuFence saved = vd::g_gpu;
    FakeGpu() {
        g_issued = 0;
        g_completed = 0;
        vd::g_gpu = {fake_last_serial, fake_serial_done};
    }
    ~FakeGpu() { vd::g_gpu = saved; }
};

// A VideoFrame as the player registers it: the fields the decoder reads and writes, and the buffer for a 64x64 NV12 picture.
struct TestFrame {
    alignas(16) uint8_t fields[0x90] = {};
    std::vector<uint8_t> nv12 = std::vector<uint8_t>(64 * 64 * 3 / 2);
    TestFrame() {
        put(vd::kVfBuf, reinterpret_cast<uint64_t>(nv12.data()));
        put(vd::kVfSize, nv12.size());
    }
    uint64_t addr() const { return reinterpret_cast<uint64_t>(fields); }
    void put(uint64_t at, uint64_t value) { std::memcpy(fields + at, &value, 8); }
};

int64_t time_of(uint64_t frame) {
    int64_t t = 0;
    std::memcpy(&t, reinterpret_cast<const void*>(frame + vd::kVfTime), 8);
    return t;
}
bool is_eos(uint64_t frame) { return reinterpret_cast<const uint8_t*>(frame)[vd::kVfEos] != 0; }

template <class Done>
bool wait_until(Done done, std::chrono::milliseconds limit) {
    const auto end = std::chrono::steady_clock::now() + limit;
    while (!done()) {
        if (std::chrono::steady_clock::now() >= end) return false;
        std::this_thread::sleep_for(1ms);
    }
    return true;
}

// Takes a ready frame as the player's handler does; 0 when none came within `limit`. acquire() only hands out frames that are ready, so
// asking moves nothing along.
uint64_t wait_for_frame(vd::Decoder& d, std::chrono::milliseconds limit) {
    uint64_t frame = 0;
    wait_until([&] { return vd::acquire(d, &frame, nullptr) == 0; }, limit);
    return frame;
}

struct Worker {   // the decoder's worker for one test, stopped however the test ends
    vd::Decoder& d;
    explicit Worker(vd::Decoder& decoder) : d(decoder) { vd::start_worker(d); }
    ~Worker() { vd::stop_worker(d); }
};
}  // namespace

// The main thread gives a frame back while its copy to the textures is still on the GPU, and the worker, with nothing else to do, sleeps.
// The frame must take the waiting picture once the GPU passed it, without another call: in the game the handler that would make one was
// blocked in DecodeAsync, and the movie stood still until the stall guard stopped it.
NXTEST(released_frame_takes_the_waiting_picture_once_the_gpu_passed_it) {
    const auto packets = read_ivf(kMovie);
    REQUIRE(packets.size() >= 2);
    FakeGpu gpu;
    vd::Decoder d;
    d.mime = vd::kMimeAv1;
    TestFrame frame;
    const uint64_t frames[] = {frame.addr()};
    vd::register_frames(d, frames, 1);
    Worker worker(d);

    CHECK_EQ(vd::decode_async(d, packets[0].data(), packets[0].size(), 0, false), 0u);
    CHECK_EQ(vd::decode_async(d, packets[1].data(), packets[1].size(), 1000, false), 0u);
    const uint64_t first = wait_for_frame(d, 2s);
    REQUIRE(first == frame.addr());
    CHECK_EQ(time_of(first), 0);
    REQUIRE(wait_until([&] {   // the second picture waits for a frame, and the worker sleeps
        std::lock_guard<std::mutex> lk(d.mu);
        return d.queue.empty() && d.decoded.size() == 1;
    }, 2s));

    g_issued = 1;   // the copy of the first picture
    vd::release(d, first);
    CHECK_EQ(wait_for_frame(d, 50ms), 0u);   // not before the GPU passed it
    g_completed = 1;
    const uint64_t second = wait_for_frame(d, 2s);
    REQUIRE(second == frame.addr());
    CHECK_EQ(time_of(second), 1000);
}

// The game's player: its handler thread feeds the packets and takes a frame only when the main thread has none waiting (guest-118 in the
// logs), so ready frames wait; the main thread copies each picture to the GPU and gives the frame back, and the GPU finishes the copies a
// little later. The cushion of decoded pictures and the packet queue fill up, so DecodeAsync waits for room. Every picture must come through
// once and in order, then the end of the stream; no frame may change while the player holds it or before the GPU finished copying it.
NXTEST(player_threads_get_every_picture_once_and_in_order) {
    const auto packets = read_ivf(kMovie);
    REQUIRE(packets.size() == 120);
    FakeGpu gpu;
    vd::Decoder d;
    d.mime = vd::kMimeAv1;
    TestFrame frames[4];
    const uint64_t list[] = {frames[0].addr(), frames[1].addr(), frames[2].addr(), frames[3].addr()};
    vd::register_frames(d, list, 4);
    Worker worker(d);

    // The GPU finishes the copies issued so far a millisecond later; until then each frame must still hold the picture it copies.
    struct Copy {
        uint64_t frame;
        int64_t time;
        bool eos;
    };
    std::mutex gpu_mu;
    std::vector<Copy> copies;   // issued, not finished
    std::atomic<int> refilled_early{0};
    std::atomic<bool> running{true};
    std::thread gpu_thread([&] {
        while (running) {
            std::this_thread::sleep_for(1ms);
            std::lock_guard<std::mutex> lk(gpu_mu);
            for (const Copy& c : copies)
                if (time_of(c.frame) != c.time || is_eos(c.frame) != c.eos) ++refilled_early;
            copies.clear();
            g_completed = g_issued.load();
        }
    });

    struct Seen {
        int64_t time;
        bool eos, changed_while_held;
    };
    std::vector<Seen> seen;   // main thread's, read after it ends
    std::mutex mu;
    std::condition_variable cv;
    std::deque<uint64_t> taken;   // frames the handler took, for the main thread
    bool stop_main = false;
    std::atomic<bool> got_eos{false};
    std::thread main_thread([&] {
        for (;;) {
            uint64_t f = 0;
            {
                std::unique_lock<std::mutex> lk(mu);
                cv.wait(lk, [&] { return !taken.empty() || stop_main; });
                if (taken.empty()) return;
                f = taken.front();
                taken.pop_front();
            }
            Seen s{time_of(f), is_eos(f), false};
            std::this_thread::sleep_for(100us);   // the copy to the textures
            s.changed_while_held = time_of(f) != s.time || is_eos(f) != s.eos;
            seen.push_back(s);
            {
                std::lock_guard<std::mutex> lk(gpu_mu);
                copies.push_back({f, s.time, s.eos});
                ++g_issued;
            }
            vd::release(d, f);
            if (s.eos) got_eos = true;
        }
    });

    const auto take_ready = [&] {   // one frame waits for the main thread at most
        std::lock_guard<std::mutex> lk(mu);
        uint64_t f = 0;
        if (taken.empty() && vd::acquire(d, &f, nullptr) == 0) {
            taken.push_back(f);
            cv.notify_one();
        }
    };
    // Timestamps 1000 apart: learn_unit() takes that spacing for no unit, so the movie clock stays off and skips no picture.
    size_t fed = 0;
    for (; fed < packets.size(); ++fed) {
        const bool last = fed + 1 == packets.size();
        if (vd::decode_async(d, packets[fed].data(), packets[fed].size(), static_cast<int64_t>(fed) * 1000, last) != 0) break;
        take_ready();
    }
    CHECK_EQ(fed, packets.size());   // DecodeAsync answers failure once the stall guard stopped the movie
    CHECK(wait_until([&] {
        take_ready();
        return got_eos.load();
    }, 10s));
    {
        std::lock_guard<std::mutex> lk(mu);
        stop_main = true;
    }
    cv.notify_all();
    main_thread.join();
    running = false;
    gpu_thread.join();

    std::vector<int64_t> times, expected;
    for (const Seen& s : seen)
        if (!s.eos) times.push_back(s.time);
    for (size_t i = 0; i < packets.size(); ++i) expected.push_back(static_cast<int64_t>(i) * 1000);
    CHECK(times == expected);
    CHECK_EQ(std::count_if(seen.begin(), seen.end(), [](const Seen& s) { return s.eos; }), 1);
    CHECK(!seen.empty() && seen.back().eos);
    CHECK(std::none_of(seen.begin(), seen.end(), [](const Seen& s) { return s.changed_while_held; }));
    CHECK_EQ(refilled_early.load(), 0);
}
#else   // built with -DNXRT_DAV1D=OFF: one skipped case, so `nxrt_tests media_vdec` still finds a test
NXTEST(dav1d_thread_hook_runs_on_each_task_thread) { nxtest::skip("built without dav1d (NXRT_DAV1D=OFF)"); }
#endif
