// SPDX-License-Identifier: GPL-3.0-or-later
// Server/Effect ports of the audio renderer (S3c): the effect state machine, the parameter update of each effect type,
// the output status, the effect-type change and the EffectContext.
//
// Every expected value below is derived by hand from the C# sources under refs/ryubing/src/Ryujinx.Audio/Renderer/Server/Effect
// (BaseEffect.cs, ReverbEffect.cs, ..., EffectContext.cs), Server/MemoryPool/PoolMapper.cs and Server/StateUpdater.cs, not from
// running the port. The derivation is noted next to each group.
#include <cstdint>
#include <cstring>
#include <memory>
#include <vector>

#include "../src/services/audren/Server/Effect/AuxiliaryBufferEffect.h"
#include "../src/services/audren/Server/Effect/BaseEffect.h"
#include "../src/services/audren/Server/Effect/BiquadFilterEffect.h"
#include "../src/services/audren/Server/Effect/BufferMixEffect.h"
#include "../src/services/audren/Server/Effect/CaptureBufferEffect.h"
#include "../src/services/audren/Server/Effect/CompressorEffect.h"
#include "../src/services/audren/Server/Effect/DelayEffect.h"
#include "../src/services/audren/Server/Effect/EffectContext.h"
#include "../src/services/audren/Server/Effect/LimiterEffect.h"
#include "../src/services/audren/Server/Effect/Reverb3dEffect.h"
#include "../src/services/audren/Server/Effect/ReverbEffect.h"
#include "../src/services/audren/Server/MemoryPool/PoolMapper.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_effect)

using namespace nxrt::audren;

namespace {

// ---- helpers ----

// A user input of either version: the two layouts are identical (audren_types.h).
template <class TIn>
TIn MakeInput(EffectType type, bool isNew, bool isEnabled, int32_t mixId, uint64_t bufferBase, uint64_t bufferSize, uint32_t order) {
    TIn in{};
    in.type = type;
    in.is_new = isNew;
    in.is_enabled = isEnabled;
    in.mix_id = mixId;
    in.buffer_base = bufferBase;
    in.buffer_size = bufferSize;
    in.processing_order = order;
    return in;
}

// Copy an effect-specific parameter into the specific data area of an input (C# SpecificData).
template <class TIn, class TSpec>
void SetSpecific(TIn& in, const TSpec& spec) {
    std::memcpy(in.specific_data, &spec, sizeof(TSpec));
}

// Read a T out of raw bytes (C# MemoryMarshal.Cast<byte, T>).
template <class T>
T ReadBytes(const uint8_t* bytes) {
    T value{};
    std::memcpy(&value, bytes, sizeof(T));
    return value;
}

// Pool [0x1000, 0x5000) attached through MemoryPoolInParameter RequestAttach. C# AudioProcessorMemoryManager is the identity
// mapping in this port, so the pool's DspAddress equals its CpuAddress (0x1000).
std::vector<MemoryPoolInfo> MakeAttachedPool() {
    std::vector<MemoryPoolInfo> pools(1, MemoryPoolInfo::Create(MemoryPoolInfo::LocationType::Cpu));
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);
    MemoryPoolInParameter in{};
    in.cpu_address = 0x1000;
    in.size = 0x4000;
    in.state = MemoryPoolUserState::RequestAttach;
    MemoryPoolOutStatus out{};
    CHECK(mapper.Update(pools[0], in, out) == PoolMapper::UpdateResult::Success);
    CHECK(pools[0].IsMapped());
    return pools;
}

// ---- BaseEffect ----

// C# BaseEffect constructor: Type Invalid, UsageState Invalid, IsEnabled false, BufferUnmapped false, MixId UnusedMixId
// (int.MaxValue), ProcessingOrder uint.MaxValue. ShouldSkip returns BufferUnmapped (false).
NXTEST(base_effect_defaults) {
    BaseEffect effect;
    CHECK(effect.Type == EffectType::Invalid);
    CHECK(effect.UsageState == UsageState::Invalid);
    CHECK(!effect.IsEnabled);
    CHECK(!effect.BufferUnmapped);
    CHECK_EQ(effect.MixId, 0x7FFFFFFF);
    CHECK(effect.ProcessingOrder == 0xFFFFFFFFu);
    CHECK(!effect.ShouldSkip());
    CHECK(effect.TargetEffectType() == EffectType::Invalid);
}

