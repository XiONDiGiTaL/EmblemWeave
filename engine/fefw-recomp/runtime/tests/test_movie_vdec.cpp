// SPDX-License-Identifier: GPL-3.0-or-later
// The dav1d thread hook (third_party/dav1d/src/win32/thread.c) that media/movie_vdec.cpp uses to give the AV1 task threads their priority
// and register them for the diagnostics (#40): it runs on each task thread, once as it starts and once before it ends.
#include <windows.h>

#include <atomic>
#include <mutex>
#include <set>

#include "test_framework.h"

NXTEST_GROUP(media_vdec)   // outside the #if: CMake registers the group from this line whether or not dav1d is built

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
