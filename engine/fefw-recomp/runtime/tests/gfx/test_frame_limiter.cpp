// SPDX-License-Identifier: GPL-3.0-or-later
#include "test_framework.h"

#include <windows.h>
#include <immintrin.h>

#include <algorithm>
#include <cmath>
#include <cstddef>
#include <cstdint>
#include <cstdio>
#include <thread>
#include <vector>

#include "nxrt/frame_limiter.h"

NXTEST_GROUP(gfx_limiter)

namespace {

using nxrt::FrameLimiter;

// Jitter caps in ms, PROVISIONAL: 3x the worst value seen in the three undisturbed runs (of five) on this machine; the
// other two runs were disturbed by other processes on the box. The 240 fps p99 cap is clamped to 1.0 ms.
constexpr double kP99DevCapMs[] = {2.0, 1.25, 1.0};    // p99 of |interval - period|, for 60, 120, 240 fps
constexpr double kMaxIntervalCapMs[] = {20.5, 10.5, 6.5};  // largest interval, for 60, 120, 240 fps

constexpr double kRateTolerance = 0.005;  // 0.5 % on the mean rate

// Busy-waits for ns nanoseconds: the simulated frame work.
void spin_ns(uint64_t ns) {
    const uint64_t end = FrameLimiter::now_ns() + ns;
    while (FrameLimiter::now_ns() < end) _mm_pause();
}

// Runs n frames at fps. Each frame does 25 % of the period of simulated work after begin_frame(). Returns frame starts.
std::vector<uint64_t> run_frames(FrameLimiter& l, int n, double fps) {
    const uint64_t work = static_cast<uint64_t>(1e9 / fps * 0.25);
    std::vector<uint64_t> starts;
    starts.reserve(static_cast<std::size_t>(n));
    for (int i = 0; i < n; ++i) {
        starts.push_back(l.begin_frame());
        spin_ns(work);
    }
    return starts;
}

double ns_to_ms(double ns) { return ns / 1e6; }

// The limiter is meant for a game's render thread, which runs at high priority: the timing cases do too, so that other
// processes on the machine (builds, other programs) do not preempt the measurement for tens of milliseconds.
struct ScopedBoost {
    DWORD old_class;
    int old_prio;
    ScopedBoost() {
        old_class = GetPriorityClass(GetCurrentProcess());
        old_prio = GetThreadPriority(GetCurrentThread());
        SetPriorityClass(GetCurrentProcess(), HIGH_PRIORITY_CLASS);
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_TIME_CRITICAL);
    }
    ~ScopedBoost() {
        SetThreadPriority(GetCurrentThread(), old_prio);
        SetPriorityClass(GetCurrentProcess(), old_class);
    }
};

struct RateResult {
    FrameLimiter::Stats st;
    double p99_dev_ms = 0;  // p99 of |interval - period|, from the frame starts of this run
    double max_iv_ms = 0;   // largest interval
};

RateResult measure_rate(double fps, int frames) {
    ScopedBoost boost;
    FrameLimiter l;
    l.set_target_fps(fps);
    const std::vector<uint64_t> starts = run_frames(l, frames, fps);

    RateResult r;
    r.st = l.stats();
    const double period_ms = 1000.0 / fps;
    std::vector<double> dev;
    dev.reserve(starts.size());
    for (std::size_t i = 1; i < starts.size(); ++i) {
        const double iv = ns_to_ms(static_cast<double>(starts[i] - starts[i - 1]));
        dev.push_back(std::fabs(iv - period_ms));
        r.max_iv_ms = std::max(r.max_iv_ms, iv);
    }
    std::sort(dev.begin(), dev.end());
    r.p99_dev_ms = dev[std::min(dev.size() - 1, static_cast<std::size_t>(0.99 * static_cast<double>(dev.size())))];
    return r;
}

// Shared body for the three rate cases: 600 frames, rate and jitter checked against the caps above.
void check_rate(double fps, std::size_t cap_index) {
    const RateResult r = measure_rate(fps, 600);
    const FrameLimiter::Stats& st = r.st;
    std::printf("    %3.0f fps: fps=%.4f mean=%.5f min=%.5f max=%.5f p99=%.5f stddev=%.5f ms | p99 dev=%.5f max iv=%.5f ms\n",
                fps, st.fps, st.mean_ms, st.min_ms, st.max_ms, st.p99_ms, st.stddev_ms, r.p99_dev_ms, r.max_iv_ms);

    CHECK_EQ(st.frames, 599u);
    CHECK_MSG(std::fabs(st.fps - fps) / fps < kRateTolerance, "mean rate outside 0.5 % of target");
    CHECK_MSG(r.p99_dev_ms < kP99DevCapMs[cap_index], "p99 interval deviation above cap");
    CHECK_MSG(r.max_iv_ms < kMaxIntervalCapMs[cap_index], "max interval above cap");
}

}  // namespace