// C# BaseEffect.StoreStatus. Active renderer: Disabled gives Disabled, anything else Enabled. Inactive renderer: New gives
// Enabled, anything else Disabled. Checked on both output versions (state is the first field of each).
NXTEST(store_status_matrix) {
    struct Row {
        UsageState state;
        bool active;
        EffectState expected;
    };
    const Row rows[] = {
        {UsageState::Invalid, true, EffectState::Enabled},   {UsageState::New, true, EffectState::Enabled},
        {UsageState::Enabled, true, EffectState::Enabled},   {UsageState::Disabled, true, EffectState::Disabled},
        {UsageState::Invalid, false, EffectState::Disabled}, {UsageState::New, false, EffectState::Enabled},
        {UsageState::Enabled, false, EffectState::Disabled}, {UsageState::Disabled, false, EffectState::Disabled},
    };
    BufferMixEffect effect;
    for (const Row& row : rows) {
        effect.UsageState = row.state;

        EffectOutStatusVersion1 out1{};
        effect.StoreStatus(out1, row.active);
        CHECK(out1.state == row.expected);

        EffectOutStatusVersion2 out2{};
        effect.StoreStatus(out2, row.active);
        CHECK(out2.state == row.expected);
    }
}

// C# BaseEffect.GetPerformanceDetailType maps each effect type to its PerformanceDetailType.
NXTEST(performance_detail_type_per_effect) {
    BiquadFilterEffect biquad(2);
    AuxiliaryBufferEffect aux;
    DelayEffect delay;
    ReverbEffect reverb;
    Reverb3dEffect reverb3d;
    BufferMixEffect mix;
    LimiterEffect limiter;
    CaptureBufferEffect capture;
    CompressorEffect compressor;
    CHECK(biquad.GetPerformanceDetailType() == PerformanceDetailType::BiquadFilter);
    CHECK(aux.GetPerformanceDetailType() == PerformanceDetailType::Aux);
    CHECK(delay.GetPerformanceDetailType() == PerformanceDetailType::Delay);
    CHECK(reverb.GetPerformanceDetailType() == PerformanceDetailType::Reverb);
    CHECK(reverb3d.GetPerformanceDetailType() == PerformanceDetailType::Reverb3d);
    CHECK(mix.GetPerformanceDetailType() == PerformanceDetailType::Mix);
    CHECK(limiter.GetPerformanceDetailType() == PerformanceDetailType::Limiter);
    CHECK(capture.GetPerformanceDetailType() == PerformanceDetailType::CaptureBuffer);
    CHECK(compressor.GetPerformanceDetailType() == PerformanceDetailType::Compressor);
}

// ---- Reverb ----

// C# ReverbEffect.Update, first update: is_new true, is_enabled true, mix 3, processing order 7, buffer [0x1000, 0x5000) in the
// attached pool. channel_count_max = channel_count = 2 are valid (1, 2, 4, 6).
// Expected: MixId 3, ProcessingOrder 7, IsEnabled true, UsageState New, Parameter.status Invalid, BufferUnmapped false
// (TryAttachBuffer returns true: the range is contained and the pool is mapped), error code Success.
// Command generation: UsageState Enabled, Parameter.status Enabled. GetWorkBuffer(0) = 0x1000 (pool DspAddress + offset 0).
// StoreStatus with an active renderer: Enabled (3).
template <class TIn>
void CheckReverbFirstUpdate() {
    std::vector<MemoryPoolInfo> pools = MakeAttachedPool();
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);

    ReverbParameter rp{};
    rp.channel_count_max = 2;
    rp.channel_count = 2;
    rp.sample_rate = 48000;
    TIn in = MakeInput<TIn>(EffectType::Reverb, true, true, 3, 0x1000, 0x4000, 7);
    SetSpecific(in, rp);

    ReverbEffect effect;
    BehaviourParameter::ErrorInfo err{};
    effect.Update(err, in, mapper);

    CHECK_EQ(effect.MixId, 3);
    CHECK(effect.ProcessingOrder == 7u);
    CHECK(effect.IsEnabled);
    CHECK(effect.UsageState == UsageState::New);
    CHECK(effect.Parameter.status == UsageState::Invalid);
    CHECK(!effect.BufferUnmapped);
    CHECK(err.error_code == ResultCode::Success);
    int32_t sampleRate = effect.Parameter.sample_rate;
    CHECK_EQ(sampleRate, 48000);

    effect.UpdateForCommandGeneration();
    CHECK(effect.UsageState == UsageState::Enabled);
    CHECK(effect.Parameter.status == UsageState::Enabled);
    CHECK_EQ(effect.GetWorkBuffer(0), 0x1000u);

    EffectOutStatusVersion1 out{};
    effect.StoreStatus(out, true);
    CHECK(out.state == EffectState::Enabled);
}

NXTEST(reverb_first_update_version1) { CheckReverbFirstUpdate<EffectInParameterVersion1>(); }
NXTEST(reverb_first_update_version2) { CheckReverbFirstUpdate<EffectInParameterVersion2>(); }

