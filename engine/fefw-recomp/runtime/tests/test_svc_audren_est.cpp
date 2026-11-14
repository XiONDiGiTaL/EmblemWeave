// SPDX-License-Identifier: GPL-3.0-or-later
// Command processing time estimators, versions 1 to 5. Every expected value below is the C# formula of
// Ryujinx.Audio/Renderer/Server/CommandProcessingTimeEstimatorVersionN.cs evaluated by hand; the arithmetic is written next to each check.
// Float results are truncated to uint like the C# (uint) cast. Where a float product lies close to an integer the float rounding was checked.
#include "../src/services/audren/Server/CommandProcessingTimeEstimatorVersion1.h"
#include "../src/services/audren/Server/CommandProcessingTimeEstimatorVersion2.h"
#include "../src/services/audren/Server/CommandProcessingTimeEstimatorVersion3.h"
#include "../src/services/audren/Server/CommandProcessingTimeEstimatorVersion4.h"
#include "../src/services/audren/Server/CommandProcessingTimeEstimatorVersion5.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_est)

using namespace nxrt::audren;

namespace {
// The effect commands keep their parameters private: they take them through Initialize, as the renderer does.
void InitDelay(DelayCommand& c, uint16_t channels, bool enabled) {
    DelayParameter p{};
    p.channel_count = channels;
    c.Initialize(0, p, Span<DelayState>(), true, 0, 0, false);
    c.Enabled = enabled;
}
void InitReverb(ReverbCommand& c, uint16_t channels, bool enabled) {
    ReverbParameter p{};
    p.channel_count = channels;
    c.Initialize(0, p, Span<ReverbState>(), enabled, 0, 0, false, false);
    c.Enabled = enabled;
}
void InitReverb3d(Reverb3dCommand& c, uint16_t channels, bool enabled) {
    Reverb3dParameter p{};
    p.channel_count = channels;
    c.Initialize(0, p, Span<Reverb3dState>(), enabled, 0, 0, false);
    c.Enabled = enabled;
}
void InitCompressor(CompressorCommand& c, uint16_t channels, bool statistics, bool enabled) {
    CompressorParameter p{};
    p.channel_count = channels;
    p.statistics_enabled = statistics;
    c.Initialize(0, p, Span<CompressorState>(), Span<EffectResultState>(), enabled, 0);
    c.Enabled = enabled;
}
void InitLimiter1(LimiterCommandVersion1& c, uint16_t channels, bool enabled) {
    LimiterParameter p{};
    p.channel_count = channels;
    c.Initialize(0, p, Span<LimiterState>(), enabled, 0, 0);
}
void InitLimiter2(LimiterCommandVersion2& c, uint16_t channels, bool statistics, bool enabled) {
    LimiterParameter p{};
    p.channel_count = channels;
    p.statistics_enabled = statistics;
    c.Initialize(0, p, Span<LimiterState>(), Span<EffectResultState>(), enabled, 0, 0);
}
void SetPcm(PcmInt16DataSourceCommandVersion1& c, uint32_t sampleRate, float pitch) {
    c.SampleRate = sampleRate;
    c.Pitch = pitch;
}
void SetPcmFloat(PcmFloatDataSourceCommandVersion1& c, uint32_t sampleRate, float pitch) {
    c.SampleRate = sampleRate;
    c.Pitch = pitch;
}
void SetAdpcm(AdpcmDataSourceCommandVersion1& c, uint32_t sampleRate, float pitch) {
    c.SampleRate = sampleRate;
    c.Pitch = pitch;
}
void SetV2(DataSourceVersion2Command& c, SampleFormat format, SampleRateConversionQuality quality, uint32_t sampleRate, float pitch) {
    c.SampleFormat_ = format;
    c.SrcQuality = quality;
    c.SampleRate = sampleRate;
    c.Pitch = pitch;
}
}  // namespace

// ---------------------------------------------------------------------------------------------------------------------
// Version 1
// ---------------------------------------------------------------------------------------------------------------------

NXTEST(est_v1_fixed_and_per_sample_costs) {
    CommandProcessingTimeEstimatorVersion1 v1(160, 24);
    ICommandProcessingTimeEstimator& e = v1;

    PerformanceCommand perf;
    CHECK_EQ(e.Estimate(perf), 1454u);  // return 1454

    ClearMixBufferCommand clear;  // 160 * 0.83 * 24 * 1.2 = 132.8 * 24 * 1.2 = 3187.2 * 1.2 = 3824.64 -> 3824
    CHECK_EQ(e.Estimate(clear), 3824u);

    CommandProcessingTimeEstimatorVersion1 v1b(240, 8);  // 240 * 0.83 * 8 * 1.2 = 199.2 * 8 * 1.2 = 1593.6 * 1.2 = 1912.32 -> 1912
    CHECK_EQ(v1b.Estimate(clear), 1912u);

    BiquadFilterCommand biquad;  // 160 * 58 * 1.2 = 9280 * 1.2 = 11136 -> 11136
    CHECK_EQ(e.Estimate(biquad), 11136u);
    CommandProcessingTimeEstimatorVersion1 v1c(240, 8);  // 240 * 58 * 1.2 = 13920 * 1.2 = 16704 -> 16704
    CHECK_EQ(v1c.Estimate(biquad), 16704u);

    MixRampCommand ramp;  // 240 * 14.4 * 1.2 = 3456 * 1.2 = 4147.2 -> 4147
    CHECK_EQ(v1c.Estimate(ramp), 4147u);

    VolumeRampCommand vramp;  // 160 * 9.8 * 1.2 = 1568 * 1.2 = 1881.6 -> 1881
    CHECK_EQ(e.Estimate(vramp), 1881u);

    VolumeCommand vol;  // 240 * 8.8 * 1.2 = 2112 * 1.2 = 2534.4 -> 2534
    CHECK_EQ(v1c.Estimate(vol), 2534u);

    MixCommand mix;  // 160 * 10 * 1.2 = 1600 * 1.2 = 1920 -> 1920
    CHECK_EQ(e.Estimate(mix), 1920u);

    DepopPrepareCommand prep;  // return 1080
    CHECK_EQ(e.Estimate(prep), 1080u);
}

