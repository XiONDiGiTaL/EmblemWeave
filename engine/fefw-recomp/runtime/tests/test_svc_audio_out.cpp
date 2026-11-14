// SPDX-License-Identifier: GPL-3.0-or-later
// audout:u and audin:u (S3c). Request bytes and expected reply bytes are written out from Switchbrew's Audio_services page
// (command ids, AudioOutParameter / AudioOutParameterInternal / AudioOutBuffer layouts, result 153-2 = 0x499, 153-1 = 0x299).
#include "svc_test_util.h"

#include <cstring>
#include <memory>
#include <vector>

#include "nxrt/audio_sink.h"

NXTEST_GROUP(svc_audio_out)

using namespace nxrt;

namespace {

// Switchbrew AudioOutBuffer (AudioInBuffer has the same layout): 0x28 bytes.
struct OutBuffer {
    uint64_t next;      // 0x00
    uint64_t buffer;    // 0x08 sample pointer
    uint64_t capacity;  // 0x10
    uint64_t size;      // 0x18 bytes of data
    uint64_t offset;    // 0x20
};
static_assert(sizeof(OutBuffer) == 0x28, "AudioOutBuffer is 0x28 bytes");

constexpr uint32_t kResOperationFailed = 0x499;   // Switchbrew error codes: module 153, description 2
constexpr uint32_t kRate48k = 48000;              // 0x0000BB80
constexpr uint32_t kRate44k1 = 44100;             // 0x0000AC44

// Four-byte buffers of the samples a test plays: 480 frames of stereo PcmInt16 = 1920 bytes (10 ms at 48 kHz).
constexpr uint64_t kFrames10ms = 480;
constexpr uint64_t kBytes10ms = kFrames10ms * 2 * 2;

nxrt::audio::NullSink* use_null_sink() {
    auto s = std::make_shared<nxrt::audio::NullSink>();
    nxrt::audio::NullSink* raw = s.get();
    nxrt::audio::set_sink(s);
    return raw;
}

// Waits for the buffer event (at most `ms`). True when it was signaled.
bool wait_event(uint32_t h, int64_t ms) {
    int32_t idx = -1;
    return nxrt::kernel::wait_synchronization(&h, 1, ms * 1000000, &idx) == 0;
}

// Calls GetReleased (cmd 5 for an out, cmd 5 for an in) until `want` tags are collected, waiting at most 100 ms per round, 3 rounds.
std::vector<uint64_t> collect_released(nxsvc::Session& s, uint32_t cmd, uint32_t event, size_t want) {
    std::vector<uint64_t> got;
    for (int round = 0; round < 3 && got.size() < want; round++) {
        if (!wait_event(event, 100)) continue;
        uint64_t tags[6] = {};   // array<u64, 6> in Switchbrew's signature
        nxsvc::Reply r = s.call(cmd, nxsvc::Args().buf_b(tags, sizeof tags));
        CHECK(r.ok());
        const uint32_t n = r.get<uint32_t>(0);
        for (uint32_t k = 0; k < n && k < 6; k++) got.push_back(tags[k]);
    }
    return got;
}

// Opens audout:u's DeviceOut at the given rate and channel count; the IAudioOut session in `out`.
nxsvc::Reply open_out(nxsvc::Session& audout, uint32_t rate, uint16_t ch, nxsvc::Session& out) {
    char name_in[0x100] = {};
    std::memcpy(name_in, "DeviceOut", 9);
    char name_out[0x100] = {};
    nxrt::services::ServiceEvent proc;       // stands in for the process handle that the request copies
    const uint32_t ph = proc.new_handle();
    // AudioOutParameter {u32 SampleRate, u16 ChannelCount, u16 Reserved}, then AppletResourceUserId (u64).
    nxsvc::Reply r = audout.call(1, nxsvc::Args().with_pid().u32(rate).u16(ch).u16(0).u64(0).hcopy(ph)
                                        .buf_a(name_in, sizeof name_in).buf_b(name_out, sizeof name_out));
    nxrt::kernel::handle_close(ph);
    if (r.ok() && r.move.size() == 1) out = audout.sub(r);
    return r;
}

}  // namespace