// C# ReverbEffect.Update, continuing the enabled effect of CheckReverbFirstUpdate: an update with is_new false and is_enabled false
// keeps the buffer. The usage state stays Enabled until command generation, and the old Parameter.status (Enabled) is not
// restored, so the status becomes the input's (0 = Invalid). Command generation then sets Disabled; GetWorkBuffer returns 0
// (GetSingleBuffer returns 0 when IsEnabled is false). StoreStatus active gives Disabled (4).
// A new update (is_new true) sets UsageState New, and StoreStatus with an inactive renderer gives Enabled.
template <class TIn>
void CheckReverbDisableThenNew() {
    std::vector<MemoryPoolInfo> pools = MakeAttachedPool();
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);

    ReverbParameter rp{};
    rp.channel_count_max = 2;
    rp.channel_count = 2;

    ReverbEffect effect;
    BehaviourParameter::ErrorInfo err{};

    TIn first = MakeInput<TIn>(EffectType::Reverb, true, true, 3, 0x1000, 0x4000, 7);
    SetSpecific(first, rp);
    effect.Update(err, first, mapper);
    effect.UpdateForCommandGeneration();

    TIn second = MakeInput<TIn>(EffectType::Reverb, false, false, 3, 0x1000, 0x4000, 7);
    SetSpecific(second, rp);
    effect.Update(err, second, mapper);
    CHECK(!effect.IsEnabled);
    CHECK(!effect.BufferUnmapped);
    CHECK(effect.UsageState == UsageState::Enabled);
    CHECK(effect.Parameter.status == UsageState::Invalid);

    effect.UpdateForCommandGeneration();
    CHECK(effect.UsageState == UsageState::Disabled);
    CHECK_EQ(effect.GetWorkBuffer(0), 0u);
    EffectOutStatusVersion2 out2{};
    effect.StoreStatus(out2, true);
    CHECK(out2.state == EffectState::Disabled);

    TIn third = MakeInput<TIn>(EffectType::Reverb, true, true, 3, 0x1000, 0x4000, 7);
    SetSpecific(third, rp);
    effect.Update(err, third, mapper);
    CHECK(effect.UsageState == UsageState::New);
    EffectOutStatusVersion1 out1{};
    effect.StoreStatus(out1, false);
    CHECK(out1.state == EffectState::Enabled);
}

NXTEST(reverb_disable_then_new_version1) { CheckReverbDisableThenNew<EffectInParameterVersion1>(); }
NXTEST(reverb_disable_then_new_version2) { CheckReverbDisableThenNew<EffectInParameterVersion2>(); }

// C# ReverbEffect.Update channel rules. A channel_count_max outside {1, 2, 4, 6} changes nothing: MixId, ProcessingOrder, IsEnabled
// and UsageState keep their defaults, and the error info is Success. A valid max with an invalid channel_count copies the
// parameter and the mix id / processing order (UpdateParameterBase runs before the channel_count check), but IsEnabled and
// UsageState are left alone, and BufferUnmapped stays false.
template <class TIn>
void CheckReverbChannelRules() {
    std::vector<MemoryPoolInfo> pools = MakeAttachedPool();
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);
    ReverbEffect effect;
    BehaviourParameter::ErrorInfo err{};

    ReverbParameter invalidMax{};
    invalidMax.channel_count_max = 3;
    invalidMax.channel_count = 2;
    TIn in = MakeInput<TIn>(EffectType::Reverb, true, true, 5, 0x1000, 0x4000, 9);
    SetSpecific(in, invalidMax);
    effect.Update(err, in, mapper);
    CHECK_EQ(effect.MixId, 0x7FFFFFFF);
    CHECK(effect.ProcessingOrder == 0xFFFFFFFFu);
    CHECK(!effect.IsEnabled);
    CHECK(effect.UsageState == UsageState::Invalid);
    CHECK(err.error_code == ResultCode::Success);

    ReverbParameter invalidCount{};
    invalidCount.channel_count_max = 2;
    invalidCount.channel_count = 3;
    TIn in2 = MakeInput<TIn>(EffectType::Reverb, true, true, 5, 0x1000, 0x4000, 9);
    SetSpecific(in2, invalidCount);
    effect.Update(err, in2, mapper);
    CHECK_EQ(effect.MixId, 5);
    CHECK(effect.ProcessingOrder == 9u);
    CHECK(!effect.IsEnabled);
    CHECK(effect.UsageState == UsageState::Invalid);
    CHECK(!effect.BufferUnmapped);
    uint32_t storedChannels = effect.Parameter.channel_count;
    CHECK_EQ(storedChannels, 3u);
}

NXTEST(reverb_channel_rules_version1) { CheckReverbChannelRules<EffectInParameterVersion1>(); }
NXTEST(reverb_channel_rules_version2) { CheckReverbChannelRules<EffectInParameterVersion2>(); }

