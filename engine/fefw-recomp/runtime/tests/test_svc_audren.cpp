// SPDX-License-Identifier: GPL-3.0-or-later
// audren:u at IPC level: GetWorkBufferSize, OpenAudioRenderer, the renderer's getters / events / Start / Stop, the audio device service.
// The sizes come from the hand derivations in test_svc_audren_sys.cpp (Ryujinx AudioRenderSystem.GetWorkBufferSize).
#include <cstring>

#include "../src/services/audren/audren_types.h"
#include "svc_test_util.h"

NXTEST_GROUP(svc_audren)

using namespace nxrt;
using nxsvc::Args;
using nxsvc::Reply;
using nxsvc::Session;

namespace {

constexpr int32_t kRev1 = 0x31564552, kRev10 = 0x3A564552;

audren::AudioRendererConfiguration config(int32_t revision, uint32_t mix, uint32_t submix, uint32_t voices, uint32_t sinks) {
    audren::AudioRendererConfiguration c{};
    c.sample_rate = 48000;
    c.sample_count = 240;
    c.mix_buffer_count = mix;
    c.sub_mix_buffer_count = submix;
    c.voice_count = voices;
    c.sink_count = sinks;
    c.rendering_device = audren::server::AudioRendererRenderingDevice::Dsp;
    c.execution_mode = audren::server::AudioRendererExecutionMode::Auto;
    c.revision = revision;
    return c;
}

Args cfg_args(const audren::AudioRendererConfiguration& c) {
    Args a;
    a.bytes(&c, sizeof c);
    return a;
}

}  // namespace

NXTEST(audren_get_work_buffer_size) {
    nxsvc::Env env;
    Session s = nxsvc::connect("audren:u");
    REQUIRE(s.valid());
    Reply r = s.call(1, cfg_args(config(kRev1, 4, 1, 0, 1)));
    REQUIRE(r.ok());
    CHECK_EQ(r.get<int64_t>(0), 0x9000);   // derivation: test_svc_audren_sys.cpp configuration A
    // an unsupported revision (REV99) is refused with audio 153-2
    r = s.call(1, cfg_args(config(0x5A564552 /* REVZ: newer than the library supports */, 4, 1, 0, 1)));
    CHECK_EQ(r.result, services::res(153, 2));
}

// Switch 2 libraries spell their revision 'R' 'E' 'v' (lower case) and go up to 16 (Eden-NS2 feature_support.h): 'REv@' is accepted and sized
// like REV15. The upper-case spelling still stops at 15 and the lower-case one at 16.
NXTEST(audren_native_revision_16) {
    nxsvc::Env env;
    Session s = nxsvc::connect("audren:u");
    REQUIRE(s.valid());
    constexpr int32_t kRev15 = 0x3F564552, kNativeRev16 = 0x40764552, kRev16 = 0x40564552, kNativeRev17 = 0x41764552;
    Reply r15 = s.call(1, cfg_args(config(kRev15, 4, 1, 2, 1)));
    REQUIRE(r15.ok());
    Reply r = s.call(1, cfg_args(config(kNativeRev16, 4, 1, 2, 1)));
    REQUIRE(r.ok());
    CHECK_EQ(r.get<int64_t>(0), r15.get<int64_t>(0));   // 16 adds no feature over 15
    CHECK_EQ(s.call(1, cfg_args(config(kRev16, 4, 1, 2, 1))).result, services::res(153, 2));
    CHECK_EQ(s.call(1, cfg_args(config(kNativeRev17, 4, 1, 2, 1))).result, services::res(153, 2));
}

NXTEST(audren_open_renderer_and_getters) {
    nxsvc::Env env;
    Session s = nxsvc::connect("audren:u");
    REQUIRE(s.valid());
    const audren::AudioRendererConfiguration c = config(kRev10, 4, 1, 2, 1);
    const int64_t size = s.call(1, cfg_args(c)).get<int64_t>(0);
    REQUIRE(size > 0);
    Args a = cfg_args(c);
    a.align(8).u64(static_cast<uint64_t>(size)).u64(0x51).hcopy(0xFFFF8001);
    Reply r = s.call(0, a);
    REQUIRE(r.ok());
    Session rend = s.sub(r);
    REQUIRE(rend.valid());
    CHECK_EQ(rend.call(0).get<int32_t>(0), 48000);   // GetSampleRate
    CHECK_EQ(rend.call(1).get<int32_t>(0), 240);     // GetSampleCount
    CHECK_EQ(rend.call(2).get<int32_t>(0), 4);       // GetMixBufferCount
    CHECK_EQ(rend.call(3).get<int32_t>(0), 1);       // GetState: not started
    CHECK_EQ(rend.call(8, Args().u32(80)).result, 0u);   // SetRenderingTimeLimit
    CHECK_EQ(rend.call(9).get<int32_t>(0), 80);
    // QuerySystemEvent: a readable event that fires every 5 ms frame once the renderer runs
    Reply ev = rend.call(7);
    REQUIRE(ev.ok());
    REQUIRE(ev.copy.size() == 1);
    CHECK_EQ(rend.call(5).result, 0u);               // Start
    CHECK_EQ(rend.call(3).get<int32_t>(0), 0);       // GetState: active
    int32_t idx = -1;
    uint32_t h = ev.copy[0];
    CHECK_EQ(kernel::wait_synchronization(&h, 1, 500000000, &idx), 0u);   // fires within 500 ms
    CHECK_EQ(rend.call(6).result, 0u);               // Stop
    kernel::handle_close(h);
}

