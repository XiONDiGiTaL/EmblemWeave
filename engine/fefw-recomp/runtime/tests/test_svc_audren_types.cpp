// SPDX-License-Identifier: GPL-3.0-or-later
// Layout checks for the audio renderer parameter structs (src/services/audren/audren_types.h). The expected sizes and offsets
// are written from the Ryujinx C# sources (Ryujinx.Audio Renderer/Parameter, Renderer/Common, Renderer/Server/Types), not copied
// from the header. Sizes marked "(computed)" in the header have no C# size assertion; they follow from the C# field layout.
#include <cstddef>

#include "../src/services/audren/audren_types.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_types)

namespace au = nxrt::audren;

#define CHECK_SIZE(T, want) CHECK_EQ(sizeof(T), static_cast<size_t>(want))
#define CHECK_OFF(T, f, want) CHECK_EQ(offsetof(T, f), static_cast<size_t>(want))

NXTEST(audren_types_common) {
    CHECK_SIZE(au::AuxiliaryBufferAddresses, 0x20);
    CHECK_OFF(au::AuxiliaryBufferAddresses, send_buffer_info, 0x00);
    CHECK_OFF(au::AuxiliaryBufferAddresses, return_buffer_info_base, 0x18);

    CHECK_SIZE(au::BehaviourParameter, 0x10);
    CHECK_OFF(au::BehaviourParameter, user_revision, 0x00);
    CHECK_OFF(au::BehaviourParameter, flags, 0x08);
    CHECK_SIZE(au::BehaviourParameter::ErrorInfo, 0x10);
    CHECK_OFF(au::BehaviourParameter::ErrorInfo, error_code, 0x00);
    CHECK_OFF(au::BehaviourParameter::ErrorInfo, extra_error_info, 0x08);

    CHECK_SIZE(au::UpdateDataHeader, 0x40);
    CHECK_OFF(au::UpdateDataHeader, revision, 0x00);
    CHECK_OFF(au::UpdateDataHeader, render_info_size, 0x28);
    CHECK_OFF(au::UpdateDataHeader, reserved, 0x2C);
    CHECK_OFF(au::UpdateDataHeader, total_size, 0x3C);

    CHECK_SIZE(au::WaveBuffer, 0x30);
    CHECK_OFF(au::WaveBuffer, buffer, 0x00);
    CHECK_OFF(au::WaveBuffer, loop_count, 0x28);
    CHECK_OFF(au::WaveBuffer, looping, 0x2C);
    CHECK_OFF(au::WaveBuffer, padding, 0x2E);

    CHECK_SIZE(au::BiquadFilterState, 0x20);
    CHECK_OFF(au::BiquadFilterState, state0, 0x00);
    CHECK_OFF(au::BiquadFilterState, state7, 0x1C);

    CHECK_SIZE(au::AdpcmLoopContext, 6);
    CHECK_OFF(au::AdpcmLoopContext, pred_scale, 0x00);
    CHECK_OFF(au::AdpcmLoopContext, history1, 0x04);

    // VoiceState: C# Pack = 0x10, natural alignment gives 2 bytes of padding before last_samples and no tail padding.
    CHECK_SIZE(au::VoiceState, 0xD8);
    CHECK_OFF(au::VoiceState, biquad_filter_state, 0x00);
    CHECK_OFF(au::VoiceState, played_sample_count, 0x40);
    CHECK_OFF(au::VoiceState, wave_buffer_consumed, 0x54);
    CHECK_OFF(au::VoiceState, pitch, 0x58);
    CHECK_OFF(au::VoiceState, loop_context, 0x6C);
    CHECK_OFF(au::VoiceState, last_samples, 0x74);
    CHECK_OFF(au::VoiceState, loop_count, 0xD4);
}