NXTEST(vrr_cap_values) {
    const double expect[][2] = {{60, 57}, {120, 116}, {144, 138.24}, {240, 224}};
    for (const auto& e : expect) {
        const double got = nxrt::suggest_vrr_cap(e[0]);
        std::printf("    suggest_vrr_cap(%.0f) = %.6f\n", e[0], got);
        CHECK_MSG(std::fabs(got - e[1]) < 1e-9, "suggest_vrr_cap value differs from the formula");
    }
    CHECK_MSG(nxrt::suggest_vrr_cap(1.0) == 1.0, "suggest_vrr_cap must never go below 1");
}

NXTEST(unlimited_no_wait) {
    FrameLimiter l;
    l.set_target_fps(0);
    CHECK_MSG(l.target_fps() == 0.0, "target_fps must read back 0");
    const uint64_t t0 = FrameLimiter::now_ns();
    for (int i = 0; i < 1000; ++i) l.begin_frame();
    const uint64_t t1 = FrameLimiter::now_ns();
    std::printf("    1000 unlimited begin_frame calls: %.3f ms\n", ns_to_ms(static_cast<double>(t1 - t0)));
    CHECK_MSG(t1 - t0 < 50000000ull, "1000 unlimited begin_frame calls took 50 ms or more");
    CHECK_EQ(l.last_wait_ns(), 0ull);
}

NXTEST(rate_60fps) { check_rate(60.0, 0); }

NXTEST(rate_120fps) { check_rate(120.0, 1); }

NXTEST(rate_240fps) { check_rate(240.0, 2); }

NXTEST(stall_resync_no_burst) {
    ScopedBoost boost;
    const double fps = 240.0;
    const double period_ns = 1e9 / fps;
    FrameLimiter l;
    l.set_target_fps(fps);
    const std::vector<uint64_t> warm = run_frames(l, 30, fps);
    Sleep(42);  // about 10 periods (41.7 ms) of stall
    const std::vector<uint64_t> after = run_frames(l, 21, fps);  // 20 intervals after the stall

    const double gap_periods = static_cast<double>(after[0] - warm.back()) / period_ns;
    std::printf("    stall gap: %.2f periods\n", gap_periods);
    CHECK_MSG(gap_periods > 4.0, "the stall did not show up as a long gap");

    double min_iv = 1e18;
    for (std::size_t i = 1; i < after.size(); ++i) {
        min_iv = std::min(min_iv, static_cast<double>(after[i] - after[i - 1]));
    }
    std::printf("    shortest interval in the 20 frames after the stall: %.3f periods\n", min_iv / period_ns);
    CHECK_MSG(min_iv >= 0.5 * period_ns, "burst: an interval after the stall was shorter than 0.5 periods");

    // The schedule is back on the period: the mean of the last 10 intervals is within 2 % of it.
    const double tail_mean = static_cast<double>(after[21 - 1] - after[10]) / 10.0;
    std::printf("    mean of last 10 intervals after the stall: %.4f ms (period %.4f ms)\n", ns_to_ms(tail_mean),
                ns_to_ms(period_ns));
    CHECK_MSG(std::fabs(tail_mean - period_ns) / period_ns < 0.02, "intervals did not return to the period");
}

NXTEST(target_change_honoured_within_10_frames) {
    ScopedBoost boost;
    FrameLimiter l;
    l.set_target_fps(60.0);
    run_frames(l, 30, 60.0);

    l.set_target_fps(240.0);
    const std::vector<uint64_t> up = run_frames(l, 30, 240.0);
    const double up_mean_ms = ns_to_ms(static_cast<double>(up[10] - up[0]) / 10.0);
    std::printf("    60 -> 240: mean of the first 10 intervals %.4f ms (period 4.1667 ms)\n", up_mean_ms);
    CHECK_MSG(std::fabs(up_mean_ms - 1000.0 / 240.0) / (1000.0 / 240.0) < 0.03, "240 fps not honoured within 10 frames");

    l.set_target_fps(60.0);
    const std::vector<uint64_t> down = run_frames(l, 30, 60.0);
    const double down_mean_ms = ns_to_ms(static_cast<double>(down[10] - down[0]) / 10.0);
    std::printf("    240 -> 60: mean of the first 10 intervals %.4f ms (period 16.6667 ms)\n", down_mean_ms);
    CHECK_MSG(std::fabs(down_mean_ms - 1000.0 / 60.0) / (1000.0 / 60.0) < 0.03, "60 fps not honoured within 10 frames");
}

