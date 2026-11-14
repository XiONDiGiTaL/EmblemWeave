// SPDX-License-Identifier: GPL-3.0-or-later
// time:u / time:a / time:s / time:r (S3c). Request bytes and expected reply bytes are written out by hand from the Switchbrew
// service list and the Ryujinx / ScarletNX command tables (time.swipc, TimeSharedMemory.cs, ISystemClock.cs, ISteadyClock.cs,
// ITimeZoneServiceForGlue.cs, IStaticServiceForGlue.cs, TimePermissions.cs), never from reading the implementation.
// Result codes are module 116 (time) with the description shifted left by 9 bits: PermissionDenied (1) = 0x274, OutOfRange (902) =
// 0x70C74, TimeZoneConversionFailed (903) = 0x70E74, TimeZoneNotFound (989) = 0x7BA74.
// Tests whose name ends in `_needs_tz` depend on src/services/timezone.cpp (tz::to_calendar / tz::to_posix); on a branch where that
// file is still the placeholder they fail for that reason only.
#include <chrono>
#include <cstring>
#include <vector>

#include "../src/services/timezone.h"
#include "nxrt/config.h"
#include "nxrt/services.h"
#include "svc_test_util.h"

NXTEST_GROUP(svc_time)

namespace tz = nxrt::services::tz;

using namespace nxrt;

namespace {

constexpr uint32_t kPermDenied = 0x274;
constexpr uint32_t kOutOfRange = 0x70C74;
constexpr uint32_t kTzConversionFailed = 0x70E74;
constexpr uint32_t kTzNotFound = 0x7BA74;
constexpr int64_t kEpochUnix2000 = 946684800;        // 2000-01-01T00:00:00Z
constexpr int64_t kSteadyEpochNs = 3600000000000ll;  // 3600 s, the start of the steady clock
constexpr uint8_t kSourceId[16] = {'n', 'x', 'r', 't', '-', 's', 't', 'e', 'a', 'd', 'y', '-', 'c', 'l', 'k', '1'};

int64_t host_unix() {
    return std::chrono::duration_cast<std::chrono::seconds>(std::chrono::system_clock::now().time_since_epoch()).count();
}
int64_t abs64(int64_t v) { return v < 0 ? -v : v; }

// Ryujinx TimeSharedMemory.ReadObjectFromSharedMemory: live slot = off + 4 + pad + (index & 1) * size.
const uint8_t* live_slot(const uint8_t* base, uint32_t off, uint32_t pad, uint32_t size) {
    uint32_t idx = 0;
    std::memcpy(&idx, base + off, 4);
    return base + off + 4 + pad + (idx & 1) * size;
}

// The shared memory object behind GetSharedMemoryNativeHandle (cmd 20 on IStaticService).
const uint8_t* shm_of(nxsvc::Session& st, uint64_t* size = nullptr) {
    nxsvc::Reply r = st.call(20);
    if (!r.ok() || r.copy.size() != 1) return nullptr;
    auto obj = kernel::handle_as<kernel::KSharedMemory>(r.copy[0]);
    if (!obj) return nullptr;
    if (size) *size = obj->size;
    return reinterpret_cast<const uint8_t*>(static_cast<uintptr_t>(obj->host));
}

// Clears a zone override made by a test, so the next test starts from the default config.
struct ConfigGuard {
    ~ConfigGuard() { config_clear(); }
};

}  // namespace

NXTEST(svc_time_flavours_connect) {
    nxsvc::Env env;
    CHECK(nxsvc::connect("time:u").valid());
    CHECK(nxsvc::connect("time:a").valid());
    CHECK(nxsvc::connect("time:s").valid());
    CHECK(nxsvc::connect("time:r").valid());
}

NXTEST(svc_time_user_clock_get_current_time) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:u");
    nxsvc::Reply r = st.call(0);                      // IStaticService::GetStandardUserSystemClock
    REQUIRE(r.ok());
    nxsvc::Session clock = st.sub(r);
    REQUIRE(clock.valid());
    nxsvc::Reply t = clock.call(0);                   // ISystemClock::GetCurrentTime -> PosixTime
    REQUIRE(t.ok());
    CHECK(abs64(t.get<int64_t>(0) - host_unix()) <= 3);
}