NXTEST(est_v1_mix_ramp_grouped_counts_nonzero_volumes) {
    // volumeCount = number of buffers with Volume0 != 0 or Volume1 != 0
    MixRampGroupedCommand g;
    g.MixBufferCount = 4;
    g.Volume0 = {1.0f, 0.0f, 0.5f, 0.0f};  // buffers 0 and 2 -> 2
    g.Volume1 = {0.0f, 0.0f, 0.0f, 0.0f};
    CommandProcessingTimeEstimatorVersion1 v1(160, 4);
    // 160 * 14.4 * 1.2 * 2 = 2304 * 1.2 * 2 = 2764.8 * 2 = 5529.6 -> 5529
    CHECK_EQ(v1.Estimate(g), 5529u);

    MixRampGroupedCommand g2;
    g2.MixBufferCount = 3;
    g2.Volume0 = {0.0f, 0.0f, 0.0f};
    g2.Volume1 = {0.0f, 0.25f, 0.0f};  // buffer 1 only -> 1
    CommandProcessingTimeEstimatorVersion1 v1b(240, 3);
    // 240 * 14.4 * 1.2 * 1 = 4147.2 -> 4147
    CHECK_EQ(v1b.Estimate(g2), 4147u);
}

NXTEST(est_v1_per_buffer_and_per_channel_costs) {
    CommandProcessingTimeEstimatorVersion1 v1(240, 5);
    ICommandProcessingTimeEstimator& e = v1;

    // (240 * 8.9) * 5 = 2136 * 5 = 10680 (240 * 8.9f rounds to exactly 2136.0f in float)
    DepopForMixBuffersCommand depop;
    depop.MixBufferCount = 5;
    CHECK_EQ(e.Estimate(depop), 10680u);

    // PCM16 data source: Pitch * 0.25 * 1.2 = 100 * 0.25 * 1.2 = 25 * 1.2 = 30 -> 30
    PcmInt16DataSourceCommandVersion1 pcm;
    SetPcm(pcm, 48000, 100.0f);
    CHECK_EQ(e.Estimate(pcm), 30u);

    // ADPCM data source: Pitch * 0.46 * 1.2 = 100 * 0.46 * 1.2 = 46 * 1.2 = 55.2 -> 55
    AdpcmDataSourceCommandVersion1 adpcm;
    SetAdpcm(adpcm, 48000, 100.0f);
    CHECK_EQ(e.Estimate(adpcm), 55u);

    // Delay: 240 * 2 channels * 202.5 = 480 * 202.5 = 97200 -> 97200
    DelayCommand delay;
    InitDelay(delay, 2, true);
    CHECK_EQ(e.Estimate(delay), 97200u);
}

NXTEST(est_v1_reverb_and_reverb3d_by_channel_count) {
    CommandProcessingTimeEstimatorVersion1 v1(160, 4);
    ICommandProcessingTimeEstimator& e = v1;
    CommandProcessingTimeEstimatorVersion1 v1b(240, 4);

    // Reverb enabled, 2 channels, 160: 750 * 160 * 2 * 1.2 = 240000 * 1.2 = 288000 -> 288000
    ReverbCommand rev;
    InitReverb(rev, 2, true);
    CHECK_EQ(e.Estimate(rev), 288000u);

    // Reverb disabled: return 0
    ReverbCommand revOff;
    InitReverb(revOff, 2, false);
    CHECK_EQ(e.Estimate(revOff), 0u);

    // Reverb enabled, 6 channels, 240: 750 * 240 * 6 * 1.2 = 1080000 * 1.2 = 1296000 -> 1296000
    ReverbCommand rev6;
    InitReverb(rev6, 6, true);
    CHECK_EQ(v1b.Estimate(rev6), 1296000u);

    // Reverb3d enabled, 1 channel, 160: 530 * 160 * 1 * 1.2 = 84800 * 1.2 = 101760 -> 101760
    Reverb3dCommand r3;
    InitReverb3d(r3, 1, true);
    CHECK_EQ(e.Estimate(r3), 101760u);

    // Reverb3d enabled, 4 channels, 240: 530 * 240 * 4 * 1.2 = 508800 * 1.2 = 610560 -> 610560
    Reverb3dCommand r3b;
    InitReverb3d(r3b, 4, true);
    CHECK_EQ(v1b.Estimate(r3b), 610560u);

    // Reverb3d disabled: return 0
    Reverb3dCommand r3off;
    InitReverb3d(r3off, 4, false);
    CHECK_EQ(v1b.Estimate(r3off), 0u);
}

NXTEST(est_v1_fixed_cost_commands) {
    CommandProcessingTimeEstimatorVersion1 v1(160, 2);
    ICommandProcessingTimeEstimator& e = v1;

    AuxiliaryBufferCommand aux;
    aux.Enabled = true;
    CHECK_EQ(e.Estimate(aux), 15956u);
    aux.Enabled = false;
    CHECK_EQ(e.Estimate(aux), 3765u);

    CircularBufferSinkCommand cbs;
    CHECK_EQ(e.Estimate(cbs), 55u);

    DownMixSurroundToStereoCommand down;
    CHECK_EQ(e.Estimate(down), 16108u);

    UpsampleCommand up;
    CHECK_EQ(e.Estimate(up), 357915u);

    DeviceSinkCommand dev;
    dev.InputCount = 2;
    CHECK_EQ(e.Estimate(dev), 10042u);

    CopyMixBufferCommand copy;  // version 1 returns 0 for the copy
    CHECK_EQ(e.Estimate(copy), 0u);

    PcmFloatDataSourceCommandVersion1 pcmf;  // version 1 float source: 0
    CHECK_EQ(e.Estimate(pcmf), 0u);

    DataSourceVersion2Command ds2;  // version 1: 0
    CHECK_EQ(e.Estimate(ds2), 0u);

    LimiterCommandVersion1 lim1;  // version 1: 0
    CHECK_EQ(e.Estimate(lim1), 0u);
}