// C# TryAttachBuffer for a range outside every pool, with force map disabled: error InvalidAddressInfo whose extra info is the CPU
// address, and the return value false, so BufferUnmapped becomes true. UsageState is New because is_new is set.
NXTEST(reverb_unmapped_buffer_reports_error) {
    std::vector<MemoryPoolInfo> pools = MakeAttachedPool();
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);

    ReverbParameter rp{};
    rp.channel_count_max = 2;
    rp.channel_count = 2;
    EffectInParameterVersion1 in = MakeInput<EffectInParameterVersion1>(EffectType::Reverb, true, true, 1, 0x9000, 0x100, 0);
    SetSpecific(in, rp);

    ReverbEffect effect;
    BehaviourParameter::ErrorInfo err{};
    effect.Update(err, in, mapper);
    CHECK(effect.BufferUnmapped);
    CHECK(effect.IsEnabled);
    CHECK(effect.UsageState == UsageState::New);
    CHECK(err.error_code == ResultCode::InvalidAddressInfo);
    uint64_t extra = err.extra_error_info;
    CHECK_EQ(extra, 0x9000u);
}

// ---- Delay ----

// C# DelayEffect.Update, first update: mix 4, processing order 2, enabled, new, buffer attached. delay_time 1000.
// Expected: MixId 4, ProcessingOrder 2, IsEnabled true, UsageState New, BufferUnmapped false, delay_time 1000; after command
// generation UsageState Enabled and GetWorkBuffer(0) = 0x1000.
template <class TIn>
void CheckDelayFirstUpdate() {
    std::vector<MemoryPoolInfo> pools = MakeAttachedPool();
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);

    DelayParameter dp{};
    dp.channel_count_max = 2;
    dp.channel_count = 2;
    dp.delay_time = 1000;
    dp.sample_rate = 48000;
    TIn in = MakeInput<TIn>(EffectType::Delay, true, true, 4, 0x1000, 0x4000, 2);
    SetSpecific(in, dp);

    DelayEffect effect;
    CHECK(effect.Type == EffectType::Delay);
    BehaviourParameter::ErrorInfo err{};
    effect.Update(err, in, mapper);

    CHECK_EQ(effect.MixId, 4);
    CHECK(effect.ProcessingOrder == 2u);
    CHECK(effect.IsEnabled);
    CHECK(effect.UsageState == UsageState::New);
    CHECK(!effect.BufferUnmapped);
    CHECK(err.error_code == ResultCode::Success);
    uint32_t delayTime = effect.Parameter.delay_time;
    CHECK_EQ(delayTime, 1000u);

    effect.UpdateForCommandGeneration();
    CHECK(effect.UsageState == UsageState::Enabled);
    CHECK_EQ(effect.GetWorkBuffer(0), 0x1000u);
}

NXTEST(delay_first_update_version1) { CheckDelayFirstUpdate<EffectInParameterVersion1>(); }
NXTEST(delay_first_update_version2) { CheckDelayFirstUpdate<EffectInParameterVersion2>(); }

// ---- Limiter ----

// C# LimiterEffect.Update has no channel-count check: channel_count_max 0 is still applied (MixId 6, IsEnabled true, UsageState New,
// attach succeeds). InitializeResultState on a zeroed result state (C# LimiterStatistics.Reset): InputMax all 0 and
// CompressionGainMin all 1. UpdateResultState copies the source state.
NXTEST(limiter_update_and_result_state) {
    std::vector<MemoryPoolInfo> pools = MakeAttachedPool();
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);

    LimiterParameter lp{};
    lp.channel_count_max = 0;
    lp.channel_count = 0;
    lp.sample_rate = 48000;
    EffectInParameterVersion1 in = MakeInput<EffectInParameterVersion1>(EffectType::Limiter, true, true, 6, 0x1000, 0x4000, 1);
    SetSpecific(in, lp);

    LimiterEffect effect;
    BehaviourParameter::ErrorInfo err{};
    effect.Update(err, in, mapper);
    CHECK_EQ(effect.MixId, 6);
    CHECK(effect.IsEnabled);
    CHECK(effect.UsageState == UsageState::New);
    CHECK(!effect.BufferUnmapped);
    CHECK(err.error_code == ResultCode::Success);

    EffectResultState zeroed{};
    effect.InitializeResultState(zeroed);
    LimiterStatistics stats = ReadBytes<LimiterStatistics>(zeroed.specific_data);
    for (int i = 0; i < 6; i++) {
        CHECK(stats.input_max[i] == 0.0f);
        CHECK(stats.compression_gain_min[i] == 1.0f);
    }

    EffectResultState source{};
    std::memset(source.specific_data, 0xAB, sizeof(source.specific_data));
    EffectResultState destination{};
    effect.UpdateResultState(destination, source);
    CHECK(std::memcmp(destination.specific_data, source.specific_data, sizeof(source.specific_data)) == 0);
}

// ---- Reverb3d ----