NXTEST(audout_list_and_open_reply_layout) {
    nxsvc::Env env;
    nxsvc::Session audout = nxsvc::connect("audout:u");
    REQUIRE(audout.valid());

    // ListAudioOuts: count, and one AudioOutInfo name (0x100 bytes, "DeviceOut" zero padded).
    uint8_t names[0x100 * 6] = {};
    nxsvc::Reply l = audout.call(0, nxsvc::Args().buf_b(names, sizeof names));
    CHECK(l.ok());
    CHECK_EQ(l.get<uint32_t>(0), 1u);
    const uint8_t devout[9] = {0x44, 0x65, 0x76, 0x69, 0x63, 0x65, 0x4F, 0x75, 0x74};   // "DeviceOut"
    CHECK_EQ(std::memcmp(names, devout, sizeof devout), 0);
    CHECK_EQ(names[9], 0);

    // ListAudioOutsAuto (cmd 2) answers the same.
    uint8_t names_auto[0x100 * 6] = {};
    nxsvc::Reply la = audout.call(2, nxsvc::Args().buf_b(names_auto, sizeof names_auto));
    CHECK(la.ok());
    CHECK_EQ(la.get<uint32_t>(0), 1u);

    // OpenAudioOut(48000 Hz, 2 channels): AudioOutParameterInternal {rate 48000, channels 2, PcmInt16 = 2, Stopped = 1}, IAudioOut.
    nxsvc::Session out;
    nxsvc::Reply o = open_out(audout, kRate48k, 2, out);
    CHECK(o.ok());
    CHECK(o.out.size() >= 16u);   // the raw reply is padded to 16 bytes; the 16 bytes of AudioOutParameterInternal come first
    CHECK_EQ(o.get<uint32_t>(0), 48000u);
    CHECK_EQ(o.get<uint32_t>(4), 2u);
    CHECK_EQ(o.get<uint32_t>(8), 2u);
    CHECK_EQ(o.get<uint32_t>(12), 1u);
    REQUIRE(out.valid());
    nxsvc::Reply st = out.call(0);
    CHECK(st.ok());
    CHECK_EQ(st.get<uint32_t>(0), 1u);   // Stopped
}

NXTEST(audout_open_honours_valid_formats) {
    nxsvc::Env env;
    nxsvc::Session audout = nxsvc::connect("audout:u");
    REQUIRE(audout.valid());
    // 44100 Hz and 6 channels are honoured (0x0000AC44, channels 6).
    nxsvc::Session out;
    nxsvc::Reply a = open_out(audout, kRate44k1, 6, out);
    CHECK(a.ok());
    CHECK_EQ(a.get<uint32_t>(0), 44100u);
    CHECK_EQ(a.get<uint32_t>(4), 6u);
    // 4 channels is not one the sink plays: stereo. A rate of 0 is the default 48 kHz.
    nxsvc::Session out2;
    nxsvc::Reply b = open_out(audout, 0, 4, out2);
    CHECK(b.ok());
    CHECK_EQ(b.get<uint32_t>(0), 48000u);
    CHECK_EQ(b.get<uint32_t>(4), 2u);
}

NXTEST(audout_open_rejects_other_device) {
    nxsvc::Env env;
    nxsvc::Session audout = nxsvc::connect("audout:u");
    REQUIRE(audout.valid());
    char name_in[0x100] = {};
    std::memcpy(name_in, "NoSuchDevice", 12);
    nxrt::services::ServiceEvent proc;
    const uint32_t ph = proc.new_handle();
    char name_out[0x100] = {};
    nxsvc::Reply r = audout.call(1, nxsvc::Args().with_pid().u32(kRate48k).u16(2).u16(0).u64(0).hcopy(ph)
                                        .buf_a(name_in, sizeof name_in).buf_b(name_out, sizeof name_out));
    nxrt::kernel::handle_close(ph);
    CHECK_EQ(r.result, 0x299u);   // ResultNotFound, module 153 description 1
}

NXTEST(audout_start_stop_state) {
    nxsvc::Env env;
    nxsvc::Session audout = nxsvc::connect("audout:u");
    REQUIRE(audout.valid());
    nxsvc::Session out;
    REQUIRE(open_out(audout, kRate48k, 2, out).ok());
    CHECK_EQ(out.call(0).get<uint32_t>(0), 1u);          // Stopped
    CHECK(out.call(1).ok());                              // Start
    CHECK_EQ(out.call(0).get<uint32_t>(0), 0u);          // Started
    nxsvc::Reply again = out.call(1);                     // a second Start is refused
    CHECK_EQ(again.result, kResOperationFailed);
    CHECK(out.call(2).ok());                              // Stop
    CHECK_EQ(out.call(0).get<uint32_t>(0), 1u);
    CHECK(out.call(2).ok());                              // Stop when stopped is accepted
}