// ---------------------------------------------------------------------------------------------------------------------
// Version 2 (sample count 160 or 240 selects the constants)
// ---------------------------------------------------------------------------------------------------------------------

NXTEST(est_v2_performance_and_clear_mix_buffer) {
    CommandProcessingTimeEstimatorVersion2 a(160, 4);
    CommandProcessingTimeEstimatorVersion2 b(240, 3);
    PerformanceCommand perf;
    CHECK_EQ(a.Estimate(perf), 489u);  // 489.35 -> 489
    CHECK_EQ(b.Estimate(perf), 491u);  // 491.18 -> 491

    ClearMixBufferCommand clear;
    // 160: 139.65 + 260.4 * 4 = 139.65 + 1041.6 = 1181.25 -> 1181
    CHECK_EQ(a.Estimate(clear), 1181u);
    // 240: 193.2 + 668.8 * 3 = 193.2 + 2006.4 = 2199.6 -> 2199
    CHECK_EQ(b.Estimate(clear), 2199u);
}

NXTEST(est_v2_biquad_mixramp_depop_volumeramp) {
    CommandProcessingTimeEstimatorVersion2 a(160, 1);
    CommandProcessingTimeEstimatorVersion2 b(240, 1);

    BiquadFilterCommand biquad;
    CHECK_EQ(a.Estimate(biquad), 4813u);  // 4813.2 -> 4813
    CHECK_EQ(b.Estimate(biquad), 6915u);  // 6915.4 -> 6915

    MixRampCommand ramp;
    CHECK_EQ(a.Estimate(ramp), 1859u);  // 1859.0
    CHECK_EQ(b.Estimate(ramp), 2286u);  // 2286.1 -> 2286

    DepopPrepareCommand prep;
    CHECK_EQ(a.Estimate(prep), 306u);  // 306.62 -> 306
    CHECK_EQ(b.Estimate(prep), 293u);  // 293.22 -> 293

    VolumeRampCommand vramp;
    CHECK_EQ(a.Estimate(vramp), 1403u);  // 1403.9 -> 1403
    CHECK_EQ(b.Estimate(vramp), 1884u);  // 1884.3 -> 1884
}

NXTEST(est_v2_mix_ramp_grouped_per_sample) {
    // 240 * 7.245 * 1 = 1738.8 -> 1738 (one buffer with a volume)
    MixRampGroupedCommand g;
    g.MixBufferCount = 4;
    g.Volume0 = {0.0f, 0.25f, 0.0f, 0.0f};
    g.Volume1 = {0.0f, 0.0f, 0.0f, 0.0f};
    CommandProcessingTimeEstimatorVersion2 b(240, 4);
    CHECK_EQ(b.Estimate(g), 1738u);

    // 160 * 7.245 * 2 = 1159.2 * 2 = 2318.4 -> 2318 (buffers 0 and 3 with volumes)
    MixRampGroupedCommand g2;
    g2.MixBufferCount = 4;
    g2.Volume0 = {1.0f, 0.0f, 0.0f, 0.0f};
    g2.Volume1 = {0.0f, 0.0f, 0.0f, 0.5f};
    CommandProcessingTimeEstimatorVersion2 a(160, 4);
    CHECK_EQ(a.Estimate(g2), 2318u);
}

NXTEST(est_v2_data_sources_pcm16_adpcm_float) {
    // Pitch 32768 is 1.0 in fixed point: 32768 * 0.000030518 = 1.000013824. SampleRate 48000 -> 48000 / 200 = 240.
    CommandProcessingTimeEstimatorVersion2 a(160, 1);
    CommandProcessingTimeEstimatorVersion2 b(240, 1);

    PcmInt16DataSourceCommandVersion1 pcm;
    SetPcm(pcm, 48000, 32768.0f);
    // 160: (240 / 160) = 1.5; 1.5 * 1.000013824 = 1.500020736; 749.27 * 1.500020736 = 1123.9205; 6138.9 + 1123.9205 = 7262.8205 -> 7262
    CHECK_EQ(a.Estimate(pcm), 7262u);
    // 240: (240 / 240) = 1.0; 1195.5 * 1.000013824 = 1195.5165; 7797.0 + 1195.5165 = 8992.5165 -> 8992
    CHECK_EQ(b.Estimate(pcm), 8992u);

    AdpcmDataSourceCommandVersion1 adpcm;
    SetAdpcm(adpcm, 48000, 32768.0f);
    // 160: 9039.5 + 2125.6 * 1.500020736 = 9039.5 + 3188.4441 = 12227.9441 -> 12227
    CHECK_EQ(a.Estimate(adpcm), 12227u);
    // 240: 6225.5 + 3564.1 * 1.000013824 = 6225.5 + 3564.1493 = 9789.6493 -> 9789
    CHECK_EQ(b.Estimate(adpcm), 9789u);

    PcmFloatDataSourceCommandVersion1 pf;
    SetPcmFloat(pf, 48000, 32768.0f);
    // 160: 7845.3 + 2310.4 * 1.500020736 = 7845.3 + 3465.6479 = 11310.9479 -> 11310
    CHECK_EQ(a.Estimate(pf), 11310u);
    // 240: 10091.0 + 3490.9 * 1.000013824 = 10091.0 + 3490.9483 = 13581.9483 -> 13581
    CHECK_EQ(b.Estimate(pf), 13581u);

    // The version 2 data source command: 0
    DataSourceVersion2Command ds2;
    SetV2(ds2, SampleFormat::PcmInt16, SampleRateConversionQuality::Default, 48000, 32768.0f);
    CHECK_EQ(a.Estimate(ds2), 0u);
}

