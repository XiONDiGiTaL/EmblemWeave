// SPDX-License-Identifier: GPL-3.0-or-later
#include <string>
// The typed thunk layer (src/gfx/nvn/nvn_abi.h): arguments from guest registers, results back, signature check against the
// generated table, and the Caller used by the tests of every NVN family.
#include <cstdint>
#include <cstring>

#include "nvn_abi.h"
#include "test_framework.h"
NXTEST_GROUP(gfx_nvn_abi)

using namespace nxrt;
using namespace nxrt::nvn;

namespace {
struct Seen {
    void* p = nullptr;
    int a = 0;
    int64_t b = 0;
    float f = 0;
    double d = 0;
    uint8_t flag = 0;
} g_seen;

int mixed(void* p, int a, int64_t b, float f, double d, uint8_t flag) {
    g_seen = {p, a, b, f, d, flag};
    return a * 2;
}
float get_float(const void*) { return 2.5f; }
double get_double(const void*) { return -7.25; }
int64_t get_neg64(const void*) { return -5; }
int get_neg32(const void*) { return -5; }
uint64_t get_big(const void*) { return 0xFEDCBA9876543210ull; }
void* get_ptr(const void*) { return reinterpret_cast<void*>(0x7A00001230ull); }
bool get_bool(const void*) { return true; }
enum Small : int { kA = 1, kB = 2 };
Small get_enum(const void*) { return kB; }
void take_enum(Small e, int* out) { *out = static_cast<int>(e); }
}  // namespace

NXTEST(thunk_unpacks_integers_pointers_floats_in_order) {
    ::Ctx c{};
    c.x[0] = 0x7A00000010;
    c.x[1] = 0xFFFFFFFFFFFFFFFEull;   // int -2 in w1; the upper half is junk
    c.x[2] = 0x1122334455667788ull;
    c.x[3] = 0xFFFFFF01;               // uint8_t 1; junk above
    c.v[0].f32[0] = 1.5f;              // the first float register, whatever the integer position
    c.v[1].f64[0] = -3.25;
    abi::Thunk<&mixed>::call(&c);
    CHECK_EQ(reinterpret_cast<uint64_t>(g_seen.p), 0x7A00000010ull);
    CHECK_EQ(g_seen.a, -2);
    CHECK_EQ(g_seen.b, 0x1122334455667788ll);
    CHECK_EQ(g_seen.f, 1.5f);
    CHECK_EQ(g_seen.d, -3.25);
    CHECK_EQ(g_seen.flag, 1);
    CHECK_EQ(static_cast<uint32_t>(c.x[0]), 0xFFFFFFFCu);   // int result in w0
    CHECK_EQ(std::string(abi::Thunk<&mixed>::kArgs), "pilfdi");
    CHECK_EQ(abi::Thunk<&mixed>::kRet, 'i');
}

NXTEST(thunk_results) {
    ::Ctx c{};
    abi::Thunk<&get_float>::call(&c);
    CHECK_EQ(c.v[0].f32[0], 2.5f);
    abi::Thunk<&get_double>::call(&c);
    CHECK_EQ(c.v[0].f64[0], -7.25);
    abi::Thunk<&get_neg64>::call(&c);
    CHECK_EQ(c.x[0], 0xFFFFFFFFFFFFFFFBull);
    abi::Thunk<&get_neg32>::call(&c);
    CHECK_EQ(c.x[0], 0xFFFFFFFBull);
    abi::Thunk<&get_big>::call(&c);
    CHECK_EQ(c.x[0], 0xFEDCBA9876543210ull);
    abi::Thunk<&get_ptr>::call(&c);
    CHECK_EQ(c.x[0], 0x7A00001230ull);
    abi::Thunk<&get_bool>::call(&c);
    CHECK_EQ(c.x[0], 1ull);
    abi::Thunk<&get_enum>::call(&c);
    CHECK_EQ(c.x[0], 2ull);
    int out = 0;
    ::Ctx e{};
    e.x[0] = 0xDEAD00000002ull;   // enum in w0 (upper junk ignored)
    e.x[1] = reinterpret_cast<uint64_t>(&out);
    abi::Thunk<&take_enum>::call(&e);
    CHECK_EQ(out, 2);
}

NXTEST(register_checked_compares_with_the_table) {
    install();
    abi::reset_mismatches();
    // nvnTextureBuilderSetSize2D is (p, i, i) -> void: the right classes register, the wrong ones are refused and counted
    static constexpr auto good = +[](void*, int, int) {};
    static constexpr auto bad_arity = +[](void*, int) {};
    static constexpr auto bad_float = +[](void*, float, int) {};
    CHECK(abi::register_checked("nvnTextureBuilderSetSize2D", &abi::Thunk<good>::call, abi::Thunk<good>::kRet, abi::Thunk<good>::kArgs));
    CHECK(!abi::register_checked("nvnTextureBuilderSetSize2D", &abi::Thunk<bad_arity>::call, abi::Thunk<bad_arity>::kRet, abi::Thunk<bad_arity>::kArgs));
    CHECK(!abi::register_checked("nvnTextureBuilderSetSize2D", &abi::Thunk<bad_float>::call, abi::Thunk<bad_float>::kRet, abi::Thunk<bad_float>::kArgs));
    CHECK(!abi::register_checked("nvnNotAnEntryPoint", &abi::Thunk<good>::call, 'v', "pii"));
    CHECK_EQ(abi::mismatches(), 3u);
    abi::reset_mismatches();
}

NXTEST(caller_round_trip_through_the_dispatcher) {
    install();
    static int g_a = 0, g_b = 0;
    static void* g_p = nullptr;
    static constexpr auto impl = +[](void* p, int a, int b) { g_p = p; g_a = a; g_b = b; };
    REQUIRE(abi::register_checked("nvnTextureBuilderSetSize2D", &abi::Thunk<impl>::call, 'v', abi::Thunk<impl>::kArgs));
    auto fn = abi::proc<PFNNVNTEXTUREBUILDERSETSIZE2DPROC>("nvnTextureBuilderSetSize2D");
    NVNtextureBuilder tb{};
    fn(&tb, 640, -480);
    CHECK_EQ(g_p, static_cast<void*>(&tb));
    CHECK_EQ(g_a, 640);
    CHECK_EQ(g_b, -480);

    // a getter with a float result and a float argument pair
    static constexpr auto lod = +[](const NVNsamplerBuilder*) { return 0.75f; };
    REQUIRE(abi::register_checked("nvnSamplerBuilderGetLodBias", &abi::Thunk<lod>::call, 'f', abi::Thunk<lod>::kArgs));
    auto g = abi::proc<PFNNVNSAMPLERBUILDERGETLODBIASPROC>("nvnSamplerBuilderGetLodBias");
    NVNsamplerBuilder sb{};
    CHECK_EQ(g(&sb), 0.75f);
}