NXTEST(audout_play_two_buffers_release_and_count) {
    nxsvc::Env env;
    nxrt::audio::NullSink* sink = use_null_sink();
    nxsvc::Session audout = nxsvc::connect("audout:u");
    REQUIRE(audout.valid());
    nxsvc::Session out;
    REQUIRE(open_out(audout, kRate48k, 2, out).ok());
    nxsvc::Reply ev = out.call(4);                        // RegisterBufferEvent
    REQUIRE(ev.ok());
    REQUIRE(ev.copy.size() == 1);
    const uint32_t event = ev.copy[0];

    nxsvc::Reply start = out.call(1);
    REQUIRE(start.ok());

    static int16_t samples_a[kFrames10ms * 2];
    static int16_t samples_b[kFrames10ms * 2];
    OutBuffer a{0, reinterpret_cast<uint64_t>(samples_a), sizeof samples_a, kBytes10ms, 0};
    OutBuffer b{0, reinterpret_cast<uint64_t>(samples_b), sizeof samples_b, kBytes10ms, 0};
    CHECK(out.call(3, nxsvc::Args().u64(0x1001).buf_a(&a, sizeof a)).ok());   // AppendAudioOutBuffer
    CHECK(out.call(3, nxsvc::Args().u64(0x1002).buf_a(&b, sizeof b)).ok());

    // Each buffer is released after 10 ms of host time, in order.
    std::vector<uint64_t> got = collect_released(out, 5, event, 2);
    REQUIRE(got.size() == 2);
    CHECK_EQ(got[0], 0x1001ull);
    CHECK_EQ(got[1], 0x1002ull);

    CHECK_EQ(out.call(9).get<uint32_t>(0), 0u);          // GetAudioOutBufferCount: nothing queued
    CHECK_EQ(out.call(10).get<uint64_t>(0), 960ull);     // GetAudioOutPlayedSampleCount: 2 x 480 frames
    CHECK_EQ(out.call(6, nxsvc::Args().u64(0x1001)).get<uint8_t>(0), 0u);   // taken back: no longer contained
    CHECK(!wait_event(event, 0));                         // the event clears once every released tag is taken

    // The sink received the 960 frames, opened stereo at 48 kHz on the first start.
    CHECK_EQ(sink->total_frames, 960ull);
    CHECK_EQ(sink->rate, 48000u);
    CHECK_EQ(sink->channels, 2u);
    CHECK(out.call(2).ok());
}

NXTEST(audout_flush_and_stop_release_unplayed) {
    nxsvc::Env env;
    use_null_sink();
    nxsvc::Session audout = nxsvc::connect("audout:u");
    REQUIRE(audout.valid());
    nxsvc::Session out;
    REQUIRE(open_out(audout, kRate48k, 2, out).ok());
    nxsvc::Reply ev = out.call(4);
    REQUIRE(ev.ok() && ev.copy.size() == 1);
    REQUIRE(out.call(1).ok());

    // One-second buffers (48000 frames, 192000 bytes) do not finish inside the test.
    static std::vector<int16_t> sec(48000 * 2);
    OutBuffer big{0, reinterpret_cast<uint64_t>(sec.data()), sec.size() * 2, sec.size() * 2, 0};
    CHECK(out.call(3, nxsvc::Args().u64(0x3001).buf_a(&big, sizeof big)).ok());
    CHECK(out.call(3, nxsvc::Args().u64(0x3002).buf_a(&big, sizeof big)).ok());
    CHECK_EQ(out.call(9).get<uint32_t>(0), 2u);
    CHECK_EQ(out.call(11).get<uint8_t>(0), 1u);          // FlushAudioOutBuffers: started, so flushed
    uint64_t tags[6] = {};
    nxsvc::Reply r = out.call(5, nxsvc::Args().buf_b(tags, sizeof tags));
    CHECK(r.ok());
    CHECK_EQ(r.get<uint32_t>(0), 2u);
    CHECK_EQ(tags[0], 0x3001ull);
    CHECK_EQ(tags[1], 0x3002ull);
    CHECK_EQ(tags[2], 0ull);                              // unused slots are zero
    CHECK_EQ(out.call(10).get<uint64_t>(0), 0ull);        // nothing was played

    // Stop releases what is still queued.
    CHECK(out.call(3, nxsvc::Args().u64(0x3003).buf_a(&big, sizeof big)).ok());
    CHECK(out.call(2).ok());
    uint64_t tags2[6] = {};
    nxsvc::Reply r2 = out.call(5, nxsvc::Args().buf_b(tags2, sizeof tags2));
    CHECK_EQ(r2.get<uint32_t>(0), 1u);
    CHECK_EQ(tags2[0], 0x3003ull);
    CHECK_EQ(out.call(9).get<uint32_t>(0), 0u);
    CHECK(!wait_event(ev.copy[0], 0));
}

