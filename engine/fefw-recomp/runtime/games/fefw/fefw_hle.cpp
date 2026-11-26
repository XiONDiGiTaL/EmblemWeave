// SPDX-License-Identifier: GPL-3.0-or-later
// Native routines that the lifted game code calls in place of a lifted guest function (GameConfig.inline_replace bodies; see
// recomp/games/fefw/config.py). They live in the game module (nxrt_game_fefw), which both fefw.exe and the release fefw_rt.dll
// link: the packaged exe imports them from the DLL and needs no zstd headers or objects on the player's PC. Each one keeps the
// guest function's results exactly or hands the call back to the lifted original (`orig`).
#include <zstd.h>

#include <emmintrin.h>   // the native culling tests (SSE2)
#include <math.h>

#include <atomic>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <string>

#include "nxrecomp/ctx.h"
#include "nxrt/diag.h"
#include "nxrt/log.h"

namespace {

// FEFW_HLE_ZSTD=0 turns the native zstd decoder off (every call goes to the lifted original).
int zstd_enabled() {
    static std::atomic<int> on{-1};
    int v = on.load(std::memory_order_relaxed);
    if (v < 0) {
        const char* e = std::getenv("FEFW_HLE_ZSTD");
        v = !(e && e[0] == '0');
        on.store(v, std::memory_order_relaxed);
    }
    return v;
}

uint32_t load32(uint64_t a) {
    uint32_t v;
    std::memcpy(&v, reinterpret_cast<const void*>(static_cast<uintptr_t>(a)), sizeof v);
    return v;
}

uint64_t load64(uint64_t a) {
    uint64_t v;
    std::memcpy(&v, reinterpret_cast<const void*>(static_cast<uintptr_t>(a)), sizeof v);
    return v;
}

}  // namespace

// The game's zstd (1.5.x, statically linked in main): ZSTD_decompressMultiFrame(dctx, dst, dstCapacity, src, srcSize, dict, dictSize,
// ddict), which ZSTD_decompressStream's single-pass shortcut and the one-shot API call when whole frames are in memory (the loading /
// area-change hot path: 50-75 % of some loading frames). Decoding is defined by the format, so any conforming decoder writes the same
// bytes and returns the same size. Without a dictionary the guest dctx contributes only parameters that make a frame fail (window
// limit, magicless format, checksum policy), and its internal tables are reset before the next frame: a native error therefore
// re-runs the call in the lifted original, and so does any call with a dictionary. The registers the call leaves are x0 = the size;
// x19-x29 and SP are untouched (callee-saved), the rest are free under the AAPCS64.
// The guest dctx fields read (this build's layout, as the lifted ZSTD_decompressMultiFrame / ZSTD_decompressStream use them): the
// frame format at +0x7598 (0 = ZSTD_f_zstd1, with magic) and maxWindowSize at +0x7638 (the default, 2^27 + 1): with anything else
// set, the call stays with the original, so a window limit or format the native decoder does not share cannot change a result.
extern "C" void nx_hle_zstd_multiframe(Ctx* c, void (*orig)(Ctx*)) {
    const uint64_t dctx = c->x[0];
    if (!zstd_enabled() || c->x[5] || c->x[6] || c->x[7] || load32(dctx + 0x7598) != 0 || load64(dctx + 0x7638) != (1ull << 27) + 1) {
        orig(c);
        return;
    }
    static thread_local ZSTD_DCtx* d = nullptr;
    if (!d && !(d = ZSTD_createDCtx())) {
        orig(c);
        return;
    }
    const size_t r = ZSTD_decompressDCtx(d, reinterpret_cast<void*>(c->x[1]), static_cast<size_t>(c->x[2]),
                                         reinterpret_cast<const void*>(c->x[3]), static_cast<size_t>(c->x[4]));
    if (ZSTD_isError(r)) {
        // ZSTD_getErrorName names the error, not the asset. Prefer this thread's newest recent-file entry (the file the load was
        // reading), falling back to the newest entry overall; that is the title asset in the usual case. The note is kept for the
        // crash report's "last zstd error" section.
        char err[256];
        std::snprintf(err, sizeof err, "%s (code %lld, source %llu bytes, destination capacity %llu bytes)",
                      ZSTD_getErrorName(r), static_cast<long long>(r), static_cast<unsigned long long>(c->x[4]),
                      static_cast<unsigned long long>(c->x[2]));
        const std::string file = nxrt::diag_last_file();
        std::string note = "last file ";
        note += file.empty() ? "(none)" : file;
        note += "; ";
        note += err;
        NXLOG_W("zstd", "native decode failed: %s (falling back to the lifted decoder)", note.c_str());
        nxrt::diag_set_zstd_error(note.c_str());
        orig(c);
        return;
    }
    c->x[0] = r;
}

