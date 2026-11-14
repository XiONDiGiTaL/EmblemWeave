// SPDX-License-Identifier: GPL-3.0-or-later
#include "test_framework.h"

#include <cmath>
#include <cstdio>

#include "fps_stats.h"

NXTEST_GROUP(gfx_fps_stats)

namespace {

using nxrt::gfx::FpsStats;

// Pushes frames of dt_ms after t, one every dt_ms, up to and including t_end. Returns the time of the last frame.
double run_steady(FpsStats& s, double t, double t_end, double dt_ms) {
    const double dt = dt_ms / 1000.0;
    const long n = static_cast<long>(std::floor((t_end - t) / dt + 1e-6));
    for (long i = 1; i <= n; ++i) s.push(t + static_cast<double>(i) * dt, dt_ms);
    return t + static_cast<double>(n) * dt;
}

void print_snapshot(const char* what, const FpsStats::Snapshot& a) {
    std::printf("    %s: valid=%d fps=%.3f avg=%.4f ms max=%.3f ms low1=%.3f fps\n", what, a.valid ? 1 : 0, a.fps, a.avg_ms, a.max_ms,
                a.low1_fps);
}

}  // namespace

NXTEST(steady_60fps) {
    FpsStats s;
    const double dt = 1000.0 / 60.0;
    const double t = run_steady(s, 0.0, 2.0, dt);
    const FpsStats::Snapshot a = s.snapshot(t);
    print_snapshot("60 fps", a);
    CHECK(a.valid);
    CHECK_MSG(std::fabs(a.fps - 60.0) <= 0.5, "fps is not 60 +- 0.5");
    CHECK_MSG(std::fabs(a.avg_ms - 1000.0 / 60.0) < 0.01, "avg_ms is not 16.67 ms");
    CHECK_MSG(std::fabs(a.max_ms - dt) < 1e-9, "max_ms of a steady series must be the frame time");
    CHECK_MSG(std::fabs(a.low1_fps - 60.0) <= 0.5, "low1 of a steady series must be 60");
}

NXTEST(steady_144fps) {
    FpsStats s;
    const double t = run_steady(s, 0.0, 2.0, 1000.0 / 144.0);
    const FpsStats::Snapshot a = s.snapshot(t);
    print_snapshot("144 fps", a);
    CHECK(a.valid);
    CHECK_MSG(std::fabs(a.fps - 144.0) <= 0.5, "fps is not 144 +- 0.5");
    CHECK_MSG(std::fabs(a.avg_ms - 1000.0 / 144.0) < 0.01, "avg_ms is not 6.94 ms");
}

NXTEST(not_valid_with_one_frame) {
    FpsStats s;
    CHECK_MSG(!s.snapshot(0.0).valid, "no frames must not be valid");
    s.push(1.0, 16.7);
    CHECK_MSG(!s.snapshot(1.0).valid, "one frame must not be valid");
    s.push(1.0 + 16.7 / 1000.0, 16.7);
    CHECK_MSG(s.snapshot(1.0 + 16.7 / 1000.0).valid, "two frames in the 0.5 s window must be valid");
}

NXTEST(hitch_shows_then_leaves_short_window_but_not_low1) {
    FpsStats s;
    const double dt = 1000.0 / 60.0;
    const double th = run_steady(s, 0.0, 1.0, dt);   // steady to about 1.0 s
    s.push(th, 50.0);                                  // one frame that took 50 ms
    const double t = run_steady(s, th, th + 0.05, dt);

    const FpsStats::Snapshot a = s.snapshot(t);
    print_snapshot("hitch +0.05 s", a);
    CHECK(a.valid);
    CHECK_MSG(std::fabs(a.max_ms - 50.0) < 1e-9, "the hitch must be the worst frame of the last 0.5 s");

    const double t2 = run_steady(s, t, th + 0.6, dt);
    const FpsStats::Snapshot b = s.snapshot(t2);
    print_snapshot("hitch +0.6 s", b);
    CHECK(b.valid);
    CHECK_MSG(std::fabs(b.max_ms - dt) < 1e-9, "the hitch must have left the 0.5 s numbers 0.6 s later");
    CHECK_MSG(std::fabs(b.avg_ms - dt) < 0.01, "the 0.5 s average must be back to the frame time");
    CHECK_MSG(std::fabs(b.low1_fps - 20.0) < 0.5, "the hitch must still lower low1 (about 1000 / 50 = 20 fps) within 5 s");
}

NXTEST(low1_is_the_slowest_frame_of_100) {
    FpsStats s;
    double t = 0;
    for (int i = 0; i < 100; ++i) {
        const double dt_ms = i == 50 ? 100.0 : 1000.0 / 60.0;
        t += dt_ms / 1000.0;
        s.push(t, dt_ms);
    }
    const FpsStats::Snapshot a = s.snapshot(t);
    print_snapshot("100 frames, one 100 ms", a);
    CHECK(a.valid);
    CHECK_MSG(std::fabs(a.low1_fps - 10.0) < 1e-9, "low1 of 100 frames with one 100 ms frame must be 10 fps");
}

NXTEST(frames_older_than_5s_are_dropped) {
    FpsStats s;
    const double dt = 1000.0 / 60.0;
    s.push(0.2, 100.0);                              // a slow frame that must be gone by t = 10 s
    const double t = run_steady(s, 0.2, 10.0, dt);
    const FpsStats::Snapshot a = s.snapshot(t);
    print_snapshot("10 s, slow frame at 0.2 s", a);
    CHECK(a.valid);
    // with the slow frame kept, the slowest 3 of about 300 frames would give about 22 fps
    CHECK_MSG(std::fabs(a.low1_fps - 60.0) <= 0.5, "a frame older than 5 s still lowers low1");
}

NXTEST(ring_keeps_only_the_newest_2048_frames) {
    FpsStats s;
    double t = 0;
    for (int i = 0; i < 2952; ++i) {       // 2952 slow frames (100 ms each) will be dropped by the ring
        t += 0.001;
        s.push(t, 100.0);
    }
    for (int i = 0; i < 2048; ++i) {       // then 2048 frames of 1 ms
        t += 0.001;
        s.push(t, 1.0);
    }
    const FpsStats::Snapshot a = s.snapshot(t);
    print_snapshot("5000 frames, newest 2048 at 1 ms", a);
    CHECK(a.valid);
    CHECK_MSG(std::fabs(a.fps - 1000.0) < 0.5, "fps must come from the newest frames");
    CHECK_MSG(std::fabs(a.low1_fps - 1000.0) < 0.5, "low1 must not see the dropped slow frames");
}

NXTEST(reset_clears_everything) {
    FpsStats s;
    const double t = run_steady(s, 0.0, 1.0, 1000.0 / 60.0);
    CHECK(s.snapshot(t).valid);
    s.reset();
    CHECK_MSG(!s.snapshot(t).valid, "reset must clear the frames");
    s.push(t + 0.1, 16.7);
    CHECK_MSG(!s.snapshot(t + 0.1).valid, "one frame after reset must not be valid");
}