NXTEST(svc_time_user_clock_set_denied_on_time_u) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:u");
    nxsvc::Session clock = st.sub(st.call(0));
    nxsvc::Reply r = clock.call(1, nxsvc::Args().i64(1700000000));   // SetCurrentTime
    CHECK_EQ(r.svc_rc, 0u);
    CHECK_EQ(r.result, kPermDenied);
}

NXTEST(svc_time_admin_set_current_time_then_get) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:a");
    nxsvc::Session clock = st.sub(st.call(0));
    REQUIRE(clock.valid());
    nxsvc::Reply s = clock.call(1, nxsvc::Args().i64(1700000000));
    REQUIRE(s.ok());
    nxsvc::Reply g = clock.call(0);
    REQUIRE(g.ok());
    CHECK(abs64(g.get<int64_t>(0) - 1700000000) <= 3);
    // The same offset is seen by the local clock (the user clock shares it) and not by the network clock.
    nxsvc::Session local = st.sub(st.call(4));
    CHECK(abs64(local.call(0).get<int64_t>(0) - 1700000000) <= 3);
    nxsvc::Session network = st.sub(st.call(1));
    CHECK(abs64(network.call(0).get<int64_t>(0) - host_unix()) <= 3);
}

NXTEST(svc_time_network_clock_on_time_s) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:s");
    nxsvc::Session user = st.sub(st.call(0));
    nxsvc::Session network = st.sub(st.call(1));
    REQUIRE(user.valid() && network.valid());
    // time:s may write the network clock only.
    CHECK_EQ(user.call(1, nxsvc::Args().i64(1700000000)).result, kPermDenied);
    CHECK_EQ(network.call(1, nxsvc::Args().i64(1700000000)).result, 0u);
    CHECK(abs64(network.call(0).get<int64_t>(0) - 1700000000) <= 3);
    CHECK(abs64(user.call(0).get<int64_t>(0) - host_unix()) <= 3);
}

NXTEST(svc_time_system_clock_context) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:u");
    nxsvc::Session clock = st.sub(st.call(0));
    nxsvc::Reply r = clock.call(2);                   // GetSystemClockContext -> {i64 offset, {i64 time_point, uuid}}
    REQUIRE(r.ok());
    REQUIRE(r.out.size() >= 0x20);
    const int64_t offset = r.get<int64_t>(0);
    const int64_t time_point = r.get<int64_t>(8);
    CHECK(time_point >= 3600);                        // steady clock starts at the 3600 s epoch
    CHECK(abs64(offset + time_point - host_unix()) <= 3);   // posix = steady + offset
    CHECK(std::memcmp(r.out.data() + 16, kSourceId, 16) == 0);
}

NXTEST(svc_time_steady_clock_get_current_time_monotonic) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:u");
    nxsvc::Session steady = st.sub(st.call(2));       // GetStandardSteadyClock
    REQUIRE(steady.valid());
    nxsvc::Reply first = steady.call(0);              // ISteadyClock::GetCurrentTimePoint -> {i64, uuid}
    REQUIRE(first.ok());
    REQUIRE(first.out.size() >= 0x18);
    CHECK(first.get<int64_t>(0) >= 3600);
    CHECK(std::memcmp(first.out.data() + 8, kSourceId, 16) == 0);
    int64_t prev = first.get<int64_t>(0);
    bool monotonic = true;
    for (int i = 0; i < 1000; ++i) {
        nxsvc::Reply r = steady.call(0);
        REQUIRE(r.ok());
        if (r.get<int64_t>(0) < prev) monotonic = false;
        prev = r.get<int64_t>(0);
    }
    CHECK(monotonic);
}

NXTEST(svc_time_steady_test_offset_on_time_r) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:r");
    nxsvc::Session steady = st.sub(st.call(2));
    REQUIRE(steady.valid());
    const int64_t before = steady.call(0).get<int64_t>(0);
    // SetTestOffset(TimeSpanType) takes nanoseconds: 1000 s = 1000 * 1e9 ns.
    CHECK_EQ(steady.call(3, nxsvc::Args().i64(1000000000000ll)).result, 0u);
    nxsvc::Reply g = steady.call(2);                  // GetTestOffset
    REQUIRE(g.ok());
    CHECK_EQ(g.get<int64_t>(0), 1000000000000ll);
    const int64_t after = steady.call(0).get<int64_t>(0);
    CHECK(after - before >= 1000 && after - before <= 1002);
}

