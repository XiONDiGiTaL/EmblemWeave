// SPDX-License-Identifier: GPL-3.0-or-later
// Time zone rules and calendar conversion (services/timezone.h). Expected values are not computed by timezone.cpp: they are IANA
// tzdata answers (Python zoneinfo), Gregorian calendar facts, or the Windows host answer. The DST rule used here is written
// byte by byte with the layout offsets below, not through make_*_rule.
#include <chrono>
#include <cstdint>
#include <cstring>
#include <random>
#include <string>
#include <vector>

#include "../src/services/timezone.h"
#include "test_framework.h"

NXTEST_GROUP(svc_timezone)

using namespace nxrt::services::tz;

namespace {

// Layout of the 0x4000-byte rule as Switchbrew/Ryujinx describe it (written out here, independent of timezone.cpp):
// timecnt@0, typecnt@4, charcnt@8, goback@12, goahead@13, ats[1000] int64@16, types[1000]@8016, ttis[128] (16 bytes)@9016,
// chars[512]@11064, defaulttype@11576.
constexpr size_t kAtsOff = 16;
constexpr size_t kTypesOff = 8016;
constexpr size_t kTtisOff = 9016;
constexpr size_t kCharsOff = 11064;
constexpr size_t kDefaultTypeOff = 11576;

struct RuleBytes {
    std::vector<uint8_t> b = std::vector<uint8_t>(kRuleSize, 0);
    void put32(size_t off, int32_t v) { std::memcpy(&b[off], &v, 4); }
    void put64(size_t off, int64_t v) { std::memcpy(&b[off], &v, 8); }
    void type(int i, int32_t utoff, bool dst, int32_t desig) {
        const size_t o = kTtisOff + 16 * static_cast<size_t>(i);
        put32(o, utoff);
        b[o + 4] = dst ? 1 : 0;
        put32(o + 8, desig);
    }
    void transition(int i, int64_t at, uint8_t type_index) {
        put64(kAtsOff + 8 * static_cast<size_t>(i), at);
        b[kTypesOff + static_cast<size_t>(i)] = type_index;
    }
    const uint8_t* data() const { return b.data(); }
};

struct Expect {
    int year, month, day, hour, minute, second, dow, doy, dst, offset;
    const char* name;
};

void check_calendar(const uint8_t* rule, int64_t t, const Expect& e) {
    CalendarTime c{};
    CalendarAdditionalInfo i{};
    REQUIRE(to_calendar(rule, t, &c, &i));
    CHECK_EQ(c.year, e.year);
    CHECK_EQ(c.month, e.month);
    CHECK_EQ(c.day, e.day);
    CHECK_EQ(c.hour, e.hour);
    CHECK_EQ(c.minute, e.minute);
    CHECK_EQ(c.second, e.second);
    CHECK_EQ(i.day_of_week, static_cast<uint32_t>(e.dow));
    CHECK_EQ(i.day_of_year, static_cast<uint32_t>(e.doy));
    CHECK_EQ(i.is_dst, e.dst);
    CHECK_EQ(i.utc_offset, e.offset);
    CHECK_EQ(std::string(i.tz_name), std::string(e.name));
}

// Last Sunday of a month at 01:00 UTC: the EU summer-time transitions since 1996.
int64_t eu_transition(int y, unsigned m) {
    using namespace std::chrono;
    sys_days d = sys_days{year_month_day_last{year{y}, month_day_last{month{m}}}};
    while (weekday{d} != Sunday) d -= days{1};
    return duration_cast<seconds>(d.time_since_epoch()).count() + 3600;
}

// Europe/Berlin as far as this test needs it: CET (+1) default, CEST (+2) from the EU rule, transitions 1996..2037.
RuleBytes berlin_rule() {
    RuleBytes r;
    r.put32(4, 2);     // typecnt
    r.put32(8, 10);    // charcnt: "CET\0CEST\0"
    r.type(0, 3600, false, 0);
    r.type(1, 7200, true, 4);
    std::memcpy(&r.b[kCharsOff], "CET\0CEST\0", 10);
    int n = 0;
    for (int y = 1996; y <= 2037; ++y) {
        r.transition(n++, eu_transition(y, 3), 1);
        r.transition(n++, eu_transition(y, 10), 0);
    }
    r.put32(0, n);     // timecnt
    r.put32(kDefaultTypeOff, 0);
    return r;
}

RuleBytes utc_rule_bytes() {
    RuleBytes r;
    r.put32(4, 1);
    r.put32(8, 4);
    r.type(0, 0, false, 0);
    std::memcpy(&r.b[kCharsOff], "UTC", 4);
    return r;
}

void round_trip(const uint8_t* rule, int count, uint64_t seed) {
    std::mt19937_64 gen(seed);
    std::uniform_int_distribution<int64_t> dist(0, 4000000000LL);
    for (int k = 0; k < count; ++k) {
        const int64_t t = dist(gen);
        CalendarTime c{};
        CalendarAdditionalInfo i{};
        REQUIRE(to_calendar(rule, t, &c, &i));
        int64_t back[2] = {0, 0};
        const int n = to_posix(rule, c, back);
        bool found = false;
        for (int j = 0; j < n; ++j) found = found || back[j] == t;
        if (!found) {
            CHECK_MSG(false, "round trip lost t=" + std::to_string(t));
            return;
        }
        CHECK(n >= 1 && n <= 2);
    }
}

}  // namespace