// ---- Culling tests ---------------------------------------------------------------------------------------------------------------------
// Two leaf functions of main that the main thread calls for every object against every culling volume: "does this box touch the view
// volume" and "does this box touch this cylinder". Both read their two arguments, write nothing and return 0 or 1 in w0, so a native
// version only has to give the same answer. Every product, sum and comparison below is in single precision in the game's order, with no
// multiply-add contraction (the pragma) and a fused multiply-add where the original has FMLA (cull_fma, following the lifted code's own
// choice for the build).
//
// FEFW_HLE_CULL (environment): unset or 1 = the native tests (default), 0 = the lifted originals, 2 = both, compared: the lifted
// answer is the one returned, and cull_verify.log beside the program gets the first 16 calls that differ and a count line at the
// first call and at each power of four after it. recomp/tests/hle_cull_test.cpp compares the two outside the game.
#pragma clang fp contract(off)

namespace {

typedef float __attribute__((may_alias)) cf32;
typedef int32_t __attribute__((may_alias)) ci32;

inline float cull_fma(float a, float b, float c) {
#if defined(__FMA__)
    return __builtin_fmaf(a, b, c);
#elif defined(NX_FAST_FMA)
    return a * b + c;                  // two roundings, as the lifted code of this build
#else
    return fmaf(a, b, c);
#endif
}
inline float cull_sqrt(float x) { return _mm_cvtss_f32(_mm_sqrt_ss(_mm_set_ss(x))); }

// A box as the callers pass it: minimum at +0, maximum at +0x10 (x, y, z).
struct CullBox { const cf32 *mn, *mx; };
inline CullBox cull_box(uint64_t a) { return {reinterpret_cast<const cf32*>(a), reinterpret_cast<const cf32*>(a + 0x10)}; }

// View volume: its own bounds at +0x10 / +0x20, planes of 0x20 bytes from +0x30 (normal x, y, z, a fourth float multiplied by 0;
// distance at +0x10 of the entry), plane count at +0xf0, "use the bounds" at +0xf4, and at +0x100 bit 0 a flag that leaves plane 3 out.
// A plane's side: dot(normal, the box corner least far along the normal) + distance, summed as (z + x) + (w + y) like the original,
// whose four lanes these are (the corner's fourth lane is 0).
inline bool view_plane_out(const cf32* p, __m128 mn, __m128 mx) {
    const __m128 n = _mm_loadu_ps(p);
    const __m128 ge = _mm_cmpge_ps(n, _mm_setzero_ps());                     // false for a negative or NaN component: the maximum
    const __m128 prod = _mm_mul_ps(n, _mm_or_ps(_mm_and_ps(ge, mn), _mm_andnot_ps(ge, mx)));
    const __m128 s = _mm_add_ps(prod, _mm_movehl_ps(prod, prod));            // (x + z, y + w)
    const float side = (_mm_cvtss_f32(s) + _mm_cvtss_f32(_mm_shuffle_ps(s, s, 1))) + p[4];
    return side > 0.0f;
}

int cull_view_box(uint64_t v, uint64_t box) {
    const CullBox b = cull_box(box);
    const cf32 *lo = reinterpret_cast<const cf32*>(v + 0x10), *hi = reinterpret_cast<const cf32*>(v + 0x20);
    if (*reinterpret_cast<const ci32*>(v + 0xf4)) {
        if (lo[0] > b.mx[0] || b.mn[0] > hi[0] || lo[1] > b.mx[1] || b.mn[1] > hi[1] || lo[2] > b.mx[2] || b.mn[2] > hi[2]) return 0;
    }
    const int n = *reinterpret_cast<const ci32*>(v + 0xf0);
    if (n < 1) return 1;
    const __m128 xyz = _mm_castsi128_ps(_mm_set_epi32(0, -1, -1, -1));       // the corner's fourth lane is 0
    const __m128 mn = _mm_and_ps(_mm_loadu_ps(b.mn), xyz), mx = _mm_and_ps(_mm_loadu_ps(b.mx), xyz);
    const cf32* plane = reinterpret_cast<const cf32*>(v + 0x30);
    const bool skip3 = *reinterpret_cast<const uint8_t*>(v + 0x100) & 1;
    for (int i = 0; i < n; ++i) {
        if (skip3 && i == 3) continue;
        if (view_plane_out(plane + 8 * i, mn, mx)) return 0;
    }
    return 1;
}

// Cylinder volume (a thick ray): a frame of four columns at +0x10 (only its x and y rows are used), two end planes at +0x70 and +0x80
// (x, y, z, d) used when bit 0 of +0xb0 is set, six box corner numbers at +0x94 (bit 0 = maximum x, bit 1 = maximum y, bit 2 =
// maximum z: the box's outline seen along the axis, in order), the radius at +0xac.
inline int cull_edge_near_origin(float ax, float ay, float bx, float by, float radius) {
    const float ex = bx - ax, ey = by - ay;
    const float nx = 0.0f - ax, ny = 0.0f - ay;
    const float t = nx * ex + ny * ey;
    float cx, cy;
    if (t <= 0.0f) {
        cx = ax, cy = ay;
    } else {
        const float len2 = ex * ex + ey * ey;
        if (t >= len2) {
            cx = bx, cy = by;
        } else {
            const float f = t / len2;
            cx = ex * f + ax, cy = ey * f + ay;
        }
    }
    return cull_sqrt(cx * cx + cy * cy) <= radius;
}

int cull_cylinder_box(uint64_t v, uint64_t box) {
    const CullBox b = cull_box(box);
    if (*reinterpret_cast<const uint8_t*>(v + 0xb0) & 1) {
        for (int i = 0; i < 2; ++i) {
            const cf32* p = reinterpret_cast<const cf32*>(v + 0x70 + 0x10 * static_cast<uint64_t>(i));
            const float vx = !(p[0] >= 0.0f) ? b.mx[0] : b.mn[0];
            const float vy = !(p[1] >= 0.0f) ? b.mx[1] : b.mn[1];
            const float vz = !(p[2] >= 0.0f) ? b.mx[2] : b.mn[2];
            const float px = p[0] * vx, py = p[1] * vy, pz = p[2] * vz, pw = p[3] * 1.0f;
            if ((pz + px) + (pw + py) > 0.0f) return 0;
        }
    }
    const cf32* m = reinterpret_cast<const cf32*>(v + 0x10);          // m[4 * column + row]
    const ci32* corner = reinterpret_cast<const ci32*>(v + 0x94);
    float px[6], py[6];
    for (int j = 0; j < 6; ++j) {
        const int c = corner[j];
        const float cx = (c & 1) ? b.mx[0] : b.mn[0], cy = (c & 2) ? b.mx[1] : b.mn[1], cz = (c & 4) ? b.mx[2] : b.mn[2];
        px[j] = m[12] + cull_fma(m[8], cz, cull_fma(m[4], cy, m[0] * cx));
        py[j] = m[13] + cull_fma(m[9], cz, cull_fma(m[5], cy, m[1] * cx));
    }
    const float kEps = 1.1920929e-07f;
    int k = -1;
    for (int j = 0; j < 5; ++j) {
        const float dx = px[5] - px[j], dy = py[5] - py[j];
        if (!(dx * dx + dy * dy <= kEps)) {
            k = j;
            break;
        }
    }
    const float radius = *reinterpret_cast<const cf32*>(v + 0xac);
    int first_ccw = 1, from = 5, to = 5;
    if (k >= 0) {
        const float ax = px[5], ay = py[5], bx = px[k], by = py[k];
        if (cull_edge_near_origin(ax, ay, bx, by, radius)) return 1;
        const float ex = bx - ax, ey = by - ay;
        const float s = ((0.0f - ay) * ex) * 1.0f + ((0.0f - ax) * ey) * -1.0f;
        first_ccw = s >= 0.0f;
        from = k;
        to = k + 1;
    }
    int inside = 1;
    for (; to != 6; ++to) {
        const float ax = px[from], ay = py[from], bx = px[to], by = py[to];
        const float dx = ax - bx, dy = ay - by;
        if (dx * dx + dy * dy <= kEps) continue;
        if (cull_edge_near_origin(ax, ay, bx, by, radius)) return 1;
        if (inside) {
            const float ex = bx - ax, ey = by - ay;
            const float s = (ex * (0.0f - ay)) * 1.0f + (ey * (0.0f - ax)) * -1.0f;
            inside = first_ccw ^ !(s >= 0.0f);
        }
        from = to;
    }
    return inside;
}

int cull_mode() {
    static int mode = -1;
    int v = __atomic_load_n(&mode, __ATOMIC_RELAXED);
    if (v < 0) {
        const char* e = getenv("FEFW_HLE_CULL");
        v = e && e[0] >= '0' && e[0] <= '2' && !e[1] ? e[0] - '0' : 1;
        __atomic_store_n(&mode, v, __ATOMIC_RELAXED);
    }
    return v;
}

// FEFW_HLE_CULL=2: the lifted function has run and left its answer in w0; `mine` is the native one for the same arguments.
void cull_verify(const char* name, int which, int mine, uint64_t vol, uint64_t box, const Ctx* c) {
    static uint64_t calls[2], differ[2];
    const uint64_t n = __atomic_add_fetch(&calls[which], 1, __ATOMIC_RELAXED);
    const int theirs = static_cast<int>(static_cast<uint32_t>(c->x[0]));
    uint64_t d = __atomic_load_n(&differ[which], __ATOMIC_RELAXED);
    const bool bad = theirs != mine;
    if (bad) d = __atomic_add_fetch(&differ[which], 1, __ATOMIC_RELAXED);
    const bool count_line = (n & (n - 1)) == 0 && (__builtin_ctzll(n) & 1) == 0;     // 1, 4, 16, 64, ...
    if (!(bad && d <= 16) && !count_line) return;
    FILE* f = fopen("cull_verify.log", "a");
    if (!f) return;
    if (bad && d <= 16) {
        fprintf(f, "%s: call %llu differs: lifted %d, native %d; volume", name, static_cast<unsigned long long>(n), theirs, mine);
        for (int i = 0; i < 0x108; i += 4) fprintf(f, " %08x", rd32(vol + static_cast<uint64_t>(i)));
        fprintf(f, "; box");
        for (int i = 0; i < 0x20; i += 4) fprintf(f, " %08x", rd32(box + static_cast<uint64_t>(i)));
        fprintf(f, "\n");
    }
    if (count_line) fprintf(f, "%s: %llu calls compared, %llu differ\n", name, static_cast<unsigned long long>(n), static_cast<unsigned long long>(d));
    fclose(f);
}

}  // namespace