NXTEST(est_v2_volume_mix_and_delay_by_channel) {
    CommandProcessingTimeEstimatorVersion2 a(160, 1);
    CommandProcessingTimeEstimatorVersion2 b(240, 1);

    VolumeCommand vol;
    CHECK_EQ(a.Estimate(vol), 1280u);  // 1280.3 -> 1280
    CHECK_EQ(b.Estimate(vol), 1737u);  // 1737.8 -> 1737

    MixCommand mix;
    CHECK_EQ(a.Estimate(mix), 1342u);  // 1342.2 -> 1342
    CHECK_EQ(b.Estimate(mix), 1833u);  // 1833.2 -> 1833

    DelayCommand d;
    InitDelay(d, 2, true);
    CHECK_EQ(a.Estimate(d), 97861u);  // 160, enabled, 2 channels: 97861.0
    InitDelay(d, 4, false);
    CHECK_EQ(a.Estimate(d), 703u);    // 160, disabled, 4 channels: 703.98 -> 703
    InitDelay(d, 6, true);
    CHECK_EQ(b.Estimate(d), 81629u);  // 240, enabled, 6 channels: 81629.0
    InitDelay(d, 1, false);
    CHECK_EQ(b.Estimate(d), 521u);    // 240, disabled, 1 channel: 521.28 -> 521
}

NXTEST(est_v2_reverb_reverb3d_aux_sinks) {
    CommandProcessingTimeEstimatorVersion2 a(160, 1);
    CommandProcessingTimeEstimatorVersion2 b(240, 1);

    ReverbCommand r;
    InitReverb(r, 4, true);
    CHECK_EQ(a.Estimate(r), 109580u);  // 160, enabled, 4 ch
    InitReverb(r, 6, false);
    CHECK_EQ(a.Estimate(r), 656u);     // 160, disabled, 6 ch: 656.62 -> 656
    InitReverb(r, 2, false);
    CHECK_EQ(b.Estimate(r), 527u);     // 240, disabled, 2 ch: 527.16 -> 527
    InitReverb(r, 1, true);
    CHECK_EQ(b.Estimate(r), 136460u);  // 240, enabled, 1 ch

    Reverb3dCommand r3;
    InitReverb3d(r3, 2, true);
    CHECK_EQ(a.Estimate(r3), 135430u);  // 160, enabled, 2 ch
    InitReverb3d(r3, 4, false);
    CHECK_EQ(b.Estimate(r3), 660u);     // 240, disabled, 4 ch: 660.93 -> 660

    AuxiliaryBufferCommand aux;
    aux.Enabled = true;
    CHECK_EQ(a.Estimate(aux), 7177u);  // 7177.9 -> 7177
    CHECK_EQ(b.Estimate(aux), 9499u);  // 9499.8 -> 9499
    aux.Enabled = false;
    CHECK_EQ(a.Estimate(aux), 489u);   // 489.16 -> 489

    CircularBufferSinkCommand cbs;
    cbs.InputCount = 2;
    // 240: 1369.7 + 1726.0 * 2 = 4821.7 -> 4821
    CHECK_EQ(b.Estimate(cbs), 4821u);
    cbs.InputCount = 6;
    // 160: 1284.5 + 853.63 * 6 = 1284.5 + 5121.78 = 6406.28 -> 6406
    CHECK_EQ(a.Estimate(cbs), 6406u);

    DeviceSinkCommand dev;
    dev.InputCount = 2;
    CHECK_EQ(a.Estimate(dev), 9261u);  // 160, 2 inputs: 9261.5 -> 9261
    dev.InputCount = 6;
    CHECK_EQ(b.Estimate(dev), 9566u);  // 240, 6 inputs: 9566.7 -> 9566

    DownMixSurroundToStereoCommand down;
    CHECK_EQ(a.Estimate(down), 10009u);  // 160: 10009.0
    CHECK_EQ(b.Estimate(down), 14577u);  // 240: 14577.0

    UpsampleCommand up;
    CHECK_EQ(a.Estimate(up), 292000u);  // 160: 292000.0
    CHECK_EQ(b.Estimate(up), 0u);       // 240: 0.0
}

// ---------------------------------------------------------------------------------------------------------------------
// Version 3
// ---------------------------------------------------------------------------------------------------------------------

NXTEST(est_v3_performance_clear_biquad_mixramp) {
    CommandProcessingTimeEstimatorVersion3 a(160, 4);
    CommandProcessingTimeEstimatorVersion3 b(240, 3);
    PerformanceCommand perf;
    CHECK_EQ(a.Estimate(perf), 498u);  // 498.17 -> 498
    CHECK_EQ(b.Estimate(perf), 489u);  // 489.42 -> 489

    ClearMixBufferCommand clear;
    // 160: 0 + 266.65 * 4 = 1066.6 -> 1066
    CHECK_EQ(a.Estimate(clear), 1066u);
    // 240: 0 + 440.68 * 3 = 1322.04 -> 1322
    CHECK_EQ(b.Estimate(clear), 1322u);

    BiquadFilterCommand biquad;
    CHECK_EQ(a.Estimate(biquad), 4173u);  // 4173.2 -> 4173
    CHECK_EQ(b.Estimate(biquad), 5585u);  // 5585.1 -> 5585

    MixRampCommand ramp;
    CHECK_EQ(a.Estimate(ramp), 1968u);  // 1968.7 -> 1968
    CHECK_EQ(b.Estimate(ramp), 2459u);  // 2459.4 -> 2459
}

NXTEST(est_v3_mix_ramp_grouped_and_volume_ramp) {
    // 160: cost 6.708 per sample; 160 * 6.708 * 3 = 1073.28 * 3 = 3219.84 -> 3219 (3 buffers with volume)
    MixRampGroupedCommand g;
    g.MixBufferCount = 3;
    g.Volume0 = {1.0f, 1.0f, 1.0f};
    g.Volume1 = {0.0f, 0.0f, 0.0f};
    CommandProcessingTimeEstimatorVersion3 a(160, 3);
    CHECK_EQ(a.Estimate(g), 3219u);

    // 240: cost 6.4434 per sample; 240 * 6.4434 * 2 = 1546.416 * 2 = 3092.832 -> 3092 (2 buffers with volume)
    MixRampGroupedCommand g2;
    g2.MixBufferCount = 3;
    g2.Volume0 = {0.0f, 0.0f, 0.0f};
    g2.Volume1 = {0.0f, 0.1f, 0.1f};
    CommandProcessingTimeEstimatorVersion3 b(240, 3);
    CHECK_EQ(b.Estimate(g2), 3092u);

    VolumeRampCommand vramp;
    CHECK_EQ(a.Estimate(vramp), 1425u);  // 1425.3 -> 1425
    CHECK_EQ(b.Estimate(vramp), 1700u);  // 1700.0
}

