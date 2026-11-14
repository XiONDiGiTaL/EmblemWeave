// SPDX-License-Identifier: GPL-3.0-or-later
// The paced sink: a simulated output device. It keeps the frames the audio renderer writes in a queue that drains at the sample rate on
// the host clock (optionally a little fast or slow, [audio] paced_ppm), and counts how often the queue ran dry, how much was dropped
// when it was full, and the lowest and highest queue level. It plays nothing. It exists to measure the renderer's timing without a
// sound card: a real device behaves the same way (XAudio2 consumes the queue at its own clock and starves when it is empty).
#include <algorithm>
#include <chrono>
#include <mutex>

#include "nxrt/audio_sink.h"
#include "nxrt/log.h"

namespace nxrt::audio {
namespace {

using Clock = std::chrono::steady_clock;

class PacedSink final : public AudioSink {
public:
    explicit PacedSink(double skew_ppm) : skew_(1.0 + skew_ppm / 1e6) {}
    ~PacedSink() override { close(); }

    bool open(uint32_t sample_rate, uint32_t channels) override {
        std::lock_guard<std::mutex> lk(mu_);
        rate_ = sample_rate;
        channels_ = channels;
        written_ = played_ = 0.0;
        running_ = starved_ = false;
        opened_ = true;
        return sample_rate > 0 && (channels == 2 || channels == 6);
    }

    void close() override {
        bool was_open;
        {
            std::lock_guard<std::mutex> lk(mu_);
            was_open = opened_;
            opened_ = false;
        }
        if (was_open) NXLOG_I("audio", "%s", format_stats(name(), stats()).c_str());
    }

    uint32_t write(const int16_t*, uint32_t frames) override {
        std::lock_guard<std::mutex> lk(mu_);
        frames_written_ += frames;
        if (!opened_ || rate_ == 0) return 0;
        advance();
        const double cap = rate_ * kCapacitySeconds;
        const double room = std::max(0.0, cap - (written_ - played_));
        const uint32_t take = static_cast<uint32_t>(std::min<double>(frames, room));
        if (take < frames) frames_dropped_ += frames - take;
        frames_accepted_ += take;
        if (!running_) {
            running_ = true;           // playback starts with the first frames, as a voice does
            last_ = Clock::now();
            if (t_first_ == Clock::time_point{}) t_first_ = last_;
        }
        starved_ = false;
        written_ += take;
        peak_ = std::max(peak_, written_ - played_);
        return take;
    }

    uint32_t queued_frames() override {
        std::lock_guard<std::mutex> lk(mu_);
        advance();
        return static_cast<uint32_t>(std::max(0.0, written_ - played_));
    }

    bool has_device_clock() const override { return true; }
    const char* name() const override { return "paced"; }

    SinkStats stats() override {
        std::lock_guard<std::mutex> lk(mu_);
        advance();
        SinkStats s;
        s.frames_written = frames_written_;
        s.frames_accepted = frames_accepted_;
        s.frames_dropped = frames_dropped_;
        s.underruns = underruns_;
        s.queued_peak_frames = static_cast<uint32_t>(peak_);
        s.queued_low_frames = low_ < 0 ? 0 : static_cast<uint32_t>(low_);
        s.queued_peak = s.queued_peak_frames;
        s.sample_rate = rate_;
        return s;
    }

private:
    static constexpr double kWarmupSeconds = 0.5;
    static constexpr double kCapacitySeconds = 0.120;   // the device queue: what XAudio2 would hold before write() drops

    // Moves the play position to now (mu_ held). A queue that empties is an underrun until the next write. The lowest level is sampled
    // by every call, so it is the level just before a refill when the renderer polls the queue each frame.
    void advance() {
        if (!running_) return;
        const auto now = Clock::now();
        const double dt = std::chrono::duration<double>(now - last_).count();
        last_ = now;
        played_ += dt * rate_ * skew_;
        const double q = written_ - played_;
        if (q <= 0.0) {
            played_ = written_;
            if (!starved_) {
                starved_ = true;
                underruns_++;
                NXLOG_W("audio", "paced sink: underrun #%llu, %.2f s after the first frame (%.0f frames written)", static_cast<unsigned long long>(underruns_),
                        std::chrono::duration<double>(now - t_first_).count(), written_);
            }
            low_ = 0;
        } else if (!starved_ && played_ > rate_ * kWarmupSeconds && (low_ < 0 || q < low_)) {
            low_ = q;   // the start-up fill is not a margin: the lowest level is taken once half a second has played
        }
    }

    std::mutex mu_;
    double skew_;
    uint32_t rate_ = 0, channels_ = 0;
    bool opened_ = false, running_ = false, starved_ = false;
    double written_ = 0.0, played_ = 0.0, peak_ = 0.0, low_ = -1.0;
    Clock::time_point last_{}, t_first_{};
    uint64_t frames_written_ = 0, frames_accepted_ = 0, frames_dropped_ = 0, underruns_ = 0;
};

}  // namespace

std::shared_ptr<AudioSink> make_paced_sink(double skew_ppm) { return std::make_shared<PacedSink>(skew_ppm); }

}  // namespace nxrt::audio
