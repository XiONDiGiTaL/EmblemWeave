// SPDX-License-Identifier: GPL-3.0-or-later
// The software equalizer (include/nxrt/audio_eq.h) and the sink wrapper that puts it on the final mix (make_eq_sink). The DSP is a
// cascade of ten peaking biquads per channel, followed by automatic headroom and a peak limiter. Nothing here opens a device: the
// wrapper forwards to the sink that was chosen for this machine, so XAudio2, the paced test sink and the WAV tee all get the same EQ.
#include "nxrt/audio_eq.h"

#include <algorithm>
#include <cctype>
#include <cmath>
#include <cstring>
#include <memory>
#include <mutex>
#include <string>
#include <vector>

#include "nxrt/audio_sink.h"

namespace nxrt::audio {
namespace {

constexpr float kPi = 3.14159265358979323846f;
constexpr float kBandHz[kEqBandCount] = {31.0f, 62.0f, 125.0f, 250.0f, 500.0f, 1000.0f, 2000.0f, 4000.0f, 8000.0f, 16000.0f};
constexpr const char* kBandLabels[kEqBandCount] = {"31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"};
constexpr float kQ = 1.41421356f;   // about one octave between the -3 dB points

// The presets are kept in one place so the F1 menu and the file writer never diverge. Gains are dB; the launcher carries the same table.
struct Preset {
    const char* name;
    float db[kEqBandCount];
    float preamp_db;
};
constexpr Preset kPresets[kEqPresetCount] = {
    {"flat", {0, 0, 0, 0, 0, 0, 0, 0, 0, 0}, 0.0f},
    {"bass", {7, 6, 5, 3, 1, 0, -1, -2, -3, -4}, 0.0f},
    {"treble", {-3, -2, -1, 0, 0, 1, 3, 5, 6, 7}, 0.0f},
    {"vocal", {-4, -2, 0, 2, 4, 4, 3, 1, 0, -1}, 0.0f},
    {"warm", {3, 2, 1, 1, 0, 0, -1, -1, -2, -3}, 0.0f},
    {"v-shape", {6, 5, 3, 0, -2, -2, 0, 3, 5, 6}, 0.0f},
    {"night", {-6, -4, -2, 0, 0, 0, 1, 2, 2, 1}, 0.0f},
    // The user's ten-band headphone curve, with the preamp at the bottom of
    // the F1 range so the boosts keep headroom before the limiter.
    {"headphones-boost", {9.5f, 6.0f, 5.0f, -2.5f, 1.0f, 0.0f, -1.0f, 7.0f, 5.0f, 9.5f}, -12.0f},
};

std::mutex g_eq_mu;
EqSettings g_eq;
uint32_t g_eq_revision = 0;

float sanitize_db(float v) {
    if (!std::isfinite(v)) return 0.0f;
    return std::clamp(v, -kEqMaxGainDb, kEqMaxGainDb);
}

std::string normalized_name(const char* name) {
    std::string out;
    if (!name) return out;
    for (const char* p = name; *p; ++p) {
        const char c = static_cast<char>(std::tolower(static_cast<unsigned char>(*p)));
        out.push_back(c == '_' || c == ' ' ? '-' : c);
    }
    return out;
}

float db_to_linear(float db) { return std::pow(10.0f, db / 20.0f); }

}  // namespace

float eq_band_hz(int band) {
    if (band < 0 || band >= kEqBandCount) return 0.0f;
    return kBandHz[band];
}

const char* eq_band_label(int band) {
    if (band < 0 || band >= kEqBandCount) return "";
    return kBandLabels[band];
}

const char* eq_preset_name(int preset) {
    if (preset < 0 || preset >= kEqPresetCount) return "custom";
    return kPresets[preset].name;
}

int eq_preset_from_name(const char* name) {
    const std::string want = normalized_name(name);
    for (int i = 0; i < kEqPresetCount; i++)
        if (want == kPresets[i].name) return i;
    return kEqPresetCustom;
}

const float* eq_preset_db(int preset) {
    if (preset < 0 || preset >= kEqPresetCount) return nullptr;
    return kPresets[preset].db;
}

float eq_preset_preamp_db(int preset) {
    if (preset < 0 || preset >= kEqPresetCount) return 0.0f;
    return kPresets[preset].preamp_db;
}

EqSettings eq_settings() {
    std::lock_guard<std::mutex> lk(g_eq_mu);
    return g_eq;
}

void set_eq_settings(const EqSettings& settings) {
    EqSettings s = settings;
    s.preamp_db = sanitize_db(s.preamp_db);
    for (float& db : s.bands_db) db = sanitize_db(db);
    std::lock_guard<std::mutex> lk(g_eq_mu);
    s.revision = ++g_eq_revision;
    g_eq = s;
}

void EqProcessor::reset(uint32_t sample_rate, uint32_t channels) {
    sample_rate_ = sample_rate;
    channel_count_ = channels;
    settings_revision_ = 0;
    configured_ = false;
    preamp_gain_ = 1.0f;
    limiter_gain_ = 1.0f;
    for (auto& band : state_)
        for (auto& st : band) st = {};
}

void EqProcessor::configure(const EqSettings& settings) {
    // The peak limiter must have the final word, so the whole EQ gets a negative preamp equal to its largest boost. A bass preset
    // therefore makes the bass loud and the rest quieter instead of pushing the bass into the limiter.
    float max_boost = 0.0f;
    for (float db : settings.bands_db) max_boost = std::max(max_boost, db);
    preamp_gain_ = db_to_linear(settings.preamp_db - max_boost);

    const double fs = sample_rate_ ? static_cast<double>(sample_rate_) : 48000.0;
    const double nyquist_limit = fs * 0.45;
    for (int b = 0; b < kEqBandCount; b++) {
        const double f = std::min(static_cast<double>(kBandHz[b]), nyquist_limit);
        const double A = std::pow(10.0, static_cast<double>(settings.bands_db[b]) / 40.0);
        const double w0 = 2.0 * static_cast<double>(kPi) * f / fs;
        const double alpha = std::sin(w0) / (2.0 * static_cast<double>(kQ));
        const double a0 = 1.0 + alpha / A;
        Coeff& c = coeffs_[b];
        c.b0 = static_cast<float>((1.0 + alpha * A) / a0);
        c.b1 = static_cast<float>(-2.0 * std::cos(w0) / a0);
        c.b2 = static_cast<float>((1.0 - alpha * A) / a0);
        c.a1 = static_cast<float>(-2.0 * std::cos(w0) / a0);
        c.a2 = static_cast<float>((1.0 - alpha / A) / a0);
    }
    configured_ = true;
    settings_revision_ = settings.revision;
}

void EqProcessor::process(int16_t* pcm, uint32_t frames, uint32_t channels, const EqSettings& settings) {
    if (!pcm || frames == 0 || channels == 0) return;
    if (channels > kMaxChannels) return;   // the engine only carries stereo or 5.1; leave an unknown layout untouched
    if (sample_rate_ == 0 || channel_count_ != channels) {
        // The caller normally opens first. A stream that did not still gets a correct processor.
        reset(sample_rate_ ? sample_rate_ : 48000, channels);
    }
    if (!settings.active()) {
        limiter_gain_ = 1.0f;
        return;
    }
    if (!configured_ || settings_revision_ != settings.revision) configure(settings);

    // One frame at a time, peak across channels, so the limiter holds every speaker under the ceiling together.
    const float ceiling = 0.99f;   // about -0.09 dBFS, with room for rounding
    const float rate = sample_rate_ ? static_cast<float>(sample_rate_) : 48000.0f;
    const float release = 1.0f - std::exp(-1.0f / (0.1f * rate));
    float gain = limiter_gain_;
    for (uint32_t f = 0; f < frames; f++) {
        int16_t* frame = pcm + static_cast<size_t>(f) * channels;
        float out[kMaxChannels] = {};
        float peak = 0.0f;
        for (uint32_t c = 0; c < channels; c++) {
            float x = static_cast<float>(frame[c]) * (1.0f / 32768.0f);
            for (int b = 0; b < kEqBandCount; b++) {
                const Coeff& co = coeffs_[b];
                State& st = state_[b][c];
                const float y = co.b0 * x + st.z1;
                st.z1 = co.b1 * x - co.a1 * y + st.z2;
                st.z2 = co.b2 * x - co.a2 * y;
                x = y;
            }
            x *= preamp_gain_;
            out[c] = x;
            peak = std::max(peak, std::fabs(x));
        }
        const float want = peak > ceiling ? ceiling / peak : 1.0f;
        if (want < gain) gain = want;                     // attack at once
        else gain += (want - gain) * release;             // release over about 100 ms
        for (uint32_t c = 0; c < channels; c++) {
            const float v = std::clamp(out[c] * gain, -1.0f, 1.0f);
            frame[c] = static_cast<int16_t>(std::lrint(v * 32767.0f));
        }
    }
    limiter_gain_ = gain;
}

namespace {

// Puts the EQ on top of the chosen sink. The wrapper is always installed, so turning the EQ on in the menu does not need a new device;
// when it is off (or flat) write() forwards the original pointer with no copy.
class EqSink final : public AudioSink {
public:
    explicit EqSink(std::shared_ptr<AudioSink> inner) : inner_(std::move(inner)), name_(inner_ ? inner_->name() : "null") {}

