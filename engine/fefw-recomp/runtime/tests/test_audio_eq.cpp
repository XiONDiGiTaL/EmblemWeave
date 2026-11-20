// SPDX-License-Identifier: GPL-3.0-or-later
// The software equalizer (src/services/audio_eq.cpp): preset tables, the biquad response, automatic headroom, the limiter and the sink
// wrapper. No output device is opened.
#include <algorithm>
#include <cmath>
#include <cstdint>
#include <cstring>
#include <memory>
#include <vector>

#include "nxrt/audio_eq.h"
#include "nxrt/audio_sink.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audio_eq)

namespace {

constexpr uint32_t kRate = 48000;
constexpr double kPi = 3.14159265358979323846;

std::vector<int16_t> sine(double hz, uint32_t frames, double amp = 0.35) {
    std::vector<int16_t> out(frames);
    for (uint32_t i = 0; i < frames; i++) {
        out[i] = static_cast<int16_t>(std::lrint(std::sin(2.0 * kPi * hz * i / kRate) * amp * 32767.0));
    }
    return out;
}

double rms(const std::vector<int16_t>& pcm, size_t from) {
    if (from >= pcm.size()) return 0.0;
    double sum = 0.0;
    for (size_t i = from; i < pcm.size(); i++) {
        const double v = static_cast<double>(pcm[i]) / 32768.0;
        sum += v * v;
    }
    return std::sqrt(sum / static_cast<double>(pcm.size() - from));
}

double response(double hz, const nxrt::audio::EqSettings& settings, uint32_t frames = 8192) {
    const std::vector<int16_t> in = sine(hz, frames);
    std::vector<int16_t> out = in;
    nxrt::audio::EqProcessor processor;
    processor.reset(kRate, 1);
    processor.process(out.data(), frames, 1, settings);
    const size_t settle = frames / 2;
    return rms(out, settle) / std::max(1e-12, rms(in, settle));
}

nxrt::audio::EqSettings saved() { return nxrt::audio::eq_settings(); }
void restore(const nxrt::audio::EqSettings& s) { nxrt::audio::set_eq_settings(s); }

// A sink that keeps the last block, so the wrapper's output can be checked.
class CaptureSink final : public nxrt::audio::AudioSink {
public:
    bool open(uint32_t, uint32_t channels) override {
        channels_ = channels;
        return true;
    }
    void close() override {}
    uint32_t write(const int16_t* pcm, uint32_t frames) override {
        last.assign(pcm, pcm + static_cast<size_t>(frames) * std::max(1u, channels_));
        return frames;
    }
    uint32_t queued_frames() override { return 0; }
    const char* name() const override { return "capture"; }
    uint32_t channels_ = 2;
    std::vector<int16_t> last;
};

nxrt::audio::EqSettings target_band(int band, float db) {
    nxrt::audio::EqSettings s;
    s.enabled = true;
    s.bands_db[band] = db;
    s.revision = 1;
    return s;
}

}  // namespace

NXTEST(preset_names_and_curves_round_trip) {
    CHECK_EQ(nxrt::audio::kEqBandCount, 10);
    CHECK_EQ(nxrt::audio::kEqPresetCount, 8);
    for (int i = 0; i < nxrt::audio::kEqPresetCount; i++) {
        const char* name = nxrt::audio::eq_preset_name(i);
        CHECK(name && *name);
        CHECK_EQ(nxrt::audio::eq_preset_from_name(name), i);
        const float* db = nxrt::audio::eq_preset_db(i);
        REQUIRE(db != nullptr);
        for (int b = 0; b < nxrt::audio::kEqBandCount; b++) CHECK(std::fabs(db[b]) <= 12.0f);
    }
    CHECK_EQ(nxrt::audio::eq_preset_from_name("V_SHAPE"), 5);
    CHECK_EQ(nxrt::audio::eq_preset_from_name("headphones-boost"), 7);
    CHECK_EQ(nxrt::audio::eq_preset_preamp_db(7), -12.0f);
    CHECK_EQ(nxrt::audio::eq_preset_from_name("custom"), nxrt::audio::kEqPresetCustom);
    CHECK_EQ(nxrt::audio::eq_preset_from_name("not-a-preset"), nxrt::audio::kEqPresetCustom);
    CHECK_EQ(std::string(nxrt::audio::eq_band_label(0)), std::string("31"));
    CHECK_EQ(std::string(nxrt::audio::eq_band_label(9)), std::string("16k"));
    CHECK(nxrt::audio::eq_band_hz(5) == 1000.0f);
}