NXTEST(svc_time_steady_test_offset_denied_on_time_u) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:u");
    nxsvc::Session steady = st.sub(st.call(2));
    REQUIRE(steady.valid());
    CHECK_EQ(steady.call(3, nxsvc::Args().i64(1000000000000ll)).result, kPermDenied);
    CHECK_EQ(steady.call(2).get<int64_t>(0), 0);      // unchanged
}

NXTEST(svc_time_steady_rtc_flags_and_setup) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:u");
    nxsvc::Session steady = st.sub(st.call(2));
    REQUIRE(steady.valid());
    nxsvc::Reply rtc = steady.call(100);              // GetRtcValue -> u64 seconds since 2000-01-01
    REQUIRE(rtc.ok());
    CHECK(abs64(static_cast<int64_t>(rtc.get<uint64_t>(0)) - (host_unix() - kEpochUnix2000)) <= 3);
    nxsvc::Reply reset = steady.call(101);            // IsRtcResetDetected -> bool
    REQUIRE(reset.ok());
    CHECK_EQ(reset.get<uint8_t>(0), 0);
    nxsvc::Reply setup = steady.call(102);            // GetSetupResultValue -> u32
    REQUIRE(setup.ok());
    CHECK_EQ(setup.get<uint32_t>(0), 0u);
    nxsvc::Reply internal = steady.call(200);         // GetInternalOffset -> TimeSpanType
    REQUIRE(internal.ok());
    CHECK_EQ(internal.get<int64_t>(0), 0);
}

NXTEST(svc_time_static_internal_offset) {
    nxsvc::Env env;
    nxsvc::Session user = nxsvc::connect("time:u");
    nxsvc::Session repair = nxsvc::connect("time:r");
    // SetStandardSteadyClockInternalOffset (cmd 50): time:u refused, time:r stores 5 s.
    CHECK_EQ(user.call(50, nxsvc::Args().i64(5000000000ll)).result, kPermDenied);
    CHECK_EQ(repair.call(50, nxsvc::Args().i64(5000000000ll)).result, 0u);
    nxsvc::Session steady = repair.sub(repair.call(2));
    CHECK_EQ(steady.call(200).get<int64_t>(0), 5000000000ll);
}

NXTEST(svc_time_shared_memory_layout) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:u");
    uint64_t size = 0;
    const uint8_t* base = shm_of(st, &size);
    REQUIRE(base != nullptr);
    CHECK_EQ(size, 0x1000u);

    // Steady clock context, pad 4 at 0x00: internal offset = 3600 s epoch (no offsets set), source uuid.
    const uint8_t* steady = live_slot(base, 0x00, 4, 0x18);
    int64_t internal = 0;
    std::memcpy(&internal, steady, 8);
    CHECK_EQ(internal, kSteadyEpochNs);
    CHECK(std::memcmp(steady + 8, kSourceId, 16) == 0);

    // Local and network system clock contexts, pad 4 at 0x38 and 0x80: {i64 offset, {i64 time_point, uuid}}, 0x20 each.
    for (uint32_t off : {0x38u, 0x80u}) {
        const uint8_t* c = live_slot(base, off, 4, 0x20);
        int64_t offset = 0, tp = 0;
        std::memcpy(&offset, c, 8);
        std::memcpy(&tp, c + 8, 8);
        CHECK(tp >= 3600);
        CHECK(abs64(offset + tp - host_unix()) <= 3);
        CHECK(std::memcmp(c + 16, kSourceId, 16) == 0);
    }

    // Automatic correction flag: counter at 0xC8, one byte per slot at 0xCC / 0xCD, fresh state = 0.
    CHECK_EQ(*live_slot(base, 0xC8, 0, 1), 0u);

    // ContinuousAdjustmentTimePoint, pad 4 at 0xD0, 0x38 per slot: {u64 clock_offset, i64 multiplier, i64 divisor_log2,
    // SystemClockContext {i64 offset, {i64 time_point, uuid}}}: multiplier 1, divisor 0, context steady time point 0, source uuid.
    const uint8_t* adj = live_slot(base, 0xD0, 4, 0x38);
    int64_t mult = 0, div = -1, ctx_tp = -1;
    std::memcpy(&mult, adj + 8, 8);
    std::memcpy(&div, adj + 16, 8);
    std::memcpy(&ctx_tp, adj + 32, 8);
    CHECK_EQ(mult, 1);
    CHECK_EQ(div, 0);
    CHECK_EQ(ctx_tp, 0);
    CHECK(std::memcmp(adj + 40, kSourceId, 16) == 0);
}

