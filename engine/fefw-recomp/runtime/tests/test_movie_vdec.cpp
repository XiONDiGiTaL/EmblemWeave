// SPDX-License-Identifier: GPL-3.0-or-later
// The dav1d thread hook (third_party/dav1d/src/win32/thread.c) that media/movie_vdec.cpp uses to give the AV1 task threads their priority
// and register them for the diagnostics (#40): it runs on each task thread, once as it starts and once before it ends.
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
#else   // built with -DNXRT_DAV1D=OFF: one skipped case, so `nxrt_tests media_vdec` still finds a test
NXTEST(dav1d_thread_hook_runs_on_each_task_thread) { nxtest::skip("built without dav1d (NXRT_DAV1D=OFF)"); }
#endif