NXTEST(est_v3_pcm16_adpcm_float_data_sources) {
    // Pitch 32768 -> factor 1.000013824, SampleRate 48000 -> 48000 / 200 = 240
    CommandProcessingTimeEstimatorVersion3 a(160, 1);
    CommandProcessingTimeEstimatorVersion3 b(240, 1);

    PcmInt16DataSourceCommandVersion1 pcm;
    SetPcm(pcm, 48000, 32768.0f);
    // 160: 1.5 * 1.000013824 = 1.500020736; 427.52 * 1.500020736 = 641.2889; 6329.442 + 641.2889 = 6970.7309 -> 6970
    CHECK_EQ(a.Estimate(pcm), 6970u);
    // 240: 1.0 * 1.000013824; 710.143 * 1.000013824 = 710.1528; 7853.286 + 710.1528 = 8563.4388 -> 8563
    CHECK_EQ(b.Estimate(pcm), 8563u);

    AdpcmDataSourceCommandVersion1 adpcm;
    SetAdpcm(adpcm, 48000, 32768.0f);
    // 160: 7913.808 + 2125.6 * 1.500020736 = 7913.808 + 3188.4441 = 11102.2521 -> 11102
    CHECK_EQ(a.Estimate(adpcm), 11102u);
    // 240: 9736.702 + 3564.1 * 1.000013824 = 9736.702 + 3564.1493 = 13300.8513 -> 13300
    CHECK_EQ(b.Estimate(adpcm), 13300u);

    PcmFloatDataSourceCommandVersion1 pf;
    SetPcmFloat(pf, 48000, 32768.0f);
    // 160: 7845.25 + 2310.4 * 1.500020736 = 7845.25 + 3465.6479 = 11310.8979 -> 11310
    CHECK_EQ(a.Estimate(pf), 11310u);
    // 240: 10090.9 + 3490.9 * 1.000013824 = 10090.9 + 3490.9483 = 13581.8483 -> 13581
    CHECK_EQ(b.Estimate(pf), 13581u);
}

NXTEST(est_v3_data_source_v2_by_format_and_quality) {
    CommandProcessingTimeEstimatorVersion3 a(160, 1);
    CommandProcessingTimeEstimatorVersion3 b(240, 1);

    // The v2 cost is base + cost * (X * Y - 1), X = (SampleRate / 200) / sampleCount, Y = Pitch * 0.000030518.
    DataSourceVersion2Command ds;
    SetV2(ds, SampleFormat::PcmInt16, SampleRateConversionQuality::Default, 48000, 32768.0f);
    // 160, PcmInt16 Default: base 6329.44, cost 427.52; X = 1.5, X*Y - 1 = 1.500020736 - 1 = 0.500020736
    // 6329.44 + 427.52 * 0.500020736 = 6329.44 + 213.7689 = 6543.2089 -> 6543
    CHECK_EQ(a.Estimate(ds), 6543u);

    SetV2(ds, SampleFormat::PcmFloat, SampleRateConversionQuality::Low, 48000, 32768.0f);
    // 240, PcmFloat Low: base 12520.85, cost 3480.61; X = 1.0, X*Y - 1 = 0.000013824
    // 12520.85 + 3480.61 * 0.000013824 = 12520.85 + 0.0481 = 12520.8981 -> 12520
    CHECK_EQ(b.Estimate(ds), 12520u);

    SetV2(ds, SampleFormat::Adpcm, SampleRateConversionQuality::High, 48000, 32768.0f);
    // 160, Adpcm High: base 9607.81, cost 1829.29; X*Y - 1 = 0.500020736
    // 9607.81 + 1829.29 * 0.500020736 = 9607.81 + 914.6829 = 10522.4929 -> 10522
    CHECK_EQ(a.Estimate(ds), 10522u);

    // Negative term: 240, PcmInt16 High, SampleRate 24000, Pitch 16384 -> X = 120 / 240 = 0.5, Y = 16384 * 0.000030518 = 0.500006912
    // X*Y - 1 = 0.250003456 - 1 = -0.749996544
    // 10138.84 + 610.49 * (-0.749996544) = 10138.84 - 457.8654 = 9680.9746 -> 9680
    SetV2(ds, SampleFormat::PcmInt16, SampleRateConversionQuality::High, 24000, 16384.0f);
    CHECK_EQ(b.Estimate(ds), 9680u);
}