extern "C" void nx_hle_cull_view_box(Ctx* c, void (*orig)(Ctx*)) {
    const int mode = cull_mode();
    if (mode == 0) {
        orig(c);
        return;
    }
    const uint64_t vol = c->x[0], box = c->x[1];
    const int r = cull_view_box(vol, box);
    if (mode == 2) {
        orig(c);
        cull_verify("view volume / box", 0, r, vol, box, c);
        return;
    }
    c->x[0] = static_cast<uint32_t>(r);
}

extern "C" void nx_hle_cull_cylinder_box(Ctx* c, void (*orig)(Ctx*)) {
    const int mode = cull_mode();
    if (mode == 0) {
        orig(c);
        return;
    }
    const uint64_t vol = c->x[0], box = c->x[1];
    const int r = cull_cylinder_box(vol, box);
    if (mode == 2) {
        orig(c);
        cull_verify("cylinder / box", 1, r, vol, box, c);
        return;
    }
    c->x[0] = static_cast<uint32_t>(r);
}

// For the test program (recomp/tests): the two tests by their arguments.
extern "C" int nx_hle_cull_view_box_raw(uint64_t volume, uint64_t box) { return cull_view_box(volume, box); }
extern "C" int nx_hle_cull_cylinder_box_raw(uint64_t volume, uint64_t box) { return cull_cylinder_box(volume, box); }
