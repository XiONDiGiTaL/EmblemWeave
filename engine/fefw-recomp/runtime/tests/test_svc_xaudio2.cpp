// SPDX-License-Identifier: GPL-3.0-or-later
// The XAudio2 host sink (src/services/audio_xaudio2.cpp). Device-dependent cases skip when open() fails (no audio device).
#include <cmath>
#include <cstdint>
#include <memory>
#include <vector>

#include "nxrt/audio_sink.h"
#include "test_framework.h"

NXTEST_GROUP(svc_xaudio2)

using namespace nxrt;

namespace {

constexpr double kPi = 3.14159265358979323846;
constexpr uint32_t kRate = 48000;
constexpr uint32_t kPieceFrames = kRate / 100;   // 10 ms
constexpr double kQuiet = 0.05;                  // volume: the speakers may be on

// Fills `frames` frames of a 440 Hz sine at kQuiet, `channels` interleaved int16 values per frame. `phase` carries across pieces.
void fill_sine(std::vector<int16_t>& out, uint32_t frames, uint32_t channels, double& phase) {
    out.resize(static_cast<size_t>(frames) * channels);
    const double step = 2.0 * kPi * 440.0 / kRate;
    for (uint32_t i = 0; i < frames; i++) {
        const int16_t v = static_cast<int16_t>(std::sin(phase) * kQuiet * 32767.0);
        for (uint32_t c = 0; c < channels; c++) out[static_cast<size_t>(i) * channels + c] = v;
        phase += step;
    }
}

}  // namespace

NXTEST(xaudio2_factory_returns_sink) {
    std::shared_ptr<audio::AudioSink> s = audio::make_xaudio2_sink();
    CHECK(s != nullptr);
    if (s) CHECK_EQ(std::string(s->name()), std::string("xaudio2"));
}

NXTEST(xaudio2_stereo_100ms_queues_and_accepts_all) {
    auto s = audio::make_xaudio2_sink();
    REQUIRE(s != nullptr);
    if (!s->open(kRate, 2)) {
        nxtest::skip("no audio device");
        return;
    }
    double phase = 0.0;
    std::vector<int16_t> piece;
    uint32_t offered = 0, accepted = 0;
    for (int i = 0; i < 10; i++) {   // 10 x 10 ms = 100 ms
        fill_sine(piece, kPieceFrames, 2, phase);
        accepted += s->write(piece.data(), kPieceFrames);
        offered += kPieceFrames;
    }
    CHECK_EQ(offered, 4800u);
    CHECK_EQ(accepted, offered);
    CHECK(s->queued_frames() > 0);
    s->close();
    CHECK_EQ(s->queued_frames(), 0u);
}

NXTEST(xaudio2_overfill_drops_and_does_not_crash) {
    auto s = audio::make_xaudio2_sink();
    REQUIRE(s != nullptr);
    if (!s->open(kRate, 2)) {
        nxtest::skip("no audio device");
        return;
    }
    double phase = 0.0;
    std::vector<int16_t> piece;
    fill_sine(piece, kPieceFrames, 2, phase);
    uint64_t offered = 0, accepted = 0;
    // 200 x 10 ms = 2 s, written with no waiting: the queue limit (about 12 buffers) must drop the rest.
    for (int i = 0; i < 200; i++) {
        accepted += s->write(piece.data(), kPieceFrames);
        offered += kPieceFrames;
    }
    CHECK(accepted > 0);
    CHECK(accepted < offered);
    s->close();
}

NXTEST(xaudio2_reopen_six_channels) {
    auto s = audio::make_xaudio2_sink();
    REQUIRE(s != nullptr);
    if (!s->open(kRate, 2)) {
        nxtest::skip("no audio device");
        return;
    }
    s->close();
    if (!s->open(kRate, 6)) {
        nxtest::skip("no 5.1 output (device refused 6 channels)");
        return;
    }
    double phase = 0.0;
    std::vector<int16_t> piece;
    fill_sine(piece, kPieceFrames, 6, phase);
    CHECK_EQ(s->write(piece.data(), kPieceFrames), kPieceFrames);
    CHECK(s->queued_frames() > 0);
    s->close();
}

NXTEST(xaudio2_double_close_is_safe) {
    auto s = audio::make_xaudio2_sink();
    REQUIRE(s != nullptr);
    s->close();   // never opened
    s->close();
    CHECK_EQ(s->queued_frames(), 0u);
    if (s->open(kRate, 2)) {
        s->close();
        s->close();
        CHECK_EQ(s->queued_frames(), 0u);
    }
}

NXTEST(xaudio2_rejects_bad_channel_count) {
    auto s = audio::make_xaudio2_sink();
    REQUIRE(s != nullptr);
    CHECK(!s->open(kRate, 4));
    CHECK_EQ(s->write(nullptr, 0), 0u);
}