NXTEST(est_v3_delay_reverb_reverb3d_by_channel) {
    CommandProcessingTimeEstimatorVersion3 a(160, 1);
    CommandProcessingTimeEstimatorVersion3 b(240, 1);

    DelayCommand d;
    InitDelay(d, 4, true);
    CHECK_EQ(a.Estimate(d), 47759u);  // 160, enabled, 4 ch: 47759.62 -> 47759
    InitDelay(d, 2, false);
    CHECK_EQ(a.Estimate(d), 1213u);   // 160, disabled, 2 ch: 1213.60 -> 1213
    InitDelay(d, 6, true);
    CHECK_EQ(b.Estimate(d), 120042u); // 240, enabled, 6 ch: 120042.40 -> 120042
    InitDelay(d, 4, false);
    CHECK_EQ(b.Estimate(d), 792u);    // 240, disabled, 4 ch: 792.30 -> 792

    ReverbCommand r;
    InitReverb(r, 2, true);
    CHECK_EQ(a.Estimate(r), 84975u);  // 160, enabled, 2 ch: 84975.0
    InitReverb(r, 4, false);
    CHECK_EQ(a.Estimate(r), 643u);    // 160, disabled, 4 ch: 643.70 -> 643
    InitReverb(r, 6, false);
    CHECK_EQ(b.Estimate(r), 778u);    // 240, disabled, 6 ch: 778.07 -> 778
    InitReverb(r, 2, true);
    CHECK_EQ(b.Estimate(r), 25262u);  // 240, enabled, 2 ch: 25262.22 -> 25262 (the C# table has this value)

    Reverb3dCommand r3;
    InitReverb3d(r3, 6, true);
    CHECK_EQ(a.Estimate(r3), 165812u);  // 160, enabled, 6 ch: 165812.66 -> 165812
    InitReverb3d(r3, 4, false);
    CHECK_EQ(b.Estimate(r3), 626u);     // 240, disabled, 4 ch: 626.42 -> 626
    InitReverb3d(r3, 1, true);
    CHECK_EQ(b.Estimate(r3), 170292u);  // 240, enabled, 1 ch: 170292.34 -> 170292
}

NXTEST(est_v3_fixed_costs_sinks_and_limiters) {
    CommandProcessingTimeEstimatorVersion3 a(160, 1);
    CommandProcessingTimeEstimatorVersion3 b(240, 1);

    AuxiliaryBufferCommand aux;
    aux.Enabled = true;
    CHECK_EQ(a.Estimate(aux), 7182u);  // 7182.14 -> 7182
    CHECK_EQ(b.Estimate(aux) , 9435u); // 9435.96 -> 9435
    aux.Enabled = false;
    CHECK_EQ(b.Estimate(aux), 462u);   // 462.62 -> 462

    VolumeCommand vol;
    CHECK_EQ(a.Estimate(vol), 1311u);  // 1311.1 -> 1311
    CHECK_EQ(b.Estimate(vol), 1713u);  // 1713.6 -> 1713

    CircularBufferSinkCommand cbs;
    cbs.InputCount = 2;
    // 160: 0 + 531.07 * 2 = 1062.14 -> 1062
    CHECK_EQ(a.Estimate(cbs), 1062u);
    cbs.InputCount = 6;
    // 240: 0 + 770.26 * 6 = 4621.56 -> 4621
    CHECK_EQ(b.Estimate(cbs), 4621u);

    DownMixSurroundToStereoCommand down;
    CHECK_EQ(a.Estimate(down), 9949u);   // 9949.7 -> 9949
    CHECK_EQ(b.Estimate(down), 14679u);  // 14679.0

    UpsampleCommand up;
    CHECK_EQ(a.Estimate(up), 312990u);  // 312990.0
    CHECK_EQ(b.Estimate(up), 0u);

    DeviceSinkCommand dev;
    dev.InputCount = 2;
    CHECK_EQ(a.Estimate(dev), 8980u);   // 160, 2 inputs: 8980.0
    CHECK_EQ(b.Estimate(dev), 9221u);   // 240, 2 inputs: 9221.9 -> 9221
    dev.InputCount = 6;
    CHECK_EQ(b.Estimate(dev), 9725u);   // 240, 6 inputs: 9725.9 -> 9725

    // Limiter version 1 (common path): 160 enabled 4 ch 32405.0; 160 disabled 6 ch 1016.8 -> 1016; 240 enabled 2 ch 39011.0; 240 disabled 1 ch 874.43 -> 874
    LimiterCommandVersion1 l1;
    InitLimiter1(l1, 4, true);
    CHECK_EQ(a.Estimate(l1), 32405u);
    InitLimiter1(l1, 6, false);
    CHECK_EQ(a.Estimate(l1), 1016u);
    InitLimiter1(l1, 2, true);
    CHECK_EQ(b.Estimate(l1), 39011u);
    InitLimiter1(l1, 1, false);
    CHECK_EQ(b.Estimate(l1), 874u);

    // Limiter version 2: statistics on and effect enabled -> statistics table; otherwise the common path.
    LimiterCommandVersion2 l2;
    InitLimiter2(l2, 2, true, true);
    CHECK_EQ(a.Estimate(l2), 29954u);   // 160, statistics, 2 ch: 29954.0
    InitLimiter2(l2, 6, true, true);
    CHECK_EQ(b.Estimate(l2), 85527u);   // 240, statistics, 6 ch: 85527.0
    InitLimiter2(l2, 6, false, true);
    CHECK_EQ(a.Estimate(l2), 52219u);   // 160, no statistics -> common enabled 6 ch: 52219.0
}

NXTEST(est_v3_zero_cost_commands) {
    CommandProcessingTimeEstimatorVersion3 a(160, 1);
    ICommandProcessingTimeEstimator& e = a;
    MultiTapBiquadFilterCommand mt;
    CHECK_EQ(e.Estimate(mt), 0u);
    CaptureBufferCommand cap;
    CHECK_EQ(e.Estimate(cap), 0u);
    CompressorCommand comp;
    CHECK_EQ(e.Estimate(comp), 0u);
    BiquadFilterAndMixCommand bfm;
    CHECK_EQ(e.Estimate(bfm), 0u);
    MultiTapBiquadFilterAndMixCommand mtm;
    CHECK_EQ(e.Estimate(mtm), 0u);
    FillBufferCommand fill;
    CHECK_EQ(e.Estimate(fill), 0u);
    DepopPrepareCommand prep;
    CHECK_EQ(e.Estimate(prep), 0u);
}

// ---------------------------------------------------------------------------------------------------------------------
// Version 4 (derives from version 3; overrides MultiTapBiquadFilter and CaptureBuffer)
// ---------------------------------------------------------------------------------------------------------------------