// C# Reverb3dEffect.Update uses ParameterStatus (the field parameter_status). Same flow as the reverb effect: first update New with
// the buffer attached, command generation sets ParameterStatus Enabled, and GetWorkBuffer(0) = 0x1000.
NXTEST(reverb3d_first_update_and_status) {
    std::vector<MemoryPoolInfo> pools = MakeAttachedPool();
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);

    Reverb3dParameter rp{};
    rp.channel_count_max = 2;
    rp.channel_count = 2;
    rp.sample_rate = 48000;
    EffectInParameterVersion2 in = MakeInput<EffectInParameterVersion2>(EffectType::Reverb3d, true, true, 7, 0x1000, 0x4000, 1);
    SetSpecific(in, rp);

    Reverb3dEffect effect;
    BehaviourParameter::ErrorInfo err{};
    effect.Update(err, in, mapper);
    CHECK_EQ(effect.MixId, 7);
    CHECK(effect.IsEnabled);
    CHECK(effect.UsageState == UsageState::New);
    CHECK(effect.Parameter.parameter_status == UsageState::Invalid);
    CHECK(!effect.BufferUnmapped);

    effect.UpdateForCommandGeneration();
    CHECK(effect.Parameter.parameter_status == UsageState::Enabled);
    CHECK_EQ(effect.GetWorkBuffer(0), 0x1000u);
}

// ---- Compressor ----

// C# CompressorEffect: the version 1 Update only initialises the error info, so MixId, ProcessingOrder, IsEnabled and UsageState
// keep their defaults. The version 2 Update applies MixId 8, ProcessingOrder 3, Parameter and IsEnabled true.
// Command generation: UsageState Enabled, Parameter.status Enabled, statistics_reset false.
// InitializeResultState: CompressorStatistics.Reset(channel_count 2) sets MaximumMean 0, MinimumGain 1 and clears LastSamples[0..2);
// LastSamples[2..6) keep the 0xFF bytes the test wrote first (bytes 16..31 of the 32-byte struct).
NXTEST(compressor_version1_is_noop_version2_applies) {
    PoolMapper mapper(0, Span<MemoryPoolInfo>(), false);

    CompressorParameter cp{};
    cp.channel_count_max = 2;
    cp.channel_count = 2;
    cp.statistics_reset = true;

    CompressorEffect effect;
    BehaviourParameter::ErrorInfo err{};

    EffectInParameterVersion1 v1 = MakeInput<EffectInParameterVersion1>(EffectType::Compressor, true, true, 8, 0x1000, 0x100, 3);
    SetSpecific(v1, cp);
    effect.Update(err, v1, mapper);
    CHECK_EQ(effect.MixId, 0x7FFFFFFF);
    CHECK(effect.ProcessingOrder == 0xFFFFFFFFu);
    CHECK(!effect.IsEnabled);
    CHECK(effect.UsageState == UsageState::Invalid);
    CHECK(err.error_code == ResultCode::Success);

    EffectInParameterVersion2 v2 = MakeInput<EffectInParameterVersion2>(EffectType::Compressor, true, true, 8, 0x1000, 0x100, 3);
    SetSpecific(v2, cp);
    effect.Update(err, v2, mapper);
    CHECK_EQ(effect.MixId, 8);
    CHECK(effect.ProcessingOrder == 3u);
    CHECK(effect.IsEnabled);
    uint32_t storedChannels = effect.Parameter.channel_count;
    CHECK_EQ(storedChannels, 2u);

    effect.UpdateForCommandGeneration();
    CHECK(effect.UsageState == UsageState::Enabled);
    CHECK(effect.Parameter.status == UsageState::Enabled);
    CHECK(!effect.Parameter.statistics_reset);

    EffectResultState result{};
    std::memset(result.specific_data, 0xFF, sizeof(result.specific_data));
    effect.InitializeResultState(result);
    CompressorStatistics stats = ReadBytes<CompressorStatistics>(result.specific_data);
    CHECK(stats.maximum_mean == 0.0f);
    CHECK(stats.minimum_gain == 1.0f);
    CHECK(stats.last_samples[0] == 0.0f);
    CHECK(stats.last_samples[1] == 0.0f);
    for (int i = 16; i < 32; i++) {
        CHECK(result.specific_data[i] == 0xFF);
    }
}

// ---- BiquadFilter ----

