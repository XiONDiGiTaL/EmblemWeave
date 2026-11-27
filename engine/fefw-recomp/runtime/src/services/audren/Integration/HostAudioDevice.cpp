// SPDX-License-Identifier: GPL-3.0-or-later
// Implementation of HostAudioDevice / HostAudioDeviceDriver (see HostAudioDevice.h).
#include "HostAudioDevice.h"

#include <chrono>
#include <mutex>
#include <string>
#include <vector>

#include "../Dsp/PcmHelper.h"
#include "nxrt/audio_sink.h"
#include "nxrt/config.h"
#include "nxrt/log.h"

namespace nxrt::audren {

namespace {

// The sink this device last opened, with the parameters it was opened with. All HostAudioDevice instances share the host sink, so the
// open state is process wide. Guarded by gSinkMutex.
std::mutex gSinkMutex;
std::shared_ptr<::nxrt::audio::AudioSink> gOpenSink;
uint32_t gOpenRate = 0;
uint32_t gOpenChannels = 0;
bool gOpenOk = false;
std::wstring gFailedDefaultId;
std::chrono::steady_clock::time_point gNextEndpointCheck{};

}  // namespace

HostAudioDevice::HostAudioDevice(uint32_t sampleRate, uint32_t channelCount) : _sampleRate(sampleRate), _channelCount(channelCount) {
    NXAUDREN_ASSERT(channelCount == 2 || channelCount == 6);
}

void HostAudioDevice::SetVolume(float volume) { _volume = volume; }

float HostAudioDevice::GetVolume() { return _volume; }

uint32_t HostAudioDevice::GetSampleRate() { return _sampleRate; }

uint32_t HostAudioDevice::GetChannelCount() { return _channelCount; }

void HostAudioDevice::AppendBuffer(Span<const int16_t> data, uint32_t channelCount) {
    NXAUDREN_ASSERT(channelCount > 0);

    const size_t frames = data.Length / channelCount;

    if (frames == 0) {
        return;
    }

    // Interleaved, scaled, at this device's channel count.
    std::vector<int16_t> out(frames * _channelCount, 0);

    const uint32_t copyChannels = channelCount < _channelCount ? channelCount : _channelCount;

    for (size_t frame = 0; frame < frames; frame++) {
        for (uint32_t c = 0; c < copyChannels; c++) {
            const int16_t sample = data[frame * channelCount + c];

            // C# PcmHelper.Saturate(sample * volume) for the scaled case; the unit volume is a copy.
            out[frame * _channelCount + c] = _volume == 1.0f ? sample : PcmHelper::Saturate(static_cast<float>(sample) * _volume);
        }
    }

    std::lock_guard<std::mutex> guard(gSinkMutex);

    std::shared_ptr<::nxrt::audio::AudioSink> sink = ::nxrt::audio::sink();

    const bool changed = sink != gOpenSink || _sampleRate != gOpenRate || _channelCount != gOpenChannels;
    if (changed || !gOpenOk) {
        if (!changed) {
            const auto now = std::chrono::steady_clock::now();
            if (now < gNextEndpointCheck) {
                return;
            }
            gNextEndpointCheck = now + std::chrono::seconds(1);
            // Only retry when the default output endpoint actually changes to a different non-empty device, so a broken or dummy
            // endpoint never churns XAudio2 while a newly connected (or reconnected) device recovers automatically.
            const std::wstring def = ::nxrt::audio::default_output_id();
            if (def.empty()) {
                gFailedDefaultId.clear();
                return;
            }
            if (def == gFailedDefaultId) {
                return;
            }
        }
        // A new sink object was installed with set_sink (it closes the previous one), or the stream format changed: (re)open.
        if (gOpenOk && gOpenSink != nullptr && sink == gOpenSink) {
            gOpenSink->close();
        }

        gOpenSink = sink;
        gOpenRate = _sampleRate;
        gOpenChannels = _channelCount;
        gOpenOk = sink->open(_sampleRate, _channelCount);
        if (!gOpenOk) {
            gFailedDefaultId = ::nxrt::audio::default_output_id();
            gNextEndpointCheck = std::chrono::steady_clock::now() + std::chrono::seconds(1);
            NXLOG_W("AudioRenderer", "host audio sink %s failed to open (%u Hz, %u channels): playing without sound", sink->name(),
                    _sampleRate, _channelCount);
            return;
        }
        gFailedDefaultId.clear();
    }

    sink->write(out.data(), static_cast<uint32_t>(frames));
}

int64_t HostAudioDevice::GetQueuedNanoseconds() {
    // Under the mutex AppendBuffer holds while it opens, closes and writes the sink: the sink cannot be reopened under this call.
    std::lock_guard<std::mutex> guard(gSinkMutex);

    if (!gOpenOk || gOpenSink == nullptr || !gOpenSink->has_device_clock() || _sampleRate == 0) {
        return -1;   // nothing written yet, or a sink without a clock (the renderer paces itself by the host clock)
    }

    return static_cast<int64_t>(gOpenSink->queued_frames()) * 1000000000ll / static_cast<int64_t>(_sampleRate);
}

HostAudioDeviceDriver::HostAudioDeviceDriver()
    : _configuredChannels(::nxrt::config_get_int("audio", "channels", 2) == 6 ? 6u : 2u) {}

std::unique_ptr<IHardwareDevice> HostAudioDeviceDriver::OpenHardwareDevice(uint32_t sampleRate, uint32_t channelCount) {
    return std::make_unique<HostAudioDevice>(sampleRate, channelCount);
}

bool HostAudioDeviceDriver::SupportsChannelCount(uint32_t channelCount) const { return channelCount == _configuredChannels; }

}  // namespace nxrt::audren