NXTEST(est_v4_overrides_multitap_and_capture) {
    CommandProcessingTimeEstimatorVersion4 a(160, 1);
    CommandProcessingTimeEstimatorVersion4 b(240, 1);
    ICommandProcessingTimeEstimator& ea = a;
    ICommandProcessingTimeEstimator& eb = b;

    MultiTapBiquadFilterCommand mt;
    CHECK_EQ(ea.Estimate(mt), 7424u);  // 160: 7424.5 -> 7424
    CHECK_EQ(eb.Estimate(mt), 9730u);  // 240: 9730.4 -> 9730

    CaptureBufferCommand cap;
    cap.Enabled = true;
    CHECK_EQ(ea.Estimate(cap), 435u);   // 160, enabled: 435.2 -> 435
    CHECK_EQ(eb.Estimate(cap), 5858u);  // 240, enabled: 5858.26 -> 5858
    cap.Enabled = false;
    CHECK_EQ(ea.Estimate(cap), 4261u);  // 160, disabled: 4261.0
    CHECK_EQ(eb.Estimate(cap), 435u);   // 240, disabled: 435.2 -> 435
}

NXTEST(est_v4_inherits_version3_costs) {
    CommandProcessingTimeEstimatorVersion4 a(160, 1);
    CommandProcessingTimeEstimatorVersion4 b(240, 1);
    ICommandProcessingTimeEstimator& ea = a;
    ICommandProcessingTimeEstimator& eb = b;

    // Delay, Reverb and Reverb3d are the version 3 tables here (version 5 changes them)
    DelayCommand d;
    InitDelay(d, 2, true);
    CHECK_EQ(ea.Estimate(d), 25500u);  // 160, enabled, 2 ch: 25500.75 -> 25500
    InitDelay(d, 1, false);
    CHECK_EQ(eb.Estimate(d), 997u);    // 240, disabled, 1 ch: 997.67 -> 997

    Reverb3dCommand r3;
    InitReverb3d(r3, 1, true);
    CHECK_EQ(ea.Estimate(r3), 116754u);  // 160, enabled, 1 ch: 116754.0
    InitReverb3d(r3, 4, true);
    CHECK_EQ(eb.Estimate(r3), 214696u);  // 240, enabled, 4 ch: 214696.19 -> 214696

    ReverbCommand r;
    InitReverb(r, 4, false);
    CHECK_EQ(eb.Estimate(r), 711u);  // 240, disabled, 4 ch: 711.43 -> 711

    // Data source: PCM16 (version 3 formula) at 160: 6970 (same as the version 3 test)
    PcmInt16DataSourceCommandVersion1 pcm;
    SetPcm(pcm, 48000, 32768.0f);
    CHECK_EQ(ea.Estimate(pcm), 6970u);

    // Data source v2, PCM16 Default at 240: 7853.28 + 710.14 * 0.000013824 = 7853.28 + 0.0098 = 7853.2898 -> 7853
    DataSourceVersion2Command ds;
    SetV2(ds, SampleFormat::PcmInt16, SampleRateConversionQuality::Default, 48000, 32768.0f);
    CHECK_EQ(eb.Estimate(ds), 7853u);

    DeviceSinkCommand dev;
    dev.InputCount = 2;
    CHECK_EQ(eb.Estimate(dev), 9221u);  // 240, 2 inputs: 9221.9

    LimiterCommandVersion2 l2;
    InitLimiter2(l2, 4, true, true);
    CHECK_EQ(eb.Estimate(l2), 52190u);  // 240, statistics, 4 ch: 52190.0

    VolumeCommand vol;
    CHECK_EQ(ea.Estimate(vol), 1311u);  // 160: 1311.1 -> 1311

    // Compressor and the biquad-and-mix commands are still zero in version 4 (version 5 changes them)
    CompressorCommand comp;
    InitCompressor(comp, 2, true, true);
    CHECK_EQ(ea.Estimate(comp), 0u);
    BiquadFilterAndMixCommand bfm;
    bfm.HasVolumeRamp = true;
    CHECK_EQ(eb.Estimate(bfm), 0u);
}

// ---------------------------------------------------------------------------------------------------------------------
// Version 5 (derives from version 4; overrides Delay, Reverb, Reverb3d, Compressor, the biquad-and-mix commands, FillBuffer)
// ---------------------------------------------------------------------------------------------------------------------

NXTEST(est_v5_delay_by_channel_and_sample_count) {
    CommandProcessingTimeEstimatorVersion5 a(160, 1);
    CommandProcessingTimeEstimatorVersion5 b(240, 1);
    ICommandProcessingTimeEstimator& ea = a;
    ICommandProcessingTimeEstimator& eb = b;

    DelayCommand d;
    InitDelay(d, 1, true);
    CHECK_EQ(ea.Estimate(d), 8929u);   // 160, enabled, 1 ch: 8929 (integer arm)
    InitDelay(d, 2, true);
    CHECK_EQ(ea.Estimate(d), 25501u);  // 160, enabled, 2 ch: 25501
    InitDelay(d, 4, true);
    CHECK_EQ(ea.Estimate(d), 47760u);  // 160, enabled, 4 ch: 47760
    InitDelay(d, 6, true);
    CHECK_EQ(ea.Estimate(d), 82203u);  // 160, enabled, 6 ch: 82203
    InitDelay(d, 6, false);
    CHECK_EQ(ea.Estimate(d), 1001u);   // 160, disabled, 6 ch: 1001.6 -> 1001
    InitDelay(d, 4, false);
    CHECK_EQ(ea.Estimate(d), 942u);    // 160, disabled, 4 ch: 942.03 -> 942

    InitDelay(d, 1, true);
    CHECK_EQ(eb.Estimate(d), 11941u);  // 240, enabled, 1 ch: 11941
    InitDelay(d, 4, true);
    CHECK_EQ(eb.Estimate(d), 69750u);  // 240, enabled, 4 ch: 69750
    InitDelay(d, 6, true);
    CHECK_EQ(eb.Estimate(d), 12004u);  // 240, enabled, 6 ch: 12004 (the C# table has this value)
    InitDelay(d, 4, false);
    CHECK_EQ(eb.Estimate(d), 792u);    // 240, disabled, 4 ch: 792.31 -> 792
}