NXTEST(audren_types_parameter_voice) {
    CHECK_SIZE(au::BiquadFilterParameter1, 0xC);
    CHECK_OFF(au::BiquadFilterParameter1, enable, 0x00);
    CHECK_OFF(au::BiquadFilterParameter1, denominator, 0x08);

    CHECK_SIZE(au::BiquadFilterParameter2, 0x18);
    CHECK_OFF(au::BiquadFilterParameter2, enable, 0x00);
    CHECK_OFF(au::BiquadFilterParameter2, denominator, 0x10);

    CHECK_SIZE(au::WaveBufferInternal, 0x38);
    CHECK_OFF(au::WaveBufferInternal, address, 0x00);
    CHECK_OFF(au::WaveBufferInternal, sent_to_server, 0x1A);
    CHECK_OFF(au::WaveBufferInternal, loop_last_sample_offset, 0x34);

    CHECK_SIZE(au::VoiceInParameter1, 0x170);
    CHECK_OFF(au::VoiceInParameter1, id, 0x00);
    CHECK_OFF(au::VoiceInParameter1, volume, 0x20);
    CHECK_OFF(au::VoiceInParameter1, biquad_filters, 0x24);
    CHECK_OFF(au::VoiceInParameter1, wave_buffers, 0x60);
    CHECK_OFF(au::VoiceInParameter1, channel_resource_ids, 0x140);
    CHECK_OFF(au::VoiceInParameter1, reset_voice_drop_flag, 0x158);
    CHECK_OFF(au::VoiceInParameter1, decoding_behaviour_flags, 0x15C);
    CHECK_OFF(au::VoiceInParameter1, src_quality, 0x15E);
    CHECK_OFF(au::VoiceInParameter1, external_context, 0x15F);
    CHECK_OFF(au::VoiceInParameter1, reserved3, 0x167);
    CHECK_OFF(au::VoiceInParameter1, padding, 0x16F);

    CHECK_SIZE(au::VoiceInParameter2, 0x188);
    CHECK_OFF(au::VoiceInParameter2, biquad_filters, 0x24);
    CHECK_OFF(au::VoiceInParameter2, wave_buffers, 0x78);
    CHECK_OFF(au::VoiceInParameter2, channel_resource_ids, 0x158);
    CHECK_OFF(au::VoiceInParameter2, decoding_behaviour_flags, 0x174);
    CHECK_OFF(au::VoiceInParameter2, src_quality, 0x176);
    CHECK_OFF(au::VoiceInParameter2, external_context, 0x177);
    CHECK_OFF(au::VoiceInParameter2, reserved3, 0x17F);
    CHECK_OFF(au::VoiceInParameter2, padding, 0x187);

    CHECK_SIZE(au::VoiceChannelResourceInParameter, 0x70);
    CHECK_OFF(au::VoiceChannelResourceInParameter, id, 0x00);
    CHECK_OFF(au::VoiceChannelResourceInParameter, mix, 0x04);
    CHECK_OFF(au::VoiceChannelResourceInParameter, is_used, 0x64);
    CHECK_OFF(au::VoiceChannelResourceInParameter, padding, 0x65);

    CHECK_SIZE(au::VoiceOutStatus, 0x10);
    CHECK_OFF(au::VoiceOutStatus, played_sample_count, 0x00);
    CHECK_OFF(au::VoiceOutStatus, voice_drop_flag, 0x0C);
    CHECK_OFF(au::VoiceOutStatus, reserved, 0x0D);
}