NXTEST(svc_tz_utc_rule_layout) {
    uint8_t rule[kRuleSize];
    make_utc_rule(rule);
    int32_t timecnt = 0, typecnt = 0, charcnt = 0, defaulttype = -1;
    std::memcpy(&timecnt, rule + 0, 4);
    std::memcpy(&typecnt, rule + 4, 4);
    std::memcpy(&charcnt, rule + 8, 4);
    std::memcpy(&defaulttype, rule + kDefaultTypeOff, 4);
    CHECK_EQ(timecnt, 0);
    CHECK_EQ(typecnt, 1);
    CHECK_EQ(charcnt, 4);
    CHECK_EQ(defaulttype, 0);
    CHECK_EQ(rule[12], 0);   // goback
    CHECK_EQ(rule[13], 0);   // goahead
    CHECK(std::memcmp(rule + kCharsOff, "UTC\0", 4) == 0);
    CHECK_EQ(rule[kTtisOff + 0], 0);   // utoff 0
    CHECK_EQ(rule[kTtisOff + 4], 0);   // not DST
    bool tail_zero = true;
    for (size_t k = 11580; k < kRuleSize; ++k) tail_zero = tail_zero && rule[k] == 0;
    CHECK(tail_zero);
}

NXTEST(svc_tz_utc_calendar_oracles) {
    const RuleBytes r = utc_rule_bytes();
    // posix 0 = 1970-01-01 00:00:00 Thursday
    check_calendar(r.data(), 0, {1970, 1, 1, 0, 0, 0, 4, 0, 0, 0, "UTC"});
    // posix 1700000000 = 2023-11-14 22:13:20 Tuesday, day 317
    check_calendar(r.data(), 1700000000, {2023, 11, 14, 22, 13, 20, 2, 317, 0, 0, "UTC"});
    // posix 951782400 = 2000-02-29 00:00:00 (leap day), Tuesday
    check_calendar(r.data(), 951782400, {2000, 2, 29, 0, 0, 0, 2, 59, 0, 0, "UTC"});
    // posix -1 = 1969-12-31 23:59:59 Wednesday, last day of 1969
    check_calendar(r.data(), -1, {1969, 12, 31, 23, 59, 59, 3, 364, 0, 0, "UTC"});
    // posix 4102444800 = 2100-01-01 00:00:00 Friday (2100 is not a leap year)
    check_calendar(r.data(), 4102444800LL, {2100, 1, 1, 0, 0, 0, 5, 0, 0, 0, "UTC"});
}

NXTEST(svc_tz_utc_to_posix) {
    const RuleBytes r = utc_rule_bytes();
    int64_t out[2] = {-7, -7};
    CalendarTime c{};
    c.year = 2023; c.month = 11; c.day = 14; c.hour = 22; c.minute = 13; c.second = 20;
    CHECK_EQ(to_posix(r.data(), c, out), 1);
    CHECK_EQ(out[0], 1700000000LL);
    c.year = 1969; c.month = 12; c.day = 31; c.hour = 23; c.minute = 59; c.second = 59;
    CHECK_EQ(to_posix(r.data(), c, out), 1);
    CHECK_EQ(out[0], -1LL);
    c.month = 13;   // not a month
    CHECK_EQ(to_posix(r.data(), c, out), 0);
}

NXTEST(svc_tz_berlin_summer_time) {
    const RuleBytes r = berlin_rule();
    // 2023-07-01 12:00:00 UTC is 14:00:00 CEST (+2), Saturday, day 181
    check_calendar(r.data(), 1688212800LL, {2023, 7, 1, 14, 0, 0, 6, 181, 1, 7200, "CEST"});
    // 2001-09-09 01:46:40 UTC is 03:46:40 CEST, Sunday
    check_calendar(r.data(), 1000000000, {2001, 9, 9, 3, 46, 40, 0, 251, 1, 7200, "CEST"});
    // 2030-07-15 10:00 UTC is 12:00 CEST, Monday
    check_calendar(r.data(), 1910340000, {2030, 7, 15, 12, 0, 0, 1, 195, 1, 7200, "CEST"});
}