NXTEST(svc_time_shared_memory_follows_set_current_time) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:a");
    nxsvc::Session user = st.sub(st.call(0));
    REQUIRE(user.call(1, nxsvc::Args().i64(1700000000)).ok());
    const uint8_t* base = shm_of(st);
    REQUIRE(base != nullptr);
    const uint8_t* local = live_slot(base, 0x38, 4, 0x20);
    int64_t offset = 0, tp = 0;
    std::memcpy(&offset, local, 8);
    std::memcpy(&tp, local + 8, 8);
    CHECK(abs64(offset + tp - 1700000000) <= 3);
}

NXTEST(svc_time_shared_memory_keeps_posix_across_steady_offset) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:r");
    nxsvc::Session steady = st.sub(st.call(2));
    const uint8_t* base = shm_of(st);
    REQUIRE(base != nullptr);
    REQUIRE(steady.call(3, nxsvc::Args().i64(1000000000000ll)).ok());   // +1000 s on the steady clock
    const uint8_t* local = live_slot(base, 0x38, 4, 0x20);
    int64_t offset = 0, tp = 0;
    std::memcpy(&offset, local, 8);
    std::memcpy(&tp, local + 8, 8);
    // The steady clock moved by 1000 s; the posix time read through the shared memory must not.
    CHECK(abs64(offset + tp - host_unix()) <= 3);
    CHECK(tp >= 4600);
    const uint8_t* steady_ctx = live_slot(base, 0x00, 4, 0x18);
    int64_t internal = 0;
    std::memcpy(&internal, steady_ctx, 8);
    CHECK_EQ(internal, kSteadyEpochNs + 1000000000000ll);
}

NXTEST(svc_time_automatic_correction_flag) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:a");
    CHECK_EQ(st.call(100).get<uint8_t>(0), 0);                       // IsStandardUserSystemClockAutomaticCorrectionEnabled
    REQUIRE(st.call(101, nxsvc::Args().u8(1)).ok());                 // SetStandardUser...Enabled(true)
    CHECK_EQ(st.call(100).get<uint8_t>(0), 1);
    const uint8_t* base = shm_of(st);
    REQUIRE(base != nullptr);
    CHECK_EQ(*live_slot(base, 0xC8, 0, 1), 1u);
    // time:u may not change it.
    nxsvc::Session user = nxsvc::connect("time:u");
    CHECK_EQ(user.call(101, nxsvc::Args().u8(0)).result, kPermDenied);
}

NXTEST(svc_time_unknown_command_is_stub) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:u");
    CHECK_EQ(st.call(999).result, kernel::kStubService);
    nxsvc::Session steady = st.sub(st.call(2));
    CHECK_EQ(steady.call(999).result, kernel::kStubService);
}

NXTEST(svc_time_tz_default_location_is_host_or_utc) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:u");
    nxsvc::Session zone = st.sub(st.call(3));           // GetTimeZoneService
    REQUIRE(zone.valid());
    nxsvc::Reply name = zone.call(0);                   // GetDeviceLocationName -> 0x24 bytes
    REQUIRE(name.ok());
    CHECK(!name.str(0, 0x24).empty());
    nxsvc::Reply ver = zone.call(5);                  // GetTimeZoneRuleVersion: 16 zero bytes (Reply.out may carry harness padding)
    REQUIRE(ver.ok());
    REQUIRE(ver.out.size() >= 0x10);
    bool zero = true;
    for (size_t i = 0; i < 0x10; ++i) zero = zero && ver.out[i] == 0;
    CHECK(zero);
}