NXTEST(audren_types_parameter_renderer) {
    CHECK_SIZE(au::AudioRendererConfiguration, 0x34);
    CHECK_OFF(au::AudioRendererConfiguration, sample_rate, 0x00);
    CHECK_OFF(au::AudioRendererConfiguration, voice_drop_enabled, 0x20);
    CHECK_OFF(au::AudioRendererConfiguration, rendering_device, 0x22);
    CHECK_OFF(au::AudioRendererConfiguration, execution_mode, 0x23);
    CHECK_OFF(au::AudioRendererConfiguration, revision, 0x30);

    CHECK_SIZE(au::BehaviourErrorInfoOutStatus, 0xB0);
    CHECK_OFF(au::BehaviourErrorInfoOutStatus, error_infos, 0x00);
    CHECK_OFF(au::BehaviourErrorInfoOutStatus, error_infos_count, 0xA0);
    CHECK_OFF(au::BehaviourErrorInfoOutStatus, reserved, 0xA4);

    CHECK_SIZE(au::EffectResultState, 0x80);
    CHECK_SIZE(au::EffectInParameterVersion1, 0xC0);
    CHECK_OFF(au::EffectInParameterVersion1, type, 0x00);
    CHECK_OFF(au::EffectInParameterVersion1, processing_order, 0x18);
    CHECK_OFF(au::EffectInParameterVersion1, reserved2, 0x1C);
    CHECK_OFF(au::EffectInParameterVersion1, specific_data, 0x20);
    CHECK_SIZE(au::EffectInParameterVersion2, 0xC0);
    CHECK_OFF(au::EffectInParameterVersion2, type, 0x00);
    CHECK_OFF(au::EffectInParameterVersion2, processing_order, 0x18);
    CHECK_OFF(au::EffectInParameterVersion2, specific_data, 0x20);

    CHECK_SIZE(au::EffectOutStatusVersion1, 0x10);
    CHECK_OFF(au::EffectOutStatusVersion1, state, 0x00);
    CHECK_OFF(au::EffectOutStatusVersion1, reserved, 0x01);
    CHECK_SIZE(au::EffectOutStatusVersion2, 0x90);
    CHECK_OFF(au::EffectOutStatusVersion2, state, 0x00);
    CHECK_OFF(au::EffectOutStatusVersion2, result_state, 0x10);

    CHECK_SIZE(au::MemoryPoolInParameter, 0x20);
    CHECK_OFF(au::MemoryPoolInParameter, cpu_address, 0x00);
    CHECK_OFF(au::MemoryPoolInParameter, state, 0x10);
    CHECK_OFF(au::MemoryPoolInParameter, reserved, 0x14);
    CHECK_SIZE(au::MemoryPoolOutStatus, 0x10);
    CHECK_OFF(au::MemoryPoolOutStatus, state, 0x00);
    CHECK_OFF(au::MemoryPoolOutStatus, reserved, 0x04);

    CHECK_SIZE(au::MixInParameterDirtyOnlyUpdate, 0x20);
    CHECK_OFF(au::MixInParameterDirtyOnlyUpdate, magic, 0x00);
    CHECK_OFF(au::MixInParameterDirtyOnlyUpdate, reserved, 0x08);

    CHECK_SIZE(au::MixParameter, 0x930);
    CHECK_OFF(au::MixParameter, volume, 0x00);
    CHECK_OFF(au::MixParameter, mix_buffer_volume, 0x24);
    CHECK_OFF(au::MixParameter, destination_mix_id, 0x924);
    CHECK_OFF(au::MixParameter, reserved3, 0x92C);

    CHECK_SIZE(au::RendererInfoOutStatus, 0x10);
    CHECK_OFF(au::RendererInfoOutStatus, elapsed_frame_count, 0x00);
    CHECK_OFF(au::RendererInfoOutStatus, reserved, 0x08);

    CHECK_SIZE(au::SinkInParameter, 0x140);
    CHECK_OFF(au::SinkInParameter, type, 0x00);
    CHECK_OFF(au::SinkInParameter, magic, 0x02);
    CHECK_OFF(au::SinkInParameter, node_id, 0x04);
    CHECK_OFF(au::SinkInParameter, reserved2, 0x08);
    CHECK_OFF(au::SinkInParameter, specific_data, 0x20);
    CHECK_SIZE(au::SinkOutStatus, 0x20);
    CHECK_OFF(au::SinkOutStatus, last_written_offset, 0x00);
    CHECK_OFF(au::SinkOutStatus, reserved, 0x08);

    CHECK_SIZE(au::SplitterDestinationInParameterVersion1, 0x70);
    CHECK_OFF(au::SplitterDestinationInParameterVersion1, magic, 0x00);
    CHECK_OFF(au::SplitterDestinationInParameterVersion1, mix_buffer_volume, 0x08);
    CHECK_OFF(au::SplitterDestinationInParameterVersion1, destination_id, 0x68);
    CHECK_OFF(au::SplitterDestinationInParameterVersion1, reset_prev_volume, 0x6D);
    CHECK_OFF(au::SplitterDestinationInParameterVersion1, reserved, 0x6E);
    CHECK_SIZE(au::SplitterDestinationInParameterVersion2a, 0x90);
    CHECK_OFF(au::SplitterDestinationInParameterVersion2a, biquad_filters, 0x6C);
    CHECK_OFF(au::SplitterDestinationInParameterVersion2a, is_used, 0x84);
    CHECK_OFF(au::SplitterDestinationInParameterVersion2a, reserved, 0x86);
    CHECK_SIZE(au::SplitterDestinationInParameterVersion2b, 0xA8);
    CHECK_OFF(au::SplitterDestinationInParameterVersion2b, biquad_filters, 0x6C);
    CHECK_OFF(au::SplitterDestinationInParameterVersion2b, is_used, 0x9C);
    CHECK_OFF(au::SplitterDestinationInParameterVersion2b, reserved, 0x9E);

    CHECK_SIZE(au::SplitterInParameter, 0x10);
    CHECK_OFF(au::SplitterInParameter, magic, 0x00);
    CHECK_OFF(au::SplitterInParameter, destination_count, 0x0C);
    CHECK_SIZE(au::SplitterInParameterHeader, 0x20);
    CHECK_OFF(au::SplitterInParameterHeader, magic, 0x00);
    CHECK_OFF(au::SplitterInParameterHeader, reserved, 0x0C);

    CHECK_SIZE(au::PerformanceInParameter, 0x10);
    CHECK_OFF(au::PerformanceInParameter, target_node_id, 0x00);
    CHECK_OFF(au::PerformanceInParameter, reserved, 0x04);
    CHECK_SIZE(au::PerformanceOutStatus, 0x10);
    CHECK_OFF(au::PerformanceOutStatus, history_size, 0x00);
    CHECK_OFF(au::PerformanceOutStatus, reserved, 0x04);
}

