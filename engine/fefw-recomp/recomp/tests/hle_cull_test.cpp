// SPDX-License-Identifier: GPL-3.0-or-later
// The native culling tests of game-build/fefw/src/fefw_hle.cpp against the game's own (lifted) functions, on random inputs: ordinary
// volumes and boxes, and "wild" ones (random bytes, NaN, infinities, signed zeros, empty and inverted boxes, every plane count and
// flag). Needs cull_fns.inc, which hle_cull_extract.py writes from a lift of your own dump (game code: never committed).
//
//   clang++ -O2 -mfma -std=c++20 -fno-exceptions -ffp-contract=off -I recomp/include -I recomp/tests -I game-build/fefw/third_party/zstd
//       recomp/tests/hle_cull_test.cpp game-build/fefw/src/fefw_hle.cpp <zstd decoder objects or stubs> -o hle_cull_test
//   hle_cull_test [cases, default 3000000]        exit 0 when no answer differs
// Run it for each way the game code is built: -mfma, and without it with and without -DNX_FAST_FMA.
#include <nxrecomp/cxx.h>
#include <cmath>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <random>
#include "cull_fns.inc"

extern "C" {
int nx_hle_cull_view_box_raw(uint64_t volume, uint64_t box);
int nx_hle_cull_cylinder_box_raw(uint64_t volume, uint64_t box);
}

namespace {
std::mt19937 g(12345);
float rf(float a, float b) { return a + (b - a) * (g() / 4294967296.0f); }
float weird(float x) {
    switch (g() % 64) {
    case 0: return NAN;
    case 1: return INFINITY;
    case 2: return -INFINITY;
    case 3: return 0.0f;
    case 4: return -0.0f;
    case 5: return 1e-30f;
    case 6: return -NAN;
    default: return x;
    }
}
uint64_t g_stack[4096];
Ctx* g_c;
int lifted(void (*fn)(Ctx*), void* v, void* b) {
    g_c->x[0] = reinterpret_cast<uint64_t>(v);
    g_c->x[1] = reinterpret_cast<uint64_t>(b);
    g_c->x[31] = reinterpret_cast<uint64_t>(&g_stack[2048]);
    fn(g_c);
    return static_cast<int>(static_cast<uint32_t>(g_c->x[0]));
}
}  // namespace

int main(int argc, char** argv) {
    g_c = static_cast<Ctx*>(calloc(1, sizeof(Ctx) + 4096));
    const long cases = argc > 1 ? atol(argv[1]) : 3000000;
    alignas(16) uint8_t vol[0x110];
    alignas(16) float box[8];
    alignas(16) uint8_t cyl[0xc0];
    long bad_v = 0, bad_c = 0, ans[4] = {0, 0, 0, 0};
    for (long it = 0; it < cases; it++) {
        const bool wild = it % 3 == 0;
        for (auto& x : vol) x = wild ? static_cast<uint8_t>(g()) : 0;
        float* f = reinterpret_cast<float*>(vol);
        for (int k = 0; k < 4; k++) {
            f[4 + k] = rf(-60, -20), f[8 + k] = rf(20, 60);
            if (wild) f[4 + k] = weird(f[4 + k]), f[8 + k] = weird(f[8 + k]);
        }
        int planes = static_cast<int>(g() % 8);
        if (planes > 6) planes = 6;
        if (g() % 50 == 0) planes = -1;
        for (int p = 0; p < 6; p++) {
            float* n = reinterpret_cast<float*>(vol + 0x30 + 0x20 * p);
            for (int k = 0; k < 4; k++) n[k] = wild ? weird(rf(-1, 1)) : rf(-1, 1);
            if (!wild) n[3] = 0;
            n[4] = wild ? weird(rf(-40, 40)) : rf(-40, 10);
        }
        memcpy(vol + 0xf0, &planes, 4);
        const int use_bounds = static_cast<int>(g() % 3);
        memcpy(vol + 0xf4, &use_bounds, 4);
        vol[0x100] = static_cast<uint8_t>(g());
        for (int k = 0; k < 4; k++) {
            const float c = rf(-50, 50);
            float h = rf(0, 8);
            if (g() % 20 == 0) h = 0;
            if (g() % 40 == 0) h = -h;
            box[k] = c - h, box[4 + k] = c + h;
            if (wild) box[k] = weird(box[k]), box[4 + k] = weird(box[4 + k]);
        }
        for (auto& x : cyl) x = wild ? static_cast<uint8_t>(g()) : 0;
        float* q = reinterpret_cast<float*>(cyl);
        for (int k = 4; k < 36; k++) q[k] = wild ? weird(rf(-3, 3)) : rf(-2, 2);
        int corner[6];
        for (int k = 0; k < 6; k++) corner[k] = (g() % 10 == 0 && k) ? corner[k - 1] : static_cast<int>(g() % 8);
        if (g() % 30 == 0)
            for (int k = 1; k < 6; k++) corner[k] = corner[0];
        memcpy(cyl + 0x94, corner, sizeof corner);
        const float radius = wild ? weird(rf(0, 4)) : rf(0, 4);
        memcpy(cyl + 0xac, &radius, 4);
        cyl[0xb0] = static_cast<uint8_t>(g());

        const int a = lifted(g_71019786c0, vol, box), b = nx_hle_cull_view_box_raw(reinterpret_cast<uint64_t>(vol), reinterpret_cast<uint64_t>(box));
        if (a != b && bad_v++ < 8) printf("view volume: lifted %d, native %d (case %ld, %d planes, flag %d)\n", a, b, it, planes, vol[0x100] & 1);
        ans[a & 1]++;
        const int a2 = lifted(g_7101980580, cyl, box), b2 = nx_hle_cull_cylinder_box_raw(reinterpret_cast<uint64_t>(cyl), reinterpret_cast<uint64_t>(box));
        if (a2 != b2 && bad_c++ < 8) printf("cylinder: lifted %d, native %d (case %ld)\n", a2, b2, it);
        ans[2 + (a2 & 1)]++;
    }
    printf("%ld cases: view volume %ld differ (answers 0 / 1: %ld / %ld), cylinder %ld differ (answers 0 / 1: %ld / %ld)\n", cases, bad_v,
           ans[0], ans[1], bad_c, ans[2], ans[3]);
    return bad_v || bad_c ? 1 : 0;
}