NXTEST(svc_time_tz_utc_config_location_and_list) {
    nxsvc::Env env;
    ConfigGuard guard;
    config_clear();
    config_set("time", "zone", "UTC");
    nxrt::services::reset_all();
    nxsvc::Session st = nxsvc::connect("time:u");
    nxsvc::Session zone = st.sub(st.call(3));
    REQUIRE(zone.valid());
    CHECK_EQ(zone.call(0).str(0, 0x24), std::string("UTC"));
    // LoadLocationNameList(index) -> u32 count, names in a buffer of 0x24-byte entries.
    const uint32_t total = zone.call(2).get<uint32_t>(0);
    CHECK(total >= 1);
    std::vector<uint8_t> names(6 * 0x24, 0);
    nxsvc::Reply list = zone.call(3, nxsvc::Args().u32(0).buf_c(names.data(), static_cast<uint32_t>(names.size())));
    REQUIRE(list.ok());
    CHECK_EQ(list.get<uint32_t>(0), total);
    CHECK(std::string(reinterpret_cast<const char*>(names.data())) == "UTC");
    // GetDeviceLocationNameAndUpdatedTime: name, 4 bytes padding, {i64 time_point, uuid} at 0x28.
    nxsvc::Reply upd = zone.call(6);
    REQUIRE(upd.ok());
    REQUIRE(upd.out.size() >= 0x40);
    CHECK_EQ(upd.str(0, 0x24), std::string("UTC"));
    CHECK(upd.get<int64_t>(0x28) >= 3600);
    CHECK(std::memcmp(upd.out.data() + 0x30, kSourceId, 16) == 0);
    config_clear();
}

NXTEST(svc_time_tz_load_rule_utc_matches_make_utc_rule) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:u");
    nxsvc::Session zone = st.sub(st.call(3));
    REQUIRE(zone.valid());
    char name[0x24] = {};
    std::memcpy(name, "UTC", 3);
    std::vector<uint8_t> want(tz::kRuleSize);
    tz::make_utc_rule(want.data());
    std::vector<uint8_t> got(tz::kRuleSize, 0xEE);
    nxsvc::Reply r = zone.call(4, nxsvc::Args().bytes(name, 0x24).buf_b(got.data(), got.size()));   // LoadTimeZoneRule
    CHECK_EQ(r.result, 0u);
    CHECK(got == want);
}

NXTEST(svc_time_tz_load_rule_errors) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:u");
    nxsvc::Session zone = st.sub(st.call(3));
    char utc[0x24] = {};
    std::memcpy(utc, "UTC", 3);
    char mars[0x24] = {};
    std::memcpy(mars, "Mars/Olympus", 12);
    std::vector<uint8_t> small(0x100), big(tz::kRuleSize);
    CHECK_EQ(zone.call(4, nxsvc::Args().bytes(utc, 0x24).buf_b(small.data(), small.size())).result, kOutOfRange);
    CHECK_EQ(zone.call(4, nxsvc::Args().bytes(mars, 0x24).buf_b(big.data(), big.size())).result, kTzNotFound);
}

NXTEST(svc_time_tz_set_location_name) {
    nxsvc::Env env;
    ConfigGuard guard;
    nxsvc::Session user = nxsvc::connect("time:u");
    nxsvc::Session admin = nxsvc::connect("time:a");
    nxsvc::Session user_tz = user.sub(user.call(3));
    nxsvc::Session admin_tz = admin.sub(admin.call(3));
    char utc[0x24] = {};
    std::memcpy(utc, "UTC", 3);
    char mars[0x24] = {};
    std::memcpy(mars, "Mars/Olympus", 12);
    CHECK_EQ(user_tz.call(1, nxsvc::Args().bytes(utc, 0x24)).result, kPermDenied);
    CHECK_EQ(admin_tz.call(1, nxsvc::Args().bytes(mars, 0x24)).result, kTzNotFound);
    CHECK_EQ(admin_tz.call(1, nxsvc::Args().bytes(utc, 0x24)).result, 0u);
    CHECK_EQ(admin_tz.call(0).str(0, 0x24), std::string("UTC"));
}

NXTEST(svc_time_tz_to_calendar_without_rule_is_error) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:u");
    nxsvc::Session zone = st.sub(st.call(3));
    // No rule buffer: TimeZoneConversionFailed.
    CHECK_EQ(zone.call(100, nxsvc::Args().i64(1700000000)).result, kTzConversionFailed);
    CHECK_EQ(zone.call(201, nxsvc::Args().put<int16_t>(2023).put<int8_t>(11).put<int8_t>(14).put<int8_t>(22).put<int8_t>(13).put<int8_t>(20).u8(0)).result,
             kTzConversionFailed);
}