// C# BiquadFilterEffect. Version 1 converts the 16-bit fixed point coefficients with BiquadFilterHelper.ToBiquadFilterEffectParameter2,
// which divides by 2^14 (C# FixedPointHelper.ToFloat): numerator {8192, 4096, 2048} -> {0.5, 0.25, 0.125};
// denominator {4096, -8192} -> {0.25, -0.5}. Version 2 copies the float parameter as it is.
NXTEST(biquad_filter_version1_converts_and_version2_copies) {
    PoolMapper mapper(0, Span<MemoryPoolInfo>(), false);
    BehaviourParameter::ErrorInfo err{};

    BiquadFilterEffectParameter1 p1{};
    p1.channel_count = 2;
    p1.numerator[0] = 8192;
    p1.numerator[1] = 4096;
    p1.numerator[2] = 2048;
    p1.denominator[0] = 4096;
    p1.denominator[1] = -8192;
    EffectInParameterVersion1 in1 = MakeInput<EffectInParameterVersion1>(EffectType::BiquadFilter, false, true, 2, 0, 0, 4);
    SetSpecific(in1, p1);

    BiquadFilterEffect v1Effect(1);
    CHECK(v1Effect.Type == EffectType::BiquadFilter);
    v1Effect.Update(err, in1, mapper);
    CHECK(v1Effect.IsEnabled);
    CHECK_EQ(v1Effect.MixId, 2);
    CHECK(v1Effect.ProcessingOrder == 4u);
    float n0 = v1Effect.Parameter.numerator[0];
    float n1 = v1Effect.Parameter.numerator[1];
    float n2 = v1Effect.Parameter.numerator[2];
    float d0 = v1Effect.Parameter.denominator[0];
    float d1 = v1Effect.Parameter.denominator[1];
    CHECK(n0 == 0.5f);
    CHECK(n1 == 0.25f);
    CHECK(n2 == 0.125f);
    CHECK(d0 == 0.25f);
    CHECK(d1 == -0.5f);

    BiquadFilterEffectParameter2 p2{};
    p2.channel_count = 2;
    p2.numerator[0] = 0.75f;
    p2.numerator[1] = -0.25f;
    p2.numerator[2] = 0.0f;
    p2.denominator[0] = 0.5f;
    p2.denominator[1] = 0.125f;
    EffectInParameterVersion2 in2 = MakeInput<EffectInParameterVersion2>(EffectType::BiquadFilter, false, true, 2, 0, 0, 4);
    SetSpecific(in2, p2);

    BiquadFilterEffect v2Effect(2);
    v2Effect.Update(err, in2, mapper);
    CHECK(v2Effect.IsEnabled);
    CHECK_EQ(v2Effect.BiquadFilterEffectVersion, 2);
    float m0 = v2Effect.Parameter.numerator[0];
    float m1 = v2Effect.Parameter.numerator[1];
    float e0 = v2Effect.Parameter.denominator[0];
    CHECK(m0 == 0.75f);
    CHECK(m1 == -0.25f);
    CHECK(e0 == 0.5f);
    CHECK(err.error_code == ResultCode::Success);
}

// ---- BufferMix ----

// C# BufferMixEffect.Update copies the parameter, IsEnabled, MixId and ProcessingOrder. It has no work buffers.
NXTEST(buffer_mix_update_copies_parameter) {
    PoolMapper mapper(0, Span<MemoryPoolInfo>(), false);
    BufferMixParameter bp{};
    bp.mixes_count = 2;
    bp.volumes[0] = 0.5f;
    bp.volumes[1] = 0.25f;
    EffectInParameterVersion1 in = MakeInput<EffectInParameterVersion1>(EffectType::BufferMix, false, true, 9, 0, 0, 5);
    SetSpecific(in, bp);

    BufferMixEffect effect;
    BehaviourParameter::ErrorInfo err{};
    effect.Update(err, in, mapper);
    CHECK(effect.IsEnabled);
    CHECK_EQ(effect.MixId, 9);
    CHECK(effect.ProcessingOrder == 5u);
    uint32_t mixes = effect.Parameter.mixes_count;
    CHECK_EQ(mixes, 2u);
    float volume0 = effect.Parameter.volumes[0];
    CHECK(volume0 == 0.5f);
    CHECK(err.error_code == ResultCode::Success);

    effect.UpdateForCommandGeneration();
    CHECK(effect.UsageState == UsageState::Enabled);
}

// ---- AuxiliaryBuffer and CaptureBuffer ----