NXTEST(audren_types_effect_and_sink) {
    CHECK_SIZE(au::AuxiliaryBufferParameter, 0x6C);
    CHECK_OFF(au::AuxiliaryBufferParameter, input, 0x00);
    CHECK_OFF(au::AuxiliaryBufferParameter, output, 0x18);
    CHECK_OFF(au::AuxiliaryBufferParameter, mix_buffer_sample_count, 0x68);

    CHECK_SIZE(au::BiquadFilterEffectParameter1, 0x18);
    CHECK_OFF(au::BiquadFilterEffectParameter1, input, 0x00);
    CHECK_OFF(au::BiquadFilterEffectParameter1, status, 0x17);
    CHECK_SIZE(au::BiquadFilterEffectParameter2, 0x28);
    CHECK_OFF(au::BiquadFilterEffectParameter2, padding, 0x0C);
    CHECK_OFF(au::BiquadFilterEffectParameter2, numerator, 0x10);
    CHECK_OFF(au::BiquadFilterEffectParameter2, status, 0x25);
    CHECK_OFF(au::BiquadFilterEffectParameter2, reserved, 0x26);

    CHECK_SIZE(au::BufferMixParameter, 0x94);
    CHECK_OFF(au::BufferMixParameter, volumes, 0x30);
    CHECK_OFF(au::BufferMixParameter, mixes_count, 0x90);

    CHECK_SIZE(au::CompressorParameter, 0x38);
    CHECK_OFF(au::CompressorParameter, channel_count_max, 0x0C);
    CHECK_OFF(au::CompressorParameter, status, 0x34);
    CHECK_OFF(au::CompressorParameter, statistics_reset, 0x37);
    CHECK_SIZE(au::CompressorStatistics, 0x20);
    CHECK_OFF(au::CompressorStatistics, last_samples, 0x08);

    CHECK_SIZE(au::DelayParameter, 0x35);
    CHECK_OFF(au::DelayParameter, channel_count_max, 0x0C);
    CHECK_OFF(au::DelayParameter, low_pass_amount, 0x30);
    CHECK_OFF(au::DelayParameter, status, 0x34);

    CHECK_SIZE(au::LimiterParameter, 0x44);
    CHECK_OFF(au::LimiterParameter, delay_buffer_sample_count_min, 0x38);
    CHECK_OFF(au::LimiterParameter, status, 0x40);
    CHECK_OFF(au::LimiterParameter, reserved, 0x43);
    CHECK_SIZE(au::LimiterStatistics, 0x30);
    CHECK_OFF(au::LimiterStatistics, input_max, 0x00);
    CHECK_OFF(au::LimiterStatistics, compression_gain_min, 0x18);

    CHECK_SIZE(au::Reverb3dParameter, 0x49);
    CHECK_OFF(au::Reverb3dParameter, reserved, 0x10);
    CHECK_OFF(au::Reverb3dParameter, sample_rate, 0x14);
    CHECK_OFF(au::Reverb3dParameter, dry_gain, 0x44);
    CHECK_OFF(au::Reverb3dParameter, parameter_status, 0x48);

    CHECK_SIZE(au::ReverbParameter, 0x41);
    CHECK_OFF(au::ReverbParameter, early_mode, 0x14);
    CHECK_OFF(au::ReverbParameter, late_mode, 0x20);
    CHECK_OFF(au::ReverbParameter, dry_gain, 0x3C);
    CHECK_OFF(au::ReverbParameter, status, 0x40);

    CHECK_SIZE(au::CircularBufferParameter, 0x24);
    CHECK_OFF(au::CircularBufferParameter, buffer_address, 0x00);
    CHECK_OFF(au::CircularBufferParameter, sample_format, 0x18);
    CHECK_OFF(au::CircularBufferParameter, input, 0x1C);
    CHECK_OFF(au::CircularBufferParameter, reserved2, 0x22);

    CHECK_SIZE(au::DeviceParameter, 0x11C);
    CHECK_OFF(au::DeviceParameter, device_name, 0x00);
    CHECK_OFF(au::DeviceParameter, padding, 0xFF);
    CHECK_OFF(au::DeviceParameter, input_count, 0x100);
    CHECK_OFF(au::DeviceParameter, input, 0x104);
    CHECK_OFF(au::DeviceParameter, down_mix_parameter_enabled, 0x10B);
    CHECK_OFF(au::DeviceParameter, down_mix_parameter, 0x10C);
}

