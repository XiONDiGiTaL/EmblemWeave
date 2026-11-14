// SPDX-License-Identifier: GPL-3.0-or-later
// NVNvertexAttribState / NVNvertexStreamState through the typed thunks (the reference test for the plain-data families).
#include <cstring>

#include "nvn_impl.h"
#include "test_framework.h"
NXTEST_GROUP(gfx_nvn_vertex)

using namespace nxrt;
using namespace nxrt::nvn;

namespace {
void setup() {
    install();
    abi::reset_mismatches();
    register_api_vertex();
    CHECK_EQ(abi::mismatches(), 0u);
}
}  // namespace

NXTEST(attrib_round_trip_and_decode) {
    setup();
    auto defaults = abi::proc<PFNNVNVERTEXATTRIBSTATESETDEFAULTSPROC>("nvnVertexAttribStateSetDefaults");
    auto set_fmt = abi::proc<PFNNVNVERTEXATTRIBSTATESETFORMATPROC>("nvnVertexAttribStateSetFormat");
    auto set_stream = abi::proc<PFNNVNVERTEXATTRIBSTATESETSTREAMINDEXPROC>("nvnVertexAttribStateSetStreamIndex");
    auto get_fmt = abi::proc<PFNNVNVERTEXATTRIBSTATEGETFORMATPROC>("nvnVertexAttribStateGetFormat");
    auto get_stream = abi::proc<PFNNVNVERTEXATTRIBSTATEGETSTREAMINDEXPROC>("nvnVertexAttribStateGetStreamIndex");
    NVNvertexAttribState s;
    std::memset(&s, 0xAB, sizeof s);
    defaults(&s);
    NVNformat f = NVN_FORMAT_R8;
    ptrdiff_t off = 99;
    get_fmt(&s, &f, &off);
    CHECK_EQ(static_cast<int>(f), 0);
    CHECK_EQ(static_cast<long long>(off), 0ll);
    CHECK_EQ(get_stream(&s), 0);

    set_fmt(&s, NVN_FORMAT_RGBA32F, 24);
    set_stream(&s, 3);
    get_fmt(&s, &f, &off);
    CHECK_EQ(static_cast<int>(f), static_cast<int>(NVN_FORMAT_RGBA32F));
    CHECK_EQ(static_cast<long long>(off), 24ll);
    CHECK_EQ(get_stream(&s), 3);
    set_fmt(&s, NVN_FORMAT_RG16F, 8);      // changing the format keeps the stream
    CHECK_EQ(get_stream(&s), 3);
    set_stream(&s, 5);                      // changing the stream keeps format and offset
    get_fmt(&s, &f, &off);
    CHECK_EQ(static_cast<int>(f), static_cast<int>(NVN_FORMAT_RG16F));
    CHECK_EQ(static_cast<long long>(off), 8ll);

    VertexAttribInfo d = decode(&s);
    CHECK_EQ(d.format, static_cast<int>(NVN_FORMAT_RG16F));
    CHECK_EQ(static_cast<long long>(d.relative_offset), 8ll);
    CHECK_EQ(d.stream, 5);
    NVNvertexAttribState zero;
    std::memset(&zero, 0, sizeof zero);
    VertexAttribInfo z = decode(&zero);
    CHECK_EQ(z.format, 0);
    CHECK_EQ(z.stream, 0);
    get_fmt(&s, nullptr, nullptr);   // null out pointers are allowed
}

NXTEST(stream_round_trip_and_decode) {
    setup();
    auto defaults = abi::proc<PFNNVNVERTEXSTREAMSTATESETDEFAULTSPROC>("nvnVertexStreamStateSetDefaults");
    auto set_stride = abi::proc<PFNNVNVERTEXSTREAMSTATESETSTRIDEPROC>("nvnVertexStreamStateSetStride");
    auto set_div = abi::proc<PFNNVNVERTEXSTREAMSTATESETDIVISORPROC>("nvnVertexStreamStateSetDivisor");
    auto get_stride = abi::proc<PFNNVNVERTEXSTREAMSTATEGETSTRIDEPROC>("nvnVertexStreamStateGetStride");
    auto get_div = abi::proc<PFNNVNVERTEXSTREAMSTATEGETDIVISORPROC>("nvnVertexStreamStateGetDivisor");
    NVNvertexStreamState s;
    std::memset(&s, 0xCD, sizeof s);
    defaults(&s);
    CHECK_EQ(static_cast<long long>(get_stride(&s)), 0ll);
    CHECK_EQ(get_div(&s), 0);
    set_stride(&s, 48);
    set_div(&s, 1);
    CHECK_EQ(static_cast<long long>(get_stride(&s)), 48ll);
    CHECK_EQ(get_div(&s), 1);
    VertexStreamInfo d = decode(&s);
    CHECK_EQ(static_cast<long long>(d.stride), 48ll);
    CHECK_EQ(d.divisor, 1);
}

NXTEST(copies_of_the_object_stay_independent) {
    setup();
    auto set_stride = abi::proc<PFNNVNVERTEXSTREAMSTATESETSTRIDEPROC>("nvnVertexStreamStateSetStride");
    auto get_stride = abi::proc<PFNNVNVERTEXSTREAMSTATEGETSTRIDEPROC>("nvnVertexStreamStateGetStride");
    NVNvertexStreamState a{}, b{};
    set_stride(&a, 16);
    std::memcpy(&b, &a, sizeof a);   // the game copies state objects around freely
    set_stride(&a, 32);
    CHECK_EQ(static_cast<long long>(get_stride(&b)), 16ll);
    CHECK_EQ(static_cast<long long>(get_stride(&a)), 32ll);
}