NXTEST(svc_time_tz_to_calendar_utc_needs_tz) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:u");
    nxsvc::Session zone = st.sub(st.call(3));
    std::vector<uint8_t> rule(tz::kRuleSize);
    tz::make_utc_rule(rule.data());
    // ToCalendarTime(1700000000) under UTC: 2023-11-14 22:13:20, Tuesday (0 = Sunday), 0-based day of year 317, "UTC", no DST, offset 0.
    nxsvc::Reply r = zone.call(100, nxsvc::Args().i64(1700000000).buf_a(rule.data(), rule.size()));
    REQUIRE(r.ok());
    REQUIRE(r.out.size() >= 0x20);
    CHECK_EQ(r.get<int16_t>(0), 2023);
    CHECK_EQ(r.get<int8_t>(2), 11);
    CHECK_EQ(r.get<int8_t>(3), 14);
    CHECK_EQ(r.get<int8_t>(4), 22);
    CHECK_EQ(r.get<int8_t>(5), 13);
    CHECK_EQ(r.get<int8_t>(6), 20);
    CHECK_EQ(r.get<uint32_t>(8), 2u);
    CHECK_EQ(r.get<uint32_t>(12), 317u);
    CHECK(r.str(16, 8) == "UTC");
    CHECK_EQ(r.get<int32_t>(24), 0);
    CHECK_EQ(r.get<int32_t>(28), 0);
}

NXTEST(svc_time_tz_to_posix_utc_needs_tz) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:u");
    nxsvc::Session zone = st.sub(st.call(3));
    std::vector<uint8_t> rule(tz::kRuleSize);
    tz::make_utc_rule(rule.data());
    std::vector<int64_t> times(10, 0);
    // CalendarTime bytes: year i16 2023, month/day/hour/minute/second i8, reserved u8.
    nxsvc::Args a = nxsvc::Args().put<int16_t>(2023).put<int8_t>(11).put<int8_t>(14).put<int8_t>(22).put<int8_t>(13).put<int8_t>(20).u8(0);
    a.buf_a(rule.data(), rule.size()).buf_c(times.data(), static_cast<uint32_t>(times.size() * 8));
    nxsvc::Reply r = zone.call(201, a);
    REQUIRE(r.ok());
    CHECK_EQ(r.get<uint32_t>(0), 1u);
    CHECK_EQ(times[0], 1700000000);
}

NXTEST(svc_time_tz_with_my_rule_utc_needs_tz) {
    nxsvc::Env env;
    ConfigGuard guard;
    config_clear();
    config_set("time", "zone", "UTC");
    nxrt::services::reset_all();
    nxsvc::Session st = nxsvc::connect("time:u");
    nxsvc::Session zone = st.sub(st.call(3));
    nxsvc::Reply cal = zone.call(101, nxsvc::Args().i64(1700000000));   // ToCalendarTimeWithMyRule
    REQUIRE(cal.ok());
    CHECK_EQ(cal.get<int16_t>(0), 2023);
    CHECK_EQ(cal.get<uint32_t>(8), 2u);
    CHECK_EQ(cal.get<uint32_t>(12), 317u);
    std::vector<int64_t> times(10, 0);
    nxsvc::Args a = nxsvc::Args().put<int16_t>(2023).put<int8_t>(11).put<int8_t>(14).put<int8_t>(22).put<int8_t>(13).put<int8_t>(20).u8(0);
    a.buf_c(times.data(), static_cast<uint32_t>(times.size() * 8));
    nxsvc::Reply pos = zone.call(202, a);               // ToPosixTimeWithMyRule
    REQUIRE(pos.ok());
    CHECK_EQ(pos.get<uint32_t>(0), 1u);
    CHECK_EQ(times[0], 1700000000);
    config_clear();
}

NXTEST(svc_time_operation_event_handles) {
    nxsvc::Env env;
    nxsvc::Session st = nxsvc::connect("time:u");
    nxsvc::Session clock = st.sub(st.call(0));
    nxsvc::Reply ev = clock.call(4);                  // ISystemClock::GetOperationEventReadableHandle
    REQUIRE(ev.ok());
    CHECK_EQ(ev.copy.size(), 1u);
    nxsvc::Session zone = st.sub(st.call(3));
    nxsvc::Reply tev = zone.call(20);                   // GetDeviceLocationNameOperationEventReadableHandle
    REQUIRE(tev.ok());
    CHECK_EQ(tev.copy.size(), 1u);
}
