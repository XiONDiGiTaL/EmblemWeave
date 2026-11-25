// SPDX-License-Identifier: GPL-3.0-or-later
// Issue #37 Phase 1: frame-time percentiles must use the samples actually in the rolling window, including before it fills.
#include "test_framework.h"

#include "nxrt/renderer_profiling.h"

#include <algorithm>
#include <string>

NXTEST_GROUP(gfx_renderer_profiling)

NXTEST(empty_frame_time_window) {
    nxrt::rp::FrameTimeWindow w;
    const auto p = w.snapshot();
    CHECK_EQ(p.frames, uint64_t{0});
    CHECK_EQ(p.p50, 0.0f);
    CHECK_EQ(p.p95, 0.0f);
    CHECK_EQ(p.p99, 0.0f);
    CHECK_EQ(p.max, 0.0f);
}

NXTEST(partial_window_uses_the_samples_it_has) {
    nxrt::rp::FrameTimeWindow w;
    w.add(10.0f);
    w.add(20.0f);
    w.add(30.0f);
    const auto p = w.snapshot();
    CHECK_EQ(p.frames, uint64_t{3});
    CHECK_EQ(p.p50, 20.0f);
    CHECK_EQ(p.p95, 30.0f);
    CHECK_EQ(p.p99, 30.0f);
    CHECK_EQ(p.max, 30.0f);
}

NXTEST(wrapped_window_keeps_the_latest_capacity_samples) {
    nxrt::rp::FrameTimeWindow w;
    const uint64_t total = nxrt::rp::FrameTimeWindow::kCapacity + 3;
    for (uint64_t i = 1; i <= total; ++i) w.add(static_cast<float>(i));
    const auto p = w.snapshot();
    CHECK_EQ(p.frames, uint64_t{nxrt::rp::FrameTimeWindow::kCapacity});
    CHECK_EQ(p.p50, 4100.0f);   // samples 4..8195, index 4096
    CHECK_EQ(p.p95, 7786.0f);   // index 7782
    CHECK_EQ(p.p99, 8114.0f);   // index 8110
    CHECK_EQ(p.max, 8195.0f);
}

NXTEST(csv_row_matches_header_shape) {
    const std::string header = nxrt::rp::renderer_profiling_csv_header();
    const std::string row = nxrt::rp::renderer_profiling_csv_row();
    CHECK(!header.empty());
    CHECK(!row.empty());
    CHECK_EQ(std::count(header.begin(), header.end(), ','), std::count(row.begin(), row.end(), ','));
}