NXTEST(audout_append_rejects_unreadable_descriptor) {
    nxsvc::Env env;
    nxsvc::Session audout = nxsvc::connect("audout:u");
    REQUIRE(audout.valid());
    nxsvc::Session out;
    REQUIRE(open_out(audout, kRate48k, 2, out).ok());
    // The descriptor address 0x1000 is not mapped: refused before anything is read.
    nxsvc::Reply r = out.call(3, nxsvc::Args().u64(0x4001).buf_a(reinterpret_cast<void*>(0x1000), 0x28));
    CHECK_EQ(r.result, kResOperationFailed);
    CHECK_EQ(out.call(9).get<uint32_t>(0), 0u);
}

NXTEST(audout_volume_defaults_and_set) {
    nxsvc::Env env;
    nxsvc::Session audout = nxsvc::connect("audout:u");
    REQUIRE(audout.valid());
    nxsvc::Session out;
    REQUIRE(open_out(audout, kRate48k, 2, out).ok());
    CHECK_EQ(out.call(13).get<uint32_t>(0), 0x3F800000u);   // 1.0f by default
    CHECK(out.call(12, nxsvc::Args().f32(0.25f)).ok());     // SetAudioOutVolume(0.25)
    CHECK_EQ(out.call(13).get<uint32_t>(0), 0x3E800000u);   // 0.25f
    float v = out.call(13).get<float>(0);
    CHECK_EQ(v, 0.25f);
}

NXTEST(audout_session_close_joins_thread) {
    nxsvc::Env env;
    use_null_sink();
    nxsvc::Session audout = nxsvc::connect("audout:u");
    REQUIRE(audout.valid());
    {
        nxsvc::Session out;
        REQUIRE(open_out(audout, kRate48k, 2, out).ok());
        REQUIRE(out.call(1).ok());
        static std::vector<int16_t> sec(48000 * 2);
        OutBuffer big{0, reinterpret_cast<uint64_t>(sec.data()), sec.size() * 2, sec.size() * 2, 0};
        CHECK(out.call(3, nxsvc::Args().u64(0x5001).buf_a(&big, sizeof big)).ok());
        out.close();   // last reference: the pacing thread is stopped and joined before close returns
    }
    nxrt::services::reset_all();   // calls audio_out_reset(): idempotent with nothing running
    nxrt::services::reset_all();
}

NXTEST(audin_list_open_and_silent_capture) {
    nxsvc::Env env;
    nxsvc::Session audin = nxsvc::connect("audin:u");
    REQUIRE(audin.valid());

    // ListAudioIns: one AudioInInfo name, "BuiltInHeadset".
    char names[0x100] = {};
    nxsvc::Reply l = audin.call(0, nxsvc::Args().buf_b(names, sizeof names));
    CHECK(l.ok());
    CHECK_EQ(l.get<uint32_t>(0), 1u);
    CHECK_EQ(std::strcmp(names, "BuiltInHeadset"), 0);

    // OpenAudioIn(48000, 2): AudioInParameterInternal {48000, 2, 2, 1}, IAudioIn.
    char name_in[0x100] = {};
    nxrt::services::ServiceEvent proc;
    const uint32_t ph = proc.new_handle();
    char name_out[0x100] = {};
    nxsvc::Reply o = audin.call(1, nxsvc::Args().with_pid().u32(kRate48k).u16(2).u16(0).u64(0).hcopy(ph)
                                       .buf_a(name_in, sizeof name_in).buf_b(name_out, sizeof name_out));
    nxrt::kernel::handle_close(ph);
    CHECK(o.ok());
    CHECK_EQ(o.get<uint32_t>(0), 48000u);
    CHECK_EQ(o.get<uint32_t>(4), 2u);
    CHECK_EQ(o.get<uint32_t>(8), 2u);
    CHECK_EQ(o.get<uint32_t>(12), 1u);
    REQUIRE(o.move.size() == 1);
    nxsvc::Session in = audin.sub(o);

    nxsvc::Reply ev = in.call(4);
    REQUIRE(ev.ok() && ev.copy.size() == 1);
    REQUIRE(in.call(1).ok());   // Start

    // A capture buffer full of 0xAB comes back zeroed, released after 10 ms.
    static uint8_t capture[kBytes10ms];
    std::memset(capture, 0xAB, sizeof capture);
    OutBuffer cap{0, reinterpret_cast<uint64_t>(capture), sizeof capture, kBytes10ms, 0};
    CHECK(in.call(3, nxsvc::Args().u64(0x6001).buf_a(&cap, sizeof cap)).ok());   // AppendAudioInBuffer
    std::vector<uint64_t> got = collect_released(in, 5, ev.copy[0], 1);
    REQUIRE(got.size() == 1);
    CHECK_EQ(got[0], 0x6001ull);
    bool all_zero = true;
    for (uint8_t c : capture)
        if (c != 0) all_zero = false;
    CHECK(all_zero);
    CHECK(in.call(2).ok());   // Stop
    CHECK_EQ(in.call(0).get<uint32_t>(0), 1u);
}
