// SPDX-License-Identifier: GPL-3.0-or-later
// The software equalizer that runs on the final 16-bit mix (include/nxrt/audio_sink.h, make_eq_sink). Ten peaking bands from 31 Hz to
// 16 kHz, a preamp, automatic headroom and a peak limiter, so a boosted band cannot clip the output. Presets are the same ones the F1
// menu and the launcher offer; the settings are process wide and can be changed while audio is playing.
#pragma once
#include <cmath>
#include <cstdint>

namespace nxrt::audio {

constexpr int kEqBandCount = 10;
constexpr float kEqMaxGainDb = 12.0f;
constexpr float kEqMaxPreampDb = 12.0f;

// 31, 62, 125, 250, 500, 1k, 2k, 4k, 8k, 16k.
float eq_band_hz(int band);
const char* eq_band_label(int band);   // "31", "62", "125", "250", "500", "1k", "2k", "4k", "8k", "16k"

// Preset indices 0..kEqPresetCount-1; kEqPresetCustom means the bands were changed by hand.
constexpr int kEqPresetCount = 8;
constexpr int kEqPresetCustom = -1;
const char* eq_preset_name(int preset);          // "flat", "bass", ... or "custom"
int eq_preset_from_name(const char* name);       // case-insensitive; -1 when the name is not a preset
const float* eq_preset_db(int preset);            // kEqBandCount gains, or nullptr for custom/unknown
float eq_preset_preamp_db(int preset);            // the preset's extra preamp, 0 when it has none

struct EqSettings {
    bool enabled = false;
    float preamp_db = 0.0f;
    float bands_db[kEqBandCount] = {};
    // Set by set_eq_settings; a processor uses it to rebuild coefficients only when the settings changed.
    uint32_t revision = 0;

    // True when the EQ would change the signal. A flat, enabled EQ is a pass-through.
    bool active() const {
        if (!enabled) return false;
        if (std::fabs(preamp_db) > 0.001f) return true;
        for (float db : bands_db)
            if (std::fabs(db) > 0.001f) return true;
        return false;
    }
};

// A copy of the live settings. Safe to call from the render thread.
EqSettings eq_settings();
// Replaces the live settings (clamped to the documented range) and bumps the revision. Safe from the UI thread.
void set_eq_settings(const EqSettings& settings);

// One processor per stream. Biquad state is per channel; coefficients are shared. process() is allocation free.
class EqProcessor {
public:
    void reset(uint32_t sample_rate, uint32_t channels);
    // Applies the preamp, the ten bands and the limiter in place. `pcm` is interleaved int16.
    void process(int16_t* pcm, uint32_t frames, uint32_t channels, const EqSettings& settings);

private:
    static constexpr int kMaxChannels = 8;
    struct Coeff {
        float b0 = 1.0f, b1 = 0.0f, b2 = 0.0f, a1 = 0.0f, a2 = 0.0f;
    };
    struct State {
        float z1 = 0.0f, z2 = 0.0f;
    };
    void configure(const EqSettings& settings);

    Coeff coeffs_[kEqBandCount];
    State state_[kEqBandCount][kMaxChannels];
    uint32_t sample_rate_ = 0;
    uint32_t channel_count_ = 0;
    uint32_t settings_revision_ = 0;
    bool configured_ = false;
    float preamp_gain_ = 1.0f;
    float limiter_gain_ = 1.0f;
};

}  // namespace nxrt::audio