NXTEST(svc_tz_berlin_transition_edges) {
    const RuleBytes r = berlin_rule();
    // spring forward at 2023-03-26 01:00 UTC
    check_calendar(r.data(), 1679792399LL, {2023, 3, 26, 1, 59, 59, 0, 84, 0, 3600, "CET"});
    check_calendar(r.data(), 1679792400LL, {2023, 3, 26, 3, 0, 0, 0, 84, 1, 7200, "CEST"});
    // fall back at 2023-10-29 01:00 UTC
    check_calendar(r.data(), 1698541199LL, {2023, 10, 29, 2, 59, 59, 0, 301, 1, 7200, "CEST"});
    check_calendar(r.data(), 1698541200LL, {2023, 10, 29, 2, 0, 0, 0, 301, 0, 3600, "CET"});
    // the same transition in 2030
    check_calendar(r.data(), 1919293199LL, {2030, 10, 27, 2, 59, 59, 0, 299, 1, 7200, "CEST"});
    check_calendar(r.data(), 1919293200LL, {2030, 10, 27, 2, 0, 0, 0, 299, 0, 3600, "CET"});
}

NXTEST(svc_tz_berlin_repeated_hour_two_candidates) {
    const RuleBytes r = berlin_rule();
    // 2023-10-29 02:30:00 happens twice: 00:30 UTC (CEST) and 01:30 UTC (CET)
    CalendarTime c{};
    c.year = 2023; c.month = 10; c.day = 29; c.hour = 2; c.minute = 30; c.second = 0;
    int64_t out[2] = {0, 0};
    CHECK_EQ(to_posix(r.data(), c, out), 2);
    CHECK_EQ(out[0], 1698539400LL);
    CHECK_EQ(out[1], 1698543000LL);
    check_calendar(r.data(), out[0], {2023, 10, 29, 2, 30, 0, 0, 301, 1, 7200, "CEST"});
    check_calendar(r.data(), out[1], {2023, 10, 29, 2, 30, 0, 0, 301, 0, 3600, "CET"});
    // 2030 repeated hour: 00:30 UTC and 01:30 UTC of 2030-10-27
    c.year = 2030; c.day = 27;
    CHECK_EQ(to_posix(r.data(), c, out), 2);
    CHECK_EQ(out[0], 1919291400LL);
    CHECK_EQ(out[1], 1919295000LL);
}

NXTEST(svc_tz_berlin_skipped_hour_none) {
    const RuleBytes r = berlin_rule();
    // 2023-03-26 02:30:00 does not exist in Berlin (clocks jump 02:00 -> 03:00): no posix time maps to it
    CalendarTime c{};
    c.year = 2023; c.month = 3; c.day = 26; c.hour = 2; c.minute = 30; c.second = 0;
    int64_t out[2] = {0, 0};
    CHECK_EQ(to_posix(r.data(), c, out), 0);
    // 2023-03-26 03:30:00 does exist (CEST, 01:30 UTC)
    c.hour = 3;
    CHECK_EQ(to_posix(r.data(), c, out), 1);
    CHECK_EQ(out[0], 1679792400LL + 1800);
}

NXTEST(svc_tz_round_trip_utc) {
    const RuleBytes r = utc_rule_bytes();
    round_trip(r.data(), 2000, 0x5EED0001u);
}

NXTEST(svc_tz_round_trip_berlin) {
    const RuleBytes r = berlin_rule();
    round_trip(r.data(), 2000, 0x5EED0002u);
}

NXTEST(svc_tz_round_trip_host) {
    std::vector<uint8_t> rule(kRuleSize, 0);
    std::string name;
    const bool ok = make_host_rule(rule.data(), &name);
    CHECK(ok);
    CHECK(!name.empty());
    round_trip(rule.data(), 2000, 0x5EED0003u);
}

NXTEST(svc_tz_invalid_rule_rejected) {
    RuleBytes r = utc_rule_bytes();
    r.put32(4, 0);   // typecnt 0
    CalendarTime c{};
    CalendarAdditionalInfo i{};
    CHECK(!to_calendar(r.data(), 0, &c, &i));
    int64_t out[2] = {0, 0};
    CHECK_EQ(to_posix(r.data(), c, out), 0);

    RuleBytes s = utc_rule_bytes();
    s.put32(0, 1);            // one transition...
    s.put64(kAtsOff, 100);
    s.b[kTypesOff] = 5;       // ...pointing at a type that does not exist
    CHECK(!to_calendar(s.data(), 0, &c, &i));
}

NXTEST(svc_tz_host_rule_name) {
    std::vector<uint8_t> rule(kRuleSize, 0);
    std::string name;
    CHECK(make_host_rule(rule.data(), &name));
    // The name is an IANA location for the common Windows zones and the Windows id otherwise; it is never empty.
    CHECK(!name.empty());
    CalendarTime c{};
    CalendarAdditionalInfo i{};
    CHECK(to_calendar(rule.data(), 1700000000, &c, &i));
    CHECK_EQ(c.year, 2023);
}
