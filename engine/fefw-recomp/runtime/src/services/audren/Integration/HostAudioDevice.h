// SPDX-License-Identifier: GPL-3.0-or-later
// Host output for the audio renderer: an IHardwareDevice that plays the renderer's PCM16 on the host audio sink (nxrt::audio::sink()).
// The C# counterpart is the HardwareDeviceImpl + IHardwareDeviceSession pair (Ryujinx.Audio/Integration); here the session is the
// nxrt AudioSink, which the host pulls at the device clock.
//
// Channel count: 2 (stereo, default) or 6 (5.1). The config key [audio] channels selects which one the processor opens (see
// HostAudioDeviceDriver). The rate is always 48000 (Constants::TargetSampleRate).
#pragma once
#include <cstdint>
#include <memory>

#include "../Constants.h"
#include "../audren_common.h"
#include "IHardwareDevice.h"
#include "IHardwareDeviceDriver.h"

namespace nxrt::audren {

class HostAudioDevice : public IHardwareDevice {
  public:
    // `channelCount` is 2 or 6. The sink is opened lazily, on the first AppendBuffer.
    HostAudioDevice(uint32_t sampleRate, uint32_t channelCount);

    void SetVolume(float volume) override;
    float GetVolume() override;
    uint32_t GetSampleRate() override;
    uint32_t GetChannelCount() override;

    // Scales the PCM16 frames by the device volume and hands them to the host sink. `channelCount` is the channel count of `data`.
    // When it differs from this device's channel count, the channels present are copied and the rest are silence.
    void AppendBuffer(Span<const int16_t> data, uint32_t channelCount) override;

    // The host sink's queue level when the sink has a device clock (XAudio2, paced), else -1.
    int64_t GetQueuedNanoseconds() override;

  private:
    uint32_t _sampleRate;
    uint32_t _channelCount;
    float _volume = 1.0f;
};

class HostAudioDeviceDriver : public IHardwareDeviceDriver {
  public:
    // Reads [audio] channels (2 or 6, default 2) with nxrt::config_get_int.
    HostAudioDeviceDriver();

    std::unique_ptr<IHardwareDevice> OpenHardwareDevice(uint32_t sampleRate, uint32_t channelCount) override;
    bool SupportsChannelCount(uint32_t channelCount) const override;

    // The channel count from the config (2 or 6).
    uint32_t ConfiguredChannelCount() const { return _configuredChannels; }

  private:
    uint32_t _configuredChannels;
};

}  // namespace nxrt::audren