NXTEST(audren_request_update_rejects_garbage) {
    nxsvc::Env env;
    Session s = nxsvc::connect("audren:u");
    const audren::AudioRendererConfiguration c = config(kRev10, 4, 1, 2, 1);
    const int64_t size = s.call(1, cfg_args(c)).get<int64_t>(0);
    Args a = cfg_args(c);
    a.align(8).u64(static_cast<uint64_t>(size)).u64(0x51).hcopy(0xFFFF8001);
    Session rend = s.sub(s.call(0, a));
    REQUIRE(rend.valid());
    std::vector<uint8_t> in(0x80, 0xFF), out(0x400, 0), perf(0x100, 0);
    Reply r = rend.call(4, Args().buf_b(out.data(), out.size()).buf_b(perf.data(), perf.size()).buf_a(in.data(), in.size()));
    CHECK(r.result != 0);   // InvalidUpdateInfo (153-41) or the revision error, never success
}

NXTEST(audren_open_refused_for_too_small_work_buffer) {
    nxsvc::Env env;
    Session s = nxsvc::connect("audren:u");
    audren::AudioRendererConfiguration c = config(kRev10, 4, 1, 2, 1);
    Args a = cfg_args(c);
    a.align(8).u64(0x10).u64(0x51).hcopy(0xFFFF8001);   // 16 bytes of work buffer
    Reply r = s.call(0, a);
    CHECK_EQ(r.result, services::res(153, 4));          // WorkBufferTooSmall
}

NXTEST(audren_audio_device_service) {
    nxsvc::Env env;
    Session s = nxsvc::connect("audren:u");
    Reply r = s.call(2, Args().u64(0x51));           // GetAudioDeviceService
    REQUIRE(r.ok());
    Session dev = s.sub(r);
    REQUIRE(dev.valid());
    std::vector<char> names(0x100 * 8, 0);
    r = dev.call(0, Args().buf_b(names.data(), names.size()));   // ListAudioDeviceName
    REQUIRE(r.ok());
    CHECK_EQ(r.get<int32_t>(0), 3);
    CHECK_EQ(std::string(names.data()), std::string("AudioStereoJackOutput"));
    CHECK_EQ(std::string(names.data() + 0x100), std::string("AudioBuiltInSpeakerOutput"));
    CHECK_EQ(std::string(names.data() + 0x200), std::string("AudioTvOutput"));
    std::vector<char> active(0x100, 0);
    CHECK(dev.call(3, Args().buf_b(active.data(), active.size())).ok());   // GetActiveAudioDeviceName: docked = TV
    CHECK_EQ(std::string(active.data()), std::string("AudioTvOutput"));
    CHECK_EQ(dev.call(5).get<int32_t>(0), 2);        // GetActiveChannelCount
    // volume round trip
    std::vector<char> name(0x100, 0);
    std::strcpy(name.data(), "AudioTvOutput");
    CHECK(dev.call(1, Args().f32(0.5f).buf_a(name.data(), name.size())).ok());
    CHECK_EQ(dev.call(2, Args().buf_a(name.data(), name.size())).get<float>(0), 0.5f);
    // the three events exist and are not signalled
    for (uint32_t cmd : {4u, 11u, 12u}) {
        Reply e = dev.call(cmd);
        REQUIRE(e.copy.size() == 1);
        int32_t idx;
        uint32_t h = e.copy[0];
        CHECK_EQ(kernel::wait_synchronization(&h, 1, 0, &idx), kernel::kTimedOut);
        kernel::handle_close(h);
    }
    // a revision aware open
    Reply r2 = s.call(4, Args().u64(0x51).u32(0x3A564552));
    CHECK(r2.ok());
    Session dev2 = s.sub(r2);
    CHECK(dev2.valid());
}

// 17.0.0+ device notifications, ids as the SDK's proxy (methods 0-8, 10-21 in order): 15/16 input, 17/18 output. The u64 id is the SDK's
// SystemEvent address; each id gets an event that is not signalled (no host device change). 19/20: the auto-tune flag round trip.
NXTEST(audren_audio_device_notifications_and_auto_tune) {
    nxsvc::Env env;
    Session s = nxsvc::connect("audren:u");
    Reply r = s.call(2, Args().u64(0x51));
    REQUIRE(r.ok());
    Session dev = s.sub(r);
    REQUIRE(dev.valid());
    for (uint32_t cmd : {15u, 17u}) {
        Reply e = dev.call(cmd, Args().u64(0x7E000FFB00ull));
        REQUIRE(e.ok());
        REQUIRE(e.copy.size() == 1);
        int32_t idx;
        uint32_t h = e.copy[0];
        CHECK_EQ(kernel::wait_synchronization(&h, 1, 0, &idx), kernel::kTimedOut);
        kernel::handle_close(h);
        CHECK(dev.call(cmd + 1, Args().u64(0x7E000FFB00ull)).ok());   // Release
    }
    CHECK(dev.call(19, Args().u8(1)).ok());
    CHECK_EQ(dev.call(20).get<uint8_t>(0), 1u);
    CHECK(dev.call(19, Args().u8(0)).ok());
    CHECK_EQ(dev.call(20).get<uint8_t>(0), 0u);
    CHECK_EQ(dev.call(21).get<uint8_t>(0), 0u);
}