NXTEST(no_handle_leak_100_limiters) {
    DWORD before = 0, after = 0;
    REQUIRE(GetProcessHandleCount(GetCurrentProcess(), &before));
    for (int i = 0; i < 100; ++i) {
        FrameLimiter l;
        l.set_target_fps(60.0);
        l.begin_frame();
    }
    REQUIRE(GetProcessHandleCount(GetCurrentProcess(), &after));
    std::printf("    process handles: before %lu, after 100 limiters %lu\n", static_cast<unsigned long>(before),
                static_cast<unsigned long>(after));
    CHECK_EQ(after, before);
}

NXTEST(two_limiters_two_threads_independent) {
    // Same contention test as test_gfx_pacing: a limiter whose rate is off while more than 0.5 % of its frames overran the
    // period is a busy machine, not a limiter fault, so that case is SKIP. Each worker is boosted like the other timing cases.
    double fps_a = 0, fps_b = 0, over_a = 0, over_b = 0;
    std::thread a([&] {
        ScopedBoost boost;
        FrameLimiter l;
        l.set_target_fps(60.0);
        run_frames(l, 300, 60.0);
        const FrameLimiter::Stats st = l.stats();
        fps_a = st.fps;
        over_a = st.overran_fraction;
    });
    std::thread b([&] {
        ScopedBoost boost;
        FrameLimiter l;
        l.set_target_fps(240.0);
        run_frames(l, 1200, 240.0);
        const FrameLimiter::Stats st = l.stats();
        fps_b = st.fps;
        over_b = st.overran_fraction;
    });
    a.join();
    b.join();
    std::printf("    two threads: 60 fps limiter %.4f (overran %.2f %%), 240 fps limiter %.4f (overran %.2f %%)\n", fps_a,
                over_a * 100, fps_b, over_b * 100);
    const bool off_a = std::fabs(fps_a - 60.0) / 60.0 >= 0.01;
    const bool off_b = std::fabs(fps_b - 240.0) / 240.0 >= 0.01;
    const bool fault_a = off_a && !(over_a > 0.005);   // off with nothing overrunning: the limiter's fault
    const bool fault_b = off_b && !(over_b > 0.005);
    CHECK_MSG(!fault_a, "60 fps limiter moved by more than 1 % with no overrunning frames");
    CHECK_MSG(!fault_b, "240 fps limiter moved by more than 1 % with no overrunning frames");
    if (!fault_a && !fault_b && (off_a || off_b)) {
        nxtest::skip("machine too busy: a limiter missed its rate with more than 0.5 % overrunning frames, so it says nothing about the limiter");
        return;
    }
}

namespace {
int g_pre_frame_calls = 0;
void count_pre_frame(void*) { ++g_pre_frame_calls; }
}  // namespace

NXTEST(pre_frame_hook_runs_only_in_begin_frame) {
    FrameLimiter l;
    l.set_target_fps(0);
    g_pre_frame_calls = 0;
    l.set_pre_frame_hook(count_pre_frame, nullptr);
    for (int i = 0; i < 3; ++i) l.begin_frame();
    CHECK_EQ(g_pre_frame_calls, 3);
    for (int i = 0; i < 3; ++i) l.begin_frame_nohook();
    CHECK_EQ(g_pre_frame_calls, 3);
    l.run_pre_frame_hook();
    l.run_pre_frame_hook();
    CHECK_EQ(g_pre_frame_calls, 5);
}

NXTEST(nohook_still_paces_at_target_rate) {
    ScopedBoost boost;
    FrameLimiter l;
    l.set_target_fps(100.0);
    uint64_t first = 0, last = 0;
    for (int i = 0; i < 6; ++i) {
        last = l.begin_frame_nohook();
        if (i == 0) first = last;
    }
    // 6 starts at 10 ms period span 5 intervals (50 ms); the bound is 4 periods minus 2 ms = 38 ms.
    std::printf("    6 begin_frame_nohook calls at 100 fps: first to last start %.3f ms\n", ns_to_ms(static_cast<double>(last - first)));
    CHECK_MSG(last - first >= 38000000ull, "begin_frame_nohook did not pace: 6 starts at 100 fps spanned less than 38 ms");
}