// C# AuxiliaryBufferEffect.Update with buffer_storage_size 0x10: bufferSize = 4 * 0x10 + 0x80 = 0xC0. Send [0x1000, 0x10C0) and
// return [0x2000, 0x20C0) both lie in the attached pool, so both attach with identity DSP addresses. The State fields are
// send_buffer_info = 0x1000 + 0x40 = 0x1040, send_buffer_info_base = 0x1000 + 0x80 = 0x1080, return_buffer_info = 0x2040,
// return_buffer_info_base = 0x2080. BufferUnmapped = false && false = false. GetWorkBuffer(0) = 0x1000, GetWorkBuffer(1) = 0x2000.
NXTEST(aux_resolves_send_and_return_through_pool) {
    std::vector<MemoryPoolInfo> pools = MakeAttachedPool();
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);

    AuxiliaryBufferParameter ap{};
    ap.buffer_storage_size = 0x10;
    ap.send_buffer_info_address = 0x1000;
    ap.return_buffer_info_address = 0x2000;
    EffectInParameterVersion1 in = MakeInput<EffectInParameterVersion1>(EffectType::AuxiliaryBuffer, true, true, 1, 0, 0, 0);
    SetSpecific(in, ap);

    AuxiliaryBufferEffect effect;
    BehaviourParameter::ErrorInfo err{};
    effect.Update(err, in, mapper);
    CHECK(!effect.BufferUnmapped);
    CHECK(effect.IsEnabled);
    CHECK(err.error_code == ResultCode::Success);

    uint64_t sendInfo = effect.State.send_buffer_info;
    uint64_t sendBase = effect.State.send_buffer_info_base;
    uint64_t returnInfo = effect.State.return_buffer_info;
    uint64_t returnBase = effect.State.return_buffer_info_base;
    CHECK_EQ(sendInfo, 0x1040u);
    CHECK_EQ(sendBase, 0x1080u);
    CHECK_EQ(returnInfo, 0x2040u);
    CHECK_EQ(returnBase, 0x2080u);

    effect.UpdateForCommandGeneration();
    CHECK_EQ(effect.GetWorkBuffer(0), 0x1000u);
    CHECK_EQ(effect.GetWorkBuffer(1), 0x2000u);
}

// C# AuxiliaryBufferEffect.Update with the return buffer outside the pool: only the return attach fails, so BufferUnmapped stays false
// (the send buffer attached). The error info is the return attach's: InvalidAddressInfo with extra 0x2000. The failed buffer's DSP
// address is ForceMappedDspAddress = 0, so return_buffer_info = 0 + 0x40 = 0x40 and return_buffer_info_base = 0x80.
NXTEST(aux_return_outside_pool_reports_error_but_stays_mapped) {
    std::vector<MemoryPoolInfo> pools = MakeAttachedPool();
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);

    AuxiliaryBufferParameter ap{};
    ap.buffer_storage_size = 0x10;
    ap.send_buffer_info_address = 0x1000;
    ap.return_buffer_info_address = 0x9000;
    EffectInParameterVersion1 in = MakeInput<EffectInParameterVersion1>(EffectType::AuxiliaryBuffer, true, true, 1, 0, 0, 0);
    SetSpecific(in, ap);

    AuxiliaryBufferEffect effect;
    BehaviourParameter::ErrorInfo err{};
    effect.Update(err, in, mapper);
    CHECK(!effect.BufferUnmapped);
    CHECK(err.error_code == ResultCode::InvalidAddressInfo);
    uint64_t extra = err.extra_error_info;
    CHECK_EQ(extra, 0x9000u);
    uint64_t returnInfo = effect.State.return_buffer_info;
    uint64_t returnBase = effect.State.return_buffer_info_base;
    CHECK_EQ(returnInfo, 0x40u);
    CHECK_EQ(returnBase, 0x80u);
}

// C# CaptureBufferEffect.Update resolves only the send buffer. [0x1000, 0x10C0) attaches: send_buffer_info = 0x1000,
// send_buffer_info_base = 0x1080, and the return fields stay 0 whatever return_buffer_info_address says.
NXTEST(capture_buffer_resolves_send_only) {
    std::vector<MemoryPoolInfo> pools = MakeAttachedPool();
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);

    AuxiliaryBufferParameter ap{};
    ap.buffer_storage_size = 0x10;
    ap.send_buffer_info_address = 0x1000;
    ap.return_buffer_info_address = 0x2000;
    EffectInParameterVersion2 in = MakeInput<EffectInParameterVersion2>(EffectType::CaptureBuffer, true, true, 1, 0, 0, 0);
    SetSpecific(in, ap);

    CaptureBufferEffect effect;
    BehaviourParameter::ErrorInfo err{};
    effect.Update(err, in, mapper);
    CHECK(!effect.BufferUnmapped);
    CHECK(effect.IsEnabled);
    uint64_t sendInfo = effect.State.send_buffer_info;
    uint64_t sendBase = effect.State.send_buffer_info_base;
    uint64_t returnInfo = effect.State.return_buffer_info;
    uint64_t returnBase = effect.State.return_buffer_info_base;
    CHECK_EQ(sendInfo, 0x1000u);
    CHECK_EQ(sendBase, 0x1080u);
    CHECK_EQ(returnInfo, 0u);
    CHECK_EQ(returnBase, 0u);

    effect.UpdateForCommandGeneration();
    CHECK_EQ(effect.GetWorkBuffer(0), 0x1000u);
}