NXTEST(audren_types_enums_and_magic) {
    // Underlying types as declared in C#.
    CHECK_SIZE(au::SampleFormat, 1);
    CHECK_SIZE(au::PlayState, 1);
    CHECK_SIZE(au::EffectType, 1);
    CHECK_SIZE(au::EffectState, 1);
    CHECK_SIZE(au::UsageState, 1);
    CHECK_SIZE(au::SinkType, 1);
    CHECK_SIZE(au::SampleRateConversionQuality, 1);
    CHECK_SIZE(au::PerformanceEntryType, 1);
    CHECK_SIZE(au::PerformanceDetailType, 1);
    CHECK_SIZE(au::NodeIdType, 1);
    CHECK_SIZE(au::DecodingBehaviour, 2);
    CHECK_SIZE(au::MemoryPoolUserState, 4);
    CHECK_SIZE(au::ReverbEarlyMode, 4);
    CHECK_SIZE(au::ReverbLateMode, 4);
    CHECK_SIZE(au::ResultCode, 4);
    CHECK_SIZE(au::server::PlayState, 4);
    CHECK_SIZE(au::server::AudioRendererExecutionMode, 1);
    CHECK_SIZE(au::server::AudioRendererRenderingDevice, 1);

    // Spot-check enumerator values against the C# source.
    CHECK_EQ(static_cast<uint8_t>(au::EffectState::Enabled), 3u);
    CHECK_EQ(static_cast<uint8_t>(au::EffectState::Disabled), 4u);
    CHECK_EQ(static_cast<uint32_t>(au::ReverbLateMode::Limit), 4u);
    CHECK_EQ(static_cast<uint8_t>(au::NodeIdType::Performance), 15u);
    CHECK_EQ(static_cast<int32_t>(au::ResultCode::DeviceNotFound), 0x299);  // (1 << 9) | 153

    // Magic values from the C# IsMagicValid() helpers.
    CHECK_EQ(au::SplitterInParameter::ValidMagic, 0x49444E53u);
    CHECK_EQ(au::SplitterInParameterHeader::ValidMagic, 0x48444E53u);
    CHECK_EQ(au::SplitterDestinationInParameterVersion1::ValidMagic, 0x44444E53u);
    au::SplitterInParameter sp{};
    sp.magic = au::SplitterInParameter::ValidMagic;
    CHECK(sp.IsMagicValid());
}
