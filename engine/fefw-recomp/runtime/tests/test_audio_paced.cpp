// SPDX-License-Identifier: GPL-3.0-or-later
// The paced sink (src/services/audio_paced.cpp): a simulated output device that drains its queue at the sample rate on the host clock.
// The audio renderer paces itself by the queue level of such a device (AudioProcessor::Wait), so these cases pin down what the sink reports.
#include <chrono>
#include <cstdint>
#include <memory>
#include <thread>
#include <vector>

#include "nxrt/audio_sink.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audio_paced)

using namespace nxrt;

namespace {
constexpr uint32_t kRate = 48000;
void sleep_ms(int ms) { std::this_thread::sleep_for(std::chrono::milliseconds(ms)); }
}  // namespace

NXTEST(paced_has_a_device_clock_and_the_null_sink_does_not) {
    auto p = audio::make_paced_sink();
    CHECK(p->has_device_clock());
    CHECK_EQ(std::string(p->name()), std::string("paced"));
    audio::NullSink n;
    CHECK(!n.has_device_clock());
}

NXTEST(paced_queue_drains_at_the_sample_rate) {
    auto p = audio::make_paced_sink();
    REQUIRE(p->open(kRate, 2));
    std::vector<int16_t> pcm(static_cast<size_t>(kRate / 10) * 2, 0);   // 100 ms
    CHECK_EQ(p->write(pcm.data(), kRate / 10), kRate / 10);
    CHECK(p->queued_frames() > kRate / 10 - 480);   // just written: still about 100 ms
    sleep_ms(50);
    const uint32_t q = p->queued_frames();
    CHECK(q > 1500 && q < 3300);                     // about 50 ms left: 2400 frames, generous bounds for a loaded machine
    audio::SinkStats s = p->stats();
    CHECK_EQ(s.underruns, 0ull);
    CHECK_EQ(s.frames_accepted, static_cast<uint64_t>(kRate / 10));
    p->close();
}

NXTEST(paced_counts_one_underrun_per_dry_stretch) {
    auto p = audio::make_paced_sink();
    REQUIRE(p->open(kRate, 2));
    std::vector<int16_t> pcm(static_cast<size_t>(480) * 2, 0);          // 10 ms
    p->write(pcm.data(), 480);
    sleep_ms(60);                                                       // the queue is empty for 50 ms: one underrun
    CHECK_EQ(p->queued_frames(), 0u);
    CHECK_EQ(p->queued_frames(), 0u);                                   // asking again does not count again
    CHECK_EQ(p->stats().underruns, 1ull);
    p->write(pcm.data(), 480);                                          // playback resumes
    sleep_ms(60);
    CHECK_EQ(p->stats().underruns, 2ull);
    p->close();
}

NXTEST(paced_drops_what_does_not_fit) {
    auto p = audio::make_paced_sink();
    REQUIRE(p->open(kRate, 2));
    std::vector<int16_t> pcm(static_cast<size_t>(kRate) * 2, 0);        // one second at once: the queue holds 120 ms
    const uint32_t took = p->write(pcm.data(), kRate);
    CHECK(took > kRate / 10 && took < kRate / 5);
    const audio::SinkStats s = p->stats();
    CHECK_EQ(s.frames_written, static_cast<uint64_t>(kRate));
    CHECK_EQ(s.frames_accepted + s.frames_dropped, static_cast<uint64_t>(kRate));
    p->close();
}

NXTEST(paced_skew_changes_the_drain_rate) {
    auto fast = audio::make_paced_sink(100000.0);   // the device runs 10 % fast
    auto slow = audio::make_paced_sink(-100000.0);  // and 10 % slow
    REQUIRE(fast->open(kRate, 2) && slow->open(kRate, 2));
    std::vector<int16_t> pcm(static_cast<size_t>(kRate / 10) * 2, 0);
    fast->write(pcm.data(), kRate / 10);
    slow->write(pcm.data(), kRate / 10);
    sleep_ms(80);
    // 80 ms of the host clock: the fast device consumed about 88 ms, the slow one about 72 ms
    CHECK(slow->queued_frames() > fast->queued_frames());
    fast->close();
    slow->close();
}