// C# CaptureBufferEffect.Update with the send buffer outside the pool: BufferUnmapped is set, the error is InvalidAddressInfo with
// extra 0x9000, and State is not written (it stays zero).
NXTEST(capture_buffer_unmapped_send_leaves_state_untouched) {
    std::vector<MemoryPoolInfo> pools = MakeAttachedPool();
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);

    AuxiliaryBufferParameter ap{};
    ap.buffer_storage_size = 0x10;
    ap.send_buffer_info_address = 0x9000;
    EffectInParameterVersion1 in = MakeInput<EffectInParameterVersion1>(EffectType::CaptureBuffer, true, true, 1, 0, 0, 0);
    SetSpecific(in, ap);

    CaptureBufferEffect effect;
    BehaviourParameter::ErrorInfo err{};
    effect.Update(err, in, mapper);
    CHECK(effect.BufferUnmapped);
    CHECK(err.error_code == ResultCode::InvalidAddressInfo);
    uint64_t extra = err.extra_error_info;
    CHECK_EQ(extra, 0x9000u);
    uint64_t sendInfo = effect.State.send_buffer_info;
    CHECK_EQ(sendInfo, 0u);
}

// ---- Effect-type change and EffectContext ----

// The type change is handled by C# StateUpdater.ResetEffect, not by the effect: IsTypeValid is false for a parameter of another
// type, and the slot is then replaced by an effect of the parameter's type (here a DelayEffect for a Delay input). The C# effect
// reports no error for a mismatch (its Debug.Assert is compiled out in Release), so the error info stays Success.
NXTEST(effect_type_change_replaces_slot) {
    std::vector<MemoryPoolInfo> pools = MakeAttachedPool();
    PoolMapper mapper(0, Span<MemoryPoolInfo>(pools), false);

    EffectContext context;
    context.Initialize(1, 1);
    context.GetEffect(0) = std::make_unique<ReverbEffect>();
    CHECK(context.GetEffect(0)->Type == EffectType::Reverb);

    DelayParameter dp{};
    dp.channel_count_max = 2;
    dp.channel_count = 2;
    EffectInParameterVersion1 delayIn = MakeInput<EffectInParameterVersion1>(EffectType::Delay, true, true, 1, 0x1000, 0x4000, 0);
    SetSpecific(delayIn, dp);
    CHECK(!context.GetEffect(0)->IsTypeValid(delayIn));

    context.GetEffect(0)->ForceUnmapBuffers(mapper);
    context.GetEffect(0) = std::make_unique<DelayEffect>();
    CHECK(context.GetEffect(0)->IsTypeValid(delayIn));
    CHECK(context.GetEffect(0)->Type == EffectType::Delay);

    BehaviourParameter::ErrorInfo err{};
    context.GetEffect(0)->Update(err, delayIn, mapper);
    CHECK(context.GetEffect(0)->IsEnabled);
    CHECK_EQ(context.GetEffect(0)->MixId, 1);
    CHECK(err.error_code == ResultCode::Success);
}

// C# EffectContext.Initialize(3, 3): three BaseEffects (Invalid), zeroed result states, GetCount 3. GetEffect replaces a slot.
// UpdateResultStateForCommandGeneration: effect i updates CPU state i from DSP state i. Only LimiterEffect overrides it (copy),
// so CPU states 1 and 2 (BaseEffect, no-op) stay zero. GetDspStateMemory(1) is a one-element view of DSP state 1.
NXTEST(effect_context_initialize_get_and_result_states) {
    EffectContext context;
    CHECK_EQ(context.GetCount(), 0u);

    context.Initialize(3, 3);
    CHECK_EQ(context.GetCount(), 3u);
    for (int i = 0; i < 3; i++) {
        CHECK(context.GetEffect(i)->Type == EffectType::Invalid);
        CHECK(context.GetEffect(i)->UsageState == UsageState::Invalid);
    }

    context.GetEffect(0) = std::make_unique<LimiterEffect>();
    CHECK(context.GetEffect(0)->Type == EffectType::Limiter);
    CHECK(context.GetEffect(1)->Type == EffectType::Invalid);

    CHECK(context.GetState(1).specific_data[0] == 0);
    CHECK(context.GetState(1).specific_data[127] == 0);

    Span<EffectResultState> view = context.GetDspStateMemory(1);
    CHECK_EQ(view.Length, 1u);
    CHECK(view.Data == &context.GetDspState(1));

    std::memset(context.GetDspState(0).specific_data, 0x5A, sizeof(EffectResultState::specific_data));
    std::memset(context.GetDspState(2).specific_data, 0x33, sizeof(EffectResultState::specific_data));

    context.UpdateResultStateForCommandGeneration();
    CHECK(std::memcmp(context.GetState(0).specific_data, context.GetDspState(0).specific_data, sizeof(EffectResultState::specific_data)) == 0);
    CHECK(context.GetState(1).specific_data[0] == 0);
    CHECK(context.GetState(2).specific_data[0] == 0);
}

}  // namespace