    ~EqSink() override {
        if (open_) close();
    }

    bool open(uint32_t sample_rate, uint32_t channels) override {
        rate_ = sample_rate;
        channels_ = channels;
        open_ = true;
        bypassed_ = true;
        processor_.reset(sample_rate, channels);
        return inner_ && inner_->open(sample_rate, channels);
    }

    void close() override {
        if (inner_) inner_->close();
        open_ = false;
        processor_.reset(0, 0);
    }

    uint32_t write(const int16_t* pcm, uint32_t frames) override {
        if (!inner_) return 0;
        const EqSettings s = eq_settings();
        if (!open_ || channels_ == 0 || !s.active()) {
            if (!bypassed_) {
                processor_.reset(rate_, channels_);
                bypassed_ = true;
            }
            return inner_->write(pcm, frames);
        }
        bypassed_ = false;
        const size_t samples = static_cast<size_t>(frames) * channels_;
        scratch_.resize(samples);
        if (samples) std::memcpy(scratch_.data(), pcm, samples * sizeof(int16_t));
        processor_.process(scratch_.data(), frames, channels_, s);
        return inner_->write(scratch_.data(), frames);
    }

    uint32_t queued_frames() override { return inner_ ? inner_->queued_frames() : 0; }
    bool has_device_clock() const override { return inner_ && inner_->has_device_clock(); }
    const char* name() const override { return name_.c_str(); }
    SinkStats stats() override { return inner_ ? inner_->stats() : SinkStats{}; }

private:
    std::shared_ptr<AudioSink> inner_;
    std::string name_;
    EqProcessor processor_;
    std::vector<int16_t> scratch_;
    uint32_t rate_ = 0;
    uint32_t channels_ = 0;
    bool open_ = false;
    bool bypassed_ = true;
};

}  // namespace

std::shared_ptr<AudioSink> make_eq_sink(std::shared_ptr<AudioSink> inner) {
    if (!inner) return inner;
    return std::make_shared<EqSink>(std::move(inner));
}

}  // namespace nxrt::audio