NXTEST(flat_settings_are_a_pass_through) {
    nxrt::audio::EqProcessor processor;
    processor.reset(kRate, 1);
    std::vector<int16_t> pcm = sine(1000.0, 2048);
    const std::vector<int16_t> in = pcm;
    nxrt::audio::EqSettings s;
    s.enabled = true;
    s.revision = 1;
    processor.process(pcm.data(), static_cast<uint32_t>(pcm.size()), 1, s);
    CHECK(pcm == in);
}

NXTEST(a_boosted_band_is_heard_and_automatic_headroom_keeps_the_rest_down) {
    const nxrt::audio::EqSettings saved_settings = saved();
    nxrt::audio::EqSettings s = target_band(5, 12.0f);   // 1 kHz
    nxrt::audio::set_eq_settings(s);
    s = nxrt::audio::eq_settings();

    const double at_1k = response(1000.0, s);
    const double at_100 = response(100.0, s);
    CHECK_MSG(at_1k > 0.80 && at_1k < 1.20, "1 kHz is at unity after the -12 dB headroom");
    CHECK_MSG(at_100 < 0.45, "100 Hz is pulled down by the headroom");
    CHECK(at_1k > at_100 * 2.0);
    restore(saved_settings);
}

NXTEST(bass_preset_lifts_lows_more_than_highs) {
    const nxrt::audio::EqSettings saved_settings = saved();
    nxrt::audio::EqSettings s;
    s.enabled = true;
    const float* bass = nxrt::audio::eq_preset_db(1);
    REQUIRE(bass != nullptr);
    for (int b = 0; b < nxrt::audio::kEqBandCount; b++) s.bands_db[b] = bass[b];
    s.revision = 1;
    nxrt::audio::set_eq_settings(s);
    s = nxrt::audio::eq_settings();

    const double low = response(60.0, s);
    const double high = response(8000.0, s);
    CHECK_MSG(low > high * 2.0, "the bass preset is tilted toward the low end");
    CHECK(low > 0.5);
    restore(saved_settings);
}

NXTEST(the_limiter_keeps_a_hot_boost_under_the_int16_ceiling) {
    const nxrt::audio::EqSettings saved_settings = saved();
    nxrt::audio::EqSettings s;
    s.enabled = true;
    for (float& db : s.bands_db) db = 12.0f;
    s.preamp_db = 12.0f;
    s.revision = 1;
    nxrt::audio::set_eq_settings(s);
    s = nxrt::audio::eq_settings();

    std::vector<int16_t> pcm = sine(1000.0, 8192, 0.95);
    nxrt::audio::EqProcessor processor;
    processor.reset(kRate, 2);
    std::vector<int16_t> stereo(pcm.size() * 2);
    for (size_t i = 0; i < pcm.size(); i++) stereo[2 * i] = stereo[2 * i + 1] = pcm[i];
    processor.process(stereo.data(), static_cast<uint32_t>(pcm.size()), 2, s);
    int peak = 0;
    for (int16_t v : stereo) peak = std::max(peak, std::abs(static_cast<int>(v)));
    CHECK_MSG(peak <= 32700, "the limiter holds the output below full scale");
    CHECK_MSG(peak > 1000, "the test signal reached the limiter instead of being silenced");
    restore(saved_settings);
}

NXTEST(the_sink_wrapper_forwards_when_the_eq_is_off) {
    const nxrt::audio::EqSettings saved_settings = saved();
    nxrt::audio::EqSettings s;
    s.enabled = false;
    nxrt::audio::set_eq_settings(s);
    auto inner = std::make_shared<CaptureSink>();
    auto sink = nxrt::audio::make_eq_sink(inner);
    REQUIRE(sink != nullptr);
    REQUIRE(sink->open(kRate, 1));
    const std::vector<int16_t> in = sine(440.0, 256);
    CHECK_EQ(sink->write(in.data(), 256), 256u);
    CHECK_EQ(inner->last.size(), in.size());
    CHECK(inner->last == in);
    sink->close();
    restore(saved_settings);
}

NXTEST(the_sink_wrapper_applies_the_live_curve) {
    const nxrt::audio::EqSettings saved_settings = saved();
    nxrt::audio::EqSettings s = target_band(1, 12.0f);   // 62 Hz
    nxrt::audio::set_eq_settings(s);
    auto inner = std::make_shared<CaptureSink>();
    auto sink = nxrt::audio::make_eq_sink(inner);
    REQUIRE(sink->open(kRate, 1));
    const std::vector<int16_t> in = sine(62.0, 4096);
    CHECK_EQ(sink->write(in.data(), static_cast<uint32_t>(in.size())), static_cast<uint32_t>(in.size()));
    CHECK_EQ(inner->last.size(), in.size());
    CHECK(inner->last != in);
    sink->close();
    restore(saved_settings);
}
