// SPDX-License-Identifier: GPL-3.0-or-later
// audren_common.h: the Span/Guest helpers every ported audio renderer file uses.
#include "../src/services/audren/audren_common.h"
#include "test_framework.h"

NXTEST_GROUP(svc_audren_common)

using namespace nxrt::audren;

NXTEST(span_basics) {
    std::vector<int32_t> v = {1, 2, 3, 4, 5, 6};
    Span<int32_t> s(v);
    CHECK_EQ(s.Length, 6u);
    CHECK_EQ(s[2], 3);
    Span<int32_t> t = s.Slice(2, 3);
    CHECK_EQ(t.Length, 3u);
    CHECK_EQ(t[0], 3);
    t.Fill(9);
    CHECK_EQ(v[2], 9);
    CHECK_EQ(v[4], 9);
    CHECK_EQ(v[5], 6);
    Span<const int32_t> c = s;
    CHECK_EQ(c.Length, 6u);
    std::vector<int32_t> d(6, 0);
    s.CopyTo(Span<int32_t>(d));
    CHECK_EQ(d[0], 1);
    CHECK_EQ(d[5], 6);
    s.Slice(1, 2).Clear();
    CHECK_EQ(v[1], 0);
    CHECK_EQ(v[2], 0);
    Span<int16_t> h = s.Cast<int16_t>();
    CHECK_EQ(h.Length, 12u);
    int sum = 0;
    for (int32_t x : s) sum += x;
    CHECK_EQ(sum, 1 + 0 + 0 + 9 + 9 + 6);
}

NXTEST(guest_span_is_a_host_pointer) {
    uint32_t words[4] = {10, 20, 30, 40};
    Span<uint32_t> g = GuestSpan<uint32_t>(reinterpret_cast<uint64_t>(words), 4);
    CHECK_EQ(g[3], 40u);
    *GuestPtr<uint32_t>(reinterpret_cast<uint64_t>(words)) = 11;
    CHECK_EQ(words[0], 11u);
    CHECK_EQ(ClampT(5, 0, 3), 3);
}