NXTEST(est_v5_reverb_and_reverb3d) {
    CommandProcessingTimeEstimatorVersion5 a(160, 1);
    CommandProcessingTimeEstimatorVersion5 b(240, 1);
    ICommandProcessingTimeEstimator& ea = a;
    ICommandProcessingTimeEstimator& eb = b;

    ReverbCommand r;
    InitReverb(r, 4, true);
    CHECK_EQ(ea.Estimate(r), 91625u);  // 160, enabled, 4 ch: 91625
    InitReverb(r, 2, true);
    CHECK_EQ(eb.Estimate(r), 125260u); // 240, enabled, 2 ch: 125260
    InitReverb(r, 2, false);
    CHECK_EQ(ea.Estimate(r), 588u);    // 160, disabled, 2 ch: 588.80 -> 588
    InitReverb(r, 4, false);
    CHECK_EQ(eb.Estimate(r), 711u);    // 240, disabled, 4 ch: 711.44 -> 711

    Reverb3dCommand r3;
    InitReverb3d(r3, 1, false);
    CHECK_EQ(ea.Estimate(r3), 735u);   // 160, disabled, 1 ch: integer arm 735
    InitReverb3d(r3, 6, true);
    CHECK_EQ(ea.Estimate(r3), 165810u);  // 160, enabled, 6 ch: 165810
    InitReverb3d(r3, 1, true);
    CHECK_EQ(eb.Estimate(r3), 170290u);  // 240, enabled, 1 ch: 170290
    InitReverb3d(r3, 6, false);
    CHECK_EQ(eb.Estimate(r3), 682u);     // 240, disabled, 6 ch: 682.47 -> 682
    InitReverb3d(r3, 4, false);
    CHECK_EQ(ea.Estimate(r3), 834u);     // 160, disabled, 4 ch: 834.07 -> 834
}

NXTEST(est_v5_compressor_statistics_switch) {
    CommandProcessingTimeEstimatorVersion5 a(160, 1);
    CommandProcessingTimeEstimatorVersion5 b(240, 1);
    ICommandProcessingTimeEstimator& ea = a;
    ICommandProcessingTimeEstimator& eb = b;

    CompressorCommand c;
    InitCompressor(c, 2, true, true);
    CHECK_EQ(ea.Estimate(c), 33211u);   // 160, enabled, statistics on, 2 ch
    InitCompressor(c, 6, false, true);
    CHECK_EQ(ea.Estimate(c), 55020u);   // 160, enabled, statistics off, 6 ch
    InitCompressor(c, 4, true, true);
    CHECK_EQ(eb.Estimate(c), 61685u);   // 240, enabled, statistics on, 4 ch
    InitCompressor(c, 1, false, true);
    CHECK_EQ(eb.Estimate(c), 27963u);   // 240, enabled, statistics off, 1 ch
    InitCompressor(c, 6, false, false);
    CHECK_EQ(ea.Estimate(c), 782u);     // 160, disabled, 6 ch: 782.02 -> 782
    InitCompressor(c, 2, true, false);
    CHECK_EQ(eb.Estimate(c), 826u);     // 240, disabled, 2 ch: 826.1 -> 826
}

NXTEST(est_v5_biquad_and_mix_with_and_without_volume_ramp) {
    CommandProcessingTimeEstimatorVersion5 a(160, 1);
    CommandProcessingTimeEstimatorVersion5 b(240, 1);
    ICommandProcessingTimeEstimator& ea = a;
    ICommandProcessingTimeEstimator& eb = b;

    BiquadFilterAndMixCommand bfm;
    bfm.HasVolumeRamp = true;
    CHECK_EQ(ea.Estimate(bfm), 5204u);  // 160, ramp: 5204
    CHECK_EQ(eb.Estimate(bfm), 6683u);  // 240, ramp: 6683
    bfm.HasVolumeRamp = false;
    CHECK_EQ(ea.Estimate(bfm), 3427u);  // 160, no ramp: 3427
    CHECK_EQ(eb.Estimate(bfm), 4752u);  // 240, no ramp: 4752

    MultiTapBiquadFilterAndMixCommand mtm;
    mtm.HasVolumeRamp = true;
    CHECK_EQ(ea.Estimate(mtm), 7939u);   // 160, ramp: 7939
    CHECK_EQ(eb.Estimate(mtm), 10669u);  // 240, ramp: 10669
    mtm.HasVolumeRamp = false;
    CHECK_EQ(ea.Estimate(mtm), 6256u);   // 160, no ramp: 6256
    CHECK_EQ(eb.Estimate(mtm), 8683u);   // 240, no ramp: 8683

    FillBufferCommand fill;
    CHECK_EQ(ea.Estimate(fill), 0u);
}

NXTEST(est_v5_inherits_v4_and_v3_costs) {
    CommandProcessingTimeEstimatorVersion5 a(160, 1);
    CommandProcessingTimeEstimatorVersion5 b(240, 1);
    ICommandProcessingTimeEstimator& ea = a;
    ICommandProcessingTimeEstimator& eb = b;

    MultiTapBiquadFilterCommand mt;
    CHECK_EQ(ea.Estimate(mt), 7424u);  // version 4 override, 160: 7424.5 -> 7424

    CaptureBufferCommand cap;
    cap.Enabled = false;
    CHECK_EQ(ea.Estimate(cap), 4261u);  // version 4 override, 160, disabled: 4261.0

    DeviceSinkCommand dev;
    dev.InputCount = 6;
    CHECK_EQ(ea.Estimate(dev), 9177u);  // version 3 table, 160, 6 inputs: 9177.9 -> 9177

    PcmInt16DataSourceCommandVersion1 pcm;
    SetPcm(pcm, 48000, 32768.0f);
    CHECK_EQ(eb.Estimate(pcm), 8563u);  // version 3 formula, 240: 8563.4388 -> 8563

    CHECK_EQ(eb.Estimate(mt), 9730u);   // version 4 override, 240: 9730.4 -> 9730
}
